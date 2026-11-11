"""`scripts/ci-apt-install.sh` keeps a slow Ubuntu mirror inside one time limit.

with-hide/hide-private#629: a stalled mirror held an `apt-get install` for
minutes and the job's own timeout killed it, so the failure named nothing. The
script owns the whole limit (`APT_BUDGET_SECONDS`), fetches with
`--download-only` until the limit is gone, and installs once, offline. These
tests run it against a stand-in `apt-get` and `sudo` that record each call.
"""
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / 'ci-apt-install.sh'

SUDO = '#!/usr/bin/env bash\nexec "$@"\n'
# `fail=N` in the stand-in's state makes the first N downloads fail; `hang`
# makes every download wait far longer than any test's limit.
APT_GET = r'''#!/usr/bin/env bash
echo "$*" >> "$APT_LOG"
case " $* " in
  *" --download-only "*)
    [ -n "${APT_HANG:-}" ] && exec sleep 60
    n=$(cat "$APT_STATE" 2>/dev/null || echo 0)
    echo $(( n + 1 )) > "$APT_STATE"
    [ "$n" -lt "${APT_FAIL_FIRST:-0}" ] && exit 100
    exit 0 ;;
esac
exit 0
'''

# DNS and resolver configuration cross the OS boundary. These stand-ins
# expose whether the configured resolver can answer, without changing the
# developer's or CI runner's real network settings.
GETENT = r'''#!/usr/bin/env bash
if [ -e "$DNS_STATE" ]; then
  [ "${DNS_REPAIRED_OK:-1}" = 1 ] || exit 2
  if [ -n "${DNS_FAILED_SERVER:-}" ] && [[ $(cat "$DNS_STATE") == *"$DNS_FAILED_SERVER"* ]]; then exit 2; fi
else
  [ "${DNS_DEFAULT_OK:-1}" = 1 ] || exit 2
fi
echo '185.125.190.82 STREAM archive.ubuntu.com'
'''
RESOLVECTL = r'''#!/usr/bin/env bash
if [ "$1" = dns ]; then printf '%s\n' "$3" > "$DNS_STATE"; fi
'''
TEE = r'''#!/usr/bin/env bash
cat > "$DNS_STATE"
'''

# GNU `timeout` ships with the Linux runners this script runs on; macOS lacks
# it, so a developer's machine gets a stand-in with the one form the script
# uses: `timeout --kill-after=<n> <seconds> <command>...`.
TIMEOUT = r'''#!/usr/bin/env perl
shift @ARGV if $ARGV[0] =~ /^--kill-after=/;
my $limit = shift @ARGV;
my $pid = fork();
if ($pid == 0) { exec @ARGV or exit 127; }
$SIG{ALRM} = sub { kill 'TERM', $pid; waitpid($pid, 0); exit 124; };
alarm $limit;
waitpid($pid, 0);
exit($? >> 8);
'''


class CiAptInstall(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        stand_ins = [('sudo', SUDO), ('apt-get', APT_GET),
                     ('uname', '#!/usr/bin/env bash\necho Linux\n'),
                     ('getent', GETENT), ('resolvectl', RESOLVECTL), ('tee', TEE),
                     ('ip', '#!/usr/bin/env bash\necho "default via 10.0.0.1 dev eth0"\n')]
        if shutil.which('timeout') is None:
            stand_ins.append(('timeout', TIMEOUT))
        for name, body in stand_ins:
            (bin_dir / name).write_text(body)
            (bin_dir / name).chmod(0o755)
        self.log = root / 'apt.log'
        self.dns_state = root / 'dns'
        self.env = {
            **os.environ,
            'PATH': f'{bin_dir}:{os.environ["PATH"]}',
            'APT_LOG': str(self.log),
            'APT_STATE': str(root / 'state'),
            'APT_RETRY_PAUSE_SECONDS': '1',
            'DNS_STATE': str(self.dns_state),
            'RUNNER_ENVIRONMENT': 'github-hosted',
        }

    def run_script(self, *packages, **env):
        started = time.monotonic()
        done = subprocess.run(['bash', str(SCRIPT), *packages], env={**self.env, **env},
                              capture_output=True, text=True, timeout=60)
        calls = self.log.read_text().splitlines() if self.log.exists() else []
        return done, calls, time.monotonic() - started

    def test_a_failed_fetch_refreshes_the_lists_and_the_install_happens_once(self):
        done, calls, _ = self.run_script('zsh', 'fish', APT_BUDGET_SECONDS='30', APT_FAIL_FIRST='2')
        self.assertEqual(done.returncode, 0, done.stderr)
        fetches = [c for c in calls if '--download-only' in c]
        installs = [c for c in calls if ' install ' in f' {c} ' and '--download-only' not in c]
        self.assertEqual(len(fetches), 3)
        self.assertEqual(sum(' update ' in f' {c} ' for c in calls), 2)
        self.assertEqual(len(installs), 1)
        self.assertIn('--no-download', installs[0])
        self.assertTrue(installs[0].endswith('zsh fish'), installs[0])

    def test_a_mirror_that_never_answers_ends_inside_the_budget_naming_the_packages(self):
        done, calls, seconds = self.run_script('zsh', 'fish', APT_BUDGET_SECONDS='3', APT_HANG='1')
        self.assertEqual(done.returncode, 1)
        self.assertIn('could not fetch zsh fish within 3 s', done.stderr)
        self.assertFalse([c for c in calls if '--download-only' not in c and ' install ' in f' {c} '],
                         'nothing is installed when nothing was fetched')
        self.assertLess(seconds, 3 + 8, 'the script must end inside its budget, not at the runner\'s step limit')

    def test_every_fetch_failing_gives_up_when_the_budget_is_spent(self):
        done, calls, seconds = self.run_script('zsh', APT_BUDGET_SECONDS='3', APT_FAIL_FIRST='1000')
        self.assertEqual(done.returncode, 1)
        self.assertIn('could not fetch zsh within 3 s', done.stderr)
        self.assertGreater(len([c for c in calls if '--download-only' in c]), 1)
        self.assertLess(seconds, 3 + 8)

    def test_a_cache_folder_is_where_apt_keeps_the_files_and_is_cleaned_after(self):
        folder = Path(self.temp.name) / 'debs'
        done, calls, _ = self.run_script('zsh', APT_BUDGET_SECONDS='30', APT_CACHE_DIR=str(folder))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(all(f'Dir::Cache::archives={folder}' in c for c in calls), calls)
        self.assertEqual(sorted(p.name for p in folder.iterdir()), [])

    def test_a_call_without_packages_or_with_a_bad_budget_is_refused(self):
        self.assertEqual(self.run_script()[0].returncode, 2)
        self.assertEqual(self.run_script('zsh', APT_BUDGET_SECONDS='0')[0].returncode, 2)

    def test_healthy_dns_keeps_the_default_resolver(self):
        done, _, _ = self.run_script('git', APT_BUDGET_SECONDS='30')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(self.dns_state.exists())

    def test_failed_dns_is_repaired_before_packages_are_installed(self):
        done, calls, _ = self.run_script('git', APT_BUDGET_SECONDS='30', DNS_DEFAULT_OK='0')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('1.1.1.1', self.dns_state.read_text())
        self.assertTrue(any('--no-download' in call for call in calls))

    def test_failed_dns_repair_reports_failure_without_installing(self):
        done, calls, _ = self.run_script('git', APT_BUDGET_SECONDS='30',
                                         DNS_DEFAULT_OK='0', DNS_REPAIRED_OK='0')
        self.assertEqual(done.returncode, 1)
        self.assertIn('Ubuntu package DNS still fails', done.stderr)
        self.assertEqual(calls, [])

    def test_an_unreachable_public_resolver_does_not_hide_a_working_candidate(self):
        done, _, _ = self.run_script('git', APT_BUDGET_SECONDS='30',
                                     DNS_DEFAULT_OK='0', DNS_FAILED_SERVER='1.1.1.1')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn('8.8.8.8', self.dns_state.read_text())

    def test_a_non_disposable_runner_refuses_dns_changes(self):
        done, calls, _ = self.run_script('git', APT_BUDGET_SECONDS='30',
                                         DNS_DEFAULT_OK='0', RUNNER_ENVIRONMENT='self-hosted')
        self.assertEqual(done.returncode, 1)
        self.assertIn('restricted to disposable GitHub-hosted runners', done.stderr)
        self.assertFalse(self.dns_state.exists())
        self.assertEqual(calls, [])

    def test_local_debs_and_options_are_refused_before_network_or_installation(self):
        for argument in ('./hide.deb', '/w/hide.deb', '--allow-downgrades'):
            with self.subTest(argument=argument):
                done, calls, _ = self.run_script(argument, APT_BUDGET_SECONDS='30')
                self.assertEqual(done.returncode, 2)
                self.assertIn('accepts repository package names', done.stderr)
                self.assertEqual(calls, [])
                self.assertFalse(self.dns_state.exists())


if __name__ == '__main__':
    unittest.main()
