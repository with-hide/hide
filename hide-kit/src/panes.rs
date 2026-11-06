//! A state folder's panes and their holders, owned through their records
//! (PRD engine-bundle-2-pane-holder D-13, D-14, D-17).
//!
//! A pane's holder outlives the node that started it, so its owner is its
//! record (`hide_holder_protocol::record`), kept in the state folder, not the
//! node. This is the node's side of that contract, as a library hide-node
//! calls (and, in a later bundle, the core): starting a pane, ending one,
//! forgetting an ended one, and, when a node starts, reconciling the records
//! with the holders that run.
//!
//! - Every change of a record, the start's reservation and the count of
//!   holders against [`HOLDER_CAP`] happen under one lock per state folder,
//!   so starts at once never pass the cap, and a start asked again with the
//!   same pane id and command comes back as the pane already started rather
//!   than a second holder.
//! - Nothing that waits on another process runs under the lock: the start of
//!   a holder, the end of a tree and a connection to a holder happen outside
//!   it, and their outcome is written under it after a fresh read.
//! - The records are the only way to a holder: a node starts one only with
//!   its record's path, and the holder writes itself into that record under
//!   the lock before it starts its program, so every holder a node started is
//!   named by a record. A reconcile reads the records and nothing else. A
//!   record it cannot read, of a format it does not know, of another state
//!   folder or of a holder version too old to speak to is left as it is and
//!   reported with the reason, never taken for "no record".
//!
//! Diagnostics go to the sink the caller gives (`hide_node::diagnostic!` in
//! the node), one JSON record per event, naming the pane, the start's
//! incarnation and the holder's pid, never what a pane's program wrote.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{self, End, EndReason, Identity, PaneRecord, Phase, Unreadable};
use hide_holder_protocol::{
    ClientMessage, FrameError, HolderMessage, OLDEST_HOLDER_VERSION, OpenError, Size, Theme,
    read_holder, write_client,
};
use hide_platform::process::{ResidentCommand, start_resident};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{holders, layout};

mod ownership;
pub use ownership::Owner;
mod requests;
mod reservation;
#[cfg(test)]
use hide_holder_protocol::{PROTOCOL_VERSION, record::RECORD_FORMAT};
pub use requests::Request;

/// The most holders a state folder keeps: every record that is not ended
/// counts, and so does one that cannot be read, since it may name a holder.
pub const HOLDER_CAP: usize = 128;

/// The most ended records kept for their reasons; past it the oldest go.
pub const ENDED_KEPT: usize = 128;

/// The most record files a reconcile or a start reads; a folder holding more
/// is reported rather than read.
pub const MAX_RECORD_FILES: usize = HOLDER_CAP + ENDED_KEPT + 64;

/// How long a holder has to say it is ready, and how long a reservation
/// with no holder written in is taken as a start still under way.
pub const START_LIMIT: Duration = Duration::from_secs(10);

/// How long a holder asked to end has to say it ended: its tree's end (5 s),
/// its terminal's drain (5 s) and its last message.
pub const CLOSE_LIMIT: Duration = Duration::from_secs(20);

/// Whether `path` was last written before `booted`. A file whose time
/// cannot be read is not known to be old.
fn written_before(path: &Path, booted: std::time::SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|written| written < booted)
}

/// How many incarnations a process takes from the folder's file at once
/// ([`Panes::next_incarnation`]).
const INCARNATIONS_TAKEN: u64 = 64;

/// How many panes a reconcile judges at once.
pub const RECONCILE_WORKERS: usize = 8;

/// How long a node's reconcile takes up panes for; the judgments under way
/// then run to their own bounds, the longest a close's (connect, end and
/// exit, about 42 s), so a reconcile ends within about 72 s however many
/// panes it finds.
pub const RECONCILE_BUDGET: Duration = Duration::from_secs(30);

/// How long the lock of a holder known to have ended is waited out before
/// its socket is removed: Windows releases a dead process's file locks a
/// moment after the process ends.
const LOCK_RELEASE: Duration = Duration::from_secs(2);

/// How long a connection to a holder may take to be answered.
const CONNECT_LIMIT: Duration = Duration::from_secs(2);

/// How long ending what a lost holder left may take.
const LEFTOVER_LIMIT: Duration = Duration::from_secs(5);

/// Where diagnostics go: one JSON record each.
pub type Sink = fn(serde_json::Value);

/// The id a pane's holder record, socket and log carry for the pane a core
/// names `pane`. The core's own engine names its panes `<workspace>:p<n>`
/// (PRD engine-bundle-3-core-owns D-4), and no Windows file name holds a
/// `:` (`hide_holder_protocol::record::check_pane`), so each is written `_`;
/// the core never names a pane with a `_`, so two of its panes never share a
/// holder. An id without a `:` is its own holder id.
pub fn holder_pane(pane: &str) -> String {
    pane.replace(':', "_")
}

/// The pane a core names by the holder id `holder`, [`holder_pane`] read
/// back: a record's file name is all a node finds of a holder when it
/// starts, and the core's pane ids hold no `_`.
pub fn core_pane(holder: &str) -> String {
    holder.replace('_', ":")
}

/// What a pane is started with.
#[derive(Clone, Debug)]
pub struct StartRequest {
    /// The pane's id, which is the start's key: the same id asked again
    /// with the same command converges on the same pane.
    pub pane: String,
    pub program: Vec<OsString>,
    pub cwd: PathBuf,
    /// Variables added to the node's environment for the pane's program.
    pub env: Vec<(OsString, OsString)>,
    pub size: Size,
    /// The colors the node draws the pane with, which the holder answers
    /// the program's color questions with from its first one. Not part of
    /// the start's key: the same pane asked again with another theme is the
    /// same pane, and its theme changes by a theme message.
    pub theme: Theme,
}

/// A start that has a pane to show for it.
#[derive(Clone, Debug)]
pub enum Started {
    /// This call started the holder.
    Started(PaneRecord),
    /// The pane was already started for this id and command (it may still
    /// be starting, or have ended since); no second holder was started.
    Existing(PaneRecord),
}

/// Why a start was refused. Each says what the caller can act on.
#[derive(Debug)]
pub enum StartRefusal {
    InvalidPane(String),
    /// [`HOLDER_CAP`] holders are recorded in this state folder.
    Cap {
        count: usize,
    },
    /// The pane id is started with another command.
    Conflict(Box<PaneRecord>),
    /// The pane's record exists and cannot be read; it is left as it is.
    RecordUnreadable(Unreadable),
    /// The pane's record belongs to another state folder.
    OtherOwner(Box<PaneRecord>),
    /// The holder could not be started, or said it failed.
    Failed(String),
    /// A record replacement or start may have happened; reconcile this
    /// exact request before trying another intent.
    Uncertain(String),
    /// An earlier start of the pane is over but these processes of it still
    /// run, so the pane's id is not free yet.
    EarlierRuns(Vec<u32>),
}

impl std::fmt::Display for StartRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPane(why) => write!(f, "{why}"),
            Self::Cap { count } => write!(
                f,
                "{count} holders are recorded in this state folder, the most it keeps ({HOLDER_CAP})"
            ),
            Self::Conflict(record) => write!(
                f,
                "pane {} is already started with another command",
                record.pane
            ),
            Self::RecordUnreadable(why) => write!(f, "{why}"),
            Self::OtherOwner(record) => write!(
                f,
                "pane {}'s record belongs to another state folder",
                record.pane
            ),
            Self::Failed(why) => write!(f, "the holder did not start: {why}"),
            Self::Uncertain(why) => write!(f, "the pane's outcome is unknown: {why}"),
            Self::EarlierRuns(left) => write!(
                f,
                "an earlier start of the pane is over and {left:?} of it still run"
            ),
        }
    }
}

/// What a reconcile found of one record.
#[derive(Debug)]
pub enum Judgment {
    /// Its holder runs: adopt it.
    Alive(PaneRecord),
    /// Reserved, with no holder written in yet, within [`START_LIMIT`].
    Starting(PaneRecord),
    /// A former owner's accepted intent, with no holder claim. Recovery
    /// replaces its incarnation before starting a shell under the same ID.
    Recoverable(PaneRecord),
    /// The pane ended, now or before; the record says why.
    Ended(PaneRecord),
    /// The holder speaks a version older than this build speaks to; it is
    /// left running and its version folder kept.
    Unsupported(PaneRecord),
    /// The record belongs to another state folder; nothing was done.
    OtherOwner(PaneRecord),
    /// The record cannot be read; nothing was done.
    Unreadable(String),
    /// Its holder is gone and part of what it left could not be ended; the
    /// record stays `closing` for the next reconcile.
    LeftoversLeft(PaneRecord),
    /// What was found could not be written: the record changed meanwhile or
    /// the write failed. The next reconcile judges it again.
    Unsettled(String),
    /// Its holder was asked to end and still runs, or could not be asked;
    /// the record stays `closing` and the next reconcile asks again.
    Closing(PaneRecord, String),
    /// Not judged within the reconcile's budget; the next reconcile judges
    /// it.
    Pending(PaneRecord),
}

/// What a reconcile found and did.
#[derive(Debug, Default)]
pub struct Reconciled {
    pub panes: BTreeMap<String, Judgment>,
    /// Records that could not be listed at all.
    pub failed: Option<String>,
}

impl Reconciled {
    /// The panes left for the next reconcile.
    pub fn pending(&self) -> usize {
        self.panes
            .values()
            .filter(|judgment| matches!(judgment, Judgment::Pending(_)))
            .count()
    }

    /// Whether everything found was taken up, so no reconcile need follow.
    pub fn is_complete(&self) -> bool {
        self.failed.is_none() && self.pending() == 0
    }
}

/// Why a close could not be asked.
#[derive(Debug)]
pub enum CloseError {
    NoRecord,
    /// The pane's start is still under way, with no holder written in yet;
    /// it was told to stop, and the start answers how it ended.
    Starting,
    RecordUnreadable(Unreadable),
    OtherOwner,
    Failed(String),
}

impl std::fmt::Display for CloseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRecord => write!(f, "the pane has no record"),
            Self::Starting => write!(f, "the pane is still starting; its start was told to stop"),
            Self::RecordUnreadable(why) => write!(f, "{why}"),
            Self::OtherOwner => write!(f, "the pane's record belongs to another state folder"),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// Why an ended pane could not be forgotten.
#[derive(Debug)]
pub enum ForgetRefusal {
    NotEnded(Phase),
    HolderRuns(Identity),
    /// The holder is gone but processes of its pane still run.
    ProcessesRun(Vec<u32>),
    RecordUnreadable(Unreadable),
    Failed(String),
}

impl std::fmt::Display for ForgetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEnded(phase) => write!(f, "the pane is {phase:?}, not ended"),
            Self::HolderRuns(holder) => write!(f, "its holder {} still runs", holder.pid),
            Self::ProcessesRun(pids) => write!(f, "processes of the pane still run: {pids:?}"),
            Self::RecordUnreadable(why) => write!(f, "{why}"),
            Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// A state folder's panes.
pub struct Panes {
    home: PathBuf,
    state_dir: PathBuf,
    owner: String,
    boot: Option<String>,
    sink: Sink,
    /// The version folder names this process took (`holders::prepare`).
    names: holders::VersionNames,
    /// Incarnations this process took from the folder's file and has not
    /// given yet ([`Panes::next_incarnation`]).
    incarnations: Mutex<std::ops::Range<u64>>,
}

impl Panes {
    /// The panes of `state_dir`, whose holders run from version folders
    /// under `home`. Makes the records' folder.
    pub fn open(home: &Path, state_dir: &Path, sink: Sink) -> Result<Self, String> {
        let records = layout::pane_records(state_dir);
        hide_platform::fs::private::create_dir_all(&records)
            .map_err(|error| format!("{} could not be made: {error}", records.display()))?;
        let owner = owner_of(state_dir)?;
        let boot = match hide_platform::process::boot_id() {
            Ok(boot) => Some(boot),
            Err(error) if error.kind() == std::io::ErrorKind::Unsupported => None,
            Err(error) => return Err(format!("this boot could not be named: {error}")),
        };
        let panes = Self {
            home: home.to_owned(),
            state_dir: state_dir.to_owned(),
            owner,
            boot,
            sink,
            names: holders::VersionNames::default(),
            incarnations: Mutex::new(0..0),
        };
        Ok(panes)
    }

    /// Mends what a machine that stopped mid-write leaves, before anything
    /// reads the records. A record and the incarnation counter are replaced
    /// whole (`hide_platform::fs::atomic::write_file`) and flushed to the
    /// device but not past its cache, since a pane record need not outlive
    /// the machine: after a power loss every holder it names is gone too.
    /// What the device then kept may be an empty or cut-off file, or an
    /// older one, and none of it may stop the panes starting again:
    ///
    /// - a record that cannot be parsed and was last written before this
    ///   boot began names nothing that runs (no process outlives its boot),
    ///   so it is removed (`pane_holder.record_removed`) and its pane
    ///   starts again as one with no record; one written in this boot, or of
    ///   a format this build does not know (a newer build's), is left as it
    ///   is and reported, as before, since its holder may run;
    /// - an older readable record is one of an earlier boot, which the
    ///   reconcile ends `machine_restarted` without touching what it names;
    /// - a counter that cannot be read, or that is behind an incarnation a
    ///   record holds, is moved past every one of them
    ///   (`pane_holder.incarnations_mended`), so no start takes a number a
    ///   record still names.
    fn mend_after_a_crash(&self) -> Result<(), String> {
        let booted = match hide_platform::process::boot_time() {
            Ok(booted) => booted,
            Err(error) => {
                // Without the boot's start nothing is known to be old:
                // every record is left as it is, as before.
                self.emit(
                    "pane_holder.mend_skipped",
                    json!({ "reason": format!("this boot's start could not be read: {error}") }),
                );
                return Ok(());
            }
        };
        let _lock = self.lock()?;
        let records = layout::pane_records(&self.state_dir);
        let entries = std::fs::read_dir(&records)
            .map_err(|error| format!("{} could not be listed: {error}", records.display()))?;
        let mut highest = 0_u64;
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("{} could not be listed: {error}", records.display()))?;
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            match record::read(&path) {
                Ok(Some(found)) => highest = highest.max(found.incarnation),
                Ok(None) => {}
                Err(Unreadable::Corrupt(why)) if written_before(&path, booted) => {
                    std::fs::remove_file(&path).map_err(|error| {
                        format!("{} could not be removed: {error}", path.display())
                    })?;
                    self.emit(
                        "pane_holder.record_removed",
                        json!({
                            "pane": path.file_stem().map(|stem| stem.to_string_lossy().into_owned()),
                            "reason": "written before this boot and cannot be parsed",
                            "detail": why,
                        }),
                    );
                }
                Err(_) => {}
            }
        }
        let counter = layout::pane_incarnations(&self.state_dir);
        let next = match std::fs::read_to_string(&counter) {
            Ok(text) => text.trim().parse::<u64>().ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(1),
            Err(error) => {
                return Err(format!("{} could not be read: {error}", counter.display()));
            }
        };
        let floor = highest.saturating_add(1);
        if next.is_none_or(|next| next < floor) {
            if next.is_none() && !written_before(&counter, booted) {
                // Damaged in this boot: not a crash's, so not ours to mend.
                return Ok(());
            }
            hide_platform::fs::atomic::write_file(
                &counter,
                floor.to_string().as_bytes(),
                hide_platform::fs::atomic::WriteOptions::UntilReboot(
                    hide_platform::fs::Access::Private,
                ),
            )
            .map_err(|error| format!("{} could not be written: {error}", counter.display()))?;
            self.emit(
                "pane_holder.incarnations_mended",
                json!({ "found": next, "next": floor }),
            );
        }
        Ok(())
    }

    /// The identity this state folder's records name as their owner.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The pane's record as it is now.
    pub fn record(&self, pane: &str) -> Result<Option<PaneRecord>, Unreadable> {
        record::read(&layout::pane_record(&self.state_dir, pane))
    }

    fn emit(&self, event: &str, fields: serde_json::Value) {
        let mut value = json!({ "event": event, "state_dir": self.state_dir });
        if let (Some(target), Some(fields)) = (value.as_object_mut(), fields.as_object()) {
            target.extend(fields.clone());
        }
        (self.sink)(value);
    }

    fn lock(&self) -> Result<hide_platform::fs::lock::Lock, String> {
        let path = layout::pane_records_lock(&self.state_dir);
        record::lock(&path, record::LOCK_WAIT)
            .map_err(|error| format!("the records' lock could not be taken: {error}"))?
            .ok_or_else(|| {
                format!(
                    "the records' lock was held for {} s",
                    record::LOCK_WAIT.as_secs()
                )
            })
    }

    /// Starts `request`'s pane from `source`'s version folder, or answers
    /// the pane its id already started. The pane id is the start's key: a
    /// live legacy pane is adopted; a new pane follows reserve and exact
    /// claim. An ended reservation needs an explicit replacement, so a late
    /// original request cannot restart a cancelled pane.
    pub fn start(
        &self,
        owner: &Owner,
        request: &StartRequest,
        source: &holders::Source,
    ) -> Result<Started, StartRefusal> {
        self.check_owner(owner).map_err(StartRefusal::Failed)?;
        let digest = spec_of(request);
        let cwd = hide_platform::path::to_wire(&request.cwd)
            .map_err(|error| StartRefusal::Failed(error.to_string()))?;
        // Compatibility for records written before existence reservations.
        // New starts, including this synchronous API, use the one claim path.
        let mut settled = false;
        let previous = loop {
            let lock = self.lock().map_err(StartRefusal::Failed)?;
            let Some(existing) = self
                .record(&request.pane)
                .map_err(StartRefusal::RecordUnreadable)?
            else {
                break None;
            };
            if existing.owner != self.owner {
                return Err(StartRefusal::OtherOwner(Box::new(existing)));
            }
            if existing.reservation.is_some() {
                break None;
            }
            match self.state_of(&existing) {
                Earlier::Lives if existing.spec == digest => {
                    return Ok(Started::Existing(existing));
                }
                Earlier::Lives => return Err(StartRefusal::Conflict(Box::new(existing))),
                Earlier::Unsettled if !settled => {
                    drop(lock);
                    settled = true;
                    self.settle_earlier(owner, &existing);
                }
                Earlier::Over | Earlier::Unsettled => break Some(existing.incarnation),
            }
        };
        // A concurrent synchronous caller joins the original numbered
        // request; after it retires, a running record answers without any
        // new start. Product callers carry their number over the node link.
        let existing = self
            .record(&request.pane)
            .map_err(StartRefusal::RecordUnreadable)?;
        if let Some(record) = &existing
            && record.phase == Phase::Running
            && record.spec == digest
        {
            return Ok(Started::Existing(record.clone()));
        }
        if let Some(record) = &existing
            && record.phase == Phase::Ended
            && record.reservation.is_some()
        {
            return Err(StartRefusal::Conflict(Box::new(record.clone())));
        }
        let number = existing
            .as_ref()
            .and_then(|record| record.reservation.as_ref())
            .filter(|intent| intent.owner == owner.incarnation() && intent.request == digest)
            .map(|intent| intent.number);
        let admitted = match number {
            Some(number) if number != 0 => {
                owner.admit(owner.incarnation(), number, &request.pane, &digest)
            }
            _ => owner.issue(&request.pane, &digest),
        };
        let guard = match admitted {
            Ok(guard) => guard,
            Err(why) if number.is_some() => {
                // The concurrent original may have completed and retired
                // after our Preparing read. Adoption performs no mutation.
                let current = self
                    .record(&request.pane)
                    .map_err(StartRefusal::RecordUnreadable)?;
                if let Some(current) = current
                    && current.phase == Phase::Running
                    && current.spec == digest
                {
                    return Ok(Started::Existing(current));
                }
                return Err(StartRefusal::Uncertain(why));
            }
            Err(why) => return Err(StartRefusal::Failed(why)),
        };
        let reserved = match self.reserve(&guard, &request.pane, &digest, &cwd, previous) {
            Ok(reserved) => reserved,
            Err(StartRefusal::Conflict(record))
                if record.phase != Phase::Ended
                    && record.reservation.as_ref().is_some_and(|intent| {
                        intent.owner == owner.incarnation() && intent.request == digest
                    }) =>
            {
                // Another synchronous caller admitted this intent first.
                // This number made no storage change and has no later work.
                owner
                    .retire(owner.incarnation(), guard.number())
                    .map_err(StartRefusal::Failed)?;
                return if record.phase == Phase::Running {
                    Ok(Started::Existing(*record))
                } else {
                    Err(StartRefusal::Uncertain(
                        "the original holder is still starting".to_owned(),
                    ))
                };
            }
            Err(error) => {
                if !matches!(error, StartRefusal::Uncertain(_)) {
                    owner
                        .retire(owner.incarnation(), guard.number())
                        .map_err(StartRefusal::Failed)?;
                }
                return Err(error);
            }
        };
        let result = self.start_reserved(
            &guard,
            &request.pane,
            reserved.incarnation,
            &digest,
            source,
            || Ok(request.clone()),
        );
        if !matches!(result, Err(StartRefusal::Uncertain(_))) {
            owner
                .retire(owner.incarnation(), guard.number())
                .map_err(StartRefusal::Failed)?;
        }
        result
    }

    /// Settle a legacy start before explicitly replacing its incarnation.
    fn settle_earlier(&self, owner: &Owner, existing: &PaneRecord) {
        match existing.holder {
            Some(holder) if existing.phase == Phase::Ended && holder.runs() => {
                self.end_unconfirmed_start(existing);
                wait_gone(holder, START_LIMIT);
            }
            _ => {
                let _ = self.judge(owner, existing);
            }
        }
    }

    /// Whether `record` was written in another boot of the machine, so
    /// nothing it names runs in this one. A record or a folder whose boot is
    /// not known is not of another boot.
    fn of_another_boot(&self, record: &PaneRecord) -> bool {
        record.boot.is_some() && self.boot.is_some() && record.boot != self.boot
    }

    /// Whether an earlier start of a pane still holds its id. Read under the
    /// lock, so it looks only at the record and the processes it names.
    fn state_of(&self, existing: &PaneRecord) -> Earlier {
        let runs = existing.holder.is_some_and(|holder| holder.runs());
        let other_boot = self.of_another_boot(existing);
        match existing.phase {
            Phase::Reserved => Earlier::Lives,
            Phase::Ended if self.still_running(existing).is_empty() => Earlier::Over,
            Phase::Ended => Earlier::Unsettled,
            _ if other_boot => Earlier::Unsettled,
            // Ended first, then started again as a new incarnation.
            Phase::Closing => Earlier::Unsettled,
            _ if runs => Earlier::Lives,
            _ if existing.holder.is_some() => Earlier::Unsettled,
            // Reserved with no holder written in yet: a start under way
            // within its bound.
            _ => {
                let age = record::now_unix_ms().saturating_sub(existing.reserved_at_unix_ms);
                if existing.phase == Phase::Preparing && Duration::from_millis(age) < START_LIMIT {
                    Earlier::Lives
                } else {
                    Earlier::Unsettled
                }
            }
        }
    }

    /// What of an earlier start still runs: its holder by pid and start
    /// time, and on Unix the members of its program's session, found by the
    /// program's recorded pid and start time; elsewhere, or when the session
    /// cannot be read, the pids its end says were left that are still alive.
    /// Nothing of another boot runs in this one.
    fn still_running(&self, existing: &PaneRecord) -> Vec<u32> {
        if self.of_another_boot(existing) {
            return Vec::new();
        }
        let left = || {
            existing
                .end
                .iter()
                .flat_map(|end| end.left.iter().copied())
                .filter(|pid| hide_platform::process::is_alive(*pid))
                .collect::<Vec<u32>>()
        };
        let mut running: Vec<u32> = existing
            .holder
            .filter(|holder| holder.runs())
            .map(|holder| holder.pid)
            .into_iter()
            .collect();
        #[cfg(unix)]
        let members = existing.program.map(|program| {
            hide_platform::process::Session::left_by(program.pid, program.started)
                .and_then(|session| session.members())
        });
        #[cfg(windows)]
        let members: Option<std::io::Result<Vec<u32>>> = None;
        match members {
            Some(Ok(members)) => running.extend(members),
            Some(Err(_)) | None => running.extend(left()),
        }
        running.sort_unstable();
        running.dedup();
        running
    }

    fn start_holder(
        &self,
        request: &StartRequest,
        reservation: &holders::Reservation,
        reserved: &PaneRecord,
        stages: &StartStages,
    ) -> Result<PaneRecord, String> {
        let pane = &request.pane;
        let mut command = ResidentCommand::new(&reservation.program);
        command
            .current_dir(&reservation.dir)
            .arg("--socket")
            .arg(layout::holder_socket(&self.state_dir, pane))
            .arg("--rows")
            .arg(request.size.rows.to_string())
            .arg("--cols")
            .arg(request.size.cols.to_string())
            .arg("--cwd")
            .arg(&request.cwd)
            .arg("--log")
            .arg(layout::holder_log(&self.state_dir, pane))
            .arg("--id")
            .arg(pane)
            .arg("--record")
            .arg(layout::pane_record(&self.state_dir, pane))
            .arg("--record-lock")
            .arg(layout::pane_records_lock(&self.state_dir))
            .arg("--incarnation")
            .arg(reserved.incarnation.to_string())
            .arg("--theme")
            .arg(request.theme.to_arg())
            .arg("--")
            .args(&request.program);
        for (key, value) in &request.env {
            command.env(key, value);
        }
        let (reader, writer) =
            std::io::pipe().map_err(|error| format!("the report pipe: {error}"))?;
        #[cfg(unix)]
        let lent = [std::os::fd::AsFd::as_fd(&writer)];
        #[cfg(windows)]
        let lent = [std::os::windows::io::AsHandle::as_handle(&writer)];
        let resident = match start_resident(&command, &lent) {
            Ok(resident) => resident,
            Err(error) => {
                drop(writer);
                let reason = format!("{} could not start: {error}", reservation.program.display());
                return self.settle_failed_start(reserved, None, reason);
            }
        };
        drop(writer);
        let spawned = stages.begun.elapsed();
        let holder = Identity::of(resident.pid);
        if resident.outer_job {
            self.emit(
                "pane_holder.outer_job",
                json!({ "pane": pane, "incarnation": reserved.incarnation, "holder": resident.pid }),
            );
        }
        match first_line(reader, START_LIMIT) {
            Report::Line(line) if line.starts_with("ready ") => {
                let ready = stages.begun.elapsed();
                // Records are replaced atomically, as `record()` reads
                // them without the writer lock. This read changes nothing
                // and checks the reserved incarnation below; taking every
                // holder's writer lock here held a completed start behind
                // unrelated starts before its worker could take another.
                match record::read(&layout::pane_record(&self.state_dir, pane)) {
                    // A program that exits at once may have ended the pane
                    // by now; the start still took place.
                    Ok(Some(written))
                        if written.incarnation == reserved.incarnation
                            && written.phase != Phase::Preparing =>
                    {
                        // `job` names the Windows job above the node's that
                        // keeps the holder, if any (`ready ... job=<limits|none>`).
                        let job = line
                            .split_whitespace()
                            .find_map(|word| word.strip_prefix("job="));
                        self.emit(
                            "pane_holder.started",
                            json!({
                                "pane": pane,
                                "incarnation": written.incarnation,
                                "holder": resident.pid,
                                "job": job,
                                "prepared_ms": millis(stages.prepared),
                                "reserved_ms": millis(stages.reserved),
                                "spawned_ms": millis(spawned),
                                "ready_ms": millis(ready),
                                "took_ms": millis(stages.begun.elapsed()),
                            }),
                        );
                        Ok(written)
                    }
                    other => Err(format!(
                        "the holder said it was ready but the record reads {other:?}"
                    )),
                }
            }
            Report::Line(line) => {
                let reason = line.strip_prefix("failed ").unwrap_or(&line).to_owned();
                if let Some(holder) = holder {
                    wait_gone(holder, START_LIMIT);
                }
                self.settle_failed_start(reserved, holder, reason)
            }
            Report::Closed => {
                if let Some(holder) = holder {
                    wait_gone(holder, START_LIMIT);
                }
                let reason = "the holder ended before it said it was ready or why".to_owned();
                self.settle_failed_start(reserved, holder, reason)
            }
            Report::Unreadable(error) => {
                let reason = format!("the holder's report could not be read: {error}");
                self.settle_failed_start(reserved, holder, reason)
            }
            Report::Silent => {
                let reason = format!("the holder said nothing within {} s", START_LIMIT.as_secs());
                self.settle_failed_start(reserved, holder, reason)
            }
        }
    }

    /// After a start that did not say it was ready: a holder that wrote
    /// itself running meanwhile (a late `ready`) is taken as started; a
    /// reservation whose holder is gone, or never ran, is removed, so the same
    /// start may be asked again; one whose holder still runs is ended
    /// `start_unconfirmed` under the lock and its holder ended whole outside
    /// it.
    fn settle_failed_start(
        &self,
        reserved: &PaneRecord,
        holder: Option<Identity>,
        reason: String,
    ) -> Result<PaneRecord, String> {
        let path = layout::pane_record(&self.state_dir, &reserved.pane);
        let decided = (|| -> Result<FailedStart, String> {
            let _lock = self.lock()?;
            let mut current = match record::read(&path) {
                Ok(Some(current)) => current,
                Ok(None) => return Ok(FailedStart::Gone),
                Err(error) => return Err(error.to_string()),
            };
            if current.incarnation != reserved.incarnation {
                return Ok(FailedStart::MovedOn);
            }
            let written_in = current
                .holder
                .is_some_and(|named| holder.is_none_or(|holder| holder == named));
            let took_place = match current.phase {
                Phase::Reserved | Phase::Preparing => false,
                Phase::Running | Phase::Closing => true,
                Phase::Ended => !matches!(
                    current.end.as_ref().map(|end| end.reason),
                    Some(EndReason::StartFailed | EndReason::StartUnconfirmed)
                ),
            };
            if written_in && took_place {
                return Ok(FailedStart::Started(Box::new(current)));
            }
            if current.phase == Phase::Ended {
                return Ok(FailedStart::MovedOn);
            }
            let holder = current.holder.or(holder);
            match holder.filter(|holder| holder.runs()) {
                None => {
                    if current.reservation.is_some() {
                        current.phase = Phase::Ended;
                        current.end = Some(End::new(EndReason::StartFailed, reason.as_str()));
                        record::write(&path, &current).map_err(|error| error.to_string())?;
                        return Ok(FailedStart::Recorded);
                    }
                    std::fs::remove_file(&path).map_err(|error| error.to_string())?;
                    Ok(FailedStart::Removed)
                }
                Some(holder) => {
                    current.holder = Some(holder);
                    current.phase = Phase::Ended;
                    current.end = Some(End::new(EndReason::StartUnconfirmed, reason.as_str()));
                    record::write(&path, &current).map_err(|error| error.to_string())?;
                    Ok(FailedStart::Unconfirmed(holder))
                }
            }
        })();
        let left = match &decided {
            // Ended outside the lock: the record no longer reserves its start,
            // so nothing writes it in meanwhile.
            Ok(FailedStart::Unconfirmed(holder)) => end_holder_tree(*holder).err(),
            // No record names the pane now, so its socket goes with the
            // reservation.
            Ok(FailedStart::Removed | FailedStart::Recorded) => {
                self.remove_stale_socket(&reserved.pane, LOCK_RELEASE);
                None
            }
            _ => None,
        };
        self.emit(
            "pane_holder.start_failed",
            json!({
                "pane": reserved.pane,
                "incarnation": reserved.incarnation,
                "holder": holder.map(|holder| holder.pid),
                "reason": reason,
                "record": decided.as_ref().map_or_else(|error| error.clone(), |done| done.name().to_owned()),
                "left": left,
            }),
        );
        match decided {
            Ok(FailedStart::Started(record)) => Ok(*record),
            _ => Err(reason),
        }
    }

    /// Asks the pane to end and waits until it has, within [`CLOSE_LIMIT`],
    /// and answers its record. A pane that already ended answers its record
    /// at once; one whose holder is gone is settled as a reconcile settles it.
    pub fn close(&self, owner: &Owner, pane: &str) -> Result<PaneRecord, CloseError> {
        self.check_owner(owner).map_err(CloseError::Failed)?;
        let path = layout::pane_record(&self.state_dir, pane);
        let current = {
            let _lock = self.lock().map_err(CloseError::Failed)?;
            let mut current = match record::read(&path) {
                Ok(Some(current)) => current,
                Ok(None) => return Err(CloseError::NoRecord),
                Err(error) => return Err(CloseError::RecordUnreadable(error)),
            };
            if current.owner != self.owner {
                return Err(CloseError::OtherOwner);
            }
            self.prepare_close(&mut current);
            record::write(&path, &current)
                .map_err(|error| CloseError::Failed(error.to_string()))?;
            current
        };
        self.finish_close(owner, current)
    }

    // Called under the records lock, including by exact-intent cancellation.
    // No gap separates matching the key from committing its close decision.
    fn prepare_close(&self, current: &mut PaneRecord) {
        if let Some(intent) = &mut current.reservation {
            intent.cancelled = true;
        }
        if current.reservation.is_some() && current.holder.is_none() && current.program.is_none() {
            current.finish(End::new(
                EndReason::Terminated,
                "the pane was closed before its holder claimed it",
            ));
        } else {
            current.phase = Phase::Closing;
        }
    }

    fn finish_close(&self, owner: &Owner, current: PaneRecord) -> Result<PaneRecord, CloseError> {
        if current.phase == Phase::Ended {
            return Ok(current);
        }
        let pane = &current.pane;
        self.emit(
            "pane_holder.closing",
            json!({ "pane": pane, "incarnation": current.incarnation, "holder": current.holder.map(|h| h.pid) }),
        );
        // Use the same boot fence as recovery before any process operation.
        let judgment = self.judge(owner, &current);
        match judgment {
            Judgment::Ended(record) if record.incarnation == current.incarnation => Ok(record),
            Judgment::LeftoversLeft(record) => Err(CloseError::Failed(format!(
                "its holder is gone and left {:?} running",
                record.end.map(|end| end.left).unwrap_or_default()
            ))),
            Judgment::Closing(_, why) | Judgment::Unsettled(why) | Judgment::Unreadable(why) => {
                Err(CloseError::Failed(why))
            }
            Judgment::Starting(_) => Err(CloseError::Starting),
            other => Err(CloseError::Failed(format!(
                "the pane did not end; it was judged {other:?}"
            ))),
        }
    }

    /// Removes an ended pane's record, its log and any socket it left. A
    /// pane that has not ended, or whose holder still runs, is refused.
    pub fn forget(&self, owner: &Owner, pane: &str) -> Result<(), ForgetRefusal> {
        self.check_owner(owner).map_err(ForgetRefusal::Failed)?;
        let path = layout::pane_record(&self.state_dir, pane);
        let _lock = self.lock().map_err(ForgetRefusal::Failed)?;
        let current = match record::read(&path) {
            Ok(Some(current)) => current,
            Ok(None) => return Ok(()),
            Err(error) => return Err(ForgetRefusal::RecordUnreadable(error)),
        };
        if current.phase != Phase::Ended {
            return Err(ForgetRefusal::NotEnded(current.phase));
        }
        if current.reservation.as_ref().is_some_and(|intent| {
            intent.owner == owner.incarnation() && owner.request_pending(intent.number)
        }) {
            return Err(ForgetRefusal::Failed(
                "the current owner can still send this reservation".to_owned(),
            ));
        }
        if let Some(holder) = current.holder
            && holder.runs()
        {
            return Err(ForgetRefusal::HolderRuns(holder));
        }
        // A record goes only once nothing it names runs: a replacement that
        // finds its record gone starts afresh (`reserve`).
        let left = self.still_running(&current);
        if !left.is_empty() {
            return Err(ForgetRefusal::ProcessesRun(left));
        }
        self.remove_pane_files(pane);
        std::fs::remove_file(&path).map_err(|error| ForgetRefusal::Failed(error.to_string()))
    }

    fn remove_pane_files(&self, pane: &str) {
        let socket = layout::holder_socket(&self.state_dir, pane);
        let _ = hide_platform::ipc::remove_leftover(&socket, LOCK_RELEASE);
        let log = layout::holder_log(&self.state_dir, pane);
        let _ = std::fs::remove_file(&log);
        let mut aside = log.into_os_string();
        aside.push(".1");
        let _ = std::fs::remove_file(PathBuf::from(aside));
    }

    /// How many panes may still run: the records not ended and the ones that
    /// cannot be read, counted under the lock as a start counts them against
    /// [`HOLDER_CAP`]. A daemon of the core's own engine lives while this is
    /// above zero (PRD engine-bundle-3-core-owns D-10).
    pub fn live(&self) -> Result<usize, String> {
        let _lock = self.lock()?;
        // A liveness query does not own the connection and must not prune
        // records while a previous connection's workers are draining.
        Ok(self
            .scan()?
            .iter()
            .filter(|(_, read)| {
                !read
                    .as_ref()
                    .is_ok_and(|record| record.phase == Phase::Ended)
            })
            .count())
    }

    /// The records not ended, and the ones that cannot be read, which may
    /// name a holder. Ended records past [`ENDED_KEPT`] go, oldest first.
    fn count_live_for(&self, owner: &Owner, scan: &[(String, RecordRead)]) -> usize {
        let mut live = 0;
        let mut ended = Vec::new();
        for (pane, read) in scan {
            match read {
                Ok(record)
                    if record.phase == Phase::Ended
                        && !record.reservation.as_ref().is_some_and(|intent| {
                            intent.owner == owner.incarnation()
                                && owner.request_pending(intent.number)
                        }) =>
                {
                    ended.push((
                        record.end.as_ref().map_or(0, |end| end.at_unix_ms),
                        pane.clone(),
                        record,
                    ))
                }
                _ => live += 1,
            }
        }
        if ended.len() > ENDED_KEPT {
            ended.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
            for (_, pane, record) in &ended[..ended.len() - ENDED_KEPT] {
                // A holder still exiting keeps its record until it is gone.
                if let Some(holder) = record.holder.filter(|holder| holder.runs()) {
                    self.emit(
                        "pane_holder.ended_kept",
                        json!({ "pane": pane, "holder": holder.pid, "reason": "holder_runs" }),
                    );
                    continue;
                }
                // So does one whose pane left processes running: a record goes
                // only once nothing it names runs, which lets a replacement
                // that finds its record gone start afresh (`reserve`).
                let left = self.still_running(record);
                if !left.is_empty() {
                    self.emit(
                        "pane_holder.ended_kept",
                        json!({ "pane": pane, "left": left, "reason": "processes_run" }),
                    );
                    continue;
                }
                self.remove_pane_files(pane);
                let _ = std::fs::remove_file(layout::pane_record(&self.state_dir, pane));
                self.emit(
                    "pane_holder.ended_dropped",
                    json!({ "pane": pane, "kept": ENDED_KEPT }),
                );
            }
        }
        live
    }

    /// Every record file, by pane, read. The caller holds the lock.
    fn scan(&self) -> Result<Vec<(String, RecordRead)>, String> {
        Ok(Self::read_records(self.record_paths()?))
    }

    fn read_records(paths: Vec<(String, PathBuf)>) -> Vec<(String, RecordRead)> {
        paths
            .into_iter()
            .map(|(pane, path)| {
                let read = record::read(&path).and_then(|found| {
                    found.ok_or(Unreadable::Corrupt("removed while read".into()))
                });
                (pane, read)
            })
            .collect()
    }

    /// Lists under the records lock, with the same bound for an admission
    /// and a full scan. Counting names alone never understates live panes.
    fn record_paths(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let dir = layout::pane_records(&self.state_dir);
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| format!("{} could not be listed: {error}", dir.display()))?;
        let mut scan = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(pane) = name.strip_suffix(".json") else {
                continue;
            };
            if scan.len() >= MAX_RECORD_FILES {
                return Err(format!(
                    "{} holds more than {MAX_RECORD_FILES} records",
                    dir.display()
                ));
            }
            scan.push((pane.to_owned(), entry.path()));
        }
        scan.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(scan)
    }

    /// The next incarnation, kept in a file of its own so a pane forgotten
    /// and started again never has an earlier start's number. The caller
    /// holds the lock. The file is moved on by [`INCARNATIONS_TAKEN`] at a
    /// time and this process gives those out itself, so one start in that
    /// many writes it rather than every start: each write is flushed, and
    /// it stood in front of every new pane. A number is never given twice,
    /// whichever process takes the next ones, and one taken but not given
    /// (the process ended) is simply never used.
    fn next_incarnation(&self) -> Result<u64, String> {
        let mut taken = self
            .incarnations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if taken.is_empty() {
            let path = layout::pane_incarnations(&self.state_dir);
            let next = match std::fs::read_to_string(&path) {
                Ok(text) => text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| format!("{} does not hold a number", path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 1,
                Err(error) => {
                    return Err(format!("{} could not be read: {error}", path.display()));
                }
            };
            let end = next
                .checked_add(INCARNATIONS_TAKEN)
                .ok_or_else(|| format!("{} holds no next number", path.display()))?;
            hide_platform::fs::atomic::write_file(
                &path,
                end.to_string().as_bytes(),
                hide_platform::fs::atomic::WriteOptions::UntilReboot(
                    hide_platform::fs::Access::Private,
                ),
            )
            .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
            *taken = next..end;
        }
        taken
            .next()
            .ok_or_else(|| "no incarnation was taken".to_owned())
    }

    /// Reads every record, adopts the holders they name and settles what
    /// lost holders left. See the module documentation.
    ///
    /// Panes are judged by up to [`RECONCILE_WORKERS`] at once, and none is
    /// taken up after `budget` has passed: a judgment already under way runs
    /// to its own bound, and what was not taken up is answered `Pending` for
    /// the next reconcile, which the
    /// caller asks for while [`Reconciled::is_complete`] says no.
    pub fn reconcile(&self, owner: &Owner, budget: Duration) -> Reconciled {
        if let Err(why) = self.check_owner(owner) {
            return Reconciled {
                failed: Some(why),
                ..Reconciled::default()
            };
        }
        let started = Instant::now();
        let deadline = started + budget;
        let mut found = Reconciled::default();
        let scan = match self.lock().and_then(|_lock| self.scan()) {
            Ok(scan) => scan,
            Err(why) => {
                self.emit("pane_holder.reconcile_failed", json!({ "error": why }));
                found.failed = Some(why);
                return found;
            }
        };
        let mut readable = Vec::new();
        for (pane, read) in scan {
            match read {
                Ok(current) => readable.push(current),
                Err(why) => {
                    self.emit(
                        "pane_holder.record_unreadable",
                        json!({ "pane": pane, "reason": why.name(), "error": why.to_string() }),
                    );
                    found
                        .panes
                        .insert(pane, Judgment::Unreadable(why.to_string()));
                }
            }
        }
        let judged = self.bounded(&readable, deadline, |current| self.judge(owner, current));
        for (current, judgment) in readable.into_iter().zip(judged) {
            let judgment = judgment.unwrap_or_else(|| Judgment::Pending(current.clone()));
            found.panes.insert(current.pane, judgment);
        }
        self.emit(
            "pane_holder.reconciled",
            json!({
                "panes": found.panes.len(),
                "pending": found.pending(),
                "took_ms": started.elapsed().as_millis() as u64,
            }),
        );
        found
    }

    /// Runs `work` on each item on up to [`RECONCILE_WORKERS`] threads,
    /// taking up no item once `deadline` has passed; an item not taken up
    /// answers `None`. Every thread ends before this returns.
    fn bounded<T: Sync, R: Send>(
        &self,
        items: &[T],
        deadline: Instant,
        work: impl Fn(&T) -> R + Sync,
    ) -> Vec<Option<R>> {
        let next = AtomicUsize::new(0);
        let answers: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..RECONCILE_WORKERS.min(items.len()) {
                let started = std::thread::Builder::new()
                    .name("pane-reconcile".to_owned())
                    .spawn_scoped(scope, || {
                        while Instant::now() < deadline {
                            let at = next.fetch_add(1, Ordering::Relaxed);
                            let Some(item) = items.get(at) else {
                                return;
                            };
                            let answer = work(item);
                            *answers[at].lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(answer);
                        }
                    });
                // Its share goes to the other workers, or to the next
                // reconcile.
                if let Err(error) = started {
                    self.emit(
                        "pane_holder.reconcile_worker_failed",
                        json!({ "error": error.to_string() }),
                    );
                }
            }
        });
        answers
            .into_iter()
            .map(|answer| answer.into_inner().unwrap_or_else(PoisonError::into_inner))
            .collect()
    }

    /// Judges one record and does what it calls for.
    fn judge(&self, owner: &Owner, current: &PaneRecord) -> Judgment {
        let pane = current.pane.clone();
        let judgment = if self.of_another_boot(current) {
            // Nothing a record of another boot names runs in this one; its
            // pids and start times may name processes of this boot. With no
            // process left to guard, the record is this folder's to settle
            // whatever owner it names: the owner is the folder's volume and
            // number, and macOS may number a volume anew at each boot, which
            // left every record of the earlier boot unjudged.
            let mine = PaneRecord {
                owner: self.owner.clone(),
                ..current.clone()
            };
            let ended = if current.phase == Phase::Ended && !current.cancelled() {
                // A boot fence forbids process cleanup, not the completed
                // outcome: changing ProgramExited to MachineRestarted would
                // make node recovery start a shell the user already ended.
                if mine == *current {
                    Judgment::Ended(mine)
                } else {
                    match self.write_if_unchanged(current, &mine) {
                        Ok(()) => Judgment::Ended(mine),
                        Err(why) => Judgment::Unsettled(why),
                    }
                }
            } else {
                self.finish_record(
                    current,
                    mine,
                    End::new(
                        EndReason::MachineRestarted,
                        "the record is from an earlier boot of the machine",
                    ),
                )
            };
            // No process of another boot holds a lock in this one.
            self.remove_stale_socket(&pane, Duration::ZERO);
            ended
        } else if current.owner != self.owner {
            Judgment::OtherOwner(current.clone())
        } else if current.protocol < OLDEST_HOLDER_VERSION {
            Judgment::Unsupported(current.clone())
        } else if current.phase == Phase::Ended && current.cancelled() {
            let mut closing = current.clone();
            closing.phase = Phase::Closing;
            match self.write_if_unchanged(current, &closing) {
                Ok(()) => match closing.holder {
                    Some(holder) if holder.runs() => self.end_closing(&pane, holder),
                    _ => self.settle_lost(&closing),
                },
                Err(why) => Judgment::Unsettled(why),
            }
        } else if current.phase == Phase::Ended {
            self.end_unconfirmed_start(current);
            let gone = current.holder.is_none_or(|holder| !holder.runs());
            self.remove_stale_socket(&pane, if gone { LOCK_RELEASE } else { Duration::ZERO });
            Judgment::Ended(current.clone())
        } else if matches!(current.phase, Phase::Reserved | Phase::Preparing)
            && current.holder.is_none()
            && current
                .reservation
                .as_ref()
                .is_some_and(|intent| intent.owner != owner.incarnation())
        {
            Judgment::Recoverable(current.clone())
        } else if current.phase == Phase::Reserved {
            // Accepted existence is not an expired attempt to start a holder.
            // Only explicit cancellation or a fenced recovery consumes it.
            Judgment::Starting(current.clone())
        } else {
            match current.holder {
                // A close a node asked for and did not see through.
                Some(holder) if holder.runs() && current.phase == Phase::Closing => {
                    self.end_closing(&pane, holder)
                }
                Some(holder) if holder.runs() => Judgment::Alive(current.clone()),
                Some(_) => self.settle_lost(current),
                None if current.cancelled() => self.settle_lost(current),
                None => {
                    let age = record::now_unix_ms().saturating_sub(current.reserved_at_unix_ms);
                    if Duration::from_millis(age) < START_LIMIT {
                        Judgment::Starting(current.clone())
                    } else {
                        self.end_record(current, End::new(
                            EndReason::StartInterrupted,
                            "the start was reserved and no holder wrote itself in within its bound",
                        ))
                    }
                }
            }
        };
        if let Judgment::Ended(record) = &judgment
            && current.phase != Phase::Ended
        {
            self.emit(
                "pane_holder.ended",
                json!({
                    "pane": pane,
                    "incarnation": record.incarnation,
                    "holder": record.holder.map(|h| h.pid),
                    "reason": record.end.as_ref().map(|end| end.reason.name()),
                    "left": record.end.as_ref().map(|end| end.left.clone()),
                }),
            );
        }
        judgment
    }

    /// The record of a pane whose holder was asked to end and given its
    /// bound: ended, lost without writing its end, or still closing.
    fn after_close(&self, pane: &str, holder: Identity, refused: Option<String>) -> Judgment {
        match self.record(pane) {
            Ok(Some(after)) if after.phase == Phase::Ended && !after.cancelled() => {
                Judgment::Ended(after)
            }
            Ok(Some(after)) if !holder.runs() => self.settle_lost(&after),
            Ok(Some(after)) => {
                let why = refused.unwrap_or_else(|| {
                    format!(
                        "it was asked to end and still runs after {} s",
                        CLOSE_LIMIT.as_secs()
                    )
                });
                let mut closing = after.clone();
                closing.phase = Phase::Closing;
                let mut end = closing
                    .end
                    .take()
                    .unwrap_or_else(|| End::new(EndReason::Terminated, &why));
                end.detail = why.clone();
                if !end.left.contains(&holder.pid) {
                    end.left.push(holder.pid);
                }
                closing.end = Some(end);
                match self.write_if_unchanged(&after, &closing) {
                    Ok(()) => Judgment::Closing(closing, why),
                    Err(why) => Judgment::Unsettled(why),
                }
            }
            Ok(None) => Judgment::Unsettled("the record was removed while it was settled".into()),
            Err(error) => Judgment::Unreadable(error.to_string()),
        }
    }

    /// A start given up on whose holder still runs, stuck before it read
    /// the record again: the record names it by pid and start time in this
    /// boot, so it is this node's to end.
    fn end_unconfirmed_start(&self, current: &PaneRecord) {
        let unconfirmed = current
            .end
            .as_ref()
            .is_some_and(|end| end.reason == EndReason::StartUnconfirmed);
        let this_boot = current.boot.is_none() || current.boot == self.boot;
        let Some(holder) = current.holder.filter(|holder| holder.runs()) else {
            return;
        };
        if !unconfirmed || !this_boot {
            return;
        }
        let left = end_holder_tree(holder).err().unwrap_or_default();
        self.emit(
            "pane_holder.unconfirmed_start_ended",
            json!({ "pane": current.pane, "incarnation": current.incarnation, "holder": holder.pid, "left": left }),
        );
    }

    /// A record whose holder ended without ending it: `holder_lost`; on Unix what its pane's session left is ended by the
    /// program's recorded identity (on Windows the holder's job ended it).
    fn settle_lost(&self, current: &PaneRecord) -> Judgment {
        // A holder that ended its pane since the record was read wrote why.
        match self.record(&current.pane) {
            Ok(Some(now)) if now.phase == Phase::Ended && !now.cancelled() => {
                return Judgment::Ended(now);
            }
            Ok(Some(now)) if now == *current => {}
            Ok(_) => return Judgment::Unsettled("the record changed while it was settled".into()),
            Err(error) => return Judgment::Unsettled(error.to_string()),
        }
        let mut end = End::new(
            EndReason::HolderLost,
            "the holder ended without saying why (killed or crashed)",
        );
        end_leftovers(current, &mut end);
        self.remove_stale_socket(&current.pane, LOCK_RELEASE);
        if end.left.is_empty() {
            self.end_record(current, end)
        } else {
            self.emit(
                "pane_holder.leftovers_unconfirmed",
                json!({ "pane": current.pane, "incarnation": current.incarnation, "left": end.left }),
            );
            let mut closing = current.clone();
            closing.phase = Phase::Closing;
            closing.end = Some(end);
            match self.write_if_unchanged(current, &closing) {
                Ok(()) => Judgment::LeftoversLeft(closing),
                Err(why) => Judgment::Unsettled(why),
            }
        }
    }

    /// Writes `current` as ended with `end`, if it has not changed since it
    /// was read, and answers its judgment.
    fn end_record(&self, current: &PaneRecord, end: End) -> Judgment {
        self.finish_record(current, current.clone(), end)
    }

    /// [`Self::end_record`] writing `base` ended in place of `current`, as
    /// long as the record is still `current`.
    fn finish_record(&self, current: &PaneRecord, base: PaneRecord, end: End) -> Judgment {
        let mut ended = base;
        ended.finish(end);
        // Every caller established that no holder of this boot remains and
        // completed its leftover cleanup before this exact-record CAS.
        ended.phase = Phase::Ended;
        match self.write_if_unchanged(current, &ended) {
            Ok(()) => Judgment::Ended(ended),
            Err(why) => Judgment::Unsettled(why),
        }
    }

    fn write_if_unchanged(&self, read: &PaneRecord, new: &PaneRecord) -> Result<(), String> {
        let path = layout::pane_record(&self.state_dir, &read.pane);
        let _lock = self.lock()?;
        match record::read(&path) {
            Ok(Some(now)) if now == *read => {
                record::write(&path, new).map_err(|error| error.to_string())
            }
            Ok(_) => Err("the record changed while it was settled".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Removes the pane's socket when no listener holds it, waiting up to
    /// `wait` for the lock of a holder known to have ended.
    fn remove_stale_socket(&self, pane: &str, wait: Duration) {
        let socket = layout::holder_socket(&self.state_dir, pane);
        match hide_platform::ipc::remove_leftover(&socket, wait) {
            Ok(true) => self.emit("pane_holder.stale_socket_removed", json!({ "pane": pane })),
            Ok(false) => {}
            // Kept: a listener still answers or holds its lock, or the
            // system refused the removal; the next reconcile tries again.
            Err(error) => self.emit(
                "pane_holder.stale_socket_kept",
                json!({ "pane": pane, "error": error.to_string() }),
            ),
        }
    }

    /// Asks the holder at the pane's socket to end the pane, once that
    /// holder is `holder` (when the system names a connection's peer), and
    /// waits for its answer.
    fn terminate(&self, pane: &str, holder: Identity) -> Result<String, NotAsked> {
        let socket = layout::holder_socket(&self.state_dir, pane);
        let deadline = Instant::now() + CLOSE_LIMIT;
        loop {
            let (mut stream, _) = match hide_holder_protocol::open(
                &socket,
                Vec::new(),
                (Instant::now() + CONNECT_LIMIT).min(deadline),
            ) {
                Ok(opened) => opened,
                Err(OpenError::Io(error)) if unreachable(&error, &socket) => {
                    return Err(NotAsked::Unreachable(format!(
                        "nothing answers at its socket: {error}"
                    )));
                }
                Err(error) => return Err(NotAsked::Refused(error.to_string())),
            };
            if let Ok(peer) = stream.peer_pid()
                && peer != holder.pid
            {
                return Err(NotAsked::Unreachable(format!(
                    "its socket is held by {peer}, not {}",
                    holder.pid
                )));
            }
            write_client(&mut stream, &ClientMessage::Terminate)
                .map_err(|error| NotAsked::Refused(error.to_string()))?;
            stream.set_deadline(deadline);
            let ended = loop {
                match read_holder(&mut stream) {
                    Ok(HolderMessage::Closed(closed)) => {
                        return Ok(closed.reason.name().to_owned());
                    }
                    Ok(_) => {}
                    Err(error) => break error,
                }
            };
            // A holder closes a connection whose output it could not hand
            // over with no `Closed`, and a terminate it had not read yet goes
            // with that connection; one that still runs is asked again on a
            // new connection until the bound.
            let dropped = matches!(
                &ended,
                FrameError::Io(error) if error.kind() == std::io::ErrorKind::UnexpectedEof
            );
            if !dropped || Instant::now() >= deadline || !holder.runs() {
                return Err(NotAsked::Refused(ended.to_string()));
            }
            self.emit(
                "pane_holder.terminate_again",
                json!({ "pane": pane, "holder": holder.pid }),
            );
        }
    }

    /// Ends a closing pane's holder: asked over its socket, or, when nothing
    /// answers there for it, by its pids; then judges what is left.
    fn end_closing(&self, pane: &str, holder: Identity) -> Judgment {
        match self.terminate(pane, holder) {
            Ok(_) => {
                wait_gone(holder, CLOSE_LIMIT);
                self.after_close(pane, holder, None)
            }
            Err(NotAsked::Unreachable(why)) => {
                let left = end_holder_tree(holder).err();
                self.emit(
                    "pane_holder.closed_by_pids",
                    json!({ "pane": pane, "holder": holder.pid, "why": why, "left": left }),
                );
                let refused =
                    left.map(|left| format!("{why}; ending it by its pids left {left:?}"));
                self.after_close(pane, holder, refused)
            }
            Err(NotAsked::Refused(why)) => self.after_close(pane, holder, Some(why)),
        }
    }
}

/// A record file as read: the record, or why it cannot be.
type RecordRead = Result<PaneRecord, Unreadable>;

/// Why a holder was not asked to end over its socket.
enum NotAsked {
    /// Nothing listens at its socket, or another process does.
    Unreachable(String),
    /// It was reached and refused, answered out of protocol or did not say
    /// it ended.
    Refused(String),
}

/// When a start reached each of its steps, for its `pane_holder.started`
/// event: the version folder found or made, the record reserved under the
/// lock, then (in `start_holder`) the holder process started and its
/// `ready` read. A new tab or split on the core's own engine is answered
/// once the start returns, so these are where its time goes.
struct StartStages {
    begun: Instant,
    prepared: Duration,
    reserved: Duration,
}

/// `elapsed` in milliseconds to a tenth, as the diagnostics give times.
fn millis(elapsed: Duration) -> f64 {
    (elapsed.as_secs_f64() * 10_000.0).round() / 10.0
}

/// How an earlier start of a pane stands to a new start of it.
enum Earlier {
    /// It holds the pane's id: starting or running.
    Lives,
    /// It ended and nothing of it runs: its record goes.
    Over,
    /// It is closing, over but not written so, or something of it still
    /// runs.
    Unsettled,
}

/// What a start that did not say it was ready left in its record.
enum FailedStart {
    Recorded,
    /// The holder wrote itself running after all.
    Started(Box<PaneRecord>),
    Gone,
    /// Another start, or the holder's own end, holds the record now.
    MovedOn,
    Removed,
    /// Ended `start_unconfirmed` with this holder still running.
    Unconfirmed(Identity),
}

impl FailedStart {
    fn name(&self) -> &'static str {
        match self {
            Self::Started(_) => "started_late",
            Self::Gone => "record_gone",
            Self::MovedOn => "record_moved_on",
            Self::Removed => "removed",
            Self::Recorded => "recorded",
            Self::Unconfirmed(_) => "start_unconfirmed",
        }
    }
}

/// Whether nothing listens at `socket`: it is gone, or what is there is a
/// leftover no listener holds.
fn unreachable(error: &std::io::Error, socket: &Path) -> bool {
    match error.kind() {
        std::io::ErrorKind::NotFound => true,
        std::io::ErrorKind::ConnectionRefused => {
            // macOS refuses a live listener whose queue is full the same way.
            hide_platform::ipc::remove_leftover(socket, Duration::ZERO).is_ok()
        }
        _ => false,
    }
}

/// Ends a holder and its pane's tree by their pids: on Unix the holder's own
/// session and each session its children lead (the pane's program leads
/// one), whose members are found by their session id even once reparented;
/// on Windows the holder's tree, and its job ends the rest.
fn end_holder_tree(holder: Identity) -> Result<(), Vec<u32>> {
    if !holder.runs() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use hide_platform::process::{Session, SessionEnd};
        // A tree that cannot be read is not ended in part.
        let Ok(tree) = hide_platform::process::descendants(holder.pid) else {
            return Err(vec![holder.pid]);
        };
        let children: Vec<Identity> = tree
            .into_iter()
            .filter(|pid| hide_platform::process::parent_of(*pid).ok() == Some(holder.pid))
            .filter_map(Identity::of)
            .collect();
        let mut left = Vec::new();
        let mut end = |leader: Identity| match Session::left_by(leader.pid, leader.started)
            .and_then(|mut session| session.end(LEFTOVER_LIMIT))
        {
            Ok(SessionEnd::Ended) => {}
            Ok(SessionEnd::Unconfirmed { left: still }) => left.extend(still),
            Err(_) => left.push(leader.pid),
        };
        end(holder);
        for child in children {
            end(child);
        }
        if holder.runs() {
            left.push(holder.pid);
        }
        left.sort_unstable();
        left.dedup();
        if left.is_empty() { Ok(()) } else { Err(left) }
    }
    #[cfg(windows)]
    {
        // A tree that cannot be read is not ended in part.
        let Ok(tree) = hide_platform::process::descendants(holder.pid) else {
            return Err(vec![holder.pid]);
        };
        let tree: Vec<Identity> = tree.into_iter().filter_map(Identity::of).collect();
        if holder.runs() {
            let _ = hide_platform::process::kill_tree(holder.pid);
        }
        wait_gone(holder, LEFTOVER_LIMIT);
        let left: Vec<u32> = std::iter::once(holder)
            .chain(tree)
            .filter(|member| {
                wait_gone(*member, LEFTOVER_LIMIT);
                member.runs()
            })
            .map(|member| member.pid)
            .collect();
        if left.is_empty() { Ok(()) } else { Err(left) }
    }
}

/// Ends what a lost holder's pane left and puts in `end` what is still
/// left: on Unix the program's session, found by the program's recorded pid
/// and start time.
#[cfg(unix)]
fn end_leftovers(current: &PaneRecord, end: &mut End) {
    use hide_platform::process::{Session, SessionEnd};
    let Some(program) = current.program else {
        return;
    };
    match Session::left_by(program.pid, program.started)
        .and_then(|mut session| session.end(LEFTOVER_LIMIT))
    {
        Ok(SessionEnd::Ended) => {}
        Ok(SessionEnd::Unconfirmed { left }) => end.left = left,
        Err(error) => {
            end.detail = format!("{}; what it left could not be ended: {error}", end.detail);
            end.left = vec![program.pid];
        }
    }
}

/// On Windows the holder's kill-on-close job ended its pane with it.
#[cfg(windows)]
fn end_leftovers(_current: &PaneRecord, _end: &mut End) {}

/// Waits up to `limit` for `process` to end.
#[allow(clippy::disallowed_methods)] // a production wait for a process, bounded
fn wait_gone(process: Identity, limit: Duration) {
    let deadline = Instant::now() + limit;
    while process.runs() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The first line `reader` gives within `limit`, without its line end.
/// What a holder's report pipe gave within a start's bound.
#[derive(Debug)]
enum Report {
    /// Its first line: `ready ...` or `failed <reason>`.
    Line(String),
    /// The pipe closed with no line: the holder ended before it said
    /// anything, which is not the same failure as a holder still silent.
    Closed,
    /// The pipe could not be read.
    Unreadable(std::io::Error),
    /// Nothing within the bound.
    Silent,
}

fn first_line(reader: impl std::io::Read + Send + 'static, limit: Duration) -> Report {
    let (sender, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let report = match BufReader::new(reader).read_line(&mut line) {
            Ok(0) => Report::Closed,
            Ok(_) => Report::Line(line.trim().to_owned()),
            Err(error) => Report::Unreadable(error),
        };
        let _ = sender.send(report);
    });
    match heard.recv_timeout(limit) {
        Ok(report) => report,
        Err(mpsc::RecvTimeoutError::Timeout) => Report::Silent,
        Err(mpsc::RecvTimeoutError::Disconnected) => Report::Unreadable(std::io::Error::other(
            "the thread reading the report ended without an answer",
        )),
    }
}

/// The state folder's identity as a record's owner: the folder's volume and
/// number, which a rename keeps and a copy does not.
fn owner_of(state_dir: &Path) -> Result<String, String> {
    let id = hide_platform::fs::identity::file_id(state_dir)
        .map_err(|error| format!("{} could not be named: {error}", state_dir.display()))?;
    Ok(format!("{:x}:{:x}", id.volume(), id.index()))
}

/// A digest of what a start runs: the program and its words, the working
/// folder and the environment it adds, each with its length.
fn spec_of(request: &StartRequest) -> String {
    let mut digest = Sha256::new();
    let mut add = |bytes: &[u8]| {
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    };
    add(b"program");
    for word in &request.program {
        add(word.as_encoded_bytes());
    }
    add(b"cwd");
    add(request.cwd.as_os_str().as_encoded_bytes());
    add(b"env");
    for (key, value) in &request.env {
        add(key.as_encoded_bytes());
        add(value.as_encoded_bytes());
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(_: serde_json::Value) {}

    /// A process standing in for a holder: this test's own child, ended
    /// through its handle however the test ends.
    struct StandIn(std::process::Child);

    impl StandIn {
        fn start() -> Self {
            #[cfg(unix)]
            let child = std::process::Command::new("/bin/sleep").arg("60").spawn();
            #[cfg(windows)]
            let child = std::process::Command::new(
                std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into()),
            )
            .args(["/d", "/c", "ping -n 60 127.0.0.1 >NUL"])
            .spawn();
            Self(child.unwrap())
        }

        fn identity(&self) -> Identity {
            Identity::of(self.0.id()).expect("the stand-in runs")
        }
    }

    impl Drop for StandIn {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn reserved(panes: &Panes, holder: Option<Identity>, phase: Phase) -> PaneRecord {
        PaneRecord {
            format: RECORD_FORMAT,
            pane: "p1".to_owned(),
            owner: panes.owner().to_owned(),
            incarnation: 7,
            phase,
            protocol: PROTOCOL_VERSION,
            version_folder: String::new(),
            spec: String::new(),
            reservation: None,
            reserved_at_unix_ms: record::now_unix_ms(),
            boot: panes.boot.clone(),
            holder,
            program: None,
            end: None,
        }
    }

    #[test]
    fn a_report_pipe_that_closes_with_no_line_reads_as_ended_not_as_silent() {
        let limit = Duration::from_millis(200);
        assert!(matches!(
            first_line(std::io::empty(), limit),
            Report::Closed
        ));
        assert!(matches!(
            first_line(&b"ready holder=1\n"[..], limit),
            Report::Line(line) if line == "ready holder=1"
        ));
        let (reader, writer) = std::io::pipe().unwrap();
        assert!(matches!(first_line(reader, limit), Report::Silent));
        drop(writer);
    }

    #[test]
    fn a_holder_that_wrote_itself_running_after_the_bound_is_taken_as_started() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
        let stand_in = StandIn::start();
        let holder = stand_in.identity();
        let running = reserved(&panes, Some(holder), Phase::Running);
        record::write(&layout::pane_record(&panes.state_dir, "p1"), &running).unwrap();

        let answer = panes.settle_failed_start(
            &reserved(&panes, None, Phase::Preparing),
            Some(holder),
            "the holder said nothing".to_owned(),
        );
        assert_eq!(
            answer.as_ref().map(|record| record.phase),
            Ok(Phase::Running),
            "{answer:?}"
        );
        assert_eq!(panes.record("p1").unwrap(), Some(running));
        assert!(holder.runs());
    }

    #[test]
    fn cancellation_from_another_boot_never_ends_a_current_process() {
        for close_directly in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let mut panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
            panes.boot = Some("current-boot".to_owned());
            let owner = panes.acquire(Duration::ZERO).unwrap();
            let stand_in = StandIn::start();
            let holder = stand_in.identity();
            let mut current = reserved(&panes, Some(holder), Phase::Ended);
            current.boot = Some("previous-boot".to_owned());
            current.reservation = Some(
                serde_json::from_value(json!({
                    "owner": owner.incarnation(), "request": "a".repeat(64),
                    "cwd": hide_platform::path::to_wire(home.path()).unwrap(), "cancelled": true,
                }))
                .unwrap(),
            );
            current.end = Some(End::new(
                EndReason::Terminated,
                "previous boot cancellation",
            ));
            record::write(&layout::pane_record(&panes.state_dir, "p1"), &current).unwrap();
            let ended = if close_directly {
                panes.close(&owner, "p1").unwrap()
            } else {
                let Judgment::Ended(ended) = panes.judge(&owner, &current) else {
                    panic!("the previous boot did not settle without cleanup");
                };
                ended
            };
            assert_eq!(ended.phase, Phase::Ended);
            assert!(ended.cancelled());
            assert!(holder.runs(), "a previous boot does not own this process");
        }
    }

    /// 2026-11-06: macOS numbered the state folder's volume anew after a
    /// reboot, so every record named another owner and was left unjudged;
    /// the restore took the panes as running and started none. A record of
    /// an earlier boot is settled as this folder's whatever owner it names,
    /// and a process of this boot that reuses its pid is left alone.
    #[test]
    fn a_record_of_an_earlier_boot_is_settled_here_whatever_owner_it_names() {
        let home = tempfile::tempdir().unwrap();
        let mut panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
        panes.boot = Some("current-boot".to_owned());
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let stand_in = StandIn::start();
        let holder = stand_in.identity();
        let mut earlier = reserved(&panes, Some(holder), Phase::Running);
        earlier.boot = Some("previous-boot".to_owned());
        earlier.owner = "1000012:12a9ed47".to_owned();
        record::write(&layout::pane_record(&panes.state_dir, "p1"), &earlier).unwrap();

        let found = panes.reconcile(&owner, Duration::from_secs(5));
        let Some(Judgment::Ended(ended)) = found.panes.get("p1") else {
            panic!("the earlier boot's record was not settled: {found:?}");
        };
        assert_eq!(
            ended.end.as_ref().unwrap().reason,
            EndReason::MachineRestarted
        );
        let kept = panes.record("p1").unwrap().unwrap();
        assert_eq!(kept.owner, panes.owner(), "the record is this folder's now");
        assert_eq!(kept.phase, Phase::Ended);
        assert!(holder.runs(), "a process of this boot is not the record's");
    }

    /// A replacement that finds its record gone starts afresh, so a record
    /// whose holder is gone but whose pane left processes running is never
    /// dropped by the ended-record cap nor forgotten: dropping it would let
    /// a restart run a second shell beside them.
    #[test]
    fn an_ended_record_whose_pane_left_processes_running_is_neither_dropped_nor_forgotten() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let stand_in = StandIn::start();
        let survivor = stand_in.identity().pid;
        let write = |pane: &str, at: u64, left: Vec<u32>| {
            let mut ended = reserved(&panes, None, Phase::Ended);
            ended.pane = pane.to_owned();
            let mut end = End::new(EndReason::HolderFailed, "test");
            end.at_unix_ms = at;
            end.left = left;
            ended.end = Some(end);
            record::write(&layout::pane_record(&panes.state_dir, pane), &ended).unwrap();
        };
        write("left", 1, vec![survivor]);
        for n in 0..ENDED_KEPT {
            write(&format!("e{n}"), 2 + n as u64, Vec::new());
        }

        let scan = panes.scan().unwrap();
        assert_eq!(panes.count_live_for(&owner, &scan), 0);
        assert!(
            panes.record("left").unwrap().is_some(),
            "dropped over a survivor"
        );
        assert!(
            panes.record("e0").unwrap().is_some(),
            "kept past the cap, as a running holder's record is"
        );
        assert!(matches!(
            panes.forget(&owner, "left"),
            Err(ForgetRefusal::ProcessesRun(pids)) if pids == vec![survivor]
        ));
        drop(stand_in);
        panes.forget(&owner, "left").unwrap();
        assert!(panes.record("left").unwrap().is_none());
    }

    #[test]
    fn cancellation_never_records_completion_before_the_live_holder_leaves() {
        for cleanup_refused in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
            let owner = panes.acquire(Duration::ZERO).unwrap();
            let stand_in = StandIn::start();
            let holder = stand_in.identity();
            let mut current = reserved(&panes, Some(holder), Phase::Ended);
            current.reservation = Some(
                serde_json::from_value(json!({
                    "owner": owner.incarnation(), "request": "a".repeat(64),
                    "cwd": hide_platform::path::to_wire(home.path()).unwrap(), "cancelled": false,
                }))
                .unwrap(),
            );
            current.end = Some(End::new(
                EndReason::ProgramExited,
                "the holder has not left yet",
            ));
            // Exactly the close decision, committed before cleanup can run.
            {
                let _lock = panes.lock().unwrap();
                panes.prepare_close(&mut current);
                record::write(&layout::pane_record(&panes.state_dir, "p1"), &current).unwrap();
            }
            assert_eq!(current.phase, Phase::Closing);
            assert!(holder.runs());
            if cleanup_refused {
                // Inject a cleanup refusal at its production result boundary;
                // the actual stand-in stays alive and must never be closed.
                assert!(matches!(
                    panes.after_close(
                        "p1",
                        holder,
                        Some("cleanup could not be confirmed".to_owned())
                    ),
                    Judgment::Closing(_, _)
                ));
                let pending = panes.record("p1").unwrap().unwrap();
                assert_eq!(pending.phase, Phase::Closing);
                assert!(pending.end.unwrap().left.contains(&holder.pid));
            }
            // The previous connection can disappear at either boundary. Its
            // successor must finish the actual process before reporting ended.
            drop(owner);
            let owner = panes.acquire(Duration::ZERO).unwrap();
            let current = panes.record("p1").unwrap().unwrap();
            let Judgment::Ended(ended) = panes.judge(&owner, &current) else {
                panic!("the successor did not finish the cancelled holder");
            };
            assert!(!holder.runs());
            assert_eq!(ended.end.unwrap().reason, EndReason::Terminated);
        }
    }

    #[test]
    fn a_start_given_up_on_is_recorded_and_its_holder_ended() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("s"), quiet).unwrap();
        let mut stand_in = StandIn::start();
        let holder = stand_in.identity();
        let preparing = reserved(&panes, Some(holder), Phase::Preparing);
        record::write(&layout::pane_record(&panes.state_dir, "p1"), &preparing).unwrap();

        let answer = panes.settle_failed_start(
            &preparing,
            Some(holder),
            "the holder said nothing".to_owned(),
        );
        assert!(answer.is_err(), "{answer:?}");
        let ended = panes.record("p1").unwrap().unwrap();
        assert_eq!(ended.phase, Phase::Ended);
        assert_eq!(
            ended.end.map(|end| end.reason),
            Some(EndReason::StartUnconfirmed)
        );
        // Ended, so its parent can collect it at once.
        let status = stand_in.0.wait().unwrap();
        assert!(!status.success(), "{status:?}");
    }
}
