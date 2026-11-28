"""Journey starts obey the product's spawn CLI boundary for launch choices.

The expected split is owned by hided/src/agent_cli.rs: --model before -- is
the retained launch choice; arguments after -- reach only the first start.
The first start of a real CLI carries its first prompt as Hide's --brief,
a markdown brief of many lines like a real spawn's, which the agent's own
record has to confirm (docs/delivery.md, Input an
agent did not take); a stand-in's first start keeps the agent's prompt
argument, and so does the untrusted start of an agent Hide launches a prompt
to, while that of an agent Hide only types to carries it as --brief.
Capture the outbound command before any product process or provider runs.
"""

import json
from pathlib import Path
import re
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
    def command(self, *, model=None, untrusted=False, brief_start=True, agent="antigravity"):
        contract = json.loads((ROOT / "contracts/agent-journey.json").read_text())
        row = next(row for row in contract if row["id"] == agent)
        runtime = SimpleNamespace(run=Path("run"), probe=Path("run/probe"))
        journey = Journey(runtime, None, SpawnBoundary(), cells.plan(contract, [row["id"]]), row,
                          Path("repository"), {}, None, branch="journey-" + agent, created=[],
                          sessions=Path("sessions"), route={}, composer_judge=None, failure_logs=None, model=model,
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
        # A real spawn's shape: Claude Code records a paste of four lines or
        # more, or of 800 characters or more, wrapped as pasted content, and
        # the record has to confirm that brief too.
        brief = outer[outer.index("--brief") + 1]
        self.assertGreaterEqual(len(brief), 800)
        self.assertGreaterEqual(len(brief.strip().splitlines()), 4)
        self.assertIn("`", brief)
        self.assertRegex(brief.strip().splitlines()[-1], r"^Reply with exactly JOURNEY-[0-9A-F]{8}$")
        self.assertEqual(len(re.findall(r"JOURNEY-[0-9A-F]{8}", brief)), 1)

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

    def test_an_untrusted_start_types_the_prompt_of_an_agent_hide_never_launches_one_to(self):
        # OpenCode 2.4.2 started cold sent its `--prompt` before its
        # configuration named the model (hand run 46094104848); Hide types
        # OpenCode's first input, so the journey's start does too.
        for agent in ("opencode", "pi"):
            with self.subTest(agent=agent):
                outer, native = self.command(untrusted=True, agent=agent)
                self.assertEqual(native, [])
                self.assertEqual(outer.count("--brief"), 1)
                self.assertRegex(outer[outer.index("--brief") + 1], r"^Reply with exactly JOURNEY-[0-9A-F]{8}$")
        # A stand-in has no ladder of its own and keeps the prompt argument.
        outer, native = self.command(untrusted=True, agent="opencode", brief_start=False)
        self.assertNotIn("--brief", outer)
        self.assertEqual(len(native), 1)
        self.assertRegex(native[0], r"^--prompt=Reply with exactly JOURNEY-[0-9A-F]{8}$")

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
