#!/usr/bin/env python3
"""The pane holder's path against Herdr's session path, measured at the node
(PRD engine-bundle-2-pane-holder D-17, B15, B21; docs/PERFORMANCE_TESTING.md,
The pane holder's path).

Measurement only, macOS only. One round measures one path at one pane count:

- `herdr`: a private Herdr server of the pinned build (HOME, every XDG folder,
  HERDR_SESSION, HERDR_CONFIG_PATH and HERDR_SOCKET_PATH its own) with N
  panes, attached through the node's Herdr sessions (`herdr terminal session`
  clients), the development switch off, as hided runs today;
- `holder`: the same node with the switch on and N pane holders.

The node is `hide-node/examples/holder_node`, started the way hided starts its
terminal service (`Holders::start_local`); both paths' panes run `/bin/sh` at
120x40 and take the same keys through the node's `key`. Each round:

1. idle with no screen attached, then with min(N, 64) attached: CPU (user and
   system), memory (`phys_footprint`) and wakeups (interrupt and idle-exit
   wakeups, `proc_pid_rusage`) of the path's processes over the window;
2. key-to-frame latency on one echo pane (`holder_node echo`): a token typed
   through the node's `key` until its frame reaches the node's screen sink,
   one after another, alone, beside one pane flooding, beside eight panes
   flooding, and beside one flooding pane whose screen delivery is slowed;
   after each flood, once the screen has been still for 2.5 s, the memory
   again;
3. the node killed outright and a new one attached again: the time until
   every attached pane drew, and latency after;
4. what the echo program received, against every token typed: loss,
   duplication and order on the input side; a token never drawn is a loss on
   the screen side.

The path's processes are the Herdr server and the node's Herdr session
clients, or the holders; the panes' shells are the same on both and left out.
Each round records the load average and the cargo and rustc processes running,
so a reader can drop a round taken under a build.

Usage: holder-perf.py --bin <folder with holder_node and hide-holder> --herdr <pinned herdr>
       --path herdr|holder --panes N --out <file.json> [--idle-seconds S] [--keys K]
"""

import argparse
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import queue
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROWS, COLS = 40, 120
ATTACH_MAX = 64
# How long a flood's memory is left to settle before it is read again.
STILL_SECONDS = 2.5
FLOOD = "od -An -tx1 -v /dev/urandom | head -c {bytes}; printf 'flood-%s\\n' done{index}"


# --- the kernel's account of a process (macOS) ------------------------------

class RusageV2(ctypes.Structure):
    _fields_ = [("uuid", ctypes.c_uint8 * 16), ("user", ctypes.c_uint64), ("system", ctypes.c_uint64),
                ("pkg_idle_wkups", ctypes.c_uint64), ("interrupt_wkups", ctypes.c_uint64),
                ("pageins", ctypes.c_uint64), ("wired", ctypes.c_uint64), ("resident", ctypes.c_uint64),
                ("phys_footprint", ctypes.c_uint64), ("start", ctypes.c_uint64), ("exit", ctypes.c_uint64),
                ("child_user", ctypes.c_uint64), ("child_system", ctypes.c_uint64),
                ("child_pkg_idle_wkups", ctypes.c_uint64), ("child_interrupt_wkups", ctypes.c_uint64),
                ("child_pageins", ctypes.c_uint64), ("child_elapsed", ctypes.c_uint64),
                ("diskio_read", ctypes.c_uint64), ("diskio_written", ctypes.c_uint64)]


class Timebase(ctypes.Structure):
    _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]


LIBPROC = ctypes.CDLL(ctypes.util.find_library("proc") or "/usr/lib/libproc.dylib")
LIBC = ctypes.CDLL(ctypes.util.find_library("c"))
TIMEBASE = Timebase()
LIBC.mach_timebase_info(ctypes.byref(TIMEBASE))


def usage(pid):
    """(CPU seconds, footprint bytes, wakeups) of `pid`, or None once it ended."""
    info = RusageV2()
    if LIBPROC.proc_pid_rusage(int(pid), 2, ctypes.byref(info)) != 0:
        return None
    ticks = info.user + info.system
    return (ticks * TIMEBASE.numer / TIMEBASE.denom / 1e9, info.phys_footprint,
            info.pkg_idle_wkups + info.interrupt_wkups)


def sample(pids):
    return {pid: usage(pid) for pid in pids}


def delta(before, after):
    """Per process: CPU seconds and wakeups over the window, footprint at its end."""
    each = {}
    for pid, end in after.items():
        start = before.get(pid)
        if end is None or start is None:
            continue
        each[pid] = {"cpu": end[0] - start[0], "footprint": end[1], "wakeups": end[2] - start[2]}
    return {"processes": len(each),
            "cpu_seconds": sum(item["cpu"] for item in each.values()),
            "footprint_bytes": sum(item["footprint"] for item in each.values()),
            "footprint_max_bytes": max((item["footprint"] for item in each.values()), default=0),
            "wakeups": sum(item["wakeups"] for item in each.values()),
            "wakeups_max": max((item["wakeups"] for item in each.values()), default=0),
            "wakeups_nonzero": sum(1 for item in each.values() if item["wakeups"]),
            "each": each}


def conditions():
    """The load and the builds running, for the round's validity."""
    builds = subprocess.run(["pgrep", "-x", "cargo|rustc|cc|ld|zig"], capture_output=True, text=True).stdout.split()
    return {"load": os.getloadavg(), "cores": os.cpu_count(), "build_processes": len(builds)}


# --- the node ---------------------------------------------------------------

class Node:
    def __init__(self, program, home, state, switch, herdr, socket, log):
        env = {key: value for key, value in os.environ.items() if not key.startswith(("HERDR_", "HIDE_"))}
        env.update(HOME=str(home), SHELL="/bin/sh", TERM="xterm-256color")
        if switch:
            env["HIDE_DEV_PANE_HOLDER"] = "1"
        argv = [str(program), "node", "--home", str(home), "--state", str(state), "--herdr-bin", str(herdr)]
        if socket:
            argv += ["--herdr-socket", str(socket)]
        self.log = open(log, "ab")
        self.started = time.monotonic()
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
                                        env=env, text=True)
        self.answers = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        self.pid = self._answer(30)["ready"]

    def _read(self):
        for line in self.process.stdout:
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if isinstance(value, dict) and "ok" in value:
                self.answers.put(value)
        self.answers.put(None)

    def _answer(self, seconds):
        value = self.answers.get(timeout=seconds)
        if value is None:
            raise SystemExit(f"the node exited ({self.process.wait()})")
        if not value["ok"]:
            raise SystemExit(value.get("error"))
        return value

    def ask(self, op, wait=120, **fields):
        self.process.stdin.write(json.dumps({"op": op, **fields}) + "\n")
        self.process.stdin.flush()
        return self._answer(wait + fields.get("timeout_ms", 0) / 1000)

    def type(self, pane, text):
        self.ask("key", pane=pane, hex=text.encode().hex())

    def kill(self):
        self.process.kill()
        self.process.wait(30)
        self.log.close()

    def quit(self):
        if self.process.poll() is None:
            self.process.stdin.write(json.dumps({"op": "quit"}) + "\n")
            self.process.stdin.flush()
            self.process.wait(60)
        self.log.close()

    def children(self):
        found = subprocess.run(["pgrep", "-P", str(self.pid)], capture_output=True, text=True).stdout.split()
        return [int(pid) for pid in found]


# --- the paths --------------------------------------------------------------

class HerdrPath:
    switch = False

    def __init__(self, herdr, root, panes):
        self.herdr = herdr
        self.env = {key: value for key, value in os.environ.items() if not key.startswith(("HERDR_", "HIDE_"))}
        for name in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            folder = root / ("herdr-" + name.lower())
            folder.mkdir()
            self.env[name] = str(folder)
        self.socket = Path(f"/tmp/hpf-{os.getpid()}.sock")
        self.env.update(HERDR_SESSION=f"holder-perf-{os.getpid()}", HERDR_CONFIG_PATH=str(root / "herdr.toml"),
                        HERDR_SOCKET_PATH=str(self.socket), SHELL="/bin/sh", TERM="xterm-256color")
        self.server = subprocess.Popen([str(herdr), "server"], env=self.env, stdin=subprocess.DEVNULL,
                                       stdout=open(root / "herdr-server.out", "w"), stderr=subprocess.STDOUT,
                                       start_new_session=True)
        self.node = None
        try:
            self._populate(root, panes)
        except BaseException:
            # A server left running keeps its panes' terminals, and enough of
            # them exhaust the machine's (macOS allows 511).
            self.close()
            raise

    def _populate(self, root, panes):
        deadline = time.monotonic() + 30
        while not self.socket.exists():
            if time.monotonic() > deadline:
                raise SystemExit("the private Herdr server never listened")
            time.sleep(0.05)
        listed = self.cli("workspace", "list")
        assert not listed.get("result", {}).get("workspaces"), f"the private server holds workspaces: {listed}"
        workspace = self.cli("workspace", "create", "--cwd", str(root), "--no-focus")["result"]["workspace"]["workspace_id"]
        for _ in range(panes - 1):
            self.cli("tab", "create", "--workspace", workspace, "--cwd", str(root), "--no-focus")
        found = self.cli("pane", "list")["result"]["panes"]
        self.panes = [pane["pane_id"] for pane in found]
        assert len(self.panes) == panes, f"{len(self.panes)} panes for {panes}"

    def cli(self, *args):
        done = subprocess.run([str(self.herdr), *args], env=self.env, capture_output=True, text=True, timeout=60)
        if done.returncode:
            raise SystemExit(f"herdr {' '.join(args)}: {done.stderr.strip()}")
        return json.loads(done.stdout) if done.stdout.strip().startswith("{") else {}

    def start(self, node):
        self.node = node

    def attach(self, panes):
        for pane in panes:
            self.node.ask("attach_herdr", pane=pane, rows=ROWS, cols=COLS)

    def reattach(self, panes):
        self.attach(panes)

    def processes(self):
        return [self.server.pid, *self.node.children()]

    def wait_done(self, pane, marker, seconds):
        self.cli("pane", "wait-output", pane, "--match", marker, "--source", "visible",
                 "--timeout", str(int(seconds * 1000)))

    def close(self):
        if self.server.poll() is None:
            self.server.kill()
        self.server.wait(30)
        self.socket.unlink(missing_ok=True)


class HolderPath:
    switch = True

    def __init__(self, herdr, root, panes):
        self.root = root
        self.panes = [f"p{index:03d}" for index in range(panes)]
        self.socket = None
        self.node = None
        self.holders = []

    def start(self, node):
        self.node = node
        if not self.holders:
            for pane in self.panes:
                started = node.ask("start", pane=pane, program=["/bin/sh"], cwd=str(self.root),
                                   env={"PS1": "$ "}, rows=ROWS, cols=COLS)
                self.holders.append(started["record"]["holder"]["pid"])
            node.ask("wait_drawn", panes=self.panes[:ATTACH_MAX], timeout_ms=120_000)
            for pane in self.panes:
                node.ask("release", pane=pane)

    def attach(self, panes):
        for pane in panes:
            self.node.ask("attach", pane=pane, rows=ROWS, cols=COLS)

    def reattach(self, panes):
        # A node's start finds its holders again and attaches them itself.
        del panes

    def processes(self):
        return list(self.holders)

    def wait_done(self, pane, marker, seconds):
        self.node.ask("wait_text", pane=pane, text=marker, lines=ROWS, every_ms=20, timeout_ms=int(seconds * 1000))

    def close(self):
        if self.node is not None and self.node.process.poll() is None:
            for pane in self.panes:
                try:
                    self.node.ask("terminate", pane=pane, wait=60)
                except SystemExit:
                    pass
        # Whatever no node ended: the holders this round started, by pid while
        # each still runs this round's holder, and then the shell each held.
        for pid in self.holders:
            found = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
            if str(self.root) not in found:
                continue
            shells = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True).stdout.split()
            os.kill(pid, 9)
            for shell in shells:
                try:
                    os.kill(int(shell), 9)
                except ProcessLookupError:
                    pass


# --- one round --------------------------------------------------------------

def percentile(values, share):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(round(share * (len(ordered) - 1))))]


def summary(micros):
    return {"count": len(micros), "p50_ms": percentile(micros, 0.5) / 1000 if micros else None,
            "p95_ms": percentile(micros, 0.95) / 1000 if micros else None,
            "max_ms": max(micros) / 1000 if micros else None}


def window(path, seconds):
    pids = path.processes()
    before = sample(pids)
    time.sleep(seconds)
    after = sample(pids)
    measured = delta(before, after)
    measured["seconds"] = seconds
    measured.pop("each")
    return measured


class Round:
    def __init__(self, args):
        self.args = args
        self.root = Path(tempfile.mkdtemp(prefix="hpf-", dir="/tmp"))
        self.home = self.root / "home"
        self.home.mkdir()
        self.state = self.home / "s"
        self.sent = []
        self.path = (HerdrPath if args.path == "herdr" else HolderPath)(args.herdr, self.root, args.panes)
        try:
            self.node = self.new_node()
        except BaseException:
            self.path.close()
            shutil.rmtree(self.root, ignore_errors=True)
            raise
        self.result = {"path": args.path, "panes": args.panes, "rows": ROWS, "cols": COLS,
                       "conditions_start": conditions(), "phases": {}}

    def new_node(self):
        node = Node(self.args.bin / "holder_node", self.home, self.state, self.path.switch, self.args.herdr,
                    self.path.socket, self.root / "node.log")
        try:
            self.path.start(node)
        except BaseException:
            self.path.close()
            node.kill()
            raise
        return node

    def latency(self, name, count):
        # A seed past the last phase's tokens, so a phase's first token
        # differs in every cell from the one before it.
        # 40 ms apart: both paths draw at most once per 16 ms after the last
        # frame, so keys typed back to back would measure that pacing.
        answer = self.node.ask("latency", pane=self.echo, count=count, seed=len(self.sent), timeout_ms=10_000,
                               gap_ms=40, wait=count * 12)
        self.sent.extend(answer["tokens"])
        measured = summary(answer["micros"])
        measured["lost_on_screen"] = len(answer["lost"])
        return measured

    def flood(self, name, panes, size, slow=0):
        node = self.node
        if slow:
            node.ask("slow", micros=slow, panes=panes)
        totals_before = node.ask("totals")
        pids = self.path.processes()
        before = sample(pids)
        started = time.monotonic()
        for index, pane in enumerate(panes):
            node.type(pane, FLOOD.format(bytes=size, index=index) + "\r")
        latency = self.latency(name, self.args.keys)
        for index, pane in enumerate(panes):
            self.path.wait_done(pane, f"flood-done{index}", 600)
        wall = time.monotonic() - started
        after = sample(pids)
        # Then still: a holder gives the flood's freed pages back a second
        # after its screen stops moving.
        time.sleep(STILL_SECONDS)
        settled = delta(before, sample(pids))
        totals_after = node.ask("totals")
        if slow:
            node.ask("slow", micros=0, panes=[])
        frames = {item["pane"]: item for item in totals_after["panes"]}
        earlier = {item["pane"]: item for item in totals_before["panes"]}
        flooded = [{"frames": frames[pane]["frames"] - earlier.get(pane, {}).get("frames", 0),
                    "bytes": frames[pane]["bytes"] - earlier.get(pane, {}).get("bytes", 0)} for pane in panes]
        usage_window = delta(before, after)
        usage_window.pop("each")
        return {"latency": latency, "flood_panes": len(panes), "flood_bytes_each": size, "wall_seconds": wall,
                "frames": sum(item["frames"] for item in flooded), "frame_bytes": sum(item["bytes"] for item in flooded),
                "usage": usage_window,
                "still": {"seconds": STILL_SECONDS, "footprint_bytes": settled["footprint_bytes"],
                          "footprint_max_bytes": settled["footprint_max_bytes"]},
                "conditions": conditions()}

    def run(self):
        args, path, phases = self.args, self.path, self.result["phases"]
        attached = path.panes[:min(args.panes, ATTACH_MAX)]
        self.echo = path.panes[0]
        phases["idle_unattached"] = window(path, args.idle_seconds)
        phases["idle_unattached"]["conditions"] = conditions()
        path.attach(attached)
        drawn = self.node.ask("wait_drawn", panes=attached, timeout_ms=120_000)
        assert drawn["drawn"], f"only {drawn['panes_drawn']} of {len(attached)} panes drew"
        # A holder wakes once, a second after its last frame, to give freed
        # pages back; this window shows that wake apart from the idle one.
        phases["settle_after_attach"] = window(path, STILL_SECONDS)
        phases["idle_attached"] = window(path, args.idle_seconds)
        phases["idle_attached"]["attached"] = len(attached)
        phases["idle_attached"]["conditions"] = conditions()
        self.echo_log = self.root / "echo.log"
        self.node.type(self.echo, f"exec {args.bin / 'holder_node'} echo {self.echo_log}\r")
        path.wait_done(self.echo, "echo ready", 30)
        phases["latency_alone"] = {"latency": self.latency("alone", args.keys), "conditions": conditions()}
        others = path.panes[1:]
        if others:
            phases["flood_one"] = self.flood("one", others[:1], args.flood_bytes)
            phases["flood_eight"] = self.flood("eight", others[:8], args.flood_bytes // 4)
            slow = [pane for pane in others[:1]]
            phases["slow_consumer"] = self.flood("slow", slow, args.flood_bytes // 4, slow=30000)
            phases["slow_consumer"]["slow_micros_per_frame"] = 30000
        # The node killed outright and a new one attached again.
        self.node.kill()
        started = time.monotonic()
        self.node = Node(self.args.bin / "holder_node", self.home, self.state, path.switch, args.herdr, path.socket,
                         self.root / "node.log")
        path.node = self.node
        path.reattach(attached)
        drawn = self.node.ask("wait_drawn", panes=attached, timeout_ms=300_000)
        phases["reattach"] = {"seconds_to_all_drawn": time.monotonic() - started, "drawn": drawn["drawn"],
                              "attached": len(attached), "latency": self.latency("after", max(20, args.keys // 4)),
                              "conditions": conditions()}
        received = self.echo_log.read_text(errors="replace")
        expected = "".join(self.sent)
        # The echo received exactly every token typed, once each and in
        # order, or the first place it did not.
        tokens = re.findall(r"<[A-Z]{12}>", received)
        first = next((index for index, (got, sent) in enumerate(zip(tokens, self.sent)) if got != sent), None)
        self.result["input"] = {"tokens": len(self.sent), "received_tokens": len(tokens),
                                "exact": received == expected, "first_difference": first,
                                "lost": max(0, len(self.sent) - len(tokens)),
                                "extra": max(0, len(tokens) - len(self.sent))}
        self.result["conditions_end"] = conditions()
        return self.result

    def close(self):
        try:
            self.path.close()
            self.node.quit()
        finally:
            if isinstance(self.path, HerdrPath):
                self.path.close()
            shutil.rmtree(self.root, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--herdr", type=Path, required=True)
    parser.add_argument("--path", choices=("herdr", "holder"), required=True)
    parser.add_argument("--panes", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--idle-seconds", type=float, default=20)
    parser.add_argument("--keys", type=int, default=200)
    parser.add_argument("--flood-bytes", type=int, default=20_000_000)
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise SystemExit("this measurement reads macOS's process accounting; Linux and Windows run the CI bounds")
    current = Round(args)
    try:
        result = current.run()
    finally:
        current.close()
    args.out.write_text(json.dumps(result, indent=2))
    print(json.dumps({key: value for key, value in result.items() if key != "phases"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
