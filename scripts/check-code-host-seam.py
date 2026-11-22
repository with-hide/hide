#!/usr/bin/env python3
"""A code host's CLI, its addresses and its key stay inside its registration
unit (docs/ARCHITECTURE.md, Code hosts).

Common code asks the registry (`CodeHosts`) and the unit (`CodeHost`) and
receives a `Place`, a `HostFailure` and a `ChangeRequestRef`; the screen opens
the addresses the core carries. This check reads the sources and fails when,
outside the files named below as a host's implementation or a parser it owns:

- Rust names a host's CLI call (`Call::Gh`) or its failure and place types
  (`GhFailure`, `github::Where`);
- Rust or TypeScript spells a host's address (`github.com`, a change request's
  `/pull/` path or its `/checks` and `/files` pages, a `/commit/` page);
- Rust branches on a host's key (`"github"` compared, matched or looked up).

Test code is not read: a Rust file's `#[cfg(test)]` module and everything
after it, `tests/` folders, `*_tests.rs` files and TypeScript `*.test.*`
files hold fixtures, which may spell any address.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# A host's implementation and the parsers it owns: the only places that may
# spell its CLI, its addresses and its key.
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

RUST_RULES = (
    ("a host's CLI call", re.compile(r"\bCall::(Gh|Glab)\b")),
    ("a host's failure or place type", re.compile(r"\bGhFailure\b|\bgithub::Where\b|\bGlabAnswer\b")),
    (
        "a host's address",
        re.compile(
            r"github\.com|gitlab\.com|/-/merge_requests/|/(pull|issues|commit)/(\{|\"|\d)"
            r"|/checks\"|/files\"|/pipelines\"|/diffs\""
        ),
    ),
    (
        "a branch on a host's key",
        re.compile(
            r"==\s*\"(github|gitlab)\"|\"(github|gitlab)\"\s*(=>|==)|\.host\(\"(github|gitlab)\"\)"
        ),
    ),
)

WEB_RULES = (
    (
        "a host's address",
        re.compile(
            r"github\\?\.com|gitlab\\?\.com|\\?/-\\?/merge_requests|\\?/(pull|issues)\\?/"
            r"|[`\"]/checks[`\"]|/checks`|[`\"]/files[`\"]|/files`"
        ),
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
    "web/src/pullRequestIdentity.ts": "gitlab-screens compares the carried reference",
    "web/src/actions.ts": "gitlab-screens opens the repository page the core carries",
    "web/src/PrCard.tsx": "gitlab-screens opens the reference's checks and files pages",
    "web/src/prCardModel.ts": "gitlab-screens opens the reference's files page",
    # Issues stay GitHub's in this plan (a GitHub issue key and its page).
    "web/src/PrLinks.tsx": "issue keys stay GitHub's",
    "hide-core/src/issues.rs": "issue references stay GitHub's",
    "web/src/SuggestionsTab.tsx": "gitlab-screens reads the suggestion's carried reference",
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
                for what, pattern in rules:
                    if pattern.search(line):
                        found.append(f"{relative}:{number}: {what}: {line.strip()}")
    return found


def main():
    found = scan(RUST_ROOTS, {".rs"}, rust_lines, RUST_RULES)
    found += scan(WEB_ROOTS, {".ts", ".tsx"}, web_lines, WEB_RULES)
    if found:
        print("A code host's CLI, address or key outside its unit:", file=sys.stderr)
        for line in found:
            print(f"  {line}", file=sys.stderr)
        print(
            "Ask the registry and the unit (hide-core/src/code_host.rs) or open the address the"
            " core carries; see docs/ARCHITECTURE.md, Code hosts.",
            file=sys.stderr,
        )
        return 1
    print("code host seam: no host CLI, address or key outside its unit")
    return 0


if __name__ == "__main__":
    sys.exit(main())
