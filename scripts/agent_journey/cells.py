"""The journey table and its verdicts: pure functions over what Hide reported.

Every adapter and every cell has one row. A cell the adapter does not declare
is N/A from the declaration; a cell that could not be measured says why. No
verdict here reads a screen: each judges a spawn answer, snapshot rows, the
conversation Hide's reader returned, a letter's record or Herdr's session
report.
"""

PASS, FAIL, NOT_MEASURED, NA, EXCLUDED, PENDING = "pass", "fail", "not measured", "N/A", "excluded", "pending"

# The contract's cell names in table order (contracts/agent-journey.json).
CELLS = ("start", "start_untrusted", "reply", "integration", "guidance_memory", "letter",
         "sleep_wake", "resume", "fork", "title", "usage")
# What this cycle measures (PRD agent-journey-check D-20); every other declared
# cell reads "not built in this cycle" until a later change builds it.
BUILT_AGENTS = ("claude-code", "codex", "grok", "opencode", "pi", "omp", "antigravity")
BUILT_CELLS = ("start", "start_untrusted", "reply", "integration", "guidance_memory", "letter", "sleep_wake",
               "resume", "fork", "title", "usage")
# A cell that cannot be measured until another one passed: a fork, a title
# and usage need a conversation with an answer in it.
PREREQUISITE = {"reply": "start", "integration": "start", "guidance_memory": "start", "letter": "start",
                "sleep_wake": "start", "resume": "sleep_wake", "fork": "reply", "title": "reply", "usage": "reply"}

NOT_BUILT = "not built in this cycle"
# Agents the operator took out of the journey (PRD D-45): every cell reads
# excluded with the reason, never fail or not measured, and counts nowhere.
EXCLUDED_AGENTS = {"cursor": "operator decision 2026-10-17"}
INTERRUPTED = "interrupted"


def cell(status, reason, evidence=None):
    return {"status": status, "reason": reason, "evidence": evidence}


def plan(contract, selected):
    """The whole table before anything runs: N/A, unselected and unbuilt cells are final."""
    table = {}
    for row in contract:
        cells = {}
        for name in CELLS:
            if row["id"] in EXCLUDED_AGENTS:
                cells[name] = cell(EXCLUDED, EXCLUDED_AGENTS[row["id"]])
            elif not row["cells"][name]:
                cells[name] = cell(NA, "not declared by the adapter")
            elif row["id"] not in selected:
                cells[name] = cell(NOT_MEASURED, "not selected (--agents)")
            elif row["id"] not in BUILT_AGENTS or name not in BUILT_CELLS:
                cells[name] = cell(NOT_MEASURED, NOT_BUILT)
            else:
                cells[name] = cell(PENDING, "")
        table[row["id"]] = cells
    return table


def settle(table, agent, name, status, reason, evidence=None):
    """Records one cell's result; a settled cell is never overwritten."""
    if status not in (PASS, FAIL, NOT_MEASURED):
        raise ValueError("unknown_cell_status")
    current = table[agent][name]
    if current["status"] != PENDING:
        return False
    table[agent][name] = cell(status, reason, evidence)
    return True


def settle_agent(table, agent, reason, evidence=None):
    """Every cell of the agent still open reads `reason` as not measured."""
    for name, current in table[agent].items():
        if current["status"] == PENDING:
            table[agent][name] = cell(NOT_MEASURED, reason, evidence)


def measurable(table, agent, name):
    """Whether the cell is still open and its prerequisite passed; an open cell
    whose prerequisite did not pass is settled as not measured, naming it."""
    current = table[agent][name]
    if current["status"] != PENDING:
        return False
    before = PREREQUISITE.get(name)
    if before is None:
        return True
    status = table[agent][before]["status"]
    if status == PENDING and not measurable(table, agent, before):
        # An open prerequisite whose own prerequisite did not pass is settled first.
        status = table[agent][before]["status"]
    if status == PASS:
        return True
    if status == PENDING:
        raise ValueError("prerequisite_not_settled:" + before)
    word = "failed" if status == FAIL else "not measured"
    table[agent][name] = cell(NOT_MEASURED, f"prerequisite cell {word} ({before})")
    return False


def close(table, interrupted):
    """Settles every cell left open: an interrupted run says so, and a cell a
    finished run never reached says that, never pass."""
    for agent, cells in table.items():
        for name in CELLS:
            if cells[name]["status"] != PENDING:
                continue
            before = PREREQUISITE.get(name)
            if not interrupted and before and cells[before]["status"] not in (PASS, PENDING):
                measurable(table, agent, name)
            else:
                cells[name] = cell(NOT_MEASURED, INTERRUPTED if interrupted else "not reached")


def exit_code(table, selected, *, setup_failed):
    """0 every measured selected cell passed, 1 a cell failed, 2 isolation,
    setup or cleanup failed, 3 nothing failed but a selected cell was not
    measured. An excluded cell counts toward none of them."""
    if setup_failed:
        return 2
    rows = [current for agent in selected for current in table[agent].values()]
    if any(row["status"] == PENDING for row in rows):
        raise ValueError("table_not_closed")
    if any(row["status"] == FAIL for row in rows):
        return 1
    if any(row["status"] == NOT_MEASURED for row in rows):
        return 3
    return 0


# Verdicts. Each answers (status, reason) from facts Hide reported.

def refusal(answer):
    """The product's refusal code and message from a `hide` answer."""
    if not isinstance(answer, dict):
        return "no answer", None
    error = answer.get("error") if isinstance(answer.get("error"), dict) else {}
    return (error.get("code") or answer.get("reason") or "no answer",
            error.get("message") or answer.get("message"))


def spawn_verdict(answer):
    """What `hide agent spawn` answered: registered, running, connected, with a native session."""
    if not isinstance(answer, dict) or answer.get("ok") is not True:
        code, message = refusal(answer)
        return FAIL, f"spawn refused: {code}" + (f": {message}" if message else "")
    view = answer.get("value") or {}
    missing = [what for what, ok in (("registered", view.get("registered") is True),
                                     ("running", view.get("runtime") == "running"),
                                     ("connected", view.get("connection") == "connected"),
                                     ("native session", bool(view.get("session"))))
               if not ok]
    if missing:
        return FAIL, "spawn answered without " + ", ".join(missing)
    return PASS, "registered, running, connected, native session " + str(view["session"])


def answer_of(events, token, after=0):
    """(prompt_at, answer_at) in Unix ms for the first assistant message from
    `after` on that carries `token`, the prompt being the last message the
    person typed before it (not context Hide or the agent injected); None when Hide's reader has no such answer yet."""
    prompt_at = None
    for event in events[after:]:
        if event.get("role") == "user" and event.get("kind", "human") == "human":
            prompt_at = event.get("at")
        elif event.get("role") == "assistant" and token in (event.get("text") or ""):
            return prompt_at, event.get("at")
    return None


def turn_verdict(observed, events, token, after=0):
    """The turn ended: Hide's reader has its answer, and the row read idle
    later than the answer's session time (PRD B5). `observed` is every
    activity the row read since the turn's action, as (activity, Unix ms
    seen), so an idle left from before the prompt is never the proof.
    Whether the row read working is reported, not required: Herdr's screen
    detection misses a turn shorter than its reading."""
    answer = answer_of(events, token, after)
    if answer is None:
        return FAIL, "Hide's reader found no answer to the prompt"
    prompt_at, answer_at = answer
    if not answer_at:
        return FAIL, "the answer has no session time"
    if not any(activity == "stopped" and seen >= answer_at for activity, seen in observed):
        return FAIL, "row never read idle after the answer"
    if any(activity == "working" for activity, _ in observed):
        return PASS, "row read working, then idle after the answer"
    length = f", {(answer_at - prompt_at) / 1000:.1f} s" if prompt_at else ""
    return PASS, f"row read idle after the answer; working not seen: a turn shorter than Herdr's detection{length}"


def with_turn(verdict, turn):
    """A reply's verdict with its turn's: a failed turn fails it, and a
    passing one adds what the row read."""
    if turn is None or verdict[0] != PASS:
        return verdict
    if turn[0] != PASS:
        return turn
    return PASS, verdict[1] + "; " + turn[1]


def emitter_verdict(session, expected):
    """Herdr's own integration reported the pane's session under `expected`,
    the contract's `herdr_source`."""
    source = (session or {}).get("source")
    if source == expected:
        return PASS, "Herdr integration reported the session as " + source
    return FAIL, f"Herdr integration did not report the session (source {source}, not {expected})"


def start_verdict(*verdicts):
    """The first verdict that did not pass, or all of their reasons."""
    for status, reason in verdicts:
        if status != PASS:
            return status, reason
    return PASS, "; ".join(reason for _, reason in verdicts)


def reply_verdict(read, session, token, after=0):
    """The exact token in an assistant message Hide's reader returned for `session`.

    `read` is the reader's answer: {"session", "events"} or {"error"}. A reply
    found only by another parser, in another session or only in a human
    message is not a pass.
    """
    if "error" in read:
        return FAIL, "Hide's reader could not read the session: " + read["error"]
    if read.get("session") != session:
        return FAIL, f"Hide's reader read session {read.get('session')}, not {session}"
    events = read.get("events", [])[after:]
    if not any(event.get("role") == "assistant" for event in events):
        return FAIL, "Hide's reader found no assistant message"
    if any(event.get("role") == "assistant" and token in (event.get("text") or "") for event in events):
        return PASS, "assistant message carries " + token
    return FAIL, "no assistant message carries " + token


def bell_turn(events, bell_line):
    """The index just past the human event the doorbell typed, or None."""
    for index, event in enumerate(events):
        if event.get("role") == "user" and bell_line and bell_line in (event.get("text") or ""):
            return index + 1
    return None


def letter_verdict(shown, read, session, token):
    """The doorbell rang, the letter was delivered, and the reply to that turn carries the token."""
    if not isinstance(shown, dict):
        return FAIL, "hide request show gave no letter"
    if not shown.get("bell_line"):
        return FAIL, "the doorbell never rang (state " + str(shown.get("state")) + ")"
    if shown.get("state") != "delivered":
        return FAIL, "letter state is " + str(shown.get("state")) + ", not delivered"
    if "error" in read:
        return FAIL, "Hide's reader could not read the session: " + read["error"]
    after = bell_turn(read.get("events", []), shown["bell_line"])
    if after is None:
        return FAIL, "Hide's reader found no turn the doorbell opened"
    status, reason = reply_verdict(read, session, token, after)
    return status, ("delivered; " + reason) if status == PASS else reason


def prompt_letter_verdict(shown, read, session, token, after):
    """A letter the agent's prompt hook handed over with the next prompt
    typed after it: Hide records it delivered and confirmed by the hook, and
    the reply to that prompt, from `after` on, carries the token."""
    if not isinstance(shown, dict):
        return FAIL, "hide request show gave no letter"
    if shown.get("state") != "delivered" or shown.get("hook_confirmed") is not True:
        return FAIL, (f"letter state is {shown.get('state')} and hook confirmed {shown.get('hook_confirmed')}, "
                      "not delivered by the prompt hook")
    status, reason = reply_verdict(read, session, token, after)
    return status, ("delivered with the next prompt; " + reason) if status == PASS else reason


def sleep_verdict(asleep, awake, failure=None):
    """`asleep`: the sleep states read after Sleep (the row's, or the sleeping
    session's phases when Sleep closed the pane); `awake`: whether the agent
    ran again after Wake; `failure`: why Wake said it did not."""
    if "sleeping" not in asleep:
        return FAIL, "row never read asleep after Sleep (" + ", ".join(map(str, asleep[-3:])) + ")"
    if not awake:
        return FAIL, "row never woke after Wake" + (f": {failure}" if failure else "")
    return PASS, "row read asleep after Sleep and running after Wake"


def resume_verdict(before, after, reply):
    if not before or before != after:
        return FAIL, f"native session changed across Wake ({before} -> {after})"
    status, reason = reply
    return status, ("same session " + before + "; " + reason) if status == PASS else reason
