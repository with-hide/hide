#!/usr/bin/env python3
"""Every Markdown file of the repository stays small enough to be read whole,
and every split document's table of contents reaches its pieces.

docs/README.md, Splitting a document, owns the rules for a guide;
docs/adr/0001-record-architecture-decisions.md owns them for a decision
record. This is their check, and scripts/hooks/docs-size-cap.sh reads its
LIMIT and EXCEPTIONS to refuse an oversized edit at write time. It fails when:

- a Markdown file git tracks (or would track: not ignored) is over LIMIT and
  not on EXCEPTIONS;
- an EXCEPTIONS entry names a file that does not exist or is at or under
  LIMIT (the list only shrinks);
- a file in docs/adr/ is not named `NNNN-<slug>.md`, or two share a number;
  a record's first line is not `# NNNN. <Title>` with its own number; a record
  lacks a field (`Status`, `Date`, `Code`), a field's value is not of its
  form, a `Superseded by` names no record; a record has no `## Decision`;
- a guide piece does not start with `# <Title>`, or lacks `- Code:` or
  `- Index:` before its first heading, or its Index does not link its own
  table of contents;
- a Code path does not exist;
- a table of contents in SPLITS does not link one of its pieces, or links a
  piece or an anchor that does not exist.

Usage: scripts/check-docs.py   (run from anywhere in the checkout)
"""
import datetime
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LIMIT = 60 * 1024
TEMPLATE = 'docs/README.md, Splitting a document'
# Over the cap when the cap arrived; each leaves the list when it is split.
EXCEPTIONS = {
    'CONTRIBUTING.md',
    'docs/AI_PROVIDERS.md',
    'docs/BROWSER_DISPLAYS.md',
    'docs/BUILD.md',
    'docs/INSTALL.md',
    'docs/TESTING.md',
    'docs/UI_BEHAVIOR.md',
    'docs/VERIFICATION.md',
    'docs/adr/0004-drawn-ahead-of-the-engine.md',
    'docs/adr/0005-platform-layer.md',
    'docs/adr/0013-core-own-engine.md',
    'docs/adr/0050-web-workspaces.md',
    'docs/adr/0059-mobile-companion.md',
    'docs/agent-hooks.md',
    'docs/delivery.md',
    'docs/performance/resident-ticks.md',
    'docs/status-model.md',
    'hided/assets/mission/help.md',
}
# Each split document: its table of contents and the folder of its pieces.
SPLITS = {
    'docs/ARCHITECTURE.md': 'docs/adr',
    'docs/PERFORMANCE_TESTING.md': 'docs/performance',
}
# The pieces in this folder are decision records, in ADR 0001's form.
RECORDS = 'docs/adr'
NAME = re.compile(r'^(\d{4})-[a-z0-9]+(?:-[a-z0-9]+)*\.md$')
RECORD_FIELD = re.compile(r'^- (Status|Date|Code): (.+)$')
PIECE_FIELD = re.compile(r'^- (Code|Index): (.+)$')
STATUS = re.compile(r'^(Accepted|Deprecated|Superseded by (\d{4}))$')
HEADING = re.compile(r'^(#{1,6}) (.+?)\s*$')
FENCE = re.compile(r'^ {0,3}(`{3,}|~{3,})')
INLINE_LINK = re.compile(r'!?\[([^\]]*)\]\([^)]*\)')
TARGET = re.compile(r'\]\(([^)\s#]*)(?:#[^)\s]*)?\)')


def structure(lines):
    """A document's headings outside code fences as (line index, level, text), the indices
    of the lines inside a fence (its markers included), and whether every fence closed; a
    fence closes on a line of its own character at least as long. scripts/split-doc.py
    reads documents through this too, so the two never disagree on what a heading is."""
    found, fenced, fence = [], set(), None
    for index, line in enumerate(lines):
        if fence:
            fenced.add(index)
            stripped = line.strip()
            if stripped and set(stripped) == {fence[0]} and len(stripped) >= len(fence):
                fence = None
            continue
        opener = FENCE.match(line)
        if opener:
            fenced.add(index)
            fence = opener.group(1)
            continue
        match = HEADING.match(line)
        if match:
            found.append((index, len(match.group(1)), match.group(2)))
    return found, fenced, fence is None


def slug(text):
    """GitHub's anchor for a heading: rendered text lowercased, punctuation dropped, spaces to hyphens."""
    text = INLINE_LINK.sub(r'\1', text).replace('`', '').lower()
    return re.sub(r'[^\w\- ]', '', text).replace(' ', '-')


def anchors(text):
    """GitHub's heading anchors in `text`, code fences left out."""
    return {slug(heading) for _, _, heading in structure(text.split('\n'))[0]}


def listed(root, pathspec):
    """The files git tracks or would track (not ignored) under `pathspec`, present on disk."""
    done = subprocess.run(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', pathspec],
        cwd=root, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f'check-docs: git ls-files failed in {root}: {done.stderr.strip()}')
    return sorted({name for name in done.stdout.split('\0') if name and (root / name).is_file()})


def markdown_files(root):
    return listed(root, '*.md')


def code_errors(root, where, value):
    paths = re.findall(r'`([^`]+)`', value)
    if not paths:
        return [f'{where}: Code names no path in backticks']
    return [f'{where}: Code path {path} does not exist' for path in paths if not (root / path.rstrip('/')).exists()]


def header(lines, pattern):
    """The fields between the title line and the first heading."""
    fields = {}
    for line in lines[1:]:
        if line.startswith('#'):
            break
        match = pattern.match(line)
        if match:
            fields[match.group(1)] = match.group(2).strip()
    return fields


def check_record(root, path, text, numbers):
    errors = []
    lines = text.split('\n')
    number = Path(path).name[:4]
    if not re.match(rf'^# {number}\. \S', lines[0]):
        errors.append(f'{path}: the first line must be "# {number}. <Title>"')
    fields = header(lines, RECORD_FIELD)
    for field in ('Status', 'Date', 'Code'):
        if field not in fields:
            errors.append(f'{path}: no "- {field}:" line before the first heading')
    status = STATUS.match(fields.get('Status', ''))
    if 'Status' in fields and not status:
        errors.append(f'{path}: Status "{fields["Status"]}" is not Accepted, Deprecated or Superseded by NNNN')
    elif status and status.group(2) and status.group(2) not in numbers:
        errors.append(f'{path}: Superseded by {status.group(2)}, which is no record')
    if 'Date' in fields:
        try:
            datetime.date.fromisoformat(fields['Date'])
        except ValueError:
            errors.append(f'{path}: Date "{fields["Date"]}" is not YYYY-MM-DD')
    if 'Code' in fields:
        errors.extend(code_errors(root, path, fields['Code']))
    if not re.search(r'^## Decision\s*$', text, re.MULTILINE):
        errors.append(f'{path}: no "## Decision" section')
    return errors


def check_piece(root, path, text, toc):
    errors = []
    lines = text.split('\n')
    if not re.match(r'^# \S', lines[0]):
        errors.append(f'{path}: the first line must be "# <Title>"')
    fields = header(lines, PIECE_FIELD)
    for field in ('Code', 'Index'):
        if field not in fields:
            errors.append(f'{path}: no "- {field}:" line before the first heading')
    if 'Code' in fields:
        errors.extend(code_errors(root, path, fields['Code']))
    if 'Index' in fields:
        here = (root / path).parent
        targets = {os.path.normpath(here / target) for target in TARGET.findall(fields['Index']) if target}
        if os.path.normpath(root / toc) not in targets:
            errors.append(f'{path}: Index does not link its table of contents, {toc}')
    return errors


def check_records(folder, names):
    """The decision records' names and numbers: (records' numbers, errors)."""
    errors, numbers = [], {}
    for name in names:
        match = NAME.match(name)
        if not match:
            errors.append(f'{folder}/{name}: not named NNNN-<slug>.md')
            continue
        if match.group(1) in numbers:
            errors.append(f'{folder}/{name}: number {match.group(1)} is also {numbers[match.group(1)]}')
        numbers.setdefault(match.group(1), name)
    return numbers, errors


def check_split(root, toc, folder):
    """A table of contents and its folder of pieces."""
    directory = root / folder
    files = [name[len(folder) + 1:] for name in listed(root, folder) if '/' not in name[len(folder) + 1:]]
    names = [name for name in files if name.endswith('.md')]
    if not names:
        return [f'{folder}/ holds no pieces']
    if not (root / toc).is_file():
        return [f'{toc}: the table of contents of {folder}/ does not exist']
    errors = []
    if folder == RECORDS:
        errors.extend(f'{folder}/{name}: not named NNNN-<slug>.md' for name in files if not name.endswith('.md'))
    texts = {name: (directory / name).read_text(encoding='utf-8') for name in names}
    if folder == RECORDS:
        numbers, found = check_records(folder, names)
        errors.extend(found)
        for name in names:
            if NAME.match(name):
                errors.extend(check_record(root, f'{folder}/{name}', texts[name], numbers))
    else:
        for name in names:
            errors.extend(check_piece(root, f'{folder}/{name}', texts[name], toc))
    rel = os.path.relpath(folder, os.path.dirname(toc)).replace(os.sep, '/')
    link = re.compile(rf'\]\(<?(?:\./)?{re.escape(rel)}/([^)#\s>]+)(?:#([^)\s>]+))?>?(?:\s+"[^"]*")?\)')
    linked = set()
    for name, anchor in link.findall((root / toc).read_text(encoding='utf-8')):
        linked.add(name)
        if name not in texts:
            errors.append(f'{toc}: links {rel}/{name}, which is no piece')
        elif anchor and anchor not in anchors(texts[name]):
            errors.append(f'{toc}: links {rel}/{name}#{anchor}, which no heading there answers')
    for name in sorted(set(texts) - linked):
        errors.append(f'{toc}: does not link {folder}/{name}; add its row to the table')
    return errors


def check(root):
    errors = []
    files = markdown_files(root)
    for path in files:
        bytes_ = (root / path).stat().st_size
        if bytes_ > LIMIT and path not in EXCEPTIONS:
            errors.append(f'{path}: {bytes_} bytes, over the {LIMIT}-byte cap; '
                          f'edit the sentence in place or split the file as {TEMPLATE} describes')
    for path in sorted(EXCEPTIONS):
        if not (root / path).is_file():
            errors.append(f'EXCEPTIONS in scripts/check-docs.py names {path}, which does not exist; remove the entry')
        elif (root / path).stat().st_size <= LIMIT:
            errors.append(f'EXCEPTIONS in scripts/check-docs.py names {path}, now within the cap; remove the entry')
    for toc, folder in SPLITS.items():
        errors.extend(check_split(root, toc, folder))
    return errors


def main():
    errors = check(ROOT)
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        print(f'See {TEMPLATE} and docs/adr/0001-record-architecture-decisions.md.', file=sys.stderr)
        return 1
    print(f'check-docs: {len(markdown_files(ROOT))} Markdown files within {LIMIT // 1024} KB or excepted '
          f'({len(EXCEPTIONS)} excepted), {len(SPLITS)} split documents complete')
    return 0


if __name__ == '__main__':
    sys.exit(main())
