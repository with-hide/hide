"""The production-only reader boundary must reject real callers, not test names."""
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

READERS = ('changes', 'ports', 'worktrees', 'code_host/read', 'github_poll', 'disk', 'ai', 'usage')
REQUESTS = ('worktrees', 'github', 'github_poll', 'disk', 'ai')


def write_readers(sources, coordinator_drives_changes=False):
    """Each reader's module, the coordinator driving all but the changes reader
    and GitHub's watch, and GitHub's unit making its watch reader, as in the
    real tree."""
    for reader in READERS:
        module = sources / f'{reader}.rs'
        module.parent.mkdir(parents=True, exist_ok=True)
        module.write_text('fn read_if_due() {}\n// BackgroundRead\n')
    (sources / 'handle.rs').write_text('let changes = changes::ChangesPump::spawn();\n')
    (sources / 'session_sync.rs').write_text('mod coordinator;\n')
    driven = [f'let {r}_reader = {r}::Reader::new();' for r in ('worktrees', 'disk', 'ai', 'ports', 'usage')]
    driven.append('let github_reader = crate::code_host::read::CodeHostReader::new(node);')
    driven.append('let watches = hosts.map(|watch| watch.reader(node));')
    if coordinator_drives_changes:
        driven.append('let changes_reader = changes::Reader::new();')
    (sources / 'session_sync').mkdir(exist_ok=True)
    (sources / 'session_sync' / 'coordinator.rs').write_text(
        '\n'.join(driven) + '\n' + '\n'.join(f'let Some(request) = read_{r}_request();' for r in REQUESTS)
    )
    (sources / 'github').mkdir(exist_ok=True)
    (sources / 'github' / 'host.rs').write_text('Box::new(crate::github_poll::GithubPollReader::new(node))\n')


class CapabilityReaderGate(unittest.TestCase):
    def run_gate(self, runtime_source, test_support=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            sources = root / 'hide-core' / 'src'
            sources.mkdir(parents=True)
            runtime_modules = sources / 'runtime'
            runtime_modules.mkdir()
            session_sync_modules = sources / 'session_sync'
            session_sync_modules.mkdir()
            shutil.copy2(ROOT / 'scripts/check-capability-readers-off-lock.sh', root / 'scripts')
            write_readers(sources)
            (sources / 'runtime.rs').write_text(runtime_source)
            if test_support is not None:
                (runtime_modules / 'tests.rs').write_text(test_support)
            (sources / 'files.rs').write_text('// no subprocess\n#[cfg(test)]\nmod tests {}\n')
            return subprocess.run(['bash', 'scripts/check-capability-readers-off-lock.sh'],
                                  cwd=root, capture_output=True, text=True)

    def test_inline_tests_do_not_drive_production_readers(self):
        result = self.run_gate('fn changes_request() {}\n#[cfg(test)]\nmod tests {\n'
                               'fn explorer_keeps_changes_reader_alive() {}\n}\n')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_production_reader_outside_coordinator_is_rejected(self):
        result = self.run_gate('fn dispatch() { let reader = changes::ChangesReader::new(); }\n'
                               '#[cfg(test)]\nmod tests {}\n')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('hide-core/src/runtime.rs', result.stderr)
        self.assertIn('driven from outside', result.stderr)

    def test_changes_reader_driven_by_the_coordinator_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            sources = root / 'hide-core' / 'src'
            (sources / 'runtime').mkdir(parents=True)
            (sources / 'session_sync').mkdir()
            shutil.copy2(ROOT / 'scripts/check-capability-readers-off-lock.sh', root / 'scripts')
            write_readers(sources, coordinator_drives_changes=True)
            (sources / 'runtime.rs').write_text('#[cfg(test)]\nmod tests {}\n')
            (sources / 'files.rs').write_text('#[cfg(test)]\nmod tests {}\n')
            result = subprocess.run(['bash', 'scripts/check-capability-readers-off-lock.sh'],
                                    cwd=root, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('the changes reader is driven from outside its own pump', result.stderr)

    def test_runtime_submodule_subprocess_is_rejected(self):
        runtime_module = ROOT / 'scripts' / 'check-capability-readers-off-lock.sh'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            sources = root / 'hide-core' / 'src'
            sources.mkdir(parents=True)
            (sources / 'runtime').mkdir()
            (sources / 'session_sync').mkdir()
            shutil.copy2(runtime_module, root / 'scripts')
            write_readers(sources)
            (sources / 'runtime.rs').write_text('#[cfg(test)]\nmod tests {}\n')
            (sources / 'files.rs').write_text('#[cfg(test)]\nmod tests {}\n')
            (sources / 'runtime' / 'projects.rs').write_text(
                'fn read_project() { std::process::Command::new("git"); }\n'
            )
            result = subprocess.run(['bash', 'scripts/check-capability-readers-off-lock.sh'],
                                    cwd=root, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('hide-core/src/runtime/projects.rs', result.stderr)

    def test_test_only_runtime_support_file_is_exempt(self):
        result = self.run_gate(
            '#[cfg(test)]\nmod tests {}\n',
            'fn fixture_repository() {\n'
            '    std::process::Command::new("git");\n'
            '    let _ = changes::ChangesReader::new();\n'
            '}\n',
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
