"""`fetch-conpty.py` makes the runtime only from the pinned package, and its pin agrees with the holder's."""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = ROOT / "scripts" / "fetch-conpty.py"
CONTRACT = ROOT / "contracts" / "conpty.json"
PSEUDOCON = ROOT / "vendor" / "portable-pty" / "src" / "win" / "psuedocon.rs"

FILES = {
    "conpty.dll": ("runtimes/win-x64/native/conpty.dll", b"dll"),
    "x64/OpenConsole.exe": ("build/native/runtimes/x64/OpenConsole.exe", b"x64 host"),
    "arm64/OpenConsole.exe": ("build/native/runtimes/arm64/OpenConsole.exe", b"arm64 host"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Pin(unittest.TestCase):
    def test_the_contract_pins_the_files_the_holder_checks(self):
        contract = json.loads(CONTRACT.read_text())
        pinned = dict(re.findall(r'\(\s*"([^"]+)",\s*"([0-9a-f]{64})",\s*\)', PSEUDOCON.read_text()))
        self.assertEqual({name: entry["sha256"] for name, entry in contract["files"].items()}, pinned)

    def test_the_holder_looks_for_the_file_list_the_fetch_writes(self):
        self.assertIn('const MANIFEST: &str = "conpty-files.json";', PSEUDOCON.read_text())


class Fetch(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w") as package:
            for source, data in FILES.values():
                package.writestr(source, data)
            package.writestr("inc/conpty.h", b"not taken")
        self.package = archive.getvalue()
        (self.root / "package.nupkg").write_bytes(self.package)

    def contract(self, **changes):
        contract = {
            "package": "Microsoft.Windows.Console.ConPTY",
            "version": "1.0.0",
            "source_url": (self.root / "package.nupkg").as_uri(),
            "sha256": sha(self.package),
            "size": len(self.package),
            "files": {name: {"from": source, "sha256": sha(data)} for name, (source, data) in FILES.items()},
        }
        contract.update(changes)
        path = self.root / "conpty.json"
        path.write_text(json.dumps(contract))
        return path

    def fetch(self, contract, *more):
        env = {**os.environ, "HIDE_CONPTY_CACHE": str(self.root / "cache")}
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), "--contract", str(contract), *more],
            capture_output=True, text=True, env=env,
        )

    def runtime_files(self, folder):
        return sorted(path.relative_to(folder).as_posix() for path in folder.rglob("*") if path.is_file())

    def test_the_pinned_package_makes_the_runtime_folder_and_a_copy_beside_a_program(self):
        made = self.fetch(self.contract(), "--out", str(self.root / "app" / "holder-conpty"))
        self.assertEqual(made.returncode, 0, made.stderr)
        folder = Path(made.stdout.strip().splitlines()[-1])
        self.assertEqual(folder, self.root / "app" / "holder-conpty")
        self.assertEqual(
            self.runtime_files(folder),
            ["arm64/OpenConsole.exe", "conpty-files.json", "conpty.dll", "x64/OpenConsole.exe"],
        )
        self.assertEqual((folder / "x64" / "OpenConsole.exe").read_bytes(), b"x64 host")
        self.assertEqual(self.fetch(self.contract(), "--out", str(self.root / "app" / "holder-conpty")).returncode, 0)

    def test_a_package_of_another_digest_makes_nothing(self):
        refused = self.fetch(self.contract(sha256="0" * 64), "--out", str(self.root / "app" / "holder-conpty"))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("not the pinned", refused.stderr)
        self.assertFalse((self.root / "app").exists())
        self.assertEqual(list((self.root / "cache" / ("0" * 64)).glob("*")), [])

    def test_a_file_of_another_digest_makes_nothing(self):
        files = {name: {"from": source, "sha256": sha(data)} for name, (source, data) in FILES.items()}
        files["conpty.dll"]["sha256"] = "1" * 64
        refused = self.fetch(self.contract(files=files))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("conpty.dll", refused.stderr)
        self.assertEqual(list((self.root / "cache" / sha(self.package)).glob("*")), [])

    def test_a_pin_without_a_digest_is_refused(self):
        contract = self.contract()
        pinned = json.loads(contract.read_text())
        del pinned["sha256"]
        contract.write_text(json.dumps(pinned))
        refused = self.fetch(contract)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("'sha256'", refused.stderr)

    def test_a_changed_copy_beside_a_program_is_refused_not_replaced(self):
        self.assertEqual(self.fetch(self.contract(), "--out", str(self.root / "app" / "holder-conpty")).returncode, 0)
        (self.root / "app" / "holder-conpty" / "conpty.dll").write_bytes(b"changed")
        refused = self.fetch(self.contract(), "--out", str(self.root / "app" / "holder-conpty"))
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual((self.root / "app" / "holder-conpty" / "conpty.dll").read_bytes(), b"changed")


if __name__ == "__main__":
    unittest.main()
