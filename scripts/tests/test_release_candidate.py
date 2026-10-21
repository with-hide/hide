"""Unsigned candidates must test one pinned public source before exposing proof.

The contract is docs/BUILD.md, Unsigned release candidates. GitHub owns the
workflow interpreter; these checks guard the security-relevant configuration
that cannot be exercised by a local receipt test.
"""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"


class ReleaseCandidate(unittest.TestCase):
    def test_receipt_accepts_only_verified_public_source_and_valid_identity(self):
        result = subprocess.run(
            ["node", "--test", "scripts/tests/release-candidate.test.mjs"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_package_version_steps_write_the_selected_order_and_keep_debug_defaults(self):
        jobs = yaml.safe_load((WORKFLOWS / "package.yml").read_text())["jobs"]
        runs = ROOT / "agents/runs/release-candidate-tests"
        runs.mkdir(parents=True, exist_ok=True)
        for name, job in jobs.items():
            command = next(step["run"] for step in job["steps"] if step.get("name") == "Name the version")
            for version, order, expected in [
                ("v1.2.3-rc.1", "501", "HIDE_VERSION=1.2.3-rc.1\nHIDE_BUILD_ORDER=501\n"),
                ("0.0.0-pr.1", "", "HIDE_VERSION=0.0.0-pr.1\nHIDE_BUILD_ORDER=0\n"),
                ("v1.2.3", "9007199254740992", None),
            ]:
                with self.subTest(system=name, order=order), tempfile.TemporaryDirectory(dir=runs) as temp:
                    output = Path(temp) / "github-env"
                    result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", command], cwd=ROOT,
                                            env={**os.environ, "LC_ALL": "en_US.UTF-8", "REQUESTED": version,
                                                 "REQUESTED_ORDER": order, "GITHUB_ENV": str(output)},
                                            capture_output=True, text=True, timeout=10)
                    if expected is None:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertNotIn("HIDE_BUILD_ORDER", output.read_text())
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(output.read_text(), expected)

    def test_candidate_refuses_a_source_whose_called_workflows_differ_from_main(self):
        steps = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())["jobs"]["validate"]["steps"]
        command = next(step["run"] for step in steps if step.get("name") == "Require the build steps the source was verified with")
        runs = ROOT / "agents/runs/release-candidate-tests"
        runs.mkdir(parents=True, exist_ok=True)
        for changed, refused in [("README.md", False), (".github/workflows/agent-journey.yml", True),
                                 (".github/workflows/package.yml", True)]:
            with self.subTest(changed=changed), tempfile.TemporaryDirectory(dir=runs) as temp:
                repo = Path(temp)
                git = lambda *args: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                                                   cwd=repo, check=True, capture_output=True, text=True).stdout.strip()
                git("init", "-q")
                for name in ("README.md", ".github/workflows/agent-journey.yml", ".github/workflows/package.yml"):
                    (repo / name).parent.mkdir(parents=True, exist_ok=True)
                    (repo / name).write_text("first\n")
                git("add", "-A")
                git("commit", "-q", "-m", "source")
                source = git("rev-parse", "HEAD")
                (repo / changed).write_text("second\n")
                git("commit", "-q", "-am", "main")
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", command], cwd=repo,
                                        env={**os.environ, "PUBLIC_SHA": source},
                                        capture_output=True, text=True, timeout=10)
                if refused:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("choose a newer public_sha", result.stdout)
                else:
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_candidate_is_manual_read_only_and_has_no_signing_secrets(self):
        workflow = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())
        self.assertEqual(set(workflow[True]), {"workflow_dispatch"})
        self.assertEqual(set(workflow[True]["workflow_dispatch"]["inputs"]),
                         {"public_sha", "tag", "verification_run_id", "build_order", "live_agents"})
        live = workflow[True]["workflow_dispatch"]["inputs"]["live_agents"]
        self.assertEqual(live["type"], "boolean")
        self.assertIs(live["default"], True)
        for permissions in [workflow["permissions"], *[
            job["permissions"] for job in workflow["jobs"].values() if "permissions" in job
        ]]:
            self.assertTrue(set(permissions) <= {"contents", "actions"})
            self.assertEqual(set(permissions.values()), {"read"})
        packages = workflow["jobs"]["packages"]
        self.assertFalse(packages["with"]["include-macos"])
        self.assertEqual(packages["with"]["macos-signing"], "adhoc")
        self.assertNotIn("secrets", packages)

    def test_receipt_waits_for_packages_and_selected_journey_of_the_same_commit(self):
        jobs = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())["jobs"]
        receipt = jobs["receipt"]
        self.assertEqual(set(receipt["needs"]), {"validate", "packages", "agent-journey"})
        self.assertNotIn("if", receipt)  # default success(), never always()
        for job in [jobs["packages"], jobs["agent-journey"]]:
            self.assertEqual(job["needs"], "validate")
            self.assertEqual(job["with"]["source-sha"], "${{ inputs.public_sha }}")
        self.assertEqual(jobs["packages"]["with"]["build-order"], "${{ inputs.build_order }}")
        journey = jobs["agent-journey"]["with"]
        self.assertEqual(journey["systems"], '["linux","macos","windows"]')
        self.assertEqual(journey["live"], "${{ inputs.live_agents }}")
        self.assertTrue(journey["fake"])
        self.assertEqual(journey["agy"], "linux")
        for filename in ["package.yml", "agent-journey.yml"]:
            workflow = yaml.safe_load((WORKFLOWS / filename).read_text())
            for job in workflow["jobs"].values():
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        self.assertEqual(step["with"]["ref"], "${{ inputs.source-sha || github.sha }}")


if __name__ == "__main__":
    unittest.main()
