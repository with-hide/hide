"""The daily agent canary: each CLI's latest release is read where its
installer reads it, and the record says per agent whether that release passed,
which release last did, and keeps one failure issue per agent until a release
passes again."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import clis, latest  # noqa: E402
from agent_journey.cells import CELLS  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402

spec = importlib.util.spec_from_file_location("agent_canary_record", ROOT / "scripts/agent-canary-record.py")
record = importlib.util.module_from_spec(spec)
spec.loader.exec_module(record)

CONTRACT = load_contract(ROOT)
AGENTS = [row["id"] for row in active_rows(CONTRACT)]
KINDS = {row["id"]: row["herdr_kind"] for row in active_rows(CONTRACT)}


def sources(url):
    return {
        latest.GROK_CHANNEL: "1.0.70\n",
        latest.OMP_LATEST: json.dumps({"tag_name": "v18.13.1"}),
        latest.AGY_MANIFEST: json.dumps({"version": "1.4.8", "url": "x"}),
        latest.HERDR_LATEST: json.dumps({"tag_name": "v0.9.6"}),
    }[url]


def report(failing=None):
    """A fake lane report in which every agent's declared cells came out as
    declared, but `failing` agent's `composer`."""
    table = {}
    for row in active_rows(CONTRACT):
        table[row["id"]] = {name: {"status": "pass" if row["cells"][name] else "N/A", "reason": ""}
                            for name in CELLS}
    if failing:
        table[failing]["composer"] = {"status": "fail", "reason": "erase: not_empty"}
    return {"table": table, "agents": [{"id": agent, "pin": {"version": "1", "matches": True}} for agent in AGENTS]}


class Latest(unittest.TestCase):
    def test_every_pinned_cli_has_a_latest_source(self):
        versions = latest.latest_table(clis.table(), read=sources, npm=lambda package: "9.9.9\n")
        self.assertEqual(set(versions), set(clis.table()))
        self.assertEqual(versions["grok"]["version"], "1.0.70")
        self.assertEqual(versions["omp"]["version"], "18.13.1")
        self.assertEqual(versions["agy"]["version"], "1.4.8")
        self.assertEqual(versions["claude"]["version"], "9.9.9")
        self.assertEqual(versions["claude"]["package"], clis.table()["claude"]["package"])
        self.assertEqual(latest.herdr_latest(read=sources), "0.9.6")

    def test_a_source_that_answers_no_version_fails_rather_than_leaving_the_pin(self):
        with self.assertRaises(latest.SourceError):
            latest.latest_table(clis.table(), read=lambda url: "<html>busy</html>", npm=lambda package: "1.0.0")
        with self.assertRaises(latest.SourceError):
            latest.latest_table(clis.table(), read=sources, npm=lambda package: "")

    def test_the_table_override_is_what_the_journey_reads(self):
        table = ROOT / "contracts/agent-clis.json"
        done = subprocess.run(
            [sys.executable, "-c", "from agent_journey import clis; print(clis.TABLE)"],
            cwd=ROOT / "scripts", capture_output=True, text=True,
            env={"PATH": "", "HIDE_AGENT_CLIS_TABLE": "/tmp/latest.json"})
        self.assertEqual(done.stdout.strip(), "/tmp/latest.json")
        self.assertEqual(clis.PINNED, table)


class Record(unittest.TestCase):
    versions = {**{kind: "2.0.0" for kind in KINDS.values()}, "herdr": "0.9.5"}

    def run_once(self, state, reports):
        after = record.updated_state(state, CONTRACT, self.versions, record.verdicts(CONTRACT, reports),
                                     "2026-10-27")
        calls = []

        def gh(argv, **_):
            calls.append(argv[1:])
            return subprocess.CompletedProcess(argv, 0, "https://github.com/o/r/issues/77\n", "")
        record.apply(after, state, "https://run", run=gh)
        return after, calls

    def test_a_pass_on_every_system_moves_last_passed(self):
        after, calls = self.run_once({"agents": {}}, {system: report() for system in record.SYSTEMS})
        self.assertEqual(calls, [])
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["last_pass_date"]), ("pass", "2.0.0", "2026-10-27"))

    def test_a_failing_cell_opens_one_issue_naming_the_system_and_the_cell(self):
        before = {"agents": {"codex": {"kind": "codex", "checked": "1.0.0", "last_pass": "1.0.0",
                                       "last_pass_date": "2026-10-26", "status": "pass"}}}
        reports = {system: report() for system in record.SYSTEMS}
        reports["windows"] = report(failing="codex")
        after, calls = self.run_once(before, reports)
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["issue"]), ("fail", "1.0.0", 77))
        self.assertEqual(row["failures"], ["windows: composer is fail (erase: not_empty)"])
        self.assertEqual([call[:2] for call in calls], [["issue", "create"]])
        self.assertIn("1.0.0", calls[0][calls[0].index("--body") + 1])

        # The next day, the same release fails the same way: nothing new is said.
        again, calls = self.run_once(after, reports)
        self.assertEqual((again["agents"]["codex"]["issue"], calls), (77, []))

        # A release that passes closes the issue.
        fixed, calls = self.run_once(again, {system: report() for system in record.SYSTEMS})
        self.assertEqual([call[:3] for call in calls], [["issue", "close", "77"]])
        self.assertIsNone(fixed["agents"]["codex"]["issue"])

    def test_a_system_without_a_report_judges_no_agent(self):
        reports = {system: report() for system in ("linux", "macos")}
        after, calls = self.run_once({"agents": {}}, reports)
        self.assertEqual({row["status"] for row in after["agents"].values()}, {"unknown"})
        self.assertEqual(calls, [])
        self.assertTrue(record.changed(self.versions, after), "an unjudged day is measured again")

    def test_a_cli_that_answered_another_version_than_the_table_fails(self):
        reports = {system: report() for system in record.SYSTEMS}
        reports["linux"]["agents"][0]["pin"] = {"version": "2.0.0", "matches": False}
        reports["linux"]["agents"][0]["version"] = "1.9.0"
        after, _ = self.run_once({"agents": {}}, reports)
        self.assertEqual(after["agents"][AGENTS[0]]["status"], "fail")

    def test_the_body_carries_the_table_and_the_state_the_next_run_reads(self):
        after, _ = self.run_once({"agents": {}}, {system: report() for system in record.SYSTEMS})
        body = record.render(after, {"codex": "0.163.0"}, "0.9.5", "https://run")
        self.assertIn("| codex | 2.0.0 (2026-10-27) | ✅ | 2.0.0 (2026-10-27) | 0.163.0 |  |", body)
        self.assertEqual(record.read_state(body), after)
        self.assertFalse(record.changed(self.versions, record.read_state(body)))
        self.assertTrue(record.changed({**self.versions, "claude": "2.0.1"}, record.read_state(body)))
        self.assertTrue(record.changed(self.versions, record.read_state("")))


if __name__ == "__main__":
    unittest.main()
