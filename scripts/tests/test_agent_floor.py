"""The agent floor measurement: the table that installs one agent's CLI at an
older version, and the record of which versions passed on every system."""

import importlib.util
import json
from pathlib import Path
import sys
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import clis, floor  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402

spec = importlib.util.spec_from_file_location("agent_floor", ROOT / "scripts/agent-floor.py")
script = importlib.util.module_from_spec(spec)
spec.loader.exec_module(script)

PINNED = json.loads(clis.PINNED.read_text())
# What npm answered for these versions on 2026-10-27 (`npm view <package>@<version> --json`).
MANIFESTS = {
    ("@anthropic-ai/claude-code", "2.1.278"): {"bin": {"claude": "bin/claude.exe"},
                                              "scripts": {"postinstall": "node install.cjs"}},
    ("@anthropic-ai/claude-code", "2.1.100"): {"bin": {"claude": "cli.js"}},
    ("opencode-ai", "1.18.35"): {"bin": {"opencode": "bin/opencode.exe"},
                                 "scripts": {"postinstall": "node ./postinstall.mjs"}},
    ("@mariozechner/pi-coding-agent", "0.73.1"): {"bin": {"pi": "dist/cli.js"}},
}


def manifest(package, version):
    return MANIFESTS.get((package, version))


def report(row, failing=None):
    cells = {name: {"status": "pass" if declared else "N/A", "reason": ""} for name, declared in row["cells"].items()}
    if failing:
        cells[failing] = {"status": "fail", "reason": "timed out"}
    return {"table": {row["id"]: cells}, "agents": [{"id": row["id"], "pin": {"version": "1", "matches": True}}]}


class Tables(unittest.TestCase):
    def test_a_table_holds_the_measured_agent_alone(self):
        table = floor.table(PINNED, "codex", "0.140.0", lambda package, version: {"bin": {"codex": "bin/codex.js"}})
        self.assertEqual(table["clis"], {"codex": {"version": "0.140.0", "installer": "npm", "package": "@openai/codex"}})

    def test_a_native_program_is_reached_by_path_only_where_that_version_ships_one(self):
        native = floor.table(PINNED, "claude", "2.1.278", manifest)["clis"]["claude"]
        self.assertEqual(native["program"], "node_modules/@anthropic-ai/claude-code/bin/claude.exe")
        self.assertIn("install_scripts", native)
        script_only = floor.table(PINNED, "claude", "2.1.100", manifest)["clis"]["claude"]
        self.assertNotIn("program", script_only)
        self.assertNotIn("install_scripts", script_only)

    def test_a_version_published_under_the_earlier_name_installs_from_it(self):
        self.assertEqual(floor.table(PINNED, "opencode", "1.18.35", manifest)["clis"]["opencode"]["package"], "opencode-ai")
        self.assertEqual(floor.table(PINNED, "pi", "0.73.1", manifest)["clis"]["pi"]["package"],
                         "@mariozechner/pi-coding-agent")

    def test_a_version_npm_does_not_have_is_refused(self):
        with self.assertRaises(floor.FloorError):
            floor.table(PINNED, "claude", "2.1.101", manifest)

    def test_an_installer_without_a_version_is_refused(self):
        with self.assertRaisesRegex(floor.FloorError, "current release"):
            floor.table(PINNED, "agy", "1.4.0", manifest)

    def test_a_versioned_download_keeps_its_installer(self):
        self.assertEqual(floor.table(PINNED, "grok", "1.0.20", manifest)["clis"],
                         {"grok": {"version": "1.0.20", "installer": "grok"}})


class Record(unittest.TestCase):
    row = next(row for row in active_rows(load_contract(ROOT)) if row["id"] == "pi")

    def test_the_oldest_version_passing_every_system_and_the_newest_failure(self):
        passing, failing = report(self.row), report(self.row, failing="reply")
        reports = {(system, version): passing for system in floor.SYSTEMS for version in ("0.80.0", "1.0.0")}
        reports["windows", "0.60.0"] = failing
        reports["linux", "0.60.0"] = reports["macos", "0.60.0"] = passing
        result = floor.record(self.row, reports)
        self.assertEqual(floor.boundary(result), ("0.80.0", "0.60.0"))
        self.assertEqual(result["0.60.0"]["windows"], ["reply is fail (timed out)"])

    def test_a_system_without_a_report_is_no_pass(self):
        reports = {("linux", "0.70.0"): report(self.row), ("macos", "0.70.0"): report(self.row),
                   ("windows", "0.70.0"): None}
        self.assertEqual(floor.boundary(floor.record(self.row, reports)), (None, None))

    def test_the_record_reads_one_artifact_folder_per_system_and_version(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            for name in ("agent-journey-fake-windows-1.0.4", "agent-journey-fake-linux-1.0.4"):
                (Path(folder) / name).mkdir()
            (Path(folder) / "agent-journey-fake-linux-1.0.4/report.json").write_text(json.dumps(report(self.row)))
            self.assertEqual(script.reports(folder), {("windows", "1.0.4"): None, ("linux", "1.0.4"): report(self.row)})


class Workflow(unittest.TestCase):
    def test_only_the_floor_workflow_lifts_the_minimum_and_names_one_agent(self):
        floor_jobs = yaml.safe_load((ROOT / ".github/workflows/agent-floor.yml").read_text())["jobs"]
        self.assertIs(floor_jobs["journey"]["with"]["lift-minimum"], True)
        self.assertEqual(floor_jobs["journey"]["with"]["agents"], "${{ inputs.agent }}")
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            if path.name not in ("agent-floor.yml", "agent-journey.yml"):
                self.assertNotIn("lift-minimum", path.read_text(), path.name)
                self.assertNotIn("HIDE_AGENT_FLOOR_MEASUREMENT", path.read_text(), path.name)

    def test_every_measurable_agent_is_offered(self):
        options = yaml.safe_load((ROOT / ".github/workflows/agent-floor.yml").read_text())[True][
            "workflow_dispatch"]["inputs"]["agent"]["options"]
        kinds = {row["id"]: row["herdr_kind"] for row in active_rows(load_contract(ROOT))}
        expected = [agent for agent, kind in kinds.items()
                    if PINNED["clis"][kind]["installer"] not in floor.UNVERSIONED]
        self.assertEqual(options, expected)


if __name__ == "__main__":
    unittest.main()
