"""hide-vt's build inputs are refused unless they are exactly what is pinned.

`scripts/prepare-vt-inputs.py` is the one place the pinned Zig, the Herdr
source archive and its patches are downloaded and checked; `hide-vt/build.rs`
trusts the folder it leaves. These tests feed it local archives through
`file://` pins, so they need no network. docs/BUILD.md, "The terminal parser
and Zig", owns the reasons.
"""
import hashlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = ROOT / 'scripts' / 'prepare-vt-inputs.py'

spec = importlib.util.spec_from_file_location('prepare_vt_inputs', SCRIPT)
prepare_vt_inputs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare_vt_inputs)

HEADER = 'int answer(void);\n'
PATCHED = 'int answer(void);\nint question(void);\n'
# Adds `question`, in the shape Herdr's patches take: paths under vendor/libghostty-vt.
PATCH = """--- a/vendor/libghostty-vt/include/vt.h
+++ b/vendor/libghostty-vt/include/vt.h
@@ -1 +1,2 @@
 int answer(void);
+int question(void);
"""
# Answers the three calls the script makes of Zig: its version, a project
# manifest to add packages to, and a fetch of none.
STAND_IN_ZIG = b"""#!/bin/sh
case "$1" in
    version) echo 0.16.0 ;;
    init) printf '.{\\n    .name = .stand_in,\\n}\\n' > build.zig.zon ;;
    build) mkdir -p zig-pkg ;;
esac
"""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def tar_bytes(files: dict[str, bytes], mode: str, executable: set[str] = frozenset()) -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode=mode) as archive:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755 if name in executable else 0o644
            archive.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


@unittest.skipIf(sys.platform == 'win32', 'the stand-in Zig is a shell script')
class PrepareVtInputsTest(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        self.cache = self.work / 'cache'
        self.previous_cache = os.environ.get('HIDE_BUILD_CACHE')
        os.environ['HIDE_BUILD_CACHE'] = str(self.cache)

    def tearDown(self):
        if self.previous_cache is None:
            os.environ.pop('HIDE_BUILD_CACHE', None)
        else:
            os.environ['HIDE_BUILD_CACHE'] = self.previous_cache
        subprocess.run(['rm', '-rf', str(self.work)], check=True)

    def publish(self, name: str, data: bytes) -> tuple[str, str]:
        path = self.work / 'published' / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path.as_uri(), sha256(data)

    def manifest(self, header: str, zig_sha: str | None = None) -> Path:
        zig = tar_bytes({'zig-stand-in/zig': STAND_IN_ZIG}, 'w:xz', {'zig-stand-in/zig'})
        zig_url, zig_digest = self.publish('zig.tar.xz', zig)
        source = tar_bytes({
            'herdr-pin/vendor/libghostty-vt/include/vt.h': header.encode(),
            'herdr-pin/vendor/patches/libghostty-vt/0001-question.patch': PATCH.encode(),
        }, 'w:gz')
        source_url, source_digest = self.publish('source.tar.gz', source)
        pins = {
            'zig': {'version': '0.16.0', 'archives': {
                prepare_vt_inputs.zig_host(): {'url': zig_url, 'sha256': zig_sha or zig_digest},
            }},
            'source': {
                'url': source_url, 'sha256': source_digest, 'root': 'herdr-pin',
                'tree': 'vendor/libghostty-vt', 'files': {},
                'patches': {'vendor/patches/libghostty-vt/0001-question.patch': sha256(PATCH.encode())},
            },
            'packages': [],
        }
        path = self.work / 'build-inputs.json'
        path.write_text(json.dumps(pins))
        return path

    def entries(self) -> list[str]:
        root = self.cache / 'vt'
        return sorted(entry.name for entry in root.iterdir()) if root.is_dir() else []

    def test_a_download_whose_digest_differs_is_refused_and_leaves_nothing(self):
        manifest = self.manifest(PATCHED, zig_sha='0' * 64)

        with self.assertRaises(prepare_vt_inputs.Refused) as refusal:
            prepare_vt_inputs.prepare(True, manifest)

        self.assertIn('zig.tar.xz has sha256', str(refusal.exception))
        self.assertIn('pins ' + '0' * 64, str(refusal.exception))
        self.assertEqual(self.entries(), [])

    def test_a_tree_without_its_patch_applied_is_refused(self):
        manifest = self.manifest(HEADER)

        with self.assertRaises(prepare_vt_inputs.Refused) as refusal:
            prepare_vt_inputs.prepare(True, manifest)

        self.assertIn('0001-question.patch is not applied', str(refusal.exception))
        self.assertEqual(self.entries(), [])

    def test_a_ready_entry_is_reused_without_downloading(self):
        manifest = self.manifest(PATCHED)
        first = prepare_vt_inputs.prepare(True, manifest)
        for published in (self.work / 'published').iterdir():
            published.unlink()

        second = prepare_vt_inputs.prepare(True, manifest)

        self.assertEqual(first, second)
        self.assertEqual((first / 'source' / 'include' / 'vt.h').read_text(), PATCHED)
        self.assertEqual(json.loads((first / 'ready.json').read_text())['zig'], '0.16.0')

    def test_only_the_three_most_recent_entries_are_kept(self):
        vt = self.cache / 'vt'
        for age, name in enumerate(['d-old', 'c-older', 'b-oldest']):
            entry = vt / name
            entry.mkdir(parents=True)
            (entry / 'ready.json').write_text('{}')
            stamp = 1_000_000 - age * 1000
            os.utime(entry / 'ready.json', (stamp, stamp))

        prepare_vt_inputs.prepare(True, self.manifest(PATCHED))

        self.assertEqual(len(self.entries()), 3)
        self.assertNotIn('b-oldest', self.entries())


if __name__ == '__main__':
    unittest.main()
