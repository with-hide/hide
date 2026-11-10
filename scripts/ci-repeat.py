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
* Rust: a module of a crate's `tests/it` binary added or modified (`foo.rs`,
  `foo/mod.rs` or `foo/bar.rs`); every test of that module and its children,
  picked with nextest's `test(/^module::/)`. `tests/it/support`, the
  helpers every module shares, is no test, and a deleted file has none left.

The budget is one number for the whole pull request: `BUDGET` test runs. Each
test runs `repeats(N)` times, N being the tests that will run, counted by the
tools that run them (`playwright test --list`, `cargo nextest list`), of both kinds:
`clamp(floor(BUDGET / N), MIN_REPEAT, MAX_REPEAT)`. A pull request that
changes more than `BUDGET / MIN_REPEAT` tests runs over the budget, because a
repeat below three shows too little.

`select` is what `scripts/ci-plan.py` calls; the subcommands are what
`repeat.yml` calls.
"""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys

ROOT = Path(__file__).resolve().parent.parent

BUDGET = 200
MIN_REPEAT = 3
MAX_REPEAT = 10
# A hand run names its own repeat; this is the most it may ask for, so a typo
# cannot hold a runner for a day.
DISPATCH_MAX_REPEAT = 100

WEB_SPEC = re.compile(r"^web/e2e/[^/]+\.spec\.ts$")
# A crate's `tests/it` binary: the crate directory, then the module's file.
RUST_MODULE = re.compile(r"^(?P<crate>.+)/tests/it/(?P<module>.+)\.rs$")
# Helpers every module of the binary shares, and the file that declares the modules.
RUST_SHARED = ("support", "main")

PACKAGE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
MODULE = re.compile(r"^[a-z0-9_]+(?:::[a-z0-9_]+)*$")
SPEC_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.spec\.ts$")

RUNNERS = {"linux": "ubuntu-latest", "macos": "macos-15", "windows": "windows-latest"}


def repeats(total):
    """How many times each of `total` changed tests runs; 0 when none changed."""
    if total < 0:
        raise ValueError(f"a negative test count: {total}")
    if total == 0:
        return 0
    return max(MIN_REPEAT, min(MAX_REPEAT, BUDGET // total))


def rust_module(path, crate_dirs):
    """The `{package, module}` a `tests/it` file declares, or None for a file that is no test module."""
    match = RUST_MODULE.match(path)
    if not match:
        return None
    package = crate_dirs.get(match["crate"])
    if package is None:
        return None
    parts = match["module"].split("/")
    if parts[0] in RUST_SHARED:
        return None
    if parts[-1] == "mod":
        parts = parts[:-1]
    return {"package": package, "module": "::".join(parts)}


def select(entries, crate_dirs):
    """The changed tests of `git diff --name-status` entries: Playwright spec files and Rust test modules."""
    web = sorted({name for status, name in entries if status in ("A", "M") and WEB_SPEC.match(name)})
    modules = {}
    for status, name in entries:
        if status not in ("A", "M"):
            continue
        found = rust_module(name, crate_dirs)
        if found:
            modules[(found["package"], found["module"])] = found
    # `foo` runs `foo::bar` already, so a changed child of a changed parent is not run twice.
    rust = [
        found for (package, module), found in sorted(modules.items())
        if not any(module.startswith(other + "::") for other_package, other in modules if other_package == package)
    ]
    return {"web": web, "rust": rust}


def nextest_listed(text):
    """The tests `cargo nextest list --message-format json` says the filterset runs.

    A test counts when its filter matched; an ignored test does not match
    unless the run asks for ignored tests, which this lane never does.
    """
    suites = json.loads(text).get("rust-suites")
    if not isinstance(suites, dict):
        raise ValueError("the nextest list has no rust-suites")
    return sum(
        1
        for suite in suites.values()
        for case in suite.get("testcases", {}).values()
        if case.get("filter-match", {}).get("status") == "matches"
    )


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


def rust_entry(item):
    """A hand-run `package:module` as the `{package, module}` it names."""
    package, separator, module = item.partition(":")
    if not separator or not PACKAGE.match(package) or not MODULE.match(module):
        raise ValueError(f"not a Rust test module: {item!r} (expected package:module, like hided:handshake)")
    return {"package": package, "module": module}


def nextest_filters(modules):
    """One `(package, filterset)` per package: its binary `it`, the selected modules' tests."""
    by_package = {}
    for module in modules:
        if not PACKAGE.match(module["package"]) or not MODULE.match(module["module"]):
            raise ValueError(f"not a Rust test module: {module!r}")
        by_package.setdefault(module["package"], []).append(module["module"])
    return [(package, "test(/^(" + "|".join(sorted(names)) + ")::/)") for package, names in sorted(by_package.items())]


def resolve(event, web, rust, repeat, runner):
    """The lane's work from what called it.

    A pull request's call hands over the lists `ci-plan.py` selected (JSON);
    a hand run names specs and `package:module` tests in plain words and the
    repeat it wants, which is validated here and never counted.
    """
    if event == "workflow_dispatch":
        if runner not in RUNNERS:
            raise ValueError(f"unknown runner {runner!r}; expected one of {', '.join(RUNNERS)}")
        if not re.fullmatch(r"[0-9]+", repeat) or not 1 <= int(repeat) <= DISPATCH_MAX_REPEAT:
            raise ValueError(f"repeat must be a number from 1 to {DISPATCH_MAX_REPEAT}, not {repeat!r}")
        specs = sorted({spec_path(name) for name in web.split()})
        modules = sorted((rust_entry(item) for item in rust.split()), key=lambda found: (found["package"], found["module"]))
        if not specs and not modules:
            raise ValueError("name at least one spec (web) or one Rust test module (rust)")
        return {"web": specs, "rust": modules, "runner": runner, "repeat": int(repeat), "counted": False}
    specs = json.loads(web or "[]")
    modules = json.loads(rust or "[]")
    for path in specs:
        if not WEB_SPEC.match(path):
            raise ValueError(f"not a web e2e spec: {path!r}")
    for module in modules:
        if not PACKAGE.match(module["package"]) or not MODULE.match(module["module"]):
            raise ValueError(f"not a Rust test module: {module!r}")
    # A pull request runs on Linux only; the hand run is the way to another system.
    return {"web": specs, "rust": modules, "runner": "linux", "repeat": 0, "counted": bool(specs or modules)}


def write_outputs(values):
    line = "".join(f"{key}={value}\n" for key, value in values.items())
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
            output.write(line)
    sys.stdout.write(line)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    resolve_parser = sub.add_parser("resolve", help="validate what called the lane and write its outputs")
    resolve_parser.add_argument("--event", required=True)
    resolve_parser.add_argument("--web", default="")
    resolve_parser.add_argument("--rust", default="")
    resolve_parser.add_argument("--repeat", default="")
    resolve_parser.add_argument("--runner", default="linux")
    budget_parser = sub.add_parser("budget", help="the repeat for a pull request's changed tests")
    budget_parser.add_argument("--web", type=int, required=True, help="Playwright tests, from `playwright test --list`")
    budget_parser.add_argument("--rust", type=int, required=True, help="Rust tests, from `cargo nextest list`")
    sub.add_parser("count-listed", help="count the tests of `playwright test --list` output on stdin")
    pattern_parser = sub.add_parser("spec-patterns", help="one Playwright file filter per line for each spec")
    pattern_parser.add_argument("specs", nargs="+")
    filter_parser = sub.add_parser("nextest-filters", help="one `package<TAB>filterset` line per package")
    filter_parser.add_argument("--rust", required=True, help="the JSON list of modules")
    sub.add_parser("count-nextest", help="count the tests of `cargo nextest list --message-format json` output on stdin")
    args = parser.parse_args()

    try:
        if args.command == "resolve":
            work = resolve(args.event, args.web, args.rust, args.repeat, args.runner)
            write_outputs({
                "web": " ".join(work["web"]),
                "rust": json.dumps(work["rust"], separators=(",", ":")),
                "runner": RUNNERS[work["runner"]],
                "os": work["runner"],
                "repeat": work["repeat"] or "",
                "counted": str(work["counted"]).lower(),
                "web-count-needed": str(work["counted"] and bool(work["web"])).lower(),
                "rust-count-needed": str(work["counted"] and bool(work["rust"])).lower(),
            })
        elif args.command == "count-nextest":
            print(nextest_listed(sys.stdin.read()))
        elif args.command == "count-listed":
            print(listed_tests(sys.stdin.read()))
        elif args.command == "budget":
            total = args.web + args.rust
            write_outputs({"repeat": repeats(total), "total": total})
        elif args.command == "spec-patterns":
            for path in args.specs:
                print(spec_pattern(path))
        else:
            for package, expression in nextest_filters(json.loads(args.rust)):
                print(f"{package}\t{expression}")
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(f"ci-repeat: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
