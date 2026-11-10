"""The Herdr-name ratchet must fail a new name and a growing count, and must read git's index.

The ratchet keeps the names Hide keeps on purpose as the only ones, and the
count only goes down. A check that counts the folder name as Herdr, hides a
name next to it, or opens a working-tree file (a FIFO hangs it, a link points it
outside the checkout) protects nothing; these tests hold each end. Temporary git
repositories are used for the index tests, never the real checkout.
"""
import importlib.util
import json
import os
import subprocess
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
    """Results as `scan_index` gives them, for texts already in memory."""
    return {path: CHECK.names_in(text) for path, text in texts.items()}


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


class TheFolderNameIsExemptOnlyAsAWholeComponent(unittest.TestCase):
    def test_the_folder_name_as_a_whole_path_component_is_not_counted(self):
        for text in [
            'cd ~/.hide/worktrees/herdr-ide/feature-x',
            'HERDR-IDE/x',
            'herdr-ide/x',
            '/r/herdr-ide',
            '~/projects/herdr-ide',
            'the herdr-ide folder',
        ]:
            with self.subTest(text=text):
                self.assertEqual(count(text), 0)

    def test_a_name_touching_the_folder_name_is_counted(self):
        for text in [
            'herdr-ide.foo',
            '/tmp/herdr-ide.foo/x',
            'herdr-ide+foo',
            'herdr-ide-foo',
            'herdr-identity',
            'herdr_ide',
            'x-herdr-ide',
            'herdr-ide2',
        ]:
            with self.subTest(text=text):
                self.assertEqual(count(text), 1)

    def test_herdr_ide_dot_git_is_counted_as_a_name_with_a_suffix(self):
        # A clone named `herdr-ide.git` is a suffixed name, not the folder, so it counts.
        self.assertEqual(count('git@host:herdr-ide.git'), 1)

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


class ReadingComesFromTheIndex(unittest.TestCase):
    """Every test builds a temporary git repository; the working tree is changed on purpose."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        (self.root / 'kept.txt').write_text('herdr\n', encoding='utf-8')
        subprocess.run(['git', 'add', 'kept.txt'], cwd=self.root, check=True)

    def tearDown(self):
        self.tmp.cleanup()

    def results(self):
        results, problems = CHECK.scan_index(self.root)
        return results, problems

    def test_a_tracked_file_replaced_by_a_fifo_or_a_link_gives_the_same_result(self):
        before, problems = self.results()
        self.assertEqual(problems, [])
        self.assertEqual(before['kept.txt'][0], 1)
        # A FIFO with no writer would hang any reader; a link would send a reader to an
        # untracked file under agents/. Neither is opened, so both change nothing.
        (self.root / 'agents').mkdir()
        (self.root / 'agents' / 'local.txt').write_text('herdr local only\n', encoding='utf-8')
        os.remove(self.root / 'kept.txt')
        os.mkfifo(self.root / 'kept.txt')
        after_fifo, problems = self.results()
        self.assertEqual(problems, [])
        self.assertEqual(after_fifo, before)
        os.remove(self.root / 'kept.txt')
        os.symlink(self.root / 'agents' / 'local.txt', self.root / 'kept.txt')
        after_link, problems = self.results()
        self.assertEqual(problems, [])
        self.assertEqual(after_link, before)

    def test_a_tracked_path_under_the_local_agents_folder_is_reported(self):
        (self.root / 'agents').mkdir()
        (self.root / 'agents' / 'note.txt').write_text('herdr\n', encoding='utf-8')
        subprocess.run(['git', 'add', '-f', 'agents/note.txt'], cwd=self.root, check=True)
        results, problems = self.results()
        self.assertNotIn('agents/note.txt', results)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn('agents/note.txt', problems[0])
        self.assertIn('untrack it', problems[0])

    def test_a_binary_blob_is_skipped_by_git(self):
        (self.root / 'bin.dat').write_bytes(b'herdr\0herdr')
        subprocess.run(['git', 'add', 'bin.dat'], cwd=self.root, check=True)
        results, _ = self.results()
        self.assertEqual(results['bin.dat'][0], 0)

    def test_a_staged_change_is_what_is_counted(self):
        (self.root / 'kept.txt').write_text('herdr herdr herdr\n', encoding='utf-8')
        subprocess.run(['git', 'add', 'kept.txt'], cwd=self.root, check=True)
        (self.root / 'kept.txt').write_text('nothing\n', encoding='utf-8')
        results, _ = self.results()
        self.assertEqual(results['kept.txt'][0], 3)

    def test_a_git_failure_is_reported_with_gits_message(self):
        with tempfile.TemporaryDirectory() as bare:
            results, problems = CHECK.scan_index(Path(bare))
        self.assertEqual(results, {})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn('git ls-files failed', problems[0])
        self.assertIn('not a git repository', problems[0].lower())


class TheTreeAsItIsPasses(unittest.TestCase):
    def test_every_listed_reason_is_defined_and_no_file_is_off_the_list(self):
        results, problems = CHECK.scan_index(ROOT)
        errors = problems + CHECK.check(results, ALLOWLIST)
        self.assertEqual(errors, [], '\n'.join(errors))

    def test_the_allowlist_gives_a_reason_to_every_entry(self):
        for path, entry in ALLOWLIST['files'].items():
            with self.subTest(path=path):
                self.assertIn(entry['reason'], ALLOWLIST['reasons'])
                self.assertGreater(entry['max'], 0)


if __name__ == '__main__':
    unittest.main()
