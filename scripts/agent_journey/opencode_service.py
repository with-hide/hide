"""OpenCode 2's background server, which the journey's OpenCode starts and
the journey owns.

OpenCode 2.0.29 runs the sessions of every terminal in one background server
(`opencode serve --service`), which the first terminal starts on the port its
`service.json` names (49374 by default, where the operator's may already
listen) and which outlives every terminal. The journey names a free port in
the private `service.json`, so its OpenCode never reaches the operator's
server, and marks the agent's environment, which the server inherits from
the terminal that started it, so the run tells its own servers from any
other. Every exit path stops them: OpenCode's own `service stop` first, then
a signal to each marked server still running.
"""

import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time

from agent_live_check.protection import private_directory, write_private
from agent_live_check.runtime import XDG_FOLDERS

# The variable whose value, the run's daemon home, marks the run's servers.
MARK = "HIDE_JOURNEY_OPENCODE"
# OpenCode's folders under the probe's home. Its configuration is
# `~/.config/opencode`, where the kit puts Hide's plugin beside Herdr's
# whatever `XDG_CONFIG_HOME` says (hide-agent-hooks/src/opencode.rs), as on a
# machine that leaves it unset; its sessions are under the probe's own
# `XDG_DATA_HOME`, the folder the daemon and Hide's reader read them from.
CONFIG = Path(".config/opencode")
DATA = Path(XDG_FOLDERS["XDG_DATA_HOME"], "opencode")


def environment(home):
    """The variables that keep an OpenCode command in those folders, a
    `service stop` run outside the pane included, and mark the server it
    starts as the run's."""
    home = Path(home)
    return {**{name: str(home / folder) for name, folder in XDG_FOLDERS.items()},
            "XDG_CONFIG_HOME": str(home / CONFIG.parent), MARK: str(home)}
STOP_SECONDS = 10


def configure(folder):
    """Writes OpenCode's `service.json` with a free port into its private
    configuration folder; returns the port."""
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    private_directory(folder)
    write_private(folder / "service.json", json.dumps({"port": port}).encode())
    return port


def listening(port):
    """Whether a server answers on the port."""
    with socket.socket() as client:
        client.settimeout(1)
        return client.connect_ex(("127.0.0.1", port)) == 0


def environment_of(pid):
    """The environment a process started with, as text; None where this
    system does not show it."""
    if sys.platform.startswith("linux"):
        try:
            return Path(f"/proc/{pid}/environ").read_bytes().replace(b"\0", b"\n").decode(errors="replace")
        except OSError:
            return ""
    if sys.platform == "darwin":
        # `ps -E` shows the environment after the command, for the account's own processes.
        found = subprocess.run(["/bin/ps", "-wwE", "-o", "command=", "-p", str(pid)], capture_output=True, text=True,
                               timeout=15)
        return found.stdout.replace(" ", "\n")
    return None


def ours(mark):
    """The pids of the OpenCode servers the run's agents started; None where
    this system cannot tell them apart."""
    if os.name == "nt":
        return None
    found = subprocess.run(["pgrep", "-f", "serve --service"], capture_output=True, text=True, timeout=15)
    marked = []
    for pid in (int(value) for value in found.stdout.split()):
        text = environment_of(pid)
        if text is None:
            return None
        if f"{MARK}={mark}" in text.splitlines():
            marked.append(pid)
    return marked


def stop(launch, environment, port, mark, owner):
    """Stops the run's OpenCode servers and confirms the port is free; returns
    what was stopped and how, and `left` naming any still running."""
    before = ours(mark)
    record = {"port": port, "servers": before, "stopped_by": None, "left": []}
    if not before and not listening(port):
        return record
    try:
        code, _, error = owner.run([str(launch), "service", "stop"], env=environment, check=False, seconds=30)
        record["service_stop"] = code if not code else f"{code}: {error.strip()[:200]}"
    except Exception as failure:
        # A stopped run's owner refuses new work; the signals below still end the server.
        record["service_stop"] = f"not run: {failure}"
    end = time.monotonic() + STOP_SECONDS
    while time.monotonic() < end and (listening(port) or ours(mark)):
        time.sleep(0.2)
    record["stopped_by"] = "service stop"
    for pid in ours(mark) or []:
        # OpenCode's own stop did not end it: the run started it, so the run ends it.
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
        record["stopped_by"] = "signal"
    end = time.monotonic() + STOP_SECONDS
    while time.monotonic() < end and (listening(port) or ours(mark)):
        time.sleep(0.2)
    record["left"] = ours(mark) or []
    if listening(port):
        record["left"].append(f"port {port}")
    return record
