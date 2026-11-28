"""The guide splitter writes a table of contents and pieces that the docs
check accepts, with the moved text unchanged, and refuses every table or
source it cannot place exactly.

Each test builds a small git checkout in a temporary folder (a guide, the
Herdr names check and its allowlist, one code path) and splits it there.
"""
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


SPLIT = load('split_doc', 'split-doc.py')
CHECK = load('check_docs', 'check-docs.py')

GUIDE = '''# A guide

Read this first; see [the build](BUILD.md) and [the rules](#rules).

## Rules

Each rule holds. See [the tools](#tools) and [an outside page](https://example.com/a#b).

### Rule one

Herdr once did this; [back to the guide](GUIDE.md).

```text
## not a heading
```

## Tools

Use `src/` and [rule one](GUIDE.md#rule-one).
'''

TABLE = {
    'source': 'docs/GUIDE.md',
    'folder': 'docs/guide',
    'pieces': [
        {'name': 'rules', 'title': 'The rules', 'code': ['src/'], 'sections': ['Rules']},
        {'name': 'tools', 'title': 'The tools', 'code': ['src/'], 'sections': ['Tools']},
    ],
}

ALLOWLIST = '{\n  "reasons": {\n    "to-rewrite": "a sentence"\n  },\n  "files": {\n    "docs/GUIDE.md": {"max": 1, "reason": "to-rewrite"},\n    "docs/OTHER.md": {"max": 1, "reason": "to-rewrite"}\n  }\n}\n'


class SplitDocTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / 'scripts').mkdir()
        (self.root / 'src').mkdir()
        (self.root / 'docs').mkdir()
        shutil.copy(SCRIPTS / 'check-herdr-names.py', self.root / 'scripts' / 'check-herdr-names.py')
        (self.root / 'scripts' / 'herdr-names-allowlist.json').write_text(ALLOWLIST)
        self.commit(GUIDE)

    def commit(self, text):
        (self.root / 'docs' / 'GUIDE.md').write_text(text)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        subprocess.run(['git', 'add', '-A'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'guide'],
                       cwd=self.root, check=True)

    def split(self, table=TABLE):
        return SPLIT.split(self.root, 'HEAD', table)

    def write(self, outputs):
        for name, text in outputs.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)

    def assertRefused(self, words, table=TABLE):
        with self.assertRaises(SPLIT.Refused) as refused:
            self.split(table)
        self.assertIn(words, str(refused.exception))

    def test_the_split_moves_the_text_and_the_check_accepts_it(self):
        outputs, _, report = self.split()
        rules = outputs['docs/guide/rules.md']
        self.assertTrue(rules.startswith('# The rules\n\n- Code: `src/`\n- Index: [GUIDE.md](../GUIDE.md)\n\n## Rules\n'))
        self.assertIn('See [the tools](tools.md#tools) and [an outside page](https://example.com/a#b).', rules)
        self.assertIn('Herdr once did this; [back to the guide](../GUIDE.md).', rules)
        self.assertIn('```text\n## not a heading\n```', rules)
        self.assertIn('Use `src/` and [rule one](rules.md#rule-one).', outputs['docs/guide/tools.md'])

        toc = outputs['docs/GUIDE.md']
        self.assertTrue(toc.startswith('# A guide\n\nRead this first; see [the build](BUILD.md) and [the rules](#rules).\n'))
        self.assertIn('| [The rules](guide/rules.md) | `src/` |', toc)
        self.assertIn('### Rule one\n\nMoved to [The rules](guide/rules.md#rule-one).', toc)

        allowlist = json.loads(outputs['scripts/herdr-names-allowlist.json'])['files']
        self.assertEqual(allowlist, {'docs/OTHER.md': {'max': 1, 'reason': 'to-rewrite'},
                                     'docs/guide/rules.md': {'max': 1, 'reason': 'to-rewrite'}})
        self.assertIn('Herdr names 1 in docs/GUIDE.md -> 1 across the outputs', '\n'.join(report))

        self.write(outputs)
        saved = CHECK.EXCEPTIONS, CHECK.SPLITS
        CHECK.EXCEPTIONS, CHECK.SPLITS = set(), {'docs/GUIDE.md': 'docs/guide'}
        try:
            self.assertEqual(CHECK.check(self.root), [])
        finally:
            CHECK.EXCEPTIONS, CHECK.SPLITS = saved

    def test_a_rerun_on_the_same_source_writes_the_same_files_and_a_split_source_is_refused(self):
        first = self.split()[0]
        self.assertEqual(self.split()[0], first)
        self.write(first)
        subprocess.run(['git', 'add', '-A'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'split'],
                       cwd=self.root, check=True)
        self.assertEqual(SPLIT.split(self.root, 'HEAD~1', TABLE)[0], first, 'a rerun from the unsplit source after the split')
        self.assertRefused('already a table of contents')

    def test_an_unplaced_heading_or_a_section_the_source_lacks_is_refused(self):
        self.commit(GUIDE + '\n## Extra\n\nMore.\n')
        self.assertRefused('heading "Extra" (line')
        self.commit(GUIDE)
        table = dict(TABLE, pieces=TABLE['pieces'] + [{'name': 'gone', 'title': 'Gone', 'code': ['src/'], 'sections': ['Gone']}])
        self.assertRefused("sections the source no longer has: ['Gone']", table)

    def test_a_section_placed_twice_or_a_missing_code_path_is_refused(self):
        twice = dict(TABLE, pieces=[TABLE['pieces'][0], dict(TABLE['pieces'][1], sections=['Tools', 'Rules'])])
        self.assertRefused('"Rules" is placed twice', twice)
        gone = dict(TABLE, pieces=[dict(TABLE['pieces'][0], code=['src/gone.rs']), TABLE['pieces'][1]])
        self.assertRefused('code path src/gone.rs does not exist', gone)

    def test_a_link_to_a_missing_anchor_or_a_duplicate_anchor_is_refused(self):
        self.commit(GUIDE + '\nSee [nothing](#nowhere).\n')
        self.assertRefused('a link to #nowhere answers no heading')
        self.commit(GUIDE.replace('## Tools\n', '## Tools\n\n### Rule one\n'))
        self.assertRefused('share the anchor #rule-one')

    def test_inline_code_and_nested_fences_are_left_as_they_are(self):
        self.commit(GUIDE.replace('Use `src/`', 'Use `[x](#rules)` and `src/`')
                    .replace('```text\n## not a heading\n```', '````md\n```\n## not a heading [y](#rules)\n```\n````'))
        outputs, _, _ = self.split()
        self.assertIn('Use `[x](#rules)` and `src/`', outputs['docs/guide/tools.md'])
        self.assertIn('## not a heading [y](#rules)', outputs['docs/guide/rules.md'])

    def test_a_link_form_the_split_cannot_retarget_is_refused(self):
        for line, form in (('[t](#rules "title")', 'a link with a title'), ('[a](<BUILD.md>)', 'an angle-bracket'),
                           ('[r]: BUILD.md', 'a reference definition'), ('<a href="BUILD.md">b</a>', 'an HTML link'),
                           ('[abs](/docs/BUILD.md)', 'a root-relative link')):
            with self.subTest(form=form):
                self.commit(GUIDE + '\n' + line + '\n')
                self.assertRefused(form)

    def test_prose_that_mentions_src_is_not_an_html_link(self):
        self.commit(GUIDE + '\nSet src= in the config.\n')
        self.assertIn('Set src= in the config.', self.split()[0]['docs/guide/tools.md'])

    def test_an_unclosed_fence_or_an_unlisted_herdr_name_is_refused(self):
        self.commit(GUIDE + '\n```\nopen\n')
        self.assertRefused('an unclosed code fence')
        (self.root / 'scripts' / 'herdr-names-allowlist.json').write_text(ALLOWLIST.replace('    "docs/GUIDE.md": {"max": 1, "reason": "to-rewrite"},\n', ''))
        self.commit(GUIDE)
        self.assertRefused('docs/guide/rules.md would name Herdr 1 times')

    def test_the_command_replaces_only_this_sources_old_pieces(self):
        table = self.root / 'table.json'
        table.write_text(json.dumps(TABLE))
        old = self.root / 'docs' / 'guide' / 'old.md'
        old.parent.mkdir()
        old.write_text('# Old\n\n- Code: `src/`\n- Index: [GUIDE.md](../GUIDE.md)\n')
        run = [sys.executable, str(SCRIPTS / 'split-doc.py'), '--source-ref', 'HEAD', '--table', str(table), '--checkout', str(self.root)]
        done = subprocess.run(run, capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(old.exists())
        self.assertEqual(sorted(p.name for p in old.parent.iterdir()), ['rules.md', 'tools.md'])

        other = self.root / 'docs' / 'guide' / 'notes.md'
        other.write_text('# Notes\n')
        subprocess.run(['git', 'checkout', '-q', '--', 'docs/GUIDE.md'], cwd=self.root, check=True)
        done = subprocess.run(run, capture_output=True, text=True)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("not pieces of GUIDE.md: ['docs/guide/notes.md']", done.stderr)
        self.assertTrue(other.exists())

    def test_a_piece_title_that_repeats_its_section_heading_is_refused(self):
        same = dict(TABLE, pieces=[dict(TABLE['pieces'][0], title='Rules'), TABLE['pieces'][1]])
        self.assertRefused('docs/guide/rules.md would hold a duplicate anchor', same)


if __name__ == '__main__':
    unittest.main()
