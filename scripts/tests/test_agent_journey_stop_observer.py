"""The private observation hook cannot feed back into the agent's turn.

Exercise the installed command and stdin boundary, not a mocked projection:
Stop evidence contains only probe metadata and is bounded and removable.
"""
import io
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import stop_observer
from agent_live_check.processes import OwnedProcesses
from agent_live_check.system import WINDOWS, home_env
from agent_live_check.runtime import clean_env


class StopObservation(unittest.TestCase):
    def test_installed_hook_emits_nothing_and_records_original_stop_sequence(self):
        with tempfile.TemporaryDirectory() as directory:
            probe = Path(directory)
            home = probe / "home"
            observer = stop_observer.Observer(home, probe, [".agent", "hooks", "observer.json"], windows=WINDOWS)
            self.addCleanup(observer.close)
            observer.arm("JOURNEY-1111AAAA", "JOURNEY-2222BBBB", 60)
            config = json.loads(observer.config.read_text())
            handler = config["hooks"]["Stop"][0]["hooks"][0]
            command = handler["command"]
            shell = ([shutil.which("pwsh") or shutil.which("powershell"), "-NoProfile", "-Command"]
                     if WINDOWS else ["/bin/sh", "-c"])
            owner = OwnedProcesses()
            try:
                for active, answer in ((False, "JOURNEY-1111AAAA"), (True, "JOURNEY-2222BBBB")):
                    payload = dict(sessionId="session", reason="end_turn", stopHookActive=active,
                                   lastAssistantMessage="private prose " + answer)
                    with tempfile.TemporaryFile() as stream:
                        stream.write(json.dumps(payload).encode())
                        stream.seek(0)
                        code, out, err = owner.run([*shell, command], env={**clean_env(private=True), **home_env(home)}, stdin=stream,
                                                   seconds=handler["timeout"])
                    self.assertEqual((code, out, err), (0, "", ""))
                rows = observer.read()
                self.assertEqual([row["active"] for row in rows], [False, True])
                self.assertEqual([row["answers"] for row in rows], [["JOURNEY-1111AAAA"], ["JOURNEY-2222BBBB"]])
                self.assertNotIn("private prose", json.dumps(rows))
                self.assertTrue(all(row["session"] == "session" for row in rows))
            finally:
                owner.close()
                observer.close()
            self.assertFalse(observer.config.exists())
            self.assertFalse((observer.directory / "armed.json").exists())

    def test_bad_payload_and_record_overflow_are_failure_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            observer = stop_observer.Observer(root / "home", root, ["hooks", "observer.json"])
            try:
                observer.arm("held", "letter", 60)
                for raw in (b"{}", b"{", b"x" * (stop_observer.INPUT_CAP + 1)):
                    stop_observer.observe(observer.directory, io.BytesIO(raw))
                self.assertTrue(all(row.get("error") for row in observer.read()))
                for _ in range(stop_observer.RECORD_CAP):
                    stop_observer.observe(observer.directory, io.BytesIO(b"{}"))
                self.assertEqual(observer.read(), [{"error": "observer_overflow"}])
                self.assertEqual(len(list(observer.directory.glob("[0-9].json"))), stop_observer.RECORD_CAP)
            finally:
                observer.close()

    def test_observer_refuses_a_home_outside_its_private_probe(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ValueError):
                stop_observer.Observer(root / "operator", root / "probe", ["hooks", "observer.json"])


if __name__ == "__main__":
    unittest.main()
