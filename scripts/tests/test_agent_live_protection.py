"""PRD B3/B6/B8: refuse routing, preserve concurrent bytes, end real children.

No provider, Herdr, or hided build is needed for these guard contracts. The
real-pinned-server lane owns the measurement tool's protocol acceptance.
"""

import json
from contextlib import contextmanager
import ctypes
import errno
import io
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.process_table import (BsdInfo, Process, ProcessTable, descendants, marked_descendants,
                                           procargs_owned, require_complete, snapshot, validate_linux_procfs)
from agent_live_check.processes import (OwnedProcesses, ProcessError, ProcessSafetyError, RssSamples, guard,
                                       control_plane, largest, linux_children_remain, signal_reserved_group)
from agent_live_check.protection import ConfigGuard, ProtectionError, stamp, validate_isolation
from agent_live_check.sandbox import WriteSandbox
from agent_live_check.runtime import Runtime
from agent_live_check.report import save
from agent_live_check.cli import operator_routing, record_process_diagnostics, wrapper


def procargs(*environment, argv=(b"fixture",), pointer_width=8):
    path = b"/fixture\0"
    return (len(argv).to_bytes(4, sys.byteorder, signed=True) + path
            + b"\0" * (-len(path) % pointer_width)
            + b"\0".join(argv) + b"\0" + b"\0".join(environment) + b"\0")


class ConfigurationProtection(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="aclp-", dir="/tmp")
        self.root = Path(self.temporary.name).resolve()
        self.run = self.root / "run"
        self.run.mkdir(mode=0o700)
        self.home = self.run / "fixture-home"
        self.home.mkdir(mode=0o700)
        self.config = self.home / ".config.json"
        self.config.write_bytes(b'{"secret":"fixture-only","model":"original"}\n')
        self.config.chmod(0o640)

    def tearDown(self):
        self.temporary.cleanup()

    def guard(self):
        return ConfigGuard(self.run / "backup", [self.config], [self.home],
                           exclusive_root=self.home)

    def test_operator_change_never_grants_recovery_ownership(self):
        guard = ConfigGuard(self.run / "backup", [self.config], [self.home])
        before = stamp(self.config)
        self.config.write_bytes(b"changed-concurrently")
        with self.assertRaisesRegex(ProtectionError, "no_exclusive_writer"):
            guard.record_write(self.config, before, stamp(self.config))
        self.assertTrue(guard.finish()["failures"])
        self.assertEqual(self.config.read_bytes(), b"changed-concurrently")

    def test_private_backup_and_owned_recovery_are_byte_exact(self):
        before = self.config.read_bytes()
        guard = self.guard()
        original = stamp(self.config)
        self.config.write_bytes(b'{"model":"temporary"}')
        guard.record_write(self.config, original, stamp(self.config))
        result = guard.finish()
        self.assertEqual(self.config.read_bytes(), before)
        self.assertEqual(self.config.stat().st_mode & 0o777, 0o640)
        self.assertEqual(result["restored"], [{"path": str(self.config), "result": "restored"}])
        self.assertEqual(result["failures"], [])
        self.assertNotIn("fixture-only", json.dumps(result))
        for path in (self.run / "backup").iterdir():
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_concurrent_change_is_preserved_even_after_owned_write(self):
        guard = self.guard()
        original = stamp(self.config)
        self.config.write_bytes(b"owned")
        guard.record_write(self.config, original, stamp(self.config))
        replacement = self.config.with_suffix(".replacement")
        replacement.write_bytes(b"operator-concurrent")
        replacement.replace(self.config)
        result = guard.finish()
        self.assertEqual(self.config.read_bytes(), b"operator-concurrent")
        self.assertEqual(len(result["failures"]), 1)
        self.assertEqual(result["restored"], [])

    def test_unattributed_change_and_new_file_are_reported_without_overwrite(self):
        guard = self.guard()
        self.config.write_bytes(b"unattributed")
        (self.home / "unknown-settings.json").write_bytes(b"another-secret")
        result = guard.finish()
        self.assertEqual(self.config.read_bytes(), b"unattributed")
        self.assertTrue(result["failures"])
        self.assertIn({"path": str(self.home / "unknown-settings.json"), "kind": "added"}, result["directory_changes"])
        self.assertNotIn("another-secret", json.dumps(result))

    def test_alias_cannot_be_a_recovery_target(self):
        self.config.unlink()
        self.config.symlink_to(self.home / "other")
        with self.assertRaises(ProtectionError) as caught:
            self.guard()
        self.assertEqual(caught.exception.path, str(self.config))

    def test_shared_files_preserve_concurrent_bytes_and_report_private_keys_only(self):
        # Lead2702 withdraws shared-file recovery, including lingering entries.
        # Both external formats must preserve every final byte and inode.
        owned = str(self.run / "probe" / "changed")
        added = str(self.run / "probe" / "new")
        foreign = str(self.root / "other-session")
        sibling = str(self.run) + "-peer"
        escaped = str(self.run / ".." / "other-project")
        for format in ("json", "toml"):
            with self.subTest(format=format):
                shared = self.home / ("shared." + format)
                def content(entries):
                    if format == "json":
                        return (json.dumps({"foreign_secret": "never-report-this", "projects": entries}, indent=2) + "\n").encode()
                    return ('foreign_secret = "never-report-this"\n' + "".join(
                        f"[projects.{json.dumps(key)}]\ntrust_level = {json.dumps(value)}\n"
                        for key, value in entries.items())).encode()
                shared.write_bytes(content({owned: "before", foreign: "original"}))
                guard = ConfigGuard(self.run / ("backup-" + format), [shared, self.config], [self.home],
                                    shared={shared: format})
                final = content({owned: "changed", added: "left-behind", foreign: "concurrent",
                                 sibling: "not-ours", escaped: "not-ours"})
                shared.write_bytes(final)
                shared.chmod(0o400)
                inode = shared.stat().st_ino
                result = guard.finish()
                self.assertEqual(result["failures"], [])
                self.assertEqual(result["restored"], [])
                self.assertEqual(shared.read_bytes(), final)
                self.assertEqual((shared.stat().st_ino, shared.stat().st_mode & 0o777), (inode, 0o400))
                self.assertEqual(result["shared_changes"], [{"path": str(shared), "result": "다른 세션의 변경"}])
                self.assertCountEqual(result["shared_leftovers"], [
                    {"path": str(shared), "key": owned, "change": "changed"},
                    {"path": str(shared), "key": added, "change": "added"}])
                self.assertNotIn("never-report-this", json.dumps(result))
                self.assertNotIn("concurrent", json.dumps(result))

    def test_shared_absence_creation_and_unchanged_leftovers_never_request_recovery(self):
        shared = self.home / "shared.json"
        guard = ConfigGuard(self.run / "backup-absent", [shared], [self.home], shared={shared: "json"})
        key = str(self.run)
        content = json.dumps({"projects": {key: {"trust": "fixture"}}}).encode()
        shared.write_bytes(content)
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["shared_leftovers"], [{"path": str(shared), "key": key, "change": "added"}])
        guard = ConfigGuard(self.run / "backup-unchanged", [shared], [self.home], shared={shared: "json"})
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["shared_changes"], [])
        self.assertEqual(result["shared_leftovers"], [{"path": str(shared), "key": key, "change": "unchanged"}])
        self.assertEqual(shared.read_bytes(), content)

    def test_unreadable_shared_table_names_file_before_launch(self):
        for format, content in (("json", b'{"projects": []}'), ("toml", b'[projects.')):
            with self.subTest(format=format):
                shared = self.home / ("broken." + format)
                shared.write_bytes(content)
                with self.assertRaises(ProtectionError) as caught:
                    ConfigGuard(self.run / ("backup-broken-" + format), [shared], [self.home], shared={shared: format})
                self.assertEqual(caught.exception.path, str(shared))
                self.assertEqual(shared.read_bytes(), content)

    def test_shared_atomic_rewrites_are_unavailable_not_failure_or_absence(self):
        # Deterministic syscall boundaries force replacements before open,
        # after open and after read, including detached descriptors (nlink=0).
        cases = [(phase, boundary) for phase in ("before", "after")
                 for boundary in ("open", "opened", "read")]
        for phase, boundary in cases:
            with self.subTest(phase=phase, boundary=boundary):
                label = phase + "-" + boundary
                shared = self.home / ("rewrite-" + label + ".json")
                key = str(self.run / "probe" / label)
                shared.write_text(json.dumps({"projects": {key: {"secret": "private-read-token"}}}))
                arguments = (self.run / ("backup-rewrite-" + label), [shared], [self.home])
                if phase == "after":
                    guard = ConfigGuard(*arguments, shared={shared: "json"})
                operation = "open" if boundary == "open" else "fstat"
                original, replacements = getattr(os, operation), []
                observations = 0
                def replace_during_observation(subject, *args, **kwargs):
                    nonlocal observations
                    observations += 1
                    replace = (Path(subject) == shared if boundary == "open" else
                               boundary == "opened" or observations % 2 == 0)
                    if replace:
                        data = json.dumps({"sequence": len(replacements) + 1,
                                           "projects": {key: {"secret": "private-read-token"}}}).encode()
                        replacement = shared.with_suffix(".replacement")
                        replacement.write_bytes(data)
                        replacement.replace(shared)
                        replacements.append(data)
                    return original(subject, *args, **kwargs)
                with patch.object(os, operation, replace_during_observation):
                    if phase == "before":
                        guard = ConfigGuard(*arguments, shared={shared: "json"})
                    else:
                        result = guard.finish()
                if phase == "before":
                    result = guard.finish()
                self.assertEqual(len(replacements), 3)
                self.assertEqual(shared.read_bytes(), replacements[-1])
                self.assertEqual(result["failures"], [])
                comparison = result["shared_comparisons"][0]
                self.assertFalse(comparison["complete"])
                self.assertEqual(comparison[phase], "unavailable")
                self.assertNotIn("absent", comparison.values())
                self.assertEqual(result["shared_leftovers"], [] if phase == "after" else
                                 [{"path": str(shared), "key": key, "change": "not_compared"}])
                self.assertNotIn("private-read-token", json.dumps(result))
                if phase == "before":
                    index = json.loads((arguments[0] / "index.json").read_text())[0]
                    self.assertIsNone(index["existed"])
                    self.assertEqual(index["observation"], "unavailable")

    def test_shared_hardlink_created_during_read_remains_a_named_refusal(self):
        shared = self.home / "aliased-shared.json"
        shared.write_bytes(b'{"projects": {}}')
        original, observations = os.fstat, 0
        def link_during_read(descriptor):
            nonlocal observations
            observations += 1
            if observations == 2:
                os.link(shared, self.home / "concurrent-alias")
            return original(descriptor)
        with patch.object(os, "fstat", link_during_read):
            with self.assertRaises(ProtectionError) as caught:
                ConfigGuard(self.run / "backup-aliased-shared", [shared], [self.home], shared={shared: "json"})
        self.assertEqual(str(caught.exception), "config_not_private_regular_file")
        self.assertEqual(caught.exception.path, str(shared))
        self.assertEqual(shared.read_bytes(), b'{"projects": {}}')

    def test_unknown_configuration_is_metadata_only_and_preserved(self):
        # Lead letter 2686: only adapter-listed files have a byte guard.
        unknown = self.home / "unknown-settings.json"
        unknown.write_bytes(b"private-before")
        fifo = self.home / "unknown-pipe"
        os.mkfifo(fifo, 0o600)
        link = self.home / "unknown-link"
        link.symlink_to(self.root / "missing-target")
        real_open, real_readlink = os.open, os.readlink
        def refuse_file_read(path, *args, **kwargs):
            if Path(path) == unknown:
                raise PermissionError("unknown_file_contents_are_not_available")
            return real_open(path, *args, **kwargs)
        def refuse_link_read(path, *args, **kwargs):
            if Path(path) == link:
                raise PermissionError("unknown_link_target_is_not_available")
            return real_readlink(path, *args, **kwargs)
        with patch.object(os, "open", refuse_file_read), patch.object(os, "readlink", refuse_link_read):
            guard = self.guard()
            unknown.write_bytes(b"private-after-longer")
            result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertTrue(result["inventory_checked"])
        self.assertIn({"path": str(unknown), "kind": "changed"}, result["directory_changes"])
        self.assertEqual(unknown.read_bytes(), b"private-after-longer")
        self.assertNotIn("private-after", json.dumps(result))

    def test_installation_subtrees_are_excluded_but_plugin_registry_is_reported(self):
        directories = ["node_modules", "model-cache", "extensions", "marketplace", "plugins/installed-code"]
        for name in directories:
            directory = self.home / name
            directory.mkdir(parents=True, exist_ok=True)
            os.mkfifo(directory / "unreadable-code", 0o600)
        registry = self.home / "plugins" / "installed_plugins.json"
        registry.write_bytes(b"before")
        guard = self.guard()
        for name in directories:
            (self.home / name / "new-code").write_bytes(b"unreported")
        registry.write_bytes(b"after-longer")
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["directory_changes"], [{"path": str(registry), "kind": "changed"}])
        self.assertEqual(result["inventory"]["after"]["excluded_boundaries"], 5)

    def test_inventory_entry_cap_reports_partial_counts_without_failing_byte_guard(self):
        # The OS directory stream supplies the unchanged 50,000-entry edge.
        # Known-file reads, backup and recovery still use the real fixture.
        # Avoid thousands of writes for a directory-enumeration boundary.
        metadata = self.config.stat()

        @contextmanager
        def large_directory(descriptor):
            actual = os.fstat(descriptor)
            expected = self.home.stat()
            self.assertEqual((actual.st_dev, actual.st_ino),
                             (expected.st_dev, expected.st_ino))
            def entries():
                yield SimpleNamespace(name=self.config.name, stat=lambda **unused: metadata)
                for index in range(50_000):
                    yield SimpleNamespace(name=f"entry-{index}", stat=lambda **unused: metadata)
            yield entries()

        with patch.object(os, "scandir", large_directory):
            guard = self.guard()
            result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertTrue(result["inventory_checked"])
        for phase in ("before", "after"):
            inventory = result["inventory"][phase]
            self.assertFalse(inventory["complete"])
            self.assertEqual(inventory["scanned_entries"], 50_000)
            self.assertGreaterEqual(inventory["omitted_entries_lower_bound"], 1)
            self.assertGreaterEqual(inventory["uninspected_subtrees"], 1)
        self.assertEqual(result["directory_changes"], [])

    def test_excluded_directory_file_transitions_are_changes_not_false_absence(self):
        installed = self.home / "plugins" / "installed-code"
        installed.mkdir(parents=True)
        guard = self.guard()
        installed.rmdir()
        installed.write_bytes(b"now-an-ordinary-file")
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertIn({"path": str(installed), "kind": "changed"}, result["directory_changes"])
        guard = ConfigGuard(self.run / "backup-reverse", [self.config], [self.home])
        installed.unlink()
        installed.mkdir()
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertIn({"path": str(installed), "kind": "changed"}, result["directory_changes"])

    def test_explicit_nested_installation_root_cannot_bypass_exclusion(self):
        nested = self.home / "node_modules" / "package" / "data"
        nested.mkdir(parents=True)
        code = nested / "code"
        code.write_bytes(b"before")
        guard = ConfigGuard(self.run / "backup", [self.config], [nested, code, self.home])
        code.write_bytes(b"after-longer")
        (nested / "new-code").write_bytes(b"unreported")
        result = guard.finish()
        self.assertEqual(result["failures"], [])
        self.assertEqual(result["directory_changes"], [])
        self.assertEqual(result["inventory"]["after"]["excluded_boundaries"], 3)

    def test_invalid_known_file_keeps_named_failure_and_independent_recovery(self):
        peer = self.home / "peer.json"
        peer.write_bytes(b"original-peer")
        guard = ConfigGuard(self.run / "backup", [self.config, peer], [self.home],
                            exclusive_root=self.home)
        before = stamp(peer)
        peer.write_bytes(b"owned-peer")
        guard.record_write(peer, before, stamp(peer))
        # Letter 2686 keeps the known-file byte failure independent of the
        # metadata inventory, which can still report the unexpected link.
        os.link(self.config, self.home / "unexpected-link")
        result = guard.finish()
        self.assertEqual(peer.read_bytes(), b"original-peer")
        self.assertIn({"path": str(peer), "result": "restored"}, result["restored"])
        self.assertIn({"path": str(self.config), "reason": "config_not_private_regular_file"}, result["failures"])
        self.assertTrue(result["inventory_checked"])
        self.assertIn({"path": str(self.home / "unexpected-link"), "kind": "added"}, result["directory_changes"])
        self.assertFalse(any(row["reason"] == "configuration_inventory_unavailable" for row in result["failures"]))

    def test_fifo_replacement_during_open_refuses_without_blocking(self):
        program = f"""
import os,sys
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,{str(Path(__file__).resolve().parents[1])!r})
from agent_live_check.protection import configuration_bytes,ProtectionError
target=Path({str(self.config)!r})
original=os.open
def replaced(path,*args,**kwargs):
 if Path(path)==target:
  target.unlink()
  os.mkfifo(target,0o600)
 return original(path,*args,**kwargs)
with patch.object(os,'open',replaced):
 try: configuration_bytes(target)
 except ProtectionError: raise SystemExit(0)
raise SystemExit('FIFO was accepted as configuration')
"""
        with OwnedProcesses() as owner:
            code, _, _ = owner.run([sys.executable, "-c", program], env=dict(os.environ), seconds=2)
        self.assertEqual(code, 0)

    def test_directory_cleanup_failure_still_removes_peer_and_restores_owner_limits(self):
        runtime = Runtime.__new__(Runtime)
        runtime.owner = OwnedProcesses()
        runtime.owner.cancelled.set()
        deadline = runtime.owner.deadline
        runtime.state = self.run / "state"
        runtime.probe, runtime.short = self.run / "probe", self.run / "short"
        runtime.probe.mkdir()
        runtime.short.mkdir()
        runtime.workspaces, runtime.credential_roots = set(), set()
        runtime.servers, runtime.started = [], False
        import shutil
        original = shutil.rmtree
        def remove(path, *args, **kwargs):
            if path == runtime.probe:
                raise PermissionError("injected_probe_removal_refused")
            return original(path, *args, **kwargs)
        with patch.object(shutil, "rmtree", remove):
            result = runtime.close()
        self.assertFalse(result["confirmed"])
        self.assertIn("injected_probe_removal_refused", result["failures"])
        self.assertTrue(runtime.probe.exists())
        self.assertFalse(runtime.short.exists())
        self.assertTrue(runtime.owner.cancelled.is_set())
        self.assertEqual(runtime.owner.deadline, deadline)

    def test_prior_missing_guardian_receipt_retains_runtime_folders(self):
        runtime = Runtime.__new__(Runtime)
        runtime.owner = OwnedProcesses(diagnostics=self.run / "guardians")
        runtime.state = self.run / "state"
        runtime.probe, runtime.short = self.run / "probe", self.run / "short"
        runtime.probe.mkdir()
        runtime.short.mkdir()
        runtime.workspaces, runtime.credential_roots = set(), set()
        runtime.servers, runtime.started = [], False
        runtime.owner.run([sys.executable, "-c", "pass"], env=dict(os.environ), seconds=3)
        (self.run / "guardians/1.json").unlink()
        result = runtime.close()
        self.assertFalse(result["confirmed"])
        self.assertFalse(result["processes_confirmed"])
        self.assertIn("guardian_cleanup_receipt_missing", result["failures"])
        self.assertTrue(runtime.probe.exists())
        self.assertTrue(runtime.short.exists())

    def test_refuses_operator_socket_state_home_and_unowned_short_directory(self):
        daemon_home = self.run / "home"
        state = self.run / "state"
        valid_socket = self.run / "herdr.sock"
        validate_isolation(self.run, daemon_home, valid_socket, state, self.home, None)
        cases = [
            (daemon_home, self.home / ".config/herdr/herdr.sock", state),
            (daemon_home, valid_socket, self.home / ".hide/state"),
            (self.home, valid_socket, state),
            (daemon_home, self.root / "short.sock", state),
            (daemon_home, valid_socket, Path("relative-state")),
        ]
        for home, sock, private_state in cases:
            with self.subTest(case=str(sock)):
                with self.assertRaises(ProtectionError):
                    validate_isolation(self.run, home, sock, private_state, self.home, None)


@unittest.skipUnless(sys.platform == "darwin" or sys.platform.startswith("linux"),
                     "process guardian supports macOS and Linux")
class ProcessProtection(unittest.TestCase):
    def test_claude_exec_boundary_removes_injected_config_root_without_replacing_home(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            executable = root / "inert-cli"
            executable.write_text("#!/bin/sh\n" +
                                  "test -z \"${CLAUDE_CONFIG_DIR+x}\" || exit 31\n" +
                                  "test \"$DISABLE_AUTOUPDATER:$DISABLE_TELEMETRY:$DISABLE_ERROR_REPORTING\" = 1:1:1 || exit 32\n" +
                                  'test "$HOME" = "$EXPECTED_OWNED_HOME" || exit 33\n' +
                                  "echo exec-boundary-confirmed\n")
            executable.chmod(0o700)
            runtime = SimpleNamespace(bin=root / "bin", fixture_bin=None)
            target = wrapper(runtime, {"id": "claude-code", "executable": "claude"}, executable, None)
            with OwnedProcesses() as owner:
                code, output, error = owner.run([str(target)], env={**os.environ,
                    "HOME": str(root / "home"), "EXPECTED_OWNED_HOME": str(root / "home"),
                    "CLAUDE_CONFIG_DIR": str(root / "injected")}, check=False)
            self.assertEqual((code, output.strip()), (0, "exec-boundary-confirmed"), error)

    @unittest.skipUnless(sys.platform == "darwin", "Darwin unreaped getpgid boundary")
    def test_fast_completed_child_keeps_cleanup_and_successful_exit(self):
        # Gate Popen's return on a real unreaped zombie. Darwin getpgid then
        # returns ESRCH, while libproc still proves the reserved owned group.
        with tempfile.TemporaryDirectory() as name:
            receipt = Path(name) / "receipt.json"
            program = ("import os,sys,time,subprocess; from pathlib import Path\n"
                       "from agent_live_check import processes as p\n"
                       "original=subprocess.Popen\n"
                       "def completed(*args,**kwargs):\n"
                       " child=original(*args,**kwargs)\n"
                       " end=time.monotonic()+2\n"
                       " while True:\n"
                       "  table=p.snapshot(child.pid); p.require_complete(table)\n"
                       "  root=table.get(child.pid)\n"
                       "  if root and root.zombie: break\n"
                       "  if time.monotonic()>=end: raise RuntimeError('zombie_barrier_unavailable')\n"
                       "  time.sleep(.01)\n"
                       " try: os.getpgid(child.pid)\n"
                       " except ProcessLookupError: pass\n"
                       " else: raise RuntimeError('ESRCH_boundary_not_observed')\n"
                       " return child\n"
                       "subprocess.Popen=completed\n"
                       "reader,writer=os.pipe()\n"
                       f"code=p.guard(reader,[sys.executable,'-c','pass'],{str(receipt)!r},"
                       "os.environ['HIDE_LIVE_CHECK_OWNER'].split(':')[0],time.monotonic()+10)\n"
                       "os.close(writer)\nraise SystemExit(code)\n")
            with OwnedProcesses() as owner:
                code, _, error = owner.run([sys.executable, "-c", program],
                    env={**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1])}, check=False)
            self.assertEqual(code, 0, error)
            self.assertTrue(json.loads(receipt.read_text())["confirmed"])

    def test_expired_prelaunch_work_never_starts_a_child_or_owes_a_receipt(self):
        # The last case waits the whole command cap; a longer wait is refused before any of this.
        for phase, seconds, delay in (("receipt", .1, 2), ("environment", .1, 2), ("environment", 15, 16)):
            with self.subTest(phase=phase, seconds=seconds), tempfile.TemporaryDirectory() as name:
                clock = [100.0]
                owner = OwnedProcesses(diagnostics=Path(name) / "diagnostics")
                owner.deadline = 500
                owner.family = "f" * 64  # No registry mutation in this admission fixture.
                if phase == "receipt":
                    old = Mock()
                    old.poll.return_value = 0
                    owner.children[old] = None
                    def retire(child):
                        owner.children.pop(child)
                        clock[0] += delay
                    owner.end = retire
                class SlowEnvironment(dict):
                    def items(self):
                        if phase == "environment":
                            clock[0] += delay
                        return super().items()
                with patch("agent_live_check.processes.time.monotonic", lambda: clock[0]), \
                        patch("agent_live_check.processes.os.pipe", wraps=os.pipe) as pipe, \
                        patch("agent_live_check.processes.os.close", wraps=os.close) as close, \
                        patch("agent_live_check.processes.subprocess.Popen") as launch:
                    with self.assertRaisesRegex(ProcessError, "command_timeout"):
                        owner.run(["fixture-no-execution"], env=SlowEnvironment(), seconds=seconds)
                    launch.assert_not_called()
                    self.assertEqual(close.call_count, 2 * pipe.call_count)
                self.assertEqual(owner.children, {})
                self.assertEqual(owner.sequence, 0)
                self.assertEqual(list(owner.diagnostics.iterdir()), [])

    def test_guardian_refuses_expired_actual_child_admission_with_empty_cleanup_receipt(self):
        with tempfile.TemporaryDirectory() as name:
            diagnostic = Path(name) / "receipt.json"
            clock = [100.0]
            table = ProcessTable()
            table[123] = Process(123, 0, 123, 1, 0, False, os.getuid())
            def initial():
                clock[0] += 2
                return table
            with patch("agent_live_check.processes.time.monotonic", lambda: clock[0]), \
                    patch("agent_live_check.processes.snapshot", initial), \
                    patch("agent_live_check.processes.os.getpid", return_value=123), \
                    patch("agent_live_check.processes.signal.signal"), \
                    patch("agent_live_check.processes.threading.Thread.start"), \
                    patch("agent_live_check.processes.ctypes.CDLL") as libc, \
                    patch("agent_live_check.processes.issued_family", return_value=True), \
                    patch("agent_live_check.processes.subprocess.Popen") as launch, \
                    patch("sys.stderr", io.StringIO()) as output:
                libc.return_value.prctl.return_value = 0
                self.assertEqual(guard(-1, ["fixture-never-executed"], str(diagnostic), "f" * 64, 100.1), 124)
                launch.assert_not_called()
                self.assertIn("guardian_launch_timeout", output.getvalue())
            receipt = json.loads(diagnostic.read_text())
            self.assertTrue(receipt["confirmed"])
            self.assertTrue(receipt["launch_timed_out"])

    def test_unavailable_receipts_preserve_known_rss_counts_without_inventing_totals(self):
        # A cleanup receipt failure must not erase a known controller count or
        # turn an unavailable guardian total into zero (letter 2709, B8/B12).
        for state in ("missing", "unconfirmed", "corrupt", "nested", "invalid_samples", "missing_omitted",
                      "missing_unattributed", "invalid_unattributed", "invalid_identity"):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as name:
                run = Path(name)
                owner = OwnedProcesses(diagnostics=run / "diagnostics")
                subject = Process(123, 2, 123, 10, -1, False, os.getuid())
                for _ in range(2):
                    owner.rss_samples.measure({123: subject})
                receipt = run / "diagnostics/1.json"
                valid = {"confirmed": True, "unattributed": [], "additional_records_omitted": False,
                         "rss_samples": {"missed": 0, "max_consecutive_misses": 0, "consecutive_miss_limit": 3}}
                receipt.write_text(json.dumps(valid))
                owner.sequence = 1
                owner.attribution_report()  # A prior complete result must not mask a later failure.
                if state == "missing":
                    receipt.unlink()
                elif state == "corrupt":
                    receipt.write_text("{")
                elif state == "nested":
                    receipt.write_text("[" * 32768 + "]" * 32768)
                elif state == "unconfirmed":
                    receipt.write_text(json.dumps({**valid, "confirmed": False}))
                elif state == "invalid_samples":
                    receipt.write_text(json.dumps({**valid, "rss_samples": {}}))
                else:
                    broken = dict(valid)
                    if state.startswith("missing_"):
                        del broken["additional_records_omitted" if state == "missing_omitted" else "unattributed"]
                    else:
                        broken["unattributed"] = None if state == "invalid_unattributed" else [{"pid": 1}]
                    receipt.write_text(json.dumps(broken))
                report = {"herdr": {}, "agents": [], "configuration": {}, "failures": [],
                          "cleanup": {"confirmed": True}, "resources": {}}
                record_process_diagnostics(owner, report)
                self.assertFalse(report["cleanup"]["confirmed"])
                self.assertEqual(len(report["failures"]), 1)
                self.assertEqual(save(run, report), 2)
                samples = json.loads((run / "report.json").read_text())["resources"]["rss_samples"]
                self.assertEqual(samples["controller"]["missed"], 2)
                self.assertEqual(samples["controller"]["max_consecutive_misses"], 2)
                self.assertIsNone(samples["guardians"])
                self.assertIsNone(samples["missed"])
                self.assertIsNone(samples["max_consecutive_misses"])
                self.assertIn('"guardians": null', (run / "report.md").read_text())

    def test_final_report_retains_controller_and_guardian_rss_misses_after_failure(self):
        # Letter 2709 requires all missed observations in the final report,
        # including the controller's failed sample, independently of cleanup.
        for controller_misses, guardian_misses in ((2, 0), (3, 1)):
            with self.subTest(controller_misses=controller_misses), tempfile.TemporaryDirectory() as name:
                run = Path(name)
                owner = OwnedProcesses(diagnostics=run / "diagnostics")
                subject = Process(123, 2, 123, 10, -1, False, os.getuid())
                for index in range(controller_misses):
                    if index == 2:
                        with self.assertRaisesRegex(ProcessError, "rss_samples_unavailable"):
                            owner.rss_samples.measure({123: subject})
                    else:
                        owner.rss_samples.measure({123: subject})
                # A real private receipt file is the guardian/report boundary.
                receipt = {"confirmed": True, "unattributed": [], "additional_records_omitted": False,
                           "rss_samples": {"missed": guardian_misses, "max_consecutive_misses": guardian_misses,
                                           "consecutive_miss_limit": 3}}
                (run / "diagnostics/1.json").write_text(json.dumps(receipt))
                owner.sequence = 1
                report = {"herdr": {}, "agents": [], "configuration": {}, "failures": [], "resources": {},
                          "cleanup": {"confirmed": True, "attribution": owner.attribution_report()}}
                self.assertEqual(save(run, report), 3)
                samples = json.loads((run / "report.json").read_text())["cleanup"]["attribution"]["rss_samples"]
                self.assertEqual(samples["missed"], controller_misses + guardian_misses)
                self.assertEqual(samples["max_consecutive_misses"], controller_misses)
                self.assertEqual(samples["controller"]["missed"], controller_misses)
                self.assertEqual(samples["guardians"]["missed"], guardian_misses)
                self.assertEqual(samples["consecutive_miss_limit"], 3)
                self.assertIn('"controller"', (run / "report.md").read_text())

    def test_rss_samples_bound_consecutive_misses_by_live_identity(self):
        # Letter 2709 supplies the oracle: two misses may recover, the third
        # consecutive miss of the same live birth fails. Pure policy input
        # avoids mocking supervision; full5 covers its real guardian callers.
        def subject(pid=123, birth=10, rss=-1, zombie=False):
            return Process(pid, 2, 123, birth, rss, zombie, os.getuid())

        samples = RssSamples()
        for count in (1, 2):
            result = samples.measure({123: subject(), 124: subject(pid=124, rss=17)})
            self.assertEqual(result["rss_bytes"], 17)
            self.assertFalse(result["rss_complete"])
            self.assertEqual(result["rss_samples_missed"], count)
        result = samples.measure({123: subject(rss=23)})
        self.assertEqual(result["rss_bytes"], 23)
        self.assertTrue(result["rss_complete"])
        for _ in range(2):
            samples.measure({123: subject()})
        with self.assertRaisesRegex(ProcessError, "rss_samples_unavailable"):
            samples.measure({123: subject()})
        self.assertEqual(samples.summary(), {"missed": 5, "max_consecutive_misses": 3,
                                             "consecutive_miss_limit": 3})

        for ending in ("disappeared", "zombie", "replaced"):
            with self.subTest(ending=ending):
                samples = RssSamples()
                for _ in range(2):
                    samples.measure({123: subject()})
                if ending == "disappeared":
                    self.assertTrue(samples.measure({})["rss_complete"])
                elif ending == "zombie":
                    self.assertTrue(samples.measure({123: subject(zombie=True)})["rss_complete"])
                else:
                    samples.measure({123: subject(birth=11)})
                # Neither disappearance, a zombie nor another birth carries
                # the previous live identity's consecutive-miss streak.
                samples.measure({123: subject(birth=11 if ending == "replaced" else 10)})

    def test_a_crossed_memory_cap_names_the_live_processes_holding_the_most(self):
        def subject(pid, rss, name, zombie=False):
            return Process(pid, 2, 123, 10, rss, zombie, os.getuid(), name=name)
        table = {pid: subject(pid, rss, name, zombie) for pid, rss, name, zombie in (
            (1, 900, "omp", False), (2, 0, "claude", True), (3, -1, "zsh", False), (4, 700, "omp", False),
            (5, 50, "herdr", False), (6, 60, "zsh", False))}
        self.assertEqual(largest(table, count=3), [{"executable": "omp", "rss_bytes": 900},
                                                   {"executable": "omp", "rss_bytes": 700},
                                                   {"executable": "zsh", "rss_bytes": 60}])

    def test_failed_rss_read_distinguishes_exit_from_live_measurement_failure(self):
        # The sampler retains unknown live RSS for the bounded miss policy;
        # identity failure remains strict. Inject only libproc's read boundary.
        import errno
        for outcome in ("gone", "zombie", "replacement_owned", "replacement_outside",
                        "replacement_host", "live", "unreadable", "uid_changed"):
            with self.subTest(outcome=outcome):
                library = Mock()
                pid = os.getpid() if outcome == "replacement_host" else 24680
                bsd_reads = 0

                def list_group(group, pids, size):
                    pids[0] = pid
                    return 1

                def read_process(subject, flavor, argument, pointer, size):
                    nonlocal bsd_reads
                    if flavor == 4:
                        ctypes.set_errno(errno.ESRCH)
                        return 0
                    self.assertEqual(flavor, 3)
                    bsd_reads += 1
                    if bsd_reads > 1 and outcome in ("gone", "unreadable"):
                        ctypes.set_errno(errno.ESRCH if outcome == "gone" else errno.EPERM)
                        return 0
                    info = ctypes.cast(pointer, ctypes.POINTER(BsdInfo)).contents
                    info.pid, info.ppid, info.pgid, info.uid = pid, 2, pid, os.getuid()
                    info.sec, info.usec, info.flags = 100, 2, 0x10
                    info.status = 5 if bsd_reads > 1 and outcome == "zombie" else 1
                    if bsd_reads > 1 and outcome.startswith("replacement_"):
                        info.sec = 101
                        if outcome != "replacement_owned":
                            info.pgid = pid + 1
                    if bsd_reads > 1 and outcome == "uid_changed":
                        info.uid += 1
                    return ctypes.sizeof(BsdInfo)

                library.proc_listpgrppids.side_effect = list_group
                library.proc_listallpids.side_effect = lambda pids, size: list_group(None, pids, size)
                library.proc_pidinfo.side_effect = read_process
                with patch("agent_live_check.process_table.sys.platform", "darwin"), \
                        patch("agent_live_check.process_table.ctypes.CDLL", return_value=library):
                    table = snapshot(None if outcome == "replacement_host" else pid)
                self.assertEqual(bsd_reads, 2)
                self.assertEqual(library.proc_pidinfo.call_count, 3)
                if outcome in ("gone", "replacement_outside"):
                    self.assertNotIn(pid, table)
                    self.assertEqual(table.vanished, [pid])
                    require_complete(table)
                elif outcome in ("unreadable", "uid_changed"):
                    with self.assertRaisesRegex(RuntimeError, "subjects_unavailable"):
                        require_complete(table)
                else:
                    self.assertEqual(table[pid].rss, -1)
                    self.assertEqual(table[pid].zombie, outcome == "zombie")
                    replaced = outcome.startswith("replacement_")
                    self.assertEqual(table[pid].birth, 101_000_002 if replaced else 100_000_002)
                    self.assertEqual(table.vanished, [])
                    if replaced:
                        self.assertEqual(table[pid].group, pid if outcome == "replacement_owned" else pid + 1)
                        old = Process(pid, 2, pid, 100_000_002, 0, False, os.getuid())
                        self.assertNotIn(pid, descendants(table, pid + 100, known={pid: old}))

    def test_unavailable_controller_ancestry_refuses_prelaunch_exclusion(self):
        table = ProcessTable()
        table[100] = Process(100, 200, 100, 10, 0, False, os.getuid())
        table.unavailable.append({"pid": 200, "errno": 13})
        with self.assertRaisesRegex(ProcessError, "control_plane_ancestry_unavailable"):
            control_plane(table, 100)
        table[200] = Process(200, 1, 200, 20, 0, False, os.getuid())
        table[1] = Process(1, 0, 1, 1, 0, False, os.getuid())
        self.assertEqual(control_plane(table, 100), {100: 10, 200: 20, 1: 1})
        del table[1]
        with self.assertRaisesRegex(ProcessError, "control_plane_ancestry_unavailable"):
            control_plane(table, 100)
        table.foreign_uid_pids.add(1)
        self.assertEqual(control_plane(table, 100), {100: 10, 200: 20})

    def test_missing_or_unconfirmed_receipt_cannot_confirm_cleanup(self):
        for missing in (True, False):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory(prefix="agent-receipt-") as name:
                with OwnedProcesses(diagnostics=Path(name) / "diagnostics") as owner:
                    child = owner.spawn([sys.executable, "-c", "pass"], env=dict(os.environ),
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    self.assertEqual(child.wait(timeout=5), 0)
                    receipt = Path(name) / "diagnostics/1.json"
                    if missing:
                        receipt.unlink()
                    else:
                        receipt.write_text('{"confirmed":false}')
                    with self.assertRaisesRegex(ProcessError, "guardian_cleanup_receipt"):
                        owner.end(child)

    def test_diagnostic_write_error_is_guardian_failure(self):
        with tempfile.TemporaryDirectory(prefix="agent-diagnostic-") as name:
            with OwnedProcesses(diagnostics=Path(name) / "diagnostics") as owner:
                (Path(name) / "diagnostics/1.json").write_text("occupied")
                with self.assertRaisesRegex(ProcessSafetyError, "guardian_cleanup_or_resource_failure"):
                    owner.run([sys.executable, "-c", "pass"], env=dict(os.environ), check=False)

    @unittest.skipUnless(sys.platform == "darwin", "XNU excludes zombies from killpg's target count")
    def test_reserved_zombie_group_finishes_after_permission_refusal(self):
        # B8 requires final absence, not an unnecessary signal to a zombie.
        # The child holds its own unreaped leader so the PGID cannot be reused.
        program = '''import os,signal,time
from agent_live_check.process_table import snapshot
from agent_live_check.processes import signal_reserved_group
reader,writer=os.pipe()
pid=os.fork()
if pid==0:
 os.close(reader);os.setsid();os.write(writer,b"1");os.close(writer);os._exit(0)
os.close(writer)
try:
 assert os.read(reader,1)==b"1"
 os.close(reader)
 deadline=time.monotonic()+2
 while True:
  table=snapshot(pid)
  if pid in table and table[pid].zombie:break
  if time.monotonic()>=deadline:raise RuntimeError("zombie_not_observed")
  time.sleep(.01)
 signal_reserved_group(pid,signal.SIGKILL)
finally:
 os.waitpid(pid,0)
try:os.killpg(pid,0)
except ProcessLookupError:print("group_absent_after_reap")
else:raise RuntimeError("group_survived_reap")
'''
        env = {**os.environ, "PYTHONPATH": str(Path(__file__).resolve().parents[1])}
        with OwnedProcesses() as owner:
            code, output, error = owner.run([sys.executable, "-c", program], env=env, seconds=10, check=False)
        self.assertEqual((code, output.strip()), (0, "group_absent_after_reap"), error)

    @unittest.skipUnless(sys.platform == "darwin", "Darwin libproc boundary")
    def test_permission_refusal_requires_complete_terminal_group(self):
        # The kernel boundary may change after killpg. Live or unreadable
        # membership cannot turn EPERM into a completed signal obligation.
        for outcome in ("live", "unavailable", "zombie", "empty"):
            with self.subTest(outcome=outcome):
                library = Mock()
                group_reads = 0
                def list_group(group, pids, size):
                    nonlocal group_reads
                    group_reads += 1
                    if group_reads > 1 and outcome == "empty":
                        return 0
                    pids[0] = 123
                    return 1
                def read_info(pid, flavor, arg, buffer, size):
                    if group_reads > 1 and outcome == "unavailable":
                        ctypes.set_errno(errno.EPERM)
                        return 0
                    if flavor == 3:
                        value = ctypes.cast(buffer, ctypes.POINTER(BsdInfo)).contents
                        value.pid, value.pgid, value.uid = 123, 123, os.getuid()
                        value.sec, value.status = 10, 5 if group_reads > 1 and outcome == "zombie" else 1
                    return size
                library.proc_listpgrppids.side_effect = list_group
                library.proc_pidinfo.side_effect = read_info
                output = io.StringIO()
                with patch.object(os, "killpg", side_effect=PermissionError(errno.EPERM, "denied")), \
                        patch("agent_live_check.process_table.ctypes.CDLL", return_value=library), \
                        patch.object(sys, "stderr", output):
                    if outcome in ("live", "unavailable"):
                        with self.assertRaisesRegex(ProcessError, '"errno":1'):
                            signal_reserved_group(123, signal.SIGKILL)
                        self.assertEqual(output.getvalue(), "")
                    else:
                        signal_reserved_group(123, signal.SIGKILL)
                        record = json.loads(output.getvalue().split(":", 1)[1])
                        self.assertEqual(record, {"group": 123, "signal": signal.SIGKILL,
                                                  "errno": errno.EPERM, "refresh": "terminal",
                                                  "zombie_members": int(outcome == "zombie")})

    def test_prior_issued_marker_is_owned_under_a_live_parent_but_fabricated_marker_is_not(self):
        with OwnedProcesses() as previous:
            _, marker, _ = previous.run([sys.executable, "-c", "import os; print(os.environ['HIDE_LIVE_CHECK_OWNER'])"],
                                        env=dict(os.environ))
        for value, owned in (("0" * 64 + ":1:0:" + "0" * 64, False), (marker.strip(), True)):
            with self.subTest(owned=owned):
                fixture = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                           env={**os.environ, "HIDE_LIVE_CHECK_OWNER": value},
                                           start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                reaper = threading.Thread(target=fixture.wait) if owned else None
                if reaper:
                    reaper.start()
                try:
                    with OwnedProcesses() as owner:
                        owner.run([sys.executable, "-c", "pass"], env=dict(os.environ))
                    if owned:
                        fixture.wait(timeout=2)
                    else:
                        self.assertIsNone(fixture.poll(), "fabricated marker claimed a foreign process")
                finally:
                    if fixture.poll() is None:
                        fixture.kill()
                    fixture.wait(timeout=2)
                    if reaper:
                        reaper.join(timeout=2)
                        self.assertFalse(reaper.is_alive())

    def test_unreadable_foreign_orphan_is_diagnostic_and_not_owned(self):
        orphan = Process(111, 1, 111, 10, 0, False, os.getuid(), name="fixture")
        for platform in ("darwin", "linux"):
            with self.subTest(platform=platform):
                library = Mock()
                library.sysctl.return_value = -1
                unknown = []
                with patch("agent_live_check.process_table.sys.platform", platform), \
                        patch("agent_live_check.process_table.ctypes.CDLL", return_value=library), \
                        patch("agent_live_check.process_table.ctypes.get_errno", return_value=errno.EACCES), \
                        patch.object(Path, "open", side_effect=PermissionError(errno.EACCES, "fixture environ")):
                    self.assertEqual(marked_descendants({111: orphan}, "fixture", unknown=unknown.append), {})
                self.assertEqual(unknown, [orphan])

    def test_term_ignoring_group_is_killed_without_ending_a_peer(self):
        with tempfile.TemporaryDirectory(prefix="agent-group-") as name:
            receipt = Path(name) / "group"
            program = ("import os,signal,time; from pathlib import Path; "
                       "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                       f"Path({str(receipt)!r}).write_text(str(os.getpgrp())); "
                       "os.fork(); time.sleep(30)")
            with OwnedProcesses() as peer, OwnedProcesses() as owner:
                unrelated = peer.spawn([sys.executable, "-c", "import time; time.sleep(30)"],
                                       env=dict(os.environ), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                with self.assertRaisesRegex(ProcessError, "command_timeout"):
                    owner.run([sys.executable, "-c", program], env=dict(os.environ), seconds=0.5)
                self.assertIsNone(unrelated.poll(), "cleanup ended concurrent work")
                self.assertTrue(receipt.exists(), "group fixture never started")
                with self.assertRaises(ProcessLookupError):
                    os.killpg(int(receipt.read_text()), 0)

    def test_helper_forked_during_term_cannot_survive_group_cleanup(self):
        with tempfile.TemporaryDirectory(prefix="agent-shutdown-") as name:
            receipt = Path(name) / "group"
            program = ("import os,signal,time; from pathlib import Path"
                       "\ndef shutdown(*args):"
                       "\n if os.fork(): raise SystemExit(0)"
                       "\n signal.signal(signal.SIGTERM,signal.SIG_IGN)"
                       "\n time.sleep(30)"
                       "\nsignal.signal(signal.SIGTERM,shutdown)"
                       f"\nPath({str(receipt)!r}).write_text(str(os.getpgrp()))"
                       "\ntime.sleep(30)")
            with OwnedProcesses() as owner:
                with self.assertRaisesRegex(ProcessError, "command_timeout"):
                    owner.run([sys.executable, "-c", program], env=dict(os.environ), seconds=0.5)
            self.assertTrue(receipt.exists(), "shutdown fixture never started")
            with self.assertRaises(ProcessLookupError):
                os.killpg(int(receipt.read_text()), 0)

    def test_linux_procfs_must_name_the_same_namespace_without_hiding_processes(self):
        mount = "1 0 0:5 / /proc rw - proc proc rw\n"
        validate_linux_procfs("NSpid:\t42\n", mount, 42)
        cases = [("NSpid:\t7000\t42\n", mount), ("NSpid:\t42\t42\n", mount),
                 ("Pid:\t42\n", mount), ("NSpid:\t42\nNSpid:\t42\n", mount)]
        for policy in ("1", "2", "4", "noaccess", "invisible", "ptraceable", "unknown"):
            cases.append(("NSpid:\t42\n", mount.replace("- proc proc rw", "- proc proc rw,hidepid=" + policy)))
        cases.extend([("NSpid:\t42\n", mount.replace("- proc", "- tmpfs")),
                      ("NSpid:\t42\n", mount + "2 1 0:6 / /proc/80/stat rw - tmpfs none rw\n")])
        for status, mounts in cases:
            with self.subTest(status=status, mounts=mounts), self.assertRaises(RuntimeError):
                validate_linux_procfs(status, mounts, 42)


    def test_empty_first_argument_cannot_hide_owned_environment_or_prove_a_cropped_negative(self):
        owner = b"HIDE_LIVE_CHECK_OWNER=fixture-run"
        for width in (4, 8):
            orphan = Process(111, 1, 111, 10, 0, False, os.getuid(), pointer_width=width)
            library = Mock()
            data = procargs(owner, b"OTHER=fixture", argv=(b"",), pointer_width=width)
            def query(mib, unused, buffer, size, *rest):
                ctypes.memmove(buffer, data, len(data))
                size._obj.value = len(data)
                return 0
            library.sysctl.side_effect = query
            def read_info(pid, flavor, unused, buffer, size):
                info = buffer._obj
                info.sec, info.usec, info.uid = 0, 10, os.getuid()
                info.flags = 0x10 if width == 8 else 0
                return size
            library.proc_pidinfo.side_effect = read_info
            with self.subTest(pointer_width=width), \
                    patch.object(sys, "platform", "darwin"), \
                    patch("agent_live_check.process_table.ctypes.CDLL", return_value=library):
                self.assertEqual(marked_descendants({111: orphan}, "fixture-run", 1), {111: orphan})
            # The restricted-target crop can retain OTHER while hiding the
            # later owner entry. A readable prefix cannot exclude ownership.
            cropped = procargs(b"OTHER=fixture", argv=(b"",), pointer_width=width)
            with self.assertRaisesRegex(RuntimeError, "owner_environment_unavailable"):
                procargs_owned(cropped, width, owner)
            malformed = bytearray(data)
            malformed[13] = ord("x")  # The first required path-padding byte.
            with self.assertRaisesRegex(RuntimeError, "owner_environment_unavailable"):
                procargs_owned(malformed, width, owner)


    def test_successful_token_answer_is_rejected_if_exec_changed_pointer_width(self):
        orphan = Process(111, 1, 111, 10, 0, False, os.getuid(), pointer_width=4)
        table = ProcessTable()
        table[111] = orphan
        library = Mock()
        def query(mib, unused, buffer, size, *rest):
            data = procargs(b"OTHER=fixture", pointer_width=4)
            ctypes.memmove(buffer, data, len(data))
            size._obj.value = len(data)
            return 0
        def changed_width(pid, flavor, unused, buffer, size):
            info = buffer._obj
            info.sec, info.usec, info.uid, info.flags = 0, 10, os.getuid(), 0x10
            return size
        library.sysctl.side_effect, library.proc_pidinfo.side_effect = query, changed_width
        unknown = []
        with patch.object(sys, "platform", "darwin"), \
                patch("agent_live_check.process_table.ctypes.CDLL", return_value=library):
            self.assertEqual(marked_descendants(table, "fixture-run", 1, unknown=unknown.append), {})
        # Letter 2787: an exec width change rejects stale argv interpretation,
        # but neither proves exit nor transfers established birth ownership.
        self.assertEqual(table.vanished, [])
        self.assertEqual(unknown, [orphan])


    def test_argument_replies_keep_independent_lengths_capacity_and_refusals(self):
        # Each sysctl reply owns only its returned prefix. Earlier PID bytes
        # cannot claim a later process, including after a shorter or denied read.
        marker = b"HIDE_LIVE_CHECK_OWNER=fixture-run"
        subjects = {pid: Process(pid, 1, pid, 10, 0, False, os.getuid())
                    for pid in (111, 112, 113, 114)}
        replies = {111: procargs(b"OTHER=" + b"x" * 8192, marker),
                   112: procargs(b"OTHER=fixture"),
                   113: procargs(b"OTHER=" + b"x" * 16384, marker),
                   114: None}
        library = Mock()

        def query(mib, unused, buffer, size, *rest):
            data = replies[mib[2]]
            if data is None or len(data) > size._obj.value:
                ctypes.set_errno(errno.EPERM if data is None else errno.ENOMEM)
                return -1
            ctypes.memmove(buffer, data, len(data))
            size._obj.value = len(data)
            return 0

        def read_info(pid, flavor, unused, buffer, size):
            info = buffer._obj
            info.sec, info.usec, info.uid, info.flags = 0, 10, os.getuid(), 0x10
            return size

        library.sysctl.side_effect, library.proc_pidinfo.side_effect = query, read_info
        unknown = []
        with patch.object(sys, "platform", "darwin"), \
                patch("agent_live_check.process_table.ctypes.CDLL", return_value=library):
            actual = marked_descendants(subjects, "fixture-run", 1, unknown=unknown.append)
        self.assertEqual(actual, {111: subjects[111], 113: subjects[113]})
        self.assertEqual(unknown, [subjects[114]])


    def test_owner_refusal_distinguishes_opaque_shapes_without_payload_contents(self):
        owner = b"HIDE_LIVE_CHECK_OWNER=private-owner-token"
        good = procargs(b"OTHER=private-environment")
        padding = bytearray(good)
        padding[13] = ord("x")
        cases = [(b"\x01", 8, "header_short"),
                 ((0).to_bytes(4, sys.byteorder, signed=True) + good[4:], 8, "argc_invalid"),
                 (good, 3, "width_invalid"), (good[:4] + b"private-path", 8, "path_unterminated"),
                 (good[:13], 8, "padding_missing"), (padding, 8, "padding_invalid"),
                 (good[:20] + b"private-argument", 8, "argv_unterminated"),
                 (good[:-1], 8, "tail_unterminated"), (procargs(), 8, "tail_empty"),
                 (procargs(b"private-unassigned-entry"), 8, "tail_non_assignment")]
        for payload, width, reason in cases:
            with self.subTest(reason=reason), self.assertRaises(RuntimeError) as failure:
                procargs_owned(payload, width, owner)
            message = str(failure.exception)
            details = json.loads(message.split(":", 1)[1])
            self.assertEqual((details["reason"], details["bytes"]), (reason, len(payload)))
            self.assertNotIn("private-", message)


    def test_successful_token_answer_is_rejected_if_sampled_pid_was_reused(self):
        orphan = Process(111, 1, 111, 10, 0, False, os.getuid())
        for environment in (b"OTHER=fixture", b"HIDE_LIVE_CHECK_OWNER=fixture-run"):
            table = ProcessTable()
            table[111] = orphan
            library = Mock()
            def query(mib, unused, buffer, size, *rest):
                data = procargs(environment)
                ctypes.memmove(buffer, data, len(data))
                size._obj.value = len(data)
                return 0
            def replacement(pid, flavor, unused, buffer, size):
                info = buffer._obj
                info.sec, info.usec, info.uid = 0, 11, os.getuid()
                return size
            library.sysctl.side_effect = query
            library.proc_pidinfo.side_effect = replacement
            with self.subTest(environment=environment), \
                    patch.object(sys, "platform", "darwin"), \
                    patch("agent_live_check.process_table.ctypes.CDLL", return_value=library):
                self.assertEqual(marked_descendants(table, "fixture-run", 1), {})
            self.assertEqual(table.vanished, [111])


    def test_linux_final_empty_view_needs_kernel_no_child_proof_including_clone_children(self):
        with patch("agent_live_check.processes.os.waitpid", side_effect=[(222, 0), (0, 0)]) as wait:
            self.assertTrue(linux_children_remain())
        self.assertTrue(all(call.args == (-1, os.WNOHANG | 0x40000000) for call in wait.call_args_list))
        with patch("agent_live_check.processes.os.waitpid", side_effect=[(222, 0), ChildProcessError]):
            self.assertFalse(linux_children_remain())


    def test_readable_marked_double_fork_is_cleaned(self):
        with tempfile.TemporaryDirectory(prefix="agent-detach-") as name:
            receipt = Path(name) / "child.pid"
            script = ("import os,time; from pathlib import Path; "
                      "pid=os.fork(); "
                      "\nif pid: raise SystemExit(0)"
                      "\nos.setsid()"
                      "\nif os.fork(): raise SystemExit(0)"
                      "\nos.close(1); os.close(2)"
                      f"\nPath({str(receipt)!r}).write_text(str(os.getpid()))"
                      "\ntime.sleep(5)")
            with OwnedProcesses() as owner:
                code, _, _ = owner.run([sys.executable, "-c", script], env=dict(os.environ))
                self.assertEqual(code, 0)
            # A missing receipt means the kernel tracked and stopped the leaf
            # before it ran. A live receipt must name an already ended process.
            if receipt.exists():
                item = snapshot().get(int(receipt.read_text()))
                self.assertTrue(item is None or item.zombie, "detached child survived confirmed cleanup")


    def test_finite_command_and_repeated_units_leave_no_child(self):
        with OwnedProcesses() as owner:
            for _ in range(20):
                code, out, err = owner.run([sys.executable, "-c", "print('finished')"], env=dict(os.environ))
                self.assertEqual((code, out.strip(), err), (0, "finished", ""))
            self.assertLessEqual(len(owner.children), 1)
        self.assertEqual(owner.children, {})


    def test_timeout_and_output_limit_fail_without_leaving_child(self):
        with OwnedProcesses() as owner:
            with self.assertRaisesRegex(ProcessError, "command_timeout"):
                owner.run([sys.executable, "-c", "import time; time.sleep(30)"], env=dict(os.environ), seconds=0.3)
            with self.assertRaisesRegex(ProcessError, "command_output_over_budget"):
                owner.run([sys.executable, "-c", "import sys; sys.stdout.write('x'*2000000)"], env=dict(os.environ))
        self.assertEqual(owner.children, {})


    def test_killed_owner_leaves_no_child_or_grandchild(self):
        # This fixture deliberately has no outer guardian: otherwise the outer
        # cleanup could hide a defect in the production child's EOF guardian.
        with tempfile.TemporaryDirectory(prefix="agent-owner-") as name:
            path = Path(name) / "pids.json"
            leaf = "import os,time; from pathlib import Path; Path(%r).write_text(str(os.getpid())); time.sleep(30)" % str(path)
            script = "import os,sys,time; sys.path.insert(0,%r); from agent_live_check.processes import OwnedProcesses; o=OwnedProcesses(); o.spawn([sys.executable,'-c',%r],env=dict(os.environ)); time.sleep(30)" % (str(Path(__file__).resolve().parents[1]), leaf)
            fixture = subprocess.Popen([sys.executable, "-c", script], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                deadline = time.monotonic() + 5
                while not path.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(path.exists(), "fixture child did not announce its pid")
                pid = int(path.read_text())
                table = snapshot()
                child = table[pid]
                guardian = table[child.parent]
                fixture.kill()
                fixture.wait(timeout=2)
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    table = snapshot()
                    remaining = [item.pid for item in (child, guardian)
                                 if item.pid in table and table[item.pid].birth == item.birth
                                 and not table[item.pid].zombie]
                    if not remaining:
                        break
                    time.sleep(0.05)
                self.assertEqual(remaining, [])
            finally:
                if fixture.poll() is None:
                    fixture.kill()
                    fixture.wait(timeout=2)
                fixture.stderr.close()



@unittest.skipUnless(sys.platform == "darwin", "authenticated write guard is macOS-only")
class NativeWriteProtection(unittest.TestCase):
    def test_cli_symlink_and_custom_operator_socket_cannot_be_removed_or_retargeted(self):
        with tempfile.TemporaryDirectory(prefix="acl-alias-", dir="/tmp") as name:
            root = Path(name).resolve()
            run, sockets, home, checkout = [root / key for key in ("run", "sockets", "operator", "candidate")]
            for path in (run, sockets, home, checkout):
                path.mkdir(mode=0o700)
            target = root / "installation/cli"
            target.parent.mkdir()
            target.write_bytes(b"inert-original")
            entry = home / "bin/cli"
            entry.parent.mkdir()
            intermediate = root / "intermediate/cli"
            intermediate.parent.mkdir()
            intermediate.symlink_to(target)
            entry.symlink_to(intermediate)
            config = root / "custom-xdg/herdr"
            config.mkdir(parents=True)
            config_file = config / "config.json"
            config_file.write_bytes(b"original-routing")
            endpoint = root / "custom-socket/operator.sock"
            endpoint.parent.mkdir()
            listener = socket.socket(socket.AF_UNIX)
            try:
                listener.bind(str(endpoint))
                identity = endpoint.stat().st_ino
                sandbox = WriteSandbox(run, sockets, home, checkout=checkout,
                                       routing=operator_routing({"HERDR_SOCKET_PATH": str(endpoint),
                                                                 "XDG_CONFIG_HOME": str(config.parent)}),
                                       executables=[entry])
                program = ("from pathlib import Path\n"
                           f"for name in {[str(entry), str(intermediate), str(endpoint), str(config_file)]!r}:\n"
                           " p=Path(name)\n"
                           " for operation in ('unlink','replace'):\n"
                           "  try:\n"
                           "   if operation=='unlink': p.unlink()\n"
                           "   else:\n"
                           f"    q=Path({str(sandbox.probe / 'replacement')!r}); q.write_bytes(b'forbidden'); q.replace(p)\n"
                           "  except PermissionError: pass\n"
                           "  else: raise SystemExit(31)\n"
                           f"try: Path({str(config_file)!r}).write_bytes(b'forbidden')\n"
                           "except PermissionError: pass\n"
                           "else: raise SystemExit(33)\n"
                           f"for name in {[str(entry.parent), str(intermediate.parent), str(target.parent), str(config)]!r}:\n"
                           " p=Path(name); q=p.with_name(p.name+'.moved')\n"
                           " try: p.rename(q)\n"
                           " except PermissionError: pass\n"
                           " else: q.rename(p); raise SystemExit(32)\n")
                with OwnedProcesses() as owner:
                    code, _, error = owner.run(sandbox.command([sys.executable, "-c", program]),
                                               env=dict(os.environ), check=False)
                self.assertEqual(code, 0, error)
                self.assertTrue(entry.is_symlink())
                self.assertTrue(intermediate.is_symlink())
                self.assertEqual(entry.resolve(), target)
                self.assertEqual(target.read_bytes(), b"inert-original")
                self.assertEqual(config_file.read_bytes(), b"original-routing")
                self.assertTrue(endpoint.is_socket())
                self.assertEqual(endpoint.stat().st_ino, identity)
            finally:
                listener.close()

    def test_other_process_control_arguments_and_mailbox_storage_are_denied(self):
        with tempfile.TemporaryDirectory(prefix="acl-", dir="/tmp") as name:
            root = Path(name).resolve()
            run, sockets, home = [root / key for key in ("run", "sockets", "operator")]
            for path in (run, sockets, home):
                path.mkdir(mode=0o700)
            state = run / "state"
            caps = state / "pane-capabilities"
            caps.mkdir(parents=True, mode=0o700)
            reference = caps / "own.json"
            reference.write_text("fixture-reference")
            reference.chmod(0o600)
            mailbox = state / "mailbox.json"
            mailbox.write_text("fresh-test-marker")
            guard = WriteSandbox(run, sockets, home, checkout=root / "candidate")
            settings = guard.probe / "integration/.claude/settings.json"
            settings.parent.mkdir(parents=True, mode=0o700)
            settings.write_text("fixture-settings")
            guard.allow_reference(reference)
            with OwnedProcesses() as owner:
                standin = owner.spawn([sys.executable, "-c", "import time; time.sleep(5)", "fixture-process-marker"],
                                     env=dict(os.environ), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                program = ("import ctypes,errno,os; from pathlib import Path"
                           # Native settings readers vet every path component.
                           # Metadata access must not disclose controller data.
                           f"\nfor parent in Path({str(settings)!r}).parents: parent.lstat()"
                           f"\nassert Path({str(settings)!r}).read_text() == 'fixture-settings'"
                           # The run root lists (Bun's realpath opens it); the capabilities folder does not.
                           f"\nlist(Path({str(run)!r}).iterdir())"
                           f"\ntry: list(Path({str(caps)!r}).iterdir())"
                           "\nexcept PermissionError: pass"
                           "\nelse: raise SystemExit(33)"
                           f"\nassert Path({str(reference)!r}).read_text() == 'fixture-reference'"
                           f"\nPath({str(reference.with_suffix('.claimed'))!r}).write_text('claim')"
                           f"\ntry: Path({str(mailbox)!r}).read_text()"
                           "\nexcept PermissionError: pass"
                           "\nelse: raise SystemExit(31)"
                           f"\ntry: os.kill({standin.pid}, 0)"
                           "\nexcept PermissionError: pass"
                           "\nelse: raise SystemExit(32)"
                           "\nlib=ctypes.CDLL(None,use_errno=True)"
                           f"\nmib=(ctypes.c_int*3)(1,49,{standin.pid})"
                           "\nsize=ctypes.c_size_t(1024*1024); data=ctypes.create_string_buffer(size.value)"
                           "\nassert lib.sysctl(mib,3,data,ctypes.byref(size),None,0) == -1"
                           "\nassert ctypes.get_errno() == errno.EPERM"
                           "\nprint('all-denials-enforced')")
                code, output, error = owner.run(guard.command([sys.executable, "-c", program]), env=dict(os.environ), check=False)
                self.assertEqual((code, output.strip()), (0, "all-denials-enforced"), error)

    def test_real_sandbox_protects_declared_files_and_allows_native_state(self):
        with tempfile.TemporaryDirectory(prefix="acl-", dir="/tmp") as name:
            root = Path(name).resolve()
            run, sockets, outside, home = [root / key for key in ("run", "sockets", "outside", "operator")]
            for path in (run, sockets, outside, home):
                path.mkdir(mode=0o700)
            history = home / "sessions"
            history.mkdir()
            previous = [history / name for name in ("previous.json", "another.json", "third.json")]
            for path in previous:
                path.write_bytes(b"operator-session")
            protected = history / "declared-config.json"
            protected.write_bytes(b"original-config")
            guard = ConfigGuard(run / "backup", [protected], [history])
            sandbox = WriteSandbox(run, sockets, home, checkout=root / "candidate", protected=[protected])
            with OwnedProcesses() as owner:
                sandbox.verify(owner, dict(os.environ), outside)
                # Lead2702 explicitly allows existing history writes. A
                # declared nonshared config remains denied even inside it.
                program = ("from pathlib import Path\n"
                           f"for name in {[str(path) for path in previous]!r}:\n"
                           " Path(name).write_bytes(b'changed')\n"
                           f"try: Path({str(protected)!r}).write_bytes(b'forbidden')\n"
                           "except PermissionError: pass\n"
                           "else: raise SystemExit(31)\n")
                code, _, _ = owner.run(sandbox.command([sys.executable, "-c", program]), env=dict(os.environ), check=False)
                self.assertEqual(code, 0)
                for path in previous:
                    self.assertEqual(path.read_bytes(), b"changed")
                self.assertEqual(protected.read_bytes(), b"original-config")
                fresh = history / "new-session.json"
                program = "from pathlib import Path; Path(%r).write_bytes(b'new-run-session')" % str(fresh)
                code, _, _ = owner.run(sandbox.command([sys.executable, "-c", program]), env=dict(os.environ), check=False)
                self.assertEqual(code, 0)
                self.assertEqual(fresh.read_bytes(), b"new-run-session")
            result = guard.finish()
            self.assertEqual(result["failures"], [])
            for path in previous:
                self.assertIn({"path": str(path), "kind": "changed"}, result["directory_changes"])
            self.assertIn({"path": str(fresh), "kind": "added"}, result["directory_changes"])

    def test_control_files_and_ancestors_cannot_be_replaced_by_native_agent(self):
        with tempfile.TemporaryDirectory(prefix="acl-", dir="/tmp") as name:
            root = Path(name).resolve()
            run, sockets, home, checkout, routing = [root / key for key in
                                                   ("run", "sockets", "operator", "candidate", "custom-state")]
            for path in (run, sockets, home, checkout, routing):
                path.mkdir(mode=0o700)
            gitdir = root / "shared-git/worktrees/candidate"
            gitdir.mkdir(parents=True)
            git_alias = root / "git-links/current"
            common_alias = root / "common-links/current"
            for path, target in ((git_alias, gitdir), (common_alias, root / "shared-git")):
                path.parent.mkdir()
                path.symlink_to(target, target_is_directory=True)
            (checkout / ".git").write_text("gitdir: " + str(git_alias) + "\n")
            (gitdir / "commondir").write_text(str(common_alias) + "\n")
            executable = root / "installed-cli"
            protected = home / "native/declared.json"
            controls = [run / "configuration-backup/0", run / "bin/wrapper", run / "evidence.json",
                        checkout / "agents/runs/process-owner-families/issued", checkout / "scripts/source.py",
                        home / ".hide/state/ledger", home / ".local/state/hide/ledger",
                        home / ".config/herdr/config", routing / "ledger", protected,
                        gitdir / "index", root / "shared-git/refs/head", executable]
            for path in controls:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"original-control")
            sandbox = WriteSandbox(run, sockets, home, checkout=checkout,
                                   protected=[protected], routing=[routing], executables=[executable])
            program = ("from pathlib import Path\n"
                       f"for name in {[str(path) for path in controls]!r}:\n"
                       " p=Path(name)\n"
                       " for operation in ('write','unlink','replace'):\n"
                       "  try:\n"
                       "   if operation=='write': p.write_bytes(b'forbidden')\n"
                       "   elif operation=='unlink': p.unlink()\n"
                       "   else:\n"
                       f"    q=Path({str(sandbox.probe / 'replacement')!r}); q.write_bytes(b'forbidden'); q.replace(p)\n"
                       "  except PermissionError: pass\n"
                       "  else: raise SystemExit(31)\n"
                       f"for name in {[str(path) for path in (git_alias, common_alias)]!r}:\n"
                       " p=Path(name)\n"
                       " try: p.unlink()\n"
                       " except PermissionError: pass\n"
                       " else: raise SystemExit(34)\n"
                       f"for name in {[str(path) for path in (run, checkout, root, protected.parent, home / '.hide', routing, git_alias.parent, common_alias.parent)]!r}:\n"
                       " p=Path(name); q=p.with_name(p.name+'.moved')\n"
                       " try: p.rename(q)\n"
                       " except PermissionError: pass\n"
                       " else: q.rename(p); raise SystemExit(32)\n"
                       f"Path({str(home / 'native/session-env')!r}).mkdir()\n"
                       f"Path({str(home / '.claude.json')!r}).write_bytes(b'native-shared-state')\n")
            with OwnedProcesses() as owner:
                code, _, error = owner.run(sandbox.command([sys.executable, "-c", program]),
                                           env=dict(os.environ), check=False)
            self.assertEqual(code, 0, error)
            for path in controls:
                self.assertEqual(path.read_bytes(), b"original-control")
            self.assertTrue(git_alias.is_symlink())
            self.assertTrue(common_alias.is_symlink())
            self.assertTrue((home / "native/session-env").is_dir())
            self.assertEqual((home / ".claude.json").read_bytes(), b"native-shared-state")


if __name__ == "__main__":
    unittest.main()
