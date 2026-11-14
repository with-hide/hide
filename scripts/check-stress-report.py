#!/usr/bin/env python3
"""Fail a repeated run when any of its iterations failed, whatever nextest's exit.

nextest's `--stress-count N` reports a run as passed when its last iteration
passed: iterations 1 to N-1 can each fail and the command still ends with exit
0. On 2026-11-14 a `wsl device windows` job ended successful while its own
summary said `20/20 stress run iterations: 19 passed, 1 failed`
(run 43626029599), and the lane that repeats a pull request's changed tests
uses the same flag (`repeat.yml`).

This reads the report a repeated run writes rather than its exit status. A
nextest JUnit report carries one suite per iteration, so a testcase with a
`failure` or `error` child, or a run whose summary counted a failure, is a
failed iteration. `--summary` reads the stress summary line nextest prints,
which names the count itself:

    Summary [ 778.209s] 20/20 stress run iterations: 19 passed, 1 failed

Either source is enough on its own; a caller passes the JUnit reports it has
and, when it kept the log, the summary text. Nothing here retries or reports a
flake: the lanes this serves (`repeat`, `remote-mailbox`, `wsl-device`) run
with no retry, where one failed iteration is a failed run.

Usage:

    check-stress-report.py --junit target/nextest/ci/junit*.xml
    check-stress-report.py --summary "$RUNNER_TEMP/nextest.log"
    check-stress-report.py --junit <reports> --summary <log> --repeat 10

A testcase's failures are counted once per iteration, so a report of one
failing iteration among ten fails the run and names the test. A report that
is missing, unreadable, or holds no testcase at all fails too: a lane that
cannot show what it ran proves nothing.
"""
import argparse
from pathlib import Path
import re
import sys
import xml.etree.ElementTree as ET

# `Summary [ 778.209s] 20/20 stress run iterations: 19 passed, 1 failed`, the
# line nextest prints after a `--stress-count` run. A run with no failure has
# no `, N failed`, and nextest prints `20/20 stress run iterations: 20 passed`.
_SUMMARY = re.compile(
    r"Summary\s*\[[^\]]*\]\s*(?P<iterations>\d+)/(?P<total>\d+) stress run iterations: "
    r"(?P<passed>\d+) passed(?:, (?P<failed>\d+) failed)?"
)
REPORT_FILES = 128
REPORT_BYTES = 16 * 1024 * 1024


def summary_failures(text):
    """The failed iterations and tests a nextest stress summary names, as
    `(iterations, failed)`; `(0, 0)` when the text holds no failure."""
    found = [(int(match["iterations"]), int(match["failed"] or 0)) for match in _SUMMARY.finditer(text)]
    if not found:
        return (0, 0)
    # Several runs in one log (a lane that ran more than one target) add up.
    return (sum(iterations for iterations, _ in found), sum(failed for _, failed in found))


def junit_failures(paths):
    """The failed testcases the reports hold, as `[(test, where)]`.

    A testcase fails when it holds a `failure` or `error` child; a `skipped`
    testcase ran nothing and is neither a pass nor a failure. `where` says
    which run it was: nextest names each of a report's suites `...@stress-<n>`,
    so that suffix is the iteration, and a lane that writes one report per
    iteration (`repeat.yml`'s unit tests) is named by the report's file.
    A single report of a run with one suite has no iteration to name.
    """
    failed = []
    for path in paths:
        report = ET.parse(path).getroot()
        if report.tag != "testsuites":
            raise ValueError(f"not a JUnit report: {path}")
        for suite in report.iter("testsuite"):
            stress = suite.get("name", "").rpartition("@stress-")[2]
            if stress.isdigit():
                where = f"iteration {stress}"
            else:
                where = Path(path).stem if len(paths) > 1 else ""
            for case in suite.iter("testcase"):
                if any(child.tag in ("failure", "error") for child in case):
                    failed.append((case.get("name", "a test"), where))
    return failed


def checked(paths, text, repeat=0):
    """The completed iterations and the failures to report.

    Reads every report and the summary, so either alone can fail the run.
    """
    if paths:
        if len(paths) > REPORT_FILES or sum(path.stat().st_size for path in paths) > REPORT_BYTES:
            raise ValueError("the reports are missing or exceed 128 files / 16 MiB")
    failed_cases = junit_failures(paths) if paths else []
    iterations, reported = summary_failures(text or "")
    if not paths and not text:
        raise ValueError("no report or summary was given")
    return {"iterations": iterations, "cases": failed_cases, "summary_failed": reported}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--junit", nargs="*", default=[], help="the nextest JUnit reports of the repeated run")
    parser.add_argument("--summary", help="a file holding the run's nextest output, for its stress summary line")
    parser.add_argument("--repeat", type=int, default=0, help="the iterations asked for, to check the report is whole")
    args = parser.parse_args()

    try:
        paths = [Path(path) for path in args.junit]
        for path in paths:
            if not path.is_file():
                raise ValueError(f"no such report: {path}")
        text = Path(args.summary).read_text(encoding="utf-8", errors="replace") if args.summary else ""
        found = checked(paths, text, args.repeat)
        if found["cases"]:
            named = ", ".join(f"{case} ({where})" if where else case for case, where in found["cases"][:5])
            raise ValueError(f"{len(found['cases'])} failed test runs, so the repeat failed: {named}")
        if found["summary_failed"]:
            raise ValueError(f"the stress summary counts {found['summary_failed']} failed iterations of {found['iterations']}")
        if args.repeat and found["iterations"] and found["iterations"] < args.repeat:
            raise ValueError(f"the summary counts {found['iterations']} iterations, fewer than the {args.repeat} asked for")
        print(f"No failed iteration in {found['iterations'] or 'the'} reported runs.")
    except (ValueError, OSError, ET.ParseError) as error:
        print(f"check-stress-report: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
