#!/usr/bin/env python3
"""Split an oversized guide into a table of contents and pieces, moving the
text unchanged, as docs/README.md, Splitting a document, describes.

Usage: scripts/split-doc.py --source-ref <git ref> --table <table.json> [--checkout <path>]

Reads the table's source document as it is at <ref> (main, so a rerun right
before the merge takes every section main has by then) and writes into the
checkout's working tree:

- one piece per table entry under the table's folder: `# <title>`, the
  `- Code:` and `- Index:` fields, then the text of its sections;
- the source file as the table of contents: its title and introduction, a
  `| Piece | Code |` table, then every heading the file had, with its name,
  level and anchor, each pointing at the piece that holds it;
- scripts/herdr-names-allowlist.json, the source's entry replaced by one
  entry per written file that names Herdr, with its count.

The table is JSON: {"source": "docs/X.md", "folder": "docs/x", "pieces":
[{"name": "<file stem>", "title": "<title>", "code": ["<path>", ...],
"sections": ["<heading text>", ...]}, ...]}. A placed heading takes every
deeper heading after it up to the next heading of its level or higher; the
title and the text before the first placed heading stay in the table of
contents.

What changes in the moved text: heading levels move so a piece's highest
heading is `##`, and relative link targets are rewritten for the folder (an
in-file anchor goes to the file that holds it). The script proves, before it
writes anything, that nothing else changed, that every source line landed in
exactly one place and that no anchor was lost.

It refuses: a source that is already a table of contents, a heading the table
does not place, a table section the source no longer has, a section placed
twice, a link to an anchor no heading answers, a link form it cannot retarget
(a title, an angle-bracket target, a reference definition, HTML, a
root-relative path), a duplicate anchor, a link between the outputs that no
heading answers, a Code path that does not exist, an unclosed code fence, and
a Markdown file in the folder that is not a piece of this source. Headings,
fences and anchors are read as scripts/check-docs.py reads them.
"""
import argparse
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = 'scripts/herdr-names-allowlist.json'
TOC_MARKER = 'is kept as pieces under'
LINK = re.compile(r'\]\(([^)\s]+)\)')
CODE_SPAN = re.compile(r'(``.+?``|`[^`]*`)')
SCHEME = re.compile(r'^[a-z][a-z0-9+.-]*:')
# Link forms the rewrite does not retarget; the source changes them to `[text](target)` first.
UNHANDLED = (
    (re.compile(r'\]\(\s*<'), 'an angle-bracket link target'),
    (re.compile(r'\]\([^)\s]*\s+["\'(]'), 'a link with a title'),
    (re.compile(r'^ {0,3}\[[^\]]+\]:\s'), 'a reference definition'),
    (re.compile(r'<(?:a|img)\b[^>]*\b(?:href|src)\s*=', re.IGNORECASE), 'an HTML link'),
    (re.compile(r'\]\(/'), 'a root-relative link'),
)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# What a heading, a fence and an anchor are: the docs check's one reading.
DOCS = load('check_docs', Path(__file__).resolve().parent / 'check-docs.py')


class Refused(Exception):
    pass


def git(checkout, *args):
    done = subprocess.run(['git', '-C', str(checkout), *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise Refused(f'git {" ".join(args)}: {done.stderr.strip()}')
    return done.stdout


def plain(line):
    """The parts of `line` outside its inline code spans."""
    return CODE_SPAN.split(line)[::2]


def links(text):
    """The link targets of a Markdown text, code fences and inline code left out."""
    lines = text.split('\n')
    fenced = DOCS.structure(lines)[1]
    return [target for index, line in enumerate(lines) if index not in fenced
            for part in plain(line) for target in LINK.findall(part)]


def outside_code(line, change):
    """`line` with `change` applied to the text outside its inline code spans."""
    parts = CODE_SPAN.split(line)
    return ''.join(change(part) if index % 2 == 0 else part for index, part in enumerate(parts))


def place(lines, headings, pieces):
    """Chunks: [(owner, start, end)], owner a piece name or 'toc'; every line in exactly one."""
    owner_of = {}
    for piece in pieces:
        for section in piece['sections']:
            if section in owner_of:
                raise Refused(f'"{section}" is placed twice in the table')
            owner_of[section] = piece['name']
    if not headings or headings[0][0] != 0 or headings[0][1] != 1:
        raise Refused('the source does not start with its "# " title')
    chunks, level, seen = [['toc', 0, None]], 1, set()
    for index, depth, text in headings[1:]:
        if text in owner_of:
            if text in seen:
                raise Refused(f'the source has two headings "{text}"')
            seen.add(text)
            chunks.append([owner_of[text], index, None])
            level = depth
        elif len(chunks) == 1 or depth <= level:
            raise Refused(f'heading "{text}" (line {index + 1}) is not placed: add it to the table')
    missing = set(owner_of) - seen
    if missing:
        raise Refused(f'the table places sections the source no longer has: {sorted(missing)}')
    for this, after in zip(chunks, chunks[1:] + [[None, len(lines), None]]):
        this[2] = after[1]
    return [tuple(chunk) for chunk in chunks]


def rewrite_links(line, own, anchor_home, source_name, up):
    """Retarget relative links for the folder; `own` is the output file the line lands in."""
    def target(match):
        url = match.group(1)
        if SCHEME.match(url):
            return match.group(0)
        path, _, anchor = url.partition('#')
        if path in ('', source_name):
            if not anchor:
                return f']({up}{source_name})'
            if anchor not in anchor_home:
                raise Refused(f'a link to #{anchor} answers no heading in the source: {line.strip()[:120]}')
            home = anchor_home[anchor]
            if home == own:
                return f'](#{anchor})'
            if home == 'toc':
                return f']({up}{source_name}#{anchor})'
            return f']({home}#{anchor})'
        return f']({up}{url})'
    return outside_code(line, lambda text: LINK.sub(target, text))


def kept(line, heading):
    """What must not change in a moved line: all but its heading marks and link targets."""
    if heading:
        line = re.sub(r'^#{1,6} ', '', line)
    return outside_code(line, lambda text: LINK.sub('](link)', text))


def anchor_list(text):
    return [DOCS.slug(heading) for _, _, heading in DOCS.structure(text.split('\n'))[0]]


def code_line(paths, checkout):
    if not paths:
        raise Refused('a piece names no Code path')
    for path in paths:
        if not (checkout / path.rstrip('/')).exists():
            raise Refused(f'code path {path} does not exist')
    return ', '.join(f'`{path}`' for path in paths)


def herdr_counter(checkout):
    module = load('check_herdr_names', checkout / 'scripts' / 'check-herdr-names.py')
    return lambda text: module.names_in(text)[0]


def allowlist_entries(text):
    """The allowlist's lines, the span of its "files" entries, and the entries."""
    lines = text.split('\n')
    entry = re.compile(r'^    "([^"]+)": \{"max": (\d+), "reason": "([^"]+)"\},?$')
    start = next(i for i, line in enumerate(lines) if line.strip() == '"files": {')
    end = next(i for i in range(start + 1, len(lines)) if lines[i].strip() in ('}', '},'))
    files = {}
    for line in lines[start + 1:end]:
        match = entry.match(line)
        if not match:
            raise Refused(f'{ALLOWLIST}: unexpected line {line!r}')
        files[match.group(1)] = (int(match.group(2)), match.group(3))
    return lines, start, end, files


def allowlist_update(checkout, commit, source, replaced, outputs):
    """The allowlist's new text: the entries of the source and of the files the split replaces,
    then one entry per output that names Herdr, with its count. The source's entry comes from
    the allowlist at the source's commit, so a rerun after an earlier split of the same source
    finds it."""
    lines, start, end, files = allowlist_entries((checkout / ALLOWLIST).read_text(encoding='utf-8'))
    old = allowlist_entries(git(checkout, 'show', f'{commit}:{ALLOWLIST}'))[3].get(source)
    for name in {source, *replaced, *outputs}:
        files.pop(name, None)
    count = herdr_counter(checkout)
    before = old[0] if old else 0
    after = 0
    for name, text in outputs.items():
        names = count(text)
        after += names
        if names:
            if old is None:
                raise Refused(f'{name} would name Herdr {names} times, and {source} has no allowlist entry')
            files[name] = (names, old[1])
    body = [f'    "{name}": {{"max": {files[name][0]}, "reason": "{files[name][1]}"}},' for name in sorted(files)]
    if body:
        body[-1] = body[-1].rstrip(',')
    return '\n'.join(lines[:start + 1] + body + lines[end:]), before, after


def owned(checkout, folder, source_name, up):
    """The pieces already in the folder; refuses a Markdown file there that is not one."""
    directory = checkout / folder
    if not directory.is_dir():
        return []
    index = re.compile(rf'^- Index: .*\]\({re.escape(up + source_name)}(?:#[^)]*)?\)', re.MULTILINE)
    pieces, foreign = [], []
    for path in sorted(directory.glob('*.md')):
        (pieces if index.search(path.read_text(encoding='utf-8')) else foreign).append(f'{folder}/{path.name}')
    if foreign:
        raise Refused(f'{folder}/ holds files that are not pieces of {source_name}: {foreign}; choose another folder')
    return pieces


def split(checkout, ref, table):
    """The files to write ({path: text}), the old pieces to delete, and a report; raises
    Refused before anything is written."""
    source, folder, pieces = table['source'], table['folder'], table['pieces']
    source_dir = os.path.dirname(source)
    rel = os.path.relpath(folder, source_dir).replace(os.sep, '/')
    up = os.path.relpath(source_dir or '.', folder).replace(os.sep, '/') + '/'
    source_name = os.path.basename(source)
    commit = git(checkout, 'rev-parse', '--verify', f'{ref}^{{commit}}').strip()
    text = git(checkout, 'show', f'{commit}:{source}')
    if TOC_MARKER in text or re.search(rf'^Moved to \[[^\]]*\]\({re.escape(rel)}/', text, re.MULTILINE):
        raise Refused(f'{source} at {ref} is already a table of contents')
    names = [piece['name'] for piece in pieces]
    if not names:
        raise Refused('the table places no piece')
    if len(set(names)) != len(names) or not all(re.match(r'^[a-z0-9]+(?:-[a-z0-9]+)*$', n) for n in names):
        raise Refused('piece names must be distinct lowercase-hyphen stems')
    existing = owned(checkout, folder, source_name, up)
    lines = text.split('\n')
    headings, fenced, closed = DOCS.structure(lines)
    if not closed:
        raise Refused('an unclosed code fence in the source')
    for number, line in enumerate(lines):
        if number in fenced:
            continue
        for pattern, form in UNHANDLED:
            if any(pattern.search(part) for part in plain(line)):
                raise Refused(f'{form} on line {number + 1}, which the split cannot retarget; '
                              f'write it as [text](target) first: {line.strip()[:120]}')
    is_heading = {index for index, _, _ in headings}
    chunks = place(lines, headings, pieces)
    file_of = {name: f'{name}.md' for name in names}

    owner_at = {}
    for owner, start, end in chunks:
        for index in range(start, end):
            owner_at[index] = owner
    anchor_home = {}
    for index, _, heading in headings:
        anchor = DOCS.slug(heading)
        if anchor in anchor_home:
            raise Refused(f'two source headings share the anchor #{anchor}')
        anchor_home[anchor] = 'toc' if owner_at[index] == 'toc' else file_of[owner_at[index]]

    outputs, covered, changed = {}, [], 0
    index_line = f'- Index: [{source_name}]({up}{source_name})'
    for piece in pieces:
        name, own = piece['name'], file_of[piece['name']]
        shift = 2 - min(depth for index, depth, _ in headings if owner_at[index] == name)
        body = []
        for owner, start, end in chunks:
            if owner != name:
                continue
            stop = end
            while stop > start and lines[stop - 1] == '':
                stop -= 1
            covered.extend(range(start, end))
            for index in range(start, stop):
                line = lines[index]
                if index not in fenced:
                    if index in is_heading:
                        marks = len(line) - len(line.lstrip('#'))
                        line = '#' * (marks + shift) + line[marks:]
                    line = rewrite_links(line, own, anchor_home, source_name, up)
                # The proof: a moved line keeps everything but its heading marks and link targets.
                if index in fenced and line != lines[index] or \
                        kept(line, index in is_heading) != kept(lines[index], index in is_heading):
                    raise Refused(f'line {index + 1} would change more than its heading level and link targets')
                changed += line != lines[index]
                body.append(line)
            body.append('')
        piece_text = '\n'.join([f'# {piece["title"]}', '', f'- Code: {code_line(piece["code"], checkout)}',
                                index_line, ''] + body).rstrip('\n') + '\n'
        found = anchor_list(piece_text)
        if len(set(found)) != len(found):
            raise Refused(f'{folder}/{own} would hold a duplicate anchor')
        outputs[f'{folder}/{own}'] = piece_text

    head = lines[0:chunks[1][1]]
    covered.extend(range(0, chunks[1][1]))
    while head and head[-1] == '':
        head = head[:-1]
    toc = head + ['', f'This guide {TOC_MARKER} [{folder}/]({rel}/), one file per subject, each naming the code it covers.',
                  'Read this table, then the pieces whose Code paths cover what you change.',
                  f'The headings after the table keep every section name and anchor this file had, so a reference such as '
                  f'`{source}, <section>` still lands here and names its piece.', '',
                  '| Piece | Code |', '| --- | --- |']
    for piece in pieces:
        toc.append(f'| [{piece["title"]}]({rel}/{file_of[piece["name"]]}) | {code_line(piece["code"], checkout)} |')
    toc.append('')
    title_of = {piece['name']: piece['title'] for piece in pieces}
    for index, _, heading in headings[1:]:
        owner = owner_at[index]
        toc.extend([lines[index], '', f'Moved to [{title_of[owner]}]({rel}/{file_of[owner]}#{DOCS.slug(heading)}).', ''])
    outputs[source] = '\n'.join(toc).rstrip('\n') + '\n'

    if sorted(covered) != list(range(len(lines))):
        raise Refused('a source line was dropped or placed twice')
    lost = set(anchor_home) - set(anchor_list(outputs[source]))
    if lost:
        raise Refused(f'the table of contents lost anchors: {sorted(lost)}')
    # Every link between the outputs reaches a heading that is there.
    for name, output in outputs.items():
        here = os.path.dirname(name)
        for target in links(output):
            path, _, anchor = target.partition('#')
            if SCHEME.match(target) or not anchor:
                continue
            goal = os.path.normpath(os.path.join(here, path)) if path else name
            if goal in outputs and anchor not in anchor_list(outputs[goal]):
                raise Refused(f'{name} links {target}, which no heading in {goal} answers')

    stale = [name for name in existing if name not in outputs]
    allowlist, herdr_before, herdr_after = allowlist_update(checkout, commit, source, existing, outputs)
    report = [
        f'split-doc: {source} at {commit[:10]} ({len(text.encode())} bytes, {len(lines)} lines) '
        f'-> {len(pieces)} pieces and the table of contents',
        f'split-doc: {changed} moved lines differ from the source, only in heading level or link target',
        f'split-doc: Herdr names {herdr_before} in {source} -> {herdr_after} across the outputs',
    ]
    for name in sorted(outputs):
        size = len(outputs[name].encode())
        if size > DOCS.LIMIT:
            report.append(f'split-doc: over {DOCS.LIMIT // 1024} KB: {name} ({size // 1024} KB); '
                          f'list it in EXCEPTIONS or split it further')
    outputs[ALLOWLIST] = allowlist
    return outputs, stale, report


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--source-ref', required=True)
    parser.add_argument('--table', required=True)
    parser.add_argument('--checkout', default=str(ROOT))
    args = parser.parse_args()
    checkout = Path(args.checkout).resolve()
    table = json.loads(Path(args.table).read_text(encoding='utf-8'))
    try:
        outputs, stale, report = split(checkout, args.source_ref, table)
    except Refused as refused:
        sys.exit(f'split-doc: {refused}')
    (checkout / table['folder']).mkdir(parents=True, exist_ok=True)
    for name in stale:
        (checkout / name).unlink()
    for name, text in outputs.items():
        (checkout / name).write_text(text, encoding='utf-8')
    print('\n'.join(report))


if __name__ == '__main__':
    main()
