"""What a pane holder check started, ended on every path by pid and start
time (docs/TESTING.md, The holder restart lane).

The restart lane, the holder measurement and the color probe start a node,
the holders it starts, CLIs and a private Herdr server. Each one is put in an
`Owned` the moment it starts: a child the script started as the child it
started, a holder by the pane record its node wrote (`holders/records/*.json`
under the state folder, which names the holder and the pane's program with
the start times `hide_platform::process::start_time` read), and everything
running under any of them by one look at the process table before the end.
`close` ends each one by itself, so one that cannot be ended does not keep
the others running, waits until none of them runs, and only then removes the
folders; what still runs is answered, never skipped.

`abort_on_signals` turns SIGINT and SIGTERM (Ctrl-Break on Windows) into
`Aborted` in the main thread, so the `finally` that closes an `Owned` runs on
a cancelled run as on any other end; a second signal during that close is
not raised again.
"""

import ctypes
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))

from agent_live_check.system import STOP_SIGNALS, WINDOWS  # noqa: E402

if not WINDOWS:
    from agent_live_check.process_table import BsdInfo, descendants, snapshot  # noqa: E402

# How long the ends may take to be confirmed.
END_SECONDS = 20

_aborted = threading.Event()


class Aborted(BaseException):
    """A stop signal: a BaseException, so no `except Exception` of a check
    takes it for one of its own failures."""

    def __init__(self, signum):
        super().__init__(f"stopped by signal {signum}")
        self.signum = signum


def abort_on_signals():
    """Raises `Aborted` in the main thread at the first stop signal."""
    def stop(signum, _frame):
        if _aborted.is_set():
            return
        _aborted.set()
        raise Aborted(signum)
    for signum in STOP_SIGNALS:
        signal.signal(signum, stop)


def aborted():
    """Whether a stop signal came: a polite end that waits is skipped then."""
    return _aborted.is_set()


# --- one process, by pid and start time --------------------------------------

if WINDOWS:
    from ctypes import wintypes

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.OpenProcess.restype = wintypes.HANDLE
    _kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _kernel32.GetProcessTimes.argtypes = [wintypes.HANDLE, *[ctypes.POINTER(wintypes.FILETIME)] * 4]
    _kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    _kernel32.WaitForSingleObject.restype = wintypes.DWORD
    _kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    _kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    _kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    _kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    QUERY, TERMINATE, SYNCHRONIZE = 0x1000, 0x0001, 0x00100000
    INVALID_HANDLE = wintypes.HANDLE(-1).value

    class _Entry(ctypes.Structure):
        _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ProcessID", wintypes.DWORD),
                    ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", wintypes.DWORD),
                    ("cntThreads", wintypes.DWORD), ("th32ParentProcessID", wintypes.DWORD),
                    ("pcPriClassBase", ctypes.c_long), ("dwFlags", wintypes.DWORD),
                    ("szExeFile", ctypes.c_wchar * 260)]

    _kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_Entry)]
    _kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_Entry)]

    def _open(pid, access):
        """A handle to `pid` while it runs, and its start time read through
        that same handle, so what is ended is what was read; None once it
        ended or was never there."""
        handle = _kernel32.OpenProcess(access | QUERY | SYNCHRONIZE, False, pid)
        if not handle:
            return None
        if _kernel32.WaitForSingleObject(handle, 0) == 0:
            _kernel32.CloseHandle(handle)
            return None
        times = [wintypes.FILETIME() for _ in range(4)]
        if not _kernel32.GetProcessTimes(handle, *map(ctypes.byref, times)):
            _kernel32.CloseHandle(handle)
            raise OSError(ctypes.get_last_error(), f"GetProcessTimes({pid})")
        return handle, (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime

    def started(pid):
        opened = _open(pid, 0)
        if opened is None:
            return None
        _kernel32.CloseHandle(opened[0])
        return opened[1]

    def _end(pid, birth):
        opened = _open(pid, TERMINATE)
        if opened is None:
            return
        handle, now = opened
        try:
            if now == birth and not _kernel32.TerminateProcess(handle, 1):
                raise OSError(ctypes.get_last_error(), f"TerminateProcess({pid})")
        finally:
            _kernel32.CloseHandle(handle)

    def _table():
        """{pid: (parent, start time)} of every process that runs."""
        shot = _kernel32.CreateToolhelp32Snapshot(0x2, 0)
        if shot == INVALID_HANDLE:
            raise OSError(ctypes.get_last_error(), "CreateToolhelp32Snapshot")
        found = {}
        try:
            entry = _Entry()
            entry.dwSize = ctypes.sizeof(_Entry)
            more = _kernel32.Process32FirstW(shot, ctypes.byref(entry))
            while more:
                birth = started(entry.th32ProcessID)
                if birth is not None:
                    found[entry.th32ProcessID] = (entry.th32ParentProcessID, birth)
                more = _kernel32.Process32NextW(shot, ctypes.byref(entry))
        finally:
            _kernel32.CloseHandle(shot)
        return found

    def _under(roots):
        """Every process under `roots` ({pid: start time}): a child is one
        whose parent column names the pid and that started after it, since
        Windows keeps a parent's pid after the parent ended."""
        table = _table()
        found = dict(roots)
        while True:
            added = {pid: birth for pid, (parent, birth) in table.items()
                     if pid not in found and parent in found and birth >= found[parent]}
            if not added:
                return found
            found.update(added)
else:
    if sys.platform == "darwin":
        _libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        _libproc.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]

    def started(pid):
        """`pid`'s start time as `hide_platform::process::start_time` reads
        it (microseconds on macOS, clock ticks after boot on Linux); None once
        it ended or was never there, or is a zombie, which runs nothing."""
        if sys.platform == "darwin":
            info = BsdInfo()
            if _libproc.proc_pidinfo(pid, 3, 1, ctypes.byref(info), ctypes.sizeof(info)) != ctypes.sizeof(info):
                return None
            return None if info.status == 5 else info.sec * 1_000_000 + info.usec
        try:
            raw = Path(f"/proc/{pid}/stat").read_text()
        except (FileNotFoundError, ProcessLookupError):
            return None
        fields = raw[raw.rindex(")") + 2:].split()
        return None if fields[0] == "Z" else int(fields[19])

    def _end(pid, birth):
        if started(pid) == birth:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def _under(roots):
        table = snapshot()
        found = dict(roots)
        for pid, birth in roots.items():
            if pid in table and table[pid].birth == birth:
                found.update({item.pid: item.birth for item in descendants(table, pid).values() if not item.zombie})
        return found


def runs(pid, birth):
    """Whether `pid` still runs as the process that started at `birth`."""
    return started(pid) == birth


# --- what a check started ----------------------------------------------------

class Owned:
    """Children, holders and folders a check started, ended on `close`."""

    def __init__(self):
        self.children = []
        self.known = {}
        self.states = []
        self.folders = []

    def spawn(self, argv, **kwargs):
        """`subprocess.Popen`, put in the list before anything else runs."""
        child = subprocess.Popen(argv, **kwargs)
        self.children.append(child)
        return child

    def adopt(self, pid, birth):
        """A process this check started some other way, such as a holder its
        node answered with."""
        self.known[int(pid)] = int(birth)

    def records(self, state):
        """A state folder whose pane records name holders to end."""
        self.states.append(Path(state))

    def remove(self, folder):
        """A folder removed once nothing of the check runs."""
        self.folders.append(Path(folder))

    def _recorded(self):
        found, unreadable = {}, []
        for state in self.states:
            folder = state / "holders/records"
            if not folder.is_dir():
                continue
            for path in folder.glob("*.json"):
                try:
                    record = json.loads(path.read_text())
                except (OSError, ValueError) as error:
                    unreadable.append(f"{path.name}: {error}")
                    continue
                for key in ("holder", "program"):
                    identity = record.get(key)
                    if isinstance(identity, dict):
                        found[int(identity["pid"])] = int(identity["started"])
        return found, unreadable

    def _roots(self):
        roots, left = dict(self.known), []
        recorded, unreadable = self._recorded()
        roots.update(recorded)
        left.extend(f"a pane record could not be read: {item}" for item in unreadable)
        for child in self.children:
            if child.poll() is None:
                birth = started(child.pid)
                if birth is not None:
                    roots[child.pid] = birth
        return roots, left

    def close(self, seconds=END_SECONDS):
        """Ends everything; answers what still runs, and what could not be
        read or removed. Two looks, so a holder its node was starting while
        the first look ran is found by the second."""
        left = []
        ending = {}
        for _ in range(2):
            roots, unreadable = self._roots()
            try:
                found = _under(roots)
            except (OSError, RuntimeError) as error:
                left.append(f"the process table could not be read: {error}")
                found = roots
            ending.update(found)
            for child in self.children:
                if child.poll() is None:
                    try:
                        child.kill()
                    except OSError as error:
                        left.append(f"{child.pid} could not be ended: {error}")
            for pid, birth in found.items():
                try:
                    _end(pid, birth)
                except (OSError, RuntimeError) as error:
                    left.append(f"{pid} could not be ended: {error}")
            left.extend(item for item in unreadable if item not in left)
        deadline = time.monotonic() + seconds
        for child in self.children:
            try:
                child.wait(max(0.1, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                pass
            for stream in (child.stdin, child.stdout, child.stderr):
                if stream is not None:
                    try:
                        stream.close()
                    except OSError:
                        pass
        running = dict(ending)
        while running and time.monotonic() < deadline:
            running = {pid: birth for pid, birth in running.items() if runs(pid, birth)}
            if running:
                time.sleep(0.1)
        running = {pid: birth for pid, birth in running.items() if runs(pid, birth)}
        left.extend(f"{pid} (started {birth}) still runs" for pid, birth in sorted(running.items()))
        left.extend(f"{child.pid} still runs" for child in self.children if child.poll() is None)
        if left:
            # A folder a running process holds stays, for the report to name.
            return left
        for folder in self.folders:
            try:
                shutil.rmtree(folder)
            except FileNotFoundError:
                pass
            except OSError as error:
                left.append(f"{folder} could not be removed: {error}")
        return left
