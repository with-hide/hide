use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::watch::Watch;
use super::{
    ANSWER_WAIT_MS, Actor, BODY_LIMIT, FILE_LIMIT, LETTER_LIMIT, OPEN_LIMIT, RETENTION_MS,
    WATCH_LIMIT,
};

/// The bells one letter has: the doorbell types it at most this many
/// times (`Letter::reserve_bell`).
pub(crate) const BELLS: u8 = 3;
/// How long a live agent that took none of a letter's bells rests before the
/// letter's next round of three (PRD letters-missions-simplify B6): Hide
/// wakes it again rather than leaving the letter to wait for it, and a round
/// per half hour keeps that from typing into it at every pass.
pub(crate) const BELL_ROUND_MS: u64 = 30 * 60 * 1000;

/// Where a letter stands. Hide ends a pending letter one of four ways
/// (PRD letters-missions-simplify D-04): `delivered` once its recipient took
/// it in, `superseded` by its sender's newer letter under the same intent,
/// `no_recipient` when its recipient ended with no agent after it in its
/// pane, and `expired` only at the update to this build. `acknowledged` is
/// a delivered letter with its recipient's receipt, and `cancelled` its
/// sender's own withdrawal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Delivered,
    Acknowledged,
    Cancelled,
    /// The same sender sent another body under the same intent while this
    /// one waited (`mailbox::send`).
    Superseded,
    /// Its recipient's registration ended with no successor (B2).
    NoRecipient,
    /// Ended at the update to this build (B25): a Hide letter still waiting
    /// then, or a letter an older build ended at its delivery deadline,
    /// which it called `undelivered`.
    #[serde(alias = "undelivered")]
    Expired,
}

/// Why a letter stopped waiting for an answer nobody gave.
// The letter keeps its state: a new `State` value would make an older build
// refuse the whole ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerWaitEnd {
    /// The sender's or the recipient's registration ended.
    PartyEnded,
    /// The letter was sent 24 hours ago and no answer came.
    Deadline,
}

impl AnswerWaitEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PartyEnded => "party_ended",
            Self::Deadline => "deadline",
        }
    }
}

/// The native session and submitted bell digest that own this intake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BellTurn {
    pub session: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Letter {
    pub id: String,
    pub intent: String,
    pub sender: Actor,
    pub recipient: Actor,
    /// The registration that sent the letter (PRD letters-missions-simplify
    /// D-02), whose live successor an answer goes to; null when the sender
    /// had none, as an older build's letters may.
    #[serde(default)]
    pub from: Option<String>,
    /// The registration the letter is addressed to: the agent, not its pane.
    /// `recipient` is where that registration runs now, which the store keeps
    /// with it: a move, a wake or Reopen carries it (`coordination::rebind`),
    /// and an end hands a pending letter to the agent that took over its pane
    /// (`coordination::succeed`) or ends it `no_recipient`. A letter an agent,
    /// a tool or Desk sends always names one, since the send registers an
    /// agent that had none (`Runtime::prepare_as`); null only when a letter
    /// Hide writes, or an older build wrote, went to an agent with none.
    #[serde(default)]
    pub to: Option<String>,
    pub kind: String,
    pub body: String,
    /// How many bytes at the start of `body` its sender wrote, when the core
    /// appended something after them (a report's list of the Views its
    /// sender opened, `Runtime::list_opened_views`); null when the body is
    /// all the sender's. A send under the same intent is the same letter
    /// again only when it says exactly the sender's part (B5).
    #[serde(default)]
    pub written: Option<usize>,
    pub state: State,
    #[serde(default)]
    pub hook_confirmed: Option<bool>,
    pub waiting_answer: bool,
    pub reply_to: Option<String>,
    pub created_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub bell_errors: u8,
    pub bell_sent: bool,
    #[serde(default)]
    pub bell_attempts: Option<u8>,
    #[serde(default)]
    pub human_notified: bool,
    #[serde(default)]
    pub watch_warning: Option<super::watch::WarningReceipt>,
    /// Set when the wait for an answer ended without one; null otherwise.
    #[serde(default)]
    pub answer_wait_ended: Option<AnswerWaitEnd>,
    /// The line the doorbell typed for this letter last, saved with its
    /// reservation before it was typed; null until the first bell. A
    /// submitted prompt is that bell when its canonical digest matches this
    /// line; older hooks' whole-line digests remain accepted too.
    #[serde(default)]
    pub bell_line: Option<String>,
    #[serde(default)]
    pub bell_turn: Option<BellTurn>,
    /// A prompt hook of the recipient's own session asked with the bell
    /// line's digest and was handed the letter's body to print. No bell is
    /// typed for it again, whether or not the hook's confirmation arrived
    /// (`delivery::doorbell`): the next bell's batch for the same recipient,
    /// a tool or turn-end hook or `hide inbox` brings it instead, and when its
    /// recipient ends with no successor it ends delivered without a receipt
    /// (`Ledger::end_letters_to`).
    #[serde(default)]
    pub handed: bool,
    /// When a hook last handed the letter over, at a prompt (`handed`) or at
    /// a tool (`tool_handovers`), on the core's clock: a rest whose state
    /// moved after it is a rest that hook had ended before, when the core
    /// asks the pane's machine for a confirmation it could not send
    /// (`Doorbell::settle_owed`). Null until handed, and for a letter an
    /// older build handed.
    #[serde(default)]
    pub handed_at_unix_ms: Option<u64>,
    /// Sent with `hide request send --now`: delivered at the recipient's
    /// next safe gap while it works, without the mid-turn wait (PRD
    /// letters-to-busy-agents D-05). An older build drops the field at its
    /// next save, and the letter then waits as any other.
    #[serde(default)]
    pub immediate: bool,
    /// Sent with `hide request send --force` (PRD engine-bundle-1 D-14): its
    /// composer was read empty when it was sent, and the doorbell types its
    /// bell even while the recipient works, after reading the composer empty
    /// once more; a composer that shows text then leaves it an ordinary
    /// letter sent with `--now`. An older build drops the field and the
    /// letter waits as a `--now` letter.
    #[serde(default)]
    pub forced: bool,
    /// How many tool events handed the letter over while its recipient
    /// worked (PRD letters-to-busy-agents D-11): a handover is confirmed by
    /// the script once the host kept it, and after three unconfirmed ones
    /// the letter waits for the next prompt or `hide inbox`, as it does
    /// after three bells.
    #[serde(default)]
    pub tool_handovers: u8,
    /// When the doorbell last typed this letter's bell into its recipient's
    /// pane (`agent.prompt` answered); the recipient's own record of the
    /// prompt confirms it (`delivery::typed`). Null until a bell was typed.
    #[serde(default)]
    pub bell_typed_at_unix_ms: Option<u64>,
    /// How the last bell was sent (`delivery::ladder`), which words its
    /// unconfirmed notice; the recipient's record confirms it either way.
    #[serde(default)]
    pub bell_route: super::typed::SendRoute,
    /// The bell whose unconfirmed notice the sender was sent, 0 for none.
    #[serde(default)]
    pub input_noticed: u8,
    /// When that notice was sent, 0 for none.
    #[serde(default)]
    pub input_noticed_at_unix_ms: u64,
    /// Why the letter's last bell failed, while no bell typed after it went
    /// through; absent otherwise. Its sender reads the cause here, and the
    /// doorbell reads whether the line may be in the agent's input. An older
    /// build drops the field at its next save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bell_failure: Option<BellFailure>,
    /// The bells whose result the store recorded: the doorbell rings a
    /// letter one bell at a time and records how each ended (typed, failed,
    /// or held after its reservation) before it reserves the next, so a
    /// result ends the bell reserved last. Fewer than `bell_attempts` while
    /// a bell is on its way. Null on a letter an earlier build rang, which
    /// kept no such count; an older build drops the field at its next save.
    #[serde(default)]
    pub bell_ended: Option<u8>,
    /// The id the sending core's ledger gives this letter, on a letter an
    /// agent of another core sent (PRD device-connect-iroh D-72): on the
    /// link a letter is named by its sender's machine and that id, so a
    /// reply to this one names it. Absent on every other letter; an older
    /// build drops the field at its next save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_id: Option<String>,
}

/// A start an agent of this core asked of another machine's core with no
/// parent (PRD device-connect-iroh D-59, D-71): the work is that machine's,
/// and this core keeps only who asked and where. A machine this one opens
/// writes nothing here first, so the result report of what was started
/// there is taken from it by this record alone (`access::core_admit`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreStart {
    /// The registration that asked: the agent, not its pane, so the agent
    /// that runs in its place later still takes the report.
    pub origin: String,
    /// The other machine's connection id, which it proved: a device id is
    /// only what a machine says of itself, and two connections may say the
    /// same one.
    pub key: String,
    pub at_unix_ms: u64,
}

/// The most starts on other cores the ledger remembers. At the cap the
/// oldest whose asker ended goes, since no report reaches an ended agent;
/// while every asker still lives a new start is refused before it leaves
/// (`Ledger::note_core_start`).
pub const CORE_STARTS: usize = 256;

/// A bell that failed: the doorbell's code for it, and whether the engine
/// had been asked to type the line, so it may be in the agent's input
/// (`docs/delivery.md`, Letters while the agent works). A failure that typed
/// nothing after one that may have keeps that one's `maybe_typed` and time:
/// the line it may have typed is still where it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BellFailure {
    pub code: String,
    pub maybe_typed: bool,
    pub at_unix_ms: u64,
}

impl Ledger {
    /// The request and block letters this pane's session sent whose answer
    /// it still waits for at `now`. A letter from another session of the
    /// same pane does not count: that session ended with its wait.
    pub(crate) fn awaited_answers<'a>(
        &'a self,
        pane_id: &'a str,
        session: Option<&'a str>,
        now: u64,
    ) -> impl Iterator<Item = &'a Letter> + 'a {
        self.letters.iter().filter(move |letter| {
            session.is_some()
                && letter.waiting_answer
                && !letter.answer_overdue(now)
                && letter.sender.pane_id == pane_id
                && letter.sender.session.as_deref() == session
        })
    }

    /// The pending letters a bell is still to name to their recipient: one
    /// rings for them (`Letter::rings`), none of their own was typed, no
    /// prompt hook was handed them, and no bell was typed into the same
    /// recipient's pane since they were queued. A bell at
    /// rest names every letter waiting for its recipient (` · and N more`,
    /// `doorbell::Hold::Announced`), and the prompt hook that takes it is
    /// handed them all as one bundle; a kind with no prompt hook, or whose
    /// line names `hide inbox` (Codex, Grok), gets the others when it runs
    /// `hide inbox`. A bell typed while the agent works names the rest too;
    /// no automatic sleep takes a working agent, and its hook hands the same
    /// bundle. The bell's time is when it was typed, after its line was
    /// worded, so a letter queued in those seconds counts as named though the
    /// line did not count it; the hook still hands it with the rest.
    pub(crate) fn unannounced(&self) -> impl Iterator<Item = &Letter> {
        // The recipient as `agent_sleep::LetterWaits` keys it: its pane and
        // the session the letter addresses.
        fn recipient(letter: &Letter) -> (&str, Option<&str>) {
            (
                &letter.recipient.pane_id,
                letter.recipient.session.as_deref(),
            )
        }
        let mut last_bell = BTreeMap::new();
        for letter in &self.letters {
            if let Some(typed) = letter.bell_typed_at_unix_ms {
                let last: &mut u64 = last_bell.entry(recipient(letter)).or_default();
                *last = (*last).max(typed);
            }
        }
        self.letters.iter().filter(move |letter| {
            letter.state == State::Pending
                && letter.rings()
                && !letter.bell_sent
                && !letter.handed
                && last_bell
                    .get(&recipient(letter))
                    .is_none_or(|typed| *typed < letter.created_at_unix_ms)
        })
    }
}

impl Letter {
    /// The letter as an answer to `reader` shows it, its sender's and
    /// recipient's panes as that reader uses them (`Actor::read_by`).
    pub(crate) fn read_by(&self, reader: crate::address::Reader<'_>) -> Letter {
        Letter {
            sender: self.sender.read_by(reader),
            recipient: self.recipient.read_by(reader),
            ..self.clone()
        }
    }

    /// Whether a bell is rung for this letter while its recipient is awake,
    /// at rest or at work: every kind but `progress`, which is news and
    /// never the reason a turn starts, `--now` or not. A progress letter
    /// rides whatever next hands its recipient's letters over: another
    /// letter's bell, a tool gap, a turn end beside a letter that rings,
    /// `hide inbox`. A `--force` letter keeps the
    /// bell its sender asked for. Asleep, fewer still wake the agent
    /// ([`Letter::wakes_a_sleeper`]).
    pub(crate) fn rings(&self) -> bool {
        self.forced || self.kind != "progress"
    }

    /// Whether this letter wakes its recipient when Hide put it to sleep:
    /// one that asks something of it (`request`, `block`), or the
    /// operator's Done on its person's turn, which it is to act on now
    /// though no answer is owed (PRD ask-human B20). Every other report or
    /// progress waits for the next wake.
    pub(crate) fn wakes_a_sleeper(&self) -> bool {
        matches!(self.kind.as_str(), "request" | "block")
            || (self.sender.operator_owned()
                && crate::coordination::human_turn::is_done_intent(&self.intent))
    }

    pub fn open(&self) -> bool {
        self.awaiting_intake() || self.waiting_answer
    }

    /// Whether the ledger still keeps the letter at `now`: it is open, or it
    /// finished less than [`RETENTION_MS`] ago.
    pub(crate) fn retained(&self, now: u64) -> bool {
        self.open()
            || self
                .finished_at_unix_ms
                .is_none_or(|finished| now.saturating_sub(finished) < RETENTION_MS)
    }

    pub(crate) fn intake_confirmed(&self) -> bool {
        // Only legacy Delivered proves a hook confirmation. A legacy Ack
        // may have preceded intake, so its missing receipt remains unknown,
        // as does a handed letter's that ended without one.
        self.hook_confirmed
            .unwrap_or(self.state == State::Delivered && !self.handed)
    }

    /// The part of the body its sender wrote ([`Letter::written`]).
    pub(crate) fn written_body(&self) -> &str {
        self.written
            .and_then(|written| self.body.get(..written))
            .unwrap_or(&self.body)
    }

    pub(crate) fn awaiting_intake(&self) -> bool {
        self.state == State::Pending
            || (self.state == State::Acknowledged && self.hook_confirmed == Some(false))
    }

    /// Whether the answer is still awaited past its deadline.
    pub(crate) fn answer_overdue(&self, now: u64) -> bool {
        self.waiting_answer && now.saturating_sub(self.created_at_unix_ms) >= ANSWER_WAIT_MS
    }

    /// Ends the wait for an answer. A letter still awaiting intake stays open
    /// for that, and `finished_at_unix_ms` is set once nothing else is awaited.
    fn end_answer_wait(&mut self, why: AnswerWaitEnd, now: u64) {
        self.waiting_answer = false;
        self.answer_wait_ended = Some(why);
        if !self.open() {
            self.finished_at_unix_ms.get_or_insert(now);
        }
    }

    /// The letter's bells as typed input awaiting its recipient's record:
    /// a pending letter has none yet, since intake takes it out of pending.
    pub(crate) fn typed_input(&self) -> Option<super::typed::TypedInput> {
        Some(super::typed::TypedInput {
            attempts: self.attempts(),
            typed_at_unix_ms: self.bell_typed_at_unix_ms?,
            first_typed_at_unix_ms: 0,
            // A bell its recipient took is recorded, confirmed or not.
            confirmed_at_unix_ms: (self.state != State::Pending || self.handed).then_some(0),
            noticed: self.input_noticed,
            noticed_at_unix_ms: self.input_noticed_at_unix_ms,
            route: self.bell_route,
        })
    }

    pub(crate) fn attempts(&self) -> u8 {
        // Legacy successes have an unknown total. Do not infer spare attempts
        // from the old boolean and repeat an already unbounded external effect.
        self.bell_attempts.unwrap_or(if self.bell_sent {
            BELLS
        } else {
            self.bell_errors
        })
    }

    /// Whether a bell is on its way: reserved, and its result not yet
    /// recorded. A letter an earlier build rang has no record of it.
    pub(crate) fn bell_in_flight(&self) -> bool {
        self.bell_ended.is_some_and(|ended| ended < self.attempts())
    }

    /// Spends one of the letter's three bells on `line`, which the doorbell
    /// types once this reservation is saved.
    pub(crate) fn reserve_bell(&mut self, line: String) -> Result<u8, String> {
        self.recipient.require_native_identity()?;
        if !super::bell::valid_line(&line) {
            return Err("bell_line_invalid".into());
        }
        if self.state != State::Pending {
            return Err("letter_not_pending".into());
        }
        if self.handed {
            return Err("letter_handed".into());
        }
        if self.attempts() >= BELLS {
            return Err("doorbell_attempt_limit".into());
        }
        let attempt = self.attempts() + 1;
        self.bell_attempts = Some(attempt);
        self.bell_line = Some(line);
        Ok(attempt)
    }

    /// When the letter's last bell was typed, failed or was cut short, or when
    /// the letter was sent if none was.
    pub(crate) fn last_bell_at(&self) -> u64 {
        self.bell_typed_at_unix_ms
            .into_iter()
            .chain(self.bell_failure.as_ref().map(|failure| failure.at_unix_ms))
            .max()
            .unwrap_or(self.created_at_unix_ms)
    }

    /// Whether the letter waits for its agent with its three bells spent, none
    /// on its way, and a round since the last (`BELL_ROUND_MS`).
    pub(crate) fn bell_round_due(&self, now: u64) -> bool {
        self.state == State::Pending
            && !self.handed
            && self.attempts() >= BELLS
            && !self.bell_in_flight()
            && now.saturating_sub(self.last_bell_at()) >= BELL_ROUND_MS
    }

    /// Gives the letter its next round of three bells (`bell_round_due`).
    /// Its sender was told of the bells not taken once, and is not again.
    pub(crate) fn renew_bell_round(&mut self) {
        let noticed = (self.input_noticed, self.input_noticed_at_unix_ms);
        self.renew_bells();
        (self.input_noticed, self.input_noticed_at_unix_ms) = noticed;
    }

    /// Gives a letter whose bells are spent three more, once its agent runs
    /// anew (woken, moved or reopened, `coordination::rebind`): the bells
    /// went to a run that did not take it (PRD letters-missions-simplify B6).
    /// A bell still on its way keeps the count it belongs to.
    pub(crate) fn renew_bells(&mut self) {
        if self.attempts() < BELLS || self.bell_in_flight() {
            return;
        }
        self.bell_errors = 0;
        self.bell_sent = false;
        self.bell_attempts = Some(0);
        self.bell_ended = Some(0);
        self.input_noticed = 0;
        self.input_noticed_at_unix_ms = 0;
    }

    /// Hands the pending letter to the registration `to` that took over its
    /// recipient's pane, running as `recipient` (B2). The new
    /// agent is another conversation, which no bell rang and no hook handed
    /// anything to: the letter is rung and handed to it as a new letter is.
    pub(crate) fn hand_to(&mut self, to: &str, recipient: Actor) {
        self.to = Some(to.to_owned());
        self.recipient = recipient;
        self.bell_errors = 0;
        self.bell_sent = false;
        self.bell_attempts = Some(0);
        self.bell_ended = Some(0);
        self.bell_line = None;
        self.bell_turn = None;
        self.bell_typed_at_unix_ms = None;
        self.bell_route = Default::default();
        self.bell_failure = None;
        self.input_noticed = 0;
        self.input_noticed_at_unix_ms = 0;
        self.handed = false;
        self.handed_at_unix_ms = None;
        self.tool_handovers = 0;
    }

    /// Ends the letter, which nothing else ends, `state` at `now`: its
    /// recipient no longer awaits it, nor its sender an answer.
    /// Ends the pending letter a newer one from its sender under the same
    /// intent replaces (`mailbox::send`, B5).
    pub(crate) fn supersede(&mut self, now: u64) {
        self.end(State::Superseded, now);
    }

    /// Ends the letter awaiting intake whose agent ended with no agent after
    /// it in its pane: a pending one `no_recipient`, unless a hook already
    /// handed it into that agent's session, which ends it `delivered`
    /// without a receipt; one an older build acknowledged without a receipt
    /// stops awaiting intake. A report handed so returns the pair whose
    /// watch it ends.
    fn recipient_ended(&mut self, now: u64) -> Option<(Actor, Actor)> {
        if self.state == State::Pending && self.handed {
            self.state = State::Delivered;
            self.hook_confirmed = None;
            if !self.open() {
                self.finished_at_unix_ms = Some(now);
            }
            return (self.kind == "report").then(|| (self.sender.clone(), self.recipient.clone()));
        }
        if self.state == State::Pending {
            self.end(State::NoRecipient, now);
            crate::diagnostic!(serde_json::json!({
                "component": "delivery", "kind": "letter.no_recipient",
                "letter_id": self.id, "agent_id": self.to,
                "pane_id": self.recipient.pane_id,
            }));
            return None;
        }
        self.hook_confirmed = None;
        if !self.open() {
            self.finished_at_unix_ms.get_or_insert(now);
        }
        None
    }

    fn end(&mut self, state: State, now: u64) {
        self.state = state;
        self.waiting_answer = false;
        self.finished_at_unix_ms = Some(now);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    pub next_id: u64,
    pub letters: Vec<Letter>,
    pub watches: Vec<Watch>,
    #[serde(default)]
    pub agents: Vec<crate::coordination::AgentRecord>,
    #[serde(default)]
    pub spawns: Vec<crate::coordination::SpawnRecord>,
    /// Registrations owed to agents Hide started that have not shown their
    /// session yet. Left out of the file while there are none, so a ledger
    /// without a wait reads the same to a build from before them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identity_waits: Vec<crate::coordination::identity_wait::IdentityWait>,
    /// Changed only when durable spawn placement moves, never by letters or watches.
    #[serde(default)]
    pub spawn_placement_revision: u64,
    /// The operator's turn each live registration declared (PRD ask-human),
    /// by registration id: at most one per agent, so the registrations
    /// bound it. Left out of the file while there are none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub human_turns: BTreeMap<String, crate::coordination::human_turn::HumanTurn>,
    /// The starts this core's agents asked of other cores ([`CoreStart`]),
    /// oldest first and [`CORE_STARTS`] at most. Left out of the file while
    /// there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub core_starts: Vec<CoreStart>,
    /// What each agent saw of its status cells (`runtime::status_cells`), by
    /// registration id: the subject of each cell and the revision it saw, so
    /// a turn after a restart is not told again what it already read. One
    /// entry per subject a cell stands for now, kept by the core; left out of
    /// the file while there are none, and an older build drops it at its
    /// next save, which shows each cell standing then once more.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub status_seen: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            letters: Vec::new(),
            watches: Vec::new(),
            agents: Vec::new(),
            spawns: Vec::new(),
            identity_waits: Vec::new(),
            spawn_placement_revision: 0,
            human_turns: BTreeMap::new(),
            core_starts: Vec::new(),
            status_seen: BTreeMap::new(),
        }
    }
}

/// What [`Ledger::address_letters`] ended.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Addressed {
    pub(crate) no_recipient: usize,
    pub(crate) expired: usize,
}

impl Ledger {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.next_id == 0 {
            return Err("ledger_unavailable".into());
        }
        if self.letters.len() > LETTER_LIMIT
            || self.letters.iter().filter(|letter| letter.open()).count() > OPEN_LIMIT
            || self.watches.len() > WATCH_LIMIT
        {
            return Err("capacity".into());
        }
        let mut ids = HashSet::new();
        for letter in &self.letters {
            if !ids.insert(&letter.id)
                || !super::valid_key(&letter.id)
                || !letter.id.starts_with("letter-")
                || !super::valid_key(&letter.intent)
                || !letter.sender.valid()
                || !letter.recipient.valid()
                || letter
                    .from
                    .as_deref()
                    .is_some_and(|id| !super::valid_key(id))
                || letter.to.as_deref().is_some_and(|id| !super::valid_key(id))
                || !matches!(
                    letter.kind.as_str(),
                    "request" | "block" | "report" | "progress" | "reply" | "watch"
                )
                || letter.body.len() > BODY_LIMIT
                || letter.body.trim().is_empty()
                || letter
                    .written
                    .is_some_and(|written| !letter.body.is_char_boundary(written))
                || letter.bell_errors > BELLS
                || letter.bell_turn.as_ref().is_some_and(|turn| {
                    !hide_agent_hooks::delivery::valid_sha256_digest(&turn.session)
                        || !hide_agent_hooks::delivery::valid_prompt_digest(&turn.digest)
                        || letter.recipient.session.as_ref() != Some(&turn.session)
                })
                || letter
                    .bell_attempts
                    .is_some_and(|attempts| attempts > BELLS)
                || letter
                    .bell_ended
                    .is_some_and(|ended| ended > letter.attempts())
                || letter
                    .bell_failure
                    .as_ref()
                    .is_some_and(|failure| !super::valid_key(&failure.code))
                || letter
                    .bell_line
                    .as_deref()
                    .is_some_and(|line| !super::bell::valid_line(line))
                || (letter.hook_confirmed == Some(true)
                    && matches!(
                        letter.state,
                        State::Pending | State::NoRecipient | State::Expired
                    ))
                || (letter.hook_confirmed == Some(false) && letter.state == State::Delivered)
                || letter
                    .watch_warning
                    .as_ref()
                    .is_some_and(|warning| letter.kind != "watch" || !warning.valid())
                || (!letter.open() && letter.finished_at_unix_ms.is_none())
                || (letter.waiting_answer && letter.answer_wait_ended.is_some())
                || (matches!(
                    letter.state,
                    State::Cancelled | State::Superseded | State::NoRecipient | State::Expired
                ) && letter.waiting_answer)
            {
                return Err("ledger_unavailable".into());
            }
        }
        let mut watches = HashSet::new();
        for watch in &self.watches {
            if !watches.insert(&watch.id) || !watch.valid() || !watch.id.starts_with("watch-") {
                return Err("ledger_unavailable".into());
            }
        }
        crate::coordination::validate_records(self)?;
        if self.core_starts.len() > CORE_STARTS {
            return Err("capacity".into());
        }
        if self
            .core_starts
            .iter()
            .any(|start| !super::valid_key(&start.origin) || !super::valid_key(&start.key))
        {
            return Err("ledger_unavailable".into());
        }
        let mut sequences = HashSet::new();
        for id in ids
            .into_iter()
            .chain(watches)
            .chain(self.agents.iter().map(|record| &record.id))
            .chain(self.spawns.iter().map(|record| &record.id))
        {
            let sequence = id
                .split_once('-')
                .and_then(|(_, value)| value.parse::<u64>().ok())
                .ok_or("ledger_unavailable")?;
            if sequence == 0 || sequence >= self.next_id || !sequences.insert(sequence) {
                return Err("ledger_unavailable".into());
            }
        }
        Ok(())
    }

    pub fn cleanup(&mut self, now: u64) -> bool {
        let before = self.letters.len();
        self.letters.retain(|letter| letter.retained(now));
        self.letters.len() != before
    }

    /// Ends each pending letter to the registration `id`, which ended with no
    /// agent after it in its pane (B2): `no_recipient`, unless a hook already
    /// handed it into that agent's session, which ends it `delivered` without
    /// a receipt. A letter with no registration of its own addressed to the
    /// same agent (`actor`) ends with it. A letter an older build
    /// acknowledged without a receipt stops awaiting intake. Returns whether
    /// any letter ended.
    pub(crate) fn end_letters_to(&mut self, id: &str, actor: &Actor, now: u64) -> bool {
        self.end_letters_where(
            |letter| match &letter.to {
                Some(to) => to == id,
                None => letter.recipient.same_identity(actor),
            },
            now,
        )
    }

    /// Ends the letter `id` as [`Ledger::end_letters_to`] ends a letter to an
    /// agent that ended: one sent to an agent that is closed
    /// (`Runtime::closed_target`). Returns whether it ended.
    pub(crate) fn end_letter(&mut self, id: &str, now: u64) -> bool {
        self.end_letters_where(|letter| letter.id == id, now)
    }

    fn end_letters_where(&mut self, ends: impl Fn(&Letter) -> bool, now: u64) -> bool {
        let mut ended = false;
        let mut reports = Vec::new();
        for letter in &mut self.letters {
            if !ends(letter) || !letter.awaiting_intake() {
                continue;
            }
            reports.extend(letter.recipient_ended(now));
            ended = true;
        }
        for (sender, recipient) in reports {
            self.end_report_watch(&sender, &recipient);
        }
        ended
    }

    /// Brings the letters an earlier build kept to the rules letters keep
    /// now (PRD letters-missions-simplify B25): each names the registrations
    /// it is between, where the ledger holds them; a pending letter Hide
    /// wrote as a notice (`notice`), which its agent's status says now
    /// (D-03), ends `expired`; and a pending letter to an agent no live
    /// registration holds ends as it would have had its agent ended then
    /// (`Letter::recipient_ended`). A pending letter to Hide's own role or
    /// the operator keeps waiting. Run once, by the store that reads a ledger
    /// an earlier build wrote (`store::upgrade`).
    pub(crate) fn address_letters(
        &mut self,
        notice: impl Fn(&Actor) -> bool,
        now: u64,
    ) -> Addressed {
        let mut addressed = Addressed::default();
        let agents = &self.agents;
        // The registration `actor` ran as: its live one, else its last.
        let registration = |actor: &Actor| {
            agents
                .iter()
                .find(|record| !record.ended && record.actor.same_identity(actor))
                .or_else(|| {
                    agents
                        .iter()
                        .rev()
                        .find(|record| record.actor.same_identity(actor))
                })
        };
        for letter in &mut self.letters {
            if letter.from.is_none() {
                letter.from = registration(&letter.sender).map(|record| record.id.clone());
            }
            let recipient = registration(&letter.recipient);
            if letter.to.is_none() {
                letter.to = recipient.map(|record| record.id.clone());
            }
            if letter.state != State::Pending {
                continue;
            }
            if notice(&letter.sender) {
                letter.end(State::Expired, now);
                addressed.expired += 1;
            } else if !letter.recipient.code_owned()
                && !recipient.is_some_and(|record| !record.ended)
            {
                letter.recipient_ended(now);
                addressed.no_recipient += 1;
            }
        }
        addressed
    }

    /// Ends the watch `recipient` keeps on `sender`, whose report it took.
    pub(crate) fn end_report_watch(&mut self, sender: &Actor, recipient: &Actor) {
        self.watches.retain(|watch| {
            !(sender.same_identity(&watch.target) && recipient.same_identity(&watch.parent))
        });
    }

    /// Ends the watch the parent of registration `agent` keeps on it, as a
    /// confirmed report from it would (`end_report_watch`): a mission
    /// checker's record of its line is its report
    /// (`Runtime::checker_reported`). Returns whether a watch ended.
    pub(crate) fn end_parent_watch(&mut self, agent: &str) -> bool {
        let Some(child) = self
            .agents
            .iter()
            .find(|record| record.id == agent && !record.ended)
        else {
            return false;
        };
        let Some(parent) = child
            .parent
            .as_ref()
            .and_then(|parent| self.agents.iter().find(|record| &record.id == parent))
        else {
            return false;
        };
        let (sender, recipient) = (child.actor.clone(), parent.actor.clone());
        let before = self.watches.len();
        self.end_report_watch(&sender, &recipient);
        self.watches.len() != before
    }

    /// The core on connection `key` took letter `id`, which this core's
    /// agent wrote to an agent of that core (PRD device-connect-iroh D-72):
    /// the letter is delivered, and its recipient takes `name`, what that
    /// agent is called there, in place of what its sender called it. Who the
    /// recipient is does not change, so the same send asked again is the
    /// same letter. Only a letter still waiting to be handed over on that
    /// connection moves, so an answer that comes twice, or from another
    /// machine, changes nothing. A refusal ends the letter another way
    /// ([`Ledger::refuse_core`]). Returns whether it moved.
    pub fn settle_core(&mut self, id: &str, key: &str, name: &str, now: u64) -> bool {
        let Some(letter) = self.letters.iter_mut().find(|letter| {
            letter.id == id && letter.state == State::Pending && letter.recipient.core_of(key)
        }) else {
            return false;
        };
        if super::valid_key(name) {
            letter.recipient.name = name.to_owned();
        }
        letter.hook_confirmed = Some(true);
        letter.state = State::Delivered;
        if !letter.open() {
            letter.finished_at_unix_ms = Some(now);
        }
        true
    }

    /// The core on connection `key` keeps nothing of letter `id`, which
    /// this core's agent wrote to an agent of that core, and says why: the
    /// letter ends `no_recipient`, as one whose agent ended with no agent
    /// after it does, and keeps the other core's `reason` where a failed
    /// bell's is kept (`Letter::bell_failure`), so its sender reads it with
    /// the letter in `hide inbox` and `hide request show`. Only a letter
    /// still waiting to be handed over on that connection ends. Returns
    /// whether it ended.
    pub fn refuse_core(&mut self, id: &str, key: &str, reason: &str, now: u64) -> bool {
        let Some(letter) = self.letters.iter_mut().find(|letter| {
            letter.id == id && letter.state == State::Pending && letter.recipient.core_of(key)
        }) else {
            return false;
        };
        letter.end(State::NoRecipient, now);
        if super::valid_key(reason) {
            letter.bell_failure = Some(BellFailure {
                code: reason.to_owned(),
                maybe_typed: false,
                at_unix_ms: now,
            });
        }
        crate::diagnostic!(serde_json::json!({
            "component": "delivery", "kind": "letter.no_recipient",
            "letter_id": letter.id, "pane_id": letter.recipient.pane_id, "reason": reason,
        }));
        true
    }

    /// Ends each letter awaiting intake by an identity `gone` names, as
    /// [`Ledger::end_letters_to`] ends the letters of a registration that
    /// ended: the agents of a machine this core is paired with no longer
    /// (`coordination::end_gone_cores`). Returns whether any ended.
    pub(crate) fn end_letters_of(&mut self, gone: impl Fn(&Actor) -> bool, now: u64) -> bool {
        self.end_letters_where(|letter| gone(&letter.recipient), now)
    }

    /// Stamps letter `id`, which an agent of another core sent, with the
    /// id its sender's core gives it (`Letter::core_id`). Returns whether
    /// the ledger changed.
    pub fn name_core_letter(&mut self, id: &str, core_id: &str) -> bool {
        match self.letters.iter_mut().find(|letter| letter.id == id) {
            Some(letter) if letter.core_id.as_deref() != Some(core_id) => {
                letter.core_id = Some(core_id.to_owned());
                true
            }
            _ => false,
        }
    }

    /// Forgets the starts asked of connections not among `keys`, which
    /// this core is paired with no longer. Returns whether any went.
    pub fn forget_core_starts(&mut self, keys: &[String]) -> bool {
        let before = self.core_starts.len();
        self.core_starts.retain(|start| keys.contains(&start.key));
        self.core_starts.len() != before
    }

    /// Remembers that the registration `origin` asked the core on
    /// connection `key` for a start ([`CoreStart`]), before the ask leaves.
    /// One record per asker and connection: asked again, the record takes
    /// the newest place and time. At [`CORE_STARTS`] the oldest record whose
    /// asker ended goes; while the asker of every record may still take its
    /// report, the new start is refused `capacity` and nothing is dropped.
    pub fn note_core_start(
        &mut self,
        origin: &str,
        key: &str,
        now: u64,
    ) -> Result<(), &'static str> {
        if let Some(at) = self
            .core_starts
            .iter()
            .position(|start| start.key == key && start.origin == origin)
        {
            let mut start = self.core_starts.remove(at);
            start.at_unix_ms = now;
            self.core_starts.push(start);
            return Ok(());
        }
        if self.core_starts.len() >= CORE_STARTS {
            let finished = self
                .core_starts
                .iter()
                .position(|start| !self.core_start_open(start))
                .ok_or("capacity")?;
            let dropped = self.core_starts.remove(finished);
            crate::diagnostic!(serde_json::json!({
                "component": "delivery", "kind": "core_start.dropped",
                "agent_id": dropped.origin,
            }));
        }
        self.core_starts.push(CoreStart {
            origin: origin.to_owned(),
            key: key.to_owned(),
            at_unix_ms: now,
        });
        Ok(())
    }

    /// Whether the report of `start` may still reach its asker: its
    /// registration is live. The agent that takes an asker's place takes
    /// its starts ([`Self::hand_core_starts`]).
    fn core_start_open(&self, start: &CoreStart) -> bool {
        self.agents
            .iter()
            .any(|record| record.id == start.origin && !record.ended)
    }

    /// The starts registration `old` asked for are `successor`'s from now
    /// on, one per connection: where both asked the same machine, the
    /// successor's own record stands.
    pub(crate) fn hand_core_starts(&mut self, old: &str, successor: &str) {
        for at in (0..self.core_starts.len()).rev() {
            if self.core_starts[at].origin != old {
                continue;
            }
            let asked = self
                .core_starts
                .iter()
                .any(|start| start.origin == successor && start.key == self.core_starts[at].key);
            if asked {
                self.core_starts.remove(at);
            } else {
                successor.clone_into(&mut self.core_starts[at].origin);
            }
        }
    }

    /// Ends the answer wait of every letter `actor` sent or received and the
    /// recipient took in, because its registration ended. A letter still
    /// awaiting intake keeps its wait: one to the ended agent ends with it
    /// (`end_letters_to`), and one from it still reaches its recipient.
    /// Returns whether any wait ended.
    pub(crate) fn end_answer_waits_of(&mut self, actor: &Actor, now: u64) -> bool {
        let mut ended = false;
        for letter in &mut self.letters {
            if letter.waiting_answer
                && !letter.awaiting_intake()
                && (letter.sender.same_identity(actor) || letter.recipient.same_identity(actor))
            {
                letter.end_answer_wait(AnswerWaitEnd::PartyEnded, now);
                ended = true;
            }
        }
        ended
    }

    /// Ends the answer wait of every letter sent `ANSWER_WAIT_MS` ago.
    pub fn end_overdue_answer_waits(&mut self, now: u64) -> bool {
        let mut ended = false;
        for letter in &mut self.letters {
            if letter.answer_overdue(now) {
                letter.end_answer_wait(AnswerWaitEnd::Deadline, now);
                ended = true;
            }
        }
        ended
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| "ledger_unavailable".to_owned())?;
        if bytes.len() > FILE_LIMIT {
            return Err("capacity".into());
        }
        Ok(bytes)
    }

    /// The check [`Self::bytes`] makes, counting the encoding instead of
    /// keeping it.
    pub fn check(&self) -> Result<(), String> {
        self.validate()?;
        match serde_json::to_writer(&mut Length::up_to(FILE_LIMIT), self) {
            Ok(()) => Ok(()),
            Err(error) if error.is_io() => Err("capacity".into()),
            Err(_) => Err("ledger_unavailable".into()),
        }
    }

    /// The length of the file [`Self::bytes`] would write, past the limit
    /// too, counted without keeping it, so it can follow every request of a
    /// batch and say how much room a full ledger has to make.
    pub(crate) fn size(&self) -> Result<usize, String> {
        self.validate()?;
        encoded_len(self)
    }
}

/// The length of `value`'s encoding in the file, counted without keeping it.
pub(crate) fn encoded_len(value: &impl Serialize) -> Result<usize, String> {
    let mut length = Length::up_to(usize::MAX);
    serde_json::to_writer(&mut length, value).map_err(|_| "ledger_unavailable")?;
    Ok(length.written)
}

/// Counts the bytes written to it and refuses those past `limit`.
struct Length {
    written: usize,
    limit: usize,
}

impl Length {
    fn up_to(limit: usize) -> Self {
        Self { written: 0, limit }
    }
}

impl std::io::Write for Length {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.written += bytes.len();
        if self.written > self.limit {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The ledger in `delivery-ledger.json` at `path`, where builds before the
/// store kept it, as such a build loads it at its start: an absent file is
/// an empty ledger. An older build started on a folder this build has used
/// reads it so (B27), since this build only ever reads it.
pub fn load(path: &Path) -> Result<Ledger, String> {
    Ok(read_file(path)?.unwrap_or_default())
}

/// The ledger `delivery-ledger.json` at `path` holds, as it holds it; `None`
/// when there is no such file. What the store imports (`delivery::store`).
pub(crate) fn read_file(path: &Path) -> Result<Option<Ledger>, String> {
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("ledger_unavailable".into()),
    };
    if !hide_platform::fs::private::is_private(path).unwrap_or(false)
        || file.metadata().map_err(|_| "ledger_unavailable")?.len() > FILE_LIMIT as u64
    {
        return Err("ledger_unavailable".into());
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "ledger_unavailable")?;
    from_bytes(&bytes).map(Some)
}

/// The ledger `bytes` hold, under the rules `delivery-ledger.json` was loaded
/// with: within the size limit, a ledger of this build's shape, and valid.
pub fn from_bytes(bytes: &[u8]) -> Result<Ledger, String> {
    if bytes.len() > FILE_LIMIT {
        return Err("ledger_unavailable".into());
    }
    let ledger: Ledger = serde_json::from_slice(bytes).map_err(|_| "ledger_unavailable")?;
    ledger.validate().map_err(|_| "ledger_unavailable")?;
    Ok(ledger)
}

/// Why the store did not keep a ledger. A write the store refused left the
/// file as it was; one whose commit failed may or may not have been kept, so
/// the core stops answering from memory until a start reads the file again.
#[derive(Debug)]
pub enum SaveError {
    /// Refused before anything was written: not a valid ledger, or past
    /// its size limit.
    Validation(String),
    /// The store refused the write and rolled it back.
    Refused(StoreFailure),
    /// The commit failed: the rows may or may not be on the disk.
    Uncertain(StoreFailure),
    /// The state folder could not be made at the start.
    Bootstrap(hide_platform::fs::private::DirectoryDurabilityError),
}

/// What SQLite answered a failed write with: its kind and extended code,
/// never a path or content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreFailure {
    pub kind: &'static str,
    pub extended: Option<i32>,
}

impl StoreFailure {
    pub(crate) fn of(error: &rusqlite::Error) -> Self {
        Self {
            kind: super::store::code(error),
            extended: error.sqlite_error().map(|error| error.extended_code),
        }
    }
}

impl SaveError {
    pub fn code(&self) -> &str {
        match self {
            Self::Validation(code) => code,
            Self::Refused(_) | Self::Uncertain(_) | Self::Bootstrap(_) => "ledger_unavailable",
        }
    }

    pub fn uncertain(&self) -> bool {
        matches!(
            self,
            Self::Uncertain(_)
                | Self::Bootstrap(
                    hide_platform::fs::private::DirectoryDurabilityError::Uncertain { .. }
                )
        )
    }

    pub fn diagnostic(&self) -> serde_json::Value {
        match self {
            Self::Validation(code) => serde_json::json!({"phase":"validation","code":code}),
            Self::Refused(failure) => serde_json::json!({"phase":"store_refused",
                "sqlite":failure.kind,"extended":failure.extended}),
            Self::Uncertain(failure) => serde_json::json!({"phase":"commit_uncertain",
                "sqlite":failure.kind,"extended":failure.extended}),
            Self::Bootstrap(error) => {
                use hide_platform::fs::private::DirectoryDurabilityError;
                let (phase, source) = match error {
                    DirectoryDurabilityError::BeforeCreate { source } => {
                        ("bootstrap_refused", source)
                    }
                    DirectoryDurabilityError::Uncertain { source, .. } => {
                        ("bootstrap_uncertain", source)
                    }
                };
                // IO text can contain private directory names.
                serde_json::json!({"phase":phase,"io_kind":format!("{:?}",source.kind())})
            }
        }
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bootstrap(source) => Some(source),
            Self::Validation(_) | Self::Refused(_) | Self::Uncertain(_) => None,
        }
    }
}

/// Startup runs before Runtime is placed behind its mutex: makes the state
/// folder when it is missing, opens the store at `path` (importing
/// `delivery-ledger.json` the first time, `delivery::store`), so the file
/// and the published ledger agree before any effect. A store the core cannot read is kept as it is,
/// never repaired by guess.
pub fn recover(path: &Path) -> Result<Ledger, SaveError> {
    let refused = || SaveError::Validation("ledger_unavailable".into());
    let parent = path.parent().ok_or_else(refused)?;
    if !path.is_absolute()
        || path.components().count() > 66
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(refused());
    }
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) => {
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || !hide_platform::fs::private::is_private(parent).map_err(|_| refused())?
            {
                return Err(refused());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    let mut ancestor = parent;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if missing.len() == 64 {
                    return Err(refused());
                }
                missing.push(ancestor.file_name().ok_or_else(refused)?);
                ancestor = ancestor.parent().ok_or_else(refused)?;
            }
            Err(_) => return Err(refused()),
        }
    }
    let anchor = hide_platform::fs::private::establish_dir_durability(ancestor)
        .map_err(SaveError::Bootstrap)?;
    let mut canonical_parent = anchor.clone();
    for part in missing.into_iter().rev() {
        canonical_parent.push(part);
    }
    hide_platform::fs::private::create_dir_all_durable(&canonical_parent, &anchor)
        .map_err(SaveError::Bootstrap)?;
    if !hide_platform::fs::private::is_private(&canonical_parent).map_err(|_| refused())? {
        return Err(refused());
    }
    let (_, stored, _) = super::store::LedgerStore::open(path).map_err(SaveError::Validation)?;
    // The file is named in its folder on the disk before any letter is
    // answered from it, whether this open made it or a migration earlier in
    // this start did (`node_migration::read_store`).
    hide_platform::fs::private::establish_dir_durability(&canonical_parent)
        .map_err(SaveError::Bootstrap)?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeps `ledger` in the store at `path`, as the store thread does.
    fn save(path: &Path, ledger: &Ledger) -> Result<(), SaveError> {
        super::super::store::save(path, ledger)
    }

    /// The ledger a core starting on the store at `path` begins with
    /// (`recover`, without its checks of the state folder).
    fn load(path: &Path) -> Result<Ledger, String> {
        super::super::store::load(path)
    }

    /// PRD device-connect-iroh D-72: at the cap only a start whose asker
    /// ended goes, the oldest such first. While every asker may still take
    /// its report a new start is refused and the oldest record, a long
    /// mission's, keeps its place; a start asked again takes the newest
    /// place and time and needs no room.
    #[test]
    fn the_remembered_starts_drop_only_ended_askers_and_refuse_when_none_ended() {
        let key = "a".repeat(64);
        let asker = |n: usize| format!("agent-{}", n + 1);
        let holds = |ledger: &Ledger, n: usize| {
            ledger
                .core_starts
                .iter()
                .any(|start| start.origin == asker(n))
        };
        let mut ledger = Ledger::default();
        for n in 0..=CORE_STARTS {
            let actor = Actor {
                pane_id: format!("w1:p{n}"),
                name: format!("asker-{n}"),
                kind: "codex".into(),
                device_id: "local".into(),
                session: crate::engine::tokens::session_digest(&format!("asker-{n}")),
            };
            ledger.agents.push(crate::coordination::AgentRecord {
                id: asker(n),
                name: Some(actor.name.clone()),
                machine: "local".into(),
                host_scope: "scope".into(),
                native_machine: "machine".into(),
                session: format!("asker-{n}"),
                instance: format!("terminal-{n}"),
                pane: actor.pane_id.clone(),
                parent: None,
                origin: None,
                project: None,
                actor,
                ended: false,
                terminal_id: None,
                forked: false,
                successor: None,
            });
        }
        for n in 0..CORE_STARTS {
            ledger.note_core_start(&asker(n), &key, n as u64).unwrap();
        }
        assert_eq!(
            ledger.note_core_start(&asker(CORE_STARTS), &key, 999),
            Err("capacity")
        );
        assert_eq!(ledger.core_starts.len(), CORE_STARTS);
        assert_eq!(ledger.core_starts[0].origin, asker(0));

        ledger.note_core_start(&asker(0), &key, 1_000).unwrap();
        assert_eq!(ledger.core_starts.len(), CORE_STARTS);
        let newest = ledger.core_starts.last().unwrap();
        assert!(newest.origin == asker(0) && newest.at_unix_ms == 1_000);
        // The same asker on another connection is another record.
        assert_eq!(
            ledger.note_core_start(&asker(0), &"b".repeat(64), 1_000),
            Err("capacity")
        );

        // Askers 9 and 5 ended: the older record of the two goes for the
        // new start, and no living asker's does.
        for n in [9, 5] {
            ledger.agents[n].ended = true;
        }
        ledger
            .note_core_start(&asker(CORE_STARTS), &key, 1_001)
            .unwrap();
        assert_eq!(ledger.core_starts.len(), CORE_STARTS);
        assert!(!holds(&ledger, 5) && holds(&ledger, 9));
        assert!(holds(&ledger, 1) && holds(&ledger, CORE_STARTS));

        // The agent that takes asker 7's place takes its start, and where
        // both asked the same machine one record stands.
        ledger.hand_core_starts(&asker(7), "agent-next");
        assert!(!holds(&ledger, 7));
        assert!(
            ledger
                .core_starts
                .iter()
                .any(|start| start.origin == "agent-next")
        );
        ledger.hand_core_starts(&asker(1), "agent-next");
        assert_eq!(ledger.core_starts.len(), CORE_STARTS - 1);
        assert!(!holds(&ledger, 1));
    }

    /// PRD device-connect-iroh D-72: a letter to another core's agent waits,
    /// with no deadline, until the core on its connection answers. Taken, it
    /// is delivered under the name the agent has there; refused, it ends
    /// with no recipient and keeps the other core's reason. An answer from
    /// another connection, or a second one, moves nothing.
    #[test]
    fn a_letter_to_another_cores_agent_ends_by_what_the_core_on_its_connection_answers() {
        let address =
            crate::address::CoreAddress::new("mini", crate::address::CoreItem::Agent, "agent-7");
        let (mini, studio) = ("a".repeat(64), "b".repeat(64));
        let there = Actor::core(address, &mini, "Mac mini", "agent-7", "local");
        let to_core = || {
            let mut ledger = letter();
            ledger.letters[0].recipient = there.clone();
            ledger
        };

        let mut ledger = to_core();
        assert!(!ledger.settle_core("letter-1", &studio, "hide:core:Mac mini:worker", 5));
        assert!(ledger.settle_core("letter-1", &mini, "hide:core:Mac mini:worker", 5));
        let letter = &ledger.letters[0];
        assert_eq!(letter.state, State::Delivered);
        assert_eq!(letter.recipient.name, "hide:core:Mac mini:worker");
        assert!(letter.recipient.core_of(&mini), "the same agent");
        assert!(letter.waiting_answer && letter.finished_at_unix_ms.is_none());
        assert!(!ledger.settle_core("letter-1", &mini, "hide:core:Mac mini:other", 6));
        assert!(!ledger.refuse_core("letter-1", &mini, "target_unavailable", 6));
        ledger.validate().unwrap();

        let mut ledger = to_core();
        assert!(!ledger.refuse_core("letter-1", &studio, "target_unavailable", 7));
        assert!(ledger.refuse_core("letter-1", &mini, "target_unavailable", 7));
        let letter = &ledger.letters[0];
        assert_eq!(letter.state, State::NoRecipient);
        assert!(!letter.waiting_answer);
        assert_eq!(letter.finished_at_unix_ms, Some(7));
        assert_eq!(
            letter.bell_failure.as_ref().map(|why| why.code.as_str()),
            Some("target_unavailable")
        );
        assert!(!ledger.settle_core("letter-1", &mini, "hide:core:Mac mini:worker", 8));
        ledger.validate().unwrap();
    }

    fn letter() -> Ledger {
        let mut ledger = Ledger::default();
        let sender = Actor {
            pane_id: "sender".into(),
            name: "sender".into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some("sender-native".into()),
        };
        let recipient = Actor {
            pane_id: "recipient".into(),
            name: "recipient".into(),
            kind: "codex".into(),
            device_id: "local".into(),
            session: Some("recipient-native".into()),
        };
        super::super::mailbox::send(
            &mut ledger,
            &sender,
            &recipient,
            "once",
            "private",
            "request",
            None,
            1,
        )
        .unwrap();
        ledger
    }

    #[test]
    fn total_doorbell_reservations_include_success_and_survive_restart_before_confirmation() {
        for sent in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("delivery.sqlite3");
            let mut ledger = letter();
            for attempt in 1..=3 {
                let line = format!("🦉 sender 보고: 시도 {attempt}");
                assert_eq!(
                    ledger.letters[0].reserve_bell(line.clone()).unwrap(),
                    attempt
                );
                save(&path, &ledger).unwrap();
                // A lost response or a crash here still consumes the
                // reservation, and the line it saved is the one typed.
                ledger = load(&path).unwrap();
                assert_eq!(ledger.letters[0].attempts(), attempt);
                assert_eq!(ledger.letters[0].bell_line.as_deref(), Some(line.as_str()));
                assert_eq!(ledger.letters[0].state, State::Pending);
                if sent {
                    ledger.letters[0].bell_sent = true;
                } else {
                    ledger.letters[0].bell_errors += 1;
                }
                save(&path, &ledger).unwrap();
                ledger = load(&path).unwrap();
            }
            let before = ledger.bytes().unwrap();
            assert_eq!(
                ledger.letters[0]
                    .reserve_bell("🦉 a 보고: b".into())
                    .unwrap_err(),
                "doorbell_attempt_limit"
            );
            assert_eq!(ledger.bytes().unwrap(), before);
            let recipient = ledger.letters[0].recipient.clone();
            let id = ledger.letters[0].id.clone();
            super::super::mailbox::apply(
                &mut ledger,
                &recipient,
                None,
                &super::super::Command::Confirm { ids: vec![id] },
                2,
            )
            .unwrap();
            save(&path, &ledger).unwrap();
            ledger = load(&path).unwrap();
            assert_eq!(
                ledger.letters[0]
                    .reserve_bell("🦉 a 보고: b".into())
                    .unwrap_err(),
                "letter_not_pending"
            );
            assert!(
                super::super::mailbox::pull(&ledger, &recipient)
                    .unwrap()
                    .ids
                    .is_empty()
            );
        }
    }

    #[test]
    fn legacy_success_does_not_infer_spare_attempts_and_invalid_budget_is_rejected() {
        let ledger = letter();
        for sent in [true, false] {
            let mut record = serde_json::to_value(&ledger).unwrap();
            let row = record["letters"][0].as_object_mut().unwrap();
            row.remove("bell_attempts");
            row.remove("bell_line");
            row.insert("bell_sent".into(), serde_json::json!(sent));
            row.insert("bell_errors".into(), serde_json::json!(2));
            let mut restored: Ledger = serde_json::from_value(record).unwrap();
            if sent {
                assert_eq!(
                    restored.letters[0]
                        .reserve_bell("🦉 a 보고: b".into())
                        .unwrap_err(),
                    "doorbell_attempt_limit"
                );
            } else {
                assert_eq!(
                    restored.letters[0]
                        .reserve_bell("🦉 a 보고: b".into())
                        .unwrap(),
                    3
                );
                assert_eq!(
                    restored.letters[0]
                        .reserve_bell("🦉 a 보고: b".into())
                        .unwrap_err(),
                    "doorbell_attempt_limit"
                );
            }
        }
        let mut invalid = ledger;
        assert_eq!(
            invalid.letters[0]
                .reserve_bell("🦉 a\n보고".into())
                .unwrap_err(),
            "bell_line_invalid"
        );
        invalid.letters[0].bell_line = Some("🦉 a\n보고".into());
        assert_eq!(invalid.validate().unwrap_err(), "ledger_unavailable");
        invalid.letters[0].bell_line = None;
        invalid.letters[0].bell_attempts = Some(4);
        assert_eq!(invalid.validate().unwrap_err(), "ledger_unavailable");
    }

    #[test]
    fn acknowledgement_receipt_survives_durable_reload_and_replay_without_closing_new_watch() {
        use super::super::{mailbox, watch};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("delivery.sqlite3");
        let mut ledger = letter();
        ledger.letters[0].kind = "report".into();
        ledger.letters[0].waiting_answer = false;
        let child = ledger.letters[0].sender.clone();
        let parent = ledger.letters[0].recipient.clone();
        let id = ledger.letters[0].id.clone();
        watch::start(&mut ledger, &parent, &child, 2).unwrap();
        let intake = mailbox::pull(&ledger, &parent).unwrap();
        mailbox::apply(
            &mut ledger,
            &parent,
            None,
            &mailbox::Command::Ack { id: id.clone() },
            3,
        )
        .unwrap();
        save(&path, &ledger).unwrap();
        let mut restored = load(&path).unwrap();
        assert!(restored.watches.is_empty());
        assert!(mailbox::pull(&restored, &parent).unwrap().ids.is_empty());
        let shown = mailbox::apply(
            &mut restored,
            &parent,
            None,
            &mailbox::Command::Show { id },
            4,
        )
        .unwrap();
        assert_eq!(shown["state"], "acknowledged");
        assert_eq!(shown["hook_confirmed"], true);
        watch::start(&mut restored, &parent, &child, 5).unwrap();
        save(&path, &restored).unwrap();
        let mut restored = load(&path).unwrap();
        let before = restored.clone();
        let confirm = mailbox::Command::Confirm { ids: intake.ids };
        mailbox::apply(&mut restored, &parent, None, &confirm, 6).unwrap();
        save(&path, &restored).unwrap();
        assert_eq!(load(&path).unwrap(), before);
    }

    #[test]
    fn legacy_delivered_proves_intake_but_legacy_ack_stays_closed_without_delivery_proof() {
        use super::super::{mailbox, watch};
        for state in [State::Delivered, State::Acknowledged] {
            let mut fixture = serde_json::to_value(letter()).unwrap();
            let row = fixture["letters"][0].as_object_mut().unwrap();
            row.remove("hook_confirmed");
            row.insert("state".into(), serde_json::to_value(state).unwrap());
            row.insert("kind".into(), serde_json::json!("report"));
            row.insert("waiting_answer".into(), serde_json::json!(false));
            row.insert("finished_at_unix_ms".into(), serde_json::json!(2));
            let mut restored: Ledger = serde_json::from_value(fixture).unwrap();
            restored.validate().unwrap();
            let explicit_null: Ledger =
                serde_json::from_value(serde_json::to_value(&restored).unwrap()).unwrap();
            assert_eq!(explicit_null, restored);
            let child = restored.letters[0].sender.clone();
            let parent = restored.letters[0].recipient.clone();
            let id = restored.letters[0].id.clone();
            let watched = watch::start(&mut restored, &parent, &child, 3).unwrap();
            let shown = mailbox::apply(
                &mut restored,
                &parent,
                None,
                &mailbox::Command::Show { id: id.clone() },
                4,
            )
            .unwrap();
            assert!(shown["hook_confirmed"].is_null());
            assert!(!restored.letters[0].open());
            assert!(mailbox::pull(&restored, &parent).unwrap().ids.is_empty());
            let mut retained = restored.clone();
            assert!(retained.cleanup(RETENTION_MS + 3));
            assert!(retained.letters.is_empty());
            assert_eq!(retained.watches[0].id, watched.id);
            if state == State::Delivered {
                assert!(restored.letters[0].intake_confirmed());
                mailbox::apply(
                    &mut restored,
                    &parent,
                    None,
                    &mailbox::Command::Confirm { ids: vec![id] },
                    5,
                )
                .unwrap();
                assert_eq!(restored.watches[0].id, watched.id);
                assert!(restored.letters[0].hook_confirmed.is_none());
            } else {
                assert!(!restored.letters[0].intake_confirmed());
                assert_eq!(restored.watches[0].id, watched.id);
                mailbox::apply(
                    &mut restored,
                    &parent,
                    None,
                    &mailbox::Command::Confirm { ids: vec![id] },
                    5,
                )
                .unwrap();
                assert!(restored.watches.is_empty());
                assert_eq!(restored.letters[0].hook_confirmed, Some(true));
            }
        }
        for (state, receipt) in [
            (State::Pending, true),
            (State::NoRecipient, true),
            (State::Expired, true),
            (State::Delivered, false),
        ] {
            let mut invalid = letter();
            invalid.letters[0].state = state;
            invalid.letters[0].hook_confirmed = Some(receipt);
            invalid.letters[0].waiting_answer = false;
            invalid.letters[0].finished_at_unix_ms = Some(2);
            assert_eq!(invalid.validate().unwrap_err(), "ledger_unavailable");
        }
        for receipt in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::json!({}),
        ] {
            let mut invalid = serde_json::to_value(letter()).unwrap();
            invalid["letters"][0]["hook_confirmed"] = receipt;
            assert!(serde_json::from_value::<Ledger>(invalid).is_err());
        }
    }

    #[test]
    fn validated_startup_reestablishes_the_installed_version_without_repairing_corruption() {
        let root = tempfile::tempdir().unwrap();
        // A tempfile root uses the normal directory default; the product
        // stores its ledger inside a private state directory.
        let state = root.path().join("state");
        hide_platform::fs::private::create_dir(&state).unwrap();
        let path = state.join("delivery.sqlite3");
        let ledger = letter();
        save(&path, &ledger).unwrap();
        assert_eq!(recover(&path).unwrap(), ledger);
        std::fs::write(&path, b"private damaged bytes").unwrap();
        assert_eq!(recover(&path).unwrap_err().code(), "ledger_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"private damaged bytes");
        // A missing parent is not implicitly bootstrapped by a write.
        let missing = root.path().join("absent/delivery.sqlite3");
        let error = save(&missing, &Ledger::default()).unwrap_err();
        assert!(!error.uncertain());
        assert!(!missing.parent().unwrap().exists());
        assert_eq!(error.diagnostic()["phase"], "validation");
        assert!(
            !error
                .diagnostic()
                .to_string()
                .contains(&root.path().display().to_string())
        );
    }

    #[test]
    fn startup_bootstraps_private_sibling_state_and_preserves_refused_or_corrupt_bytes() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        hide_platform::fs::private::create_dir(&home).unwrap();
        let state = root.path().join("new/state");
        let path = state.join("delivery.sqlite3");
        assert_eq!(recover(&path).unwrap(), Ledger::default());
        assert!(hide_platform::fs::private::is_private(&state).unwrap());
        assert!(hide_platform::fs::private::is_private(&path).unwrap());
        assert!(home.is_dir());
        std::fs::write(&path, b"private corruption").unwrap();
        assert_eq!(recover(&path).unwrap_err().code(), "ledger_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"private corruption");
        #[cfg(unix)]
        {
            let link = root.path().join("linked-state");
            std::os::unix::fs::symlink(&state, &link).unwrap();
            assert_eq!(
                recover(&link.join("delivery.sqlite3")).unwrap_err().code(),
                "ledger_unavailable"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"private corruption");
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[test]
    fn durable_ledger_is_private_and_corruption_is_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("delivery.sqlite3");
        save(&path, &Ledger::default()).unwrap();
        assert!(hide_platform::fs::private::is_private(&path).unwrap());
        assert_eq!(load(&path).unwrap(), Ledger::default());
        std::fs::write(&path, b"broken ledger").unwrap();
        assert_eq!(load(&path).unwrap_err(), "ledger_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"broken ledger");
    }
}
