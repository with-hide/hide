"""Admission and operator watch for a local keyless journey, never a login run."""

import json
import os
from pathlib import Path
import stat

from agent_live_check.protection import ProtectionError, write_private

WATCH = (".claude", ".codex", ".gemini", ".grok", ".omp", ".pi")
LINK = ".local/bin/claude"


class OperatorWatch:
    """Only metadata, never configuration content; the root is supplied by the caller."""

    def __init__(self, root):
        self.root = root
        self.before = self.snapshot()

    def snapshot(self):
        result = {}
        for name in (*WATCH, LINK):
            path = self.root / name
            try:
                info = path.lstat()
                value = {"mtime_ns": info.st_mtime_ns, "type": stat.S_IFMT(info.st_mode)}
                if path.is_symlink():
                    value["target"] = os.readlink(path)
                    try:
                        value["target_mtime_ns"] = path.stat().st_mtime_ns
                    except FileNotFoundError:
                        value["target_missing"] = True
                result[name] = value
            except FileNotFoundError:
                result[name] = None
        return result

    def compare(self):
        after = self.snapshot()
        return {"before": self.before, "after": after,
                "changed": [name for name in self.before if self.before[name] != after[name]]}

    def check(self):
        result = self.compare()
        if result["changed"]:
            raise ProtectionError("operator_watch_changed: " + ", ".join(result["changed"]))
        return result


def exception(row, pin):
    """A missing proof preserves the existing operator-HOME route."""
    declaration = row.get("fake_home") or {}
    if declaration.get("status") != "proven":
        return declaration.get("reason", "The adapter declares no private HOME proof.")
    if not pin or pin.get("pinned") or pin.get("installer") not in ("npm", "grok", "omp"):
        return "The installer cannot select a pinned release."
    if not declaration.get("key_env") or not declaration.get("evidence"):
        return "The adapter declares no API-key environment proof."
    if declaration.get("auto_update", {}).get("kind") not in ("environment", "config"):
        return "The adapter declares no auto-update disable policy."
    return None


def configure_update(declaration, home):
    """Apply the Rust declaration after the provider config, before any CLI starts."""
    update = declaration["auto_update"]
    if update["kind"] == "environment":
        return {update["name"]: update["value"]}
    path = home / update["path"]
    value = json.loads(update["value"])
    replacement = path.with_suffix(path.suffix + ".next")
    if path.suffix == ".toml":
        # Root TOML values must precede the provider's tables.
        write_private(replacement, (f'{update["key"]} = {update["value"]}\n' + path.read_text()).encode())
    elif path.suffix == ".json":
        data = json.loads(path.read_text())
        data[update["key"]] = value
        write_private(replacement, json.dumps(data).encode())
    elif path.suffix in (".yml", ".yaml"):
        write_private(replacement, (f'{update["key"]}: {update["value"]}\n' + path.read_text()).encode())
    else:
        raise ProtectionError("unsupported_update_config_format")
    replacement.replace(path)
    return {}
