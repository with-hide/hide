"""The code host seam check finds a host's CLI, address or key outside its unit."""
import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_check(root):
    spec = importlib.util.spec_from_file_location(
        'check_code_host_seam', ROOT / 'scripts/check-code-host-seam.py'
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.ROOT = root
    return module


class CodeHostSeam(unittest.TestCase):
    def tree(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for relative in ('hide-core/src', 'hided/src', 'web/src'):
            (root / relative).mkdir(parents=True, exist_ok=True)
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return load_check(root)

    def found(self, check):
        rust = check.scan(check.RUST_ROOTS, {'.rs'}, check.rust_lines, check.RUST_RULES)
        web = check.scan(check.WEB_ROOTS, {'.ts', '.tsx'}, check.web_lines, check.WEB_RULES)
        return rust + web

    def test_common_code_that_spells_a_host_is_refused(self):
        check = self.tree({
            'hide-core/src/caller.rs': 'fn page(n: u32) -> String { format!("https://github.com/a/b/pull/{n}") }\n'
                                       'fn ask() { let _ = Call::Gh { args: vec![] }; }\n'
                                       'fn pick(key: &str) -> bool { key == "github" }\n',
            'web/src/open.ts': 'export const page = (n: number) => `https://github.com/a/b/pull/${n}`;\n',
        })
        found = self.found(check)
        self.assertEqual(len(found), 4, found)
        self.assertTrue(any('caller.rs:1: a host\'s address' in line for line in found), found)
        self.assertTrue(any('caller.rs:2: a host\'s CLI call' in line for line in found), found)
        self.assertTrue(any('caller.rs:3: a branch on a host\'s key' in line for line in found), found)
        self.assertTrue(any('web/src/open.ts:1' in line for line in found), found)
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(check.main(), 1)

    def test_a_hosts_unit_its_tests_and_hides_own_pages_may_spell_it(self):
        check = self.tree({
            'hide-core/src/github/host.rs': 'const SERVER: &str = "github.com";\n',
            'hide-core/src/caller.rs': 'fn keep() {}\n#[cfg(test)]\nmod tests {\n'
                                       '    const URL: &str = "https://github.com/a/b/pull/1";\n}\n',
            'hide-core/src/code_host/tests.rs': 'const URL: &str = "https://github.com/a/b/pull/1";\n',
            'web/src/open.test.ts': 'const url = "https://github.com/a/b/pull/1";\n',
            'web/src/help.ts': 'export const help = "https://github.com/with-hide/hide/issues";\n',
        })
        self.assertEqual(self.found(check), [])
        with redirect_stdout(io.StringIO()):
            self.assertEqual(check.main(), 0)

    def test_a_named_exception_is_skipped_and_nothing_else_is(self):
        check = self.tree({
            'web/src/PrCard.tsx': 'const files = `${url}/files`;\n',
            'web/src/Other.tsx': 'const files = `${url}/files`;\n',
        })
        found = self.found(check)
        self.assertEqual(len(found), 1, found)
        self.assertIn('web/src/Other.tsx:1', found[0])


if __name__ == '__main__':
    unittest.main()
