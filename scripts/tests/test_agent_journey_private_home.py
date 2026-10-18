"""Run traces stop; external configuration and entry changes are reported.

Every filesystem case owns a fake operator root. None reads Path.home().
"""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cli_cache, clis, fake_lane, fake_private, fake_model, database_watch
from agent_live_check.protection import ProtectionError


class PrivateHome(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def test_directory_mtime_and_existing_session_appends_are_not_configuration_changes(self):
        folder = self.root / '.claude'
        folder.mkdir()
        history = folder / 'history.jsonl'
        history.write_text('first record\n')
        watch = fake_private.OperatorWatch(self.root)
        self.assertEqual(watch.check()['changed'], [])
        info = folder.stat()
        with history.open('a') as stream:
            stream.write('next record\n')
        os.utime(folder, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
        self.assertEqual(watch.check()['changed'], [])

    def test_external_configuration_change_with_same_size_and_mtime_is_reported(self):
        config = self.root / '.codex/config.toml'
        config.parent.mkdir()
        config.write_text('model="first"\n')
        info = config.stat()
        watch = fake_private.OperatorWatch(self.root)
        config.write_text('model="other"\n')
        os.utime(config, ns=(info.st_atime_ns, info.st_mtime_ns))
        self.assertEqual(watch.check()['external_changes'], ['.codex/config.toml'])

    def test_new_top_level_entry_is_scanned_without_pattern_exceptions(self):
        folder = self.root / '.claude'
        folder.mkdir()
        watch = fake_private.OperatorWatch(self.root, markers=("own-marker",))
        temporary = folder / 'settings.json.tmp.123'
        temporary.write_text('fixture')
        self.assertEqual(watch.check()['external_changes'], ['.claude'])
        temporary.write_text('own-marker')
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_run_trace: .claude'):
            watch.check()
        temporary.unlink()
        self.assertEqual(watch.check()['changed'], [])

    @unittest.skipIf(os.name == 'nt', 'local fake mode and its symlink watch run on macOS')
    def test_only_a_link_retargeted_inside_run_stops(self):
        link = self.root / fake_private.LINK
        link.parent.mkdir(parents=True)
        link.symlink_to('first')
        info = link.lstat()
        watch = fake_private.OperatorWatch(self.root, run=self.root / 'run')
        link.unlink()
        link.symlink_to('second')
        os.utime(link, ns=(info.st_atime_ns, info.st_mtime_ns), follow_symlinks=False)
        self.assertEqual(watch.check()['external_changes'], ['.local/bin/claude'])
        link.unlink()
        link.symlink_to(self.root / 'run/program')
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_run_trace: .local/bin/claude'):
            watch.check()

    def test_external_agent_folder_appearing_is_reported(self):
        watch = fake_private.OperatorWatch(self.root)
        (self.root / '.grok').mkdir()
        self.assertEqual(watch.check()['external_changes'], ['.grok'])

    def test_claude_live_state_is_ignored_but_login_and_own_project_paths_are_watched(self):
        config = self.root / '.claude.json'
        run = self.root / 'private-run'
        data = {'oauthAccount': {'accountUuid': 'fixture'}, 'numStartups': 1, 'projects': {}}
        config.write_text(json.dumps(data))
        watch = fake_private.OperatorWatch(self.root, run=run)
        data['numStartups'] = 2
        data['projects'][str(self.root / 'other')] = {'hasTrustDialogAccepted': True}
        config.write_text(json.dumps(data))
        result = watch.check()
        self.assertEqual(result['changed'], [])
        self.assertNotIn(watch.before['.claude.json']['sha256'], json.dumps(result))
        self.assertNotIn(str(self.root / 'other'), json.dumps(result))
        data['oauthAccount']['accountUuid'] = 'changed'
        config.write_text(json.dumps(data))
        self.assertEqual(watch.check()['external_changes'], ['.claude.json'])
        data['oauthAccount']['accountUuid'] = 'fixture'
        data['projects'][str(run / 'probe/worktrees/pi')] = {}
        config.write_text(json.dumps(data))
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_run_trace: .claude.json'):
            watch.check()
        # Even a path already present at the start cannot pass the end guard.
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_run_trace: .claude.json'):
            fake_private.OperatorWatch(self.root, run=run).check()

    def test_half_written_claude_json_is_not_an_empty_configuration(self):
        (self.root / '.claude.json').write_text('{"oauthAccount":')
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_claude_configuration_unreadable'):
            fake_private.OperatorWatch(self.root)

    def test_a_missing_key_update_or_pin_proof_keeps_the_exception(self):
        row = {'fake_home': {'status': 'proven', 'key_env': 'KEY', 'evidence': 'vendor docs',
                            'auto_update': {'kind': 'environment', 'name': 'NO_UPDATE', 'value': '1'}}}
        pin = {'version': '1.0.0', 'installer': 'npm'}
        self.assertIsNone(fake_private.exception(row, pin))
        for missing in ('key_env', 'evidence', 'auto_update'):
            incomplete = {'fake_home': {key: value for key, value in row['fake_home'].items() if key != missing}}
            self.assertIsNotNone(fake_private.exception(incomplete, pin))
        self.assertIsNotNone(fake_private.exception(row, {**pin, 'pinned': 'installer takes no version'}))
        self.assertIsNotNone(fake_private.exception({}, pin))
        for missing in ('name', 'value'):
            update = {key: value for key, value in row['fake_home']['auto_update'].items() if key != missing}
            self.assertIsNotNone(fake_private.exception({'fake_home': {**row['fake_home'], 'auto_update': update}}, pin))

    @unittest.skipUnless(sys.platform == 'darwin', 'private local runs use the macOS sandbox')
    def test_private_sandbox_allows_the_probe_and_refuses_an_outside_write(self):
        from agent_live_check.processes import OwnedProcesses
        from agent_live_check.sandbox import WriteSandbox
        from agent_live_check.runtime import clean_env
        run, outside = self.root / 'run', self.root / 'outside'
        run.mkdir()
        outside.mkdir()
        skills = self.root / 'operator/.agents/skills'
        skills.mkdir(parents=True)
        private_skill = skills / 'private.txt'
        private_skill.write_text('operator-only skill')
        sandbox = WriteSandbox(run, self.root / 'sockets', self.root / 'operator',
                               checkout=self.root / 'checkout', fake_private=True)
        with OwnedProcesses() as owner:
            sandbox.verify_private(owner, clean_env(), outside)
            # Pi's ancestor existence probe must see no operator skills;
            # direct reads must also fail, even when their name is known.
            probe = ("import os,pathlib\n"
                     f"assert not os.path.exists({str(skills)!r})\n"
                     f"assert not os.access({str(skills)!r}, os.F_OK)\n"
                     f"try: pathlib.Path({str(private_skill)!r}).read_bytes()\n"
                     "except PermissionError: pass\n"
                     "else: raise AssertionError('operator skill readable')\n")
            owner.run(sandbox.command([sys.executable, '-c', probe]), env=clean_env())
        self.assertEqual((outside / 'forbidden').read_bytes(), b'unchanged')

    def test_private_environment_admits_only_process_basics(self):
        from unittest.mock import patch
        from agent_live_check.runtime import clean_env
        with patch.dict(os.environ, {'PATH': '/usr/bin', 'LANG': 'C',
                        'CLAUDE_CODE_OAUTH_TOKEN': 'secret', 'CLAUDE_CODE_USE_BEDROCK': '1',
                        'NPM_CONFIG_USERCONFIG': '/private/npmrc', 'OPENCODE_AUTH': 'secret'}, clear=True):
            self.assertEqual(clean_env(private=True), {'PATH': '/usr/bin', 'LANG': 'C'})

    def test_a_cache_refuses_the_first_byte_past_its_cap(self):
        payload = self.root / 'payload'
        payload.write_bytes(b'x' * 100)
        self.assertEqual(cli_cache.size(payload, 100), 100)
        with self.assertRaisesRegex(ProtectionError, 'cache_over_budget'):
            cli_cache.size(payload, 99)

    @unittest.skipIf(os.name == 'nt', 'local cache lease uses the macOS lock boundary')
    def test_existing_oversized_cache_is_refused_before_a_pin_is_installed(self):
        (self.root / 'payload').write_bytes(b'x' * 16385)
        with self.assertRaisesRegex(ProtectionError, 'cache_over_budget'):
            cli_cache.Cache(self.root, 16384)

    @unittest.skipIf(os.name == 'nt', 'local cache lease uses the macOS lock boundary')
    def test_cache_reuse_rejects_changed_executable_and_concurrent_writer(self):
        cache = cli_cache.Cache(self.root, 65536)
        self.addCleanup(cache.close)
        pin = {'version': '1.0.0', 'installer': 'npm'}
        folder = self.root / cli_cache.pin_key('example', pin)
        folder.mkdir()
        program = folder / 'program'
        program.write_bytes(b'original')
        import hashlib
        (folder / 'receipt.json').write_text(json.dumps({'pin': pin, 'program': 'program',
                                                       'sha256': hashlib.sha256(b'original').hexdigest()}))
        self.assertEqual(cache.program('example', pin), program)
        with self.assertRaisesRegex(ProtectionError, 'cache_busy'):
            cli_cache.Cache(self.root, 65536)
        program.write_bytes(b'replaced')
        with self.assertRaisesRegex(ProtectionError, 'cache_receipt_mismatch'):
            cache.program('example', pin)

    @unittest.skipIf(os.name == 'nt', 'local cache lease uses the macOS lock boundary')
    def test_unconfirmed_children_prevent_the_next_run_from_reusing_the_cache(self):
        cache = cli_cache.Cache(self.root, 65536)
        cache.close(confirmed=False)
        with self.assertRaisesRegex(ProtectionError, 'cache_cleanup_unconfirmed'):
            cli_cache.Cache(self.root, 65536)

    def test_provider_configs_use_environment_keys_and_apply_adapter_update_policy(self):
        contract = json.loads((Path(__file__).resolve().parents[2] / 'contracts/agent-journey.json').read_text())
        for row in contract:
            if row['fake_home']['status'] != 'proven':
                continue
            kind = row['herdr_kind']
            with self.subTest(kind=kind):
                home = self.root / kind
                env = fake_lane.configure(kind, home, 'http://127.0.0.1:12345', home / 'project', local=False)
                self.assertEqual(env[row['fake_home']['key_env']], fake_lane.KEY)
                update = row['fake_home']['auto_update']
                if update['kind'] == 'environment':
                    self.assertEqual(env[update['name']], update['value'])
                else:
                    content = (home / update['path']).read_text()
                    self.assertIn(update['key'], content)
                self.assertIsNone(fake_private.exception(row, clis.table()[kind]))
        self.assertEqual(json.loads((self.root / 'pi/.pi/agent/models.json').read_text())
                         ['providers']['journey']['apiKey'], '${JOURNEY_FAKE_KEY}')
        self.assertIn('env_key = "JOURNEY_FAKE_KEY"', (self.root / 'grok/.grok/config.toml').read_text())


class ApiKey(unittest.TestCase):
    def test_only_the_key_from_the_launch_environment_proves_the_route(self):
        import http.client
        server, _ = fake_model.serve(limit=4, api_key='private-fixture-key')
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        connection = http.client.HTTPConnection('127.0.0.1', server.server_address[1], timeout=5)
        self.addCleanup(connection.close)
        for header, status in (({}, 401), ({'Authorization': 'Bearer wrong'}, 401),
                               ({'Authorization': 'Bearer private-fixture-key'}, 200),
                               ({'x-api-key': 'private-fixture-key'}, 200)):
            connection.request('POST', '/v1/chat/completions', json.dumps({'messages': []}), header)
            answer = connection.getresponse()
            answer.read()
            self.assertEqual(answer.status, status)
        self.assertEqual(server.authenticated, 2)
        self.assertEqual(server.auth_failures, 2)


class OmpWatch(unittest.TestCase):
    def setUp(self):
        import sqlite3
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.path = self.root / ".omp/agent/agent.db"
        self.path.parent.mkdir(parents=True)
        self.connection = sqlite3.connect(self.path)
        self.addCleanup(self.connection.close)
        self.connection.executescript('''
            PRAGMA journal_mode=WAL;
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT, updated_at INTEGER);
            CREATE TABLE auth_credentials (id INTEGER PRIMARY KEY, provider TEXT, credential_type TEXT,
                data TEXT, disabled_cause TEXT, identity_key TEXT, created_at INTEGER, updated_at INTEGER);
            CREATE TABLE command_usage (name TEXT PRIMARY KEY, count INTEGER);
            INSERT INTO settings VALUES ('theme', 'dark', 1);
            INSERT INTO auth_credentials VALUES (1, 'fixture', 'api_key', 'fixture-secret', NULL, NULL, 1, 1);
            INSERT INTO command_usage VALUES ('help', 1);
        ''')
        self.scratch = self.root / 'private-copy'

    def test_wal_credentials_are_watched_but_usage_is_not_and_original_bytes_stay_untouched(self):
        originals = {path: (path.read_bytes(), path.stat().st_mtime_ns) for path in self.path.parent.iterdir()}
        watch = fake_private.OperatorWatch(self.root, self.scratch)
        self.assertEqual({path: (path.read_bytes(), path.stat().st_mtime_ns)
                          for path in self.path.parent.iterdir()}, originals)
        self.assertNotIn('fixture-secret', json.dumps(watch.before))
        evidence = json.dumps(watch.compare())
        self.assertNotIn(watch.before['.omp/agent/agent.db']['auth_credentials']['sha256'], evidence)
        self.assertNotIn(str(self.path), evidence)
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.connection.execute('UPDATE command_usage SET count = 2')
        self.connection.commit()
        self.assertEqual(watch.check()['changed'], [])
        self.connection.execute("UPDATE auth_credentials SET data = 'changed-secret'")
        self.connection.commit()
        self.assertEqual(watch.check()['external_changes'], ['.omp/agent/agent.db'])
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_settings_change_is_not_hidden_with_the_usage_rows(self):
        watch = fake_private.OperatorWatch(self.root, self.scratch)
        self.connection.execute("UPDATE settings SET value = 'light'")
        self.connection.commit()
        self.assertEqual(watch.check()['external_changes'], ['.omp/agent/agent.db'])

    def test_unreadable_schema_is_a_failure_not_missing_configuration(self):
        self.connection.execute('DROP TABLE auth_credentials')
        self.connection.commit()
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_database_snapshot_unreadable'):
            database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db")
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_run_marker_in_credential_row_fails_without_exposing_it(self):
        watch = fake_private.OperatorWatch(self.root, self.scratch, markers=('unique-fake-key',))
        self.connection.execute("UPDATE auth_credentials SET data = 'unique-fake-key'")
        self.connection.commit()
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_run_trace'):
            watch.check()
        self.assertNotIn('unique-fake-key', json.dumps(watch.compare()))
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_copy_retries_only_three_times_when_open_observes_new_wal_state(self):
        from unittest.mock import patch
        real_open = os.open
        for moving_opens, expected_opens in ((1, 2), (3, 3)):
            opened = []

            def open_during_writer(path, flags, *args, **kwargs):
                descriptor = real_open(path, flags, *args, **kwargs)
                if Path(path) == self.path:
                    opened.append(path)
                    if len(opened) <= moving_opens:
                        self.connection.execute('UPDATE settings SET updated_at = updated_at + 1')
                        self.connection.commit()
                return descriptor

            # The OS-open boundary schedules a real SQLite writer after the
            # metadata read; the copier, retries and queries all run normally.
            with patch('os.open', side_effect=open_during_writer):
                if moving_opens == 3:
                    with self.assertRaisesRegex(ProtectionError, 'database_copy_changed'):
                        database_watch.digest(self.root, self.scratch, '.omp/agent/agent.db')
                else:
                    database_watch.digest(self.root, self.scratch, '.omp/agent/agent.db')
            self.assertEqual(len(opened), expected_opens)
            self.assertEqual(list(self.scratch.iterdir()), [])

    def test_checkpoint_removing_wal_before_copy_retries_without_failing_the_watch(self):
        from unittest.mock import patch
        watch = fake_private.OperatorWatch(self.root, self.scratch)
        wal = Path(str(self.path) + '-wal')
        self.assertTrue(wal.exists())
        original = os.open

        def checkpoint(path, flags, *args, **kwargs):
            if Path(path) == wal:
                self.connection.execute('PRAGMA wal_checkpoint(TRUNCATE)')
                self.connection.close()
                # The open copy descriptor can defer SQLite's own unlink on
                # macOS; complete the fixture checkpoint's removal explicitly.
                wal.unlink(missing_ok=True)
            return original(path, flags, *args, **kwargs)

        with patch('os.open', side_effect=checkpoint):
            self.assertEqual(watch.check()['changed'], [])
        self.assertFalse(wal.exists())
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_copy_budget_is_enforced_before_reading_a_sparse_file(self):
        self.connection.close()
        with self.path.open('wb') as stream:
            stream.truncate(database_watch.MAX_DATABASE_BYTES + 1)
        with self.assertRaisesRegex(ProtectionError, 'credential_copy_total_over_budget'):
            database_watch.digest(self.root, self.scratch, '.omp/agent/agent.db')


class OpenCodeWatch(unittest.TestCase):
    def test_credentials_change_stops_but_sessions_do_not_and_the_original_is_untouched(self):
        import sqlite3
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            path = root / '.local/share/opencode/opencode.db'
            path.parent.mkdir(parents=True)
            connection = sqlite3.connect(path)
            try:
                connection.executescript('''
                    PRAGMA journal_mode=WAL;
                    CREATE TABLE credential (id TEXT PRIMARY KEY, integration_id TEXT, label TEXT,
                        value TEXT, connector_id TEXT, method_id TEXT, active INTEGER,
                        time_created INTEGER, time_updated INTEGER);
                    CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT);
                    INSERT INTO credential VALUES ('id', 'fixture', 'key', 'private-fixture', NULL, NULL, 1, 1, 1);
                    INSERT INTO session VALUES ('session', 'first');
                ''')
                before = {p: (p.read_bytes(), p.stat().st_mtime_ns) for p in path.parent.iterdir()}
                scratch = root / 'private-copy'
                watch = fake_private.OperatorWatch(root, scratch)
                self.assertEqual(before, {p: (p.read_bytes(), p.stat().st_mtime_ns) for p in path.parent.iterdir()})
                self.assertNotIn('private-fixture', json.dumps(watch.before))
                connection.execute("UPDATE session SET title = 'second'")
                connection.commit()
                self.assertEqual(watch.check()['changed'], [])
                connection.execute("UPDATE credential SET value = 'changed'")
                connection.commit()
                self.assertEqual(watch.check()['external_changes'], ['.local/share/opencode/opencode.db'])
                self.assertEqual(list(scratch.iterdir()), [])
                if sys.platform == 'darwin':
                    self.assertEqual({row['method'] for row in watch.copies}, {'apfs_clone'})
            finally:
                connection.close()


if __name__ == '__main__':
    unittest.main()
