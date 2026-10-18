"""Expired external replies cannot qualify a bounded observation or new input.

Root2952 fixes each phase at the command transport cap plus its observation
window. Replies beyond that absolute end cannot certify a result or input.
Herdr responses and the clock are the only
substituted boundaries; the real screen, identity and input adapters run.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from agent_live_check.delivery import measure
from agent_live_check.history import LABEL, seed
from agent_live_check.runtime import Runtime
from agent_live_check.scenes import observe
from agent_live_check.processes import COMMAND_SECONDS, OwnedProcesses, ProcessError, ProcessSafetyError
from agent_live_check.timing import Deadline


class HerdrReplies(Runtime):
    """Finite external CLI replies, without a Herdr server or provider."""

    def __init__(self, root, *, late=None, late_seconds=COMMAND_SECONDS + 1.1,
                 effect="NO MATCH", history=False, status="idle"):
        self.clock = [100.0]
        self.owner = type("Owner", (), {"deadline": 500, "cancelled": threading.Event()})()
        self.fixture_bin = None
        self.root, self.late, self.effect, self.history = root, late, effect, history
        self.late_seconds = late_seconds
        self.status = status
        self.counts, self.inputs, self.budgets, self.ends = {}, [], [], []
        self.session = {"kind": "path", "source": "herdr:claude", "value": str(root / "native.jsonl")}
        self.after_reply = lambda kind, ordinal: None

    def command(self, args, *, seconds=15, **kwargs):
        kind = "screen" if args[:2] == ["pane", "read"] else "agent" if args == ["agent", "list"] else "input"
        ordinal = self.counts.get(kind, 0) + 1
        self.counts[kind] = ordinal
        self.budgets.append(seconds)
        self.ends.append(kwargs.get("deadline"))
        if kind == "input":
            self.inputs.append(args[-1])
        self.clock[0] += self.late_seconds if (kind, ordinal) == self.late else 0.025
        if self.history:
            status = ["idle", "working", "done"][min(self.counts.get("agent", 0), 2)]
            screen = LABEL + "\nREADY" if self.counts.get("agent", 0) >= 2 else "READY"
        else:
            status = self.status
            screen = self.effect if ordinal >= 3 else "READY"
        self.after_reply(kind, ordinal)
        if kind == "screen":
            return 0, screen, ""
        if kind == "agent":
            if self.history:
                status = ["idle", "working", "done"][min(ordinal - 1, 2)]
            agent = {"pane_id": "owned", "agent_status": status, "agent_session": self.session}
            return 0, json.dumps({"result": {"agents": [agent]}}), ""
        return 0, "", ""

    def send_letter(self, pane, intent, body):
        self.marker = body.rsplit(" ", 1)[-1]
        return "private-letter"

    def bell_line(self, pane, letter_id, *, seconds=15, deadline=None):
        # hided rings on the second look, with the line it kept.
        self.budgets.append(seconds)
        return "bell" if self.counts.get("agent", 0) >= 2 else None


class ObservationDeadlines(unittest.TestCase):
    def test_preemption_before_real_transport_admission_cannot_start_expired_input(self):
        clock, emitted = [100.0], []
        owner = OwnedProcesses()
        owner.deadline = 500
        # No actual command is launched. An external spawn attempt is itself
        # the observable failure, rather than a mocked owned-process helper.
        owner.family = "finite-admission-fixture"
        runtime = Runtime.__new__(Runtime)
        runtime.owner, runtime.herdr_bin, runtime.env = owner, Path("/external/herdr"), {}
        def launch(*args, **kwargs):
            emitted.append(args)
            raise AssertionError("expired pane input reached OS spawn")
        with patch("time.monotonic", lambda: clock[0]), patch("subprocess.Popen", launch):
            phase = Deadline(owner, 1)
            seconds = phase.command_seconds()
            clock[0] = 101.1  # scheduling delay after budget calculation
            with self.assertRaisesRegex(ProcessError, "command_timeout"):
                runtime.command(["pane", "run", "owned", "bell"], seconds=seconds, deadline=phase.end)
        self.assertEqual(emitted, [])
        self.assertEqual(owner.children, {})

    def recipe(self, kind="claude", unsafe="SELECTED"):
        scene = {"send": "", "arrived": "READY", "draft": "DRAFT", "no_match": "NO MATCH", "unsafe": unsafe}
        return {"kind": kind, "scenes": {"model_picker": scene, "rest": scene}}

    def observe(self, root, runtime, seconds=1):
        evidence = root / ("scene-" + str(len(list(root.glob("scene-*.json")))) + ".json")
        with patch("time.monotonic", lambda: runtime.clock[0]):
            result = observe(runtime, "owned", self.recipe(), "model_picker", "bell", root,
                             seconds, evidence, root, {"session_root": root, "settings": []})
        return result, json.loads(evidence.read_text())

    def test_late_arrival_or_identity_retains_frame_and_never_types(self):
        for late in (("screen", 1), ("agent", 1)):
            with self.subTest(late=late), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=late, status="blocked")
                result, record = self.observe(root, runtime)
                self.assertEqual((result["arrival"], result["effect"]), ("timeout", "not_tested"))
                self.assertEqual(record["samples"][-1]["screen"], "READY")
                self.assertEqual(runtime.inputs, [])

    def test_input_authentication_read_cannot_renew_arrival_deadline(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            runtime = HerdrReplies(root, late=("screen", 2))
            result, _ = self.observe(root, runtime)
            self.assertEqual(result["reason"], "arrival_deadline")
            self.assertEqual(runtime.inputs, [])

    def test_late_effect_read_cannot_certify_no_match(self):
        for late in (("screen", 3), ("agent", 2)):
            with self.subTest(late=late), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=late)
                result, record = self.observe(root, runtime)
                self.assertEqual((result["arrival"], result["effect"]), ("reached", "ambiguous"))
                self.assertEqual(result["reason"], "effect_deadline")
                self.assertEqual(record["samples"][-1]["screen"], "NO MATCH")
                self.assertEqual(runtime.inputs, ["bell"])

    def test_native_bell_turn_must_be_observed_before_effect_deadline(self):
        for late in (("screen", 3), ("agent", 2), None):
            with self.subTest(late=late), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=late, effect="ANSWER")
                file = root / "native.jsonl"
                file.write_text("")
                def reply(kind, ordinal):
                    if kind == "screen" and ordinal == 3:
                        rows = [{"type": role, "message": {"role": role, "content": text}}
                                for role, text in (("user", "bell"), ("assistant", "ANSWER"))]
                        file.write_text("\n".join(json.dumps(row) for row in rows))
                runtime.after_reply = reply
                result, record = self.observe(root, runtime)
                self.assertEqual((result["arrival"], result["effect"]),
                                 ("reached", "ambiguous" if late else "new_turn"))
                self.assertEqual(record["samples"][-1]["screen"], "ANSWER")
                self.assertEqual(runtime.inputs, ["bell"])

    def test_effect_phase_timeout_preserves_arrival_without_hiding_fatal_or_early_failures(self):
        cases = [
            (("screen", 3), ProcessError("command_timeout"), 16.1, False, 500, True),
            (("agent", 2), ProcessError("command_timeout"), 16.1, False, 500, True),
            (("screen", 1), ProcessError("command_timeout"), 16.1, False, 500, False),
            (("screen", 3), ProcessSafetyError("command_timeout"), 16.1, False, 500, False),
            (("screen", 3), ProcessError("command_output_over_budget"), 16.1, False, 500, False),
            (("screen", 3), ProcessError("run_cancelled"), 16.1, False, 500, False),
            (("screen", 3), ProcessError("command_timeout"), .1, False, 500, False),
            (("screen", 3), ProcessError("command_timeout"), 16.1, True, 500, False),
            (("screen", 3), ProcessError("command_timeout"), 16.1, False, 110, False),
        ]
        for failed, error, delay, cancel, owner_end, observed in cases:
            with self.subTest(failed=failed, error=repr(error), delay=delay, cancel=cancel, owner_end=owner_end):
                with tempfile.TemporaryDirectory() as name:
                    root = Path(name)
                    runtime = HerdrReplies(root, late=failed, late_seconds=delay, effect="SELECTED")
                    runtime.owner.deadline = owner_end
                    def failure(kind, ordinal):
                        if (kind, ordinal) == failed:
                            if cancel:
                                runtime.owner.cancelled.set()
                            raise error
                    runtime.after_reply = failure
                    if observed:
                        result, record = self.observe(root, runtime)
                        self.assertEqual((result["arrival"], result["effect"], result["reason"]),
                                         ("reached", "ambiguous", "effect_deadline"))
                        self.assertEqual(record["observation"], result)
                    else:
                        with self.assertRaises(type(error)) as raised:
                            self.observe(root, runtime)
                        self.assertIs(raised.exception, error)

    def test_timely_safe_and_unsafe_evidence_keep_separate_phase_windows(self):
        for effect, expected in (("NO MATCH", "no_match"), ("SELECTED", "selection")):
            with self.subTest(effect=effect), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=("screen", 1), late_seconds=1.1, effect=effect)
                result, _ = self.observe(root, runtime)
                self.assertEqual((result["arrival"], result["effect"]), ("reached", expected))
                self.assertEqual(runtime.inputs, ["bell"])
                self.assertTrue(all(0 < budget <= COMMAND_SECONDS for budget in runtime.budgets))
                self.assertTrue(all(end is not None for end in runtime.ends))

    def test_large_window_keeps_command_cap_and_global_owner_deadline(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            runtime = HerdrReplies(root)
            self.assertEqual(self.observe(root, runtime, 120)[0]["effect"], "no_match")
            self.assertTrue(all(value == 15 for value in runtime.budgets))
            runtime = HerdrReplies(root, late=("agent", 1))
            runtime.owner.deadline = 100.5
            self.assertEqual(self.observe(root, runtime, 120)[0]["arrival"], "timeout")
            self.assertTrue(all(0 < value <= .5 for value in runtime.budgets))

    def test_a_command_asked_to_run_past_the_cap_is_refused_before_it_starts(self):
        owner = OwnedProcesses()
        self.addCleanup(owner.close)
        with self.assertRaisesRegex(ValueError, f"at most {COMMAND_SECONDS} s"):
            owner.run([sys.executable, "-c", "pass"], env=dict(os.environ), seconds=COMMAND_SECONDS + 1)
        self.assertEqual(owner.children, {})

    def test_seed_cannot_submit_or_certify_after_late_native_replies(self):
        for late, input_count in ((("screen", 1), 0), (("agent", 1), 0),
                                  (("screen", 2), 0), (("agent", 3), 1), (None, 1)):
            with self.subTest(late=late), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=late, history=True)
                runtime.session = {"kind": "id", "source": "herdr:grok", "value": "native-history"}
                with patch("time.monotonic", lambda: runtime.clock[0]):
                    result = seed(runtime, "owned", self.recipe("grok"), root, {"session_root": root}, 1)
                self.assertEqual(result["status"], "unknown" if late else "native_prompt_completed")
                self.assertEqual(len(runtime.inputs), input_count)

    def test_delivery_rejects_late_marker_and_cannot_type_after_auth_read_expires(self):
        for late in (("screen", 2), ("agent", 2), ("screen", 3), None):
            with self.subTest(late=late), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                runtime = HerdrReplies(root, late=late)
                file = root / "native.jsonl"
                file.write_text("")
                def reply(kind, ordinal):
                    if kind == "agent" and ordinal == 2 and late != ("screen", 3):
                        rows = [{"type": role, "message": {"role": role, "content": text}}
                                for role, text in (("user", "bell"), ("assistant", runtime.marker))]
                        file.write_text("\n".join(json.dumps(row) for row in rows))
                runtime.after_reply = reply
                with patch("time.monotonic", lambda: runtime.clock[0]):
                    result = measure(runtime, "owned", self.recipe(), root, root, 1, 0,
                                     {"session_root": root})
                self.assertEqual(result["outcome"], "unknown" if late else "verified")
                self.assertEqual(runtime.inputs, [])



@unittest.skipIf(sys.platform == "win32", "a Windows run's children end with its job, not a guardian")
class RunDeadline(unittest.TestCase):
    def test_a_child_started_first_ends_with_the_owners_run_not_its_own_clock(self):
        with self.assertRaisesRegex(ProcessError, "cleanup_unconfirmed"):
            with OwnedProcesses(run_seconds=3) as owner:
                child = owner.spawn([sys.executable, "-c", "import time; time.sleep(60)"],
                                    env={"PATH": "/usr/bin:/bin"}, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL)
                started = time.monotonic()
                # The guardian ends its child at the owner's deadline and says so.
                child.wait(timeout=20)
                self.assertLess(time.monotonic() - started, 15)


class Teardown(unittest.TestCase):
    def test_a_run_past_its_deadline_and_cancelled_still_runs_its_stop(self):
        # Windows run 39104633599: the deadline refused `opencode service stop` and the server stayed.
        with OwnedProcesses(run_seconds=0) as owner:
            owner.cancelled.set()
            stop = [sys.executable, "-c", "print('stopped')"]
            with self.assertRaisesRegex(ProcessError, "run_cancelled_or_timed_out"):
                owner.run(stop, env=dict(os.environ))
            with owner.teardown():
                self.assertEqual(owner.run(stop, env=dict(os.environ))[:2], (0, "stopped\n"))
            self.assertTrue(owner.cancelled.is_set())
            with self.assertRaisesRegex(ProcessError, "run_cancelled_or_timed_out"):
                owner.run(stop, env=dict(os.environ))


if __name__ == "__main__":
    unittest.main()
