"""One agent's journey on the candidate: spawn, reply, letter, sleep and wake, resume.

Each step asks the product to act the way an operator or a parent agent does
(`hide agent spawn`, `hide request send`, the daemon's Sleep, Wake and terminal
input) and judges only what Hide then reports. A step that cannot run settles
its cell with the reason; nothing here types into a screen it read.
"""

import base64
import json
import os
from pathlib import Path
import secrets
import time

from agent_live_check.processes import ProcessError
from agent_live_check.protection import write_private
from agent_live_check.system import git

from . import cells, fork, kit, memory, title, usage
from .ci import redact
from .quota import model_requests, spent_quota

SPAWN_SECONDS = 150
TURN_SECONDS = 180
LETTER_SECONDS = 240
SLEEP_SECONDS = 90
WAKE_SECONDS = 150
ATTACH_SECONDS = 30
TURN_POLL_SECONDS = 1
# The grid the journey's one view of a pane draws at.
VIEW_COLS, VIEW_ROWS = 120, 40
# Refusals the product answers while the same spawn is still under way, each
# saying to ask again with the same intent.
RETRY = ("request_timeout", "spawn_busy", "native_identity_unavailable")
# The cells each measured by a module of its own, judging what Hide shows.
MEASURES = {module.CELL: module.measure for module in (kit, memory, fork, title, usage)}
# The steps of one agent's journey in order, each from the state the last
# left: the cells that only read what Hide shows come where that state is,
# the fork before Sleep so its source still runs the session it copies.
STEPS = ("start", "integration", "guidance_memory", "letter", "fork", "sleep_wake", "resume", "title", "usage")


def created_branch(runtime, repository, branch, worktree):
    """What a spawn made in the measured repository: the branch, the commit it
    started at, and its worktree; the cleanup deletes only what still matches."""
    code, out, _ = runtime.owner.run([git(), "-C", str(repository), "rev-parse", "--verify", "-q",
                                      "refs/heads/" + branch], env=git_env(runtime), check=False)
    return {"branch": branch, "start": out.strip() if not code else None, "worktree": str(worktree)}


def git_env(runtime):
    return {**runtime.env, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}


def token():
    return "JOURNEY-" + secrets.token_hex(4).upper()


def prompt(value):
    return "Reply with exactly " + value


class Journey:
    def __init__(self, runtime, daemon, controller, table, contract_row, repository, record, reader, *,
                 branch, created, sessions, route, model=None, quota=None):
        self.runtime, self.daemon, self.controller = runtime, daemon, controller
        self.table, self.row, self.repository = table, contract_row, repository
        self.record, self.reader = record, reader
        # The HOME the agent writes its sessions under, which Hide's reader searches.
        self.sessions = sessions
        # The session files Hide's reader read, for the request count and,
        # where the lane classifies them (CI login, which passes the rows of
        # contracts/provider-quota.json), a spent daily quota.
        self.paths, self.quota = [], quota
        # What the mode adds to the agent: arguments it passes, and the
        # environment it sets, recorded beside the spawn's own arguments.
        self.route = route
        # The branch this run makes in the measured repository, and where the
        # run keeps what it created there for the cleanup (PRD D-22).
        self.branch, self.created = branch, created
        self.id, self.kind = contract_row["id"], contract_row["herdr_kind"]
        self.run = runtime.run
        self.pane = self.agent = self.session = self.reference = self.checkout = None
        # Every pane this journey's spawns and fork opened, which close() ends.
        self.opened = []
        # A model reaches the agent only through the product's own model flag.
        self.model_args = [part.replace("{}", model) for part in contract_row["model_flag"]] if model else []

    # Evidence -----------------------------------------------------------

    def evidence(self, name, value):
        path = self.run / f"{self.id}-{name}.json"
        path.unlink(missing_ok=True)
        write_private(path, redact((json.dumps(value, indent=2, ensure_ascii=False, default=str) + "\n").encode()))
        return path.name

    def capture(self, name):
        """The agent pane's screen, kept only for a failed cell (PRD B12)."""
        if not self.pane:
            return None
        try:
            _, text, _ = self.runtime.command(["pane", "read", self.pane, "--source", "recent-unwrapped",
                                               "--lines", "200"], check=False)
        except ProcessError:
            return None
        path = self.run / f"{self.id}-{name}-pane.txt"
        path.unlink(missing_ok=True)
        write_private(path, redact(text.encode()))
        return path.name

    def settle(self, name, verdict, evidence=None):
        status, reason = verdict
        if status == cells.FAIL and self.quota is not None:
            spent = spent_quota(self.id, self.paths, self.quota)
            if spent:
                # Not retried: a retry spends the requests again (docs/TESTING.md, A real agent on CI).
                self.record["quota"] = spent
                reason = f"provider quota exhausted ({', '.join(spent)})"
                cells.settle(self.table, self.id, name, cells.NOT_MEASURED, reason, evidence)
                cells.settle_agent(self.table, self.id, reason)
                return False
        if status == cells.FAIL:
            pane = self.capture(name)
            evidence = ", ".join(value for value in (evidence, pane) if value) or None
        cells.settle(self.table, self.id, name, status, reason, evidence)
        return status == cells.PASS

    # Facts Hide reports ---------------------------------------------------

    def herdr_agent(self):
        return self.runtime.agent(self.pane)

    def conversation(self):
        """The whole conversation of the native session, as Hide's reader reads it."""
        reference = self.reference or {}
        kind = reference.get("kind") or "id"
        value = reference.get("value")
        if not value:
            return {"error": "Herdr reported no native session reference"}
        code, out, err = self.runtime.owner.run(
            [str(self.reader), str(self.sessions), self.kind, kind, value, str(self.checkout)],
            env={**self.runtime.env}, check=False, seconds=15)
        if code:
            return {"error": err.strip()[:300] or f"reader exit {code}"}
        try:
            answer = json.loads(out)
        except ValueError:
            answer = None
        if not (isinstance(answer, dict) and isinstance(answer.get("path"), str)
                and isinstance(answer.get("events"), list)):
            raise ProcessError("journey_conversation_answer_malformed: " + out[:200])
        if answer["path"] not in self.paths:
            self.paths.append(answer["path"])
        return {"session": self.session if kind == "path" else value, "path": answer["path"],
                "events": answer["events"]}

    def wait_turn(self, since, value, after=lambda events: 0, seconds=TURN_SECONDS):
        """Until the turn ended (cells.turn_verdict): Hide's reader has the
        answer carrying `value` from `after(events)` on, and the row read idle
        later than that answer. The row's history holds every activity the
        daemon's frames carried since `since`, so a turn that ended before
        this wait began is still read. Returns (verdict, read, observed)."""
        end = time.monotonic() + seconds
        while True:
            if not self.reference:
                self.reference = (self.herdr_agent() or {}).get("agent_session")
            read = self.conversation()
            observed = self.daemon.activities(self.pane, since)
            if "error" in read:
                verdict = cells.FAIL, "Hide's reader could not read the session: " + read["error"]
            else:
                events = read.get("events", [])
                verdict = cells.turn_verdict(observed, events, value, after(events))
            if verdict[0] == cells.PASS or time.monotonic() > end:
                return verdict, read, observed
            if self.runtime.owner.cancelled.wait(TURN_POLL_SECONDS):
                raise ProcessError("run_cancelled")

    # Cells ----------------------------------------------------------------

    STEP_CELLS = {"start": ("start", "reply"), "letter": ("letter",), "sleep_wake": ("sleep_wake",),
                  "resume": ("resume",), "untrusted": ("start_untrusted",)}

    def step(self, name, *args):
        """Runs one step; a transport failure or a bound it reached fails the
        step's open cell with that reason, and cancellation stops the run."""
        try:
            if name in MEASURES:
                MEASURES[name](self, *args)
            else:
                getattr(self, name)(*args)
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            for cell in self.STEP_CELLS.get(name, (name,)):
                if self.table[self.id][cell]["status"] == cells.PENDING:
                    self.settle(cell, (cells.FAIL, str(error)))
                    break

    def spawn(self, name, repository, branch, path, prompt_text):
        """`hide agent spawn` from the controller; a local request_timeout is the
        CLI's own wait running out while the spawn continues, so the same intent
        is asked again until it answers (it converges on one agent)."""
        flag = self.row["prompt_flag"]
        # The route's arguments are the wrapper's, on every start of the agent.
        native = [*self.model_args, flag, prompt_text]
        args = ["agent", "spawn", "--parent", "here", "--name", name, "--intent", name,
                "--kind", self.kind, "--repo", str(repository),
                "--branch", branch, "--path", str(path), "--", *native]
        attempts = []
        end = time.monotonic() + SPAWN_SECONDS
        while True:
            answer = self.controller.hide(args)
            attempts.append(answer)
            if answer.get("ok") is True or cells.refusal(answer)[0] not in RETRY:
                break
            if time.monotonic() > end:
                break
            if self.runtime.owner.cancelled.wait(2):
                raise ProcessError("run_cancelled")
        return args, answer, attempts

    def start(self):
        """start, then reply: the spawn, the turn its first prompt opened, the emitter, the reply."""
        first = token()
        worktree = self.runtime.probe / "worktrees" / self.id
        # Named before the spawn, so a spawn that raises or is stopped still leaves its branch to the cleanup.
        entry = {"branch": self.branch, "start": None, "worktree": str(worktree)}
        self.created.append(entry)
        args, answer, attempts = self.spawn("journey-" + self.id, self.repository, self.branch, worktree, prompt(first))
        entry.update(created_branch(self.runtime, self.repository, self.branch, worktree))
        self.record["spawn"] = {"argv": ["hide", *args], "route": self.route,
                                "answers": attempts}
        spawned = cells.spawn_verdict(answer)
        view = answer.get("value") or {}
        self.pane, self.agent, self.session = view.get("pane"), view.get("id"), view.get("session")
        self.opened.append(self.pane)
        self.checkout = worktree.resolve()
        evidence = self.evidence("spawn", self.record["spawn"])
        if spawned[0] != cells.PASS:
            self.pane = self.pane or self.pane_of(answer)
            self.record["spawn"]["herdr"] = self.herdr_view(worktree)
            evidence = self.evidence("spawn", self.record["spawn"])
            self.settle("start", spawned, evidence)
            return
        turn, read, observed = self.wait_turn(0, first)
        native = self.herdr_agent() or {}
        self.reference = native.get("agent_session") or self.reference
        self.record["start"] = {"activities": observed, "turn": turn, "herdr_agent": native,
                                "row": self.daemon.row(self.pane)}
        evidence = self.evidence("start", self.record["start"])
        verdict = cells.start_verdict(spawned, turn, cells.emitter_verdict(self.reference, self.kind))
        if not self.settle("start", verdict, evidence):
            return
        self.record["reply"] = {"token": first, "read": read}
        self.settle("reply", cells.reply_verdict(read, self.session, first), self.evidence("reply", self.record["reply"]))

    def letter(self):
        if not cells.measurable(self.table, self.id, "letter"):
            return
        # The doorbell rings only after the pane has been quiet; the body's
        # first line is what the bell line types, so the token stays in the
        # second line and only the letter Hide hands over can carry it.
        before = len(self.conversation().get("events", []))
        value = token()
        since = self.daemon.mark(self.pane)
        sent = self.controller.hide(["request", "send", self.agent, "--intent", "journey-letter-" + self.id,
                                     "--kind", "report", "--body",
                                     "Reply with exactly the token on the next line of this letter.\n" + value])
        record = {"token": value, "sent": sent, "shown": []}
        self.record["letter"] = record
        if sent.get("ok") is not True:
            self.settle("letter", (cells.FAIL, "hide request send refused: " + ": ".join(filter(None, cells.refusal(sent)))),
                        self.evidence("letter", record))
            return
        letter = sent["result"]["id"]
        end = time.monotonic() + LETTER_SECONDS
        shown = {}
        while time.monotonic() < end:
            answer = self.controller.hide(["request", "show", letter])
            shown = answer.get("result") or {}
            record["shown"].append({key: shown.get(key) for key in ("state", "bell_line", "hook_confirmed", "bell_attempts")})
            if shown.get("bell_line") and shown.get("state") == "delivered":
                break
            if self.runtime.owner.cancelled.wait(3):
                raise ProcessError("run_cancelled")
        turn = None
        if shown.get("bell_line"):
            # The turn the doorbell opened: from the human message it typed on.
            def opened(events):
                at = cells.bell_turn(events[before:], shown["bell_line"])
                return len(events) if at is None else before + at
            turn, read, record["activities"] = self.wait_turn(since, value, opened)
            record["turn"] = turn
        else:
            read = self.conversation()
        record["read"] = {**read, "events": read.get("events", [])[before:]} if "events" in read else read
        self.settle("letter", cells.with_turn(cells.letter_verdict(shown, record["read"], self.session, value), turn),
                    self.evidence("letter", record))

    def sleep_wake(self):
        if not cells.measurable(self.table, self.id, "sleep_wake"):
            return
        record = {"asleep": [], "awake": []}
        self.record["sleep_wake"] = record
        before = (self.herdr_agent() or {}).get("agent_session") or {}
        record["session_before"] = before
        # The operator has looked at the pane before they choose Sleep from its menu.
        self.daemon.dispatch("pane_visit", {"pane_id": self.pane})
        self.daemon.dispatch("agent_sleep", {"pane_id": self.pane})

        def asleep(_rest):
            row = self.daemon.row(self.pane) or {}
            state = (row.get("sleep") or {}).get("state")
            if not record["asleep"] or record["asleep"][-1] != state:
                record["asleep"].append(state)
            return state == "sleeping"
        try:
            self.daemon.wait(asleep, SLEEP_SECONDS, "the row to read asleep")
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            record["sleep_wait"] = str(error)
            record["errors"] = list(self.daemon.errors)
            self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], False), self.evidence("sleep-wake", record))
            return
        self.daemon.dispatch("agent_wake", {"pane_id": self.pane, "fresh": False})

        # Running again: the row is no longer asleep and the agent reports a
        # state of its own. The session it resumed is the resume cell's to judge.
        def awake(_rest):
            row = self.daemon.row(self.pane)
            if not row or row.get("sleep"):
                return False
            if not record["awake"] or record["awake"][-1] != row.get("activity"):
                record["awake"].append(row.get("activity"))
            return row.get("activity") in ("working", "stopped")
        woke = True
        try:
            self.daemon.wait(awake, WAKE_SECONDS, "the agent to run again")
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            woke = False
            record["wake_wait"] = str(error)
        self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], woke), self.evidence("sleep-wake", record))

    def resume(self):
        """The woken agent answers a typed prompt in the session it slept in.

        Herdr's integration reports a resumed session when its next turn
        starts (Codex reports none before it), so the session is read after
        that turn; the reply is read from the session slept in.
        """
        if not cells.measurable(self.table, self.id, "resume"):
            return
        before = self.record["sleep_wake"]["session_before"].get("value")
        record = {"session_before": before}
        self.record["resume"] = record
        events = len(self.conversation().get("events", []))
        value = token()
        record["token"] = value
        since = self.daemon.mark(self.pane)
        if not self.type_prompt(record, prompt(value)):
            self.settle("resume", (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                                   + " -> ".join(map(str, record["terminal"])) + ")"), self.evidence("resume", record))
            return
        turn, read, record["activities"] = self.wait_turn(since, value, lambda _events: events)
        record["turn"] = turn
        native = self.herdr_agent() or {}
        after = (native.get("agent_session") or {}).get("value")
        record["session_after"] = after
        if not before or before != after:
            self.settle("resume", cells.resume_verdict(before, after, (cells.FAIL, "")), self.evidence("resume", record))
            return
        record["read"] = read
        reply = cells.with_turn(cells.reply_verdict(read, self.session, value, events), turn)
        self.settle("resume", cells.resume_verdict(before, after, reply), self.evidence("resume", record))

    def type_prompt(self, record, text):
        """Types `text` and Return as the operator does: opening the agent puts
        its tab on screen, which attaches its terminal; the screen gives the
        terminal its grid once it exists and waits for a size, as the shell's
        terminal view does, and they type once it takes this screen's keys.
        Whether it did is left in `record`."""
        self.daemon.dispatch("focus_pane", {"pane_id": self.pane, "origin": "operator", "focus_device": False})
        states = record["terminal"] = []

        def controlling(_rest):
            state = (self.daemon.terminal(self.pane) or {}).get("transport_state")
            if not states or states[-1] != state:
                states.append(state)
                if state == "waiting_size":
                    grid = {"pane_id": self.pane, "cols": VIEW_COLS, "rows": VIEW_ROWS}
                    self.daemon.dispatch("terminal_viewport", {**grid, "new_view": True})
                    self.daemon.dispatch("terminal_resize", grid)
            return state == "controlling"
        try:
            self.daemon.wait(controlling, ATTACH_SECONDS, "the pane's terminal to take this screen's keys")
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            record["attach_wait"] = str(error)
            return False
        self.wait_ready()
        self.daemon.dispatch("key", {"pane_id": self.pane, "bytes_base64": base64.b64encode(text.encode()).decode()})
        self.runtime.owner.cancelled.wait(1)
        self.daemon.dispatch("key", {"pane_id": self.pane, "bytes_base64": base64.b64encode(b"\r").decode()})
        return True

    def turn_in(self, pane, record):
        """Types a prompt into `pane`, another agent pane this journey
        opened, as the operator does, and waits for its answer there. Returns
        (verdict, read, token); the journey's own pane is unchanged after."""
        saved = self.pane, self.session, self.reference
        self.pane, self.session, self.reference = pane, None, None
        try:
            value = record["token"] = token()
            since = self.daemon.mark(pane)
            if not self.type_prompt(record, prompt(value)):
                return (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                        + " -> ".join(map(str, record["terminal"])) + ")"), {}, value
            turn, read, record["activities"] = self.wait_turn(since, value)
            return turn, read, value
        finally:
            self.pane, self.session, self.reference = saved

    def wait_ready(self):
        """Herdr reads the woken agent ready for input."""
        try:
            self.runtime.wait(lambda: (agent := self.herdr_agent()) and agent.get("interactive_ready", True)
                              and agent.get("agent_status") in ("idle", "done") and not agent.get("launch_pending"), 60)
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            agent = self.herdr_agent() or {}
            raise ProcessError(f"agent_not_ready_for_input: Herdr reads {agent.get('agent_status')}, "
                               f"interactive_ready {agent.get('interactive_ready')}, "
                               f"launch_pending {agent.get('launch_pending')}") from error

    def untrusted(self, repository):
        """A start in a repository the account never trusted, with no preparation."""
        if not cells.measurable(self.table, self.id, "start_untrusted"):
            return
        value = token()
        worktree = self.runtime.probe / "worktrees-untrusted" / self.id
        args, answer, attempts = self.spawn("journey-untrusted-" + self.id, repository, "journey-untrusted-" + self.id,
                                            worktree, prompt(value))
        record = {"argv": ["hide", *args], "answers": attempts}
        self.record["start_untrusted"] = record
        spawned = cells.spawn_verdict(answer)
        saved_pane, saved = self.pane, (self.agent, self.session, self.reference, self.checkout)
        self.pane = (answer.get("value") or {}).get("pane") or self.pane_of(answer)
        self.opened.append(self.pane)
        try:
            if spawned[0] != cells.PASS:
                self.settle("start_untrusted", spawned, self.evidence("start-untrusted", record))
                return
            self.agent, self.session, self.checkout = answer["value"]["id"], answer["value"]["session"], worktree.resolve()
            self.reference = None
            turn, read, record["activities"] = self.wait_turn(0, value)
            native = self.herdr_agent() or {}
            record["herdr_agent"], record["turn"], record["read"] = native, turn, read
            self.settle("start_untrusted", cells.start_verdict(spawned, turn,
                                                               cells.emitter_verdict(native.get("agent_session"), self.kind),
                                                               cells.reply_verdict(read, self.session, value)),
                        self.evidence("start-untrusted", record))
        finally:
            self.pane = saved_pane
            self.agent, self.session, self.reference, self.checkout = saved

    def close(self):
        """Ends the panes this journey opened once its cells are settled. The
        run's guardian caps the private Herdr's processes at 2 GiB together,
        and each finished agent would otherwise keep its share beside the
        next agent's; a pane that does not close is named, and the run's
        teardown ends it."""
        closed = self.record["closed"] = {}
        for pane in dict.fromkeys(pane for pane in self.opened if pane):
            try:
                code, _, err = self.runtime.command(["pane", "close", pane], check=False)
            except ProcessError as error:
                if self.runtime.owner.cancelled.is_set():
                    raise
                code, err = 1, str(error)
            closed[pane] = "closed" if not code else err.strip()[:300] or f"exit {code}"

    def requests(self):
        """Model requests the run's sessions of this agent made."""
        return model_requests(self.kind, self.paths)

    def herdr_view(self, worktree):
        """What Herdr holds for a refused spawn: its workspaces and the
        repository's worktrees, as diagnostic evidence."""
        view = {}
        for name, args in (("workspaces", ["workspace", "list"]),
                           ("worktrees", ["worktree", "list", "--cwd", str(self.repository)])):
            code, out, err = self.runtime.command(args, check=False)
            view[name] = out[:20000] if not code else {"exit": code, "stderr": err[:500], "stdout": out[:2000]}
        view["worktree_exists"] = worktree.exists()
        return view

    def pane_of(self, answer):
        """A refused spawn may still have opened its pane: the newest pane that
        is neither the controller's nor one this journey already knows, for the
        failure capture only."""
        try:
            snapshot = self.runtime.json(["api", "snapshot"])["snapshot"]
        except (ProcessError, KeyError, ValueError):
            return None
        known = {self.pane, self.controller.pane, *self.record.get("panes", [])}
        panes = [pane["pane_id"] for pane in snapshot.get("panes", []) if pane["pane_id"] not in known]
        return panes[-1] if panes else None
