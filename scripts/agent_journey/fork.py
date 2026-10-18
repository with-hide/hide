"""The fork cell: Fork agent makes the operator's own copy of the agent's
conversation, and Hide names where it came from.

The operator chooses Fork agent from the pane menu, which sends `fork_pane`
with the tab placement, and types into the fork to use it; the journey sends
the same event on its own screen and types one prompt into the fork's pane.
An agent that tells its session only at its first turn, as Codex does, is
registered as a fork only after it. Hide then shows a new row whose
`forked_from` names the source's pane, with a session of its own that
Herdr's integration reported and in which Hide's reader finds the answer, and
`hide agent list` registers the fork as a root (no parent) whose origin is
the source (docs/status-model.md, Where a parent comes from).
"""

from agent_live_check.processes import ProcessError

from . import cells

CELL = "fork"
# The fork starts the agent again; a CLI's start takes seconds, and Hide
# registers a fork once Herdr reports the fork's session.
FORK_SECONDS = 90


def forked_row(rest, source):
    """The row Hide shows as forked from pane `source`, or None."""
    return next((row for row in (rest.get("navigator") or {}).get("agents") or []
                 if (row.get("forked_from") or {}).get("pane_id") == source), None)


def panes(runtime):
    """The pane ids the run's Herdr holds now."""
    return {pane["pane_id"] for pane in runtime.json(["api", "snapshot"])["snapshot"].get("panes", [])}


def verdict(row, source_session, herdr_session, source, listed, source_agent):
    """`row` the forked row, `herdr_session` its pane's session as Herdr
    reports it, `listed` its `hide agent list` entry."""
    if not row:
        return cells.FAIL, "no row is shown as forked from the source"
    session = row.get("session_id")
    if not session or session == source_session:
        return cells.FAIL, f"the forked row's session is {session}, not one of its own"
    status, reason = cells.emitter_verdict(herdr_session, source)
    if status != cells.PASS:
        return status, "the fork's " + reason[0].lower() + reason[1:]
    if (herdr_session or {}).get("value") != session:
        return cells.FAIL, f"Herdr reports session {(herdr_session or {}).get('value')} in the fork, Hide {session}"
    if not listed:
        return cells.FAIL, "hide agent list has no registration for the fork's pane"
    if listed.get("parent") is not None or listed.get("origin") != source_agent:
        return cells.FAIL, (f"the fork is registered with parent {listed.get('parent')} and origin "
                            f"{listed.get('origin')}, not as a root from {source_agent}")
    return cells.PASS, (f"forked from {row['forked_from']['pane_id']} into its own session {session}, a root "
                        f"whose origin is {source_agent}")


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    record = {"source": {"pane": journey.pane, "agent": journey.agent}}
    journey.record[CELL] = record
    source_session = (journey.daemon.row(journey.pane) or {}).get("session_id")
    record["source"]["session"] = source_session
    before = panes(journey.runtime)
    journey.daemon.dispatch("fork_pane", {"pane_id": journey.pane, "placement": "tab"})
    try:
        journey.runtime.wait(lambda: panes(journey.runtime) - before, FORK_SECONDS)
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        record["pane_wait"] = str(error)
    # Only this agent runs, so every pane that appeared since is the fork's,
    # marked by Hide or not; the journey closes each one with the agent's.
    opened = record["opened"] = sorted(panes(journey.runtime) - before)
    journey.opened.extend(opened)
    if len(opened) != 1:
        journey.settle(CELL, (cells.FAIL, f"Fork agent opened {len(opened)} panes, not one"),
                       journey.evidence(CELL, record))
        return
    turn, read, value = journey.turn_in(opened[0], record.setdefault("prompt", {}))

    def forked(rest):
        row = forked_row(rest, journey.pane)
        return row if row and row.get("session_id") and row["session_id"] != source_session else None
    try:
        journey.daemon.wait(forked, FORK_SECONDS, "a row forked from the source with a session of its own")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        record["fork_wait"] = str(error)
    rest = journey.daemon.snapshot() or {}
    row = record["row"] = forked_row(rest, journey.pane)
    pane = (row or {}).get("pane_id")
    herdr_session = (journey.runtime.agent(pane) or {}).get("agent_session") if pane else None
    if not row:
        # A fork Hide did not mark: what Herdr and Hide show in its pane.
        record["unmarked"] = {"herdr_session": (journey.runtime.agent(opened[0]) or {}).get("agent_session"),
                              "hide_row": {key: (journey.daemon.row(opened[0]) or {}).get(key)
                                           for key in ("session_id", "forked_from", "identity_label")}}
    listing = journey.controller.hide(["agent", "list"])
    listed = next((item for item in (listing.get("value") or {}).get("items") or []
                   if pane and item.get("pane") == pane), None)
    record["prompt"].update(turn=turn, read=read)
    record.update(herdr_session=herdr_session, listed=listed, errors=list(journey.daemon.errors))
    status, reason = verdict(row, source_session, herdr_session, journey.row["herdr_source"], listed, journey.agent)
    if status == cells.PASS:
        answer = cells.with_turn(cells.reply_verdict(read, row["session_id"], value), turn)
        status, reason = (status, reason + "; " + answer[1]) if answer[0] == cells.PASS else \
            (answer[0], "the fork's answer: " + answer[1])
    journey.settle(CELL, (status, reason), journey.evidence(CELL, record), pane=opened[0])
