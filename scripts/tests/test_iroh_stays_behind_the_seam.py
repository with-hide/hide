"""Only hide-peer-iroh may link the transport, and nothing that must stay
free of it may reach that crate."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SEAM = '''[package]
name = "hide-peer-iroh"
[dependencies]
hide-peer = { path = "../hide-peer" }
iroh = { version = "=1.4.1", default-features = false, features = ["tls-ring"] }
noq = { version = "=1.4.0", default-features = false }
'''


def load_check():
    spec = importlib.util.spec_from_file_location(
        'check_iroh_stays_behind_the_seam', ROOT / 'scripts/check-iroh-stays-behind-the-seam.py'
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def package(name, dependencies=''):
    return f'[package]\nname = "{name}"\n[dependencies]\n{dependencies}'


def lock(packages):
    """A lock file of `packages`: name to (source or None, dependencies)."""
    text = ''
    for name, (source, dependencies) in packages.items():
        text += f'[[package]]\nname = "{name}"\nversion = "1.0.0"\n'
        if source:
            text += f'source = "{source}"\n'
        text += f'dependencies = {list(dependencies)!r}\n'.replace("'", '"')
    return text


REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'


class IrohStaysBehindTheSeam(unittest.TestCase):
    def tree(self, crates, locked=None):
        """A workspace of `crates` (name to manifest text) beside the seam,
        with `locked` as its lock file."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        crates = {'hide-peer-iroh': SEAM, 'hide-peer': package('hide-peer'), **crates}
        names = ', '.join(f'"{name}"' for name in crates)
        (root / 'Cargo.toml').write_text(f'[workspace]\nmembers = [{names}]\n')
        for name, text in crates.items():
            (root / name).mkdir()
            (root / name / 'Cargo.toml').write_text(text)
        (root / 'Cargo.lock').write_text(lock(locked or {}))
        return load_check().findings(root)

    def test_the_repository_keeps_the_transport_behind_the_seam(self):
        self.assertEqual(load_check().findings(ROOT), [])

    def test_a_daemon_that_links_the_seam_is_allowed(self):
        found = self.tree({
            'hided': package('hided', 'hide-peer-iroh = { path = "../hide-peer-iroh" }\n'
                                      'hide-core = { path = "../hide-core" }\n'),
            'hide-core': package('hide-core'),
        })
        self.assertEqual(found, [])

    def test_another_member_naming_a_transport_crate_is_found(self):
        found = self.tree({'hided': package('hided', 'iroh-base = "1"\n')})
        self.assertEqual(len(found), 1, found)
        self.assertIn('hided depends on iroh-base', found[0])

    def test_a_renamed_transport_dependency_is_found(self):
        found = self.tree({'hide-peer': package('hide-peer', 'quic = { package = "noq", version = "1" }\n')})
        self.assertEqual(len(found), 1, found)
        self.assertIn('hide-peer depends on noq', found[0])

    def test_a_guarded_member_reaching_the_seam_through_another_is_found(self):
        found = self.tree({
            'hide-node': package('hide-node', 'hide-kit = { path = "../hide-kit" }\n'),
            'hide-kit': '[package]\nname = "hide-kit"\n[target."cfg(unix)".dev-dependencies]\n'
                        'hide-peer-iroh = { path = "../hide-peer-iroh" }\n',
        })
        self.assertEqual(len(found), 1, found)
        self.assertIn('hide-node reaches hide-peer-iroh (hide-node -> hide-kit -> hide-peer-iroh)', found[0])

    def test_workspace_hack_may_list_what_another_member_reaches_too(self):
        found = self.tree({}, locked={
            'hided': (None, ['tokio', 'workspace-hack']),
            'hide-peer-iroh': (None, ['iroh', 'workspace-hack']),
            'iroh': (REGISTRY, ['tokio', 'netwatch']),
            'tokio': (REGISTRY, []),
            'netwatch': (REGISTRY, []),
            'workspace-hack': (None, ['tokio']),
        })
        self.assertEqual(found, [])

    def test_a_crate_only_the_seam_reaches_in_workspace_hack_is_found(self):
        found = self.tree({}, locked={
            'hided': (None, ['tokio', 'workspace-hack']),
            'hide-peer-iroh': (None, ['iroh', 'workspace-hack']),
            'iroh': (REGISTRY, ['tokio', 'netwatch']),
            'tokio': (REGISTRY, []),
            'netwatch': (REGISTRY, []),
            'workspace-hack': (None, ['tokio', 'netwatch']),
        })
        self.assertEqual(len(found), 1, found)
        self.assertIn('workspace-hack lists netwatch 1.0.0, which only hide-peer-iroh reaches', found[0])

    def test_a_loose_pin_and_default_features_are_found(self):
        loose = SEAM.replace('"=1.4.1", default-features = false', '"1.4"')
        found = self.tree({'hide-peer-iroh': loose})
        self.assertEqual(len(found), 2, found)
        self.assertIn("pin it exactly", found[0])
        self.assertIn("default features", found[1])


if __name__ == '__main__':
    unittest.main()
