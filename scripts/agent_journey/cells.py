"""The journey table and its verdicts: pure functions over what Hide reported.

Every adapter and every cell has one row. A cell the adapter does not declare
is N/A from the declaration; a cell that could not be measured says why. No
verdict here reads a screen: each judges a spawn answer, snapshot rows, the
conversation Hide's reader returned, a letter's record or Herdr's session
report, or, for the composer, the verdicts the product's own composer parser
gave the screens (`journey_composer`).
"""

import hashlib

PASS, FAIL, NOT_MEASURED, NA, EXCLUDED, PENDING = "pass", "fail", "not measured", "N/A", "excluded", "pending"
# A failed turn the agent never answered at all, after its prompt: the one
# failure a provider's refusal of that turn can explain (provider_limit.py).
NO_ANSWER = "no_answer"


class Verdict(tuple):
    """A (status, reason) verdict that also says what kind of failure it
    is; it unpacks and settles as the pair."""

    def __new__(cls, status, reason, kind=None):
        verdict = super().__new__(cls, (status, reason))
        verdict.kind = kind
        return verdict


def kind(verdict):
    """The kind of a failed verdict, or None for a plain pair."""
    return getattr(verdict, "kind", None)


def assistant_after(events, after=0):
    """Whether the agent said anything from `after` on."""
    return any(event.get("role") == "assistant" for event in events[after:])

# The contract's cell names in table order (contracts/agent-journey.json).
CELLS = ("start", "start_untrusted", "launch_brief", "reply", "integration", "guidance_memory", "letter",
         "letter_busy", "letter_menu", "composer", "sleep_wake", "resume", "fork", "title", "usage")
# A cell that cannot be measured until another one passed: a fork, a title
# and usage need a conversation with an answer in it.
PREREQUISITE = {"reply": "start", "integration": "start", "guidance_memory": "start", "letter": "start",
                "letter_busy": "letter", "letter_menu": "letter", "composer": "start", "sleep_wake": "start", "resume": "sleep_wake", "fork": "reply", "title": "reply", "usage": "reply"}

# Agents the operator took out of the journey (PRD D-45): every cell reads
# excluded with the reason, never fail or not measured, and counts nowhere.
EXCLUDED_AGENTS = {"cursor": "operator decision 2026-10-17"}
INTERRUPTED = "interrupted"


def excluded_agents(engine="herdr", fake_model=False):
    """Explicit transition gaps, shared by the run plan and CI verdict.

    The own-start-fixes work removes Pi's whole row exclusion once the
    product confirms its first brief; later cells cannot run without it.
    """
    return {**EXCLUDED_AGENTS, **({"pi": "own-start-fixes 노드 대기(Pi 첫 지시문 input_unconfirmed, 제품 결함)"}
                                if engine == "own" and fake_model else {})}


def cell(status, reason, evidence=None):
    return {"status": status, "reason": reason, "evidence": evidence}


def plan(contract, selected, *, engine="herdr", fake_model=False):
    """Every registered adapter participates; undeclared and unselected cells are final."""
    table = {}
    excluded = excluded_agents(engine, fake_model)
    for row in contract:
        cells = {}
        for name in CELLS:
            if row["id"] in excluded:
                cells[name] = cell(EXCLUDED, excluded[row["id"]])
            elif not row["cells"][name]:
                cells[name] = cell(NA, "not declared by the adapter")
            elif row["id"] not in selected:
                cells[name] = cell(NOT_MEASURED, "not selected (--agents)")
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


def brief_verdict(spawn, launched=False):
    """The spawn's brief, confirmed by the agent's own record (its prompt hook
    or the person's message in its session file) at the first typing
    (docs/delivery.md, Input an agent did not take). `spawn` is the spawn's
    record in the candidate's delivery ledger, or None."""
    if not isinstance(spawn, dict):
        return FAIL, "the candidate's ledger holds no record of the spawn"
    typed = spawn.get("brief_input") or {}
    attempts, at, confirmed = typed.get("attempts"), typed.get("typed_at_unix_ms"), typed.get("confirmed_at_unix_ms")
    if confirmed is None:
        return FAIL, f"the agent never recorded its brief (typed {attempts or 0} time(s))"
    if attempts != 1:
        return FAIL, f"the brief was recorded only after {attempts} typings"
    # An agent whose ladder starts with the launch prompt is handed its
    # brief by its start, never typed (docs/delivery.md, The input ladder).
    if launched != (typed.get("route") == "launch"):
        return FAIL, ("the brief was typed, though the agent declares the launch prompt" if launched
                      else "the brief rode the start, though the agent declares no launch prompt")
    how = "its start's prompt" if launched else "its one typing"
    return PASS, f"brief recorded by the agent {max(0, confirmed - at) / 1000:.1f} s after {how}"


def as_started(text):
    """`text` as an agent's record of a start's prompt argument may give it:
    each line break, which the start line carries as U+2028 and an agent
    records as given, as LF or as CRLF, read as LF, and each tab as the four
    spaces the start line gives it (`worktree_control::prompt_argument`).
    Nothing else changes, so a record that lost, added or moved a character
    differs."""
    return (text.replace("\r\n", "\n").replace("\u2028", "\n").replace("\t", "    ")).strip()


def record_verdict(read, brief, token):
    """The agent's own record of a brief it was started with: exactly one
    message of the person carries the brief's token, and it is the brief
    itself, line for line (`as_started`), so a second submit, a brief the
    CLI cut or took apart as options or files, or one it wrapped, fails."""
    if "error" in read:
        return FAIL, "Hide's reader could not read the session: " + read["error"]
    messages = [event.get("text") or "" for event in read.get("events", [])
                if event.get("role") == "user" and event.get("kind", "human") == "human"
                and token in (event.get("text") or "")]
    if len(messages) != 1:
        return FAIL, f"the agent recorded the brief {len(messages)} times, not once"
    recorded, sent = as_started(messages[0]), as_started(brief)
    if recorded != sent:
        at = next((i for i, (a, b) in enumerate(zip(recorded, sent)) if a != b), min(len(recorded), len(sent)))
        return FAIL, (f"the agent's record of the brief differs from it at character {at} of {len(sent)} "
                      f"(recorded {recorded[max(0, at - 20):at + 20]!r}, sent {sent[max(0, at - 20):at + 20]!r})")
    return PASS, f"the agent recorded the brief once and exactly, {len(brief.encode())} bytes"


# The core's log lines for a rung of the input ladder (herdr-core/src/delivery/ladder.rs).
RUNG_KINDS = ("input.rung", "input.rung_unavailable", "input.rung_lost")


def rung_verdict(declared, records):
    """A letter's bell took the first rung after the launch prompt the
    agent declares (`declared`, the contract's `input`), as the core's log
    lines for the pane (`records`) say."""
    ladder = [rung for rung in declared if rung != "launch_prompt"]
    if not ladder:
        return FAIL, "the agent declares no input rung"
    rung = ladder[0]
    used = [record.get("rung") for record in records if record.get("kind") == "input.rung"]
    if rung in used:
        return PASS, "carried by " + rung
    reasons = "; ".join(str(record.get("reason")) for record in records
                        if record.get("kind") in ("input.rung_unavailable", "input.rung_lost"))
    return FAIL, f"not carried by {rung}, its declared rung" + (f" ({reasons})" if reasons else
                                                                  f" (the log names {used or 'none'})")


def busy_letter_verdict(shown, read, session, held, token, *, declared, input_rungs,
                        rungs, responses=None, sent_at=None, stops=None, observed_at=None):
    """One declared delivery path and one held -> letter answer sequence.

    Stop can continue the same assistant event. Such an event alone cannot
    prove two responses: require Stop payloads or a fake provider trace.
    docs/delivery.md, Letters while the agent works, owns these expectations.
    """
    if not isinstance(shown, dict):
        return FAIL, "hide request show gave no letter"
    if shown.get("state") != "delivered" or shown.get("hook_confirmed") is not True:
        return FAIL, "the letter has no confirmed delivery"
    bells, handovers = shown.get("bell_attempts"), shown.get("tool_handovers")
    if (bells, handovers) == (1, 0):
        route = "bell"
    elif (bells, handovers) == (1, 1):
        route = "bell_hook"
    elif (bells, handovers) == (0, 1):
        route = "turn_end"
    else:
        return FAIL, "the letter did not take exactly one delivery path"
    if route not in declared:
        return FAIL, "the observed delivery path is not declared: " + route
    if route in ("bell", "bell_hook"):
        rung = rung_verdict(input_rungs, rungs)
        if rung[0] != PASS:
            return rung
        if sum(record.get("kind") == "input.rung" for record in rungs) != 1:
            return FAIL, "the bell did not take exactly one input rung"
    elif rungs or shown.get("bell_line") or shown.get("bell_turn"):
        return FAIL, "turn-end delivery also carried bell input evidence"
    verdict = reply_verdict(read, session, token)
    if verdict[0] != PASS:
        return verdict
    events = read.get("events", [])
    positions = []
    for value in (held, token):
        matches = [(index, (event.get("text") or "").find(value))
                   for index, event in enumerate(events) if event.get("role") == "assistant"
                   for _ in range((event.get("text") or "").count(value))]
        if len(matches) != 1:
            return FAIL, "the held and letter answers must each occur exactly once"
        positions.append(matches[0])
    if positions[0] >= positions[1]:
        return FAIL, "the letter answer did not follow the held answer"
    if route == "bell_hook":
        claim = shown.get("bell_turn")
        between = events[positions[0][0] + 1:positions[1][0]]
        line = canonical_prompt(shown.get("bell_line") or "")
        bells_in_turn = sum(event.get("role") == "user" and line
                            == canonical_prompt(event.get("text") or "") for event in between)
        if (not isinstance(claim, dict)
                or claim.get("session") != hashlib.sha256(session.encode()).hexdigest()
                or claim.get("digest") != hashlib.sha256(canonical_prompt(shown.get("bell_line") or "").encode()).hexdigest()
                or not line or bells_in_turn != 1):
            return FAIL, "hook intake has no bell-owned turn between the held and letter answers"
    if stops is not None and any(row.get("error") for row in stops):
        return FAIL, "the Stop observer recorded an invalid invocation"
    merged = positions[0][0] == positions[1][0]
    # The ledger counter is shared by tool and Stop intake. Only these
    # independent payloads (or the no-tool fake) can distinguish them.
    if stops is not None and route == "turn_end":
        if (len(stops) != 2 or any(row.get("error") or row.get("session") != session
                                  or row.get("reason") != "end_turn" or row.get("subagent")
                                  or type(row.get("active")) is not bool
                                  for row in stops)
                or [row.get("active") for row in stops] != [False, True]
                or stops[0].get("answers") != [held]
                or stops[1].get("answers") not in ([token], [held, token])):
            return FAIL, "Stop evidence did not show one held completion then one continuation in this session"
        times = [sent_at, *(row.get("at") for row in stops), observed_at]
        if any(type(at) is not int or at <= 0 for at in times) or times != sorted(times):
            return FAIL, "Stop observations are not ordered after send"
    if responses is not None:
        answers = [row for row in responses if row.get("generation") is True
                   and row.get("schema") is False and row.get("answered") in (held, token)]
        if [row.get("answered") for row in answers] != [held, token]:
            return FAIL, "the fake trace did not answer held then letter exactly once"
        times = [sent_at, *(row.get("at") for row in answers), observed_at]
        if any(type(at) is not int or at <= 0 for at in times) or not times[0] <= times[1] <= times[2] <= times[3]:
            return FAIL, "the fake responses are not ordered within the observation window"
        if stops is not None and route == "turn_end":
            if not answers[0]["at"] <= stops[0]["at"] <= answers[1]["at"] <= stops[1]["at"]:
                return FAIL, "fake responses and Stop observations do not alternate in causal order"
    if (merged or route == "turn_end") and responses is None and (stops is None or route != "turn_end"):
        return FAIL, "merged or turn-end answers have no independent response order evidence"
    return PASS, "confirmed once through " + route + "; held then letter answered once in order"


def composer_check(records, agent):
    """The core's last passed composer check of `agent` among its log
    `records` (`composer.version_checked` read empty), or None."""
    passed = [record for record in records if record.get("kind") == "composer.version_checked"
              and record.get("agent") == agent and record.get("verdict") == "empty"]
    return passed[-1] if passed else None


def menu_letter_verdict(shown, read, session, token, after):
    """A letter sent while a picker of the agent's own was open is delivered
    after one bell and answered from `after` on: a bell line the picker took
    as its choice starts no turn, so it would ring again or never answer."""
    if not isinstance(shown, dict):
        return FAIL, "hide request show gave no letter"
    if shown.get("state") != "delivered":
        return FAIL, "letter state is " + str(shown.get("state")) + ", not delivered"
    if shown.get("bell_attempts") != 1:
        return FAIL, f"the letter took {shown.get('bell_attempts')} bells, not one"
    verdict = reply_verdict(read, session, token, after)
    return (PASS, "delivered past the picker; " + verdict[1]) if verdict[0] == PASS else verdict


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
        return Verdict(FAIL, "Hide's reader found no answer to the prompt",
                       None if assistant_after(events, after) else NO_ANSWER)
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
    if expected is None:
        if ((session or {}).get("observation") == "hide.agent.list" and session.get("kind") in ("id", "path")
                and isinstance(session.get("value"), str) and session["value"]):
            return PASS, "Hide's registration ledger names the native session checked by the conversation reader"
        return FAIL, "Hide's registration ledger did not name a native session"
    source = (session or {}).get("source")
    if source == expected:
        return PASS, "Herdr integration reported the session as " + source
    return FAIL, f"Herdr integration did not report the session (source {source}, not {expected})"


def start_verdict(*verdicts):
    """The first verdict that did not pass, a turn never answered last, or
    all of their reasons. A provider's limit explains only that turn, so a
    check that failed on its own beside it is the start's failure."""
    failed = [verdict for verdict in verdicts if verdict[0] != PASS]
    if failed:
        return next((verdict for verdict in failed if kind(verdict) != NO_ANSWER), failed[0])
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
    if not assistant_after(events):
        return Verdict(FAIL, "Hide's reader found no assistant message", NO_ANSWER)
    if any(event.get("role") == "assistant" and token in (event.get("text") or "") for event in events):
        return PASS, "assistant message carries " + token
    return FAIL, "no assistant message carries " + token


def canonical_prompt(text):
    # hide_agent_hooks::delivery::canonical_prompt: line breaks (CR, LF,
    # U+2028, U+2029) and characters above U+FFFF removed, a tab read as
    # four spaces. Trim Unicode White_Space. Python's default strip also
    # removes the four C0 separators U+001C..U+001F, which remain part of
    # the prompt.
    return "".join("    " if char == "\t" else char for char in text
                   if ord(char) <= 0xffff and char not in "\r\n\u2028\u2029").strip(
        "\t\n\v\f\r \u0085\u00a0\u1680"
        "\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a"
        "\u2028\u2029\u202f\u205f\u3000")


def bell_turn(events, bell_line):
    """The index just past the human event the doorbell typed, or None."""
    # The delivery contract ignores line breaks and characters above U+FFFF,
    # which a console reader may drop. Compare the whole prompt; a prefix
    # proves no bell turn.
    line = canonical_prompt(bell_line or "")
    for index, event in enumerate(events):
        if event.get("role") == "user" and line and line == canonical_prompt(event.get("text") or ""):
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
    verdict = reply_verdict(read, session, token, after)
    return (PASS, "delivered; " + verdict[1]) if verdict[0] == PASS else verdict


def prompt_letter_verdict(shown, read, session, token, after):
    """A letter the agent's prompt hook handed over with the next prompt
    typed after it: Hide records it delivered and confirmed by the hook, and
    the reply to that prompt, from `after` on, carries the token."""
    if not isinstance(shown, dict):
        return FAIL, "hide request show gave no letter"
    if shown.get("state") != "delivered" or shown.get("hook_confirmed") is not True:
        return FAIL, (f"letter state is {shown.get('state')} and hook confirmed {shown.get('hook_confirmed')}, "
                      "not delivered by the prompt hook")
    verdict = reply_verdict(read, session, token, after)
    return (PASS, "delivered with the next prompt; " + verdict[1]) if verdict[0] == PASS else verdict


def composer_verdict(steps):
    """The composer cell: every step of the typed, erased, pasted and cleared
    draft read the verdict the operator would see. `steps` is one dict per
    step, in order, with its name, the verdict it expected and the last
    verdict read (the example's code, or None when nothing was read), and
    how many prompts the session gained since the cell began."""
    for step in steps:
        if step.get("prompts"):
            return FAIL, (f"after {step['name']} the session holds {step['prompts']} new prompt(s): "
                          "the keys started a turn")
        if step["verdict"] != step["expected"]:
            return FAIL, (f"after {step['name']} the composer read {step['verdict']}, "
                          f"not {step['expected']} (last of {step['reads']} reads in {step['seconds']:.1f} s)")
    return PASS, "the composer read " + ", ".join(f"{step['verdict']} {step['name']}" for step in steps)


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
    return (PASS, "same session " + before + "; " + reply[1]) if reply[0] == PASS else reply
