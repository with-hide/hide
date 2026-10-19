"""The guidance_memory cell: the start guidance and a Project Memory card
reach the agent at its start.

The run writes one rule card into its private state for the measured
repository before any agent starts (`hide-memory`'s `journey_memory`
example, through the store's own writer). Hide gives a session the card
index in its start guidance, one title per card; the card's title carries a
fresh token, and a turn after the start asks for it. The question shares no
word with the card, so Hide's prompt-time Memory, which picks cards by the
prompt's words, never gives it: an answer carrying the token came from what
the start gave, and is read, as every answer is, by Hide's reader.
"""

import json

from agent_live_check.processes import ProcessError

from . import cells
from .kit import local_device

CELL = "guidance_memory"
QUESTION = "Run no tool. Reply with exactly the uppercase code in the title of your only Project Memory card."


def title(value):
    """The card's one line in the start index; no word of it is in QUESTION."""
    return f"Marker {value} for journey checks"


def seed(runtime, daemon, repository, writer, value):
    """Writes the run's card, whose title carries the token `value`; its id,
    token and project. A refusal stops the run before any agent starts, as a
    setup failure."""
    device = daemon.wait(lambda rest: (local_device(rest) or {}).get("id"), 30, "the core's own machine row")
    code, out, err = runtime.owner.run([str(writer), str(runtime.state), str(repository), device, title(value)],
                                       env=runtime.env, check=False)
    if code:
        raise ProcessError("journey_memory_card_refused: " + (err.strip() or out.strip())[:300])
    try:
        answer = json.loads(out)
    except ValueError as error:
        raise ProcessError("journey_memory_card_answer_malformed: " + out[:200]) from error
    return {"card": answer["card"], "project": answer["project"], "token": value, "title": title(value),
            "device": device}


def measure(journey, card):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    record = {"card": card, "question": QUESTION}
    journey.record[CELL] = record
    events = len(journey.conversation().get("events", []))
    since = journey.daemon.mark(journey.pane)
    if not journey.type_prompt(record, QUESTION):
        journey.settle(CELL, (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                              + " -> ".join(map(str, record["terminal"])) + ")"), journey.evidence(CELL, record))
        return
    turn, read, record["activities"] = journey.wait_turn(since, card["token"], lambda _events: events)
    record["turn"] = turn
    record["read"] = {**read, "events": read.get("events", [])[events:]} if "events" in read else read
    status, reason = cells.with_turn(cells.reply_verdict(read, journey.session, card["token"], events), turn)
    if status == cells.PASS:
        reason = f"the start gave card {card['card']}: " + reason
    journey.settle(CELL, (status, reason), journey.evidence("guidance-memory", record))
