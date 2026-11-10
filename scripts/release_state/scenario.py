"""The release-state scenario, run inside the core container as its account.

It drives a released core the way the operator and agents do: through the
shell's /ws (`agent_journey.daemon`) and through the release's own `hide` CLI
typed into stand-in agents (`agent.py`), on this machine and on the dialed
device `mini`. Each step stores what a later build has to rename when the
device takes its machine id, or what it reads back about an agent: its
launch, sleep and archive records, the project settings, and the device
kit's record and hooks. A step the release refuses is recorded with its
answer and the run goes on; a step the rest depends on stops the run.

It writes what each step did to the report path it is given (JSON).
"""

import base64
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import uuid

sys.path.insert(0, "/scripts")
from agent_journey.daemon import Daemon  # noqa: E402
from agent_live_check.processes import ProcessError  # noqa: E402

HIDE = "/opt/hide/hide"
CORE_ID = "c0de0000000000000000000000000001"
# Claude Code's adapter id in the kit, which its switch names.
KIT_CLAUDE = "claude-code"
HOME = Path.home()
STATE = HOME / ".hide/state"
# The ssh alias the core dials. A release before device naming registers the
# device under it; a later one under the machine id its node reports.
ALIAS = "mini"
CORE_REPO = HOME / "core-proj"
DEVICE_REPO = "/home/remote/dev-proj"
ORIGIN = "https://github.com/example/release-state.git"
WAIT = 60
COMMAND = 90


class Stop(Exception):
    """A step the rest of the scenario needs did not happen."""


class NotInRelease(Exception):
    """The release itself refuses the step: it stores nothing to cover."""


class Scenario:
    def __init__(self, report):
        self.report_path = Path(report)
        self.steps = []
        self.daemon = Daemon(STATE, threading.Event())
        self.request = 0
        self.agents = {}
        self.device = None

    # --- the shell's side -------------------------------------------------

    def send(self, kind, payload):
        self.daemon.dispatch(kind, payload)

    def rid(self, what):
        self.request += 1
        return f"release-state-{what}-{self.request}"

    def wait(self, predicate, what, seconds=WAIT):
        try:
            return self.daemon.wait(lambda rest: predicate(rest), seconds, what)
        except ProcessError as error:
            raise Stop(str(error)) from error

    def device_session(self, rest):
        row = next((row for row in (rest.get("status") or {}).get("remote") or [] if row.get("target_id") == self.device), None)
        return (row or {}).get("session") or {}

    def checkouts(self, rest, device):
        workspaces = (self.device_session(rest) if device else rest.get("navigator") or {}).get("workspaces") or []
        return [(workspace, checkout) for workspace in workspaces for checkout in workspace.get("checkouts") or []]

    def checkout(self, rest, device, path):
        return next(((w, c) for w, c in self.checkouts(rest, device) if c.get("path") == path), None)

    def tabs(self, rest, device, path):
        found = self.checkout(rest, device, path)
        return found[1].get("tabs") or [] if found else []

    def place(self, pane):
        """(device, workspace, checkout, tab) of a pane shown anywhere."""
        rest = self.daemon.snapshot() or {}
        for device in (False, True):
            for workspace, checkout in self.checkouts(rest, device):
                for tab in checkout.get("tabs") or []:
                    if any(row.get("id") == pane for row in tab.get("panes") or []):
                        return device, workspace, checkout, tab
        raise Stop(f"pane {pane} is in no tab")

    def agent_rows(self, rest):
        return ((rest.get("navigator") or {}).get("agents") or []) + (self.device_session(rest).get("agents") or [])

    def show(self, pane):
        """Brings the pane's tab to the screen: the shell types only into a shown tab."""
        device, workspace, checkout, tab = self.place(pane)
        if device:
            self.send("remote_control", {"target_id": self.device, "request_id": self.rid("focus"), "action": "focus_workspace",
                                         "workspace_id": workspace["id"], "checkout_id": checkout["id"], "focus_device": True})
            self.wait(lambda rest: (rest.get("workspace_view") or {}).get("path") == checkout["path"], "device workspace in front")
            self.send("remote_control", {"target_id": self.device, "request_id": self.rid("tab"), "action": "focus_tab",
                                         "tab_id": tab["id"]})
        else:
            self.send("focus_tab", {"workspace_id": workspace["id"], "checkout_id": checkout["id"], "tab_id": tab["id"],
                                    "focus_device": True})

        def shown(rest):
            view = rest.get("workspace_view") or {}
            if view.get("path") != checkout["path"]:
                return False
            return tab["id"] in json.dumps((view.get("agent_layout") or {}).get("root"))
        self.wait(shown, f"tab of {pane} in front")
        time.sleep(1)

    def type(self, pane, text):
        self.show(pane)
        data = (text + "\r").encode()
        self.send("key", {"pane_id": pane, "bytes_base64": base64.b64encode(data).decode()})

    # --- an agent's side --------------------------------------------------

    def start_agent(self, pane, name, host_scope, raw_pane):
        session = str(uuid.uuid4())
        self.type(pane, f"claude --session-id {session}")
        self.wait(lambda rest: any(row.get("session_id") == session for row in self.agent_rows(rest)),
                  f"{name}'s session on its row")
        device = pane.startswith("remote:")
        self.agents[name] = {"pane": pane, "session": session, "device": device, "seen": 0}
        answer = self.run(name, f"hide agent register --host-scope {host_scope} --session {session} "
                                f"--instance {raw_pane} --name {name} --pane {raw_pane} --json")
        if not (answer.get("json") or {}).get("ok"):
            raise Stop(f"{name} did not register: {answer}")
        self.agents[name]["id"] = answer["json"]["value"]["id"]
        return self.agents[name]

    def results(self, agent):
        path = f"/tmp/release-state-agents/{agent['session']}.jsonl"
        if agent["device"]:
            done = subprocess.run(["ssh", ALIAS, f"cat {path} 2>/dev/null"], text=True, capture_output=True, timeout=30)
            text = done.stdout
        else:
            text = Path(path).read_text() if Path(path).exists() else ""
        return [json.loads(line) for line in text.splitlines() if line.strip()]

    def run(self, name, command):
        """Runs `command` in the agent's pane as the agent; its result."""
        agent = self.agents[name]
        self.type(agent["pane"], "!" + command)
        end = time.monotonic() + COMMAND
        while time.monotonic() < end:
            rows = self.results(agent)
            if len(rows) > agent["seen"]:
                agent["seen"] = len(rows)
                result = rows[-1]
                for line in reversed(result["out"].splitlines()):
                    try:
                        result["json"] = json.loads(line)
                        break
                    except ValueError:
                        continue
                return result
            time.sleep(1)
        raise Stop(f"{name} did not finish `{command}`")

    # --- the steps --------------------------------------------------------

    def step(self, name, function, required=False):
        started = time.time()
        try:
            detail = function()
            self.steps.append({"step": name, "ok": True, "detail": detail})
        except NotInRelease as error:
            self.steps.append({"step": name, "ok": None, "detail": f"not in this release: {error}"})
        except Exception as error:  # noqa: BLE001 - a step's failure is the report's content
            self.steps.append({"step": name, "ok": False, "detail": f"{type(error).__name__}: {error}"})
            if required:
                raise
        finally:
            self.steps[-1]["seconds"] = round(time.time() - started, 1)
            self.save()

    def save(self):
        self.report_path.write_text(json.dumps({"steps": self.steps, "agents": self.agents}, indent=1))

    def spawn(self, name, command):
        """A spawn, run again while it answers that it may still be running."""
        for _ in range(8):
            result = self.run(name, command)
            answer = result.get("json") or {}
            if answer.get("ok") is True:
                return answer
            if (answer.get("error") or {}).get("code") not in ("request_timeout", "native_identity_unavailable", "spawn_busy"):
                break
            time.sleep(5)
        raise Stop(f"`{command}` answered rc={result['rc']} {result['out'][-1500:]} {result['err'][-400:]}")

    def checked(self, name, command, accept=lambda answer: answer.get("ok") is True):
        result = self.run(name, command)
        answer = result.get("json") or {}
        if result["rc"] != 0 or not accept(answer):
            raise Stop(f"`{command}` answered rc={result['rc']} {result['out'][-1500:]} {result['err'][-400:]}")
        return answer

    def projects(self):
        git = ["git", "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid"]
        script = (f"set -e; mkdir -p {{0}} && cd {{0}} && git init -q -b main && git remote add origin {ORIGIN}"
                  " && printf 'release state\\n' > README.md && printf '# notes\\n' > notes.md && git add ."
                  f" && {' '.join(git)} commit -q -m init")
        subprocess.run(["bash", "-c", script.format(CORE_REPO)], check=True)
        subprocess.run(["ssh", ALIAS, script.format(DEVICE_REPO)], check=True)
        self.send("create_workspace", {"path": str(CORE_REPO), "label": "core-proj", "initialize_git": False})
        self.wait(lambda rest: self.tabs(rest, False, str(CORE_REPO)), "core project with its first tab")
        self.send("register_device", {"id": ALIAS, "label": ALIAS, "ssh_alias": ALIAS, "host_consent": True})
        self.device = self.wait(lambda rest: next((row["target_id"] for row in (rest.get("status") or {}).get("remote") or []
                                                   if row.get("state") == "connected" and row.get("session")), None),
                                "device connected", 180)
        self.send("create_workspace", {"device_id": self.device, "path": DEVICE_REPO, "label": "dev-proj", "initialize_git": False})
        self.wait(lambda rest: self.tabs(rest, True, DEVICE_REPO), "device project with its first tab")
        return "core-proj on the core, dev-proj on mini"

    def first_pane(self, device, path):
        return self.tabs(self.daemon.snapshot(), device, path)[0]["panes"][0]["id"]

    def new_device_tab(self):
        rest = self.daemon.snapshot()
        workspace, checkout = self.checkout(rest, True, DEVICE_REPO)
        before = {pane["id"] for tab in checkout["tabs"] for pane in tab["panes"]}
        self.send("remote_control", {"target_id": self.device, "request_id": self.rid("create-tab"), "action": "create_tab",
                                     "workspace_id": workspace["id"], "checkout_id": checkout["id"], "cwd": DEVICE_REPO,
                                     "label": checkout.get("next_tab_label") or "Tab"})
        return self.new_pane(before)

    def split(self, pane):
        before = {row["id"] for tab in self.tabs(self.daemon.snapshot(), True, DEVICE_REPO) for row in tab["panes"]}
        self.send("remote_control", {"target_id": self.device, "request_id": self.rid("split"), "action": "split_pane",
                                     "pane_id": pane, "direction": "right", "cwd": None})
        return self.new_pane(before)

    def new_pane(self, before):
        def created(rest):
            now = [row["id"] for tab in self.tabs(rest, True, DEVICE_REPO) for row in tab["panes"]]
            return next((pane for pane in now if pane not in before), None)
        pane = self.wait(created, "a new device pane")
        time.sleep(2)
        return pane

    def kit_agent(self, rest):
        """Claude Code's row in the device's kit report, once it has one."""
        device = next((row for row in (rest.get("navigator") or {}).get("devices") or [] if row.get("id") == self.device), None)
        return next((row for row in ((device or {}).get("kit") or {}).get("agents") or [] if row.get("id") == KIT_CLAUDE), None)

    def claude_on(self):
        """Claude Code switched on for the device, as its row's switch does,
        once the device's kit has reported: a switch sent before the report
        changes nothing, and a later release starts no agent there while it
        is off. It leaves the device's kit record with the choice and
        Claude's settings.json with Hide's hooks beside the operator's.
        The core runs outside a desktop package, whose kit installs nothing
        (`kit.unavailable`, standalone), so only the device has a kit."""
        row = self.wait(self.kit_agent, "the device kit's Claude Code row", 120)
        if row.get("availability") != "available":
            raise Stop(f"Claude Code is {row.get('availability')} on the device")
        if not row.get("enabled"):
            self.send("kit_agent_set", {"device_id": self.device, "agent": KIT_CLAUDE, "enabled": True})
            row = self.wait(lambda rest: (lambda now: now if now and now.get("enabled") and (now.get("hook") or {}).get("state")
                                          == "installed" else None)(self.kit_agent(rest)), "Claude Code on with its hook", 120)
        return {"enabled": row.get("enabled"), "hook": (row.get("hook") or {}).get("state"),
                "skill": (row.get("skill") or {}).get("state")}

    def agents_up(self):
        core = self.first_pane(False, str(CORE_REPO))
        self.start_agent(core, "core-a", str(STATE / "engine"), core)
        device_a = self.first_pane(True, DEVICE_REPO)
        self.start_agent(device_a, "dev-a", self.device, raw(device_a))
        device_b = self.new_device_tab()
        self.start_agent(device_b, "dev-b", self.device, raw(device_b))
        return {name: {k: agent[k] for k in ("id", "pane")} for name, agent in self.agents.items()}

    def views(self):
        self.checked("dev-a", "hide file open notes.md", lambda answer: answer.get("ok") is True)
        self.checked("dev-b", "hide file open notes.md", lambda answer: answer.get("ok") is True)
        self.checked("core-a", "hide file open README.md", lambda answer: answer.get("ok") is True)
        return "notes.md on mini opened by dev-a and dev-b; README.md on the core by core-a"

    def pane_move(self):
        shell = self.split(self.agents["dev-a"]["pane"])
        _, _, _, source = self.place(shell)
        _, _, _, destination = self.place(self.agents["dev-b"]["pane"])
        self.show(shell)
        request = self.rid("move")
        self.send("pane_move", {"request_id": request, "workspace": {"device_id": self.device, "checkout_path": DEVICE_REPO},
                                "group_id": None,
                                "source": {"pane_id": shell, "tab_id": source["id"], "area_id": "a1"},
                                "destination": {"type": "tab", "pane_id": self.agents["dev-b"]["pane"],
                                                "tab_id": destination["id"], "area_id": "a1"}})
        operation = self.wait(lambda rest: next((op for op in (rest.get("status") or {}).get("async_operations") or []
                                                 if op.get("id") == request and op.get("phase") in ("completed", "failed")),
                                                None), "the pane move")
        if operation["phase"] != "completed":
            raise Stop(f"the move failed: {operation.get('message')}")
        return f"{shell} moved into dev-b's tab"

    def letters(self):
        self.checked("dev-a", "hide request send core-a --intent release-state-a-to-core --kind progress "
                              "--body 'dev-a reached the core'")
        self.checked("core-a", "hide request send dev-a --intent release-state-core-to-a --kind request "
                               "--body 'core-a asks dev-a'")
        return "dev-a -> core-a (progress), core-a -> dev-a (request)"

    def memory(self):
        rest = self.daemon.snapshot()
        for device, path in ((True, DEVICE_REPO), (False, str(CORE_REPO))):
            workspace, _ = self.checkout(rest, device, path)
            # One Memory change runs at a time; another is refused as busy.
            self.send("memory_action", {"workspace_id": workspace["id"], "action": "enable", "number": None})
            time.sleep(8)
        device = self.checked("dev-a", "hide memory add 'The release-state project keeps its notes in notes.md'",
                              lambda answer: True)
        core = self.checked("core-a", "hide memory add 'The core project keeps its readme short'", lambda answer: True)
        return {"device": device, "core": core}

    def plan_spawn(self):
        plan = {"nodes": [{"key": "review", "title": "리뷰", "depends_on": [], "status": "planned", "delegate": True}]}
        self.checked("dev-a", f"hide task set --json '{json.dumps(plan)}'")
        answer = self.spawn("dev-a", "hide agent spawn --parent here --plan-node review --name dev-plan-child "
                                       f"--kind claude --repo {DEVICE_REPO} --branch plan-child "
                                       "--brief 'Review notes.md and report'")
        return answer

    def mission(self):
        """A mission conducted from the device, with its worker there: a
        worker on another machine than the mission's needs the project's
        GitHub repository read, which no run without GitHub can give."""
        created = self.checked("dev-a", "hide mission create --title 'Release state'")
        mission = (created.get("result") or {}).get("mission", {}).get("id") or (created.get("result") or {}).get("id")
        plan = {"scope": "pr", "strength": "fast",
                "nodes": [{"key": "look", "title": "notes check", "kind": "research",
                           "criteria": ["notes.md is named with what it holds"],
                           "steps": ["Read notes.md", "Write what it holds", "Report it"],
                           "worker": {"agent": "claude", "model": "sonnet"}}]}
        self.checked("dev-a", f"hide mission plan --json '{json.dumps(plan)}'")
        self.checked("dev-a", "hide mission start")
        spawned = self.spawn("dev-a", f"hide agent spawn --parent here --intent {mission}:look --name dev-worker "
                                      f"--kind claude --repo {DEVICE_REPO} --branch main --path {DEVICE_REPO} "
                                      "--brief 'Read notes.md and report'")
        worker = (spawned.get("value") or {}).get("id")
        self.checked("dev-a", f"hide mission node look --status working --agent-id {worker}")
        return {"mission": mission, "worker": worker}

    def decision_view(self):
        """The device mission's decision naming notes.md, opened by dev-a on
        the device, then closed: the question's link keeps the View with the
        device agent that had it open (`kept.opener`: its machine, terminal
        and session), which a later build stamps the View it opens again
        with. A release before decisions named Views refuses `--show`."""
        result = self.run("dev-a", "hide mission decide --line 'Is notes.md ready?' --option ready --option 'not yet' "
                                   "--show notes.md")
        answer = result.get("json") or {}
        if answer.get("ok") is not True:
            code = (answer.get("error") or {}).get("code")
            if result["rc"] != 0 and (code in (None, "invalid_request", "usage") or "--show" in result["err"]):
                raise NotInRelease(f"rc={result['rc']} {result['err'][-300:]}")
            raise Stop(f"the decision was refused: {result['out'][-800:]} {result['err'][-300:]}")
        rows = self.wait(lambda rest: next((mission.get("decision_views") for mission in
                                            (rest.get("missions") or {}).get("items") or []
                                            if mission.get("decision_views")), None), "the decision's View")
        view = rows[0]["display_id"]
        self.checked("dev-a", f"hide view close {view}")
        # A release before links kept their View drops the row once it closes.
        def closed(rest):
            rows = [row for mission in (rest.get("missions") or {}).get("items") or []
                    for row in mission.get("decision_views") or [] if row.get("display_id") == view or row.get("closed")]
            return (not rows or any(row.get("closed") for row in rows)) and ("kept" if rows else "dropped")
        kept = self.wait(closed, "the decision's View closed")
        return f"{view} linked to the device mission's decision and closed ({kept})"

    def sleep(self):
        pane, session = self.agents["dev-b"]["pane"], self.agents["dev-b"]["session"]
        self.send("agent_sleep", {"pane_id": pane})

        def answered(rest):
            error = (rest.get("status") or {}).get("last_error") or {}
            if error.get("kind") == "agent_sleep.refused":
                return error.get("message") or "refused"
            return all(row.get("session_id") != session for row in self.agent_rows(rest)) and "ended"
        answer = self.wait(answered, "the sleep's answer", 120)
        if answer != "ended":
            if "another device" in answer:
                raise NotInRelease(answer)
            raise Stop(answer)
        listed = self.checked("dev-a", "hide agent list --json")
        runtime = next((row.get("runtime") for row in _records(listed) if row.get("name") == "dev-b"), None)
        if runtime != "asleep":
            raise Stop(f"dev-b ended but is listed as {runtime}")
        return f"{pane} asleep"

    def dormant(self):
        """Pi on the core put to sleep, which closes its pane and keeps the
        session (`agent_sleep.dormant`): the record a later build adds the
        agent's profile to. Only the core's own machine sleeps an agent that
        way, and only once the release's own reader has proven the session
        from its file, which the stand-in Pi writes as Pi does (`agent.py`)."""
        rest = self.daemon.snapshot()
        workspace, checkout = self.checkout(rest, False, str(CORE_REPO))
        before = {row["id"] for tab in checkout["tabs"] for row in tab["panes"]}
        self.send("create_tab", {"workspace_id": workspace["id"], "checkout_id": checkout["id"],
                                 "label": checkout.get("next_tab_label") or "Tab", "request_id": self.rid("tab")})
        pane = self.wait(lambda rest: next((row["id"] for tab in self.tabs(rest, False, str(CORE_REPO))
                                            for row in tab["panes"] if row["id"] not in before), None), "a new core pane")
        time.sleep(2)
        sessions = HOME / ".pi/agent/sessions" / ("--" + str(CORE_REPO).lstrip("/").replace("/", "-") + "--")
        self.type(pane, "pi")
        native = self.wait(lambda rest: next((path.name.split("_", 1)[1].removesuffix(".jsonl")
                                              for path in sessions.glob("*.jsonl")), None), "Pi's session file")
        self.agents["core-pi"] = {"pane": pane, "session": native, "device": False, "seen": 0}
        self.type(pane, "Say hello")

        def at_rest(rest):
            row = next((row for row in self.agent_rows(rest) if row.get("pane_id") == pane), None)
            return row if row and row.get("session_id") == native and row.get("activity") == "stopped" else None
        row = self.wait(at_rest, "Pi proven by its session and at rest", 120)
        if row.get("agent_kind") != "pi":
            raise Stop(f"the pane's agent reads as {row.get('agent_kind')}")
        state = STATE / "core-state.json"
        last = None
        for _ in range(6):
            seen = ((self.daemon.snapshot() or {}).get("status") or {}).get("last_error") or {}
            self.send("agent_sleep", {"pane_id": pane})

            def answered(rest):
                error = (rest.get("status") or {}).get("last_error") or {}
                if error != seen and str(error.get("kind")).startswith("agent_sleep."):
                    return error
                stored = json.loads(state.read_text()) if state.exists() else {}
                return next(("sleeping" for record in ((stored.get("agent_sleep") or {}).get("dormant") or {}).values()
                             if record.get("native_session_id") == native and record.get("phase") == "sleeping"
                             and record.get("closed")), None)
            answer = self.wait(answered, "the dormant sleep's answer", 120)
            if answer == "sleeping":
                return f"{pane} closed, Pi session {native} sleeping"
            last = answer
            if not answer.get("retryable", True):
                break
            time.sleep(5)
        raise Stop(f"the dormant sleep was refused: {last}")

    def remains(self):
        workspace, checkout = self.checkout(self.daemon.snapshot(), True, DEVICE_REPO)
        self.show(self.agents["dev-a"]["pane"])
        self.send("detail_open", {"item": {"kind": "remains", "workspace_id": workspace["id"], "checkout_id": checkout["id"]}})
        self.wait(lambda rest: "remains" in json.dumps(((rest.get("ui_state") or {}).get("detail") or {}).get("items")),
                  "the Remains item in the slot")
        return checkout["id"]

    def project_settings(self):
        """Project settings saved for a core and a device project, as the
        Project settings sheet saves them: the record a project default
        profile is added to. The base branch is set on the core only: a
        device's is refused until its branch list has been read."""
        for device, root, name, patch in (
                (CORE_ID, str(CORE_REPO), "Core project", {"base_branch": {"mode": "branch", "branch": "main"}}),
                (self.device, DEVICE_REPO, "Device project", {})):
            seen = ((self.daemon.snapshot() or {}).get("status") or {}).get("last_error") or {}
            self.send("project_settings_set", {
                "device_id": device, "root": root, "request_id": self.rid("settings"),
                "patch": {"name": name, "new_work": "worktree", "auto_fetch": False, **patch}})

            def saved(rest):
                error = (rest.get("status") or {}).get("last_error") or {}
                if error != seen and "project_settings" in str(error.get("kind")):
                    raise Stop(f"{name} was refused: {error}")
                return any(workspace.get("path") == root and name in json.dumps(workspace) for workspace in self.workspaces(rest))
            self.wait(saved, f"{name} saved")
        return "core-proj and dev-proj named, new work in a worktree, no background fetch; core-proj based on main"

    def workspaces(self, rest):
        return ((rest.get("navigator") or {}).get("workspaces") or []) + (self.device_session(rest).get("workspaces") or [])

    def start_from_panel(self, name, device):
        """One agent started from the start panel with a model, on the core
        or the device; its launch choice is stored (`pane_launches`)."""
        path = DEVICE_REPO if device else str(CORE_REPO)
        before = {row["id"] for tab in self.tabs(self.daemon.snapshot(), device, path) for row in tab["panes"]}
        self.send("agent_start_in_checkout", {"checkout_path": path, "provider": "claude", "model": "sonnet",
                                              "request_id": self.rid("start"), **({"device_id": self.device} if device else {})})

        def started(rest):
            panes = {row["id"] for tab in self.tabs(rest, device, path) for row in tab["panes"]} - before
            return next((row for row in self.agent_rows(rest) if row.get("pane_id") in panes and row.get("session_id")), None)
        row = self.wait(started, f"{name}'s session on its row", 120)
        self.agents[name] = {"pane": row["pane_id"], "session": row["session_id"], "device": device, "seen": 0}
        return row["pane_id"]

    def old_home(self):
        """A Home started on the core and on the device, as the releases that
        kept one did: its registration (flagged `home`) is stored with each
        machine's id, which a later build reads once as a pending retirement
        and moves with the device. A release without a Home refuses the start.
        One start at a time: the core runs one task operation at once."""
        ids = []
        for device in (False, True):
            known = {row.get("id") for row in self.home_rows(self.daemon.snapshot())}
            rows = None
            # The core runs one task operation at once, so a start sent while
            # the one before is still opening its pane is refused as busy; it
            # is sent again until it is taken.
            for _ in range(12):
                seen = ((self.daemon.snapshot() or {}).get("status") or {}).get("last_error") or {}
                self.send("agent_start_in_checkout", {"home": True, "provider": "claude", "request_id": self.rid("home"),
                                                      **({"device_id": self.device} if device else {})})

                def registered(rest):
                    fresh = [row for row in self.home_rows(rest) if row.get("id") not in known]
                    error = (rest.get("status") or {}).get("last_error") or {}
                    if fresh:
                        return fresh
                    if error == seen:
                        return None
                    if "busy" in str(error.get("kind")):
                        return "busy"
                    if "home" in str(error.get("kind")) or "invalid" in str(error.get("kind")):
                        return error
                    return None
                rows = self.wait(registered, "the Home registration", 30)
                if rows != "busy":
                    break
                time.sleep(5)
            if rows == "busy" or rows is None:
                raise Stop("the Home start stayed busy")
            if isinstance(rows, dict):
                raise NotInRelease(rows)
            ids.extend(row.get("id") for row in rows)
        return ids

    @staticmethod
    def home_rows(rest):
        return [row for row in ((rest or {}).get("ui_state") or {}).get("workspace_registrations") or [] if row.get("home")]

    def start_panel(self):
        """Agents started from the start panel on the core and the device,
        left running: the launch record a later build adds the profile to."""
        return {"core": self.start_from_panel("core-started", False),
                "device": self.start_from_panel("dev-started", True)}

    def archive(self):
        """The start-panel agent's session archived (세션 보관): the
        archive record a later build resumes with the agent's profile. An
        archive is refused until Hide knows the agent's conversation, which
        its first turn gives it."""
        name = "core-archived"
        self.start_from_panel(name, False)
        pane, session = self.agents[name]["pane"], self.agents[name]["session"]
        self.type(pane, "Say hello")
        last = None
        for _ in range(12):
            time.sleep(5)
            seen = ((self.daemon.snapshot() or {}).get("status") or {}).get("last_error") or {}
            self.send("archive_session", {"pane_id": pane, "confirmed": True})

            def answered(rest):
                if session in json.dumps((rest.get("navigator") or {}).get("archived_sessions") or []):
                    return "archived"
                error = (rest.get("status") or {}).get("last_error") or {}
                if error != seen and "archive" in str(error.get("kind")):
                    return error
                return None
            answer = self.wait(answered, "the archive's answer", 60)
            if answer == "archived":
                return f"{pane} archived"
            last = answer
            if answer.get("kind") != "archive.session_unknown":
                break
        raise Stop(f"the archive was refused: {last}")

    def main(self):
        try:
            self.step("projects and device", self.projects, required=True)
            self.step("Claude Code on for the device", self.claude_on)
            self.step("project settings for a core and a device project", self.project_settings)
            self.step("agents", self.agents_up, required=True)
            self.step("views opened from two device sessions and the core", self.views)
            self.step("device pane move", self.pane_move)
            self.step("letters to and from a device agent", self.letters)
            self.step("project memory cards", self.memory)
            self.step("plan step spawned from a device session", self.plan_spawn)
            self.step("mission conducted on the device with its worker there", self.mission)
            self.step("decision View opened by a device agent and closed", self.decision_view)
            self.step("remains item in the detail slot", self.remains)
            self.step("sleeping device agent", self.sleep)
            self.step("Pi on the core asleep with its pane closed", self.dormant)
            self.step("agent started from the start panel", self.start_panel)
            self.step("archived session", self.archive)
            self.step("a Home started on the core and the device", self.old_home)
            # Let the core write what it holds in memory before it stops.
            time.sleep(5)
        except Stop:
            pass
        finally:
            self.daemon.close()
            self.save()
        needed = [step for step in self.steps if step["step"] in ("projects and device", "agents")]
        return 0 if len(needed) == 2 and all(step["ok"] for step in needed) else 1


def _records(value):
    """Every object inside a JSON answer, however it nests them."""
    if isinstance(value, dict):
        yield value
        for item in value.values():
            yield from _records(item)
    elif isinstance(value, list):
        for item in value:
            yield from _records(item)


def raw(pane):
    """A device pane's id as its own engine names it."""
    return pane.split(":pane:", 1)[1]


if __name__ == "__main__":
    sys.exit(Scenario(sys.argv[1]).main())
