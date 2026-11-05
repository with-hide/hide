"""The relay binding tool's CI entry has one saver per system, on main.

`.github/actions/relay-tools` restores the wasm-bindgen command line that
`scripts/build-relay-wasm.sh` would otherwise compile on every web job, and
saves it only where its `save` is true. GitHub never replaces an entry, so two
savers of one key race, and a pull request must never save, because what it
saved would be read by main's jobs.
"""
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/relay-tools'
ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"

# Every job that builds the relay WebAssembly, and its exact `save`: the Linux
# shell build of a sharded lane, and a single-shard lane (Windows) for itself.
USERS = {
    ('pr.yml', 'checks'): False,
    ('pr.yml', 'merge-web'): False,
    ('pr.yml', 'windows-check'): False,
    ('web-e2e.yml', 'build-web'): ON_MAIN,
    ('web-e2e.yml', 'e2e'): "${{ github.ref == 'refs/heads/main' && inputs.shards == 1 }}",
}


class RelayToolsCache(unittest.TestCase):
    def test_one_saver_per_system_on_main_and_the_rest_only_restore(self):
        found = {}
        for workflow in sorted(WORKFLOWS.glob('*.yml')):
            jobs = yaml.safe_load(workflow.read_text()).get('jobs', {})
            for job, body in jobs.items():
                for step in body.get('steps', []):
                    if step.get('uses') == ACTION:
                        found[(workflow.name, job)] = step['with']['save']
        self.assertEqual(found, USERS)

    def test_every_web_job_that_prepares_the_relay_restores_the_tool_first(self):
        # A job that builds the WebAssembly without the tool compiles it again.
        for (workflow, job), _ in USERS.items():
            steps = yaml.safe_load((WORKFLOWS / workflow).read_text())['jobs'][job]['steps']
            uses = [index for index, step in enumerate(steps) if step.get('uses') == ACTION]
            builds = [index for index, step in enumerate(steps)
                      if 'build-relay-wasm' in (step.get('run') or '') or 'verify-web.sh web' in (step.get('run') or '')
                      or 'verify-web.sh desktop' in (step.get('run') or '')]
            with self.subTest(workflow=workflow, job=job):
                self.assertTrue(builds)
                self.assertLess(uses[0], builds[0])

    def test_the_key_is_the_locked_version_and_the_system(self):
        action = yaml.safe_load((ROOT / '.github/actions/relay-tools/action.yml').read_text())
        keys = {step['with']['key'] for step in action['runs']['steps'] if 'with' in step}
        self.assertEqual(keys, {'relay-tools-v1-${{ runner.os }}-${{ runner.arch }}-${{ steps.version.outputs.version }}'})
        self.assertIn('target/relay-tools', (ROOT / 'scripts/build-relay-wasm.sh').read_text())


if __name__ == '__main__':
    unittest.main()
