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
    'retired_words': [
        {'old': ['plan'], 'now': ['task']},
        {'old': ['request'], 'now': ['letter']},
        {'old': ['workspace', 'info'], 'now': ['checkout', 'show']},
        {'old': ['workspace', 'info', '--hook'], 'now': ['workspace', 'info', '--hook']},
    ],
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
        self.assertEqual(errors, ['hide-core/src/plans.rs:2: hide plan is retired; write hide task'])

    def test_a_retired_topic_alone_fails(self):
        errors = errors_of({'docs/x.md': 'Use `hide plan` to keep your steps, or hide workspace info.\n'})
        self.assertEqual(errors, [
            'docs/x.md:1: hide plan is retired; write hide task',
            'docs/x.md:1: hide workspace info is retired; write hide checkout show',
        ])

    def test_a_retired_option_fails_after_its_command(self):
        errors = errors_of({'site/src/content/a.mdx': 'hide letter send a --body hi --machine m1\n'})
        self.assertEqual(errors, ['site/src/content/a.mdx:1: hide letter send --machine is retired; write hide letter send --device'])

    def test_a_retired_command_of_a_topic_that_stays_fails(self):
        errors = errors_of({'web/src/i18n/a.ts': 'const s = "hide request send a";\n'})
        self.assertEqual(errors, ['web/src/i18n/a.ts:1: hide request is retired; write hide letter'])

    def test_a_retired_option_under_its_old_command_fails_twice(self):
        errors = errors_of({'README.md': 'Run hide request send a --machine m1 there.\n'})
        self.assertEqual(errors, [
            'README.md:1: hide request is retired; write hide letter',
            'README.md:1: hide letter send --machine is retired; write hide letter send --device',
        ])

    def test_every_tracked_text_file_is_read(self):
        files = {name: 'hide plan show\n' for name in (
            'README.md', 'AGENTS.md', '.claude/skills/x/SKILL.md', 'desktop/src/main/a.ts',
            'hide-session/src/a.rs', 'scripts/agent_journey/run.py', 'scripts/a.mjs')}
        self.assertEqual(len(errors_of(files)), len(files))


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

    def test_a_test_only_item_does_not_hide_the_code_after_it(self):
        errors = errors_of({'hide-core/src/mailbox.rs': (
            '#[cfg(test)]\npub(crate) fn tool_pull() {}\n'
            'const NEXT: &str = "hide plan show";\n'
            '#[cfg(test)]\n#[allow(dead_code)]\npub mod tests {\n    const S: &str = "hide plan set";\n}\n'
        )})
        self.assertEqual(errors, ['hide-core/src/mailbox.rs:3: hide plan is retired; write hide task'])

    def test_a_kept_region_leaves_out_its_lines_only(self):
        errors = errors_of({'hide-kit/src/agents.rs': (
            'pub const CURRENT: &str = "run hide workspace info";\n'
            '// retired-spellings: kept from here: an earlier build\'s stub, byte for byte\n'
            'pub const RETIRED: &str = "run hide workspace info";\n'
            '// retired-spellings: kept to here\n'
            'pub const AFTER: &str = "run hide plan show";\n'
        )})
        self.assertEqual(errors, [
            'hide-kit/src/agents.rs:1: hide workspace info is retired; write hide checkout show',
            'hide-kit/src/agents.rs:5: hide plan is retired; write hide task',
        ])

    def test_a_kept_region_with_no_reason_or_no_end_fails(self):
        errors = errors_of({
            'docs/a.md': '<!-- retired-spellings: kept from here -->\nhide plan\n<!-- retired-spellings: kept to here -->\n',
            'docs/b.md': '<!-- retired-spellings: kept from here: history -->\nhide plan\n',
            # Prose that names the marker opens nothing.
            'docs/c.md': 'Wrap it in `retired-spellings: kept from here: <why>`, then hide plan.\n',
        })
        self.assertEqual(errors, [
            'docs/a.md:1: a kept region says why after "retired-spellings: kept from here:"',
            'docs/b.md:1: a kept region has no "retired-spellings: kept to here"',
            'docs/c.md:1: hide plan is retired; write hide task',
        ])

    def test_the_same_word_elsewhere_passes(self):
        files = {
            'docs/a.md': '`hide device request k1`, a letter of kind request, the hide-plan skill, hide planning.\n',
            'docs/b.md': 'hide task show, then hide letter send a --device m1 --body "--machine".\n',
            # A kept row and a command no row names pass.
            'docs/c.md': 'hide workspace info --hook, hide workspace bootstrap, hide workspace purpose x\n',
            # An option of the next command on the line is that command's.
            'docs/d.md': 'hide letter send a, then hide agent spawn --machine m1\n',
        }
        self.assertEqual(errors_of(files), [])


class TheRealCheckout(unittest.TestCase):
    def test_the_checkout_teaches_no_retired_spelling(self):
        self.assertEqual(CHECK.check(ROOT), [])


if __name__ == '__main__':
    unittest.main()
