"""Only a pull request's package lane builds its release binaries unoptimized.

A pull request checks that the app packages, installs, starts and updates, so
it skips the release profile's LTO; the nightly, a candidate and a release
build what ships, and main's runs save the release dependency set the next
such build reads. A pull request reads the quick set instead, which
`package-cache.yml` saves on main with the compile jobs' own steps.
docs/BUILD.md, "A pull request's release build", owns the measurement.
"""
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/quick-release-build'
CACHE = './.github/actions/rust-cache'


def load(name):
    return yaml.safe_load((WORKFLOWS / name).read_text())


class QuickReleaseBuildTest(unittest.TestCase):
    def test_only_a_pull_request_asks_for_it(self):
        callers = {}
        for path in sorted(WORKFLOWS.glob('*.yml')):
            for job_id, job in (yaml.safe_load(path.read_text()).get('jobs') or {}).items():
                if job.get('uses') == './.github/workflows/package.yml':
                    callers[(path.name, job_id)] = (job.get('with') or {}).get('quick-build')
        self.assertIn(('pr.yml', 'package'), callers)
        self.assertEqual(callers.pop(('pr.yml', 'package')), "${{ github.event_name == 'pull_request' }}")
        self.assertTrue(callers, 'the nightly, a candidate and a release call package.yml too')
        self.assertEqual(set(callers.values()), {None}, callers)
        self.assertIs(load('package.yml')[True]['workflow_call']['inputs']['quick-build']['default'], False)

    def test_every_release_build_honours_it_after_the_cache_restore(self):
        for job_id, job in load('package.yml')['jobs'].items():
            steps = job.get('steps', [])
            uses = [step.get('uses') for step in steps]
            # `assemble` packages binaries a compile job built; the others build them.
            runs = [step.get('run') or '' for step in steps]
            builds = any('verify-web.sh desktop package' in run and 'assemble' not in run for run in runs)
            with self.subTest(job=job_id):
                if not builds:
                    self.assertNotIn(ACTION, uses)
                    continue
                self.assertEqual(uses.count(ACTION), 1)
                quick = steps[uses.index(ACTION)]
                self.assertEqual(quick['if'], 'inputs.quick-build')
                # The settings are in the environment before the restore, so the
                # quick set's key changes with them.
                self.assertEqual(uses.count(CACHE), 1)
                self.assertEqual(uses.index(CACHE), uses.index(ACTION) + 1)
                # The quick build reads the set built at its settings and saves
                # nothing; every other build reads and on main saves the release set.
                self.assertEqual(steps[uses.index(CACHE)]['with'], {
                    'set': "${{ inputs.quick-build && 'quick' || 'release' }}",
                    'save': "${{ !inputs.quick-build && github.ref == 'refs/heads/main' }}"})

    def test_main_saves_the_quick_set_with_the_compile_jobs_own_steps(self):
        # The same actions in the same order and the same build: rust-cache
        # keys the environment it restores in, the quick settings included, and
        # cargo reuses a dependency only at the profile, features and target it
        # was built with.
        def shape(steps):
            return [step.get('uses') or step['run'] for step in steps
                    if step.get('uses') != 'actions/checkout@v7']
        compile_steps = load('package.yml')['jobs']['windows-compile']['steps']
        compiled = shape(compile_steps)
        compiled = compiled[:compiled.index('bash scripts/verify-web.sh desktop package compile') + 1]
        saver = load('package-cache.yml')['jobs']['quick']
        self.assertEqual(shape(saver['steps']), compiled)
        self.assertEqual(saver['strategy']['matrix']['include'], [
            {'system': 'linux', 'runner': load('package.yml')['jobs']['package']['runs-on']},
            {'system': 'windows', 'runner': load('package.yml')['jobs']['windows-compile']['runs-on']},
            {'system': 'macos', 'runner': 'macos-15'},
        ])


if __name__ == '__main__':
    unittest.main()
