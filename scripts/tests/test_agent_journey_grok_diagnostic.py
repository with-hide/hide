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
            program = run / "native"
            program.write_text(f"#!{sys.executable}\n"
                               "import os,sys\n"
                               "with open(os.environ['GROK_DEBUG_LOG'],'w') as stream:\n"
                               " stream.write('private log: '+os.environ['JOURNEY_FAKE_KEY']+'\\n')\n"
                               " stream.write('config model override applied model_key=journey base_url=private\\n')\n"
                               " stream.write('auth: authenticate request method=xai.api_key\\n')\n"
                               "sys.exit(7)\n")
            program.chmod(0o700)
            diagnostic = Diagnostic(runtime)
            try:
                command = diagnostic.launcher(runtime, program)
                answer = subprocess.run([str(command)], env={"PATH": os.environ["PATH"], "HOME": str(home),
                    "JOURNEY_FAKE_KEY": "must-never-appear"}, capture_output=True, timeout=5)
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
                    self.assertTrue(runtime.owner.cancelled.wait(2))
                finally:
                    result = diagnostic.close()
            self.assertEqual(result["failure"], "grok_diagnostic_unreadable_or_over_budget")
            self.assertNotIn("private diagnostic", json.dumps(result))


if __name__ == "__main__":
    unittest.main()
