"""`scripts/check-agent-folders.sh` fails where production Rust names an
agent's account folder outside its definition and the agent's own modules
(PRD agent-profiles D-13), and passes the places it allows."""
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-agent-folders.sh"


def check(files: dict) -> subprocess.CompletedProcess:
    with tempfile.TemporaryDirectory() as root:
        for relative, text in files.items():
            path = pathlib.Path(root, relative)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return subprocess.run(
            ["bash", str(SCRIPT), root], capture_output=True, text=True, check=False
        )


NAMES_CODEX = 'pub fn codex(home: &Path) -> PathBuf {\n    home.join(".codex")\n}\n'


class CheckAgentFolders(unittest.TestCase):
    def test_a_folder_named_in_a_shared_module_fails_with_its_line(self):
        for literal in ['".codex"', '".claude/projects"', '".pi/agent"', '".omp/agent/sessions"',
                        '".grok"', '".gemini/config"', '".local/share/opencode"', '"opencode.db"',
                        '".claude.json"', 'r".config/opencode"']:
            result = check({"crate/src/lib.rs": f"fn f() {{}}\nconst X: &str = {literal};\n"})
            self.assertEqual(result.returncode, 1, literal)
            self.assertIn("crate/src/lib.rs:2: names an agent's folder", result.stderr)

    def test_a_folder_built_from_several_literals_fails(self):
        for built in ['home.join(".config").join("opencode")',
                      'home.join(".gemini")\n        .join("config")',
                      'home.join(".local").join("share").join("opencode")',
                      'home.join(".claude").join("projects")',
                      '[".config", "opencode"].iter().collect::<PathBuf>()']:
            result = check({"crate/src/lib.rs": f"fn f(home: &Path) -> PathBuf {{\n    {built}\n}}\n"})
            self.assertEqual(result.returncode, 1, built)
            self.assertIn("crate/src/lib.rs:2: names an agent's folder", result.stderr)

    def test_only_the_root_agents_folder_is_left_out(self):
        result = check({"crate/src/agents/config.rs": NAMES_CODEX})
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("crate/src/agents/config.rs:2", result.stderr)
        result = check({"agents/runs/scratch/src/lib.rs": NAMES_CODEX})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_joined_names_that_are_no_agents_folder_pass(self):
        result = check({"crate/src/lib.rs": 'fn f(home: &Path) -> PathBuf {\n'
                        '    home.join(".config").join("hide").join("opencode.json")\n}\n'
                        'const NAMES: [&str; 2] = [".gemini", "settings"];\n'})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_an_agent_module_a_comment_and_test_code_may_name_it(self):
        brace = 'const OPEN: &str = "{";\n'
        result = check({
            "crate/src/codex.rs": NAMES_CODEX,
            "crate/src/pi_extension.rs": 'const P: &str = ".pi/agent";\n',
            "crate/src/opencode/mod.rs": 'const D: &str = ".local/share/opencode";\n',
            "crate/src/lib.rs": brace + "// ~/.codex is Codex's\nfn f() {}\n"
            "#[cfg(test)]\nmod tests {\n    const C: &str = \".claude\";\n}\n",
            "crate/src/fixtures_tests.rs": NAMES_CODEX,
            "crate/tests/it/main.rs": NAMES_CODEX,
            "crate/target/debug/build/out.rs": NAMES_CODEX,
        })
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_retired_agents_folder_and_a_word_alike_pass(self):
        result = check({"crate/src/lib.rs": 'const G: &str = ".gemini";\nconst C: &str = ".cursor";\n'
                        'const K: &str = "codex";\nconst M: &str = "$HOME/.codex is used";\n'})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_the_checkout_passes(self):
        result = subprocess.run(["bash", str(SCRIPT)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
