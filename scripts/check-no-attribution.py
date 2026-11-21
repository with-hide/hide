#!/usr/bin/env python3
"""Reject AI tooling attribution in the branch name, commits and PR body.

CONTRIBUTING.md states the policy; this is its check. It was written against
one run's prepared body and only ever ran for that run, so the PR body is now
an argument and the commit range has a default any branch can use.

The policy is about crediting a tool, not about naming one. This repository
integrates two AI CLIs on purpose, so a change whose whole subject is one of
them has to be able to say so. Two rules keep those apart:

- An attribution phrase always fails, wherever it appears. Quoting it does not
  make it a citation.
- A tool name fails unless it is a reference: inside a code span or fence, part
  of a path or identifier, or one of the product names spelled in full.

Say `claude -p` and `codex app-server` in backticks, or write Claude Code and
Codex CLI in full. A bare "Claude" or "Codex" in prose reads as a byline, which
is the thing the policy forbids.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Crediting a tool for the work. Matched on the original text: a phrase inside
# backticks is still a phrase.
#
# A byline is a preposition away from a subject line, so the credit verbs carry
# their prepositions too. `Drive Claude Code print mode` is the change's
# subject; `Built using Codex CLI` is the same sentence with the work handed
# over, and only the second one is a byline.
ATTRIBUTION = re.compile(
    r'co-authored-by:'
    r'|\b(?:generated|authored|written|created|produced|developed|implemented'
    r'|co-developed|co-written|built|made|assisted)'
    r'\s+(?:by|with|using)\b',
    re.IGNORECASE,
)

# The tool names are matched as words, never as part of a path or identifier:
# this repository tracks `.claude/agents/`, defines `ClaudeCliBackend`, and has
# a `hide_ai::claude` module, and text that names any of those is describing a
# file, a type or a module, not crediting a tool.
TOOL = re.compile(
    r'(?<![./\w-])(?<!::)'
    r'(?:codex|chatgpt|openai|anthropic|claude|gpt-\d|sonnet|opus)'
    r'\b(?!::)',
    re.IGNORECASE,
)

# Fenced blocks and code spans quote a command; they are citations.
CODE = re.compile(r'```.*?```|`[^`]*`', re.DOTALL)

# The products' own names, written out. A full name is a subject, not a byline.
PRODUCT = re.compile(r'\b(?:Claude Code|Codex CLI|codex-cli)\b')


def redact(text: str) -> str:
    """Blanks every reference, leaving prose for the tool-name check."""
    without_code = CODE.sub(lambda match: ' ' * len(match.group(0)), text)
    return PRODUCT.sub(lambda match: ' ' * len(match.group(0)), without_code)


def offence(text: str) -> str | None:
    """The matched attribution in `text`, or None when it carries none."""
    credited = ATTRIBUTION.search(text)
    if credited:
        return credited.group(0)
    named = TOOL.search(redact(text))
    return named.group(0) if named else None


def git(*args: str) -> str:
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--range', default='origin/main..HEAD',
                        help='commit range to read (default: origin/main..HEAD)')
    parser.add_argument('--pr-body', type=Path,
                        help='path to the prepared pull request body, when one exists')
    arguments = parser.parse_args()

    texts = {
        'branch': git('branch', '--show-current'),
        f'commits in {arguments.range}': git('log', '--format=%B', arguments.range),
    }
    if arguments.pr_body is not None:
        if not arguments.pr_body.is_file():
            print(f'error: no pull request body at {arguments.pr_body}', file=sys.stderr)
            return 1
        texts[str(arguments.pr_body)] = arguments.pr_body.read_text()

    failed = False
    for name, text in texts.items():
        found = offence(text)
        if found:
            print(f'error: attribution "{found}" found in {name}', file=sys.stderr)
            failed = True
    if failed:
        print('error: name a product in full or quote a command in backticks; '
              'a bare tool name in prose reads as a byline', file=sys.stderr)
        return 1
    print('No attribution in: ' + ', '.join(texts))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
