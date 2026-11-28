"""The daily real-login journey's record and the journey account's logins
(scripts/agent_journey/daily.py, account.py; docs/VERIFICATION.md, The daily
real-login journey): what the morning check reads tells an agent's own state
apart from Hide's failure and carries no account name or screen text, and a
login an agent refreshed inside the run goes back to the account."""

import contextlib
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
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
        {"id": "grok", "version": "grok 1.0.52"},
        {"id": "cursor", "excluded": "excluded: operator decision 2026-10-17"},
        {"id": "pi", "version": "1.0.4", "login": "pi's login is open in another process (busy)",
         "login_in_use": True},
        {"id": "omp", "version": "18.7.0"},
    ],
    "table": {
        "claude-code": cells(start="pass", reply="pass", profile="not measured"),
        "grok": cells(start="pass", reply="fail", title="fail"),
        "cursor": cells(start="excluded"),
        "pi": cells(start="not measured"),
        "omp": cells(start="not measured"),
    },
}
# A synthetic path, address and token a command or an agent might print.
PRIVATE = "/Users/someone/secret someone@example.com sk-ant-0123456789abcdef0123456789"


class Summary(unittest.TestCase):
    def test_each_agent_reads_as_what_the_journey_saw(self):
        rows = {row["id"]: row for row in daily.summarize(REPORT)}
        self.assertEqual(rows["claude-code"]["result"], "pass")
        self.assertEqual(rows["claude-code"]["cli"], "2.1.302")
        self.assertEqual(rows["grok"]["result"], "fail")
        self.assertEqual(rows["grok"]["failed"], ["reply", "title"])
        self.assertEqual(rows["cursor"]["result"], "excluded")
        # A login held by an agent left open is not a sign-in the person must make.
        self.assertEqual(rows["pi"]["result"], "login in use")
        self.assertEqual(rows["omp"]["result"], "not measured")

    def test_an_agents_own_state_goes_beside_hides_failure_never_over_it(self):
        # A resume Hide broke stays failed when the agent's login expires
        # afterwards: the reader sees both.
        rows = daily.summarize(REPORT)
        asked = []
        daily.recheck(rows, {"grok": "grok"}, lambda kind: asked.append(kind) or "login needed")
        grok = next(row for row in rows if row["id"] == "grok")
        self.assertEqual((grok["result"], grok["failed"], grok["outside"]), ("fail", ["reply", "title"], "login needed"))
        self.assertEqual(asked, ["grok"])

    def test_an_agent_that_does_not_answer_first_is_not_measured_and_says_why(self):
        self.assertEqual(daily.unmeasured("codex", "login needed"),
                         {"id": "codex", "cli": None, "cells": {}, "failed": [], "result": "login needed",
                          "outside": "login needed"})
        self.assertEqual(daily.unmeasured("grok", "usage limit")["result"], "not measured")

    def test_what_the_agent_said_outside_hide_is_kept_nowhere(self):
        said = daily.Tail(daily.OUTPUT_TAIL)
        said.data += f"signed in as {PRIVATE}: usage limit reached".encode()
        stream = io.StringIO()
        with tempfile.TemporaryDirectory() as root, mock.patch.object(daily, "journey_root", return_value=Path(root)), \
                mock.patch.object(daily, "step", return_value=(1, said)):
            (Path(root) / "bin").mkdir()
            (Path(root) / "bin/grok").write_text("")
            self.assertEqual(daily.check_outside(None, "grok", stream, {}), "usage limit")
        for private in PRIVATE.split():
            self.assertNotIn(private, stream.getvalue())

    def test_what_an_agent_said_reads_as_its_kind(self):
        for said, kind in (("Claude AI usage limit reached|1764000000", "usage limit"),
                           ("You've hit your usage limit. Upgrade to Pro", "usage limit"),
                           ("ERROR: 429 Too Many Requests", "usage limit"),
                           ("Invalid API key · Please run /login", "login needed"),
                           ("Your refresh token was already used. Please log out and sign in again.", "login needed"),
                           ("401 Unauthorized", "login needed"),
                           ("Error: Not logged in. Run grok login.", "login needed"),
                           ("A new version is required; run npm i -g to update", "update needed"),
                           # A failure that only names a login on its way is not a refusal of it.
                           ("connect ECONNRESET while loading login state", "agent error"),
                           ("Segmentation fault", "agent error")):
            self.assertEqual(daily.agent_state(said), kind, said)

    def test_the_record_holds_results_and_no_account_or_screen_text(self):
        record = {"format": 1, "started": "2026-11-29T05:00:07+09:00", "if_stale": daily.STALE, "source": "main",
                  "request": None, "run": "ok", "exit": 1, "commit": "a" * 40, "minutes": 84,
                  "agents": daily.summarize(REPORT)}
        with tempfile.TemporaryDirectory() as root, mock.patch.object(daily, "journey_root", return_value=Path(root)):
            daily.publish(record, io.StringIO())
            text = (Path(root) / daily.RECORD).read_text()
            self.assertEqual(sorted(path.name for path in Path(root).iterdir()), [daily.RECORD])
        read = json.loads(text)
        self.assertEqual({row["id"]: row["result"] for row in read["agents"]},
                         {"claude-code": "pass", "grok": "fail", "cursor": "excluded", "pi": "login in use",
                          "omp": "not measured"})
        for private in ("/Users/", "another process", "private words"):
            self.assertNotIn(private, text)


class Runner(unittest.TestCase):
    SETTINGS = {"toolchain_bin": "/nowhere", "rustup_home": "/nowhere", "cargo_lock": "/nowhere"}

    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.root, ignore_errors=True))
        self.runs = self.root / "runs"
        for target, name, value in ((daily, "journey_root", mock.Mock(return_value=self.root)),
                                    (daily, "runs_folder", mock.Mock(return_value=self.runs)),
                                    (daily.signal, "signal", mock.Mock())):
            patcher = mock.patch.object(target, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def run_with(self, failure):
        (self.root / "request.json").write_text(json.dumps({"commit": "b" * 40, "id": "0123abcd"}))
        with mock.patch.object(daily, "checkout", side_effect=failure):
            try:
                code = daily.run(self.SETTINGS)
            except BaseException as error:  # noqa: BLE001 - the run re-raises what it did not expect
                code = type(error).__name__
        return code, json.loads((self.root / daily.RECORD).read_text())

    def test_a_run_that_did_not_finish_never_reads_as_ok(self):
        for failure, run in ((daily.Stage("install timed out"), "install timed out"),
                             (subprocess.TimeoutExpired("installer", 1), "runner failed: TimeoutExpired"),
                             (OSError(PRIVATE), "runner failed: OSError"),
                             (daily.Stopped(), "runner stopped")):
            with self.subTest(run=run):
                code, record = self.run_with(failure)
                self.assertNotEqual(code, 0)
                self.assertEqual((record["run"], record["request"]), (run, "0123abcd"))
                logs = "".join(path.read_text() for path in (self.root / "logs").iterdir())
                for private in PRIVATE.split():
                    self.assertNotIn(private, json.dumps(record))
                    self.assertNotIn(private, logs)

    def test_only_the_agents_that_answer_first_are_measured(self):
        kinds = {row["id"]: row["agent_cli"] for row in json.loads(
            (daily.CHECKOUT / "contracts/agent-journey.json").read_text())}
        silent = {"codex": "login needed", "grok": "usage limit"}
        journeys = []

        def journey(owner, name, argv, stream, env, seconds, cwd=daily.CHECKOUT):
            journeys.append(argv)
            run_dir = Path(argv[argv.index("--run-dir") + 1])
            run_dir.mkdir(parents=True)
            agents = argv[argv.index("--agents") + 1].split(",")
            (run_dir / "report.json").write_text(json.dumps({
                "agents": [{"id": key, "version": "1.0.0"} for key in agents],
                "table": {key: cells(start="pass") for key in agents}}))
            return 0, daily.Tail(daily.OUTPUT_TAIL)

        def outside(owner, kind, stream, env):
            return next((state for key, state in silent.items() if kinds[key] == kind), "answers")

        with mock.patch.object(daily, "checkout", return_value="c" * 40), mock.patch.object(daily, "install_clis"), \
                mock.patch.object(daily, "build"), mock.patch.object(daily, "check_outside", side_effect=outside), \
                mock.patch.object(daily, "step", side_effect=journey):
            code = daily.run(self.SETTINGS)
        record = json.loads((self.root / daily.RECORD).read_text())
        self.assertEqual((code, record["run"]), (0, "ok"))
        selected = journeys[0][journeys[0].index("--agents") + 1].split(",")
        self.assertEqual(sorted(selected), sorted(key for key in kinds
                                                  if key not in silent and key not in daily.EXCLUDED_AGENTS))
        rows = {row["id"]: row for row in record["agents"]}
        self.assertEqual(list(rows), list(kinds))
        self.assertEqual((rows["codex"]["result"], rows["codex"]["outside"]), ("login needed", "login needed"))
        self.assertEqual((rows["grok"]["result"], rows["grok"]["outside"]), ("not measured", "usage limit"))
        for key in set(daily.EXCLUDED_AGENTS) & set(kinds):
            self.assertEqual(rows[key]["result"], "excluded")
        self.assertEqual(rows["claude-code"]["result"], "pass")

    def test_a_step_logs_its_exit_and_nothing_it_printed(self):
        stream = io.StringIO()
        with daily.OwnedProcesses(run_seconds=60) as owner:
            code, _ = daily.step(owner, "build", [sys.executable, "-c", f"print({PRIVATE!r}); raise SystemExit(3)"],
                                 stream, dict(os.environ), 30, cwd=self.root)
        self.assertEqual(code, 3)
        self.assertIn("build: exit 3", stream.getvalue())
        for private in PRIVATE.split():
            self.assertNotIn(private, stream.getvalue())

    GRANDCHILD = "import os, sys, time; open(sys.argv[1], 'w').write(str(os.getpid())); time.sleep(60)"
    PARENT = ("import subprocess, sys, time\n"
              "subprocess.Popen([sys.executable, '-c', sys.argv[1], sys.argv[2]])\n"
              "time.sleep(60)\n")

    def ended(self, marker, seconds):
        """Whether the process whose pid `marker` holds ends within `seconds`."""
        end = time.monotonic() + seconds
        while not marker.is_file() or not marker.read_text():
            self.assertLess(time.monotonic(), end, "the grandchild never started")
            time.sleep(0.05)
        pid = int(marker.read_text())
        while True:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                return True
            if time.monotonic() >= end:
                os.kill(pid, 9)
                return False
            time.sleep(0.05)

    def test_a_step_ends_its_whole_tree_at_its_deadline(self):
        marker = self.root / "grandchild"
        with daily.OwnedProcesses(run_seconds=60) as owner:
            with self.assertRaisesRegex(daily.Stage, "probe timed out"):
                daily.step(owner, "probe", [sys.executable, "-c", self.PARENT, self.GRANDCHILD, str(marker)],
                           io.StringIO(), dict(os.environ), 3, cwd=self.root)
        self.assertTrue(self.ended(marker, 0))

    def test_a_runner_killed_outright_leaves_no_command_running(self):
        # The guardian of each command ends its tree when the runner's end of
        # their pipe closes, SIGKILL included.
        marker = self.root / "grandchild"
        runner = (f"import io, os, sys\nsys.path.insert(0, {str(daily.CHECKOUT / 'scripts')!r})\n"
                  "from agent_journey import daily\n"
                  "with daily.OwnedProcesses(run_seconds=60) as owner:\n"
                  "    daily.step(owner, 'probe', [sys.executable, '-c', sys.argv[1], sys.argv[2], sys.argv[3]],"
                  " io.StringIO(), dict(os.environ), 60)\n")
        process = subprocess.Popen([sys.executable, "-c", runner, self.PARENT, self.GRANDCHILD, str(marker)],
                                   cwd=self.root)
        try:
            end = time.monotonic() + 20
            while not marker.is_file() or not marker.read_text():
                self.assertLess(time.monotonic(), end, "the grandchild never started")
                time.sleep(0.05)
        finally:
            process.kill()
            process.wait()
        self.assertTrue(self.ended(marker, 10), "the grandchild outlived its killed runner")

    @unittest.skipUnless(Path("/usr/bin/lockf").exists(), "lockf(1) is BSD's and macOS's")
    def test_the_shared_lock_is_free_once_a_step_that_held_it_ends(self):
        lock, marker = self.root / "cargo.lock", self.root / "grandchild"
        argv = ["/usr/bin/lockf", "-k", "-t", "0", str(lock), sys.executable, "-c", self.PARENT, self.GRANDCHILD,
                str(marker)]
        with daily.OwnedProcesses(run_seconds=60) as owner:
            with self.assertRaisesRegex(daily.Stage, "build timed out"):
                daily.step(owner, "build", argv, io.StringIO(), dict(os.environ), 3, cwd=self.root)
        self.assertTrue(self.ended(marker, 0))
        free = subprocess.run(["/usr/bin/lockf", "-k", "-t", "0", str(lock), "/usr/bin/true"])
        self.assertEqual(free.returncode, 0)

    def test_old_runs_go_past_their_count_or_their_bytes(self):
        self.runs.mkdir()
        for day in range(1, 6):
            (self.runs / f"daily-2026112{day}").mkdir()
            (self.runs / f"daily-2026112{day}" / "report.json").write_bytes(b"x" * 100)
        daily.keep_newest(self.runs, 4, 250)
        self.assertEqual(sorted(path.name for path in self.runs.iterdir()), ["daily-20261124", "daily-20261125"])
        daily.keep_newest(self.runs, 1, 10 ** 6)
        self.assertEqual([path.name for path in self.runs.iterdir()], ["daily-20261125"])


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
        with tempfile.TemporaryDirectory() as home, mock.patch.object(account.sys, "platform", "darwin"):
            (Path(home) / account.MARKER).write_text("")
            with mock.patch.object(account, "administrator", return_value=True):
                self.assertFalse(account.dedicated(home))
            with mock.patch.object(account, "administrator", return_value=False):
                self.assertTrue(account.dedicated(home))
            (Path(home) / account.MARKER).unlink()
            with mock.patch.object(account, "administrator", return_value=False):
                self.assertFalse(account.dedicated(home))

    def test_the_journey_account_exists_only_on_macos(self):
        # Windows has no account database to ask; the CLI installer asks
        # this question on every system.
        with tempfile.TemporaryDirectory() as home, mock.patch.object(account.sys, "platform", "win32"), \
                mock.patch.object(account, "administrator", side_effect=AssertionError("asked off macOS")):
            (Path(home) / account.MARKER).write_text("")
            self.assertFalse(account.dedicated(home))


if __name__ == "__main__":
    unittest.main()
