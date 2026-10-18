//! What the engine asks of the world. The engine knows these traits only;
//! the daemon supplies the real ones (local git, `gh`, Herdr workers, the
//! judgment provider) and the tests supply recording fakes.
//!
//! Every call runs on the engine's own thread, never under the core's
//! runtime lock. A call that can take minutes (a verify command, a judgment)
//! is started and polled instead of waited for.

use serde::{Deserialize, Serialize};

use crate::judgment::{Judgment, JudgmentAnswer};
use crate::model::{Card, Factory, IssueRef, MergeMethod, PullRequest, Runtime, Task, UnixMs};

pub trait Clock {
    fn now(&self) -> UnixMs;
    /// The local time zone's offset from UTC in milliseconds, east positive;
    /// the flow's 완료 오늘 counts by the local day it names.
    fn utc_offset_ms(&self) -> i64;
}

/// A structured failure an adapter reports. `signal` is set only for the
/// structured environment signals of D-31 (ENOSPC, exit 137, GitHub
/// 401/403/429/5xx, network, Herdr socket, usage limit); everything else is
/// the Task's (B58).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub stage: String,
    pub signal: Option<EnvSignal>,
    /// A diagnostic detail for the log; never a token or card text.
    pub detail: String,
    /// GitHub scope missing for a 403 (B59).
    pub missing_scope: Option<String>,
    /// Usage limit reset time (B58).
    pub reset_at: Option<UnixMs>,
    /// The worker's pane exists but its agent has not shown a session yet
    /// (a first-run prompt, a slow start): not a failure, ask again later.
    pub starting: bool,
    /// When a still-starting worker is asked about again; the engine's own
    /// interval when absent.
    #[serde(default)]
    pub again_in_ms: Option<u64>,
}

impl Failure {
    pub fn task(stage: &str, detail: impl Into<String>) -> Self {
        Self {
            stage: stage.to_owned(),
            signal: None,
            detail: detail.into(),
            missing_scope: None,
            reset_at: None,
            starting: false,
            again_in_ms: None,
        }
    }

    /// A start the adapter is carrying out off the engine's thread: ask
    /// again on the next tick.
    pub fn start_pending(stage: &str) -> Self {
        Self {
            again_in_ms: Some(0),
            ..Self::starting(stage, "start_in_flight")
        }
    }

    pub fn starting(stage: &str, detail: impl Into<String>) -> Self {
        Self {
            starting: true,
            ..Self::task(stage, detail)
        }
    }

    pub fn environment(stage: &str, signal: EnvSignal, detail: impl Into<String>) -> Self {
        Self {
            stage: stage.to_owned(),
            signal: Some(signal),
            detail: detail.into(),
            missing_scope: None,
            reset_at: None,
            starting: false,
            again_in_ms: None,
        }
    }
}

/// The structured environment signals (D-31 rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvSignal {
    DiskFull,
    OutOfMemory,
    GithubAuth,
    GithubForbidden,
    GithubRateLimit,
    GithubServer,
    Network,
    HerdrSocket,
    /// The machine that holds the Factory's project is not linked to the
    /// core (PRD core-host-node-move Q17): its work waits for the link.
    NodeLink,
    UsageLimit,
}

impl EnvSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DiskFull => "disk_full",
            Self::OutOfMemory => "out_of_memory",
            Self::GithubAuth => "github_auth",
            Self::GithubForbidden => "github_forbidden",
            Self::GithubRateLimit => "github_rate_limit",
            Self::GithubServer => "github_server",
            Self::Network => "network",
            Self::HerdrSocket => "herdr_socket",
            Self::NodeLink => "node_link",
            Self::UsageLimit => "usage_limit",
        }
    }
}

/// What `init` detected for a project (B1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectProbe {
    pub github: bool,
    pub repo: Option<String>,
    /// The `gh` login a GitHub Factory acts as.
    pub account: Option<String>,
    /// Required checks of the default branch's protection.
    pub required_checks: Vec<String>,
    /// Verify command candidates read from repository files.
    pub verify_candidates: Vec<String>,
    pub merge_methods: Vec<MergeMethod>,
    pub default_branch: String,
    /// Agent runtimes on this machine's path, Claude Code first (D-45).
    pub runtimes: Vec<Runtime>,
}

/// What changed outside the Factory since the last read (D-27).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutsideEvent {
    /// An open pull request not made by the Factory closes a Task's issue.
    ClosingPr {
        issue: IssueRef,
        pr: u64,
        url: String,
        merged: bool,
    },
    /// The issue was closed with no pull request.
    IssueClosed {
        issue: IssueRef,
    },
    IssueReopened {
        issue: IssueRef,
    },
    LabelRemoved {
        issue: IssueRef,
    },
    /// A person put the `factory` label on an issue the Factory does not
    /// hold yet (B15).
    Labeled {
        issue: IssueRef,
        title: String,
        body: String,
    },
    BodyEdited {
        issue: IssueRef,
        body_hash: String,
        body: String,
    },
    /// A push to main the Factory did not make (B47).
    OutsidePush {
        sha: String,
    },
}

/// Issues, labels and outside work (GitHub or local).
pub trait TaskSource {
    /// What `project` on the machine `node` names holds (`Factory::node`).
    fn probe(&mut self, node: Option<&str>, project: &str) -> Result<ProjectProbe, Failure>;
    fn prepare(&mut self, factory: &Factory) -> Result<(), Failure>;
    /// Creates the Task's issue with the card summary and the label; converges
    /// on an existing issue carrying the same Task marker (B73).
    fn create_issue(
        &mut self,
        factory: &Factory,
        task: &Task,
        body: &str,
    ) -> Result<IssueRef, Failure>;
    fn label_issue(&mut self, factory: &Factory, issue: &IssueRef) -> Result<(), Failure>;
    fn read_issue(&mut self, factory: &Factory, issue: &IssueRef) -> Result<IssueText, Failure>;
    fn observe(&mut self, factory: &Factory, tasks: &[&Task])
    -> Result<Vec<OutsideEvent>, Failure>;
    /// The names in the project's top folder and its guide, for a review
    /// judgment; a project that cannot be read answers neither.
    fn repo_context(&mut self, node: Option<&str>, project: &str) -> RepoContext;
    /// The PRD file at `path` on the machine `node` names that a Task
    /// attaches; `Err` names why it cannot be read.
    fn read_prd(&mut self, node: Option<&str>, path: &str) -> Result<Vec<u8>, String>;
    /// What the repository and GitHub say about a card before its review
    /// (D-02): the files it names and the issues and pull requests that look
    /// related. What cannot be read is left out.
    fn intake_facts(&mut self, _factory: &Factory, _card: &Card) -> IntakeFacts {
        IntakeFacts::default()
    }
    /// Creates an issue for a follow-up candidate, with the factory label
    /// when `labelled` (the Factory picks it up and starts it) and without
    /// it otherwise; converges on the issue whose body carries `marker`
    /// (D-31).
    fn create_follow_up(
        &mut self,
        factory: &Factory,
        title: &str,
        body: &str,
        marker: &str,
        labelled: bool,
    ) -> Result<IssueRef, Failure>;
    /// Whether GitHub takes the Factory's sign-in and reads again, for the
    /// recheck a person presses after signing in (D-46).
    fn check_access(&mut self, _factory: &Factory) -> Result<(), Failure> {
        Ok(())
    }
    /// Whether the machine `node` names has the agent a worker candidate
    /// names (B28).
    fn installed(&mut self, _node: Option<&str>, _agent: Runtime) -> bool {
        true
    }
}

/// What an intake review checks before it asks (D-02).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeFacts {
    /// The start of each repository file the card names.
    pub files: Vec<FileFact>,
    /// Issues and pull requests that look related.
    pub related: Vec<RelatedItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFact {
    pub path: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedItem {
    /// `issue` or `pull_request`.
    pub kind: String,
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

/// What a review judgment is told of a project (`repo_context`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoContext {
    pub files: Vec<String>,
    pub guide: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueText {
    pub title: String,
    pub body: String,
    pub open: bool,
}

/// A verification run the engine polls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyRun {
    pub id: String,
    pub log: Option<String>,
    /// The commit this run verifies, when it is known at the start: the
    /// merge is pinned to it (B39).
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VerifyPoll {
    Pending,
    Passed,
    Failed {
        check: String,
        link: String,
    },
    Environment {
        signal: EnvSignal,
        check: String,
    },
    /// The answer could not be read, and no environment signal says why: it
    /// decides nothing, and the run is asked again.
    Unread {
        check: String,
        detail: String,
    },
}

/// CI required checks or the verify bundle (D-53).
pub trait Verifier {
    /// Starts the Task-stage verification (after done).
    fn start(&mut self, factory: &Factory, task: &Task) -> Result<VerifyRun, Failure>;
    /// Starts the bundle on the latest main merged into the Task (B38).
    fn start_premerge(&mut self, factory: &Factory, task: &Task) -> Result<VerifyRun, Failure>;
    fn poll(&mut self, factory: &Factory, run: &VerifyRun) -> VerifyPoll;
    fn cancel(&mut self, factory: &Factory, run: &VerifyRun);
    /// The end of a run's log, where the run wrote one.
    fn log_tail(&self, factory: &Factory, log: &str) -> Option<String>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PreMerge {
    Clean,
    Conflict {
        files: Vec<String>,
    },
    QuickCheckFailed {
        check: String,
    },
    /// The Task touches a configured risk path (B41).
    RiskPath {
        paths: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MainCheck {
    Pending,
    Green,
    Red {
        link: String,
    },
    /// No main verification configured.
    None,
}

/// Branches, pull requests, merges and reverts.
pub trait MergeTarget {
    fn main_head(&mut self, factory: &Factory) -> Result<String, Failure>;
    /// Reads, without merging, whether the Task's pull request merged: its
    /// merge commit, `None` when it has not merged, or a failure asking
    /// again when GitHub merged it without naming the commit yet.
    fn merged_commit(
        &mut self,
        _factory: &Factory,
        _task: &Task,
    ) -> Result<Option<String>, Failure> {
        Ok(None)
    }
    /// Pushes the worktree's commits to the Task branch on every report, then
    /// finds the open pull request for it (a harness's or an earlier
    /// report's) or opens one (B36).
    fn open_pr(
        &mut self,
        factory: &Factory,
        task: &Task,
        body: &str,
    ) -> Result<Option<PullRequest>, Failure>;
    fn close_pr(&mut self, factory: &Factory, pr: &PullRequest) -> Result<(), Failure>;
    fn reopen_pr(&mut self, factory: &Factory, pr: &PullRequest) -> Result<(), Failure>;
    fn diff_lines(&mut self, factory: &Factory, task: &Task) -> Result<u32, Failure>;
    fn changed_paths(&mut self, factory: &Factory, task: &Task) -> Result<Vec<String>, Failure>;
    /// The Task's diff against main, for the drift and user checks (D-44).
    fn diff_text(&mut self, factory: &Factory, task: &Task) -> Result<String, Failure>;
    /// merge-tree against the current main and the quick check (B38).
    fn premerge(&mut self, factory: &Factory, task: &Task) -> Result<PreMerge, Failure>;
    /// Whether the local main checkout has uncommitted changes (B42).
    fn main_dirty(&mut self, factory: &Factory) -> Result<bool, Failure>;
    /// Merges with the head SHA pinned; converges on an already-merged PR.
    fn merge(
        &mut self,
        factory: &Factory,
        task: &Task,
        method: MergeMethod,
    ) -> Result<String, Failure>;
    /// Main verification of a commit (B43, B44).
    fn main_check(&mut self, factory: &Factory, sha: &str) -> Result<MainCheck, Failure>;
    /// Re-runs a skipped or cancelled main run (B45).
    fn rerun_main(&mut self, factory: &Factory, sha: &str) -> Result<(), Failure>;
    /// Reverts a merge (revert PR on GitHub, revert commit locally).
    fn revert(&mut self, factory: &Factory, task: &Task, sha: &str) -> Result<RevertRef, Failure>;
    fn revert_check(&mut self, factory: &Factory, revert: &RevertRef)
    -> Result<MainCheck, Failure>;
    fn merge_revert(&mut self, factory: &Factory, revert: &RevertRef) -> Result<String, Failure>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertRef {
    pub task: String,
    pub sha: String,
    pub pr: Option<u64>,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSpawn {
    pub factory: String,
    /// The machine the worker starts on (`Factory::node`).
    #[serde(default)]
    pub node: Option<String>,
    pub task: String,
    pub name: String,
    pub runtime: Runtime,
    pub project: String,
    pub branch: String,
    pub prompt: String,
    /// The runtime's extra arguments from the Factory's configuration.
    pub args: Vec<String>,
    /// The candidate's model and effort: the spawn's launch choice, which its
    /// start and every restart of the worker carry, and kept on the worker.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Reuse the worktree and session (retry, wake, relanding).
    pub resume: Option<crate::model::WorkerRef>,
    /// How many earlier fresh starts were refused: each attempt is its own
    /// spawn intent, and the same attempt asked again converges.
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Working,
    /// At rest (idle or done) since the given time.
    Resting {
        since: UnixMs,
    },
    Blocked,
    Gone,
    /// The agent's activity cannot be read: never counted as resting (B43).
    Unknown,
}

/// What a resting worker last showed, each as its adapter declares it can
/// be read; `None` when undeclared or empty (D-37, D-52).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkerTexts {
    pub user_turn: Option<String>,
    pub last_answer: Option<String>,
    pub screen: Option<String>,
}

/// How much of a Task's worktree a removal may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// A finished Task: a clean worktree goes and its branch stays with the
    /// merge; uncommitted leftovers keep the worktree and fail the removal,
    /// so a person sees them (D-58).
    Finished,
    /// A Task cancelled or taken over past its keep period, or a start
    /// abandoned on its way: what is left goes, with the local branch.
    Discarded,
}

/// Starts, messages, sleeps and wakes workers (D-14).
pub trait WorkerRuntime {
    fn spawn(&mut self, request: &WorkerSpawn) -> Result<crate::model::WorkerRef, Failure>;
    /// The engine no longer wants the worker a pending start makes (the
    /// Task was cancelled or paused while it started); the adapter ends it
    /// when it arrives. True when the adapter took a start over, so the next
    /// start is a new attempt; false when it held none, so the next start
    /// asks the same spawn again and gets what it already reserved (rule 11).
    fn abandon_start(&mut self, _factory: &str, _task: &str) -> bool {
        false
    }
    /// Sends a reply or wake message through the mailbox.
    fn message(
        &mut self,
        worker: &crate::model::WorkerRef,
        intent: &str,
        reply_to: Option<&str>,
        body: &str,
    ) -> Result<(), Failure>;
    fn sleep(&mut self, worker: &crate::model::WorkerRef) -> Result<(), Failure>;
    fn wake(&mut self, worker: &crate::model::WorkerRef, body: &str) -> Result<(), Failure>;
    fn status(&mut self, worker: &crate::model::WorkerRef) -> WorkerStatus;
    fn stop(&mut self, worker: &crate::model::WorkerRef) -> Result<(), Failure>;
    /// Removes the Task's worktree (B43, B62, B71); the remote branch stays
    /// (D-58). Removing what is already gone succeeds.
    fn remove_worktree(
        &mut self,
        worker: &crate::model::WorkerRef,
        removal: Removal,
    ) -> Result<(), Failure>;
    fn usage_limited(&mut self, runtime: Runtime) -> Option<UnixMs>;
    /// The worker's texts a diagnosis may read.
    fn texts(&mut self, _worker: &crate::model::WorkerRef) -> WorkerTexts {
        WorkerTexts::default()
    }
}

/// Submits judgments and hands back finished answers (D-13, D-44).
pub trait Judge {
    /// Queues a judgment; refused when the Factory's queue is full.
    fn submit(&mut self, judgment: Judgment) -> Result<(), Failure>;
    /// Answers finished since the last call.
    fn finished(&mut self) -> Vec<JudgmentAnswer>;
    /// Whether a Factory AI choice names an agent, model and effort Hide AI
    /// can run; the reason when it cannot (B31).
    fn check_ai(&mut self, _ai: &crate::model::FactoryAi) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPressure {
    Normal,
    Warn,
    Critical,
}

pub trait Environment {
    /// Free space where `project` lives on the machine `node` names.
    fn disk_free(&mut self, node: Option<&str>, project: &str) -> Option<u64>;
    /// Memory pressure on the machine `node` names.
    fn memory_pressure(&mut self, node: Option<&str>) -> MemoryPressure;
    /// The operator's language, read when a judgment is queued or a worker
    /// starts, so a change in Settings applies to the next one.
    fn language(&mut self) -> crate::words::Language;
    /// The `hide` a worker on the machine `node` names runs, when it is not
    /// the one the engine was given at its open (the core's own).
    fn hide_program(&mut self, _node: Option<&str>) -> Option<String> {
        None
    }
}

/// Where a person is told (D-30): an inbox item exists in the store; this
/// only reaches a producer pane. The macOS notification is the desktop
/// app's, read from the summary (D-50).
pub trait Notifier {
    /// Sends a pending review's result to the producer pane; false when the
    /// pane is gone (B11).
    fn producer(&mut self, factory: &str, pane: &str, body: &str) -> bool;
}
