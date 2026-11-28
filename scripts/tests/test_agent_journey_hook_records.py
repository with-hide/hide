"""The journey's named cause for a start refused on the agent's session.

A start refused with `native_identity_unavailable` used to read as the same
unclear sentence in every cell that followed it, while the reason the session
never reached Hide was recorded nowhere the run read. These read the
candidate's own hook records, where the candidate keeps them beside its state
folder, and check that a refused record write is a named cause on the cell
that met it and that the agent's later starts keep the first outcome. No
agent runs.
"""

import hashlib
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


def failure(root, error, *, pane="h4:p2", event="SessionStart", relocated=True, kind="record"):
    """A hook's last outcome as the hook writes it.

    `kind` is the step that failed (`record`, `report`, `record_and_report`);
    a record an earlier build wrote has none, which `legacy` asks for.
    """
    folder = hook_records.folder(runtime(root, relocated=relocated))
    folder.mkdir(parents=True, exist_ok=True)
    record = {"pane_id": pane, "agent": "claude-code", "event": event, "error": error, "at_unix_ms": 1}
    if kind is not None:
        record["kind"] = kind
    (folder / "last-report-failure.json").write_text(json.dumps(record))
    return folder


class Records(unittest.TestCase):
    def test_the_records_folder_follows_the_state_folder_the_daemon_was_given(self):
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            self.assertEqual(hook_records.folder(runtime(root)), root / "state" / "agent-hooks")
            # The account's own state folder keeps the folder every build wrote.
            self.assertEqual(hook_records.folder(runtime(root, relocated=False)),
                             root / "daemon-home" / ".hide" / "agent-hooks")

    def test_a_helpers_installation_is_composed_once_under_its_mark(self):
        # A hook's failure record sits in its own installation's folder, the
        # same rule `route::pane_records_at` applies: `<records>/installs/<mark>`
        # for any installation but the app's, whose panes are the folder itself.
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            base = root / "state" / "agent-hooks"
            self.assertEqual(hook_records.folder(runtime(root), "app"), base)
            self.assertEqual(hook_records.folder(runtime(root), ""), base)
            mark = hashlib.sha256(b"root:/helper").hexdigest()[:16]
            self.assertEqual(hook_records.folder(runtime(root), "root:/helper"),
                             base / "installs" / mark)

    def test_another_installations_record_is_never_read_as_the_app_runs_cause(self):
        # A pane id is reused across installations, so the presence of a record
        # under some other mark says nothing about the app's panes. Only the
        # candidate's own folder is read (`route::pane_records_at`), and the
        # Rust reader answers absence in this case too
        # (`records_route::another_installations_failure_record_is_not_this_daemons`).
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            folder = hook_records.folder(runtime(root), "root:/other-helper")
            folder.mkdir(parents=True)
            (folder / "last-report-failure.json").write_text(json.dumps(
                {"pane_id": "h4:p2", "agent": "claude-code", "event": "SessionStart",
                 "error": "other helper record denied", "at_unix_ms": 1, "kind": "record"}))
            # The app run's own folder holds nothing, so the cause is absence.
            cause = hook_records.identity_cause(runtime(root), "h4:p2")
            self.assertTrue(cause.startswith("no_hook_outcome: "), cause)
            self.assertIn(str(root / "state" / "agent-hooks"), cause)
            self.assertNotIn("other helper record denied", cause)
            self.assertIsNone(hook_records.last_failure(runtime(root)))
            self.assertIsNone(hook_records.pane_cause(runtime(root), "h4:p2"))
            # Asked for that installation, the same rule reads its own folder.
            self.assertEqual(
                hook_records.identity_cause(runtime(root), "h4:p2", "root:/other-helper"),
                "hook_record_refused (h4:p2, SessionStart): other helper record denied")

    def test_a_refused_record_write_is_a_named_cause(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be changed: Permission denied (os error 13)")
            self.assertEqual(
                hook_records.identity_cause(runtime(Path(root)), "h4:p2"),
                "hook_record_refused (h4:p2, SessionStart): "
                "the pane's count could not be changed: Permission denied (os error 13)")

    def test_a_report_that_never_reached_the_core_is_its_own_named_cause(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "unavailable", kind="report")
            cause = hook_records.identity_cause(runtime(Path(root)), None)
            self.assertTrue(cause.startswith("hook_report_failed (h4:p2, SessionStart): "), cause)
            # A record of another pane explains no start of this one.
            self.assertIn("names pane h4:p2, not h2:p2", hook_records.identity_cause(runtime(Path(root)), "h2:p2"))

    def test_both_steps_failing_is_named_as_both_and_keeps_each_sentence(self):
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root),
                    "the pane's count could not be changed: denied; "
                    "The hook could not reach Hide: unavailable",
                    kind="record_and_report")
            self.assertEqual(
                hook_records.identity_cause(runtime(Path(root)), "h4:p2"),
                "hook_record_and_report_failed (h4:p2, SessionStart): "
                "the pane's count could not be changed: denied; "
                "The hook could not reach Hide: unavailable")

    def test_a_report_carrying_count_wording_stays_a_report(self):
        # The same sentence an old build wrote for a refused record is what
        # one plugin path writes for a report that never arrived, so the cell
        # must read the kind and never the words.
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's count could not be stored: denied", kind="report")
            cause = hook_records.identity_cause(runtime(Path(root)), "h4:p2")
            self.assertTrue(cause.startswith("hook_report_failed (h4:p2, SessionStart): "), cause)

    def test_a_record_without_a_kind_is_named_unknown_and_keeps_its_sentence(self):
        # An earlier build's record holds no kind, and its plugin's turn path
        # wrote refused records into it too, so guessing from the sentence is
        # how a record failure used to read as a report failure.
        with tempfile.TemporaryDirectory() as root:
            failure(Path(root), "the pane's record could not be changed: denied", kind=None)
            self.assertEqual(
                hook_records.identity_cause(runtime(Path(root)), "h4:p2"),
                "hook_failure_unknown_kind (h4:p2, SessionStart): "
                "the pane's record could not be changed: denied")

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


class PaneRecord(unittest.TestCase):
    """A failed cell keeps what the pane's hooks recorded: the sessions its
    record holds and its lines of the hooks' log, so a start that names
    another agent's session shows whether the pane's hook named it
    (with-hide/hide#3124). Nothing the agent was told is copied."""

    def test_a_failed_cell_keeps_the_panes_sessions_and_log_lines_and_no_task(self):
        with tempfile.TemporaryDirectory() as root:
            run = runtime(Path(root))
            records = hook_records.folder(run)
            (records / "panes").mkdir(parents=True)
            (records / "panes" / "hW_p2.json").write_text(json.dumps({"sessions": [
                {"agent": "antigravity", "version": 1, "session": "3e3e5f18", "turn": "done", "working": 0,
                 "running": [{"id": "s-1", "task": "the parent's words"}]}]}))
            (records / hook_records.LOG_NAME).write_text("\n".join([
                json.dumps({"component": "agent_hooks", "kind": "antigravity.conversation_opened_elsewhere",
                            "pane": "hW:p2", "event": "UserPromptSubmit", "conversation": "9aacde14"}),
                json.dumps({"kind": "report.failed", "pane": "hV:p2"}),
                json.dumps({"kind": "report.failed", "pane": "hW:p2", "cause": "/srv/private-folder/notes"}),
                "not json",
            ]) + "\n")

            kept = json.loads(hook_records.pane_record(run, "hW:p2", 32 * 1024))

            self.assertEqual(kept["sessions"], [{"agent": "antigravity", "session": "3e3e5f18", "turn": "done"}])
            self.assertEqual([line.get("conversation") for line in kept["log"]], ["9aacde14", None])
            self.assertNotIn("private-folder", json.dumps(kept))
            self.assertNotIn("the parent's words", json.dumps(kept))

    def test_a_pane_the_hooks_kept_nothing_of_keeps_no_file(self):
        with tempfile.TemporaryDirectory() as root:
            self.assertIsNone(hook_records.pane_record(runtime(Path(root)), "hW:p2", 32 * 1024))

    def test_a_record_that_cannot_be_read_is_named_by_its_code_not_its_path(self):
        with tempfile.TemporaryDirectory() as root:
            run = runtime(Path(root))
            hook_records.pane_file(hook_records.folder(run), "hW:p2").mkdir(parents=True)
            kept = hook_records.pane_record(run, "hW:p2", 32 * 1024)
            self.assertIn('"record_unread"', kept)
            self.assertNotIn(root, kept)
            self.assertNotIn("agent-hooks", kept)

    def test_a_long_log_keeps_its_newest_lines_within_the_cap_and_says_it_was_cut(self):
        with tempfile.TemporaryDirectory() as root:
            run = runtime(Path(root))
            records = hook_records.folder(run)
            records.mkdir(parents=True)
            (records / hook_records.LOG_NAME).write_text("".join(
                json.dumps({"kind": "antigravity.conversation_opened_elsewhere", "pane": "hW:p2",
                            "event": "Stop", "at_unix_ms": n}) + "\n" for n in range(5000)))
            text = hook_records.pane_record(run, "hW:p2", 4096)
            self.assertLessEqual(len(text.encode()), 4096)
            kept = json.loads(text)
            self.assertTrue(kept["cut"])
            self.assertEqual(kept["log"][-1]["at_unix_ms"], 4999)

    def test_a_record_longer_than_its_read_is_named_too_large_and_its_sessions_are_cut_to_the_cap(self):
        with tempfile.TemporaryDirectory() as root:
            run = runtime(Path(root))
            record = hook_records.pane_file(hook_records.folder(run), "hW:p2")
            record.parent.mkdir(parents=True)
            record.write_text(" " * (hook_records.RECORD_BYTES + 1))
            self.assertEqual(json.loads(hook_records.pane_record(run, "hW:p2", 4096))["record_unread"], "too_large")
            record.write_text(json.dumps({"sessions": [
                {"agent": "antigravity", "session": f"s{n}", "turn": "idle"} for n in range(500)]}))
            text = hook_records.pane_record(run, "hW:p2", 2048)
            self.assertLessEqual(len(text.encode()), 2048)
            kept = json.loads(text)
            self.assertTrue(kept["cut"])
            self.assertEqual(kept["sessions"][-1]["session"], "s499")

    def test_a_log_longer_than_its_read_says_it_was_cut_even_when_the_panes_lines_fit(self):
        with tempfile.TemporaryDirectory() as root:
            run = runtime(Path(root))
            records = hook_records.folder(run)
            records.mkdir(parents=True)
            line = lambda pane, at: json.dumps({"kind": "antigravity.conversation_opened_elsewhere",
                                                "pane": pane, "event": "Stop", "at_unix_ms": at}) + "\n"
            (records / hook_records.LOG_NAME).write_text(
                line("hW:p2", 1) + "".join(line("hV:p9", n) for n in range(1500)) + line("hW:p2", 2))
            kept = json.loads(hook_records.pane_record(run, "hW:p2", 32 * 1024))
            self.assertEqual([entry["at_unix_ms"] for entry in kept["log"]], [2])
            self.assertTrue(kept["cut"])

            # With no line of the pane left in what was read, and no record
            # of it, the answer still says the log was cut.
            (records / hook_records.LOG_NAME).write_text(
                line("hW:p2", 1) + "".join(line("hV:p9", n) for n in range(1500)))
            self.assertEqual(json.loads(hook_records.pane_record(run, "hW:p2", 32 * 1024)),
                             {"pane": "hW:p2", "cut": True})
