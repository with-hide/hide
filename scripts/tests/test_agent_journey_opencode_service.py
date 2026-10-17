"""The journey ends the OpenCode server its agent started, and no other: the
server outlives every terminal, so a run that misses it leaves it running."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import opencode_service

# A stand-in for `opencode serve --service`: it listens on the port it is given until it is signalled.
SERVER = """import socket, sys, time
server = socket.socket()
server.bind(("127.0.0.1", int(sys.argv[3])))
server.listen()
open(sys.argv[4], "w").close()
time.sleep(120)
"""


def run(argv, env, check, seconds):
    del check
    result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=seconds)
    return result.returncode, result.stdout, result.stderr


@unittest.skipIf(os.name == "nt", "the run's servers are told apart by their environment on Linux and macOS")
class Stop(unittest.TestCase):
    def start(self, folder, mark):
        home = folder / mark
        port = opencode_service.configure(home / opencode_service.CONFIG)
        program, ready = home / "opencode", home / "ready"
        program.write_text(SERVER)
        server = subprocess.Popen([sys.executable, str(program), "serve", "--service", str(port), str(ready)],
                                  env={**os.environ, opencode_service.MARK: mark})
        self.addCleanup(server.kill)
        for _ in range(100):
            if ready.exists():
                break
            subprocess.run(["sleep", "0.05"])
        return port, server

    def test_a_server_its_own_stop_left_running_is_signalled_and_another_stays(self):
        with tempfile.TemporaryDirectory() as scratch:
            folder = Path(scratch)
            port, server = self.start(folder, "run-home")
            other_port, other = self.start(folder, "operator")
            # An OpenCode whose `service stop` answers without stopping anything.
            launch = folder / "launch"
            launch.write_text("#!/bin/sh\nexit 0\n")
            launch.chmod(0o700)
            self.assertEqual(opencode_service.ours("run-home"), [server.pid])
            with mock.patch.object(opencode_service, "STOP_SECONDS", 1):
                record = opencode_service.stop(launch, dict(os.environ), port, "run-home", SimpleNamespace(run=run))
            self.assertEqual((record["servers"], record["stopped_by"], record["left"]), ([server.pid], "signal", []))
            self.assertFalse(opencode_service.listening(port))
            self.assertIsNone(other.poll())
            self.assertTrue(opencode_service.listening(other_port))


if __name__ == "__main__":
    unittest.main()
