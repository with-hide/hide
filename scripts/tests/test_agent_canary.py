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


def agy_manifest(system, version="1.4.9"):
    return json.dumps({"version": version, "url": f"https://agy/{version}/{system}", "sha512": "AB" * 64})


def sources(url):
    return {
        latest.GROK_CHANNEL: "1.0.70\n",
        latest.OMP_LATEST: json.dumps({"tag_name": "v18.13.1"}),
        **{manifest: agy_manifest(system) for system, manifest in latest.AGY_MANIFESTS.items()},
        latest.HERDR_LATEST: json.dumps({"tag_name": "v0.9.6"}),
    }[url]


def report(failing=None, agents=AGENTS, cell="composer", reason="erase: not_empty"):
    """A fake lane report of `agents` in which every declared cell came out
    as declared, but `failing` agent's `cell`."""
    table = {}
    for row in active_rows(CONTRACT):
        if row["id"] in agents:
            table[row["id"]] = {name: {"status": "pass" if row["cells"][name] else "N/A", "reason": ""}
                                for name in CELLS}
    if failing:
        table[failing][cell] = {"status": "fail", "reason": reason}
    return {"selected": list(agents), "table": table,
            "agents": [{"id": agent, "pin": {"version": "1", "matches": True}} for agent in agents]}


class Latest(unittest.TestCase):
    def test_every_pinned_cli_has_a_latest_source(self):
        versions = latest.latest_table(clis.table(), read=sources, npm=lambda package: "9.9.9\n")
        self.assertEqual(set(versions), set(clis.table()))
        self.assertEqual(versions["grok"]["version"], "1.0.70")
        self.assertEqual(versions["omp"]["version"], "18.13.1")
        self.assertEqual(versions["agy"]["version"], "1.4.9")
        # The row the canary installs from is the row a pin move copies.
        self.assertEqual(versions["agy"]["files"], {system: {"url": f"https://agy/1.4.9/{system}", "sha512": "ab" * 64}
                                                    for system in clis.AGY_SYSTEMS})
        self.assertEqual(versions["claude"]["version"], "9.9.9")
        self.assertEqual(versions["claude"]["package"], clis.table()["claude"]["package"])
        self.assertEqual(latest.herdr_latest(read=sources), "0.9.6")

    def test_a_source_that_answers_no_version_fails_rather_than_leaving_the_pin(self):
        with self.assertRaises(latest.SourceError):
            latest.latest_table(clis.table(), read=lambda url: "<html>busy</html>", npm=lambda package: "1.0.0")
        with self.assertRaises(latest.SourceError):
            latest.latest_table(clis.table(), read=sources, npm=lambda package: "")

    def test_agy_manifests_that_name_different_releases_or_no_file_fail(self):
        linux = latest.AGY_MANIFESTS["linux-x64"]
        for answer in (agy_manifest("linux-x64", "1.4.8"), json.dumps({"version": "1.4.9", "url": "x"})):
            with self.subTest(answer=answer), self.assertRaises(latest.SourceError):
                latest.agy_latest("agy", lambda url: answer if url == linux else sources(url))

    def test_the_table_override_is_what_the_journey_reads(self):
        table = ROOT / "contracts/agent-clis.json"
        done = subprocess.run(
            [sys.executable, "-c", "from agent_journey import clis; print(clis.TABLE)"],
            cwd=ROOT / "scripts", capture_output=True, text=True,
            env={"PATH": "", "HIDE_AGENT_CLIS_TABLE": "/tmp/latest.json"})
        self.assertEqual(done.stdout.strip(), "/tmp/latest.json")
        self.assertEqual(clis.PINNED, table)


class Record(unittest.TestCase):
    # Every latest release moved off its pin: `1.0.0` is pinned, `2.0.0` the latest.
    versions = {**{kind: "2.0.0" for kind in KINDS.values()}, "herdr": "0.9.5"}
    pins = {kind: "1.0.0" for kind in KINDS.values()}

    def run_once(self, state, reports, pinned=None, versions=None):
        versions = versions or self.versions
        after = record.updated_state(state, CONTRACT, versions,
                                     record.verdicts(CONTRACT, versions, self.pins, reports, pinned or {}), "2026-10-27")
        calls = []

        def gh(argv, **_):
            # Every issue lives in the private development repository.
            self.assertEqual(argv[-2:], ["--repo", "with-hide/hide-private"])
            calls.append(argv[1:-2])
            return subprocess.CompletedProcess(argv, 0, "https://github.com/o/r/issues/77\n", "")
        record.apply(after, state, "https://run", "abc123", self.pins, run=gh)
        return after, calls

    def test_a_pass_on_every_system_moves_last_passed(self):
        after, calls = self.run_once({"agents": {}}, {system: report() for system in record.SYSTEMS})
        self.assertEqual(calls, [])
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["last_pass_date"]), ("pass", "2.0.0", "2026-10-27"))

    def test_a_miss_the_pin_does_not_repeat_opens_one_issue_against_the_release(self):
        before = {"agents": {"codex": {"kind": "codex", "checked": "1.0.0", "last_pass": "1.0.0",
                                       "last_pass_date": "2026-10-26", "status": "pass"}}}
        reports = {system: report() for system in record.SYSTEMS}
        reports["windows"] = report(failing="codex")
        pinned = {"windows": report(agents=["codex"])}
        after, calls = self.run_once(before, reports, pinned)
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["issue"]), ("fail", "1.0.0", 77))
        self.assertEqual(row["failures"], ["windows: composer is fail (erase: not_empty)"])
        self.assertEqual([call[:2] for call in calls], [["issue", "create"]])
        body = calls[0][calls[0].index("--body") + 1]
        self.assertIn("the pinned 1.0.0 does not fail these on the same commit abc123", body)
        self.assertIsNone(after["hide"]["issue"])

        # The next day, the same release fails the same way: nothing new is said.
        again, calls = self.run_once(after, reports, pinned)
        self.assertEqual((again["agents"]["codex"]["issue"], calls), (77, []))

        # A release that passes closes the issue.
        fixed, calls = self.run_once(again, {system: report() for system in record.SYSTEMS})
        self.assertEqual([call[:3] for call in calls], [["issue", "close", "77"]])
        self.assertIsNone(fixed["agents"]["codex"]["issue"])

    def test_a_miss_the_pin_repeats_on_the_same_commit_is_hides_and_blames_no_release(self):
        # with-hide/hide-private#358: #1816 failed `guidance_memory` for every
        # agent, and the canary filed five agent releases for it.
        reason = "no assistant message carries JOURNEY-288EB49C"
        reports = {system: report(failing="codex", cell="guidance_memory", reason=reason) for system in record.SYSTEMS}
        pinned = {system: report(failing="codex", agents=["codex"], cell="guidance_memory", reason="JOURNEY-1")
                  for system in record.SYSTEMS}
        after, calls = self.run_once({"agents": {}}, reports, pinned)
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["issue"], row["last_pass"]), ("hide", None, None))
        self.assertEqual([call[:2] for call in calls], [["issue", "create"]])
        self.assertEqual(calls[0][calls[0].index("--title") + 1], record.HIDE_TITLE)
        body = calls[0][calls[0].index("--body") + 1]
        self.assertIn(f"- codex 1.0.0 (pinned; latest 2.0.0): linux: guidance_memory is fail ({reason})", body)
        self.assertIn("abc123", body)
        self.assertEqual(after["hide"]["issue"], 77)
        self.assertTrue(record.changed(self.versions, after), "a Hide failure is measured again the next day")
        body = record.render(after, {}, {}, "0.9.5", "https://run")
        self.assertIn("| codex | 2.0.0 (2026-10-27) | ⚠️ Hide |", body)
        self.assertIn("#77", body)

        # The next day the same cells fail with another run's token: nothing new is said.
        later = {system: report(failing="codex", cell="guidance_memory", reason="no message carries JOURNEY-ABCDEF12")
                 for system in record.SYSTEMS}
        again, calls = self.run_once(after, later, pinned)
        self.assertEqual((again["hide"]["issue"], calls), (77, []))

        # Once Hide is fixed every agent passes, and the issue closes.
        fixed, calls = self.run_once(again, {system: report() for system in record.SYSTEMS})
        self.assertEqual([call[:3] for call in calls], [["issue", "close", "77"]])
        self.assertIsNone(fixed["hide"]["issue"])

    def test_an_agent_measured_at_its_pin_already_fails_for_hide(self):
        reports = {system: report() for system in record.SYSTEMS}
        reports["linux"] = report(failing="codex", cell="letter", reason="no bell")
        at_pin = {**self.versions, "codex": "1.0.0"}
        self.assertEqual(record.reruns(CONTRACT, at_pin, self.pins, reports), ([], []))
        after, calls = self.run_once({"agents": {}}, reports, versions=at_pin)
        self.assertEqual(after["agents"]["codex"]["status"], "hide")
        self.assertEqual(after["hide"]["failures"], ["codex 1.0.0 (pinned): linux: letter is fail (no bell)"])

    def test_a_miss_with_no_pinned_measurement_blames_no_one(self):
        reports = {system: report() for system in record.SYSTEMS}
        reports["macos"] = report(failing="codex")
        broken_install = report(agents=["codex"])
        broken_install["agents"][0]["cli_missing"] = "codex"
        for pinned in ({}, {"linux": report(agents=["codex"])}, {"macos": report(agents=["claude-code"])},
                       {"macos": broken_install}):
            with self.subTest(pinned=pinned):
                after, calls = self.run_once({"agents": {}}, reports, pinned)
                self.assertEqual(after["agents"]["codex"]["status"], "unknown")
                self.assertEqual((calls, after["hide"]["issue"]), ([], None))
                self.assertTrue(record.changed(self.versions, after))

    def test_the_reruns_are_the_failing_releases_on_the_systems_they_failed(self):
        reports = {system: report() for system in record.SYSTEMS}
        reports["windows"] = report(failing="codex")
        reports["linux"] = report(failing="claude-code", cell="letter")
        self.assertEqual(record.reruns(CONTRACT, self.versions, self.pins, reports),
                         (["claude-code", "codex"], ["linux", "windows"]))
        self.assertEqual(record.reruns(CONTRACT, self.versions, self.pins,
                                       {system: report() for system in record.SYSTEMS}), ([], []))
        # A missing system's report judges no agent, so nothing is rerun.
        self.assertEqual(record.reruns(CONTRACT, self.versions, self.pins, {"linux": reports["linux"]}), ([], []))

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
        body = record.render(after, {"codex": "0.163.0"}, {"codex": "0.155.1"}, "0.9.5", "https://run")
        self.assertIn("| Agent | Latest checked | Status | Minimum | Last passed | Pinned | Issue |", body)
        self.assertIn("| codex | 2.0.0 (2026-10-27) | ✅ | 0.155.1 | 2.0.0 (2026-10-27) | 0.163.0 |  |", body)
        self.assertEqual(record.read_state(body), after)
        self.assertFalse(record.changed(self.versions, record.read_state(body)))
        self.assertTrue(record.changed({**self.versions, "claude": "2.0.1"}, record.read_state(body)))
        self.assertTrue(record.changed(self.versions, record.read_state("")))


if __name__ == "__main__":
    unittest.main()
