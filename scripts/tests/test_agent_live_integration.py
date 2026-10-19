"""Version claims require isolated prepared bytes and positive native reports."""

import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.contracts import recipes, source_contract
from agent_live_check.integration import observe, prepare, project_args
from agent_live_check.overlay import prepare as prepare_overlay
from agent_live_check.protection import ProtectionError, private_directory, stamp, write_private


class IntegrationEvidence(unittest.TestCase):
    def test_observation_reserves_each_original_or_routed_cohort_before_reading(self):
        for second_cohort in ("generated", "routed"):
            with self.subTest(second_cohort=second_cohort), tempfile.TemporaryDirectory() as name:
                probe = Path(name).resolve()
                artifacts = []
                for index, cohort in enumerate(("generated", second_cohort)):
                    file = probe / str(index)
                    content = b"prepared fixture"
                    write_private(file, content)
                    artifacts.append({"file": file, "content": content, "stamp": stamp(file),
                                      "cohort": cohort, "integration_id": "codex", "version": 8})
                    file.write_bytes(b"x" * (9 * 1024 * 1024))
                queried = []
                runtime = SimpleNamespace(probe=probe, agent=lambda pane: queried.append(pane))
                plan = {"artifacts": artifacts, "sole_route": True, "synthetic": True}
                if second_cohort == "generated":
                    with self.assertRaisesRegex(ProtectionError, "prepared_integration_bytes_over_budget") as caught:
                        observe(runtime, "owned", {"kind": "codex"}, plan)
                    self.assertEqual(caught.exception.path, str(artifacts[1]["file"]))
                    self.assertEqual(queried, [], "the byte refusal must precede native queries")
                else:
                    result = observe(runtime, "owned", {"kind": "codex"}, plan)
                    self.assertEqual(result["integrity"], "unproven")
                    self.assertEqual(len(result["integrity_changes"]), 2)
                    self.assertIsNone(result["loaded_version"])

    def test_private_preparation_and_late_binding_never_replace_missing_load_proof(self):
        checkout = Path(__file__).resolve().parents[2]
        data = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])
        with tempfile.TemporaryDirectory() as name:
            base = Path(name).resolve()
            operator = base / "operator"
            operator.mkdir()
            for key, recipe in data.items():
                probe = base / key
                probe.mkdir()
                kind = recipe["kind"]
                # The installer double writes fixture bytes. This test does
                # not attest that a real CLI loaded an integration.
                relative = {"claude": ".claude", "codex": ".codex", "pi": ".pi/agent",
                            "omp": ".omp/agent", "opencode": ".config/opencode",
                            "grok": ".grok", "agy": ".gemini/config"}[kind]
                target = {"agy": "antigravity-cli"}.get(kind, kind)
                marked = {"agy": "antigravity_cli"}.get(kind, kind)
                def installer(args, *, env, cwd):
                    self.assertEqual(args, ["/fixture/pinned-herdr", "integration", "install", target])
                    self.assertNotEqual(env["HOME"], str(operator))
                    self.assertEqual(env["XDG_CONFIG_HOME"], str(Path(env["HOME"]) / ".config"))
                    self.assertTrue(all(str(value).startswith(env["HOME"] + "/") for key, value in env.items()
                                        if key.startswith("XDG_")))
                    root = Path(env["HOME"]) / relative
                    write_private(root / "fixture-hook", (
                        "# HERDR_INTEGRATION_ID=" + marked + "\n# HERDR_INTEGRATION_VERSION=17\n").encode())
                    if kind == "opencode":
                        write_private(root / "fixture-tui", (
                            "// HERDR_INTEGRATION_ID=opencode-tui\n// HERDR_INTEGRATION_VERSION=18\n").encode())
                    if kind in ("claude", "agy"):
                        write_private(root / ("settings.json" if kind == "claude" else "hooks.json"), b"{}")
                native = [None]
                runtime = SimpleNamespace(probe=probe, fixture_bin=None,
                                          env={"XDG_CONFIG_HOME": "/other/config", "XDG_DATA_HOME": "/other/data",
                                               "XDG_STATE_HOME": "/other/state", "XDG_CACHE_HOME": "/other/cache"},
                                          herdr_bin=Path("/fixture/pinned-herdr"),
                                          owner=SimpleNamespace(run=installer), agent=lambda pane: native[0])
                overlay = prepare_overlay(recipe, probe, operator)
                plan = prepare(runtime, recipe, overlay)
                first = observe(runtime, "owned", recipe, plan)
                self.assertEqual(first["status"], "not_observed")
                self.assertIsNone(first["loaded_version"])
                native[0] = {"agent_session": {"source": "herdr:" + marked, "kind": "id", "value": "native"}}
                later = observe(runtime, "owned", recipe, plan)
                if kind == "agy":
                    cwd = probe / "project"
                    private_directory(cwd)
                    project_args(plan, recipe, cwd)
                    self.assertEqual(json.loads((cwd / ".agents/hooks.json").read_text()), {})
                    self.assertIsNone(later["loaded_version"], "project hooks cannot exclude global ambiguity")
                elif kind == "opencode":
                    self.assertIsNone(later["loaded_version"], "native source does not identify alternative frontend versions")
                    self.assertEqual({row["version"] for row in later["prepared_artifacts"]}, {17, 18})
                else:
                    self.assertEqual(later["loaded_version"], [17])
                plan["artifacts"][0]["file"].write_bytes(b"changed after preparation")
                changed = observe(runtime, "owned", recipe, plan)
                self.assertEqual(changed["status"], "integrity_unproven")
                self.assertIsNone(changed["loaded_version"])
                self.assertEqual(changed["integrity_changes"][0]["changed_fields"], ["digest", "size"])
                self.assertNotIn("observed_sha256", changed["integrity_changes"][0])
                plan["artifacts"][0]["file"].write_bytes(plan["artifacts"][0]["content"])
                native[0] = None
                restored = observe(runtime, "owned", recipe, plan)
                self.assertEqual(restored["status"], "integrity_unproven", "a later observation cannot erase the change")
                self.assertIsNone(restored["loaded_version"])
                plan["artifacts"][0]["file"].unlink()
                plan["artifacts"][0]["file"].symlink_to(operator)
                with self.assertRaises(ProtectionError):
                    observe(runtime, "owned", recipe, plan)

    def test_copied_opencode_configuration_cannot_prove_exclusive_emitter(self):
        checkout = Path(__file__).resolve().parents[2]
        recipe = recipes(checkout / "scripts/agent_live_check/recipes", source_contract(checkout)["targets"])["opencode"]
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            operator, probe = root / "operator", root / "probe"
            private_directory(probe)
            config = operator / ".config/opencode/opencode.json"
            private_directory(config.parent)
            write_private(config, b'{"plugin":["/external/integration.js"]}')
            def installer(args, *, env, cwd):
                write_private(Path(env["HOME"]) / ".config/opencode/fixture-hook",
                              b"// HERDR_INTEGRATION_ID=opencode\n// HERDR_INTEGRATION_VERSION=17\n")
            runtime = SimpleNamespace(probe=probe, fixture_bin=None, env={}, herdr_bin=Path("/fixture/herdr"),
                                      owner=SimpleNamespace(run=installer),
                                      agent=lambda pane: {"agent_session": {"source": "herdr:opencode"}})
            overlay = prepare_overlay(recipe, probe, operator)
            plan = prepare(runtime, recipe, overlay)
            result = observe(runtime, "owned", recipe, plan)
            self.assertIsNone(result["loaded_version"], "copied configuration can load a competing integration")
            self.assertEqual(result["status"], "native_session_observed")
            self.assertEqual(config.read_bytes(), b'{"plugin":["/external/integration.js"]}')


if __name__ == "__main__":
    unittest.main()
