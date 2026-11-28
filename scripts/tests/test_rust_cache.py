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
# `package.yml`'s choice between a pull request's quick set and the release set.
BY_BUILD = "${{ inputs.quick-build && 'quick' || 'release' }}"
# The root manifest, which rust-cache's own hash leaves out.
MANIFEST = "hashFiles('Cargo.toml')"
# Each set's savers, one per system: (workflow, job id) and the exact `save`.
SAVERS = {
    'workspace': {
        ('rust.yml', 'build'): ON_MAIN,
        # The Windows test build, the most any Windows job builds and the
        # lane's longest job; `compile` beside it only restores.
        ('windows-check.yml', 'tests'): ON_MAIN,
        # The one macOS build of a desktop e2e lane with shards; the shards
        # build nothing, and a lone shard only restores.
        ('desktop-e2e.yml', 'build'):
            "${{ github.ref == 'refs/heads/main' && runner.os == 'macOS' }}",
    },
    # Clippy's check of the dependencies, which no test build makes; the Rust
    # lane's lint job runs beside its build rather than in it.
    'lint': {('rust.yml', 'lint'): ON_MAIN},
    'platform': {('os-contract.yml', 'contract'): ON_MAIN},
    # The Windows app's binaries compile in their own job, beside the device
    # helpers, so it saves Windows's release set and the package matrix only
    # Linux's.
    'release': {('package.yml', 'package'): ON_MAIN, ('package.yml', 'macos-compile'): ON_MAIN,
                ('package.yml', 'windows-compile'): ON_MAIN},
    # A pull request's package build reads it; `package cache` builds it at
    # the same settings.
    'quick': {('package-cache.yml', 'quick'): ON_MAIN},
    # The device helpers' release build, one entry per processor.
    'helper': {('device-helpers.yml', 'build'): ON_MAIN},
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

    def test_a_job_whose_binaries_ship_removes_every_workspace_crate_it_restored(self):
        clean = self.steps[2]
        self.assertEqual(clean['if'], "inputs.set == 'release' || inputs.set == 'helper'")
        self.assertIn('cargo clean --release --workspace ${TARGET:+--target "$TARGET"}', clean['run'])
        self.assertEqual(clean['env'], {'TARGET': '${{ inputs.target }}'})

    def test_the_device_helpers_clean_the_folder_their_target_builds_in(self):
        steps = load(WORKFLOWS / 'device-helpers.yml')['jobs']['build']['steps']
        cache = next(step for step in steps if step.get('uses') == ACTION)
        self.assertEqual(cache['with']['target'], '${{ matrix.target }}')


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
            settings = dict(step.get('with') or {})
            if settings.get('set') == BY_BUILD:
                # A package build reads the quick set in a pull request and the
                # release set otherwise; only the release set is ever saved.
                self.assertEqual(settings['save'], f"${{{{ !inputs.quick-build && {ON_MAIN[4:-3]} }}}}")
                settings = {'set': 'release', 'save': ON_MAIN}
            self.assertIn(settings.get('set'), SAVERS, f'{name} job {job}: unknown set')
            if settings.get('save') is not False:
                savers[settings['set']][(name, job)] = settings.get('save')
        self.assertEqual(savers, SAVERS)

    def test_no_workflow_that_saves_the_quick_set_runs_for_a_pull_request(self):
        triggers = load(WORKFLOWS / 'package-cache.yml')[True]
        self.assertEqual(set(triggers), {'push', 'schedule', 'workflow_dispatch'})
        self.assertEqual(triggers['push']['branches'], ['main'])

    def test_only_this_repositorys_runs_on_main_start_the_prune(self):
        # `branches` matches a head branch by name, which a fork's pull request
        # can share, so the job that may delete entries checks the run itself.
        workflow = load(WORKFLOWS / 'cache-generations.yml')
        self.assertEqual(workflow[True], {
            'workflow_run': {'workflows': ['verify', 'nightly', 'package cache'], 'types': ['completed'],
                             'branches': ['main']},
            'workflow_dispatch': None})
        self.assertEqual(workflow['permissions'], {'contents': 'read'})
        job = workflow['jobs']['prune']
        self.assertEqual(job['if'], "github.event_name == 'workflow_dispatch' || ( "
                         "github.event.workflow_run.event != 'pull_request' && "
                         "github.event.workflow_run.event != 'merge_group' && "
                         "github.event.workflow_run.head_repository.full_name == github.repository && "
                         "github.event.workflow_run.head_branch == 'main')")
        self.assertEqual(job['permissions'], {'actions': 'write', 'contents': 'read', 'issues': 'write'})


if __name__ == '__main__':
    unittest.main()
