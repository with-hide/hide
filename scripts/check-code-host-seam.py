#!/usr/bin/env python3
"""Starting a code host's program, its addresses and its key stay inside its
registration unit (docs/ARCHITECTURE.md, Code hosts).

Common code asks the registry (`CodeHosts`) and the unit (`CodeHost`) and
receives a `Place`, a `HostFailure` and a `ChangeRequestRef`; the screen opens
the addresses the core carries. This check reads the sources and fails when,
outside the files named below as a host's implementation or a parser it owns:

- Rust starts a host's program by one of its three doors: constructing its
  node call (`Call::Gh`, `Call::Glab`), naming the program (`"gh"`,
  `"glab"`), or using the node link's module for it (`hide_node_link::gh`,
  `hide_node_link::glab`) beyond the items that start
  nothing (`HOST_MODULE_ITEMS`). A list of argument words that reaches none of
  the doors is dead data and is not read: no operation of a unit takes
  argument words from common code.
- Rust names a host's failure or place type (`GhFailure`, `github::Where`,
  `GlabAnswer`);
- Rust or TypeScript spells a host's address (`github.com`, a change request's
  `/pull/` path or its `/checks` and `/files` pages, a `/commit/` page;
  `gitlab.com`, a merge request's `/-/merge_requests/` path or its
  `/pipelines` and `/diffs` pages);
- Rust spells a host's key (`"github"`, `"gitlab"`) in any form: compared, matched,
  `matches!`, looked up or passed on. The files in `KEY_FILES` name GitHub's
  issues or its store, and the files in `COMPONENT_FILES` name a diagnostic's
  component, each with why and, while it is temporary, the node that ends it.

Test code is not read: a Rust file's `#[cfg(test)]` module and everything
after it, `tests/` folders, `*_tests.rs` files and TypeScript `*.test.*`
files hold fixtures, which may spell any address.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# A host's implementation and the parsers it owns: the only places that may
# start its program, spell its addresses and its key.
IMPLEMENTATION = (
    "hide-core/src/github.rs",
    "hide-core/src/github/",
    "hide-core/src/github_poll.rs",
    # The node side of GitHub's CLI: its allowlist and the runner.
    "hide-node-link/src/gh.rs",
    "hide-host/src/gh.rs",
    "hide-host/src/serve.rs",
    "hide-core/src/gitlab.rs",
    "hide-core/src/gitlab/",
    # The node side of gitlab.com's CLI: its allowlist and the runner.
    "hide-node-link/src/glab.rs",
    "hide-host/src/glab.rs",
)

RUST_ROOTS = ("hide-core/src", "hided/src")
WEB_ROOTS = ("web/src",)

# The node link's module for GitHub's program holds its argument builders,
# allowlist and field lists; these two of its items start nothing: the read
# failure's category and a check that a word is a full commit name.
HOST_MODULE_ITEMS = ("GithubFailureCategory", "is_commit")

# Files that spell a host's key for GitHub's issues, which stay GitHub's in
# this plan, or for GitHub's own store, with why.
KEY_FILES = {
    "hide-core/src/tasks.rs": "an issue's source kind",
    "hide-core/src/project_settings.rs": "the issue source choice's saved name",
    "hide-core/src/runtime/backlog.rs": "a backlog item's issue destination",
    "hide-core/src/github_store.rs": "the GitHub snapshot store's file label",
}

# Files whose diagnostics name their component by a host's key: GitHub's
# own files, which keep it for good. Common code takes the component from
# the host it reads, or `code_host::COMPONENT` for a line about no one host.
COMPONENT_FILES = {
    "hide-core/src/github_store.rs": "GitHub's own snapshot store",
}

# Each rule: what it finds, its pattern, and the files outside the units that
# may still match it.
RUST_RULES = (
    (
        "a host's program started",
        re.compile(r'\bCall::(?:Gh|Glab)\b|\bCall::\{[^}]*\b(?:Gh|Glab)\b|"(?:gh|glab)"'),
        {},
    ),
    (
        "the node link's module for a host's program",
        # The module by its path, in a one-line `use` group, or as a line of
        # a group rustfmt wrapped.
        re.compile(
            r"\bhide_node_link::(?:gh|glab)\b(?!::(?:" + "|".join(HOST_MODULE_ITEMS) + r")\b)"
            r"|\bhide_node_link::\{[^}]*\b(?:gh|glab)\b"
            r"|^\s*(?:gh|glab)(?:\s*[,}]|\s+as\b|::(?!(?:" + "|".join(HOST_MODULE_ITEMS) + r")\b))"
        ),
        {},
    ),
    (
        "a host's failure or place type",
        re.compile(r"\bGhFailure\b|\bgithub::Where\b|\bGlabAnswer\b"),
        {},
    ),
    (
        "a host's address",
        re.compile(
            r"github\.com|gitlab\.com|/-/merge_requests/|/(pull|issues|commit)/(\{|\"|\d)"
            r"|/checks\"|/files\"|/pipelines\"|/diffs\""
        ),
        {},
    ),
    # A diagnostic's component is its own rule below; rustfmt leaves a
    # `json!` body as written, so the field may have no space.
    (
        "a host's key",
        re.compile(r'(?<!"component": )(?<!"component":)"(?:github|gitlab)"'),
        KEY_FILES,
    ),
    (
        "a host's key as a log's component",
        re.compile(r'"component":\s*"(?:github|gitlab)"'),
        COMPONENT_FILES,
    ),
)

WEB_RULES = (
    (
        "a host's address",
        re.compile(
            r"github\\?\.com|gitlab\\?\.com|\\?/-\\?/merge_requests|\\?/(pull|issues)\\?/"
            r"|[`\"]/checks[`\"]|/checks`|[`\"]/files[`\"]|/files`"
        ),
        {},
    ),
)

# Hide's own pages (help, release notes, its guides) live on GitHub whatever
# host a project uses; they are Hide's address, not a project's host.
HIDE_OWN = re.compile(r"github\.com/with-hide/")

# Sample data a demo, a practice world or a scene draws, never read from a
# host, like a test fixture; and addresses that are not a project's host.
SAMPLES = (
    "web/src/practice/",
    "web/src/docs/parts/",
    "web/src/mobile/demoData.ts",
    "web/src/mobile/prFixture.ts",
    "web/src/i18n/resources/whatsNewScenes.ts",
    # A skill's source is a git address the skills format names, not a
    # project's code host.
    "web/src/settings/skillsMcp.ts",
)

# A file that still reads or makes a host's address outside its unit, with
# the mission node that removes it, or why it stays.
KNOWN_FILES = {
    # Issues stay GitHub's in this plan (a GitHub issue key and its page).
    "web/src/PrLinks.tsx": "issue keys stay GitHub's",
    "hide-core/src/issues.rs": "issue references stay GitHub's",
}

# A line a rule matches that is not a host's address in use, with why.
KNOWN_LINES = {
    "hide-core/src/agent_state/work/board.rs": {
        # A GitHub issue's page for a board row whose task holds no address;
        # issues stay GitHub's.
        '"https://github.com/{}/issues/{}",',
    },
    "hided/src/server.rs": {
        # An example of a page a person acts on, in an AI prompt's prose.
        '"Name where the person acts: the page\'s address (github.com/login), the app (1Password) or the setting (System Settings > Privacy & Security > Camera)"',
    },
    "web/src/i18n/resources/projectSettings.ts": {
        # The repository field's hint and refusal: the forms a person types
        # (`owner/repository`, `gitlab.com/group/project`), in each language.
        '"projectSettings.repositoryHint": "owner/repository or gitlab.com/group/project",',
        '"projectSettings.repositoryInvalid": "Enter owner/repository or gitlab.com/group/project, without a URL.",',
        '"projectSettings.repositoryHint": "owner/repository 또는 gitlab.com/group/project",',
        '"projectSettings.repositoryInvalid": "URL 대신 owner/repository 또는 gitlab.com/group/project를 입력하세요.",',
        '"projectSettings.repositoryHint": "owner/repository 或 gitlab.com/group/project",',
        '"projectSettings.repositoryInvalid": "请输入 owner/repository 或 gitlab.com/group/project，而不是 URL。",',
        '"projectSettings.repositoryHint": "owner/repository または gitlab.com/group/project",',
        '"projectSettings.repositoryInvalid": "URLではなく owner/repository または gitlab.com/group/project を入力してください。",',
    },
    "web/src/CloneFromUrl.tsx": {
        # The clone field's placeholder: an example the person replaces.
        'placeholder="https://github.com/user/repo.git"',
    },
}


def is_test_path(relative):
    parts = relative.split("/")
    name = parts[-1]
    return (
        "tests" in parts[:-1]
        or name == "tests.rs"
        or name.endswith("_tests.rs")
        or re.search(r"\.test\.[jt]sx?$", name) is not None
        or "/gallery/" in relative
        or "/e2e/" in relative
        or relative.startswith(SAMPLES)
    )


def rust_lines(text):
    """The lines before a file's test module, with their numbers."""
    for number, line in enumerate(text.splitlines(), start=1):
        if line.strip() == "#[cfg(test)]":
            return
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        yield number, line


def web_lines(text):
    in_block = False
    for number, line in enumerate(text.splitlines(), start=1):
        stripped = line.strip()
        if in_block:
            if "*/" in stripped:
                in_block = False
            continue
        if stripped.startswith("/*"):
            in_block = "*/" not in stripped
            continue
        if stripped.startswith("//") or stripped.startswith("*"):
            continue
        yield number, line


def scan(roots, suffixes, lines_of, rules):
    found = []
    for root in roots:
        for path in sorted((ROOT / root).rglob("*")):
            if path.suffix not in suffixes or not path.is_file():
                continue
            relative = path.relative_to(ROOT).as_posix()
            if is_test_path(relative) or relative.startswith(IMPLEMENTATION) or relative in KNOWN_FILES:
                continue
            text = path.read_text(encoding="utf-8")
            for number, line in lines_of(text):
                if HIDE_OWN.search(line) or line.strip() in KNOWN_LINES.get(relative, ()):
                    continue
                for what, pattern, allowed in rules:
                    if relative not in allowed and pattern.search(line):
                        found.append(f"{relative}:{number}: {what}: {line.strip()}")
    return found


def main():
    found = scan(RUST_ROOTS, {".rs"}, rust_lines, RUST_RULES)
    found += scan(WEB_ROOTS, {".ts", ".tsx"}, web_lines, WEB_RULES)
    if found:
        print("A code host's program, address or key outside its unit:", file=sys.stderr)
        for line in found:
            print(f"  {line}", file=sys.stderr)
        print(
            "Ask the registry and the unit (hide-core/src/code_host.rs) or open the address the"
            " core carries; see docs/ARCHITECTURE.md, Code hosts.",
            file=sys.stderr,
        )
        return 1
    print("code host seam: no host program, address or key outside its unit")
    return 0


if __name__ == "__main__":
    sys.exit(main())
