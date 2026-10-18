"""Hash login/install state without Claude's live counters and unrelated projects."""

import hashlib
import json
from pathlib import Path

from agent_live_check.protection import ProtectionError, configuration_bytes

KEYS = ("oauthAccount", "primaryApiKey", "customApiKeyResponses", "installMethod", "autoUpdates",
        "hasCompletedOnboarding", "lastOnboardingVersion")


def snapshot(root, run):
    value = configuration_bytes((root / ".claude.json").resolve())
    if value is None:
        return None
    try:
        data = json.loads(value[1])
        if not isinstance(data, dict) or not isinstance(data.get("projects", {}), dict):
            raise ValueError("configuration shape")
        selected = {key: data[key] for key in KEYS if key in data}
        projects = sorted(name for name in data.get("projects", {})
                          if run is not None and Path(name).is_absolute()
                          and Path(name).resolve().is_relative_to(run.resolve()))
        return {"sha256": hashlib.sha256(json.dumps(selected, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
                "run_projects": projects}
    except (ValueError, TypeError):
        raise ProtectionError("operator_watch_claude_configuration_unreadable") from None
