"""The release candidate's first gate: a red scheduled nightly or an open
nightly-failure or agent-canary issue stops it unless a reason is given.
GitHub's answers are recorded ones (trimmed from with-hide/hide on
2026-11-28); no test reaches the network."""

import contextlib
import importlib.util
import io
import re
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("check_nightly_green", ROOT / "scripts/check-nightly-green.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

RUNS = "repos/with-hide/hide/actions/workflows/nightly.yml/runs?branch=main&event=schedule&status=completed&per_page=1"
FAILURES = "repos/with-hide/hide/issues?labels=nightly-failure&state=open&per_page=100"
CANARY = "repos/with-hide/hide/issues?labels=agent-canary&state=open&per_page=100"


def run(conclusion):
    return {"workflow_runs": [{
        "id": 44834270233, "event": "schedule", "conclusion": conclusion,
        "html_url": "https://github.com/with-hide/hide/actions/runs/44834270233",
        "head_sha": "8c2f0e9a1b7d4e5f60718293a4b5c6d7e8f90123", "created_at": "2026-11-20T23:06:41Z"}]}


RECORD = {"number": 2552, "title": "Agent CLI support record", "html_url": "https://github.com/with-hide/hide/issues/2552"}
NIGHTLY_ISSUE = {"number": 2812, "title": "Nightly failure (agent journey / agent journey fake model (windows)): "
                 "The lane's verdict", "html_url": "https://github.com/with-hide/hide/issues/2812"}
CANARY_ISSUE = {"number": 2990, "title": "agent canary: claude-code 2.1.332 fails the agent journey",
                "html_url": "https://github.com/with-hide/hide/issues/2990"}


def answers(runs=None, failures=(), canary=(RECORD,)):
    recorded = {RUNS: runs if runs is not None else run("success"), FAILURES: list(failures), CANARY: list(canary)}

    def call(path):
        return recorded[path]
    return call


def check(call, *argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = gate.main(list(argv), call=call)
    return code, out.getvalue()


class Gate(unittest.TestCase):
    def test_a_green_nightly_and_only_the_record_open_lets_the_candidate_start(self):
        for runs in (run("success"), run("cancelled"), {"workflow_runs": []}):
            with self.subTest(runs=runs):
                self.assertEqual(check(answers(runs=runs))[0], 0)

    def test_a_failed_scheduled_nightly_stops_the_candidate(self):
        code, out = check(answers(runs=run("failure")))
        self.assertEqual(code, 1)
        self.assertIn("::error title=nightly gate::the newest scheduled nightly on main failed: "
                      "https://github.com/with-hide/hide/actions/runs/44834270233 (8c2f0e9a1b7d", out)

    def test_an_open_nightly_failure_or_agent_canary_issue_stops_it(self):
        for failures, canary, number in (([NIGHTLY_ISSUE], [RECORD], 2812), ([], [RECORD, CANARY_ISSUE], 2990)):
            with self.subTest(number=number):
                code, out = check(answers(failures=failures, canary=canary))
                self.assertEqual(code, 1)
                self.assertIn(f"issue #{number}:", out)
                self.assertNotIn("#2552", out)

    def test_a_pull_request_under_the_label_is_not_an_issue(self):
        pull = {**NIGHTLY_ISSUE, "pull_request": {"url": "x"}}
        self.assertEqual(check(answers(failures=[pull]))[0], 0)

    def test_a_reason_lets_an_urgent_release_through_and_is_printed(self):
        call = answers(runs=run("failure"), failures=[NIGHTLY_ISSUE])
        code, out = check(call, "--allow-red-nightly", "security fix for a leaked token")
        self.assertEqual(code, 0)
        self.assertIn("::warning title=nightly gate::starting with a red nightly: security fix for a leaked token", out)
        self.assertIn("issue #2812", out)
        # A blank reason is none.
        self.assertEqual(check(call, "--allow-red-nightly", "  ")[0], 1)

    def test_an_answer_it_cannot_read_stops_the_candidate(self):
        def broken(path):
            raise RuntimeError("gh api exited 1: HTTP 502")
        code, out = check(broken)
        self.assertEqual(code, 1)
        self.assertIn("could not read the nightly's state", out)
        too_many = [{**NIGHTLY_ISSUE, "number": number} for number in range(100)]
        self.assertEqual(check(answers(failures=too_many))[0], 1)


class Workflow(unittest.TestCase):
    def test_the_gate_is_the_release_candidates_first_job_and_every_job_waits_for_it(self):
        workflow = (ROOT / ".github/workflows/release-candidate.yml").read_text()
        jobs = workflow.split("\njobs:\n", 1)[1]
        self.assertEqual(re.findall(r"^  ([a-z-]+):$", jobs, re.M)[0], "nightly")
        body = jobs.split("\n  nightly:\n", 1)[1].split("\n\n", 1)[0]
        self.assertIn('python3 scripts/check-nightly-green.py --allow-red-nightly "$ALLOW_RED_NIGHTLY"', body)
        self.assertIn("ALLOW_RED_NIGHTLY: ${{ inputs.allow_red_nightly }}", body)
        self.assertIn("\n      allow_red_nightly:\n", workflow)
        validate = jobs.split("\n  validate:\n", 1)[1].split("\n    steps:", 1)[0]
        self.assertIn("needs: nightly", validate)


if __name__ == "__main__":
    unittest.main()
