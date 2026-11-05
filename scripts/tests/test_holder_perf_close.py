"""A measurement round's close tells what it left without replacing the
round's own failure (scripts/holder-perf.py, Round.close and main)."""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SCRIPTS))


def perf():
    spec = importlib.util.spec_from_file_location("holder_perf", SCRIPTS / "holder-perf.py")
    module = importlib.util.module_from_spec(spec)
    # The measurement loads macOS's libproc as it is imported; its close
    # runs nothing of it, so these run on every system.
    with mock.patch("ctypes.CDLL"), mock.patch("ctypes.util.find_library", return_value="stand-in"):
        spec.loader.exec_module(module)
    return module


class Owned:
    def __init__(self, left):
        self.left = left

    def close(self):
        return list(self.left)


class Path_:
    def close(self):
        raise OSError("socket moved")


class RoundClose(unittest.TestCase):
    def setUp(self):
        self.perf = perf()
        self.out = Path(tempfile.mkdtemp(prefix="hpc-")) / "out.json"
        self.addCleanup(lambda: (self.out.unlink(missing_ok=True), self.out.parent.rmdir()))

    def test_close_answers_a_path_that_failed_and_what_still_runs_without_raising(self):
        current = object.__new__(self.perf.Round)
        current.args = mock.Mock(path="holder")
        current.path = Path_()
        current.owned = Owned(["4242 still runs"])
        self.assertEqual(current.close(), ["the holder path did not close: socket moved", "4242 still runs"])

    def main(self, run, left):
        class Round:
            def __init__(self, _args):
                pass

            def run(self):
                return run()

            def close(self):
                return left
        argv = ["holder-perf.py", "--bin", "b", "--panes", "1",
                "--out", str(self.out)]
        stderr = io.StringIO()
        with mock.patch.object(self.perf, "Round", Round), mock.patch.object(self.perf.sys, "platform", "darwin"), \
                mock.patch.object(self.perf.sys, "argv", argv), \
                mock.patch.object(self.perf.holder_owned, "abort_on_signals", lambda: None), \
                contextlib.redirect_stderr(stderr), contextlib.redirect_stdout(io.StringIO()):
            try:
                return self.perf.main(), stderr.getvalue()
            finally:
                self.stderr = stderr.getvalue()

    def test_a_round_that_failed_keeps_its_failure_and_tells_what_is_left(self):
        def run():
            raise RuntimeError("the node exited")
        with self.assertRaisesRegex(RuntimeError, "the node exited"):
            self.main(run, ["4242 still runs"])
        self.assertIn(self.perf.LEFT + "4242 still runs", self.stderr)

    def test_a_round_that_measured_fails_on_what_is_left(self):
        with self.assertRaisesRegex(SystemExit, "4242 still runs"):
            self.main(lambda: {"path": "holder", "phases": {}}, ["4242 still runs"])
        self.assertFalse(self.out.exists())

    def test_a_round_that_left_nothing_writes_its_result(self):
        self.assertEqual(self.main(lambda: {"path": "holder", "phases": {}}, [])[0], 0)
        self.assertTrue(self.out.exists())


if __name__ == "__main__":
    unittest.main()
