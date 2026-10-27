"""The generated adapter contract shared by the runner and its CI verdict."""

import json

from agent_live_check.protection import ProtectionError

from .cells import CELLS, EXCLUDED_AGENTS


def load_contract(checkout):
    path = checkout / "contracts/agent-journey.json"
    try:
        rows = json.loads(path.read_text())
        valid = isinstance(rows, list) and bool(rows)
        ids = set()
        for row in rows if valid else ():
            key = row.get("id") if isinstance(row, dict) else None
            declared = row.get("cells") if isinstance(row, dict) else None
            if (not isinstance(key, str) or not key or key in ids
                    or not isinstance(declared, dict) or set(declared) != set(CELLS)
                    or any(type(value) is not bool for value in declared.values())):
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


def misses(row, report):
    """What one system's fake-lane `report` shows against the agent `row`
    declares: its CLI missing or another version than the table's, no row,
    and each cell that did not come out as declared, with its reason. An
    empty list is a pass on that system."""
    agent = row["id"]
    found = []
    entry = next((item for item in report.get("agents", []) if item.get("id") == agent), {})
    if entry.get("cli_missing"):
        found.append(f"{entry['cli_missing']} is not installed")
    pin = entry.get("pin") or {}
    if pin and not pin.get("matches"):
        found.append(f"answered {entry.get('version')!r}, not {pin.get('version')}")
    cells = report.get("table", {}).get(agent)
    if not isinstance(cells, dict):
        return found + ["no row"]
    for name in CELLS:
        expected = "pass" if row["cells"][name] else "N/A"
        cell = cells.get(name) or {}
        if cell.get("status") != expected:
            reason = str(cell.get("reason") or "")[:160]
            found.append(f"{name} is {cell.get('status', 'absent')}" + (f" ({reason})" if reason else ""))
    return found
