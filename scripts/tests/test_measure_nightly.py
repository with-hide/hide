"""What turns the nightly performance lane red, and what its record keeps.

The lane fails only when the fixture is not the lane's, a measurement is
invalid by `summarize.py`'s own contract, or a measurement is missing
(docs/PERFORMANCE_TESTING.md, "The nightly performance lane"). Only the first
two are failures the nightly report files by name; a missing one is left to
the step that stopped the run. No figure is a limit: the echo is reported with
no verdict, and the frame window is not covered.
"""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
NIGHTLY = ROOT / "scripts" / "web-shell-measure" / "nightly.py"
ECHO_CHECK = "key echo at operator scale: invalid measurement"
FIXTURE_CHECK = "fixture: not the lane's topology at operator scale"


def write(path, doc):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(doc if isinstance(doc, str) else json.dumps(doc))


def complete_run(run):
    """Every file a finished `topology` run at operator scale writes, with known figures."""
    write(run / "identity.txt", "\n".join([
        "worktree_head=" + "c" * 40, "hided_sha256=" + "d" * 64, "herdr_version=herdr 0.9.3",
        "chrome_version=Google Chrome 154.0", "scenario=topology", "scale=operator", "isolated_headless=true",
        "started_at=2026-10-14T17:30:00Z", "17:30  up 1 day, 2 users, load averages: 1.50 1.20 1.00", ""]))
    write(run / "scale.json", {"scale": "operator", "workspaces": 43, "tabs": 62, "panes": 66, "agents_reported": 30,
                               "printing": 0, "printing_targets": 5, "target": {"workspaces": 43, "tabs": 62, "panes": 66}})
    write(run / "echo-1.json", {"boundary": "page_keydown_to_xterm_write", "expected_samples": 50, "complete": True,
                                "samples": list(range(1, 51)), "load": "load averages: 1.5"})
    write(run / "resources-idle.json", {"method": "20 one-second CPU-time deltas", "samples": [
        {"hided": {"cpu_percent": cpu, "rss_kb": 40960, "self_rss_kb": 30720}} for cpu in (0.0, 1.0, 2.0)]})
    write(run / "topology.json", {"load": "load averages: 1.5", "rounds": 20, "samples": [
        {"kind": "split", "screen_ms": 30.0, "frame_ms": 45.0, "timeout": False},
        {"kind": "split", "screen_ms": None, "frame_ms": None, "timeout": True}]})
    write(run / "isolated" / "hide-state" / "Logs" / "core.jsonl", json.dumps({"event": {
        "kind": "pane_op.timing", "op": "split", "outcome": "applied", "herdr_ack_ms": 5, "layout_event_ms": 12, "sent_ms": 14}}) + "\n")
    for name in ("memory-settled.json", "memory-after-topology.json"):
        write(run / name, {"hided_rss_kb": 40960, "chrome_tree_rss_kb": 409600, "page": {"js_heap_used_bytes": 1, "live_terminals": 2}})


def judge(run):
    env = {key: value for key, value in os.environ.items() if not key.startswith("GITHUB_")}
    record = run / "nightly-summary.json"
    done = subprocess.run(["python3", str(NIGHTLY), str(run), str(record)], capture_output=True, text=True, env=env)
    return done.returncode, json.loads(record.read_text())


class Verdict(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.run = Path(folder.name) / "run"
        complete_run(self.run)

    def test_a_valid_run_passes_and_records_its_figures_without_a_limit(self):
        status, record = judge(self.run)
        self.assertEqual(status, 0)
        self.assertEqual((record["failures"], record["missing"]), ([], []))
        self.assertEqual(record["date"], "2026-10-14")
        # Nearest rank over 1..50: p50 is the 25th value, p95 the 48th.
        self.assertEqual((record["echo"]["p50_ms"], record["echo"]["p95_ms"], record["echo"]["max_ms"]), (25, 48, 50))
        self.assertIsNone(record["echo"]["pass"], "the guide names no agreed echo limit")
        self.assertEqual(record["topology"]["operations"]["split"]["timeouts"], 1)
        self.assertEqual(record["stages"]["split"]["herdr_share_ms"]["p50"], 7.0)
        self.assertEqual(record["fixture"]["panes"], 66)
        self.assertFalse(record["frames"]["covered"])

    def test_an_invalid_echo_is_a_failure_the_report_files(self):
        echo = json.loads((self.run / "echo-1.json").read_text())
        write(self.run / "echo-1.json", {**echo, "samples": echo["samples"][:49]})
        status, record = judge(self.run)
        self.assertEqual(status, 1)
        self.assertEqual([failure["name"] for failure in record["failures"]], [ECHO_CHECK])
        self.assertIn("expected sample count", record["failures"][0]["error"])

    def test_a_fixture_other_than_the_lanes_is_a_failure(self):
        scale = json.loads((self.run / "scale.json").read_text())
        write(self.run / "scale.json", {**scale, "panes": 65})
        identity = (self.run / "identity.txt").read_text().replace("isolated_headless=true", "isolated_headless=false")
        write(self.run / "identity.txt", identity)
        status, record = judge(self.run)
        self.assertEqual(status, 1)
        (failure,) = record["failures"]
        self.assertEqual(failure["name"], FIXTURE_CHECK)
        self.assertIn("panes: the private server has 65, the scale asks for 66", failure["error"])
        self.assertIn("isolated_headless", failure["error"])

    def test_an_own_engine_run_with_no_herdr_passes_and_names_its_engine(self):
        identity = (self.run / "identity.txt").read_text().replace("herdr_version=herdr 0.9.3\n", "")
        write(self.run / "identity.txt", identity + "engine=own\n")
        status, record = judge(self.run)
        self.assertEqual(status, 0)
        self.assertEqual((record["failures"], record["missing"]), ([], []))
        self.assertEqual(record["engine"], "own")
        self.assertEqual(record["lane"], "topology at operator scale, own engine")
        self.assertIsNone(record["identity"]["herdr_version"])

    def test_a_run_that_names_no_engine_ran_on_herdr(self):
        status, record = judge(self.run)
        self.assertEqual(status, 0)
        self.assertEqual((record["engine"], record["lane"]), ("herdr", "topology at operator scale"))

    def test_a_run_that_stopped_fails_without_naming_a_measurement(self):
        for name in ("echo-1.json", "topology.json", "memory-after-topology.json"):
            (self.run / name).unlink()
        status, record = judge(self.run)
        self.assertEqual(status, 1)
        self.assertEqual(record["failures"], [], "the failed step that stopped the run is the one filed")
        self.assertEqual(len(record["missing"]), 3)


if __name__ == "__main__":
    unittest.main()
