"""`scripts/prepare-ssh-accounts.sh` names the command it stops in.

with-hide/hide-private#898: the remote mailbox lane's preparation step ran
with no output, timed out at 3 minutes, and the log named none of its
commands, so nobody could say where it stopped. The script prints each
command's name before it starts, stops a command that overruns its limit, and
keeps the whole run inside one budget. These tests run it against stand-ins
for `sudo`, the account tools and `ssh-keygen`, one of which can hang.
"""
import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = ROOT / 'scripts' / 'prepare-ssh-accounts.sh'

# Every stand-in appends its own invocation, so a test can say which command
# ran. `HANG=<name>` makes that one wait far past any limit a test sets.
STAND_IN = r'''#!/usr/bin/env bash
echo "{name} $*" >> "$SSH_LOG"
[ "${{HANG:-}}" = "{name}" ] && exec sleep 60
exit 0
'''
# `sudo` records the command it was given and then runs it, so the account
# tools' own stand-ins are what a test observes behind it.
SUDO = r'''#!/usr/bin/env bash
echo "sudo $*" >> "$SSH_LOG"
[ "${HANG:-}" = sudo ] && exec sleep 60
# The sshd the script starts is the system's own, by its absolute path; the
# stand-in stands in for it so a test never starts a server on the machine.
if [ "$1" = /usr/sbin/sshd ]; then
  shift
  exec "$SSH_SANDBOX/sshd" "$@"
fi
exec "$@"
'''
# The Linux branch creates `/run/sshd`, which is the runner's own file system
# and which macOS refuses; the stand-in creates the same tree under the test's
# folder so the run can be exercised on either system, and the real `mkdir`
# does everything else.
MKDIR = r'''#!/usr/bin/env bash
arguments=()
for argument in "$@"; do
  case "$argument" in
    /run|/run/*) argument="$SSH_SANDBOX$argument" ;;
  esac
  arguments+=("$argument")
done
exec REAL_MKDIR "${arguments[@]}"
'''
# The default key's public half must exist for the script to name it, and the
# host key's own `-f` path is the path the script then writes into the
# configuration.
SSH_KEYGEN = r'''#!/usr/bin/env bash
echo "ssh-keygen $*" >> "$SSH_LOG"
[ "${HANG:-}" = ssh-keygen ] && exec sleep 60
case " $* " in
  *" -f "*)
    out=""
    arguments=("$@")
    for ((i = 0; i < ${#arguments[@]}; i++)); do
      [ "${arguments[i]}" = -f ] && out="${arguments[i + 1]}"
    done
    [ -n "$out" ] && : > "$out"
    ;;
esac
exit 0
'''


class PrepareSshAccounts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.home = root / 'home'
        self.home.mkdir()
        self.runner_temp = root / 'runner-temp'
        self.runner_temp.mkdir()
        self.github_env = root / 'github-env'
        self.github_env.touch()
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        # The account tools and the ones that change the machine are stand-ins;
        # `mkdir`, `chmod` and `chown` are the real ones, because the script's
        # own files and folders must exist for the rest of it to run, and
        # `openssl` is the machine's own, so the password it writes is real.
        for name in ('useradd', 'chpasswd', 'sysadminctl', 'createhomedir', 'chown'):
            (bin_dir / name).write_text(STAND_IN.format(name=name))
            (bin_dir / name).chmod(0o755)
        (bin_dir / 'sudo').write_text(SUDO)
        (bin_dir / 'sudo').chmod(0o755)
        (bin_dir / 'ssh-keygen').write_text(SSH_KEYGEN)
        (bin_dir / 'ssh-keygen').chmod(0o755)
        sandbox = root / 'sandbox'
        sandbox.mkdir()
        (sandbox / 'sshd').write_text(STAND_IN.format(name='sshd'))
        (sandbox / 'sshd').chmod(0o755)
        (bin_dir / 'mkdir').write_text(MKDIR.replace('REAL_MKDIR', shutil.which('mkdir')))
        (bin_dir / 'mkdir').chmod(0o755)
        self.log = root / 'calls.log'
        self.env = {
            **os.environ,
            'HOME': str(self.home),
            'PATH': f'{bin_dir}:{os.environ["PATH"]}',
            'SSH_LOG': str(self.log),
            'SSH_SANDBOX': str(sandbox),
            'GITHUB_ENV': str(self.github_env),
            'RUNNER_TEMP': str(self.runner_temp),
            'RUNNER_OS': 'Linux',
        }

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def run_script(self, **env):
        started = time.monotonic()
        done = subprocess.run(['bash', str(SCRIPT)], env={**self.env, **env},
                              capture_output=True, text=True, timeout=120)
        return done, time.monotonic() - started

    def test_every_command_is_named_before_it_runs(self):
        done, _ = self.run_script()
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        printed = [line for line in done.stdout.splitlines() if line.startswith('-> ')]
        self.assertGreaterEqual(len(printed), 8, printed)
        self.assertTrue(any('useradd hidepw' in line for line in printed), printed)
        self.assertTrue(any('chpasswd hidepw' in line for line in printed), printed)
        self.assertTrue(any('the password sshd starts' in line for line in printed), printed)
        # Each named step is a command the stand-ins really saw, so the names
        # are the commands that ran and not decoration.
        self.assertTrue(any(call.startswith('sudo useradd -m -s /bin/sh hidepw') for call in self.calls()),
                        self.calls())

    def test_a_hanging_command_is_stopped_and_named_inside_the_budget(self):
        done, seconds = self.run_script(HANG='useradd', PREPARE_COMMAND_SECONDS='3',
                                        PREPARE_BUDGET_SECONDS='20')
        self.assertEqual(done.returncode, 1)
        self.assertIn('the password account (useradd hidepw) did not finish within 3 s and was stopped',
                      done.stderr)
        # The run ends inside its budget, not at the runner's step limit.
        self.assertLess(seconds, 3 + 10, 'the script must end inside its budget')
        # Nothing past the command that hung ran, so the log's last name is the one that stopped.
        self.assertFalse(any('sshd' in call for call in self.calls()), self.calls())

    def test_a_budget_under_one_commands_limit_still_ends_the_run(self):
        # The command's own limit is a minute and the run has four seconds:
        # the smaller of the two is what the command gets, so a stuck command
        # late in an almost-spent run cannot hold the step to its limit.
        done, seconds = self.run_script(HANG='ssh-keygen', PREPARE_COMMAND_SECONDS='60',
                                        PREPARE_BUDGET_SECONDS='4')
        self.assertEqual(done.returncode, 1)
        self.assertRegex(done.stderr,
                         r'the default key ssh finds by itself did not finish within \d+ s')
        self.assertLess(seconds, 4 + 10, 'the budget, not the command limit, ends this run')

    def test_a_missing_tool_is_named_before_anything_runs(self):
        # A runner without one of the script's own tools says so in one line
        # up front, instead of failing part way with a command it cannot name.
        empty = Path(self.temp.name) / 'empty-bin'
        empty.mkdir()
        failing = subprocess.run(
            [shutil.which('bash'), str(SCRIPT)], env={**self.env, 'PATH': str(empty)},
            capture_output=True, text=True, timeout=120)
        self.assertEqual(failing.returncode, 1)
        self.assertIn('this runner has no', failing.stderr)
        self.assertIn('openssl', failing.stderr)
        self.assertEqual(self.calls(), [])

    def test_the_two_variables_the_journeys_read_are_named(self):
        done, _ = self.run_script()
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        written = self.github_env.read_text()
        self.assertIn('HIDE_E2E_SSH_DEFAULT_KEY=', written)
        self.assertIn('HIDE_E2E_SSH_PASSWORD=hidepw:', written)
        self.assertIn(':2222', written)

    def test_the_password_is_masked_and_never_passed_as_an_argument(self):
        done, _ = self.run_script()
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        self.assertIn('::add-mask::', done.stdout)
        password = done.stdout.split('::add-mask::')[1].splitlines()[0]
        self.assertRegex(password, '^[0-9a-f]{32}$', 'openssl writes the password, unmasked, once')
        for call in self.calls():
            self.assertNotIn(password, call, 'the password reached a command line')

    def test_a_bad_limit_is_refused_before_anything_runs(self):
        for value in ('0', '-1', 'soon'):
            with self.subTest(value=value):
                done, _ = self.run_script(PREPARE_BUDGET_SECONDS=value)
                self.assertEqual(done.returncode, 2)
                self.assertIn('positive whole number', done.stderr)
                self.assertEqual(self.calls(), [])

    def test_macos_makes_the_account_the_mac_way(self):
        done, _ = self.run_script(RUNNER_OS='macOS')
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        calls = self.calls()
        self.assertTrue(any(call.startswith('sudo sysadminctl -addUser hidepw') for call in calls), calls)
        self.assertTrue(any(call.startswith('sudo createhomedir -c -u hidepw') for call in calls), calls)
        self.assertFalse(any(call.startswith('sudo useradd') for call in calls), calls)

    def test_the_summary_says_how_long_it_took_and_what_it_was_allowed(self):
        done, _ = self.run_script(PREPARE_BUDGET_SECONDS='150', PREPARE_COMMAND_SECONDS='45')
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        self.assertIn('prepare-ssh-accounts: ready in ', done.stdout)
        self.assertIn('(budget 150 s, 45 s a command)', done.stdout)

    def test_the_scripts_budget_is_shorter_than_the_workflow_steps_limit(self):
        # The step that once named nothing did so because its own
        # `timeout-minutes` was the only limit and it ended the run. The
        # script's budget has to be the shorter one, so the script usually
        # ends a stuck run itself and says where; the step's limit is the
        # backstop for a hang the script cannot see
        # (with-hide/hide-private#898).
        workflow = yaml.safe_load((ROOT / '.github' / 'workflows' / 'remote-mailbox.yml').read_text())
        steps = [step for step in workflow['jobs']['mailbox']['steps']
                 if 'prepare-ssh-accounts.sh' in str(step.get('run', ''))]
        self.assertEqual(len(steps), 1, 'one step runs the preparation')
        step = steps[0]
        step_seconds = step['timeout-minutes'] * 60

        def limit(name, default):
            # The default the script uses when the workflow names no value.
            found = re.search(rf'\$\{{{name}:-(\d+)\}}', SCRIPT.read_text())
            self.assertIsNotNone(found, f'the script names a default for {name}')
            return int(found.group(1))

        budget = limit('PREPARE_BUDGET_SECONDS', 150)
        command = limit('PREPARE_COMMAND_SECONDS', 45)
        self.assertLess(budget, step_seconds,
                        'the script ends a stuck run before the step limit does')
        self.assertLess(command, budget, 'a command gets less than the whole run')
        self.assertLessEqual(step_seconds - budget, 60,
                             'the step limit leaves the script room to say why it stopped')


if __name__ == '__main__':
    unittest.main()
