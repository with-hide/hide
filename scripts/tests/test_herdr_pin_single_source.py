"""`check-herdr-pin-single-source.sh` finds the pinned version as a whole token, not as part of another number."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = ROOT / "scripts" / "check-herdr-pin-single-source.sh"
DERIVED = (
    "scripts/fetch-herdr-runtime.sh", "scripts/fetch-herdr-runtime.ps1", "web/e2e/herdr-fixture.ts",
    "desktop/e2e/fixture.ts", "scripts/web-shell-measure/isolated-env.sh",
)


@unittest.skipUnless(shutil.which("zsh") and shutil.which("jq"), "the check is a zsh script that needs jq")
class PinSingleSource(unittest.TestCase):
    def check(self, text):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        root = Path(folder.name)
        (root / "scripts").mkdir()
        shutil.copy(SCRIPT, root / "scripts" / SCRIPT.name)
        (root / "contracts").mkdir()
        (root / "contracts" / "herdr-bundle.json").write_text(json.dumps({
            "repo": "example/herdr", "version": "0.9.1", "sha256": "a" * 64,
            "linux_x86_64": {"sha256": "b" * 64}, "windows_x86_64": {"sha256": "c" * 64},
        }))
        for relative in DERIVED:
            (root / relative).parent.mkdir(parents=True, exist_ok=True)
            (root / relative).write_text("")
        (root / DERIVED[0]).write_text(text)
        return subprocess.run(["zsh", str(root / "scripts" / SCRIPT.name)], capture_output=True, text=True).returncode

    def test_another_tools_longer_version_is_not_the_pin(self):
        for text in ("nextest/0.9.143/linux\n", "version 10.9.1 here\n", "x 0.9.12\n", "0.9.1.4\n", "1.0.9.1\n"):
            with self.subTest(text=text):
                self.assertEqual(self.check(text), 0)

    def test_the_pin_itself_is_found_however_it_is_written(self):
        for text in ("herdr 0.9.1\n", "v0.9.1\n", "0.9.1", "release/0.9.1/linux\n", "0.9.1.\n", "0.9.1-rc1\n", "pin: '0.9.1'\n"):
            with self.subTest(text=text):
                self.assertEqual(self.check(text), 1)

    def test_a_dot_in_the_version_is_not_a_wildcard(self):
        self.assertEqual(self.check("0x9y1\n"), 0)


if __name__ == "__main__":
    unittest.main()
