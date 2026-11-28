"""Read-only copies of a quiescent SQLite DB and its WAL/SHM as one bundle."""

import os
from pathlib import Path
import stat

from .processes import ProcessError
from .protection import (ConfigurationChanged, ProtectionError, beneath,
                         configuration_bytes)


class CredentialSnapshotUnavailable(RuntimeError):
    """No consistent private login snapshot was established; launch nothing."""


def metadata(paths, operator, budget):
    result, total = {}, 0
    for path in paths:
        if (not beneath(path, operator) or path.is_symlink()
                or any(parent.is_symlink() for parent in path.parents if beneath(parent, operator))):
            raise ProtectionError("credential_copy_alias_or_path_refused")
        try:
            info = path.lstat()
        except FileNotFoundError:
            result[path] = None
            continue
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                or info.st_nlink != 1):
            raise ProtectionError("config_not_private_regular_file")
        total += info.st_size
        if total > budget:
            raise ProtectionError("credential_copy_total_over_budget")
        result[path] = (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_mode)
    return result


def require_unopened(paths, owner, env):
    # Authenticated measurement is macOS-only. Use the system executable,
    # never a provider-controlled PATH entry; OwnedProcesses bounds this call.
    program = Path("/usr/sbin/lsof")
    if not program.is_file():
        raise CredentialSnapshotUnavailable("credential_opener_check_unavailable")
    try:
        code, output, error = owner.run([str(program), "-nP", "-F", "p", "--", *map(str, paths)],
                                        env=env, check=False)
    except ProcessError as failure:
        # An answer that never came is no answer: this login goes unmeasured,
        # never the whole run (it ended a run on the mini, 2026-11-28, while
        # an agent the person had left open held the file).
        raise CredentialSnapshotUnavailable(f"credential_opener_check_failed: {failure}") from failure
    # lsof exits 1 with no output when it found no matching open file.
    # Every diagnostic or ambiguous answer is unavailable, never absence.
    if code != 1 or output.strip() or error.strip():
        raise CredentialSnapshotUnavailable("credential_snapshot_unavailable")


def snapshot(source: Path, operator: Path, budget: int, owner, env) -> list:
    """Never open the original with SQLite or checkpoint it.

    Opener queries are observations, not an atomic lock against future opens.
    Before/after identity, size, mtime and bytes must agree for all three files.
    Recovery, if needed, happens only when the native CLI opens the copies.
    """
    paths = [source, Path(str(source) + "-wal"), Path(str(source) + "-shm")]
    before = metadata(paths, operator, budget)
    if before[source] is None:
        if any(before.values()):
            raise ProtectionError("credential_database_bundle_incomplete")
        return []
    if owner is None:
        raise ProtectionError("credential_database_opener_owner_missing")
    existing = [path for path in paths if before[path] is not None]
    require_unopened(existing, owner, env)
    values, total = [], 0
    try:
        for path in existing:
            read = configuration_bytes(path, max_bytes=budget - total)
            if read is None:
                raise CredentialSnapshotUnavailable("credential_snapshot_changed")
            stamp, content = read
            if (*stamp.identity, stamp.size) != before[path][:3]:
                raise CredentialSnapshotUnavailable("credential_snapshot_changed")
            total += len(content)
            values.append((path, stamp, content))
        if metadata(paths, operator, budget) != before:
            raise CredentialSnapshotUnavailable("credential_snapshot_changed")
        # Detect an opener that arrived during the bounded read as well.
        require_unopened(existing, owner, env)
        for path, original, _ in values:
            read = configuration_bytes(path, max_bytes=budget)
            if read is None or read[0] != original:
                raise CredentialSnapshotUnavailable("credential_snapshot_changed")
        if metadata(paths, operator, budget) != before:
            raise CredentialSnapshotUnavailable("credential_snapshot_changed")
    except (ConfigurationChanged, FileNotFoundError) as error:
        raise CredentialSnapshotUnavailable("credential_snapshot_changed") from error
    return values
