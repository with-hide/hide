"""Runs a command in a Windows job that lets a process leave it, and waits for it.

A GitHub runner's step runs in a job that forbids breakaway. Node and Electron
start their children in a libuv job that lets them leave silently, so the
daemon the desktop app's `hide connect` starts lands in the runner's job, and
every pane holder it starts is refused (`hide_platform::process::start_resident`,
docs/ARCHITECTURE.md, The pane holder). An operator's desktop session puts the
app in no such job. This job stands between the runner's and the command's
tree, as that session does: it allows breakaway and nothing else, so the
daemon stays in it and its holders may leave it.

Usage: python scripts/windows-breakaway-job.py -- <program> [args...]
The command's exit status is this script's.
"""

import ctypes
import subprocess
import sys
from ctypes import wintypes

JOB_OBJECT_LIMIT_BREAKAWAY_OK = 0x0000_0800
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9
CREATE_SUSPENDED = 0x0000_0004


class IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_ulonglong) for name in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]


class BasicLimits(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_longlong),
        ("PerJobUserTimeLimit", ctypes.c_longlong),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class ExtendedLimits(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", BasicLimits),
        ("IoInfo", IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


def checked(result, what):
    if not result:
        raise OSError(ctypes.get_last_error(), f"{what} failed: {ctypes.FormatError(ctypes.get_last_error())}")
    return result


def main(argv):
    if sys.platform != "win32":
        sys.exit("windows-breakaway-job.py runs on Windows only")
    if argv[:1] != ["--"] or len(argv) < 2:
        sys.exit(__doc__.strip().splitlines()[-2])
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll")
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]

    job = checked(kernel32.CreateJobObjectW(None, None), "CreateJobObjectW")
    limits = ExtendedLimits()
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_BREAKAWAY_OK
    checked(kernel32.SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                                             ctypes.byref(limits), ctypes.sizeof(limits)),
            "SetInformationJobObject")
    # Suspended until it is in the job, so no child it starts escapes it.
    child = subprocess.Popen(argv[1:], creationflags=CREATE_SUSPENDED)
    try:
        checked(kernel32.AssignProcessToJobObject(job, int(child._handle)), "AssignProcessToJobObject")
    except OSError:
        child.kill()
        raise
    status = ntdll.NtResumeProcess(int(child._handle))
    if status != 0:
        child.kill()
        raise OSError(f"NtResumeProcess failed with status {status:#x}")
    return child.wait()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
