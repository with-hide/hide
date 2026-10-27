#!/usr/bin/env python3
"""Writes contracts/agent-clis.json with each CLI at its latest release.

Usage: agent-cli-latest.py <out.json>

The daily agent canary installs from the written table through
`HIDE_AGENT_CLIS_TABLE` (scripts/agent_journey/clis.py). It prints the
versions as one JSON object on stdout, by kind, with Herdr's latest release
under `herdr`.
"""

import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_journey import clis, latest  # noqa: E402


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print(__doc__.splitlines()[2].strip(), file=sys.stderr)
        return 2
    pinned = json.loads(clis.PINNED.read_text())
    try:
        rows = latest.latest_table(pinned["clis"])
        herdr = latest.herdr_latest()
    except (latest.SourceError, OSError, ValueError) as error:
        print(f"::error title=agent canary::a latest release could not be read: {error}", file=sys.stderr)
        return 1
    Path(argv[0]).write_text(json.dumps({**pinned, "clis": rows}, indent=2) + "\n")
    print(json.dumps({**{kind: row["version"] for kind, row in rows.items()}, "herdr": herdr}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
