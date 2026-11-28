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
CRATES = ci.cargo_crates(ROOT)
CRATE_DIRS = {directory: crate["name"] for directory, crate in CRATES.items()}
TARGETS = {directory: crate["targets"] for directory, crate in CRATES.items() if crate.get("targets")}


def entries(*paths, status="M"):
    return [(status, path) for path in paths]


class Budget(unittest.TestCase):
    """200 runs of the unit and Rust tests of a pull request, 3 to 10 repeats of each test."""

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
        # Unit and Rust tests share the one budget: 3 unit + 47 Rust tests are 50 tests, 4 runs each.
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


def listing(*tests):
    """`playwright test --list --reporter=list` output naming these `file › title` tests."""
    lines = [f"  {test.split(' › ')[0]}:1:1 › {test.split(' › ', 1)[1]}" for test in tests]
    files = len({test.split(" › ")[0] for test in tests})
    return "Listing tests:\n" + "".join(f"{line}\n" for line in lines) + f"Total: {len(tests)} tests in {files} files\n"


class WebBudget(unittest.TestCase):
    """2400 recorded test-seconds of web specs, on shards of 600 s or less.

    The expected numbers are worked by hand from the contract: repeat =
    clamp(floor(2400 / one pass), 3, 10); shards = the fewest that keep each at
    600 s or less, at most 4, or at most 6 when even 3 repeats exceed 2400.
    """

    def plan(self, table, *tests):
        return repeat.web_plan(listing(*(tests or table)), table)

    def test_the_repeat_is_the_seconds_budget_over_one_pass_between_three_and_ten(self):
        for seconds, expected in [(10, 10), (240, 10), (241, 9), (600, 4), (800, 3), (801, 3), (5000, 3)]:
            with self.subTest(seconds=seconds):
                self.assertEqual(self.plan({"a.spec.ts › one": seconds})["repeat"], expected)

    def test_one_pass_is_every_listed_test_and_a_test_the_table_lacks_weighs_its_median(self):
        # The table's median is 20 s (10, 20, 30): `new` counts 20, as
        # `e2e-shard.py` deals it, so one pass is 10 + 30 + 20 = 60 s.
        table = {"a.spec.ts › one": 10, "a.spec.ts › two": 30, "b.spec.ts › other": 20}
        plan = self.plan(table, "a.spec.ts › one", "a.spec.ts › two", "a.spec.ts › new")
        self.assertEqual((plan["tests"], plan["seconds"], plan["repeat"]), (3, 60.0, 10))
        self.assertEqual(repeat.e2e_shard.weights(["a.spec.ts › new"], table), {"a.spec.ts › new": 20.0})

    def test_a_spec_under_600_s_of_repeats_runs_on_one_shard(self):
        plan = self.plan({"a.spec.ts › one": 6.6})
        self.assertEqual((plan["repeat"], plan["shards"], plan["parts"], plan["heaviest"]), (10, 1, 1, 66.0))

    def test_the_shards_are_as_many_as_keep_each_at_600_s(self):
        # 8 tests of 30 s: one pass 240 s, 10 repeats, 2400 s: four groups of
        # two tests each plan 600 s, so four shards; three would plan 900.
        table = {f"a.spec.ts › t{index}": 30 for index in range(8)}
        plan = self.plan(table)
        self.assertEqual((plan["repeat"], plan["shards"], plan["heaviest"]), (10, 4, 600.0))
        # Half the seconds fits in two shards.
        table = {f"a.spec.ts › t{index}": 15 for index in range(8)}
        self.assertEqual((self.plan(table)["shards"], self.plan(table)["heaviest"]), (2, 600.0))

    def test_one_test_heavier_than_a_shard_repeats_on_several(self):
        # 150 s x 10 = 1500 s: no dealing of one test splits it, so the ten
        # repeats are cut into 4 + 3 + 3 on three shards.
        plan = self.plan({"a.spec.ts › slow": 150})
        self.assertEqual((plan["repeat"], plan["shards"], plan["parts"], plan["heaviest"]), (10, 3, 3, 600.0))
        self.assertEqual([repeat.shard_share(shard, 3, 3, 10) for shard in (1, 2, 3)],
                         [(1, 1, 4), (1, 1, 3), (1, 1, 3)])

    def test_within_the_budget_the_shards_stop_at_four(self):
        # 2 tests of 240 s: one pass 480 s, 5 repeats, 2400 s; within the
        # budget, at most four shards of 600 s each.
        table = {"a.spec.ts › one": 240, "a.spec.ts › two": 240}
        plan = self.plan(table)
        self.assertEqual((plan["repeat"], plan["shards"]), (5, 4))
        self.assertLessEqual(plan["heaviest"], 720.0)

    def test_three_repeats_over_the_budget_run_anyway_on_at_most_six_shards(self):
        # 30 tests of 50 s: one pass 1500 s, 3 x 1500 = 4500 s over 2400: still
        # three repeats, and six shards plan 750 s each, the most there may be.
        table = {f"a.spec.ts › t{index:02}": 50 for index in range(30)}
        plan = self.plan(table)
        self.assertEqual((plan["repeat"], plan["shards"], plan["heaviest"]), (3, 6, 750.0))
        # Over the budget yet under 600 s a shard at five: five, not six.
        table = {f"a.spec.ts › t{index:02}": 40 for index in range(25)}
        plan = self.plan(table)
        self.assertEqual((plan["repeat"], plan["shards"], plan["heaviest"]), (3, 5, 600.0))

    def test_every_shard_holds_a_test_and_a_run_and_every_group_runs_every_repeat(self):
        for seconds in ([1], [100], [700], [50] * 30, [5, 90, 200], [600, 1, 1]):
            table = {f"a.spec.ts › t{index:02}": value for index, value in enumerate(seconds)}
            plan = self.plan(table)
            with self.subTest(seconds=seconds, plan=plan):
                groups = plan["shards"] // plan["parts"]
                self.assertEqual(plan["shards"] % plan["parts"], 0)
                self.assertLessEqual(groups, len(table))
                self.assertLessEqual(plan["parts"], plan["repeat"])
                shares = [repeat.shard_share(shard, plan["shards"], plan["parts"], plan["repeat"])
                          for shard in range(1, plan["shards"] + 1)]
                for group in range(1, groups + 1):
                    self.assertEqual(sum(times for found, _, times in shares if found == group), plan["repeat"])
                    self.assertTrue(all(times >= 1 for _, _, times in shares))

    def test_tests_the_linux_lane_only_skips_repeat_the_most(self):
        self.assertEqual(self.plan({"a.spec.ts › skipped": 0})["repeat"], 10)

    def test_a_listing_that_names_no_test_or_disagrees_with_its_total_is_refused(self):
        with self.assertRaisesRegex(ValueError, "names no test"):
            repeat.web_plan("Listing tests:\nTotal: 0 tests in 0 files\n", {"a.spec.ts › one": 1})
        with self.assertRaisesRegex(ValueError, "totals 3 tests but names 1"):
            repeat.web_plan("Listing tests:\n  a.spec.ts:1:1 › one\nTotal: 3 tests in 1 file\n", {})

    def test_a_shard_outside_its_repeat_has_no_share(self):
        for shard, shards, parts, times in ((0, 2, 1, 10), (3, 2, 1, 10), (1, 4, 3, 10), (1, 3, 3, 2), (1, 2, 0, 10)):
            with self.subTest(shard=shard, shards=shards, parts=parts, times=times), self.assertRaises(ValueError):
                repeat.shard_share(shard, shards, parts, times)

    def test_the_total_is_one_count_from_every_shard(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for index, count in enumerate((40, 30, 30), 1):
                paths.append(Path(directory) / f"{index}.txt")
                paths[-1].write_text(f"{count}\n", encoding="utf-8")
            self.assertEqual(repeat.total_completed(paths, 3), 100)
            for wrong in (2, 4):
                with self.subTest(shards=wrong), self.assertRaisesRegex(ValueError, "shard counts"):
                    repeat.total_completed(paths, wrong)
            paths[0].write_text("0\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "zero"):
                repeat.total_completed(paths, 3)
            paths[0].write_text("\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                repeat.total_completed(paths, 3)


class Detection(unittest.TestCase):
    """The tests a diff adds or changes."""

    def test_an_added_or_modified_web_spec_is_a_changed_test_file(self):
        for status in ("A", "M"):
            with self.subTest(status=status):
                found = repeat.select([(status, "web/e2e/s3.spec.ts")], CRATE_DIRS)
                self.assertEqual(found, {"web": ["web/e2e/s3.spec.ts"], "units": [], "rust": [], "rust_sources": []})

    def test_a_deleted_spec_a_helper_a_desktop_spec_and_a_nested_file_are_not(self):
        found = repeat.select([
            ("D", "web/e2e/gone.spec.ts"),
            ("M", "web/e2e/wire.ts"),
            ("M", "web/e2e/shims/build.ts"),
            ("M", "web/e2e/nested/deep.spec.ts"),
            ("M", "desktop/e2e/attach.spec.ts"),
            ("M", "web/src/App.tsx"),
        ], CRATE_DIRS)
        self.assertEqual(found, {"web": [], "units": [], "rust": [], "rust_sources": []})

    def test_an_added_or_modified_unit_test_file_is_a_changed_test(self):
        for status in ("A", "M"):
            with self.subTest(status=status):
                found = repeat.select([(status, "web/src/searchDetail.test.ts")], CRATE_DIRS)
                self.assertEqual(found["units"], ["web/src/searchDetail.test.ts"])
                self.assertEqual((found["web"], found["rust"]), ([], []))

    def test_each_package_of_a_unit_test_file_is_named_and_product_code_is_not(self):
        found = repeat.select([
            ("M", "web/src/App.tsx"), ("M", "web/e2e/wire.ts"),
            ("M", "desktop/scripts/device-helpers.test.mjs"), ("M", "mobile/scripts/release.test.mjs"),
            ("M", "site/tests/legal.test.mjs"), ("M", "web/src/gallery/Scene.tsx"),
            ("D", "web/src/gone.test.ts"), ("M", "elsewhere/src/thing.test.ts"),
        ], CRATE_DIRS)
        self.assertEqual(found["units"], ["desktop/scripts/device-helpers.test.mjs", "mobile/scripts/release.test.mjs"])

    def test_a_unit_test_file_is_every_test_file_the_three_packages_collect(self):
        for path in ("web/src/a.test.tsx", "web/e2e/close-order.unit.ts", "desktop/src/main/env.test.ts",
                     "mobile/scripts/release.test.mjs"):
            with self.subTest(path=path):
                self.assertEqual(repeat.select(entries(path), CRATE_DIRS)["units"], [path])

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

        def lister(package, test, expression, run_ignored=""):
            asked.append((package, test, expression, run_ignored))
            return (0, 0) if expression else (293, 0)

        plan = repeat.rust_plan([{"package": "hide-platform", "module": "stand_ins"}], lister)
        self.assertEqual(
            plan,
            [{"package": "hide-platform", "test": "it", "filter": "", "tests": 293, "run_ignored": ""}],
        )
        self.assertEqual(
            asked,
            [("hide-platform", "it", "test(/^(stand_ins)::/)", ""), ("hide-platform", "it", "", "")],
        )
        # A helper beside a module that lists tests still runs the whole binary.
        plan = repeat.rust_plan(
            [{"package": "hide-platform", "module": "resident"}, {"package": "hide-platform", "module": "stand_ins"}],
            lambda package, test, expression, run_ignored="": (293, 0) if not expression else (0, 0) if "stand_ins)" in expression and "resident" not in expression else (9, 0),
        )
        self.assertEqual(plan, [{"package": "hide-platform", "test": "it", "filter": "", "tests": 293, "run_ignored": ""}])
        # A module that lists tests keeps its filter, and a helper adds nothing to 20 web tests: N = 20 + 5.
        plan = repeat.rust_plan([{"package": "hide-relay-protocol", "module": "admission"}], lambda package, test, expression, run_ignored="": (5, 0))
        self.assertEqual(
            plan,
            [{"package": "hide-relay-protocol", "test": "it", "filter": "test(/^(admission)::/)", "tests": 5, "run_ignored": ""}],
        )

    def test_a_hand_run_module_that_does_not_exist_is_refused_before_anything_runs(self):
        for item in ("hided:lib:no_such_module", "no-such-crate:lib:seat", "hided:lib:", "hided:seat", "hided:examples:seat"):
            with self.subTest(item=item), self.assertRaises(ValueError):
                repeat.rust_entry(item)

    def test_a_hand_run_names_a_src_test_module_by_the_target_it_is_built_into(self):
        # The case this lane first missed: a library's own unit test module.
        self.assertEqual(repeat.rust_entry("hided:lib:mobile::clear_tests"),
                         {"package": "hided", "test": "lib", "module": "mobile::clear_tests"})
        # A crate with no `tests/it` binary at all, and a test-gated block in a
        # product file, both reachable by name.
        self.assertEqual(repeat.rust_entry("hide-kit:lib:process::tests"),
                         {"package": "hide-kit", "test": "lib", "module": "process::tests"})
        self.assertEqual(repeat.rust_entry("hide-node:lib:ssh::host::bootstrap::windows_tests"),
                         {"package": "hide-node", "test": "lib", "module": "ssh::host::bootstrap::windows_tests"})
        # The whole target, and a `tests/it` module as before.
        self.assertEqual(repeat.rust_entry("hide-kit:lib:*"), {"package": "hide-kit", "test": "lib", "module": "*"})
        self.assertEqual(repeat.rust_entry("hided:it:seat"), {"package": "hided", "test": "it", "module": "seat"})

    def test_a_changed_src_file_selects_the_modules_it_declares_its_tests_in(self):
        # A file only a test build compiles is selected by its own module.
        found = repeat.select(entries("hided/src/mobile/clear_tests.rs"), CRATE_DIRS, TARGETS)
        self.assertEqual(found["rust_sources"], [{"package": "hided", "test": "lib", "module": "mobile::clear_tests"}])
        # A product file is selected by each test-gated block it holds, never
        # by the product module itself: that name holds the product's code.
        found = repeat.select(entries("hide-kit/src/process.rs"), CRATE_DIRS, TARGETS)
        self.assertEqual(found["rust_sources"], [{"package": "hide-kit", "test": "lib", "module": "process::tests"}])
        # A product file with no test of its own selects nothing.
        self.assertEqual(repeat.select(entries("hided/src/node_cli.rs"), CRATE_DIRS, TARGETS)["rust_sources"], [])

    def test_a_changed_src_file_selects_only_the_tests_it_declares_itself(self):
        # `runtime.rs` declares `runtime::tests` as a file and holds a
        # test-gated block of its own. A change to it selects the block, whose
        # tests it holds; the tests of `runtime/tests.rs` are that file's, and
        # a change to them selects them where they are written.
        found = repeat.select(entries("hide-core/src/runtime.rs"), CRATE_DIRS, TARGETS)
        self.assertEqual([entry["module"] for entry in found["rust_sources"]], ["runtime::deepest_holder_tests"])
        found = repeat.select(entries("hide-core/src/runtime/tests.rs"), CRATE_DIRS, TARGETS)
        self.assertEqual([entry["module"] for entry in found["rust_sources"]], ["runtime::tests"])

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

    def test_a_listing_names_the_ignored_tests_its_matched_ones_include(self):
        # A listing without `--run-ignored` names the ignored tests and marks
        # them, so `nextest_counted` tells a test that runs from one that waits
        # to be asked for; `nextest_listed` counts the migrated filter only.
        self.assertEqual(repeat.nextest_counted(json.dumps(NEXTEST_LIST)), (3, 0))
        both = {"rust-suites": {"hide-core": {"testcases": {
            "contract::one": {"ignored": True, "filter-match": {"status": "matches"}},
            "contract::two": {"ignored": False, "filter-match": {"status": "matches"}},
        }}}}
        self.assertEqual(repeat.nextest_counted(json.dumps(both)), (2, 1))
        self.assertEqual(repeat.nextest_listed(json.dumps(both)), 2)


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


class UnitFiles(unittest.TestCase):
    """A unit test file is named from the repository root and listed from its
    package's folder: the two are different strings, and Vitest matches only
    the second."""

    def test_a_hand_run_name_finds_its_package_and_must_exist(self):
        self.assertEqual(repeat.unit_path("web/src/searchDetail.test.ts"), "web/src/searchDetail.test.ts")
        self.assertEqual(repeat.unit_path("searchDetail.test.ts"), "web/src/searchDetail.test.ts")
        self.assertEqual(repeat.unit_path("env.test.ts"), "desktop/src/main/env.test.ts")
        for name in ("web/src/nothere.test.ts", "site/tests/legal.test.mjs", "../web/src/a.test.ts", ""):
            with self.subTest(name=name), self.assertRaises(ValueError):
                repeat.unit_path(name)

    def test_the_plan_names_the_file_from_the_root_and_lists_it_from_the_package(self):
        # What the lister receives is the package's spelling, which is what
        # `vitest list` matches; the plan keeps the root's, so a job can find
        # the file. A root-spelled filter listed 0 tests and would have failed
        # every units run at its count.
        asked = []

        def lister(package, files):
            asked.append((package, files))
            return {path: 4 for path in files}

        plan, moved = repeat.units_plan(["web/src/searchDetail.test.ts", "desktop/src/main/env.test.ts"], lister, "linux")
        self.assertEqual(asked, [("desktop", ["desktop/src/main/env.test.ts"]), ("web", ["web/src/searchDetail.test.ts"])])
        self.assertEqual(plan, [{"package": "desktop", "files": ["desktop/src/main/env.test.ts"], "tests": 4},
                                {"package": "web", "files": ["web/src/searchDetail.test.ts"], "tests": 4}])
        self.assertEqual(moved, [])

    def test_a_file_that_lists_no_test_fails_a_leg_and_names_its_system(self):
        with self.assertRaisesRegex(ValueError, "list no test on windows: web/src/searchDetail.test.ts"):
            repeat.units_plan(["web/src/searchDetail.test.ts"], lambda package, files: {path: 0 for path in files}, "windows")

    def test_a_pull_requests_count_moves_a_file_linux_lists_no_test_of_to_windows(self):
        # A file whose tests all declare Windows (`describe.runIf(process.platform
        # === "win32")`) lists none on Linux, which completed zero runs of it
        # (#3150); the count keeps its Linux files and hands that one on.
        counts = {"desktop/e2e/windows-processes.unit.ts": 0, "desktop/src/main/env.test.ts": 5, "web/e2e/windows-sockets.unit.ts": 0}
        plan, moved = repeat.units_plan(sorted(counts), lambda package, files: {path: counts[path] for path in files}, "linux",
                                        repeat.UNITS_ELSEWHERE)
        self.assertEqual(plan, [{"package": "desktop", "files": ["desktop/src/main/env.test.ts"], "tests": 5}])
        self.assertEqual(moved, ["desktop/e2e/windows-processes.unit.ts", "web/e2e/windows-sockets.unit.ts"])

    def test_a_test_counts_for_the_file_vitest_names_and_only_a_selected_one(self):
        # A filter is a substring, so `env.test.ts` also lists `myenv.test.ts`.
        selected = ["desktop/src/main/env.test.ts", "desktop/e2e/windows-processes.unit.ts"]
        tests = [{"name": name, "file": (repeat.ROOT / path).as_posix()} for name, path in (
            ("a", "desktop/src/main/env.test.ts"), ("b", "desktop/src/main/env.test.ts"), ("c", "desktop/src/main/myenv.test.ts"))]
        self.assertEqual(repeat.listed_per_file(tests, selected),
                         {"desktop/src/main/env.test.ts": 2, "desktop/e2e/windows-processes.unit.ts": 0})


class Nextest(unittest.TestCase):
    def test_the_modules_of_a_target_are_one_filterset_over_it(self):
        modules = [
            {"package": "hided", "module": "seat"}, {"package": "hided", "module": "handshake"},
            {"package": "hide-holder", "module": "node::pty"},
            {"package": "hided", "test": "lib", "module": "mobile::clear_tests"},
            {"package": "hide-kit", "test": "lib", "module": "process::tests"},
        ]
        self.assertEqual(
            repeat.nextest_filters(modules),
            [("hide-holder", "it", "test(/^(node::pty)::/)"), ("hide-kit", "lib", "test(/^(process::tests)::/)"),
             ("hided", "it", "test(/^(handshake|seat)::/)"), ("hided", "lib", "test(/^(mobile::clear_tests)::/)")],
        )

    def test_a_name_that_is_not_an_identifier_never_reaches_the_filterset(self):
        for module in ({"package": "hided", "module": "a)|test(/.*"}, {"package": "-p x", "module": "seat"}, {"package": "hided", "module": ""},
                       {"package": "hided", "test": "examples", "module": "seat"}):
            with self.subTest(module=module), self.assertRaises(ValueError):
                repeat.nextest_filters([module])


class RustIgnoredAndHolder(unittest.TestCase):
    """What the Rust job is told beyond the filterset: the flag an ignored test
    needs and the holder it starts."""

    def test_a_pull_requests_module_of_ignored_tests_runs_its_target_whole_without_them(self):
        # `node_contract` is all ignored, and its tests need an sshd, the CLI
        # build and a retained older build this lane does not prepare; a pull
        # request that changes it repeats its binary's ordinary tests, as main
        # did, and asks for no ignored test.
        asked = []

        def lister(package, test, expression, run_ignored=""):
            asked.append(run_ignored)
            return (0, 0) if expression else (293, 0)

        plan = repeat.rust_plan([{"package": "hided", "test": "it", "module": "node_contract"}], lister)
        self.assertEqual(plan, [{"package": "hided", "test": "it", "filter": "", "tests": 293, "run_ignored": ""}])
        self.assertEqual(set(asked), {""})

    def test_a_module_mixing_ignored_tests_runs_its_ordinary_tests_only(self):
        # Its ignored tests need what other lanes prepare, or are a
        # subprocess's entry point (opener_lifecycle::fake_program) that
        # fails run alone, so asking for them would fail an ordinary change.
        def lister(package, test, expression, run_ignored=""):
            return {"": (5, 0), "all": (7, 2), "ignored-only": (2, 2)}[run_ignored]

        plan = repeat.rust_plan([{"package": "hided", "test": "it", "module": "opener_lifecycle"}], lister)
        self.assertEqual((plan[0]["run_ignored"], plan[0]["tests"]), ("", 5))
        decided = repeat.plan_run_ignored([{"package": "hided", "test": "it", "filter": "test(/^(opener_lifecycle)::/)"}], lister)
        self.assertEqual(decided[0]["run_ignored"], "")

    def test_an_ordinary_module_passes_no_flag_and_builds_no_holder(self):
        def lister(package, test, expression, run_ignored=""):
            self.assertEqual(run_ignored, "")
            return (8, 0)

        plan = repeat.rust_plan([{"package": "hide-kit", "test": "lib", "module": "process::tests"}], lister)
        self.assertEqual(
            plan,
            [{"package": "hide-kit", "test": "lib", "filter": "test(/^(process::tests)::/)",
              "tests": 8, "run_ignored": ""}],
        )

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

    def test_a_hand_run_names_specs_modules_and_unit_files_in_words_and_the_repeat_it_wants(self):
        work = repeat.resolve("workflow_dispatch", "s3.spec.ts  web/e2e/s7.spec.ts",
                              "hided:it:handshake hided:it:seat hided:lib:mobile::clear_tests", "7", "windows",
                              "web/src/searchDetail.test.ts desktop/src/main/env.test.ts")
        self.assertEqual(work["web"], ["web/e2e/s3.spec.ts", "web/e2e/s7.spec.ts"])
        self.assertEqual([(m["package"], m["test"], m["module"]) for m in work["rust"]],
                         [("hided", "it", "handshake"), ("hided", "it", "seat"), ("hided", "lib", "mobile::clear_tests")])
        self.assertEqual(work["units"], ["desktop/src/main/env.test.ts", "web/src/searchDetail.test.ts"])
        self.assertEqual((work["runner"], work["repeat"], work["counted"]), ("windows", 7, False))

    def test_a_hand_run_with_a_bad_repeat_runner_or_name_is_refused(self):
        for web, rust, units, count, runner in [
            ("s3.spec.ts", "", "", "0", "linux"), ("s3.spec.ts", "", "", "101", "linux"), ("s3.spec.ts", "", "", "ten", "linux"),
            ("s3.spec.ts", "", "", "", "linux"), ("s3.spec.ts", "", "", "5", "plan9"), ("", "", "", "5", "linux"),
            ("", "hided", "", "5", "linux"), ("", "hided:lib:Bad-Name", "", "5", "linux"),
            ("", "", "web/src/nothere.test.ts", "5", "linux"), ("", "", "site/tests/legal.test.mjs", "5", "linux"),
        ]:
            with self.subTest(web=web, rust=rust, units=units, count=count, runner=runner), self.assertRaises(ValueError):
                repeat.resolve("workflow_dispatch", web, rust, count, runner, units)


class Command(unittest.TestCase):
    """The script as the workflow runs it."""

    def run_script(self, *args, stdin=None, needs=None):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), *args], input=stdin, text=True, capture_output=True,
                env={**os.environ, "GITHUB_OUTPUT": str(output), **({"NEEDS": json.dumps(needs)} if needs is not None else {})},
            )
            written = output.read_text(encoding="utf-8") if output.exists() else ""
        return result, dict(line.split("=", 1) for line in written.splitlines())

    def test_hand_and_pr_decisions_always_write_both_run_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            # One test of 150 s: 10 repeats on 3 shards; 8 of 30 s: 10 on 4.
            table = Path(directory) / "durations.json"
            table.write_text(json.dumps({"s3.spec.ts › slow": 150, **{f"s3.spec.ts › t{i}": 30 for i in range(8)}}))
            slow, many = Path(directory) / "slow.txt", Path(directory) / "many.txt"
            slow.write_text(listing("s3.spec.ts › slow"), encoding="utf-8")
            many.write_text(listing(*(f"s3.spec.ts › t{i}" for i in range(8))), encoding="utf-8")
            web_only = ["--table", str(table), "--web-list"]
            for event, web, rust, extra, expected in [
                ("workflow_dispatch", "web/e2e/s3.spec.ts", "[]", ["--repeat", "10"], ("true", "false", "0", "10", "1", "1")),
                ("workflow_dispatch", "", '[{"package":"hided","module":"seat"}]', ["--repeat", "3"],
                 ("false", "true", "3", "0", "0", "0")),
                ("pull_request", "web/e2e/s3.spec.ts", "[]", [*web_only, str(slow)], ("true", "false", "0", "10", "3", "3")),
                # The web specs are planned by their seconds and the Rust
                # tests by their count, each with its own repeat.
                ("pull_request", "web/e2e/s3.spec.ts", '[{"package":"hided","module":"seat"}]',
                 [*web_only, str(many), "--rust-count", "47"], ("true", "true", "4", "10", "4", "1")),
                ("pull_request", "", "[]", [], ("false", "false", "0", "0", "0", "0")),
            ]:
                with self.subTest(event=event, expected=expected):
                    result, outputs = self.run_script("choose", "--event", event, "--web", web, "--rust", rust, *extra)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual((outputs["run-web"], outputs["run-rust"], outputs["repeat"], outputs["web-repeat"],
                                      outputs["web-shards"], outputs["web-parts"]), expected)

    def test_a_pr_selection_needs_a_real_positive_count(self):
        with tempfile.TemporaryDirectory() as directory:
            empty = Path(directory) / "list.txt"
            empty.write_text("Listing tests:\nTotal: 0 tests in 0 files\n", encoding="utf-8")
            for arguments in ([], ["--web-list", str(empty)], ["--web-list", str(Path(directory) / "missing.txt")]):
                with self.subTest(web=arguments):
                    result, outputs = self.run_script("choose", "--event", "pull_request", "--web", "web/e2e/s3.spec.ts",
                                                      "--rust", "[]", *arguments)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(outputs, {})
        for kind, selection in (("units", '["web/src/searchDetail.test.ts"]'),
                                ("rust", '[{"package":"hided","test":"lib","module":"seat"}]')):
            for count in ("", "0", "-1"):
                with self.subTest(kind=kind, count=count):
                    result, outputs = self.run_script(
                        "choose", "--event", "pull_request", "--web", "",
                        "--units", selection if kind == "units" else "[]",
                        "--rust", selection if kind == "rust" else "[]", f"--{kind}-count", count,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(outputs, {})

    def test_the_workflow_verdict_exits_nonzero_for_a_successful_zero_run(self):
        needs = {"count": {"result": "success", "outputs": {"run-web": "true", "run-units": "false", "run-rust": "false",
                                                            "repeat": "0", "web-repeat": "10", "units-legs": "[]"}},
                 "web": {"result": "success", "outputs": {"completed": "0"}}, "units": {"result": "skipped"},
                 "rust": {"result": "skipped"}}
        result, _ = self.run_script("verdict", needs=needs)
        self.assertEqual(result.returncode, 1)
        self.assertIn("0 completed test runs", result.stderr)
        needs["web"]["outputs"]["completed"] = "10"
        result, _ = self.run_script("verdict", needs=needs)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_hand_run_resolves_to_outputs_the_jobs_read(self):
        result, outputs = self.run_script(
            "resolve", "--event", "workflow_dispatch", "--web", "s3.spec.ts", "--units", "web/src/searchDetail.test.ts",
            "--rust", "hided:it:handshake", "--repeat", "10", "--runner", "windows",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(outputs["runner"], "windows-latest")
        self.assertEqual(outputs["os"], "windows")
        self.assertEqual(outputs["web"], "web/e2e/s3.spec.ts")
        self.assertEqual(json.loads(outputs["units"]), ["web/src/searchDetail.test.ts"])
        self.assertEqual(json.loads(outputs["rust"]), [{"package": "hided", "test": "it", "module": "handshake"}])
        # A hand run counts no tests, and its unit files are listed by the
        # units leg on the system asked for, where their tests may be the
        # only ones that run.
        self.assertEqual((outputs["repeat"], outputs["counted"], outputs["web-count-needed"], outputs["units-count-needed"],
                          outputs["rust-count-needed"]), ("10", "false", "false", "false", "false"))

    def test_a_pull_request_resolves_with_its_rust_count_and_no_repeat_yet(self):
        rust = json.dumps([{"package": "hided", "test": "lib", "module": "handshake"}], separators=(",", ":"))
        result, outputs = self.run_script("resolve", "--event", "pull_request", "--web", '["web/e2e/s3.spec.ts"]',
                                          "--units", '["web/src/searchDetail.test.ts"]', "--rust", rust)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((outputs["repeat"], outputs["counted"]), ("", "true"))
        self.assertEqual((outputs["web-count-needed"], outputs["units-count-needed"], outputs["rust-count-needed"]),
                         ("true", "true", "true"))

    def test_a_hand_run_names_its_unit_files_and_the_count_lists_none_of_them(self):
        _, outputs = self.run_script("resolve", "--event", "workflow_dispatch", "--units", "web/src/searchDetail.test.ts",
                                    "--repeat", "3")
        self.assertEqual((outputs["units"], outputs["units-count-needed"]), ('["web/src/searchDetail.test.ts"]', "false"))
        _, outputs = self.run_script("resolve", "--event", "workflow_dispatch", "--web", "s3.spec.ts", "--repeat", "3")
        self.assertEqual((outputs["units"], outputs["units-count-needed"]), ("[]", "false"))

    def test_a_refusal_is_one_line_and_a_failed_exit(self):
        result, outputs = self.run_script("resolve", "--event", "workflow_dispatch", "--web", "nothing.spec.ts", "--repeat", "5")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr.strip(), "ci-repeat: no such spec: web/e2e/nothing.spec.ts")
        self.assertEqual(outputs, {})

    def test_the_commands_the_jobs_pipe_through(self):
        result, _ = self.run_script("count-nextest", stdin=json.dumps(NEXTEST_LIST))
        self.assertEqual(result.stdout.strip(), "3")
        result, _ = self.run_script("spec-patterns", "web/e2e/s3.spec.ts")
        self.assertEqual(result.stdout.strip(), r"web/e2e/s3\.spec\.ts$")
        # A plan whose flags are decided, as the count job writes it: an
        # undecided one asks nextest on the runner, which this job has not.
        plan = json.dumps([{"package": "hided", "test": "it", "filter": "test(/^(seat)::/)", "tests": 1, "run_ignored": ""},
                           {"package": "hide-platform", "test": "lib", "filter": "", "tests": 9, "run_ignored": "ignored-only"}])
        result, _ = self.run_script("plan-lines", "--plan", plan)
        self.assertEqual((result.returncode, result.stderr), (0, ""))
        self.assertEqual(result.stdout, "hided\x1fit\x1ftest(/^(seat)::/)\x1f\x1fhided-it.tar.zst\n"
                                        "hide-platform\x1flib\x1f\x1fignored-only\x1fhide-platform-lib.tar.zst\n")
        result, outputs = self.run_script("shard-share", "--shard", "2/3", "--parts", "3", "--repeat", "10")
        self.assertEqual((result.returncode, result.stdout), (0, "1/1 3\n"))
        result, _ = self.run_script("shard-share", "--shard", "2/3", "--parts", "2", "--repeat", "10")
        self.assertEqual(result.returncode, 1)
        units = json.dumps([{"package": "web", "files": ["web/src/a.test.ts"], "tests": 2},
                            {"package": "desktop", "files": ["desktop/src/main/env.test.ts"], "tests": 5}])
        result, _ = self.run_script("units-lines", "--plan", units)
        self.assertEqual(result.stdout, "web\tweb/src/a.test.ts\ndesktop\tdesktop/src/main/env.test.ts\n")

    def test_every_line_the_jobs_read_ends_with_lf_even_on_a_windows_runner(self):
        # Windows translates `\n` to `\r\n` on stdout, and the trailing `\r`
        # reached nextest as unparsed input (run 43619979993).
        plan = json.dumps([{"package": "hided", "test": "it", "filter": "test(/^(seat)::/)", "tests": 1, "run_ignored": "ignored-only"}])
        result, _ = self.run_script("plan-lines", "--plan", plan)
        self.assertEqual((result.returncode, result.stderr), (0, ""))
        self.assertEqual(result.stdout, "hided\x1fit\x1ftest(/^(seat)::/)\x1fignored-only\x1fhided-it.tar.zst\n")
        self.assertNotIn("\r", result.stdout)

    def test_the_rust_job_reads_an_empty_filterset_as_its_own_field(self):
        # A tab-separated line lost its empty filterset to `read`, which folds
        # a run of tabs, and the ignored flag became the filterset.
        plan = json.dumps([{"package": "hide-platform", "test": "lib", "filter": "", "tests": 9, "run_ignored": "ignored-only"}])
        result, _ = self.run_script("plan-lines", "--plan", plan)
        read = subprocess.run(
            ["bash", "-c", "while IFS=$'\\x1f' read -r package test expression run_ignored archive; do "
                           "printf '%s|%s|%s|%s|%s\\n' \"$package\" \"$test\" \"$expression\" \"$run_ignored\" \"$archive\"; done"],
            input=result.stdout, text=True, capture_output=True, check=True,
        )
        self.assertEqual(read.stdout, "hide-platform|lib||ignored-only|hide-platform-lib.tar.zst\n")


class Verdict(unittest.TestCase):
    """Chosen tests cannot disappear between selection and a green workflow."""

    def needs(self, event, kind, completed="30", result="success"):
        selected = {"web": kind == "web", "units": kind == "units", "rust": kind == "rust"}
        decision = repeat.choose(event, "web/e2e/s3.spec.ts" if selected["web"] else "",
                                 [{"package": "hided", "test": "lib", "module": "seat"}] if selected["rust"] else [],
                                 "3", {"tests": 10, "seconds": 900.0, "repeat": 3, "shards": 2, "parts": 1, "heaviest": 600.0},
                                 "100", ["web/src/searchDetail.test.ts"] if selected["units"] else [], "100")
        # A units leg names its outputs by its system and says the repeat it ran.
        outputs = ({f"completed-{leg['os']}": completed, f"repeat-{leg['os']}": str(leg["repeat"])}
                   for leg in json.loads(decision["units-legs"])) if kind == "units" else [{"completed": completed}]
        return {"count": {"result": "success", "outputs": decision},
                "web": {"result": "skipped", "outputs": {}}, "units": {"result": "skipped", "outputs": {}},
                "rust": {"result": "skipped", "outputs": {}},
                kind: {"result": result, "outputs": {key: value for leg in outputs for key, value in leg.items()}}}

    def test_a_hand_run_and_a_pr_require_actual_runs_of_each_chosen_kind(self):
        for event in ("workflow_dispatch", "pull_request"):
            for kind in ("web", "units", "rust"):
                with self.subTest(event=event, kind=kind):
                    repeat.verdict(self.needs(event, kind))
                    for count in ("", "0", "null", "-1", "1", "29"):
                        with self.subTest(completed=count), self.assertRaises(ValueError):
                            repeat.verdict(self.needs(event, kind, completed=count))
                    for outcome in ("skipped", "cancelled", "failure"):
                        with self.subTest(outcome=outcome), self.assertRaises(ValueError):
                            repeat.verdict(self.needs(event, kind, result=outcome))

    def test_a_pull_request_runs_a_file_linux_lists_no_test_of_on_windows_at_the_repeat_listed_there(self):
        linux, windows = "desktop/src/main/env.test.ts", "desktop/e2e/windows-processes.unit.ts"
        decision = repeat.choose("pull_request", "", [], "", None, "", [linux, windows], "5", [windows])
        self.assertEqual(json.loads(decision["units-legs"]), [
            {"os": "linux", "runner": "ubuntu-latest", "files": [linux], "repeat": 10},
            {"os": "windows", "runner": "windows-latest", "files": [windows], "repeat": 0}])

        def needs(**units):
            outputs = {"completed-linux": "50", "repeat-linux": "10", "completed-windows": "20", "repeat-windows": "10", **units}
            return {"count": {"result": "success", "outputs": decision}, "web": {"result": "skipped"},
                    "rust": {"result": "skipped"},
                    "units": {"result": "success", "outputs": {key: value for key, value in outputs.items() if value is not None}}}

        repeat.verdict(needs())
        # The Windows leg is held to the repeat it chose, the Linux leg to the count's.
        for units in ({"completed-windows": "0"}, {"completed-windows": None}, {"completed-windows": "15"},
                      {"repeat-windows": "0"}, {"repeat-windows": None}, {"repeat-linux": "5", "completed-linux": "25"}):
            with self.subTest(units=units), self.assertRaises(ValueError):
                repeat.verdict(needs(**units))
        # Windows files alone: no Linux leg, and no shared repeat.
        alone = repeat.choose("pull_request", "", [], "", None, "", [windows], "0", [windows])
        self.assertEqual((alone["repeat"], alone["run-units"], len(json.loads(alone["units-legs"]))), (0, "true", 1))
        repeat.verdict({"count": {"result": "success", "outputs": alone}, "web": {"result": "skipped"}, "rust": {"result": "skipped"},
                        "units": {"result": "success", "outputs": {"completed-windows": "20", "repeat-windows": "10"}}})

    def test_files_moved_elsewhere_must_be_selected_and_a_linux_file_must_list_tests(self):
        for units, count, elsewhere in ((["web/src/a.test.ts"], "3", ["web/src/b.test.ts"]),
                                        (["web/src/a.test.ts", "web/src/b.test.ts"], "0", ["web/src/b.test.ts"])):
            with self.subTest(units=units, count=count), self.assertRaises(ValueError):
                repeat.choose("pull_request", "", [], "", None, "", units, count, elsewhere)

    def test_a_hand_runs_unit_files_are_one_leg_on_the_system_it_asked_for(self):
        decision = repeat.choose("workflow_dispatch", "", [], "7", None, "", ["web/e2e/windows-sockets.unit.ts"], "", None, "windows")
        self.assertEqual(json.loads(decision["units-legs"]),
                         [{"os": "windows", "runner": "windows-latest", "files": ["web/e2e/windows-sockets.unit.ts"], "repeat": 7}])

    def test_missing_decisions_failed_count_and_missing_jobs_fail_closed(self):
        for field in ("run-web", "run-units", "run-rust", "repeat", "web-repeat", "units-legs"):
            needs = self.needs("workflow_dispatch", "web")
            del needs["count"]["outputs"][field]
            with self.subTest(field=field), self.assertRaises((ValueError, KeyError)):
                repeat.verdict(needs)
        needs = self.needs("pull_request", "rust")
        needs["count"]["result"] = "failure"
        with self.assertRaises(ValueError):
            repeat.verdict(needs)
        del needs["count"]
        with self.assertRaises(KeyError):
            repeat.verdict(needs)

    def test_only_a_pr_with_nothing_chosen_may_skip_every_leg(self):
        decision = repeat.choose("pull_request", "", [], "", None, "")
        needs = {"count": {"result": "success", "outputs": decision},
                 "web": {"result": "skipped"}, "units": {"result": "skipped"}, "rust": {"result": "skipped"}}
        repeat.verdict(needs)
        for field in ("repeat", "web-repeat"):
            with self.subTest(field=field), self.assertRaises(ValueError):
                repeat.verdict({**needs, "count": {"result": "success", "outputs": {**decision, field: "10"}}})

    def test_each_leg_is_proved_against_its_own_repeat(self):
        # The web specs repeat 3 times and the Rust tests 4: 30 web runs prove
        # the web leg and not the Rust one, and 40 Rust runs the reverse.
        decision = repeat.choose("pull_request", "web/e2e/s3.spec.ts", [{"package": "hided", "test": "lib", "module": "seat"}],
                                 "", {"tests": 10, "seconds": 900.0, "repeat": 3, "shards": 2, "parts": 1, "heaviest": 600.0},
                                 "50")
        self.assertEqual((decision["web-repeat"], decision["repeat"]), (3, 4))

        def needs(web, rust):
            return {"count": {"result": "success", "outputs": decision}, "units": {"result": "skipped"},
                    "web": {"result": "success", "outputs": {"completed": web}},
                    "rust": {"result": "success", "outputs": {"completed": rust}}}

        repeat.verdict(needs("30", "200"))
        for web, rust in (("40", "200"), ("30", "30")):
            with self.subTest(web=web, rust=rust), self.assertRaises(ValueError):
                repeat.verdict(needs(web, rust))

    def test_the_required_verify_aggregate_receives_a_failed_repeat_lane(self):
        lanes = {"lanes": '["repeat"]'}
        needs = {lane: {"result": "failure" if lane == "repeat" else "skipped"} for lane in ci.LANES}
        needs["plan"] = {"result": "success", "outputs": lanes}
        with self.assertRaises(ValueError):
            ci.aggregate(needs)


class Reports(unittest.TestCase):
    """Execution reports, rather than a list or a successful shell, prove work."""

    def test_playwright_counts_passed_results_inside_nested_suites_and_repeat_copies(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            for statuses, expected in ((["passed"] * 10 + ["skipped"], 10), (["skipped"], 0), ([], 0)):
                path.write_text(json.dumps({"suites": [{"suites": [{"specs": [{"tests": [
                    {"results": [{"status": status}]} for status in statuses
                ]}]}]}]}), encoding="utf-8")
                if expected:
                    self.assertEqual(repeat.completed_runs(str(path), None), expected)
                else:
                    with self.assertRaisesRegex(ValueError, "zero tests"):
                        repeat.completed_runs(str(path), None)
            path.write_text('{"errors":[{"message":"No tests found"}]}', encoding="utf-8")
            with self.assertRaises(ValueError):
                repeat.completed_runs(str(path), None)

    def test_nextest_counts_stress_iterations_and_reports_from_multiple_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / f"junit.{index}.xml" for index in range(2)]
            for path in paths:
                path.write_text('<testsuites>' + ''.join(
                    f'<testsuite name="it@stress-{index}"><testcase name="one"/><testcase name="ignored"><skipped/></testcase></testsuite>'
                    for index in range(3)
                ) + '</testsuites>', encoding="utf-8")
            self.assertEqual(repeat.completed_runs(None, paths), 6)
            # A units leg's count is named by its system, so the legs of one
            # matrix job do not overwrite each other's.
            with tempfile.TemporaryDirectory() as outputs:
                written = Path(outputs) / "output"
                subprocess.run([sys.executable, str(SCRIPT), "report", "--junit", *map(str, paths), "--leg", "windows"],
                               check=True, capture_output=True, env={**os.environ, "GITHUB_OUTPUT": str(written)})
                self.assertEqual(written.read_text(encoding="utf-8"), "completed-windows=6\n")
            paths[0].write_text('<testsuites><testsuite><testcase><skipped/></testcase></testsuite></testsuites>', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "zero tests"):
                repeat.completed_runs(None, paths[:1])
            with self.assertRaises(OSError):
                repeat.completed_runs(None, [Path(directory) / "missing.xml"])


STRESS = importlib.util.spec_from_file_location("check_stress", ROOT / "scripts/check-stress-report.py")
stress = importlib.util.module_from_spec(STRESS)
STRESS.loader.exec_module(stress)


class StressReport(unittest.TestCase):
    """A repeated run whose iterations include a failure fails the lane, even
    when nextest's own exit status says the run passed.

    nextest 0.9.146's `--stress-count N` exits 0 when the last iteration
    passes; a `wsl device windows` job ended successful on 2026-11-14 with
    `20/20 stress run iterations: 19 passed, 1 failed` in its log
    (run 43626029599).
    """

    def run_script(self, *args):
        return subprocess.run([sys.executable, str(ROOT / "scripts/check-stress-report.py"), *args],
                              text=True, capture_output=True)

    def report(self, directory, failures=()):
        """A nextest JUnit report of ten iterations, the named ones failing."""
        path = Path(directory) / "junit.xml"
        cases = {True: '<testcase name="seat::a_test"><failure message="boom"/></testcase>',
                 False: '<testcase name="seat::a_test"/>'}
        suites = "".join(
            f'<testsuite name="it@stress-{iteration}">{cases[iteration in failures]}</testsuite>'
            for iteration in range(1, 11)
        )
        path.write_text(f"<testsuites>{suites}</testsuites>", encoding="utf-8")
        return path

    def test_every_iteration_passing_passes_the_check(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_script("--junit", str(self.report(directory)), "--repeat", "10")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_one_failed_iteration_fails_the_check_and_names_the_test(self):
        for failure in (1, 5, 9, 10):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                result = self.run_script("--junit", str(self.report(directory, {failure})))
                self.assertEqual(result.returncode, 1)
                self.assertIn("seat::a_test", result.stderr)
                self.assertIn(f"iteration {failure}", result.stderr)
                self.assertIn("1 failed test runs", result.stderr)

    def test_the_stress_summary_line_alone_fails_the_check(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "nextest.log"
            log.write_text("     Summary [ 778.209s] 20/20 stress run iterations: 19 passed, 1 failed\n", encoding="utf-8")
            result = self.run_script("--summary", str(log))
            self.assertEqual(result.returncode, 1)
            self.assertIn("1 failed iterations of 20", result.stderr)
            log.write_text("     Summary [ 12.000s] 10/10 stress run iterations: 10 passed\n", encoding="utf-8")
            result = self.run_script("--summary", str(log))
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_report_shorter_than_the_repeat_asked_for_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "nextest.log"
            log.write_text("     Summary [ 12.000s] 4/4 stress run iterations: 4 passed\n", encoding="utf-8")
            result = self.run_script("--summary", str(log), "--repeat", "10")
            self.assertEqual(result.returncode, 1)
            self.assertIn("fewer than the 10 asked for", result.stderr)

    def test_a_log_with_no_stress_summary_proves_no_iteration_when_a_repeat_was_asked(self):
        # A lane can pass the reports of a run that never repeated anything,
        # or whose log the step did not keep: with no summary line, nothing
        # shows how many iterations ran, so the repeat is unproven. This is
        # also what a lane that forgot its `--stress-count` looks like, and
        # the check has to tell that apart from a run whose iterations passed.
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "nextest.log"
            log.write_text("    Starting 3 tests across 1 binary\n     Summary [ 1.2s] 3 tests run: 3 passed\n",
                           encoding="utf-8")
            report = self.report(directory)
            result = self.run_script("--junit", str(report), "--summary", str(log), "--repeat", "10")
            self.assertEqual(result.returncode, 1)
            self.assertIn("no stress summary", result.stderr)
            # The same report without a summary handed in stays readable: the
            # JUnit report alone is what a lane without a log can pass.
            result = self.run_script("--junit", str(report), "--repeat", "10")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_the_summary_nextest_prints_for_one_suite_is_read_without_a_total(self):
        # nextest's own parser leaves the `/M` total optional and anchors the
        # line at its end, so a cancelled run's suffixed line is no summary.
        self.assertEqual(stress.summary_failures("Summary [ 2.4s] 5 stress run iterations: 5 passed\n"), (5, 0))
        self.assertEqual(stress.summary_failures("Summary [ 2.4s] 5/5 stress run iterations: 4 passed, 1 failed\n"), (5, 1))
        self.assertEqual(stress.summary_failures(
            "Summary [ 2.4s] 5/5 stress run iterations: 4 passed, 1 failed; cancelled due to INT signal\n"), (0, 0))
        # A Windows runner's tee leaves CRLF, which must not read as "no summary".
        self.assertEqual(stress.summary_failures(
            "Summary [ 2.4s] 5/5 stress run iterations: 4 passed, 1 failed\r\n"), (5, 1))
        # A colorized summary is the same summary, not an unproven run.
        self.assertEqual(stress.summary_failures(
            "\x1b[1m     Summary\x1b[0m [ 2.4s] 5/5 stress run iterations: 4 passed, "
            "\x1b[31m1 failed\x1b[0m\n"), (5, 1))
        # The per-iteration line is not the summary, so it proves no whole run.
        self.assertEqual(stress.summary_failures("Stress test [ 0.1s] iteration 1/5: 3 tests run: 3 passed\n"), (0, 0))
        # Several runs in one log (a lane that ran more than one target) add up.
        self.assertEqual(stress.summary_failures(
            "Summary [ 1.0s] 3/3 stress run iterations: 2 passed, 1 failed\n"
            "Summary [ 2.0s] 4/4 stress run iterations: 4 passed\n"), (7, 1))

    def test_a_missing_report_or_no_input_at_all_fails_rather_than_passing_silently(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("no report or summary", result.stderr)
        result = self.run_script("--junit", "/nonexistent/junit.xml")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no such report", result.stderr)

    def test_one_report_per_iteration_is_read_by_its_file_name(self):
        # `repeat.yml`'s unit leg has no `--repeat-each` to use, so it writes
        # one Vitest report per iteration: a failure in the second names that
        # report, and a run whose iterations all passed still passes.
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for iteration in (1, 2, 3):
                path = Path(directory) / f"junit-web-src-a-test-ts.{iteration}.xml"
                failure = '<failure message="boom"/>' if iteration == 2 else ""
                path.write_text(
                    f'<testsuites><testsuite name="src/a.test.ts"><testcase name="a test">{failure}</testcase></testsuite></testsuites>',
                    encoding="utf-8")
                paths.append(path)
            result = self.run_script("--junit", *map(str, paths), "--repeat", "3")
            self.assertEqual(result.returncode, 1)
            self.assertIn("a test", result.stderr)
            self.assertIn("junit-web-src-a-test-ts.2", result.stderr)
            paths[1].write_text(
                '<testsuites><testsuite name="src/a.test.ts"><testcase name="a test"/></testsuite></testsuites>',
                encoding="utf-8")
            result = self.run_script("--junit", *map(str, paths), "--repeat", "3")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_skipped_iteration_is_neither_a_pass_that_proves_repeats_nor_a_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "junit.xml"
            path.write_text('<testsuites><testsuite name="it@stress-1"><testcase name="a"><skipped/></testcase></testsuite></testsuites>',
                            encoding="utf-8")
            self.assertEqual(stress.junit_failures([path]), [])


if __name__ == "__main__":
    unittest.main()


class IgnoredPreparation(unittest.TestCase):
    """What an ignored test needs is prepared once, and both lanes call that.

    `pr.yml` and `repeat.yml` run ignored tests (`--run-ignored only`) that
    start the built pane holder and real login shells. The list of what they
    need is one script, so a test that needs one more thing is a change in one
    place and not in two workflows that then drift.
    """

    def workflow(self, name):
        import yaml
        return yaml.safe_load((ROOT / ".github/workflows" / name).read_text())["jobs"]

    def steps_running(self, job, needle):
        return [step for step in job["steps"] if needle in (step.get("run") or "")]

    def test_the_script_prepares_the_holder_and_the_shells_it_names(self):
        script = (ROOT / "scripts/prepare-ignored-tests.sh").read_text(encoding="utf-8")
        self.assertIn("HIDE_TEST_HOLDER_BIN", script)
        self.assertIn("HIDE_HOLDER_CONPTY", script)
        # The shells are named once, as the table the loop reads, so a shell
        # added to the table is the one change its new variable needs.
        for shell in ("ZSH:zsh", "FISH:fish", "BASH:bash", "TCSH:tcsh", "CSH:bsd-csh"):
            self.assertIn(shell, script)
        self.assertIn("verify-cargo.sh build -p hide-holder --bin hide-holder", script)
        self.assertIn("fetch-conpty.py", script)

    def test_both_lanes_prepare_ignored_tests_with_that_script_and_nothing_of_their_own(self):
        windows = self.workflow("windows-check.yml")["tests"]
        rust = self.workflow("rust.yml")["build"]
        repeat = self.workflow("repeat.yml")["rust"]
        for job, lane in ((rust, "rust"), (windows, "windows-check"), (repeat, "rust")):
            with self.subTest(lane=lane):
                sourced = self.steps_running(job, ". scripts/prepare-ignored-tests.sh")
                self.assertTrue(sourced, f"{lane} calls no shared preparation")
                for step in job["steps"]:
                    run = step.get("run") or ""
                    self.assertNotIn("HIDE_TEST_HOLDER_BIN=$PWD", run, f"{lane} prepares the holder itself: {step.get('name')}")
                    self.assertNotIn("--bin hide-holder", run, f"{lane} builds the holder itself: {step.get('name')}")

    def test_a_hand_run_asks_this_runner_which_flag_its_modules_need(self):
        # A hand run names modules and lists nothing, since the system asked
        # for may not be the count job's. The Rust job asks nextest where it
        # runs: a module with no default match and ignored matches is proved
        # with `--run-ignored ignored-only`, and a module with a mix (the
        # whole hided binary) runs its ordinary tests only.
        asked = []

        def lister(package, test, expression, run_ignored=""):
            asked.append((package, expression, run_ignored))
            return {"": (0, 0), "ignored-only": (10, 10), "all": (11, 10)}[run_ignored]

        plan = repeat.plan_run_ignored([{"package": "hide-core", "test": "lib",
                                         "filter": "test(/^(engine::contract)::/)"}], lister)
        self.assertEqual(plan, [{"package": "hide-core", "test": "lib", "filter": "test(/^(engine::contract)::/)",
                                 "run_ignored": "ignored-only"}])

        mixed = {}

        def mixed_lister(package, test, expression, run_ignored=""):
            return {"": (5, 1), "all": (7, 1)}[run_ignored]

        self.assertEqual(
            repeat.plan_run_ignored([{"package": "hided", "test": "it", "filter": ""}], mixed_lister)[0]["run_ignored"],
            "",
        )
        # A plan the count job already decided is not asked again, an empty
        # flag included: a pull request's plan never lists on the Rust runner.
        for flag in ("ignored-only", ""):
            decided = {"package": "hide-kit", "test": "lib", "filter": "", "run_ignored": flag}
            self.assertEqual(repeat.plan_run_ignored([decided], lambda *args: self.fail("asked again")), [decided])
        self.assertEqual(mixed, {})

    def test_the_repeat_lane_prepares_only_where_the_plan_asked_for_an_ignored_test(self):
        repeat = self.workflow("repeat.yml")["rust"]
        step = self.steps_running(repeat, "plan-lines")[0]["run"]
        # The same decision the `--run-ignored` flag comes from, so a plan
        # with no ignored test builds nothing.
        self.assertLess(step.index("if [ -n \"$run_ignored\" ]"), step.index(". scripts/prepare-ignored-tests.sh"))


class RustLegRuns(unittest.TestCase):
    """What the repeat lane's Rust leg keeps and prepares for the tests it repeats.

    A repeated run is proved by two things the lane has to make itself: the
    nextest summary it reads afterward, and, on Windows, the ConPTY runtime a
    pane holder runs on. Both were missing on real runs (43658075817 and
    43658081062), and neither is about the tests a pull request changed.
    """

    def workflow(self):
        import yaml
        return yaml.safe_load((ROOT / ".github/workflows/repeat.yml").read_text())["jobs"]["rust"]

    def steps_where(self, predicate):
        return [step for step in self.workflow()["steps"] if predicate(step)]

    def test_every_repeated_nextest_run_keeps_both_streams_its_summary_can_take(self):
        # nextest writes its stress summary to standard error, so a tee of
        # standard output alone can leave the log without one; the report step
        # then fails a run whose every iteration passed.
        repeats = self.steps_where(lambda step: "verify-cargo.sh nextest" in (step.get("run") or ""))
        self.assertTrue(repeats, "the lane repeats no Rust run")
        for step in repeats:
            with self.subTest(step=step.get("name")):
                run = step["run"]
                self.assertIn("2>&1", run)
                self.assertLess(run.index("2>&1"), run.index("| tee"))

    def test_the_sibling_lanes_this_one_is_held_to_read_the_same_way(self):
        # The rule is the lane's, not this file's: the two lanes that already
        # got it right are the reference a reader can check. They write it
        # across a continuation, so the pipe is found by the joined line.
        for name in ("remote-mailbox.yml", "wsl-device.yml"):
            text = (ROOT / ".github/workflows" / name).read_text(encoding="utf-8")
            self.assertTrue(
                re.search(r"2>&1\s*(?:\\\n)?\s*\| tee", text),
                f"{name} tees only one stream of a repeated run",
            )

    def test_the_windows_leg_has_the_conpty_runtime_its_holder_tests_start_on(self):
        # `hide-holder::it` starts a pane holder, which runs on the pinned
        # ConPTY runtime; without the variable every test of it stops in its
        # fixture before reaching its own body.
        fetched = self.steps_where(lambda step: "fetch-conpty.py" in (step.get("run") or ""))
        self.assertTrue(fetched, "no step fetches the ConPTY runtime")
        for step in fetched:
            self.assertEqual(step.get("if"), "runner.os == 'Windows'")
            self.assertIn("HIDE_HOLDER_CONPTY", (step.get("env") or {}))
        # The repeated run hands it on: an ordinary module never reaches the
        # preparation that exports it for an ignored test.
        repeating = self.steps_where(lambda step: "verify-cargo.sh nextest" in (step.get("run") or ""))
        self.assertTrue(any("HIDE_HOLDER_CONPTY" in (step.get("env") or {}) for step in repeating))

    def test_the_runtime_is_fetched_for_a_runner_not_for_a_module_name(self):
        # A hand run of any crate's module on Windows needs it, so the
        # condition is the runner: a step keyed to one crate's name would miss
        # the next crate that starts a holder.
        step = self.steps_where(lambda step: "fetch-conpty.py" in (step.get("run") or ""))[0]
        self.assertEqual(step["if"], "runner.os == 'Windows'")
        self.assertNotIn("hide-holder", step["if"])

    def test_the_listing_calls_bash_by_the_path_it_resolved(self):
        # `subprocess` on Windows searches System32 before PATH, where
        # `bash.exe` is the WSL launcher: a bare "bash" made every listing fail
        # there with "Windows Subsystem for Linux has no installed
        # distributions", while the lane's own steps, run by Git Bash, went on
        # (run 43661175346). The lister names the resolved program instead.
        source = (ROOT / "scripts/ci-repeat.py").read_text(encoding="utf-8")
        self.assertNotIn('["bash", "scripts/verify-cargo.sh"', source)
        self.assertIn('[bash(), "scripts/verify-cargo.sh"', source)
        # The helper is the repository's own answer to that trap.
        self.assertIn("shutil.which", source)


class RustArchive(unittest.TestCase):
    """A pull request's Rust tests are built once: the listing builds them, the
    archive packs what it built, and the Rust leg runs the archive."""

    PLAN = [{"package": "hided", "test": "it", "filter": "test(/^(seat)::/)", "tests": 3, "run_ignored": ""},
            {"package": "hide-kit", "test": "lib", "filter": "", "tests": 9, "run_ignored": ""},
            {"package": "hide-agent-hooks", "test": "bin", "filter": "", "tests": 2, "run_ignored": ""}]

    def cargo_arguments(self, command):
        """What a `verify-cargo.sh` call hands cargo to pick the tests it builds."""
        arguments = command[command.index("scripts/verify-cargo.sh") + 2:]
        if command[command.index("scripts/verify-cargo.sh") + 1] == "nextest-archive":
            arguments = arguments[1:]
        picked, skip = [], False
        for index, argument in enumerate(arguments):
            if skip:
                skip = False
            elif argument in ("--profile", "-E", "--message-format", "--run-ignored"):
                skip = True
            else:
                picked.append(argument)
        return picked

    def test_each_target_is_packed_with_the_cargo_arguments_that_listed_it(self):
        # Same arguments, same build: cargo finds the listed tests fresh and
        # only packs them, so no test compiles twice.
        calls = []

        def run(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps(NEXTEST_LIST), stderr="")

        original = repeat.subprocess.run
        repeat.subprocess.run = run
        try:
            with tempfile.TemporaryDirectory() as directory:
                for entry in self.PLAN:
                    repeat.nextest_lister(entry["package"], entry["test"], entry["filter"])
                repeat.rust_archive(self.PLAN, directory, repeat.nextest_packer)
                packed = [Path(command[command.index("nextest-archive") + 1]) for command in calls[3:]]
                self.assertEqual([path.parent for path in packed], [Path(directory)] * 3)
        finally:
            repeat.subprocess.run = original
        listed, archived = calls[:3], calls[3:]
        self.assertEqual([self.cargo_arguments(command) for command in listed],
                         [self.cargo_arguments(command) for command in archived])
        self.assertEqual([self.cargo_arguments(command) for command in archived],
                         [["-p", "hided", "--test", "it"], ["-p", "hide-kit", "--lib"], ["-p", "hide-agent-hooks", "--bins"]])
        self.assertEqual([path.name for path in packed], ["hided-it.tar.zst", "hide-kit-lib.tar.zst", "hide-agent-hooks-bin.tar.zst"])

    def test_a_plan_that_asks_for_an_ignored_test_or_is_malformed_packs_nothing(self):
        for entry in ({**self.PLAN[0], "run_ignored": "ignored-only"}, {**self.PLAN[0], "test": "doc"},
                      {**self.PLAN[0], "filter": "all()"}):
            with self.subTest(entry=entry), self.assertRaises(ValueError):
                repeat.rust_archive([entry], "unused", lambda *args: self.fail("packed"))

    def workflow(self):
        import yaml
        return yaml.safe_load((ROOT / ".github/workflows/repeat.yml").read_text())["jobs"]

    def test_the_count_job_keeps_the_archives_it_packed_after_the_listing(self):
        steps = self.workflow()["count"]["steps"]
        names = [step.get("id") or step.get("name") for step in steps]
        self.assertLess(names.index("rust-count"), names.index("rust-archive"))
        kept = next(step for step in steps if step.get("uses", "").startswith("actions/upload-artifact"))
        self.assertEqual(kept["with"]["name"], "rust-repeat-tests")
        self.assertEqual(self.workflow()["count"]["outputs"]["rust-archived"],
                         "${{ steps.rust-archive.outputs.rust-archived || 'false' }}")

    def test_a_pull_requests_rust_leg_runs_the_archive_and_builds_nothing(self):
        steps = self.workflow()["rust"]["steps"]
        archived = "needs.count.outputs.rust-archived == 'true'"
        # The caches a build reads are a hand run's alone.
        for action in ("./.github/actions/rust-cache", "./.github/actions/vt-cache"):
            step = next(step for step in steps if step.get("uses") == action)
            self.assertEqual(step["if"], "needs.count.outputs.rust-archived != 'true'")
        fetched = next(step for step in steps if step.get("uses", "").startswith("actions/download-artifact"))
        self.assertEqual((fetched["if"], fetched["with"]["name"]), (archived, "rust-repeat-tests"))
        run = next(step for step in steps if "plan-lines" in (step.get("run") or ""))
        self.assertEqual(run["env"]["ARCHIVED"], "${{ needs.count.outputs.rust-archived }}")
        script = run["run"]
        self.assertIn('target=(--archive-file "$RUNNER_TEMP/rust-repeat/$archive")', script)
        # An archive run takes no package flag: `-p` is only in the build branch.
        archive_branch = script[script.index('if [ "$ARCHIVED" = true ]'):script.index("else", script.index('if [ "$ARCHIVED" = true ]'))]
        self.assertNotIn("-p", archive_branch.replace("--archive-file", ""))


class WebLegShards(unittest.TestCase):
    """The web leg runs on `web-e2e.yml`'s shards with the count job's plan."""

    def test_the_web_leg_takes_the_planned_shards_parts_and_repeat(self):
        import yaml
        jobs = yaml.safe_load((ROOT / ".github/workflows/repeat.yml").read_text())["jobs"]
        web = jobs["web"]["with"]
        self.assertEqual(jobs["web"]["uses"], "./.github/workflows/web-e2e.yml")
        self.assertEqual(web["shards"], "${{ fromJSON(needs.count.outputs.web-shards) }}")
        self.assertEqual(web["repeat-parts"], "${{ fromJSON(needs.count.outputs.web-parts) }}")
        self.assertEqual(web["repeat"], "${{ needs.count.outputs.web-repeat }}")
        choose = next(step for step in jobs["count"]["steps"] if step.get("id") == "repeat")
        self.assertIn('--web-list "$WEB_LIST"', choose["run"])

    def test_the_lanes_completed_runs_are_every_shards_summed(self):
        import yaml
        # PyYAML reads the key `on` as true.
        lane = yaml.safe_load((ROOT / ".github/workflows/web-e2e.yml").read_text())
        self.assertEqual(lane[True]["workflow_call"]["outputs"]["completed"]["value"],
                         "${{ jobs.repeat-total.outputs.completed }}")
        total = lane["jobs"]["repeat-total"]
        self.assertEqual(total["needs"], "e2e")
        self.assertIn('total --shards "$SHARDS"', total["steps"][-1]["run"])
        kept = next(step for step in lane["jobs"]["e2e"]["steps"] if step.get("name") == "Keep this shard's completed repeat runs")
        self.assertEqual(kept["with"]["name"], "repeat-completed-${{ inputs.lane }}-${{ matrix.shard }}")
        fetched = next(step for step in total["steps"] if step.get("uses", "").startswith("actions/download-artifact"))
        self.assertEqual(fetched["with"]["pattern"], "repeat-completed-${{ inputs.lane }}-*")

