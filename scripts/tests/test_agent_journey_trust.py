"""What the journey lets an agent read and finds in the operator's files: no agent runs."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey.local import allow_one_shot_references, operator_trust
from agent_journey.routes import allow_hook_records
from agent_live_check.protection import ProtectionError


class OperatorTrust(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.operator = Path(folder.name) / "home"
        self.run = Path(folder.name) / "run"
        (self.operator / ".codex").mkdir(parents=True)
        self.run.mkdir()

    def test_only_entries_for_the_runs_paths_are_named(self):
        worktree = self.run / "probe/worktrees/codex"
        (self.operator / ".codex/config.toml").write_text(
            f'[projects."/elsewhere"]\ntrust_level = "trusted"\n[projects.{json.dumps(str(worktree))}]\ntrust_level = "trusted"\n')
        (self.operator / ".claude.json").write_text(json.dumps({"projects": {
            str(worktree): {"hasTrustDialogAccepted": True}, str(self.run / "a"): {"allowedTools": []},
            "/elsewhere": {"hasTrustDialogAccepted": True}}}))
        found = operator_trust(self.operator, self.run)
        self.assertEqual(found[str(self.operator / ".codex/config.toml")], [f"projects.{json.dumps(str(worktree))}"])
        self.assertEqual(found[str(self.operator / ".claude.json")],
                         [f"projects.{json.dumps(str(worktree))}.hasTrustDialogAccepted"])

    def test_a_file_that_cannot_be_read_fails_and_a_missing_one_is_empty(self):
        self.assertEqual(operator_trust(self.operator, self.run), {})
        (self.operator / ".codex/config.toml").write_text("[projects\n")
        with self.assertRaisesRegex(ProtectionError, "operator_codex_config_unreadable"):
            operator_trust(self.operator, self.run)



@unittest.skipUnless(sys.platform == "darwin" and Path("/usr/bin/sandbox-exec").is_file(), "the write sandbox is macOS's")
class OneShotReferences(unittest.TestCase):
    def test_the_rule_opens_the_reference_files_and_nothing_else_of_the_state(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        # The real path, as the daemon writes it (macOS's /var is /private/var).
        state = Path(folder.name).resolve() / "run (1)" / "state"
        references = state / "pane-capabilities"
        references.mkdir(parents=True)
        profile = Path(folder.name) / "guard.sb"
        sandbox = type("Sandbox", (), {"profile": profile,
                                       "rules": ["(version 1)", "(allow default)",
                                                 f"(deny file-read* (subpath {json.dumps(str(state))}))"]})()
        allow_one_shot_references(sandbox, type("Runtime", (), {"state": state})())
        files = {"0123456789abcdef" * 2 + ".json": True, "0123456789abcdef" * 2 + ".claimed": False,
                 "0123456789abcde.json": False, "0123456789abcdef" * 2 + "0.json": False}
        for name in files:
            (references / name).write_text("{}")
        (state / "hided.json").write_text("{}")
        files["../hided.json"] = False
        for name, readable in files.items():
            read = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(profile), "/bin/cat", str(references / name)],
                                  capture_output=True, text=True, timeout=10)
            self.assertEqual(read.returncode == 0, readable, f"{name}: {read.stderr.strip()}")


@unittest.skipUnless(sys.platform == "darwin" and Path("/usr/bin/sandbox-exec").is_file(), "the write sandbox is macOS's")
class HookRecords(unittest.TestCase):
    def test_the_rule_opens_the_candidates_records_folder_and_nothing_else_of_the_state(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        root = Path(folder.name).resolve()
        home, state = root / "daemon-home", root / "state"
        home.mkdir()
        state.mkdir()
        # The candidate's own state folder, so the records sit beside it
        # (`hide_agent_hooks::route::records_of_state`).
        (state / "hided.json").write_text("{}")
        profile = root / "guard.sb"
        sandbox = type("Sandbox", (), {"profile": profile,
                                       "rules": ["(version 1)", "(allow default)",
                                                 f"(deny file-read* file-write* (subpath {json.dumps(str(state))}))"]})()
        allow_hook_records(sandbox, type("Runtime", (), {"home": home, "state": state})())
        records = state / "agent-hooks" / "panes"
        # A hook creates the folder on its first record, keeps its record and
        # the lock beside it, and reads both back.
        write = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(profile), "/bin/sh", "-c",
                                f"mkdir -p {json.dumps(str(records))} && printf '{{}}' > {json.dumps(str(records / 'h1_p1.json'))}"
                                f" && printf '' >> {json.dumps(str(state / 'agent-hooks/panes.lock'))}"],
                               capture_output=True, text=True, timeout=10)
        self.assertEqual(write.returncode, 0, write.stderr.strip())
        self.assertEqual((records / "h1_p1.json").read_text(), "{}")
        # The rest of this daemon's state stays denied, read and written: its
        # core's files, its mailbox and its pane credentials are not a hook's.
        for path in (state / "hided.json", state / "core-state.json", state / "pane-capabilities/own.json"):
            touched = subprocess.run(["/usr/bin/sandbox-exec", "-f", str(profile), "/bin/sh", "-c",
                                      f"cat {json.dumps(str(path))}; printf x > {json.dumps(str(path))}"],
                                     capture_output=True, text=True, timeout=10)
            self.assertNotEqual(touched.returncode, 0, f"{path.name} was reachable")


if __name__ == "__main__":
    unittest.main()
