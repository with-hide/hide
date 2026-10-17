"""The title cell: Hide shows the agent's task under a title, not under the
agent's name.

A row's `identity_label` is one title order the core keeps (docs/status-model.md,
Task identity): the operator's own title, the goal Hide's labels made, the
agent's own automatic title, and last the agent's name, which the contract
names (`untitled`). The cell passes once the row shows a title other than
that name after the agent answered; whether Hide's label sentences reached
the row is reported beside it.
"""

from agent_live_check.processes import ProcessError

from . import cells

CELL = "title"
# Hide's labels read the session when a turn's state moves and answer within
# their provider's deadline (60 s); a session not written yet is read again
# 15 s later.
TITLE_SECONDS = 120


def verdict(row, untitled):
    if not row:
        return cells.FAIL, "the agent's row is gone"
    label = row.get("identity_label")
    if not label or label == untitled:
        return cells.FAIL, f"the row still shows {label!r}, the agent's name, as its title"
    laid = "with Hide's label sentences" if row.get("message") or (row.get("request") or {}).get("line") else \
        "without Hide's label sentences"
    return cells.PASS, f"the row's title is {label!r}, {laid}"


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    untitled = journey.row["untitled"]
    try:
        journey.daemon.wait(lambda _rest: (row := journey.daemon.row(journey.pane)) and row.get("identity_label")
                            not in (None, "", untitled), TITLE_SECONDS, "the row to show a task title")
    except ProcessError as error:
        if journey.runtime.owner.cancelled.is_set():
            raise
        journey.record[CELL] = {"wait": str(error)}
    row = journey.daemon.row(journey.pane)
    record = journey.record.setdefault(CELL, {})
    record["row"] = {key: (row or {}).get(key) for key in ("identity_label", "message", "detail", "request")}
    journey.settle(CELL, verdict(row, untitled), journey.evidence(CELL, record))
