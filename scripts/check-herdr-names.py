#!/usr/bin/env python3
"""Ratchet the case-insensitive `herdr` names in tracked files down to the kept ones.

Herdr is gone from Hide, but a few hundred names stay on purpose: the read-only
import, attribution, the old values Hide still reads, fixtures and test
messages. `scripts/herdr-names-allowlist.json` lists each file that keeps names,
its number and one reason. A document split into a table of contents and pieces
is listed once, under its own path: its entry counts the table of contents,
every piece its split table (`scripts/split-tables/`, read from the index like
the files it counts) names and that table, together, so a split moves names between files without editing a line another
split edits.
This check fails when:

- a file that is not listed names Herdr;
- a listed file names more than its number, so a new name arrived;
- a listed file names fewer, so its number must come down (the ratchet only goes down);
- a listed file names nothing any more, or names a reason the list does not define;
- a piece or the split table of a split document has an entry of its own;
- a tracked path under the root `agents/` folder exists (that folder is local-only);
- git cannot read the index, which is reported with git's own message.

The candidate files are read from git's index with `git grep --cached`, never
from the working tree: no path is opened, so a file replaced on disk by a FIFO
or a link changes nothing. The allow-list file itself is read from the working
tree. git streams the matching lines to this process one at a time, and the
check keeps only per-file counts and line numbers, not the text; the longest
single matching line is the one thing held whole, and the size of the matching
text is what review lets into the repository. Binary blobs are skipped by git
(`-I`). The root `agents/` folder is left out of the search by the pathspec,
and a tracked path under it is reported instead.

The folder name `herdr-ide` is exempt only as a whole path component: it is
exempt when the character before it is the start of the line or one of
whitespace, `/`, a quote, a backtick, `(` or `~`, and the character after it is
the end of the line or one of whitespace, `/`, a quote, a backtick, `)`, `,`,
`:`, `;` or `]`. Anything else counts, so `herdr-ide.git` and `herdr-ide@x` count.

The guard's own three files are not scanned, because they must name the word.
CONTRIBUTING.md, CI gates, owns the rule and the reasons; this is its check.

Usage: scripts/check-herdr-names.py   (run from anywhere in the checkout)
"""
import importlib.util
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
FOLDER = re.compile(r"(?<![^\s/'`\"(~])herdr-ide(?![^\s/'`\")\],:;])", re.IGNORECASE)
SHOWN = 5
NAMESPACE = 'agents'


def names_in(text):
    """Count and line numbers of the Herdr names in `text`; the folder name is removed first."""
    count = 0
    lines = []
    for number, line in enumerate(text.split('\n'), 1):
        found = len(NAME.findall(FOLDER.sub('', line)))
        if found:
            count += found
            lines.append(number)
    return count, lines


def split_files(root):
    """The files each split document is counted with, {piece or table path: table of
    contents path}, from the split tables in git's index, the snapshot the names are counted
    in, parsed as scripts/check-docs.py parses them. Raises GitFailure when git cannot answer."""
    spec = importlib.util.spec_from_file_location('check_docs', Path(__file__).resolve().parent / 'check-docs.py')
    docs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(docs)
    listing = git(root, ['ls-files', '-z', '--cached', '--', f'{docs.TABLES}/*.json']).decode('utf-8', errors='replace')
    indexed = [(path, git(root, ['show', f':{path}']).decode('utf-8', errors='replace'))
               for path in listing.split('\0') if path and '/' not in path[len(docs.TABLES) + 1:]]
    files = {}
    for where, table in docs.parse_tables(indexed)[0]:
        files[where] = table['source']
        files.update({f'{table["folder"]}/{piece["name"]}.md': table['source'] for piece in table['pieces']})
    return files


def fold(results, pieces):
    """`results` with the names of each piece and split table counted under its document;
    their lines are named `<path>:<line>`."""
    folded = {path: entry for path, entry in results.items() if path not in pieces}
    for path, source in sorted(pieces.items()):
        if path in results:
            names, lines = results[path]
            count, before = folded.get(source, (0, []))
            folded[source] = (count + names, before + [f'{path}:{n}' for n in lines])
    return folded


def where(lines):
    shown = ', '.join(str(n) for n in lines[:SHOWN])
    more = '' if len(lines) <= SHOWN else f' and {len(lines) - SHOWN} more'
    return f'lines {shown}{more}'


def check(results, allowlist, pieces=None):
    """Return errors for `results` ({path: (count, lines)}) against `allowlist`; `pieces`
    ({piece or split table path: its document}) counts each split document with them."""
    pieces = pieces or {}
    documents = set(pieces.values())
    results = fold(results, pieces)
    reasons = allowlist['reasons']
    listed = allowlist['files']
    errors = []
    for path, entry in sorted(listed.items()):
        if path in pieces:
            errors.append(f'{ALLOWLIST}: {path} is counted with the split document {pieces[path]}; remove its entry')
            continue
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
        names, lines = results[path]
        entry = listed.get(path)
        label = f'{path} with its pieces and split table' if path in documents else path
        if entry is None:
            if names:
                errors.append(
                    f'{label}: names Herdr in {where(lines)} and is not on {ALLOWLIST}. '
                    'Remove the name (Herdr is gone), or list the file with its count and one reason.'
                )
            continue
        if names > entry['max']:
            errors.append(
                f'{label}: names Herdr {names} times, the list allows {entry["max"]} ({where(lines)}). '
                'Remove the new names; a file on the list may not gain any.'
            )
        elif names == 0:
            errors.append(f'{label}: names Herdr no more; remove its entry from {ALLOWLIST}.')
        elif names < entry['max']:
            errors.append(
                f'{label}: names Herdr {names} times, the list allows {entry["max"]}. '
                f'Lower its "max" to {names} in {ALLOWLIST}; the ratchet only goes down.'
            )
    return errors


class GitFailure(Exception):
    """git could not answer; the message is git's own."""


def git(root, arguments):
    """Run git in `root` and return its stdout bytes; raise GitFailure with git's own message otherwise."""
    completed = subprocess.run(['git', *arguments], cwd=root, capture_output=True)
    if completed.returncode != 0:
        message = completed.stderr.decode('utf-8', errors='replace').strip()
        raise GitFailure(f'git {arguments[0]} failed: {message or "exit " + str(completed.returncode)}')
    return completed.stdout


def git_grep_matches(root):
    """Yield (path, line number, text) for each match in the index, streamed one record at a time."""
    process = subprocess.Popen(
        ['git', 'grep', '--cached', '-I', '-n', '-i', '-z', '-e', 'herdr', '--',
         '.', f':(exclude){NAMESPACE}'],
        cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    for raw in process.stdout:
        # -z puts a NUL after the path and after the line number.
        path, number, text = raw.rstrip(b'\n').decode('utf-8', errors='replace').split('\0', 2)
        yield path, int(number), text
    message = process.stderr.read().decode('utf-8', errors='replace').strip()
    code = process.wait()
    if code not in (0, 1):  # exit status 1 is no match
        raise GitFailure(f'git grep failed: {message or "exit " + str(code)}')


def scan_index(root):
    """Return (results, problems) from git's index.

    `results` maps each tracked path outside `agents/` to (count, lines). `problems`
    holds a tracked path under `agents/` and any git failure, with git's own message.
    """
    problems = []
    results = {}
    try:
        listing = git(root, ['ls-files', '-z']).decode('utf-8', errors='replace')
        for path in (p for p in listing.split('\0') if p):
            if path == NAMESPACE or path.startswith(NAMESPACE + '/'):
                problems.append(f'{path}: is tracked under the root {NAMESPACE}/ folder, which is local-only; untrack it')
            elif path not in GUARD:
                results[path] = (0, [])
        for path, number, text in git_grep_matches(root):
            if path not in results:
                continue
            hits, _ = names_in(text)
            if hits:
                count, lines = results[path]
                results[path] = (count + hits, lines + [number])
    except GitFailure as failure:
        problems.append(str(failure))
    return results, problems


def main():
    allowlist = json.loads((ROOT / ALLOWLIST).read_text(encoding='utf-8'))
    results, problems = scan_index(ROOT)
    try:
        pieces = split_files(ROOT)
    except GitFailure as failure:
        problems.append(str(failure))
        pieces = {}
    errors = problems + check(results, allowlist, pieces)
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
