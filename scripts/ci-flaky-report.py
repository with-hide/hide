#!/usr/bin/env python3
"""Turn one CI run's test outcomes into GitHub issues.

Playwright and cargo-nextest retry a failed test once in CI. Each lane's report
step reads its tool's own report (Playwright's JSON, nextest's JUnit):

- a test that passes the retry is flaky, and the run passes. The step files it
  as an issue labelled `quarantine` with a seven-day deadline, or adds the run
  to the open issue that already tracks it;
- a test that fails the retry too failed the lane. The step writes it on the
  job as an error annotation titled `Failed test`, with the test's file and
  line and its name on the first line of the message, and files nothing: the
  annotations are the record the nightly report reads, and the run's summary
  page shows the names.

    ci-flaky-report.py --suite web --playwright web/e2e-report.json
    ci-flaky-report.py --suite rust --junit target/nextest/ci/junit.xml

The nightly performance lane has no tests and no retry: its judge, in
`scripts/web-shell-measure/`, writes a record whose `failures` are the checks
the run failed, and `--measurement` writes each as an error annotation titled
`Failed measurement`, with the check on the first line:

    ci-flaky-report.py --measurement nightly-summary.json

A run with a flaky test also sets the step output `flaky=true`, so the lane
keeps the same logs it keeps for a failure: the first attempt's daemon, Herdr
and input logs are the evidence its issue needs, and a passing job uploads
nothing otherwise.

The nightly's report job (`--nightly`) is the one writer of `nightly-failure`
issues, so the run's cap, the recurrence comments and the closing never race:
one issue per test or measurement a lane annotated, or, for a failed job with
no such annotation, per lane and failed step; a comment per run attempt on the
issue already open for the same failure; and a lane that passes a scheduled
run on main closes its issues. A check that passed without measuring (a
`Not measured` warning annotation) keeps its lane from passing and is listed
in the job summary. docs/TESTING.md, "Flaky tests" and "Nightly failures",
owns the policy.

    ci-flaky-report.py --nightly --lane-input all
    ci-flaky-report.py --nightly --repo OWNER/REPO --run RUN_ID --dry-run

It talks to GitHub through `gh` (GH_TOKEN, GITHUB_REPOSITORY and the run's
GITHUB_* variables come from Actions). A lane's step needs `issues: write` and
must not fail the lane: the flaky run already passed, and the report is a
notice, so a GitHub error costs one missed issue the next flaky run files. The
nightly report reads job logs, which needs gh 2.97 or newer, and refuses an
older gh before the first log read. The report job needs `actions: read`, `checks: read`, `pull-requests: read` and
`issues: write`, and fails when GitHub does, so a failure it could not file is
a red job rather than a silence.
"""
import argparse
from datetime import date, timedelta
import functools
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

LABEL = "quarantine"
EXPIRY_DAYS = 7
# A run with more flaky tests than this is a broken environment, not flakiness;
# the rest are counted in the log, not filed.
MAX_PER_RUN = 20
# The oldest gh that hands over a job log: `gh_reads_logs`.
GH_FOR_LOGS = (2, 97)
# Terminal control in text read from a job log or bound for a public page:
# ECMA-48 escape sequences (CSI, such as the color the runner puts around each
# step's command in every job log; the OSC, DCS, SOS, PM and APC strings, each
# to its terminator or the end of its line; and the shorter forms) and every
# other C0 and C1 control but tab and newline. A carriage return ends a line
# (`plain`), as a terminal shows it.
CONTROL = re.compile(
    r"(?:\x1b\[|\x9b)[0-?]*[ -/]*[@-~]|\x1b(?:[\]PX^_][^\x07\x1b\n]*(?:\x07|\x1b\\)?|[ -/]*[0-~])?"
    r"|[\x00-\x08\x0b-\x1f\x7f-\x9f]"
)
# Hand-written issues quote a title cut short ("Agent tabs shrink in stages...").
MATCH_PREFIX = 20
# A home folder in a quoted path names the account that ran it; an excerpt
# goes to a public page. (The literals are split so the workstation-identity
# check does not read this line as one.)
HOME = re.compile(r"(/Users" r"/|/home" r"/|[A-Za-z]:\\Users" r"\\)[^/\\\s]+")
TOKEN = re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})")

# The annotation a lane writes for a test that failed its retry too, and the
# one that counts those it had no room for. GitHub keeps 10 error annotations
# per step (REST check runs, "Update a check run"); one report step runs this
# script once, so 8 tests and the count leave one for the runner's own.
FAILED_TITLE = "Failed test"
UNANNOTATED_TITLE = "Failed tests not annotated"
ANNOTATED_TESTS = 8
# A failed check of a performance measurement. It names no file: a figure can
# move with any change, so every pull request merged since the lane's last
# green nightly is a candidate.
MEASUREMENT_TITLE = "Failed measurement"
# A check that passed without measuring anything, written by the check as a
# warning annotation with the check on the first line and the reason after:
# a real agent's provider refused its turn on a quota the CI key had spent
# (docs/TESTING.md, "A real agent on CI"). Its job passes, since the cause is
# not the code's, but its lane is not green: it closes no issue, is no lane's
# last green nightly, and the report lists it.
NOT_MEASURED_TITLE = "Not measured"

NIGHTLY_LABEL = "nightly-failure"
INFRA_LABEL = "infra"
NIGHTLY_EXPIRY_DAYS = 2
# A broken environment fails everything at once; past this many new issues a
# run files one issue listing the rest (design principle 15).
NEW_ISSUES_PER_RUN = 10
# How far back the last green nightly of a lane is looked for.
GREEN_LOOKBACK_RUNS = 14
# Every listing reads at most this many pages of 100.
PAGES = 10
# Jobs that only report on the others: this job, and `verify`'s gate, which
# fails whenever a lane it waits for did and so names no failure of its own.
REPORT_JOBS = ("nightly report", "verify / verify")
# Full nightly runs moved these duplicate calls under verify. Keep old issue
# keys and green history attached to the identical suite, including build
# failures; OS-only dispatch still uses the former job names.
LANE_MOVES = {
    "web e2e (Windows @platform) / build hided nightly-windows": "verify / windows e2e / build hided windows",
    "web e2e (Windows @platform) / web e2e nightly-windows": "verify / windows e2e / web e2e windows",
    "desktop e2e (macos full) / desktop e2e nightly-macos": "verify / desktop e2e / desktop e2e macos",
    "desktop e2e (windows @platform) / desktop e2e nightly-windows": "verify / windows desktop / desktop e2e windows",
}
SHARD = re.compile(r" \d+/\d+$")
# Steps that prepare the runner rather than check the code: a failure there is
# the runner's or a download's. Anything else, a build included, is not.
INFRA_STEP = re.compile(
    r"Set up job|Keep line endings as committed|Install .+|Fetch the pinned Herdr runtime|\(runner\)"
    r"|Run (actions/checkout|actions/setup-node|actions/download-artifact|pnpm/action-setup|Swatinem/rust-cache)@.+"
)
# A failed job with no failed step lost its runner.
RUNNER_STEP = "(runner)"


def playwright_tests(report, suite, status):
    found = []

    def walk(node, trail):
        for spec in node.get("specs", []):
            for test in spec.get("tests", []):
                if test.get("status") != status:
                    continue
                results = test.get("results") or [{}]
                # A flaky test's evidence is its first attempt; a failed
                # test's is the last one, the failure that stands.
                result = results[0] if status == "flaky" else results[-1]
                found.append({
                    "file": f"{suite}/e2e/{spec['file']}",
                    "line": spec.get("line"),
                    "name": " > ".join(trail + [spec["title"]]),
                    "error": (result.get("error") or {}).get("message", ""),
                })
        for child in node.get("suites", []):
            walk(child, trail + [child["title"]])

    for top in report.get("suites", []):
        walk(top, [])
    return found


def playwright_flaky(report, suite):
    return playwright_tests(report, suite, "flaky")


def playwright_failed(report, suite):
    return playwright_tests(report, suite, "unexpected")


def nextest_error(node):
    # A hang ended by the profile's slow-timeout has no text or message, only
    # its type ("test timeout").
    return (node.text or "").strip() or node.get("message") or node.get("type") or ""


def nextest_flaky(root):
    found = []
    for case in root.iter("testcase"):
        flake = case.find("flakyFailure")
        if flake is None:
            flake = case.find("flakyError")
        if flake is not None:
            found.append({"file": None, "name": f"{case.get('classname')} {case.get('name')}", "error": nextest_error(flake)})
    return found


def nextest_failed(root):
    found = []
    for case in root.iter("testcase"):
        failure = case.find("failure")
        if failure is None:
            failure = case.find("error")
        if failure is None:
            continue
        classname = case.get("classname") or ""
        error = nextest_error(failure)
        if error == failure.get("type"):
            error = (case.findtext("system-err") or "").strip() or error
        found.append({
            # The crate's folder: nextest names a test by crate and binary, not file.
            "file": classname.split("::")[0] or None,
            "line": None,
            "name": f"{classname} {case.get('name')}",
            "error": error,
        })
    return found


def plain(text):
    """`text` with its line breaks as newlines and no terminal control."""
    return CONTROL.sub("", re.sub(r"\r\n?", "\n", text))


def clean(text):
    text = HOME.sub(r"\1~", plain(text))
    return TOKEN.sub("***", text).strip()[:600] or "(no message)"


def escape_data(text):
    return text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def escape_property(text):
    return escape_data(text).replace(":", "%3A").replace(",", "%2C")


def annotate(failed, write=print, title=FAILED_TITLE):
    """Writes each test that failed its retry (or each failed measurement) as an annotation on the job."""
    for test in failed[:ANNOTATED_TESTS]:
        properties = [f"file={escape_property(test['file'])}"] if test.get("file") else []
        if test.get("line"):
            properties.append(f"line={int(test['line'])}")
        properties.append(f"title={escape_property(title)}")
        name = " ".join(test["name"].splitlines())
        write(f"::error {','.join(properties)}::{escape_data(name + chr(10) + clean(test['error']))}")
    if len(failed) > ANNOTATED_TESTS:
        write(f"::error title={escape_property(UNANNOTATED_TITLE)}::{len(failed) - ANNOTATED_TESTS}")


def tracks(issue, test):
    """Whether an open issue is about this test: its file and the start of its name appear in the body."""
    body = issue.get("body") or ""
    if test["file"] and test["file"] not in body:
        return False
    name = test["name"] if not test["file"] else test["name"].split(" > ")[-1][:MATCH_PREFIX]
    return name in body


def run_facts(env):
    server, repo, run = env["GITHUB_SERVER_URL"], env["GITHUB_REPOSITORY"], env["GITHUB_RUN_ID"]
    pull = re.fullmatch(r"refs/pull/(\d+)/merge", env.get("GITHUB_REF", ""))
    return {
        "marker": f"<!-- flaky-run:{run}:{env.get('GITHUB_RUN_ATTEMPT', '1')} -->",
        "run": f"{server}/{repo}/actions/runs/{run}",
        "change": f"pull request #{pull.group(1)}" if pull else env.get("GITHUB_REF", "unknown ref"),
        "sha": env.get("GITHUB_SHA", "unknown")[:12],
    }


def label_of(test):
    return f"`{test['file']}` \"{test['name']}\"" if test["file"] else f"`{test['name']}`"


def new_issue(test, system, facts, today):
    expires = (today + timedelta(days=EXPIRY_DAYS)).isoformat()
    return {
        "title": f"Flaky test: {test['name']}"[:200],
        "labels": [LABEL, "bug"],
        "body": (
            "CI found this test flaky: it failed, then passed on the one retry in the same job, so the run passed.\n\n"
            f"- Test: {label_of(test)}\n- System: {system}\n"
            f"- First seen: {facts['run']} ({facts['change']}, commit {facts['sha']})\n- Expires: {expires}\n\n"
            f"First attempt's failure:\n\n```\n{clean(test['error'])}\n```\n\n"
            "Fix the cause or delete the test by the expiry date; a flaky test is not left running. "
            "A longer timeout, more retries or a lower expectation does not end this issue. "
            "Each later flaky run adds a comment here.\n"
        ),
    }


def comment(test, system, facts, expired):
    late = "\n\nThis issue is past its expiry date. Decide now: fix it, delete the test, or say here why not." if expired else ""
    return (
        f"{facts['marker']}\nFlaky again on {system}: {facts['run']} ({facts['change']}, commit {facts['sha']}).\n\n"
        f"```\n{clean(test['error'])}\n```{late}"
    )


@functools.cache
def gh_reads_logs():
    """Refuse a gh too old to read a job log, before the first one is read.

    gh 2.97 and later refuse a raw body that carries a terminal escape, as every
    job log does, unless told `--allow-escape-sequences`, which an older gh does
    not know: it would answer `unknown flag` and print its usage.
    """
    try:
        done = subprocess.run(["gh", "--version"], capture_output=True)
    except OSError as error:
        raise RuntimeError(f"gh could not run: {error}") from error
    said = done.stdout.decode("utf-8", "replace")
    version = re.match(r"gh version ((\d+)\.(\d+)\S*)", said)
    if done.returncode or not version:
        raise RuntimeError(f"gh --version did not say a version: {(said or done.stderr.decode('utf-8', 'replace')).strip()}")
    if (int(version[2]), int(version[3])) < GH_FOR_LOGS:
        raise RuntimeError(
            f"needs gh {'.'.join(map(str, GH_FOR_LOGS))} or newer to read job logs (found {version[1]}); upgrade gh"
        )


def gh(args, body=None, raw=False):
    """`gh api`'s JSON answer, or with `raw` a body's text with its terminal control removed.

    A raw body, such as a job log, is the bytes the job wrote, terminal escapes
    and all (`gh_reads_logs`), and a job may have written bytes that are not UTF-8.
    """
    if raw:
        gh_reads_logs()
    try:
        done = subprocess.run(
            ["gh", "api", *(["--allow-escape-sequences"] if raw else []), *args],
            input=None if body is None else json.dumps(body).encode("utf-8"), capture_output=True,
        )
    except OSError as error:
        raise RuntimeError(f"gh could not run: {error}") from error
    # gh speaks UTF-8; a named encoding keeps a Windows runner from reading it as cp1252.
    if done.returncode:
        raise RuntimeError(f"gh api {' '.join(args)}: {done.stderr.decode('utf-8', 'replace').strip()}")
    if raw:
        return plain(done.stdout.decode("utf-8", "replace"))
    return json.loads(done.stdout.decode("utf-8")) if done.stdout.strip() else None


def open_issues(repo, call=gh):
    issues = listing(call, f"repos/{repo}/issues?labels={LABEL}&state=open")
    return [issue for issue in issues if "pull_request" not in issue]


def report(tests, system, env, call=gh, today=None):
    today = today or date.today()
    repo, facts = env["GITHUB_REPOSITORY"], run_facts(env)
    if len(tests) > MAX_PER_RUN:
        print(f"::warning::{len(tests)} flaky tests in one run; filing the first {MAX_PER_RUN}")
    issues = open_issues(repo, call)
    lines = [f"### Flaky tests ({system})", ""]
    for test in tests[:MAX_PER_RUN]:
        issue = next((item for item in issues if tracks(item, test)), None)
        if issue is None:
            created = call([f"repos/{repo}/issues", "--input", "-"], new_issue(test, system, facts, today))
            issues.append({**created, "body": created.get("body") or new_issue(test, system, facts, today)["body"]})
            print(f"filed {created['html_url']} for {test['name']}")
            lines.append(f"- filed {created['html_url']}: {label_of(test)}")
            continue
        number = issue["number"]
        comments = listing(call, f"repos/{repo}/issues/{number}/comments")
        lines.append(f"- #{number}: {label_of(test)}")
        if any(facts["marker"] in (item.get("body") or "") for item in comments):
            continue
        match = re.search(r"^- Expires: (\d{4}-\d\d-\d\d)", issue.get("body") or "", re.M)
        expired = bool(match) and date.fromisoformat(match.group(1)) < today
        call([f"repos/{repo}/issues/{number}/comments", "--input", "-"], {"body": comment(test, system, facts, expired)})
        print(f"added the run to #{number} for {test['name']}")
    summary(lines)


def summary(lines):
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write("\n".join(lines) + "\n")


# The nightly report job.


def listing(call, path, key=None):
    """Every entry of a paged listing, up to PAGES pages; more is an error, not a cut."""
    items = []
    for page in range(1, PAGES + 1):
        data = call([f"{path}{'&' if '?' in path else '?'}per_page=100&page={page}"])
        batch = data[key] if key else data
        items.extend(batch)
        if len(batch) < 100:
            return items
    raise RuntimeError(f"{path} lists more than {PAGES * 100} entries")


def lane_of(job_name):
    """A job's lane: its name without the shard, so a test is the same failure in whichever shard it ran."""
    return SHARD.sub("", job_name).replace(":", "-")


def canonical_key(key):
    """Resolve a moved suite's lane while retaining its test or step identity."""
    lane, separator, subject = key.partition(":")
    return LANE_MOVES.get(lane, lane) + separator + subject


def marker_text(text):
    return " ".join(text.split()).replace("--", "-")


def failure_marker(key):
    return f"<!-- {NIGHTLY_LABEL}:{key} -->"


def key_of(issue):
    match = re.search(rf"<!-- {NIGHTLY_LABEL}:(.+?) -->", issue.get("body") or "")
    return match.group(1) if match else None


def overflow_run(issue):
    match = re.search(r"<!-- nightly-overflow:(\d+) -->", issue.get("body") or "")
    return match.group(1) if match else None


def annotation_path(job):
    # The job's check run, named by the job rather than assumed from its id.
    return job["check_run_url"].split("api.github.com/", 1)[1] + "/annotations"


def unannotated_count(annotation):
    first = (annotation.get("message") or "").partition("\n")[0].strip()
    return int(first) if first.isdigit() else 1


def test_failure(job, lane, annotation):
    name, _, error = (annotation.get("message") or "").partition("\n")
    if annotation.get("title") == MEASUREMENT_TITLE:
        # Its candidates are every merged pull request, so it names no paths.
        return {
            "kind": "measurement", "lane": lane, "key": marker_text(f"{lane}:measurement {name}"), "job": job,
            "name": name, "error": error, "paths": None, "infra": False,
        }
    path = annotation.get("path")
    # A test with no file is annotated on GitHub's default path.
    file = None if path in (None, "", ".github") else path
    subject = f"{file} > {name}" if file else name
    return {
        "kind": "test", "lane": lane, "key": marker_text(f"{lane}:{subject}"), "job": job,
        "file": file, "line": annotation.get("start_line"), "name": name, "error": error,
        "paths": [file] if file else [], "infra": False,
    }


def log_excerpt(log):
    lines = [re.sub(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ?", "", line) for line in log.splitlines()]
    end = next((index for index, line in enumerate(lines) if "##[error]" in line), len(lines) - 1)
    return "\n".join(lines[max(0, end - 12):end + 1])


def step_failure(job, lane, log, root):
    steps = job.get("steps") or []
    # A job that ran out of time cancels the step it was in.
    step = next((s for s in steps if s.get("conclusion") == "failure"), None) \
        or next((s for s in steps if s.get("conclusion") == "cancelled"), None)
    name = step["name"] if step else RUNNER_STEP
    return {
        "kind": "step", "lane": lane, "key": marker_text(f"{lane}:step {name}"), "job": job,
        "step": name, "error": log_excerpt(log), "paths": lane_paths(lane, root),
        "infra": bool(INFRA_STEP.fullmatch(name)),
    }


def called_workflow(text, job_name):
    """The workflow a job of `text` named `job_name` calls, if it calls one."""
    for block in re.split(r"\n  (?=[\w-]+:\n)", text):
        name = re.search(r"^    name: (.+)$", block, re.M)
        uses = re.search(r"^    uses: \./(\.github/workflows/[\w.-]+)$", block, re.M)
        if not (name and uses):
            continue
        pattern = ".*".join(re.escape(part) for part in re.split(r"\$\{\{.*?\}\}", name.group(1).strip()))
        if re.fullmatch(pattern, job_name):
            return uses.group(1)
    return None


def lane_paths(lane, root):
    """The workflows a lane runs through and the scripts they call: what a step failure's candidate touched."""
    files = [".github/workflows/nightly.yml"]
    for segment in lane.split(" / ")[:-1]:
        called = called_workflow((root / files[-1]).read_text(encoding="utf-8"), segment)
        if called is None:
            break
        files.append(called)
    scripts = {
        script for file in files
        for script in re.findall(r"scripts/[\w./-]+\.(?:sh|py|mjs|cjs|ps1)", (root / file).read_text(encoding="utf-8"))
    }
    return files + sorted(scripts)


def touches(filenames, paths):
    return any(name == path or name.startswith(path.rstrip("/") + "/") for name in filenames for path in paths)


class Nightly:
    """One nightly run's report: what it opens, comments and closes."""

    def __init__(self, repo, run_id, attempt, lane_input, call, root, today):
        self.repo, self.call, self.root, self.today = repo, call, root, today
        self.run = call([f"repos/{repo}/actions/runs/{run_id}"])
        self.run_id = str(run_id)
        self.attempt = int(attempt or self.run["run_attempt"])
        self.lane_input = lane_input or ""
        self.marker = f"<!-- nightly-run:{self.run_id}:{self.attempt} -->"
        self.jobs_of = {}
        self.annotations_of = {}
        self.pr_files = {}

    def jobs(self, run_id, attempt):
        if (run_id, attempt) not in self.jobs_of:
            jobs = listing(self.call, f"repos/{self.repo}/actions/runs/{run_id}/attempts/{attempt}/jobs", "jobs")
            self.jobs_of[(run_id, attempt)] = [
                job for job in jobs if job.get("status") == "completed" and job["name"] not in REPORT_JOBS
            ]
        return self.jobs_of[(run_id, attempt)]

    def annotations(self, job):
        if job["id"] not in self.annotations_of:
            self.annotations_of[job["id"]] = listing(self.call, annotation_path(job))
        return self.annotations_of[job["id"]]

    def unmeasured(self, jobs):
        """Each check a passing job of `jobs` ran without measuring, as (lane, check, reason)."""
        return [
            (lane_of(job["name"]), *(annotation.get("message") or "").partition("\n")[::2])
            for job in jobs if job["conclusion"] == "success"
            for annotation in self.annotations(job) if annotation.get("title") == NOT_MEASURED_TITLE
        ]

    def green(self, jobs, lane):
        """Whether `lane` ran in `jobs`, every job of it passed, and none passed without measuring."""
        mine = [job for job in jobs if canonical_key(lane_of(job["name"])) == canonical_key(lane)]
        return bool(mine) and all(job["conclusion"] == "success" for job in mine) and not self.unmeasured(mine)

    def failures(self):
        """Each failure of this run once, in job order, and the count of failed tests no lane annotated."""
        found, unannotated = {}, 0
        for job in self.jobs(self.run_id, self.attempt):
            if job["conclusion"] not in ("failure", "timed_out"):
                continue
            lane = lane_of(job["name"])
            annotations = self.annotations(job)
            tests = [a for a in annotations if a.get("title") in (FAILED_TITLE, MEASUREMENT_TITLE)]
            left = sum(unannotated_count(a) for a in annotations if a.get("title") == UNANNOTATED_TITLE)
            unannotated += left
            if tests or left:
                items = [test_failure(job, lane, annotation) for annotation in tests]
            else:
                # The report step died or wrote nothing: the failure is still one.
                log = self.call([f"repos/{self.repo}/actions/jobs/{job['id']}/logs"], raw=True)
                items = [step_failure(job, lane, log, self.root)]
            for item in items:
                found.setdefault(item["key"], item)
        return list(found.values()), unannotated

    def last_green(self, lane):
        runs = self.call([
            f"repos/{self.repo}/actions/workflows/{self.run['workflow_id']}/runs"
            f"?branch=main&status=completed&per_page={GREEN_LOOKBACK_RUNS + 5}"
        ])["workflow_runs"]
        earlier = [
            run for run in runs
            if run["created_at"] < self.run["created_at"] and run["event"] in ("schedule", "workflow_dispatch")
        ][:GREEN_LOOKBACK_RUNS]
        for run in earlier:
            if self.green(self.jobs(str(run["id"]), run["run_attempt"]), lane):
                return run
        return None

    def candidates(self, green, paths):
        """Pull requests merged into main since `green` that touched `paths` (all of them for None), or None when the range is too long to read."""
        base, head = green["head_sha"], self.run["head_sha"]
        shas, total = set(), None
        for page in range(1, PAGES + 1):
            compare = self.call([f"repos/{self.repo}/compare/{base}...{head}?per_page=100&page={page}"])
            total = compare.get("total_commits", total)
            shas.update(commit["sha"] for commit in compare["commits"])
            if len(compare["commits"]) < 100:
                break
        if total is not None and len(shas) < total:
            return None
        merged = []
        for page in range(1, PAGES + 1):
            pulls = self.call([f"repos/{self.repo}/pulls?state=closed&base=main&sort=updated&direction=desc&per_page=100&page={page}"])
            merged += [pull for pull in pulls if pull.get("merge_commit_sha") in shas]
            # A pull request merged in the range was updated at its merge, after the green run.
            if len(pulls) < 100 or pulls[-1]["updated_at"] < green["created_at"]:
                break
        found = []
        for pull in sorted(merged, key=lambda pull: pull["number"]):
            number = pull["number"]
            if paths is None:
                found.append(number)
                continue
            if number not in self.pr_files:
                files = listing(self.call, f"repos/{self.repo}/pulls/{number}/files")
                self.pr_files[number] = [f["filename"] for f in files] + [f["previous_filename"] for f in files if f.get("previous_filename")]
            if touches(self.pr_files[number], paths):
                found.append(number)
        return found

    def subject(self, failure):
        if failure["kind"] == "measurement":
            return f"\"{failure['name']}\""
        if failure["kind"] == "test":
            where = f"{failure['file']}:{failure['line']}" if failure["file"] and failure["line"] else failure["file"]
            return f"`{where}` \"{failure['name']}\"" if where else f"`{failure['name']}`"
        return f"step \"{failure['step']}\""

    def new_issue(self, failure):
        run_url, sha = self.run["html_url"], self.run["head_sha"][:12]
        green = self.last_green(failure["lane"])
        if green is None:
            since = f"- Last green nightly of this lane: none in the last {GREEN_LOOKBACK_RUNS} nightly runs\n"
            candidates = "- Candidate pull requests: unknown without a green nightly to start from\n"
        else:
            since = f"- Last green nightly of this lane: {green['head_sha'][:12]} ({green['html_url']})\n"
            paths = failure["paths"]
            numbers = self.candidates(green, paths) if paths is None or paths else []
            if paths is None:
                touched = "any path, since a measurement moves with any change"
            else:
                touched = ", ".join(f"`{path}`" for path in paths[:6]) + (", ..." if len(paths) > 6 else "")
            if numbers is None:
                listed = f"unknown, more than {PAGES * 100} commits since then"
            else:
                listed = ", ".join(f"#{number}" for number in numbers) or "none"
            candidates = f"- Candidate pull requests (merged since then, touching {touched or 'nothing named'}): {listed}\n"
        label = {"test": "Test", "measurement": "Measurement"}.get(failure["kind"])
        what = f"- {label}: {self.subject(failure)}\n" if label else f"- Step: \"{failure['step']}\"\n"
        # The lane is in the title: the same test failing on two systems is two issues, told apart in a list.
        title = f"Nightly failure ({failure['lane']}): " + (failure["step"] if failure["kind"] == "step" else failure["name"])
        labels = [NIGHTLY_LABEL] + ([INFRA_LABEL] if failure["infra"] else [])
        return {
            "title": title[:200],
            "labels": labels,
            "body": (
                f"{failure_marker(failure['key'])}\n{self.marker}\n"
                f"Nightly CI failed in the `{failure['lane']}` lane.\n\n"
                f"{what}- Lane: `{failure['lane']}` on {', '.join(failure['job'].get('labels') or ['an unnamed runner'])}\n"
                f"- Run: {run_url} (attempt {self.attempt}), job {failure['job']['html_url']}\n"
                f"- Commit: {sha}\n{since}{candidates}"
                f"- Expires: {(self.today + timedelta(days=NIGHTLY_EXPIRY_DAYS)).isoformat()}\n\n"
                f"```\n{clean(failure['error'])}\n```\n\n"
                "Fix the cause or revert the candidate pull request; a longer timeout, more retries or a skip does not end this issue. "
                "Each nightly that fails here again adds a comment, and the first scheduled nightly on main that passes this lane closes it.\n"
            ),
        }

    def recurrence(self, failure):
        return {"body": (
            f"{self.marker}\nFailed again at {self.run['head_sha'][:12]}: {self.run['html_url']} (attempt {self.attempt}).\n\n"
            f"```\n{clean(failure['error'])}\n```"
        )}

    def overflow_issue(self, failures, unannotated):
        lines = [f"- `{failure['lane']}`: {self.subject(failure)}" for failure in failures]
        if unannotated:
            lines.append(f"- {unannotated} failed test(s) a lane had no annotation room for; read the failed jobs")
        count = len(failures) + unannotated
        return {
            "title": f"nightly: {count} more failures in run {self.run_id}",
            "labels": [NIGHTLY_LABEL],
            "body": (
                f"<!-- nightly-overflow:{self.run_id} -->\n{self.marker}\n"
                f"Nightly run {self.run['html_url']} (commit {self.run['head_sha'][:12]}) failed more than "
                f"{NEW_ISSUES_PER_RUN} ways at once, which is usually one broken environment. These were not filed one by one:\n\n"
                + "\n".join(lines)
                + "\n\nThe next scheduled nightly files whichever still fail and closes this issue.\n"
            ),
        }

    def commented(self, issue):
        if self.marker in (issue.get("body") or ""):
            return True
        comments = listing(self.call, f"repos/{self.repo}/issues/{issue['number']}/comments")
        return any(self.marker in (item.get("body") or "") for item in comments)

    def plan(self):
        """What this run writes, as (action, issue number or None, payload), without writing it."""
        if self.run.get("head_branch") != "main":
            return []
        issues = [
            issue for issue in listing(self.call, f"repos/{self.repo}/issues?labels={NIGHTLY_LABEL}&state=open")
            if "pull_request" not in issue
        ]
        open_by_key = {canonical_key(key_of(issue)): issue for issue in issues if key_of(issue)}
        failures, unannotated = self.failures()
        actions, opened, over = [], 0, []
        for failure in failures:
            issue = open_by_key.get(canonical_key(failure["key"]))
            if issue is not None:
                if not self.commented(issue):
                    actions.append(("comment", issue["number"], self.recurrence(failure)))
            elif opened < NEW_ISSUES_PER_RUN:
                actions.append(("open", None, self.new_issue(failure)))
                opened += 1
            else:
                over.append(failure)
        if over or unannotated:
            mine = next((issue for issue in issues if overflow_run(issue) == self.run_id), None)
            if mine is None:
                actions.append(("open", None, self.overflow_issue(over, unannotated)))
        scheduled = self.run["event"] == "schedule"
        by_hand = self.run["event"] == "workflow_dispatch" and self.lane_input in ("", "all")
        if scheduled or by_hand:
            jobs = self.jobs(self.run_id, self.attempt)
            passed = {"body": f"Passed at {self.run['head_sha'][:12]}: {self.run['html_url']}"}
            for issue in issues:
                key, earlier = key_of(issue), overflow_run(issue)
                if key and self.green(jobs, key.split(":", 1)[0]):
                    actions.append(("close", issue["number"], passed))
                elif earlier and earlier != self.run_id:
                    actions.append(("close", issue["number"], {"body": f"Run {self.run['html_url']} files its own failures."}))
        return actions

    def apply(self, actions):
        # Read before writing, so a GitHub error here leaves nothing half written.
        unmeasured = self.unmeasured_lines()
        lines = ["### Nightly failures", ""]
        for action, number, payload in actions:
            if action == "open":
                created = self.call([f"repos/{self.repo}/issues", "--input", "-"], payload)
                lines.append(f"- opened {created['html_url']}: {payload['title']}")
            elif action == "comment":
                self.call([f"repos/{self.repo}/issues/{number}/comments", "--input", "-"], payload)
                lines.append(f"- commented on #{number}")
            else:
                self.call([f"repos/{self.repo}/issues/{number}/comments", "--input", "-"], payload)
                self.call([f"repos/{self.repo}/issues/{number}", "-X", "PATCH", "--input", "-"], {"state": "closed", "state_reason": "completed"})
                lines.append(f"- closed #{number}")
        for line in lines[2:]:
            print(line[2:])
        for line in unmeasured[2:]:
            print(f"not measured: {line[2:]}")
        summary((lines if actions else lines + ["Nothing to write."]) + ([""] + unmeasured if unmeasured else []))

    def unmeasured_lines(self):
        """This run's checks that passed without measuring, as a summary section; none is no section."""
        found = self.unmeasured(self.jobs(self.run_id, self.attempt))
        return ["### Not measured", ""] + [f"- `{lane}`: {check}: {reason}" for lane, check, reason in found] if found else []


def describe(actions):
    """The plan as text, for a dry run."""
    if not actions:
        return "nothing to write"
    out = []
    for action, number, payload in actions:
        if action == "open":
            out.append(f"would open [{', '.join(payload['labels'])}] {payload['title']}")
            out.extend("    " + line for line in payload["body"].splitlines())
        else:
            out.append(f"would {action} #{number}")
            out.extend("    " + line for line in payload["body"].splitlines())
    return "\n".join(out)


def nightly(args, env, call=gh):
    repo = args.repo or env["GITHUB_REPOSITORY"]
    run = args.run or env["GITHUB_RUN_ID"]
    attempt = args.attempt or (None if args.run else env.get("GITHUB_RUN_ATTEMPT"))
    if not args.dry_run and env.get("GITHUB_REF") != "refs/heads/main":
        print("not a run of main; the nightly report writes nothing")
        return 0
    report_run = Nightly(repo, run, attempt, args.lane_input, call, Path(__file__).resolve().parents[1], date.today())
    actions = report_run.plan()
    if args.dry_run:
        unmeasured = report_run.unmeasured_lines()
        print(describe(actions) + ("\n\n" + "\n".join(unmeasured) if unmeasured else ""))
        return 0
    report_run.apply(actions)
    return 0


def measurement(path):
    """Annotates each check a performance lane's record failed; a run that wrote no record has its failed step instead."""
    if not path.is_file():
        print("no record; nothing to annotate")
        return 0
    failures = json.loads(path.read_text(encoding="utf-8"))["failures"]
    annotate([{"file": None, "line": None, "name": f["name"], "error": f["error"]} for f in failures], title=MEASUREMENT_TITLE)
    print(f"{len(failures)} failed measurement(s)" if failures else "no failed measurement")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--suite", choices=("web", "desktop", "rust"))
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--playwright", type=Path, help="a Playwright JSON report")
    source.add_argument("--junit", type=Path, nargs="+", help="cargo-nextest JUnit reports (a lane writes one per nextest run)")
    source.add_argument("--measurement", type=Path, help="a performance lane's record, whose failures are annotated")
    parser.add_argument("--system", default=os.environ.get("RUNNER_OS", "unknown"))
    parser.add_argument("--nightly", action="store_true", help="the nightly report job: file, comment on and close nightly-failure issues (needs gh 2.97 or newer)")
    parser.add_argument("--repo", help="OWNER/REPO (default: GITHUB_REPOSITORY)")
    parser.add_argument("--run", help="the nightly run to report (default: this run)")
    parser.add_argument("--attempt", help="its attempt (default: this attempt, or the run's latest)")
    parser.add_argument("--lane-input", default="", help="the hand run's lane input; empty for the schedule")
    parser.add_argument("--dry-run", action="store_true", help="print what the nightly report would write, and write nothing")
    args = parser.parse_args(argv)
    if args.nightly:
        try:
            return nightly(args, os.environ)
        except RuntimeError as error:
            # A GitHub or gh failure is the job's red, said in one line.
            print(f"nightly report: {error}", file=sys.stderr)
            return 1
    if args.measurement:
        return measurement(args.measurement)
    if not args.suite or not (args.playwright or args.junit):
        parser.error("a lane's report needs --suite and --playwright or --junit")
    paths = [args.playwright] if args.playwright else args.junit
    paths = [path for path in paths if path.is_file()]
    if not paths:
        print("no report; nothing to file")
        return 0
    if args.playwright:
        report_data = json.loads(paths[0].read_text(encoding="utf-8"))
        tests, failed = playwright_flaky(report_data, args.suite), playwright_failed(report_data, args.suite)
    else:
        roots = [ET.parse(path).getroot() for path in paths]
        tests = [test for root in roots for test in nextest_flaky(root)]
        failed = [test for root in roots for test in nextest_failed(root)]
    annotate(failed)
    if not tests:
        print("no flaky test in this run")
        return 0
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write("flaky=true\n")
    try:
        report(tests, args.system, os.environ)
    except (RuntimeError, KeyError, ValueError) as error:
        # The lane has passed and stays passed, so the unfiled tests go where a
        # reader of the run sees them: the annotation and the job summary.
        print(f"::warning::the flaky report failed, so {len(tests)} flaky test(s) are not filed: {error}")
        summary(["### Flaky tests that were not filed", "", f"Filing failed: {error}", ""]
                + [f"- {label_of(test)} ({args.system})" for test in tests])
        return 1
    return 0


if __name__ == "__main__":
    # A test name can hold any character (⌘D splits ...) and the Actions log
    # reads UTF-8, but a Windows runner writes stdout as cp1252: printing
    # the name of an issue just filed raised, and the run was told nothing was filed.
    sys.stdout.reconfigure(encoding="utf-8")
    sys.exit(main())
