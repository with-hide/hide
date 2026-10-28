"""Each Rust dependency set is saved by one job per system, on main only.

GitHub never replaces a cache entry, and a job whose key already exists saves
nothing, so two jobs saving one set race and the loser's build is never kept,
while a key per job kept the same dependencies four or five times over and
pushed main's own entries out of the 10 GB limit (2026-10-28). docs/BUILD.md,
"CI's Rust caches", owns the sets, the savers and the release argument.
"""
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/rust-cache'

ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"
# The root manifest, which rust-cache's own hash leaves out.
MANIFEST = "hashFiles('Cargo.toml')"
# Each set's savers, one per system: (workflow, job id) and the exact `save`.
SAVERS = {
    'workspace': {
        ('pr.yml', 'rust'): ON_MAIN,
        ('pr.yml', 'windows-check'): ON_MAIN,
        ('desktop-e2e.yml', 'e2e'):
            "${{ github.ref == 'refs/heads/main' && matrix.shard == 1 && runner.os == 'macOS' }}",
    },
    'platform': {('os-contract.yml', 'contract'): ON_MAIN},
    # The Windows app's binaries compile in their own job, beside the device
    # helpers, so it saves Windows's release set and the package matrix only
    # Linux's.
    'release': {('package.yml', 'package'): ON_MAIN, ('package.yml', 'macos-compile'): ON_MAIN,
                ('package.yml', 'windows-compile'): ON_MAIN},
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
            'shared-key': f'${{{{ inputs.set }}}}-${{{{ {MANIFEST} }}}}',
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


if __name__ == '__main__':
    unittest.main()
