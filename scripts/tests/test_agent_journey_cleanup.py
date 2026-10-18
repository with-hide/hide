"""What the journey removes, names and reports after a run: no agent runs."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_journey import ci
from agent_journey.journey import Journey
from agent_journey.cli import budget_failures, hide_ai_requests, leftover_copies, remove_created, taken_branches
from agent_journey.local import claude_session_folders, disable_hide_ai, wrapper
from agent_journey.report import markdown

GIT_ENV = {"PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
           "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@invalid", "GIT_COMMITTER_NAME": "t",
           "GIT_COMMITTER_EMAIL": "t@invalid"}


def git(root, *args):
    return subprocess.run(["/usr/bin/git", "-C", str(root), *args], env=GIT_ENV, check=True,
                          capture_output=True, text=True).stdout.strip()


# The branch names of a run `r1` that measured these agents.
BRANCHES = ["journey/r1-claude-code", "journey/r1-codex", "journey/r1-pi"]


class RepositoryCleanup(unittest.TestCase):
    """The cleanup runs after the run's owner closed, so it takes no owner."""

    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        base = Path(folder.name).resolve()
        self.root, self.probe, self.elsewhere = base / "repo", base / "run/probe", base / "elsewhere"
        self.root.mkdir()
        self.probe.mkdir(parents=True)
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "commit", "-q", "--allow-empty", "-m", "base")
        self.runtime = SimpleNamespace(env=GIT_ENV, probe=self.probe)

    def worktree(self, branch, path):
        git(self.root, "worktree", "add", "-q", "-b", branch, str(path))
        return git(self.root, "rev-parse", branch)

    def branches(self):
        return git(self.root, "for-each-ref", "--format=%(refname:short)", "refs/heads/").split()

    def test_a_recorded_branch_goes_with_its_worktree(self):
        start = self.worktree("journey/r1-codex", self.probe / "worktrees/codex")
        result = remove_created(self.runtime, self.root, [
            {"branch": "journey/r1-codex", "start": start, "worktree": str(self.probe / "worktrees/codex")}], BRANCHES)
        self.assertEqual(result["kept"], [])
        self.assertEqual(self.branches(), ["main"])
        self.assertNotIn(str(self.probe), git(self.root, "worktree", "list"))

    def test_a_spawn_that_raised_before_its_start_was_read_is_still_removed(self):
        self.worktree("journey/r1-codex", self.probe / "worktrees/codex")
        result = remove_created(self.runtime, self.root, [
            {"branch": "journey/r1-codex", "start": None, "worktree": str(self.probe / "worktrees/codex")}], BRANCHES)
        self.assertEqual(result["kept"], [])
        self.assertTrue(result["removed"][0]["start_read_at_cleanup"])
        self.assertEqual(self.branches(), ["main"])

    def test_a_branch_of_this_run_the_record_lacks_is_swept_and_another_runs_stays(self):
        self.worktree("journey/r1-claude-code", self.probe / "worktrees/claude-code")
        git(self.root, "branch", "journey/r2-codex")
        # A sibling run whose name begins with this one's.
        git(self.root, "branch", "journey/r1-2-codex")
        result = remove_created(self.runtime, self.root, [], BRANCHES)
        self.assertEqual([item["branch"] for item in result["removed"]], ["journey/r1-claude-code"])
        self.assertTrue(result["removed"][0]["swept"])
        self.assertEqual(sorted(self.branches()), ["journey/r1-2-codex", "journey/r2-codex", "main"])

    def test_a_moved_branch_and_a_worktree_outside_the_run_are_kept_and_named(self):
        start = self.worktree("journey/r1-codex", self.probe / "worktrees/codex")
        git(self.probe / "worktrees/codex", "commit", "-q", "--allow-empty", "-m", "someone else's")
        self.worktree("journey/r1-pi", self.elsewhere)
        result = remove_created(self.runtime, self.root, [
            {"branch": "journey/r1-codex", "start": start, "worktree": str(self.probe / "worktrees/codex")}], BRANCHES)
        reasons = {item["branch"]: item["reason"] for item in result["kept"]}
        self.assertEqual(reasons, {"journey/r1-codex": "branch_moved_since_the_run_made_it",
                                   "journey/r1-pi": "worktree_outside_the_run"})
        self.assertTrue(self.elsewhere.is_dir())

    def test_a_rerun_finds_a_branch_an_earlier_run_of_the_same_name_left(self):
        # The run refuses to start on it, so its cleanup never takes it for its own.
        git(self.root, "branch", "journey/r1-codex")
        self.assertEqual(taken_branches(self.root, BRANCHES), ["journey/r1-codex"])

    def test_a_worktree_folder_already_gone_with_the_probe_is_still_unregistered(self):
        start = self.worktree("journey/r1-codex", self.probe / "worktrees/codex")
        subprocess.run(["/bin/rm", "-r", str(self.probe / "worktrees")], check=True)
        result = remove_created(self.runtime, self.root, [
            {"branch": "journey/r1-codex", "start": start, "worktree": str(self.probe / "worktrees/codex")}], BRANCHES)
        self.assertEqual(result["kept"], [])
        self.assertEqual(self.branches(), ["main"])


class Redaction(unittest.TestCase):
    def test_each_key_and_the_tail_claude_shows_of_it_are_replaced(self):
        key = "sk-ant-" + "a1b2c3d4" * 6
        with mock.patch.dict(os.environ, {"HIDE_CLAUDE_API_KEY": key, "HIDE_CODEX_API_KEY": ""}, clear=False):
            text = f"key {key} approve ...{key[-20:]}? other".encode()
            self.assertEqual(ci.redact(text), b"key [redacted] approve ...[redacted]? other")


class LocalStarts(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.base = Path(folder.name).resolve()

    def test_a_wake_start_through_the_panes_claude_carries_the_route(self):
        # Hide's Wake and Fork start `claude --resume <id> ...` by name in the pane,
        # which runs this wrapper, so the route rides on every start.
        home = self.base / "daemon-home"
        stand_in, argv = self.base / "claude", self.base / "argv"
        stand_in.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > {argv}\n')
        stand_in.chmod(0o700)
        target = wrapper(self.base / "bin", stand_in, self.base / "operator", self.base / "state", "claude",
                         home, None, "/usr/bin")
        subprocess.run([str(target), "--resume", "session-1"], check=True, env={"PATH": "/usr/bin:/bin"})
        self.assertEqual(argv.read_text().splitlines(), [
            "--settings", str(home / ".claude/settings.json"), "--setting-sources", "project,local",
            "--resume", "session-1"])
        # The file those arguments name is there before the kit fills it, as Claude refuses a missing one.
        self.assertEqual(json.loads((home / ".claude/settings.json").read_text()), {})

    def test_hide_ai_is_off_in_the_candidates_private_home(self):
        disable_hide_ai(self.base / "home")
        settings = self.base / "home/Library/Application Support/hide/ai.json"
        self.assertEqual(json.loads(settings.read_text()), {"enabled": False})
        self.assertEqual(settings.stat().st_mode & 0o777, 0o600)

    def test_hide_ais_requests_are_counted_by_provider_one_per_attempt(self):
        log = self.base / "hided.log"
        log.write_text("\n".join([
            '{"feature_id":"context_label","provider":"claude","outcome_class":"ok","input_chars":3}',
            '{"feature_id":"context_label","provider":"claude","outcome_class":"ok","duration_ms":5}',
            '{"feature_id":"context_label","provider":"claude","outcome_class":"invalid_output","duration_ms":4}',
            '{"component":"kit","kind":"apply.completed"}', "not json"]) + "\n")
        self.assertEqual(hide_ai_requests(log), {"claude": 2})
        self.assertEqual(hide_ai_requests(self.base / "missing.log"), {})


class FinishedAgent(unittest.TestCase):
    def test_each_pane_the_journey_opened_is_closed_once_and_one_that_stays_is_named(self):
        calls = []

        def command(argv, check=True):
            calls.append(argv)
            return (1, "", "pane_not_found\n") if argv[-1] == "w5:p1" else (0, "", "")
        runtime = SimpleNamespace(run=Path("/run"), command=command,
                                  owner=SimpleNamespace(cancelled=threading.Event()))
        record = {}
        journey = Journey(runtime, None, None, {}, {"id": "codex", "herdr_kind": "codex", "model_flag": []},
                          Path("/repo"), record, None, branch="b", created=[], sessions=Path("/h"), route={})
        journey.opened += ["w4:p2", "w4:p3", "w4:p2", None, "w5:p1"]
        journey.close()
        self.assertEqual(calls, [["pane", "close", pane] for pane in ("w4:p2", "w4:p3", "w5:p1")])
        self.assertEqual(record["closed"], {"w4:p2": "closed", "w4:p3": "closed", "w5:p1": "pane_not_found"})


class LeftoversAndReport(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.base = Path(folder.name).resolve()

    def test_a_login_copy_left_in_any_earlier_run_folder_is_named(self):
        runs = self.base / "agents/runs"
        for name in ("agent-journey-a", "release-0.5.0-journey", "this-run"):
            (runs / name / "probe/daemon-home/.codex").mkdir(parents=True)
            (runs / name / "probe/daemon-home/.codex/auth.json").write_text("{}")
        found = leftover_copies(self.base, runs / "this-run")
        self.assertEqual(found, [str(runs / name / "probe/daemon-home/.codex/auth.json")
                                 for name in ("agent-journey-a", "release-0.5.0-journey")])

    def test_claudes_session_folders_for_the_runs_paths_are_named_by_exact_path(self):
        run = self.base / "agents/runs/agent-journey-x"
        projects = self.base / "home/.claude/projects"
        # Claude's folder name: every character but a letter or digit as `-`.
        encoded = re.sub("[^A-Za-z0-9]", "-", str(run))
        other = re.sub("[^A-Za-z0-9]", "-", str(run.with_name("agent-journey-y")))
        sibling = re.sub("[^A-Za-z0-9]", "-", str(run.with_name("agent-journey-x-2")))
        for name in (encoded + "-probe-worktrees-claude-code", other + "-probe-worktrees-codex",
                     sibling + "-probe-worktrees-codex", "-elsewhere"):
            (projects / name).mkdir(parents=True)
        self.assertEqual(claude_session_folders(self.base / "home", run),
                         [str(projects / (encoded + "-probe-worktrees-claude-code"))])

    def test_only_an_agent_over_its_budget_fails_by_name(self):
        failures = budget_failures([{"id": "codex", "requests": 13, "request_budget": 12},
                                    {"id": "claude-code", "requests": 12, "request_budget": 12},
                                    {"id": "pi"}])
        self.assertEqual(failures, ["request_budget_exceeded: codex made 13 model requests, budget 12"])

    def test_an_agent_without_its_secret_is_not_configured_and_a_stand_in_needs_none(self):
        with mock.patch.dict(os.environ, {"HIDE_CODEX_API_KEY": ""}, clear=False):
            self.assertEqual(ci.not_configured("codex", None), "HIDE_CODEX_API_KEY")
            self.assertIsNone(ci.not_configured("codex", self.base))
        with mock.patch.dict(os.environ, {"HIDE_CODEX_API_KEY": "set"}, clear=False):
            self.assertIsNone(ci.not_configured("codex", None))

    def test_a_fixture_report_says_so_and_lists_what_a_stand_in_cannot_show(self):
        report = {"mode": "ci", "fixture": True, "fixture_limits": ci.FIXTURE_LIMITS, "herdr": {}, "table": {},
                  "systems": {}, "agents": []}
        text = markdown(report)
        self.assertIn("**Fixture run**", text)
        self.assertIn("never a release's agent evidence", text)
        self.assertIn("## What a stand-in cannot show", text)
        self.assertIn(ci.FIXTURE_LIMITS[0], text)
        self.assertNotIn("Fixture run", markdown({**report, "fixture": False, "fixture_limits": None}))


if __name__ == "__main__":
    unittest.main()
