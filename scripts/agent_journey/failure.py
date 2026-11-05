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
# of a run keeps together; hided's whole output is capped at 1 MiB
# (agent_live_check.processes.MAX_OUTPUT), so one cell's step rarely nears it.
CELL_BYTES = 256 * 1024
RUN_BYTES = 4 * 1024 * 1024
# Any account's home in a path, on macOS, Linux or Windows in either case,
# with a JSON string's doubled backslashes or a short name such as RUNNER~1;
# a name with a space is only found as the running account's own home
# (`account_homes`). Each prefix is split so this file names no home itself.
SEPARATOR = r"(?:\\+|/)"
OTHER_HOME = r"(?:/Users" r"/|/home" r"/|\b[A-Za-z]:" + SEPARATOR + "Users" + SEPARATOR + r")[^/\\\s\"':]+"


def now_ms():
    return int(time.time() * 1000)


def moment(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat(timespec="milliseconds")


def account_homes():
    """The home of the account running the journey, as each variable and the
    system name it: the one home whose name can hold a space."""
    values = {os.environ.get("HOME"), os.environ.get("USERPROFILE"), str(Path.home())}
    return tuple(sorted((value for value in values if value and value.strip("/\\")), key=len, reverse=True))


@functools.lru_cache(maxsize=4)
def homes_pattern(homes):
    def written(home):
        # Every separator as a path or a JSON string may write it.
        parts = [re.escape(part) for part in re.split(r"[\\/]+", home) if part]
        return (SEPARATOR if home[:1] in "/\\" else "") + SEPARATOR.join(parts) + r"(?![\w.-])"
    return re.compile("|".join([*map(written, homes), OTHER_HOME]), re.IGNORECASE)


def scrub(text):
    """`text` with every account home written as `~` (scripts/check-no-workstation-identity.py)."""
    return homes_pattern(account_homes()).sub("~", text)


class FailureLogs:
    """hided's output, read where the runtime drains it, kept per failed cell."""

    def __init__(self, output, *, cell_bytes=CELL_BYTES, run_bytes=RUN_BYTES):
        self.output, self.cell_bytes, self.left = output, cell_bytes, run_bytes
        self.run_bytes = run_bytes

    def mark(self):
        """Where a step begins: the start of the line hided is writing, so
        what a failed cell keeps starts on a whole line."""
        return self.output.line_start()

    def keep(self, path, mark, step, cell):
        """Writes the whole lines hided wrote since `mark`, where `step`
        began, to `path`, its home paths written as `~`, under one line saying
        what it holds; past the cell's cap or what is left of the run's, the
        file, that line included, keeps the last whole lines that fit and says
        it was cut. Returns the file's name, or None when the run's budget has
        no room even for that line."""
        data = self.output.since(mark)
        # A line still being written may stop inside a home or a character.
        data = data[:data.rfind(b"\n") + 1]
        lines = scrub(data.decode("utf-8", errors="replace")).encode().splitlines(keepends=True)
        cap = min(self.cell_bytes, self.left)
        head = (f"journey: hided's output from step {step['step']} (began {moment(step['started_ms'])}) "
                f"to the failure of cell {cell} ({moment(now_ms())}), {len(data)} bytes of whole lines")
        text = (head + "\n").encode() + b"".join(lines)
        if len(text) > cap:
            limit = (f"the run's {self.run_bytes}-byte budget for failed cells' logs, {cap} bytes of it left"
                     if cap < self.cell_bytes else f"the cell's {cap}-byte cap")
            text = (head + f"; cut to its last whole lines at {limit}\n").encode()
            if len(text) > cap:
                return None
            room, kept = cap - len(text), []
            for line in reversed(lines):
                if len(line) > room:
                    break
                kept.append(line)
                room -= len(line)
            text += b"".join(reversed(kept))
        self.left -= len(text)
        path.unlink(missing_ok=True)
        write_private(path, text)
        return path.name


def drop_on_pass(run, code, interrupted):
    """A run nothing failed in keeps no `hided.log`: exit 0, or 3 for a cell
    not measured (every stand-in run has some) when the run was not stopped.
    A failed cell, setup, cleanup or budget, or a stop, keeps it to be read."""
    if code == 0 or (code == 3 and not interrupted):
        (run / "hided.log").unlink(missing_ok=True)
