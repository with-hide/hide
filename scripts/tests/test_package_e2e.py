"""A failed package pass must retain its evidence when the next one runs."""

import json
import runpy
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class PackageEvidence(unittest.TestCase):
    def test_combined_failures_keep_each_scenario_and_leave_the_originals(self):
        combine = runpy.run_path(str(ROOT / "scripts/package-e2e-report.py"))["combine"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for scenario, status in (("window", "flaky"), ("update", "unexpected")):
                report = root / scenario / "report.json"
                report.parent.mkdir(parents=True)
                report.write_text(json.dumps({"suites": [{"title": f"{scenario}.spec.ts", "specs": [
                    {"title": "shows the window", "file": f"{scenario}.spec.ts", "tests": [
                        {"status": status, "results": [{"error": {"message": scenario}}]}]}]}]}))
            reader = runpy.run_path(str(ROOT / "scripts/ci-flaky-report.py"))
            result = combine(root)
            for status, scenario in (("flaky", "window"), ("unexpected", "update")):
                found = reader["playwright_tests"](result, "desktop", status)
                self.assertEqual(len(found), 1)
                self.assertIn(scenario, found[0]["name"])
                self.assertEqual(found[0]["error"], scenario)
                self.assertTrue((root / scenario / "report.json").is_file())

    def test_each_suite_keeps_its_report_and_propagates_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "scripts").mkdir()
            (root / "desktop").mkdir()
            shutil.copyfile(ROOT / "scripts/package-e2e.sh", root / "scripts/package-e2e.sh")
            (root / "scripts/verify-web.sh").write_text(
                '#!/usr/bin/env bash\n'
                'printf \'{"suite":"%s"}\\n\' "$FIXTURE_SUITE" > desktop/e2e-report.json\n'
                'printf "%s\\n" "$@" > "$HIDE_E2E_SCREENSHOT_DIR/args.txt"\n'
                'exit "${FIXTURE_EXIT:-0}"\n'
            )
            for suite, status in (("window", 23), ("update", 0)):
                result = subprocess.run(
                    ["bash", "scripts/package-e2e.sh", suite, "packaged-window.spec.ts"],
                    cwd=root, env={**os.environ, "RUNNER_OS": "Windows", "FIXTURE_EXIT": str(status), "FIXTURE_SUITE": suite},
                    capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, status, result.stderr)
            for suite in ("window", "update"):
                evidence = root / "agents/runs/package-e2e" / suite
                self.assertEqual(json.loads((evidence / "report.json").read_text()), {"suite": suite})
                args = (evidence / "screenshots/args.txt").read_text().splitlines()
                self.assertEqual(args[:3], ["desktop", "e2e", "packaged-window.spec.ts"])
                self.assertNotIn("--grep-invert", args)
                self.assertEqual(args[-2:], ["--output", str(evidence / "test-results")])
            self.assertFalse((root / "desktop/e2e-report.json").exists())


if __name__ == "__main__":
    unittest.main()
