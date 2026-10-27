#!/usr/bin/env python3
"""Which `pr.yml` lanes a change needs, and whether `verify` may pass.

`plan` classifies the paths a pull request changes into lanes; every job in
`pr.yml` runs when its lane is in the plan, and `aggregate` is `verify`'s one
step: a planned lane must have succeeded and an unplanned one must have been
skipped, so a lane whose `if:` is wrong fails `verify` instead of passing it.

A push to main, a comparison that cannot be computed, a path no rule claims
and the nightly call all plan every lane but `package`, which only a change to
what goes into a package plans. No lane takes a macOS runner for anything but
what only macOS proves (`os-contract-macos`, `desktop-e2e`, and `package`'s
packaged app); the rest of the macOS checks are jobs of `nightly.yml`.

A pull request whose last green run still stands plans `policy` alone, or
also the unit checks when only registered desktop unit tests changed: what
it adds to main differs from what that run checked only in policy paths or
registered desktop unit tests, and main has changed none of the paths it adds that a lane
reads. Each pull request run keeps a plan record for the next one to read; a
missing run or record, or a lookup, fetch or comparison that fails, leaves
the plan as it is.
docs/TESTING.md, "Which lanes a pull request runs", owns the rules and the
reasons for them.
"""
import argparse
from fnmatch import fnmatchcase
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent

# The order is the order the summary prints.
LANES = (
    "policy",
    "rust",
    "checks",
    "os-contract",
    "os-contract-macos",
    "agy-contract",
    "omp-contract",
    "agent-journey",
    "agent-clis",
    "windows-check",
    "windows-e2e",
    "windows-desktop",
    "web-e2e",
    "remote-mailbox",
    "desktop-e2e",
    "package",
)
# What a full plan runs. `package` builds host packages and device kits from cold
# on their runners, so it runs for a change to what goes into one (PACKAGE_PATHS) and
# nothing else: main's push runs leave it out, the nightly calls
# `package.yml` itself and a release builds its own. `agent-clis` installs
# every pinned agent CLI on three runners, macOS's among them, for a change to
# the pin table or its installer and nothing else, since no other path
# reaches what it installs; the nightly's and a release's journey install
# every CLI too.
FULL_LANES = tuple(lane for lane in LANES if lane not in ("package", "agent-clis"))

# Crates whose change reaches what differs by operating system: the platform
# layer, the Herdr client's IPC, the install kit and hooks, and the daemon and
# core that `windows check` and the OS contract run on each system.
OS_CRATES = {
    "herdr-core",
    "hided",
    "hide-agent-hooks",
    "hide-herdr-client",
    "hide-host",
    "hide-kit",
    "hide-platform",
}
# The OS contract's macOS leg tests only these two, so only a change to one of
# them plans it; the others reach it through the nightly.
OS_CONTRACT_CRATES = {"hide-herdr-client", "hide-platform"}
# The crates whose agy tests the agy contract runs on Linux and Windows: the
# agent table, the hook and trust writer, the install kit, the conversation
# readers and the Hide AI backend (`agy-contract.yml`).
AGY_CRATES = {"hide-agent-adapter", "hide-agent-hooks", "hide-ai", "hide-kit", "hide-session"}
# `windows-desktop` is among them because the desktop app starts hided
# through `hide connect` there, the system's processes and local stream.
OS_LANES = ("os-contract", "windows-check", "windows-e2e", "windows-desktop")
# The remote mailbox lane builds `hided`, `hide`, the host helper and the hook
# binary, and runs `hided`'s `remote_delivery` test against them over
# private SSH, and its `held_letter_notice` test against the pinned Herdr; a
# change to any crate they link, or to the tests, can change its result, and
# nothing else can. Web code and web specs do not reach it.
MAILBOX_CRATES = {"herdr-core", "hided", "hide-agent-hooks", "hide-host"}

# The desktop suite drives the web shell and the daemon inside the app, so
# every web shell file plans `desktop-e2e` beside `web-e2e`: a desktop spec
# that asserts what the shell shows breaks on main's full run otherwise (#1106
# did, issue #1288). A crate change plans `web-e2e` alone: the macOS queue it
# would wait in cost a crate-only pull request half its run, and main's full
# run is the net for a desktop spec that asserts what the core shows (#1147,
# the accepted risk).
#
# Web files the desktop host imports or drives through native input: the host
# bridge, the shortcut registry the menu is built from, the store and
# snapshot, terminal input, and the focus and area code chords move. What they
# do differs by system, so they also plan the Windows suites.
SHARED_WEB = (
    "web/src/host.ts",
    "web/src/revealExternal.ts",
    "web/src/shortcuts.ts",
    "web/src/shortcutLabels.ts",
    "web/src/keyboard.ts",
    "web/src/keys.ts",
    "web/src/store.ts",
    "web/src/snapshot.ts",
    "web/src/ws.ts",
    "web/src/connection.ts",
    "web/src/terminals.ts",
    "web/src/terminalLinks.ts",
    "web/src/terminalLinkProvider.ts",
    "web/src/selection.ts",
    "web/src/areaCycle.ts",
    "web/src/areaFocus.ts",
    "web/src/viewFocus.ts",
    "web/src/browserViews.ts",
    "web/src/BrowserDisplay.tsx",
    "web/src/PaneView.tsx",
    "web/src/App.tsx",
    "web/src/main.tsx",
)
SHARED_WEB_LANES = ("windows-e2e", "windows-desktop")

# Files a lane outside their own directory reads: a change to one also plans
# that lane, and the Rust crates whose tests read it. An entry ending in `/`
# is a folder. Keep this beside the test that reads the file. A path no other
# rule claims plans its readers here and nothing else.
READERS = (
    # herdr-core/src/runtime/tests/snapshot_delta.rs asserts the agent notes.
    ("AGENTS.md", {"rust"}, {"herdr-core"}),
    ("CLAUDE.md", {"rust"}, {"herdr-core"}),
    # herdr-core's structure tests scan the shell (runtime/tests.rs, sidebar.rs)
    # and the daemon (runtime/tests/snapshot_delta.rs).
    ("web/src/", {"rust"}, {"herdr-core"}),
    ("hided/src/", {"rust"}, {"herdr-core"}),
    # web/e2e/s2, s3 and s7 import the wire path conversion.
    ("desktop/src/main/wirePath.ts", {"checks", "web-e2e", "windows-e2e"}, set()),
    # web/src/settings.test.ts, theme.test.ts and web/e2e/theme.spec.ts read the tokens.
    ("design/tokens.json", {"checks", "web-e2e"}, set()),
    # web/src/shortcutsDoc.test.ts checks the guide's chord table.
    ("docs/UI_BEHAVIOR.md", {"checks"}, set()),
    # hide-agent-adapter's contract test checks the usage coverage table in
    # the guide and the logo manifest the shell bundles.
    ("docs/AI_PROVIDERS.md", {"rust"}, {"hide-agent-adapter"}),
    ("web/src/assets/agents/manifest.json", {"rust"}, {"hide-agent-adapter"}),
    # The contracts a test holds the code to; the code does not read them.
    # herdr-core's memory_hook test reads the WebSocket schema. The web shell's types are generated from it
    # by hand (`gen:types`) and committed, so no web lane reads it.
    ("contracts/hided-ws.schema.json", {"rust"}, {"herdr-core"}),
    # herdr-core's `model::wire_enum_tests`, and web/src/userTurn.test.ts,
    # prMark.test.ts.
    ("contracts/snapshot-wire-enums.json", {"rust", "checks"}, {"herdr-core"}),
    # hide-agent-adapter's contract test; web/src/agentAdapters.ts imports it
    # into the shell, which plans what a web shell file plans.
    ("contracts/agent-adapters.json", {"rust", "checks", "web-e2e", "desktop-e2e"}, {"hide-agent-adapter"}),
    # hided's `cli_contract` test.
    ("contracts/hide-cli.json", {"rust"}, {"hided"}),
    # policy's test_agent_journey_cells shares the hook's prompt identity examples.
    ("hide-agent-hooks/tests/fixtures/bell-prompt-identity.json", {"policy"}, set()),
    # hide-agent-adapter's contract test.
    ("contracts/usage-coverage.json", {"rust"}, {"hide-agent-adapter"}),
)


def reader_entries(name):
    """The READERS entries that read the changed path `name`."""
    return [entry for entry in READERS
            if name == entry[0] or (entry[0].endswith("/") and name.startswith(entry[0]))]


# The paths that change what goes into a package: the package scripts and
# resources, the Herdr pin and its fetch, the build entrypoints, the web shell
# `hided/build.rs` embeds, the daemon replacement in `hided/src/cli.rs`, the
# install kit and hooks a packaged daemon runs, and the workflows that build
# them. This is the only list: `package.yml` has no trigger of its own.
PACKAGE_PATHS = (
    "desktop/scripts/*", "desktop/package.json", "desktop/resources/*", "desktop/native/*",
    "contracts/herdr-bundle.json", "scripts/fetch-herdr-runtime.*",
    "scripts/verify-cargo.sh", "scripts/verify-web.sh", "scripts/toolchain-env.sh",
    "hided/build.rs", "hided/src/cli.rs", "hide-kit/*", "hide-agent-hooks/*",
    ".github/workflows/package.yml", ".github/workflows/device-helpers.yml", ".github/workflows/release.yml",
    # The updater and the release files it reads, and the specs only this lane runs.
    "desktop/src/main/updater*", "scripts/check-release-assets.mjs", "desktop/e2e/packaged-*",
)


# Paths named here and in READERS are the only ones narrower than "every
# lane"; a path no rule names still plans every lane. Each entry says who reads the path, and
# `scripts/tests/test_ci_plan.py` checks the claim where it can be checked.
#
# Paths no `pr.yml` lane reads: `policy` alone. Its script suite and the
# repository invariants are what reads them.
POLICY_ONLY = (
    "agents/*", "site/*", "plugins/*", "tools/*", "spikes/*",
    ".gitignore", "web/.gitignore", "desktop/.gitignore",
    # hide's own license and copyright notice; no build or package reads them.
    "LICENSE", "NOTICE",
    ".github/pull_request_template.md", ".github/dependabot.yml",
    # The contribution intake: forms, the approved list and the gate that reads it.
    ".github/ISSUE_TEMPLATE/*", ".github/DISCUSSION_TEMPLATE/*", ".github/APPROVED_CONTRIBUTORS",
    ".github/workflows/contribution-gate.yml",
    # Workflows no `pr.yml` job calls; `design-contract.yml` has its own pull
    # request trigger, and the nightly calls `perf.yml` and
    # `agent-trust-contract.yml`.
    ".github/workflows/nightly.yml", ".github/workflows/herdr-update.yml", ".github/workflows/design-contract.yml",
    ".github/workflows/perf.yml", ".github/workflows/agent-trust-contract.yml",
    "scripts/tests/*",
    # Scripts only `policy`, another workflow or nobody runs.
    "scripts/check-agent-asset-committed.sh", "scripts/check-capability-readers-off-lock.sh",
    "scripts/check-harness-ignore-anchor.sh", "scripts/check-herdr-pin-single-source.sh",
    "scripts/check-core-touches-no-machine.py", "scripts/check-ai-prompt-language.py", "scripts/check-pr-screenshots.py",
    "scripts/check-no-workstation-identity.*", "scripts/check-worktree-removal-boundary.sh",
    "scripts/check-hide-full.sh", "scripts/check-hide-screens.mjs", "scripts/check-typed-live-remote.sh",
    "scripts/check-release-assets.mjs", "scripts/release-draft.mjs", "scripts/premerge-check.py",
    "scripts/merge-order.py", "scripts/pull_requests.py",
    "scripts/pen-*.mjs", "scripts/design-review.mjs", "scripts/web-shell-measure/*",
    # The design contract's generators, their token map and its checks:
    # `design-contract.yml` runs the checks on every pull request, and what
    # the generators write (`web/src/tokens.css`, the Pen files) is committed
    # and planned as its own path.
    "scripts/gen-tokens.mjs", "scripts/gen-pen.mjs", "scripts/gen-screens.mjs", "scripts/pen-token-map.json",
    "scripts/check-design-contract.mjs", "scripts/check-pen.mjs", "scripts/check-pen-gallery.mjs",
    "scripts/check-web-tokens.mjs", "scripts/check-agent-logos.mjs", "scripts/check-hide-design-enforcement.mjs",
)

# Paths whose readers are a known set of lanes. `web/e2e` helpers are imported
# by the desktop suites too, and `desktop/e2e` unit tests run in `windows
# check`, so neither is web-only or desktop-only.
WEB_E2E_LANES = {"checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"}
DESKTOP_E2E_LANES = {"checks", "desktop-e2e", "windows-desktop", "windows-check"}
# Desktop Vitest inputs, outside the app's explicit esbuild entry graphs:
# `checks` and `windows-check` run them. Web tests are deliberately absent:
# sidebar.rs scans every web/src TypeScript file, and Tailwind's web-root
# scanner reads web tests as CSS inputs. Their existing app lanes must stay.
# An unregistered convention keeps its folder's existing plan.
UNIT_TEST_PATHS = (
    "desktop/src/*.test.ts", "desktop/scripts/*.test.mjs",
    "desktop/e2e/fixture-cleanup.unit.ts", "desktop/e2e/windows-processes.unit.ts",
)


def is_unit_test(name):
    return any(fnmatchcase(name, pattern) for pattern in UNIT_TEST_PATHS)


# The agent journey's keyless lane runs its code on the compiled stand-ins
# (`agent-journey.yml`); the stand-ins' sources are also the e2e's.
JOURNEY_LANES = {"agent-journey"}
# The pinned agent CLIs and their installer: a pin moves in the pull request
# that installs it on every system (contracts/README.md, Agent CLI pins).
AGENT_CLI_LANES = {"agent-clis"}
# Files inside a crate the journey runs as its own tools: Hide's conversation
# reader, its Project Memory card writer, the composer judge and Codex's
# app-server stand-in.
JOURNEY_CRATE_FILES = ("hide-session/examples/journey_conversation.rs", "hide-memory/examples/journey_memory.rs",
                       "hide-agent-adapter/examples/journey_composer.rs",
                       "hide-agent-hooks/tests/fixtures/fake-codex.py")
# The live check's code, which the journey runs and `agent-live-check.spec.ts`
# drives in the web e2e; its unit tests are `policy`'s.
LIVE_CHECK_LANES = {"web-e2e"} | JOURNEY_LANES
# Script tests that also run on another system than `policy`'s Linux, in
# these lanes: `agent-journey.yml` runs the live check's Windows forms on Windows.
SYSTEM_TESTS = (
    ("scripts/tests/test_agent_live_windows.py", JOURNEY_LANES),
)
NAMED_LANES = (
    ("scripts/agent_live_check/*", LIVE_CHECK_LANES),
    ("scripts/agent-live-check.py", LIVE_CHECK_LANES),
    ("web/e2e/shims/claude-shim.c", WEB_E2E_LANES | JOURNEY_LANES),
    ("web/e2e/shims/live-check.h", WEB_E2E_LANES | JOURNEY_LANES),
    # The journey's controller builds the desktop e2e's spawn provider.
    ("web/e2e/shims/spawn-provider.c", WEB_E2E_LANES | JOURNEY_LANES),
    # The pin table's reader, which the installer imports too.
    ("scripts/agent_journey/clis.py", JOURNEY_LANES | AGENT_CLI_LANES | {"agy-contract"}),
    ("scripts/agent_journey/*", JOURNEY_LANES),
    ("scripts/agent-journey-check.py", JOURNEY_LANES),
    ("scripts/agent-journey-lane.py", JOURNEY_LANES),
    ("contracts/agent-journey.json", JOURNEY_LANES),
    ("web/e2e/*", WEB_E2E_LANES),
    ("desktop/e2e/*", DESKTOP_E2E_LANES),
    ("web/playwright.config.ts", {"checks", "web-e2e", "windows-e2e"}),
    ("desktop/playwright.config.ts", DESKTOP_E2E_LANES),
    ("desktop/vitest.config.ts", DESKTOP_E2E_LANES),
    # `web` and `desktop` lint share the e2e rules and the size check.
    ("web/eslint.config.js", {"checks"}),
    ("web/eslint.e2e.mjs", {"checks"}),
    ("web/eslint-rules/*", {"checks"}),
    ("web/scripts/check-e2e-test-size.mjs", {"checks"}),
    ("web/scripts/gen-types.mjs", {"checks", "web-e2e"}),
    # `vite build` in `checks` and in the web and desktop e2e builds (Linux,
    # Windows, macOS), and its `test` block is vitest's in `checks` and
    # `windows check`.
    ("web/vite.config.ts", {"checks", "web-e2e", "windows-e2e", "desktop-e2e", "windows-desktop", "windows-check"}),
    ("desktop/eslint.config.mjs", {"checks"}),
    ("desktop/eslint.globals.mjs", {"checks"}),
    # `build.mjs` is the e2e's build; `package.mjs` and `smoke-package.mjs` run
    # in the `package` lane, which PACKAGE_PATHS plans for this folder.
    ("desktop/scripts/build.mjs", {"checks", "desktop-e2e", "windows-desktop"}),
    ("desktop/scripts/package.mjs", {"checks"}),
    ("desktop/scripts/smoke-package.mjs", {"checks"}),
    # The workflows `pr.yml` lanes call: each plans the lanes that call it
    # (the nightly calls them too). The `package` lane's workflow and the
    # release that calls it plan `package` through PACKAGE_PATHS.
    (".github/workflows/web-e2e.yml", {"web-e2e", "windows-e2e"}),
    (".github/workflows/desktop-e2e.yml", {"desktop-e2e", "windows-desktop"}),
    (".github/workflows/os-contract.yml", {"os-contract", "os-contract-macos"}),
    (".github/workflows/agy-contract.yml", {"agy-contract"}),
    (".github/workflows/omp-contract.yml", {"omp-contract"}),
    (".github/workflows/agent-journey.yml", JOURNEY_LANES),
    (".github/workflows/agent-clis.yml", AGENT_CLI_LANES),
    # The agy contract installs the pinned agy with the same script.
    ("contracts/agent-clis.json", {"agy-contract"} | AGENT_CLI_LANES),
    ("scripts/install-agent-clis.py", {"agy-contract"} | AGENT_CLI_LANES),
    (".github/workflows/remote-mailbox.yml", {"remote-mailbox"}),
    (".github/workflows/package.yml", set()),
    (".github/workflows/device-helpers.yml", set()),
    (".github/workflows/release.yml", set()),
)


def named_lanes(path):
    """The lanes a named path needs, or None when no rule names it."""
    name = path.as_posix()
    for pattern, lanes in SYSTEM_TESTS:
        if fnmatchcase(name, pattern):
            return set(lanes), f"script test that also runs on another system: {name}"
    if any(fnmatchcase(name, pattern) for pattern in POLICY_ONLY):
        return set(), f"named path, no lane reads it: {name}"
    # Documentation below a folder no rule claims (a crate's own README is
    # claimed by its crate, which may include it in a doc test).
    if path.suffix == ".md":
        return set(), f"documentation: {name}"
    for pattern, lanes in NAMED_LANES:
        if fnmatchcase(name, pattern):
            return set(lanes), f"named path read by a known set of lanes: {name}"
    return None


def cargo_crates(root=ROOT):
    """Each workspace crate's directory, mapped to the crates that consume it."""
    raw = subprocess.check_output(
        ["bash", "scripts/verify-cargo.sh", "metadata", "--format-version", "1", "--no-deps"],
        cwd=root,
        stderr=subprocess.PIPE,
    )
    metadata = json.loads(raw)
    workspace = PurePosixPath(Path(metadata["workspace_root"]).as_posix())
    directory = {}
    for package in metadata["packages"]:
        manifest = PurePosixPath(Path(package["manifest_path"]).as_posix())
        directory[package["name"]] = manifest.parent.relative_to(workspace).as_posix()
    consumers = {name: set() for name in directory}
    for package in metadata["packages"]:
        for dependency in package["dependencies"]:
            # Normal, build and dev edges alike: a dev-dependency's tests run
            # against the changed crate too.
            if dependency.get("path") and dependency["name"] in directory:
                consumers[dependency["name"]].add(package["name"])
    return {directory[name]: {"name": name, "consumers": consumers[name]} for name in directory}


def reverse_closure(crates, start):
    """The crate named `start` and every crate that depends on it, transitively."""
    by_name = {crate["name"]: crate for crate in crates.values()}
    seen, pending = {start}, [start]
    while pending:
        for consumer in by_name[pending.pop()]["consumers"]:
            if consumer not in seen:
                seen.add(consumer)
                pending.append(consumer)
    return seen


def is_docs(path):
    parts = path.parts
    if parts[0] == "docs":
        return True
    if path.name in ("AGENTS.md", "CLAUDE.md"):
        return True
    return len(parts) == 1 and path.suffix == ".md"


def classify(path, status, crates, root):
    """The lanes one changed path needs, a reason, and the Rust packages it reaches.

    None for the lanes means the path needs every lane.
    """
    parts = path.parts
    if status not in ("A", "M", "D"):
        return None, f"{status} change: {path}", set()
    if is_docs(path):
        return set(), f"documentation: {path}", set()
    if parts[0] == "design":
        # design-contract.yml checks the library; no pr.yml lane reads it.
        return set(), f"design library: {path}", set()
    if is_unit_test(path.as_posix()):
        return {"checks", "windows-check"}, f"registered desktop unit test: {path}", set()
    crate = crates.get(parts[0])
    if crate is not None:
        packages = reverse_closure(crates, crate["name"])
        # Every crate compiles into the Windows workspace `windows check` builds.
        lanes = {"rust", "windows-check"}
        # The web suite drives hided, so a crate it links reaches it; the
        # desktop suite drives it too, and main's full run runs that one.
        if "hided" in packages:
            lanes.add("web-e2e")
        if packages & MAILBOX_CRATES:
            lanes.add("remote-mailbox")
        if crate["name"] in OS_CRATES:
            lanes.update(OS_LANES)
        if crate["name"] in OS_CONTRACT_CRATES:
            lanes.add("os-contract-macos")
        if crate["name"] in AGY_CRATES:
            lanes.add("agy-contract")
        if crate["name"] in {"hide-ai", "hide-platform"}:
            lanes.add("omp-contract")
        if path.as_posix() in JOURNEY_CRATE_FILES:
            lanes.update(JOURNEY_LANES)
        return lanes, f"crate {crate['name']}: {path}", packages
    if parts[0] == "web" and len(parts) > 1:
        if parts[1] == "e2e" and len(parts) == 3 and path.name.endswith(".spec.ts"):
            lanes = {"checks", "web-e2e"}
            spec = root / path
            # A deleted spec cannot be read; keep the platform lanes for it.
            if status == "D" or "@platform" in spec.read_text(encoding="utf-8"):
                lanes.add("windows-e2e")
            return lanes, f"web e2e spec: {path}", set()
        if path.as_posix() == "web/mobile.html" or parts[1:3] == ("public", "m"):
            # The phone page and its assets: hided serves them at `/m/`, and
            # the app never loads them.
            return {"checks", "web-e2e"}, f"phone page: {path}", set()
        if parts[1] in ("src", "public") or path.as_posix() == "web/index.html":
            lanes = {"checks", "web-e2e", "desktop-e2e"}
            if path.as_posix() in SHARED_WEB:
                lanes.update(SHARED_WEB_LANES)
                return lanes, f"web shared with the desktop host: {path}", set()
            return lanes, f"web shell: {path}", set()
    if parts[0] == "desktop" and len(parts) > 1:
        if parts[1] == "e2e" and len(parts) == 3 and path.name.endswith(".spec.ts"):
            lanes = {"checks", "desktop-e2e"}
            spec = root / path
            # A deleted spec cannot be read; keep the Windows lane for it.
            if status == "D" or "@platform" in spec.read_text(encoding="utf-8"):
                lanes.add("windows-desktop")
            return lanes, f"desktop e2e spec: {path}", set()
        if parts[1] in ("src", "static"):
            lanes = {"checks", "desktop-e2e", "windows-desktop"}
            if parts[1:3] == ("src", "main"):
                # The main process owns paths, child processes and the daemon
                # start; its unit suite runs on Windows in `windows check`.
                lanes.add("windows-check")
            return lanes, f"desktop host: {path}", set()
    named = named_lanes(path)
    if named is not None:
        return named[0], named[1], set()
    if reader_entries(path.as_posix()):
        # `select` adds the readers' lanes and crates.
        return set(), f"read by a known set of readers: {path}", set()
    # `pr.yml`, the scripts the lanes call, the Herdr pin and schema, lockfiles,
    # toolchain and package configuration, and any path no rule above, in
    # `named_lanes` or in READERS names: every lane. `Cargo.lock` is among
    # them: every member but two depends on `workspace-hack`, which holds
    # nearly every third-party crate, so a lock change reaches every crate.
    return None, f"shared or unclassified: {path}", set()


def select(entries, crates, root=ROOT, full_reason=None):
    """The plan for a list of (status, path) entries from `git diff --name-status`."""
    reasons = {lane: [] for lane in LANES}
    packages = set()
    full = []
    if full_reason:
        full.append(full_reason)
    elif not entries:
        full.append("the comparison listed no changed path")
    for status, name in entries:
        path = PurePosixPath(name)
        if not name or path.is_absolute() or ".." in path.parts:
            raise ValueError(f"invalid changed path: {name!r}")
        # A broad package input (updater*) can name its unit test too, but a
        # registered desktop unit test is not built into the package.
        if not is_unit_test(name) and any(fnmatchcase(name, pattern) for pattern in PACKAGE_PATHS):
            reasons["package"].append(f"package input: {name}")
        lanes, reason, reached = classify(path, status, crates, root)
        if lanes is None:
            full.append(reason)
            continue
        for _, extra_lanes, extra_packages in reader_entries(name):
            lanes |= extra_lanes
            reached = reached | extra_packages
        packages |= reached
        reasons["policy"].append(reason)
        for lane in lanes:
            reasons[lane].append(reason)
    if full:
        for lane in FULL_LANES:
            reasons[lane] = list(full)
        packages = {crate["name"] for crate in crates.values()}
    planned = [lane for lane in LANES if reasons[lane]]
    return {
        "full": bool(full),
        "lanes": planned,
        "rust_packages": sorted(packages) if reasons["rust"] else [],
        "reasons": {lane: reasons[lane] for lane in planned},
    }


def reads_no_lane(status, name, crates, root=ROOT):
    """True when no lane but `policy` reads the changed path."""
    if status not in ("A", "M", "D") or any(fnmatchcase(name, pattern) for pattern in PACKAGE_PATHS) or reader_entries(name):
        return False
    # The status decides only whether a spec is read for its tag, never
    # whether a path has a lane, so the answer comes from the name alone.
    lanes, _, _ = classify(PurePosixPath(name), "D", crates, root)
    return lanes == set()


def changed_entries(base, head, root=ROOT):
    raw = subprocess.check_output(
        ["git", "diff", "--name-status", "-z", "--no-renames", base, head], cwd=root, stderr=subprocess.PIPE
    )
    fields = raw.decode("utf-8").split("\0")
    if fields[-1] != "" or len(fields[:-1]) % 2:
        raise ValueError("malformed diff output")
    return list(zip(fields[:-1:2], fields[1:-1:2]))


# The workflow whose green runs a pull request may reuse, and the artifact
# each of its pull request runs keeps the plan record in.
WORKFLOW = "pr.yml"
RECORD_ARTIFACT = "ci-plan"
RECORD_FILE = "ci-plan.json"
RECORD_VERSION = 1
COMMIT = re.compile(r"[0-9a-f]{40}")


class NotReused(Exception):
    """Why the last green run does not stand for this one; the plan stays as it is."""


def error_detail(error):
    if isinstance(error, subprocess.CalledProcessError):
        stderr = error.stderr if isinstance(error.stderr, str) else (error.stderr or b"").decode(errors="replace")
        return stderr.strip() or str(error)
    return str(error)


def listing(names, shown=5):
    return ", ".join(names[:shown]) + (f" (+{len(names) - shown} more)" if len(names) > shown else "")


def call_gh(*args):
    try:
        # gh speaks UTF-8; without a named encoding a Windows runner reads it as cp1252.
        done = subprocess.run(["gh", *args], capture_output=True, encoding="utf-8")
    except OSError as error:
        raise NotReused(f"gh could not run: {error}") from error
    if done.returncode != 0:
        raise NotReused(f"`gh {args[0]} {args[1]}` failed: {done.stderr.strip()}")
    return done.stdout


def verified_commits(record, number, run):
    """What a green run's plan record says its lanes checked: a run, a main and a head."""
    if not isinstance(record, dict) or record.get("version") != RECORD_VERSION:
        raise NotReused(f"run {run['id']} kept a plan record this plan cannot read")
    if record.get("pr") != number or record.get("head") != run["head_sha"]:
        raise NotReused(f"run {run['id']}'s plan record is not for this pull request's head {run['head_sha']}")
    verified = record.get("verified")
    if (not isinstance(verified, dict) or type(verified.get("run")) is not int
            or not all(isinstance(verified.get(key), str) and COMMIT.fullmatch(verified[key]) for key in ("base", "head"))):
        raise NotReused(f"run {run['id']} recorded no commits its lanes checked")
    return {"run": verified["run"], "base": verified["base"], "head": verified["head"]}


def last_green_run(repo, number, head_repo, head_ref, run_id, gh=call_gh):
    """The commits the lanes of this pull request's latest green run checked.

    A run that reused an earlier one recorded that run's commits, so the
    answer always names a run whose lanes ran.
    """
    if not head_repo:
        raise NotReused("the pull request's head repository is gone")
    runs = json.loads(gh(
        "api", "-X", "GET", f"repos/{repo}/actions/workflows/{WORKFLOW}/runs",
        "-f", "event=pull_request", "-f", "status=success", "-f", f"branch={head_ref}", "-F", "per_page=100",
    ))["workflow_runs"]
    # Another fork's branch of the same name is not this pull request.
    runs = [run for run in runs if run["id"] != run_id and (run.get("head_repository") or {}).get("full_name") == head_repo]
    if not runs:
        raise NotReused("this pull request has no earlier green run")
    run = max(runs, key=lambda run: (run["created_at"], run["id"]))
    with tempfile.TemporaryDirectory() as directory:
        gh("run", "download", str(run["id"]), "--repo", repo, "--name", RECORD_ARTIFACT, "--dir", directory)
        record = json.loads((Path(directory) / RECORD_FILE).read_text(encoding="utf-8"))
    return verified_commits(record, number, run)


def fetch_history(root, *commits):
    """Make the commits and their history readable: a pull request's checkout is two commits deep."""
    shallow = subprocess.check_output(
        ["git", "rev-parse", "--is-shallow-repository"], cwd=root, text=True, stderr=subprocess.PIPE
    ).strip() == "true"
    missing = [
        commit for commit in commits
        if subprocess.run(["git", "cat-file", "-e", f"{commit}^{{commit}}"], cwd=root, capture_output=True).returncode
    ]
    if shallow or missing:
        # Commits and trees only, about 9 MB for main's whole history;
        # `merge-tree` fetches the few file contents it has to merge.
        subprocess.check_output(
            ["git", "fetch", "--quiet", "--no-tags", "--filter=blob:none", *(["--unshallow"] if shallow else []),
             "origin", *commits],
            cwd=root, stderr=subprocess.PIPE,
        )


def still_stands(verified, base, head, entries, crates, root=ROOT):
    """Why the run stands and the unit-test delta to recheck, or NotReused.

    It stands when the pull request adds to main what that run's head added,
    but for policy paths and registered desktop unit tests, and main has changed none of the
    paths the pull request changes that a lane reads since that run's main.
    """
    run, old_base, old_head = verified["run"], verified["base"], verified["head"]
    fetch_history(root, old_base, old_head)
    # The checked head merged into main as it is now. A conflict leaves its
    # markers in the tree, so a path resolved differently since differs here.
    merged = subprocess.run(
        ["git", "merge-tree", "--write-tree", "--no-messages", base, old_head], cwd=root, capture_output=True, text=True
    )
    if merged.returncode not in (0, 1) or not merged.stdout.split():
        raise NotReused(f"`git merge-tree` failed: {merged.stderr.strip()}")
    delta = [(status, name) for status, name in changed_entries(merged.stdout.split()[0], head, root)
             if not reads_no_lane(status, name, crates, root)]
    changed = [name for status, name in delta if status not in ("A", "M", "D") or not is_unit_test(name)]
    if changed:
        raise NotReused(f"since run {run} the pull request changed {listing(changed)}, which a lane reads")
    main_changed = {name for _, name in changed_entries(old_base, base, root)}
    overlap = [name for status, name in entries if name in main_changed and not reads_no_lane(status, name, crates, root)]
    if overlap:
        raise NotReused(f"since run {run} main changed {listing(overlap)}, which the pull request changes too")
    why = (f"run {run} checked head {old_head[:9]} on main {old_base[:9]}; since then the pull request changed only "
           "policy paths or registered desktop unit tests, and main none of the paths it changes that a lane reads")
    return why, delta


def plan(event, base, head, root=ROOT, crates=None, draft=False, last_green=None):
    """The lanes for base..head. `last_green` answers the commits of the pull
    request's latest green run, or raises; None skips the reuse check."""
    if draft:
        # A draft is not a merge candidate: no lane runs, `aggregate` fails
        # `verify` for it, and marking the pull request ready for review
        # starts the run that plans lanes.
        return {"full": False, "lanes": [], "rust_packages": [], "reasons": {}, "draft": True}
    if crates is None:
        try:
            crates = cargo_crates(root)
        except (subprocess.CalledProcessError, ValueError, KeyError) as error:
            # Without the crate graph no Rust change can be narrowed; the full
            # Rust lane runs the whole workspace and needs no package list.
            return select([], {}, root, f"crate graph unavailable: {error}")
    if event != "pull_request":
        return select([], crates, root, f"{event}: every lane runs on main")
    try:
        # A pull request's checkout is the merge commit GitHub made; anything
        # else would compare against a parent that is not the base.
        parents = subprocess.check_output(
            ["git", "rev-list", "--parents", "-n", "1", head], cwd=root, text=True, stderr=subprocess.PIPE
        ).split()
        if len(parents) != 3:
            raise ValueError(f"{head} is not a merge commit")
        entries = changed_entries(base, head, root)
    except (subprocess.CalledProcessError, UnicodeDecodeError, ValueError) as error:
        return select([], crates, root, f"comparison unavailable: {error_detail(error)}")
    result = select(entries, crates, root)
    result["commits"] = {"base": parents[1], "head": parents[2]}
    # `policy` alone is what a reuse plans; nothing to save.
    if last_green is None or result["lanes"] == ["policy"]:
        return result
    try:
        verified = last_green()
        why, delta = still_stands(verified, base, head, entries, crates, root)
    except (NotReused, subprocess.CalledProcessError, OSError, ValueError) as error:
        # Every failure keeps the plan the change itself needs.
        result["reuse"] = {"run": None, "why": error_detail(error)}
        return result
    # Never call select with an empty delta: an empty comparison deliberately
    # plans every lane. Preserve the original green anchor even after these
    # checks succeed, so a later run compares with the app code actually tested.
    rerun = select(delta, crates, root) if delta else {"lanes": ["policy"], "rust_packages": [], "reasons": {"policy": []}}
    rerun["reasons"]["policy"].append(f"reuses run {verified['run']}: {why}")
    return {
        "full": False,
        "lanes": rerun["lanes"],
        "rust_packages": rerun["rust_packages"],
        "reasons": rerun["reasons"],
        "commits": result["commits"],
        "reuse": {"run": verified["run"], "why": why},
        "verified": verified,
    }


def plan_record(result, number, run_id):
    """The app-tested anchor, preserved across policy and unit-only reruns."""
    commits = result.get("commits")
    verified = result.get("verified") or (commits and {"run": run_id, **commits})
    return {"version": RECORD_VERSION, "pr": number, "head": commits and commits["head"], "verified": verified}


def aggregate(needs):
    """Raise unless every planned lane succeeded and every other lane was skipped."""
    if set(needs) != set(LANES) | {"plan"}:
        missing = sorted((set(LANES) | {"plan"}) - set(needs))
        unknown = sorted(set(needs) - set(LANES) - {"plan"})
        raise ValueError(f"verify's needs do not match the lanes: missing {missing}, unknown {unknown}")
    if needs["plan"]["result"] != "success":
        raise ValueError(f"plan: {needs['plan']['result']}")
    # A skipped required check counts as passed, so a draft's `verify` fails
    # instead; the run that follows marking it ready replaces this one.
    if needs["plan"]["outputs"].get("draft") == "true":
        raise ValueError("draft: lanes not run, mark ready for review")
    planned = json.loads(needs["plan"]["outputs"]["lanes"])
    if not planned or not set(planned) <= set(LANES):
        raise ValueError(f"plan named no lane or an unknown one: {planned}")
    errors = []
    for lane in LANES:
        expected = "success" if lane in planned else "skipped"
        result = needs[lane]["result"]
        if result != expected:
            errors.append(f"{lane}: {'planned' if lane in planned else 'not planned'}, expected {expected}, got {result}")
    if errors:
        raise ValueError("; ".join(errors))


def summary(result, runs_url=None):
    if result.get("draft"):
        return "A draft pull request plans no lane; marking it ready for review runs them.\n"
    lines = ["| Lane | Runs | Why |", "| --- | --- | --- |"]
    for lane in LANES:
        reasons = result["reasons"].get(lane, [])
        why = "; ".join(reasons[:3]) + (f" (+{len(reasons) - 3} more)" if len(reasons) > 3 else "")
        lines.append(f"| {lane} | {'yes' if reasons else 'no'} | {why} |")
    if result["rust_packages"]:
        lines.append("")
        lines.append("Rust packages: " + ", ".join(result["rust_packages"]))
    reuse = result.get("reuse")
    if reuse:
        lines.append("")
        if reuse["run"] is None:
            lines.append(f"No earlier green run stands for this one: {reuse['why']}.")
        else:
            run = f"[run {reuse['run']}]({runs_url}{reuse['run']})" if runs_url else f"run {reuse['run']}"
            lines.append(f"Reuses {run} for unchanged lanes; this run checks {', '.join(result['lanes'])}: {reuse['why']}.")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    plan_parser = sub.add_parser("plan", help="classify base..head and write the job outputs")
    plan_parser.add_argument("--event", required=True)
    plan_parser.add_argument("--base", default="HEAD^1")
    plan_parser.add_argument("--head", default="HEAD")
    plan_parser.add_argument("--draft", choices=("true", "false"), default="false", help="the pull request is a draft")
    plan_parser.add_argument("--record", type=Path, help="write the plan record a pull request's next run reads here")
    sub.add_parser("aggregate", help="check toJSON(needs), read from NEEDS")
    args = parser.parse_args()

    if args.command == "plan":
        draft = args.draft == "true"
        repo = os.environ.get("GITHUB_REPOSITORY")
        run_id = int(os.environ["GITHUB_RUN_ID"]) if os.environ.get("GITHUB_RUN_ID") else None
        pull = None
        if args.event == "pull_request" and os.environ.get("GITHUB_EVENT_PATH"):
            pull = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))["pull_request"]
        last_green = None
        if pull is not None and repo and run_id is not None:
            # A deleted fork leaves the pull request no head repository.
            head_repo = (pull["head"].get("repo") or {}).get("full_name")

            def last_green():
                return last_green_run(repo, pull["number"], head_repo, pull["head"]["ref"], run_id)
        result = plan(args.event, args.base, args.head, draft=draft, last_green=last_green)
        print(json.dumps(result, indent=2))
        if args.record and pull is not None and not draft:
            args.record.write_text(json.dumps(plan_record(result, pull["number"], run_id), indent=2) + "\n", encoding="utf-8")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
                output.write(f"lanes={json.dumps(result['lanes'])}\n")
                output.write(f"full={str(result['full']).lower()}\n")
                output.write(f"draft={str(result.get('draft', False)).lower()}\n")
                output.write(f"rust-packages={json.dumps(result['rust_packages'])}\n")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
                server = os.environ.get("GITHUB_SERVER_URL", "https://github.com")
                output.write(summary(result, f"{server}/{repo}/actions/runs/" if repo else None))
    else:
        needs = json.loads(os.environ["NEEDS"])
        try:
            aggregate(needs)
        except ValueError as error:
            print(f"verify failed: {error}", file=sys.stderr)
            print(json.dumps(needs, indent=2), file=sys.stderr)
            sys.exit(1)
        print("every planned lane succeeded and every other lane was skipped")


if __name__ == "__main__":
    main()
