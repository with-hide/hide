"""The core boundary check must find machine access in production code only."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_check(root):
    spec = importlib.util.spec_from_file_location(
        'check_core_touches_no_machine', ROOT / 'scripts/check-core-touches-no-machine.py'
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.ROOT = root
    module.CORE = root / 'herdr-core' / 'src'
    return module


class CoreTouchesNoMachine(unittest.TestCase):
    def tree(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for relative, text in files.items():
            path = root / 'herdr-core' / 'src' / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root, load_check(root)

    def test_production_machine_access_is_found(self):
        root, check = self.tree({'reader.rs': 'fn read() { let _ = std::fs::read("x"); }\n'
                                              'fn run() { std::process::Command::new("git"); }\n'})
        found = check.findings(root / 'herdr-core/src/reader.rs')
        self.assertEqual(len(found), 2, found)
        self.assertIn('reader.rs:1', found[0])
        self.assertIn('reader.rs:2', found[1])

    def test_a_brace_in_a_string_does_not_end_a_test_item_early(self):
        root, check = self.tree({'store.rs': 'fn keep() {}\n#[cfg(test)]\nmod tests {\n'
                                             '    fn t() { std::fs::write("p", b"{not json").unwrap(); }\n'
                                             '    fn u() { std::fs::read("q").unwrap(); }\n}\n'})
        self.assertEqual(check.findings(root / 'herdr-core/src/store.rs'), [])

    def test_a_quote_character_does_not_open_a_string(self):
        root, check = self.tree({'quote.rs': 'fn q(v: &mut String) { v.push(\'"\'); v.push(\'\\\\\'); }\n'
                                             '#[cfg(test)]\nmod tests {\n    fn t() { std::fs::read("q").unwrap(); }\n}\n'
                                             'fn after() { std::fs::read("x"); }\n'})
        found = check.findings(root / 'herdr-core/src/quote.rs')
        self.assertEqual(len(found), 1, found)
        self.assertIn('quote.rs:6', found[0])

    def test_the_own_pid_and_a_path_in_a_message_are_not_machine_access(self):
        root, check = self.tree({'name.rs': 'fn name() -> String { format!("{}", std::process::id()) }\n'
                                            'fn advice() -> &\'static str { "see std::fs" }\n'})
        self.assertEqual(check.findings(root / 'herdr-core/src/name.rs'), [])

    def test_http_clients_are_machine_access_even_through_an_import_alias(self):
        clients = ('ureq', 'reqwest', 'hyper', 'attohttpc', 'isahc', 'curl')
        root, check = self.tree({'network.rs': '\n'.join(
            f'use {client} as transport_{index};' for index, client in enumerate(clients))})
        found = check.findings(root / 'herdr-core/src/network.rs')
        self.assertEqual(len(found), len(clients), found)

    def test_a_module_file_declared_behind_cfg_test_is_test_code(self):
        root, check = self.tree({'lib.rs': '#[cfg(test)]\nmod fake;\n#[cfg(all(test, unix))]\nmod fixture;\nmod real;\n',
                                 'fake.rs': 'fn f() { std::fs::read("x"); }\n',
                                 'fixture.rs': 'fn f() { std::fs::read("x"); }\n',
                                 'real.rs': 'fn f() {}\n'})
        self.assertEqual(check.test_modules(),
                         {'herdr-core/src/fake.rs', 'herdr-core/src/fixture.rs'})

    def test_a_new_touch_in_an_excused_store_fails(self):
        root, check = self.tree({'store.rs': 'fn save() { std::fs::write("s", b"x"); }\n'})
        check.STORES = {'herdr-core/src/store.rs': (1, 'the core state file')}
        check.LATER_LAYERS = {}
        check.FIXTURES = {}
        self.assertEqual(check.main(), 0)
        (root / 'herdr-core/src/store.rs').write_text(
            'fn save() { std::fs::write("s", b"x"); }\n'
            'fn peek() -> bool { std::path::Path::new("/etc").try_exists().is_ok() }\n')
        self.assertEqual(check.main(), 1)

    def test_the_repository_passes(self):
        check = load_check(ROOT)
        self.assertEqual(check.main(), 0)


if __name__ == '__main__':
    unittest.main()
