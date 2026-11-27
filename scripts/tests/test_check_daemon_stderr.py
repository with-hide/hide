"""Daemon code may not print with `eprintln!`; a command's, a pane's and an entry point's output may."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load():
    spec = importlib.util.spec_from_file_location("check_daemon_stderr", ROOT / "scripts/check-daemon-stderr.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PRINTS = 'fn f() { eprintln!("{}", 1); }\n'


class DaemonStderr(unittest.TestCase):
    def tree(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for name, text in files.items():
            path = root / "hided" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root

    def test_the_repository_holds_it(self):
        self.assertEqual(load().violations(), [])

    def test_a_daemon_module_that_prints_is_named_with_its_line(self):
        root = self.tree({"src/server.rs": "fn a() {}\n" + PRINTS, "src/lib.rs": "fn b() { eprint!(\"x\"); }\n"})
        self.assertEqual(load().violations(root), ["hided/src/lib.rs:1", "hided/src/server.rs:2"])

    def test_the_commands_the_panes_and_the_entry_points_print_to_their_terminal(self):
        root = self.tree({
            "src/cli.rs": PRINTS,
            "src/device_cli.rs": PRINTS,
            "src/agent_exec.rs": PRINTS,
            "src/pane_install.rs": PRINTS,
            "src/bin/hided.rs": PRINTS,
        })
        self.assertEqual(load().violations(root), [])

    def test_a_new_file_is_daemon_code_whatever_its_name_ends_in(self):
        root = self.tree({"src/server_cli.rs": PRINTS, "src/tools/device_cli.rs": PRINTS})
        self.assertEqual(
            load().violations(root), ["hided/src/server_cli.rs:1", "hided/src/tools/device_cli.rs:1"]
        )

    def test_an_alias_or_a_wrapper_of_the_macro_is_a_print(self):
        root = self.tree({
            "src/a.rs": "use std::eprintln as log;\nfn f() { log!(\"x\"); }\n",
            "src/b.rs": "macro_rules! say { ($m:expr) => { std::eprintln!($m) }; }\n",
        })
        self.assertEqual(load().violations(root), ["hided/src/a.rs:1", "hided/src/b.rs:1"])

    def test_a_comment_a_string_and_test_code_are_not_prints(self):
        root = self.tree({
            "src/server.rs": '// eprintln!("x")\nfn a() { let _ = "eprintln!(x)"; }\n'
            "#[cfg(test)]\nmod tests {\n    fn t() { eprintln!(\"x\"); }\n}\n",
            "src/probe.rs": PRINTS,
            "src/lib.rs": "#[cfg(test)]\nmod probe;\n",
            "tests/it/run.rs": PRINTS,
        })
        self.assertEqual(load().violations(root), [])


if __name__ == "__main__":
    unittest.main()
