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

CONTRACT = Path(__file__).resolve().parents[2] / "contracts" / "provider-quota.json"

# Provider -> reader(session paths, row): the spent quota ids when the turn
# that failed got no model answer and ended on the provider refusing quotas
# the row names, each above 0; otherwise None.
READERS = {}


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


def spent_quota(agent, paths, rows):
    """The spent quota ids that leave the agent's failed turn unmeasured, or None."""
    name = next((name for name, row in rows.items() if agent in row["agents"]), None)
    if name is None:
        return None
    if name not in READERS:
        raise ValueError(f"provider_quota_reader_missing: {name} names {agent}, and the journey cannot read its refusals")
    return READERS[name](paths, rows[name])


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


def model_requests(kind, paths):
    """Model requests an agent's sessions made: Claude Code writes one
    assistant record per response, several for one response sharing its
    message id, and a forked session repeats the responses it copied under
    their ids, so each id counts once across `paths`; Codex writes one token
    count event per response, and a forked rollout repeats none of its
    source's."""
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
