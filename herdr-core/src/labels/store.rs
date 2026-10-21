//! What the label workers remember across a daemon restart (PRD
//! labels-in-hided D-08): per pane, the proven session owner and its label,
//! which turns were already analyzed, where the conversation was last read,
//! and when the pane last changed state.
//!
//! The file is `labels.json` beside `core-state.json`, written whole through
//! a temporary file and a rename, mode 0600, never under the runtime mutex
//! and never on the thread that applies Herdr's events: a save hands the
//! write to the store's own thread, and dropping the store waits for it.
//! Labels are the provider's short summaries, positions are byte offsets and
//! file identities, turns are hashes; the only conversation text is each
//! local pane's last request and reply, capped (`facts`, PRD overview-request-view
//! B31), and up to 800 masked characters of its assignment requests and early clarifying replies (`goal_evidence`). A device's records remain in memory, except for [`DeviceMark`]: the
//! session proof, the hashes of the turns already analyzed and the label the
//! model wrote, which keep a restart from asking about the same turns again
//! and from losing the label. A file
//! that cannot be read starts empty with a diagnostic (B21); nothing in it is
//! shown until the pane's current session reference proves it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use hide_session::ConversationCheckpoint;
use hide_session::turns::{TurnTracker, Waiting};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::analysis::LabelEnd;
use super::facts::{Requester, SessionFacts};

pub(crate) const LABELS_FILE: &str = "labels.json";
/// The turns a pane keeps a request summary for: the current one and the two
/// before it (PRD pane-last-request D-09). An older one is dropped.
pub(crate) const RECENT_TURNS: usize = 3;
const SCHEMA_VERSION: u32 = 1;
/// The store key of the test core's own Herdr server: its node id.
#[cfg(test)]
pub(crate) const LOCAL_TARGET: &str = "test-node";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PaneRecord {
    /// The provider's native session, proven from transcript metadata
    /// (`label_reference_token(provider, "id", native id)`).
    #[serde(default)]
    pub(crate) owner: Option<String>,
    /// Provider of the last native-proven display record. Old records can
    /// still prove this with their explicit native id and owner digest.
    /// DeviceMark deliberately keeps its existing hash-only format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<String>,
    /// Native metadata proven for `owner`; missing on prior helper answers.
    #[serde(default)]
    pub(crate) native_session_id: Option<String>,
    /// The native-file reader's resolved path, re-proved before any effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) native_source_path: Option<String>,
    /// The Herdr reference under which `owner` was last proven. A path
    /// reference names the same session as its id only once a read has
    /// shown it; an id reference proves itself.
    #[serde(default)]
    pub(crate) proven_reference: Option<String>,
    /// The session's goal (D-38); a v3 record named it `task`.
    #[serde(default, alias = "task")]
    pub(crate) goal: Option<String>,
    /// Bounded, masked requests and early replies that establish and clarify the assignment.
    /// Kept only in memory for device sessions, like their other request text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal_evidence: Option<String>,
    /// The assignment's first request, so later reads can retain its early
    /// replies even when the first analysis preceded those replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal_evidence_start: Option<u64>,
    /// The last analysis's line; a v3 record's `progress`.
    #[serde(default, alias = "progress")]
    pub(crate) line: String,
    /// How the last analyzed turn ended, until the agent runs again.
    #[serde(default)]
    pub(crate) end: Option<LabelEnd>,
    /// A v3 record's question and the reply it asked for, read only to
    /// carry a pending question into `end` and `line` on load; retired with
    /// the v3 records (`upgrade_v3`).
    #[serde(default, rename = "question", skip_serializing)]
    pub(crate) v3_question: bool,
    #[serde(default, rename = "expected_reply", skip_serializing)]
    pub(crate) v3_expected_reply: String,
    /// The last human event that fed goal analysis.
    #[serde(default)]
    pub(crate) task_input_cursor: Option<u64>,
    /// The turn whose start was named and the turn whose end was judged.
    #[serde(default)]
    pub(crate) analysis_turn_start: Option<u64>,
    #[serde(default)]
    pub(crate) analysis_turn_end: Option<u64>,
    /// The reference whose file `checkpoint` and `anchor` belong to.
    #[serde(default)]
    pub(crate) read_reference: Option<String>,
    /// Where the last read stopped.
    #[serde(default)]
    pub(crate) checkpoint: Option<ConversationCheckpoint>,
    /// The last human record read, so a restarted reader recovers the
    /// current turn without reading the conversation from its start.
    #[serde(default)]
    pub(crate) anchor: Option<ConversationCheckpoint>,
    #[serde(default)]
    pub(crate) incarnation: Option<String>,
    /// The turn the last read ended in, which the next read continues, for
    /// an agent whose session read reports its turns (`hide_session::turns`).
    #[serde(default)]
    pub(crate) turns: Option<TurnTracker>,
    /// Herdr's `state_change_seq` the last complete read was asked under.
    /// What `turns` says the agent waits for is known for that state only
    /// (PRD codex-plan-approval-hold D-06); `None` while a read is partial.
    #[serde(default)]
    pub(crate) turns_seq: Option<u64>,
    /// The Herdr state of a turn that just ended whose session read has not
    /// landed, for an agent whose read reports background tasks: until it
    /// does, nothing says whether a task will wake the agent, so the row is
    /// not a stop (#902). Not saved: a daemon that restarts reads each
    /// session again and shows its rows as Herdr reports them meanwhile.
    #[serde(skip)]
    pub(crate) turn_end_owed: Option<u64>,
    /// Herdr's per-pane state counter and when the core saw it move, which
    /// is the pane's elapsed time and recency whichever session it runs.
    #[serde(default)]
    pub(crate) state_change_seq: u64,
    #[serde(default)]
    pub(crate) changed_unix_ms: u64,
    #[serde(default)]
    pub(crate) agent_status: Option<String>,
    /// What the session says without any AI; see `facts`.
    #[serde(default)]
    pub(crate) facts: SessionFacts,
    /// The label's one-line summary of each recent turn's request and how
    /// that turn ended, oldest first, at most [`RECENT_TURNS`]. A record
    /// written before it was kept reads it empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) recent_turns: Vec<RecentTurn>,
}

/// One turn the label analysis summarized, by the offset of the message that
/// began it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecentTurn {
    pub(crate) offset: u64,
    /// The label's request summary; empty when it gave none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) requester: Option<Requester>,
    /// How the turn stood at its last judgment and its line; none once a
    /// judgment of it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) end: Option<LabelEnd>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) line: String,
    pub(crate) at_unix_ms: u64,
}

/// Whether `reference` names the session a label was made for: its owner,
/// or a path reference a read has shown to be that owner.
pub(crate) fn proves(
    owner: Option<&str>,
    proven_reference: Option<&str>,
    reference: Option<&str>,
) -> bool {
    let (Some(reference), Some(owner)) = (reference, owner) else {
        return false;
    };
    owner == reference || proven_reference == Some(reference)
}

/// What a read says of the agent's background tasks.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WakeRead {
    /// When each task proven alive expires on its own, if it does.
    pub(crate) expiries: Vec<Option<u64>>,
    pub(crate) vanished: bool,
}

impl PaneRecord {
    pub(crate) fn first_seen(state_change_seq: u64, now_unix_ms: u64) -> Self {
        Self {
            state_change_seq,
            changed_unix_ms: now_unix_ms,
            ..Self::default()
        }
    }

    /// Whether the next analysis builds its context from the operator's
    /// requests in the whole conversation rather than rolling on a goal
    /// already named: no goal or input cursor yet. A legacy named assignment
    /// has no proven origin; the session's first request may be an older job.
    pub(crate) fn needs_initial_context(&self) -> bool {
        self.goal.is_none() || self.task_input_cursor.is_none()
    }

    /// Whether the label belongs to the session the pane runs now. With no
    /// current reference nothing is shown, and the proof is kept for when
    /// the same reference returns (session-label-isolation B9).
    pub(crate) fn proven_for(&self, reference: Option<&str>) -> bool {
        proves(
            self.owner.as_deref(),
            self.proven_reference.as_deref(),
            reference,
        )
    }

    /// Carries a v3 record's pending question into v5's fields, so a row
    /// that was asking still asks after the upgrade.
    fn upgrade_v3(&mut self) {
        let question = std::mem::take(&mut self.v3_question);
        let reply = std::mem::take(&mut self.v3_expected_reply);
        if self.end.is_none() && question && !reply.trim().is_empty() {
            self.end = Some(LabelEnd::Question);
            self.line = reply;
        }
    }

    /// A new session in the pane: nothing the old one said or decided stays.
    pub(crate) fn reset_session(&mut self, owner: Option<String>) {
        self.owner = owner;
        self.native_session_id = None;
        self.native_source_path = None;
        self.proven_reference = None;
        self.goal = None;
        self.goal_evidence = None;
        self.goal_evidence_start = None;
        self.line.clear();
        self.end = None;
        self.facts = SessionFacts::default();
        self.recent_turns.clear();
        self.reset_analysis();
    }

    /// The same session's transcript started over (replaced or truncated):
    /// the label stays, the turn bookkeeping does not.
    pub(crate) fn reset_analysis(&mut self) {
        self.task_input_cursor = None;
        self.analysis_turn_start = None;
        self.analysis_turn_end = None;
        self.facts.forget_offsets();
        // Offsets of a rewritten conversation name other messages.
        self.recent_turns.clear();
    }

    /// Keeps a judgment of the turn begun at `offset`: its request summary,
    /// unless the judgment gave none, and how it stood, when `outcome` says.
    /// A turn older than every kept one, once the ring is full, is not
    /// brought back.
    pub(crate) fn record_turn(
        &mut self,
        offset: u64,
        request: &str,
        requester: Option<Requester>,
        outcome: Option<(LabelEnd, &str)>,
        at_unix_ms: u64,
    ) {
        let index = match self
            .recent_turns
            .binary_search_by_key(&offset, |turn| turn.offset)
        {
            Ok(index) => index,
            Err(index) => {
                self.recent_turns.insert(
                    index,
                    RecentTurn {
                        offset,
                        request: String::new(),
                        requester: None,
                        end: None,
                        line: String::new(),
                        at_unix_ms,
                    },
                );
                index
            }
        };
        let turn = &mut self.recent_turns[index];
        if !request.trim().is_empty() {
            request.clone_into(&mut turn.request);
        }
        if requester.is_some() {
            turn.requester = requester;
        }
        if let Some((end, line)) = outcome {
            turn.end = Some(end);
            line.clone_into(&mut turn.line);
        }
        let over = self.recent_turns.len().saturating_sub(RECENT_TURNS);
        self.recent_turns.drain(..over);
    }

    /// A judgment of the turn begun at `offset` failed: how it stood is
    /// unknown again, while its request summary stays.
    /// Returns whether it knew one.
    pub(crate) fn forget_turn_outcome(&mut self, offset: u64) -> bool {
        let Some(turn) = self
            .recent_turns
            .iter_mut()
            .find(|turn| turn.offset == offset && turn.end.is_some())
        else {
            return false;
        };
        turn.end = None;
        turn.line.clear();
        true
    }

    pub(crate) fn forget_position(&mut self) {
        self.read_reference = None;
        self.checkpoint = None;
        self.anchor = None;
        self.incarnation = None;
        self.turns = None;
        self.turns_seq = None;
    }

    /// What the last complete read says the agent waits for, with the Herdr
    /// state it was read under; the wait is `None` when the records read do
    /// not settle it.
    pub(crate) fn turn_read(&self) -> Option<(u64, Option<Waiting>)> {
        Some((self.turns_seq?, self.turns.as_ref()?.waiting()))
    }

    /// When each background task the session proves alive expires on its own,
    /// with the Herdr state the read was made under.
    pub(crate) fn wake_read(&self) -> Option<(u64, WakeRead)> {
        let turns = self.turns.as_ref()?;
        Some((
            self.turns_seq?,
            WakeRead {
                expiries: turns.wake_expiries(),
                vanished: turns.wake_vanished(),
            },
        ))
    }

    pub(crate) fn user_turn(&self) -> Option<(u64, hide_session::turns::UserTurnFact)> {
        Some((self.turns_seq?, self.turns.as_ref()?.user_turn()?))
    }
}

/// The most panes of one device whose analyzed turns are kept, and the most
/// devices; a record beyond either is dropped, newest first kept.
const DEVICE_MARK_PANES: usize = 512;
const DEVICE_MARK_DEVICES: usize = 64;

/// What a device pane keeps on disk, and nothing else: which session it ran
/// (a hash token), which turns were analyzed (hashes), how the last one ended
/// and the label the model wrote (its goal, line and recent request
/// summaries, capped). No request or
/// reply of the conversation, no fact, no path. A restart restores these so a
/// turn that was already analyzed is not paid for again and its label is
/// still there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct DeviceMark {
    owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proven_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    analysis_turn_start: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    analysis_turn_end: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<LabelEnd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    goal: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    line: String,
    /// The label's summaries of the recent turns' requests, model text like
    /// `goal` and `line` (PRD pane-last-request D-09).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recent_turns: Vec<RecentTurn>,
    #[serde(default)]
    changed_unix_ms: u64,
}

impl DeviceMark {
    /// The mark of a record, or `None` when it has no proven session or no
    /// analyzed turn: nothing there would spare a request.
    fn of(record: &PaneRecord) -> Option<Self> {
        let owner = record.owner.clone()?;
        if record.analysis_turn_start.is_none() && record.analysis_turn_end.is_none() {
            return None;
        }
        Some(Self {
            owner,
            proven_reference: record.proven_reference.clone(),
            analysis_turn_start: record.analysis_turn_start,
            analysis_turn_end: record.analysis_turn_end,
            end: record.end,
            goal: record.goal.clone(),
            line: record.line.clone(),
            recent_turns: record.recent_turns.clone(),
            changed_unix_ms: record.changed_unix_ms,
        })
    }

    /// What decides a request, without the time that only orders the cap:
    /// the worker moves `changed_unix_ms` on every state change of the pane,
    /// and a state change alone must not rewrite the file.
    fn proof(&self) -> impl PartialEq + '_ {
        (
            &self.owner,
            &self.proven_reference,
            self.analysis_turn_start,
            self.analysis_turn_end,
            self.end,
            &self.goal,
            &self.line,
            &self.recent_turns,
        )
    }

    fn into_record(self) -> PaneRecord {
        PaneRecord {
            owner: Some(self.owner),
            proven_reference: self.proven_reference,
            analysis_turn_start: self.analysis_turn_start,
            analysis_turn_end: self.analysis_turn_end,
            end: self.end,
            goal: self.goal,
            line: self.line,
            recent_turns: self.recent_turns,
            changed_unix_ms: self.changed_unix_ms,
            ..PaneRecord::default()
        }
    }
}

type DeviceMarks = BTreeMap<String, BTreeMap<String, DeviceMark>>;

/// The marks of the device targets among `targets`, each device's newest
/// panes first up to the cap, and how many the caps left out; a device with
/// none is left out.
fn device_marks<'a>(
    targets: impl Iterator<Item = (&'a String, &'a BTreeMap<String, PaneRecord>)>,
) -> (DeviceMarks, usize) {
    let prefix = super::device_target("");
    let mut dropped = 0;
    let mut devices: Vec<_> = targets
        .filter(|(target, _)| target.starts_with(&prefix))
        .filter_map(|(target, records)| {
            let mut marks: Vec<_> = records
                .iter()
                .filter_map(|(pane, record)| Some((pane.clone(), DeviceMark::of(record)?)))
                .collect();
            if marks.is_empty() {
                return None;
            }
            marks.sort_by_key(|(_, mark)| std::cmp::Reverse(mark.changed_unix_ms));
            dropped += marks.len().saturating_sub(DEVICE_MARK_PANES);
            marks.truncate(DEVICE_MARK_PANES);
            let newest = marks[0].1.changed_unix_ms;
            Some((newest, target.clone(), marks))
        })
        .collect();
    devices.sort_by_key(|(newest, ..)| std::cmp::Reverse(*newest));
    dropped += devices
        .iter()
        .skip(DEVICE_MARK_DEVICES)
        .map(|(.., marks)| marks.len())
        .sum::<usize>();
    devices.truncate(DEVICE_MARK_DEVICES);
    let marks = devices
        .into_iter()
        .map(|(_, target, marks)| (target, marks.into_iter().collect()))
        .collect();
    (marks, dropped)
}

/// Whether two sets of one device's marks hold the same proofs.
fn same_proofs(a: &BTreeMap<String, DeviceMark>, b: &BTreeMap<String, DeviceMark>) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((pane_a, a), (pane_b, b))| pane_a == pane_b && a.proof() == b.proof())
}

/// One device target's marks, to tell whether a save changed what is written.
fn device_marks_of(
    targets: &BTreeMap<String, BTreeMap<String, PaneRecord>>,
    key: &str,
) -> BTreeMap<String, DeviceMark> {
    device_marks(targets.get_key_value(key).into_iter())
        .0
        .remove(key)
        .unwrap_or_default()
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LabelsFile {
    version: u32,
    #[serde(default)]
    targets: BTreeMap<String, BTreeMap<String, PaneRecord>>,
    /// Each device's [`DeviceMark`]s, the only thing of a device that is
    /// written. Read as raw values so that one entry this build cannot read
    /// (a word a newer build wrote) is dropped alone and never takes the
    /// local records of the whole file with it.
    #[serde(default, skip_serializing)]
    devices: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// Another machine's records a core move carried here (PRD
    /// core-host-node-move B4): taken into memory for that machine's worker
    /// once, and never written again, so they stay memory-only as every
    /// device's do.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    moved: BTreeMap<String, BTreeMap<String, PaneRecord>>,
}

/// The daemon's one `labels.json`, shared by the worker of every Herdr
/// server it follows. Each worker owns its target's entry.
pub(crate) struct LabelStore {
    path: Option<PathBuf>,
    /// The store key of the core's own Herdr server: its node id. Only this
    /// target reaches disk.
    node: String,
    file: Arc<Mutex<LabelsFile>>,
    /// Held from encoding to rename, so two workers saving at once cannot
    /// share the temporary file and the last write carries the newest data.
    writing: Arc<Mutex<()>>,
    /// The thread that writes this machine's records, started by the first
    /// save: a write ends in an fsync, and the workers save from the
    /// coordinator thread that applies Herdr's events (PRD
    /// instant-pane-topology D-13). It ends, after its last write, when the
    /// store is dropped.
    saver: Mutex<Option<Saver>>,
}

/// The saver thread's requests: `None` asks for a save, and `Some` is
/// answered once every save asked for before it is on disk.
type SaveRequest = Option<Sender<()>>;

struct Saver {
    requests: Sender<SaveRequest>,
    worker: JoinHandle<()>,
}

impl LabelStore {
    /// Loads the store beside `state_dir`, or imports the retired plugin's
    /// state from `home` once when there is no store yet (D-11).
    pub(crate) fn open(state_dir: Option<&Path>, home: Option<&Path>, node: &str) -> Self {
        let Some(path) = state_dir.map(|dir| dir.join(LABELS_FILE)) else {
            return Self::in_memory(node);
        };
        let mut writable = true;
        let file = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<LabelsFile>(&bytes) {
                Ok(file) if file.version == SCHEMA_VERSION => file,
                Ok(file) => {
                    writable = false;
                    crate::diagnostic!(json!({
                        "component": "labels",
                        "kind": "store.version_unknown",
                        "version": file.version,
                    }));
                    LabelsFile::default()
                }
                Err(error) => {
                    writable = false;
                    crate::diagnostic!(json!({
                        "component": "labels",
                        "kind": "store.unreadable",
                        "message": error.to_string(),
                    }));
                    LabelsFile::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut file = LabelsFile::default();
                if let Some(home) = home {
                    let imported = super::import::plugin_state(home);
                    if !imported.is_empty() {
                        crate::diagnostic!(json!({
                            "component": "labels",
                            "kind": "store.imported",
                            "panes": imported.len(),
                        }));
                        file.targets.insert(node.to_owned(), imported);
                    }
                }
                file
            }
            Err(error) => {
                writable = false;
                crate::diagnostic!(json!({
                    "component": "labels",
                    "kind": "store.unreadable",
                    "message": error.to_string(),
                }));
                LabelsFile::default()
            }
        };
        let mut targets = file.targets;
        // Older versions wrote device facts here. Do not restore them,
        // and the save below removes those records from the existing file.
        targets.retain(|target, _| target == node);
        // A device comes back as its marks only: the session it ran and the
        // turns already analyzed.
        let device_prefix = super::device_target("");
        let mut devices = file.devices;
        devices.retain(|target, _| target.starts_with(&device_prefix));
        let (mut unreadable, mut dropped) = (0usize, 0usize);
        let mut restored: Vec<_> = devices
            .into_iter()
            .map(|(target, raw)| {
                let mut marks: Vec<(String, DeviceMark)> = raw
                    .into_iter()
                    .filter_map(|(pane, value)| match serde_json::from_value(value) {
                        Ok(mark) => Some((pane, mark)),
                        Err(_) => {
                            unreadable += 1;
                            None
                        }
                    })
                    .collect();
                marks.sort_by_key(|(_, mark)| std::cmp::Reverse(mark.changed_unix_ms));
                dropped += marks.len().saturating_sub(DEVICE_MARK_PANES);
                marks.truncate(DEVICE_MARK_PANES);
                (target, marks)
            })
            .filter(|(_, marks)| !marks.is_empty())
            .collect();
        restored.sort_by_key(|(_, marks)| std::cmp::Reverse(marks[0].1.changed_unix_ms));
        dropped += restored
            .iter()
            .skip(DEVICE_MARK_DEVICES)
            .map(|(_, marks)| marks.len())
            .sum::<usize>();
        for (target, marks) in restored.into_iter().take(DEVICE_MARK_DEVICES) {
            targets.insert(
                target,
                marks
                    .into_iter()
                    .map(|(pane, mark)| (pane, mark.into_record()))
                    .collect(),
            );
        }
        if unreadable > 0 || dropped > 0 {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "store.device_marks_dropped",
                "unreadable": unreadable,
                "over_cap": dropped,
            }));
        }
        let moved = file.moved.len();
        targets.extend(
            file.moved
                .into_iter()
                .filter(|(target, _)| target.starts_with(&super::device_target(""))),
        );
        if moved > 0 {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "store.moved_in",
                "targets": moved,
            }));
        }
        for record in targets.values_mut().flat_map(BTreeMap::values_mut) {
            record.upgrade_v3();
        }
        let store = Self {
            // A future or broken file is preserved byte-for-byte. The
            // current runtime can recover in memory; reopening a readable
            // file restores persistence without overwriting unknown data.
            path: writable.then_some(path),
            node: node.to_owned(),
            writing: Arc::new(Mutex::new(())),
            file: Arc::new(Mutex::new(LabelsFile {
                version: SCHEMA_VERSION,
                targets,
                devices: BTreeMap::new(),
                moved: BTreeMap::new(),
            })),
            saver: Mutex::new(None),
        };
        // An import is written at once, so the kit's later removal of the
        // plugin's folder cannot take it away.
        store.save();
        store
    }

    pub(crate) fn in_memory(node: &str) -> Self {
        Self {
            path: None,
            node: node.to_owned(),
            writing: Arc::new(Mutex::new(())),
            file: Arc::new(Mutex::new(LabelsFile {
                version: SCHEMA_VERSION,
                targets: BTreeMap::new(),
                devices: BTreeMap::new(),
                moved: BTreeMap::new(),
            })),
            saver: Mutex::new(None),
        }
    }

    /// The store key of the core's own Herdr server.
    pub(crate) fn node(&self) -> &str {
        &self.node
    }

    /// A worker's starting records.
    pub(crate) fn target(&self, key: &str) -> BTreeMap<String, PaneRecord> {
        self.lock().targets.get(key).cloned().unwrap_or_default()
    }

    /// A target's records as `labels.json` stores them, for a core move to
    /// carry the records only memory holds (PRD core-host-node-move Q18).
    pub(crate) fn export(&self, key: &str) -> Result<serde_json::Value, String> {
        serde_json::to_value(self.target(key)).map_err(|error| error.to_string())
    }

    /// Replaces a worker's records. This machine's records reach disk; a
    /// device's conversation stays available to its worker in memory, and
    /// only its [`DeviceMark`]s are written, when they change.
    pub(crate) fn save_target(&self, key: &str, records: &BTreeMap<String, PaneRecord>) {
        let write = {
            let mut file = self.lock();
            if file.targets.get(key) == Some(records) {
                return;
            }
            let marks_before = (key != self.node).then(|| device_marks_of(&file.targets, key));
            file.targets.insert(key.to_owned(), records.clone());
            key == self.node
                || marks_before.is_some_and(|before| {
                    !same_proofs(&before, &device_marks_of(&file.targets, key))
                })
        };
        if write {
            self.save_later();
        }
    }

    /// Returns once every save asked for so far is on disk; the product
    /// waits for its saves only by dropping the store.
    #[cfg(test)]
    pub(crate) fn flush(&self) {
        let saver = self.saver.lock().unwrap_or_else(|error| error.into_inner());
        let Some(saver) = saver.as_ref() else {
            return;
        };
        let (answer, answered) = channel();
        if saver.requests.send(Some(answer)).is_ok() {
            let _ = answered.recv();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LabelsFile> {
        self.file.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Hands the write to the saver thread; saves asked for while one is
    /// being written become one more write.
    fn save_later(&self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let mut saver = self.saver.lock().unwrap_or_else(|error| error.into_inner());
        if saver.is_none() {
            let (requests, receiver) = channel::<SaveRequest>();
            let file = Arc::clone(&self.file);
            let writing = Arc::clone(&self.writing);
            let node = self.node.clone();
            let spawned = std::thread::Builder::new()
                .name("hide-labels-save".to_owned())
                .spawn(move || {
                    while let Ok(request) = receiver.recv() {
                        let mut flushes = Vec::new();
                        let mut save = false;
                        for request in std::iter::once(request).chain(receiver.try_iter()) {
                            match request {
                                None => save = true,
                                Some(answer) => flushes.push(answer),
                            }
                        }
                        if save {
                            write_store(&path, &node, &writing, &file);
                        }
                        for answer in flushes {
                            let _ = answer.send(());
                        }
                    }
                });
            match spawned {
                Ok(worker) => *saver = Some(Saver { requests, worker }),
                Err(error) => {
                    crate::diagnostic!(json!({
                        "component": "labels",
                        "kind": "store.saver_failed",
                        "message": error.to_string(),
                    }));
                    drop(saver);
                    self.save();
                    return;
                }
            }
        }
        if let Some(running) = saver.as_ref()
            && running.requests.send(None).is_err()
        {
            // The saver ended (a write panicked): this save is written here,
            // and the next one starts a new saver.
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "store.saver_ended",
            }));
            *saver = None;
            drop(saver);
            self.save();
        }
    }

    fn save(&self) {
        if let Some(path) = self.path.as_deref() {
            write_store(path, &self.node, &self.writing, &self.file);
        }
    }
}

impl Drop for LabelStore {
    fn drop(&mut self) {
        let saver = self
            .saver
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(Saver { requests, worker }) = saver {
            drop(requests);
            if worker.join().is_err() {
                crate::diagnostic!(json!({
                    "component": "labels",
                    "kind": "store.saver_join_failed",
                }));
            }
        }
    }
}

/// Encodes this machine's records and writes them in place of the file.
fn write_store(path: &Path, node: &str, writing: &Mutex<()>, file: &Mutex<LabelsFile>) {
    let _writing = writing.lock().unwrap_or_else(|error| error.into_inner());
    let encoded = {
        let file = file.lock().unwrap_or_else(|error| error.into_inner());
        // Filter at the final serialization boundary as well: a later
        // local save must never serialize device entries held in memory.
        #[derive(Serialize)]
        struct LocalFile<'a> {
            version: u32,
            targets: BTreeMap<&'a str, &'a BTreeMap<String, PaneRecord>>,
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            devices: DeviceMarks,
        }
        let (devices, dropped) = device_marks(file.targets.iter());
        if dropped > 0 {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "store.device_marks_capped",
                "dropped": dropped,
            }));
        }
        serde_json::to_vec(&LocalFile {
            version: SCHEMA_VERSION,
            targets: file
                .targets
                .iter()
                .filter(|(target, _)| target.as_str() == node)
                .map(|(target, records)| (target.as_str(), records))
                .collect(),
            devices,
        })
    };
    let bytes = match encoded {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "labels",
                "kind": "store.encode_failed",
                "message": error.to_string(),
            }));
            return;
        }
    };
    if let Err(error) = write_private(path, &bytes) {
        crate::diagnostic!(json!({
            "component": "labels",
            "kind": "store.write_failed",
            "message": error.to_string(),
        }));
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::other("labels path has no directory"))?;
    hide_platform::fs::private::create_dir_all(directory)?;
    hide_platform::fs::atomic::write_file(path, bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(owner: &str) -> PaneRecord {
        PaneRecord {
            owner: Some(owner.to_owned()),
            goal: Some("라벨 저장소 확인".to_owned()),
            state_change_seq: 3,
            changed_unix_ms: 1_000,
            ..PaneRecord::default()
        }
    }

    #[test]
    fn a_saved_target_survives_a_reopen() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let mut records = BTreeMap::new();
        records.insert("w1:p1".to_owned(), record("v1:owner"));
        store.save_target(LOCAL_TARGET, &records);
        // Dropping the store waits for the write it handed off.
        drop(store);
        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        assert_eq!(reopened.target(LOCAL_TARGET), records);
        assert!(hide_platform::fs::private::is_private(&root.path().join(LABELS_FILE)).unwrap());
    }

    /// D-09: a pane keeps the current turn and the two before it, oldest
    /// dropped; a late answer about a turn already dropped does not come
    /// back, and an answer with no summary keeps the one it had.
    #[test]
    fn recent_turns_keep_three_by_offset_and_a_late_older_one_stays_out() {
        let mut record = record("v1:owner");
        for offset in [10, 20, 30, 40] {
            record.record_turn(
                offset,
                &format!("요청 {offset}"),
                Some(Requester::Operator),
                Some((LabelEnd::Done, "끝")),
                offset,
            );
        }
        let kept = |record: &PaneRecord| {
            record
                .recent_turns
                .iter()
                .map(|turn| (turn.offset, turn.request.clone()))
                .collect::<Vec<_>>()
        };
        let three = vec![
            (20, "요청 20".to_owned()),
            (30, "요청 30".to_owned()),
            (40, "요청 40".to_owned()),
        ];
        assert_eq!(kept(&record), three);
        record.record_turn(10, "늦은 답", None, None, 10);
        assert_eq!(kept(&record), three, "a dropped turn does not come back");
        record.record_turn(40, " ", None, Some((LabelEnd::Question, "골라 주세요")), 40);
        assert_eq!(kept(&record), three, "an empty summary keeps the old one");
        assert_eq!(record.recent_turns[2].end, Some(LabelEnd::Question));
        assert!(record.forget_turn_outcome(40));
        assert_eq!(
            (
                record.recent_turns[2].end,
                record.recent_turns[2].line.as_str()
            ),
            (None, "")
        );
        record.reset_session(Some("v1:other".to_owned()));
        assert!(record.recent_turns.is_empty());
    }

    /// B13: a device pane's request summaries come back with its mark after a
    /// restart, as its goal and line do.
    #[test]
    fn a_device_marks_recent_turns_survive_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let mut device = record("remote-owner");
        device.analysis_turn_end = Some(7);
        device.record_turn(7, "원격 요청 요약", Some(Requester::Operator), None, 1);
        store.save_target(
            "device:mini",
            &BTreeMap::from([("w2:p1".to_owned(), device)]),
        );
        drop(store);
        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let restored = &reopened.target("device:mini")["w2:p1"];
        assert_eq!(restored.recent_turns[0].request, "원격 요청 요약");
    }

    #[test]
    fn device_conversation_stays_in_memory_even_after_a_local_save() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let mut device = record("remote-owner");
        device.facts = serde_json::from_value(json!({
            "operator_request": {"text":"private remote operator", "at_unix_ms":1, "requester":{"kind":"operator"}},
            "other_request": {"text":"private remote sender", "at_unix_ms":2, "requester":{"kind":"agent"}},
            "reply": {"text":"private remote reply", "at_unix_ms":3}
        })).unwrap();
        let remote = BTreeMap::from([("w2:p1".to_owned(), device)]);
        store.save_target("device:mini", &remote);
        assert_eq!(store.target("device:mini"), remote);
        let local = BTreeMap::from([("w1:p1".to_owned(), record("local-owner"))]);
        store.save_target(LOCAL_TARGET, &local);
        store.flush();
        let disk = std::fs::read_to_string(root.path().join(LABELS_FILE)).unwrap();
        assert!(
            !disk.contains("private remote"),
            "device conversation reached disk"
        );
        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        assert_eq!(reopened.target(LOCAL_TARGET), local);
        assert!(reopened.target("device:mini").is_empty());
        assert_eq!(store.target("device:mini"), remote);
    }

    fn analyzed(owner: &str, start: u64, end: u64, changed: u64) -> PaneRecord {
        PaneRecord {
            analysis_turn_start: Some(start),
            analysis_turn_end: Some(end),
            changed_unix_ms: changed,
            ..record(owner)
        }
    }

    /// A restart must not ask the model about a device turn twice, and the
    /// device's conversation and labels still never reach disk.
    #[test]
    fn a_device_keeps_only_its_session_proof_and_analyzed_turns_across_a_reopen() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let mut device = analyzed("remote-owner", 7, 9, 5);
        device.proven_reference = Some("v1:path".to_owned());
        device.goal_evidence = Some("private remote assignment".to_owned());
        device.end = Some(LabelEnd::Question);
        device.line = "private remote line".to_owned();
        device.facts = serde_json::from_value(json!({
            "reply": {"text":"private remote reply", "at_unix_ms":3}
        }))
        .unwrap();
        store.save_target(
            "device:mini",
            &BTreeMap::from([("w2:p1".to_owned(), device)]),
        );
        store.flush();
        let disk = std::fs::read_to_string(root.path().join(LABELS_FILE)).unwrap();
        // The conversation does not reach disk; the label is the model's own
        // words.
        assert!(!disk.contains("private remote reply"), "{disk}");
        assert!(!disk.contains("private remote assignment"), "{disk}");

        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let restored = &reopened.target("device:mini")["w2:p1"];
        assert_eq!(restored.owner.as_deref(), Some("remote-owner"));
        assert_eq!(restored.proven_reference.as_deref(), Some("v1:path"));
        assert_eq!(
            (restored.analysis_turn_start, restored.analysis_turn_end),
            (Some(7), Some(9))
        );
        // The label the model wrote survives with how the turn ended.
        assert_eq!(restored.end, Some(LabelEnd::Question));
        assert_eq!(restored.goal.as_deref(), Some("라벨 저장소 확인"));
        assert_eq!(restored.line, "private remote line");
        assert_eq!(restored.facts, SessionFacts::default());
        assert_eq!(restored.goal_evidence, None);
    }

    /// A save that moves only what stays in memory writes nothing, so the
    /// per-state-change saves of a device do not become disk writes.
    #[test]
    fn a_device_save_that_changes_no_mark_does_not_write() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let mut device = analyzed("remote-owner", 7, 9, 5);
        store.save_target(
            "device:mini",
            &BTreeMap::from([("w2:p1".to_owned(), device.clone())]),
        );
        store.flush();
        let path = root.path().join(LABELS_FILE);
        std::fs::remove_file(&path).unwrap();
        // The worker moves both on every state change of the pane.
        device.state_change_seq = 99;
        device.changed_unix_ms += 1_000;
        // The conversation is not a mark either.
        device.facts = serde_json::from_value(json!({
            "reply": {"text":"a reply the file does not hold", "at_unix_ms":3}
        }))
        .unwrap();
        store.save_target(
            "device:mini",
            &BTreeMap::from([("w2:p1".to_owned(), device.clone())]),
        );
        store.flush();
        assert!(!path.exists(), "an unchanged mark rewrote the file");
        // A new label is one, and is written.
        device.line = "a new line".to_owned();
        store.save_target(
            "device:mini",
            &BTreeMap::from([("w2:p1".to_owned(), device)]),
        );
        store.flush();
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(disk.contains("a new line"), "{disk}");
        assert!(!disk.contains("a reply the file does not hold"), "{disk}");
    }

    #[test]
    fn the_marks_of_a_device_are_capped_and_keep_the_newest_panes() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let records: BTreeMap<_, _> = (0..DEVICE_MARK_PANES as u64 + 10)
            .map(|index| (format!("w{index}:p1"), analyzed("o", 1, 1, index + 1)))
            .collect();
        store.save_target("device:mini", &records);
        store.flush();
        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let kept = reopened.target("device:mini");
        assert_eq!(kept.len(), DEVICE_MARK_PANES);
        assert!(kept.contains_key(&format!("w{}:p1", DEVICE_MARK_PANES as u64 + 9)));
        assert!(!kept.contains_key("w0:p1"));
    }

    /// One mark this build cannot read (a word a newer build wrote) is
    /// dropped alone; it must not reset the whole file, whose local records
    /// are what keep a restart from asking about every pane again.
    #[test]
    fn an_unreadable_device_mark_is_dropped_alone_and_the_local_records_stay() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(LABELS_FILE),
            serde_json::to_vec(&json!({"version":1,
                "targets":{LOCAL_TARGET:{"w1:p1":record("local-owner")}},
                "devices":{"device:mini":{
                    "w2:p1":{"owner":"o","analysis_turn_end":9,"end":"a_word_from_the_future"},
                    "w2:p2":{"owner":"o","analysis_turn_end":10}}}}))
            .unwrap(),
        )
        .unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        assert_eq!(store.target(LOCAL_TARGET)["w1:p1"], record("local-owner"));
        let kept = store.target("device:mini");
        assert_eq!(kept.keys().collect::<Vec<_>>(), vec!["w2:p2"]);
    }

    #[test]
    fn the_devices_kept_are_capped_and_the_newest_stay() {
        let root = tempfile::tempdir().unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        for index in 0..DEVICE_MARK_DEVICES as u64 + 3 {
            store.save_target(
                &format!("device:{index}"),
                &BTreeMap::from([("w1:p1".to_owned(), analyzed("o", 1, 1, index + 1))]),
            );
        }
        store.flush();
        let reopened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        let kept = (0..DEVICE_MARK_DEVICES as u64 + 3)
            .filter(|index| !reopened.target(&format!("device:{index}")).is_empty())
            .count();
        assert_eq!(kept, DEVICE_MARK_DEVICES);
        assert!(reopened.target("device:0").is_empty());
        assert!(
            !reopened
                .target(&format!("device:{}", DEVICE_MARK_DEVICES + 2))
                .is_empty()
        );
    }

    #[test]
    fn opening_a_legacy_store_removes_device_records_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(LABELS_FILE);
        std::fs::write(&path, serde_json::to_vec(&json!({"version":1,"targets":{
            LOCAL_TARGET:{"w1:p1":record("local-owner")},
            "device:mini":{"w2:p1":{"facts":{"reply":{"text":"legacy private remote","at_unix_ms":1}}}}
        }})).unwrap()).unwrap();
        let opened = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        assert_eq!(opened.target(LOCAL_TARGET)["w1:p1"], record("local-owner"));
        assert!(opened.target("device:mini").is_empty());
        assert!(
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("legacy private remote")
        );
    }

    #[test]
    fn a_v3_record_keeps_its_goal_and_its_pending_question() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(LABELS_FILE),
            r#"{"version":1,"targets":{"test-node":{
                "w1:p1":{"owner":"v1:a","task":"요청 보기 만들기","progress":"테스트 중","question":true,"expected_reply":"A/B 선택"},
                "w1:p2":{"owner":"v1:b","task":"라벨 저장소 확인","progress":"끝남"}}}}"#,
        )
        .unwrap();
        let records = LabelStore::open(Some(root.path()), None, LOCAL_TARGET).target(LOCAL_TARGET);
        let asking = &records["w1:p1"];
        assert_eq!(asking.goal.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(asking.end, Some(LabelEnd::Question));
        assert_eq!(asking.line, "A/B 선택");
        let done = &records["w1:p2"];
        assert_eq!(done.end, None);
        assert_eq!(done.line, "끝남");
        let written = std::fs::read_to_string(root.path().join(LABELS_FILE)).unwrap();
        assert!(!written.contains("expected_reply"), "{written}");
    }

    #[test]
    fn a_corrupt_file_starts_empty() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(LABELS_FILE), b"{not json").unwrap();
        let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
        assert!(store.target(LOCAL_TARGET).is_empty());
    }

    #[test]
    fn unreadable_and_future_files_are_never_replaced_by_an_empty_save() {
        for original in [
            b"{not json".as_slice(),
            br#"{"version":999,"targets":{},"devices":{}}"#,
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(LABELS_FILE);
            std::fs::write(&path, original).unwrap();
            let store = LabelStore::open(Some(root.path()), None, LOCAL_TARGET);
            store.save_target(
                LOCAL_TARGET,
                &BTreeMap::from([("w1:p1".into(), record("new-owner"))]),
            );
            store.flush();
            drop(store);
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn a_label_is_shown_only_under_the_reference_that_proves_it() {
        let mut record = record("v1:id-a");
        assert!(record.proven_for(Some("v1:id-a")));
        assert!(!record.proven_for(None));
        assert!(!record.proven_for(Some("v1:path-a")));
        record.proven_reference = Some("v1:path-a".to_owned());
        assert!(record.proven_for(Some("v1:path-a")));
        assert!(!record.proven_for(Some("v1:id-b")));
        let mut ownerless = record.clone();
        ownerless.owner = None;
        assert!(!ownerless.proven_for(Some("v1:path-a")));
    }
}
