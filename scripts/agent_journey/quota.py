"""What the provider did to a CI login run: whether a failed turn was a
spent daily quota, and the number of model requests the run made (PRD
agent-journey-check D-48, B29).

The answer that means a key's requests for the day are spent is data every
signed-in check reads, `contracts/provider-quota.json` (docs/TESTING.md, A
real agent on CI). A provider's row comes with a reader of its agent's own
structured error report, never its screen, added only once a recorded
refusal shows that report; Claude Code and Codex have none yet, so every
refusal of theirs is a failure.
"""

import json
from pathlib import Path
import re
import sqlite3

from . import opencode_service

CONTRACT = Path(__file__).resolve().parents[2] / "contracts" / "provider-quota.json"

AGY_ERROR = "AGY_ERROR: "
# agy's own logs, under the HOME it ran in (the run's disposable one on CI).
AGY_LOGS = ".gemini/antigravity-cli/log"
# The first line of a record in agy's log (glog): severity, date, time,
# thread, then the source file and line; a message's own line breaks
# continue the record on the lines after it.
GLOG = re.compile(r"^[IWEF]\d{4} \d\d:\d\d:\d\d\.\d+ +\d+ ([^:\]]+):\d+\] ")
# The error an interactive agy reports in its log (`errorreport.go`) when a
# turn fails on the provider: the Gemini SDK's API error, whose code and
# status the print mode's `AGY_ERROR:` report carries as fields.
SDK_ERROR = re.compile(r"Error (\d+), Message: .*, Status: ([A-Z_]+), Details: ", re.S)


def agy_report(line):
    """The JSON error report of one `AGY_ERROR:` line agy wrote, or None."""
    at = line.find(AGY_ERROR)
    if at < 0:
        return None
    try:
        report = json.loads(line[at + len(AGY_ERROR):])
    except ValueError:
        return None
    return report if isinstance(report, dict) else None


def log_records(text):
    """The records of one agy log, each its first line and the lines its message continues on."""
    found = []
    for line in text.splitlines():
        if GLOG.match(line) or not found:
            found.append(line)
        else:
            found[-1] += "\n" + line
    return found


def agy_reports(record):
    """The error reports one record of agy's log carries: an `AGY_ERROR:`
    line's JSON, or an interactive turn's `errorreport.go` error read into
    the same fields (`error_code`, `status`, `short_error`)."""
    reports = [report for report in map(agy_report, record.splitlines()) if report]
    header = GLOG.match(record)
    if record.startswith("E") and header and header.group(1) == "errorreport.go":
        message = record[header.end():]
        sdk = SDK_ERROR.search(message)
        if sdk:
            reports.append({"error_code": int(sdk.group(1)), "status": sdk.group(2), "short_error": message})
    return reports


def gemini_quotas(report, row):
    """The quota ids of a Gemini refusal that spent the key's day, as
    `hide-agent-hooks/tests/it/agy_binary.rs` reads them: the report carries
    the row's code and status, and every `google.rpc.QuotaFailure`
    violation names one of the row's quota ids with a value above 0 (the API
    leaves out a value of 0); otherwise None."""
    if report.get("error_code") != row["error_code"] or report.get("status") != row["status"]:
        return None
    _, marker, failure = str(report.get("short_error") or "").partition("google.rpc.QuotaFailure violations:")
    if not marker:
        return None
    quotas = []
    for violation in failure.split("quotaId:")[1:]:
        fields = violation.split("]", 1)[0].split()
        value = next((field.removeprefix("quotaValue:") for field in fields[1:] if field.startswith("quotaValue:")), None)
        if not fields or value is None or not value.isdigit() or int(value) == 0 or fields[0] not in row["spent_quota_ids"]:
            return None
        quotas.append(fields[0])
    return quotas or None


def logs_since(home, since):
    """agy's own logs under `home` written at or after `since` (Unix
    seconds), oldest first: an older one is another run's, whose refusal says
    nothing of this turn."""
    if not home:
        return []
    written = [(log.stat().st_mtime, log) for log in (home / AGY_LOGS).glob("*.log")]
    return [log for mtime, log in sorted(written) if mtime >= since]


def gemini_spent(paths, row, home, since):
    """agy's turn that failed got no model answer (no `PLANNER_RESPONSE`
    after its last `USER_INPUT` in the transcripts Hide's reader read), and
    the newest error report in agy's own logs is a spent day. The log is
    where an interactive agy can leave the `AGY_ERROR:` report its print
    mode writes to stderr, and where an interactive agy's `errorreport.go`
    record carries the same error (2026-10-18, run 39097738256)."""
    for path in paths:
        steps = [record.get("type") for record in records(path)]
        last_input = max((index for index, step in enumerate(steps) if step == "USER_INPUT"), default=-1)
        if "PLANNER_RESPONSE" in steps[last_input + 1:]:
            return None
    for log in reversed(logs_since(home, since)):
        try:
            lines = log.read_text(errors="replace").splitlines()
        except OSError:
            continue
        reports = [report for record in log_records("\n".join(lines)) for report in agy_reports(record)]
        if reports:
            return gemini_quotas(reports[-1], row)
    return None


# How many of the agent's own error records a failed turn keeps, and how much of each.
KEPT_RECORDS = 20
KEPT_RECORD_CHARS = 2000


def agy_records(paths, home, since):
    """agy's own records of a failed turn, kept as its evidence so the next
    refusal shows where an interactive agy reports it: each step its
    transcripts hold after their last `USER_INPUT` (its type, status and
    error, never its content), and the error records (glog `E`) of its
    newest logs."""
    steps = []
    for path in paths:
        found = records(path)
        last_input = max((index for index, record in enumerate(found) if record.get("type") == "USER_INPUT"), default=-1)
        steps.extend({"type": record.get("type"), "status": record.get("status"),
                      "error": str(record["error"])[:KEPT_RECORD_CHARS] if "error" in record else None}
                     for record in found[last_input + 1:])
    logs = logs_since(home, since)[-3:]
    lines = []
    for log in logs:
        try:
            text = log.read_text(errors="replace")
        except OSError:
            continue
        lines.extend({"log": log.name, "record": record[:KEPT_RECORD_CHARS]} for record in log_records(text)
                     if record.startswith("E") or AGY_ERROR in record)
    return {"transcript_steps": steps[-KEPT_RECORDS:], "log_errors": lines[-KEPT_RECORDS:]}


# Provider -> reader(session paths, row, the agent's HOME, when its journey
# began): the spent quota
# ids when the turn that failed got no model answer and ended on the provider
# refusing quotas the row names, each above 0; otherwise None.
READERS = {"gemini": gemini_spent}
# Provider -> the agent's own records of a failed turn (session paths, HOME, when its journey began).
RECORDS = {"gemini": agy_records}


def contract(path=CONTRACT):
    """The provider rows, checked for the shape `contracts/README.md` gives them."""
    try:
        rows = json.loads(Path(path).read_text())
    except (OSError, ValueError) as error:
        raise ValueError(f"provider_quota_contract_unreadable: {path}: {error}") from error
    if not isinstance(rows, dict) or not rows:
        raise ValueError(f"provider_quota_contract_malformed: {path}: not an object of provider rows")
    for name, row in rows.items():
        shaped = (isinstance(row, dict) and isinstance(row.get("agents"), list) and row["agents"]
                  and all(isinstance(agent, str) for agent in row["agents"])
                  and isinstance(row.get("error_code"), int) and isinstance(row.get("status"), str)
                  and isinstance(row.get("spent_quota_ids"), list) and row["spent_quota_ids"]
                  and all(isinstance(quota, str) for quota in row["spent_quota_ids"]))
        if not shaped:
            raise ValueError(f"provider_quota_contract_malformed: {path}: row {name}")
    return rows


def spent_quota(agent, paths, rows, home, since):
    """The spent quota ids that leave the agent's failed turn unmeasured, or None."""
    name = next((name for name, row in rows.items() if agent in row["agents"]), None)
    if name is None:
        return None
    if name not in READERS:
        raise ValueError(f"provider_quota_reader_missing: {name} names {agent}, and the journey cannot read its refusals")
    return READERS[name](paths, rows[name], home, since)


def refusal_records(agent, paths, rows, home, since):
    """The agent's own records of a failed turn its provider row reads, or None."""
    name = next((name for name, row in rows.items() if agent in row["agents"]), None)
    return RECORDS[name](paths, home, since) if name in RECORDS else None


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
