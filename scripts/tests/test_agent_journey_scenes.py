"""The scene cells: how a run plans each scene from the contract, what a
survey's watch makes of what it saw, and that the ordinary lane requires only
what it proves. No agent runs (scene.py, docs/VERIFICATION.md#agent-scenes)."""

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import cells, scene  # noqa: E402
from agent_journey.contract import load_contract  # noqa: E402
from agent_live_check.protection import ProtectionError  # noqa: E402

ROWS = {row["id"]: row for row in load_contract(ROOT)}


def case(row, name):
    return next(found for found in ROWS[row]["scenes"] if found["scene"] == name)


class Contract(unittest.TestCase):
    def test_every_agent_lists_all_fourteen_scenes_with_a_door_or_an_absence(self):
        for key, row in ROWS.items():
            self.assertEqual([found["scene"] for found in row["scenes"]], list(scene.ALL_SCENES), key)
            for found in row["scenes"]:
                self.assertEqual(found["door"] is None, found["finding"] == "none", (key, found["scene"]))

    def test_a_contract_row_that_lists_fewer_scenes_is_refused(self):
        rows = copy.deepcopy(list(ROWS.values()))
        del rows[0]["scenes"][-1]
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "contracts").mkdir()
            (Path(folder) / "contracts/agent-journey.json").write_text(json.dumps(rows))
            with self.assertRaises(ProtectionError):
                load_contract(Path(folder))

    def test_a_scene_cell_is_declared_exactly_where_the_journey_opens_the_scene_itself(self):
        for key, row in ROWS.items():
            for name in scene.SCENES:
                found = case(key, name)
                opens = row["cells"]["start"] and found["door"] is not None and found["door"]["kind"] != "cell"
                self.assertEqual(row["cells"][scene.cell_name(name)], opens, (key, name))

    def test_the_cell_table_has_no_agent_specific_menu_cell_left(self):
        self.assertNotIn("letter_menu", cells.CELLS)
        self.assertEqual(set(cells.SCENE_CELLS), {scene.cell_name(name) for name in scene.SCENES})
        for name in cells.SCENE_CELLS:
            self.assertEqual(cells.PREREQUISITE[name], "letter")


class Planning(unittest.TestCase):
    def test_the_ordinary_run_checks_the_release_of_the_scenes_that_carry_one_and_nothing_else(self):
        checked = [(key, name) for key in ROWS for name, found in ((n, case(key, n)) for n in scene.SCENES)
                   if found["door"] and scene.plan(found, scene.CHECK)[0] == "release"]
        self.assertEqual(checked, [("codex", "model_picker")],
                         "the measured Codex picker is the one scene the ordinary run keeps checking")

    def test_a_scene_opened_by_keys_is_watched_by_a_survey_whatever_its_finding(self):
        for key in ROWS:
            for name in scene.SCENES:
                found = case(key, name)
                if not found["door"]:
                    continue
                how, why = scene.plan(found, scene.SURVEY)
                if found["door"]["kind"] == "keys":
                    self.assertEqual((how, why), ("watch", None), (key, name))
                else:
                    self.assertIsNone(how)
                    self.assertTrue(why, (key, name))

    def test_a_scripted_turn_and_a_start_say_what_they_need(self):
        self.assertIn("calls a tool", scene.plan(case("claude-code", "tool_approval"), scene.SURVEY)[1])
        self.assertIn("untrusted_folder", scene.plan(case("claude-code", "trust_dialog"), scene.CHECK)[1])

    def test_a_finding_that_is_unsafe_is_never_run_unattended(self):
        found = copy.deepcopy(case("claude-code", "popup"))
        found["finding"] = "unsafe"
        self.assertEqual(scene.plan(found, scene.CHECK)[0], None)
        self.assertIn("unsafe", scene.plan(found, scene.CHECK)[1])
        self.assertEqual(scene.plan(found, scene.SURVEY)[0], "watch")

    def test_a_safe_finding_is_proven_again_by_a_survey_and_not_by_the_ordinary_run(self):
        found = copy.deepcopy(case("claude-code", "popup"))
        found["finding"] = "safe"
        self.assertEqual(scene.plan(found, scene.CHECK)[0], None)
        self.assertEqual(scene.plan(found, scene.SURVEY)[0], "watch")

    def test_the_lane_requires_a_pass_exactly_where_the_ordinary_run_opens_the_scene(self):
        required = [(key, name) for key, row in ROWS.items() for name in scene.SCENES
                    if scene.lane_status(row, scene.cell_name(name)) == cells.PASS]
        self.assertEqual(required, [("codex", "model_picker")])
        # A safe finding opens nothing in the ordinary run, so the lane asks no pass of it.
        declared_safe = [(key, name) for key, row in ROWS.items() for name in scene.SCENES
                         if row["cells"][scene.cell_name(name)] and case(key, name)["finding"] == "safe"]
        self.assertTrue(declared_safe, "the declarations carry safe findings")
        for key, name in declared_safe:
            self.assertEqual(scene.lane_status(ROWS[key], scene.cell_name(name)),
                             cells.PASS if (key, name) == ("codex", "model_picker") else cells.NOT_MEASURED)

    def test_a_turn_scene_is_never_required_to_pass_in_ci(self):
        found = copy.deepcopy(ROWS["claude-code"])
        for case_ in found["scenes"]:
            if case_["scene"] == "tool_approval":
                case_["finding"] = "safe"
        self.assertEqual(scene.lane_status(found, "scene_tool_approval"), cells.NOT_MEASURED)


class Survey(unittest.TestCase):
    def watched(self, **override):
        return {"attempts": 0, "quiet_ended": True, "guard": "draft_record", "released": True, **override}

    def test_a_held_letter_released_once_is_a_safe_candidate(self):
        self.assertEqual(scene.candidate(self.watched())[0], "safe")

    def test_a_bell_typed_into_an_open_scene_is_unsafe(self):
        value, why = scene.candidate(self.watched(attempts=1, guard=None))
        self.assertEqual(value, "unsafe")
        self.assertIn("1 time", why)

    def test_a_window_the_quiet_period_filled_is_unknown(self):
        self.assertEqual(scene.candidate(self.watched(quiet_ended=False))[0], "unknown")

    def test_only_a_clock_holding_the_letter_is_unknown(self):
        for reasons in (["quiet_period"], ["quiet_period", "working"], []):
            self.assertIsNone(scene.guard_of(reasons))
        value, why = scene.candidate(self.watched(guard=scene.guard_of(["quiet_period", "working"])))
        self.assertEqual((value, why), ("unknown", "only a clock held the letter"))

    def test_a_structural_guard_is_named_from_the_core_s_hold_reasons(self):
        for reason, guard in (("blocked", "engine_blocked"), ("composer_unconfirmed", "composer_not_clear"),
                              ("awaiting_operator", "native_wait"), ("session_unread", "native_wait"),
                              ("draft", "draft_record"), ("not_ready", "not_ready")):
            self.assertEqual(scene.guard_of(["quiet_period", reason]), guard)

    def test_a_letter_lost_or_doubled_after_the_scene_closed_is_unsafe(self):
        self.assertEqual(scene.candidate(self.watched(released=False))[0], "unsafe")

    def test_repetitions_agree_only_when_every_one_is_safe(self):
        self.assertEqual(scene.combine(["safe", "safe", "safe"]), "safe")
        self.assertEqual(scene.combine(["safe", "unknown", "safe"]), "unknown")
        self.assertEqual(scene.combine(["safe", "unsafe", "unknown"]), "unsafe")
        self.assertEqual(scene.combine([]), "unknown")

    def test_a_survey_adds_the_requests_of_the_scenes_it_watches_to_the_budget(self):
        keys = sum(1 for name in scene.SCENES if (case("codex", name)["door"] or {}).get("kind") == "keys"
                   and ROWS["codex"]["cells"][scene.cell_name(name)])
        self.assertEqual(scene.survey_requests(ROWS["codex"], 3), keys * 3 * scene.SURVEY_REQUESTS_PER_SCENE)
        self.assertEqual(scene.survey_requests(ROWS["codex"], None),
                         keys * scene.SURVEY_REPETITIONS * scene.SURVEY_REQUESTS_PER_SCENE)


class Repetitions(unittest.TestCase):
    """watch_once against a journey that answers as the mailbox does."""

    class Journey:
        id, record, session = "codex", {"version": "codex-cli 0.163.0"}, "s"

        def __init__(self, folder, refuse=False):
            self.run, self.refuse, self.intents, self.pressed = Path(folder), refuse, [], []
            self.table = {}
            self.daemon = self
            self.controller = self

        # The mailbox refuses a send under an intent it holds with another body.
        def send_now(self, intent, value, record):
            conflict = intent in self.intents
            self.intents.append(intent)
            record["sent"] = {"ok": False, "error": {"code": "intent_conflict"}} if conflict or self.refuse else {}
            return None if conflict or self.refuse else "letter-" + str(len(self.intents))

        def composer_checked(self): return {}
        def conversation(self): return {"events": []}
        def mark(self, pane): return 0
        def activities(self, pane, since): return []
        def take_keys(self, record): return True
        def log_mark(self): return 0
        def core_lines(self, mark): return [{"kind": "doorbell.held", "reason": "draft"}]
        def read_pane(self, pane, visible=False): return "screen"
        pane = "p1"

        def hide(self, args):
            return {"result": {"state": "pending", "bell_attempts": 0}}

        def shown(self, letter, record, done, seconds): return {"state": "delivered", "bell_attempts": 1}
        def wait_turn(self, since, value, after, seconds): return "turn", {"events": []}, []

    def probe(self, **kwargs):
        patched = {name: getattr(scene, name) for name in ("press", "wait", "WATCH_SECONDS")}
        pressed = []
        scene.press = lambda journey, keys: pressed.append([key["key"] for key in keys])
        scene.wait = lambda journey, seconds: None
        scene.WATCH_SECONDS = 0.01
        original = cells.menu_letter_verdict
        cells.menu_letter_verdict = lambda *args: (cells.PASS, "ok")
        try:
            folder = tempfile.mkdtemp()
            journey = self.Journey(folder, **kwargs)
            found = [scene.watch_once(journey, "model_picker", case("codex", "model_picker"),
                                      Path(folder) / str(n), n) for n in (1, 2, 3)]
        finally:
            cells.menu_letter_verdict = original
            for name, value in patched.items():
                setattr(scene, name, value)
        return journey, found, pressed, Path(folder)

    def test_each_repetition_of_a_scene_sends_its_own_letter(self):
        journey, found, _, _ = self.probe()
        self.assertEqual(len(set(journey.intents)), 3, journey.intents)
        self.assertEqual([record["candidate"] for record in found], ["safe"] * 3)

    def test_a_refused_letter_still_closes_the_scene_and_keeps_its_record(self):
        journey, found, pressed, folder = self.probe(refuse=True)
        for n, record in enumerate(found, 1):
            self.assertEqual(record["candidate"], "unknown")
            self.assertIn("hide request send refused", record["why"])
            self.assertTrue((folder / str(n) / "record.json").is_file())
        # Each repetition opened the scene by its keys and closed it again.
        self.assertEqual(pressed.count(["esc"]), 3)


class Settling(unittest.TestCase):
    """measure() settles every declared scene cell without a probe for the ones the run does not open."""

    class Stub:
        def __init__(self, key, mode):
            self.id, self.row, self.scene_mode = key, ROWS[key], mode
            self.scene_repetitions = None
            self.table = {key: {name: cells.cell(cells.PENDING, "") for name in cells.CELLS}}
            self.table[key]["letter"] = cells.cell(cells.PASS, "")

    def measured(self, key, mode):
        calls = []
        original = scene.release, scene.watch
        scene.release = lambda journey, name, case_, cell: calls.append(("release", name))
        scene.watch = lambda journey, name, case_, cell, how: calls.append(("watch", name))
        try:
            journey = self.Stub(key, mode)
            scene.measure(journey)
        finally:
            scene.release, scene.watch = original
        return journey, calls

    def test_the_ordinary_run_opens_only_codex_s_model_picker(self):
        journey, calls = self.measured("codex", scene.CHECK)
        self.assertEqual(calls, [("release", "model_picker")])
        settled = journey.table["codex"]
        self.assertEqual(settled["scene_popup"]["status"], cells.NOT_MEASURED)
        self.assertEqual(settled["scene_tool_approval"]["status"], cells.NOT_MEASURED)
        self.assertEqual(settled["scene_model_picker"]["status"], cells.PENDING, "the probe settles it")

    def test_an_agent_with_no_release_check_opens_nothing_in_the_ordinary_run(self):
        for key in ROWS:
            if key == "codex":
                continue
            journey, calls = self.measured(key, scene.CHECK)
            self.assertEqual(calls, [], key)
            for name in scene.SCENES:
                if not ROWS[key]["cells"][scene.cell_name(name)]:
                    continue
                status = journey.table[key][scene.cell_name(name)]["status"]
                self.assertEqual(status, cells.NOT_MEASURED, (key, name))

    def test_a_survey_watches_every_scene_opened_by_keys(self):
        journey, calls = self.measured("claude-code", scene.SURVEY)
        self.assertEqual([name for how, name in calls if how == "watch"],
                         [name for name in scene.SCENES if case("claude-code", name)["door"]
                          and case("claude-code", name)["door"]["kind"] == "keys"])


if __name__ == "__main__":
    unittest.main()
