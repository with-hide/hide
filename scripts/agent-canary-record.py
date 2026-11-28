#!/usr/bin/env python3
"""The agent canary's record: whether each agent CLI's latest release passed
Hide's agent journey on each system, kept in one GitHub issue, and an issue
for each agent whose latest release fails (the `agent canary` jobs of
`.github/workflows/nightly.yml`).

Usage:
  agent-canary-record.py --versions <latest.json> --systems <JSON> --reports <dir>
                         --run-url <url> --commit <sha> [--write]

The nightly runs the fake model journey twice on the same commit: its
`agent journey` at the pins (`agent-journey-fake-<system>`) and the canary at
each CLI's latest release (`agent-canary-latest-<system>`), whose reports sit
under `<dir>` one folder per artifact. Each agent gets a verdict on each
system the canary measured: `pass` when every cell its declaration expects
came out as declared; `fail` for a miss its pin does not repeat, which is the
release's, so the agent gets its own issue, or a comment on the open one, and
an agent that passes again closes it; `hide` for a miss its pin repeats,
which is Hide's and which the nightly's report files from `agent journey`;
and `unknown` when a report is missing, which proves nothing either way. The
record's state rides in the issue body as one JSON comment, so the next run
reads what this one measured without a commit to main. Every issue lives in
the public repository (`ISSUE_REPOSITORY`) with the other development
issues, under the `agent-canary` label. Without `--write`, as on a run of
another branch, the record is read and nothing is written: the table and the
issue actions it would take go to the job summary and standard output.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey.cells import CELLS  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402
from agent_journey.scene import lane_status  # noqa: E402

# Where the record and every failure issue live.
ISSUE_REPOSITORY = "with-hide/hide"
LABEL = "agent-canary"
# The record, "Agent CLI support record", is this one issue, found by its number: a title can be edited or
# repeated. scripts/check-nightly-green.py exempts the same number.
RECORD_ISSUE = 2552
STATE = "<!-- agent-canary-state "
# The systems the canary can measure; Windows runs no journey while a holder
# cannot leave the CI job (docs/INSTALL.md). A run names the ones it measured.
SYSTEMS = ("linux", "macos")
# The artifact names of the latest releases' reports and the pins'.
LATEST = "agent-canary-latest"
PINNED = "agent-journey-fake"
MARK = {"pass": "✅", "fail": "❌", "unknown": "❔", "hide": "⚠️ Hide"}
# An agent's verdict is its worst system's, in this order.
WORST = ("fail", "unknown", "hide", "pass")


def read_state(body):
    """The JSON the last run left in the record issue's body, or an empty record."""
    start = (body or "").find(STATE)
    if start < 0:
        return {"agents": {}}
    end = body.find(" -->", start)
    try:
        state = json.loads(body[start + len(STATE):end])
    except ValueError:
        return {"agents": {}}
    return state if isinstance(state, dict) and isinstance(state.get("agents"), dict) else {"agents": {}}


def misses(row, report, system):
    """What one system's report measured short of the agent's declaration:
    `(what, head, reason)` triples, `what` being a cell's name, `install` for
    a CLI that is missing or answered another version than the table, or
    `row` for a report with no row for the agent; `head` says which and how,
    and `reason`, which can name a run's own token, why."""
    agent = row["id"]
    found = []
    entry = next((item for item in report.get("agents", []) if item.get("id") == agent), {})
    if entry.get("cli_missing"):
        found.append(("install", f"{system}: {entry['cli_missing']} is not installed", ""))
    pin = entry.get("pin") or {}
    if pin and not pin.get("matches"):
        found.append(("install", f"{system}: answered {entry.get('version')!r}, not {pin.get('version')}", ""))
    cells = report.get("table", {}).get(agent)
    if not isinstance(cells, dict):
        return found + [("row", f"{system}: no row", "")]
    for name in CELLS:
        expected = lane_status(row, name)
        cell = cells.get(name) or {}
        if cell.get("status") != expected:
            found.append((name, f"{system}: {name} is {cell.get('status', 'absent')}",
                          str(cell.get("reason") or "")[:160]))
    return found


def line(head, reason):
    return head + (f" ({reason})" if reason else "")


def pinned_already(row, versions, pins):
    """Whether the latest release the run measured is the agent's pin, so the
    latest run already measured the pinned version."""
    return versions.get(row["agent_cli"]) == pins.get(row["agent_cli"])


def system_verdict(row, system, versions, pins, latest, pinned):
    """One agent on one system: `(status, failures)`, as the module says."""
    if system not in latest:
        return "unknown", [f"{system}: no report"]
    found = misses(row, latest[system], system)
    if not found:
        return "pass", []
    agent, kind = row["id"], row["agent_cli"]
    pin = pins.get(kind)
    if pinned_already(row, versions, pins):
        return "hide", [f"{line(head, reason)} (the pinned {pin} is the latest)" for _, head, reason in found]
    again = pinned.get(system)
    measured = again is not None and agent in again.get("selected", [])
    repeated = {miss[0] for miss in misses(row, again, system)} if measured else set()
    release, hide, unjudged = [], [], []
    for what, head, reason in found:
        if what == "install":
            # The latest release's own install or version answer.
            release.append(line(head, reason))
        elif not measured or "install" in repeated:
            unjudged.append(f"{line(head, reason)}; the pinned {pin} was not measured on {system}")
        elif what in repeated:
            hide.append(f"{line(head, reason)}; the pinned {pin} fails it too")
        else:
            release.append(line(head, reason))
    if release:
        return "fail", release
    if unjudged:
        return "unknown", unjudged
    return "hide", hide


def verdicts(contract, versions, pins, systems, latest, pinned):
    """Each active agent's `{system: (status, failures)}` on the systems this
    run measured."""
    return {row["id"]: {system: system_verdict(row, system, versions, pins, latest, pinned) for system in systems}
            for row in active_rows(contract)}


def updated_state(state, contract, versions, results, today):
    """The record after this run. Each agent keeps one entry per system it was
    ever measured on, `{status, version, failures}`; this run replaces the
    entries of the systems it measured and keeps the others, so a day that
    measures Linux alone says nothing about macOS. An entry for an older
    version than the one checked now is unknown for the agent's verdict: a
    failure there is not cleared until that system passes the current
    version. `checked` and `date` move for every agent, `last_pass` only for
    one whose every entry passes at the current version, and an agent's
    failure issue stays with it until then."""
    agents = {}
    for row in active_rows(contract):
        agent, kind = row["id"], row["agent_cli"]
        version = versions.get(kind)
        before = state["agents"].get(agent, {})
        systems = {system: entry for system, entry in (before.get("systems") or {}).items()
                   if isinstance(entry, dict) and system in SYSTEMS}
        for system, (status, failures) in results[agent].items():
            systems[system] = {"status": status, "version": version, "failures": failures}
        statuses, failures = [], []
        for system, entry in sorted(systems.items()):
            if entry["version"] == version:
                statuses.append(entry["status"])
                failures += entry["failures"]
            else:
                statuses.append("unknown")
                failures += [f"{system}: not measured at {version} since {entry['version']}, which was "
                             f"{entry['status']}"] + [f"{text} (at {entry['version']})" for text in entry["failures"]]
        status = min(statuses, key=WORST.index, default="unknown")
        passed = status == "pass"
        agents[agent] = {
            "kind": kind,
            "checked": version,
            "date": today,
            "status": status,
            "systems": systems,
            "failures": failures,
            "last_pass": version if passed else before.get("last_pass"),
            "last_pass_date": today if passed else before.get("last_pass_date"),
            "issue": None if passed else before.get("issue"),
        }
    return {"agents": agents}


def render(state, pins, minimums, run_url):
    """The record issue's body: the table, then the state for the next run."""
    lines = [
        "Each agent CLI's latest release, measured by the nightly's agent canary on the fake model "
        "(`.github/workflows/nightly.yml`): on Linux every day and on macOS on Monday and Thursday, "
        "beside the same journey at the pins on the same commit. "
        "`Minimum` is the oldest version Hide puts its hook, plugin or extension in place for "
        "(`minimum_version` in `contracts/agent-journey.json`); "
        "`Pinned` is what every other CI lane installs (`contracts/agent-clis.json`). "
        "⚠️ Hide marks a cell the pin fails too, which the nightly files as a `nightly-failure` issue.",
        "",
        "| Agent | Latest checked | Status | Minimum | Last passed | Pinned | Issue |",
        "|---|---|---|---|---|---|---|",
    ]
    for agent, row in sorted(state["agents"].items()):
        last = f"{row['last_pass']} ({row['last_pass_date']})" if row.get("last_pass") else "—"
        status = " · ".join(f"{system} {MARK[entry['status']]}"
                            + ("" if entry["version"] == row["checked"] else f" ({entry['version']})")
                            for system, entry in sorted(row.get("systems", {}).items()))
        issue = f"#{row['issue']}" if row.get("issue") else ""
        lines.append(f"| {agent} | {row['checked']} ({row['date']}) | {status or MARK[row['status']]} | "
                     f"{minimums.get(agent, '—')} | {last} | {pins.get(row['kind'], '—')} | {issue} |")
    failing = [(agent, row) for agent, row in sorted(state["agents"].items()) if row["failures"]]
    if failing:
        lines += ["", "<details><summary>What each agent missed</summary>", ""]
        lines += [f"- {agent} {row['checked']}: {text}" for agent, row in failing for text in row["failures"]]
        lines += ["", "</details>"]
    lines += ["", f"Last run: {run_url}", "",
              STATE + json.dumps(state, separators=(",", ":")) + " -->"]
    return "\n".join(lines) + "\n"


def failure_body(agent, row, run_url, commit, pin):
    return (f"`{agent}` {row['checked']} fails Hide's agent journey on the fake model "
            f"(last passed: {row.get('last_pass') or 'never recorded'}); the pinned {pin} does not fail these "
            f"on the same commit {commit}, so this release is what changed.\n\n"
            + "\n".join(f"- {line}" for line in row["failures"])
            + f"\n\nRun: {run_url}\n\nA fix moves `contracts/agent-clis.json` to this version in the same pull "
            "request, so its CI proves the fix on it.\n")


def actions(state, before, run_url, commit, pins):
    """What this run says on each agent's failure issue, as `(verb, agent,
    gh arguments)`: open one for a release that fails with none open, comment
    when a later release or other cells fail, close it when the agent passes.
    A run that measured nothing new for an agent says nothing on its issue."""
    found = []
    for agent, row in state["agents"].items():
        was = before["agents"].get(agent, {})
        if row["status"] == "fail":
            body = failure_body(agent, row, run_url, commit, pins.get(row["kind"]))
            if not row.get("issue"):
                found.append(("open", agent, ["issue", "create", "--label", LABEL, "--title",
                                              f"agent canary: {agent} {row['checked']} fails the agent journey",
                                              "--body", body]))
            elif (was.get("checked"), was.get("failures")) != (row["checked"], row["failures"]):
                found.append(("comment", agent, ["issue", "comment", str(row["issue"]), "--body", body]))
        elif row["status"] == "pass" and was.get("issue"):
            found.append(("close", agent, ["issue", "close", str(was["issue"]), "--comment",
                                           f"`{agent}` {row['checked']} passes the agent journey again: {run_url}"]))
    return found


def gh(*args, run=subprocess.run):
    done = run(["gh", *args, "--repo", ISSUE_REPOSITORY], capture_output=True, text=True, timeout=120)
    if done.returncode:
        raise SystemExit(f"gh {' '.join(args[:2])} exited {done.returncode}: {done.stderr.strip()[:300]}")
    return done.stdout


def apply(state, found, run=subprocess.run):
    """Writes each action, and records each opened issue's number in `state`."""
    for verb, agent, args in found:
        out = gh(*args, run=run)
        if verb == "open":
            state["agents"][agent]["issue"] = int(out.strip().rstrip("/").rsplit("/", 1)[-1])


def record_issue(run=subprocess.run):
    """The record issue's body; a record that is closed is refused, since the
    release gate exempts its number only while it holds the record."""
    issue = json.loads(gh("issue", "view", str(RECORD_ISSUE), "--json", "state,body", run=run))
    if issue["state"] != "OPEN":
        raise SystemExit(f"the support record #{RECORD_ISSUE} is {issue['state'].lower()}; reopen it")
    return issue["body"]


def load_reports(folder, name):
    """Each system's report of the artifacts named `<name>-<system>`, each
    downloaded into a folder of its name with the report at its root; a
    missing one is no measurement."""
    reports = {}
    for system in SYSTEMS:
        path = folder / f"{name}-{system}" / "report.json"
        if path.is_file():
            reports[system] = json.loads(path.read_text())
    return reports


def summary(text):
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write(text)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--versions", type=Path, required=True)
    parser.add_argument("--systems", required=True, help="the systems the canary measured, as a JSON array")
    parser.add_argument("--reports", type=Path, required=True)
    parser.add_argument("--run-url", default="")
    parser.add_argument("--commit", required=True, help="the commit both journeys measured")
    parser.add_argument("--write", action="store_true", help="write the record and the issues (main's run only)")
    args = parser.parse_args(argv)
    systems = json.loads(args.systems)
    if not systems or not set(systems) <= set(SYSTEMS):
        parser.error(f"--systems names none or one outside {list(SYSTEMS)}: {systems}")
    versions = json.loads(args.versions.read_text())
    contract = load_contract(ROOT)
    pins = {kind: row["version"] for kind, row in
            json.loads((ROOT / "contracts/agent-clis.json").read_text())["clis"].items()}
    state = read_state(record_issue())
    today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    results = verdicts(contract, versions, pins, systems, load_reports(args.reports, LATEST),
                       load_reports(args.reports, PINNED))
    after = updated_state(state, contract, versions, results, today)
    found = actions(after, state, args.run_url, args.commit, pins)
    if args.write:
        gh("label", "create", LABEL, "--force", "--color", "C5DEF5",
           "--description", "The agent CLI canary's record and failures")
        apply(after, found)
    minimums = {row["id"]: row["minimum_version"] for row in active_rows(contract)}
    text = render(after, pins, minimums, args.run_url)
    if args.write:
        gh("issue", "edit", str(RECORD_ISSUE), "--body", text)
    where = f"#{RECORD_ISSUE}"
    done = [f"- {'' if args.write else 'would '}{verb} the issue of {agent}" for verb, agent, _ in found]
    summary(f"### Agent CLI support record ({'written to' if args.write else 'not written; would write'} {where})"
            "\n\n" + text.split(STATE, 1)[0] + "\n" + "\n".join(done or ["- no agent issue to write"]) + "\n")
    if not args.write:
        print(text.split(STATE, 1)[0])
        print("\n".join(done or ["no agent issue to write"]))
    # The job fails on what this run measured; a system it did not measure
    # (macOS on a Linux-only day) waits for its own run.
    failed = {agent: [(system, status, failures) for system, (status, failures) in sorted(per.items())
                      if status != "pass"] for agent, per in results.items()}
    for agent, misses in failed.items():
        for system, status, failures in misses:
            print(f"::error title=agent canary {agent}::{after['agents'][agent]['checked']} on {system} ({status}): "
                  + "; ".join(failures[:5]))
    return 1 if any(failed.values()) else 0


if __name__ == "__main__":
    sys.exit(main())
