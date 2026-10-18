"""Pre-launch isolation and private, conflict-preserving configuration recovery."""

from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import stat
import tomllib

from .system import WINDOWS

if WINDOWS:
    from . import windows

MAX_CONFIG_FILES = 50_000
MAX_CONFIG_BYTES = 256 * 1024 * 1024
MAX_BACKUP_BYTES = 16 * 1024 * 1024
SHARED_READ_ATTEMPTS = 3


class ProtectionError(RuntimeError):
    """A guard failed before permission to launch or restore was established."""

    def __init__(self, reason: str, *, path: Path | None = None):
        super().__init__(reason)
        self.path = str(path) if path is not None else None


class ConfigurationChanged(ProtectionError):
    """An otherwise ordinary file changed version during observation."""


def beneath(path: Path, root: Path) -> bool:
    return path.resolve().is_relative_to(root.resolve())


def private(path: Path) -> bool:
    """Whether this account owns `path` and no other account may open it: no
    group or other mode bits on Unix, an access list naming no other account
    (the system and the administrators aside) on Windows."""
    if WINDOWS:
        return windows.is_private(path)
    info = path.stat()
    return info.st_uid == os.getuid() and not info.st_mode & 0o077


def private_directory(path: Path) -> None:
    """Create every missing component privately; never widen existing access."""
    pending, parent = [], path
    while not parent.exists():
        if parent.is_symlink():
            raise ProtectionError("private_directory_is_link")
        pending.append(parent)
        parent = parent.parent
    if parent.is_symlink() or path.is_symlink():
        raise ProtectionError("private_directory_is_link")
    for folder in reversed(pending):
        if WINDOWS:
            windows.make_private_directory(folder)
        else:
            folder.mkdir(mode=0o700)
    if not private(path):
        raise ProtectionError("private_directory_not_private")


def write_private(path: Path, data: bytes) -> None:
    """Create a new private file, never follow or replace a preexisting name."""
    fd = (windows.create_private_file(path) if WINDOWS
          else os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600))
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def validate_isolation(run: Path, home: Path, socket: Path, state: Path,
                       operator_home: Path, operator_socket: Path | None,
                       socket_root: Path | None = None) -> None:
    """Refuse operator routing before even starting a version/auth subprocess."""
    if not all(path.is_absolute() for path in (run, home, socket, state)):
        raise ProtectionError("isolation_paths_must_be_absolute")
    if home.resolve() == operator_home.resolve():
        raise ProtectionError("daemon_home_is_operator_home")
    for path in (home, state):
        if not beneath(path, run) or path.resolve() == run.resolve():
            raise ProtectionError("runtime_path_outside_run")
    # Herdr's config folder, where its default socket is: ~/.config/herdr,
    # and %APPDATA%\herdr on Windows (hide-platform's `host`).
    herdr_config = operator_home / ("AppData/Roaming/herdr" if WINDOWS else ".config/herdr")
    defaults = [herdr_config / "herdr.sock",
                operator_home / ".hide/state"]
    if operator_socket is not None:
        defaults.append(operator_socket)
    if any(socket.resolve() == path.resolve() for path in defaults):
        raise ProtectionError("operator_socket_refused")
    if socket.exists() or socket.is_symlink():
        raise ProtectionError("socket_already_exists")
    if len(os.fsencode(socket)) > 85:
        raise ProtectionError("socket_path_too_long")
    # Only the run's root or its owned short socket directory may contain it.
    parent = socket.parent
    if not beneath(socket, run):
        if (socket_root is None or parent != socket_root
                or not parent.is_dir() or parent.is_symlink()
                or not private(parent)):
            raise ProtectionError("socket_parent_not_owned_by_run")
    for forbidden in (operator_home / ".hide", herdr_config):
        if beneath(state, forbidden) or beneath(socket, forbidden):
            raise ProtectionError("operator_state_refused")


@dataclass(frozen=True)
class FileStamp:
    digest: str
    size: int
    mode: int
    identity: tuple[int, int]


def configuration_bytes(path: Path, *, dir_fd: int | None = None,
                        max_bytes: int = MAX_BACKUP_BYTES) -> tuple[FileStamp, bytes] | None:
    """Read and stamp one bounded ordinary file through the same descriptor."""
    if not 0 <= max_bytes <= MAX_BACKUP_BYTES:
        raise ProtectionError("config_read_budget_invalid")
    try:
        info = path.lstat() if dir_fd is None else os.stat(path, dir_fd=dir_fd, follow_symlinks=False)
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
        raise ProtectionError("config_not_private_regular_file")
    if info.st_size > max_bytes:
        raise ProtectionError("config_file_over_budget")
    # A regular pathname can become a FIFO between lstat and open. Do not
    # block on that replacement while trying to inspect its descriptor.
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=dir_fd)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        if (not stat.S_ISREG(opened.st_mode) or opened.st_uid != os.getuid()
                or opened.st_nlink not in {0, 1}):
            raise ProtectionError("config_not_private_regular_file")
        if opened.st_size > max_bytes:
            raise ProtectionError("config_file_over_budget")
        if opened.st_nlink == 0 or (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
            raise ConfigurationChanged("config_changed_during_open")
        data = stream.read(max_bytes + 1)
        after = os.fstat(stream.fileno())
    if len(data) > max_bytes or after.st_size > max_bytes:
        raise ProtectionError("config_file_over_budget")
    if not stat.S_ISREG(after.st_mode) or after.st_uid != os.getuid() or after.st_nlink not in {0, 1}:
        raise ProtectionError("config_not_private_regular_file")
    if after.st_nlink == 0:
        raise ConfigurationChanged("config_changed_during_read")
    if ((opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns, opened.st_mode, opened.st_nlink)
            != (after.st_size, after.st_mtime_ns, after.st_ctime_ns, after.st_mode, after.st_nlink)):
        raise ConfigurationChanged("config_changed_during_read")
    return (FileStamp(hashlib.sha256(data).hexdigest(), len(data),
                      stat.S_IMODE(opened.st_mode), (opened.st_dev, opened.st_ino)), data)


def stamp(path: Path, *, dir_fd: int | None = None, max_bytes: int = MAX_BACKUP_BYTES) -> FileStamp | None:
    """Hash ordinary owned files; reject aliases and unbounded reads."""
    value = configuration_bytes(path, dir_fd=dir_fd, max_bytes=max_bytes)
    return value[0] if value is not None else None


def shared_configuration_bytes(path: Path) -> tuple:
    """Retry only version races; an unavailable observation is not absence."""
    reason = None
    for _ in range(SHARED_READ_ATTEMPTS):
        try:
            return configuration_bytes(path), None
        except ConfigurationChanged as error:
            reason = str(error)
        except FileNotFoundError:
            reason = "config_changed_during_open"
    return None, reason


@dataclass
class ConfigInventory:
    entries: dict
    listed: set
    scanned: int
    excluded_subtrees: int
    omitted_entries_lower_bound: int
    uninspected_subtrees: int

    def summary(self) -> dict:
        return {"complete": not self.uninspected_subtrees,
                "scanned_entries": self.scanned,
                "excluded_boundaries": self.excluded_subtrees,
                "omitted_entries_lower_bound": self.omitted_entries_lower_bound,
                "uninspected_subtrees": self.uninspected_subtrees}

    def observed_absence(self, path: str) -> bool:
        current = Path(path)
        while current != current.parent:
            if str(current) in self.entries:
                return self.entries[str(current)]["kind"] == "absent"
            if str(current.parent) in self.listed:
                return True
            current = current.parent
        return False


def excluded_installation(path: Path) -> bool:
    # Registry/config files directly in plugins remain observable; the
    # installed code directories beneath it do not belong to this inventory.
    # Inspect ancestors too: an explicit nested root cannot reopen code.
    for current in (path, *path.parents):
        name = current.name.lower()
        if (name in {"node_modules", "extensions", "marketplace", "marketplaces", "bundled"}
                or "cache" in name or current.parent.name.lower() == "plugins"):
            return True
    return False


def inventory_directory(path: Path) -> int:
    """Open directories only, without following a link in any component."""
    descriptor = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def fingerprint(roots: list[Path]) -> ConfigInventory:
    """Bounded metadata only; never open a file or read a link target."""
    result, listed, pending = {}, set(), []
    scanned = excluded = omitted = 0

    def record(path, info):
        nonlocal excluded
        if info is None:
            result[str(path)] = {"kind": "absent"}
        elif excluded_installation(path if stat.S_ISDIR(info.st_mode) else path.parent):
            excluded += 1
            result[str(path)] = {"kind": "excluded"}
        elif stat.S_ISDIR(info.st_mode):
            result[str(path)] = {"kind": "directory"}
            pending.append(path)
        else:
            kind = "file" if stat.S_ISREG(info.st_mode) else "link" if stat.S_ISLNK(info.st_mode) else "other"
            result[str(path)] = {"kind": kind, "size": info.st_size,
                                 "mtime_ns": info.st_mtime_ns}

    for path in dict.fromkeys(roots):
        if scanned == MAX_CONFIG_FILES:
            omitted += 1
            continue
        scanned += 1
        try:
            info = path.lstat()
        except FileNotFoundError:
            info = None
        record(path, info)
    while pending and scanned < MAX_CONFIG_FILES:
        path = pending.pop()
        descriptor = inventory_directory(path)
        try:
            with os.scandir(descriptor) as entries:
                for entry in entries:
                    if scanned == MAX_CONFIG_FILES:
                        # One sentinel establishes a lower bound without
                        # walking an unbounded remainder just to count it.
                        omitted += 1
                        break
                    scanned += 1
                    record(path / entry.name, entry.stat(follow_symlinks=False))
                else:
                    listed.add(str(path))
        finally:
            os.close(descriptor)
    return ConfigInventory(result, listed, scanned, excluded, omitted,
                           len(pending) + omitted)


def shared_project_entries(data: bytes, format: str, run: Path) -> dict:
    """Inspect declared shared files; retain only this run's project keys."""
    try:
        document = json.loads(data) if format == "json" else tomllib.loads(data.decode("utf-8"))
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ProtectionError("shared_configuration_unreadable") from error
    if not isinstance(document, dict) or not isinstance(document.get("projects", {}), dict):
        raise ProtectionError("shared_projects_not_a_table")
    projects = document.get("projects", {})
    if len(projects) > MAX_CONFIG_FILES:
        raise ProtectionError("shared_projects_over_budget")
    return {key: value for key, value in projects.items()
            if Path(key).is_absolute() and Path(os.path.normpath(key)).is_relative_to(run)}


class ConfigGuard:
    """Backups plus explicit mutation ownership; observing a diff is not ownership.

    The live sandbox protects declared nonshared config files, not all native
    account/session state. A caller that deliberately
    performs a reversible, owned write must register its exact before/after
    stamps with record_write. Shared project files are observed only, and their
    private-path entries are named without recovery or failure. Other unknown
    declared-file changes fail recovery without overwriting the operator. Backups never
    contain a public report or log field.
    """

    def __init__(self, backup: Path, known: list[Path], roots: list[Path], *,
                 exclusive_root: Path | None = None,
                 shared: dict[Path, str] | None = None):
        private_directory(backup)
        self.backup = backup
        self.known = list(dict.fromkeys(known))
        self.roots = roots
        self.exclusive_root = exclusive_root
        self.shared = shared or {}
        if (any(path not in self.known or format not in {"json", "toml"}
                for path, format in self.shared.items())
                or (exclusive_root is not None and self.shared)):
            raise ProtectionError("invalid_shared_configuration_declaration")
        self.shared_before = {}
        self.shared_unavailable = {}
        if exclusive_root is not None and (not beneath(exclusive_root, backup.parent)
                                           or exclusive_root == backup.parent):
            raise ProtectionError("exclusive_recovery_root_must_be_owned_by_run")
        self.before = {}
        self.writes = {}
        self.inventory = fingerprint(roots)
        total = 0
        for index, path in enumerate(self.known):
            try:
                if path in self.shared:
                    value, reason = shared_configuration_bytes(path)
                    self.shared_unavailable[path] = reason
                else:
                    value = configuration_bytes(path)
                before = value[0] if value is not None else None
                self.before[path] = before
                if path in self.shared:
                    self.shared_before[path] = (shared_project_entries(value[1], self.shared[path], backup.parent.resolve())
                                                if value is not None else {})
                if value is not None:
                    total += value[0].size
                    if total > MAX_CONFIG_BYTES:
                        raise ProtectionError("config_backup_byte_budget")
                    write_private(backup / str(index), value[1])
            except (OSError, ProtectionError) as error:
                raise ProtectionError(str(error), path=path) from error
        write_private(backup / "index.json", json.dumps([
            {"path": str(path), "backup": str(index),
             "existed": None if self.shared_unavailable.get(path) else self.before[path] is not None,
             "observation": "unavailable" if self.shared_unavailable.get(path) else "observed"}
            for index, path in enumerate(self.known)
        ]).encode())

    def record_write(self, path: Path, before: FileStamp | None,
                     after: FileStamp | None) -> None:
        """Called by the owner of a write, never by a before/after scan."""
        # Filesystem rename has no conditional compare-and-swap against an
        # unrelated writer. Only a sole-owned disposable HOME may opt into
        # restoration. Declared nonshared writes are denied by the sandbox, and
        # an unexpected operator change is preserved and reported as failure.
        if self.exclusive_root is None or not beneath(path, self.exclusive_root):
            raise ProtectionError("operator_configuration_has_no_exclusive_writer")
        if path not in self.before or before != self.before[path]:
            raise ProtectionError("write_has_no_original_ownership")
        if after != stamp(path):
            raise ProtectionError("write_already_changed")
        self.writes[path] = after

    def finish(self) -> dict:
        changes = []
        failures = []
        shared_changes, shared_leftovers, shared_comparisons = [], [], []

        def observe_shared(path):
            value, reason = shared_configuration_bytes(path)
            before_reason = self.shared_unavailable[path]
            comparison = {"path": str(path), "complete": not reason and not before_reason,
                          "before": "unavailable" if before_reason else "present" if self.before[path] else "absent",
                          "after": "unavailable" if reason else "present" if value else "absent"}
            if before_reason:
                comparison["before_reason"] = before_reason
            if reason:
                comparison["after_reason"] = reason
            shared_comparisons.append(comparison)
            if reason:
                return
            current = value[0] if value is not None else None
            entries = (shared_project_entries(value[1], self.shared[path], self.backup.parent.resolve())
                       if value is not None else {})
            if not before_reason and current != self.before[path]:
                shared_changes.append({"path": str(path), "result": "다른 세션의 변경"})
            before_entries = self.shared_before[path]
            for key, value in sorted(entries.items()):
                change = ("not_compared" if before_reason else "added" if key not in before_entries
                          else "unchanged" if before_entries[key] == value else "changed")
                shared_leftovers.append({"path": str(path), "key": key, "change": change})

        def recover(index, path):
            original = self.before[path]
            current = stamp(path)
            if current == original:
                return
            if path not in self.writes or current != self.writes[path]:
                failures.append({"path": str(path), "reason": "unattributed_or_concurrent_change_preserved"})
                return
            # No replace is authorized merely because its bytes happen to match.
            # The owner has ended its writer before calling finish. A competing
            # writer after this check is outside the available filesystem CAS.
            if current != stamp(path):
                failures.append({"path": str(path), "reason": "concurrent_change_preserved"})
                return
            if original is None:
                if current is not None:
                    path.unlink()
            else:
                saved = configuration_bytes(self.backup / str(index))
                if saved is None or saved[0].digest != original.digest:
                    raise ProtectionError("backup_integrity_failed")
                data = saved[1]
                temporary = path.with_name(path.name + ".live-check-restore")
                write_private(temporary, data)
                try:
                    os.chmod(temporary, original.mode)
                    if current != stamp(path):
                        failures.append({"path": str(path), "reason": "concurrent_change_preserved"})
                        return
                    os.replace(temporary, path)
                finally:
                    temporary.unlink(missing_ok=True)
            restored = stamp(path)
            if (restored is None) != (original is None) or (restored and original and (restored.digest, restored.mode) != (original.digest, original.mode)):
                failures.append({"path": str(path), "reason": "restore_failed"})
            else:
                changes.append({"path": str(path), "result": "restored"})
        for index, path in enumerate(self.known):
            try:
                if path in self.shared:
                    observe_shared(path)
                else:
                    recover(index, path)
            except (OSError, ProtectionError) as error:
                # An alias, unreadable file or failed restore names its
                # subject and cannot skip the other safe comparisons.
                failures.append({"path": str(path), "reason": str(error)})
        try:
            after = fingerprint(self.roots)
        except (OSError, ProtectionError) as error:
            failures.append({"reason": "configuration_inventory_unavailable", "detail": str(error)})
            return {"restored": changes, "failures": failures,
                    "shared_changes": shared_changes, "shared_leftovers": shared_leftovers,
                    "shared_comparisons": shared_comparisons,
                    "directory_changes": None, "inventory_checked": False}
        directory_changes, uncompared = [], 0
        before_entries, after_entries = self.inventory.entries, after.entries
        for key in sorted(before_entries.keys() | after_entries.keys()):
            before_value, after_value = before_entries.get(key), after_entries.get(key)
            if before_value == after_value:
                continue
            if before_value is None and not self.inventory.observed_absence(key):
                uncompared += 1
                continue
            if after_value is None and not after.observed_absence(key):
                uncompared += 1
                continue
            kind = "added" if before_value is None or before_value["kind"] == "absent" else "removed" if after_value is None or after_value["kind"] == "absent" else "changed"
            directory_changes.append({"path": key, "kind": kind})
        return {"restored": changes, "failures": failures,
                "shared_changes": shared_changes, "shared_leftovers": shared_leftovers,
                "shared_comparisons": shared_comparisons,
                "directory_changes": directory_changes, "inventory_checked": True,
                "inventory": {"before": self.inventory.summary(), "after": after.summary(),
                              "uncompared_entries": uncompared}}
