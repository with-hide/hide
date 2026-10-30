"""The composer cell: the agent's composer reads empty or holding text the way
the operator sees it, judged by the parser the core uses.

On an agent at rest the cell types a short text, erases it, pastes a block of
lines, and clears it, with the keys Hide's terminal sends; an agent that on
Windows can only guess a paste from key timing gets the lines typed with its
newline key instead (`TIMED_PASTE`). After each step it
reads the pane's visible screen with its styling through Herdr
(`pane read --source visible --format ansi`) and has `journey_composer`, which
holds the adapter's own composer declaration and the one parser, say `empty`,
`not_empty` or `not_found`. A step waits for the verdict it expects and does
not sleep for a time. Nothing is submitted, so no model turn starts and the
cell spends no request; after each step Hide's reader counts the session's
prompts, and a step after which the session holds a new one fails the cell
there, before the turns it started spend a later cell's requests.

The screen is kept only while one verdict is made, in a private file of the
run folder; a failed cell keeps the pane capture every failed cell keeps.
"""

import base64
import json
import time

from agent_live_check.processes import ProcessError
from agent_live_check.protection import write_private
from agent_live_check.system import WINDOWS

from . import cells

CELL = "composer"
TEXT = "hello 안녕하세요"
# Twelve short lines, pasted as Hide's terminal pastes them: xterm.js turns
# each line break into CR and brackets the text when the program asked for
# it (`prepareTextForTerminal`, `bracketTextForPaste`). An LF-separated paste
# reached Grok 1.0.52 on Windows as eleven prompts (run 39635682277).
BLOCK = "\r".join(f"composer check line {number:02d}" for number in range(1, 13))
PASTE = ("\x1b[200~" + BLOCK + "\x1b[201~").encode()
# The agents that on Windows read console key records, are never told of a
# paste, and join its keys into one only when each arrives soon after the
# last: Grok's window is 10 ms (grok-build `coalesce_live_keys`,
# `PASTE_CONTINUE_TIMEOUT`), and a pause under load made a line break of the
# paste an Enter that sent lines 01 to 04 (with-hide/hide-private#361). No
# form of a paste is safe there (docs/INSTALL.md, What each system supports),
# so the same twelve lines go in with Alt+Enter, Grok's own newline key,
# which never submits.
TIMED_PASTE = {"grok"}
DRAFT = "\x1b\r".join(BLOCK.split("\r")).encode()
BACKSPACE = b"\x7f"
# An erase sends a few Backspaces before each read, up to a cap, since an
# agent may keep pasted lines as text and not as one mark; the screen says
# when the composer is empty, not a count.
BACKSPACES_PER_READ = 8
MAX_BACKSPACES = 400
# How long a step waits for the verdict it expects after the last key it
# sent: an erase is still typing while it sends, and a pane read slower than
# another system's sends fewer Backspaces in the same time (macOS, 2026-10-22:
# 248 in 30 s, short of the 275 characters of a paste Codex keeps as text).
STEP_SECONDS = 30
POLL_SECONDS = 0.5
# Each step and the verdict the operator would see after it.
STEPS = (("typed", "not_empty"), ("erased", "empty"), ("pasted", "not_empty"), ("cleared", "empty"))


def block(journey):
    """The step that puts the twelve lines in the composer and what it sends."""
    if WINDOWS and journey.id in TIMED_PASTE:
        return "drafted", DRAFT
    return "pasted", PASTE


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
    """Sends a few Backspaces each time it is called, up to the cap, and
    says whether it sent any."""

    def __init__(self, journey):
        self.journey, self.sent = journey, 0

    def __call__(self):
        if self.sent >= MAX_BACKSPACES:
            return False
        send(self.journey, BACKSPACE * BACKSPACES_PER_READ)
        self.sent += BACKSPACES_PER_READ
        return True


def wait_for(journey, expected, before_read=None):
    """Reads until the verdict is `expected` or STEP_SECONDS pass after the
    last key `before_read` sent; returns (last verdict, reads, seconds)."""
    start = last_key = time.monotonic()
    reads = 0
    while True:
        if before_read and before_read():
            last_key = time.monotonic()
        verdict = read(journey)
        reads += 1
        now = time.monotonic()
        if verdict == expected or now - last_key > STEP_SECONDS:
            return verdict, reads, round(now - start, 1)
        if journey.runtime.owner.cancelled.wait(POLL_SECONDS):
            raise ProcessError("run_cancelled")


def prompts(journey):
    """How many prompts of the operator the session holds, by Hide's reader."""
    read = journey.conversation()
    if "error" in read:
        raise ProcessError("composer_session_unreadable: " + read["error"][:200])
    return sum(1 for event in read["events"] if event.get("role") == "user")


def measure(journey):
    if not cells.measurable(journey.table, journey.id, CELL):
        return
    record = journey.record.setdefault(CELL, {"text": TEXT, "paste_lines": len(BLOCK.splitlines()), "steps": []})
    if not journey.take_keys(record):
        journey.settle(CELL, (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                              + " -> ".join(map(str, record["terminal"])) + ")"), journey.evidence(CELL, record))
        return
    try:
        before = prompts(journey)
        for name, expected in STEPS:
            backspaces = None
            if name == "typed":
                send(journey, TEXT.encode())
            elif name == "pasted":
                name, data = block(journey)
                send(journey, data)
            else:
                backspaces = Backspaces(journey)
            verdict, reads, seconds = wait_for(journey, expected, backspaces)
            record["steps"].append({"name": name, "expected": expected, "verdict": verdict, "reads": reads,
                                    "seconds": seconds, "prompts": prompts(journey) - before,
                                    **({"backspaces": backspaces.sent} if backspaces else {})})
            if verdict != expected or record["steps"][-1]["prompts"]:
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
