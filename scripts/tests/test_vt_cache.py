"""hide-vt's CI entries are saved by the vt lane alone, once per system, on main.

`.github/actions/vt-cache` restores hide-vt's prepared inputs and Zig's
caches for every job that builds hide-vt, and saves them only where its
`save` is true. GitHub never replaces an entry, so two savers of one key race
and the loser's build is lost; a pull request must never save, because what it
saved would be read by main's jobs. docs/BUILD.md, "The terminal parser and
Zig", owns the keys.
"""
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/vt-cache'
ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"

# Every job that builds hide-vt with the shared inputs, and its exact `save`.
USERS = {
    ('vt.yml', 'build'): ON_MAIN,
    ('pr.yml', 'rust'): False,
    ('pr.yml', 'windows-check'): False,
    ('release.yml', 'test'): False,
    ('holder-restart.yml', 'restart'): False,
}


class VtCache(unittest.TestCase):
    def test_the_vt_lane_is_the_one_saver_and_the_other_builders_only_restore(self):
        found = {}
        for workflow in sorted(WORKFLOWS.glob('*.yml')):
            jobs = yaml.safe_load(workflow.read_text()).get('jobs', {})
            for job, body in jobs.items():
                for step in body.get('steps', []):
                    if step.get('uses') == ACTION:
                        found[(workflow.name, job)] = step['with']['save']
        self.assertEqual(found, USERS)

    def test_each_key_names_what_its_entry_was_built_from(self):
        action = yaml.safe_load((ROOT / '.github/actions/vt-cache/action.yml').read_text())
        keys = {step['name']: step['with']['key'] for step in action['runs']['steps'] if 'with' in step}
        for name, key in keys.items():
            with self.subTest(step=name):
                self.assertIn('${{ runner.os }}-${{ runner.arch }}', key)
                self.assertIn("hashFiles('hide-vt/build-inputs.json'", key)
                if 'Zig' in name:
                    self.assertIn("'hide-vt/build.rs'", key)


if __name__ == '__main__':
    unittest.main()
