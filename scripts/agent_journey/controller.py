"""The journey's controller: the registered agent pane every spawn is made from.

`hide agent spawn` takes its caller from the pane's process tree, so the hide
commands run as descendants of an agent the engine knows. The controller is the
desktop e2e's spawn provider (`web/e2e/shims/spawn-provider.c`, built here):
started as a Claude kind in its own private pane, it reports a synthetic
native session to the core through the `session_report.py` it writes and, each time it reads `!`, runs the command
`command.json` holds through `controller_command.py`, writing the command's
exit status to a completion file. Its process outcomes are part of the
measurement boundary, checked independently of the engine and the agent CLIs.
"""

import json
from pathlib import Path
import shutil
import sys

from agent_live_check.processes import Cancelled, ProcessError
from agent_live_check.protection import private_directory, write_private
from agent_live_check.runtime import clean_env
from agent_live_check.system import WINDOWS, exe, home_env, remove_pane_files, temp_env

# The local CLI may bootstrap (10s), exchange a spawn (30s), then claim (2s).
# Its supervisor must outlive that protocol, rather than cut it off at 20s.
COMMAND_SECONDS = 10 + 30 + 2
SUPERVISOR_CLEANUP_SECONDS = 20
COMPLETION_SECONDS = COMMAND_SECONDS + SUPERVISOR_CLEANUP_SECONDS + 5
OUTPUT_BYTES = 1024 * 1024
MAX_COMMANDS = 512


class ControllerUnavailable(ProcessError):
    """No command runs after one whose processes were not confirmed ended."""


def command_answer(diagnostic, text):
    """The product's answer, trusted only from a settled, whole run.

    Settled: the supervisor's outcome for this command confirms every process
    of it ended, so nothing of it can overlap the next command; anything else
    is `controller_command_unsettled`. Whole: the supervisor and the driver
    both saw an ordinary exit with the same status and the driver captured
    both streams to their end; a settled run that is not whole is
    `controller_command_failed`, however good its JSON looks.
    """
    supervisor, driver = diagnostic["supervisor"], diagnostic["driver"]
    settled = (not diagnostic["wait_error"] and isinstance(supervisor, dict)
               and supervisor.get("id") == diagnostic["id"] and supervisor.get("cleanup_confirmed") is True
               and str(supervisor.get("exit")) == diagnostic["exit"])
    if not settled:
        return {"ok": False, "reason": "controller_command_unsettled", **diagnostic}
    whole = (supervisor.get("phase") == "exited" and isinstance(driver, dict)
             and driver.get("id") == diagnostic["id"] and driver.get("phase") == "exited"
             and driver.get("exit") == supervisor.get("exit"))
    if not whole:
        return {"ok": False, "reason": "controller_command_failed", **diagnostic}
    try:
        answer = json.loads(text)
    except ValueError:
        answer = None
    if not isinstance(answer, dict) or (answer.get("ok") is True and driver["exit"] != 0):
        return {"ok": False, "reason": "unreadable_answer", **diagnostic}
    return answer


def outcome(diagnostic):
    """One line of constant identifiers for what the command's processes reported."""
    if diagnostic["wait_error"]:
        return "no completion: " + diagnostic["wait_error"]
    supervisor, driver = diagnostic["supervisor"], diagnostic["driver"]
    if not isinstance(supervisor, dict) or supervisor.get("id") != diagnostic["id"]:
        return "no supervisor outcome for this command"
    parts = [f"supervisor {supervisor.get('phase')} exit {supervisor.get('exit')}"]
    if str(supervisor.get("exit")) != diagnostic["exit"]:
        parts.append(f"completion status {diagnostic['exit']}")
    if supervisor.get("cleanup_confirmed") is not True:
        parts.append(f"cleanup unconfirmed (error {supervisor.get('cleanup_error')})")
    if not isinstance(driver, dict) or driver.get("id") != diagnostic["id"]:
        parts.append("no driver result for this command")
    else:
        details = [f"driver {driver.get('phase')} exit {driver.get('exit')}"]
        if driver.get("os_error") is not None:
            details.append(f"os_error {driver['os_error']}")
        if "child_ms" in driver:
            details.append(f"after {driver['child_ms']} ms")
        details.extend(f"{name} {state}" for name, state in (driver.get("streams") or {}).items())
        parts.append(" ".join(details))
    return "; ".join(parts)


class Controller:
    def __init__(self, runtime, provider=None):
        self.runtime = runtime
        # The spawn provider built before the run (agent-journey.yml), or None to build it here.
        self.provider = provider
        self.root = runtime.probe / "controller"
        private_directory(self.root)
        self.bin = self.root / "bin"
        private_directory(self.bin)
        self.command = self.root / "command.json"
        self.completed = self.root / "command-completed"
        self.response = self.root / "response.json"
        self.stderr = self.root / "stderr.txt"
        self.result = self.root / "command-result.json"
        self.outcome = self.root / "command-completed.json"
        self.sequence = 0
        # The command whose processes were not confirmed ended, once one was not.
        self.unsettled = None
        self.pane = self.agent_id = None

    def start(self):
        runtime = self.runtime
        source = runtime.checkout / "web/e2e/shims/spawn-provider.c"
        shim = self.bin / exe("claude")
        if self.provider:
            write_private(shim, self.provider.read_bytes())
            shim.chmod(0o700)
        else:
            # The e2e's compilers (web/e2e/shims/build.ts), with the account's
            # environment, where clang finds the Windows SDK and linker; a
            # Windows link can outlast the command cap, so CI builds it first.
            compiler = shutil.which("clang") if WINDOWS else "/usr/bin/cc"
            if not compiler:
                raise ProcessError("controller_compiler_missing: install clang (LLVM)")
            runtime.owner.run([compiler, "-O2", "-o", str(shim), str(source)],
                              env={**clean_env(), **temp_env(runtime.run / "agent-tmp")})
        driver = Path(__file__).resolve().with_name("controller_command.py")
        # The provider reports its session as `<python> <reporter> <pane> <session>`
        # (spawn-provider.c): through Hide's SessionStart hook, as Claude Code does.
        reporter = self.root / "session_report.py"
        hooks = runtime.hided.with_name(exe("hide-agent-hooks"))
        write_private(reporter, (
            "import json, os, subprocess, sys\n"
            "pane, session = sys.argv[1:3]\n"
            "payload = json.dumps(dict(session_id=session, hook_event_name='SessionStart', source='startup', cwd=os.getcwd()))\n"
            f"sys.exit(subprocess.run([{str(hooks)!r}, 'hook', '--runtime', 'claude-code', '--event', 'SessionStart'], input=payload, text=True, timeout=20).returncode)\n").encode())
        write_private(self.root / "provider.config", b"")
        runtime.owner.run([str(shim), "--write-config", str(self.root / "provider.config"),
                           sys.executable, str(reporter), str(self.completed),
                           f"within-ms={COMMAND_SECONDS * 1000}", sys.executable,
                           "-I", str(driver), str(self.command)], env=clean_env())
        recipe = {"id": "controller"}
        workspace, pane, _ = runtime.new_workspace(recipe, "journey", runtime.home, shim,
                                                   {"env": {"HIDE_E2E_ROOT": str(self.root)}})
        self.workspace, self.pane = workspace, pane
        runtime.send(pane, runtime.shell.run([shim]))
        runtime.initial_shells.discard(pane)
        native = runtime.wait(lambda: (agent := runtime.agent(pane)) and (agent.get("agent_session") or {}).get("value")
                              and agent, 20)
        answer = self.hide(["agent", "register", "--host-scope", str(runtime.state / "engine"),
                            "--session", native["agent_session"]["value"], "--instance", native["terminal_id"],
                            "--name", "journey-controller", "--pane", pane])
        if answer.get("ok") is not True:
            raise ProcessError("controller_register_refused:" + json.dumps(answer)[:300])
        self.agent_id = answer["value"]["id"]
        runtime.controller_registered = True
        return self.agent_id

    def _read(self, args):
        """Observe through the controller's capability without waking its pane.

        Polling owns no command file and consumes none of the controller's
        bounded mutation budget. The process owner still bounds each read.
        """
        runtime = self.runtime
        reference = runtime.pane_credentials[self.pane]
        code, output, error = runtime.owner.run(
            [str(runtime.hide), *args, "--json"],
            env={**runtime.daemon_env, "HIDE_CAP_REF": str(reference)}, check=False,
        )
        if code:
            raise ProcessError(f"journey_read_exit_{code}: {error.strip()[:200]}")
        try:
            answer = json.loads(output)
            value = answer["value"]
            if answer.get("ok") is not True or not isinstance(value, dict):
                raise ValueError("missing read result")
            return value
        except (ValueError, KeyError, TypeError) as error:
            raise ProcessError("journey_read_unreadable") from error

    def list_agents(self):
        """The same public registration list, read without an input turn."""
        value = self._read(["agent", "list"])
        if not isinstance(value.get("items"), list):
            raise ProcessError("journey_agent_list_unreadable")
        return value["items"]

    def read_pane(self, pane, *, visible=False, lines=120):
        """A styled screen must be whole before the composer can judge it."""
        flags = ["--source", "visible", "--format", "ansi"] if visible else ["--lines", str(lines)]
        value = self._read(["pane", "read", pane, *flags])
        try:
            if not isinstance(value.get("text"), str):
                raise ValueError("missing pane text")
            if visible and value.get("truncated") is not False:
                raise ProcessError("journey_visible_screen_truncated")
            return value["text"]
        except (ValueError, KeyError, TypeError) as error:
            raise ProcessError("journey_pane_read_unreadable") from error

    def hide(self, args, seconds=COMPLETION_SECONDS):
        """One `hide` command run by the controller agent; the product's JSON answer.

        A command the controller could not run to a trusted answer raises,
        naming the command and what its processes reported, so the cell reads
        as the controller's failure rather than a refusal of the product. One
        whose processes were all confirmed ended leaves the next command free;
        one that was not refuses every later command (ControllerUnavailable).
        """
        runtime = self.runtime
        if self.unsettled:
            raise ControllerUnavailable("controller_previous_command_unsettled: " + self.unsettled)
        if self.sequence >= MAX_COMMANDS:
            raise ProcessError("controller_command_limit")
        self.sequence += 1
        command = f"command {self.sequence} ({' '.join(args[:2])})"
        remove_pane_files(self.completed, self.response, self.stderr, self.result, self.outcome)
        environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                       "PATH": runtime.env["PATH"], **temp_env(runtime.run / "agent-tmp")}
        remove_pane_files(self.command)
        write_private(self.command, json.dumps({"id": self.sequence, "argv": [str(runtime.hide), *args],
                                                "env": environment, "response": str(self.response),
                                                "stderr": str(self.stderr), "result": str(self.result)}).encode())
        runtime.send_text(self.pane, "!")
        wait_error, status = None, None
        try:
            status = runtime.wait(lambda: self.completed.is_file() and self.completed.read_text().strip(), seconds)
        except (ProcessError, Cancelled) as error:
            wait_error = str(error)
            self.unsettled = f"{command} no completion: {wait_error}"
            if isinstance(error, Cancelled):
                raise
        def output(path):
            if not path.is_file():
                return b""
            with path.open("rb") as stream:
                return stream.read(OUTPUT_BYTES)
        raw_response, raw_stderr = output(self.response), output(self.stderr)
        diagnostic = {"id": self.sequence, "exit": status, "wait_error": wait_error,
                      "stdout_bytes": len(raw_response), "stderr_bytes": len(raw_stderr)}
        for key, path in (("supervisor", self.outcome), ("driver", self.result)):
            try:
                diagnostic[key] = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                diagnostic[key] = None
        # Preserve each command's adverse evidence even when later cells run.
        evidence = runtime.run / f"controller-{self.sequence:04d}.json"
        write_private(evidence.with_suffix(".stdout"), raw_response)
        write_private(evidence.with_suffix(".stderr"), raw_stderr)
        write_private(evidence, json.dumps(diagnostic).encode())
        answer = command_answer(diagnostic, raw_response)
        reason = answer.get("reason")
        if reason in ("controller_command_unsettled", "controller_command_failed"):
            failure = f"{command} {outcome(diagnostic)}"
            if reason == "controller_command_unsettled":
                self.unsettled = failure
            raise ProcessError(f"{reason}: {failure}")
        return answer
