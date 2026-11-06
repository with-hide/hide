"""Expired external replies cannot qualify a bounded observation or new input.

Root2952 fixes each phase at the command transport cap plus its observation
window. Replies beyond that absolute end cannot certify a result or input.
The clock is the only substituted boundary; the real screen, identity and
input adapters run.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.processes import COMMAND_SECONDS, Cancelled, OwnedProcesses, ProcessError, ProcessSafetyError
from agent_live_check.timing import Deadline




class ObservationDeadlines(unittest.TestCase):
    def test_preemption_before_real_transport_admission_cannot_start_expired_input(self):
        clock, emitted = [100.0], []
        owner = OwnedProcesses()
        owner.deadline = 500
        # No actual command is launched. An external spawn attempt is itself
        # the observable failure, rather than a mocked owned-process helper.
        owner.family = "finite-admission-fixture"
        def launch(*args, **kwargs):
            emitted.append(args)
            raise AssertionError("expired pane input reached OS spawn")
        with patch("time.monotonic", lambda: clock[0]), patch("subprocess.Popen", launch):
            phase = Deadline(owner, 1)
            seconds = phase.command_seconds()
            clock[0] = 101.1  # scheduling delay after budget calculation
            with self.assertRaisesRegex(ProcessError, "command_timeout"):
                owner.run([sys.executable, "-c", "pass"], env={}, seconds=seconds, deadline=phase.end)
        self.assertEqual(emitted, [])
        self.assertEqual(owner.children, {})

    def recipe(self, kind="claude", unsafe="SELECTED"):
        scene = {"send": "", "arrived": "READY", "draft": "DRAFT", "no_match": "NO MATCH", "unsafe": unsafe}
        return {"kind": kind, "scenes": {"model_picker": scene, "rest": scene}}

    def test_a_command_asked_to_run_past_the_cap_is_refused_before_it_starts(self):
        owner = OwnedProcesses()
        self.addCleanup(owner.close)
        with self.assertRaisesRegex(ValueError, f"at most {COMMAND_SECONDS} s"):
            owner.run([sys.executable, "-c", "pass"], env=dict(os.environ), seconds=COMMAND_SECONDS + 1)
        self.assertEqual(owner.children, {})

@unittest.skipIf(sys.platform == "win32", "a Windows run's children end with its job, not a guardian")
class RunDeadline(unittest.TestCase):
    def test_a_child_started_first_ends_with_the_owners_run_not_its_own_clock(self):
        with self.assertRaisesRegex(ProcessError, "cleanup_unconfirmed"):
            with OwnedProcesses(run_seconds=3) as owner:
                child = owner.spawn([sys.executable, "-c", "import time; time.sleep(60)"],
                                    env={"PATH": "/usr/bin:/bin"}, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL)
                started = time.monotonic()
                # The guardian ends its child at the owner's deadline and says so.
                child.wait(timeout=20)
                self.assertLess(time.monotonic() - started, 15)


class Teardown(unittest.TestCase):
    def test_a_run_past_its_deadline_and_cancelled_still_runs_its_stop(self):
        # Windows run 39104633599: the deadline refused `opencode service stop` and the server stayed.
        with OwnedProcesses(run_seconds=0) as owner:
            owner.cancelled.set()
            stop = [sys.executable, "-c", "print('stopped')"]
            with self.assertRaises(Cancelled):
                owner.run(stop, env=dict(os.environ))
            with owner.teardown():
                self.assertEqual(owner.run(stop, env=dict(os.environ))[:2], (0, "stopped\n"))
            self.assertTrue(owner.cancelled.is_set())
            with self.assertRaises(Cancelled):
                owner.run(stop, env=dict(os.environ))

    def test_a_run_past_its_deadline_without_a_cancel_is_a_timeout_not_a_cancel(self):
        with OwnedProcesses(run_seconds=0) as owner:
            with self.assertRaisesRegex(ProcessError, "run_timed_out"):
                owner.run([sys.executable, "-c", "print('late')"], env=dict(os.environ))


if __name__ == "__main__":
    unittest.main()
