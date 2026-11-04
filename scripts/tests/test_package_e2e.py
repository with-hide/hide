"""A failed package pass must retain its evidence when the other engine runs."""

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
    def test_combined_failures_keep_both_engines_and_leave_the_originals(self):
        combine = runpy.run_path(str(ROOT / "scripts/package-e2e-report.py"))["combine"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for engine, status in (("herdr", "flaky"), ("own", "unexpected")):
                report = root / "window" / engine / "report.json"
                report.parent.mkdir(parents=True)
                report.write_text(json.dumps({"suites": [{"title": "window.spec.ts", "specs": [
                    {"title": "shows the window", "file": "window.spec.ts", "tests": [
                        {"status": status, "results": [{"error": {"message": engine}}]}]}]}]}))
            reader = runpy.run_path(str(ROOT / "scripts/ci-flaky-report.py"))
            result = combine(root)
            for status, engine in (("flaky", "herdr"), ("unexpected", "own")):
                found = reader["playwright_tests"](result, "desktop", status)
                self.assertEqual(len(found), 1)
                self.assertIn("window/" + engine, found[0]["name"])
                self.assertEqual(found[0]["error"], engine)
                self.assertTrue((root / "window" / engine / "report.json").is_file())

    def test_each_engine_keeps_its_report_and_propagates_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / "scripts").mkdir()
            (root / "desktop").mkdir()
            shutil.copyfile(ROOT / "scripts/package-e2e.sh", root / "scripts/package-e2e.sh")
            (root / "scripts/verify-web.sh").write_text(
                '#!/usr/bin/env bash\n'
                'printf \'{"engine":"%s"}\\n\' "$HIDE_E2E_ENGINE" > desktop/e2e-report.json\n'
                'printf "%s\\n" "$@" > "$HIDE_E2E_SCREENSHOT_DIR/args.txt"\n'
                'exit "${FIXTURE_EXIT:-0}"\n'
            )
            for engine, status in (("herdr", 23), ("own", 0)):
                result = subprocess.run(
                    ["bash", "scripts/package-e2e.sh", "window", engine, "packaged-window.spec.ts"],
                    cwd=root, env={**os.environ, "RUNNER_OS": "Windows", "FIXTURE_EXIT": str(status)},
                    capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, status, result.stderr)
            for engine in ("herdr", "own"):
                evidence = root / "agents/runs/package-e2e/window" / engine
                self.assertEqual(json.loads((evidence / "report.json").read_text()), {"engine": engine})
                args = (evidence / "screenshots/args.txt").read_text().splitlines()
                self.assertEqual(args[:3], ["desktop", "e2e", "packaged-window.spec.ts"])
                self.assertEqual("--grep-invert" in args, engine == "own")
                self.assertEqual(args[-2:], ["--output", str(evidence / "test-results")])
            self.assertFalse((root / "desktop/e2e-report.json").exists())


if __name__ == "__main__":
    unittest.main()
