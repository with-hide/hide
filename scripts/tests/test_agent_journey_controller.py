"""Exercise the controller driver through its real command-file/process boundary.

Only Python children run here; the shared C supervisor owns their process tree
in the journey and has its own platform-boundary checks.
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey.controller import command_answer

DRIVER = Path(__file__).resolve().parents[1] / "agent_journey/controller_command.py"


class ControllerCommand(unittest.TestCase):
    def execute(self, code, *, executable=None):
        with tempfile.TemporaryDirectory(prefix="journey-controller-") as folder:
            root = Path(folder)
            command = {"id": 7, "argv": [executable or sys.executable, "-I", "-c", code],
                       "env": {}, "response": str(root / "response.json"),
                       "stderr": str(root / "stderr.txt"), "result": str(root / "result.json")}
            path = root / "command.json"
            path.write_text(json.dumps(command), encoding="utf-8")
            child = subprocess.run([sys.executable, "-I", str(DRIVER), str(path)],
                                   capture_output=True, timeout=5)
            result = json.loads((root / "result.json").read_text(encoding="utf-8"))
            return child.returncode, result, (root / "response.json").read_bytes(), (root / "stderr.txt").read_bytes()

    def test_stderr_only_exit_keeps_the_child_outcome(self):
        status, result, out, err = self.execute("import sys; sys.stderr.write('controlled refusal'); sys.exit(1)")
        self.assertEqual(status, 1)
        self.assertEqual((result["id"], result["phase"], result["exit"]), (7, "exited", 1))
        self.assertEqual(out, b"")
        self.assertEqual(err, b"controlled refusal")

    def test_nonzero_structured_refusal_keeps_its_answer(self):
        status, result, out, _ = self.execute("import sys; print('{\"ok\":false,\"reason\":\"request_timeout\"}'); sys.exit(1)")
        self.assertEqual((status, result["phase"]), (1, "exited"))
        self.assertEqual(json.loads(out), {"ok": False, "reason": "request_timeout"})

    def test_missing_executable_records_the_os_error(self):
        status, result, out, _ = self.execute("", executable=str(DRIVER.parent / "missing-controller-child"))
        self.assertEqual(status, 1)
        self.assertEqual((result["id"], result["phase"]), (7, "spawn"))
        self.assertIsInstance(result["os_error"], int)
        self.assertEqual(out, b"")

    def test_output_overflow_is_bounded_and_cannot_be_success(self):
        status, result, out, err = self.execute("import sys; sys.stdout.write('x' * (2 * 1024 * 1024)); sys.stderr.write('y' * (2 * 1024 * 1024))")
        self.assertEqual(status, 1)
        self.assertEqual(result["phase"], "output_limit")
        self.assertLessEqual(len(out), 1024 * 1024)
        self.assertLessEqual(len(err), 1024 * 1024)


class ControllerAnswer(unittest.TestCase):
    def diagnostic(self):
        return {"id": 7, "exit": "1", "wait_error": None, "stdout": "", "stderr": "controlled refusal",
                "supervisor": {"id": 7, "exit": 1, "phase": "exited", "cleanup_confirmed": True},
                "driver": {"id": 7, "exit": 1, "phase": "exited"}}

    def test_nonzero_structured_refusal_is_not_replaced(self):
        self.assertEqual(command_answer(self.diagnostic(), '{"ok":false,"reason":"request_timeout"}'),
                         {"ok": False, "reason": "request_timeout"})

    def test_empty_output_retains_stderr_and_exit(self):
        answer = command_answer(self.diagnostic(), "")
        self.assertEqual((answer["ok"], answer["reason"], answer["exit"], answer["stderr"]),
                         (False, "unreadable_answer", "1", "controlled refusal"))

    def test_unsettled_or_stale_outcome_never_accepts_success_json(self):
        for change in ("deadline", "cleanup", "missing", "stale", "driver", "status"):
            with self.subTest(change=change):
                record = self.diagnostic()
                if change == "deadline":
                    record["supervisor"]["phase"] = "deadline"
                elif change == "cleanup":
                    record["supervisor"]["cleanup_confirmed"] = False
                elif change == "missing":
                    record["supervisor"] = None
                elif change == "stale":
                    record["driver"]["id"] = 6
                elif change == "driver":
                    record["driver"]["phase"] = "output_limit"
                else:
                    record["driver"]["exit"] = 0
                self.assertFalse(command_answer(record, '{"ok":true}')["ok"])

    def test_nonzero_exit_cannot_claim_success(self):
        self.assertFalse(command_answer(self.diagnostic(), '{"ok":true}')["ok"])

    def test_invalid_utf8_cannot_be_accepted_as_json(self):
        record = self.diagnostic()
        record["exit"] = "0"
        record["driver"]["exit"] = record["supervisor"]["exit"] = 0
        self.assertFalse(command_answer(record, b'{"ok":true,"value":"\xff"}')["ok"])


if __name__ == "__main__":
    unittest.main()
