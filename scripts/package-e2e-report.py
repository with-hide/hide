"""Combine package passes before the job's single bounded failure report.

The original files stay beside each scenario's logs. Nested suite titles keep
the package scenario in a failure's name.
"""

import json
from pathlib import Path
import sys


def combine(root):
    suites = []
    for scenario in ("window", "update", "macos"):
        path = root / scenario / "report.json"
        if not path.is_file():
            continue  # A build failure has no Playwright result.
        with path.open("rb") as stream:
            content = stream.read(16 * 1024 * 1024 + 1)
        if len(content) > 16 * 1024 * 1024:
            raise ValueError(f"package report over budget: {scenario}")
        report = json.loads(content)
        suites.append({"title": scenario, "suites": report["suites"]})
    return {"suites": [{"title": "package", "suites": suites}]}


if __name__ == "__main__":
    root = Path(sys.argv[1])
    root.mkdir(parents=True, exist_ok=True)
    (root / "report.json").write_text(json.dumps(combine(root)), encoding="utf-8")
