//! The 보기 slot's branch card (PRD hide-ai-branch-first-apply D-02, B4):
//! the files a Hide AI branch changes come with its 제안 row, and the diff of
//! one of them is read here, on the machine that holds the Project, as the
//! branch against its base. The checkout in front, its Agent View and its
//! tab never move for it (B25).

use super::*;
use crate::model::{BranchDiffSnapshot, DetailItem};
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

/// A file of a row's branch: the Project, the row and the path.
pub(super) type BranchFile = (String, String, String);

/// The most branch diffs held at once: the 보기 slot's file and the open
/// 제안 row's.
const BRANCH_DIFFS: usize = 2;

fn shows(diff: &BranchDiffSnapshot, target: &BranchFile) -> bool {
    diff.workspace_id == target.0 && diff.row == target.1 && diff.path == target.2
}

impl Runtime {
    /// The files whose diffs are wanted now: the one the 보기 slot shows and
    /// the one the open 제안 row unfolded.
    fn wanted_branch_diffs(&self) -> Vec<BranchFile> {
        let mut wanted = Vec::with_capacity(BRANCH_DIFFS);
        if let Some(DetailItem::Branch {
            workspace_id,
            row,
            path: Some(path),
        }) = self.snapshot.ui_state.detail.top()
        {
            wanted.push((workspace_id.clone(), row.clone(), path.clone()));
        }
        if let Some(target) = &self.suggestions_work.row_file
            && !wanted.contains(target)
        {
            wanted.push(target.clone());
        }
        wanted
    }

    /// Keeps the diffs wanted now, forgets the others and starts the read of
    /// one not held yet, or of `again` once more.
    pub(super) fn sync_branch_diffs(&mut self, again: Option<&BranchFile>) {
        let wanted = self.wanted_branch_diffs();
        self.snapshot
            .pr_work
            .branch_diffs
            .retain(|diff| wanted.iter().any(|target| shows(diff, target)));
        for target in wanted {
            let held = self
                .snapshot
                .pr_work
                .branch_diffs
                .iter()
                .any(|diff| shows(diff, &target));
            if !held || again == Some(&target) {
                self.read_branch_diff(target);
            }
        }
    }

    /// The open 제안 row's file whose diff it unfolds, or none (PRD
    /// hide-ai-branch-first-apply B4: the diff the 보기 slot reads, read here
    /// for the row too).
    pub(super) fn show_row_branch_file(&mut self, payload: RowBranchFilePayload) -> bool {
        let target = payload
            .path
            .map(|path| (payload.workspace_id, payload.row, path));
        if self.suggestions_work.row_file == target {
            return false;
        }
        self.suggestions_work.row_file = target;
        self.sync_branch_diffs(None);
        true
    }

    /// Starts the read of `path` on the branch of row `row`; what it held
    /// for that file before is replaced.
    fn read_branch_diff(&mut self, target: BranchFile) {
        let (workspace_id, row, path) = target.clone();
        let mut shown = BranchDiffSnapshot {
            workspace_id: workspace_id.clone(),
            row: row.clone(),
            path: path.clone(),
            reading: true,
            ..BranchDiffSnapshot::default()
        };
        let place = self.branch_diff_place(&workspace_id, &row, &path);
        // Stamped before any answer, so a failed read is not read again
        // until the branch moves (`follow_branch_diff`).
        shown.tip = place.as_ref().and_then(|place| place.tip.clone());
        let link = match &place {
            None => {
                log(&workspace_id, &row, "no_branch");
                None
            }
            Some(place) => match self.node_link(&place.node) {
                Ok(link) => Some(link),
                Err(_) => {
                    log(&workspace_id, &row, "node_unavailable");
                    None
                }
            },
        };
        if link.is_none() {
            shown.reading = false;
            shown.failed = true;
        }
        let diffs = &mut self.snapshot.pr_work.branch_diffs;
        diffs.retain(|diff| !shows(diff, &target));
        diffs.push(shown);
        let (Some(place), Some(link)) = (place, link) else {
            return;
        };
        let tip = place.tip.clone();
        let call = Call::BranchDiff {
            root: place.root,
            branch: place.branch,
            base: place.base,
            path,
        };
        let asked = target.clone();
        let started = self.spawn_issue_worker(
            "branch-diff",
            move || crate::node_access::call_as::<Diff>(link.as_ref(), call, READ_TIMEOUT),
            move |runtime, answer| {
                // The diff left, or the branch moved, while the read ran.
                let Some(shown) = runtime
                    .snapshot
                    .pr_work
                    .branch_diffs
                    .iter_mut()
                    .find(|diff| shows(diff, &asked) && diff.tip == tip)
                else {
                    return false;
                };
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
                        log(&asked.0, &asked.1, "read_failed");
                        shown.failed = true;
                    }
                }
                true
            },
        );
        if started.is_err()
            && let Some(shown) = self
                .snapshot
                .pr_work
                .branch_diffs
                .iter_mut()
                .find(|diff| shows(diff, &target))
        {
            shown.reading = false;
            shown.failed = true;
        }
    }

    /// Reads an open file again when its row's branch moved since it was
    /// read: an agent that committed after the diff opened must not leave
    /// the operator looking at the older one.
    pub(super) fn follow_branch_diff(&mut self) {
        let moved = self
            .snapshot
            .pr_work
            .branch_diffs
            .iter()
            .filter_map(|diff| {
                let target = (
                    diff.workspace_id.clone(),
                    diff.row.clone(),
                    diff.path.clone(),
                );
                let place = self.branch_diff_place(&target.0, &target.1, &target.2)?;
                (place.tip != diff.tip).then_some(target)
            })
            .collect::<Vec<_>>();
        for target in moved {
            self.read_branch_diff(target);
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

/// `suggestion_branch_file`: the file the open 제안 row unfolds, or none.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RowBranchFilePayload {
    workspace_id: String,
    row: String,
    path: Option<String>,
}

/// Ids and a code only: never the path, the branch or the text (B27).
fn log(workspace_id: &str, row: &str, outcome: &str) {
    crate::diagnostic!(json!({
        "component": "branch_card", "kind": "branch_card.diff",
        "workspace_id": workspace_id, "row": row, "outcome": outcome,
    }));
}
