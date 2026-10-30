#!/usr/bin/env python3
"""The daily agent canary's record: whether each agent CLI's latest release
passed Hide's agent journey, kept in one GitHub issue, and an issue for each
agent whose latest release fails (`.github/workflows/agent-canary.yml`).

Usage:
  agent-canary-record.py decide --versions <latest.json>
  agent-canary-record.py reruns --versions <latest.json> --reports <dir>
  agent-canary-record.py record --versions <latest.json> --reports <dir> --run-url <url> --commit <sha>

`decide` prints `changed=true` when a version differs from the one the
record last measured, or the record has none, or the last run left an agent
unjudged or failing for Hide's reason, and `changed=false` otherwise, so a
day on which no vendor released starts no journey. `reruns` prints the
agents whose latest release failed a cell while their pin is another version
(`agents=<id,...>`) and the systems it failed on (`systems=<JSON>`): the
workflow measures those again at their pins, on the same commit. `record`
judges each agent from the fake model lane's reports, one per system under
`<dir>` (the latest releases' `agent-journey-fake-<system>` and the pinned
rerun's `agent-journey-pinned-<system>`), and rewrites the record issue's
table. A miss the pinned version does not repeat is the release's: the agent
gets its own issue, or a comment on the open one, and an agent that passes
again closes it. A miss the pinned version repeats on the same commit is
Hide's, not the release's (with-hide/hide-private#358): it goes on one issue
against Hide, which a run with no such miss closes. A miss with no pinned
measurement leaves the agent `unknown`, blaming no one.
The record's state rides in the issue body as one JSON comment, so the next
run reads what this one measured without a commit to main. Every issue lives in
the private development repository (`ISSUE_REPOSITORY`), not the public one,
whose issues are user bug reports; `--run-url` names the run where it ran.
"""

import argparse
import datetime
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey.cells import CELLS  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402

# Where the record and every failure issue live.
ISSUE_REPOSITORY = "with-hide/hide-private"
LABEL = "agent-canary"
RECORD_TITLE = "Agent CLI support record"
HIDE_TITLE = "agent canary: Hide fails the agent journey with the pinned agent CLIs"
STATE = "<!-- agent-canary-state "
SYSTEMS = ("linux", "macos", "windows")
# The artifact names of the latest releases' reports and the pinned rerun's.
LATEST = "agent-journey-fake"
PINNED = "agent-journey-pinned"
MARK = {"pass": "✅", "fail": "❌", "unknown": "❔", "hide": "⚠️ Hide"}


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


def changed(versions, state):
    """Whether any version differs from what the record last measured, or
    the last run could not judge an agent or found Hide failing: those are
    measured again every day until they pass."""
    rows = state["agents"].values()
    measured = {row.get("kind"): row.get("checked") for row in rows}
    measured["herdr"] = state.get("herdr", {}).get("checked")
    return (any(measured.get(kind) != version for kind, version in versions.items())
            or any(row.get("status") in ("unknown", "hide") for row in rows))


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
        expected = "pass" if row["cells"][name] else "N/A"
        cell = cells.get(name) or {}
        if cell.get("status") != expected:
            found.append((name, f"{system}: {name} is {cell.get('status', 'absent')}",
                          str(cell.get("reason") or "")[:160]))
    return found


def line(head, reason):
    return head + (f" ({reason})" if reason else "")


def latest_misses(contract, reports):
    """Each active agent's misses on the latest releases as `(system, what,
    head, reason)`, or None when a system's report is missing: that run
    proves nothing about the system."""
    if any(system not in reports for system in SYSTEMS):
        return {row["id"]: None for row in active_rows(contract)}
    return {row["id"]: [(system, *miss) for system in SYSTEMS for miss in misses(row, reports[system], system)]
            for row in active_rows(contract)}


def pinned_already(row, versions, pins):
    """Whether the latest release the run measured is the agent's pin, so the
    latest run already measured the pinned version."""
    return versions.get(row["herdr_kind"]) == pins.get(row["herdr_kind"])


def reruns(contract, versions, pins, reports):
    """The agents to measure again at their pins, and the systems to measure
    them on: each agent whose latest release missed something, on a system
    where it did, unless that release is its pin."""
    agents, systems = [], set()
    found = latest_misses(contract, reports)
    for row in active_rows(contract):
        if found[row["id"]] and not pinned_already(row, versions, pins):
            agents.append(row["id"])
            systems.update(miss[0] for miss in found[row["id"]])
    return agents, [system for system in SYSTEMS if system in systems]


def verdicts(contract, versions, pins, reports, pinned):
    """Each active agent's `(status, failures, keys)`: `pass` when every
    system measured it and every cell its declaration expects came out as
    declared; `fail` with the misses its latest release has and its pin,
    measured on the same commit, does not; `hide` with the misses its pin
    repeats, which are Hide's, and their `keys` without the run's reasons;
    `unknown` when a system's report is missing or a miss has no pinned
    measurement to say whose it is."""
    result, found = {}, latest_misses(contract, reports)
    for row in active_rows(contract):
        agent, kind = row["id"], row["herdr_kind"]
        if found[agent] is None:
            result[agent] = ("unknown", [f"{system}: no report" for system in SYSTEMS if system not in reports], [])
            continue
        same, pin = pinned_already(row, versions, pins), pins.get(kind)
        label = f"{agent} {pin} (pinned)" if same else f"{agent} {pin} (pinned; latest {versions.get(kind)})"
        release, hide, keys, unjudged = [], [], [], []
        for system, what, head, reason in found[agent]:
            if not same and what == "install":
                # The latest release's own install or version answer.
                release.append(line(head, reason))
                continue
            if not same:
                again = pinned.get(system)
                measured = again is not None and agent in again.get("selected", [])
                repeated = {miss[0] for miss in misses(row, again, system)} if measured else set()
                if not measured or "install" in repeated:
                    unjudged.append(f"{line(head, reason)}; the pinned {pin} was not measured on {system}")
                    continue
                if what not in repeated:
                    release.append(line(head, reason))
                    continue
            hide.append(f"{label}: {line(head, reason)}")
            keys.append(f"{agent}: {head}")
        if release:
            result[agent] = ("fail", release, [])
        elif unjudged:
            result[agent] = ("unknown", unjudged, [])
        elif hide:
            result[agent] = ("hide", hide, keys)
        else:
            result[agent] = ("pass", [], [])
    return result


def updated_state(state, contract, versions, results, today):
    """The record after this run: `checked` and `date` move for every agent,
    `last_pass` only for one that passed, an agent's failure issue stays with
    it until it passes, and the issue against Hide stays until a run closes it."""
    agents = {}
    for row in active_rows(contract):
        agent, kind = row["id"], row["herdr_kind"]
        before = state["agents"].get(agent, {})
        status, failures, _ = results[agent]
        passed = status == "pass"
        agents[agent] = {
            "kind": kind,
            "checked": versions.get(kind),
            "date": today,
            "status": status,
            "failures": failures,
            "last_pass": versions.get(kind) if passed else before.get("last_pass"),
            "last_pass_date": today if passed else before.get("last_pass_date"),
            "issue": None if passed else before.get("issue"),
        }
    hide = [results[agent] for agent in agents if results[agent][0] == "hide"]
    return {"agents": agents, "herdr": {"checked": versions.get("herdr"), "date": today},
            "hide": {"issue": state.get("hide", {}).get("issue"),
                     "failures": [text for _, failures, _ in hide for text in failures],
                     "keys": sorted(key for _, _, keys in hide for key in keys)}}


def render(state, pins, minimums, herdr_pin, run_url):
    """The record issue's body: the table, then the state for the next run."""
    lines = [
        "Each agent CLI's latest release, measured daily by the agent journey on the fake model on "
        "Linux, macOS and Windows (`.github/workflows/agent-canary.yml`). "
        "`Minimum` is the oldest version Hide puts its hook, plugin or extension in place for "
        "(`minimum_version` in `contracts/agent-journey.json`); "
        "`Pinned` is what every other CI lane installs (`contracts/agent-clis.json`).",
        "",
        "| Agent | Latest checked | Status | Minimum | Last passed | Pinned | Issue |",
        "|---|---|---|---|---|---|---|",
    ]
    for agent, row in sorted(state["agents"].items()):
        last = f"{row['last_pass']} ({row['last_pass_date']})" if row.get("last_pass") else "—"
        number = state.get("hide", {}).get("issue") if row["status"] == "hide" else row.get("issue")
        issue = f"#{number}" if number else ""
        lines.append(f"| {agent} | {row['checked']} ({row['date']}) | {MARK[row['status']]} | "
                     f"{minimums.get(agent, '—')} | {last} | {pins.get(row['kind'], '—')} | {issue} |")
    hide = state.get("hide", {})
    if hide.get("issue"):
        lines += ["", f"⚠️ Hide: the pinned version fails the same cells on the same commit, so Hide is what broke "
                  f"(#{hide['issue']}); the canary measures again every day until they pass."]
    herdr = state.get("herdr", {})
    lines += ["", f"Herdr: latest release {herdr.get('checked')}, bundled {herdr_pin} "
              "(moved by `.github/workflows/herdr-update.yml`).", "", f"Last run: {run_url}", "",
              STATE + json.dumps(state, separators=(",", ":")) + " -->"]
    return "\n".join(lines) + "\n"


def failure_body(agent, row, run_url, commit, pin):
    return (f"`{agent}` {row['checked']} fails Hide's agent journey on the fake model "
            f"(last passed: {row.get('last_pass') or 'never recorded'}); the pinned {pin} does not fail these "
            f"on the same commit {commit}, so this release is what changed.\n\n"
            + "\n".join(f"- {line}" for line in row["failures"])
            + f"\n\nRun: {run_url}\n\nA fix moves `contracts/agent-clis.json` to this version in the same pull "
            "request, so its CI proves the fix on it.\n")


def hide_body(failures, run_url, commit):
    return (f"Hide fails its agent journey on the fake model at {commit}: each cell below failed for an agent's "
            "latest release and again for its pinned version on the same commit, so the cause is Hide's, not an "
            "agent release's, and no agent's issue was filed for it.\n\n"
            + "\n".join(f"- {line}" for line in failures)
            + f"\n\nRun: {run_url}\n\nThe fix lands in Hide. The canary measures again every day until these "
            "cells pass, then closes this issue.\n")


def gh(*args, run=subprocess.run):
    done = run(["gh", *args, "--repo", ISSUE_REPOSITORY], capture_output=True, text=True, timeout=120)
    if done.returncode:
        raise SystemExit(f"gh {' '.join(args[:2])} exited {done.returncode}: {done.stderr.strip()[:300]}")
    return done.stdout


def record_issue(run=subprocess.run):
    """(number, body) of the open record issue, or (None, "")."""
    found = json.loads(gh("issue", "list", "--label", LABEL, "--state", "open", "--search",
                          f'in:title "{RECORD_TITLE}"', "--json", "number,title,body", run=run))
    found = [issue for issue in found if issue["title"] == RECORD_TITLE]
    return (found[0]["number"], found[0]["body"]) if found else (None, "")


def apply(state, before, run_url, commit, pins, run=subprocess.run):
    """Opens, comments on or closes each agent's failure issue and the one
    issue against Hide, and records each opened issue's number in `state`.
    A run that measured nothing new for an agent says nothing on its issue."""
    for agent, row in state["agents"].items():
        was = before["agents"].get(agent, {})
        if row["status"] == "fail":
            body = failure_body(agent, row, run_url, commit, pins.get(row["kind"]))
            if not row.get("issue"):
                url = gh("issue", "create", "--label", LABEL, "--title",
                         f"agent canary: {agent} {row['checked']} fails the agent journey", "--body", body,
                         run=run).strip()
                row["issue"] = int(url.rstrip("/").rsplit("/", 1)[-1])
            elif (was.get("checked"), was.get("failures")) != (row["checked"], row["failures"]):
                gh("issue", "comment", str(row["issue"]), "--body", body, run=run)
        elif row["status"] == "pass" and was.get("issue"):
            gh("issue", "close", str(was["issue"]), "--comment",
               f"`{agent}` {row['checked']} passes the agent journey again: {run_url}", run=run)
    hide, was = state["hide"], before.get("hide", {})
    if hide["failures"]:
        if not hide.get("issue"):
            url = gh("issue", "create", "--label", LABEL, "--title", HIDE_TITLE,
                     "--body", hide_body(hide["failures"], run_url, commit), run=run).strip()
            hide["issue"] = int(url.rstrip("/").rsplit("/", 1)[-1])
        elif was.get("keys") != hide["keys"]:
            gh("issue", "comment", str(hide["issue"]), "--body", hide_body(hide["failures"], run_url, commit), run=run)
    elif hide.get("issue") and not any(row["status"] == "unknown" for row in state["agents"].values()):
        # Every agent was judged and none fails for Hide's reason.
        gh("issue", "close", str(hide["issue"]), "--comment",
           f"Every pinned agent CLI passes the agent journey at {commit} again: {run_url}", run=run)
        hide["issue"] = None


def load_reports(folder, name=LATEST):
    """Each system's report, from the artifacts `<name>-<system>/report.json`."""
    reports = {}
    for system in SYSTEMS:
        path = folder / f"{name}-{system}" / "report.json"
        if path.is_file():
            reports[system] = json.loads(path.read_text())
    return reports


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("command", choices=("decide", "reruns", "record"))
    parser.add_argument("--versions", type=Path, required=True)
    parser.add_argument("--reports", type=Path)
    parser.add_argument("--run-url", default="")
    parser.add_argument("--commit", default="", help="the commit both journeys measured")
    args = parser.parse_args(argv)
    versions = json.loads(args.versions.read_text())
    contract = load_contract(ROOT)
    pins = {kind: row["version"] for kind, row in
            json.loads((ROOT / "contracts/agent-clis.json").read_text())["clis"].items()}
    if args.command == "reruns":
        if not args.reports:
            parser.error("reruns needs --reports")
        agents, systems = reruns(contract, versions, pins, load_reports(args.reports))
        print(f"agents={','.join(agents)}")
        print(f"systems={json.dumps(systems)}")
        return 0
    number, body = record_issue()
    state = read_state(body)
    if args.command == "decide":
        print(f"changed={'true' if changed(versions, state) else 'false'}")
        return 0
    if not args.reports or not args.commit:
        parser.error("record needs --reports and --commit")
    today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    results = verdicts(contract, versions, pins, load_reports(args.reports), load_reports(args.reports, PINNED))
    after = updated_state(state, contract, versions, results, today)
    gh("label", "create", LABEL, "--force", "--color", "C5DEF5",
       "--description", "The daily agent CLI canary's record and failures")
    apply(after, state, args.run_url, args.commit, pins)
    minimums = {row["id"]: row["minimum_version"] for row in active_rows(contract)}
    herdr_pin = json.loads((ROOT / "contracts/herdr-bundle.json").read_text())["version"]
    text = render(after, pins, minimums, herdr_pin, args.run_url)
    if number:
        gh("issue", "edit", str(number), "--body", text)
    else:
        gh("issue", "create", "--label", LABEL, "--title", RECORD_TITLE, "--body", text)
    failed = [agent for agent, row in after["agents"].items() if row["status"] != "pass"]
    for agent in failed:
        row = after["agents"][agent]
        print(f"::error title=agent canary {agent}::{row['checked']} ({row['status']}): "
              + "; ".join(row["failures"][:5]))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
