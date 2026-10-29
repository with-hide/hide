//! What happens to a line after the notebook raised it (PRD
//! hide-ai-suggestions B10-B16): its raise, the attempt that applies it, the
//! watch after its merge and its escalation. The core keeps this beside the
//! line, never in it, so an investigation reads where a line stands
//! (`Line::stage`) and cannot move it.

use super::ledger::{PrRef, SessionRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One line's story past the notebook.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineLife {
    /// When it was raised as a new suggestion, the last time.
    #[serde(default)]
    pub raised_at_unix_ms: Option<u64>,
    /// The attempt applying it, the last one.
    #[serde(default)]
    pub attempt: Option<Attempt>,
    /// The watch after its fix merged.
    #[serde(default)]
    pub watch: Option<Watch>,
    /// It came back after a merge and is suggested again with a stronger
    /// fix (D-13): the pull request that did not hold.
    #[serde(default)]
    pub escalated_from: Option<PrRef>,
    /// When it rose again after the operator skipped it (D-43); a skip
    /// after that is for good.
    #[serde(default)]
    pub returned_at_unix_ms: Option<u64>,
    /// What it stood on when the operator last skipped it (D-43), kept here
    /// because its records move to newer heads and leave the ledger with
    /// age: a return is measured against this, never against the records
    /// left.
    #[serde(default)]
    pub skipped_with: Option<Evidence>,
    /// A line for the Hide global instruction: what Hide wrote, and why the
    /// last press wrote nothing (PRD memory-self-improve D-07, D-30, D-33).
    #[serde(default)]
    pub local: Option<LocalApply>,
}

/// A global instruction line's story; the file itself is the operator's
/// and Hide's together, so only the line and its times are kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalApply {
    /// The line the last apply wrote, which its Undo takes out.
    #[serde(default)]
    pub line: Option<String>,
    /// The file already had the line when the operator pressed, so Hide
    /// wrote nothing and its Undo takes nothing out (B8, B9).
    #[serde(default)]
    pub adopted: bool,
    /// A write saved before it is made, so a press repeated after a write
    /// whose answer was lost reads the file as that write's (B8, B9).
    #[serde(default)]
    pub writing: Option<LocalWrite>,
    #[serde(default)]
    pub applied_at_unix_ms: Option<u64>,
    /// The operator said it again in a session that was given the line, so
    /// it is offered again (B11).
    #[serde(default)]
    pub recurred_at_unix_ms: Option<u64>,
    #[serde(default)]
    pub failure: Option<LocalFailure>,
}

/// A write to the Hide global instruction that may have landed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalWrite {
    Apply(String),
    Revert(String),
}

/// Why a press on a global instruction line wrote nothing (B5, B8-B10,
/// B29), as the row says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalFailure {
    /// The line would take the file past its cap (`자리 없음`, B10).
    NoRoom,
    /// The file is over its cap as it is.
    TooLarge,
    /// The file is not UTF-8 or cannot be read.
    Unreadable,
    /// The file changed after the preview the operator saw (B8).
    Changed,
    /// The line Hide wrote was edited or removed by hand (B9).
    Edited,
    /// Writing the file failed (B29): `다시 시도`.
    WriteFailed,
    /// No readable words stand behind the line (B7, D-34).
    EvidenceUnconfirmed,
    /// The line carries markup, a character no one would see, or a secret,
    /// so no session is given it.
    BadLine,
}

impl LocalFailure {
    pub(crate) fn of(refusal: &crate::global_instruction::Refusal) -> Self {
        use crate::global_instruction::{Refusal, Unusable};
        match refusal {
            Refusal::Unusable {
                unusable: Unusable::TooLarge,
            } => Self::TooLarge,
            Refusal::Unusable {
                unusable: Unusable::Unreadable,
            } => Self::Unreadable,
            Refusal::NoRoom => Self::NoRoom,
            Refusal::BadLine => Self::BadLine,
            Refusal::Changed { .. } => Self::Changed,
            Refusal::Edited { .. } => Self::Edited,
            Refusal::WriteFailed => Self::WriteFailed,
        }
    }
}

/// The sessions and pull requests behind a line: the units its score
/// counts (D-12).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub sessions: BTreeSet<SessionRef>,
    #[serde(default)]
    pub prs: BTreeSet<u32>,
}

impl Evidence {
    pub(crate) fn units(&self) -> usize {
        self.sessions.len() + self.prs.len()
    }
}

impl LineLife {
    /// Whether the operator's skip of the line may bring it back once
    /// (D-43): only until it has come back once, and only when the skip
    /// kept what the line stood on; a skip that kept nothing is for good.
    pub(crate) fn skip_may_return(&self) -> bool {
        self.returned_at_unix_ms.is_none() && self.skipped_with.is_some()
    }

    /// Whether its last rise was its return after a skip, which the row
    /// says (`넘어갔지만 다시 생겼어요`).
    pub(crate) fn rose_after_skip(&self) -> bool {
        self.returned_at_unix_ms
            .is_some_and(|at| self.raised_at_unix_ms == Some(at))
    }
}

/// Where an attempt stands (B11-B14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    /// Written before the spawn; the agent has not shown yet.
    Starting,
    Working,
    /// The agent committed on its branch and stopped: the operator's turn to
    /// merge it, open a pull request or discard it (PRD
    /// hide-ai-branch-first-apply D-01, D-02).
    Ready,
    PrOpen,
    CiFailed,
    /// The agent ended with no pull request, or never started.
    Failed,
    IssueOpen,
    Merged,
    /// The pull request was closed unmerged, the branch was discarded, or the
    /// issue closed.
    Closed,
}

impl AttemptPhase {
    /// Whether the attempt is still followed.
    pub(crate) fn followed(self) -> bool {
        matches!(
            self,
            Self::Starting
                | Self::Working
                | Self::Ready
                | Self::PrOpen
                | Self::CiFailed
                | Self::IssueOpen
        )
    }
}

/// What an attempt's agent is asked to do on its branch (PRD
/// hide-ai-branch-first-apply D-01, D-06, B12).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPurpose {
    /// Commit the fix on the branch and stop.
    Branch,
    /// Push the branch and open one pull request. An attempt saved before
    /// branch-first apply has no purpose and was asked for a pull request.
    #[default]
    Pr,
    /// Make the branch again on the base as it is now and stop.
    Rebase,
}

/// What a ready branch holds against its base, as the node read it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchSummary {
    pub base: String,
    /// The branch's commit when it was read, which tells a merge done by
    /// hand from a branch deleted unmerged once the branch is gone (D-08).
    pub tip: String,
    pub ahead: u32,
    #[serde(default)]
    pub files: Vec<BranchFileSummary>,
    /// Every changed file, of which `files` keeps the first few.
    #[serde(default)]
    pub files_total: u32,
    pub added: u32,
    pub removed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchFileSummary {
    /// Repository-relative.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    #[serde(default)]
    pub binary: bool,
}

/// One go at applying a line: one agent in one worktree on one branch, with
/// Hide as its origin (D-17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub started_at_unix_ms: u64,
    /// Hide applied it without the operator's press (D-28).
    pub auto: bool,
    pub phase: AttemptPhase,
    /// The branch it works on and the name its agent is listed under.
    pub branch: String,
    pub agent_name: String,
    /// The agent's registration once the core saw it.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The repository the attempt was started against (`owner/name`), which
    /// a pull request must belong to before it settles the attempt.
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub pr: Option<PrRef>,
    #[serde(default)]
    pub issue_url: Option<String>,
    /// An issue line's title, kept so a create whose answer was lost is found
    /// again by it (B14).
    #[serde(default)]
    pub issue_title: Option<String>,
    /// Since when an earlier create of this line's issue may have landed
    /// unconfirmed: a retry looks for that issue before creating another.
    #[serde(default)]
    pub issue_since: Option<u64>,
    /// Why it failed, as a code the screen words.
    #[serde(default)]
    pub failure: Option<String>,
    /// The operator has looked at the pull request: an automatic one no
    /// longer holds the next (D-29).
    #[serde(default)]
    pub reviewed: bool,
    #[serde(default)]
    pub purpose: AttemptPurpose,
    /// What the branch holds once it is ready; kept past a pull request so
    /// the row still names the branch.
    #[serde(default)]
    pub branch_facts: Option<BranchSummary>,
}

impl Attempt {
    /// An automatic attempt nobody has decided yet, from its start until the
    /// operator chose what to do with its branch, or reviewed its pull
    /// request: it holds the next automatic one (D-29, PRD
    /// hide-ai-branch-first-apply D-04).
    pub(crate) fn holds_next_auto(&self) -> bool {
        self.auto
            && !self.reviewed
            && matches!(
                self.phase,
                AttemptPhase::Starting
                    | AttemptPhase::Working
                    | AttemptPhase::Ready
                    | AttemptPhase::PrOpen
                    | AttemptPhase::CiFailed
            )
    }
}

/// What the runtime saw happen to an attempt, handed back for the worker to
/// save (B11-B14). Each names the attempt by its start, so a move meant for
/// an earlier attempt of the same line changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttemptMove {
    /// Its agent showed, with its coordination id and the repository it
    /// works against.
    Started {
        agent_id: String,
        repository: Option<String>,
    },
    /// Its agent stopped with commits on the branch: the operator's turn.
    Ready(BranchSummary),
    /// Hide's merge or discard of a ready branch did not happen, with the
    /// code the row words (`merge_dirty`, `merge_conflict`, `merge_moved`,
    /// `merge_failed`, `discard_failed`); the branch stays ready.
    DecisionRefused(String),
    /// The branch was discarded, or deleted unmerged: the line is skipped as
    /// `넘어가기` skips it (D-07, D-08).
    Discarded,
    /// Its pull request is open, or its checks pass again.
    PrOpen(PrRef),
    /// Its pull request's checks failed and the agent is not working.
    CiFailed(PrRef),
    /// Someone reviewed its pull request.
    Reviewed,
    /// It reached its base (D-08): its pull request merged, or its branch
    /// merged locally, by Hide or by hand, with no pull request.
    Merged {
        pr: Option<PrRef>,
        at_unix_ms: u64,
        /// The branch it merged into.
        base: Option<String>,
        /// The commit on the base that holds it, when known without GitHub.
        merge_commit: Option<String>,
    },
    /// Its pull request closed unmerged: the operator's `넘어가기`.
    Closed,
    IssueOpen {
        url: String,
    },
    IssueClosed,
    /// It ended with no pull request or issue, with a code the screen words.
    Failed(String),
}

/// The watch after a merge (B15): counted only from sessions and pull
/// requests in checkouts that hold the merge commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watch {
    pub merged_at_unix_ms: u64,
    /// The pull request that merged; none for a branch merged locally (PRD
    /// hide-ai-branch-first-apply B14).
    pub pr: Option<PrRef>,
    /// The branch the pull request merged into, which a checkout must have
    /// caught up with for its records to count.
    #[serde(default)]
    pub base: Option<String>,
    /// A CI line ends its watch sooner after the pull request closed.
    pub ci: bool,
    /// The checkouts that hold the merge, by path on the Project's machine,
    /// as the runtime last judged them from the node's worktree facts.
    #[serde(default)]
    pub holding: BTreeSet<String>,
    /// Sessions in a checkout holding the merge, so far.
    #[serde(default)]
    pub sessions: BTreeSet<SessionRef>,
    /// The commit the merge made on the base, once GitHub named it.
    #[serde(default)]
    pub merge_commit: Option<String>,
    /// Each pull request head asked about, and whether its history has the
    /// merge commit: a pull request's record counts only on a head shown to
    /// have it.
    #[serde(default)]
    pub heads: BTreeMap<String, bool>,
}
