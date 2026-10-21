"""The public CLI rejects traces and unreadable watches without launching an agent."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(SOURCE / 'scripts'))


@unittest.skipUnless(sys.platform == 'darwin', 'local private mode requires the macOS sandbox')
class PrivateCLI(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.checkout = self.base / 'checkout'
        self.operator = self.base / 'operator'
        self.operator.mkdir()
        self.run = self.checkout / 'agents/runs/fixture-run'
        for name in ('agent_journey', 'agent_live_check'):
            shutil.copytree(SOURCE / 'scripts' / name, self.checkout / 'scripts' / name,
                            ignore=shutil.ignore_patterns('__pycache__'))
        contracts = self.checkout / 'contracts'
        contracts.mkdir()
        shutil.copyfile(SOURCE / 'contracts/agent-journey.json', contracts / 'agent-journey.json')
        # These executable fixtures would fail if reached: watch refusal must happen first.
        program = b'#!/bin/sh\necho unexpected_execution >&2\nexit 99\n'
        for name in ('hided', 'hide', 'herdr', 'examples/journey_conversation', 'examples/journey_memory',
                     'examples/journey_composer'):
            path = self.checkout / 'target/debug' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(program)
            path.chmod(0o700)
        (contracts / 'herdr-bundle.json').write_text(json.dumps({
            'version': 'fixture', 'sha256': hashlib.sha256(program).hexdigest()}))
        self.env = {'HOME': str(self.operator), 'PATH': os.environ['PATH'], 'LC_ALL': 'C',
                    'PYTHONPATH': str(self.checkout / 'scripts'),
                    'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': os.devnull,
                    'GIT_AUTHOR_NAME': 'fixture', 'GIT_AUTHOR_EMAIL': 'fixture@invalid',
                    'GIT_COMMITTER_NAME': 'fixture', 'GIT_COMMITTER_EMAIL': 'fixture@invalid'}
        for args in (['init', '-q'], ['commit', '-q', '--allow-empty', '-m', 'fixture']):
            subprocess.run(['/usr/bin/git', *args], cwd=self.checkout, env=self.env, check=True,
                           capture_output=True)

    def invoke(self, during_package=''):
        command = 'from agent_journey.cli import main; import sys; sys.exit(main(sys.argv[1:]))'
        if during_package:
            # The OS directory boundary orders the competing operator write
            # after the initial watch and before the unconditional end check.
            package = self.run / 'probe/package/hide.app/Contents/Resources'
            command = (f'import os\noriginal_mkdir = os.mkdir\n'
                       f'def mkdir(path, *args, **kwargs):\n'
                       f'    if os.fspath(path) == {str(package)!r}:\n'
                       + ''.join('        ' + line + '\n' for line in during_package.splitlines())
                       + '    return original_mkdir(path, *args, **kwargs)\n'
                       + 'os.mkdir = mkdir\n' + command)
        result = subprocess.run([sys.executable, '-c', command, '--mode', 'local', '--fake-model',
                                 '--agents', 'pi', '--run-dir', str(self.run), '--herdr-bin',
                                 str(self.checkout / 'target/debug/herdr')],
                                env=self.env, cwd=self.checkout, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        report = json.loads((self.run / 'report.json').read_text())
        self.assertTrue(report['cleanup']['confirmed'], report['cleanup'])
        self.assertTrue(report['cleanup']['watch_copies_removed'])
        self.assertFalse((self.run / 'watch-private').exists())
        self.assertNotIn('unexpected_execution', result.stdout + result.stderr)
        return report

    def test_end_comparison_catches_configuration_link_and_new_entry_traces(self):
        config = self.operator / '.codex/config.toml'
        config.parent.mkdir()
        link = self.operator / '.local/bin/claude'
        link.parent.mkdir(parents=True)
        for case, expected in (('config', '.codex/config.toml'),
                               ('link', '.local/bin/claude'), ('new', '.codex')):
            with self.subTest(case=case):
                shutil.rmtree(self.run, ignore_errors=True)
                config.write_text('external configuration')
                link.unlink(missing_ok=True)
                link.symlink_to(self.base / 'old-program')
                entry = config.parent / 'new-session'
                entry.unlink(missing_ok=True)
                action = {
                    'config': f'open({str(config)!r}, "w").write({str(self.run)!r})',
                    'link': f'os.unlink({str(link)!r})\nos.symlink({str(self.run / "program")!r}, {str(link)!r})',
                    'new': f'open({str(entry)!r}, "w").write({str(self.run)!r})',
                }[case]
                report = self.invoke(during_package=action)
                self.assertEqual(report['operator_watch']['changed'], [expected])
                # The package's missing hook binary proves the initial guard
                # admitted the run; its separate error cannot hide end refusal.
                self.assertTrue(any(row['reason'] == 'build_this_worktrees_hide_agent_hooks_first'
                                    for row in report['failures']))
                self.assertTrue(any('operator_watch_run_trace' in row['reason']
                                    for row in report['failures']))

    def test_run_path_in_configuration_is_exit_two_and_reports_no_hash_or_link_target(self):
        config = self.operator / '.codex/auth.json'
        config.parent.mkdir()
        config.write_text(json.dumps({'fixture': str(self.run)}))
        link = self.operator / '.local/bin/claude'
        link.parent.mkdir(parents=True)
        private_target = self.base / 'private-operator-program'
        link.symlink_to(private_target)
        report = self.invoke()
        self.assertEqual(report['operator_watch']['changed'], ['.codex/auth.json'])
        self.assertTrue(all('operator_watch_run_trace' in row['reason'] for row in report['failures']))
        for suffix in ('json', 'md'):
            output = (self.run / ('report.' + suffix)).read_text()
            self.assertNotIn(hashlib.sha256(config.read_bytes()).hexdigest(), output)
            self.assertNotIn(str(private_target), output)
            self.assertNotIn('"entries"', output)
            self.assertNotIn('"target"', output)

    def test_link_retargeted_into_run_is_exit_two(self):
        link = self.operator / '.local/bin/claude'
        link.parent.mkdir(parents=True)
        link.symlink_to(self.base / 'old-program')
        link.unlink()
        link.symlink_to(self.run / 'program')
        report = self.invoke()
        self.assertEqual(report['operator_watch']['changed'], ['.local/bin/claude'])

    def test_unreadable_database_schema_still_removes_private_copy(self):
        database = self.operator / '.omp/agent/agent.db'
        database.parent.mkdir(parents=True)
        with sqlite3.connect(database) as connection:
            connection.execute('CREATE TABLE unrelated (value TEXT)')
        report = self.invoke()
        self.assertEqual([row['reason'] for row in report['failures']],
                         ['operator_watch_database_snapshot_unreadable: OperationalError'])

    def test_installer_outside_write_fails_and_removes_its_incomplete_pin_after_child_cleanup(self):
        from agent_journey.cli_cache import Cache, pin_key
        from agent_live_check.processes import OwnedProcesses
        from agent_live_check.protection import ProtectionError, private_directory
        from agent_live_check.runtime import Runtime
        outside = self.base / 'outside-cache'
        outside.write_text('unchanged')
        script = self.checkout / 'scripts/install-agent-clis.py'
        script.write_text("import pathlib, sys\n"
                          "prefix = pathlib.Path(sys.argv[sys.argv.index('--prefix') + 1])\n"
                          "(prefix / 'partial').write_text('incomplete')\n"
                          f"pathlib.Path({str(outside)!r}).write_text('outside write')\n")
        private_directory(self.run)
        owner = OwnedProcesses(diagnostics=self.run / 'process-diagnostics')
        runtime = Runtime(self.checkout, self.run, self.operator, owner,
                          herdr_bin=self.checkout / 'target/debug/herdr', private_env=True)
        self.addCleanup(runtime.close)
        cache = Cache(self.base / 'cache', 1024 ** 2)
        self.addCleanup(cache.close)
        pin = {'version': '1.0.0', 'installer': 'npm'}
        with self.assertRaisesRegex(ProtectionError, 'journey_cli_install_failed'):
            cache.install(runtime, 'fixture', pin, {})
        self.assertFalse((cache.root / pin_key('fixture', pin)).exists())
        self.assertEqual(outside.read_text(), 'unchanged')
        self.assertFalse(owner.children)
        owner.attribution_report()

    def test_native_install_link_survives_removal_of_temporary_home(self):
        from agent_journey.cli_cache import Cache
        from agent_live_check.processes import OwnedProcesses
        from agent_live_check.protection import private_directory
        from agent_live_check.runtime import Runtime
        script = self.checkout / 'scripts/install-agent-clis.py'
        script.write_text("import pathlib, sys, json\n"
                          "prefix = pathlib.Path(sys.argv[sys.argv.index('--prefix') + 1])\n"
                          "payload = prefix / 'home/native-program'\n"
                          "payload.write_text('fixture executable')\n"
                          "program = prefix / 'program'\n"
                          "program.symlink_to(payload)\n"
                          "(prefix / 'installed.json').write_text(json.dumps({'fixture': str(program)}))\n")
        private_directory(self.run)
        owner = OwnedProcesses(diagnostics=self.run / 'process-diagnostics')
        runtime = Runtime(self.checkout, self.run, self.operator, owner,
                          herdr_bin=self.checkout / 'target/debug/herdr', private_env=True)
        self.addCleanup(runtime.close)
        cache = Cache(self.base / 'cache', 1024 ** 2)
        self.addCleanup(cache.close)
        pin = {'version': '1.0.0', 'installer': 'npm'}
        program = cache.install(runtime, 'fixture', pin, {})
        self.assertEqual(program.read_text(), 'fixture executable')
        self.assertFalse((program.parent / 'home').exists())
        self.assertEqual(cache.program('fixture', pin), program)

    def test_unresolvable_claude_link_does_not_expose_its_path(self):
        config = self.operator / '.claude.json'
        config.symlink_to(config)
        report = self.invoke()
        self.assertEqual([row['reason'] for row in report['failures']],
                         ['operator_watch_claude_configuration_unreadable'])
        for suffix in ('json', 'md'):
            self.assertNotIn(str(config), (self.run / ('report.' + suffix)).read_text())


if __name__ == '__main__':
    unittest.main()
