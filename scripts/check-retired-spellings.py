#!/usr/bin/env python3
"""Fail on a retired `hide` spelling in what Hide says and what its documents teach.

A renamed `hide` command keeps its old spelling for good: the parser and Desk's
two readers read it as the command it is now (`hide-shell/src/retired.rs`), and
the command contract keeps its entry, marked with `retired_for`
(`docs/delivery.md`, The contract a calling tool checks). What Hide writes must
use the new spelling, so the old one is never taught again (`hided/AGENTS.md`,
Renaming). The `hided` tests hold the guide, the help texts, the skill stubs
and Desk's instructions to that; this script holds every other tracked text
file of the checkout: the core's texts (`next_action`, answer values that are
commands such as `ack_command`), the other crates, `docs/`, `site/`, `web/src`,
`desktop/src`, the root documents, `.claude/skills` and the scripts.

It reads the retired spellings from `contracts/hide-cli.json`, which `hided`
writes from that one table:

- `retired_words` is the table's command rows. The words after `hide` are
  read by the longest row that starts them, as the CLI reads them, so a row
  that keeps a longer spelling (`workspace info --hook`) and a command no row
  names pass, and a row whose words moved (`plan` to `task`) fails;
- an option with `retired_for` is a retired option of its command, counted as
  a word of its own after that command, before the next `hide` on the line,
  a quote or a backtick.

Left out: `web/src/releases.json` (the release notes are history), test code
and session fixtures (a path part `tests`, `__tests__`, `fixtures` or `e2e`, a
`*.test.*` or `*.spec.*` file, a Rust `tests.rs`, and a Rust file's lines from
the `#[cfg(test)]` of an inline test module on), Rust comment lines, links,
this guard's own two files, and a kept region: text that must keep the
spelling of its day, such as an earlier build's skill stub that a kit pass
recognises byte for byte, between `retired-spellings: kept from here: <why>`
and `retired-spellings: kept to here`; a region with no reason or no end fails. A spelling split over two source lines or
built by `concat!` is not seen. CONTRIBUTING.md, CI gates, owns the rule; this
is its check.

Usage: scripts/check-retired-spellings.py [--root <checkout>]
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

SUFFIXES = {
    '.rs', '.md', '.mdx', '.json', '.ts', '.tsx', '.js', '.mjs', '.astro', '.txt', '.html',
    '.toml', '.sh', '.py', '.yml', '.yaml',
}
SKIPPED_PARTS = {'tests', '__tests__', 'fixtures', 'e2e', 'node_modules', 'dist', '.astro', 'target', '.git'}
SKIPPED_FILES = {
    'web/src/releases.json',
    'scripts/check-retired-spellings.py',
    'scripts/tests/test_check_retired_spellings.py',
}
TEST_MODULE = re.compile(r'^\s*#\[cfg\(test\)\]')
INLINE_MODULE = re.compile(r'^(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{')
HIDE = re.compile(r'(?<![\w-])hide[ \t]+')


class Retired:
    """The contract's retired spellings: the table's command rows and each
    current command's retired options."""

    def __init__(self, contract):
        self.rows = [(row['old'], row['now']) for row in contract.get('retired_words', [])]
        self.options = {}
        for command in contract['commands']:
            if 'retired_for' in command:
                continue
            renamed = {o['name']: o['retired_for'] for o in command['options'] if 'retired_for' in o}
            if renamed:
                self.options[tuple(command['command'].split())] = renamed

    def problems(self, words):
        """What is retired in `words`, the words after one `hide`."""
        found = []
        rows = [row for row in self.rows if words[:len(row[0])] == row[0]]
        if rows:
            old, now = max(rows, key=lambda row: len(row[0]))
            if old != now:
                found.append(f'hide {" ".join(old)} is retired; write hide {" ".join(now)}')
            words = now + words[len(old):]
        commands = [c for c in self.options if tuple(words[:len(c)]) == c]
        if commands:
            command = max(commands, key=len)
            for word in words[len(command):]:
                if word == '--':
                    break
                for old, now in self.options[command].items():
                    if word == old or word.startswith(old + '='):
                        name = ' '.join(command)
                        found.append(f'hide {name} {old} is retired; write hide {name} {now}')
        return found


def segments(line):
    """The words after each `hide` of `line`, up to the next one, a quote or
    a backtick, with closing punctuation taken off."""
    starts = [match.end() for match in HIDE.finditer(line)]
    for index, start in enumerate(starts):
        end = starts[index + 1] if index + 1 < len(starts) else len(line)
        text = re.split(r'[`"\']', line[start:end], maxsplit=1)[0]
        yield [word.rstrip('.,;:)') for word in text.split()]


def tracked(root):
    """The checkout's tracked files, or every file below `root` outside git."""
    try:
        listed = subprocess.run(
            ['git', '-C', str(root), 'ls-files', '-z'], capture_output=True, check=True
        ).stdout.decode('utf-8').split('\0')
        return [Path(path) for path in listed if path]
    except (subprocess.CalledProcessError, FileNotFoundError):
        return [path.relative_to(root) for path in root.rglob('*')]


def candidates(root):
    for relative in sorted(tracked(root)):
        path = root / relative
        if (
            path.is_symlink()
            or not path.is_file()
            or path.suffix not in SUFFIXES
            or relative.as_posix() in SKIPPED_FILES
            or relative.parts[0] == 'agents'
            or SKIPPED_PARTS & set(relative.parts[:-1])
            or re.search(r'\.(test|spec)\.', path.name)
            or path.name == 'tests.rs'
        ):
            continue
        yield relative


KEPT_FROM = 'retired-spellings: kept from here'
KEPT_TO = 'retired-spellings: kept to here'
# A marker is a comment line of its own, so prose that names it is read.
MARKER = re.compile(r'^\s*(//+|#+|<!--|/?\*+)\s*retired-spellings: kept (from|to) here')


class Unreadable(Exception):
    """A kept region with no reason or no end."""


def lines_of(path, text):
    """The lines of `path` to read: a Rust file's up to an inline test
    module and outside its comments, and no file's inside a kept region,
    which history that must not change (an earlier build's text, byte for
    byte) opens with a comment line `retired-spellings: kept from here:
    <why>` and closes with one `retired-spellings: kept to here`."""
    lines = text.split('\n')
    kept = None
    for index, line in enumerate(lines):
        marker = MARKER.match(line)
        if marker and marker.group(2) == 'from':
            # The reason, with a comment's own marks around it taken off.
            if not line.split(KEPT_FROM, 1)[1].strip(' :-*/>#'):
                raise Unreadable(f'{path}:{index + 1}: a kept region says why after "{KEPT_FROM}:"')
            kept = index + 1
            continue
        if marker:
            kept = None
            continue
        if kept is not None:
            continue
        if path.suffix == '.rs':
            if TEST_MODULE.match(line) and opens_module(lines[index + 1:]):
                return
            if line.lstrip().startswith('//'):
                continue
        yield index + 1, line
    if kept is not None:
        raise Unreadable(f'{path}:{kept}: a kept region has no "{KEPT_TO}"')


def opens_module(rest):
    """Whether the item an attribute stands on opens an inline module
    (`mod tests {`): a `#[cfg(test)]` on one function or constant stops
    nothing after it."""
    for line in rest:
        stripped = line.strip()
        if not stripped or stripped.startswith(('#[', '//')):
            continue
        return bool(INLINE_MODULE.match(stripped))
    return False


def check(root):
    contract = json.loads((root / 'contracts/hide-cli.json').read_text(encoding='utf-8'))
    retired = Retired(contract)
    errors = []
    for relative in candidates(root):
        try:
            text = (root / relative).read_text(encoding='utf-8')
        except UnicodeDecodeError:
            continue
        try:
            for number, line in lines_of(relative, text):
                for words in segments(line):
                    for problem in retired.problems(words):
                        errors.append(f'{relative}:{number}: {problem}')
        except Unreadable as unreadable:
            errors.append(str(unreadable))
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent.parent)
    arguments = parser.parse_args(argv)
    errors = check(arguments.root)
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        print(
            f'{len(errors)} retired hide spelling(s); hided/AGENTS.md, Renaming, says why each must go',
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
