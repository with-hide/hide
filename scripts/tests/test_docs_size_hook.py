"""The docs size hook refuses an edit that would leave a Markdown file of the
repository over the cap and larger than before, in each runtime's payload
shape, and lets every other edit through."""

import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
HOOK = SCRIPTS / "hooks" / "docs-size-cap.sh"
SPEC = importlib.util.spec_from_file_location("check_docs", SCRIPTS / "check-docs.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
LIMIT = CHECK.LIMIT
LISTED = sorted(CHECK.EXCEPTIONS)[0]


class DocsSizeHookTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.repo = self.tmp / "repo"
        (self.repo / "scripts" / "hooks").mkdir(parents=True)
        (self.repo / "docs").mkdir()
        shutil.copy(HOOK, self.repo / "scripts" / "hooks" / HOOK.name)
        shutil.copy(SCRIPTS / "check-docs.py", self.repo / "scripts" / "check-docs.py")
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)
        self.other = self.tmp / "other"
        self.other.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=self.other, check=True)

    def write_file(self, rel, text, root=None):
        path = (root or self.repo) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def call(self, tool, tool_input, cwd=None, env=None):
        payload = {"hook_event_name": "PreToolUse", "session_id": "s1", "cwd": str(cwd or self.repo),
                   "tool_name": tool, "tool_input": tool_input}
        return subprocess.run([str(self.repo / "scripts" / "hooks" / HOOK.name)], input=json.dumps(payload),
                              capture_output=True, text=True, env=env)

    def edit(self, path, old, new, **extra):
        return self.call("Edit", {"file_path": str(path), "old_string": old, "new_string": new, **extra})

    def test_a_write_or_an_edit_that_grows_a_file_past_the_cap_is_refused_with_the_way_out(self):
        refused = self.call("Write", {"file_path": str(self.repo / "docs/NEW.md"), "content": "x" * (LIMIT + 1)})
        self.assertEqual(refused.returncode, 2)
        for words in ("docs/NEW.md", f"{LIMIT + 1} bytes", f"{LIMIT}-byte", "in place", CHECK.TEMPLATE):
            self.assertIn(words, refused.stderr)

        path = self.write_file("docs/GUIDE.md", "x" * (LIMIT - 10) + "\nend\n")
        self.assertEqual(self.edit(path, "end", "end" + "y" * 20).returncode, 2)
        self.assertEqual(self.edit(path, "end", "the end").returncode, 0, "still within the cap")

    def test_a_shrinking_edit_of_an_oversized_file_passes(self):
        path = self.write_file("docs/GUIDE.md", "x" * LIMIT + "\nold paragraph\n")
        self.assertEqual(self.edit(path, "old paragraph", "new").returncode, 0)
        self.assertEqual(self.edit(path, "old paragraph", "new paragraph").returncode, 0, "same size passes")
        self.assertEqual(self.edit(path, "old paragraph", "a longer paragraph").returncode, 2)

    def test_multiedit_and_replace_all_are_applied_in_order(self):
        path = self.write_file("docs/GUIDE.md", "x" * (LIMIT - 100) + "\na\nb\n")
        grows = [{"old_string": "a", "new_string": "a" * 60}, {"old_string": "b", "new_string": "b" * 60}]
        self.assertEqual(self.call("MultiEdit", {"file_path": str(path), "edits": grows}).returncode, 2)
        self.assertEqual(self.call("MultiEdit", {"file_path": str(path), "edits": grows[:1]}).returncode, 0)
        self.assertEqual(self.edit(path, "x", "", replace_all=True).returncode, 0)

    def test_codex_and_opencode_patches_are_sized_from_their_hunk_lines(self):
        added = "*** Begin Patch\n*** Add File: docs/NEW.md\n" + f"+{'x' * 1000}\n" * 62 + "*** End Patch"
        refused = self.call("apply_patch", {"command": ["apply_patch", added]})
        self.assertEqual(refused.returncode, 2)
        self.assertIn("docs/NEW.md", refused.stderr)

        self.write_file("docs/GUIDE.md", "x" * (LIMIT - 10) + "\nold\n")
        grow = "*** Begin Patch\n*** Update File: GUIDE.md\n@@\n-old\n+" + "y" * 20 + "\n*** End Patch"
        shrink = "*** Begin Patch\n*** Update File: GUIDE.md\n@@\n-old\n+n\n*** End Patch"
        self.assertEqual(self.call("apply_patch", {"command": grow}, cwd=self.repo / "docs").returncode, 2)
        self.assertEqual(self.call("apply_patch", {"command": shrink}, cwd=self.repo / "docs").returncode, 0)
        self.assertEqual(self.call("Edit", {"patchText": grow}, cwd=self.repo / "docs").returncode, 2, "OpenCode's patch")
        self.assertEqual(self.call("write", {"filePath": str(self.repo / "docs/NEW.md"),
                                             "content": "x" * (LIMIT + 1)}).returncode, 2, "OpenCode's write")

    def test_an_edit_of_a_crlf_file_is_sized_with_its_own_line_endings(self):
        path = self.repo / "docs/GUIDE.md"
        path.write_bytes(("x" * 40 + "\r\n").encode() * 1490 + b"old\r\nmore\r\n")
        self.assertEqual(self.edit(path, "old\nmore", "old\n" + "y" * 3000 + "\nmore").returncode, 2)
        self.assertEqual(self.edit(path, "old\nmore", "more").returncode, 0)

    def test_a_moved_file_is_compared_with_the_file_it_moved_from(self):
        self.write_file("docs/OLD.md", "x" * (LIMIT + 100) + "\nold\n")
        move = "*** Begin Patch\n*** Update File: docs/OLD.md\n*** Move to: docs/NEW.md\n*** End Patch"
        self.assertEqual(self.call("apply_patch", {"command": move}).returncode, 0, "a rename alone does not grow it")
        grow = move.replace("*** End Patch", "@@\n-old\n+" + "y" * 20 + "\n*** End Patch")
        refused = self.call("apply_patch", {"command": grow})
        self.assertEqual(refused.returncode, 2)
        self.assertIn(f"`docs/NEW.md` would be {LIMIT + 100 + 1 + 20 + 1} bytes after this edit ({LIMIT + 100 + 5} now)",
                      refused.stderr)

    def test_listed_ignored_unsizable_and_foreign_files_pass(self):
        big = "x" * (LIMIT + 1)
        self.write_file(".gitignore", "scratch/\n")
        guide = self.write_file("docs/GUIDE.md", "x" * LIMIT + "\nold\n")
        for path in (self.repo / LISTED, self.repo / "scratch/BIG.md", self.repo / "docs/big.txt",
                     self.other / "docs/BIG.md"):
            with self.subTest(path=path):
                self.assertEqual(self.call("Write", {"file_path": str(path), "content": big}).returncode, 0)
        self.assertEqual(self.edit(guide, "not in the file", big).returncode, 0, "an edit it cannot size")

    def test_a_worktree_without_the_check_passes(self):
        (self.repo / "scripts" / "check-docs.py").unlink()
        self.assertEqual(self.call("Write", {"file_path": str(self.repo / "docs/NEW.md"),
                                             "content": "x" * (LIMIT + 1)}).returncode, 0)

    def test_a_call_that_names_no_markdown_file_does_not_start_python(self):
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        marker = self.tmp / "python-started"
        (bin_dir / "python3").write_text(f"#!/bin/sh\ntouch '{marker}'\ncat >/dev/null\nexit 0\n")
        (bin_dir / "python3").chmod(0o755)
        env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}"}
        self.assertEqual(self.call("Edit", {"file_path": str(self.repo / "src/a.rs"), "old_string": "a",
                                            "new_string": "b"}, env=env).returncode, 0)
        self.assertFalse(marker.exists())
        self.call("Edit", {"file_path": str(self.repo / "docs/a.md"), "old_string": "a", "new_string": "b"}, env=env)
        self.assertTrue(marker.exists(), "the stub stands in for python on a Markdown call")


if __name__ == "__main__":
    unittest.main()
