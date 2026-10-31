//! Settings › Agents › 권한: the core's copy of each machine's choice, the
//! one event that changes it, and the reads every start and restart takes it
//! through (`crate::agent_permission`).

use super::*;
use crate::agent_permission::{AgentPermission, AgentPermissions};
use serde::Deserialize;

/// `agent_permission_set`: one agent's 권한 on one machine's row. `text` is
/// what the operator typed for `직접 입력`, and is refused for another mode.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentPermissionSetPayload {
    pub(super) device_id: String,
    pub(super) agent: String,
    pub(super) mode: AgentPermissionMode,
    #[serde(default)]
    pub(super) text: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum AgentPermissionMode {
    Full,
    Ask,
    Custom,
}

impl Runtime {
    /// Each agent's choice on `device`, this machine's included.
    pub(crate) fn agent_permissions_of(&self, device: &str) -> AgentPermissions {
        let ui = &self.snapshot.ui_state;
        if device == self.node.as_str() {
            ui.agent_permissions.clone()
        } else {
            ui.device_agent_permissions
                .get(device)
                .cloned()
                .unwrap_or_default()
        }
    }

    /// A `kind` agent's choice on `device`.
    pub(crate) fn agent_permission(&self, device: &str, kind: &str) -> AgentPermission {
        crate::agent_permission::of(&self.agent_permissions_of(device), kind)
    }

    /// [`Self::agent_permission`] for the machine a pane id belongs to.
    pub(crate) fn agent_permission_for_pane(&self, pane_id: &str, kind: &str) -> AgentPermission {
        self.agent_permission(self.device_of_pane(pane_id), kind)
    }

    /// Records one agent's choice on one machine, saving only one that is
    /// not the default; the same choice again changes nothing (engineering
    /// rule 11).
    pub(super) fn set_agent_permission(&mut self, payload: AgentPermissionSetPayload) -> bool {
        let Some(row) =
            hide_agent_adapter::adapter(&payload.agent).filter(|row| row.start.is_some())
        else {
            self.set_error(
                "agent_permission.unknown_agent",
                format!("{} is not an agent Hide starts", payload.agent),
                false,
            );
            return true;
        };
        let device = payload.device_id.as_str();
        if device != self.node.as_str() && !self.device_registration_exists(device) {
            self.set_error(
                "agent_permission.unknown_machine",
                format!("Device {device} is not registered"),
                false,
            );
            return true;
        }
        let permission = match (payload.mode, payload.text.as_deref()) {
            (AgentPermissionMode::Full, None) => AgentPermission::Full,
            (AgentPermissionMode::Ask, None) => AgentPermission::Ask,
            (AgentPermissionMode::Custom, Some(text)) => match AgentPermission::custom(text) {
                Ok(permission) => permission,
                Err(reason) => {
                    self.set_error("agent_permission.invalid_options", reason, false);
                    return true;
                }
            },
            (AgentPermissionMode::Custom, None) | (_, Some(_)) => {
                self.set_error(
                    "agent_permission.invalid",
                    "Only 직접 입력 takes typed options",
                    false,
                );
                return true;
            }
        };
        let node = self.node.to_string();
        let ui = &mut self.snapshot.ui_state;
        let permissions = if device == node {
            &mut ui.agent_permissions
        } else {
            ui.device_agent_permissions
                .entry(device.to_owned())
                .or_default()
        };
        let changed = if permission == AgentPermission::Full {
            permissions.remove(row.id).is_some()
        } else {
            permissions.insert(row.id.to_owned(), permission.clone()) != Some(permission)
        };
        ui.device_agent_permissions
            .retain(|_, permissions| !permissions.is_empty());
        if changed {
            self.persist_ui_state();
        }
        changed
    }
}
