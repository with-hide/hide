"""The CLI pin table every CI install reads, and the one download helper every
pinned released file comes through: a row the installer cannot install, a
download curl was not told to retry, and a pin a downloaded file does not match
fail here, not on a runner. Curl's own retry, timeout and truncation behavior
is curl's, and is not retested here."""

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

URL = "https://example.invalid/releases/v1.2.3/omp-linux-x64"


class Curl:
    """curl as a subprocess would run it: each call is recorded, and the run
    writes `payload` where `--output` names or exits with `code` and `error`."""

    def __init__(self, payload=b"the file", code=0, error=""):
        self.payload = payload
        self.code = code
        self.error = error
        self.calls = []

    def __call__(self, argv, **options):
        self.calls.append(argv)
        if self.code:
            return subprocess.CompletedProcess(argv, self.code, "", self.error)
        Path(argv[argv.index("--output") + 1]).write_bytes(self.payload)
        return subprocess.CompletedProcess(argv, 0, "", "")

    @property
    def command(self):
        return self.calls[-1]


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


class Grok(unittest.TestCase):
    def installer(self, root):
        """A finite Bash installer fixture with the vendor's PATH condition,
        and the file a run of it leaves as proof it ran."""
        ran = root / "installer-ran"
        script = root / "install.sh"
        script.write_text(f'set -eu\n: > {str(ran)!r}\nmkdir -p "$GROK_BIN_DIR"\n'
                          ': > "$GROK_BIN_DIR/grok"\n'
                          'case ":$PATH:" in *":$GROK_BIN_DIR:"*) ;;\n'
                          f'*) ln -s "$GROK_BIN_DIR/grok" {str(root / "outside-link")!r} ;; esac\n')
        return script, ran

    @unittest.skipIf(os.name == "nt", "Unix installer link behavior")
    def test_native_installer_sees_its_target_on_path_and_leaves_no_other_link(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prefix = root / "cache"
            (prefix / "home").mkdir(parents=True)
            script, ran = self.installer(root)
            with mock.patch.dict(os.environ, {"CI": "true", "HOME": str(prefix / "home")}), \
                    mock.patch.object(install, "GROK_INSTALLER", script.as_uri()):
                self.assertEqual(install.main(["grok", "--prefix", str(prefix), "--private-fake"]), 0)
            self.assertTrue(ran.is_file(), "the installer never ran")
            self.assertTrue((prefix / "grok/grok").is_file())
            self.assertFalse((root / "outside-link").is_symlink())

    def test_a_failed_download_stops_the_step_and_never_runs_the_installer(self):
        # curl's own exit and error are the step's, and Bash never sees a file.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script, ran = self.installer(root)
            curl = Curl(code=22, error="curl: (22) The requested URL returned error: 403")
            with mock.patch.object(install, "GROK_INSTALLER", script.as_uri()), \
                    mock.patch.object(install.shutil, "which", return_value="/usr/bin/curl"), \
                    mock.patch.object(install.subprocess, "run", self.only(curl)):
                with self.assertRaises(SystemExit) as stopped:
                    install.grok({"version": "1.0.52"}, root)
            self.assertIn("curl exited 22", str(stopped.exception))
            self.assertIn("error: 403", str(stopped.exception))
            self.assertFalse(ran.exists(), "the step ran the installer after its download failed")

    def only(self, curl):
        """The process runner where curl is the only program that may run: a
        Bash call means the installer ran after its download failed."""
        def runner(argv, **options):
            if argv[0] != "/usr/bin/curl":
                raise AssertionError(f"the installer ran after its download failed: {argv}")
            return curl(argv, **options)
        return runner


class Download(unittest.TestCase):
    def install_pi(self, answer):
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
        self.assertEqual(self.install_pi(clis.table()["pi"]["version"]), 0)

    def test_another_version_than_the_pin_fails_the_install(self):
        with self.assertRaises(SystemExit) as stopped:
            self.install_pi("9.9.9")
        self.assertIn("pi answered '9.9.9', agent-clis.json pins", str(stopped.exception))

    def fetch(self, curl, dest):
        with mock.patch.object(install.shutil, "which", return_value="/usr/bin/curl"), \
                mock.patch.object(install.subprocess, "run", curl):
            return install.fetch(URL, dest)

    def test_the_download_asks_curl_to_retry_a_transient_failure_and_bound_its_time(self):
        # The flags are curl's contract with the step: what it retries, what it
        # never retries, and how long one attempt and the whole run may take.
        with tempfile.TemporaryDirectory() as temporary:
            dest = Path(temporary) / "file"
            curl = Curl()
            self.assertEqual(self.fetch(curl, dest), dest)
            self.assertEqual(dest.read_bytes(), b"the file")
            command = curl.command
        options = {}
        for index, word in enumerate(command):
            if word.startswith("--") and index + 1 < len(command) and not command[index + 1].startswith("-"):
                options[word] = command[index + 1]
            else:
                options.setdefault(word, True)
        self.assertEqual(command[0], "/usr/bin/curl")
        for flag in ("--fail", "--location", "--silent", "--show-error", "--retry-connrefused"):
            self.assertIn(flag, options, f"curl was given no {flag}")
        self.assertEqual(options["--retry"], "3")
        self.assertEqual(options["--output"], str(dest))
        self.assertEqual(command[-1], URL)
        # Every timeout is a number, so none is a flag curl would read as one.
        for flag in ("--connect-timeout", "--max-time", "--retry-max-time"):
            self.assertRegex(options[flag], r"^\d+$", f"curl was given no time for {flag}")

    def test_a_failed_download_ends_the_step_with_curl_s_exit_and_its_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(SystemExit) as stopped:
                self.fetch(Curl(code=18, error="curl: (18) end of response with 9 bytes missing"),
                           Path(temporary) / "file")
        self.assertIn("curl exited 18", str(stopped.exception))
        self.assertIn("9 bytes missing", str(stopped.exception))

    def install_kind(self, kind, help_text="", render=""):
        """main() for `kind` at its pin, its `--help` answering `help_text` and
        anything else it is asked answering `render`."""
        with tempfile.TemporaryDirectory() as temporary:
            program = Path(temporary) / kind
            program.write_text("")
            with mock.patch.dict(os.environ, {"CI": "true", "RUNNER_TEMP": temporary}), \
                    mock.patch.dict(os.environ, {"GITHUB_PATH": "", "GITHUB_OUTPUT": ""}), \
                    mock.patch.object(install, "npm", return_value={kind: program}), \
                    mock.patch.object(install, "answered", return_value=clis.table()[kind]["version"]), \
                    mock.patch.object(install, "helped", return_value=help_text), \
                    mock.patch.object(install, "rendered", return_value=render) as asked:
                return install.main([kind]), asked

    def test_every_started_agent_has_its_full_access_flag_in_the_contract(self):
        flags = install.full_access_flags()
        self.assertEqual(flags["opencode"], ["--auto"])
        self.assertEqual(flags["pi"], [])
        self.assertEqual(install.full_access_words("codex"),
                         ["-c", "approval_policy=never", "-c", "sandbox_mode=danger-full-access"])

    def test_a_cli_whose_help_lists_its_full_access_flag_installs(self):
        self.assertEqual(self.install_kind("opencode", "      --auto  run without asking\n")[0], 0)

    def test_a_cli_whose_help_lacks_its_full_access_flag_fails_the_install(self):
        # A longer option that starts with the flag is not the flag.
        with self.assertRaises(SystemExit) as stopped:
            self.install_kind("opencode", "      --auto-later\n")
        self.assertIn("--help lists no --auto", str(stopped.exception))

    def test_codex_installs_when_its_full_access_words_render_full_access(self):
        # Codex 0.166.0's prompt input with those words (measured 2026-11-01).
        render = "Approval policy is currently never. ... `sandbox_mode` is `danger-full-access`: No filesystem sandboxing"
        code, asked = self.install_kind("codex", render=render)
        self.assertEqual(code, 0)
        self.assertEqual(asked.call_args.args[1:],
                         (["-c", "approval_policy=never", "-c", "sandbox_mode=danger-full-access"],
                          ["debug", "prompt-input"]))

    def test_codex_whose_full_access_words_start_it_with_defaults_fails_the_install(self):
        # A key Codex no longer knows is ignored, so the approval line goes
        # missing while the command still succeeds.
        with self.assertRaises(SystemExit) as stopped:
            self.install_kind("codex", help_text="  -c, --config <key=value>\n",
                              render="`sandbox_mode` is `danger-full-access`")
        self.assertIn("lacks Approval policy is currently never", str(stopped.exception))

    def install_agy(self, url, payload, sha512):
        """agy() on this runner, the row naming `url` and `sha512` and curl
        writing `payload`, the check reading the file curl left."""
        row = {"version": "1.4.8", "installer": "agy", "files": {install.target(): {"url": url, "sha512": sha512}}}
        folder = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(folder, ignore_errors=True))
        curl = Curl(payload=payload)

        def fetch(url, dest):
            curl(["/usr/bin/curl", "--output", str(dest), url])
            return dest

        with mock.patch.object(install, "fetch", fetch):
            self.prefix = folder
            program = install.agy(row, folder)
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
        # A file of another digest is refused and leaves no program to run.
        with self.assertRaisesRegex(SystemExit, "does not match the sha512"):
            self.install_agy(url, payload, hashlib.sha512(b"another release").hexdigest())
        self.assertFalse((self.prefix / "agy").exists(), "a refused file still left a program")

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
