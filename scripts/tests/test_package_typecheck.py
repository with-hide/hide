"""A package may delegate web tsc only to its exact-source CI verify gate.

Run the real packager in a private checkout and stop at the external pnpm
boundary, before native compilation or packaging. Electron dependency stubs
only satisfy imports: no Electron operation is reached by these tests.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"


class PackageTypecheck(unittest.TestCase):
    def setUp(self):
        runs = ROOT / "agents/runs/package-typecheck-tests"
        runs.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=runs)
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        scripts = self.repo / "desktop/scripts"
        scripts.mkdir(parents=True)
        for name in ("package", "build-identity", "device-helpers", "fuses", "installers", "macos-signing", "release-binaries"):
            shutil.copyfile(ROOT / f"desktop/scripts/{name}.mjs", scripts / f"{name}.mjs")
        (self.repo / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/check-release-assets.mjs", self.repo / "scripts/check-release-assets.mjs")
        (self.repo / "web").mkdir()
        shutil.copyfile(ROOT / "web/package.json", self.repo / "web/package.json")
        (self.repo / "web/source.ts").write_text("const value: number = 1;\n")
        (self.repo / "pnpm-lock.yaml").write_text("lockfileVersion: '9.0'\n")
        # These external packages are never used: pnpm deliberately fails
        # before the packager builds native programs or invokes Electron.
        for name, exports in {
            "@electron/packager": "packager",
            "@electron/fuses": "FuseV1Options,FuseVersion,flipFuses,getCurrentFuseWire",
            "@electron/asar": "createPackage,extractAll",
            "electron-builder": "Arch,build,Platform",
        }.items():
            module = self.repo / "node_modules" / name
            module.mkdir(parents=True)
            (module / "package.json").write_text(json.dumps({"type": "module", "exports": "./index.js"}))
            (module / "index.js").write_text("\n".join(f"export const {key} = {{}};" for key in exports.split(",")))
        self.bin = self.repo / "bin"
        self.bin.mkdir()
        pnpm = self.bin / "pnpm"
        pnpm.write_text(
            '#!/bin/sh\n'
            'cd web || exit 25\n'
            'case "$*" in\n'
            '  "--dir web build") script=$(node -p "require(\'./package.json\').scripts.build"); exec sh -c "$script" ;;\n'
            '  "--dir web bundle") script=$(node -p "require(\'./package.json\').scripts.bundle"); exec sh -c "$script" ;;\n'
            '  *) echo "unexpected package command" >&2; exit 25 ;;\n'
            'esac\n'
        )
        pnpm.chmod(0o755)
        for name, message, code in (("tsc", "compiler required", 23), ("vite", "bundle without compiler", 24)):
            tool = self.bin / name
            tool.write_text(f'#!/bin/sh\necho "{message}"\nexit {code}\n')
            tool.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        HIDE_VERSION="0.0.0-fixture", HIDE_MACOS_SIGNING="adhoc", LC_ALL="C", LANG="C")
        self.env.pop("HIDE_WEB_TYPECHECKED_SHA", None)
        self.env.pop("GITHUB_ACTIONS", None)
        self.git("init", "-q")
        self.git("add", "web", "pnpm-lock.yaml")
        self.git("-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid",
                 "commit", "-qm", "fixture")
        self.commit = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args], cwd=self.repo, env=self.env,
                              check=True, capture_output=True, text=True, timeout=10).stdout.strip()

    def package(self, *args, **env):
        result = subprocess.run(["node", "desktop/scripts/package.mjs", *args], cwd=self.repo,
                                env={**self.env, **env}, capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0, "fixture must stop before native compilation")
        return result.stdout + result.stderr

    def test_standalone_and_ci_without_a_verify_dependency_keep_typechecking(self):
        for args in ((), ("compile",)):
            for env in ({}, {"GITHUB_ACTIONS": "true"}):
                with self.subTest(args=args, env=env):
                    output = self.package(*args, **env)
                    self.assertIn("compiler required", output)
                    self.assertNotIn("bundle without compiler", output)

    def test_exact_source_ci_delegates_typecheck_and_propagates_bundle_failure(self):
        for args in ((), ("compile",)):
            with self.subTest(args=args):
                output = self.package(*args, GITHUB_ACTIONS="true", HIDE_WEB_TYPECHECKED_SHA=self.commit)
                self.assertIn("bundle without compiler", output)
                self.assertIn(f"Web typecheck delegated to verify for {self.commit}", output)
                self.assertNotIn("compiler required", output)

    def test_local_override_and_other_or_malformed_sources_are_refused(self):
        for env in (
            {"HIDE_WEB_TYPECHECKED_SHA": self.commit},
            {"GITHUB_ACTIONS": "true", "HIDE_WEB_TYPECHECKED_SHA": "a" * 40},
            {"GITHUB_ACTIONS": "true", "HIDE_WEB_TYPECHECKED_SHA": "true"},
        ):
            with self.subTest(env=env):
                output = self.package(**env)
                self.assertIn("requires GitHub Actions and the package checkout's exact commit", output)
                self.assertNotIn("bundle without compiler", output)
                self.assertNotIn("compiler required", output)

    def test_modified_source_or_lockfile_cannot_claim_the_committed_typecheck(self):
        for name in ("web/source.ts", "pnpm-lock.yaml"):
            with self.subTest(name=name):
                original = (self.repo / name).read_text()
                (self.repo / name).write_text(original + "changed\n")
                try:
                    output = self.package(GITHUB_ACTIONS="true", HIDE_WEB_TYPECHECKED_SHA=self.commit)
                    self.assertIn("cannot delegate the web typecheck with changed web inputs or lockfile", output)
                    self.assertNotIn("bundle without compiler", output)
                    self.assertNotIn("compiler required", output)
                finally:
                    (self.repo / name).write_text(original)

    def test_untracked_web_input_cannot_claim_the_committed_typecheck(self):
        (self.repo / "web/added.ts").write_text("const value: number = 'invalid';\n")
        output = self.package(GITHUB_ACTIONS="true", HIDE_WEB_TYPECHECKED_SHA=self.commit)
        self.assertIn("cannot delegate the web typecheck with changed web inputs or lockfile", output)
        self.assertNotIn("bundle without compiler", output)


class PackageTypecheckWorkflow(unittest.TestCase):
    def test_only_the_initial_package_build_receives_the_optional_source_override(self):
        package = yaml.safe_load((WORKFLOWS / "package.yml").read_text())
        option = package[True]["workflow_call"]["inputs"]["web-typechecked"]
        self.assertEqual(option["type"], "boolean")
        self.assertIs(option["default"], False)
        expected = "${{ inputs.web-typechecked && (inputs.source-sha || github.sha) || '' }}"
        for name in ("package", "macos-compile"):
            job = package["jobs"][name]
            self.assertNotIn("HIDE_WEB_TYPECHECKED_SHA", job.get("env", {}))
            delegated = [step for step in job["steps"] if "HIDE_WEB_TYPECHECKED_SHA" in step.get("env", {})]
            self.assertEqual(len(delegated), 1)
            self.assertEqual(delegated[0]["env"]["HIDE_WEB_TYPECHECKED_SHA"], expected)
            self.assertIn("bash scripts/verify-web.sh desktop package", delegated[0]["run"])
        assembly = package["jobs"]["macos-package"]
        self.assertNotIn("HIDE_WEB_TYPECHECKED_SHA", assembly.get("env", {}))
        for step in assembly["steps"]:
            self.assertNotIn("HIDE_WEB_TYPECHECKED_SHA", step.get("env", {}))

    def test_pr_delegates_only_to_a_required_check_of_the_same_source_and_toolchain(self):
        verify = yaml.safe_load((WORKFLOWS / "pr.yml").read_text())["jobs"]
        self.assertEqual(verify["package"]["with"]["web-typechecked"],
                         "${{ contains(fromJSON(needs.plan.outputs.lanes), 'checks') }}")
        self.assertNotIn("source-sha", verify["package"]["with"])
        self.assertIn("checks", verify["verify"]["needs"])
        self.assertIn("package", verify["verify"]["needs"])
        commands = "\n".join(step.get("run", "") for step in verify["checks"]["steps"])
        self.assertIn("bash scripts/verify-web.sh web build", commands)
        self.assertIn("bash scripts/verify-web.sh install", commands)
        package = yaml.safe_load((WORKFLOWS / "package.yml").read_text())["jobs"]
        for job in (verify["checks"], package["package"], package["macos-compile"]):
            for action, key, expected in (("actions/setup-node@", "node-version", 22),
                                          ("pnpm/action-setup@", "version", 10)):
                step = next(step for step in job["steps"] if step.get("uses", "").startswith(action))
                self.assertEqual(step["with"][key], expected)

    def test_candidate_delegates_to_the_verify_run_bound_into_its_receipt(self):
        jobs = yaml.safe_load((WORKFLOWS / "release-candidate.yml").read_text())["jobs"]
        self.assertIs(jobs["packages"]["with"]["web-typechecked"], True)
        self.assertEqual(jobs["packages"]["with"]["source-sha"], "${{ inputs.public_sha }}")
        self.assertEqual(jobs["packages"]["needs"], "validate")
        source = next(step for step in jobs["validate"]["steps"] if step.get("id") == "source")
        self.assertEqual(source["env"]["VERIFICATION_RUN"], "${{ inputs.verification_run_id }}")
        self.assertEqual(source["env"]["PUBLIC_SHA"], "${{ inputs.public_sha }}")
        self.assertIn("node scripts/release-candidate.mjs source", source["run"])
        receipt = next(step for step in jobs["receipt"]["steps"] if "RECEIPT" in step.get("env", {}))
        self.assertEqual(receipt["env"]["RECEIPT"], "${{ needs.validate.outputs.receipt }}")


if __name__ == "__main__":
    unittest.main()
