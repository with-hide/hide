"""The Mac mini runner's hooks: which jobs it takes and what a job leaves.

The contract is docs/BUILD.md, The candidate's Mac build. The hooks run as
the runner's account before and after every job; these run them the same
way, with the variables the runner gives a hook.
"""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/candidate-runner"
CANDIDATE = {
    "GITHUB_REPOSITORY": "with-hide/hide",
    "GITHUB_EVENT_NAME": "workflow_dispatch",
    "GITHUB_REF": "refs/heads/main",
    "GITHUB_WORKFLOW_REF": "with-hide/hide/.github/workflows/release-candidate.yml@refs/heads/main",
}


def hook(name, env, cwd=None):
    # The shell and flags the runner starts a hook with.
    return subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", str(RUNNER / name)],
                          env={"PATH": "/usr/bin:/bin", **env}, cwd=cwd, capture_output=True, text=True, timeout=30)


class JobStarted(unittest.TestCase):
    def setUp(self):
        runs = ROOT / "agents/runs/candidate-runner-tests"
        runs.mkdir(parents=True, exist_ok=True)
        self.home = Path(tempfile.mkdtemp(dir=runs))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.home)])
        self.work = self.home / "actions-runner/_work"
        (self.work / "_actions/someone/action/v1").mkdir(parents=True)
        (self.work / "_actions/someone/action/v1/pre.js").write_text("downloaded before the hook\n")
        # The runner's worker as the hook sees it: its parent, named
        # .../bin/Runner.Worker spawnclient, which runs the steps after it.
        self.ran = self.home / "steps-ran"
        worker = self.home / "bin/Runner.Worker"
        worker.parent.mkdir()
        worker.write_text("#!/bin/bash\n"
                          f"bash --noprofile --norc -e -o pipefail {RUNNER / 'job-started.sh'}\n"
                          f"echo ran > {self.ran}\n")
        worker.chmod(0o755)
        self.worker = worker

    def run_worker(self, env):
        return subprocess.run([str(self.worker), "spawnclient", "1", "2"],
                              env={"PATH": "/usr/bin:/bin", "HOME": str(self.home), **env},
                              capture_output=True, text=True, timeout=30)

    def test_takes_the_candidate_dispatched_on_main(self):
        result = hook("job-started.sh", CANDIDATE)
        self.assertEqual(result.returncode, 0, result.stdout)
        result = self.run_worker(CANDIDATE)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(self.ran.exists())

    def test_a_refused_jobs_worker_is_killed_before_any_step_and_leaves_nothing(self):
        result = self.run_worker({**CANDIDATE, "GITHUB_EVENT_NAME": "push",
                                  "GITHUB_REF": "refs/heads/feature",
                                  "GITHUB_WORKFLOW_REF": "with-hide/hide/.github/workflows/x.yml@refs/heads/feature"})
        self.assertEqual(result.returncode, -9)
        self.assertIn("::error::this runner builds only", result.stdout)
        self.assertFalse(self.ran.exists())
        self.assertEqual(list(self.work.iterdir()), [])

    def test_where_the_parent_is_not_the_worker_the_whole_runner_stops_until_restored(self):
        # As if an update put another process between the worker and its
        # hooks: the hook's parent is a wrapper, not the worker.
        runner = self.home / "actions-runner"
        (runner / "bin").mkdir()
        (runner / ".runner").write_text("{}\n")
        listener = runner / "bin/Runner.Listener"
        listener.write_text("#!/bin/bash\nwhile :; do sleep 1; done\n")
        listener.chmod(0o755)
        wrapper = self.home / "wrapper"
        wrapper.write_text(f"#!/bin/bash\nbash --noprofile --norc -e -o pipefail {RUNNER / 'job-started.sh'}\necho wrapped\n")
        wrapper.chmod(0o755)
        worker = runner / "bin/Runner.Worker"
        worker.write_text(f"#!/bin/bash\n{wrapper}\necho ran > {self.ran}\n")
        worker.chmod(0o755)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(self.home), "GITHUB_EVENT_NAME": "pull_request"}
        running = subprocess.Popen([str(listener), "run", "--startuptype", "service"], env=env)
        self.addCleanup(running.kill)
        result = subprocess.run([str(worker), "spawnclient", "1", "2"], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, -9)
        self.assertEqual(running.wait(timeout=10), -9)
        self.assertIn("the runner stops taking jobs", result.stdout)
        self.assertFalse(self.ran.exists())
        self.assertEqual(sorted(path.name for path in runner.iterdir() if path.name.startswith(".runner")), [".runner.stopped"])

    def test_refuses_every_other_job_before_its_steps(self):
        workflow = "with-hide/hide/.github/workflows/{}@{}"
        for patch in [
            {"GITHUB_EVENT_NAME": "pull_request", "GITHUB_REF": "refs/pull/1/merge",
             "GITHUB_WORKFLOW_REF": workflow.format("release-candidate.yml", "refs/pull/1/merge")},
            {"GITHUB_EVENT_NAME": "pull_request_target"},
            {"GITHUB_EVENT_NAME": "push"},
            {"GITHUB_REF": "refs/heads/feature",
             "GITHUB_WORKFLOW_REF": workflow.format("release-candidate.yml", "refs/heads/feature")},
            {"GITHUB_REF": "refs/tags/main"},
            {"GITHUB_WORKFLOW_REF": workflow.format("pr.yml", "refs/heads/main")},
            {"GITHUB_WORKFLOW_REF": workflow.format("release-candidate.yml", "refs/heads/feature")},
            {"GITHUB_REPOSITORY": "fork/hide",
             "GITHUB_WORKFLOW_REF": "fork/hide/.github/workflows/release-candidate.yml@refs/heads/main"},
            {"GITHUB_REPOSITORY": ""},
        ]:
            with self.subTest(patch=patch):
                result = hook("job-started.sh", {**CANDIDATE, **patch, "HOME": str(self.home)})
                self.assertEqual(result.returncode, 1)
                self.assertIn("::error::this runner builds only", result.stdout)
        result = hook("job-started.sh", {"HOME": str(self.home)})
        self.assertEqual(result.returncode, 1)
        self.assertIn("the runner stops taking jobs", result.stdout)


class JobCompleted(unittest.TestCase):
    def setUp(self):
        runs = ROOT / "agents/runs/candidate-runner-tests"
        runs.mkdir(parents=True, exist_ok=True)
        self.home = Path(tempfile.mkdtemp(dir=runs))
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.home)])
        self.work = self.home / "actions-runner/_work"
        self.workspace = self.work / "hide/hide"
        for name in ["_temp/runner-file", "_actions/actions/checkout/v7/action.yml", "hide/hide/target/release/hided",
                     "_cache/cargo/registry/crate", "_tool/setup-pnpm/node_modules/.bin/store/package", ".hidden"]:
            (self.work / name).parent.mkdir(parents=True, exist_ok=True)
            (self.work / name).write_text("left by a job\n")
        (self.workspace / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "2.0.0"\nprofile = "minimal"\n')
        (self.home / "kept").write_text("not the job's\n")
        # rustup as the account has it: two pins installed, one the source names.
        # Like rustup, it will not start in a folder that no longer exists.
        bin = self.home / "bin"
        bin.mkdir()
        (bin / "rustup").write_text(
            "#!/bin/bash\n"
            '/bin/pwd >/dev/null 2>&1 || { echo "error: Could not locate working directory." >&2; exit 1; }\n'
            'if [[ "$*" == "toolchain list" ]]; then\n'
            '  printf "2.0.0-aarch64-apple-darwin (active)\\n1.0.0-aarch64-apple-darwin\\n"\n'
            f'else echo "$*" >> "{self.home}/rustup.log"; fi\n')
        (bin / "rustup").chmod(0o755)
        self.env = {"HOME": str(self.home), "PATH": f"{bin}:/usr/bin:/bin", "GITHUB_WORKSPACE": str(self.workspace)}

    def test_removes_everything_a_job_left_and_toolchains_the_source_does_not_pin(self):
        result = hook("job-completed.sh", self.env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])
        self.assertTrue((self.home / "kept").exists())
        self.assertEqual((self.home / "rustup.log").read_text(), "toolchain uninstall 1.0.0-aarch64-apple-darwin\n")

    def test_runs_from_the_jobs_checkout_it_removes(self):
        # The runner starts the hook in the job's workspace, which the hook removes.
        result = hook("job-completed.sh", self.env, cwd=self.workspace)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])
        self.assertEqual((self.home / "rustup.log").read_text(), "toolchain uninstall 1.0.0-aarch64-apple-darwin\n")

    def test_without_a_checkout_it_still_empties_the_work_folder(self):
        result = hook("job-completed.sh", {**self.env, "GITHUB_WORKSPACE": ""})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])
        self.assertFalse((self.home / "rustup.log").exists())

    def test_a_failing_rustup_never_keeps_the_jobs_files(self):
        (self.home / "bin/rustup").write_text("#!/bin/bash\nexit 1\n")
        result = hook("job-completed.sh", self.env)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])

    def test_removes_odd_names_and_links_without_following_them(self):
        outside = self.home / "outside"
        (outside / "dir").mkdir(parents=True)
        (outside / "dir/kept").write_text("not the job's\n")
        (self.work / "..dots").write_text("left by a job\n")
        (self.work / "broken").symlink_to(self.home / "nothing-here")
        (self.work / "to-outside").symlink_to(outside / "dir")
        result = hook("job-completed.sh", self.env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sorted(path.name for path in self.work.iterdir()), ["_temp"])
        self.assertTrue((outside / "dir/kept").exists())

    def test_refuses_a_work_folder_reached_through_a_link(self):
        outside = self.home / "outside"
        outside.mkdir()
        (outside / "kept").write_text("not the job's\n")
        for linked in ["actions-runner/_work", "actions-runner"]:
            with self.subTest(linked=linked):
                home = Path(tempfile.mkdtemp(dir=self.home))
                target = home / "real"
                target.mkdir()
                (home / "actions-runner").mkdir()
                if linked == "actions-runner":
                    (home / "actions-runner").rmdir()
                    (target / "_work").symlink_to(outside)
                    (home / "actions-runner").symlink_to(target)
                else:
                    (home / "actions-runner/_work").symlink_to(outside)
                result = hook("job-completed.sh", {**self.env, "HOME": str(home)})
                self.assertEqual(result.returncode, 1)
                self.assertIn("nothing was removed", result.stdout)
                self.assertTrue((outside / "kept").exists())

    def test_fails_where_there_is_no_work_folder(self):
        result = hook("job-completed.sh", {**self.env, "HOME": str(self.home / "elsewhere")})
        self.assertNotEqual(result.returncode, 0)


class Setup(unittest.TestCase):
    def test_setup_parses(self):
        result = subprocess.run(["bash", "-n", str(RUNNER / "setup.sh")], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
