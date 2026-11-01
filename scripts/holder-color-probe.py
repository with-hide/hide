#!/usr/bin/env python3
"""How long a pane's color question (OSC 11) takes to be answered, on the
pane holder's path and on Herdr's, measured by the program on its own clock
(PRD engine-bundle-2-pane-holder D-21; docs/TESTING.md, The holder restart
lane).

A CLI that detects its theme asks once and waits a short while; an answer
that comes after it stopped waiting is read as typed keys, and its closing
BEL as Ctrl+G. `holder-color-probe.mjs` asks in raw mode, with nothing
before the question and right after 64 KiB of output, and records each
answer's delay or that none came within 2 s. This runs it in a holder a real
node started (the restart lane's node) and in a pane of a private Herdr
server of the pinned build, and writes both to `<run-dir>/color-probe.json`.
It measures; it fails only when a probe could not run, or when anything it
started is left running: the node, the holder and the Herdr server with its
panes are ended by pid and start time on every path (`holder_owned.Owned`).

Usage: holder-color-probe.py --bin <folder with holder_node and hide-holder>
       --herdr <pinned herdr> --run-dir <folder> [--count N]
"""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

CHECKOUT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(CHECKOUT / "scripts"))

from agent_live_check.runtime import clean_env  # noqa: E402
from agent_live_check.system import WINDOWS, exe, home_env, temp_env  # noqa: E402
import holder_owned  # noqa: E402

PROBE = CHECKOUT / "scripts/holder-color-probe.mjs"
DONE_SECONDS = 120


def restart_check():
    """The restart lane's module, for its node client."""
    spec = importlib.util.spec_from_file_location("holder_restart_check", CHECKOUT / "scripts/holder-restart-check.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def wait_file(path, seconds, alive):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if path.exists() and path.stat().st_size:
            return json.loads(path.read_text())
        if not alive():
            break
        time.sleep(0.2)
    raise SystemExit(f"the probe wrote nothing to {path}")


def holder_path(root, binaries, herdr, node_program, count, owned):
    lane = restart_check()
    home, state = root / "h", root / "h/s"
    home.mkdir()
    owned.records(state)
    out = root / "holder.json"
    node = lane.Node(binaries / exe("holder_node"), home, state, herdr, root / "node.log", owned)
    try:
        started = node.ask("start", pane="probe", program=[node_program, str(PROBE), str(out), str(count)],
                           cwd=str(root), env={}, rows=40, cols=120)
        holder = started["record"]["holder"]
        owned.adopt(holder["pid"], holder["started"])
        return wait_file(out, DONE_SECONDS, lambda: node.process.poll() is None)
    finally:
        if not holder_owned.aborted() and node.process.poll() is None:
            try:
                node.ask("terminate", pane="probe", wait=60)
            except lane.Failed as error:
                print(f"the probe pane's end: {error}", file=sys.stderr)
            node.quit()


def herdr_path(root, herdr, node_program, count, owned):
    env = {**clean_env(), **home_env(root / "hh"), **temp_env(root)}
    for name in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        folder = root / ("herdr-" + name.lower())
        folder.mkdir()
        env[name] = str(folder)
    (root / "hh").mkdir()
    env.update(HERDR_SESSION=f"color-probe-{os.getpid()}", HERDR_CONFIG_PATH=str(root / "herdr.toml"),
               HERDR_SOCKET_PATH=str(root / "herdr.sock"))
    with open(root / "herdr-server.out", "w") as log:
        server = owned.spawn([str(herdr), "server"], env=env, stdin=subprocess.DEVNULL, stdout=log,
                             stderr=subprocess.STDOUT,
                             **({"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP} if WINDOWS
                                else {"start_new_session": True}))

    def cli(*args):
        done = subprocess.run([str(herdr), *args], env=env, capture_output=True, text=True, timeout=60)
        if done.returncode:
            raise SystemExit(f"herdr {' '.join(args)}: {done.stderr.strip() or done.stdout.strip()}")
        return json.loads(done.stdout) if done.stdout.strip().startswith("{") else {}

    # The server and every pane it started are this probe's alone; its
    # Owned ends them.
    deadline = time.monotonic() + 30
    while True:
        try:
            cli("workspace", "list")
            break
        except SystemExit:
            if time.monotonic() > deadline or server.poll() is not None:
                raise
            time.sleep(0.2)
    cli("workspace", "create", "--cwd", str(root), "--no-focus")
    pane = cli("pane", "list")["result"]["panes"][0]["pane_id"]
    out = root / "herdr.json"
    cli("pane", "run", pane, node_program, str(PROBE), str(out), str(count))
    return wait_file(out, DONE_SECONDS, lambda: server.poll() is None)


def summary(asks):
    answered = [ask["ms"] for ask in asks if ask["ms"] is not None]
    return {"asked": len(asks), "answered": len(answered),
            "p50_ms": statistics.median(answered) if answered else None,
            "max_ms": max(answered) if answered else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--herdr", type=Path, required=True)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--count", type=int, default=10)
    args = parser.parse_args()
    node_program = shutil.which("node")
    if not node_program:
        raise SystemExit("node is not on PATH")
    args.run_dir.mkdir(parents=True, exist_ok=True)
    holder_owned.abort_on_signals()
    report = {"system": sys.platform}
    paths = (("holder", lambda root, owned: holder_path(root, args.bin, args.herdr, node_program, args.count, owned)),
             ("herdr", lambda root, owned: herdr_path(root, args.herdr, node_program, args.count, owned)))
    for name, measure in paths:
        root = Path(tempfile.mkdtemp(prefix="hcp-", dir=None if WINDOWS else "/tmp"))
        owned = holder_owned.Owned()
        owned.remove(root)
        try:
            measured = measure(root, owned)
        except holder_owned.Aborted as stop:
            left = owned.close()
            print(f"aborted: {stop}; left running: {left}", file=sys.stderr)
            return 128 + stop.signum
        except BaseException:
            left = owned.close()
            if left:
                print(f"left running: {left}", file=sys.stderr)
            raise
        left = owned.close()
        if left:
            raise SystemExit(f"the {name} probe left running: {'; '.join(left)}")
        report[name] = {"raw": measured, "idle": summary(measured["idle"]),
                        "after_output": summary(measured["after_output"])}
        print(f"{name}: idle {report[name]['idle']}, after 64 KiB {report[name]['after_output']}, "
              f"first answer {measured['idle'][0]['raw'][:80]}")
    (args.run_dir / "color-probe.json").write_text(json.dumps(report, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
