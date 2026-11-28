"""The daily real-login journey on the journey account (account.py).

Once a day, inside the account's own login session, where its keychain is
open, a LaunchAgent runs `run`: it moves the account's one checkout to main
(or to the commit a `request` names), installs the CLIs contracts/agent-clis.json
pins, builds the journey's binaries while holding the machine's shared cargo
lock and no longer, runs every agent's journey on their real logins, and
replaces one file, ~/journey/record.json, with what the morning check acts
on: each agent's CLI version and result, never an account name or screen
text. The account holds no GitHub login: the morning check reads the file
over SSH and reports from the operator's machine.
Before the journey each agent answers one prompt outside Hide, and only the
ones that answer are measured: whether an agent is signed in is its CLI's
answer, never a reading of its files. An agent with a failed cell answers
again afterwards; what it does goes beside the journey's result, never over
it (`outside`), so a reader tells Hide's failure from the agent's own state
(`login needed`, `usage limit`, `update needed`, `agent error`). Every
command of the run is owned (agent_live_check.processes): its whole tree ends
at its deadline, with the run, or when the runner dies. The log names each
command and its exit and nothing a command printed. Everything else stays in
the run folder on that machine. docs/VERIFICATION.md, The daily real-login
journey.
"""

import argparse
import contextlib
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import secrets
import shutil
import signal
import subprocess
import sys
import threading
import time

from agent_live_check.processes import Limits, OwnedProcesses, ProcessError

from . import account
from .cells import EXCLUDED_AGENTS, FAIL, NOT_MEASURED, PASS

CHECKOUT = Path(__file__).resolve().parents[2]
LABEL = "dev.withhide.agent-journey"
# The one place the morning check and a release read (docs/VERIFICATION.md,
# The daily real-login journey, for its fields).
RECORD = "record.json"
STALE = ("a record older than a day means the run did not start: a restart of the machine ends the account's "
         "login session, which the run needs, until someone logs in to the account again")
# 05:00 on the machine's clock (Asia/Seoul on the mini): after the nightly
# (17:17 UTC, 02:17 KST) and before the morning check reads the record.
HOUR, MINUTE = 5, 0
# What a run keeps, newest first: run folders and logs by count and by
# bytes together, the oldest going first past either.
KEEP_RUNS, RUNS_CAP = 7, 2 * 1024 ** 3
KEEP_LOGS, LOGS_CAP = 14, 64 * 1024 ** 2
# The last bytes of a command's output the runner reads (an agent's answer,
# the installer's list of programs); none of it is written anywhere.
OUTPUT_TAIL = 64 * 1024
# What lockf exits with when the lock stayed taken past its wait (EX_TEMPFAIL).
LOCK_TIMEOUT_EXIT = 75
# A folder of the account's own that a build or an install grows, removed
# whole past its cap, before the build and after it; the next build fills
# it again. A day's build of the journey's binaries leaves target at about
# 4 GiB (2026-11-28).
CAPS = {"target": 30 * 1024 ** 3, "cargo-home": 4 * 1024 ** 3, "build-cache": 2 * 1024 ** 3}
# The agents' own stores in the account's home, which their prompts and the
# journey's sessions grow: past STORES_CAP before or after a run, fail and
# ask the operator to clear space. Never remove anything from these stores.
STORES = (".claude", ".codex", ".local/share/opencode", ".pi", ".omp", ".grok", ".gemini")
STORES_CAP = 8 * 1024 ** 3
LOCK_WAIT_SECONDS = 3 * 3600
BUILD_SECONDS = 90 * 60
JOURNEY_SECONDS = 150 * 60
CHECK_SECONDS = 180
STEP_SECONDS = 30 * 60
# The whole run, every step at its own deadline: the owner's guardians end
# anything still running past it.
RUN_SECONDS = 2 * STEP_SECONDS + LOCK_WAIT_SECONDS + BUILD_SECONDS + JOURNEY_SECONDS + 8 * CHECK_SECONDS
# A build of Hide's binaries runs about a dozen compilers at once, past an
# agent's limits (agent_live_check.processes.Limits), on a 16 GiB mini.
BUILD_LIMITS = Limits(descendants=512, rss_bytes=12 * 1024 ** 3)
SYSTEM_PATH = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
BUILDS = [["cli"], ["build", "-p", "hide-session", "--example", "journey_conversation"],
          ["build", "-p", "hide-memory", "--example", "journey_memory"],
          ["build", "-p", "hide-agent-adapter", "--example", "journey_composer"]]
# The one prompt an agent answers outside Hide, in its own folders, when a
# cell of its failed, and the arguments each CLI takes it with
# non-interactively.
CHECK_PROMPT = "Reply with the single word ok."
ONE_SHOT = {
    # The four that can answer without keeping a session do.
    "claude": ["-p", "--no-session-persistence", CHECK_PROMPT],
    "codex": ["exec", "--ephemeral", "--skip-git-repo-check", CHECK_PROMPT],
    # A private server, which ends with the prompt, not the background one.
    "opencode": ["run", "--standalone", CHECK_PROMPT],
    "pi": ["-p", "--no-session", CHECK_PROMPT],
    "omp": ["-p", "--no-session", CHECK_PROMPT],
    "grok": ["-p", CHECK_PROMPT],
    "agy": ["-p", CHECK_PROMPT],
}
# What an agent that could not answer outside Hide said, by kind, read on
# the machine only: the record keeps the kind, never the words. The first
# that matches wins, so a usage limit that says "upgrade" reads as a limit.
AGENT_STATES = [
    ("usage limit", re.compile(r"usage limit|rate.?limit|\b429\b|quota|too many requests|limit reached|"
                               r"out of credits|insufficient (?:credit|balance|funds)", re.I)),
    # Only a refusal of the sign-in itself: an unknown failure that names a
    # login on the way is an agent error, not a sign-in for a person to make.
    ("login needed", re.compile(r"not (?:logged|signed) in|logged out|(?:log|sign) ?in (?:again|required)|"
                                r"please (?:log|sign) ?in|run /?login|unauthori[sz]ed|\b401\b|"
                                r"authentication (?:failed|required)|invalid (?:api key|credentials?|token)|"
                                r"(?:token|session|credentials?|login) (?:has )?expired|re-?authenticate", re.I)),
    ("update needed", re.compile(r"update|upgrade|newer version|no longer supported|unsupported version", re.I)),
]


def journey_root():
    return account_home() / "journey"


def account_home():
    """The journey account's home, which holds the agents' stores."""
    return Path.home()


def runs_folder():
    """Where each run's report folder goes, kept as the journey keeps its own."""
    return CHECKOUT / "agents/runs"


class Stage(Exception):
    """A whole-run failure, by a constant name the record shows."""


class Stopped(Exception):
    """The runner was asked to stop (launchd's SIGTERM)."""


class Tail:
    """The last `cap` bytes a stream wrote, read to its end."""

    def __init__(self, cap):
        self.cap, self.data, self.error = cap, bytearray(), None

    def read(self, stream):
        # A failed read is the step's failure, raised by step, not a
        # thread's traceback on stderr.
        try:
            while chunk := os.read(stream.fileno(), 65536):
                self.data += chunk
                if len(self.data) > self.cap:
                    del self.data[:len(self.data) - self.cap]
        except Exception as error:
            self.error = error

    def text(self):
        return bytes(self.data).decode("utf-8", errors="replace")


def log(stream, message):
    stream.write(f"{datetime.datetime.now().isoformat(timespec='seconds')} {message}\n")
    stream.flush()


def environment(settings, extra=None):
    """The run's environment: none of the launching shell's Hide variables
    or provider keys, the journey's CLIs first on PATH, then the toolchain."""
    root = journey_root()
    base = {key: value for key, value in os.environ.items()
            if not key.startswith("HIDE_") and not key.endswith("_API_KEY")}
    path = [str(root / "bin"), settings["toolchain_bin"], *SYSTEM_PATH]
    return {**base, "PATH": os.pathsep.join(path), "LANG": "en_US.UTF-8", "PYTHONUTF8": "1",
            "CARGO_HOME": str(root / "cargo-home"), "RUSTUP_HOME": settings["rustup_home"],
            # The terminal parser's prepared inputs are the account's own:
            # preparing them stamps the folder it reads (prepare-vt-inputs.py).
            "HIDE_BUILD_CACHE": str(root / "build-cache"), **(extra or {})}


def step(owner, name, argv, stream, env, seconds, *, cwd=CHECKOUT):
    """One command of the run, owned by `owner`: its whole tree ends at
    `seconds`, with the run, or when the runner dies. The log names the step
    and its exit, never what it printed, which can carry a path, an account
    or a secret. Its code and its output's last OUTPUT_TAIL bytes."""
    end = time.monotonic() + seconds
    child = owner.spawn(argv, env=env, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, deadline=end)
    tail = Tail(OUTPUT_TAIL)
    reader = threading.Thread(target=tail.read, args=(child.stdout,), daemon=True)
    reader.start()
    try:
        try:
            code = child.wait(timeout=max(0.0, end - time.monotonic()))
        except subprocess.TimeoutExpired as error:
            log(stream, f"{name}: no end in {seconds} s")
            raise Stage(f"{name} timed out") from error
        finally:
            try:
                owner.end(child)
            finally:
                reader.join(timeout=5)
                child.stdout.close()
    except ProcessError as error:
        # The exception type is ours; the combined pipe is command-controlled.
        # Never derive log text from that pipe, even a guardian-shaped line.
        log(stream, f"{name}: {type(error).__name__}")
        raise
    if tail.error is not None:
        log(stream, f"{name}: output unreadable: {type(tail.error).__name__}")
        raise tail.error
    log(stream, f"{name}: exit {code}")
    return code, tail


def checkout(owner, target, stream, env):
    """Moves the checkout to `target` (a commit or origin/main); its commit."""
    ref = target or "main"
    for name, argv in (("fetch", ["git", "fetch", "--quiet", "origin", ref]),
                       ("checkout", ["git", "checkout", "--quiet", "--detach", "FETCH_HEAD"]),
                       # The run folders and the target stay; anything else the last commit left goes.
                       ("clean", ["git", "clean", "-fdq", "-e", "/agents/", "-e", "/target/"])):
        if step(owner, name, argv, stream, env, 600)[0]:
            raise Stage("checkout failed")
    code, head = step(owner, "rev-parse", ["git", "rev-parse", "HEAD"], stream, env, 60)
    if code or not re.fullmatch(r"[0-9a-f]{40}", head.text().strip()):
        raise Stage("checkout failed")
    return head.text().strip()


def install_clis(owner, stream, env):
    """The pinned CLIs, installed once per pin table into a folder of their
    own, linked from ~/journey/bin by name; an older table's folder goes."""
    root = journey_root()
    table = (CHECKOUT / "contracts/agent-clis.json").read_bytes()
    folder = root / "clis" / hashlib.sha256(table).hexdigest()[:16]
    installed = folder / "installed.json"
    if not installed.is_file():
        remove(folder)
        folder.mkdir(parents=True)
        code, output = step(owner, "install", [sys.executable, "scripts/install-agent-clis.py", "--all",
                                               "--prefix", str(folder)], stream, env, STEP_SECONDS)
        if code:
            raise Stage("CLI install failed")
        programs = dict(line.split("=", 1) for line in output.text().splitlines()
                        if re.fullmatch(r"[a-z]+=/.+", line))
        installed.write_text(json.dumps(programs))
    programs = json.loads(installed.read_text())
    links = root / "bin"
    links.mkdir(exist_ok=True)
    for kind, program in programs.items():
        link = links / kind
        require_removable(link, link=True)
        replacement = link.with_name(f".{kind}-{secrets.token_hex(8)}.next")
        replacement.symlink_to(program)
        replace(replacement, link, links=True)
    for other in (root / "clis").iterdir():
        if other != folder:
            remove(other)
    return programs


def controller_provider():
    return journey_root() / "controller/claude"


def require_removable(path, *, link=False):
    """One account/path gate for removals and atomic replacements.

    Only a CLI link replacement may address a final symlink; its parent
    still must be reached without crossing a link from the account home.
    """
    path = Path(path)
    if not account.removable(account_home(), path.parent if link else path):
        raise RuntimeError("a change outside the journey account's owned paths")


def replace(source, target, *, links=False):
    for path in (source, target):
        require_removable(path, link=links)
    os.replace(source, target)


def remove(path):
    """Removes a file or a folder, only inside the journey account's own home
    (account.removable): every removal of the runner goes through here."""
    path = Path(path)
    if not path.exists() and not path.is_symlink():
        return
    require_removable(path)
    if path.is_dir():
        shutil.rmtree(path)
    else:
        path.unlink()


def size(path):
    total = 0
    for folder, _, files in os.walk(path):
        for name in files:
            with contextlib.suppress(OSError):
                total += os.lstat(os.path.join(folder, name)).st_size
    return total


def within_caps(stream):
    """Removes each grown folder of the account's past its cap."""
    for name, cap in CAPS.items():
        folder = CHECKOUT / name if name == "target" else journey_root() / name
        if folder.is_dir() and size(folder) > cap:
            log(stream, f"{name} past its cap: removed")
            remove(folder)


def stores_size(home):
    """Measure the agents' stores without changing sessions or login files."""
    return sum(size(home / name) for name in STORES)


def build(settings, stream, env):
    """The journey's binaries, built while holding the shared cargo lock,
    under an owner of the build's own limits; the lock goes with the build's
    tree (`lockf` holds it for its child)."""
    within_caps(stream)
    script = " && ".join("bash scripts/verify-cargo.sh " + " ".join(argv) for argv in BUILDS)
    argv = ["/usr/bin/lockf", "-k", "-t", str(LOCK_WAIT_SECONDS), settings["cargo_lock"], "/bin/bash", "-c", script]
    with OwnedProcesses(run_seconds=LOCK_WAIT_SECONDS + BUILD_SECONDS + STEP_SECONDS, limits=BUILD_LIMITS) as owner:
        code, _ = step(owner, "build", argv, stream, env, LOCK_WAIT_SECONDS + BUILD_SECONDS)
        if code == LOCK_TIMEOUT_EXIT:
            raise Stage("build lock not free")
        if code:
            raise Stage("build failed")
        # The controller's spawn provider, built here as CI builds it: inside
        # the journey a compile shares one command's 15 s cap, which a loaded
        # mini's first compile in the session outlasted (2026-11-28).
        provider = controller_provider()
        provider.parent.mkdir(parents=True, exist_ok=True)
        if step(owner, "controller provider", ["/usr/bin/cc", "-O2", "-o", str(provider),
                                               "web/e2e/shims/spawn-provider.c"], stream, env, 600)[0]:
            raise Stage("build failed")
    within_caps(stream)


def summarize(report):
    """Each agent's result from the journey's report: what the record shows."""
    agents = []
    for row in report["agents"]:
        key = row["id"]
        cells = report["table"].get(key, {})
        counts = {}
        for cell in cells.values():
            counts[cell["status"]] = counts.get(cell["status"], 0) + 1
        failed = sorted(name for name, cell in cells.items() if cell["status"] == FAIL)
        version = re.search(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?", row.get("version") or "")
        entry = {"id": key, "cli": version.group(0) if version else None, "cells": counts, "failed": failed}
        if row.get("excluded"):
            entry["result"] = "excluded"
        elif failed:
            entry["result"] = "fail"
        elif counts.get(PASS):
            entry["result"] = "pass"
        elif row.get("login_in_use"):
            # An agent someone left open in the account holds its login.
            entry["result"] = "login in use"
        else:
            entry["result"] = NOT_MEASURED
        agents.append(entry)
    return agents


def agent_state(output):
    """The kind of what an agent said when it could not answer outside Hide."""
    for name, pattern in AGENT_STATES:
        if pattern.search(output):
            return name
    return "agent error"


def check_outside(owner, kind, stream, env):
    """`answers` when the agent answers one prompt in its own folders,
    outside Hide; else the kind of its state. What it said is read here and
    kept nowhere, the log included: it can carry the account's name. It runs
    in one folder kept for it, so an agent that keeps a session per folder
    keeps these in one place."""
    program = journey_root() / "bin" / kind
    if kind not in ONE_SHOT or not program.exists():
        return "agent error"
    folder = journey_root() / "probe"
    folder.mkdir(exist_ok=True)
    try:
        code, output = step(owner, f"{kind} outside Hide", [str(program), *ONE_SHOT[kind]], stream, env,
                            CHECK_SECONDS, cwd=folder)
    except Stage:
        return "agent error"
    said = output.text()
    if not code and re.search(r"\bok\b", said, re.I):
        return "answers"
    return agent_state(said)


def unmeasured(key, state):
    """An agent that did not answer before the journey, so was not measured."""
    return {"id": key, "cli": None, "cells": {}, "failed": [],
            "result": "login needed" if state == "login needed" else NOT_MEASURED, "outside": state}


def recheck(agents, kinds, check):
    """An agent with a failed cell answers again (`check(kind)`), as its
    failure may come from its own state, such as a limit reached during the
    run. The answer goes beside the result as `outside`, never over it: a
    cell Hide failed stays failed whatever the agent's state."""
    for entry in agents:
        if entry["result"] == FAIL:
            entry["outside"] = check(kinds[entry["id"]])


def publish(record, stream):
    """Replaces the record file whole, so a reader never sees half of one."""
    target = journey_root() / RECORD
    replacement = target.with_name(target.name + ".next")
    require_removable(replacement)
    require_removable(target)
    replacement.write_text(json.dumps(record, indent=1) + "\n")
    replace(replacement, target)
    log(stream, f"record: run {record['run']}")


def keep_newest(folder, count, cap):
    """Run folders or logs past the newest `count`, or past `cap` bytes
    together, oldest first."""
    if not folder.is_dir():
        return
    entries = sorted((entry for entry in folder.iterdir() if entry.name.startswith("daily-")),
                     key=lambda entry: entry.name, reverse=True)
    total = 0
    for index, entry in enumerate(entries):
        total += size(entry) if entry.is_dir() else entry.stat().st_size
        if index >= count or total > cap:
            remove(entry)


def trim(path, cap):
    """Keeps a log that only grows (launchd's, setup's) to its last `cap` bytes."""
    if path.is_file() and path.stat().st_size > cap:
        with open(path, "rb") as stream:
            stream.seek(-cap, os.SEEK_END)
            kept = stream.read()
        path.write_bytes(kept)


def cleanup():
    """What a run leaves: run folders and logs past the newest few. Its
    processes the owners' guardians end, and its folder in /private/tmp the
    journey removes (agent_live_check/runtime.py); the runner removes nothing
    there whose maker it cannot prove, such as an agent's own folders."""
    root = journey_root()
    keep_newest(runs_folder(), KEEP_RUNS, RUNS_CAP)
    keep_newest(root / "logs", KEEP_LOGS, LOGS_CAP)
    for name in ("launchd.log", "setup.log"):
        trim(root / "logs" / name, OUTPUT_TAIL)


def stop(*_):
    raise Stopped()


def run(settings):
    # Before it makes or removes anything: a run anywhere but the journey
    # account's own home would act on a person's files.
    if not account.dedicated(account_home()):
        raise RuntimeError("the daily run runs only as the journey account")
    import fcntl  # the runner is macOS's; nothing else loads this module's run

    root = journey_root()
    (root / "logs").mkdir(parents=True, exist_ok=True)
    started = datetime.datetime.now().astimezone()
    stamp = started.strftime("%Y%m%dT%H%M%S")
    with open(root / "run.lock", "w") as lock, open(root / "logs" / f"daily-{stamp}.log", "w") as stream:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log(stream, "another run holds the journey; it takes a request made meanwhile when it ends")
            return None
        # launchd stops a job with SIGTERM: the run still ends its commands
        # and says so in the record.
        signal.signal(signal.SIGTERM, stop)
        request = root / "request.json"
        run_dir = runs_folder() / f"daily-{stamp}"
        asked = {}
        if request.exists() or request.is_symlink():
            require_removable(request)
            asked = json.loads(request.read_text())
            require_removable(run_dir)
            run_dir.mkdir(parents=True, exist_ok=True)
            replace(request, run_dir / "request.json")
        target = asked.get("commit")
        record = {"format": 1, "started": started.isoformat(timespec="seconds"), "if_stale": STALE,
                  "source": f"request {target}" if target else "main", "request": asked.get("id"),
                  "run": "running", "exit": None, "agents": []}
        # A reader that finds this record knows the run has begun and not ended.
        publish(record, stream)
        env = environment(settings)
        try:
            # Before any agent runs: a run would grow the stores further.
            if stores_size(account_home()) > STORES_CAP:
                raise Stage("agent stores over cap")
            with OwnedProcesses(run_seconds=RUN_SECONDS) as owner:
                record["commit"] = checkout(owner, target, stream, env)
                install_clis(owner, stream, env)
                build(settings, stream, env)
                kinds = {row["id"]: row["agent_cli"] for row in json.loads(
                    (CHECKOUT / "contracts/agent-journey.json").read_text())}
                # Whether each agent is signed in is its own answer, asked first.
                before = {key: check_outside(owner, kind, stream, env)
                          for key, kind in kinds.items() if key not in EXCLUDED_AGENTS}
                answering = [key for key, state in before.items() if state == "answers"]
                measured = {}
                if answering:
                    # The journey's output is its screens' words: its report folder
                    # keeps what it found, the log only its exit.
                    record["exit"], _ = step(owner, "journey", [
                        sys.executable, "-X", "utf8", "scripts/agent-journey-check.py", "--run-dir", str(run_dir),
                        "--controller-provider", str(controller_provider()), "--agents", ",".join(answering)],
                        stream, env, JOURNEY_SECONDS)
                    report = run_dir / "report.json"
                    if not report.is_file():
                        raise Stage("journey left no report")
                    measured = {entry["id"]: entry for entry in summarize(json.loads(report.read_text()))}
                    recheck(measured.values(), kinds, lambda kind: check_outside(owner, kind, stream, env))
                record["agents"] = [
                    {"id": key, "cli": None, "cells": {}, "failed": [], "result": "excluded"}
                    if key in EXCLUDED_AGENTS else measured.get(key) or unmeasured(key, before[key])
                    for key in kinds]
            record["run"] = "setup or cleanup failure" if record["exit"] == 2 else "ok"
        except Stage as stage:
            record["run"] = str(stage)
            log(stream, f"stopped: {stage}")
        except Stopped:
            record["run"] = "runner stopped"
            log(stream, "stopped by launchd")
        except Exception as error:
            # Only the kind: a message can carry a path or what a command said.
            record["run"] = f"runner failed: {type(error).__name__}"
            log(stream, record["run"])
        except BaseException as error:
            # An interrupt still ends the runner; the entry point reports its kind.
            record["run"] = f"runner failed: {type(error).__name__}"
            log(stream, record["run"])
            raise
        finally:
            record["minutes"] = round((time.time() - started.timestamp()) / 60)
            cleanup()
            try:
                total = stores_size(account_home())
                record["stores_mb"] = round(total / 1024 ** 2)
                if total > STORES_CAP:
                    record["run"] = "agent stores over cap"
                    log(stream, record["run"])
            except Exception as error:
                record["run"] = f"runner failed: {type(error).__name__}"
                log(stream, f"agent stores: {record['run']}")
            publish(record, stream)
    return 0 if record["run"] == "ok" else 1


def plist(settings):
    root = journey_root()
    return {"Label": LABEL, "ProgramArguments": [sys.executable, "-X", "utf8",
                                                 str(CHECKOUT / "scripts/agent-journey-daily.py"), "run"],
            "StartCalendarInterval": {"Hour": HOUR, "Minute": MINUTE},
            "LimitLoadToSessionType": "Aqua", "ProcessType": "Standard",
            "StandardOutPath": str(root / "logs/launchd.log"), "StandardErrorPath": str(root / "logs/launchd.log"),
            "EnvironmentVariables": {"PATH": os.pathsep.join(SYSTEM_PATH), "LANG": "en_US.UTF-8"}}


def setup(args):
    """Run once as the journey account (over SSH is fine): marks it, writes
    the settings and the LaunchAgent, installs the pinned CLIs."""
    name = os.environ.get("USER") or ""
    if account.administrator(name):
        print("refused: the journey account is a standard account; this one is an administrator", file=sys.stderr)
        return 2
    root = journey_root()
    if CHECKOUT != root / "hide":
        print(f"refused: run setup from the journey's checkout, {root / 'hide'}", file=sys.stderr)
        return 2
    partial = subprocess.run(["git", "config", "--get", "remote.origin.partialclonefilter"], cwd=CHECKOUT,
                             capture_output=True, text=True)
    if partial.stdout.strip():
        # The journey's shared clone of this checkout checks every object, and
        # a partial clone fetches each missing one from GitHub, past the
        # journey's 15 s command cap (on the mini, 2026-11-28).
        print("refused: the journey's checkout must be a full clone, not a partial one", file=sys.stderr)
        return 2
    (Path.home() / account.MARKER).write_text("The daily real-login journey's account (scripts/agent_journey/account.py).\n")
    for folder in ("logs", "bin", "clis", "cargo-home"):
        (root / folder).mkdir(parents=True, exist_ok=True)
    settings = {"cargo_lock": str(args.cargo_lock), "toolchain_bin": str(args.toolchain_bin),
                "rustup_home": str(args.rustup_home)}
    (root / "settings.json").write_text(json.dumps(settings, indent=1))
    agents = Path.home() / "Library/LaunchAgents"
    agents.mkdir(parents=True, exist_ok=True)
    with open(agents / f"{LABEL}.plist", "wb") as stream:
        plistlib.dump(plist(settings), stream)
    profile = Path.home() / ".zprofile"
    line = f'export PATH="{root / "bin"}:/opt/homebrew/bin:$PATH"\n'
    if line not in (profile.read_text() if profile.is_file() else ""):
        with open(profile, "a") as stream:
            stream.write(line)
    with open(root / "logs/setup.log", "w") as stream, OwnedProcesses(run_seconds=2 * STEP_SECONDS) as owner:
        print(json.dumps(install_clis(owner, stream, environment(settings)), indent=1))
    print(f"set up; the LaunchAgent loads at this account's next login, or now with:\n"
          f"  launchctl bootstrap gui/{os.getuid()} {agents / (LABEL + '.plist')}")
    return 0


def request(args):
    """Asks the account's session to run now, on `--commit` when given: a
    run outside that session would find the keychain locked."""
    root = journey_root()
    asked = {"commit": args.commit, "id": secrets.token_hex(8)}
    (root / "request.json").write_text(json.dumps(asked))
    code = subprocess.run(["launchctl", "kickstart", f"gui/{os.getuid()}/{LABEL}"]).returncode
    if not code:
        # The record that answers it carries this id as `request`.
        print(asked["id"])
    return code


def status(args):
    record = journey_root() / RECORD
    print(record.read_text() if record.is_file() else "{}")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description="The daily real-login journey on the journey account.")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("run", help="one run now (the LaunchAgent's command)")
    made = commands.add_parser("setup", help="once, as the journey account")
    made.add_argument("--cargo-lock", type=Path, required=True, help="the machine's shared cargo lock file")
    made.add_argument("--toolchain-bin", type=Path, required=True, help="the folder of the machine's rustup cargo")
    made.add_argument("--rustup-home", type=Path, required=True, help="the machine's RUSTUP_HOME, read only")
    asked = commands.add_parser("request", help="ask the account's session to run now")
    asked.add_argument("--commit", help="a commit of with-hide/hide to run instead of main")
    commands.add_parser("status", help="print the newest record")
    args = parser.parse_args(argv)
    if args.command == "setup":
        return setup(args)
    if not account.dedicated():
        print("refused: this is not the journey account (scripts/agent_journey/account.py)", file=sys.stderr)
        return 2
    if args.command == "request":
        return request(args)
    if args.command == "status":
        return status(args)
    settings = json.loads((journey_root() / "settings.json").read_text())
    code = run(settings)
    # A request made while that run went on starts one more, on its commit.
    while code is not None and (journey_root() / "request.json").is_file():
        code = run(settings)
    return code or 0
