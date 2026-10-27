"""The live check's Windows forms, run on Windows (docs/VERIFICATION.md, Agent
journey): a child's job ends its whole tree when the owner dies, every cap
is the Unix guardian's and a crossed one is a named failure, private files
admit only this account, the pane shell's commands and the launchers carry
their arguments and exit status, and the run's folder leaves the account's
Path as it found it.

No Herdr, hided or agent is needed; `agent-journey.yml` runs this file on a
Windows runner. Elsewhere every test is skipped.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check import processes
from agent_live_check.processes import OwnedProcesses, ProcessError, ProcessSafetyError
from agent_live_check.protection import private, private_directory, write_private
from agent_live_check.system import WINDOWS, PaneShell, launcher, remove_pane_files, remove_tree, same_path

SCRIPTS = Path(__file__).resolve().parents[1]
WAIT_SECONDS = 10


def wait_ended(pid, seconds):
    """Whether `pid` ends within `seconds`; a pid that is already gone has ended."""
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    synchronize = 0x00100000
    handle = kernel32.OpenProcess(synchronize, False, pid)
    if not handle:
        return True
    try:
        return kernel32.WaitForSingleObject(handle, int(seconds * 1000)) == 0
    finally:
        kernel32.CloseHandle(handle)


@unittest.skipUnless(WINDOWS, "the Windows forms run on Windows")
class JobOwnership(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp(prefix="aclw-"))
        self.addCleanup(remove_tree, self.folder)
        self.owner = OwnedProcesses()
        self.addCleanup(self.close_owner)

    def close_owner(self):
        try:
            self.owner.close()
        except ProcessError:
            pass

    def python(self, code):
        return [sys.executable, "-c", textwrap.dedent(code)]

    def test_an_owner_that_dies_without_cleaning_up_takes_the_tree_with_it(self):
        # The job's only handle is the owner's: a child that inherited it
        # would keep the job open and the tree alive after the owner died.
        pids = self.folder / "pids.json"
        owner = subprocess.Popen(self.python(f"""
            import json, os, sys, time
            from pathlib import Path
            sys.path.insert(0, {str(SCRIPTS)!r})
            from agent_live_check.processes import OwnedProcesses
            grandchild = "import time; time.sleep(600)"
            child = OwnedProcesses().spawn([sys.executable, "-c",
                "import subprocess, sys, time; "
                f"p = subprocess.Popen([sys.executable, '-c', {{grandchild!r}}]); "
                "print(p.pid, flush=True); time.sleep(600)"], env=dict(os.environ))
            grandchild_pid = int(child.stdout.readline())
            Path({str(pids)!r}).write_text(json.dumps([child.pid, grandchild_pid]))
            time.sleep(600)
        """), stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            end = time.monotonic() + 30
            while not pids.exists():
                self.assertIsNone(owner.poll(), owner.stderr.read().decode(errors="replace") if owner.poll() is not None else "")
                self.assertLess(time.monotonic(), end, "the owner never started its tree")
                time.sleep(0.05)
            child, grandchild = json.loads(pids.read_text())
            self.assertFalse(wait_ended(child, 0), "the child ended before its owner")
            self.assertFalse(wait_ended(grandchild, 0), "the grandchild ended before its owner")
            owner.kill()
            owner.wait(timeout=WAIT_SECONDS)
            self.assertTrue(wait_ended(child, WAIT_SECONDS), "the child outlived its owner")
            self.assertTrue(wait_ended(grandchild, WAIT_SECONDS), "the grandchild outlived its owner")
        finally:
            owner.kill()
            owner.wait(timeout=WAIT_SECONDS)
            owner.stderr.close()

    def test_the_caps_are_the_unix_guardians(self):
        self.assertEqual((processes.MAX_CHILDREN, processes.MAX_DESCENDANTS, processes.MAX_RSS_BYTES,
                          processes.MAX_OUTPUT, processes.COMMAND_SECONDS, processes.RUN_SECONDS),
                         (16, 128, 2 * 1024 ** 3, 1024 ** 2, 15, 30 * 60))

    def test_a_process_past_the_descendant_cap_is_a_named_failure(self):
        with patch.object(processes, "MAX_DESCENDANTS", 3):
            self.owner = OwnedProcesses()
            with self.assertRaisesRegex(ProcessSafetyError, r'owned_processes_over_budget:\{"descendants":3\}'):
                # The job refuses the fourth process; only the job's end, not
                # the child, ends the run.
                self.owner.run(self.python("""
                    import subprocess, sys, time
                    for _ in range(5):
                        try:
                            subprocess.Popen([sys.executable, "-c", "import time; time.sleep(600)"])
                        except OSError:
                            pass
                    time.sleep(600)
                """), env=dict(os.environ), check=False)

    def test_resident_memory_past_the_cap_is_a_named_failure(self):
        cap = 256 * 1024 * 1024
        with patch.object(processes, "MAX_RSS_BYTES", cap):
            self.owner = OwnedProcesses()
            with self.assertRaises(ProcessSafetyError) as raised:
                # Every page written, so it is resident, as the Unix guardian counts RSS.
                self.owner.run(self.python(f"""
                    import time
                    held = b"x" * {2 * cap}
                    time.sleep(600)
                """), env=dict(os.environ), check=False)
        reason, _, measured = str(raised.exception).partition(":")
        self.assertEqual(reason, "owned_processes_over_budget")
        measured = json.loads(measured)
        self.assertGreater(measured["rss_bytes"], cap)
        self.assertEqual(measured["largest"][0]["executable"].lower(), Path(sys.executable).name.lower())

    def test_memory_committed_past_the_hard_stop_is_a_named_failure(self):
        committed = 256 * 1024 * 1024
        with patch.object(processes, "MAX_COMMITTED_BYTES", committed):
            self.owner = OwnedProcesses()
            with self.assertRaisesRegex(ProcessSafetyError,
                                        rf'owned_processes_over_budget:\{{"committed_bytes":{committed}\}}'):
                # Refused by Windows at the allocation, before any of it is resident.
                self.owner.run(self.python(f"""
                    import time
                    try:
                        held = bytearray({2 * committed})
                    except MemoryError:
                        pass
                    time.sleep(600)
                """), env=dict(os.environ), check=False)

    def test_memory_committed_but_never_touched_is_within_the_cap(self):
        cap = 256 * 1024 * 1024
        with patch.object(processes, "MAX_RSS_BYTES", cap):
            self.owner = OwnedProcesses()
            # Committed and never written: Windows charges it, RSS does not hold it.
            code, _, stderr = self.owner.run(self.python(f"""
                import ctypes, time
                VirtualAlloc = ctypes.windll.kernel32.VirtualAlloc
                VirtualAlloc.restype = ctypes.c_void_p
                VirtualAlloc.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_ulong, ctypes.c_ulong]
                if not VirtualAlloc(None, {2 * cap}, 0x1000, 0x04):  # MEM_COMMIT, PAGE_READWRITE
                    raise SystemExit("VirtualAlloc failed")
                time.sleep(1)
            """), env=dict(os.environ), check=False)
        self.assertEqual(code, 0, stderr)

    def test_output_past_the_cap_ends_the_command(self):
        with self.assertRaisesRegex(ProcessError, "command_output_over_budget"):
            self.owner.run(self.python(f"""
                import sys, time
                sys.stdout.write("x" * {processes.MAX_OUTPUT + 1})
                sys.stdout.flush()
                time.sleep(600)
            """), env=dict(os.environ))
        self.assertEqual(self.owner.children, {})

    def test_a_command_past_its_time_ends_with_its_tree(self):
        pid = self.folder / "pid"
        started = time.monotonic()
        with self.assertRaisesRegex(ProcessError, "command_timeout"):
            self.owner.run(self.python(f"""
                import subprocess, sys, time
                from pathlib import Path
                p = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(600)"])
                Path({str(pid)!r}).write_text(str(p.pid))
                time.sleep(600)
            """), env=dict(os.environ), seconds=3)
        self.assertLess(time.monotonic() - started, 3 + processes.END_SECONDS + 2)
        self.assertTrue(wait_ended(int(pid.read_text()), WAIT_SECONDS), "the timed-out command's child survived")


@unittest.skipUnless(WINDOWS, "the Windows forms run on Windows")
class PrivateFiles(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp(prefix="aclw-"))
        self.addCleanup(remove_tree, self.folder)

    def test_a_private_folder_and_file_admit_only_this_account(self):
        folder = self.folder / "run/home"
        private_directory(folder)
        self.assertTrue(private(folder))
        self.assertTrue(private(self.folder / "run"))
        write_private(folder / "settings.json", b"{}\n")
        self.assertTrue(private(folder / "settings.json"))
        self.assertEqual((folder / "settings.json").read_bytes(), b"{}\n")
        with self.assertRaises(FileExistsError):
            write_private(folder / "settings.json", b"replaced\n")

    def test_access_for_another_account_is_not_private(self):
        private_directory(self.folder / "run")
        file = self.folder / "run/key"
        write_private(file, b"secret\n")
        # Everyone (S-1-1-0) may read it.
        subprocess.run(["icacls", str(file), "/grant", "*S-1-1-0:R"], check=True, capture_output=True)
        self.assertFalse(private(file))


@unittest.skipUnless(WINDOWS, "the Windows forms run on Windows")
class ShellForms(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp(prefix="aclw-"))
        self.addCleanup(remove_tree, self.folder)

    def cmd(self, line):
        """`line` run by cmd.exe as a pane's prompt would run it: passed whole,
        which a list's quoting would not."""
        shell = os.environ["ComSpec"]
        return subprocess.run(f'"{shell}" /d /s /c "{line}"', capture_output=True, timeout=60)

    def test_a_pane_command_keeps_its_output_and_exit_status(self):
        output, error, status = self.folder / "pane out", self.folder / "pane err", self.folder / "status"
        program = [sys.executable, "-c", "import sys; print(sys.argv[1:]); sys.stderr.write('noise'); sys.exit(3)",
                   "two words", "it's"]
        ran = self.cmd(PaneShell().program(program, output, error, status))
        self.assertEqual(ran.stderr, b"")
        self.assertEqual(output.read_text().strip(), str(["two words", "it's"]))
        self.assertEqual(error.read_text(), "noise")
        self.assertEqual(status.read_text().strip(), "3")

    def test_a_variable_read_from_a_file_reaches_what_the_shell_starts(self):
        source, ready = self.folder / "reference", self.folder / "ready"
        source.write_bytes(b"C:/private/pane capabilities/reference.json")
        ran = self.cmd(PaneShell().read_variable("HIDE_CAP_REF", source, ready)
                       + f' & "{sys.executable}" -c "import os; print(os.environ[\'HIDE_CAP_REF\'])"')
        self.assertEqual(ran.returncode, 0, ran.stderr)
        self.assertEqual(ran.stdout.decode().strip(), "C:/private/pane capabilities/reference.json")
        self.assertEqual(ready.read_text().strip(), "ready")

    def test_a_readable_completion_marker_is_released_after_its_writer_closes(self):
        import ctypes
        from ctypes import wintypes
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                      wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        marker = self.folder / "reference-ready"
        marker.write_text("ready")
        # Read sharing, without delete sharing, as cmd's redirect can hold
        # after its completion bytes are visible to the controller.
        handle = kernel.CreateFileW(str(marker), 0x40000000, 1, None, 3, 0, None)
        self.assertNotEqual(handle, ctypes.c_void_p(-1).value)
        unlink = os.unlink

        def release_after_refusal(*args, **kwargs):
            nonlocal handle
            try:
                return unlink(*args, **kwargs)
            except PermissionError:
                kernel.CloseHandle(handle)
                handle = None
                raise
        try:
            self.assertEqual(marker.read_text(), "ready")
            with patch("os.unlink", release_after_refusal):
                remove_pane_files(marker)
            self.assertFalse(marker.exists())
        finally:
            if handle is not None:
                kernel.CloseHandle(handle)

    def test_the_pane_shell_is_the_one_herdr_is_told_to_start(self):
        config = PaneShell().herdr_config()
        self.assertEqual(config, f"[terminal]\ndefault_shell = {json.dumps(os.environ['ComSpec'])}\n")
        self.assertEqual(PaneShell().environment(self.folder), {"PROMPT": "LIVE_CHECK_READY$G "})

    def test_a_launcher_sets_its_environment_and_passes_arguments_and_status(self):
        private_directory(self.folder / "bin")
        program = launcher(self.folder / "bin/agent", [sys.executable, "-c",
                                                       "import os, sys; print(os.environ['ROUTE'], os.environ.get('UNSET'), "
                                                       "sys.argv[1:]); sys.exit(7)"],
                           environment={"ROUTE": "a b"}, unset=["UNSET"])
        self.assertEqual(program.suffix, ".cmd")
        ran = subprocess.run([str(program), "one", "two words"], capture_output=True, timeout=60,
                             env={**os.environ, "UNSET": "operator value"})
        self.assertEqual(ran.returncode, 7, ran.stderr)
        self.assertEqual(ran.stdout.decode().strip(), "a b None " + str(["one", "two words"]))

    def test_a_launcher_refuses_what_cmd_would_reinterpret(self):
        private_directory(self.folder / "bin")
        for environment, argv in (({"ROUTE": "100%"}, [sys.executable]), ({}, [sys.executable, 'say "hi"'])):
            with self.assertRaisesRegex(ValueError, "cmd.exe cannot carry"):
                launcher(self.folder / "bin/agent", argv, environment=environment)

    def test_a_path_hide_spells_with_slashes_names_the_same_folder(self):
        folder = self.folder / "Worktrees" / "codex"
        self.assertTrue(same_path(folder.as_posix(), folder))
        self.assertTrue(same_path(str(folder).lower(), folder))
        self.assertFalse(same_path((self.folder / "worktrees/claude").as_posix(), folder))
        self.assertFalse(same_path(None, folder))

    def test_a_tree_with_read_only_files_is_removed(self):
        folder = self.folder / "checkout/.git/objects"
        folder.mkdir(parents=True)
        (folder / "object").write_bytes(b"blob")
        (folder / "object").chmod(0o444)
        remove_tree(self.folder / "checkout")
        self.assertFalse((self.folder / "checkout").exists())


@unittest.skipUnless(WINDOWS, "the Windows forms run on Windows")
class UserPathHold(unittest.TestCase):
    """The account's Path in the registry, where a Herdr pane reads its PATH."""

    def read(self):
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment") as key:
            try:
                return winreg.QueryValueEx(key, "Path")
            except FileNotFoundError:
                return None

    def write(self, value):
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0, winreg.KEY_SET_VALUE) as key:
            if value is None:
                winreg.DeleteValue(key, "Path")
            else:
                winreg.SetValueEx(key, "Path", 0, value[1], value[0])

    def test_the_run_folder_comes_first_and_leaves_what_changed_meanwhile(self):
        from agent_live_check.windows import UserPath
        before = self.read()
        self.addCleanup(self.write, before)
        folder = Path(tempfile.gettempdir()) / "journey-run/bin"
        held = UserPath(folder)
        value, kind = self.read()
        self.assertEqual(value.split(";")[0], str(folder))
        # An installer adds its own folder while the run holds the Path.
        self.write((value + ";C:\\installed\\bin", kind))
        held.close()
        expected = ";".join(filter(None, [before[0] if before else "", "C:\\installed\\bin"]))
        self.assertEqual(self.read(), (expected, before[1] if before else kind))

    def test_it_is_refused_outside_a_ci_runner(self):
        from agent_live_check.protection import ProtectionError
        from agent_live_check.windows import UserPath
        before = self.read()
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "false"}):
            with self.assertRaisesRegex(ProtectionError, "only_on_a_ci_runner"):
                UserPath(Path(tempfile.gettempdir()) / "journey-run/bin")
        self.assertEqual(self.read(), before)

    def test_a_path_that_did_not_exist_is_removed_again(self):
        from agent_live_check.windows import UserPath
        before = self.read()
        self.addCleanup(self.write, before)
        if before is not None:
            self.write(None)
        UserPath(Path(tempfile.gettempdir()) / "journey-run/bin").close()
        self.assertIsNone(self.read())


if __name__ == "__main__":
    unittest.main()
