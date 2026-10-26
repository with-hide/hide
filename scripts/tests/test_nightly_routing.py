"""Each nightly browser/desktop suite runs once, with OS-only dispatch preserved.

Read the actual calling workflows, including verify's calls. The small adapter
below models GitHub's expression boundary for these jobs; actionlint checks the
workflow syntax separately. Expected suites come from docs/TESTING.md, not the
workflow's current matrix.
"""
from collections import Counter
import json
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ROOT / ".github/workflows"


def expression(value, lane, system=""):
    if not isinstance(value, str) or not value.startswith("${{"):
        return value
    code = value[3:-2].replace("inputs.lane", repr(lane)).replace("matrix.system", repr(system))
    code = code.replace("&&", " and ").replace("||", " or ")
    return eval(code, {"__builtins__": {}, "fromJSON": json.loads})


def suites(lane):
    nightly = yaml.safe_load((WORKFLOWS / "nightly.yml").read_text())["jobs"]
    verify = yaml.safe_load((WORKFLOWS / "pr.yml").read_text())["jobs"]
    found = []

    def add(job, system=""):
        name = job["uses"].rsplit("/", 1)[-1]
        if name not in ("web-e2e.yml", "desktop-e2e.yml"):
            return
        called = yaml.safe_load((WORKFLOWS / name).read_text())
        defaults = called[True]["workflow_call"]["inputs"]
        values = {key: item.get("default") for key, item in defaults.items()}
        values.update({key: expression(value, lane, system) for key, value in job.get("with", {}).items()})
        found.append((name, values["runner"], values["grep"], values["shards"], values.get("keep-screenshots", False)))

    for job in nightly.values():
        uses = job.get("uses", "")
        if not uses or not expression(job.get("if", True), lane):
            continue
        if uses.endswith("/pr.yml"):
            # Nightly's non-PR call plans every web and desktop lane. The plan
            # and its required-gate enforcement are held by test_ci_plan.py.
            for nested in verify.values():
                if "uses" in nested:
                    add(nested)
        elif "strategy" in job:
            for system in expression(job["strategy"]["matrix"]["system"], lane):
                add(job, system)
        else:
            add(job)
    return Counter(found)


WEB_LINUX = ("web-e2e.yml", "ubuntu-latest", "", 5, False)
WEB_MACOS = ("web-e2e.yml", "macos-15", "@platform", 1, False)
WEB_WINDOWS = ("web-e2e.yml", "windows-latest", "@platform", 1, False)
DESKTOP_MACOS = ("desktop-e2e.yml", "macos-15", "", 2, False)
DESKTOP_LINUX = ("desktop-e2e.yml", "ubuntu-latest", "@platform", 1, True)
DESKTOP_WINDOWS = ("desktop-e2e.yml", "windows-latest", "@platform", 1, True)


class NightlyRouting(unittest.TestCase):
    def test_schedule_and_all_run_each_suite_once(self):
        expected = Counter([WEB_LINUX, WEB_MACOS, WEB_WINDOWS, DESKTOP_MACOS, DESKTOP_LINUX, DESKTOP_WINDOWS])
        for lane in ("", "all"):
            with self.subTest(lane=lane or "schedule"):
                self.assertEqual(suites(lane), expected)

    def test_os_only_dispatch_keeps_its_suites_without_starting_other_systems(self):
        for lane, expected in (
            ("linux", [DESKTOP_LINUX]),
            ("macos", [WEB_MACOS, DESKTOP_MACOS]),
            ("windows", [WEB_WINDOWS, DESKTOP_WINDOWS]),
            ("agy", []),
        ):
            with self.subTest(lane=lane):
                self.assertEqual(suites(lane), Counter(expected))


if __name__ == "__main__":
    unittest.main()
