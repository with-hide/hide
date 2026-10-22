//! Session work records. Values cross worker boundaries; SQLite remains
//! exclusively owned by `hide-links`.

use crate::labels::analysis::LabelEnd;
use serde::{Deserialize, Serialize};

pub const REQUEST_LIMIT: usize = 2_000;
/// Goal flows one session keeps; the oldest goes first past it (PRD
/// session-record-goal-flow B30).
pub const FLOW_LIMIT: usize = 200;
/// Pull request sightings one session keeps.
pub const SIGHTING_LIMIT: usize = 200;
pub const MISSED_LIMIT: usize = 200;
pub const NEW_MISSED_LIMIT: usize = 3;
pub const MISSED_CHARS: usize = 80;
/// Requests one analysis summarizes, oldest first; the rest wait for the
/// next one (B34).
pub const SUMMARY_BATCH: usize = 8;
/// Requests owed a summary that a context carries, oldest first.
pub const OWED_IN_CONTEXT: usize = 16;
/// Non-PR outputs one turn may name (D-16).
pub const ARTIFACTS_PER_TURN: usize = 3;
/// Non-PR outputs a record shows, newest first.
pub const ARTIFACT_LIMIT: usize = 24;
/// The session's recorded artifacts an END analysis reads, so it names the
/// same output the same way (B22).
pub const ARTIFACTS_IN_CONTEXT: usize = 12;
/// The longest artifact name kept, in characters.
pub const ARTIFACT_CHARS: usize = 80;
/// Two identical messages from one sender this close together read as one
/// line with a count (D-18).
pub const REPEAT_WINDOW_MS: u64 = 60_000;
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

/// The selected item's owner on the wire, across window holds and worker reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BacklogOriginIdentity {
    pub workspace_id: String,
    pub device_id: String,
    pub task_key: String,
}

impl BacklogOriginIdentity {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, limit) in [
            ("workspace_id", self.workspace_id.as_str(), 256),
            ("device_id", self.device_id.as_str(), 64),
            ("task_key", self.task_key.as_str(), 256),
        ] {
            if value.is_empty() || value.len() > limit {
                return Err(format!(
                    "backlog_origin.{name} must have 1 to {limit} bytes"
                ));
            }
        }
        Ok(())
    }
}

/// Local memo numbers belong to one device and path; GitHub numbers to a repository.
#[derive(Clone, Debug)]
pub enum BacklogTarget {
    Local {
        device_id: String,
        path: String,
        number: u32,
    },
    Github(crate::issues::IssueReference),
}

impl BacklogTarget {
    pub fn for_project(device: &str, path: &str, key: &str) -> Option<Self> {
        if let Some((root, number)) = crate::tasks::parse_local_key(key) {
            return (root == path && !device.is_empty()).then(|| Self::Local {
                device_id: device.to_owned(),
                path: root.to_owned(),
                number,
            });
        }
        crate::tasks::parse_github_key(key).map(Self::Github)
    }

    pub fn key(&self) -> String {
        match self {
            Self::Local { path, number, .. } => crate::tasks::local_key(path, *number),
            Self::Github(reference) => crate::tasks::github_key(reference),
        }
    }

    pub fn local_device(&self) -> Option<&str> {
        match self {
            Self::Local { device_id, .. } => Some(device_id),
            Self::Github(_) => None,
        }
    }
}

/// Short store operations asked by a worker, never waited on by Runtime.
#[derive(Clone, Debug)]
pub enum LedgerCall {
    Record(SessionIdentity),
    /// Display-only history. Never contains a turn, wait or control proof.
    Metadata(SessionIdentity),
    /// Returns only after the existing writer commits the display update.
    SaveMetadata {
        session: SessionIdentity,
        metadata: Box<SessionMetadata>,
        at_unix_ms: u64,
    },
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
        target: BacklogTarget,
    },
    /// The operator cleared a typed item that failed.
    DismissBacklog {
        intent: String,
    },
    /// Typed items whose create is not confirmed.
    PendingBacklog,
    /// The missed line a backlog item came from, if one did.
    BacklogOrigin {
        target: BacklogTarget,
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

/// A Local backlog flag belongs to a device; a GitHub flag is shared by repository/number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BacklogMissedKey {
    pub task_key: String,
    pub device_id: Option<String>,
}

#[derive(Debug)]
pub enum LedgerAnswer {
    Record(Box<SessionRecord>),
    Metadata(Option<SessionMetadata>),
    Attempt(IssueAttempt),
    Saved,
    Pending(Vec<PendingBacklog>),
    Origin(Option<Box<BacklogOrigin>>),
    BacklogKeys {
        missed: Vec<BacklogMissedKey>,
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

/// Conversation-owned words, separate from a pane's current execution.
/// Uses the link ledger's existing session retention and bounded PR list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMetadata {
    pub native_title: Option<String>,
    pub manual_title: Option<String>,
    pub goal: Option<String>,
    pub summary: String,
    pub created_prs: Vec<(String, u64, u64)>,
    pub analysed_start: Option<u64>,
    pub analysed_end: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkAuthor {
    #[default]
    Operator,
    Parent,
}

/// One message a session was sent, as its record keeps it. A letter (a
/// Hide envelope, or the doorbell's `🦉` line) names its sender and is no
/// request of the operator's (D-18, D-19).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRequest {
    pub offset: u64,
    pub at_unix_ms: u64,
    /// The words as the record shows them: an attachment's path is counted
    /// in `images` instead, and a `/goal` command's name in `goal_command`.
    pub text: String,
    pub author: WorkAuthor,
    #[serde(default)]
    pub images: u32,
    /// The label's one-line summary; absent until an analysis gave one.
    #[serde(default)]
    pub summary: Option<String>,
    /// For a reply to the agent's question, which question it answers.
    #[serde(default)]
    pub answers: Option<String>,
    #[serde(default)]
    pub letter_from: Option<String>,
    /// The same words sent again within [`REPEAT_WINDOW_MS`], this one
    /// included; 1 for one message.
    #[serde(default = "one")]
    pub count: u32,
    /// The offsets of the later copies merged into this line, so a History
    /// hit or a missed line on one of them finds it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repeats: Vec<u64>,
    #[serde(default)]
    pub goal_command: bool,
}

impl RecordedRequest {
    /// Whether this line is the message at `offset`, or one merged into it.
    pub fn holds(&self, offset: u64) -> bool {
        self.offset == offset || self.repeats.contains(&offset)
    }
}

fn one() -> u32 {
    1
}

/// A quoted or bare path to an image the operator attached, or Claude
/// Code's placeholder for a pasted one.
static ATTACHMENT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)"[^"\n]*\.(?:png|jpe?g|gif|webp|heic|bmp|tiff?)"|'[^'\n]*\.(?:png|jpe?g|gif|webp|heic|bmp|tiff?)'|(?:~|/)[^\s"']*\.(?:png|jpe?g|gif|webp|heic|bmp|tiff?)\b|\[Image #\d+\]"#,
    )
    .expect("valid attachment expression")
});

/// `text` without the attachments in it, and how many there were (D-18,
/// B25): the record and the label show a count, never a path.
pub fn without_attachments(text: &str) -> (String, u32) {
    let found = ATTACHMENT.find_iter(text).count() as u32;
    if found == 0 {
        return (text.to_owned(), 0);
    }
    let kept = ATTACHMENT
        .replace_all(text, " ")
        .split(' ')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (kept.trim().to_owned(), found)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkChapter {
    pub id: String,
    pub goal: String,
    pub at_unix_ms: u64,
}

/// How a turn ended and its one line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TurnOutcome {
    pub end: LabelEnd,
    pub line: String,
    pub at_unix_ms: u64,
}

/// The record's top state (D-10): the last turn that was not stopped by a
/// usage limit, and the limit's line when the newest turn was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordStatus {
    pub end: LabelEnd,
    pub line: String,
    pub at_unix_ms: u64,
    pub limited: Option<String>,
}

/// One goal flow (D-03, D-05): the requests sent while it was open, placed
/// by time, and for a past flow the result of its turns (D-11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkFlow {
    pub id: String,
    pub name: String,
    pub at_unix_ms: u64,
    /// Oldest first, letters included.
    pub requests: Vec<RecordedRequest>,
    /// The lines that are requests: letters do not count.
    pub request_count: usize,
    pub result: Option<TurnOutcome>,
}

/// The pull requests of this session or of one direct child's subtree (D-14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrGroup {
    /// `None` for this session's own.
    pub child: Option<SessionIdentity>,
    pub name: String,
    pub outputs: Vec<WorkOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissedKind {
    /// Read only from records `context_label.v9` wrote; v10 no longer asks
    /// for it, because the pull request and CI already carry an assigned
    /// task's own remainder.
    Leftover,
    Problem,
    FollowUp,
    /// A question or decision the agent put to the operator that waits for
    /// an answer (D-12): the record's `나를 기다리는 것`, never the backlog's.
    Ask,
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

/// A request's summary from one analysis, keyed by the request's offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestSummary {
    pub offset: u64,
    pub summary: String,
    pub answers: Option<String>,
}

/// The extra judgment travels with the existing turn-end answer, never in
/// an additional provider call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnJudgment {
    pub missed: Vec<NewMissedWork>,
    pub resolved: Vec<ResolvedMissedWork>,
    /// What the turn made besides a pull request: a board, a document, an
    /// issue, a release (D-16).
    #[serde(default)]
    pub artifacts: Vec<String>,
}

/// Whether a goal starts a flow or renames the current one (D-09).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowChange {
    New,
    Rename,
}

#[derive(Clone, Debug)]
pub enum LedgerUpdate {
    /// A native-proven label, including the compatibility import. Older
    /// observations cannot replace a newer saved label.
    Metadata {
        session: SessionIdentity,
        native_title: Option<String>,
        manual_title: Option<String>,
        goal: Option<String>,
        summary: String,
        created_prs: Vec<(String, u64, u64)>,
        at_unix_ms: u64,
        analysed_start: Option<u64>,
        analysed_end: Option<u64>,
    },
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
        change: FlowChange,
    },
    /// Summaries one analysis gave, at either boundary (D-06).
    Summaries {
        session: SessionIdentity,
        summaries: Vec<RequestSummary>,
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
        line: String,
        work_author: bool,
        turn_witness: Option<String>,
        judgment: TurnJudgment,
    },
    MissedStatus {
        session: SessionIdentity,
        id: String,
        status: MissedStatus,
    },
}

/// A request that waits for its summary, with the words the ledger keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwedRequest {
    pub offset: u64,
    pub at_unix_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LedgerContext {
    pub parent_name: Option<String>,
    pub open_missed: Vec<MissedWork>,
    /// The oldest requests still owed a summary, at most
    /// [`OWED_IN_CONTEXT`] (B34).
    pub owed: Vec<OwedRequest>,
    /// The newest request offset the ledger holds: a message past it is not
    /// recorded yet, so it is owed a summary too.
    pub newest_offset: Option<u64>,
    /// This session's recorded artifacts, newest first, each once, at most
    /// [`ARTIFACTS_IN_CONTEXT`].
    pub artifacts: Vec<String>,
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
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub repository: Option<String>,
    pub path: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub ended_at_unix_ms: Option<u64>,
    /// The last substantive turn and a limit's badge (D-10, B10).
    pub status: Option<RecordStatus>,
    /// Questions and decisions waiting on the operator (D-12).
    pub asks: Vec<MissedWork>,
    /// Oldest first; the last is the current flow (D-03).
    pub flows: Vec<WorkFlow>,
    /// Requests the cap dropped from this session (B30).
    pub omitted_requests: u64,
    /// This session's and every descendant's outputs, newest first.
    pub outputs: Vec<WorkOutput>,
    /// The pull requests among `outputs`: this session's first, then one
    /// group per direct child holding its whole subtree's (B18).
    pub pr_groups: Vec<PrGroup>,
    /// Non-PR outputs of this session and its descendants, newest first,
    /// each once (B22).
    pub artifacts: Vec<String>,
    pub children: Vec<SessionRecord>,
    /// Every missed line, for the ledger's own context and the backlog; the
    /// screen reads `asks`.
    #[serde(skip)]
    pub missed: Vec<MissedWork>,
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
            | Self::Metadata { session, .. }
            | Self::Requests { session, .. }
            | Self::Chapter { session, .. }
            | Self::Summaries { session, .. }
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
    pub include_flows: bool,
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
