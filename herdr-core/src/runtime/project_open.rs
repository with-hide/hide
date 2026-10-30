//! `hide <path>` in the core (`crate::project_open`): the folder's project
//! comes forward, registered first through the same `create_workspace` the
//! Add a project dialog sends when no registered checkout holds it.
//!
//! Each request is a receipt kept by its id, so the command reads its own
//! answer rather than the shared `last_error`, which the next event of any
//! kind clears. A registration settles every receipt waiting on its device
//! and path, the dialog's own included.

use std::collections::VecDeque;

use super::events::{CreateWorkspacePayload, Event, RemoteControlPayload, RemoteControlRequest};
use super::*;
use crate::project_open::{ProjectOpenReceipt, Unheld};

/// Receipts kept at once; past it a settled one makes room, and with every
/// one still waiting a new request is refused `project_open_busy`.
const MAX_OPENS: usize = 32;

#[derive(Default)]
pub(super) struct ProjectOpens {
    entries: VecDeque<ProjectOpen>,
}

struct ProjectOpen {
    request_id: String,
    device: String,
    path: String,
    receipt: ProjectOpenReceipt,
}

/// What a settled registration brings in front.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Forward {
    /// Nothing: a refusal, or a local creation, which put its first tab in
    /// front itself.
    No,
    /// The registered checkout that holds the folder.
    Checkout,
    /// The device alone: a device's new project is listed once its helper
    /// confirms the folder, and its first tab opens there then
    /// (`open_registration_terminals`), so the device shows it when it does.
    Device,
}

/// The checkout a request brings forward.
struct Target {
    workspace_id: String,
    checkout_id: String,
    label: String,
    path: String,
}

/// Whether the checkout at `root` holds `path`, both in the wire spelling.
fn holds(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The folder's name, which a registration is labelled with as the dialog
/// labels a picked folder.
fn folder_label(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
}

impl Runtime {
    /// Starts `hide <path>` for a caller on `device`, `path` being that
    /// device's absolute folder (on this machine, the spelling hided's
    /// `$HOME` line or a registered checkout's line checked), and `unheld`
    /// what happens when no registered checkout holds it. A request id seen
    /// before changes nothing, so a retry never registers twice; true when
    /// the snapshot changed.
    pub(crate) fn project_open(
        &mut self,
        device: &str,
        path: &str,
        request_id: &str,
        unheld: Unheld,
    ) -> Result<bool, String> {
        if self
            .project_opens
            .entries
            .iter()
            .any(|open| open.request_id == request_id)
        {
            return Ok(false);
        }
        if self.project_opens.entries.len() >= MAX_OPENS {
            let settled = self
                .project_opens
                .entries
                .iter()
                .position(|open| open.receipt != ProjectOpenReceipt::Pending)
                .ok_or("project_open_busy")?;
            self.project_opens.entries.remove(settled);
        }
        let receipt = match self.open_target(device, path) {
            Some(target) => {
                self.bring_target_forward(device, &target, request_id);
                ProjectOpenReceipt::Opened {
                    label: target.label,
                    path: target.path,
                    checkout_id: Some(target.checkout_id),
                    added: false,
                }
            }
            None => match unheld {
                Unheld::Register => ProjectOpenReceipt::Pending,
                Unheld::Refuse => ProjectOpenReceipt::Refused {
                    reason: "needs_confirmation".to_owned(),
                },
            },
        };
        let pending = receipt == ProjectOpenReceipt::Pending;
        let registered = matches!(receipt, ProjectOpenReceipt::Opened { .. });
        self.project_opens.entries.push_back(ProjectOpen {
            request_id: request_id.to_owned(),
            device: device.to_owned(),
            path: path.to_owned(),
            receipt,
        });
        crate::diagnostic!(serde_json::json!({
            "component": "project_open", "kind": "requested",
            "request_id": request_id, "device": device, "registered": registered,
        }));
        if pending {
            let local = device == self.node.as_str();
            self.apply(Event::CreateWorkspace(CreateWorkspacePayload {
                device_id: (!local).then(|| device.to_owned()),
                path: path.to_owned(),
                label: folder_label(path),
                initialize_git: false,
                new_folder: false,
                confirmed_outside_home: None,
            }));
        }
        Ok(true)
    }

    /// The receipt of the request `request_id`, if the core still keeps it.
    pub(crate) fn project_open_receipt(&self, request_id: &str) -> Option<ProjectOpenReceipt> {
        self.project_opens
            .entries
            .iter()
            .find(|open| open.request_id == request_id)
            .map(|open| open.receipt.clone())
    }

    /// Settles every request waiting on `device` and `path` with the
    /// registration's outcome, an error kind or the project registered, and
    /// brings in front what `forward` says.
    pub(super) fn settle_project_open(
        &mut self,
        device: &str,
        path: &str,
        outcome: Result<(), &str>,
        forward: Forward,
    ) {
        let waiting = self
            .project_opens
            .entries
            .iter()
            .filter(|open| {
                open.receipt == ProjectOpenReceipt::Pending
                    && open.device == device
                    && open.path == path
            })
            .map(|open| open.request_id.clone())
            .collect::<Vec<_>>();
        let Some(first) = waiting.first() else {
            return;
        };
        let first = first.clone();
        let receipt = match outcome {
            Err(kind) => ProjectOpenReceipt::Refused {
                reason: kind.replace('.', "_"),
            },
            Ok(()) => {
                let target = self.open_target(device, path);
                match (forward, &target) {
                    (Forward::Checkout, Some(target)) => {
                        self.bring_target_forward(device, target, &first);
                    }
                    (Forward::Checkout | Forward::Device, _) if !self.device_in_front(device) => {
                        self.bring_device_forward(device.to_owned());
                    }
                    _ => {}
                }
                match target {
                    Some(target) => ProjectOpenReceipt::Opened {
                        label: target.label,
                        path: target.path,
                        checkout_id: Some(target.checkout_id),
                        added: true,
                    },
                    None => ProjectOpenReceipt::Opened {
                        label: folder_label(path),
                        path: path.to_owned(),
                        checkout_id: None,
                        added: true,
                    },
                }
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "project_open", "kind": "settled",
            "request_ids": waiting, "device": device,
            "reason": match &receipt {
                ProjectOpenReceipt::Refused { reason } => Some(reason.as_str()),
                _ => None,
            },
        }));
        for open in &mut self.project_opens.entries {
            if waiting.contains(&open.request_id) {
                open.receipt = receipt.clone();
            }
        }
    }

    /// The deepest registered checkout on `device` that holds `path`.
    fn open_target(&self, device: &str, path: &str) -> Option<Target> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == device && workspace.registered)
            .flat_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .map(move |checkout| (workspace, checkout))
            })
            .filter(|(_, checkout)| holds(&checkout.path, path))
            .max_by_key(|(_, checkout)| checkout.path.len())
            .map(|(workspace, checkout)| Target {
                workspace_id: workspace.id.clone(),
                checkout_id: checkout.id.clone(),
                label: workspace.label.clone(),
                path: checkout.path.clone(),
            })
    }

    /// Brings `target` in front, as a sidebar click on its row does: this
    /// machine's checkout through `focus_checkout`, a device's through that
    /// device's Herdr with the device moved in front.
    fn bring_target_forward(&mut self, device: &str, target: &Target, request_id: &str) {
        if device == self.node.as_str() {
            self.focus_checkout(&target.workspace_id, &target.checkout_id);
            if self.snapshot.navigator.focused_checkout_id.as_deref()
                == Some(target.checkout_id.as_str())
                && !self.device_in_front(device)
            {
                self.bring_device_forward(device.to_owned());
            }
            return;
        }
        self.request_remote_control(RemoteControlPayload {
            target_id: device.to_owned(),
            request_id: format!("project-open-{request_id}"),
            report_pane_focus_outcome: false,
            focus_device: true,
            request: RemoteControlRequest::FocusWorkspace {
                workspace_id: target.workspace_id.clone(),
                checkout_id: Some(target.checkout_id.clone()),
            },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkout_holds_its_own_folder_and_those_below_it_only() {
        assert!(holds("/home/me/app", "/home/me/app"));
        assert!(holds("/home/me/app/", "/home/me/app/src"));
        assert!(!holds("/home/me/app", "/home/me/application"));
        assert!(!holds("/home/me/app", "/home/me"));
    }
}
