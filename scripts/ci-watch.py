#!/usr/bin/env python3
"""Watch a `verify` run and end at its first failed job.

`gh run watch --exit-status` returns when the whole run ends, and a run goes on
for minutes after its first failure while every other lane finishes: over 191
pull request runs to 2026-11-21 a failure was known 15.9 minutes into a run at
the median, and the watch returned at 27.3. This watch reads the run's jobs
every `--interval` seconds and ends as soon as one has finished failed, naming
it and the command that prints its log. A failed step alone decides nothing: a
step marked `continue-on-error` (a flaky report, a plan record) can fail in a
job that passes.
It ends 0 when the run's `verify` job passes, and a run without one (a hand run
of another workflow) when the run succeeds.

    python3 scripts/ci-watch.py --pr <number>          the newest verify run on the pull request's head
    python3 scripts/ci-watch.py --branch main --sha <commit>   main's push run of a merge
    python3 scripts/ci-watch.py --run <run id>

Exit status: 0 the run passed, 1 a job failed, 2 the run ended otherwise (a
newer push cancelled it) or the watch could not finish (GitHub did not answer,
no run appeared, or `--timeout` passed). It asks GitHub's REST API through
`gh api` only. CONTRIBUTING.md, Waiting for CI, says when to run it.
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = "verify"
# What a finished job reports when it failed. A job ended by its own
# `timeout-minutes` reports `cancelled`, as does every job of a run a newer
# push replaced, so a cancelled job is judged only when the run has ended: a
# cancelled run is replaced, not failed. Every step that can hang has its own
# timeout, which fails the step, then the job, and so ends the watch.
FAILED = {"failure", "timed_out"}
PASSED = "success"


class Unanswered(Exception):
    """GitHub could not be asked, or did not answer what the watch needs."""


def gh_api(path):
    """GitHub's REST answer for `path`, `{owner}/{repo}` being this checkout's repository."""
    result = subprocess.run(["gh", "api", path], cwd=ROOT, capture_output=True, text=True)
    if result.returncode != 0:
        raise Unanswered(f"`gh api {path}` failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def run_jobs(api, run_id):
    """Every job of the run's latest attempt, across pages."""
    jobs, page = [], 1
    while True:
        answer = api(f"repos/{{owner}}/{{repo}}/actions/runs/{run_id}/jobs?filter=latest&per_page=100&page={page}")
        jobs += answer["jobs"]
        if not answer["jobs"] or len(jobs) >= answer["total_count"]:
            return jobs
        page += 1


def newest_verify(runs, sha=None):
    """The newest `verify` run among `runs`, on `sha` when given; None before one starts."""
    runs = [run for run in runs if run["name"] == WORKFLOW and (sha is None or run["head_sha"] == sha)]
    return max(runs, key=lambda run: (run["created_at"], run["id"])) if runs else None


def find_run(api, pr=None, branch=None, sha=None):
    """The run to watch for a pull request (its head's newest `verify` run) or a branch."""
    if pr is not None:
        sha = api(f"repos/{{owner}}/{{repo}}/pulls/{pr}")["head"]["sha"]
        path = f"repos/{{owner}}/{{repo}}/actions/runs?head_sha={sha}&event=pull_request&per_page=100"
    else:
        path = f"repos/{{owner}}/{{repo}}/actions/runs?branch={branch}&per_page=100"
    return newest_verify(api(path)["workflow_runs"], sha)


def failures(jobs):
    """Each finished job that failed, the one that ended first first."""
    failed = [job for job in jobs if job["status"] == "completed" and job["conclusion"] in FAILED]
    return sorted(failed, key=lambda job: (job.get("completed_at") or "", job["id"]))


def judge(run, jobs):
    """(exit status, lines) once the run is decided, or None while it is not."""
    failed = failures(jobs)
    if failed:
        job = failed[0]
        step = next((step["name"] for step in job.get("steps") or [] if step.get("conclusion") in FAILED), None)
        where = f" (step `{step}`)" if step else ""
        lines = [f"failed: {job['name']}{where}",
                 f"log: gh run view {run['id']} --job {job['id']} --log-failed",
                 f"     {job['html_url']}"]
        lines += [f"also failed: {other['name']}" for other in failed[1:]]
        return 1, lines
    verify = [job for job in jobs if job["name"] == WORKFLOW]
    if verify and verify[0]["status"] == "completed" and verify[0]["conclusion"] == PASSED:
        return 0, [f"passed: {WORKFLOW} ({run['html_url']})"]
    if run["status"] != "completed":
        return None
    if run["conclusion"] == "cancelled":
        return 2, [f"cancelled: run {run['id']} ended before {WORKFLOW} did; a newer push replaces it ({run['html_url']})"]
    if run["conclusion"] == PASSED and not verify:
        return 0, [f"passed: run {run['id']} ({run['html_url']})"]
    # A run that ended failed with no failed job or step: a job its own limit
    # cancelled, or `verify` skipped.
    ended = [job for job in jobs if job["conclusion"] not in (PASSED, "skipped")]
    if ended:
        job = min(ended, key=lambda job: (job.get("completed_at") or "", job["id"]))
        return 1, [f"failed: {job['name']} ({job['conclusion']})",
                   f"log: gh run view {run['id']} --job {job['id']} --log-failed",
                   f"     {job['html_url']}"]
    return 2, [f"ended: run {run['id']} {run['conclusion']} without a {WORKFLOW} verdict ({run['html_url']})"]


def watch(api, arguments, sleep=time.sleep, clock=time.monotonic, out=print):
    """Poll until the run is decided; the exit status."""
    deadline = clock() + arguments.timeout * 60
    run = None
    while run is None:
        if arguments.run is not None:
            run = api(f"repos/{{owner}}/{{repo}}/actions/runs/{arguments.run}")
        else:
            run = find_run(api, pr=arguments.pr, branch=arguments.branch, sha=arguments.sha)
        if run is None:
            # A run takes a few seconds to appear after a push.
            if clock() >= deadline:
                out(f"no {WORKFLOW} run appeared within {arguments.timeout} minutes")
                return 2
            sleep(arguments.interval)
    out(f"watching run {run['id']} on {run['head_sha'][:10]}: {run['html_url']}")
    while True:
        run = api(f"repos/{{owner}}/{{repo}}/actions/runs/{run['id']}")
        verdict = judge(run, run_jobs(api, run["id"]))
        if verdict is not None:
            status, lines = verdict
            for line in lines:
                out(line)
            return status
        if clock() >= deadline:
            out(f"still running after {arguments.timeout} minutes: {run['html_url']}")
            return 2
        sleep(arguments.interval)


def main(argv=None, api=gh_api):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--pr", type=int, help="watch the newest verify run on this pull request's head commit")
    target.add_argument("--branch", help="watch the newest verify run on this branch, such as main after a merge")
    target.add_argument("--run", type=int, help="watch this run")
    parser.add_argument("--sha", help="with --branch, the commit the run must be for")
    parser.add_argument("--interval", type=int, default=30, help="seconds between reads (default 30)")
    parser.add_argument("--timeout", type=int, default=120, help="minutes before the watch gives up (default 120)")
    arguments = parser.parse_args(argv)
    if arguments.sha and arguments.branch is None:
        parser.error("--sha goes with --branch")
    try:
        return watch(api, arguments)
    except Unanswered as error:
        print(f"ci-watch: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
