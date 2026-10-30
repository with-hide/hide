#!/usr/bin/env python3
"""A CI lane's outcome from one agent journey report (PRD agent-journey-check B26, B27, B29).

Usage: agent-journey-lane.py <report.json> --lane fixture|fake [--agents <id,...>]

The fixture lane passes only when every cell the stand-ins imitate passed for
each agent they imitate, and the fake lane (the real CLIs answered by the
journey's fake model) only when every declared cell passed for every registered agent, so
it can take the stand-ins' place. Each lane fails on a failed cell, an isolation, setup or
cleanup failure (the journey's exit 2), a request budget overrun, or a report
with no row for an agent it stands for; the fake lane also fails when an
agent's CLI is missing, as the job installed it, or answers another version
than contracts/agent-clis.json pins. `--agents` names the agents a fake run
was asked to measure (the agent canary's pinned rerun), and the lane then
stands for those alone. Each finding is printed as a GitHub annotation; the
exit code is the lane's.
"""

import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_journey.cells import CELLS  # noqa: E402
from agent_journey.ci import FIXTURE_AGENTS, FIXTURE_CELLS  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402
from agent_live_check.protection import ProtectionError  # noqa: E402

LANES = {"fixture": "fixture", "fake": "fake_model"}


def escaped(text):
    """A workflow command's text with GitHub's escapes, so a reason's line
    break or `::` cannot start a command of its own."""
    return str(text).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def annotation(level, title, *lines):
    """One annotation; its lines, each escaped, join as the message's lines."""
    return f"::{level} title={escaped(title).replace(':', '%3A').replace(',', '%2C')}::" + "%0A".join(map(escaped, lines))


def judge(report, lane, contract=None, agents=None):
    """(passed, annotations) for one report; `agents`, when given, the
    adapter IDs the caller asked the run to measure."""
    if not isinstance(report, dict) or not isinstance(report.get("table"), dict):
        return False, [annotation("error", "agent journey", "the report has no journey table")]
    notes, passed = [], True
    for name, field in LANES.items():
        if (lane == name) != bool(report.get(field)):
            return False, [annotation("error", "agent journey", f"the {lane} lane read a report that is "
                                      + (f"not a {name} run" if lane == name else f"a {name} run"))]
    if report.get("exit_code") == 2:
        passed = False
        notes.append(annotation("error", "agent journey", "the journey exited 2: isolation, setup or cleanup failed"))
    for failure in report.get("failures", []):
        passed = False
        notes.append(annotation("error", "agent journey setup", failure.get("reason", "setup failed")))
    cleanup = report.get("cleanup", {})
    if not cleanup.get("confirmed"):
        passed = False
        notes.append(annotation("error", "agent journey cleanup", "cleanup unconfirmed: "
                                + ("; ".join(map(str, cleanup.get("failures", []))) or "no receipt")))
    for failure in report.get("budget_failures", []):
        passed = False
        notes.append(annotation("error", "agent journey budget", failure))
    table = report.get("table", {})
    for agent, cells in table.items():
        if not isinstance(cells, dict):
            return False, [annotation("error", f"agent journey {agent}", "the row is not a cell table")]
        for name, cell in cells.items():
            if (not isinstance(cell, dict) or not isinstance(cell.get("status"), str)
                    or not isinstance(cell.get("reason"), str)):
                return False, [annotation("error", f"agent journey {agent}", f"{name}: no status and reason")]
            if cell["status"] == "fail":
                passed = False
                notes.append(annotation("error", f"agent journey {agent}", f"{name}: {cell['reason']}"))
    # Use this checkout's independently generated declaration, never a list
    # of successes or selected agents supplied by the report itself.
    contract = load_contract(Path(__file__).resolve().parents[1]) if contract is None else contract
    rows = active_rows(contract)
    if agents is not None:
        unknown = sorted(set(agents) - {row["id"] for row in rows})
        if unknown or not agents:
            return False, [annotation("error", "agent journey", f"--agents names no active adapter: {', '.join(unknown)}")]
        rows = [row for row in rows if row["id"] in agents]
    required = [row for row in rows if lane != "fixture" or row["id"] in FIXTURE_AGENTS]
    for row in required:
        agent = row["id"]
        if agent not in table:
            passed = False
            notes.append(annotation("error", f"agent journey {agent}", f"the report has no {agent} row"))
            continue
        cells = table[agent]
        for name in FIXTURE_CELLS if lane == "fixture" else CELLS:
            cell = cells.get(name, {"status": "absent", "reason": ""})
            expected = "pass" if row["cells"][name] else "N/A"
            status = cell.get("status")
            if status != expected:
                passed = False
                notes.append(annotation("error", f"agent journey {agent}", f"{name} is {cell['status']}, "
                                        + ("a stand-in passes it" if lane == "fixture" else
                                           f"the {lane} lane requires {expected} from the adapter declaration")
                                        + f": {cell['reason']}"))
    for agent in report.get("agents", []):
        if lane == "fake" and agent.get("cli_missing"):
            passed = False
            notes.append(annotation("error", f"agent journey {agent['id']}",
                                    f"{agent['cli_missing']} is not installed, though the job installs it"))
        pin = agent.get("pin") or {}
        if lane == "fake" and pin and not pin["matches"]:
            # A night measures what the table pins; a CLI that answers another
            # version is the one that changed (contracts/agent-clis.json).
            passed = False
            notes.append(annotation("error", f"agent journey {agent['id']}",
                                    f"answered {agent.get('version')!r}, contracts/agent-clis.json pins {pin['version']}"))
    return passed, notes


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("report", type=Path)
    parser.add_argument("--lane", choices=tuple(LANES), required=True)
    parser.add_argument("--agents", help="comma-separated adapter IDs the run was asked to measure")
    args = parser.parse_args(argv)
    try:
        report = json.loads(args.report.read_text())
        passed, notes = judge(report, args.lane, agents=args.agents.split(",") if args.agents else None)
    except (OSError, ValueError, ProtectionError) as error:
        print(annotation("error", "agent journey", f"no readable report at {args.report}: {error}"))
        return 1
    print("\n".join(notes))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
