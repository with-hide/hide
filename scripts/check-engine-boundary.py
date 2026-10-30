#!/usr/bin/env python3
"""Hide speaks to the program that runs its panes through one engine
boundary (PRD engine-bundle-1 D-2, D-12): `herdr_core::engine`, whose
Herdr implementation is the only code that holds a Herdr connection, names
Herdr's wire types or uses `hide-herdr-client`. This check fails when
production code anywhere else does one of those, or starts the Herdr
program, and says what to use instead.

Test code is exempt, as in `check-core-touches-no-machine.py`: test-only
files, a module file its parent declares behind `#[cfg(test)]`, and every
item behind `#[cfg(test)]`; so are test fixtures and measurement scripts,
which never ship. What may still speak Herdr directly is listed below by
file, each with the number of findings it has and the bundle of the
own-terminal-engine plan that deletes it; a new finding in a listed file
fails like one anywhere else, and an entry whose file no longer speaks
Herdr fails until it is removed.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# The Rust crates that ship, and the desktop host.
RUST_TREES = [
    "hide-agent-adapter/src",
    "herdr-core/src",
    "hide-herdr-client/src",
    "hide-host/src",
    "hide-kit/src",
    "hide-agent-hooks/src",
    "hide-ai/src",
    "hide-memory/src",
    "hide-node/src",
    "hide-node-link/src",
    "hide-platform/src",
    "hide-project/src",
    "hide-session/src",
    "hide-shell/src",
    "hided/src",
]
TS_TREES = ["desktop/src"]

INSTEAD = (
    "ask the engine boundary instead: `herdr_core::engine::EngineHandle` outside "
    "the core, `&dyn Engine` inside it, or a `hide` command from a pane"
)

# What speaking Herdr directly looks like in Rust, with what to do instead.
RUST_BANNED = [
    (r"\bhide_herdr_client\b", f"uses the Herdr client; {INSTEAD}"),
    (r"\bApiConnector\b", f"holds a Herdr connection; {INSTEAD}"),
    (r"(?<![\w:])(?:[\w:]*::)?wire::|herdr_(?:request|success_response|event)\.rs",
     f"names Herdr's wire terms, which only the engine's Herdr implementation reads; {INSTEAD}"),
]

# A process start whose program is Herdr's: the start itself, and Herdr's
# program named on its line or in the few lines before it (the binding or
# the function signature that brought the path in).
PROCESS_START = re.compile(
    r"\bCommand::new\(|\bprograms::command\(|\bprocess::run\(|\bOwnedChild::spawn\("
)
HERDR_PROGRAM = re.compile(
    r"(?i)\bherdr_(?:bin|program|exe)(?:_path)?\b|HERDR_BIN_PATH|start-herdr|ensure_herdr"
    r"|\bherdr\s*\.\s*bin\b|device_server|\bherdr\s*[:=,)]|\(\s*&?herdr\b"
)
PROGRAM_CONTEXT = 8
LAUNCH_ADVICE = f"starts the Herdr program; {INSTEAD}"

# The desktop host names the Herdr program it hands to `hide`.
TS_BANNED = [
    (r"\bHERDR_BIN_PATH\b|\bherdrBinPath\b|\bchooseHerdr\b",
     f"names the Herdr program a child runs; {INSTEAD}"),
]

# The engine's Herdr implementation: the one place Herdr's API is spoken.
# Bundle 4 (the switch release) deletes Herdr, and these with it.
IMPLEMENTATION = [
    "herdr-core/src/engine/herdr.rs",
    "herdr-core/src/engine/herdr/",
    "hide-herdr-client/src/",
]

# What may still speak Herdr directly, with its number of findings and the
# bundle that deletes it. Nothing joins without that bundle.
ALLOWED = {
    # Herdr runtime management: the operator's Herdr server and its program.
    "hided/src/herdr_handoff.rs": (
        10, "moves the operator's Herdr server onto the bundled one; bundle 4 deletes the handoff"),
    "hided/src/login_item.rs": (
        1, "starts this account's Herdr server at login; bundle 4 deletes the bundled Herdr"),
    "hided/src/core_move/preflight.rs": (
        1, "asks a moved core's Herdr whether it runs; bundle 4 deletes the bundled Herdr"),
    "desktop/src/main/herdr.ts": (
        5, "chooses the Herdr program `hide` runs; bundle 4 removes `HERDR_*`"),
    "desktop/src/main/host.ts": (
        3, "hands that Herdr program to `hide`; bundle 4 removes `HERDR_*`"),
    "desktop/src/main/env.ts": (
        5, "reads `HERDR_BIN_PATH` into the host's environment; bundle 4 removes `HERDR_*`"),
    # The terminal path: Herdr's own terminal client.
    "hide-node/src/terminal/session.rs": (
        1, "runs `herdr terminal session` for a pane's bytes; bundle 2 replaces the terminal path"),
    # Herdr's plugin, which the kit installs and retires.
    "hide-kit/src/herdr_integration.rs": (
        1, "installs and removes Hide's Herdr plugin; bundle 4 deletes the plugin"),
    "hide-kit/src/labels.rs": (
        10, "uninstalls the retired labels plugin; bundle 4 deletes the plugin"),
    "hide-kit/src/coordination_retirement.rs": (
        1, "names the retired coordination plugin's Herdr types; bundle 4 deletes the plugin"),
    # A device's half: the node holds that machine's Herdr socket and proves
    # a pane's process against it.
    "hide-node/src/lib.rs": (
        3, "builds this machine's Herdr connection for the engine; bundle 3 moves panes to the node"),
    "hide-node/src/ssh.rs": (
        2, "names the Herdr connection a device transport hands out; bundle 3 moves panes to the node"),
    "hide-node/src/ssh/device.rs": (
        4, "carries a device's Herdr connection over its link; bundle 3 moves panes to the node"),
    "hide-node/src/ssh/inbound.rs": (
        2, "carries an inbound node's Herdr connection; bundle 3 moves panes to the node"),
    "hide-node/src/ssh/link_streams.rs": (
        3, "opens Herdr streams over a device link; bundle 3 moves panes to the node"),
    "hide-node/src/ssh/wsl.rs": (
        8, "carries a WSL device's Herdr connection; bundle 3 moves panes to the node"),
    "hide-node-link/src/device.rs": (
        2, "names a device's Herdr connection in the link contract; bundle 3 moves panes to the node"),
    "hide-host/src/pane_peer.rs": (
        3, "proves a pane's process on a device against its Herdr; bundle 3 proves it with `HIDE_PANE_ID`"),
    # Temporary: the hooks-status node deletes these.
    "hide-agent-hooks/src/bin/hide-agent-hooks.rs": (
        3, "carries a hook report's Herdr failure; hooks-status deletes it"),
    "hide-agent-hooks/src/report.rs": (
        1, "reports a hook to Herdr; hooks-status deletes it"),
    "herdr-core/src/coordination/hook_tokens.rs": (
        5, "restores a hook's tokens on Herdr; hooks-status deletes it"),
}

# Built for tests and never shipped.
FIXTURES = {
    "herdr-core/src/fake_herdr.rs": "the fake Herdr server the core's tests ask",
    "herdr-core/src/bin/hide-fixture.rs": "the e2e fixture binary; the package ships none of herdr-core's binaries",
}


def trees() -> list:
    return [ROOT / tree for tree in RUST_TREES + TS_TREES]


def implementation(relative: str) -> bool:
    return any(
        relative == entry or (entry.endswith("/") and relative.startswith(entry))
        for entry in IMPLEMENTATION
    )


def test_only(path: pathlib.Path) -> bool:
    relative = path.relative_to(ROOT).as_posix()
    name = path.name
    return (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or "/tests/" in f"/{relative}"
        or re.search(r"\.(test|spec)\.tsx?$", name) is not None
        or "/__tests__/" in f"/{relative}"
    )


def blank(match: re.Match) -> str:
    return re.sub(r"[^\n]", " ", match.group(0))


def strip_comments(text: str) -> str:
    """Blanks comments, keeping line numbers and string literals: a Herdr
    program is often named by the variable a string literal reads."""
    pattern = re.compile(
        r"(?<![A-Za-z0-9_])b?r(#*)\".*?\"\1"
        r"|b?\"(?:\\.|[^\"\\])*\""
        r"|/\*.*?\*/"
        r"|//[^\n]*",
        re.S,
    )
    return pattern.sub(
        lambda match: match.group(0) if match.group(0).lstrip("br#").startswith('"') else blank(match),
        text,
    )


def strip_literals(text: str) -> str:
    """Blanks string literals too, so a path in a message is not code."""
    pattern = re.compile(
        r"b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^'\\\n])'"
        r"|(?<![A-Za-z0-9_])b?r(#*)\".*?\"\1"
        r"|b?\"(?:\\.|[^\"\\])*\"",
        re.S,
    )
    return pattern.sub(blank, text)


def test_modules() -> set:
    """Module files a parent declares behind `#[cfg(test)]`."""
    found = set()
    declaration = re.compile(
        r"#\[cfg\((?:all\()?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
    )
    for tree in trees():
        for parent in tree.rglob("*.rs"):
            text = strip_literals(strip_comments(parent.read_text()))
            directory = (
                parent.parent if parent.name in ("lib.rs", "mod.rs", "main.rs") else parent.with_suffix("")
            )
            for match in declaration.finditer(text):
                for candidate in (directory / f"{match.group(1)}.rs", directory / match.group(1) / "mod.rs"):
                    if candidate.exists():
                        found.add(candidate.relative_to(ROOT).as_posix())
    return found


def strip_test_items(text: str) -> str:
    """Blanks every item behind `#[cfg(test)]`, keeping line numbers."""
    out = list(text)
    for match in re.finditer(r"#\[cfg\((all\()?test\b[^\]]*\]", text):
        start, position, depth, end = match.start(), match.end(), 0, None
        while position < len(text):
            char = text[position]
            if char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    end = position + 1
                    break
            elif char == ";" and depth == 0:
                end = position + 1
                break
            position += 1
        if end is None:
            continue
        for index in range(start, end):
            if out[index] != "\n":
                out[index] = " "
    return "".join(out)


def program_context(lines: list, index: int) -> str:
    """The start's line, the two after it, and the lines before it back to
    the signature of the function it is in, at most `PROGRAM_CONTEXT`."""
    before = []
    for back in range(index - 1, max(-1, index - 1 - PROGRAM_CONTEXT), -1):
        before.append(lines[back])
        if re.search(r"\bfn\s+\w+", lines[back]):
            break
    if re.search(r"\bfn\s+\w+", lines[index]):
        before = []
    return " ".join(list(reversed(before)) + lines[index:index + 3])


def rust_findings(relative: str, text: str) -> list:
    production = strip_test_items(strip_comments(text))
    code = strip_literals(production).splitlines()
    named = production.splitlines()
    found = []
    for number, line in enumerate(code, 1):
        for pattern, advice in RUST_BANNED:
            if re.search(pattern, line):
                found.append(f"{relative}:{number}: {line.strip()}\n    {advice}")
        if PROCESS_START.search(line) and HERDR_PROGRAM.search(program_context(named, number - 1)):
            found.append(f"{relative}:{number}: {line.strip()}\n    {LAUNCH_ADVICE}")
    return found


def ts_findings(relative: str, text: str) -> list:
    code = strip_comments(text).splitlines()
    found = []
    for number, line in enumerate(code, 1):
        for pattern, advice in TS_BANNED:
            if re.search(pattern, line):
                found.append(f"{relative}:{number}: {line.strip()}\n    {advice}")
    return found


def sources() -> list:
    found = []
    for tree in trees():
        for pattern in ("*.rs", "*.ts", "*.tsx"):
            found.extend(tree.rglob(pattern))
    return sorted(found)


def main() -> int:
    failures = []
    for listed in sorted({**ALLOWED, **FIXTURES}):
        if not (ROOT / listed).exists():
            failures.append(f"{listed}: listed as allowed but no longer exists; remove the entry")
    tests = test_modules()
    for path in sources():
        relative = path.relative_to(ROOT).as_posix()
        if (
            implementation(relative)
            or test_only(path)
            or relative in tests
            or relative in FIXTURES
        ):
            continue
        text = path.read_text()
        found = rust_findings(relative, text) if path.suffix == ".rs" else ts_findings(relative, text)
        if relative in ALLOWED:
            allowed, reason = ALLOWED[relative]
            if not found:
                failures.append(
                    f"{relative}: listed as allowed ({reason}) but no longer speaks Herdr; remove the entry"
                )
            elif len(found) != allowed:
                failures.append(
                    f"{relative}: listed with {allowed} finding(s) ({reason}) but has {len(found)}; "
                    "move a new one behind the engine boundary, or change the count with its reason\n"
                    + "\n".join(found)
                )
            continue
        failures.extend(found)
    if failures:
        print("Code outside the engine boundary speaks Herdr directly (PRD engine-bundle-1 D-12):", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        print(f"{len(failures)} finding(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
