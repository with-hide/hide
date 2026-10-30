"""One agent's journey on the candidate: spawn, reply, letter, sleep and wake, resume.

Each step asks the product to act the way an operator or a parent agent does
(`hide agent spawn`, `hide request send`, the daemon's Sleep, Wake and terminal
input) and judges only what Hide then reports. A step that cannot run settles
its cell with the reason; nothing here types into a screen it read, but the
composer cell, which reads the pane's screen only to judge it.
"""

import base64
import json
import os
from pathlib import Path
import secrets
import time

from agent_live_check.processes import ProcessError
from agent_live_check.protection import write_private
from agent_live_check.system import git, same_path

from . import cells, composer, fork, kit, memory, provider_limit, title, usage
from .requests import model_requests

SPAWN_SECONDS = 150
TURN_SECONDS = 180
LETTER_SECONDS = 240
SLEEP_SECONDS = 90
WAKE_SECONDS = 150
ATTACH_SECONDS = 30
TURN_POLL_SECONDS = 1
# The moments a turn's trace keeps: each change of the row's activity, up to
# this many, with the screen and Herdr's reading at that change.
TURN_TRACE_CAP = 8
# The grid the journey's one view of a pane draws at.
VIEW_COLS, VIEW_ROWS = 120, 40
# Refusals the product answers while the same spawn is still under way, each
# saying to ask again with the same intent.
RETRY = ("request_timeout", "spawn_busy", "native_identity_unavailable")
# An agent whose letters ride its next prompt confirms one once its host
# wrote the reply after it (Pi, omp), which can land after the turn reads idle.
CONFIRM_SECONDS = 60
# A letter's path at rest is the contract's `letter` form (contracts/agent-journey.json).
TOOL_LETTER = ("this agent takes a letter only at a tool call its model makes while it works, or through a "
               "hide inbox it runs, and the journey never asks a model to choose an action (PRD agent-journey-check D-11); "
               "it closes once the adapter declares a measured path at rest: a prompt hook that carries the letter "
               "or a bell whose turn can take it")
LETTER_BODY = "Reply with exactly the token on the next line of this letter.\n"
LETTER_PROMPT = "Reply with exactly the token on the second line of the letter attached to this prompt."
# The cells each measured by a module of its own, judging what Hide shows.
MEASURES = {module.CELL: module.measure for module in (kit, memory, composer, fork, title, usage)}
# The steps of one agent's journey in order, each from the state the last
# left: the cells that only read what Hide shows come where that state is,
# the fork before Sleep so its source still runs the session it copies.
STEPS = ("start", "integration", "guidance_memory", "letter", "composer", "fork", "sleep_wake", "resume", "title", "usage")


def created_branch(runtime, repository, branch, worktree):
    """What a spawn made in the measured repository: the branch, the commit it
    started at, and its worktree, as evidence; they go with the probe."""
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
                 branch, created, sessions, route, composer_judge, model=None):
        self.runtime, self.daemon, self.controller = runtime, daemon, controller
        self.table, self.row, self.repository = table, contract_row, repository
        self.record, self.reader = record, reader
        # `journey_composer`, which judges a screen by the adapter's composer declaration.
        self.composer_judge = composer_judge
        # The HOME the agent writes its sessions under, which Hide's reader searches.
        self.sessions = sessions
        # The session files Hide's reader read, for the request count.
        self.paths = []
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
        self.model = model
        # Where the agent's provider log stood when the running step began:
        # what was logged before it is another step's, and never settles
        # this one's cell (provider_limit.cursor).
        self.provider_mark = None

    # Evidence -----------------------------------------------------------

    def evidence(self, name, value):
        path = self.run / f"{self.id}-{name}.json"
        path.unlink(missing_ok=True)
        write_private(path, (json.dumps(value, indent=2, ensure_ascii=False, default=str) + "\n").encode())
        return path.name

    def capture(self, name, pane=None):
        """The screen of the pane the cell measured, the agent's own unless
        it names another, kept only for a failed cell (PRD B12)."""
        pane = pane or self.pane
        if not pane:
            return None
        try:
            _, text, _ = self.runtime.command(["pane", "read", pane, "--source", "recent-unwrapped",
                                               "--lines", "200"], check=False)
        except ProcessError:
            return None
        path = self.run / f"{self.id}-{name}-pane.txt"
        path.unlink(missing_ok=True)
        write_private(path, text.encode())
        return path.name

    def settle(self, name, verdict, evidence=None, pane=None):
        """`pane` the pane the cell measured when it is not the agent's own,
        as a fork's is. A cell whose turn the agent never answered, while its
        provider still refused that conversation's request, is not measured,
        and so is every cell of the agent still open: the provider answers
        its next turns the same way, and a retry spends the requests again.
        Any other failure stays one."""
        status, reason = verdict
        refused = self.provider_refusal() if cells.kind(verdict) == cells.NO_ANSWER else None
        if refused:
            self.record["provider_limit"] = {"cell": name, "reason": refused, "failure": reason}
            cells.settle(self.table, self.id, name, cells.NOT_MEASURED, refused, evidence)
            cells.settle_agent(self.table, self.id, refused)
            return False
        if status == cells.FAIL:
            screen = self.capture(name, pane)
            evidence = ", ".join(value for value in (evidence, screen) if value) or None
        cells.settle(self.table, self.id, name, status, reason, evidence)
        return status == cells.PASS

    def provider_refusal(self):
        """Why the agent's provider refused its conversation's request in
        the running step, read where the agent itself records it
        (provider_limit.py), or None."""
        if self.kind != "codex" or self.provider_mark is None or not self.session:
            return None
        try:
            return provider_limit.codex_refusal(self.codex_home(), self.provider_mark, self.session)
        except provider_limit.Unreadable as error:
            self.record.setdefault("provider_limit_unread", []).append(str(error))
            return None

    def codex_home(self):
        """Codex's CODEX_HOME: `.codex` in the HOME it writes its sessions
        under in every mode (local.ROUTES, ci.AGENT_ENV)."""
        return self.sessions / ".codex"

    def mark_provider(self):
        """Where the agent's provider log stands as a step begins; None when
        the agent keeps none or it cannot be read, which reads no refusal."""
        if self.kind != "codex":
            return None
        try:
            return provider_limit.cursor(self.codex_home())
        except provider_limit.Unreadable as error:
            self.record.setdefault("provider_limit_unread", []).append(str(error))
            return None

    # Facts Hide reports ---------------------------------------------------

    def herdr_agent(self):
        return self.runtime.agent(self.pane)

    def turn_moment(self, activity):
        """The row's `activity` as it changed, with Herdr's reading of the
        agent and the pane's visible screen at that moment: a row that never
        reads idle again says, from these, whether the agent still showed a
        turn or Herdr kept reading one on an idle screen (with-hide/hide-private#309)."""
        moment = {"at_ms": int(time.time() * 1000), "activity": activity}
        try:
            agent = self.herdr_agent() or {}
            moment["herdr"] = {key: agent.get(key) for key in ("agent_status", "interactive_ready", "launch_pending")}
            _, moment["screen"], _ = self.runtime.command(["pane", "read", self.pane, "--source", "visible"], check=False)
        except ProcessError as error:
            moment["unreadable"] = str(error)
        return moment

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
        # A path session is the journey's own when it named one, else the
        # one Herdr reported in the pane, as a fork's is.
        return {"session": (self.session or value) if kind == "path" else value, "path": answer["path"],
                "events": answer["events"]}

    def wait_turn(self, since, value, after=lambda events: 0, seconds=TURN_SECONDS, trace=None):
        """Until the turn ended (cells.turn_verdict): Hide's reader has the
        answer carrying `value` from `after(events)` on, and the row read idle
        later than that answer. The row's history holds every activity the
        daemon's frames carried since `since`, so a turn that ended before
        this wait began is still read. Returns (verdict, read, observed).
        A `trace` list gets, at each change of the row's activity, what the
        pane showed and what Herdr read then (`turn_moment`)."""
        end = time.monotonic() + seconds
        last = None
        while True:
            if not self.reference:
                self.reference = (self.herdr_agent() or {}).get("agent_session")
            read = self.conversation()
            observed = self.daemon.activities(self.pane, since)
            if trace is not None and observed and observed[-1][0] != last and len(trace) < TURN_TRACE_CAP:
                last = observed[-1][0]
                trace.append(self.turn_moment(last))
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
        self.provider_mark = self.mark_provider()
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
        args = ["agent", "spawn", "--parent", "here", "--name", name, "--intent", name,
                "--kind", self.kind, "--repo", str(repository),
                "--branch", branch, "--path", str(path)]
        # The product retains its launch choice for Wake, Fork and reopen;
        # native arguments after -- reach only this first start.
        if self.model:
            args.extend(["--model", self.model])
        args.extend(["--", flag, prompt_text])
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
            if self.pane:
                self.record["spawn"]["shell_children"] = self.shell_children(self.pane)
            evidence = self.evidence("spawn", self.record["spawn"])
            self.settle("start", spawned, evidence)
            return
        turn, read, observed = self.wait_turn(0, first)
        native = self.herdr_agent() or {}
        self.reference = native.get("agent_session") or self.reference
        self.record["start"] = {"activities": observed, "turn": turn, "herdr_agent": native,
                                "row": self.daemon.row(self.pane)}
        evidence = self.evidence("start", self.record["start"])
        verdict = cells.start_verdict(spawned, turn, cells.emitter_verdict(self.reference, self.row["herdr_source"]))
        if not self.settle("start", verdict, evidence):
            return
        self.record["reply"] = {"token": first, "read": read}
        self.settle("reply", cells.reply_verdict(read, self.session, first), self.evidence("reply", self.record["reply"]))

    def letter(self):
        """The letter's path at rest is the agent's own, as the contract
        declares it: the doorbell's line, the next prompt, or a tool call."""
        if not cells.measurable(self.table, self.id, "letter"):
            return
        form = self.row["letter"]
        if form == "tool":
            cells.settle(self.table, self.id, "letter", cells.NOT_MEASURED, TOOL_LETTER)
            return
        # The token stays in the body's second line: a bell line types only
        # the first, so only the letter Hide hands over can carry it.
        before = len(self.conversation().get("events", []))
        value = token()
        since = self.daemon.mark(self.pane)
        sent = self.controller.hide(["request", "send", self.agent, "--intent", "journey-letter-" + self.id,
                                     "--kind", "report", "--body", LETTER_BODY + value])
        record = {"form": form, "token": value, "sent": sent, "shown": []}
        self.record["letter"] = record
        if sent.get("ok") is not True:
            self.settle("letter", (cells.FAIL, "hide request send refused: " + ": ".join(filter(None, cells.refusal(sent)))),
                        self.evidence("letter", record))
            return
        letter = sent["result"]["id"]
        if form == "next_prompt":
            self.prompt_letter(letter, record, before, since, value)
            return
        # The doorbell rings only after the pane has been quiet.
        shown = self.shown(letter, record, lambda shown: shown.get("bell_line") and shown.get("state") == "delivered",
                           LETTER_SECONDS)
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

    def prompt_letter(self, letter, record, before, since, value):
        """The letter rides the next prompt, as it would the operator's: it
        waits while the agent rests, and the prompt typed after it reaches
        the model with the letter the agent's prompt hook attached."""
        if not self.type_prompt(record, LETTER_PROMPT):
            self.settle("letter", (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                                   + " -> ".join(map(str, record["terminal"])) + ")"), self.evidence("letter", record))
            return
        turn, read, record["activities"] = self.wait_turn(since, value, lambda _events: before)
        record["turn"] = turn
        shown = self.shown(letter, record, lambda shown: shown.get("hook_confirmed") is True
                           and shown.get("state") == "delivered", CONFIRM_SECONDS)
        record["read"] = {**read, "events": read.get("events", [])[before:]} if "events" in read else read
        self.settle("letter", cells.with_turn(cells.prompt_letter_verdict(shown, read, self.session, value, before), turn),
                    self.evidence("letter", record))

    def shown(self, letter, record, done, seconds):
        """`hide request show` until `done(result)` or `seconds` pass; the last answer."""
        end = time.monotonic() + seconds
        shown = {}
        while time.monotonic() < end:
            answer = self.controller.hide(["request", "show", letter])
            shown = answer.get("result") or {}
            record["shown"].append({key: shown.get(key) for key in (
                "state", "bell_line", "hook_confirmed", "bell_attempts", "bell_turn", "tool_handovers")})
            if done(shown):
                break
            if self.runtime.owner.cancelled.wait(3):
                raise ProcessError("run_cancelled")
        return shown

    def sleep_wake(self):
        if not cells.measurable(self.table, self.id, "sleep_wake"):
            return
        record = {"form": self.row["wake"], "asleep": [], "awake": []}
        self.record["sleep_wake"] = record
        before = (self.herdr_agent() or {}).get("agent_session") or {}
        record["session_before"] = before
        # The operator has looked at the pane before they choose Sleep from its menu.
        self.daemon.dispatch("pane_visit", {"pane_id": self.pane})
        self.daemon.dispatch("agent_sleep", {"pane_id": self.pane})
        if record["form"] == "sleeping_session":
            self.wake_session(record)
            return

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

    def wake_session(self, record):
        """Sleep closed the pane and left a sleeping session in its checkout,
        which the operator's Wake opens in a new pane
        (`wake_sleeping_session`); the registration moves to that pane, which
        the resume then types into."""
        checkouts = [path for path in (self.checkout, self.runtime.probe / "worktrees" / self.id) if path]

        def sleeping(rest):
            return (rest.get("navigator") or {}).get("sleeping_sessions") or []

        def asleep(rest):
            found = next((entry for entry in sleeping(rest) if entry.get("kind") == self.kind
                          and any(same_path(entry.get("checkout_path"), path) for path in checkouts)), None)
            phase = found.get("phase") if found else None
            if not record["asleep"] or record["asleep"][-1] != phase:
                record["asleep"].append(phase)
            return found if phase == "sleeping" else None
        try:
            found = self.daemon.wait(asleep, SLEEP_SECONDS, "the sleeping session")
            record["checkouts_asleep"] = [checkout.get("path") for project in
                                          (self.daemon.rest.get("navigator") or {}).get("workspaces", [])
                                          for checkout in project.get("checkouts") or []][:64]
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            record["sleep_wait"] = str(error)
            record["errors"] = list(self.daemon.errors)
            self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], False), self.evidence("sleep-wake", record))
            return
        record["sleep_id"], record["wake_available"] = found["sleep_id"], found.get("wake_available")
        old, before = self.pane, found.get("reason")
        self.daemon.dispatch("wake_sleeping_session", {"sleep_id": found["sleep_id"]})
        record["waking"] = []

        # Running again: the sleeping session is gone, and Hide moved the
        # agent's registration to the pane Wake opened, which hosts the session
        # it slept in. The registration names that pane whatever the session is
        # called: a row carries no session id for a session recorded as a path,
        # such as Pi's (NavigatorAgent::session_id).
        def woken(rest):
            entry = next((entry for entry in sleeping(rest) if entry.get("sleep_id") == found["sleep_id"]), None)
            if entry:
                if not record["waking"] or record["waking"][-1] != entry.get("phase"):
                    record["waking"].append(entry.get("phase"))
                if entry.get("phase") == "failed":
                    return {"failed": entry.get("reason") or "failed"}
                # A refused Wake keeps the session asleep and says why on it.
                if entry.get("phase") == "sleeping" and entry.get("reason") and entry.get("reason") != before:
                    paths = [checkout.get("path") for project in (rest.get("navigator") or {}).get("workspaces", [])
                             for checkout in project.get("checkouts") or []]
                    record["refused"] = {"reason": entry["reason"], "entry": entry,
                                         "checkout_listed": entry.get("checkout_path") in paths,
                                         "checkouts": paths[:64]}
                    return {"failed": "refused: " + entry["reason"]}
                return None
            return {"gone": True}

        def registered():
            listing = self.controller.hide(["agent", "list"])
            item = next((item for item in (listing.get("value") or {}).get("items") or []
                         if item.get("id") == self.agent), None)
            pane = (item or {}).get("pane")
            return pane if pane and pane != old else None

        def running(rest):
            row = next((agent for agent in (rest.get("navigator") or {}).get("agents", [])
                        if agent.get("pane_id") == record["registered_pane"]), None)
            if not row:
                return None
            if not record["awake"] or record["awake"][-1] != row.get("activity"):
                record["awake"].append(row.get("activity"))
            return row if row.get("activity") in ("working", "stopped") else None
        end = time.monotonic() + WAKE_SECONDS
        try:
            woke = self.daemon.wait(woken, WAKE_SECONDS, "the sleeping session to wake")
            if "failed" not in woke:
                try:
                    # Outside the snapshot lock: the listing is a command the controller runs.
                    record["registered_pane"] = self.runtime.wait(registered, max(1, end - time.monotonic()))
                except ProcessError as error:
                    if self.runtime.owner.cancelled.is_set():
                        raise
                    raise ProcessError(f"timeout after {WAKE_SECONDS:.0f}s waiting for Hide's registration of "
                                       f"{self.agent} to name a pane other than {old}") from error
                woke = self.daemon.wait(running, max(1, end - time.monotonic()), "the woken session's pane")
        except ProcessError as error:
            if self.runtime.owner.cancelled.is_set():
                raise
            record["wake_wait"] = str(error)
            record["errors"] = list(self.daemon.errors)
            self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], False, str(error)),
                        self.evidence("sleep-wake", record))
            return
        if "failed" in woke:
            self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], False, "wake failed: " + woke["failed"]),
                        self.evidence("sleep-wake", record))
            return
        self.record.setdefault("panes", []).append(old)
        self.pane = record["woken_pane"] = woke["pane_id"]
        # A pane Wake opened is the journey's to close, as a spawn's is.
        self.opened.append(self.pane)
        self.settle("sleep_wake", cells.sleep_verdict(record["asleep"], True), self.evidence("sleep-wake", record))

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

    def take_keys(self, record):
        """Opens the agent as the operator does: opening it puts its tab on
        screen, which attaches its terminal; the screen gives the terminal
        its grid once it exists and waits for a size, as the shell's terminal
        view does, and the keys go once it takes this screen's keys and
        Herdr reads the agent ready. Whether it took them is left in
        `record`."""
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
        return True

    def type_prompt(self, record, text):
        """Types `text` and Return as the operator does, once the terminal
        takes this screen's keys (`take_keys`)."""
        if not self.take_keys(record):
            return False
        self.daemon.dispatch("key", {"pane_id": self.pane, "bytes_base64": base64.b64encode(text.encode()).decode()})
        self.runtime.owner.cancelled.wait(1)
        self.daemon.dispatch("key", {"pane_id": self.pane, "bytes_base64": base64.b64encode(b"\r").decode()})
        return True

    def turn_in(self, pane, record):
        """Types a prompt into `pane`, another agent pane this journey
        opened, as the operator does, and waits for its answer there. Returns
        (verdict, read, token); the journey's own pane is unchanged after, so
        a cell settled after it names `pane` for its screen capture."""
        saved = self.pane, self.session, self.reference
        self.pane, self.session, self.reference = pane, None, None
        try:
            value = record["token"] = token()
            since = self.daemon.mark(pane)
            if not self.type_prompt(record, prompt(value)):
                return (cells.FAIL, "the pane's terminal never took this screen's keys (transport "
                        + " -> ".join(map(str, record["terminal"])) + ")"), {}, value
            trace = []
            turn, read, record["activities"] = self.wait_turn(since, value, trace=trace)
            if turn[0] != cells.PASS:
                record["turn_trace"] = trace
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
                                                               cells.emitter_verdict(native.get("agent_session"), self.row["herdr_source"]),
                                                               cells.reply_verdict(read, self.session, value)),
                        self.evidence("start-untrusted", record))
        finally:
            self.pane = saved_pane
            self.agent, self.session, self.reference, self.checkout = saved

    def close(self, keep=None):
        """Ends the panes this journey opened that no later cell types into:
        each one but `keep` after a step, so a fork's ends with its cell, and
        every one once its cells are settled. The run's guardian caps the
        private Herdr's processes at 2 GiB together, and a fork, the session
        it came from and an untrusted start of omp held 2.17 GB at once on
        Linux (2026-10-18); a pane that does not close is named, and the run's
        teardown ends it."""
        closed = self.record.setdefault("closed", {})
        for pane in dict.fromkeys(pane for pane in self.opened if pane and pane != keep and pane not in closed):
            try:
                code, _, err = self.runtime.command(["pane", "close", pane], check=False)
            except ProcessError as error:
                if self.runtime.owner.cancelled.is_set():
                    raise
                code, err = 1, str(error)
            closed[pane] = "closed" if not code else err.strip()[:300] or f"exit {code}"

    def requests(self):
        """Model requests the run's sessions of this agent made."""
        return model_requests(self.kind, self.paths, self.sessions)

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

    def shell_children(self, pane):
        """The processes under a refused spawn's pane shell, by pid, parent
        and name only: Herdr refuses `agent.start` on Windows while the shell
        has any child, and its `pane.process_info` names none that is not an
        agent, so this is the one record of what held the pane."""
        try:
            info = self.runtime.json(["pane", "process-info", pane])["process_info"]
            shell = int(info["shell_pid"])
        except (ProcessError, KeyError, TypeError, ValueError) as error:
            return {"error": str(error)[:200]}
        if os.name == "nt":
            argv = ["powershell.exe", "-NoProfile", "-Command",
                    "Get-CimInstance Win32_Process | ForEach-Object { '{0} {1} {2}' -f "
                    "$_.ProcessId, $_.ParentProcessId, $_.Name }"]
        else:
            argv = ["ps", "-A", "-o", "pid=,ppid=,comm="]
        code, out, err = self.runtime.owner.run(argv, env={**self.runtime.env}, check=False, seconds=15)
        if code:
            return {"shell_pid": shell, "error": (err.strip() or f"exit {code}")[:200]}
        parents = {}
        for line in out.splitlines():
            fields = line.split(None, 2)
            if len(fields) == 3 and fields[0].isdigit() and fields[1].isdigit():
                parents[int(fields[0])] = (int(fields[1]), os.path.basename(fields[2].strip()))
        below, queue = [], [shell]
        while queue and len(below) < 64:
            parent = queue.pop(0)
            for pid, (ppid, name) in sorted(parents.items()):
                if ppid == parent and pid != shell and all(pid != seen["pid"] for seen in below):
                    below.append({"pid": pid, "parent": ppid, "name": name})
                    queue.append(pid)
        return {"shell_pid": shell, "children": below}

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
