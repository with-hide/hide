//! What `hide workspace remove` takes from the runtime under its lock: the
//! caller's machine and checkout, the worktree's row, the command's own
//! consent in place of the dialog's checkboxes, and then the removal the
//! dialog starts (`crate::worktree_command`).

use super::Runtime;
use super::events::RemoveWorktreePayload;
use crate::model::{AgentStatusCode, WorktreeRemovalSnapshot, WorktreeSnapshot};
use crate::workspace_control::{Caller, Context};
use crate::worktree_command::WorktreeRemove;

/// How a removal request was taken: started now, or the same checkout's
/// removal already running, which the caller waits for instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RemovalStart {
    Started(u64),
    Running(u64),
}

/// Why a removal could not start: the error kind the dialog shows and its
/// sentence, which a command answers as `kind` with `_` for `.`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemovalRefused {
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

impl RemovalRefused {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// The command's refusal code, with the detail its next action carries
    /// when it is one of `worktree_command::DETAILED`.
    fn code(&self) -> String {
        let code = self.kind.replace('.', "_");
        if crate::worktree_command::DETAILED.contains(&code.as_str()) {
            crate::coordination::refusal(&code, &self.message)
        } else {
            code
        }
    }
}

/// An agent in one of these states is still at work or waiting on someone,
/// so closing its pane would cut it off; a finished, idle or stopped agent's
/// pane closes with the worktree, as the dialog closes it.
fn still_at_work(status: AgentStatusCode) -> bool {
    matches!(
        status,
        AgentStatusCode::Working
            | AgentStatusCode::Starting
            | AgentStatusCode::Waiting
            | AgentStatusCode::Question
            | AgentStatusCode::Approval
            | AgentStatusCode::Waking
    )
}

/// The command's consent, judged on the row as it is under the lock: what
/// the dialog asks the operator, `--force` and `--delete-branch` answer.
fn consent(
    command: &WorktreeRemove,
    worktree: &WorktreeSnapshot,
    payload: &mut RemoveWorktreePayload,
) -> Result<(), RemovalRefused> {
    let gate = &worktree.deletion_gate;
    if let Some(blocked) = gate.blocked.as_ref() {
        return Err(RemovalRefused::new(
            "worktree.remove_blocked",
            serde_json::to_string(blocked).unwrap_or_default(),
        ));
    }
    let working = worktree
        .agent_line
        .agents
        .iter()
        .filter(|agent| still_at_work(agent.status_code))
        .map(|agent| format!("{} ({})", agent.label, agent.status_code.english()))
        .collect::<Vec<_>>();
    if !working.is_empty() {
        return Err(RemovalRefused::new(
            "worktree.remove_agent_working",
            working.join(", "),
        ));
    }
    if let Some(discard) = gate.discard.as_ref() {
        if !command.force {
            return Err(RemovalRefused::new(
                "worktree.remove_needs_force",
                discard_words(discard),
            ));
        }
        payload.discard_changes = true;
        payload.expected_ignored_repositories = worktree.ignored_repositories.clone();
    }
    if command.delete_branch {
        if !gate.can_delete_branch {
            return Err(RemovalRefused::new(
                "worktree.remove_branch_kept",
                "the branch stays",
            ));
        }
        if let Some(loss) = gate.branch_loss.as_ref()
            && !command.force
        {
            return Err(RemovalRefused::new(
                "worktree.remove_branch_unmerged",
                serde_json::to_string(loss).unwrap_or_default(),
            ));
        }
        payload.delete_branch = true;
    }
    Ok(())
}

/// What removing the folder would lose, in the words a caller fixes it by.
fn discard_words(discard: &crate::model::WorktreeDiscard) -> String {
    let mut words = Vec::new();
    match discard.changed_files {
        Some(0) => words.push("uncommitted changes".to_owned()),
        Some(files) => words.push(format!("{files} uncommitted or untracked files")),
        None => {}
    }
    if discard.status_unknown {
        words.push("a status Git could not read".to_owned());
    }
    if discard.nested {
        words.push("a worktree inside it".to_owned());
    }
    if !discard.ignored_repositories.is_empty() {
        words.push(format!(
            "ignored repositories {}",
            discard.ignored_repositories.join(", ")
        ));
    }
    words.join(", ")
}

impl Runtime {
    /// Admits `hide workspace remove` for `caller` on `device` whose checkout
    /// is `expected`, and starts the removal of the worktree it names on the
    /// caller's machine: the id of the removal record to wait for.
    pub(crate) fn worktree_remove_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: &WorktreeRemove,
    ) -> Result<u64, String> {
        if !command.valid() {
            return Err("worktree_remove_unknown".into());
        }
        if Caller::parse(caller) == Caller::Desk {
            return Err("agent_pane_required".into());
        }
        let context = self
            .resolve_caller(device, Caller::parse(caller))
            .map_err(|refusal| refusal.reason.to_owned())?;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        if context.checkout_path == command.path {
            return Err("worktree_remove_own_checkout".into());
        }
        // A retry of a removal that already ended finds no row to remove:
        // the record of its end answers it, so the same command converges.
        if let Some(ended) = self.snapshot.worktree_removal.as_ref().filter(|removal| {
            removal.checkout_path == command.path
                && removal.phase == "finished"
                && removal.device_id.as_deref().unwrap_or(self.node.as_str()) == device
        }) {
            return Ok(ended.id);
        }
        let payload = RemoveWorktreePayload {
            device_id: Some(device.to_owned()),
            checkout_path: command.path.clone(),
            delete_branch: false,
            discard_changes: false,
            expected_ignored_repositories: Vec::new(),
            close_descendant_pane_ids: Vec::new(),
        };
        let consent = |worktree: &WorktreeSnapshot, payload: &mut RemoveWorktreePayload| {
            consent(command, worktree, payload)
        };
        let started = self.start_git_worktree_removal(payload, Some(&consent));
        crate::diagnostic!(serde_json::json!({
            "component": "worktree_removal", "kind": "command_requested",
            "target": device, "caller": caller,
            "id": match started { Ok(RemovalStart::Started(id) | RemovalStart::Running(id)) => Some(id), Err(_) => None },
            "refused": started.as_ref().err().map(|refused| refused.kind),
        }));
        match started {
            Ok(RemovalStart::Started(id) | RemovalStart::Running(id)) => Ok(id),
            Err(refused) => Err(refused.code()),
        }
    }

    /// The removal record `id`, while it is the one the runtime holds.
    pub(crate) fn worktree_removal_record(&self, id: u64) -> Option<&WorktreeRemovalSnapshot> {
        self.snapshot
            .worktree_removal
            .as_ref()
            .filter(|removal| removal.id == id)
    }
}
