#!/usr/bin/env python3
"""Fail on a retired `hide` spelling in what Hide says and what its documents teach.

A renamed `hide` command keeps its old spelling for good: the parser and Desk's
two readers read it as the command it is now (`hide-shell/src/retired.rs`), and
the command contract keeps its entry, marked with `retired_for`
(`docs/delivery.md`, The contract a calling tool checks). What Hide writes must
use the new spelling, so the old one is never taught again (`hided/AGENTS.md`,
Renaming). The `hided` tests hold the guide, the help texts, the skill stubs
and Desk's instructions to that; this script holds the rest:

- the core's texts and the other Rust strings an agent or a person reads
  (`next_action`, answer values that are commands such as `ack_command`), in
  `hide-core`, `hided`, `hide-agent-hooks`, `hide-kit` and `hide-desk`;
- `docs/`, `site/` and `web/src`.

It reads the retired spellings from `contracts/hide-cli.json`, which `hided`
writes from that one table:

- an entry with `retired_for` is a retired command (`plan set` for `task set`);
  a first word that no current command starts with is a retired topic
  (`plan`), so `hide plan` alone counts too;
- an option with `retired_for` is a retired option of its command, counted on
  a line that names that command.

Left out: `web/src/releases.json` (the release notes are history), test code
and session fixtures (a path part `tests`, `__tests__`, `fixtures` or `e2e`, a
`*.test.*` or `*.spec.*` file, a Rust `tests.rs`, and a Rust file's lines from
its `#[cfg(test)]` on), and Rust comment lines. CONTRIBUTING.md, CI gates, owns
the rule; this is its check.

Usage: scripts/check-retired-spellings.py [--root <checkout>]
"""
import argparse
import json
import re
import sys
from pathlib import Path

ROOTS = (
    'hide-core/src', 'hided/src', 'hided/assets', 'hide-agent-hooks/src', 'hide-kit/src',
    'hide-desk/src', 'docs', 'site', 'web/src',
)
SUFFIXES = {'.rs', '.md', '.mdx', '.json', '.ts', '.tsx', '.astro', '.txt', '.html'}
SKIPPED_PARTS = {'tests', '__tests__', 'fixtures', 'e2e', 'node_modules', 'dist', '.astro'}
SKIPPED_FILES = {'web/src/releases.json'}
TEST_MODULE = re.compile(r'^\s*#\[cfg\(test\)\]')


def retired(contract):
    """The retired spellings the contract lists: (pattern, old, new) triples."""
    commands = contract['commands']
    current = [c['command'].split() for c in commands if 'retired_for' not in c]
    found = []
    topics = {}
    for command in commands:
        name = command['command']
        if 'retired_for' in command:
            found.append((name, command['retired_for']))
            old, new = name.split()[0], command['retired_for'].split()[0]
            if not any(words[0] == old for words in current):
                topics[old] = new
        else:
            for option in command['options']:
                if 'retired_for' in option:
                    found.append((f'{name} {option["name"]}', f'{name} {option["retired_for"]}'))
    found.extend(topics.items())
    spellings = []
    for old, new in found:
        words = old.split()
        if words[-1].startswith('--') and len(words) > 1:
            # An option counts after its command on the same line, as a word
            # of its own: a quoted `"--machine"` is a value.
            command = r'\s+'.join(map(re.escape, words[:-1]))
            pattern = rf'(?<![\w-])hide\s+{command}\b.*?(?<=\s){re.escape(words[-1])}(?=\s|$|=)'
        else:
            pattern = r'(?<![\w-])hide\s+' + r'\s+'.join(map(re.escape, words)) + r'(?![\w-])'
        spellings.append((re.compile(pattern), old, new))
    return spellings


def candidates(root):
    for top in ROOTS:
        base = root / top
        if not base.exists():
            continue
        for path in sorted(base.rglob('*')):
            relative = path.relative_to(root)
            if (
                not path.is_file()
                or path.suffix not in SUFFIXES
                or relative.as_posix() in SKIPPED_FILES
                or SKIPPED_PARTS & set(relative.parts[:-1])
                or re.search(r'\.(test|spec)\.', path.name)
                or path.name == 'tests.rs'
            ):
                continue
            yield relative


def lines_of(path, text):
    for number, line in enumerate(text.split('\n'), 1):
        if path.suffix == '.rs':
            if TEST_MODULE.match(line):
                return
            if line.lstrip().startswith('//'):
                continue
        yield number, line


def check(root):
    contract = json.loads((root / 'contracts/hide-cli.json').read_text(encoding='utf-8'))
    spellings = retired(contract)
    errors = []
    for relative in candidates(root):
        try:
            text = (root / relative).read_text(encoding='utf-8')
        except UnicodeDecodeError:
            continue
        for number, line in lines_of(relative, text):
            for pattern, old, new in spellings:
                if pattern.search(line):
                    errors.append(f'{relative}:{number}: hide {old} is retired; write hide {new}')
                    break
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
