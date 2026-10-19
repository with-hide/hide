"""Verification must build this checkout and propagate failures, even when warm.

Use a tiny real Cargo workspace at the wrapper boundary. No compiler mocks: a
redirected build directory, skipped tests, and a swallowed compile failure
must change the executable result or exit status.

Every build lands inside the checkout, in `target/`. A caller's
CARGO_TARGET_DIR must not move the release binaries the desktop packager reads
from their fixed path, and a runner's private HOME must not install a second
toolchain.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ('toolchain-env.sh', 'verify-cargo.sh')
# The binaries `release` builds, as the packager names them (desktop/scripts/package.mjs).
RELEASE_BINARIES = ('hided', 'hide', 'hide-agent-hooks')


class BuildFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='hide-build-test-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.env = dict(os.environ, LC_ALL='en_US.UTF-8', LANG='en_US.UTF-8')
        self.env.pop('CARGO_TARGET_DIR', None)

    def checkout(self, parent):
        root = self.base / parent / 'checkout'
        (root / 'scripts').mkdir(parents=True)
        for name in SCRIPTS:
            shutil.copyfile(ROOT / 'scripts' / name, root / 'scripts' / name)
        subprocess.run(['git', 'init', '-q', str(root)], check=True, env=self.env)
        return root


class WrapperArguments(BuildFixture):
    def test_invalid_mode_fails_before_any_build(self):
        root = self.checkout('one')
        result = subprocess.run(['bash', 'scripts/verify-cargo.sh', 'invalid'], cwd=root,
                                env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn('usage:', result.stderr)
        self.assertFalse((root / 'target').exists())


@unittest.skipUnless(sys.platform == 'darwin' and shutil.which('cargo'),
                     'real builds require macOS and Cargo')
class RealVerificationBuilds(BuildFixture):
    def run_wrapper(self, root, mode, *, succeeds=True, **extra):
        home, tmp = self.base / 'runner-home', self.base / 'runner-tmp'
        home.mkdir(exist_ok=True)
        tmp.mkdir(exist_ok=True)
        env = dict(self.env, HOME=str(home), TMPDIR=str(tmp) + '/',
                   CARGO_TARGET_DIR=str(self.base / 'must-not-use'), **extra)
        result = subprocess.run(['bash', 'scripts/verify-cargo.sh', mode], cwd=root, env=env,
                                capture_output=True, text=True)
        if succeeds:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse((self.base / 'must-not-use').exists())
        self.assertFalse((home / '.rustup').exists())
        return result

    def workspace(self, parent, value):
        """The packages `release` names, each one binary that prints the core's value."""
        root = self.checkout(parent)
        members = {'hided': ('hided', 'hide'), 'hide-agent-hooks': ('hide-agent-hooks',)}
        (root / 'Cargo.toml').write_text(
            '[workspace]\nmembers = ["herdr-core", "hided", "hide-agent-hooks"]'
            '\nresolver = "2"\n')
        (root / 'herdr-core/src').mkdir(parents=True)
        (root / 'herdr-core/Cargo.toml').write_text(
            '[package]\nname = "herdr-core"\nversion = "0.1.0"\nedition = "2021"\n')
        self.rust_source(root, value)
        for package, binaries in members.items():
            (root / package / 'src/bin').mkdir(parents=True)
            core = '../' * package.count('/') + '../herdr-core'
            (root / package / 'Cargo.toml').write_text(
                f'[package]\nname = "{package.rsplit("/", 1)[-1]}"\nversion = "0.1.0"\nedition = "2021"\n'
                f'[dependencies]\nherdr-core = {{ path = "{core}" }}\n')
            (root / package / 'src/lib.rs').write_text('')
            for binary in binaries:
                (root / package / 'src/bin' / f'{binary}.rs').write_text(
                    'fn main() { println!("{}", herdr_core::fixture_value()); }\n')
        subprocess.run(['bash', '-ec', '. scripts/toolchain-env.sh; cargo generate-lockfile'],
                       cwd=root, env=self.env, check=True, capture_output=True)
        return root

    def rust_source(self, root, value, expected=None):
        (root / 'herdr-core/src/lib.rs').write_text(
            f'pub fn fixture_value() -> i32 {{ {value} }}\n'
            f'#[test]\nfn current_value() {{ assert_eq!(fixture_value(), {value if expected is None else expected}); }}\n')

    def output(self, root, binary):
        return subprocess.check_output([str(root / 'target/release' / binary)], text=True).strip()

    def test_tests_can_start_the_system_lock_holder_with_an_unsupported_caller_locale(self):
        root = self.workspace('locale', 41)
        self.env.pop('LC_ALL', None)
        self.env.pop('LC_CTYPE', None)
        self.env['LANG'] = 'C.UTF-8'
        (root / 'herdr-core/src/lib.rs').write_text('''
pub fn fixture_value() -> i32 { 41 }
#[test]
fn system_lock_holder_starts() {
    let status = std::process::Command::new("perl")
        .args(["-MFcntl=:flock", "-e", "exit 0;"])
        .status().unwrap();
    assert!(status.success(), "the system lock holder failed before opening its lock");
}
''')
        self.run_wrapper(root, 'test')

    def test_warm_builds_observe_core_and_test_changes_and_propagate_failures(self):
        root = self.workspace('one', 41)
        self.run_wrapper(root, 'test')
        self.assertTrue((root / 'target/debug').is_dir(), 'the test build left the checkout')
        self.run_wrapper(root, 'release')
        for binary in RELEASE_BINARIES:
            built = root / 'target/release' / binary
            self.assertTrue(os.access(built, os.X_OK), f'{binary} is not an executable in target/release')
            self.assertEqual(self.output(root, binary), '41')

        self.rust_source(root, 42, expected=41)
        result = self.run_wrapper(root, 'test', succeeds=False)
        self.assertEqual(result.returncode, 101)
        self.rust_source(root, 42)
        self.run_wrapper(root, 'test')
        self.run_wrapper(root, 'release')
        self.assertEqual(self.output(root, 'hided'), '42', 'the release daemon linked a stale core')

        (root / 'herdr-core/src/lib.rs').write_text('compile_error!("broken core");\n')
        for mode in ('test', 'release'):
            result = self.run_wrapper(root, mode, succeeds=False)
            self.assertIn('broken core', result.stderr)
            self.assertEqual(result.returncode, 101)


if __name__ == '__main__':
    unittest.main()
