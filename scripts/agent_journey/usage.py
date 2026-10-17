"""The usage cell: Hide reads the tokens the agent's turns spent.

Usage rides the snapshot beside the rest (`payload.usage`): per node, per agent
kind, the consumption Hide's usage ledger read from the agent's session files
under the daemon's HOME. The ledger publishes often only while a shell window
is on screen, which the shell says with the `usage_window_visible` hint; the
journey says it as that window does. The run's HOME holds only this run's
sessions, so a reading with calls in it is this run's turns.
"""

from pathlib import Path

from agent_live_check.processes import ProcessError

from . import cells
from .kit import local_device

CELL = "usage"
# The ledger looks for files every 3 s and publishes every 5 s while a
# window is on screen; the first read of a session folder takes longer.
USAGE_SECONDS = 90
SHELL_OMITS = ("workspace_registrations", "device_registrations", "sidebar_width", "interface_language",
               "collapsed_workspace_ids", "expanded_checkout_ids", "expanded_agent_pane_ids",
               "sessions_expanded_agent_pane_ids", "selected_pane_id", "focused_checkout_id", "focused_device_id")


def consumption(usage, node, kind):
    """The consumption cell of agent `kind` on `node`, or None."""
    entry = next((item for item in (usage or {}).get("nodes") or [] if item.get("node") == node), None)
    agent = next((item for item in (entry or {}).get("agents") or [] if item.get("agent") == kind), None)
    return (agent or {}).get("consumption")


def verdict(cell):
    if not cell:
        return cells.FAIL, "Hide's usage has no row for the agent on this machine"
    if cell.get("state") != "reading":
        return cells.FAIL, f"the agent's usage reads {cell.get('state')}" + (
            f" ({cell['cause']})" if cell.get("cause") else "")
    amount = (cell.get("value") or {}).get("popover", {}).get("amount") or {}
    if not amount.get("calls"):
        return cells.FAIL, "the agent's usage reads no model call from the run's turns"
    return cells.PASS, f"Hide read {amount['calls']} model calls and {amount.get('new_tokens', 0)} new tokens"


def observing(daemon, visible):
    """Says what the shell says while a window is on screen, or after it left."""
    rest = daemon.snapshot() or {}
    # The UI state the shell sends with a hint: its own, less what other
    # events own (web/src/actions.ts, updateUiState).
    state = {key: value for key, value in (rest.get("ui_state") or {}).items() if key not in SHELL_OMITS}
    daemon.dispatch("ui_state_update", {**state, "usage_window_visible": visible})


def unreadable(journey):
    """Why Hide cannot read this agent's sessions in this mode, or None."""
    if Path(journey.sessions).resolve() == Path(journey.runtime.home).resolve():
        return None
    return (f"the agent writes its sessions under {journey.sessions}, and the candidate's usage ledger reads only "
            "its own HOME; the CI mode measures it")


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    reason = unreadable(journey)
    if reason:
        cells.settle(journey.table, journey.id, CELL, cells.NOT_MEASURED, reason)
        return
    node = (local_device(journey.daemon.snapshot() or {}) or {}).get("id")
    observing(journey.daemon, True)
    record = {"node": node}
    journey.record[CELL] = record
    try:
        journey.daemon.wait(lambda _rest: verdict(consumption(journey.daemon.usage_section(), node,
                                                              journey.kind))[0] == cells.PASS,
                            USAGE_SECONDS, "Hide's usage to read the run's model calls")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        record["wait"] = str(error)
    finally:
        observing(journey.daemon, False)
    cell = consumption(journey.daemon.usage_section(), node, journey.kind)
    record["consumption"] = cell
    journey.settle(CELL, verdict(cell), journey.evidence(CELL, record))
