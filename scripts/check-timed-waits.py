#!/usr/bin/env python3
"""No new time-based wait enters a test (docs/TESTING.md, "Wait for state, not time").

Two kinds of wait are counted, per file, against `scripts/timed-waits-baseline.json`:

- `e2e_timeouts`: a `timeout:` override in `web/e2e` or `desktop/e2e`, a spec or a
  helper. A longer limit does not make a wait correct; it hides a state the test
  does not name.
- `rust_sleeps`: `thread::sleep`, `tokio::time::sleep` (also imported, as
  `use tokio::time::{sleep}` and then `sleep(d)`, also as an alias) in test code: a file under `tests/`, a `tests.rs` or
  `*_tests.rs` file, a module its parent declares behind `#[cfg(test)]`, or an
  item behind `#[cfg(test)]`. Clippy refuses `std::thread::sleep` already; this
  also counts the async sleep and the one an `#[allow]` lets through.

The baseline holds the number each file has today, never a line number, so an
edit that moves a line, or another change that removes a wait, cannot fail
this check. A file fails only when it has more unannotated waits than its
baseline (0 for a file the baseline does not list). The baseline only ever goes
down: `--lower` rewrites it to the smaller of the recorded and the current
count.

Editing a number up would let a new wait through, so CI also runs
`--base HEAD^1`: no file's number in the baseline may be higher than the
baseline of the base commit holds for that file (0 for a file it does not list).
A file `git` shows as renamed from the base may carry its old file's number, so
a renamed spec moves its baseline key by hand and the ratchet follows the move.

A wait that waits for a named state says so on its own line:

    await expect(row).toBeVisible({ timeout: 90_000 }); // wait-ok: a cold holder start on Windows
    thread::sleep(POLL); // wait-ok: the next poll of the pid file

The text after `wait-ok:` names the state, is not empty, and is read by the
reviewer. An annotated wait is not counted.
"""
import argparse
import json
import pathlib
import re
import subprocess
import sys
from bisect import bisect_right
from collections import Counter

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rust_scan  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "timed-waits-baseline.json"

E2E_DIRS = ("web/e2e", "desktop/e2e")
E2E_SUFFIXES = (".ts", ".mts", ".mjs")
SKIPPED_DIRS = {"target", "node_modules", "vendor", ".git", "agents", "dist", "out"}

E2E_WAIT = re.compile(r"\btimeout\s*:")
RUST_WAIT = re.compile(r"\bthread::sleep\b|\btime::sleep\b")
RUST_USE = re.compile(r"\buse\b\s*(?=[\w:{])[^;]*;")
RUST_USE_GLOB = re.compile(r"\b(?:thread|time)::(?:\*|\{[^}]*\*[^}]*\})")
RUST_USE_NAME = re.compile(r"\b(?:thread|time)::sleep\b(?:\s+as\s+(\w+))?")
RUST_USE_GROUP = re.compile(r"\b(?:thread|time)::\{([^}]*)\}")
RUST_USE_MEMBER = re.compile(r"\s*sleep(?:\s+as\s+(\w+))?\s*")
RUST_SCOPE_TOKEN = re.compile(
    r"(?P<use>" + RUST_USE.pattern + r")"
    r"|(?P<module>\bmod\s+\w+\s*\{)"
    r"|\bfn\s+(?P<function>\w+)"
    r"|(?P<brace>[{}])"
    r"|(?P<call>(?<![\w:.])\w+)\s*\("
)
ANNOTATION = re.compile(r"//\s*wait-ok:[ \t]*(\S[^\n]*)")

KINDS = {
    "e2e_timeouts": "a `timeout:` override in an e2e spec or helper",
    "rust_sleeps": "a sleep in a Rust test",
}

ADVICE = """\
A new time-based wait is not allowed in a test. Wait for the state the next step needs instead
(docs/TESTING.md, "Wait for state, not time"):
  - web e2e: `control.until` (snapshot) or `untilLogged` (the core's log) in web/e2e/daemon-actions.ts, a data attribute, a diagnostic `kind`;
  - Rust: the event, file or answer the next line needs, or an injected clock.
If the wait really is for a state that only a time bound can express, end its line with
`// wait-ok: <the state it waits for>`; the check accepts that form and a reviewer reads the reason."""

_TS_BLANKED = re.compile(
    r"//[^\n]*|/\*.*?\*/|'(?:\\.|[^'\\\n])*'|\"(?:\\.|[^\"\\\n])*\"|`(?:\\.|[^`\\])*`", re.S
)


def _blank(text: str) -> str:
    return re.sub(r"[^\n]", " ", text)


def strip_ts(text: str) -> str:
    """Blanks comments and string and template literals, keeping line numbers and columns."""
    return _TS_BLANKED.sub(lambda match: _blank(match.group(0)), text)


def test_part(text: str) -> str:
    """The code behind `#[cfg(test)]` in a Rust file, comments and literals blanked, the rest blanked too."""
    stripped = rust_scan.strip(text)
    production = rust_scan.strip_test_items(stripped)
    return "".join(
        char if char == "\n" or char != kept else " " for char, kept in zip(stripped, production)
    )


def pattern_matches(code: str, pattern: re.Pattern) -> list:
    """(1-based line, column after the match) of every match of `pattern` in `code`."""
    return [
        (number, match.end())
        for number, line in enumerate(code.split("\n"), 1)
        for match in pattern.finditer(line)
    ]


def unannotated(raw: str, matches: list) -> list:
    """The 1-based lines of `matches` that carry no `// wait-ok: <state>` after the match."""
    raw_lines = raw.split("\n")
    return [number for number, end in matches if not ANNOTATION.search(raw_lines[number - 1], end)]


def sleep_names(code: str) -> set:
    """Names brought in by the supplied `use` items, including aliases and timer globs."""
    names = set()
    for statement in RUST_USE.findall(code):
        if RUST_USE_GLOB.search(statement):
            names.add("sleep")
        for match in RUST_USE_NAME.finditer(statement):
            names.add(match.group(1) or "sleep")
        for group in RUST_USE_GROUP.finditer(statement):
            for member in group.group(1).split(","):
                named = RUST_USE_MEMBER.fullmatch(member)
                if named:
                    names.add(named.group(1) or "sleep")
    return names


class SleepScope:
    """Item bindings belong to a brace scope, even before their declaration.

    Blocks inherit their enclosing scope. Modules require `use super::*` to
    inherit it, and a local function shadows an inherited timer binding.
    """
    def __init__(self, parent=None, module=False):
        self.parent = parent
        self.inherits = not module
        self.bindings = {}

    def is_timer(self, name: str) -> bool:
        if name in self.bindings:
            return self.bindings[name]
        return bool(self.inherits and self.parent and self.parent.is_timer(name))


def rust_waits(whole: str, tested: str) -> list:
    """Qualified sleeps and calls bound to a timer import in their brace scope.

    Consume whole use items so grouped imports' braces cannot open a scope.
    Resolve after scanning so item order within a scope does not matter.
    The test mask limits findings, but production imports still bind names.
    """
    code = RUST_USE.sub(lambda match: _blank(match.group(0)), tested)
    found = pattern_matches(code, RUST_WAIT)
    scopes = [SleepScope()]
    calls = []
    for token in RUST_SCOPE_TOKEN.finditer(whole):
        scope = scopes[-1]
        if token.group("use"):
            for name in sleep_names(token.group()):
                scope.bindings.setdefault(name, True)
            if re.search(r"\buse\s+super::\*\s*;", token.group()):
                scope.inherits = True
        elif token.group("module") or token.group("brace") == "{":
            scopes.append(SleepScope(scope, module=bool(token.group("module"))))
        elif token.group("brace") == "}":
            scopes.pop()
        elif token.group("function"):
            scope.bindings[token.group("function")] = False
        elif tested[token.start():token.end()].strip():
            calls.append((token, scope))
    newlines = [-1] + [match.start() for match in re.finditer("\n", whole)]
    for token, scope in calls:
        if scope.is_timer(token.group("call")):
            number = bisect_right(newlines, token.start())
            found.append((number, token.end() - newlines[number - 1] - 1))
    return sorted(found)


def e2e_files() -> list:
    files = []
    for directory in E2E_DIRS:
        base = ROOT / directory
        if base.is_dir():
            files += [
                path for path in base.rglob("*")
                if path.suffix in E2E_SUFFIXES and not SKIPPED_DIRS.intersection(path.relative_to(base).parts)
            ]
    return sorted(files)


def rust_trees() -> list:
    """The workspace's crate folders: the folders with a Cargo.toml, `vendor` left out."""
    return sorted(
        directory for directory in ROOT.iterdir()
        if directory.is_dir() and directory.name not in SKIPPED_DIRS and (directory / "Cargo.toml").exists()
    )


def rust_files(trees: list) -> list:
    return sorted(
        path for tree in trees for path in tree.rglob("*.rs")
        if not SKIPPED_DIRS.intersection(path.relative_to(tree).parts)
    )


def measure() -> dict:
    """The unannotated waits now: kind, then file (relative, `/`-separated), then the lines."""
    found = {kind: {} for kind in KINDS}
    for path in e2e_files():
        raw = path.read_text()
        lines = unannotated(raw, pattern_matches(strip_ts(raw), E2E_WAIT))
        if lines:
            found["e2e_timeouts"][path.relative_to(ROOT).as_posix()] = lines
    trees = rust_trees()
    test_modules = rust_scan.test_modules(ROOT, trees)
    for path in rust_files(trees):
        relative = path.relative_to(ROOT).as_posix()
        raw = path.read_text()
        whole = rust_scan.strip(raw)
        tested = whole if rust_scan.test_file(relative) or relative in test_modules else test_part(raw)
        lines = unannotated(raw, rust_waits(whole, tested))
        if lines:
            found["rust_sleeps"][relative] = lines
    return found


def load_baseline() -> dict:
    data = json.loads(BASELINE.read_text())
    unknown = set(data) - set(KINDS)
    if unknown:
        raise SystemExit(f"{BASELINE.name}: unknown section(s) {sorted(unknown)}")
    return {kind: dict(data.get(kind, {})) for kind in KINDS}


def violations(found: dict, baseline: dict) -> list:
    out = []
    for kind, files in found.items():
        for relative, lines in sorted(files.items()):
            allowed = baseline[kind].get(relative, 0)
            if len(lines) > allowed:
                out.append(
                    f"{relative}: {len(lines)} unannotated, {KINDS[kind]}; the baseline allows {allowed} "
                    f"(lines {', '.join(map(str, lines))})"
                )
    return out


def lowered(found: dict, baseline: dict) -> dict:
    """The baseline with every count brought down to what the files have now."""
    return {
        kind: {
            relative: min(count, len(found[kind].get(relative, [])))
            for relative, count in sorted(baseline[kind].items())
            if found[kind].get(relative)
        }
        for kind in KINDS
    }


def write(baseline: dict) -> None:
    BASELINE.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n")


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True, timeout=10)


def base_baseline(ref: str):
    """The baseline file as `ref` holds it, or None where `ref` has none yet (the change that introduces it)."""
    if git("rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}").returncode != 0:
        raise SystemExit(f"--base {ref}: not a commit this checkout has (CI checks out two commits for it)")
    shown = git("show", f"{ref}:{BASELINE.relative_to(ROOT).as_posix()}")
    if shown.returncode != 0:
        return None
    data = json.loads(shown.stdout)
    return {kind: dict(data.get(kind, {})) for kind in KINDS}


def renamed_since(ref: str) -> dict:
    """New path -> old path for every file `git` shows renamed between `ref` and the working tree."""
    out = git("diff", "-M", "--name-status", "--diff-filter=R", ref).stdout
    renames = {}
    for line in out.splitlines():
        status, *paths = line.split("\t")
        if len(paths) == 2:
            renames[paths[1]] = paths[0]
    return renames


def raised(baseline: dict, base: dict, renames: dict) -> list:
    """The baseline entries higher than the base commit's baseline allows: a file's number may only stay or fall,
    and a file the base does not list starts at 0 unless `git` shows it renamed from one the base lists."""
    out = []
    for kind in KINDS:
        for relative, count in sorted(baseline[kind].items()):
            allowed = base[kind].get(relative, base[kind].get(renames.get(relative, ""), 0))
            if count > allowed:
                out.append(
                    f"{BASELINE.name}: {kind} {relative} is {count}, up from {allowed}; "
                    "the baseline only goes down, so remove the wait or end its line with `// wait-ok: <state>`"
                )
    return out


def main(argv: list) -> int:
    parser = argparse.ArgumentParser(description="Refuse new time-based waits in tests.")
    parser.add_argument("--lower", action="store_true", help="bring the baseline down to the waits there are now")
    parser.add_argument("--base", metavar="REF", help="also refuse a baseline number higher than REF's (CI: HEAD^1)")
    args = parser.parse_args(argv)
    found = measure()
    baseline = load_baseline()
    if args.lower:
        write(lowered(found, baseline))
        print(f"{BASELINE.relative_to(ROOT)} lowered")
        return 0
    failures = violations(found, baseline)
    if args.base:
        base = base_baseline(args.base)
        if base is None:
            print(f"{args.base} has no {BASELINE.name}: nothing to compare the baseline with")
        else:
            failures += raised(baseline, base, renamed_since(args.base))
    if failures:
        print("\n".join(failures), file=sys.stderr)
        print(f"{len(failures)} finding(s) of a new time-based wait\n\n{ADVICE}", file=sys.stderr)
        return 1
    slack = Counter()
    for kind in KINDS:
        for relative, count in baseline[kind].items():
            if count > len(found[kind].get(relative, [])):
                slack[kind] += count - len(found[kind].get(relative, []))
    if slack:
        print(
            "baseline can be lowered ("
            + ", ".join(f"{kind}: {n}" for kind, n in sorted(slack.items()))
            + "): python3 scripts/check-timed-waits.py --lower"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
