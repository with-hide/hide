"""The design guide hook refuses a session's first screen or token edit until
the session has read docs/DESIGN_GUIDE.md, in each runtime's payload shape."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HOOK = Path(__file__).resolve().parents[1] / "hooks" / "design-guide-first.sh"


class DesignGuideHookTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        (self.repo / "scripts" / "hooks").mkdir(parents=True)
        (self.repo / "web" / "src").mkdir(parents=True)
        shutil.copy(HOOK, self.repo / "scripts" / "hooks" / HOOK.name)
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)
        self.other = self.tmp / "other"
        (self.other / "web" / "src").mkdir(parents=True)
        subprocess.run(["git", "init", "-q"], cwd=self.other, check=True)

    def call(self, tool, tool_input, session="s1", cwd=None):
        payload = {"hook_event_name": "PreToolUse", "session_id": session, "cwd": str(cwd or self.repo),
                   "tool_name": tool, "tool_input": tool_input}
        return subprocess.run([str(self.repo / "scripts" / "hooks" / HOOK.name)], input=json.dumps(payload),
                              capture_output=True, text=True)

    def edit(self, path, session="s1"):
        return self.call("Edit", {"file_path": str(path), "old_string": "a", "new_string": "b"}, session)

    def test_first_screen_edit_is_refused_until_the_guide_is_read(self):
        refused = self.edit(self.repo / "web/src/Sidebar.tsx")
        self.assertEqual(refused.returncode, 2)
        self.assertIn("Read docs/DESIGN_GUIDE.md first", refused.stderr)
        self.assertIn("web/src/Sidebar.tsx", refused.stderr)
        self.assertEqual(self.edit(self.repo / "web/src/Sidebar.tsx").returncode, 2, "a retry alone does not pass")

        self.assertEqual(self.call("Read", {"file_path": str(self.repo / "docs/DESIGN_GUIDE.md")}).returncode, 0)
        self.assertEqual(self.edit(self.repo / "web/src/Sidebar.tsx").returncode, 0)
        self.assertEqual(self.edit(self.repo / "design/tokens.json", session="s2").returncode, 2, "the record is per session")

    def test_a_shell_read_counts_and_codex_patches_are_judged_by_their_file_lines(self):
        patch = "*** Begin Patch\n*** Update File: src/Overview.tsx\n@@\n-a\n+b\n*** End Patch"
        refused = self.call("apply_patch", {"command": patch}, cwd=self.repo / "web")
        self.assertEqual(refused.returncode, 2)
        self.assertIn("web/src/Overview.tsx", refused.stderr)
        self.assertEqual(self.call("Bash", {"command": "sed -n 1,200p docs/DESIGN_GUIDE.md"}).returncode, 0)
        self.assertEqual(self.call("apply_patch", {"command": patch}, cwd=self.repo / "web").returncode, 0)

    def test_other_files_tests_and_other_repositories_pass(self):
        for path in (self.repo / "web/src/Sidebar.test.tsx", self.repo / "web/src/store.ts",
                     self.repo / "docs/UI_BEHAVIOR.md", self.other / "web/src/App.tsx"):
            with self.subTest(path=path):
                self.assertEqual(self.edit(path).returncode, 0)
        new_file = self.call("write", {"filePath": str(self.repo / "web/src/new/Panel.tsx"), "content": "x"})
        self.assertEqual(new_file.returncode, 2, "a new file in a new folder is a screen too (OpenCode payload)")


if __name__ == "__main__":
    unittest.main()
