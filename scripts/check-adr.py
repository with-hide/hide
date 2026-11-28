#!/usr/bin/env python3
"""The architecture decision records under docs/adr/ keep their form, and the
table of contents in docs/ARCHITECTURE.md reaches every one.

docs/adr/0001-record-architecture-decisions.md owns the rules; this is their
check. It fails when:

- a file in docs/adr/ is not named `NNNN-<slug>.md`, or two share a number;
- a record's first line is not `# NNNN. <Title>` with its own number;
- a record lacks a field (`Status`, `Date`, `Code`), a field's value is not of
  its form, a `Superseded by` names no record, or a Code path does not exist;
- a record has no `## Decision`;
- a record is over 60 KB and not on the known list below, or a listed record
  has come under the limit (the list only shrinks);
- docs/ARCHITECTURE.md does not link a record, or links a record or an anchor
  that does not exist.

Usage: scripts/check-adr.py   (run from anywhere in the checkout)
"""
import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ADR_DIR = 'docs/adr'
TOC = 'docs/ARCHITECTURE.md'
LIMIT = 60 * 1024
# Over the limit when they moved out of docs/ARCHITECTURE.md; each is split by a follow-up.
OVERSIZE = {
    '0004-drawn-ahead-of-the-engine.md',
    '0005-platform-layer.md',
    '0013-core-own-engine.md',
    '0050-web-workspaces.md',
    '0059-mobile-companion.md',
}
NAME = re.compile(r'^(\d{4})-[a-z0-9]+(?:-[a-z0-9]+)*\.md$')
FIELD = re.compile(r'^- (Status|Date|Code): (.+)$')
STATUS = re.compile(r'^(Accepted|Deprecated|Superseded by (\d{4}))$')
LINK = re.compile(r'\]\(adr/([^)#\s]+)(?:#([^)\s]+))?\)')
HEADING = re.compile(r'^#{1,6} (.+?)\s*$', re.MULTILINE)


def anchors(text):
    """GitHub's heading anchors in `text`, code fences left out."""
    text = re.sub(r'^```.*?^```', '', text, flags=re.MULTILINE | re.DOTALL)
    found = set()
    for heading in HEADING.findall(text):
        anchor = re.sub(r'[^\w\- ]', '', heading.replace('`', '').lower()).replace(' ', '-')
        found.add(anchor)
    return found


def check_record(root, name, text, numbers):
    errors = []
    lines = text.split('\n')
    number = name[:4]
    if not re.match(rf'^# {number}\. \S', lines[0]):
        errors.append(f'{name}: the first line must be "# {number}. <Title>"')
    fields = {}
    for line in lines[1:]:
        if line.startswith('## '):
            break
        match = FIELD.match(line)
        if match:
            fields[match.group(1)] = match.group(2).strip()
    for field in ('Status', 'Date', 'Code'):
        if field not in fields:
            errors.append(f'{name}: no "- {field}:" line before the first "## " heading')
    status = STATUS.match(fields.get('Status', ''))
    if 'Status' in fields and not status:
        errors.append(f'{name}: Status "{fields["Status"]}" is not Accepted, Deprecated or Superseded by NNNN')
    elif status and status.group(2) and status.group(2) not in numbers:
        errors.append(f'{name}: Superseded by {status.group(2)}, which is no record')
    if 'Date' in fields:
        try:
            datetime.date.fromisoformat(fields['Date'])
        except ValueError:
            errors.append(f'{name}: Date "{fields["Date"]}" is not YYYY-MM-DD')
    if 'Code' in fields:
        paths = re.findall(r'`([^`]+)`', fields['Code'])
        if not paths:
            errors.append(f'{name}: Code names no path in backticks')
        for path in paths:
            if not (root / path.rstrip('/')).exists():
                errors.append(f'{name}: Code path {path} does not exist')
    if not re.search(r'^## Decision\s*$', text, re.MULTILINE):
        errors.append(f'{name}: no "## Decision" section')
    size = len(text.encode('utf-8'))
    if size > LIMIT and name not in OVERSIZE:
        errors.append(f'{name}: {size} bytes, over the {LIMIT}-byte limit; split the decision into records')
    if size <= LIMIT and name in OVERSIZE:
        errors.append(f'{name}: now within the limit; remove it from OVERSIZE in scripts/check-adr.py')
    return errors


def check(root):
    errors = []
    folder = root / ADR_DIR
    names = sorted(path.name for path in folder.iterdir()) if folder.is_dir() else []
    if not names:
        return [f'{ADR_DIR}/ holds no records']
    records, numbers = {}, {}
    for name in names:
        match = NAME.match(name)
        if not match:
            errors.append(f'{ADR_DIR}/{name}: not named NNNN-<slug>.md')
            continue
        if match.group(1) in numbers:
            errors.append(f'{ADR_DIR}/{name}: number {match.group(1)} is also {numbers[match.group(1)]}')
        numbers.setdefault(match.group(1), name)
        records[name] = (folder / name).read_text(encoding='utf-8')
    for name in OVERSIZE - set(records):
        errors.append(f'OVERSIZE in scripts/check-adr.py names {name}, which is no record')
    for name, text in records.items():
        errors.extend(check_record(root, name, text, numbers))
    toc = (root / TOC).read_text(encoding='utf-8')
    linked = set()
    for name, anchor in LINK.findall(toc):
        linked.add(name)
        if name not in records:
            errors.append(f'{TOC}: links adr/{name}, which is no record')
        elif anchor and anchor not in anchors(records[name]):
            errors.append(f'{TOC}: links adr/{name}#{anchor}, which no heading there answers')
    for name in sorted(set(records) - linked):
        errors.append(f'{TOC}: does not link {ADR_DIR}/{name}; add its row to the table')
    return errors


def main():
    errors = check(ROOT)
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        print('See docs/adr/0001-record-architecture-decisions.md.', file=sys.stderr)
        return 1
    print(f'check-adr: {len(list((ROOT / ADR_DIR).glob("*.md")))} records, table of contents complete')
    return 0


if __name__ == '__main__':
    sys.exit(main())
