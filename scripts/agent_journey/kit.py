"""The integration cell: the hook the candidate's kit installed where the
agent reads it is the one that registered the agent's session.

Hide's own record of what its kit put on this Mac is the machine's kit row in
the snapshot (`navigator.devices[].kit.agents[]`): its hook piece, with a
state and a location. An installed hook in the HOME the agent reads its
configuration from, and a session Hide's registration ledger names, say the
agent loaded the candidate's hook and not one of the operator's.
"""

from pathlib import Path

from agent_live_check.processes import ProcessError

from . import cells

CELL = "integration"
# The kit row is read once the kit pass that installed the pieces published.
KIT_SECONDS = 60


def local_device(rest):
    """The core's own machine row, or None before the core published it."""
    return next((device for device in (rest.get("navigator") or {}).get("devices") or []
                 if device.get("kind") == "local"), None)


def kit_agent(rest, agent):
    """The kit's row for adapter `agent` on this Mac, or None."""
    device = local_device(rest) or {}
    return next((row for row in (device.get("kit") or {}).get("agents") or [] if row.get("id") == agent), None)


def verdict(row, session, home):
    """`row` the kit's agent row, `session` the pane's session as Hide's
    registration ledger names it, `home` where the agent reads its
    configuration."""
    if not row:
        return cells.FAIL, "the kit has no row for the agent on this Mac"
    piece = row.get("hook") or {}
    if piece.get("state") != "installed":
        return cells.FAIL, f"the kit's hook piece is {piece.get('state')}" + (
            f": {piece['reason']}" if piece.get("reason") else "")
    location = piece.get("location")
    if not location or not Path(location).resolve().is_relative_to(Path(home).resolve()):
        return cells.FAIL, f"the kit's hook piece is at {location}, outside the HOME the agent reads"
    status, reason = cells.emitter_verdict(session)
    if status != cells.PASS:
        return status, reason
    return cells.PASS, f"the kit installed Hide's hook in the agent's HOME, and {reason}"


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return

    def installed(rest):
        row = kit_agent(rest, journey.id) or {}
        return (row.get("hook") or {}).get("state") == "installed" and row
    try:
        journey.daemon.wait(installed, KIT_SECONDS, "the kit row to read the agent's pieces installed")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        journey.record.setdefault(CELL, {})["kit_wait"] = str(error)
    row = kit_agent(journey.daemon.snapshot() or {}, journey.id)
    session = (journey.engine_agent() or {}).get("agent_session")
    record = journey.record.setdefault(CELL, {})
    record.update(kit=row, session=session, home=str(journey.runtime.home))
    if not row:
        record["devices"] = [{"id": device.get("id"), "kind": device.get("kind"), "state": device.get("state"),
                              "agents": [agent.get("id") for agent in (device.get("kit") or {}).get("agents") or []]}
                             for device in ((journey.daemon.snapshot() or {}).get("navigator") or {}).get("devices") or []]
    journey.settle(CELL, verdict(row, session, journey.runtime.home), journey.evidence(CELL, record))
