"""The holder checks end what they started on every path (scripts/holder_owned.py).

The restart lane and the holder measurement put each node, holder and
server they start in an `Owned`, and end it by pid and start time
however the run ends. These run the lane's own node client
(`holder-restart-check.py`'s `Node` and `clean`) against a stand-in node
that starts a stand-in holder as the real one does: in a session of its own,
outliving its node, with a program under it and a pane record naming both.
After a node restart that fails, and after a run stopped by SIGTERM, none of
them runs and the run's folder is gone.
"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))

import holder_owned  # noqa: E402

PREAMBLE = f"""\
import json, os, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, {str(SCRIPTS)!r})
import holder_owned
"""

# A holder: its own session, a program under it, and the record a real
# holder writes, with both start times as the process table reads them.
HOLDER = PREAMBLE + """
path = Path(sys.argv[1])
program = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(600)"])
identity = lambda pid: {"pid": pid, "started": holder_owned.started(pid)}
record = {"pane": path.stem, "holder": identity(os.getpid()), "program": identity(program.pid)}
written = path.with_suffix(".tmp")
written.write_text(json.dumps(record))
written.replace(path)
time.sleep(600)
"""

# A node: the request lines the lane speaks, and `start` starts a holder
# that outlives it.
NODE = PREAMBLE + """
holder = sys.argv[1]
args = sys.argv[2:]
state = Path(args[args.index("--state") + 1])
print(json.dumps({"ok": True, "ready": os.getpid()}), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    if request["op"] != "start":
        print(json.dumps({"ok": False, "error": "unknown op"}), flush=True)
        continue
    records = state / "holders/records"
    records.mkdir(parents=True, exist_ok=True)
    path = records / (request["pane"] + ".json")
    subprocess.Popen([sys.executable, holder, str(path)], start_new_session=True,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not path.exists():
        time.sleep(0.02)
    print(json.dumps({"ok": True, "started": "started", "record": json.loads(path.read_text())}), flush=True)
"""

# A node that never says it is ready: a restart that fails.
FAILING_NODE = "import sys; sys.exit(3)\n"


def lane():
    spec = importlib.util.spec_from_file_location("holder_restart_check", SCRIPTS / "holder-restart-check.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def program(folder, name, text, *args):
    """An executable that runs `text` with `args` before the node's own."""
    script = folder / f"{name}.py"
    script.write_text(text)
    launcher = folder / name
    launcher.write_text("#!/bin/sh\nexec " + " ".join(
        [sys.executable, str(script), *map(str, args)]) + ' "$@"\n')
    launcher.chmod(0o700)
    return launcher


@unittest.skipIf(os.name == "nt", "the lane's Windows node starts through Node.js; Linux runs these in CI")
class EndsWhatItStarted(unittest.TestCase):
    def setUp(self):
        self.tools = Path(tempfile.mkdtemp(prefix="hot-tools-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.tools)], check=False))
        holder = self.tools / "holder.py"
        holder.write_text(HOLDER)
        self.node = program(self.tools, "node", NODE, holder)
        self.failing = program(self.tools, "failing", FAILING_NODE)

    def assert_none_runs(self, identities):
        running = [identity for identity in identities if holder_owned.runs(identity["pid"], identity["started"])]
        # A failing run ends what it left, so the next test starts clean.
        for identity in running:
            holder_owned._end(identity["pid"], identity["started"])
        self.assertEqual(running, [], "left running")

    def test_a_node_restart_that_fails_leaves_nothing_running(self):
        module = lane()
        root = Path(tempfile.mkdtemp(prefix="hot-"))
        owned = holder_owned.Owned()
        owned.remove(root)
        state = root / "h/state"
        owned.records(state)
        node = module.Node(self.node, root / "h", state, None, root / "node.log", owned)
        record = node.ask("start", pane="p", program=[], cwd=str(root), env={}, rows=1, cols=1)["record"]
        first = {"pid": node.process.pid, "started": holder_owned.started(node.process.pid)}
        node.kill()
        with self.assertRaises(module.Failed):
            module.Node(self.failing, root / "h", state, None, root / "node.log", owned)
        self.assertTrue(holder_owned.runs(record["holder"]["pid"], record["holder"]["started"]),
                        "the holder should outlive its node, as a real one does")
        # The cell's own end, after the failed restart: the killed node is
        # all it still names.
        cleaned = module.clean(node, "p", None, owned)
        self.assertEqual(cleaned["left"], [])
        self.assert_none_runs([first, record["holder"], record["program"]])
        self.assertFalse(root.exists())

    def test_a_run_stopped_by_sigterm_leaves_nothing_running(self):
        root = Path(tempfile.mkdtemp(prefix="hot-"))
        driver = self.tools / "driver.py"
        driver.write_text(PREAMBLE + textwrap.dedent(f"""
            import importlib.util
            spec = importlib.util.spec_from_file_location("lane", {str(SCRIPTS / "holder-restart-check.py")!r})
            lane = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(lane)
            root = Path({str(root)!r})
            holder_owned.abort_on_signals()
            owned = holder_owned.Owned()
            owned.remove(root)
            owned.records(root / "h/state")
            node = None
            try:
                node = lane.Node({str(self.node)!r}, root / "h", root / "h/state", None, root / "node.log", owned)
                record = node.ask("start", pane="p", program=[], cwd=str(root), env={{}}, rows=1, cols=1)["record"]
                record["node"] = {{"pid": node.process.pid, "started": holder_owned.started(node.process.pid)}}
                print(json.dumps(record), flush=True)
                time.sleep(600)
            except holder_owned.Aborted as stop:
                print(json.dumps({{"stopped": stop.signum}}), flush=True)
                raise SystemExit(128 + stop.signum)
            finally:
                print(json.dumps({{"left": lane.clean(node, "p", None, owned)["left"]}}), flush=True)
        """))
        run = subprocess.Popen([sys.executable, str(driver)], stdout=subprocess.PIPE, text=True)
        try:
            record = json.loads(run.stdout.readline())
            run.send_signal(signal.SIGTERM)
            lines = [json.loads(line) for line in run.stdout]
            code = run.wait(60)
        finally:
            if run.poll() is None:
                run.kill()
                run.wait()
            run.stdout.close()
        self.assertEqual(code, 128 + signal.SIGTERM)
        self.assertEqual(lines, [{"stopped": signal.SIGTERM}, {"left": []}])
        self.assert_none_runs([record["node"], record["holder"], record["program"]])
        self.assertFalse(root.exists())

    def test_one_end_that_fails_does_not_keep_the_others_running(self):
        owned = holder_owned.Owned()
        sleepers = [owned.spawn([sys.executable, "-c", "import time; time.sleep(600)"]) for _ in range(2)]
        refused = sleepers[0].pid
        real_end = holder_owned._end

        def end(pid, birth):
            if pid == refused:
                raise OSError("refused")
            real_end(pid, birth)
        holder_owned._end = end
        try:
            left = owned.close(seconds=5)
        finally:
            holder_owned._end = real_end
        # The refused one was still ended as a child of this run's; the
        # refusal is answered, not swallowed.
        self.assertIn(f"{refused} could not be ended: refused", left)
        self.assertTrue(all(sleeper.poll() is not None for sleeper in sleepers))


if __name__ == "__main__":
    unittest.main()
