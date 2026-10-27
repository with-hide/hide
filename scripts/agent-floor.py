#!/usr/bin/env python3
"""The agent floor measurement's tables and record (`.github/workflows/agent-floor.yml`).

Usage:
  agent-floor.py tables --agent <id> --versions <x.y.z,...>
  agent-floor.py record --agent <id> --reports <dir>

`tables` prints, as step outputs, `versions` (a JSON array, oldest first) and
`tables` (a JSON object of each version's CLI table, contracts/agent-clis.json's
shape holding that agent's row alone). `record` reads the fake model lane's
reports, one artifact folder per system and version
(`agent-journey-fake-<system>-<version>`), and prints the record as Markdown:
pass or fail per version per system, each failed cell with its reason, and
the oldest version that passed on all three systems.
"""

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import clis, floor  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402

ARTIFACT = "agent-journey-fake-"
MARK = {True: "✅", False: "❌", None: "❔ no report"}


def agent_row(agent):
    rows = {row["id"]: row for row in active_rows(load_contract(ROOT))}
    if agent not in rows:
        raise floor.FloorError(f"{agent!r} is no active agent ({', '.join(rows)})")
    return rows[agent]


def tables(agent, versions):
    kind = agent_row(agent)["herdr_kind"]
    pinned = json.loads(clis.PINNED.read_text())
    wanted = floor.ordered(dict.fromkeys(version.strip() for version in versions.split(",") if version.strip()))
    if not wanted:
        raise floor.FloorError("--versions names no version")
    built = {version: json.dumps(floor.table(pinned, kind, version)) for version in wanted}
    return f"versions={json.dumps(wanted)}\ntables={json.dumps(built)}"


def reports(folder):
    """Each report by (system, version); a folder without report.json is
    one whose job ended before the journey wrote it."""
    found = {}
    for path in sorted(Path(folder).iterdir()) if Path(folder).is_dir() else ():
        system, _, version = path.name.removeprefix(ARTIFACT).partition("-")
        if not path.name.startswith(ARTIFACT) or system not in floor.SYSTEMS or not floor.VERSION.match(version):
            continue
        report = path / "report.json"
        found[system, version] = json.loads(report.read_text()) if report.is_file() else None
    return found


def markdown(agent, result, versions):
    oldest, newest_failure = floor.boundary(result)
    lines = [f"## Agent floor: {agent}", "",
             "| Version | " + " | ".join(floor.SYSTEMS) + " |", "| --- |" + " --- |" * len(floor.SYSTEMS)]
    for version in versions:
        cells = result.get(version, {system: None for system in floor.SYSTEMS})
        lines.append(f"| {version} | " + " | ".join(
            MARK[None if cells[system] is None else not cells[system]] for system in floor.SYSTEMS) + " |")
    lines += ["", f"Oldest version that passed on every system: {oldest or 'none'}.",
              f"Newest version that failed on a system: {newest_failure or 'none'}.", ""]
    for version in versions:
        for system in floor.SYSTEMS:
            for miss in (result.get(version) or {}).get(system) or ():
                lines.append(f"- {version} {system}: {miss.replace('|', '/')}")
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    made = commands.add_parser("tables")
    made.add_argument("--agent", required=True)
    made.add_argument("--versions", required=True)
    read = commands.add_parser("record")
    read.add_argument("--agent", required=True)
    read.add_argument("--reports", type=Path, required=True)
    read.add_argument("--versions", required=True, help="the JSON array `tables` printed")
    args = parser.parse_args(argv)
    try:
        if args.command == "tables":
            print(tables(args.agent, args.versions))
            return 0
        versions = json.loads(args.versions)
        result = floor.record(agent_row(args.agent), reports(args.reports))
        print(markdown(args.agent, result, versions))
        oldest, newest_failure = floor.boundary(result)
        print(f"::notice title=agent floor {args.agent}::oldest pass on every system {oldest or 'none'}, "
              f"newest failure {newest_failure or 'none'}", file=sys.stderr)
        return 0
    except (floor.FloorError, OSError, ValueError) as error:
        print(f"::error title=agent floor::{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
