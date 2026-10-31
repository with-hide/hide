#!/usr/bin/env python3
"""Runs one build command and reports its wall time and peak memory as JSON.

Usage: measure-build.py <label> -- <command> [args...]

Peak memory is the largest single process the command started, so a build
whose Zig compiler peaks reads as Zig's peak, on every system: on macOS and
Linux the kernel's maximum resident set of any waited-for descendant
(`getrusage(RUSAGE_CHILDREN)`), on Windows the largest committed memory of any
process in a job object the command runs in. The vt lane prints one line per
build; docs/BUILD.md, "The terminal parser and Zig", cites what it measured.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import time


def run_windows(command: list[str]) -> tuple[int, int]:
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    class IoCounters(ctypes.Structure):
        _fields_ = [(name, ctypes.c_ulonglong) for name in (
            "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
            "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]

    class BasicLimits(ctypes.Structure):
        _fields_ = [
            ("PerProcessUserTimeLimit", ctypes.c_longlong), ("PerJobUserTimeLimit", ctypes.c_longlong),
            ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
            ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
            ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD), ("SchedulingClass", wintypes.DWORD),
        ]

    class ExtendedLimits(ctypes.Structure):
        _fields_ = [
            ("BasicLimitInformation", BasicLimits), ("IoInfo", IoCounters),
            ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
            ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t),
        ]

    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.OpenProcess.restype = wintypes.HANDLE
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise OSError(ctypes.get_last_error(), "CreateJobObjectW failed")
    process = subprocess.Popen(command)
    handle = kernel32.OpenProcess(0x0100 | 0x0001, False, process.pid)  # SET_QUOTA | TERMINATE
    # Processes the command starts after this join the job with it; cargo takes
    # far longer than this call to start its first compiler.
    if not handle or not kernel32.AssignProcessToJobObject(wintypes.HANDLE(job), wintypes.HANDLE(handle)):
        raise OSError(ctypes.get_last_error(), "AssignProcessToJobObject failed")
    status = process.wait()
    limits = ExtendedLimits()
    if not kernel32.QueryInformationJobObject(wintypes.HANDLE(job), 9, ctypes.byref(limits), ctypes.sizeof(limits), None):
        raise OSError(ctypes.get_last_error(), "QueryInformationJobObject failed")
    kernel32.CloseHandle(wintypes.HANDLE(handle))
    kernel32.CloseHandle(wintypes.HANDLE(job))
    return status, int(limits.PeakProcessMemoryUsed)


def run_unix(command: list[str]) -> tuple[int, int]:
    import resource

    status = subprocess.run(command).returncode
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    # Linux reports kibibytes, macOS bytes.
    return status, peak if sys.platform == "darwin" else peak * 1024


def main() -> int:
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    label, command = sys.argv[1], sys.argv[3:]
    # Windows starts a bare name from System32 before PATH, where `bash` is
    # the WSL launcher, not the Git Bash the caller runs in.
    program = shutil.which(command[0])
    if program is None:
        print(f"measure-build: {command[0]} is not on PATH", file=sys.stderr)
        return 127
    command = [program, *command[1:]]
    start = time.monotonic()
    status, peak = (run_windows if sys.platform == "win32" else run_unix)(command)
    seconds = time.monotonic() - start
    print(json.dumps({"measure": label, "status": status, "seconds": round(seconds, 1),
                      "peak_mib": round(peak / (1 << 20))}))
    return status


if __name__ == "__main__":
    sys.exit(main())
