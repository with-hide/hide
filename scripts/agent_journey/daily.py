"""The daily real-login journey on the journey account (account.py).

Once a day, inside the account's own login session, where its keychain is
open, a LaunchAgent runs `run`: it moves the account's one checkout to main
(or to the commit a `request` names), installs the CLIs contracts/agent-clis.json
pins, builds the journey's binaries while holding the machine's shared cargo
lock and no longer, runs every agent's journey on their real logins, and
replaces the body of one GitHub issue with what an agent reading it acts on:
each agent's CLI version and result, never an account name or screen text.
An agent with a failed cell answers one prompt outside Hide afterwards; when
it cannot, the failure is the agent's state (`login needed`, `usage limit`,
`update needed`, `agent error`), not Hide's. Everything else stays in the
run folder on that machine. docs/VERIFICATION.md, The daily real-login journey.
"""

import argparse
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import time

from . import account
from .cells import FAIL, NOT_MEASURED, PASS

CHECKOUT = Path(__file__).resolve().parents[2]
REPOSITORY = "with-hide/hide"
TITLE = "Real-login journey record"
MARKER = "<!-- real-login-journey-record -->"
LABEL = "dev.withhide.agent-journey"
# 05:00 on the machine's clock (Asia/Seoul on the mini): after the nightly
# (17:17 UTC, 02:17 KST) and before the morning check reads the record.
HOUR, MINUTE = 5, 0
KEEP_RUNS = 7
KEEP_LOGS = 14
# A kept target past this is removed before the build: one build of the
# journey's binaries is about TARGET_ESTIMATE, and what a day's build leaves
# behind accumulates.
TARGET_CAP = 30 * 1024 ** 3
LOCK_WAIT_SECONDS = 3 * 3600
BUILD_SECONDS = 90 * 60
JOURNEY_SECONDS = 150 * 60
CHECK_SECONDS = 180
SYSTEM_PATH = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
BUILDS = [["cli"], ["build", "-p", "hide-session", "--example", "journey_conversation"],
          ["build", "-p", "hide-memory", "--example", "journey_memory"],
          ["build", "-p", "hide-agent-adapter", "--example", "journey_composer"]]
# The one prompt an agent answers outside Hide, in its own folders, when a
# cell of its failed, and the arguments each CLI takes it with
# non-interactively.
CHECK_PROMPT = "Reply with the single word ok."
ONE_SHOT = {
    "claude": ["-p", CHECK_PROMPT],
    "codex": ["exec", "--skip-git-repo-check", CHECK_PROMPT],
    "opencode": ["run", CHECK_PROMPT],
    "pi": ["-p", CHECK_PROMPT],
    "omp": ["-p", CHECK_PROMPT],
    "grok": ["-p", CHECK_PROMPT],
    "agy": ["-p", CHECK_PROMPT],
}
# What an agent that could not answer outside Hide said, by kind, read on
# the machine only: the record keeps the kind, never the words. The first
# that matches wins, so a usage limit that says "upgrade" reads as a limit.
AGENT_STATES = [
    ("usage limit", re.compile(r"usage limit|rate.?limit|\b429\b|quota|too many requests|limit reached|"
                               r"out of credits|insufficient (?:credit|balance|funds)", re.I)),
    ("login needed", re.compile(r"log ?in|sign ?in|logged out|unauthori[sz]ed|\b401\b|\b403\b|authenticat|"
                                r"credential|expired|re-?auth|api key", re.I)),
    ("update needed", re.compile(r"update|upgrade|newer version|no longer supported|unsupported version", re.I)),
]
# The journey's own refusals before an agent starts that mean its login.
LOGIN_REFUSAL = re.compile(r"^(?:not logged in|login (?:expired|near refresh|expiry unreadable|refresh time))")


def journey_root():
    return Path.home() / "journey"


class Stage(Exception):
    """A whole-run failure, by a constant name the record shows."""


def log(stream, message):
    stream.write(f"{datetime.datetime.now().isoformat(timespec='seconds')} {message}\n")
    stream.flush()


def environment(settings, extra=None):
    """The run's environment: none of the launching shell's Hide or Herdr
    variables, the journey's CLIs first on PATH, then the toolchain."""
    root = journey_root()
    base = {key: value for key, value in os.environ.items()
            if not key.startswith(("HIDE_", "HERDR_")) and not key.endswith("_API_KEY")}
    path = [str(root / "bin"), settings["toolchain_bin"], *SYSTEM_PATH]
    return {**base, "PATH": os.pathsep.join(path), "LANG": "en_US.UTF-8", "PYTHONUTF8": "1",
            "CARGO_HOME": str(root / "cargo-home"), "RUSTUP_HOME": settings["rustup_home"], **(extra or {})}


def run_logged(argv, stream, env, cwd=CHECKOUT, timeout=None):
    log(stream, "$ " + " ".join(argv))
    return subprocess.run(argv, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=timeout).returncode


def checkout(target, stream, env):
    """Moves the checkout to `target` (a commit or origin/main); its commit."""
    ref = target or "main"
    if run_logged(["git", "fetch", "--quiet", "origin", ref], stream, env, timeout=600):
        raise Stage("checkout failed")
    if run_logged(["git", "checkout", "--quiet", "--detach", "FETCH_HEAD"], stream, env, timeout=120):
        raise Stage("checkout failed")
    # The run folders and the target stay; anything else the last commit left goes.
    run_logged(["git", "clean", "-fdq", "-e", "/agents/", "-e", "/target/"], stream, env, timeout=300)
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=CHECKOUT, capture_output=True, text=True,
                          check=True).stdout.strip()


def install_clis(stream, env):
    """The pinned CLIs, installed once per pin table into a folder of their
    own, linked from ~/journey/bin by name; an older table's folder goes."""
    root = journey_root()
    table = (CHECKOUT / "contracts/agent-clis.json").read_bytes()
    folder = root / "clis" / hashlib.sha256(table).hexdigest()[:16]
    installed = folder / "installed.json"
    if not installed.is_file():
        shutil.rmtree(folder, ignore_errors=True)
        folder.mkdir(parents=True)
        result = subprocess.run([sys.executable, "scripts/install-agent-clis.py", "--all", "--prefix", str(folder)],
                                cwd=CHECKOUT, env=env, capture_output=True, text=True, timeout=1800)
        stream.write(result.stdout + result.stderr)
        if result.returncode:
            raise Stage("CLI install failed")
        programs = dict(line.split("=", 1) for line in result.stdout.splitlines()
                        if re.fullmatch(r"[a-z]+=/.+", line))
        installed.write_text(json.dumps(programs))
    programs = json.loads(installed.read_text())
    links = root / "bin"
    links.mkdir(exist_ok=True)
    for kind, program in programs.items():
        link = links / kind
        link.unlink(missing_ok=True)
        link.symlink_to(program)
    for other in (root / "clis").iterdir():
        if other != folder:
            shutil.rmtree(other, ignore_errors=True)
    return programs


def size(path):
    total = 0
    for folder, _, files in os.walk(path):
        for name in files:
            with contextlib.suppress(OSError):
                total += os.lstat(os.path.join(folder, name)).st_size
    return total


def build(settings, stream, env):
    """The journey's binaries, built while holding the shared cargo lock."""
    target = CHECKOUT / "target"
    if target.is_dir() and size(target) > TARGET_CAP:
        log(stream, "target past its cap: removed")
        shutil.rmtree(target)
    script = " && ".join("bash scripts/verify-cargo.sh " + " ".join(step) for step in BUILDS)
    argv = ["/usr/bin/lockf", "-k", "-t", str(LOCK_WAIT_SECONDS), settings["cargo_lock"], "/bin/bash", "-c", script]
    try:
        if run_logged(argv, stream, env, timeout=LOCK_WAIT_SECONDS + BUILD_SECONDS):
            raise Stage("build failed")
    except subprocess.TimeoutExpired as error:
        raise Stage("build timed out") from error


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
        elif LOGIN_REFUSAL.match(row.get("login") or ""):
            entry["result"] = "login needed"
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


def check_outside(kind, stream, env):
    """None when the agent answers one prompt in its own folders, outside
    Hide; else the kind of its state."""
    program = journey_root() / "bin" / kind
    if kind not in ONE_SHOT or not program.exists():
        return "agent error"
    with tempfile.TemporaryDirectory(dir=journey_root()) as folder:
        try:
            result = subprocess.run([str(program), *ONE_SHOT[kind]], cwd=folder, env=env, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=CHECK_SECONDS)
        except subprocess.TimeoutExpired:
            log(stream, f"{kind}: no answer outside Hide in {CHECK_SECONDS} s")
            return "agent error"
    output = result.stdout + result.stderr
    stream.write(f"--- {kind} outside Hide, exit {result.returncode}\n{output[-4000:]}\n")
    if not result.returncode and re.search(r"\bok\b", result.stdout, re.I):
        return None
    return agent_state(output)


def recheck(agents, kinds, stream, env):
    """A failed agent that cannot answer outside Hide either carries its
    state, its failed cells kept beside it for the reader."""
    for entry in agents:
        if entry["result"] == "fail":
            state = check_outside(kinds[entry["id"]], stream, env)
            if state:
                entry["result"] = state


def markdown(record):
    lines = [MARKER, "",
             "The newest real-login journey on the Mac mini's journey account, replaced after each run by "
             "`scripts/agent-journey-daily.py` (docs/VERIFICATION.md, The daily real-login journey). "
             "Only agents, versions and cell results are here; each run's report and failure captures stay "
             "on that machine. The run needs that account's login session, which a restart of the machine ends: "
             "a record older than a day means someone has to log in to it again.", "",
             f"- Started: {record['started']}, {record.get('minutes', '?')} min",
             f"- Hide: {record.get('commit') or 'not checked out'} ({record['source']})",
             f"- Run: {record['run']}" + (f", journey exit {record['exit']}" if record.get("exit") is not None else ""),
             ""]
    if record.get("agents"):
        lines += ["| Agent | CLI | Result | Failed cells | Cells |", "| --- | --- | --- | --- | --- |"]
        for entry in record["agents"]:
            counts = ", ".join(f"{count} {status}" for status, count in sorted(entry["cells"].items()))
            lines.append(f"| {entry['id']} | {entry['cli'] or '?'} | {entry['result']} | "
                         f"{', '.join(entry['failed']) or '-'} | {counts} |")
        lines.append("")
    lines += ["```json", json.dumps(record, indent=1), "```", ""]
    return "\n".join(lines)


def gh(args, env, payload=None):
    result = subprocess.run(["gh", *args], env=env, input=json.dumps(payload) if payload is not None else None,
                            capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: exit {result.returncode}")
    return json.loads(result.stdout) if result.stdout.strip() else None


def publish(record, stream, env):
    """Replaces the record issue's body; makes the issue once."""
    root = journey_root()
    body = markdown(record)
    (root / "last-record.json").write_text(json.dumps(record, indent=1))
    remembered = root / "record-issue"
    number = remembered.read_text().strip() if remembered.is_file() else None
    try:
        if not number:
            found = gh(["api", "-X", "GET", f"repos/{REPOSITORY}/issues", "-f", "state=open", "-f", "per_page=100",
                        "-f", "creator=@me"], env) or []
            number = next((str(issue["number"]) for issue in found if issue["title"] == TITLE), None)
        if number:
            gh(["api", "-X", "PATCH", f"repos/{REPOSITORY}/issues/{number}", "--input", "-"], env, {"body": body})
        else:
            number = str(gh(["api", "-X", "POST", f"repos/{REPOSITORY}/issues", "--input", "-"], env,
                            {"title": TITLE, "body": body})["number"])
        remembered.write_text(number)
        log(stream, f"record: {REPOSITORY}#{number}")
    except (RuntimeError, subprocess.TimeoutExpired, ValueError, KeyError) as error:
        log(stream, f"record not published: {error}")


def ancestors():
    found, pid = set(), os.getpid()
    while pid > 1 and pid not in found:
        found.add(pid)
        answer = subprocess.run(["ps", "-o", "ppid=", "-p", str(pid)], capture_output=True, text=True)
        pid = int(answer.stdout.strip() or 1)
    return found


def cleanup(stream):
    """What a run of this account may have left: its processes started from
    the journey folder (a candidate hided, a pane holder, an agent's
    server), its files in /private/tmp, and run folders and logs past the
    newest few. The account runs nothing else."""
    root = journey_root()
    keep = ancestors()
    listing = subprocess.run(["ps", "-U", str(os.getuid()), "-o", "pid=,command="], capture_output=True, text=True)
    for line in listing.stdout.splitlines():
        pid, _, command = line.strip().partition(" ")
        if pid.isdigit() and int(pid) not in keep and str(root) in command:
            log(stream, f"ended leftover {pid}: {command[:160]}")
            with contextlib.suppress(ProcessLookupError, PermissionError):
                os.kill(int(pid), 9)
    for entry in Path("/private/tmp").iterdir():
        with contextlib.suppress(OSError):
            if entry.lstat().st_uid == os.getuid():
                log(stream, f"removed {entry}")
                if entry.is_dir() and not entry.is_symlink():
                    shutil.rmtree(entry, ignore_errors=True)
                else:
                    entry.unlink()
    for folder, keep_count in ((CHECKOUT / "agents/runs", KEEP_RUNS), (root / "logs", KEEP_LOGS)):
        if folder.is_dir():
            entries = sorted((entry for entry in folder.iterdir() if entry.name.startswith("daily-")),
                             key=lambda entry: entry.name, reverse=True)
            for old in entries[keep_count:]:
                if old.is_dir():
                    shutil.rmtree(old, ignore_errors=True)
                else:
                    old.unlink()


def run(settings):
    root = journey_root()
    (root / "logs").mkdir(parents=True, exist_ok=True)
    started = datetime.datetime.now().astimezone()
    stamp = started.strftime("%Y%m%dT%H%M%S")
    with open(root / "run.lock", "w") as lock, open(root / "logs" / f"daily-{stamp}.log", "w") as stream:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log(stream, "another run holds the journey; this one ends")
            return 0
        request = root / "request.json"
        target = json.loads(request.read_text()).get("commit") if request.is_file() else None
        request.unlink(missing_ok=True)
        record = {"format": 1, "started": started.isoformat(timespec="minutes"),
                  "source": f"request {target}" if target else "main", "run": "ok", "exit": None, "agents": []}
        env = environment(settings)
        try:
            record["commit"] = checkout(target, stream, env)
            install_clis(stream, env)
            build(settings, stream, env)
            run_dir = CHECKOUT / "agents/runs" / f"daily-{stamp}"
            try:
                record["exit"] = run_logged([sys.executable, "-X", "utf8", "scripts/agent-journey-check.py",
                                             "--run-dir", str(run_dir)], stream, env, timeout=JOURNEY_SECONDS)
            except subprocess.TimeoutExpired as error:
                raise Stage("journey timed out") from error
            report = run_dir / "report.json"
            if not report.is_file():
                raise Stage("journey left no report")
            data = json.loads(report.read_text())
            record["agents"] = summarize(data)
            kinds = {row["id"]: row["agent_cli"] for row in json.loads(
                (CHECKOUT / "contracts/agent-journey.json").read_text())}
            recheck(record["agents"], kinds, stream, env)
            if record["exit"] == 2:
                record["run"] = "setup or cleanup failure"
        except Stage as stage:
            record["run"] = str(stage)
            log(stream, f"stopped: {stage}")
        finally:
            record["minutes"] = round((time.time() - started.timestamp()) / 60)
            cleanup(stream)
            publish(record, stream, env)
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
    with open(root / "logs/setup.log", "a") as stream:
        print(json.dumps(install_clis(stream, environment(settings)), indent=1))
    print(f"set up; the LaunchAgent loads at this account's next login, or now with:\n"
          f"  launchctl bootstrap gui/{os.getuid()} {agents / (LABEL + '.plist')}")
    return 0


def request(args):
    """Asks the account's session to run now, on `--commit` when given: a
    run outside that session would find the keychain locked."""
    root = journey_root()
    (root / "request.json").write_text(json.dumps({"commit": args.commit}))
    return subprocess.run(["launchctl", "kickstart", f"gui/{os.getuid()}/{LABEL}"]).returncode


def status(args):
    record = journey_root() / "last-record.json"
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
    return run(json.loads((journey_root() / "settings.json").read_text()))
