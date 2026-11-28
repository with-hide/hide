"""The docs check must pass a well-formed checkout and name each break of the
cap, the exception list, a split table and its over-cap pieces, a decision
record's form, a guide piece's header and a table of contents.

Each test builds a small git checkout in a temporary folder (records, a table
of contents, pieces and one code path) and runs the check on it, never on the
real one.
"""
import importlib.util
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = importlib.util.spec_from_file_location('check_docs', ROOT / 'scripts' / 'check-docs.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def record(number, title='A decision', status='Accepted', date='2026-11-28', code='`src/`', body='Text.\n'):
    head = [f'# {number}. {title}', '']
    if status is not None:
        head.append(f'- Status: {status}')
    if date is not None:
        head.append(f'- Date: {date}')
    if code is not None:
        head.append(f'- Code: {code}')
    return '\n'.join(head + ['', '## Decision', '', '### Section', '', body])


def piece(title='A piece', code='`src/`', index='[Guide](../GUIDE.md)', body='Text.\n'):
    head = [f'# {title}', '']
    if code is not None:
        head.append(f'- Code: {code}')
    if index is not None:
        head.append(f'- Index: {index}')
    return '\n'.join(head + ['', '## Section', '', body])


def table(pieces, source='docs/GUIDE.md', folder='docs/guide'):
    """A split table's text, as scripts/split-tables/ holds it; `pieces` are names or piece dicts."""
    return json.dumps({'source': source, 'folder': folder,
                       'pieces': [{'name': p} if isinstance(p, str) else p for p in pieces]})


def adr_toc(records):
    return '# Runtime architecture\n\n' + ''.join(f'- [{name}](adr/{name})\n' for name in records)


class Checkout:
    """A git checkout with records under docs/adr/ and any other files given."""

    def __init__(self, records=None, toc=None, files=None):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        (self.root / 'src').mkdir()
        records = {'0001-first.md': record('0001')} if records is None else records
        (self.root / 'docs' / 'adr').mkdir(parents=True)
        for name, text in records.items():
            (self.root / 'docs' / 'adr' / name).write_text(text, encoding='utf-8')
        (self.root / 'docs' / 'ARCHITECTURE.md').write_text(adr_toc(records) if toc is None else toc, encoding='utf-8')
        for name, text in (files or {}).items():
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            (self.root / name).write_text(text, encoding='utf-8')

    def errors(self):
        try:
            return CHECK.check(self.root)
        finally:
            self.folder.cleanup()


def errors(records=None, toc=None, files=None):
    return Checkout(records, toc, files).errors()


class Base(unittest.TestCase):
    def setUp(self):
        self.saved = CHECK.EXCEPTIONS, CHECK.SPLITS
        CHECK.EXCEPTIONS = set()
        CHECK.SPLITS = {'docs/ARCHITECTURE.md': 'docs/adr'}

    def tearDown(self):
        CHECK.EXCEPTIONS, CHECK.SPLITS = self.saved

    def assertNamed(self, found, words):
        self.assertTrue(any(words in error for error in found), found)


class TheCap(Base):
    def test_a_file_over_the_cap_fails_with_its_size_and_the_template_named(self):
        found = errors(files={'docs/GROWN.md': 'x' * (CHECK.LIMIT + 1)})
        self.assertNamed(found, f'docs/GROWN.md: {CHECK.LIMIT + 1} bytes, over the {CHECK.LIMIT}-byte cap')
        self.assertNamed(found, CHECK.TEMPLATE)

    def test_a_file_at_the_cap_passes_and_bytes_not_characters_count(self):
        self.assertEqual(errors(files={'docs/FULL.md': 'x' * CHECK.LIMIT}), [])
        found = errors(files={'docs/WIDE.md': '가' * (CHECK.LIMIT // 3 + 1)})
        self.assertNamed(found, 'docs/WIDE.md:')

    def test_an_ignored_or_non_markdown_file_is_not_counted(self):
        big = 'x' * (CHECK.LIMIT + 1)
        self.assertEqual(errors(files={'.gitignore': 'scratch/\n', 'scratch/BIG.md': big, 'docs/big.txt': big}), [])

    def test_a_listed_file_passes_and_must_leave_the_list_when_small_or_gone(self):
        CHECK.EXCEPTIONS = {'docs/GROWN.md'}
        self.assertEqual(errors(files={'docs/GROWN.md': 'x' * (CHECK.LIMIT + 1)}), [])
        self.assertNamed(errors(files={'docs/GROWN.md': 'x'}), 'names docs/GROWN.md, now within the cap; remove the entry')
        self.assertNamed(errors(), 'names docs/GROWN.md, which does not exist; remove the entry')

    def test_a_record_over_the_cap_fails_unless_listed(self):
        big = {'0001-first.md': record('0001', body='x' * (CHECK.LIMIT + 1))}
        self.assertNamed(errors(big), 'docs/adr/0001-first.md:')
        CHECK.EXCEPTIONS = {'docs/adr/0001-first.md'}
        self.assertEqual(errors(big), [])


class TheRecords(Base):
    def test_a_well_formed_set_passes(self):
        self.assertEqual(errors({
            '0001-first.md': record('0001'),
            '0002-second.md': record('0002', status='Superseded by 0001'),
        }, toc='# T\n\n[a](adr/0001-first.md#section)\n[b](adr/0002-second.md)\n'), [])

    def test_two_records_with_one_number_fail(self):
        self.assertNamed(errors({'0001-first.md': record('0001'), '0001-again.md': record('0001')}), 'number 0001 is also')

    def test_a_badly_named_file_fails(self):
        self.assertNamed(errors({'1-First.md': record('0001')}), 'not named NNNN-<slug>.md')

    def test_a_title_with_another_number_fails(self):
        self.assertNamed(errors({'0001-first.md': record('0002')}), 'first line must be "# 0001.')

    def test_each_missing_field_is_named(self):
        for field in ('Status', 'Date', 'Code'):
            with self.subTest(field=field):
                self.assertNamed(errors({'0001-first.md': record('0001', **{field.lower(): None})}), f'no "- {field}:"')

    def test_a_field_out_of_its_form_fails(self):
        cases = [
            ({'status': 'Done'}, 'is not Accepted'),
            ({'status': 'Superseded by 0009'}, 'which is no record'),
            ({'date': '28 Nov'}, 'is not YYYY-MM-DD'),
            ({'code': '`src/gone.rs`'}, 'src/gone.rs does not exist'),
            ({'code': 'src/'}, 'names no path in backticks'),
        ]
        for fields, words in cases:
            with self.subTest(fields=fields):
                self.assertNamed(errors({'0001-first.md': record('0001', **fields)}), words)

    def test_a_record_without_a_decision_fails(self):
        text = record('0001').replace('## Decision', '## Notes')
        self.assertNamed(errors({'0001-first.md': text}), 'no "## Decision"')

    def test_a_file_in_the_records_folder_that_is_not_a_record_fails(self):
        self.assertNamed(errors(files={'docs/adr/0002-notes.txt': 'x'}), 'docs/adr/0002-notes.txt: not named NNNN-<slug>.md')
        self.assertEqual(errors(files={'.gitignore': '.DS_Store\n', 'docs/adr/.DS_Store': 'x'}), [], 'an ignored file is no record')

    def test_the_table_of_contents_must_reach_every_record_and_only_real_anchors(self):
        records = {'0001-first.md': record('0001'), '0002-second.md': record('0002')}
        found = errors(records, toc='# T\n\n[a](adr/0001-first.md#gone)\n[c](adr/0003-third.md)\n')
        self.assertNamed(found, 'does not link docs/adr/0002-second.md')
        self.assertNamed(found, '0001-first.md#gone, which no heading')
        self.assertNamed(found, 'links adr/0003-third.md, which is no piece')


class TheGuidePieces(Base):
    TOC = '# Guide\n\n| Piece | Code |\n| --- | --- |\n| [A](guide/a.md) | `src/` |\n\n## Section\n\nMoved to [A](guide/a.md#section).\n'

    def guide(self, pieces, toc=None, split=None):
        files = {'docs/GUIDE.md': self.TOC if toc is None else toc,
                 'scripts/split-tables/guide.json': table([name[:-3] for name in pieces]) if split is None else split}
        files.update({f'docs/guide/{name}': text for name, text in pieces.items()})
        return errors(files=files)

    def test_a_well_formed_guide_passes(self):
        self.assertEqual(self.guide({'a.md': piece(index='[Guide](../GUIDE.md#section)')}), [])

    def test_a_titled_or_dot_relative_table_link_counts(self):
        toc = '# Guide\n\n[a](./guide/a.md "the piece")\n[b](<guide/b.md#section>)\n'
        self.assertEqual(self.guide({'a.md': piece(), 'b.md': piece()}, toc=toc), [])

    def test_headings_inside_any_fence_are_not_anchors(self):
        text = '# T\n\n~~~\n## tilde\n~~~\n\n````md\n```\n## inner\n```\n````\n\n## Real [link](x.md)\n'
        self.assertEqual(CHECK.anchors(text), {'t', 'real-link'})
        self.assertFalse(CHECK.structure(['```', '## open'])[2], 'an unclosed fence is reported')

    def test_each_missing_field_is_named(self):
        for field in ('Code', 'Index'):
            with self.subTest(field=field):
                self.assertNamed(self.guide({'a.md': piece(**{field.lower(): None})}), f'docs/guide/a.md: no "- {field}:"')

    def test_a_field_after_the_first_heading_does_not_count(self):
        text = '# A piece\n\n## Section\n\n- Code: `src/`\n- Index: [Guide](../GUIDE.md)\n'
        self.assertNamed(self.guide({'a.md': text}), 'no "- Code:"')

    def test_a_piece_without_a_title_fails(self):
        self.assertNamed(self.guide({'a.md': 'A piece\n' + piece()}), 'the first line must be "# <Title>"')

    def test_a_code_path_must_exist(self):
        self.assertNamed(self.guide({'a.md': piece(code='`src/gone.rs`')}), 'src/gone.rs does not exist')

    def test_an_index_must_link_its_own_table_of_contents(self):
        for index in ('[Other](../OTHER.md)', 'the guide', '[Guide](GUIDE.md)'):
            with self.subTest(index=index):
                self.assertNamed(self.guide({'a.md': piece(index=index)}), 'Index does not link its table of contents, docs/GUIDE.md')

    def test_the_table_of_contents_must_reach_every_piece_and_only_real_anchors(self):
        toc = '# Guide\n\n[a](guide/a.md#gone)\n[c](guide/c.md)\n'
        found = self.guide({'a.md': piece(), 'b.md': piece()}, toc=toc)
        self.assertNamed(found, 'does not link docs/guide/b.md')
        self.assertNamed(found, 'guide/a.md#gone, which no heading')
        self.assertNamed(found, 'links guide/c.md, which is no piece')

    def test_a_registered_split_needs_its_folder_and_its_table_of_contents(self):
        self.assertNamed(self.guide({}), 'docs/guide/ holds no pieces')
        self.assertNamed(errors(files={'scripts/split-tables/guide.json': table(['a']), 'docs/guide/a.md': piece()}),
                         'docs/GUIDE.md: the table of contents of docs/guide/ does not exist')


class TheSplitTables(Base):
    """A split is learned from its own table file, never from a list another split edits."""

    TOC = TheGuidePieces.TOC
    guide = TheGuidePieces.guide

    def test_a_split_is_checked_only_once_its_table_file_names_it(self):
        broken = {'docs/GUIDE.md': '# Guide\n', 'docs/guide/a.md': piece(code='`src/gone.rs`')}
        self.assertEqual(errors(files=broken), [], 'without a table the folder is no split')
        found = errors(files=dict(broken, **{'scripts/split-tables/guide.json': table(['a'])}))
        self.assertNamed(found, 'src/gone.rs does not exist')
        self.assertNamed(found, 'does not link docs/guide/a.md')

    def test_the_table_names_exactly_the_pieces_its_folder_holds(self):
        found = self.guide({'a.md': piece(), 'b.md': piece()}, split=table(['a', 'c']),
                           toc='# Guide\n\n[a](guide/a.md)\n[b](guide/b.md)\n')
        self.assertNamed(found, 'scripts/split-tables/guide.json: names the piece docs/guide/c.md, which does not exist')
        self.assertNamed(found, 'docs/guide/b.md: no piece of scripts/split-tables/guide.json; add it to the table')

    def test_a_malformed_table_or_a_second_split_of_one_document_fails(self):
        for text in ('{', '{"source": "docs/GUIDE.md"}', table([{'name': 'a', 'over_cap': 'yes'}])):
            with self.subTest(text=text):
                self.assertNamed(self.guide({'a.md': piece()}, split=text), 'scripts/split-tables/guide.json: not a split table')
        again = {'scripts/split-tables/other.json': table(['a'], folder='docs/other'), 'docs/other/a.md': piece()}
        found = errors(files=dict(again, **{'docs/GUIDE.md': self.TOC, 'docs/guide/a.md': piece(),
                                            'scripts/split-tables/guide.json': table(['a'])}))
        self.assertNamed(found, 'scripts/split-tables/other.json: splits docs/GUIDE.md, which scripts/split-tables/guide.json already splits')
        found = errors(files={'scripts/split-tables/adr.json': table(['a'], source='docs/ARCHITECTURE.md')})
        self.assertNamed(found, 'splits docs/ARCHITECTURE.md, which SPLITS already splits')

    def test_a_piece_over_the_cap_is_allowed_only_by_its_over_cap_mark(self):
        big = piece(body='x' * CHECK.LIMIT)
        self.assertNamed(self.guide({'a.md': big}), f'docs/guide/a.md: {len(big.encode())} bytes, over the')
        self.assertEqual(self.guide({'a.md': big}, split=table([{'name': 'a', 'over_cap': True}])), [])

    def test_an_over_cap_mark_on_a_small_or_still_divisible_piece_fails(self):
        marked = table([{'name': 'a', 'over_cap': True}])
        self.assertNamed(self.guide({'a.md': piece()}, split=marked),
                         'scripts/split-tables/guide.json marks docs/guide/a.md over_cap, now within the cap; remove the mark')
        two = piece(body='x' * CHECK.LIMIT + '\n\n## Another\n\nText.\n')
        self.assertNamed(self.guide({'a.md': two}, split=marked), 'more than one section heading; split it further')


def exceptions_layout(text):
    """The EXCEPTIONS entries written in a check-docs.py text, and its layout problems: an entry
    not alone on its line, or two entries on neighbouring lines. Blank lines a split left
    behind are allowed."""
    start = text.index('EXCEPTIONS = {\n') + len('EXCEPTIONS = {\n')
    lines = text[start:text.index('\n}\n', start - 1) + 1].split('\n')
    entries, problems, last = [], [], None
    for number, line in enumerate(lines):
        if not line.strip():
            continue
        match = re.match(r"^    '([^']+)',$", line)
        if not match:
            problems.append(f'not an entry alone on its line: {line!r}')
            continue
        if last is not None and number == last + 1:
            problems.append(f'{entries[-1]} and {match.group(1)} are on neighbouring lines')
        entries.append(match.group(1))
        last = number
    return entries, problems


class TheExceptionsLayout(unittest.TestCase):
    """Two splits each remove their own EXCEPTIONS entry; a blank line between entries keeps
    those removals off neighbouring lines, so git merges them (test_split_doc merges two)."""

    def test_the_real_list_keeps_every_entry_apart(self):
        entries, problems = exceptions_layout((ROOT / 'scripts' / 'check-docs.py').read_text(encoding='utf-8'))
        self.assertEqual(problems, [])
        self.assertEqual(sorted(entries), sorted(CHECK.EXCEPTIONS))

    def test_the_list_stays_apart_after_each_split_removes_its_entry(self):
        split = importlib.util.spec_from_file_location('split_doc', ROOT / 'scripts' / 'split-doc.py')
        module = importlib.util.module_from_spec(split)
        split.loader.exec_module(module)
        for source in sorted(CHECK.EXCEPTIONS):
            with self.subTest(source=source):
                entries, problems = exceptions_layout(module.exceptions_update(ROOT, source))
                self.assertEqual(problems, [])
                self.assertEqual(sorted(entries), sorted(CHECK.EXCEPTIONS - {source}))

    def test_neighbouring_or_shared_lines_are_problems(self):
        self.assertEqual(exceptions_layout("EXCEPTIONS = {\n    'a',\n    'b',\n}\n")[1], ['a and b are on neighbouring lines'])
        self.assertEqual(len(exceptions_layout("EXCEPTIONS = {\n    'a', 'b',\n}\n")[1]), 1)


if __name__ == '__main__':
    unittest.main()
