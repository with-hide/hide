#!/usr/bin/env python3
"""The nightly performance lane's record and verdict for one run of run.sh.

usage: nightly.py <run-dir> <record.json>

<run-dir> is one `MEASURE_SCENARIO=topology MEASURE_SCALE=operator` run of
run.sh with `--isolated-headless` (docs/PERFORMANCE_TESTING.md, "The nightly
performance lane"), on either engine (`MEASURE_ENGINE`), which the record
names. Every figure is derived again from the run's raw files
through summarize.py, so the record follows the measurement contract rather
than a copy of it. It writes the run's record to <record.json>, the same
figures as a table on stdout and on $GITHUB_STEP_SUMMARY when Actions sets
one, and exits 1 when:
- the fixture is not the lane's: another scenario, scale or browser mode, or
  a session whose topology is not the one fixture.mjs asked for;
- a measurement is invalid by summarize.py's contract; or
- a measurement is missing, because the run stopped before it.
The first two are the record's `failures`, which `ci-flaky-report.py
--measurement` annotates for the nightly report. A missing measurement is
not one: the step that stopped the run failed first, and its log says why.
No historical figure is a limit. The echo has no agreed limit, so it is
reported with `pass: null`, and the frame window does not run here.
"""
import hashlib
import json
import os
import sys
from pathlib import Path

import summarize

HERE = Path(__file__).resolve().parent
LANE = {"scenario": "topology", "scale": "operator", "isolated_headless": "true"}
FRAMES = {
    "covered": False,
    "reason": "headless Chrome calls animation frames at a nominal 60 Hz, so every interval sits on the "
              "16.7 ms budget and the strict check measures the browser's scheduler rather than the shell",
}


def identity(run):
    """identity.txt's `key=value` lines, and the `uptime` line among them as `load`."""
    values = {}
    for line in (run / "identity.txt").read_text().splitlines():
        key, separator, value = line.partition("=")
        if separator and key.isidentifier():
            values[key] = value.strip()
        elif "load average" in line:
            values["load"] = line.strip()
    return values


def driver_sha256():
    """One hash over the files that measure, so a trend can tell when the driver changed.

    This judge is left out: it reads and reports, and a change to its table is
    not a new series."""
    digest = hashlib.sha256()
    for path in sorted(HERE.iterdir()):
        if path.is_file() and path.suffix in (".sh", ".js", ".mjs", ".py", ".rs") and path.name != Path(__file__).name:
            digest.update(path.name.encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def fixture(run, ident, failures):
    scale = json.loads((run / "scale.json").read_text())
    wrong = [f"{key} {ident.get(key)!r}, not {want!r}" for key, want in LANE.items() if ident.get(key) != want]
    target = scale.get("target") or {}
    wrong += [f"{key}: the private server has {scale.get(key)}, the scale asks for {target.get(key)}"
              for key in ("workspaces", "tabs", "panes") if scale.get(key) != target.get(key)]
    if wrong:
        failures.append({"name": "fixture: not the lane's topology at operator scale", "error": "; ".join(wrong)})
    return {key: scale.get(key) for key in ("scale", "workspaces", "tabs", "panes", "agents_reported", "printing_targets")}


def echo(run, failures):
    path = run / "echo-1.json"
    if not path.is_file():
        raise FileNotFoundError(2, "missing", str(path))
    result = summarize.echo([str(path)])
    (trial,) = result["trials"]
    if not result["valid"]:
        failures.append({"name": "key echo at operator scale: invalid measurement", "error": "; ".join(trial["errors"])})
    keep = ("count", "p50_ms", "p95_ms", "p99_ms", "max_ms", "errors", "load")
    return {"valid": result["valid"], "pass": result["pass"], "threshold_ms": result["threshold_ms"],
            **{key: trial[key] for key in keep}}


def memory(path):
    doc = json.loads(path.read_text())
    page = doc.get("page") or {}
    return {"hided_rss_kb": doc["hided_rss_kb"], "chrome_tree_rss_kb": doc["chrome_tree_rss_kb"],
            "js_heap_used_bytes": page.get("js_heap_used_bytes"), "live_terminals": page.get("live_terminals")}


def stages(run):
    timing = summarize.timing(run / "isolated" / "hide-state" / "Logs" / "core.jsonl")
    return {op: {"records": row["records"], "outcomes": row["outcomes"],
                 "herdr_share_ms": row["herdr_share_ms"], "hide_share_ms": row["hide_share_ms"]}
            for op, row in timing.items()}


def build(run):
    failures, missing = [], []

    def part(name, read):
        try:
            return read()
        except FileNotFoundError as error:
            missing.append(f"{name}: {Path(error.filename).name} was not written")
            return None

    ident = part("identity", lambda: identity(run)) or {}
    # A run before the engine line was written ran on Herdr.
    engine = ident.get("engine", "herdr")
    record = {
        "lane": "topology at operator scale" + ("" if engine == "herdr" else f", {engine} engine"),
        "engine": engine,
        "date": ident.get("started_at", "")[:10] or None,
        "started_at": ident.get("started_at"),
        "commit": ident.get("worktree_head"),
        "run": os.environ.get("GITHUB_RUN_ID") and (
            f"{os.environ['GITHUB_SERVER_URL']}/{os.environ['GITHUB_REPOSITORY']}/actions/runs/"
            f"{os.environ['GITHUB_RUN_ID']}/attempts/{os.environ.get('GITHUB_RUN_ATTEMPT', '1')}"),
        "identity": {"hided_sha256": ident.get("hided_sha256"), "herdr_version": ident.get("herdr_version"),
                     "chrome_version": ident.get("chrome_version"), "driver_sha256": driver_sha256(),
                     # The hosted image's name and version; absent off Actions.
                     "runner_image": " ".join(filter(None, (os.environ.get("ImageOS"), os.environ.get("ImageVersion")))) or None,
                     "load": ident.get("load")},
        "fixture": part("fixture", lambda: fixture(run, ident, failures)),
        "idle": part("idle resources", lambda: summarize.resources(run / "resources-idle.json")),
        "echo": part("key echo", lambda: echo(run, failures)),
        "topology": part("topology", lambda: summarize.topology(run / "topology.json")),
        "stages": part("stage times", lambda: stages(run)),
        "memory": {"settled": part("memory settled", lambda: memory(run / "memory-settled.json")),
                   "after_topology": part("memory after topology", lambda: memory(run / "memory-after-topology.json"))},
        "frames": FRAMES,
        "failures": failures,
        "missing": missing,
    }
    return record


def ms(value):
    return "-" if value is None else f"{value:.1f} ms"


def mib(kb):
    return "-" if kb is None else f"{kb / 1024:.1f} MiB"


def percent(value):
    return "-" if value is None else f"{value:.1f}%"


def table(record):
    fixture, echo_row, idle = record["fixture"] or {}, record["echo"] or {}, record["idle"] or {}
    ident = record["identity"]
    if record["failures"] or record["missing"]:
        verdict = "**Failed**: " + "; ".join([f["name"] for f in record["failures"]] + record["missing"])
    else:
        verdict = ("**Passed**: every measurement is valid. No limit is checked: the echo has no agreed limit "
                   "and the frame window is not covered on this runner.")
    lines = [
        f"### Performance: {record['lane']}, {record['date']}", "", verdict, "",
        f"Fixture: {fixture.get('workspaces')} workspaces, {fixture.get('tabs')} tabs, {fixture.get('panes')} panes, "
        f"{fixture.get('printing_targets')} printing, {fixture.get('agents_reported')} agents reported; "
        f"{ident['chrome_version']} headless; {ident['herdr_version'] or 'no Herdr'}; hided {str(ident['hided_sha256'])[:12]}; "
        f"driver {ident['driver_sha256'][:12]}; image {ident['runner_image'] or 'not a hosted runner'}; {ident['load']}", "",
        "| Key echo at operator scale | n | p50 | p95 | p99 | max | verdict |",
        "| --- | ---: | ---: | ---: | ---: | ---: | --- |",
        f"| renderer keydown to xterm write | {echo_row.get('count', '-')} | {ms(echo_row.get('p50_ms'))} | "
        f"{ms(echo_row.get('p95_ms'))} | {ms(echo_row.get('p99_ms'))} | {ms(echo_row.get('max_ms'))} | "
        f"{'valid, no agreed limit' if echo_row.get('valid') else 'invalid' if echo_row else 'missing'} |",
        "",
        "| Operation | n | timeouts | to screen p50 | p95 | max | to terminal p50 | p95 | max |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for kind, row in ((record["topology"] or {}).get("operations") or {}).items():
        screen, frame = row["screen_ms"], row["frame_ms"]
        lines.append(
            f"| {kind} | {row['count']} | {row['timeouts']} | {ms(screen['p50'])} | {ms(screen['p95'])} | {ms(screen['max'])} | "
            f"{ms(frame['p50'])} | {ms(frame['p95'])} | {ms(frame['max'])} |")
    lines += ["", "| `pane_op.timing` | records | Herdr share p50 | p95 | Hide share p50 | p95 |",
              "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for op, row in (record["stages"] or {}).items():
        herdr, hide = row["herdr_share_ms"], row["hide_share_ms"]
        lines.append(f"| {op} | {row['records']} | {ms(herdr['p50'])} | {ms(herdr['p95'])} | {ms(hide['p50'])} | {ms(hide['p95'])} |")
    settled, after = record["memory"]["settled"] or {}, record["memory"]["after_topology"] or {}
    lines += [
        "",
        "| Idle, one-second samples | n | hided CPU median | p95 | hided tree RSS max | hided alone RSS max |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
        f"| before any printing | {idle.get('samples', '-')} | {percent(idle.get('hided_cpu_median_percent'))} | "
        f"{percent(idle.get('hided_cpu_p95_percent'))} | {mib(idle.get('hided_rss_max_kb'))} | {mib(idle.get('hided_self_rss_max_kb'))} |",
        "",
        f"Memory, settled then after topology: hided {mib(settled.get('hided_rss_kb'))} then {mib(after.get('hided_rss_kb'))}, "
        f"Chrome tree {mib(settled.get('chrome_tree_rss_kb'))} then {mib(after.get('chrome_tree_rss_kb'))}.",
        "",
        f"Frame window: not covered by this lane; {record['frames']['reason']}.",
        "",
        "<details><summary>The run's record</summary>", "", "```json", json.dumps(record, indent=2), "```", "", "</details>", "",
    ]
    return "\n".join(lines)


def main(argv):
    if len(argv) != 3:
        raise SystemExit("usage: nightly.py <run-dir> <record.json>")
    record = build(Path(argv[1]))
    Path(argv[2]).write_text(json.dumps(record, indent=2, allow_nan=False) + "\n")
    text = table(record)
    print(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write(text)
    for failure in record["failures"]:
        print(f"failed: {failure['name']}: {failure['error']}", file=sys.stderr)
    for gap in record["missing"]:
        print(f"missing: {gap}", file=sys.stderr)
    return 1 if record["failures"] or record["missing"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
