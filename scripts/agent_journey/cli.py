"""One command: every adapter's journey cells on one isolated candidate.

The candidate is the pinned Herdr on a private socket and this checkout's
hided on private state, both started by agent_live_check's Runtime under its
process guardian; agents run inside its write sandbox. See docs/VERIFICATION.md,
Agent journey.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import traceback

from agent_live_check.cli import default_herdr, operator_routing
from agent_live_check.credential_snapshot import CredentialSnapshotUnavailable
from agent_live_check.mid_turn import allow_hook_route
from agent_live_check.overlay import prepare as prepare_overlay
from agent_live_check.processes import OwnedProcesses, ProcessError
from agent_live_check.protection import (ConfigGuard, ProtectionError, beneath, private_directory,
                                         write_private)
from agent_live_check.runtime import Runtime, clean_env
from agent_live_check.sandbox import WriteSandbox
from agent_live_check.system import STOP_SIGNALS, WINDOWS, exe, git as git_program, launcher

from . import ci, clis, cells, fake_lane, fake_model, memory, omp_state, opencode_service, quota
from .controller import Controller
from .daemon import Daemon
from .journey import STEPS, Journey, token
from .local import (ROUTES, TRUST, allow_one_shot_references, claude_session_folders, disable_hide_ai,
                    opencode_model, operator_trust, route, wrapper)
from .report import save, table_lines

KIT_SECONDS = 90
SYSTEM_NAMES = {"darwin": "macOS", "linux": "Linux", "win32": "Windows"}


def systems(mode):
    """What this run says about each system: it measures its own."""
    this = SYSTEM_NAMES.get(sys.platform, sys.platform)
    return {name: (f"{mode} mode, this run" if name == this else
                   "not measured: local mode runs on macOS only; the CI layer measures it" if mode == "local" else
                   "not measured: another CI leg measures it")
            for name in SYSTEM_NAMES.values()}


def parser():
    value = argparse.ArgumentParser(description="Every agent's journey through hide agent spawn on an isolated candidate.")
    value.add_argument("--agents", help="Comma-separated adapter IDs; default is every adapter")
    value.add_argument("--model", action="append", default=[], metavar="ID=MODEL",
                       help="Passed through the product's own model flag; default is the account's model")
    value.add_argument("--mode", choices=("local", "ci"), default="local")
    value.add_argument("--run-dir", type=Path, help="New directory below this checkout's agents/runs")
    value.add_argument("--herdr-bin", type=Path, help="Previously fetched pinned Herdr; bytes are checked")
    value.add_argument("--fixture-bin", type=Path,
                       help="CI only: folder holding the compiled claude and codex stand-ins (web/e2e/shims)")
    value.add_argument("--unmeasured", action="append", default=[], metavar="ID=REASON",
                       help="Leaves a selected agent unstarted, its cells not measured for REASON (a lane's quota rotation)")
    value.add_argument("--fake-model", action="store_true",
                       help="The real CLIs answered by the journey's fake model (fake_model.py), with no key or login")
    return value


def refuse(reason):
    raise ProtectionError(reason)


def load_contract(checkout):
    rows = json.loads((checkout / "contracts/agent-journey.json").read_text())
    if not rows or any(set(row["cells"]) != set(cells.CELLS) for row in rows):
        refuse("journey_contract_unreadable: regenerate contracts/agent-journey.json with export_journey_contract")
    return rows


def leftover_copies(checkout, run):
    """Login copies an uncatchable kill left in earlier runs: named, never reused."""
    found = []
    for folder in sorted((checkout / "agents/runs").iterdir()):
        if folder == run or folder.is_symlink() or not folder.is_dir():
            continue
        copy = folder / "probe/daemon-home/.codex/auth.json"
        if copy.exists() or copy.is_symlink():
            found.append(str(copy))
    return found


def measured_source(checkout):
    """The commit of the checkout the journey runs from, which the measured
    repository is a clone of (PRD D-43)."""
    common = subprocess.run([git_program(), "-C", str(checkout), "rev-parse", "--path-format=absolute",
                             "--git-common-dir"], capture_output=True, text=True, timeout=15)
    if common.returncode:
        refuse("measured_repository_unreadable: run the journey from a Git checkout")
    if Path(common.stdout.strip()).name != ".git":
        refuse("measured_repository_is_bare: run the journey from a non-bare checkout")
    head = subprocess.run([git_program(), "-C", str(checkout), "rev-parse", "--verify", "HEAD"],
                          capture_output=True, text=True, timeout=15)
    if head.returncode:
        refuse("measured_repository_has_no_commit: run the journey from a checkout with a commit")
    return head.stdout.strip()


def measured_clone(runtime, checkout, commit):
    """The measured repository: a shared clone of the checkout's commit in the
    candidate's HOME, on `main`, as an operator's project is in theirs. Hide
    adds a project only inside its HOME (hided refuses another as
    `outside_home`), and a Wake after a Sleep that closed the pane finds the
    agent's checkout among the registered project's. The clone reads the
    checkout's objects and writes only its own, so the run's branches and
    worktrees never reach the operator's repository and go with the probe."""
    path = runtime.home / "projects/measured"
    private_directory(path.parent)
    git(runtime, ["clone", "-q", "--shared", "--no-checkout", str(checkout), str(path)], runtime.home)
    git(runtime, ["checkout", "-q", "-B", "main", commit], path)
    return path.resolve()


def register_measured(daemon, root):
    """Adds the measured repository as a project, as Add project does, and
    waits until the navigator lists it as a registered Git project."""
    daemon.dispatch("create_workspace", {"path": str(root), "label": "measured", "initialize_git": False})
    wanted = os.path.normcase(str(root))

    def registered(rest):
        for project in (rest.get("navigator") or {}).get("workspaces", []):
            if (os.path.normcase(str(Path(project.get("path") or ""))) == wanted
                    and project.get("registered") and project.get("is_git")):
                return project
        return None

    project = daemon.wait(registered, KIT_SECONDS, "the measured repository added as a project")
    return {"path": str(root), "checkouts": [checkout.get("path") for checkout in project.get("checkouts") or []]}


def check_operator_trust(report, operator, run, when):
    """Records the operator files' trust entries for this run's paths at
    `when`: a Codex entry there is an isolation failure, a Claude one is
    reported by key, and one left after the cleanup is a product finding."""
    trust = report.setdefault("operator_trust", {})
    if when == "after":
        # Claude's own session folders for the run's paths: listed, left alone.
        report["operator_session_folders"] = claude_session_folders(operator, run)
    try:
        found = trust[when] = operator_trust(operator, run)
    except ProtectionError as error:
        report["failures"].append({"type": "ProtectionError", "reason": str(error)})
        return
    for path, keys in found.items():
        if not keys:
            continue
        if path.endswith("config.toml"):
            report["failures"].append({"type": "IsolationError", "path": path,
                                       "reason": f"operator_codex_trust_written: {', '.join(keys)}"})
        elif when == "after":
            report.setdefault("product_findings", []).append(
                f"the candidate left Claude trust in the operator's {path} after its worktrees were removed: {', '.join(keys)}")


def claude_trusts(operator, root):
    """Whether the operator's Claude trusts the measured repository, read
    only: Claude Code 2.1.302 takes a folder's trust from the folder or the
    nearest one above it that is trusted."""
    try:
        projects = json.loads((operator / ".claude.json").read_text()).get("projects", {})
    except (OSError, ValueError):
        return False
    return any((projects.get(str(folder)) or {}).get("hasTrustDialogAccepted") is True
               for folder in (root, *root.parents))


def git(runtime, args, cwd):
    env = {**runtime.env, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    runtime.owner.run([git_program(), *args], env=env, cwd=cwd)


def repository(runtime, name):
    """A disposable repository nobody ever trusted, for the never-trusted start cell."""
    path = runtime.probe / name
    private_directory(path)
    git(runtime, ["init", "-q", "-b", "main"], path)
    write_private(path / "README.md", b"Disposable repository of one agent journey run.\n")
    git(runtime, ["add", "README.md"], path)
    git(runtime, ["-c", "user.name=Journey", "-c", "user.email=journey@invalid", "commit", "-qm", "Journey baseline"], path)
    return path


def package(runtime):
    """The candidate as an app package the kit installs from.

    Only a daemon running from a package installs the kit (hide-kit
    `bundled_kit_dir`), so this run's binaries are cloned into a private
    `hide.app/Contents/Resources` under the probe, which goes with the run.
    The operator's installed app is never read or started. On Windows the
    pinned Herdr's ConPTY runtime goes beside it, as the package carries it.
    """
    resources = runtime.probe / "package/hide.app/Contents/Resources"
    private_directory(resources)
    for name, source in (("hided", runtime.hided), ("hide", runtime.hide),
                         ("hide-agent-hooks", runtime.checkout / "target/debug" / exe("hide-agent-hooks")),
                         ("herdr", runtime.herdr_bin)):
        if not source.is_file():
            refuse(f"build_this_worktrees_{name.replace('-', '_')}_first")
        # On macOS an APFS clone: the probe's copy can change without touching the build.
        if sys.platform == "darwin":
            runtime.owner.run(["/bin/cp", "-c", str(source), str(resources / name)], env=runtime.env, seconds=30)
        else:
            shutil.copyfile(source, resources / exe(name))
            (resources / exe(name)).chmod(0o700)
    if WINDOWS:
        shutil.copytree(runtime.herdr_bin.parent / "conpty", resources / "conpty")
    runtime.hided, runtime.hide = resources / exe("hided"), resources / exe("hide")
    return resources


def wait_kit(daemon, ids):
    """The candidate kit's own report, as Settings reads it: Hide's hook and
    Herdr's integration of each agent in the private home, once each is
    installed or failed. Returns each failed piece's reason by agent."""
    def report(rest):
        devices = (rest.get("navigator") or {}).get("devices") or []
        device = next((device for device in devices if device.get("kind") == "local"), None)
        rows = {row.get("id"): row for row in ((device or {}).get("kit") or {}).get("agents") or []}
        failed = {}
        for key in ids:
            row = rows.get(key)
            if not row or not row.get("herdr"):
                return None
            for name, piece in (("hook", row.get("hook")), ("herdr", row["herdr"])):
                # No hook: the agent takes the skill only.
                if piece is None:
                    continue
                if piece.get("state") == "failed":
                    failed.setdefault(key, []).append(f"{name}: {piece.get('reason')}")
                elif piece.get("state") != "installed":
                    return None
        return {"failed": failed}
    return daemon.wait(report, KIT_SECONDS, "the kit to install each agent's hook and Herdr integration")["failed"]


def prepare_local(runtime, report, table, rows, measured, operator, root, fake, services):
    """Local mode's guard, write sandbox and each agent's route; the agents
    ready to measure. With `fake` (each agent's fake model URL) no login is
    read or copied: each agent's private folder points it at the fake."""
    run, owner = runtime.run, runtime.owner
    report["agents"].extend({"id": key} for key in measured)
    for key in measured:
        unmeasured = ROUTES[rows[key]["herdr_kind"]].get("unmeasured")
        if unmeasured:
            cells.settle_agent(table, key, unmeasured)
    measured = [key for key in measured if "unmeasured" not in ROUTES[rows[key]["herdr_kind"]]]
    kinds = [rows[key]["herdr_kind"] for key in measured]
    known = list(dict.fromkeys(operator / rel for kind in kinds for rel in ROUTES[kind]["known"]))
    roots = list(dict.fromkeys(operator / rel for kind in kinds for rel in ROUTES[kind]["roots"]))
    shared = {operator / rel: form for kind in kinds for rel, form in ROUTES[kind]["shared"].items()}
    guard = ConfigGuard(run / "configuration-backup", known, roots, shared=shared)
    executables = {key: shutil.which(rows[key]["herdr_kind"]) for key in measured}
    sandbox = WriteSandbox(run, runtime.short, operator, runtime.state, checkout=runtime.checkout,
                           routing=operator_routing(os.environ),
                           executables=[runtime.herdr_bin, Path(sys.executable),
                                        *(Path(value) for value in executables.values() if value)],
                           protected=[path for path in known if path not in shared])
    runtime.sandbox = sandbox
    with tempfile.TemporaryDirectory(prefix="ajc-guard-", dir="/tmp") as outside:
        sandbox.verify(owner, {**clean_env(), "TMPDIR": str(run / "agent-tmp")}, Path(outside).resolve())
    ready = []
    for key in measured:
        kind = rows[key]["herdr_kind"]
        entry = ROUTES[kind]
        agent = next(row for row in report["agents"] if row["id"] == key)
        if not executables[key]:
            cells.settle_agent(table, key, f"CLI not installed: {kind} is not on PATH")
            continue
        busy = entry.get("busy") and entry["busy"]()
        if busy:
            agent["busy"] = busy
            cells.settle_agent(table, key, busy)
            continue
        if entry.get("operator_state"):
            agent["operator_state"] = entry["operator_state"]
        extra = fake_lane.configure(kind, runtime.home, fake[key], root, local=True) if fake else None
        if extra is not None:
            agent["fake_env"] = sorted(extra)
        elif kind == "opencode":
            opencode_model(operator, runtime.home / opencode_service.CONFIG)
        launch = wrapper(runtime.bin, executables[key], operator, runtime.state, kind, runtime.home, sandbox,
                         os.environ.get("PATH", ""), extra)
        environment = {**clean_env(), "HOME": str(operator), "TMPDIR": str(run / "agent-tmp")}
        if kind == "opencode":
            agent["service_port"] = opencode_service.configure(runtime.home / opencode_service.CONFIG)
            services[key] = (launch, environment, agent["service_port"])
        _, version, _ = owner.run([str(launch), "--version"], env=environment, check=False)
        agent["version"] = version.strip()[:120]
        agent["pin"] = clis.pin(kind, agent["version"])
        if fake:
            ready.append(key)
            continue
        if kind == "claude" and not claude_trusts(operator, root):
            agent["trust"] = f"Claude trusts neither the measured repository {root} nor a folder above it"
            cells.settle_agent(table, key, "Claude trusts no folder above the measured repository; open Claude once in "
                               f"{runtime.checkout} and accept")
            continue
        private = runtime.home / entry["config"] if entry["config"] else None
        if entry["copies"]:
            runtime.credential_roots.add(private)
            recipe = {"id": key, "overlay": {"env": {}, "copies": entry["copies"], "settings": [],
                                             "provenance": entry["provenance"]}}
            try:
                agent["private_configuration"] = prepare_overlay(recipe, runtime.probe, operator, owner=owner,
                                                                 env=clean_env(), root=private)["copies"]
            except CredentialSnapshotUnavailable as error:
                # Another process has the login open; it is never ended for the journey.
                agent["login"] = (f"{kind}'s login is open in another process ({error}); close that {kind} "
                                  "yourself, then rerun; the journey never ends another session's agent")
                cells.settle_agent(table, key, agent["login"])
                continue

        def status(args, launch=launch, environment=environment):
            code, output, _ = owner.run([str(launch), *args], env=environment, check=False)
            return code, output
        refusal = entry["login"](private, status)
        if refusal:
            agent["login"] = refusal
            cells.settle_agent(table, key, refusal)
            # A login the run will not use leaves no copy behind.
            for _, name in entry["copies"]:
                for suffix in ("", "-wal", "-shm"):
                    (private / (name + suffix)).unlink(missing_ok=True)
            continue
        ready.append(key)
    return sandbox, guard, ready


def prepare_ci(runtime, report, table, rows, measured, fixture_bin, root, fake, services):
    """CI mode's agents in the disposable HOME, signed in with the lane's keys,
    pointed at the fake model (`fake`, each agent's URL) or replaced by the
    fixture stand-ins; the agents ready to measure."""
    ready = []
    for key in measured:
        kind = rows[key]["herdr_kind"]
        agent = {"id": key}
        report["agents"].append(agent)
        secret = ci.not_configured(kind, fixture_bin, bool(fake))
        if secret:
            agent["not_configured"] = secret
            cells.settle_agent(table, key, f"not configured ({secret})")
            continue
        program = ci.executable(fixture_bin, kind)
        if not program:
            agent["cli_missing"] = kind
            cells.settle_agent(table, key, f"CLI not installed: {kind} is not on PATH")
            continue
        # The kit installs an agent's hooks only where its own folder is, as
        # a CLI that has run once leaves it; a new runner has none. Made
        # before the CLI first runs, as Codex writes into it (CODEX_HOME).
        private_directory(runtime.home.joinpath(*rows[key]["home_folder"]))
        fake_env = fake_lane.configure(kind, runtime.home, fake[key], root, local=False) if fake else None
        if fake_env is not None:
            agent["fake_env"] = sorted(fake_env)
        launch = ci.wrapper(runtime, program, kind, os.environ.get("PATH", ""),
                            runtime.probe / f"fixture-{kind}.config" if fixture_bin else None, fake_env)
        environment = {**clean_env(), "HOME": str(runtime.home), "TMPDIR": runtime.env["TMPDIR"]}
        if kind == "opencode":
            agent["service_port"] = opencode_service.configure(runtime.home / opencode_service.CONFIG)
            services[key] = (launch, environment, agent["service_port"])
        _, version, _ = runtime.owner.run([str(launch), "--version"], env=environment, check=False)
        agent["version"] = version.strip()[:120]
        agent["pin"] = None if fixture_bin else clis.pin(kind, agent["version"])
        if not fixture_bin and not fake:
            refusal = ci.login(runtime, kind, launch, root)
            if refusal:
                agent["login"] = refusal
                cells.settle_agent(table, key, refusal)
                continue
        ready.append(key)
    return ready


def hide_ai_requests(log):
    """The candidate's Hide AI model requests by provider, one per attempt
    its log records with a duration; none when it never ran."""
    found = {}
    if not log.is_file():
        return found
    for line in log.read_text(errors="replace").splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if isinstance(record, dict) and "feature_id" in record and "duration_ms" in record:
            found[record.get("provider")] = found.get(record.get("provider"), 0) + 1
    return found


def budget_failures(agents):
    """A signed-in run's agents over their request budget, by name (PRD B29)."""
    return [f"request_budget_exceeded: {agent['id']} made {agent['requests']} model requests, "
            f"budget {agent['request_budget']}" for agent in agents
            if agent.get("request_budget") is not None and agent.get("requests", 0) > agent["request_budget"]]


def main(argv=None):
    args = parser().parse_args(argv)
    started = time.time()
    checkout = Path(__file__).resolve().parents[2]
    operator = Path.home().resolve()
    local = args.mode == "local"
    if local and args.fixture_bin:
        refuse("fixture_bin_is_ci_only: the stand-ins run with --mode ci")
    if args.fake_model and args.fixture_bin:
        refuse("fake_model_or_fixture_bin: the stand-ins answer for themselves")
    fake = args.fake_model
    if local and sys.platform != "darwin":
        refuse("local_mode_runs_on_macos_only: it needs sandbox-exec and the keychain logins; the CI layer measures other systems")
    fixture_bin = args.fixture_bin.resolve() if args.fixture_bin else None
    contract = load_contract(checkout)
    # A signed-in CI run reads which refusals leave a turn unmeasured before
    # it spends a request; a missing or misshapen contract stops it here.
    quota_rows = None
    if not local and not fixture_bin and not fake:
        try:
            quota_rows = quota.contract()
        except ValueError as error:
            refuse(str(error))
    known_ids = [row["id"] for row in contract]
    selected = args.agents.split(",") if args.agents else [key for key in known_ids if key not in cells.EXCLUDED_AGENTS]
    if not selected or len(set(selected)) != len(selected) or set(selected) - set(known_ids):
        refuse("invalid_agent_selection")
    for key in selected:
        if key in cells.EXCLUDED_AGENTS:
            refuse(f"agent_excluded: {key} is excluded from the journey ({cells.EXCLUDED_AGENTS[key]})")
    models = {}
    for entry in args.model:
        key, separator, model = entry.partition("=")
        if (not separator or key not in selected or not model or key in models or model.startswith("-")
                or not next(row for row in contract if row["id"] == key)["model_flag"]):
            refuse("invalid_model_override")
        models[key] = model
    unmeasured = {}
    for entry in args.unmeasured:
        key, separator, reason = entry.partition("=")
        if not separator or key not in selected or not reason.strip() or key in unmeasured:
            refuse("invalid_unmeasured")
        unmeasured[key] = reason.strip()
    if fixture_bin:
        # The stand-ins imitate Claude Code and Codex only (web/e2e/shims/live-check.h).
        for key in selected:
            if key in cells.BUILT_AGENTS and key not in ci.FIXTURE_AGENTS and key not in unmeasured:
                if args.agents:
                    refuse(f"fixture_bin_imitates_claude_code_and_codex_only: {key}")
                unmeasured[key] = ci.FIXTURE_ONLY
    herdr = args.herdr_bin or default_herdr(checkout, operator)
    if not herdr.is_file():
        refuse("pinned_herdr_missing: run zsh scripts/fetch-herdr-runtime.sh, or pass --herdr-bin")
    reader = checkout / "target/debug/examples" / exe("journey_conversation")
    if not reader.is_file():
        refuse("build_the_reader_first: bash scripts/verify-cargo.sh build -p hide-session --example journey_conversation")
    card_writer = checkout / "target/debug/examples" / exe("journey_memory")
    if not card_writer.is_file():
        refuse("build_the_memory_card_writer_first: bash scripts/verify-cargo.sh build -p hide-memory --example journey_memory")
    slug = ("agent-journey-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S")
            + "-" + os.urandom(3).hex())
    run = (args.run_dir or checkout / "agents/runs" / slug).absolute()
    if run.exists() or run.is_symlink() or not beneath(run, checkout / "agents/runs"):
        refuse("report_directory_must_be_new_and_local_only")
    commit = measured_source(checkout)
    run_id = slug.removeprefix("agent-journey-") if not args.run_dir else run.name
    measured = [key for key in selected if key in cells.BUILT_AGENTS and key not in unmeasured]
    private_directory(run)
    rows = {row["id"]: row for row in contract}
    table = cells.plan(contract, selected)
    report = {"format": 1, "mode": args.mode, "fixture": bool(fixture_bin), "selected": selected, "models": models,
              "herdr": {}, "agents": [], "table": table, "systems": systems(args.mode), "configuration": {},
              **({"fixture_limits": ci.FIXTURE_LIMITS} if fixture_bin else {}),
              **({"fake_model": True, "fake_model_limits": fake_lane.LIMITS} if fake else {}),
              "leftover_copies": leftover_copies(checkout, run), "measured_commit": commit, "created": [],
              "cleanup": {"confirmed": False}, "failures": []}
    for key, reason in unmeasured.items():
        report["agents"].append({"id": key, "unmeasured": reason})
        cells.settle_agent(table, key, reason)
    owner = OwnedProcesses(diagnostics=run / "process-diagnostics")
    previous = {signum: signal.signal(signum, lambda *_: owner.cancelled.set())
                for signum in STOP_SIGNALS}
    runtime = guard = daemon = None
    # One fake model per agent, so each one's log counts that agent's requests alone.
    servers = {}
    # Each OpenCode agent's program, environment and private server port, whose server the run ends.
    services = {}
    try:
        home = run / "probe/daemon-home"
        # The macOS sandbox opens the probe to agents, so a local run keeps the
        # state outside it. A CI run keeps it where an operator's is, under
        # HOME, which the kit trusts as its anchor: a state folder elsewhere it
        # inspects from the drive root, and a Windows runner's D:\ lets every
        # account change its entries.
        runtime = Runtime(checkout, run, operator, owner, herdr_bin=herdr, home=home,
                          state=None if local else home / ".hide/state")
        report["package"] = str(package(runtime))
        if fake:
            for key in measured:
                servers[key] = fake_model.serve(run / f"fake-model-{key}.jsonl")[0]
        urls = {key: f"http://127.0.0.1:{server.server_address[1]}" for key, server in servers.items()}
        # Codex's sessions belong in its private home, never the operator's, and
        # in CI mode or with the fake model Claude's do too: it writes them there.
        for relative in (".codex/sessions",) if local and not fake else (".codex/sessions", ".claude/projects"):
            sessions = runtime.home / relative
            if sessions.is_symlink():
                sessions.unlink()
        launcher(runtime.bin / "hide", [runtime.hide])
        root = measured_clone(runtime, checkout, commit)
        report["measured_repository"] = str(root)
        if local or fake:
            # Before hided starts, which reads the file once.
            disable_hide_ai(runtime.home)
        if local:
            sandbox, guard, ready = prepare_local(runtime, report, table, rows, measured, operator, root, urls, services)
        else:
            sandbox, ready = None, prepare_ci(runtime, report, table, rows, measured, fixture_bin, root, urls,
                                              services)
        report["herdr"] = runtime.start()
        if sandbox:
            allow_hook_route(sandbox, runtime)
            allow_one_shot_references(sandbox, runtime)
        if ready:
            daemon = Daemon(runtime.state, owner.cancelled)
            # The operator's first-run answer, once the daemon asks for it.
            daemon.wait(lambda rest: (rest.get("ui_state") or {}).get("agent_onboarding") == "pending",
                        KIT_SECONDS, "the first-run agent question")
            daemon.dispatch("agent_onboarding_apply", {"agents": ready, "finish": True})
            for key, pieces in wait_kit(daemon, ready).items():
                # The product's own answer: the agent cannot start as Hide sets it up.
                cells.settle(table, key, "start", cells.FAIL, "the kit did not install " + "; ".join(pieces))
                ready.remove(key)
            report["measured_project"] = register_measured(daemon, root)
            untrusted = repository(runtime, "repo-untrusted")
            for kind in {rows[key]["herdr_kind"] for key in ready} & TRUST.keys():
                path, write = TRUST[kind]
                write(runtime.home / path, root)
            if fixture_bin:
                for key in ready:
                    ci.fixture_config(runtime, rows[key]["herdr_kind"])
                    for name, reason in ci.FIXTURE_UNIMITATED.items():
                        cells.settle(table, key, name, cells.NOT_MEASURED, reason)
            controller = Controller(runtime)
            report["controller"] = controller.start()
            # Before any agent starts, so each start's guidance carries it.
            card = report["memory"] = (memory.seed(runtime, daemon, root, card_writer, token())
                                       if any(table[key][memory.CELL]["status"] == cells.PENDING for key in ready)
                                       else None)
            for key in ready:
                agent = next(row for row in report["agents"] if row["id"] == key)
                journey = Journey(runtime, daemon, controller, table, rows[key], root, agent, reader,
                                  branch=f"journey/{run_id}-{key}", created=report["created"],
                                  sessions=(ROUTES[rows[key]["herdr_kind"]]["sessions"](operator, runtime.home)
                                            if local and not fake else runtime.home),
                                  route=({**route(rows[key]["herdr_kind"], runtime.home),
                                          **({"fake_env": agent["fake_env"]} if fake else {})} if local
                                         else {"args": [], "env": ci.route_names(rows[key]["herdr_kind"], fixture_bin,
                                                                                 agent.get("fake_env"))}),
                                  model=rows[key]["model_flag"] and models.get(key),
                                  quota=quota_rows)
                for step in STEPS:
                    journey.step(step, *((card,) if step == memory.CELL else ()))
                if key in services:
                    # Its pane closed at Sleep and opened again at Wake: one server must still serve it.
                    running = opencode_service.ours(str(runtime.home))
                    agent["opencode_servers_after_wake"] = running
                    if running is not None and len(running) > 1:
                        report.setdefault("product_findings", []).append(
                            f"{len(running)} OpenCode servers of the run ran after Sleep and Wake: {running}")
                untrusted_reason = (ROUTES[rows[key]["herdr_kind"]]["untrusted"] if local and not fake else
                                    ci.FIXTURE_UNTRUSTED if fixture_bin else None)
                if untrusted_reason:
                    cells.settle(table, key, "start_untrusted", cells.NOT_MEASURED,
                                 untrusted_reason.removeprefix("not measured: "))
                else:
                    journey.step("untrusted", untrusted)
                agent["requests"] = journey.requests()
                if fake:
                    # What a provider would have been asked; the session files may not show each one.
                    agent["session_requests"] = agent["requests"]
                    agent["requests"] = fake_lane.requests(run / f"fake-model-{key}.jsonl")
                agent["request_budget"] = rows[key].get("request_budget")
                journey.close()
    except Exception as error:
        # A stop the operator asked for is not a failure; the cells say "interrupted".
        if not (owner.cancelled.is_set() and isinstance(error, ProcessError) and "cancelled" in str(error)):
            report["failures"].insert(0, {"type": type(error).__name__, "reason": str(error),
                                          "at": [f"{Path(frame.filename).name}:{frame.lineno} {frame.name}"
                                                 for frame in traceback.extract_tb(error.__traceback__)[-4:]],
                                          **({"path": error.path} if isinstance(error, ProtectionError) and error.path else {})})
    finally:
        for server in servers.values():
            server.shutdown()
            server.server_close()
        if daemon:
            daemon.close()
        for key, (launch, environment, port) in services.items():
            # Before the owner closes: the server outlives every terminal, and the run started it.
            agent = next(row for row in report["agents"] if row["id"] == key)
            try:
                agent["opencode_service"] = opencode_service.stop(launch, environment, port, str(runtime.home),
                                                                  runtime.owner)
            except Exception as error:
                agent["opencode_service"] = {"port": port, "left": [f"stop failed: {error}"]}
            if agent["opencode_service"]["left"]:
                report["failures"].append({"type": "CleanupError", "reason": "opencode_server_left: "
                                           + json.dumps(agent["opencode_service"]["left"])})
        if runtime:
            # Herdr's own log lives in the private home the cleanup removes.
            log = runtime.home / "config/herdr/herdr-server.log"
            if log.is_file() and not log.is_symlink():
                with open(log, "rb") as stream:
                    write_private(run / "herdr-server.log", ci.redact(stream.read(1024 * 1024)))
        if local:
            # Before the cleanup, which removes the trust the candidate recorded with its worktrees.
            check_operator_trust(report, operator, run, "during")
        # omp's sessions, named in its state in the operator's HOME, before the cleanup removes the private home.
        omp_sessions = (omp_state.sessions(runtime.home / ".omp/agent")
                        if local and runtime and any(rows[key]["herdr_kind"] == "omp" for key in measured) else None)
        # Every folder an agent of the run started in, each start's own worktree included.
        started_in = ([path for folder in ("worktrees", "worktrees-untrusted") for path in (runtime.probe / folder).glob("*")]
                      + [runtime.probe / "repo-untrusted"]) if omp_sessions is not None else []
        try:
            if runtime:
                report["cleanup"] = runtime.close()
            else:
                owner.close()
                report["cleanup"] = {"confirmed": True}
        except Exception as error:
            report["cleanup"] = {"confirmed": False, "failures": [str(error)]}
            try:
                owner.close()
            except Exception as cleanup_error:
                report["cleanup"]["failures"].append(str(cleanup_error))
        if omp_sessions is not None and report["cleanup"].get("processes_confirmed"):
            # After every process ended, so no omp still writes its state.
            projects = [Path(item["worktree"]) for item in report["created"] if item.get("worktree")]
            try:
                report["operator_state"] = omp_state.take_back(
                    operator, list(dict.fromkeys([*projects, *started_in])), omp_sessions, started)
            except OSError as error:
                report["failures"].append({"type": "OSError", "reason": f"omp_state_cleanup_failed: {error}"})
        # Hide AI's model requests, apart from the agents' budget: none in
        # local mode, where it is off, and counted in CI mode, where it is on.
        try:
            report["hide_ai_requests"] = hide_ai_requests(run / "hided.log")
        except OSError as error:
            report["hide_ai_requests"] = {}
            report["failures"].append({"type": "OSError", "reason": f"hided_log_unreadable: {error}"})
        if local and report["hide_ai_requests"]:
            report["failures"].append({"type": "IsolationError", "reason": "hide_ai_ran_in_local_mode: "
                                       + json.dumps(report["hide_ai_requests"])})
        if guard:
            try:
                report["configuration"] = guard.finish()
            except Exception as error:
                report["configuration"] = {"failures": [{"reason": str(error)}]}
        if local:
            check_operator_trust(report, operator, run, "after")
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    interrupted = owner.cancelled.is_set()
    cells.close(table, interrupted)
    setup_failed = bool(report["failures"] or report["configuration"].get("failures")
                        or not report["cleanup"].get("confirmed"))
    # An interrupted run says so with 3 whatever it measured, or 2 when its cleanup is unconfirmed.
    code = (2 if setup_failed else 3) if interrupted else cells.exit_code(table, selected, setup_failed=setup_failed)
    report["budget_failures"] = budget_failures(report["agents"]) if fake or (not local and not fixture_bin) else []
    if report["budget_failures"] and code in (0, 3):
        code = 1
    report["exit_code"] = code
    save(run, report)
    print("\n".join(table_lines(report)))
    print(f"{run / 'report.md'}\n{run / 'report.json'}\nexit={code}")
    return code
