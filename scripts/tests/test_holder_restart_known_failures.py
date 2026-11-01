"""A cell whose failure is open as an issue is marked and does not fail the
restart lane, unless it left a process running (scripts/holder-restart-check.py,
KNOWN_FAILURES; docs/TESTING.md, The holder restart lane)."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))
CONTRACT = SCRIPTS.parent / "contracts/agent-journey.json"
ISSUE = "with-hide/hide-private#405"


def lane():
    spec = importlib.util.spec_from_file_location("holder_restart_check", SCRIPTS / "holder-restart-check.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class KnownFailures(unittest.TestCase):
    def setUp(self):
        self.lane = lane()
        self.run = Path(tempfile.mkdtemp(prefix="hrk-"))
        self.addCleanup(shutil.rmtree, self.run, True)

    def cells(self, results):
        """Runs `run_cells` on Windows for Antigravity and Codex, each cell
        answering the given (result, left running)."""
        def cell(row, *_args):
            result, left = results[row["id"]]
            return {"agent": row["id"], "result": result, "reason": "stand-in", "cleanup": {"left": left}}
        report = {"cells": []}
        rows = [{"id": "antigravity"}, {"id": "codex"}]
        with mock.patch.object(self.lane, "cell", cell), \
                mock.patch.object(self.lane.platform, "system", return_value="Windows"):
            self.lane.run_cells(rows, None, self.run, report)
        return report

    def test_a_known_failure_that_ended_everything_is_marked_and_passes_the_lane(self):
        report = self.cells({"antigravity": ("fail", []), "codex": ("pass", [])})
        self.assertEqual(report["cells"][0]["known_failure"], ISSUE)
        self.assertEqual(self.lane.failed_cells(report), [])

    def test_a_known_failure_that_left_a_process_running_fails_the_lane(self):
        report = self.cells({"antigravity": ("fail", ["4242 still runs"]), "codex": ("pass", [])})
        self.assertNotIn("known_failure", report["cells"][0])
        self.assertEqual(self.lane.failed_cells(report), ["antigravity"])

    def test_a_pass_that_left_a_process_running_fails_even_when_known(self):
        report = self.cells({"antigravity": ("pass", ["4242 still runs"]), "codex": ("pass", [])})
        self.assertEqual(report["cells"][0]["result"], "fail")
        self.assertEqual(self.lane.failed_cells(report), ["antigravity"])

    def test_another_cell_that_fails_is_not_marked(self):
        report = self.cells({"antigravity": ("pass", []), "codex": ("fail", [])})
        self.assertNotIn("known_failure", report["cells"][1])
        self.assertEqual(self.lane.failed_cells(report), ["codex"])

    def test_every_known_failure_names_an_agent_of_the_contract(self):
        rows = json.loads(CONTRACT.read_text())
        self.assertEqual(self.lane.unknown_known_failures(rows), [])
        with mock.patch.dict(self.lane.KNOWN_FAILURES, {("agy", "Windows"): ISSUE}):
            self.assertEqual(self.lane.unknown_known_failures(rows), ["agy on Windows"])


if __name__ == "__main__":
    unittest.main()
