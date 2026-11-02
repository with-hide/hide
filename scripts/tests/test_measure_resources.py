"""Engine accounting keeps holder groups and daemon-only costs visible."""
import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest.mock import patch

MEASURE = Path(__file__).resolve().parents[1] / "web-shell-measure"


class EngineResources(unittest.TestCase):
    def test_multiple_holder_roots_count_descendants_once_and_keep_daemon_self(self):
        # Hided owns holders 2 and 3, holder 2 owns a shell, and an unrelated
        # process must never enter the engine group or the daemon's tree.
        before = "1 0 0:01.00 100 hided\n2 1 0:02.00 20 holder\n3 1 0:03.00 30 holder\n4 2 0:04.00 40 shell\n5 0 0:10.00 999 unrelated\n"
        after = "1 0 0:01.50 100 hided\n2 1 0:02.25 20 holder\n3 1 0:03.25 30 holder\n4 2 0:04.50 40 shell\n5 0 0:20.00 999 unrelated\n"
        output = io.StringIO()
        with patch.object(sys, "argv", ["resources.py", "1", "engine:2,3,2", "0"]), \
                patch.dict(os.environ, {"MEASURE_RESOURCE_SECONDS": "1"}), \
                patch("subprocess.check_output", side_effect=[before, after]), \
                patch("time.monotonic", side_effect=[10, 11]), patch("time.sleep"), \
                contextlib.redirect_stdout(output):
            runpy.run_path(str(MEASURE / "resources.py"), run_name="__main__")
        sample = json.loads(output.getvalue())["samples"][0]
        self.assertEqual(sample["engine"]["root_pids"], [2, 3])
        self.assertEqual(sample["engine"]["processes"], 3)
        self.assertEqual(sample["engine"]["rss_kb"], 90)
        self.assertEqual(sample["engine"]["self_rss_kb"], 50)
        self.assertEqual(sample["engine"]["cpu_percent"], 100)
        self.assertEqual(sample["engine"]["self_cpu_percent"], 50)
        self.assertEqual(sample["hided"]["self_rss_kb"], 100)
        self.assertEqual(sample["hided"]["self_cpu_percent"], 50)
        self.assertEqual(sample["hided"]["rss_kb"], 190)

    def test_a_missing_owned_holder_fails_instead_of_understating_cost(self):
        rows = "1 0 0:01.00 100 hided\n"
        with patch.object(sys, "argv", ["resources.py", "1", "engine:2", "0"]), \
                patch.dict(os.environ, {"MEASURE_RESOURCE_SECONDS": "1"}), \
                patch("subprocess.check_output", return_value=rows), patch("time.sleep"):
            with self.assertRaisesRegex(RuntimeError, "owned process 2 ended"):
                runpy.run_path(str(MEASURE / "resources.py"), run_name="__main__")

    def test_diagnostics_distinguish_an_event_name_from_an_old_envelope(self):
        with tempfile.TemporaryDirectory() as folder:
            log = Path(folder) / "core.jsonl"
            log.write_text('\n'.join(map(json.dumps, [
                {"kind": "terminal.input", "event": "key"},
                {"event": {"kind": "terminal.resume", "panes": 5}},
                {"kind": "label", "event": "turn"},
            ])))
            output = io.StringIO()
            with patch.object(sys, "argv", ["diag-counts.py", str(log)]), contextlib.redirect_stdout(output):
                runpy.run_path(str(MEASURE / "diag-counts.py"), run_name="__main__")
        self.assertEqual(json.loads(output.getvalue()), {"terminal.input": 1, "terminal.resume": 1, "terminal.resume.panes": 5})
