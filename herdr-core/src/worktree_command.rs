//! `hide workspace remove <path> [--delete-branch] [--force]`: an agent's
//! removal of a worktree Hide shows, through the delete dialog's own path.
//!
//! The runtime admits it under its lock (`Runtime::worktree_remove_command`)
//! and starts the same removal the dialog's `remove_worktree` event starts:
//! the deletion gate, the host's recheck, the close of the checkout's
//! workspace and panes, then Git. The daemon waits for that removal's record
//! to end ([`answer`]) and answers the caller with how it ended, so a raw
//! `git worktree remove` never leaves the workspace behind.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{WorktreeRemovalNotice, WorktreeRemovalSnapshot};

/// How long the daemon waits for a removal it started to end: past the
/// host's recheck and removal bounds (`worktree_control`, 120 s each) and the
/// close in between, so the core's own answer arrives before the wait ends.
pub const REMOVE_WAIT: Duration = Duration::from_secs(280);

/// One command, as the CLI sends it to the daemon.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRemove {
    /// The checkout's absolute path on the caller's machine, spelled with
    /// `/` between names (`hide_platform::path::to_wire`).
    pub path: String,
    /// The local branch goes with the folder, as the dialog's checkbox.
    #[serde(default)]
    pub delete_branch: bool,
    /// Accepts every loss the deletion gate names: uncommitted or untracked
    /// files, a worktree or ignored repository inside it, and, with
    /// `delete_branch`, commits the base does not have.
    #[serde(default)]
    pub force: bool,
}

impl WorktreeRemove {
    /// The path is one a caller could name a checkout by.
    pub fn valid(&self) -> bool {
        self.path.len() <= 4096
            && !self.path.chars().any(char::is_control)
            && hide_platform::path::is_wire_absolute(&self.path)
    }
}

/// `hide workspace remove`: the worktree removed, with its workspace and
/// panes closed first.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeRemoved {
    /// The checkout's path, as the command named it.
    pub removed: String,
    /// The repository it was a worktree of.
    pub repository: String,
    /// Its local branch, when it had one.
    pub branch: Option<String>,
    /// What became of that branch: `kept` (not asked to go), `deleted`, or
    /// `remains` (Git refused to delete it).
    pub branch_result: String,
}

/// The answer schemas of this command, by the name the CLI contract gives
/// each.
pub fn answer_schemas() -> Vec<(&'static str, Value)> {
    vec![(
        "worktree_removed",
        serde_json::to_value(schemars::schema_for!(WorktreeRemoved)).expect("a schema serializes"),
    )]
}

/// The refusals whose detail (what would be lost, what still runs, why Git
/// stopped) the next action carries (`coordination::split_refusal`).
pub const DETAILED: [&str; 8] = [
    "worktree_remove_blocked",
    "worktree_remove_needs_force",
    "worktree_remove_branch_unmerged",
    "worktree_remove_agent_working",
    "worktree_remove_busy",
    "worktree_remove_stopped",
    "worktree_remove_still_running",
    "worktree_remove_host_failed",
];

/// How a removal's record answers the command that started it, once it has
/// ended; `None` while it runs.
pub fn answer(removal: &WorktreeRemovalSnapshot) -> Option<Result<Value, String>> {
    let refused = |code: &str, detail: Option<String>| -> Result<Value, String> {
        Err(match detail {
            Some(detail) => crate::coordination::refusal(code, &detail),
            None => code.to_owned(),
        })
    };
    match (removal.phase.as_str(), removal.notice.as_ref()) {
        ("finished", Some(WorktreeRemovalNotice::Removed { branch })) => Some(
            serde_json::to_value(WorktreeRemoved {
                removed: removal.checkout_path.clone(),
                repository: removal.repository_root.clone(),
                branch: removal.branch.clone(),
                branch_result: serde_json::to_value(branch)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            })
            .map_err(|_| "worktree_remove_result_unknown".to_owned()),
        ),
        ("failed", Some(notice)) => Some(match notice {
            WorktreeRemovalNotice::Stopped { stop, panes_closed } => refused(
                "worktree_remove_stopped",
                Some(format!(
                    "{}{}",
                    serde_json::to_string(stop).unwrap_or_default(),
                    if *panes_closed {
                        "; its panes were closed"
                    } else {
                        "; no pane was closed"
                    }
                )),
            ),
            WorktreeRemovalNotice::StillRunning { programs } => refused(
                "worktree_remove_still_running",
                Some(
                    programs
                        .iter()
                        .map(|program| format!("{} (pid {})", program.name, program.pid))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ),
            WorktreeRemovalNotice::PaneAppeared => refused("worktree_remove_pane_appeared", None),
            WorktreeRemovalNotice::ChangedWhileClosing => refused("worktree_remove_changed", None),
            WorktreeRemovalNotice::CloseFailed => refused("worktree_remove_close_failed", None),
            WorktreeRemovalNotice::HostFailed { reason, .. } => {
                refused("worktree_remove_host_failed", reason.clone())
            }
            WorktreeRemovalNotice::ResultUnknown => refused("worktree_remove_result_unknown", None),
            WorktreeRemovalNotice::WorkerUnavailable => {
                refused("worktree_remove_worker_unavailable", None)
            }
            WorktreeRemovalNotice::Removed { .. } => {
                refused("worktree_remove_result_unknown", None)
            }
        }),
        ("finished" | "failed", _) => Some(refused("worktree_remove_result_unknown", None)),
        _ => None,
    }
}

/// The next action a refusal of `hide workspace remove` names.
pub fn next_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "worktree_remove_unknown" => {
            "Hide lists no worktree at that path on your machine; check it with git worktree list and hide ps"
        }
        "worktree_remove_blocked" => {
            "Hide never removes the main checkout or a locked worktree; remove another path, or unlock it with git worktree unlock first"
        }
        "worktree_remove_needs_force" => {
            "The worktree holds work that removing it would lose; commit or move it, or add --force to discard it"
        }
        "worktree_remove_branch_unmerged" => {
            "The branch holds commits its base does not have; leave out --delete-branch to keep it, or add --force to delete it anyway"
        }
        "worktree_remove_branch_kept" => {
            "This branch is the repository's base or has no name, so it stays; run again without --delete-branch"
        }
        "worktree_remove_agent_working" => {
            "An agent is still working or waiting in that worktree; wait for it to finish or end it with hide agent end, then retry"
        }
        "worktree_remove_own_checkout" => {
            "A command cannot remove the checkout it runs in; ask your parent or run it from another checkout"
        }
        "worktree_remove_busy" => {
            "Hide is removing another worktree; run the command again once it ends"
        }
        "worktree_remove_unavailable" => {
            "The device that holds the worktree is not connected; retry once it reconnects"
        }
        "worktree_remove_ignored_changed" | "worktree_remove_discard_unconfirmed" => {
            "The worktree changed while Hide read it; run the command again"
        }
        "worktree_remove_stopped" => {
            "The repository's host stopped the removal and the worktree remains; fix what it names and retry"
        }
        "worktree_remove_still_running" => {
            "Its panes were closed but these programs still run in the folder; end them and run the command again"
        }
        "worktree_remove_pane_appeared" => {
            "A pane opened in the worktree while Hide closed its panes; close it and run the command again"
        }
        "worktree_remove_changed" => {
            "The worktree changed while its panes closed; run the command again"
        }
        "worktree_remove_close_failed" => {
            "The engine did not confirm its panes closed and the worktree remains; run the command again"
        }
        "worktree_remove_host_failed" => {
            "The repository's host could not be asked and the worktree remains; retry, and the reason is in Hide's diagnostic log"
        }
        "worktree_remove_result_unknown" => {
            "Hide sent the removal but its result never came; check git worktree list before running the command again"
        }
        "worktree_remove_worker_unavailable" => {
            "Hide could not start the removal and nothing was closed; retry"
        }
        "worktree_remove_pending" => {
            "The removal is still running; run the same command again to wait for its result"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn removal(phase: &str, notice: Option<WorktreeRemovalNotice>) -> WorktreeRemovalSnapshot {
        WorktreeRemovalSnapshot {
            id: 1,
            device_id: None,
            repository_root: "/repo".into(),
            checkout_path: "/repo-wt".into(),
            expected_head_sha: None,
            expected_branch: Some("feat".into()),
            protected_base_branch: None,
            branch: Some("feat".into()),
            delete_branch: true,
            force_delete_branch: false,
            discard_changes: false,
            expected_ignored_repositories: Vec::new(),
            phase: phase.into(),
            notice,
        }
    }

    #[test]
    fn a_running_removal_has_no_answer_and_an_ended_one_names_its_end() {
        for phase in ["checking", "closing", "removing"] {
            assert_eq!(answer(&removal(phase, None)), None, "{phase}");
        }
        let finished = answer(&removal(
            "finished",
            Some(WorktreeRemovalNotice::Removed {
                branch: hide_node_link::worktrees::RemovalBranch::Deleted,
            }),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(finished["removed"], "/repo-wt");
        assert_eq!(finished["branch_result"], "deleted");

        let running = answer(&removal(
            "failed",
            Some(WorktreeRemovalNotice::StillRunning {
                programs: vec![crate::model::RunningProgram {
                    name: "node".into(),
                    pid: 42,
                }],
            }),
        ))
        .unwrap()
        .unwrap_err();
        let (code, detail) = crate::coordination::split_refusal(&running);
        assert_eq!(code, "worktree_remove_still_running");
        assert_eq!(detail, Some("node (pid 42)"));
        assert!(next_action(code).is_some());
    }

    #[test]
    fn every_refusal_the_command_answers_names_a_next_action() {
        for code in DETAILED {
            assert!(next_action(code).is_some(), "{code}");
            let refusal = crate::coordination::refusal(code, "why");
            let (split, detail) = crate::coordination::split_refusal(&refusal);
            assert_eq!((split, detail), (code, Some("why")));
        }
    }
}
