"""The operator watch tolerates live files and matches only this run's paths."""

import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey.fake_private import OperatorWatch
from agent_live_check.protection import ProtectionError


class WatchRaces(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.run = self.root / 'agent-journey-1'
        self.folder = self.root / '.codex'
        self.folder.mkdir()

    def test_run_aliases_and_escaping_are_detected_after_start_without_matching_siblings(self):
        config = self.folder / 'config.toml'
        config.write_text('external')
        watch = OperatorWatch(self.root, run=self.run,
                              markers=('http://127.0.0.1:43210',))
        for content in (str(self.run) + '0', str(self.run) + '-other',
                        '~/agent-journey-10', 'http://localhost:432100'):
            with self.subTest(unrelated=content):
                config.write_text(content)
                self.assertEqual(watch.check()['changed'], [])
        spellings = {str(self.run), '~/agent-journey-1', 'http://localhost:43210'}
        for a, b in (('/private/tmp/', '/tmp/'), ('/private/var/', '/var/')):
            for prefix, alias in ((a, b), (b, a)):
                if str(self.run).startswith(prefix):
                    spellings.add(alias + str(self.run)[len(prefix):])
        for value in spellings:
            for content in (value, value + '/child', json.dumps(value),
                            json.dumps(value.replace('/', r'\/'))):
                with self.subTest(trace=content):
                    config.write_text(content)
                    self.assertEqual(watch.compare()['changed'], ['.codex/config.toml'])

    def test_tmp_aliases_and_unicode_json_paths_are_run_traces(self):
        config = self.folder / 'config.toml'
        for run, spellings in (
                (Path('/tmp/agent-journey-1'), ('/tmp/agent-journey-1', '/private/tmp/agent-journey-1')),
                (Path('/private/tmp/agent-journey-1'), ('/tmp/agent-journey-1', '/private/tmp/agent-journey-1')),
                (self.root / '한 글', (str(self.root / '한 글'), '~/한 글'))):
            config.write_text('external')
            watch = OperatorWatch(self.root, run=run)
            for value in spellings:
                for content in (value, json.dumps(value), value.replace('/', r'\/'),
                                json.dumps(value.replace('/', r'\/'))):
                    with self.subTest(run=run, trace=content):
                        config.write_text(content)
                        self.assertEqual(watch.compare()['changed'], ['.codex/config.toml'])

    def test_new_entry_stream_waits_for_the_path_boundary_across_chunks(self):
        watch = OperatorWatch(self.root, run=self.run)
        entry = self.folder / 'new-session'
        path = str(self.run).encode()
        prefix = b'x' * (1024 ** 2 - len(path))
        entry.write_bytes(prefix + path + b'0/other-run')
        self.assertEqual(watch.check()['changed'], [])
        entry.write_bytes(prefix + path + b'/our-run')
        self.assertEqual(watch.compare()['changed'], ['.codex'])

    @unittest.skipIf(os.name == 'nt', 'Unix live directory entries')
    def test_disappearing_wal_and_special_entries_are_not_run_traces(self):
        watch = OperatorWatch(self.root, run=self.run)
        wal = self.folder / 'checkpoint.sqlite-wal'
        wal.write_bytes(b'concurrent session')
        fifo = self.folder / 'live.fifo'
        os.mkfifo(fifo)
        peer = socket.socket(socket.AF_UNIX)
        self.addCleanup(peer.close)
        peer.bind(str(self.folder / 'live.sock'))
        original = Path.lstat

        def checkpoint(path, *args, **kwargs):
            if path == wal:
                wal.unlink(missing_ok=True)
            return original(path, *args, **kwargs)

        # SQLite checkpoints after enumeration, before the watch stats the WAL.
        with patch.object(Path, 'lstat', checkpoint):
            self.assertEqual(watch.check()['changed'], [])
        self.assertFalse(wal.exists())

    def test_entry_disappearing_at_scandir_or_open_is_not_an_unreadable_watch(self):
        for boundary in ('scandir', 'open'):
            with self.subTest(boundary=boundary):
                watch = OperatorWatch(self.root, run=self.run)
                entry = self.folder / 'checkpoint.sqlite-wal'
                if boundary == 'scandir':
                    entry.mkdir()
                else:
                    entry.write_bytes(b'concurrent session')
                original = getattr(os, boundary)

                def checkpoint(path, *args, **kwargs):
                    if Path(path) == entry:
                        entry.rmdir() if boundary == 'scandir' else entry.unlink()
                    return original(path, *args, **kwargs)

                with patch.object(os, boundary, checkpoint):
                    self.assertEqual(watch.check()['changed'], [])
                self.assertFalse(entry.exists())

    def test_configuration_rewrites_retry_then_fail_after_three_moving_reads(self):
        for name in ('.codex/config.toml', '.claude.json'):
            config = self.root / name
            config.write_text('{}')
            watch = OperatorWatch(self.root, run=self.run)
            for movements in (1, 3):
                with self.subTest(name=name, movements=movements):
                    identity = (config.stat().st_dev, config.stat().st_ino)
                    original, reads, writes = os.fstat, [], []

                    def rewrite(fd):
                        info = original(fd)
                        if (info.st_dev, info.st_ino) == identity:
                            reads.append(fd)
                            if len(reads) % 2 == 0 and len(writes) < movements:
                                writes.append(len(writes))
                                config.write_text(json.dumps({'numStartups': len(writes)}))
                                return original(fd)
                        return info

                    # Real bytes change between the descriptor's two observations.
                    with patch('os.fstat', side_effect=rewrite):
                        if movements == 3:
                            reason = ('operator_watch_claude_configuration_unreadable'
                                      if name == '.claude.json' else 'config_changed_during_read')
                            with self.assertRaisesRegex(ProtectionError, reason):
                                watch.check()
                        else:
                            self.assertEqual(watch.check()['changed'], [])
                    self.assertEqual(len(writes), movements)


if __name__ == '__main__':
    unittest.main()
