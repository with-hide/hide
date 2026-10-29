"""A Codex turn its provider refused as rate limited settles not measured
(with-hide/hide-private#38): the journey reads the refusal from Codex's own
log database, where codex-cli 0.164.0 recorded it on 2026-10-26, for the
agent's own conversation since the step began, and only for a turn the
agent never answered."""

from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from agent_journey import cells, provider_limit  # noqa: E402
from agent_journey.journey import Journey  # noqa: E402

# The columns Codex 0.165.0 creates its `logs` table with.
SCHEMA = """CREATE TABLE logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, ts_nanos INTEGER NOT NULL,
    level TEXT NOT NULL, target TEXT NOT NULL, feedback_log_body TEXT, module_path TEXT,
    file TEXT, line INTEGER, thread_id TEXT, process_uuid TEXT,
    estimated_bytes INTEGER NOT NULL DEFAULT 0)"""
THREAD = "01a18610-a7a1-7812-981e-e852faafe089"
OTHER = "01a18611-c6d1-72f0-9a1c-d5c1d8265321"
REFUSED = ("session_loop{thread_id=01a1}:run_sampling_request{model=gpt}: stream disconnected - retrying "
           "sampling request (1/5 in 500.99994175s)... last status: 429 Too Many Requests")
TIMED_OUT = ("session_loop{thread_id=01a1}:run_sampling_request{model=gpt}: stream disconnected - retrying "
             "sampling request (2/5 in 181ms)... sampling_error=IO error: Operation timed out (os error 60)")
# The same second on both sides of a step's start.
SECOND = 1_793_000_000


def log(codex_home, rows, name="logs_2.sqlite"):
    """Appends (thread, target, body) rows, all stamped in one second."""
    codex_home.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(codex_home / name) as db:
        db.execute(SCHEMA.replace("CREATE TABLE", "CREATE TABLE IF NOT EXISTS"))
        db.executemany("INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body, thread_id)"
                       " VALUES (?, 0, 'WARN', ?, ?, ?)",
                       [(SECOND, target, body, thread) for thread, target, body in rows])


def retry(body, thread=THREAD):
    return thread, provider_limit.RETRY_TARGET, body


class CodexRefusal(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.home = Path(folder.name) / ".codex"

    def refusal(self, mark):
        return provider_limit.codex_refusal(self.home, mark, THREAD)

    def test_a_429_the_conversation_logged_after_the_mark_is_the_provider_limit_with_its_wait(self):
        mark = provider_limit.cursor(self.home)
        self.assertEqual(mark, (None, 0), "no database before Codex first runs")
        log(self.home, [retry(REFUSED)])
        reason = self.refusal(mark)
        self.assertIn("429 Too Many Requests", reason)
        self.assertIn("retry 1 of 5 in 501 s", reason)

    def test_a_429_logged_before_the_mark_in_the_same_second_is_the_earlier_steps(self):
        log(self.home, [retry(REFUSED)])
        mark = provider_limit.cursor(self.home)
        self.assertIsNone(self.refusal(mark))
        log(self.home, [retry(REFUSED)])
        self.assertIsNotNone(self.refusal(mark), "a row after the mark in the same second is this step's")

    def test_another_conversation_or_another_target_is_no_refusal(self):
        mark = provider_limit.cursor(self.home)
        log(self.home, [retry(REFUSED, OTHER), (THREAD, "codex_core::client", "last status: 429 Too Many Requests")])
        self.assertIsNone(self.refusal(mark))

    def test_a_newer_retry_with_another_cause_is_no_provider_limit(self):
        mark = provider_limit.cursor(self.home)
        log(self.home, [retry(REFUSED), retry(TIMED_OUT)])
        self.assertIsNone(self.refusal(mark))

    def test_the_newest_schemas_database_is_read_from_its_start(self):
        log(self.home, [], name="logs_1.sqlite")
        mark = provider_limit.cursor(self.home)
        log(self.home, [retry(REFUSED)], name="logs_10.sqlite")
        self.assertEqual(provider_limit.codex_logs(self.home).name, "logs_10.sqlite")
        self.assertIsNotNone(self.refusal(mark))

    def test_a_database_that_cannot_be_read_says_so(self):
        self.home.mkdir(parents=True)
        (self.home / "logs_2.sqlite").write_bytes(b"not a database")
        with self.assertRaises(provider_limit.Unreadable):
            provider_limit.cursor(self.home)
        with self.assertRaises(provider_limit.Unreadable):
            self.refusal((None, 0))


class RefusedCell(unittest.TestCase):
    """What the journey settles a Codex cell as when its conversation logged
    a 429 since the step began."""

    def settled(self, verdict):
        with tempfile.TemporaryDirectory() as folder:
            def command(argv, check=True):
                return 0, "screen\n", ""
            runtime = SimpleNamespace(run=Path(folder), command=command,
                                      owner=SimpleNamespace(cancelled=threading.Event()))
            table = {"codex": {name: {"status": cells.PENDING} for name in ("resume", "title")}}
            record = {}
            journey = Journey(runtime, None, None, table, {"id": "codex", "herdr_kind": "codex", "model_flag": []},
                              Path("/repo"), record, None, branch="b", created=[], sessions=Path(folder),
                              route={}, composer_judge=None)
            journey.pane, journey.session = "w1:p1", THREAD
            journey.provider_mark = journey.mark_provider()
            log(Path(folder) / ".codex", [retry(REFUSED)])
            journey.settle("resume", verdict)
            return table["codex"], record

    def test_a_turn_the_agent_never_answered_is_not_measured_with_the_rest_of_the_agent(self):
        unanswered = cells.reply_verdict({"session": THREAD, "events": [{"role": "user", "text": "go"}]},
                                         THREAD, "JOURNEY-1")
        self.assertEqual(cells.kind(unanswered), cells.NO_ANSWER)
        table, record = self.settled(cells.with_turn(unanswered, (cells.FAIL, "row never read idle")))
        self.assertEqual(table["resume"]["status"], cells.NOT_MEASURED)
        self.assertIn("429 Too Many Requests", table["resume"]["reason"])
        self.assertEqual(table["title"]["status"], cells.NOT_MEASURED)
        self.assertEqual(record["provider_limit"]["failure"], "Hide's reader found no assistant message")

    def test_a_start_whose_emitter_failed_beside_an_unanswered_turn_stays_the_emitters_failure(self):
        unanswered = cells.turn_verdict([], [{"role": "user", "text": "go"}], "JOURNEY-1")
        emitter = cells.emitter_verdict(None, "herdr:codex")
        verdict = cells.start_verdict((cells.PASS, "spawned"), unanswered, emitter)
        self.assertEqual(cells.kind(unanswered), cells.NO_ANSWER)
        table, record = self.settled(verdict)
        self.assertEqual((table["resume"]["status"], table["resume"]["reason"]), emitter)
        self.assertEqual(table["title"]["status"], cells.PENDING)
        self.assertNotIn("provider_limit", record)
        alone = cells.start_verdict((cells.PASS, "spawned"), unanswered, (cells.PASS, "emitter"))
        self.assertEqual(self.settled(alone)[0]["resume"]["status"], cells.NOT_MEASURED)

    def test_an_answered_turn_another_session_or_a_plain_failure_stays_a_failure(self):
        answered = {"session": THREAD, "events": [{"role": "assistant", "text": "something else"}]}
        for verdict in (
            cells.reply_verdict(answered, THREAD, "JOURNEY-1"),
            cells.reply_verdict({"session": OTHER, "events": []}, THREAD, "JOURNEY-1"),
            cells.turn_verdict([], answered["events"], "JOURNEY-1"),
            (cells.FAIL, "the pane's terminal never took this screen's keys"),
        ):
            table, record = self.settled(verdict)
            self.assertEqual(table["resume"]["status"], cells.FAIL, verdict)
            self.assertEqual(table["title"]["status"], cells.PENDING, verdict)
            self.assertNotIn("provider_limit", record)


if __name__ == "__main__":
    unittest.main()
