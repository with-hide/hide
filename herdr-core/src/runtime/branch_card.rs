//! The 보기 slot's branch card (PRD hide-ai-branch-first-apply D-02, B4):
//! the files a Hide AI branch changes come with its 제안 row, and the diff of
//! one of them is read here, on the machine that holds the Project, as the
//! branch against its base. The checkout in front, its Agent View and its
//! tab never move for it (B25).

use super::*;
use crate::model::BranchDiffSnapshot;
use hide_node_link::git::{Diff, DiffNotice};
use hide_node_link::protocol::Call;
use serde_json::json;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// What the open card's file is read against.
struct DiffPlace {
    node: String,
    root: String,
    branch: String,
    base: String,
    tip: Option<String>,
}

impl Runtime {
    /// Starts the read of `path` on the branch of row `row`, or forgets the
    /// diff when the slot shows no branch file.
    pub(super) fn read_branch_diff(&mut self, target: Option<(String, String, String)>) {
        let Some((workspace_id, row, path)) = target else {
            self.snapshot.pr_work.branch_diff = None;
            return;
        };
        let mut shown = BranchDiffSnapshot {
            workspace_id: workspace_id.clone(),
            row: row.clone(),
            path: path.clone(),
            reading: true,
            ..BranchDiffSnapshot::default()
        };
        let Some(place) = self.branch_diff_place(&workspace_id, &row, &path) else {
            log(&workspace_id, &row, "no_branch");
            shown.reading = false;
            shown.failed = true;
            self.snapshot.pr_work.branch_diff = Some(shown);
            return;
        };
        let link = match self.node_link(&place.node) {
            Ok(link) => link,
            Err(_) => {
                log(&workspace_id, &row, "node_unavailable");
                shown.reading = false;
                shown.failed = true;
                self.snapshot.pr_work.branch_diff = Some(shown);
                return;
            }
        };
        let tip = place.tip.clone();
        shown.tip = tip.clone();
        self.snapshot.pr_work.branch_diff = Some(shown);
        let call = Call::BranchDiff {
            root: place.root,
            branch: place.branch,
            base: place.base,
            path: path.clone(),
        };
        let started = self.spawn_issue_worker(
            "branch-diff",
            move || crate::node_access::call_as::<Diff>(link.as_ref(), call, READ_TIMEOUT),
            move |runtime, answer| {
                let Some(shown) = runtime.snapshot.pr_work.branch_diff.as_mut() else {
                    return false;
                };
                // The slot moved on, or the branch did, while the read ran.
                if shown.workspace_id != workspace_id
                    || shown.row != row
                    || shown.path != path
                    || shown.tip != tip
                {
                    return false;
                }
                shown.reading = false;
                match answer {
                    Ok(diff) => {
                        shown.notice = diff
                            .kind
                            .clone()
                            .or_else(|| diff.notice.as_ref().map(|_| DiffNotice::Unreadable));
                        shown.text = Some(diff.text);
                    }
                    Err(_) => {
                        log(&workspace_id, &row, "read_failed");
                        shown.failed = true;
                    }
                }
                true
            },
        );
        if started.is_err()
            && let Some(shown) = self.snapshot.pr_work.branch_diff.as_mut()
        {
            shown.reading = false;
            shown.failed = true;
        }
    }

    /// Reads the open file again when its row's branch moved since it was
    /// read: an agent that committed after the card opened must not leave
    /// the operator looking at the older diff.
    pub(super) fn follow_branch_diff(&mut self) {
        let Some(shown) = self.snapshot.pr_work.branch_diff.as_ref() else {
            return;
        };
        let target = (
            shown.workspace_id.clone(),
            shown.row.clone(),
            shown.path.clone(),
        );
        let Some(place) = self.branch_diff_place(&target.0, &target.1, &target.2) else {
            return;
        };
        if place.tip != shown.tip {
            self.read_branch_diff(Some(target));
        }
    }

    /// The Project's machine and folder and the row's branch and base, from
    /// the 제안 tab the rail keeps open for the Project in front; a file the
    /// branch does not change is not read.
    fn branch_diff_place(&self, workspace_id: &str, row: &str, path: &str) -> Option<DiffPlace> {
        let suggestions = self.snapshot.suggestions.as_ref()?.project.as_ref()?;
        if suggestions.workspace_id != workspace_id {
            return None;
        }
        let branch = suggestions
            .rows
            .iter()
            .find(|candidate| candidate.id == row)?
            .attempt
            .as_ref()?
            .branch
            .as_ref()?;
        if !branch.files.iter().any(|file| file.path == path) {
            return None;
        }
        let workspace = self
            .catalog_workspaces()
            .find(|workspace| workspace.id == workspace_id)?;
        Some(DiffPlace {
            node: workspace.device_id.clone(),
            root: workspace.path.clone(),
            branch: branch.name.clone(),
            base: branch.base.clone()?,
            tip: branch.tip.clone(),
        })
    }
}

/// Ids and a code only: never the path, the branch or the text (B27).
fn log(workspace_id: &str, row: &str, outcome: &str) {
    crate::diagnostic!(json!({
        "component": "branch_card", "kind": "branch_card.diff",
        "workspace_id": workspace_id, "row": row, "outcome": outcome,
    }));
}
