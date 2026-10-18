"""Reach a data-defined scene, ring only when unblocked, observe effects."""

import json
from pathlib import Path
import re

from .conversation import bell_turn, messages
from .authentication import AuthenticationRequired, require_no_login
from .processes import ProcessError
from .protection import ProtectionError, beneath, write_private
from .system import same_path
from .timing import Deadline, ObservationTimeout


def matches(pattern: str, screen: str, bell: str) -> bool:
    # Match active detection rows, never a provider's growing scrollback.
    return bool(re.search(pattern.replace("{bell}", re.escape(bell)), screen,
                          flags=re.MULTILINE | re.IGNORECASE))


def arrived(data: dict, screen: str, bell: str) -> bool:
    return matches(data["arrived"], screen, bell) and (
        not data.get("controls") or matches(data["controls"], screen, bell))


def owned_launch(scene, recipe, pane, actual, *, cwd, workspace):
    return bool(actual and actual.get("pane_id") == pane and actual.get("agent") == recipe["kind"]
                and actual.get("workspace_id") == workspace and same_path(actual.get("cwd"), cwd)
                and actual.get("name") in (None, "live-" + recipe["id"] + "-" + scene))


def readiness_refusal(result):
    code, _, error = result
    try:
        refusal = json.loads(error).get("error", {}).get("code")
    except (ValueError, AttributeError):
        return False
    return bool(code and refusal in ("agent_not_ready", "timeout"))


def startup_blocker(scene, recipe, pane, result, actual, screen, *, cwd, workspace):
    # Startup readiness can time out on the very unclassified dialog this
    # tool measures. Registration can still be absent at that point. Fresh
    # owned pane/workspace/cwd/kind plus the actual menu proves arrival;
    # Herdr's blocked classification is an observation, never a prerequisite.
    return bool(readiness_refusal(result) and scene == "startup"
                and owned_launch(scene, recipe, pane, actual, cwd=cwd, workspace=workspace)
                and arrived(recipe["scenes"]["startup"], screen, ""))


def transcript(home: Path, kind: str, session: dict | None, session_root: Path | None = None) -> Path | None:
    if not session or session.get("source") != "herdr:" + kind:
        return None
    roots = {"claude": ".claude/projects", "codex": ".codex/sessions",
             "pi": ".pi/agent/sessions", "omp": ".omp/agent/sessions"}
    if kind not in roots:
        return None
    root = session_root or home / roots[kind]
    if session.get("kind") == "path":
        file = Path(session.get("value", ""))
        if not file.is_absolute() or not beneath(file, root) or file.is_symlink():
            raise ProtectionError("native_session_path_outside_owned_history")
        return file if file.is_file() else None
    if session.get("kind") != "id":
        return None
    identity = session.get("value", "")
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,256}", identity):
        raise ProtectionError("invalid_native_session_identity")
    if not root.is_dir():
        return None
    pending, count, found = [root], 0, []
    while pending:
        path = pending.pop()
        count += 1
        if count > 50_000:
            raise ProtectionError("session_location_over_budget")
        if path.is_symlink():
            continue
        if path.is_dir():
            pending.extend(path.iterdir())
        elif path.name.endswith(identity + ".jsonl"):
            found.append(path)
    if len(found) > 1:
        raise ProtectionError("ambiguous_native_session_location")
    return found[0] if found else None


def observe(runtime, pane: str, recipe: dict, scene: str, bell: str,
            home: Path, seconds: float, evidence: Path, cwd: Path, overlay: dict,
            previous_session: dict | None = None) -> dict:
    data = recipe["scenes"][scene]
    row = {"scene": scene, "arrival": "unreached", "status": "unknown",
           "effect": "not_tested", "reason": "scene_not_observed", "evidence": evidence.name}
    samples = []
    before = None
    phase = "arrival"
    try:
        if data["send"]:
            runtime.send(pane, data["send"])
        # Start after the trigger, with a fixed transport allowance alongside
        # the original observation window.
        deadline = Deadline.observation(runtime.owner, seconds)
        while not deadline.expired():
            screen = runtime.screen(pane, seconds=deadline.command_seconds(), deadline=deadline.end)
            sample = {"phase": "arrival", "screen": screen, "agent": None}
            samples = [sample]
            deadline.remaining()
            require_no_login(screen)
            agent = runtime.agent(pane, seconds=deadline.command_seconds(), deadline=deadline.end)
            sample["agent"] = agent
            deadline.remaining()
            previous_visible = (scene != "resume_picker" or runtime.fixture_bin or
                                (previous_session and any(token in screen for token in previous_session["visible_tokens"])))
            if agent and arrived(data, screen, bell) and previous_visible:
                deadline.remaining()
                before = {"screen": screen, "agent": agent}
                break
            runtime.wait(lambda: True, 0.01)
            runtime.owner.cancelled.wait(0.15)
            if runtime.owner.cancelled.is_set():
                raise ProcessError("run_cancelled")
        if before is None:
            row["arrival"] = "timeout"
            row["reason"] = "arrival_deadline"
            return row
        row.update(arrival="reached", status=before["agent"]["agent_status"])
        samples = [{"phase": "before", **before}]
        if row["status"] == "blocked":
            deadline.remaining()
            row.update(effect="guarded", reason="Herdr_blocked_no_bell_typed")
            return row
        # The deliberate unguarded input is the experiment; agent.prompt would
        # refuse the very menus this tool needs to measure.
        session = before["agent"].get("agent_session")
        file = transcript(home, recipe["kind"], session, overlay["session_root"])
        previous = messages(file, recipe["kind"]) if file else []
        from .protection import stamp
        configuration = {path: stamp(path) for path in overlay["settings"]}
        existing_probes = {path.name for path in cwd.glob("probe-*.txt")}
        runtime.send(pane, bell, deadline=deadline)
        phase = "effect"
        deadline = Deadline.observation(runtime.owner, seconds)
        effect = "ambiguous"
        reason = "no_positive_effect_evidence"
        while not deadline.expired():
            screen = runtime.screen(pane, seconds=deadline.command_seconds(), deadline=deadline.end)
            sample = {"phase": "after", "screen": screen, "agent": None}
            samples = [samples[0], sample]
            deadline.remaining()
            require_no_login(screen)
            agent = runtime.agent(pane, seconds=deadline.command_seconds(), deadline=deadline.end)
            sample["agent"] = agent
            deadline.remaining()
            if any(stamp(path) != original for path, original in configuration.items()):
                effect, reason = "settings_write", "private_settings_or_trust_changed_after_bell"
                break
            if agent and session and agent.get("agent_session") != session:
                effect, reason = "resumed_session", "native_session_identity_changed"
                break
            if scene in ("shell_approval", "file_approval", "plan_approval") and (
                    {path.name for path in cwd.glob("probe-*.txt")} - existing_probes):
                effect, reason = "approval", "probe_file_created_after_bell"
                break
            # The fixture and some native CLIs report explicit completed
            # selection/approval actions; losing the menu alone proves nothing.
            if matches(data["unsafe"], screen, bell):
                effect, reason = ("selection" if scene.endswith("picker") or scene == "startup"
                                  else "approval"), "explicit_completed_action"
                break
            deadline.remaining()
            if matches(data["no_match"], screen, bell) and scene.endswith("picker"):
                effect, reason = "no_match", "picker_reports_no_matches"
                break
            if file:
                fresh = messages(file, recipe["kind"])[len(previous):]
                deadline.remaining()
                if bell_turn(fresh, bell):
                    effect, reason = "new_turn", "native_user_bell_then_assistant_reply"
                    break
            if runtime.owner.cancelled.wait(0.15):
                raise ProcessError("run_cancelled")
        # A draft must still be present at the end of the observation window;
        # seeing the compositor briefly while input is submitted is not a draft.
        if effect == "ambiguous" and samples and matches(data["draft"], samples[-1]["screen"], bell):
            effect, reason = "unsent_draft", "bell_remains_in_composer_at_deadline"
        if effect in ("ambiguous", "no_match", "new_turn"):
            deadline.remaining()
        row.update(effect=effect, reason=reason)
        return row
    except AuthenticationRequired:
        row.update(arrival="skipped", effect="not_tested", reason="not_authenticated_no_login_attempted")
        raise
    except ProcessError as error:
        effect_read_expired = (phase == "effect" and type(error) is ProcessError
                               and str(error) == "command_timeout" and deadline.expired()
                               and deadline.end < runtime.owner.deadline)
        if runtime.owner.cancelled.is_set() or not (isinstance(error, ObservationTimeout) or effect_read_expired):
            raise
        if row["arrival"] == "reached":
            # A later expiry cannot revoke timely arrival or certify an effect.
            row.update(effect="ambiguous", reason=phase + "_deadline")
        else:
            row.update(arrival="timeout", effect="not_tested", reason=phase + "_deadline")
        return row
    finally:
        write_private(evidence, (json.dumps({"observation": row, "samples": samples}, indent=2) + "\n").encode())
