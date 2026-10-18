"""Bootstrap refusal regressions from the journey's setup contract.

The external pane transport executes real shell commands against a finite
CLI stand-in. No server, installed agent or operator configuration is used.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.runtime import Runtime
from agent_live_check.system import PaneShell, READY, WINDOWS, remove_pane_files, tool_path
from agent_live_check.processes import MAX_OUTPUT, ProcessError
from agent_journey import cells, report


class Clock:
    def __init__(self):
        self.now = 100.0
        self.stopped = False

    def is_set(self):
        return self.stopped

    def wait(self, seconds):
        self.now += seconds
        return self.stopped


class PaneTransport(Runtime):
    def __init__(self, root, replies, *, elapsed=1, claim=None):
        self.clock = Clock()
        self.owner = SimpleNamespace(deadline=200, cancelled=self.clock)
        self.run = self.probe = root
        self.state = root / "state"
        self.bin = root
        self.shell = PaneShell()
        self.workspaces, self.pane_credentials = set(), {}
        self.sandbox = None
        self.checkout_directories = {root / "controller-journey"}
        self.elapsed = elapsed
        self.reference = self.state / "pane-capabilities/reference.json"
        self.reference.parent.mkdir(parents=True)
        self.fixture = root / "cli.py"
        config = {"replies": replies, "reference": str(self.reference),
                  "claim": claim or {"code": 2, "out": {"ok": False, "reason": "renderer_unavailable"}}}
        (root / "replies.json").write_text(json.dumps(config))
        self.fixture.write_text('''import json, pathlib, sys
root = pathlib.Path(__file__).parent
config = json.loads((root / "replies.json").read_text())
if sys.argv[-1] == "bootstrap":
    count = root / "attempts"
    n = int(count.read_text()) if count.exists() else 0
    count.write_text(str(n + 1))
    answer = config["replies"][min(n, len(config["replies"]) - 1)]
else:
    pathlib.Path(config["reference"]).with_suffix(".claimed").touch()
    answer = config["claim"]
out = answer.get("out", "")
if out == "accepted":
    out = {"ok": True, "reference": config["reference"]}
sys.stdout.write(json.dumps(out) if isinstance(out, (dict, list)) else out)
sys.stderr.write(answer.get("err", ""))
sys.exit(answer["code"])
''')
        self.hide = self.fixture

    def command(self, args, **kwargs):
        if args[:2] == ["workspace", "create"]:
            return 0, json.dumps({"result": {"workspace": {"workspace_id": "w1"},
                                             "root_pane": {"pane_id": "w1:p1"}}}), ""
        if args[:2] == ["pane", "read"]:
            return 0, READY, ""
        if args[:2] == ["pane", "run"]:
            # Only the external CLI executable is substituted. Production
            # quoting, redirection, status, parsing and retry all run intact.
            import shlex
            line = args[-1]
            if WINDOWS:
                line = line.replace('"' + str(self.fixture) + '"',
                                    '"' + sys.executable + '" "' + str(self.fixture) + '"')
                command = f'"{os.environ["ComSpec"]}" /d /s /c "{line}"'
            else:
                line = line.replace(shlex.quote(str(self.fixture)),
                                    shlex.quote(sys.executable) + " " + shlex.quote(str(self.fixture)))
                command = ["/bin/zsh", "-f", "-c", line]
            ran = subprocess.run(command, capture_output=True, timeout=30)
            self.clock.now += self.elapsed
            return ran.returncode, ran.stdout.decode(), ran.stderr.decode()
        raise AssertionError(args)


class Bootstrap(unittest.TestCase):
    def run_workspace(self, root, replies, **options):
        runtime = PaneTransport(root, replies, **options)
        result = {"mode": "fixture", "herdr": {}, "table": {}, "systems": {}, "agents": [], "failures": []}
        with patch("time.monotonic", lambda: runtime.clock.now):
            try:
                runtime.new_workspace({"id": "controller"}, "journey", root, root / "wrapper", {"env": {}})
            except Exception as error:
                result["failures"].append({"type": type(error).__name__, "reason": str(error)})
        report.save(root, result)
        return runtime, json.loads((root / "report.json").read_text())

    def test_refused_then_accepted_reaches_the_claim_without_a_setup_failure(self):
        for reason in ("pane_not_connected", "caller_unavailable"):
            with self.subTest(reason=reason), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                runtime, saved = self.run_workspace(root, [{"code": 2, "err": reason},
                                                           {"code": 0, "out": "accepted"}])
                self.assertEqual(saved["failures"], [])
                self.assertEqual(runtime.pane_credentials, {"w1:p1": runtime.reference})
                self.assertEqual(list(root.glob("*.out")) + list(root.glob("*.err")), [])
                measurement = json.loads((root / "controller-journey-bootstrap.json").read_text())
                self.assertTrue(measurement["accepted"])
                self.assertEqual(measurement["last_refusal"], reason)
                self.assertNotIn(str(runtime.reference), json.dumps(measurement))

    def test_a_refusal_that_never_clears_reports_the_last_reason_at_the_deadline(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            _, saved = self.run_workspace(root, [{"code": 2, "err": "pane_not_connected"}], elapsed=8)
            reason = saved["failures"][0]["reason"]
            self.assertIn("private_checkout_not_registered: pane_not_connected", reason)
            self.assertIn("deadline", reason)
            self.assertIn(reason, (root / "report.md").read_text())
            self.assertEqual(cells.exit_code({}, [], setup_failed=True), 2)
            self.assertEqual(list(root.glob("*.out")) + list(root.glob("*.err")), [])

    def test_permanent_refusals_and_unreadable_successes_fail_without_retrying(self):
        cases = [({"code": 2, "err": "checkout_not_registered"}, "checkout_not_registered"),
                 ({"code": 0, "out": "not json"}, "unreadable_answer"),
                 ({"code": 0, "out": []}, "unreadable_answer"),
                 ({"code": 2, "out": "accepted", "err": "invalid_reference"}, "invalid_reference")]
        for answer, reason in cases:
            with self.subTest(answer=answer), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                runtime, saved = self.run_workspace(root, [answer, {"code": 0, "out": "accepted"}])
                self.assertIn(reason, saved["failures"][0]["reason"])
                self.assertEqual((root / "attempts").read_text(), "1")
                self.assertEqual(runtime.pane_credentials, {})
                self.assertEqual(list(root.glob("*.out")) + list(root.glob("*.err")), [])

    def test_claim_stderr_is_reported_and_nonzero_success_is_never_accepted(self):
        for claim in ({"code": 2, "err": "credential_expired"}, {"code": 2, "out": {"ok": True}}):
            with self.subTest(claim=claim), tempfile.TemporaryDirectory() as folder:
                _, saved = self.run_workspace(Path(folder), [{"code": 0, "out": "accepted"}], claim=claim)
                self.assertIn("private_pane_reference_not_claimed", saved["failures"][0]["reason"])
                self.assertNotEqual(saved["failures"][0]["type"], "JSONDecodeError")

    def test_both_streams_are_bounded_and_removed_even_on_failure(self):
        for stream in ("out", "err"):
            with self.subTest(stream=stream), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                _, saved = self.run_workspace(root, [{"code": 2, stream: "x" * (MAX_OUTPUT + 1)}])
                self.assertIn("pane_command_output_over_budget", saved["failures"][0]["reason"])
                self.assertEqual(list(root.glob("*.out")) + list(root.glob("*.err")), [])

    def test_more_than_twenty_quick_refusals_can_connect_within_the_time_budget(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            _, saved = self.run_workspace(root, [{"code": 2, "err": "pane_not_connected"}] * 21
                                          + [{"code": 0, "out": "accepted"}], elapsed=0.01)
            self.assertEqual(saved["failures"], [])


class PaneFileRelease(unittest.TestCase):
    def test_only_windows_handle_conflicts_are_retried_with_a_bounded_wait(self):
        # The OS unlink boundary answers the Windows errors. Virtual time
        # fixes handle-release order without a flaky scheduler delay.
        for code, clears in ((32, True), (33, True), (32, False), (5, False)):
            with self.subTest(code=code, clears=clears):
                clock = Clock()
                error = PermissionError("OS fixture")
                error.winerror = code
                class File:
                    removed = False

                    def unlink(self, **_):
                        if clears and clock.now > 100:
                            self.removed = True
                        else:
                            raise error
                file = File()
                with patch("time.monotonic", lambda: clock.now), patch("time.sleep", clock.wait):
                    if clears:
                        remove_pane_files(file)
                        self.assertTrue(file.removed)
                    elif code == 5:
                        with self.assertRaises(PermissionError):
                            remove_pane_files(file)
                        self.assertEqual(clock.now, 100)
                    else:
                        with self.assertRaisesRegex(ProcessError, "pane_file_release_timeout"):
                            remove_pane_files(file)
                        self.assertEqual(clock.now, 105)


@unittest.skipIf(WINDOWS, "Windows panes use cmd.exe and the private user PATH")
class PaneShellPath(unittest.TestCase):
    def test_login_and_nonlogin_shells_run_the_private_program_before_system_programs(self):
        # macOS's /etc/zprofile runs path_helper, which places /usr/bin and
        # /usr/local/bin before an inherited private PATH. No system file or
        # installed CLI is changed: uname is a finite executable stand-in.
        with tempfile.TemporaryDirectory(prefix="journey shell '") as folder:
            home = Path(folder)
            private_bin = home / "bin"
            private_bin.mkdir()
            program = private_bin / "uname"
            program.write_text("#!/bin/sh\nprintf 'private-program\\n'\n")
            program.chmod(0o700)
            shell = PaneShell()
            path = tool_path(private_bin)
            for name, data in shell.files():
                (home / name).write_bytes(data)
            controller_bin = home / "controller/bin"
            controller_bin.mkdir(parents=True)
            controller = controller_bin / "uname"
            controller.write_text("#!/bin/sh\nprintf 'controller-program\\n'\n")
            controller.chmod(0o700)
            # A controller pane uses the same private shell files but its
            # own stand-in first, ahead of the measured native wrappers.
            for pane_path, expected in ((path, "private-program"),
                                        (os.pathsep.join((str(controller_bin), path)), "controller-program")):
                environment = {"HOME": str(home), "PATH": pane_path, "LC_ALL": "C", **shell.environment(home)}
                for flags in ("-ic", "-ilc"):
                    with self.subTest(flags=flags, program=expected):
                        ran = subprocess.run(["/bin/zsh", flags, "uname"], env=environment,
                                             capture_output=True, text=True, timeout=5)
                        self.assertEqual(ran.returncode, 0, ran.stderr)
                        self.assertEqual(ran.stdout.strip(), expected)


if __name__ == "__main__":
    unittest.main()
