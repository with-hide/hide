"""The number of model requests an agent's sessions made, which a run
compares with the agent's request budget (PRD agent-journey-check B29).
"""

import json
from pathlib import Path
import sqlite3

from . import opencode_service


def records(path):
    """The session file's JSON records; a line that is not one is skipped,
    as a reader of a file still being written must."""
    try:
        with open(path, encoding="utf-8", errors="replace") as stream:
            lines = stream.read().splitlines()
    except OSError:
        return []
    found = []
    for line in lines:
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if isinstance(value, dict):
            found.append(value)
    return found


def opencode_responses(home, path):
    """The assistant messages OpenCode 2 stored for the session `path` names
    (`opencode/<id>`, as Hide's reader names it), one per model response,
    read from its database under the probe's home."""
    database = Path(home) / opencode_service.DATA / "opencode.db"
    if not database.is_file():
        return 0
    connection = sqlite3.connect(f"{database.as_uri()}?mode=ro", uri=True, timeout=5)
    try:
        return connection.execute("SELECT count(*) FROM session_message WHERE session_id = ? AND type = 'assistant'",
                                  (path.removeprefix("opencode/"),)).fetchone()[0]
    finally:
        connection.close()


def model_requests(kind, paths, home=None):
    """Model requests an agent's sessions made: Claude Code writes one
    assistant record per response, several for one response sharing its
    message id, and a forked session repeats the responses it copied under
    their ids, so each id counts once across `paths`; Codex writes one token
    count event per response, and a forked rollout repeats none of its
    source's; Grok counts its model calls on each prompt's `turn_completed`
    usage, each prompt once across `paths`; Pi and omp write one assistant
    message per response; agy one `PLANNER_RESPONSE` step; OpenCode one
    assistant message per response in its database under `home`."""
    if kind == "opencode":
        return sum(opencode_responses(home, path) for path in paths)
    if kind == "grok":
        calls = {}
        for path in paths:
            for record in records(path):
                params = record.get("params") if isinstance(record.get("params"), dict) else {}
                update = params.get("update")
                if isinstance(update, dict) and update.get("sessionUpdate") == "turn_completed":
                    usage = update.get("usage") if isinstance(update.get("usage"), dict) else {}
                    if isinstance(usage.get("modelCalls"), int):
                        calls[update.get("prompt_id")] = usage["modelCalls"]
        return sum(calls.values())
    if kind in ("pi", "omp"):
        return sum(1 for path in paths for record in records(path) if record.get("type") == "message"
                   and isinstance(record.get("message"), dict) and record["message"].get("role") == "assistant")
    if kind == "agy":
        return sum(1 for path in paths for record in records(path) if record.get("type") == "PLANNER_RESPONSE")
    if kind == "claude":
        ids = set()
        for path in paths:
            for record in records(path):
                message = record.get("message") if isinstance(record.get("message"), dict) else {}
                if record.get("type") == "assistant" and record.get("isApiErrorMessage") is not True and message.get("id"):
                    ids.add(message["id"])
        return len(ids)
    if kind == "codex":
        return sum(1 for path in paths for record in records(path) if record.get("type") == "event_msg"
                   and isinstance(record.get("payload"), dict) and record["payload"].get("type") == "token_count")
    raise ValueError("no_request_count_for:" + kind)
