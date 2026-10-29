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

Each row names the conversation it belongs to (`thread_id`, the session id
the journey knows the agent by), so only the agent's own conversation is
read, and only rows written since the step began (`cursor`).
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


def cursor(codex_home):
    """Where Codex's log stands now: its database's name and newest row id,
    so a row read later against it is one Codex wrote after this moment,
    whatever its clock says. (None, 0) before Codex made the database.
    Raises Unreadable."""
    logs = codex_logs(codex_home)
    if logs is None:
        return None, 0
    try:
        with contextlib.closing(sqlite3.connect(f"file:{logs}?mode=ro", uri=True, timeout=2)) as db:
            newest = db.execute("SELECT max(id) FROM logs").fetchone()[0]
    except sqlite3.Error as error:
        raise Unreadable(f"{logs.name}: {error}") from error
    return logs.name, newest or 0


def codex_refusal(codex_home, mark, thread):
    """Why Codex got no answer from its provider for the conversation
    `thread` since `mark` (a `cursor`): its newest retry there waits on a
    429, or None when that thread retried nothing since or its newest retry
    had another cause. A database of a newer schema than the mark's is read
    from its start. Raises Unreadable, which leaves the cell's verdict
    standing."""
    logs = codex_logs(codex_home)
    if logs is None:
        return None
    name, after = mark
    if name != logs.name:
        after = 0
    try:
        with contextlib.closing(sqlite3.connect(f"file:{logs}?mode=ro", uri=True, timeout=2)) as db:
            row = db.execute(
                "SELECT feedback_log_body FROM logs WHERE target = ? AND thread_id = ? AND id > ?"
                " ORDER BY id DESC LIMIT 1",
                (RETRY_TARGET, thread, after)).fetchone()
    except sqlite3.Error as error:
        raise Unreadable(f"{logs.name}: {error}") from error
    body = (row or ("",))[0] or ""
    if TOO_MANY_REQUESTS not in body:
        return None
    retry = RETRY.search(body)
    wait = ""
    if retry:
        seconds = float(retry.group(3)) / (1000 if retry.group(4) == "ms" else 1)
        wait = f", retry {retry.group(1)} of {retry.group(2)} in {seconds:.0f} s"
    return f"provider rate limit: Codex's sampling request was refused with {TOO_MANY_REQUESTS}{wait}"
