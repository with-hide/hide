"""A script that runs cargo must reuse the machine's toolchain, not install one.

rustup resolves its toolchain from RUSTUP_HOME, which defaults to
`$HOME/.rustup`. A verification runner gets its own HOME, and rustup answers
that empty directory by downloading and installing the whole toolchain into it
while still exiting 0:

    $ env HOME="$(mktemp -d)" cargo --version
    warn: the missing active toolchain `stable-aarch64-apple-darwin` has been
          auto-installed
    $ echo $?
    0

Because the exit code is 0, nothing downstream notices. Every run directory
under `agents/runs/` had grown its own copy: 1.3 GB of `.rustup` plus 128 MB of
`.cargo` per run, 9.1 GB across eleven runs.

Two earlier test scripts had both already diagnosed this, and both workarounds
were unreachable for the same reason: they were guarded by a cargo invocation
failing, which the auto-install prevents. Resolution now lives once, in
`toolchain-env.sh`. This test is what keeps the next cargo-running script
from rediscovering the problem rather than sourcing it.
"""
import re
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
RESOLVER = 'scripts/toolchain-env.sh'

# The resolver is sourced, so it does not invoke cargo itself and is not a
# caller.
EXEMPT = {
    RESOLVER,
}

CARGO_CALL = re.compile(r'(?:^|[|;&(]|\bexec |\bthen |\$\()\s*cargo\b')


def tracked_shell_scripts():
    listed = subprocess.check_output(['git', 'ls-files', 'scripts'],
                                     cwd=ROOT, text=True).split()
    return [relative for relative in listed if relative.endswith('.sh')]


def scripts_that_run_cargo():
    callers = []
    for relative in tracked_shell_scripts():
        for line in (ROOT / relative).read_text().splitlines():
            if CARGO_CALL.search(line.split('#', 1)[0]):
                callers.append(relative)
                break
    return callers


class ToolchainReuse(unittest.TestCase):
    def test_the_resolver_is_tracked(self):
        tracked = subprocess.check_output(['git', 'ls-files', RESOLVER], cwd=ROOT, text=True)
        self.assertEqual(tracked.strip(), RESOLVER,
                         f'{RESOLVER} owns toolchain resolution but is not tracked')

    def test_every_script_that_runs_cargo_sources_the_resolver(self):
        offenders = []
        for relative in scripts_that_run_cargo():
            if relative in EXEMPT:
                continue
            if RESOLVER not in (ROOT / relative).read_text():
                offenders.append(relative)
        self.assertEqual(
            offenders, [],
            'these scripts run cargo without sourcing ' + RESOLVER + ', so rustup will install a\n'
            'private toolchain under a runner HOME:\n  ' + '\n  '.join(offenders))

    def test_the_resolver_defers_to_an_explicit_toolchain(self):
        text = (ROOT / RESOLVER).read_text()
        for variable in ('CARGO_HOME', 'RUSTUP_HOME'):
            self.assertIn(f'export {variable}="${{{variable}:-',
                          text,
                          f'{RESOLVER} must not overwrite an explicitly set {variable}')

    def test_the_resolver_is_sourced_and_never_executed(self):
        """A subshell's exports do not reach its caller, so a run would be silent."""
        offenders = []
        for relative in tracked_shell_scripts():
            if relative == RESOLVER:
                continue
            for number, line in enumerate((ROOT / relative).read_text().splitlines(), start=1):
                code = line.split('#', 1)[0]
                if RESOLVER not in code:
                    continue
                if not re.match(r'\s*(?:\.|source)\s', code):
                    offenders.append(f'{relative}:{number}: {line.strip()}')
        self.assertEqual(offenders, [], f'{RESOLVER} must be sourced, not executed:\n'
                                        + '\n'.join(offenders))


if __name__ == '__main__':
    unittest.main()
