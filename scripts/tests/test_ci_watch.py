"""`scripts/ci-watch.py` ends a CI watch at the first failed job, and at `verify`'s pass."""
import argparse
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("ci_watch", Path(__file__).parents[1] / "ci-watch.py")
watch = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watch)

RUN_URL = "https://github.com/o/r/actions/runs/7"


def job(name, status="in_progress", conclusion=None, completed_at=None, steps=(), id=None):
    return {"id": id or abs(hash(name)) % 10**9, "name": name, "status": status, "conclusion": conclusion,
            "completed_at": completed_at, "html_url": f"{RUN_URL}/job/{name}", "steps": list(steps)}


def failed(name, at, step="Test"):
    return job(name, "completed", "failure", at, steps=[{"name": "Set up job", "conclusion": "success"},
                                                         {"name": step, "conclusion": "failure"}])


def passed(name, at="2026-11-21T12:10:00Z"):
    return job(name, "completed", "success", at)


def run(status="in_progress", conclusion=None, sha="abc1234567890", name="verify", id=7, created="2026-11-21T12:00:00Z"):
    return {"id": id, "name": name, "status": status, "conclusion": conclusion, "head_sha": sha,
            "created_at": created, "html_url": RUN_URL}


class GitHub:
    """GitHub's REST answers: the run and its jobs as they stand, one state per read of the jobs."""

    def __init__(self, states, pulls=None, runs=None, pages=100):
        self.states, self.pulls, self.runs, self.pages = list(states), pulls or {}, runs, pages
        self.polls = 0
        self.asked = []

    def now(self):
        return self.states[min(self.polls, len(self.states) - 1)]

    def __call__(self, path):
        self.asked.append(path)
        if "/pulls/" in path:
            return self.pulls[int(path.rsplit("/", 1)[1])]
        if "/actions/runs?" in path:
            return {"workflow_runs": self.runs}
        if "/jobs?" in path:
            jobs = self.now()[1]
            page = int(path.rsplit("page=", 1)[1])
            answer = {"total_count": len(jobs), "jobs": jobs[(page - 1) * self.pages:page * self.pages]}
            if page * self.pages >= len(jobs):
                self.polls += 1
            return answer
        return self.now()[0]


def arguments(**given):
    values = {"pr": None, "branch": None, "run": 7, "sha": None, "interval": 30, "timeout": 120}
    values.update(given)
    return argparse.Namespace(**values)


def follow(github, **given):
    lines, sleeps = [], []

    def sleep(seconds):
        # A watch that never decides fails the test instead of hanging it.
        if len(sleeps) == 20:
            raise AssertionError(f"the watch did not decide: {lines}")
        sleeps.append(seconds)

    status = watch.watch(github, arguments(**given), sleep=sleep, clock=lambda: 0, out=lines.append)
    return status, lines, len(sleeps)


class FirstFailure(unittest.TestCase):
    def test_ends_at_the_first_failed_job_while_the_rest_still_run(self):
        github = GitHub([
            (run(), [job("plan", "completed", "success"), job("rust workspace / tests 1/3")]),
            (run(), [job("rust workspace / tests 1/3"), failed("web e2e / web e2e linux 5/9", "2026-11-21T12:12:00Z", step="End-to-end"),
                     job("verify", "queued")]),
            (run("completed", "failure"), [failed("verify", "2026-11-21T12:20:00Z")]),
        ])
        status, lines, sleeps = follow(github)
        self.assertEqual(status, 1)
        self.assertEqual(sleeps, 1, "the watch read the run twice and stopped at the failure")
        self.assertEqual(lines[1], "failed: web e2e / web e2e linux 5/9 (step `End-to-end`)")
        failed_id = github.states[1][1][1]["id"]
        self.assertEqual(lines[2], f"log: gh run view 7 --job {failed_id} --log-failed")

    def test_names_the_failure_that_ended_first_and_the_others_after_it(self):
        jobs = [failed("user docs", "2026-11-21T12:20:00Z"), failed("rust workspace / tests 2/3", "2026-11-21T12:11:00Z")]
        status, lines, _ = follow(GitHub([(run(), jobs)]))
        self.assertEqual(status, 1)
        self.assertTrue(lines[1].startswith("failed: rust workspace / tests 2/3"))
        self.assertEqual(lines[-1], "also failed: user docs")

    def test_a_failed_step_in_a_job_that_goes_on_decides_nothing(self):
        # A `continue-on-error` step, such as the flaky report, can fail in a job that passes.
        reporting = job("rust workspace / tests 1/3", steps=[{"name": "Report flaky and failed tests", "conclusion": "failure"}])
        github = GitHub([(run(), [reporting]), (run("completed", "success"), [passed("rust workspace / tests 1/3"), passed("verify")])])
        status, lines, sleeps = follow(github)
        self.assertEqual((status, sleeps), (0, 1))
        self.assertEqual(lines[-1], f"passed: verify ({RUN_URL})")


class Passing(unittest.TestCase):
    def test_ends_when_verify_passes(self):
        github = GitHub([(run(), [passed("rust workspace / build tests"), job("verify", "queued")]),
                         (run(), [passed("rust workspace / build tests"), passed("verify")])])
        status, lines, sleeps = follow(github)
        self.assertEqual((status, sleeps), (0, 1))

    def test_a_run_without_verify_passes_when_it_succeeds(self):
        github = GitHub([(run("completed", "success", name="repeat"), [passed("repeat web specs")])])
        self.assertEqual(follow(github)[0], 0)


class Replaced(unittest.TestCase):
    def test_a_cancelled_run_is_replaced_not_failed(self):
        cancelled = job("web e2e / web e2e linux 1/9", "completed", "cancelled", "2026-11-21T12:05:00Z")
        github = GitHub([(run(), [cancelled]), (run("completed", "cancelled"), [cancelled])])
        status, lines, _ = follow(github)
        self.assertEqual(status, 2)
        self.assertTrue(lines[-1].startswith("cancelled: run 7"))

    def test_a_job_its_own_limit_cancelled_fails_the_watch_once_the_run_ends_failed(self):
        cancelled = job("rust workspace / build tests", "completed", "cancelled", "2026-11-21T12:26:00Z")
        github = GitHub([(run(), [cancelled]), (run("completed", "failure"), [cancelled, job("verify", "completed", "skipped")])])
        status, lines, sleeps = follow(github)
        self.assertEqual((status, sleeps), (1, 1))
        self.assertEqual(lines[1], "failed: rust workspace / build tests (cancelled)")


class WhichRun(unittest.TestCase):
    def test_a_pull_request_watches_the_newest_verify_run_on_its_head(self):
        runs = [run(id=5, created="2026-11-21T11:00:00Z"), run(id=6, sha="old"), run(id=8, name="contribution gate"),
                run(id=7, created="2026-11-21T12:00:00Z")]
        github = GitHub([(run(), [passed("verify")])], pulls={42: {"head": {"sha": "abc1234567890"}}}, runs=runs)
        status, lines, _ = follow(github, run=None, pr=42)
        self.assertEqual(status, 0)
        self.assertIn("head_sha=abc1234567890&event=pull_request", github.asked[1])
        self.assertEqual(lines[0], f"watching run 7 on abc1234567: {RUN_URL}")

    def test_waits_for_the_run_a_push_starts(self):
        github = GitHub([(run(), [passed("verify")])], pulls={42: {"head": {"sha": "abc1234567890"}}}, runs=[])
        lines, sleeps = [], []

        def appear(seconds):
            sleeps.append(seconds)
            github.runs = [run()]

        status = watch.watch(github, arguments(run=None, pr=42), sleep=appear, clock=lambda: 0, out=lines.append)
        self.assertEqual((status, len(sleeps)), (0, 1))

    def test_reads_every_page_of_a_runs_jobs(self):
        jobs = [passed(f"lane {index}") for index in range(3)] + [failed("lane 3", "2026-11-21T12:11:00Z")]
        github = GitHub([(run(), jobs)], pages=2)
        status, lines, _ = follow(github)
        self.assertEqual(status, 1)
        self.assertTrue(lines[1].startswith("failed: lane 3"))


class GivingUp(unittest.TestCase):
    def test_a_run_still_going_at_the_timeout_ends_the_watch_with_2(self):
        times = iter([0, 0, 61 * 60])
        lines = []
        status = watch.watch(GitHub([(run(), [job("verify", "queued")])]), arguments(timeout=60),
                             sleep=lambda _: None, clock=lambda: next(times), out=lines.append)
        self.assertEqual(status, 2)
        self.assertTrue(lines[-1].startswith("still running after 60 minutes"))

    def test_an_unanswered_api_is_one_line_and_2(self):
        def refuse(path):
            raise watch.Unanswered("`gh api x` failed: HTTP 502")

        self.assertEqual(watch.main(["--run", "7"], api=refuse), 2)


if __name__ == "__main__":
    unittest.main()
