"""The relay binding tool's CI entry: every job that builds it restores it, one saves it.

`.github/actions/relay-tools` restores the wasm-bindgen command line that
`scripts/build-relay-wasm.sh` would otherwise compile, about a minute, on every
job that builds the relay WebAssembly. That is any job that builds or checks the
web shell (the `web` package's build, bundle, typecheck, test, build:phone and
e2e all run `prepare:relay` first), compiles the desktop package (which builds
the shell), runs a bare `verify-web.sh`, or runs the build script or a script
that calls it (`WRAPPERS`, which the test checks against the scripts). This test reads
every workflow for those jobs instead of keeping a list of them, so a new job
cannot miss the action.

The action saves only where its `save` is true. GitHub never replaces an entry,
so two savers of one key race, and a pull request must never save, because what
it saved would be read by main's jobs.
"""
import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / '.github' / 'workflows'
ACTION = './.github/actions/relay-tools'
ON_MAIN = "${{ github.ref == 'refs/heads/main' }}"

# The jobs that save: the Linux shell build of a sharded lane, and a
# single-shard lane (Windows, and the nightly's macOS) for itself. Every other
# job restores, with `save: false`.
SAVERS = {
    ('web-e2e.yml', 'build-web'): ON_MAIN,
    ('web-e2e.yml', 'e2e'): "${{ github.ref == 'refs/heads/main' && inputs.shards == 1 }}",
}

# A job that builds the relay and still compiles the tool itself, with the reason.
# The phone job publishes to production behind a test that holds every action it
# runs to a pinned commit (test_phone_release.py); the action's own cache step is
# a mutable tag, and a restored binary would build what is published.
UNRESTORED = {
    ('release-candidate.yml', 'phone'): 'its actions are pinned to a commit',
}

# What a `run` step calls to reach the relay WebAssembly build. `package
# assemble` assembles the app from binaries `package compile` built, and
# `e2e-list` only lists tests, so neither builds the shell. A bare
# `verify-web.sh` runs the web build and the phone build.
# Arguments are split by spaces in a shell line and by `", "` in a Node argv.
SPLIT = r'''[\s"',]+'''
DIRECT = (
    r'build-relay-wasm\.sh'
    rf'|verify-web\.sh{SPLIT}web{SPLIT}(?:build:phone|build|bundle|typecheck|test|e2e)(?![-\w:])'
    rf'|verify-web\.sh{SPLIT}desktop{SPLIT}package(?!{SPLIT}assemble)'
    r'|verify-web\.sh\s*(?:$|\|)'
)


def code_lines(path):
    return [line for line in path.read_text().splitlines() if not re.match(r'\s*(#|//|\*|/\*)', line)]


# The scripts whose code reaches DIRECT, read from the repository, and how a
# workflow step calls each of them so that the call builds. A script that
# calls one of these in turn is not followed: `package.mjs` builds the shell
# with pnpm and is reached through `verify-web.sh desktop package`.
WRAPPERS = {
    'check-relay-protocol.sh': r'check-relay-protocol\.sh',
    'device-helpers.mjs': rf'device-helpers\.mjs{SPLIT}build',
}
DIRECT_PATTERN = re.compile(DIRECT, re.M)
CALLERS = sorted(
    path.name for pattern in ('scripts/*.sh', 'desktop/scripts/*.mjs') for path in sorted(ROOT.glob(pattern))
    if not path.name.endswith('.test.mjs') and path.name not in ('verify-web.sh', 'build-relay-wasm.sh')
    and DIRECT_PATTERN.search('\n'.join(code_lines(path)))
)
BUILDS = re.compile(DIRECT + ''.join(f'|{call}' for call in WRAPPERS.values()), re.M)


def jobs_with_steps():
    for workflow in sorted(WORKFLOWS.glob('*.yml')):
        for job, body in (yaml.safe_load(workflow.read_text()).get('jobs') or {}).items():
            if body.get('steps'):
                yield workflow.name, job, body['steps']


def first_index(steps, wanted):
    return next((index for index, step in enumerate(steps) if wanted(step)), None)


class RelayToolsCache(unittest.TestCase):
    def test_the_scan_finds_the_jobs_it_should(self):
        # A scan that matched nothing would pass every other test.
        # A script that builds the relay and is not in WRAPPERS would be missed by the scan.
        self.assertEqual(CALLERS, sorted(WRAPPERS))
        builders = {(workflow, job) for workflow, job, steps in jobs_with_steps()
                    if first_index(steps, lambda step: BUILDS.search(step.get('run') or '')) is not None}
        self.assertTrue(set(SAVERS) <= builders)
        self.assertIn(('pr.yml', 'checks'), builders)
        # A bare `verify-web.sh` and a script that calls the build.
        self.assertIn(('release.yml', 'test'), builders)
        self.assertIn(('device-helpers.yml', 'build'), builders)
        self.assertNotIn(('package.yml', 'macos-package'), builders)

    def test_every_job_that_builds_the_relay_restores_the_tool_first(self):
        # A job that builds the WebAssembly without the tool compiles it again.
        for workflow, job, steps in jobs_with_steps():
            build = first_index(steps, lambda step: BUILDS.search(step.get('run') or ''))
            if build is None:
                continue
            if (workflow, job) in UNRESTORED:
                continue
            with self.subTest(workflow=workflow, job=job):
                restore = first_index(steps, lambda step: step.get('uses') == ACTION)
                self.assertIsNotNone(restore, f'{workflow} {job} builds the relay without {ACTION}')
                self.assertLess(restore, build)

    def test_a_job_uses_the_tool_only_where_it_builds_the_relay(self):
        for workflow, job, steps in jobs_with_steps():
            if first_index(steps, lambda step: step.get('uses') == ACTION) is None:
                continue
            self.assertNotIn((workflow, job), UNRESTORED)
            with self.subTest(workflow=workflow, job=job):
                self.assertIsNotNone(first_index(steps, lambda step: BUILDS.search(step.get('run') or '')))

    def test_one_saver_per_system_on_main_and_the_rest_only_restore(self):
        found = {}
        for workflow, job, steps in jobs_with_steps():
            for step in steps:
                if step.get('uses') == ACTION:
                    found[(workflow, job)] = step['with']['save']
        for key, save in found.items():
            with self.subTest(job=key):
                self.assertEqual(save, SAVERS.get(key, False))
        self.assertTrue(set(SAVERS) <= set(found))

    def test_the_key_is_the_locked_version_and_the_system(self):
        action = yaml.safe_load((ROOT / '.github/actions/relay-tools/action.yml').read_text())
        keys = {step['with']['key'] for step in action['runs']['steps'] if 'with' in step}
        self.assertEqual(keys, {'relay-tools-v1-${{ runner.os }}-${{ runner.arch }}-${{ steps.version.outputs.version }}'})
        self.assertIn('target/relay-tools', (ROOT / 'scripts/build-relay-wasm.sh').read_text())


if __name__ == '__main__':
    unittest.main()
