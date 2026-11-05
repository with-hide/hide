"""What a failed cell keeps for the person reading why it failed.

hided's own output reaches the run folder as `hided.log` only when the run
closes, so a job killed before its close kept none. A failed cell instead
keeps, the moment it fails, what hided wrote while its step ran, as
`<agent>-<cell>-hided.log` beside its pane capture, under a cap per cell and
one for the run; the step times are the agent's `steps` in the report
(with-hide/hide-private#548). A run nothing failed in keeps no `hided.log`.
"""

from datetime import datetime, timezone
import functools
import os
from pathlib import Path
import re
import time

from agent_live_check.protection import write_private

# What one failed cell keeps of hided's output, and what every failed cell
# of a run keeps together past each one's tail; hided's whole output is
# capped at 1 MiB (agent_live_check.processes.MAX_OUTPUT), so one cell's step
# rarely nears it. Every failed cell keeps its last `FLOOR_BYTES` whatever
# the run spent, so a run keeps at most RUN_BYTES + FLOOR_BYTES a failed cell,
# and its cells are counted.
CELL_BYTES = 256 * 1024
RUN_BYTES = 4 * 1024 * 1024
FLOOR_BYTES = 16 * 1024
# A cell's first line is a few hundred bytes; a tail has room for it.
FIRST_LINE_ROOM = 1024
# A home is a path's segment, whatever its name holds (spaces included):
# the one after `Users` or `home` that follows a separator or a drive, up to
# the next separator, quote or line end, in either case and as a JSON string
# doubles a backslash (scripts/check-no-workstation-identity.py). The running
# account's own home, wherever it lies, is the whole value its variables give,
# where a separator, quote or line end follows it. Each prefix is split so
# this file names no home itself.
SEPARATOR = r"(?:\\+|/)"
SEGMENT_END = r"(?=[\\/\"']|$)"
OTHER_HOME = r"(?:\b[A-Za-z]:)?" + SEPARATOR + r"(?:Users|home)" + SEPARATOR + r"[^\\/\"'\r\n]+"


def now_ms():
    return int(time.time() * 1000)


def moment(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat(timespec="milliseconds")


def account_homes():
    """The home of the account running the journey, as each variable and the
    system name it."""
    values = {os.environ.get("HOME"), os.environ.get("USERPROFILE"), str(Path.home())}
    return tuple(sorted((value for value in values if value and value.strip("/\\")), key=len, reverse=True))


@functools.lru_cache(maxsize=4)
def homes_pattern(homes):
    def written(home):
        # Every separator as a path or a JSON string may write it.
        parts = [re.escape(part) for part in re.split(r"[\\/]+", home) if part]
        return (SEPARATOR if home[:1] in "/\\" else "") + SEPARATOR.join(parts) + SEGMENT_END
    return re.compile("|".join([*map(written, homes), OTHER_HOME]), re.IGNORECASE | re.MULTILINE)


def scrub(text):
    """`text` with every account home written as `~`."""
    return homes_pattern(account_homes()).sub("~", text)


class FailureLogs:
    """hided's output, read where the runtime drains it, kept per failed cell."""

    def __init__(self, output, *, cell_bytes=CELL_BYTES, run_bytes=RUN_BYTES, floor_bytes=FLOOR_BYTES):
        if not FIRST_LINE_ROOM <= floor_bytes <= cell_bytes:
            raise ValueError("failure_log_caps_refused: a cell's tail holds its first line and fits its cap")
        self.output, self.cell_bytes, self.run_bytes, self.floor = output, cell_bytes, run_bytes, floor_bytes
        self.left = run_bytes

    def mark(self):
        """Where a step begins: the start of the line hided is writing, so
        what a failed cell keeps starts on a whole line."""
        return self.output.line_start()

    def keep(self, path, mark, step, cell):
        """Writes the whole lines hided wrote since `mark`, where `step`
        began, to `path`, its home paths written as `~`, under one line saying
        what it holds. Past the cell's cap, or past what the run's budget has
        left when that is more than the cell's tail, the file, that line
        included, keeps the last whole lines that fit and its first line names
        the cap. Returns the file's name and that cap, or None."""
        data = self.output.since(mark)
        # A line still being written may stop inside a home or a character.
        data = data[:data.rfind(b"\n") + 1]
        lines = scrub(data.decode("utf-8", errors="replace")).encode().splitlines(keepends=True)
        cap = min(self.cell_bytes, max(self.floor, self.left))
        head = (f"journey: hided's output from step {step['step']} (began {moment(step['started_ms'])}) "
                f"to the failure of cell {cell} ({moment(now_ms())}), {len(data)} bytes of whole lines")
        text, limit = (head + "\n").encode() + b"".join(lines), None
        if len(text) > cap:
            limit = (f"the cell's {cap}-byte cap" if cap == self.cell_bytes else
                     f"the run's {self.run_bytes}-byte budget for failed cells' logs, spent but for "
                     f"this cell's {cap}-byte tail" if cap == self.floor else
                     f"the {cap} bytes the run's {self.run_bytes}-byte budget for failed cells' logs had left")
            text = (head + f"; cut to its last whole lines at {limit}\n").encode()
            room, kept = cap - len(text), []
            for line in reversed(lines):
                if len(line) > room:
                    break
                kept.append(line)
                room -= len(line)
            text += b"".join(reversed(kept))
        self.left = max(0, self.left - len(text))
        path.unlink(missing_ok=True)
        write_private(path, text)
        return path.name, limit


def drop_on_pass(run, code, interrupted):
    """A run nothing failed in keeps no `hided.log`: exit 0, or 3 for a cell
    not measured (every stand-in run has some) when the run was not stopped.
    A failed cell, setup, cleanup or budget, or a stop, keeps it to be read."""
    if code == 0 or (code == 3 and not interrupted):
        (run / "hided.log").unlink(missing_ok=True)
