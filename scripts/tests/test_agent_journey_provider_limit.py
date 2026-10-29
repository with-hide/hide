"""A Codex turn its provider refused as rate limited settles not measured
(with-hide/hide-private#38): the journey reads the refusal from Codex's own
log database, where codex-cli 0.164.0 recorded it on 2026-10-26."""

from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
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
REFUSED = ("session_loop{thread_id=01a1}:run_sampling_request{model=gpt}: stream disconnected - retrying "
           "sampling request (1/5 in 500.99994175s)... last status: 429 Too Many Requests")
TIMED_OUT = ("session_loop{thread_id=01a1}:run_sampling_request{model=gpt}: stream disconnected - retrying "
             "sampling request (1/5 in 181ms)... sampling_error=IO error: Operation timed out (os error 60)")


def write_logs(codex_home, rows, name="logs_2.sqlite"):
    codex_home.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(codex_home / name) as db:
        db.execute(SCHEMA)
        db.executemany("INSERT INTO logs (ts, ts_nanos, level, target, feedback_log_body) VALUES (?, 0, 'WARN', ?, ?)",
                       rows)


class CodexRefusal(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.home = Path(folder.name) / ".codex"
        self.since = 1_793_000_000

    def test_a_429_retry_in_the_step_is_the_provider_limit_with_its_wait(self):
        write_logs(self.home, [(self.since + 30, provider_limit.RETRY_TARGET, REFUSED)])
        reason = provider_limit.codex_refusal(self.home, self.since)
        self.assertIn("429 Too Many Requests", reason)
        self.assertIn("retry 1 of 5 in 501 s", reason)

    def test_a_429_before_the_step_or_another_retry_or_no_log_is_no_refusal(self):
        self.assertIsNone(provider_limit.codex_refusal(self.home, self.since))
        write_logs(self.home, [
            (self.since - 1, provider_limit.RETRY_TARGET, REFUSED),
            (self.since + 5, provider_limit.RETRY_TARGET, TIMED_OUT),
            (self.since + 6, "codex_core::client", "last status: 429 Too Many Requests"),
        ])
        self.assertIsNone(provider_limit.codex_refusal(self.home, self.since))

    def test_the_newest_schemas_database_is_read(self):
        write_logs(self.home, [], name="logs_1.sqlite")
        write_logs(self.home, [(self.since + 1, provider_limit.RETRY_TARGET, REFUSED)], name="logs_10.sqlite")
        self.assertEqual(provider_limit.codex_logs(self.home).name, "logs_10.sqlite")
        self.assertIsNotNone(provider_limit.codex_refusal(self.home, self.since))

    def test_a_database_that_cannot_be_read_says_so(self):
        self.home.mkdir(parents=True)
        (self.home / "logs_2.sqlite").write_bytes(b"not a database")
        with self.assertRaises(provider_limit.Unreadable):
            provider_limit.codex_refusal(self.home, self.since)


class RefusedCell(unittest.TestCase):
    def journey(self, folder, kind):
        def command(argv, check=True):
            return 0, "screen\n", ""
        runtime = SimpleNamespace(run=Path(folder), command=command,
                                  owner=SimpleNamespace(cancelled=threading.Event()))
        table = {kind: {name: {"status": cells.PENDING} for name in ("resume", "title", "start_untrusted")}}
        record = {}
        journey = Journey(runtime, None, None, table, {"id": kind, "herdr_kind": kind, "model_flag": []},
                          Path("/repo"), record, None, branch="b", created=[], sessions=Path(folder),
                          route={}, composer_judge=None)
        journey.pane = "w1:p1"
        return journey, table, record

    def test_a_codex_cell_the_provider_refused_is_not_measured_with_the_rest_of_the_agent(self):
        with tempfile.TemporaryDirectory() as folder:
            journey, table, record = self.journey(folder, "codex")
            journey.step_began = time.time()
            write_logs(Path(folder) / ".codex", [(int(time.time()), provider_limit.RETRY_TARGET, REFUSED)])
            self.assertFalse(journey.settle("resume", (cells.FAIL, "Hide's reader found no answer to the prompt")))
            self.assertEqual(table["codex"]["resume"]["status"], cells.NOT_MEASURED)
            self.assertIn("429 Too Many Requests", table["codex"]["resume"]["reason"])
            self.assertEqual(table["codex"]["title"]["status"], cells.NOT_MEASURED)
            self.assertEqual(record["provider_limit"]["failure"], "Hide's reader found no answer to the prompt")

    def test_a_failure_with_no_refusal_in_its_step_stays_a_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            journey, table, _ = self.journey(folder, "codex")
            # A refusal an earlier step met and recovered from.
            write_logs(Path(folder) / ".codex", [(int(time.time()) - 60, provider_limit.RETRY_TARGET, REFUSED)])
            journey.step_began = time.time()
            journey.settle("resume", (cells.FAIL, "Hide's reader found no answer to the prompt"))
            self.assertEqual(table["codex"]["resume"]["status"], cells.FAIL)
            self.assertEqual(table["codex"]["title"]["status"], cells.PENDING)


if __name__ == "__main__":
    unittest.main()
