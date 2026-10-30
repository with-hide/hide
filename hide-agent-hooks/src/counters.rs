//! The per-pane subagent count the hook keeps between invocations.
//!
//! A hook script is stateless: it is started fresh for every event and knows
//! only its environment. The count therefore lives here, keyed by
//! `$HERDR_PANE_ID`, one record per session the pane's hooks heard from, and
//! is reported to the core after the events that change it
//! ([`crate::report`]; PRD D-53, PRD engine-bundle-1 D-7).
//!
//! A pane runs more than one session's hooks when the operator's own hook
//! starts `claude -p` in the pane's environment; that session gets a record
//! of its own, so it never resets the pane's. Which record is the pane's
//! agent is the core's to say, from the session Herdr reports for the pane.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::report::HookTurn;
use crate::runtime::{AgentRuntime, HOOK_VERSION, HookEvent};

/// The most subagents one session may have running at once, and the most
/// finished ones it remembers by id. A record is read and rewritten whole by
/// every event, so it is bounded; crossing the running cap is a failed hook
/// event the pane's last report failure shows, not a larger record.
const RUNNING_LIMIT: usize = 256;
const FINISHED_LIMIT: usize = 256;
/// The sessions one pane's file keeps; the one heard from least recently
/// goes first. The core keeps the same number (`agent_hooks::facts`).
pub const SESSIONS_PER_PANE: usize = 8;

/// What one pane's session has spawned.
///
/// `blocked` is deliberately absent rather than zero: neither shipped runtime
/// reports a blocked subagent, and drawing a zero would claim knowledge the
/// adapter does not have (PRD B32, D-53).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneCounters {
    #[serde(default)]
    pub working: u32,
    #[serde(default)]
    pub done: u32,
}

/// One session's record in its pane's file: the counts, who reported them,
/// and the turn its hook last reported.
///
/// `version` is the hook version of the helper that wrote it, `agent` the
/// adapter id of the runtime that reported, and `session` the agent's own id
/// of the session, when the hook's input named it.
///
/// `running` and `finished` are the subagents an agent names by id: a start
/// puts an id in `running`, its stop moves it to `finished` once, and a
/// repeated stop or the stop of an id that never started changes nothing.
/// `working` counts the subagents of an agent whose events carry no id.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    working: u32,
    #[serde(default)]
    done: u32,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    running: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    finished: Vec<String>,
    /// The turn the last report said, so a tool event reports only a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<HookTurn>,
}

impl Record {
    fn counters(&self) -> PaneCounters {
        PaneCounters {
            working: self
                .working
                .saturating_add(u32::try_from(self.running.len()).unwrap_or(u32::MAX)),
            done: self.done,
        }
    }

    /// Whether this record is `who`'s: the same runtime, and the same
    /// session when both name one. An event that names none is the runtime's
    /// last session, as a record that names none is taken by the first that
    /// does.
    fn is(&self, who: Reporter<'_>) -> bool {
        self.agent.as_deref() == who.agent
            && match (who.session, self.session.as_deref()) {
                (Some(event), Some(kept)) => event == kept,
                _ => true,
            }
    }
}

/// A pane's file: its sessions' records, the last heard from last.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct PaneFile {
    sessions: Vec<Record>,
}

impl PaneFile {
    /// A file as written, or one record of a helper that kept one per pane.
    fn decode(raw: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice::<Self>(raw).or_else(|_| {
            serde_json::from_slice::<Record>(raw).map(|record| Self {
                sessions: vec![record],
            })
        })
    }
}

/// Who is reporting a pane's counts: the runtime's adapter id, and the
/// session the hook's input named, when it did.
///
/// A session the hook could not read stays unknown rather than carried over
/// from an earlier event of the pane, except by an event that changes no
/// count ([`Change::None`]), which is not evidence of a different session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Reporter<'a> {
    pub agent: Option<&'a str>,
    pub session: Option<&'a str>,
}

/// Where the counts live. Under Hide's own directory, never the runtime's.
pub fn state_directory(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks").join("panes")
}

/// A pane id is Herdr's (`w7B:pM`), so it is folded into a file name rather
/// than used as one.
fn record_path(home: &Path, pane_id: &str) -> PathBuf {
    let safe: String = pane_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    state_directory(home).join(format!("{safe}.json"))
}

/// How long one event waits for another event of the same pane to finish its
/// update. Parallel subagents start and stop together, and an update read
/// before another one's write would lose that one.
const LOCK_WAIT: Duration = Duration::from_millis(1_000);

/// One change an event makes to a pane's counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change<'a> {
    /// A session started. The record already is that session's when the event
    /// names it (a resume or a compaction of the same session keeps its
    /// subagents); an event that names none starts the record over.
    SessionStarted,
    /// One subagent started: by id when the agent names it, and a subagent that
    /// stopped and is running again is not counted done any more.
    Started(Option<&'a str>),
    /// One subagent stopped: a known id moves from working to done once.
    Stopped(Option<&'a str>),
    /// The turn ended with this many subagents still running, for an agent
    /// that names none by id: none for an agent whose subagents end with its
    /// turn, and the background ones Grok lists for a turn that leaves them
    /// running.
    Settled { running: u32 },
    /// The turn ended and the agent listed the subagents that still run. A
    /// subagent it no longer lists ended without a stop reaching the hook, and
    /// is no longer working; it is not counted done, since nothing said it
    /// finished.
    Listed { running: &'a [String] },
    /// The event changes no count; the pane's counts are only republished.
    None,
}

impl<'a> Change<'a> {
    /// The change one event of a six-event runtime makes, from what its input
    /// said.
    ///
    /// Claude Code names each subagent and lists the ones still running when
    /// a turn ends, so its `Stop` never sweeps: a background subagent outlives
    /// its parent's turn (measured: the turn's `Stop` fires before the
    /// subagent's `SubagentStop`). A `Stop` without that list changes nothing
    /// rather than guessing. A runtime that lists nothing keeps the older
    /// rule, that the turn is over and so are the subagents it started.
    pub fn of(runtime: Option<AgentRuntime>, event: HookEvent, input: &'a SubagentInput) -> Self {
        match event {
            HookEvent::SessionStart => Self::SessionStarted,
            HookEvent::UserPromptSubmit
            | HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure
            | HookEvent::PermissionRequest
            | HookEvent::Notification
            | HookEvent::Interrupt => Self::None,
            HookEvent::SubagentStart => Self::Started(input.subagent.as_deref()),
            HookEvent::SubagentStop => Self::Stopped(input.subagent.as_deref()),
            // A subagent's own turn end is not the session's.
            HookEvent::Stop if input.subagent.is_some() => Self::None,
            HookEvent::Stop if runtime == Some(AgentRuntime::ClaudeCode) => match &input.running {
                Some(running) => Self::Listed { running },
                None => Self::None,
            },
            HookEvent::Stop => Self::Settled { running: 0 },
        }
    }
}

/// What a six-event runtime's hook input says about the subagent count: the
/// session it came from, the subagent it is about, and the subagents that
/// were running when a turn ended.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubagentInput {
    pub session: Option<String>,
    pub subagent: Option<String>,
    /// The ids Claude Code's `background_tasks` lists as running subagents,
    /// when the input has that list.
    pub running: Option<Vec<String>>,
}

impl SubagentInput {
    /// Reads the input whole. One that was cut off or cannot be read names
    /// nothing, which the counts treat as an event of an unknown session.
    pub fn read(payload: &[u8], truncated: bool) -> Self {
        #[derive(Deserialize)]
        struct Task {
            id: Option<String>,
            #[serde(rename = "type")]
            kind: Option<String>,
            status: Option<String>,
        }
        #[derive(Deserialize)]
        struct Input {
            session_id: Option<String>,
            agent_id: Option<String>,
            background_tasks: Option<Vec<Task>>,
        }
        let Some(input) = (!truncated)
            .then(|| serde_json::from_slice::<Input>(payload).ok())
            .flatten()
        else {
            return Self::default();
        };
        Self {
            session: input
                .session_id
                .filter(|id| crate::delivery::valid_session(id)),
            subagent: input.agent_id.filter(|id| valid_subagent(id)),
            running: input.background_tasks.map(|tasks| {
                tasks
                    .into_iter()
                    .filter(|task| {
                        task.kind.as_deref() == Some("subagent")
                            && task.status.as_deref() == Some("running")
                    })
                    .filter_map(|task| task.id)
                    .filter(|id| valid_subagent(id))
                    .collect()
            }),
        }
    }
}

fn valid_subagent(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}

/// What a change did to its session's record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Applied {
    /// The session's counts after the change.
    pub counters: PaneCounters,
    /// The turn the session's last report said, before this change.
    pub previous_turn: Option<HookTurn>,
}

/// One session's record as a reader outside the hook sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub agent: String,
    pub version: u32,
    pub session: Option<String>,
    pub counters: PaneCounters,
}

/// Every pane's records, oldest first, by the Herdr pane id its file is named
/// for (`w7B_pM.json` for `w7B:pM`), at most `limit` panes. A core that has no
/// facts of its own yet reads them once, so a pane whose agent last reported
/// to Herdr keeps its connection after the hooks start reporting to the core.
/// A file whose name no pane id makes, or that holds no record of an agent
/// and its version, is passed over.
pub fn every_pane(home: &Path, limit: usize) -> Vec<(String, Vec<SessionRecord>)> {
    let Ok(entries) = fs::read_dir(state_directory(home)) else {
        return Vec::new();
    };
    let mut panes = Vec::new();
    for path in entries.flatten().map(|entry| entry.path()) {
        if panes.len() >= limit {
            break;
        }
        let Some(pane_id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|stem| stem.split_once('_'))
            .filter(|(tab, pane)| {
                tab.starts_with('w') && pane.starts_with('p') && !pane.contains('_')
            })
            .map(|(tab, pane)| format!("{tab}:{pane}"))
        else {
            continue;
        };
        let Ok(file) = read_file(home, &pane_id) else {
            continue;
        };
        let records: Vec<SessionRecord> = file
            .sessions
            .iter()
            .filter_map(|record| {
                Some(SessionRecord {
                    agent: record.agent.clone()?,
                    version: record.version?,
                    session: record.session.clone(),
                    counters: record.counters(),
                })
            })
            .collect();
        if !records.is_empty() {
            panes.push((pane_id, records));
        }
    }
    panes
}

/// The session a tool event moves back to working, when it does: the
/// runtime's last session in the pane, whose last report said its turn ended,
/// waited or rested. A session with no turn yet is left to its prompt's
/// report, so an ordinary tool call reads one small file and starts no
/// process. The inner `None` is a session that never named itself.
pub fn resumes(home: &Path, pane_id: &str, who: Reporter<'_>) -> Option<Option<String>> {
    let file = read_file(home, pane_id).ok()?;
    let record = file.sessions.iter().rev().find(|record| record.is(who))?;
    record
        .turn
        .is_some_and(|turn| turn != HookTurn::Working)
        .then(|| record.session.clone())
}

fn read_file(home: &Path, pane_id: &str) -> io::Result<PaneFile> {
    let _held = hold(home, hide_platform::fs::lock::Mode::Shared)?;
    let raw = fs::read(record_path(home, pane_id))?;
    PaneFile::decode(&raw).map_err(io::Error::from)
}

/// The counts of the session the pane's file heard from last.
pub fn read(home: &Path, pane_id: &str) -> PaneCounters {
    read_file(home, pane_id)
        .ok()
        .and_then(|file| file.sessions.last().map(Record::counters))
        .unwrap_or_default()
}

/// Applies one [`Change`] to `who`'s record in the pane's file and returns
/// its new counts.
pub fn change(
    home: &Path,
    pane_id: &str,
    change: Change<'_>,
    who: Reporter<'_>,
) -> io::Result<PaneCounters> {
    apply(home, pane_id, change, who, None).map(|applied| applied.counters)
}

/// Applies one [`Change`] to `who`'s record, and notes `turn` as the one its
/// next report says.
///
/// The record is read, changed and written while the lock is held, so two
/// events of one pane never both start from the same count. The write is not
/// synced: a count describes running sessions and is rebuilt by their next
/// events, so it does not have to survive a crash, and a sync inside the lock
/// would make parallel subagent starts wait out the lock.
pub fn apply(
    home: &Path,
    pane_id: &str,
    change: Change<'_>,
    who: Reporter<'_>,
    turn: Option<HookTurn>,
) -> io::Result<Applied> {
    let path = record_path(home, pane_id);
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)?;
    }
    let held = hold(home, hide_platform::fs::lock::Mode::Exclusive)?;
    let mut file = read_lenient(&path);
    let before = file.clone();
    let mut record = match file.sessions.iter().rposition(|record| record.is(who)) {
        Some(at) => file.sessions.remove(at),
        None => Record::default(),
    };
    let previous_turn = record.turn;
    match change {
        // A session that names itself has its own record already, or is the
        // record's own (a resume or a compaction keeps its subagents); one
        // that names none cannot say which, and starts over.
        Change::SessionStarted => {
            if who.session.is_none() {
                record = Record::default();
            }
            record.turn = None;
        }
        Change::Started(Some(subagent)) => {
            if let Some(at) = record.finished.iter().position(|id| id == subagent) {
                record.finished.remove(at);
                record.done = record.done.saturating_sub(1);
            }
            if !record.running.iter().any(|id| id == subagent) {
                if record.running.len() >= RUNNING_LIMIT {
                    return Err(too_many());
                }
                record.running.push(subagent.to_owned());
            }
        }
        Change::Started(None) => record.working = record.working.saturating_add(1),
        Change::Stopped(Some(subagent)) => {
            if let Some(at) = record.running.iter().position(|id| id == subagent) {
                record.running.remove(at);
                remember_finished(&mut record, subagent);
                record.done = record.done.saturating_add(1);
            }
        }
        Change::Stopped(None) => {
            record.working = record.working.saturating_sub(1);
            record.done = record.done.saturating_add(1);
        }
        Change::Settled { running } => record.working = running,
        Change::Listed { running } => {
            let mut listed: Vec<String> = Vec::with_capacity(running.len());
            for id in running {
                if !listed.contains(id) {
                    listed.push(id.clone());
                }
            }
            if listed.len() > RUNNING_LIMIT {
                return Err(too_many());
            }
            for id in &listed {
                if let Some(at) = record.finished.iter().position(|finished| finished == id) {
                    record.finished.remove(at);
                    record.done = record.done.saturating_sub(1);
                }
            }
            record.running = listed;
        }
        Change::None => {}
    }
    if turn.is_some() {
        record.turn = turn;
    }
    record.version = Some(HOOK_VERSION);
    record.agent = who.agent.map(str::to_owned);
    if who.session.is_some() {
        record.session = who.session.map(str::to_owned);
    }
    let counters = record.counters();
    file.sessions.push(record);
    if file.sessions.len() > SESSIONS_PER_PANE {
        file.sessions.remove(0);
    }
    // A tool event that moved nothing leaves the file as it was.
    if file != before {
        write_file(&path, &file)?;
    }
    drop(held);
    Ok(Applied {
        counters,
        previous_turn,
    })
}

fn too_many() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("a session has more than {RUNNING_LIMIT} subagents running"),
    )
}

/// Remembers that `subagent` finished, so that one that starts again is not
/// counted done twice. The oldest ids go first.
fn remember_finished(record: &mut Record, subagent: &str) {
    if record.finished.len() >= FINISHED_LIMIT {
        record.finished.remove(0);
    }
    record.finished.push(subagent.to_owned());
}

/// Replaces `who`'s counts with ones its agent keeps itself: OpenCode's
/// plugin follows its child sessions and sends the totals, not events. It
/// holds the same lock as [`change`], so a total and an event never
/// interleave.
pub fn store(
    home: &Path,
    pane_id: &str,
    counters: PaneCounters,
    who: Reporter<'_>,
) -> io::Result<()> {
    let path = record_path(home, pane_id);
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)?;
    }
    let held = hold(home, hide_platform::fs::lock::Mode::Exclusive)?;
    let mut file = read_lenient(&path);
    let turn = file
        .sessions
        .iter()
        .rposition(|record| record.is(who))
        .and_then(|at| file.sessions.remove(at).turn);
    file.sessions.push(Record {
        working: counters.working,
        done: counters.done,
        version: Some(HOOK_VERSION),
        agent: who.agent.map(str::to_owned),
        session: who.session.map(str::to_owned),
        turn,
        ..Record::default()
    });
    if file.sessions.len() > SESSIONS_PER_PANE {
        file.sessions.remove(0);
    }
    write_file(&path, &file)?;
    drop(held);
    Ok(())
}

/// The file as it is, or an empty one when it is missing or cannot be read:
/// the next write replaces it whole, under the lock the caller holds.
fn read_lenient(path: &Path) -> PaneFile {
    fs::read(path)
        .ok()
        .and_then(|raw| PaneFile::decode(&raw).ok())
        .unwrap_or_default()
}

/// Writes a pane's file in place, private to the account, under a held lock.
fn write_file(path: &Path, file: &PaneFile) -> io::Result<()> {
    let mut handle = hide_platform::fs::private::open_or_create_file(path)?;
    handle.set_len(0)?;
    handle.write_all(&serde_json::to_vec(file)?)
}

/// One lock file beside the records, not the record itself: Windows locks a
/// byte range against every other handle, so a record locked by one handle
/// could not be rewritten through another. It is the account's own, so no
/// other account can hold it and stall every count.
fn hold(
    home: &Path,
    mode: hide_platform::fs::lock::Mode,
) -> io::Result<hide_platform::fs::lock::Lock> {
    let lock = hide_platform::fs::private::open_or_create_file(&lock_path(home))?;
    match hide_platform::fs::lock::lock_file(lock, mode, LOCK_WAIT, &|| false)? {
        hide_platform::fs::lock::Waited::Locked(held) => Ok(held),
        _ => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "another hook event held the pane counts",
        )),
    }
}

/// The lock every count update holds. It sits beside the records' folder,
/// so the sweep in [`retain`] never takes it for a record.
fn lock_path(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks").join("panes.lock")
}

/// Drops a pane's record. Used by the diagnosis when a pane is gone.
pub fn forget(home: &Path, pane_id: &str) -> io::Result<()> {
    match fs::remove_file(record_path(home, pane_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Drops every record whose pane no longer exists, and reports how many went.
///
/// A pane that Herdr has stopped listing cannot be running a session, so its
/// counts belong to one that is over and nothing can ask for them again (PRD
/// B31, D-53). Panes are the key rather than agents because the record is
/// written by `SessionStart`, which can run before the agent is listed;
/// sweeping by agent would race a session into losing its own count.
///
/// A record the sweep cannot read its name back from is left alone: this
/// directory is Hide's, but deleting a file on a guess is not a cleanup.
pub fn retain<'a>(
    home: &Path,
    live_pane_ids: impl IntoIterator<Item = &'a str>,
) -> io::Result<usize> {
    let keep: HashSet<PathBuf> = live_pane_ids
        .into_iter()
        .map(|pane_id| record_path(home, pane_id))
        .collect();
    let directory = state_directory(home);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut dropped = 0;
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "json") || keep.contains(&path) {
            continue;
        }
        fs::remove_file(&path)?;
        dropped += 1;
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHO: Reporter<'static> = Reporter {
        agent: Some("claude-code"),
        session: Some("session-a"),
    };

    fn home(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hide-agent-hooks-counters-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_started_subagent_counts_until_it_stops_and_then_counts_as_done() {
        let root = home("lifecycle");
        let pane = "w7B:pM";
        assert_eq!(
            change(&root, pane, Change::SessionStarted, WHO).unwrap(),
            PaneCounters::default()
        );
        change(&root, pane, Change::Started(None), WHO).unwrap();
        let two = change(&root, pane, Change::Started(None), WHO).unwrap();
        assert_eq!(
            two,
            PaneCounters {
                working: 2,
                done: 0
            }
        );
        let one = change(&root, pane, Change::Stopped(None), WHO).unwrap();
        assert_eq!(
            one,
            PaneCounters {
                working: 1,
                done: 1
            }
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_turn_that_ends_sweeps_a_count_a_missing_stop_left_behind() {
        let root = home("sweep");
        let pane = "w7B:pM";
        change(&root, pane, Change::SessionStarted, WHO).unwrap();
        change(&root, pane, Change::Started(None), WHO).unwrap();
        change(&root, pane, Change::Started(None), WHO).unwrap();
        let swept = change(&root, pane, Change::Settled { running: 0 }, WHO).unwrap();
        assert_eq!(swept.working, 0, "no subagent outlives its turn");
        assert_eq!(swept.done, 0, "the sweep does not invent completions");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_session_in_the_same_pane_does_not_inherit_the_last_ones_numbers() {
        let root = home("reset");
        let pane = "w7B:pM";
        change(&root, pane, Change::Started(None), WHO).unwrap();
        change(&root, pane, Change::Stopped(None), WHO).unwrap();
        assert_eq!(
            read(&root, pane),
            PaneCounters {
                working: 0,
                done: 1
            }
        );
        let next = Reporter {
            session: Some("session-b"),
            ..WHO
        };
        assert_eq!(
            change(&root, pane, Change::SessionStarted, next).unwrap(),
            PaneCounters::default()
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_sweep_drops_the_records_of_panes_that_are_gone_and_keeps_the_rest() {
        let root = home("retain");
        change(&root, "w1:pA", Change::Started(None), WHO).unwrap();
        change(&root, "w1:pB", Change::Started(None), WHO).unwrap();
        change(&root, "w2:pC", Change::Started(None), WHO).unwrap();
        // A file the sweep did not write is not its business.
        fs::write(state_directory(&root).join("notes.txt"), b"kept").unwrap();

        assert_eq!(retain(&root, ["w1:pA", "w2:pC"]).unwrap(), 1);
        assert_eq!(read(&root, "w1:pA").working, 1);
        assert_eq!(read(&root, "w2:pC").working, 1);
        assert_eq!(
            read(&root, "w1:pB"),
            PaneCounters::default(),
            "a dead session's count is gone rather than waiting to be redrawn"
        );
        assert!(state_directory(&root).join("notes.txt").exists());

        // Idempotent: the same live set sweeps nothing the second time.
        assert_eq!(retain(&root, ["w1:pA", "w2:pC"]).unwrap(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_sweep_before_any_hook_has_run_is_not_an_error() {
        let root = home("retain-empty");
        assert_eq!(retain(&root, ["w1:pA"]).unwrap(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parallel_starts_of_one_pane_all_count() {
        let root = home("parallel");
        let pane = "w7B:pM";
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || change(&root, pane, Change::Started(None), WHO).unwrap())
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(read(&root, pane).working, 8);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_turn_that_leaves_background_subagents_running_keeps_them_counted() {
        let root = home("settled");
        let pane = "w7B:pM";
        for _ in 0..3 {
            change(&root, pane, Change::Started(None), WHO).unwrap();
        }
        let settled = change(&root, pane, Change::Settled { running: 2 }, WHO).unwrap();
        assert_eq!(settled.working, 2);
        let one = change(&root, pane, Change::Stopped(None), WHO).unwrap();
        assert_eq!((one.working, one.done), (1, 1));
        fs::remove_dir_all(&root).unwrap();
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    fn counts(root: &Path, pane: &str) -> (u32, u32) {
        let counters = read(root, pane);
        (counters.working, counters.done)
    }

    // The order below is what an interactive Claude Code 2.1.300 sent for two
    // background subagents that each paused on a background command twice
    // (agents/runs/fix-claude-subagent-counts/dbg2/events.log): a `Stop` that
    // lists both before either stops, a `SubagentStop` at each pause, and a
    // `SubagentStart` with the same agent_id when the command ends.
    #[test]
    fn a_subagent_that_pauses_and_resumes_is_one_subagent() {
        let root = home("pause");
        let pane = "w7B:pM";
        let apply = |change| {
            super::change(&root, pane, change, WHO).unwrap();
        };
        let both = ids(&["a", "b"]);
        apply(Change::SessionStarted);
        apply(Change::Started(Some("a")));
        apply(Change::Started(Some("b")));
        apply(Change::Listed { running: &both });
        assert_eq!(
            counts(&root, pane),
            (2, 0),
            "the parent's turn end is no sweep"
        );
        apply(Change::Stopped(Some("a")));
        apply(Change::Stopped(Some("b")));
        assert_eq!(counts(&root, pane), (0, 2));
        apply(Change::Listed { running: &[] });
        assert_eq!(counts(&root, pane), (0, 2));
        // Resumed by its command's end: running again, and not done twice.
        apply(Change::Started(Some("a")));
        apply(Change::Started(Some("b")));
        assert_eq!(counts(&root, pane), (2, 0));
        apply(Change::Stopped(Some("a")));
        apply(Change::Stopped(Some("b")));
        assert_eq!(counts(&root, pane), (0, 2));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_repeated_stop_or_the_stop_of_an_unknown_subagent_changes_nothing() {
        let root = home("repeat-stop");
        let pane = "w7B:pM";
        change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        change(&root, pane, Change::Stopped(Some("a")), WHO).unwrap();
        for stopped in ["a", "never-started"] {
            change(&root, pane, Change::Stopped(Some(stopped)), WHO).unwrap();
        }
        assert_eq!(counts(&root, pane), (0, 1));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_start_repeated_for_a_running_subagent_counts_it_once() {
        let root = home("repeat-start");
        let pane = "w7B:pM";
        for _ in 0..3 {
            change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        }
        assert_eq!(counts(&root, pane), (1, 0));
        fs::remove_dir_all(&root).unwrap();
    }

    // A subagent stopped with the agent's TaskStop tool sends no SubagentStop
    // (measured), so the next turn end's list is what ends it, uncounted as
    // done because nothing said it finished.
    #[test]
    fn a_subagent_killed_without_a_stop_leaves_when_the_next_turn_end_no_longer_lists_it() {
        let root = home("killed");
        let pane = "w7B:pM";
        change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        change(
            &root,
            pane,
            Change::Listed {
                running: &ids(&["a"]),
            },
            WHO,
        )
        .unwrap();
        assert_eq!(counts(&root, pane), (1, 0));
        change(&root, pane, Change::Listed { running: &[] }, WHO).unwrap();
        assert_eq!(counts(&root, pane), (0, 0));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_list_names_a_subagent_whose_start_never_arrived() {
        let root = home("listed-new");
        let pane = "w7B:pM";
        change(
            &root,
            pane,
            Change::Listed {
                running: &ids(&["a", "a", "b"]),
            },
            WHO,
        )
        .unwrap();
        assert_eq!(counts(&root, pane), (2, 0));
        fs::remove_dir_all(&root).unwrap();
    }

    // The measured trace of the lead pane: a nested `claude -p` started by the
    // operator's Stop hook runs SessionStart, UserPromptSubmit and Stop with
    // the pane's environment, in a session of its own.
    #[test]
    fn a_nested_session_keeps_a_record_of_its_own_and_leaves_the_panes_alone() {
        let root = home("nested");
        let pane = "w7B:pM";
        let nested = Reporter {
            session: Some("nested"),
            ..WHO
        };
        change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        change(&root, pane, Change::Stopped(Some("a")), WHO).unwrap();
        change(&root, pane, Change::Started(Some("b")), WHO).unwrap();
        for event in [
            Change::SessionStarted,
            Change::None,
            Change::Listed { running: &[] },
            Change::Settled { running: 0 },
        ] {
            change(&root, pane, event, nested).unwrap();
        }
        let own = change(&root, pane, Change::None, WHO).unwrap();
        assert_eq!((own.working, own.done), (1, 1));
        let file = PaneFile::decode(&fs::read(record_path(&root, pane)).unwrap()).unwrap();
        assert_eq!(file.sessions.len(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_pane_keeps_its_latest_sessions_and_drops_the_oldest() {
        let root = home("sessions-cap");
        let pane = "w7B:pM";
        for index in 0..=SESSIONS_PER_PANE {
            let session = format!("s-{index}");
            let who = Reporter {
                session: Some(&session),
                ..WHO
            };
            change(&root, pane, Change::Started(None), who).unwrap();
        }
        let file = PaneFile::decode(&fs::read(record_path(&root, pane)).unwrap()).unwrap();
        assert_eq!(file.sessions.len(), SESSIONS_PER_PANE);
        assert_eq!(file.sessions[0].session.as_deref(), Some("s-1"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_an_older_helper_wrote_is_read_as_its_one_session() {
        let root = home("older");
        let pane = "w7B:pM";
        fs::create_dir_all(state_directory(&root)).unwrap();
        fs::write(
            record_path(&root, pane),
            br#"{"working":2,"done":4,"version":8,"agent":"claude-code","session":"session-a"}"#,
        )
        .unwrap();
        let counted = change(&root, pane, Change::Started(None), WHO).unwrap();
        assert_eq!((counted.working, counted.done), (3, 4));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_session_notes_the_turn_its_report_said_and_a_session_start_forgets_it() {
        let root = home("turn");
        let pane = "w7B:pM";
        let first = apply(&root, pane, Change::None, WHO, Some(HookTurn::Working)).unwrap();
        assert_eq!(first.previous_turn, None);
        let again = apply(&root, pane, Change::None, WHO, Some(HookTurn::Done)).unwrap();
        assert_eq!(again.previous_turn, Some(HookTurn::Working));
        apply(&root, pane, Change::SessionStarted, WHO, None).unwrap();
        let after = apply(&root, pane, Change::None, WHO, None).unwrap();
        assert_eq!(after.previous_turn, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_compaction_or_resume_of_the_same_session_keeps_its_subagents() {
        let root = home("same-session-start");
        let pane = "w7B:pM";
        change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        change(&root, pane, Change::SessionStarted, WHO).unwrap();
        assert_eq!(counts(&root, pane), (1, 0));
        // A runtime that names no session cannot say it is the same one.
        let blind = Reporter {
            session: None,
            ..WHO
        };
        change(&root, pane, Change::SessionStarted, blind).unwrap();
        assert_eq!(counts(&root, pane), (0, 0));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_running_set_has_a_cap_and_crossing_it_is_an_error() {
        let root = home("cap");
        let pane = "w7B:pM";
        for index in 0..RUNNING_LIMIT {
            change(
                &root,
                pane,
                Change::Started(Some(&format!("a{index}"))),
                WHO,
            )
            .unwrap();
        }
        let error = change(&root, pane, Change::Started(Some("one-more")), WHO).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(counts(&root, pane).0, RUNNING_LIMIT as u32);
        let too_long: Vec<String> = (0..=RUNNING_LIMIT)
            .map(|index| format!("b{index}"))
            .collect();
        assert!(change(&root, pane, Change::Listed { running: &too_long }, WHO).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_finished_ids_a_record_remembers_are_bounded_and_the_count_is_not() {
        let root = home("finished-cap");
        let pane = "w7B:pM";
        for index in 0..FINISHED_LIMIT + 5 {
            let id = format!("a{index}");
            change(&root, pane, Change::Started(Some(&id)), WHO).unwrap();
            change(&root, pane, Change::Stopped(Some(&id)), WHO).unwrap();
        }
        assert_eq!(counts(&root, pane), (0, (FINISHED_LIMIT + 5) as u32));
        let file = PaneFile::decode(&fs::read(record_path(&root, pane)).unwrap()).unwrap();
        assert_eq!(file.sessions[0].finished.len(), FINISHED_LIMIT);
        fs::remove_dir_all(&root).unwrap();
    }

    // Claude Code 2.1.300 payloads, trimmed to the fields the hook reads.
    const CLAUDE_STOP: &[u8] = br#"{"session_id":"s1","hook_event_name":"Stop","stop_hook_active":false,
        "background_tasks":[
          {"id":"a4d1fa","type":"subagent","status":"running","description":"A","agent_type":"general-purpose"},
          {"id":"bmfo3f","type":"shell","status":"running","command":"sleep 100"},
          {"id":"aa43d5","type":"subagent","status":"completed"}]}"#;
    const CLAUDE_SUBAGENT_STOP: &[u8] = br#"{"session_id":"s1","agent_id":"a4d1fa","agent_type":"general-purpose",
        "hook_event_name":"SubagentStop","background_tasks":[{"id":"a4d1fa","type":"subagent","status":"running"}]}"#;

    #[test]
    fn the_input_names_the_session_the_subagent_and_the_ones_still_running() {
        let stop = SubagentInput::read(CLAUDE_STOP, false);
        assert_eq!(stop.session.as_deref(), Some("s1"));
        assert_eq!(stop.subagent, None);
        assert_eq!(
            stop.running,
            Some(ids(&["a4d1fa"])),
            "only running subagents"
        );
        let sub = SubagentInput::read(CLAUDE_SUBAGENT_STOP, false);
        assert_eq!(sub.subagent.as_deref(), Some("a4d1fa"));
        // No list is not an empty one.
        let none = SubagentInput::read(br#"{"session_id":"s1"}"#, false);
        assert_eq!(none.running, None);
        let empty = SubagentInput::read(br#"{"session_id":"s1","background_tasks":[]}"#, false);
        assert_eq!(empty.running, Some(Vec::new()));
        // Cut off or unreadable names nothing.
        assert_eq!(
            SubagentInput::read(CLAUDE_STOP, true),
            SubagentInput::default()
        );
        assert_eq!(SubagentInput::read(b"{", false), SubagentInput::default());
        assert_eq!(
            SubagentInput::read(br#"{"session_id":"-rf","agent_id":""}"#, false),
            SubagentInput::default()
        );
    }

    #[test]
    fn each_event_makes_the_change_its_runtime_can_vouch_for() {
        let stop = SubagentInput::read(CLAUDE_STOP, false);
        let sub = SubagentInput::read(CLAUDE_SUBAGENT_STOP, false);
        let blank = SubagentInput::default();
        let claude = Some(AgentRuntime::ClaudeCode);
        let of = |runtime, event, input| Change::of(runtime, event, input);
        assert_eq!(
            of(claude, HookEvent::Stop, &stop),
            Change::Listed {
                running: &ids(&["a4d1fa"])
            }
        );
        // A turn end that lists nothing changes nothing for Claude Code, and
        // a subagent's own end is not the session's.
        assert_eq!(of(claude, HookEvent::Stop, &blank), Change::None);
        assert_eq!(of(claude, HookEvent::Stop, &sub), Change::None);
        assert_eq!(
            of(Some(AgentRuntime::Codex), HookEvent::Stop, &blank),
            Change::Settled { running: 0 }
        );
        assert_eq!(
            of(claude, HookEvent::SubagentStart, &sub),
            Change::Started(Some("a4d1fa"))
        );
        assert_eq!(
            of(claude, HookEvent::SubagentStop, &sub),
            Change::Stopped(Some("a4d1fa"))
        );
        assert_eq!(
            of(claude, HookEvent::SessionStart, &blank),
            Change::SessionStarted
        );
        assert_eq!(
            of(claude, HookEvent::UserPromptSubmit, &blank),
            Change::None
        );
    }

    #[test]
    fn two_panes_keep_separate_counts() {
        let root = home("panes");
        change(&root, "w1:pA", Change::Started(None), WHO).unwrap();
        change(&root, "w2:pB", Change::Started(None), WHO).unwrap();
        change(&root, "w2:pB", Change::Started(None), WHO).unwrap();
        assert_eq!(read(&root, "w1:pA").working, 1);
        assert_eq!(read(&root, "w2:pB").working, 2);
        fs::remove_dir_all(&root).unwrap();
    }
}
