"""The CLI pin table every CI install reads: a row the installer cannot
install fails here, not on a runner."""

from pathlib import Path
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
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from agent_journey import clis  # noqa: E402

spec = importlib.util.spec_from_file_location("install_agent_clis", SCRIPTS / "install-agent-clis.py")
install = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install)


class Table(unittest.TestCase):
    def test_every_row_names_a_version_and_an_installer_the_script_has(self):
        rows = install.table()
        for kind, row in rows.items():
            with self.subTest(kind=kind):
                self.assertRegex(row["version"], r"^\d+\.\d+\.\d+$")
                if row["installer"] == "npm":
                    self.assertTrue(row.get("package"))

    def test_an_agy_row_without_a_url_and_sha512_for_each_system_is_refused(self):
        pinned = json.loads(clis.PINNED.read_text())
        row = pinned["clis"]["agy"]
        for system in clis.AGY_SYSTEMS:
            for field in ("url", "sha512"):
                with self.subTest(system=system, field=field):
                    files = {**row["files"], system: {key: value for key, value in row["files"][system].items()
                                                      if key != field}}
                    with mock.patch.object(install, "pins", return_value={"agy": {**row, "files": files}}), \
                            self.assertRaisesRegex(SystemExit, f"agy needs a url and sha512 for {system}"):
                        install.table()
        with mock.patch.object(install, "pins", return_value={"agy": {**row, "files": {}}}), \
                self.assertRaises(SystemExit):
            install.table()

    def test_every_journey_agent_has_a_pin(self):
        contract = json.loads((SCRIPTS.parent / "contracts/agent-journey.json").read_text())
        kinds = {row["herdr_kind"] for row in contract if row["id"] != "cursor"}
        self.assertEqual(kinds - set(clis.table()), set())

    def test_a_version_matches_only_itself(self):
        self.assertTrue(clis.matches("1.4.0", "agy 1.4.0 (abc)"))
        self.assertFalse(clis.matches("1.4.0", "agy 1.4.01"))
        self.assertFalse(clis.matches("1.0.4", "11.0.4"))


class Install(unittest.TestCase):
    @unittest.skipIf(os.name == "nt", "Unix installer link behavior")
    def test_native_installer_sees_its_target_on_path_and_leaves_no_other_link(self):
        # The external download is a finite Bash installer fixture with the
        # vendor's PATH condition. Run our actual entrypoint and Bash route.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prefix, tools = root / "cache", root / "tools"
            home = prefix / "home"
            home.mkdir(parents=True)
            tools.mkdir()
            outside = root / "outside-link"
            installer = ('set -eu\n'
                         'mkdir -p "$GROK_BIN_DIR"\n'
                         ': > "$GROK_BIN_DIR/grok"\n'
                         'case ":$PATH:" in *":$GROK_BIN_DIR:"*) ;;\n'
                         f'*) ln -s "$GROK_BIN_DIR/grok" {str(outside)!r} ;; esac\n')
            curl = tools / "curl"
            curl.write_text(f"#!{sys.executable}\nprint({installer!r})\n")
            curl.chmod(0o700)
            env = {"HOME": str(home), "PATH": os.pathsep.join((str(tools), "/usr/bin", "/bin")),
                   "LC_ALL": "C"}
            ran = subprocess.run([sys.executable, str(SCRIPTS / "install-agent-clis.py"), "grok",
                                  "--prefix", str(prefix), "--private-fake"],
                                 env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(ran.returncode, 0, ran.stdout + ran.stderr)
            self.assertTrue((prefix / "grok/grok").is_file())
            self.assertFalse(outside.is_symlink())

    def install(self, answer):
        """main() for Pi, its npm install replaced by a program that answers `answer`."""
        with tempfile.TemporaryDirectory() as temporary:
            program = Path(temporary) / "pi"
            program.write_text("")
            with mock.patch.dict(os.environ, {"CI": "true", "RUNNER_TEMP": temporary}), \
                    mock.patch.dict(os.environ, {"GITHUB_PATH": "", "GITHUB_OUTPUT": ""}), \
                    mock.patch.object(install, "npm", return_value={"pi": program}), \
                    mock.patch.object(install, "answered", return_value=answer):
                return install.main(["pi"])

    def test_the_pinned_version_installs(self):
        self.assertEqual(self.install(clis.table()["pi"]["version"]), 0)

    def test_another_version_than_the_pin_fails_the_install(self):
        with self.assertRaises(SystemExit) as stopped:
            self.install("9.9.9")
        self.assertIn("pi answered '9.9.9', agent-clis.json pins", str(stopped.exception))

    def install_agy(self, url, payload, sha512):
        """agy() on this runner, the row naming `url` and `sha512` and the download answering `payload`."""
        row = {"version": "1.4.8", "installer": "agy", "files": {install.target(): {"url": url, "sha512": sha512}}}
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(install, "fetch", return_value=payload):
            program = install.agy(row, Path(temporary))
            return program.name, program.read_bytes()

    def test_agy_installs_the_program_of_the_recorded_archive_only_when_its_sha512_matches(self):
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w:gz") as writer:
            member = tarfile.TarInfo("antigravity")
            member.size = len(b"agy program")
            writer.addfile(member, io.BytesIO(b"agy program"))
        payload = archive.getvalue()
        url = "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.4.8-1/x/cli.tar.gz"
        name, content = self.install_agy(url, payload, hashlib.sha512(payload).hexdigest())
        self.assertEqual((name, content), ("agy.exe" if os.name == "nt" else "agy", b"agy program"))
        with self.assertRaisesRegex(SystemExit, "does not match the sha512"):
            self.install_agy(url, payload, hashlib.sha512(b"another release").hexdigest())

    def test_agy_installs_a_recorded_program_as_it_is(self):
        url = "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.4.8-1/windows-x64/cli.exe"
        self.assertEqual(self.install_agy(url, b"exe", hashlib.sha512(b"exe").hexdigest())[1], b"exe")

    def test_npm_runs_the_install_scripts_of_only_the_rows_that_name_them(self):
        rows = install.table()
        kinds = [kind for kind, row in rows.items() if row["installer"] == "npm"]
        calls = []
        with mock.patch.object(install, "run", side_effect=lambda argv, env=None: calls.append(argv)), \
                mock.patch.object(install.shutil, "which", return_value="npm"):
            install.npm(kinds, rows, Path("prefix"))
        self.assertIn("--ignore-scripts", calls[0])
        self.assertEqual(calls[1][:2], ["npm", "rebuild"])
        self.assertEqual(set(calls[1][4:]), {rows[kind]["package"] for kind in kinds if rows[kind].get("install_scripts")})


if __name__ == "__main__":
    unittest.main()
