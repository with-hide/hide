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
    /// Every pane closed: the root is under `보관됨`.
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
    #[serde(default)]
    pub closed: bool,
    pub resume: Resume,
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
        let mut records = BTreeMap::new();
        let mut members = 0usize;
        for (id, value) in raw {
            let parsed = ArchiveId::parse(&id)
                .ok_or("invalid archive identity")
                .and_then(|id| {
                    let record = serde_json::from_value::<ArchiveRecord>(value)
                        .map_err(|_| "unreadable archive record")?;
                    record.validate()?;
                    Ok((id, record))
                });
            match parsed {
                Ok((id, record)) if members + record.members.len() <= MAX_ARCHIVED_SESSIONS => {
                    members += record.members.len();
                    records.insert(id, record);
                }
                Ok(_) => crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.load_over_cap",
                })),
                Err(reason) => crate::diagnostic!(serde_json::json!({
                    "component": "session_archive", "kind": "session_archive.load_dropped",
                    "reason": reason,
                })),
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

    /// Admits a new record, or refuses before anything closes when it would
    /// take the archive past its cap; nothing is evicted (B12).
    pub fn admit(&mut self, id: ArchiveId, record: ArchiveRecord) -> Result<(), &'static str> {
        record.validate()?;
        let others = self
            .records
            .iter()
            .filter(|(held, _)| **held != id)
            .map(|(_, record)| record.held().count())
            .sum::<usize>();
        if others + record.members.len() > MAX_ARCHIVED_SESSIONS {
            return Err(ARCHIVE_FULL);
        }
        self.records.insert(id, record);
        Ok(())
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

/// The refusal that only deleting archived sessions cures (B12).
pub const ARCHIVE_FULL: &str =
    "The archive is full; delete archived sessions first. Nothing was closed.";

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
            .admit(id(1), fixture_record(vec![fixture_member("a", None)], 10))
            .unwrap();
        archive
            .admit(
                id(2),
                fixture_record(
                    vec![fixture_member("b", None), fixture_member("c", Some("b"))],
                    30,
                ),
            )
            .unwrap();
        archive
            .admit(id(3), fixture_record(vec![fixture_member("d", None)], 20))
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

    #[test]
    fn an_archive_past_the_cap_is_refused_and_nothing_is_evicted() {
        let mut archive = SessionArchive::default();
        let members = (0..MAX_ARCHIVED_SESSIONS - 1)
            .map(|n| fixture_member(&format!("s{n}"), (n > 0).then_some("s0")))
            .collect();
        archive.admit(id(1), fixture_record(members, 1)).unwrap();
        let two = vec![fixture_member("x", None), fixture_member("y", Some("x"))];
        assert_eq!(
            archive.admit(id(2), fixture_record(two, 2)),
            Err(ARCHIVE_FULL)
        );
        assert_eq!(archive.count(), MAX_ARCHIVED_SESSIONS - 1);
        archive
            .admit(id(3), fixture_record(vec![fixture_member("z", None)], 3))
            .unwrap();
        assert_eq!(archive.count(), MAX_ARCHIVED_SESSIONS);
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
        archive.admit(id(1), record).unwrap();
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
            .admit(id(1), fixture_record(vec![fixture_member("a", None)], 1))
            .unwrap();
        archive
            .admit(id(2), fixture_record(vec![fixture_member("b", None)], 2))
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
            )
            .unwrap();
        archive
            .admit(id(2), fixture_record(vec![fixture_member("c", None)], 2))
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
