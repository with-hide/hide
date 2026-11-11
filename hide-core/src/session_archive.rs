//! Session archive (PRD session-archive): the sessions the operator put
//! away with `세션 보관`, each root with the descendants that went with it.
//!
//! This module remembers; it closes and starts nothing. The runtime's
//! `runtime/session_archive.rs` saves a record, waits for the coalesced
//! writer's receipt, and only then closes the panes through the subtree
//! close (D-02, B10). A record lives in `core-state.json` beside the
//! dormant sleep records and is counted apart from them (D-18).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent_sleep::DormantRegistration;
use crate::model::PrState;
use crate::sidebar::SessionAgentSessionPayload;

/// Sessions the archive holds, each descendant counting (D-18).
pub const MAX_ARCHIVED_SESSIONS: usize = 256;
/// The kept last line's length in characters (D-21).
pub const LAST_LINE_LIMIT: usize = 500;
/// How long a started resume may take before its agent reports the saved
/// conversation; past it the resume is failed and retried in the same tab.
pub const RESUME_CONFIRM_TIMEOUT_MS: u64 = 180_000;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ArchiveId(String);

impl ArchiveId {
    pub(crate) fn new() -> Result<Self, &'static str> {
        // An intent identity reused by every retry; never an address.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let sequence = SEQUENCE
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| value.checked_add(1),
            )
            .map_err(|_| "Archive identity capacity is exhausted")?;
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System time cannot establish an archive identity")?
            .as_nanos();
        Ok(Self(format!("archive-{time:x}-{sequence:x}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id a shell event names, refused when it is not one.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let valid = value.strip_prefix("archive-").is_some_and(|suffix| {
            let mut parts = suffix.split('-');
            let hex = |part: Option<&str>| {
                part.is_some_and(|part| {
                    !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            };
            hex(parts.next()) && hex(parts.next()) && parts.next().is_none()
        });
        (valid && (10..=80).contains(&value.len())).then(|| Self(value.to_owned()))
    }
}

impl<'de> Deserialize<'de> for ArchiveId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| serde::de::Error::custom("invalid archive identity"))
    }
}

/// Where the close of an archive stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePhase {
    /// Saved to the writer, whose receipt starts the close.
    Saving,
    /// The subtree close is running.
    Closing,
    /// A pane refused, timed out or its device left; the root is still
    /// open, and `세션 보관` again converges on this record (B10, B11).
    CloseFailed,
    /// Hide restarted while saving or closing; nothing is repeated until
    /// the operator checks its status (B34).
    CloseUnknown,
    /// Every pane closed: the root is under `보관됨`. A conversation
    /// `hide agent end` kept is archived as its pane closes, and its member
    /// closes once the pane left its machine's listing.
    Archived,
}

impl ArchivePhase {
    pub fn in_flight(self) -> bool {
        matches!(self, Self::Saving | Self::Closing)
    }
}

/// Where one member's resume stands (B21-B26).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Resume {
    /// Archived, nothing asked.
    Idle,
    /// Asked, waiting for the one start slot; `front` takes the keyboard
    /// when its tab opens (the first one asked, B21).
    Queued { front: bool },
    /// Its start-in-checkout task is running.
    Starting {
        task: u64,
        since_unix_ms: u64,
        front: bool,
    },
    /// Its agent started in `pane_id`; waiting for the row to report the
    /// saved conversation.
    Confirming { pane_id: String, since_unix_ms: u64 },
    /// The resume failed; `pane_id` is the tab it made, which a retry
    /// reuses (B26).
    Failed {
        reason: String,
        #[serde(default)]
        pane_id: Option<String>,
    },
    /// Hide restarted or lost the start's answer (B34).
    Unknown {
        #[serde(default)]
        pane_id: Option<String>,
    },
    /// The conversation is live again in `pane_id`; kept only while its
    /// delivery registration moves over.
    Done { pane_id: String },
}

impl Resume {
    pub fn in_flight(&self) -> bool {
        matches!(
            self,
            Self::Queued { .. } | Self::Starting { .. } | Self::Confirming { .. }
        )
    }

    /// The tab an earlier attempt made, which the next one reuses.
    pub fn made_pane(&self) -> Option<&str> {
        match self {
            Self::Confirming { pane_id, .. } | Self::Done { pane_id } => Some(pane_id),
            Self::Failed { pane_id, .. } | Self::Unknown { pane_id } => pane_id.as_deref(),
            Self::Idle | Self::Queued { .. } | Self::Starting { .. } => None,
        }
    }
}

/// A pull request a member held when it was archived (D-22).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedPull {
    pub url: String,
    pub number: u32,
    pub title: String,
    /// Its state when archived, drawn while the core has no newer read.
    pub state: PrState,
}

/// One archived session: the root or a descendant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedMember {
    /// The machine the session runs on: this core's node or a device id.
    pub machine: String,
    /// The pane it had, which the close follows.
    pub pane_id: String,
    pub kind: String,
    pub native_session_id: String,
    /// The proof reference a natively proven resume reads (`id` or `path`).
    #[serde(default)]
    pub source_reference: Option<SessionAgentSessionPayload>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub checkout_path: String,
    pub title: String,
    #[serde(default)]
    pub last_line: Option<String>,
    #[serde(default)]
    pub pulls: Vec<ArchivedPull>,
    #[serde(default)]
    pub registration: Option<DormantRegistration>,
    /// The parent's native session inside this record; `None` for the root.
    #[serde(default)]
    pub parent: Option<String>,
    pub depth: u8,
    /// Its pane is gone from its machine's listing; only a closed member
    /// resumes, and a live row of its conversation then is its resume.
    #[serde(default)]
    pub closed: bool,
    pub resume: Resume,
    /// The profile it ran with, which its resume runs it with again (PRD
    /// agent-profiles D-27); none, as in a record from before profiles, for
    /// the built-in default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::launch::LaunchProfile>,
}

impl ArchivedMember {
    /// Whether a live row is this session's conversation.
    pub fn is(&self, machine: &str, kind: &str, session: &str) -> bool {
        self.machine == machine
            && self.native_session_id == session
            && hide_agent_adapter::canonical_kind(&self.kind)
                == hide_agent_adapter::canonical_kind(kind)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveRecord {
    pub phase: ArchivePhase,
    pub revision: u64,
    pub archived_at_unix_ms: u64,
    /// The project the root belongs to, by repository root path.
    pub project_root: String,
    pub project_label: String,
    /// The root first, then its descendants deepest last.
    pub members: Vec<ArchivedMember>,
    #[serde(default)]
    pub reason: Option<String>,
    /// The root's ★, kept while archived and given back on resume (PRD
    /// revisit-group D-05, B15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starred_at_unix_ms: Option<u64>,
    /// A delete whose link-record lines are gone and whose removal waits
    /// for the writer's receipt; never written, so the save leaves it out.
    #[serde(skip)]
    pub deleting: bool,
}

impl ArchiveRecord {
    pub fn root(&self) -> &ArchivedMember {
        &self.members[0]
    }

    /// The members still archived: not resumed, not left open by a failed
    /// close.
    pub fn held(&self) -> impl Iterator<Item = &ArchivedMember> {
        self.members
            .iter()
            .filter(|member| !matches!(member.resume, Resume::Done { .. }))
    }

    /// Whether the root itself is archived, so the row stands in `보관됨`;
    /// otherwise its archived members show in the live root's record.
    pub fn root_archived(&self) -> bool {
        self.phase == ArchivePhase::Archived && !matches!(self.root().resume, Resume::Done { .. })
    }

    /// Whether nothing is under way on this record, so it may go to make
    /// room for a newer one: archived whole, every member closed and none
    /// resumed or being resumed, and no delete out. A record still saving
    /// or closing, one whose close failed or is unknown, and the record of
    /// an agent end whose pane has not left yet are all still archiving.
    fn at_rest(&self) -> bool {
        self.phase == ArchivePhase::Archived
            && !self.deleting
            && self
                .members
                .iter()
                .all(|member| member.closed && member.resume == Resume::Idle)
    }

    fn validate(&self) -> Result<(), &'static str> {
        let root = self
            .members
            .first()
            .ok_or("archive record has no members")?;
        if root.parent.is_some() || root.depth != 0 {
            return Err("archive record does not start at its root");
        }
        if self.members.len() > MAX_ARCHIVED_SESSIONS {
            return Err("archive record exceeds the archive");
        }
        for member in &self.members {
            if member.native_session_id.is_empty()
                || member.kind.is_empty()
                || member.machine.is_empty()
                || member.checkout_path.is_empty()
            {
                return Err("archive member is incomplete");
            }
            if member
                .last_line
                .as_ref()
                .is_some_and(|line| line.chars().count() > LAST_LINE_LIMIT)
            {
                return Err("archive member's last line is too long");
            }
        }
        Ok(())
    }

    /// What a restart leaves of a record: an unfinished close or resume is
    /// unknown until checked, and nothing is repeated by itself (B34).
    fn after_load(&mut self) {
        if self.phase.in_flight() {
            self.phase = ArchivePhase::CloseUnknown;
        }
        for member in &mut self.members {
            if member.resume.in_flight() {
                let pane_id = member.resume.made_pane().map(str::to_owned);
                member.resume = Resume::Unknown { pane_id };
            }
        }
    }
}

/// The archive kept in `core-state.json`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionArchive {
    pub records: BTreeMap<ArchiveId, ArchiveRecord>,
}

impl Serialize for SessionArchive {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // A record whose delete waits on this very save is left out of it.
        serializer.collect_map(self.records.iter().filter(|(_, record)| !record.deleting))
    }
}

impl<'de> Deserialize<'de> for SessionArchive {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // One unreadable record is dropped with a diagnostic rather than
        // emptying the archive, as the dormant store does.
        let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let mut parsed = Vec::new();
        for (id, value) in raw {
            let read = ArchiveId::parse(&id)
                .ok_or("invalid archive identity")
                .and_then(|id| {
                    let record = serde_json::from_value::<ArchiveRecord>(value)
                        .map_err(|_| "unreadable archive record")?;
                    record.validate()?;
                    Ok((id, record))
                });
            match read {
                Ok(read) => parsed.push(read),
                Err(reason) => crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.load_dropped",
                    "reason": reason,
                })),
            }
        }
        // Past the cap, as a restart between an admission and the delete it
        // started can leave it, the newest records are kept. Counted as
        // `admit` counts: the sessions still archived.
        parsed.sort_by(|(a, left), (b, right)| {
            right
                .archived_at_unix_ms
                .cmp(&left.archived_at_unix_ms)
                .then_with(|| b.cmp(a))
        });
        let mut records = BTreeMap::new();
        let mut members = 0usize;
        for (id, record) in parsed {
            if members + record.held().count() <= MAX_ARCHIVED_SESSIONS {
                members += record.held().count();
                records.insert(id, record);
            } else {
                crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.load_over_cap",
                    "archive_id": id.as_str(),
                }));
            }
        }
        Ok(Self { records })
    }
}

impl SessionArchive {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn after_load(&mut self) {
        for record in self.records.values_mut() {
            record.after_load();
        }
    }

    /// Sessions held now, each member counting (D-18).
    pub fn count(&self) -> usize {
        self.records
            .values()
            .map(|record| record.held().count())
            .sum()
    }

    /// Admits a new record, making room from the oldest records at rest
    /// when it would take the archive past its cap (operator decision
    /// 2026-11-11, replacing B12's refusal). `leaving` names the records
    /// whose delete is already out: they count for nothing and are never
    /// picked again. `pinned` names the records something else still needs
    /// (an open mission or a plan that waits), which a delete would take
    /// with it, so only the operator's own delete removes them. The answer
    /// is the records to delete, oldest first, which the caller deletes
    /// through the archive's own delete; it is refused, with nothing
    /// admitted, only when what is at rest cannot make room.
    pub fn admit(
        &mut self,
        id: ArchiveId,
        record: ArchiveRecord,
        leaving: &dyn Fn(&ArchiveId) -> bool,
        pinned: &dyn Fn(&ArchiveRecord) -> bool,
    ) -> Result<Vec<ArchiveId>, AdmitRefusal> {
        record.validate().map_err(AdmitRefusal::Fault)?;
        let staying = self
            .records
            .iter()
            .filter(|(held, kept)| **held != id && !kept.deleting && !leaving(held))
            .collect::<Vec<_>>();
        let mut over = (staying
            .iter()
            .map(|(_, kept)| kept.held().count())
            .sum::<usize>()
            + record.members.len())
        .saturating_sub(MAX_ARCHIVED_SESSIONS);
        let mut oldest = staying
            .into_iter()
            .filter(|(_, kept)| kept.at_rest())
            .collect::<Vec<_>>();
        oldest.sort_by_key(|(held, kept)| (kept.archived_at_unix_ms, *held));
        let mut evicted = Vec::new();
        for (held, kept) in oldest {
            if over == 0 {
                break;
            }
            if pinned(kept) {
                continue;
            }
            over = over.saturating_sub(kept.held().count());
            evicted.push(held.clone());
        }
        if over > 0 {
            return Err(AdmitRefusal::Full);
        }
        self.records.insert(id, record);
        Ok(evicted)
    }

    /// The record and member holding this conversation, if any.
    pub fn find(&self, machine: &str, kind: &str, session: &str) -> Option<(&ArchiveId, usize)> {
        self.records.iter().find_map(|(id, record)| {
            record
                .members
                .iter()
                .position(|member| member.is(machine, kind, session))
                .map(|index| (id, index))
        })
    }

    /// The delivery registrations archived sessions hold for their resume,
    /// so the absence of their panes ends none of them.
    pub fn held_registrations(&self) -> impl Iterator<Item = &DormantRegistration> {
        // A resumed member keeps holding it until the rebind lands.
        self.records
            .values()
            .flat_map(|record| &record.members)
            .filter_map(|member| member.registration.as_ref())
    }

    /// Drops the members whose link-record lines were pruned (B31): a root
    /// takes its record with it, a descendant leaves alone.
    pub fn drop_pruned(&mut self, pruned: &[(String, String, String)]) -> bool {
        // The record names an agent by its session format, which several
        // kinds share; the conversation id on its machine is the identity.
        let gone = |member: &ArchivedMember| {
            pruned
                .iter()
                .any(|(device, _, id)| member.machine == *device && member.native_session_id == *id)
        };
        let before = self.clone();
        self.records.retain(|_, record| {
            if record.phase.in_flight() || record.root().resume.in_flight() {
                return true;
            }
            if gone(record.root()) {
                return false;
            }
            record
                .members
                .retain(|member| member.resume.in_flight() || !gone(member));
            true
        });
        *self != before
    }
}

/// Why the archive did not take a record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmitRefusal {
    /// It would hold more than [`MAX_ARCHIVED_SESSIONS`] sessions, and
    /// every record that could make room is being archived, resumed or
    /// deleted.
    Full,
    /// Hide could not form the record (its shape or its identity): a fault
    /// of Hide's, never the operator's.
    Fault(&'static str),
}

/// The refusal of an archive whose every record is busy, so none could make
/// room.
pub const ARCHIVE_FULL: &str = "The archive is full and every archived session is being archived, resumed or deleted; archive again once they finish. Nothing was closed.";

/// One archived root as the 태스크 list and the record draw it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchivedSessionSnapshot {
    pub id: String,
    pub device_id: String,
    pub project_root: String,
    pub project_label: String,
    pub checkout_path: String,
    /// `archiving`, `close_failed`, `unknown`, `archived` or `deleting`.
    pub phase: &'static str,
    /// Whether this row stands in `보관됨`; otherwise the root is live and
    /// the archived members show in its record (B24).
    pub listed: bool,
    pub archived_at_unix_ms: u64,
    pub reason: Option<String>,
    /// The root carries the operator's ★ (PRD revisit-group B15).
    pub starred: bool,
    pub members: Vec<ArchivedMemberSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchivedMemberSnapshot {
    pub session: crate::links::ledger::SessionIdentity,
    pub agent_kind: String,
    pub title: String,
    pub depth: u8,
    pub parent_session_id: Option<String>,
    pub last_line: Option<String>,
    pub pulls: Vec<ArchivedPullSnapshot>,
    /// `archived`, `queued`, `starting`, `failed`, `unknown` or `resumed`.
    pub resume: &'static str,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchivedPullSnapshot {
    pub url: String,
    pub number: u32,
    pub title: String,
    pub state: PrState,
}

impl ArchiveRecord {
    /// The record's row. `current` answers a pull request's state now from
    /// the core's own reads, or `None` when it has no newer one (D-22).
    pub fn snapshot(
        &self,
        id: &ArchiveId,
        current: &dyn Fn(&str) -> Option<PrState>,
    ) -> ArchivedSessionSnapshot {
        let root = self.root();
        let phase = if self.deleting {
            "deleting"
        } else {
            match self.phase {
                ArchivePhase::Saving | ArchivePhase::Closing => "archiving",
                ArchivePhase::CloseFailed => "close_failed",
                ArchivePhase::CloseUnknown => "unknown",
                ArchivePhase::Archived => "archived",
            }
        };
        ArchivedSessionSnapshot {
            id: id.as_str().to_owned(),
            device_id: root.machine.clone(),
            project_root: self.project_root.clone(),
            project_label: self.project_label.clone(),
            checkout_path: root.checkout_path.clone(),
            phase,
            listed: self.root_archived() || self.phase == ArchivePhase::CloseUnknown,
            archived_at_unix_ms: self.archived_at_unix_ms,
            reason: self.reason.clone(),
            starred: self.starred_at_unix_ms.is_some(),
            members: self
                .members
                .iter()
                .filter(|member| {
                    // A member a failed close left open is a live session.
                    member.closed || self.phase != ArchivePhase::CloseFailed
                })
                .map(|member| {
                    let (resume, reason) = match &member.resume {
                        Resume::Idle => ("archived", None),
                        Resume::Queued { .. } => ("queued", None),
                        Resume::Starting { .. } | Resume::Confirming { .. } => ("starting", None),
                        Resume::Failed { reason, .. } => ("failed", Some(reason.clone())),
                        Resume::Unknown { .. } => ("unknown", None),
                        Resume::Done { .. } => ("resumed", None),
                    };
                    ArchivedMemberSnapshot {
                        session: crate::links::ledger::SessionIdentity {
                            device_id: member.machine.clone(),
                            agent: member.kind.clone(),
                            session_id: member.native_session_id.clone(),
                        },
                        agent_kind: member.kind.clone(),
                        title: member.title.clone(),
                        depth: member.depth,
                        parent_session_id: member.parent.clone(),
                        last_line: member.last_line.clone(),
                        pulls: member
                            .pulls
                            .iter()
                            .map(|pull| ArchivedPullSnapshot {
                                url: pull.url.clone(),
                                number: pull.number,
                                title: pull.title.clone(),
                                state: current(&pull.url).unwrap_or(pull.state),
                            })
                            .collect(),
                        resume,
                        reason,
                    }
                })
                .collect(),
        }
    }
}

impl SessionArchive {
    /// Every record's row, newest archived first (D-09).
    pub fn snapshots(
        &self,
        current: &dyn Fn(&str) -> Option<PrState>,
    ) -> Vec<ArchivedSessionSnapshot> {
        let mut rows = self
            .records
            .iter()
            .map(|(id, record)| (record.archived_at_unix_ms, record.snapshot(id, current)))
            .collect::<Vec<_>>();
        rows.sort_by(|(a, left), (b, right)| b.cmp(a).then_with(|| left.id.cmp(&right.id)));
        rows.into_iter().map(|(_, row)| row).collect()
    }
}

/// The last non-empty line of an agent's message, cut to the kept length
/// (D-21).
pub fn last_line(message: Option<&str>) -> Option<String> {
    let line = message?
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())?;
    Some(line.chars().take(LAST_LINE_LIMIT).collect())
}

#[cfg(test)]
pub(crate) fn fixture_member(session: &str, parent: Option<&str>) -> ArchivedMember {
    ArchivedMember {
        machine: "node-a".into(),
        pane_id: format!("w1:{session}"),
        kind: "claude".into(),
        native_session_id: session.into(),
        source_reference: None,
        cwd: Some("/repo".into()),
        checkout_path: "/repo".into(),
        title: format!("Task {session}"),
        last_line: Some("Done.".into()),
        pulls: Vec::new(),
        registration: None,
        parent: parent.map(str::to_owned),
        depth: u8::from(parent.is_some()),
        closed: true,
        resume: Resume::Idle,
        profile: None,
    }
}

#[cfg(test)]
pub(crate) fn fixture_record(members: Vec<ArchivedMember>, at: u64) -> ArchiveRecord {
    ArchiveRecord {
        phase: ArchivePhase::Archived,
        revision: 1,
        archived_at_unix_ms: at,
        project_root: "/repo".into(),
        project_label: "repo".into(),
        members,
        reason: None,
        starred_at_unix_ms: None,
        deleting: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> ArchiveId {
        ArchiveId(format!("archive-{n:x}-1"))
    }

    #[test]
    fn archived_rows_list_newest_archived_first_with_one_row_per_root() {
        let mut archive = SessionArchive::default();
        archive
            .admit(
                id(1),
                fixture_record(vec![fixture_member("a", None)], 10),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        archive
            .admit(
                id(2),
                fixture_record(
                    vec![fixture_member("b", None), fixture_member("c", Some("b"))],
                    30,
                ),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        archive
            .admit(
                id(3),
                fixture_record(vec![fixture_member("d", None)], 20),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        let rows = archive.snapshots(&|_| None);
        let order = rows
            .iter()
            .map(|row| row.members[0].session.session_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(order, ["b", "d", "a"]);
        assert_eq!(rows[0].members.len(), 2);
        assert_eq!(archive.count(), 4);
    }

    /// `n` archived records of one session each, archived at 1..=n, so
    /// record 1 is the oldest.
    fn full_of_singles(n: usize) -> SessionArchive {
        let mut archive = SessionArchive::default();
        for at in 1..=n as u64 {
            let record = fixture_record(vec![fixture_member(&format!("s{at}"), None)], at);
            assert_eq!(
                archive.admit(id(at), record, &|_| false, &|_| false),
                Ok(Vec::new())
            );
        }
        archive
    }

    #[test]
    fn a_full_archive_makes_room_from_its_oldest_records_at_rest() {
        let mut archive = full_of_singles(MAX_ARCHIVED_SESSIONS);
        assert_eq!(archive.count(), MAX_ARCHIVED_SESSIONS);
        let two = vec![fixture_member("x", None), fixture_member("y", Some("x"))];
        assert_eq!(
            archive.admit(id(1000), fixture_record(two, 1000), &|_| false, &|_| false),
            Ok(vec![id(1), id(2)]),
            "the two oldest go for a record of two"
        );
        assert!(archive.records.contains_key(&id(1000)));
        // The caller deletes them; until then they are still held.
        assert!(archive.records.contains_key(&id(1)));
    }

    #[test]
    fn a_tree_is_evicted_whole_and_the_age_decides_not_the_identity() {
        let mut archive = full_of_singles(MAX_ARCHIVED_SESSIONS - 3);
        let tree = vec![
            fixture_member("t0", None),
            fixture_member("t1", Some("t0")),
            fixture_member("t2", Some("t1")),
        ];
        // The highest identity, archived before every other record.
        archive
            .admit(id(5000), fixture_record(tree, 0), &|_| false, &|_| false)
            .unwrap();
        assert_eq!(archive.count(), MAX_ARCHIVED_SESSIONS);
        assert_eq!(
            archive.admit(
                id(6000),
                fixture_record(vec![fixture_member("n", None)], 6000),
                &|_| false,
                &|_| false
            ),
            Ok(vec![id(5000)]),
            "one eviction frees all three of its sessions"
        );
    }

    #[test]
    fn a_record_being_resumed_deleted_or_still_archiving_is_never_evicted() {
        let mut archive = full_of_singles(MAX_ARCHIVED_SESSIONS);
        let busy: [(u64, fn(&mut ArchiveRecord)); 6] = [
            (1, |record| {
                record.members[0].resume = Resume::Queued { front: true }
            }),
            (2, |record| record.phase = ArchivePhase::Closing),
            (3, |record| record.phase = ArchivePhase::CloseUnknown),
            // An agent end whose pane has not left its machine yet.
            (4, |record| record.members[0].closed = false),
            (5, |record| {
                record.members[0].resume = Resume::Failed {
                    reason: "start refused".into(),
                    pane_id: Some("w3:p1".into()),
                }
            }),
            (6, |record| record.deleting = true),
        ];
        for (at, make_busy) in busy {
            make_busy(archive.records.get_mut(&id(at)).unwrap());
        }
        let leaving = |held: &ArchiveId| *held == id(7);
        assert_eq!(
            archive.admit(
                id(1000),
                fixture_record(vec![fixture_member("n", None)], 1000),
                &leaving,
                &|_| false
            ),
            Ok(Vec::new()),
            "the records already being deleted make the room"
        );
        let three = (0..3)
            .map(|n| fixture_member(&format!("m{n}"), (n > 0).then_some("m0")))
            .collect();
        assert_eq!(
            archive.admit(id(1001), fixture_record(three, 1001), &leaving, &|_| false),
            Ok(vec![id(8), id(9)]),
            "the oldest at rest after the busy ones"
        );
    }

    #[test]
    fn a_pinned_record_is_passed_over_for_the_next_oldest() {
        let mut archive = full_of_singles(MAX_ARCHIVED_SESSIONS);
        let pinned = |record: &ArchiveRecord| record.root().native_session_id == "s1";
        assert_eq!(
            archive.admit(
                id(1000),
                fixture_record(vec![fixture_member("n", None)], 1000),
                &|_| false,
                &pinned,
            ),
            Ok(vec![id(2)])
        );
        let mut only_pinned = SessionArchive::default();
        let members = (0..MAX_ARCHIVED_SESSIONS)
            .map(|n| fixture_member(&format!("s{n}"), (n > 0).then_some("s0")))
            .collect();
        only_pinned
            .admit(id(1), fixture_record(members, 1), &|_| false, &|_| false)
            .unwrap();
        assert_eq!(
            only_pinned.admit(
                id(2),
                fixture_record(vec![fixture_member("n", None)], 2),
                &|_| false,
                &|_| true,
            ),
            Err(AdmitRefusal::Full)
        );
    }

    #[test]
    fn an_archive_is_refused_only_when_nothing_at_rest_can_make_room() {
        let mut archive = SessionArchive::default();
        let members = (0..MAX_ARCHIVED_SESSIONS - 1)
            .map(|n| fixture_member(&format!("s{n}"), (n > 0).then_some("s0")))
            .collect();
        let mut resuming = fixture_record(members, 1);
        resuming.members[0].resume = Resume::Starting {
            task: 1,
            since_unix_ms: 1,
            front: true,
        };
        archive
            .admit(id(1), resuming, &|_| false, &|_| false)
            .unwrap();
        let two = vec![fixture_member("x", None), fixture_member("y", Some("x"))];
        assert_eq!(
            archive.admit(id(2), fixture_record(two, 2), &|_| false, &|_| false),
            Err(AdmitRefusal::Full)
        );
        assert_eq!(archive.records.len(), 1, "nothing admitted");
        assert_eq!(
            archive.admit(
                id(3),
                fixture_record(vec![fixture_member("z", None)], 3),
                &|_| false,
                &|_| false
            ),
            Ok(Vec::new()),
            "what fits is admitted"
        );
        assert_eq!(archive.count(), MAX_ARCHIVED_SESSIONS);
    }

    #[test]
    fn a_load_past_the_cap_keeps_the_newest_records() {
        let mut archive = full_of_singles(MAX_ARCHIVED_SESSIONS);
        archive.records.insert(
            id(1000),
            fixture_record(vec![fixture_member("newest", None)], 1000),
        );
        let loaded =
            serde_json::from_str::<SessionArchive>(&serde_json::to_string(&archive).unwrap())
                .unwrap();
        assert_eq!(loaded.count(), MAX_ARCHIVED_SESSIONS);
        assert!(loaded.records.contains_key(&id(1000)));
        assert!(
            !loaded.records.contains_key(&id(1)),
            "the oldest is dropped"
        );
    }

    #[test]
    fn a_restart_turns_an_unfinished_close_and_resume_into_unknown() {
        let mut record = fixture_record(
            vec![fixture_member("a", None), fixture_member("b", Some("a"))],
            1,
        );
        record.phase = ArchivePhase::Closing;
        record.members[1].resume = Resume::Confirming {
            pane_id: "w2:p1".into(),
            since_unix_ms: 5,
        };
        let mut archive = SessionArchive::default();
        archive
            .admit(id(1), record, &|_| false, &|_| false)
            .unwrap();
        let text = serde_json::to_string(&archive).unwrap();
        let mut loaded = serde_json::from_str::<SessionArchive>(&text).unwrap();
        loaded.after_load();
        let record = &loaded.records[&id(1)];
        assert_eq!(record.phase, ArchivePhase::CloseUnknown);
        assert_eq!(
            record.members[1].resume,
            Resume::Unknown {
                pane_id: Some("w2:p1".into())
            }
        );
    }

    #[test]
    fn a_record_whose_delete_waits_on_the_save_is_not_written() {
        let mut archive = SessionArchive::default();
        archive
            .admit(
                id(1),
                fixture_record(vec![fixture_member("a", None)], 1),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        archive
            .admit(
                id(2),
                fixture_record(vec![fixture_member("b", None)], 2),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        archive.records.get_mut(&id(1)).unwrap().deleting = true;
        let loaded =
            serde_json::from_str::<SessionArchive>(&serde_json::to_string(&archive).unwrap())
                .unwrap();
        assert_eq!(loaded.records.keys().collect::<Vec<_>>(), [&id(2)]);
    }

    #[test]
    fn a_pruned_root_takes_its_record_and_a_pruned_descendant_leaves_alone() {
        let mut archive = SessionArchive::default();
        archive
            .admit(
                id(1),
                fixture_record(
                    vec![fixture_member("a", None), fixture_member("b", Some("a"))],
                    1,
                ),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        archive
            .admit(
                id(2),
                fixture_record(vec![fixture_member("c", None)], 2),
                &|_| false,
                &|_| false,
            )
            .unwrap();
        let key = |id: &str| ("node-a".to_owned(), "claude".to_owned(), id.to_owned());
        assert!(archive.drop_pruned(&[key("b"), key("c")]));
        assert_eq!(archive.records.len(), 1);
        assert_eq!(archive.records[&id(1)].members.len(), 1);
        assert!(!archive.drop_pruned(&[key("zzz")]));
    }

    #[test]
    fn the_last_line_is_the_last_non_empty_line_cut_to_its_limit() {
        assert_eq!(
            last_line(Some("first\n\nsecond  \n  \n")),
            Some("second".into())
        );
        assert_eq!(last_line(Some("  \n")), None);
        let long = "가".repeat(LAST_LINE_LIMIT + 10);
        assert_eq!(
            last_line(Some(&long)).unwrap().chars().count(),
            LAST_LINE_LIMIT
        );
    }
}
