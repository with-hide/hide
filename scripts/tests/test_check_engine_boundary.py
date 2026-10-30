"""The engine boundary check must find Herdr spoken outside the engine, in
production code only, and say what to use instead."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_check(root):
    spec = importlib.util.spec_from_file_location(
        'check_engine_boundary', ROOT / 'scripts/check-engine-boundary.py'
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.ROOT = root
    module.RUST_TREES = ['hided/src', 'herdr-core/src']
    module.TS_TREES = ['desktop/src']
    module.ALLOWED = {}
    module.FIXTURES = {}
    return module


class EngineBoundary(unittest.TestCase):
    def tree(self, files):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        for tree in ('hided/src', 'herdr-core/src', 'desktop/src'):
            (root / tree).mkdir(parents=True, exist_ok=True)
        for relative, text in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return root, load_check(root)

    def failures(self, check):
        import contextlib
        import io
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status = check.main()
        return status, stderr.getvalue()

    def test_herdr_spoken_outside_the_engine_is_found_with_its_line_and_what_to_use(self):
        root, check = self.tree({'hided/src/reader.rs':
            'use hide_herdr_client::request_with_connector;\n'
            'fn read(c: &dyn ApiConnector) {}\n'
            'fn pong() { herdr_core::engine::herdr::wire::ping_params(); }\n'})
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        for line in (1, 2, 3):
            self.assertIn(f'hided/src/reader.rs:{line}:', output)
        self.assertIn('herdr_core::engine::EngineHandle', output)

    def test_the_handoff_reached_from_anywhere_else_is_found(self):
        root, check = self.tree({'hided/src/probe.rs':
            'fn probe() { herdr_core::engine::herdr::handoff::ping_params(); }\n'})
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        self.assertIn('hided/src/probe.rs:1:', output)
        self.assertIn('only `hided/src/herdr_handoff.rs` may', output)

    def test_starting_the_herdr_program_is_found(self):
        root, check = self.tree({'hided/src/start.rs':
            'fn status(herdr_bin: &Path) {\n'
            '    let mut command = Command::new(herdr_bin);\n'
            '}\n'
            'fn git() { Command::new("git"); }\n'})
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        self.assertIn('hided/src/start.rs:2:', output)
        self.assertIn('starts the Herdr program', output)
        self.assertNotIn('hided/src/start.rs:4:', output)

    def test_the_desktop_host_naming_the_herdr_program_is_found(self):
        root, check = self.tree({'desktop/src/main/spawn.ts':
            'const env = { HERDR_BIN_PATH: bundled };\n'})
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        self.assertIn('desktop/src/main/spawn.ts:1:', output)

    def test_the_engine_implementation_tests_and_comments_are_not_findings(self):
        root, check = self.tree({
            'herdr-core/src/engine/herdr.rs': 'use hide_herdr_client::ApiConnector;\n',
            'herdr-core/src/engine/herdr/wire.rs': 'use hide_herdr_client::ApiError;\n',
            'hided/src/quiet.rs': '// hide_herdr_client is the engine\'s\n'
                                  'fn advice() -> &\'static str { "not ApiConnector" }\n'
                                  '#[cfg(test)]\nmod tests {\n    use hide_herdr_client::ApiError;\n}\n',
            'hided/src/quiet_tests.rs': 'use hide_herdr_client::ApiError;\n',
            'desktop/src/main/herdr.test.ts': 'const HERDR_BIN_PATH = "x";\n',
        })
        self.assertEqual(self.failures(check), (0, ''))

    def test_a_new_finding_in_an_allowed_file_fails_and_a_stale_entry_fails(self):
        root, check = self.tree({'hided/src/handoff.rs': 'use hide_herdr_client::ApiError;\n'})
        check.ALLOWED = {'hided/src/handoff.rs': (1, 'the handoff; bundle 4 deletes it')}
        self.assertEqual(self.failures(check)[0], 0)
        (root / 'hided/src/handoff.rs').write_text(
            'use hide_herdr_client::ApiError;\nfn c(c: &dyn ApiConnector) {}\n')
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        self.assertIn('listed with 1 finding(s)', output)
        (root / 'hided/src/handoff.rs').write_text('fn spoken_through_the_engine() {}\n')
        status, output = self.failures(check)
        self.assertEqual(status, 1)
        self.assertIn('no longer speaks Herdr; remove the entry', output)

    def test_every_allowed_entry_names_the_bundle_that_deletes_it(self):
        check = importlib.util.module_from_spec(importlib.util.spec_from_file_location(
            'check_engine_boundary_entries', ROOT / 'scripts/check-engine-boundary.py'))
        check.__spec__.loader.exec_module(check)
        for path, (_, reason) in check.ALLOWED.items():
            self.assertRegex(reason, r'bundle [234]', path)

    def test_the_repository_passes(self):
        spec = importlib.util.spec_from_file_location(
            'check_engine_boundary_repository', ROOT / 'scripts/check-engine-boundary.py')
        check = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(check)
        self.assertEqual(self.failures(check), (0, ''))


if __name__ == '__main__':
    unittest.main()
