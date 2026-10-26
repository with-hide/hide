//! What happens to a line after the notebook raised it (PRD
//! hide-ai-suggestions B10-B16): its raise, the attempt that applies it, the
//! watch after its merge and its escalation. The core keeps this beside the
//! line, never in it, so an investigation reads where a line stands
//! (`Line::stage`) and cannot move it.

use super::ledger::{PrRef, SessionRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
    /// Why it failed, as a code the screen words.
    #[serde(default)]
    pub failure: Option<String>,
    /// The operator has looked at the pull request: an automatic one no
    /// longer holds the next (D-29).
    #[serde(default)]
    pub reviewed: bool,
}

/// The watch after a merge (B15): counted only from sessions and pull
/// requests in checkouts that hold the merge commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watch {
    pub merged_at_unix_ms: u64,
    pub pr: PrRef,
    /// The merge commit a checkout must hold for its records to count.
    pub merge_commit: Option<String>,
    /// A CI line ends its watch sooner after the pull request closed.
    pub ci: bool,
    /// Sessions in a checkout holding the merge, so far.
    #[serde(default)]
    pub sessions: BTreeSet<SessionRef>,
}
