"""Independent safety/letter counterexamples; synthetic fixtures make no native claim."""

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.contracts import SCENES, recipes, source_contract
from agent_live_check.conversation import bell_turn, messages
from agent_live_check.protection import ProtectionError
from agent_live_check.report import exit_code, save, verdict
from agent_live_check.runtime import Runtime
from agent_live_check.processes import OwnedProcesses
from agent_live_check.authentication import AuthenticationRequired
from agent_live_check.scenes import arrived, matches, observe
from agent_live_check.system import exe, remove_tree


class RuntimeRouting(unittest.TestCase):
    def test_the_daemon_and_shell_keep_the_dock_and_shell_state_environment(self):
        # A native start can bypass an agent wrapper. Giving every child an
        # explicit Hide route would hide the installed hook's routing bug.
        with tempfile.TemporaryDirectory() as temporary, OwnedProcesses() as owner:
            root = Path(temporary).resolve()
            checkout, run, operator = root / "checkout", root / "run", root / "operator"
            for name in ("hided", "hide"):
                binary = checkout / "target/debug" / exe(name)
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"unused fixture executable")
                binary.chmod(0o700)
            manifest = checkout / "contracts/herdr-bundle.json"
            manifest.parent.mkdir()
            manifest.write_text('{"version":"fixture"}')
            with mock.patch("agent_live_check.runtime.herdr_matches_pin", return_value=True):
                runtime = Runtime(checkout, run, operator, owner, herdr_bin=root / exe("herdr"),
                                  state=run / "daemon-home/.hide/state")
            self.addCleanup(remove_tree, runtime.short)
            code, output, error = owner.run([sys.executable, "-c", (
                "import os; print('XDG_STATE_HOME' in os.environ, 'HIDE_STATE_DIR' in os.environ)"
            )], env=runtime.env, check=False)
            self.assertEqual(code, 0, error)
            self.assertEqual(output.strip(), "True False")
            self.assertNotIn("XDG_STATE_HOME", runtime.daemon_env)
            self.assertNotIn("HIDE_STATE_DIR", runtime.daemon_env)


def safe_agent():
    return {"id": "codex", "version": "fixture", "model": "fixture", "integration": {},
            "bell_target": True, "delivery": {"outcome": "verified"},
            "scenes": [{"scene": scene, "arrival": "reached", "status": "done",
                        "effect": "new_turn", "reason": "fixture"} for scene in SCENES]}


class MeasurementResults(unittest.TestCase):
    def test_unavailable_snapshot_keeps_unknown_rows_and_recovery_guidance(self):
        unavailable = safe_agent()
        unavailable["id"] = "omp"
        unavailable["bell_target"] = False
        unavailable["unavailable"] = {"reason": "credential_snapshot_unavailable",
                                      "next_action": "Close all omp instances and rerun."}
        unavailable["scenes"] = [{"scene": scene, "arrival": "unreached", "status": "unknown",
                                  "effect": "not_tested", "reason": "credential_snapshot_unavailable"}
                                 for scene in SCENES]
        with tempfile.TemporaryDirectory() as temporary:
            run = Path(temporary)
            report = {"herdr": {}, "agents": [unavailable, safe_agent()], "configuration": {},
                      "cleanup": {"confirmed": True}, "failures": [], "resources": {}}
            self.assertEqual(save(run, report), 3)
            saved = json.loads((run / "report.json").read_text())
            self.assertEqual([a["verdict"] for a in saved["agents"]], ["unknown", "safe_useful"])
            self.assertEqual(len(saved["agents"][0]["scenes"]), 10)
            self.assertIn("Close all omp instances and rerun.", (run / "report.md").read_text())

    def test_question_requires_a_menu_row_and_rejects_echo_or_prose(self):
        checkout = Path(__file__).resolve().parents[2]
        data = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])
        for recipe in data.values():
            question = recipe["scenes"]["question"]
            self.assertFalse(arrived(question, question["send"], "bell"))
            self.assertFalse(arrived(question, "One option is to continue.", "bell"))
            self.assertFalse(arrived(question, "Here are the choices; no interactive tool is open:\n1. One\n2. Two\n❯ ", "bell"))
            self.assertTrue(arrived(question, "1. One\n2. Two\nEnter to select · Esc to cancel", "bell"))

    def test_login_appearing_immediately_before_bell_receives_no_input(self):
        with tempfile.TemporaryDirectory() as name, OwnedProcesses() as owner:
            runtime = Runtime.__new__(Runtime)
            runtime.owner = owner
            screens = iter(["❯ ", "Sign in to continue"])
            runtime.screen = lambda pane, **kwargs: next(screens)
            runtime.agent = lambda pane, **kwargs: {"agent_status": "idle"}
            submitted = []
            runtime.command = lambda args, **kwargs: submitted.append(args)
            recipe = {"kind": "claude", "scenes": {"rest": {"send": "", "arrived": "❯", "draft": "", "no_match": "", "unsafe": ""}}}
            evidence = Path(name) / "login.json"
            with self.assertRaises(AuthenticationRequired):
                observe(runtime, "owned-pane", recipe, "rest", "bell", Path(name), 1, evidence,
                        Path(name), {"session_root": None, "settings": []})
            self.assertEqual(submitted, [])
            self.assertEqual(json.loads(evidence.read_text())["observation"]["arrival"], "skipped")

    def test_all_current_adapters_have_both_pickers_and_every_required_scene(self):
        checkout = Path(__file__).resolve().parents[2]
        contract = source_contract(checkout)
        data = recipes(checkout / "scripts/agent_live_check/recipes", contract["targets"])
        self.assertEqual(set(data), {"claude-code", "codex", "grok", "opencode", "pi", "omp", "antigravity"})
        for recipe in data.values():
            self.assertEqual(set(recipe["scenes"]), {"rest", "working", "shell_approval", "file_approval",
                "question", "plan_approval", "model_picker", "resume_picker", "mcp_approval", "startup"})
            self.assertEqual(recipe["scenes"]["model_picker"]["send"], "/model")
            self.assertEqual(recipe["scenes"]["resume_picker"]["send"], "/resume")

    def test_missing_or_retired_recipes_refuse_the_active_inventory(self):
        checkout = Path(__file__).resolve().parents[2]
        declared = source_contract(checkout)["targets"]
        directory = checkout / "scripts/agent_live_check/recipes"
        with tempfile.TemporaryDirectory() as temporary:
            copied = Path(temporary)
            for source in directory.glob("*.json"):
                (copied / source.name).write_bytes(source.read_bytes())
            codex = copied / "codex.json"
            content = codex.read_bytes()
            codex.unlink()
            with self.assertRaisesRegex(ProtectionError, "recipe_adapter_coverage_mismatch"):
                recipes(copied, declared)
            codex.write_bytes(content)
            retired = json.loads(content)
            retired["id"] = "cursor"
            (copied / "cursor.json").write_text(json.dumps(retired))
            with self.assertRaisesRegex(ProtectionError, "recipe_adapter_coverage_mismatch"):
                recipes(copied, declared)

    def test_unsafe_selection_approval_resume_settings_and_draft_dominate_unknown(self):
        for effect in ("selection", "approval", "resumed_session", "settings_write", "unsent_draft"):
            with self.subTest(effect=effect):
                agent = safe_agent()
                agent["scenes"][0].update(arrival="timeout", effect="not_tested")
                agent["scenes"][-1]["effect"] = effect
                self.assertEqual(verdict(agent), "unsafe")
                agent["verdict"] = verdict(agent)
                report = {"agents": [agent], "failures": [], "configuration": {}, "cleanup": {"confirmed": True}}
                self.assertEqual(exit_code(report), 1)
                agent["bell_target"] = False
                self.assertEqual(exit_code(report), 0)

    def test_missing_duplicate_skipped_or_timed_out_scene_never_passes(self):
        base = safe_agent()
        self.assertEqual(verdict(base), "safe_useful")
        for changed in (base["scenes"][:-1], base["scenes"] + [base["scenes"][0]]):
            agent = copy.deepcopy(base)
            agent["scenes"] = changed
            self.assertEqual(verdict(agent), "unknown")
        for arrival in ("timeout", "unreached", "skipped"):
            agent = copy.deepcopy(base)
            agent["scenes"][2]["arrival"] = arrival
            self.assertEqual(verdict(agent), "unknown")
        base["delivery"]["outcome"] = "unknown"
        self.assertEqual(verdict(base), "unknown")

    def test_integration_integrity_loss_refuses_positive_aggregation_but_preserves_unsafe(self):
        agent = safe_agent()
        agent["integration"] = {"status": "integrity_unproven", "integrity": "unproven", "loaded_version": None}
        with tempfile.TemporaryDirectory() as temporary:
            run = Path(temporary)
            report = {"herdr": {}, "agents": [agent], "configuration": {}, "cleanup": {"confirmed": True},
                      "failures": [], "resources": {}}
            self.assertEqual(save(run, report), 3)
            self.assertEqual(json.loads((run / "report.json").read_text())["agents"][0]["verdict"], "unknown")
            self.assertIn("| True | unknown | verified |", (run / "report.md").read_text())
        agent["scenes"][-1]["effect"] = "selection"
        self.assertEqual(verdict(agent), "unsafe")

    def test_marker_in_prompt_tool_echo_or_another_turn_is_not_delivery(self):
        bell, marker = "fixture bell", "fresh-fixture-marker"
        self.assertFalse(bell_turn([("user", bell), ("tool", marker)], bell, marker))
        self.assertFalse(bell_turn([("user", marker), ("user", bell), ("assistant", marker)], bell, marker))
        self.assertFalse(bell_turn([("tool", marker), ("user", bell), ("assistant", marker)], bell, marker))
        self.assertFalse(bell_turn([("user", bell), ("user", "another prompt"), ("assistant", marker)], bell, marker))
        self.assertTrue(bell_turn([("user", bell), ("tool", marker), ("assistant", marker)], bell, marker))

    def test_native_readers_keep_tool_output_separate_from_assistant_text(self):
        with tempfile.TemporaryDirectory() as temporary:
            file = Path(temporary) / "session.jsonl"
            file.write_text('\n'.join(json.dumps(value) for value in [
                {"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "bell"}]}},
                {"type": "response_item", "payload": {"type": "function_call_output", "output": "marker"}},
                {"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "marker"}]}}
            ]))
            self.assertEqual(messages(file, "codex"), [("user", "bell"), ("tool", "marker"), ("assistant", "marker")])
            file.write_text('\n'.join(json.dumps(value) for value in [
                {"type": "user", "message": {"role": "user", "content": "bell"}},
                {"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "content": "marker"}]}},
                {"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": "marker"}]}}
            ]))
            self.assertEqual(messages(file, "claude"), [("user", "bell"), ("tool", '"marker"'), ("assistant", "marker")])

    def test_two_reports_preserve_unsafe_scene_and_refuse_overwrite(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = Path(temporary)
            agent = safe_agent()
            agent["scenes"][-1]["effect"] = "unsent_draft"
            report = {"herdr": {}, "agents": [agent], "configuration": {}, "cleanup": {"confirmed": True},
                      "failures": [], "resources": {}}
            self.assertEqual(save(run, report), 1)
            stored = json.loads((run / "report.json").read_text())
            self.assertEqual(stored["agents"][0]["verdict"], "unsafe")
            self.assertIn("unsent_draft", (run / "report.md").read_text())
            with self.assertRaises(FileExistsError):
                save(run, report)

    def test_mixed_provider_reports_keep_all_rows_and_unsafe_exit_in_both_orders(self):
        # B1/B9: a later provider must not overwrite an earlier unsafe result.
        # The expected provider/scene pairs come from the complete PRD matrix.
        unsafe = safe_agent()
        unsafe["id"] = "claude-code"
        unsafe["scenes"][-1].update(effect="unsent_draft", reason="unsafe fixture menu")
        other = safe_agent()
        for outcome in ("verified", "unknown"):
            for reverse in (False, True):
                with self.subTest(outcome=outcome, reverse=reverse), tempfile.TemporaryDirectory() as temporary:
                    other["delivery"]["outcome"] = outcome
                    agents = copy.deepcopy([other, unsafe] if reverse else [unsafe, other])
                    report = {"herdr": {}, "agents": agents, "configuration": {},
                              "cleanup": {"confirmed": True}, "failures": [], "resources": {}}
                    run = Path(temporary)
                    self.assertEqual(save(run, report), 1)
                    stored = json.loads((run / "report.json").read_text())
                    self.assertEqual([agent["id"] for agent in stored["agents"]],
                                     ["codex", "claude-code"] if reverse else ["claude-code", "codex"])
                    pairs = [(agent["id"], row["scene"]) for agent in stored["agents"] for row in agent["scenes"]]
                    self.assertCountEqual(pairs, [(agent, scene) for agent in ("claude-code", "codex") for scene in SCENES])
                    markdown = (run / "report.md").read_text()
                    for agent in ("claude-code", "codex"):
                        for scene in SCENES:
                            self.assertIn(f"| {agent} | {scene} |", markdown)
                    self.assertIn("| claude-code | fixture | fixture | {} | True | unsafe |", markdown)
                    self.assertIn("unsent_draft", markdown)

    def test_operator_socket_refused_before_any_executable_or_candidate_check(self):
        with tempfile.TemporaryDirectory() as temporary, OwnedProcesses() as owner:
            root = Path(temporary).resolve()
            run, operator = root / "run", root / "operator"
            run.mkdir(mode=0o700)
            operator.mkdir(mode=0o700)
            with self.assertRaisesRegex(ProtectionError, "operator_socket_refused"):
                Runtime(root, run, operator, owner, herdr_bin=root / "does-not-exist",
                        socket=operator / ".config/herdr/herdr.sock")
            self.assertEqual(owner.children, {})


if __name__ == "__main__":
    unittest.main()
