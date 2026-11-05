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
        child = SimpleNamespace(stdout=os.fdopen(out_read, "rb", buffering=0),
                                stderr=os.fdopen(err_read, "rb", buffering=0))
        self.output = ResidentOutput(SimpleNamespace(cancelled=threading.Event()), child, run / "hided.log")
        self.written = 0
        test.addCleanup(self.close)

    def write(self, data):
        os.write(self.err, data)
        self.written += len(data)
        end = time.monotonic() + 10
        while self.output.mark() < self.written:
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

    def test_a_log_past_the_cells_cap_keeps_its_last_whole_lines_and_says_it_was_cut(self):
        journey = self.journey(cell_bytes=64)
        lines = b"".join(b'{"n":%02d,"pad":"xxxxxxx"}\n' % n for n in range(10))
        journey.step("resume", lines, (cells.FAIL, "no answer"))
        first, *kept = (self.run_dir / "claude-resume-hided.log").read_text().splitlines()
        self.assertIn("cut", first)
        self.assertEqual(kept[-1], '{"n":09,"pad":"xxxxxxx"}')
        self.assertLessEqual(sum(len(line) + 1 for line in kept), 64)
        self.assertTrue(all(line.startswith('{"n":') for line in kept))

    def test_a_spent_run_budget_keeps_no_more_log_and_says_so(self):
        journey = self.journey(cell_bytes=64, run_bytes=64)
        line = b'{"pad":"' + b"x" * 50 + b'"}\n'
        journey.step("resume", line, (cells.FAIL, "no answer"))
        journey.step("letter", line, (cells.FAIL, "no answer"))
        first, *kept = (self.run_dir / "claude-letter-hided.log").read_text().splitlines()
        self.assertIn("budget", first)
        self.assertEqual(kept, [])

    def test_no_account_home_reaches_the_kept_log_or_screen(self):
        journey = self.journey()
        line = (f'{{"cwd":"{MACOS}/project","home":"{LINUX}/work",'
                f'"win":"{WINDOWS_JSON}\\\\AppData","short":"{WINDOWS_SHORT}\\Temp"}}\n')
        journey.step("resume", line.encode(), (cells.FAIL, "no answer"))
        for name in ("claude-resume-hided.log", "claude-resume-pane.txt"):
            text = (self.run_dir / name).read_text()
            for home in (MACOS, LINUX, "Users\\"):
                self.assertNotIn(home, text, name)
        self.assertIn("~/project", (self.run_dir / "claude-resume-pane.txt").read_text())


class PassedRun(unittest.TestCase):
    def test_a_passed_run_keeps_no_hided_log_and_any_other_run_keeps_it(self):
        for code, kept in ((0, False), (1, True), (2, True), (3, True)):
            run = Path(tempfile.mkdtemp())
            self.addCleanup(shutil.rmtree, run)
            (run / "hided.log").write_text("{}\n")
            drop_on_pass(run, code)
            self.assertEqual((run / "hided.log").exists(), kept, code)


if __name__ == "__main__":
    unittest.main()
