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
import traceback

from agent_live_check.cli import default_herdr, operator_routing
from agent_live_check.mid_turn import allow_hook_route
from agent_live_check.overlay import prepare as prepare_overlay
from agent_live_check.processes import OwnedProcesses, ProcessError
from agent_live_check.protection import (ConfigGuard, ProtectionError, beneath, private_directory,
                                         write_private)
from agent_live_check.runtime import Runtime, clean_env
from agent_live_check.sandbox import WriteSandbox
from agent_live_check.system import STOP_SIGNALS, WINDOWS, exe, git as git_program, launcher

from . import ci, cells, memory, quota
from .controller import Controller
from .daemon import Daemon
from .journey import STEPS, Journey, token
from .local import (ROUTES, allow_one_shot_references, claude_session_folders, codex_login_refusal, disable_hide_ai,
                    operator_trust, route, trust_codex, wrapper)
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
CODEX_LOGIN_COPY = {"id": "codex", "overlay": {
    "env": {"CODEX_HOME": "."}, "copies": [[".codex/auth.json", "auth.json"]], "settings": [],
    "provenance": "A private CODEX_HOME holding a copy of the login (docs/AI_PROVIDERS.md); a refresh stays in the copy."}}


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


def measured_repository(checkout):
    """The main repository of the checkout the journey runs from (PRD D-43)."""
    common = subprocess.run([git_program(), "-C", str(checkout), "rev-parse", "--path-format=absolute",
                             "--git-common-dir"], capture_output=True, text=True, timeout=15)
    if common.returncode:
        refuse("measured_repository_unreadable: run the journey from a Git checkout")
    root = Path(common.stdout.strip())
    if root.name != ".git":
        refuse("measured_repository_is_bare: run the journey from a non-bare checkout")
    return root.parent.resolve()


def journey_leftovers(root, checkout, run):
    """Branches and worktrees an uncatchable kill left in the measured
    repository: named for the operator, never reused or removed."""
    branches = subprocess.run([git_program(), "-C", str(root), "for-each-ref", "--format=%(refname:short)",
                               "refs/heads/journey/"], capture_output=True, text=True, timeout=15).stdout.split()
    listed = subprocess.run([git_program(), "-C", str(root), "worktree", "list", "--porcelain"],
                            capture_output=True, text=True, timeout=15).stdout
    runs = (checkout / "agents/runs").resolve()
    worktrees = [line.removeprefix("worktree ") for line in listed.splitlines() if line.startswith("worktree ")
                 and Path(line.removeprefix("worktree ")).is_relative_to(runs)
                 and not Path(line.removeprefix("worktree ")).is_relative_to(run.resolve())]
    return {"branches": branches, "worktrees": worktrees}


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
    """Whether the operator's Claude trusts the measured repository, read only."""
    try:
        projects = json.loads((operator / ".claude.json").read_text()).get("projects", {})
    except (OSError, ValueError):
        return False
    return (projects.get(str(root)) or {}).get("hasTrustDialogAccepted") is True


def taken_branches(root, branches):
    """Which of `branches` the measured repository already has."""
    return [branch for branch in branches
            if not subprocess.run([git_program(), "-C", str(root), "rev-parse", "--verify", "-q", "refs/heads/" + branch],
                                  capture_output=True, timeout=15).returncode]


def plain_git(env, root, *args):
    """Git in the measured repository, outside the owner: the repository
    cleanup runs after the owner closed, and after a stop or the deadline."""
    result = subprocess.run([git_program(), "-C", str(root), *args], env=env, capture_output=True, text=True,
                            timeout=15)
    return result.returncode, result.stdout, result.stderr


def registered_worktrees(env, root):
    """Each registered worktree's path and branch in the measured repository."""
    code, out, err = plain_git(env, root, "worktree", "list", "--porcelain")
    if code:
        raise ProtectionError("worktree_list_failed: " + err.strip()[:200])
    found, path = {}, None
    for line in out.splitlines():
        if line.startswith("worktree "):
            path = line.removeprefix("worktree ")
        elif line.startswith("branch ") and path:
            found[path] = line.removeprefix("branch refs/heads/")
    return found


def remove_created(runtime, root, created, branches):
    """Deletes the worktrees and branches this run made, each only while its
    name and start commit still match the run's record; anything else stays
    and is named. A branch the record lacks a start for (a spawn that raised
    or was stopped) and one of the run's own `journey/<run_id>-<agent>` names
    the record lacks are read now: that name is this run's alone."""
    removed, kept = [], []
    env = {**runtime.env, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    worktrees = registered_worktrees(env, root)
    items = [dict(item) for item in created]
    for branch in branches:
        code, _, _ = plain_git(env, root, "rev-parse", "--verify", "-q", "refs/heads/" + branch)
        if not code and not any(item["branch"] == branch for item in items):
            path = next((path for path, name in worktrees.items() if name == branch), None)
            items.append({"branch": branch, "start": None, "worktree": path, "swept": True})
    probe = runtime.probe.resolve()
    for item in items:
        if not item["start"]:
            code, tip, _ = plain_git(env, root, "rev-parse", "--verify", "-q", "refs/heads/" + item["branch"])
            if code:
                continue  # the spawn never made it
            item.update(start=tip.strip(), start_read_at_cleanup=True)
        worktree = item["worktree"]
        registered = worktree and (worktree in worktrees or str(Path(worktree).resolve()) in worktrees)
        if registered:
            if not Path(worktree).resolve().is_relative_to(probe):
                kept.append({**item, "reason": "worktree_outside_the_run"})
                continue
            code, _, err = plain_git(env, root, "worktree", "remove", "--force", worktree)
            if code:
                kept.append({**item, "reason": "worktree_remove_failed: " + err.strip()[:200]})
                continue
        code, tip, _ = plain_git(env, root, "rev-parse", "--verify", "-q", "refs/heads/" + item["branch"])
        if code:
            removed.append(item)
            continue
        if tip.strip() != item["start"]:
            kept.append({**item, "reason": "branch_moved_since_the_run_made_it"})
            continue
        code, _, err = plain_git(env, root, "branch", "-D", item["branch"])
        (kept if code else removed).append({**item, **({"reason": err.strip()[:200]} if code else {})})
    return {"removed": removed, "kept": kept}


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


def wait_kit(runtime, kinds):
    """The candidate kit installed Hide's hooks and Herdr's integration in the private home."""
    files = {"claude": [runtime.home / ".claude/settings.json"],
             "codex": [runtime.home / ".codex/hooks.json"]}

    def installed():
        for kind in kinds:
            for file in files[kind]:
                if not file.is_file():
                    return False
                content = file.read_text(errors="replace")
                if "hide-agent-hooks" not in content or "herdr" not in content.lower():
                    return False
        return True
    runtime.wait(installed, KIT_SECONDS)


def prepare_local(runtime, report, table, rows, measured, operator, root):
    """Local mode's guard, write sandbox and each agent's route; the agents
    ready to measure."""
    run, owner = runtime.run, runtime.owner
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
        agent = {"id": key}
        report["agents"].append(agent)
        if not executables[key]:
            cells.settle_agent(table, key, f"CLI not installed: {kind} is not on PATH")
            continue
        launch = wrapper(runtime.bin, executables[key], operator, runtime.state, kind, runtime.home, sandbox,
                         os.environ.get("PATH", ""))
        environment = {**clean_env(), "HOME": str(operator), "TMPDIR": str(run / "agent-tmp")}
        _, version, _ = owner.run([str(launch), "--version"], env=environment, check=False)
        agent["version"] = version.strip()[:120]
        if kind == "codex":
            refusal = codex_login_refusal(operator / ".codex/auth.json")
            if refusal:
                agent["login"] = refusal
                cells.settle_agent(table, key, refusal)
                continue
            codex_home = runtime.home / ".codex"
            runtime.credential_roots.add(codex_home)
            agent["private_configuration"] = prepare_overlay(CODEX_LOGIN_COPY, runtime.probe, operator,
                                                             owner=owner, env=clean_env(), root=codex_home)["copies"]
        if kind == "claude" and not claude_trusts(operator, root):
            agent["trust"] = f"the measured repository {root} is not trusted in Claude"
            cells.settle_agent(table, key, f"the measured repository is not trusted in Claude; open Claude once in {root} and accept")
            continue
        if kind == "claude":
            code, output, _ = owner.run([str(launch), "auth", "status"], env=environment, check=False)
            try:
                logged_in = json.loads(output).get("loggedIn") is True
            except ValueError:
                logged_in = False
            if code or not logged_in:
                agent["login"] = "not logged in: run claude auth login yourself; the journey never logs in"
                cells.settle_agent(table, key, agent["login"])
                continue
        ready.append(key)
    return sandbox, guard, ready


def prepare_ci(runtime, report, table, rows, measured, fixture_bin, root):
    """CI mode's agents in the disposable HOME, signed in with the lane's keys
    or replaced by the fixture stand-ins; the agents ready to measure."""
    ready = []
    for key in measured:
        kind = rows[key]["herdr_kind"]
        agent = {"id": key}
        report["agents"].append(agent)
        secret = ci.not_configured(kind, fixture_bin)
        if secret:
            agent["not_configured"] = secret
            cells.settle_agent(table, key, f"not configured ({secret})")
            continue
        program = ci.executable(fixture_bin, kind)
        if not program:
            agent["cli_missing"] = kind
            cells.settle_agent(table, key, f"CLI not installed: {kind} is not on PATH")
            continue
        launch = ci.wrapper(runtime, program, kind, os.environ.get("PATH", ""),
                            runtime.probe / f"fixture-{kind}.config" if fixture_bin else None)
        # The kit installs an agent's hooks only where its own folder is, as
        # a CLI that has run once leaves it; a new runner has none. Made
        # before the CLI first runs, as Codex writes into it (CODEX_HOME).
        private_directory(runtime.home / ci.HOME_FOLDERS[kind])
        environment = {**clean_env(), "HOME": str(runtime.home), "TMPDIR": runtime.env["TMPDIR"]}
        _, version, _ = runtime.owner.run([str(launch), "--version"], env=environment, check=False)
        agent["version"] = version.strip()[:120]
        if not fixture_bin:
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
    checkout = Path(__file__).resolve().parents[2]
    operator = Path.home().resolve()
    local = args.mode == "local"
    if local and args.fixture_bin:
        refuse("fixture_bin_is_ci_only: the stand-ins run with --mode ci")
    if local and sys.platform != "darwin":
        refuse("local_mode_runs_on_macos_only: it needs sandbox-exec and the keychain logins; the CI layer measures other systems")
    fixture_bin = args.fixture_bin.resolve() if args.fixture_bin else None
    contract = load_contract(checkout)
    # A signed-in CI run reads which refusals leave a turn unmeasured before
    # it spends a request; a missing or misshapen contract stops it here.
    quota_rows = None
    if not local and not fixture_bin:
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
    root = measured_repository(checkout)
    run_id = slug.removeprefix("agent-journey-") if not args.run_dir else run.name
    measured = [key for key in selected if key in cells.BUILT_AGENTS]
    # The cleanup deletes the run's branch names, so none may be there before it starts.
    taken = taken_branches(root, [f"journey/{run_id}-{key}" for key in measured])
    if taken:
        refuse(f"journey_branch_exists: {', '.join(taken)} is in {root}; remove it or choose another --run-dir")
    private_directory(run)
    rows = {row["id"]: row for row in contract}
    table = cells.plan(contract, selected)
    report = {"format": 1, "mode": args.mode, "fixture": bool(fixture_bin), "selected": selected, "models": models,
              "herdr": {}, "agents": [], "table": table, "systems": systems(args.mode), "configuration": {},
              **({"fixture_limits": ci.FIXTURE_LIMITS} if fixture_bin else {}),
              "leftover_copies": leftover_copies(checkout, run), "measured_repository": str(root),
              "leftovers": journey_leftovers(root, checkout, run), "created": [],
              "cleanup": {"confirmed": False}, "failures": []}
    owner = OwnedProcesses(diagnostics=run / "process-diagnostics")
    previous = {signum: signal.signal(signum, lambda *_: owner.cancelled.set())
                for signum in STOP_SIGNALS}
    runtime = guard = daemon = None
    try:
        runtime = Runtime(checkout, run, operator, owner, herdr_bin=herdr,
                          home=run / "probe/daemon-home")
        report["package"] = str(package(runtime))
        # Codex's sessions belong in its private home, never the operator's, and
        # in CI mode Claude's do too: its agents run in this home.
        for relative in (".codex/sessions",) if local else (".codex/sessions", ".claude/projects"):
            sessions = runtime.home / relative
            if sessions.is_symlink():
                sessions.unlink()
        launcher(runtime.bin / "hide", [runtime.hide])
        if local:
            # Before hided starts, which reads the file once.
            disable_hide_ai(runtime.home)
            sandbox, guard, ready = prepare_local(runtime, report, table, rows, measured, operator, root)
        else:
            sandbox, ready = None, prepare_ci(runtime, report, table, rows, measured, fixture_bin, root)
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
            wait_kit(runtime, [rows[key]["herdr_kind"] for key in ready])
            untrusted = repository(runtime, "repo-untrusted")
            if "codex" in ready:
                trust_codex(runtime.home / ".codex/config.toml", root)
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
                                            if local else runtime.home),
                                  route=(route(rows[key]["herdr_kind"], runtime.home) if local
                                         else {"args": [], "env": ci.route_names(rows[key]["herdr_kind"], fixture_bin)}),
                                  model=rows[key]["model_flag"] and models.get(key),
                                  quota=quota_rows)
                for step in STEPS:
                    journey.step(step, *((card,) if step == memory.CELL else ()))
                untrusted_reason = (ROUTES[rows[key]["herdr_kind"]]["untrusted"] if local else
                                    ci.FIXTURE_UNTRUSTED if fixture_bin else None)
                if untrusted_reason:
                    cells.settle(table, key, "start_untrusted", cells.NOT_MEASURED,
                                 untrusted_reason.removeprefix("not measured: "))
                else:
                    journey.step("untrusted", untrusted)
                agent["requests"] = journey.requests()
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
        if daemon:
            daemon.close()
        if runtime:
            # Herdr's own log lives in the private home the cleanup removes.
            log = runtime.home / "config/herdr/herdr-server.log"
            if log.is_file() and not log.is_symlink():
                with open(log, "rb") as stream:
                    write_private(run / "herdr-server.log", ci.redact(stream.read(1024 * 1024)))
        if local:
            # Before the cleanup, which removes the trust the candidate recorded with its worktrees.
            check_operator_trust(report, operator, run, "during")
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
        if runtime and not report["cleanup"].get("processes_confirmed"):
            # An agent that may still run keeps its worktree and branch.
            report["repository_cleanup"] = {"removed": [], "kept": [
                {**item, "reason": "processes_unconfirmed"} for item in report["created"]]}
        elif runtime:
            # After every process ended, so no agent still writes in a worktree.
            try:
                report["repository_cleanup"] = remove_created(runtime, root, report["created"],
                                                              [f"journey/{run_id}-{key}" for key in measured])
            except Exception as error:
                report["repository_cleanup"] = {"removed": [], "failure": str(error), "kept": [
                    {**item, "reason": "cleanup_failed: " + str(error)[:200]} for item in report["created"]]}
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
                        or not report["cleanup"].get("confirmed")
                        or (report.get("repository_cleanup") or {}).get("kept"))
    # An interrupted run says so with 3 whatever it measured, or 2 when its cleanup is unconfirmed.
    code = (2 if setup_failed else 3) if interrupted else cells.exit_code(table, selected, setup_failed=setup_failed)
    report["budget_failures"] = budget_failures(report["agents"]) if not local and not fixture_bin else []
    if report["budget_failures"] and code in (0, 3):
        code = 1
    report["exit_code"] = code
    save(run, report)
    print("\n".join(table_lines(report)))
    print(f"{run / 'report.md'}\n{run / 'report.json'}\nexit={code}")
    return code
