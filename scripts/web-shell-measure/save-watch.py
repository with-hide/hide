#!/usr/bin/env python3
"""How often the daemon and its engine write the saved layout while the topology rounds run
(PRD engine-bundle-3-core-owns B44, perf-gate row 29).

usage: save-watch.py <group>=<path>...   (until SIGTERM, then JSON on stdout)

Each <path> is a file or a folder whose files are watched, recursively; every
10 ms each file's size and modification time are read, and a change of either
is one write, its bytes the size after it. A group names what its files are:
`engine` (the own engine's saved session), `hide` (the daemon's screen
saves). Locks, sockets and SQLite
journals are not saves and are skipped. A write that lands twice within one
10 ms poll counts once, the same for every build, and a save made through a
temporary file and a rename counts the temporary file too when it lives past
one poll: the count is what the folder showed changing, so an engine's way of
saving is part of its number.
"""
import json
import os
import signal
import sys
import time
from pathlib import Path

POLL_SECONDS = 0.010
SKIPPED = (".lock", ".sock", "-wal", "-shm", "-journal")


def files(root):
    if root.is_file():
        yield root
        return
    for dirpath, _dirs, names in os.walk(root):
        for name in names:
            if not name.endswith(SKIPPED):
                yield Path(dirpath) / name


def main():
    groups = []
    for arg in sys.argv[1:]:
        group, sep, path = arg.partition("=")
        if not sep or not group or not path:
            sys.exit("usage: save-watch.py <group>=<path>...")
        groups.append((group, Path(path)))
    if not groups:
        sys.exit("usage: save-watch.py <group>=<path>...")
    stopped = []
    signal.signal(signal.SIGTERM, lambda *_: stopped.append(True))
    signal.signal(signal.SIGINT, lambda *_: stopped.append(True))
    seen = {}
    writes = {}
    started = time.time()
    polls = 0
    while not stopped:
        for group, root in groups:
            for path in files(root):
                try:
                    stat = path.stat()
                except FileNotFoundError:
                    continue
                key = str(path)
                now = (stat.st_mtime_ns, stat.st_size)
                before = seen.get(key)
                seen[key] = now
                # A file present at the first poll is the baseline, not a write.
                if before is None and polls == 0:
                    continue
                if before != now:
                    row = writes.setdefault(key, {"group": group, "writes": 0, "bytes": 0})
                    row["writes"] += 1
                    row["bytes"] += stat.st_size
        polls += 1
        time.sleep(POLL_SECONDS)
    totals = {}
    for row in writes.values():
        total = totals.setdefault(row["group"], {"writes": 0, "bytes": 0})
        total["writes"] += row["writes"]
        total["bytes"] += row["bytes"]
    print(json.dumps({
        "method": "stat every 10 ms; a change of size or mtime is one write, its bytes the size after it; changes within one poll count once, and a temporary file seen before its rename counts as a write",
        "seconds": round(time.time() - started, 3),
        "polls": polls,
        "groups": {group: str(path) for group, path in groups},
        "totals": totals,
        "files": writes,
    }, indent=2))


if __name__ == "__main__":
    main()
