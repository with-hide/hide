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
//! - A reconcile ends only holders no record names, after it has read their
//!   own arguments to know they are this state folder's, and ends them whole.
//!   A record it cannot read, of a format it does not know, of another state
//!   folder or of a holder version too old to speak to, and a holder that
//!   refuses or does not answer a connection, are left running and reported
//!   with the reason, never taken for "no record".
//!
//! Diagnostics go to the sink the caller gives (`hide_node::diagnostic!` in
//! the node), one JSON record per event, naming the pane, the start's
//! incarnation and the holder's pid, never what a pane's program wrote.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use hide_holder_protocol::record::{
    self, End, EndReason, Identity, PaneRecord, Phase, RECORD_FORMAT, Unreadable,
};
use hide_holder_protocol::{
    ClientMessage, HolderMessage, OLDEST_HOLDER_VERSION, OpenError, PROTOCOL_VERSION, Size,
    read_holder, write_client,
};
use hide_platform::process::{ResidentCommand, start_resident};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{holders, layout};

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

/// How much of a holder's log is read looking for an end it could not
/// record.
const LOG_TAIL: u64 = 256 * 1024;

/// Where diagnostics go: one JSON record each.
pub type Sink = fn(serde_json::Value);

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

/// A holder no record names, or what one left, and what was done about it.
#[derive(Debug)]
pub struct Orphan {
    /// The pane its socket names, when one does.
    pub pane: Option<String>,
    pub holder: Option<Identity>,
    pub outcome: OrphanOutcome,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OrphanOutcome {
    /// Asked to end over its socket; it said why it closed.
    Terminated(String),
    /// Unreachable over its socket, so its tree was ended by its pids.
    Ended,
    /// Ending it left these processes running.
    EndUnconfirmed(Vec<u32>),
    /// Only a socket a killed holder left; removed.
    StaleSocketRemoved,
    /// Left running, with the reason it could not be judged or ended.
    Kept(String),
    /// Not taken up within the reconcile's budget; the next reconcile takes
    /// it up.
    Pending,
}

/// What a reconcile found and did.
#[derive(Debug, Default)]
pub struct Reconciled {
    pub panes: BTreeMap<String, Judgment>,
    pub orphans: Vec<Orphan>,
    /// Records that could not be listed at all.
    pub failed: Option<String>,
}

impl Reconciled {
    /// The panes and orphans left for the next reconcile.
    pub fn pending(&self) -> usize {
        let panes = self
            .panes
            .values()
            .filter(|judgment| matches!(judgment, Judgment::Pending(_)))
            .count();
        let orphans = self
            .orphans
            .iter()
            .filter(|orphan| orphan.outcome == OrphanOutcome::Pending)
            .count();
        panes + orphans
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
    RecordUnreadable(Unreadable),
    Failed(String),
}

impl std::fmt::Display for ForgetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEnded(phase) => write!(f, "the pane is {phase:?}, not ended"),
            Self::HolderRuns(holder) => write!(f, "its holder {} still runs", holder.pid),
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
        Ok(Self {
            home: home.to_owned(),
            state_dir: state_dir.to_owned(),
            owner,
            boot,
            sink,
        })
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
    /// pane that lives (starting, running or closing) is answered as it is,
    /// one that is over (ended, or its holder gone) is settled, cleared and
    /// started again as a new incarnation, so a caller asks the same start
    /// again whatever became of the last one.
    pub fn start(
        &self,
        request: &StartRequest,
        source: &holders::Source,
    ) -> Result<Started, StartRefusal> {
        record::check_pane(&request.pane).map_err(StartRefusal::InvalidPane)?;
        let spec = spec_of(request);
        let reservation =
            holders::prepare(&self.home, PROTOCOL_VERSION, source).map_err(StartRefusal::Failed)?;
        let version_folder = reservation
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = layout::pane_record(&self.state_dir, &request.pane);
        // One settle of an earlier start that is over but not yet written so,
        // outside the lock, then one more look.
        let mut settled = false;
        let reserved = loop {
            let lock = self.lock().map_err(StartRefusal::Failed)?;
            match record::read(&path) {
                Ok(Some(existing)) if existing.owner != self.owner => {
                    return Err(StartRefusal::OtherOwner(Box::new(existing)));
                }
                Ok(Some(existing)) => match self.state_of(&existing) {
                    Earlier::Lives if existing.spec == spec => {
                        return Ok(Started::Existing(existing));
                    }
                    Earlier::Lives => return Err(StartRefusal::Conflict(Box::new(existing))),
                    Earlier::Over => self.clear_earlier(&existing, &path)?,
                    Earlier::Unsettled if !settled => {
                        drop(lock);
                        settled = true;
                        self.settle_earlier(&existing);
                        continue;
                    }
                    Earlier::Unsettled => {
                        let left = self.still_running(&existing);
                        if !left.is_empty() {
                            return Err(StartRefusal::EarlierRuns(left));
                        }
                        // Not written so, but nothing of it runs: over.
                        self.clear_earlier(&existing, &path)?;
                    }
                },
                Err(error) => return Err(StartRefusal::RecordUnreadable(error)),
                Ok(None) => {}
            }
            let count = self.count_live().map_err(StartRefusal::Failed)?;
            if count >= HOLDER_CAP {
                self.emit(
                    "pane_holder.start_refused",
                    json!({ "pane": request.pane, "reason": "holder_cap", "count": count, "cap": HOLDER_CAP }),
                );
                return Err(StartRefusal::Cap { count });
            }
            let incarnation = self.next_incarnation().map_err(StartRefusal::Failed)?;
            let reserved = PaneRecord {
                format: RECORD_FORMAT,
                pane: request.pane.clone(),
                owner: self.owner.clone(),
                incarnation,
                phase: Phase::Preparing,
                protocol: PROTOCOL_VERSION,
                version_folder: version_folder.clone(),
                spec: spec.clone(),
                reserved_at_unix_ms: record::now_unix_ms(),
                boot: self.boot.clone(),
                holder: None,
                program: None,
                end: None,
            };
            record::write(&path, &reserved).map_err(|error| {
                StartRefusal::Failed(format!("the record could not be written: {error}"))
            })?;
            break reserved;
        };
        let started = self.start_holder(request, &reservation, &reserved);
        drop(reservation);
        started.map(Started::Started).map_err(StartRefusal::Failed)
    }

    /// Removes an earlier start that is over, under the lock, for a new start
    /// of its pane. Its log stays: the next holder writes on after it, and
    /// its lines name their own incarnation.
    fn clear_earlier(&self, existing: &PaneRecord, path: &Path) -> Result<(), StartRefusal> {
        let _ = hide_platform::ipc::remove_leftover(
            &layout::holder_socket(&self.state_dir, &existing.pane),
            LOCK_RELEASE,
        );
        std::fs::remove_file(path).map_err(|error| {
            StartRefusal::Failed(format!("the earlier record could not be removed: {error}"))
        })?;
        self.emit(
            "pane_holder.start_again",
            json!({
                "pane": existing.pane,
                "earlier": existing.incarnation,
                "phase": existing.phase,
                "reason": existing.end.as_ref().map(|end| end.reason.name()),
            }),
        );
        Ok(())
    }

    /// Settles an earlier start that is over but not written so, outside
    /// the lock: a holder that wrote its end is given its bound to exit, and
    /// anything else is judged as a reconcile judges it.
    fn settle_earlier(&self, existing: &PaneRecord) {
        match existing.holder {
            Some(holder) if existing.phase == Phase::Ended && holder.runs() => {
                self.end_unconfirmed_start(existing);
                wait_gone(holder, START_LIMIT);
            }
            _ => {
                let _ = self.judge(existing);
            }
        }
    }

    /// Whether an earlier start of a pane still holds its id. Read under the
    /// lock, so it looks only at the record and the processes it names.
    fn state_of(&self, existing: &PaneRecord) -> Earlier {
        let runs = existing.holder.is_some_and(|holder| holder.runs());
        let other_boot =
            existing.boot.is_some() && self.boot.is_some() && existing.boot != self.boot;
        match existing.phase {
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
        if existing.boot.is_some() && self.boot.is_some() && existing.boot != self.boot {
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
        let holder = Identity::of(resident.pid);
        if resident.outer_job {
            self.emit(
                "pane_holder.outer_job",
                json!({ "pane": pane, "incarnation": reserved.incarnation, "holder": resident.pid }),
            );
        }
        match first_line(reader, START_LIMIT) {
            Some(line) if line.starts_with("ready ") => {
                let _lock = self.lock()?;
                match record::read(&layout::pane_record(&self.state_dir, pane)) {
                    // A program that exits at once may have ended the pane
                    // by now; the start still took place.
                    Ok(Some(written))
                        if written.incarnation == reserved.incarnation
                            && written.phase != Phase::Preparing =>
                    {
                        self.emit(
                            "pane_holder.started",
                            json!({ "pane": pane, "incarnation": written.incarnation, "holder": resident.pid }),
                        );
                        Ok(written)
                    }
                    other => Err(format!(
                        "the holder said it was ready but the record reads {other:?}"
                    )),
                }
            }
            Some(line) => {
                let reason = line.strip_prefix("failed ").unwrap_or(&line).to_owned();
                if let Some(holder) = holder {
                    wait_gone(holder, START_LIMIT);
                }
                self.settle_failed_start(reserved, holder, reason)
            }
            None => {
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
                Phase::Preparing => false,
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
    pub fn close(&self, pane: &str) -> Result<PaneRecord, CloseError> {
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
            if current.phase == Phase::Ended {
                return Ok(current);
            }
            // Written `closing`, a start whose holder has not written itself
            // in yet finds the record no longer reserves it and stops.
            if current.phase == Phase::Preparing && current.holder.is_none() {
                current.phase = Phase::Closing;
                record::write(&path, &current).map_err(|error| {
                    CloseError::Failed(format!("the record could not be written: {error}"))
                })?;
                return Err(CloseError::Starting);
            }
            if current.phase != Phase::Closing {
                current.phase = Phase::Closing;
                record::write(&path, &current).map_err(|error| {
                    CloseError::Failed(format!("the record could not be written: {error}"))
                })?;
            }
            current
        };
        self.emit(
            "pane_holder.closing",
            json!({ "pane": pane, "incarnation": current.incarnation, "holder": current.holder.map(|h| h.pid) }),
        );
        let judgment = match current.holder {
            Some(holder) if holder.runs() => self.end_closing(pane, holder),
            // The holder is gone: the record says why, or it is settled as a
            // reconcile settles a lost holder.
            _ => match self.record(pane) {
                Ok(Some(after)) => self.judge(&after),
                Ok(None) => return Err(CloseError::NoRecord),
                Err(error) => return Err(CloseError::RecordUnreadable(error)),
            },
        };
        match judgment {
            Judgment::Ended(record) => Ok(record),
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
    pub fn forget(&self, pane: &str) -> Result<(), ForgetRefusal> {
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
        if let Some(holder) = current.holder
            && holder.runs()
        {
            return Err(ForgetRefusal::HolderRuns(holder));
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

    /// The records not ended, and the ones that cannot be read, which may
    /// name a holder. Ended records past [`ENDED_KEPT`] go, oldest first.
    fn count_live(&self) -> Result<usize, String> {
        let scan = self.scan()?;
        let mut live = 0;
        let mut ended = Vec::new();
        for (pane, read) in &scan {
            match read {
                Ok(record) if record.phase == Phase::Ended => ended.push((
                    record.end.as_ref().map_or(0, |end| end.at_unix_ms),
                    pane.clone(),
                    record.holder,
                )),
                _ => live += 1,
            }
        }
        if ended.len() > ENDED_KEPT {
            ended.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
            for (_, pane, holder) in &ended[..ended.len() - ENDED_KEPT] {
                // A holder still exiting keeps its record until it is gone.
                if let Some(holder) = holder.filter(|holder| holder.runs()) {
                    self.emit(
                        "pane_holder.ended_kept",
                        json!({ "pane": pane, "holder": holder.pid, "reason": "holder_runs" }),
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
        Ok(live)
    }

    /// Every record file, by pane, read. The caller holds the lock.
    fn scan(&self) -> Result<Vec<(String, RecordRead)>, String> {
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
            let read = record::read(&entry.path())
                .and_then(|found| found.ok_or(Unreadable::Corrupt("removed while read".into())));
            scan.push((pane.to_owned(), read));
        }
        scan.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(scan)
    }

    /// The next incarnation, kept in a file of its own so a pane forgotten
    /// and started again never has an earlier start's number. The caller
    /// holds the lock.
    fn next_incarnation(&self) -> Result<u64, String> {
        let path = layout::pane_incarnations(&self.state_dir);
        let next = match std::fs::read_to_string(&path) {
            Ok(text) => text
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("{} does not hold a number", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 1,
            Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
        };
        hide_platform::fs::atomic::write_file(
            &path,
            (next + 1).to_string().as_bytes(),
            hide_platform::fs::Access::Private,
        )
        .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
        Ok(next)
    }

    /// Reads every record and every holder this state folder runs, adopts
    /// what a record names, settles what lost holders left, and ends the
    /// holders no record names. See the module documentation.
    ///
    /// Panes are judged by up to [`RECONCILE_WORKERS`] at once, and none is
    /// taken up after `budget` has passed: a judgment already under way runs
    /// to its own bound, and what was not taken up is answered `Pending` (an
    /// orphan `OrphanOutcome::Pending`) for the next reconcile, which the
    /// caller asks for while [`Reconciled::is_complete`] says no.
    pub fn reconcile(&self, budget: Duration) -> Reconciled {
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
        let mut named_panes = BTreeSet::new();
        let mut readable = Vec::new();
        for (pane, read) in scan {
            named_panes.insert(pane.clone());
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
        let judged = self.bounded(&readable, deadline, |current| self.judge(current));
        for (current, judgment) in readable.into_iter().zip(judged) {
            let judgment = judgment.unwrap_or_else(|| Judgment::Pending(current.clone()));
            found.panes.insert(current.pane, judgment);
        }
        let candidates = self.orphan_candidates(&named_panes, &mut found.orphans);
        let ended = self.bounded(&candidates, deadline, |candidate| {
            self.end_orphan(
                candidate.pane.as_deref(),
                &candidate.socket,
                candidate.holder,
            )
        });
        for (candidate, outcome) in candidates.into_iter().zip(ended) {
            found.orphans.push(Orphan {
                pane: candidate.pane,
                holder: candidate.holder,
                outcome: outcome.unwrap_or(OrphanOutcome::Pending),
            });
        }
        self.emit(
            "pane_holder.reconciled",
            json!({
                "panes": found.panes.len(),
                "pending": found.pending(),
                "took_ms": started.elapsed().as_millis() as u64,
                "orphans": found.orphans.iter().map(|orphan| json!({
                    "pane": orphan.pane,
                    "holder": orphan.holder.map(|h| h.pid),
                    "outcome": format!("{:?}", orphan.outcome),
                })).collect::<Vec<_>>(),
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
    fn judge(&self, current: &PaneRecord) -> Judgment {
        let pane = current.pane.clone();
        let judgment = if current.owner != self.owner {
            Judgment::OtherOwner(current.clone())
        } else if current.protocol < OLDEST_HOLDER_VERSION {
            Judgment::Unsupported(current.clone())
        } else if current.phase == Phase::Ended {
            self.end_unconfirmed_start(current);
            let gone = current.holder.is_none_or(|holder| !holder.runs());
            self.remove_stale_socket(&pane, if gone { LOCK_RELEASE } else { Duration::ZERO });
            Judgment::Ended(current.clone())
        } else if current.boot.is_some() && self.boot.is_some() && current.boot != self.boot {
            // Nothing a record of another boot names runs in this one; its
            // pids and start times may name processes of this boot.
            let ended = self.end_record(
                current,
                End::new(
                    EndReason::MachineRestarted,
                    "the record is from an earlier boot of the machine",
                ),
            );
            // No process of another boot holds a lock in this one.
            self.remove_stale_socket(&pane, Duration::ZERO);
            ended
        } else {
            match current.holder {
                // A close a node asked for and did not see through.
                Some(holder) if holder.runs() && current.phase == Phase::Closing => {
                    self.end_closing(&pane, holder)
                }
                Some(holder) if holder.runs() => Judgment::Alive(current.clone()),
                Some(holder) => self.settle_lost(current, holder),
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
            Ok(Some(after)) if after.phase == Phase::Ended => Judgment::Ended(after),
            Ok(Some(after)) if !holder.runs() => self.settle_lost(&after, holder),
            Ok(Some(after)) => Judgment::Closing(
                after,
                refused.unwrap_or_else(|| {
                    format!(
                        "it was asked to end and still runs after {} s",
                        CLOSE_LIMIT.as_secs()
                    )
                }),
            ),
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

    /// A record whose holder ended without ending it: the end it logged, or
    /// `holder_lost`; on Unix what its pane's session left is ended by the
    /// program's recorded identity (on Windows the holder's job ended it).
    fn settle_lost(&self, current: &PaneRecord, holder: Identity) -> Judgment {
        // A holder that ended its pane since the record was read wrote why.
        match self.record(&current.pane) {
            Ok(Some(now)) if now.phase == Phase::Ended => return Judgment::Ended(now),
            Ok(Some(now)) if now == *current => {}
            Ok(_) => return Judgment::Unsettled("the record changed while it was settled".into()),
            Err(error) => return Judgment::Unsettled(error.to_string()),
        }
        let mut end = self.logged_end(current, holder).unwrap_or_else(|| {
            End::new(
                EndReason::HolderLost,
                "the holder ended without saying why (killed or crashed)",
            )
        });
        // What the holder said it left is looked for again; only what is
        // still left now keeps the record open.
        let said_left = std::mem::take(&mut end.left);
        if !said_left.is_empty() {
            end.detail = format!("{}; it said it left {said_left:?}", end.detail);
        }
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

    /// The end the holder logged it could not record, if it did.
    fn logged_end(&self, current: &PaneRecord, holder: Identity) -> Option<End> {
        let log = layout::holder_log(&self.state_dir, &current.pane);
        let mut aside = log.clone().into_os_string();
        aside.push(".1");
        for file in [log, PathBuf::from(aside)] {
            let Some(text) = tail(&file, LOG_TAIL) else {
                continue;
            };
            for line in text.lines().rev() {
                let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                // The line of this start's holder: a pane started again logs
                // into the same file.
                if event["event"] != record::UNWRITTEN_EVENT
                    || event["holder"].as_u64() != Some(u64::from(holder.pid))
                    || event["incarnation"].as_u64() != Some(current.incarnation)
                {
                    continue;
                }
                let mut end = End::new(
                    EndReason::Unrecorded,
                    format!(
                        "the holder logged {} but could not record it: {}",
                        event["reason"].as_str().unwrap_or("an end"),
                        event["error"].as_str().unwrap_or("")
                    ),
                );
                end.exit_code = event["exit_code"]
                    .as_i64()
                    .and_then(|code| i32::try_from(code).ok());
                end.left = event["left"]
                    .as_array()
                    .map(|left| {
                        left.iter()
                            .filter_map(|pid| pid.as_u64()?.try_into().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                return Some(end);
            }
        }
        None
    }

    /// Writes `current` as ended with `end`, if it has not changed since it
    /// was read, and answers its judgment.
    fn end_record(&self, current: &PaneRecord, end: End) -> Judgment {
        let mut ended = current.clone();
        ended.phase = Phase::Ended;
        ended.end = Some(end);
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
        if let Ok(true) = hide_platform::ipc::remove_leftover(&socket, wait) {
            self.emit("pane_holder.stale_socket_removed", json!({ "pane": pane }));
        }
    }

    /// Asks the holder at the pane's socket to end the pane, once that
    /// holder is `holder` (when the system names a connection's peer), and
    /// waits for its answer.
    fn terminate(&self, pane: &str, holder: Identity) -> Result<String, NotAsked> {
        let socket = layout::holder_socket(&self.state_dir, pane);
        let (mut stream, _) =
            match hide_holder_protocol::open(&socket, Vec::new(), Instant::now() + CONNECT_LIMIT) {
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
        stream.set_deadline(Instant::now() + CLOSE_LIMIT);
        loop {
            match read_holder(&mut stream) {
                Ok(HolderMessage::Closed(closed)) => return Ok(closed.reason.name().to_owned()),
                Ok(_) => {}
                Err(error) => return Err(NotAsked::Refused(error.to_string())),
            }
        }
    }

    /// Ends a closing pane's holder: asked over its socket, or, when nothing
    /// answers there for it, by its pids as an orphan is; then judges what
    /// is left.
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

    /// The holders this state folder runs that no record names, found by
    /// the version folders' programs and the sockets in the state folder;
    /// what cannot be judged is put in `kept` with its reason, and a socket
    /// file no process holds is removed. Nothing is ended here.
    fn orphan_candidates(
        &self,
        named_panes: &BTreeSet<String>,
        kept: &mut Vec<Orphan>,
    ) -> Vec<Candidate> {
        let panes_dir = layout::holder_panes(&self.state_dir);
        let mut candidates = Vec::new();
        let mut sockets_seen = BTreeSet::new();
        let mut keep = |pane: Option<String>, holder: Option<Identity>, why: String| {
            kept.push(Orphan {
                pane,
                holder,
                outcome: OrphanOutcome::Kept(why),
            });
        };
        let (processes, unlisted) = self.holder_processes();
        for why in unlisted {
            keep(None, None, why);
        }
        for (holder, version) in processes {
            let arguments = match hide_platform::process::arguments(holder.pid) {
                Ok(arguments) => arguments,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    keep(
                        None,
                        Some(holder),
                        format!(
                            "its arguments could not be read, so its state folder is unknown: {error}"
                        ),
                    );
                    continue;
                }
            };
            let Some(socket) = value_after(&arguments, "--socket") else {
                continue;
            };
            let socket = PathBuf::from(socket);
            // A folder that is gone is not this state folder's, which exists.
            match socket
                .parent()
                .map(|parent| hide_platform::fs::identity::same_file(parent, &panes_dir))
            {
                Some(Ok(true)) => {}
                None | Some(Ok(false)) => continue,
                Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Some(Err(error)) => {
                    keep(
                        None,
                        Some(holder),
                        format!(
                            "its socket's folder could not be compared with this state folder's: {error}"
                        ),
                    );
                    continue;
                }
            }
            let pane = socket
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".sock"))
                .map(str::to_owned);
            sockets_seen.insert(socket.clone());
            let incarnation = value_after(&arguments, "--incarnation")
                .and_then(|value| value.to_str()?.parse::<u64>().ok());
            if let Some(pane) = &pane
                && self.names(pane, holder, incarnation) != Named::No
            {
                continue;
            }
            if version < OLDEST_HOLDER_VERSION {
                keep(
                    pane,
                    Some(holder),
                    format!("it speaks version {version}, older than this build speaks to"),
                );
                continue;
            }
            candidates.push(Candidate {
                pane,
                socket,
                holder: Some(holder),
            });
        }
        let Ok(entries) = std::fs::read_dir(&panes_dir) else {
            return candidates;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(pane) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".sock"))
                .map(str::to_owned)
            else {
                continue;
            };
            if named_panes.contains(&pane)
                || sockets_seen.contains(&path)
                || !hide_platform::ipc::is_endpoint(&path).unwrap_or(false)
            {
                continue;
            }
            match hide_platform::ipc::remove_leftover(&path, Duration::ZERO) {
                Ok(_) => kept.push(Orphan {
                    pane: Some(pane),
                    holder: None,
                    outcome: OrphanOutcome::StaleSocketRemoved,
                }),
                // A listener no process scan found: one reachable only
                // through its socket.
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    candidates.push(Candidate {
                        pane: Some(pane),
                        socket: path,
                        holder: None,
                    });
                }
                Err(error) => kept.push(Orphan {
                    pane: Some(pane),
                    holder: None,
                    outcome: OrphanOutcome::Kept(format!(
                        "its socket could not be judged: {error}"
                    )),
                }),
            }
        }
        candidates
    }

    /// Ends one holder no record names: over its socket when it answers
    /// there, by its pids when nothing listens at its socket or the socket is
    /// another holder's. Anything else leaves it running, with the reason.
    fn end_orphan(
        &self,
        pane: Option<&str>,
        socket: &Path,
        holder: Option<Identity>,
    ) -> OrphanOutcome {
        let taken = pane.is_some_and(|pane| self.record(pane).is_ok_and(|read| read.is_some()));
        let reach = if taken {
            Err(OpenError::Io(std::io::Error::other(
                "its socket is another pane holder's",
            )))
        } else {
            hide_holder_protocol::open(socket, Vec::new(), Instant::now() + CONNECT_LIMIT)
        };
        let outcome = match reach {
            Ok((mut stream, _)) => {
                if let (Some(holder), Ok(peer)) = (holder, stream.peer_pid())
                    && peer != holder.pid
                {
                    // Another holder listens at its socket; this one is
                    // reachable only by its pids.
                    drop(stream);
                    return self.end_by_pids(pane, holder);
                }
                // Asked under the lock, so a record made for the pane since
                // the look is seen first.
                let asked = (|| -> Result<(), String> {
                    let _lock = self.lock()?;
                    if let Some(pane) = pane
                        && self
                            .record(pane)
                            .map_err(|error| error.to_string())?
                            .is_some()
                    {
                        return Err("a record names its pane now".to_owned());
                    }
                    write_client(&mut stream, &ClientMessage::Terminate)
                        .map_err(|error| error.to_string())
                })();
                match asked {
                    Err(why) => OrphanOutcome::Kept(why),
                    Ok(()) => {
                        stream.set_deadline(Instant::now() + CLOSE_LIMIT);
                        let answer = loop {
                            match read_holder(&mut stream) {
                                Ok(HolderMessage::Closed(closed)) => break Ok(closed),
                                Ok(_) => {}
                                Err(error) => break Err(error),
                            }
                        };
                        match answer {
                            Ok(closed) => {
                                if let Some(holder) = holder {
                                    wait_gone(holder, CLOSE_LIMIT);
                                }
                                OrphanOutcome::Terminated(closed.reason.name().to_owned())
                            }
                            Err(error) => OrphanOutcome::Kept(format!(
                                "it was asked to end and did not say it had: {error}"
                            )),
                        }
                    }
                }
            }
            Err(OpenError::Refused(closed)) => OrphanOutcome::Kept(format!(
                "it refused the connection: {}",
                closed.reason.name()
            )),
            Err(OpenError::Unsupported { version }) => OrphanOutcome::Kept(format!(
                "it speaks version {version}, older than this build speaks to"
            )),
            Err(OpenError::Protocol(error)) => {
                OrphanOutcome::Kept(format!("it answered out of protocol: {error}"))
            }
            Err(OpenError::Io(error)) => match holder {
                Some(holder) if taken || unreachable(&error, socket) => {
                    self.end_by_pids(pane, holder)
                }
                _ => OrphanOutcome::Kept(format!("it could not be reached: {error}")),
            },
        };
        self.emit(
            "pane_holder.orphan",
            json!({
                "pane": pane,
                "holder": holder.map(|h| h.pid),
                "outcome": format!("{outcome:?}"),
            }),
        );
        outcome
    }

    /// Ends a holder no record names, and its pane's tree, by their pids,
    /// once it is sure no record has named it since.
    fn end_by_pids(&self, pane: Option<&str>, holder: Identity) -> OrphanOutcome {
        // Looked at under the lock and ended outside it: a holder writes
        // itself only into the reservation of its own incarnation, which no
        // later start reuses, so no record can come to name it meanwhile.
        {
            let _lock = match self.lock() {
                Ok(lock) => lock,
                Err(why) => return OrphanOutcome::Kept(why),
            };
            if let Some(pane) = pane
                && self.names(pane, holder, None) != Named::No
            {
                return OrphanOutcome::Kept("a record names it now".to_owned());
            }
        }
        match end_holder_tree(holder) {
            Ok(()) => OrphanOutcome::Ended,
            Err(left) => OrphanOutcome::EndUnconfirmed(left),
        }
    }

    /// Whether the pane's record names `holder`, or a start of
    /// `incarnation` whose holder has not written itself in yet.
    fn names(&self, pane: &str, holder: Identity, incarnation: Option<u64>) -> Named {
        match self.record(pane) {
            Err(_) => Named::Unknown,
            Ok(None) => Named::No,
            Ok(Some(current)) => {
                if current.holder == Some(holder)
                    || (current.holder.is_none() && Some(current.incarnation) == incarnation)
                {
                    Named::Yes
                } else {
                    Named::No
                }
            }
        }
    }

    /// Every holder process run from a version folder under `home`, with
    /// the protocol version its folder names.
    fn holder_processes(&self) -> (Vec<(Identity, u16)>, Vec<String>) {
        let root = layout::holders_root(&self.home);
        let mut processes = Vec::new();
        let mut unlisted = Vec::new();
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (processes, unlisted);
            }
            Err(error) => {
                unlisted.push(format!("{} could not be listed: {error}", root.display()));
                return (processes, unlisted);
            }
        };
        for entry in entries.flatten() {
            let program = entry.path().join(holders::program_name());
            let Ok(file) = hide_platform::fs::identity::file_id(&program) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            // A version folder, or one a cleanup claimed; a staging folder
            // runs nothing.
            let Some(version) = name
                .strip_prefix(".removing-")
                .unwrap_or(&name)
                .strip_prefix('v')
                .and_then(|rest| rest.split('-').next())
                .and_then(|number| number.parse::<u16>().ok())
            else {
                continue;
            };
            match hide_platform::process::running(&file) {
                Ok(pids) => processes.extend(
                    pids.into_iter()
                        .filter_map(Identity::of)
                        .map(|holder| (holder, version)),
                ),
                Err(error) => unlisted.push(format!(
                    "the holders of {name} could not be listed: {error}"
                )),
            }
        }
        (processes, unlisted)
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

/// A holder, or a socket, no record names, to be ended.
struct Candidate {
    pane: Option<String>,
    socket: PathBuf,
    holder: Option<Identity>,
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
            Self::Unconfirmed(_) => "start_unconfirmed",
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Named {
    Yes,
    No,
    /// The record cannot be read: it may name it.
    Unknown,
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

/// The word after `flag` in `arguments`.
fn value_after<'a>(arguments: &'a [OsString], flag: &str) -> Option<&'a OsString> {
    arguments
        .iter()
        .position(|word| word == flag)
        .and_then(|at| arguments.get(at + 1))
}

/// The first line `reader` gives within `limit`, without its line end.
fn first_line(reader: impl std::io::Read + Send + 'static, limit: Duration) -> Option<String> {
    let (sender, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = BufReader::new(reader).read_line(&mut line);
        let _ = sender.send(
            read.ok()
                .filter(|count| *count > 0)
                .map(|_| line.trim().to_owned()),
        );
    });
    heard.recv_timeout(limit).ok().flatten()
}

/// The last `limit` bytes of `path`, as text.
fn tail(path: &Path, limit: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(limit)))
        .ok()?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
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
            reserved_at_unix_ms: record::now_unix_ms(),
            boot: panes.boot.clone(),
            holder,
            program: None,
            end: None,
        }
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
