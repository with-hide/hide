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


def hook(name, env):
    # The shell and flags the runner starts a hook with.
    return subprocess.run(["bash", "--noprofile", "--norc", "-e", "-o", "pipefail", str(RUNNER / name)],
                          env={"PATH": "/usr/bin:/bin", **env}, capture_output=True, text=True, timeout=30)


class JobStarted(unittest.TestCase):
    def test_takes_the_candidate_dispatched_on_main(self):
        result = hook("job-started.sh", CANDIDATE)
        self.assertEqual(result.returncode, 0, result.stdout)

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
                result = hook("job-started.sh", {**CANDIDATE, **patch})
                self.assertEqual(result.returncode, 1)
                self.assertIn("::error::this runner builds only", result.stdout)
        result = hook("job-started.sh", {})
        self.assertEqual(result.returncode, 1)


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
        bin = self.home / "bin"
        bin.mkdir()
        (bin / "rustup").write_text(
            "#!/bin/bash\n"
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

    def test_fails_where_there_is_no_work_folder(self):
        result = hook("job-completed.sh", {**self.env, "HOME": str(self.home / "elsewhere")})
        self.assertEqual(result.returncode, 1)
        self.assertIn("does not exist", result.stdout)


class Setup(unittest.TestCase):
    def test_setup_parses(self):
        result = subprocess.run(["bash", "-n", str(RUNNER / "setup.sh")], capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
