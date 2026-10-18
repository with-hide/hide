"""One local command; native and synthetic measurements are explicitly distinct."""

import argparse
import datetime
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import sys
import tempfile
import time

from .contracts import SCENES, recipes, source_contract
from .authentication import AuthenticationRequired, require_no_login
from .delivery import measure as measure_delivery
from .overlay import prepare as prepare_overlay
from .credential_snapshot import CredentialSnapshotUnavailable
from .history import LABEL as PREVIOUS_LABEL, seed as seed_history
from .integration import (prepare as prepare_integration, project_args, observe as observe_integration,
                          provenance as integration_provenance)
from .processes import COMMAND_SECONDS, OwnedProcesses, ProcessError, ProcessSafetyError
from .protection import ConfigGuard, ProtectionError, beneath, private_directory, stamp, write_private
from .report import save
from .runtime import Runtime, clean_env
from .sandbox import WriteSandbox
from .scenes import observe, readiness_refusal, startup_blocker
from .setup import configure, prepare_startup


def parser():
    value = argparse.ArgumentParser(description="Measure bell safety on private pinned Herdr and this checkout's hided.")
    value.add_argument("--agents", help="Comma-separated adapter IDs; default is all current adapters")
    value.add_argument("--model", action="append", default=[], metavar="ID=MODEL",
                       help="Account's cheapest suitable model, passed only on the command line")
    value.add_argument("--run-dir", type=Path, help="New directory below this checkout's agents/runs")
    value.add_argument("--herdr-bin", type=Path, help="Previously fetched pinned Herdr; bytes are checked")
    value.add_argument("--socket", type=Path, help="Private socket; operator/default/existing paths are refused")
    value.add_argument("--state-dir", type=Path, help="Private state directory inside the run")
    value.add_argument("--scene-seconds", type=float, default=30,
                       help=f"Observation seconds; each phase also allows {COMMAND_SECONDS}s for transport")
    value.add_argument("--fixture-bin", type=Path, help="CI only: compiled Claude/Codex shim directory; uses private HOME")
    return value


def default_herdr(checkout, home):
    pin = json.loads((checkout / "contracts/herdr-bundle.json").read_text())
    if sys.platform == "darwin":
        cache, digest = home / "Library/Caches/hide/herdr-runtime", pin["sha256"]
    elif sys.platform.startswith("linux"):
        cache, digest = home / ".cache/hide/herdr-runtime", pin["linux_x86_64"]["sha256"]
    elif sys.platform == "win32" and os.environ.get("LOCALAPPDATA"):
        # scripts/fetch-herdr-runtime.ps1's cache: the zip and what it unpacks to.
        digest = pin["windows_x86_64"]["sha256"]
        return Path(os.environ["LOCALAPPDATA"]) / "hide/herdr-runtime" / digest / "unpacked/herdr.exe"
    else:
        raise ProtectionError("live_check_host_not_supported")
    return cache / digest / "herdr"


def operator_routing(environment):
    """Inherited operator routes, before the private environment replaces them."""
    routes = []
    for key, suffix in {"HIDE_STATE_DIR": "", "HERDR_CONFIG_PATH": "",
                        "HERDR_SESSION_PATH": "", "HERDR_SOCKET_PATH": "",
                        "XDG_STATE_HOME": "hide", "XDG_CONFIG_HOME": "herdr"}.items():
        if value := environment.get(key):
            path = Path(value)
            if not path.is_absolute():
                raise ProtectionError("operator_routing_must_be_absolute")
            routes.append(path / suffix if suffix else path)
    return routes


def wrapper(runtime, recipe, executable, sandbox):
    directory = runtime.bin / recipe["id"]
    private_directory(directory)
    target = directory / recipe["executable"]
    command = [str(executable)]
    if sandbox:
        command = sandbox.command(command)
    # The driver routes the pinned installer's assets through disposable
    # configuration. This executable wrapper only enforces the OS guard.
    fixture_env = "export HIDE_E2E_LIVE_CHECK=1\n" if runtime.fixture_bin else ""
    # The pane/server can add environment after clean_env. Enforce the actual
    # Claude exec boundary without redirecting its HOME or authenticating it.
    native_env = ("unset CLAUDE_CONFIG_DIR\n"
                  "export DISABLE_AUTOUPDATER=1 DISABLE_TELEMETRY=1 DISABLE_ERROR_REPORTING=1\n"
                  if recipe["id"] == "claude-code" and not runtime.fixture_bin else "")
    write_private(target, ("#!/bin/sh\n" + fixture_env + native_env + "exec " + " ".join(shlex.quote(v) for v in command) + ' "$@"\n').encode())
    target.chmod(0o700)
    return target


def record_process_diagnostics(owner, report):
    """Receipt failures fail cleanup but preserve known resource accounting."""
    try:
        report["cleanup"]["attribution"] = owner.attribution_report()
    except Exception as error:
        # Even a bounded malformed JSON receipt can raise RecursionError.
        # Diagnostics cannot prevent independent config recovery/report save.
        report["cleanup"]["confirmed"] = False
        report["failures"].append({"type": type(error).__name__, "reason": str(error)})
    report["resources"]["rss_samples"] = owner.rss_report()


def main(argv=None):
    args = parser().parse_args(argv)
    checkout = Path(__file__).resolve().parents[2]
    operator = Path.home().resolve()
    if not 1 <= args.scene_seconds <= 120:
        raise ProtectionError("scene_seconds_out_of_bounds")
    contract = source_contract(checkout)
    available = recipes(Path(__file__).with_name("recipes"), contract["targets"])
    selected = args.agents.split(",") if args.agents else list(available)
    if not selected or len(set(selected)) != len(selected) or set(selected) - available.keys():
        raise ProtectionError("invalid_agent_selection")
    overrides = {}
    for entry in args.model:
        key, separator, model = entry.partition("=")
        if not separator or key not in selected or not model or key in overrides:
            raise ProtectionError("invalid_model_override")
        overrides[key] = model
    for key, model in overrides.items():
        available[key]["model"] = model
    if args.fixture_bin and (set(selected) - {"claude-code", "codex"}):
        raise ProtectionError("fixtures_only_support_claude_and_codex")
    slug = "live-check-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S") + "-" + os.urandom(3).hex()
    run = (args.run_dir or checkout / "agents/runs" / slug).absolute()
    if run.exists() or run.is_symlink() or not beneath(run, checkout / "agents/runs"):
        raise ProtectionError("report_directory_must_be_new_and_local_only")
    private_directory(run)
    owner = OwnedProcesses(diagnostics=run / "process-diagnostics")
    report = {"format": 1, "fixture": bool(args.fixture_bin), "herdr": {}, "agents": [],
              "source": contract["declaration_sha256"], "configuration": {},
              "timing": {"transport_seconds": COMMAND_SECONDS, "observation_seconds": args.scene_seconds,
                         "phase_seconds": COMMAND_SECONDS + args.scene_seconds},
              "cleanup": {"confirmed": False}, "failures": [], "resources": {}}
    runtime = guard = None
    previous_signals = {}
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous_signals[signum] = signal.signal(signum, lambda *_: owner.cancelled.set())
    try:
        runtime = Runtime(checkout, run, operator, owner,
                          herdr_bin=args.herdr_bin or default_herdr(checkout, operator),
                          fixture_bin=args.fixture_bin, socket=args.socket, state=args.state_dir)
        # Runtime rejects operator paths and wrong binaries before this point.
        agent_home = runtime.home if args.fixture_bin else operator
        all_recipes = [available[key] for key in selected]
        known = list(dict.fromkeys(agent_home / relative for recipe in all_recipes for relative in recipe["known"]))
        roots = list(dict.fromkeys(agent_home / relative for recipe in all_recipes for relative in recipe["roots"]))
        # A disposable fixture HOME has one writer and still proves exact
        # recovery. Live shared project files are strictly read-only observers.
        shared = {} if args.fixture_bin else {agent_home / relative: format
                  for recipe in all_recipes for relative, format in recipe.get("shared", {}).items()}
        guard = ConfigGuard(run / "configuration-backup", known, roots,
                            exclusive_root=agent_home if args.fixture_bin else None, shared=shared)
        routing = operator_routing(os.environ)
        executables = [runtime.herdr_bin, Path(sys.executable)]
        executables.extend(Path(value) for recipe in all_recipes
                           if (value := shutil.which(recipe["executable"])))
        sandbox = None if args.fixture_bin else WriteSandbox(run, runtime.short, operator, runtime.state,
                                                            checkout=checkout, routing=routing,
                                                            executables=executables,
                                                            protected=[path for path in known if path not in shared])
        runtime.sandbox = sandbox
        if sandbox:
            with tempfile.TemporaryDirectory(prefix="acl-guard-", dir="/tmp") as outside:
                sandbox.verify(owner, {**clean_env(), "TMPDIR": str(run / "agent-tmp")}, Path(outside).resolve())
        # Only the candidate CLI is reachable as `hide` in a probe workspace.
        write_private(runtime.bin / "hide", ("#!/bin/sh\nexec " + shlex.quote(str(runtime.hide)) + ' "$@"\n').encode())
        (runtime.bin / "hide").chmod(0o700)
        report["herdr"] = runtime.start()
        for recipe in all_recipes:
            provider = {"id": recipe["id"], "version": "unavailable", "model": recipe["model"],
                        "model_note": recipe["model_note"], "bell_target": contract["targets"][recipe["id"]],
                        "integration": {"status": "not_observed"}, "scenes": [], "login": recipe["login"],
                        "delivery": {"outcome": "unknown", "reason": "not_measured"}}
            report["agents"].append(provider)
            executable = ((args.fixture_bin / recipe["executable"]).resolve() if args.fixture_bin
                          else shutil.which(recipe["executable"]))
            if not executable or not Path(executable).is_file():
                provider["skipped"] = "not_installed"
            else:
                launch = wrapper(runtime, recipe, Path(executable), sandbox)
                if recipe.get("overlay") and not args.fixture_bin:
                    runtime.credential_roots.add(runtime.probe / ("config-" + recipe["id"]))
                try:
                    overlay = prepare_overlay(recipe, runtime.probe, agent_home, owner=owner,
                                              env=clean_env()) if not args.fixture_bin else {
                        "env": {}, "copies": [], "settings": [], "session_root": None}
                except CredentialSnapshotUnavailable as error:
                    reason = {"reason": "credential_snapshot_unavailable", "detail": str(error),
                              "next_action": f"Close all {recipe['executable']} instances and rerun."}
                    provider["unavailable"] = reason
                    evidence = recipe["id"] + "-credential-snapshot.json"
                    write_private(run / evidence, json.dumps(reason, indent=2).encode())
                    provider["scenes"] = [{"scene": scene, "arrival": "unreached", "status": "unknown",
                                           "effect": "not_tested", "reason": reason["reason"],
                                           "evidence": evidence} for scene in SCENES]
                    continue
                integration = prepare_integration(runtime, recipe, overlay)
                provider["private_configuration"] = {key: value for key, value in overlay.items()
                                                      if key in ("copies", "provenance")}
                if recipe["kind"] == "codex" and overlay["session_root"]:
                    private_directory(overlay["session_root"])
                    link = runtime.home / ".codex/sessions"
                    if link.is_symlink():
                        link.unlink()
                    link.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    link.symlink_to(overlay["session_root"], target_is_directory=True)
                metadata_env = {**clean_env(), **overlay["env"], "HOME": str(agent_home), "TMPDIR": str(run / "agent-tmp")}
                code, version, err = owner.run([str(launch), "--version"], env=metadata_env, check=False)
                provider["version"] = version.strip()[:256]
                if code:
                    provider["skipped"] = "version_probe_failed"
                    write_private(run / (recipe["id"] + "-version-error.txt"), err.encode())
                else:
                    previous_session = None
                    for scene in SCENES:
                        workspace = None
                        evidence = run / (recipe["id"] + "-" + scene + ".json")
                        try:
                            workspace, pane, cwd = runtime.new_workspace(recipe, scene, agent_home, launch, overlay)
                            history = cwd / "history"
                            private_directory(history)
                            scene_overlay = {**overlay, "settings": list(overlay["settings"])}
                            if recipe["kind"] in ("pi", "omp"):
                                scene_overlay["session_root"] = history
                            extra = configure(runtime, launch, recipe, scene, cwd) + project_args(integration, recipe, cwd)
                            for relative in (recipe["mcp"].get("path"), ".pi/settings.json", ".cursor/hooks.json"):
                                if relative and (cwd / relative).is_file():
                                    scene_overlay["settings"].append(cwd / relative)
                            if scene == "rest" and recipe["kind"] in ("pi", "claude"):
                                extra.extend(["--name", PREVIOUS_LABEL])
                            values = dict(model=recipe["model"], sockets=str(runtime.short),
                                          history=str(history))
                            native_args = [value.format_map(values) for value in recipe["argv"]] + extra
                            code, output, err = runtime.command(["agent", "start", "live-" + recipe["id"] + "-" + scene,
                                             "--kind", recipe["kind"], "--pane", pane, "--timeout", "5000",
                                             "--", *native_args], check=False, seconds=10)
                            if code:
                                write_private(run / (recipe["id"] + "-" + scene + "-start-error.json"),
                                              json.dumps({"exit_code": code, "stdout": output, "stderr": err}).encode())
                            prepared = False
                            if not code or readiness_refusal((code, output, err)):
                                prepared = prepare_startup(runtime, pane, recipe, scene, cwd, workspace,
                                                           args.scene_seconds,
                                                           run / (recipe["id"] + "-" + scene + "-startup-preparation.json"))
                            if code and not prepared:
                                actual = runtime.agent(pane)
                                refused_screen = runtime.screen(pane)
                                require_no_login(refused_screen)
                                if not startup_blocker(scene, recipe, pane, (code, output, err), actual, refused_screen,
                                                       cwd=cwd, workspace=workspace):
                                    write_private(evidence, json.dumps({"reason": "agent_start_refused_" + str(code),
                                        "samples": [{"phase": "startup_refusal", "screen": refused_screen,
                                                     "agent": actual}]}).encode())
                                    raise ProcessError("agent_start_refused_" + str(code))
                            startup_screen = runtime.screen(pane)
                            require_no_login(startup_screen)
                            if scene == "rest":
                                previous_session = seed_history(runtime, pane, recipe, agent_home, scene_overlay,
                                                                args.scene_seconds)
                                provider["previous_session"] = previous_session
                            provider["scenes"].append(observe(runtime, pane, recipe, scene, contract["bell"],
                                                               agent_home, args.scene_seconds, evidence, cwd,
                                                               scene_overlay, previous_session))
                            if scene == "rest" and provider["scenes"][-1]["arrival"] == "reached":
                                provider["delivery"] = measure_delivery(runtime, pane, recipe, agent_home,
                                                                        cwd, args.scene_seconds,
                                                                        contract["quiet_seconds"], scene_overlay)
                        except AuthenticationRequired:
                            provider["skipped"] = "not_authenticated"
                            if not evidence.exists():
                                write_private(evidence, json.dumps({"screen": runtime.screen(pane),
                                              "reason": "not_authenticated_no_login_attempted"}).encode())
                            break
                        except ProcessError as error:
                            if owner.cancelled.is_set() or isinstance(error, ProcessSafetyError):
                                raise
                            provider["scenes"].append({"scene": scene, "arrival": "timeout", "status": "unknown",
                                                       "effect": "not_tested", "reason": str(error), "evidence": evidence.name})
                            if not evidence.exists():
                                write_private(evidence, json.dumps({"reason": str(error)}).encode())
                        finally:
                            if workspace:
                                primary = sys.exc_info()[1]
                                finalization_errors = []
                                try:
                                    current_integration = observe_integration(runtime, pane, recipe, integration)
                                    if (current_integration["integrity"] == "unproven" or
                                            current_integration["native_source"] or
                                            not provider["integration"].get("native_source")):
                                        provider["integration"] = current_integration
                                except Exception as error:
                                    if integration.get("integrity_changes"):
                                        provider["integration"] = integration_provenance(runtime.probe, recipe, integration)
                                    finalization_errors.append(("scene_integration", error))
                                try:
                                    runtime.close_workspace(workspace)
                                except Exception as error:
                                    finalization_errors.append(("scene_workspace_close", error))
                                # Keep a pending scene exception primary. With
                                # no pending exception, the first failed close
                                # becomes primary and still aborts measurement.
                                secondary = finalization_errors if primary else finalization_errors[1:]
                                for phase, error in secondary:
                                    failure = {"type": type(error).__name__, "reason": str(error),
                                               "phase": phase, "agent": recipe["id"], "scene": scene}
                                    if isinstance(error, ProtectionError) and error.path is not None:
                                        failure["path"] = error.path
                                    report["failures"].append(failure)
                                if primary is None and finalization_errors:
                                    raise finalization_errors[0][1]
            if provider.get("skipped"):
                observed = {row["scene"] for row in provider["scenes"]}
                provider["scenes"].extend({"scene": scene, "arrival": "skipped", "status": "unknown",
                                         "effect": "not_tested", "reason": provider["skipped"]}
                                        for scene in SCENES if scene not in observed)
            report["resources"] = owner.usage()
    except Exception as error:
        report["failures"].insert(0, {"type": type(error).__name__, "reason": str(error)})
        if isinstance(error, ProtectionError) and error.path is not None:
            report["failures"][0]["path"] = error.path
    finally:
        try:
            if runtime:
                report["cleanup"] = runtime.close()
            else:
                owner.close()
                report["cleanup"] = {"confirmed": True, "probe_removed": True}
        except Exception as error:
            report["cleanup"] = {"confirmed": False, "failures": [str(error)]}
            report["failures"].append({"type": type(error).__name__, "reason": str(error)})
            # A protocol/filesystem close failure must not prevent the
            # process owner's independent final attempt or config recovery.
            try:
                owner.close()
            except Exception as cleanup_error:
                report["cleanup"]["failures"].append(str(cleanup_error))
        record_process_diagnostics(owner, report)
        if guard:
            attribution_failures = []
            try:
                if args.fixture_bin:
                    # Only a sole-owned disposable HOME may claim all its
                    # writers. Native runs never infer operator ownership.
                    for path, before in guard.before.items():
                        try:
                            current = stamp(path)
                            if current != before:
                                guard.record_write(path, before, current)
                        except (OSError, ProtectionError) as error:
                            attribution_failures.append({"path": str(path), "reason": str(error)})
                report["configuration"] = guard.finish()
            except Exception as error:
                report["configuration"] = {"failures": [{"reason": str(error)}]}
            report["configuration"]["failures"].extend(attribution_failures)
        for signum, handler in previous_signals.items():
            signal.signal(signum, handler)
    code = save(run, report)
    print(f"{run / 'report.md'}\n{run / 'report.json'}\nexit={code}")
    return code
