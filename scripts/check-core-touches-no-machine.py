#!/usr/bin/env python3
"""The core decides and keeps state; it never touches a machine (PRD
core-host-node D-21). Files, processes, the Herdr socket and session files
are a node's, reached through `NodeLink`; this check fails when production
code under `herdr-core/src`, or under `hide-factory/src`, the Software
Factory engine the core runs (D-01), reaches one directly.

Test code is exempt: test-only files, a module file its parent declares
behind `#[cfg(test)]`, and every item behind `#[cfg(test)]`. The core's own
stores are excused by name below, each with the reason it is the core's and
not a machine fact, and so is what a later layer of the PRD moves out, named
with that layer; nothing else may join either list without a reason. Each
excused file carries the number of machine touches it has, so a new one in
an excused file fails like one anywhere else.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORE = ROOT / "herdr-core" / "src"


def scanned() -> list:
    """The production trees the core runs: its own, and the Factory engine's."""
    return [CORE, ROOT / "hide-factory" / "src"]

# What reaching a machine looks like, with what to do instead.
BANNED = [
    (r"\b(ureq|reqwest|hyper|attohttpc|isahc|curl)\b",
     "uses an HTTP client; ask the core own node instead"),
    (r"\bstd::process\b(?!::id\(\))|\bCommand::new\b|\bprocess::Command\b",
     "starts a process; ask a node (`Call::*`) instead"),
    (r"\bstd::fs\b|(?<![\w:])fs::|\bFile::(open|create)\b|\bOpenOptions\b",
     "touches a file; ask a node, or keep it in one of the core's stores"),
    (r"\.(exists|is_file|is_dir|canonicalize|metadata|symlink_metadata|read_dir|read_link)\(\)",
     "reads the file system; ask a node"),
    (r"\bcap_std\b", "opens a directory capability; ask a node"),
    (r"\bhide_host::", "calls node code in process; go through `NodeLink`"),
    (r"\bhide_platform::(process|watch|ipc|fs)\b",
     "uses the platform layer's machine access; that is a node's"),
    (r"\bhide_project::(resolve|facts|git)\b",
     "reads a folder; ask a node for `Call::Project`"),
    (r"\bhide_session::(SessionCatalog|SessionCursor|read_bounded|search_read)\b"
     r"|\bhide_session::(links::(candidates|read)|label_transcript::read|session_activity::read)\b",
     "reads session files; ask the node that holds them"),
    (r"\blibc::", "calls the operating system; that is a node's"),
    (r"\bUnixStream\b|\bTcpStream\b|\bLocalSocketConnector\b",
     "opens a socket; a node holds the Herdr connection"),
    (r"\bstd::net\b|\bUnixListener\b|\bTcpListener\b",
     "listens on the network; that is a node's"),
    (r"\btokio::(fs|process|net)\b", "reaches the machine through tokio; ask a node"),
    (r"\.(try_exists|is_symlink)\(\)", "reads the file system; ask a node"),
    (r"\b(std::)?env::(current_exe|current_dir|set_current_dir|home_dir)\b",
     "reads where this process runs; that is a node's"),
    (r"\brussh(_config|_sftp)?\b",
     "speaks SSH; a device is reached through `hide_node_link::device`"),
    (r"\bhide_ai::settings::(load|save)\b",
     "reads or writes the AI choice file; keep it in one of the core's stores"),
]

# The core's own stores and their plumbing: state the core owns and moves
# with it (PRD core-host-node D-10), never a fact of the machine it runs on.
# Each entry is the number of machine touches the file has and why.
STORES = {
    "herdr-core/src/persistence.rs": (
        13,
        "the core's UI state file, including durable directory/write receipts "
        "with the durable writer's own access type, the state folder's identity "
        "that keeps its directory receipt once per folder, and explicit "
        "write-error variants before sleeping-session effects, and a moved copy "
        "of it read to name what this build cannot read",
    ),
    "herdr-core/src/workspace_views.rs": (6, "the core's saved View layouts"),
    "herdr-core/src/delivery/ledger.rs": (
        20,
        "the core's delivery ledger, written with the durable writer's own access type",
    ),
    "herdr-core/src/labels/store.rs": (3, "the core's agent label store"),
    "herdr-core/src/labels/import.rs": (1, "reads the core's own earlier label store once"),
    "herdr-core/src/links/store.rs": (6, "the core's link graph store"),
    "herdr-core/src/usage_ledger/store.rs": (1, "the core's usage ledger and retained daily totals"),
    "herdr-core/src/local_issues.rs": (5, "the core's local issue store"),
    "herdr-core/src/latest_file.rs": (
        4,
        "the write and read of a core store that keeps only its latest value "
        "(the saved GitHub answers, the panes held as a draft)",
    ),
    "herdr-core/src/diagnostics.rs": (7, "the core's own log file"),
    "herdr-core/src/labels/mod.rs": (
        1,
        "reads the operator's AI choice, a core store beside the core's "
        "state file (`hide_ai::settings`)",
    ),
    "herdr-core/src/session_sync/coordinator.rs": (
        2,
        "reads and writes the operator's AI choice, a core store beside the "
        "core's state file (`hide_ai::settings`)",
    ),
    "herdr-core/src/factory.rs": (
        2,
        "opens the Factory's store when one exists and reads the operator's "
        "AI choice, both core stores beside the core's state file",
    ),
    "hide-factory/src/store.rs": (
        13,
        "the Factory's store, which the core keeps beside its state file, and"
        " the one copy of a schema 1 store its migration keeps beside it",
    ),
    "hide-factory/src/engine.rs": (
        1,
        "reads a Task's attached PRD, a file in the Factory's own store, "
        "in `attached_prd`, which every engine reader of it calls",
    ),
    "herdr-core/src/node_migration.rs": (
        15,
        "converts the core's own stores to node ids once, where they were "
        "written, before any link exists (PRD core-host-node D-10)",
    ),
    "herdr-core/src/node_migration/copy.rs": (
        58,
        "copies, places and sets aside the core's own stores for a move of "
        "the core (PRD core-host-node-move), run by the hided whose state "
        "folder holds them while no core runs on it; the retired "
        "session-search store moves read-only, kept for its retention "
        "settings (PRD overview-device-tasks D-20)",
    ),
    "herdr-core/src/node_migration/reown.rs": (
        4,
        "converts a moved copy of the core's own link and Factory stores to "
        "the receiving node's ids, before a core starts on it, and carries "
        "the retired session-search store's retention to them (PRD "
        "overview-device-tasks D-20)",
    ),
    "herdr-core/src/memory_hook.rs": (
        1,
        "looks whether the core's Memory store exists before asking a node, "
        "a core store beside the core's state file",
    ),
}

# What a later layer of the PRD moves to a node, with that layer. Each entry
# goes in the change that moves it; the check fails once a listed file no
# longer reaches a machine, so a stale entry cannot linger.
LATER_LAYERS = {}

# Fixtures that are built for tests and never shipped.
FIXTURES = {
    "herdr-core/src/bin/hide-fixture.rs": "the e2e fixture binary; the package ships none of herdr-core's binaries",
}


def test_only(path: pathlib.Path) -> bool:
    tree = next(root for root in scanned() if path.is_relative_to(root))
    relative = path.relative_to(tree).as_posix()
    name = path.name
    return (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or "/tests/" in f"/{relative}"
    )


def blank(match: re.Match) -> str:
    return re.sub(r"[^\n]", " ", match.group(0))


def strip_comments_and_literals(text: str) -> str:
    """Blanks comments, string literals and character literals, keeping
    line numbers, so a brace in a string cannot end an item early and a path
    in a message is not code."""
    pattern = re.compile(
        r"/\*.*?\*/"
        r"|//[^\n]*"
        r"|b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^'\\\n])'"
        r"|(?<![A-Za-z0-9_])b?r(#*)\".*?\"\1"
        r"|b?\"(?:\\.|[^\"\\])*\"",
        re.S,
    )
    return pattern.sub(blank, text)


def test_modules() -> set:
    """Module files a parent declares behind `#[cfg(test)]` (or a cfg that
    requires test), which only a test build compiles."""
    found = set()
    declaration = re.compile(
        r"#\[cfg\((?:all\()?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
    )
    for parent in (path for tree in scanned() for path in tree.rglob("*.rs")):
        text = strip_comments_and_literals(parent.read_text())
        directory = parent.parent if parent.name in ("lib.rs", "mod.rs", "main.rs") else parent.with_suffix("")
        for match in declaration.finditer(text):
            name = match.group(1)
            for candidate in (directory / f"{name}.rs", directory / name / "mod.rs"):
                if candidate.exists():
                    found.add(candidate.relative_to(ROOT).as_posix())
    return found


def strip_test_items(text: str) -> str:
    """Blanks every item behind `#[cfg(test)]`, keeping line numbers."""
    out = list(text)
    for match in re.finditer(r"#\[cfg\((all\()?test\b[^\]]*\]", text):
        start = match.start()
        position = match.end()
        depth = 0
        end = None
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


def findings(path: pathlib.Path) -> list:
    relative = path.relative_to(ROOT).as_posix()
    text = strip_test_items(strip_comments_and_literals(path.read_text()))
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        for pattern, advice in BANNED:
            if re.search(pattern, line):
                found.append(f"{relative}:{number}: {line.strip()}\n    {advice}")
    return found


def main() -> int:
    failures = []
    excused = {**STORES, **LATER_LAYERS}
    for listed in sorted({**excused, **FIXTURES}):
        if not (ROOT / listed).exists():
            failures.append(f"{listed}: listed as excused but no longer exists; remove the entry")
    tests = test_modules()
    for path in sorted(path for tree in scanned() for path in tree.rglob("*.rs")):
        relative = path.relative_to(ROOT).as_posix()
        if test_only(path) or relative in tests or relative in FIXTURES:
            continue
        found = findings(path)
        if relative in excused:
            allowed, reason = excused[relative]
            if not found:
                failures.append(
                    f"{relative}: listed as excused ({reason}) but reaches no machine; remove the entry"
                )
            elif len(found) != allowed:
                failures.append(
                    f"{relative}: listed with {allowed} machine touch(es) ({reason}) but has "
                    f"{len(found)}; move a new one to a node, or change the count with its reason\n"
                    + "\n".join(found)
                )
            continue
        failures.extend(found)
    if failures:
        print("The core reaches the machine directly (PRD core-host-node D-21):", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        print(f"{len(failures)} finding(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
