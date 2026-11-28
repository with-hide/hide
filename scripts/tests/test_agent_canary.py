"""The nightly's agent canary: each CLI's latest release is read where its
installer reads it, and the record says per agent and system whether that
release passed against the pins measured on the same commit, which release
last did, and keeps one failure issue per agent until a release passes
again."""

import importlib.util
import json
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from agent_journey import clis, latest  # noqa: E402
from agent_journey.cells import CELLS  # noqa: E402
from agent_journey.contract import active_rows, load_contract  # noqa: E402
from agent_journey.scene import lane_status  # noqa: E402

spec = importlib.util.spec_from_file_location("agent_canary_record", ROOT / "scripts/agent-canary-record.py")
record = importlib.util.module_from_spec(spec)
spec.loader.exec_module(record)

CONTRACT = load_contract(ROOT)
AGENTS = [row["id"] for row in active_rows(CONTRACT)]
KINDS = {row["id"]: row["agent_cli"] for row in active_rows(CONTRACT)}


def agy_manifest(system, version="1.4.9"):
    return json.dumps({"version": version, "url": f"https://agy/{version}/{system}", "sha512": "AB" * 64})


def sources(url):
    return {
        latest.GROK_CHANNEL: "1.0.70\n",
        latest.OMP_LATEST: json.dumps({"tag_name": "v18.13.1"}),
        **{manifest: agy_manifest(system) for system, manifest in latest.AGY_MANIFESTS.items()},
    }[url]


def report(failing=None, agents=AGENTS, cell="composer", reason="erase: not_empty"):
    """A fake lane report of `agents` in which every declared cell came out
    as the lane requires it (`scene.lane_status`: a scene cell no survey has
    closed reads `not measured`), but `failing` agent's `cell`."""
    table = {}
    for row in active_rows(CONTRACT):
        if row["id"] in agents:
            table[row["id"]] = {name: {"status": lane_status(row, name), "reason": ""} for name in CELLS}
    if failing:
        table[failing][cell] = {"status": "fail", "reason": reason}
    return {"selected": list(agents), "table": table,
            "agents": [{"id": agent, "pin": {"version": "1", "matches": True}} for agent in agents]}


class Verdict(unittest.TestCase):
    def test_a_normal_report_has_no_miss_and_the_lane_and_the_canary_agree(self):
        # Scene cells whose findings are unknown read `not measured` in a
        # healthy run; the canary asks the same owner as the lane.
        lane_spec = importlib.util.spec_from_file_location("lane_for_canary", ROOT / "scripts/agent-journey-lane.py")
        lane = importlib.util.module_from_spec(lane_spec)
        lane_spec.loader.exec_module(lane)
        healthy = {**report(), "fake_model": True, "fixture": False, "cleanup": {"confirmed": True},
                   "failures": []}
        self.assertEqual(lane.judge(healthy, "fake"), (True, []))
        for row in active_rows(CONTRACT):
            self.assertEqual(record.misses(row, healthy, "linux"), [], row["id"])

    def test_a_scene_cell_that_fails_or_passes_unproven_is_still_a_miss(self):
        row = next(row for row in active_rows(CONTRACT) if row["id"] == "claude-code")
        for status in ("fail", "pass"):
            value = report()
            value["table"]["claude-code"]["scene_popup"] = {"status": status, "reason": "x"}
            self.assertEqual([name for name, _, _ in record.misses(row, value, "linux")], ["scene_popup"])


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
    versions = {kind: "2.0.0" for kind in KINDS.values()}
    pins = {kind: "1.0.0" for kind in KINDS.values()}
    systems = list(record.SYSTEMS)

    def verdicts(self, latest, pinned, versions=None, systems=None):
        return record.verdicts(CONTRACT, versions or self.versions, self.pins, systems or self.systems,
                               latest, pinned)

    def test_each_report_is_read_from_its_artifacts_root_and_a_missing_one_is_not_a_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name in (record.LATEST, record.PINNED):
                for system in record.SYSTEMS:
                    path = root / f"{name}-{system}" / "report.json"
                    path.parent.mkdir()
                    failing = "codex" if (name, system) == (record.LATEST, "linux") else None
                    path.write_text(json.dumps(report(failing=failing)))
            latest, pinned = record.load_reports(root, record.LATEST), record.load_reports(root, record.PINNED)
            result = self.verdicts(latest, pinned)
            self.assertEqual(result["codex"][0], "fail")
            self.assertEqual(result["codex"][2], {"linux": "fail", "macos": "pass"})
            self.assertEqual(result["claude-code"], ("pass", [], {"linux": "pass", "macos": "pass"}))
            (root / f"{record.LATEST}-linux/report.json").unlink()
            result = self.verdicts(record.load_reports(root, record.LATEST), pinned)
            self.assertEqual(result["claude-code"], ("unknown", ["linux: no report"],
                                                     {"linux": "unknown", "macos": "pass"}))

    def test_the_journey_keeps_its_report_at_its_artifacts_root(self):
        # 2026-11-03 to 2026-11-28 the controller's folder rode in the same
        # artifact, which rooted it at the parent and left every agent
        # `no report`.
        journey = (ROOT / ".github/workflows/agent-journey.yml").read_text().split("\n  fake:\n", 1)[1]
        keep = journey.split("- name: Keep the report\n", 1)[1].split("\n      - ", 1)[0]
        name = re.search(r"name: (.+)", keep).group(1)
        self.assertEqual(name, "${{ inputs.report-name || 'agent-journey-fake' }}-${{ matrix.system }}")
        lines = keep.split("path: |\n", 1)[1].splitlines()
        paths = [text.strip() for text in lines if text.startswith(" " * 12)]
        self.assertTrue(paths and all(path.startswith("${{ env.RUN_DIR }}/") for path in paths), paths)
        # The nightly names the artifacts the record reads.
        nightly = (ROOT / ".github/workflows/nightly.yml").read_text()
        self.assertIn(f"report-name: {record.LATEST}\n", nightly)
        for name in (record.LATEST, record.PINNED):
            self.assertIn(f"pattern: {name}-*\n", nightly)

    def run_once(self, state, latest, pinned=None, versions=None, systems=None):
        versions = versions or self.versions
        after = record.updated_state(state, CONTRACT, versions, self.verdicts(latest, pinned or {}, versions, systems),
                                     "2026-10-27")
        found = record.actions(after, state, "https://run", "abc123", self.pins)
        calls = []

        def gh(argv, **_):
            # Every issue lives in the public repository with the other development issues.
            self.assertEqual(argv[-2:], ["--repo", "with-hide/hide"])
            calls.append(argv[1:-2])
            return subprocess.CompletedProcess(argv, 0, "https://github.com/o/r/issues/77\n", "")
        record.apply(after, found, run=gh)
        return after, calls

    def test_a_pass_on_every_system_moves_last_passed(self):
        after, calls = self.run_once({"agents": {}}, {system: report() for system in record.SYSTEMS})
        self.assertEqual(calls, [])
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["last_pass_date"]), ("pass", "2.0.0", "2026-10-27"))

    def test_a_day_without_macos_judges_linux_alone(self):
        after, calls = self.run_once({"agents": {}}, {"linux": report(failing="codex")}, {"linux": report()},
                                     systems=["linux"])
        self.assertEqual(after["agents"]["codex"]["systems"], {"linux": "fail"})
        self.assertEqual(after["agents"]["pi"]["status"], "pass")

    def test_a_miss_the_pin_does_not_repeat_opens_one_issue_against_the_release(self):
        before = {"agents": {"codex": {"kind": "codex", "checked": "1.0.0", "last_pass": "1.0.0",
                                       "last_pass_date": "2026-10-26", "status": "pass"}}}
        latest = {system: report() for system in record.SYSTEMS}
        latest["macos"] = report(failing="codex")
        pinned = {system: report() for system in record.SYSTEMS}
        after, calls = self.run_once(before, latest, pinned)
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["last_pass"], row["issue"]), ("fail", "1.0.0", 77))
        self.assertEqual(row["failures"], ["macos: composer is fail (erase: not_empty)"])
        self.assertEqual([call[:2] for call in calls], [["issue", "create"]])
        body = calls[0][calls[0].index("--body") + 1]
        self.assertIn("the pinned 1.0.0 does not fail these on the same commit abc123", body)

        # The next day, the same release fails the same way: nothing new is said.
        again, calls = self.run_once(after, latest, pinned)
        self.assertEqual((again["agents"]["codex"]["issue"], calls), (77, []))

        # A release that passes closes the issue.
        fixed, calls = self.run_once(again, {system: report() for system in record.SYSTEMS}, pinned)
        self.assertEqual([call[:3] for call in calls], [["issue", "close", "77"]])
        self.assertIsNone(fixed["agents"]["codex"]["issue"])

    def test_a_miss_the_pin_repeats_on_the_same_commit_is_hides_and_files_nothing_here(self):
        # with-hide/hide-private#358: #1816 failed `guidance_memory` for every
        # agent, and the canary filed five agent releases for it. The nightly's
        # report files the pins' failure from `agent journey`.
        reason = "no assistant message carries JOURNEY-288EB49C"
        latest = {system: report(failing="codex", cell="guidance_memory", reason=reason) for system in record.SYSTEMS}
        pinned = {system: report(failing="codex", cell="guidance_memory", reason="JOURNEY-1")
                  for system in record.SYSTEMS}
        after, calls = self.run_once({"agents": {}}, latest, pinned)
        row = after["agents"]["codex"]
        self.assertEqual((row["status"], row["issue"], row["last_pass"], calls), ("hide", None, None, []))
        self.assertEqual(row["failures"][0],
                         f"linux: guidance_memory is fail ({reason}); the pinned 1.0.0 fails it too")
        body = record.render(after, {}, {}, "https://run")
        self.assertIn("| codex | 2.0.0 (2026-10-27) | linux ⚠️ Hide · macos ⚠️ Hide |", body)

    def test_an_agent_measured_at_its_pin_already_fails_for_hide(self):
        latest = {system: report() for system in record.SYSTEMS}
        latest["linux"] = report(failing="codex", cell="letter", reason="no bell")
        at_pin = {**self.versions, "codex": "1.0.0"}
        after, calls = self.run_once({"agents": {}}, latest, versions=at_pin)
        self.assertEqual(after["agents"]["codex"]["status"], "hide")
        self.assertEqual(after["agents"]["codex"]["failures"],
                         ["linux: letter is fail (no bell) (the pinned 1.0.0 is the latest)"])
        self.assertEqual(calls, [])

    def test_a_miss_with_no_pinned_measurement_blames_no_one(self):
        latest = {system: report() for system in record.SYSTEMS}
        latest["macos"] = report(failing="codex")
        broken_install = report()
        broken_install["agents"][1]["cli_missing"] = "codex"
        self.assertEqual(broken_install["agents"][1]["id"], "codex")
        for pinned in ({}, {"linux": report()}, {"macos": report(agents=["claude-code"])},
                       {"macos": broken_install}):
            with self.subTest(pinned=pinned):
                after, calls = self.run_once({"agents": {}}, latest, pinned)
                self.assertEqual(after["agents"]["codex"]["status"], "unknown")
                self.assertEqual(calls, [])

    def test_a_cli_that_answered_another_version_than_the_table_fails(self):
        latest = {system: report() for system in record.SYSTEMS}
        latest["linux"]["agents"][0]["pin"] = {"version": "2.0.0", "matches": False}
        latest["linux"]["agents"][0]["version"] = "1.9.0"
        after, _ = self.run_once({"agents": {}}, latest)
        self.assertEqual(after["agents"][AGENTS[0]]["status"], "fail")

    def test_the_body_carries_the_table_and_the_state_the_next_run_reads(self):
        after, _ = self.run_once({"agents": {}}, {system: report() for system in record.SYSTEMS})
        body = record.render(after, {"codex": "0.163.0"}, {"codex": "0.155.1"}, "https://run")
        self.assertIn("| Agent | Latest checked | Status | Minimum | Last passed | Pinned | Issue |", body)
        self.assertIn("| codex | 2.0.0 (2026-10-27) | linux ✅ · macos ✅ | 0.155.1 | 2.0.0 (2026-10-27) | 0.163.0 |  |",
                      body)
        self.assertEqual(record.read_state(body), after)

    def test_a_run_of_another_branch_reads_the_record_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            for name in (record.LATEST, record.PINNED):
                path = root / f"{name}-linux" / "report.json"
                path.parent.mkdir()
                path.write_text(json.dumps(report(failing="codex" if name == record.LATEST else None)))
            versions = root / "versions.json"
            versions.write_text(json.dumps(self.versions))
            calls = []

            def gh(*args, run=None):
                calls.append(args[:2])
                return "[]"
            original, record.gh = record.gh, gh
            try:
                code = record.main(["--versions", str(versions), "--systems", '["linux"]', "--reports", str(root),
                                    "--commit", "abc123"])
            finally:
                record.gh = original
        self.assertEqual(code, 1)
        self.assertEqual(calls, [("issue", "list")])


if __name__ == "__main__":
    unittest.main()
