"""The CI lane's verdict from a journey report; no agent runs."""

import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "agent-journey-lane.py"
spec = importlib.util.spec_from_file_location("agent_journey_lane", SCRIPT)
lane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lane)


def passing_table():
    return {row["id"]: {name: {"status": "pass" if declared else "N/A", "reason": "adapter declaration"}
                        for name, declared in row["cells"].items()}
            for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1]))}


def report(table, agents, **extra):
    complete = passing_table()
    for key, cells in table.items():
        complete[key] = {**complete.get(key, {}), **cells}
    return {"fixture": False, "cleanup": {"confirmed": True}, "failures": [], "table": complete, "agents": agents, **extra}


class Login(unittest.TestCase):
    def test_missing_rows_and_cells_are_not_a_login_exception(self):
        for row in lane.active_rows(lane.load_contract(SCRIPT.parents[1])):
            for defect in ("row", "cell"):
                with self.subTest(agent=row["id"], defect=defect):
                    value = report({}, [])
                    if defect == "row":
                        del value["table"][row["id"]]
                    else:
                        del value["table"][row["id"]]["title"]
                    self.assertFalse(lane.judge(value, "login")[0])

    def test_a_spent_day_passes_as_not_measured_for_the_nightly_report(self):
        quota_reason = "provider quota exhausted (RequestsPerDay)"
        passed, notes = lane.judge(report(
            {"codex": {"start": {"status": "pass", "reason": ""},
                       "reply": {"status": "not measured", "reason": quota_reason},
                       "resume": {"status": "not measured", "reason": quota_reason}}},
            [{"id": "codex", "quota": ["RequestsPerDay"]}]), "login")
        self.assertTrue(passed)
        # The title and first line scripts/ci-flaky-report.py reads to keep the lane from being green.
        self.assertEqual(notes, ["::warning title=Not measured::agent journey codex%0Aprovider quota exhausted "
                                 "(RequestsPerDay): reply, resume not reached"])

    def test_a_refusal_the_contract_does_not_name_fails(self):
        passed, notes = lane.judge(report({"claude-code": {"reply": {"status": "fail", "reason": "rate_limit"}}},
                                          [{"id": "claude-code"}]), "login")
        self.assertFalse(passed)
        self.assertIn("::error title=agent journey claude-code::reply: rate_limit", notes)

    def test_a_refused_login_or_a_missing_cli_with_the_key_set_fails(self):
        for agent, message in (({"id": "codex", "login": "codex login --with-api-key refused: bad key"},
                                "::error title=agent journey codex::login refused: codex login --with-api-key refused: bad key"),
                               ({"id": "codex", "cli_missing": "codex"},
                                "::error title=agent journey codex::codex is not installed, though the job installs it")):
            with self.subTest(agent=agent):
                passed, notes = lane.judge(report({}, [agent]), "login")
                self.assertFalse(passed)
                self.assertIn(message, notes)

    def test_an_agent_without_its_key_is_a_notice(self):
        passed, notes = lane.judge(report({}, [{"id": "codex", "not_configured": "HIDE_CODEX_API_KEY"}]), "login")
        self.assertTrue(passed)
        self.assertTrue(notes[0].startswith("::notice "))

    def test_an_isolation_exit_fails_either_lane(self):
        for name in ("login", "fixture"):
            with self.subTest(lane=name):
                passed, notes = lane.judge(report({}, [], exit_code=2, fixture=name == "fixture"), name)
                self.assertFalse(passed)
                self.assertIn("::error title=agent journey::the journey exited 2: isolation, setup or cleanup failed", notes)

    def test_a_reason_with_a_line_break_cannot_start_a_workflow_command(self):
        passed, notes = lane.judge(report({}, [{"id": "codex", "login": "refused: 100%\n::stop-commands::x"}]), "login")
        self.assertFalse(passed)
        self.assertEqual(notes, ["::error title=agent journey codex::login refused: refused: 100%25%0A::stop-commands::x"])


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
        self.assertFalse(lane.judge(report(self.table(), [], fixture=True), "login")[0])
        for name in ("fixture", "login"):
            with self.subTest(name):
                self.assertFalse(lane.judge(report(self.table(), [], fake_model=True), name)[0])
        self.assertFalse(lane.judge(report(self.table(), []), "fake")[0])


class FakeModel(unittest.TestCase):
    def table(self, **override):
        return {agent: {**cells, **override} for agent, cells in passing_table().items()}

    def test_every_declared_cell_passing_passes(self):
        self.assertEqual(lane.judge(report(self.table(), [], fake_model=True), "fake"), (True, []))

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
                 "pin": {"version": "1.4.2", "matches": False, "pinned": "installer takes no version"}}
        passed, notes = lane.judge(report(self.table(), [agent], fake_model=True), "fake")
        self.assertFalse(passed)
        self.assertEqual(notes, ["::error title=agent journey antigravity::answered '1.4.3', contracts/agent-clis.json "
                                 "pins 1.4.2 (installer takes no version)"])

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
                for status in ("not measured", "N/A", "pending", None):
                    with self.subTest(agent=key, cell=name, status=status):
                        value = report({}, [], fake_model=True)
                        if status is None:
                            del value["table"][key][name]
                        else:
                            value["table"][key][name] = {"status": status, "reason": "omitted"}
                        self.assertFalse(lane.judge(value, "fake")[0])
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
