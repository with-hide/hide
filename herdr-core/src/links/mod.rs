//! The link record (PRD link-graph): which sessions worked on or made which
//! pull request, which issue a pull request closes or was linked to, and the
//! branches and worktrees between them, kept after a pane closes and after a
//! session file is gone.
//!
//! The record only says a link held, with its source and its first and last
//! time; what a pull request, CI, a worktree or a pane is now is read from
//! GitHub, Git and Herdr as before, and wins over the record (D-27).
//!
//! `store` owns the SQLite file (`links.sqlite3`), `worker` the one thread
//! that writes it, and `runtime/links.rs` what the runtime hands the worker
//! and publishes.

pub mod ledger;
pub mod observation;
pub mod query;
pub mod store;
pub mod worker;

use crate::node_access::{LinkError, NodeLink, call_as};
use hide_node_link::protocol::Call;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// How long the own node may take to say which session files it holds.
const FILES_PRESENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Which of `asked` the own node still holds; one it cannot read is kept.
pub fn files_present(own: &dyn NodeLink, asked: &[String]) -> Result<BTreeSet<String>, String> {
    use hide_node_link::cleanup::PathState;
    let states = call_as::<Vec<PathState>>(
        own,
        Call::RealPaths {
            paths: asked.to_vec(),
        },
        FILES_PRESENT_TIMEOUT,
    )
    .map_err(|error| link_code(&error, "node"))?;
    if states.len() != asked.len() {
        return Err("node_answer_mismatched".to_owned());
    }
    Ok(asked
        .iter()
        .zip(states)
        .filter(|(_, state)| !matches!(state, PathState::Missing))
        .map(|(path, _)| path.clone())
        .collect())
}

/// A failed link call as a diagnostic code, prefixed by who was asked.
pub(crate) fn link_code(error: &LinkError, who: &str) -> String {
    match error {
        LinkError::NotConnected(_) => format!("{who}_not_connected"),
        LinkError::Busy => format!("{who}_busy"),
        LinkError::Unknown(_) => format!("{who}_unknown"),
        // A helper older than protocol 18 does not know the call.
        LinkError::Refused(error) if error.code == hide_node_link::ErrorCode::InvalidRequest => {
            format!("{who}_unsupported")
        }
        LinkError::Refused(error) => error.message.clone(),
    }
}

/// The most sessions one panel read returns (D-40).
pub const PANEL_SESSION_LIMIT: usize = 200;
/// The most pull request addresses kept per session (D-40).
pub const SESSION_PR_LIMIT: usize = 20;
/// The most pull requests kept per session that its tool calls named; a
/// session that reviews or merges many keeps its first ones.
pub const SESSION_NAMED_LIMIT: usize = 200;
/// The store's page cap: 64 MiB of 4 KiB pages (D-40).
pub const STORE_PAGE_LIMIT: u32 = 16_384;
/// How far back the first fill reaches (D-17).
pub const BACKFILL_MS: u64 = 90 * DAY_MS;
/// How long a link to an open pull request or issue outlives its retention,
/// counted from the session's end (D-25).
pub const OPEN_EXCEPTION_MS: u64 = 365 * DAY_MS;
pub const DAY_MS: u64 = 86_400_000;
/// The created-PR judgement (D-31, D-44), the link record's and the label
/// worker's (`labels/facts.rs`): an address first printed within this window
/// of GitHub's `createdAt` is the session's making of it. A print reads
/// before `createdAt` by the two clocks alone, the session machine's and
/// GitHub's, so the early side stays narrow: a listing another session
/// printed then must not read as a making.
pub const CREATED_BEFORE_MS: u64 = 2_000;
pub const CREATED_AFTER_MS: u64 = 30_000;
/// The early side for an address its successful creating call gave back
/// (`hide_session::links::PrMark::made`), which only clock skew can put
/// before `createdAt`: this Mac's record held 234 prints, the earliest 3.4 s
/// before GitHub's time, and mission-13's #1810 to #1813 were made 2.6 s to
/// 3.2 s before it. The bound still keeps out a pull request made long ago
/// that the same call happened to print.
pub const MADE_BEFORE_MS: u64 = 10_000;

/// Whether a print at `printed` is the making of a pull request GitHub made
/// at `created`: within [`MADE_BEFORE_MS`] before it when its creating call
/// gave it back (`made`), else within [`CREATED_BEFORE_MS`]; within
/// [`CREATED_AFTER_MS`] after it either way.
pub fn made_by_print(made: bool, printed: u64, created: Option<u64>) -> bool {
    let before = if made {
        MADE_BEFORE_MS
    } else {
        CREATED_BEFORE_MS
    };
    created.is_some_and(|created| {
        printed.saturating_add(before) >= created
            && printed <= created.saturating_add(CREATED_AFTER_MS)
    })
}

/// One registered project as the runtime knows it, handed to the worker
/// whenever it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectFacts {
    /// `hide_project::project_id` of the device and root: the record's key
    /// for the project, and the key its Copied history days are stored under
    /// in the old session search file.
    pub key: String,
    pub device_id: String,
    /// The project's row id, which the summary is published under.
    pub workspace_id: String,
    /// The project's main worktree, which identifies it on its device.
    pub root: String,
    /// `owner/name` GitHub answered for the project, lower case.
    pub repository: Option<String>,
    /// GitHub's id of that repository, which a rename keeps.
    pub repository_id: Option<String>,
    /// Changes on an accepted repository connection edit,
    /// including an A-to-B-to-A edit between worker observations.
    pub connection_epoch: u64,
    /// The project's checkouts now, with their branches.
    pub worktrees: Vec<WorktreeFact>,
    /// The pull requests `gh pr list` answered, with their issues.
    pub prs: Vec<PrFact>,
    /// Whether GitHub answered this pass: a failed read is not an empty one,
    /// so nothing is closed while it fails (D-35).
    pub prs_read: bool,
    /// Current GitHub output states, including issue creation times. Failed
    /// polls retain the last successful facts rather than closing outputs.
    pub outputs: Vec<OutputFact>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputFact {
    pub kind: String,
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub created_at: Option<u64>,
    pub stale: bool,
    pub last_read_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeFact {
    pub path: String,
    pub branch: Option<String>,
    /// When a linked worktree was added (its `.git/worktrees/<name>`
    /// entry); none for the main worktree or before Git has been read.
    pub created_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrFact {
    /// `owner/name`, lower case.
    pub repository: String,
    pub number: u64,
    pub branch: String,
    pub title: String,
    pub url: String,
    pub created_at: Option<u64>,
    pub closed_at: Option<u64>,
    pub merged_at: Option<u64>,
    /// The issues the pull request is linked to now, by task key, each with
    /// the source that says so.
    pub issues: Vec<(String, IssueSource)>,
    /// A checkout of the branch exists, so its Hide issue link was read and a
    /// missing one means the link was cleared; without a checkout the branch
    /// may simply be gone (D-20), which closes nothing (D-35).
    pub hide_issue_known: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSource {
    /// A closing keyword in the pull request's body (GitHub).
    Closes,
    /// Hide's own issue link on the branch (이슈 잇기, a Local issue).
    Hide,
}

impl IssueSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closes => "closes",
            Self::Hide => "hide",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "closes" => Some(Self::Closes),
            "hide" => Some(Self::Hide),
            _ => None,
        }
    }
}

/// An agent session a Hide pane carries, with where the pane works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFact {
    pub device_id: String,
    pub agent: String,
    pub session_id: String,
    pub cwd: String,
    pub branch: Option<String>,
}

/// A session `hide agent spawn` started for a parent session (D-27).
///
/// Each side names its session as the core knows it: a live agent by its
/// reader's id, a registration by the value Herdr reported, which for Pi and
/// omp is the session's file. The store names both as its reader does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentFact {
    pub device_id: String,
    pub agent: String,
    pub session_id: String,
    pub parent_device_id: String,
    pub parent_agent: String,
    pub parent_session_id: String,
    pub parent_name: String,
}

/// Why a session is on a pull request's panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    /// It printed the pull request's address when GitHub made it.
    Created,
    /// It worked on the branch while the pull request lived, or printed the
    /// address later.
    Worked,
}

/// Whether a session's own file is still where the record saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Present,
    Missing,
    /// A device's file, which this machine cannot look at.
    Unknown,
}

/// One session line of a panel: a session, or several joined by continuation
/// (`ids`, oldest first, the last the one to resume).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkedSession {
    pub agent: String,
    pub id: String,
    pub ids: Vec<String>,
    pub device_id: String,
    pub role: SessionRole,
    /// The pull request the line belongs to (the issue panel names it).
    pub pr: u64,
    pub request: Option<String>,
    pub started_at_unix_ms: Option<u64>,
    pub ended_at_unix_ms: Option<u64>,
    pub path: Option<String>,
    pub cwd: Option<String>,
    pub file: FileState,
    pub parent: Option<LinkedParent>,
    /// The line worked on the pull request's branch while it lived (a branch
    /// span, not only a printed address). Core only: `summary` weighs it.
    #[serde(skip)]
    pub on_branch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkedParent {
    pub name: String,
    pub agent: String,
    pub session_id: String,
    /// The parent's own record is still kept (B19).
    pub available: bool,
}

/// A pull request's issue link as the record holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkedIssue {
    pub key: String,
    pub source: IssueSource,
}

/// The small per-project values ⌘K and the Sessions tab read (D-45).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ProjectLinkSummary {
    #[serde(skip)]
    pub record_issues: Vec<crate::issues::IssueReference>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub creators: Vec<ledger::OutputCreator>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub work: Vec<ledger::SessionWorkSummary>,
    /// Pull request number → its recorded session lines.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub prs: BTreeMap<u64, u32>,
    /// Issue task key → its recorded session lines.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub issues: BTreeMap<String, u32>,
    /// Session id → the pull requests it made or worked on.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sessions: BTreeMap<String, Vec<SessionPrChip>>,
    /// Pull request number → the ids of the sessions that made it or worked
    /// on its branch; a session that only printed its address is not one.
    /// Core only: the PRs tool's agent of a pull request reads it (PRD
    /// work-tab-pr-lens D-09).
    #[serde(skip)]
    pub pr_workers: BTreeMap<u64, BTreeSet<String>>,
    /// Pull request number → the paths of the checkouts its working sessions
    /// ran in (the same sessions as `pr_workers`). Core only: the PRs tool's
    /// `<checkout> only` scope reads it (PRD work-tab-pr-lens D-19).
    #[serde(skip)]
    pub pr_checkouts: BTreeMap<u64, BTreeSet<String>>,
    /// The checkouts, by path, whose work landed: the sessions that started
    /// in each since it was added made, or worked on the branch of, a merged
    /// pull request and none still open, of the checkout's own branch when it
    /// is on one. A session belongs to the deepest checkout holding its folder.
    /// Core only: the runtime carries it onto each checkout's `landed`.
    #[serde(skip)]
    pub landed: BTreeSet<String>,
}

impl ProjectLinkSummary {
    pub fn is_empty(&self) -> bool {
        self.work.is_empty()
            && self.prs.is_empty()
            && self.issues.is_empty()
            && self.sessions.is_empty()
            && self.landed.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SessionPrChip {
    pub number: u64,
    pub created: bool,
}

/// Which record a panel reads: a pull request by number, or an issue by its
/// task key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LinkTarget {
    Pr { number: u64 },
    Issue { key: String },
}

impl Default for LinkTarget {
    fn default() -> Self {
        Self::Pr { number: 0 }
    }
}

/// A pull request as the record holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkedPr {
    pub number: u64,
    pub branch: String,
    pub title: String,
    pub url: String,
    pub created_at_unix_ms: Option<u64>,
    pub closed_at_unix_ms: Option<u64>,
    pub merged_at_unix_ms: Option<u64>,
    /// The one PR state ([`crate::model::PrState::of`]) the record can say:
    /// merged, closed, or open with no checks or review read.
    pub state: crate::model::PrState,
    pub issues: Vec<LinkedIssue>,
    /// The worktrees recorded for its branch, newest first; whether each is
    /// still there is the navigator's to say (B5).
    pub worktrees: Vec<String>,
}

/// The `link_panel` section: the record for the one pull request or issue a
/// panel shows, on its own delta revision (D-45). Failures are codes the
/// shell words (B44).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkPanelSnapshot {
    pub workspace_id: String,
    pub target: LinkTarget,
    pub loading: bool,
    pub failure: Option<String>,
    /// The pull request a `pr` target names, when the record has it.
    pub pr: Option<LinkedPr>,
    /// The pull requests an `issue` target is linked to, newest first.
    pub prs: Vec<u64>,
    /// At most [`PANEL_SESSION_LIMIT`] lines, newest first.
    pub sessions: Vec<LinkedSession>,
    /// Every line the record holds for the target.
    pub total: usize,
}

/// The `link_summaries` section: each project's counts and chips by
/// workspace id, and whether files are still being read (B24).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkSummariesSnapshot {
    pub projects: BTreeMap<String, ProjectLinkSummary>,
    pub filling: bool,
}

/// The most sessions a project's summary names chips for.
pub const SUMMARY_SESSION_LIMIT: usize = 2_000;

/// An issue's key as the record holds it: a GitHub key lower case, since
/// GitHub answers a repository's name in either case; a local key as it is,
/// since it names a path.
pub fn issue_key(key: &str) -> String {
    if key.starts_with("github:") {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    }
}

/// `owner/name` from a GitHub pull request address, lower case.
pub fn repository_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|part| !part.is_empty())?;
    let name = parts.next().filter(|part| !part.is_empty())?;
    Some(format!("{owner}/{name}").to_ascii_lowercase())
}

/// A session's cwd lies in `path` or below it. The filesystem root holds
/// every folder and so names no project's place: it matches nothing.
pub fn within(cwd: &str, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return false;
    }
    cwd == path
        || cwd
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The time now in Unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
