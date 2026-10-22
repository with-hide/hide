//! A repository's worktree facts, and the removal shapes a request and its
//! answer carry.

use serde::{Deserialize, Serialize};

/// The header reads only the five newest ancestors of its exact HEAD.
pub const CHECKOUT_COMMIT_LIMIT: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckoutCommit {
    pub sha: String,
    pub subject: String,
    pub commit_unix_seconds: u64,
}

/// The front checkout's small, whole-repository header read. Unlike the
/// catalog this includes ordinary file edits even while Changes is closed.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CheckoutGit {
    #[serde(default)]
    pub recent_commits: Vec<CheckoutCommit>,
    pub head_sha: Option<String>,
    pub subject: Option<String>,
    pub commit_unix_seconds: Option<u64>,
    pub changed_files: u32,
    pub conflicts: u32,
    pub upstream: Option<String>,
    /// `none`, `gone`, or `tracked`; an unsuccessful read is an error.
    pub upstream_state: String,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub last_fetch_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<String>,
    /// The upstream's commits HEAD does not have yet, newest first, at most
    /// [`CHECKOUT_COMMIT_LIMIT`]; `behind` counts them all.
    #[serde(default)]
    pub incoming_commits: Vec<CheckoutCommit>,
    /// Which of `recent_commits` no remote-tracking ref holds: commits not
    /// pushed anywhere this clone knows of.
    #[serde(default)]
    pub unpushed_commits: Vec<String>,
    /// Why the incoming and unpushed commits could not be read; both lists
    /// are then empty and every other fact stands, and no commit has a page,
    /// since whether a remote holds it is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commits_unread: Option<String>,
}

/// The most a node gives one background fetch (`Call::FetchRemote`).
pub const FETCH_TIME: std::time::Duration = std::time::Duration::from_secs(60);

/// How long a core waits for a node's fetch. It is longer than
/// [`FETCH_TIME`], so a fetch that runs out is the node's answer while its
/// core still waits.
pub const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(75);

const _: () = assert!(FETCH_TIME.as_secs() + 10 <= FETCH_TIMEOUT.as_secs());

/// What a background fetch of a repository's `origin` did
/// (`Call::FetchRemote`, PRD git-freshness D-01).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum FetchOutcome {
    /// `origin`'s remote-tracking refs are as the remote has them now.
    Fetched,
    /// Nothing was asked of the remote, and nothing failed.
    Skipped { reason: FetchSkip },
    /// The remote was asked and did not answer. `reason` is Git's last
    /// fatal line with any URL credentials taken out.
    Failed {
        failure: FetchFailure,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchSkip {
    /// The repository has no `origin` remote.
    NoOrigin,
    /// Another Git (the operator's own fetch) held a ref lock.
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchFailure {
    /// A credential was needed that nothing could give without asking.
    Auth,
    /// The host could not be reached, or did not answer in time.
    Offline,
    Other,
}

/// One linked worktree as Git registers it, read with NUL porcelain so no
/// folder name is interpreted.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Registered {
    pub path: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub bare: bool,
    /// Prunable or bare: Git lists it but cannot use it.
    pub unavailable: bool,
}

/// One operator-confirmed worktree deletion, recorded before the host's
/// preflight and reused after Herdr confirms every pane is gone.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfirmedRemoval {
    pub repository_root: String,
    pub checkout_path: String,
    pub expected_head_sha: Option<String>,
    pub expected_branch: Option<String>,
    pub protected_base_branch: Option<String>,
    /// The branch to delete after the folder is gone; `None` keeps it.
    pub delete_branch: Option<String>,
    /// Delete the branch with `git branch -D`: the operator was told it
    /// holds commits the base does not. Otherwise `-d`, which keeps an
    /// unmerged branch. Absent from an older sender, which means `-d`.
    #[serde(default)]
    pub force_delete_branch: bool,
    /// The operator accepted losing the folder's changes and the exact
    /// ignored repositories named below. Git removes with `--force`, while
    /// a lock or an unavailable scan still refuses it.
    /// Absent from an older sender, no discard was accepted.
    #[serde(default)]
    pub discard_changes: bool,
    /// The exact measured names shown in the destructive confirmation.
    #[serde(default)]
    pub expected_ignored_repositories: Vec<String>,
}

/// What a confirmed removal did, as the helper answers it: a stopped
/// removal is an answer too, distinct from a request that got none.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemovalOutcome {
    /// Whether the folder is gone; `false` means the worktree remains.
    pub removed: bool,
    pub message: String,
}

impl From<Result<String, String>> for RemovalOutcome {
    fn from(result: Result<String, String>) -> Self {
        match result {
            Ok(message) => Self {
                removed: true,
                message,
            },
            Err(message) => Self {
                removed: false,
                message,
            },
        }
    }
}

/// A repository's worktrees, as Git reports them. `None` from [`read`] is a
/// folder that is not a repository, which is not a failure.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepositoryWorktrees {
    pub root_path: String,
    pub shared_git_path: Option<String>,
    pub default_branch: Option<String>,
    pub branches: Vec<String>,
    pub base_branch: Option<String>,
    pub base_source: String,
    pub base_branch_fallback: Option<String>,
    pub worktrees: Vec<WorktreeFacts>,
    /// Why this repository has no worktree list. An empty list with no reason
    /// means the repository genuinely has none.
    pub unavailable_reason: Option<String>,
    /// The `origin` remote's URL from the repository's own configuration,
    /// which names its GitHub repository for a core on another machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<String>,
}

/// One worktree's Git facts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeFacts {
    pub path: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    /// Git lists the worktree but its path is not on disk.
    pub missing: bool,
    pub is_main: bool,
    /// `Some("")` is locked without a reason; `None` is unlocked.
    pub lock_reason: Option<String>,
    /// Measured repository boundaries relative to this checkout.
    pub ignored_repositories: Vec<String>,
    pub ignored_scan_unavailable: Option<String>,
    /// Another listed worktree lies inside this one.
    pub nested: bool,
    pub dirty: bool,
    pub changed_file_count: u32,
    pub base_branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub added_lines: u32,
    pub removed_lines: u32,
    pub merged: Option<bool>,
    pub upstream_state: String,
    pub unpushed: Option<Unpushed>,
    pub behind_upstream: Option<u32>,
    pub created_at_unix_ms: Option<u64>,
    pub last_commit_unix_seconds: Option<u64>,
    pub last_commit_subject: Option<String>,
    pub last_fetch_at_unix_ms: Option<u64>,
    pub measured_at_unix_ms: Option<u64>,
    pub unavailable_reason: Option<String>,
}

/// Commits a branch has that its upstream does not, and the upstream's remote.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Unpushed {
    pub remote: String,
    pub count: u32,
}

/// Names the locked checkout and the manual action; never executes this text.
pub fn locked_removal_reason(name: &str, reason: &str) -> String {
    let detail = if reason.is_empty() {
        "Git supplied no lock reason"
    } else {
        reason
    };
    format!(
        "Worktree {name} is locked: {detail}. Unlock it with git worktree unlock before deleting, then refresh its Git state."
    )
}

/// The most repositories one Git watch follows: the worktree reader's cap.
pub const GIT_WATCH_LIMIT: usize = 64;

/// How long a core waits for one repository's worktrees (`Call::Worktrees`).
pub const WORKTREES_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// How long a node measures one repository's worktrees. It is short of
/// [`WORKTREES_TIMEOUT`], so the answer arrives while its core still waits:
/// a read the core gave up on kept measuring on the node beside the read
/// the core asked for next.
pub const WORKTREES_READ_TIME: std::time::Duration = std::time::Duration::from_secs(45);

const _: () = assert!(WORKTREES_READ_TIME.as_secs() + 10 <= WORKTREES_TIMEOUT.as_secs());

/// What a Git watch reports while it runs (`Call::GitWatch`), at least once a
/// second so its caller can end it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "report", rename_all = "snake_case")]
pub enum GitWatchReport {
    /// The watch began; each common directory it could not watch, with why.
    Watching { unwatched: Vec<(String, String)> },
    /// A Git fact changed in each of these common directories.
    Changed { common_dirs: Vec<String> },
    /// Changes were lost: anything in any watched repository may have changed.
    Overflow { reason: String },
    /// Nothing changed since the last report.
    Quiet,
}

/// Whether `relative`, a path under a repository's common directory, names
/// one of the Git facts the worktree reader reads: the head, the index, the
/// refs and the worktrees' own folders. Object writes during a fetch or a gc
/// are not.
pub fn git_fact_path(relative: &std::path::Path) -> bool {
    relative.as_os_str().is_empty()
        || matches!(
            relative.to_str(),
            Some("HEAD" | "index" | "packed-refs" | "FETCH_HEAD")
        )
        || relative.starts_with("refs")
        || relative.starts_with("worktrees")
}
