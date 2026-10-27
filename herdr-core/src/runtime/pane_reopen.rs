//! Reopen on a pane Hide cannot hear, inside the runtime (PRD settings-cleanup
//! B29, D-11): the event, the one-in-flight rule per pane, and the worker's
//! answer. What touches Herdr lives in `crate::pane_reopen`; nothing here
//! blocks.

use super::*;
use crate::model::{PaneReopenFailure, PaneReopenSnapshot};
use crate::pane_reopen::{ReopenFailure, ReopenRequest};

impl Runtime {
    /// The pane menu's Reopen. A second press while one runs is the same
    /// intent and starts nothing; a press on a pane that is connected, asleep
    /// or not Reopen's to fix is a no-op the log records.
    pub(super) fn request_pane_reopen(&mut self, pane_id: &str) -> bool {
        if matches!(
            self.pane_reopens.get(pane_id),
            Some(PaneReopenSnapshot::Pending)
        ) {
            return false;
        }
        let Some(connection) = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.children.as_ref())
            .and_then(|children| children.connection.clone())
        else {
            self.pane_reopen_ignored(pane_id, "no_such_pane");
            return false;
        };
        if connection.connected || !connection.can_reopen {
            self.pane_reopen_ignored(
                pane_id,
                if connection.connected {
                    "already_connected"
                } else {
                    "not_reopenable"
                },
            );
            return false;
        }
        if self
            .snapshot
            .ui_state
            .agent_sleep
            .records
            .contains_key(pane_id)
        {
            self.pane_reopen_ignored(pane_id, "asleep");
            return false;
        }
        let Some(agent) = self
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|agent| agent.pane_id == pane_id)
        else {
            return self.settle_pane_reopen(
                pane_id,
                PaneReopenFailure::SessionGone,
                "no agent row",
            );
        };
        let Some(session_id) = agent.session_id.clone() else {
            return self.settle_pane_reopen(
                pane_id,
                PaneReopenFailure::SessionGone,
                "Herdr has not reported this agent's conversation",
            );
        };
        if crate::agent_state::rest_refusal(agent).is_some() {
            return self.settle_pane_reopen(
                pane_id,
                PaneReopenFailure::AgentBusy,
                "agent not idle",
            );
        }
        if crate::codex_launch::start_arguments(
            &agent.agent_kind,
            self.codex_daemon_for_pane(pane_id),
            Vec::new(),
        )
        .is_err()
        {
            return self.settle_pane_reopen(
                pane_id,
                PaneReopenFailure::CodexUnread,
                "the machine's Codex has not been read",
            );
        }
        let request = ReopenRequest {
            pane_id: pane_id.to_owned(),
            kind: agent.agent_kind.clone(),
            session_id,
            name: if agent.id != agent.pane_id && !agent.id.trim().is_empty() {
                agent.id.clone()
            } else {
                crate::fork::wake_name(pane_id)
            },
            cwd: self.reopen_pane_cwd(pane_id),
            codex_daemon: self.codex_daemon_for_pane(pane_id),
            launch: self.launch_choice_of(agent),
        };
        let Some(context) = self.live.clone() else {
            return self.settle_pane_reopen(
                pane_id,
                PaneReopenFailure::StartRefused,
                "no live Herdr connection",
            );
        };
        crate::diagnostic!(serde_json::json!({
            "component": "pane_reopen",
            "kind": "pane_reopen.started",
            "pane_id": pane_id,
            "provider": request.kind,
        }));
        self.pane_reopens
            .insert(pane_id.to_owned(), PaneReopenSnapshot::Pending);
        if let Err(detail) = crate::pane_reopen::spawn_reopen(context, request) {
            return self.settle_pane_reopen(pane_id, PaneReopenFailure::StartRefused, &detail);
        }
        self.sync_pane_lineage();
        true
    }

    /// The worker's answer. A reopened session is confirmed by its own hook
    /// reaching Hide, which is what turns the pane's chip off; a refusal
    /// known before the agent was touched leaves the pane as it was with the
    /// reason published, and one after the end leaves a pane with no agent
    /// and the shell's notice (`pane_reopen.not_restarted`).
    pub(crate) fn ingest_pane_reopen(
        &mut self,
        pane_id: &str,
        result: Result<(), ReopenFailure>,
    ) -> bool {
        if self.pane_reopens.remove(pane_id).is_none() {
            // The pane closed, or its own hook connected, before the answer.
            crate::diagnostic!(serde_json::json!({
                "component": "pane_reopen",
                "kind": "pane_reopen.answered",
                "pane_id": pane_id,
                "outcome": "settled_elsewhere",
            }));
            return false;
        }
        match result {
            Ok(()) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_reopen",
                    "kind": "pane_reopen.answered",
                    "pane_id": pane_id,
                    "outcome": "started",
                }));
                self.sync_pane_lineage();
                true
            }
            Err(failure) => {
                self.settle_pane_reopen(pane_id, failure.code, &failure.detail);
                // The chip is read from the agent's row, and an agent that
                // was ended and not started again has none: with nothing on
                // the pane to carry the failure, the operator is told once
                // that their agent is gone and its conversation was kept.
                if failure.ended
                    && !self
                        .snapshot
                        .navigator
                        .agents
                        .iter()
                        .any(|agent| agent.pane_id == pane_id)
                {
                    self.set_error(
                        "pane_reopen.not_restarted",
                        "Reopen ended the agent but could not start it again; its conversation is kept",
                        false,
                    );
                }
                true
            }
        }
    }

    /// Publishes a refusal as the pane's one line and logs Herdr's words.
    fn settle_pane_reopen(
        &mut self,
        pane_id: &str,
        reason: PaneReopenFailure,
        detail: &str,
    ) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "pane_reopen",
            "kind": "pane_reopen.answered",
            "pane_id": pane_id,
            "outcome": "refused",
            "reason": reason,
            "detail": detail,
        }));
        self.pane_reopens
            .insert(pane_id.to_owned(), PaneReopenSnapshot::Failed { reason });
        self.sync_pane_lineage();
        true
    }

    fn pane_reopen_ignored(&self, pane_id: &str, why: &str) {
        crate::diagnostic!(serde_json::json!({
            "component": "pane_reopen",
            "kind": "pane_reopen.ignored",
            "pane_id": pane_id,
            "why": why,
        }));
    }

    fn reopen_pane_cwd(&self, pane_id: &str) -> Option<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.cwd.clone())
            .filter(|cwd| !cwd.trim().is_empty())
    }
}
