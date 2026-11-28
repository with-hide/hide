"""The daily real-login journey's record and the journey account's logins
(scripts/agent_journey/daily.py, account.py; docs/VERIFICATION.md, The daily
real-login journey): what the morning check reads tells an agent's own state
apart from Hide's failure and carries no account name or screen text, and a
login an agent refreshed inside the run goes back to the account."""

import contextlib
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from agent_journey import account, daily  # noqa: E402
from agent_live_check.runtime import kept_login  # noqa: E402


def cells(**statuses):
    return {name: {"status": status, "reason": "screen said: private words", "evidence": None} for name, status in statuses.items()}


REPORT = {
    "agents": [
        {"id": "claude-code", "version": "2.1.302 (Claude Code)"},
        {"id": "codex", "version": "codex-cli 0.163.0",
         "login": "not logged in: run codex login yourself; the journey never logs in"},
        {"id": "grok", "version": "grok 1.0.52"},
        {"id": "cursor", "excluded": "excluded: operator decision 2026-10-17"},
        {"id": "pi", "version": "1.0.4", "login": "pi's login is open in another process (busy)"},
    ],
    "table": {
        "claude-code": cells(start="pass", reply="pass", profile="not measured"),
        "codex": cells(start="not measured", reply="not measured"),
        "grok": cells(start="pass", reply="fail", title="fail"),
        "cursor": cells(start="excluded"),
        "pi": cells(start="not measured"),
    },
}


class Summary(unittest.TestCase):
    def test_each_agent_reads_as_what_its_reader_acts_on(self):
        rows = {row["id"]: row for row in daily.summarize(REPORT)}
        self.assertEqual(rows["claude-code"]["result"], "pass")
        self.assertEqual(rows["claude-code"]["cli"], "2.1.302")
        self.assertEqual(rows["codex"]["result"], "login needed")
        self.assertEqual(rows["grok"]["result"], "fail")
        self.assertEqual(rows["grok"]["failed"], ["reply", "title"])
        self.assertEqual(rows["cursor"]["result"], "excluded")
        # A login held by another process is not a sign-in the person must make.
        self.assertEqual(rows["pi"]["result"], "not measured")

    def test_a_failed_agent_that_cannot_answer_outside_hide_carries_its_state(self):
        rows = daily.summarize(REPORT)
        with mock.patch.object(daily, "check_outside", side_effect=lambda kind, stream, env: "usage limit"):
            daily.recheck(rows, {"grok": "grok"}, None, {})
        grok = next(row for row in rows if row["id"] == "grok")
        self.assertEqual(grok["result"], "usage limit")
        self.assertEqual(grok["failed"], ["reply", "title"])

    def test_a_failed_agent_that_answers_outside_hide_stays_hides_failure(self):
        rows = daily.summarize(REPORT)
        with mock.patch.object(daily, "check_outside", return_value=None):
            daily.recheck(rows, {"grok": "grok"}, None, {})
        self.assertEqual(next(row for row in rows if row["id"] == "grok")["result"], "fail")

    def test_what_an_agent_said_reads_as_its_kind(self):
        for said, kind in (("Claude AI usage limit reached|1764000000", "usage limit"),
                           ("You've hit your usage limit. Upgrade to Pro", "usage limit"),
                           ("ERROR: 429 Too Many Requests", "usage limit"),
                           ("Invalid API key · Please run /login", "login needed"),
                           ("Your refresh token was already used. Please log out and sign in again.", "login needed"),
                           ("401 Unauthorized", "login needed"),
                           ("A new version is required; run npm i -g to update", "update needed"),
                           ("Segmentation fault", "agent error")):
            self.assertEqual(daily.agent_state(said), kind, said)

    def test_the_record_holds_results_and_no_account_or_screen_text(self):
        record = {"format": 1, "started": "2026-11-29T05:00+09:00", "if_stale": daily.STALE, "source": "main",
                  "run": "ok", "exit": 1, "commit": "a" * 40, "minutes": 84, "agents": daily.summarize(REPORT)}
        with tempfile.TemporaryDirectory() as root, mock.patch.object(daily, "journey_root", return_value=Path(root)):
            daily.publish(record, io.StringIO())
            text = (Path(root) / daily.RECORD).read_text()
            self.assertEqual(sorted(path.name for path in Path(root).iterdir()), [daily.RECORD])
        read = json.loads(text)
        self.assertEqual({row["id"]: row["result"] for row in read["agents"]},
                         {"claude-code": "pass", "codex": "login needed", "grok": "fail", "cursor": "excluded",
                          "pi": "not measured"})
        self.assertEqual(read["agents"][2]["failed"], ["reply", "title"])
        for private in ("/Users/", "not logged in", "another process", "private words"):
            self.assertNotIn(private, text)


class WriteBackTest(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.folder))
        (self.folder / "account").mkdir()
        (self.folder / "private").mkdir()

    def copy(self, name, content):
        source, private = self.folder / "account" / name, self.folder / "private" / name
        source.write_bytes(content)
        private.write_bytes(content)
        return source, private

    def test_a_login_the_agent_refreshed_goes_back_to_the_account(self):
        source, private = self.copy("auth.json", b'{"token": "old"}')
        entry = account.WriteBack(source, private)
        private.write_bytes(b'{"token": "new"}')
        self.assertEqual(account.write_back([entry], {private: kept_login(private)}),
                         {"auth.json": "written back"})
        self.assertEqual(source.read_bytes(), b'{"token": "new"}')
        self.assertEqual(source.stat().st_mode & 0o777, 0o600)

    def test_an_untouched_login_stays_as_it_is(self):
        source, private = self.copy("auth.json", b'{"token": "old"}')
        entry = account.WriteBack(source, private)
        before = source.stat().st_ino
        self.assertEqual(entry.apply(kept_login(private)), "unchanged")
        self.assertEqual(source.stat().st_ino, before)

    def test_a_login_changed_in_the_account_during_the_run_is_kept(self):
        source, private = self.copy("auth.json", b'{"token": "old"}')
        entry = account.WriteBack(source, private)
        private.write_bytes(b'{"token": "copy"}')
        source.write_bytes(b'{"token": "signed in again"}')
        self.assertIn("changed during the run", entry.apply(kept_login(private)))
        self.assertEqual(source.read_bytes(), b'{"token": "signed in again"}')

    def test_a_database_goes_back_whole_with_no_stale_journal(self):
        source = self.folder / "account" / "agent.db"
        with contextlib.closing(sqlite3.connect(source)) as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("CREATE TABLE auth (token TEXT)")
            db.execute("INSERT INTO auth VALUES ('old')")
            db.commit()
        private = self.folder / "private" / "agent.db"
        with contextlib.closing(sqlite3.connect(source)) as db, contextlib.closing(sqlite3.connect(private)) as copy:
            db.backup(copy)
        entry = account.WriteBack(source, private)
        with contextlib.closing(sqlite3.connect(private)) as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("UPDATE auth SET token = 'new'")
            db.commit()
        self.assertEqual(entry.apply(kept_login(private)), "written back")
        self.assertFalse(source.with_name("agent.db-wal").exists())
        with contextlib.closing(sqlite3.connect(source)) as db:
            self.assertEqual(db.execute("SELECT token FROM auth").fetchall(), [("new",)])


class Account(unittest.TestCase):
    def test_the_signed_in_journey_asks_whether_it_runs_on_the_journey_account(self):
        from agent_journey import cli
        from agent_live_check.protection import ProtectionError
        with mock.patch.object(cli, "journey_account", return_value=False) as asked:
            # Refused after the question, for an agent no adapter declares.
            with self.assertRaises(ProtectionError):
                cli.main(["--mode", "local", "--agents", "no-such-agent"])
        asked.assert_called_once()


    def test_an_administrator_is_never_the_journey_account(self):
        with tempfile.TemporaryDirectory() as home:
            (Path(home) / account.MARKER).write_text("")
            with mock.patch.object(account, "administrator", return_value=True):
                self.assertFalse(account.dedicated(home))
            with mock.patch.object(account, "administrator", return_value=False):
                self.assertTrue(account.dedicated(home))
            (Path(home) / account.MARKER).unlink()
            with mock.patch.object(account, "administrator", return_value=False):
                self.assertFalse(account.dedicated(home))


if __name__ == "__main__":
    unittest.main()
