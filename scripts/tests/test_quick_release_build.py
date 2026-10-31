"""Only a pull request's package lane builds its release binaries unoptimized.

A pull request checks that the app packages, installs, starts and updates, so
it skips the release profile's LTO; main's push run, the nightly, a candidate
and a release build what ships, and main's runs save the release dependency
set the next build reads. docs/BUILD.md, "A pull request's release build",
owns the measurement.
"""
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/quick-release-build'


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
                self.assertLess(uses.index('./.github/actions/rust-cache'), uses.index(ACTION))


if __name__ == '__main__':
    unittest.main()
