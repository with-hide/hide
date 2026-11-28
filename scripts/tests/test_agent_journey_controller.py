"""Exercise the controller driver through its real command-file/process boundary.

Only Python children run here; the shared C supervisor owns their process tree
in the journey and has its own platform-boundary checks.
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey.controller import Controller, ControllerUnavailable, command_answer
from agent_live_check.processes import ProcessError

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
        self.assertEqual(result["streams"], {"stdout": "ended", "stderr": "ended"})
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


class PaneRead(unittest.TestCase):
    def controller(self, value, code=0):
        controller = Controller.__new__(Controller)
        controller.pane = "controller-pane"
        controller.runtime = SimpleNamespace(
            hide=Path("private/bin/hide"), daemon_env={"HOME": "private/home"},
            pane_credentials={controller.pane: Path("private/reference.json")},
            owner=SimpleNamespace(run=Mock(return_value=(code, json.dumps(value), ""))),
        )
        return controller

    def test_visible_screen_reaches_the_judge_without_losing_style_or_rows(self):
        screen = "\x1b[31mred\x1b[0m\r\n  next"
        controller = self.controller({"ok": True, "value": {"text": screen, "truncated": False}})
        self.assertEqual(controller.read_pane("child", visible=True), screen)
        args, kwargs = controller.runtime.owner.run.call_args
        self.assertEqual(args[0], [str(controller.runtime.hide), "pane", "read", "child",
                                  "--source", "visible", "--format", "ansi", "--json"])
        self.assertEqual(kwargs["env"]["HIDE_CAP_REF"], str(Path("private/reference.json")))

    def test_a_truncated_or_refused_screen_cannot_be_judged_as_empty(self):
        for answer, code in [({"ok": True, "value": {"text": "", "truncated": True}}, 0),
                             ({"ok": False, "reason": "pane_unavailable"}, 1),
                             ({"ok": True, "value": {}}, 0)]:
            with self.subTest(answer=answer), self.assertRaises(ProcessError):
                self.controller(answer, code).read_pane("child", visible=True)


class ControllerAnswer(unittest.TestCase):
    def diagnostic(self):
        return {"id": 7, "exit": "1", "wait_error": None, "stdout_bytes": 0, "stderr_bytes": 18,
                "supervisor": {"id": 7, "exit": 1, "phase": "exited", "cleanup_confirmed": True},
                "driver": {"id": 7, "exit": 1, "phase": "exited"}}

    def test_nonzero_structured_refusal_is_not_replaced(self):
        self.assertEqual(command_answer(self.diagnostic(), '{"ok":false,"reason":"request_timeout"}'),
                         {"ok": False, "reason": "request_timeout"})

    def test_empty_output_retains_stderr_size_and_exit(self):
        answer = command_answer(self.diagnostic(), "")
        self.assertEqual((answer["ok"], answer["reason"], answer["exit"], answer["stderr_bytes"]),
                         (False, "unreadable_answer", "1", 18))

    def test_unsettled_or_stale_outcome_never_accepts_success_json(self):
        # Unsettled only while no correlated supervisor outcome confirms every
        # process of the command ended; a settled run that is not whole fails.
        for change, reason in (("deadline", "controller_command_failed"),
                               ("cleanup", "controller_command_unsettled"),
                               ("missing", "controller_command_unsettled"),
                               ("stale supervisor", "controller_command_unsettled"),
                               ("no completion", "controller_command_unsettled"),
                               ("stale driver", "controller_command_failed"),
                               ("driver", "controller_command_failed"),
                               ("capture", "controller_command_failed"),
                               ("status", "controller_command_failed")):
            with self.subTest(change=change):
                record = self.diagnostic()
                if change == "deadline":
                    record["supervisor"]["phase"] = "deadline"
                elif change == "cleanup":
                    record["supervisor"]["cleanup_confirmed"] = False
                elif change == "missing":
                    record["supervisor"] = None
                elif change == "stale supervisor":
                    record["supervisor"]["id"] = 6
                elif change == "no completion":
                    record["wait_error"] = "scene_timeout"
                elif change == "stale driver":
                    record["driver"]["id"] = 6
                elif change == "driver":
                    record["driver"]["phase"] = "output_limit"
                elif change == "capture":
                    record["driver"]["phase"] = "capture_incomplete"
                else:
                    record["driver"]["exit"] = 0
                answer = command_answer(record, '{"ok":true}')
                self.assertEqual((answer["ok"], answer["reason"]), (False, reason))

    def test_nonzero_exit_cannot_claim_success(self):
        self.assertFalse(command_answer(self.diagnostic(), '{"ok":true}')["ok"])

    def test_invalid_utf8_cannot_be_accepted_as_json(self):
        record = self.diagnostic()
        record["exit"] = "0"
        record["driver"]["exit"] = record["supervisor"]["exit"] = 0
        self.assertFalse(command_answer(record, b'{"ok":true,"value":"\xff"}')["ok"])


class CommandSequence(unittest.TestCase):
    """What the controller lets run after a failed command, from the files
    the supervisor and the driver publish for it (issue 3166)."""

    SETTLED = {"exit": 1, "phase": "exited", "os_error": 0, "cleanup_confirmed": True, "cleanup_error": 0}
    ORDINARY = ({"exit": 0, "phase": "exited", "os_error": 0, "cleanup_confirmed": True, "cleanup_error": 0},
                {"phase": "exited", "exit": 0, "os_error": None, "child_ms": 30,
                 "streams": {"stdout": "ended", "stderr": "ended"}}, b'{"ok":true,"value":2}')

    def setUp(self):
        folder = tempfile.TemporaryDirectory(prefix="journey-controller-")
        self.addCleanup(folder.cleanup)
        root = Path(folder.name)
        self.outcomes = []
        runtime = SimpleNamespace(probe=root, run=root, home=root, state=root / "state", env={"PATH": ""},
                                  hide=Path("private/bin/hide"), send_text=self.run_command,
                                  wait=lambda predicate, _seconds: predicate() or self.fail("no completion"))
        self.controller = Controller(runtime)
        self.controller.pane = "controller-pane"

    def run_command(self, _pane, _text):
        # The pane's `!`: the supervisor publishes its outcome and status last.
        command = json.loads(self.controller.command.read_text(encoding="utf-8"))
        supervisor, driver, response = self.outcomes.pop(0)
        Path(command["response"]).write_bytes(response)
        Path(command["result"]).write_text(json.dumps({"id": command["id"], **driver}), encoding="utf-8")
        self.controller.outcome.write_text(json.dumps({"id": command["id"], **supervisor}), encoding="utf-8")
        self.controller.completed.write_text(f"{supervisor['exit']}\n", encoding="utf-8")

    def test_a_capture_failure_whose_processes_ended_fails_alone(self):
        held = {"phase": "capture_incomplete", "exit": 1, "os_error": None, "child_ms": 40,
                "streams": {"stdout": "open", "stderr": "ended"}}
        self.outcomes = [(self.SETTLED, held, b'{"ok":true,"value":1}'), self.ORDINARY]
        with self.assertRaises(ProcessError) as failed:
            self.controller.hide(["letter", "show", "letter-7"])
        self.assertNotIsInstance(failed.exception, ControllerUnavailable)
        message = str(failed.exception)
        self.assertTrue(message.startswith("controller_command_failed: command 1 (letter show)"), message)
        self.assertIn("stdout open", message)
        self.assertEqual(self.controller.hide(["letter", "show", "letter-7"]), {"ok": True, "value": 2})

    def test_unconfirmed_cleanup_refuses_every_later_command_naming_it(self):
        unconfirmed = {**self.SETTLED, "cleanup_confirmed": False, "cleanup_error": 60}
        self.outcomes = [(unconfirmed, self.ORDINARY[1] | {"exit": 1}, b""), self.ORDINARY]
        with self.assertRaises(ProcessError) as failed:
            self.controller.hide(["agent", "spawn", "journey-claude-code"])
        self.assertTrue(str(failed.exception).startswith("controller_command_unsettled: command 1 (agent spawn)"))
        with self.assertRaises(ControllerUnavailable) as refused:
            self.controller.hide(["letter", "show", "letter-7"])
        self.assertTrue(str(refused.exception).startswith(
            "controller_previous_command_unsettled: command 1 (agent spawn)"), str(refused.exception))
        self.assertIn("cleanup unconfirmed", str(refused.exception))
        # The refused command never reached the pane.
        self.assertEqual(self.outcomes, [self.ORDINARY])


if __name__ == "__main__":
    unittest.main()
