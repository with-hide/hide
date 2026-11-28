"""The decision-record check must pass a well-formed set and name each break of the form.

Each test builds a small checkout in a temporary folder (records, a table of
contents and one code path) and runs the check on it, never on the real one.
"""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = importlib.util.spec_from_file_location('check_adr', ROOT / 'scripts' / 'check-adr.py')
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


class Checkout:
    def __init__(self, records, toc=None):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        (self.root / 'src').mkdir()
        (self.root / 'docs' / 'adr').mkdir(parents=True)
        for name, text in records.items():
            (self.root / 'docs' / 'adr' / name).write_text(text, encoding='utf-8')
        if toc is None:
            toc = '# Runtime architecture\n\n' + ''.join(f'- [{name}](adr/{name})\n' for name in records)
        (self.root / 'docs' / 'ARCHITECTURE.md').write_text(toc, encoding='utf-8')

    def errors(self):
        try:
            return CHECK.check(self.root)
        finally:
            self.folder.cleanup()


def errors(records, toc=None):
    return Checkout(records, toc).errors()


class TheCheck(unittest.TestCase):
    def setUp(self):
        self.oversize = CHECK.OVERSIZE
        CHECK.OVERSIZE = set()

    def tearDown(self):
        CHECK.OVERSIZE = self.oversize

    def test_a_well_formed_set_passes(self):
        self.assertEqual(errors({
            '0001-first.md': record('0001'),
            '0002-second.md': record('0002', status='Superseded by 0001'),
        }, toc='# T\n\n[a](adr/0001-first.md#section)\n[b](adr/0002-second.md)\n'), [])

    def test_two_records_with_one_number_fail(self):
        found = errors({'0001-first.md': record('0001'), '0001-again.md': record('0001')})
        self.assertTrue(any('number 0001 is also' in error for error in found), found)

    def test_a_badly_named_file_fails(self):
        found = errors({'1-First.md': record('0001')})
        self.assertTrue(any('not named NNNN-<slug>.md' in error for error in found), found)

    def test_a_title_with_another_number_fails(self):
        found = errors({'0001-first.md': record('0002')})
        self.assertTrue(any('first line must be "# 0001.' in error for error in found), found)

    def test_each_missing_field_is_named(self):
        for field in ('Status', 'Date', 'Code'):
            with self.subTest(field=field):
                found = errors({'0001-first.md': record('0001', **{field.lower(): None})})
                self.assertTrue(any(f'no "- {field}:"' in error for error in found), found)

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
                found = errors({'0001-first.md': record('0001', **fields)})
                self.assertTrue(any(words in error for error in found), found)

    def test_a_record_without_a_decision_fails(self):
        text = record('0001').replace('## Decision', '## Notes')
        found = errors({'0001-first.md': text})
        self.assertTrue(any('no "## Decision"' in error for error in found), found)

    def test_a_record_over_the_limit_fails_unless_listed(self):
        big = record('0001', body='x' * (CHECK.LIMIT + 1))
        found = errors({'0001-first.md': big})
        self.assertTrue(any('over the' in error for error in found), found)
        CHECK.OVERSIZE = {'0001-first.md'}
        self.assertEqual(errors({'0001-first.md': big}), [])

    def test_a_listed_record_under_the_limit_must_leave_the_list(self):
        CHECK.OVERSIZE = {'0001-first.md'}
        found = errors({'0001-first.md': record('0001')})
        self.assertTrue(any('remove it from OVERSIZE' in error for error in found), found)

    def test_the_table_of_contents_must_reach_every_record_and_only_real_anchors(self):
        records = {'0001-first.md': record('0001'), '0002-second.md': record('0002')}
        found = errors(records, toc='# T\n\n[a](adr/0001-first.md#gone)\n[c](adr/0003-third.md)\n')
        self.assertTrue(any('does not link docs/adr/0002-second.md' in error for error in found), found)
        self.assertTrue(any('0001-first.md#gone, which no heading' in error for error in found), found)
        self.assertTrue(any('links adr/0003-third.md, which is no record' in error for error in found), found)


if __name__ == '__main__':
    unittest.main()
