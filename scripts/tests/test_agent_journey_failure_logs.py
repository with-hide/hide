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
from agent_live_check.runtime import ResidentOutput

# Account homes as hided's output and a pane's screen write them, each built
# from its parts so this file names no home (check-no-workstation-identity).
MACOS, LINUX = "/" + "Users/someone", "/" + "home/runner"
WINDOWS_JSON = "C:" + "\\\\" + "Users" + "\\\\" + "runneradmin"
WINDOWS_SHORT = "C:" + "\\" + "Users" + "\\" + "RUNNER~1"


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
    def journey(self, **caps):
        self.run_dir = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.run_dir)
        self.hided = Hided(self, self.run_dir)
        self.screen = MACOS + "/project $ claude\n"
        runtime = SimpleNamespace(run=self.run_dir, home=self.run_dir, owner=SimpleNamespace(cancelled=threading.Event()))
        controller = SimpleNamespace(read_pane=lambda pane, lines=200: self.screen)
        self.table = {"claude": {name: cells.cell(cells.PENDING, "") for name in cells.CELLS}}
        self.record = {"id": "claude"}
        journey = Measured(runtime, None, controller, self.table, {"id": "claude", "herdr_kind": "claude"}, None,
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
        journey = self.journey(cell_bytes=400)
        lines = [b'{"n":%02d,"pad":"xxxxxxx"}\n' % n for n in range(40)]
        journey.step("resume", b"".join(lines), (cells.FAIL, "no answer"))
        kept = self.kept()
        # The cap holds the whole file, its first line included.
        self.assertLessEqual(len(kept), 400)
        first, body = kept.split(b"\n", 1)
        self.assertIn(b"cut", first)
        # The longest run of last whole lines that fits: one line more would not.
        self.assertTrue(body and b"".join(lines).endswith(body))
        held = body.count(b"\n")
        self.assertGreater(len(kept) + len(lines[-held - 1]), 400)

    def test_every_cap_keeps_the_longest_run_of_last_whole_lines_that_fits(self):
        # Swept over every cap, the cut lands on a line start as often as inside a line.
        self.journey()
        lines = [b"old\n"] + [b"keep%02d\n" % n for n in range(12)]
        self.hided.write(b"".join(lines))
        step = {"step": "resume", "started_ms": 0}
        for cap in range(220, 320):
            path = self.run_dir / f"cap-{cap}.log"
            FailureLogs(self.hided.output, cell_bytes=cap).keep(path, 0, step, "resume")
            if not path.exists():
                continue
            kept = path.read_bytes()
            held = kept.split(b"\n", 1)[1].count(b"\n")
            self.assertLessEqual(len(kept), cap)
            if held < len(lines):
                self.assertGreater(len(kept) + len(lines[-held - 1]), cap, cap)

    def test_the_runs_failed_cells_keep_no_more_than_the_runs_budget_and_a_spent_one_is_named(self):
        journey = self.journey(cell_bytes=400, run_bytes=1000)
        line = b'{"pad":"' + b"x" * 50 + b'"}\n'
        for step in ("resume", "letter", "resume", "letter"):
            for name in cells.CELLS:
                self.table["claude"][name] = cells.cell(cells.PENDING, "")
            journey.step(step, line * 20, (cells.FAIL, "no answer"))
        logs = list(self.run_dir.glob("*-hided.log"))
        self.assertLessEqual(sum(path.stat().st_size for path in logs), 1000)
        self.assertTrue(all(path.stat().st_size <= 400 for path in logs))
        # A cell the budget had no room for says so in the report rather than a file past it.
        self.assertTrue(self.record.get("hided_logs_not_kept"))

    def test_a_step_that_began_inside_a_line_keeps_that_whole_line(self):
        journey = self.journey()
        # The step begins after a home's prefix and inside a character's bytes.
        self.hided.write(b'{"cwd":"' + MACOS[:7].encode() + "\ud55c".encode()[:1])
        journey.step("resume", "\ud55c".encode()[1:] + MACOS[7:].encode() + b'/project"} error\n',
                     (cells.FAIL, "no answer"))
        text = self.kept().decode()
        self.assertNotIn("someone", text)
        self.assertNotIn("\ufffd", text)

    def test_a_line_still_being_written_at_the_failure_is_left_out(self):
        journey = self.journey()
        journey.step("resume", b'{"kind":"whole"}\n{"cwd":"' + MACOS[:9].encode(), (cells.FAIL, "no answer"))
        text = self.kept().decode()
        self.assertIn('"whole"', text)
        self.assertNotIn('"cwd"', text)

    def test_no_account_home_reaches_the_kept_log_or_screen(self):
        spaced = "/" + "Users/Review Person"
        lower = "C:" + "\\" + "users" + "\\" + "review-person"
        with mock.patch.dict(os.environ, {"HOME": spaced, "USERPROFILE": spaced}):
            journey = self.journey()
            self.screen = f"{spaced}/project $ {lower}\\project\n"
            line = (f'{{"cwd":"{MACOS}/project","home":"{LINUX}/work",'
                    f'"win":"{WINDOWS_JSON}\\\\AppData","short":"{WINDOWS_SHORT}\\Temp",'
                    f'"spaced":"{spaced}/project","lower":"{lower}\\project"}}\n')
            journey.step("resume", line.encode(), (cells.FAIL, "no answer"))
        for name in ("claude-resume-hided.log", "claude-resume-pane.txt"):
            text = (self.run_dir / name).read_text()
            for home in (MACOS, LINUX, "Users\\", "Person", "review-person"):
                self.assertNotIn(home, text, name)
        self.assertIn("~/project", (self.run_dir / "claude-resume-pane.txt").read_text())


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
