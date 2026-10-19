"""Scene preparation must connect real project tools and owned native history."""

import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.contracts import recipes, source_contract
from agent_live_check.conversation import messages
from agent_live_check.protection import ProtectionError
from agent_live_check.scenes import transcript
from agent_live_check.setup import configure, prepare_startup
from agent_live_check.runtime import Runtime
from agent_live_check.authentication import AuthenticationRequired
from agent_live_check.processes import OwnedProcesses, ProcessError
from agent_live_check.history import LABEL, seed
from agent_live_check.scenes import observe, startup_blocker


class ScenePreparation(unittest.TestCase):
    def test_scene_keeps_received_frame_when_auth_or_identity_check_fails(self):
        for phase in ("arrival", "after"):
            for failure in ("identity", "authentication"):
                with self.subTest(phase=phase, failure=failure), tempfile.TemporaryDirectory() as name:
                    root = Path(name)
                    evidence = root / "scene.json"
                    screens, queries, sent = [], [], []
                    ordinal = 1 if phase == "arrival" else 2
                    current = "Sign in to continue" if failure == "authentication" else "current received frame"
                    def screen(pane, **kwargs):
                        screens.append(pane)
                        return current if len(screens) == ordinal else "READY before"
                    def agent(pane, **kwargs):
                        queries.append(pane)
                        if failure == "identity" and len(queries) == ordinal:
                            raise ProcessError("fixture_identity_query_failed")
                        return {"agent_status": "idle"}
                    runtime = SimpleNamespace(fixture_bin=None, screen=screen, agent=agent,
                                              send=lambda pane, text, **kwargs: sent.append(text),
                                              owner=SimpleNamespace(deadline=time.monotonic() + 5,
                                                                    cancelled=threading.Event()))
                    recipe = {"kind": "claude", "scenes": {"rest": {
                        "send": "", "arrived": "READY", "draft": "never", "no_match": "never", "unsafe": "never"}}}
                    expected = ProcessError if failure == "identity" else AuthenticationRequired
                    with self.assertRaises(expected):
                        observe(runtime, "owned", recipe, "rest", "bell", root, 1,
                                evidence, root, {"session_root": None, "settings": []})
                    record = json.loads(evidence.read_text())
                    sample = record["samples"][-1]
                    self.assertEqual(sample, {"phase": phase, "screen": current, "agent": None})
                    self.assertEqual(len(record["samples"]), ordinal)
                    self.assertEqual(len(screens), ordinal)
                    self.assertEqual(len(queries), ordinal if failure == "identity" else ordinal - 1)
                    self.assertEqual(sent, [] if phase == "arrival" else ["bell"])

    def trust_fixture(self, root):
        checkout = Path(__file__).resolve().parents[2]
        recipe = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])["claude-code"]
        cwd = root / "owned-folder"
        cwd.mkdir(mode=0o700)
        state, commands = [0], []
        prompt = "Is this a project you created or one you trust?\n"
        screens = [prompt + "❯ No, exit\n  Yes, I trust this folder\n",
                   prompt + "  No, exit\n❯ Yes, I trust this folder\n", "❯ \n"]
        actual = {"pane_id": "owned", "workspace_id": "owned-workspace", "cwd": str(cwd),
                  "agent": "claude", "name": "live-claude-code-rest"}
        def command(args, **kwargs):
            commands.append(args)
            state[0] += 1
        runtime = SimpleNamespace(fixture_bin=None, probe=root, workspaces={"owned-workspace"},
                                  checkout_directories={cwd}, pane_credentials={"owned": object()},
                                  screen=lambda pane, **kwargs: screens[state[0]], command=command,
                                  agent=lambda pane, **kwargs: {**actual, "agent_status": "idle" if state[0] == 2 else "blocked",
                                                      "launch_pending": state[0] != 2},
                                  owner=SimpleNamespace(deadline=time.monotonic() + 5, cancelled=threading.Event()))
        runtime.wait = lambda predicate, seconds: Runtime.wait(runtime, predicate, seconds)
        return runtime, recipe, cwd, state, commands, screens, actual

    def test_preparation_uses_one_deadline_for_initial_queries_keys_and_ready(self):
        for operation, ordinal, expected_keys in (("screen", 1, []), ("screen", 3, ["down"]),
                ("agent", 2, ["down"]), ("screen", 4, ["down"]), ("command", 1, ["down"]),
                ("command", 2, ["down", "enter"]), ("screen", 5, ["down", "enter"])):
            with self.subTest(operation=operation, ordinal=ordinal), tempfile.TemporaryDirectory() as name:
                root = Path(name).resolve()
                runtime, recipe, cwd, _, commands, _, _ = self.trust_fixture(root)
                clock, counts, budgets = [100.0], {}, []
                runtime.owner.deadline = 200
                def transport(kind, original):
                    def call(*args, seconds, **kwargs):
                        self.assertGreater(seconds, 0)
                        self.assertLessEqual(seconds, 101.0 - clock[0])
                        budgets.append((kind, seconds))
                        counts[kind] = counts.get(kind, 0) + 1
                        clock[0] += 1.1 if (kind, counts[kind]) == (operation, ordinal) else 0.025
                        return original(*args)
                    return call
                for kind in ("screen", "agent", "command"):
                    setattr(runtime, kind, transport(kind, getattr(runtime, kind)))
                evidence = root / "preparation.json"
                with patch("agent_live_check.setup.time.monotonic", lambda: clock[0]):
                    with self.assertRaisesRegex(ProcessError, "scene_timeout"):
                        prepare_startup(runtime, "owned", recipe, "rest", cwd, "owned-workspace", 1, evidence)
                self.assertEqual([args[-1] for args in commands], expected_keys)
                self.assertEqual(json.loads(evidence.read_text())["outcome"], "unknown")
                self.assertEqual(counts[operation], ordinal, "no transport call may start after expiration")
                self.assertTrue(budgets)
                if operation == "screen" and ordinal == 1:
                    first = json.loads(evidence.read_text())["samples"][0]
                    self.assertIn("❯ No, exit", first["screen"])
                    self.assertIsNone(first["agent"])

    def test_large_preparation_window_keeps_each_transport_at_fifteen_seconds(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            runtime, recipe, cwd, _, _, _, _ = self.trust_fixture(root)
            runtime.owner.deadline = 500
            budgets = []
            def transport(original):
                def call(*args, seconds, **kwargs):
                    budgets.append(seconds)
                    return original(*args)
                return call
            for kind in ("screen", "agent", "command"):
                setattr(runtime, kind, transport(getattr(runtime, kind)))
            with patch("agent_live_check.setup.time.monotonic", lambda: 100):
                self.assertTrue(prepare_startup(runtime, "owned", recipe, "rest", cwd,
                                                "owned-workspace", 120, root / "prep.json"))
            self.assertTrue(budgets)
            self.assertTrue(all(value == 15 for value in budgets))

    def test_trust_keys_keep_original_deadline_through_real_process_admission(self):
        for key in ("down", "enter"):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as name:
                root = Path(name).resolve()
                runtime, recipe, cwd, _, commands, _, _ = self.trust_fixture(root)
                clock, emitted = [100.0], []
                runtime.owner = OwnedProcesses()
                runtime.owner.deadline = 500
                runtime.owner.family = "finite-preparation-admission-fixture"
                runtime.herdr_bin, runtime.env = Path("/external/herdr"), {}
                external_command = runtime.command
                def command(args, **kwargs):
                    if args[-1] == key:
                        clock[0] = 101.1  # after the caller computed remaining
                        return Runtime.command(runtime, args, **kwargs)
                    return external_command(args, **kwargs)
                def launch(*args, **kwargs):
                    emitted.append(args)
                    raise AssertionError("expired trust key reached OS spawn")
                runtime.command = command
                evidence = root / "prep.json"
                with patch("time.monotonic", lambda: clock[0]), patch("subprocess.Popen", launch):
                    with self.assertRaisesRegex(ProcessError, "command_timeout"):
                        prepare_startup(runtime, "owned", recipe, "rest", cwd, "owned-workspace", 1, evidence)
                self.assertEqual(emitted, [])
                self.assertEqual([args[-1] for args in commands], [] if key == "down" else ["down"])
                self.assertEqual(json.loads(evidence.read_text())["outcome"], "unknown")
                self.assertEqual(runtime.owner.children, {})

    def test_received_screen_survives_a_failed_following_identity_query(self):
        for ordinal, phase in ((1, "before"), (2, "selected")):
            with self.subTest(ordinal=ordinal), tempfile.TemporaryDirectory() as name:
                root = Path(name).resolve()
                runtime, recipe, cwd, _, commands, screens, _ = self.trust_fixture(root)
                original, queries = runtime.agent, []
                def agent(*args, **kwargs):
                    queries.append(args)
                    if len(queries) == ordinal:
                        raise ProcessError("fixture_identity_query_failed")
                    return original(*args, **kwargs)
                runtime.agent = agent
                evidence = root / "preparation.json"
                with self.assertRaisesRegex(ProcessError, "fixture_identity_query_failed"):
                    prepare_startup(runtime, "owned", recipe, "rest", cwd, "owned-workspace", 1, evidence)
                record = json.loads(evidence.read_text())
                sample = next(row for row in record["samples"] if row["phase"] == phase)
                self.assertEqual(sample["screen"], screens[ordinal - 1])
                self.assertIsNone(sample["agent"])
                self.assertEqual([row[-1] for row in commands], [] if ordinal == 1 else ["down"])
                self.assertEqual(record["outcome"], "unknown")
                self.assertEqual(len(queries), ordinal)

    def test_nonstartup_trust_preparation_confirms_each_owned_selection_before_ready(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            runtime, recipe, cwd, state, commands, _, _ = self.trust_fixture(root)
            evidence = root / "preparation.json"
            self.assertFalse(prepare_startup(runtime, "owned", recipe, "startup", cwd,
                                            "owned-workspace", 1, evidence))
            self.assertEqual(commands, [])
            self.assertTrue(prepare_startup(runtime, "owned", recipe, "rest", cwd,
                                           "owned-workspace", 1, evidence))
            self.assertEqual(commands, [["pane", "send-keys", "owned", "down"],
                                        ["pane", "send-keys", "owned", "enter"]])
            record = json.loads(evidence.read_text())
            self.assertEqual(record["outcome"], "ready")
            self.assertEqual(record["key_attempts"], ["down", "enter"])
            self.assertEqual([row["phase"] for row in record["samples"]], ["before", "selected", "confirmation", "ready"])
            self.assertEqual(state[0], 2)

    def test_trust_preparation_refuses_changed_identity_selection_or_authentication(self):
        cases = [(field, value) for field, value in (("pane_id", "other"), ("workspace_id", "other"),
                 ("cwd", "/operator"), ("agent", "codex"), ("name", "other"))]
        cases += [("default", None), ("authentication", None), ("registration", None), ("deadline", None)]
        for field, value in cases:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as name:
                root = Path(name).resolve()
                runtime, recipe, cwd, _, commands, screens, actual = self.trust_fixture(root)
                if field == "default":
                    screens[0] = screens[1]
                elif field == "authentication":
                    screens[1] = "Sign in to continue"
                elif field == "registration":
                    runtime.pane_credentials.clear()
                elif field == "deadline":
                    runtime.owner.deadline = time.monotonic() - 1
                else:
                    actual[field] = value
                evidence = root / "preparation.json"
                expected = AuthenticationRequired if field == "authentication" else ProcessError
                with self.assertRaises(expected):
                    prepare_startup(runtime, "owned", recipe, "rest", cwd, "owned-workspace", 1, evidence)
                self.assertEqual(commands, [["pane", "send-keys", "owned", "down"]] if field == "authentication" else [])
                self.assertEqual(json.loads(evidence.read_text())["outcome"], "unknown")

    def test_blocked_startup_is_observed_only_for_the_owned_matching_native_agent(self):
        recipe = {"id": "pi", "kind": "pi", "scenes": {"startup": {"arrived": "Do you trust"}}}
        actual = {"pane_id": "owned", "workspace_id": "owned-workspace", "cwd": "/owned/probe",
                  "agent": "pi", "name": "live-pi-startup", "agent_status": "blocked"}
        error = json.dumps({"error": {"code": "agent_not_ready"}})
        result = (1, "", error)  # Pinned CLI puts structured failures on stderr.
        screen = "Do you trust this folder?"
        def observed(scene="startup", pane="owned", response=result, identity=actual, frame=screen):
            return startup_blocker(scene, recipe, pane, response, identity, frame,
                                   cwd=Path("/owned/probe"), workspace="owned-workspace")
        self.assertTrue(observed())
        unregistered = {key: value for key, value in actual.items() if key != "name"}
        self.assertTrue(observed(response=(1, "", '{"error":{"code":"timeout"}}'),
                                 identity={**unregistered, "agent_status": "unknown"}))
        self.assertFalse(observed(frame="ordinary input"))
        self.assertFalse(observed(scene="rest"))
        self.assertFalse(observed(pane="other"))
        for field, value in (("agent", "codex"), ("cwd", "/other"), ("workspace_id", "other"), ("name", "other")):
            self.assertFalse(observed(identity={**actual, field: value}))
        self.assertFalse(observed(response=(0, error, "")))
        self.assertFalse(observed(response=(1, error, "not JSON")))
        self.assertFalse(observed(response=(1, "", '{"error":{"code":"agent_pane_busy"}}')))

    def test_non_jsonl_providers_seed_real_completed_native_prompts_for_resume(self):
        checkout = Path(__file__).resolve().parents[2]
        data = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])
        for key in ("grok", "opencode"):
            recipe, index, submitted = data[key], [0], []
            screens = ["❯ ", LABEL + "\nWorking", LABEL + "\n❯ "]
            statuses = ["idle", "working", "done"]
            session = {"kind": "id", "source": "herdr:" + key, "value": "owned-native-session"}
            def agent(pane, **kwargs):
                value = {"agent_status": statuses[index[0]], "agent_session": session}
                index[0] += 1
                return value
            runtime = SimpleNamespace(fixture_bin=None, screen=lambda pane, **kwargs: screens[index[0]], agent=agent,
                                      send=lambda pane, text, **kwargs: submitted.append(text),
                                      owner=SimpleNamespace(deadline=time.monotonic() + 5,
                                                            cancelled=threading.Event()))
            result = seed(runtime, "owned", recipe, Path("/unused"), {"session_root": None}, 2)
            self.assertEqual(len(submitted), 1)
            self.assertEqual(result["visible_tokens"], [LABEL])
            self.assertFalse(result["assistant_reply_verified"], "screen/status evidence cannot prove assistant mail")

    def test_every_mcp_recipe_connects_only_the_inert_project_server(self):
        checkout = Path(__file__).resolve().parents[2]
        data = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])
        with tempfile.TemporaryDirectory() as name:
            for key, recipe in data.items():
                cwd = Path(name) / key
                cwd.mkdir(mode=0o700)
                calls = []
                runtime = SimpleNamespace(checkout=checkout, short=Path(name), native_env={},
                                          owner=SimpleNamespace(run=lambda args, **kw: calls.append((args, kw))))
                arguments = configure(runtime, Path("/fixture/cli"), recipe, "mcp_approval", cwd)
                if key == "codex":
                    self.assertIn('mcp_servers.live_probe.default_tools_approval_mode="prompt"', arguments)
                    self.assertTrue(any("mcp_fixture.py" in item for item in arguments))
                elif key == "grok":
                    self.assertEqual(calls[0][1]["cwd"], cwd)
                    self.assertIn("--scope", calls[0][0])
                    self.assertIn("project", calls[0][0])
                    self.assertIn("--leader-socket", calls[0][0])
                else:
                    file = cwd / ("live-mcp-mcp_approval.json" if key == "claude-code" else recipe["mcp"]["path"])
                    config = json.loads(file.read_text())
                    servers = config["mcp" if key == "opencode" else "mcpServers"]
                    self.assertEqual(set(servers), {"live_probe"})
                    self.assertIn("mcp_fixture.py", json.dumps(servers))
                    self.assertEqual(file.stat().st_mode & 0o777, 0o600)

    def test_pi_and_omp_use_actual_native_identity_and_separate_tool_text(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            file = root / "2026_fixture-native-id.jsonl"
            file.write_text("\n".join(json.dumps(row) for row in [
                {"type": "session", "id": "fixture-native-id", "cwd": name},
                {"type": "message", "message": {"role": "user", "content": "bell"}},
                {"type": "message", "message": {"role": "toolResult", "content": [{"type": "text", "text": "marker"}]}},
                {"type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": "reply"}]}}
            ]))
            for kind in ("pi", "omp"):
                session = {"kind": "id", "source": "herdr:" + kind, "value": "fixture-native-id"}
                self.assertEqual(transcript(root, kind, session, root), file)
                self.assertEqual(messages(file, kind), [("user", "bell"), ("tool", "marker"), ("assistant", "reply")])
                session.update(kind="path", value=str(file))
                self.assertEqual(transcript(root, kind, session, root), file)
                session["value"] = str(root.parent / "operator.jsonl")
                with self.assertRaisesRegex(ProtectionError, "outside_owned_history"):
                    transcript(root, kind, session, root)


if __name__ == "__main__":
    unittest.main()
