"""Process identities and RSS, without subprocesses or inspecting argv.

Darwin layouts follow sys/proc_info.h in the installed SDK. Linux uses proc(5).
"""

import ctypes
import errno
import json
from dataclasses import dataclass
from enum import Enum
import os
from pathlib import Path
import sys

MAX_SYSTEM_PROCESSES = 65_536
MAX_PROC_CONTEXT_BYTES = 1024 * 1024


class ProcessTable(dict):
    """Keep readable peers when some subjects cannot be inspected."""

    def __init__(self):
        super().__init__()
        self.unavailable = []
        self.vanished = []
        self.foreign_uid_pids = set()
        # Live members of a sampled group left out of the table because they
        # run a set-user-ID program: not measured, still ours to end.
        self.unmeasured = set()


class ProcessState(Enum):
    LIVE = "live"
    VANISHED = "vanished"
    ZOMBIE = "zombie"


def require_complete(table):
    unavailable = getattr(table, "unavailable", ())
    if unavailable:
        raise RuntimeError("process_table_subjects_unavailable:" + json.dumps(
            {"count": len(unavailable), "subjects": unavailable[:4]},
            sort_keys=True, separators=(",", ":")))


def validate_linux_procfs(status: str, mountinfo: str, own_pid: int):
    # fs/proc/array.c emits one NSpid per namespace, beginning at procfs's
    # namespace. Even coincident PID numbers cannot hide an extra namespace.
    identities = [line.split()[1:] for line in status.splitlines()
                  if line.startswith("NSpid:")]
    if identities != [[str(own_pid)]]:
        raise RuntimeError("procfs_pid_namespace_unconfirmed")
    roots = []
    for line in mountinfo.splitlines():
        before, separator, after = line.partition(" - ")
        fields, filesystem = before.split(), after.split()
        if not separator or len(fields) < 6 or len(filesystem) < 3:
            raise RuntimeError("procfs_mount_context_unavailable")
        point = fields[4]
        if point == "/proc":
            roots.append((fields, filesystem))
        elif point.startswith("/proc/"):
            component = point.split("/")[2]
            if component.isdecimal() or component in ("self", "thread-self"):
                raise RuntimeError("procfs_process_view_overmounted")
    if len(roots) != 1:
        raise RuntimeError("procfs_mount_context_unavailable")
    fields, filesystem = roots[0]
    if fields[3] != "/" or filesystem[0] != "proc":
        raise RuntimeError("procfs_mount_context_unavailable")
    options = fields[5].split(",") + filesystem[2].split(",")
    if any(option.startswith("hidepid=") and option not in ("hidepid=0", "hidepid=off")
           for option in options):
        # hidepid can omit live processes from readdir or return ENOENT.
        raise RuntimeError("procfs_process_visibility_restricted")


def linux_procfs_context():
    contents = []
    for name in ("status", "mountinfo"):
        with (Path("/proc/self") / name).open("rb") as stream:
            data = stream.read(MAX_PROC_CONTEXT_BYTES + 1)
        if len(data) > MAX_PROC_CONTEXT_BYTES:
            raise RuntimeError("procfs_mount_context_over_budget")
        contents.append(data.decode("utf-8", errors="strict"))
    validate_linux_procfs(*contents, os.getpid())


@dataclass(frozen=True)
class Process:
    pid: int
    parent: int
    group: int
    birth: int
    rss: int
    zombie: bool
    uid: int
    traced: bool = False
    pointer_width: int = 8
    name: str = ""


class BsdInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint32) for name in (
        "flags", "status", "xstatus", "pid", "ppid", "uid", "gid",
        "ruid", "rgid", "svuid", "svgid", "reserved")]
    _fields_ += [("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32)]
    _fields_ += [(name, ctypes.c_uint32) for name in (
        "nfiles", "pgid", "jobc", "tdev", "tpgid")]
    _fields_ += [("nice", ctypes.c_int32), ("sec", ctypes.c_uint64),
                ("usec", ctypes.c_uint64)]


class ShortBsdInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint32) for name in ("pid", "ppid", "pgid", "status")]
    _fields_ += [("comm", ctypes.c_char * 16)]
    _fields_ += [(name, ctypes.c_uint32) for name in (
        "flags", "uid", "gid", "ruid", "rgid", "svuid", "svgid", "reserved")]


class TaskInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "virtual", "resident", "total_user", "total_system", "threads_user",
        "threads_system")]
    _fields_ += [("counts", ctypes.c_int32 * 12)]


def snapshot(group: int | None = None) -> dict[int, Process]:
    result = ProcessTable()
    if sys.platform == "darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        library.proc_listallpids.argtypes = [ctypes.c_void_p, ctypes.c_int]
        library.proc_listpgrppids.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
        library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int,
                                        ctypes.c_uint64, ctypes.c_void_p,
                                        ctypes.c_int]
        pids = (ctypes.c_int * MAX_SYSTEM_PROCESSES)()
        ctypes.set_errno(0)
        count = (library.proc_listallpids(pids, ctypes.sizeof(pids)) if group is None
                 else library.proc_listpgrppids(group, pids, ctypes.sizeof(pids)))
        if group is not None and count <= 0 and ctypes.get_errno() == errno.ESRCH:
            return result
        if count < 0 or (count == 0 and (group is None or ctypes.get_errno())) or count >= MAX_SYSTEM_PROCESSES:
            raise RuntimeError("process_table_unavailable_or_over_budget")

        def short_read(pid):
            # Full BSD info requires the same UID; short BSD info does not.
            short = ShortBsdInfo()
            ctypes.set_errno(0)
            read = library.proc_pidinfo(pid, 13, 1, ctypes.byref(short), ctypes.sizeof(short))
            return (short if read == ctypes.sizeof(short) else None), ctypes.get_errno()

        def another_users(pid, short):
            # A process that runs as another UID is refused the full read by
            # design. A host-wide sample excludes it, rather than confusing
            # that refusal with our ownership. A group sample excludes only
            # its own member that runs a set-user-ID program (ps, which
            # Claude Code runs in its group): its real UID is still ours.
            # It is not measured, but it is still the group's live member:
            # the guardian signals the group while it lives (`unmeasured`).
            if short is None or short.uid == os.getuid():
                return False
            if group is None:
                return True
            if short.pgid == group and short.ruid == os.getuid():
                if short.status != 5:
                    result.unmeasured.add(pid)
                return True
            return False

        for pid in pids[:count]:
            info, task = BsdInfo(), TaskInfo()
            ctypes.set_errno(0)
            if library.proc_pidinfo(pid, 3, 1, ctypes.byref(info), ctypes.sizeof(info)) != ctypes.sizeof(info):
                error = ctypes.get_errno()
                short, short_error = short_read(pid)
                if error == errno.ESRCH:
                    # A later foreign UID may be a reused PID. Preserve the
                    # earlier disappearance before excluding that replacement.
                    result.vanished.append(pid)
                if another_users(pid, short):
                    result.foreign_uid_pids.add(pid)
                    continue
                if error == errno.ESRCH and (short is not None or short_error == errno.ESRCH):
                    continue
                else:
                    result.unavailable.append({"pid": pid, "errno": error})
                continue
            read = library.proc_pidinfo(pid, 4, 0, ctypes.byref(task), ctypes.sizeof(task))
            if read != ctypes.sizeof(task):
                # A process can exit between the BSD and task reads. One
                # fresh kernel identity distinguishes that terminal state
                # from an active process whose RSS cannot be measured.
                later = BsdInfo()
                ctypes.set_errno(0)
                refreshed = library.proc_pidinfo(pid, 3, 1, ctypes.byref(later), ctypes.sizeof(later))
                refresh_error = ctypes.get_errno()
                if refreshed != ctypes.sizeof(later) and refresh_error == errno.ESRCH:
                    result.vanished.append(pid)
                    continue
                if refreshed != ctypes.sizeof(later):
                    # A child that execs a set-user-ID program (ps, login)
                    # between the reads now runs as another UID: the same
                    # process a sample taken a moment later excludes.
                    if another_users(pid, short_read(pid)[0]):
                        result.foreign_uid_pids.add(pid)
                        continue
                    result.unavailable.append({"pid": pid, "errno": refresh_error})
                    continue
                if later.uid != info.uid:
                    result.unavailable.append({"pid": pid, "errno": errno.EPERM,
                                               "reason": "uid_changed_during_rss_read"})
                    continue
                if ((later.sec, later.usec) != (info.sec, info.usec)
                        and group is not None and later.pgid != group):
                    # Only the old group's member ended. A host-wide sample
                    # retains replacements for independent ownership checks;
                    # a fresh member of this group still has unknown RSS.
                    result.vanished.append(pid)
                    continue
                info = later
            if group is not None and info.pgid != group:
                # Enumeration and identity reads can straddle exit/reuse or
                # a group move. Only this identity's own group can enroll it.
                continue
            result[pid] = Process(pid, info.ppid, info.pgid,
                                  info.sec * 1_000_000 + info.usec,
                                  task.resident if read == ctypes.sizeof(task) else -1,
                                  info.status == 5, info.uid, bool(info.flags & 2),
                                  8 if info.flags & 0x10 else 4,
                                  bytes(info.comm).decode("utf-8", errors="replace"))
    elif sys.platform.startswith("linux"):
        linux_procfs_context()
        with os.scandir("/proc") as entries:
            for index, entry in enumerate(entries):
                if index >= MAX_SYSTEM_PROCESSES:
                    raise RuntimeError("process_table_over_budget")
                if not entry.name.isdecimal():
                    continue
                try:
                    raw = (Path(entry.path) / "stat").read_text()
                    fields = raw[raw.rindex(")") + 2:].split()
                    if group is not None and int(fields[2]) != group:
                        continue
                    pid = int(entry.name)
                    result[pid] = Process(pid, int(fields[1]), int(fields[2]),
                                          int(fields[19]),
                                          int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
                                          fields[0] == "Z", os.stat(entry.path).st_uid,
                                          name=raw[raw.index("(") + 1:raw.rindex(")")])
                except (FileNotFoundError, ProcessLookupError):
                    continue
                except PermissionError as error:
                    result.unavailable.append({"pid": int(entry.name), "errno": error.errno})
    else:
        raise RuntimeError("process_supervision_requires_darwin_or_linux")
    if group is None and os.getpid() not in result:
        if not any(subject["pid"] == os.getpid() for subject in result.unavailable):
            raise RuntimeError("own_process_missing_from_table")
    return result


def descendants(table: dict[int, Process], root: int, known=None) -> dict[int, Process]:
    # A token-proven orphan remains an owned ancestry root. A recycled PID
    # cannot carry that proof into a different process's subtree.
    selected = {root} | {pid for pid, identity in (known or {}).items()
                         if pid in table and table[pid].birth == identity.birth}
    while True:
        added = {pid for pid, item in table.items() if item.parent in selected}
        if added <= selected:
            return {pid: table[pid] for pid in selected if pid in table}
        selected |= added


def identity_state(pid, birth, *, uid=None, pointer_width=None, table=None):
    """Classify an expected birth; denied or inconsistent identity is unknown.

    Width binds a procargs interpretation, not established process ownership.
    A caller with group/token proof checks UID and birth before signalling.
    """
    if table is None and sys.platform == "darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64,
                                        ctypes.c_void_p, ctypes.c_int]
        info = BsdInfo()
        ctypes.set_errno(0)
        read = library.proc_pidinfo(pid, 3, 1, ctypes.byref(info), ctypes.sizeof(info))
        if read != ctypes.sizeof(info):
            error = ctypes.get_errno()
            if error == errno.ESRCH:
                return ProcessState.VANISHED
            raise RuntimeError("owned_process_identity_unavailable_" + str(error) + ":" + str(pid))
        current_birth, current_uid = info.sec * 1_000_000 + info.usec, info.uid
        width, zombie = (8 if info.flags & 0x10 else 4), info.status == 5
    else:
        table = snapshot() if table is None else table
        if (any(subject["pid"] == pid for subject in getattr(table, "unavailable", ()))
                or pid in getattr(table, "foreign_uid_pids", ())):
            raise RuntimeError("owned_process_identity_unavailable:" + str(pid))
        actual = table.get(pid)
        if actual is None:
            return ProcessState.VANISHED
        current_birth, current_uid = actual.birth, actual.uid
        width, zombie = actual.pointer_width, actual.zombie
    if current_birth != birth:
        return ProcessState.VANISHED
    if uid is not None and current_uid != uid:
        raise RuntimeError("owned_process_uid_changed:" + str(pid))
    if pointer_width is not None and width != pointer_width:
        raise RuntimeError("process_argument_width_changed:" + str(pid))
    return ProcessState.ZOMBIE if zombie else ProcessState.LIVE


def retain_owned(group_table, host_table, known, group):
    """Keep readable identities and group-owned set-user-ID members separate.

    A reserved group's short BSD read proves membership and real UID even
    when exec makes birth/RSS unreadable. Such members remain owned by the
    group, never by a stale per-PID identity used for individual signals.
    """
    retained = {}
    for pid, previous in known.items():
        if group is not None and pid in getattr(host_table, "foreign_uid_pids", ()):
            # Exec may occur between the group and host samples. Refresh the
            # group once at that boundary instead of rejecting the old birth.
            current = group_table if pid in group_table.foreign_uid_pids else snapshot(group)
            require_complete(current)
            if pid in current.foreign_uid_pids:
                group_table.pop(pid, None)
                if pid in current.unmeasured:
                    group_table.unmeasured.add(pid)
                continue
            # It may have exited or execed back between samples. Use the
            # fresh readable identity when present; absence from a group
            # alone cannot prove an escaped process ended.
            if pid in current:
                state = identity_state(pid, previous.birth, uid=previous.uid, table=current)
                if state is not ProcessState.VANISHED:
                    retained[pid] = current[pid]
                continue
            if identity_state(pid, previous.birth, uid=previous.uid) is ProcessState.VANISHED:
                continue
        state = identity_state(pid, previous.birth, uid=previous.uid, table=host_table)
        if state is not ProcessState.VANISHED:
            retained[pid] = host_table[pid]
    return retained


def live_members(table):
    """Whether a group sample holds a member that still runs: a readable one,
    or one left unmeasured because it runs a set-user-ID program."""
    return any(not p.zombie for p in table.values()) or bool(getattr(table, "unmeasured", ()))


def group_state(table):
    require_complete(table)
    if not table and not getattr(table, "unmeasured", ()):
        return ProcessState.VANISHED
    return ProcessState.LIVE if live_members(table) else ProcessState.ZOMBIE


class OwnerEnvironmentUnavailable(RuntimeError):
    """Structural facts only: never retain the queried argv or environment."""

    def __init__(self, reason, byte_count, argc, offset):
        self.details = {"reason": reason, "bytes": byte_count, "argc": argc, "offset": offset}
        super().__init__("owner_environment_unavailable:" + json.dumps(
            self.details, sort_keys=True, separators=(",", ":")))


def procargs_owned(data: bytes, pointer_width: int, expected) -> bool:
    """Prove token ownership or a complete negative KERN_PROCARGS2 answer."""
    argc = int.from_bytes(data[:4], sys.byteorder, signed=True) if len(data) >= 4 else None
    def refuse(reason, offset=0):
        raise OwnerEnvironmentUnavailable(reason, len(data), argc, offset)
    if len(data) < 4:
        refuse("header_short")
    if not 1 <= argc <= MAX_SYSTEM_PROCESSES:
        refuse("argc_invalid")
    if pointer_width not in (4, 8):
        refuse("width_invalid")
    offset = data.find(b"\0", 4)
    if offset < 0:
        refuse("path_unterminated", 4)
    offset += 1
    # exec_extract_strings aligns the saved executable path to the target's
    # pointer width. sysctl strips its 16-byte key and prepends a 4-byte argc.
    # Skipping every NUL here would also consume a legitimate empty argv[0].
    aligned = 4 + ((offset - 4 + pointer_width - 1) // pointer_width) * pointer_width
    if aligned >= len(data):
        refuse("padding_missing", offset)
    if any(data[offset:aligned]):
        refuse("padding_invalid", offset)
    offset = aligned
    first_empty = False
    for index in range(argc):
        end = data.find(b"\0", offset)
        if end < 0:
            refuse("argv_unterminated", offset)
        if index == 0:
            first_empty = end == offset
        offset = end + 1
    environment = [entry for entry in data[offset:].split(b"\0") if entry]
    # XNU may successfully return argv but omit ALL environment variables for
    # a restricted target. An empty/invalid environment cannot exclude ours.
    if not data.endswith(b"\0"):
        refuse("tail_unterminated", offset)
    if not environment:
        refuse("tail_empty", offset)
    if any(b"=" not in entry for entry in environment):
        refuse("tail_non_assignment", offset)
    if (any(expected(entry) for entry in environment) if callable(expected) else expected in environment):
        return True
    # XNU's restricted-target crop also skips every NUL after the path. With
    # empty argv[0] it can expose only an environment prefix. Missing ownership
    # in that prefix is not a complete negative observation.
    if first_empty:
        refuse("empty_argv_negative", offset)
    return False


def marked_descendants(table: dict[int, Process], marker, earliest: int = 0, *, known=None, remember=None, unknown=None) -> dict[int, Process]:
    """Find readable marked orphans; unavailable foreign context is diagnostic.

    A caller supplies the current/previous marker predicate. A missing or
    unreadable token never proves ownership; known identities retain their
    earlier proof only while their sampled birth still matches.
    """
    if sys.platform == "darwin":
        library = ctypes.CDLL(None, use_errno=True)
        library.sysctl.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.c_uint,
                                   ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
                                   ctypes.c_void_p, ctypes.c_size_t]
        buffer = None
    expected = marker if callable(marker) else ("HIDE_LIVE_CHECK_OWNER=" + marker).encode()
    result = {}
    for pid, process in table.items():
        if process.uid != os.getuid() or process.birth < earliest or process.zombie:
            continue
        identity = known.get(pid) if known else None
        if identity and identity.birth == process.birth:
            # An ancestry/token proof survives reparenting and exec. Reading
            # the argument stack again adds no ownership evidence and can race
            # with exec/exit. PID reuse still requires a new token proof.
            result[pid] = process
            if remember:
                remember(pid, process)
            continue
        # A marked helper can have a live escaped parent. Parentage and the
        # traced flag are not filters for positive marker ownership.
        if sys.platform.startswith("linux"):
            try:
                with (Path("/proc") / str(pid) / "environ").open("rb") as stream:
                    data = stream.read(MAX_PROC_CONTEXT_BYTES + 1)
                if not data or len(data) > MAX_PROC_CONTEXT_BYTES:
                    if unknown:
                        unknown(process)
                    continue
                entries = data.split(b"\0")
                owned = any(expected(entry) for entry in entries) if callable(expected) else expected in entries
                current = identity_state(pid, process.birth, uid=process.uid)
                if current is ProcessState.LIVE and owned:
                    result[pid] = process
            except (FileNotFoundError, ProcessLookupError):
                if hasattr(table, "vanished"):
                    table.vanished.append(pid)
            except (OSError, RuntimeError):
                if unknown:
                    unknown(process)
            continue
        mib = (ctypes.c_int * 3)(1, 49, pid)  # CTL_KERN, KERN_PROCARGS2, pid (installed SDK).
        size = ctypes.c_size_t(MAX_PROC_CONTEXT_BYTES)
        if buffer is None:
            buffer = ctypes.create_string_buffer(MAX_PROC_CONTEXT_BYTES)
        if library.sysctl(mib, 3, buffer, ctypes.byref(size), None, 0):
            error = ctypes.get_errno()
            if error == errno.ESRCH:
                if hasattr(table, "vanished"):
                    table.vanished.append(pid)
                continue
            if unknown:
                unknown(process)
            continue
        try:
            current = identity_state(pid, process.birth, uid=process.uid,
                                     pointer_width=process.pointer_width)
        except RuntimeError:
            if unknown:
                unknown(process)
            continue
        if current is not ProcessState.LIVE:
            if current is ProcessState.VANISHED and hasattr(table, "vanished"):
                table.vanished.append(pid)
            continue
        try:
            # Only this reply's prefix has kernel provenance. Reusing one
            # bounded buffer avoids allocating and copying 1 MiB per PID.
            owned = procargs_owned(buffer[:size.value], process.pointer_width, expected)
        except OwnerEnvironmentUnavailable:
            if unknown:
                unknown(process)
            continue
        if owned:
            result[pid] = process
            if remember:
                # Retain proven ownership even if a later unrelated process
                # refuses inspection and the overall scan must fail closed.
                remember(pid, process)
    return result
