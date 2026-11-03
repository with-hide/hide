"""The journey's controller: the registered agent pane every spawn is made from.

`hide agent spawn` takes its caller from the pane's process tree, so the hide
commands run as descendants of an agent Herdr knows. The controller is the
desktop e2e's spawn provider (`web/e2e/shims/spawn-provider.c`, built here):
started as a Claude kind in its own private pane, it reports a synthetic
native session to Herdr and, each time it reads `!`, runs the command
`command.json` holds through `controller_command.py`, writing the command's
exit status to a completion file. It is not under test.
"""

import json
from pathlib import Path
import shutil
import sys
import time

from agent_live_check.processes import ProcessError
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
START_SECONDS = 60


def command_answer(diagnostic, text):
    """A settled, correlated child result is required before trusting JSON."""
    supervisor, driver = diagnostic["supervisor"], diagnostic["driver"]
    settled = (not diagnostic["wait_error"] and isinstance(supervisor, dict)
               and supervisor.get("id") == diagnostic["id"] and supervisor.get("phase") == "exited"
               and supervisor.get("cleanup_confirmed") is True and isinstance(driver, dict)
               and driver.get("id") == diagnostic["id"] and driver.get("phase") == "exited"
               and str(supervisor.get("exit")) == diagnostic["exit"]
               and driver.get("exit") == supervisor.get("exit"))
    if not settled:
        return {"ok": False, "reason": "controller_command_failed", **diagnostic}
    try:
        answer = json.loads(text)
    except ValueError:
        answer = None
    if not isinstance(answer, dict) or (answer.get("ok") is True and driver["exit"] != 0):
        return {"ok": False, "reason": "unreadable_answer", **diagnostic}
    return answer


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
        self.unsettled = False
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
        write_private(self.root / "provider.config", "\n".join(
            [str(runtime.herdr_bin), str(self.completed), f"within-ms={COMMAND_SECONDS * 1000}",
             sys.executable, "-I", str(driver), str(self.command)]).encode())
        recipe = {"id": "controller"}
        workspace, pane, _ = runtime.new_workspace(recipe, "journey", runtime.home, shim,
                                                   {"env": {"HIDE_E2E_ROOT": str(self.root)}})
        self.workspace, self.pane = workspace, pane
        # Herdr starts an agent only in a shell with nothing of its own in the
        # foreground; the bootstrap's last command may still be finishing.
        def shell_alone():
            _, out, _ = runtime.command(["pane", "process-info", "--pane", pane], check=False)
            try:
                info = json.loads(out)["result"]["process_info"]
            except (ValueError, KeyError, TypeError):
                return False
            shell = info.get("shell_pid")
            return bool(shell) and info.get("foreground_process_group_id") == shell and all(
                process.get("pid") == shell for process in info.get("foreground_processes", []))
        # Herdr can still read the pane busy just after the shell looks alone
        # (its own process read lags); each refusal waits for the shell again,
        # within one bound.
        end = time.monotonic() + START_SECONDS
        while True:
            runtime.wait(shell_alone, 20)
            code, out, err = runtime.command(["agent", "start", "controller", "--kind", "claude", "--pane", pane,
                                              "--timeout", "5000"], seconds=15, check=False)
            # Herdr's own wait for the start can run out on a loaded machine
            # while the provider still starts; its session report below decides.
            if not code or '"timeout"' in out + err:
                break
            if "agent_pane_busy" not in out + err or time.monotonic() > end or runtime.owner.cancelled.wait(2):
                raise ProcessError("controller_start_refused:" + (out + err).strip()[:300])
        native = runtime.wait(lambda: (agent := runtime.agent(pane)) and (agent.get("agent_session") or {}).get("value")
                              and agent, 20)
        answer = self.hide(["agent", "register", "--host-scope", str(runtime.socket),
                            "--session", native["agent_session"]["value"], "--instance", native["terminal_id"],
                            "--name", "journey-controller", "--pane", pane])
        if answer.get("ok") is not True:
            raise ProcessError("controller_register_refused:" + json.dumps(answer)[:300])
        self.agent_id = answer["value"]["id"]
        return self.agent_id

    def hide(self, args, seconds=COMPLETION_SECONDS):
        """One `hide` command run by the controller agent; its JSON answer."""
        runtime = self.runtime
        if self.unsettled:
            raise ProcessError("controller_previous_command_unsettled")
        if self.sequence >= MAX_COMMANDS:
            raise ProcessError("controller_command_limit")
        self.sequence += 1
        remove_pane_files(self.completed, self.response, self.stderr, self.result, self.outcome)
        environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                       "PATH": runtime.env["PATH"], **temp_env(runtime.run / "agent-tmp")}
        remove_pane_files(self.command)
        write_private(self.command, json.dumps({"id": self.sequence, "argv": [str(runtime.hide), *args],
                                                "env": environment, "response": str(self.response),
                                                "stderr": str(self.stderr), "result": str(self.result)}).encode())
        runtime.command(["pane", "send-text", self.pane, "!"])
        wait_error, status = None, None
        try:
            status = runtime.wait(lambda: self.completed.is_file() and self.completed.read_text().strip(), seconds)
        except ProcessError as error:
            wait_error = str(error)
            self.unsettled = True
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
        if answer.get("reason") == "controller_command_failed":
            self.unsettled = True
        return answer
