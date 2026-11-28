"""Which lanes `scripts/ci-plan.py` plans for real changes, and what `verify` accepts."""
import contextlib
import importlib.util
import io
from fnmatch import fnmatchcase
import json
import os
import re
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tempfile
import unittest
import unittest.mock

import yaml

SPEC = importlib.util.spec_from_file_location("ci_plan", Path(__file__).parents[1] / "ci-plan.py")
ci = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci)
ROOT = Path(__file__).parents[2]
CRATES = ci.cargo_crates(ROOT)
EVERY_PACKAGE = sorted(crate["name"] for crate in CRATES.values())
# The lanes that take a macOS runner; every other check on macOS is a job of
# `nightly.yml`.
MACOS_LANES = {"os-contract-macos", "vt", "holder-restart", "desktop-e2e", "package", "omp-contract", "relay-protocol"}
# What a push, the nightly and an unclassified path plan: every lane but
# `package`, `agent-clis` and `holder-restart`, which are planned by their own
# paths only.
FULL = [
    "policy", "rust", "checks", "android", "docs", "relay-protocol", "os-contract", "os-contract-macos", "vt", "agy-contract", "omp-contract", "agent-journey",
    "windows-check", "windows-e2e", "windows-desktop", "windows-wsl", "web-e2e", "remote-mailbox", "remote-mailbox-arm",
    "desktop-e2e",
]
# Every lane, independently named: CI edits must exercise the planner's decisions.
ALL = [
    "policy", "rust", "checks", "android", "docs", "relay-protocol", "os-contract", "os-contract-macos", "vt", "holder-restart", "agy-contract", "omp-contract", "agent-journey",
    "agent-clis", "windows-check", "windows-e2e", "windows-desktop", "windows-wsl", "web-e2e", "remote-mailbox",
    "remote-mailbox-arm", "repeat", "desktop-e2e", "package",
]
# A full plan with a package input.
PACKAGED = FULL + ["package"]
# The paths a pull request changed when `package.yml` had its own trigger.
PACKAGE_INPUTS = (
    "desktop/scripts/package.mjs", "desktop/scripts/build.mjs", "desktop/package.json", "desktop/resources/entitlements.plist",
    "scripts/verify-cargo.sh", "scripts/verify-web.sh", "scripts/toolchain-env.sh",
    "hided/build.rs", "hided/src/cli.rs", "hide-kit/src/lib.rs", "hide-agent-hooks/src/main.rs",
    ".github/workflows/package.yml", ".github/workflows/device-helpers.yml", ".github/workflows/release.yml",
)


def plan(*paths, status="M"):
    return ci.select([(status, path) for path in paths], CRATES, ROOT)


def needs_for(result, **override):
    needs = {lane: {"result": "success" if lane in result["lanes"] else "skipped"} for lane in ci.LANES}
    needs["plan"] = {"result": "success", "outputs": {"lanes": json.dumps(result["lanes"])}}
    for lane, outcome in override.items():
        needs[lane.replace("_", "-")] = {"result": outcome}
    return needs


class Selection(unittest.TestCase):
    def test_unit_test_changes_keep_existing_systems_without_application_suites(self):
        for status in ("A", "M", "D"):
            for path in ("desktop/src/main/new.test.ts", "desktop/src/main/updater.test.ts",
                         "desktop/e2e/fixture-cleanup.unit.ts"):
                with self.subTest(status=status, path=path):
                    result = plan(path, status=status)
                    # An added or modified unit test also plans the lane that
                    # repeats it; a deleted one has no test left to repeat.
                    expected = ["policy", "checks", "windows-check"] + (["repeat"] if status in ("A", "M") else [])
                    self.assertEqual(result["lanes"], expected)
                    self.assertEqual(result["rust_packages"], [])
                    self.assertFalse(ci.reads_no_lane(status, path, CRATES, ROOT))

    def test_the_android_app_host_plans_its_own_lane_and_the_mobile_check(self):
        for status in ("A", "M", "D"):
            for path in ("mobile/android/app/src/main/java/me/grab/hide/android/BridgeGate.java", "mobile/android/app/build.gradle"):
                with self.subTest(status=status, path=path):
                    self.assertEqual(plan(path, status=status)["lanes"], ["policy", "checks", "android"])
        # The script that checks its lists is the iOS app host's unit suite's, which `checks` runs.
        self.assertEqual(plan("mobile/scripts/check-versions.mjs")["lanes"], ["policy", "checks"])

    def test_web_unit_tests_keep_their_css_build_readers_and_plan_no_rust(self):
        # Tailwind's web-root scan reads source and e2e unit tests; no Rust test
        # reads web/src, so a web-only change builds no crate.
        for path in ("web/src/new.test.ts", "web/src/components/new.test.tsx"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(set(result["lanes"]), {"policy", "checks", "web-e2e", "desktop-e2e", "repeat"})
                self.assertEqual(result["rust_packages"], [])
                self.assertEqual(result["repeat"]["units"], [path])
        self.assertEqual(set(plan("web/e2e/platform-program.unit.ts")["lanes"]),
                         {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check", "repeat"})

    def test_unit_tests_do_not_remove_a_products_lanes_or_a_conservative_fallback(self):
        for product in ("web/src/terminals.ts", "desktop/src/main/index.ts", "desktop/src/main/updater.ts", "hide-platform/src/lib.rs",
                        "Cargo.lock", "scripts/ci-plan.py", "unclaimed/config.json"):
            with self.subTest(product=product):
                # The changed unit test adds the lane that repeats it, beside
                # a product that plans every lane too: the pull request's diff
                # still names the test.
                base = plan(product)
                expected = set(base["lanes"]) | {"checks", "repeat"}
                self.assertEqual(set(plan("desktop/src/main/new.test.ts", product)["lanes"]), expected | {"windows-check"})
        self.assertIn("package", plan("desktop/src/main/updater.ts")["lanes"])
        self.assertEqual(plan("web/src/new.test.ts", status="R100")["lanes"], FULL)
        for path in ("desktop/src/main/new.test.tsx", "web/e2e/new.unit.ts", "desktop/e2e/new.unit.ts"):
            with self.subTest(unregistered=path):
                self.assertIn("desktop-e2e", plan(path)["lanes"])
        self.assertEqual(ci.select([("M", "web/src/new.test.ts")], CRATES, ROOT,
                                   full_reason="push to main")["lanes"], FULL)

    def test_documentation_runs_only_the_policy_checks(self):
        result = plan("docs/ARCHITECTURE.md", "CONTRIBUTING.md", "desktop/AGENTS.md", "design/hide-ui.lib.pen")
        self.assertEqual(result["lanes"], ["policy"])
        self.assertEqual(result["rust_packages"], [])

    def test_a_file_another_lane_reads_plans_that_lane(self):
        # The agent notes and the daemon's text are held by `check-shell-structure.py`
        # in `policy`; no Rust test reads them, so hide-core is not built for them.
        for path in ("AGENTS.md", "CLAUDE.md"):
            with self.subTest(path=path):
                notes = plan(path)
                self.assertEqual(notes["lanes"], ["policy"])
                self.assertEqual(notes["rust_packages"], [])
        self.assertNotIn("hide-core", plan("hided/src/core.rs")["rust_packages"])
        self.assertTrue({"web-e2e", "windows-e2e"} <= set(plan("desktop/src/main/wirePath.ts")["lanes"]))
        self.assertTrue({"checks", "web-e2e"} <= set(plan("design/tokens.json")["lanes"]))
        # The README e2e spec reads the README, its logo and its hero picture.
        for path in ("README.md", "design/brand/hide-icon-blinking.svg", "site/public/assets/video/kick-conductor.gif"):
            with self.subTest(path=path):
                self.assertIn("web-e2e", plan(path)["lanes"])
        # A guide a test reads is more than documentation.
        self.assertEqual(plan("docs/UI_BEHAVIOR.md")["lanes"], ["policy", "checks"])
        guide = plan("docs/AI_PROVIDERS.md")
        self.assertEqual(guide["lanes"], ["policy", "rust"])
        self.assertEqual(guide["rust_packages"], ["hide-agent-adapter"])
        self.assertEqual(plan("web/src/assets/agents/manifest.json")["rust_packages"], ["hide-agent-adapter"])

    def test_a_web_display_change_runs_both_suites_of_the_shell_and_skips_windows(self):
        result = plan("web/src/components/ui/button.tsx", "web/src/Overview.tsx")
        # The desktop suite drives the same shell inside the app; no Rust test
        # reads the shell's sources, so no Rust lane builds for it.
        self.assertEqual(set(result["lanes"]), {"policy", "checks", "docs", "web-e2e", "desktop-e2e"})
        self.assertEqual(result["rust_packages"], [])
        # The phone page is served by hided, and the app never loads it.
        for path in ("web/mobile.html", "web/public/m/sw.js"):
            with self.subTest(path=path):
                self.assertEqual(set(plan(path)["lanes"]), {"policy", "checks", "web-e2e"})

    def test_a_shell_change_that_broke_a_desktop_spec_on_main_plans_the_desktop_suite(self):
        # Issue #1288: #1106's run left `desktop-e2e` out, and main's full run
        # failed desktop/e2e/session-work-ledger.spec.ts, which read the
        # session record it changed.
        result = plan("web/src/SessionRecord.tsx", "web/src/sessionWork.ts", "hide-session/src/conversation_cursor.rs",
                      "docs/UI_BEHAVIOR.md")
        self.assertFalse(result["full"])
        self.assertIn("desktop-e2e", result["lanes"])

    def test_a_crate_only_change_leaves_the_desktop_suite_to_mains_full_run(self):
        # The accepted risk: #1147 changed only hide-core and broke
        # desktop/e2e/right-lane.spec.ts, which main's full run found.
        result = plan("hide-core/src/runtime/workspace_view.rs", "hide-core/src/runtime/tests/workspace_view.rs",
                      "web/e2e/s3.spec.ts", "docs/UI_BEHAVIOR.md")
        self.assertFalse(result["full"])
        self.assertIn("web-e2e", result["lanes"])
        self.assertNotIn("desktop-e2e", result["lanes"])
        for event in ("push", "schedule", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertIn("desktop-e2e", ci.plan(event, None, None, ROOT)["lanes"])

    def test_web_code_the_desktop_host_reads_reaches_desktop_and_windows(self):
        for path in ("web/src/shortcuts.ts", "web/src/host.ts", "web/src/store.ts", "web/src/keys.ts"):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertTrue({"checks", "web-e2e", "desktop-e2e", "windows-e2e"} <= lanes)
                self.assertNotIn("windows-check", lanes)
                self.assertNotIn("remote-mailbox", lanes)

    def test_a_platform_spec_keeps_its_platform_lanes_on_linux_and_windows(self):
        lanes = set(plan("web/e2e/s3.spec.ts")["lanes"])
        self.assertEqual(lanes, {"policy", "checks", "web-e2e", "windows-e2e", "repeat"})
        lanes = set(plan("web/e2e/new-tab.spec.ts")["lanes"])
        self.assertEqual(lanes, {"policy", "checks", "web-e2e", "repeat"})

    def test_the_remote_mailbox_lane_follows_the_crates_it_builds_and_tests(self):
        # It builds hided, hide, the agent hooks and the host helper and runs
        # hided's remote_delivery test, so a change to one of those crates
        # or to a crate they depend on plans it.
        for path in (
            "hide-core/src/lib.rs", "hided/src/lib.rs", "hide-agent-hooks/src/lib.rs", "hide-host/src/lib.rs",
            "hide-platform/src/process.rs", "hide-kit/src/lib.rs", "hide-session/src/lib.rs", "hided/tests/it/remote_delivery.rs",
            "hide-node/src/ssh/bootstrap.rs", "hide-node-link/src/device.rs", "hided/tests/it/system_ssh.rs",
        ):
            with self.subTest(path=path):
                self.assertIn("remote-mailbox", plan(path)["lanes"])
        # The web shell, its specs and the desktop host do not reach it.
        for path in (
            "web/e2e/s3.spec.ts", "web/e2e/new-tab.spec.ts", "web/src/host.ts", "web/src/Overview.tsx", "web/playwright.config.ts",
            "web/e2e/pane-fixture.ts", "desktop/src/main/wirePath.ts", "desktop/src/preload/index.ts", "docs/TESTING.md",
        ):
            with self.subTest(path=path):
                self.assertNotIn("remote-mailbox", plan(path)["lanes"])
        # Every workspace crate is built into one of the four binaries or one
        # they depend on, so each crate's change plans the lane.
        for directory in CRATES:
            with self.subTest(crate=directory):
                self.assertIn("remote-mailbox", plan(f"{directory}/src/lib.rs")["lanes"])
        self.assertIn("remote-mailbox", plan(".github/workflows/pr.yml")["lanes"])

    def test_the_remote_mailbox_lane_follows_the_crates_it_builds_and_tests(self):
        for path in (
            "hide-core/src/lib.rs", "hided/tests/it/remote_delivery.rs", "hided/src/main.rs",
            "hide-host/src/lib.rs", "hide-agent-hooks/src/lib.rs", "hide-platform/src/process.rs",
            "hide-session/src/lib.rs", "hide-ai/src/lib.rs",
        ):
            with self.subTest(path=path):
                self.assertIn("remote-mailbox", plan(path)["lanes"])
        # Web and desktop files and documentation cannot
        # change it.
        for path in (
            "web/e2e/s3.spec.ts", "web/e2e/new-tab.spec.ts", "web/src/host.ts", "web/src/Overview.tsx",
            "web/playwright.config.ts", "desktop/src/main/wirePath.ts", "desktop/src/preload/index.ts",
            "docs/TESTING.md",
        ):
            with self.subTest(path=path):
                self.assertNotIn("remote-mailbox", plan(path)["lanes"])
        # A change no rule claims plans every lane, this one included.
        self.assertIn("remote-mailbox", plan("scripts/verify-cargo.sh")["lanes"])

    def test_the_arm64_mailbox_lane_follows_the_device_link_and_its_kit_contract(self):
        for path in (
            "hide-node/src/ssh/kit/cache.rs", "hide-node/src/ssh/bootstrap.rs", "hide-node-link/src/device.rs",
            "hided/tests/it/device_own_engine.rs", "contracts/device-kits.json",
        ):
            with self.subTest(path=path):
                self.assertIn("remote-mailbox-arm", plan(path)["lanes"])
        # What the x86_64 lane builds besides the device link is its to catch.
        for path in (
            "hide-core/src/lib.rs", "hided/src/lib.rs", "hide-platform/src/process.rs", "hide-host/src/lib.rs",
            "hided/tests/it/remote_delivery.rs", "web/src/host.ts", "docs/TESTING.md",
        ):
            with self.subTest(path=path):
                self.assertNotIn("remote-mailbox-arm", plan(path)["lanes"])
        for event in ("push", "schedule", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertIn("remote-mailbox-arm", ci.plan(event, None, None, ROOT)["lanes"])
        self.assertIn("runner: ubuntu-24.04-arm", pr_jobs()["remote-mailbox-arm"])

    def test_the_device_kit_contract_plans_what_hide_node_plans_and_the_scripts_that_read_it(self):
        contract = plan("contracts/device-kits.json")
        self.assertFalse(contract["full"])
        node = plan("hide-node/src/ssh/kit/contract.rs")
        self.assertEqual(contract["rust_packages"], node["rust_packages"])
        # The desktop unit suite runs the packaging scripts' tests, and the
        # package lane carries the kits. The contract file holds no test, so
        # it plans no repeat of one; the Rust file that reads it does.
        self.assertEqual(set(contract["lanes"]), (set(node["lanes"]) | {"checks", "package"}) - {"repeat"})
        self.assertIn("repeat", node["lanes"])

    def test_the_app_identity_contract_plans_what_hide_kit_plans_and_the_package_that_signs_with_it(self):
        contract = plan("contracts/app-identity.json")
        self.assertFalse(contract["full"])
        kit = plan("hide-kit/src/app_identity.rs")
        self.assertEqual(contract["rust_packages"], kit["rust_packages"])
        # The desktop unit suite runs the script that reads it, and the
        # package signs the core and the pane holder under the name it states.
        self.assertEqual(set(contract["lanes"]), (set(kit["lanes"]) | {"checks", "package"}) - {"repeat"})
        # Only the Mac package signs with it and reads it back.
        for path in ("contracts/app-identity.json", "desktop/scripts/app-identity.mjs"):
            with self.subTest(path=path):
                self.assertTrue(pr_plan(path)["macos"]["runs"])

    def test_the_peer_link_contract_plans_what_the_crate_that_includes_it_plans(self):
        contract = plan("contracts/peer-link.json")
        self.assertFalse(contract["full"])
        protocol = plan("hide-peer-protocol/src/hello.rs")
        self.assertEqual(contract["rust_packages"], protocol["rust_packages"])
        # The daemon links it through hide-peer, so its lanes run too.
        self.assertLessEqual({"hide-peer-protocol", "hide-peer", "hide-kit", "hided"}, set(contract["rust_packages"]))
        self.assertEqual(set(contract["lanes"]), set(protocol["lanes"]) - {"repeat"})

    def test_licenses_and_advisories_are_checked_where_a_dependency_changed_and_nightly(self):
        for path in ("Cargo.lock", "deny.toml", "scripts/install-cargo-deny.sh"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertTrue(result["deny"])
                # The Rust lane, which runs the check, is planned with it.
                self.assertIn("rust", result["lanes"])
        for path in ("hide-peer/src/peers.rs", "hide-peer-iroh/Cargo.toml", ".github/workflows/pr.yml", "docs/BUILD.md"):
            with self.subTest(path=path):
                self.assertFalse(plan(path)["deny"])
        # A new advisory is the nightly's to find; it never stops main.
        self.assertFalse(ci.plan("push", None, None, ROOT)["deny"])
        for event in ("schedule", "workflow_dispatch"):
            with self.subTest(event=event):
                self.assertTrue(ci.plan(event, None, None, ROOT)["deny"])
                self.assertTrue(ci.plan(event, None, None, ROOT, nightly_agent_checks=True)["deny"])
        self.assertIn("deny: ${{ needs.plan.outputs.deny }}", pr_jobs()["rust"])
        lane = (ROOT / ".github/workflows/rust.yml").read_text()
        self.assertIn("DENY: ${{ inputs.deny }}", lane)
        self.assertEqual(lane.count("if: env.DENY == 'true'"), 2)

    def test_the_conpty_pin_runs_the_kit_contract_test_that_holds_the_windows_kit_to_it(self):
        pin = plan("contracts/conpty.json")
        self.assertFalse(pin["full"])
        self.assertIn("rust", pin["lanes"])
        self.assertIn("hide-node", pin["rust_packages"])
        # Its own lanes stay: the holder's Windows tests and the package.
        self.assertLessEqual({"vt", "package", "windows-e2e", "holder-restart"}, set(pin["lanes"]))

    def test_the_wsl_lane_follows_the_crates_the_wsl_link_crosses(self):
        for path in (
            "hide-platform/src/wsl.rs", "hide-node/src/ssh/wsl.rs", "hide-node-link/src/device.rs",
            "hide-host/src/serve.rs", "hided/src/node_cli.rs", "hided/tests/it/wsl_device.rs",
            "hided/tests/it/windows_ssh_device.rs", ".github/workflows/wsl-device.yml",
            "hide-kit/src/cli_windows.rs", "hide-kit/src/current.rs",
        ):
            with self.subTest(path=path):
                self.assertIn("windows-wsl", plan(path)["lanes"])
        # The core's WSL decisions have unit tests of their own; the daemon,
        # the web shell and documentation do not reach the link.
        for path in (
            "hide-core/src/runtime/hosts.rs", "hided/src/lib.rs", "web/src/settings/AddDevice.tsx", "docs/TESTING.md",
            "hide-kit/src/agents.rs", "hide-kit/src/cli.rs", "hide-kit/src/device.rs",
        ):
            with self.subTest(path=path):
                self.assertNotIn("windows-wsl", plan(path)["lanes"])

    def test_desktop_changes_run_the_desktop_lanes(self):
        result = plan("desktop/src/preload/index.ts")
        self.assertEqual(set(result["lanes"]), {"policy", "checks", "desktop-e2e", "windows-desktop"})
        self.assertIn("windows-check", plan("desktop/src/main/index.ts")["lanes"])

    def test_a_desktop_spec_reaches_windows_only_when_it_checks_what_differs_by_system(self):
        self.assertEqual(set(plan("desktop/e2e/desktop.spec.ts")["lanes"]), {"policy", "checks", "desktop-e2e", "windows-desktop"})
        self.assertEqual(set(plan("desktop/e2e/keys.spec.ts")["lanes"]), {"policy", "checks", "desktop-e2e"})
        self.assertIn("windows-desktop", plan("desktop/e2e/gone.spec.ts", status="D")["lanes"])

    def test_the_windows_desktop_lane_follows_what_differs_by_system(self):
        for path in ("hided/src/lib.rs", "hide-platform/src/process.rs", "hide-kit/src/lib.rs", "web/src/host.ts", "web/e2e/pane-fixture.ts"):
            with self.subTest(path=path):
                self.assertIn("windows-desktop", plan(path)["lanes"])
        for path in ("web/src/Overview.tsx", "hide-session/src/lib.rs", "web/e2e/new-tab.spec.ts", "docs/TESTING.md"):
            with self.subTest(path=path):
                self.assertNotIn("windows-desktop", plan(path)["lanes"])

    def test_relay_wire_and_its_build_and_process_owner_run_interoperability(self):
        for path in ("hide-relay-protocol/src/noise.rs", "hide-relay-protocol/tests/cross-wasm.ts",
                     "hide-relay/src/server.rs", "hide-relay/tests/it/relay.rs",
                     "scripts/build-relay-wasm.sh", "scripts/check-relay-protocol.sh", "web/e2e/worker-owned.ts"):
            with self.subTest(path=path):
                self.assertIn("relay-protocol", plan(path)["lanes"])
        self.assertNotIn("relay-protocol", plan("web/src/Overview.tsx")["lanes"])

    def test_a_platform_crate_reaches_every_os_and_its_consumers(self):
        result = plan("hide-platform/src/process.rs")
        self.assertTrue(
            {"rust", "os-contract", "os-contract-macos", "windows-check", "windows-e2e", "web-e2e", "remote-mailbox"}
            <= set(result["lanes"])
        )
        self.assertNotIn("desktop-e2e", result["lanes"])
        # The data-only adapter and the screen rules it compiles, the
        # text-only shell reader, the terminal parser and what two machines
        # say to each other are independent, and workspace-hack depends on no
        # member; every other crate consumes the platform layer.
        self.assertEqual(result["rust_packages"],
                         [name for name in EVERY_PACKAGE
                          if name not in ("hide-agent-adapter", "hide-screen-rules", "hide-shell", "hide-vt", "hide-relay", "hide-relay-protocol", "hide-peer-protocol",
                                          "workspace-hack")])
        self.assertFalse(result["full"])

    def test_an_adapter_change_tests_its_real_consumers_and_compiles_on_windows(self):
        # These consumers follow the workspace manifests, independently of the planner.
        consumers = [
            "hide-agent-adapter", "hide-agent-hooks", "hide-ai", "hide-core", "hide-desk", "hide-holder", "hide-host",
            "hide-kit", "hide-memory", "hide-node", "hide-node-link", "hide-peer", "hide-peer-iroh", "hide-session", "hided",
        ]
        for path in ("hide-agent-adapter/src/lib.rs", "hide-agent-adapter/Cargo.toml"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(result["rust_packages"], consumers)
                self.assertTrue({"rust", "windows-check", "web-e2e", "remote-mailbox"} <= set(result["lanes"]))
                self.assertFalse(set(result["lanes"]) & {"os-contract", "os-contract-macos", "windows-e2e", "desktop-e2e"})
                self.assertFalse(result["full"])

    def test_a_leaf_crate_tests_its_reverse_dependencies_and_compiles_on_windows(self):
        result = plan("hide-session/src/lib.rs")
        self.assertIn("hide-session", result["rust_packages"])
        self.assertIn("hided", result["rust_packages"])
        self.assertNotIn("hide-platform", result["rust_packages"])
        self.assertIn("windows-check", result["lanes"])
        self.assertNotIn("windows-e2e", result["lanes"])
        self.assertNotIn("os-contract", result["lanes"])
        self.assertEqual(plan("hided/tests/it/handshake.rs")["rust_packages"], ["hided"])

    def test_the_vt_lane_follows_the_parser_and_the_pane_holder(self):
        # vt.yml builds hide-vt and runs the holder's tests, which start the
        # holder from a version folder hide-kit makes and through the pane
        # records hide-kit keeps.
        for path in ("hide-vt/build.rs", "hide-holder/src/holder.rs", "hide-holder-protocol/src/lib.rs",
                     "hide-kit/src/holders.rs", "hide-kit/src/panes.rs",
                     "vendor/portable-pty/src/win/psuedocon.rs"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertIn("vt", result["lanes"])
                self.assertFalse(result["full"])
        self.assertIn("hide-holder", plan("vendor/portable-pty/Cargo.toml")["rust_packages"])
        # The web suite's own-engine job runs each pane in a holder, which
        # hided does not link, so the holder's crates reach the web suite.
        for path in ("hide-holder/src/holder.rs", "hide-holder-protocol/src/lib.rs", "hide-vt/build.rs",
                     "vendor/portable-pty/src/unix.rs"):
            with self.subTest(path=path):
                self.assertIn("web-e2e", plan(path)["lanes"])
        # The platform layer's own contract covers the resident start and the
        # session end; its every change does not wait for the Zig build.
        for path in ("hide-platform/src/process/resident.rs", "hide-kit/src/layout.rs"):
            with self.subTest(path=path):
                self.assertNotIn("vt", plan(path)["lanes"])

    def test_the_macos_os_contract_runs_for_the_crate_it_tests(self):
        for path in ("hide-platform/src/process.rs",):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertTrue({"os-contract", "os-contract-macos"} <= lanes)

    def test_a_core_or_daemon_change_asks_for_no_macos_runner_and_a_shell_change_for_the_desktop_suite(self):
        # Their OS differences are checked on Linux and Windows; main's full
        # run runs the desktop suite for them, and the nightly the macOS leg
        # and the macOS `@platform` tests.
        for path in ("hide-core/src/lib.rs", "hided/src/lib.rs", "hide-host/src/lib.rs", "hide-session/src/lib.rs"):
            with self.subTest(path=path):
                self.assertEqual(set(plan(path)["lanes"]) & MACOS_LANES, set())
        self.assertEqual(set(plan("web/src/Overview.tsx")["lanes"]) & MACOS_LANES, {"desktop-e2e"})
        # What only the web suite runs asks for no macOS runner.
        for path in ("web/e2e/s3.spec.ts", "web/playwright.config.ts", "web/mobile.html"):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertFalse(lanes & MACOS_LANES, lanes & MACOS_LANES)
        self.assertEqual(set(plan("hide-core/src/lib.rs")["lanes"]) & {"os-contract", "windows-e2e"}, {"os-contract", "windows-e2e"})

    def test_what_goes_into_a_package_plans_the_package_lane_and_nothing_else_does(self):
        # The kit and hooks a packaged daemon runs reach the packaged app only through it.
        for path in PACKAGE_INPUTS:
            with self.subTest(path=path):
                self.assertIn("package", plan(path)["lanes"])
        for path in (
            "hided/src/lib.rs", "hided/src/core.rs", "hide-core/src/lib.rs", "hide-platform/src/process.rs",
            "desktop/src/main/index.ts", "desktop/e2e/fixture.ts", "web/src/store.ts", "docs/TESTING.md",
            "Cargo.lock", "pnpm-lock.yaml",
        ):
            with self.subTest(path=path):
                self.assertNotIn("package", plan(path)["lanes"])
        self.assertEqual(set(plan("hide-kit/src/lib.rs")["lanes"]) & MACOS_LANES, {"package"})
        self.assertEqual(plan(".github/workflows/package.yml")["lanes"], ALL)
        self.assertEqual(plan(".github/workflows/device-helpers.yml")["lanes"], ALL)

    def test_each_macos_lane_keeps_its_inputs_for_added_modified_and_deleted_paths(self):
        # Expected coverage from the lane commands and approved crate-only rule,
        # independent of the planner's map. Other operating systems stay planned.
        cases = (
            ("docs/TESTING.md", set()),
            ("hide-core/src/lib.rs", set()),
            ("web/mobile.html", set()),
            ("web/public/m/sw.js", set()),
            ("web/e2e/new-tab.spec.ts", set()),
            ("web/src/Overview.tsx", {"desktop-e2e"}),
            ("web/e2e/pane-fixture.ts", {"desktop-e2e"}),
            ("desktop/src/main/index.ts", {"desktop-e2e"}),
            ("desktop/src/main/updater.ts", {"desktop-e2e", "package"}),
            ("desktop/src/main/updater.test.ts", set()),
            ("desktop/scripts/installers.mjs", {"package"}),
            ("hide-kit/src/lib.rs", {"package"}),
            ("hide-ai/src/omp.rs", {"omp-contract"}),
            ("hide-platform/src/process.rs", {"os-contract-macos", "omp-contract"}),
        )
        for status in ("A", "M", "D"):
            for path, expected in cases:
                with self.subTest(status=status, path=path):
                    result = plan(path, status=status)
                    self.assertEqual(set(result["lanes"]) & MACOS_LANES, expected)
                    ci.aggregate(needs_for(result))
                    for lane in expected:
                        for outcome in ("skipped", "failure", "cancelled"):
                            with self.assertRaises(ValueError):
                                ci.aggregate(needs_for(result, **{lane: outcome}))

    def test_package_helpers_need_only_checks_and_the_package_chain(self):
        for status in ("A", "M", "D"):
            for name in ("app-identity.mjs", "build-identity.mjs", "device-helpers.mjs", "installers.mjs",
                         "macos-signing.mjs", "release-binaries.mjs", "smoke-windows-installer.mjs"):
                with self.subTest(status=status, helper=name):
                    result = plan("desktop/scripts/" + name, status=status)
                    self.assertFalse(result["full"])
                    self.assertEqual(result["lanes"], ["policy", "checks", "package"])
                    ci.aggregate(needs_for(result))
                    for outcome in ("skipped", "failure", "cancelled"):
                        with self.assertRaises(ValueError):
                            ci.aggregate(needs_for(result, package=outcome))
        # New helper names stay conservative until their readers are established.
        self.assertEqual(plan("desktop/scripts/new-helper.mjs")["lanes"], PACKAGED)
        self.assertEqual(plan("desktop/scripts/installers.mjs", "hide-core/src/lib.rs")["lanes"],
                         ["policy", "rust", "checks", "os-contract", "agent-journey", "windows-check", "windows-e2e",
                          "windows-desktop", "web-e2e", "remote-mailbox", "package"])

    def test_fuses_keep_the_desktop_suites_that_apply_them_in_global_setup(self):
        for status in ("A", "M", "D"):
            for name in ("fuses.mjs", "fuses.d.mts"):
                with self.subTest(status=status, name=name):
                    self.assertEqual(plan("desktop/scripts/" + name, status=status)["lanes"],
                                     ["policy", "checks", "windows-desktop", "desktop-e2e", "package"])

    def test_e2e_documentation_planning_and_green_reuse_agree(self):
        for status in ("A", "M", "D"):
            for name in ("desktop/e2e/README.md", "web/e2e/README.md", "web/e2e/shims/notes.md"):
                with self.subTest(status=status, name=name):
                    self.assertEqual(plan(name, status=status)["lanes"], ["policy"])
                    self.assertTrue(ci.reads_no_lane(status, name, CRATES, ROOT))

    def test_the_desktop_lane_follows_what_the_app_runs(self):
        for path in (
            "desktop/src/main/index.ts", "desktop/static/icon.png", "desktop/e2e/desktop.spec.ts", "web/src/shortcuts.ts",
            "web/src/Overview.tsx", "web/index.html", "web/e2e/pane-fixture.ts",
        ):
            with self.subTest(path=path):
                self.assertIn("desktop-e2e", plan(path)["lanes"])
        # A crate the app runs is left to main's full run.
        for path in (
            "web/e2e/new-tab.spec.ts", "web/playwright.config.ts", "web/mobile.html", "web/public/m/manifest.webmanifest",
            "docs/TESTING.md", "design/tokens.json",
            "hide-core/src/lib.rs", "hided/src/lib.rs", "hide-platform/src/process.rs", "hide-session/src/lib.rs",
        ):
            with self.subTest(path=path):
                self.assertNotIn("desktop-e2e", plan(path)["lanes"])

    def test_the_whole_plan_of_each_kind_of_change(self):
        # The desktop suite waits in the macOS queue, so a crate-only pull
        # request leaves it to main's full run; anything under desktop/ or the
        # web shell still plans it, and every full plan keeps it.
        crate_only = ["policy", "rust", "os-contract", "agent-journey", "windows-check", "windows-e2e", "windows-desktop",
                      "web-e2e", "remote-mailbox"]
        for paths, lanes in (
            (("hide-core/src/lib.rs",), crate_only),
            (("hide-core/src/lib.rs", "desktop/src/main/index.ts"),
             ["policy", "rust", "checks", "os-contract", "agent-journey", "windows-check", "windows-e2e",
              "windows-desktop", "web-e2e", "remote-mailbox", "desktop-e2e"]),
            (("web/src/Overview.tsx",), ["policy", "checks", "docs", "web-e2e", "desktop-e2e"]),
            (("docs/TESTING.md",), ["policy"]),
            (("mystery/file.txt",), FULL),
            (("desktop/scripts/package.mjs", "mystery/file.txt"), PACKAGED),
        ):
            with self.subTest(paths=paths):
                self.assertEqual(plan(*paths)["lanes"], lanes)
        for event in ("push", "schedule", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                self.assertEqual(ci.plan(event, None, None, ROOT)["lanes"], FULL)

    def test_shared_unknown_and_unsafe_changes_run_everything(self):
        for entries in (
            [("M", ".github/workflows/pr.yml")],
            [("M", "scripts/verify-web.sh")],
            [("M", "Cargo.lock")],
            [("M", "pnpm-lock.yaml")],
            [("M", "contracts/delivery-envelope.json")],
            [("M", "web/package.json")],
            [("M", "scripts/design-scratch.mjs")],
            [("M", "mystery/file.txt")],
            [("T", "web/src/store.ts")],
            [("M", "docs/README.md"), ("M", "scripts/ci-plan.py")],
            [],
        ):
            with self.subTest(entries=entries):
                result = ci.select(entries, CRATES, ROOT)
                self.assertTrue(result["full"])
                package = any(path in PACKAGE_INPUTS for _, path in entries)
                ci_change = any(path.startswith(".github/workflows/") or path == "scripts/ci-plan.py" for _, path in entries)
                self.assertEqual(result["lanes"], ALL if ci_change else PACKAGED if package else FULL)
                self.assertEqual(result["rust_packages"], EVERY_PACKAGE)

    def test_a_push_and_a_failed_comparison_run_everything(self):
        self.assertEqual(ci.plan("push", None, None, ROOT)["lanes"], FULL)
        result = ci.plan("pull_request", "0" * 40, "HEAD", ROOT)
        self.assertTrue(result["full"])
        self.assertIn("comparison unavailable", result["reasons"]["rust"][0])

    def test_a_merge_group_runs_the_fast_checks_and_nothing_else(self):
        # What two changes that each passed break together without a conflict
        # is a compile, type or baseline failure: a group runs those checks only,
        # and main's push run after the merge runs every lane.
        result = ci.plan("merge_group", None, None, ROOT)
        self.assertEqual(result["lanes"], ["policy", "merge-rust", "merge-web"])
        self.assertFalse(result["macos"]["runs"])
        ci.aggregate(needs_for(result))
        with self.assertRaisesRegex(ValueError, "merge-web: planned, expected success, got failure"):
            ci.aggregate(needs_for(result, merge_web="failure"))
        with self.assertRaisesRegex(ValueError, "rust: not planned, expected skipped"):
            ci.aggregate(needs_for(result, rust="success"))
        # No other run plans the group's own checks, a change to CI included.
        for result in (ci.plan("push", None, None, ROOT), ci.select([("M", ".github/workflows/pr.yml")], CRATES, ROOT)):
            self.assertFalse({"merge-rust", "merge-web"} & set(result["lanes"]))
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        self.assertRegex(workflow[:workflow.index("\njobs:\n")], r"\n  merge_group:\n")

    def test_nightly_owns_the_agent_checks_once_and_keeps_failure_reporting(self):
        # The default remains a full plan. Only the explicit caller contract
        # removes the internal duplicates; the direct jobs remain mandatory.
        self.assertEqual(ci.plan("schedule", None, None, ROOT)["lanes"], FULL)
        for event in ("schedule", "workflow_dispatch"):
            result = ci.plan(event, None, None, ROOT, nightly_agent_checks=True)
            self.assertEqual(result["lanes"], [lane for lane in FULL if lane not in {"agy-contract", "agent-journey"}])
            ci.aggregate(needs_for(result))
            for lane in ("agy_contract", "agent_journey"):
                for outcome in ("success", "failure", "cancelled"):
                    with self.subTest(event=event, lane=lane, outcome=outcome):
                        with self.assertRaisesRegex(ValueError, "expected skipped"):
                            ci.aggregate(needs_for(result, **{lane: outcome}))
            with self.assertRaisesRegex(ValueError, "expected success"):
                ci.aggregate(needs_for(result, web_e2e="failure"))
        for event in ("pull_request", "push"):
            with self.assertRaisesRegex(ValueError, "only nightly"):
                ci.plan(event, None, None, ROOT, nightly_agent_checks=True)
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        self.assertRegex(workflow, r"\n  workflow_call:\n")
        self.assertIn('--nightly-agent-checks "${{ inputs.nightly-agent-checks || false }}"', workflow)
        # A called run takes the caller's name, so it never takes a push run's
        # pending place in the `verify-refs/heads/main` group.
        self.assertIn("group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}", workflow)
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        call = nightly[nightly.index("\n  verify:\n"):nightly.index("\n  package:\n")]
        self.assertIn("uses: ./.github/workflows/pr.yml", call)
        self.assertIn("nightly-agent-checks: true", call)
        # Its lanes file flaky tests with the development issues App's key.
        self.assertIn("HIDE_ISSUES_APP_KEY: ${{ secrets.HIDE_ISSUES_APP_KEY }}", call)
        report = nightly[nightly.index("\n  report:\n"):]
        for job in ("verify", "agy-contract-all", "agent-journey"):
            self.assertRegex(report, rf"needs: \[[^\]]*\b{job}\b[^\]]*\]")
        self.assertIn("always() && github.ref == 'refs/heads/main'", report)
        # A failed direct job fails the workflow; neither call may tolerate
        # failure. Every dispatch choice retains exactly its direct checks.
        for name in ("agy-contract-all", "agent-journey"):
            block = nightly[nightly.index(f"\n  {name}:\n"):].split("\n\n")[0]
            self.assertNotIn("continue-on-error", block)
            # Its systems are the run's scope's (nightly-scope).
            self.assertIn("systems: ${{ needs.scope.outputs.systems }}", block)
            if name == "agent-journey":
                self.assertIn("fake: true", block)
                self.assertNotIn("stand-ins: false", block)

    def test_a_draft_plans_no_lane_and_ready_for_review_runs_them(self):
        result = ci.plan("pull_request", "0" * 40, "HEAD", ROOT, draft=True)
        self.assertEqual(result["lanes"], [])
        self.assertEqual(result["rust_packages"], [])
        self.assertIn("draft", ci.summary(result))
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        # Without the `ready_for_review` run nothing replaces the draft's `verify`.
        self.assertRegex(workflow, r"types: \[[^\]]*\bready_for_review\b[^\]]*\]")
        self.assertIn('--draft "${{ github.event.pull_request.draft || false }}"', workflow)
        # `verify` runs on a draft and fails there: a skipped one would count as
        # passed for the minutes before the ready run's `verify` starts.
        verify = workflow[workflow.index("\n  verify:\n"):]
        self.assertIn("    if: always()\n", verify)
        self.assertNotIn("draft", verify.split("steps:")[0])

    def test_a_missing_crate_graph_runs_everything(self):
        with tempfile.TemporaryDirectory() as directory:
            result = ci.plan("pull_request", "HEAD^1", "HEAD", Path(directory))
            self.assertEqual(result["lanes"], FULL)
            self.assertIn("crate graph unavailable", result["reasons"]["rust"][0])

    def test_a_pull_request_diff_is_read_from_git(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)
            git("init", "-q")
            git("config", "user.email", "ci@example.invalid")
            git("config", "user.name", "ci")
            (repo / "docs").mkdir()
            (repo / "docs/a.md").write_text("a\n")
            git("add", ".")
            git("commit", "-qm", "base")
            (repo / "docs/a.md").write_text("b\n")
            (repo / "docs/b.md").write_text("new\n")
            git("add", ".")
            git("commit", "-qm", "head")
            self.assertEqual(ci.changed_entries("HEAD^1", "HEAD", repo), [("M", "docs/a.md"), ("A", "docs/b.md")])
            # Not the merge commit a pull request checks out: every lane.
            self.assertTrue(ci.plan("pull_request", "HEAD^1", "HEAD", repo, CRATES)["full"])
            git("checkout", "-qb", "topic", "HEAD^1")
            (repo / "docs/c.md").write_text("c\n")
            git("add", ".")
            git("commit", "-qm", "topic")
            git("checkout", "-q", "-")
            git("merge", "-q", "--no-ff", "-m", "merge", "topic")
            result = ci.plan("pull_request", "HEAD^1", "HEAD", repo, CRATES)
            self.assertEqual(result["lanes"], ["policy"])


# The retired-name check and its list, found by their naming convention.
NAMES_GUARD = tuple(sorted(
    found.relative_to(ROOT).as_posix()
    for pattern in ("check-*-names.py", "*-names-allowlist.json") for found in (ROOT / "scripts").glob(pattern)
))


class NamedPaths(unittest.TestCase):
    """The paths `ci-plan.py` names; every other path still plans every lane."""

    def lanes(self, *paths):
        return set(plan(*paths)["lanes"])

    def test_paths_no_lane_reads_plan_policy_alone(self):
        self.assertEqual(len(NAMES_GUARD), 2, NAMES_GUARD)
        for path in (
            "agents/prd/x/prd.md",
            ".gitignore", "web/.gitignore", "LICENSE", "NOTICE", ".github/pull_request_template.md", ".github/dependabot.yml",
            ".github/ISSUE_TEMPLATE/bug.yml", ".github/DISCUSSION_TEMPLATE/ideas.yml", ".github/APPROVED_CONTRIBUTORS",
            "scripts/tests/test_ci_plan.py", "scripts/check-harness-ignore-anchor.sh",
            "scripts/web-shell-measure/run.sh",
            # The design contract's generators and checks, which `design-contract.yml` runs.
            "scripts/gen-tokens.mjs",
            "scripts/check-design-contract.mjs", "scripts/check-web-tokens.mjs", "scripts/check-agent-logos.mjs",
            "scripts/merge-order.py", "scripts/pull_requests.py", "scripts/merge-hide-cli-contract.py",
            "contracts/README.md",
            # `check-shell-structure.py` and the checks `policy` and `design-contract.yml` run.
            "scripts/check-shell-structure.py",
            "scripts/check-ui-parts.mjs", "scripts/ui-parts-baseline.json", "mobile/.gitignore",
            # The release checklist's generator; the upgrade tests read what it writes, not it.
            "scripts/release_state/generate.py", "scripts/release_state/scenario.py", "scripts/release_state/Dockerfile",
        ) + NAMES_GUARD:
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(result["lanes"], ["policy"])
                self.assertFalse(result["full"])

    def test_the_ios_app_host_files_plan_its_unit_suite_and_nothing_else(self):
        # `verify-web.sh mobile test` is the `checks` lane's step; the Android lane builds
        # `mobile/android` alone and the iOS app is built on the Mac mini, so no other lane
        # reads the scripts, the release inputs, the Xcode project, the Swift sources or the store text.
        for path in (
            "mobile/scripts/build-ios.sh", "mobile/scripts/mini-ios.sh",
            "mobile/scripts/check-store-text.mjs", "mobile/scripts/check-versions.mjs",
            "mobile/ios/App/App/Info.plist", "mobile/ios/App/App.xcodeproj/project.pbxproj",
            "mobile/ios/App/HideShell/Sources/HideShell/BridgeGate.swift",
            "mobile/store/metadata/en-US/description.txt", "mobile/store/metadata/ko/subtitle.txt",
            "mobile/store/android/metadata/ko-KR/full_description.txt", "mobile/store/shots.json",
            "mobile/release-env.json", "mobile/release-expect.json", "mobile/release.env.example",
            "mobile/android-release-env.json", "mobile/android-release-expect.json", "mobile/android-release.env.example",
            "mobile/capacitor.config.json", "mobile/retained-bundles.json",
        ):
            with self.subTest(path=path):
                result = plan(path)
                self.assertFalse(result["full"])
                self.assertEqual(result["lanes"], ["policy", "checks"])
                self.assertEqual(result["rust_packages"], [])
        # The Play data safety answers are a document the docs lane's site tests read.
        self.assertEqual(plan("mobile/store/android/play-forms.md")["lanes"], ["policy", "docs"])
        # A changed unit test is also repeated, as everywhere.
        self.assertEqual(plan("mobile/scripts/release.test.mjs")["lanes"], ["policy", "checks", "repeat"])
        # The package manifest reaches the lock, which every lane reads.
        self.assertTrue(plan("mobile/package.json")["full"])

    def test_a_web_only_change_plans_no_rust_lane(self):
        # hide-core's tests used to read the shell, the daemon and the agent notes, so each of
        # these built the crate's whole test binary; `check-shell-structure.py` reads that text in `policy` now.
        for paths in (
            ("web/src/Overview.tsx",), ("web/src/PaneGrid.tsx",), ("web/src/terminals.ts",), ("web/src/store.ts",),
            ("web/src/releases.json",), ("web/src/practice/chapters/mission.ts",), ("web/src/new.test.ts",),
            ("web/src/components/ui/button.tsx", "web/src/sidebar.tsx", "web/src/App.test.tsx"),
        ):
            with self.subTest(paths=paths):
                result = plan(*paths)
                self.assertFalse(result["full"])
                self.assertNotIn("rust", result["lanes"])
                self.assertEqual(result["rust_packages"], [])
        # The text the check reads still plans the lane that runs it.
        for path in ("web/src/PaneGrid.tsx", "web/src/releases.json", "hided/src/core.rs", "AGENTS.md", "CLAUDE.md",
                     "hide-core/src/product_analytics.rs", "scripts/check-shell-structure.py"):
            with self.subTest(read=path):
                self.assertIn("policy", self.lanes(path))
        # The daemon's own crates still build for the daemon, and hide-core for nothing of it.
        self.assertNotIn("hide-core", plan("hided/src/core.rs")["rust_packages"])
        self.assertIn("hided", plan("hided/src/core.rs")["rust_packages"])

    def test_the_user_docs_lane_reads_the_site_the_parts_the_shell_and_the_cli_list(self):
        for path in ("site/public/index.html", "site/src/content/docs/docs/guides/x.md", "site/README.md", "site/astro.config.mjs",
                     "scripts/docs/check-all.mjs", "web/docs-embed.html", "web/docs-embed.config.ts", "web/src/docs/embed.tsx",
                     "web/src/components/session-row.tsx", "contracts/hide-cli.json", "hided/src/cli.rs"):
            with self.subTest(path=path):
                self.assertIn("docs", self.lanes(path))
        # Only the site, its scripts and the parts' build plan `docs` alone with policy.
        for path in ("site/public/index.html", "site/src/content/docs/docs/guides/x.md", "scripts/docs/lib.mjs", "web/docs-embed.html"):
            with self.subTest(path=path):
                self.assertEqual(plan(path)["lanes"], ["policy", "docs"])
        # A unit test and a crate other than the CLI's help are not what the docs build reads.
        for path in ("web/src/new.test.ts", "hide-core/src/lib.rs", "docs/UI_BEHAVIOR.md"):
            with self.subTest(path=path):
                self.assertNotIn("docs", self.lanes(path))

    def test_a_change_to_each_file_that_owns_help_text_plans_the_docs_lane(self):
        # `hide --help` is what the CLI reference reads, so a command added in any of its owners
        # needs its note in the same pull request, not first on main.
        for path in ci.HELP_SOURCES:
            with self.subTest(path=path):
                result = plan(path)
                self.assertIn("docs", result["lanes"])
                self.assertFalse(result["full"])
        # A sibling that owns no help text does not.
        for path in ("hided/src/browser_routes.rs", "hided/src/profile_cli.rs", "hided/src/server.rs"):
            with self.subTest(path=path):
                self.assertNotIn("docs", self.lanes(path))

    def test_help_text_owners_are_the_modules_cli_rs_joins(self):
        source = (ROOT / "hided/src/cli.rs").read_text()
        body = source[source.index("fn help_text()"):]
        body = body[:body.index("\n}\n")]
        joined = {f"hided/src/{name}.rs" for name in re.findall(r"crate::(\w+)::(?:USAGE\b|usage_here\(\))", body)}
        self.assertEqual(joined | {"hided/src/cli.rs"}, set(ci.HELP_SOURCES))

    def test_web_e2e_helpers_reach_the_desktop_and_windows_lanes_but_not_rust(self):
        want = {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"}
        for path in ("web/e2e/pane-fixture.ts", "web/e2e/shims/build.ts", "web/e2e/shims/noop.c", "web/e2e/test-size-baseline.json"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), want)

    def test_a_pin_or_its_installer_plans_the_lanes_that_install_pinned_clis(self):
        for path in ("contracts/agent-clis.json", "scripts/install-agent-clis.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agy-contract", "agent-clis", "holder-restart"})
        self.assertEqual(plan(".github/workflows/agent-clis.yml")["lanes"], ALL)
        self.assertEqual(self.lanes("scripts/agent_journey/clis.py"),
                         {"policy", "agent-journey", "agent-clis", "agy-contract", "holder-restart"})

    def test_the_journey_and_its_stand_ins_plan_the_journey_lane(self):
        for path in ("scripts/agent_journey/cells.py", "scripts/agent-journey-check.py", "scripts/agent-journey-lane.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agent-journey"})
        # The installer finds the contract's full-access flags in each CLI's
        # help, and the holder restart lane reads the agent table too.
        self.assertEqual(self.lanes("contracts/agent-journey.json"),
                         {"policy", "agent-journey", "agent-clis", "holder-restart"})
        # The holder restart lane reads the fake model's helpers.
        for path in ("scripts/agent_journey/fake_model.py", "scripts/agent_journey/fake_lane.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agent-journey", "holder-restart"})
        for path in ("web/e2e/shims/claude-shim.c", "web/e2e/shims/live-check.h", "web/e2e/shims/spawn-provider.c"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e",
                                                    "windows-desktop", "windows-check", "agent-journey"})
        for path in ("hide-session/examples/journey_conversation.rs", "hide-memory/examples/journey_memory.rs",
                     "hide-agent-adapter/examples/journey_composer.rs", "hide-agent-hooks/tests/fixtures/fake-codex.py"):
            with self.subTest(path=path):
                self.assertIn("agent-journey", self.lanes(path))
                self.assertIn("rust", self.lanes(path))
        # The live check the journey runs, and its Windows forms' test,
        # which only the journey's Windows job runs.
        for path in ("scripts/agent_live_check/processes.py", "scripts/agent_live_check/windows.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agent-journey", "holder-restart"})
        self.assertEqual(self.lanes("scripts/tests/test_agent_live_windows.py"), {"policy", "agent-journey"})
        self.assertEqual(self.lanes("scripts/tests/test_agent_live_protection.py"), {"policy"})

    def test_the_product_code_the_journey_runs_plans_the_journey_lane(self):
        # #1816 aged Memory cards out of the start guidance and broke
        # `guidance_memory` for every agent; its pull request never ran the
        # journey, and the nightly canary blamed five agent releases for it
        # (with-hide/hide-private#358). Every crate builds into a program the
        # journey runs, so its product code plans the lane.
        for path in (
            "hide-memory/src/store/upkeep.rs", "hide-agent-hooks/src/guidance.rs", "hide-core/src/memory_hook.rs",
            "hide-core/src/runtime/memory_analysis.rs", "hide-core/src/coordination/started.rs",
            "hide-core/src/delivery/mod.rs", "hide-core/src/agent_sleep.rs", "hide-session/src/lib.rs",
            "hide-agent-adapter/src/lib.rs", "hided/src/spawn.rs", "hided/build.rs", "hide-memory/Cargo.toml",
        ):
            with self.subTest(path=path):
                self.assertIn("agent-journey", self.lanes(path))
        for directory in CRATES:
            with self.subTest(crate=directory):
                self.assertIn("agent-journey", self.lanes(f"{directory}/src/lib.rs"))
        # A crate's tests, test modules and other examples build into none of them.
        for path in (
            "hided/tests/it/remote_delivery.rs", "hide-core/src/runtime/tests/agent_sleep.rs",
            "hide-core/src/runtime/tests.rs", "hide-core/src/checkout_start_tests.rs",
            "hide-agent-hooks/tests/fixtures/bell-prompt-identity.json", "hide-memory/benches/x.rs",
        ):
            with self.subTest(path=path):
                self.assertNotIn("agent-journey", self.lanes(path))

    def test_the_shared_prompt_identity_examples_run_the_hooks_and_policy_readers(self):
        result = plan("hide-agent-hooks/tests/fixtures/bell-prompt-identity.json")
        # The journey runs the hook program, which the test fixture is not part
        # of, and the fixture declares no test, so it repeats none: the Rust
        # file beside it does.
        self.assertEqual(set(result["lanes"]), self.lanes("hide-agent-hooks/src/delivery.rs") - {"agent-journey", "repeat"})
        self.assertIn("hide-agent-hooks", result["rust_packages"])
        self.assertIn("policy", result["lanes"])

    def test_desktop_e2e_helpers_run_the_desktop_suites_and_windows_unit_tests(self):
        for path in ("desktop/e2e/fixture.ts", "desktop/playwright.config.ts", "desktop/vitest.config.ts"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "checks", "desktop-e2e", "windows-desktop", "windows-check"})

    def test_the_narrow_unit_paths_are_exactly_the_registered_desktop_vitest_tests(self):
        # Independent authority: Vitest's actual include lists, not the selector.
        # A new test convention must be registered before its app lanes can go.
        registered = set()
        for folder, config in (("desktop", "vitest.config.ts"),):
            text = (ROOT / folder / config).read_text()
            patterns = json.loads(re.search(r"include: (\[[^\]]+\])", text).group(1))
            for pattern in patterns:
                expanded = [pattern]
                if "{ts,tsx}" in pattern:
                    expanded = [pattern.replace("{ts,tsx}", ext) for ext in ("ts", "tsx")]
                for glob in expanded:
                    registered.update(path.relative_to(ROOT).as_posix() for path in (ROOT / folder).glob(glob))
        sources = code_sources()
        selected = {path for path in sources if any(fnmatchcase(path, glob) for glob in ci.UNIT_TEST_PATHS)}
        self.assertTrue(registered)
        self.assertEqual(selected, registered)
        for path in registered:
            with self.subTest(path=path):
                # A registered desktop unit test is also a changed test, which
                # the `repeat` lane repeats.
                self.assertEqual(self.lanes(path), {"policy", "checks", "windows-check", "repeat"})
        # Both retained jobs run both Vitest suites; a configured matrix alone
        # would not establish that these tests are executed on Windows. The
        # Windows lane's suites are its called workflow's `web` job.
        windows = yaml.safe_load((ROOT / ".github/workflows/windows-check.yml").read_text())["jobs"]["web"]
        self.assertIn("uses: ./.github/workflows/windows-check.yml", pr_jobs()["windows-check"])
        for lane, text in (("checks", pr_jobs()["checks"]), ("windows-check", yaml.safe_dump(windows, width=1000))):
            for folder in ("web", "desktop"):
                with self.subTest(lane=lane, folder=folder):
                    self.assertIn(f"bash scripts/verify-web.sh {folder} test", text)

    def test_configuration_names_the_lanes_that_read_it(self):
        expected = {
            "web/playwright.config.ts": {"checks", "web-e2e", "windows-e2e"},
            "web/eslint.config.js": {"checks"},
            "web/eslint.e2e.mjs": {"checks"},
            "web/eslint-rules/hide-e2e.mjs": {"checks"},
            "web/scripts/check-e2e-test-size.mjs": {"checks"},
            "web/scripts/gen-types.mjs": {"checks", "web-e2e"},
            # Every lane that builds the shell or runs its unit suite, and none that only builds Rust.
            "web/vite.config.ts": {"checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"},
            "desktop/eslint.config.mjs": {"checks"},
            "desktop/eslint.globals.mjs": {"checks"},
            "desktop/scripts/build.mjs": {"checks", "desktop-e2e", "windows-desktop", "package"},
            "desktop/scripts/package.mjs": {"checks", "package"},
            "desktop/scripts/smoke-package.mjs": {"checks", "package"},
        }
        for path, lanes in expected.items():
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), lanes | {"policy"})

    def test_a_contract_plans_the_tests_and_the_shell_that_read_it(self):
        expected = {
            "contracts/hided-ws.schema.json": ({"policy", "rust"}, ["hide-core", "hided"]),
            "contracts/snapshot-wire-enums.json": ({"policy", "rust", "checks"}, ["hide-core"]),
            "contracts/agent-adapters.json": ({"policy", "rust", "checks", "web-e2e", "desktop-e2e"}, ["hide-agent-adapter"]),
            "contracts/hide-cli.json": ({"policy", "rust", "docs"}, ["hided"]),
            "contracts/usage-coverage.json": ({"policy", "rust"}, ["hide-agent-adapter"]),
        }
        sources = code_sources()
        for path, (lanes, packages) in expected.items():
            with self.subTest(path=path):
                result = plan(path)
                self.assertFalse(result["full"])
                self.assertEqual(set(result["lanes"]), lanes)
                self.assertEqual(result["rust_packages"], packages)
                # The claim behind it, from the sources rather than READERS:
                # the crates whose code names the file are the ones planned, a
                # web unit test that reads it brings `checks`, and web shell
                # code brings `checks` and both suites of the shell,
                # `web-e2e` and `desktop-e2e`. A reader anywhere else
                # names a lane this rule does not plan.
                read = set()
                for source, (crate, text) in sources.items():
                    if PurePosixPath(path).name not in text:
                        continue
                    if crate:
                        read |= {"rust", f"crate:{crate}"}
                    elif source.startswith("web/src/") and re.search(r"\.test\.tsx?$", source):
                        read.add("checks")
                    elif source.startswith("web/src/"):
                        read |= {"checks", "web-e2e", "desktop-e2e"}
                    elif source.startswith("scripts/docs/"):
                        # The user docs' CLI reference reads the commands the contract describes.
                        read.add("docs")
                    else:
                        read.add(f"unplanned reader: {source}")
                self.assertEqual(read, (lanes - {"policy"}) | {f"crate:{name}" for name in packages})

    def test_a_contract_no_reader_is_named_for_still_plans_every_lane(self):
        for path in ("contracts/delivery-envelope.json", "contracts/some-new-contract.json"):
            with self.subTest(path=path):
                self.assertEqual(plan(path)["lanes"], FULL)

    def test_everything_else_still_plans_every_lane(self):
        for path in (
            ".github/workflows/pr.yml",
            "scripts/ci-plan.py", "scripts/verify-cargo.sh", "scripts/verify-web.sh", "scripts/ci-flaky-report.py",
            "scripts/install-nextest.sh", "scripts/toolchain-env.sh",
            # Every member but two depends on `workspace-hack`, which holds
            # nearly every third-party crate: a lock change reaches them all.
            "pnpm-lock.yaml", "web/package.json", "desktop/package.json", "Cargo.lock", ".config/nextest.toml", "clippy.toml",
            "mystery/file.txt", "scripts/some-new-script.sh", "web/some-new-config.json",
        ):
            with self.subTest(path=path):
                self.assertTrue(plan(path)["full"])
                ci_change = path.startswith(".github/workflows/") or path == "scripts/ci-plan.py"
                self.assertEqual(plan(path)["lanes"], ALL if ci_change else PACKAGED if path in PACKAGE_INPUTS else FULL)

    def test_a_crate_readme_is_still_the_crates_change(self):
        # The crate may include it in a doc test; the documentation rule only
        # covers folders no rule claims.
        self.assertTrue({"rust", "windows-check"} <= self.lanes("hide-core/README.md"))

    def test_a_named_path_does_not_narrow_a_plan_that_has_another_reason(self):
        self.assertEqual(self.lanes("agents/prd/x/prd.md", "web/src/Overview.tsx"), self.lanes("web/src/Overview.tsx"))
        self.assertEqual(plan("agents/prd/x/prd.md", "Cargo.lock")["lanes"], FULL)

    def test_no_script_named_as_unread_is_called_by_a_lane(self):
        # The claim behind POLICY_ONLY's scripts: nothing but `policy` (and a
        # workflow of its own) names them. `pr.yml`'s policy job may.
        workflows = ROOT / ".github/workflows"
        pr = (workflows / "pr.yml").read_text()
        policy = pr[pr.index("\n  policy:\n"):pr.index("\n  rust:\n")]
        callers = pr.replace(policy, "") + "".join(
            (workflows / name).read_text()
            for name in ("web-e2e.yml", "desktop-e2e.yml", "os-contract.yml", "remote-mailbox.yml", "package.yml")
        )
        for helper in ("verify-cargo.sh", "verify-web.sh", "toolchain-env.sh", "install-nextest.sh", "ci-flaky-report.py"):
            callers += (ROOT / "scripts" / helper).read_text()
        checked = 0
        for pattern in ci.POLICY_ONLY:
            if not pattern.startswith("scripts/") or pattern.startswith("scripts/tests/"):
                continue
            for found in ROOT.glob(pattern):
                checked += 1
                with self.subTest(script=found.name):
                    self.assertNotIn(found.name, callers)
        self.assertGreater(checked, 8)


def code_sources():
    """Each Rust, web, desktop and user docs source's code with comment lines left out, by path, with its crate."""
    crate_of = {directory: crate["name"] for directory, crate in CRATES.items()}
    sources = {}
    for folder in [*crate_of, "web/src", "web/e2e", "desktop/src", "desktop/e2e", "desktop/scripts", "scripts/docs"]:
        for source in (ROOT / folder).rglob("*"):
            if source.suffix not in (".rs", ".ts", ".tsx", ".mjs", ".js") or "node_modules" in source.parts:
                continue
            lines = source.read_text(encoding="utf-8").splitlines()
            code = "\n".join(line for line in lines if not line.lstrip().startswith(("//", "/*", "*")))
            sources[source.relative_to(ROOT).as_posix()] = (crate_of.get(folder), code)
    return sources


def pr_jobs():
    """Each job of `pr.yml`, by id, as the text of its block."""
    workflow = (ROOT / ".github/workflows/pr.yml").read_text()
    body = workflow[workflow.index("\njobs:\n") + len("\njobs:\n"):]
    blocks = re.split(r"\n(?=  [a-z0-9-]+:\n)", "\n" + body)
    return {re.match(r"\s*([a-z0-9-]+):", block).group(1): block for block in blocks if block.strip()}


# The package call's macOS jobs, which follow the plan's `macos` output.
INCLUDE_MACOS = r"include-macos: \$\{\{ needs\.plan\.outputs\.macos == 'true' \}\}"


def runner_jobs(block):
    """How many runner jobs one `pr.yml` job starts: a reusable workflow starts its own."""
    if "uses: ./.github/workflows/rust.yml" in block:
        rust = yaml.safe_load((ROOT / ".github/workflows/rust.yml").read_text())["jobs"]
        # The build, its test shards and the lint beside them.
        return sum(len(job.get("strategy", {}).get("matrix", {}).get("shard", [None])) for job in rust.values())
    if "uses: ./.github/workflows/web-e2e.yml" in block:
        shards = int(re.search(r"\n      shards: (\d+)", block).group(1))
        # With more than one shard parallel daemon and web builds precede them.
        return shards + (2 if shards > 1 else 0)
    if "uses: ./.github/workflows/desktop-e2e.yml" in block:
        shards = int(re.search(r"\n      shards: (\d+)", block).group(1))
        # With more than one shard the cargo build and the web shell build, two
        # parallel jobs, precede them.
        return shards + (2 if shards > 1 else 0)
    if any(f"uses: ./.github/workflows/{name}.yml" in block for name in ("os-contract", "vt", "holder-restart", "agy-contract", "omp-contract", "agent-clis")):
        # A lane whose macOS or Windows leg follows the plan; the most is every leg.
        systems = re.search(r"\n      systems: (.*)", block).group(1)
        if systems.startswith("${{"):
            return len(json.loads(evaluate(systems[3:-2], "true")))
        return len(json.loads(systems.strip("'")))
    if "uses: ./.github/workflows/relay-protocol.yml" in block:
        return 3
    if "uses: ./.github/workflows/repeat.yml" in block:
        # The count, the unit tests' run, the Rust modules' run and the
        # verdict, and the web specs' run at its most: its daemon and shell
        # builds, as many shards as an over-budget plan may take, and their total.
        return 4 + 2 + ci.ci_repeat.WEB_OVER_BUDGET_MAX_SHARDS + 1
    if "uses: ./.github/workflows/windows-check.yml" in block:
        # The compile, the Rust tests and the unit suites, each on its own runner.
        return len(yaml.safe_load((ROOT / ".github/workflows/windows-check.yml").read_text())["jobs"])
    if "uses: ./.github/workflows/wsl-device.yml" in block:
        # Windows cannot build the Linux programs the distribution runs.
        return 2
    if "uses: ./.github/workflows/agent-journey.yml" in block:
        # Stand-ins run on each system but Windows; Windows has its forms job.
        systems = re.search(r"\n      systems: (.*)", block).group(1)
        systems = json.loads(evaluate(systems[3:-2], "true") if systems.startswith("${{") else systems.strip("'"))
        return len([system for system in systems if system != "windows"]) + ("windows" in systems)
    if "uses: ./.github/workflows/package.yml" in block:
        package = (ROOT / ".github/workflows/package.yml").read_text()
        helpers = (ROOT / ".github/workflows/device-helpers.yml").read_text()
        # Count every runner, including preparation and separate compiles, and
        # expand the nested helper matrix. Both macOS jobs follow the caller.
        count = len(re.findall(r"\n    runs-on:", package))
        count += len(re.findall(r"\n          - platform:", helpers))
        if not re.search(INCLUDE_MACOS, block):
            count -= len(re.findall(r"\n    if: \$\{\{ inputs\.include-macos \}\}", package))
        return count
    return 1


class Workflows(unittest.TestCase):
    """`pr.yml` has the jobs the plan names, and the job count TESTING.md states."""

    def test_package_counts_preparation_and_compile_jobs_as_well_as_packages(self):
        call = pr_jobs()["package"]
        # Identity, two Linux helpers, bundles, Windows compile and two packages.
        self.assertEqual(runner_jobs(re.sub(INCLUDE_MACOS, "include-macos: false", call)), 7)
        # macOS adds both a compile and a package runner.
        self.assertEqual(runner_jobs(call), 9)

    def test_ci_definitions_exercise_every_lane_including_package(self):
        paths = [path.relative_to(ROOT).as_posix() for path in (ROOT / ".github/workflows").glob("*.yml")]
        paths += [".github/actions/rust-cache/action.yml", "scripts/ci-plan.py"]
        for status in ("A", "M", "D"):
            for path in paths:
                with self.subTest(status=status, path=path):
                    result = plan(path, status=status)
                    self.assertTrue(result["full"])
                    self.assertEqual(result["lanes"], ALL)
                    self.assertEqual(result["rust_packages"], EVERY_PACKAGE)

    def test_a_new_push_cancels_the_previous_pr_run_but_a_body_edit_does_not(self):
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        concurrency = workflow[workflow.index("\nconcurrency:\n"):workflow.index("\njobs:\n")]
        self.assertIn("group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}", concurrency)
        condition = re.search(r"cancel-in-progress: \$\{\{(.+?)\}\}", concurrency).group(1)
        # The same expression-boundary adapter as test_nightly_routing: evaluate
        # the committed condition for real event kinds, without freezing its text.
        for event, action, expected in (("pull_request", "synchronize", True),
                                        ("pull_request", "edited", False),
                                        ("pull_request", "opened", True),
                                        ("push", "", False), ("merge_group", "", False), ("schedule", "", False),
                                        ("workflow_dispatch", "", False)):
            with self.subTest(event=event, action=action):
                code = condition.replace("github.event_name", repr(event)).replace("github.event.action", repr(action))
                code = code.replace("&&", " and ").replace("||", " or ")
                self.assertEqual(eval(code, {"__builtins__": {}}), expected)
        self.assertRegex(workflow, r"types: \[[^\]]*\bsynchronize\b[^\]]*\]")

    def test_the_package_lane_is_the_only_way_a_pull_request_reaches_package_yml(self):
        # One list of package paths: `package.yml` has no trigger to keep in step with PACKAGE_PATHS.
        package = (ROOT / ".github/workflows/package.yml").read_text()
        triggers = package[package.index("\non:\n"):package.index("\njobs:\n")]
        self.assertNotIn("pull_request", triggers)
        self.assertNotIn("paths:", triggers)
        # The call builds the macOS package and runs its specs when the run takes macOS.
        self.assertRegex(pr_jobs()["package"], INCLUDE_MACOS)

    def test_every_lane_has_a_job_that_asks_the_plan_and_verify_waits_on_it(self):
        jobs = pr_jobs()
        for lane in ci.LANES:
            with self.subTest(lane=lane):
                self.assertIn(f"if: contains(fromJSON(needs.plan.outputs.lanes), '{lane}')", jobs[lane])
        verify = re.search(r"needs: \[([^\]]*)\]", jobs["verify"]).group(1)
        self.assertEqual(sorted(name.strip() for name in verify.split(",")), sorted(["plan", *ci.LANES]))
        self.assertEqual(set(jobs), {"plan", "verify", *ci.LANES})

    def test_the_nightly_runs_each_macos_check_once(self):
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        # The OS contract runs through `verify` (all three legs), not again as a
        # job of its own or a step in the desktop job.
        self.assertNotIn("\n  os-contract:\n", nightly)
        self.assertNotIn("OS contract on macOS", nightly)
        # The macOS web tests are the nightly's own job, not a `verify` lane.
        self.assertIn("\n  web-e2e-macos:\n", nightly)
        self.assertNotIn("web-e2e-platform", nightly)
        # The mailbox lane runs on macOS here and on Linux in `pr.yml`.
        call = nightly[nightly.index("\n  remote-mailbox-macos:\n"):]
        self.assertIn("uses: ./.github/workflows/remote-mailbox.yml", call)
        self.assertIn("runner: macos-15", call)
        self.assertRegex(nightly[nightly.index("\n  report:\n"):], r"needs: \[[^\]]*\bremote-mailbox-macos\b[^\]]*\]")

    def test_the_nightly_runs_only_what_differs_by_system_off_the_systems_that_run_it_whole(self):
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        # Linux's whole web suite is a `verify` lane, so the nightly has no web job for it;
        # macOS and Windows run the web tests tagged `@platform`, one shard each.
        self.assertNotIn("\n  web-e2e-linux:\n", nightly)
        for name in ("web-e2e-macos", "web-e2e-windows"):
            with self.subTest(job=name):
                call = nightly[nightly.index(f"\n  {name}:\n"):].split("\n\n")[0]
                self.assertIn("shards: 1", call)
                self.assertIn('grep: "@platform"', call)
        # The desktop suite runs whole on macOS and only its `@platform` tests elsewhere.
        desktop = nightly[nightly.index("\n  desktop-e2e:\n"):nightly.index("\n  verify:\n")]
        self.assertIn("uses: ./.github/workflows/desktop-e2e.yml", desktop)
        self.assertIn("grep: ${{ matrix.system != 'macos' && '@platform' || '' }}", desktop)
        # A desktop change plans the macOS desktop job; a web spec, which the app does not run, does not.
        self.assertIn("desktop-e2e", plan("desktop/e2e/fixture.ts")["lanes"])
        self.assertNotIn("desktop-e2e", plan("web/e2e/new-tab.spec.ts")["lanes"])

    def test_only_the_macos_lanes_name_a_macos_runner(self):
        # The macOS jobs are the ones in MACOS_LANES; no other job names a macOS
        # runner, so a plan without those lanes holds no macOS job.
        jobs = pr_jobs()
        for lane, block in jobs.items():
            with self.subTest(lane=lane):
                body = "\n".join(line for line in block.splitlines() if not line.lstrip().startswith("#"))
                names_macos = bool(re.search(r"macos-\d+|\"macos\"|include-macos:", body))
                self.assertEqual(names_macos, lane in MACOS_LANES)

    def test_the_most_a_pull_request_starts_is_the_count_testing_md_states(self):
        jobs = pr_jobs()
        # A merge group's own two checks never run on a pull request.
        most = 2 + sum(runner_jobs(jobs[lane]) for lane in ci.PATH_LANES)
        # The 2026-10-05 run started 23 jobs. Since then the macOS desktop
        # suite's second shard and the fifth and sixth web shards were added
        # because the suites outgrew their shards' step caps, which a smaller
        # queue does not fix (docs/TESTING.md, "Balancing the e2e shards"), and
        # the agent journey's Linux stand-in job and Windows forms job were
        # added, then the agent CLIs' Linux and Windows installs, which only a
        # pin change plans; a job beyond these is a new decision, made with
        # this number. The independent shared web build adds one job so it
        # does not serialize the daemon and shell builds. The refreshed
        # twenty-run median requires six web shards. The WSL device lane's
        # Linux build and Windows run add two, planned only for the crates the
        # WSL link crosses. The vt lane's three legs, planned only for a change
        # to hide-vt's build, add three. The holder restart lane's three legs,
        # planned only for a change to the holder, the node's holder code, the
        # agent pins or the lane's scripts, add three.
        # Native and WASM relay interoperability adds one job on each OS.
        # The package lane already ran six preparation/compile jobs omitted
        # from the old count; correcting the accounting starts no new job.
        # The macOS desktop lane's daemon and kit binaries build once in one
        # macOS `build` job and its web shell once in a Linux `build-web` job
        # beside it, ahead of its two shards, which build nothing; that is one
        # macOS job and one Linux job for the 8 minutes of builds each shard
        # did itself (docs/TESTING.md, "Balancing the e2e shards"): two jobs.
        # Every suite runs once, on the core's own engine, since the Herdr lanes
        # were removed (PRD engine-bundle-4-herdr-removal D-2).
        # The remote mailbox lane's Linux arm64 leg adds one, planned only
        # for the device link's crates, its device test and the device kit
        # contract (with-hide/hide-private#501).
        # The repeat lane's count, web specs, unit tests, Rust modules and
        # final verdict raise it from 51 to 56. The verdict requires actual
        # completed runs from every leg the count chose. The user docs lane
        # adds one, planned for `site/`, the web shell's sources,
        # `scripts/docs/`, `hided/src/cli.rs` and the CLI contract.
        # The Android app host's lane adds one, planned for `mobile/android/`.
        # The Windows `@platform` lane's third shard, its daemon and shell
        # builds now outside the shard, and its second shard raise it from 58
        # to 62: one job held the suite at 22.4 minutes, the longest of a run,
        # and three shards plan 4.2 minutes of tests each (docs/TESTING.md,
        # "Balancing the e2e shards").
        # The Rust lane's one job became its build, three test shards and its
        # lint (seven with the web lane's nine shards, from 62 to 69), so the
        # two longest Linux lines end about 13 minutes into a run, not 20
        # (docs/TESTING.md, "The Rust lane's shards").
        # The Windows lane's one job became its compile, its Rust tests and its
        # unit suites on three runners (from 69 to 71), so a pull request with a
        # Windows path ends near the Linux lines, not 27 minutes in
        # (docs/TESTING.md, "Where the Windows runners went").
        # The repeat lane's web specs moved onto `web-e2e.yml`'s shards, at
        # most six with their two builds and their total (from 71 to 79), so a
        # pull request that changes a spec no longer waits on one runner
        # repeating it for up to 91 minutes (docs/TESTING.md, "The repeat lane").
        # The macOS desktop lane's two builds (see above) are two of the 81.
        self.assertLessEqual(most, 81, "a run starts more jobs than the suites and native SDK contracts need")
        testing = (ROOT / "docs/TESTING.md").read_text()
        self.assertIn(f"starts at most {most} jobs", testing)


class RustLane(unittest.TestCase):
    """The Rust lane is one `verify` lane over `rust.yml`'s jobs (docs/TESTING.md, The Rust lane's shards).

    `verify` sees only the call's result, and a called workflow fails when one of
    its jobs fails but not when one is skipped, so this holds what keeps a
    skipped job from passing for a run test: every job runs unconditionally,
    the shards wait on the build alone, and nothing ignores a failure.
    """

    def setUp(self):
        self.lane = yaml.safe_load((ROOT / ".github/workflows/rust.yml").read_text())
        self.jobs = self.lane["jobs"]

    def test_the_build_three_shards_and_the_lint_each_run_or_fail_the_call(self):
        self.assertEqual(set(self.jobs), {"build", "tests", "lint"})
        for name, job in self.jobs.items():
            with self.subTest(job=name):
                # A job-level condition could skip it while the call passes.
                self.assertNotIn("if", job)
                self.assertFalse(job.get("continue-on-error", False))
        # A shard is skipped only when the build failed or was cancelled,
        # which fails the call.
        self.assertEqual(self.jobs["tests"]["needs"], "build")
        self.assertNotIn("needs", self.jobs["build"])
        self.assertNotIn("needs", self.jobs["lint"])
        strategy = self.jobs["tests"]["strategy"]
        self.assertEqual(strategy["matrix"], {"shard": [1, 2, 3]})
        self.assertFalse(strategy["fail-fast"])
        run = next(step["run"] for step in self.jobs["tests"]["steps"] if "--archive-file" in step.get("run", ""))
        self.assertIn("--partition 'slice:${{ matrix.shard }}/3'", run)

    def test_the_call_passes_the_plan_and_ignores_no_failure(self):
        call = yaml.safe_load((ROOT / ".github/workflows/pr.yml").read_text())["jobs"]["rust"]
        self.assertEqual(call["uses"], "./.github/workflows/rust.yml")
        self.assertFalse(call.get("continue-on-error", False))
        self.assertEqual(call["with"], {"full": "${{ needs.plan.outputs.full }}",
                                        "packages": "${{ needs.plan.outputs.rust-packages }}",
                                        "deny": "${{ needs.plan.outputs.deny }}"})
        # YAML 1.1 reads the `on` key as true.
        self.assertEqual(set(self.lane[True]["workflow_call"]["inputs"]), {"full", "packages", "deny"})
        self.assertEqual({name: self.lane["env"][name] for name in ("FULL", "PACKAGES", "DENY")},
                         {"FULL": "${{ inputs.full }}", "PACKAGES": "${{ inputs.packages }}", "DENY": "${{ inputs.deny }}"})

    def run_step(self, job, name, full, packages):
        """The verify-cargo calls a step's script makes for a plan, with the script itself stood in."""
        script = next(step["run"] for step in self.jobs[job]["steps"] if step.get("name") == name)
        with tempfile.TemporaryDirectory() as temp:
            (Path(temp) / "rust-tests.tar.zst").write_bytes(b"")
            result = subprocess.run(
                ["bash", "-euo", "pipefail", "-c", script.replace("bash scripts/verify-cargo.sh", "echo verify-cargo")],
                # The runner's bash (5, for `mapfile`), found as the caller finds it.
                env={"PATH": os.environ["PATH"], "FULL": full, "PACKAGES": packages,
                     "RUNNER_TEMP": temp, "ARCHIVE": "rust-tests.tar.zst"},
                capture_output=True, text=True)
            calls = [line.replace(temp, "$RUNNER_TEMP") for line in result.stdout.splitlines() if line.startswith("verify-cargo")]
        return result.returncode, calls

    def test_both_plan_forms_reach_the_build_and_the_lint(self):
        archive = "Build and pack the planned crates' tests"
        lint = "Check formatting and lint the planned crates"
        self.assertEqual(self.run_step("build", archive, "true", "[]"),
                         (0, ["verify-cargo nextest-archive $RUNNER_TEMP/rust-tests.tar.zst --workspace"]))
        self.assertEqual(self.run_step("build", archive, "false", '["hide-core","hided"]'),
                         (0, ["verify-cargo nextest-archive $RUNNER_TEMP/rust-tests.tar.zst -p hide-core -p hided"]))
        self.assertEqual(self.run_step("lint", lint, "true", "[]"), (0, ["verify-cargo lint"]))
        self.assertEqual(self.run_step("lint", lint, "false", '["hide-core"]'),
                         (0, ["verify-cargo fmt-check", "verify-cargo clippy -p hide-core --all-targets -- -D warnings"]))
        # A plan that chose the lane but no crate stops both rather than test nothing.
        for job, name in (("build", archive), ("lint", lint)):
            with self.subTest(job=job):
                status, calls = self.run_step(job, name, "false", "[]")
                self.assertNotEqual(status, 0)
                self.assertEqual(calls, [])


class WindowsCheckLane(unittest.TestCase):
    """The Windows lane is one `verify` lane over `windows-check.yml`'s three jobs.

    `verify` sees only the call's result, and a called workflow fails when one of
    its jobs fails but not when one is skipped, so every job runs
    unconditionally, waits on none of the others, and ignores no failure.
    """

    def setUp(self):
        self.lane = yaml.safe_load((ROOT / ".github/workflows/windows-check.yml").read_text())
        self.jobs = self.lane["jobs"]

    def commands(self, job):
        return "\n".join(step.get("run", "") for step in self.jobs[job]["steps"])

    def test_compile_rust_tests_and_unit_suites_each_start_at_once_and_run_or_fail_the_call(self):
        self.assertEqual(set(self.jobs), {"compile", "tests", "web"})
        for name, job in self.jobs.items():
            with self.subTest(job=name):
                self.assertEqual(job["runs-on"], "windows-latest")
                # A job-level condition could skip it while the call passes.
                self.assertNotIn("if", job)
                self.assertNotIn("needs", job)
                self.assertFalse(job.get("continue-on-error", False))
        call = yaml.safe_load((ROOT / ".github/workflows/pr.yml").read_text())["jobs"]["windows-check"]
        self.assertEqual(call["uses"], "./.github/workflows/windows-check.yml")
        self.assertEqual(call["name"], "windows check")
        self.assertFalse(call.get("continue-on-error", False))
        self.assertEqual(call["secrets"], {"HIDE_ISSUES_APP_KEY": "${{ secrets.HIDE_ISSUES_APP_KEY }}"})

    def test_each_check_runs_in_exactly_one_job(self):
        self.assertIn("verify-cargo.sh check --workspace --all-targets", self.commands("compile"))
        tests = self.commands("tests")
        self.assertIn("nextest --profile ci $TEST_TARGETS --no-run", tests)
        self.assertIn("ci-flaky-report.py --suite rust", tests)
        self.assertIn(". scripts/prepare-ignored-tests.sh", tests)
        for suite in ("web", "desktop"):
            self.assertIn(f"verify-web.sh {suite} test", self.commands("web"))
        # A cargo or vitest command a job runs, no other job runs.
        for name in self.jobs:
            for line in self.commands(name).splitlines():
                line = line.strip()
                if not line.startswith(("bash scripts/verify-cargo.sh", "bash scripts/verify-web.sh")):
                    continue
                with self.subTest(job=name, command=line[:80]):
                    self.assertEqual([other for other in self.jobs if line in self.commands(other)], [name])
        # Only the Rust tests read the selection the test binaries were built for.
        self.assertEqual([name for name, job in self.jobs.items() if "TEST_TARGETS" in job.get("env", {})], ["tests"])
        self.assertNotIn("$TEST_TARGETS", self.commands("compile") + self.commands("web"))

    def test_a_failed_setup_step_stops_no_check_the_one_job_ran_after_it(self):
        # A failed setup step stops no check the one job would have run after it:
        # the test build, the web setup and the install run unless cancelled,
        # and every check runs on what it needs (the build, the install).
        conditions = {(name, step.get("name") or step.get("uses")): step.get("if")
                      for name, job in self.jobs.items() for step in job["steps"]}
        for key in (("tests", "Build the Windows test binaries"), ("web", "pnpm/action-setup@v6"),
                    ("web", "actions/setup-node@v4"), ("web", "Install dependencies")):
            with self.subTest(step=key):
                self.assertEqual(conditions[key], "${{ !cancelled() }}")
        test_steps = [step for step in self.jobs["tests"]["steps"] if "$TEST_TARGETS -E" in step.get("run", "")]
        self.assertEqual(len(test_steps), 30)
        for step in test_steps:
            with self.subTest(step=step["name"][:60]):
                self.assertEqual(step.get("if"), "${{ !cancelled() && steps.tests.outcome == 'success' }}")
        for step in self.jobs["web"]["steps"]:
            if "verify-web.sh" in step.get("run", "") and " test" in step["run"]:
                with self.subTest(step=step["name"]):
                    self.assertEqual(step.get("if"), "${{ !cancelled() && steps.web-deps.outcome == 'success' }}")


def pr_plan(*paths, status="M"):
    """A pull request's plan, with its macOS and Windows jobs decided as `plan` decides them."""
    entries = [(status, path) for path in paths]
    return ci.pull_request_systems(ci.select(entries, CRATES, ROOT), entries, CRATES, ROOT)


def evaluate(expression, macos, legs=ci.WINDOWS_LEG_LANES):
    """A `pr.yml` expression evaluated for the plan's `macos` and
    `windows-legs` outputs, the way the concurrency test evaluates its
    condition; `format` is GitHub's, whose `{0}` placeholders are Python's."""
    code = re.sub(r"contains\(fromJSON\(needs\.plan\.outputs\.windows-legs\), '([a-z-]+)'\)",
                  lambda found: repr(found.group(1) in legs), expression)
    code = code.replace("needs.plan.outputs.macos", repr(macos)).replace("&&", " and ").replace("||", " or ")
    return eval(code, {"__builtins__": {}, "format": lambda text, *values: text.format(*values)})


class MacosOnPullRequests(unittest.TestCase):
    """A pull request takes a macOS runner only for a macOS path; main and the nightly run every macOS job."""

    def assertNoMacos(self, result):
        self.assertFalse(result["macos"]["runs"], result["macos"])
        self.assertFalse(set(result["lanes"]) & set(ci.MACOS_ONLY_LANES))
        # `verify` passes with the macOS-only lanes skipped, and fails if one ran.
        ci.aggregate(needs_for(result))
        for lane in ci.MACOS_ONLY_LANES:
            with self.assertRaises(ValueError):
                ci.aggregate(needs_for(result, **{lane: "success"}))

    def test_a_web_only_pull_request_plans_no_macos_job(self):
        before = plan("web/src/Overview.tsx")
        self.assertIn("desktop-e2e", before["lanes"])
        result = pr_plan("web/src/Overview.tsx")
        self.assertEqual(result["lanes"], ["policy", "checks", "docs", "web-e2e"])
        self.assertNoMacos(result)
        self.assertIn("macOS jobs: skipped", ci.summary(result))

    def test_a_pull_request_without_a_macos_path_plans_none_whatever_else_it_plans(self):
        for status in ("A", "M"):
            for path in (
                "hide-core/src/lib.rs", "hided/src/lib.rs", "desktop/src/main/index.ts", "desktop/src/main/host.ts",
                "desktop/static/icon.png", "desktop/e2e/desktop.spec.ts", "web/e2e/pane-fixture.ts",
                "hide-platform/src/wsl.rs", "hide-platform/src/path.rs",
                "hide-kit/src/lib.rs", "hide-ai/src/omp.rs", "hide-holder/src/lib.rs",
                "desktop/scripts/build-identity.mjs", "desktop/scripts/macos-signing.test.mjs",
                "desktop/src/main/updater.test.ts", "Cargo.lock", "mystery/file.txt",
                ".github/workflows/nightly.yml", "docs/TESTING.md",
            ):
                with self.subTest(status=status, path=path):
                    self.assertNoMacos(pr_plan(path, status=status))
        # Lanes with a macOS leg still run their Linux and Windows legs.
        self.assertIn("package", pr_plan("hide-kit/src/lib.rs")["lanes"])
        self.assertIn("omp-contract", pr_plan("hide-ai/src/omp.rs")["lanes"])
        self.assertIn("vt", pr_plan("hide-holder/src/lib.rs")["lanes"])
        self.assertTrue({"os-contract", "windows-check"} <= set(pr_plan("hide-platform/src/wsl.rs")["lanes"]))
        # A CI definition outside the macOS and Windows jobs still exercises every other lane.
        self.assertEqual(pr_plan(".github/workflows/nightly.yml")["lanes"],
                         [lane for lane in ALL if lane not in ci.MACOS_ONLY_LANES + ci.WINDOWS_ONLY_LANES])

    def test_a_pull_request_that_changes_a_macos_path_plans_its_macos_jobs(self):
        cases = (
            ("desktop/src/main/windowChrome.ts", {"desktop-e2e"}),
            ("desktop/src/main/menu.ts", {"desktop-e2e"}),
            ("desktop/src/main/updater.ts", {"desktop-e2e", "package"}),
            ("hide-platform/src/process.rs", {"os-contract-macos", "omp-contract"}),
            ("hide-platform/src/user_agents/launchd.rs", {"os-contract-macos", "omp-contract"}),
            ("hide-platform/tests/it/user_agents.rs", {"os-contract-macos", "omp-contract"}),
            ("desktop/scripts/macos-signing.mjs", {"package"}),
            ("desktop/scripts/app-identity.mjs", {"package"}),
            ("desktop/scripts/package.mjs", {"package"}),
            # No narrower rule names these, so they plan every lane and the package
            # (not `holder-restart`, which only its own paths plan).
            ("desktop/native/notification-status/main.swift", set(MACOS_LANES) - {"holder-restart"}),
            ("desktop/resources/entitlements/app.plist", set(MACOS_LANES) - {"holder-restart"}),
            ("hide-vt/build.rs", {"vt", "holder-restart"}),
            (".github/workflows/pr.yml", set(MACOS_LANES)),
            (".github/workflows/desktop-e2e.yml", set(MACOS_LANES)),
            (".github/actions/rust-cache/action.yml", set(MACOS_LANES)),
            ("scripts/ci-plan.py", set(MACOS_LANES)),
        )
        for status in ("A", "M", "D"):
            for path, expected in cases:
                with self.subTest(status=status, path=path):
                    result = pr_plan(path, status=status)
                    self.assertTrue(result["macos"]["runs"], result["macos"])
                    self.assertEqual(set(result["lanes"]) & MACOS_LANES, expected)
                    ci.aggregate(needs_for(result))
        # A mixed change keeps the macOS jobs every path plans once one path is macOS.
        result = pr_plan("web/src/Overview.tsx", "desktop/src/main/windowChrome.ts")
        self.assertIn("desktop-e2e", result["lanes"])
        self.assertIn("macOS path: desktop/src/main/windowChrome.ts", result["macos"]["why"])
        # A deleted hide-platform file cannot be read for its macOS code.
        self.assertTrue(pr_plan("hide-platform/src/wsl.rs", status="D")["macos"]["runs"])

    def test_every_macos_path_names_a_tracked_file(self):
        tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.splitlines()
        for pattern in ci.MACOS_PATHS:
            with self.subTest(pattern=pattern):
                self.assertTrue(any(fnmatchcase(name, pattern) for name in tracked), f"{pattern} names no file")
        # The macOS code marker finds hide-platform's macOS forms and leaves its other files out.
        marked = {name for name in tracked if name.startswith("hide-platform/") and ci.macos_path("M", name)}
        self.assertTrue({"hide-platform/src/process.rs", "hide-platform/src/user_agents/launchd.rs"} <= marked)
        self.assertFalse({"hide-platform/src/wsl.rs", "hide-platform/src/path.rs", "hide-platform/Cargo.toml"} & marked)

    def test_main_pushes_the_nightly_and_unreadable_plans_run_every_macos_job(self):
        for event in ("push", "schedule", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                result = ci.plan(event, None, None, ROOT)
                self.assertTrue(result["macos"]["runs"])
                self.assertEqual(result["lanes"], FULL)
                self.assertTrue(set(ci.MACOS_ONLY_LANES) <= set(result["lanes"]))
        nightly = ci.plan("schedule", None, None, ROOT, nightly_agent_checks=True)
        self.assertTrue(nightly["macos"]["runs"])
        self.assertTrue(set(ci.MACOS_ONLY_LANES) <= set(nightly["lanes"]))
        unreadable = ci.plan("pull_request", "0" * 40, "HEAD", ROOT)
        self.assertTrue(unreadable["macos"]["runs"])
        self.assertIn("comparison unavailable", unreadable["macos"]["why"][0])
        self.assertEqual(unreadable["lanes"], FULL)
        self.assertTrue(ci.select([("T", "web/src/store.ts")], CRATES, ROOT)["full"])
        self.assertTrue(ci.macos_path("T", "web/src/store.ts"))

    def test_a_pull_request_read_from_git_decides_macos_from_its_files(self):
        with tempfile.TemporaryDirectory() as directory:
            pr = PullRequest(directory)
            pr.commit("main", "base", {"web/src/x.ts": "one\n", "hide-platform/src/a.rs": "fn a() {}\n"})
            pr.git("checkout", "-qb", "topic")
            pr.commit("topic", "web", {"web/src/x.ts": "two\n"})
            pr.check_out_merge()
            result = pr.plan(None)
            self.assertEqual(result["lanes"], ["policy", "checks", "docs", "web-e2e"])
            self.assertFalse(result["macos"]["runs"])
            pr.commit("topic", "platform", {"hide-platform/src/a.rs": '#[cfg(target_os = "macos")]\nfn a() {}\n'})
            pr.check_out_merge()
            result = pr.plan(None)
            self.assertTrue(result["macos"]["runs"])
            self.assertIn("desktop-e2e", result["lanes"])
            self.assertEqual(result["macos"]["why"], ["macOS code: hide-platform/src/a.rs"])

    def test_pr_yml_runs_the_macos_legs_the_plan_names(self):
        jobs = pr_jobs()
        self.assertIn("macos: ${{ steps.plan.outputs.macos }}", jobs["plan"])
        for lane in ("vt", "holder-restart", "omp-contract"):
            with self.subTest(lane=lane):
                expression = re.search(r"\n      systems: \$\{\{(.+?)\}\}", jobs[lane]).group(1)
                self.assertEqual(json.loads(evaluate(expression, "true")), ["linux", "macos", "windows"])
                self.assertEqual(json.loads(evaluate(expression, "false")), ["linux", "windows"])
        expression = re.search(r"\n      include-macos: \$\{\{(.+?)\}\}", jobs["package"]).group(1)
        self.assertIs(evaluate(expression, "true"), True)
        self.assertIs(evaluate(expression, "false"), False)
        # The macOS-only lanes follow the plan's lanes, which leave them out.
        for lane in ci.MACOS_ONLY_LANES:
            self.assertIn(f"if: contains(fromJSON(needs.plan.outputs.lanes), '{lane}')", jobs[lane])
        self.assertEqual(set(ci.MACOS_ONLY_LANES) | set(ci.MACOS_LEG_LANES), MACOS_LANES)


# What recently merged pull requests changed, as `git diff --name-status`
# against main listed it (read from GitHub, 2026-11-21): the cases the
# Windows rule was decided on. They are read here a whole file at a time, as
# the plan reads a pull request whose diff it cannot read; `RustChange`
# below reads a change against a file's Windows items. A file a later change
# gives Windows code moves its pull request to WINDOWS_PULL_REQUESTS, which
# is the rule working.
NO_WINDOWS_PULL_REQUESTS = {
    # A core runtime fix with its tests and the guide.
    2785: (("M", "docs/UI_BEHAVIOR.md"), ("M", "hide-core/src/runtime/automations.rs"),
           ("M", "hide-core/src/runtime/tests/desk_views.rs"), ("M", "hide-core/src/runtime/unseen_views.rs"),
           ("M", "hide-core/src/runtime/view_areas.rs")),
    2789: (("M", "docs/ARCHITECTURE.md"), ("M", "docs/PERFORMANCE_TESTING.md"), ("M", "hide-core/src/runtime/agent_areas.rs"),
           ("M", "hide-core/src/runtime/session.rs"), ("M", "hide-core/src/runtime/tests/agent_areas.rs"),
           ("M", "hide-core/src/workspace_views.rs")),
    # A core change with a new web e2e spec that is not tagged `@platform`.
    2787: (("M", "docs/ARCHITECTURE.md"), ("M", "hide-core/src/runtime/agent_sleep.rs"),
           ("M", "hide-core/src/runtime/agent_sleep/dormant.rs"), ("M", "hide-core/src/runtime/agent_sleep/letters.rs"),
           ("M", "hide-core/src/runtime/tests/agent_sleep.rs"), ("A", "web/e2e/sleeping-tab-live-elsewhere.spec.ts")),
    # A web e2e spec tagged `@platform`, with the core and the shell: the
    # Linux suite runs it, its Windows leg runs on main.
    2781: (("M", "hide-core/src/runtime/detail.rs"), ("A", "web/e2e/card-keyboard.spec.ts"), ("M", "web/src/ViewAreas.tsx"),
           ("M", "web/src/ws.ts")),
    # A web screen, its tokens and the guide.
    2774: (("M", "design/tokens.json"), ("M", "docs/UI_BEHAVIOR.md"), ("M", "web/src/AutomationScreen.tsx"),
           ("M", "web/src/tokens.css")),
    2725: (("M", "docs/ARCHITECTURE.md"), ("M", "hide-core/src/runtime/automations.rs"),
           ("M", "hide-core/src/runtime/tests/automations.rs")),
    2713: (("M", "docs/delivery.md"), ("M", "hide-core/src/coordination/mod.rs"), ("M", "hide-core/src/coordination/room.rs"),
           ("M", "hide-core/src/delivery/ledger.rs"), ("M", "hide-core/src/delivery/watch.rs"),
           ("M", "hide-core/src/delivery/worker.rs")),
    # A CLI command of hided, its help, its test and the user docs' notes.
    2698: (("M", "hided/assets/automation/help.md"), ("M", "hided/assets/guide.md"), ("M", "hided/src/automation_cli.rs"),
           ("M", "hided/tests/it/automations.rs"), ("M", "site/src/data/cli-notes.json")),
    # The core and the phone's daemon side.
    2697: (("M", "docs/ARCHITECTURE.md"), ("M", "hide-core/src/agent_state/tally/phone/automations.rs"),
           ("M", "hide-core/src/automations.rs"), ("M", "hide-core/src/runtime/automations.rs"),
           ("M", "hide-core/src/runtime/tests/automations.rs"), ("M", "hide-core/src/runtime/usage_ledger.rs"),
           ("M", "hided/src/mobile/phone.rs")),
    # The core with an added test fixture.
    2757: (("M", "docs/AI_PROVIDERS.md"), ("M", "docs/ARCHITECTURE.md"), ("M", "hide-core/src/runtime/automations.rs"),
           ("M", "hide-core/src/runtime/tests/automations.rs"), ("M", "hide-core/src/usage.rs"),
           ("M", "hide-core/src/usage/limits/claude.rs"), ("M", "hide-core/src/usage/limits/tests.rs"),
           ("A", "hide-core/tests/fixtures/claude-usage/cost-2.1.329.json")),
    # Documentation alone.
    2747: (("M", "docs/BUILD.md"),),
    # The core, the web shell and a web unit test.
    2738: (("M", "docs/UI_BEHAVIOR.md"), ("M", "hide-core/src/missions.rs"), ("M", "hide-core/src/runtime/events.rs"),
           ("M", "hide-core/src/runtime/mission_health.rs"), ("M", "hide-core/src/runtime/missions.rs"),
           ("M", "hide-core/src/runtime/tests/missions.rs"), ("M", "web/src/actions.ts"),
           ("M", "web/src/components/sidebar-mission-row.test.tsx"), ("M", "web/src/components/sidebar-mission-row.tsx")),
}
# Each with the Windows jobs that prove what it changed, beside the
# `package` lane when it changes a package input.
RUST_LEGS = {"windows-check", "os-contract"}
WINDOWS_PULL_REQUESTS = {
    # `hide-platform`'s host code with its Windows form: `windows check`, and
    # the Windows legs of the platform layer's lanes (its WSL link among them).
    2794: ((("M", "docs/ARCHITECTURE.md"), ("M", "docs/INSTALL.md"), ("M", "hide-holder/src/pane_env.rs"),
            ("M", "hide-platform/src/host.rs"), ("M", "hide-platform/tests/it/host.rs")),
           {"windows-check", "windows-wsl", "os-contract", "omp-contract"}),
    # A Windows device's PowerShell bootstrap in the device link.
    2791: ((("M", "docs/ARCHITECTURE.md"), ("M", "hide-node/src/ssh/bootstrap.ps1"), ("M", "hide-node/src/ssh/bootstrap.rs"),
            ("M", "hide-node/src/ssh/bootstrap_tests.rs"), ("M", "hide-node/src/ssh/bootstrap_windows_tests.rs")),
           {"windows-check", "windows-wsl"}),
    # The package manager's workspace file, which no rule narrows: every Windows job.
    2704: ((("M", "pnpm-workspace.yaml"),), set(ci.WINDOWS_LANES) - {"holder-restart", "agent-clis"}),
    # The workflow that defines the Windows jobs: every one.
    2612: ((("M", ".github/workflows/pr.yml"),), set(ci.WINDOWS_LANES)),
    # The hooks the Windows installer carries, with no Windows code: `package` alone.
    2684: ((("M", "docs/agent-hooks.md"), ("M", "hide-agent-hooks/src/memory.rs"), ("M", "hide-agent-hooks/src/plugin/helper.rs"),
            ("M", "hide-core/src/handle.rs"), ("A", "hide-core/src/links/session_brief.rs"), ("M", "hide-memory/src/hook.rs"),
            ("M", "hided/src/core.rs")),
           set()),
    # The core's engine, whose file holds a Windows form: not the e2e suites.
    2788: ((("M", "docs/status-model.md"), ("M", "hide-core/src/delivery/ladder.rs"), ("M", "hide-core/src/engine/fake.rs"),
            ("M", "hide-core/src/engine/mod.rs"), ("M", "hided/src/mobile/pane.rs")),
           RUST_LEGS),
    # The pane holder, whose PTY is ConPTY on Windows: its vt and restart legs.
    2753: ((("M", "docs/status-model.md"), ("M", "hide-holder-protocol/src/lib.rs"), ("M", "hide-holder/src/holder.rs"),
            ("M", "hide-holder/tests/it/watch.rs")),
           {"windows-check", "vt", "holder-restart"}),
    # A crate's manifest and the platform layer's process code.
    2723: ((("M", "AGENTS.md"), ("M", "hide-platform/Cargo.toml"), ("M", "hide-platform/src/process.rs"),
            ("A", "hide-platform/src/process/open_files.rs"), ("M", "hided/src/server.rs")),
           {"windows-check", "windows-wsl", "os-contract", "omp-contract"}),
    # The desktop host's environment, read differently on Windows: its unit
    # suite in `windows check` and the Windows desktop suite.
    2755: ((("M", "desktop/src/main/env.ts"), ("M", "desktop/src/main/env.test.ts"), ("M", "web/src/App.tsx")),
           {"windows-check", "windows-desktop"}),
    # The keyboard, whose Windows form differs: the Windows e2e suites, not `windows check`.
    2680: ((("M", "web/src/keyboard.ts"), ("M", "web/src/Sidebar.tsx")), {"windows-e2e", "windows-desktop"}),
}


def system_legs(expression, macos, legs):
    """The systems a `pr.yml` lane's leg expression names for the plan's outputs."""
    return json.loads(evaluate(expression, macos, legs))


class WindowsOnPullRequests(unittest.TestCase):
    """A pull request runs a Windows job only for a path that job proves; main and the nightly run every one."""

    def assertNoWindows(self, result):
        self.assertEqual(result["windows"]["lanes"], [])
        self.assertFalse(set(result["lanes"]) & set(ci.WINDOWS_ONLY_LANES))
        # `verify` passes with the Windows-only lanes skipped, and fails if one ran.
        ci.aggregate(needs_for(result))
        for lane in ci.WINDOWS_ONLY_LANES:
            with self.assertRaises(ValueError):
                ci.aggregate(needs_for(result, **{lane: "success"}))

    def test_recent_pull_requests_without_a_windows_path_plan_no_windows_job(self):
        dropped = set()
        for number, entries in NO_WINDOWS_PULL_REQUESTS.items():
            with self.subTest(pr=number):
                result = ci.pull_request_systems(ci.select(list(entries), CRATES, ROOT), list(entries), CRATES, ROOT)
                self.assertNoWindows(result)
                self.assertFalse(result["windows"]["runs"])
                self.assertIn("main's push run and the nightly run the Windows jobs", result["windows"]["why"][0])
                self.assertIn("Windows jobs: skipped", ci.summary(result))
                # What the paths plan otherwise stays: only the Windows and macOS jobs go.
                paths = ci.select(list(entries), CRATES, ROOT)["lanes"]
                self.assertEqual(result["lanes"], [lane for lane in paths if lane not in ci.WINDOWS_ONLY_LANES + ci.MACOS_ONLY_LANES])
                dropped |= set(paths) & set(ci.WINDOWS_ONLY_LANES)
        # Before the rule these pull requests waited for `windows check` and the Windows suites.
        self.assertTrue({"windows-check", "windows-e2e", "windows-desktop"} <= dropped)

    def test_recent_pull_requests_with_a_windows_path_plan_the_windows_jobs_that_prove_it(self):
        for number, (entries, proved) in WINDOWS_PULL_REQUESTS.items():
            with self.subTest(pr=number):
                result = ci.pull_request_systems(ci.select(list(entries), CRATES, ROOT), list(entries), CRATES, ROOT)
                self.assertTrue(result["windows"]["runs"])
                self.assertEqual(set(result["windows"]["lanes"]), proved)
                self.assertEqual(set(result["lanes"]) & set(ci.WINDOWS_ONLY_LANES), proved & set(ci.WINDOWS_ONLY_LANES))
                # `verify` requires each planned Windows-only lane and refuses the others.
                ci.aggregate(needs_for(result))
                for lane in ci.WINDOWS_ONLY_LANES:
                    with self.assertRaises(ValueError):
                        ci.aggregate(needs_for(result, **{lane: "skipped" if lane in proved else "success"}))
        # A package input runs `package`, which builds the Windows installer.
        self.assertIn("package", pr_plan("hide-agent-hooks/src/memory.rs")["lanes"])

    def test_main_pushes_the_nightly_and_unreadable_plans_run_every_windows_job(self):
        for event in ("push", "schedule", "workflow_dispatch", "workflow_call"):
            with self.subTest(event=event):
                result = ci.plan(event, None, None, ROOT)
                self.assertTrue(result["windows"]["runs"])
                self.assertEqual(result["windows"]["lanes"], list(ci.WINDOWS_LANES))
                self.assertEqual(result["lanes"], FULL)
                self.assertTrue(set(ci.WINDOWS_ONLY_LANES) <= set(result["lanes"]))
        # The nightly owns agy in its own job on every system; the other Windows lanes are verify's.
        nightly = ci.plan("schedule", None, None, ROOT, nightly_agent_checks=True)
        self.assertEqual(nightly["windows"]["lanes"], list(ci.WINDOWS_LANES))
        self.assertTrue(set(ci.WINDOWS_ONLY_LANES) - {"agy-contract"} <= set(nightly["lanes"]))
        unreadable = ci.plan("pull_request", "0" * 40, "HEAD", ROOT)
        self.assertEqual(unreadable["windows"]["lanes"], list(ci.WINDOWS_LANES))
        self.assertIn("comparison unavailable", unreadable["windows"]["why"][0])
        self.assertTrue(ci.windows_path("T", "web/src/store.ts", CRATES))

    def test_a_merge_group_and_a_draft_plan_no_windows_job(self):
        group = ci.plan("merge_group", None, None, ROOT)
        self.assertEqual(group["lanes"], list(ci.MERGE_GROUP_LANES))
        self.assertEqual(group["windows"], {"runs": False, "lanes": [], "why": ["merge group: no Windows job"]})
        draft = ci.plan("pull_request", None, None, ROOT, draft=True)
        self.assertEqual(draft["lanes"], [])
        self.assertFalse(draft["windows"]["runs"])

    def test_each_kind_of_windows_path_plans_the_windows_jobs_that_prove_it(self):
        every = set(ci.WINDOWS_LANES)
        for path, proved in (
            # CI that defines the Windows jobs, and this plan: every one.
            (".github/workflows/web-e2e.yml", every), (".github/actions/windows-fixture-path/action.yml", every),
            ("scripts/ci-plan.py", every),
            # A path no rule narrows: every Windows job its plan has.
            ("mystery/file.txt", every - {"holder-restart", "agent-clis"}),
            # Rust: `windows check` and the Windows legs of the crate's lanes, not the e2e suites.
            ("Cargo.lock", every - {"windows-e2e", "windows-desktop", "agent-journey", "holder-restart", "agent-clis"}),
            ("hide-core/Cargo.toml", RUST_LEGS), ("hided/build.rs", RUST_LEGS),
            ("hide-platform/src/process.rs", {"windows-check", "windows-wsl", "os-contract", "omp-contract"}),
            ("hide-shell/src/lib.rs", {"windows-check"}),
            ("vendor/portable-pty/src/win/conpty.rs", {"windows-check", "vt", "holder-restart"}),
            ("hide-node/src/ssh/wsl.rs", {"windows-check", "windows-wsl"}),
            ("hide-kit/src/cli_windows.rs", {"windows-check", "windows-wsl", "agy-contract", "os-contract"}),
            # Pinned ConPTY: the legs that fetch it and the Windows e2e's own-engine job.
            ("contracts/conpty.json", {"windows-e2e", "vt", "holder-restart"}),
            # Web code whose Windows form differs: the Windows e2e suites.
            ("web/src/keys.ts", {"windows-e2e", "windows-desktop"}),
            ("web/src/revealExternal.ts", {"windows-e2e", "windows-desktop"}),
            ("web/e2e/windows-command.ts", {"windows-check", "windows-e2e", "windows-desktop"}),
            # The desktop host's Windows code: its unit suite, the Windows desktop
            # suite, and the Windows web suite, whose specs import the wire path.
            ("desktop/src/main/wirePath.ts", {"windows-check", "windows-desktop", "windows-e2e"}),
            # The journey's Windows forms.
            ("scripts/tests/test_agent_live_windows.py", {"agent-journey"}),
            ("scripts/agent_live_check/windows.py", {"agent-journey", "holder-restart"}),
            # A C `_WIN32` branch and a shell's `MINGW*` case: their lanes' Windows legs.
            ("web/e2e/shims/spawn-provider.c", {"windows-check", "windows-e2e", "windows-desktop", "agent-journey"}),
            ("web/e2e/shims/claude-shim.c", {"windows-check", "windows-e2e", "windows-desktop", "agent-journey"}),
            ("web/e2e/shims/live-check.h", {"windows-check", "windows-e2e", "windows-desktop", "agent-journey"}),
            ("scripts/check-relay-protocol.sh", {"relay-protocol"}),
            ("scripts/build-relay-wasm.sh", {"relay-protocol"}),
            # The pins a Windows leg installs or runs from.
            ("contracts/agent-clis.json", {"agy-contract", "holder-restart", "agent-clis"}),
            ("contracts/agent-journey.json", {"holder-restart", "agent-journey", "agent-clis"}),
            # The installer's Windows smoke: `package`, which builds and starts it.
            ("desktop/scripts/smoke-windows-installer.mjs", set()),
        ):
            with self.subTest(path=path):
                result = pr_plan(path)
                self.assertTrue(result["windows"]["runs"], result["windows"])
                self.assertEqual(set(result["windows"]["lanes"]), proved)
                self.assertIn("Windows jobs: run", ci.summary(result))
        # A deleted code file cannot be read for its Windows code; a change that is
        # not an add, modify or delete is unreadable.
        self.assertIn("windows-check", pr_plan("hide-core/src/missions.rs", status="D")["lanes"])
        self.assertTrue(ci.windows_path("T", "docs/TESTING.md", CRATES))

    def test_what_no_windows_job_proves_plans_none(self):
        for status in ("A", "M"):
            for path in (
                "docs/TESTING.md", "AGENTS.md", "design/tokens.json", "site/src/content/docs/docs/start/install-windows.md",
                "scripts/tests/test_fetch_conpty.py", "agents/prd/x.md",
                # Code with no Windows form: the core, the shell, the desktop host's shared code, specs.
                "hide-core/src/missions.rs", "web/src/store.ts", "web/src/App.tsx", "desktop/src/main/windowState.ts",
                "web/e2e/sleeping-tab-live-elsewhere.spec.ts", "web/e2e/card-keyboard.spec.ts", "mobile/android/app/build.gradle",
            ):
                with self.subTest(status=status, path=path):
                    self.assertNoWindows(pr_plan(path, status=status))
        # A CI definition with no Windows job a pull request runs plans no
        # Windows lane; `package`, which every CI edit plans, takes its own.
        self.assertNoWindows(pr_plan(".github/workflows/nightly.yml"))
        self.assertNoWindows(pr_plan("docs/TESTING.md", "design/tokens.json", status="D"))

    def test_every_package_input_runs_the_package_lane_and_its_windows_jobs(self):
        # `package` builds the Windows installer and the Windows device kit and has no form without them.
        tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.splitlines()
        for pattern in ci.PACKAGE_PATHS:
            with self.subTest(pattern=pattern):
                name = next(name for name in tracked if fnmatchcase(name, pattern) and not ci.is_unit_test(name))
                result = pr_plan(name)
                self.assertIn("package", result["lanes"])
                self.assertTrue(result["windows"]["runs"])
                self.assertIn("Windows jobs: run", ci.summary(result))

    def test_every_windows_path_names_a_tracked_file_and_the_marker_finds_windows_code(self):
        tracked = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.splitlines()
        for pattern in ci.WINDOWS_PATHS + ci.WINDOWS_LAYER:
            with self.subTest(pattern=pattern):
                self.assertTrue(any(fnmatchcase(name, pattern) for name in tracked), f"{pattern} names no file")
        marked = {name for name in tracked if name.endswith(ci.CODE_SUFFIXES) and name.startswith(("hide-platform/", "desktop/src/main/"))
                  and ci.WINDOWS_CODE.search((ROOT / name).read_text(encoding="utf-8", errors="replace"))}
        self.assertTrue({"hide-platform/src/process.rs", "hide-platform/src/host.rs", "desktop/src/main/wirePath.ts"} <= marked)
        # The macOS and Linux login items, and a window's bounds, are no Windows form.
        self.assertFalse({"hide-platform/src/user_agents/launchd.rs", "hide-platform/src/user_agents/systemd.rs",
                          "desktop/src/main/windowState.ts"} & marked)
        for code, windows in (('#[cfg(windows)]\nfn a() {}', True), ('#[cfg(any(windows, target_os = "linux"))]', True),
                              ('#[cfg_attr(windows, path = "w.rs")]', True), ('#[cfg(target_os = "windows")]', True),
                              ('use std::os::windows::fs::OpenOptionsExt;', True), ('process.platform === "win32"', True),
                              ('if os.name == "nt":', True), ("if os.name != 'nt':", True),
                              ('#[cfg(\n    windows\n)]', True), ('#[cfg(all(not(unix), windows))]', True),
                              ('#ifdef _WIN32', True), ('#if defined(_MSC_VER)', True),
                              ('case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) suffix=.exe ;; esac', True),
                              ('#[cfg(unix)]\nfn windows() {}', False), ('cfg!(unix);\nlet windows = 1;', False),
                              ('test("x", { tag: "@platform" })', False), ('#[cfg(unix)]\nfn a() {}', False),
                              ('let windows = 2;', False), ('// Windows Terminal', False), ('windows_sys::Win32', False)):
            with self.subTest(code=code):
                self.assertEqual(bool(ci.WINDOWS_CODE.search(code)), windows)

    def test_every_workflow_a_pull_request_runs_on_windows_is_a_windows_path(self):
        jobs = pr_jobs()
        called = {re.search(r"uses: \./\.github/workflows/(\S+\.yml)", block).group(1)
                  for block in jobs.values() if "uses: ./.github/workflows/" in block}
        for name in sorted(called):
            text = (ROOT / ".github/workflows" / name).read_text()
            if "windows-latest" in text:
                with self.subTest(workflow=name):
                    self.assertTrue(ci.windows_path("M", f".github/workflows/{name}", CRATES))
        # `repeat.yml` and `remote-mailbox.yml` name no Windows runner: the
        # repeat's Windows unit leg takes its runner from `ci-repeat.py`.
        for name in ("repeat.yml", "remote-mailbox.yml"):
            self.assertIsNone(ci.windows_path("M", f".github/workflows/{name}", CRATES))
            self.assertTrue(pr_plan(f".github/workflows/{name}")["full"])

    def test_only_the_windows_lanes_name_a_windows_runner(self):
        # Every pr.yml job that can start a Windows runner is a Windows-only lane,
        # a lane with a Windows leg, or `package`; `windows check`, `windows wsl`
        # and `package` call workflows that name their Windows runners themselves.
        for lane, block in pr_jobs().items():
            with self.subTest(lane=lane):
                body = "\n".join(line for line in block.splitlines() if not line.lstrip().startswith("#"))
                names_windows = bool(re.search(r"windows-latest|\"windows\"|include-windows:|wsl-device\.yml|windows-check\.yml|package\.yml", body))
                self.assertEqual(names_windows, lane in set(ci.WINDOWS_LANES) | {"package"})

    def test_pr_yml_runs_the_windows_legs_the_plan_names(self):
        jobs = pr_jobs()
        self.assertIn("windows-legs: ${{ steps.plan.outputs.windows-legs }}", jobs["plan"])
        for lane in ("vt", "holder-restart", "omp-contract"):
            with self.subTest(lane=lane):
                expression = re.search(r"\n      systems: \$\{\{(.+?)\}\}", jobs[lane]).group(1)
                self.assertEqual(system_legs(expression, "true", [lane]), ["linux", "macos", "windows"])
                self.assertEqual(system_legs(expression, "false", [lane]), ["linux", "windows"])
                self.assertEqual(system_legs(expression, "true", []), ["linux", "macos"])
                # Another lane's Windows leg is not this one's.
                self.assertEqual(system_legs(expression, "false", ["os-contract"] if lane != "os-contract" else []), ["linux"])
        for lane in ("os-contract", "agent-journey", "agent-clis"):
            with self.subTest(lane=lane):
                expression = re.search(r"\n      systems: \$\{\{(.+?)\}\}", jobs[lane]).group(1)
                self.assertEqual(system_legs(expression, "true", [lane]), ["linux", "windows"])
                self.assertEqual(system_legs(expression, "true", ["vt"]), ["linux"])
        relay = re.search(r"\n      include-windows: \$\{\{(.+?)\}\}", jobs["relay-protocol"]).group(1)
        self.assertIs(evaluate(relay, "true", ["relay-protocol"]), True)
        self.assertIs(evaluate(relay, "true", ["vt"]), False)
        workflow = (ROOT / ".github/workflows/relay-protocol.yml").read_text()
        matrix = re.search(r"\n        os: \$\{\{ fromJSON\((.+?)\) \}\}", workflow).group(1)
        for macos, windows, systems in ((True, True, ["ubuntu-latest", "windows-latest", "macos-15"]),
                                        (False, True, ["ubuntu-latest", "windows-latest"]),
                                        (True, False, ["ubuntu-latest", "macos-15"]), (False, False, ["ubuntu-latest"])):
            code = (matrix.replace("inputs.include-windows", repr(windows)).replace("inputs.include-macos", repr(macos))
                    .replace("&&", " and ").replace("||", " or "))
            self.assertEqual(json.loads(eval(code, {"__builtins__": {}, "format": lambda text, *values: text.format(*values)})), systems)
        # The Windows-only lanes follow the plan's lanes, which leave them out.
        for lane in ci.WINDOWS_ONLY_LANES:
            self.assertIn(f"if: contains(fromJSON(needs.plan.outputs.lanes), '{lane}')", jobs[lane])
        self.assertIn("systems: '[\"windows\"]'", jobs["agy-contract"])

    def test_the_plan_writes_the_windows_legs_pr_yml_reads(self):
        with tempfile.TemporaryDirectory() as directory:
            pr = PullRequest(directory)
            pr.commit("main", "base", {"docs/a.md": "one\n"})
            pr.git("checkout", "-qb", "topic")
            pr.commit("topic", "relay", {"hide-relay-protocol/src/lib.rs": '#[cfg(windows)]\nfn a() {}\n'})
            pr.check_out_merge()
            output = Path(directory) / "output"
            plan = ci.plan

            def plan_the_fixture(*args, **kwargs):
                # The command plans the checkout its script is in. In CI that is
                # the pull request's own merge commit, whose paths would decide
                # the legs, so the fixture is named as the checkout to read.
                return plan(*args, root=Path(directory), **kwargs)

            environment = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", ""), "GITHUB_OUTPUT": str(output)}
            with unittest.mock.patch.object(ci, "plan", plan_the_fixture), \
                    unittest.mock.patch.object(sys, "argv", ["ci-plan.py", "plan", "--event", "pull_request"]), \
                    unittest.mock.patch.dict(os.environ, environment, clear=True), \
                    contextlib.redirect_stdout(io.StringIO()) as printed:
                ci.main()
            # The plan compared the fixture's merge commit, not this checkout's.
            self.assertEqual(json.loads(printed.getvalue())["reasons"]["policy"][0].split(":")[0], "crate graph unavailable")
            written = dict(row.split("=", 1) for row in output.read_text().splitlines())
            # The crate graph comes from the fixture, which has no workspace, so every lane is planned.
            self.assertEqual(json.loads(written["windows-legs"]), list(ci.WINDOWS_LEG_LANES))
            self.assertEqual(written["body-only"], "false")


# Real Windows items, cut from the files whose Windows code most often met a
# pull request (2026-11-21): a `cfg!` branch inside a function
# (`hide-kit/src/layout.rs`), a `cfg!` constant (`hide-agent-hooks/src/runtime.rs`),
# a `cfg!` early return (`hide-kit/src/agents.rs`) and attributes on imports
# inside a test (`hided/src/cli.rs`).
LAYOUT = """/// The folders a fixture HOME may be made in.
fn temporary_roots(own: &Path) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![own.join("AppData").join("Local").join("Temp")]
    } else {
        let _ = own;
        vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")]
    }
}

/// The store of the skills and MCP items.
pub fn store_dir(home: &Path) -> PathBuf {
    home.join(".hide").join("store")
}
"""
RUNTIME = """/// The helper's name.
pub const HELPER_BINARY_NAME: &str = if cfg!(windows) {
    "hide-agent-hooks.exe"
} else {
    "hide-agent-hooks"
};

/// The pane's `hide_hooks` reporting protocol.
pub const PROTOCOL: u32 = 4;
"""
AGENTS = """pub(crate) fn login_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    hide_platform::host::default_shell()
        .inspect_err(|error| eprintln!("kit.login_shell_missing kind={:?}", error.kind()))
        .ok()
}
"""
CLI = """    #[test]
    fn a_daemon_that_exits_ends_the_wait_at_the_next_look() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        let start = std::time::Instant::now();
        let error = wait(|| std::process::ExitStatus::from_raw(256));
        assert!(error.contains("'{'"), "{error}");
    }
"""


class RustChange(unittest.TestCase):
    """A shared Rust file plans `windows check` only when its change touches the file's Windows code."""

    def test_the_item_reader_finds_each_windows_item_and_where_it_ends(self):
        self.assertEqual(ci.windows_items(LAYOUT), [(3, 8)])
        self.assertEqual(ci.windows_items(RUNTIME), [(2, 6)])
        self.assertEqual(ci.windows_items(AGENTS), [(2, 4)])
        # The attribute and its import, twice: the marker in the attribute and the one in the path.
        self.assertEqual(ci.windows_items(CLI), [(5, 6), (6, 6)])
        self.assertEqual(ci.windows_items("fn a() {}\n"), [])
        # A comment that names Windows code is not Windows code.
        self.assertEqual(ci.windows_items("// #[cfg(windows)]\nfn a() {}\n"), [])
        # An inner attribute holds the whole file.
        self.assertEqual(ci.windows_items('#![cfg(windows)]\nfn a() {}\nfn b() {}\n'), [(1, 4)])

    def test_a_file_the_reader_cannot_place_counts_as_windows(self):
        for text, why in ((LAYOUT.replace("    }\n}", "    }\n", 1), "brackets"),
                          ('fn a() { let s = "open;\n #[cfg(windows)] fn b() {}', "string"),
                          ("/* open\n#[cfg(windows)]\nfn b() {}", "comment"),
                          ('macro_rules! m { () => { #[cfg(windows)] fn b() {} } }', "macro"),
                          ('fn a() { println!("{}", cfg!(windows)); }', "macro")):
            with self.subTest(why=why):
                with self.assertRaises(ci.Unreadable):
                    ci.windows_items(text)

    def plan(self, name, before, after, declared=None):
        """The plan of a pull request that changes one shared Rust file from
        `before` to `after`, in a crate whose `lib.rs` declares its module
        (`declared`, a plain `mod` by default)."""
        crate, module = name.split("/")[0], PurePosixPath(name).stem
        with tempfile.TemporaryDirectory() as directory:
            pr = PullRequest(directory)
            pr.commit("main", "base", {name: before, "docs/a.md": "one\n", f"{crate}/Cargo.toml": "[package]\n",
                                       f"{crate}/src/lib.rs": declared or f"mod {module};\n"})
            pr.git("checkout", "-qb", "topic")
            if after is None:
                pr.git("rm", "-q", name)
                pr.git("commit", "-qm", "delete")
            else:
                pr.commit("topic", "change", {name: after})
            pr.check_out_merge()
            return pr.plan(None)

    def test_a_change_inside_a_windows_item_plans_windows_check(self):
        name = "hide-core/src/layout.rs"
        for after, why in (
            (LAYOUT.replace('join("Temp")', 'join("Tmp")'), "a change inside a Windows item: hide-core/src/layout.rs:4"),
            # Its non-Windows branch is part of the same statement, which is read whole.
            (LAYOUT.replace('"/private/tmp"', '"/var/tmp"'), "a change inside a Windows item: hide-core/src/layout.rs:7"),
            # A line that names Windows code, added or removed.
            (LAYOUT.replace("fn store_dir", "#[cfg(windows)]\nfn store_dir"), "Windows code changed: hide-core/src/layout.rs"),
            (LAYOUT.replace("    if cfg!(windows) {", "    if false {"), "Windows code changed: hide-core/src/layout.rs"),
        ):
            with self.subTest(why=why):
                result = self.plan(name, LAYOUT, after)
                self.assertIn("windows-check", result["lanes"])
                self.assertTrue(any(reason.startswith(why) for reason in result["windows"]["why"]), result["windows"])
                self.assertNotIn("windows-e2e", result["lanes"])

    def test_a_change_beside_or_far_from_the_windows_items_plans_none(self):
        name = "hide-core/src/layout.rs"
        for after in (
            # The line right after the item, and the doc comment right before it.
            LAYOUT.replace("}\n\n/// The store", "}\n// beside\n\n/// The store"),
            LAYOUT.replace("/// The folders a fixture HOME may be made in.", "/// The folders a fixture HOME is made in."),
            # Another function, far from the item.
            LAYOUT.replace('join("store")', 'join("items")'),
        ):
            with self.subTest(after=after[:60]):
                result = self.plan(name, LAYOUT, after)
                self.assertNoWindows(result)
        result = self.plan("hide-core/src/runtime.rs", RUNTIME, RUNTIME.replace("= 4;", "= 5;"))
        self.assertNoWindows(result)
        self.assertIn("rust", result["lanes"])

    def assertNoWindows(self, result):
        self.assertEqual(result["windows"]["lanes"], [], result["windows"])
        self.assertFalse(set(result["lanes"]) & set(ci.WINDOWS_ONLY_LANES))
        ci.aggregate(needs_for(result))

    def test_a_deleted_windows_block_or_an_unreadable_file_plans_windows_check(self):
        name = "hide-core/src/agents.rs"
        deleted = AGENTS.replace("    if cfg!(windows) {\n        return None;\n    }\n", "")
        self.assertIn("windows-check", self.plan(name, AGENTS, deleted)["lanes"])
        # A deleted file whose lines held Windows code, and one whose did not.
        self.assertIn("windows-check", self.plan(name, AGENTS, None)["lanes"])
        self.assertNoWindows(self.plan(name, "fn a() {}\n", None))
        # Braces that do not balance: the reader cannot place the change, so Windows runs.
        unbalanced = LAYOUT + "fn broken() {\n"
        result = self.plan(name, unbalanced, unbalanced.replace('join("store")', 'join("items")'))
        self.assertIn("windows-check", result["lanes"])
        self.assertTrue(any("cannot place the change" in reason for reason in result["windows"]["why"]))

    def test_a_multi_line_or_nested_windows_cfg_is_an_item(self):
        for attribute in ("#[cfg(\n    windows\n)]", "#[cfg(all(not(unix), windows))]",
                          '#[cfg(not(any(\n    target_os = "macos",\n    all(target_os = "linux", target_env = "gnu"),\n    windows\n)))]'):
            with self.subTest(attribute=attribute):
                before = f"{attribute}\nfn only_windows() {{\n    old();\n}}\n\nfn shared() {{}}\n"
                result = self.plan("hide-core/src/conditional.rs", before, before.replace("old();", 'compile_error!("x");'))
                self.assertIn("windows-check", result["lanes"])
                self.assertIn("a change inside a Windows item", " ".join(result["windows"]["why"]))
                self.assertNoWindows(self.plan("hide-core/src/conditional.rs", before, before.replace("fn shared() {}", "fn shared() { a(); }")))

    def test_a_module_declared_for_windows_is_windows_whatever_line_changed(self):
        name = "hide-core/src/console.rs"
        plain = "fn read() {\n    one();\n}\n"
        changed = plain.replace("one", "two")
        for declared in ("#[cfg(windows)]\nmod console;\n", "#[cfg(windows)]\nmod sys {\n    #[path = \"../console.rs\"]\n    mod console;\n}\n"):
            with self.subTest(declared=declared):
                result = self.plan(name, plain, changed, declared=declared)
                self.assertIn("windows-check", result["lanes"])
                self.assertIn("a Windows module: hide-core/src/console.rs", " ".join(result["windows"]["why"]))
        self.assertNoWindows(self.plan(name, plain, changed, declared="#[cfg(unix)]\nmod other;\nmod console;\n"))
        # A file no module declares cannot be shown to build off Windows too.
        result = self.plan(name, plain, changed, declared="mod other;\n")
        self.assertIn("windows-check", result["lanes"])
        self.assertIn("no file declares its module", " ".join(result["windows"]["why"]))
        # The real one: the platform layer's console reader, declared under `cfg(windows)` in `process.rs`.
        real = pr_plan("hide-platform/src/process/console_input.rs")
        self.assertIn("a Windows module", real["windows"]["why"][0])
        self.assertEqual(set(real["windows"]["lanes"]), {"windows-check", "windows-wsl", "os-contract", "omp-contract"})

    def test_a_git_read_that_fails_counts_as_windows(self):
        name = "hide-core/src/agents.rs"
        deleted = AGENTS.replace("        return None;\n", "")
        with tempfile.TemporaryDirectory() as directory:
            pr = PullRequest(directory)
            pr.commit("main", "base", {name: AGENTS, "hide-core/Cargo.toml": "[package]\n", "hide-core/src/lib.rs": "mod agents;\n"})
            pr.git("checkout", "-qb", "topic")
            pr.commit("topic", "change", {name: deleted})
            base, head = pr.git("rev-parse", "main").strip(), pr.git("rev-parse", "topic").strip()
            change = ci.Change(base, head, Path(directory))
            self.assertIn("a change inside a Windows item", ci.windows_change(name, change))
            # A commit that cannot be read is no answer about the file, so Windows runs.
            broken = ci.Change(base, head, Path(directory))
            broken.base = "0" * 40
            self.assertIn("cannot place the change", ci.windows_change(name, broken))
            # A file the commit does not hold is empty, which is an answer.
            self.assertEqual(change.text("old", "hide-core/src/none.rs"), "")
            # The commit is there but its blob cannot be read: the plan still runs Windows.
            pr.check_out_merge()
            self.assertEqual(set(pr.plan(None)["windows"]["lanes"]), {"windows-check", "os-contract"})
            run = subprocess.run

            def blob_unreadable(command, *args, **kwargs):
                if command[:2] == ["git", "show"] and command[2] == f"HEAD^1:{name}":
                    return subprocess.CompletedProcess(command, 128, b"", b"fatal: unable to read blob\n")
                return run(command, *args, **kwargs)
            with unittest.mock.patch.object(ci.subprocess, "run", blob_unreadable):
                result = pr.plan(None)
            self.assertEqual(set(result["windows"]["lanes"]), {"windows-check", "os-contract"})
            self.assertIn(f"cannot read {name} at HEAD^1", " ".join(result["windows"]["why"]))

    def test_the_windows_layer_is_read_a_whole_file_at_a_time(self):
        # `hide-platform` writes each system's form: any change to a file with Windows code runs Windows.
        for name in ("hide-platform/src/layout.rs", "hide-holder/src/layout.rs"):
            with self.subTest(name=name):
                result = self.plan(name, LAYOUT, LAYOUT.replace('join("store")', 'join("items")'))
                self.assertIn("windows-check", result["lanes"])
                self.assertIn(f"Windows code: {name}", " ".join(result["windows"]["why"]))


class Aggregate(unittest.TestCase):
    def test_a_sparse_checkout_carries_every_script_its_scripts_load(self):
        # A job that checks out only some scripts dies at import when one of them
        # loads a sibling the list left out, before it can report on any lane.
        for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
            for block in re.findall(r"sparse-checkout: \|\n((?:[ \t]+\S.*\n)+?)[ \t]+sparse-checkout-cone-mode", workflow.read_text()):
                listed = {line.strip() for line in block.splitlines()}
                for script in sorted(path for path in listed if path.endswith(".py")):
                    for sibling in re.findall(r'Path\(__file__\)\.resolve\(\)\.parent / "([^"]+\.py)"', (ROOT / script).read_text()):
                        self.assertIn(f"scripts/{sibling}", listed, f"{workflow.name}: {script} loads {sibling}")

    def test_the_nightly_runs_linux_and_windows_daily_and_macos_twice_a_week(self):
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        crons = re.findall(r'- cron: "([^"]+)"', nightly)
        self.assertEqual(sorted(crons), sorted(ci.NIGHTLY_SCHEDULES))
        # Every weekday is one schedule's, never two.
        days = [day for cron in crons for day in cron.split()[-1].split(",")]
        self.assertEqual(sorted(days), [str(day) for day in range(7)])
        scopes = {lane: ci.nightly_scope("", cron) for cron, lane in ci.NIGHTLY_SCHEDULES.items()}
        daily, full = scopes["daily"], scopes["all"]
        self.assertEqual(daily["systems"], ["linux", "windows"])
        self.assertEqual(full["systems"], ["linux", "macos", "windows"])
        self.assertEqual((daily["canary"], full["canary"]), (["linux"], ["linux", "macos"]))
        self.assertEqual((daily["package-macos"], full["package-macos"]), (False, True))
        # No macOS job on a daily run: its macOS-only jobs and `verify`, which runs macOS lanes.
        for job in ("web-e2e-macos", "remote-mailbox-macos", "perf", "verify"):
            self.assertNotIn(job, daily["jobs"])
            self.assertIn(job, full["jobs"])
        self.assertNotIn("macos", daily["desktop"])
        # A hand run names its lane; the schedule's cron line names it otherwise.
        self.assertEqual(ci.nightly_scope("macos", "17 17 * * 1,2,4,5,6")["lane"], "macos")
        for lane, schedule in (("", ""), ("", "0 0 * * *"), ("nightly", "")):
            with self.subTest(lane=lane, schedule=schedule), self.assertRaises(ValueError):
                ci.nightly_scope(lane, schedule)

    def test_nightly_requires_selected_jobs_and_rejects_failed_or_missing_agent_checks(self):
        jobs = set(ci.NIGHTLY_JOBS)
        for lane, selected in (
            ("all", jobs - {"web-e2e-windows"}),
            ("daily", {"agy-contract-all", "agent-trust-contract-all", "agent-journey", "holder-restart", "desktop-e2e",
                       "web-e2e-windows", "package"}),
            ("linux", {"agy-contract-all", "agent-trust-contract-all", "agent-journey", "holder-restart", "desktop-e2e"}),
            ("macos", {"agy-contract-all", "agent-trust-contract-all", "agent-journey", "holder-restart", "desktop-e2e",
                       "web-e2e-macos", "remote-mailbox-macos", "perf"}),
            ("windows", {"agy-contract-all", "agent-trust-contract-all", "agent-journey", "holder-restart", "desktop-e2e",
                         "web-e2e-windows"}),
            ("agy", {"agy-contract-all", "agent-trust-contract-all"}),
        ):
            self.assertEqual(set(ci.nightly_scope(lane)["jobs"]), selected)
            needs = {job: {"result": "success" if job in selected else "skipped"} for job in jobs}
            needs["scope"] = {"result": "success"}
            ci.nightly_aggregate(needs, lane)
            for job in jobs | {"scope"}:
                # Includes the direct agy and journey calls: a failed OS,
                # stand-in or forms job fails that call, then this aggregate.
                for outcome in {"success", "skipped", "failure", "cancelled"} - {needs[job]["result"]}:
                    with self.subTest(lane=lane, job=job, outcome=outcome):
                        with self.assertRaisesRegex(ValueError, job):
                            ci.nightly_aggregate({**needs, job: {"result": outcome}}, lane)
            with self.assertRaisesRegex(ValueError, "job contract"):
                ci.nightly_aggregate({job: value for job, value in needs.items() if job != "agent-journey"}, lane)
        with self.assertRaisesRegex(ValueError, "no lane"):
            ci.nightly_aggregate(needs, "unknown")
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        report = nightly.split("\n  report:\n", 1)[1]
        actual = {job.strip() for job in re.search(r"needs: \[([^\]]+)\]", report).group(1).split(",")}
        # The agent canary's jobs are not the report's to read.
        self.assertEqual(actual, jobs | {"scope"})
        self.assertIn('run: python3 scripts/ci-plan.py nightly-aggregate --lane "$LANE"', report)
        self.assertIn("NEEDS: ${{ toJSON(needs) }}", report)
        self.assertIn("if: ${{ always() }}", report.split("steps:", 1)[0])
        reporting = report.split("- name: Open, comment on and close", 1)[1]
        self.assertIn("if: ${{ always() && github.ref == 'refs/heads/main' }}", reporting)
        # Each selectable job reads the scope's job list in its `if:`.
        for job in jobs:
            block = nightly.split(f"\n  {job}:\n", 1)[1].split("\n\n", 1)[0]
            self.assertIn(f"if: ${{{{ contains(fromJSON(needs.scope.outputs.jobs), '{job}') }}}}", block, job)
        canary = [name for name in re.findall(r"\n  (canary-[a-z]+):\n", nightly)]
        self.assertEqual(canary, ["canary-latest", "canary-journey", "canary-record"])
        for job in canary:
            name = re.search(r"name: (.+)", nightly.split(f"\n  {job}:\n", 1)[1]).group(1)
            self.assertTrue(name.startswith("agent canary "), name)

    def test_planned_success_and_unplanned_skip_pass(self):
        for result in (plan("docs/BUILD.md"), plan("web/src/Overview.tsx"), plan(".github/workflows/pr.yml"),
                       plan("hide-core/src/lib.rs")):
            ci.aggregate(needs_for(result))

    def test_a_crate_only_plan_fails_when_the_desktop_suite_ran(self):
        with self.assertRaisesRegex(ValueError, "desktop-e2e: not planned"):
            ci.aggregate(needs_for(plan("hide-core/src/lib.rs"), desktop_e2e="success"))

    def test_a_planned_lane_that_did_not_succeed_fails(self):
        result = plan("web/src/shortcuts.ts")
        for outcome in ("skipped", "failure", "cancelled"):
            with self.subTest(outcome=outcome), self.assertRaises(ValueError):
                ci.aggregate(needs_for(result, desktop_e2e=outcome))

    def test_an_unplanned_lane_that_ran_fails(self):
        with self.assertRaises(ValueError):
            ci.aggregate(needs_for(plan("docs/BUILD.md"), rust="success"))

    def test_a_draft_fails_verify_even_when_every_lane_was_skipped(self):
        needs = {lane: {"result": "skipped"} for lane in ci.LANES}
        needs["plan"] = {"result": "success", "outputs": {"lanes": "[]", "draft": "true"}}
        with self.assertRaisesRegex(ValueError, "draft: lanes not run, mark ready for review"):
            ci.aggregate(needs)
        needs["plan"]["outputs"] = {"lanes": json.dumps(["policy"]), "draft": "false"}
        needs["policy"] = {"result": "success"}
        ci.aggregate(needs)

    def test_a_missing_or_unknown_lane_or_a_failed_plan_fails(self):
        needs = needs_for(plan("docs/BUILD.md"))
        del needs["rust"]
        with self.assertRaises(ValueError):
            ci.aggregate(needs)
        with self.assertRaises(ValueError):
            ci.aggregate({**needs_for(plan("docs/BUILD.md")), "extra": {"result": "success"}})
        failed = needs_for(plan("docs/BUILD.md"))
        failed["plan"] = {"result": "failure", "outputs": {}}
        with self.assertRaises(ValueError):
            ci.aggregate(failed)
        empty = needs_for(plan("docs/BUILD.md"))
        empty["plan"]["outputs"]["lanes"] = "[]"
        with self.assertRaises(ValueError):
            ci.aggregate(empty)


class PullRequest:
    """A repository with a main and a pull request branch, merged the way GitHub checks one out."""

    def __init__(self, directory):
        self.repo = Path(directory)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.email", "ci@example.invalid")
        self.git("config", "user.name", "ci")
        self.git("config", "commit.gpgsign", "false")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, branch, message, files):
        # The first commit lands on the branch `init` named, which has none to check out.
        if subprocess.run(["git", "symbolic-ref", "-q", "--short", "HEAD"], cwd=self.repo, capture_output=True,
                          text=True).stdout.strip() != branch:
            self.git("checkout", "-q", branch)
        for name, text in files.items():
            (self.repo / name).parent.mkdir(parents=True, exist_ok=True)
            (self.repo / name).write_text(text)
        self.git("add", ".")
        self.git("commit", "-qm", message)
        return self.git("rev-parse", "HEAD")

    def merge_main_into_topic(self, resolve=None):
        """The author's `git merge main` on the branch, with the conflicted files written as `resolve` says."""
        self.git("checkout", "-q", "topic")
        if subprocess.run(["git", "merge", "-q", "--no-edit", "main"], cwd=self.repo, capture_output=True).returncode:
            for name, text in (resolve or {}).items():
                (self.repo / name).write_text(text)
            self.git("add", ".")
            self.git("commit", "-q", "--no-edit")
        return self.git("rev-parse", "HEAD")

    def check_out_merge(self):
        """GitHub's merge commit: main first, the pull request's head second."""
        self.git("checkout", "-q", "--detach", "main")
        self.git("merge", "-q", "--no-ff", "-m", "merge", "topic")

    def plan(self, last_green, body_edit=False):
        return ci.plan("pull_request", "HEAD^1", "HEAD", self.repo, CRATES, last_green=last_green, body_edit=body_edit)


LINES = "".join(f"line {n}\n" for n in range(1, 21))


class ReuseGreenRun(unittest.TestCase):
    """Reuse unchanged app checks, rerunning policy and any changed registered unit tests."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.pr = PullRequest(directory.name)
        base = self.pr.commit("main", "base", {"docs/a.md": LINES, "web/src/x.ts": LINES, "hide-core/src/lib.rs": "one\n"})
        self.pr.git("checkout", "-qb", "topic")
        head = self.pr.commit("topic", "change", {
            "web/src/x.ts": LINES.replace("line 2\n", "line two\n"),
            "docs/a.md": LINES.replace("line 2\n", "line two\n"),
        })
        # The run whose lanes passed on this head over this main.
        self.green = {"run": 101, "base": base, "head": head}

    def assertRuns(self, result, why):
        self.assertIn("web-e2e", result["lanes"])
        self.assertIsNone(result["reuse"]["run"])
        self.assertIn(why, result["reuse"]["why"])
        self.assertNotIn("verified", result)

    def test_a_push_that_only_merges_main_and_resolves_a_documentation_conflict_reuses_the_run(self):
        # Main changed a file the pull request does not change, and the line
        # of the guide the pull request changed.
        self.pr.commit("main", "main moves", {"hide-core/src/lib.rs": "two\n", "docs/a.md": LINES.replace("line 2\n", "line 2!\n")})
        self.pr.merge_main_into_topic(resolve={"docs/a.md": LINES.replace("line 2\n", "line two!\n")})
        self.pr.commit("topic", "guide", {"docs/b.md": "new\n"})
        self.pr.check_out_merge()
        result = self.pr.plan(lambda: self.green)
        self.assertEqual(result["lanes"], ["policy"])
        self.assertEqual(result["rust_packages"], [])
        self.assertFalse(result["full"])
        self.assertEqual(result["verified"], self.green)
        self.assertIn("reuses run 101", result["reasons"]["policy"][0])
        self.assertIn("[run 101](https://example.invalid/runs/101)", ci.summary(result, "https://example.invalid/runs/"))
        # The next run compares with the run whose lanes ran, not with this one.
        record = ci.plan_record(result, 7, 202)
        self.assertEqual(record["verified"], self.green)
        self.assertEqual(record["head"], self.pr.git("rev-parse", "HEAD^2"))
        # Without the earlier run the same push needs its lanes.
        self.assertIn("web-e2e", self.pr.plan(None)["lanes"])

    def test_main_changing_a_file_the_pull_request_changes_runs_the_lanes(self):
        # Pull request #1133: main's changes to the files it changed merged
        # cleanly, and the lanes still have to see them together.
        self.pr.commit("main", "main moves", {"web/src/x.ts": LINES.replace("line 19\n", "line nineteen\n")})
        self.pr.merge_main_into_topic()
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: self.green), "main changed web/src/x.ts")

    def test_only_registered_unit_checks_rerun_after_a_green_app_run(self):
        # A changed unit test also plans the lane that repeats it; the run
        # that reuses a green one rechecks it, so it plans that lane too. One
        # with no Windows code reruns on Linux alone; the Windows one on Windows too.
        for path, lanes in (("desktop/src/main/x.test.ts", ["policy", "checks", "repeat"]),
                            ("desktop/e2e/windows-processes.unit.ts", ["policy", "checks", "windows-check", "repeat"])):
            with self.subTest(path=path):
                self.pr.git("checkout", "-q", "topic")
                self.pr.git("reset", "--hard", self.green["head"])
                self.pr.commit("topic", "unit assertion", {path: "test\n", "docs/b.md": "guide\n"})
                self.pr.check_out_merge()
                result = self.pr.plan(lambda: self.green)
                self.assertEqual(result["lanes"], lanes)
                self.assertEqual(result["rust_packages"], [])
                self.assertEqual(result["verified"], self.green)
                self.assertEqual(ci.plan_record(result, 7, 202)["verified"], self.green)
                self.assertIn("this run checks " + ", ".join(lanes), ci.summary(result))
                # A lost artifact must run the whole PR's app checks again.
                self.assertIn("web-e2e", self.pr.plan(None)["lanes"])

    def test_modified_and_deleted_unit_tests_are_rechecked_against_the_original_green_anchor(self):
        self.green["head"] = self.pr.commit("topic", "unit fixture", {"desktop/src/main/x.test.ts": "first\n"})
        self.pr.commit("topic", "unit assertion", {"desktop/src/main/x.test.ts": "second\n"})
        self.pr.check_out_merge()
        first = self.pr.plan(lambda: self.green)
        self.assertEqual(first["lanes"], ["policy", "checks", "repeat"])
        # This partial green must not claim that its app lanes checked a new
        # head: later runs still compare with the original app-tested commits.
        anchor = ci.plan_record(first, 7, 202)["verified"]
        self.pr.git("checkout", "-q", "topic")
        self.pr.git("rm", "desktop/src/main/x.test.ts")
        self.pr.git("commit", "-qm", "delete unit test")
        self.pr.check_out_merge()
        # A deleted unit test has no test left to repeat; the pull request
        # adds no Windows code to main, so the suite reruns on Linux alone.
        deleted = self.pr.plan(lambda: anchor)
        self.assertEqual(deleted["lanes"], ["policy", "checks"])
        self.assertEqual(deleted["verified"], self.green)
        self.pr.commit("topic", "product change", {"web/src/x.ts": "changed again\n"})
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: anchor), "the pull request changed web/src/x.ts")

    def test_unit_edits_do_not_reuse_app_checks_when_main_changed_shared_product_code(self):
        self.pr.commit("main", "main moves", {"web/src/x.ts": LINES.replace("line 19\n", "line nineteen\n")})
        self.pr.merge_main_into_topic()
        self.pr.commit("topic", "unit assertion", {"desktop/src/main/x.test.ts": "test\n"})
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: self.green), "main changed web/src/x.ts")

    def test_an_unregistered_test_or_shared_test_configuration_does_not_reuse_app_checks(self):
        for path in ("web/src/x.test.ts", "web/e2e/platform-program.unit.ts", "web/e2e/new.unit.ts",
                     "web/vite.config.ts", "desktop/src/main/new.test.tsx"):
            with self.subTest(path=path):
                self.pr.git("checkout", "-q", "topic")
                self.pr.git("reset", "--hard", self.green["head"])
                self.pr.commit("topic", "unclassified input", {path: "changed\n"})
                self.pr.check_out_merge()
                self.assertRuns(self.pr.plan(lambda: self.green), f"the pull request changed {path}")

    def test_a_conflict_resolved_in_code_runs_the_lanes(self):
        self.pr.commit("main", "main moves", {"web/src/x.ts": LINES.replace("line 2\n", "line 2!\n")})
        self.pr.merge_main_into_topic(resolve={"web/src/x.ts": LINES.replace("line 2\n", "line two!\n")})
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: self.green), "the pull request changed web/src/x.ts")

    def test_a_code_change_since_the_run_runs_the_lanes(self):
        self.pr.commit("topic", "more", {"web/src/x.ts": LINES.replace("line 2\n", "line 2?\n"), "docs/b.md": "new\n"})
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: self.green), "the pull request changed web/src/x.ts")

    def test_a_guide_a_test_reads_is_not_skipped_as_documentation(self):
        self.pr.commit("topic", "guide", {"docs/UI_BEHAVIOR.md": "chords\n"})
        self.pr.check_out_merge()
        self.assertRuns(self.pr.plan(lambda: self.green), "the pull request changed docs/UI_BEHAVIOR.md")

    def test_no_run_a_failed_lookup_or_an_unreadable_record_keeps_the_plan(self):
        self.pr.commit("topic", "guide", {"docs/b.md": "new\n"})
        self.pr.check_out_merge()
        expected = self.pr.plan(None)["lanes"]

        def raises(error):
            def last_green():
                raise error
            return last_green
        for last_green, why in (
            (raises(ci.NotReused("this pull request has no earlier green run")), "no earlier green run"),
            (raises(subprocess.CalledProcessError(1, ["gh"], stderr="HTTP 502")), "HTTP 502"),
            (raises(OSError("gh: not found")), "gh: not found"),
            (raises(ValueError("Expecting value")), "Expecting value"),
            # A record naming a commit this checkout lacks and cannot fetch.
            (lambda: {**self.green, "head": "f" * 40}, "origin"),
        ):
            result = self.pr.plan(last_green)
            self.assertEqual(result["lanes"], expected)
            self.assertIsNone(result["reuse"]["run"])
            self.assertIn(why, result["reuse"]["why"])
            self.assertIn("No earlier green run stands for this one", ci.summary(result))
            self.assertEqual(ci.plan_record(result, 7, 202)["verified"]["run"], 202)

    def test_a_plan_that_is_already_policy_alone_looks_nothing_up(self):
        self.pr.git("checkout", "-q", "main")
        self.pr.git("branch", "-qf", "topic", "main")
        self.pr.commit("topic", "guide", {"docs/b.md": "new\n"})
        self.pr.check_out_merge()

        def last_green():
            raise AssertionError("looked up")
        self.assertEqual(self.pr.plan(last_green)["lanes"], ["policy"])


class BodyEdit(unittest.TestCase):
    """A body edit runs only the checks that read the body when the last green run checked this head."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.pr = PullRequest(directory.name)
        base = self.pr.commit("main", "base", {"docs/a.md": LINES, "web/src/x.ts": LINES})
        self.pr.git("checkout", "-qb", "topic")
        self.head = self.pr.commit("topic", "change", {"web/src/x.ts": LINES.replace("line 2\n", "line two\n")})
        self.pr.check_out_merge()
        # The green run of this head, as `last_green_run` answers it.
        self.green = {"run": 101, "base": base, "head": self.head, "pr_head": self.head, "run_id": 101}

    def test_a_body_edit_of_the_head_a_green_run_checked_runs_only_the_body_check(self):
        result = self.pr.plan(lambda: self.green, body_edit=True)
        self.assertEqual(result["lanes"], ["policy"])
        self.assertIn("which run 101 checked", result["body_only"])
        self.assertIn("`policy` runs only the checks that read the body", ci.summary(result))
        # The record keeps the commits the lanes checked, as any reuse does.
        self.assertEqual(ci.plan_record(result, 7, 202)["verified"], {key: self.green[key] for key in ("run", "base", "head")})
        # The same run started by a push is a reuse, with the whole `policy`.
        self.assertNotIn("body_only", self.pr.plan(lambda: self.green))

    def test_a_body_edit_of_a_documentation_only_pull_request_asks_for_the_green_run(self):
        self.pr.git("checkout", "-q", "topic")
        self.pr.git("reset", "-q", "--hard", "main")
        head = self.pr.commit("topic", "guide", {"docs/a.md": "guide\n"})
        self.pr.check_out_merge()
        result = self.pr.plan(lambda: {**self.green, "pr_head": head}, body_edit=True)
        self.assertEqual(result["lanes"], ["policy"])
        self.assertIn("body_only", result)

    def test_a_body_edit_without_a_green_run_of_this_head_runs_what_it_ran_before(self):
        expected = self.pr.plan(None)
        reused = self.pr.plan(lambda: {**self.green, "pr_head": "f" * 40}, body_edit=True)
        # The last green run checked another head: the plan reuses it as before,
        # and `policy` runs whole, since this head's structural checks never ran.
        self.assertEqual(reused["lanes"], ["policy"])
        self.assertNotIn("body_only", reused)

        def raises():
            raise ci.NotReused("this pull request has no earlier green run")
        for last_green in (raises, None):
            with self.subTest(last_green=last_green):
                result = self.pr.plan(last_green, body_edit=True)
                self.assertEqual(result["lanes"], expected["lanes"])
                self.assertNotIn("body_only", result)

    def test_policy_skips_its_suite_and_structural_checks_only_for_a_body_only_run(self):
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        jobs = pr_jobs()
        self.assertIn("body-only: ${{ steps.plan.outputs.body-only }}", jobs["plan"])
        self.assertIn("--body-edit \"${{ github.event.action == 'edited' }}\"", jobs["plan"])
        self.assertRegex(workflow, r"types: \[[^\]]*\bedited\b[^\]]*\]")
        steps = {re.search(r"name: (.+)", step).group(1).strip(): step
                 for step in re.split(r"\n      - ", jobs["policy"])[1:] if re.search(r"name: (.+)", step)}

        def runs(step, event, body_only):
            condition = re.search(r"\n        if: (.+)", steps[step])
            if not condition:
                return True
            code = (condition.group(1).replace("github.event_name", repr(event))
                    .replace("needs.plan.outputs.body-only", repr(body_only)).replace("&&", " and ").replace("||", " or "))
            return eval(code, {"__builtins__": {}})
        for step in ("Install zsh", "Test the repository scripts, the lane plan included", "Structural checks"):
            with self.subTest(step=step):
                self.assertTrue(runs(step, "pull_request", "false"))
                self.assertFalse(runs(step, "pull_request", "true"))
        # The check that reads the body runs on every pull request run, body-only included.
        body = "A pull request that changes a screen shows it"
        self.assertIn("PR_BODY: ${{ github.event.pull_request.body }}", steps[body])
        self.assertTrue(runs(body, "pull_request", "true"))
        self.assertTrue(runs(body, "pull_request", "false"))
        self.assertFalse(runs(body, "merge_group", "false"))


class LastGreenRun(unittest.TestCase):
    """Which earlier run a pull request reads, and which plan records it accepts."""

    HEAD = "a" * 40
    VERIFIED = {"run": 7, "base": "b" * 40, "head": "c" * 40}

    def gh(self, runs, record):
        calls = []

        def gh(*args):
            calls.append(args)
            if args[0] == "api":
                return json.dumps({"workflow_runs": runs})
            if record is None:
                raise ci.NotReused("`gh run download` failed: no artifact matches")
            directory = Path(args[args.index("--dir") + 1])
            (directory / ci.RECORD_FILE).write_text(json.dumps(record))
            return ""
        return gh, calls

    def run_(self, run_id, created, repo="o/r", head=HEAD):
        return {"id": run_id, "created_at": created, "head_sha": head, "head_repository": {"full_name": repo}}

    def test_the_newest_green_run_of_this_head_repository_is_read(self):
        record = {"version": 1, "pr": 5, "head": self.HEAD, "verified": self.VERIFIED}
        runs = [
            self.run_(1, "2026-10-15T01:00:00Z", head="d" * 40),
            self.run_(2, "2026-10-15T02:00:00Z"),
            # A fork's branch of the same name, and this run itself.
            self.run_(3, "2026-10-15T03:00:00Z", repo="fork/r"),
            self.run_(9, "2026-10-15T04:00:00Z"),
        ]
        gh, calls = self.gh(runs, record)
        # The commits its lanes checked, and the head this run checked itself.
        self.assertEqual(ci.last_green_run("o/r", 5, "o/r", "topic", 9, gh=gh),
                         {**self.VERIFIED, "pr_head": self.HEAD, "run_id": 2})
        query = calls[0]
        self.assertIn("repos/o/r/actions/workflows/pr.yml/runs", query)
        self.assertIn("status=success", query)
        self.assertIn("branch=topic", query)
        self.assertEqual(calls[1][:3], ("run", "download", "2"))
        self.assertIn(ci.RECORD_ARTIFACT, calls[1])

    def test_a_record_that_does_not_belong_or_cannot_be_read_is_refused(self):
        good = {"version": 1, "pr": 5, "head": self.HEAD, "verified": self.VERIFIED}
        for runs, record, why in (
            ([], good, "no earlier green run"),
            ([self.run_(2, "2026-10-15T02:00:00Z")], good, "head repository is gone"),
            ([self.run_(2, "2026-10-15T02:00:00Z", repo="fork/r")], good, "no earlier green run"),
            ([self.run_(2, "2026-10-15T02:00:00Z")], None, "no artifact"),
            ([self.run_(2, "2026-10-15T02:00:00Z")], {**good, "version": 2}, "cannot read"),
            ([self.run_(2, "2026-10-15T02:00:00Z")], {**good, "pr": 6}, "not for this pull request"),
            ([self.run_(2, "2026-10-15T02:00:00Z", head="d" * 40)], good, "not for this pull request"),
            ([self.run_(2, "2026-10-15T02:00:00Z")], {**good, "verified": None}, "recorded no commits"),
            # A commit is passed to git, so it has to be one.
            ([self.run_(2, "2026-10-15T02:00:00Z")], {**good, "verified": {**self.VERIFIED, "head": "--upload-pack=x"}},
             "recorded no commits"),
        ):
            gh, _ = self.gh(runs, record)
            with self.assertRaises(ci.NotReused) as caught:
                ci.last_green_run("o/r", 5, None if why == "head repository is gone" else "o/r", "topic", 9, gh=gh)
            self.assertIn(why, str(caught.exception))

    def test_only_a_path_no_lane_but_policy_reads_is_left_out_of_the_comparison(self):
        for name in ("docs/ARCHITECTURE.md", "CONTRIBUTING.md", "design/hide-ui.lib.pen",
                     "scripts/tests/test_ci_plan.py", "AGENTS.md", "CLAUDE.md"):
            self.assertTrue(ci.reads_no_lane("M", name, CRATES, ROOT), name)
        for name in ("docs/UI_BEHAVIOR.md", "hided/README.md", "web/src/App.tsx", "Cargo.lock",
                     "contracts/conpty.json", "web/e2e/cmdk.spec.ts", "unknown/file", "site/public/index.html"):
            self.assertFalse(ci.reads_no_lane("M", name, CRATES, ROOT), name)
        self.assertFalse(ci.reads_no_lane("T", "docs/ARCHITECTURE.md", CRATES, ROOT))

    def test_the_plan_job_reads_earlier_runs_and_keeps_its_record(self):
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        plan_job = workflow[workflow.index("\n  plan:\n"):workflow.index("\n  policy:\n")]
        self.assertIn("      actions: read\n", plan_job)
        self.assertIn("GH_TOKEN: ${{ github.token }}", plan_job)
        record = re.search(r"--record (\S+)", plan_job).group(1)
        self.assertEqual(Path(record).name, ci.RECORD_FILE)
        upload = plan_job[plan_job.index("actions/upload-artifact"):]
        self.assertIn(f"name: {ci.RECORD_ARTIFACT}\n", upload)
        self.assertIn(f"path: {record}\n", upload)
        # A rerun replaces its own record; a record that fails to upload
        # leaves the next run to plan its lanes, not this one red.
        self.assertIn("overwrite: true", upload)
        self.assertIn("continue-on-error: true", plan_job)


class ReverseDependencies(unittest.TestCase):
    def test_consumers_come_from_the_manifests(self):
        crates = {
            "leaf": {"name": "leaf", "consumers": {"middle"}},
            "middle": {"name": "middle", "consumers": {"top"}},
            "top": {"name": "top", "consumers": set()},
            "other": {"name": "other", "consumers": set()},
        }
        self.assertEqual(ci.reverse_closure(crates, "leaf"), {"leaf", "middle", "top"})
        self.assertEqual(ci.reverse_closure(crates, "top"), {"top"})

    def test_the_workspace_graph_has_every_crate(self):
        self.assertIn("hide-platform", CRATES)
        self.assertEqual(CRATES["hide-platform"]["name"], "hide-platform")
        self.assertIn("hided", CRATES["hide-core"]["consumers"])


if __name__ == "__main__":
    unittest.main()
