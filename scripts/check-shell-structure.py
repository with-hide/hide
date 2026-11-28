#!/usr/bin/env python3
"""Text the web shell, the daemon and the agent notes must keep saying, read
as text so no Rust test has to build to read it (docs/TESTING.md, Structure
checks).

These checks used to be tests inside `hide-core`, which read `web/src` and
`hided/src` from the crate's test binary. A change to the shell then planned
and built the whole crate (about 5,500 tests) to run eight text scans.
`scripts/ci-plan.py` plans the `policy` lane for the files read here instead.

Each check fails when the text it guards breaks:

- no list of agent state names remains in the shell (the retired
  `unseen_completion`, or a literal array that names both `blocked` and
  `question`): the core derives the status, and the shell draws it;
- the pane grid draws only the layout the snapshot carries, and no uniform
  grid stand-in is back;
- one module makes terminal instances and parks a left tab's in a hidden lot,
  so a tab switch never recreates one;
- no position-derived or reverse-parsed tab label path is back in the shell;
- `AGENTS.md` still states the view authority, the announcement rule and the
  performance guide's vocabulary;
- one place in the daemon creates the core, and it is the owner-thread
  wrapper;
- every practice chapter the shell offers is named in
  `PracticeChapter::of`, and every What's new highlight from 1.2.11 on names a
  `measure` this build sends (`product_analytics::measures`): both are Rust
  values compared with web text, so the Rust side is read as text here too.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

SHELL = "web/src"
# The first release whose highlights must name how their use is counted.
FIRST_MEASURED_VERSION = (1, 2, 11)


def sources(root, relative, extensions, tests=False):
    """`(path relative to root, text)` of every source file below `relative`,
    sorted; a `*.test.*` file only when `tests` asks for it."""
    base = root / relative
    if not base.is_dir():
        raise SystemExit(f"{relative} is not a folder under {root}")
    found = []
    for path in sorted(base.rglob("*")):
        if not path.is_file() or path.suffix.lstrip(".") not in extensions:
            continue
        if not tests and ".test." in path.name:
            continue
        found.append((path.relative_to(root).as_posix(), path.read_text()))
    return found


def check_no_state_name_array(root):
    """The shell keeps no list of agent state names of its own.

    Five copies of that vocabulary had drifted apart, which is how the pet
    dashboard and the sidebar came to disagree about whether a finished agent
    needs attention. Tests are read too: a fixture that spells the retired
    state keeps the old vocabulary alive.
    """
    files = sources(root, SHELL, {"ts", "tsx"}, tests=True)
    if not files:
        return [f"{SHELL}: no shell sources were scanned"]
    offenders = []
    for name, text in files:
        if "unseen_completion" in text:
            offenders.append(f"{name}: the retired unseen_completion state")
        rest = text
        while (open_at := rest.find("[")) != -1:
            after = rest[open_at + 1:]
            close = after.find("]")
            if close == -1:
                break
            span = after[:close]
            if '"blocked"' in span and '"question"' in span:
                offenders.append(f"{name}: a literal agent state array")
            rest = after[close + 1:]
    return [f"the shell still keeps its own agent state vocabulary: {offender}" for offender in offenders]


def check_no_uniform_grid_stand_in(root):
    """The pane grid draws the layout the snapshot carries for the tab, or
    nothing: an even grid the engine never applied would decide the PTY size."""
    files = sources(root, SHELL, {"ts", "tsx"})
    grid = dict(files).get(f"{SHELL}/PaneGrid.tsx")
    if grid is None:
        return [f"{SHELL}/PaneGrid.tsx: the pane grid is missing"]
    problems = []
    if "layoutForTab(" not in grid or "!layout" not in grid:
        problems.append(f"{SHELL}/PaneGrid.tsx no longer draws only the layout the snapshot carries")
    for name, text in files:
        if "uniformItems" in text or "uniformGrid" in text:
            problems.append(f"{name}: a uniform pane grid stand-in is back")
    return problems


def check_terminals_are_retained(root):
    """Every attached pane's terminal instance lives in a hidden parking lot
    and is re-parented on return, and only that module makes an instance."""
    files = sources(root, SHELL, {"ts", "tsx"})
    module = f"{SHELL}/terminals.ts"
    terminals = dict(files).get(module)
    if terminals is None:
        return [f"{module}: the terminal instance module is missing"]
    problems = []
    if "function parkingLot(" not in terminals or "hidden = true" not in terminals:
        problems.append(f"{module} no longer parks a left tab's instances in a hidden lot")
    for name, text in files:
        if name != module and "new Terminal(" in text:
            problems.append(
                f"{name}: a terminal instance is made outside the parking module, "
                "so a tab switch can recreate it"
            )
    return problems


def check_tab_label_has_one_owner(root):
    """The core decides the next tab label and the shell draws it: no
    position-derived label, and no parsing of the number back out of a label
    the core formatted, which writes the `Tab N` convention down twice."""
    problems = []
    for name, text in sources(root, SHELL, {"ts", "tsx"}):
        if "fallbackIndex" in text or "displayLabel" in text:
            problems.append(f"{name}: a position-derived tab label path is back")
        if 'startsWith("tab ")' in text or 'startsWith("Tab ")' in text:
            problems.append(f"{name}: a reverse-parsed tab label path is back")
    return problems


NOTES_ARCHITECTURE = [
    # The boundary itself, both halves of it.
    "visible tab",
    "keyboard focus pane",
    # The paths a core-owned value can take.
    "pending",
    "followed",
    "refusal",
    # What replaced the per-change read and the second core.
    "once per burst",
    "before** it takes the lock",
    "creates the core once",
]
NOTES_PERFORMANCE = ["serialize_snapshot_delta", "ChangeNotifier", "idle", "driven"]


def check_agent_notes(root):
    """The agent notes are what the next person reads before they touch the
    runtime, so the claims that matter are asserted, not trusted."""
    path = root / "AGENTS.md"
    if not path.is_file():
        return ["AGENTS.md: the agent notes are missing"]
    notes = path.read_text()
    if "## Runtime Architecture" not in notes:
        return ["AGENTS.md: a Runtime Architecture section is missing"]
    rest = notes.split("## Runtime Architecture", 1)[1]
    if "\n## " not in rest:
        return ["AGENTS.md: Runtime Architecture must be followed by another section"]
    architecture = rest.split("\n## ", 1)[0]
    if "## Performance Guide" not in notes:
        return ["AGENTS.md: a Performance Guide section is missing"]
    performance = notes.split("## Performance Guide", 1)[1]
    problems = []
    for name, section, wanted in (
        ("Runtime Architecture", architecture, NOTES_ARCHITECTURE),
        ("Performance Guide", performance, NOTES_PERFORMANCE),
    ):
        for phrase in wanted:
            if phrase not in section:
                problems.append(f"AGENTS.md {name} does not say {phrase!r}")
    if "main thread spent 47% of wall time" in performance:
        problems.append(
            "AGENTS.md Performance Guide keeps the stale 47% figure; "
            "measurements carry their load instead"
        )
    return problems


CORE_CREATIONS = ("Core::create(", "Core::create_with_product_analytics(")
CORE_OWNER = "hided/src/core.rs"


def check_core_is_created_once(root):
    """A launch used to create a core, start its session sync, and then throw
    both away for a second core once the runtime was known. One core per
    daemon means one `session.snapshot`, one subscription and one catalog
    build, and the daemon is the only place that can put the second back."""
    creations = []
    for name, text in sources(root, "hided/src", {"rs"}):
        for line in map(str.strip, text.splitlines()):
            if line.startswith("//"):
                continue
            if any(call in line for call in CORE_CREATIONS):
                creations.append(f"{name}: {line}")
    if len(creations) != 1:
        return [f"the daemon must create the core from one place: {creations}"]
    if not creations[0].startswith(CORE_OWNER):
        return [f"the one creation belongs to the owner-thread wrapper {CORE_OWNER}: {creations}"]
    return []


class ShapeError(Exception):
    """The Rust source no longer has the shape this script reads; it never guesses."""


def strip_rust_comments(text):
    """`text` without `//` and `/* */` comments, so a commented-out arm or entry
    is not one that Rust sees. String literals are kept whole."""
    out, i, n = [], 0, len(text)
    while i < n:
        two = text[i:i + 2]
        if two == "//":
            while i < n and text[i] != "\n":
                i += 1
        elif two == "/*":
            end = text.find("*/", i + 2)
            i = n if end == -1 else end + 2
        elif text[i] == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        else:
            out.append(text[i])
            i += 1
    return "".join(out)


def rust_block(text, start, what):
    """The text from `start` to the first closing brace in column 0."""
    begin = text.find(start)
    if begin == -1:
        raise ShapeError(f"{ANALYTICS}: {what} is missing ({start!r})")
    end = text.find("\n}\n", begin)
    return text[begin:end if end != -1 else len(text)]


def practice_chapters(analytics):
    """The ids `PracticeChapter::of` names, each by an arm that returns a chapter
    and is not commented out, ending in the arm that names none."""
    block = rust_block(analytics, "impl PracticeChapter {", "PracticeChapter::of")
    if not re.search(r"_\s*=>\s*return None\s*,", block):
        raise ShapeError(f"{ANALYTICS}: PracticeChapter::of no longer ends in `_ => return None`")
    return set(re.findall(r'"([\w-]+)"\s*=>\s*Self::\w+\s*,', block))


# What `measures()` does besides listing events: one measure per feature in
# `Feature::ALL`, named by `Feature::name`. Any other shape is refused.
FEATURE_EXTENSION = re.compile(
    r'measures\.extend\(\s*Feature::ALL\s*\.iter\(\)\s*'
    r'\.map\(\|feature\|\s*format!\(\s*"hide_feature_used:\{\}"\s*,\s*feature\.name\(\)\s*\)\s*\)\s*,?\s*\)\s*;'
)


def measures(analytics):
    """What `product_analytics::measures()` returns: the events its array lists
    and `hide_feature_used:` and the name of each feature in `Feature::ALL`."""
    block = rust_block(analytics, "pub fn measures()", "measures()")
    listed = re.search(r"let mut measures: Vec<String> = \[(.*?)\]\s*\.map\(str::to_owned\)\s*\.to_vec\(\);", block, re.S)
    if listed is None:
        raise ShapeError(f"{ANALYTICS}: measures() no longer starts from an array mapped with `str::to_owned`")
    if not FEATURE_EXTENSION.search(block):
        raise ShapeError(f"{ANALYTICS}: measures() no longer extends the list with `hide_feature_used:` and each `Feature::ALL` name")
    events = set(re.findall(r'"(hide_\w+)"', listed.group(1)))
    feature = rust_block(analytics, "impl Feature {", "Feature")
    all_variants = re.search(r"const ALL: \[Self; \d+\] = \[(.*?)\];", feature, re.S)
    if all_variants is None:
        raise ShapeError(f"{ANALYTICS}: Feature::ALL is missing")
    variants = re.findall(r"Self::(\w+)", all_variants.group(1))
    names = dict(re.findall(r'Self::(\w+)\s*=>\s*"([\w-]+)"\s*,', feature.split("fn name", 1)[-1]))
    unnamed = [variant for variant in variants if variant not in names]
    if unnamed:
        raise ShapeError(f"{ANALYTICS}: Feature::name does not name {unnamed}")
    return events | {f"hide_feature_used:{names[variant]}" for variant in variants}


def analytics_source(root):
    return strip_rust_comments((root / ANALYTICS).read_text())


ANALYTICS = "hide-core/src/product_analytics.rs"


def check_practice_chapters_are_named(root):
    """A chapter added under `web/src/practice/chapters/` without its name in
    `PracticeChapter::of` would count nothing (CONTRIBUTING.md, A main feature
    brings its practice, its scene and its measure)."""
    try:
        named = practice_chapters(analytics_source(root))
    except ShapeError as error:
        return [str(error)]
    folder = root / SHELL / "practice" / "chapters"
    ids = {}
    for path in sorted(folder.glob("*.ts")):
        if path.name.endswith(".test.ts"):
            continue
        found = next(
            (line[len('  id: "'):].split('"')[0]
             for line in path.read_text().splitlines() if line.startswith('  id: "')),
            None,
        )
        if found is None:
            return [f"{path.name} names no chapter id"]
        ids[path.name] = found
    if len(ids) < 8:
        return [f"{folder.relative_to(root)} holds {len(ids)} chapters, expected at least 8: {sorted(ids.values())}"]
    return [
        f"practice chapter `{chapter}` ({name}) has no name in PracticeChapter::of"
        for name, chapter in ids.items() if chapter not in named
    ]


def version_of(text):
    return tuple(int(part) for part in text.split("."))


def check_highlights_name_a_measure(root):
    """Every main feature the What's new window shows from 1.2.11 on names how
    its use is counted, and the measure is one this build sends, so a main
    feature cannot ship without one."""
    try:
        sent = measures(analytics_source(root))
    except ShapeError as error:
        return [str(error)]
    releases = json.loads((root / SHELL / "releases.json").read_text())
    problems = []
    checked = 0
    for entry in releases["versions"]:
        name = entry["version"]
        if version_of(name) < FIRST_MEASURED_VERSION:
            continue
        for highlight in entry["highlights"]:
            measure = highlight.get("measure")
            if not isinstance(measure, str):
                problems.append(f"a {name} highlight ({highlight.get('drawing')}) names no `measure`")
            elif measure not in sent:
                problems.append(f"{name}: `{measure}` is not a measure this build sends: {sorted(sent)}")
            checked += 1
    if checked == 0:
        problems.append("no highlight from 1.2.11 on was checked")
    return problems


CHECKS = (
    check_no_state_name_array,
    check_no_uniform_grid_stand_in,
    check_terminals_are_retained,
    check_tab_label_has_one_owner,
    check_agent_notes,
    check_core_is_created_once,
    check_practice_chapters_are_named,
    check_highlights_name_a_measure,
)


def main():
    failed = False
    for check in CHECKS:
        for problem in check(ROOT):
            failed = True
            print(f"{check.__name__}: {problem}", file=sys.stderr)
    if failed:
        return 1
    print(f"shell structure: {len(CHECKS)} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
