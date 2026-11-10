"""Which tests `scripts/ci-repeat.py` repeats for a pull request, and how many times."""
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "scripts/ci-repeat.py"
SPEC = importlib.util.spec_from_file_location("ci_repeat", SCRIPT)
repeat = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(repeat)

PLAN = importlib.util.spec_from_file_location("ci_plan", ROOT / "scripts/ci-plan.py")
ci = importlib.util.module_from_spec(PLAN)
PLAN.loader.exec_module(ci)
CRATE_DIRS = {directory: crate["name"] for directory, crate in ci.cargo_crates(ROOT).items()}


def entries(*paths, status="M"):
    return [(status, path) for path in paths]


class Budget(unittest.TestCase):
    """200 test runs for the whole pull request, 3 to 10 repeats of each test."""

    def test_the_repeat_is_the_budget_over_the_changed_tests_between_three_and_ten(self):
        for changed, expected in [(1, 10), (5, 10), (20, 10), (21, 9), (50, 4), (66, 3), (67, 3), (100, 3), (101, 3), (1000, 3)]:
            with self.subTest(changed=changed):
                self.assertEqual(repeat.repeats(changed), expected)

    def test_the_named_budget_examples_run_the_stated_number_of_tests(self):
        # 5 tests x 10 = 50 runs; 20 x 10 = 200; 50 x 4 = 200; 100 x 3 = 300, the floor of 3 winning.
        self.assertEqual([changed * repeat.repeats(changed) for changed in (5, 20, 50, 100)], [50, 200, 200, 300])

    def test_the_count_is_the_tests_inside_the_chosen_files_not_the_files(self):
        # One spec file of 12 tests: N is 12, so each runs floor(200 / 12) = 16 -> 10 times, 120 runs.
        found = repeat.select(entries("web/e2e/s3.spec.ts"), CRATE_DIRS)
        self.assertEqual(len(found["web"]), 1)
        self.assertEqual(repeat.repeats(12), 10)
        self.assertEqual(12 * repeat.repeats(12), 120)
        # Web and Rust tests share the one budget: 3 web + 47 Rust tests are 50 tests, 4 runs each.
        self.assertEqual(repeat.repeats(3 + 47), 4)
        # A selected helper module lists no test, so it adds nothing: 20 web tests and a helper are N = 20.
        self.assertEqual(repeat.repeats(20 + 0), 10)

    def test_no_changed_test_runs_nothing_and_a_negative_count_is_refused(self):
        self.assertEqual(repeat.repeats(0), 0)
        with self.assertRaises(ValueError):
            repeat.repeats(-1)

    def test_the_budget_never_falls_below_the_floor_or_rises_above_the_ceiling(self):
        for changed in range(1, 400):
            self.assertTrue(repeat.MIN_REPEAT <= repeat.repeats(changed) <= repeat.MAX_REPEAT)
            if changed <= repeat.BUDGET // repeat.MIN_REPEAT:
                self.assertLessEqual(changed * repeat.repeats(changed), repeat.BUDGET)


class Detection(unittest.TestCase):
    """The tests a diff adds or changes."""

    def test_an_added_or_modified_web_spec_is_a_changed_test_file(self):
        for status in ("A", "M"):
            with self.subTest(status=status):
                found = repeat.select([(status, "web/e2e/s3.spec.ts")], CRATE_DIRS)
                self.assertEqual(found, {"web": ["web/e2e/s3.spec.ts"], "rust": []})

    def test_a_deleted_spec_a_helper_a_desktop_spec_and_a_nested_file_are_not(self):
        found = repeat.select([
            ("D", "web/e2e/gone.spec.ts"),
            ("M", "web/e2e/wire.ts"),
            ("M", "web/e2e/shims/build.ts"),
            ("M", "web/e2e/nested/deep.spec.ts"),
            ("M", "desktop/e2e/attach.spec.ts"),
            ("M", "web/src/App.tsx"),
        ], CRATE_DIRS)
        self.assertEqual(found, {"web": [], "rust": []})

    def test_a_tests_it_module_is_a_changed_rust_test_selected_by_its_name(self):
        found = repeat.select(entries("hided/tests/it/handshake.rs"), CRATE_DIRS)
        self.assertEqual(found["rust"], [{"package": "hided", "module": "handshake"}])
        self.assertEqual(found["web"], [])

    def test_a_module_in_a_folder_is_named_with_its_parents_and_mod_rs_is_its_folder(self):
        found = repeat.select(entries("hide-holder/tests/it/node/pty.rs", "hided/tests/it/remote_delivery/mod.rs"), CRATE_DIRS)
        self.assertEqual(
            [(module["package"], module["module"]) for module in found["rust"]],
            [("hide-holder", "node::pty"), ("hided", "remote_delivery")],
        )

    def test_a_changed_child_of_a_changed_module_runs_once_through_its_parent(self):
        found = repeat.select(entries("hided/tests/it/remote_delivery/mod.rs", "hided/tests/it/remote_delivery/mail.rs"), CRATE_DIRS)
        self.assertEqual([module["module"] for module in found["rust"]], ["remote_delivery"])

    def test_the_helpers_every_module_shares_select_the_crates_whole_it_binary(self):
        found = repeat.select(entries("hided/tests/it/support/mod.rs", "hided/tests/it/support/run.rs", "hided/tests/it/main.rs", "hided/tests/it/seat.rs"), CRATE_DIRS)
        self.assertEqual(found["rust"], [{"package": "hided", "module": "*"}])

    def test_a_module_that_lists_no_test_runs_its_crates_whole_binary_and_n_is_what_lists(self):
        asked = []

        def lister(package, expression):
            asked.append((package, expression))
            return 0 if expression else 293

        plan = repeat.rust_plan([{"package": "hide-platform", "module": "stand_ins"}], lister)
        self.assertEqual(plan, [{"package": "hide-platform", "filter": "", "tests": 293}])
        self.assertEqual(asked, [("hide-platform", "test(/^(stand_ins)::/)"), ("hide-platform", "")])
        # A module that lists tests keeps its filter, and a helper adds nothing to 20 web tests: N = 20 + 5.
        plan = repeat.rust_plan([{"package": "hide-relay-protocol", "module": "admission"}], lambda package, expression: 5)
        self.assertEqual(plan, [{"package": "hide-relay-protocol", "filter": "test(/^(admission)::/)", "tests": 5}])

    def test_a_hand_run_module_that_does_not_exist_is_refused_before_anything_runs(self):
        for item in ("hided:no_such_module", "no-such-crate:seat", "hided:"):
            with self.subTest(item=item), self.assertRaises(ValueError):
                repeat.rust_entry(item)

    def test_product_code_other_files_and_unknown_directories_are_no_test(self):
        found = repeat.select(entries(
            "hided/src/cli.rs", "hided/tests/it/notes.md", "hided/tests/fixtures/it/x.rs", "elsewhere/tests/it/thing.rs",
        ), CRATE_DIRS)
        self.assertEqual(found["rust"], [])

    def test_a_deleted_rust_module_has_no_test_left(self):
        self.assertEqual(repeat.select(entries("hided/tests/it/gone.rs", status="D"), CRATE_DIRS)["rust"], [])

    def test_web_and_rust_changes_of_one_pull_request_are_both_selected(self):
        found = repeat.select(entries("web/e2e/a.spec.ts", "hided/tests/it/seat.rs", "hide-core/tests/it/anything.rs"), CRATE_DIRS)
        self.assertEqual(found["web"], ["web/e2e/a.spec.ts"])
        self.assertEqual([(module["package"], module["module"]) for module in found["rust"]], [("hide-core", "anything"), ("hided", "seat")])

    def test_every_tests_it_folder_of_the_workspace_belongs_to_a_crate_the_selection_names(self):
        # A `tests/it` folder under a directory that is not a workspace crate would be changed and never run.
        for folder in ROOT.glob("*/tests/it"):
            with self.subTest(folder=folder.relative_to(ROOT).as_posix()):
                self.assertIn(folder.parent.parent.name, CRATE_DIRS)


NEXTEST_LIST = {
    "rust-build-meta": {},
    "test-count": 5,
    "rust-suites": {
        "hided::it": {
            "testcases": {
                "handshake::a": {"ignored": False, "filter-match": {"status": "matches"}},
                "handshake::b": {"ignored": False, "filter-match": {"status": "matches"}},
                "handshake::slow": {"ignored": True, "filter-match": {"status": "mismatch", "reason": "ignored"}},
                "seat::other": {"ignored": False, "filter-match": {"status": "mismatch", "reason": "expression"}},
            },
        },
        "hided": {"testcases": {"lib_test": {"ignored": False, "filter-match": {"status": "matches"}}}},
    },
}


class RustCount(unittest.TestCase):
    """N counts the tests nextest would run, not the ones the source seems to declare."""

    def test_the_count_is_the_tests_whose_filter_matched(self):
        self.assertEqual(repeat.nextest_listed(json.dumps(NEXTEST_LIST)), 3)

    def test_a_filterset_that_matches_nothing_counts_zero(self):
        empty = {"rust-suites": {"hided::it": {"testcases": {"x": {"filter-match": {"status": "mismatch"}}}}}}
        self.assertEqual(repeat.nextest_listed(json.dumps(empty)), 0)
        self.assertEqual(repeat.nextest_listed(json.dumps({"rust-suites": {}})), 0)

    def test_output_that_is_no_nextest_list_is_refused(self):
        for text in ("", "not json", "{}", json.dumps({"rust-suites": []})):
            with self.subTest(text=text), self.assertRaises(ValueError):
                repeat.nextest_listed(text)


class Playwright(unittest.TestCase):
    def test_the_count_is_the_lists_total_line(self):
        self.assertEqual(repeat.listed_tests("Listing tests:\n  a.spec.ts:1:1 › one\nTotal: 1 test in 1 file\n"), 1)
        self.assertEqual(repeat.listed_tests("Listing tests:\n  a\n  b\nTotal: 12 tests in 3 files\n"), 12)

    def test_a_list_without_a_total_is_refused(self):
        for text in ("", "Listing tests:\nError: No tests found\n", "Total: 1 tests in 1 files\nTotal: 2 tests in 1 files\n"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                repeat.listed_tests(text)

    def test_a_named_spec_is_a_path_of_a_spec_that_exists(self):
        self.assertEqual(repeat.spec_path("s3.spec.ts"), "web/e2e/s3.spec.ts")
        self.assertEqual(repeat.spec_path("web/e2e/s3.spec.ts"), "web/e2e/s3.spec.ts")

    def test_a_name_that_is_no_spec_or_leaves_the_folder_is_refused(self):
        for name in ("missing-spec-name.spec.ts", "s3.ts", "../playwright.config.ts", "web/e2e/../x.spec.ts", "a/b.spec.ts", "s3.spec.ts; rm -rf", ""):
            with self.subTest(name=name), self.assertRaises(ValueError):
                repeat.spec_path(name)

    def test_the_file_filter_matches_that_spec_and_no_other(self):
        pattern = re.compile(repeat.spec_pattern("web/e2e/s3.spec.ts"), re.IGNORECASE)
        self.assertTrue(pattern.search("/work/hide/web/e2e/s3.spec.ts"))
        self.assertFalse(pattern.search("/work/hide/web/e2e/xs3.spec.ts"))
        self.assertFalse(pattern.search("/work/hide/web/e2e/s3-spec.ts"))
        self.assertFalse(pattern.search("/work/hide/web/e2e/s3.spec.tsx"))


class Nextest(unittest.TestCase):
    def test_the_modules_of_a_package_are_one_filterset_over_its_it_binary(self):
        modules = [
            {"package": "hided", "module": "seat"}, {"package": "hided", "module": "handshake"},
            {"package": "hide-holder", "module": "node::pty"},
        ]
        self.assertEqual(
            repeat.nextest_filters(modules),
            [("hide-holder", "test(/^(node::pty)::/)"), ("hided", "test(/^(handshake|seat)::/)")],
        )

    def test_a_name_that_is_not_an_identifier_never_reaches_the_filterset(self):
        for module in ({"package": "hided", "module": "a)|test(/.*"}, {"package": "-p x", "module": "seat"}, {"package": "hided", "module": ""}):
            with self.subTest(module=module), self.assertRaises(ValueError):
                repeat.nextest_filters([module])


class Resolve(unittest.TestCase):
    """What a pull request's call and a hand run each hand the lane."""

    def test_a_pull_request_hands_over_the_selection_and_waits_for_the_count(self):
        web = json.dumps(["web/e2e/s3.spec.ts"])
        rust = json.dumps([{"package": "hided", "module": "handshake"}])
        work = repeat.resolve("pull_request", web, rust, "", "linux")
        self.assertEqual((work["web"], work["runner"], work["repeat"], work["counted"]), (["web/e2e/s3.spec.ts"], "linux", 0, True))
        self.assertEqual(len(work["rust"]), 1)

    def test_a_pull_request_with_no_changed_test_has_nothing_to_count(self):
        work = repeat.resolve("pull_request", "[]", "[]", "", "linux")
        self.assertFalse(work["counted"])

    def test_a_pull_request_selection_that_is_not_a_spec_or_module_is_refused(self):
        with self.assertRaises(ValueError):
            repeat.resolve("pull_request", json.dumps(["web/e2e/wire.ts"]), "[]", "", "linux")
        with self.assertRaises(ValueError):
            repeat.resolve("pull_request", "[]", json.dumps([{"package": "hided", "module": "x y"}]), "", "linux")

    def test_a_hand_run_names_specs_and_modules_in_words_and_the_repeat_it_wants(self):
        work = repeat.resolve("workflow_dispatch", "s3.spec.ts  web/e2e/s7.spec.ts", "hided:handshake hided:seat", "7", "windows")
        self.assertEqual(work["web"], ["web/e2e/s3.spec.ts", "web/e2e/s7.spec.ts"])
        self.assertEqual([(m["package"], m["module"]) for m in work["rust"]], [("hided", "handshake"), ("hided", "seat")])
        self.assertEqual((work["runner"], work["repeat"], work["counted"]), ("windows", 7, False))

    def test_a_hand_run_with_a_bad_repeat_runner_or_name_is_refused(self):
        for web, rust, count, runner in [
            ("s3.spec.ts", "", "0", "linux"), ("s3.spec.ts", "", "101", "linux"), ("s3.spec.ts", "", "ten", "linux"),
            ("s3.spec.ts", "", "", "linux"), ("s3.spec.ts", "", "5", "plan9"), ("", "", "5", "linux"),
            ("", "hided", "5", "linux"), ("", "hided:Bad-Name", "5", "linux"),
        ]:
            with self.subTest(web=web, rust=rust, count=count, runner=runner), self.assertRaises(ValueError):
                repeat.resolve("workflow_dispatch", web, rust, count, runner)


class Command(unittest.TestCase):
    """The script as the workflow runs it."""

    def run_script(self, *args, stdin=None):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), *args], input=stdin, text=True, capture_output=True,
                env={**os.environ, "GITHUB_OUTPUT": str(output)},
            )
            written = output.read_text(encoding="utf-8") if output.exists() else ""
        return result, dict(line.split("=", 1) for line in written.splitlines())

    def test_budget_writes_the_repeat_and_the_total(self):
        result, outputs = self.run_script("budget", "--web", "3", "--rust", "47")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(outputs, {"repeat": "4", "total": "50"})

    def test_a_hand_run_resolves_to_outputs_the_jobs_read(self):
        result, outputs = self.run_script(
            "resolve", "--event", "workflow_dispatch", "--web", "s3.spec.ts", "--rust", "hided:handshake", "--repeat", "10", "--runner", "windows",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(outputs["runner"], "windows-latest")
        self.assertEqual(outputs["os"], "windows")
        self.assertEqual(outputs["web"], "web/e2e/s3.spec.ts")
        self.assertEqual(json.loads(outputs["rust"]), [{"package": "hided", "module": "handshake"}])
        self.assertEqual((outputs["repeat"], outputs["counted"], outputs["web-count-needed"], outputs["rust-count-needed"]), ("10", "false", "false", "false"))

    def test_a_pull_request_resolves_with_its_rust_count_and_no_repeat_yet(self):
        rust = json.dumps([{"package": "hided", "module": "handshake"}], separators=(",", ":"))
        result, outputs = self.run_script("resolve", "--event", "pull_request", "--web", '["web/e2e/s3.spec.ts"]', "--rust", rust)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((outputs["repeat"], outputs["counted"]), ("", "true"))
        self.assertEqual((outputs["web-count-needed"], outputs["rust-count-needed"]), ("true", "true"))

    def test_a_refusal_is_one_line_and_a_failed_exit(self):
        result, outputs = self.run_script("resolve", "--event", "workflow_dispatch", "--web", "nothing.spec.ts", "--repeat", "5")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.strip(), "ci-repeat: no such spec: web/e2e/nothing.spec.ts")
        self.assertEqual(outputs, {})

    def test_the_commands_the_jobs_pipe_through(self):
        result, _ = self.run_script("count-listed", stdin="Listing tests:\n  a\nTotal: 4 tests in 1 file\n")
        self.assertEqual(result.stdout.strip(), "4")
        result, _ = self.run_script("count-nextest", stdin=json.dumps(NEXTEST_LIST))
        self.assertEqual(result.stdout.strip(), "3")
        result, _ = self.run_script("spec-patterns", "web/e2e/s3.spec.ts")
        self.assertEqual(result.stdout.strip(), r"web/e2e/s3\.spec\.ts$")
        plan = json.dumps([{"package": "hided", "filter": "test(/^(seat)::/)", "tests": 1}, {"package": "hide-platform", "filter": "", "tests": 9}])
        result, _ = self.run_script("plan-lines", "--plan", plan)
        self.assertEqual(result.stdout, "hided\ttest(/^(seat)::/)\nhide-platform\t\n")


if __name__ == "__main__":
    unittest.main()
