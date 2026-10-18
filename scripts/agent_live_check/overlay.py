"""Supported private config roots, with bounded copies of existing login data.

HOME is never redirected for an authenticated CLI. A credential copy can be
refreshed only in the disposable tree; it is never written back to the account.
"""

from pathlib import Path
import os

from .credential_snapshot import snapshot
from .protection import MAX_BACKUP_BYTES, ProtectionError, beneath, private_directory, stamp, write_private


def prepare(recipe: dict, probe: Path, operator: Path, *, owner=None, env=None, root: Path | None = None) -> dict:
    data = recipe.get("overlay")
    if not data:
        return {"env": {}, "copies": [], "settings": [], "session_root": None}
    # The journey copies into the candidate's own config root under the probe.
    root = root or probe / ("config-" + recipe["id"])
    if not beneath(root, probe):
        raise ProtectionError("credential_copy_root_outside_probe")
    private_directory(root)
    total = 0
    copied = []
    for source_name, destination_name in data["copies"]:
        source, destination = operator / source_name, root / destination_name
        if (not beneath(source, operator) or not beneath(destination, root)
                or source.is_symlink()
                or any(parent.is_symlink() for parent in source.parents if beneath(parent, operator))):
            raise ProtectionError("credential_copy_alias_or_path_refused")
        if source.suffix == ".db":
            for member, original, content in snapshot(source, operator, MAX_BACKUP_BYTES - total,
                                                       owner, env if env is not None else dict(os.environ)):
                target = Path(str(destination) + str(member)[len(str(source)):])
                private_directory(target.parent)
                write_private(target, content)
                if stamp(target).digest != original.digest:
                    raise ProtectionError("credential_private_copy_integrity_failure")
                copied.append({"source": str(member.relative_to(operator)),
                               "private_name": str(target.relative_to(root))})
                total += len(content)
            continue
        before = stamp(source)
        if before is None:
            continue
        total += before.size
        if total > MAX_BACKUP_BYTES:
            raise ProtectionError("credential_copy_total_over_budget")
        descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as stream:
            opened = os.fstat(stream.fileno())
            if (opened.st_dev, opened.st_ino) != before.identity:
                raise ProtectionError("credential_changed_during_private_copy")
            content = stream.read(MAX_BACKUP_BYTES + 1)
        if (len(content) > MAX_BACKUP_BYTES or stamp(source) != before
                or any(parent.is_symlink() for parent in source.parents if beneath(parent, operator))):
            raise ProtectionError("credential_changed_during_private_copy")
        private_directory(destination.parent)
        write_private(destination, content)
        if stamp(destination).digest != before.digest:
            raise ProtectionError("credential_private_copy_integrity_failure")
        copied.append({"source": source_name, "private_name": destination_name})
    environment = {key: str(root / relative) for key, relative in data["env"].items()}
    for value in environment.values():
        private_directory(Path(value))
    return {"env": environment, "copies": copied, "provenance": data["provenance"],
            "settings": [root / name for name in data["settings"]],
            "session_root": root / data["sessions"] if data.get("sessions") else None}
