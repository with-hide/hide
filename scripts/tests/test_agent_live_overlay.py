"""Existing login copies stay private, bounded, disposable and never write back."""

import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.overlay import prepare
from agent_live_check.credential_snapshot import CredentialSnapshotUnavailable
from agent_live_check.protection import MAX_BACKUP_BYTES, ProtectionError
from agent_live_check.processes import OwnedProcesses
from agent_live_check.runtime import Runtime


class PrivateConfiguration(unittest.TestCase):
    def test_unconfirmed_native_panes_keep_state_but_remove_disposable_credentials(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            runtime = Runtime.__new__(Runtime)
            runtime.probe, runtime.short, runtime.state = root / "probe", root / "short", root / "state"
            for folder in (runtime.probe, runtime.short, runtime.state):
                folder.mkdir(mode=0o700)
            credential = runtime.probe / "config-fixture"
            credential.mkdir(mode=0o700)
            (credential / "auth.json").write_bytes(b"private-fixture-token")
            runtime.credential_roots = {credential}
            runtime.owner = OwnedProcesses()
            runtime.workspaces, runtime.started, runtime.servers = set(), False, []
            def refused():
                raise RuntimeError("injected_holder_end_refused")
            runtime.stop_panes = refused
            result = runtime.close()
            self.assertFalse(result["confirmed"])
            self.assertFalse(result["processes_confirmed"])
            self.assertTrue(result["credential_copies_removed"])
            self.assertTrue(runtime.probe.exists())
            self.assertTrue(runtime.short.exists())

    def test_protocol_cleanup_failure_does_not_retain_credential_copies(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            runtime = Runtime.__new__(Runtime)
            runtime.probe, runtime.short, runtime.state = root / "probe", root / "short", root / "state"
            for folder in (runtime.probe, runtime.short, runtime.state):
                folder.mkdir(mode=0o700)
            credential = runtime.probe / "config-fixture"
            credential.mkdir(mode=0o700)
            (credential / "auth.json").write_bytes(b"private-fixture-token")
            runtime.credential_roots = {credential}
            runtime.owner = OwnedProcesses()
            runtime.workspaces, runtime.started, runtime.servers = {"own-workspace"}, False, []
            def refused(_):
                raise RuntimeError("injected_protocol_cleanup_failure")
            runtime.close_workspace = refused
            runtime.stop_panes = lambda: None
            result = runtime.close()
            self.assertFalse(result["confirmed"])
            self.assertTrue(result["credential_copies_removed"])
            self.assertFalse(credential.exists())
            self.assertTrue(result["processes_confirmed"])
            self.assertFalse(runtime.probe.exists())
            self.assertFalse(runtime.short.exists())

    def recipe(self, source="auth.json"):
        return {"id": "fixture", "overlay": {"env": {"FIXTURE_CONFIG_ROOT": "."},
                "copies": [[source, source]], "settings": ["settings.json"], "sessions": "sessions",
                "provenance": "fixture-only contract"}}

    def test_private_copy_preserves_source_and_reports_no_values(self):
        with tempfile.TemporaryDirectory() as name:
            operator, probe = Path(name) / "operator", Path(name) / "probe"
            operator.mkdir(mode=0o700)
            probe.mkdir(mode=0o700)
            source = operator / "auth.json"
            content = b'{"test-only-credential":"sensitive-fixture"}'
            source.write_bytes(content)
            result = prepare(self.recipe(), probe, operator)
            copied = probe / "config-fixture/auth.json"
            self.assertEqual(copied.read_bytes(), content)
            self.assertEqual(copied.stat().st_mode & 0o777, 0o600)
            copied.write_bytes(b"refreshed-in-private-copy")
            self.assertEqual(source.read_bytes(), content)
            self.assertNotIn("HOME", result["env"])
            self.assertNotIn("sensitive-fixture", json.dumps(result["copies"]))

    def test_alias_and_oversized_copy_are_refused(self):
        for kind in ("alias", "oversized"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as name:
                operator, probe = Path(name) / "operator", Path(name) / "probe"
                operator.mkdir(mode=0o700)
                probe.mkdir(mode=0o700)
                source = operator / "auth.json"
                if kind == "alias":
                    target = operator / "other.json"
                    target.write_bytes(b"other")
                    source.symlink_to(target)
                else:
                    source.write_bytes(b"fixture")
                    with source.open("r+b") as stream:
                        stream.truncate(MAX_BACKUP_BYTES + 1)
                with self.assertRaises(ProtectionError):
                    prepare(self.recipe(source.name), probe, operator)
                self.assertFalse((probe / "config-fixture" / source.name).exists())

    def wal_fixture(self, root):
        """An agreed SQLite row committed only to WAL; no opener of the copy."""
        seed = root / "seed.db"
        operator, probe = root / "operator", root / "probe"
        operator.mkdir(mode=0o700)
        probe.mkdir(mode=0o700)
        connection = sqlite3.connect(seed)
        try:
            connection.execute("PRAGMA journal_mode=WAL")
            connection.execute("CREATE TABLE credentials (value TEXT)")
            connection.execute("INSERT INTO credentials VALUES ('fixture-credential')")
            connection.commit()
            source = operator / "auth.db"
            for suffix in ("", "-wal", "-shm"):
                shutil.copyfile(Path(str(seed) + suffix), Path(str(source) + suffix))
        finally:
            connection.close()
        return operator, probe, source

    @unittest.skipUnless(Path("/usr/sbin/lsof").is_file(), "native snapshot needs the macOS lsof")
    def test_quiescent_wal_bundle_recovers_only_in_private_copy(self):
        with tempfile.TemporaryDirectory() as name, OwnedProcesses() as owner:
            operator, probe, source = self.wal_fixture(Path(name))
            paths = [Path(str(source) + suffix) for suffix in ("", "-wal", "-shm")]
            before = [(p.read_bytes(), p.stat().st_mtime_ns, p.stat().st_ino) for p in paths]
            result = prepare(self.recipe("auth.db"), probe, operator, owner=owner)
            self.assertEqual({row["private_name"] for row in result["copies"]},
                             {"auth.db", "auth.db-wal", "auth.db-shm"})
            private = probe / "config-fixture/auth.db"
            connection = sqlite3.connect(private)
            try:
                self.assertEqual(connection.execute("SELECT value FROM credentials").fetchall(),
                                 [("fixture-credential",)])
                connection.execute("UPDATE credentials SET value = 'private-refresh'")
                connection.commit()
            finally:
                connection.close()
            self.assertEqual([(p.read_bytes(), p.stat().st_mtime_ns, p.stat().st_ino) for p in paths], before)
            self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in private.parent.iterdir()))

    @unittest.skipUnless(Path("/usr/sbin/lsof").is_file(), "native snapshot needs the macOS lsof")
    def test_open_database_is_unavailable_without_a_private_copy(self):
        with tempfile.TemporaryDirectory() as name, OwnedProcesses() as owner:
            operator, probe, source = self.wal_fixture(Path(name))
            connection = sqlite3.connect(source)
            try:
                connection.execute("SELECT value FROM credentials").fetchall()
                with self.assertRaises(CredentialSnapshotUnavailable):
                    prepare(self.recipe("auth.db"), probe, operator, owner=owner)
                self.assertEqual(list((probe / "config-fixture").iterdir()), [])
            finally:
                connection.close()

    @unittest.skipUnless(Path("/usr/sbin/lsof").is_file(), "native snapshot needs the macOS lsof")
    def test_wal_metadata_change_during_read_discards_the_whole_bundle(self):
        with tempfile.TemporaryDirectory() as name, OwnedProcesses() as owner:
            operator, probe, source = self.wal_fixture(Path(name))
            wal = Path(str(source) + "-wal")
            original_open, changed = os.open, []

            def open_with_external_change(path, flags, *args, **kwargs):
                descriptor = original_open(path, flags, *args, **kwargs)
                if Path(path) == source and not changed:
                    info = wal.stat()
                    os.utime(wal, ns=(info.st_atime_ns, info.st_mtime_ns + 1_000_000))
                    changed.append(True)
                return descriptor

            # Only the external filesystem boundary is substituted. The
            # changed metadata, snapshot, refusal and copies are all real.
            with patch("os.open", side_effect=open_with_external_change):
                with self.assertRaises(CredentialSnapshotUnavailable):
                    prepare(self.recipe("auth.db"), probe, operator, owner=owner)
            self.assertEqual(changed, [True])
            self.assertEqual(list((probe / "config-fixture").iterdir()), [])

    def test_database_sidecar_alias_and_combined_budget_stay_fatal(self):
        for kind in ("alias", "budget"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as name:
                operator, probe, source = self.wal_fixture(Path(name))
                wal = Path(str(source) + "-wal")
                if kind == "alias":
                    wal.unlink()
                    wal.symlink_to(source)
                else:
                    with wal.open("r+b") as stream:
                        stream.truncate(MAX_BACKUP_BYTES)
                with self.assertRaises(ProtectionError):
                    prepare(self.recipe("auth.db"), probe, operator)
                self.assertEqual(list((probe / "config-fixture").iterdir()), [])

    @unittest.skipUnless(Path("/usr/sbin/lsof").is_file(), "native snapshot needs the macOS lsof")
    def test_database_removed_during_open_is_unavailable_without_a_copy(self):
        with tempfile.TemporaryDirectory() as name, OwnedProcesses() as owner:
            operator, probe, source = self.wal_fixture(Path(name))
            original_open, removed = os.open, []

            def open_after_external_removal(path, flags, *args, **kwargs):
                if Path(path) == source and not removed:
                    source.unlink()
                    removed.append(True)
                return original_open(path, flags, *args, **kwargs)

            with patch("os.open", side_effect=open_after_external_removal):
                with self.assertRaises(CredentialSnapshotUnavailable):
                    prepare(self.recipe("auth.db"), probe, operator, owner=owner)
            self.assertEqual(removed, [True])
            self.assertEqual(list((probe / "config-fixture").iterdir()), [])


if __name__ == "__main__":
    unittest.main()
