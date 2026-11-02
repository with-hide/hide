#!/usr/bin/env python3
"""Counts the core log's terminal, redraw and resume records by kind (run.sh)."""
import collections, json, sys
counts = collections.Counter()
try:
    with open(sys.argv[1]) as log:
        lines = log.read().splitlines()
except FileNotFoundError:
    lines = []
for line in lines:
    try:
        record = json.loads(line)
    except json.JSONDecodeError:
        continue
    # Older logs wrap an object in event; current records also use event
    # for a string naming the input. Only an object is an envelope.
    if isinstance(record.get("event"), dict):
        record = record["event"]
    kind = str(record.get("kind", ""))
    if kind.startswith("terminal.") or "redraw" in kind or "resume" in kind:
        counts[kind] += 1
        if "panes" in record and isinstance(record["panes"], int):
            counts[kind + ".panes"] += record["panes"]
print(json.dumps(dict(sorted(counts.items()))))
