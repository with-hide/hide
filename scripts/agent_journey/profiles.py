"""The `profile` cell's accounts: one profile per measured agent, in an
account of its own under the run's HOME, pointed at the agent's fake model
(fake_lane.profile_account) and added to the candidate's profile file the way
Settings adds one, through the node's `profile_apply` (PRD agent-profiles
B19), before the daemon starts, so its first kit pass puts Hide's pieces in
those accounts as it does in the default ones.
"""

import json

from agent_live_check.processes import COMMAND_SECONDS, ProcessError
from agent_live_check.protection import write_private

from . import cells, fake_lane


def prepare(runtime, table, rows, ready, root, urls, key):
    """Each ready agent whose `profile` cell is open gets its account and its
    profile; answers {agent id: the account (fake_lane.profile_account) with
    the profile's `name`}. A profile the node refuses settles the cell with
    the node's reason."""
    accounts = {}
    for agent in ready:
        if table[agent]["profile"]["status"] != cells.PENDING:
            continue
        account = fake_lane.profile_account(rows[agent]["herdr_kind"], runtime.home, urls[agent], root, key)
        try:
            added = add(runtime, agent, account["account"])
        except ProcessError as error:
            cells.settle(table, agent, "profile", cells.FAIL, "the node did not add the profile: " + str(error))
            continue
        accounts[agent] = {**account, "name": fake_lane.PROFILE_NAME, "id": added}
    return accounts


def add(runtime, agent, account):
    """Adds the profile `fake_lane.PROFILE_NAME` of `agent` in `account`
    through the candidate's node, against the list's version as the node
    answers it; answers the new profile's id."""
    listed = call(runtime, agent, "profile_list", {})
    applied = call(runtime, agent, "profile_apply", {
        "change": {"op": "add", "kind": agent, "name": fake_lane.PROFILE_NAME, "account": account},
        "expected_version": listed["version"]})
    if applied.get("refused") or not applied.get("added"):
        raise ProcessError("profile_refused: " + json.dumps(applied.get("refused"))[:300])
    return applied["added"]


def call(runtime, agent, op, fields):
    """One call to a `hided node serve` of the candidate's state, after its
    `hello`, as the daemon's own node answers it; its answer."""
    requests = runtime.run / f"profile-{agent}-{op}.jsonl"
    write_private(requests, "".join(json.dumps(line) + "\n" for line in (
        {"id": 1, "op": "hello"}, {"id": 2, "op": op, **fields})).encode())
    with requests.open("rb") as stdin:
        _, out, err = runtime.owner.run([str(runtime.hided), "node", "serve"], env=runtime.daemon_env,
                                        cwd=runtime.home, seconds=COMMAND_SECONDS, stdin=stdin)
    answers = {}
    for line in out.splitlines():
        try:
            answer = json.loads(line)
        except ValueError:
            continue
        if isinstance(answer, dict) and "id" in answer:
            answers[answer["id"]] = answer
    answer = answers.get(2)
    if not answer or "ok" not in answer:
        raise ProcessError(f"node_{op}_failed: " + json.dumps(answer or err.strip()[-300:])[:400])
    return answer["ok"]
