#!/usr/bin/env bash
# An agent's account folders are named in one place: its definition's
# `[account]` (`hide-agent-adapter/agents/*.toml`), which
# `hide_agent_adapter::resolve` reads for every caller (docs/ARCHITECTURE.md,
# Agent profiles and accounts). This check fails when production Rust names
# one of those folders anywhere else, so a reader or an installer cannot
# fix an agent's folder again and miss the account a profile runs in.
#
# What it reads: every string literal of production Rust (comments, test
# files, a module behind `#[cfg(test)]` and every item behind it are left
# out, as `scripts/rust_scan.py` reads them). What it allows: a module that
# is one agent's own (its file or folder is named after the agent), which
# handles what only that agent does with the folder the account names; and
# the files listed in ALLOWED below, each with its reason.
#
# Usage: scripts/check-agent-folders.sh [root]   (root defaults to the checkout)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="${1:-$here/..}"

exec python3 - "$here" "$root" <<'PY'
import pathlib
import re
import sys

sys.path.insert(0, sys.argv[1])
import rust_scan  # noqa: E402

ROOT = pathlib.Path(sys.argv[2]).resolve()

# The folders the seven agents keep an account in, as a path names them:
# Claude Code's, Codex's, pi's, omp's, Grok's and agy's folders, and
# OpenCode's config and data folders and its database. A retired agent's
# folders (`.gemini` alone, `.cursor`) are not an account's.
FOLDER = re.compile(
    r"(?:^|[/\\])(?:"
    r"\.claude(?:$|[/\\.])"
    r"|\.codex(?:$|[/\\])"
    r"|\.pi(?:$|[/\\])"
    r"|\.omp(?:$|[/\\])"
    r"|\.grok(?:$|[/\\])"
    r"|\.gemini[/\\](?:config|antigravity-cli)"
    r"|\.config[/\\]opencode"
    r"|\.local[/\\]share[/\\]opencode"
    r")"
    r"|opencode\.db"
)

# A module that is one agent's own: its file, or a folder it sits in, is
# named after the agent.
AGENT_MODULE = re.compile(
    r"^(?:claude|codex|opencode|pi|omp|grok|antigravity|agy|cursor)(?:[_.-].*)?$"
)

# Files that may name an agent's folder, each with why.
ALLOWED = {
    "hide-agent-adapter/src/declarations.rs":
        "each agent's Rust declaration: the documented install routes an "
        "update is told apart by and the configuration files a journey "
        "watches, which are the vendor's install layout, not an account's places",
    "hide-agent-adapter/src/journey.rs":
        "the journey's fake home: Grok's stop observer file, which the "
        "journey writes into the fake account it lays out",
}

# Folders whose Rust is not Hide's own production code.
SKIPPED = {"target", "node_modules", "vendor", "agents", ".git", "dist", "out"}


def rust_files():
    for path in sorted(ROOT.rglob("*.rs")):
        relative = path.relative_to(ROOT)
        if SKIPPED.intersection(relative.parts[:-1]):
            continue
        yield relative.as_posix(), path


def agent_module(relative: str) -> bool:
    parts = relative.split("/")
    names = [pathlib.PurePosixPath(parts[-1]).stem] + parts[:-1]
    return any(AGENT_MODULE.match(name) for name in names if name not in ("src", "mod"))


def without_test_items(kept: str, blanked: str) -> str:
    """`kept` with every item behind `#[cfg(test)]` blanked: the items are
    found in `blanked`, where every literal is blanked too, so a brace in a
    string cannot end one early (`rust_scan.strip_test_items`'s reading)."""
    out = list(kept)
    for match in re.finditer(r"#\[cfg\((all\()?test\b[^\]]*\]", blanked):
        position, depth, end = match.end(), 0, None
        while position < len(blanked):
            char = blanked[position]
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
        for index in range(match.start(), end if end is not None else match.start()):
            if out[index] != "\n":
                out[index] = " "
    return "".join(out)


LITERAL = re.compile(r'(?<![A-Za-z0-9_])b?r(#*)"(.*?)"\1|b?"((?:\\.|[^"\\])*)"', re.S)


def main() -> int:
    trees = [ROOT]
    tested = rust_scan.test_modules(ROOT, trees)
    failures = []
    for relative, path in rust_files():
        if rust_scan.test_file(relative) or relative in tested:
            continue
        if relative in ALLOWED or agent_module(relative):
            continue
        source = path.read_text(errors="replace")
        text = without_test_items(
            rust_scan.strip(source, keep_literals=True), rust_scan.strip(source)
        )
        for match in LITERAL.finditer(text):
            content = match.group(2) if match.group(2) is not None else match.group(3)
            if FOLDER.search(content):
                line = text.count("\n", 0, match.start()) + 1
                failures.append(f"{relative}:{line}: names an agent's folder: {content!r}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        print(
            "\nAn agent's folders come from its definition's [account] through "
            "hide_agent_adapter::resolve (Account::path, AgentAdapter::place_in); "
            "agent-specific handling belongs in that agent's own module "
            "(scripts/check-agent-folders.sh).",
            file=sys.stderr,
        )
        return 1
    print("agent folders: named only in agent definitions and agent modules")
    return 0


sys.exit(main())
PY
