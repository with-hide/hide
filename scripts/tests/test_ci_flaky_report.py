"""What `scripts/ci-flaky-report.py` reads from a report and files as issues."""
from datetime import date
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

SCRIPT = Path(__file__).parents[1] / "ci-flaky-report.py"
SPEC = importlib.util.spec_from_file_location("ci_flaky_report", SCRIPT)
r = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(r)

TODAY = date(2026, 10, 5)
ENV = {
    "GITHUB_SERVER_URL": "https://github.com", "GITHUB_REPOSITORY": "o/r", "GITHUB_RUN_ID": "77",
    "GITHUB_RUN_ATTEMPT": "1", "GITHUB_REF": "refs/pull/9/merge", "GITHUB_SHA": "abcdef0123456789",
}


def spec(title, status, message="boom"):
    return {"title": title, "file": "a.spec.ts", "tests": [{"status": status, "results": [{"error": {"message": message}}]}]}


class Reading(unittest.TestCase):
    def test_only_a_flaky_playwright_test_is_listed_with_its_describe_path(self):
        report = {"suites": [{"title": "a.spec.ts", "specs": [spec("ok", "expected"), spec("bad", "unexpected"), spec("wobbles", "flaky")],
                              "suites": [{"title": "group", "specs": [spec("inner", "flaky")]}]}]}
        found = r.playwright_flaky(report, "web")
        self.assertEqual([(t["file"], t["name"]) for t in found], [("web/e2e/a.spec.ts", "wobbles"), ("web/e2e/a.spec.ts", "group > inner")])

    def test_only_a_flaky_nextest_case_is_listed(self):
        root = ET.fromstring(
            '<testsuites><testsuite name="p::t">'
            '<testcase name="steady" classname="p::t"/>'
            '<testcase name="wobbles" classname="p::t"><flakyFailure message="m">panicked at x</flakyFailure></testcase>'
            "</testsuite></testsuites>"
        )
        self.assertEqual(r.nextest_flaky(root), [{"file": None, "name": "p::t wobbles", "error": "panicked at x"}])

    def test_a_nextest_case_that_hung_once_is_listed_as_a_timeout(self):
        # The shape cargo-nextest 0.9.143 writes for a test ended by slow-timeout that passed its retry.
        root = ET.fromstring(
            '<testsuites><testsuite name="p">'
            '<testcase name="hangs" classname="p"><flakyFailure time="120.003" type="test timeout">'
            "\n<system-out>running 1 test\n</system-out><system-err></system-err></flakyFailure></testcase>"
            "</testsuite></testsuites>"
        )
        self.assertEqual(r.nextest_flaky(root), [{"file": None, "name": "p hangs", "error": "test timeout"}])


class Matching(unittest.TestCase):
    TEST = {"file": "web/e2e/tab-strip-fit.spec.ts", "name": "Agent tabs shrink in stages, the selected one keeping its title longest", "error": ""}

    def test_a_hand_written_issue_with_a_cut_title_tracks_the_test(self):
        body = '`web/e2e/tab-strip-fit.spec.ts:62`("Agent tabs shrink in stages...") flakes'
        self.assertTrue(r.tracks({"body": body}, self.TEST))

    def test_another_file_or_another_test_is_not_tracked(self):
        self.assertFalse(r.tracks({"body": '`web/e2e/other.spec.ts` "Agent tabs shrink in stages"'}, self.TEST))
        self.assertFalse(r.tracks({"body": "`web/e2e/tab-strip-fit.spec.ts` something else"}, self.TEST))

    def test_a_rust_test_is_tracked_by_its_whole_name(self):
        test = {"file": None, "name": "hide-platform::ipc a_burst", "error": ""}
        self.assertTrue(r.tracks({"body": "`hide-platform::ipc a_burst` flakes"}, test))
        self.assertFalse(r.tracks({"body": "`hide-platform::ipc another`"}, test))


class Filing(unittest.TestCase):
    def setUp(self):
        self.calls = []

    def call(self, existing=(), comments=()):
        def call(args, body=None):
            self.calls.append((args, body))
            path = args[0]
            if "/issues?labels=" in path:
                return list(existing)
            if path.endswith("/comments?per_page=100"):
                return list(comments)
            if path.endswith("/issues"):
                return {"number": 50, "html_url": "https://example/50", "body": body["body"]}
            return {}
        return call

    def test_a_new_flaky_test_is_filed_with_run_pull_request_and_a_week(self):
        test = {"file": "web/e2e/a.spec.ts", "name": "wobbles", "error": "\x1b[31mboom\x1b[0m"}
        r.report([test], "Linux", ENV, self.call(), TODAY)
        (args, body), = [c for c in self.calls if c[0][0].endswith("/issues")]
        self.assertEqual(body["labels"], ["quarantine", "bug"])
        for text in ("actions/runs/77", "pull request #9", "abcdef012345", "Expires: 2026-10-12", "boom", "Linux"):
            self.assertIn(text, body["body"])
        self.assertNotIn("\x1b", body["body"])

    def test_a_tracked_test_gets_a_comment_and_no_second_issue(self):
        test = {"file": "web/e2e/a.spec.ts", "name": "wobbles", "error": "boom"}
        issue = {"number": 5, "body": '`web/e2e/a.spec.ts` "wobbles"'}
        r.report([test], "Linux", ENV, self.call([issue]), TODAY)
        posts = [c for c in self.calls if c[1] is not None]
        self.assertEqual([c[0][0] for c in posts], ["repos/o/r/issues/5/comments"])

    def test_the_same_run_is_not_added_twice(self):
        test = {"file": "web/e2e/a.spec.ts", "name": "wobbles", "error": "boom"}
        issue = {"number": 5, "body": '`web/e2e/a.spec.ts` "wobbles"'}
        r.report([test], "Linux", ENV, self.call([issue], [{"body": "<!-- flaky-run:77:1 -->"}]), TODAY)
        self.assertEqual([c for c in self.calls if c[1] is not None], [])

    def test_a_flake_past_its_expiry_says_so(self):
        test = {"file": "web/e2e/a.spec.ts", "name": "wobbles", "error": "boom"}
        issue = {"number": 5, "body": '`web/e2e/a.spec.ts` "wobbles"\n- Expires: 2026-10-01\n'}
        r.report([test], "Linux", ENV, self.call([issue]), TODAY)
        self.assertIn("past its expiry", [c for c in self.calls if c[1] is not None][0][1]["body"])

    def test_a_run_files_at_most_a_cap(self):
        tests = [{"file": None, "name": f"t{n}", "error": ""} for n in range(r.MAX_PER_RUN + 5)]
        r.report(tests, "Linux", ENV, self.call(), TODAY)
        self.assertEqual(len([c for c in self.calls if c[0][0].endswith("/issues")]), r.MAX_PER_RUN)

    def test_a_failed_filing_leaves_the_tests_in_the_job_summary(self):
        import os, tempfile
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        report = Path(folder.name) / "junit.xml"
        report.write_text('<testsuites><testsuite name="p"><testcase name="wobbles" classname="p"><flakyFailure message="m">boom</flakyFailure></testcase></testsuite></testsuites>')
        summary = Path(folder.name) / "summary.md"
        saved = dict(os.environ)
        os.environ.update({"GITHUB_STEP_SUMMARY": str(summary), "PATH": "/nonexistent"})
        self.addCleanup(lambda: (os.environ.clear(), os.environ.update(saved)))
        os.environ.update(ENV)
        self.assertEqual(r.main(["--suite", "rust", "--junit", str(report), "--system", "Linux"]), 1)
        self.assertIn("p wobbles", summary.read_text())
        self.assertIn("not filed", summary.read_text())

    def test_several_junit_reports_are_read_together(self):
        import tempfile
        folder = Path(tempfile.mkdtemp())
        for n in (1, 2):
            (folder / f"junit.{n}.xml").write_text(f'<testsuites><testsuite name="p"><testcase name="t{n}" classname="p"><flakyFailure message="m">x</flakyFailure></testcase></testsuite></testsuites>')
        seen = []
        saved, r.report = r.report, lambda tests, system, env, **kw: seen.extend(t["name"] for t in tests)
        self.addCleanup(setattr, r, "report", saved)
        self.assertEqual(r.main(["--suite", "rust", "--junit", str(folder / "junit.1.xml"), str(folder / "junit.2.xml")]), 0)
        self.assertEqual(seen, ["p t1", "p t2"])

    def test_a_missing_report_is_not_an_error(self):
        self.assertEqual(r.main(["--suite", "web", "--playwright", "/nonexistent/report.json"]), 0)

    def test_only_a_run_with_a_flaky_test_says_so_in_its_step_output(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        output = Path(folder.name) / "output"
        saved = dict(os.environ)
        os.environ["GITHUB_OUTPUT"] = str(output)
        self.addCleanup(lambda: (os.environ.clear(), os.environ.update(saved)))
        saved_report, r.report = r.report, lambda *args, **kwargs: None
        self.addCleanup(setattr, r, "report", saved_report)
        for name, status in (("steady", "expected"), ("failed", "unexpected"), ("flaky", "flaky")):
            report = Path(folder.name) / f"{name}.json"
            report.write_text(json.dumps({"suites": [{"title": "a.spec.ts", "specs": [spec(name, status)]}]}))
            self.assertEqual(r.main(["--suite", "web", "--playwright", str(report)]), 0)
            self.assertEqual(output.read_text() if output.exists() else "", "flaky=true\n" if status == "flaky" else "")


class Workflows(unittest.TestCase):
    def test_every_e2e_log_upload_keeps_a_flaky_run_as_it_keeps_a_failed_one(self):
        # A flaky run passed, but its first attempt failed, and the daemon,
        # Herdr and input logs of that attempt are the only evidence its issue
        # gets. Each job that reports flaky Playwright tests and keeps those
        # logs reads the output of its own report.
        uploads = 0
        for workflow in sorted((SCRIPT.parents[1] / ".github" / "workflows").glob("*.yml")):
            for job in re.split(r"\n  (?=[\w-]+:\n)", workflow.read_text()):
                steps = re.split(r"\n      - ", job)
                if not any(re.search(r"ci-flaky-report\.py --suite (web|desktop)", step) for step in steps):
                    continue
                reports = [step for step in steps if "ci-flaky-report.py" in step and "\n        id: flaky\n" in step]
                for step in steps:
                    if "upload-artifact" not in step or "hide-e2e" not in step:
                        continue
                    # The window a passing run drew is kept on success, not as a failure's logs.
                    if re.search(r"path: \$\{\{ runner\.temp \}\}/hide-e2e/\*\.png\n", step):
                        continue
                    uploads += 1
                    self.assertIn("if: ${{ failure() || steps.flaky.outputs.flaky == 'true' }}", step, workflow.name)
                    self.assertEqual(len(reports), 1, f"{workflow.name}: the job's report step has id flaky")
        self.assertEqual(uploads, 4)

    def test_a_report_step_runs_the_script_once_so_its_annotations_fit_one_step(self):
        # GitHub keeps 10 error annotations per step; two reports in one step
        # would share them and lose failed tests past the tenth.
        steps = 0
        for workflow in sorted((SCRIPT.parents[1] / ".github" / "workflows").glob("*.yml")):
            for step in re.split(r"\n      - ", workflow.read_text()):
                if re.search(r"ci-flaky-report\.py --(suite|measurement)", step):
                    steps += 1
                    self.assertEqual(step.count("ci-flaky-report.py"), 1, f"{workflow.name}: {step.splitlines()[0]}")
        self.assertEqual(steps, 10)


class Annotating(unittest.TestCase):
    def test_a_test_that_failed_its_retry_is_annotated_with_its_file_line_and_name(self):
        report = {"suites": [{"title": "s.spec.ts", "specs": [
            {"title": "picks 50%", "file": "s.spec.ts", "line": 29, "tests": [{"status": "unexpected", "results": [
                {"error": {"message": "first attempt"}}, {"error": {"message": "\x1b[31mlast attempt\x1b[0m"}}]}]},
            spec("wobbles", "flaky"), spec("steady", "expected"),
        ]}]}
        failed = r.playwright_failed(report, "desktop")
        self.assertEqual([(t["file"], t["line"], t["name"], t["error"]) for t in failed],
                         [("desktop/e2e/s.spec.ts", 29, "picks 50%", "\x1b[31mlast attempt\x1b[0m")])
        lines = []
        r.annotate(failed, lines.append)
        self.assertEqual(lines, ["::error file=desktop/e2e/s.spec.ts,line=29,title=Failed test::picks 50%25%0Alast attempt"])

    def test_a_nextest_failure_is_annotated_on_its_crate(self):
        root = ET.fromstring(
            '<testsuites><testsuite name="hided::real_herdr">'
            '<testcase name="echoes" classname="hided::real_herdr"><failure type="test failure"/>'
            "<system-err>thread panicked at hided/tests/it/real_herdr.rs:467:5</system-err></testcase>"
            '<testcase name="wobbles" classname="hided::real_herdr"><flakyFailure message="m">x</flakyFailure></testcase>'
            "</testsuite></testsuites>"
        )
        failed = r.nextest_failed(root)
        self.assertEqual([(t["file"], t["name"]) for t in failed], [("hided", "hided::real_herdr echoes")])
        self.assertIn("panicked at", failed[0]["error"])

    def test_a_failed_measurement_is_annotated_with_its_check_and_no_file(self):
        # The nightly report reads the title and the first line; a measurement
        # has no test file to name.
        with tempfile.TemporaryDirectory() as tmp:
            record = Path(tmp) / "nightly-summary.json"
            record.write_text(json.dumps({"failures": [
                {"name": "key echo under agent churn: invalid measurement", "error": "trial did not complete successfully"}]}))
            lines = []
            saved = r.annotate
            self.addCleanup(setattr, r, "annotate", saved)
            r.annotate = lambda failed, title=r.FAILED_TITLE: saved(failed, lines.append, title)
            self.assertEqual(r.main(["--measurement", str(record)]), 0)
            self.assertEqual(lines, ["::error title=Failed measurement::key echo under agent churn: invalid measurement"
                                     "%0Atrial did not complete successfully"])
            lines.clear()
            record.write_text(json.dumps({"failures": []}))
            self.assertEqual(r.main(["--measurement", str(record)]), 0)
            self.assertEqual(lines, [])
            # A run that wrote no record is reported by its failed step instead.
            self.assertEqual(r.main(["--measurement", str(Path(tmp) / "missing.json")]), 0)
            self.assertEqual(lines, [])

    def test_more_failed_tests_than_a_step_has_room_for_end_with_their_count(self):
        # GitHub keeps 10 error annotations per step; one is left for the runner's own.
        failed = [{"file": None, "line": None, "name": f"t{n}", "error": ""} for n in range(r.ANNOTATED_TESTS + 3)]
        lines = []
        r.annotate(failed, lines.append)
        self.assertEqual(len(lines), r.ANNOTATED_TESTS + 1)
        self.assertLessEqual(len(lines), 9)
        self.assertEqual(lines[-1], "::error title=Failed tests not annotated::3")

    def test_an_excerpt_names_no_home_folder_and_no_token(self):
        text = r.clean("at /Users/alice/x, /home/alice/z and C:\\Users\\alice\\y with ghp_" + "a" * 30)
        self.assertNotIn("alice", text)
        self.assertNotIn("ghp_", text)


RUN = 100
HEAD = "c" * 40
GREEN = "a" * 40


def job(id, name, conclusion, steps=(), labels=("macos-15",)):
    return {
        "id": id, "name": name, "status": "completed", "conclusion": conclusion, "labels": list(labels),
        "html_url": f"https://github.com/o/r/actions/runs/{RUN}/job/{id}",
        "check_run_url": f"https://api.github.com/repos/o/r/check-runs/{id}",
        "steps": [{"name": step, "conclusion": result} for step, result in steps],
    }


def failed_test(path, line, name, error="boom"):
    return {"title": "Failed test", "path": path, "start_line": line, "message": f"{name}\n{error}"}


# A failed job's log as the runner writes it (run 38958746826, macOS agy job):
# every step echoes its command in color, and a test may print any byte (an
# OSC title, a NUL, a byte that is not UTF-8).
JOB_LOG = (
    b"2026-10-16T20:25:19.3493170Z ##[group]Run bash scripts/verify-cargo.sh nextest --profile ci $TEST_TARGETS\n"
    b"2026-10-16T20:25:19.3494430Z \x1b[36;1mbash scripts/verify-cargo.sh nextest --profile ci $TEST_TARGETS\x1b[0m\n"
    b"2026-10-16T20:25:19.3655790Z shell: /bin/bash --noprofile --norc -e -o pipefail {0}\n"
    b"2026-10-16T20:25:19.3656820Z ##[endgroup]\n"
    b"2026-10-16T20:29:27.0950000Z \x1b]0;unterminated title\n"
    b"2026-10-16T20:29:27.0960000Z Compiling a\rCompiling b\x7f\xc2\x9b31m\x1bPq#0;2\x1b\\\n"
    b"2026-10-16T20:29:28.0947990Z      Summary [ 248.007s] 2 tests run: 1 passed, 1 failed, 105 skipped\n"
    b"2026-10-16T20:29:28.0948760Z   TRY 2 FAIL [ 105.471s] (1/2) hide-agent-hooks::it agy_binary::a_signed_in_agy_turn_fires_hide_s_hooks\n"
    b"2026-10-16T20:29:28.1043050Z \x1b]0;agy\x07error: test run failed \xff\x00\r\n"
    b"2026-10-16T20:29:28.1255500Z ##[error]Process completed with exit code 100.\n"
    b"2026-10-16T20:29:28.1471320Z Post job cleanup.\n"
)
CONTROL_CHARACTER = re.compile(r"[\x00-\x08\x0b-\x1f\x7f-\x9f]")


class FakeGitHub:
    """GitHub's REST answers for one nightly run; every write is recorded, none is sent."""

    def __init__(self, jobs, annotations=None, logs=None, issues=(), comments=None, event="schedule", branch="main",
                 earlier=None, commits=(), pulls=(), files=None):
        self.run = {"id": RUN, "run_attempt": 1, "event": event, "head_branch": branch, "head_sha": HEAD,
                    "html_url": f"https://github.com/o/r/actions/runs/{RUN}", "created_at": "2026-10-07T00:00:00Z", "workflow_id": 9}
        self.jobs = {RUN: jobs}
        self.earlier = earlier or []
        for run, run_jobs in (earlier or {}).items() if isinstance(earlier, dict) else []:
            self.jobs[run] = run_jobs
        self.annotations = annotations or {}
        self.logs = logs or {}
        self.issues = list(issues)
        self.comments = comments or {}
        self.commits, self.pulls, self.files = list(commits), list(pulls), files or {}
        self.total_commits = None
        self.writes = []

    def add_earlier(self, run_id, sha, created_at, jobs, event="schedule"):
        self.earlier.append({"id": run_id, "run_attempt": 1, "event": event, "head_sha": sha, "created_at": created_at,
                             "html_url": f"https://github.com/o/r/actions/runs/{run_id}"})
        self.jobs[run_id] = jobs

    def __call__(self, args, body=None, raw=False):
        path = args[0]
        if body is not None:
            self.writes.append((path, args[1:], body))
            if path == "repos/o/r/issues":
                return {"number": 900 + len(self.writes), "html_url": f"https://example/{900 + len(self.writes)}"}
            return {}
        first_page = not re.search(r"[?&]page=(?!1(&|$))", path)
        if raw:
            return self.logs[int(re.search(r"jobs/(\d+)/logs", path).group(1))]
        if path == f"repos/o/r/actions/runs/{RUN}":
            return self.run
        if match := re.match(r"repos/o/r/actions/runs/(\d+)/attempts/\d+/jobs", path):
            return {"jobs": self.jobs[int(match.group(1))] if first_page else []}
        if match := re.match(r"repos/o/r/check-runs/(\d+)/annotations", path):
            return self.annotations.get(int(match.group(1)), []) if first_page else []
        if path.startswith("repos/o/r/issues?labels=nightly-failure"):
            return self.issues if first_page else []
        if match := re.match(r"repos/o/r/issues/(\d+)/comments", path):
            return self.comments.get(int(match.group(1)), []) if first_page else []
        if path.startswith("repos/o/r/actions/workflows/9/runs"):
            return {"workflow_runs": sorted(self.earlier, key=lambda run: run["created_at"], reverse=True)}
        if path.startswith(f"repos/o/r/compare/{GREEN}...{HEAD}"):
            # A long range comes a page of 100 at a time.
            page = int(re.search(r"[?&]page=(\d+)", path).group(1))
            return {"total_commits": self.total_commits or len(self.commits),
                    "commits": [{"sha": sha} for sha in self.commits[(page - 1) * 100:page * 100]]}
        if path.startswith("repos/o/r/pulls?"):
            return self.pulls if first_page else []
        if match := re.match(r"repos/o/r/pulls/(\d+)/files", path):
            return [{"filename": name} for name in self.files.get(int(match.group(1)), [])] if first_page else []
        raise AssertionError(f"unexpected call {args}")

    def opened(self):
        return [body for path, _, body in self.writes if path == "repos/o/r/issues"]

    def commented(self):
        return [int(re.search(r"issues/(\d+)/comments", path).group(1)) for path, _, _ in self.writes if path.endswith("/comments")]

    def closed(self):
        return [int(re.search(r"issues/(\d+)$", path).group(1)) for path, _, body in self.writes if body.get("state") == "closed"]


PACKAGE = "package / package (macos)"


def nightly(github, lane_input=""):
    report_run = r.Nightly("o/r", RUN, None, lane_input, github, SCRIPT.parents[1], TODAY)
    report_run.apply(report_run.plan())
    return github


class NightlyFailures(unittest.TestCase):
    def package_failed(self, **extra):
        github = FakeGitHub(
            [job(1, PACKAGE, "failure", [("Packaged app launch", "failure")]), job(2, "desktop e2e (macos full)", "success")],
            annotations={1: [failed_test("desktop/e2e/s.spec.ts", 29, "server picker", "/Users/alice/x boom")]},
            commits=["m1", "m2", "m3"],
            pulls=[{"number": 668, "merge_commit_sha": "m1", "updated_at": "2026-10-06T10:00:00Z"},
                   {"number": 670, "merge_commit_sha": "m2", "updated_at": "2026-10-06T11:00:00Z"},
                   {"number": 600, "merge_commit_sha": "old", "updated_at": "2026-10-06T12:00:00Z"}],
            files={668: ["desktop/e2e/s.spec.ts"], 670: ["hide-kit/src/lib.rs"], 600: ["desktop/e2e/s.spec.ts"]},
            **extra,
        )
        github.add_earlier(90, GREEN, "2026-10-05T18:00:00Z", [job(11, PACKAGE, "success")])
        return github

    def test_a_first_failure_opens_one_issue_with_its_candidates_and_a_two_day_expiry(self):
        github = nightly(self.package_failed())
        (issue,) = github.opened()
        self.assertEqual(issue["labels"], ["nightly-failure"])
        self.assertEqual(issue["title"], "Nightly failure (package / package (macos)): server picker")
        body = issue["body"]
        for text in ("<!-- nightly-failure:package / package (macos):desktop/e2e/s.spec.ts > server picker -->",
                     "`desktop/e2e/s.spec.ts:29` \"server picker\"", "actions/runs/100", HEAD[:12], GREEN[:12],
                     "#668", "Expires: 2026-10-07", "revert the candidate"):
            self.assertIn(text, body)
        self.assertNotIn("#670", body, "it touched another file")
        self.assertNotIn("#600", body, "it merged outside the range")
        self.assertNotIn("alice", body)

    def test_a_failed_measurement_names_every_pull_request_merged_since_the_lane_passed(self):
        lane = "performance (macOS) / topology operator"
        check = "key echo under agent churn: invalid measurement"
        github = FakeGitHub(
            [job(1, lane, "failure", [("Judge the run", "failure")])],
            annotations={1: [{"title": "Failed measurement", "path": ".github", "start_line": 1,
                              "message": f"{check}\ntrial did not complete successfully"}]},
            commits=["m1", "m2"],
            pulls=[{"number": 668, "merge_commit_sha": "m1", "updated_at": "2026-10-06T10:00:00Z"},
                   {"number": 670, "merge_commit_sha": "m2", "updated_at": "2026-10-06T11:00:00Z"},
                   {"number": 600, "merge_commit_sha": "old", "updated_at": "2026-10-06T12:00:00Z"}],
            files={668: ["hided/src/server.rs"], 670: ["docs/BUILD.md"]},
        )
        github.add_earlier(90, GREEN, "2026-10-05T18:00:00Z", [job(11, lane, "success")])
        (issue,) = nightly(github).opened()
        self.assertEqual(issue["title"], f"Nightly failure ({lane}): {check}")
        body = issue["body"]
        for text in (f"<!-- nightly-failure:{lane}:measurement {check} -->", f"- Measurement: \"{check}\"",
                     "touching any path, since a measurement moves with any change): #668, #670",
                     "trial did not complete successfully"):
            self.assertIn(text, body)
        self.assertNotIn("#600", body, "it merged outside the range")
        # The same check failing again is a comment on that issue.
        again = nightly(FakeGitHub(
            [job(1, lane, "failure")], annotations=github.annotations,
            issues=[{"number": 5, "body": f"<!-- nightly-failure:{lane}:measurement {check} -->"}]))
        self.assertEqual((again.opened(), again.commented()), ([], [5]))

    def test_the_same_test_failing_in_two_lanes_is_two_issues_their_titles_tell_apart(self):
        name = "a test whose name runs long " * 10
        github = nightly(FakeGitHub(
            [job(1, "web e2e (macOS @platform) / web e2e nightly-macos 1/1", "failure"),
             job(2, "web e2e (Windows @platform) / web e2e nightly-windows 1/1", "failure")],
            annotations={1: [failed_test("web/e2e/a.spec.ts", 3, name)], 2: [failed_test("web/e2e/a.spec.ts", 3, name)]}))
        titles = [issue["title"] for issue in github.opened()]
        self.assertEqual(len(titles), 2)
        self.assertTrue(titles[0].startswith("Nightly failure (web e2e (macOS @platform) / web e2e nightly-macos): a test"))
        self.assertTrue(titles[1].startswith("Nightly failure (web e2e (Windows @platform) / web e2e nightly-windows): a test"))
        self.assertEqual([len(title) for title in titles], [200, 200])

    def test_the_same_failure_again_is_one_comment_per_attempt(self):
        marker = "<!-- nightly-failure:package / package (macos):desktop/e2e/s.spec.ts > server picker -->"
        github = nightly(self.package_failed(issues=[{"number": 5, "body": marker}]))
        self.assertEqual((github.opened(), github.commented()), ([], [5]))
        seen = {5: [{"body": "<!-- nightly-run:100:1 -->"}]}
        again = nightly(self.package_failed(issues=[{"number": 5, "body": marker}], comments=seen))
        self.assertEqual(again.writes, [])
        rerun = self.package_failed(issues=[{"number": 5, "body": marker}], comments=seen)
        attempt = r.Nightly("o/r", RUN, 2, "", rerun, SCRIPT.parents[1], TODAY)
        attempt.apply(attempt.plan())
        self.assertEqual(rerun.commented(), [5])
        self.assertIn("<!-- nightly-run:100:2 -->", rerun.writes[0][2]["body"])

    def test_a_lane_that_passes_closes_its_issues_and_a_skipped_or_cancelled_one_does_not(self):
        issues = [{"number": n, "body": f"<!-- nightly-failure:{lane}:step x -->"}
                  for n, lane in ((1, "desktop e2e (macos full)"), (2, "desktop e2e (linux @platform)"), (3, "verify / os contract"))]
        github = nightly(FakeGitHub(
            [job(1, "desktop e2e (macos full)", "success"), job(2, "desktop e2e (linux @platform)", "skipped"),
             job(3, "verify / os contract", "cancelled")], issues=issues))
        self.assertEqual(github.closed(), [1])
        self.assertIn("Passed at cccccccccccc", github.writes[0][2]["body"])

    def test_only_a_hand_run_of_every_lane_closes(self):
        issues = [{"number": 1, "body": "<!-- nightly-failure:desktop e2e (macos full):step x -->"}]
        jobs = [job(1, "desktop e2e (macos full)", "success")]
        self.assertEqual(nightly(FakeGitHub(jobs, issues=issues, event="workflow_dispatch"), "macos").closed(), [])
        self.assertEqual(nightly(FakeGitHub(jobs, issues=issues, event="workflow_dispatch"), "all").closed(), [1])

    def test_a_failed_job_with_no_failed_test_annotation_is_a_step_failure(self):
        github = FakeGitHub(
            [job(1, "desktop e2e (linux @platform)", "failure", [("Set up job", "success"), ("Desktop suite", "failure")]),
             job(2, "verify / verify", "failure", [("Every planned lane succeeded", "failure")])],
            logs={1: "2026-10-07T01:02:03.4Z line a\n2026-10-07T01:02:04.5Z ##[error]Process completed with exit code 1.\nlater"},
        )
        nightly(github)
        (issue,) = github.opened()
        self.assertIn("<!-- nightly-failure:desktop e2e (linux @platform):step Desktop suite -->", issue["body"])
        self.assertIn("line a\n##[error]Process completed", issue["body"])
        self.assertNotIn("later", issue["body"])
        self.assertEqual(issue["labels"], ["nightly-failure"])

    def test_an_excerpt_carries_no_terminal_control_from_a_log_or_an_annotation(self):
        # gh hands a JSON body over with its controls in caret notation, but a lane
        # writes its annotations from its tool's own report, which carries them raw.
        github = FakeGitHub(
            [job(1, "agy contract / agy signed in (macos)", "failure", [("The real agy runs one turn", "failure")]),
             job(2, PACKAGE, "failure")],
            logs={1: JOB_LOG.decode("utf-8", "replace")},
            annotations={2: [failed_test("desktop/e2e/s.spec.ts", 29, "server picker", "\x1b[31mExpected\x1b[0m \x1b]8;;x\x07a\x1b]8;;\x07\x00")]},
        )
        opened = nightly(github).opened()
        self.assertEqual(len(opened), 2)
        for issue in opened:
            self.assertIsNone(CONTROL_CHARACTER.search(issue["body"]), issue["body"])
        self.assertIn("error: test run failed \ufffd\n##[error]Process completed with exit code 100.", opened[0]["body"])
        self.assertIn("Expected a", opened[1]["body"])

    def test_a_failure_while_preparing_the_runner_is_infra(self):
        cases = (("Fetch the pinned Herdr runtime", True), ("Run actions/checkout@v7", True), ("Install dependencies", True),
                 ("Build the desktop app's daemon and kit binaries", False), ("Desktop suite", False))
        for step, infra in cases:
            with self.subTest(step=step):
                github = nightly(FakeGitHub([job(1, "desktop e2e (macos full)", "failure", [(step, "failure")])], logs={1: "x"}))
                self.assertEqual("infra" in github.opened()[0]["labels"], infra)
        lost = nightly(FakeGitHub([job(1, "desktop e2e (macos full)", "failure", [("Desktop suite", "success")])], logs={1: "x"}))
        self.assertIn("step (runner)", lost.opened()[0]["body"])
        self.assertIn("infra", lost.opened()[0]["labels"])

    def test_a_run_opens_at_most_ten_and_lists_the_rest_with_what_no_lane_annotated(self):
        tests = [failed_test("web/e2e/a.spec.ts", n, f"t{n}") for n in range(13)]
        tests.append({"title": "Failed tests not annotated", "path": ".github", "start_line": 1, "message": "4"})
        github = nightly(FakeGitHub([job(1, "verify / web e2e / web e2e linux 2/4", "failure")], annotations={1: tests}))
        opened = github.opened()
        self.assertEqual(len(opened), r.NEW_ISSUES_PER_RUN + 1)
        self.assertEqual(opened[-1]["title"], "nightly: 7 more failures in run 100")
        self.assertIn("4 failed test(s) a lane had no annotation room for", opened[-1]["body"])

    def test_a_run_off_main_writes_nothing(self):
        github = FakeGitHub([job(1, PACKAGE, "failure")], branch="topic")
        self.assertEqual(r.Nightly("o/r", RUN, None, "", github, SCRIPT.parents[1], TODAY).plan(), [])
        args = r.argparse.Namespace(repo="o/r", run=None, attempt=None, lane_input="", dry_run=False)
        calls = []
        self.assertEqual(r.nightly(args, {**ENV, "GITHUB_REF": "refs/heads/topic"}, lambda *a, **k: calls.append(a)), 0)
        self.assertEqual(calls, [])

    def test_a_dry_run_reads_and_writes_nothing(self):
        github = self.package_failed()
        args = r.argparse.Namespace(repo="o/r", run=str(RUN), attempt=None, lane_input="", dry_run=True)
        saved, r.date = r.date, type("D", (), {"today": staticmethod(lambda: TODAY), "fromisoformat": date.fromisoformat})
        self.addCleanup(setattr, r, "date", saved)
        self.assertEqual(r.nightly(args, {}, github), 0)
        self.assertEqual(github.writes, [])

    def test_candidates_come_from_every_page_of_a_long_range(self):
        github = self.package_failed()
        github.commits = [f"x{n}" for n in range(250)] + ["m1"]
        self.assertIn("): #668", nightly(github).opened()[0]["body"])

    def test_a_range_longer_than_is_read_says_the_candidates_are_unknown(self):
        github = self.package_failed()
        github.total_commits = r.PAGES * 100 + 1
        github.commits = [f"x{n}" for n in range(r.PAGES * 100)]
        self.assertIn("unknown, more than 1000 commits since then", nightly(github).opened()[0]["body"])

    def test_no_green_nightly_in_reach_leaves_the_candidates_unknown(self):
        github = FakeGitHub([job(1, PACKAGE, "failure")], annotations={1: [failed_test("desktop/e2e/s.spec.ts", 29, "server picker")]})
        github.add_earlier(90, GREEN, "2026-10-05T18:00:00Z", [job(11, PACKAGE, "failure")])
        self.assertIn("unknown without a green nightly", nightly(github).opened()[0]["body"])


class Lanes(unittest.TestCase):
    def test_a_shard_is_not_part_of_the_lane(self):
        self.assertEqual(r.lane_of("web e2e (Windows @platform) / web e2e nightly-windows 2/4"),
                         "web e2e (Windows @platform) / web e2e nightly-windows")

    def test_a_step_failure_names_the_workflows_its_lane_ran_through_and_their_scripts(self):
        paths = r.lane_paths("verify / web e2e / web e2e linux", SCRIPT.parents[1])
        self.assertEqual(paths[:3], [".github/workflows/nightly.yml", ".github/workflows/pr.yml", ".github/workflows/web-e2e.yml"])
        self.assertIn("scripts/verify-web.sh", paths)
        self.assertEqual(r.lane_paths("desktop e2e (macos full)", SCRIPT.parents[1])[0], ".github/workflows/nightly.yml")

    def test_the_jobs_that_only_report_are_the_ones_the_workflows_name(self):
        nightly_yml = (SCRIPT.parents[1] / ".github" / "workflows" / "nightly.yml").read_text()
        pr_yml = (SCRIPT.parents[1] / ".github" / "workflows" / "pr.yml").read_text()
        report_job, gate = r.REPORT_JOBS
        self.assertIn(f"    name: {report_job}\n", nightly_yml)
        caller, callee = gate.split(" / ")
        self.assertRegex(nightly_yml, rf"\n  verify:\n    name: {caller}\n    if: .*\n    uses: ./.github/workflows/pr.yml")
        self.assertIn(f"\n  verify:\n    name: {callee}\n", pr_yml)


class Gh(unittest.TestCase):
    def test_an_answer_is_read_as_utf8_whatever_the_runner_locale(self):
        # A child's text is read in the locale's encoding unless one is named, which is
        # cp1252 on a Windows runner: an open issue with a Korean title broke every report there.
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "gh"
            fake.write_text("#!/bin/sh\nprintf '[{\"title\": \"\\355\\203\\255\"}]'\n")
            fake.chmod(0o755)
            load = (
                "import importlib.util, sys; s = importlib.util.spec_from_file_location('r', sys.argv[1]); "
                "m = importlib.util.module_from_spec(s); s.loader.exec_module(m); print(ascii(m.gh(['x'])))"
            )
            done = subprocess.run(
                [sys.executable, "-X", "utf8=0", "-c", load, str(SCRIPT)],
                env={"PATH": f"{tmp}{os.pathsep}{os.environ['PATH']}", "LC_ALL": "C", "PYTHONCOERCECLOCALE": "0"},
                capture_output=True, text=True, encoding="utf-8",
            )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.strip(), ascii([{"title": "탭"}]))


    def test_a_job_log_is_read_whatever_bytes_it_carries(self):
        # gh 2.97 and later refuse a raw body carrying a terminal escape unless told
        # --allow-escape-sequences, and every job log carries one: the nightly report of
        # run 38958746826 died on it and filed nothing. The stand-in answers as such a gh.
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "log").write_bytes(JOB_LOG)
            fake = Path(tmp) / "gh"
            fake.write_text(
                '#!/bin/sh\ncase " $* " in\n'
                '  " --version ") echo "gh version 2.97.0 (2026-07-31)" ;;\n'
                '  *" --allow-escape-sequences "*) cat "$(dirname "$0")/log" ;;\n'
                '  *) echo "the response contains terminal escape sequences; pass --allow-escape-sequences to output it anyway" >&2; exit 1 ;;\n'
                'esac\n'
            )
            fake.chmod(0o755)
            self.use_gh(tmp)
            text = r.gh(["repos/o/r/actions/jobs/117168142601/logs"], raw=True)
        self.assertIsNone(CONTROL_CHARACTER.search(text), repr(text))
        self.assertIn("Z bash scripts/verify-cargo.sh nextest --profile ci $TEST_TARGETS\n", text)
        # An unterminated string ends with its line, and a carriage return breaks one.
        self.assertIn("0950000Z \n2026-10-16T20:29:27.0960000Z Compiling a\nCompiling b\n", text)
        self.assertTrue(r.log_excerpt(text).endswith(
            "TRY 2 FAIL [ 105.471s] (1/2) hide-agent-hooks::it agy_binary::a_signed_in_agy_turn_fires_hide_s_hooks\n"
            "error: test run failed \ufffd\n##[error]Process completed with exit code 100."))

    def test_a_gh_too_old_to_read_a_job_log_is_refused_before_it_is_asked_for_one(self):
        # gh 2.76.2 does not know --allow-escape-sequences: asked for a log, it answers
        # `unknown flag` and its usage, which says nothing of the version the report needs.
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "gh"
            fake.write_text(
                '#!/bin/sh\necho "$*" >> "$(dirname "$0")/asked"\ncase "$1" in\n'
                '  --version) printf "gh version 2.76.2 (2025-07-30)\\nhttps://github.com/cli/cli/releases/tag/v2.76.2\\n" ;;\n'
                '  *) echo "unknown flag: --allow-escape-sequences" >&2; exit 1 ;;\n'
                'esac\n'
            )
            fake.chmod(0o755)
            self.use_gh(tmp)
            with self.assertRaises(RuntimeError) as refused:
                r.gh(["repos/o/r/actions/jobs/117168142601/logs"], raw=True)
            asked = (Path(tmp) / "asked").read_text()
        self.assertEqual(str(refused.exception), "needs gh 2.97 or newer to read job logs (found 2.76.2); upgrade gh")
        self.assertEqual(asked, "--version\n")

    def use_gh(self, folder):
        """Put the stand-in `gh` in `folder` first on PATH and forget the version read from another."""
        saved = os.environ["PATH"]
        os.environ["PATH"] = f"{folder}{os.pathsep}{saved}"
        self.addCleanup(os.environ.__setitem__, "PATH", saved)
        r.gh_reads_logs.cache_clear()
        self.addCleanup(r.gh_reads_logs.cache_clear)

    def test_a_test_name_outside_the_runner_code_page_is_filed_and_printed(self):
        # A Windows runner writes stdout as cp1252; the ⌘ in a test name raised
        # after its issue was filed, and the run reported it as not filed.
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "gh"
            fake.write_text('#!/bin/sh\ncase "$2" in *labels=*) echo "[]" ;; *) echo \'{"number": 50, "html_url": "https://example/50"}\' ;; esac\n')
            fake.chmod(0o755)
            report = Path(tmp) / "report.json"
            report.write_text(json.dumps({"suites": [{"title": "a.spec.ts", "specs": [spec("⌘D splits the pane", "flaky")]}]}))
            done = subprocess.run(
                [sys.executable, str(SCRIPT), "--suite", "web", "--playwright", str(report), "--system", "Windows"],
                env={**ENV, "PATH": f"{tmp}{os.pathsep}{os.environ['PATH']}", "PYTHONIOENCODING": "cp1252"},
                capture_output=True, text=True, encoding="utf-8",
            )
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertIn("filed https://example/50 for ⌘D splits the pane", done.stdout)


if __name__ == "__main__":
    unittest.main()
