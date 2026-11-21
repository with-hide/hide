"""The generated adapter contract shared by the runner and its CI verdict."""

import json

from agent_live_check.protection import ProtectionError

from .cells import ALL_SCENES, CELLS, EXCLUDED_AGENTS


def load_contract(checkout):
    path = checkout / "contracts/agent-journey.json"
    try:
        rows = json.loads(path.read_text())
        valid = isinstance(rows, list) and bool(rows)
        ids = set()
        for row in rows if valid else ():
            key = row.get("id") if isinstance(row, dict) else None
            declared = row.get("cells") if isinstance(row, dict) else None
            scenes = row.get("scenes") if isinstance(row, dict) else None
            if (not isinstance(key, str) or not key or key in ids
                    or not isinstance(declared, dict) or set(declared) != set(CELLS)
                    or any(type(value) is not bool for value in declared.values())
                    # Every agent declares all fourteen scenes, each with a door or an absence.
                    or not isinstance(scenes, list) or [case.get("scene") for case in scenes] != list(ALL_SCENES)):
                valid = False
                break
            ids.add(key)
        if valid:
            return rows
    except (OSError, ValueError):
        pass
    raise ProtectionError("journey_contract_unreadable: regenerate contracts/agent-journey.json with export_journey_contract")


def active_rows(rows):
    """Every registered adapter participates unless the operator excluded it."""
    return [row for row in rows if row["id"] not in EXCLUDED_AGENTS]
