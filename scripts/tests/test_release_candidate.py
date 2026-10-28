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
        for name in ("package", "macos-package"):
            job = jobs[name]
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
                                 (".github/workflows/package.yml", True), (".github/workflows/device-helpers.yml", True)]:
            with self.subTest(changed=changed), tempfile.TemporaryDirectory(dir=runs) as temp:
                repo = Path(temp)
                git = lambda *args: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                                                   cwd=repo, check=True, capture_output=True, text=True).stdout.strip()
                git("init", "-q")
                for name in ("README.md", ".github/workflows/agent-journey.yml", ".github/workflows/package.yml",
                             ".github/workflows/device-helpers.yml"):
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
                         {"public_sha", "tag", "verification_run_id", "build_order", "macos_runner"})
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
        self.assertEqual(set(receipt["needs"]), {"validate", "packages", "macos-binaries", "agent-journey"})
        self.assertNotIn("if", receipt)  # default success(), never always()
        for job in [jobs["packages"], jobs["agent-journey"]]:
            self.assertEqual(job["needs"], "validate")
            self.assertEqual(job["with"]["source-sha"], "${{ inputs.public_sha }}")
        self.assertEqual(jobs["packages"]["with"]["build-order"], "${{ inputs.build_order }}")
        journey = jobs["agent-journey"]["with"]
        self.assertEqual(journey["systems"], '["linux","macos","windows"]')
        self.assertEqual(set(journey), {"source-sha", "systems", "fake"})
        self.assertTrue(journey["fake"])
        self.assertNotIn("secrets", jobs["agent-journey"])
        for filename in ["package.yml", "agent-journey.yml"]:
            workflow = yaml.safe_load((WORKFLOWS / filename).read_text())
            for job in workflow["jobs"].values():
                for step in job.get("steps", []):
                    if step.get("uses", "").startswith("actions/checkout@"):
                        self.assertEqual(step["with"]["ref"], "${{ inputs.source-sha || github.sha }}")

    def test_no_receipt_or_mac_app_exists_without_both_device_helpers(self):
        # A failed helper must skip what needs it: the bundles both apps ship
        # the manifest of, so the Mac app's assembly, the Windows app's and,
        # through `packages`, the candidate's receipt. The Mac compile alone
        # starts beside the helpers.
        jobs = yaml.safe_load((WORKFLOWS / "package.yml").read_text())["jobs"]
        self.assertNotIn("if", jobs["device-helpers"])
        self.assertNotIn("if", jobs["helper-identity"])
        self.assertNotIn("if", jobs["device-bundles"])
        self.assertEqual(set(jobs["device-bundles"]["needs"]), {"helper-identity", "device-helpers"})
        self.assertEqual(jobs["macos-compile"]["needs"], "helper-identity")
        self.assertEqual(set(jobs["macos-package"]["needs"]), {"helper-identity", "device-bundles", "macos-compile"})
        self.assertEqual(set(jobs["windows-package"]["needs"]), {"helper-identity", "device-bundles", "windows-compile"})
        # The Linux app carries no device kit, so its package waits for nothing.
        self.assertNotIn("needs", jobs["package"])
        for name in ("macos-compile", "macos-package"):
            self.assertEqual(jobs[name]["if"], "${{ inputs.include-macos }}")
        receipt = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())["jobs"]["receipt"]
        downloads = [step["with"]["name"] for step in receipt["steps"] if step.get("uses", "").startswith("actions/download-artifact@")]
        self.assertEqual(downloads, ["hide-device-helper-linux-x86_64", "hide-device-helper-linux-aarch64",
                                     "hide-macos-release-binaries"])

    def test_the_mac_binaries_compile_on_the_mini_or_on_github_from_the_verified_source(self):
        workflow = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())
        runner = workflow[True]["workflow_dispatch"]["inputs"]["macos_runner"]
        self.assertEqual((runner["options"], runner["default"]), (["mini", "hosted"], "mini"))
        job = workflow["jobs"]["macos-binaries"]
        self.assertEqual(job["needs"], "validate")
        self.assertEqual(job["runs-on"], "${{ inputs.macos_runner == 'hosted' && fromJSON('[\"macos-15\"]') "
                                         "|| fromJSON('[\"self-hosted\",\"macOS\",\"ARM64\",\"hide-candidate\"]') }}")
        self.assertNotIn("environment", job)
        self.assertNotIn("secrets", str(job))
        checkout = next(step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout@"))
        self.assertEqual(checkout["with"], {"ref": "${{ inputs.public_sha }}", "persist-credentials": False})
        self.assertEqual((job["env"]["HIDE_VERSION"], job["env"]["HIDE_BUILD_ORDER"]),
                         ("${{ inputs.tag }}", "${{ inputs.build_order }}"))
        upload = next(step for step in job["steps"] if step.get("uses", "").startswith("actions/upload-artifact@"))
        self.assertEqual(upload["with"]["name"], "hide-macos-release-binaries")
        self.assertEqual(upload["with"]["retention-days"], 7)

    def test_no_workflow_but_the_candidate_names_the_mini_label(self):
        # The Mac mini's job-started hook refuses any other workflow anyway
        # (scripts/candidate-runner/job-started.sh); a workflow that asked for
        # the label would wait on a runner that only ever refuses it.
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            with self.subTest(workflow=path.name):
                text = path.read_text()
                self.assertEqual("hide-candidate" in text, path.name == "release-candidate.yml")
        workflow = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())
        naming = [name for name, job in workflow["jobs"].items() if "hide-candidate" in str(job.get("runs-on", ""))]
        self.assertEqual(naming, ["macos-binaries"])


if __name__ == "__main__":
    unittest.main()
