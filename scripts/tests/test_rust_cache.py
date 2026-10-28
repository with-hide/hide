"""Each Rust dependency set is saved by one job per system, on main only.

GitHub never replaces a cache entry, and a job whose key already exists saves
nothing, so two jobs saving one set race and the loser's build is never kept,
while a key per job kept the same dependencies four or five times over and
pushed main's own entries out of the 10 GB limit (2026-10-28). docs/BUILD.md,
"CI's Rust caches", owns the sets, the savers and the release argument.
"""
import contextlib
import importlib.util
import io
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

import tomllib

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/rust-cache'

SPEC = importlib.util.spec_from_file_location('prune', ROOT / 'scripts' / 'prune-rust-caches.py')
prune = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prune)

ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"
# What decides a dependency build, hashed into every key and by the prune.
DEPS = "hashFiles('Cargo.lock', 'Cargo.toml', '*/Cargo.toml', 'rust-toolchain.toml')"
# Each set's savers, one per system: (workflow, job id) and the exact `save`.
SAVERS = {
    'workspace': {
        ('pr.yml', 'rust'): ON_MAIN,
        ('pr.yml', 'windows-check'): ON_MAIN,
        ('desktop-e2e.yml', 'e2e'):
            "${{ github.ref == 'refs/heads/main' && matrix.shard == 1 && runner.os == 'macOS' }}",
    },
    'platform': {('os-contract.yml', 'contract'): ON_MAIN},
    'release': {('package.yml', 'package'): ON_MAIN, ('package.yml', 'macos-compile'): ON_MAIN},
}


def load(path):
    return yaml.safe_load(path.read_text())


def load_toml(path):
    with open(path, 'rb') as file:
        return tomllib.load(file)


def cache_steps():
    for path in sorted(WORKFLOWS.glob('*.yml')):
        for job_id, job in (load(path).get('jobs') or {}).items():
            for step in job.get('steps') or []:
                yield path.name, job_id, step


class RustCacheAction(unittest.TestCase):
    """The one action every job uses: what it keys, keeps and saves."""

    def setUp(self):
        self.steps = load(ROOT / '.github' / 'actions' / 'rust-cache' / 'action.yml')['runs']['steps']

    def test_keeps_only_the_pinned_toolchain_before_rust_cache_hashes_them(self):
        # rust-cache's environment hash stays on: it is what moves the key with
        # the pin; it is made stable by leaving the pinned toolchain alone.
        normalize, cache = self.steps[0], self.steps[1]
        self.assertIn('rustup show active-toolchain', normalize['run'])
        self.assertIn('rustup toolchain uninstall', normalize['run'])
        self.assertEqual(cache['uses'], 'Swatinem/rust-cache@v2')

    def test_keys_by_set_and_dependencies_keeps_dependencies_only_and_saves_as_told(self):
        self.assertEqual(self.steps[1]['with'], {
            'workspaces': '. -> target',
            'shared-key': f'${{{{ inputs.set }}}}-${{{{ {DEPS} }}}}',
            'cache-workspace-crates': False,
            'save-if': '${{ inputs.save }}',
        })

    def test_a_release_removes_every_workspace_crate_it_restored(self):
        clean = self.steps[2]
        self.assertEqual(clean['if'], "inputs.set == 'release'")
        self.assertIn('cargo clean --release --workspace', clean['run'])


class RustCacheSteps(unittest.TestCase):
    def test_every_job_takes_its_cache_from_the_one_action(self):
        direct = [f'{name} job {job}' for name, job, step in cache_steps()
                  if str(step.get('uses', '')).startswith('Swatinem/rust-cache')]
        self.assertEqual(direct, [], f'use {ACTION}, which keys by set rather than by job')

    def test_each_set_is_saved_only_by_its_savers_with_their_exact_condition(self):
        savers = {name: {} for name in SAVERS}
        for name, job, step in cache_steps():
            if step.get('uses') != ACTION:
                continue
            settings = step.get('with') or {}
            self.assertIn(settings.get('set'), SAVERS, f'{name} job {job}: unknown set')
            if settings.get('save') is not False:
                savers[settings['set']][(name, job)] = settings.get('save')
        self.assertEqual(savers, SAVERS)


class PruneWorkflow(unittest.TestCase):
    def setUp(self):
        self.workflow = load(WORKFLOWS / 'rust-cache-prune.yml')

    def test_runs_after_runs_of_main_s_head_from_this_repository_only(self):
        # PyYAML reads the bare key `on` as True.
        self.assertEqual(self.workflow[True], {'workflow_run': {
            'workflows': ['verify', 'nightly'], 'types': ['completed'], 'branches': ['main']}})
        guard = self.workflow['jobs']['prune']['if']
        self.assertIn("github.event.workflow_run.head_branch == 'main'", guard)
        self.assertIn('github.event.workflow_run.head_repository.full_name == github.repository', guard)
        self.assertIn("""contains(fromJSON('["push", "schedule", "workflow_dispatch"]'), github.event.workflow_run.event)""",
                      guard)

    def test_only_the_prune_job_may_write_actions(self):
        self.assertEqual(self.workflow['permissions'], {'contents': 'read'})
        self.assertEqual(self.workflow['jobs']['prune']['permissions'], {'contents': 'read', 'actions': 'write'})

    def test_hands_the_head_and_its_dependency_hash_as_the_action_computes_it(self):
        checkout, step = self.workflow['jobs']['prune']['steps']
        self.assertEqual(checkout['with']['ref'], '${{ github.event.workflow_run.head_sha }}')
        self.assertEqual(step['env']['HEAD_SHA'], '${{ github.event.workflow_run.head_sha }}')
        self.assertEqual(step['env']['DEPS'], f'${{{{ {DEPS} }}}}')
        self.assertIn('--head "$HEAD_SHA" --deps "$DEPS"', step['run'])

    def test_the_dependency_hash_covers_every_member_manifest(self):
        # `*/Cargo.toml` reaches members one folder down, and no deeper.
        members = load_toml(ROOT / 'Cargo.toml')['workspace']['members']
        self.assertEqual([member for member in members if '/' in member], [])


HEAD = 'c' * 40
B = 'b' * 64  # main's head
A = 'a' * 64  # an older commit


def entry(id, key, created='2026-10-28T04:00:00Z', ref=prune.MAIN):
    return {'id': id, 'key': key, 'ref': ref, 'created_at': created,
            'last_accessed_at': created, 'size_in_bytes': 1}


def key(set, deps, system='Linux-x64'):
    return f'v0-rust-{set}-{deps}-{system}-770ca156-de5473c3'


def gone(entries, deps=B):
    return [doomed['id'] for doomed in prune.doomed(entries, deps)]


class Prune(unittest.TestCase):
    def test_deletes_another_commit_s_entry_once_the_head_s_is_saved(self):
        self.assertEqual(gone([
            entry(1, key('workspace', B)),
            entry(2, key('workspace', A), created='2026-10-27T19:33:00Z'),
            entry(3, key('workspace', A, 'Windows_NT-x64'), created='2026-10-27T19:39:00Z'),
        ]), [2])

    def test_a_late_save_or_read_of_an_older_commit_never_deletes_the_head_s(self):
        # A candidate built A after the head's run, saving and reading A's
        # entry last; only the dependency hash names what to keep.
        self.assertEqual(gone([
            entry(1, key('release', B), created='2026-10-28T03:00:00Z'),
            entry(2, key('release', A), created='2026-10-28T05:30:00Z'),
        ]), [2])

    def test_leaves_a_set_whose_head_entry_is_not_saved_yet(self):
        self.assertEqual(gone([
            entry(1, key('platform', B)),
            entry(2, key('workspace', A)),
            entry(3, key('release', A)),
        ]), [])

    def test_never_deletes_a_key_from_before_the_sets_for_another_set_s_save(self):
        self.assertEqual(gone([
            entry(1, key('platform', B)),
            entry(2, 'v0-rust-web-e2e-mailbox-Linux-x64-770ca156-de5473c3'),
            entry(3, 'v0-rust-rust-Linux-x64-e3a6d410-de5473c3'),
        ]), [])

    def test_leaves_other_refs_and_other_caches(self):
        self.assertEqual(gone([
            entry(1, key('workspace', B)),
            entry(2, key('workspace', A), ref='refs/pull/1/merge'),
            entry(3, 'node-cache-Linux-x64-pnpm-a944ed6b'),
        ]), [])

    def run_main(self, entries, head=HEAD, delete=None):
        output = io.StringIO()
        answer = delete or subprocess.CompletedProcess([], 0, '', '')
        with mock.patch.object(prune, 'listed', return_value=entries), \
                mock.patch.object(prune, 'main_head', return_value=head), \
                mock.patch.object(prune.subprocess, 'run', return_value=answer) as run, \
                mock.patch.object(sys, 'argv', ['prune', '--repo', 'owner/repo', '--head', HEAD, '--deps', B]), \
                contextlib.redirect_stdout(output):
            self.assertEqual(prune.main(), 0)
        return output.getvalue(), run

    def test_deletes_nothing_once_main_has_moved_past_the_head(self):
        # A newer head's entry carries other deps and would look replaced.
        output, run = self.run_main([entry(1, key('workspace', B)), entry(2, key('workspace', A))], head='d' * 40)
        self.assertIn('nothing deleted', output)
        run.assert_not_called()

    def test_a_failed_delete_is_a_warning_and_the_run_passes(self):
        stale = entry(2, key('workspace', A))
        refused = subprocess.CompletedProcess([], 1, '', 'HTTP 403: Resource not accessible by integration')
        output, _ = self.run_main([entry(1, key('workspace', B)), stale], delete=refused)
        self.assertIn(f'::warning title=rust cache prune::deleting {stale["key"]} failed: HTTP 403', output)
        self.assertIn('2 of 2 Rust entries on main kept', output)


if __name__ == '__main__':
    unittest.main()
