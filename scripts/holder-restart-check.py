#!/usr/bin/env python3
"""The holder restart lane (PRD engine-bundle-2-pane-holder D-21, B8;
docs/TESTING.md, The holder restart lane).

Each supported agent's real CLI, at the version contracts/agent-clis.json
pins, runs in a pane holder that a real node started, the node the daemon
builds (`Holders::start_local`, through `hide-node/examples/holder_node`),
answered by the agent journey's fake model with no key. For each agent:

1. the first node removes an older build's holder version folder nothing
   runs from and logs it (`pane_holder.folders_pruned`, B20), then the CLI
   starts in the holder with its first prompt, and the fake model
   receives that turn's token and the CLI draws its answer;
2. the node alone is killed outright;
3. a new node starts on the same state folder and attaches the pane again;
   the holder and the CLI are the same processes (pid and start time), and
   a process of its tree that ended is only a helper nothing running
   descends from;
4. the new node's first full frame, drawn on a fresh xterm and on one that
   drew another screen first, is the holder's screen at that moment, as the
   holder's own read (a path apart from its frame encoder) gives it, judged
   only when no frame came between the two;
5. a second prompt with another token reaches the fake model and the CLI
   draws its answer.

Nothing waits a fixed time: each step waits for the fake model's log, the
holder's read or a frame. A cell that cannot run fails; the report names each
agent's system, CLI version, pin, result and reason, and the lane fails
unless every cell passed. A CLI whose version is not the pinned one fails its
cell. Only the processes the lane started are ended, by the pane's own end
and then, on every path, a stop signal included, by pid and start time
(`holder_owned.Owned`); a cell fails when any of them is left running.

Usage: holder-restart-check.py --bin <folder with holder_node and hide-holder>
       --run-dir <new folder under agents/runs> [--agents id,...]

On Windows the binaries folder also holds the pinned ConPTY runtime a holder
runs on, in `holder-conpty` (`scripts/fetch-conpty.py --out
<folder>/holder-conpty`), where the node finds it beside its holder as hided
does in the app. Herdr is
never started. Each CLI is found on PATH, where
scripts/install-agent-clis.py put it.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import queue
import secrets
import subprocess
import sys
import tempfile
import threading
import time

CHECKOUT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CHECKOUT / "scripts"))

from agent_journey import ci, clis, fake_lane, fake_model, opencode_service  # noqa: E402
from agent_journey.local import TRUST  # noqa: E402
from agent_live_check.protection import private_directory, write_private  # noqa: E402
from agent_live_check.runtime import clean_env  # noqa: E402
from agent_live_check.system import WINDOWS, home_env, temp_env  # noqa: E402
import holder_owned  # noqa: E402

ROWS, COLS = 40, 120
DRAW = CHECKOUT / "hide-holder/tests/xterm/draw.mjs"
# How long a CLI takes to start and answer its first turn, the longest
# step: a Windows runner's first start of an npm CLI took minutes.
FIRST_TURN = 300
TURN = 180
ATTACH = 60
# Restarts tried when the screen moved between the first full frame and
# the holder's read, each a new kill and a new node.
SCREEN_TRIES = 5
# How long a pane draws nothing before the node is killed, and how long a
# frame may take to follow a change: a holder sends one at most 16 ms after
# the last, so a change made before the read has sent its frame well within it.
QUIET_MS = 500
FOLLOW_MS = 1000
PASS, FAIL = "pass", "fail"
# An older build's holder version folder nothing runs from, put in the home
# before the first node starts, which must remove it (B20); a prune waits
# at most 30 s for starts that hold the folders.
OLD_HOLDER_FOLDER = "v0-" + "0" * 16
PRUNE = 60
# Cells whose failure is open as an issue, keyed by the agent's id in
# contracts/agent-journey.json and `platform.system()`: each still runs and
# shows its result, and its failure, whatever its cause, does not fail the
# lane unless it left a process running. A key that names no agent of the
# contract stops the run before any cell. An entry is removed in the pull
# request that closes its issue.
KNOWN_FAILURES = {
    # Antigravity on Windows failed one run in ten: it read the holder's
    # answer to its color question as typed keys and opened its editor.
    ("antigravity", "Windows"): "with-hide/hide-private#405",
}


def token():
    return "JOURNEY-" + secrets.token_hex(4).upper()


def prompt(value):
    """One word, so no shell or cmd.exe between the pane and the CLI can
    split or requote it; the fake model answers the token alone, so a line
    with the token and without `Reply_` is the answer."""
    return "Reply_exactly_" + value


class Failed(Exception):
    pass


class Owner:
    """What opencode_service.stop runs its commands with."""

    def run(self, argv, env=None, check=False):
        done = subprocess.run([str(word) for word in argv], env=env, capture_output=True, text=True, timeout=60)
        if check and done.returncode:
            raise Failed(f"{argv[0]} exited {done.returncode}")
        return done.returncode, done.stdout, done.stderr


class Node:
    """A `holder_node node` process and its request line."""

    def __init__(self, program, home, state, log, owned):
        # The runner's environment less any Herdr, Hide or provider
        # variable: a holder hands the node's environment to its program.
        env = {**clean_env(), **home_env(home)}
        argv = [str(program), "node", "--home", str(home), "--state", str(state)]
        if WINDOWS:
            # Through libuv's job, as the desktop app starts its daemon: a
            # job the runner's own (which lets no process leave it) is
            # outside of (holder-restart-spawn.mjs).
            argv = ["node", str(CHECKOUT / "scripts/holder-restart-spawn.mjs"), *argv]
        self.stderr = open(log, "ab")
        try:
            self.process = owned.spawn(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
                                       env=env, text=True, encoding="utf-8")
        finally:
            self.stderr.close()
        self.lines = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        ready = self._answer(30)
        self.pid = ready["ready"]

    def _read(self):
        for line in self.process.stdout:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict) and "ok" in value:
                self.lines.put(value)
        self.lines.put(None)

    def _answer(self, seconds):
        try:
            value = self.lines.get(timeout=seconds)
        except queue.Empty:
            raise Failed(f"the node answered nothing in {seconds} s") from None
        if value is None:
            raise Failed(f"the node exited ({self.process.wait()})")
        if not value["ok"]:
            raise Failed(value.get("error", "the node refused"))
        return value

    def ask(self, op, wait=60, **fields):
        timeout = fields.get("timeout_ms")
        self.process.stdin.write(json.dumps({"op": op, **fields}) + "\n")
        self.process.stdin.flush()
        return self._answer(wait + (timeout or 0) / 1000)

    def kill(self):
        """Kills the node outright, the way a crash ends it; on Windows the
        starter, whose job's close ends the node."""
        self.process.kill()
        self.process.wait(30)

    def quit(self):
        """Asks the node to end; one that does not is ended by its Owned."""
        if self.process.poll() is None:
            try:
                self.process.stdin.write(json.dumps({"op": "quit"}) + "\n")
                self.process.stdin.flush()
                self.process.wait(30)
            except (OSError, subprocess.TimeoutExpired):
                pass


def turns(log, value):
    """The fake model's generation requests that answered `value`."""
    if not log.is_file():
        return 0
    found = 0
    for line in log.read_text(encoding="utf-8").splitlines():
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        if entry.get("generation") and entry.get("answered") == value:
            found += 1
    return found


def wait_turn(log, value, seconds):
    """Waits for the fake model to have answered `value`."""
    deadline = time.monotonic() + seconds
    while turns(log, value) == 0:
        if time.monotonic() > deadline:
            raise Failed(f"the fake model never received the turn of {value} in {seconds} s")
        time.sleep(0.2)


def plant_old_holder_folder(home):
    """An older build's version folder, as a build of Hide before this one
    leaves under the home; nothing runs from it."""
    folder = home / ".hide/holders" / OLD_HOLDER_FOLDER
    private_directory(folder)
    write_private(folder / "hide-holder", b"an older holder")
    return folder


def wait_pruned(log, folder, seconds):
    """The node's `pane_holder.folders_pruned` record (its diagnostic log is
    its error stream, `log`) that names `folder` removed; fails when it
    names a failure, or when none comes in `seconds`."""
    deadline = time.monotonic() + seconds
    while True:
        text = log.read_text(encoding="utf-8", errors="replace") if log.is_file() else ""
        for line in text.splitlines():
            try:
                entry = json.loads(line)
            except ValueError:
                continue
            if not isinstance(entry, dict) or entry.get("kind") != "pane_holder.folders_pruned":
                continue
            if entry.get("failed"):
                raise Failed(f"the node failed to remove a holder version folder: {line}")
            if folder.name in (entry.get("removed") or []):
                if folder.exists():
                    raise Failed(f"the node logged {folder.name} removed and it is still there")
                return entry
        if time.monotonic() > deadline:
            raise Failed(f"the node logged no removal of the older holder folder {folder.name} in {seconds} s")
        time.sleep(0.2)


def draw(screens):
    """Each screen's rows and soft wraps, drawn on headless xterms
    (hide-holder/tests/xterm/draw.mjs)."""
    document = {"terminals": [{"cols": COLS, "rows": ROWS, "steps": [{"write": step} for step in steps]}
                              for steps in screens]}
    done = subprocess.run(["node", str(DRAW)], input=json.dumps(document), capture_output=True, text=True,
                          encoding="utf-8", timeout=120)
    if done.returncode:
        raise Failed(f"the xterm could not draw: {done.stderr.strip()[-400:]}")
    return json.loads(done.stdout)["screens"]


def logical(screen):
    """The screen's lines with soft wraps joined, as a holder's read joins
    them, trailing spaces and blank rows at the end left out; and whether the
    first line continues a row above the screen."""
    lines = []
    for text, wrapped in zip(screen["lines"], screen["wrapped"]):
        if wrapped and lines:
            lines[-1] += text
        else:
            lines.append(text)
    lines = [line.rstrip() for line in lines]
    while lines and not lines[-1]:
        lines.pop()
    return lines, bool(screen["wrapped"] and screen["wrapped"][0])


def compare(screen, read):
    """None when the drawn screen is the holder's read; else what differs."""
    lines, continued = logical(screen)
    rows = [line.rstrip() for line in read.split("\n")]
    while rows and not rows[-1]:
        rows.pop()
    if not lines:
        return None if not rows else "the frame drew nothing and the holder holds text"
    tail = rows[-len(lines):]
    if len(tail) < len(lines):
        return f"the holder's read has {len(rows)} lines and the frame draws {len(lines)}"
    for index, (drawn, held) in enumerate(zip(lines, tail)):
        same = held.endswith(drawn) if index == 0 and continued else drawn == held
        if not same:
            return f"line {index} differs: frame {drawn!r}, holder {held!r}"
    return None


def trust_agy(home, repo):
    """agy's trust of the folder it starts in, the entry Hide's kit records
    for a worktree it makes (hide-agent-hooks folder_trust.rs): without it
    agy asks whether to trust the folder before it reads the prompt."""
    settings = home / ".gemini/antigravity-cli/settings.json"
    document = json.loads(settings.read_text()) if settings.is_file() else {}
    document.setdefault("trustedWorkspaces", []).append(str(repo))
    replacement = settings.with_name(settings.name + ".lane")
    replacement.unlink(missing_ok=True)
    write_private(replacement, json.dumps(document).encode())
    replacement.replace(settings)


def repository(root):
    """The folder each CLI starts in: a git repository of one commit."""
    folder = root / "repo"
    folder.mkdir()
    (folder / "README.md").write_text("holder restart lane\n")
    env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    for argv in (["init", "-q"], ["add", "README.md"],
                 ["-c", "user.name=lane", "-c", "user.email=lane@invalid", "commit", "-q", "-m", "start"]):
        subprocess.run(["git", "-C", str(folder), *argv], check=True, env=env, capture_output=True, timeout=60)
    return folder.resolve()


class Runtime:
    """What ci.wrapper writes a CLI's launcher with."""

    def __init__(self, root):
        self.run = root
        self.home = root / "h"
        self.state = self.home / ".hide/state"
        self.bin = root / "bin"


def same_processes(before, after):
    """(None when the holder, the pane's program and every process of its
    tree that something still running hangs from run as the same processes
    as before the kill, else what moved; and the processes that ended with
    everything under them, a CLI's helpers such as omp's `gh`, OpenCode's
    `git` or a shell's `grep`, which end on their own). The CLI is the pane's
    program; the second turn shows it still answers."""
    if before["holder"] != after["holder"]:
        return f"the holder moved: {before['holder']} -> {after['holder']}", []
    # The node's word checked against the process table's own.
    holder = after["holder"]
    if not holder_owned.runs(holder["pid"], holder["started"]):
        return (f"the holder {holder['pid']} does not run with the start time {holder['started']} "
                f"(the process table reads {holder_owned.started(holder['pid'])})"), []
    if before["program"] != after["program"]:
        return f"the pane's program moved: {before['program']} -> {after['program']}", []
    now = {(process["pid"], process["started"]) for process in after["processes"]}
    if (before["program"]["pid"], before["program"]["started"]) not in now:
        return f"the pane's program {before['program']['pid']} ended across the restart", []
    parent = {process["pid"]: process["parent"] for process in before["processes"]}
    alive = [process["pid"] for process in before["processes"] if (process["pid"], process["started"]) in now]
    above_alive = set()
    for pid in alive:
        while pid in parent and parent[pid] not in above_alive:
            pid = parent[pid]
            above_alive.add(pid)
    gone = [process for process in before["processes"] if (process["pid"], process["started"]) not in now]
    held = [process for process in gone if process["pid"] in above_alive]
    helpers = [f"{process['name']} {process['pid']}" for process in gone if process not in held]
    if held:
        return "processes of the CLI ended across the restart: " + ", ".join(
            f"{process['name']} {process['pid']}" for process in held), helpers
    return None, helpers


def cell(row, binaries, run):
    """One agent's cell: its record, whatever happens."""
    kind = row["herdr_kind"]
    record = {"agent": row["id"], "kind": kind, "system": system(), "result": FAIL}
    program = ci.executable(None, kind)
    if not program:
        record["reason"] = f"CLI not installed: {kind} is not on PATH"
        return record
    # A short root: a holder's socket is a path under the state folder, and
    # a Unix socket's path is at most about 100 bytes.
    root = Path(tempfile.mkdtemp(prefix="hrl-", dir=None if WINDOWS else "/tmp")).resolve()
    owned = holder_owned.Owned()
    owned.remove(root)
    runtime = Runtime(root)
    owned.records(runtime.state)
    private_directory(runtime.home)
    log = run / f"fake-model-{row['id']}.jsonl"
    server, _ = fake_model.serve(log, row["request_budget"])
    url = f"http://127.0.0.1:{server.server_address[1]}"
    node = None
    service = None
    try:
        repo = repository(root)
        (root / "agent-tmp").mkdir()
        private_directory(runtime.home.joinpath(*row["home_folder"]))
        fake_env = fake_lane.configure(kind, runtime.home, url, repo, local=False)
        if kind in TRUST:
            path, write = TRUST[kind]
            write(runtime.home / path, repo)
        if kind == "agy":
            trust_agy(runtime.home, repo)
        launch = ci.wrapper(runtime, program, kind, os.environ.get("PATH", ""), None, fake_env)
        environment = {**clean_env(), **home_env(runtime.home), **temp_env(runtime.run / "agent-tmp")}
        if kind == "opencode":
            port = opencode_service.configure(runtime.home / opencode_service.CONFIG)
            service = (launch, {**environment, **opencode_service.environment(runtime.home)}, port,
                       str(runtime.home))
        version = subprocess.run([str(launch), "--version"], env=environment, capture_output=True, text=True,
                                 timeout=120)
        record["version"] = version.stdout.strip()[:120]
        record["pin"] = clis.pin(kind, record["version"])
        if record["pin"] is not None and not record["pin"]["matches"]:
            raise Failed(f"{kind} {record['version']} is not the pinned {record['pin']['version']}")
        first, second = token(), token()
        argv = [str(launch), *(part.replace("{}", prompt(first)) for part in row["prompt_form"])]
        if WINDOWS:
            # A .cmd launcher runs under cmd.exe, which a ConPTY start does not add.
            argv = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *argv]
        pane = kind
        old_folder = plant_old_holder_folder(runtime.home)
        node_log = run / f"node-{row['id']}.log"
        node = Node(binaries / exe("holder_node"), runtime.home, runtime.state, node_log, owned)
        record["pruned"] = wait_pruned(node_log, old_folder, PRUNE)
        started = time.monotonic()
        holder = node.ask("start", pane=pane, program=argv, cwd=str(repo), env={}, rows=ROWS, cols=COLS)
        holder = holder["record"]["holder"]
        owned.adopt(holder["pid"], holder["started"])
        if not holder_owned.runs(holder["pid"], holder["started"]):
            raise Failed(f"the holder {holder['pid']} does not run with the start time its record names "
                         f"({holder['started']}; the process table reads {holder_owned.started(holder['pid'])})")
        first_screen = node.ask("wait_full", pane=pane, since=0, timeout_ms=ATTACH * 1000)["hex"]
        wait_turn(log, first, FIRST_TURN)
        node.ask("wait_text", pane=pane, text=first, absent="Reply_", lines=ROWS, timeout_ms=TURN * 1000)
        record["first_turn_seconds"] = round(time.monotonic() - started, 1)
        for attempt in range(1, SCREEN_TRIES + 1):
            record.setdefault("quiet_before_kill", []).append(
                node.ask("wait_quiet", pane=pane, quiet_ms=QUIET_MS, timeout_ms=30_000)["quiet"])
            before = node.ask("tree", pane=pane)
            node.kill()
            killed = time.monotonic()
            node = Node(binaries / exe("holder_node"), runtime.home, runtime.state,
                        run / f"node-{row['id']}.log", owned)
            full = node.ask("wait_full", pane=pane, since=0, timeout_ms=ATTACH * 1000)
            record["reattach_seconds"] = round(time.monotonic() - killed, 2)
            held = node.ask("read", pane=pane, lines=ROWS)["text"]
            after_read = node.ask("frames", pane=pane)["count"]
            after = node.ask("tree", pane=pane)
            moved, leaves = same_processes(before, after)
            record.setdefault("restarts", []).append({"before": before, "after": after, "helpers_ended": leaves})
            if moved:
                raise Failed(moved)
            if after_read > full["index"] + 1:
                record.setdefault("screen_moved", []).append(attempt)
                continue
            fresh, stale = draw([[full["hex"]], [first_screen, full["hex"]]])
            differs = compare(fresh, held)
            if differs and node.ask("wait_more", pane=pane, count=full["index"] + 1,
                                    timeout_ms=FOLLOW_MS)["count"] > full["index"] + 1:
                # The program drew between the frame and the read (a
                # spinner): another restart, another first frame.
                record.setdefault("screen_moved", []).append(attempt)
                continue
            if differs:
                record["frame_lines"] = fresh["lines"]
                record["holder_read"] = held
                raise Failed("the first full frame after the restart is not the holder's screen: " + differs)
            if (fresh["lines"], fresh["x"], fresh["y"]) != (stale["lines"], stale["x"], stale["y"]):
                raise Failed("the first full frame draws another screen on a terminal that drew another first")
            record["screen_attempt"] = attempt
            record["screen_lines_compared"] = len(logical(fresh)[0])
            break
        else:
            raise Failed(f"the screen moved between the frame and the read in {SCREEN_TRIES} restarts")
        typed = time.monotonic()
        text = prompt(second)
        node.ask("key", pane=pane, hex=text.encode().hex())
        node.ask("wait_text", pane=pane, text=text, lines=ROWS, timeout_ms=TURN * 1000)
        node.ask("key", pane=pane, hex=b"\r".hex())
        wait_turn(log, second, TURN)
        node.ask("wait_text", pane=pane, text=second, absent="Reply_", lines=ROWS, timeout_ms=TURN * 1000)
        record["second_turn_seconds"] = round(time.monotonic() - typed, 1)
        record["states"] = node.ask("states", pane=pane)["states"]
        record["result"] = PASS
        record["reason"] = "the CLI kept its processes, the screen and its second turn across a node restart"
    except (Failed, OSError, subprocess.SubprocessError, ValueError) as error:
        record["reason"] = str(error)
    finally:
        try:
            record["cleanup"] = clean(node, kind, service, owned)
        finally:
            server.shutdown()
    return record


def clean(node, pane, service, owned):
    """Ends the pane, then what of the lane's still runs, by pid and start
    time; answers what was left, which fails the cell. A stopped run skips
    the polite end, which can wait minutes, and ends everything at once."""
    left = []
    if node is not None and node.process.poll() is None and not holder_owned.aborted():
        try:
            ended = node.ask("terminate", pane=pane, wait=90)
            if ended["record"].get("phase") != "ended":
                left.append(f"pane ended in phase {ended['record'].get('phase')}")
        except Failed as error:
            left.append(f"the pane did not end: {error}")
        node.quit()
    if service is not None and not holder_owned.aborted():
        launch, environment, port, mark = service
        try:
            stopped = opencode_service.stop(launch, environment, port, mark, Owner())
            left.extend(str(item) for item in stopped.get("left", []))
        except (Failed, OSError, subprocess.SubprocessError) as error:
            left.append(f"the OpenCode server did not stop: {error}")
    left.extend(owned.close())
    return {"left": left}


def run_cells(selected, binaries, run, report):
    for row in selected:
        print(f"== {row['id']}", flush=True)
        record = cell(row, binaries, run)
        if record["result"] == PASS and record["cleanup"]["left"]:
            record["result"] = FAIL
            record["reason"] = "left running: " + "; ".join(record["cleanup"]["left"])
        issue = KNOWN_FAILURES.get((row["id"], platform.system()))
        if record["result"] != PASS and issue and not record["cleanup"]["left"]:
            # Shown in the table and the report; a cell that left a process
            # running fails the lane whatever is known of it.
            record["known_failure"] = issue
        report["cells"].append(record)
        print(f"   {record['result']}: {record.get('version', '?')}: {record.get('reason')}", flush=True)
        (run / "report.json").write_text(json.dumps(report, indent=2))


def failed_cells(report):
    """The cells that fail the lane: a known failure still shows its cell,
    and only the others decide."""
    return [cell["agent"] for cell in report["cells"] if cell["result"] != PASS and not cell.get("known_failure")]


def unknown_known_failures(rows):
    """KNOWN_FAILURES keys that name no agent of the contract, which would
    never mark a cell."""
    ids = {row["id"] for row in rows}
    return sorted(f"{agent} on {os_name}" for agent, os_name in KNOWN_FAILURES if agent not in ids)


def exe(name):
    return name + (".exe" if WINDOWS else "")


def system():
    return {"os": platform.system(), "release": platform.release(), "machine": platform.machine()}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--agents", default="")
    args = parser.parse_args(argv)
    binaries = args.bin.resolve()
    for name in ("holder_node", "hide-holder"):
        if not (binaries / exe(name)).is_file():
            raise SystemExit(f"{binaries / exe(name)} is missing: copy the built holder and the example there")
    if WINDOWS and not (binaries / "holder-conpty").is_dir():
        raise SystemExit(f"{binaries / 'holder-conpty'} is missing: a Windows holder runs on the pinned ConPTY runtime beside it (scripts/fetch-conpty.py --out)")
    run = args.run_dir.absolute()
    if run.exists():
        raise SystemExit(f"{run} exists: the run directory must be new")
    private_directory(run)
    rows = json.loads((CHECKOUT / "contracts/agent-journey.json").read_text())
    unknown = unknown_known_failures(rows)
    if unknown:
        raise SystemExit("KNOWN_FAILURES names no agent of contracts/agent-journey.json: " + ", ".join(unknown))
    selected = [row for row in rows if not args.agents or row["id"] in args.agents.split(",")]
    holder_owned.abort_on_signals()
    report = {"format": 1, "lane": "holder-restart", "system": system(),
              "started": datetime.datetime.now(datetime.timezone.utc).isoformat(), "cells": []}
    try:
        run_cells(selected, binaries, run, report)
    except holder_owned.Aborted as stop:
        report["aborted"] = str(stop)
        report["passed"] = False
        (run / "report.json").write_text(json.dumps(report, indent=2))
        print(f"aborted: {stop}", flush=True)
        return 128 + stop.signum
    failed = failed_cells(report)
    report["passed"] = not failed and len(report["cells"]) == len(selected)
    (run / "report.json").write_text(json.dumps(report, indent=2))
    lines = [f"| {cell['agent']} | {report['system']['os']} | {cell.get('version', '-')} | {cell['result']}"
             + (f" (known failure, {cell['known_failure']})" if cell.get("known_failure") else "") + " |"
             for cell in report["cells"]]
    (run / "report.md").write_text("| agent | system | CLI | result |\n| --- | --- | --- | --- |\n"
                                   + "\n".join(lines) + "\n")
    known = [f"{cell['agent']} ({cell['known_failure']})" for cell in report["cells"] if cell.get("known_failure")]
    if known:
        print("known failures, not counted: " + ", ".join(known), flush=True)
    if failed:
        print("failed: " + ", ".join(failed), flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
