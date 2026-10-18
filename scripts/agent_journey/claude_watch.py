"""Hash login/install state without Claude's live counters and unrelated projects."""

import hashlib
import json
from pathlib import Path

from agent_live_check.protection import ProtectionError, configuration_bytes

KEYS = ("oauthAccount", "primaryApiKey", "customApiKeyResponses", "installMethod", "autoUpdates",
        "hasCompletedOnboarding", "lastOnboardingVersion")


def snapshot(root, markers):
    try:
        path = root / ".claude.json"
        value = configuration_bytes(path.resolve())
        if value is None:
            return {"marker": markers.link(path)} if path.is_symlink() else None
        data = json.loads(value[1])
        if not isinstance(data, dict) or not isinstance(data.get("projects", {}), dict):
            raise ValueError("configuration shape")
        selected = {key: data[key] for key in KEYS if key in data}
        project_trace = any(Path(name).is_absolute() and markers.link(Path(name))
                            for name in data.get("projects", {}))
        return {"sha256": hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                "marker": markers.link(path) or project_trace or markers.contains(value[1])}
    except (OSError, RuntimeError, ValueError, TypeError):
        raise ProtectionError("operator_watch_claude_configuration_unreadable") from None
