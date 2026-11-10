#!/usr/bin/env python3
"""Ratchet the case-insensitive `herdr` names in tracked files down to the kept ones.

Herdr is gone from Hide, but a few hundred names stay on purpose: the read-only
import, attribution, the old values Hide still reads, fixtures and test
messages. `scripts/herdr-names-allowlist.json` lists each file that keeps names,
its number and one reason. This check fails when:

- a file that is not listed names Herdr;
- a listed file names more than its number, so a new name arrived;
- a listed file names fewer, so its number must come down (the ratchet only goes down);
- a listed file names nothing any more, or names a reason the list does not define;
- a tracked text file cannot be read, or is too large to count (see MAX_BYTES).

The folder name `herdr-ide` is a path component, not Herdr, so the exact folder
name is removed before counting when no letter, digit, underscore or hyphen is
attached to it: `herdr-ide/` and `~/projects/herdr-ide` are not counted, while
`herdr-ide-foo`, `herdr-identity` and `herdr_ide` are.

Only tracked regular files are read. A tracked symlink or submodule is not
followed, so a link cannot pull in an untracked file (such as the local-only
root `agents/` folder). Binary files (a NUL byte) are skipped.

The guard's own three files are not scanned, because they must name the word.
CONTRIBUTING.md, CI gates, owns the rule and the reasons; this is its check.

Usage: scripts/check-herdr-names.py   (run from anywhere in the checkout)
"""
import json
import os
import re
import stat
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
# 8 MiB: the largest tracked file today is a 4.8 MB GIF, so the cap only fires on
# a new large text file. It also bounds what one file costs in memory: a file is
# counted and discarded before the next one is read.
MAX_BYTES = 8 * 1024 * 1024
CHUNK = 64 * 1024
NAME = re.compile(r'herdr', re.IGNORECASE)
FOLDER = re.compile(r'(?<![\w-])herdr-ide(?![\w-])', re.IGNORECASE)
SHOWN = 5
SYMLINK_MODE = '120000'
SUBMODULE_MODE = '160000'


def names_in(text):
    """Line numbers and count of Herdr names in `text`; the folder name is removed first."""
    lines = [
        number
        for number, line in enumerate(text.split('\n'), 1)
        if NAME.search(FOLDER.sub('', line))
    ]
    return len(NAME.findall(FOLDER.sub('', text))), lines


def where(lines):
    shown = ', '.join(str(n) for n in lines[:SHOWN])
    more = '' if len(lines) <= SHOWN else f' and {len(lines) - SHOWN} more'
    return f'lines {shown}{more}'


def scan(path, cap=MAX_BYTES):
    """Count the names in one tracked file. Returns ('binary', None), ('count', (n, lines)) or ('error', message).

    A file is read with O_NOFOLLOW, so a link cannot send the read elsewhere.
    A file over `cap` is only counted when it has no NUL byte; it is otherwise an error.
    """
    try:
        descriptor = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
    except OSError as error:
        return 'error', f'cannot be read ({error.strerror or error})'
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            return 'error', 'is not a regular file'
        if info.st_size > cap:
            with os.fdopen(os.dup(descriptor), 'rb') as handle:
                while True:
                    chunk = handle.read(CHUNK)
                    if not chunk:
                        break
                    if b'\0' in chunk:
                        return 'binary', None
            return 'error', (
                f'is {info.st_size} bytes, over the {cap}-byte cap, and is not binary; '
                'the check cannot count its names: split it or keep it out of git'
            )
        data = b''
        while True:
            chunk = os.read(descriptor, CHUNK)
            if not chunk:
                break
            data += chunk
        if b'\0' in data:
            return 'binary', None
        return 'count', names_in(data.decode('utf-8', errors='replace'))
    except OSError as error:
        return 'error', f'cannot be read ({error.strerror or error})'
    finally:
        os.close(descriptor)


def check(results, allowlist):
    """Return errors for `results` ({path: ('count', (n, lines)) or ('error', message)}) against `allowlist`."""
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
        elif path not in results:
            errors.append(f'{ALLOWLIST}: {path} is listed but not a tracked text file; remove its entry')
    for path in sorted(results):
        if path in GUARD:
            continue
        kind, value = results[path]
        if kind == 'error':
            errors.append(f'{path}: {value}. Fix the file, or remove it from git if it does not belong.')
            continue
        names, lines = value
        entry = listed.get(path)
        if entry is None:
            if names:
                errors.append(
                    f'{path}: names Herdr in {where(lines)} and is not on {ALLOWLIST}. '
                    'Remove the name (Herdr is gone), or list the file with its count and one reason.'
                )
            continue
        if names > entry['max']:
            errors.append(
                f'{path}: names Herdr {names} times, the list allows {entry["max"]} ({where(lines)}). '
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


def tracked_entries(root):
    """(mode, path) of every tracked entry, from the index; the root `agents/` namespace is left out."""
    listing = subprocess.run(
        ['git', 'ls-files', '-s', '-z'], cwd=root, capture_output=True, check=True
    ).stdout.decode('utf-8').split('\0')
    entries = []
    for record in listing:
        if not record:
            continue
        meta, path = record.split('\t', 1)
        mode = meta.split(' ')[0]
        if path.startswith('agents/'):
            continue
        entries.append((mode, path))
    return entries


def collect(root, entries, cap=MAX_BYTES):
    """Scan the tracked entries under `root`. Links, submodules and binary files are left out."""
    results = {}
    for mode, path in entries:
        if mode in (SYMLINK_MODE, SUBMODULE_MODE):
            continue
        kind, value = scan(Path(root) / path, cap)
        if kind == 'binary':
            continue
        results[path] = (kind, value)
    return results


def main():
    allowlist = json.loads((ROOT / ALLOWLIST).read_text(encoding='utf-8'))
    errors = check(collect(ROOT, tracked_entries(ROOT)), allowlist)
    if errors:
        for error in errors:
            print(f'error: {error}', file=sys.stderr)
        print(f'{len(errors)} Herdr name problem(s); see CONTRIBUTING.md, CI gates.', file=sys.stderr)
        return 1
    files = len(allowlist['files'])
    total = sum(entry['max'] for entry in allowlist['files'].values())
    print(f'Herdr names: {total} on the list in {files} files; nothing new.')
    return 0


if __name__ == '__main__':
    sys.exit(main())
