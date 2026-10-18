"""The diagnostic runs the native command and retains no original log or key."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey.grok_diagnostic import Diagnostic


@unittest.skipUnless(sys.platform == "darwin", "the diagnostic is macOS fake CI only")
class AuthenticationDiagnostic(unittest.TestCase):
    def test_launch_observes_the_route_and_native_choice_without_retaining_raw_data(self):
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory).resolve()
            home, probe, binaries = run / "home", run / "probe", run / "bin"
            for folder in (home / ".grok", probe, binaries):
                folder.mkdir(parents=True)
            (home / ".grok/config.toml").write_text('[models]\ndefault="journey"\n')
            runtime = SimpleNamespace(run=run, probe=probe, bin=binaries,
                                      owner=SimpleNamespace(cancelled=threading.Event()))
            # Existing interpreters read the fixture as data: no freshly
            # written executable waits in macOS's first-execution queue.
            native = ("import os,sys\n"
                      "with open(os.environ['GROK_DEBUG_LOG'],'w') as stream:\n"
                      " stream.write('private log: '+os.environ['JOURNEY_FAKE_KEY']+'\\n')\n"
                      " stream.write('config model override applied model_key=journey base_url=private\\n')\n"
                      " stream.write('auth: authenticate request method=xai.api_key\\n')\n"
                      "sys.exit(7)\n")
            diagnostic = Diagnostic(runtime)
            try:
                command = diagnostic.launcher(runtime, Path(sys.executable))
                # Process exit is the completion event. This test asserts
                # no latency; the suite/job owns its overall hang guard.
                answer = subprocess.run(["/bin/sh", str(command), "-c", native],
                    env={"PATH": os.environ["PATH"], "HOME": str(home),
                         "JOURNEY_FAKE_KEY": "must-never-appear"}, capture_output=True)
                self.assertEqual(answer.returncode, 7, answer.stderr)
            finally:
                result = diagnostic.close()
            self.assertEqual(result["authentication_methods"], ["xai.api_key"])
            self.assertTrue(result["config_model_seen"])
            self.assertEqual(result["routes"], [{"key_name": "JOURNEY_FAKE_KEY", "key_present": True,
                "home": "home", "config": "home/.grok/config.toml", "config_exists": True}])
            self.assertNotIn("must-never-appear", json.dumps(result))
            self.assertFalse(diagnostic.path.exists())
            self.assertFalse(runtime.owner.cancelled.is_set())

    def test_a_crossed_stream_budget_cancels_the_run(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = SimpleNamespace(probe=Path(directory), owner=SimpleNamespace(cancelled=threading.Event()))
            with patch("agent_journey.grok_diagnostic.MAX_BYTES", 8):
                diagnostic = Diagnostic(runtime)
                try:
                    with open(diagnostic.path, "wb") as stream:
                        stream.write(b"a private diagnostic line\n")
                    # Crossing the byte cap ends the reader. Wait for that
                    # event, not an assumption about scheduler latency.
                    diagnostic.thread.join()
                    self.assertTrue(runtime.owner.cancelled.is_set())
                finally:
                    result = diagnostic.close()
            self.assertEqual(result["failure"], "grok_diagnostic_unreadable_or_over_budget")
            self.assertNotIn("private diagnostic", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
