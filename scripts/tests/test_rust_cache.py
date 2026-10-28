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

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/rust-cache'
MAIN = "github.ref == 'refs/heads/main'"

SPEC = importlib.util.spec_from_file_location('prune', ROOT / 'scripts' / 'prune-rust-caches.py')
prune = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prune)

# Each set's savers, one per system: (workflow, job id).
SAVERS = {
    'workspace': {('pr.yml', 'rust'), ('pr.yml', 'windows-check'), ('desktop-e2e.yml', 'e2e')},
    'platform': {('os-contract.yml', 'contract')},
    'release': {('package.yml', 'package'), ('package.yml', 'macos-compile')},
}


def cache_steps():
    for path in sorted(WORKFLOWS.glob('*.yml')):
        workflow = yaml.safe_load(path.read_text())
        for job_id, job in (workflow.get('jobs') or {}).items():
            for step in job.get('steps') or []:
                yield path.name, job_id, step


class RustCacheSteps(unittest.TestCase):
    def test_every_job_takes_its_cache_from_the_one_action(self):
        direct = [f'{name} job {job}' for name, job, step in cache_steps()
                  if str(step.get('uses', '')).startswith('Swatinem/rust-cache')]
        self.assertEqual(direct, [], f'use {ACTION}, which keys by set rather than by job')

    def test_each_set_is_saved_only_by_its_savers_and_only_on_main(self):
        savers = {name: set() for name in SAVERS}
        for name, job, step in cache_steps():
            if step.get('uses') != ACTION:
                continue
            settings = step.get('with') or {}
            self.assertIn(settings.get('set'), SAVERS, f'{name} job {job}: unknown set')
            save = settings.get('save')
            if save is False:
                continue
            self.assertIn(MAIN, str(save), f'{name} job {job} saves off main')
            savers[settings['set']].add((name, job))
        self.assertEqual(savers, SAVERS)

    def test_the_prune_runs_after_main_s_runs_only(self):
        workflow = yaml.safe_load((WORKFLOWS / 'rust-cache-prune.yml').read_text())
        # PyYAML reads the bare key `on` as True.
        trigger = workflow[True]['workflow_run']
        self.assertEqual(trigger['branches'], ['main'])
        guard = workflow['jobs']['prune']['if']
        self.assertIn("github.ref == 'refs/heads/main'", guard)
        self.assertIn("workflow_run.event != 'pull_request'", guard)


def entry(id, key, created, ref=prune.MAIN):
    return {'id': id, 'key': key, 'ref': ref, 'created_at': created, 'size_in_bytes': 1}


MANIFEST = 'a' * 64


class Prune(unittest.TestCase):
    def test_keeps_the_newest_entry_of_each_set_and_system(self):
        entries = [
            entry(1, f'v0-rust-workspace-{MANIFEST}-Linux-x64-e3a6d410-75249a22', '2026-10-27T19:33:00Z'),
            entry(2, f'v0-rust-workspace-{MANIFEST}-Linux-x64-e3a6d410-de5473c3', '2026-10-28T04:39:00Z'),
            # Another root manifest is the same set and system.
            entry(3, f'v0-rust-workspace-{"b" * 64}-Linux-x64-e3a6d410-de5473c3', '2026-10-28T01:00:00Z'),
            entry(4, f'v0-rust-workspace-{MANIFEST}-Windows_NT-x64-15d0a0c1-75249a22', '2026-10-27T19:39:00Z'),
            entry(5, f'v0-rust-release-{MANIFEST}-Linux-x64-e3a6d410-75249a22', '2026-10-27T19:40:00Z'),
        ]
        self.assertEqual([gone['id'] for gone in prune.doomed(entries)], [1, 3])

    def test_deletes_main_keys_from_before_the_sets_and_leaves_other_refs(self):
        entries = [
            entry(1, 'v0-rust-rust-Linux-x64-e3a6d410-de5473c3', '2026-10-28T04:39:00Z'),
            entry(2, 'v0-rust-rust-Linux-x64-e3a6d410-de5473c3', '2026-10-28T04:39:00Z', ref='refs/pull/1/merge'),
            entry(3, 'node-cache-Linux-x64-pnpm-a944ed6b', '2026-10-28T04:42:00Z'),
            entry(4, f'v0-rust-platform-{MANIFEST}-Darwin-arm64-f8d01195-de5473c3', '2026-10-28T04:34:00Z'),
        ]
        self.assertEqual([gone['id'] for gone in prune.doomed(entries)], [1])

    def test_a_failed_delete_is_a_warning_and_the_run_passes(self):
        stale = entry(1, 'v0-rust-rust-Linux-x64-e3a6d410-de5473c3', '2026-10-28T04:39:00Z')
        refused = subprocess.CompletedProcess([], 1, '', 'HTTP 403: Resource not accessible by integration')
        output = io.StringIO()
        with mock.patch.object(prune, 'listed', return_value=[stale]), \
                mock.patch.object(prune.subprocess, 'run', return_value=refused), \
                mock.patch.object(sys, 'argv', ['prune', '--repo', 'owner/repo']), \
                contextlib.redirect_stdout(output):
            self.assertEqual(prune.main(), 0)
        self.assertIn('::warning title=rust cache prune::deleting v0-rust-rust-Linux-x64-e3a6d410-de5473c3 failed: HTTP 403',
                      output.getvalue())
        self.assertIn('1 of 1 Rust entries on main kept', output.getvalue())


if __name__ == '__main__':
    unittest.main()
