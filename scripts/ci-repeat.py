#!/usr/bin/env python3
"""Which tests a pull request adds or changes, and how often the repeat lane runs them.

A new test that fails one run in ten passes its own pull request nine times in
ten, so the `repeat` lane (`.github/workflows/repeat.yml`) runs every test a
pull request adds or changes many times, with no retry: one failed run fails
the lane. docs/TESTING.md, "Flaky tests", owns the policy and the reasons.

What counts as a changed test, by file, because a changed line says nothing
reliable about which test it reaches (a helper, a hook, a loop that declares
tests, a fixture at the top of the file):

* Playwright: a `web/e2e/*.spec.ts` added or modified; every test in the file.
* A unit test file added or modified: a `*.test.ts`, `*.test.tsx`,
  `*.test.mjs` or `*.unit.ts` of the `web`, `desktop` or `mobile` package,
  whose tests the `checks` lane runs under Vitest; every test in the file,
  run with `vitest run --retry=0` once per iteration (Vitest has no
  `--repeat-each`). The package is the file's
  top folder, and a file the package's Vitest config does not collect is not
  selected. A file runs on Linux unless its tests all declare another system
  (`describe.runIf(process.platform === "win32")`), so Linux lists none:
  it then runs on `UNITS_ELSEWHERE`, in a units leg of its own that lists it
  there and chooses its repeat from that count.
* Rust: a module of a crate's `tests/it` binary added or modified (`foo.rs`,
  `foo/mod.rs` or `foo/bar.rs`); every test of that module and its children,
  picked with nextest's `test(/^module::/)`. A deleted file has none left.
  `tests/it/support` and `main.rs`, which every module of the binary shares,
  select the crate's whole `it` binary, and so does a module that lists no
  test (a helper file such as `stand_ins.rs`): what a helper reaches is not
  knowable from the diff, and the list decides, once, in `rust_plan`.
* Rust under `src`: a file the crate's `lib`, `main` or `bin` target builds
  that declares tests. Its tests are selected by the module names it declares
  them in: a file only a test build compiles (`#[cfg(test)] mod clear_tests;`)
  by its own module (`mobile::clear_tests`), and a product file by the
  test-gated module blocks it holds (`#[cfg(test)] mod tests { ... }` gives
  `runtime::deepest_holder_tests`). `scripts/rust_scan.py`, "module_tree",
  reads the declarations the way rustc does; a file whose tests sit nowhere
  it can name is not selected, because the diff cannot say what they are.

Two budgets, because a web test costs seconds and a unit or Rust test does not:

* Web specs: `WEB_BUDGET_SECONDS` recorded test-seconds for the pull request,
  read from the Linux suite's duration table (`WEB_TABLE`, the one
  `scripts/e2e-shard.py` deals the web shards by; a test it does not know
  weighs the table's median, as there). One pass is the summed seconds of the
  tests `playwright test --list` names, and each test runs
  `clamp(floor(WEB_BUDGET_SECONDS / one pass), MIN_REPEAT, MAX_REPEAT)` times.
  The specs run on as many shards of `web-e2e.yml` as keep each one at
  `WEB_SHARD_SECONDS` of planned test time or less, at most `WEB_MAX_SHARDS`,
  dealt by `e2e-shard.py split`; when even `MIN_REPEAT` passes exceed the
  budget they run `MIN_REPEAT` times anyway, on at most
  `WEB_OVER_BUDGET_MAX_SHARDS` shards (`web_plan`).
* Unit test files and Rust modules: `BUDGET` test runs. Each test runs
  `repeats(N)` times, N being the unit and Rust tests that will run, counted by
  the tools that run them (`vitest list`, `cargo nextest list`):
  `clamp(floor(BUDGET / N), MIN_REPEAT, MAX_REPEAT)`. A pull request that
  changes more than `BUDGET / MIN_REPEAT` of them runs over the budget,
  because a repeat below three shows too little.

The count job's listing builds the changed crates' tests once; it packs each
target it listed into a nextest archive with the same cargo arguments
(`rust_archive`), so cargo only packs what it built, and the Rust leg runs
that archive rather than building the tests again. A hand run names its own
repeat, counts nothing and runs on one web shard, its Rust leg building and its
units leg listing on the system it asked for.

`select` is what `scripts/ci-plan.py` calls; the subcommands are what
`repeat.yml` and `web-e2e.yml` call.
"""
import argparse
import importlib.util
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
_RUST_SCAN = importlib.util.spec_from_file_location("rust_scan", Path(__file__).resolve().parent / "rust_scan.py")
rust_scan = importlib.util.module_from_spec(_RUST_SCAN)
_RUST_SCAN.loader.exec_module(rust_scan)
_E2E_SHARD = importlib.util.spec_from_file_location("e2e_shard", Path(__file__).resolve().parent / "e2e-shard.py")
e2e_shard = importlib.util.module_from_spec(_E2E_SHARD)
_E2E_SHARD.loader.exec_module(e2e_shard)

BUDGET = 200
# The web specs' budget, in the recorded seconds of one test run each.
WEB_BUDGET_SECONDS = 2400
WEB_SHARD_SECONDS = 600
WEB_MAX_SHARDS = 4
WEB_OVER_BUDGET_MAX_SHARDS = 6
# The Linux suite's table, which a pull request's web repeat runs against.
WEB_TABLE = "web/e2e/shard-durations.json"
MIN_REPEAT = 3
MAX_REPEAT = 10
# A hand run names its own repeat; this is the most it may ask for, so a typo
# cannot hold a runner for a day.
DISPATCH_MAX_REPEAT = 100

WEB_SPEC = re.compile(r"^web/e2e/[^/]+\.spec\.ts$")
# A crate's `tests/it` binary: the crate directory, then the module's file.
RUST_MODULE = re.compile(r"^(?P<crate>.+)/tests/it/(?P<module>.+)\.rs$")
# A file a crate's `src` builds, and the target whose tests its test
# declarations belong to.
RUST_SOURCE = re.compile(r"^(?P<crate>[^/]+)/src/(?P<file>.+)\.rs$")
# Helpers every module of the binary shares, and the file that declares the modules.
RUST_SHARED = ("support", "main")
# A module name for "every test of a Rust target".
WHOLE = "*"

# The packages whose unit tests a lane runs, and the Vitest config that says
# which files each package collects (`web/vite.config.ts`,
# `desktop/vitest.config.ts`, `mobile`'s default).
UNIT_PACKAGES = ("web", "desktop", "mobile")
UNIT_TEST = re.compile(r"^(?P<package>web|desktop|mobile)/(?P<file>.+\.(?:test\.tsx?|test\.mjs|unit\.ts))$")

PACKAGE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
MODULE = re.compile(r"^(?:\*|[a-z0-9_]+(?:::[a-z0-9_]+)*)$")
SPEC_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.spec\.ts$")

RUNNERS = {"linux": "ubuntu-latest", "macos": "macos-15", "windows": "windows-latest"}
# Where a pull request repeats a unit test file that runs no test on Linux: a
# file whose tests all declare another system (Vitest's
# `describe.runIf(process.platform === "win32")`) is skipped by Linux `checks`, and
# Windows is the one other system whose `windows check` runs the packages'
# Vitest suites in a pull request.
UNITS_ELSEWHERE = "windows"
REPORT_BYTES = 16 * 1024 * 1024
REPORT_FILES = 128


def count_value(value):
    """A required nonnegative count; a missing output is never zero."""
    if not re.fullmatch(r"[0-9]+", str(value)):
        raise ValueError(f"missing or invalid test count: {value!r}")
    return int(value)


def choose(event, web, rust, named, planned, rust_count, units=None, units_count="", units_elsewhere=None, runner="linux"):
    """One explicit run decision for every leg of the lane.

    The legs are the Playwright specs (`web`), the Rust tests (`rust`) and the
    packages' Vitest unit test files (`units`), one units leg per system. A
    hand run names its own repeat, counts nothing and runs its specs on one
    shard and its unit files on the system it asked for; a pull request runs
    `count` first and this decides from what the runners listed: the web
    specs from `planned`, what `web_plan` made of their seconds budget, and
    the unit and Rust tests from their count (`repeat`, the run-count budget
    they share). A pull request's unit files that list no test on Linux
    (`units_elsewhere`) are their own leg on `UNITS_ELSEWHERE`, which lists
    them and chooses their repeat from that count (`repeat` 0 in the leg),
    since Linux cannot count them.
    """
    units = [] if units is None else units
    units_elsewhere = [] if units_elsewhere is None else units_elsewhere
    selected = {"web": bool(web), "rust": bool(rust), "units": bool(units)}
    if event == "workflow_dispatch":
        named = count_value(named)
        if not 1 <= named <= DISPATCH_MAX_REPEAT or not any(selected.values()):
            raise ValueError("a hand run needs named tests and a repeat from 1 to 100")
        web_times, web_shards, web_parts = (named, 1, 1) if selected["web"] else (0, 0, 0)
        times = named if selected["rust"] or selected["units"] else 0
        legs = [{"os": runner, "runner": RUNNERS[runner], "files": units, "repeat": named}] if units else []
    else:
        if selected["web"] and not planned:
            raise ValueError("selected web files listed no tests")
        web_times, web_shards, web_parts = ((planned["repeat"], planned["shards"], planned["parts"])
                                            if selected["web"] else (0, 0, 0))
        if not set(units_elsewhere) <= set(units):
            raise ValueError(f"files to repeat elsewhere that were not selected: {sorted(set(units_elsewhere) - set(units))}")
        linux = [path for path in units if path not in units_elsewhere]
        counts = {"rust": count_value(rust_count) if selected["rust"] else 0,
                  "units": count_value(units_count) if linux else 0}
        for kind, chosen in (("rust", selected["rust"]), ("units", bool(linux))):
            if chosen and counts[kind] == 0:
                raise ValueError(f"selected {kind} files listed no tests")
        times = repeats(sum(counts.values()))
        legs = ([{"os": "linux", "runner": RUNNERS["linux"], "files": linux, "repeat": times}] if linux else []) + (
            [{"os": UNITS_ELSEWHERE, "runner": RUNNERS[UNITS_ELSEWHERE], "files": sorted(units_elsewhere), "repeat": 0}]
            if units_elsewhere else [])
    return {
        "run-web": str(selected["web"]).lower(),
        "run-rust": str(selected["rust"]).lower(),
        "run-units": str(selected["units"]).lower(),
        "repeat": times,
        "web-repeat": web_times,
        "web-shards": web_shards,
        "web-parts": web_parts,
        "units-legs": json.dumps(legs, separators=(",", ":")),
    }


def web_plan(listing, table):
    """How a pull request's changed web specs repeat, and on how many shards.

    `listing` is `playwright test --list --reporter=list` of the specs and
    `table` the Linux suite's duration table. Each test weighs what
    `e2e-shard.py` weighs it, so a test the table lacks counts at the table's
    median there and here.

    A shard runs one group of the tests, dealt by `e2e-shard.py split` as
    `web-e2e.yml` deals them, for one part of the repeat: `groups` x `parts`
    shards, so one test heavier than a shard's share runs its repeats on
    several runners. The plan is the fewest shards whose heaviest plans
    `WEB_SHARD_SECONDS` or less, cutting the repeat only where dealing the
    tests alone needs more shards; when no count within the cap gets there, it
    is the one whose heaviest is lightest.
    """
    try:
        tests = e2e_shard.listed_tests(listing)
    except e2e_shard.Refused as error:
        raise ValueError(f"the web listing: {error}")
    total = listed_tests(listing)
    if len(tests) != total:
        raise ValueError(f"the Playwright list totals {total} tests but names {len(tests)}")
    one_pass = sum(e2e_shard.weights(tests, table).values())
    if one_pass > 0:
        times = max(MIN_REPEAT, min(MAX_REPEAT, math.floor(WEB_BUDGET_SECONDS / one_pass)))
    else:
        # Tests the Linux lane only skips cost it nothing.
        times = MAX_REPEAT
    most = WEB_MAX_SHARDS if one_pass * times <= WEB_BUDGET_SECONDS else WEB_OVER_BUDGET_MAX_SHARDS
    candidates = []
    # No more groups than tests and no more parts than repeats: a shard with
    # no test or no run would prove nothing.
    for groups in range(1, min(most, len(tests)) + 1):
        heaviest_pass = max(e2e_shard.split(tests, table, groups)[1])
        for parts in range(1, min(most // groups, times) + 1):
            heaviest = heaviest_pass * math.ceil(times / parts)
            fits = heaviest <= WEB_SHARD_SECONDS
            shards = groups * parts
            candidates.append(((0, shards, parts, heaviest) if fits else (1, heaviest, shards, parts), groups, parts, heaviest))
    _, groups, parts, heaviest = min(candidates)
    return {"tests": len(tests), "seconds": round(one_pass, 1), "repeat": times, "shards": groups * parts,
            "parts": parts, "heaviest": round(heaviest, 1)}


def shard_share(shard, shards, parts, times):
    """What one shard of a web repeat runs: `(group, groups, repeat)`.

    The shards are `groups` x `parts`; shard `shard` (one-based) runs test
    group `(shard - 1) % groups + 1` of `groups` for its part of the `times`
    repeats, the first `times % parts` parts taking one more, so every group
    runs `times` times across its parts.
    """
    if not (1 <= parts <= shards and shards % parts == 0 and 1 <= shard <= shards and parts <= times):
        raise ValueError(f"no share of a repeat of {times} in {parts} parts for shard {shard}/{shards}")
    groups = shards // parts
    part = (shard - 1) // groups
    return (shard - 1) % groups + 1, groups, times // parts + (part < times % parts)


def total_completed(paths, shards):
    """The completed runs of a sharded web repeat: one count per shard, each read from that shard's own report."""
    if len(paths) != shards:
        raise ValueError(f"{len(paths)} shard counts for a repeat on {shards} shards")
    counts = [count_value(Path(path).read_text(encoding="utf-8").strip()) for path in paths]
    if 0 in counts:
        raise ValueError("a repeat shard completed zero tests")
    return sum(counts)


def verdict(needs):
    """A selected repeat leg must succeed and report completed test runs, each units leg its own."""
    if needs["count"]["result"] != "success":
        raise ValueError("choosing the repeat did not succeed")
    outputs = needs["count"]["outputs"]
    # The web specs repeat by their seconds budget, the Rust tests and the
    # counted units leg by their shared count.
    times = {"web": count_value(outputs["web-repeat"])}
    times["rust"] = count_value(outputs["repeat"])
    legs = json.loads(outputs["units-legs"])
    for kind in ("web", "rust", "units"):
        selected = outputs[f"run-{kind}"]
        if selected not in ("true", "false"):
            raise ValueError(f"missing or invalid run-{kind} decision: {selected!r}")
        job = needs[kind]
        expected = "success" if selected == "true" else "skipped"
        if job["result"] != expected:
            raise ValueError(f"{kind}: expected {expected}, got {job['result']}")
        if selected == "true" and kind != "units":
            completed = count_value(job["outputs"].get("completed", ""))
            if times[kind] == 0 or completed == 0 or completed % times[kind]:
                raise ValueError(f"{kind}: {completed} completed test runs do not prove {times[kind]} repeats")
    if (outputs["run-units"] == "true") != bool(legs):
        raise ValueError("the units legs and run decision disagree")
    # Each units leg reports its own completed runs and the repeat it ran: the
    # one the count chose, or, for a leg the count could not list, the one it
    # chose from its own listing (`repeat` 0).
    for leg in legs:
        system = leg["os"]
        completed = count_value(needs["units"]["outputs"].get(f"completed-{system}", ""))
        ran = count_value(needs["units"]["outputs"].get(f"repeat-{system}", ""))
        if leg["repeat"] and ran != leg["repeat"]:
            raise ValueError(f"units on {system}: ran {ran} repeats, not the {leg['repeat']} chosen")
        if ran == 0 or completed == 0 or completed % ran:
            raise ValueError(f"units on {system}: {completed} completed test runs do not prove {ran} repeats")
    if (times["web"] > 0) != (outputs["run-web"] == "true"):
        raise ValueError("the web repeat and run decisions disagree")
    if (times["rust"] > 0) != (outputs["run-rust"] == "true" or any(leg["repeat"] for leg in legs)):
        raise ValueError("the repeat and run decisions disagree")


def completed_runs(playwright, junit):
    """Read actual passes, excluding skips, from the runners' own reports.

    Nextest emits a suite per stress iteration, so each testcase is one run:
    https://nexte.st/docs/features/stress-tests/#junit-output
    """
    paths = [Path(playwright)] if playwright else [Path(path) for path in junit]
    if not paths or len(paths) > REPORT_FILES or sum(path.stat().st_size for path in paths) > REPORT_BYTES:
        raise ValueError("test reports are missing or exceed 128 files / 16 MiB")
    if playwright:
        report = json.loads(paths[0].read_text(encoding="utf-8"))
        if report.get("errors"):
            raise ValueError("the Playwright report contains run errors")

        def passed(suite):
            return sum(
                result["status"] == "passed"
                for spec in suite.get("specs", [])
                for test in spec["tests"] for result in test["results"]
            ) + sum(passed(child) for child in suite.get("suites", []))

        completed = passed(report)
    else:
        completed = 0
        for path in paths:
            report = ET.parse(path).getroot()
            if report.tag != "testsuites":
                raise ValueError(f"not a nextest JUnit report: {path}")
            completed += sum(
                not any(child.tag in ("skipped", "failure", "error") for child in case)
                for case in report.iter("testcase")
            )
    if completed == 0:
        raise ValueError("the repeat job completed zero tests")
    return completed


def repeats(total):
    """How many times each of `total` changed tests runs; 0 when none changed."""
    if total < 0:
        raise ValueError(f"a negative test count: {total}")
    if total == 0:
        return 0
    return max(MIN_REPEAT, min(MAX_REPEAT, BUDGET // total))


def rust_module(path, crate_dirs):
    """The `{package, module}` a `tests/it` file selects, or None for a file that is not in a crate's `it` binary."""
    match = RUST_MODULE.match(path)
    if not match:
        return None
    package = crate_dirs.get(match["crate"])
    if package is None:
        return None
    parts = match["module"].split("/")
    if parts[0] in RUST_SHARED:
        return {"package": package, "module": WHOLE}
    if parts[-1] == "mod":
        parts = parts[:-1]
    return {"package": package, "module": "::".join(parts)}


def changed_source(path, package, targets):
    """The `{package, test, module}` list one changed crate `src` file selects.

    A file the crate's `lib`, `main` or `bin` target builds is selected by the
    modules it declares its tests in: its own module when only a test build
    compiles it, and each test-gated module block when a product file holds
    them. Empty for a file that is not in a crate's `src`, and for one whose
    tests the walk cannot name. `targets` is the crate's source targets as
    `(test, path)` pairs, the way `cargo metadata` names them.
    """
    if not package or not RUST_SOURCE.match(path):
        return []
    entry = module_tree(targets).get((ROOT / path).resolve())
    if entry is None:
        return []
    modules = [entry["module"]] if entry["test_only"] else entry["inline_tests"]
    return [
        {"package": package, "test": entry["target"], "module": module}
        for module in modules if module
    ]


_TREES = {}


def module_tree(targets):
    """One crate's module tree, walked once: `select` asks per changed path,
    and a walk reads every file of the crate's targets."""
    key = tuple(targets)
    if key not in _TREES:
        _TREES[key] = rust_scan.module_tree(ROOT, targets)
    return _TREES[key]


def select(entries, crate_dirs, targets=None):
    """The changed tests of `git diff --name-status` entries.

    Playwright specs (`web`), Vitest unit test files (`units`), the modules of
    a crate's `tests/it` binary (`rust`), and the test modules a crate's `src`
    file declares (`rust_sources`). `targets` maps each crate directory to the
    source targets `cargo metadata` reports; without it, a caller that has no
    crate graph gets the `tests/it` modules and the unit test files only.
    """
    web = sorted({name for status, name in entries if status in ("A", "M") and WEB_SPEC.match(name)})
    units = sorted({name for status, name in entries if status in ("A", "M") and UNIT_TEST.match(name)})
    modules = set()
    sources = set()
    for status, name in entries:
        if status not in ("A", "M"):
            continue
        found = rust_module(name, crate_dirs)
        if found:
            modules.add((found["package"], found["module"]))
        crate = name.partition("/")[0]
        if targets and crate in targets:
            for source in changed_source(name, crate_dirs[crate], targets[crate]):
                sources.add((source["package"], source["test"], source["module"]))
    # The whole binary runs every module, and `foo` runs `foo::bar`, so neither is run twice.
    wholes = {package for package, module in modules if module == WHOLE}
    rust = [
        {"package": package, "module": module} for package, module in sorted(modules)
        if package not in wholes or module == WHOLE
        if not any(module.startswith(other + "::") for other_package, other in modules if other_package == package)
    ]
    # A target's whole binary covers silence, and a module covers its children.
    source_wholes = {(package, test) for package, test, module in sources if module == WHOLE}
    rust_sources = [
        {"package": package, "test": test, "module": module} for package, test, module in sorted(sources)
        if (package, test) not in source_wholes or module == WHOLE
        if not any(module.startswith(other + "::") for other_package, other_test, other in sources
                   if (other_package, other_test) == (package, test))
    ]
    return {"web": web, "units": units, "rust": rust, "rust_sources": rust_sources}


def nextest_listed(text):
    """The tests `cargo nextest list --message-format json` says the filterset runs.

    A test counts when its filter matched. A listing made without
    `--run-ignored` names the ignored tests too and marks them, so they count
    only when the run asks for them (`nextest_counted`, which the plan uses).
    """
    return nextest_counted(text)[0]


def nextest_counted(text):
    """`(tests, ignored)`: what a `cargo nextest list --message-format json` would run.

    A test counts when its filter matched; `ignored` counts those of them
    nextest marks ignored, which a listing without `--run-ignored` includes
    and the run then skips.
    """
    suites = json.loads(text).get("rust-suites")
    if not isinstance(suites, dict):
        raise ValueError("the nextest list has no rust-suites")
    matched = [
        case
        for suite in suites.values()
        for case in suite.get("testcases", {}).values()
        if case.get("filter-match", {}).get("status") == "matches"
    ]
    return len(matched), sum(1 for case in matched if case.get("ignored"))


def listed_tests(text):
    """The test count of `playwright test --list` output, from its closing `Total:` line."""
    found = re.findall(r"^Total: (\d+) tests? in \d+ files?$", text, re.MULTILINE)
    if len(found) != 1:
        raise ValueError("the Playwright list has no single `Total: N tests in M files` line")
    return int(found[0])


def spec_path(name):
    """A hand-run spec name (`s3.spec.ts` or `web/e2e/s3.spec.ts`) as the path of a spec that exists."""
    name = name.strip()
    path = name if name.startswith("web/e2e/") else f"web/e2e/{name}"
    if not WEB_SPEC.match(path) or not SPEC_NAME.match(PurePosixPath(path).name):
        raise ValueError(f"not a web e2e spec: {name!r} (expected a name like s3.spec.ts)")
    if not (ROOT / path).is_file():
        raise ValueError(f"no such spec: {path}")
    return path


def spec_pattern(path):
    """The Playwright file filter that matches exactly this spec: filters are regular expressions on the full path."""
    return re.escape(path) + "$"


def unit_path(name):
    """A hand-run unit test name as the path of a file the package's Vitest
    config collects: its path from the repository root, or a name a search of
    the three packages finds once."""
    name = name.strip()
    path = name if UNIT_TEST.match(name) else ""
    if not path:
        # A name alone is found anywhere below one of the three packages, and
        # only when exactly one file has it: two files of one name are two
        # different tests.
        found = sorted(
            candidate.relative_to(ROOT).as_posix()
            for package in UNIT_PACKAGES
            for candidate in (ROOT / package).rglob(name)
            if candidate.is_file() and "node_modules" not in candidate.parts
        )
        if len(found) != 1:
            raise ValueError(f"not a unit test file: {name!r} (expected a path like web/src/searchDetail.test.ts)")
        path = found[0]
    if not UNIT_TEST.match(path) or not (ROOT / path).is_file():
        raise ValueError(f"no such unit test: {path}")
    return path


def units_plan(units, lister, system, elsewhere=None):
    """What a units leg runs on `system`, from the tests each file lists there.

    `lister(package, files)` answers, for each file, how many tests Vitest
    would run on this runner; a test a file declares for another system
    (`describe.runIf(process.platform === "win32")`) is not among them. Answers
    `(plan, moved)`: the plan of the files that list tests here, and the files
    that list none, which go to the `elsewhere` system when one is given (a
    pull request's count on Linux) and fail otherwise, so N is what runs and
    never an estimate, and a file runs where its tests do.
    """
    plan, moved = [], []
    for package in sorted({UNIT_TEST.match(path)["package"] for path in units}):
        files = sorted(path for path in units if UNIT_TEST.match(path)["package"] == package)
        counts = lister(package, files)
        listed = [path for path in files if counts[path]]
        moved += [path for path in files if not counts[path]]
        if listed:
            plan.append({"package": package, "files": listed, "tests": sum(counts[path] for path in listed)})
    if moved and not elsewhere:
        raise ValueError(f"these files list no test on {system}: {', '.join(moved)}")
    return plan, moved


def units_lister(package, files):
    """How many tests `vitest list` says the package would run of each file.

    The file filters are the ones the package's own Vitest matches, so they
    are spelled from the package's folder: `desktop/src/main/env.test.ts` is
    `src/main/env.test.ts` to a run in `desktop/`, and a path from the
    repository root matches nothing there (measured: it listed 0 tests, which
    would have made every units run fail its count). A filter is a substring,
    so the listing can hold another file's tests; each test is counted for the
    file Vitest names, and only the selected files are answered.
    """
    relative = [PurePosixPath(path).relative_to(package).as_posix() for path in files]
    with tempfile.TemporaryDirectory() as directory:
        listed = Path(directory) / "listed.json"
        # Looked up first, because on Windows `pnpm` is a `.cmd` that
        # `subprocess` finds only by its full name.
        pnpm = shutil.which("pnpm")
        if not pnpm:
            raise ValueError("pnpm is not on PATH, so no unit test can be listed")
        command = [pnpm, "--dir", package, "exec", "vitest", "list", *relative, f"--json={listed}"]
        subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True)
        tests = json.loads(listed.read_text(encoding="utf-8"))
    return listed_per_file(tests, files)


def listed_per_file(tests, files):
    """The tests of `vitest list --json` counted for each of `files`.

    Vitest names a test's file by its absolute path, with `/` between names
    on every system; the count is keyed by the path from the repository root.
    """
    root = ROOT.resolve()
    counts = {path: 0 for path in files}
    for test in tests:
        path = Path(test["file"]).resolve()
        if path.is_relative_to(root) and path.relative_to(root).as_posix() in counts:
            counts[path.relative_to(root).as_posix()] += 1
    return counts


def rust_entry(item):
    """A hand-run `package:test:module` as the `{package, test, module}` it names.

    `test` is the target the tests are built into: `it` for the crate's
    `tests/it` binary, `lib` for its library, and `bin` for the programs it
    ships. `package:*` and `package:lib:*` name a whole target. The module is
    refused unless the crate's source declares a file or a test-gated module
    block of that name, the way `scripts/rust_scan.py` reads them.
    """
    package, separator, rest = item.partition(":")
    test, separator2, module = rest.partition(":")
    if not separator2 or not PACKAGE.match(package) or not MODULE.match(module) or test not in ("it", "lib", "bin"):
        raise ValueError(f"not a Rust test module: {item!r} (expected package:test:module, like hided:lib:mobile::clear_tests)")
    if test == "it":
        binary = ROOT / package / "tests/it"
        if not binary.is_dir():
            raise ValueError(f"no such test binary: {package}/tests/it")
        parts = module.split("::")
        if module != WHOLE and not ((binary.joinpath(*parts).with_suffix(".rs")).is_file() or (binary.joinpath(*parts) / "mod.rs").is_file()):
            raise ValueError(f"no such module: {package}/tests/it/{'/'.join(parts)}.rs")
        return {"package": package, "test": test, "module": module}
    declarations = declared_modules(package)
    if module != WHOLE and module not in declarations:
        raise ValueError(f"no such module: {package} {test} {module} (one of {', '.join(sorted(declarations)[:5])})")
    return {"package": package, "test": test, "module": module}


def declared_modules(package):
    """Every module name one crate's `src` files and test-gated blocks declare."""
    tree = module_tree(crate_targets(package))
    return {entry["module"] for entry in tree.values() if entry["module"]} | {
        inline for entry in tree.values() for inline in entry["inline_tests"]
    }


def crate_targets(package):
    """The `(target, path)` pairs `cargo metadata` reports for one crate's `src`."""
    metadata = subprocess.run(
        [bash(), "scripts/verify-cargo.sh", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    for entry in json.loads(metadata.stdout)["packages"]:
        if entry["name"] == package:
            return [
                ("lib" if "lib" in target["kind"] else "bin", target["src_path"])
                for target in entry["targets"]
                if "lib" in target["kind"] or "bin" in target["kind"]
            ]
    return []


def nextest_filters(modules):
    """One `(package, test, filterset)` per target: its modules' tests.

    The filterset is empty for a whole target.
    """
    grouped = {}
    for module in modules:
        if (not PACKAGE.match(module["package"]) or not MODULE.match(module["module"])
                or module.get("test", "it") not in ("it", "lib", "bin")):
            raise ValueError(f"not a Rust test module: {module!r}")
        grouped.setdefault((module["package"], module.get("test", "it")), []).append(module["module"])
    return [
        (package, test, "" if WHOLE in names else "test(/^(" + "|".join(sorted(names)) + ")::/)")
        for (package, test), names in sorted(grouped.items())
    ]


def rust_plan(modules, lister):
    """What the Rust job runs, from the tests the built target lists.

    `lister(package, test, filterset, run_ignored)` answers `(tests, ignored)`
    for what nextest would run with `--run-ignored run_ignored`: how many, and
    how many of those are marked ignored. A target with a selected module that
    lists none (a helper file lists no test of its own, and what it reaches is
    unknown) runs whole instead, whatever else was selected beside it, so N is
    always what runs and never an estimate.

    A pull request's plan asks for no ignored test. One is marked so because
    it needs what an ordinary run does not prepare (an sshd, an agent's
    program, Windows OpenSSH, a retained older build, or the call of a
    subprocess whose entry point it is), and this lane prepares only the pane
    holder and shells, so asking for them turned an ordinary change red. A
    module of ignored tests alone lists none here and runs its target whole,
    as a helper does, with its ordinary tests; a hand run that names it asks
    for its ignored tests (`plan_run_ignored`). `run_ignored` is always empty.
    """
    plan = []
    for package, test, expression in nextest_filters(modules):
        names = sorted(module["module"] for module in modules
                       if module["package"] == package and module.get("test", "it") == test)
        # A module that lists no ordinary test of its own runs its target whole.
        if expression and any(not lister(package, test, "test(/^(" + name + ")::/)", "")[0] for name in names):
            expression = ""
        plan.append({
            "package": package, "test": test, "filter": expression, "tests": lister(package, test, expression, "")[0],
            "run_ignored": "",
        })
    return plan


def plan_run_ignored(plan, lister=None):
    """Each plan entry with the flag its tests need, asked on this runner.

    A pull request's plan is counted in the `count` job, which lists on a
    runner of every system; a hand run names modules and nothing lists them,
    because the system asked for may not be the one the count job runs on. The
    Rust job is on the system that runs them, so it asks nextest here:
    `ignored-only` where a listing without the flag matches no test and one
    asking for ignored tests matches some (a module someone named to repeat
    its ignored tests, `engine::contract`), and empty otherwise. An entry with
    ordinary tests runs those only, as `rust_plan` says why.
    """
    lister = lister or nextest_lister
    decided = []
    for entry in plan:
        package, test, expression = entry["package"], entry["test"], entry["filter"]
        # A pull request's plan carries the flag the count job decided, empty
        # included; only a hand run's entry, which has none yet, is asked.
        if "run_ignored" in entry:
            decided.append(entry)
            continue
        run_ignored = ""
        if not lister(package, test, expression, "")[0] and lister(package, test, expression, "ignored-only")[0]:
            run_ignored = "ignored-only"
        decided.append({**entry, "run_ignored": run_ignored})
    return decided


def bash():
    """The bash on PATH, and on Windows the Git Bash beside the checkout's own tools.

    A bare `bash` in `subprocess` finds `C:\\Windows\\System32\\bash.exe` first,
    which is the WSL launcher: with no distribution installed it prints that and
    exits 1, so every listing this lane makes failed on Windows while its own
    steps, run by Git Bash, went on (run 43661175346). Looking the program up
    here and handing `subprocess` its path skips that directory.
    """
    program = shutil.which("bash")
    if not program:
        raise ValueError("bash is not on PATH, so nothing can be listed or run")
    return program


def nextest_lister(package, test, expression, run_ignored=""):
    """What `cargo nextest list` says one target of the package would run.

    Answers `(tests, ignored)`: how many whose filter matched, and how many of
    those are marked ignored. `test` is `it` for the crate's `tests/it` binary,
    and `lib` or `bin` for the library and programs, which nextest selects by
    its own target names. `run_ignored` is nextest's own `default`, `all` or
    `ignored-only`; only the last two are passed, so the ordinary listing
    stays the one nextest makes by itself.
    """
    command = [bash(), "scripts/verify-cargo.sh", "nextest-list", "--profile", "ci", *nextest_target(package, test)]
    if expression:
        command += ["-E", expression]
    if run_ignored:
        command += ["--run-ignored", run_ignored]
    command += ["--message-format", "json"]
    listed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    # A listing that fails is the lane's whole plan failing, and its reason is
    # in the command's own output: the exit status alone named none of it (run
    # 43659916395, where a Windows repeat stopped here). Both streams are
    # shown, cut, because which one a failed `cargo` writes to is not fixed.
    if listed.returncode != 0:
        tail = lambda text: (text or "").strip()[-4000:]
        raise ValueError(
            f"listing what {package}'s {test} target would run failed with exit "
            f"{listed.returncode} ({subprocess.list2cmdline(command)}):\n"
            f"stdout:\n{tail(listed.stdout)}\nstderr:\n{tail(listed.stderr)}"
        )
    return nextest_counted(listed.stdout)


def nextest_target(package, test):
    """The cargo arguments that pick one target of a package: `it` is the
    crate's `tests/it` binary, and `lib` and `bin` its library and programs.

    The listing and the archive both use these, so the archive asks cargo for
    exactly the tests the listing built and cargo only packs them.
    """
    return ["-p", package, *(["--test", "it"] if test == "it" else ["--bins"] if test == "bin" else ["--lib"])]


def archive_name(entry):
    """The file one Rust plan entry's tests are packed into."""
    return f"{entry['package']}-{entry['test']}.tar.zst"


def plan_entry(entry):
    """A Rust plan entry the jobs may read, or a refusal."""
    if (not PACKAGE.match(entry["package"]) or entry.get("test") not in ("it", "lib", "bin")
            or not re.fullmatch(r"(?:test\(/\^\([a-z0-9_:|]+\)::/\))?", entry["filter"])
            or entry.get("run_ignored", "") not in ("", "ignored-only")):
        raise ValueError(f"not a Rust plan entry: {entry!r}")
    return entry


def rust_archive(plan, directory, packer):
    """Pack each target of a pull request's plan into its own nextest archive.

    `packer(path, arguments)` runs `cargo nextest archive` with the cargo
    arguments the listing used, so it finds the listed tests built. One archive
    per target, because an archive run takes no `-p` or target flag: the
    archive holds that target's tests and nothing else, and the filterset picks
    the modules. A plan asks for no ignored test (`rust_plan`); one that did
    would need the holder built, which the Rust leg cannot do without a
    compiler, so it is refused here.
    """
    for entry in plan:
        plan_entry(entry)
        if entry.get("run_ignored"):
            raise ValueError(f"a pull request's plan asks for no ignored test: {entry!r}")
        packer(Path(directory) / archive_name(entry), nextest_target(entry["package"], entry["test"]))


def nextest_packer(path, arguments):
    """`cargo nextest archive` into `path`, its output in the job's log."""
    path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([bash(), "scripts/verify-cargo.sh", "nextest-archive", str(path), *arguments], cwd=ROOT, check=True)


def resolve(event, web, rust, repeat, runner, units=""):
    """The lane's work from what called it.

    A pull request's call hands over the lists `ci-plan.py` selected (JSON);
    a hand run names specs, unit test files and `package:test:module` tests in
    plain words and the repeat it wants, which is validated here and never
    counted.
    """
    if event == "workflow_dispatch":
        if runner not in RUNNERS:
            raise ValueError(f"unknown runner {runner!r}; expected one of {', '.join(RUNNERS)}")
        if not re.fullmatch(r"[0-9]+", repeat) or not 1 <= int(repeat) <= DISPATCH_MAX_REPEAT:
            raise ValueError(f"repeat must be a number from 1 to {DISPATCH_MAX_REPEAT}, not {repeat!r}")
        specs = sorted({spec_path(name) for name in web.split()})
        unit_files = sorted({unit_path(name) for name in units.split()})
        modules = sorted((rust_entry(item) for item in rust.split()),
                         key=lambda found: (found["package"], found["test"], found["module"]))
        if not specs and not modules and not unit_files:
            raise ValueError("name at least one spec (web), one unit test file (units) or one Rust test module (rust)")
        # Not listed: the system asked for may not be the one this job runs on, so
        # a module that lists no test there fails in the Rust job, where it runs,
        # and whether it asks for ignored tests is left for that job to decide
        # (no `run_ignored` yet; `plan_run_ignored`).
        plan = [{"package": package, "test": test, "filter": expression}
                for package, test, expression in nextest_filters(modules)]
        # The unit files are listed by the units leg, on the system asked for:
        # a file's tests may declare that system, and Linux would list none.
        return {"web": specs, "units": unit_files, "rust": modules, "runner": runner,
                "repeat": int(repeat), "counted": False, "units_counted": False, "plan": plan}
    specs = json.loads(web or "[]")
    unit_files = json.loads(units or "[]")
    modules = json.loads(rust or "[]")
    for path in specs:
        if not WEB_SPEC.match(path):
            raise ValueError(f"not a web e2e spec: {path!r}")
    for path in unit_files:
        if not UNIT_TEST.match(path):
            raise ValueError(f"not a unit test file: {path!r}")
    for module in modules:
        if (not PACKAGE.match(module["package"]) or not MODULE.match(module["module"])
                or module.get("test", "it") not in ("it", "lib", "bin")):
            raise ValueError(f"not a Rust test module: {module!r}")
    # A pull request runs on Linux only; the hand run is the way to another system.
    return {"web": specs, "units": unit_files, "rust": modules, "runner": "linux", "repeat": 0,
            "counted": bool(specs or modules or unit_files), "units_counted": bool(unit_files), "plan": []}


def write_outputs(values):
    line = "".join(f"{key}={value}\n" for key, value in values.items())
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(line)
    sys.stdout.write(line)


# What separates the fields of a Rust plan line. A shell's `read` folds a run
# of whitespace separators into one, so an empty filterset between two tabs
# moved the ignored flag into its place; this separator is not whitespace.
UNIT_SEPARATOR = "\x1f"


def write_lines(lines):
    """Lines of fields for a shell to read, with LF endings.

    `sys.stdout` translates `\\n` to the system's own ending, so on a Windows
    runner each line ended with `\\r\\n`; a trailing `\\r` entered nextest's
    filterset as unparsed input and the Windows Rust repeat never ran. The
    bytes are written as they are read here, so the reader needs no fix of
    its own.
    """
    sys.stdout.buffer.write("".join(f"{line}\n" for line in lines).encode("utf-8"))
    sys.stdout.buffer.flush()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    resolve_parser = sub.add_parser("resolve", help="validate what called the lane and write its outputs")
    resolve_parser.add_argument("--event", required=True)
    resolve_parser.add_argument("--web", default="")
    resolve_parser.add_argument("--units", default="")
    resolve_parser.add_argument("--rust", default="")
    resolve_parser.add_argument("--repeat", default="")
    resolve_parser.add_argument("--runner", default="linux")
    pattern_parser = sub.add_parser("spec-patterns", help="one Playwright file filter per line for each spec")
    pattern_parser.add_argument("specs", nargs="+")
    sub.add_parser("count-nextest", help="count the tests of `cargo nextest list --message-format json` output on stdin")
    plan_parser = sub.add_parser("rust-plan", help="list the selected modules' tests and write the Rust job's plan")
    plan_parser.add_argument("--rust", required=True, help="the JSON list of modules")
    archive_parser = sub.add_parser("rust-archive", help="pack each target of a Rust plan into its own nextest archive")
    archive_parser.add_argument("--plan", required=True, help="the JSON plan")
    archive_parser.add_argument("--dir", required=True, type=Path, help="where the archives are written")
    lines_parser = sub.add_parser("plan-lines", help="one `package test filterset run_ignored archive` line per target of a Rust plan, fields split by UNIT_SEPARATOR")
    lines_parser.add_argument("--plan", required=True, help="the JSON plan")
    units_parser = sub.add_parser("units-plan", help="list the selected unit test files' tests on this runner and write a units plan")
    units_parser.add_argument("--units", required=True, help="the JSON list of unit test paths")
    units_parser.add_argument("--system", required=True, choices=sorted(RUNNERS), help="the system this runner is")
    units_parser.add_argument("--elsewhere", choices=[UNITS_ELSEWHERE],
                              help="a pull request's count: the system a file that lists no test here goes to")
    units_parser.add_argument("--repeat", help="a units leg: the repeat the count chose, or 0 to choose it from this listing")
    units_lines = sub.add_parser("units-lines", help="one `package<TAB>file` line per file of a units plan")
    units_lines.add_argument("--plan", required=True, help="the JSON plan")
    choose_parser = sub.add_parser("choose", help="write an explicit run decision for every repeat leg")
    choose_parser.add_argument("--event", required=True)
    choose_parser.add_argument("--web", required=True)
    choose_parser.add_argument("--units", default="[]")
    choose_parser.add_argument("--rust", required=True)
    choose_parser.add_argument("--repeat", default="")
    choose_parser.add_argument("--web-list", default="", help="a pull request's `playwright test --list` output of its specs")
    choose_parser.add_argument("--table", default=WEB_TABLE, help="the duration table the web shards are dealt by")
    choose_parser.add_argument("--units-count", default="")
    choose_parser.add_argument("--units-elsewhere", default="[]", help="the unit files the count found no test of on Linux")
    choose_parser.add_argument("--runner", default="linux", choices=sorted(RUNNERS), help="a hand run's system")
    choose_parser.add_argument("--rust-count", default="")
    sub.add_parser("verdict", help="fail unless every chosen repeat leg completed tests (NEEDS JSON)")
    share_parser = sub.add_parser("shard-share", help="print `group/groups repeat` for one shard of a web repeat")
    share_parser.add_argument("--shard", required=True, help="CURRENT/TOTAL, one-based")
    share_parser.add_argument("--parts", required=True, type=int)
    share_parser.add_argument("--repeat", required=True)
    total_parser = sub.add_parser("total", help="sum the completed runs each shard of a web repeat kept")
    total_parser.add_argument("--shards", required=True, type=int)
    total_parser.add_argument("counts", nargs="*", help="one file per shard holding its completed runs")
    report_parser = sub.add_parser("report", help="report completed runs from Playwright JSON or a JUnit report")
    reports = report_parser.add_mutually_exclusive_group(required=True)
    reports.add_argument("--playwright")
    reports.add_argument("--junit", nargs="+")
    report_parser.add_argument("--leg", choices=sorted(RUNNERS), help="a units leg's system, which names its output")
    args = parser.parse_args()

    try:
        if args.command == "choose":
            plan = None
            if args.web_list:
                plan = web_plan(Path(args.web_list).read_text(encoding="utf-8"), e2e_shard.read_table(args.table))
            decision = choose(args.event, args.web, json.loads(args.rust), args.repeat, plan, args.rust_count,
                              json.loads(args.units), args.units_count, json.loads(args.units_elsewhere), args.runner)
            write_outputs(decision)
            if os.environ.get("GITHUB_STEP_SUMMARY"):
                with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
                    web = (f"{plan['tests']} ({plan['seconds']} s a pass, the heaviest of {plan['shards']} shards "
                           f"in {plan['parts']} repeat parts {plan['heaviest']} s)") if plan else "not counted"
                    summary.write(f"Tests counted: web={web}, "
                                  f"units={args.units_count or 'not counted'}, Rust={args.rust_count or 'not counted'}; "
                                  f"unit files listed on {UNITS_ELSEWHERE}: {args.units_elsewhere}.\n")
                    summary.write(f"Repeat decision: {json.dumps(decision)}\n")
        elif args.command == "shard-share":
            found = re.fullmatch(r"([0-9]+)/([0-9]+)", args.shard)
            if not found:
                raise ValueError(f"expected CURRENT/TOTAL, got {args.shard!r}")
            group, groups, times = shard_share(int(found[1]), int(found[2]), args.parts, count_value(args.repeat))
            print(f"{group}/{groups} {times}")
        elif args.command == "total":
            write_outputs({"completed": total_completed(args.counts, args.shards)})
        elif args.command == "rust-archive":
            rust_archive(json.loads(args.plan), args.dir, nextest_packer)
            write_outputs({"rust-archived": "true"})
        elif args.command == "verdict":
            verdict(json.loads(os.environ["NEEDS"]))
            print("Every chosen repeat leg completed its tests.")
        elif args.command == "report":
            write_outputs({f"completed-{args.leg}" if args.leg else "completed": completed_runs(args.playwright, args.junit)})
        elif args.command == "resolve":
            work = resolve(args.event, args.web, args.rust, args.repeat, args.runner, args.units)
            write_outputs({
                "web": " ".join(work["web"]),
                "units": json.dumps(work["units"], separators=(",", ":")),
                "rust": json.dumps(work["rust"], separators=(",", ":")),
                "runner": RUNNERS[work["runner"]],
                "os": work["runner"],
                "repeat": work["repeat"] or "",
                "counted": str(work["counted"]).lower(),
                "web-count-needed": str(work["counted"] and bool(work["web"])).lower(),
                "units-count-needed": str(work["units_counted"]).lower(),
                "rust-count-needed": str(work["counted"] and bool(work["rust"])).lower(),
                "rust-plan": json.dumps(work["plan"], separators=(",", ":")),
            })
        elif args.command == "rust-plan":
            plan = rust_plan(json.loads(args.rust), nextest_lister)
            write_outputs({"rust-plan": json.dumps(plan, separators=(",", ":")), "rust-count": sum(entry["tests"] for entry in plan)})
        elif args.command == "units-plan":
            plan, moved = units_plan(json.loads(args.units), units_lister, args.system, args.elsewhere)
            count = sum(entry["tests"] for entry in plan)
            written = {"units-plan": json.dumps(plan, separators=(",", ":")), "units-count": count}
            if args.elsewhere:
                written["units-elsewhere"] = json.dumps(moved, separators=(",", ":"))
            if args.repeat is not None:
                # A leg the count could not list chooses its repeat from what it
                # lists here; the leg's job names the output by its system.
                written["repeat"] = written[f"repeat-{args.system}"] = count_value(args.repeat) or repeats(count)
            write_outputs(written)
        elif args.command == "units-lines":
            lines = []
            for entry in json.loads(args.plan):
                if entry["package"] not in UNIT_PACKAGES:
                    raise ValueError(f"not a units plan entry: {entry!r}")
                for path in entry["files"]:
                    if not UNIT_TEST.match(path) or UNIT_TEST.match(path)["package"] != entry["package"]:
                        raise ValueError(f"not a units plan entry: {entry!r}")
                    lines.append(f"{entry['package']}\t{path}")
            write_lines(lines)
        elif args.command == "plan-lines":
            lines = []
            for entry in plan_run_ignored(json.loads(args.plan)):
                plan_entry(entry)
                lines.append(UNIT_SEPARATOR.join((entry["package"], entry["test"], entry["filter"],
                                                  entry.get("run_ignored", ""), archive_name(entry))))
            write_lines(lines)
        elif args.command == "count-nextest":
            print(nextest_listed(sys.stdin.read()))
        elif args.command == "spec-patterns":
            for path in args.specs:
                print(spec_pattern(path))
        else:
            raise ValueError(f"unknown command {args.command}")
    except (ValueError, KeyError, TypeError, OSError, ET.ParseError, subprocess.CalledProcessError) as error:
        print(f"ci-repeat: {error}" + (f"\n{error.stderr}" if isinstance(error, subprocess.CalledProcessError) and error.stderr else ""), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
