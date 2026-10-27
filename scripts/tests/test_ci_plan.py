"""Which lanes `scripts/ci-plan.py` plans for real changes, and what `verify` accepts."""
import importlib.util
from fnmatch import fnmatchcase
import json
import re
from pathlib import Path, PurePosixPath
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("ci_plan", Path(__file__).parents[1] / "ci-plan.py")
ci = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci)
ROOT = Path(__file__).parents[2]
CRATES = ci.cargo_crates(ROOT)
EVERY_PACKAGE = sorted(crate["name"] for crate in CRATES.values())
# The lanes that take a macOS runner; every other check on macOS is a job of
# `nightly.yml`.
MACOS_LANES = {"os-contract-macos", "desktop-e2e", "package", "omp-contract"}
# Every lane but `package`: what a push, the nightly and an unclassified path plan.
FULL = [lane for lane in ci.LANES if lane not in ("package", "agent-clis")]
# A full plan with a package input: `agent-clis` is planned by its own paths only.
PACKAGED = [lane for lane in ci.LANES if lane != "agent-clis"]
# The paths a pull request changed when `package.yml` had its own trigger.
PACKAGE_INPUTS = (
    "desktop/scripts/package.mjs", "desktop/scripts/build.mjs", "desktop/package.json", "desktop/resources/entitlements.plist",
    "contracts/herdr-bundle.json", "scripts/fetch-herdr-runtime.sh", "scripts/fetch-herdr-runtime.ps1",
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
                    self.assertEqual(result["lanes"], ["policy", "checks", "windows-check"])
                    self.assertEqual(result["rust_packages"], [])
                    self.assertFalse(ci.reads_no_lane(status, path, CRATES, ROOT))

    def test_web_unit_tests_keep_their_rust_and_css_build_readers(self):
        # sidebar.rs scans all web/src TS, including comments in test files;
        # Tailwind's web-root scan also reads source and e2e unit tests.
        for path in ("web/src/new.test.ts", "web/src/components/new.test.tsx"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(set(result["lanes"]), {"policy", "rust", "checks", "web-e2e", "desktop-e2e"})
                self.assertEqual(result["rust_packages"], ["herdr-core"])
        self.assertEqual(set(plan("web/e2e/platform-program.unit.ts")["lanes"]),
                         {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"})

    def test_unit_tests_do_not_remove_a_products_lanes_or_a_conservative_fallback(self):
        for product in ("web/src/terminals.ts", "desktop/src/main/index.ts", "desktop/src/main/updater.ts", "hide-platform/src/lib.rs",
                        "Cargo.lock", "scripts/ci-plan.py", "unclaimed/config.json"):
            with self.subTest(product=product):
                expected = set(plan(product)["lanes"]) | {"checks"}
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
        notes = plan("AGENTS.md")
        self.assertEqual(notes["lanes"], ["policy", "rust"])
        self.assertEqual(notes["rust_packages"], ["herdr-core"])
        self.assertIn("herdr-core", plan("hided/src/core.rs")["rust_packages"])
        self.assertTrue({"web-e2e", "windows-e2e"} <= set(plan("desktop/src/main/wirePath.ts")["lanes"]))
        self.assertTrue({"checks", "web-e2e"} <= set(plan("design/tokens.json")["lanes"]))
        # A guide a test reads is more than documentation.
        self.assertEqual(plan("docs/UI_BEHAVIOR.md")["lanes"], ["policy", "checks"])
        guide = plan("docs/AI_PROVIDERS.md")
        self.assertEqual(guide["lanes"], ["policy", "rust"])
        self.assertEqual(guide["rust_packages"], ["hide-agent-adapter"])
        self.assertEqual(plan("web/src/assets/agents/manifest.json")["rust_packages"], ["herdr-core", "hide-agent-adapter"])

    def test_a_web_display_change_runs_both_suites_of_the_shell_and_skips_windows(self):
        result = plan("web/src/components/ui/button.tsx", "web/src/Overview.tsx")
        # herdr-core's structure tests scan the shell's sources, and the
        # desktop suite drives the same shell inside the app.
        self.assertEqual(set(result["lanes"]), {"policy", "checks", "web-e2e", "desktop-e2e", "rust"})
        self.assertEqual(result["rust_packages"], ["herdr-core"])
        # The phone page is served by hided, and the app never loads it.
        for path in ("web/mobile.html", "web/public/m/sw.js"):
            with self.subTest(path=path):
                self.assertEqual(set(plan(path)["lanes"]), {"policy", "checks", "web-e2e"})

    def test_the_changes_that_broke_a_desktop_spec_on_main_plan_the_desktop_suite(self):
        # Issue #1288: each pull request's run left `desktop-e2e` out, and
        # main's full run failed the desktop spec that asserted what it changed.
        for number, paths in (
            # desktop/e2e/session-work-ledger.spec.ts read the session record.
            (1106, ("web/src/SessionRecord.tsx", "web/src/sessionWork.ts", "hide-session/src/conversation_cursor.rs")),
            # desktop/e2e/right-lane.spec.ts read the file tree a lane close keeps.
            (1147, ("herdr-core/src/runtime/workspace_view.rs", "herdr-core/src/runtime/tests/workspace_view.rs", "web/e2e/s3.spec.ts")),
        ):
            with self.subTest(pull_request=number):
                result = plan(*paths, "docs/UI_BEHAVIOR.md")
                self.assertFalse(result["full"])
                self.assertIn("desktop-e2e", result["lanes"])

    def test_web_code_the_desktop_host_reads_reaches_desktop_and_windows(self):
        for path in ("web/src/shortcuts.ts", "web/src/host.ts", "web/src/store.ts", "web/src/keys.ts"):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertTrue({"checks", "web-e2e", "desktop-e2e", "windows-e2e"} <= lanes)
                self.assertNotIn("windows-check", lanes)
                self.assertNotIn("remote-mailbox", lanes)

    def test_a_platform_spec_keeps_its_platform_lanes_on_linux_and_windows(self):
        lanes = set(plan("web/e2e/s3.spec.ts")["lanes"])
        self.assertEqual(lanes, {"policy", "checks", "web-e2e", "windows-e2e"})
        lanes = set(plan("web/e2e/new-tab.spec.ts")["lanes"])
        self.assertEqual(lanes, {"policy", "checks", "web-e2e"})

    def test_the_remote_mailbox_lane_follows_the_crates_it_builds_and_tests(self):
        # It builds hided, hide, the agent hooks and the host helper and runs
        # hided's remote_delivery test, so a change to one of those crates
        # or to a crate they depend on plans it.
        for path in (
            "herdr-core/src/lib.rs", "hided/src/lib.rs", "hide-agent-hooks/src/lib.rs", "hide-host/src/lib.rs",
            "hide-platform/src/process.rs", "hide-kit/src/lib.rs", "hide-session/src/lib.rs", "hided/tests/it/remote_delivery.rs",
        ):
            with self.subTest(path=path):
                self.assertIn("remote-mailbox", plan(path)["lanes"])
        # The web shell, its specs and the desktop host do not reach it.
        for path in (
            "web/e2e/s3.spec.ts", "web/e2e/new-tab.spec.ts", "web/src/host.ts", "web/src/Overview.tsx", "web/playwright.config.ts",
            "web/e2e/herdr-fixture.ts", "desktop/src/main/wirePath.ts", "desktop/src/preload/index.ts", "docs/TESTING.md",
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
            "herdr-core/src/lib.rs", "hided/tests/it/remote_delivery.rs", "hided/src/main.rs",
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

    def test_desktop_changes_run_the_desktop_lanes(self):
        result = plan("desktop/src/preload/index.ts")
        self.assertEqual(set(result["lanes"]), {"policy", "checks", "desktop-e2e", "windows-desktop"})
        self.assertIn("windows-check", plan("desktop/src/main/index.ts")["lanes"])

    def test_a_desktop_spec_reaches_windows_only_when_it_checks_what_differs_by_system(self):
        self.assertEqual(set(plan("desktop/e2e/desktop.spec.ts")["lanes"]), {"policy", "checks", "desktop-e2e", "windows-desktop"})
        self.assertEqual(set(plan("desktop/e2e/keys.spec.ts")["lanes"]), {"policy", "checks", "desktop-e2e"})
        self.assertIn("windows-desktop", plan("desktop/e2e/gone.spec.ts", status="D")["lanes"])

    def test_the_windows_desktop_lane_follows_what_differs_by_system(self):
        for path in ("hided/src/lib.rs", "hide-platform/src/process.rs", "hide-kit/src/lib.rs", "web/src/host.ts", "web/e2e/herdr-fixture.ts"):
            with self.subTest(path=path):
                self.assertIn("windows-desktop", plan(path)["lanes"])
        for path in ("web/src/Overview.tsx", "hide-session/src/lib.rs", "web/e2e/new-tab.spec.ts", "docs/TESTING.md"):
            with self.subTest(path=path):
                self.assertNotIn("windows-desktop", plan(path)["lanes"])

    def test_a_platform_crate_reaches_every_os_and_its_consumers(self):
        result = plan("hide-platform/src/process.rs")
        self.assertTrue(
            {"rust", "os-contract", "os-contract-macos", "windows-check", "windows-e2e", "web-e2e", "desktop-e2e", "remote-mailbox"}
            <= set(result["lanes"])
        )
        # The data-only adapter and the text-only shell reader are independent, and
        # workspace-hack depends on no member; every other crate consumes the platform layer.
        self.assertEqual(result["rust_packages"],
                         [name for name in EVERY_PACKAGE
                          if name not in ("hide-agent-adapter", "hide-shell", "workspace-hack")])
        self.assertFalse(result["full"])

    def test_an_adapter_change_tests_its_real_consumers_and_compiles_on_windows(self):
        # These consumers follow the workspace manifests, independently of the planner.
        consumers = [
            "herdr-core", "hide-agent-adapter", "hide-agent-hooks", "hide-host", "hide-kit",
            "hide-node", "hide-node-link", "hide-session", "hided",
        ]
        for path in ("hide-agent-adapter/src/lib.rs", "hide-agent-adapter/Cargo.toml"):
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(result["rust_packages"], consumers)
                self.assertTrue({"rust", "windows-check", "web-e2e", "desktop-e2e", "remote-mailbox"} <= set(result["lanes"]))
                self.assertFalse(set(result["lanes"]) & {"os-contract", "os-contract-macos", "windows-e2e"})
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

    def test_the_macos_os_contract_runs_for_the_two_crates_it_tests(self):
        for path in ("hide-herdr-client/src/lib.rs", "hide-platform/src/process.rs"):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertTrue({"os-contract", "os-contract-macos"} <= lanes)

    def test_a_core_daemon_or_shell_change_asks_for_the_desktop_suite_and_no_other_macos_runner(self):
        # The app runs them, so the desktop suite does too; their OS
        # differences are checked on Linux and Windows, and the nightly runs
        # the macOS leg and the macOS `@platform` tests for them.
        for path in (
            "herdr-core/src/lib.rs", "hided/src/lib.rs", "hide-host/src/lib.rs", "hide-session/src/lib.rs",
            "web/src/Overview.tsx",
        ):
            with self.subTest(path=path):
                self.assertEqual(set(plan(path)["lanes"]) & MACOS_LANES, {"desktop-e2e"})
        # What only the web suite runs asks for no macOS runner.
        for path in ("web/e2e/s3.spec.ts", "web/playwright.config.ts", "web/mobile.html"):
            with self.subTest(path=path):
                lanes = set(plan(path)["lanes"])
                self.assertFalse(lanes & MACOS_LANES, lanes & MACOS_LANES)
        self.assertEqual(set(plan("herdr-core/src/lib.rs")["lanes"]) & {"os-contract", "windows-e2e"}, {"os-contract", "windows-e2e"})

    def test_what_goes_into_a_package_plans_the_package_lane_and_nothing_else_does(self):
        # The kit and hooks a packaged daemon runs reach the packaged app only through it.
        for path in PACKAGE_INPUTS:
            with self.subTest(path=path):
                self.assertIn("package", plan(path)["lanes"])
        for path in (
            "hided/src/lib.rs", "hided/src/core.rs", "herdr-core/src/lib.rs", "hide-platform/src/process.rs",
            "desktop/src/main/index.ts", "desktop/e2e/fixture.ts", "web/src/store.ts", "docs/TESTING.md",
            ".github/workflows/pr.yml", "Cargo.lock", "pnpm-lock.yaml",
        ):
            with self.subTest(path=path):
                self.assertNotIn("package", plan(path)["lanes"])
        self.assertEqual(set(plan("hide-kit/src/lib.rs")["lanes"]) & MACOS_LANES, {"desktop-e2e", "package"})
        self.assertEqual(plan(".github/workflows/package.yml")["lanes"], ["policy", "package"])
        self.assertEqual(plan(".github/workflows/device-helpers.yml")["lanes"], ["policy", "package"])

    def test_the_desktop_lane_follows_what_the_app_runs(self):
        for path in (
            "desktop/src/main/index.ts", "desktop/static/icon.png", "desktop/e2e/desktop.spec.ts", "web/src/shortcuts.ts",
            "web/src/Overview.tsx", "web/index.html", "web/e2e/herdr-fixture.ts",
            "herdr-core/src/lib.rs", "hided/src/lib.rs", "hide-platform/src/process.rs",
        ):
            with self.subTest(path=path):
                self.assertIn("desktop-e2e", plan(path)["lanes"])
        for path in (
            "web/e2e/new-tab.spec.ts", "web/playwright.config.ts", "web/mobile.html", "web/public/m/manifest.webmanifest",
            "docs/TESTING.md", "design/tokens.json",
        ):
            with self.subTest(path=path):
                self.assertNotIn("desktop-e2e", plan(path)["lanes"])

    def test_shared_unknown_and_unsafe_changes_run_everything(self):
        for entries in (
            [("M", ".github/workflows/pr.yml")],
            [("M", "scripts/verify-web.sh")],
            [("M", "contracts/herdr-bundle.json")],
            [("M", "Cargo.lock")],
            [("M", "pnpm-lock.yaml")],
            [("M", "scripts/check-runtime-gates.sh")],
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
                self.assertEqual(result["lanes"], PACKAGED if package else FULL)
                self.assertEqual(result["rust_packages"], EVERY_PACKAGE)

    def test_a_push_and_a_failed_comparison_run_everything(self):
        self.assertEqual(ci.plan("push", None, None, ROOT)["lanes"], FULL)
        result = ci.plan("pull_request", "0" * 40, "HEAD", ROOT)
        self.assertTrue(result["full"])
        self.assertIn("comparison unavailable", result["reasons"]["rust"][0])

    def test_nightly_calls_verify_with_every_lane(self):
        # The call plans every lane like a push, and a lane it fails reaches
        # the nightly report, which waits for it in `needs`. The packages are
        # the nightly's own call, so `verify` leaves them out.
        self.assertEqual(ci.plan("schedule", None, None, ROOT)["lanes"], FULL)
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        self.assertRegex(workflow, r"\n  workflow_call:\n")
        # A called run takes the caller's name, so it never takes a push run's
        # pending place in the `verify-refs/heads/main` group.
        self.assertIn("group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}", workflow)
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        call = nightly[nightly.index("\n  verify:\n"):nightly.index("\n  package:\n")]
        self.assertIn("uses: ./.github/workflows/pr.yml", call)
        self.assertIn("issues: write", call)
        report = nightly[nightly.index("\n  report:\n"):]
        self.assertRegex(report, r"needs: \[[^\]]*\bverify\b[^\]]*\]")

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


class NamedPaths(unittest.TestCase):
    """The paths `ci-plan.py` names; every other path still plans every lane."""

    def lanes(self, *paths):
        return set(plan(*paths)["lanes"])

    def test_paths_no_lane_reads_plan_policy_alone(self):
        for path in (
            "site/index.html", "tools/t1-preflight/README.md", "spikes/web-shell/measure/summarize.py", "agents/prd/x/prd.md",
            ".gitignore", "web/.gitignore", "LICENSE", "NOTICE", ".github/pull_request_template.md", ".github/dependabot.yml",
            ".github/workflows/nightly.yml", ".github/workflows/herdr-update.yml", ".github/workflows/perf.yml",
            ".github/ISSUE_TEMPLATE/bug.yml", ".github/DISCUSSION_TEMPLATE/ideas.yml", ".github/APPROVED_CONTRIBUTORS",
            ".github/workflows/contribution-gate.yml",
            "scripts/tests/test_ci_plan.py", "scripts/check-harness-ignore-anchor.sh",
            "scripts/pen-system.mjs", "scripts/design-review.mjs", "scripts/web-shell-measure/run.sh",
            # The design contract's generators and checks, which `design-contract.yml` runs.
            "scripts/gen-tokens.mjs", "scripts/gen-screens.mjs", "scripts/gen-pen.mjs", "scripts/pen-token-map.json",
            "scripts/check-design-contract.mjs", "scripts/check-web-tokens.mjs", "scripts/check-agent-logos.mjs",
            "scripts/premerge-check.py", "scripts/merge-order.py", "scripts/pull_requests.py",
            "contracts/README.md", "site/README.md",
        ):
            with self.subTest(path=path):
                result = plan(path)
                self.assertEqual(result["lanes"], ["policy"])
                self.assertFalse(result["full"])

    def test_web_e2e_helpers_reach_the_desktop_and_windows_lanes_but_not_rust(self):
        want = {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"}
        for path in ("web/e2e/herdr-fixture.ts", "web/e2e/shims/build.ts", "web/e2e/shims/noop.c", "web/e2e/test-size-baseline.json"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), want)

    def test_a_pin_or_its_installer_plans_the_lanes_that_install_pinned_clis(self):
        for path in ("contracts/agent-clis.json", "scripts/install-agent-clis.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agy-contract", "agent-clis"})
        self.assertEqual(self.lanes(".github/workflows/agent-clis.yml"), {"policy", "agent-clis"})
        self.assertEqual(self.lanes("scripts/agent_journey/clis.py"),
                         {"policy", "agent-journey", "agent-clis", "agy-contract"})

    def test_the_journey_and_its_stand_ins_plan_the_journey_lane(self):
        for path in ("scripts/agent_journey/cells.py", "scripts/agent-journey-check.py", "scripts/agent-journey-lane.py",
                     "contracts/agent-journey.json", ".github/workflows/agent-journey.yml"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "agent-journey"})
        for path in ("web/e2e/shims/claude-shim.c", "web/e2e/shims/live-check.h", "web/e2e/shims/spawn-provider.c"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "checks", "web-e2e", "windows-e2e", "desktop-e2e",
                                                    "windows-desktop", "windows-check", "agent-journey"})
        for path in ("hide-session/examples/journey_conversation.rs", "hide-memory/examples/journey_memory.rs",
                     "hide-agent-adapter/examples/journey_composer.rs", "hide-agent-hooks/tests/fixtures/fake-codex.py"):
            with self.subTest(path=path):
                self.assertIn("agent-journey", self.lanes(path))
                self.assertIn("rust", self.lanes(path))
        # The live check the journey runs and the web e2e drives, and its
        # Windows forms' test, which only the journey's Windows job runs.
        for path in ("scripts/agent_live_check/processes.py", "scripts/agent_live_check/windows.py",
                     "scripts/agent-live-check.py"):
            with self.subTest(path=path):
                self.assertEqual(self.lanes(path), {"policy", "web-e2e", "agent-journey"})
        self.assertEqual(self.lanes("scripts/tests/test_agent_live_windows.py"), {"policy", "agent-journey"})
        self.assertEqual(self.lanes("scripts/tests/test_agent_live_protection.py"), {"policy"})
        self.assertNotIn("agent-journey", self.lanes("hide-session/src/lib.rs"))

    def test_the_shared_prompt_identity_examples_run_the_hooks_and_policy_readers(self):
        result = plan("hide-agent-hooks/tests/fixtures/bell-prompt-identity.json")
        self.assertEqual(set(result["lanes"]), self.lanes("hide-agent-hooks/src/delivery.rs"))
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
                self.assertEqual(self.lanes(path), {"policy", "checks", "windows-check"})
        # Both retained jobs run both Vitest suites; a configured matrix alone
        # would not establish that these tests are executed on Windows.
        for lane in ("checks", "windows-check"):
            for folder in ("web", "desktop"):
                self.assertIn(f"bash scripts/verify-web.sh {folder} test", pr_jobs()[lane])

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
            "contracts/hided-ws.schema.json": ({"policy", "rust"}, ["herdr-core"]),
            "contracts/snapshot-wire-enums.json": ({"policy", "rust", "checks"}, ["herdr-core"]),
            "contracts/agent-adapters.json": ({"policy", "rust", "checks", "web-e2e", "desktop-e2e"}, ["hide-agent-adapter"]),
            "contracts/hide-cli.json": ({"policy", "rust"}, ["hided"]),
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
            "scripts/fetch-herdr-runtime.sh", "scripts/install-nextest.sh", "scripts/toolchain-env.sh",
            # Every member but two depends on `workspace-hack`, which holds
            # nearly every third-party crate: a lock change reaches them all.
            "pnpm-lock.yaml", "web/package.json", "desktop/package.json", "Cargo.lock", ".config/nextest.toml", "clippy.toml",
            "contracts/herdr-bundle.json", "contracts/herdr-api.schema.json",
            "mystery/file.txt", "scripts/some-new-script.sh", "web/some-new-config.json",
        ):
            with self.subTest(path=path):
                self.assertTrue(plan(path)["full"])
                self.assertEqual(plan(path)["lanes"], PACKAGED if path in PACKAGE_INPUTS else FULL)

    def test_a_crate_readme_is_still_the_crates_change(self):
        # The crate may include it in a doc test; the documentation rule only
        # covers folders no rule claims.
        self.assertTrue({"rust", "windows-check"} <= self.lanes("herdr-core/README.md"))

    def test_a_named_path_does_not_narrow_a_plan_that_has_another_reason(self):
        self.assertEqual(self.lanes("site/index.html", "web/src/Overview.tsx"), self.lanes("web/src/Overview.tsx"))
        self.assertEqual(plan("site/index.html", "Cargo.lock")["lanes"], FULL)

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
        for helper in ("verify-cargo.sh", "verify-web.sh", "toolchain-env.sh", "install-nextest.sh", "fetch-herdr-runtime.sh",
                       "fetch-herdr-runtime.ps1", "ci-flaky-report.py", "check-herdr-schema.py"):
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
    """Each Rust, web and desktop source's code with comment lines left out, by path, with its crate."""
    crate_of = {directory: crate["name"] for directory, crate in CRATES.items()}
    sources = {}
    for folder in [*crate_of, "web/src", "web/e2e", "desktop/src", "desktop/e2e", "desktop/scripts"]:
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


def runner_jobs(block):
    """How many runner jobs one `pr.yml` job starts: a reusable workflow starts its own."""
    if "uses: ./.github/workflows/web-e2e.yml" in block:
        shards = int(re.search(r"\n      shards: (\d+)", block).group(1))
        # With more than one shard parallel daemon and web builds precede them.
        return shards + (2 if shards > 1 else 0)
    if "uses: ./.github/workflows/desktop-e2e.yml" in block:
        # Each shard builds in place.
        return int(re.search(r"\n      shards: (\d+)", block).group(1))
    if any(f"uses: ./.github/workflows/{name}.yml" in block for name in ("os-contract", "agy-contract", "omp-contract", "agent-clis")):
        return len(json.loads(re.search(r"systems: '(\[[^']*\])'", block).group(1)))
    if "uses: ./.github/workflows/agent-journey.yml" in block:
        # The stand-ins run on each system but Windows, and Windows has its forms job.
        systems = json.loads(re.search(r"systems: '(\[[^']*\])'", block).group(1))
        return len([system for system in systems if system != "windows"]) + ("windows" in systems)
    if "uses: ./.github/workflows/package.yml" in block:
        package = (ROOT / ".github/workflows/package.yml").read_text()
        return len(re.findall(r"\n          - system: ", package)) + ("include-macos: true" in block)
    return 1


class Workflows(unittest.TestCase):
    """`pr.yml` has the jobs the plan names, and the job count TESTING.md states."""

    def test_a_workflow_a_lane_calls_plans_the_lanes_that_call_it(self):
        callers = {}
        for lane, block in pr_jobs().items():
            for called in re.findall(r"uses: \./(\.github/workflows/[a-z0-9-]+\.yml)", block):
                callers.setdefault(called, set()).add(lane)
        self.assertEqual(len(callers), 9)
        for workflow, lanes in callers.items():
            with self.subTest(workflow=workflow):
                result = plan(workflow)
                self.assertFalse(result["full"])
                # `package.yml` reaches `package` through PACKAGE_PATHS, like every package input.
                self.assertEqual(set(result["lanes"]), lanes | {"policy"})
                self.assertEqual(result["rust_packages"], [])

    def test_the_package_lane_is_the_only_way_a_pull_request_reaches_package_yml(self):
        # One list of package paths: `package.yml` has no trigger to keep in step with PACKAGE_PATHS.
        package = (ROOT / ".github/workflows/package.yml").read_text()
        triggers = package[package.index("\non:\n"):package.index("\njobs:\n")]
        self.assertNotIn("pull_request", triggers)
        self.assertNotIn("paths:", triggers)
        # The pull request's call builds the macOS package and runs its specs.
        self.assertIn("include-macos: true", pr_jobs()["package"])

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
                names_macos = bool(re.search(r"macos-\d+|\"macos\"|include-macos: true", body))
                self.assertEqual(names_macos, lane in MACOS_LANES)

    def test_the_most_a_pull_request_starts_is_the_count_testing_md_states(self):
        jobs = pr_jobs()
        most = 2 + sum(runner_jobs(jobs[lane]) for lane in ci.LANES)
        # The 2026-10-05 run started 23 jobs. Since then the macOS desktop
        # suite's second shard and the fifth and sixth web shards were added
        # because the suites outgrew their shards' step caps, which a smaller
        # queue does not fix (docs/TESTING.md, "Balancing the e2e shards"), and
        # the agent journey's Linux stand-in job and Windows forms job were
        # added, then the agent CLIs' Linux and Windows installs, which only a
        # pin change plans; a job beyond these is a new decision, made with
        # this number. The independent shared web build adds one job so it
        # does not serialize the daemon and shell builds, offset by the measured
        # reduction from six web shards to five.
        self.assertLessEqual(most, 32, "a run starts more jobs than the suites and native SDK contracts need")
        testing = (ROOT / "docs/TESTING.md").read_text()
        self.assertIn(f"starts at most {most} jobs", testing)


class Aggregate(unittest.TestCase):
    def test_planned_success_and_unplanned_skip_pass(self):
        for result in (plan("docs/BUILD.md"), plan("web/src/Overview.tsx"), plan(".github/workflows/pr.yml")):
            ci.aggregate(needs_for(result))

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

    def plan(self, last_green):
        return ci.plan("pull_request", "HEAD^1", "HEAD", self.repo, CRATES, last_green=last_green)


LINES = "".join(f"line {n}\n" for n in range(1, 21))


class ReuseGreenRun(unittest.TestCase):
    """Reuse unchanged app checks, rerunning policy and any changed registered unit tests."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.pr = PullRequest(directory.name)
        base = self.pr.commit("main", "base", {"docs/a.md": LINES, "web/src/x.ts": LINES, "herdr-core/src/lib.rs": "one\n"})
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
        self.pr.commit("main", "main moves", {"herdr-core/src/lib.rs": "two\n", "docs/a.md": LINES.replace("line 2\n", "line 2!\n")})
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
        for path, lanes in (("desktop/src/main/x.test.ts", ["policy", "checks", "windows-check"]),
                            ("desktop/e2e/windows-processes.unit.ts", ["policy", "checks", "windows-check"])):
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
                self.assertIn("desktop-e2e", self.pr.plan(None)["lanes"])

    def test_modified_and_deleted_unit_tests_are_rechecked_against_the_original_green_anchor(self):
        self.green["head"] = self.pr.commit("topic", "unit fixture", {"desktop/src/main/x.test.ts": "first\n"})
        self.pr.commit("topic", "unit assertion", {"desktop/src/main/x.test.ts": "second\n"})
        self.pr.check_out_merge()
        first = self.pr.plan(lambda: self.green)
        self.assertEqual(first["lanes"], ["policy", "checks", "windows-check"])
        # This partial green must not claim that its app lanes checked a new
        # head: later runs still compare with the original app-tested commits.
        anchor = ci.plan_record(first, 7, 202)["verified"]
        self.pr.git("checkout", "-q", "topic")
        self.pr.git("rm", "desktop/src/main/x.test.ts")
        self.pr.git("commit", "-qm", "delete unit test")
        self.pr.check_out_merge()
        deleted = self.pr.plan(lambda: anchor)
        self.assertEqual(deleted["lanes"], ["policy", "checks", "windows-check"])
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
        self.assertEqual(ci.last_green_run("o/r", 5, "o/r", "topic", 9, gh=gh), self.VERIFIED)
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
        for name in ("docs/ARCHITECTURE.md", "CONTRIBUTING.md", "design/hide-ui.lib.pen", "site/index.html",
                     "scripts/tests/test_ci_plan.py"):
            self.assertTrue(ci.reads_no_lane("M", name, CRATES, ROOT), name)
        for name in ("docs/UI_BEHAVIOR.md", "AGENTS.md", "hided/README.md", "web/src/App.tsx", "Cargo.lock",
                     "contracts/herdr-bundle.json", "web/e2e/cmdk.spec.ts", "unknown/file"):
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
        self.assertIn("hided", CRATES["herdr-core"]["consumers"])


if __name__ == "__main__":
    unittest.main()
