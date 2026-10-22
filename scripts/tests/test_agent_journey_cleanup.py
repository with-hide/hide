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
from agent_journey.cli import budget_failures, hide_ai_requests, leftover_copies, measured_clone, provider_keys
from agent_journey.local import ROUTES, claude_session_folders, disable_hide_ai, opencode_key_names, wrapper
from agent_journey.report import markdown

GIT_ENV = {"PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
           "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@invalid", "GIT_COMMITTER_NAME": "t",
           "GIT_COMMITTER_EMAIL": "t@invalid"}


def git(root, *args):
    return subprocess.run(["/usr/bin/git", "-C", str(root), *args], env=GIT_ENV, check=True,
                          capture_output=True, text=True).stdout.strip()


class MeasuredRepository(unittest.TestCase):
    def test_the_clone_is_the_checkouts_commit_and_the_checkout_gains_nothing(self):
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder).resolve()
            checkout, home = base / "checkout", base / "run/probe/daemon-home"
            checkout.mkdir()
            home.mkdir(parents=True)
            git(checkout, "init", "-q", "-b", "work")
            git(checkout, "commit", "-q", "--allow-empty", "-m", "base")
            commit = git(checkout, "rev-parse", "HEAD")
            before = git(checkout, "for-each-ref")
            runtime = SimpleNamespace(home=home, env=GIT_ENV, owner=SimpleNamespace(
                run=lambda argv, env, cwd: subprocess.run(argv, env=env, cwd=cwd, check=True, capture_output=True)))
            root = measured_clone(runtime, checkout, commit)
            self.assertTrue(root.is_relative_to(home))
            self.assertEqual((git(root, "rev-parse", "--abbrev-ref", "HEAD"), git(root, "rev-parse", "HEAD")), ("main", commit))
            # The run's branches are the clone's; the checkout's refs stay as they were.
            git(root, "branch", "journey/r1-codex")
            self.assertEqual(git(checkout, "for-each-ref"), before)


class ProviderKeys(unittest.TestCase):
    def test_a_provider_key_in_the_shell_reaches_no_agent(self):
        from agent_live_check.runtime import clean_env
        shell = {"GEMINI_API_KEY": "g", "OPENROUTER_API_KEY": "o", "ANTHROPIC_AUTH_TOKEN": "a", "PATH": "/usr/bin"}
        with mock.patch.dict(os.environ, shell, clear=True):
            self.assertEqual(clean_env(), {"PATH": "/usr/bin"})


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

    def start(self, kind, keys=None, shell=None):
        """Starts the pane's `kind` as Hide's Wake does (`--resume <id>`), from
        a pane environment holding `shell`; returns the stand-in's arguments and
        environment."""
        home = self.base / "daemon-home"
        stand_in, argv, env = self.base / kind, self.base / "argv", self.base / "env"
        stand_in.write_text(f'#!/bin/sh\nprintf "%s\\n" "$@" > {argv}\nenv > {env}\n')
        stand_in.chmod(0o700)
        target = wrapper(self.base / "bin", stand_in, self.base / "operator", self.base / "state", kind,
                         home, None, "/usr/bin:/bin", keys=keys)
        subprocess.run([str(target), "--resume", "session-1"], check=True,
                       env={"PATH": "/usr/bin:/bin", **(shell or {})})
        variables = dict(line.split("=", 1) for line in env.read_text().splitlines() if "=" in line)
        return home, target, argv.read_text().splitlines(), variables

    def test_a_wake_start_through_the_panes_claude_reads_its_private_folder_and_the_operators_login(self):
        # Hide's Wake and Fork start `claude --resume <id> ...` by name in the pane,
        # which runs this wrapper, so the route rides on every start.
        home, _, argv, variables = self.start("claude", shell={"CLAUDE_CONFIG_DIR": "/operator/elsewhere"})
        self.assertEqual(argv, ["--resume", "session-1"])
        # Its configuration, sessions and the trust Hide records are the candidate's;
        # an empty secure-storage folder keeps the keychain entry the operator's own.
        self.assertEqual(variables["CLAUDE_CONFIG_DIR"], str(home / ".claude"))
        self.assertEqual(variables["CLAUDE_SECURESTORAGE_CONFIG_DIR"], "")
        self.assertEqual(variables["HOME"], str(self.base / "operator"))
        # So the never-trusted start leaves nothing in the operator's ~/.claude.json and is measured.
        self.assertIsNone(ROUTES["claude"]["untrusted"])

    def test_opencode_receives_only_the_variables_its_chosen_provider_names(self):
        config = {"model": "bai/flash", "small_model": "bai/flash", "provider": {
            "bai": {"options": {"apiKey": "{env:BAI_API_KEY}", "headers": {"X-Org": "{env:BAI_ORG}"}}},
            "other": {"options": {"apiKey": "{env:OTHER_API_KEY}"}}}}
        names = opencode_key_names(config)
        self.assertEqual(names, {"bai": ["BAI_API_KEY", "BAI_ORG"]})
        self.assertEqual(opencode_key_names({"provider": config["provider"]}), {})
        shell = {"BAI_API_KEY": "bai-secret-value", "BAI_ORG": "org-1", "OTHER_API_KEY": "other-secret",
                 "GEMINI_API_KEY": "gemini-secret"}
        runtime = SimpleNamespace(probe=self.base / "probe", credential_roots=set())
        keys, refusal = provider_keys(runtime, "opencode", names, shell)
        self.assertIsNone(refusal)
        # The files go with the probe, even when the cleanup is unconfirmed.
        self.assertEqual(runtime.credential_roots, {self.base / "probe/keys/opencode"})
        self.assertTrue(all(file.stat().st_mode & 0o777 == 0o600 for file in keys.values()))
        # The pane's environment carries no key; the wrapper adds exactly the named ones.
        _, target, _, variables = self.start("opencode", keys)
        self.assertEqual((variables["BAI_API_KEY"], variables["BAI_ORG"]), ("bai-secret-value", "org-1"))
        self.assertNotIn("OTHER_API_KEY", variables)
        self.assertNotIn("GEMINI_API_KEY", variables)
        self.assertNotIn("bai-secret-value", target.read_text())

    def test_a_chosen_providers_variable_the_shell_lacks_leaves_opencode_unmeasured_with_its_name(self):
        runtime = SimpleNamespace(probe=self.base / "probe", credential_roots=set())
        keys, refusal = provider_keys(runtime, "opencode", {"bai": ["BAI_API_KEY"]}, {"BAI_API_KEY": ""})
        self.assertEqual(keys, {})
        self.assertEqual(refusal, "opencode's chosen provider bai reads BAI_API_KEY from the environment, which the "
                                  "shell running the journey does not set; set it there and rerun")
        self.assertFalse((self.base / "probe").exists())

    def test_hide_ai_is_off_in_the_candidates_private_home(self):
        disable_hide_ai(self.base / "home")
        # The system's state folder under a home, as hide_platform::host::state_dir_under names it.
        state = {"darwin": "Library/Application Support", "win32": "AppData/Local"}.get(sys.platform, ".local/state")
        settings = self.base / "home" / state / "hide/ai.json"
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
                          Path("/repo"), record, None, branch="b", created=[], sessions=Path("/h"), route={}, composer_judge=None)
        journey.opened += ["w4:p2", "w4:p3", "w4:p2", None, "w5:p1"]
        journey.close()
        self.assertEqual(calls, [["pane", "close", pane] for pane in ("w4:p2", "w4:p3", "w5:p1")])
        self.assertEqual(record["closed"], {"w4:p2": "closed", "w4:p3": "closed", "w5:p1": "pane_not_found"})

    def test_a_pane_no_later_cell_types_into_closes_after_its_step_and_each_pane_closes_once(self):
        calls = []

        def command(argv, check=True):
            calls.append(argv[-1])
            return 0, "", ""
        runtime = SimpleNamespace(run=Path("/run"), command=command,
                                  owner=SimpleNamespace(cancelled=threading.Event()))
        journey = Journey(runtime, None, None, {}, {"id": "omp", "herdr_kind": "omp", "model_flag": []},
                          Path("/repo"), {}, None, branch="b", created=[], sessions=Path("/h"), route={}, composer_judge=None)
        # The agent's pane, then its fork's: the fork's ends with the fork cell.
        journey.pane = "w4:p2"
        journey.opened += ["w4:p2", "w4:p3"]
        journey.close(keep=journey.pane)
        self.assertEqual(calls, ["w4:p3"])
        # Wake moved the agent to a new pane; the untrusted start opens one more.
        journey.pane = "w4:p4"
        journey.opened += ["w4:p4"]
        journey.close(keep=journey.pane)
        journey.close()
        journey.opened += ["w5:p2"]
        journey.close()
        self.assertEqual(calls, ["w4:p3", "w4:p2", "w4:p4", "w5:p2"])


class FailedCellScreen(unittest.TestCase):
    def test_a_failed_cell_keeps_the_screen_of_the_pane_it_measured(self):
        with tempfile.TemporaryDirectory() as folder:
            read = []

            def command(argv, check=True):
                read.append(argv[2])
                return 0, f"screen of {argv[2]}\n", ""
            runtime = SimpleNamespace(run=Path(folder), command=command,
                                      owner=SimpleNamespace(cancelled=threading.Event()))
            table = {"opencode": {name: {"status": "pending"} for name in ("fork", "resume")}}
            journey = Journey(runtime, None, None, table, {"id": "opencode", "herdr_kind": "opencode", "model_flag": []},
                              Path("/repo"), {}, None, branch="b", created=[], sessions=Path("/h"), route={}, composer_judge=None)
            journey.pane = "w3:p1"
            # A fork is measured in its own pane while the journey's stays the source's.
            journey.settle("fork", ("fail", "no answer"), pane="w3:p7")
            journey.settle("resume", ("fail", "no answer"))
            self.assertEqual(read, ["w3:p7", "w3:p1"])
            self.assertEqual((Path(folder) / "opencode-fork-pane.txt").read_text(), "screen of w3:p7\n")
            self.assertEqual(table["opencode"]["fork"]["evidence"], "opencode-fork-pane.txt")


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
        watch = runs / "killed-run/watch-private/database-copy"
        watch.mkdir(parents=True)
        (watch / 'credential.db').write_bytes(b'fixture')
        found = leftover_copies(self.base, runs / "this-run")
        self.assertIn(str(watch.parent), found)
        found.remove(str(watch.parent))
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
        # The fake model answers no more than the budget, so a refusal past it is the overrun.
        self.assertEqual(budget_failures([{"id": "omp", "requests": 18, "request_budget": 18, "refused_past_budget": 3}]),
                         ["request_budget_exceeded: omp made 18 model requests, budget 18, "
                          "and the fake model refused 3 past it"])

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
