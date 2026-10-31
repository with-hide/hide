"""Journey starts obey the product's spawn CLI boundary for launch choices.

The expected split is owned by hided/src/agent_cli.rs: --model before -- is
the retained launch choice; arguments after -- reach only the first start.
The first start of a real CLI carries its first prompt as Hide's --brief,
which the agent's own record has to confirm (docs/delivery.md, Input an
agent did not take); the untrusted start and a stand-in's first start keep
the agent's prompt argument.
Capture the outbound command before any product process or provider runs.
"""

import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells
from agent_journey.journey import Journey

ROOT = Path(__file__).resolve().parents[2]
MODEL = "gemini-3.8-flash-medium"


class SpawnCommand(Exception):
    def __init__(self, argv):
        super().__init__("spawn command captured without execution")
        self.argv = argv


class SpawnBoundary:
    def hide(self, argv):
        raise SpawnCommand(argv)


class JourneySpawn(unittest.TestCase):
    def command(self, *, model=None, untrusted=False, brief_start=True):
        contract = json.loads((ROOT / "contracts/agent-journey.json").read_text())
        row = next(row for row in contract if row["id"] == "antigravity")
        runtime = SimpleNamespace(run=Path("run"), probe=Path("run/probe"))
        journey = Journey(runtime, None, SpawnBoundary(), cells.plan(contract, [row["id"]]), row,
                          Path("repository"), {}, None, branch="journey-antigravity", created=[],
                          sessions=Path("sessions"), route={}, composer_judge=None, model=model,
                          brief_start=brief_start)
        with self.assertRaises(SpawnCommand) as captured:
            if untrusted:
                journey.untrusted(Path("untrusted-repository"))
            else:
                journey.start()
        argv = captured.exception.argv
        self.assertEqual(argv[:2], ["agent", "spawn"])
        if "--" not in argv:
            return argv, []
        separator = argv.index("--")
        return argv[:separator], argv[separator + 1:]

    def assert_brief_only(self, outer, native):
        self.assertEqual(native, [])
        self.assertEqual(outer.count("--brief"), 1)
        self.assertRegex(outer[outer.index("--brief") + 1], r"^Reply with exactly JOURNEY-[0-9A-F]{8}$")

    def assert_prompt_only(self, native):
        self.assertEqual(len(native), 2)
        self.assertEqual(native[0], "-i")
        self.assertRegex(native[1], r"^Reply with exactly JOURNEY-[0-9A-F]{8}$")

    def test_first_start_sets_the_products_retained_model_choice(self):
        outer, native = self.command(model=MODEL)
        self.assertIn("--model", outer)
        self.assertEqual(outer[outer.index("--model") + 1], MODEL)
        self.assertEqual(outer.count("--model"), 1)
        self.assert_brief_only(outer, native)

    def test_a_stand_ins_first_start_keeps_the_agents_prompt_argument(self):
        outer, native = self.command(brief_start=False)
        self.assertNotIn("--brief", outer)
        self.assert_prompt_only(native)

    def test_untrusted_start_sets_the_products_retained_model_choice(self):
        outer, native = self.command(model=MODEL, untrusted=True)
        self.assertIn("--model", outer)
        self.assertEqual(outer[outer.index("--model") + 1], MODEL)
        self.assertEqual(outer.count("--model"), 1)
        self.assert_prompt_only(native)

    def test_starts_without_a_model_leave_the_product_default(self):
        for untrusted in (False, True):
            with self.subTest(untrusted=untrusted):
                outer, native = self.command(untrusted=untrusted)
                self.assertNotIn("--model", outer)
                if untrusted:
                    self.assert_prompt_only(native)
                else:
                    self.assert_brief_only(outer, native)


if __name__ == "__main__":
    unittest.main()
