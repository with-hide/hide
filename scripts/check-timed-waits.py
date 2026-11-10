#!/usr/bin/env python3
"""No new time-based wait enters a test (docs/TESTING.md, "Wait for state, not time").

Two kinds of wait are counted, per file, against `scripts/timed-waits-baseline.json`:

- `e2e_timeouts`: a `timeout:` override in `web/e2e` or `desktop/e2e`, a spec or a
  helper. A longer limit does not make a wait correct; it hides a state the test
  does not name.
- `rust_sleeps`: `thread::sleep`, `tokio::time::sleep` (also imported, as
  `use tokio::time::{sleep}`) in test code: a file under `tests/`, a `tests.rs` or
  `*_tests.rs` file, a module its parent declares behind `#[cfg(test)]`, or an
  item behind `#[cfg(test)]`. Clippy refuses `std::thread::sleep` already; this
  also counts the async sleep and the one an `#[allow]` lets through.

The baseline holds the number each file has today, never a line number, so an
edit that moves a line, or another change that removes a wait, cannot fail
this check. A file fails only when it has more unannotated waits than its
baseline (0 for a file the baseline does not list). The baseline only ever goes
down: `--lower` rewrites it to the smaller of the recorded and the current
count. A renamed file carries its baseline entry by hand, in the diff.

A wait that waits for a named state says so on its own line:

    await expect(row).toBeVisible({ timeout: 90_000 }); // wait-ok: a cold holder start on Windows
    thread::sleep(POLL); // wait-ok: the next poll of the pid file

The text after `wait-ok:` names the state, is not empty, and is read by the
reviewer. An annotated wait is not counted.
"""
import json
import pathlib
import re
import sys
from collections import Counter

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rust_scan  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "timed-waits-baseline.json"

E2E_DIRS = ("web/e2e", "desktop/e2e")
E2E_SUFFIXES = (".ts", ".mts", ".mjs")
SKIPPED_DIRS = {"target", "node_modules", "vendor", ".git", "agents", "dist", "out"}

E2E_WAIT = re.compile(r"\btimeout\s*:")
RUST_WAIT = re.compile(r"\bthread::sleep\b|\btime::sleep\b|\b(?:thread|time)::\{[^}]*\bsleep\b")
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


def unannotated(raw: str, code: str, pattern: re.Pattern) -> list:
    """The 1-based lines of `code` that match `pattern` and carry no `// wait-ok: <state>` after the match."""
    raw_lines = raw.split("\n")
    lines = []
    for number, line in enumerate(code.split("\n"), 1):
        for match in pattern.finditer(line):
            note = ANNOTATION.search(raw_lines[number - 1], match.end())
            if not note:
                lines.append(number)
    return lines


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
        lines = unannotated(raw, strip_ts(raw), E2E_WAIT)
        if lines:
            found["e2e_timeouts"][path.relative_to(ROOT).as_posix()] = lines
    trees = rust_trees()
    test_modules = rust_scan.test_modules(ROOT, trees)
    for path in rust_files(trees):
        relative = path.relative_to(ROOT).as_posix()
        raw = path.read_text()
        if rust_scan.test_file(relative) or relative in test_modules:
            code = rust_scan.strip(raw)
        else:
            code = test_part(raw)
        lines = unannotated(raw, code, RUST_WAIT)
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


def main(argv: list) -> int:
    found = measure()
    baseline = load_baseline()
    if argv == ["--lower"]:
        write(lowered(found, baseline))
        print(f"{BASELINE.relative_to(ROOT)} lowered")
        return 0
    if argv:
        print("usage: check-timed-waits.py [--lower]", file=sys.stderr)
        return 2
    failures = violations(found, baseline)
    if failures:
        print("\n".join(failures), file=sys.stderr)
        print(f"{len(failures)} file(s) with a new time-based wait\n\n{ADVICE}", file=sys.stderr)
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
