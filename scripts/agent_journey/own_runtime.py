"""The journey's native panes, through the same daemon events as its screen.

The controller is bootstrapped in a newly created private shell. After that,
reads use the shipped pane CLI and session references use Hide's registration
ledger. Holder records identify cleanup targets, never terminal contents.
"""

import base64
import json
from pathlib import Path
import shlex

from agent_live_check.authentication import require_no_login
from agent_live_check.processes import ProcessError
from agent_live_check.protection import ProtectionError, write_private
from agent_live_check.runtime import Runtime
from agent_live_check.system import READY, remove_pane_files
from holder_owned import Owned

from .daemon import Daemon


def layout_panes(node):
    if node.get("type") == "pane":
        return [node["pane_id"]]
    return layout_panes(node["first"]) + layout_panes(node["second"])


class OwnRuntime(Runtime):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.control = None
        self.owned = Owned()
        self.owned.records(self.state)
        self.initial_shells = set()
        self.controller_pane = None
        self.controller_registered = False

    def start(self):
        result = super().start()
        self.control = Daemon(self.state, self.owner.cancelled)
        self.control.wait(lambda rest: rest, 20, "the native engine's first snapshot")
        return result

    def pane_ids(self):
        return {pane for layout in (self.control.snapshot() or {}).get("pane_layouts", [])
                for pane in layout_panes(layout["root"])}

    def open_workspace(self, cwd, environment):
        folder = str(cwd.resolve())
        self.control.dispatch("create_workspace", {"path": folder, "label": "Local bell check",
                              "initialize_git": False, "confirmed_outside_home": folder})
        # The core spells a path with `/` between names on every system.
        spelled = cwd.resolve().as_posix()

        def opened(rest):
            for workspace in (rest.get("navigator") or {}).get("workspaces", []):
                checkout = next((row for row in workspace.get("checkouts", []) if row["path"] == spelled), None)
                if not checkout:
                    continue
                tabs = [tab for group in (checkout.get("agent_groups") or {}).get("groups", [])
                        for tab in group.get("tabs", [])]
                for layout in rest.get("pane_layouts", []):
                    if layout["tab_id"] in tabs:
                        panes = layout_panes(layout["root"])
                        if panes:
                            return workspace["id"], panes[0]
            return None
        workspace, pane = self.control.wait(opened, 30, "the controller's new shell")
        self.workspaces.add(workspace)
        self.initial_shells.add(pane)
        self.controller_pane = pane
        self.control.dispatch("terminal_viewport", {"pane_id": pane, "cols": 120, "rows": 40, "new_view": True})
        self.control.dispatch("terminal_resize", {"pane_id": pane, "cols": 120, "rows": 40})
        self.control.wait(lambda _rest: (self.control.terminal(pane) or {}).get("transport_state") == "controlling",
                          20, "the new shell's input attachment")
        # Only this fresh shell takes uncredentialed setup input. A file
        # written by its shell proves it consumed the complete environment
        # setup before bootstrap, rather than a delay or a guessed prompt.
        ready = self.run / "native-controller-shell-ready"
        remove_pane_files(ready)
        for name in environment:
            if not name.replace("_", "").isalnum() or name[0].isdigit():
                raise ProtectionError("invalid_controller_environment_name")
        # The PTY can accept input before zsh leaves canonical mode. Its line
        # limit can truncate a long PATH along with the final Enter. Source a
        # private file instead, so only its short invocation crosses the PTY.
        setup = self.run / "native-controller-shell-setup.zsh"
        write_private(setup, ("\n".join(f"export {name}={shlex.quote(value)}" for name, value in environment.items())
                              + f"\nprintf ready > {shlex.quote(str(ready))}\n").encode())
        try:
            self.send_text(pane, f". {shlex.quote(str(setup))}\r")
            self.wait(lambda: ready.exists() and ready.read_text() == "ready", 20)
        finally:
            remove_pane_files(ready, setup)
        return workspace, pane

    def wait_shell(self, pane):
        if pane not in self.initial_shells:
            raise ProtectionError("unowned_initial_shell")

    def pane_at_shell(self, pane, deadline):
        if pane in self.initial_shells and pane not in self.pane_credentials:
            deadline.remaining()
            return True  # open_workspace proved this exact newly created shell.
        return READY in self.screen(pane, seconds=deadline.command_seconds(), deadline=deadline.end)

    def cli(self, args, *, seconds=15, deadline=None):
        reference = self.pane_credentials.get(self.controller_pane)
        if reference is None:
            raise ProtectionError("native_controller_not_bootstrapped")
        code, output, _ = self.owner.run([str(self.hide), *args],
            env={**self.daemon_env, "HIDE_CAP_REF": str(reference)}, check=False,
            seconds=seconds, deadline=deadline)
        if code:
            raise ProcessError(f"native_journey_cli_exit_{code}")
        try:
            answer = json.loads(output)
            if answer.get("ok") is not True:
                raise ValueError("refused")
            return answer["value"]
        except (ValueError, TypeError, KeyError) as error:
            raise ProcessError("native_journey_cli_answer_unreadable") from error

    def screen(self, pane, *, seconds=15, deadline=None):
        value = self.cli(["pane", "read", pane, "--lines", "120", "--json"], seconds=seconds, deadline=deadline)
        if not isinstance(value.get("text"), str):
            raise ProcessError("native_journey_screen_unreadable")
        return value["text"]

    def send_text(self, pane, text):
        self.control.dispatch("key", {"pane_id": pane, "bytes_base64": base64.b64encode(text.encode()).decode()})

    def send(self, pane, text, *, deadline=None):
        if pane in self.pane_credentials:
            require_no_login(self.screen(pane, seconds=deadline.command_seconds() if deadline else 15,
                                         deadline=deadline.end if deadline else None))
        elif pane not in self.initial_shells:
            raise ProtectionError("unowned_initial_shell")
        if deadline:
            deadline.remaining()
        self.send_text(pane, text + "\r")

    def agent(self, pane, *, seconds=15, deadline=None):
        row = self.control.row(pane)
        if not row or not row.get("agent_kind"):
            return None
        reference = None
        if pane == self.controller_pane and not self.controller_registered:
            if row.get("session_id"):
                reference = {"kind": "id", "value": row["session_id"], "observation": "snapshot"}
        else:
            listing = self.cli(["agent", "list"], seconds=seconds, deadline=deadline)
            record = next((item for item in listing["items"] if item.get("pane") == pane and item.get("registered")), None)
            if record and record.get("session"):
                value = record["session"]
                reference = {"kind": "path" if Path(value).is_absolute() else "id", "value": value,
                             "observation": "hide.agent.list"}
        # The journey's readiness contract uses engine idle/done, whereas
        # the screen calls that activity stopped and carries approval apart.
        status = "idle" if row.get("activity") == "stopped" else row.get("activity")
        return {"pane_id": pane, "terminal_id": pane, "agent_session": reference,
                "agent_status": "blocked" if row.get("blocked") else status,
                "launch_pending": row.get("status_code") == "starting"}

    def close_pane(self, pane):
        self.control.dispatch("close_pane", {"pane_id": pane, "confirmed": True})
        self.control.wait(lambda _rest: pane not in self.pane_ids(), 20, "the owned pane's close")
        return 0, ""

    def close_workspace(self, workspace):
        if workspace not in self.workspaces:
            raise ProtectionError("unowned_workspace_close_refused")
        layouts = (self.control.snapshot() or {}).get("pane_layouts", [])
        for pane in [pane for layout in layouts if layout.get("workspace_id") == workspace
                     for pane in layout_panes(layout["root"])]:
            self.close_pane(pane)
        self.workspaces.remove(workspace)

    def shell_pid(self, pane):
        for file in (self.state / "holders/records").glob("*.json"):
            record = json.loads(file.read_text())
            if record.get("pane") == pane.replace(":", "_") and record.get("program"):
                return record["program"]["pid"]
        raise ProcessError("owned_pane_shell_not_recorded")

    def stop_panes(self):
        failures = self.owned.close()
        if failures:
            raise ProcessError("native_holders_cleanup_unconfirmed:" + "; ".join(failures))

    def close(self):
        try:
            return super().close()
        finally:
            if self.control:
                self.control.close()
