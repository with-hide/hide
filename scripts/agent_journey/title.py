"""The title cell: Hide shows the agent's task under a title, not under the
agent's name.

A row's `identity_label` is one title order the core keeps (docs/status-model.md,
Task identity): the operator's own title, the goal Hide's labels made, the
agent's own native title, and last the agent's name, which the contract
names (`untitled`). The cell measures the native title the contract's
`title` form declares, made the way the operator makes it: an `automatic`
title the agent writes itself after it answered (its own model request,
which the journey's model answers), or a `named` one the operator gives it
with the agent's naming command (Pi's `/name`), which the row must then show.
Hide's labels are off in the journey (`local.disable_hide_ai`), so a goal they
would make is not measured here; whether their sentences reached the row is
reported beside the verdict.
"""

from agent_live_check.processes import ProcessError

from . import cells

CELL = "title"
# Hide's labels read the session when a turn's state moves and answer within
# their provider's deadline (60 s); a session not written yet is read again
# 15 s later.
TITLE_SECONDS = 120


def verdict(row, untitled, named=None):
    """`named` the name the operator gave the session, which the row must show."""
    if not row:
        return cells.FAIL, "the agent's row is gone"
    label = row.get("identity_label")
    if named is not None and label != named:
        return cells.FAIL, f"the row shows {label!r}, not {named!r}, the name the operator gave the session"
    if not label or label == untitled:
        return cells.FAIL, f"the row still shows {label!r}, the agent's name, as its title"
    laid = "with Hide's label sentences" if row.get("message") or (row.get("request") or {}).get("line") else \
        "without Hide's label sentences"
    source = "the name the operator gave it" if named is not None else "its own title"
    return cells.PASS, f"the row's title is {label!r}, {source}, {laid}"


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    untitled = journey.row["untitled"]
    form = journey.row.get("title")
    record = journey.record.setdefault(CELL, {})
    named = None
    if isinstance(form, dict) and form.get("named"):
        # The agent writes no title of its own: name the session as the operator does.
        from .journey import token
        named = token()
        record["named"] = {"command": form["named"], "name": named}
        if not journey.type_prompt(record["named"], f"{form['named']} {named}"):
            journey.settle(CELL, (cells.FAIL, "the pane's terminal never took the naming command (transport "
                                  + " -> ".join(map(str, record["named"]["terminal"])) + ")"),
                           journey.evidence(CELL, record))
            return

    def titled(_rest):
        label = (journey.daemon.row(journey.pane) or {}).get("identity_label")
        return label == named if named is not None else label not in (None, "", untitled)
    try:
        journey.daemon.wait(titled, TITLE_SECONDS, "the row to show a task title")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        record["wait"] = str(error)
    row = journey.daemon.row(journey.pane)
    record["row"] = {key: (row or {}).get(key) for key in ("identity_label", "message", "detail", "request")}
    journey.settle(CELL, verdict(row, untitled, named), journey.evidence(CELL, record))
