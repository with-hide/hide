"""Admission and operator watch for a local keyless journey, never a login run."""

import json
import os
from pathlib import Path
import stat

from agent_live_check.protection import MAX_CONFIG_FILES, ProtectionError, stamp, write_private
from . import claude_watch, database_watch

WATCH = (".claude", ".codex", ".gemini", ".grok", ".omp", ".pi", ".local/share/opencode")
LINK = ".local/bin/claude"
# Configuration and login bytes, never session/history append records.
# Sources and the exact list are kept in docs/VERIFICATION.md.
CONFIGURATION = (
    ".claude/settings.json", ".claude/settings.local.json", ".claude/.credentials.json",
    ".codex/config.toml", ".codex/auth.json", ".codex/hooks.json", ".codex/.credentials.json",
    ".grok/config.toml", ".grok/auth.json", ".grok/trusted_folders.toml", ".grok/mcp_credentials.json",
    ".pi/agent/settings.json", ".pi/agent/models.json", ".pi/agent/auth.json",
    ".pi/agent/oauth.json", ".pi/agent/mcp-auth.json", ".pi/agent/keybindings.json", ".pi/agent/trust.json", ".pi/agent/mcp.json",
    ".omp/agent/config.yml", ".omp/agent/config.yaml", ".omp/agent/models.yml",
    ".omp/agent/settings.json", ".omp/agent/secrets.yml", ".omp/agent/secret-placeholder.key",
    ".config/opencode/opencode.json", ".config/opencode/opencode.jsonc",
    ".config/opencode/tui.json", ".config/opencode/cli.json", ".config/opencode/service.json",
)


class OperatorWatch:
    """Read bounded hashes and names; retain no configuration or login bytes."""

    def __init__(self, root, scratch=None, run=None):
        self.root, self.scratch, self.run = root, scratch, run
        self.copies = []
        self.before = self.snapshot()

    def snapshot(self):
        result = {}
        entries = 0
        for name in (*WATCH, LINK):
            path = self.root / name
            try:
                info = path.lstat()
                value = {"type": stat.S_IFMT(info.st_mode)}
                if path.is_symlink():
                    value["target"] = os.readlink(path)
                if name in WATCH and path.is_dir():
                    names = []
                    with os.scandir(path) as children:
                        for child in children:
                            entries += 1
                            if entries > MAX_CONFIG_FILES:
                                raise ProtectionError("operator_watch_entry_budget_exceeded")
                            names.append(child.name)
                    value["entries"] = sorted(names)
                result[name] = value
            except FileNotFoundError:
                result[name] = None
        for name in CONFIGURATION:
            path = self.root / name
            # A user may keep settings through a symlink; compare its target
            # and the bounded ordinary file it names, without retaining bytes.
            target = os.readlink(path) if path.is_symlink() else None
            value = stamp(path.resolve())
            result[name] = {"sha256": value.digest, "target": target} if value else None
        result[".claude.json"] = claude_watch.snapshot(self.root, self.run)
        for name in database_watch.DATABASES:
            result[name] = database_watch.digest(self.root, self.scratch, name, self.copies)
        return result

    def compare(self):
        after = self.snapshot()
        changed = [name for name in self.before if self.before[name] != after[name]]
        if (after.get(".claude.json") or {}).get("run_projects") and ".claude.json" not in changed:
            changed.append(".claude.json")
        return {"before": self.before, "after": after, "database_copies": self.copies, "changed": changed}

    def check(self, result=None):
        result = self.compare() if result is None else result
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
    update = declaration.get("auto_update") or {}
    fields = {"environment": ("name", "value"), "config": ("path", "key", "value")}.get(update.get("kind"))
    if not fields or any(not update.get(field) for field in fields):
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
