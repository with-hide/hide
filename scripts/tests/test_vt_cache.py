"""hide-vt's CI entries are saved once per system, on main, and read by every job that builds hide-vt.

`.github/actions/vt-cache` restores hide-vt's prepared inputs and Zig's
caches for every job that builds hide-vt, and saves them only where its
`save` is true. GitHub never replaces an entry, so two savers of one key race
and the loser's build is lost; a pull request must never save, because what it
saved would be read by main's jobs. The vt lane saves Linux x64, macOS and
Windows; it has no Linux arm64 leg, so the remote mailbox lane's arm64 run
saves that system. A job that builds hide-vt without the action downloads the
pinned packages on every run, and one host being down then fails every pull
request (#2631). docs/BUILD.md, "The terminal parser and Zig", owns the keys.
"""
import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/vt-cache'
ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"
ON_MAIN_ARM64 = "${{ github.ref == 'refs/heads/main' && runner.arch == 'ARM64' }}"

# Every job that builds hide-vt with the shared inputs, and its exact `save`.
USERS = {
    ('vt.yml', 'build'): ON_MAIN,
    ('rust.yml', 'build'): False,
    ('rust.yml', 'lint'): False,
    ('pr.yml', 'merge-rust'): False,
    ('pr.yml', 'windows-check'): False,
    ('release.yml', 'test'): False,
    ('holder-restart.yml', 'restart'): False,
    ('remote-mailbox.yml', 'mailbox'): ON_MAIN_ARM64,
    ('repeat.yml', 'count'): False,
    ('repeat.yml', 'rust'): False,
    ('wsl-device.yml', 'linux-kit'): False,
    ('wsl-device.yml', 'windows'): False,
    ('package.yml', 'windows-compile'): False,
    ('package.yml', 'package'): False,
    ('package.yml', 'macos-compile'): False,
    ('release-candidate.yml', 'macos-binaries'): False,
    ('agent-journey.yml', 'fixture'): False,
    ('agent-journey.yml', 'fake'): False,
    ('agent-trust-contract.yml', 'contract'): False,
    ('desktop-e2e.yml', 'e2e'): False,
    ('perf.yml', 'measure'): False,
    ('web-e2e.yml', 'build'): False,
    ('web-e2e.yml', 'e2e'): False,
    ('device-helpers.yml', 'build'): False,
}

# `scripts/verify-cargo.sh` modes that always prepare hide-vt's inputs, and
# those that do when their cargo arguments select it (`selects_vt` there).
ALWAYS_VT = {'test', 'lint', 'release', 'cli'}
SCOPED = {'test-scoped', 'check', 'build', 'clippy', 'nextest', 'nextest-list'}
VT_FLAGS = {'--workspace', '--all', '-phide-vt', '--package=hide-vt', '-phide-holder', '--package=hide-holder'}
VT_PAIRS = {('-p', 'hide-vt'), ('--package', 'hide-vt'), ('-p', 'hide-holder'), ('--package', 'hide-holder')}


def selects_vt(arguments):
    return any(a in VT_FLAGS or (b, a) in VT_PAIRS for b, a in zip([''] + arguments, arguments))


def invokes_vt(text):
    """Whether a script's lines prepare hide-vt's inputs, directly or through `verify-cargo.sh`."""
    for line in text.splitlines():
        if 'prepare-vt-inputs.py' in line:
            return True
        found = re.search(r'verify-cargo\.sh\W+(\S+)(.*)', line)
        if not found:
            continue
        mode, rest = found.groups()
        arguments = [word.strip('\'",[]()') for word in re.split(r'\|\||&&|;', rest)[0].replace(',', ' ').split()]
        if mode.strip('\'",') in ALWAYS_VT or (mode.strip('\'",') in SCOPED and selects_vt(arguments)):
            return True
    return False


# Commands that build hide-vt inside a script rather than in the step's own
# `verify-cargo.sh` call: the device helpers' build builds the pane holder for
# its target (`desktop/scripts/device-helpers.mjs`, `build` only).
WRAPPERS = (re.compile(r'desktop/scripts/device-helpers\.mjs\s+build\b'),)


def builds_vt(run, env):
    """Whether a step builds hide-vt, with the job's variables put in."""
    text = re.sub(r'\$\{?([A-Z_][A-Z0-9_]*)\}?', lambda m: str(env.get(m.group(1), m.group(0))), run).replace('\\\n', ' ')
    return invokes_vt(text) or any(wrapper.search(text) for wrapper in WRAPPERS)


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

    def test_every_job_that_builds_hide_vt_restores_its_inputs(self):
        missing = []
        for workflow in sorted(WORKFLOWS.glob('*.yml')):
            document = yaml.safe_load(workflow.read_text())
            for job, body in (document.get('jobs') or {}).items():
                steps = body.get('steps') or []
                env = {**(document.get('env') or {}), **(body.get('env') or {})}
                builds = any(builds_vt(str(step.get('run', '')), {**env, **(step.get('env') or {})}) for step in steps)
                if builds and not any(step.get('uses') == ACTION for step in steps):
                    missing.append(f'{workflow.name}:{job}')
        self.assertEqual(missing, [], 'these jobs build hide-vt without .github/actions/vt-cache, so they download its packages every run')

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
