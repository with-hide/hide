#!/usr/bin/env python3
"""A CI lane's outcome from one agent journey report (PRD agent-journey-check B26, B27, B29).

Usage: agent-journey-lane.py <report.json> --lane fixture|login

The fixture lane passes only when every cell the stand-ins imitate passed for
both agents. Either lane fails on a failed cell, an isolation, setup or
cleanup failure (the journey's exit 2), or a request budget overrun; the login
lane also fails when an agent whose key is set is refused its login or its CLI
is missing, as the job installed it. An agent with no key is a notice, and a turn the provider answered nothing because the key's day was
spent is a `Not measured` warning, which keeps the lane from being green in
the nightly report (scripts/ci-flaky-report.py), and neither fails it. Each
finding is printed as a GitHub annotation; the exit code is the lane's.
"""

import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_journey.cells import BUILT_AGENTS as AGENTS  # noqa: E402
from agent_journey.ci import FIXTURE_CELLS  # noqa: E402


def escaped(text):
    """A workflow command's text with GitHub's escapes, so a reason's line
    break or `::` cannot start a command of its own."""
    return str(text).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def annotation(level, title, *lines):
    """One annotation; its lines, each escaped, join as the message's lines."""
    return f"::{level} title={escaped(title).replace(':', '%3A').replace(',', '%2C')}::" + "%0A".join(map(escaped, lines))


def judge(report, lane):
    """(passed, annotations) for one report."""
    notes, passed = [], True
    if lane == "fixture" and not report.get("fixture"):
        return False, [annotation("error", "agent journey", "the fixture lane read a report that is not a fixture run")]
    if lane == "login" and report.get("fixture"):
        return False, [annotation("error", "agent journey", "the login lane read a fixture report")]
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
    for item in (report.get("repository_cleanup") or {}).get("kept", []):
        passed = False
        notes.append(annotation("error", "agent journey cleanup", f"kept {item.get('branch')}: {item.get('reason')}"))
    for failure in report.get("budget_failures", []):
        passed = False
        notes.append(annotation("error", "agent journey budget", failure))
    table = report.get("table", {})
    for agent in AGENTS:
        cells = table.get(agent, {})
        for name, cell in cells.items():
            if cell["status"] == "fail":
                passed = False
                notes.append(annotation("error", f"agent journey {agent}", f"{name}: {cell['reason']}"))
        if lane == "fixture":
            for name in FIXTURE_CELLS:
                cell = cells.get(name, {"status": "absent", "reason": ""})
                if cell["status"] != "pass":
                    passed = False
                    notes.append(annotation("error", f"agent journey {agent}",
                                            f"{name} is {cell['status']}, a stand-in passes it: {cell['reason']}"))
    for agent in report.get("agents", []):
        if lane == "login" and agent.get("login"):
            passed = False
            notes.append(annotation("error", f"agent journey {agent['id']}", f"login refused: {agent['login']}"))
        if lane == "login" and agent.get("cli_missing"):
            passed = False
            notes.append(annotation("error", f"agent journey {agent['id']}",
                                    f"{agent['cli_missing']} is not installed, though the job installs it"))
        if agent.get("not_configured"):
            notes.append(annotation("notice", f"agent journey {agent['id']}",
                                    f"not configured (no {agent['not_configured']} secret); its signed-in journey stays unproven"))
        if agent.get("quota"):
            # The check on the first line and the reason after, as the nightly
            # report reads a `Not measured` annotation (docs/TESTING.md, A real agent on CI).
            unmeasured = [name for name, cell in table.get(agent["id"], {}).items()
                          if cell["status"] == "not measured" and cell["reason"].startswith("provider quota exhausted")]
            notes.append(annotation("warning", "Not measured", f"agent journey {agent['id']}",
                                    f"provider quota exhausted ({', '.join(agent['quota'])}): "
                                    f"{', '.join(unmeasured)} not reached"))
    return passed, notes


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("report", type=Path)
    parser.add_argument("--lane", choices=("fixture", "login"), required=True)
    args = parser.parse_args(argv)
    try:
        report = json.loads(args.report.read_text())
    except (OSError, ValueError) as error:
        print(annotation("error", "agent journey", f"no readable report at {args.report}: {error}"))
        return 1
    passed, notes = judge(report, args.lane)
    print("\n".join(notes))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
