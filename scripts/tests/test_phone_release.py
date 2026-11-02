"""B25/B29: main CI uploads retained phone bundles before any release artifacts.

GitHub interprets the dependency graph; test that contract here and the actual
filesystem/HTTP boundary in Node. No test contacts or deploys to the host.
"""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]


class PhoneRelease(unittest.TestCase):
    def test_retention_production_bytes_and_refusals(self):
        result = subprocess.run(["node", "--test", "scripts/tests/phone-release.test.mjs"],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_failed_upload_cannot_produce_a_release_candidate(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        jobs = workflow["jobs"]
        phone = jobs["phone"]
        self.assertEqual(phone["needs"], "validate")
        self.assertEqual(phone["environment"], "phone-production")
        self.assertIn("github.ref == 'refs/heads/main'", phone["if"])
        self.assertIn("github.repository == 'with-hide/hide'", phone["if"])
        self.assertEqual(phone["concurrency"], {"group": "phone-production", "cancel-in-progress": False})
        self.assertLessEqual(phone["timeout-minutes"], 30)
        for name in ["packages", "macos-binaries", "receipt"]:
            self.assertIn("phone", jobs[name]["needs"])
            self.assertNotIn("if", jobs[name])  # GitHub's default success dependency
        for job in [phone, jobs["packages"], jobs["macos-binaries"], jobs["receipt"]]:
            self.assertNotIn("continue-on-error", job)
            for step in job.get("steps", []):
                self.assertNotIn("continue-on-error", step)
                self.assertNotIn("if", step)
        commands = [step["run"] for step in phone["steps"] if "run" in step]
        self.assertEqual(len(commands), 5)
        self.assertIn("phone-release.mjs check", commands[0])
        self.assertIn("verify-web.sh web build:phone", commands[1])
        self.assertIn("phone-release.mjs prepare", commands[2])
        self.assertIn("deploy --prebuilt --prod", commands[3])
        self.assertIn("phone-release.mjs confirm", commands[4])
        # No other workflow may deploy this host or receive its credential.
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            if path.name != "release-candidate.yml":
                self.assertNotIn("PHONE_VERCEL_TOKEN", path.read_text())

    def test_named_secrets_and_example_are_one_contract(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        env = workflow["jobs"]["phone"]["env"]
        keys = {line.split("=", 1)[0] for line in (ROOT / "scripts/phone-release.env.example").read_text().splitlines()
                if line and not line.startswith("#")}
        self.assertEqual(set(env), keys)
        for key in keys:
            self.assertEqual(env[key], "${{ secrets." + key + " }}")
        result = subprocess.run(["node", "--input-type=module", "-e",
                                 "import {SECRETS} from './scripts/phone-release.mjs'; console.log(Object.keys(SECRETS).sort().join('\\n'))"],
                                cwd=ROOT, capture_output=True, text=True, timeout=10, check=True)
        self.assertEqual(set(result.stdout.splitlines()), keys)

    def test_upload_command_propagates_the_hosts_failure(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/release-candidate.yml").read_text())
        upload = next(step["run"] for step in workflow["jobs"]["phone"]["steps"]
                      if step.get("name") == "Upload only the CI-built static site")
        runs = ROOT / "agents/runs/phone-release-tests"
        runs.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=runs) as directory:
            # Substitute only the external upload executable. Run the actual
            # workflow command and require its failure status to reach GitHub.
            fake = Path(directory) / "npx"
            fake.write_text("#!/bin/sh\nexit 23\n")
            fake.chmod(0o755)
            result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", upload], cwd=ROOT,
                                    env={**os.environ, "PATH": directory + os.pathsep + os.environ["PATH"],
                                         "PHONE_VERCEL_TOKEN": "fixture-token"},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 23, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
