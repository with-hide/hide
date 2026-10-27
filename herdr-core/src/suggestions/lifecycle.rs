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
    PrOpen,
    CiFailed,
    /// The agent ended with no pull request, or never started.
    Failed,
    IssueOpen,
    Merged,
    /// The pull request was closed unmerged or the issue closed.
    Closed,
}

impl AttemptPhase {
    /// Whether the attempt is still followed.
    pub(crate) fn followed(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Working | Self::PrOpen | Self::CiFailed | Self::IssueOpen
        )
    }
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
}

impl Attempt {
    /// An automatic attempt nobody has reviewed yet, from its start to its
    /// pull request's first review: it holds the next automatic one (D-29).
    pub(crate) fn holds_next_auto(&self) -> bool {
        self.auto
            && !self.reviewed
            && matches!(
                self.phase,
                AttemptPhase::Starting
                    | AttemptPhase::Working
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
    /// Its pull request is open, or its checks pass again.
    PrOpen(PrRef),
    /// Its pull request's checks failed and the agent is not working.
    CiFailed(PrRef),
    /// Someone reviewed its pull request.
    Reviewed,
    Merged {
        pr: PrRef,
        at_unix_ms: u64,
        /// The branch it merged into.
        base: Option<String>,
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
    pub pr: PrRef,
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
