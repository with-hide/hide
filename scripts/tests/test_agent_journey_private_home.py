"""The approved fake-only boundary: metadata changes stop, unproven routes do not launch.

Every filesystem case owns a fake operator root. None reads Path.home().
"""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cli_cache, clis, fake_lane, fake_private, fake_model
from agent_live_check.protection import ProtectionError


class PrivateHome(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def test_directory_mtime_change_stops_even_when_its_contents_are_the_same(self):
        folder = self.root / '.pi'
        folder.mkdir()
        watch = fake_private.OperatorWatch(self.root)
        self.assertEqual(watch.check()['changed'], [])
        info = folder.stat()
        os.utime(folder, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
        with self.assertRaisesRegex(ProtectionError, 'operator_watch_changed: .pi'):
            watch.check()
        self.assertEqual(watch.compare()['changed'], ['.pi'])

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

    def test_a_missing_key_update_or_pin_proof_keeps_the_exception(self):
        row = {'fake_home': {'status': 'proven', 'key_env': 'KEY', 'evidence': 'vendor docs',
                            'auto_update': {'kind': 'environment'}}}
        pin = {'version': '1.0.0', 'installer': 'npm'}
        self.assertIsNone(fake_private.exception(row, pin))
        for missing in ('key_env', 'evidence', 'auto_update'):
            incomplete = {'fake_home': {key: value for key, value in row['fake_home'].items() if key != missing}}
            self.assertIsNotNone(fake_private.exception(incomplete, pin))
        self.assertIsNotNone(fake_private.exception(row, {**pin, 'pinned': 'installer takes no version'}))
        self.assertIsNotNone(fake_private.exception({}, pin))

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


if __name__ == '__main__':
    unittest.main()
