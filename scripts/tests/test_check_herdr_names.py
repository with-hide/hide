"""The Herdr-name ratchet must fail a new name and a growing count, and must not miscount.

The ratchet exists so that the names Hide keeps on purpose stay the only ones,
and so that the count only goes down. A check that passes a new name, counts
the folder name as Herdr, hides a name next to the folder name, or follows a
link out of the checkout protects nothing; these tests hold each end.
"""
import importlib.util
import json
import os
import tempfile
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


def counted(texts):
    """Results as `collect` gives them, for texts already in memory."""
    return {path: ('count', CHECK.names_in(text)) for path, text in texts.items()}


def count(text):
    return CHECK.names_in(text)[0]


class NamesOutsideTheListFail(unittest.TestCase):
    def test_a_new_name_in_an_unlisted_file_fails_with_its_file_and_line(self):
        errors = CHECK.check(
            counted({'hide-core/src/new.rs': 'fn a() {}\n// Herdr is back\n'}), allowlist({}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('hide-core/src/new.rs', errors[0])
        self.assertIn('lines 2', errors[0])
        self.assertIn('scripts/herdr-names-allowlist.json', errors[0])

    def test_a_name_in_any_case_counts(self):
        for text in ['HERDR_PANE_ID', 'herdr', 'Herdr', 'hErDr']:
            with self.subTest(text=text):
                self.assertEqual(len(CHECK.check(counted({'a.rs': text}), allowlist({}))), 1, text)


class TheListHoldsOnlyTheRatchet(unittest.TestCase):
    def test_a_listed_file_that_gains_a_name_fails(self):
        errors = CHECK.check(
            counted({'hide-core/src/import.rs': 'herdr\nherdr\nherdr\n'}),
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('names Herdr 3 times, the list allows 2', errors[0])

    def test_a_listed_file_that_loses_a_name_must_lower_its_number(self):
        errors = CHECK.check(
            counted({'hide-core/src/import.rs': 'herdr\n'}),
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('Lower its "max" to 1', errors[0])

    def test_a_listed_file_at_its_number_passes(self):
        errors = CHECK.check(
            counted({'hide-core/src/import.rs': 'herdr\nHerdr\n'}),
            allowlist({'hide-core/src/import.rs': {'max': 2, 'reason': 'import'}}))
        self.assertEqual(errors, [])

    def test_an_entry_for_a_file_with_no_names_is_removed(self):
        errors = CHECK.check(
            counted({'hide-core/src/clean.rs': 'nothing here\n'}),
            allowlist({'hide-core/src/clean.rs': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('remove its entry', errors[0])

    def test_an_entry_for_an_untracked_file_is_removed(self):
        errors = CHECK.check({}, allowlist({'gone.rs': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('not a tracked text file', errors[0])

    def test_an_entry_with_an_undefined_reason_fails(self):
        errors = CHECK.check(
            counted({'a.rs': 'herdr'}), allowlist({'a.rs': {'max': 1, 'reason': 'because'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('does not define', errors[0])


class TheFolderNameIsRemovedOnlyWhenWhole(unittest.TestCase):
    def test_the_folder_name_as_a_path_component_is_not_counted(self):
        for text in [
            'cd ~/.hide/worktrees/herdr-ide/feature-x',
            'HERDR-IDE/x',
            'herdr-ide/',
            '~/projects/herdr-ide',
            'the herdr-ide folder',
        ]:
            with self.subTest(text=text):
                self.assertEqual(count(text), 0)

    def test_a_name_attached_to_the_folder_name_is_counted(self):
        for text in ['herdr-ide-foo', 'herdr-identity', 'herdr_ide', 'x-herdr-ide', 'herdr-ide2']:
            with self.subTest(text=text):
                self.assertEqual(count(text), 1)

    def test_the_folder_name_beside_a_real_name_counts_only_the_real_name(self):
        self.assertEqual(count('herdr-ide and herdr'), 1)

    def test_a_listed_file_with_only_the_folder_name_is_removed_not_kept(self):
        errors = CHECK.check(
            counted({'a.md': 'herdr-ide'}), allowlist({'a.md': {'max': 1, 'reason': 'import'}}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('remove its entry', errors[0])


class TheGuardFilesAreNotScanned(unittest.TestCase):
    def test_the_guard_names_the_word_without_being_listed(self):
        for path in CHECK.GUARD:
            with self.subTest(path=path):
                self.assertEqual(CHECK.check(counted({path: 'herdr herdr'}), allowlist({})), [])


class ReadingStaysInsideTheCheckout(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / 'agents').mkdir()
        (self.root / 'agents' / 'local.txt').write_text('herdr local only\n', encoding='utf-8')
        (self.root / 'kept.txt').write_text('herdr\n', encoding='utf-8')

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_tracked_symlink_is_not_followed_into_the_untracked_agents_folder(self):
        os.symlink(self.root / 'agents' / 'local.txt', self.root / 'link.txt')
        results = CHECK.collect(self.root, [('120000', 'link.txt'), ('100644', 'kept.txt')])
        self.assertNotIn('link.txt', results)
        self.assertEqual(results['kept.txt'][1][0], 1)
        self.assertEqual(CHECK.check(results, allowlist({'kept.txt': {'max': 1, 'reason': 'import'}})), [])

    def test_a_symlink_to_a_file_outside_the_checkout_is_not_read(self):
        outside = Path(self.tmp.name + '-outside.txt')
        outside.write_text('herdr\n', encoding='utf-8')
        try:
            os.symlink(outside, self.root / 'escape.txt')
            results = CHECK.collect(self.root, [('120000', 'escape.txt')])
            self.assertEqual(results, {})
        finally:
            outside.unlink()

    def test_a_submodule_entry_is_not_read(self):
        self.assertEqual(CHECK.collect(self.root, [('160000', 'kept.txt')]), {})

    def test_a_tracked_file_that_cannot_be_read_is_a_reported_failure(self):
        results = CHECK.collect(self.root, [('100644', 'missing.txt')])
        self.assertEqual(results['missing.txt'][0], 'error')
        errors = CHECK.check(results, allowlist({}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('missing.txt: cannot be read', errors[0])

    def test_a_directory_in_the_index_is_a_reported_failure_not_a_traceback(self):
        (self.root / 'dir').mkdir()
        results = CHECK.collect(self.root, [('100644', 'dir')])
        errors = CHECK.check(results, allowlist({}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('dir: ', errors[0])


class LargeFilesAreBoundedAndBinaryOnesSkipped(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_file_over_the_cap_without_a_nul_byte_fails_with_a_clear_message(self):
        (self.root / 'big.txt').write_text('herdr ' * 100, encoding='utf-8')
        results = CHECK.collect(self.root, [('100644', 'big.txt')], cap=64)
        errors = CHECK.check(results, allowlist({}))
        self.assertEqual(len(errors), 1, errors)
        self.assertIn('over the 64-byte cap', errors[0])
        self.assertIn('big.txt', errors[0])

    def test_a_file_over_the_cap_with_a_nul_byte_is_binary_and_skipped(self):
        (self.root / 'big.bin').write_bytes(b'herdr' * 50 + b'\0' + b'x' * 300)
        results = CHECK.collect(self.root, [('100644', 'big.bin')], cap=64)
        self.assertEqual(results, {})

    def test_a_small_file_with_a_nul_byte_is_binary_and_skipped(self):
        (self.root / 'small.bin').write_bytes(b'herdr\0herdr')
        self.assertEqual(CHECK.collect(self.root, [('100644', 'small.bin')]), {})

    def test_a_file_at_the_cap_is_counted(self):
        (self.root / 'edge.txt').write_text('herdr', encoding='utf-8')
        results = CHECK.collect(self.root, [('100644', 'edge.txt')], cap=5)
        self.assertEqual(results['edge.txt'][1][0], 1)


class TheTreeAsItIsPasses(unittest.TestCase):
    def test_every_listed_reason_is_defined_and_no_file_is_off_the_list(self):
        errors = CHECK.check(CHECK.collect(ROOT, CHECK.tracked_entries(ROOT)), ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))

    def test_the_allowlist_gives_a_reason_to_every_entry(self):
        for path, entry in ALLOWLIST['files'].items():
            with self.subTest(path=path):
                self.assertIn(entry['reason'], ALLOWLIST['reasons'])
                self.assertGreater(entry['max'], 0)


if __name__ == '__main__':
    unittest.main()
