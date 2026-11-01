"""A built-in agent's screen rules change only with a higher version and a
captured screen of that agent."""
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "check_agent_screen_rules", ROOT / "scripts/check-agent-screen-rules.py"
)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def definition(version, contains="ready", agent="gemini"):
    return {
        "id": agent,
        "screen": {"version": version, "rules": [{"id": "r", "contains": [contains]}]},
    }


class ScreenRuleGate(unittest.TestCase):
    def test_unchanged_rules_pass_without_a_capture(self):
        files = {"gemini.toml": definition("1")}
        self.assertEqual(check.problems(files, files, set()), [])

    def test_changed_rules_need_a_higher_version(self):
        base = {"gemini.toml": definition("2026.06.10.1")}
        head = {"gemini.toml": definition("2026.06.10.1", contains="other")}
        [problem] = check.problems(base, head, {"gemini"})
        self.assertIn("not higher than 2026.06.10.1", problem)
        head = {"gemini.toml": definition("2026.06.10.2", contains="other")}
        self.assertEqual(check.problems(base, head, {"gemini"}), [])

    def test_versions_compare_number_by_number(self):
        base = {"gemini.toml": definition("2026.9.30")}
        head = {"gemini.toml": definition("2026.10.1", contains="other")}
        self.assertEqual(check.problems(base, head, {"gemini"}), [])
        head = {"gemini.toml": definition("2026.9.30.0", contains="other")}
        self.assertEqual(len(check.problems(base, head, {"gemini"})), 1)

    def test_changed_rules_need_a_captured_screen(self):
        base = {"gemini.toml": definition("1")}
        head = {"gemini.toml": definition("2", contains="other")}
        [problem] = check.problems(base, head, set())
        self.assertIn("screens/gemini/", problem)
        # Rules a file gains are a change too.
        base = {"omp.toml": {"id": "omp"}}
        [problem] = check.problems(base, {"omp.toml": definition("1", agent="omp")}, set())
        self.assertIn("screens/omp/", problem)

    def test_a_new_file_adds_an_agent_and_is_not_a_rule_change(self):
        self.assertEqual(check.problems({}, {"amp.toml": definition("1", agent="amp")}, set()), [])

    def test_the_repository_passes(self):
        head = {
            path.name: check.tomllib.loads(path.read_text(encoding="utf-8"))
            for path in (ROOT / check.AGENTS).glob("*.toml")
        }
        self.assertEqual(len(head), 23)
        self.assertEqual(check.problems(head, head, set()), [])


if __name__ == "__main__":
    unittest.main()
