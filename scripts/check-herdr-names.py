#!/usr/bin/env python3
"""Ratchet the case-insensitive `herdr` names in tracked files down to the kept ones.

Herdr is gone from Hide, but a few hundred names stay on purpose: the read-only
import, attribution, the old values Hide still reads, fixtures and test
messages. `scripts/herdr-names-allowlist.json` lists each file that keeps names,
its number and one reason. This check fails when:

- a file that is not listed names Herdr;
- a listed file names more than its number, so a new name arrived;
- a listed file names fewer, so its number must come down (the ratchet only goes down);
- a listed file names nothing any more, or names a reason the list does not define.

The folder name `herdr-ide` is not Herdr, so it is removed before counting. The
guard's own three files are not scanned, because they must name the word.

CONTRIBUTING.md, CI gates, owns the rule and the reasons; this is its check.

Usage: scripts/check-herdr-names.py   (run from anywhere in the checkout)
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = 'scripts/herdr-names-allowlist.json'
GUARD = {
    'scripts/check-herdr-names.py',
    ALLOWLIST,
    'scripts/tests/test_check_herdr_names.py',
}
NAME = re.compile(r'herdr', re.IGNORECASE)
FOLDER = re.compile(r'herdr-ide', re.IGNORECASE)
SHOWN = 5


def count(text):
    """The names in one file's text: `herdr` matches once the folder name is removed."""
    return len(NAME.findall(FOLDER.sub('', text)))


def lines_with_names(text):
    """The 1-based line numbers that name Herdr, the folder name removed first."""
    return [
        number
        for number, line in enumerate(text.split('\n'), 1)
        if NAME.search(FOLDER.sub('', line))
    ]


def where(text):
    numbers = lines_with_names(text)
    shown = ', '.join(str(n) for n in numbers[:SHOWN])
    more = '' if len(numbers) <= SHOWN else f' and {len(numbers) - SHOWN} more'
    return f'lines {shown}{more}'


def check(texts, allowlist):
    """Return the errors for `texts` ({path: text} of tracked files) against `allowlist`."""
    reasons = allowlist['reasons']
    listed = allowlist['files']
    errors = []
    for path, entry in sorted(listed.items()):
        if entry['reason'] not in reasons:
            errors.append(
                f'{ALLOWLIST}: {path} gives the reason "{entry["reason"]}", '
                f'which the list does not define; use one of: {", ".join(reasons)}'
            )
        if path in GUARD:
            errors.append(f'{ALLOWLIST}: {path} is one of the guard files and is not scanned; remove its entry')
        elif path not in texts:
            errors.append(f'{ALLOWLIST}: {path} is listed but not tracked; remove its entry')
    for path in sorted(texts):
        if path in GUARD:
            continue
        text = texts[path]
        names = count(text)
        entry = listed.get(path)
        if entry is None:
            if names:
                errors.append(
                    f'{path}: names Herdr in {where(text)} and is not on {ALLOWLIST}. '
                    'Remove the name (Herdr is gone), or list the file with its count and one reason.'
                )
            continue
        if names > entry['max']:
            errors.append(
                f'{path}: names Herdr {names} times, the list allows {entry["max"]} ({where(text)}). '
                'Remove the new names; a file on the list may not gain any.'
            )
        elif names == 0:
            errors.append(f'{path}: names Herdr no more; remove its entry from {ALLOWLIST}.')
        elif names < entry['max']:
            errors.append(
                f'{path}: names Herdr {names} times, the list allows {entry["max"]}. '
                f'Lower its "max" to {names} in {ALLOWLIST}; the ratchet only goes down.'
            )
    return errors


def tracked_texts():
    listing = subprocess.run(
        ['git', 'ls-files', '-z'], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode('utf-8').split('\0')
    texts = {}
    for path in listing:
        if not path or path.startswith('agents/'):
            continue
        full = ROOT / path
        if not full.is_file():
            continue
        texts[path] = full.read_bytes().decode('utf-8', errors='replace')
    return texts


def main():
    allowlist = json.loads((ROOT / ALLOWLIST).read_text(encoding='utf-8'))
    errors = check(tracked_texts(), allowlist)
    if errors:
        for error in errors:
            print(f'error: {error}', file=sys.stderr)
        print(f'{len(errors)} Herdr name problem(s); see docs in CONTRIBUTING.md, CI gates.', file=sys.stderr)
        return 1
    files = len(allowlist['files'])
    total = sum(entry['max'] for entry in allowlist['files'].values())
    print(f'Herdr names: {total} on the list in {files} files; nothing new.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
