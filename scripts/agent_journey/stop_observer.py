"""Read-only Stop evidence in the journey's private HOME, never a delivery hook.

The command emits nothing. Its bounded records contain probe token positions,
not assistant text. Missing, malformed or overflowed evidence fails the cell.
"""
import json
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


def contained(path, root):
    """Refuse aliases at every component before creating, reading or removing."""
    path.absolute().relative_to(root)
    path.resolve().relative_to(root)
    for part in (path, *path.parents):
        if part.is_symlink() or (hasattr(part, "is_junction") and part.is_junction()):
            raise ValueError("observer_path_is_link")
        if part == root:
            return
    raise ValueError("observer_path_outside_probe")


def project(payload, state, now):
    if not isinstance(payload, dict):
        raise ValueError("payload_not_object")
    text = payload.get("lastAssistantMessage")
    if not isinstance(text, str):
        raise ValueError("missing_response")
    return {"session": payload.get("sessionId"), "active": payload.get("stopHookActive"),
            "reason": payload.get("reason"), "subagent": bool(payload.get("subagentType")),
            "answers": re.findall("|".join(re.escape(state[key]) for key in ("held", "token")), text),
            "at": now}


def observe(directory, stream):
    """Reserve a failure slot before parsing or reading the payload."""
    contained(directory, directory.parent)
    state_path = directory / "armed.json"
    contained(state_path, directory.parent)
    if not state_path.exists():
        return
    for index in range(RECORD_CAP):
        try:
            output = (directory / f"{index}.json").open("x+", encoding="utf-8")
            break
        except FileExistsError:
            continue
    else:
        (directory / "overflow").touch(exist_ok=True)
        return
    with output:
        # A killed command, parser exception or failed rewrite leaves a
        # reserved incomplete/invalid record, never just the earlier passes.
        json.dump({"error": "observer_incomplete"}, output)
        output.flush()
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
        except Exception:
            record = {"error": "observer_failed"}
        output.seek(0)
        json.dump(record, output)
        output.truncate()
        output.flush()


class Observer:
    def __init__(self, home, probe, relative, *, windows=False):
        from agent_live_check.protection import private_directory, write_private
        self.write_private = write_private
        # Installation must never reach the signed-in operator's HOME.
        self.probe = probe.resolve()
        home = self.probe / home.relative_to(probe)
        contained(home, self.probe)
        self.directory = Path(tempfile.mkdtemp(prefix="journey-stop-", dir=self.probe))
        self.config = home.joinpath(*relative)
        contained(self.config, self.probe)
        private_directory(self.config.parent)
        contained(self.config, self.probe)
        argv = [sys.executable, str(Path(__file__).resolve()), str(self.directory)]
        command = ("& " + " ".join("'" + word.replace("'", "''") + "'" for word in argv)
                   if windows else shlex.join(argv))
        self.write_private(self.config, json.dumps({"hooks": {"Stop": [{"hooks": [
            {"type": "command", "command": command, "timeout": HOOK_SECONDS}]}]}}).encode())
        info = self.config.stat()
        self.identity = (info.st_dev, info.st_ino)

    def arm(self, held, token, seconds):
        contained(self.directory / "armed.json", self.probe)
        now = time.time_ns() // 1_000_000
        self.write_private(self.directory / "armed.json", json.dumps(
            {"held": held, "token": token, "start": now, "end": now + seconds * 1000}).encode())

    def read(self):
        contained(self.directory, self.probe)
        if self.directory.with_suffix(".failed").exists():
            return [{"error": "observer_write_failed"}]
        if (self.directory / "overflow").exists():
            return [{"error": "observer_overflow"}]
        records = []
        for index in range(RECORD_CAP):
            path = self.directory / f"{index}.json"
            contained(path, self.probe)
            if path.exists():
                try:
                    with path.open("rb") as source:
                        records.append(json.loads(source.read(RECORD_BYTES + 1)))
                except (OSError, ValueError):
                    return [{"error": "observer_unreadable"}]
        return records

    def close(self):
        contained(self.directory / "armed.json", self.probe)
        (self.directory / "armed.json").unlink(missing_ok=True)
        contained(self.config, self.probe)
        if self.config.exists():
            info = self.config.stat()
            if (info.st_dev, info.st_ino) != self.identity:
                raise ValueError("observer_config_replaced")
            self.config.unlink()
        # Metadata remains in the disposable probe until runtime teardown;
        # Journey copies it to the cell's owned evidence before this close.


def main(directory, stream):
    try:
        observe(directory, stream)
    except Exception:
        # A reservation failure is recorded separately from the bounded
        # slots. Exit 1 is fail-open in the host, never Stop feedback (exit 2).
        contained(directory.with_suffix(".failed"), directory.parent)
        directory.with_suffix(".failed").touch(exist_ok=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), sys.stdin.buffer))
