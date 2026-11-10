"""The Herdr-name ratchet must fail a new name and a growing count, and must not count the folder name.

The ratchet exists so that the names Hide keeps on purpose (the import, credit,
old values read from disk, fixtures, test messages) stay the only ones, and
so that the count only goes down. A check that passes on a new name, or that
counts `herdr-ide` as Herdr, protects nothing; these tests hold both ends.
"""
import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = importlib.util.spec_from_file_location(
    'check_herdr_names', ROOT / 'scripts' / 'check-herdr-names.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)

ALLOWLIST = json.loads((ROOT / 'scripts' / 'herdr-names-allowlist.json').read_text(encoding='utf-8'))


def allowlist(files):
    return {'reasons': {'import': 'the read-only import'}, 'files': files}


class NamesOutsideTheListFail(unittest.TestCase):
    def test_a_new_name_in_an_unlisted_file_fails_with_its_file_and_line(self):
        errors = CHECK.check(
            {'hide-core/src/new.rs': 'fn a() {}\n// Herdr is back\n'}, allowlist({}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('hide-core/src/new.rs', errors[0])
        self.assertIn('lines 2', errors[0])
        self.assertIn('scripts/herdr-names-allowlist.json', errors[0])

    def test_a_name_in_any_case_counts(self):
        for text in ['HERDR_PANE_ID', 'herdr', 'Herdr', 'hErDr']:
            with self.subTest(text=text):
                errors = CHECK.check({'a.rs': text}, allowlist({}))
                self.assertEqual(len(errors), 1, text)


class TheListHoldsOnlyTheRatchet(unittest.TestCase):
    def test_a_listed_file_that_gains_a_name_fails(self):
        errors = CHECK.check(
            {'hide-core/src/import.rs': 'herdr\nherdr\nherdr\n'},
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('names Herdr 3 times, the list allows 2', errors[0])

    def test_a_listed_file_that_loses_a_name_must_lower_its_number(self):
        errors = CHECK.check(
            {'hide-core/src/import.rs': 'herdr\n'},
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('Lower its "max" to 1', errors[0])

    def test_a_listed_file_at_its_number_passes(self):
        errors = CHECK.check(
            {'hide-core/src/import.rs': 'herdr\nHerdr\n'},
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(errors, [])

    def test_an_entry_for_a_file_with_no_names_is_removed(self):
        errors = CHECK.check(
            {'hide-core/src/clean.rs': 'nothing here\n'},
            allowlist({'hide-core/src/clean.rs': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('remove its entry', errors[0])

    def test_an_entry_for_an_untracked_file_is_removed(self):
        errors = CHECK.check(
            {}, allowlist({'gone.rs': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('not tracked', errors[0])

    def test_an_entry_with_an_undefined_reason_fails(self):
        errors = CHECK.check(
            {'a.rs': 'herdr'}, allowlist({'a.rs': {'max': 1, 'reason': 'because'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('does not define', errors[0])


class TheFolderNameDoesNotCount(unittest.TestCase):
    def test_herdr_ide_alone_is_not_a_name(self):
        text = 'cd ~/.hide/worktrees/herdr-ide/feature-x\nHERDR-IDE/x\n'
        self.assertEqual(CHECK.count(text), 0)
        self.assertEqual(CHECK.check({'CLAUDE.md': text}, allowlist({})), [])

    def test_herdr_ide_beside_a_real_name_counts_only_the_real_name(self):
        self.assertEqual(CHECK.count('herdr-ide and herdr'), 1)

    def test_a_listed_file_with_only_the_folder_name_is_removed_not_kept(self):
        errors = CHECK.check(
            {'a.md': 'herdr-ide'}, allowlist({'a.md': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('remove its entry', errors[0])


class TheGuardFilesAreNotScanned(unittest.TestCase):
    def test_the_guard_names_the_word_without_being_listed(self):
        for path in CHECK.GUARD:
            with self.subTest(path=path):
                self.assertEqual(CHECK.check({path: 'herdr herdr'}, allowlist({})), [])


class TheTreeAsItIsPasses(unittest.TestCase):
    def test_every_listed_reason_is_defined_and_no_file_is_off_the_list(self):
        texts = CHECK.tracked_texts()
        errors = CHECK.check(texts, ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))

    def test_the_allowlist_gives_a_reason_to_every_entry(self):
        for path, entry in ALLOWLIST['files'].items():
            with self.subTest(path=path):
                self.assertIn(entry['reason'], ALLOWLIST['reasons'])
                self.assertGreater(entry['max'], 0)


if __name__ == '__main__':
    unittest.main()
