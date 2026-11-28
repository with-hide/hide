"""The retired-spelling check must catch an old `hide` spelling where Hide teaches it, and only there.

A renamed command keeps its old spelling for good, but nothing Hide writes may
teach it again. A check that misses a retired topic, a retired option or a
core text protects nothing, and one that flags the release notes, a test or a
comment blocks a rename that did everything right. Each test builds a small
checkout in a temporary folder, never the real one.
"""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SPEC = importlib.util.spec_from_file_location(
    'check_retired_spellings', ROOT / 'scripts' / 'check-retired-spellings.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)

CONTRACT = {
    'commands': [
        {'command': 'task set', 'options': [{'name': '--json'}]},
        {'command': 'task show', 'options': []},
        {'command': 'device request', 'options': []},
        {'command': 'letter send', 'options': [
            {'name': '--device'}, {'name': '--machine', 'retired_for': '--device'}]},
        {'command': 'plan set', 'retired_for': 'task set', 'options': [{'name': '--json'}]},
        {'command': 'plan show', 'retired_for': 'task show', 'options': []},
        {'command': 'request send', 'retired_for': 'letter send', 'options': [
            {'name': '--device'}, {'name': '--machine', 'retired_for': '--device'}]},
    ],
}


class Checkout:
    def __init__(self, files):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        (self.root / 'contracts').mkdir()
        (self.root / 'contracts/hide-cli.json').write_text(json.dumps(CONTRACT))
        for path, text in files.items():
            (self.root / path).parent.mkdir(parents=True, exist_ok=True)
            (self.root / path).write_text(text)

    def errors(self):
        return CHECK.check(self.root)

    def close(self):
        self.folder.cleanup()


def errors_of(files):
    checkout = Checkout(files)
    try:
        return checkout.errors()
    finally:
        checkout.close()


class RetiredSpellingsFail(unittest.TestCase):
    def test_a_retired_command_in_a_core_text_fails_with_its_file_line_and_new_spelling(self):
        errors = errors_of({'hide-core/src/plans.rs': 'fn f() {}\nlet next = "run hide plan show";\n'})
        self.assertEqual(errors, ['hide-core/src/plans.rs:2: hide plan show is retired; write hide task show'])

    def test_a_retired_topic_alone_fails(self):
        errors = errors_of({'docs/x.md': 'Use `hide plan` to keep your steps.\n'})
        self.assertEqual(errors, ['docs/x.md:1: hide plan is retired; write hide task'])

    def test_a_retired_option_fails_after_its_command(self):
        errors = errors_of({'site/src/content/a.mdx': 'hide letter send a --body hi --machine m1\n'})
        self.assertEqual(errors, ['site/src/content/a.mdx:1: hide letter send --machine is retired; write hide letter send --device'])

    def test_a_retired_command_of_a_topic_that_stays_fails(self):
        errors = errors_of({'web/src/i18n/a.ts': 'const s = "hide request send a";\n'})
        self.assertEqual(len(errors), 1)
        self.assertIn('hide request send is retired; write hide letter send', errors[0])


class WhatIsLeftOut(unittest.TestCase):
    def test_history_tests_fixtures_and_comments_pass(self):
        files = {
            'web/src/releases.json': '{"en": "hide plan is now hide task"}\n',
            'hide-core/src/a.rs': '// hide plan was the old name\nfn f() {}\n#[cfg(test)]\nmod tests { const S: &str = "hide plan show"; }\n',
            'hide-core/src/b/tests.rs': 'const S: &str = "hide plan show";\n',
            'web/src/a.test.ts': 'const s = "hide plan show";\n',
            'hided/src/fixtures/session.json': '{"command": "hide plan set"}\n',
        }
        self.assertEqual(errors_of(files), [])

    def test_the_same_word_elsewhere_passes(self):
        files = {
            'docs/a.md': '`hide device request k1`, a letter of kind request, the hide-plan skill, hide planning.\n',
            'docs/b.md': 'hide task show, then hide letter send a --device m1 --body "--machine".\n',
        }
        self.assertEqual(errors_of(files), [])


class TheRealCheckout(unittest.TestCase):
    def test_the_checkout_teaches_no_retired_spelling(self):
        self.assertEqual(CHECK.check(ROOT), [])


if __name__ == '__main__':
    unittest.main()
