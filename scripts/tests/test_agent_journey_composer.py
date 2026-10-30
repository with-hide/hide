"""The composer cell: typed, erased, pasted and cleared text, each judged by
the verdict the product's composer parser gave the pane's screen. A scripted
journey stands in for the pane; no agent runs."""

import base64
import json
from pathlib import Path
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import ci, cells, composer

ROOT = Path(__file__).resolve().parents[2]


class Pane:
    """A composer that holds what was sent to it, read as the example would judge it."""

    def __init__(self, paste_survives=0, submits_lines=False):
        self.held, self.sent, self.paste_survives = "", [], paste_survives
        # A program that takes each pasted line break as Enter, as Grok 1.0.52
        # took an LF-separated paste on Windows.
        self.submits_lines, self.prompts = submits_lines, 0

    def key(self, _event, payload):
        data = base64.b64decode(payload["bytes_base64"])
        self.sent.append(data)
        if data.startswith(b"\x1b[200~"):
            lines = data[6:-6].split(b"\r")
            if self.submits_lines:
                self.prompts += len(lines) - 1
                lines = lines[-1:]
            self.held += "P" * len(b"\r".join(lines))
        elif data == composer.BACKSPACE * composer.BACKSPACES_PER_READ:
            self.held = self.held[:max(0, len(self.held) - composer.BACKSPACES_PER_READ)]
        else:
            self.held += data.decode()

    def verdict(self, _journey, _screen):
        return "not_empty" if self.held else "empty"


def journey_over(pane, agent="codex"):
    table = cells.plan(json.loads((ROOT / "contracts/agent-journey.json").read_text()), [agent])
    cells.settle(table, agent, "start", cells.PASS, "fixture")
    settled = []
    journey = SimpleNamespace(
        id=agent, table=table, record={}, pane="w1:p1", run=Path("run"),
        runtime=SimpleNamespace(owner=SimpleNamespace(cancelled=threading.Event()),
                                command=lambda *_args, **_kw: (0, "screen", "")),
        daemon=SimpleNamespace(dispatch=pane.key),
        conversation=lambda: {"events": [{"role": "user"}] * (1 + pane.prompts)},
        take_keys=lambda record: record.setdefault("terminal", ["controlling"]) and True,
        evidence=lambda name, value: name + ".json",
        settle=lambda name, verdict, evidence=None: settled.append((name, verdict)))
    return journey, settled


class Measure(unittest.TestCase):
    def run_cell(self, pane, agent="codex"):
        journey, settled = journey_over(pane, agent)
        with mock.patch.object(composer, "verdict_of", pane.verdict), \
                mock.patch.object(composer, "POLL_SECONDS", 0):
            composer.measure(journey)
        return journey, settled

    def test_typing_erasing_pasting_and_clearing_pass_and_submit_nothing(self):
        pane = Pane()
        journey, settled = self.run_cell(pane)
        self.assertEqual(settled[0][1][0], cells.PASS, settled)
        self.assertEqual([step["verdict"] for step in journey.record["composer"]["steps"]],
                         ["not_empty", "empty", "not_empty", "empty"])
        self.assertFalse(any(b"\r" in data.replace(composer.PASTE, b"") or b"\n" in data.replace(composer.PASTE, b"")
                             for data in pane.sent), "no Return reaches the agent outside the paste")

    def test_the_paste_is_bracketed_with_the_line_breaks_hides_terminal_sends(self):
        pane = Pane()
        self.run_cell(pane)
        paste = next(data for data in pane.sent if data.startswith(b"\x1b[200~"))
        self.assertTrue(paste.endswith(b"\x1b[201~"))
        self.assertEqual(paste.count(b"\r"), 11)
        self.assertNotIn(b"\n", paste)

    def test_grok_on_windows_gets_the_lines_with_its_newline_key_and_no_line_break_that_submits(self):
        # Grok on Windows joins a paste's keys only when they arrive within
        # 10 ms; a pause sent lines 01 to 04 as a prompt (hide-private#361).
        for windows, agent, step in ((True, "grok", "drafted"), (True, "codex", "pasted"),
                                     (False, "grok", "pasted")):
            with self.subTest(windows=windows, agent=agent), mock.patch.object(composer, "WINDOWS", windows):
                pane = Pane()
                journey, settled = self.run_cell(pane, agent)
                self.assertEqual(settled[0][1][0], cells.PASS, settled)
                names = [entry["name"] for entry in journey.record["composer"]["steps"]]
                self.assertEqual(names, ["typed", "erased", step, "cleared"])
                block = next(data for data in pane.sent if b"composer check line 12" in data)
                if step == "drafted":
                    self.assertNotIn(b"\x1b[200~", block)
                    self.assertEqual(block.count(b"\x1b\r"), 11)
                    self.assertEqual(block.count(b"\r"), 11, "every line break is Alt+Enter")
                    self.assertEqual(block.replace(b"\x1b\r", b"\r"), composer.BLOCK.encode())
                else:
                    self.assertEqual(block, composer.PASTE)

    def test_a_paste_that_submits_its_lines_fails_at_the_paste_step(self):
        pane = Pane(submits_lines=True)
        journey, settled = self.run_cell(pane)
        self.assertEqual(settled[0][1], (cells.FAIL, "after pasted the session holds 11 new prompt(s): "
                                                     "the keys started a turn"))
        self.assertEqual(journey.record["composer"]["steps"][-1]["name"], "pasted")
        self.assertEqual(pane.held, "", "the cell still clears what the paste left")

    def test_the_screen_decides_when_a_paste_is_gone_not_a_count_of_presses(self):
        pane = Pane()
        journey, _ = self.run_cell(pane)
        steps = {step["name"]: step for step in journey.record["composer"]["steps"]}
        self.assertGreater(steps["cleared"]["backspaces"], steps["erased"]["backspaces"])

    def test_a_slow_screen_read_leaves_a_long_erase_its_whole_wait_after_the_last_key(self):
        # Codex and agy keep the pasted lines as text, 275 characters: 35
        # reads of 8 Backspaces, which a read of a second each made 35 s.
        pane, clock = Pane(), [0.0]

        def slow(_journey, _screen):
            clock[0] += 1
            return Pane.verdict(pane, _journey, _screen)
        pane.verdict = slow
        with mock.patch.object(composer.time, "monotonic", lambda: clock[0]):
            journey, settled = self.run_cell(pane)
        self.assertEqual(settled[0][1][0], cells.PASS, settled)
        cleared = journey.record["composer"]["steps"][-1]
        self.assertGreater(cleared["seconds"], composer.STEP_SECONDS)

    def test_a_composer_that_never_empties_fails_naming_the_step_it_stopped_at(self):
        pane = Pane()
        pane.verdict = lambda _journey, _screen: "not_found"
        with mock.patch.object(composer, "STEP_SECONDS", -1):
            _, settled = self.run_cell(pane)
        status, reason = settled[0][1]
        self.assertEqual(status, cells.FAIL)
        self.assertEqual(reason, "after typed the composer read not_found, not not_empty (last of 1 reads in 0.0 s)")

    def test_a_terminal_that_never_takes_keys_fails_before_typing(self):
        pane = Pane()
        journey, settled = journey_over(pane)
        journey.take_keys = lambda record: record.setdefault("terminal", ["waiting_size"]) and False
        composer.measure(journey)
        self.assertEqual(settled[0][1][0], cells.FAIL)
        self.assertEqual(pane.sent, [])


class Declaration(unittest.TestCase):
    def test_every_started_agent_declares_the_cell_and_a_stand_in_leaves_it_unmeasured(self):
        for row in json.loads((ROOT / "contracts/agent-journey.json").read_text()):
            self.assertTrue(row["cells"]["composer"], row["id"])
        self.assertIn("composer", ci.FIXTURE_UNIMITATED)
        self.assertNotIn("composer", ci.FIXTURE_CELLS)
        self.assertEqual(cells.PREREQUISITE["composer"], "start")


class Verdict(unittest.TestCase):
    def test_the_first_step_that_read_otherwise_fails(self):
        steps = [{"name": "typed", "expected": "not_empty", "verdict": "not_empty", "reads": 1, "seconds": 0.5},
                 {"name": "erased", "expected": "empty", "verdict": "not_empty", "reads": 9, "seconds": 30.1}]
        self.assertEqual(cells.composer_verdict(steps)[0], cells.FAIL)
        steps[1]["verdict"] = "empty"
        self.assertEqual(cells.composer_verdict(steps),
                         (cells.PASS, "the composer read not_empty typed, empty erased"))


if __name__ == "__main__":
    unittest.main()
