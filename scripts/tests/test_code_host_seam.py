"""The code host seam check finds a host's program started, its address or its
key outside its unit."""
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

    def exit_code(self, check):
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return check.main()

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
        self.assertTrue(any('caller.rs:2: a host\'s program started' in line for line in found), found)
        self.assertTrue(any('caller.rs:3: a host\'s key' in line for line in found), found)
        self.assertTrue(any('web/src/open.ts:1' in line for line in found), found)
        self.assertEqual(self.exit_code(check), 1)

    def test_each_door_to_a_hosts_program_and_each_form_of_its_key_alone_is_refused(self):
        probes = {
            'the node call': 'fn ask() { let _ = Call::Gh { args: Vec::new() }; }\n',
            'the node call by its variant': 'use hide_node_link::protocol::Call::{Git, Gh};\n',
            'the program': 'const PROGRAM: &str = "gh";\n',
            'the module': 'use hide_node_link::gh;\n',
            'an item of the module that is not one that starts nothing':
                'const ASK: [&str; 3] = hide_node_link::gh::AUTH_STATUS;\n',
            'the module in a use group': 'use hide_node_link::{NodeLink, gh};\n',
            'the module in a wrapped use group':
                'use hide_node_link::{\n    NodeLink,\n    gh::{AUTH_STATUS, GhAnswer},\n};\n',
            'an address': 'fn page(n: u32) -> String { format!("https://github.com/acme/app/pull/{n}") }\n',
            'the key compared': 'fn pick(key: &str) -> bool { key == "github" }\n',
            'the key in a match arm': 'fn pick(key: &str) -> bool { match key { "github" => true, _ => false } }\n',
            'the key in matches!': 'fn pick(key: &str) -> bool { matches!(key, "github") }\n',
            'the key as the component of a log no file list names':
                'fn log() { diagnostic!(json!({"component": "github", "kind": "x"})); }\n',
            "the key as a common reader's component, once the reader takes the host's":
                'fn log() { diagnostic!(json!({"component":"github","kind":"y"})); }\n',
            "gitlab.com's node call": 'fn ask() { let _ = Call::Glab { args: Vec::new() }; }\n',
            "gitlab.com's node call by its variant": 'use hide_node_link::protocol::Call::{Git, Glab};\n',
            "gitlab.com's program": 'const PROGRAM: &str = "glab";\n',
            "gitlab.com's module": 'use hide_node_link::glab;\n',
            "an item of gitlab.com's module": 'const ASK: [&str; 1] = hide_node_link::glab::VERSION;\n',
            "gitlab.com's module in a use group": 'use hide_node_link::{NodeLink, glab};\n',
            "gitlab.com's answer type": 'fn read(answer: GlabAnswer) {}\n',
            "a gitlab.com address": 'fn page(n: u32) -> String { format!("https://gitlab.com/a/b/-/merge_requests/{n}") }\n',
            "a merge request's page": 'fn page(base: &str, n: u32) -> String { format!("{base}/-/merge_requests/{n}") }\n',
            "gitlab.com's key compared": 'fn pick(key: &str) -> bool { key == "gitlab" }\n',
            "gitlab.com's key in matches!": 'fn pick(key: &str) -> bool { matches!(key, "gitlab") }\n',
            "gitlab.com's key as a log's component": 'fn log() { diagnostic!(json!({"component": "gitlab"})); }\n',
        }
        for what, text in probes.items():
            with self.subTest(what):
                check = self.tree({'hide-core/src/verify_boundary_probe.rs': text})
                self.assertEqual(len(self.found(check)), 1, what)
                self.assertEqual(self.exit_code(check), 1, what)

    def test_argument_words_alone_and_what_starts_nothing_pass(self):
        check = self.tree({
            # A list of a host's argument words reaches its program only
            # through a door the other rules refuse; alone it is dead data,
            # and refusing words would refuse Hide's own flags (--json,
            # --state) too.
            'hide-core/src/words.rs': 'const WORDS: &[&str] = &["pr", "list", "--state", "all", "--json", "number,url"];\n',
            'hide-core/src/categories.rs': 'pub use hide_node_link::gh::GithubFailureCategory;\n'
                                           'fn commit(sha: &str) -> bool { hide_node_link::gh::is_commit(sha) }\n',
            'hide-core/src/github_store.rs': 'fn log() { diagnostic!(json!({"component": "github", "kind": "x"})); }\n'
                                             'fn tight() { diagnostic!(json!({"component":"github","kind":"y"})); }\n',
            'hide-core/src/tasks.rs': 'pub const GITHUB: &str = "github";\n',
        })
        self.assertEqual(self.found(check), [])
        self.assertEqual(self.exit_code(check), 0)

    def test_gitlab_coms_unit_and_its_node_side_may_spell_it(self):
        check = self.tree({
            'hide-core/src/gitlab.rs': 'fn ask() { let _ = Call::Glab { args: hide_node_link::glab::VERSION.map(Into::into).into() }; }\n',
            'hide-core/src/gitlab/host.rs': 'const KEY: &str = "gitlab";\nconst PROGRAM: &str = "glab";\n',
            'hide-node-link/src/glab.rs': 'pub const HOST: &str = "gitlab.com";\n',
            'hide-host/src/glab.rs': 'fn run() { command("glab"); }\n',
        })
        self.assertEqual(self.found(check), [])
        self.assertEqual(self.exit_code(check), 0)

    def test_a_hosts_unit_its_tests_and_hides_own_pages_may_spell_it(self):
        check = self.tree({
            'hide-core/src/github/host.rs': 'const SERVER: &str = "github.com";\n'
                                            'fn ask() { let _ = Call::Gh { args: vec!["pr".into()] }; }\n',
            'hide-core/src/caller.rs': 'fn keep() {}\n#[cfg(test)]\nmod tests {\n'
                                       '    const URL: &str = "https://github.com/a/b/pull/1";\n}\n',
            'hide-core/src/code_host/tests.rs': 'const URL: &str = "https://github.com/a/b/pull/1";\n',
            'web/src/open.test.ts': 'const url = "https://github.com/a/b/pull/1";\n',
            'web/src/help.ts': 'export const help = "https://github.com/with-hide/hide/issues";\n',
        })
        self.assertEqual(self.found(check), [])
        self.assertEqual(self.exit_code(check), 0)

    def test_a_named_exception_is_skipped_and_nothing_else_is(self):
        check = self.tree({
            'web/src/PrLinks.tsx': 'const files = `${url}/files`;\n',
            'web/src/Other.tsx': 'const files = `${url}/files`;\n',
            # A key file may spell the key, and nothing else.
            'hide-core/src/tasks.rs': 'pub const GITHUB: &str = "github";\nconst P: &str = "gh";\n',
        })
        found = self.found(check)
        self.assertEqual(len(found), 2, found)
        self.assertIn('web/src/Other.tsx:1', found[1])
        self.assertIn('hide-core/src/tasks.rs:2: a host\'s program started', found[0])


if __name__ == '__main__':
    unittest.main()
