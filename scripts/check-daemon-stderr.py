#!/usr/bin/env python3
"""Daemon code in `hided/src` writes its log lines through `hide_core::diagnostic!`,
never `eprintln!` (docs/ARCHITECTURE.md, Diagnostics).

`eprintln!` panics when stderr cannot be written, and a login item's stderr is
`Logs/core.log`: a full disk made one panic under `ObservationDemand`'s lock
on 2026-11-27, which poisoned it and panicked every later connection event
until the core restarted. `diagnostic!` loses a line it cannot write and never
unwinds its caller.

The files below run in the operator's terminal or in a pane's shell, where
printing to stderr is the program's output and nothing holds a lock; each is
named with why. Test code is not read: a `#[cfg(test)]` item, a module declared
behind it, `tests/` folders and `*_tests.rs` files.
"""
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import rust_scan  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCES = ROOT / "hided" / "src"

# Each file is named: a new file, whatever its name ends in, is daemon code until
# it is added here with its reason.
# `hide` and `hided`'s `main`: the entry points print their usage and their
# last error to the terminal that started them, before any role holds a lock
# and just before they exit with a code.
ENTRY_POINTS = ("bin/",)
# What an operator or an agent types: `hide <command>` prints its answer and
# its refusal to its own terminal.
COMMAND_FILES = (
    "access_cli.rs", "agent_cli.rs", "automation_cli.rs", "browser_cli.rs", "cli.rs",
    "delivery_cli.rs", "device_cli.rs", "issue_cli.rs", "mission_cli.rs", "node_cli.rs",
    "pane_cli.rs", "phone_cli.rs", "plan_cli.rs", "profile_cli.rs", "ps_cli.rs",
    "session_cli.rs", "store_cli.rs", "wait_cli.rs", "workspace_cli.rs", "worktree_cli.rs",
)
# A pane's shell: `hide agent exec` and the pane's own `hide` install run as
# the pane's foreground program and tell the person typing there.
PANE_FILES = ("agent_exec.rs", "pane_install.rs")

# The macro's name wherever it appears, not only as a call: `use std::eprintln as log;`
# or a `macro_rules!` that expands to it panics the same way.
STDERR_MACRO = re.compile(r"\b(?:eprintln|eprint|dbg)\b")
# What a terminal-printing file may still do with the name: call it. Naming it
# in a `use`, or defining a macro around it, would hand a daemon module a way to
# print that the daemon scan above cannot see (`pub use std::eprintln as log;`
# in a command file, `log!` in the daemon).
STDERR_CALL = re.compile(r"\b(?:eprintln|eprint|dbg)\s*!\s*[(\[{]")
# A write to the stderr handle that unwraps its result panics the same way.
STDERR_UNWRAP = re.compile(r"\bstderr\s*\(\)[^;]*\.(?:unwrap|expect)\s*\(")
EXPORTED_MACRO = re.compile(r"\bmacro_rules\s*!|#\s*\[\s*macro_(?:use|export)\b")

ADVICE = """\
Daemon code does not print to stderr: `eprintln!` panics when the write fails, and a panic under a
lock poisons it for every later caller. Record the event instead:
    hide_core::diagnostic!(serde_json::json!({"component": "hided", "kind": "<area>.<what>", ...}));
It reaches stderr and the Logs file, and a failed write loses the line. A file that prints to a terminal
as its output is listed in scripts/check-daemon-stderr.py with the reason; it may call `eprintln!`, but
not re-export or wrap it (`use ... as`, `macro_rules!`), which would let daemon code print through it."""


def allowed(relative: str) -> bool:
    return (
        relative.startswith(ENTRY_POINTS)
        or relative in COMMAND_FILES
        or relative in PANE_FILES
    )


def violations(root: pathlib.Path = ROOT) -> list:
    """`path:line` of each stderr macro in daemon production code under `root`."""
    sources = root / "hided" / "src"
    tests = rust_scan.test_modules(root, [sources])
    found = []
    for path in sorted(sources.rglob("*.rs")):
        relative = path.relative_to(sources).as_posix()
        repo_relative = path.relative_to(root).as_posix()
        if repo_relative in tests or rust_scan.test_file(repo_relative):
            continue
        code = rust_scan.strip_test_items(rust_scan.strip(path.read_text()))
        terminal = allowed(relative)
        for number, line in enumerate(code.split("\n"), 1):
            if terminal:
                named = STDERR_MACRO.sub(lambda m: "" if STDERR_CALL.match(line, m.start()) else m.group(0), line)
                bad = STDERR_MACRO.search(named) or EXPORTED_MACRO.search(line)
            else:
                bad = STDERR_MACRO.search(line) or STDERR_UNWRAP.search(line)
            if bad:
                found.append(f"{repo_relative}:{number}")
    return found


def main() -> int:
    found = violations()
    if not found:
        return 0
    print("daemon code prints to stderr:", file=sys.stderr)
    for place in found:
        print(f"  {place}", file=sys.stderr)
    print(ADVICE, file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
