"""The guide splitter writes a table of contents and pieces that the docs
check accepts, with the moved text unchanged, and refuses every table or
source it cannot place exactly.

Each test builds a small git checkout in a temporary folder (a guide, the
Herdr names check and its allowlist, one code path) and splits it there; the
merge test splits two guides on two branches of one and merges them.
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
GIT = ['git', '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false']


class SplitDocTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / 'scripts').mkdir()
        (self.root / 'src').mkdir()
        (self.root / 'docs').mkdir()
        shutil.copy(SCRIPTS.parent / SPLIT.NAMES_CHECK, self.root / SPLIT.NAMES_CHECK)
        (self.root / SPLIT.ALLOWLIST).write_text(ALLOWLIST)
        self.commit(GUIDE)

    def commit(self, text):
        (self.root / 'docs' / 'GUIDE.md').write_text(text)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        subprocess.run(['git', 'add', '-A'], cwd=self.root, check=True)
        subprocess.run(GIT + ['commit', '-qm', 'guide'], cwd=self.root, check=True)

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

        self.assertNotIn(SPLIT.ALLOWLIST, outputs, 'the entry of docs/GUIDE.md counts its pieces')
        self.assertIn('Herdr names 1 in docs/GUIDE.md -> 1 across the outputs and the table', '\n'.join(report))

        self.write(outputs)
        self.write({'scripts/split-tables/guide.json': json.dumps(TABLE)})
        saved = CHECK.EXCEPTIONS
        CHECK.EXCEPTIONS = set()
        try:
            self.assertEqual([e for e in CHECK.check(self.root) if not e.startswith('docs/adr/')
                              and not e.startswith('docs/ARCHITECTURE.md')], [])
        finally:
            CHECK.EXCEPTIONS = saved

    def test_a_split_that_repeats_a_retired_name_raises_only_its_own_entry(self):
        table = dict(TABLE, pieces=[TABLE['pieces'][0], dict(TABLE['pieces'][1], title='Tools after Herdr')])
        outputs, _, report = self.split(table)
        before = ALLOWLIST.split('\n')
        after = outputs[SPLIT.ALLOWLIST].split('\n')
        changed = [(a, b) for a, b in zip(before, after) if a != b]
        self.assertEqual(changed, [('    "docs/GUIDE.md": {"max": 1, "reason": "to-rewrite"},',
                                    '    "docs/GUIDE.md": {"max": 5, "reason": "to-rewrite"},')],
                         'the title is in the table, the piece, its row and the Moved to line of its section')
        self.assertIn(f'{SPLIT.ALLOWLIST} counts 5 for it', '\n'.join(report))

    def test_the_split_removes_only_its_own_exceptions_line(self):
        check_docs = "EXCEPTIONS = {\n    'docs/A.md',\n\n    'docs/GUIDE.md',\n\n    'docs/Z.md',\n}\n"
        self.write({'scripts/check-docs.py': check_docs})
        outputs, _, report = self.split()
        self.assertEqual(outputs['scripts/check-docs.py'], "EXCEPTIONS = {\n    'docs/A.md',\n\n\n    'docs/Z.md',\n}\n")
        self.assertIn('removed docs/GUIDE.md from EXCEPTIONS', '\n'.join(report))
        self.write(outputs)
        self.assertNotIn('scripts/check-docs.py', self.split()[0], 'a rerun has nothing more to remove')

    def test_a_rerun_on_the_same_source_writes_the_same_files_and_a_split_source_is_refused(self):
        first = self.split()[0]
        self.assertEqual(self.split()[0], first)
        self.write(first)
        subprocess.run(['git', 'add', '-A'], cwd=self.root, check=True)
        subprocess.run(GIT + ['commit', '-qm', 'split'], cwd=self.root, check=True)
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

    def test_an_unclosed_fence_or_an_unlisted_retired_name_is_refused(self):
        self.commit(GUIDE + '\n```\nopen\n')
        self.assertRefused('an unclosed code fence')
        (self.root / SPLIT.ALLOWLIST).write_text(ALLOWLIST.replace('    "docs/GUIDE.md": {"max": 1, "reason": "to-rewrite"},\n', ''))
        self.commit(GUIDE)
        self.assertRefused('the split would name Herdr 1 times, and docs/GUIDE.md has no allowlist entry')

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


class TwoSplitsMerge(unittest.TestCase):
    """Two splits of neighbouring documents, made on one main with the real scripts, merge
    with no conflict, and the merged checkout passes the docs and retired-name checks."""

    DOCS = ('docs/ALPHA.md', 'docs/BETA.md')

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / 'scripts').mkdir()
        for name in (SPLIT.CHECK_DOCS, SPLIT.NAMES_CHECK, 'scripts/split-doc.py'):
            shutil.copy(SCRIPTS.parent / name, self.root / name)
        check_docs = self.root / 'scripts' / 'check-docs.py'
        text = check_docs.read_text()
        start = text.index('EXCEPTIONS = {\n')
        end = text.index('\n}\n', start) + 3
        # The real layout, with two neighbouring entries.
        check_docs.write_text(text[:start] + 'EXCEPTIONS = {\n' + '\n'.join(f"    '{doc}',\n" for doc in self.DOCS) + '}\n' + text[end:])
        (self.root / '.gitignore').write_text('__pycache__/\n')
        (self.root / 'src').mkdir()
        (self.root / 'src' / 'a.rs').write_text('')
        (self.root / 'docs' / 'adr').mkdir(parents=True)
        (self.root / 'docs' / 'adr' / '0001-first.md').write_text('# 0001. First\n\n- Status: Accepted\n- Date: 2026-11-29\n- Code: `src/`\n\n## Decision\n\nText.\n')
        (self.root / 'docs' / 'ARCHITECTURE.md').write_text('# Architecture\n\n[First](adr/0001-first.md)\n')
        filler = ('A sentence that keeps the guide over the cap. ' * 20 + '\n\n') * 40
        for doc in self.DOCS:
            # A heading with the retired name makes each split repeat it and raise its own allowlist line.
            (self.root / doc).write_text(f'# {doc}\n\nIntro.\n\n## One from Herdr\n\n{filler}\n## Two\n\n{filler}')
        module = load('names_check_copy', Path(SPLIT.NAMES_CHECK).name)
        counts = {doc: module.names_in((self.root / doc).read_text())[0] for doc in self.DOCS}
        counts['scripts/split-doc.py'] = module.names_in((self.root / 'scripts' / 'split-doc.py').read_text())[0]
        entries = ',\n\n'.join(f'    "{name}": {{"max": {n}, "reason": "r"}}' for name, n in sorted(counts.items()))
        (self.root / SPLIT.ALLOWLIST).write_text(
            '{\n  "reasons": {"r": "a reason"},\n  "files": {\n' + entries + '\n  }\n}\n')
        self.git('init', '-q', '-b', 'main')
        self.git('add', '-A')
        self.git('commit', '-qm', 'main')

    def git(self, *args):
        return subprocess.run(GIT + list(args), cwd=self.root, capture_output=True, text=True)

    def split_on_branch(self, doc):
        stem = Path(doc).stem.lower()
        table = {'source': doc, 'folder': f'docs/{stem}', 'pieces': [
            {'name': 'one', 'title': 'First part', 'code': ['src/a.rs'], 'sections': ['One from Herdr']},
            {'name': 'two', 'title': 'Second part', 'code': ['src/a.rs'], 'sections': ['Two']}]}
        self.git('checkout', '-q', '-b', stem, 'main')
        (self.root / 'scripts' / 'split-tables').mkdir(exist_ok=True)
        path = self.root / 'scripts' / 'split-tables' / f'{stem}.json'
        path.write_text(json.dumps(table, indent=2) + '\n')
        done = subprocess.run([sys.executable, str(self.root / 'scripts' / 'split-doc.py'), '--source-ref', 'main',
                               '--table', str(path), '--checkout', str(self.root)], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.git('add', '-A')
        self.git('commit', '-qm', f'split {doc}')
        return self.git('diff', '--name-only', 'main', stem).stdout.split()

    def test_two_splits_merge_without_a_conflict_and_the_result_passes_both_checks(self):
        alpha = self.split_on_branch('docs/ALPHA.md')
        beta = self.split_on_branch('docs/BETA.md')
        shared = {SPLIT.CHECK_DOCS, SPLIT.ALLOWLIST}
        self.assertTrue(shared <= set(alpha) and shared <= set(beta), f'both splits edit both lists: {alpha} {beta}')
        merged = self.git('merge', '-q', '--no-edit', 'alpha')
        self.assertEqual(merged.returncode, 0, merged.stdout + merged.stderr)
        self.git('add', '-A')
        for check in (SPLIT.CHECK_DOCS, SPLIT.NAMES_CHECK):
            done = subprocess.run([sys.executable, str(self.root / check)], capture_output=True, text=True)
            self.assertEqual(done.returncode, 0, check + ': ' + done.stderr)
        layout = load('test_check_docs', 'tests/test_check_docs.py').exceptions_layout
        self.assertEqual(layout((self.root / SPLIT.CHECK_DOCS).read_text()), ([], []), 'both entries gone, the layout kept')
        # A third split starting from either branch still finds a list it can merge with.
        self.assertEqual(layout(SPLIT.exceptions_update(self.root, 'docs/ALPHA.md') or
                                (self.root / SPLIT.CHECK_DOCS).read_text())[1], [])


if __name__ == '__main__':
    unittest.main()
