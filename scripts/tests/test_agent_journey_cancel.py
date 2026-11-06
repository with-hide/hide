"""A stop the operator asks for (Ctrl-C or a cancel) during a cell's wait ends
the run and concludes no cell: the wait raises `Cancelled`, which no cell's
`except ProcessError` catches, so the cell stays pending instead of failing
with "run_cancelled" as its reason."""

import json
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from agent_journey import cells, journey as journey_module, kit  # noqa: E402
from agent_journey.daemon import Daemon  # noqa: E402
from agent_journey.journey import Journey  # noqa: E402
from agent_live_check.processes import Cancelled, ProcessError  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def daemon_over(cancelled):
    """A `Daemon` with no socket: its snapshot never answers, so a wait ends only by its bound or the cancel."""
    daemon = object.__new__(Daemon)
    daemon.cancelled, daemon.lock = cancelled, threading.Condition()
    daemon.rest, daemon.closed, daemon.errors = None, None, []
    return daemon


class CancelDuringACellWait(unittest.TestCase):
    def journey(self, folder, daemon, cancelled, runtime_wait=None):
        runtime = SimpleNamespace(run=Path(folder), home=Path(folder), wait=runtime_wait,
                                  owner=SimpleNamespace(cancelled=cancelled))
        table = cells.plan(json.loads((ROOT / "contracts/agent-journey.json").read_text()), ["codex"])
        cells.settle(table, "codex", "start", cells.PASS, "fixture")
        journey = Journey(runtime, daemon, None, table, {"id": "codex", "herdr_kind": "codex", "model_flag": []},
                          Path("/repo"), {}, None, branch="b", created=[], sessions=Path(folder),
                          route={}, composer_judge=None, failure_logs=SimpleNamespace(mark=lambda: 0))
        return journey, table["codex"]

    def cancel_soon(self, cancelled):
        timer = threading.Timer(0.2, cancelled.set)
        timer.start()
        self.addCleanup(timer.cancel)

    def test_a_cell_that_catches_its_own_wait_does_not_record_the_cancel(self):
        # The kit cell catches its wait's ProcessError and carries on with a `kit_wait` reason.
        cancelled = threading.Event()
        with tempfile.TemporaryDirectory() as folder:
            journey, table = self.journey(folder, daemon_over(cancelled), cancelled)
            journey.mark_provider = lambda: None
            self.cancel_soon(cancelled)
            with mock.patch.object(kit, "KIT_SECONDS", 30), self.assertRaises(Cancelled):
                journey.step("integration")
            self.assertEqual(table["integration"]["status"], cells.PENDING)
            self.assertNotIn("kit_wait", journey.record.get("integration", {}))

    def test_a_step_does_not_fail_its_open_cell_for_a_cancel(self):
        cancelled = threading.Event()

        def wait(_predicate, _seconds):
            cancelled.wait(5)
            if cancelled.is_set():
                raise Cancelled("run_cancelled")
            raise ProcessError("scene_timeout")
        with tempfile.TemporaryDirectory() as folder:
            journey, table = self.journey(folder, None, cancelled, runtime_wait=wait)
            journey.mark_provider = lambda: None
            self.cancel_soon(cancelled)
            with mock.patch.dict(journey_module.MEASURES, {"letter": lambda self: self.runtime.wait(None, 30)}), \
                    self.assertRaises(Cancelled):
                journey.step("letter")
            self.assertEqual(table["letter"]["status"], cells.PENDING)

    def test_the_same_wait_that_times_out_still_fails_its_cell(self):
        cancelled = threading.Event()

        def wait(_predicate, _seconds):
            raise ProcessError("scene_timeout")
        with tempfile.TemporaryDirectory() as folder:
            journey, table = self.journey(folder, None, cancelled, runtime_wait=wait)
            journey.mark_provider = lambda: None
            journey.capture = lambda *_args, **_kw: None
            journey.daemon_log = lambda *_args, **_kw: None
            journey.provider_refusal = lambda: None
            with mock.patch.dict(journey_module.MEASURES, {"letter": lambda self: self.runtime.wait(None, 30)}):
                journey.step("letter")
            self.assertEqual(table["letter"]["status"], cells.FAIL)
            self.assertEqual(table["letter"]["reason"], "scene_timeout")


if __name__ == "__main__":
    unittest.main()
