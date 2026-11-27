//! The runtime's side of automations (PRD automations): the book behind the
//! runtime lock and its file saved off it, the clock on the local
//! coordinator's tick, each run's start, follow and end, the closes runs
//! leave, the calls the CLI and the windows make, the `automations` snapshot
//! section and the automation's own Needs You line.
//!
//! A run's agent is Hide's own (`hide:automation`), started with no parent
//! as a Hide AI task whose home is the automation screen (D-11, D-22): its
//! row leaves the operator's lists and reaches Needs You on its own demand,
//! block or a View it opened. Every effect, a start, a close, an answer's
//! read, runs on a worker with the lock released, and its answer is applied
//! under the lock by the run's number, so a run that moved on meanwhile is
//! left as it is.
//!
//! What a run leaves to close, its terminals and its own worktree, is one
//! list in the book (`Stored::cleanups`): every path that ends an attempt, a
//! run, an opened conversation or an automation adds to it, a close that
//! fails is tried again later, and one a restart cut off is finished by the
//! next core (D-13, D-18, engineering 14).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jiff::tz::TimeZone;
use serde_json::{Value, json};

use super::Runtime;
use super::workspace_view::WorkspaceKey;
use crate::automations::{
    AgentChoice, Ask, Automation, Call, Changes, Conversation, Draft, Entry, Folder,
    MACHINE_CHANGED, Notice, Opened, Pause, PauseReason, RESULT_ADDRESS_LIMIT, RunResult, RunState,
    RunWorktree, SkipReason, Stored, Trigger, Unreadable, attempt_intent, open_intent, reason_code,
};
use crate::coordination::SPAWN_BUSY;
use crate::latest_file::{LatestFile, read_capped};
use crate::model::InterfaceLanguage;
use crate::view_layout::{DisplayKind, KeptView};

/// Hide's own identity the runs start under (D-22).
pub(crate) const AUTOMATION_ROLE: &str = "automation";
/// A gap between two ticks past this means the machine slept (D-15).
const SLEEP_GAP_MS: u64 = 60_000;
/// Runs are followed at most this often; a tick between is a comparison.
const DRIVE_INTERVAL_MS: u64 = 1_000;
/// How long an attempt may take to show its agent's row, a folder its
/// registration, or a start that stopped partway to finish, before the
/// attempt fails.
const START_DEADLINE_MS: u64 = 3 * 60 * 1000;
/// How long a row that was shown may be missing before its agent is read as
/// gone: an engine restart lists it again within this.
const ABSENT_GRACE_MS: u64 = 30_000;
/// How long after a start that stopped partway (its pane made) the same
/// start is asked again, which continues it (docs/delivery.md).
const RESUME_GAP_MS: u64 = 10_000;
/// How long after a start refused because another start held this
/// machine's start line (`spawn_busy`: nothing was started) it is asked
/// again; a start holds the line for seconds.
const BUSY_GAP_MS: u64 = 1_000;
/// How long one start may wait for what it needs and has not got yet
/// before it is given up as a transient failure: the line is never held
/// that long by starts alone, nor a machine's kit that long unread.
const WAIT_LIMIT_MS: u64 = 15 * 60 * 1000;
/// The refusals of a start that started nothing and that a later ask of
/// the same intent may pass: the start line held, a Codex start on a
/// machine whose kit has not read its Codex yet (the unused pane is closed
/// by the spawn), and a tab asked of the folder's workspace while its last
/// pane closes (the last run's terminal; the engine ends that start, and
/// the next ask opens the folder again). Asked again under the intent,
/// outside its deadline.
const WAIT_REFUSALS: [&str; 3] = [
    SPAWN_BUSY,
    crate::codex_launch::CAPABILITY_UNKNOWN,
    crate::coordination::WORKSPACE_CLOSING,
];
/// The refusal of a start whose agent's program this machine does not find
/// (`Call::AgentInstalled`), which a person fixes (B23).
const AGENT_NOT_INSTALLED: &str = "agent_not_installed";
/// Why an attempt whose stored start this build cannot read failed.
const START_UNREADABLE: &str = "start_unreadable";
/// The wait after a close's first failure, doubled each try up to the last.
const CLEANUP_RETRY_MS: u64 = 5_000;
const CLEANUP_RETRY_MAX_MS: u64 = 10 * 60 * 1000;
/// Tries a close gets, about 70 minutes of them, before it is given up with
/// a diagnostic: a worktree whose project folder was deleted never releases
/// (engineering 15).
const CLEANUP_TRIES: u32 = 12;
/// Closes running at once.
const CLEANUPS_AT_ONCE: usize = 4;
/// How often the machine's time zone is read again, so a zone the machine
/// moved to is the schedules' within this.
const ZONE_READ_MS: u64 = 60_000;
/// How long a window's word that it shows a run's terminal holds unless it
/// sends it again, so a window that went without saying so holds nothing.
const VIEWING_HOLD_MS: u64 = 30 * 60 * 1000;
/// Terminals marked on screen at once; past it the oldest mark goes.
const VIEWING_CAP: usize = 16;
/// The line a retry's resumed conversation is handed (D-17), typed as the
/// operator's words, so in the operator's language.
fn continue_prompt(language: InterfaceLanguage) -> &'static str {
    match language {
        InterfaceLanguage::English => "Continue. Finish what you were doing.",
        InterfaceLanguage::Korean => "이어서 해줘. 하던 일을 마저 끝내 줘.",
        InterfaceLanguage::Japanese => "続けて。やりかけの作業を最後まで終えて。",
        InterfaceLanguage::SimplifiedChinese => "继续。把没做完的事做完。",
    }
}
/// How long the core's own node may take to make an automation's folder.
const FOLDER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// One run, by its automation's id and its number; an opening of its
/// conversation is tracked under the number with `OPEN_BIT` set.
type RunKey = (String, u64);

/// An opening's key keeps it apart from its run's attempts.
const OPEN_BIT: u64 = 1 << 62;

fn open_key(key: &RunKey) -> RunKey {
    (key.0.clone(), key.1 | OPEN_BIT)
}

/// What a run's agent start is handed: Hide's own delivery identity.
type Delivery = (
    crate::delivery::worker::Client,
    crate::delivery::worker::Authority,
    crate::delivery::Actor,
);

/// The work of a run's agent start, run on a worker: the agent start
/// (`coordination::run`), answering the agent's registration id or the
/// start's refusal code.
pub(crate) type Start =
    Arc<dyn Fn(Delivery, crate::coordination::Command) -> Result<String, String> + Send + Sync>;

struct StartWork(Start);

impl Default for StartWork {
    fn default() -> Self {
        Self(Arc::new(|(client, authority, actor), command| {
            crate::coordination::run(client, authority, actor, command).and_then(|view| {
                view["id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "spawn_answer_unreadable".to_owned())
            })
        }))
    }
}

#[derive(Default)]
pub(crate) struct AutomationsRuntime {
    book: Stored,
    store: Option<LatestFile<Stored>>,
    zone: Option<TimeZone>,
    next_zone_read_unix_ms: u64,
    zone_reading: bool,
    /// Counts what moves the section; the section is made again only when
    /// it or a live run's pane moved.
    edit: u64,
    composed: Option<(u64, Vec<Option<String>>, Vec<RunKey>)>,
    /// The agents of the runs whose terminal the section names, kept when
    /// the book changes so a snapshot read looks only these up.
    live_agents: Vec<String>,
    /// The intents of the starts the book asks now, an attempt's or an
    /// opening's: a pane such a start made is the run's before its agent
    /// registers (`started::StartingPane`).
    asking: HashSet<String>,
    /// The once automations, by id: their runs' rows say so
    /// (`HideAiSnapshot::once`), laid again when this set moves.
    once: HashSet<String>,
    /// What last kept each finished run's terminal open, so the hold is
    /// logged when it changes, not on every look.
    close_held: HashMap<RunKey, &'static str>,
    /// The paths of this machine's automation folders, which no list shows.
    unlisted: HashSet<String>,
    last_tick_unix_ms: Option<u64>,
    next_drive_unix_ms: u64,
    start: StartWork,
    /// Starts on a worker now, by run or opening.
    starting: HashSet<RunKey>,
    /// When each attempt began waiting for its folder or its start, or each
    /// opening for its start, by this core: the start's deadline counts
    /// from here, and once it started, the wait for its agent's row.
    attempt_since: HashMap<RunKey, u64>,
    /// A start that stopped partway with its pane made: its code, and when
    /// it is asked again.
    resume: HashMap<RunKey, (String, u64)>,
    /// A start refused for what it needs and has not got yet, nothing
    /// started (`WAIT_REFUSALS`): the refusal, since when it waits, and when
    /// it is asked again. The start's deadline does not run while it waits.
    waiting: HashMap<RunKey, (String, u64, u64)>,
    /// Whether the attempt's agent was seen at work, so a stop with no
    /// answer after it is the end of its turn, not a start on its way.
    worked: HashSet<RunKey>,
    /// Whether the attempt's or opening's row was ever shown.
    shown: HashSet<RunKey>,
    /// Since when a shown row has been missing.
    absent_since: HashMap<RunKey, u64>,
    /// Closes and answer reads on a worker now.
    cleaning: HashSet<RunKey>,
    reading: HashSet<RunKey>,
    /// Folders whose registration this core asked for, since when.
    registering: HashMap<String, u64>,
    /// The runs whose terminal a window shows in the automation screen,
    /// since when it said so (`Call::RunViewing`), at most `VIEWING_CAP`.
    viewing: HashMap<RunKey, u64>,
    /// Their panes, sorted: on screen as a shown tab's are, so they stay
    /// attached and the nodes draw them (`viewed_panes`).
    viewed_panes: Vec<String>,
    /// Whether any automation is on: hided reads it, never the book, to
    /// skip its idle exit (D-19).
    on: Arc<AtomicBool>,
}

/// The book's file, read before the runtime lock is taken, and the zone the
/// schedules are read in. A file that cannot be read is set aside beside
/// the store with the time in its name and the book starts empty, saying
/// so (`Stored::unreadable`); one that cannot be set aside either leaves the
/// automations unavailable, so the file is never written over.
pub(crate) fn restore(path: PathBuf) -> (Option<LatestFile<Stored>>, Stored, TimeZone) {
    let read = read_capped(&path, crate::automations::READ_CAP).and_then(|bytes| {
        bytes
            .map(|bytes| crate::automations::decode(&bytes))
            .transpose()
    });
    let stored = match read {
        Ok(stored) => stored.unwrap_or_default(),
        Err(message) => {
            let now = super::unix_milliseconds();
            let file_name = |path: &std::path::Path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            let mut stored = Stored::default();
            match crate::latest_file::set_aside(&path, now) {
                Ok(aside) => {
                    crate::diagnostic!(json!({
                        "component": "automations",
                        "kind": "automations_store.restore_failed",
                        "message": message,
                        "set_aside": aside.display().to_string(),
                    }));
                    stored.unreadable = Some(Unreadable {
                        file: file_name(&aside),
                        at_unix_ms: now,
                    });
                }
                Err(kept) => {
                    crate::diagnostic!(json!({
                        "component": "automations",
                        "kind": "automations_store.restore_failed",
                        "message": message,
                        "set_aside_failed": kept,
                    }));
                    stored.unreadable = Some(Unreadable {
                        file: file_name(&path),
                        at_unix_ms: now,
                    });
                    return (None, stored, TimeZone::system());
                }
            }
            stored
        }
    };
    let file = LatestFile::new(
        path,
        "hide-automations-save",
        "automations",
        "automations_store",
        crate::automations::encode,
    );
    (Some(file), stored, TimeZone::system())
}

/// The window event kinds, each one `Call` by the same name.
pub(super) const EVENT_KINDS: [&str; 12] = [
    "automation_add",
    "automation_edit",
    "automation_pause",
    "automation_resume",
    "automation_run",
    "automation_remove",
    "automation_run_stop",
    "automation_run_open",
    "automation_run_viewing",
    "automation_run_result_open",
    "automation_seen",
    "automation_look_seen",
];

pub(super) fn decode_event(kind: &str, payload: Value) -> Option<Call> {
    let call = kind.strip_prefix("automation_")?;
    let mut payload = match payload {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        _ => return None,
    };
    payload.insert("call".into(), Value::String(call.into()));
    serde_json::from_value(Value::Object(payload)).ok()
}

/// What decides whether a run may start (D-16, D-17).
enum Gate {
    Go,
    Skip(SkipReason),
    Pause(PauseReason),
}

/// How an attempt ended without the task's answer.
enum Failure {
    /// Tried again in the same run (D-17).
    Transient(String),
    /// The agent itself failed the attempt: it exited, never showed, was
    /// refused, or stopped with no answer. With its account read signed out
    /// this is the sign-in that ended (B23), which stops the automation;
    /// otherwise it is tried again as a transient one. A signed-out read
    /// alone stops nothing, since the read guesses from what `/usage`
    /// printed and the agent running is the fact.
    Agent(String),
    /// Stops the automation for a person to fix (D-12).
    Person(PauseReason),
}

/// When a finished run's terminal closes.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Close {
    /// Now: nothing in it is the operator's to look at.
    Now,
    /// Once nothing in it waits on the operator (B15).
    Idle,
}

impl Runtime {
    pub(crate) fn install_automations_store(
        &mut self,
        store: Option<LatestFile<Stored>>,
        mut stored: Stored,
        zone: TimeZone,
    ) {
        let now = super::unix_milliseconds();
        let here = self.node.as_str().to_owned();
        // A book that says it lost the one before is saved saying so.
        let mut save = stored.unreadable.is_some();
        for cleanup in &mut stored.cleanups {
            if cleanup.machine.is_empty() {
                cleanup.machine = here.clone();
                save = true;
            }
        }
        let mut left_elsewhere = Vec::new();
        for automation in &mut stored.automations {
            if automation.machine.is_empty() {
                automation.machine = here.clone();
                save = true;
            } else if automation.machine != here {
                save |= moved_here(automation, &here, now, &mut left_elsewhere);
            }
            // A book saved by an earlier run keeps its times; one on with no
            // time takes its next one now, which a once automation whose time
            // was taken never gets again. A once automation still waiting is
            // read again as its wall date and time in this zone, the rule a
            // zone change follows, so a core started in another zone than it
            // was made in runs it there, once (mission-104 review S1).
            if automation.on()
                && (automation.next_due_unix_ms.is_none() || automation.schedule.is_once())
            {
                let next = automation.next_time(now, &zone);
                save |= next != automation.next_due_unix_ms;
                automation.next_due_unix_ms = next;
            }
        }
        for (automation, run, machine, (intents, worktree)) in left_elsewhere {
            stored.clean_up(&automation, run, &machine, intents, worktree, now);
        }
        self.automations.book = stored;
        self.automations.store = store;
        self.automations.zone = Some(zone);
        self.automations.next_zone_read_unix_ms = now.saturating_add(ZONE_READ_MS);
        self.automations_changed(save);
    }

    /// Whether the book holds the automation whose id is `id`.
    pub(super) fn automation_held(&self, id: &str) -> bool {
        self.automations
            .book
            .automations
            .iter()
            .any(|automation| automation.id == id)
    }

    /// Whether the automation `id` runs once (mission-104 O-4).
    pub(super) fn automation_once(&self, id: &str) -> bool {
        self.automations.once.contains(id)
    }

    /// Whether `agent_id` is the agent of one of the automations' runs.
    pub(super) fn automation_run_agent(&self, agent_id: &str) -> bool {
        self.automations
            .book
            .automations
            .iter()
            .flat_map(|automation| automation.runs())
            .any(|run| run.agent_id.as_deref() == Some(agent_id))
    }

    /// The flag hided reads for its idle exit (D-19).
    pub(crate) fn automations_on(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.automations.on)
    }

    /// The start a test hands in for the agent start, which tests cannot
    /// run without a real engine and agent.
    #[cfg(test)]
    pub(super) fn set_automation_start(&mut self, start: Start) {
        self.automations.start = StartWork(start);
    }

    /// The closes the book holds, as a test reads them.
    #[cfg(test)]
    pub(super) fn automation_cleanups(&self) -> Vec<crate::automations::Cleanup> {
        self.automations.book.cleanups.clone()
    }

    /// Whether the automation `id` runs in the folder Hide made for it
    /// (`Folder::default`), whose agent's folder trust a run's spawn records.
    pub(crate) fn automation_in_own_folder(&self, id: &str) -> bool {
        self.automations
            .book
            .find(id)
            .is_some_and(|automation| automation.folder.default)
    }

    /// The automations the book holds, as a test reads them.
    #[cfg(test)]
    pub(super) fn automation_book(&self) -> Vec<crate::automations::Automation> {
        self.automations.book.automations.clone()
    }

    /// Whether a start, close, answer read or zone read is on a worker.
    #[cfg(test)]
    pub(super) fn automation_busy(&self) -> bool {
        let work = &self.automations;
        !work.starting.is_empty()
            || !work.cleaning.is_empty()
            || !work.reading.is_empty()
            || work.zone_reading
    }

    fn zone(&self) -> TimeZone {
        self.automations.zone.clone().unwrap_or(TimeZone::UTC)
    }

    fn automations_changed(&mut self, save: bool) {
        self.automations.edit += 1;
        self.automations.live_agents = self
            .automations
            .book
            .automations
            .iter()
            .flat_map(|automation| automation.runs())
            .flat_map(|run| {
                let live = (run.state.active() || run.closing)
                    .then(|| run.agent_id.clone())
                    .flatten();
                live.into_iter().chain(
                    run.opened
                        .as_ref()
                        .and_then(|opened| opened.agent_id.clone()),
                )
            })
            .collect();
        self.automations
            .on
            .store(self.automations.book.any_on(), Ordering::SeqCst);
        let asking: HashSet<String> = self
            .automations
            .book
            .automations
            .iter()
            .flat_map(|automation| automation.runs())
            .flat_map(|run| {
                let attempt = (run.state == RunState::Starting)
                    .then_some(run.ask.as_ref())
                    .flatten();
                let opening = run
                    .opened
                    .as_ref()
                    .filter(|opened| opened.agent_id.is_none())
                    .and_then(|opened| opened.ask.as_ref());
                attempt.into_iter().chain(opening).filter_map(Ask::intent)
            })
            .map(str::to_owned)
            .collect();
        let once: HashSet<String> = self
            .automations
            .book
            .automations
            .iter()
            .filter(|automation| automation.schedule.is_once())
            .map(|automation| automation.id.clone())
            .collect();
        let once_moved = once != self.automations.once;
        self.automations.once = once;
        if asking != self.automations.asking || once_moved {
            self.automations.asking = asking;
            // A start's pane changes hands, or a run's automation became or
            // stopped being once: its row is laid again.
            self.refresh_agent_lineage();
        }
        let unlisted: HashSet<String> = self
            .automations
            .book
            .automations
            .iter()
            .filter(|automation| automation.folder.default)
            .map(|automation| automation.folder.path.clone())
            .collect();
        if unlisted != self.automations.unlisted {
            self.automations.unlisted = unlisted;
            let mut workspaces = std::mem::take(&mut self.snapshot.navigator.workspaces);
            self.mark_unlisted(&mut workspaces);
            self.snapshot.navigator.workspaces = workspaces;
        }
        if save {
            self.save_automations();
        }
        self.sync_viewed_panes();
    }

    fn save_automations(&self) {
        if let Some(store) = &self.automations.store {
            store.save(self.automations.book.clone());
        }
    }

    /// Marks the projects no list shows: this machine's automation folders,
    /// registered only so their runs can start there (D-08, B30). The
    /// sidebar, the phone and the project picker read the one flag.
    pub(super) fn mark_unlisted(&self, workspaces: &mut [crate::model::WorkspaceSnapshot]) {
        for workspace in workspaces {
            workspace.unlisted = workspace.device_id == self.node.as_str()
                && self.automations.unlisted.contains(&workspace.path);
        }
    }

    /// The clock (D-15, B12): run on the local coordinator's tick. A time
    /// that came is taken once; the runs and the closes they left are
    /// followed at most once a second.
    pub(crate) fn tick_automations(&mut self, now: u64) -> bool {
        if self.automations.store.is_none()
            || self.live.is_none()
            || (self.automations.book.automations.is_empty()
                && self.automations.book.cleanups.is_empty())
        {
            self.automations.last_tick_unix_ms = Some(now);
            return false;
        }
        let notice = match self.automations.last_tick_unix_ms {
            None => Notice::Started,
            Some(last) if now.saturating_sub(last) > SLEEP_GAP_MS => Notice::AfterGap,
            Some(_) => Notice::OnTick,
        };
        self.automations.last_tick_unix_ms = Some(now);
        if now >= self.automations.next_zone_read_unix_ms {
            self.read_zone(now);
        }
        let zone = self.zone();
        let mut changed = false;
        for index in 0..self.automations.book.automations.len() {
            let due = self.automations.book.automations[index].take_due(now, &zone, notice);
            if let Some(due) = due {
                self.fire(index, due, now);
                changed = true;
            }
        }
        if changed || now >= self.automations.next_drive_unix_ms {
            self.automations.next_drive_unix_ms = now.saturating_add(DRIVE_INTERVAL_MS);
            changed |= self.drive_runs(now);
            self.drive_cleanups(now);
        }
        if changed {
            self.automations_changed(true);
        } else {
            // A window's word that lapsed changes no record.
            changed |= self.sync_viewed_panes();
        }
        changed
    }

    /// The panes of the runs a window shows in the automation screen: on
    /// screen as a shown tab's panes are (`track_visible_tab_attachments`),
    /// though no tab of theirs is in front, so they are not released and
    /// their nodes draw them (B16, B17).
    /// Whether a start under `intent` is one the book asks now.
    pub(super) fn automation_asking(&self, intent: &str) -> bool {
        self.automations.asking.contains(intent)
    }

    pub(super) fn viewed_panes(&self) -> &[String] {
        &self.automations.viewed_panes
    }

    /// Reads the viewed runs' panes again: the run's agent's while it runs or
    /// its terminal is open, else its opened conversation's. When they moved
    /// (a window looked or looked away, a new attempt or opening started),
    /// the attachments follow. Whether a pane's state changed.
    fn sync_viewed_panes(&mut self) -> bool {
        if self.automations.viewing.is_empty() && self.automations.viewed_panes.is_empty() {
            return false;
        }
        let ledger = self.delivery_state().ok();
        let mut panes: Vec<String> = self
            .automations
            .viewing
            .keys()
            .filter_map(|key| self.run_of(key))
            .filter_map(|run| self.run_pane(run, ledger.as_ref()))
            .collect();
        panes.sort();
        panes.dedup();
        if panes == self.automations.viewed_panes {
            return false;
        }
        self.automations.viewed_panes = panes;
        self.track_visible_tab_attachments()
    }

    /// The pane a run's terminal is: its agent's while it runs or its
    /// terminal is open, else its opened conversation's; none once both
    /// ended.
    fn run_pane(
        &self,
        run: &crate::automations::Run,
        ledger: Option<&std::sync::Arc<crate::delivery::ledger::Ledger>>,
    ) -> Option<String> {
        let node = self.node.as_str();
        let pane = |agent: Option<&String>| -> Option<String> {
            let agent = agent?;
            ledger?
                .agents
                .iter()
                .find(|record| &record.id == agent && !record.ended)
                .map(|record| record.pane_on(node))
        };
        let live = run.state.active() || run.closing;
        live.then(|| pane(run.agent_id.as_ref()))
            .flatten()
            .or_else(|| pane(run.opened.as_ref()?.agent_id.as_ref()))
    }

    /// Keeps the zone the book was installed with: a test that installs one
    /// zone is not moved to the machine's own a minute later.
    #[cfg(test)]
    pub(crate) fn hold_automation_zone(&mut self) {
        self.automations.next_zone_read_unix_ms = u64::MAX;
    }

    /// Reads the machine's zone again on a worker; a zone that changed moves
    /// every automation's next time to that zone's wall clock.
    fn read_zone(&mut self, now: u64) {
        self.automations.next_zone_read_unix_ms = now.saturating_add(ZONE_READ_MS);
        if self.automations.zone_reading {
            return;
        }
        let spawned =
            self.spawn_issue_worker("automation-zone", TimeZone::system, |runtime, zone| {
                runtime.apply_zone(zone)
            });
        match spawned {
            Ok(()) => self.automations.zone_reading = true,
            Err(message) => crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.zone_unread",
                "message": message,
            })),
        }
    }

    fn apply_zone(&mut self, zone: TimeZone) -> bool {
        self.automations.zone_reading = false;
        let current = self.zone();
        if zone.iana_name() == current.iana_name() && format!("{zone:?}") == format!("{current:?}")
        {
            return false;
        }
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.zone_changed",
            "from": current.iana_name(), "to": zone.iana_name(),
        }));
        let now = super::unix_milliseconds();
        for automation in &mut self.automations.book.automations {
            if automation.on() {
                automation.next_due_unix_ms = automation.next_time(now, &zone);
            }
        }
        self.automations.zone = Some(zone);
        self.automations_changed(true);
        true
    }

    /// A time came for the automation at `index` (D-14, D-15, D-16). A
    /// once automation's one time is never skipped at the usage limit
    /// (mission-104 O-3): it starts as a manual run does, and a refusal for
    /// the limit ends the run, which the operator then finds in Needs You.
    /// A run of it already going is the run for its time.
    fn fire(&mut self, index: usize, due: crate::automations::Due, now: u64) {
        let automation = &self.automations.book.automations[index];
        let gate = self.run_gate(&automation.agent, now, !automation.schedule.is_once());
        let automation = &mut self.automations.book.automations[index];
        let missed = due.missed.is_some();
        if let Some(line) = due.missed {
            automation.push(line);
        }
        if automation.active_run().is_some() {
            automation.skip(now, SkipReason::PreviousRunning);
            return;
        }
        let trigger = if missed {
            Trigger::Missed
        } else {
            Trigger::Scheduled
        };
        match gate {
            Gate::Go => {
                automation.new_run(trigger, Some(due.due_unix_ms), now);
            }
            Gate::Skip(reason) => automation.skip(now, reason),
            Gate::Pause(reason) => {
                let number = automation.new_run(trigger, Some(due.due_unix_ms), now);
                let id = automation.id.clone();
                self.pause_for_person(&(id, number), reason, now);
            }
        }
    }

    /// Whether a run of `agent` may start now, judged on the account it
    /// runs in: its profile's, or the agent's default account for no profile
    /// or a profile in that account. An agent that is gone stops the
    /// automation (D-17); an account read at 100% before its reset skips a
    /// scheduled run (D-16). An account read signed out, or whose usage
    /// cannot be read, runs: a sign-in that ended stops the automation once
    /// the agent itself fails the attempt (`Failure::Agent`).
    fn run_gate(&self, agent: &AgentChoice, now: u64, scheduled: bool) -> Gate {
        use crate::usage::model::{Cause, Cell};
        let usage = &self.snapshot.usage;
        let profile = agent.profile.as_deref();
        if let Some(Cell::Unavailable {
            cause: Cause::NotInstalled,
            ..
        }) = usage.account_limits(&agent.kind, profile)
        {
            return Gate::Pause(PauseReason::AgentMissing);
        }
        if scheduled
            && usage
                .exhausted_until(&agent.kind, profile, now / 1000)
                .is_some()
        {
            return Gate::Skip(SkipReason::UsageLimit {
                agent: agent.kind.clone(),
            });
        }
        Gate::Go
    }

    /// Whether `agent`'s account, as `run_gate` reads it, was last read
    /// signed out.
    fn signed_out(&self, agent: &AgentChoice) -> bool {
        use crate::usage::model::{Cause, Cell};
        matches!(
            self.snapshot
                .usage
                .account_limits(&agent.kind, agent.profile.as_deref()),
            Some(Cell::Unavailable {
                cause: Cause::SignedOut,
                ..
            })
        )
    }

    /// A failure a person fixes: the run ends failed, its terminal closes,
    /// and the automation stops and raises its own Needs You line (D-12,
    /// B23); no retry. A once automation with no time left has nothing to
    /// stop: its run ends failed with the reason, which its own line in
    /// Needs You shows (mission-104 O-3, O-4).
    fn pause_for_person(&mut self, key: &RunKey, reason: PauseReason, now: u64) {
        let Some(automation) = self.automation_mut(&key.0) else {
            return;
        };
        if !automation.schedule.is_once() || automation.next_due_unix_ms.is_some() {
            automation.paused = Some(Pause {
                reason,
                at_unix_ms: now,
            });
            automation.next_due_unix_ms = None;
        }
        self.finish_run(key, RunState::Failed, Some(reason.code()), Close::Now, now);
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.paused_for_person",
            "automation": key.0, "run": key.1, "reason": reason.code(),
        }));
    }

    fn automation_mut(&mut self, id: &str) -> Option<&mut Automation> {
        self.automations
            .book
            .automations
            .iter_mut()
            .find(|automation| automation.id == id)
    }

    fn run_mut(&mut self, key: &RunKey) -> Option<&mut crate::automations::Run> {
        self.automation_mut(&key.0)?.run_mut(key.1)
    }

    fn run_of(&self, key: &RunKey) -> Option<&crate::automations::Run> {
        self.automations
            .book
            .find(&key.0)?
            .runs()
            .find(|run| run.number == key.1)
    }

    /// Forgets what this core followed of an attempt or an opening.
    fn forget(&mut self, key: &RunKey) {
        self.automations.attempt_since.remove(key);
        self.automations.resume.remove(key);
        self.automations.waiting.remove(key);
        self.automations.worked.remove(key);
        self.automations.shown.remove(key);
        self.automations.absent_since.remove(key);
    }

    /// Ends a run in `state`: what its terminal holds is kept until nothing
    /// in it waits on the operator (`Close::Idle`), or closed now with its
    /// own worktree.
    fn finish_run(
        &mut self,
        key: &RunKey,
        state: RunState,
        reason: Option<&str>,
        close: Close,
        now: u64,
    ) {
        self.forget(key);
        // The screen's word that it shows the run holds for a while after
        // the window went; only an attached window has the operator see the
        // end (mission-104 review O2).
        let viewed = self.ui_attached() && self.automations.viewing.contains_key(key);
        let Some(automation) = self.automation_mut(&key.0) else {
            return;
        };
        let once = automation.schedule.is_once();
        let Some(run) = automation.run_mut(key.1) else {
            return;
        };
        let ended = run.state.active() && !state.active();
        let trigger = run.trigger;
        run.state = state;
        run.ended_at_unix_ms = Some(now);
        run.reason = reason.map(reason_code);
        run.closing = close == Close::Idle;
        run.ask = None;
        // A once automation's run that ended waits in the operator's Needs
        // You until they open it, whatever its result (mission-104 O-4),
        // unless a window shows it as it ends, or the failure stopped the
        // automation for a person, whose own line already says so.
        let stopped = automation
            .paused
            .as_ref()
            .is_some_and(|pause| pause.reason.asks());
        if ended && once && !viewed && !stopped {
            automation.look_run = Some(key.1);
        }
        if ended && let Some(sink) = &self.product_analytics {
            use crate::product_analytics::{AutomationOutcome, AutomationTrigger, Event};
            let outcome = match state {
                RunState::Succeeded => AutomationOutcome::Succeeded,
                RunState::Stopped => AutomationOutcome::Stopped,
                _ => AutomationOutcome::Failed,
            };
            let trigger = match trigger {
                Trigger::Scheduled => AutomationTrigger::Scheduled,
                Trigger::Missed => AutomationTrigger::Missed,
                Trigger::Manual => AutomationTrigger::Manual,
            };
            sink.capture(Event::AutomationRunEnded { outcome, trigger });
        }
        if close == Close::Now {
            self.clean_up_run(key, now);
        }
    }

    /// Adds everything of the run to close on this machine: the terminal of
    /// each of its attempts and of its conversation opened again, then its
    /// own worktree. A start of an intent that made nothing closes nothing.
    fn clean_up_run(&mut self, key: &RunKey, now: u64) {
        let Some(automation) = self.automations.book.find(&key.0) else {
            return;
        };
        let Some(run) = automation.runs().find(|run| run.number == key.1) else {
            return;
        };
        let (intents, worktree) = run_leftovers(automation, run);
        let here = self.node.as_str().to_owned();
        self.automations
            .book
            .clean_up(&key.0, key.1, &here, intents, worktree, now);
    }

    /// Follows every run that is not done with its terminal: starts what is
    /// due to start, reads the agents of the runs going, and hands the
    /// terminals of finished runs nothing waits on to their close.
    fn drive_runs(&mut self, now: u64) -> bool {
        self.automations
            .viewing
            .retain(|_, since| now.saturating_sub(*since) < VIEWING_HOLD_MS);
        let mut work: Vec<(RunKey, RunState, bool, bool)> = Vec::new();
        for automation in &self.automations.book.automations {
            for run in automation.runs() {
                if run.state.active() || run.closing || run.opened.is_some() {
                    work.push((
                        (automation.id.clone(), run.number),
                        run.state,
                        run.closing,
                        run.opened.is_some(),
                    ));
                }
            }
        }
        let mut changed = false;
        for (key, state, closing, opened) in work {
            changed |= match state {
                RunState::Starting => self.drive_start(&key, now),
                RunState::Running => self.follow_run(&key, now),
                _ => false,
            };
            if closing {
                changed |= self.drive_close(&key, now);
            }
            if opened {
                changed |= self.follow_opened(&key, now);
            }
        }
        changed
    }

    /// The run's row, when its agent's registration still holds the pane.
    fn run_row(&self, agent_id: &str) -> RowLook {
        let Ok(ledger) = self.delivery_state() else {
            return RowLook::Unknown;
        };
        let Some(record) = ledger.agents.iter().find(|record| record.id == agent_id) else {
            return RowLook::Gone;
        };
        if record.ended {
            return RowLook::Gone;
        }
        let pane = record.pane_on(self.node.as_str());
        match self
            .agent_row(&pane)
            .filter(|row| row.lineage_session == record.actor.session)
        {
            Some(row) => RowLook::Shown(Box::new(row.clone())),
            None => RowLook::Missing,
        }
    }

    /// Starts the run's current attempt once its folder is ready (D-08), and
    /// once the close of the attempt before it was tried (a run is one
    /// terminal, D-13): its start is made then, once, and asked under its
    /// intent until it leaves (`drive_ask`). The attempt's deadline counts
    /// from its first look, the folder's wait included.
    fn drive_start(&mut self, key: &RunKey, now: u64) -> bool {
        let here = self.node.as_str();
        if self.automations.starting.contains(key)
            || self.automations.book.cleanups.iter().any(|cleanup| {
                cleanup.automation == key.0
                    && cleanup.run == key.1
                    && cleanup.machine == here
                    && cleanup.tries == 0
            })
        {
            return false;
        }
        let since = *self
            .automations
            .attempt_since
            .entry(key.clone())
            .or_insert(now);
        if self.run_of(key).is_some_and(|run| run.ask.is_some()) {
            return self.drive_ask(key, now);
        }
        // A folder that never got ready fails the attempt like an agent that
        // never showed.
        let late = now.saturating_sub(since) > START_DEADLINE_MS;
        let Some(automation) = self.automations.book.find(&key.0) else {
            return false;
        };
        let folder = automation.folder.clone();
        let node = self.node.as_str().to_owned();
        use super::projects::CheckoutKind;
        let kind = self.checkout_kind(&node, &folder.path);
        let branch = match kind {
            Some(CheckoutKind::Folder) => None,
            // A folder the catalog last read as gone may be back by now, and
            // the catalog may read it so for minutes: whether it is there is
            // the spawn's to ask its machine right before the start
            // (`coordination::executor` `folder_there`), which refuses one
            // still gone as folder missing (B23). The same stale read makes a
            // Git project's row look plain, so the automation's own choice
            // says where the run works: a new worktree when it asked for one
            // (D-08), which the spawn refuses in a folder that is no
            // repository, else the folder itself.
            Some(CheckoutKind::Unreadable) => folder.worktree.then(|| automation.run_branch(key.1)),
            Some(CheckoutKind::Git) if folder.worktree => Some(automation.run_branch(key.1)),
            // The project's own folder: the branch it has out, so the run
            // works in the folder itself (D-08). One with none out is a
            // person's to fix.
            Some(CheckoutKind::Git) => match self.root_branch(&node, &folder.path) {
                Some(branch) => Some(branch),
                None => {
                    return self.attempt_failed(
                        key,
                        Failure::Person(PauseReason::FolderDetached),
                        now,
                    );
                }
            },
            Some(CheckoutKind::Unread) | None => {
                if late {
                    return self.attempt_failed(
                        key,
                        Failure::Transient("folder_unregistered".into()),
                        now,
                    );
                }
                if kind.is_none() {
                    if !folder.default {
                        return self.attempt_failed(
                            key,
                            Failure::Person(PauseReason::FolderMissing),
                            now,
                        );
                    }
                    // Its machine read the folder as part of a repository
                    // around it, whose project holds no checkout there.
                    if self
                        .catalog_paths
                        .project(&folder.path)
                        .is_some_and(|facts| facts.kind == hide_project::ProjectKind::Git)
                    {
                        return self.attempt_failed(
                            key,
                            Failure::Person(PauseReason::FolderInRepository),
                            now,
                        );
                    }
                    self.register_own_folder(&key.0, &folder.path, now);
                }
                return false;
            }
        };
        if late {
            return self.attempt_failed(key, Failure::Transient("start_unavailable".into()), now);
        }
        self.make_attempt(key, folder, branch);
        // Saved before it is asked, so a core that starts again asks the
        // same start.
        self.drive_ask(key, now);
        true
    }

    /// The branch the project's own folder has out.
    fn root_branch(&self, node: &str, path: &str) -> Option<String> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == node)
            .flat_map(|workspace| workspace.checkouts.iter())
            .find(|checkout| checkout.path == path)
            .and_then(|checkout| checkout.branch.clone())
    }

    /// Makes the automation's own folder and registers it for its runs
    /// (D-08), once per folder at a time. No list shows it: its project is
    /// marked unlisted (`mark_unlisted`).
    fn register_own_folder(&mut self, id: &str, path: &str, now: u64) {
        if self
            .automations
            .registering
            .get(path)
            .is_some_and(|since| now.saturating_sub(*since) < START_DEADLINE_MS)
        {
            return;
        }
        self.automations.registering.insert(path.to_owned(), now);
        let label = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.name.clone())
            .unwrap_or_default();
        // The core's own node makes it, as it makes a new project's folder.
        let local = self.node.clone();
        let node = match self.node_link(local.as_str()) {
            Ok(node) => node,
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.folder_failed",
                    "message": message,
                }));
                return;
            }
        };
        let requested = path.to_owned();
        let name = path.rsplit('/').next().unwrap_or_default().to_owned();
        let spawned = self.spawn_issue_worker(
            "automation-folder",
            move || {
                crate::node_access::call_as::<hide_node_link::project::AutomationFolder>(
                    node.as_ref(),
                    hide_node_link::protocol::Call::AutomationFolder { name },
                    FOLDER_WAIT,
                )
                .map(|made| made.path)
                .map_err(|error| error.to_string())
            },
            move |runtime, made: Result<String, String>| match made {
                Ok(real) => runtime.register_made_folder(&requested, real, label),
                Err(message) => {
                    crate::diagnostic!(json!({
                        "component": "automations", "kind": "automation.folder_failed",
                        "message": message,
                    }));
                    false
                }
            },
        );
        if let Err(message) = spawned {
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.folder_failed",
                "message": message,
            }));
        }
    }

    /// The made folder is registered as a project of this machine, so a
    /// run's spawn finds it there, with no engine workspace and no pane: a
    /// pane would hold the daemon from its idle exit (B32). The node answers
    /// its real path, which the automation keeps from then on, so the
    /// registration and the automation name one folder.
    fn register_made_folder(&mut self, requested: &str, folder: String, label: String) -> bool {
        let mut moved = false;
        for automation in &mut self.automations.book.automations {
            if automation.folder.default
                && automation.folder.path == requested
                && requested != folder
            {
                automation.folder.path = folder.clone();
                moved = true;
            }
        }
        if moved {
            // The registration asked for is the real path's now.
            if let Some(since) = self.automations.registering.remove(requested) {
                self.automations.registering.insert(folder.clone(), since);
            }
            self.automations_changed(true);
        }
        match crate::workspace::registration(&folder, &label, self.node.as_str()) {
            Ok(registration) => {
                let line = format!(
                    "Registered the automation folder {} at {}",
                    registration.label, registration.path
                );
                self.register_without_workspace(registration, "automation.folder_registered", line)
                    | moved
            }
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.folder_register_failed",
                    "message": message,
                }));
                moved
            }
        }
    }

    /// Makes the current attempt's start, once: a new conversation with the
    /// task for the first, the same conversation resumed for a retry (D-10,
    /// D-17). It is kept on the run (`Run::ask`) until the start ended.
    fn make_attempt(&mut self, key: &RunKey, folder: Folder, branch: Option<String>) {
        let zone = self.zone();
        let language = self.interface_language();
        let Some(automation) = self.automations.book.find(&key.0) else {
            return;
        };
        let Some(run) = automation.runs().find(|run| run.number == key.1) else {
            return;
        };
        let attempt = run.attempt;
        let (args, brief) = match (attempt, run.conversation.as_ref()) {
            (1, _) | (_, None) => (Vec::new(), automation.prompt(key.1, &zone, language)),
            (_, Some(conversation)) => match resume_arguments(conversation) {
                Some(args) => (args, continue_prompt(language).to_owned()),
                None => (Vec::new(), automation.prompt(key.1, &zone, language)),
            },
        };
        let command = crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: None,
            machine: None,
            name: format!("auto-{}-r{}-a{attempt}", automation.id, key.1),
            intent: attempt_intent(&automation.id, key.1, attempt),
            kind: automation.agent.kind.clone(),
            repo: Some(folder.path.clone()),
            branch: branch.clone(),
            path: None,
            args,
            launch: hide_agent_adapter::LaunchChoice {
                model: automation.agent.model.clone(),
                effort: automation.agent.effort.clone(),
            },
            brief: Some(brief),
            task: Some(crate::coordination::SpawnTask {
                kind: crate::coordination::TaskKind::Automation,
                target: Some(automation.id.clone()),
            }),
            beside: false,
            profile: automation.agent.profile.clone(),
        };
        let worktree = folder.worktree.then_some(branch).flatten();
        if let Some(run) = self.run_mut(key) {
            run.ask = Some(Ask {
                command: Some(command),
                worktree,
            });
        }
    }

    /// The start `track` waits on: the run's current attempt's while it is
    /// starting, or the opening's until its agent registered.
    fn pending_ask(&self, track: &RunKey) -> Option<(Purpose, Ask)> {
        let run = self.run_of(&(track.0.clone(), track.1 & !OPEN_BIT))?;
        if track.1 & OPEN_BIT != 0 {
            let opened = run
                .opened
                .as_ref()
                .filter(|opened| opened.agent_id.is_none())?;
            return Some((Purpose::Open(opened.number), opened.ask.clone()?));
        }
        (run.state == RunState::Starting)
            .then(|| Some((Purpose::Attempt(run.attempt), run.ask.clone()?)))
            .flatten()
    }

    /// Asks a start under its intent, an attempt's or an opening's (D-13,
    /// B36): asked again after one that stopped with its pane made, which
    /// continues that start in its pane, never a second terminal, and after
    /// one refused because another start held the line, which started
    /// nothing. Past the deadline from its first ask by this core, the time
    /// spent waiting for the line left out, its pane goes to the close and
    /// the start ends as its purpose says (`ask_expired`).
    fn drive_ask(&mut self, track: &RunKey, now: u64) -> bool {
        if self.automations.starting.contains(track) {
            return false;
        }
        let Some((purpose, ask)) = self.pending_ask(track) else {
            return false;
        };
        let since = *self
            .automations
            .attempt_since
            .entry(track.clone())
            .or_insert(now);
        let resume = self.automations.resume.get(track).cloned();
        let waiting = self.automations.waiting.get(track).cloned();
        if let Some((code, waits_since, _)) = &waiting
            && now.saturating_sub(*waits_since) > WAIT_LIMIT_MS
        {
            return self.ask_expired(track, purpose, code.clone(), now);
        }
        // The start's own time stands still while it waits.
        let clock = waiting
            .as_ref()
            .map_or(now, |(_, waits_since, _)| *waits_since);
        if clock.saturating_sub(since) > START_DEADLINE_MS {
            let code = resume.map_or_else(|| "start_unavailable".to_owned(), |(code, _)| code);
            return self.ask_expired(track, purpose, code, now);
        }
        if waiting.is_some_and(|(_, _, at)| now < at) || resume.is_some_and(|(_, at)| now < at) {
            return false;
        }
        let Some(command) = ask.command else {
            return self.ask_unreadable(track, purpose, now);
        };
        self.spawn_for_run(track, command, purpose, now);
        false
    }

    /// A start the book holds in a shape this build cannot read (a build
    /// whose spawn differs wrote it): it cannot be asked again as it was
    /// made, so the pane it may have made goes to the close and an attempt
    /// fails as a transient failure, tried again as a new start; an opening
    /// ends, and a person can open the conversation again.
    fn ask_unreadable(&mut self, track: &RunKey, purpose: Purpose, now: u64) -> bool {
        let key = (track.0.clone(), track.1 & !OPEN_BIT);
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.start_unreadable",
            "automation": key.0, "run": key.1, "purpose": purpose.name(),
            "number": purpose.number(),
        }));
        match purpose {
            Purpose::Attempt(_) => {
                self.attempt_failed(&key, Failure::Transient(START_UNREADABLE.to_owned()), now)
            }
            Purpose::Open(number) => self.end_opening(&key, number, false, now),
        }
    }

    /// A start that did not leave by its deadline: the pane it made goes to
    /// the close, then an attempt fails as a transient failure, tried again
    /// up to three attempts, and an opening ends as a conversation that
    /// could not be opened, which a person can open again (B18).
    fn ask_expired(&mut self, track: &RunKey, purpose: Purpose, code: String, now: u64) -> bool {
        let key = (track.0.clone(), track.1 & !OPEN_BIT);
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.start_expired",
            "automation": key.0, "run": key.1, "purpose": purpose.name(),
            "number": purpose.number(), "reason": code,
        }));
        match purpose {
            Purpose::Attempt(_) => self.attempt_failed(&key, Failure::Transient(code), now),
            Purpose::Open(number) => self.end_opening(&key, number, true, now),
        }
    }

    /// Runs a start on a worker; `track` names it among the starts going.
    fn spawn_for_run(
        &mut self,
        track: &RunKey,
        command: crate::coordination::Command,
        purpose: Purpose,
        now: u64,
    ) {
        let delivery = match self.hide_delivery(AUTOMATION_ROLE) {
            Ok(delivery) => delivery,
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_unavailable",
                    "automation": track.0, "run": track.1 & !OPEN_BIT, "reason": reason,
                }));
                return;
            }
        };
        let start = Arc::clone(&self.automations.start.0);
        let probe = self.program_probe(&command);
        let worker_key = track.clone();
        let result = self.spawn_issue_worker(
            "automation-start",
            move || {
                if let Some((node, program)) = probe
                    && let Ok(None) = crate::start_prompt::start_form(
                        node.as_ref(),
                        &program,
                        crate::start_prompt::START_FORM_TIMEOUT,
                    )
                {
                    return Err(AGENT_NOT_INSTALLED.to_owned());
                }
                start(delivery, command)
            },
            move |runtime, started| runtime.apply_start(&worker_key, purpose, now, started),
        );
        match result {
            // The start's answer is applied under this lock, after it is
            // released, so marking it now is in time.
            Ok(()) => {
                self.automations.starting.insert(track.clone());
            }
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_unavailable",
                    "automation": track.0, "run": track.1 & !OPEN_BIT, "reason": message,
                }));
            }
        }
    }

    /// What finds whether this machine has the start's agent program: its
    /// own node and the program the engine starts the agent by. A spawn
    /// types the program whether or not the machine has it, so a missing
    /// one would leave a shell that never starts an agent and read as a
    /// start that stopped partway; asked first, it stops the automation for
    /// a person (B23). A node that does not answer says nothing, and the
    /// start goes on.
    fn program_probe(
        &mut self,
        command: &crate::coordination::Command,
    ) -> Option<(Arc<dyn crate::node_access::NodeLink>, String)> {
        let crate::coordination::Command::Spawn { kind, .. } = command else {
            return None;
        };
        let local = self.node.as_str().to_owned();
        let engine = self.delivery_engine(&local)?;
        let program = crate::start_prompt::program_of(engine.as_ref(), kind);
        Some((self.node_link(&local).ok()?, program))
    }

    /// A start's answer, applied to the start it was for. One for a start
    /// no longer waited on (a person ended the run, the automation was
    /// removed, the opening ended) changes nothing: the close of what it was
    /// for names its intent and waits for this answer. A start that stopped
    /// with its pane made, or found the line held by another start, is asked
    /// again under its intent (`drive_ask`); `asked_at` is when it was asked.
    fn apply_start(
        &mut self,
        track: &RunKey,
        purpose: Purpose,
        asked_at: u64,
        started: Result<String, String>,
    ) -> bool {
        self.automations.starting.remove(track);
        if self.pending_ask(track).map(|(waited, _)| waited) != Some(purpose) {
            return false;
        }
        let now = super::unix_milliseconds();
        let key = (track.0.clone(), track.1 & !OPEN_BIT);
        let refusal = started
            .as_ref()
            .err()
            .and_then(|reason| reason.split('\u{1f}').next())
            .filter(|code| WAIT_REFUSALS.contains(code));
        if let Some(code) = refusal {
            let waits_since = match self.automations.waiting.get(track) {
                Some((waited, waits_since, _)) if waited == code => *waits_since,
                _ => {
                    crate::diagnostic!(json!({
                        "component": "automations", "kind": "automation.start_waits",
                        "automation": key.0, "run": key.1, "purpose": purpose.name(),
                        "number": purpose.number(), "reason": code,
                    }));
                    asked_at
                }
            };
            // A kit read takes its while; the line and a closing pane free
            // within seconds.
            let gap = if code == crate::codex_launch::CAPABILITY_UNKNOWN {
                RESUME_GAP_MS
            } else {
                BUSY_GAP_MS
            };
            self.automations.waiting.insert(
                track.clone(),
                (code.to_owned(), waits_since, asked_at.saturating_add(gap)),
            );
            return false;
        }
        if let Some((_, waits_since, _)) = self.automations.waiting.remove(track) {
            // The wait was no part of the start's own time.
            if let Some(since) = self.automations.attempt_since.get_mut(track) {
                *since = since.saturating_add(asked_at.saturating_sub(waits_since));
            }
        }
        let changed = match started {
            Ok(agent_id) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_left",
                    "automation": key.0, "run": key.1, "purpose": purpose.name(),
                    "number": purpose.number(), "agent": agent_id,
                }));
                self.forget(track);
                self.automations.attempt_since.insert(track.clone(), now);
                match purpose {
                    Purpose::Attempt(_) => {
                        let conversation = self.registered_conversation(&agent_id);
                        let run = self.run_mut(&key).expect("waited on above");
                        run.branch = run.ask.take().and_then(|ask| ask.worktree);
                        run.agent_id = Some(agent_id);
                        run.state = RunState::Running;
                        // Named from the start, so a run that ends before its
                        // row is read can still be opened again (B17).
                        if conversation.is_some() {
                            run.conversation = conversation;
                        }
                    }
                    Purpose::Open(_) => {
                        let opened = self
                            .run_mut(&key)
                            .and_then(|run| run.opened.as_mut())
                            .expect("waited on above");
                        opened.agent_id = Some(agent_id);
                        opened.ask = None;
                    }
                }
                true
            }
            Err(reason) => {
                let code = reason.split('\u{1f}').next().unwrap_or_default().to_owned();
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.start_failed",
                    "automation": key.0, "run": key.1, "purpose": purpose.name(),
                    "number": purpose.number(), "reason": reason,
                }));
                if crate::coordination::INCOMPLETE_SPAWN.contains(&code.as_str()) {
                    self.automations
                        .resume
                        .insert(track.clone(), (code, now.saturating_add(RESUME_GAP_MS)));
                    return false;
                }
                match purpose {
                    Purpose::Attempt(_) => self.attempt_failed(&key, start_failure(code), now),
                    Purpose::Open(number) => self.end_opening(&key, number, false, now),
                }
            }
        };
        if changed {
            self.automations_changed(true);
        }
        changed
    }

    /// An attempt that ended without the task's answer (D-17): a transient
    /// failure closes the attempt's terminal and tries the run again up to
    /// three attempts, then ends it failed with the automation still on; a
    /// person's failure stops the automation at once.
    fn attempt_failed(&mut self, key: &RunKey, failure: Failure, now: u64) -> bool {
        self.forget(key);
        let reason = match failure {
            Failure::Person(reason) => {
                self.pause_for_person(key, reason, now);
                return true;
            }
            Failure::Transient(reason) => reason,
            Failure::Agent(reason) => {
                if self.signed_out(&self.run_agent(key)) {
                    self.pause_for_person(key, PauseReason::LoginExpired, now);
                    return true;
                }
                reason
            }
        };
        let Some(failed) = self.run_of(key).map(|run| run.attempt) else {
            return false;
        };
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.attempt_failed",
            "automation": key.0, "run": key.1, "attempt": failed, "reason": reason,
        }));
        if failed >= crate::automations::MAX_ATTEMPTS {
            self.finish_run(key, RunState::Failed, Some(&reason), Close::Now, now);
            return true;
        }
        let run = self.run_mut(key).expect("read above");
        run.reason = Some(reason_code(&reason));
        run.attempt += 1;
        run.state = RunState::Starting;
        run.agent_id = None;
        run.ask = None;
        // The failed attempt's terminal goes before the next one opens: a run
        // is one terminal (D-13). Its worktree stays for the next.
        let intent = attempt_intent(&key.0, key.1, failed);
        let here = self.node.as_str().to_owned();
        self.automations
            .book
            .clean_up(&key.0, key.1, &here, vec![intent], None, now);
        true
    }

    /// Reads a running run's agent (D-17, B22, B24): a finished turn is the
    /// run's answer; a refused, vanished or causeless stopped one a failed
    /// attempt, a refusal for the usage limit a skipped line; a turn a
    /// person ended while looking at it a stopped run; a question, an
    /// approval or a block waits on the operator in their Needs You.
    fn follow_run(&mut self, key: &RunKey, now: u64) -> bool {
        let Some(agent_id) = self.run_of(key).and_then(|run| run.agent_id.clone()) else {
            return self.attempt_failed(key, Failure::Transient("agent_unknown".into()), now);
        };
        let since = *self
            .automations
            .attempt_since
            .entry(key.clone())
            .or_insert(now);
        let row = match self.run_row(&agent_id) {
            RowLook::Unknown => return false,
            RowLook::Gone => {
                return self.attempt_failed(key, Failure::Agent("agent_exited".into()), now);
            }
            RowLook::Missing => {
                // A row shown before may be missing for an engine restart;
                // one never shown has the start's whole deadline.
                let gone = if self.automations.shown.contains(key) {
                    let absent = *self
                        .automations
                        .absent_since
                        .entry(key.clone())
                        .or_insert(now);
                    now.saturating_sub(absent) > ABSENT_GRACE_MS
                } else {
                    now.saturating_sub(since) > START_DEADLINE_MS
                };
                if gone {
                    let reason = if self.automations.shown.contains(key) {
                        "agent_exited"
                    } else {
                        "agent_not_ready"
                    };
                    return self.attempt_failed(key, Failure::Agent(reason.into()), now);
                }
                return false;
            }
            RowLook::Shown(row) => row,
        };
        self.automations.shown.insert(key.clone());
        self.automations.absent_since.remove(key);
        let mut changed = false;
        if let Some(conversation) = row_conversation(&row) {
            let run = self.run_mut(key).expect("followed run");
            if run.conversation.as_ref() != Some(&conversation) {
                run.conversation = Some(conversation);
                changed = true;
            }
        }
        if row.activity == "working" {
            self.automations.worked.insert(key.clone());
        }
        match crate::agent_state::rest_refusal(&row) {
            Some(_) => return changed,
            None if row.blocked => return changed,
            None => {}
        }
        if let Some(refused) = row.refused.as_ref() {
            return match refused.refusal.kind {
                // The account's limit: trying again before it resets would
                // be refused the same (D-16).
                // A once automation's run stays a run, failed for the limit,
                // so its one time is not a skip nobody is told of
                // (mission-104 O-3).
                hide_node_link::panes::RefusalKind::Limit => {
                    self.finish_run(key, RunState::Failed, Some("usage_limit"), Close::Now, now);
                    self.automations.viewing.remove(key);
                    if let Some(automation) = self.automation_mut(&key.0)
                        && !automation.schedule.is_once()
                    {
                        automation.skip_run(key.1, now);
                    }
                    true
                }
                // The provider refused the turn: tried again in the same
                // conversation (D-17).
                hide_node_link::panes::RefusalKind::Error => {
                    self.attempt_failed(key, Failure::Agent("provider_error".into()), now)
                }
            };
        }
        if row.completed {
            return self.run_succeeded(key, &row, now) | changed;
        }
        // The turn ended after work with no answer and no refusal. A person
        // ended it only when they were looking at the run's terminal in the
        // automation screen (B16); otherwise it is a turn that stopped with
        // no cause, as a refusal reads for an agent whose hooks do not say
        // why (PRD Risks), tried again in the same conversation (D-17).
        if self.automations.worked.contains(key) {
            if self.automations.viewing.contains_key(key) {
                self.finish_run(
                    key,
                    RunState::Stopped,
                    Some("stopped_by_person"),
                    Close::Idle,
                    now,
                );
                return true;
            }
            return self.attempt_failed(key, Failure::Agent("stopped_without_answer".into()), now);
        }
        if now.saturating_sub(since) > START_DEADLINE_MS {
            return self.attempt_failed(key, Failure::Agent("agent_not_ready".into()), now);
        }
        changed
    }

    /// The conversation an agent's registration names: the session its
    /// pane reported when the agent registered, the one its row reads once
    /// it is bound (`row_conversation`), and its agent's kind.
    fn registered_conversation(&self, agent_id: &str) -> Option<Conversation> {
        let ledger = self.delivery_state().ok()?;
        let record = ledger.agents.iter().find(|record| record.id == agent_id)?;
        if !hide_session::valid_native_id(&record.session) {
            return None;
        }
        Some(Conversation {
            kind: record.actor.kind.clone(),
            id: record.session.clone(),
        })
    }

    fn run_agent(&self, key: &RunKey) -> AgentChoice {
        self.automations
            .book
            .find(&key.0)
            .map(|automation| automation.agent.clone())
            .unwrap_or_default()
    }

    /// The task's answer (D-10, B14, B24): the run is done the moment it is
    /// recorded, so the next time is not skipped for it (D-14), and its
    /// terminal closes once nothing in it waits on the operator (B15).
    fn run_succeeded(
        &mut self,
        key: &RunKey,
        row: &crate::model::SidebarAgentSnapshot,
        now: u64,
    ) -> bool {
        let head = row
            .request
            .as_ref()
            .and_then(|request| request.answer.as_ref());
        let paragraph = head
            .and_then(|answer| answer.head.as_deref())
            .and_then(crate::automations::answer_paragraph);
        // The whole message, read where the session runs, is the answer
        // when it can be read; the row's first lines stand until then.
        let whole = head
            .filter(|answer| answer.whole)
            .map(|answer| (row.pane_id.clone(), answer.offset));
        self.finish_run(key, RunState::Succeeded, None, Close::Idle, now);
        let Some(run) = self.run_mut(key) else {
            return false;
        };
        run.answer = paragraph;
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.run_succeeded",
            "automation": key.0, "run": key.1, "attempt": run.attempt,
        }));
        if let Some((pane_id, offset)) = whole {
            self.read_whole_answer(key, pane_id, offset);
        }
        true
    }

    fn read_whole_answer(&mut self, key: &RunKey, pane_id: String, offset: u64) {
        if !self.automations.reading.insert(key.clone()) {
            return;
        }
        let read = self.whole_answer_reader(pane_id, offset);
        let worker_key = key.clone();
        let spawned = match read {
            Ok(read) => self.spawn_issue_worker("automation-answer", read, move |runtime, text| {
                runtime.apply_whole_answer(&worker_key, text)
            }),
            Err(code) => Err(code.to_owned()),
        };
        if let Err(code) = spawned {
            self.automations.reading.remove(key);
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.answer_unread",
                "automation": key.0, "run": key.1, "code": code,
            }));
        }
    }

    fn apply_whole_answer(&mut self, key: &RunKey, text: Result<String, String>) -> bool {
        self.automations.reading.remove(key);
        let text = match text {
            Ok(text) => text,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.answer_unread",
                    "automation": key.0, "run": key.1, "code": code,
                }));
                return false;
            }
        };
        let Some(paragraph) = crate::automations::answer_paragraph(&text) else {
            return false;
        };
        let Some(run) = self
            .run_mut(key)
            .filter(|run| run.state == RunState::Succeeded)
        else {
            return false;
        };
        run.answer = Some(paragraph);
        // Not on the screen: the next run reads it.
        self.save_automations();
        false
    }

    /// Hands a finished run's terminal to its close once nothing in it
    /// waits on the operator (B15): not working, nothing asked, no View
    /// unseen, not on the operator's screen or in the automation screen,
    /// and no letter on its way to it, as an automatic sleep waits
    /// (`agent_sleep::Awake`).
    /// A row missing for less than `ABSENT_GRACE_MS` holds the terminal as a
    /// running run's does: an engine restart lists it again.
    fn drive_close(&mut self, key: &RunKey, now: u64) -> bool {
        if let Some(agent_id) = self.run_of(key).and_then(|run| run.agent_id.clone()) {
            match self.run_row(&agent_id) {
                RowLook::Unknown => return false,
                RowLook::Missing => {
                    let absent = *self
                        .automations
                        .absent_since
                        .entry(key.clone())
                        .or_insert(now);
                    if now.saturating_sub(absent) <= ABSENT_GRACE_MS {
                        return false;
                    }
                }
                RowLook::Shown(row) => {
                    self.automations.absent_since.remove(key);
                    let hold = self.close_hold(key, &row);
                    if self.automations.close_held.get(key).copied() != hold {
                        crate::diagnostic!(json!({
                            "component": "automations", "kind": "automation.close_held",
                            "automation": key.0, "run": key.1, "pane_id": row.pane_id,
                            "hold": hold,
                        }));
                        match hold {
                            Some(hold) => self.automations.close_held.insert(key.clone(), hold),
                            None => self.automations.close_held.remove(key),
                        };
                    }
                    if hold.is_some() {
                        return false;
                    }
                }
                RowLook::Gone => {}
            }
        }
        self.automations.absent_since.remove(key);
        self.automations.close_held.remove(key);
        if let Some(run) = self.run_mut(key) {
            run.closing = false;
        }
        self.clean_up_run(key, now);
        true
    }

    /// What keeps a finished run's terminal open now, if anything: a
    /// refusal it rests on, a block, Needs You, the automation screen
    /// showing it, a window drawing its tab (with no window attached
    /// nothing is on screen, whatever pane the core has selected), or a
    /// letter on its way to its agent.
    fn close_hold(
        &self,
        key: &RunKey,
        row: &crate::model::SidebarAgentSnapshot,
    ) -> Option<&'static str> {
        if crate::agent_state::rest_refusal(row).is_some() {
            return Some("refusal");
        }
        if row.blocked {
            return Some("blocked");
        }
        if row.group == crate::agent_state::AgentGroup::NeedsYou.name() {
            return Some("needs_you");
        }
        if self.automations.viewing.contains_key(key) {
            return Some("viewing");
        }
        if self.ui_attached() && self.agent_sleep_on_screen(None).contains(&row.pane_id) {
            return Some("on_screen");
        }
        // The operator's own words wait for this agent's rest (PRD
        // desk-queue D-16).
        if self.input_queue_panes().contains(&row.pane_id) {
            return Some("queued_input");
        }
        let letters = self
            .delivery_state()
            .map(|ledger| crate::agent_sleep::LetterWaits::of(&ledger))
            .unwrap_or_default();
        letters.holds(row).then_some("letter")
    }

    /// Runs this machine's closes that are due on workers: a run's own close
    /// waits for a start of it still going, whose pane it may have to close.
    /// A close the book brought from another machine is that machine's.
    fn drive_cleanups(&mut self, now: u64) {
        let room = CLEANUPS_AT_ONCE.saturating_sub(self.automations.cleaning.len());
        let here = self.node.as_str();
        let due: Vec<RunKey> = self
            .automations
            .book
            .cleanups
            .iter()
            .filter(|cleanup| cleanup.machine == here && cleanup.next_at_unix_ms <= now)
            .map(|cleanup| (cleanup.automation.clone(), cleanup.run))
            .filter(|key| {
                !self.automations.cleaning.contains(key)
                    && !self.automations.starting.contains(key)
                    && !self.automations.starting.contains(&open_key(key))
            })
            .take(room)
            .collect();
        for key in due {
            self.run_cleanup(&key);
        }
    }

    /// Closes the panes the cleanup's starts made, by the spawn records of
    /// their intents, then releases the run's worktree, on a worker.
    fn run_cleanup(&mut self, key: &RunKey) {
        let Some(cleanup) = self.own_cleanup(key).cloned() else {
            return;
        };
        let generation = cleanup.generation;
        let ledger = match self.delivery_state() {
            Ok(ledger) => ledger,
            Err(reason) => {
                self.apply_cleanup(key, generation, Err(reason));
                return;
            }
        };
        let spawns: Vec<crate::coordination::SpawnRecord> = cleanup
            .intents
            .iter()
            .filter_map(|intent| {
                ledger
                    .spawns
                    .iter()
                    .rev()
                    .find(|spawn| {
                        spawn.intent == *intent
                            && spawn.task.as_ref().is_some_and(|task| {
                                task.kind == crate::coordination::TaskKind::Automation
                            })
                    })
                    .cloned()
            })
            .collect();
        let local = self.node.clone();
        let engine = self.delivery_engine(local.as_str());
        let node = cleanup
            .worktree
            .is_some()
            .then(|| self.node_link(local.as_str()).ok())
            .flatten();
        let worker_key = key.clone();
        let worktree = cleanup.worktree.clone();
        let spawned = self.spawn_issue_worker(
            "automation-close",
            move || close_and_release(engine, node, &spawns, worktree.as_ref()),
            move |runtime, closed| runtime.apply_cleanup(&worker_key, generation, closed),
        );
        match spawned {
            Ok(()) => {
                self.automations.cleaning.insert(key.clone());
            }
            Err(message) => {
                self.apply_cleanup(key, generation, Err(message));
            }
        }
    }

    /// This machine's close of the run.
    fn own_cleanup(&self, key: &RunKey) -> Option<&crate::automations::Cleanup> {
        self.automations.book.cleanups.iter().find(|cleanup| {
            cleanup.automation == key.0
                && cleanup.run == key.1
                && cleanup.machine == self.node.as_str()
        })
    }

    /// A close's answer: done, unless more was added while it ran; a failure
    /// is tried again later, spaced out, and kept across restarts, until its
    /// last try, when it is given up with a diagnostic (engineering 15).
    fn apply_cleanup(&mut self, key: &RunKey, generation: u64, closed: Result<(), String>) -> bool {
        self.automations.cleaning.remove(key);
        let now = super::unix_milliseconds();
        let here = self.node.as_str().to_owned();
        let Some(index) = self.automations.book.cleanups.iter().position(|cleanup| {
            cleanup.automation == key.0 && cleanup.run == key.1 && cleanup.machine == here
        }) else {
            return false;
        };
        match closed {
            Ok(()) if self.automations.book.cleanups[index].generation == generation => {
                self.automations.book.cleanups.remove(index);
            }
            Ok(()) => {}
            Err(message) => {
                let cleanup = &mut self.automations.book.cleanups[index];
                cleanup.tries = cleanup.tries.saturating_add(1);
                if cleanup.tries >= CLEANUP_TRIES {
                    let dropped = self.automations.book.cleanups.remove(index);
                    crate::diagnostic!(json!({
                        "component": "automations", "kind": "automation.cleanup_abandoned",
                        "automation": key.0, "run": key.1, "tries": dropped.tries,
                        "intents": dropped.intents,
                        "branch": dropped.worktree.map(|worktree| worktree.branch),
                        "message": message,
                    }));
                } else {
                    let wait = CLEANUP_RETRY_MS
                        .saturating_mul(1 << cleanup.tries.min(16))
                        .min(CLEANUP_RETRY_MAX_MS);
                    cleanup.next_at_unix_ms = now.saturating_add(wait);
                    crate::diagnostic!(json!({
                        "component": "automations", "kind": "automation.cleanup_failed",
                        "automation": key.0, "run": key.1, "tries": cleanup.tries,
                        "message": message,
                    }));
                }
            }
        }
        self.save_automations();
        false
    }

    /// Follows a finished run's conversation opened again (B17, B18): its
    /// start is asked under its intent as an attempt's is (`drive_ask`),
    /// after a restart too; it closes by Hide's sleep (its pane, once
    /// asleep), and one whose start never left or whose agent left before
    /// it ever showed could not open the conversation.
    fn follow_opened(&mut self, key: &RunKey, now: u64) -> bool {
        let Some(opened) = self.run_of(key).and_then(|run| run.opened.clone()) else {
            return false;
        };
        let track = open_key(key);
        let Some(agent_id) = opened.agent_id else {
            if opened.ask.is_none() {
                return self.end_opening(key, opened.number, true, now);
            }
            return self.drive_ask(&track, now);
        };
        let since = *self
            .automations
            .attempt_since
            .entry(track.clone())
            .or_insert(now);
        let shown = self.automations.shown.contains(&track);
        let (ended, missing) = match self.run_row(&agent_id) {
            RowLook::Unknown => return false,
            RowLook::Gone => (true, !shown),
            RowLook::Missing => {
                let late = if shown {
                    let absent = *self
                        .automations
                        .absent_since
                        .entry(track.clone())
                        .or_insert(now);
                    now.saturating_sub(absent) > ABSENT_GRACE_MS
                } else {
                    now.saturating_sub(since) > START_DEADLINE_MS
                };
                (late, late && !shown)
            }
            RowLook::Shown(row) => {
                self.automations.shown.insert(track.clone());
                self.automations.absent_since.remove(&track);
                (row.sleep.is_some(), false)
            }
        };
        if !ended {
            return false;
        }
        self.end_opening(key, opened.number, missing, now)
    }

    /// The opening `number` of the run ended: its terminal closes, and the
    /// run's own worktree is released again, since an opening works there.
    fn end_opening(&mut self, key: &RunKey, number: u32, missing: bool, now: u64) -> bool {
        self.forget(&open_key(key));
        let Some(automation) = self.automations.book.find(&key.0) else {
            return false;
        };
        let worktree = automation
            .runs()
            .find(|run| run.number == key.1)
            .and_then(|run| run_worktree(automation, run));
        let here = self.node.as_str().to_owned();
        self.automations.book.clean_up(
            &key.0,
            key.1,
            &here,
            vec![open_intent(&key.0, key.1, number)],
            worktree,
            now,
        );
        let Some(run) = self.run_mut(key) else {
            return true;
        };
        if run
            .opened
            .as_ref()
            .is_some_and(|opened| opened.number == number)
        {
            run.opened = None;
        }
        run.conversation_missing |= missing;
        true
    }

    /// The start of the run's conversation opened again, the `opening`th
    /// time; none when its conversation cannot be resumed.
    fn open_command(&self, key: &RunKey, opening: u32) -> Option<crate::coordination::Command> {
        let automation = self.automations.book.find(&key.0)?;
        let run = automation.runs().find(|run| run.number == key.1)?;
        let conversation = run.conversation.as_ref()?;
        let args = resume_arguments(conversation)?;
        Some(crate::coordination::Command::Spawn {
            pick: None,
            mission: None,
            plan_node: None,
            plan: None,
            parent: None,
            machine: None,
            name: format!("auto-{}-r{}-o{opening}", automation.id, key.1),
            intent: open_intent(&automation.id, key.1, opening),
            kind: conversation.kind.clone(),
            repo: Some(automation.folder.path.clone()),
            branch: run.branch.clone().or_else(|| {
                (!automation.folder.default && !automation.folder.worktree)
                    .then(|| self.root_branch(self.node.as_str(), &automation.folder.path))
                    .flatten()
            }),
            path: None,
            args,
            launch: hide_agent_adapter::LaunchChoice {
                model: automation.agent.model.clone(),
                effort: automation.agent.effort.clone(),
            },
            brief: None,
            task: Some(crate::coordination::SpawnTask {
                kind: crate::coordination::TaskKind::Automation,
                target: Some(automation.id.clone()),
            }),
            beside: false,
            profile: automation.agent.profile.clone(),
        })
    }

    /// The CLI's and the windows' calls (B10): an answer, and whether the
    /// book changed so the screen is sent again.
    pub(crate) fn automation_call(&mut self, call: Call) -> (Result<Value, String>, bool) {
        let now = super::unix_milliseconds();
        let answer = match call {
            Call::Add(draft) => self.add_automation(draft, now),
            Call::Edit { id, changes } => self.edit_automation(&id, changes, now),
            Call::Pause { id } => self.pause_automation(&id),
            Call::Resume { id } => self.resume_automation(&id, now),
            Call::Run { id } => self.run_now(&id, now),
            Call::Remove { id } => self.remove_automation(&id, now),
            Call::RunStop { id, run } => self.stop_run(&id, run, now),
            Call::RunOpen { id, run } => self.open_run(&id, run, now),
            Call::Seen { id } => self.see_automation(&id),
            Call::LookSeen { id, run } => self.see_look(&id, run),
            Call::RunViewing {
                id,
                run,
                on,
                restore,
            } => {
                let before = (!on).then(|| self.viewed_run_pane(&id, run)).flatten();
                let (answer, laid) = match self.view_run(&id, run, on, now) {
                    Ok((value, laid)) => (Ok(value), laid),
                    Err(code) => (Err(code), false),
                };
                let synced = self.sync_viewed_panes();
                let restored = match (before, restore) {
                    (Some(pane), Some(back)) => self.give_focus_back(&pane, back),
                    _ => false,
                };
                return (answer, laid || synced || restored);
            }
            Call::RunResultOpen { id, run, key } => {
                return match self.open_run_result(&id, run, &key) {
                    Ok(value) => (Ok(value), true),
                    Err(code) => {
                        crate::diagnostic!(json!({
                            "component": "automations",
                            "kind": "automation.result_open_refused",
                            "automation": id, "run": run, "code": code,
                        }));
                        (Err(code.to_owned()), false)
                    }
                };
            }
            Call::List => {
                return (Ok(json!({"items": self.automations_view().items})), false);
            }
            Call::Show { id } => {
                let view = self.automations_view();
                let found = self
                    .automations
                    .book
                    .find(&id)
                    .and_then(|automation| view.items.iter().find(|item| item.id == automation.id))
                    .map(|item| json!(item))
                    .ok_or_else(|| "automation_unknown".to_owned());
                return (found, false);
            }
            Call::Preview { schedule, name } => {
                return (self.preview(schedule, &name, now), false);
            }
        };
        match answer {
            Ok(value) => {
                self.automations_changed(true);
                (Ok(value), true)
            }
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.call_refused",
                    "reason": code,
                }));
                (Err(code.to_owned()), false)
            }
        }
    }

    /// A window's event: the same call, its refusal to the log.
    pub(super) fn automation_event(&mut self, call: Call) -> bool {
        self.automation_call(call).1
    }

    /// A window shows the run's terminal in the automation screen, or no
    /// longer does: while it does, the terminal is on screen and is not
    /// closed (B15). The word holds `VIEWING_HOLD_MS` unless sent again.
    ///
    /// Showing it also puts the run's summary card first on its tab's desk
    /// and brings onto the desk what its agent opened unwatched
    /// (`place_run_summary`, `gather_run_views`); whether either moved a
    /// View, or the run's once line was put away, comes back with the
    /// answer, so the change is announced even when the run's terminal has
    /// closed and nothing else moved.
    fn view_run(
        &mut self,
        id: &str,
        number: u64,
        on: bool,
        now: u64,
    ) -> Result<(Value, bool), String> {
        let id = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.id.clone())
            .ok_or_else(|| "automation_unknown".to_owned())?;
        let key = (id.clone(), number);
        let mut laid = false;
        if on {
            let arriving = !self.automations.viewing.contains_key(&key);
            self.automations.viewing.insert(key.clone(), now);
            if arriving {
                laid |= self.put_away_viewed_run(&key);
            }
            if let Some(pane) = self.viewed_run_pane(&id, number) {
                laid |= self.place_run_summary(&pane, &id, number);
                laid |= self.gather_run_views(&pane);
            }
            while self.automations.viewing.len() > VIEWING_CAP {
                let oldest = self
                    .automations
                    .viewing
                    .iter()
                    .min_by_key(|(_, since)| **since)
                    .map(|(key, _)| key.clone())
                    .expect("over the cap");
                self.automations.viewing.remove(&oldest);
            }
        } else {
            self.automations.viewing.remove(&key);
        }
        Ok((json!({"id": id, "run": number, "on": on}), laid))
    }

    /// The window moved the focus to `pane` to show it: when it is the
    /// terminal of a run the screen shows, its desk gets the summary card and
    /// what the agent opened unwatched, as when the screen said it shows the
    /// run. The word can come before the run has a pane or a tab (a run
    /// still starting), so the pane arriving is the second moment it is laid.
    pub(super) fn lay_viewed_run_desk(&mut self, pane: &str) {
        // At most `VIEWING_CAP` runs are shown, so the pane is asked of those only.
        let Some(key) = self
            .automations
            .viewing
            .keys()
            .find(|(id, number)| self.viewed_run_pane(id, *number).as_deref() == Some(pane))
            .cloned()
        else {
            return;
        };
        self.place_run_summary(pane, &key.0, key.1);
        self.gather_run_views(pane);
    }

    /// The pane of the run `number` of `id` while it has one.
    pub(super) fn viewed_run_pane(&self, id: &str, number: u64) -> Option<String> {
        let id = self.automations.book.find(id)?.id.clone();
        let run = self.run_of(&(id, number))?;
        let ledger = self.delivery_state().ok();
        self.run_pane(run, ledger.as_ref())
    }

    /// The run whose terminal is `pane`, as the core names it, while the
    /// run goes or its conversation is open again.
    fn run_of_pane(&self, pane: &str) -> Option<RunKey> {
        let ledger = self.delivery_state().ok();
        self.automations
            .book
            .automations
            .iter()
            .find_map(|automation| {
                automation
                    .runs()
                    .find(|run| self.run_pane(run, ledger.as_ref()).as_deref() == Some(pane))
                    .map(|run| (automation.id.clone(), run.number))
            })
    }

    /// An agent's `hide file|diff|browser open` in `pane` landed in the View
    /// `view_id` of `key` (`stamp_control_opener`): when `pane` is a run's
    /// terminal, the View is one of the run's results, the newest, once. A
    /// View that keeps no address (an outside file) or one past
    /// `RESULT_ADDRESS_LIMIT` is not recorded. The book is saved off the lock
    /// as every run change is.
    pub(super) fn record_run_result(
        &mut self,
        key: &WorkspaceKey,
        view_id: &str,
        pane: &str,
        now: u64,
    ) {
        if self.automations.book.automations.is_empty() {
            return;
        }
        let pane = crate::address::named_pane_id(self.node.as_str(), &key.0, pane);
        let Some((id, number)) = self.run_of_pane(&pane) else {
            return;
        };
        let Some(display) = self.view_display(key, view_id) else {
            return;
        };
        let Some(kept) = KeptView::of(
            display,
            super::view_areas::display_label(display),
            RESULT_ADDRESS_LIMIT,
        ) else {
            // A View with no address to open again (an outside file, a card)
            // is quietly not a result; one whose address is too long is said.
            let bytes = match display.kind {
                DisplayKind::File | DisplayKind::Diff => display.path.len(),
                DisplayKind::Browser => display.url.as_ref().map_or(0, String::len),
                DisplayKind::External | DisplayKind::Card => 0,
            };
            if bytes > RESULT_ADDRESS_LIMIT {
                crate::diagnostic!(serde_json::json!({
                    "component": "automations", "kind": "automation.result_not_kept",
                    "automation": id, "run": number, "address_bytes": bytes,
                    "limit": RESULT_ADDRESS_LIMIT,
                }));
            }
            return;
        };
        // The run's own agent is the opener, found again by its pane when a
        // result is pressed, so the book keeps no agent's identity per row.
        let result = RunResult {
            display_id: view_id.to_owned(),
            at_unix_ms: now,
            kept: KeptView {
                opener: None,
                ..kept
            },
        };
        let changed = self
            .automation_mut(&id)
            .is_some_and(|automation| automation.record_result(number, result));
        if changed {
            self.automations_changed(true);
        }
    }

    /// The operator pressed a result row of the run's screen
    /// (`automation_run_result_open`): the View comes in front on the run's
    /// desk, the open one as it is, a closed one opened again from what the
    /// result kept, as the View of the run's agent. Refused, with the code
    /// the log names, when the run has no terminal now, its desk is not the
    /// one in front, the row is not one of its results, or the open was
    /// refused.
    fn open_run_result(&mut self, id: &str, number: u64, key: &str) -> Result<Value, &'static str> {
        use super::mission_views::KeptOpenRefusal;
        let id = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.id.clone())
            .ok_or("automation_unknown")?;
        let run = self
            .run_of(&(id.clone(), number))
            .ok_or("automation_run_unknown")?;
        let result = run
            .results
            .iter()
            .find(|result| result.key() == key)
            .cloned()
            .ok_or("automation_result_unknown")?;
        let pane = self
            .viewed_run_pane(&id, number)
            .ok_or("automation_run_no_pane")?;
        let place = self
            .pane_workspace_key(&pane)
            .ok_or("automation_run_no_pane")?;
        // The View opens on the desk in front: the run's, while the screen
        // shows it (its focus is on the run's pane).
        if self.pane_tab(&place, &pane).as_deref() != Some(self.front_tab(&place).as_str()) {
            return Err("automation_run_desk_not_in_front");
        }
        let kept = KeptView {
            opener: self.opener_of_pane(&pane),
            ..result.kept
        };
        match self.open_kept_view(&place, Some(&result.display_id), Some(&kept)) {
            Ok(open) => Ok(json!({"id": id, "run": number, "open": open})),
            Err(KeptOpenRefusal::CheckoutGone) => Err("automation_run_checkout_gone"),
            Err(KeptOpenRefusal::NotKept) => Err("automation_result_not_kept"),
            Err(KeptOpenRefusal::Page(refusal)) => {
                crate::diagnostic!(json!({
                    "component": "automations", "kind": "automation.result_page_refused",
                    "automation": id, "run": number, "refusal": refusal,
                }));
                Err("automation_result_open_refused")
            }
            Err(KeptOpenRefusal::Refused) => Err("automation_result_open_refused"),
        }
    }

    /// The automation screen left a run whose pane it had given the focus
    /// (operator decision 2026-11-21): the pane the operator was on takes it
    /// back, unless something since moved the focus off the run's pane (a
    /// press that opened another pane arrives before the screen's leaving,
    /// so it wins).
    fn give_focus_back(&mut self, run_pane: &str, back: String) -> bool {
        if self.snapshot.focused.pane_id.as_deref() != Some(run_pane) || back == run_pane {
            return false;
        }
        self.focus_pane(back, super::events::PaneFocusOrigin::Operator, None);
        true
    }

    /// A finished run the operator opened is dealt with (operator decision
    /// 2026-11-21): its row is resolved, so it leaves Needs You, and comes
    /// back when the run's agent asks again. A run going, or an agent that
    /// works or waits on an approval, keeps its row (`resolve_refusal`). A
    /// once automation's line for the run goes too (mission-104 O-4); the
    /// answer is whether it did.
    fn put_away_viewed_run(&mut self, key: &RunKey) -> bool {
        let mut put_away = false;
        if self.run_of(key).is_some_and(|run| !run.state.active())
            && let Some(automation) = self.automation_mut(&key.0)
            && automation.look_run == Some(key.1)
        {
            automation.look_run = None;
            self.automations_changed(true);
            put_away = true;
        }
        let Some(run) = self.run_of(key).filter(|run| !run.state.active()) else {
            return put_away;
        };
        let ledger = self.delivery_state().ok();
        let Some(pane) = self.run_pane(run, ledger.as_ref()) else {
            return put_away;
        };
        self.resolve_session(&pane, crate::agent_state::sessions::ResolveSource::Operator);
        put_away
    }

    /// Puts on each run what its conversation spent, from the sessions the
    /// usage ledger's last report priced (D-26, B39): read on the ledger's
    /// worker, joined here by the conversation's agent and session id when a
    /// report lands, so nothing is read under the lock. A conversation's
    /// tokens only grow, so a report with fewer than the run keeps changes
    /// nothing: the ledger's seven days are cut by day, and a conversation
    /// whose first day left them reads smaller. A run the report does not
    /// hold, older than its seven days or not read yet, keeps what it had.
    pub(super) fn price_automation_runs(&mut self) -> bool {
        let Some(projects) = self.ledger_spends.clone() else {
            return false;
        };
        let key = |kind: &str, session: &str| {
            (
                hide_agent_adapter::canonical_kind(kind).to_owned(),
                session.to_owned(),
            )
        };
        let wanted: HashSet<(String, String)> = self
            .automations
            .book
            .automations
            .iter()
            .flat_map(|automation| automation.runs())
            .filter_map(|run| run.conversation.as_ref())
            .map(|conversation| key(&conversation.kind, &conversation.id))
            .collect();
        if wanted.is_empty() {
            return false;
        }
        let mut spends: HashMap<(String, String), crate::automations::Spend> = HashMap::new();
        // A session that worked in two projects is priced in each.
        for spend in projects.iter().flat_map(|project| &project.spends) {
            let session = key(&spend.agent, &spend.session_id);
            if wanted.contains(&session) {
                spends
                    .entry(session)
                    .or_default()
                    .add(crate::automations::Spend {
                        tokens: spend.tokens,
                        micro_usd: spend.micro_usd,
                        unpriced: spend.unpriced,
                    });
            }
        }
        let mut changed = false;
        for automation in &mut self.automations.book.automations {
            for entry in &mut automation.records {
                let crate::automations::Entry::Run(run) = entry else {
                    continue;
                };
                let read = run.conversation.as_ref().and_then(|conversation| {
                    spends.get(&key(&conversation.kind, &conversation.id))
                });
                if let Some(read) = read
                    && run.spend.as_ref() != Some(read)
                    && run.spend.is_none_or(|had| read.tokens >= had.tokens)
                {
                    run.spend = Some(*read);
                    changed = true;
                }
            }
        }
        if changed {
            self.automations_changed(true);
        }
        changed
    }

    /// The form's preview (B5, B6). `own_folder` is where an automation of
    /// `name` with no folder of its own works, its id still to come
    /// (`ID_TO_COME`), so the form names it by the core's rule; none while
    /// the home is not known.
    fn preview(
        &self,
        schedule: crate::automations::Asked,
        name: &str,
        now: u64,
    ) -> Result<Value, String> {
        let zone = self.zone();
        let schedule = schedule.checked(now, &zone).map_err(str::to_owned)?;
        Ok(json!({
            "reads_as": schedule.meaning().reads_as,
            "next_run_at_unix_ms": schedule.next_after(now, &zone),
            "zone": zone.iana_name(),
            "own_folder": self.own_folder(name, crate::automations::ID_TO_COME).ok(),
        }))
    }

    /// The folder a draft names, checked: a project of this machine, and a
    /// Git one for a worktree per run (B5, B7).
    fn checked_folder(
        &self,
        folder: Option<String>,
        worktree: bool,
        own: impl FnOnce() -> Result<String, &'static str>,
    ) -> Result<Folder, &'static str> {
        use super::projects::CheckoutKind;
        let Some(path) = folder.map(|path| path.trim_end_matches('/').to_owned()) else {
            if worktree {
                return Err("automation_worktree_needs_git");
            }
            return Ok(Folder {
                path: own()?,
                default: true,
                worktree: false,
            });
        };
        let kind = self.checkout_kind(self.node.as_str(), &path);
        match kind {
            None | Some(CheckoutKind::Unreadable) => Err("automation_folder_unknown"),
            Some(CheckoutKind::Git) => Ok(Folder {
                path,
                default: false,
                worktree,
            }),
            Some(CheckoutKind::Folder | CheckoutKind::Unread) if worktree => {
                Err("automation_worktree_needs_git")
            }
            Some(_) => Ok(Folder {
                path,
                default: false,
                worktree: false,
            }),
        }
    }

    fn own_folder(&self, name: &str, id: &str) -> Result<String, &'static str> {
        let home = self.home_path.as_ref().ok_or("automation_home_unknown")?;
        let path = hide_kit::layout::automations_home(home)
            .join(crate::automations::own_folder_name(name, id));
        hide_platform::path::to_wire(&path).map_err(|_| "automation_home_unknown")
    }

    /// The registration of an automation's own folder goes with the
    /// automation, or when it leaves the folder: nothing else lists it.
    fn retire_own_folder(&mut self, path: &str) {
        let registration = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .find(|row| row.device_id == self.node.as_str() && row.path == path)
            .map(|row| row.id.clone());
        if let Some(registration) = registration {
            self.retire_workspace_registration(&registration);
        }
    }

    fn add_automation(&mut self, draft: Draft, now: u64) -> Result<Value, &'static str> {
        if self.automations.store.is_none() {
            return Err("automations_unavailable");
        }
        let full = self.automations.book.automations.len() >= crate::automations::AUTOMATION_CAP;
        if full && self.automations.book.oldest_done().is_none() {
            return Err("automation_cap");
        }
        let zone = self.zone();
        let name = crate::automations::checked_name(&draft.name)?;
        let task = crate::automations::checked_task(&draft.task)?;
        let schedule = draft.schedule.checked(now, &zone)?;
        if self.automations.book.find(&name).is_some() {
            return Err("automation_name_taken");
        }
        let agent = crate::automations::checked_agent(&draft.agent)?;
        let taken: Vec<&str> = self
            .automations
            .book
            .automations
            .iter()
            .map(|automation| automation.id.as_str())
            .collect();
        let id = crate::automations::new_id(now, &taken);
        let folder =
            self.checked_folder(draft.folder, draft.worktree, || self.own_folder(&name, &id))?;
        // At the cap the done once automation that ended first goes, its
        // records with it and its folder kept (mission-104 O-5), once the
        // new one is known to be made.
        if full && let Some(index) = self.automations.book.oldest_done() {
            let dropped = self.automations.book.automations[index].id.clone();
            self.remove_automation(&dropped, now)?;
            crate::diagnostic!(json!({
                "component": "automations", "kind": "automation.cap_dropped",
                "automation": dropped, "cap": crate::automations::AUTOMATION_CAP,
            }));
        }
        let automation = Automation {
            id: id.clone(),
            name,
            task,
            next_due_unix_ms: schedule.next_after(now, &zone),
            schedule,
            agent: agent.clone(),
            folder,
            machine: self.node.as_str().to_owned(),
            paused: None,
            created_at_unix_ms: now,
            records: Vec::new(),
            next_run_number: 0,
            look_run: None,
        };
        let mut answer = json!({
            "id": id,
            "name": automation.name,
            "folder": automation.folder.path,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        });
        let once = automation.schedule.is_once();
        self.automations.book.automations.push(automation);
        self.automations.book.last_choice = Some(agent);
        if let Some(sink) = &self.product_analytics {
            use crate::automations::Example;
            use crate::product_analytics::{AutomationSchedule, AutomationStart, Event};
            let start = match draft.example {
                Some(Example::Report) => AutomationStart::Report,
                Some(Example::Briefing) => AutomationStart::Briefing,
                Some(Example::Ci) => AutomationStart::Ci,
                None => AutomationStart::Own,
            };
            let schedule = if once {
                AutomationSchedule::Once
            } else {
                AutomationSchedule::Repeat
            };
            sink.capture(Event::AutomationCreated { start, schedule });
        }
        // The operator works in the screen that said the book before was
        // lost: the line has been read.
        self.automations.book.unreadable = None;
        // Made and run now (B8): the run is judged as [지금 실행] judges it.
        if draft.run_now {
            let run = self.run_now(&id, now)?;
            answer["run"] = run["run"].clone();
            if let Some(paused) = run.get("paused") {
                answer["paused"] = paused.clone();
                answer["next_run_at_unix_ms"] = Value::Null;
            }
        }
        Ok(answer)
    }

    fn edit_automation(
        &mut self,
        id: &str,
        changes: Changes,
        now: u64,
    ) -> Result<Value, &'static str> {
        let zone = self.zone();
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let current = self.automations.book.automations[index].clone();
        let name = changes
            .name
            .as_deref()
            .map(crate::automations::checked_name)
            .transpose()?;
        if let Some(name) = &name
            && self
                .automations
                .book
                .find(name)
                .is_some_and(|other| other.id != current.id)
        {
            return Err("automation_name_taken");
        }
        let task = changes
            .task
            .as_deref()
            .map(crate::automations::checked_task)
            .transpose()?;
        let schedule = changes
            .schedule
            .map(|schedule| schedule.checked(now, &zone))
            .transpose()?;
        let agent = changes
            .agent
            .as_ref()
            .map(crate::automations::checked_agent)
            .transpose()?;
        let folder = match (changes.folder, changes.worktree) {
            (None, None) => None,
            (folder, worktree) => {
                let path = match folder {
                    Some(path) => path,
                    None => (!current.folder.default).then(|| current.folder.path.clone()),
                };
                let worktree = worktree.unwrap_or(current.folder.worktree);
                let own_name = name.clone().unwrap_or_else(|| current.name.clone());
                let folder = self.checked_folder(path, worktree, || {
                    // Its own folder keeps the name it was made under.
                    if current.folder.default {
                        Ok(current.folder.path.clone())
                    } else {
                        self.own_folder(&own_name, &current.id)
                    }
                })?;
                Some(folder)
            }
        };
        // Leaving its own folder takes that folder's registration with it.
        if current.folder.default
            && folder
                .as_ref()
                .is_some_and(|folder| folder.path != current.folder.path)
        {
            self.retire_own_folder(&current.folder.path);
        }
        // A run going keeps what it started with; the next one takes the
        // new values (B26).
        let automation = &mut self.automations.book.automations[index];
        if let Some(name) = name {
            automation.name = name;
        }
        if let Some(task) = task {
            automation.task = task;
        }
        // A new schedule takes effect from its next time: a done once
        // automation given a new time or a repeating schedule waits again
        // (mission-104 O-3).
        // A time it is given here is this machine's to keep, as a resume
        // makes it (mission-104 review m2).
        if let Some(schedule) = schedule {
            automation.schedule = schedule;
            if automation.on() {
                automation.next_due_unix_ms = automation.schedule.next_after(now, &zone);
                if automation.next_due_unix_ms.is_some() {
                    automation.machine = self.node.as_str().to_owned();
                }
            }
        }
        if let Some(agent) = agent.clone() {
            automation.agent = agent;
        }
        if let Some(folder) = folder {
            automation.folder = folder;
        }
        let answer = json!({
            "id": automation.id,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        });
        if let Some(agent) = agent {
            self.automations.book.last_choice = Some(agent);
        }
        Ok(answer)
    }

    /// The run going continues to its end; the next time does not come
    /// (B25).
    fn pause_automation(&mut self, id: &str) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        // A once automation whose one time was taken, done or with its run
        // going, has no time left to stop (mission-104 O-3).
        if automation.schedule.is_once() && automation.on() && automation.next_due_unix_ms.is_none()
        {
            return Err("automation_done");
        }
        if automation.paused.is_none() {
            automation.paused = Some(Pause {
                reason: PauseReason::Operator,
                at_unix_ms: super::unix_milliseconds(),
            });
            automation.next_due_unix_ms = None;
        }
        Ok(json!({"id": automation.id}))
    }

    /// On again from the next time (B23, B25); a person's line goes with it,
    /// and an automation that moved with the core runs on this machine now.
    fn resume_automation(&mut self, id: &str, now: u64) -> Result<Value, &'static str> {
        let zone = self.zone();
        let here = self.node.as_str().to_owned();
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        automation.machine = here;
        if automation.paused.take().is_some() {
            // A once automation whose one time passed while it was paused
            // runs now, once, rather than end unrun (mission-104 O-3).
            let passed = automation.schedule.is_once().then_some(now);
            automation.next_due_unix_ms = automation.schedule.next_after(now, &zone).or(passed);
        }
        Ok(json!({
            "id": automation.id,
            "next_run_at_unix_ms": automation.next_due_unix_ms,
        }))
    }

    /// A manual run (B19): refused while a run goes, so a second press
    /// starts nothing (B36).
    fn run_now(&mut self, id: &str, now: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        if self.automations.book.automations[index]
            .active_run()
            .is_some()
        {
            return Err("automation_running");
        }
        let gate = self.run_gate(&self.automations.book.automations[index].agent, now, false);
        let automation = &mut self.automations.book.automations[index];
        let number = automation.new_run(Trigger::Manual, None, now);
        let id = automation.id.clone();
        // A run that cannot start for a person's reason is recorded and
        // stops the automation (B23); the answer says why.
        if let Gate::Pause(reason) = gate {
            self.pause_for_person(&(id.clone(), number), reason, now);
            return Ok(json!({"id": id, "run": number, "paused": reason.code()}));
        }
        Ok(json!({"id": id, "run": number}))
    }

    /// Removes the automation and its records; the terminals of its runs
    /// and its runs' own worktrees go, its folder and what is in it stay
    /// (B27, D-11). What is left to close is added to the book's closes
    /// before the records go, so the close finishes even after a restart.
    /// Its own folder's registration goes with it.
    fn remove_automation(&mut self, id: &str, now: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let id = self.automations.book.automations[index].id.clone();
        let open: Vec<u64> = self.automations.book.automations[index]
            .runs()
            .filter(|run| run.state.active() || run.closing || run.opened.is_some())
            .map(|run| run.number)
            .collect();
        for number in open {
            let key = (id.clone(), number);
            self.forget(&key);
            self.forget(&open_key(&key));
            self.clean_up_run(&key, now);
        }
        self.automations.viewing.retain(|key, _| key.0 != id);
        self.automations.close_held.retain(|key, _| key.0 != id);
        let automation = self.automations.book.automations.remove(index);
        if automation.folder.default {
            self.retire_own_folder(&automation.folder.path);
        }
        Ok(json!({"id": automation.id, "records": automation.records.len()}))
    }

    /// A person ends the run going (B16): its terminal ends and the run
    /// stays as stopped.
    fn stop_run(&mut self, id: &str, number: u64, now: u64) -> Result<Value, &'static str> {
        let id = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.id.clone())
            .ok_or("automation_unknown")?;
        let key = (id.clone(), number);
        let run = self.run_of(&key).ok_or("automation_run_unknown")?;
        if !run.state.active() {
            return Err("automation_run_finished");
        }
        self.finish_run(
            &key,
            RunState::Stopped,
            Some("stopped_by_person"),
            Close::Now,
            now,
        );
        Ok(json!({"id": id, "run": number}))
    }

    /// Opens a finished run's conversation again in a terminal (B17): the
    /// one still open is answered, a new one resumes the conversation under
    /// the run's next opening number, so every opening is a start of its own
    /// (B18). It is not a run going, so the next time is not skipped for it.
    fn open_run(&mut self, id: &str, number: u64, now: u64) -> Result<Value, &'static str> {
        let id = self
            .automations
            .book
            .find(id)
            .map(|automation| automation.id.clone())
            .ok_or("automation_unknown")?;
        let key = (id.clone(), number);
        let run = self.run_of(&key).ok_or("automation_run_unknown")?;
        if !run.finished() {
            return Err("automation_run_going");
        }
        if run.closing || run.opened.is_some() {
            return Ok(json!({"id": id, "run": number}));
        }
        // One opening of the automation starts at a time: a start is kept
        // until it leaves, which bounds the book (engineering 15).
        if self.automations.book.find(&id).is_some_and(|automation| {
            automation.runs().any(|run| {
                run.opened
                    .as_ref()
                    .is_some_and(|opened| opened.ask.is_some())
            })
        }) {
            return Err("automation_opening");
        }
        let opening = run.opens + 1;
        let command = self
            .open_command(&key, opening)
            .ok_or("automation_conversation_missing")?;
        let run = self.run_mut(&key).expect("found above");
        run.opens = opening;
        run.opened = Some(Opened {
            number: opening,
            agent_id: None,
            ask: Some(Ask {
                command: Some(command),
                worktree: None,
            }),
        });
        run.conversation_missing = false;
        run.seen = true;
        self.drive_ask(&open_key(&key), now);
        Ok(json!({"id": id, "run": number}))
    }

    /// The operator looked at the automation (B11).
    /// A once automation's line for `run` goes from Needs You, as it does
    /// when the automation screen shows the run; a newer run's line stays.
    fn see_look(&mut self, id: &str, run: u64) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        if automation.look_run == Some(run) {
            automation.look_run = None;
        }
        Ok(json!({"id": automation.id, "look_run": automation.look_run}))
    }

    fn see_automation(&mut self, id: &str) -> Result<Value, &'static str> {
        let index = self
            .automations
            .book
            .position(id)
            .ok_or("automation_unknown")?;
        let automation = &mut self.automations.book.automations[index];
        for entry in &mut automation.records {
            if let Entry::Run(run) = entry
                && run.finished()
            {
                run.seen = true;
            }
        }
        Ok(json!({"id": automation.id}))
    }

    /// The panes of the runs' terminals, each run's or its opened
    /// conversation's, as the section names them.
    /// Marks each catalog tab whose panes are all automation runs'
    /// (`TabSnapshot::automation`), on this machine's checkouts and each
    /// device's, so the sidebar's tree and its numbers leave it out as the tab
    /// strip does (`Runtime::automation_run_tabs`).
    pub(super) fn mark_automation_run_tabs(&mut self) {
        let runs: std::collections::HashSet<String> =
            self.automation_run_panes().map(str::to_owned).collect();
        let sessions = self
            .snapshot
            .status
            .remote
            .iter_mut()
            .filter_map(|remote| remote.session.as_mut())
            .flat_map(|session| session.workspaces.iter_mut());
        for workspace in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .chain(sessions)
        {
            for tab in workspace
                .checkouts
                .iter_mut()
                .flat_map(|checkout| checkout.tabs.iter_mut())
            {
                tab.automation =
                    !tab.panes.is_empty() && tab.panes.iter().all(|pane| runs.contains(&pane.id));
            }
        }
    }

    pub(super) fn automation_run_panes(&self) -> impl Iterator<Item = &str> {
        self.snapshot
            .automations
            .iter()
            .flat_map(|section| &section.items)
            .flat_map(|automation| &automation.records)
            .filter_map(|record| match record {
                crate::automations::RecordSnapshot::Run(run) => run.pane_id.as_deref(),
                _ => None,
            })
    }

    /// The `automations` section, made again only when the book or the
    /// ledger moved (a run's pane is read from its agent's registration).
    pub(super) fn refresh_automations(&mut self) {
        // With no store, only a book that could not be read is shown.
        if self.automations.store.is_none() && self.automations.book.unreadable.is_none() {
            return;
        }
        let ledger = self.delivery_state().ok();
        let panes = self
            .automations
            .live_agents
            .iter()
            .map(|agent| {
                ledger.as_ref().and_then(|ledger| {
                    ledger
                        .agents
                        .iter()
                        .find(|record| &record.id == agent && !record.ended)
                        .map(|record| record.pane_on(self.node.as_str()))
                })
            })
            .collect();
        // The runs a window shows carry their answer and results (`shown`),
        // so the section is made again when that set moves.
        let mut shown: Vec<RunKey> = self.automations.viewing.keys().cloned().collect();
        shown.sort();
        let inputs = (self.automations.edit, panes, shown);
        if self.automations.composed.as_ref() == Some(&inputs) {
            return;
        }
        self.automations.composed = Some(inputs);
        let view = Arc::new(self.automations_view());
        if self.snapshot.automations.as_deref() != Some(&*view) {
            self.snapshot.automations = Some(view);
        }
    }

    fn automations_view(&self) -> crate::automations::AutomationsSnapshot {
        let zone = self.zone();
        let ledger = self.delivery_state().ok();
        let pane = |agent: Option<&str>| -> Option<String> {
            let agent = agent?;
            ledger
                .as_ref()?
                .agents
                .iter()
                .find(|record| record.id == agent && !record.ended)
                .map(|record| record.pane_on(self.node.as_str()))
        };
        crate::automations::AutomationsSnapshot::of(
            &self.automations.book,
            &zone,
            self.home_path.as_deref().map(|home| {
                hide_platform::path::to_wire_lossy(&hide_kit::layout::automations_home(home))
            }),
            |run| {
                let live = run.state.active() || run.closing;
                live.then(|| pane(run.agent_id.as_deref())).flatten()
            },
            |opened| pane(opened.agent_id.as_deref()),
            |id, number| {
                self.automations
                    .viewing
                    .contains_key(&(id.to_owned(), number))
            },
        )
    }

    /// The automations stopped for a person to fix, oldest first: each one
    /// its own Needs You line on this machine's scope (D-12, B23), counted
    /// with its agents as a mission's own ask is; and beside them the once
    /// automations whose run ended and waits for the operator to open it,
    /// the one that ended first first (mission-104 O-4).
    pub(super) fn publish_automation_asks(&mut self) -> bool {
        let mut asks: Vec<(u64, String)> = self
            .automations
            .book
            .automations
            .iter()
            .filter_map(|automation| {
                let pause = automation.paused.as_ref()?;
                pause
                    .reason
                    .asks()
                    .then(|| (pause.at_unix_ms, automation.id.clone()))
            })
            .collect();
        asks.sort();
        let mut looks: Vec<(u64, String)> = self
            .automations
            .book
            .automations
            .iter()
            .filter_map(|automation| {
                let number = automation.look_run?;
                let run = automation.runs().find(|run| run.number == number)?;
                Some((
                    run.ended_at_unix_ms.unwrap_or(run.started_at_unix_ms),
                    automation.id.clone(),
                ))
            })
            .collect();
        looks.sort();
        let here = self.node.as_str();
        let by_device = |lines: Vec<(u64, String)>| {
            let mut by_device = std::collections::BTreeMap::new();
            if !lines.is_empty() {
                by_device.insert(
                    here.to_owned(),
                    lines.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
                );
            }
            by_device
        };
        let (asks, looks) = (by_device(asks), by_device(looks));
        self.agent_scope_cache.note_automation_asks(asks, looks) && self.refresh_agent_scopes()
    }
}

/// The run's own worktree: the branch its start made, or, for an automation
/// that makes one per run, the branch a start that stopped partway may have
/// made already.
fn run_worktree(automation: &Automation, run: &crate::automations::Run) -> Option<RunWorktree> {
    let branch = run.branch.clone().or_else(|| {
        automation
            .folder
            .worktree
            .then(|| automation.run_branch(run.number))
    })?;
    Some(RunWorktree {
        repo: automation.folder.path.clone(),
        branch,
    })
}

/// A cleanup's work off the lock: each start's pane closed while it still
/// runs the terminal its start recorded, then the worktree released when
/// nothing of the run is left in it. A worktree another agent still runs in
/// is tried again later.
fn close_and_release(
    engine: Option<crate::engine::SharedEngine>,
    node: Option<Arc<dyn crate::node_access::NodeLink>>,
    spawns: &[crate::coordination::SpawnRecord],
    worktree: Option<&RunWorktree>,
) -> Result<(), String> {
    let engine = engine.ok_or("engine_unavailable")?;
    for spawn in spawns {
        let closed = crate::coordination::executor::close_spawn_pane(engine.as_ref(), spawn);
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.terminal_closed",
            "intent": spawn.intent, "pane_id": spawn.pane,
            "outcome": closed.as_ref().ok(), "reason": closed.as_ref().err(),
        }));
        closed?;
    }
    let Some(worktree) = worktree else {
        return Ok(());
    };
    let node = node.ok_or("node_unavailable")?;
    let released = super::memory_promotion::release_branch_worktree(
        engine.as_ref(),
        node.as_ref(),
        &worktree.repo,
        &worktree.branch,
    );
    crate::diagnostic!(json!({
        "component": "automations", "kind": "automation.worktree_released",
        "branch": worktree.branch,
        "outcome": released.as_ref().ok(), "reason": released.as_ref().err(),
    }));
    match released? {
        "active" => Err("worktree_active".to_owned()),
        _ => Ok(()),
    }
}

/// What a start under an intent is for: the run's attempt, or an opening
/// of its finished conversation, by its number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Purpose {
    Attempt(u32),
    Open(u32),
}

impl Purpose {
    fn name(self) -> &'static str {
        match self {
            Self::Attempt(_) => "attempt",
            Self::Open(_) => "open",
        }
    }

    fn number(self) -> u32 {
        match self {
            Self::Attempt(number) | Self::Open(number) => number,
        }
    }
}

/// How an attempt's refused start fails: an agent not there to start, or
/// one its machine's kit has off or not set up, and a folder that is gone
/// are a person's to fix (B23, D-17); anything else is tried again.
fn start_failure(code: String) -> Failure {
    match code.as_str() {
        AGENT_NOT_INSTALLED
        | "agent_hook_off"
        | "agent_not_set_up"
        | "agent_hook_missing"
        | "agent_hook_untrusted" => Failure::Person(PauseReason::AgentMissing),
        crate::coordination::FOLDER_GONE => Failure::Person(PauseReason::FolderMissing),
        _ => Failure::Transient(code),
    }
}

/// What a run leaves to close: the terminal of each of its attempts and of
/// its conversation opened now, by their intents, and its own worktree.
fn run_leftovers(
    automation: &Automation,
    run: &crate::automations::Run,
) -> (Vec<String>, Option<RunWorktree>) {
    let intents = (1..=run.attempt)
        .map(|attempt| attempt_intent(&automation.id, run.number, attempt))
        .chain(
            run.opened
                .as_ref()
                .map(|opened| open_intent(&automation.id, run.number, opened.number)),
        )
        .collect();
    (intents, run_worktree(automation, run))
}

/// What the runs left to close on another machine, as `left` keeps them:
/// automation, run, machine and what to close there.
type LeftElsewhere = Vec<(String, u64, String, (Vec<String>, Option<RunWorktree>))>;

/// An automation the book brought from the core of another machine (a core
/// move, Decide 1): it runs here only once a person resumes it, with its
/// own Needs You line, unless the operator had paused it themselves, which
/// stays as it was. A run that went there does not go on here: it ends
/// stopped as the machine changed, and what it and any finished run left
/// open is kept to close on that machine, never judged by this one's
/// engine. Whether the book changed.
fn moved_here(automation: &mut Automation, here: &str, now: u64, left: &mut LeftElsewhere) -> bool {
    let made_on = automation.machine.clone();
    let mut changed = false;
    let id = automation.id.clone();
    let mut leftovers = Vec::new();
    let mut ended = None;
    for entry in &mut automation.records {
        let Entry::Run(run) = entry else {
            continue;
        };
        if !(run.state.active() || run.closing || run.opened.is_some()) {
            continue;
        }
        if run.state.active() {
            run.state = RunState::Stopped;
            run.reason = Some(MACHINE_CHANGED.to_owned());
            run.ended_at_unix_ms = Some(now);
            run.ask = None;
            ended = Some(run.number);
        }
        leftovers.push(run.number);
        changed = true;
    }
    // A once automation's run ended: the operator learns it in Needs You
    // (mission-104 O-4).
    if automation.schedule.is_once() && ended.is_some() {
        automation.look_run = ended;
    }
    for number in leftovers {
        let run = automation
            .runs()
            .find(|run| run.number == number)
            .expect("listed above");
        left.push((
            id.clone(),
            number,
            made_on.clone(),
            run_leftovers(automation, run),
        ));
        let run = automation.run_mut(number).expect("listed above");
        run.closing = false;
        run.opened = None;
    }
    let operator = automation.paused.map(|pause| pause.reason) == Some(PauseReason::Operator);
    let flagged = automation.paused.map(|pause| pause.reason) == Some(PauseReason::MachineChanged);
    // A done once automation has no time to run on this machine.
    if !operator && !flagged && !automation.done() {
        automation.paused = Some(Pause {
            reason: PauseReason::MachineChanged,
            at_unix_ms: now,
        });
        automation.next_due_unix_ms = None;
        changed = true;
        crate::diagnostic!(json!({
            "component": "automations", "kind": "automation.machine_changed",
            "automation": id, "made_on": made_on, "here": here,
        }));
    }
    changed
}

/// A run's agent's row as this core sees it now.
enum RowLook {
    /// The ledger cannot be read now.
    Unknown,
    /// The registration ended or is gone.
    Gone,
    /// Registered, with no row in its pane under its session.
    Missing,
    Shown(Box<crate::model::SidebarAgentSnapshot>),
}

/// The provider's arguments that open `conversation` again (D-13, D-17).
fn resume_arguments(conversation: &Conversation) -> Option<Vec<String>> {
    if !hide_session::valid_native_id(&conversation.id) {
        return None;
    }
    let dialect = hide_agent_adapter::adapter(&conversation.kind)?.resume?;
    Some(vec![dialect.resume_flag().into(), conversation.id.clone()])
}

/// The row's conversation, by the provider's own session id.
fn row_conversation(row: &crate::model::SidebarAgentSnapshot) -> Option<Conversation> {
    Some(Conversation {
        kind: row.agent_kind.clone(),
        id: row.session_id.clone()?,
    })
}
