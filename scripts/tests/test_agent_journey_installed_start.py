"""Which Windows agents the journey runs as installed, and how their
environment reaches the daemon whose panes inherit it; no agent runs."""

from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import ci  # noqa: E402
from agent_live_check.protection import ProtectionError  # noqa: E402


class InstalledStart(unittest.TestCase):
    def test_only_a_windows_agent_whose_first_input_rides_its_start_runs_as_installed(self):
        launched = {"input": ["launch_prompt", "typed"]}
        typed = {"input": ["typed"]}
        with mock.patch.object(ci, "WINDOWS", True):
            self.assertTrue(ci.runs_as_installed(launched, None))
            self.assertFalse(ci.runs_as_installed(typed, None))
            self.assertFalse(ci.runs_as_installed({}, None))
            self.assertFalse(ci.runs_as_installed(launched, Path("fixture-bin")))
        with mock.patch.object(ci, "WINDOWS", False):
            self.assertFalse(ci.runs_as_installed(launched, None))

    def test_an_agents_environment_joins_the_daemons_and_a_conflicting_value_is_refused(self):
        runtime = SimpleNamespace(env={"PATH": "controlled"},
                                  daemon_env={"PATH": "controlled", "HOME": "/h", "CLAUDE_CONFIG_DIR": "/c"})
        ci.share_with_daemon(runtime, {"PATH": "agents", "HOME": "/h", "CODEX_HOME": "/h/.codex"})
        # A second agent with the same values joins as well.
        ci.share_with_daemon(runtime, {"PATH": "agents", "HOME": "/h", "GROK_HOME": "/h/.grok"})
        self.assertEqual(runtime.daemon_env, {"PATH": "agents", "HOME": "/h", "CODEX_HOME": "/h/.codex",
                                              "GROK_HOME": "/h/.grok"})
        for name, value in (("HOME", "/other"), ("PATH", "another")):
            with self.subTest(name=name), self.assertRaises(ProtectionError) as refused:
                ci.share_with_daemon(runtime, {name: value})
            self.assertEqual(str(refused.exception), f"agent_environment_conflict_{name}")


if __name__ == "__main__":
    unittest.main()
