"""A device measurement must refuse any target but the isolated account.

A private sshd still signs in as the account itself, so a `~/` folder spelled
from any HOME but the private one lands in the account's own. On 2026-10-09 a
probe with the `~/` defaults uploaded its node into a production account's
`~/.hide/host-helper` while every command it ran saw the private HOME. `scripts/web-shell-measure/device-guard.sh` is what
stops the measurement harness doing the same; this checks that it refuses
each such target before anything dials, accepts the isolated one, and writes
the alias hided reads in the one form its parser and OpenSSH read alike.
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
GUARD = ROOT / 'scripts' / 'web-shell-measure' / 'device-guard.sh'

HOST_KEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFixtureFixtureFixtureFixtureFixtureFixture'

ISOLATED = {
    'MEASURE_DEVICE_ALIAS': 'private',
    'MEASURE_DEVICE_HOST': '127.0.0.1',
    'MEASURE_DEVICE_PORT': '22841',
    'MEASURE_DEVICE_USER': 'me',
    'MEASURE_DEVICE_HOME': '/tmp/hcn/home',
    'MEASURE_DEVICE_SOCKET': '/tmp/hcn/h.sock',
    'MEASURE_DEVICE_HELPER_ROOT': '/tmp/hcn/home/.hide/host-helper',
    'MEASURE_DEVICE_CLI_DIR': '/tmp/hcn/home/.local/bin',
}


class MeasureDeviceGuard(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.folder = Path(folder.name)
        self.identity = self.folder / 'client_key'
        self.identity.write_text('not a key; the guard only checks it exists\n')
        self.known_hosts = self.known_hosts_with(f'[127.0.0.1]:22841 {HOST_KEY}\n')

    def known_hosts_with(self, text):
        path = self.folder / f'known_hosts_{abs(hash(text))}'
        path.write_text(text)
        return path

    def guard(self, script='echo accepted', **changes):
        variables = {**ISOLATED, 'MEASURE_DEVICE_IDENTITY': str(self.identity),
                     'MEASURE_DEVICE_KNOWN_HOSTS': str(self.known_hosts), **changes}
        env = {name: value for name, value in os.environ.items()
               if not name.startswith('MEASURE_DEVICE_')}
        env.update({name: value for name, value in variables.items() if value is not None})
        return subprocess.run(['bash', '-c', f'source "$1"; {script}', 'guard', str(GUARD)],
                              env=env, capture_output=True, text=True, timeout=30)

    def test_the_isolated_account_is_accepted(self):
        result = self.guard()
        self.assertEqual((result.returncode, result.stdout.strip()), (0, 'accepted'),
                         result.stderr)

    def test_the_alias_hided_reads_is_written_in_one_plain_form(self):
        result = self.guard(script='device_guard_config')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, (
            'Host private\n'
            '  HostName 127.0.0.1\n'
            '  Port 22841\n'
            '  User me\n'
            f'  IdentityFile {self.identity}\n'
            '  IdentityAgent none\n'))
        result = self.guard(script='printf "%s\\n" "${device_guard_ssh[@]}"')
        self.assertEqual(result.returncode, 0, result.stderr)
        options = result.stdout.splitlines()
        self.assertEqual(options[:2], ['-F', '/dev/null'])
        for option in ('HostName=127.0.0.1', 'Port=22841', 'User=me',
                       f'UserKnownHostsFile={self.known_hosts}', 'GlobalKnownHostsFile=/dev/null',
                       'StrictHostKeyChecking=yes'):
            self.assertIn(option, options)

    def test_every_other_target_is_refused_before_dialing(self):
        port_22_key = self.known_hosts_with(f'127.0.0.1 {HOST_KEY}\n')
        explicit_22 = self.known_hosts_with(
            f'[127.0.0.1]:22841 {HOST_KEY}\n[127.0.0.1]:22 {HOST_KEY}\n')
        hashed = self.known_hosts_with(f'|1|c2FsdA==|aGFzaA== {HOST_KEY}\n')
        authority = self.known_hosts_with(f'@cert-authority * {HOST_KEY}\n')
        empty = self.known_hosts_with('# nothing recorded\n')
        refusals = [
            ({'MEASURE_DEVICE_SSH_CONFIG': '/tmp/config'}, 'MEASURE_DEVICE_SSH_CONFIG is not read'),
            ({'MEASURE_DEVICE_HELPER_ROOT': '~/.hide/host-helper'}, 'MEASURE_DEVICE_HELPER_ROOT'),
            ({'MEASURE_DEVICE_CLI_DIR': '~/.local/bin'}, 'MEASURE_DEVICE_CLI_DIR'),
            ({'MEASURE_DEVICE_HELPER_ROOT': '/opt/me/.hide/host-helper'},
             'MEASURE_DEVICE_HELPER_ROOT'),
            ({'MEASURE_DEVICE_HELPER_ROOT': '/tmp/hcn/home/../../opt/me/.hide/host-helper'},
             'MEASURE_DEVICE_HELPER_ROOT'),
            ({'MEASURE_DEVICE_HOME': '/opt/me'}, 'MEASURE_DEVICE_HOME'),
            ({'MEASURE_DEVICE_HOME': None}, 'MEASURE_DEVICE_HOME'),
            ({'MEASURE_DEVICE_SOCKET': '~/.config/herdr/herdr.sock'}, 'MEASURE_DEVICE_SOCKET'),
            ({'MEASURE_DEVICE_PORT': '22'}, 'is 22'),
            ({'MEASURE_DEVICE_PORT': '022'}, 'is 22'),
            ({'MEASURE_DEVICE_PORT': 'ssh'}, 'is not a port'),
            ({'MEASURE_DEVICE_PORT': '70000'}, 'is not a port'),
            ({'MEASURE_DEVICE_ALIAS': '-oProxyCommand=sh'}, 'MEASURE_DEVICE_ALIAS'),
            ({'MEASURE_DEVICE_ALIAS': 'private own'}, 'MEASURE_DEVICE_ALIAS'),
            ({'MEASURE_DEVICE_USER': '-l'}, 'MEASURE_DEVICE_USER'),
            ({'MEASURE_DEVICE_HOST': '127.0.0.1\tPort 22'}, 'MEASURE_DEVICE_HOST'),
            ({'MEASURE_DEVICE_IDENTITY': 'client_key'}, 'MEASURE_DEVICE_IDENTITY'),
            ({'MEASURE_DEVICE_IDENTITY': '/tmp/no such key'}, 'MEASURE_DEVICE_IDENTITY'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': str(port_22_key)}, 'another host or port'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': str(explicit_22)}, 'another host or port'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': str(hashed)}, 'another host or port'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': str(authority)}, 'another host or port'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': str(empty)}, 'records no key'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': '/tmp/hcn/absent'}, 'not a readable file'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': f'{self.known_hosts} /tmp/other'},
             'not an absolute path without spaces'),
            ({'MEASURE_DEVICE_KNOWN_HOSTS': 'known_hosts'}, 'not an absolute path without spaces'),
        ]
        for changes, refusal in refusals:
            with self.subTest(changes=changes):
                result = self.guard(**changes)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertNotIn('accepted', result.stdout)
                self.assertIn(refusal, result.stderr)

    def test_the_harness_runs_the_guard_before_it_dials(self):
        measure = ROOT / 'scripts' / 'web-shell-measure'
        run = (measure / 'run.sh').read_text()
        guarded = run.index('source "$measure_dir/device-guard.sh"')
        for dial in ('"$measure_dir/device-herdr.sh"', '"$measure_dir/device-front.mjs"',
                     'spawn_owned hided', 'device_guard_config >'):
            self.assertLess(guarded, run.index(dial), f'run.sh reaches {dial} before the guard')
        self.assertNotIn('MEASURE_DEVICE_SSH_CONFIG', run)
        herdr = (measure / 'device-herdr.sh').read_text()
        self.assertLess(herdr.index('device-guard.sh'), herdr.index('exec ssh'))
        self.assertIn('exec ssh "${device_guard_ssh[@]}" --', herdr)


if __name__ == '__main__':
    unittest.main()
