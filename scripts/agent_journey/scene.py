"""The `scene_<name>` cells: what a letter's bell does while a screen of the
agent's own is open (docs/VERIFICATION.md#agent-scenes).

Every agent declares fourteen scenes (`hide-agent-adapter` `scene.rs`); the
contract carries each one's door and finding. A scene the journey opens by
keys is put on the screen, a letter is sent with `--now`, and the run judges
what Hide did with it. Two kinds of run:

- `check`, the ordinary run: a scene that carries a release check (what the
  `letter_menu` cell proved) holds a letter for `RELEASE_SECONDS`, closes the
  scene, and passes only when the letter is delivered once and answered. A
  scene whose finding is `safe` is re-proven by the survey's probe. Any other
  scene reads `not measured` with the reason.
- `survey`, the measurement: every scene with a door the journey can open is
  watched for `WATCH_SECONDS`, the cell passes or fails nothing, and the run
  records what it saw (record.json and three screens per repetition) as the
  candidate finding. The declaration PR copies a survey's record into the
  finding; this module never writes one.

A door that is a start or a scripted turn is not opened by this module yet:
a start needs the agent launched into a state the journey does not prepare,
and a turn needs a model that calls a tool, which the fake model does not.
Those cells read `not measured` with that reason in both runs.
"""

import base64
import json
import time

from agent_live_check.processes import Cancelled
from agent_live_check.protection import private_directory, write_private

from . import cells, composer

CELL = "scenes"
CHECK, SURVEY = "check", "survey"
MODES = (CHECK, SURVEY)
# The cells this module owns, in the order of the contract's `Scene::ALL`.
ALL_SCENES = cells.ALL_SCENES
# The scenes that have a cell of their own, in the order of the contract's `Scene::ALL`.
SCENES = tuple(name for name in ALL_SCENES if name not in cells.PROVEN_BY_CELL)
# How long a scene is left open before the letter is sent, so the screen has drawn.
OPEN_SECONDS = 3
# How long the release check keeps the scene open after the letter is sent:
# inside the core's 30 s quiet period, so it proves the draft record only.
RELEASE_SECONDS = 10
# The survey's window: the 30 s quiet period and 45 s of doorbell passes
# after it (a pass every 250 ms, bounded backoff on a refusal), and then the
# release after the scene is closed.
WATCH_SECONDS = 75
RELEASE_WATCH_SECONDS = 60
POLL_SECONDS = 1
# Repetitions a survey makes of one scene from a clean one.
SURVEY_REPETITIONS = 3

KEYS = {"enter": b"\r", "esc": b"\x1b", "tab": b"\t", "shift_tab": b"\x1b[Z", "up": b"\x1b[A", "down": b"\x1b[B"}
# What the core's `doorbell.held` reasons mean for a scene (delivery.md): the
# guard that held a letter, and whether it is structural. A clock only delays
# the line, so a cell that only a clock held until the end of the window is
# `unknown`.
GUARDS = {"blocked": "engine_blocked", "composer_unconfirmed": "composer_not_clear",
          "awaiting_operator": "native_wait", "session_unread": "native_wait",
          "draft": "draft_record", "input_after_verdict": "draft_record",
          "not_ready": "not_ready", "launch_pending": "not_ready",
          "quiet_period": "clock", "working": "clock"}
STRUCTURAL = ("engine_blocked", "composer_not_clear", "native_wait", "draft_record", "not_ready")

TURN_REASON = ("needs a model that calls a tool ({script}), which this run's model does not: surveyed on a "
               "real login and not CI-verified (docs/VERIFICATION.md#agent-scenes)")
START_REASON = ("the journey does not yet start the agent in the state this door needs ({start}); a survey "
                "of it is a start of its own")
NOT_CHECKED = "not checked in the ordinary run: `--scenes survey` measures it (docs/VERIFICATION.md#agent-scenes)"
UNSAFE_FINDING = "its finding is unsafe: a letter is not held on it unattended, since it may answer the screen"


def cell_name(scene):
    return "scene_" + scene


def declared(row):
    """(scene, case) for each scene cell the contract declares for `row`."""
    cases = {case["scene"]: case for case in row["scenes"]}
    return [(scene, cases[scene]) for scene in SCENES if row["cells"].get(cell_name(scene))]


def plan(case, mode):
    """How a run measures one scene: ("release", None), ("watch", None), or
    (None, why not)."""
    door = case["door"]
    if door["kind"] == "turn":
        return None, TURN_REASON.format(script=door["script"])
    if door["kind"] == "start":
        return None, START_REASON.format(start=door["start"].get("kind"))
    if mode == SURVEY or case["finding"] == "safe":
        return "watch", None
    if case["release_check"]:
        return "release", None
    return None, UNSAFE_FINDING if case["finding"] == "unsafe" else NOT_CHECKED


# The most model requests one watched scene may take: the held turn of a
# scene opened from a working agent and the letter's turn, at most four each
# (the most any agent's turn takes, hide-agent-adapter `requests_per_turn`).
SURVEY_REQUESTS_PER_SCENE = 2 * 4


def survey_requests(row, repetitions):
    """The model requests a survey adds to the contract's budget: each scene
    the journey opens by keys, `repetitions` times."""
    watched = sum(1 for _, case in declared(row) if case["door"]["kind"] == "keys")
    return watched * (repetitions or SURVEY_REPETITIONS) * SURVEY_REQUESTS_PER_SCENE


def lane_status(row, name):
    """The status a lane that runs the journey in its ordinary mode requires
    of cell `name` of contract `row`: N/A for a cell the adapter does not
    declare, and for a scene cell `pass` when the ordinary run opens that
    scene (a release check or a `safe` finding) and `not measured` when it
    does not (a scripted turn, a start, or a finding no survey has closed),
    so a cell that passed without a proof fails the lane too."""
    if not row["cells"][name]:
        return cells.NA
    if name not in cells.SCENE_CELLS:
        return cells.PASS
    case = next(case for case in row["scenes"] if cell_name(case["scene"]) == name)
    return cells.PASS if plan(case, CHECK)[0] else cells.NOT_MEASURED


def guard_of(reasons):
    """(guard, structural) from the `doorbell.held` reasons a letter met, the
    first structural one in the order the core met them; a window held only
    by clocks has none."""
    for reason in reasons:
        guard = GUARDS.get(reason)
        if guard in STRUCTURAL:
            return guard
    return None


def candidate(watch):
    """(value, why) the survey's finding candidate for one repetition, from
    what the watch recorded: `attempts` bells typed while the scene was open,
    `quiet_ended` (the core's quiet period was over inside the window),
    `guard`, and the release's `released` (delivered once and answered)."""
    if watch["attempts"]:
        return "unsafe", f"the bell typed {watch['attempts']} time(s) while the scene was open"
    if not watch["quiet_ended"]:
        return "unknown", "the quiet period never ended inside the window, so only a clock held the letter"
    if watch["guard"] is None:
        return "unknown", "only a clock held the letter"
    if not watch["released"]:
        return "unsafe", "the letter was not delivered once and answered after the scene was closed"
    return "safe", f"held by {watch['guard']}, then delivered once"


def combine(values):
    """One value for the repetitions: any unsafe makes it unsafe, all safe
    makes it safe, anything else is unknown."""
    values = list(values)
    if "unsafe" in values:
        return "unsafe"
    return "safe" if values and all(value == "safe" for value in values) else "unknown"


# Probes ----------------------------------------------------------------


def screen(journey):
    return journey.controller.read_pane(journey.pane, visible=True)


def send(journey, data):
    journey.daemon.dispatch("key", {"pane_id": journey.pane, "bytes_base64": base64.b64encode(data).decode()})


def press(journey, keys):
    """Sends declared keys as the operator would: a typed word waits a
    second before the next key, so the scene's own screen has drawn."""
    for key in keys:
        if key["key"] == "text":
            send(journey, key["text"].encode())
            wait(journey, 1)
        elif key["key"] == "clear_composer":
            composer.wait_for(journey, "empty", composer.Backspaces(journey))
        else:
            send(journey, KEYS[key["key"]])


def wait(journey, seconds):
    if journey.runtime.owner.cancelled.wait(seconds):
        raise Cancelled("run_cancelled")


def hold_a_turn(journey, record):
    """A turn the fake model holds, so a scene opened from a working agent
    meets one running; False when it never read working."""
    from .journey import HELD_PROMPT, TURN_POLL_SECONDS, WORKING_SECONDS, token
    held = record["held"] = token()
    since = journey.daemon.mark(journey.pane)
    journey.type_prompt(record, HELD_PROMPT.format(held))
    end = time.monotonic() + WORKING_SECONDS
    while not any(activity == "working" for activity, _ in journey.daemon.activities(journey.pane, since)):
        if time.monotonic() > end:
            return False
        wait(journey, TURN_POLL_SECONDS)
    return True


def open_scene(journey, case, record):
    """Puts the pane on the scene through its keys; False with the reason
    settled in `record["failed"]` when it could not."""
    door = case["door"]
    if not journey.take_keys(record):
        record["failed"] = ("the pane's terminal never took this screen's keys (transport "
                            + " -> ".join(map(str, record["terminal"])) + ")")
        return False
    if door["from"] == "working" and not hold_a_turn(journey, record):
        record["failed"] = "the held turn never read working"
        return False
    press(journey, door["open"])
    wait(journey, OPEN_SECONDS)
    return True


def letter_into(journey, scene, value, record):
    return journey.send_now(f"journey-letter-scene-{scene}-{journey.id}", value, record)


def release(journey, scene, case, name):
    """The release check (what `letter_menu` was): a letter sent with
    `--now` while the scene is open is delivered after one bell and answered
    once the journey closes the scene."""
    from .journey import COMPOSER_CHECK_SECONDS, CONFIRM_SECONDS, LETTER_SECONDS, token
    record = {"scene": scene, "mode": "release", "shown": []}
    journey.record[name] = record
    # A letter held on the scene is released when Hide reads the composer
    # empty after it closes, which Hide does only for a version whose composer
    # it has checked; the cell starts from that settled pane.
    record["composer_check"] = journey.composer_checked()
    if record["composer_check"] is None:
        journey.settle(name, (cells.FAIL, "Hide had not checked this version's composer within "
                                          f"{COMPOSER_CHECK_SECONDS} s at rest, so no letter held on the "
                                          "scene could be released"), journey.evidence(name, record))
        return
    value = record["token"] = token()
    before = len(journey.conversation().get("events", []))
    since = journey.daemon.mark(journey.pane)
    if not open_scene(journey, case, record):
        journey.settle(name, (cells.FAIL, record["failed"]), journey.evidence(name, record))
        return
    record["opened"] = screen(journey)
    log = journey.log_mark()
    letter = letter_into(journey, scene, value, record)
    if letter is None:
        journey.settle(name, (cells.FAIL, "hide request send refused: "
                              + ": ".join(filter(None, cells.refusal(record["sent"])))),
                       journey.evidence(name, record))
        return
    wait(journey, RELEASE_SECONDS)
    record["held"] = screen(journey)
    press(journey, case["door"]["close"])
    turn, read, record["activities"] = journey.wait_turn(since, value, lambda _events: before,
                                                         seconds=LETTER_SECONDS)
    record["turn"] = turn
    shown = journey.shown(letter, record, lambda shown: shown.get("state") == "delivered", CONFIRM_SECONDS)
    record["read"] = {**read, "events": read.get("events", [])[before:]} if "events" in read else read
    record["rungs"] = journey.rungs_since(log)
    if shown.get("state") != "delivered":
        # A letter left pending would ring into the cells after this one.
        record["cancelled"] = journey.controller.hide(["request", "cancel", letter])
    verdict = cells.with_turn(cells.menu_letter_verdict(shown, read, journey.session, value, before), turn)
    journey.settle(name, journey.with_rung(verdict, record["rungs"]), journey.evidence(name, record))


def record_screens(directory, **screens):
    for name, text in screens.items():
        write_private(directory / f"{name}.ansi", text.encode())


def watch_once(journey, scene, case, directory, repetition):
    """One repetition of the survey's probe (docs/VERIFICATION.md#agent-scenes):
    the scene opened, a letter sent with `--now`, `WATCH_SECONDS` of reads
    once a second, the scene closed, and `RELEASE_WATCH_SECONDS` for the
    letter to arrive. Returns the repetition's record."""
    from .journey import CONFIRM_SECONDS, LETTER_SECONDS, token
    record = {"agent": journey.id, "scene": scene, "repetition": repetition, "version": journey.record.get("version"),
              "door": case["door"], "shown": [], "samples": []}
    private_directory(directory)
    record["composer_check"] = journey.composer_checked()
    value = record["token"] = token()
    before = len(journey.conversation().get("events", []))
    since = journey.daemon.mark(journey.pane)
    if not open_scene(journey, case, record):
        return {**record, "candidate": "unknown", "why": record["failed"]}
    opened = screen(journey)
    log = journey.log_mark()
    letter = letter_into(journey, scene, value, record)
    if letter is None:
        return {**record, "candidate": "unknown",
                "why": "hide request send refused: " + ": ".join(filter(None, cells.refusal(record["sent"])))}
    start = time.monotonic()
    shown = {}
    middle = None
    try:
        while time.monotonic() - start < WATCH_SECONDS:
            shown = (journey.controller.hide(["request", "show", letter]).get("result") or {})
            record["samples"].append({
                "t": round(time.monotonic() - start, 1), "state": shown.get("state"),
                "bell_attempts": shown.get("bell_attempts"),
                "activity": ([activity for activity, _ in journey.daemon.activities(journey.pane, since)] or [None])[-1]})
            if middle is None and time.monotonic() - start >= WATCH_SECONDS / 2:
                middle = screen(journey)
            wait(journey, POLL_SECONDS)
        during = screen(journey)
        records = journey.core_lines(log)
        reasons = [line.get("reason") for line in records if line.get("kind") == "doorbell.held"]
        record["hold_reasons"] = reasons
        record["core_log"] = [line.get("kind") for line in records
                              if line.get("kind") in ("doorbell.held", "composer.read", "composer.not_read",
                                                      "input.rung")]
        attempts = shown.get("bell_attempts") or 0
        # The quiet period ended when a pass met another reason than it, or
        # a bell was typed, or the letter went in.
        after_quiet = [reason for reason in reasons if reason != "quiet_period"]
        quiet_ended = bool(after_quiet) or attempts > 0 or shown.get("state") == "delivered"
        guard = guard_of(reasons)
        record.update(attempts=attempts, quiet_ended=quiet_ended, guard=guard,
                      engine=[activity for activity, _ in journey.daemon.activities(journey.pane, since)])
        press(journey, case["door"]["close"])
        after = screen(journey)
        shown_after = journey.shown(letter, record, lambda value: value.get("state") == "delivered",
                                    RELEASE_WATCH_SECONDS)
        turn, read, _ = journey.wait_turn(since, value, lambda _events: before, seconds=LETTER_SECONDS)
        verdict = cells.menu_letter_verdict(shown_after, read, journey.session, value, before)
        record["released"] = verdict[0] == cells.PASS and shown_after.get("bell_attempts") == attempts + 1
        record["release_verdict"] = list(verdict)
        value_, why = candidate(record)
        record.update(candidate=value_, why=why)
        record_screens(directory, before=opened, during=during, after=after, **({"middle": middle} if middle else {}))
    finally:
        if record.get("candidate") is None:
            record.update(candidate="unknown", why="the probe stopped before the scene was closed")
        # A letter still pending would ring into the next scene.
        if shown.get("state") != "delivered":
            record["cancelled"] = journey.controller.hide(["request", "cancel", letter])
        write_private(directory / "record.json",
                      (json.dumps(record, indent=2, ensure_ascii=False, default=str) + "\n").encode())
    return record


def watch(journey, scene, case, name, mode):
    record = {"scene": scene, "mode": mode, "repetitions": []}
    journey.record[name] = record
    count = journey.scene_repetitions or (SURVEY_REPETITIONS if mode == SURVEY else 1)
    for repetition in range(1, count + 1):
        directory = journey.run / "scenes" / journey.id / scene / str(repetition)
        found = watch_once(journey, scene, case, directory, repetition)
        record["repetitions"].append({key: found.get(key) for key in (
            "repetition", "candidate", "why", "guard", "attempts", "quiet_ended", "released", "engine")})
    values = [found["candidate"] for found in record["repetitions"]]
    record["candidate"] = combine(values)
    reasons = "; ".join(found["why"] or "" for found in record["repetitions"][:1])
    text = (f"survey: candidate {record['candidate']} ({reasons}); the declared finding is {case['finding']}")
    evidence = journey.evidence(name, record)
    if mode == SURVEY:
        # A survey records and judges nothing: the declaration PR copies it.
        cells.settle(journey.table, journey.id, name, cells.NOT_MEASURED, text, evidence)
    elif record["candidate"] == "safe":
        journey.settle(name, (cells.PASS, "re-proven safe: " + reasons), evidence)
    else:
        journey.settle(name, (cells.FAIL, f"the finding is safe but this run saw {record['candidate']}: {reasons}"),
                       evidence)


def measure(journey):
    mode = journey.scene_mode
    for scene, case in declared(journey.row):
        name = cell_name(scene)
        if not cells.measurable(journey.table, journey.id, name):
            continue
        how, why = plan(case, mode)
        if how is None:
            cells.settle(journey.table, journey.id, name, cells.NOT_MEASURED, why)
        elif how == "release":
            release(journey, scene, case, name)
        else:
            watch(journey, scene, case, name, mode)
