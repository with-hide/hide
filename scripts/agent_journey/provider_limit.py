"""A turn the agent's provider refused, read where the agent records it, so
the cell settles `not measured` with the provider's reason instead of `fail`
(with-hide/hide-private#38).

Codex CLI records a refused sampling request only in its own log database in
CODEX_HOME (`logs_<schema>.sqlite`, table `logs`): its session file keeps the
prompt and nothing after it while Codex waits to retry, and its pane reads
Working. On 2026-10-26 (codex-cli 0.164.0) OpenAI refused the sixth turn of
a few minutes on the operator's account, and Codex logged, under the target
`codex_core::responses_retry`, `stream disconnected - retrying sampling
request (1/5 in 500.99994175s) ... last status: 429 Too Many Requests`, so the
cell's 3-minute wait ended with no answer.
"""

import contextlib
from pathlib import Path
import re
import sqlite3

RETRY_TARGET = "codex_core::responses_retry"
TOO_MANY_REQUESTS = "429 Too Many Requests"
LOGS = re.compile(r"logs_(\d+)\.sqlite")
RETRY = re.compile(r"retrying sampling request \((\d+)/(\d+) in ([\d.]+)(ms|s)\)")


class Unreadable(Exception):
    """Codex's log database is there and could not be read."""


def codex_logs(codex_home):
    """Codex's log database under `codex_home`, the newest schema's, or None."""
    found = []
    for path in Path(codex_home).glob("logs_*.sqlite"):
        match = LOGS.fullmatch(path.name)
        if match and path.is_file() and not path.is_symlink():
            found.append((int(match.group(1)), path))
    return max(found)[1] if found else None


def codex_refusal(codex_home, since):
    """Why Codex got no answer from its provider for a turn from `since`
    (Unix seconds) on: the newest retry Codex logged after a 429, or None
    when it logged none. Raises Unreadable when the database cannot be read,
    which leaves the cell's own verdict standing."""
    logs = codex_logs(codex_home)
    if logs is None:
        return None
    try:
        with contextlib.closing(sqlite3.connect(f"file:{logs}?mode=ro", uri=True, timeout=2)) as db:
            row = db.execute(
                "SELECT feedback_log_body FROM logs WHERE target = ? AND ts >= ? AND instr(feedback_log_body, ?) > 0"
                " ORDER BY ts DESC, id DESC LIMIT 1",
                (RETRY_TARGET, int(since), TOO_MANY_REQUESTS)).fetchone()
    except sqlite3.Error as error:
        raise Unreadable(f"{logs.name}: {error}") from error
    if row is None:
        return None
    retry = RETRY.search(row[0] or "")
    wait = ""
    if retry:
        seconds = float(retry.group(3)) / (1000 if retry.group(4) == "ms" else 1)
        wait = f", retry {retry.group(1)} of {retry.group(2)} in {seconds:.0f} s"
    return f"provider rate limit: Codex's sampling request was refused with {TOO_MANY_REQUESTS}{wait}"
