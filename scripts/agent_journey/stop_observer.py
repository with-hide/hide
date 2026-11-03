"""A fixture-owned HTTP Stop observer, with no command or callback file writes.

The native host posts its original payload to loopback. The observer keeps
only bounded probe metadata in memory and replies with no model feedback.
"""
import copy
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import re
import secrets
import threading
import time

INPUT_CAP = 1024 * 1024
RECORD_CAP = 8
RECORD_BYTES = 4096
HOOK_SECONDS = 5


def contained(path, root):
    """Refuse aliases at every component before creating or removing config."""
    path.absolute().relative_to(root)
    path.resolve().relative_to(root)
    for part in (path, *path.parents):
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError("observer_path_is_link")
        if part == root:
            return
    raise ValueError("observer_path_outside_probe")


def project(payload, state, now):
    if not isinstance(payload, dict) or not isinstance(payload.get("lastAssistantMessage"), str):
        raise ValueError("invalid_stop_payload")
    return {"session": payload.get("sessionId"), "active": payload.get("stopHookActive"),
            "reason": payload.get("reason"), "subagent": bool(payload.get("subagentType")),
            "answers": re.findall("|".join(re.escape(state[key]) for key in ("held", "token")),
                                  payload["lastAssistantMessage"]), "at": now}


class Server(HTTPServer):
    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(HOOK_SECONDS)
        return connection, address

    def handle_error(self, _request, _address):
        # Never print a payload/traceback. A handler or response-write failure
        # is independent in-memory evidence, not a write to the failed disk.
        with self.observer.lock:
            self.observer.failure = "observer_transport_failed"


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        observer = self.server.observer
        with observer.lock:
            self.state = observer.state
            self.slot = None
            if self.state is not None:
                if len(observer.records) == RECORD_CAP:
                    observer.failure = "observer_overflow"
                else:
                    self.slot = len(observer.records)
                    observer.records.append({"error": "observer_incomplete"})

    def do_POST(self):
        observer = self.server.observer
        if self.state is not None and self.slot is not None:
            record = {"error": "observer_failed"}
            try:
                if self.path != observer.route:
                    raise ValueError("wrong_route")
                size = int(self.headers.get("Content-Length", "-1"))
                if not 0 <= size <= INPUT_CAP or self.headers.get("Transfer-Encoding"):
                    raise ValueError("input_cap")
                raw = self.rfile.read(size)
                now = time.time_ns() // 1_000_000
                if len(raw) != size or not self.state["start"] <= now <= self.state["end"]:
                    raise ValueError("incomplete_or_expired")
                record = project(json.loads(raw), self.state, now)
                if len(json.dumps(record).encode()) > RECORD_BYTES:
                    raise ValueError("record_cap")
            except Exception:
                record = {"error": "observer_failed"}
            with observer.lock:
                observer.records[self.slot] = record
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, _format, *_args):
        pass


class Observer:
    def __init__(self, home, probe, relative):
        from agent_live_check.protection import private_directory, write_private
        self.probe = probe.resolve()
        home = self.probe / home.relative_to(probe)
        contained(home, self.probe)
        self.config = home.joinpath(*relative)
        contained(self.config, self.probe)
        if not self.config.parent.exists():
            private_directory(self.config.parent)
        contained(self.config, self.probe)
        self.lock = threading.Lock()
        self.state, self.records, self.failure = None, [], None
        self.route = "/" + secrets.token_hex(24)
        self.server = Server(("127.0.0.1", 0), Handler)
        self.server.observer = self
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={"poll_interval": 0.05}, daemon=True)
        try:
            url = f"http://127.0.0.1:{self.server.server_address[1]}{self.route}"
            write_private(self.config, json.dumps({"hooks": {"Stop": [{"hooks": [
                {"type": "http", "url": url, "timeout": HOOK_SECONDS}]}]}}).encode())
            info = self.config.stat()
            self.identity = (info.st_dev, info.st_ino)
            self.thread.start()
        except Exception:
            self.server.server_close()
            raise

    def arm(self, held, token, seconds):
        now = time.time_ns() // 1_000_000
        with self.lock:
            if self.state is not None:
                raise ValueError("observer_already_armed")
            self.state = {"held": held, "token": token, "start": now, "end": now + seconds * 1000}

    def read(self):
        with self.lock:
            if self.closed or not self.thread.is_alive():
                return [{"error": "observer_not_running"}]
            return [{"error": self.failure}] if self.failure else copy.deepcopy(self.records)

    def close(self):
        # End the owned listener even when configuration recovery refuses a
        # replaced path. A request is capped at HOOK_SECONDS by the socket.
        if not self.closed:
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(HOOK_SECONDS + 1)
            self.closed = True
            if self.thread.is_alive():
                raise RuntimeError("observer_cleanup_unconfirmed")
        contained(self.config, self.probe)
        if self.config.exists():
            info = self.config.stat()
            if (info.st_dev, info.st_ino) != self.identity:
                raise ValueError("observer_config_replaced")
            self.config.unlink()
