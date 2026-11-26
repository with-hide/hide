//! The sessions Desk's conversation names, kept past their panes (feedback
//! 2, frame K). A chip names a session by its pane, and a pane's id says
//! nothing once the pane is gone, so Desk's store keeps, for each pane the
//! newest lines' chips name, the session it ran and where its thread is
//! read (`crate::desk::NamedSession`, at most
//! [`crate::desk::MAX_NAMED_SESSIONS`]). A chip whose pane is gone draws
//! that session's record, read from the link record on a worker outside the
//! lock, the read every session record takes (`LedgerCall::Record`), and
//! opens its thread read-only through the same thread reader a live pane's
//! takes (`session_thread.rs`).

use std::collections::{BTreeMap, HashMap, HashSet};

use super::super::Runtime;
use super::jobs::FOLLOWED_CHIPS;
use crate::desk::NamedSession;
use crate::links::ledger::{LedgerAnswer, LedgerCall, SessionIdentity, SessionRecord};
use crate::model::{DeskItem, DeskPullRequest, DeskRecordSnapshot, PrState};

/// The pull requests a record's chip and card count at most.
const RECORD_PULLS: usize = 8;
/// How long a session whose read failed (a timeout, a busy record) waits
/// before it is asked again, on a later compose; each failure in a row
/// doubles it, up to [`RECORD_RETRY_MAX_MS`].
const RECORD_RETRY_MS: u64 = 30_000;
const RECORD_RETRY_MAX_MS: u64 = 15 * 60_000;

/// A session whose last read failed: when it may be asked again, how many
/// failed in a row, and the code last logged, so one cause is logged once.
struct Failed {
    retry_at_ms: u64,
    failures: u32,
    code: String,
}

/// What the link record answered for each named session, and whether a read
/// is out; the runtime's, never stored, and only for sessions still named.
#[derive(Default)]
pub(super) struct Records {
    /// `None` for a session the record does not hold: its chip stays
    /// dimmed, and it is not asked again while Hide runs.
    read: BTreeMap<SessionIdentity, Option<DeskRecordSnapshot>>,
    /// Sessions whose read failed, each waiting on its own; the others are
    /// read meanwhile.
    failed: BTreeMap<SessionIdentity, Failed>,
    reading: bool,
}

/// One answer of the reader.
type Answer = (SessionIdentity, Result<LedgerAnswer, String>);

impl Runtime {
    /// The panes the newest lines' chips name (a reply's, a letter's, the
    /// operator's message), newest first, each once.
    fn desk_chip_panes(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        let mut panes = Vec::new();
        for item in self
            .desk
            .stored
            .conversation
            .items
            .iter()
            .rev()
            .take(FOLLOWED_CHIPS)
        {
            let named: Vec<&str> = match item {
                DeskItem::Handed { pane_id, .. } => vec![pane_id.as_str()],
                DeskItem::Reply { mentions, .. } => {
                    mentions.iter().map(|chip| chip.pane_id.as_str()).collect()
                }
                DeskItem::User { mentions, .. } => mentions
                    .iter()
                    .filter(|mention| mention.kind == crate::model::DeskMentionKind::Session)
                    .map(|mention| mention.id.as_str())
                    .collect(),
                _ => continue,
            };
            panes.extend(named.into_iter().filter(|pane| seen.insert(*pane)));
        }
        panes
    }

    /// Keeps the session each chipped pane runs now, as its row and its
    /// delivery observation say it; whether the store changed, for a save.
    pub(super) fn desk_keep_named(&mut self) -> bool {
        let rows: HashMap<&str, &crate::model::SidebarAgentSnapshot> = self
            .agent_rows()
            .map(|row| (row.pane_id.as_str(), row))
            .collect();
        let found: Vec<NamedSession> = self
            .desk_chip_panes()
            .into_iter()
            .filter_map(|pane| {
                let row = rows.get(pane)?;
                let agent = super::super::links::session_agent(&row.agent_kind)?;
                // The machine as the one address owner reads it from the pane, as a row's is stamped.
                let device_id = crate::address::split_pane(self.node.as_str(), pane)
                    .0
                    .to_owned();
                Some(NamedSession {
                    pane_id: pane.to_owned(),
                    session: SessionIdentity {
                        device_id,
                        agent: agent.to_owned(),
                        session_id: row.session_id.clone()?,
                    },
                    agent_kind: row.agent_kind.clone(),
                    thread: self
                        .delivery_observations
                        .get(pane)
                        .and_then(|observation| observation.thread.clone()),
                })
            })
            .collect();
        self.desk.stored.keep_named(found)
    }

    /// The records of the sessions the newest lines' chips name whose pane
    /// no row holds now, in the chips' order; a session not read yet is
    /// handed to the reader and joins once it answers.
    pub(super) fn desk_records(&mut self) -> Vec<DeskRecordSnapshot> {
        let rows: HashSet<&str> = self.agent_rows().map(|row| row.pane_id.as_str()).collect();
        let mut records = Vec::new();
        let mut unread = Vec::new();
        for pane in self.desk_chip_panes() {
            if rows.contains(pane) {
                continue;
            }
            let Some(named) = self
                .desk
                .stored
                .named
                .iter()
                .find(|named| named.pane_id == pane)
            else {
                continue;
            };
            match self.desk.records.read.get(&named.session) {
                Some(Some(record)) => records.push(DeskRecordSnapshot {
                    pane_id: named.pane_id.clone(),
                    thread: named.thread.is_some(),
                    ..record.clone()
                }),
                Some(None) => {}
                None if self.desk_record_due(&named.session) => unread.push(named.session.clone()),
                None => {}
            }
        }
        self.desk_read_records(unread);
        records
    }

    /// Reads `sessions`' records on a worker, one read out at a time; the
    /// lock is not held while the link record answers.
    fn desk_read_records(&mut self, sessions: Vec<SessionIdentity>) {
        if self.desk.records.reading || sessions.is_empty() {
            return;
        }
        let Some(client) = self.links_work.client.clone() else {
            return;
        };
        self.desk.records.reading = true;
        let asked = sessions.clone();
        let spawned = self.spawn_issue_worker(
            "desk-records",
            move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    sessions
                        .iter()
                        .map(|session| {
                            (
                                session.clone(),
                                client.call(LedgerCall::Record(session.clone())),
                            )
                        })
                        .collect::<Vec<Answer>>()
                }))
                .unwrap_or_else(|_| {
                    asked
                        .into_iter()
                        .map(|session| (session, Err("desk_records_reader_stopped".to_owned())))
                        .collect()
                })
            },
            Runtime::desk_records_read,
        );
        if let Err(error) = spawned {
            self.desk.records.reading = false;
            crate::diagnostic!(serde_json::json!({
                "component": "desk", "kind": "desk.records_worker_unavailable", "error": error,
            }));
        }
    }

    /// The reader's answers, under a short lock: each record becomes what a
    /// chip draws, and the cards are composed again.
    fn desk_records_read(&mut self, answers: Vec<Answer>) -> bool {
        self.desk.records.reading = false;
        for (session, answer) in answers {
            let kind = self
                .desk
                .stored
                .named
                .iter()
                .find(|named| named.session == session)
                .map(|named| named.agent_kind.clone());
            let record = match (answer, kind) {
                (Ok(LedgerAnswer::Record(record)), Some(kind)) => Some(record_facts(&record, kind)),
                (Ok(LedgerAnswer::Record(_)), None) => None,
                (Ok(_), _) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "desk", "kind": "desk.record_answer_unexpected",
                        "session_id": session.session_id,
                    }));
                    None
                }
                // A session the record never held is the dimmed chip, not a failure.
                (Err(code), _) if code == "ledger_session_missing" => None,
                (Err(code), _) => {
                    self.desk_record_failed(session, code);
                    continue;
                }
            };
            self.desk.records.failed.remove(&session);
            self.desk.records.read.insert(session, record);
        }
        let named: HashSet<&SessionIdentity> = self
            .desk
            .stored
            .named
            .iter()
            .map(|named| &named.session)
            .collect();
        self.desk
            .records
            .read
            .retain(|session, _| named.contains(session));
        self.desk
            .records
            .failed
            .retain(|session, _| named.contains(session));
        self.compose_desk_jobs();
        true
    }

    /// Whether a session not read yet may be asked now: one whose last read
    /// failed waits out its own wait, and no other session waits with it.
    fn desk_record_due(&self, session: &SessionIdentity) -> bool {
        self.desk
            .records
            .failed
            .get(session)
            .is_none_or(|failed| crate::links::now_ms() >= failed.retry_at_ms)
    }

    /// A session whose read failed waits longer after each failure in a row,
    /// and its cause is logged once until it changes.
    fn desk_record_failed(&mut self, session: SessionIdentity, code: String) {
        let failed = self
            .desk
            .records
            .failed
            .entry(session.clone())
            .or_insert(Failed {
                retry_at_ms: 0,
                failures: 0,
                code: String::new(),
            });
        if failed.code != code {
            crate::diagnostic!(serde_json::json!({
                "component": "desk", "kind": "desk.record_read_failed",
                "session_id": session.session_id, "code": code,
            }));
            failed.code = code;
        }
        failed.failures = failed.failures.saturating_add(1);
        let wait = RECORD_RETRY_MS
            .saturating_mul(1u64 << (failed.failures - 1).min(10))
            .min(RECORD_RETRY_MAX_MS);
        failed.retry_at_ms = crate::links::now_ms() + wait;
    }

    /// Where the thread of a session a chip names is read once its pane is
    /// gone: its machine and its kept reference.
    pub(in crate::runtime) fn desk_named_thread(
        &self,
        pane: &str,
    ) -> Option<(String, crate::runtime::ThreadReference)> {
        let named = self
            .desk
            .stored
            .named
            .iter()
            .find(|named| named.pane_id == pane)?;
        Some((named.session.device_id.clone(), named.thread.clone()?))
    }
}

/// What a chip draws of a record; its pane and whether a thread is kept are
/// the named session's.
fn record_facts(record: &SessionRecord, agent_kind: String) -> DeskRecordSnapshot {
    let title = record
        .manual_title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| record.title.clone());
    DeskRecordSnapshot {
        pane_id: String::new(),
        session: record.session.clone(),
        agent_kind,
        title,
        started_at_unix_ms: record.started_at_unix_ms,
        ended_at_unix_ms: record.ended_at_unix_ms,
        line: record
            .status
            .as_ref()
            .map(|status| status.line.clone())
            .filter(|line| !line.trim().is_empty()),
        pull_requests: record
            .outputs
            .iter()
            .filter(|output| output.kind == "pr")
            .take(RECORD_PULLS)
            .map(|output| DeskPullRequest {
                number: u32::try_from(output.number).unwrap_or(u32::MAX),
                state: pr_state(&output.state),
            })
            .collect(),
        cwd: record.cwd.clone(),
        path: record.path.clone(),
        project: record.project.clone(),
        thread: false,
    }
}

/// A record's pull request state as a PR mark draws it; an open one whose
/// checks the record does not keep reads pending.
fn pr_state(state: &str) -> PrState {
    match state.to_ascii_lowercase().as_str() {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        "draft" => PrState::Draft,
        _ => PrState::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeskReplyState, DeskSessionChip, ThreadFailure, ThreadPane};
    use serde_json::json;

    const SESSION: &str = "0b9e2a4c-4c0e-4d7a-9a51-2f7f6a1e3c11";

    /// A sidebar row for `pane` whose agent reported `session`.
    fn row(pane: &str, session: &str) -> crate::model::SidebarAgentSnapshot {
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [
                {"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t1",
                 "revision": 1, "focused": false, "agent_status": "done", "agent": "codex",
                 "name": "builder", "cwd": "/checkouts/fixture", "state_change_seq": 1,
                 "agent_session": {"kind": "id", "value": session}},
            ]}))
            .unwrap();
        crate::sidebar::project_agents(payload).agents.remove(0)
    }

    fn reply_naming(runtime: &mut Runtime, panes: &[&str]) {
        let id = runtime.desk.stored.conversation.next_id();
        runtime.desk.stored.conversation.push(DeskItem::Reply {
            id,
            agent: hide_desk::AdapterId::CLAUDE,
            text: String::new(),
            at_ms: 0,
            state: DeskReplyState::Done,
            thought_ms: None,
            steps: Default::default(),
            mentions: panes
                .iter()
                .map(|pane| DeskSessionChip {
                    pane_id: (*pane).into(),
                    name: (*pane).into(),
                    agent_kind: None,
                    mark: None,
                })
                .collect(),
            missions: Vec::new(),
            compaction: None,
        });
    }

    fn records(runtime: &Runtime) -> Vec<DeskRecordSnapshot> {
        runtime.snapshot.desk_jobs.as_ref().unwrap().records.clone()
    }

    #[test]
    fn a_chip_whose_pane_is_gone_draws_its_sessions_record_and_one_the_record_lacks_draws_none() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _orders) = super::super::tests::runtime(root.path());
        runtime.snapshot.navigator.agents = vec![row("p1", SESSION), row("p2", "other")];
        reply_naming(&mut runtime, &["p1", "p2", "p3"]);
        runtime.refresh_desk_jobs();
        let kept: Vec<_> = runtime
            .desk
            .stored
            .named
            .iter()
            .map(|named| {
                (
                    named.pane_id.as_str(),
                    named.session.session_id.as_str(),
                    named.agent_kind.as_str(),
                )
            })
            .collect();
        // p3 never had a row: Hide never knew its session.
        assert_eq!(kept, [("p2", "other", "codex"), ("p1", SESSION, "codex")]);
        assert!(
            records(&runtime).is_empty(),
            "a live pane's chip draws its row"
        );

        runtime.snapshot.navigator.agents.clear();
        runtime.refresh_desk_jobs();
        let session = |id: &str| {
            runtime
                .desk
                .stored
                .named
                .iter()
                .find(|named| named.session.session_id == id)
                .unwrap()
                .session
                .clone()
        };
        let (built, missing) = (session(SESSION), session("other"));
        let record = SessionRecord {
            session: built.clone(),
            title: "1.3.0 후보 빌드 만들기".into(),
            started_at_unix_ms: Some(10),
            ended_at_unix_ms: Some(20),
            cwd: Some("/checkouts/fixture".into()),
            ..SessionRecord::default()
        };
        runtime.desk_records_read(vec![
            (built.clone(), Ok(LedgerAnswer::Record(Box::new(record)))),
            (missing, Err("ledger_session_missing".into())),
        ]);
        let drawn = records(&runtime);
        assert_eq!(
            drawn.len(),
            1,
            "a session the record lacks stays the dimmed chip"
        );
        assert_eq!(
            (
                drawn[0].pane_id.as_str(),
                drawn[0].title.as_str(),
                drawn[0].agent_kind.as_str(),
                drawn[0].ended_at_unix_ms
            ),
            ("p1", "1.3.0 후보 빌드 만들기", "codex", Some(20))
        );
        assert_eq!(drawn[0].session, built);

        // Its pane back (a session woken in it), the chip draws the row again.
        runtime.snapshot.navigator.agents = vec![row("p1", SESSION)];
        runtime.refresh_desk_jobs();
        assert!(records(&runtime).is_empty());
    }

    /// A read that failed (a timeout, a busy record) is not taken for "no
    /// record": that session alone waits, longer after each failure in a row
    /// up to a cap, while another chip's session is read, and it draws once
    /// it answers.
    #[test]
    fn a_record_read_that_failed_waits_alone_longer_each_time_and_draws_once_it_answers() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _orders) = super::super::tests::runtime(root.path());
        runtime.snapshot.navigator.agents = vec![row("p1", SESSION), row("p2", "other")];
        reply_naming(&mut runtime, &["p1", "p2"]);
        runtime.refresh_desk_jobs();
        runtime.snapshot.navigator.agents.clear();
        runtime.refresh_desk_jobs();
        let session = |runtime: &Runtime, id: &str| {
            runtime
                .desk
                .stored
                .named
                .iter()
                .find(|named| named.session.session_id == id)
                .unwrap()
                .session
                .clone()
        };
        let (built, other) = (session(&runtime, SESSION), session(&runtime, "other"));
        let wait = |runtime: &Runtime| {
            runtime.desk.records.failed[&built]
                .retry_at_ms
                .saturating_sub(crate::links::now_ms())
        };
        runtime.desk_records_read(vec![(built.clone(), Err("ledger_rpc_capacity".into()))]);
        assert!(records(&runtime).is_empty());
        assert!(
            !runtime.desk.records.read.contains_key(&built),
            "a failure is not remembered as no record"
        );
        assert!(
            !runtime.desk_record_due(&built) && runtime.desk_record_due(&other),
            "one session's failure holds no other"
        );
        let first = wait(&runtime);
        assert!((RECORD_RETRY_MS - 1_000..=RECORD_RETRY_MS).contains(&first));
        runtime.desk_records_read(vec![(built.clone(), Err("ledger_rpc_capacity".into()))]);
        assert!((2 * RECORD_RETRY_MS - 1_000..=2 * RECORD_RETRY_MS).contains(&wait(&runtime)));
        for _ in 0..20 {
            runtime.desk_records_read(vec![(built.clone(), Err("ledger_rpc_capacity".into()))]);
        }
        assert!(
            wait(&runtime) <= RECORD_RETRY_MAX_MS,
            "the wait stops growing at its cap"
        );
        let record = SessionRecord {
            session: built.clone(),
            title: "후보 빌드".into(),
            ..SessionRecord::default()
        };
        runtime.desk_records_read(vec![(
            built.clone(),
            Ok(LedgerAnswer::Record(Box::new(record))),
        )]);
        assert_eq!(records(&runtime).len(), 1);
        assert!(!runtime.desk.records.failed.contains_key(&built));
    }

    /// desk.json is outside the node migration (PR Decide): a session kept
    /// under a machine id that changed since names no record, so its chip
    /// stays the dimmed one it was before frame K.
    #[test]
    fn a_session_kept_under_a_machine_id_that_changed_reads_no_record_and_its_chip_stays_dimmed() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _orders) = super::super::tests::runtime(root.path());
        let old = SessionIdentity {
            device_id: "machine-before-the-move".into(),
            agent: "codex".into(),
            session_id: SESSION.into(),
        };
        runtime.desk.stored.keep_named(vec![NamedSession {
            pane_id: "p1".into(),
            session: old.clone(),
            agent_kind: "codex".into(),
            thread: None,
        }]);
        reply_naming(&mut runtime, &["p1"]);
        runtime.refresh_desk_jobs();
        // The link record moved its rows to the machine's new id with the node migration.
        runtime.desk_records_read(vec![(old, Err("ledger_session_missing".into()))]);
        assert!(records(&runtime).is_empty());
    }

    #[test]
    fn a_gone_pane_a_chip_names_opens_its_kept_thread_read_only_and_any_other_has_no_record() {
        let root = tempfile::tempdir().unwrap();
        let (mut runtime, _orders) = super::super::tests::runtime(root.path());
        runtime.desk.stored.keep_named(vec![NamedSession {
            pane_id: "p1".into(),
            session: SessionIdentity {
                device_id: "test-node".into(),
                agent: "codex".into(),
                session_id: SESSION.into(),
            },
            agent_kind: "codex".into(),
            thread: Some(
                serde_json::from_value(json!({
                    "agent": "codex", "account": "default", "reference_kind": "id",
                    "reference_value": SESSION, "cwd": "/checkouts/fixture",
                }))
                .unwrap(),
            ),
        }]);
        let open = |runtime: &mut Runtime, pane: &str| {
            runtime.open_session_thread(serde_json::from_value(json!({"pane_id": pane})).unwrap());
            runtime.snapshot.session_thread.as_deref().unwrap().clone()
        };
        let kept = open(&mut runtime, "p1");
        assert_eq!(
            kept.pane,
            ThreadPane::Gone,
            "no input reaches a session whose pane is gone"
        );
        // Read where the session ran; this test's node holds no such file, so what it answers is the reader's.
        assert_eq!(kept.machine.as_deref(), Some("test-node"));

        let unknown = open(&mut runtime, "p9");
        assert_eq!(
            (unknown.pane, unknown.failure, unknown.machine),
            (ThreadPane::Gone, Some(ThreadFailure::NoRecord), None)
        );
    }
}
