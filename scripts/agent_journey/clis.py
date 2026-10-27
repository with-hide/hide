"""The agent CLI versions CI installs (contracts/agent-clis.json), read by
scripts/install-agent-clis.py to install them and by the journey to say in
its report whether the CLI it measured answered the pinned version.

`HIDE_AGENT_CLIS_TABLE` names another table of the same shape, which the
daily agent canary (`.github/workflows/agent-canary.yml`) writes with each
vendor's latest release (scripts/agent-cli-latest.py), so the install, the
report and the lane's verdict all measure those versions instead."""

import json
import os
from pathlib import Path
import re

PINNED = Path(__file__).resolve().parents[2] / "contracts/agent-clis.json"
TABLE = Path(os.environ.get("HIDE_AGENT_CLIS_TABLE") or PINNED)


def table():
    """Each CLI's row by its Herdr kind."""
    return json.loads(TABLE.read_text())["clis"]


def matches(version, answer):
    """Whether a CLI's `--version` answer names exactly `version`, not a longer one it starts."""
    return re.search(r"(?<![\d.])" + re.escape(version) + r"(?![\d.])", answer) is not None


def pin(kind, answer):
    """What the report says of the measured CLI against the table, or None for a CLI it does not pin."""
    row = table().get(kind)
    if not row:
        return None
    return {"version": row["version"], "matches": matches(row["version"], answer),
            **({"pinned": row["pinned"]} if row.get("pinned") else {})}
