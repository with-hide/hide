//! The per-pane subagent count the hook keeps between invocations.
//!
//! A hook script is stateless: it is started fresh for every event and knows
//! only its environment. The count therefore lives here, keyed by
//! `$HERDR_PANE_ID`, and is republished to Herdr after every event so the
//! core reads it back out of the pane tokens its ordinary snapshot already
//! carries (PRD D-53, and Technical structure).

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::runtime::{AgentRuntime, HOOK_VERSION, HookEvent};

/// The most subagents one session may have running at once, and the most
/// finished ones it remembers by id. A record is read and rewritten whole by
/// every event, so it is bounded; crossing the running cap is a failed hook
/// event the pane's last report failure shows, not a larger record.
const RUNNING_LIMIT: usize = 256;
const FINISHED_LIMIT: usize = 256;

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

/// What the file of one pane holds: the counts, and who reported them.
///
/// Herdr keeps the tokens a report sets only as long as the server that took
/// it, so a handoff or a restart empties them while this file stays and the
/// pane ids stay. Which pane the file is for is therefore known, and what
/// else a restore must know is whether the agent running there now is the
/// one that reported: `version` is the hook version of the helper that wrote
/// the file, `agent` the adapter id of the runtime that reported, and
/// `session` the agent's own id of the session that did, when the hook's
/// input named it. A file an older helper wrote has none of them and is not
/// restored from.
///
/// A record belongs to one session: an event of another session starts it
/// over, and an event of a session Herdr does not report for the pane never
/// reaches it ([`change_for_pane`]).
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

    /// Whether this file already says what an event of `who` would write.
    /// An event that names no session says nothing against the file's.
    fn names(&self, who: Reporter<'_>) -> bool {
        self.version == Some(HOOK_VERSION)
            && self.agent.as_deref() == who.agent
            && who
                .session
                .is_none_or(|session| self.session.as_deref() == Some(session))
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

/// What the pane's file said last, as a restore reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Restorable {
    pub counters: PaneCounters,
    pub version: u32,
    /// The adapter id of the runtime that reported.
    pub agent: String,
    /// The session that reported, when its hook's input named one.
    pub session: Option<String>,
}

/// What a restore can do with a pane's file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Restore {
    /// No event ever counted this pane here.
    NoRecord,
    /// An older helper wrote the file, or a hook that did not know its own
    /// runtime: which agent reported is unknown, so nothing is guessed and the
    /// pane waits for its next event.
    Unpairable,
    /// What the pane's last report said.
    Report(Restorable),
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
            | HookEvent::PostToolUseFailure => Self::None,
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

/// What a change did to the pane's file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Applied {
    /// The pane's counts after the change.
    pub counters: PaneCounters,
    /// The event came from a session that is not the pane's agent session as
    /// Herdr reports it, so the file was left as it was.
    pub foreign: bool,
}

/// The pane's record as the last finished change left it, read under the
/// shared side of the lock, so a change being written is never read half
/// way; a record that cannot be read fails rather than answering zeros.
pub fn read_settled(home: &Path, pane_id: &str) -> io::Result<PaneCounters> {
    read_record(home, pane_id).map(|record| record.counters())
}

/// What a pane's last report said, read under the shared side of the lock
/// like [`read_settled`], for putting it back on a Herdr that lost it.
pub fn restore_of(home: &Path, pane_id: &str) -> io::Result<Restore> {
    match read_record(home, pane_id) {
        Ok(record) => Ok(match (&record.version, &record.agent) {
            (Some(version), Some(agent)) => Restore::Report(Restorable {
                counters: record.counters(),
                version: *version,
                agent: agent.clone(),
                session: record.session.clone(),
            }),
            _ => Restore::Unpairable,
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Restore::NoRecord),
        Err(error) => Err(error),
    }
}

fn read_record(home: &Path, pane_id: &str) -> io::Result<Record> {
    let _held = hold(home, hide_platform::fs::lock::Mode::Shared)?;
    let raw = fs::read(record_path(home, pane_id))?;
    serde_json::from_slice(&raw).map_err(io::Error::from)
}

pub fn read(home: &Path, pane_id: &str) -> PaneCounters {
    read_lenient(&record_path(home, pane_id)).counters()
}

/// Applies one [`Change`] under the pane's lock and returns the new counts,
/// for a caller that has no pane session to compare with.
pub fn change(
    home: &Path,
    pane_id: &str,
    change: Change<'_>,
    who: Reporter<'_>,
) -> io::Result<PaneCounters> {
    change_for_pane(home, pane_id, change, who, || None).map(|applied| applied.counters)
}

/// Applies one [`Change`] to the pane's record, unless the event is another
/// session's.
///
/// `pane_session` asks for the session Herdr reports for the pane's agent. A
/// session that is not that one is a foreign one (a nested `claude -p` that a
/// user's own Stop hook starts runs the same hooks with the pane's
/// environment), and its events change nothing: Herdr, not the hook's
/// environment, says which session the pane's agent is. Without a session
/// every event is the pane's, as the events of a runtime that names no session
/// always were.
///
/// It is asked at most once, before the lock is taken (a socket round trip
/// never waits inside it), and not at all for an event that changes no count
/// and finds the record already naming its reporter.
///
/// The record is read, changed and written while the lock is held, so two
/// events of one pane never both start from the same count. The write is not
/// synced: a count describes running sessions and is rebuilt by their next
/// events, so it does not have to survive a crash, and a sync inside the lock
/// would make parallel subagent starts wait out the lock.
pub fn change_for_pane(
    home: &Path,
    pane_id: &str,
    change: Change<'_>,
    who: Reporter<'_>,
    pane_session: impl FnOnce() -> Option<String>,
) -> io::Result<Applied> {
    let path = record_path(home, pane_id);
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)?;
    }
    if change == Change::None {
        // An event that changes nothing leaves a file that already names this
        // reporter alone, so the common event stays a shared read. A pane with
        // no file yet, or one that names another version, agent or session,
        // gets one below: every pane that has reported has a file that says
        // who reported, which is what a restore reads.
        match read_record(home, pane_id) {
            Ok(record) if record.names(who) => {
                return Ok(Applied {
                    counters: record.counters(),
                    foreign: false,
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if pane_session().is_some_and(|session| who.session != Some(session.as_str())) {
        let counters = match read_record(home, pane_id) {
            Ok(record) => record.counters(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => PaneCounters::default(),
            Err(error) => return Err(error),
        };
        return Ok(Applied {
            counters,
            foreign: true,
        });
    }
    let held = hold(home, hide_platform::fs::lock::Mode::Exclusive)?;
    let mut record = read_lenient(&path);
    // A record belongs to one session: the first event of another one, that
    // the pane's agent now is, starts it over. A record that names no session
    // or agent (an older helper's, or a runtime's that reads none) is taken
    // over by the first one that does, with its counts.
    let new_session = who.session.is_some()
        && (record
            .session
            .as_deref()
            .is_some_and(|session| Some(session) != who.session)
            || record
                .agent
                .as_deref()
                .is_some_and(|agent| Some(agent) != who.agent));
    if new_session {
        record = Record::default();
    }
    let kept_session = record.session.clone();
    let kept_agent = record.agent.clone();
    match change {
        // A session that names itself has a fresh record already, or is
        // the record's own; one that names none cannot say which.
        Change::SessionStarted => {
            if who.session.is_none() {
                record = Record::default();
            }
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
    let session = match change {
        // Not evidence of another session, so the one the file names stays
        // unless this event names its own, and only for the same agent.
        Change::None => who
            .session
            .map(str::to_owned)
            .or_else(|| kept_session.filter(|_| kept_agent.as_deref() == who.agent)),
        _ => who.session.map(str::to_owned),
    };
    let counters = record.counters();
    write_record(&path, record, who, session)?;
    drop(held);
    Ok(Applied {
        counters,
        foreign: false,
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

/// Replaces a pane's counts with ones its agent keeps itself: OpenCode's
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
    let record = Record {
        working: counters.working,
        done: counters.done,
        ..Record::default()
    };
    write_record(&path, record, who, who.session.map(str::to_owned))?;
    drop(held);
    Ok(())
}

/// The file as it is, or an empty record for one that is missing or cannot be
/// read: the next write replaces it whole, under the lock the caller holds.
fn read_lenient(path: &Path) -> Record {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

/// Writes a record in place, private to the account, under a held lock.
fn write_record(
    path: &Path,
    record: Record,
    who: Reporter<'_>,
    session: Option<String>,
) -> io::Result<()> {
    let mut file = hide_platform::fs::private::open_or_create_file(path)?;
    file.set_len(0)?;
    file.write_all(&serde_json::to_vec(&Record {
        version: Some(HOOK_VERSION),
        agent: who.agent.map(str::to_owned),
        session,
        ..record
    })?)
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

    fn restorable(working: u32, done: u32, session: Option<&str>) -> Restore {
        Restore::Report(Restorable {
            counters: PaneCounters { working, done },
            version: HOOK_VERSION,
            agent: "claude-code".into(),
            session: session.map(str::to_owned),
        })
    }

    #[test]
    fn a_pane_that_reported_has_a_file_that_says_who_reported_what() {
        let root = home("restore");
        let pane = "w7B:pM";
        assert_eq!(restore_of(&root, pane).unwrap(), Restore::NoRecord);
        change(&root, pane, Change::Started(None), WHO).unwrap();
        change(&root, pane, Change::Started(None), WHO).unwrap();
        change(&root, pane, Change::Stopped(None), WHO).unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            restorable(1, 1, Some("session-a"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_event_that_names_another_session_starts_the_file_over_for_it() {
        let root = home("restore-session");
        let pane = "w7B:pM";
        change(&root, pane, Change::Started(None), WHO).unwrap();
        let other = Reporter {
            session: Some("session-b"),
            ..WHO
        };
        change(&root, pane, Change::Started(None), other).unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            restorable(1, 0, Some("session-b"))
        );
        // A hook that could not read its input names no session, and the
        // count it changed is not the earlier session's to claim.
        let unknown = Reporter {
            session: None,
            ..WHO
        };
        change(&root, pane, Change::Started(None), unknown).unwrap();
        assert_eq!(restore_of(&root, pane).unwrap(), restorable(2, 0, None));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_event_that_changes_nothing_keeps_the_session_for_the_same_agent_only() {
        let root = home("restore-none");
        let pane = "w7B:pM";
        change(&root, pane, Change::SessionStarted, WHO).unwrap();
        let blind = Reporter {
            session: None,
            ..WHO
        };
        change(&root, pane, Change::None, blind).unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            restorable(0, 0, Some("session-a"))
        );
        // Another runtime in the pane: the earlier session is not its own.
        let codex = Reporter {
            agent: Some("codex"),
            session: None,
        };
        change(&root, pane, Change::None, codex).unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            Restore::Report(Restorable {
                counters: PaneCounters::default(),
                version: HOOK_VERSION,
                agent: "codex".into(),
                session: None,
            })
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_event_that_changes_nothing_still_leaves_a_file_for_a_pane_with_none() {
        let root = home("restore-first");
        let pane = "w7B:pM";
        assert_eq!(
            change(&root, pane, Change::None, WHO).unwrap(),
            PaneCounters::default()
        );
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            restorable(0, 0, Some("session-a"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_an_older_helper_wrote_is_unpairable_and_the_next_event_rewrites_it() {
        let root = home("restore-old");
        let pane = "w7B:pM";
        fs::create_dir_all(state_directory(&root)).unwrap();
        // Counts and nothing else, or a version without who reported.
        for old in [
            br#"{"working":2,"done":4}"#.as_slice(),
            br#"{"working":2,"done":4,"version":6}"#.as_slice(),
        ] {
            fs::write(record_path(&root, pane), old).unwrap();
            assert_eq!(restore_of(&root, pane).unwrap(), Restore::Unpairable);
            // The counts are still read the way they always were.
            assert_eq!(
                read_settled(&root, pane).unwrap(),
                PaneCounters {
                    working: 2,
                    done: 4
                }
            );
        }
        change(&root, pane, Change::None, WHO).unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap(),
            restorable(2, 4, Some("session-a"))
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_not_an_empty_restore() {
        let root = home("restore-corrupt");
        let pane = "w7B:pM";
        fs::create_dir_all(state_directory(&root)).unwrap();
        fs::write(record_path(&root, pane), b"not a record").unwrap();
        assert_eq!(
            restore_of(&root, pane).unwrap_err().kind(),
            io::ErrorKind::InvalidData
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
    // the pane's environment, in a session that has no transcript.
    #[test]
    fn a_session_herdr_does_not_report_for_the_pane_leaves_its_counts_alone() {
        let root = home("foreign");
        let pane = "w7B:pM";
        let pane_session = Some("session-a");
        let nested = Reporter {
            session: Some("nested"),
            ..WHO
        };
        change(&root, pane, Change::Started(Some("a")), WHO).unwrap();
        change(&root, pane, Change::Stopped(Some("a")), WHO).unwrap();
        change(&root, pane, Change::Started(Some("b")), WHO).unwrap();
        let before = fs::read(record_path(&root, pane)).unwrap();
        for event in [
            Change::SessionStarted,
            Change::None,
            Change::Started(Some("x")),
            Change::Listed { running: &[] },
            Change::Settled { running: 0 },
        ] {
            let applied = change_for_pane(&root, pane, event, nested, || {
                pane_session.map(str::to_owned)
            })
            .unwrap();
            assert!(applied.foreign, "{event:?}");
            assert_eq!(
                applied.counters,
                PaneCounters {
                    working: 1,
                    done: 1
                }
            );
        }
        assert_eq!(
            fs::read(record_path(&root, pane)).unwrap(),
            before,
            "the file is byte for byte what the pane's own events left"
        );
        // The pane's own session goes on, and a session that names none is not it.
        let own = change_for_pane(&root, pane, Change::Stopped(Some("b")), WHO, || {
            pane_session.map(str::to_owned)
        })
        .unwrap();
        assert!(!own.foreign);
        assert_eq!((own.counters.working, own.counters.done), (0, 2));
        // A session that names none is not the pane's when it would change
        // something.
        let blind = Reporter {
            session: None,
            ..WHO
        };
        assert!(
            change_for_pane(&root, pane, Change::Started(Some("y")), blind, || {
                pane_session.map(str::to_owned)
            })
            .unwrap()
            .foreign
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_event_that_changes_no_count_asks_herdr_only_when_it_would_write() {
        let root = home("lazy-session");
        let pane = "w7B:pM";
        let asked = std::cell::Cell::new(0);
        let ask = || {
            asked.set(asked.get() + 1);
            Some("session-a".to_owned())
        };
        // No file yet: the event writes one, so Herdr is asked.
        change_for_pane(&root, pane, Change::None, WHO, ask).unwrap();
        assert_eq!(asked.get(), 1);
        // The file names this reporter: a shared read, no question.
        change_for_pane(&root, pane, Change::None, WHO, ask).unwrap();
        assert_eq!(asked.get(), 1);
        // Another session's event that changes nothing is asked about and
        // leaves the file as it was.
        let nested = Reporter {
            session: Some("nested"),
            ..WHO
        };
        let before = fs::read(record_path(&root, pane)).unwrap();
        let applied = change_for_pane(&root, pane, Change::None, nested, ask).unwrap();
        assert!(applied.foreign);
        assert_eq!(asked.get(), 2);
        assert_eq!(fs::read(record_path(&root, pane)).unwrap(), before);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_pane_herdr_names_no_session_for_takes_every_event_as_its_own() {
        let root = home("no-pane-session");
        let pane = "w7B:pM";
        let applied =
            change_for_pane(&root, pane, Change::Started(Some("a")), WHO, || None).unwrap();
        assert!(!applied.foreign);
        assert_eq!(applied.counters.working, 1);
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
        let file: Record =
            serde_json::from_slice(&fs::read(record_path(&root, pane)).unwrap()).unwrap();
        assert_eq!(file.finished.len(), FINISHED_LIMIT);
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
