//! The import record (PRD engine-bundle-4-herdr-removal D-25): one per
//! machine, in that machine's engine folder (`hide_kit::layout::
//! herdr_import_in`), the one owner of what moving its Herdr session into
//! the core's own engine leaves between two starts of the daemon.
//!
//! It holds what the Herdr save was found to hold and is not moved yet (the
//! workspaces whose Hide state the engine's cleanup must not drop until they
//! are moved, or nothing could be read and it must drop only what the
//! engine made itself), the workspaces moved (Hide's from then on, never
//! offered again, D-9), the conversations waiting to resume because the
//! Herdr they came from still has them open (D-8), the move the operator
//! asked for and the core starts with, the mark that the move was committed
//! (D-27: after it no older build is started on the state folder again),
//! and what the last move came to, which the screen shows.
//!
//! Every step runs while the core starts, before the engine opens on the
//! folder: that is the one path a Herdr session takes into the engine, for a
//! move at a reboot, at the update, later, or for a read-only import (D-7),
//! so nothing else writes the engine's session then.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

pub use hide_node_link::herdr_save::Fetched;

pub(crate) mod device;

use crate::engine::own::herdr_session::{self, HerdrRead, PendingResume};

/// The format this build writes and reads.
pub const FORMAT: u32 = 1;

/// The largest record read: a record of a session of 1024 workspaces with a
/// waiting conversation in each of 1024 panes stays far below it.
const READ_CAP: u64 = 4 * 1024 * 1024;

/// The import record of one machine's engine folder.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    pub(crate) format: u32,
    /// What the Herdr save held when last read.
    pub(crate) found: Found,
    /// The workspaces moved into the engine.
    pub(crate) moved: BTreeSet<String>,
    /// The workspaces an import was writing into the engine's session when
    /// it last ran: those the engine holds are moved, the rest are not.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) moving: BTreeSet<String>,
    pub(crate) pending: Vec<PendingResume>,
    /// The panes the last import gave a waiting conversation back to, until
    /// the engine's open resumed it there or set it waiting again (B15).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) given_back: BTreeSet<String>,
    /// The move the core starts with next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) asked: Option<Asked>,
    /// Set right before the first step of a move that cannot be undone.
    pub(crate) committed: bool,
    /// What the last move or read came to, for the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) outcome: Option<Outcome>,
}

/// What reading a machine's Herdr save found.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Found {
    /// No save, or none read yet: nothing of Herdr's is held.
    #[default]
    Nothing,
    /// The save's workspaces not moved yet, and the save as it was read, so
    /// a save Herdr changed since is offered again.
    NotMoved {
        workspaces: BTreeSet<String>,
        stamp: Stamp,
    },
    /// A save is there and could not be read.
    Unknown { reason: String },
}

/// A file's length and last change, which tell a save Herdr wrote again.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stamp {
    pub(crate) len: u64,
    pub(crate) modified_ns: u64,
}

/// A move the operator asked for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Asked {
    /// Whether the agents resume: false while the Herdr the save is read
    /// from runs on with their conversations open (D-8).
    pub resume: bool,
}

/// What the last move or read came to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Moved {
        workspaces: usize,
        tabs: usize,
        agents: usize,
        waiting: usize,
        refused: Vec<RefusedWorkspace>,
    },
    /// The save could not be read: why, in one line.
    Unreadable { reason: String },
    /// Hide's Herdr could not be stopped, so nothing was moved.
    NotStopped,
}

/// A workspace a move left out, with its name and why.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefusedWorkspace {
    pub workspace: String,
    pub label: String,
    pub reason: String,
}

/// What the cleanup of a machine's pane state must leave alone (D-25).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Held {
    /// Nothing: every id the machine's engine does not hold may go.
    #[default]
    Nothing,
    /// The state of every id in these workspaces of the Herdr save.
    Workspaces(BTreeSet<String>),
    /// Everything but what the engine made itself, since the save could not
    /// be read.
    Unknown,
}

impl Held {
    /// Whether a cleanup that holds the machine's whole topology may drop the
    /// state keyed by the machine's `id` (a workspace, tab or pane id).
    pub(crate) fn releases(&self, id: &str) -> bool {
        match self {
            Self::Nothing => true,
            Self::Workspaces(workspaces) => {
                !workspaces.contains(id.split_once(':').map_or(id, |(workspace, _)| workspace))
            }
            Self::Unknown => crate::engine::own::made_by_engine(id),
        }
    }
}

impl Record {
    fn new() -> Self {
        Self {
            format: FORMAT,
            ..Self::default()
        }
    }

    /// What the cleanup must leave alone while this record stands.
    pub(crate) fn held(&self) -> Held {
        match &self.found {
            Found::Nothing => Held::Nothing,
            Found::NotMoved { workspaces, .. } if workspaces.is_empty() => Held::Nothing,
            Found::NotMoved { workspaces, .. } => Held::Workspaces(workspaces.clone()),
            Found::Unknown { .. } => Held::Unknown,
        }
    }
}

fn path_in(engine_dir: &Path) -> PathBuf {
    hide_kit::layout::herdr_import_in(engine_dir)
}

/// The record in `engine_dir`, `None` when there is none yet.
pub(crate) fn read(engine_dir: &Path) -> Result<Option<Record>, String> {
    let path = path_in(engine_dir);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be opened: {error}", path.display())),
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, READ_CAP + 1), &mut text)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    if text.len() as u64 > READ_CAP {
        return Err(format!(
            "{} is larger than {READ_CAP} bytes",
            path.display()
        ));
    }
    let record: Record = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not an import record: {error}", path.display()))?;
    if record.format != FORMAT {
        return Err(format!(
            "{} is format {}, this build reads format {FORMAT}",
            path.display(),
            record.format
        ));
    }
    Ok(Some(record))
}

/// What the cleanup of `engine_dir`'s machine must leave alone: what its
/// record holds, and everything the engine did not make while the record
/// cannot be read, since then nothing says what is Herdr's.
pub(crate) fn held(engine_dir: &Path) -> Held {
    match read(engine_dir) {
        Ok(record) => record.map(|record| record.held()).unwrap_or_default(),
        Err(reason) => {
            crate::diagnostic!(serde_json::json!({
                "component": "herdr_import",
                "kind": "herdr_import.record_unreadable",
                "reason": reason,
            }));
            Held::Unknown
        }
    }
}

fn write(engine_dir: &Path, record: &Record) -> Result<(), String> {
    let path = path_in(engine_dir);
    let bytes = serde_json::to_vec(record)
        .map_err(|error| format!("the import record could not be written out: {error}"))?;
    hide_platform::fs::private::create_dir_all(engine_dir)
        .map_err(|error| format!("{} could not be made: {error}", engine_dir.display()))?;
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{} could not be replaced: {error}", path.display()))
}

/// One change of an import record at a time in this process: each change
/// reads the record, changes it and writes it back holding this, so a look,
/// a move and the engine's restore, each on its own thread, never write over
/// what another changed meanwhile.
static CHANGING: Mutex<()> = Mutex::new(());

/// This thread's turn to change a record, which [`open`] asks for.
type Changing = MutexGuard<'static, ()>;

fn changing() -> Changing {
    CHANGING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Reads `engine_dir`'s record, or starts one, for a change made in `turn`;
/// a record that cannot be read is refused, so no step runs on a guess of
/// what it held.
fn open(engine_dir: &Path, _turn: &Changing) -> Result<Record, String> {
    let mut record = read(engine_dir)?.unwrap_or_else(Record::new);
    // An import that ended while it wrote the engine's session: what the
    // engine holds of it moved, the rest did not (principle #11).
    if !record.moving.is_empty() {
        let held = crate::engine::own::saved_workspaces(engine_dir);
        let moving = std::mem::take(&mut record.moving);
        record.moved.extend(
            moving
                .into_iter()
                .filter(|workspace| held.contains(workspace)),
        );
    }
    Ok(record)
}

/// Whether the pane, tab or workspace `id` of this machine belongs to the
/// Herdr save and is not moved yet: a `hide` called there is refused, and
/// what it reports changes nothing on screen (D-7, B10).
pub fn holds(engine_dir: &Path, id: &str) -> bool {
    !held(engine_dir).releases(id)
}

/// The panes whose conversation waits in the Herdr it came from (D-8,
/// B15): a restore opens their shell saying so. A record that cannot be read
/// names none.
pub(crate) fn waiting(engine_dir: &Path) -> BTreeSet<String> {
    read(engine_dir)
        .ok()
        .flatten()
        .map(|record| {
            record
                .pending
                .into_iter()
                .map(|pending| pending.pane)
                .collect()
        })
        .unwrap_or_default()
}

/// The panes the last import gave a waiting conversation back to (B15).
pub(crate) fn given_back(engine_dir: &Path) -> BTreeSet<String> {
    read(engine_dir)
        .ok()
        .flatten()
        .map(|record| record.given_back)
        .unwrap_or_default()
}

/// The engine's open settled the conversations given back, each resumed in
/// its pane or a new tab of its workspace; `busy`, as `(pane, kind, name,
/// session)`, are the ones whose tab could not open, which wait again for
/// the next import.
pub(crate) fn wait_again(
    engine_dir: &Path,
    busy: impl IntoIterator<Item = (String, String, Option<String>, Option<String>)>,
) -> Result<(), String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    record.given_back.clear();
    record.pending.extend(
        busy.into_iter()
            .map(|(pane, kind, name, session)| PendingResume {
                pane,
                kind,
                name,
                session,
            }),
    );
    write(engine_dir, &record)
}

/// The workspaces moved into `engine_dir`'s engine.
pub fn moved(engine_dir: &Path) -> Result<BTreeSet<String>, String> {
    Ok(open(engine_dir, &changing())?.moved)
}

/// A device's save as its node read it when a move of it was asked, with
/// the device's machine id, which its registrations name: the core's next
/// start moves it before any link to the device is up (D-7, D-10).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeptSave {
    pub machine: String,
    pub save: Fetched,
}

/// Keeps `kept` in the device's engine folder `engine_dir`.
pub fn keep_save(engine_dir: &Path, kept: &KeptSave) -> Result<(), String> {
    let path = hide_kit::layout::herdr_save_kept_in(engine_dir);
    let bytes = serde_json::to_vec(kept)
        .map_err(|error| format!("the device's save could not be written out: {error}"))?;
    hide_platform::fs::private::create_dir_all(engine_dir)
        .map_err(|error| format!("{} could not be made: {error}", engine_dir.display()))?;
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{} could not be replaced: {error}", path.display()))
}

/// The save kept in `engine_dir`, `None` when none is.
pub fn kept_save(engine_dir: &Path) -> Result<Option<KeptSave>, String> {
    let path = hide_kit::layout::herdr_save_kept_in(engine_dir);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be opened: {error}", path.display())),
    };
    // The save and its JSON quoting.
    let cap = 2 * hide_node_link::herdr_save::READ_CAP + 4096;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, cap + 1), &mut text)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    if text.len() as u64 > cap {
        return Err(format!("{} is larger than {cap} bytes", path.display()));
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("{} is not a kept save: {error}", path.display()))
}

/// Forgets the save kept in `engine_dir` once its move is made.
pub fn drop_kept_save(engine_dir: &Path) -> Result<(), String> {
    let path = hide_kit::layout::herdr_save_kept_in(engine_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{} could not be removed: {error}", path.display())),
    }
}

/// Whether Hide registered an agent of this machine on the Herdr socket
/// `herdr_socket`, which makes a save beside a socket no server listens on
/// Hide's (D-5). A ledger that cannot be read says no.
pub fn hide_used(state_dir: &Path, herdr_socket: &str) -> bool {
    crate::delivery::ledger::load(&hide_kit::layout::delivery_ledger(state_dir)).is_ok_and(
        |ledger| {
            ledger
                .agents
                .iter()
                .any(|record| record.host_scope == herdr_socket)
        },
    )
}

/// What moves this machine's Herdr save for the screen (D-6 to D-8, B2 to
/// B5, B11, B14 to B17): `hided` reads the save and the server on its
/// socket and makes the move, and the core decides what is shown, asking it
/// on a worker of its own, never under the runtime lock, as it asks the
/// takeover (`herdr_takeover.rs`).
pub trait Machine: Send + Sync {
    /// What the save offers, whose Herdr runs on its socket, and what the
    /// last move came to while the operator has not read it.
    fn look(&self) -> Seen;
    /// Moves the save the way `take` says. Once it answers
    /// [`Taken::Restarting`], the core role starts again and moves it
    /// before the engine opens (D-7).
    fn take(&self, take: Take) -> Taken;
    /// The last move's result was read.
    fn dismiss(&self);
    /// Starts the core role again, whose start makes the move a device's
    /// record asks for before any link to it is up (D-7, D-10).
    fn restart(&self);
}

/// One look at this machine's Herdr save.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Seen {
    pub look: Look,
    pub server: Server,
    /// The last move's result, while the operator has not read it.
    pub outcome: Option<Outcome>,
}

/// Whose Herdr runs on the socket beside the save (D-5).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Server {
    /// None does.
    #[default]
    Stopped,
    /// Hide's own: moving the save stops it first (D-6).
    Hide,
    /// Someone else's, or one whose program the system will not name: its
    /// save is only imported, and it runs on untouched (D-8).
    Other,
}

/// How the operator asked for the save to move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Take {
    /// Hide's Herdr is stopped, then its session moves and its agents
    /// resume (D-6).
    Move,
    /// The save is imported and nothing is stopped; agents whose Herdr runs
    /// on are left waiting (D-8).
    Import,
}

/// What asking for a move came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Taken {
    /// The move is asked for and the core role starts again with it.
    Restarting,
    /// Hide's Herdr could not be stopped, so nothing moved (B4).
    NotStopped,
    /// Nothing moved: why, for the log.
    Failed(String),
}

/// The last move's result, while the operator has not read it.
pub fn outcome(engine_dir: &Path) -> Result<Option<Outcome>, String> {
    Ok(read(engine_dir)?.and_then(|record| record.outcome))
}

/// The operator read the last move's result.
pub fn dismiss(engine_dir: &Path) -> Result<(), String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    if record.outcome.take().is_none() {
        return Ok(());
    }
    write(engine_dir, &record)
}

/// What a machine's Herdr save offers to move.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Look {
    /// No save, or every workspace in it moved: nothing is shown.
    #[default]
    Nothing,
    Offer(Offer),
    /// A save is there and could not be read: why, in one line.
    Unreadable(String),
}

/// What moving the save would move.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Offer {
    pub workspaces: usize,
    pub tabs: usize,
    pub agents: usize,
    /// Conversations an earlier import left waiting in the Herdr they came
    /// from, which an import resumes once that Herdr no longer runs (B15).
    pub waiting: usize,
}

/// Reads the Herdr save its machine read (`save`), records what of
/// it is not moved in `engine_dir`'s record, and says what moving it would
/// move. It runs at every start of the core before the engine opens, so the
/// engine's cleanup never owns a pane of the save before it is moved.
pub fn look(engine_dir: &Path, save: &Fetched) -> Result<Look, String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    let look = match herdr_session::of(save) {
        HerdrRead::Missing => {
            record.found = Found::Nothing;
            // Conversations waiting from an earlier import go back without
            // the save that held them.
            if record.pending.is_empty() {
                Look::Nothing
            } else {
                Look::Offer(Offer {
                    workspaces: 0,
                    tabs: 0,
                    agents: 0,
                    waiting: record.pending.len(),
                })
            }
        }
        HerdrRead::Unreadable(reason) => {
            record.found = Found::Unknown {
                reason: reason.clone(),
            };
            Look::Unreadable(reason)
        }
        HerdrRead::Read(session) => {
            let workspaces = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            record.found = Found::NotMoved {
                workspaces,
                stamp: stamp(save),
            };
            let held = crate::engine::own::saved_workspaces(engine_dir);
            let converted = herdr_session::convert(&session, &record.moved, &held, String::new);
            let offer = Offer {
                workspaces: converted.moved.len(),
                tabs: converted.tabs(),
                agents: converted.agents(),
                waiting: record.pending.len(),
            };
            if offer.workspaces == 0 && offer.waiting == 0 {
                Look::Nothing
            } else {
                Look::Offer(offer)
            }
        }
    };
    write(engine_dir, &record)?;
    Ok(look)
}

/// Whether a save Herdr changed since `engine_dir`'s record read it waits to
/// be read again: an offer the operator put off is made again then (D-9).
pub fn changed_since_read(engine_dir: &Path, save: &Fetched) -> bool {
    match read(engine_dir) {
        Ok(Some(Record {
            found: Found::NotMoved { stamp: read, .. },
            ..
        })) => read != stamp(save),
        _ => true,
    }
}

fn stamp(save: &Fetched) -> Stamp {
    match save {
        Fetched::Text {
            len, modified_ns, ..
        } => Stamp {
            len: *len,
            modified_ns: *modified_ns,
        },
        Fetched::Missing | Fetched::Unreadable { .. } => Stamp::default(),
    }
}

/// Records the move the operator asked for, which the core starts with next.
pub fn ask(engine_dir: &Path, asked: Asked) -> Result<(), String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    record.asked = Some(asked);
    write(engine_dir, &record)
}

/// The move the core is to start with, if one was asked for.
pub fn asked(engine_dir: &Path) -> Result<Option<Asked>, String> {
    Ok(read(engine_dir)?.and_then(|record| record.asked))
}

/// Marks the move committed (D-27), right before its first step that cannot
/// be undone: from then on no older build is started on this state folder.
pub fn commit(engine_dir: &Path) -> Result<(), String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    if record.committed {
        return Ok(());
    }
    record.committed = true;
    write(engine_dir, &record)
}

/// Whether a move was committed on `engine_dir`'s machine; a record that
/// cannot be read counts as committed, so no older build is started on a
/// guess.
pub fn committed(engine_dir: &Path) -> bool {
    read(engine_dir).map_or(true, |record| record.is_some_and(|record| record.committed))
}

/// Records that Hide's Herdr could not be stopped, so nothing was moved, and
/// puts the asked move away: the operator asks again from the screen.
pub fn not_stopped(engine_dir: &Path) -> Result<(), String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    record.asked = None;
    record.outcome = Some(Outcome::NotStopped);
    write(engine_dir, &record)
}

/// Moves the workspaces of the Herdr save its machine read (`save`) that no import
/// moved into the engine's session in `engine_dir` (D-3), recording which
/// moved, which were left out and why, and the conversations left waiting
/// when `asked.resume` is false; the asked move is done after it, whatever
/// it came to. A save that cannot be read moves nothing and says why.
pub fn import(engine_dir: &Path, save: &Fetched, asked: Asked) -> Result<Outcome, String> {
    let turn = changing();
    let mut record = open(engine_dir, &turn)?;
    record.asked = None;
    // The conversations that waited while their Herdr ran go back to their
    // panes once it no longer does (D-8, B15).
    let given = if asked.resume && !record.pending.is_empty() {
        herdr_session::give_back(engine_dir, &record.pending)?
    } else {
        BTreeSet::new()
    };
    record
        .pending
        .retain(|waiting| !given.contains(&waiting.pane));
    record.given_back.extend(given.iter().cloned());
    let outcome = match herdr_session::of(save) {
        HerdrRead::Missing => {
            record.found = Found::Nothing;
            Outcome::Moved {
                workspaces: 0,
                tabs: 0,
                agents: 0,
                waiting: 0,
                refused: Vec::new(),
            }
        }
        HerdrRead::Unreadable(reason) => {
            record.found = Found::Unknown {
                reason: reason.clone(),
            };
            Outcome::Unreadable { reason }
        }
        HerdrRead::Read(session) => {
            record.moving = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            write(engine_dir, &record)?;
            let imported =
                herdr_session::import(engine_dir, &session, &record.moved, asked.resume)?;
            record.moving.clear();
            record.moved.extend(imported.workspaces.iter().cloned());
            let waiting = imported.pending.len();
            record.pending.extend(imported.pending);
            let workspaces = session
                .ids()
                .filter(|workspace| !record.moved.contains(*workspace))
                .map(str::to_owned)
                .collect();
            record.found = Found::NotMoved {
                workspaces,
                stamp: stamp(save),
            };
            Outcome::Moved {
                workspaces: imported.workspaces.len(),
                tabs: imported.tabs,
                agents: imported.agents,
                waiting,
                refused: imported
                    .refused
                    .into_iter()
                    .map(|refused| RefusedWorkspace {
                        workspace: refused.workspace,
                        label: refused.label,
                        reason: refused.reason,
                    })
                    .collect(),
            }
        }
    };
    record.outcome = Some(outcome.clone());
    write(engine_dir, &record)?;
    Ok(outcome)
}

/// Whose registrations a handover gives, and the scope they take: this
/// machine's are scoped by its Herdr socket and take the engine's folder; a
/// device's are scoped by its device id, which they keep (D-4, D-10).
pub struct Handover<'a> {
    /// The machine id the registrations name.
    pub machine: &'a str,
    /// The device id the View openers name.
    pub device: &'a str,
    pub from_scope: &'a str,
    pub to_scope: &'a str,
}

/// Gives the Hide state of the panes of the moved workspaces `moved` to the
/// engine (D-4), in the state folder `state_dir`, the core move's way
/// (`node_migration/reown.rs`): each registration of `handover`'s machine
/// whose host scope is its `from_scope` and whose pane is in a moved
/// workspace takes its `to_scope`, and its Herdr terminal id becomes its
/// pane's id, the engine's terminal id, in the registration and in each View
/// opener of its device that named it. Everything else Hide keeps of a pane
/// is keyed by the pane, tab or workspace id the move keeps, or by the
/// agent's session. A registration already given is left as it is, so a
/// start that runs it again changes nothing (principle #11). It runs before
/// the core starts, which owns both files once it runs. How many
/// registrations it gave.
pub fn adopt(
    state_dir: &Path,
    handover: &Handover<'_>,
    moved: &BTreeSet<String>,
) -> Result<usize, String> {
    let ledger_path = hide_kit::layout::delivery_ledger(state_dir);
    if !ledger_path.exists() {
        return Ok(0);
    }
    let mut ledger = crate::delivery::ledger::load(&ledger_path)
        .map_err(|reason| format!("the delivery ledger could not be read: {reason}"))?;
    let mut terminals = BTreeMap::new();
    let mut given = 0;
    for record in &mut ledger.agents {
        if record.machine != handover.machine
            || record.host_scope != handover.from_scope
            || !moved.contains(workspace_of(&record.pane))
            || (record.instance == record.pane
                && record
                    .terminal_id
                    .as_ref()
                    .is_none_or(|id| *id == record.pane)
                && record.host_scope == handover.to_scope)
        {
            continue;
        }
        handover.to_scope.clone_into(&mut record.host_scope);
        for terminal in std::iter::once(&record.instance).chain(record.terminal_id.as_ref()) {
            if *terminal != record.pane {
                terminals.insert(terminal.clone(), record.pane.clone());
            }
        }
        record.instance.clone_from(&record.pane);
        if record.terminal_id.is_some() {
            record.terminal_id = Some(record.pane.clone());
        }
        given += 1;
    }
    if given == 0 {
        return Ok(0);
    }
    // The Views first: a start cut short before the ledger is written finds
    // its registrations still Herdr's and maps their openers again.
    let views_path = state_dir.join(crate::node_migration::WORKSPACE_VIEWS);
    if let Some(mut views) = crate::node_migration::read_json(&views_path)
        .map_err(|error| format!("{} could not be read: {error}", views_path.display()))?
        && rename_openers(&mut views, handover.device, &terminals)
    {
        let bytes = serde_json::to_vec(&views)
            .map_err(|error| format!("the Views could not be written out: {error}"))?;
        crate::node_migration::write(&views_path, &bytes)
            .map_err(|error| format!("{} could not be replaced: {error}", views_path.display()))?;
    }
    crate::delivery::ledger::save(&ledger_path, &ledger)
        .map_err(|error| format!("the delivery ledger could not be written: {error}"))?;
    Ok(given)
}

/// The workspace part of a pane, tab or workspace id.
fn workspace_of(id: &str) -> &str {
    id.split_once(':').map_or(id, |(workspace, _)| workspace)
}

/// Renames, at any depth of `value`, the terminal of each View opener on
/// `node` that `terminals` maps; whether any moved.
fn rename_openers(
    value: &mut serde_json::Value,
    node: &str,
    terminals: &BTreeMap<String, String>,
) -> bool {
    use serde_json::Value;
    let rename = |opener: &mut Value| {
        if opener.get("device_id").and_then(Value::as_str) != Some(node) {
            return false;
        }
        let Some(pane) = opener
            .get("terminal_id")
            .and_then(Value::as_str)
            .and_then(|terminal| terminals.get(terminal))
        else {
            return false;
        };
        opener["terminal_id"] = Value::String(pane.clone());
        true
    };
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (key, entry) in map.iter_mut() {
                changed |= match (key.as_str(), entry) {
                    ("opener", opener @ Value::Object(_)) => rename(opener),
                    ("openers", Value::Array(openers)) => openers
                        .iter_mut()
                        .fold(false, |changed, opener| rename(opener) | changed),
                    (_, entry) => rename_openers(entry, node, terminals),
                };
            }
            changed
        }
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            rename_openers(item, node, terminals) | changed
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_host::herdr_save::fetch;

    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("herdr").join("session.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/herdr-session/herdr-0.9.5.json"),
            &path,
        )
        .unwrap();
        path
    }

    // D-25, B38: before a move, the save's workspaces are held; once moved,
    // nothing of them is, and the save is offered no more.
    #[test]
    fn a_look_holds_the_save_s_workspaces_until_they_are_moved() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        assert_eq!(
            look(&engine, &fetch(&save)).unwrap(),
            Look::Offer(Offer {
                workspaces: 2,
                tabs: 3,
                agents: 1,
                waiting: 0,
            })
        );
        let before = held(&engine);
        assert!(!before.releases("w1:p1") && !before.releases("w2"));
        assert!(before.releases("h1:p1") && before.releases("w9:p1"));

        let outcome = import(&engine, &fetch(&save), Asked { resume: true }).unwrap();
        assert!(matches!(
            outcome,
            Outcome::Moved {
                workspaces: 2,
                tabs: 3,
                agents: 1,
                waiting: 0,
                ..
            }
        ));
        assert_eq!(held(&engine), Held::Nothing);
        assert_eq!(look(&engine, &fetch(&save)).unwrap(), Look::Nothing);
        assert!(!changed_since_read(&engine, &fetch(&save)));
    }

    // D-25, B43: while the save cannot be read, only what the engine made
    // may go; a move then moves nothing and says why.
    #[test]
    fn an_unreadable_save_holds_all_but_the_engine_s_own_ids() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        std::fs::write(&save, "{").unwrap();
        assert!(matches!(
            look(&engine, &fetch(&save)).unwrap(),
            Look::Unreadable(_)
        ));
        let unknown = held(&engine);
        assert!(!unknown.releases("w1:p1") && !unknown.releases("x"));
        assert!(unknown.releases("h1:p1"));
        assert!(matches!(
            import(&engine, &fetch(&save), Asked { resume: true }).unwrap(),
            Outcome::Unreadable { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(&save).unwrap(),
            "{",
            "the save is only read"
        );
    }

    // Principle #11: an import that ended while it wrote the engine's
    // session finishes on the next start without moving anything twice.
    #[test]
    fn an_import_cut_short_converges_on_what_the_engine_holds() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        import(&engine, &fetch(&save), Asked { resume: true }).unwrap();
        let mut record = read(&engine).unwrap().unwrap();
        record.moving = record.moved.clone();
        record.moved.clear();
        write(&engine, &record).unwrap();
        let again = import(&engine, &fetch(&save), Asked { resume: true }).unwrap();
        assert!(
            matches!(again, Outcome::Moved { workspaces: 0, .. }),
            "{again:?}"
        );
        assert_eq!(
            crate::engine::own::saved_workspaces(&engine),
            BTreeSet::from(["w1".to_owned(), "w2".to_owned()])
        );
    }

    // B10: a pane of the save that is not moved yet is held, one the engine
    // made is not, and once moved none of the save's is.
    #[test]
    fn a_pane_of_the_save_is_held_until_its_workspace_moves() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        assert!(!holds(&engine, "w1:p1"));
        look(&engine, &fetch(&save)).unwrap();
        assert!(holds(&engine, "w1:p1"));
        assert!(!holds(&engine, "h1:p1"));
        import(&engine, &fetch(&save), Asked { resume: true }).unwrap();
        assert!(!holds(&engine, "w1:p1"));
    }

    // D-8, B15: an import beside a running Herdr leaves its conversation
    // waiting; one once Herdr stopped gives it back, and a pane that ran
    // another command at the open has it wait again.
    #[test]
    fn a_waiting_conversation_is_given_back_once_and_waits_again_when_its_pane_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fixture(dir.path());
        import(&engine, &fetch(&save), Asked { resume: false }).unwrap();
        assert!(waiting(&engine).contains("w1:p1"));
        import(&engine, &fetch(&save), Asked { resume: true }).unwrap();
        assert!(waiting(&engine).is_empty());
        assert_eq!(given_back(&engine), BTreeSet::from(["w1:p1".to_owned()]));
        wait_again(
            &engine,
            [("w1:p1".to_owned(), "claude".to_owned(), None, None)],
        )
        .unwrap();
        assert!(given_back(&engine).is_empty());
        assert!(waiting(&engine).contains("w1:p1"));
    }

    // A look and the engine's restore change one record from their own
    // threads: neither writes over what the other changed meanwhile, so no
    // waiting conversation is lost.
    #[test]
    fn changes_from_many_threads_at_once_lose_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        let save = fetch(&fixture(dir.path()));
        std::thread::scope(|scope| {
            for n in 0..16 {
                let (engine, save) = (&engine, &save);
                scope.spawn(move || {
                    look(engine, save).unwrap();
                    wait_again(
                        engine,
                        [(format!("w{n}:p1"), "claude".to_owned(), None, None)],
                    )
                    .unwrap();
                });
            }
        });
        assert_eq!(
            waiting(&engine),
            (0..16).map(|n| format!("w{n}:p1")).collect::<BTreeSet<_>>()
        );
    }

    // D-27: a move is committed once, and a record that cannot be read
    // counts as committed.
    #[test]
    fn a_commit_stands_and_an_unreadable_record_counts_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine");
        assert!(!committed(&engine));
        commit(&engine).unwrap();
        assert!(committed(&engine));
        std::fs::write(path_in(&engine), "{").unwrap();
        assert!(committed(&engine));
    }

    // D-4, B7: a moved pane's registration is scoped to the engine with its
    // pane as its terminal, and so is each View opener that named it; one
    // in a workspace not moved, or of another machine, stays Herdr's, and a
    // second pass changes nothing.
    #[test]
    fn a_moved_pane_s_registration_and_openers_are_given_to_the_engine_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let engine = hide_kit::layout::engine_dir(state);
        let socket = "/home/me/.config/herdr/herdr.sock";
        let agent = |id: &str, machine: &str, pane: &str, terminal: &str| {
            serde_json::json!({
                "id": id, "name": id, "machine": machine, "host_scope": socket,
                "native_machine": machine, "session": format!("s-{id}"),
                "instance": terminal, "terminal_id": terminal, "pane": pane,
                "parent": null, "project": "/work",
                "actor": {
                    "pane_id": pane, "name": id, "kind": "claude", "device_id": machine,
                    "session": crate::engine::tokens::session_digest(&format!("s-{id}")),
                },
                "ended": false,
            })
        };
        let ledger_path = hide_kit::layout::delivery_ledger(state);
        // The core reads only a private ledger, so it is written as the core
        // writes it.
        let ledger = crate::delivery::ledger::from_bytes(
            serde_json::json!({
                "version": 1, "next_id": 4, "letters": [], "watches": [],
                "agents": [
                    agent("agent-1", "mac", "w1:p1", "term_7"),
                    agent("agent-2", "mac", "w2:p1", "term_8"),
                    agent("agent-3", "mini", "w1:p1", "term_7"),
                ],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        crate::delivery::ledger::save(&ledger_path, &ledger).unwrap();
        let views_path = state.join(crate::node_migration::WORKSPACE_VIEWS);
        std::fs::write(
            &views_path,
            serde_json::json!({"displays": [
                {"id": "d1", "opener": {"device_id": "mac", "terminal_id": "term_7"}},
                {"id": "d2", "openers": [
                    {"device_id": "mini", "terminal_id": "term_7"},
                    {"device_id": "mac", "terminal_id": "term_8"},
                ]},
            ]})
            .to_string(),
        )
        .unwrap();
        let moved = BTreeSet::from(["w1".to_owned()]);

        let scope = engine.to_string_lossy();
        let handover = Handover {
            machine: "mac",
            device: "mac",
            from_scope: socket,
            to_scope: &scope,
        };
        assert_eq!(adopt(state, &handover, &moved).unwrap(), 1);
        let ledger = crate::delivery::ledger::load(&ledger_path).unwrap();
        let given = |id: &str| {
            let record = ledger.agents.iter().find(|record| record.id == id).unwrap();
            (
                record.host_scope.clone(),
                record.instance.clone(),
                record.terminal_id.clone(),
            )
        };
        let engine_scope = engine.to_string_lossy().into_owned();
        assert_eq!(
            given("agent-1"),
            (engine_scope, "w1:p1".to_owned(), Some("w1:p1".to_owned()))
        );
        for (id, terminal) in [("agent-2", "term_8"), ("agent-3", "term_7")] {
            assert_eq!(
                given(id),
                (
                    socket.to_owned(),
                    terminal.to_owned(),
                    Some(terminal.to_owned())
                )
            );
        }
        let views: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&views_path).unwrap()).unwrap();
        assert_eq!(views["displays"][0]["opener"]["terminal_id"], "w1:p1");
        assert_eq!(views["displays"][1]["openers"][0]["terminal_id"], "term_7");
        assert_eq!(views["displays"][1]["openers"][1]["terminal_id"], "term_8");

        let before = (
            std::fs::read(&ledger_path).unwrap(),
            std::fs::read(&views_path).unwrap(),
        );
        assert_eq!(adopt(state, &handover, &moved).unwrap(), 0);
        assert_eq!(
            (
                std::fs::read(&ledger_path).unwrap(),
                std::fs::read(&views_path).unwrap()
            ),
            before
        );
    }
}
