"""How `scripts/e2e-shard.py` deals the web and desktop end-to-end tests out to shards."""
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
SCRIPT = ROOT / "scripts/e2e-shard.py"
SPEC = importlib.util.spec_from_file_location("e2e_shard", SCRIPT)
shard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(shard)

LIST = """Listing tests:
  a.spec.ts:10:1 › one
  a.spec.ts:40:1 › two
  b.spec.ts:7:1 › describe › nested
  c.spec.ts:3:1 › four
  c.spec.ts:30:1 › five
Total: 5 tests in 3 files
"""
TESTS = ["a.spec.ts › one", "a.spec.ts › two", "b.spec.ts › describe › nested", "c.spec.ts › four", "c.spec.ts › five"]


def run(*args, env=None):
    """The script as a process, its stdout as raw bytes: that is what a shard file holds."""
    return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, env=env)


class Listing(unittest.TestCase):
    def test_reads_the_tests_playwright_lists_in_its_order(self):
        self.assertEqual(shard.listed_tests(LIST), TESTS)

    def test_a_list_without_a_test_or_with_a_repeated_one_is_refused(self):
        with self.assertRaises(shard.Refused):
            shard.listed_tests("Listing tests:\nTotal: 0 tests in 0 files\n")
        with self.assertRaises(shard.Refused):
            shard.listed_tests("  a.spec.ts:1:1 › x\n  a.spec.ts:9:1 › x\n")
        self.assertEqual(shard.listed_tests("Total: 0 tests in 0 files\n", allow_none=True), [])

    def test_a_named_project_does_not_change_how_a_test_is_named(self):
        # The desktop config's `background` and `needs-focus` projects hold
        # each test once, and a shard file without the project matches it.
        listing = "  [background] › a.spec.ts:10:1 › one\n  [needs-focus] › b.spec.ts:7:1 › focus @needs-focus\n"
        self.assertEqual(shard.listed_tests(listing), ["a.spec.ts › one", "b.spec.ts › focus @needs-focus"])
        with self.assertRaises(shard.Refused):
            shard.listed_tests("  [background] › a.spec.ts:1:1 › x\n  [needs-focus] › a.spec.ts:1:1 › x\n")


class Splitting(unittest.TestCase):
    def test_fills_the_lightest_shard_with_the_longest_test_first(self):
        table = {"a.spec.ts › one": 50.0, "a.spec.ts › two": 30.0, "b.spec.ts › describe › nested": 25.0,
                 "c.spec.ts › four": 20.0, "c.spec.ts › five": 5.0}
        assigned, load = shard.split(TESTS, table, 2)
        self.assertEqual(assigned, {"a.spec.ts › one": 0, "a.spec.ts › two": 1, "b.spec.ts › describe › nested": 1,
                                    "c.spec.ts › four": 0, "c.spec.ts › five": 1})
        self.assertEqual(load, [70.0, 60.0])

    def test_every_test_lands_in_exactly_one_shard_whatever_the_table_knows(self):
        for table in ({}, {"a.spec.ts › one": 9.0}, {"gone.spec.ts › x": 1.0}):
            for total in (1, 2, 4):
                with self.subTest(table=table, total=total):
                    assigned, load = shard.split(TESTS, table, total)
                    self.assertEqual(sorted(assigned), sorted(TESTS))
                    self.assertTrue(all(0 <= index < total for index in assigned.values()))
                    self.assertEqual(len(load), total)

    def test_a_test_the_table_lacks_weighs_the_tables_median(self):
        assigned, load = shard.split(["x", "y", "z"], {"x": 10.0, "y": 30.0, "k": 20.0}, 2)
        # z is new and weighs 20; y (30) opens a shard, x (10) and z (20) fill the other.
        self.assertEqual(sorted(load), [30.0, 30.0])

    def test_the_same_input_gives_the_same_shards(self):
        table = {test: 5.0 for test in TESTS}
        self.assertEqual(shard.split(TESTS, table, 3), shard.split(list(TESTS), dict(table), 3))

    def test_a_plan_reports_tests_the_table_lacks_and_a_heavy_shard_but_does_not_fail(self):
        self.assertEqual(shard.drift(["a", "b"], {"a": 5.0, "b": 5.0}, [5.0, 5.0], "t.json"), [])
        notes = shard.drift(["a", "b", "c"], {"a": 5.0, "b": 5.0}, [5.0, 5.0], "t.json")
        self.assertEqual(len(notes), 1)
        self.assertIn("1 of 3 listed tests are not in t.json", notes[0])
        notes = shard.drift(["a", "b"], {"a": 30.0, "b": 5.0}, [30.0, 5.0], "t.json")
        self.assertIn("the heaviest shard plans 30 s against a mean of 18 s; refresh t.json", notes[0])

    def test_a_plan_past_the_lanes_budget_asks_for_another_shard(self):
        tests, table = ["a", "b"], {"a": 20.0, "b": 20.0}
        self.assertEqual(shard.drift(tests, table, [20.0, 20.0], "t.json", budget=20), [])
        notes = shard.drift(tests, table, [20.0, 21.0], "t.json", budget=20)
        self.assertEqual(notes, ['the heaviest of 2 shards plans 21 s, over the 20 s this lane sizes a shard for; '
                                 'add a shard (docs/TESTING.md, "Balancing the e2e shards")'])

    # The Linux lane is sized by when it ends too, beside the Rust lane: the two
    # longest Linux lines of a run, so neither alone sets the run's length.
    # The Rust lane is held to 13 minutes of a run's start, and the plan job
    # takes 18 s of that, which leaves this lane 762 s. Its own numbers,
    # medians of twenty successful main runs (2026-11-20..21, 120 shard
    # jobs): `build hided` 194 s, a shard's setup before its step 80 s, and
    # the step's listing and report around its tests 37 s (the step's 692 s
    # less the 655 s it planned).
    LINUX_BUILD_SECONDS = 194
    LINUX_SHARD_SETUP_SECONDS = 80
    LINUX_STEP_OVERHEAD_SECONDS = 37
    LINUX_LANE_TARGET_SECONDS = 762

    def test_the_web_lane_uses_the_fewest_shards_that_end_beside_the_rust_lane(self):
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        lane = re.split(r"\n(?=  [a-z0-9-]+:\n)",
                        workflow.split("\n  web-e2e:\n", 1)[1], maxsplit=1)[0]
        count = int(re.search(r"\n      shards: (\d+)", lane).group(1))
        table = json.loads((ROOT / "web/e2e/shard-durations.json").read_text(encoding="utf-8"))

        def finish(seconds):
            return (self.LINUX_BUILD_SECONDS + self.LINUX_SHARD_SETUP_SECONDS
                    + self.LINUX_STEP_OVERHEAD_SECONDS + seconds)

        _, load = shard.split(sorted(table), table, count)
        # The shard's own step cap still bounds its test work (60 % of 1200 s).
        self.assertLessEqual(max(load), 720)
        self.assertGreater(count, 1)
        self.assertLessEqual(finish(max(load)), self.LINUX_LANE_TARGET_SECONDS)
        # One fewer would end past the target, so the count is the fewest that does.
        _, smaller = shard.split(sorted(table), table, count - 1)
        self.assertGreater(finish(max(smaller)), self.LINUX_LANE_TARGET_SECONDS)

    # The Windows lane is sized by when a shard finishes, not by its test work
    # alone: the daemon and shell builds precede the shards (`needs:`) and each
    # shard then pays its own setup before a test runs, so the fixed cost is
    # about 402 seconds whatever the count, and only another shard shortens the
    # wait. The lane's own numbers, medians of 115 successful `web e2e windows`
    # jobs measured on 2026-11-14: `build hided` 288 s, the shard's setup 114 s.
    # The target is the 720 s the shard's own step is sized to, which the test
    # work alone already met at two shards, so this holds what that does not:
    # that the lane, fixed cost and tests together, clears it at the fewest
    # shards the recording allows.
    WINDOWS_BUILD_SECONDS = 288
    WINDOWS_SHARD_SETUP_SECONDS = 114
    WINDOWS_LANE_TARGET_SECONDS = 720

    def test_the_windows_lane_uses_the_fewest_shards_that_finish_within_its_target(self):
        workflow = (ROOT / ".github/workflows/pr.yml").read_text()
        lane = re.split(r"\n(?=  [a-z0-9-]+:\n)",
                        workflow.split("\n  windows-e2e:\n", 1)[1], maxsplit=1)[0]
        count = int(re.search(r"\n      shards: (\d+)", lane).group(1))
        table_name = re.search(r"\n      table: (\S+)", lane).group(1)
        self.assertEqual(table_name, "web/e2e/shard-durations-windows.json")
        table = json.loads((ROOT / table_name).read_text(encoding="utf-8"))

        def finish(seconds):
            return self.WINDOWS_BUILD_SECONDS + self.WINDOWS_SHARD_SETUP_SECONDS + seconds

        _, load = shard.split(sorted(table), table, count)
        # The shard's own step cap still bounds its test work.
        self.assertLessEqual(max(load), 720)
        self.assertGreater(count, 1)
        self.assertLessEqual(finish(max(load)), self.WINDOWS_LANE_TARGET_SECONDS)
        # One fewer would miss the target, so the count is the fewest that does.
        _, smaller = shard.split(sorted(table), table, count - 1)
        self.assertGreater(finish(max(smaller)), self.WINDOWS_LANE_TARGET_SECONDS)

    def test_the_committed_tables_name_only_whole_tests_of_existing_specs(self):
        for suite, name in (("web/e2e", "shard-durations.json"),
                            ("web/e2e", "shard-durations-windows.json"),
                            ("desktop/e2e", "shard-durations.json")):
            table = json.loads((ROOT / suite / name).read_text(encoding="utf-8"))
            self.assertGreater(len(table), 100 if name == "shard-durations.json" else 50)
            for key, seconds in table.items():
                with self.subTest(suite=f"{suite}/{name}", key=key):
                    match = re.fullmatch(r"([\w.-]+\.spec\.ts) › (.+)", key)
                    self.assertTrue(match, "not `file › title`")
                    self.assertTrue((ROOT / suite / match[1]).is_file(), "no such spec")
                    self.assertNotRegex(match[2], r"\(retry #\d+\)$", "a retry mark in the title")
                    self.assertGreaterEqual(seconds, 0)


class Command(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.table = self.path / "table.json"
        self.table.write_text(json.dumps({"a.spec.ts › one": 50, "a.spec.ts › two": 30, "c.spec.ts › four": 20}), encoding="utf-8")

    def file(self, name, text):
        path = self.path / name
        path.write_text(text, encoding="utf-8")
        return str(path)

    def split(self, current, total, listing=LIST, env=None):
        return run("split", "--shard", f"{current}/{total}", "--table", str(self.table), self.file("list.txt", listing), env=env)

    def test_the_shards_together_run_every_listed_test_once(self):
        results = [self.split(current, 2) for current in (1, 2)]
        for result in results:
            self.assertEqual(result.returncode, 0, result.stderr)
        lines = [line for result in results for line in result.stdout.decode("utf-8").splitlines()]
        self.assertEqual(sorted(lines), sorted(TESTS))
        self.assertIn("shard 1/2", results[0].stderr.decode())

    def test_the_shard_file_is_utf8_whatever_encoding_the_runner_gives_stdout(self):
        # A Windows runner's stdout is cp1252, which writes `›` as the byte
        # 0x9b; Playwright reads the file as UTF-8 and would match no test.
        env = {**os.environ, "PYTHONIOENCODING": "cp1252", "PYTHONUTF8": "0"}
        result = self.split(1, 1, env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "".join(f"{test}\n" for test in TESTS).encode("utf-8"))
        self.assertIn("›".encode("utf-8"), result.stdout)
        self.assertNotIn(b"\r", result.stdout)

    def test_fewer_tests_than_shards_leaves_the_extra_shards_an_empty_file(self):
        one = "  a.spec.ts:10:1 › one\n"
        first, second = self.split(1, 4, listing=one), self.split(2, 4, listing=one)
        self.assertEqual((first.returncode, first.stdout), (0, b"a.spec.ts \xe2\x80\xba one\n"))
        self.assertEqual((second.returncode, second.stdout), (0, b""))

    def test_a_problem_with_an_input_is_one_line_not_a_traceback(self):
        for result in (
            self.split(1, 2, listing="Listing tests:\nTotal: 0 tests in 0 files\n"),
            self.split(1, 2, listing="  a.spec.ts:1:1 › x\n  a.spec.ts:9:1 › x\n"),
            run("split", "--shard", "1/2", "--table", str(self.path / "gone.json"), self.file("l.txt", LIST)),
            run("split", "--shard", "1/2", "--table", self.file("bad.json", "{"), self.file("l.txt", LIST)),
            run("split", "--shard", "1/2", "--table", self.file("bad2.json", '{"a.spec.ts › one": "slow"}'), self.file("l.txt", LIST)),
        ):
            with self.subTest(stderr=result.stderr):
                self.assertEqual(result.returncode, 1)
                self.assertNotIn(b"Traceback", result.stderr)
                self.assertTrue(result.stderr.startswith(b"e2e-shard: "))
                self.assertEqual(len(result.stderr.splitlines()), 1)

    def test_a_shard_outside_the_total_is_refused(self):
        for value in ("0/4", "5/4", "4", "a/b"):
            with self.subTest(value=value):
                self.assertNotEqual(run("split", "--shard", value, "--table", str(self.table), "unused").returncode, 0)

    def test_a_plan_names_its_table_because_each_suite_records_its_own(self):
        result = run("split", "--shard", "1/2", self.file("list.txt", LIST))
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"--table", result.stderr)

    def test_only_the_first_shard_warns_of_a_plan_past_the_budget(self):
        first = run("split", "--shard", "1/2", "--table", str(self.table), "--budget", "60", self.file("list.txt", LIST))
        second = run("split", "--shard", "2/2", "--table", str(self.table), "--budget", "60", self.file("list.txt", LIST))
        self.assertEqual((first.returncode, second.returncode), (0, 0))
        # The table's 50, 30 and 20, and its median 30 for the two it lacks: 80 s a shard.
        self.assertIn("::warning::the heaviest of 2 shards plans 80 s, over the 60 s", first.stderr.decode())
        self.assertNotIn("::warning::", second.stderr.decode())

    def check(self, current, total, listed):
        planned = self.split(current, total)
        self.assertEqual(planned.returncode, 0, planned.stderr)
        return run("check", "--shard", f"{current}/{total}", "--table", str(self.table),
                   self.file("list.txt", LIST), self.file("shard-list.txt", listed))

    def test_check_passes_when_playwright_lists_the_planned_tests(self):
        planned = self.split(1, 2).stdout.decode("utf-8").splitlines()
        listed = "Listing tests:\n" + "".join(f"  {test.replace(' › ', ':1:1 › ', 1)}\n" for test in planned) + "Total\n"
        result = self.check(1, 2, listed)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_check_fails_when_the_shard_file_matches_nothing_or_the_wrong_tests(self):
        for listed, words in (
            ("Listing tests:\nTotal: 0 tests in 0 files\n", "would run 0 tests, not the"),
            ("  a.spec.ts:10:1 › one\n  c.spec.ts:30:1 › five\n  b.spec.ts:7:1 › describe › nested\n  c.spec.ts:3:1 › four\n", "unplanned tests are selected"),
        ):
            with self.subTest(words=words):
                result = self.check(1, 2, listed)
                self.assertEqual(result.returncode, 1)
                self.assertIn(words, result.stderr.decode())
                self.assertNotIn(b"Traceback", result.stderr)

    def test_check_passes_for_a_shard_dealt_no_test(self):
        one = "  a.spec.ts:10:1 › one\n"
        result = run("check", "--shard", "2/4", "--table", str(self.table), self.file("list.txt", one), self.file("shard-list.txt", "Total: 0 tests in 0 files\n"))
        self.assertEqual(result.returncode, 0, result.stderr)


class Durations(unittest.TestCase):
    LOG = (
        "2026-10-06T00:35:07.4913637Z   ✓   1 e2e/s3.spec.ts:262:1 › editing a document @platform (11.9s)\n"
        "2026-10-06T00:35:09.0000000Z   ✓   2 e2e/a.spec.ts:10:1 › one (850ms)\n"
        "2026-10-06T00:35:09.0000000Z   ✘   3 e2e/a.spec.ts:40:1 › two (1.2s)\n"
        "2026-10-06T00:35:09.0000000Z   ✓   4 e2e/b.spec.ts:7:1 › describe › nested (1.5m)\n"
        "2026-10-06T00:35:09.0000000Z unrelated line (3.0s)\n"
    )
    LISTED = ["a.spec.ts › one", "b.spec.ts › describe › nested", "s3.spec.ts › editing a document", "z.spec.ts › never ran"]

    def test_reads_passing_lines_without_their_tags_and_in_seconds(self):
        self.assertEqual(
            shard.durations([self.LOG], self.LISTED),
            {"a.spec.ts › one": 0.8, "b.spec.ts › describe › nested": 90.0, "s3.spec.ts › editing a document": 11.9},
        )

    def test_a_passing_retry_is_the_test_not_a_test_named_after_the_retry(self):
        log = (
            "  ✓  14 e2e/a.spec.ts:10:1 › one (retry #1) (4.2s)\n"
            "  ✓  15 e2e/a.spec.ts:40:1 › two @platform (retry #2) (6.0s)\n"
        )
        self.assertEqual(shard.durations([log], ["a.spec.ts › one", "a.spec.ts › two"]), {"a.spec.ts › one": 4.2, "a.spec.ts › two": 6.0})

    def test_a_windows_runners_ok_mark_counts_as_a_pass(self):
        log = "  ok   1 e2e/a.spec.ts:10:1 › one (2.0s)\n  ok   2 e2e/a.spec.ts:40:1 › look ok (3.0s)\n"
        self.assertEqual(shard.durations([log], ["a.spec.ts › one", "a.spec.ts › look ok"]), {"a.spec.ts › one": 2.0, "a.spec.ts › look ok": 3.0})

    def test_a_windows_runners_backslash_path_names_the_listed_test(self):
        # A Windows runner prints the directory and separator as Windows spells
        # them: `ok 1 e2e\a.spec.ts:10:1 › one (2.0s)`. A table built from those
        # logs must key the test as the list does, or every entry is unknown and
        # the lane plans against the median instead of the recording.
        log = "  ok   1 e2e\\a.spec.ts:10:1 › one (2.0s)\n  ok   2 e2e\\a.spec.ts:40:1 › look ok (3.0s)\n"
        self.assertEqual(shard.durations([log], ["a.spec.ts › one", "a.spec.ts › look ok"]),
                         {"a.spec.ts › one": 2.0, "a.spec.ts › look ok": 3.0})

    def test_keeps_only_the_tests_the_list_names(self):
        log = "  ✓   1 e2e/a.spec.ts:10:1 › one (4.0s)\n  ✓   2 e2e/a.spec.ts:40:1 › renamed away (9.0s)\n"
        self.assertEqual(shard.durations([log], ["a.spec.ts › one"]), {"a.spec.ts › one": 4.0})

    def test_takes_the_median_across_logs(self):
        logs = [f"  ✓   1 e2e/a.spec.ts:10:1 › one ({seconds}s)\n" for seconds in (4.0, 100.0, 6.0)]
        self.assertEqual(shard.durations(logs, ["a.spec.ts › one"]), {"a.spec.ts › one": 6.0})

    def test_a_listed_title_matches_the_title_a_run_prints(self):
        # `--list` omits the tags a run prints, so both name a test the same way.
        listed = shard.listed_tests("  s3.spec.ts:262:1 › editing a document\n")
        self.assertEqual(list(shard.durations([self.LOG], listed)), listed)

    def test_a_title_that_ends_in_a_tag_of_its_own_keeps_it(self):
        log = "  ✓  99 [needs-focus] › e2e/view.spec.ts:213:5 › gives it back @needs-focus (retry #1) (7.5s)\n"
        self.assertEqual(shard.durations([log], ["view.spec.ts › gives it back @needs-focus"]),
                         {"view.spec.ts › gives it back @needs-focus": 7.5})

    def test_a_desktop_runs_project_and_a_skipped_test_are_read(self):
        # A desktop run names the project; a test it only skipped (one that
        # needs a packaged app) costs the lane nothing, while one that passed
        # anywhere keeps its passing median.
        log = (
            "2026-10-17T12:30:27.7652520Z   ✓    1 [background] › e2e/add.spec.ts:37:5 › picks a folder @platform (21.8s)\n"
            "2026-10-17T12:37:19.8049500Z   -   42 [background] › e2e/kit.spec.ts:101:5 › a device's Home\n"
            "2026-10-17T12:37:19.8049500Z   -   43 [background] › e2e/add.spec.ts:37:5 › picks a folder @platform\n"
        )
        self.assertEqual(
            shard.durations([log], ["add.spec.ts › picks a folder", "kit.spec.ts › a device's Home", "x.spec.ts › never"]),
            {"add.spec.ts › picks a folder": 21.8, "kit.spec.ts › a device's Home": 0.0},
        )

    def test_the_command_writes_utf8_and_names_the_tests_it_found_no_line_for(self):
        with tempfile.TemporaryDirectory() as directory:
            listing = Path(directory) / "list.txt"
            listing.write_text("  a.spec.ts:10:1 › one\n  a.spec.ts:40:1 › two\n", encoding="utf-8")
            log = Path(directory) / "job.log"
            log.write_text("  ✓   1 e2e/a.spec.ts:10:1 › one (4.0s)\n", encoding="utf-8")
            env = {**os.environ, "PYTHONIOENCODING": "cp1252", "PYTHONUTF8": "0"}
            result = run("durations", "--list", str(listing), str(log), env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout.decode("utf-8")), {"a.spec.ts › one": 4.0})
        self.assertIn("1 of 2 listed tests have no passing or skipped line", result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
