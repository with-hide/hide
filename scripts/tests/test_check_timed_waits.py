"""The timed-wait check refuses a new `timeout:` in an e2e file and a new sleep in a Rust test,
accepts the annotated form, and keeps a baseline that an unrelated edit cannot fail."""
import contextlib
import importlib.util
import io
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SPEC = 'await expect(row).toBeVisible({ timeout: 20_000 });\n'
SLEEP = 'fn t() { std::thread::sleep(std::time::Duration::from_millis(50)); }\n'


def load(root):
    spec = importlib.util.spec_from_file_location("check_timed_waits", ROOT / "scripts/check-timed-waits.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.ROOT = root
    module.BASELINE = root / "scripts" / "timed-waits-baseline.json"
    return module


class TimedWaits(unittest.TestCase):
    def tree(self, files, baseline=None):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        files = {"scripts/timed-waits-baseline.json": json.dumps(baseline or {}), **files}
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root, load(root)

    def run_check(self, check, *argv):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = check.main(list(argv))
        return code, err.getvalue()

    def rust(self, body, name="hide-x/tests/it/wait.rs"):
        return {"hide-x/Cargo.toml": "[package]\n", name: body}

    def test_a_new_timeout_in_a_spec_fails_and_the_message_says_what_to_do(self):
        _, check = self.tree({"web/e2e/new.spec.ts": SPEC})
        code, err = self.run_check(check)
        self.assertEqual(code, 1)
        self.assertIn("web/e2e/new.spec.ts: 1 unannotated", err)
        self.assertIn("lines 1", err)
        self.assertIn("Wait for state, not time", err)
        self.assertIn("wait-ok:", err)

    def test_a_new_timeout_in_an_e2e_helper_of_the_desktop_suite_fails(self):
        _, check = self.tree({"desktop/e2e/fixture.ts": "const x = { timeout: 5000 };\n"})
        self.assertEqual(self.run_check(check)[0], 1)

    def test_a_new_sleep_in_a_rust_integration_test_fails(self):
        _, check = self.tree(self.rust(SLEEP))
        code, err = self.run_check(check)
        self.assertEqual(code, 1)
        self.assertIn("hide-x/tests/it/wait.rs: 1 unannotated", err)

    def test_every_qualified_form_of_sleep_in_a_test_is_found(self):
        body = (
            "fn a() { thread::sleep(d); }\n"
            "async fn b() { tokio::time::sleep(d).await; }\n"
            "async fn c() { time::sleep(d).await; }\n"
        )
        root, check = self.tree(self.rust(body))
        self.assertEqual(check.measure()["rust_sleeps"]["hide-x/tests/it/wait.rs"], [1, 2, 3])

    def test_a_sleep_imported_by_name_is_counted_at_its_calls_and_not_at_the_import(self):
        body = (
            "use tokio::time::{Instant, sleep};\n"
            "async fn a() { sleep(d).await; }\n"
            "async fn b() { let x = 1; sleep(d).await; }\n"
        )
        root, check = self.tree(self.rust(body))
        self.assertEqual(check.measure()["rust_sleeps"]["hide-x/tests/it/wait.rs"], [2, 3])

    def test_the_other_ways_to_import_a_sleep_are_counted_at_their_calls(self):
        cases = {
            "use std::thread::sleep;\nfn a() { sleep(d); }\n": [2],
            "use std::{thread::sleep, time::Duration};\nfn a() { sleep(d); }\n": [2],
            "use tokio::time::sleep as nap;\nasync fn a() { nap(d).await; sleep(d); }\n": [2],
            "use tokio::time::{self, sleep as nap};\nasync fn a() { nap(d).await; }\n": [2],
            "use std::thread::*;\nfn a() { sleep(d); }\n": [2],
            "use tokio::{time::{sleep}};\nasync fn a() { sleep(d).await; }\n": [2],
            "use std::thread::{\n    sleep,\n};\nfn a() { sleep(d); }\n": [4],
        }
        for body, lines in cases.items():
            with self.subTest(body=body):
                _, check = self.tree(self.rust(body))
                self.assertEqual(check.measure()["rust_sleeps"].get("hide-x/tests/it/wait.rs"), lines)

    def test_a_function_named_sleep_that_is_not_imported_is_not_a_sleep(self):
        body = "fn sleep(runtime: &mut Runtime) {}\nfn t() { sleep(&mut r); }\n"
        _, check = self.tree(self.rust(body))
        self.assertEqual(check.measure()["rust_sleeps"], {})

    def test_an_imported_sleep_called_in_a_cfg_test_module_counts_and_in_production_does_not(self):
        body = (
            "use std::thread::sleep;\n"
            "fn production() { sleep(d); }\n"
            "#[cfg(test)]\nmod tests {\n    use super::*;\n    fn t() { sleep(d); }\n}\n"
        )
        _, check = self.tree({"hide-x/Cargo.toml": "[package]\n", "hide-x/src/lib.rs": body})
        self.assertEqual(check.measure()["rust_sleeps"], {"hide-x/src/lib.rs": [6]})

    def test_an_annotated_call_of_an_imported_sleep_is_accepted(self):
        body = "use std::thread::sleep;\nfn t() { sleep(d); } // wait-ok: the next poll of the pid file\n"
        _, check = self.tree(self.rust(body))
        self.assertEqual(self.run_check(check)[0], 0)

    def test_a_sleep_in_a_cfg_test_module_of_a_source_file_fails_and_in_production_does_not(self):
        body = (
            "fn production() { std::thread::sleep(d); }\n"
            "#[cfg(test)]\nmod tests {\n    fn t() { std::thread::sleep(d); }\n}\n"
        )
        root, check = self.tree({"hide-x/Cargo.toml": "[package]\n", "hide-x/src/lib.rs": body})
        self.assertEqual(check.measure()["rust_sleeps"], {"hide-x/src/lib.rs": [4]})

    def test_a_module_the_parent_declares_behind_cfg_test_is_test_code(self):
        files = {
            "hide-x/Cargo.toml": "[package]\n",
            "hide-x/src/lib.rs": "#[cfg(test)]\nmod support;\n",
            "hide-x/src/support.rs": SLEEP,
        }
        _, check = self.tree(files)
        self.assertEqual(check.measure()["rust_sleeps"], {"hide-x/src/support.rs": [1]})

    def test_the_annotated_form_is_accepted_with_a_named_state(self):
        files = {
            "web/e2e/ok.spec.ts": 'await go({ timeout: 90_000 }); // wait-ok: a cold holder start on Windows\n',
            **self.rust('fn t() { std::thread::sleep(d); } // wait-ok: the next poll of the pid file\n'),
        }
        _, check = self.tree(files)
        self.assertEqual(self.run_check(check)[0], 0)

    def test_an_annotation_without_a_state_does_not_count(self):
        _, check = self.tree({"web/e2e/empty.spec.ts": "await go({ timeout: 9 }); // wait-ok:\n"})
        self.assertEqual(self.run_check(check)[0], 1)

    def test_an_annotation_before_the_wait_on_the_same_line_does_not_count(self):
        _, check = self.tree({"web/e2e/early.spec.ts": "// wait-ok: not a trailing comment\nawait go({ timeout: 9 });\n"})
        self.assertEqual(self.run_check(check)[0], 1)

    def test_a_timeout_named_in_a_comment_or_a_string_is_not_a_wait(self):
        text = '// { timeout: 1 }\nconst s = "timeout: 2";\nconst t = `timeout: ${x}`;\n/* timeout: 3 */\n'
        _, check = self.tree({"web/e2e/words.spec.ts": text})
        self.assertEqual(self.run_check(check)[0], 0)

    def test_set_timeout_and_a_timeout_variable_are_not_a_timeout_override(self):
        _, check = self.tree({"web/e2e/other.spec.ts": "test.setTimeout(9);\nconst timeout = 1;\n"})
        self.assertEqual(self.run_check(check)[0], 0)

    def test_the_baseline_allows_what_a_file_has_whatever_line_it_is_on(self):
        baseline = {"e2e_timeouts": {"web/e2e/old.spec.ts": 2}}
        before = "x\n" + SPEC + SPEC
        after = "x\ny\nz\n" + SPEC + "more();\n" + SPEC
        for text in (before, after):
            _, check = self.tree({"web/e2e/old.spec.ts": text}, baseline)
            self.assertEqual(self.run_check(check)[0], 0)

    def test_one_more_than_the_baseline_fails_and_one_fewer_passes(self):
        baseline = {"e2e_timeouts": {"web/e2e/old.spec.ts": 2}}
        _, more = self.tree({"web/e2e/old.spec.ts": SPEC * 3}, baseline)
        self.assertEqual(self.run_check(more)[0], 1)
        _, fewer = self.tree({"web/e2e/old.spec.ts": SPEC}, baseline)
        code, _ = self.run_check(fewer)
        self.assertEqual(code, 0)

    def test_fewer_than_the_baseline_passes_without_touching_it(self):
        baseline = {"e2e_timeouts": {"web/e2e/old.spec.ts": 5}, "rust_sleeps": {"hide-x/tests/it/wait.rs": 3}}
        files = {"web/e2e/old.spec.ts": SPEC * 2, **self.rust(SLEEP)}
        _, check = self.tree(files, baseline)
        before = check.BASELINE.read_text()
        self.assertEqual(self.run_check(check)[0], 0)
        self.assertEqual(check.BASELINE.read_text(), before)

    def test_removing_a_file_or_every_wait_in_it_passes(self):
        baseline = {"e2e_timeouts": {"web/e2e/gone.spec.ts": 4, "web/e2e/clean.spec.ts": 1}}
        _, check = self.tree({"web/e2e/clean.spec.ts": "await go();\n"}, baseline)
        self.assertEqual(self.run_check(check)[0], 0)

    def test_a_wait_moved_to_a_file_the_baseline_does_not_list_fails(self):
        baseline = {"e2e_timeouts": {"web/e2e/old.spec.ts": 1}}
        _, check = self.tree({"web/e2e/renamed.spec.ts": SPEC}, baseline)
        self.assertEqual(self.run_check(check)[0], 1)

    def test_lower_brings_a_count_down_and_never_up(self):
        baseline = {"e2e_timeouts": {"web/e2e/a.spec.ts": 3, "web/e2e/b.spec.ts": 1, "web/e2e/gone.spec.ts": 2}}
        root, check = self.tree({"web/e2e/a.spec.ts": SPEC, "web/e2e/b.spec.ts": SPEC * 5}, baseline)
        self.assertEqual(self.run_check(check, "--lower")[0], 0)
        written = json.loads(check.BASELINE.read_text())
        self.assertEqual(written["e2e_timeouts"], {"web/e2e/a.spec.ts": 1, "web/e2e/b.spec.ts": 1})
        self.assertEqual(written["rust_sleeps"], {})

    def test_vendored_and_build_folders_are_not_scanned(self):
        files = {
            "vendor/portable-pty/Cargo.toml": "[package]\n",
            "vendor/portable-pty/tests/it/a.rs": SLEEP,
            "web/e2e/node_modules/pkg/index.ts": SPEC,
        }
        _, check = self.tree(files)
        self.assertEqual(self.run_check(check)[0], 0)

    def repo(self, files, baseline):
        """A git repository whose one commit holds `files` and `baseline`."""
        root, check = self.tree(files, baseline)
        self.git(root, "init", "-q")
        self.git(root, "add", "-A")
        self.git(root, "commit", "-q", "-m", "base")
        return root, check

    def git(self, root, *args):
        subprocess.run(
            ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
            check=True, capture_output=True,
        )

    def edit_baseline(self, root, check, baseline):
        check.BASELINE.write_text(json.dumps(baseline))

    def test_a_baseline_number_raised_by_hand_fails_against_the_base(self):
        old = {"e2e_timeouts": {"web/e2e/old.spec.ts": 1}}
        root, check = self.repo({"web/e2e/old.spec.ts": SPEC}, old)
        (root / "web/e2e/old.spec.ts").write_text(SPEC * 2)
        self.edit_baseline(root, check, {"e2e_timeouts": {"web/e2e/old.spec.ts": 2}})
        self.assertEqual(self.run_check(check)[0], 0)
        code, err = self.run_check(check, "--base", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("web/e2e/old.spec.ts is 2, up from 1", err)

    def test_a_file_new_to_the_baseline_starts_at_zero_against_the_base(self):
        root, check = self.repo({"web/e2e/old.spec.ts": SPEC}, {"e2e_timeouts": {"web/e2e/old.spec.ts": 1}})
        (root / "web/e2e/new.spec.ts").write_text(SPEC)
        self.edit_baseline(root, check, {"e2e_timeouts": {"web/e2e/old.spec.ts": 1, "web/e2e/new.spec.ts": 1}})
        code, err = self.run_check(check, "--base", "HEAD")
        self.assertEqual(code, 1)
        self.assertIn("web/e2e/new.spec.ts is 1, up from 0", err)

    def test_the_same_or_a_lower_baseline_passes_against_the_base(self):
        old = {"e2e_timeouts": {"web/e2e/a.spec.ts": 2, "web/e2e/b.spec.ts": 3}}
        root, check = self.repo({"web/e2e/a.spec.ts": SPEC * 2, "web/e2e/b.spec.ts": SPEC * 3}, old)
        self.assertEqual(self.run_check(check, "--base", "HEAD")[0], 0)
        (root / "web/e2e/b.spec.ts").write_text(SPEC)
        self.edit_baseline(root, check, {"e2e_timeouts": {"web/e2e/a.spec.ts": 2, "web/e2e/b.spec.ts": 1}})
        self.assertEqual(self.run_check(check, "--base", "HEAD")[0], 0)

    def test_a_renamed_file_carries_its_old_baseline_number_against_the_base(self):
        root, check = self.repo({"web/e2e/old.spec.ts": SPEC * 2 + "more();\n"}, {"e2e_timeouts": {"web/e2e/old.spec.ts": 2}})
        self.git(root, "mv", "web/e2e/old.spec.ts", "web/e2e/renamed.spec.ts")
        self.edit_baseline(root, check, {"e2e_timeouts": {"web/e2e/renamed.spec.ts": 2}})
        self.assertEqual(self.run_check(check, "--base", "HEAD")[0], 0)
        self.edit_baseline(root, check, {"e2e_timeouts": {"web/e2e/renamed.spec.ts": 3}})
        (root / "web/e2e/renamed.spec.ts").write_text(SPEC * 3)
        self.assertEqual(self.run_check(check, "--base", "HEAD")[0], 1)

    def test_a_base_without_a_baseline_is_nothing_to_compare_and_an_unknown_base_is_refused(self):
        root, check = self.repo({"README": "x\n"}, None)
        self.git(root, "rm", "-q", "--cached", "scripts/timed-waits-baseline.json")
        self.git(root, "commit", "-q", "-m", "drop it")
        self.assertEqual(self.run_check(check, "--base", "HEAD")[0], 0)
        with self.assertRaises(SystemExit):
            self.run_check(check, "--base", "no-such-ref")

    def test_the_checked_in_baseline_passes_on_this_tree(self):
        check = load(ROOT)
        self.assertEqual(self.run_check(check)[0], 0)


if __name__ == "__main__":
    unittest.main()
