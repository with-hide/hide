#!/usr/bin/env python3
"""Nearest-rank echo percentiles and the topology, resource and save summaries.

usage: summarize.py echo [--max-p95-ms N] <echo-*.json...>
                                             -> per-trial p50/p95/p99/max, median of trial p95s,
                                                  and a key echo's clock offset cross-check
       summarize.py topology <topology.json>   -> per operation p50/p95/max to screen and to frame
       summarize.py timing <core.jsonl>        -> per operation stage times from pane_op.timing lines
       summarize.py resources <resources.json> -> hided's median one-second CPU and its largest RSS
       summarize.py saves <save-watch.json> <topology.json>
                                             -> layout writes and bytes per group, and writes per operation
"""
import argparse
import json
import math
import sys
from pathlib import Path

KEY_ECHO_BOUNDARY = "page_keydown_to_xterm_write"


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)] if ordered else None


def numbers(values, *, signed=False):
    """Reject a whole series instead of quietly selecting its good samples."""
    if not isinstance(values, list):
        raise ValueError("samples must be an array")
    for value in values:
        if type(value) not in (int, float) or not math.isfinite(value):
            raise ValueError("samples must be finite numbers")
        if not signed and value < 0:
            raise ValueError("duration samples must not be negative")
    return values


def measurement(path):
    doc = json.loads(Path(path).read_text())
    if not isinstance(doc, dict):
        raise ValueError("measurement must be an object")
    return doc


def clocks(doc):
    """A key echo's cross-check across two clocks: the driver's clock minus
    the page's per hop, and the samples timed from the driver's t0. Empty for
    an echo file timed on one clock throughout or written before these."""
    hops = doc.get("hops", [])
    if not isinstance(hops, list) or any(not isinstance(h, dict) for h in hops):
        raise ValueError("clock hops must be an array of objects")
    offsets = numbers([h["clock_offset_ms"] for h in hops if "clock_offset_ms" in h], signed=True)
    if not offsets:
        return {}
    trips = numbers([h["clock_round_trip_ms"] for h in hops if "clock_round_trip_ms" in h])
    node = numbers(doc.get("node_t0_samples", []), signed=True)
    return {
        "clock_offset_ms": {"p50": percentile(offsets, 0.50), "min": min(offsets), "max": max(offsets)},
        "clock_round_trip_max_ms": max(trips) if trips else None,
        "node_t0_p50_ms": percentile(node, 0.50),
        "node_t0_p95_ms": percentile(node, 0.95),
    }


def echo(paths, threshold=None):
    trials = []
    for path in paths:
        errors = []
        values, cross_check, doc = [], {}, {}
        try:
            doc = measurement(path)
            values = numbers(doc.get("samples"))
            cross_check = clocks(doc)
        except (OSError, ValueError, OverflowError) as error:
            errors.append(type(error).__name__ + ": invalid measurement data")
            values = []
        if not values:
            errors.append("no valid samples")
        if doc.get("boundary") != KEY_ECHO_BOUNDARY:
            errors.append("unproven measurement clock boundary")
        expected = doc.get("expected_samples")
        if type(expected) is not int or expected <= 0 or expected != len(values):
            errors.append("missing or mismatched expected sample count")
        if doc.get("complete") is not True or doc.get("error"):
            errors.append("trial did not complete successfully")
        trials.append({
            "file": Path(path).name,
            "count": len(values),
            "expected_samples": expected,
            "boundary": doc.get("boundary"),
            "valid": not errors,
            "errors": errors,
            "p50_ms": percentile(values, 0.50),
            "p95_ms": percentile(values, 0.95),
            "p99_ms": percentile(values, 0.99),
            "max_ms": max(values) if values else None,
            "negative": sum(1 for v in values if v < 0),
            "load": doc.get("load"),
            **cross_check,
        })
    valid = bool(trials) and all(t["valid"] for t in trials)
    p95s = sorted(t["p95_ms"] for t in trials) if valid else []
    median_p95 = p95s[len(p95s) // 2] if p95s else None
    return {
        "trials": trials,
        "valid": valid,
        "errors": [] if trials else ["no trials"],
        "median_trial_p95_ms": median_p95,
        "threshold_ms": threshold,
        # A dated observation is not a threshold for a different run or clock.
        "pass": False if not valid else None if threshold is None else median_p95 <= threshold,
    }


def topology(path):
    doc = json.loads(Path(path).read_text())
    operations = {}
    for sample in doc["samples"]:
        operations.setdefault(sample["kind"], []).append(sample)
    result = {}
    for kind, samples in operations.items():
        row = {"count": len(samples), "timeouts": sum(1 for s in samples if s.get("timeout"))}
        for stage in ("screen_ms", "frame_ms"):
            values = [float(s[stage]) for s in samples if s.get(stage) is not None]
            row[stage] = {
                "p50": percentile(values, 0.50),
                "p95": percentile(values, 0.95),
                "max": max(values) if values else None,
            }
        result[kind] = row
    return {"load": doc.get("load"), "rounds": doc.get("rounds"), "operations_made": doc.get("operations"), "operations": result}


def saves(watch_path, topology_path):
    """The layout saves of one topology run (perf-gate row 29): what the
    daemon's screen saves and the engine's own session file wrote, per group
    and together, and the writes per operation the rounds made."""
    watch = json.loads(Path(watch_path).read_text())
    operations = json.loads(Path(topology_path).read_text()).get("operations")
    totals = watch.get("totals") or {}
    writes = sum(row["writes"] for row in totals.values())
    return {
        "method": watch.get("method"),
        "seconds": watch.get("seconds"),
        "groups": totals,
        "writes": writes,
        "bytes": sum(row["bytes"] for row in totals.values()),
        "operations": operations,
        "writes_per_operation": writes / operations if operations else None,
    }


def stats(values):
    return {
        "count": len(values),
        "p50": percentile(values, 0.50),
        "p95": percentile(values, 0.95),
        "max": max(values) if values else None,
    }


def resources(path):
    doc = json.loads(Path(path).read_text())
    samples = [s["hided"] for s in doc["samples"] if "hided" in s]
    cpu = sorted(float(s["cpu_percent"]) for s in samples)
    return {
        "samples": len(samples),
        "hided_cpu_median_percent": percentile(cpu, 0.50),
        "hided_cpu_p95_percent": percentile(cpu, 0.95),
        "hided_rss_max_kb": max((int(s["rss_kb"]) for s in samples), default=None),
        "hided_self_rss_max_kb": max((int(s["self_rss_kb"]) for s in samples if "self_rss_kb" in s), default=None),
        "method": doc.get("method"),
    }


def timing(path):
    records = {}
    for line in Path(path).read_text().splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        # A line's `event` is the record when it is an object; some lines carry a string there.
        record = entry.get("event") if isinstance(entry.get("event"), dict) else entry
        if not isinstance(record, dict) or record.get("kind") != "pane_op.timing":
            continue
        records.setdefault(record["op"], []).append(record)
    result = {}
    for op, rows in records.items():
        stages = {}
        for stage in ("drawn_ms", "applied_ms", "sent_ms", "first_frame_ms"):
            stages[stage] = stats([float(r[stage]) for r in rows if stage in r])
        outcomes = {}
        for r in rows:
            outcomes[r.get("outcome")] = outcomes.get(r.get("outcome"), 0) + 1
        result[op] = {"records": len(rows), "outcomes": outcomes, "stages": stages}
    return result


if __name__ == "__main__":
    kind = sys.argv[1]
    if kind == "echo":
        parser = argparse.ArgumentParser(description="Summarize complete same-page-clock key echo trials.")
        parser.add_argument("--max-p95-ms", type=float)
        parser.add_argument("paths", nargs="*")
        args = parser.parse_args(sys.argv[2:])
        if args.max_p95_ms is not None and (not math.isfinite(args.max_p95_ms) or args.max_p95_ms < 0):
            parser.error("--max-p95-ms must be a finite nonnegative number")
        result = echo(args.paths, args.max_p95_ms)
    elif kind == "topology":
        result = topology(sys.argv[2])
    elif kind == "timing":
        result = timing(sys.argv[2])
    elif kind == "resources":
        result = resources(sys.argv[2])
    elif kind == "saves":
        result = saves(sys.argv[2], sys.argv[3])
    else:
        raise SystemExit(f"unknown summary {kind!r}; see the usage at the top of summarize.py")
    print(json.dumps(result, indent=2, allow_nan=False))
    if result.get("valid") is False or result.get("pass") is False:
        sys.exit(1)
