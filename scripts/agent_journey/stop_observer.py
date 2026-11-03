"""Read-only Stop evidence in the journey's private HOME, never a delivery hook.

The command emits nothing. Its bounded records contain probe token positions,
not assistant text. Missing, malformed or overflowed evidence fails the cell.
"""
import json
import os
from pathlib import Path
import re
import shlex
import sys
import time
import tempfile

INPUT_CAP = 1024 * 1024
RECORD_CAP = 8
STATE_CAP = 4096
RECORD_BYTES = 4096
HOOK_SECONDS = 5


def project(payload, state, now):
    text = payload.get("lastAssistantMessage")
    if not isinstance(text, str):
        raise ValueError("missing_response")
    return {"session": payload.get("sessionId"), "active": payload.get("stopHookActive"),
            "reason": payload.get("reason"), "subagent": bool(payload.get("subagentType")),
            "answers": re.findall("|".join(re.escape(state[key]) for key in ("held", "token")), text),
            "at": now}


def observe(directory, stream):
    """One hook invocation; no stdout, stderr or change to the hook payload."""
    state_path = directory / "armed.json"
    if not state_path.exists():
        return
    record = {"error": "observer_failed"}
    try:
        with state_path.open("rb") as source:
            state = json.loads(source.read(STATE_CAP + 1))
        now = time.time_ns() // 1_000_000
        if not state["start"] <= now <= state["end"]:
            raise ValueError("outside_window")
        raw = stream.read(INPUT_CAP + 1)
        if len(raw) > INPUT_CAP:
            raise ValueError("input_cap")
        record = project(json.loads(raw), state, now)
        if len(json.dumps(record).encode()) > RECORD_BYTES:
            raise ValueError("record_cap")
    except (OSError, ValueError, KeyError, TypeError):
        record = {"error": "observer_failed"}
    # Exclusive slots make concurrent/duplicate invocations visible without
    # an unbounded append log or a lock that could delay the real Stop hook.
    for index in range(RECORD_CAP):
        try:
            with (directory / f"{index}.json").open("x", encoding="utf-8") as output:
                json.dump(record, output)
            return
        except FileExistsError:
            continue
    (directory / "overflow").touch(exist_ok=True)


class Observer:
    def __init__(self, home, probe, relative, *, windows=False):
        # Installation must never reach the signed-in operator's HOME.
        home.resolve().relative_to(probe.resolve())
        self.directory = Path(tempfile.mkdtemp(prefix="journey-stop-", dir=probe))
        self.config = home.joinpath(*relative)
        self.config.parent.mkdir(parents=True, exist_ok=True)
        argv = [sys.executable, str(Path(__file__).resolve()), str(self.directory)]
        command = ("& " + " ".join("'" + word.replace("'", "''") + "'" for word in argv)
                   if windows else shlex.join(argv))
        with self.config.open("x", encoding="utf-8") as output:
            json.dump({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": command,
                                                      "timeout": HOOK_SECONDS}]}]}}, output)

    def arm(self, held, token, seconds):
        now = time.time_ns() // 1_000_000
        with (self.directory / "armed.json").open("x", encoding="utf-8") as output:
            json.dump({"held": held, "token": token, "start": now, "end": now + seconds * 1000}, output)

    def read(self):
        if (self.directory / "overflow").exists():
            return [{"error": "observer_overflow"}]
        records = []
        for index in range(RECORD_CAP):
            path = self.directory / f"{index}.json"
            if path.exists():
                try:
                    with path.open("rb") as source:
                        records.append(json.loads(source.read(RECORD_BYTES + 1)))
                except (OSError, ValueError):
                    return [{"error": "observer_unreadable"}]
        return records

    def close(self):
        (self.directory / "armed.json").unlink(missing_ok=True)
        self.config.unlink(missing_ok=True)
        # Metadata remains in the disposable probe until runtime teardown;
        # Journey copies it to the cell's owned evidence before this close.


if __name__ == "__main__":
    try:
        observe(Path(sys.argv[1]), sys.stdin.buffer)
    except Exception:
        # A write failure leaves missing evidence, which the caller fails.
        # The observer must never block Stop or emit model feedback.
        pass
