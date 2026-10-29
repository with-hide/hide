//! A branch Hide AI's agent left for the operator to decide (PRD
//! hide-ai-branch-first-apply D-01, D-05, D-07, D-08): what it holds against
//! its base, Hide's own merge of it, and its discard. A 제안 line's attempt
//! (`suggestion_apply`) and a Memory promotion (`memory_promotion`) both end
//! their agent's work on such a branch, and both ask here.
//!
//! Everything here runs on a worker thread, off `Mutex<Runtime>`: the git
//! work is the node's, on the machine that holds the checkout (B25), and the
//! panes are that machine's Herdr. Nothing here pushes.

use super::memory_promotion::{Retire, panes_in, retire_attempt, worktree_path};
use crate::node::NodeId;
use crate::node_access::NodeLink;
use hide_node_link::branch::{BranchFacts, DiscardOutcome, MergeOutcome};
use hide_node_link::protocol::Call;
use serde_json::json;
use std::time::Duration;

/// How long a facts read may take; a merge gets the long bound of a change.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const CHANGE_TIMEOUT: Duration = Duration::from_secs(120);

/// How many branches may wait for the operator's decision in one Project
/// (D-17): past it no new fix starts.
pub(crate) const WAITING_BRANCHES: usize = 5;

/// Where a branch lives: the Project's machine and folder, and the branch
/// and the base it is measured against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BranchPlace {
    pub(super) node: NodeId,
    pub(super) project_id: String,
    pub(super) root: String,
    pub(super) branch: String,
    pub(super) base: String,
}

/// Reads what the branch holds against its base; with `tip`, a branch that
/// is gone is still judged merged or not by the commit it last had (D-08).
pub(super) fn read_facts(
    link: &dyn NodeLink,
    place: &BranchPlace,
    tip: Option<&str>,
) -> Result<BranchFacts, String> {
    crate::node_access::call_as::<BranchFacts>(
        link,
        Call::BranchFacts {
            root: place.root.clone(),
            branch: place.branch.clone(),
            tip: tip.map(str::to_owned),
            base: place.base.clone(),
        },
        READ_TIMEOUT,
    )
    .map_err(|error| error.to_string())
}

/// The operator's `main에 합치기` (B6-B8): the node merges the branch into
/// the base where the base is checked out, fast-forward when it can, and
/// changes nothing when it cannot. On a merge the attempt's agent ends and
/// its worktree goes; the branch stays.
pub(super) fn merge(
    connector: Option<&dyn hide_herdr_client::ApiConnector>,
    link: &dyn NodeLink,
    place: &BranchPlace,
    own_panes: &[String],
) -> Result<MergeOutcome, String> {
    let outcome = crate::node_access::call_as::<MergeOutcome>(
        link,
        Call::BranchMerge {
            root: place.root.clone(),
            branch: place.branch.clone(),
            base: place.base.clone(),
            message: format!("Merge branch '{}'", place.branch),
        },
        CHANGE_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;
    if matches!(
        outcome,
        MergeOutcome::Merged { .. } | MergeOutcome::AlreadyMerged { .. }
    ) && let Some(connector) = connector
    {
        // The merge stands whatever the retire answers; a worktree kept is
        // logged and found in the cleanup review.
        retire_attempt(
            connector,
            link,
            &Retire {
                node: place.node.clone(),
                project_id: place.project_id.clone(),
                root: place.root.clone(),
                branch: place.branch.clone(),
            },
            own_panes,
            false,
        );
    }
    Ok(outcome)
}

/// The operator's `버리기` (D-07, B10): the attempt's agent ends, and its
/// worktree and branch are deleted without asking. A worktree another agent
/// works in is left, and the discard fails for the operator to see again.
pub(super) fn discard(
    connector: &dyn hide_herdr_client::ApiConnector,
    link: &dyn NodeLink,
    place: &BranchPlace,
    own_panes: &[String],
    tip: Option<&str>,
) -> Result<(), String> {
    if let Some(path) = worktree_path(connector, &place.root, &place.branch)? {
        // The branch checked out in the Project's own folder is the
        // operator's, never an attempt's worktree.
        if path.trim_end_matches('/') == place.root.trim_end_matches('/') {
            return Err("the branch is checked out in the Project's own folder".into());
        }
        let panes = panes_in(connector, &path)?;
        let (own, others): (Vec<_>, Vec<_>) =
            panes.into_iter().partition(|pane| own_panes.contains(pane));
        let agents = hide_herdr_client::request_with_connector(
            connector,
            "agent.list",
            crate::wire::empty_params(),
            Duration::from_secs(5),
        )
        .map_err(|error| error.to_string())
        .and_then(|listed| {
            crate::wire::agents_response(listed).map_err(|_| "agent.list is malformed".to_owned())
        })?;
        if agents.iter().any(|agent| others.contains(&agent.pane_id)) {
            return Err("another agent works in the worktree".into());
        }
        let all = own.iter().chain(&others).cloned().collect::<Vec<_>>();
        crate::live::close_checkout_panes(
            connector,
            link,
            std::slice::from_ref(&path),
            &all,
            crate::live::ProcessWait::for_folder_removal(true),
            crate::live::CONFIRM_TIMEOUT,
        )?;
        crate::node_access::call_as::<()>(
            link,
            Call::WorktreeRetire {
                root: place.root.clone(),
                checkout: path,
                branch: place.branch.clone(),
                discard: true,
            },
            CHANGE_TIMEOUT,
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }
    match crate::node_access::call_as::<DiscardOutcome>(
        link,
        Call::BranchDiscard {
            root: place.root.clone(),
            branch: place.branch.clone(),
            tip: tip.map(str::to_owned),
        },
        CHANGE_TIMEOUT,
    )
    .map_err(|error| error.to_string())?
    {
        DiscardOutcome::Deleted | DiscardOutcome::Missing => Ok(()),
        DiscardOutcome::CheckedOut => Err("the branch is checked out".into()),
        DiscardOutcome::Moved => Err("the branch moved since it was read".into()),
    }
}

/// Ends the agent of an attempt whose branch a new agent takes over (`PR
/// 올리기`, `다시 맞추기`), so one worktree never holds two of Hide's agents.
pub(super) fn end_agent(
    connector: &dyn hide_herdr_client::ApiConnector,
    link: &dyn NodeLink,
    pane: &str,
) -> Result<(), String> {
    crate::live::close_checkout_panes(
        connector,
        link,
        &[],
        &[pane.to_owned()],
        crate::live::ProcessWait::for_folder_removal(false),
        crate::live::CONFIRM_TIMEOUT,
    )
}

/// The code a ready row words for a merge that did not happen; `None` when
/// the branch is merged or gone.
pub(super) fn refusal_code(outcome: &MergeOutcome) -> Option<&'static str> {
    match outcome {
        MergeOutcome::Merged { .. }
        | MergeOutcome::AlreadyMerged { .. }
        | MergeOutcome::BranchMissing => None,
        // B7: the operator's own uncommitted work on the same files.
        MergeOutcome::Dirty { .. } => Some("merge_dirty"),
        // B8: the base moved and the branch has to be made again on it.
        MergeOutcome::Conflict => Some("merge_conflict"),
        // The base moved during the merge: pressing again merges it.
        MergeOutcome::BaseMoved => Some("merge_moved"),
    }
}

/// Logs a decision's outcome with ids and codes only (B27).
pub(super) fn log(place: &BranchPlace, subject: &str, kind: &str, outcome: &str) {
    crate::diagnostic!(json!({
        "component": "branch_decision", "kind": kind,
        "project_id": place.project_id, "subject": subject, "outcome": outcome,
    }));
}

/// Whether a Project's agent stopped its turn: it is not at work and asks
/// nothing (a question, an approval or an error is still its turn).
pub(super) fn agent_stopped(activity: &str, demand: &str) -> bool {
    activity == "stopped" && !matches!(demand, "error" | "question" | "approval")
}
