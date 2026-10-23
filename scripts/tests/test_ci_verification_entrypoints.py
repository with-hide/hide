"""Cheap subprocess-boundary checks; no cargo build or package install.

The external commands record the argv and environment received by CI, and
fail on demand. Real build freshness remains test_verification_builds.py's job.
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


class EntryPoints(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        (self.root / "scripts").mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ["verify-cargo.sh", "verify-web.sh", "toolchain-env.sh"]:
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        for name in ["cargo", "pnpm", "rustc"]:
            file = self.bin / name
            file.write_text("#!/usr/bin/env python3\n"
                            "import json, os, sys\n"
                            "with open(os.environ['RECORD'], 'a') as f:\n"
                            " f.write(json.dumps({'argv': [os.path.basename(sys.argv[0]), *sys.argv[1:]], 'cwd': os.getcwd(), 'env': dict(os.environ)})+'\\n')\n"
                            "if sys.argv[1:] == ['--version']:\n"
                            " print(os.path.basename(sys.argv[0]) + ' 0.0.0-fixture')\n"
                            " sys.exit(0)\n"
                            "sys.exit(int(os.environ.get('FAIL_COMMAND', '0')))\n")
            file.chmod(0o755)
        self.record = self.root / "record.jsonl"
        self.env = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                    "RECORD": str(self.record), "CARGO_TARGET_DIR": str(self.root / "foreign"),
                    "HERDR_SOCKET_PATH": "operator-server", "HERDR_ENV": "1",
                    "HCOORD_HOME": "operator-legacy-state",
                    "HIDE_E2E_HERDR_BIN": "explicit-test-binary"}

    def run_entry(self, script, *args, **env):
        return subprocess.run(["bash", str(self.root / "scripts" / script), *args],
                              env={**self.env, **env}, capture_output=True, text=True, timeout=10)

    def records(self):
        """The work calls, without the program name and the version report."""
        every = [json.loads(line) for line in self.record.read_text().splitlines()]
        return [{**r, "argv": r["argv"][1:]} for r in every if r["argv"][1:] != ["--version"]]

    def enable_package_scripts(self):
        """Run the real package scripts, replacing only external Node tools.

        Counts at this boundary are the requested work budget, not internal
        wiring. A duplicated typecheck or a skipped prerequisite is observable.
        """
        for package in ("web", "desktop"):
            (self.root / package).mkdir()
            shutil.copyfile(ROOT / package / "package.json", self.root / package / "package.json")
        program = '''#!/usr/bin/env python3
import json, os, pathlib, shlex, subprocess, sys
name, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ['RECORD'], 'a') as record:
    record.write(json.dumps({'argv': [name, *args], 'cwd': os.getcwd()}) + '\\n')
if name == os.environ.get('FAIL_PROGRAM'):
    sys.exit(23)
if name == 'pnpm':
    if args[0] == 'install':
        sys.exit(0)
    if args[0] == '--dir':
        os.chdir(args[1])
        args = args[2:]
    if args[0] == 'exec':
        sys.exit(subprocess.run(args[1:]).returncode)
    script = json.loads(pathlib.Path('package.json').read_text())['scripts'][args[0]]
    sys.exit(subprocess.run(script + ' ' + shlex.join(args[1:]), shell=True).returncode)
if name == 'playwright' and '--list' in args:
    tests = ['a.spec.ts › one', 'a.spec.ts › two']
    if '--test-list' in args:
        tests = pathlib.Path(args[args.index('--test-list') + 1]).read_text().splitlines()
    for test in tests:
        file, title = test.split(' › ', 1)
        print(f'  [background] › {file}:1:1 › {title}')
'''
        for name in ("pnpm", "tsc", "vite", "eslint", "vitest", "node", "playwright", "xvfb-run"):
            file = self.bin / name
            file.write_text(program if name != "xvfb-run" else '#!/bin/sh\nshift\nexec "$@"\n')
            file.chmod(0o755)

    def tool_calls(self, tool, package):
        return [r["argv"][1:] for r in map(json.loads, self.record.read_text().splitlines())
                if r["argv"][0] == tool and Path(r["cwd"]).name == package]

    def workflow_step(self, workflow, job, name, **env):
        steps = yaml.safe_load((ROOT / ".github/workflows" / workflow).read_text())["jobs"][job]["steps"]
        command = next(step["run"] for step in steps if step.get("name") == name)
        return subprocess.run(["bash", "-euo", "pipefail", "-c", command], cwd=self.root,
                              env={**self.env, **env}, capture_output=True, text=True, timeout=15)

    def test_full_gate_typechecks_and_builds_each_input_once(self):
        self.enable_package_scripts()
        result = self.run_entry("verify-web.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.tool_calls("tsc", "web"), [["--noEmit"]])
        # Main and e2e are different TypeScript inputs, so both remain.
        self.assertEqual(self.tool_calls("tsc", "desktop"), [["--noEmit"], ["--noEmit", "-p", "e2e"]])
        self.assertEqual(self.tool_calls("vite", "web"), [["build"]])
        self.assertEqual(self.tool_calls("node", "desktop").count(["scripts/build.mjs"]), 1)
        for package in ("web", "desktop"):
            self.assertEqual(self.tool_calls("vitest", package), [["run"]])
            self.assertEqual(len(self.tool_calls("eslint", package)), 1)

    def test_ci_web_step_typechecks_once_and_stops_on_type_errors(self):
        self.enable_package_scripts()
        name = "Web shell typecheck, lint, unit test, and build"
        result = self.workflow_step("pr.yml", "checks", name)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.tool_calls("tsc", "web"), [["--noEmit"]])
        self.assertEqual(self.tool_calls("vite", "web"), [["build"]])
        self.record.unlink()
        result = self.workflow_step("pr.yml", "checks", name, FAIL_PROGRAM="tsc")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertFalse(self.tool_calls("vite", "web"))
        self.assertFalse(self.tool_calls("vitest", "web"))

    def test_full_gate_stops_on_a_type_error_before_bundling_or_desktop(self):
        self.enable_package_scripts()
        result = self.run_entry("verify-web.sh", FAIL_PROGRAM="tsc")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertFalse(self.tool_calls("vite", "web"))
        self.assertFalse(self.tool_calls("tsc", "desktop"))

    def test_standalone_web_build_and_package_e2e_keep_typecheck(self):
        self.enable_package_scripts()
        for action in ("build", "e2e"):
            with self.subTest(action=action):
                self.record.unlink(missing_ok=True)
                result = subprocess.run(["pnpm", "--dir", "web", action], cwd=self.root,
                                        env=self.env, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.tool_calls("tsc", "web"), [["--noEmit"]])
                self.assertEqual(self.tool_calls("vite", "web"), [["build"]])
                self.record.unlink()
                result = subprocess.run(["pnpm", "--dir", "web", action], cwd=self.root,
                                        env={**self.env, "FAIL_PROGRAM": "tsc"},
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertFalse(self.tool_calls("vite", "web"))
                self.assertFalse(self.tool_calls("playwright", "web"))
                self.record.unlink()

    def test_ci_desktop_discovers_twice_but_builds_once_on_each_system(self):
        self.enable_package_scripts()
        shutil.copyfile(ROOT / "scripts/e2e-shard.py", self.root / "scripts/e2e-shard.py")
        (self.root / "desktop/e2e").mkdir()
        (self.root / "desktop/e2e/shard-durations.json").write_text(
            json.dumps({"a.spec.ts › one": 1, "a.spec.ts › two": 1}))
        for system, shard in (("macOS", "1/2"), ("Windows", "2/2"), ("Linux", "1/1")):
            with self.subTest(system=system):
                self.record.unlink(missing_ok=True)
                result = self.workflow_step("desktop-e2e.yml", "e2e", "Desktop app end-to-end",
                                            RUNNER_OS=system, RUNNER_TEMP=str(self.root),
                                            HIDE_E2E_SCREENSHOT_DIR=str(self.root / "screenshots"),
                                            GREP="@platform", SHARD=shard)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.tool_calls("node", "desktop"), [["scripts/build.mjs"]])
                calls = self.tool_calls("playwright", "desktop")
                self.assertEqual(len(calls), 3 if shard.endswith("/2") else 1)
                self.assertTrue(all("--grep" in call and "@platform" in call for call in calls))
                self.assertNotIn("--list", calls[-1])
                if shard.endswith("/2"):
                    self.assertTrue(all("--list" in call for call in calls[:2]))
                    self.assertTrue(all("--no-deps" in call for call in calls))
                    self.assertIn("--test-list", calls[-1])
                self.record.unlink()

    def test_desktop_execution_always_builds_and_propagates_prerequisite_and_test_failures(self):
        self.enable_package_scripts()
        # No receipt or pre-existing output is required for a safe direct run.
        for fail, expected in (("node", 23), ("playwright", 23), ("", 0)):
            with self.subTest(fail=fail):
                self.record.unlink(missing_ok=True)
                result = self.run_entry("verify-web.sh", "desktop", "e2e", "--grep", "one", FAIL_PROGRAM=fail)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual(self.tool_calls("node", "desktop"), [["scripts/build.mjs"]])
                self.assertEqual(len(self.tool_calls("playwright", "desktop")), 0 if fail == "node" else 1)
                self.record.unlink()

    def test_discovery_never_builds_or_executes_tests_and_propagates_failures(self):
        self.enable_package_scripts()
        result = self.run_entry("verify-web.sh", "desktop", "e2e-list", "--grep", "one", FAIL_PROGRAM="playwright")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertFalse(self.tool_calls("node", "desktop"))
        self.assertEqual(self.tool_calls("playwright", "desktop"), [["test", "--list", "--grep", "one"]])

    def test_the_toolchain_is_named_on_stderr_before_cargo_runs(self):
        result = self.run_entry("verify-cargo.sh", "metadata", "--format-version", "1")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = [json.loads(line)["argv"] for line in self.record.read_text().splitlines()]
        self.assertEqual(calls, [["rustc", "--version"], ["cargo", "--version"],
                                 ["cargo", "metadata", "--locked", "--format-version", "1"]])
        self.assertEqual(result.stderr.splitlines()[:2], ["rustc 0.0.0-fixture", "cargo 0.0.0-fixture"])
        self.assertNotIn("0.0.0-fixture", result.stdout)

    def test_scoped_cargo_keeps_target_and_drops_operator_identity(self):
        result = self.run_entry("verify-cargo.sh", "test-scoped", "-p", "hide-platform", "--", "--ignored")
        self.assertEqual(result.returncode, 0, result.stderr)
        received = self.records()[0]
        self.assertEqual(received["argv"], ["test", "--locked", "-p", "hide-platform", "--", "--ignored"])
        self.assertEqual(received["cwd"], str(self.root))
        self.assertEqual(received["env"]["CARGO_TARGET_DIR"], str(self.root / "target"))
        self.assertFalse(any(key.startswith(("HERDR_", "HCOORD_")) for key in received["env"]))
        self.assertEqual(received["env"]["HIDE_E2E_HERDR_BIN"], "explicit-test-binary")
        self.assertEqual(self.run_entry("verify-cargo.sh", "check", "--workspace", FAIL_COMMAND="23").returncode, 23)

    def test_invalid_cargo_mode_or_artifact_redirection_never_calls_cargo(self):
        for args in [("unknown",), ("build", "--target-dir", "elsewhere"),
                     ("check", "--config=build.target-dir=elsewhere"), ("clippy", "--manifest-path=../Cargo.toml"),
                     ("test-scoped", *(["arg"] * 129))]:
            with self.subTest(args=args[:2]):
                self.assertEqual(self.run_entry("verify-cargo.sh", *args).returncode, 2)
        self.assertFalse(self.record.exists())

    def test_sealed_test_and_lint_arguments_remain_unchanged(self):
        self.assertEqual(self.run_entry("verify-cargo.sh", "test", "selected", "--", "--ignored").returncode, 0)
        self.assertEqual(self.records()[0]["argv"], ["test", "--locked", "--workspace", "selected", "--", "--ignored"])
        self.assertEqual(self.run_entry("verify-cargo.sh", "lint").returncode, 0)
        self.assertEqual([r["argv"] for r in self.records()[1:]],
                         [["fmt", "--all", "--check"], ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"]])

    def test_web_scoped_step_forwards_playwright_failure_and_package(self):
        result = self.run_entry("verify-web.sh", "web", "e2e", "--shard=1/4", FAIL_COMMAND="19")
        self.assertEqual(result.returncode, 19)
        self.assertEqual(self.records()[0]["argv"], ["--dir", "web", "exec", "playwright", "test", "--shard=1/4"])
        self.assertEqual(self.run_entry("verify-web.sh", "desktop", "package").returncode, 0)
        self.assertEqual(self.records()[1]["argv"], ["--dir", "desktop", "package"])

    def test_web_install_is_locked_and_invalid_package_actions_fail(self):
        self.assertEqual(self.run_entry("verify-web.sh", "install", "--ignore-scripts").returncode, 0)
        self.assertEqual(self.records()[0]["argv"], ["install", "--frozen-lockfile", "--ignore-scripts"])
        for args in [("install", "--no-frozen-lockfile"), ("elsewhere", "test"),
                     ("web", "package"), ("web", "test:e2e"), ("retired", "e2e"), ("desktop", "unknown")]:
            with self.subTest(args=args):
                self.assertEqual(self.run_entry("verify-web.sh", *args).returncode, 2)
        self.assertEqual(len(self.records()), 1)

    def test_electron_acquisition_runs_once_and_propagates_the_installer_failure(self):
        result = self.run_entry("verify-web.sh", "desktop", "electron-install", FAIL_COMMAND="17")
        self.assertEqual(result.returncode, 17)
        self.assertEqual([record["argv"] for record in self.records()],
                         [["--dir", "desktop", "exec", "install-electron", "--no"]])
        for args in [("web", "electron-install"), ("other", "electron-install"),
                     ("desktop", "electron-install", "--force")]:
            with self.subTest(args=args):
                self.assertEqual(self.run_entry("verify-web.sh", *args).returncode, 2)
        self.assertEqual(len(self.records()), 1)


if __name__ == "__main__":
    unittest.main()
