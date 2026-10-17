"""The candidate daemon's token-gated /ws, as the shell speaks it.

One connection, standard library only. The handshake asks for no snapshot
format, so every snapshot and delta carries the whole rest and the newest one
is the state; the usage section rides beside it with a revision of its own,
so a frame without it keeps the last one. Events go out as the shell sends
them (contracts/hided-ws.schema.json).
"""

import base64
import json
import os
from pathlib import Path
import socket
import struct
import threading
import time

from agent_live_check.processes import ProcessError

# A snapshot of a private daemon with a few agents is well under this.
MAX_FRAME = 32 * 1024 * 1024
CONNECT_SECONDS = 10
HISTORY_CAP = 512


class Frames:
    """RFC 6455 framing for a client: masked sends, unmasked reads."""

    @staticmethod
    def encode(opcode, data, mask=None):
        mask = mask or os.urandom(4)
        head = bytes([0x80 | opcode])
        size = len(data)
        if size < 126:
            head += bytes([0x80 | size])
        elif size < 1 << 16:
            head += bytes([0x80 | 126]) + struct.pack("!H", size)
        else:
            head += bytes([0x80 | 127]) + struct.pack("!Q", size)
        return head + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))

    @staticmethod
    def read(read_exact):
        """One whole message: (opcode, payload); continuation frames joined."""
        message, first = bytearray(), None
        while True:
            head = read_exact(2)
            final, opcode = head[0] & 0x80, head[0] & 0x0F
            size = head[1] & 0x7F
            if head[1] & 0x80:
                raise ProcessError("daemon_frame_masked")
            if size == 126:
                size = struct.unpack("!H", read_exact(2))[0]
            elif size == 127:
                size = struct.unpack("!Q", read_exact(8))[0]
            if len(message) + size > MAX_FRAME:
                raise ProcessError("daemon_frame_over_budget")
            payload = read_exact(size) if size else b""
            if opcode >= 0x8:
                return opcode, payload
            if first is None:
                first = opcode
            message.extend(payload)
            if final:
                return first, bytes(message)


class Daemon:
    def __init__(self, state: Path, cancelled: threading.Event):
        value = json.loads((state / "hided.json").read_text())
        self.port, token = int(value["port"]), value["token"]
        self.cancelled = cancelled
        self.lock = threading.Condition()
        self.rest = None
        self.usage = None
        self.revision = 0
        self.errors = []
        # Every activity each pane's row read, with when this client saw it,
        # in the order the frames carried them, so a turn shorter than the
        # wait for its action's answer is still seen; a run makes a few turns
        # per pane.
        self.history = {}
        self.closed = None
        self.sock = socket.create_connection(("127.0.0.1", self.port), timeout=CONNECT_SECONDS)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{self.port}\r\nUpgrade: websocket\r\n"
                           f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nOrigin: http://127.0.0.1:{self.port}\r\n"
                           "Sec-WebSocket-Version: 13\r\n\r\n").encode())
        reply = b""
        while b"\r\n\r\n" not in reply:
            chunk = self.sock.recv(4096)
            if not chunk or len(reply) > 65536:
                raise ProcessError("daemon_upgrade_refused")
            reply += chunk
        head, self.buffer = reply.split(b"\r\n\r\n", 1)
        if not head.startswith(b"HTTP/1.1 101"):
            raise ProcessError("daemon_upgrade_refused")
        self.sock.settimeout(None)
        self.send_lock = threading.Lock()
        self._send(0x1, json.dumps({"token": token, "schema_version": 2, "client_kind": "web"}).encode())
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _exact(self, size):
        while len(self.buffer) < size:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("daemon_socket_closed")
            self.buffer += chunk
        data, self.buffer = self.buffer[:size], self.buffer[size:]
        return data

    def _send(self, opcode, data):
        with self.send_lock:
            self.sock.sendall(Frames.encode(opcode, data))

    def _read(self):
        try:
            while True:
                opcode, payload = Frames.read(self._exact)
                if opcode == 0x8:
                    code = struct.unpack("!H", payload[:2])[0] if len(payload) >= 2 else None
                    raise ConnectionError(f"daemon_closed_{code}:{payload[2:].decode(errors='replace')}")
                if opcode == 0x9:
                    self._send(0xA, payload)
                    continue
                if opcode != 0x1:
                    continue
                frame = json.loads(payload)
                with self.lock:
                    if frame.get("type") in ("snapshot", "delta") and isinstance(frame["payload"].get("usage"), dict):
                        self.usage = frame["payload"]["usage"]
                    if frame.get("type") in ("snapshot", "delta") and isinstance(frame["payload"].get("rest"), dict):
                        rest = frame["payload"]["rest"]
                        # A schema-2 delta carries each changed section whole
                        # and leaves out the rest; an omitted Workspace clears it.
                        self.rest = rest if frame["type"] == "snapshot" else {
                            **(self.rest or {}), **rest, "workspace_view": rest.get("workspace_view")}
                        self.revision += 1
                        self.record_activities()
                    elif frame.get("type") == "error":
                        self.errors.append(frame.get("message") or frame.get("payload"))
                    self.lock.notify_all()
        except Exception as error:
            # Any reader failure, a malformed frame included, ends the connection by name.
            with self.lock:
                self.closed = f"{type(error).__name__}: {error}"
                self.lock.notify_all()

    def record_activities(self):
        """Appends each row's activity when it changed; called under the lock."""
        for agent in (self.rest.get("navigator") or {}).get("agents", []):
            seen = self.history.setdefault(agent.get("pane_id"), [])
            if not seen or seen[-1][0] != agent.get("activity"):
                if len(seen) >= HISTORY_CAP:
                    self.closed = f"activity_history_over_cap: {agent.get('pane_id')} changed {HISTORY_CAP} times"
                    return
                seen.append((agent.get("activity"), int(time.time() * 1000)))

    def mark(self, pane):
        """Where the pane's activity history stands, for a turn about to start."""
        with self.lock:
            return len(self.history.get(pane, []))

    def activities(self, pane, since=0):
        """(activity, Unix ms seen) from `since` on, with the row as it reads
        now as the last observation."""
        with self.lock:
            # A row from a reader that stopped is no observation of now.
            if self.closed:
                raise ProcessError("daemon_connection_lost:" + self.closed)
            seen = list(self.history.get(pane, [])[since:])
            row = self.row(pane)
            if row:
                seen.append((row.get("activity"), int(time.time() * 1000)))
            return seen

    def dispatch(self, kind, payload):
        if self.closed:
            raise ProcessError("daemon_connection_lost:" + self.closed)
        self._send(0x1, json.dumps({"schema_version": 2, "kind": kind, "payload": payload}).encode())

    def wait(self, predicate, seconds, what):
        """The first value `predicate(rest)` answers truthy, read on each new snapshot."""
        end = time.monotonic() + seconds
        with self.lock:
            while True:
                if self.cancelled.is_set():
                    raise ProcessError("run_cancelled")
                if self.closed:
                    raise ProcessError("daemon_connection_lost:" + self.closed)
                if self.rest is not None and (value := predicate(self.rest)):
                    return value
                left = end - time.monotonic()
                if left <= 0:
                    raise ProcessError(f"timeout after {seconds:.0f}s waiting for {what}")
                self.lock.wait(min(left, 0.5))

    def snapshot(self):
        with self.lock:
            return self.rest

    def usage_section(self):
        """The usage the core published last (`usageSnapshot`), or None."""
        with self.lock:
            return self.usage

    def row(self, pane):
        rest = self.snapshot() or {}
        agents = (rest.get("navigator") or {}).get("agents", [])
        return next((agent for agent in agents if agent.get("pane_id") == pane), None)

    def terminal(self, pane):
        """The pane's terminal as the core reports it to this screen, or None."""
        rest = self.snapshot() or {}
        panes = (rest.get("terminal") or {}).get("panes", [])
        return next((entry for entry in panes if entry.get("pane_id") == pane), None)

    def close(self):
        try:
            self._send(0x8, struct.pack("!H", 1000))
        except OSError:
            pass
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()
        self.reader.join(timeout=5)
