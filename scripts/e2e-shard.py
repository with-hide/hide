#!/usr/bin/env python3
"""Deal an end-to-end suite's tests out to CI shards by how long each one takes.

Playwright's own `--shard` cuts the tests in file order into runs of equal
count, and a suite's tests range from 1 second to a minute, so one web shard
held 11 minutes of work while another held 6. `split` reads the tests
Playwright lists (`playwright test --list`) and the suite's own duration table
(`--table`: `web/e2e/shard-durations.json` from the Linux web lane,
`desktop/e2e/shard-durations.json` from the macOS desktop lane), gives each
test its recorded seconds (a test the table does not know gets the table's
median), and fills the shards longest test first, each into the shard with the
least work so far. The same list and table always give the same shards, and
every listed test lands in exactly one shard. The output is a file for
`playwright test --test-list`. With `--budget`, a plan whose heaviest shard
holds more seconds than the lane sized its shards for says so: the suite has
outgrown its shards, and the answer is another shard, not a longer timeout.

`check` compares what Playwright lists for that file with what `split` planned,
because Playwright treats a `--test-list` that matches nothing as a pass.
`durations` writes a table from the job logs of earlier runs: the median
seconds of each test's passing line in the list reporter's output, and 0 for a
test those runs only skipped, for the tests a `--list` output names.
docs/TESTING.md, "Balancing the e2e shards", says when and how to refresh it.

Everything is read and written as UTF-8 (a title holds `›` and other characters
the locale encoding of a Windows runner cannot write), and a problem with the
list, the table or a shard is one line on stderr and exit status 1, never a
traceback.
"""
import argparse
import json
import re
import statistics
import sys
from pathlib import Path

# A shard whose planned work is this much over the mean of the shards is
# reported, not failed: the table is a recording, and a refresh fixes it.
SPREAD_WARNING = 1.15

# `playwright test --list`: "  [project] › file.spec.ts:45:1 › [describe › ]title",
# the project only in a config that names one (the desktop suite's `background`
# and `needs-focus`); a test is in one project, so `file › title` names it. A
# run's list reporter adds the `e2e/` directory and a leading mark, number and
# duration, and a test's tags and retry mark after its title:
# "✓   1 e2e/file.spec.ts:45:1 › title @platform (retry #1) (4.9s)", and a
# skipped test's line has `-` and no duration. A Windows runner's list
# reporter prints `ok` where the others print `✓`.
PROJECT = r"(?:\[[^\]]+\] › )?"
LISTED = re.compile(r"^\s+" + PROJECT + r"(?:e2e/)?(?P<file>[^\s:]+\.spec\.ts):\d+:\d+ › (?P<title>.+?)\s*$")
RUN_LINE = r"\s+\d+\s+" + PROJECT + r"(?:e2e/)?(?P<file>[^\s:]+\.spec\.ts):\d+:\d+ › "
PASSED = re.compile(r"(?:^|\s)(?:✓|ok)" + RUN_LINE + r"(?P<title>.+) \((?P<value>\d+(?:\.\d+)?)(?P<unit>ms|s|m)\)\s*$")
SKIPPED = re.compile(r"(?:^|\s)-" + RUN_LINE + r"(?P<title>.+?)\s*$")
# What a run prints after a title and `--list` does not; a title may end in a
# tag of its own (`... gives it back @needs-focus`), so each is taken off only
# while the title is not yet one the list names.
TRAILING_MARK = re.compile(r"\s+(?:@[\w-]+|\(retry #\d+\))$")
UNITS = {"ms": 0.001, "s": 1.0, "m": 60.0}


class Refused(Exception):
    """A problem with an input, reported as one line."""


def listed_tests(text, allow_none=False):
    """The tests `playwright test --list` printed, as `file › title`, in its order."""
    tests = []
    for line in text.splitlines():
        match = LISTED.match(line)
        if match:
            tests.append(f"{match['file']} › {match['title']}")
    if not tests and not allow_none:
        raise Refused("the list names no test (is it the output of `playwright test --list --reporter=list`?)")
    if len(set(tests)) != len(tests):
        raise Refused("two listed tests share a file and title")
    return tests


def split(tests, table, shards):
    """Shard index (0-based) for each test: longest first into the lightest shard."""
    if shards < 1:
        raise Refused("a run has at least one shard")
    unknown = statistics.median(table.values()) if table else 1.0
    weight = {test: float(table.get(test, unknown)) for test in tests}
    load = [0.0] * shards
    assigned = {}
    for test in sorted(tests, key=lambda test: (-weight[test], test)):
        lightest = min(range(shards), key=lambda index: (load[index], index))
        assigned[test] = lightest
        load[lightest] += weight[test]
    return assigned, load


def shard_tests(tests, table, current, total):
    """The tests of shard `current` (one-based), in the list's order, and every shard's load."""
    assigned, load = split(tests, table, total)
    return [test for test in tests if assigned[test] == current - 1], load


def drift(tests, table, load, name, budget=None):
    """What a reader of the plan should know: tests the table `name` lacks, a heavy shard, a suite past its budget."""
    notes = []
    missing = [test for test in tests if test not in table]
    if missing:
        notes.append(
            f"{len(missing)} of {len(tests)} listed tests are not in {name} and weigh its median "
            f"({', '.join(missing[:3])}{', ...' if len(missing) > 3 else ''}); refresh it"
        )
    mean = sum(load) / len(load)
    if mean and max(load) > mean * SPREAD_WARNING:
        notes.append(f"the heaviest shard plans {max(load):.0f} s against a mean of {mean:.0f} s; refresh {name}")
    if budget is not None and max(load) > budget:
        notes.append(
            f"the heaviest of {len(load)} shards plans {max(load):.0f} s, over the {budget:.0f} s this lane sizes a shard for; "
            "add a shard (docs/TESTING.md, \"Balancing the e2e shards\")"
        )
    return notes


def parse_shard(value):
    match = re.fullmatch(r"(\d+)/(\d+)", value)
    if not match or not 1 <= int(match[1]) <= int(match[2]):
        raise argparse.ArgumentTypeError(f"expected CURRENT/TOTAL, got {value!r}")
    return int(match[1]), int(match[2])


def listed_key(file, title, listed):
    """The listed test a run's line names, or None: the title less the marks a run adds after it."""
    while True:
        key = f"{file} › {title}"
        if key in listed:
            return key
        shorter = TRAILING_MARK.sub("", title)
        if shorter == title:
            return None
        title = shorter


def durations(logs, listed):
    """Median seconds per listed test across the passing lines of the given logs; 0 for one they only skipped."""
    listed = set(listed)
    seen = {}
    skipped = set()
    for text in logs:
        for line in text.splitlines():
            match = PASSED.search(line)
            if match:
                key = listed_key(match["file"], match["title"], listed)
                if key:
                    seen.setdefault(key, []).append(float(match["value"]) * UNITS[match["unit"]])
                continue
            match = SKIPPED.search(line)
            if match:
                key = listed_key(match["file"], match["title"], listed)
                if key:
                    skipped.add(key)
    # A test a lane skips (one that needs a packaged app or a device) costs
    # that lane nothing; weighing the median would plan seconds it never runs.
    return {key: round(statistics.median(seen[key]), 1) if key in seen else 0.0
            for key in sorted(listed) if key in seen or key in skipped}


def read(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise Refused(f"cannot read {path}: {error}")


def read_table(path):
    try:
        table = json.loads(read(path))
    except json.JSONDecodeError as error:
        raise Refused(f"{path} is not JSON: {error}")
    if not isinstance(table, dict) or not all(isinstance(value, (int, float)) for value in table.values()):
        raise Refused(f"{path} must map `file › title` to seconds")
    return table


def emit(text):
    sys.stdout.buffer.write(text.encode("utf-8"))
    sys.stdout.buffer.flush()


def run(arguments):
    if arguments.command == "durations":
        listed = listed_tests(read(arguments.list))
        table = durations((read(path) for path in arguments.logs), listed)
        if not table:
            raise Refused("no passing line of a listed test in the logs")
        lacking = [test for test in listed if test not in table]
        if lacking:
            print(f"{len(lacking)} of {len(listed)} listed tests have no passing or skipped line in the logs and keep the median: "
                  + ", ".join(lacking[:5]), file=sys.stderr)
        emit(json.dumps(table, ensure_ascii=False, indent=2) + "\n")
        return

    current, total = arguments.shard
    tests = listed_tests(read(arguments.list), allow_none=arguments.command == "check")
    table = read_table(arguments.table)
    mine, load = shard_tests(tests, table, current, total)

    if arguments.command == "split":
        # Fewer tests than shards (a narrow `grep`) leaves a shard with none: an
        # empty file, which the caller skips. With at least as many tests as
        # shards every shard gets one, so none is a fault.
        if not mine and len(tests) >= total:
            raise Refused(f"shard {current}/{total} would run no test; {len(tests)} tests listed")
        print(
            f"shard {current}/{total}: {len(mine)} of {len(tests)} tests, "
            f"{load[current - 1]:.0f} s of {sum(load):.0f} s planned; every shard: "
            + ", ".join(f"{seconds:.0f} s" for seconds in load),
            file=sys.stderr,
        )
        if current == 1:
            for note in drift(tests, table, load, arguments.table.name, arguments.budget):
                print(f"::warning::{note}", file=sys.stderr)
        emit("".join(f"{test}\n" for test in mine))
        return

    # check: what Playwright lists for the shard file is what was planned.
    actual = listed_tests(read(arguments.shard_list), allow_none=True)
    missing = [test for test in mine if test not in actual]
    extra = [test for test in actual if test not in mine]
    if missing or extra:
        raise Refused(
            f"shard {current}/{total} would run {len(actual)} tests, not the {len(mine)} planned; "
            f"{len(missing)} planned tests match nothing in the shard file"
            + (f" (first: {missing[0]})" if missing else "")
            + (f"; {len(extra)} unplanned tests are selected (first: {extra[0]})" if extra else "")
        )
    print(f"shard {current}/{total}: Playwright lists the {len(actual)} planned tests", file=sys.stderr)


def main(argv=None):
    sys.stderr.reconfigure(encoding="utf-8", errors="backslashreplace")
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    cut = commands.add_parser("split", help="print the tests of one shard, one per line")
    cut.add_argument("--shard", type=parse_shard, required=True, help="CURRENT/TOTAL, one-based")
    cut.add_argument("--table", type=Path, required=True, help="the suite's own duration table")
    cut.add_argument("--budget", type=float, help="seconds a shard is sized for; a heavier plan warns")
    cut.add_argument("list", type=Path, help="the output of `playwright test --list --reporter=list`")
    verify = commands.add_parser("check", help="fail unless Playwright lists exactly the planned tests for the shard file")
    verify.add_argument("--shard", type=parse_shard, required=True, help="CURRENT/TOTAL, one-based")
    verify.add_argument("--table", type=Path, required=True, help="the table `split` read")
    verify.add_argument("list", type=Path, help="the full list that `split` read")
    verify.add_argument("shard_list", type=Path, help="`playwright test --list --test-list <shard file>` output")
    refresh = commands.add_parser("durations", help="print the duration table of the listed tests from job logs")
    refresh.add_argument("--list", type=Path, required=True, help="the output of `playwright test --list --reporter=list`")
    refresh.add_argument("logs", type=Path, nargs="+")
    try:
        run(parser.parse_args(argv))
    except Refused as error:
        print(f"e2e-shard: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
