use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Component, Path};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::watch::Watch;
use super::{
    ANSWER_WAIT_MS, Actor, BODY_LIMIT, DELIVERY_EXPIRY_MS, FILE_LIMIT, LETTER_LIMIT, OPEN_LIMIT,
    RETENTION_MS, WATCH_LIMIT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Delivered,
    Acknowledged,
    Cancelled,
    Expired,
    Undelivered,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Letter {
    pub id: String,
    pub intent: String,
    pub sender: Actor,
    pub recipient: Actor,
    pub kind: String,
    pub body: String,
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
    /// submitted prompt is that bell exactly when it is this line.
    #[serde(default)]
    pub bell_line: Option<String>,
    /// Where the letter's wait for intake began: on its recipient machine's
    /// clock while the ledger keeps one (`Ledger::node_clocks`), on the wall
    /// clock otherwise; null is its creation on the wall clock.
    #[serde(default)]
    pub intake_from: Option<u64>,
    /// Sent with `hide request send --now`: delivered at the recipient's
    /// next safe gap while it works, without the mid-turn wait (PRD
    /// letters-to-busy-agents D-05). An older build drops the field at its
    /// next save, and the letter then waits as any other.
    #[serde(default)]
    pub immediate: bool,
    /// How many tool events handed the letter over while its recipient
    /// worked (PRD letters-to-busy-agents D-11): a handover is confirmed by
    /// the script once the host kept it, and after three unconfirmed ones
    /// the letter waits for the next prompt or `hide inbox`, as it does
    /// after three bells.
    #[serde(default)]
    pub tool_handovers: u8,
}

/// How long one machine other than this core's own has been reachable, which
/// the delivery deadline of a letter to an agent there counts instead of wall
/// time (PRD core-host-node-move B13, D-11).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeClock {
    /// Reachable time in ended stretches.
    pub connected_ms: u64,
    /// When the stretch running now began; null while the machine is not
    /// reachable.
    #[serde(default)]
    pub since: Option<u64>,
}

impl NodeClock {
    pub(crate) fn reading(&self, now: u64) -> u64 {
        self.connected_ms
            .saturating_add(self.since.map_or(0, |since| now.saturating_sub(since)))
    }
}

/// The machines other than this core's own that the delivery store hears of,
/// each with whether a letter can reach it now (PRD core-host-node-move B13).
pub type Reach = BTreeMap<String, bool>;

/// How often a reachable machine's running stretch is written into its clock,
/// which bounds what a core that stops without a last save loses of it.
pub const CLOCK_FOLD_MS: u64 = 60_000;

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
}

impl Letter {
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
        // may have preceded intake, so its missing receipt remains unknown.
        self.hook_confirmed
            .unwrap_or(self.state == State::Delivered)
    }

    pub(crate) fn awaiting_intake(&self) -> bool {
        self.state == State::Pending
            || (self.state == State::Acknowledged && self.hook_confirmed == Some(false))
    }

    /// How long the letter has waited for intake at `now`: the time its
    /// recipient's machine was reachable since then, while `clocks` keeps one
    /// for it, and wall time otherwise (this core's own machine).
    pub(crate) fn intake_waited(&self, clocks: &BTreeMap<String, NodeClock>, now: u64) -> u64 {
        match clocks.get(&self.recipient.device_id) {
            Some(clock) => clock
                .reading(now)
                .saturating_sub(self.intake_from.unwrap_or_default()),
            None => now.saturating_sub(self.intake_from.unwrap_or(self.created_at_unix_ms)),
        }
    }

    /// Whether the letter is still within its delivery deadline at `now`.
    pub(crate) fn within_deadline(&self, clocks: &BTreeMap<String, NodeClock>, now: u64) -> bool {
        self.intake_waited(clocks, now) < DELIVERY_EXPIRY_MS
    }

    /// Whether the delivery deadline passed while the letter still waits for
    /// intake; `Ledger::expire` ends that wait.
    pub(crate) fn intake_overdue(&self, clocks: &BTreeMap<String, NodeClock>, now: u64) -> bool {
        self.awaiting_intake() && !self.within_deadline(clocks, now)
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

    pub(crate) fn attempts(&self) -> u8 {
        // Legacy successes have an unknown total. Do not infer spare attempts
        // from the old boolean and repeat an already unbounded external effect.
        self.bell_attempts
            .unwrap_or(if self.bell_sent { 3 } else { self.bell_errors })
    }

    /// Spends one of the letter's three bells on `line`, which the doorbell
    /// types once this reservation is saved.
    pub(crate) fn reserve_bell(
        &mut self,
        clocks: &BTreeMap<String, NodeClock>,
        now: u64,
        line: String,
    ) -> Result<u8, String> {
        self.recipient.require_native_identity()?;
        if !super::bell::valid_line(&line) {
            return Err("bell_line_invalid".into());
        }
        if self.state != State::Pending || !self.within_deadline(clocks, now) {
            return Err("letter_not_pending".into());
        }
        if self.attempts() >= 3 {
            return Err("doorbell_attempt_limit".into());
        }
        let attempt = self.attempts() + 1;
        self.bell_attempts = Some(attempt);
        self.bell_line = Some(line);
        Ok(attempt)
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
    /// The clock of each machine other than this core's own that a letter
    /// waits on, by its registration id; a machine without one, this core's
    /// own among them, is waited on in wall time. One per machine with a
    /// letter awaiting intake, so the letters bound them.
    #[serde(default)]
    pub node_clocks: BTreeMap<String, NodeClock>,
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
            node_clocks: BTreeMap::new(),
        }
    }
}

impl Ledger {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 || self.next_id == 0 {
            return Err("ledger_unavailable".into());
        }
        if self.letters.len() > LETTER_LIMIT
            || self.letters.iter().filter(|letter| letter.open()).count() > OPEN_LIMIT
            || self.watches.len() > WATCH_LIMIT
            || self.node_clocks.len() > OPEN_LIMIT
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
                || !matches!(
                    letter.kind.as_str(),
                    "request" | "block" | "report" | "progress" | "reply" | "watch"
                )
                || letter.body.len() > BODY_LIMIT
                || letter.body.trim().is_empty()
                || letter.bell_errors > 3
                || letter.bell_attempts.is_some_and(|attempts| attempts > 3)
                || letter
                    .bell_line
                    .as_deref()
                    .is_some_and(|line| !super::bell::valid_line(line))
                || (letter.hook_confirmed == Some(true)
                    && matches!(
                        letter.state,
                        State::Pending | State::Undelivered | State::Expired
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
                    State::Cancelled | State::Undelivered | State::Expired
                ) && letter.waiting_answer)
                // A letter waiting on a machine's clock knows where on it.
                || (letter.awaiting_intake()
                    && letter.intake_from.is_none()
                    && self.node_clocks.contains_key(&letter.recipient.device_id))
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

    pub fn expire(&mut self, now: u64) -> bool {
        let mut changed = self.cleanup(now);
        let clocks = &self.node_clocks;
        for letter in &mut self.letters {
            if !letter.intake_overdue(clocks, now) {
                continue;
            }
            if letter.state == State::Pending {
                letter.state = State::Undelivered;
                letter.waiting_answer = false;
                letter.finished_at_unix_ms = Some(now);
            } else {
                // Acknowledged without a receipt by a build before the
                // acknowledgement was the receipt. Its intake is no longer
                // awaited: the hook stops handing it over and it stops
                // counting as open, while an awaited reply keeps it open.
                letter.hook_confirmed = None;
                if !letter.open() {
                    letter.finished_at_unix_ms.get_or_insert(now);
                }
            }
            changed = true;
        }
        changed
    }

    /// Brings the machines' clocks to what `reach` says at `now` (PRD
    /// core-host-node-move B13): a machine with a letter awaiting intake and
    /// no clock gets one, a stretch starts when the machine becomes
    /// reachable and ends when it stops being so, a running stretch is
    /// written in every [`CLOCK_FOLD_MS`], and a clock goes once no letter
    /// waits on it or its machine is no longer another machine this core
    /// hears of (it left, or it is now this core's own). Every letter keeps
    /// the wait it had through each of these. Returns whether anything
    /// changed.
    pub fn observe_reach(&mut self, reach: &Reach, now: u64) -> bool {
        let mut changed = false;
        let waited_on: HashSet<String> = self
            .letters
            .iter()
            .filter(|letter| letter.awaiting_intake())
            .map(|letter| letter.recipient.device_id.clone())
            .collect();
        let ended: Vec<String> = self
            .node_clocks
            .keys()
            .filter(|device| !reach.contains_key(*device) || !waited_on.contains(*device))
            .cloned()
            .collect();
        for device in ended {
            self.stop_clock(&device, now);
            changed = true;
        }
        for (device, &reachable) in reach {
            if !waited_on.contains(device) {
                continue;
            }
            let Some(clock) = self.node_clocks.get_mut(device) else {
                self.start_clock(device, reachable, now);
                changed = true;
                continue;
            };
            match (clock.since, reachable) {
                (None, true) => clock.since = Some(now),
                (Some(since), false) => {
                    clock.connected_ms =
                        clock.connected_ms.saturating_add(now.saturating_sub(since));
                    clock.since = None;
                }
                (Some(since), true) if now.saturating_sub(since) >= CLOCK_FOLD_MS => {
                    clock.connected_ms =
                        clock.connected_ms.saturating_add(now.saturating_sub(since));
                    clock.since = Some(now);
                }
                _ => continue,
            }
            changed = true;
        }
        changed
    }

    /// Whether [`Ledger::observe_reach`] would change anything at `now`, asked
    /// without a copy of the ledger.
    pub fn reach_moves(&self, reach: &Reach, now: u64) -> bool {
        let waited_on = |device: &str| {
            self.letters
                .iter()
                .any(|letter| letter.awaiting_intake() && letter.recipient.device_id == device)
        };
        self.node_clocks
            .iter()
            .any(|(device, clock)| match reach.get(device) {
                None => true,
                Some(&reachable) => {
                    !waited_on(device)
                        || match (clock.since, reachable) {
                            (None, true) | (Some(_), false) => true,
                            (Some(since), true) => now.saturating_sub(since) >= CLOCK_FOLD_MS,
                            (None, false) => false,
                        }
                }
            })
            || reach
                .keys()
                .any(|device| !self.node_clocks.contains_key(device) && waited_on(device))
    }

    /// Puts the letters awaiting intake by an agent on `device` on a new
    /// clock for it, each with the wall time it waited so far: the clock
    /// starts at the longest of those waits.
    fn start_clock(&mut self, device: &str, reachable: bool, now: u64) {
        let waits: Vec<(usize, u64)> = self
            .letters
            .iter()
            .enumerate()
            .filter(|(_, letter)| letter.awaiting_intake() && letter.recipient.device_id == device)
            .map(|(index, letter)| (index, letter.intake_waited(&self.node_clocks, now)))
            .collect();
        let start = waits
            .iter()
            .map(|(_, waited)| *waited)
            .max()
            .unwrap_or_default();
        for (index, waited) in waits {
            self.letters[index].intake_from = Some(start - waited);
        }
        self.node_clocks.insert(
            device.to_owned(),
            NodeClock {
                connected_ms: start,
                since: reachable.then_some(now),
            },
        );
    }

    /// Ends the clock of `device`; the letters still awaiting intake by an
    /// agent there wait on in wall time, each with the wait it had.
    fn stop_clock(&mut self, device: &str, now: u64) {
        let waits: Vec<(usize, u64)> = self
            .letters
            .iter()
            .enumerate()
            .filter(|(_, letter)| letter.recipient.device_id == device)
            .map(|(index, letter)| (index, letter.intake_waited(&self.node_clocks, now)))
            .collect();
        self.node_clocks.remove(device);
        for (index, waited) in waits {
            let letter = &mut self.letters[index];
            letter.intake_from = letter.awaiting_intake().then(|| now.saturating_sub(waited));
        }
    }

    /// What a ledger read at a core's start knows of a stretch that was
    /// running when the last core stopped is only up to its last write, so
    /// every running stretch ends there, with what was written of it.
    fn end_stretches_at_start(&mut self) {
        for clock in self.node_clocks.values_mut() {
            clock.since = None;
        }
    }

    /// Ends the answer wait of every letter `actor` sent or received and the
    /// recipient took in, because its registration ended. A letter still
    /// awaiting intake is left to the delivery deadline. Returns whether any
    /// wait ended.
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
}

pub fn load(path: &Path) -> Result<Ledger, String> {
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
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
    if bytes.len() > FILE_LIMIT {
        return Err("ledger_unavailable".into());
    }
    let mut ledger: Ledger = serde_json::from_slice(&bytes).map_err(|_| "ledger_unavailable")?;
    ledger.validate().map_err(|_| "ledger_unavailable")?;
    ledger.end_stretches_at_start();
    Ok(ledger)
}

/// Keep the persistence phase intact until the store has applied its
/// admission policy. An uncertain replacement can never mean old bytes won.
#[derive(Debug)]
pub enum SaveError {
    Validation(String),
    Persistence(hide_platform::fs::atomic::DurableWriteError),
    Bootstrap(hide_platform::fs::private::DirectoryDurabilityError),
}

impl SaveError {
    pub fn code(&self) -> &str {
        match self {
            Self::Validation(code) => code,
            Self::Persistence(_) | Self::Bootstrap(_) => "ledger_unavailable",
        }
    }

    pub fn uncertain(&self) -> bool {
        matches!(
            self,
            Self::Persistence(
                hide_platform::fs::atomic::DurableWriteError::ReplacementUncertain { .. }
                    | hide_platform::fs::atomic::DurableWriteError::ReplacedNotDurable { .. }
            ) | Self::Bootstrap(
                hide_platform::fs::private::DirectoryDurabilityError::Uncertain { .. }
            )
        )
    }

    pub fn diagnostic(&self) -> serde_json::Value {
        use hide_platform::fs::atomic::DurableWriteError;
        let (phase, source, cleanup) = match self {
            Self::Validation(code) => return serde_json::json!({"phase":"validation","code":code}),
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
                return serde_json::json!({"phase":phase,"io_kind":format!("{:?}",source.kind())});
            }
            Self::Persistence(DurableWriteError::BeforeReplace { source, cleanup }) => {
                ("before_replace", source, cleanup.is_some())
            }
            Self::Persistence(DurableWriteError::ReplacementUncertain { source, cleanup }) => {
                ("replacement_uncertain", source, cleanup.is_some())
            }
            Self::Persistence(DurableWriteError::ReplacedNotDurable { source, .. }) => {
                ("replaced_not_durable", source, false)
            }
        };
        // IO text and temporary paths can contain private directory names.
        serde_json::json!({"phase":phase,"io_kind":format!("{:?}",source.kind()),"cleanup_failed":cleanup})
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
            Self::Validation(_) => None,
            Self::Persistence(source) => Some(source),
            Self::Bootstrap(source) => Some(source),
        }
    }
}

pub fn save(path: &Path, ledger: &Ledger) -> Result<(), SaveError> {
    let bytes = ledger.bytes().map_err(SaveError::Validation)?;
    // The parent and its durable ancestry are established by initialization,
    // never opportunistically created by a transaction after admission.
    hide_platform::fs::atomic::write_file_durable(
        path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map_err(SaveError::Persistence)?;
    Ok(())
}

/// Startup runs before Runtime is placed behind its mutex. Re-read the
/// actual installed version and establish a new barrier before any effect.
/// Corrupt or inaccessible bytes are preserved and never repaired by guess.
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
    let ledger = load(path).map_err(SaveError::Validation)?;
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
    save(path, &ledger)?;
    Ok(ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;

    /// A ledger holding one letter sent at `sent` to an agent on `device`.
    fn letter_to(device: &str, sent: u64) -> Ledger {
        let mut ledger = Ledger::default();
        let actor = |name: &str, device: &str| Actor {
            pane_id: name.into(),
            name: name.into(),
            kind: "codex".into(),
            device_id: device.into(),
            session: Some(format!("{name}-native")),
        };
        super::super::mailbox::send(
            &mut ledger,
            &actor("sender", "local"),
            &actor("recipient", device),
            "once",
            "private",
            "request",
            None,
            sent,
        )
        .unwrap();
        ledger
    }

    fn reach(device: &str, reachable: bool) -> Reach {
        [(device.to_owned(), reachable)].into()
    }

    /// The deadline of a letter to another machine runs only while that
    /// machine is reachable: two hours away count nothing, and the letter
    /// turns undelivered 60 reachable minutes after it was sent; then the
    /// clock goes with the last letter waiting on it (B13).
    #[test]
    fn a_letter_to_another_machine_waits_only_while_that_machine_is_reachable() {
        let mut ledger = letter_to("mini", 0);
        ledger.observe_reach(&reach("mini", true), 0);
        ledger.observe_reach(&reach("mini", false), 10 * MINUTE);
        ledger.expire(130 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Pending);
        ledger.observe_reach(&reach("mini", true), 130 * MINUTE);
        ledger.expire(179 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Pending);
        ledger.expire(180 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Undelivered);
        ledger.observe_reach(&reach("mini", true), 180 * MINUTE);
        assert!(ledger.node_clocks.is_empty());
    }

    /// A letter keeps the wait it had when its recipient's machine changes
    /// sides in a move (amendment 4): fifty wall minutes to this core's own
    /// machine leave ten reachable minutes once it is another machine, and
    /// fifty reachable minutes leave ten wall minutes once it is this core's.
    #[test]
    fn a_letter_keeps_its_wait_when_its_machine_changes_sides() {
        let mut ledger = letter_to("macbook", 0);
        ledger.observe_reach(&Reach::new(), 50 * MINUTE);
        ledger.observe_reach(&reach("macbook", false), 50 * MINUTE);
        ledger.observe_reach(&reach("macbook", true), 300 * MINUTE);
        ledger.expire(309 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Pending);
        ledger.expire(310 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Undelivered);

        let mut ledger = letter_to("mini", 0);
        ledger.observe_reach(&reach("mini", true), 0);
        ledger.observe_reach(&reach("mini", false), 50 * MINUTE);
        ledger.observe_reach(&Reach::new(), 400 * MINUTE);
        ledger.expire(409 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Pending);
        ledger.expire(410 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Undelivered);
    }

    /// A core that stops loses only the part of a reachable stretch it had
    /// not written: the stretch is written every minute, and the next core
    /// counts from what was written, not from the stretch's start (B13).
    #[test]
    fn a_restart_counts_a_running_stretch_only_up_to_its_last_write() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ledger.json");
        let mut ledger = letter_to("mini", 0);
        ledger.observe_reach(&reach("mini", true), 0);
        assert!(ledger.observe_reach(&reach("mini", true), 30 * MINUTE));
        save(&path, &ledger).unwrap();
        // Stopped at 31 minutes, started again an hour later.
        let mut ledger = load(&path).unwrap();
        ledger.observe_reach(&reach("mini", true), 90 * MINUTE);
        ledger.expire(119 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Pending);
        ledger.expire(120 * MINUTE);
        assert_eq!(ledger.letters[0].state, State::Undelivered);
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
            let path = root.path().join("ledger.json");
            let mut ledger = letter();
            for attempt in 1..=3 {
                let line = format!("🔔 sender 보고: 시도 {attempt}");
                assert_eq!(
                    ledger.letters[0]
                        .reserve_bell(&Default::default(), 2, line.clone())
                        .unwrap(),
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
                    .reserve_bell(&Default::default(), 2, "🔔 a 보고: b".into())
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
                    .reserve_bell(&Default::default(), 2, "🔔 a 보고: b".into())
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
                        .reserve_bell(&Default::default(), 2, "🔔 a 보고: b".into())
                        .unwrap_err(),
                    "doorbell_attempt_limit"
                );
            } else {
                assert_eq!(
                    restored.letters[0]
                        .reserve_bell(&Default::default(), 2, "🔔 a 보고: b".into())
                        .unwrap(),
                    3
                );
                assert_eq!(
                    restored.letters[0]
                        .reserve_bell(&Default::default(), 2, "🔔 a 보고: b".into())
                        .unwrap_err(),
                    "doorbell_attempt_limit"
                );
            }
        }
        let mut invalid = ledger;
        assert_eq!(
            invalid.letters[0]
                .reserve_bell(&Default::default(), 2, "🔔 a\n보고".into())
                .unwrap_err(),
            "bell_line_invalid"
        );
        invalid.letters[0].bell_line = Some("🔔 a\n보고".into());
        assert_eq!(invalid.validate().unwrap_err(), "ledger_unavailable");
        invalid.letters[0].bell_line = None;
        invalid.letters[0].bell_attempts = Some(4);
        assert_eq!(invalid.validate().unwrap_err(), "ledger_unavailable");
    }

    #[test]
    fn acknowledgement_receipt_survives_durable_reload_and_replay_without_closing_new_watch() {
        use super::super::{mailbox, watch};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ledger.json");
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
            (State::Undelivered, true),
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
        let path = state.join("ledger.json");
        let ledger = letter();
        save(&path, &ledger).unwrap();
        assert_eq!(recover(&path).unwrap(), ledger);
        std::fs::write(&path, b"private damaged bytes").unwrap();
        assert_eq!(recover(&path).unwrap_err().code(), "ledger_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"private damaged bytes");
        // A missing parent is not implicitly bootstrapped by a transaction.
        let missing = root.path().join("absent/ledger.json");
        let error = save(&missing, &Ledger::default()).unwrap_err();
        assert!(!error.uncertain());
        assert!(!missing.parent().unwrap().exists());
        assert_eq!(error.diagnostic()["phase"], "before_replace");
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
        let path = state.join("ledger.json");
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
                recover(&link.join("ledger.json")).unwrap_err().code(),
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
        let path = root.path().join("ledger.json");
        save(&path, &Ledger::default()).unwrap();
        assert!(hide_platform::fs::private::is_private(&path).unwrap());
        assert_eq!(load(&path).unwrap(), Ledger::default());
        std::fs::write(&path, b"broken ledger").unwrap();
        assert_eq!(load(&path).unwrap_err(), "ledger_unavailable");
        assert_eq!(std::fs::read(&path).unwrap(), b"broken ledger");
    }
}
