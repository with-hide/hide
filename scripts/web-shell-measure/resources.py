#!/usr/bin/env python3
"""One-second CPU deltas and RSS samples for owned process trees.

usage: resources.py <hided-pid> <herdr-pid|engine:pid,...> <chrome-pid> [--memory-series]
MEASURE_RESOURCE_SECONDS sets how many one-second samples (20 by default).
A chrome pid of 0 samples no browser (a windowless window).
CPU is one-core percent from process CPU-time deltas, not ps's lifetime average.
Each one-second sample also keeps the roots' own CPU and RSS and each child's, named
by its accounting name (ps ucomm), so a short child is told apart from the
root growing. No command line, environment, or operator socket is read.
"""
import json
import os
import subprocess
import sys
import time


def snapshot():
    raw = subprocess.check_output(["ps", "-axo", "pid=,ppid=,time=,rss=,ucomm="], text=True)
    rows = {}
    for line in raw.splitlines():
        pid, parent, cpu, rss, *name = line.split(None, 4)
        days, separator, clock = cpu.partition("-")
        seconds = 0.0
        for part in (clock if separator else cpu).split(":"):
            seconds = seconds * 60 + float(part)
        if separator:
            seconds += int(days) * 86400
        rows[int(pid)] = (int(parent), seconds, int(rss), name[0].strip() if name else "")
    return rows


def tree(root, rows):
    if root not in rows:
        raise RuntimeError(f"owned process {root} ended before sampling finished")
    found = {root}
    while True:
        grown = found | {pid for pid, row in rows.items() if row[0] in found}
        if grown == found:
            return found
        found = grown


if len(sys.argv) not in (4, 5) or (len(sys.argv) == 5 and sys.argv[4] != "--memory-series"):
    raise SystemExit("usage: resources.py <hided-pid> <herdr-pid|engine:pid,...> <chrome-pid> [--memory-series]")
# The engine argument can name a group of owned holder roots; each tree is
# counted once, with the roots themselves also reported apart from children.
engine = sys.argv[2]
engine_name = "engine" if engine.startswith("engine:") else "herdr"
engine_pids = engine.removeprefix("engine:")
roots = {name: set(int(pid) for pid in raw.split(",") if int(pid))
         for name, raw in (("hided", sys.argv[1]), (engine_name, engine_pids), ("chrome", sys.argv[3]))}
roots = {name: pids for name, pids in roots.items() if pids}


def group_tree(group, rows):
    return set().union(*(tree(pid, rows) for pid in group))


if len(sys.argv) == 5:
    origin = time.monotonic()
    samples = []
    for minute in range(11):
        time.sleep(max(0, origin + minute * 60 - time.monotonic()))
        rows = snapshot()
        sample = {"seconds": time.monotonic() - origin}
        for name, group in roots.items():
            pids = group_tree(group, rows)
            sample[name] = {"rss_kb": sum(rows[pid][2] for pid in pids), "pids": sorted(pids)}
        samples.append(sample)
    print(json.dumps({"method": "eleven one-minute RSS samples over ten minutes; owned process trees", "samples": samples}))
    raise SystemExit(0)
before, started = snapshot(), time.monotonic()
samples = []
seconds = int(os.environ.get("MEASURE_RESOURCE_SECONDS", "20"))
for _ in range(seconds):
    time.sleep(1)
    after, ended = snapshot(), time.monotonic()
    sample = {"seconds": ended - started}
    for name, group in roots.items():
        pids = group_tree(group, after)
        cpu = sum(max(0, after[pid][1] - before.get(pid, (0, 0, 0, ""))[1]) for pid in pids)
        sample[name] = {"cpu_percent": cpu / (ended - started) * 100,
                        "self_cpu_percent": sum(max(0, after[root][1] - before.get(root, (0, 0, 0, ""))[1]) for root in group) / (ended - started) * 100,
                        "rss_kb": sum(after[pid][2] for pid in pids), "processes": len(pids),
                        "self_rss_kb": sum(after[root][2] for root in group), "root_pids": sorted(group),
                        "children": [{"pid": pid, "name": after[pid][3], "rss_kb": after[pid][2]}
                                     for pid in sorted(pids - group)]}
    sample["host_load"] = list(os.getloadavg())
    samples.append(sample)
    if os.environ.get("MEASURE_RESOURCE_STOP_FILE") and os.path.exists(os.environ["MEASURE_RESOURCE_STOP_FILE"]):
        break
    before, started = after, ended
print(json.dumps({"method": f"{seconds} one-second CPU-time deltas; RSS sums, the root's own RSS and each child's; owned process trees", "samples": samples}))
