"""The integration cell: the Herdr integration that reported the agent's
session is the one the candidate's kit installed where the agent reads it.

Hide's own record of what its kit put on this Mac is the machine's kit row in
the snapshot (`navigator.devices[].kit.agents[]`): its hook and Herdr
integration pieces, each with a state and a location. The kit reads Herdr's
own `integration status` for the second, and calls it installed only while
Herdr lists it current. Herdr names the integration that reported a session
in the session's source, the contract's `herdr_source`
(`herdr:<integration target>`). Both together, with both pieces in
the HOME the agent reads its configuration from, say the agent loaded the
candidate's integration and not one of the operator's.
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


def verdict(row, session, source, home):
    """`row` the kit's agent row, `session` the pane's session as Herdr
    reports it, `home` where the agent reads its configuration."""
    if not row:
        return cells.FAIL, "the kit has no row for the agent on this Mac"
    for name in ("hook", "herdr"):
        piece = row.get(name) or {}
        if piece.get("state") != "installed":
            return cells.FAIL, f"the kit's {name} piece is {piece.get('state')}" + (
                f": {piece['reason']}" if piece.get("reason") else "")
        location = piece.get("location")
        if not location or not Path(location).resolve().is_relative_to(Path(home).resolve()):
            return cells.FAIL, f"the kit's {name} piece is at {location}, outside the HOME the agent reads"
    status, reason = cells.emitter_verdict(session, source)
    if status != cells.PASS:
        return status, reason
    return cells.PASS, ("the kit installed Hide's hook and Herdr's current integration in the agent's HOME, "
                        "and " + reason)


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return

    def installed(rest):
        row = kit_agent(rest, journey.id) or {}
        return all((row.get(name) or {}).get("state") == "installed" for name in ("hook", "herdr")) and row
    try:
        journey.daemon.wait(installed, KIT_SECONDS, "the kit row to read the agent's pieces installed")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        journey.record.setdefault(CELL, {})["kit_wait"] = str(error)
    row = kit_agent(journey.daemon.snapshot() or {}, journey.id)
    session = (journey.herdr_agent() or {}).get("agent_session")
    record = journey.record.setdefault(CELL, {})
    record.update(kit=row, session=session, home=str(journey.runtime.home))
    if not row:
        record["devices"] = [{"id": device.get("id"), "kind": device.get("kind"), "state": device.get("state"),
                              "agents": [agent.get("id") for agent in (device.get("kit") or {}).get("agents") or []]}
                             for device in ((journey.daemon.snapshot() or {}).get("navigator") or {}).get("devices") or []]
    journey.settle(CELL, verdict(row, session, journey.row["herdr_source"], journey.runtime.home), journey.evidence(CELL, record))
