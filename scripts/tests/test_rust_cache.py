"""Each Rust dependency set is saved by one job per system, on main only.

GitHub never replaces a cache entry, and a job whose key already exists saves
nothing, so two jobs saving one set race and the loser's build is never kept,
while a key per job kept the same dependencies four or five times over and
pushed main's own entries out of the 10 GB limit (2026-10-28). docs/BUILD.md,
"CI's Rust caches", owns the sets, the savers and the release argument.
"""
import contextlib
from datetime import datetime, timezone
import importlib.util
import io
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/rust-cache'

SPEC = importlib.util.spec_from_file_location('prune', ROOT / 'scripts' / 'prune-rust-caches.py')
prune = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prune)

ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"
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

    def test_keys_by_set_and_root_manifest_keeps_dependencies_only_and_saves_as_told(self):
        self.assertEqual(self.steps[1]['with'], {
            'workspaces': '. -> target',
            'shared-key': "${{ inputs.set }}-${{ hashFiles('Cargo.toml') }}",
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

    def test_passes_when_the_head_s_run_started(self):
        step = self.workflow['jobs']['prune']['steps'][-1]
        self.assertEqual(step['env']['SINCE'], '${{ github.event.workflow_run.run_started_at }}')
        self.assertIn('--since "$SINCE"', step['run'])


RUN = datetime(2026, 10, 28, 6, 0, tzinfo=timezone.utc)
M = 'a' * 64


def entry(id, key, created, accessed=None, ref=prune.MAIN):
    return {'id': id, 'key': key, 'ref': ref, 'created_at': created,
            'last_accessed_at': accessed or created, 'size_in_bytes': 1}


def gone(entries):
    return [doomed['id'] for doomed in prune.doomed(entries, RUN)]


class Prune(unittest.TestCase):
    def test_keeps_what_the_head_s_run_restored_and_deletes_the_rest_of_its_group(self):
        self.assertEqual(gone([
            # Restored by the run (an exact hit moves only last_accessed_at).
            entry(1, f'v0-rust-workspace-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T04:39:00Z',
                  accessed='2026-10-28T06:05:00Z'),
            entry(2, f'v0-rust-workspace-{M}-Linux-x64-770ca156-75249a22', '2026-10-27T19:33:00Z'),
            # Another root manifest is the same set and system.
            entry(3, f'v0-rust-workspace-{"b" * 64}-Linux-x64-770ca156-75249a22', '2026-10-27T19:30:00Z'),
        ]), [3, 2])

    def test_never_deletes_the_head_s_entry_for_a_later_save_of_an_older_commit(self):
        # A candidate built an older commit and saved its key after the
        # head's last restore; the head's run touched only its own.
        self.assertEqual(gone([
            entry(1, f'v0-rust-release-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T03:00:00Z',
                  accessed='2026-10-28T06:10:00Z'),
            entry(2, f'v0-rust-release-{M}-Linux-x64-770ca156-75249a22', '2026-10-28T05:30:00Z'),
        ]), [2])

    def test_keeps_an_entry_saved_after_the_run_started(self):
        # A later merge's run saved a newer key while this one ran.
        self.assertEqual(gone([
            entry(1, f'v0-rust-workspace-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T06:05:00Z'),
            entry(2, f'v0-rust-workspace-{M}-Linux-x64-770ca156-0badf00d', '2026-10-28T06:20:00Z'),
            entry(3, f'v0-rust-workspace-{M}-Linux-x64-770ca156-75249a22', '2026-10-27T19:33:00Z'),
        ]), [3])

    def test_leaves_a_group_the_run_did_not_touch(self):
        # A push run builds no release, and a failed save leaves nothing new.
        self.assertEqual(gone([
            entry(1, f'v0-rust-release-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T03:00:00Z'),
            entry(2, f'v0-rust-release-{M}-Linux-x64-770ca156-75249a22', '2026-10-27T19:33:00Z'),
        ]), [])

    def test_deletes_a_key_from_before_the_sets_only_when_its_system_has_a_replacement(self):
        self.assertEqual(gone([
            entry(1, 'v0-rust-rust-Linux-x64-e3a6d410-de5473c3', '2026-10-28T04:39:00Z'),
            entry(2, 'v0-rust-web-e2e-mailbox-Linux-x64-770ca156-de5473c3', '2026-10-28T04:30:00Z'),
            entry(3, f'v0-rust-workspace-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T06:30:00Z'),
            # No Windows entry of the sets was saved or restored yet.
            entry(4, 'v0-rust-windows-check-Windows_NT-x64-15d0a0c1-de5473c3', '2026-10-28T04:39:00Z'),
        ]), [2, 1])

    def test_leaves_other_refs_and_other_caches(self):
        self.assertEqual(gone([
            entry(1, f'v0-rust-workspace-{M}-Linux-x64-770ca156-75249a22', '2026-10-27T19:33:00Z',
                  ref='refs/pull/1/merge'),
            entry(2, 'node-cache-Linux-x64-pnpm-a944ed6b', '2026-10-27T04:42:00Z'),
            entry(3, f'v0-rust-workspace-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T06:05:00Z'),
        ]), [])

    def test_a_failed_delete_is_a_warning_and_the_run_passes(self):
        stale = entry(1, f'v0-rust-workspace-{M}-Linux-x64-770ca156-75249a22', '2026-10-27T19:33:00Z')
        head = entry(2, f'v0-rust-workspace-{M}-Linux-x64-770ca156-de5473c3', '2026-10-28T06:05:00Z')
        refused = subprocess.CompletedProcess([], 1, '', 'HTTP 403: Resource not accessible by integration')
        output = io.StringIO()
        with mock.patch.object(prune, 'listed', return_value=[stale, head]), \
                mock.patch.object(prune.subprocess, 'run', return_value=refused), \
                mock.patch.object(sys, 'argv', ['prune', '--repo', 'owner/repo', '--since', '2026-10-28T06:00:00Z']), \
                contextlib.redirect_stdout(output):
            self.assertEqual(prune.main(), 0)
        self.assertIn(f'::warning title=rust cache prune::deleting {stale["key"]} failed: HTTP 403', output.getvalue())
        self.assertIn('2 of 2 Rust entries on main kept', output.getvalue())


if __name__ == '__main__':
    unittest.main()
