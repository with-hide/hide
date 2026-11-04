#!/usr/bin/env python3
"""The pane holder's path, measured at the node (PRD
engine-bundle-2-pane-holder D-17, B15, B21; docs/PERFORMANCE_TESTING.md, The
pane holder's path).

Measurement only, macOS only. One round measures N pane holders.

The node is `hide-node/examples/holder_node`, started the way hided starts its
terminal service (`Holders::start_local`); its panes run `/bin/sh` at 120x40
and take keys through the node's `key`. Each round:

1. idle with no screen attached, then with min(N, 64) attached: CPU (user and
   system), memory (`phys_footprint`) and wakeups (interrupt and idle-exit
   wakeups, `proc_pid_rusage`) of the path's processes over the window;
2. key-to-frame latency on one echo pane (`holder_node echo`): a token typed
   through the node's `key` until its frame reaches the node's screen sink,
   one after another, alone, beside one pane flooding, beside eight panes
   flooding, and beside one flooding pane whose consumer is slow; after each
   flood, once the screen has been still for 2.5 s, the memory again, the
   node's own beside the path's;
3. the node killed outright and a new one attached again: the time until
   every attached pane drew, and latency after;
4. what the echo program received, against every token typed: loss,
   duplication and order on the input side; a token never drawn is a loss on
   the screen side.

The path's processes are the holders; the panes' shells are left out.
Each round records the load average and the cargo and rustc processes running,
so a reader can drop a round taken under a build.

A slow consumer is one pane's connection read slowly and nothing else: the
holder's socket moved aside behind a proxy that passes the node's messages at
once and reads the holder's for 10 ms of every 40 ms, so the holder's own
backpressure and its fragment cap run. The node's sink is never slowed.

With `--load N` the round first starts N busy processes of its own (a
Python loop each), the gate's load L (PRD engine-bundle-3-core-owns D-15:
28), so the path is measured under a known standing load and not under
whatever else the machine runs; they are not the path's processes, and the
result names how many ran (`load_busy`).

Everything a round starts (the node, the holders, the busy processes) is ended by pid and start time on every path, a
stop signal included (`holder_owned.Owned`); a round that leaves any of it
running fails.

Usage: holder-perf.py --bin <folder with holder_node and hide-holder>
       --panes N --out <file.json> [--idle-seconds S] [--keys K]
       [--phases all|slow_consumer|flood_unattached] [--load N]
"""

import argparse
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import queue
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))

import holder_owned  # noqa: E402

ROWS, COLS = 40, 120
ATTACH_MAX = 64
# How long a flood's memory is left to settle before it is read again.
STILL_SECONDS = 2.5
# A slow consumer reads, or runs, for SLOW_READ of every SLOW_PERIOD seconds.
SLOW_READ, SLOW_PERIOD = 0.010, 0.040
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
    def __init__(self, program, home, state, log, owned):
        env = {key: value for key, value in os.environ.items() if not key.startswith(("HERDR_", "HIDE_"))}
        env.update(HOME=str(home), SHELL="/bin/sh", TERM="xterm-256color")
        argv = [str(program), "node", "--home", str(home), "--state", str(state)]
        self.started = time.monotonic()
        with open(log, "ab") as stderr:
            self.process = owned.spawn(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
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
        try:
            value = self.answers.get(timeout=seconds)
        except queue.Empty:
            raise SystemExit(f"the node answered nothing in {seconds:.0f} s") from None
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

    def children(self):
        found = subprocess.run(["pgrep", "-P", str(self.pid)], capture_output=True, text=True).stdout.split()
        return [int(pid) for pid in found]


# --- the paths --------------------------------------------------------------

class HolderPath:
    def __init__(self, root, panes, owned):
        self.root = root
        self.panes = [f"p{index:03d}" for index in range(panes)]
        self.node = None
        self.holders = []
        self.owned = owned

    def start(self, node):
        self.node = node
        if not self.holders:
            for pane in self.panes:
                started = node.ask("start", pane=pane, program=["/bin/sh"], cwd=str(self.root),
                                   env={"PS1": "$ "}, rows=ROWS, cols=COLS)
                holder = started["record"]["holder"]
                self.owned.adopt(holder["pid"], holder["started"])
                self.holders.append(holder["pid"])
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

    def slow(self, pane):
        """The pane's holder connection through a slow proxy: the node lets
        go of the pane and attaches it again, through the proxy."""
        link = SlowLink(self.state / "holders" / f"{pane}.sock")
        self.reconnect(pane)
        return link

    def reconnect(self, pane):
        since = self.node.ask("frames", pane=pane)["count"]
        self.node.ask("release", pane=pane)
        self.node.ask("attach", pane=pane, rows=ROWS, cols=COLS)
        self.node.ask("wait_full", pane=pane, since=since, timeout_ms=30_000)

    def close(self):
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
    # How many processes woke how many times: one return of freed pages
    # each after an attach, or more where a screen drew again.
    counts = {}
    for item in measured.pop("each").values():
        counts[item["wakeups"]] = counts.get(item["wakeups"], 0) + 1
    measured["processes_by_wakeups"] = {str(key): counts[key] for key in sorted(counts)}
    return measured


LEFT = "the round left running: "


class Round:
    def __init__(self, args):
        self.args = args
        self.root = Path(tempfile.mkdtemp(prefix="hpf-", dir="/tmp"))
        self.home = self.root / "home"
        self.home.mkdir()
        self.state = self.home / "s"
        self.sent = []
        self.owned = holder_owned.Owned()
        self.owned.remove(self.root)
        self.owned.records(self.state)
        self.path = self.node = None
        try:
            for _ in range(args.load):
                self.owned.spawn([sys.executable, "-c", "while True: pass"], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.path = HolderPath(self.root, args.panes, self.owned)
            self.path.state = self.state
            self.node = self.new_node()
        except BaseException:
            # The failure stays the one raised; what is left beside it is
            # told, never put in its place.
            left = self.close()
            if left:
                print(LEFT + "; ".join(left), file=sys.stderr)
            raise
        self.result = {"path": "holder", "panes": args.panes, "rows": ROWS, "cols": COLS, "load_busy": args.load,
                       "conditions_start": conditions(), "phases": {}}

    def new_node(self):
        node = Node(self.args.bin / "holder_node", self.home, self.state, self.root / "node.log", self.owned)
        self.path.start(node)
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

    def flood(self, name, panes, size, slow=False):
        link = self.path.slow(panes[0]) if slow else None
        try:
            return self._flood(name, panes, size)
        finally:
            if link is not None:
                link.close()
                if isinstance(self.path, HolderPath):
                    self.path.reconnect(panes[0])

    def _flood(self, name, panes, size):
        node = self.node
        totals_before = node.ask("totals")
        pids = self.path.processes()
        before = sample(pids)
        node_before = usage(node.pid)
        started = time.monotonic()
        for index, pane in enumerate(panes):
            node.type(pane, FLOOD.format(bytes=size, index=index) + "\r")
        latency = self.latency(name, self.args.keys)
        for index, pane in enumerate(panes):
            self.path.wait_done(pane, f"flood-done{index}", 600)
        wall = time.monotonic() - started
        after = sample(pids)
        node_after = usage(node.pid)
        # Then still: a holder gives the flood's freed pages back a second
        # after its screen stops moving.
        time.sleep(STILL_SECONDS)
        settled = delta(before, sample(pids))
        node_still = usage(node.pid)
        totals_after = node.ask("totals")
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
                "node": {"footprint_before_bytes": node_before[1], "footprint_after_bytes": node_after[1],
                         "footprint_still_bytes": node_still[1], "cpu_seconds": node_after[0] - node_before[0]},
                "conditions": conditions()}

    def run(self):
        args, path, phases = self.args, self.path, self.result["phases"]
        attached = path.panes[:min(args.panes, ATTACH_MAX)]
        self.echo = path.panes[0]
        if args.phases == "flood_unattached":
            return self.flood_unattached()
        if args.phases == "all":
            phases["idle_unattached"] = window(path, args.idle_seconds)
            phases["idle_unattached"]["conditions"] = conditions()
        path.attach(attached)
        drawn = self.node.ask("wait_drawn", panes=attached, timeout_ms=120_000)
        assert drawn["drawn"], f"only {drawn['panes_drawn']} of {len(attached)} panes drew"
        # A holder wakes once, a second after its last frame, to give freed
        # pages back; this window shows that wake apart from the idle one.
        if args.phases == "all":
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
            if args.phases == "all":
                phases["flood_one"] = self.flood("one", others[:1], args.flood_bytes)
                phases["flood_eight"] = self.flood("eight", others[:8], args.flood_bytes // 4)
            phases["slow_consumer"] = self.flood("slow", others[:1], args.flood_bytes // 4, slow=True)
            phases["slow_consumer"]["slow"] = {"read_seconds": SLOW_READ, "period_seconds": SLOW_PERIOD,
                                               "consumer": "holder connection proxy"}
        # The node killed outright and a new one attached again.
        self.node.kill()
        started = time.monotonic()
        self.node = Node(self.args.bin / "holder_node", self.home, self.state, self.root / "node.log", self.owned)
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

    def flood_unattached(self):
        """One holder flooded with no screen attached: its memory after the
        flood and once still, for the budget an unattached pane keeps."""
        pane = self.path.panes[-1]
        pid = self.path.holders[-1]
        # Keys reach a pane only while it is attached: typed then, and let
        # go of before the flood starts.
        self.path.attach([pane])
        self.node.ask("wait_drawn", panes=[pane], timeout_ms=30_000)
        self.node.type(pane, "sleep 2; " + FLOOD.format(bytes=self.args.flood_bytes, index=0) + "\r")
        self.node.ask("release", pane=pane)
        before = usage(pid)
        self.path.wait_done(pane, "flood-done0", 600)
        after = usage(pid)
        time.sleep(STILL_SECONDS)
        still = usage(pid)
        self.result["phases"]["flood_unattached"] = {
            "flood_bytes": self.args.flood_bytes, "footprint_before_bytes": before[1],
            "footprint_after_bytes": after[1], "footprint_still_bytes": still[1], "conditions": conditions()}
        self.result["conditions_end"] = conditions()
        return self.result

    def close(self):
        """Ends everything the round started, by pid and start time, and
        answers what is left running or could not be closed; it never
        raises, so a failure of the round is not replaced by its own."""
        left = []
        if self.path is not None:
            try:
                self.path.close()
            except Exception as error:
                left.append(f"the {self.args.path} path did not close: {error}")
        left.extend(self.owned.close())
        return left


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--bin", type=Path, required=True)
    parser.add_argument("--panes", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--idle-seconds", type=float, default=20)
    parser.add_argument("--keys", type=int, default=200)
    parser.add_argument("--flood-bytes", type=int, default=20_000_000)
    parser.add_argument("--phases", choices=("all", "slow_consumer", "flood_unattached"), default="all")
    parser.add_argument("--load", type=int, default=0, help="busy processes the round runs beside the path (L: 28)")
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise SystemExit("this measurement reads macOS's process accounting; Linux and Windows run the CI bounds")
    holder_owned.abort_on_signals()
    try:
        current = Round(args)
        failed = True
        try:
            result = current.run()
            failed = False
        finally:
            left = current.close()
            # A round that failed keeps its own failure and tells what is
            # left beside it; one that measured fails on what is left.
            if left and failed:
                print(LEFT + "; ".join(left), file=sys.stderr)
            elif left:
                raise SystemExit(LEFT + "; ".join(left))
    except holder_owned.Aborted as stop:
        print(f"aborted: {stop}", file=sys.stderr)
        return 128 + stop.signum
    args.out.write_text(json.dumps(result, indent=2))
    print(json.dumps({key: value for key, value in result.items() if key != "phases"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
