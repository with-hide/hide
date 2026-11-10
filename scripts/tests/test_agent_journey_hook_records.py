"""The journey's named cause for a start refused on the agent's session.

A start refused with `native_identity_unavailable` used to read as the same
unclear sentence in every cell that followed it, while the reason the session
never reached Hide was recorded nowhere the run read. These read the
candidate's own hook records, where the candidate keeps them beside its state
folder, and check that a refused record write is a named cause on the cell
that met it and that the agent's later starts keep the first outcome. No
agent runs.
"""

import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells, hook_records
from agent_journey.journey import Journey
from agent_journey.report import markdown

CHECKOUT = Path(__file__).resolve().parents[2]
REFUSED = {"ok": False, "error": {"code": "native_identity_unavailable", "message": "not reported yet",
                                  "spawn": {"id": "spawn-4", "pane": "h4:p2"}}}


def contract():
    return json.loads((CHECKOUT / "contracts/agent-journey.json").read_text())


def runtime(root, *, relocated=True):
    """A run whose daemon keeps a state folder of its own, as every journey's does."""
    home = root / "daemon-home"
    state = root / "state" if relocated else home / ".hide" / "state"
    return SimpleNamespace(run=root, probe=root / "probe", home=home, state=state)


def failure(root, error, *, pane="h4:p2", event="SessionStart", relocated=True):
    folder = hook_records.folder(runtime(root, relocated=relocated))
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "last-report-failure.json").write_text(json.dumps(
        {"pane_id": pane, "agent": "claude-code", "event": event, "error": error, "at_unix_ms": 1}))
    return folder


class Records(unittest.TestCase):
    def test_the_records_folder_follows_the_state_folder_the_daemon_was_given(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            self.assertEqual(hook_records.folder(runtime(root)), root / "state" / "agent-hooks")
            # The account's own state folder keeps the folder every build wrote.
            self.assertEqual(hook_records.folder(runtime(root, relocated=False)),
                             root / "daemon-home" / ".hide" / "agent-hooks")

    def test_a_refused_record_write_is_a_named_cause(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: Permission denied (os error 13)")
            self.assertEqual(
                hook_records.identity_cause(runtime(Path(root)), "h4:p2"),
                "hook_record_refused (h4:p2, SessionStart): "
                "the pane's count could not be changed: Permission denied (os error 13)")

    def test_a_report_that_never_reached_the_core_is_its_own_named_cause(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "unavailable")
            cause = hook_records.identity_cause(runtime(Path(root)), None)
            self.assertTrue(cause.startswith("hook_report_failed (h4:p2, SessionStart): "), cause)
            # A record of another pane explains no start of this one.
            self.assertIn("names pane h4:p2, not h2:p2", hook_records.identity_cause(runtime(Path(root)), "h2:p2"))

    def test_no_record_is_named_rather_than_read_as_a_clean_bill(self):
        with tempfile.TemporaryDirectory() as root:
            cause = hook_records.identity_cause(runtime(Path(root)), "h4:p2")
            self.assertTrue(cause.startswith("no_hook_outcome: "), cause)
            self.assertIn("agent-hooks", cause)
            # A record that cannot be read back is its own outcome.
            folder = hook_records.folder(runtime(Path(root)))
            folder.mkdir(parents=True)
            (folder / "last-report-failure.json").write_text("not json")
            self.assertIn("could not be read back", hook_records.identity_cause(runtime(Path(root)), None))


class Spawn(unittest.TestCase):
    def test_the_identity_refusal_carries_its_cause_and_the_other_refusals_do_not(self):
        refused = "spawn refused: native_identity_unavailable: not reported yet"
        self.assertEqual(cells.spawn_verdict(REFUSED), (cells.FAIL, refused))
        self.assertEqual(cells.spawn_verdict(REFUSED, "hook_record_refused (h4:p2, SessionStart): denied"),
                         (cells.FAIL, refused + "; hook_record_refused (h4:p2, SessionStart): denied"))
        # A cause is the identity refusal's; another refusal keeps the product's own sentence.
        other = {"ok": False, "error": {"code": "spawn_busy", "message": "wait"}}
        self.assertEqual(cells.spawn_verdict(other, "hook_record_refused (h4:p2, SessionStart): denied"),
                         (cells.FAIL, "spawn refused: spawn_busy: wait"))


class Reported(unittest.TestCase):
    def test_a_refused_record_reaches_the_report_as_its_own_section(self):
        report = {"mode": "local", "holders": {}, "table": {}, "systems": {}, "agents": [],
                  "hook_records": {"last_failure": {"pane_id": "h3:p2"},
                                   "cause": "hook_record_refused (h3:p2, UserPromptSubmit): "
                                            "the pane's count could not be changed: File exists (os error 17)"}}
        text = markdown(report)
        self.assertIn("## Hook records", text)
        self.assertIn("hook_record_refused (h3:p2, UserPromptSubmit)", text)
        # A run whose hooks kept their records says nothing rather than a clean bill.
        self.assertNotIn("## Hook records", markdown({**report, "hook_records": None}))


class JourneyStarts(unittest.TestCase):
    def journey(self, root):
        table = cells.plan(contract(), ["claude-code"])
        return Journey(runtime(Path(root)), None, None, table,
                       next(row for row in contract() if row["id"] == "claude-code"),
                       Path("repository"), {}, None, branch="journey/claude-code", created=[],
                       sessions=Path("sessions"), route={}, composer_judge=None, failure_logs=None)

    def test_a_later_start_keeps_the_first_identity_outcome_instead_of_asking_again(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: denied")
            journey = self.journey(root)
            verdict = journey.spawn_verdict(REFUSED)
            self.assertEqual(verdict[0], cells.FAIL)
            self.assertIn("hook_record_refused (h4:p2, SessionStart)", verdict[1])
            # The second start is not measured, and says which outcome stands.
            self.assertTrue(journey.identity_refused("start_untrusted"))
            cell = journey.table["claude-code"]["start_untrusted"]
            self.assertEqual(cell["status"], cells.NOT_MEASURED)
            self.assertEqual(cell["reason"], "not measured: the identity refusal of an earlier start stands: "
                                             "hook_record_refused (h4:p2, SessionStart): "
                                             "the pane's count could not be changed: denied")
            # The settled cell keeps its first reason, and the start is skipped.
            self.assertTrue(journey.identity_refused("start_untrusted"))
            self.assertEqual(journey.table["claude-code"]["start_untrusted"], cell)

    def test_a_turn_that_never_reads_idle_names_the_record_the_hook_could_not_keep(self):
        # The turn and the counts are read from the pane's record, so a refused
        # record write is what leaves a row working: the cell says which pane
        # and why instead of only that the row never read idle.
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: Permission denied (os error 13)",
                    pane="h4:p2")
            journey = self.journey(root)
            journey.pane = "h4:p2"
            journey.reference = {"kind": "id", "value": "session-1"}
            journey.conversation = lambda: {"session": "session-1", "path": "p", "events": [
                {"role": "user", "kind": "human", "text": "Reply with exactly TOKEN", "at": 1000},
                {"role": "assistant", "text": "TOKEN", "at": 2000}]}
            journey.daemon = SimpleNamespace(activities=lambda pane, since: [("working", 1500)])
            verdict, _, _ = journey.wait_turn(0, "TOKEN", seconds=0)
            self.assertEqual(verdict[0], cells.FAIL)
            self.assertEqual(verdict[1], "row never read idle after the answer; "
                                         "hook_record_refused (h4:p2, SessionStart): "
                                         "the pane's count could not be changed: Permission denied (os error 13)")

    def test_another_panes_record_explains_no_turn_of_this_one(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: denied", pane="h4:p2")
            journey = self.journey(root)
            journey.pane = "h9:p2"
            journey.reference = {"kind": "id", "value": "session-1"}
            journey.conversation = lambda: {"session": "session-1", "path": "p", "events": [
                {"role": "user", "kind": "human", "text": "Reply with exactly TOKEN", "at": 1000},
                {"role": "assistant", "text": "TOKEN", "at": 2000}]}
            journey.daemon = SimpleNamespace(activities=lambda pane, since: [("working", 1500)])
            verdict, _, _ = journey.wait_turn(0, "TOKEN", seconds=0)
            self.assertEqual(verdict, (cells.FAIL, "row never read idle after the answer"))

    def test_a_turn_that_reads_idle_carries_no_cause_even_with_a_record_on_file(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: denied", pane="h4:p2")
            journey = self.journey(root)
            journey.pane = "h4:p2"
            journey.reference = {"kind": "id", "value": "session-1"}
            journey.conversation = lambda: {"session": "session-1", "path": "p", "events": [
                {"role": "user", "kind": "human", "text": "Reply with exactly TOKEN", "at": 1000},
                {"role": "assistant", "text": "TOKEN", "at": 2000}]}
            journey.daemon = SimpleNamespace(activities=lambda pane, since: [("stopped", 2500), ("working", 1500)])
            verdict, _, _ = journey.wait_turn(0, "TOKEN", seconds=0)
            self.assertEqual(verdict, (cells.PASS, "row read working, then idle after the answer"))

    def test_with_no_identity_refusal_the_next_start_is_measured_as_its_own(self):
        with tempfile.TemporaryDirectory() as root:
            journey = self.journey(root)
            journey.spawn_verdict({"ok": True, "value": {"pane": "h4:p2", "id": "a", "registered": True,
                                                        "runtime": "running", "connection": "connected",
                                                        "session": "native"}})
            self.assertFalse(journey.identity_refused("start_untrusted"))
            self.assertEqual(journey.table["claude-code"]["start_untrusted"]["status"], cells.PENDING)


if __name__ == "__main__":
    unittest.main()
