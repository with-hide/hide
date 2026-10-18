"""What the live check needs from Windows itself: a job that owns a child's
whole process tree, and files only this account can open.

The job follows hide-platform's `process` (Windows): the child starts
suspended, joins its job, and only then runs, so nothing it starts is ever
outside the job; the job ends everything in it when its last handle closes
(`KILL_ON_JOB_CLOSE`), so an owner that dies without cleaning up still takes
the tree with it. The handle is never inheritable and no child is given it.
The private files follow hide-platform's `fs::private` (Windows): created
with an access list naming only this account, and checked by owner and
access list. The account's Path in the registry is where a pane Herdr opens
reads its PATH (`UserPath`). Imported only on Windows.
"""

import ctypes
from ctypes import wintypes
import msvcrt
import os
import threading
import time
from typing import NamedTuple
import winreg

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)

HANDLE = wintypes.HANDLE
INVALID_HANDLE_VALUE = HANDLE(-1).value
INFINITE = 0xFFFFFFFF
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 258
ERROR_INVALID_PARAMETER = 87

CREATE_SUSPENDED = 0x00000004
CREATE_NEW_PROCESS_GROUP = 0x00000200
CREATE_NO_WINDOW = 0x08000000

PROCESS_TERMINATE = 0x0001
PROCESS_SET_QUOTA = 0x0100
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
THREAD_SUSPEND_RESUME = 0x0002
TH32CS_SNAPTHREAD = 0x00000004

JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 0x00000008
JOB_OBJECT_LIMIT_JOB_MEMORY = 0x00000200
JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION = 0x00000400
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JOB_BASIC_ACCOUNTING = 1
JOB_BASIC_PROCESS_ID_LIST = 3
JOB_ASSOCIATE_COMPLETION_PORT = 7
JOB_EXTENDED_LIMIT = 9
JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT = 3
JOB_OBJECT_MSG_JOB_MEMORY_LIMIT = 10


def failed(call):
    """The last Windows error as the OSError subclass Python maps it to
    (FileExistsError for an existing name, as os.open's O_EXCL raises)."""
    error = ctypes.get_last_error()
    return ctypes.WinError(error, f"{call}: {ctypes.FormatError(error).strip()}")


class BasicLimit(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD), ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t), ("PriorityClass", wintypes.DWORD),
                ("SchedulingClass", wintypes.DWORD)]


class IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]


class ExtendedLimit(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", BasicLimit), ("IoInfo", IoCounters),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]


class BasicAccounting(ctypes.Structure):
    _fields_ = [("TotalUserTime", ctypes.c_int64), ("TotalKernelTime", ctypes.c_int64),
                ("ThisPeriodTotalUserTime", ctypes.c_int64), ("ThisPeriodTotalKernelTime", ctypes.c_int64),
                ("TotalPageFaultCount", wintypes.DWORD), ("TotalProcesses", wintypes.DWORD),
                ("ActiveProcesses", wintypes.DWORD), ("TotalTerminatedProcesses", wintypes.DWORD)]


class MemoryCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [(name, ctypes.c_size_t) for name in (
        "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage", "QuotaPagedPoolUsage",
        "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage")]


class CompletionPortLink(ctypes.Structure):
    _fields_ = [("CompletionKey", ctypes.c_void_p), ("CompletionPort", HANDLE)]


class ThreadEntry(ctypes.Structure):
    _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ThreadID", wintypes.DWORD),
                ("th32OwnerProcessID", wintypes.DWORD), ("tpBasePri", wintypes.LONG),
                ("tpDeltaPri", wintypes.LONG), ("dwFlags", wintypes.DWORD)]


def declare(library, name, result, *arguments):
    function = getattr(library, name)
    function.restype, function.argtypes = result, list(arguments)
    return function


CloseHandle = declare(kernel32, "CloseHandle", wintypes.BOOL, HANDLE)
CreateJobObjectW = declare(kernel32, "CreateJobObjectW", HANDLE, ctypes.c_void_p, wintypes.LPCWSTR)
SetInformationJobObject = declare(kernel32, "SetInformationJobObject", wintypes.BOOL,
                                  HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD)
QueryInformationJobObject = declare(kernel32, "QueryInformationJobObject", wintypes.BOOL,
                                    HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p)
AssignProcessToJobObject = declare(kernel32, "AssignProcessToJobObject", wintypes.BOOL, HANDLE, HANDLE)
TerminateJobObject = declare(kernel32, "TerminateJobObject", wintypes.BOOL, HANDLE, wintypes.UINT)
OpenProcess = declare(kernel32, "OpenProcess", HANDLE, wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
OpenThread = declare(kernel32, "OpenThread", HANDLE, wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
ResumeThread = declare(kernel32, "ResumeThread", wintypes.DWORD, HANDLE)
CreateToolhelp32Snapshot = declare(kernel32, "CreateToolhelp32Snapshot", HANDLE, wintypes.DWORD, wintypes.DWORD)
Thread32First = declare(kernel32, "Thread32First", wintypes.BOOL, HANDLE, ctypes.POINTER(ThreadEntry))
Thread32Next = declare(kernel32, "Thread32Next", wintypes.BOOL, HANDLE, ctypes.POINTER(ThreadEntry))
CreateIoCompletionPort = declare(kernel32, "CreateIoCompletionPort", HANDLE,
                                 HANDLE, HANDLE, ctypes.c_size_t, wintypes.DWORD)
GetQueuedCompletionStatus = declare(kernel32, "GetQueuedCompletionStatus", wintypes.BOOL, HANDLE,
                                    ctypes.POINTER(wintypes.DWORD), ctypes.POINTER(ctypes.c_size_t),
                                    ctypes.POINTER(ctypes.c_void_p), wintypes.DWORD)
PostQueuedCompletionStatus = declare(kernel32, "PostQueuedCompletionStatus", wintypes.BOOL,
                                     HANDLE, wintypes.DWORD, ctypes.c_size_t, ctypes.c_void_p)
GetConsoleCP = declare(kernel32, "GetConsoleCP", wintypes.UINT)
K32GetProcessMemoryInfo = declare(kernel32, "K32GetProcessMemoryInfo", wintypes.BOOL,
                                  HANDLE, ctypes.POINTER(MemoryCounters), wintypes.DWORD)
QueryFullProcessImageNameW = declare(kernel32, "QueryFullProcessImageNameW", wintypes.BOOL,
                                     HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD))


def process_name(pid):
    """The file name of `pid`'s executable, or "" when Windows does not give
    it (it refuses some processes even this account starts), as the Unix
    table leaves a name it could not read: a name only labels a failure."""
    process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not process:
        return ""
    try:
        image, length = ctypes.create_unicode_buffer(32768), wintypes.DWORD(32768)
        if not QueryFullProcessImageNameW(process, 0, image, ctypes.byref(length)):
            return ""
        return os.path.basename(image.value)
    finally:
        CloseHandle(process)


class Resident(NamedTuple):
    """A process of a job and the memory it keeps resident, read as the
    Unix process table's entries are. A Windows process is never a zombie: one
    that ended is no longer listed by its job."""
    name: str
    rss: int
    zombie: bool = False


def console_flags():
    """What a child is started with besides suspended, as hide-platform
    starts one: its own process group, and a console without a window when
    this process has no console to share (a child would otherwise open a
    visible window)."""
    return CREATE_NEW_PROCESS_GROUP | (CREATE_NO_WINDOW if GetConsoleCP() == 0 else 0)


class Job:
    """One child's process tree. `processes` and `committed` (bytes of
    committed memory) are the job's own limits; a process or an allocation
    past them is refused by Windows, and the refusal arrives at `port` named
    by `key`. Its resident memory is measured from outside (`working_sets`)."""

    def __init__(self, port, key, *, processes, committed):
        # No security attributes: the handle is not inheritable.
        handle = CreateJobObjectW(None, None)
        if not handle:
            raise failed("CreateJobObjectW")
        self.handle, self.key, self.failure = handle, key, None

        class ProcessIds(ctypes.Structure):
            _fields_ = [("NumberOfAssignedProcesses", wintypes.DWORD), ("NumberOfProcessIdsInList", wintypes.DWORD),
                        ("ProcessIdList", ctypes.c_size_t * processes)]

        self.ids = ProcessIds()
        try:
            limits = ExtendedLimit()
            limits.BasicLimitInformation.LimitFlags = (
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION)
            limits.BasicLimitInformation.ActiveProcessLimit = processes
            limits.JobMemoryLimit = committed
            if not SetInformationJobObject(handle, JOB_EXTENDED_LIMIT, ctypes.byref(limits), ctypes.sizeof(limits)):
                raise failed("SetInformationJobObject(limits)")
            link = CompletionPortLink(key, port)
            if not SetInformationJobObject(handle, JOB_ASSOCIATE_COMPLETION_PORT, ctypes.byref(link),
                                           ctypes.sizeof(link)):
                raise failed("SetInformationJobObject(port)")
        except BaseException:
            self.close()
            raise

    def adopt(self, pid):
        """Puts the suspended process `pid` in the job, then lets it run."""
        process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, False, pid)
        if not process:
            raise failed("OpenProcess")
        try:
            if not AssignProcessToJobObject(self.handle, process):
                raise failed("AssignProcessToJobObject")
        finally:
            CloseHandle(process)
        resume(pid)

    def active(self):
        """How many processes the job holds now."""
        accounting = BasicAccounting()
        if not QueryInformationJobObject(self.handle, JOB_BASIC_ACCOUNTING, ctypes.byref(accounting),
                                         ctypes.sizeof(accounting), None):
            raise failed("QueryInformationJobObject")
        return accounting.ActiveProcesses

    def working_sets(self):
        """The resident memory (working set) in bytes of each process the job
        holds now, by pid; a process that ended since the job listed it is
        left out."""
        if not QueryInformationJobObject(self.handle, JOB_BASIC_PROCESS_ID_LIST, ctypes.byref(self.ids),
                                         ctypes.sizeof(self.ids), None):
            raise failed("QueryInformationJobObject(process ids)")
        sizes = {}
        for pid in self.ids.ProcessIdList[:self.ids.NumberOfProcessIdsInList]:
            process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
            if not process:
                if ctypes.get_last_error() == ERROR_INVALID_PARAMETER:
                    continue
                raise failed("OpenProcess")
            try:
                counters = MemoryCounters()
                counters.cb = ctypes.sizeof(counters)
                if not K32GetProcessMemoryInfo(process, ctypes.byref(counters), counters.cb):
                    raise failed("GetProcessMemoryInfo")
                sizes[pid] = counters.WorkingSetSize
            finally:
                CloseHandle(process)
        return sizes

    def terminate(self):
        if not TerminateJobObject(self.handle, 1):
            raise failed("TerminateJobObject")

    def close(self):
        if self.handle:
            CloseHandle(self.handle)
            self.handle = None


def resume(pid):
    """Resumes every thread of `pid`; a process started suspended has one."""
    snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
    if snapshot == INVALID_HANDLE_VALUE:
        raise failed("CreateToolhelp32Snapshot")
    resumed = 0
    try:
        entry = ThreadEntry()
        entry.dwSize = ctypes.sizeof(entry)
        more = Thread32First(snapshot, ctypes.byref(entry))
        while more:
            if entry.th32OwnerProcessID == pid:
                thread = OpenThread(THREAD_SUSPEND_RESUME, False, entry.th32ThreadID)
                if not thread:
                    raise failed("OpenThread")
                try:
                    if ResumeThread(thread) == 0xFFFFFFFF:
                        raise failed("ResumeThread")
                finally:
                    CloseHandle(thread)
                resumed += 1
            more = Thread32Next(snapshot, ctypes.byref(entry))
    finally:
        CloseHandle(snapshot)
    if not resumed:
        raise OSError("resume: the suspended process has no thread")


class JobPort:
    """The completion port every job of one owner reports to, and the thread
    that reads it and, every `every` seconds, asks `watch` about each job: a
    job whose process limit refused something, or that `watch` names a
    reason for, is ended at once and keeps the reason for its owner."""

    QUIT = 0
    # A message no job sends: the port has read everything queued before it.
    SETTLED = 0xFFFFFFFF

    def __init__(self, reasons, watch, every):
        self.handle = CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1)
        if not self.handle:
            raise failed("CreateIoCompletionPort")
        self.jobs, self.lock, self.reasons = {}, threading.Lock(), reasons
        self.watch, self.every = watch, every
        self.settling = {}
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def register(self, job):
        with self.lock:
            self.jobs[job.key] = job

    def forget(self, job):
        with self.lock:
            self.jobs.pop(job.key, None)

    def settle(self, job, seconds):
        """Returns once the port has read every message `job` sent before
        now, so a refusal that came just before its end is on the job."""
        done = threading.Event()
        with self.lock:
            self.settling[job.key] = done
        if not PostQueuedCompletionStatus(self.handle, self.SETTLED, job.key, None):
            raise failed("PostQueuedCompletionStatus")
        if not done.wait(seconds):
            raise OSError("job_port_unsettled")

    def read(self):
        message, key, overlapped = wintypes.DWORD(), ctypes.c_size_t(), ctypes.c_void_p()
        due = time.monotonic()
        while True:
            if time.monotonic() >= due:
                # Under the lock, so no job is forgotten and closed while it is measured.
                with self.lock:
                    for job in self.jobs.values():
                        if job.failure is None:
                            reason = self.watch(job)
                            if reason is not None:
                                self.end(job, reason)
                due = time.monotonic() + self.every
            if not GetQueuedCompletionStatus(self.handle, ctypes.byref(message), ctypes.byref(key),
                                             ctypes.byref(overlapped),
                                             max(0, int((due - time.monotonic()) * 1000))):
                if overlapped.value is None and ctypes.get_last_error() == WAIT_TIMEOUT:
                    continue
                return
            if key.value == self.QUIT:
                return
            if message.value == self.SETTLED:
                with self.lock:
                    done = self.settling.pop(key.value, None)
                if done is not None:
                    done.set()
                continue
            reason = self.reasons.get(message.value)
            if reason is None:
                continue
            with self.lock:
                job = self.jobs.get(key.value)
                if job is not None and job.failure is None:
                    self.end(job, reason)

    @staticmethod
    def end(job, reason):
        job.failure = reason
        try:
            job.terminate()
        except OSError:
            # The owner's end() still terminates and confirms it.
            pass

    def close(self):
        if self.handle:
            PostQueuedCompletionStatus(self.handle, 0, self.QUIT, None)
            self.thread.join(timeout=2)
            CloseHandle(self.handle)
            self.handle = None


# Private files ---------------------------------------------------------------

SE_FILE_OBJECT = 1
OWNER_SECURITY_INFORMATION = 0x1
DACL_SECURITY_INFORMATION = 0x4
TOKEN_QUERY = 0x0008
TOKEN_USER = 1
TOKEN_OWNER = 4
SDDL_REVISION_1 = 1
GENERIC_WRITE = 0x40000000
FILE_SHARE_ALL = 0x7
CREATE_NEW = 1
FILE_ATTRIBUTE_NORMAL = 0x80
ACL_SIZE_INFORMATION = 2
INHERIT_ONLY_ACE = 0x08
ACCESS_ALLOWED = 0
DENIALS = (1, 6, 0x0A, 0x0C)
# Accounts that may hold access to a private file besides this account, as
# hide-platform's check counts them: the system, the administrators, and
# OWNER RIGHTS (S-1-3-4), which stands for whoever owns the file.
TRUSTED_WELL_KNOWN = (22, 26, 71)  # WinLocalSystemSid, WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid


class SecurityAttributes(ctypes.Structure):
    _fields_ = [("nLength", wintypes.DWORD), ("lpSecurityDescriptor", ctypes.c_void_p),
                ("bInheritHandle", wintypes.BOOL)]


class AclSize(ctypes.Structure):
    _fields_ = [("AceCount", wintypes.DWORD), ("AclBytesInUse", wintypes.DWORD), ("AclBytesFree", wintypes.DWORD)]


class AceHeader(ctypes.Structure):
    _fields_ = [("AceType", ctypes.c_ubyte), ("AceFlags", ctypes.c_ubyte), ("AceSize", wintypes.WORD)]


class AllowedAce(ctypes.Structure):
    _fields_ = [("Header", AceHeader), ("Mask", wintypes.DWORD), ("SidStart", wintypes.DWORD)]


GetCurrentProcess = declare(kernel32, "GetCurrentProcess", HANDLE)
LocalFree = declare(kernel32, "LocalFree", ctypes.c_void_p, ctypes.c_void_p)
CreateDirectoryW = declare(kernel32, "CreateDirectoryW", wintypes.BOOL, wintypes.LPCWSTR,
                           ctypes.POINTER(SecurityAttributes))
CreateFileW = declare(kernel32, "CreateFileW", HANDLE, wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                      ctypes.POINTER(SecurityAttributes), wintypes.DWORD, wintypes.DWORD, HANDLE)
OpenProcessToken = declare(advapi32, "OpenProcessToken", wintypes.BOOL, HANDLE, wintypes.DWORD,
                           ctypes.POINTER(HANDLE))
GetTokenInformation = declare(advapi32, "GetTokenInformation", wintypes.BOOL, HANDLE, ctypes.c_int,
                              ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD))
GetLengthSid = declare(advapi32, "GetLengthSid", wintypes.DWORD, ctypes.c_void_p)
EqualSid = declare(advapi32, "EqualSid", wintypes.BOOL, ctypes.c_void_p, ctypes.c_void_p)
CreateWellKnownSid = declare(advapi32, "CreateWellKnownSid", wintypes.BOOL, ctypes.c_int, ctypes.c_void_p,
                             ctypes.c_void_p, ctypes.POINTER(wintypes.DWORD))
ConvertSidToStringSidW = declare(advapi32, "ConvertSidToStringSidW", wintypes.BOOL, ctypes.c_void_p,
                                 ctypes.POINTER(wintypes.LPWSTR))
ConvertStringSecurityDescriptorToSecurityDescriptorW = declare(
    advapi32, "ConvertStringSecurityDescriptorToSecurityDescriptorW", wintypes.BOOL, wintypes.LPCWSTR,
    wintypes.DWORD, ctypes.POINTER(ctypes.c_void_p), ctypes.c_void_p)
GetNamedSecurityInfoW = declare(advapi32, "GetNamedSecurityInfoW", wintypes.DWORD, wintypes.LPCWSTR,
                                ctypes.c_int, wintypes.DWORD, ctypes.POINTER(ctypes.c_void_p), ctypes.c_void_p,
                                ctypes.POINTER(ctypes.c_void_p), ctypes.c_void_p,
                                ctypes.POINTER(ctypes.c_void_p))
GetAclInformation = declare(advapi32, "GetAclInformation", wintypes.BOOL, ctypes.c_void_p, ctypes.c_void_p,
                            wintypes.DWORD, ctypes.c_int)
GetAce = declare(advapi32, "GetAce", wintypes.BOOL, ctypes.c_void_p, wintypes.DWORD,
                 ctypes.POINTER(ctypes.c_void_p))


def token_sid(token, kind):
    """The account (TokenUser) or the default owner (TokenOwner) as SID bytes."""
    size = wintypes.DWORD()
    GetTokenInformation(token, kind, None, 0, ctypes.byref(size))
    buffer = (ctypes.c_uint64 * ((size.value + 7) // 8 or 1))()
    if not GetTokenInformation(token, kind, buffer, size, ctypes.byref(size)):
        raise failed("GetTokenInformation")
    # TOKEN_USER and TOKEN_OWNER both begin with the pointer to the SID.
    sid = ctypes.c_void_p.from_buffer(buffer).value
    return ctypes.string_at(sid, GetLengthSid(sid))


def account():
    """(this account, the owner its new files get): the Administrators group
    owns what an elevated process creates."""
    token = HANDLE()
    if not OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, ctypes.byref(token)):
        raise failed("OpenProcessToken")
    try:
        return token_sid(token, TOKEN_USER), token_sid(token, TOKEN_OWNER)
    finally:
        CloseHandle(token)


def well_known(kind):
    buffer, size = ctypes.create_string_buffer(68), wintypes.DWORD(68)
    if not CreateWellKnownSid(kind, None, buffer, ctypes.byref(size)):
        raise failed("CreateWellKnownSid")
    return buffer.raw[:size.value]


def sid_text(sid):
    text = wintypes.LPWSTR()
    if not ConvertSidToStringSidW(sid, ctypes.byref(text)):
        raise failed("ConvertSidToStringSidW")
    try:
        return text.value
    finally:
        LocalFree(ctypes.cast(text, ctypes.c_void_p))


class PrivateDescriptor:
    """An access list giving this account everything and nobody else
    anything, cut off from what the parent folder would pass down; a
    folder's children inherit it."""

    def __init__(self, folder):
        user, _ = account()
        text = f"D:P(A;{'OICI' if folder else ''};FA;;;{sid_text(user)})"
        self.pointer = ctypes.c_void_p()
        if not ConvertStringSecurityDescriptorToSecurityDescriptorW(text, SDDL_REVISION_1,
                                                                    ctypes.byref(self.pointer), None):
            raise failed("ConvertStringSecurityDescriptorToSecurityDescriptorW")
        self.attributes = SecurityAttributes(ctypes.sizeof(SecurityAttributes), self.pointer, False)

    def __enter__(self):
        return ctypes.byref(self.attributes)

    def __exit__(self, *_):
        LocalFree(self.pointer)


def make_private_directory(path):
    with PrivateDescriptor(True) as attributes:
        if not CreateDirectoryW(str(path), attributes):
            raise failed("CreateDirectoryW")


def create_private_file(path):
    """A new file only this account can open, as a writable descriptor; an
    existing name, a link included, is refused."""
    with PrivateDescriptor(False) as attributes:
        handle = CreateFileW(str(path), GENERIC_WRITE, FILE_SHARE_ALL, attributes, CREATE_NEW,
                             FILE_ATTRIBUTE_NORMAL, None)
    if handle == INVALID_HANDLE_VALUE:
        raise failed("CreateFileW")
    return msvcrt.open_osfhandle(handle, os.O_WRONLY)


def is_private(path):
    """Whether this account owns `path` and its access list lets no other
    account in, the Windows form of a 0700/0600 mode check."""
    owner, acl, descriptor = ctypes.c_void_p(), ctypes.c_void_p(), ctypes.c_void_p()
    status = GetNamedSecurityInfoW(str(path), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                                   ctypes.byref(owner), None, ctypes.byref(acl), None, ctypes.byref(descriptor))
    if status:
        raise OSError(status, f"GetNamedSecurityInfoW: {ctypes.FormatError(status).strip()}")
    try:
        user, default_owner = account()
        if not any(EqualSid(owner, ctypes.c_char_p(sid)) for sid in (user, default_owner)):
            return False
        if not acl.value:
            # No access list at all lets everybody in.
            return False
        trusted = [user, default_owner, *(well_known(kind) for kind in TRUSTED_WELL_KNOWN)]
        size = AclSize()
        if not GetAclInformation(acl, ctypes.byref(size), ctypes.sizeof(size), ACL_SIZE_INFORMATION):
            raise failed("GetAclInformation")
        for index in range(size.AceCount):
            entry = ctypes.c_void_p()
            if not GetAce(acl, index, ctypes.byref(entry)):
                raise failed("GetAce")
            header = AceHeader.from_address(entry.value)
            if header.AceFlags & INHERIT_ONLY_ACE or header.AceType in DENIALS:
                continue
            if header.AceType != ACCESS_ALLOWED:
                # An entry this check cannot read is counted as access.
                return False
            allowed = AllowedAce.from_address(entry.value)
            sid = entry.value + AllowedAce.SidStart.offset
            if allowed.Mask and not any(EqualSid(sid, ctypes.c_char_p(known)) for known in trusted):
                return False
        return True
    finally:
        LocalFree(descriptor)


class UserPath:
    """`folder` first on this account's own Path in the registry until
    `close`. Herdr builds a Windows pane's environment from the registry (its
    portable-pty reads HKLM's and HKCU's Environment over the server's own),
    so a PATH given to Herdr never reaches a pane Hide opens, and an agent the
    run starts there is found only through this Path, as an operator's is.
    It changes the account, so it is held only on a disposable CI runner and
    refused anywhere else."""

    def __init__(self, folder):
        from .protection import ProtectionError
        if os.environ.get("GITHUB_ACTIONS") != "true":
            raise ProtectionError("user_path_changed_only_on_a_ci_runner: GITHUB_ACTIONS is not true, "
                                  "and this account's registry is not the run's to change")
        self.folder = str(folder)
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0,
                            winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as key:
            try:
                value, kind = winreg.QueryValueEx(key, "Path")
                self.existed = True
            except FileNotFoundError:
                value, kind, self.existed = "", winreg.REG_EXPAND_SZ, False
            winreg.SetValueEx(key, "Path", 0, kind, f"{self.folder};{value}" if value else self.folder)

    def close(self):
        """Takes `folder` back out, keeping whatever else changed the Path
        meanwhile; a Path that did not exist before is removed again."""
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Environment", 0,
                            winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as key:
            try:
                value, kind = winreg.QueryValueEx(key, "Path")
            except FileNotFoundError:
                return
            entries = value.split(";")
            wanted = os.path.normcase(self.folder)
            entries = next(([*entries[:index], *entries[index + 1:]] for index, entry in enumerate(entries)
                            if os.path.normcase(entry) == wanted), entries)
            if entries or self.existed:
                winreg.SetValueEx(key, "Path", 0, kind, ";".join(entries))
            else:
                winreg.DeleteValue(key, "Path")
