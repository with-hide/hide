"""Temporary macOS fake-CI authentication evidence, with no raw log retained."""

import json
import os
from pathlib import Path
import select
import sys
import threading

MAX_BYTES = 32 * 1024 * 1024
MAX_LINE = 4 * 1024 * 1024
MAX_EVENTS = 64
KEY = "JOURNEY_FAKE_KEY"
ROUTE = "JOURNEY_GROK_ROUTE "


def relative(path, run):
    try:
        return str(path.resolve().relative_to(run.resolve()))
    except ValueError:
        return "outside-run"


def route(run):
    home = Path(os.environ["HOME"])
    grok = Path(os.environ.get("GROK_HOME", str(home / ".grok")))
    return {"key_name": KEY, "key_present": bool(os.environ.get(KEY)),
            "home": relative(home, run), "config": relative(grok / "config.toml", run),
            "config_exists": (grok / "config.toml").is_file()}


class Diagnostic:
    def __init__(self, runtime):
        self.owner = runtime.owner
        self.path = runtime.probe / "grok-diagnostic.fifo"
        os.mkfifo(self.path, 0o600)
        self.fd = os.open(self.path, os.O_RDWR | os.O_NONBLOCK)
        self.stop = threading.Event()
        self.result = {"routes": [], "authentication_methods": [], "config_model_seen": False,
                       "scope": "wrapper environment and native log; no filesystem-open tracing"}
        self.thread = threading.Thread(target=self.read, daemon=True)
        self.thread.start()

    def launcher(self, runtime, program):
        from agent_live_check.system import launcher
        return launcher(runtime.bin / "grok-diagnostic", [sys.executable, Path(__file__).resolve(),
                        runtime.run, self.path, program])

    def line(self, line):
        if line.startswith(ROUTE.encode()):
            self.result["routes"].append(json.loads(line[len(ROUTE):]))
        if b"config model override applied model_key=journey " in line:
            self.result["config_model_seen"] = True
        marker = b"auth: authenticate request method="
        if marker in line:
            method = line.split(marker, 1)[1].split()[0].decode("ascii", errors="replace")
            self.result["authentication_methods"].append(method if method in ("xai.api_key", "grok.com") else "other")
        if len(self.result["routes"]) + len(self.result["authentication_methods"]) > MAX_EVENTS:
            raise ValueError("event_budget")

    def read(self):
        pending = bytearray()
        total = 0
        try:
            while True:
                if not select.select([self.fd], [], [], 0.1)[0]:
                    if self.stop.is_set():
                        break
                    continue
                chunk = os.read(self.fd, 65536)
                total += len(chunk)
                if total > MAX_BYTES:
                    raise ValueError("byte_budget")
                pending.extend(chunk)
                while b"\n" in pending:
                    line, _, pending = pending.partition(b"\n")
                    if len(line) > MAX_LINE:
                        raise ValueError("line_budget")
                    self.line(line)
                if len(pending) > MAX_LINE:
                    raise ValueError("line_budget")
        except Exception:
            self.result["failure"] = "grok_diagnostic_unreadable_or_over_budget"
            self.owner.cancelled.set()

    def close(self):
        self.stop.set()
        self.thread.join(timeout=2)
        if self.thread.is_alive():
            raise RuntimeError("grok_diagnostic_reader_did_not_end")
        os.close(self.fd)
        self.path.unlink(missing_ok=True)
        return self.result


if __name__ == "__main__":
    run, pipe, program, *arguments = sys.argv[1:]
    # The preceding CI wrapper already supplied HOME and the fake key.
    # One short, atomic FIFO record, followed by the original native process.
    with open(pipe, "w") as stream:
        stream.write(ROUTE + json.dumps(route(Path(run))) + "\n")
    os.environ["GROK_DEBUG_LOG"] = pipe
    os.execv(program, [program, *arguments])
