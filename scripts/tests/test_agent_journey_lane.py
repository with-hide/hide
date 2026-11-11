"""The CI lane's verdict from a journey report; no agent runs."""

import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "agent-journey-lane.py"
spec = importlib.util.spec_from_file_location("agent_journey_lane", SCRIPT)
lane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lane)


def passing_table():
    """What an ordinary run of the fake lane must report: pass for every
    declared cell, and for a scene cell what its release check or finding
    proves (scene.lane_status)."""
    return {row["id"]: {name: {"status": lane.lane_status(row, name), "reason": "adapter declaration"}
                        for name in row["cells"]}
            for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1]))}


def report(table, agents, **extra):
    complete = passing_table()
    # A fake-model run records every approved transition gap as excluded.
    if extra.get("fake_model"):
        for (agent, name), reason in lane.excluded_cells(True).items():
            if agent in complete:
                complete[agent][name] = {"status": "excluded", "reason": reason}
    for key, cells in table.items():
        complete[key] = {**complete.get(key, {}), **cells}
    return {"fixture": False, "cleanup": {"confirmed": True}, "failures": [], "table": complete, "agents": agents, **extra}


class EitherLane(unittest.TestCase):
    def test_a_failed_cell_fails(self):
        passed, notes = lane.judge(report({"claude-code": {"reply": {"status": "fail", "reason": "rate_limit"}}},
                                          [{"id": "claude-code"}], fake_model=True), "fake")
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey claude-code::reply: rate_limit", notes)

    def test_an_isolation_exit_fails_either_lane(self):
        for name, flag in (("fake", "fake_model"), ("fixture", "fixture")):
            with self.subTest(lane=name):
                passed, notes = lane.judge(report({}, [], exit_code=2, **{flag: True}), name)
                self.assertFalse(passed)
                self.assertIn("::error title=agent journey::the journey exited 2: isolation, setup or cleanup failed", notes)

    def test_a_reason_with_a_line_break_cannot_start_a_workflow_command(self):
        passed, notes = lane.judge(report({"codex": {"reply": {"status": "fail", "reason": "refused: 100%\n::stop-commands::x"}}},
                                          [], fake_model=True), "fake")
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey codex::reply: refused: 100%25%0A::stop-commands::x", notes)
        self.assertFalse(any("\n" in note for note in notes))


class Fixture(unittest.TestCase):
    def table(self, **override):
        cells = {name: {"status": "pass", "reason": ""} for name in lane.FIXTURE_CELLS}
        cells["start_untrusted"] = {"status": "not measured", "reason": "a stand-in has no folder question"}
        return {agent: {**cells, **override} for agent in lane.FIXTURE_AGENTS}

    def test_every_imitated_cell_passing_passes(self):
        self.assertEqual(lane.judge(report(self.table(), [], fixture=True), "fixture"), (True, []))

    def test_an_imitated_cell_not_passing_fails(self):
        passed, notes = lane.judge(report(self.table(resume={"status": "not measured", "reason": "stopped"}), [],
                                          fixture=True), "fixture")
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey codex::resume is not measured, a stand-in passes it: stopped", notes)

    def test_each_lane_refuses_the_other_lanes_report(self):
        self.assertFalse(lane.judge(report(self.table(), []), "fixture")[0])
        self.assertFalse(lane.judge(report(self.table(), [], fixture=True), "fake")[0])
        self.assertFalse(lane.judge(report(self.table(), [], fake_model=True), "fixture")[0])
        self.assertFalse(lane.judge(report(self.table(), []), "fake")[0])


class FakeModel(unittest.TestCase):
    def table(self, **override):
        table = {agent: {**cells, **override} for agent, cells in passing_table().items()}
        for (agent, name), reason in lane.excluded_cells(True).items():
            if agent in table:
                table[agent][name] = {"status": "excluded", "reason": reason}
        return table

    def test_every_declared_cell_passing_passes(self):
        self.assertEqual(lane.judge(report(self.table(), [], fake_model=True), "fake"), (True, []))

    def test_no_agent_cell_may_record_an_exclusion(self):
        # Pi, OpenCode, and the Codex, Antigravity and OMP fork and wake
        # cells waited for the own engine's first-input fix; every one of
        # them is required now.
        cells_of = {"pi": lane.CELLS, "opencode": lane.CELLS, "codex": ("fork",), "antigravity": ("fork",),
                    "omp": ("sleep_wake", "resume", "fork", "title")}
        for agent, names in cells_of.items():
            for name in names:
                with self.subTest(agent=agent, cell=name):
                    value = report(self.table(), [], fake_model=True)
                    self.assertEqual(lane.judge(value, "fake"), (True, []))
                    value["table"][agent][name] = {"status": "excluded", "reason": "own-start-fixes 노드 대기"}
                    self.assertFalse(lane.judge(value, "fake")[0])

    def test_the_folder_question_a_stand_in_cannot_ask_must_pass(self):
        # The lane takes the stand-ins' place only when it measures what they cannot.
        passed, notes = lane.judge(report(self.table(start_untrusted={"status": "not measured", "reason": "x"}), [],
                                          fake_model=True), "fake")
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey claude-code::start_untrusted is not measured, the fake lane "
                      "requires pass from the adapter declaration: x", notes)

    def test_a_missing_cli_fails(self):
        passed, _ = lane.judge(report(self.table(), [{"id": "grok", "cli_missing": "grok"}], fake_model=True), "fake")
        self.assertFalse(passed)

    def test_a_cli_answering_another_version_than_the_pin_fails(self):
        agent = {"id": "antigravity", "version": "1.4.3",
                 "pin": {"version": "1.4.2", "matches": False}}
        passed, notes = lane.judge(report(self.table(), [agent], fake_model=True), "fake")
        self.assertFalse(passed)
        self.assertEqual(notes, ["::error title=agent journey antigravity::answered '1.4.3', contracts/agent-clis.json "
                                 "pins 1.4.2"])

    def test_a_report_without_a_row_the_lane_stands_for_fails(self):
        # Every row it has passes, but Codex's is not there to pass.
        for name, flag in (("fake", "fake_model"), ("fixture", "fixture")):
            with self.subTest(name):
                value = report(self.table(), [], **{flag: True})
                del value["table"]["codex"]
                passed, notes = lane.judge(value, name)
                self.assertFalse(passed)
                self.assertIn("::error title=agent journey codex::the report has no codex row", notes)

    def test_every_registered_agent_and_declared_cell_is_required(self):
        # A release must not be green when any supported adapter is absent,
        # or when its declared behavior was skipped or incorrectly called N/A.
        for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1])):
            key = row["id"]
            with self.subTest(agent=key, defect="missing row"):
                value = report({}, [], fake_model=True)
                del value["table"][key]
                self.assertFalse(lane.judge(value, "fake")[0])
            for name, declared in row["cells"].items():
                if not declared:
                    continue
                for status in ("pass", "not measured", "N/A", "pending", None):
                    if status == lane.lane_status(row, name):
                        continue
                    with self.subTest(agent=key, cell=name, status=status):
                        value = report({}, [], fake_model=True)
                        if status is None:
                            del value["table"][key][name]
                        else:
                            value["table"][key][name] = {"status": status, "reason": "omitted"}
                        self.assertFalse(lane.judge(value, "fake")[0])
    def test_a_scene_nobody_has_proven_cannot_be_reported_as_passing(self):
        # A scene whose finding is unknown and which carries no release check
        # reads `not measured` in the ordinary run; a pass there would claim
        # a proof the run never made.
        rows = {row["id"]: row for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1]))}
        self.assertEqual(lane.lane_status(rows["claude-code"], "scene_popup"), "not measured")
        value = report(self.table(), [], fake_model=True)
        value["table"]["claude-code"]["scene_popup"] = {"status": "pass", "reason": "x"}
        passed, notes = lane.judge(value, "fake")
        self.assertFalse(passed)
        self.assertTrue(any("scene_popup is pass" in note for note in notes))

    def test_a_new_registration_automatically_becomes_required(self):
        rows = lane.load_contract(SCRIPT.parents[1])
        added = {**rows[0], "id": "newly-registered-agent"}
        value = report({}, [], fake_model=True)
        passed, notes = lane.judge(value, "fake", [*rows, added])
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey newly-registered-agent::the report has no newly-registered-agent row", notes)

    def test_an_undeclared_cell_cannot_be_reported_as_proven(self):
        for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1])):
            for name, declared in row["cells"].items():
                if not declared:
                    with self.subTest(agent=row["id"], cell=name):
                        value = report({}, [], fake_model=True)
                        value["table"][row["id"]][name] = {"status": "pass", "reason": "claimed support"}
                        self.assertFalse(lane.judge(value, "fake")[0])

    def test_a_run_asked_for_some_agents_stands_for_those_alone(self):
        # The agent canary reruns only the agents whose latest release failed, at their pins.
        value = report({}, [], fake_model=True)
        value["table"] = {"codex": value["table"]["codex"]}
        self.assertEqual(lane.judge(value, "fake", agents=["codex"]), (True, []))
        self.assertFalse(lane.judge(value, "fake")[0])
        value["table"]["codex"]["letter"] = {"status": "fail", "reason": "no bell"}
        passed, notes = lane.judge(value, "fake", agents=["codex"])
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey codex::letter: no bell", notes)
        # An agent asked for but not in the report fails; an unknown or empty selection is refused.
        self.assertFalse(lane.judge(value, "fake", agents=["codex", "claude-code"])[0])
        for agents in (["no-such-agent"], []):
            with self.subTest(agents=agents):
                self.assertFalse(lane.judge(report({}, [], fake_model=True), "fake", agents=agents)[0])


class MalformedReport(unittest.TestCase):
    def test_missing_tables_or_invalid_cells_fail_with_an_annotation(self):
        for table in (None, [], {"codex": None}, {"codex": {"title": {}}},
                      {"codex": {"title": {"status": "pass"}}}):
            with self.subTest(table=table):
                value = {**report({}, [], fake_model=True), "table": table}
                passed, notes = lane.judge(value, "fake")
                self.assertFalse(passed)
                self.assertTrue(any(note.startswith("::error ") for note in notes))


if __name__ == "__main__":
    unittest.main()
