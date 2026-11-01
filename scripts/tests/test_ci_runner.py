"""The Mac mini CI runner: which jobs reach it and what a job leaves.

The contract is docs/BUILD.md, The Mac mini CI runner. The hooks run
as the runner's account before and after every job; these run them the same
way, with the variables the runner gives a hook.
"""
from pathlib import Path
import json
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/ci-runner"
WORKFLOWS = ROOT / ".github/workflows"


def hook(name, env, cwd=None):
    # The shell and flags the runner starts a hook with.
    return subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", str(RUNNER / name)],
                          env={"PATH": "/usr/bin:/bin", **env}, cwd=cwd, capture_output=True, text=True, timeout=30)


class Folder(unittest.TestCase):
    def setUp(self):
        runs = ROOT / "agents/runs/ci-runner-tests"
        runs.mkdir(parents=True, exist_ok=True)
        self.base = Path(tempfile.mkdtemp(dir=runs))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.base)])
        # The real path: the hooks refuse a runner folder reached through a link.
        self.runner = self.base.resolve() / "actions-runner-hide-ci"
        self.work = self.runner / "_work"
        (self.work / "_temp").mkdir(parents=True)


class JobStarted(Folder):
    def setUp(self):
        super().setUp()
        (self.work / "_actions/someone/action/v1").mkdir(parents=True)
        (self.work / "_actions/someone/action/v1/pre.js").write_text("downloaded before the hook\n")
        # The runner's worker as the hook sees it: its parent, named
        # .../bin/Runner.Worker spawnclient, which runs the steps after it.
        self.ran = self.base / "steps-ran"
        worker = self.runner / "bin/Runner.Worker"
        worker.parent.mkdir()
        worker.write_text("#!/bin/bash\n"
                          f"bash --noprofile --norc -e -o pipefail {RUNNER / 'job-started.sh'}\n"
                          f"echo ran > {self.ran}\n")
        worker.chmod(0o755)
        self.worker = worker

    def event(self, head):
        path = self.base / "event.json"
        path.write_text(json.dumps({"pull_request": {"head": {"repo": {"full_name": head}}}}))
        return {"GITHUB_REPOSITORY": "with-hide/hide", "GITHUB_EVENT_NAME": "pull_request",
                "GITHUB_EVENT_PATH": str(path), "HIDE_CI_RUNNER": str(self.runner)}

    def run_worker(self, env):
        return subprocess.run([str(self.worker), "spawnclient", "1", "2"], env={"PATH": "/usr/bin:/bin", **env},
                              capture_output=True, text=True, timeout=30)

    def test_takes_this_repositorys_branches_pushes_and_scheduled_runs(self):
        own = {"GITHUB_REPOSITORY": "with-hide/hide", "HIDE_CI_RUNNER": str(self.runner)}
        for env in [self.event("with-hide/hide"), {**own, "GITHUB_EVENT_NAME": "push"},
                    {**own, "GITHUB_EVENT_NAME": "schedule"}, {**own, "GITHUB_EVENT_NAME": "workflow_dispatch"}]:
            with self.subTest(event=env["GITHUB_EVENT_NAME"]):
                result = self.run_worker(env)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(self.ran.exists())
                self.ran.unlink()
        # The job's HOME and TMPDIR exist again before its first step.
        self.assertTrue((self.runner / "home").is_dir() and (self.runner / "tmp").is_dir())

    def test_a_fork_pull_request_is_killed_before_any_step_and_leaves_nothing(self):
        result = self.run_worker(self.event("someone/hide"))
        self.assertEqual(result.returncode, -9)
        self.assertIn("refused a pull request from someone/hide", result.stdout)
        self.assertFalse(self.ran.exists())
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])

    def test_refuses_what_it_cannot_tell_is_this_repositorys_code(self):
        unreadable = {**self.event("with-hide/hide"), "GITHUB_EVENT_PATH": str(self.base / "missing.json")}
        for env in [unreadable, {**self.event("with-hide/hide"), "GITHUB_EVENT_NAME": "pull_request_target",
                                 "GITHUB_EVENT_PATH": ""},
                    {**self.event("with-hide/hide"), "GITHUB_REPOSITORY": "someone/hide"},
                    {**self.event("with-hide/hide"), "GITHUB_EVENT_NAME": ""},
                    {**self.event("with-hide/hide"), "HIDE_CI_RUNNER": ""}]:
            with self.subTest(env=env):
                result = hook("job-started.sh", env)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("::error::this runner takes only with-hide/hide's own code", result.stdout)

    def test_where_the_parent_is_not_the_worker_the_whole_runner_stops_until_restored(self):
        (self.runner / ".runner").write_text("{}\n")
        listener = self.runner / "bin/Runner.Listener"
        listener.write_text("#!/bin/bash\nwhile :; do sleep 1; done\n")
        listener.chmod(0o755)
        env = {"PATH": "/usr/bin:/bin", **self.event("someone/hide")}
        running = subprocess.Popen([str(listener), "run", "--startuptype", "service"], env=env)
        self.addCleanup(running.kill)
        result = hook("job-started.sh", env)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(running.wait(timeout=10), -9)
        self.assertIn("the runner stops taking jobs", result.stdout)
        self.assertEqual(sorted(p.name for p in self.runner.iterdir() if p.name.startswith(".runner")),
                         [".runner.stopped"])


class JobCompleted(Folder):
    def setUp(self):
        super().setUp()
        self.workspace = self.work / "hide/hide"
        for name in ["_work/_temp/runner-file", "_work/_actions/actions/checkout/v7/action.yml",
                     "_work/hide/hide/target/release/hided", "_work/_cache/cargo/registry/crate",
                     "_work/_tool/node/22/bin/node", "_work/.hidden", "home/.gitconfig", "home/.bun/bin/bun",
                     "tmp/.tmpXYZ/socket", "rustup/toolchains/2.0.0-aarch64-apple-darwin/bin/rustc"]:
            (self.runner / name).parent.mkdir(parents=True, exist_ok=True)
            (self.runner / name).write_text("left by a job\n")
        (self.workspace / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "2.0.0"\nprofile = "minimal"\n')
        # rustup as the runner has it: two pins installed, one the source names.
        bin = self.base / "bin"
        bin.mkdir()
        (bin / "rustup").write_text(
            "#!/bin/bash\n"
            '/bin/pwd >/dev/null 2>&1 || { echo "error: Could not locate working directory." >&2; exit 1; }\n'
            'if [[ "$*" == "toolchain list" ]]; then\n'
            '  printf "2.0.0-aarch64-apple-darwin (active)\\n1.0.0-aarch64-apple-darwin\\n"\n'
            f'else echo "$*" >> "{self.base}/rustup.log"; fi\n')
        (bin / "rustup").chmod(0o755)
        self.env = {"HIDE_CI_RUNNER": str(self.runner), "PATH": f"{bin}:/usr/bin:/bin",
                    "GITHUB_WORKSPACE": str(self.workspace)}

    def assert_emptied(self):
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])
        self.assertEqual(list((self.runner / "home").iterdir()), [])
        self.assertEqual(list((self.runner / "tmp").iterdir()), [])
        self.assertTrue((self.runner / "rustup/toolchains/2.0.0-aarch64-apple-darwin/bin/rustc").exists())

    def test_removes_what_a_job_left_and_toolchains_the_source_does_not_pin(self):
        # The runner starts the hook in the job's workspace, which the hook removes.
        result = hook("job-completed.sh", self.env, cwd=self.workspace)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_emptied()
        self.assertEqual((self.base / "rustup.log").read_text(), "toolchain uninstall 1.0.0-aarch64-apple-darwin\n")

    def test_removes_links_without_following_them(self):
        outside = self.base / "outside"
        outside.mkdir()
        (outside / "kept").write_text("not the job's\n")
        (self.runner / "home/to-outside").symlink_to(outside)
        result = hook("job-completed.sh", self.env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_emptied()
        self.assertTrue((outside / "kept").exists())

    def test_refuses_a_runner_folder_it_cannot_trust(self):
        linked = self.base / "linked"
        linked.symlink_to(self.runner)
        for env in [{**self.env, "HIDE_CI_RUNNER": str(linked)}, {**self.env, "HIDE_CI_RUNNER": ""}]:
            with self.subTest(runner=env["HIDE_CI_RUNNER"]):
                result = hook("job-completed.sh", env)
                self.assertEqual(result.returncode, 1)
                self.assertIn("nothing was removed", result.stdout)
                self.assertTrue((self.workspace / "rust-toolchain.toml").exists())


class Routing(unittest.TestCase):
    """Only verify's macOS jobs that need no screen, keychain or login session reach the runner."""

    def load(self, name):
        return yaml.safe_load((WORKFLOWS / name).read_text())

    def test_one_variable_chooses_and_a_fork_never_gets_it(self):
        plan = self.load("pr.yml")["jobs"]["plan"]["outputs"]["self-hosted-macos"]
        self.assertEqual(plan, "${{ (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name"
                               " == github.repository) && vars.HIDE_MACOS_CI_RUNNER || '' }}")
        text = "\n".join(path.read_text() for path in sorted(WORKFLOWS.glob("*.y*ml")))
        self.assertEqual(text.count("vars.HIDE_MACOS_CI_RUNNER"), 1)

    def test_only_vt_omp_and_the_macos_compile_follow_it_and_default_to_hosted(self):
        jobs = self.load("pr.yml")["jobs"]
        passing = sorted(name for name, job in jobs.items() if "self-hosted-macos" in str(job.get("with", {})))
        self.assertEqual(passing, ["omp-contract", "package", "vt"])
        for name, job, key in [("vt.yml", "build", "macos-runner"), ("omp-contract.yml", "contract", "macos-runner")]:
            with self.subTest(workflow=name):
                workflow = self.load(name)
                self.assertEqual(workflow[True]["workflow_call"]["inputs"][key]["default"], "")
                self.assertEqual(workflow["jobs"][job]["runs-on"],
                                 "${{ matrix.system == 'macos' && fromJSON(inputs.macos-runner || '\"macos-15\"') || "
                                 "fromJSON('{\"linux\":\"ubuntu-latest\",\"windows\":\"windows-latest\"}')[matrix.system] }}")
        package = self.load("package.yml")
        self.assertEqual(package[True]["workflow_call"]["inputs"]["macos-compile-runner"]["default"], "")
        self.assertEqual(package["jobs"]["macos-compile"]["runs-on"],
                         "${{ fromJSON(inputs.macos-compile-runner || '\"macos-15\"') }}")
        # The packaged app's job starts the app, which needs a login session.
        self.assertEqual(package["jobs"]["macos-package"]["runs-on"], "macos-15")

    def test_no_workflow_names_the_runner_label(self):
        # A workflow that named it would bypass the variable and its fork rule.
        for path in sorted(WORKFLOWS.glob("*.y*ml")):
            with self.subTest(workflow=path.name):
                self.assertNotIn("hide-ci", path.read_text())


if __name__ == "__main__":
    unittest.main()
