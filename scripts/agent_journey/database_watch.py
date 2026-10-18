"""Compare login rows from bounded private clones, never an original SQLite handle."""

import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
import tempfile

from agent_live_check.credential_snapshot import metadata
from agent_live_check.protection import MAX_BACKUP_BYTES, MAX_CONFIG_FILES, ProtectionError, private_directory

MAX_DATABASE_BYTES = 1024 ** 3
CHUNK_BYTES = 1024 ** 2
# Pinned table/column sources are named in docs/VERIFICATION.md.
DATABASES = {
    ".omp/agent/agent.db": {
        "settings": "SELECT key, value, updated_at FROM settings ORDER BY key",
        "auth_credentials": "SELECT id, provider, credential_type, data, disabled_cause, identity_key, "
                            "created_at, updated_at FROM auth_credentials ORDER BY id",
    },
    ".local/share/opencode/opencode.db": {
        "credential": "SELECT id, integration_id, label, value, connector_id, method_id, active, "
                      "time_created, time_updated FROM credential ORDER BY id",
    },
}


def clone(source_fd, destination):
    """Clone the already-open regular file; unsupported filesystems use streaming."""
    if sys.platform != "darwin":
        return False
    library = ctypes.CDLL(None, use_errno=True)
    function = library.fclonefileat
    function.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int]
    function.restype = ctypes.c_int
    folder_fd = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        if function(source_fd, folder_fd, os.fsencode(destination.name), 0) == 0:
            destination.chmod(0o600)
            return True
        number = ctypes.get_errno()
        if number not in (errno.ENOTSUP, errno.EXDEV, errno.ENOSYS):
            raise OSError(number, os.strerror(number))
        return False
    finally:
        os.close(folder_fd)


def signature(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_mode,
            info.st_ctime_ns, info.st_nlink)


class CopyChanged(ProtectionError):
    """The live bundle moved during a copy; only this condition is retried."""


def copy_file(path, expected, destination):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as source:
        before = os.fstat(source.fileno())
        if (signature(before)[:5] != expected or not stat.S_ISREG(before.st_mode)
                or before.st_uid != os.getuid() or before.st_nlink != 1):
            raise CopyChanged("operator_watch_database_copy_changed")
        if clone(source.fileno(), destination):
            method = "apfs_clone"
        else:
            method, count = "chunked_copy", 0
            with os.fdopen(os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as sink:
                while data := source.read(CHUNK_BYTES):
                    count += len(data)
                    if count > expected[2]:
                        raise CopyChanged("operator_watch_database_copy_changed")
                    sink.write(data)
                if count != expected[2]:
                    raise CopyChanged("operator_watch_database_copy_changed")
        if signature(os.fstat(source.fileno())) != signature(before):
            raise CopyChanged("operator_watch_database_copy_changed")
        return method


def digest(root, scratch, name, copies=None, markers=None):
    try:
        for attempt in range(3):
            try:
                return snapshot(root, scratch, name, copies, markers)
            except CopyChanged:
                if attempt == 2:
                    raise
    except OSError as error:
        raise ProtectionError(f"operator_watch_database_copy_failed: errno={error.errno}") from None
    except (sqlite3.Error, ValueError, TypeError) as error:
        raise ProtectionError("operator_watch_database_snapshot_unreadable: " + type(error).__name__) from None


def snapshot(root, scratch, name, copies, markers):
    source = root / name
    paths = [source, Path(str(source) + "-wal")]
    before = metadata(paths, root, MAX_DATABASE_BYTES)
    if before[source] is None:
        if any(before.values()):
            raise ProtectionError("operator_watch_database_bundle_incomplete")
        return None
    if scratch is None:
        raise ProtectionError("operator_watch_database_private_copy_directory_missing")
    private_directory(scratch)
    with tempfile.TemporaryDirectory(prefix="database-", dir=scratch) as folder:
        copy = Path(folder) / source.name
        copied = []
        for path in paths:
            if before[path] is None:
                continue
            try:
                method = copy_file(path, before[path], Path(folder) / path.name)
            except FileNotFoundError:
                raise CopyChanged("operator_watch_database_copy_changed") from None
            copied.append({"agent": "omp" if name.startswith(".omp/") else "opencode",
                           "file": "database" if path == source else "wal",
                           "method": method, "bytes": before[path][2]})
        if metadata(paths, root, MAX_DATABASE_BYTES) != before:
            raise CopyChanged("operator_watch_database_copy_changed")
        connection = sqlite3.connect(copy, timeout=1)
        try:
            connection.execute("PRAGMA trusted_schema=OFF")
            result, total = {}, 0
            for table, query in DATABASES[name].items():
                hashed, count, marked = hashlib.sha256(), 0, False
                for row in connection.execute(query):
                    count += 1
                    if count > MAX_CONFIG_FILES:
                        raise ProtectionError("operator_watch_database_rows_over_budget")
                    content = json.dumps(row, ensure_ascii=True, separators=(",", ":")).encode() + b"\n"
                    total += len(content)
                    if total > MAX_BACKUP_BYTES:
                        raise ProtectionError("operator_watch_database_row_bytes_over_budget")
                    hashed.update(content)
                    marked |= bool(markers and markers.contains(content))
                result[table] = {"sha256": hashed.hexdigest(), "rows": count, "marker": marked}
            if copies is not None:
                copies.extend(copied)
            return result
        finally:
            connection.close()
