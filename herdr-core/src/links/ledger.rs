//! Session work records. Values cross worker boundaries; SQLite remains
//! exclusively owned by `hide-links`.

use crate::labels::analysis::LabelEnd;
use serde::{Deserialize, Serialize};

pub const REQUEST_LIMIT: usize = 2_000;
pub const TASK_LIMIT: usize = 200;
pub const MISSED_LIMIT: usize = 200;
pub const NEW_MISSED_LIMIT: usize = 3;
pub const MISSED_CHARS: usize = 80;
/// Task groups one END may continue: a request's tasks form one group, so a
/// split request counts once.
pub const CONTINUED_TASK_LIMIT: usize = 2;
/// How many finished requests the END judgment is shown beside the open tasks, newest first,
/// each with all its tasks, so a request that asks for one of them again can continue it.
pub const FINISHED_TASKS_IN_CONTEXT: usize = 3;
/// Tasks one request can make: one per independent deliverable, the extras
/// bundled into the last name (PRD task-split-checklist D-01, D-02).
pub const NEW_TASK_LIMIT: usize = 6;
pub const LEDGER_QUEUE_LIMIT: usize = 512;
pub const SEARCH_LIMIT: usize = 50;
pub const LEDGER_RPC_LIMIT: usize = 16;
/// Typed backlog attempts kept: the newest complete ones are dropped past it.
pub const BACKLOG_INTENT_LIMIT: usize = 512;
/// Unconfirmed typed items the ledger holds at once, all of which a restart
/// offers back; a new create past them is refused rather than one evicted,
/// since its row may still retry it.
pub const PENDING_BACKLOG_LIMIT: usize = 32;
/// Descendant edges below a root; traversal also includes the root itself.
pub const LEDGER_DEPTH_LIMIT: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissedSource {
    pub session: SessionIdentity,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueAttempt {
    pub repository: String,
    pub title: String,
    pub body: String,
    pub started_at_unix_ms: u64,
    pub reconcile: bool,
    pub number: Option<u64>,
    pub url: Option<String>,
    /// Issue numbers other attempts in the same repository already name, so
    /// a reconcile by title never takes one of them for this attempt's.
    pub claimed: Vec<u64>,
}

/// A backlog item typed into the backlog's input or the quick add (PRD
/// overview-device-tasks D-34): the shell's intent key names one item across
/// every retry and restart, so a retry never makes a second issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BacklogIntent {
    pub intent: String,
    pub device: String,
    pub project: String,
    pub workspace_id: String,
}

/// Whose GitHub create an attempt is: a missed-work line's, or a typed item's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptKey {
    Missed(MissedSource),
    Intent(BacklogIntent),
}

/// A typed item whose create was not confirmed, kept across restarts so its
/// retry still converges on one issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingBacklog {
    pub intent: BacklogIntent,
    pub title: String,
    pub started_at_unix_ms: u64,
}

/// Where a backlog item came from, when a missed-work line made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BacklogOrigin {
    pub session: SessionIdentity,
    pub missed: MissedWork,
}

/// Short store operations asked by a worker, never waited on by Runtime.
#[derive(Clone, Debug)]
pub enum LedgerCall {
    Record(SessionIdentity),
    ReserveIssue {
        key: AttemptKey,
        repository: String,
        title: String,
        body: String,
        at_unix_ms: u64,
    },
    StartIssue {
        key: AttemptKey,
        title: String,
        body: String,
        at_unix_ms: u64,
    },
    /// The issue exists: a missed line turns `backlog` with the item's key,
    /// a typed item's attempt completes.
    FinishIssue {
        key: AttemptKey,
        number: u64,
        url: String,
    },
    UnknownIssue(AttemptKey),
    /// A missed line became a memo (`backlog_key` is the memo's task key).
    BacklogLocal {
        source: MissedSource,
        backlog_key: String,
    },
    /// The item was undone: the missed line it came from is open again and
    /// its attempt is forgotten, so the next `백로그로` makes a new item.
    ReleaseBacklog {
        backlog_key: String,
    },
    /// The operator cleared a typed item that failed.
    DismissBacklog {
        intent: String,
    },
    /// Typed items whose create is not confirmed.
    PendingBacklog,
    /// The missed line a backlog item came from, if one did.
    BacklogOrigin {
        backlog_key: String,
    },
    /// Every backlog item key a missed line made, and every GitHub issue
    /// Hide made for the backlog, each newest first and at most
    /// `BACKLOG_KEYS_LIMIT`.
    BacklogKeys,
    /// Whether Hide made this GitHub issue for the backlog: a typed item's
    /// or a missed line's completed create names it. Only such an issue is
    /// closed by `되돌리기` (D-21).
    MadeIssue {
        repository: String,
        number: u32,
    },
}

/// The backlog item keys `BacklogKeys` answers per list; the backlog shows
/// far fewer.
pub const BACKLOG_KEYS_LIMIT: usize = 1024;

#[derive(Debug)]
pub enum LedgerAnswer {
    Record(Box<SessionRecord>),
    Attempt(IssueAttempt),
    Saved,
    Pending(Vec<PendingBacklog>),
    Origin(Option<Box<BacklogOrigin>>),
    BacklogKeys {
        missed: Vec<String>,
        made: Vec<String>,
    },
    Made(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    pub device_id: String,
    pub agent: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkAuthor {
    Operator,
    Parent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRequest {
    pub offset: u64,
    pub at_unix_ms: u64,
    pub text: String,
    pub author: WorkAuthor,
    pub task_ids: Vec<String>,
    pub chapter_id: Option<String>,
    pub judged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkChapter {
    pub id: String,
    pub goal: String,
    pub at_unix_ms: u64,
}

/// The group a task belongs to. The tasks one request made share its id
/// `task-<offset>`: the first carries it, the rest are `task-<offset>.<n>`.
/// A task written before requests split is a group of one.
pub fn task_group(id: &str) -> &str {
    id.split_once('.').map_or(id, |(group, _)| group)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkTask {
    pub id: String,
    pub chapter_id: String,
    pub name: String,
    pub at_unix_ms: u64,
    pub last_at_unix_ms: u64,
    pub end: LabelEnd,
    pub done: bool,
    /// The session's newest turn went unjudged and the task is not done:
    /// its END is an earlier turn's, so no word is drawn for it (PRD
    /// task-continuation D-09).
    #[serde(default)]
    pub unjudged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedKind {
    /// Read only from records `context_label.v9` wrote; v10 no longer asks
    /// for it, because the task verdict, the pull request and CI already
    /// carry an assigned task's own remainder.
    Leftover,
    Problem,
    FollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedStatus {
    Open,
    Resolved,
    /// A GitHub issue Hide made from it before the backlog existed, or one
    /// the session's own turn made and reported.
    Issue,
    Dismissed,
    /// `백로그로` made an item of it: `backlog_key` names the item.
    Backlog,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedWork {
    pub id: String,
    pub kind: MissedKind,
    pub text: String,
    pub status: MissedStatus,
    pub at_unix_ms: u64,
    pub turn_offset: u64,
    #[serde(skip_serializing)]
    pub turn_witness: Option<String>,
    pub issue_repository: Option<String>,
    pub issue_number: Option<u64>,
    /// The backlog item `백로그로` made, as a task key
    /// (`github:owner/repo#7`, `local:/path#3`).
    pub backlog_key: Option<String>,
    pub creating: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewMissedWork {
    pub kind: MissedKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMissedWork {
    pub id: String,
    pub issue_number: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskDecision {
    /// One name per independent deliverable of the request, in its order.
    New {
        names: Vec<String>,
    },
    Continue {
        ids: Vec<String>,
    },
    #[default]
    None,
}

/// The extra judgment travels with the existing turn-end answer, never in
/// an additional provider call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnJudgment {
    pub task: TaskDecision,
    pub missed: Vec<NewMissedWork>,
    pub resolved: Vec<ResolvedMissedWork>,
    pub output_task_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum LedgerUpdate {
    /// A turn observed with summaries off, or whose END judgment failed.
    /// It remains a raw request even if the label itself is retried later.
    Unjudged {
        session: SessionIdentity,
        offsets: Vec<u64>,
    },
    ExcludeRequests {
        session: SessionIdentity,
        parent_name: Option<String>,
        offsets: Vec<u64>,
    },
    Title {
        session: SessionIdentity,
        native_title: Option<String>,
        manual_title: Option<String>,
        state: String,
    },
    Requests {
        session: SessionIdentity,
        cwd: String,
        path: Option<String>,
        requests: Vec<RecordedRequest>,
    },
    Chapter {
        session: SessionIdentity,
        goal: String,
        at_unix_ms: u64,
    },
    TurnEnd {
        session: SessionIdentity,
        offset: u64,
        /// When the input that woke the turn arrived: a request, a letter, an
        /// answer or an alert. A turn with no recorded request starts here,
        /// so what it makes falls inside it (PRD task-continuation D-03).
        started_at_unix_ms: u64,
        at_unix_ms: u64,
        end: LabelEnd,
        work_author: bool,
        /// The sender's name when a report, progress or block letter woke
        /// the turn: the ledger records the turn against the task that
        /// spawned that descendant (D-02), never one the model picks.
        letter: Option<String>,
        turn_witness: Option<String>,
        judgment: TurnJudgment,
    },
    MissedStatus {
        session: SessionIdentity,
        id: String,
        status: MissedStatus,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LedgerContext {
    pub parent_name: Option<String>,
    pub open_tasks: Vec<WorkTask>,
    /// The done tasks of the newest [`FINISHED_TASKS_IN_CONTEXT`] groups, last worked first.
    pub finished_tasks: Vec<WorkTask>,
    pub open_missed: Vec<MissedWork>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkOutput {
    pub kind: String,
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub stale: bool,
    pub last_read_at_unix_ms: Option<u64>,
    pub created_at_unix_ms: u64,
    pub task_id: Option<String>,
    pub source: SessionIdentity,
    pub source_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionRecord {
    pub session: SessionIdentity,
    pub title: String,
    pub fallback_title: String,
    pub native_title: Option<String>,
    pub manual_title: Option<String>,
    pub state: Option<String>,
    pub last_end: Option<LabelEnd>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub repository: Option<String>,
    pub path: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub ended_at_unix_ms: Option<u64>,
    pub chapters: Vec<WorkChapter>,
    pub tasks: Vec<WorkTask>,
    pub requests: Vec<RecordedRequest>,
    pub missed: Vec<MissedWork>,
    pub outputs: Vec<WorkOutput>,
    pub children: Vec<SessionRecord>,
    /// How its direct children stand; absent without children.
    pub child_progress: Option<ChildProgress>,
    /// For a child, the task group of the parent's turn that spawned it;
    /// its outputs belong there and so does the child (B7).
    pub spawn_task: Option<String>,
}

/// How one direct child stands for its parent's `맡긴 일` (PRD
/// task-split-checklist D-04, task-continuation D-08, D-09).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildWork {
    Done,
    Working,
    Other,
    /// No judgment says how it stands: none was ever made, or its newest
    /// turn went unjudged.
    Unjudged,
}

/// One segment per direct child, in the record's order. `done` is absent
/// while any child is unjudged, so a missing judgment never reads as not done.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildProgress {
    pub children: Vec<ChildWork>,
    pub done: Option<usize>,
}

/// What the ledger reads about a child beside its record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChildFacts {
    /// It sent its parent a report letter and started no turn after it.
    pub reported_idle: bool,
    /// Its newest turn went unjudged.
    pub latest_unjudged: bool,
    /// A pane runs its session now.
    pub live: bool,
}

impl ChildFacts {
    /// Whether the child holds the task that spawned it open (PRD
    /// task-continuation D-05, B23): it works in a live pane on a judged
    /// newest turn. A resting child, one whose newest turn went unjudged and
    /// one no pane runs any more, whatever state it was last read in, hold
    /// nothing.
    pub fn holds_task(self, child: &SessionRecord) -> bool {
        self.live && !self.latest_unjudged && child.state.as_deref() == Some("working")
    }
}

impl ChildWork {
    pub fn of(child: &SessionRecord, facts: ChildFacts) -> Self {
        // A child at work is not done, whatever an earlier turn finished: its
        // new request has no END yet (B8). Its state is what the label worker
        // last read from its live pane. One never judged stays unjudged.
        let judged = !child.tasks.is_empty() || child.last_end.is_some();
        let working = child.state.as_deref() == Some("working");
        if judged && working {
            return Self::Working;
        }
        // Facts finish a resting child before any judgment does: its report
        // to the parent, or merged pull requests with none still open.
        let prs = child.outputs.iter().filter(|output| output.kind == "pr");
        let merged = prs.clone().any(|output| output.state == "merged");
        let open = prs.clone().any(|output| pr_open(&output.state));
        if !working && (facts.reported_idle || (merged && !open)) {
            return Self::Done;
        }
        if !child.tasks.is_empty() && child.tasks.iter().all(|task| task.done) {
            return Self::Done;
        }
        if facts.latest_unjudged {
            return Self::Unjudged;
        }
        if !child.tasks.is_empty() {
            return if child
                .tasks
                .iter()
                .any(|task| matches!(task.end, LabelEnd::Working | LabelEnd::Waiting))
            {
                Self::Working
            } else {
                Self::Other
            };
        }
        match child.last_end {
            None => Self::Unjudged,
            Some(LabelEnd::Done) => Self::Done,
            Some(LabelEnd::Working | LabelEnd::Waiting) => Self::Working,
            Some(_) => Self::Other,
        }
    }
}

impl ChildProgress {
    pub fn of(children: Vec<ChildWork>) -> Option<Self> {
        if children.is_empty() {
            return None;
        }
        let done = (!children.contains(&ChildWork::Unjudged)).then(|| {
            children
                .iter()
                .filter(|child| **child == ChildWork::Done)
                .count()
        });
        Some(Self { children, done })
    }
}

/// A pull request still in play: neither merged nor closed unmerged.
pub fn pr_open(state: &str) -> bool {
    state != "merged" && state != "closed"
}

/// Whether a task group is finished (PRD task-continuation D-05, D-06): with
/// a merged pull request and none still open, unless its last END says work
/// is left or blocked or a child it spawned is still at work (B23); with no
/// pull request that counts (a closed unmerged one does not), only when its
/// last END is Done.
pub fn task_done<'a>(
    end: LabelEnd,
    prs: impl Iterator<Item = &'a WorkOutput>,
    child_working: bool,
) -> bool {
    let (mut merged, mut open) = (false, false);
    for pr in prs {
        merged |= pr.state == "merged";
        open |= pr_open(&pr.state);
    }
    if open {
        return false;
    }
    if merged {
        return !child_working && !matches!(end, LabelEnd::Unfinished | LabelEnd::Blocked);
    }
    end == LabelEnd::Done
}

/// What a live pane's chip and checklist popover draw: its session's tasks
/// with the pull requests assigned to them, the count of finished tasks, and
/// its direct children's progress. Only a session with a task has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkProgress {
    pub session: SessionIdentity,
    pub done: usize,
    pub total: usize,
    /// First asked first.
    pub tasks: Vec<WorkTask>,
    /// The pull requests assigned to a task, as the record holds them.
    pub outputs: Vec<WorkOutput>,
    pub children: Option<ChildProgress>,
}

impl WorkProgress {
    pub fn of(record: &SessionRecord) -> Option<Self> {
        if record.tasks.is_empty() {
            return None;
        }
        let mut tasks = record.tasks.clone();
        // The record lists newest first; a request's tasks keep their order.
        tasks.sort_by_key(|task| task.at_unix_ms);
        let outputs = record
            .outputs
            .iter()
            .filter(|output| output.kind == "pr" && output.task_id.is_some())
            .cloned()
            .collect();
        Some(Self {
            session: record.session.clone(),
            done: tasks.iter().filter(|task| task.done).count(),
            total: tasks.len(),
            tasks,
            outputs,
            children: record.child_progress.clone(),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionRecordSnapshot {
    pub session: Option<SessionIdentity>,
    pub loading: bool,
    pub failure: Option<String>,
    pub record: Option<SessionRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionWorkSummary {
    pub session: SessionIdentity,
    pub title: String,
    pub fallback_title: String,
    pub native_title: Option<String>,
    pub manual_title: Option<String>,
    pub cwd: Option<String>,
    pub ended_at_unix_ms: Option<u64>,
    pub open_missed: usize,
    pub subtree_open_missed: usize,
    pub missed: Vec<MissedWork>,
}

impl LedgerUpdate {
    pub fn session(&self) -> &SessionIdentity {
        match self {
            Self::Unjudged { session, .. }
            | Self::ExcludeRequests { session, .. }
            | Self::Title { session, .. }
            | Self::Requests { session, .. }
            | Self::Chapter { session, .. }
            | Self::TurnEnd { session, .. }
            | Self::MissedStatus { session, .. } => session,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionSearchHit {
    pub session: SessionIdentity,
    pub title: String,
    pub matched: String,
    pub at_unix_ms: u64,
    pub cwd: Option<String>,
    pub project_root: Option<String>,
    /// Current core/archival evidence, never the last recorded native state.
    pub status: String,
    pub matches: Vec<SessionSearchMatch>,
    pub match_count: usize,
    pub request_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSearchMatch {
    pub kind: String,
    pub text: String,
    pub at_unix_ms: u64,
    pub request_offset: Option<u64>,
}

/// A total order even when devices/providers reuse a native session id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSearchCursor {
    pub at_unix_ms: u64,
    pub device_id: String,
    pub agent: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionSearchOptions {
    pub request_id: Option<String>,
    pub device_id: Option<String>,
    pub project_root: Option<String>,
    pub agent: Option<String>,
    pub since_unix_ms: Option<u64>,
    pub until_unix_ms: Option<u64>,
    pub status: Option<SessionSearchStatus>,
    pub include_titles: bool,
    pub include_tasks: bool,
    pub cursor: Option<SessionSearchCursor>,
}

impl SessionSearchOptions {
    pub fn bounded(&self) -> bool {
        self.request_id
            .as_ref()
            .is_none_or(|value| value.len() <= 128)
            && [&self.device_id, &self.project_root, &self.agent]
                .iter()
                .all(|value| value.as_ref().is_none_or(|value| value.len() <= 4096))
            && self.cursor.as_ref().is_none_or(|cursor| {
                [&cursor.device_id, &cursor.agent, &cursor.session_id]
                    .iter()
                    .all(|value| !value.is_empty() && value.len() <= 4096)
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionSearchStatus {
    Current,
    Working,
    Done,
    NeedsYou,
    Archived,
    Retained,
}

/// The runtime's identity-proven state at query admission, bounded before
/// leaving its lock. It is not persisted as another source of truth.
#[derive(Clone, Debug, Serialize)]
pub struct SessionSearchPresence {
    pub session: SessionIdentity,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionSearchPage {
    pub results: Vec<SessionSearchHit>,
    pub next_cursor: Option<SessionSearchCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputCreator {
    pub source: SessionIdentity,
    pub kind: String,
    pub repository: String,
    pub number: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SessionSearchSnapshot {
    pub query: String,
    pub options: SessionSearchOptions,
    pub loading: bool,
    pub failure: Option<String>,
    pub results: Vec<SessionSearchHit>,
    pub next_cursor: Option<SessionSearchCursor>,
}
