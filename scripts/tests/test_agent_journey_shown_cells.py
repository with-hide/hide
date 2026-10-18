"""The journey cells that judge what Hide shows of an agent: its integration,
its start's Memory, its fork, its title and its usage. Synthetic snapshot
rows and answers in the wire's shapes (contracts/hided-ws.schema.json,
contracts/hide-cli.json); no agent runs."""

import json
from pathlib import Path
import re
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells, fork, kit, memory, title, usage
from agent_journey.journey import token

HOME = "/runs/journey/probe/daemon-home"


def kit_row(hook="installed", herdr="installed", hook_at=HOME + "/.codex/hooks.json", herdr_at=HOME + "/.codex"):
    return {"id": "codex", "hook": {"state": hook, "reason": None, "location": hook_at},
            "herdr": {"state": herdr, "reason": "an older version of Herdr's integration is there"
                      if herdr == "outdated" else None, "location": herdr_at}}


def rest(*agents, devices=()):
    return {"navigator": {"agents": list(agents), "devices": list(devices)}}


class Integration(unittest.TestCase):
    session = {"source": "herdr:codex", "agent": "codex", "kind": "id", "value": "s1"}

    def test_the_kit_installed_both_pieces_in_the_agents_home_and_herdr_reported_the_session(self):
        status, reason = kit.verdict(kit_row(), self.session, "codex", HOME)
        self.assertEqual(status, cells.PASS)
        self.assertIn("herdr:codex", reason)

    def test_an_outdated_or_missing_piece_fails_with_the_kits_reason(self):
        self.assertEqual(kit.verdict(kit_row(herdr="outdated"), self.session, "codex", HOME),
                         (cells.FAIL, "the kit's herdr piece is outdated: an older version of Herdr's integration is there"))
        self.assertEqual(kit.verdict(kit_row(hook="not_installed"), self.session, "codex", HOME),
                         (cells.FAIL, "the kit's hook piece is not_installed"))
        self.assertEqual(kit.verdict(None, self.session, "codex", HOME),
                         (cells.FAIL, "the kit has no row for the agent on this Mac"))

    def test_a_piece_outside_the_home_the_agent_reads_is_not_the_one_it_loaded(self):
        status, reason = kit.verdict(kit_row(herdr_at="/elsewhere/operator/.codex"), self.session, "codex", HOME)
        self.assertEqual(status, cells.FAIL)
        self.assertIn("outside the HOME the agent reads", reason)

    def test_a_session_another_emitter_reported_fails(self):
        self.assertEqual(kit.verdict(kit_row(), {**self.session, "source": "hide:hook"}, "codex", HOME)[0], cells.FAIL)
        self.assertEqual(kit.verdict(kit_row(), None, "codex", HOME)[0], cells.FAIL)

    def test_the_kit_row_is_this_macs(self):
        remote = {"id": "device-1", "kind": "remote", "state": "ready", "kit": {"agents": [kit_row(herdr="outdated")]}}
        local = {"id": "node-1", "kind": "local", "state": "ready", "kit": {"agents": [kit_row()]}}
        snapshot = rest(devices=(remote, local))
        self.assertEqual(kit.local_device(snapshot)["id"], "node-1")
        self.assertEqual(kit.kit_agent(snapshot, "codex")["herdr"]["state"], "installed")
        self.assertIsNone(kit.kit_agent(snapshot, "claude-code"))
        self.assertIsNone(kit.local_device(rest(devices=(remote,))))


class Memory(unittest.TestCase):
    def test_the_card_and_the_question_share_no_word(self):
        # Hide's prompt-time Memory picks a card by one whole word of the
        # prompt (hide-memory's matcher keeps `-` inside a word), so a shared
        # word would let the answer come from the prompt's turn, not the start.
        def words(text):
            return {word.lower() for word in re.findall(r"[\w-]+", text)}
        value = token()
        self.assertIn(value, memory.title(value))
        self.assertNotIn(value, memory.QUESTION)
        self.assertEqual(words(memory.title(value)) & words(memory.QUESTION), set())


class Fork(unittest.TestCase):
    herdr = {"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "fork-1"}
    listed = {"id": "agent-9", "pane": "w4:p1", "parent": None, "origin": "agent-2"}

    def row(self, **fields):
        return {"pane_id": "w4:p1", "session_id": "fork-1", "forked_from": {"pane_id": "w3:p2", "label": "Claude"},
                **fields}

    def test_a_root_fork_with_its_own_session_from_the_source_passes(self):
        status, reason = fork.verdict(self.row(), "source-1", self.herdr, "claude", self.listed, "agent-2")
        self.assertEqual(status, cells.PASS)
        self.assertIn("from w3:p2", reason)

    def test_the_forked_row_is_the_one_naming_the_source(self):
        source = {"pane_id": "w3:p2", "session_id": "source-1"}
        other = self.row(pane_id="w5:p1", forked_from={"pane_id": "w9:p9"})
        self.assertEqual(fork.forked_row(rest(source, other, self.row()), "w3:p2")["pane_id"], "w4:p1")
        self.assertIsNone(fork.forked_row(rest(source, other), "w3:p2"))
        self.assertEqual(fork.verdict(None, "source-1", self.herdr, "claude", self.listed, "agent-2"),
                         (cells.FAIL, "no row is shown as forked from the source"))

    def test_a_fork_that_kept_the_sources_session_or_another_session_fails(self):
        self.assertEqual(fork.verdict(self.row(session_id="source-1"), "source-1", self.herdr, "claude", self.listed,
                                      "agent-2")[0], cells.FAIL)
        status, reason = fork.verdict(self.row(), "source-1", {**self.herdr, "value": "fork-2"}, "claude",
                                      self.listed, "agent-2")
        self.assertEqual((status, reason), (cells.FAIL, "Herdr reports session fork-2 in the fork, Hide fork-1"))
        self.assertEqual(fork.verdict(self.row(), "source-1", None, "claude", self.listed, "agent-2")[0], cells.FAIL)

    def test_a_fork_registered_as_a_child_or_from_another_source_fails(self):
        child = {**self.listed, "parent": "agent-2"}
        self.assertEqual(fork.verdict(self.row(), "source-1", self.herdr, "claude", child, "agent-2")[0], cells.FAIL)
        stranger = {**self.listed, "origin": "agent-7"}
        self.assertEqual(fork.verdict(self.row(), "source-1", self.herdr, "claude", stranger, "agent-2")[0], cells.FAIL)
        self.assertEqual(fork.verdict(self.row(), "source-1", self.herdr, "claude", None, "agent-2"),
                         (cells.FAIL, "hide agent list has no registration for the fork's pane"))


class Title(unittest.TestCase):
    def test_a_title_other_than_the_agents_name_passes(self):
        self.assertEqual(title.verdict({"identity_label": "Codex"}, "Codex"),
                         (cells.FAIL, "the row still shows 'Codex', the agent's name, as its title"))
        self.assertEqual(title.verdict({"identity_label": ""}, "Codex")[0], cells.FAIL)
        self.assertEqual(title.verdict(None, "Codex"), (cells.FAIL, "the agent's row is gone"))
        status, reason = title.verdict({"identity_label": "Reply with a token", "message": None,
                                        "request": {"line": None}}, "Codex")
        self.assertEqual(status, cells.PASS)
        self.assertIn("without Hide's label sentences", reason)
        status, reason = title.verdict({"identity_label": "Reply with a token", "message": "Replying."}, "Codex")
        self.assertIn("with Hide's label sentences", reason)

    def test_a_named_title_passes_only_with_the_name_the_operator_gave(self):
        self.assertEqual(title.verdict({"identity_label": "JOURNEY-1"}, "pi", "JOURNEY-1")[0], cells.PASS)
        self.assertEqual(title.verdict({"identity_label": "Reply with a token"}, "pi", "JOURNEY-1"),
                         (cells.FAIL, "the row shows 'Reply with a token', not 'JOURNEY-1', the name the operator "
                                      "gave the session"))


def usage_section(node="node-1", agent="codex", consumption=None):
    return {"nodes": [{"node": node, "agents": [{"agent": agent, "limits": {"state": "unsupported"},
                                                  "consumption": consumption}], "history": None}]}


def reading(calls, new_tokens=40):
    return {"state": "reading", "freshness": "fresh", "observed_at_ms": 1,
            "value": {"popover": {"amount": {"calls": calls, "new_tokens": new_tokens}}}}


class Usage(unittest.TestCase):
    def test_a_reading_with_the_runs_calls_passes(self):
        cell = usage.consumption(usage_section(consumption=reading(3)), "node-1", "codex")
        self.assertEqual(usage.verdict(cell), (cells.PASS, "Hide read 3 model calls and 40 new tokens"))

    def test_no_calls_no_reading_or_no_row_fails(self):
        self.assertEqual(usage.verdict(reading(0)), (cells.FAIL, "the agent's usage reads no model call from the run's turns"))
        self.assertEqual(usage.verdict({"state": "unavailable", "cause": "unreadable", "last_checked_at_ms": None}),
                         (cells.FAIL, "the agent's usage reads unavailable (unreadable)"))
        self.assertEqual(usage.verdict({"state": "loading"}), (cells.FAIL, "the agent's usage reads loading"))
        section = usage_section(consumption=reading(3))
        self.assertIsNone(usage.consumption(section, "node-2", "codex"))
        self.assertIsNone(usage.consumption(section, "node-1", "claude"))
        self.assertEqual(usage.verdict(None)[0], cells.FAIL)

    def test_an_agent_whose_sessions_are_outside_the_daemons_home_is_not_measured(self):
        home = Path("/runs/journey/probe/daemon-home")
        self.assertIsNone(usage.unreadable(SimpleNamespace(sessions=home, runtime=SimpleNamespace(home=home))))
        reason = usage.unreadable(SimpleNamespace(sessions=Path("/elsewhere/operator"), runtime=SimpleNamespace(home=home)))
        self.assertIn("reads only its own HOME", reason)


class Prerequisites(unittest.TestCase):
    def test_what_hide_shows_of_a_conversation_waits_for_its_answer(self):
        table = cells.plan(json.loads((Path(__file__).resolve().parents[2] / "contracts/agent-journey.json").read_text()),
                           ["codex"])
        cells.settle(table, "codex", "start", cells.PASS, "fixture")
        cells.settle(table, "codex", "reply", cells.FAIL, "no token")
        for name in ("fork", "title", "usage"):
            self.assertFalse(cells.measurable(table, "codex", name))
            self.assertEqual(table["codex"][name]["reason"], "prerequisite cell failed (reply)")
        self.assertTrue(cells.measurable(table, "codex", "integration"))
        self.assertTrue(cells.measurable(table, "codex", "guidance_memory"))


if __name__ == "__main__":
    unittest.main()
