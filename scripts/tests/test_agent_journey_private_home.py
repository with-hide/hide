"""Configuration and entry changes stop; live session appends do not.

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

    def test_configuration_bytes_change_stops_even_with_the_same_size_and_mtime(self):
        config = self.root / '.codex/config.toml'
        config.parent.mkdir()
        config.write_text('model="first"\n')
        info = config.stat()
        watch = fake_private.OperatorWatch(self.root)
        config.write_text('model="other"\n')
        os.utime(config, ns=(info.st_atime_ns, info.st_mtime_ns))
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: .codex/config.toml'):
            watch.check()
        self.assertEqual(watch.compare()['changed'], ['.codex/config.toml'])

    def test_a_new_top_level_name_stops_without_any_pattern_exception(self):
        folder = self.root / '.claude'
        folder.mkdir()
        watch = fake_private.OperatorWatch(self.root)
        temporary = folder / 'settings.json.tmp.123'
        temporary.write_text('fixture')
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: .claude'):
            watch.check()
        temporary.unlink()
        self.assertEqual(watch.check()['changed'], [])

    @unittest.skipIf(os.name == 'nt', 'local fake mode and its symlink watch run on macOS')
    def test_changed_cli_link_target_stops_with_the_same_mtime(self):
        link = self.root / fake_private.LINK
        link.parent.mkdir(parents=True)
        link.symlink_to('first')
        info = link.lstat()
        watch = fake_private.OperatorWatch(self.root)
        link.unlink()
        link.symlink_to('second')
        os.utime(link, ns=(info.st_atime_ns, info.st_mtime_ns), follow_symlinks=False)
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: .local/bin/claude'):
            watch.check()

    def test_missing_agent_folder_appearing_stops(self):
        watch = fake_private.OperatorWatch(self.root)
        (self.root / '.grok').mkdir()
        with self.assertRaises(ProtectionError):
            watch.check()

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
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: claude_login_install'):
            watch.check()
        data['oauthAccount']['accountUuid'] = 'fixture'
        data['projects'][str(run / 'probe/worktrees/pi')] = {}
        config.write_text(json.dumps(data))
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: claude_login_install'):
            watch.check()
        # Even a path already present at the start cannot pass the end guard.
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: claude_login_install'):
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
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: omp_login_settings'):
            watch.check()
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_settings_change_is_not_hidden_with_the_usage_rows(self):
        watch = fake_private.OperatorWatch(self.root, self.scratch)
        self.connection.execute("UPDATE settings SET value = 'light'")
        self.connection.commit()
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: omp_login_settings'):
            watch.check()

    def test_unreadable_schema_is_a_failure_not_missing_configuration(self):
        self.connection.execute('DROP TABLE auth_credentials')
        self.connection.commit()
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_database_snapshot_unreadable'):
            database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db")
        self.assertEqual(list(self.scratch.iterdir()), [])

    def test_a_concurrent_wal_write_refuses_the_copy(self):
        from unittest.mock import patch
        original = database_watch.read_file
        changed = False

        def concurrent(path, expected, destination=None):
            nonlocal changed
            value = original(path, expected, destination)
            if not changed:
                changed = True
                self.connection.execute("UPDATE settings SET value = 'changed-during-copy'")
                self.connection.commit()
            return value

        with patch.object(database_watch, 'read_file', concurrent):
            with self.assertRaisesRegex(ProtectionError, 'operator_watch_database_copy_changed'):
                database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db")

    def test_streaming_fallback_is_reported_and_a_copy_budget_failure_is_not_a_pass(self):
        from unittest.mock import patch
        copies = []
        with patch.object(database_watch, 'clone', return_value=False):
            database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db", copies)
        self.assertTrue(copies)
        self.assertEqual({row['method'] for row in copies}, {'chunked_copy'})
        self.assertEqual(list(self.scratch.iterdir()), [])
        with patch.object(database_watch, 'MAX_DATABASE_BYTES', 1):
            with self.assertRaisesRegex(ProtectionError, 'credential_copy_total_over_budget'):
                database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db")

    def test_space_failure_has_its_errno_and_removes_the_copy(self):
        import errno
        from unittest.mock import patch
        with patch.object(database_watch, 'clone', side_effect=OSError(errno.ENOSPC, 'fixture')):
            with self.assertRaisesRegex(ProtectionError, f'copy_failed: errno={errno.ENOSPC}'):
                database_watch.digest(self.root, self.scratch, ".omp/agent/agent.db")
        self.assertEqual(list(self.scratch.iterdir()), [])


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
                with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: opencode_login'):
                    watch.check()
                self.assertEqual(list(scratch.iterdir()), [])
                if sys.platform == 'darwin':
                    self.assertEqual({row['method'] for row in watch.copies}, {'apfs_clone'})
            finally:
                connection.close()


if __name__ == '__main__':
    unittest.main()
