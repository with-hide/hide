#!/usr/bin/env python3
"""Check the compiled journey controller's actual process boundary, without Herdr.

The caller builds the shared shim once and supplies it; this entrypoint never
builds Rust, starts a desktop, uses credentials or contacts a model.
"""

import argparse
import json
import os
from pathlib import Path
import sys
import unittest

from agent_live_check.processes import OwnedProcesses
from agent_journey.controller import COMMAND_SECONDS, command_answer

DRIVER = Path(__file__).resolve().with_name("agent_journey") / "controller_command.py"


class Supervisor(unittest.TestCase):
    provider = root = None

    def execute(self, name, code, *, within_ms=5000, executable=None):
        root = self.root / name
        root.mkdir(mode=0o700)
        command = {"id": 1, "argv": [executable or sys.executable, "-I", "-c", code], "env": {},
                   "response": str(root / "response.json"), "stderr": str(root / "stderr.txt"),
                   "result": str(root / "result.json")}
        path = root / "command.json"
        path.write_text(json.dumps(command), encoding="utf-8")
        completed = root / "completed"
        owner = OwnedProcesses(run_seconds=within_ms / 1000 + 30)
        try:
            child = owner.spawn([str(self.provider), "--run-child", str(within_ms), str(completed),
                                 sys.executable, "-I", str(DRIVER), str(path)], env=dict(os.environ))
            out, err = child.communicate(timeout=within_ms / 1000 + 25)
            (root / "provider.stdout").write_bytes(out)
            (root / "provider.stderr").write_bytes(err)
            owner.end(child)
        finally:
            owner.close()
        supervisor = json.loads(completed.with_suffix(".json").read_text(encoding="utf-8"))
        result = root / "result.json"
        diagnostic = {"id": 1, "exit": completed.read_text().strip(), "wait_error": None,
                      "supervisor": supervisor,
                      "driver": json.loads(result.read_text()) if result.is_file() else None}
        response = Path(command["response"])
        text = response.read_bytes() if response.is_file() else b""
        return root, supervisor, command_answer(diagnostic, text)

    def test_stderr_exit_is_distinct_from_a_supervisor_deadline(self):
        root, supervisor, answer = self.execute("stderr-exit", "import sys; sys.stderr.write('controlled refusal'); sys.exit(1)")
        self.assertEqual((supervisor["phase"], supervisor["exit"]), ("exited", 1))
        self.assertTrue(supervisor["cleanup_confirmed"])
        self.assertEqual((root / "stderr.txt").read_text(), "controlled refusal")
        self.assertEqual(answer["reason"], "unreadable_answer")

    def test_driver_spawn_error_retains_os_error(self):
        _, supervisor, answer = self.execute("driver-spawn", "", executable=str(self.root / "absent-child"))
        self.assertEqual(supervisor["phase"], "exited")
        self.assertEqual(answer["driver"]["phase"], "spawn")
        self.assertIsInstance(answer["driver"]["os_error"], int)
        self.assertFalse(answer["ok"])

    def test_deadline_cannot_accept_success_written_before_exit(self):
        root, supervisor, answer = self.execute("deadline", "import time; print('{\"ok\":true}', flush=True); time.sleep(5)", within_ms=1500)
        self.assertEqual((root / "response.json").read_text().strip(), '{"ok":true}')
        self.assertEqual(supervisor["phase"], "deadline")
        self.assertTrue(supervisor["cleanup_confirmed"])
        self.assertEqual(answer["reason"], "controller_command_failed")

    def test_inner_structured_timeout_reaches_the_caller(self):
        _, supervisor, answer = self.execute("inner-refusal", "import sys; print('{\"ok\":false,\"reason\":\"request_timeout\"}'); sys.exit(1)")
        self.assertEqual(supervisor["phase"], "exited")
        self.assertEqual(answer, {"ok": False, "reason": "request_timeout"})

    def test_native_spawn_failure_reports_its_phase(self):
        root = self.root / "native-spawn"
        root.mkdir(mode=0o700)
        completed = root / "completed"
        owner = OwnedProcesses(run_seconds=10)
        try:
            child = owner.spawn([str(self.provider), "--run-child", "500", str(completed), str(root / "absent-child")], env=dict(os.environ))
            child.communicate(timeout=5)
            owner.end(child)
        finally:
            owner.close()
        result = json.loads(completed.with_suffix(".json").read_text())
        self.assertEqual(result["phase"], "spawn")
        self.assertGreater(result["os_error"], 0)


class DeadlineComparison(Supervisor):
    def test_old_watchdog_and_protocol_budget_on_the_same_child(self):
        # A controlled child, not evidence of the historical Windows cause.
        code = "import time,sys; time.sleep(21); print('{\"ok\":false,\"reason\":\"request_timeout\"}'); sys.exit(1)"
        _, old, denied = self.execute("old-20s", code, within_ms=20000)
        self.assertEqual(old["phase"], "deadline")
        self.assertFalse(denied["ok"])
        _, current, answer = self.execute("protocol-budget", code, within_ms=COMMAND_SECONDS * 1000)
        self.assertEqual(current["phase"], "exited")
        self.assertEqual(answer, {"ok": False, "reason": "request_timeout"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider", required=True, type=Path)
    parser.add_argument("--run-dir", required=True, type=Path)
    parser.add_argument("--deadline-comparison", action="store_true")
    args = parser.parse_args()
    Supervisor.provider, Supervisor.root = args.provider.resolve(), args.run_dir.resolve()
    Supervisor.root.mkdir(mode=0o700, parents=True, exist_ok=True)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Supervisor)
    if args.deadline_comparison:
        suite.addTest(DeadlineComparison("test_old_watchdog_and_protocol_budget_on_the_same_child"))
    return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
