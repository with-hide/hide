"""The CLI pin table every CI install reads, and the one download helper every
pinned released file comes through: a row the installer cannot install and a
download it should not retry fail here, not on a runner."""

from pathlib import Path
import contextlib
import hashlib
import http.client
import importlib.util
import io
import json
import os
import socket
import ssl
import subprocess
import sys
import tarfile
import tempfile
import unittest
import urllib.error
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from agent_journey import clis  # noqa: E402

spec = importlib.util.spec_from_file_location("install_agent_clis", SCRIPTS / "install-agent-clis.py")
install = importlib.util.module_from_spec(spec)
spec.loader.exec_module(install)

URL = "https://example.invalid/releases/v1.2.3/omp-linux-x64"


class Clock:
    """The clock a download runs on: it moves when the code sleeps and when a
    body arrives, so a retry's waits, its budget and the time a transfer takes
    are checked without waiting for any of them."""

    def __init__(self):
        self.now = 100.0
        self.slept = []

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.slept.append(seconds)
        self.now += seconds

    def advance(self, seconds):
        self.now += seconds


class Answer:
    """One HTTP answer, as `urlopen` returns it: a body read inside a `with`.
    The server sends `arriving` bytes every `seconds`, so the clock shows what a
    trickle does to a deadline. `read1` returns the bytes that have arrived, as
    a real answer does; `read` waits to fill what it was asked for, so a body
    taken through it checks the deadline once per chunk instead of per piece.
    The answer can also raise the error a body still ends in partway through."""

    def __init__(self, payload=b"", clock=None, seconds=0.0, raises=None, arriving=None):
        self.payload = payload
        self.clock = clock
        self.seconds = seconds
        self.raises = raises
        self.arriving = arriving
        self.pieces = 0

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False

    def read1(self, size=-1):
        if size is None or size < 0 or (self.arriving and self.arriving < size):
            size = self.arriving or size
        return self.take(size)

    def read(self, size=-1):
        if size is None or size < 0:
            return self.take(size)
        piece = b""
        while len(piece) < size:
            more = self.take(size - len(piece))
            if not more:
                break
            piece += more
        return piece

    def take(self, size):
        # The piece takes its time either way, and a body that ends partway
        # spends that time before it reports the failure.
        if self.clock is not None:
            self.clock.advance(self.seconds)
        self.pieces += 1
        if self.raises is not None:
            error, self.raises = self.raises, None
            raise error
        if size is None or size < 0:
            piece, self.payload = self.payload, b""
        else:
            piece, self.payload = self.payload[:size], self.payload[size:]
        return piece


def status(code):
    """The error a server's answer of `code` raises, as urlopen raises it."""
    return urllib.error.HTTPError(URL, code, f"answer {code}", {}, io.BytesIO(b""))


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
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script, ran = self.installer(root)
            with mock.patch.object(install, "GROK_INSTALLER", script.as_uri()), \
                    mock.patch.object(install.urllib.request, "urlopen", side_effect=status(403)):
                with self.assertRaises(SystemExit) as stopped:
                    install.grok({"version": "1.0.52"}, root)
            self.assertIn("403", str(stopped.exception))
            self.assertFalse(ran.exists(), "the step ran the installer after its download failed")


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

    def fetch(self, answers, budget=None, clock=None, asked=None):
        """What `fetch(URL)` returned and the clock it ran on, against a server
        answering `answers` in turn: bytes for a whole body, an `Answer` for a
        body with its own timing or failure, an exception for a refused ask.
        Every answer reads on the same clock `fetch` itself runs on, and a
        caller that expects the download to fail passes its own clock and
        `asked` list, since nothing is returned to read the waits from."""
        clock = clock or Clock()
        answered = iter(answers)

        def urlopen(url, timeout=None):
            if asked is not None:
                asked.append(url)
            answer = next(answered)
            if isinstance(answer, BaseException):
                raise answer
            if not isinstance(answer, Answer):
                answer = Answer(answer)
            answer.clock = clock
            return answer

        patches = [mock.patch.object(install.time, "monotonic", clock.monotonic),
                   mock.patch.object(install.time, "sleep", clock.sleep),
                   mock.patch.object(install.urllib.request, "urlopen", urlopen)]
        if budget is not None:
            patches.append(mock.patch.object(install, "DOWNLOAD_BUDGET_SECONDS", budget))
        with contextlib.ExitStack() as stack:
            for patch in patches:
                stack.enter_context(patch)
            return install.fetch(URL), clock

    def test_a_download_that_answers_after_transient_failures_returns_the_bytes(self):
        for answer in (status(429), status(503), ConnectionResetError("connection reset"),
                       TimeoutError("timed out"), http.client.RemoteDisconnected("no answer"),
                       urllib.error.URLError(TimeoutError("timed out")),
                       urllib.error.URLError(ConnectionRefusedError("connection refused"))):
            with self.subTest(answer=answer):
                payload, clock = self.fetch([answer, answer, b"the file"])
                self.assertEqual(payload, b"the file")
                self.assertEqual(clock.slept, list(install.DOWNLOAD_RETRY_DELAYS[:2]))

    def test_a_permanent_refusal_is_not_asked_again(self):
        # A 403, a 404, a certificate of another name, a scheme urlopen does not
        # know and a name that does not resolve answer the same on every ask, so
        # the server is asked once and the step ends with that cause.
        for answer in (status(403), status(404),
                       urllib.error.URLError(ssl.SSLCertVerificationError("hostname mismatch")),
                       urllib.error.URLError(ValueError("unknown url type: htp")),
                       urllib.error.URLError(socket.gaierror("nodename nor servname provided"))):
            with self.subTest(answer=answer):
                asked, clock = [], Clock()
                with self.assertRaises(SystemExit) as stopped:
                    self.fetch([answer, b"the file"], clock=clock, asked=asked)
                self.assertEqual(asked, [URL], "a permanent refusal asked the server again")
                self.assertEqual(clock.slept, [])
                self.assertIn("could not be downloaded", str(stopped.exception))
                self.assertIn(str(getattr(answer, "reason", answer)), str(stopped.exception))

    def test_a_download_that_keeps_failing_stops_on_its_own_error(self):
        attempts = len(install.DOWNLOAD_RETRY_DELAYS) + 1
        for answer in (status(500), ConnectionResetError("connection reset")):
            with self.subTest(answer=answer):
                with self.assertRaises(SystemExit) as stopped:
                    self.fetch([answer] * attempts)
                self.assertIn(f"{attempts} attempts", str(stopped.exception))
                self.assertIn(str(answer), str(stopped.exception))

    def test_a_file_that_does_not_match_its_pin_is_refused_without_asking_again(self):
        row = {"version": "1.4.8", "installer": "agy",
               "files": {install.target(): {"url": URL, "sha512": "0" * 128}}}
        asked = []
        with tempfile.TemporaryDirectory() as temporary, \
                mock.patch.object(install.urllib.request, "urlopen",
                                  side_effect=lambda url, timeout: asked.append(url) or Answer(b"another release")):
            with self.assertRaisesRegex(SystemExit, "does not match the sha512"):
                install.agy(row, Path(temporary))
        self.assertEqual(asked, [URL], "a hash mismatch asked the server for the file again")

    def test_a_body_that_ends_partway_is_asked_again_unless_its_answer_is_final(self):
        # A connection that dropped mid-body is transient; a 403 once the body
        # started is the server's answer, and a 503 is worth asking again.
        for error in (http.client.IncompleteRead(b"half", 10), http.client.RemoteDisconnected("gone"),
                      status(503)):
            with self.subTest(error=error):
                payload, clock = self.fetch([Answer(b"half", raises=error), b"the file"])
                self.assertEqual(payload, b"the file")
                self.assertEqual(clock.slept, [install.DOWNLOAD_RETRY_DELAYS[0]])
        clock = Clock()
        with self.assertRaisesRegex(SystemExit, "could not be downloaded: .*403"):
            self.fetch([Answer(b"half", raises=status(403)), b"the file"], clock=clock)
        self.assertEqual(clock.slept, [], "a 403 in the body asked the server again")

    def test_a_body_still_arriving_at_the_budget_fails_with_the_budget_as_the_reason(self):
        # A server that keeps trickling bytes: each piece arrives well inside
        # the request's own wait, and there is far more than the budget at the
        # rate it sends them, so only the deadline stops it. The body is a
        # single chunk's worth read a byte at a time, which is what a `read`
        # that waits to fill its chunk would hide.
        trickle = Answer(b"x" * 4096, arriving=1,
                         seconds=install.DOWNLOAD_BUDGET_SECONDS / 1024)
        with self.assertRaisesRegex(SystemExit, f"inside {install.DOWNLOAD_BUDGET_SECONDS} s"):
            self.fetch([trickle])
        # A transfer that finishes inside the budget does finish.
        payload, clock = self.fetch([Answer(b"the file", seconds=1.0)])
        self.assertEqual(payload, b"the file")
        self.assertLess(clock.now, 100.0 + install.DOWNLOAD_BUDGET_SECONDS)

    def test_a_download_stops_when_the_next_wait_would_pass_its_budget(self):
        # The budget bounds the waits: a wait that ends at it leaves no time
        # for the attempt after it, and a second more lets that one start.
        fits = sum(install.DOWNLOAD_RETRY_DELAYS)
        for budget, attempts in ((fits - 1, 3), (fits, 3), (fits + 1, 4)):
            with self.subTest(budget=budget):
                with self.assertRaisesRegex(SystemExit, f"{attempts} attempts"):
                    self.fetch([status(503)] * 5, budget=budget)

    def test_a_request_with_no_time_left_is_never_sent(self):
        # The budget bounds the whole download, so a download that has none is
        # refused without asking the server, and the budget is its reason.
        asked = []
        with self.assertRaisesRegex(SystemExit, "inside 0 s"):
            self.fetch([b"the file"], budget=0, asked=asked)
        self.assertEqual(asked, [], "a download with no budget left asked the server")

    def test_another_version_than_the_pin_fails_the_install(self):
        with self.assertRaises(SystemExit) as stopped:
            self.install_pi("9.9.9")
        self.assertIn("pi answered '9.9.9', agent-clis.json pins", str(stopped.exception))

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
