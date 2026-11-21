//! `hide <path>` in the core (`crate::project_open`): the folder's project
//! comes forward, registered first through the same `create_workspace` the
//! Add a project dialog sends when no registered checkout holds it.
//!
//! Each request is a receipt kept by its id, so the command reads its own
//! answer rather than the shared `last_error`, which the next event of any
//! kind clears. A registration settles every receipt waiting on its device
//! and path, the dialog's own included.
//!
//! A `create_workspace` that names a request id (Add a project's importable
//! list, which adds one row after another) keeps a receipt here too, beside
//! these and under the same bounds, which brings nothing forward and is
//! published as `status.project_adds`, so each row reads its own outcome
//! after any later event; an add that is not admitted is kept as a receipt
//! refused at once.

use std::collections::VecDeque;

use super::events::{CreateWorkspacePayload, Event, RemoteControlPayload, RemoteControlRequest};
use super::*;
use crate::project_open::{ProjectOpenReceipt, Unheld};

/// Requests waiting at once: with this many waiting a new one is refused
/// (`project_open_busy`, or an add's refused receipt). Settled receipts are
/// bounded apart, by the same number with the oldest dropped first, so a
/// refusal always has room to be kept.
const MAX_OPENS: usize = 32;

#[derive(Default)]
pub(super) struct ProjectOpens {
    entries: VecDeque<ProjectOpen>,
}

/// The longest request id an add may carry.
const REQUEST_ID_LIMIT: usize = 128;

struct ProjectOpen {
    request_id: String,
    device: String,
    path: String,
    receipt: ProjectOpenReceipt,
    /// `hide <path>`, which brings its project forward; an add from the
    /// importable list only adds it, while its dialog stays open.
    forward: bool,
    /// The refusal's error kind and message, for an add's receipt.
    refusal: Option<(String, String)>,
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
        if !self.admits_open() {
            return Err("project_open_busy".to_owned());
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
        self.keep_open(ProjectOpen {
            request_id: request_id.to_owned(),
            device: device.to_owned(),
            path: path.to_owned(),
            receipt,
            forward: true,
            refusal: None,
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
                request_id: None,
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

    /// Whether another request may wait: fewer than [`MAX_OPENS`] do.
    fn admits_open(&self) -> bool {
        let waiting = self
            .project_opens
            .entries
            .iter()
            .filter(|open| open.receipt == ProjectOpenReceipt::Pending)
            .count();
        waiting < MAX_OPENS
    }

    /// Keeps `open`'s receipt, dropping the oldest settled receipts past
    /// [`MAX_OPENS`] of them; a waiting one is never dropped.
    fn keep_open(&mut self, open: ProjectOpen) {
        self.project_opens.entries.push_back(open);
        self.trim_settled_opens();
    }

    fn trim_settled_opens(&mut self) {
        let entries = &mut self.project_opens.entries;
        let mut settled = entries
            .iter()
            .filter(|open| open.receipt != ProjectOpenReceipt::Pending)
            .count();
        while settled > MAX_OPENS {
            let Some(oldest) = entries
                .iter()
                .position(|open| open.receipt != ProjectOpenReceipt::Pending)
            else {
                break;
            };
            entries.remove(oldest);
            settled -= 1;
        }
        self.publish_project_adds();
    }

    /// Keeps a receipt for the add `request_id` of `path` on `device`:
    /// waiting, settled later with its registration, and `true`; or, when
    /// the add is not admitted (a reused id, or [`MAX_OPENS`] requests all
    /// waiting), already refused with why, and `false`. A malformed id can
    /// name no receipt, so its refusal is the `last_error` alone.
    pub(super) fn await_add(&mut self, request_id: &str, device: &str, path: &str) -> bool {
        if request_id.is_empty()
            || request_id.len() > REQUEST_ID_LIMIT
            || request_id.chars().any(char::is_control)
        {
            self.set_error(
                "workspace.create_refused",
                "The add named a request id that is empty, too long or holds a control character",
                false,
            );
            return false;
        }
        let refused = if self
            .project_opens
            .entries
            .iter()
            .any(|open| open.request_id == request_id)
        {
            Some((
                "workspace.create_refused",
                "The add named a request id another request already used".to_owned(),
            ))
        } else if !self.admits_open() {
            Some((
                "workspace.create_busy",
                format!(
                    "{MAX_OPENS} projects are already being added; add this one once they finish"
                ),
            ))
        } else {
            None
        };
        let (receipt, refusal) = match &refused {
            Some((kind, message)) => {
                // Logged as every refusal is; the receipt is what the sender reads.
                let retryable = *kind == "workspace.create_busy";
                self.set_request_error(*kind, message.clone(), retryable, Some(request_id));
                (
                    ProjectOpenReceipt::Refused {
                        reason: kind.replace('.', "_"),
                    },
                    Some(((*kind).to_owned(), message.clone())),
                )
            }
            None => (ProjectOpenReceipt::Pending, None),
        };
        self.keep_open(ProjectOpen {
            request_id: request_id.to_owned(),
            device: device.to_owned(),
            path: path.to_owned(),
            receipt,
            forward: false,
            refusal,
        });
        refused.is_none()
    }

    /// The adds' receipts as the snapshot carries them.
    fn publish_project_adds(&mut self) {
        self.snapshot.status.project_adds = self
            .project_opens
            .entries
            .iter()
            .filter(|open| !open.forward)
            .map(|open| crate::model::ProjectAddReceiptSnapshot {
                request_id: open.request_id.clone(),
                device_id: open.device.clone(),
                path: open.path.clone(),
                outcome: match (&open.receipt, &open.refusal) {
                    (ProjectOpenReceipt::Pending, _) => crate::model::ProjectAddOutcome::Pending,
                    (ProjectOpenReceipt::Opened { .. }, _) => {
                        crate::model::ProjectAddOutcome::Added
                    }
                    (ProjectOpenReceipt::Refused { reason }, refusal) => {
                        let (kind, message) = refusal
                            .clone()
                            .unwrap_or_else(|| (reason.clone(), reason.clone()));
                        crate::model::ProjectAddOutcome::Refused { kind, message }
                    }
                },
            })
            .collect();
    }

    /// Settles every request waiting on `device` and `path`, each its own
    /// receipt, with the registration's outcome, an error kind or the
    /// project registered, and brings in front what `forward` says when a
    /// `hide <path>` is among them. Every outcome of a `create_workspace`
    /// passes here, so an add's receipt is settled here too.
    pub(super) fn settle_project_open(
        &mut self,
        device: &str,
        path: &str,
        outcome: Result<(), &str>,
        forward: Forward,
    ) -> bool {
        // Answers whether what it brought forward was read, for a caller
        // that announces only what changed (`ingest_device_registration`).
        self.change_look(|runtime| {
            runtime.settle_project_open_steps(device, path, outcome, forward)
        })
        .1
    }

    /// The steps of `settle_project_open`, which runs them as one selection (`look.rs`).
    fn settle_project_open_steps(
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
            .map(|open| (open.request_id.clone(), open.forward))
            .collect::<Vec<_>>();
        if waiting.is_empty() {
            return;
        }
        let leading = waiting
            .iter()
            .find(|(_, forward)| *forward)
            .map(|(id, _)| id.clone());
        let (receipt, refusal) = match outcome {
            Err(kind) => {
                // The refusal's own words when the error it just set is
                // this one; a refusal that set none is named by its kind.
                let message = self
                    .snapshot
                    .status
                    .last_error
                    .as_ref()
                    .filter(|error| error.kind == kind)
                    .map_or_else(
                        || format!("{path} could not be added"),
                        |error| error.message.clone(),
                    );
                (
                    ProjectOpenReceipt::Refused {
                        reason: kind.replace('.', "_"),
                    },
                    Some((kind.to_owned(), message)),
                )
            }
            Ok(()) => {
                let target = self.open_target(device, path);
                if let Some(leading) = &leading {
                    match (forward, &target) {
                        (Forward::Checkout, Some(target)) => {
                            self.bring_target_forward(device, target, leading);
                        }
                        (Forward::Checkout | Forward::Device, _)
                            if !self.device_in_front(device) =>
                        {
                            self.bring_device_forward(device.to_owned());
                        }
                        _ => {}
                    }
                }
                let receipt = match target {
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
                };
                (receipt, None)
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "project_open", "kind": "settled",
            "request_ids": waiting.iter().map(|(id, _)| id).collect::<Vec<_>>(), "device": device,
            "reason": match &receipt {
                ProjectOpenReceipt::Refused { reason } => Some(reason.as_str()),
                _ => None,
            },
        }));
        // The receipts the selection chose, by the same test: an id reused
        // for another folder keeps its own receipt.
        for open in &mut self.project_opens.entries {
            if open.receipt == ProjectOpenReceipt::Pending
                && open.device == device
                && open.path == path
            {
                open.receipt = receipt.clone();
                open.refusal = refusal.clone();
            }
        }
        self.trim_settled_opens();
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
    /// device's engine with the device moved in front.
    fn bring_target_forward(&mut self, device: &str, target: &Target, request_id: &str) {
        self.change_look(|runtime| runtime.bring_target_forward_steps(device, target, request_id));
    }

    /// The steps of `bring_target_forward`, which runs them as one selection (`look.rs`).
    fn bring_target_forward_steps(&mut self, device: &str, target: &Target, request_id: &str) {
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
