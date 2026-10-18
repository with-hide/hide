"""The journey table's verdicts and exit codes, from synthetic answers; no agent runs."""

import datetime
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import cells
from agent_journey.daemon import Daemon
from agent_live_check.processes import ProcessError
from agent_journey.local import (codex_login_refusal, grok_login_refusal, pi_login_refusal,
                                 trust_codex, trust_grok)

CHECKOUT = Path(__file__).resolve().parents[2]
HOUR = 3600


def contract():
    return json.loads((CHECKOUT / "contracts/agent-journey.json").read_text())


def spawned(**view):
    value = {"id": "journey-codex", "pane": "w1:p2", "registered": True, "runtime": "running",
             "connection": "connected", "session": "native-1", **view}
    return {"ok": True, "value": value}


def measured_table(*passed):
    """A one-agent table whose built cells are open, with `passed` settled pass."""
    table = cells.plan(contract(), ["codex"])
    for name in passed:
        cells.settle(table, "codex", name, cells.PASS, "fixture")
    return table


class Plan(unittest.TestCase):
    def test_every_adapter_has_every_cell_and_undeclared_cells_are_na(self):
        rows = contract()
        table = cells.plan(rows, ["codex"])
        self.assertEqual(set(table), {row["id"] for row in rows})
        for row in rows:
            self.assertEqual(tuple(table[row["id"]]), cells.CELLS)
            for name in cells.CELLS:
                if row["id"] not in cells.EXCLUDED_AGENTS and not row["cells"][name]:
                    self.assertEqual(table[row["id"]][name]["status"], cells.NA)

    def test_cursor_is_excluded_in_every_cell_and_counts_nowhere(self):
        table = cells.plan(contract(), ["codex"])
        self.assertTrue(all(cell == {"status": cells.EXCLUDED, "reason": "operator decision 2026-10-17", "evidence": None}
                            for cell in table["cursor"].values()))
        passed = {name: cells.cell(cells.PASS, "fixture") for name in cells.CELLS}
        self.assertEqual(cells.exit_code({"codex": passed, "cursor": table["cursor"]}, ["codex", "cursor"],
                                         setup_failed=False), 0)

    def test_unbuilt_agents_and_unselected_agents_are_not_measured_with_the_reason(self):
        codex = next(row for row in contract() if row["id"] == "codex")
        unbuilt = {**codex, "id": "an-agent-not-built-yet"}
        table = cells.plan([*contract(), unbuilt], ["codex", unbuilt["id"]])
        for name in cells.CELLS:
            self.assertEqual(table[unbuilt["id"]][name], cells.cell(cells.NOT_MEASURED, cells.NOT_BUILT))
            self.assertEqual(table["codex"][name]["status"], cells.PENDING)
        self.assertEqual(table["claude-code"]["start"]["reason"], "not selected (--agents)")


class Propagation(unittest.TestCase):
    def test_a_failed_prerequisite_settles_its_dependents_as_not_measured_naming_it(self):
        table = measured_table()
        cells.settle(table, "codex", "start", cells.FAIL, "spawn refused")
        self.assertFalse(cells.measurable(table, "codex", "reply"))
        self.assertEqual(table["codex"]["reply"]["reason"], "prerequisite cell failed (start)")
        cells.close(table, interrupted=False)
        self.assertEqual(table["codex"]["resume"]["reason"], "prerequisite cell not measured (sleep_wake)")
        self.assertEqual(cells.exit_code(table, ["codex"], setup_failed=False), 1)

    def test_a_cell_two_steps_after_a_failed_start_reads_not_measured(self):
        # A failed start leaves its reply open; the fork, which needs the reply, settles both.
        table = measured_table()
        cells.settle(table, "codex", "start", cells.FAIL, "spawn refused")
        self.assertFalse(cells.measurable(table, "codex", "fork"))
        self.assertEqual(table["codex"]["reply"]["reason"], "prerequisite cell failed (start)")
        self.assertEqual(table["codex"]["fork"]["reason"], "prerequisite cell not measured (reply)")

    def test_a_settled_cell_is_never_overwritten(self):
        table = measured_table("start")
        self.assertFalse(cells.settle(table, "codex", "start", cells.FAIL, "late"))
        self.assertEqual(table["codex"]["start"]["status"], cells.PASS)

    def test_an_interrupted_run_says_so_and_never_passes_an_open_cell(self):
        table = measured_table("start")
        cells.close(table, interrupted=True)
        self.assertEqual(table["codex"]["reply"]["reason"], cells.INTERRUPTED)
        self.assertEqual(cells.exit_code(table, ["codex"], setup_failed=False), 3)


class ExitCodes(unittest.TestCase):
    def test_codes(self):
        table = measured_table(*cells.BUILT_CELLS)
        self.assertEqual(cells.exit_code(table, ["codex"], setup_failed=True), 2)
        self.assertEqual(cells.exit_code(table, ["codex"], setup_failed=False), 0)
        # One cell not measured, the others passed.
        table["codex"]["usage"] = cells.cell(cells.NOT_MEASURED, "the agent writes its sessions elsewhere")
        self.assertEqual(cells.exit_code(table, ["codex"], setup_failed=False), 3)
        open_table = measured_table()
        with self.assertRaises(ValueError):
            cells.exit_code(open_table, ["codex"], setup_failed=False)
        cells.settle(open_table, "codex", "start", cells.NOT_MEASURED, "login near refresh")
        cells.close(open_table, interrupted=False)
        self.assertEqual(cells.exit_code(open_table, ["codex"], setup_failed=False), 3)


class Verdicts(unittest.TestCase):
    def test_spawn_needs_registered_running_connected_and_a_native_session(self):
        self.assertEqual(cells.spawn_verdict(spawned())[0], cells.PASS)
        status, reason = cells.spawn_verdict(spawned(session=None, connection="pending"))
        self.assertEqual((status, reason), (cells.FAIL, "spawn answered without connected, native session"))
        refused = {"ok": False, "error": {"code": "native_identity_unavailable", "message": "retry the same intent"}}
        self.assertEqual(cells.spawn_verdict(refused),
                         (cells.FAIL, "spawn refused: native_identity_unavailable: retry the same intent"))

    def test_a_turn_ends_when_the_row_reads_idle_after_its_answer(self):
        events = [{"role": "assistant", "text": "JOURNEY-0", "at": 1_000},
                  {"role": "user", "text": "Reply with exactly JOURNEY-1", "at": 10_000},
                  {"role": "assistant", "text": "JOURNEY-1", "at": 12_300}]
        status, reason = cells.turn_verdict([("working", 10_500), ("stopped", 12_400)], events, "JOURNEY-1")
        self.assertEqual((status, reason), (cells.PASS, "row read working, then idle after the answer"))
        # A turn Herdr never saw working passes on the idle after its answer, and says so with its length.
        status, reason = cells.turn_verdict([("unknown", 9_000), ("stopped", 13_000)], events, "JOURNEY-1")
        self.assertEqual(status, cells.PASS)
        self.assertIn("working not seen: a turn shorter than Herdr's detection, 2.3 s", reason)

    def test_an_idle_from_before_the_prompt_or_another_turns_answer_is_not_a_pass(self):
        events = [{"role": "assistant", "text": "JOURNEY-0", "at": 1_000},
                  {"role": "user", "text": "Reply with exactly JOURNEY-1", "at": 10_000},
                  {"role": "assistant", "text": "JOURNEY-1", "at": 12_300}]
        # Idle read only before the answer, the row still working, or an answer from an earlier turn.
        self.assertEqual(cells.turn_verdict([("stopped", 9_000), ("working", 12_350)], events, "JOURNEY-1"),
                         (cells.FAIL, "row never read idle after the answer"))
        self.assertEqual(cells.turn_verdict([("stopped", 2_000)], events[:2], "JOURNEY-1"),
                         (cells.FAIL, "Hide's reader found no answer to the prompt"))
        self.assertEqual(cells.turn_verdict([("stopped", 13_000)], events, "JOURNEY-0", after=1),
                         (cells.FAIL, "Hide's reader found no answer to the prompt"))

    def test_the_emitter_must_be_herdrs_own_integration(self):
        self.assertEqual(cells.emitter_verdict({"source": "herdr:codex"}, "herdr:codex")[0], cells.PASS)
        self.assertEqual(cells.emitter_verdict({"source": "hide:hook"}, "herdr:codex")[0], cells.FAIL)
        self.assertEqual(cells.emitter_verdict(None, "herdr:codex")[0], cells.FAIL)
        # agy's integration reports under its target, not its kind (the contract's `herdr_source`).
        self.assertEqual(cells.emitter_verdict({"source": "herdr:antigravity_cli"}, "herdr:antigravity_cli")[0],
                         cells.PASS)
        self.assertEqual(cells.emitter_verdict({"source": "herdr:agy"}, "herdr:antigravity_cli")[0], cells.FAIL)

    def test_a_reply_passes_only_with_the_token_from_the_assistant_in_that_session(self):
        events = [{"role": "user", "text": "Reply with exactly JOURNEY-1"}, {"role": "assistant", "text": "JOURNEY-1"}]
        self.assertEqual(cells.reply_verdict({"session": "s", "events": events}, "s", "JOURNEY-1")[0], cells.PASS)
        # The token only in the human message, in another session, or before the turn is not a pass.
        self.assertEqual(cells.reply_verdict({"session": "s", "events": events[:1]}, "s", "JOURNEY-1"),
                         (cells.FAIL, "Hide's reader found no assistant message"))
        self.assertEqual(cells.reply_verdict({"session": "other", "events": events}, "s", "JOURNEY-1")[0], cells.FAIL)
        self.assertEqual(cells.reply_verdict({"session": "s", "events": events}, "s", "JOURNEY-1", after=2)[0], cells.FAIL)
        wrong = [{"role": "assistant", "text": "JOURNEY-2"}]
        self.assertEqual(cells.reply_verdict({"session": "s", "events": wrong}, "s", "JOURNEY-1"),
                         (cells.FAIL, "no assistant message carries JOURNEY-1"))
        self.assertEqual(cells.reply_verdict({"error": "no file"}, "s", "JOURNEY-1")[0], cells.FAIL)

    def test_a_letter_passes_only_when_delivered_and_answered_in_the_turn_the_bell_opened(self):
        bell = "🔔 controller: Reply with exactly the token on the next line"
        events = [{"role": "assistant", "text": "JOURNEY-1"}, {"role": "user", "text": bell},
                  {"role": "assistant", "text": "JOURNEY-1"}]
        read = {"session": "s", "events": events}
        delivered = {"state": "delivered", "bell_line": bell}
        self.assertEqual(cells.letter_verdict(delivered, read, "s", "JOURNEY-1")[0], cells.PASS)
        self.assertEqual(cells.letter_verdict({**delivered, "state": "pending"}, read, "s", "JOURNEY-1"),
                         (cells.FAIL, "letter state is pending, not delivered"))
        self.assertEqual(cells.letter_verdict({"state": "pending", "bell_line": None}, read, "s", "JOURNEY-1"),
                         (cells.FAIL, "the doorbell never rang (state pending)"))
        # A token answered before the bell's turn is not the letter's answer.
        early = {"session": "s", "events": events[:2]}
        self.assertEqual(cells.letter_verdict(delivered, early, "s", "JOURNEY-1")[0], cells.FAIL)

    def test_a_next_prompt_letter_passes_only_when_the_prompt_hook_confirmed_it_and_the_reply_carries_it(self):
        events = [{"role": "assistant", "text": "JOURNEY-0"},
                  {"role": "user", "text": "Reply with exactly the token on the second line of the letter attached to this prompt."},
                  {"role": "assistant", "text": "JOURNEY-1"}]
        read = {"session": "s", "events": events}
        confirmed = {"state": "delivered", "hook_confirmed": True}
        self.assertEqual(cells.prompt_letter_verdict(confirmed, read, "s", "JOURNEY-1", 1),
                         (cells.PASS, "delivered with the next prompt; assistant message carries JOURNEY-1"))
        # Handed over but never confirmed by the hook, or still pending, is not a delivery.
        for shown in ({"state": "delivered", "hook_confirmed": False}, {"state": "pending", "hook_confirmed": False}):
            status, reason = cells.prompt_letter_verdict(shown, read, "s", "JOURNEY-1", 1)
            self.assertEqual(status, cells.FAIL)
            self.assertIn("not delivered by the prompt hook", reason)
        # A token answered before the prompt the letter rode is not its answer.
        self.assertEqual(cells.prompt_letter_verdict(confirmed, read, "s", "JOURNEY-0", 1),
                         (cells.FAIL, "no assistant message carries JOURNEY-0"))

    def test_the_contract_names_each_agents_letter_and_wake_forms(self):
        rows = {row["id"]: row for row in contract()}
        self.assertEqual({key: (rows[key]["letter"], rows[key]["wake"]) for key in rows if key != "cursor"}, {
            "claude-code": ("bell", "pane"), "codex": ("bell", "pane"), "antigravity": ("next_prompt", "pane"),
            "grok": ("bell", "sleeping_session"), "opencode": ("next_prompt", "sleeping_session"),
            "pi": ("next_prompt", "sleeping_session"), "omp": ("next_prompt", "sleeping_session")})

    def test_sleep_and_wake_and_resume(self):
        self.assertEqual(cells.sleep_verdict([None, "sleeping"], True)[0], cells.PASS)
        self.assertEqual(cells.sleep_verdict([None, "waking"], True)[0], cells.FAIL)
        self.assertEqual(cells.sleep_verdict([None, "sleeping"], False), (cells.FAIL, "row never woke after Wake"))
        # A sleeping session's phases read the same, and a Wake the core says failed names why.
        self.assertEqual(cells.sleep_verdict(["saving_close", "closing", "sleeping"], True)[0], cells.PASS)
        self.assertEqual(cells.sleep_verdict(["sleeping"], False, "wake failed: reader unavailable"),
                         (cells.FAIL, "row never woke after Wake: wake failed: reader unavailable"))
        self.assertEqual(cells.resume_verdict("a", "b", (cells.PASS, "ok")),
                         (cells.FAIL, "native session changed across Wake (a -> b)"))
        self.assertEqual(cells.resume_verdict("a", "a", (cells.PASS, "ok")), (cells.PASS, "same session a; ok"))


class ClosedReader(unittest.TestCase):
    def daemon(self, closed):
        daemon = Daemon.__new__(Daemon)
        daemon.lock, daemon.closed, daemon.history = threading.Condition(), closed, {"p1": [("working", 1)]}
        daemon.rest = {"navigator": {"agents": [{"pane_id": "p1", "activity": "idle"}]}}
        return daemon

    def test_a_live_reader_adds_the_row_as_it_reads_now(self):
        self.assertEqual([activity for activity, _ in self.daemon(None).activities("p1")], ["working", "idle"])

    def test_a_reader_that_stopped_fails_instead_of_offering_its_last_row(self):
        # Its cached row would read idle forever and pass a turn that never ended.
        with self.assertRaisesRegex(ProcessError, "daemon_connection_lost:ConnectionError: daemon_closed_1006"):
            self.daemon("ConnectionError: daemon_closed_1006").activities("p1")


class UntrustedStart(unittest.TestCase):
    def test_an_answer_read_from_another_session_fails_the_start(self):
        spawned, turn, emitter = (cells.PASS, "spawned"), (cells.PASS, "turn ended"), (cells.PASS, "emitter")
        read = {"session": "other", "events": [{"role": "assistant", "text": "JOURNEY-1"}]}
        self.assertEqual(cells.start_verdict(spawned, turn, emitter, cells.reply_verdict(read, "spawned-session", "JOURNEY-1")),
                         (cells.FAIL, "Hide's reader read session other, not spawned-session"))


class LocalRoute(unittest.TestCase):
    def test_codex_login_copies_older_than_seven_days_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            auth = Path(temporary) / "auth.json"
            self.assertIn("not logged in", codex_login_refusal(auth))
            now = datetime.datetime(2026, 10, 17, tzinfo=datetime.timezone.utc)
            auth.write_text(json.dumps({"last_refresh": "2026-10-12T00:00:00Z"}))
            self.assertIsNone(codex_login_refusal(auth, now))
            auth.write_text(json.dumps({"last_refresh": "2026-10-09T00:00:00Z"}))
            self.assertEqual(codex_login_refusal(auth, now), "login near refresh (8.0 days old); run Codex once, then rerun")
            auth.write_text("{}")
            self.assertIn("unreadable", codex_login_refusal(auth, now))

    def test_an_oauth_login_copy_is_refused_unless_its_access_outlasts_the_run(self):
        now = datetime.datetime(2026, 10, 18, tzinfo=datetime.timezone.utc).timestamp()
        with tempfile.TemporaryDirectory() as temporary:
            auth = Path(temporary) / "auth.json"
            self.assertIn("not logged in", grok_login_refusal(auth, HOUR, now))
            self.assertIn("not logged in", pi_login_refusal(auth, HOUR, now))
            iso = lambda hours: datetime.datetime.fromtimestamp(now + hours * 3600, datetime.timezone.utc).isoformat()
            auth.write_text(json.dumps({"https://auth.x.ai::a": {"expires_at": iso(3), "refresh_token": "r"}}))
            self.assertIsNone(grok_login_refusal(auth, HOUR, now))
            auth.write_text(json.dumps({"https://auth.x.ai::a": {"expires_at": iso(0.5)}}))
            self.assertIn("login near refresh (0.5 h left, the run needs 1.0 h)", grok_login_refusal(auth, HOUR, now))
            # Pi's OpenAI access token lasts an hour: a short run may use it, a long one may not.
            auth.write_text(json.dumps({"openai": {"type": "oauth", "expires": (now + 0.8 * HOUR) * 1000}}))
            self.assertIsNone(pi_login_refusal(auth, 0.5 * HOUR, now))
            self.assertIn("near refresh", pi_login_refusal(auth, 0.7 * HOUR, now))
            auth.write_text(json.dumps({"openai": {"type": "oauth", "expires": (now - 3600) * 1000}}))
            refusal = pi_login_refusal(auth, HOUR, now)
            self.assertIn("login expired", refusal)
            self.assertIn("run pi once yourself", refusal)
            auth.write_text(json.dumps({"openai": {"type": "oauth", "expires": "soon"}}))
            self.assertIn("unreadable", pi_login_refusal(auth, HOUR, now))

    def test_grok_trust_adds_one_entry_for_the_resolved_repository(self):
        import tomllib
        with tempfile.TemporaryDirectory() as temporary:
            trusted = Path(temporary) / "trusted_folders.toml"
            repository = Path(temporary) / "repo"
            repository.mkdir()
            trust_grok(trusted, repository)
            trust_grok(trusted, repository)
            folders = tomllib.loads(trusted.read_text())["folders"]
            self.assertEqual(list(folders), [str(repository.resolve())])
            self.assertIs(folders[str(repository.resolve())]["trusted"], True)

    def test_codex_trust_adds_one_entry_for_the_resolved_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / "config.toml"
            config.write_text('model = "x"\n')
            repository = Path(temporary) / "repo"
            repository.mkdir()
            trust_codex(config, repository)
            trust_codex(config, repository)
            text = config.read_text()
            self.assertTrue(text.startswith('model = "x"\n'))
            self.assertEqual(text.count("[projects."), 1)
            self.assertIn(json.dumps(str(repository.resolve())), text)


if __name__ == "__main__":
    unittest.main()
