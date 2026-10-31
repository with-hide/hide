#!/usr/bin/env python3
"""The holder restart lane (PRD engine-bundle-2-pane-holder D-21, B8;
docs/TESTING.md, The holder restart lane).

Each supported agent's real CLI, at the version contracts/agent-clis.json
pins, runs in a pane holder that a real node started, the node the daemon
builds (`Holders::start_local`, through `hide-node/examples/holder_node`),
answered by the agent journey's fake model with no key. For each agent:

1. the CLI starts in the holder with its first prompt, and the fake model
   receives that turn's token and the CLI draws its answer;
2. the node alone is killed outright;
3. a new node starts on the same state folder and attaches the pane again;
   the holder and every process of the CLI's tree are the same processes
   (pid and start time);
4. the new node's first full frame, drawn on a fresh xterm and on one that
   drew another screen first, is the holder's screen at that moment, as the
   holder's own read (a path apart from its frame encoder) gives it, judged
   only when no frame came between the two;
5. a second prompt with another token reaches the fake model and the CLI
   draws its answer.

Nothing waits a fixed time: each step waits for the fake model's log, the
holder's read or a frame. A cell that cannot run fails; the report names each
agent's system, CLI version, pin, result and reason, and the lane fails
unless every cell passed. Only the processes the lane started are ended, by
the pane's own end and then by pid and start time.

Usage: holder-restart-check.py --bin <folder with holder_node and hide-holder>
       --run-dir <new folder under agents/runs> [--agents id,...] [--herdr-bin <herdr>]

`--herdr-bin` names the pinned Herdr whose folder holds the ConPTY runtime a
Windows holder runs on; Herdr itself is never started. Each CLI is found on
PATH, where scripts/install-agent-clis.py put it.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import queue
import secrets
import shutil
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

ROWS, COLS = 40, 120
DRAW = CHECKOUT / "hide-holder/tests/xterm/draw.mjs"
# How long a CLI takes to start and answer its first turn, the longest
# step: a Windows runner's first start of an npm CLI took minutes.
FIRST_TURN = 300
TURN = 180
ATTACH = 60
# Restarts tried when the screen moved between the first full frame and
# the holder's read, each a new kill and a new node.
SCREEN_TRIES = 3
PASS, FAIL = "pass", "fail"


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

    def __init__(self, program, home, state, herdr_bin, log):
        # The runner's environment less any Herdr, Hide or provider
        # variable: a holder hands the node's environment to its program.
        env = {**clean_env(), **home_env(home)}
        env["HIDE_DEV_PANE_HOLDER"] = "1"
        argv = [str(program), "node", "--home", str(home), "--state", str(state)]
        if herdr_bin:
            argv += ["--herdr-bin", str(herdr_bin)]
        self.stderr = open(log, "ab")
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
                                        env=env, text=True, encoding="utf-8")
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
        """Kills the node outright, the way a crash ends it."""
        self.process.kill()
        self.process.wait(30)
        self.stderr.close()

    def quit(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.write(json.dumps({"op": "quit"}) + "\n")
                self.process.stdin.flush()
                self.process.wait(30)
            except (OSError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait(30)
        self.stderr.close()


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
    tree that had children before the kill still run as the same processes,
    else what moved; and the leaf processes that ended, which a CLI's helper
    such as omp's `gh` or a shell's `grep` does on its own). A leaf is not held
    to the restart because nothing tells a helper's own end from a kill's;
    the CLI itself is the program, or a process its children hang from, and
    the second turn shows it still answers."""
    if before["holder"] != after["holder"]:
        return f"the holder moved: {before['holder']} -> {after['holder']}", []
    if before["program"] != after["program"]:
        return f"the pane's program moved: {before['program']} -> {after['program']}", []
    now = {(process["pid"], process["started"]) for process in after["processes"]}
    program = before["program"]["pid"]
    parents = {process["parent"] for process in before["processes"]}
    gone = [process for process in before["processes"] if (process["pid"], process["started"]) not in now]
    held = [process for process in gone if process["pid"] == program or process["pid"] in parents]
    leaves = [f"{process['name']} {process['pid']}" for process in gone if process not in held]
    if held:
        return "processes of the CLI ended across the restart: " + ", ".join(
            f"{process['name']} {process['pid']}" for process in held), leaves
    return None, leaves


def cell(row, binaries, herdr_bin, run):
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
    runtime = Runtime(root)
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
        first, second = token(), token()
        argv = [str(launch), row["prompt_flag"], prompt(first)]
        if WINDOWS:
            # A .cmd launcher runs under cmd.exe, which a ConPTY start does not add.
            argv = [os.environ.get("COMSPEC", "cmd.exe"), "/d", "/c", *argv]
        pane = kind
        node = Node(binaries / exe("holder_node"), runtime.home, runtime.state, herdr_bin, run / f"node-{row['id']}.log")
        started = time.monotonic()
        node.ask("start", pane=pane, program=argv, cwd=str(repo), env={}, rows=ROWS, cols=COLS)
        first_screen = node.ask("wait_full", pane=pane, since=0, timeout_ms=ATTACH * 1000)["hex"]
        wait_turn(log, first, FIRST_TURN)
        node.ask("wait_text", pane=pane, text=first, absent="Reply_", lines=ROWS, timeout_ms=TURN * 1000)
        record["first_turn_seconds"] = round(time.monotonic() - started, 1)
        for attempt in range(1, SCREEN_TRIES + 1):
            before = node.ask("tree", pane=pane)
            node.kill()
            node = Node(binaries / exe("holder_node"), runtime.home, runtime.state, herdr_bin,
                        run / f"node-{row['id']}.log")
            full = node.ask("wait_full", pane=pane, since=0, timeout_ms=ATTACH * 1000)
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
        text = prompt(second)
        node.ask("key", pane=pane, hex=text.encode().hex())
        node.ask("wait_text", pane=pane, text=text, lines=ROWS, timeout_ms=TURN * 1000)
        node.ask("key", pane=pane, hex=b"\r".hex())
        wait_turn(log, second, TURN)
        node.ask("wait_text", pane=pane, text=second, absent="Reply_", lines=ROWS, timeout_ms=TURN * 1000)
        record["states"] = node.ask("states", pane=pane)["states"]
        record["result"] = PASS
        record["reason"] = "the CLI kept its processes, the screen and its second turn across a node restart"
    except (Failed, OSError, subprocess.SubprocessError, ValueError) as error:
        record["reason"] = str(error)
    finally:
        record["cleanup"] = clean(node, kind, service, root)
        server.shutdown()
    return record


def clean(node, pane, service, root):
    """Ends the pane, then what of the lane's still runs; answers what was
    left, which fails the cell."""
    left = []
    if node is not None and node.process.poll() is None:
        try:
            ended = node.ask("terminate", pane=pane, wait=90)
            if ended["record"].get("phase") != "ended":
                left.append(f"pane ended in phase {ended['record'].get('phase')}")
        except Failed as error:
            left.append(f"the pane did not end: {error}")
        node.quit()
    if service is not None:
        launch, environment, port, mark = service
        stopped = opencode_service.stop(launch, environment, port, mark, Owner())
        left.extend(str(item) for item in stopped.get("left", []))
    shutil.rmtree(root, ignore_errors=True)
    return {"left": left}


def exe(name):
    return name + (".exe" if WINDOWS else "")


def system():
    return {"os": platform.system(), "release": platform.release(), "machine": platform.machine()}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--agents", default="")
    parser.add_argument("--herdr-bin", type=Path)
    args = parser.parse_args(argv)
    binaries = args.bin.resolve()
    for name in ("holder_node", "hide-holder"):
        if not (binaries / exe(name)).is_file():
            raise SystemExit(f"{binaries / exe(name)} is missing: copy the built holder and the example there")
    if WINDOWS and not args.herdr_bin:
        raise SystemExit("--herdr-bin is required on Windows: the holder runs on the ConPTY runtime beside it")
    run = args.run_dir.absolute()
    if run.exists():
        raise SystemExit(f"{run} exists: the run directory must be new")
    private_directory(run)
    rows = json.loads((CHECKOUT / "contracts/agent-journey.json").read_text())
    selected = [row for row in rows if not args.agents or row["id"] in args.agents.split(",")]
    report = {"format": 1, "lane": "holder-restart", "system": system(),
              "started": datetime.datetime.now(datetime.timezone.utc).isoformat(), "cells": []}
    for row in selected:
        print(f"== {row['id']}", flush=True)
        record = cell(row, binaries, args.herdr_bin, run)
        if record["result"] == PASS and record["cleanup"]["left"]:
            record["result"] = FAIL
            record["reason"] = "left running: " + "; ".join(record["cleanup"]["left"])
        report["cells"].append(record)
        print(f"   {record['result']}: {record.get('version', '?')}: {record.get('reason')}", flush=True)
        (run / "report.json").write_text(json.dumps(report, indent=2))
    failed = [cell["agent"] for cell in report["cells"] if cell["result"] != PASS]
    report["passed"] = not failed and len(report["cells"]) == len(selected)
    (run / "report.json").write_text(json.dumps(report, indent=2))
    lines = [f"| {cell['agent']} | {report['system']['os']} | {cell.get('version', '-')} | {cell['result']} |"
             for cell in report["cells"]]
    (run / "report.md").write_text("| agent | system | CLI | result |\n| --- | --- | --- | --- |\n"
                                   + "\n".join(lines) + "\n")
    if failed:
        print("failed: " + ", ".join(failed), flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
