"""What a failed cell keeps for the person reading why it failed.

hided's own output reaches the run folder as `hided.log` only when the run
closes, so a job killed before its close kept none. A failed cell instead
keeps, the moment it fails, what hided wrote while its step ran, as
`<agent>-<cell>-hided.log` beside its pane capture, under a cap per cell and
one for the run; the step times are the agent's `steps` in the report
(with-hide/hide-private#548). A run that passed keeps no `hided.log`.
"""

from datetime import datetime, timezone
import re
import time

from agent_live_check.protection import write_private

# What one failed cell keeps of hided's output, and what every failed cell
# of a run keeps together; hided's whole output is capped at 1 MiB
# (agent_live_check.processes.MAX_OUTPUT), so one cell's step rarely nears it.
CELL_BYTES = 256 * 1024
RUN_BYTES = 4 * 1024 * 1024
# An account's home in a path, on macOS, Linux or Windows, with a JSON
# string's doubled backslashes or a short name such as RUNNER~1 included:
# what the run keeps names no account (scripts/check-no-workstation-identity.py).
# Each home's prefix is split so this file names no home itself.
HOME = re.compile(r"(?:/Users" r"/|/home" r"/|\b[A-Za-z]:(?:\\+|/)Users(?:\\+|/))[^/\\\s\"':]+")


def now_ms():
    return int(time.time() * 1000)


def moment(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).isoformat(timespec="milliseconds")


def scrub(text):
    """`text` with every account home written as `~`."""
    return HOME.sub("~", text)


class FailureLogs:
    """hided's output, read where the runtime drains it, kept per failed cell."""

    def __init__(self, output, *, cell_bytes=CELL_BYTES, run_bytes=RUN_BYTES):
        self.output, self.cell_bytes, self.left = output, cell_bytes, run_bytes
        self.run_bytes = run_bytes

    def mark(self):
        return self.output.mark()

    def keep(self, path, mark, step, cell):
        """Writes what hided wrote since `mark`, where `step` began, to `path`:
        its last whole lines within the caps, under one line saying what it
        holds and whether it was cut. Returns the file's name."""
        data = self.output.since(mark)
        cap = min(self.cell_bytes, self.left)
        header = (f"journey: hided's output from step {step['step']} (began {moment(step['started_ms'])}) "
                  f"to the failure of cell {cell} ({moment(now_ms())}), {len(data)} bytes")
        if len(data) > cap:
            kept = data[len(data) - cap:] if cap else b""
            # From the first whole line: the cut lands inside one.
            kept = kept[kept.index(b"\n") + 1:] if b"\n" in kept else b""
            header += (f"; cut: the run's {self.run_bytes}-byte budget for failed cells' logs had {cap} bytes left"
                       if cap < self.cell_bytes else f"; cut at the cell's {cap}-byte cap")
            header += f", so it keeps its last {len(kept)} bytes of whole lines"
            data = kept
        self.left -= len(data)
        path.unlink(missing_ok=True)
        write_private(path, (header + "\n" + scrub(data.decode("utf-8", errors="replace"))).encode())
        return path.name


def drop_on_pass(run, code):
    """A run that passed (exit 0) keeps no `hided.log`: only a run that did
    not pass is read for why."""
    if code == 0:
        (run / "hided.log").unlink(missing_ok=True)
