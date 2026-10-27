#!/usr/bin/env python3
"""The daily agent canary's record: whether each agent CLI's latest release
passed Hide's agent journey, kept in one GitHub issue, and an issue for each
agent whose latest release fails (`.github/workflows/agent-canary.yml`).

Usage:
  agent-canary-record.py decide --versions <latest.json>
  agent-canary-record.py record --versions <latest.json> --reports <dir> --run-url <url>

`decide` prints `changed=true` when a version differs from the one the
record last measured, or the record has none, and `changed=false` otherwise,
so a day on which no vendor released starts no journey. `record` judges each
agent from the fake model lane's reports, one per system under `<dir>`, and
rewrites the record issue's table; an agent that fails gets its own issue, or
a comment on the open one, and an agent that passes again closes its issue.
The record's state rides in the issue body as one JSON comment, so the next
run reads what this one measured without a commit to main.
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

LABEL = "agent-canary"
RECORD_TITLE = "Agent CLI support record"
STATE = "<!-- agent-canary-state "
SYSTEMS = ("linux", "macos", "windows")
MARK = {"pass": "✅", "fail": "❌", "unknown": "❔"}


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
    the last run could not judge an agent."""
    rows = state["agents"].values()
    measured = {row.get("kind"): row.get("checked") for row in rows}
    measured["herdr"] = state.get("herdr", {}).get("checked")
    return (any(measured.get(kind) != version for kind, version in versions.items())
            or any(row.get("status") == "unknown" for row in rows))


def verdicts(contract, reports):
    """Each active agent's `(status, failures)` across the systems' reports:
    `pass` when every system measured it and every cell its declaration
    expects came out as declared, `fail` with the system and cell of each
    miss otherwise. A system whose report is missing leaves every agent
    `unknown`, since the run proves nothing about that system."""
    missing = [system for system in SYSTEMS if system not in reports]
    result = {}
    for row in active_rows(contract):
        agent = row["id"]
        if missing:
            result[agent] = ("unknown", [f"{system}: no report" for system in missing])
            continue
        failures = []
        for system in SYSTEMS:
            report = reports[system]
            entry = next((item for item in report.get("agents", []) if item.get("id") == agent), {})
            if entry.get("cli_missing"):
                failures.append(f"{system}: {entry['cli_missing']} is not installed")
            pin = entry.get("pin") or {}
            if pin and not pin.get("matches"):
                failures.append(f"{system}: answered {entry.get('version')!r}, not {pin.get('version')}")
            cells = report.get("table", {}).get(agent)
            if not isinstance(cells, dict):
                failures.append(f"{system}: no row")
                continue
            for name in CELLS:
                expected = "pass" if row["cells"][name] else "N/A"
                cell = cells.get(name) or {}
                if cell.get("status") != expected:
                    reason = str(cell.get("reason") or "")[:160]
                    failures.append(f"{system}: {name} is {cell.get('status', 'absent')}"
                                    + (f" ({reason})" if reason else ""))
        result[agent] = ("fail" if failures else "pass", failures)
    return result


def updated_state(state, contract, versions, results, today):
    """The record after this run: `checked` and `date` move for every agent,
    `last_pass` only for one that passed, and an agent's failure issue stays
    with it until it passes."""
    agents = {}
    for row in active_rows(contract):
        agent, kind = row["id"], row["herdr_kind"]
        before = state["agents"].get(agent, {})
        status, failures = results[agent]
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
    return {"agents": agents, "herdr": {"checked": versions.get("herdr"), "date": today}}


def render(state, pins, herdr_pin, run_url):
    """The record issue's body: the table, then the state for the next run."""
    lines = [
        "Each agent CLI's latest release, measured daily by the agent journey on the fake model on "
        "Linux, macOS and Windows (`.github/workflows/agent-canary.yml`). "
        "`Pinned` is what every other CI lane installs (`contracts/agent-clis.json`).",
        "",
        "| Agent | Latest checked | Status | Last passed | Pinned | Issue |",
        "|---|---|---|---|---|---|",
    ]
    for agent, row in sorted(state["agents"].items()):
        last = f"{row['last_pass']} ({row['last_pass_date']})" if row.get("last_pass") else "—"
        issue = f"#{row['issue']}" if row.get("issue") else ""
        lines.append(f"| {agent} | {row['checked']} ({row['date']}) | {MARK[row['status']]} | {last} | "
                     f"{pins.get(row['kind'], '—')} | {issue} |")
    herdr = state.get("herdr", {})
    lines += ["", f"Herdr: latest release {herdr.get('checked')}, bundled {herdr_pin} "
              "(moved by `.github/workflows/herdr-update.yml`).", "", f"Last run: {run_url}", "",
              STATE + json.dumps(state, separators=(",", ":")) + " -->"]
    return "\n".join(lines) + "\n"


def failure_body(agent, row, run_url):
    return (f"`{agent}` {row['checked']} fails Hide's agent journey on the fake model "
            f"(last passed: {row.get('last_pass') or 'never recorded'}).\n\n"
            + "\n".join(f"- {line}" for line in row["failures"])
            + f"\n\nRun: {run_url}\n\nA fix moves `contracts/agent-clis.json` to this version in the same pull "
            "request, so its CI proves the fix on it.\n")


def gh(*args, run=subprocess.run):
    done = run(["gh", *args], capture_output=True, text=True, timeout=120)
    if done.returncode:
        raise SystemExit(f"gh {' '.join(args[:2])} exited {done.returncode}: {done.stderr.strip()[:300]}")
    return done.stdout


def record_issue(run=subprocess.run):
    """(number, body) of the open record issue, or (None, "")."""
    found = json.loads(gh("issue", "list", "--label", LABEL, "--state", "open", "--search",
                          f'in:title "{RECORD_TITLE}"', "--json", "number,title,body", run=run))
    found = [issue for issue in found if issue["title"] == RECORD_TITLE]
    return (found[0]["number"], found[0]["body"]) if found else (None, "")


def apply(state, before, run_url, run=subprocess.run):
    """Opens, comments on or closes each agent's failure issue, and records
    each opened issue's number in `state`. A run that measured nothing new
    for an agent says nothing on its issue."""
    for agent, row in state["agents"].items():
        was = before["agents"].get(agent, {})
        if row["status"] == "fail":
            body = failure_body(agent, row, run_url)
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


def load_reports(folder):
    """Each system's report, from the artifacts `agent-journey-fake-<system>/report.json`."""
    reports = {}
    for system in SYSTEMS:
        path = folder / f"agent-journey-fake-{system}" / "report.json"
        if path.is_file():
            reports[system] = json.loads(path.read_text())
    return reports


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("command", choices=("decide", "record"))
    parser.add_argument("--versions", type=Path, required=True)
    parser.add_argument("--reports", type=Path)
    parser.add_argument("--run-url", default="")
    args = parser.parse_args(argv)
    versions = json.loads(args.versions.read_text())
    number, body = record_issue()
    state = read_state(body)
    if args.command == "decide":
        print(f"changed={'true' if changed(versions, state) else 'false'}")
        return 0
    if not args.reports:
        parser.error("record needs --reports")
    contract = load_contract(ROOT)
    today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    after = updated_state(state, contract, versions, verdicts(contract, load_reports(args.reports)), today)
    gh("label", "create", LABEL, "--force", "--color", "C5DEF5",
       "--description", "The daily agent CLI canary's record and failures")
    apply(after, state, args.run_url)
    pins = {kind: row["version"] for kind, row in
            json.loads((ROOT / "contracts/agent-clis.json").read_text())["clis"].items()}
    herdr_pin = json.loads((ROOT / "contracts/herdr-bundle.json").read_text())["version"]
    text = render(after, pins, herdr_pin, args.run_url)
    if number:
        gh("issue", "edit", str(number), "--body", text)
    else:
        gh("issue", "create", "--label", LABEL, "--title", RECORD_TITLE, "--body", text)
    failed = [agent for agent, row in after["agents"].items() if row["status"] != "pass"]
    for agent in failed:
        row = after["agents"][agent]
        print(f"::error title=agent canary {agent}::{row['checked']}: " + "; ".join(row["failures"][:5]))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
