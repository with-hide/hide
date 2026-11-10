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
        for name in ["verify-cargo.sh", "verify-web.sh", "toolchain-env.sh", "build-relay-wasm.sh"]:
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        for name in ["cargo", "pnpm", "rustc", "rustup"]:
            file = self.bin / name
            file.write_text("#!/usr/bin/env python3\n"
                            "import json, os, sys\n"
                            "if os.path.basename(sys.argv[0]) == os.environ.get('FAIL_PROGRAM'):\n"
                            " sys.exit(23)\n"
                            "with open(os.environ['RECORD'], 'a') as f:\n"
                            " f.write(json.dumps({'argv': [os.path.basename(sys.argv[0]), *sys.argv[1:]], 'cwd': os.getcwd(), 'env': dict(os.environ)})+'\\n')\n"
                            "if sys.argv[1:] == ['--version']:\n"
                            " print(os.path.basename(sys.argv[0]) + ' 0.0.0-fixture')\n"
                            " sys.exit(0)\n"
                            "sys.exit(int(os.environ.get('FAIL_COMMAND', '0')))\n")
            file.chmod(0o755)
        # Run the actual WASM preparation script with external compilers and
        # the binding generator replaced. The generated ABI must exist before
        # a web consumer starts, even for the packager's bundle-only path.
        (self.root / "Cargo.lock").write_text('[[package]]\nname = "wasm-bindgen"\nversion = "0.0.0-fixture"\n')
        generator = self.root / "target/relay-tools/bin/wasm-bindgen"
        generator.parent.mkdir(parents=True)
        generator.write_text('''#!/usr/bin/env python3
import os, pathlib, sys
if sys.argv[1:] == ['--version']:
    print('wasm-bindgen 0.0.0-fixture')
    sys.exit(0)
if os.environ.get('FAIL_PROGRAM') == 'wasm-bindgen':
    sys.exit(23)
output = pathlib.Path(sys.argv[sys.argv.index('--out-dir') + 1])
output.mkdir(parents=True, exist_ok=True)
for name in ('relay.js', 'relay.d.ts', 'relay_bg.wasm'):
    (output / name).write_text('fixture')
''')
        generator.chmod(0o755)
        # hide-vt's preparation, standing in for the download: it records each
        # call and fails on demand (scripts/prepare-vt-inputs.py).
        (self.root / "scripts" / "prepare-vt-inputs.py").write_text(
            "import os, sys\n"
            "with open(os.environ['PREPARE_RECORD'], 'a') as f:\n"
            " f.write(' '.join(sys.argv[1:]) + '\\n')\n"
            "if os.environ.get('FAIL_PREPARE'):\n"
            " print('prepare-vt-inputs: could not download the pinned Zig', file=sys.stderr)\n"
            " sys.exit(1)\n")
        self.prepared = self.root / "prepare.log"
        self.record = self.root / "record.jsonl"
        self.env = {**os.environ, "PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                    "RECORD": str(self.record), "PREPARE_RECORD": str(self.prepared),
                    "CARGO_TARGET_DIR": str(self.root / "foreign"),
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
if name in ('tsc', 'vite', 'vitest') and pathlib.Path.cwd().name == 'web':
    if not pathlib.Path('../hide-relay-protocol/pkg/relay_bg.wasm').is_file():
        sys.exit('web consumer started without the relay ABI')
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

    def test_windows_runs_both_unit_suites_without_repeating_static_checks(self):
        self.enable_package_scripts()
        steps = yaml.safe_load((ROOT / ".github/workflows/pr.yml").read_text())["jobs"]["windows-check"]["steps"]
        for package, name in (("web", "Web shell unit suite"), ("desktop", "Desktop app unit suite")):
            with self.subTest(package=package):
                self.record.unlink(missing_ok=True)
                result = self.workflow_step("pr.yml", "windows-check", name)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.tool_calls("vitest", package), [["run"]])
                self.record.unlink()
                result = self.workflow_step("pr.yml", "windows-check", name, FAIL_PROGRAM="vitest")
                self.assertEqual(result.returncode, 23, result.stderr)
        # Check every step, so putting a duplicate back under another name fails.
        commands = "\n".join(step.get("run", "") for step in steps)
        for package in ("web", "desktop"):
            for action in ("typecheck", "lint", "build"):
                self.assertNotIn(f"verify-web.sh {package} {action}", commands)
        linux = yaml.safe_load((ROOT / ".github/workflows/pr.yml").read_text())["jobs"]["checks"]["steps"]
        linux_commands = "\n".join(step.get("run", "") for step in linux)
        for command in ("web build", "web lint", "desktop typecheck", "desktop lint"):
            self.assertIn(f"verify-web.sh {command}", linux_commands)

    def test_web_shards_share_current_run_output_and_single_shard_keeps_its_build(self):
        lane = yaml.safe_load((ROOT / ".github/workflows/web-e2e.yml").read_text())
        daemon, build, e2e = (lane["jobs"][key] for key in ("build", "build-web", "e2e"))
        self.assertEqual(build["if"], "${{ inputs.shards > 1 }}")
        self.assertEqual(daemon["if"], build["if"])
        for job in (daemon, build):
            self.assertNotIn("needs", job)
            self.assertNotIn("continue-on-error", job)
        self.assertEqual(e2e["needs"], ["build", "build-web"])
        self.assertEqual(e2e["if"], "${{ !cancelled() && "
                         "contains(fromJSON('[\"success\",\"skipped\"]'), needs.build.result) && "
                         "contains(fromJSON('[\"success\",\"skipped\"]'), needs.build-web.result) }}")
        self.assertNotIn("continue-on-error", build)
        self.assertEqual(build["runs-on"], e2e["runs-on"])
        self.assertEqual(build.get("env", {}), e2e.get("env", {}))

        def one(job, *, name=None, uses=None):
            return next(step for step in job["steps"]
                        if (step.get("name") == name if name else step.get("uses", "").startswith(uses)))

        # Both preparations consume the same commit, lockfile and build inputs.
        for action in ("actions/checkout@", "pnpm/action-setup@", "actions/setup-node@"):
            left, right = (one(job, uses=action) for job in (build, e2e))
            self.assertEqual(left["uses"], right["uses"])
            self.assertEqual(left.get("with", {}), right.get("with", {}))
        for name in ("Install web dependencies", "Prepare the relay WebAssembly", "Build the web shell"):
            left, right = (one(job, name=name) for job in (build, e2e))
            self.assertEqual(left["run"], right["run"])
            self.assertEqual(left.get("env", {}), right.get("env", {}))
        for job in (build, e2e):
            prepare = one(job, name="Prepare the relay WebAssembly")
            shell = one(job, name="Build the web shell")
            self.assertLess(job["steps"].index(prepare), job["steps"].index(shell))
            self.assertEqual(shell["timeout-minutes"], 5)
        self.assertNotIn("if", one(build, name="Build the web shell"))
        self.assertEqual(one(e2e, name="Build the web shell")["if"], "${{ inputs.shards == 1 }}")
        upload = one(build, name="Keep the web shell for the shards")
        download = one(e2e, name="Fetch the web shell from the build job")
        self.assertEqual(upload["uses"], "actions/upload-artifact@v7")
        self.assertEqual(upload["with"], {"name": "web-dist-${{ inputs.lane }}", "path": "web/dist",
                                          "if-no-files-found": "error", "retention-days": 1})
        self.assertEqual(download["uses"], "actions/download-artifact@v8")
        # No token/run-id lookup: download-artifact defaults to this run.
        self.assertEqual(download["with"], {"name": upload["with"]["name"], "path": "web/dist"})
        self.assertEqual(download["if"], "${{ inputs.shards > 1 }}")
        self.assertLess(e2e["steps"].index(download),
                        e2e["steps"].index(one(e2e, name="End-to-end")))
        for step in (upload, download):
            self.assertNotIn("continue-on-error", step)
        # Flaky tests are filed with the development issues App's token, not this one.
        self.assertEqual(lane["permissions"], {"contents": "read"})

        # Exercise the actual build command's prerequisite failure boundary.
        self.enable_package_scripts()
        for job in ("build-web", "e2e"):
            with self.subTest(job=job):
                self.record.unlink(missing_ok=True)
                result = self.workflow_step("web-e2e.yml", job, "Build the web shell")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.tool_calls("tsc", "web"), [["--noEmit"]])
                self.assertEqual(self.tool_calls("vite", "web"), [["build"]])
                self.record.unlink()
                result = self.workflow_step("web-e2e.yml", job, "Build the web shell", FAIL_PROGRAM="tsc")
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertFalse(self.tool_calls("vite", "web"))

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
                self.assertTrue(all("--grep-invert" not in call for call in calls))
                self.assertNotIn("--list", calls[-1])
                if shard.endswith("/2"):
                    self.assertTrue(all("--list" in call for call in calls[:2]))
                    self.assertTrue(all("--no-deps" in call for call in calls))
                    self.assertIn("--test-list", calls[-1])
                self.record.unlink()

    def test_web_shard_discovers_twice_and_prepares_phone_once(self):
        self.enable_package_scripts()
        shutil.copyfile(ROOT / "scripts/e2e-shard.py", self.root / "scripts/e2e-shard.py")
        (self.root / "web/e2e").mkdir()
        (self.root / "web/e2e/shard-durations.json").write_text(
            json.dumps({"a.spec.ts › one": 1, "a.spec.ts › two": 1}))
        result = self.workflow_step("web-e2e.yml", "e2e", "End-to-end",
                                    RUNNER_TEMP=str(self.root),
                                    HIDE_E2E_SCREENSHOT_DIR=str(self.root / "screenshots"),
                                    GREP="", SPECS="", REPEAT="", SHARD="1/2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.tool_calls("node", "web"), [["scripts/build-phone.mjs"]])
        calls = self.tool_calls("playwright", "web")
        self.assertEqual(len(calls), 3)
        self.assertTrue(all("--list" in call for call in calls[:2]))
        self.assertNotIn("--list", calls[-1])
        self.assertTrue(all("--grep-invert" not in call for call in calls))

    def test_web_repeat_runs_the_named_specs_n_times_with_no_retry(self):
        self.enable_package_scripts()
        shutil.copyfile(ROOT / "scripts/ci-repeat.py", self.root / "scripts/ci-repeat.py")
        result = self.workflow_step("web-e2e.yml", "e2e", "End-to-end",
                                    RUNNER_TEMP=str(self.root), GITHUB_STEP_SUMMARY=str(self.root / "summary.md"),
                                    HIDE_E2E_SCREENSHOT_DIR=str(self.root / "screenshots"),
                                    GREP="", SPECS="web/e2e/s3.spec.ts web/e2e/s7.spec.ts", REPEAT="10", SHARD="1/1")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.tool_calls("playwright", "web")
        self.assertEqual(calls, [[
            "test", "--retries=0", "--repeat-each=10", r"web/e2e/s3\.spec\.ts$", r"web/e2e/s7\.spec\.ts$",
        ]])

    def test_web_repeat_refuses_a_bad_repeat_and_a_second_shard(self):
        self.enable_package_scripts()
        shutil.copyfile(ROOT / "scripts/ci-repeat.py", self.root / "scripts/ci-repeat.py")
        for repeat, shard in (("", "1/1"), ("0", "1/1"), ("ten", "1/1"), ("10", "1/2")):
            with self.subTest(repeat=repeat, shard=shard):
                result = self.workflow_step("web-e2e.yml", "e2e", "End-to-end",
                                            RUNNER_TEMP=str(self.root), GITHUB_STEP_SUMMARY=str(self.root / "summary.md"),
                                            HIDE_E2E_SCREENSHOT_DIR=str(self.root / "screenshots"),
                                            GREP="", SPECS="web/e2e/s3.spec.ts", REPEAT=repeat, SHARD=shard)
                self.assertNotEqual(result.returncode, 0)

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
        for package in ("web", "desktop"):
            with self.subTest(package=package):
                self.record.unlink(missing_ok=True)
                result = self.run_entry("verify-web.sh", package, "e2e-list", "--grep", "one", FAIL_PROGRAM="playwright")
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertFalse(self.tool_calls("node", package))
                self.assertFalse(self.tool_calls("cargo", self.root.name))
                self.assertEqual(self.tool_calls("playwright", package), [["test", "--list", "--grep", "one"]])

    def test_bundle_only_packaging_prepares_abi_and_stops_on_preparation_failure(self):
        self.enable_package_scripts()
        for fail in ("cargo", "wasm-bindgen", ""):
            with self.subTest(fail=fail):
                shutil.rmtree(self.root / "hide-relay-protocol", ignore_errors=True)
                self.record.unlink(missing_ok=True)
                result = subprocess.run(["pnpm", "--dir", "web", "bundle"], cwd=self.root,
                                        env={**self.env, "FAIL_PROGRAM": fail},
                                        capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 23 if fail else 0, result.stderr)
                self.assertEqual(self.tool_calls("vite", "web"), [] if fail else [["build"]])
                self.assertFalse(self.tool_calls("tsc", "web"))

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

    def test_a_hide_pane_s_handles_never_reach_a_test_but_its_inputs_do(self):
        pane = {"HIDE_STATE_DIR": "operator-state", "HIDE_CAP_REF": "operator-credential",
                "HIDE_PANE_ID": "h1:p2", "HIDE_INSTALL": "app", "HIDE_INSTALL_CLI": "/operator/hide",
                "HIDE_PORT": "7777", "HIDE_SOMETHING_NEW": "a daemon setting added later"}
        inputs = {"HIDE_TEST_HOLDER_BIN": "holder", "HIDE_E2E_WSL_DISTRO": "distro", "HIDE_BUILD_ORDER": "7",
                  "HIDE_VT_ZIG_CACHE": "zig", "HIDE_HOLDER_CONPTY": "conpty", "HIDE_VERSION": "1.2.3",
                  "HIDE_AGY_BINARY": "agy"}
        for mode in (("test",), ("test-scoped", "-p", "hide-kit"), ("nextest", "-p", "hide-kit"), ("check", "--workspace")):
            with self.subTest(mode=mode[0]):
                self.record.unlink(missing_ok=True)
                self.assertEqual(self.run_entry("verify-cargo.sh", *mode, **pane, **inputs).returncode, 0)
                received = self.records()[0]["env"]
                self.assertEqual([key for key in received if key in pane], [])
                self.assertEqual({key: received[key] for key in inputs}, inputs)

    def test_a_test_run_reads_none_of_the_account_s_git_settings(self):
        for mode in (("test",), ("test-scoped", "-p", "hide-kit"), ("nextest", "-p", "hide-kit")):
            with self.subTest(mode=mode[0]):
                self.record.unlink(missing_ok=True)
                self.assertEqual(self.run_entry("verify-cargo.sh", *mode, GIT_CONFIG_GLOBAL="~/.gitconfig").returncode, 0)
                received = self.records()[0]["env"]
                self.assertEqual(received["GIT_CONFIG_GLOBAL"], "/dev/null")
                self.assertEqual(received["GIT_CONFIG_NOSYSTEM"], "1")
        # A build or a lint runs no test, and leaves cargo's own git reads alone.
        self.record.unlink(missing_ok=True)
        self.assertEqual(self.run_entry("verify-cargo.sh", "check", "--workspace", GIT_CONFIG_GLOBAL="~/.gitconfig").returncode, 0)
        self.assertEqual(self.records()[0]["env"]["GIT_CONFIG_GLOBAL"], "~/.gitconfig")

    def test_a_build_that_can_reach_hide_vt_prepares_its_inputs_first(self):
        self.assertEqual(self.run_entry("verify-cargo.sh", "test").returncode, 0)
        self.assertEqual(self.run_entry("verify-cargo.sh", "nextest", "-p", "hide-vt").returncode, 0)
        self.assertEqual(self.prepared.read_text().splitlines(), ["--quiet", "--quiet"])
        self.assertEqual(self.run_entry("verify-cargo.sh", "test-scoped", "-p", "hide-platform").returncode, 0)
        self.assertEqual(len(self.prepared.read_text().splitlines()), 2)

    def test_a_failed_preparation_stops_before_cargo_and_names_the_command_to_run(self):
        result = self.run_entry("verify-cargo.sh", "check", "--workspace", FAIL_PREPARE="1")
        self.assertEqual(result.returncode, 1)
        self.assertIn("could not download the pinned Zig", result.stderr)
        self.assertIn("Run `python3 scripts/prepare-vt-inputs.py` once on this machine where", result.stderr)
        self.assertEqual(self.records(), [])

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
        self.enable_package_scripts()
        result = self.run_entry("verify-web.sh", "web", "e2e", "--shard=1/4", FAIL_PROGRAM="playwright")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertEqual(self.tool_calls("playwright", "web"), [["test", "--shard=1/4"]])
        self.assertEqual(self.tool_calls("node", "web"), [["scripts/build-phone.mjs"]])

    def test_desktop_packaging_modes_reach_the_packager(self):
        self.assertEqual(self.run_entry("verify-web.sh", "desktop", "package").returncode, 0)
        self.assertEqual(self.records()[0]["argv"], ["--dir", "desktop", "package"])
        # The macOS lane's two halves: the release binaries alone, then the host build and the app from them.
        self.assertEqual(self.run_entry("verify-web.sh", "desktop", "package", "compile").returncode, 0)
        self.assertEqual(self.records()[1]["argv"], ["--dir", "desktop", "exec", "node", "scripts/package.mjs", "compile"])
        self.assertEqual(self.run_entry("verify-web.sh", "desktop", "package", "assemble").returncode, 0)
        self.assertEqual(self.records()[2]["argv"], ["--dir", "desktop", "package", "assemble"])

    def test_web_install_is_locked_and_invalid_package_actions_fail(self):
        self.assertEqual(self.run_entry("verify-web.sh", "install", "--ignore-scripts").returncode, 0)
        self.assertEqual(self.records()[0]["argv"], ["install", "--frozen-lockfile", "--ignore-scripts"])
        for args in [("install", "--no-frozen-lockfile"), ("elsewhere", "test"),
                     ("web", "package"), ("desktop", "bundle"), ("desktop", "build:phone"), ("web", "test:e2e"), ("retired", "e2e"), ("desktop", "unknown"),
                     ("desktop", "package", "sign"), ("desktop", "package", "compile", "assemble")]:
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
