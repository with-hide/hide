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
from agent_live_check.system import WINDOWS, exe, home_env, temp_env

COMMAND_SECONDS = 25
START_SECONDS = 60


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
        self.sequence = 0
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
            [str(runtime.herdr_bin), str(self.completed), sys.executable, "-I", str(driver), str(self.command)]).encode())
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

    def hide(self, args, seconds=COMMAND_SECONDS):
        """One `hide` command run by the controller agent; its JSON answer."""
        runtime = self.runtime
        self.sequence += 1
        self.completed.unlink(missing_ok=True)
        self.response.unlink(missing_ok=True)
        environment = {**home_env(runtime.home), "HIDE_STATE_DIR": str(runtime.state),
                       "PATH": runtime.env["PATH"], **temp_env(runtime.run / "agent-tmp")}
        self.command.unlink(missing_ok=True)
        write_private(self.command, json.dumps({"argv": [str(runtime.hide), *args], "env": environment,
                                                "response": str(self.response)}).encode())
        runtime.command(["pane", "send-text", self.pane, "!"])
        status = runtime.wait(lambda: self.completed.is_file() and self.completed.read_text().strip(), seconds)
        text = self.response.read_text() if self.response.is_file() else ""
        try:
            answer = json.loads(text)
        except ValueError:
            answer = None
        if not isinstance(answer, dict):
            return {"ok": False, "reason": "unreadable_answer", "exit": status, "stdout": text[:500]}
        return answer
