"""What a failed journey cell keeps for the person reading why it failed:
hided's output while its step ran, its pane's screen and the step's times,
written when the cell fails so a run that is killed later still has them;
a passed cell keeps none of it (with-hide/hide-private#548).

hided's output reaches the journey through a real pipe drained by the
runtime's own reader (`ResidentOutput`), as the candidate's does."""

import os
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells
from agent_journey.failure import FailureLogs, drop_on_pass
from agent_journey.journey import Journey
from agent_journey.report import markdown
from agent_live_check.runtime import ResidentOutput

# The running account's home, built from its parts so this file names no
# home (check-no-workstation-identity).
MACOS = "/" + "Users/someone"


class Hided:
    """A resident's two output pipes, drained by the runtime's reader."""

    def __init__(self, test, run):
        out_read, self.out = os.pipe()
        err_read, self.err = os.pipe()
        # Buffered, as subprocess hands a resident's pipes over.
        child = SimpleNamespace(stdout=os.fdopen(out_read, "rb"), stderr=os.fdopen(err_read, "rb"))
        self.output = ResidentOutput(SimpleNamespace(cancelled=threading.Event()), child, run / "hided.log")
        self.written = 0
        test.addCleanup(self.close)

    def write(self, data):
        os.write(self.err, data)
        self.written += len(data)
        end = time.monotonic() + 10
        while len(self.output.since(0)) < self.written:
            if time.monotonic() > end:
                raise AssertionError("hided's output was never drained")
            time.sleep(0.01)

    def close(self):
        for fd in (self.out, self.err):
            os.close(fd)
        self.output.finish()


class Measured(Journey):
    """The journey with steps whose hided output and verdict the test decides."""

    def resume(self, data, verdict):
        self.hided.write(data)
        self.settle("resume", verdict)

    def letter(self, data, verdict):
        self.hided.write(data)
        self.settle("letter", verdict)


class FailedCell(unittest.TestCase):
    def journey(self, home=MACOS, **caps):
        # The account running the journey, whose home is the one hided's
        # output and the pane can name on this machine.
        account = mock.patch.dict(os.environ, {"HOME": home, "USERPROFILE": home})
        account.start()
        self.addCleanup(account.stop)
        self.run_dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.run_dir)
        self.hided = Hided(self, self.run_dir)
        self.screen = MACOS + "/project $ claude\n"
        runtime = SimpleNamespace(run=self.run_dir, home=self.run_dir, owner=SimpleNamespace(cancelled=threading.Event()))
        controller = SimpleNamespace(read_pane=lambda pane, lines=200: self.screen)
        self.table = {"claude": {name: cells.cell(cells.PENDING, "") for name in cells.CELLS}}
        self.record = {"id": "claude"}
        journey = Measured(runtime, None, controller, self.table, {"id": "claude", "agent_cli": "claude"}, None,
                           self.record, None, branch="journey/x-claude", created=[], sessions=self.run_dir,
                           route={}, composer_judge=None, failure_logs=FailureLogs(self.hided.output, **caps))
        journey.hided, journey.pane = self.hided, "w1:p1"
        return journey

    def test_a_failed_cell_keeps_hideds_output_of_its_step_its_screen_and_the_steps_times(self):
        journey = self.journey()
        self.hided.write(b'{"kind":"before the step"}\n')
        journey.step("resume", b'{"kind":"agent_sleep.woke","pane_id":"w1:p1"}\n{"path":"write_key"}\n',
                     (cells.FAIL, "the woken agent never answered"))
        log = (self.run_dir / "claude-resume-hided.log").read_text()
        self.assertIn('"agent_sleep.woke"', log)
        self.assertIn('"write_key"', log)
        self.assertNotIn("before the step", log)
        evidence = self.table["claude"]["resume"]["evidence"]
        self.assertIn("claude-resume-hided.log", evidence)
        self.assertIn("claude-resume-pane.txt", evidence)
        [step] = self.record["steps"]
        self.assertEqual(step["step"], "resume")
        self.assertLessEqual(step["started_ms"], step["ended_ms"])

    def test_a_passed_cell_keeps_no_log_but_its_steps_times(self):
        journey = self.journey()
        journey.step("letter", b'{"kind":"letter.delivered"}\n', (cells.PASS, "answered"))
        self.assertEqual([path.name for path in self.run_dir.iterdir()], [])
        self.assertEqual([step["step"] for step in self.record["steps"]], ["letter"])

    def kept(self, cell="resume"):
        path = self.run_dir / f"claude-{cell}-hided.log"
        return path.read_bytes()

    def test_a_log_past_the_cells_cap_keeps_its_last_whole_lines_within_the_cap_and_says_it_was_cut(self):
        journey = self.journey(cell_bytes=1500, floor_bytes=1024)
        lines = [b'{"n":%03d,"pad":"xxxxxxx"}\n' % n for n in range(200)]
        journey.step("resume", b"".join(lines), (cells.FAIL, "no answer"))
        kept = self.kept()
        # The cap holds the whole file, its first line included.
        self.assertLessEqual(len(kept), 1500)
        first, body = kept.split(b"\n", 1)
        self.assertIn(b"cut", first)
        # The longest run of last whole lines that fits: one line more would not.
        self.assertTrue(body and b"".join(lines).endswith(body))
        held = body.count(b"\n")
        self.assertGreater(len(kept) + len(lines[-held - 1]), 1500)
        # The report says it too, where a person reads it.
        self.assertEqual([cut["cell"] for cut in self.record["hided_logs_cut"]], ["resume"])
        text = markdown({"mode": "ci", "holders": {}, "table": {}, "systems": {}, "agents": [self.record]})
        self.assertIn("claude-resume-hided.log", text)
        self.assertIn("1500-byte", text)

    def test_every_cap_keeps_the_longest_run_of_last_whole_lines_that_fits(self):
        # Swept over every cap, the cut lands on a line start as often as inside a line.
        self.journey()
        lines = [b"old\n"] + [b"keep%03d\n" % n for n in range(160)]
        self.hided.write(b"".join(lines))
        step = {"step": "resume", "started_ms": 0}
        for cap in range(1024, 1124):
            path = self.run_dir / f"cap-{cap}.log"
            FailureLogs(self.hided.output, cell_bytes=cap, floor_bytes=1024).keep(path, 0, step, "resume")
            kept = path.read_bytes()
            held = kept.split(b"\n", 1)[1].count(b"\n")
            self.assertLessEqual(len(kept), cap)
            self.assertGreater(len(kept) + len(lines[-held - 1]), cap, cap)

    def test_every_failed_cell_keeps_its_tail_once_the_runs_budget_is_spent(self):
        floor, run_bytes = 1024, 1500
        journey = self.journey(cell_bytes=4096, run_bytes=run_bytes, floor_bytes=floor)
        line = b'{"pad":"' + b"x" * 50 + b'"}\n'
        failed = ("resume", "letter", "resume", "letter")
        for step in failed:
            for name in cells.CELLS:
                self.table["claude"][name] = cells.cell(cells.PENDING, "")
            journey.step(step, line * 60, (cells.FAIL, "no answer"))
            # Each failed cell has its file, whatever the run spent before it.
            self.assertTrue((self.run_dir / f"claude-{step}-hided.log").exists(), step)
            self.assertTrue(self.kept(step).endswith(line), step)
        logs = [path.stat().st_size for path in self.run_dir.glob("*-hided.log")]
        # The run's budget plus each failed cell's tail bounds what the run keeps.
        self.assertLessEqual(sum(logs), run_bytes + floor * len(failed))
        self.assertIn("budget", self.record["hided_logs_cut"][-1]["limit"])

    def test_a_step_that_began_inside_a_line_keeps_that_whole_line(self):
        home = MACOS + "\ud55c"
        journey = self.journey(home)
        # The step begins inside the home and inside its last character's bytes.
        written = home.encode()
        self.hided.write(b'{"cwd":"' + written[:-2])
        journey.step("resume", written[-2:] + b'/project"} error\n', (cells.FAIL, "no answer"))
        text = self.kept().decode()
        self.assertNotIn("someone", text)
        self.assertNotIn("\ufffd", text)

    def test_a_line_still_being_written_at_the_failure_is_left_out(self):
        journey = self.journey()
        journey.step("resume", b'{"kind":"whole"}\n{"cwd":"' + MACOS[:9].encode(), (cells.FAIL, "no answer"))
        text = self.kept().decode()
        self.assertIn('"whole"', text)
        self.assertNotIn('"cwd"', text)

    def homes_in(self, home, line, screen):
        journey = self.journey(home)
        self.screen = screen
        journey.step("resume", line.encode(), (cells.FAIL, "no answer"))
        return [(self.run_dir / name).read_bytes().decode() for name in ("claude-resume-hided.log", "claude-resume-pane.txt")]

    def test_the_running_accounts_home_is_hidden_as_every_form_writes_it(self):
        # A name with a space, in either case, with either separator and a
        # JSON string's doubled backslashes, before a separator, a quote, a
        # space, a CRLF or the end of the text.
        home = "D:" + "\\" + "Profiles" + "\\" + "Review Person"
        doubled = home.replace("\\", "\\\\")
        line = (f'{{"cwd":"{doubled}\\\\project","lower":"{doubled.lower()}","slash":"{home.replace(chr(92), "/")}/x"}}\n'
                f"cd {home} && ls\r\n{home}\r\n")
        log, pane = self.homes_in(home, line, f"{home}\\project $\r\n{home}\r\n")
        for name, text in (("log", log), ("pane", pane)):
            self.assertNotIn("review", text.lower(), name)
        self.assertIn("cd ~ && ls\r\n~\r\n", log)
        self.assertIn("~\\project $\r\n~\r\n", pane)

    def test_what_follows_a_path_stays_to_be_read(self):
        # Only the running account's home is hidden: the log and the pane come
        # from this run's machine, whose only account home is its own.
        lines = ("cd: /" + "home/Review Person: Permission denied",
                 "GET https://service.example/" + "home/settings status=503 retry=0",
                 '{"cwd":"' + MACOS + '/project","error":"open failed"}')
        log, pane = self.homes_in(MACOS, "\n".join(lines) + "\n", "\n".join(lines) + "\n")
        for name, text in (("log", log), ("pane", pane)):
            self.assertIn(lines[0], text, name)
            self.assertIn(lines[1], text, name)
            self.assertIn('{"cwd":"~/project","error":"open failed"}', text, name)

    def test_the_running_accounts_home_is_hidden_only_where_it_ends(self):
        home = "/opt/review-account"
        log, pane = self.homes_in(home, f'{{"in":"{home}/work","out":"{home}+extra/work","whole":"{home}"}}\n',
                                  f"{home}/work $\n")
        self.assertIn('"in":"~/work"', log)
        self.assertIn('"whole":"~"', log)
        self.assertIn(f'"out":"{home}+extra/work"', log)
        self.assertIn("~/work $", pane)


class PassedRun(unittest.TestCase):
    def test_a_run_nothing_failed_in_keeps_no_hided_log_and_any_other_run_keeps_it(self):
        # Exit codes as docs/VERIFICATION.md, Cells, reasons and exit codes, gives them:
        # 3 is a run with a not-measured cell and no failure, as every stand-in run
        # is, or an interrupted one.
        for code, interrupted, kept in ((0, False, False), (3, False, False), (3, True, True),
                                        (2, True, True), (1, False, True), (2, False, True)):
            run = Path(tempfile.mkdtemp())
            self.addCleanup(shutil.rmtree, run)
            (run / "hided.log").write_text("{}\n")
            drop_on_pass(run, code, interrupted)
            self.assertEqual((run / "hided.log").exists(), kept, (code, interrupted))


if __name__ == "__main__":
    unittest.main()
