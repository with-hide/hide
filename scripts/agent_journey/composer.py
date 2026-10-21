"""The composer cell: the agent's composer reads empty or holding text the way
the operator sees it, judged by the parser the core uses.

On an agent at rest the cell types a short text, erases it, pastes a block of
lines, and clears it, with the keys the terminal sends. After each step it
reads the pane's visible screen with its styling through Herdr
(`pane read --source visible --format ansi`) and has `journey_composer`, which
holds the adapter's own composer declaration and the one parser, say `empty`,
`not_empty` or `not_found`. A step waits for the verdict it expects and does
not sleep for a time. Nothing is submitted, so no model turn starts and the
cell spends no request.

The screen is kept only while one verdict is made, in a private file of the
run folder; a failed cell keeps the pane capture every failed cell keeps.
"""

import base64
import json
import time

from agent_live_check.processes import ProcessError
from agent_live_check.protection import write_private

from . import cells

CELL = "composer"
TEXT = "hello 안녕하세요"
# Twelve short lines, pasted in one bracketed paste as the terminal sends a paste.
BLOCK = "\n".join(f"composer check line {number:02d}" for number in range(1, 13))
PASTE = ("\x1b[200~" + BLOCK + "\x1b[201~").encode()
BACKSPACE = b"\x7f"
# An erase sends a few Backspaces before each read, up to a cap, since an
# agent may keep pasted lines as text and not as one mark; the screen says
# when the composer is empty, not a count.
BACKSPACES_PER_READ = 8
MAX_BACKSPACES = 400
# How long a step waits for the verdict it expects.
STEP_SECONDS = 30
POLL_SECONDS = 0.5
# Each step and the verdict the operator would see after it.
STEPS = (("typed", "not_empty"), ("erased", "empty"), ("pasted", "not_empty"), ("cleared", "empty"))


def verdict_of(journey, screen):
    """The code `journey_composer` gives `screen` under this agent's declaration."""
    path = journey.run / f"{journey.id}-composer-screen.ansi"
    path.unlink(missing_ok=True)
    write_private(path, screen.encode())
    try:
        code, out, err = journey.runtime.owner.run([str(journey.composer_judge), journey.id, str(path)],
                                                   env={**journey.runtime.env}, check=False, seconds=15)
    finally:
        path.unlink(missing_ok=True)
    if code:
        raise ProcessError("journey_composer_refused: " + (err.strip() or f"exit {code}")[:300])
    try:
        return json.loads(out)["verdict"]
    except (ValueError, KeyError, TypeError) as error:
        raise ProcessError("journey_composer_answer_malformed: " + out[:200]) from error


def read(journey):
    """The verdict of the pane's visible screen now."""
    _, screen, _ = journey.runtime.command(["pane", "read", journey.pane, "--source", "visible",
                                            "--format", "ansi"])
    return verdict_of(journey, screen)


def send(journey, data):
    journey.daemon.dispatch("key", {"pane_id": journey.pane, "bytes_base64": base64.b64encode(data).decode()})


class Backspaces:
    """Sends a few Backspaces each time it is called, up to the cap."""

    def __init__(self, journey):
        self.journey, self.sent = journey, 0

    def __call__(self):
        if self.sent < MAX_BACKSPACES:
            send(self.journey, BACKSPACE * BACKSPACES_PER_READ)
            self.sent += BACKSPACES_PER_READ


def wait_for(journey, expected, before_read=None):
    """Reads until the verdict is `expected` or STEP_SECONDS pass; returns
    (last verdict, reads, seconds)."""
    start, reads = time.monotonic(), 0
    while True:
        if before_read:
            before_read()
        verdict = read(journey)
        reads += 1
        seconds = time.monotonic() - start
        if verdict == expected or seconds > STEP_SECONDS:
            return verdict, reads, round(seconds, 1)
        if journey.runtime.owner.cancelled.wait(POLL_SECONDS):
            raise ProcessError("run_cancelled")


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    record = journey.record.setdefault(CELL, {"text": TEXT, "paste_lines": len(BLOCK.splitlines()), "steps": []})
    if not journey.take_keys(record):
        journey.settle(CELL, (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                              + " -> ".join(map(str, record["terminal"])) + ")"), journey.evidence(CELL, record))
        return
    try:
        for name, expected in STEPS:
            backspaces = None
            if name == "typed":
                send(journey, TEXT.encode())
            elif name == "pasted":
                send(journey, PASTE)
            else:
                backspaces = Backspaces(journey)
            verdict, reads, seconds = wait_for(journey, expected, backspaces)
            record["steps"].append({"name": name, "expected": expected, "verdict": verdict, "reads": reads,
                                    "seconds": seconds, **({"backspaces": backspaces.sent} if backspaces else {})})
            if verdict != expected:
                break
    finally:
        # A draft left in the composer would sit in front of the next cell's prompt.
        if record["steps"] and record["steps"][-1]["verdict"] != "empty":
            try:
                wait_for(journey, "empty", Backspaces(journey))
            except ProcessError:
                if journey.runtime.owner.cancelled.is_set():
                    raise
    journey.settle(CELL, cells.composer_verdict(record["steps"]), journey.evidence(CELL, record))
