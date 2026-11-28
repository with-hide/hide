"""Each shell structure check fails when the text it guards breaks, and the
repository as it stands passes all of them."""
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_check():
    spec = importlib.util.spec_from_file_location(
        "check_shell_structure", ROOT / "scripts/check-shell-structure.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ANALYTICS = """\
pub enum Feature { Focus, Gitlab }

impl Feature {
    const ALL: [Self; 2] = [
        Self::Focus,
        Self::Gitlab,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Gitlab => "gitlab",
        }
    }
}

pub fn measures() -> Vec<String> {
    let mut measures: Vec<String> = [
        "hide_agent_started",
        "hide_practice",
    ]
    .map(str::to_owned)
    .to_vec();
    measures.extend(
        Feature::ALL
            .iter()
            .map(|feature| format!("hide_feature_used:{}", feature.name())),
    );
    measures
}

impl PracticeChapter {
    pub(crate) fn of(id: &str) -> Option<Self> {
        Some(match id {
            "mission" => Self::Mission,
            "focus" => Self::Focus,
            _ => return None,
        })
    }
}
"""

NOTES = """\
# Notes

## Runtime Architecture

The visible tab and the keyboard focus pane are the core's. An event is pending,
then followed, and a refusal keeps the core's value. The notifier announces
once per burst, clears its latch **before** it takes the lock, and launch
creates the core once.

## Next Section

Elsewhere.

## Performance Guide

serialize_snapshot_delta, the ChangeNotifier, idle and driven measurements.
"""

FILES = {
    "web/src/PaneGrid.tsx": "const layout = layoutForTab(tab);\nif (!layout) return null;\n",
    "web/src/terminals.ts": "function parkingLot() { hidden = true; }\nexport const make = () => new Terminal();\n",
    "web/src/other.ts": "export const x = 1;\n",
    "web/src/practice/chapters/mission.ts": '{\n  id: "mission",\n}\n',
    "web/src/releases.json": json.dumps({
        "versions": [
            {"version": "1.2.10", "highlights": [{"drawing": "old"}]},
            {"version": "1.2.11", "highlights": [
                {"drawing": "a", "measure": "hide_practice"},
                {"drawing": "b", "measure": "hide_feature_used:gitlab"},
            ]},
        ]
    }),
    "hide-core/src/product_analytics.rs": ANALYTICS,
    "hided/src/core.rs": "let core = Core::create(args);\n",
    "hided/src/main.rs": "// Core::create( is described here\nfn main() {}\n",
    "AGENTS.md": NOTES,
}


class ShellStructure(unittest.TestCase):
    def setUp(self):
        self.check = load_check()
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        for relative, text in FILES.items():
            self.write(relative, text)
        # The chapter count floor and the shell scan need more than one file.
        for index in range(8):
            self.write(f"web/src/practice/chapters/c{index}.ts", f'  id: "c{index}",\n')
        self.named_chapters()

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def append(self, relative, text):
        path = self.root / relative
        path.write_text(path.read_text() + text)

    def replace(self, relative, old, new):
        path = self.root / relative
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new))

    def fails(self, check):
        problems = check(self.root)
        self.assertTrue(problems, f"{check.__name__} passed on broken text")
        return problems

    def named_chapters(self):
        # Name the eight filler chapters so the fixture starts clean.
        names = "".join(f'            "c{i}" => Self::C{i},\n' for i in range(8))
        self.replace(
            "hide-core/src/product_analytics.rs",
            '            "focus" => Self::Focus,\n',
            '            "focus" => Self::Focus,\n' + names,
        )

    def test_the_fixture_passes_every_check(self):
        for check in self.check.CHECKS:
            self.assertEqual(check(self.root), [], check.__name__)

    def test_the_repository_passes_every_check(self):
        for check in self.check.CHECKS:
            self.assertEqual(check(ROOT), [], check.__name__)

    def test_a_retired_state_name_in_the_shell_fails(self):
        self.append("web/src/other.ts", 'const s = "unseen_completion";\n')
        self.assertIn("unseen_completion", self.fails(self.check.check_no_state_name_array)[0])

    def test_a_literal_agent_state_array_in_the_shell_fails(self):
        self.append("web/src/other.ts", 'const s = ["blocked", "question"];\n')
        self.assertIn("literal agent state array", self.fails(self.check.check_no_state_name_array)[0])

    def test_an_unrelated_array_in_the_shell_passes(self):
        self.append("web/src/other.ts", 'const s = ["blocked", "done"];\n')
        self.assertEqual(self.check.check_no_state_name_array(self.root), [])

    def test_a_uniform_grid_stand_in_fails(self):
        self.append("web/src/other.ts", "const g = uniformGrid(panes);\n")
        self.assertIn("uniform pane grid", self.fails(self.check.check_no_uniform_grid_stand_in)[0])

    def test_a_pane_grid_that_draws_no_layout_fails(self):
        self.write("web/src/PaneGrid.tsx", "const g = evenSplit(panes);\n")
        self.assertIn("no longer draws only", self.fails(self.check.check_no_uniform_grid_stand_in)[0])

    def test_a_terminal_made_outside_the_parking_module_fails(self):
        self.append("web/src/other.ts", "const t = new Terminal({});\n")
        self.assertIn("outside the parking module", self.fails(self.check.check_terminals_are_retained)[0])

    def test_a_terminal_module_without_a_hidden_lot_fails(self):
        self.write("web/src/terminals.ts", "export const make = () => new Terminal();\n")
        self.assertIn("hidden lot", self.fails(self.check.check_terminals_are_retained)[0])

    def test_a_test_file_may_make_a_terminal(self):
        self.write("web/src/other.test.ts", "const t = new Terminal({});\n")
        self.assertEqual(self.check.check_terminals_are_retained(self.root), [])

    def test_a_position_derived_tab_label_fails(self):
        self.append("web/src/other.ts", "const l = fallbackIndex + 1;\n")
        self.assertIn("position-derived", self.fails(self.check.check_tab_label_has_one_owner)[0])

    def test_a_reverse_parsed_tab_label_fails(self):
        self.append("web/src/other.ts", 'if (label.startsWith("Tab ")) {}\n')
        self.assertIn("reverse-parsed", self.fails(self.check.check_tab_label_has_one_owner)[0])

    def test_notes_that_lose_a_claim_fail(self):
        for phrase in ("keyboard focus pane", "once per burst", "ChangeNotifier"):
            with self.subTest(phrase=phrase):
                self.write("AGENTS.md", NOTES)
                self.replace("AGENTS.md", phrase, "something else")
                self.assertIn(repr(phrase), "".join(self.fails(self.check.check_agent_notes)))

    def test_notes_that_keep_the_stale_figure_fail(self):
        self.append("AGENTS.md", "\nmain thread spent 47% of wall time\n")
        self.assertIn("47%", self.fails(self.check.check_agent_notes)[0])

    def test_a_second_core_creation_in_the_daemon_fails(self):
        self.write("hided/src/launch.rs", "let again = Core::create_with_product_analytics(a);\n")
        self.assertIn("one place", self.fails(self.check.check_core_is_created_once)[0])

    def test_a_core_creation_outside_its_owner_fails(self):
        self.write("hided/src/core.rs", "fn nothing() {}\n")
        self.write("hided/src/launch.rs", "let core = Core::create(a);\n")
        self.assertIn("owner-thread wrapper", self.fails(self.check.check_core_is_created_once)[0])

    def test_a_chapter_the_core_does_not_name_fails(self):
        self.write("web/src/practice/chapters/new.ts", '  id: "brand-new",\n')
        self.assertIn("brand-new", self.fails(self.check.check_practice_chapters_are_named)[0])

    def test_a_chapter_with_no_id_fails(self):
        self.write("web/src/practice/chapters/blank.ts", "export {};\n")
        self.assertIn("names no chapter id", self.fails(self.check.check_practice_chapters_are_named)[0])

    def test_a_highlight_with_no_measure_fails(self):
        self.replace("web/src/releases.json", '"measure": "hide_practice"', '"x": 1')
        self.assertIn("names no `measure`", self.fails(self.check.check_highlights_name_a_measure)[0])

    def test_a_highlight_with_an_unsent_measure_fails(self):
        self.replace("web/src/releases.json", "hide_practice", "hide_unsent")
        self.assertIn("hide_unsent", self.fails(self.check.check_highlights_name_a_measure)[0])

    def test_a_feature_measure_the_core_does_not_list_fails(self):
        self.replace("hide-core/src/product_analytics.rs", "Self::Gitlab,\n    ];", "];")
        self.replace("hide-core/src/product_analytics.rs", "[Self; 2]", "[Self; 1]")
        self.assertIn("hide_feature_used:gitlab", self.fails(self.check.check_highlights_name_a_measure)[0])


if __name__ == "__main__":
    unittest.main()
