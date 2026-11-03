"""Keys measurement admission at real process/file boundaries, without a browser.

The delivery executable stands in for CDP/PTY transport, not the printer or OS
identity implementation. The enclosing fixture owns everything it delivers.
Set TMPDIR to the run evidence folder to retain receipts and child output.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from holder_owned import Owned, runs, started  # noqa: E402

HELPER = SCRIPTS / "web-shell-measure/keys_printers.py"
PANES = [f"pane-{index}" for index in range(5)]

# An external command transport: execute the received shell line and return
# while the pane program continues. Invalid modes corrupt external receipts;
# they never replace the printer's implementation or the OS identity reader.
TRANSPORT = r'''
import json, os, signal, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from holder_owned import Owned, started
root, mode, pane, focus, interrupt, command = sys.argv[2:]
assert (focus, interrupt) == ("--focus", "--interrupt")
root = Path(root)
index = int(pane.split("-")[-1])
phase = root / "phase"
if mode == "blocked" and index == 4:
    (root / "blocked.json").write_text(json.dumps({"pid": os.getpid(), "birth": started(os.getpid())}))
    signal.pause()
output = (root / (pane + ".out")).open("wb")
owner = Owned()
child = owner.spawn(["sh", "-c", command], stdout=output, stderr=output)
(root / (pane + ".process.json")).write_text(json.dumps({"pid": child.pid, "birth": started(child.pid)}))
receipt = phase / (str(index) + ".json")
deadline = time.monotonic() + 5
while not receipt.exists():
    if child.poll() is not None or time.monotonic() >= deadline:
        raise RuntimeError("transport did not observe first flush")
    time.sleep(0.01)
row = json.loads(receipt.read_text())
(root / (pane + ".printer.json")).write_text(json.dumps(row))
if mode == "blocked" and index == 0:
    # Deliberate external delivery stall, part of the shared startup budget.
    # The fifth delivery never returns: a per-command budget would take 40s.
    import threading
    threading.Event().wait(10)
if index == 1 and mode in ("stale", "duplicate", "birth", "missing-birth"):
    if mode == "stale": row["nonce"] = "previous-run"
    elif mode == "duplicate":
        first = json.loads((phase / "0.json").read_text())
        row.update(pid=first["pid"], birth=first["birth"])
    elif mode == "birth": row["birth"] += 1
    else: row["birth"] = None
    receipt.write_text(json.dumps(row))
'''


@unittest.skipUnless(os.name == "posix", "the shell measurement uses a POSIX host")
class PrinterBoundary(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="keys-printers-"))
        self.phase = self.root / "phase"
        self.transport = self.root / "transport.py"
        self.transport.write_text(TRANSPORT)
        self.owned = Owned()
        self.addCleanup(self.cleanup)

    def cleanup(self):
        # Original delivery records, never the receipts a negative case corrupts.
        for pattern in ("*.process.json", "*.printer.json", "blocked.json"):
            for path in self.root.glob(pattern):
                row = json.loads(path.read_text())
                if type(row["birth"]) is int:
                    self.owned.adopt(row["pid"], row["birth"])
        left = self.owned.close()
        print(json.dumps({"fixture": str(self.root), "cleanup_left": left}), flush=True)
        self.assertEqual(left, [])

    def command(self, *args, timeout=40):
        child = self.owned.spawn([sys.executable, str(HELPER), *map(str, args)],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        out, err = child.communicate(timeout=timeout)
        with (self.root / "commands.jsonl").open("a") as stream:
            stream.write(json.dumps({"args": list(map(str, args)), "exit": child.returncode,
                                     "stdout": out, "stderr": err}) + "\n")
        return child.returncode, out, err

    def launch(self, mode="normal"):
        return self.command("start", self.phase, 120, *PANES, sys.executable,
                            self.transport, SCRIPTS, self.root, mode)

    def end_printer(self, index):
        row = json.loads((self.root / f"pane-{index}.printer.json").read_text())
        self.assertTrue(runs(row["pid"], row["birth"]))
        os.kill(row["pid"], signal.SIGTERM)
        deadline = time.monotonic() + 5
        while runs(row["pid"], row["birth"]):
            if time.monotonic() >= deadline:
                self.fail("printer did not exit")
            time.sleep(0.01)

    def test_five_first_flushes_and_four_echo_survivors(self):
        self.assertEqual(self.launch()[0], 0)
        report = json.loads((self.phase / "start.json").read_text())
        self.assertTrue(report["complete"])
        self.assertEqual(len({row["pid"] for row in report["printers"]}), 5)
        for pane in PANES:
            self.assertIn(b"drive 00000", (self.root / f"{pane}.out").read_bytes())
        for boundary in ("resources-before", "resources-after"):
            self.assertEqual(self.command("check", self.phase, boundary)[0], 0)
        self.end_printer(4)  # The measured pane has stopped printing for cat.
        for boundary in ("echo-before", "echo-after"):
            self.assertEqual(self.command("check", self.phase, boundary, "--exclude", PANES[4])[0], 0)
            echo = json.loads((self.phase / f"{boundary}.json").read_text())
            self.assertEqual(len(echo["printers"]), 4)
            self.assertEqual(echo["cat_pane"], PANES[4])
        self.assertNotEqual(self.launch()[0], 0, "a phase must never be reused")

    def test_an_early_exit_fails_the_resource_end(self):
        self.assertEqual(self.launch()[0], 0)
        self.assertEqual(self.command("check", self.phase, "resources-before")[0], 0)
        self.end_printer(0)
        self.assertNotEqual(self.command("check", self.phase, "resources-after")[0], 0)
        self.assertFalse(json.loads((self.phase / "resources-after.json").read_text())["complete"])

    def reject_receipt(self, mode):
        self.assertNotEqual(self.launch(mode)[0], 0)
        self.assertFalse(json.loads((self.phase / "start.json").read_text())["complete"])

    def test_a_previous_nonce_cannot_admit_a_phase(self):
        self.reject_receipt("stale")

    def test_a_duplicate_pid_cannot_admit_a_phase(self):
        self.reject_receipt("duplicate")

    def test_a_wrong_birth_cannot_admit_a_phase(self):
        self.reject_receipt("birth")

    def test_an_unknown_birth_cannot_admit_a_phase(self):
        self.reject_receipt("missing-birth")

    def test_delivery_and_partial_readiness_share_one_thirty_second_budget(self):
        began = time.monotonic()
        code, _, _ = self.launch("blocked")
        elapsed = time.monotonic() - began
        self.assertNotEqual(code, 0)
        report = json.loads((self.phase / "start.json").read_text())
        self.assertFalse(report["complete"])
        self.assertEqual(len(report["printers"]), 4)
        self.assertGreaterEqual(report["startup_seconds"], 29)
        self.assertLess(report["startup_seconds"], 35)  # A new per-call 30s would take >=40s.
        self.assertLess(elapsed, 38)
        self.assertEqual(report["delivery_processes_left"], [])
        blocked = json.loads((self.root / "blocked.json").read_text())
        self.assertFalse(runs(blocked["pid"], blocked["birth"]))

    def test_observer_receipt_must_name_the_live_observer(self):
        path = self.root / "observer.json"
        program = "import signal; signal.pause()"
        child = self.owned.spawn([sys.executable, "-c", program])
        path.write_text(json.dumps({"pid": child.pid, "boundary": "Network.enable completed"}))
        self.assertEqual(self.command("observer", path, child.pid)[0], 0)
        path.write_text(json.dumps({"pid": child.pid + 1, "boundary": "Network.enable completed"}))
        self.assertNotEqual(self.command("observer", path, child.pid)[0], 0)
        child.terminate()
        child.wait(timeout=5)
        self.assertNotEqual(self.command("observer", path, child.pid)[0], 0)


if __name__ == "__main__":
    unittest.main()
