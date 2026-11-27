//! What Desk's agent opens for the operator with `hide file open`: the file
//! goes in front of Desk's own desk (PRD desk-own-desk D-13, B35), as a
//! press in Desk's room opens it. Whether the desk is drawn is the window's:
//! a window showing Desk shows it for the open (`desk_view.opened`), and one
//! on another screen moves nothing. `hide diff open` is refused Desk: Desk's
//! desk has no diff tab, and the core reads a diff only for the checkout in
//! front (`changes_request`).
//!
//! The path is checked as an agent's `file_open` is: it must sit inside a
//! checkout Hide shows on a connected machine, by whole path segments, and
//! that checkout's own host reads it there (`ActionSource::read`) before it
//! opens, so a link out of the checkout or a missing file is refused before
//! anything moves.

use std::collections::VecDeque;

use crate::runtime::Runtime;
use crate::runtime::workspace_control::{DESK_REFUSAL, path_within};
use crate::workspace_control::{
    Action, ActionMaterial, ActionPreparation, ActionResult, ActionSource, Context, Refusal,
};

/// The requests whose answers are kept; an older one asked again opens
/// the file again, which brings its tab in front as before.
const OPENS_ANSWERED: usize = 16;

/// The answers to the recent opens, each by the request id that made it.
#[derive(Default)]
pub(super) struct DeskOpens {
    answered: VecDeque<(String, u64)>,
}

/// `hide diff open` from Desk: Desk's desk has no diff tab.
const DIFF_UNSUPPORTED: Refusal = Refusal {
    reason: "desk_diff_unsupported",
    next_action: "Desk's desk shows files, not diffs: open the changed file with hide file open <path>",
};

const OUTSIDE_PROJECTS: Refusal = Refusal {
    reason: "desk_file_outside_project",
    next_action: "Desk shows files of the projects Hide shows; give the path of a file inside one",
};

const AMBIGUOUS_DEVICE: Refusal = Refusal {
    reason: "desk_file_ambiguous_device",
    next_action: "Projects on two machines hold this path; ask the session that made the file to open it from its own pane",
};

const CHECKOUT_UNAVAILABLE: Refusal = Refusal {
    reason: "host_unavailable",
    next_action: "Reconnect the machine that holds the project and retry",
};

/// The file `action` opens, or the refusal for any other Workspace action.
fn file_of(action: &Action) -> Result<&str, Refusal> {
    match action {
        Action::OpenFile { path } => Ok(path),
        Action::OpenDiff { .. } => Err(DIFF_UNSUPPORTED),
        _ => Err(DESK_REFUSAL),
    }
}

/// The checkout that holds `path`: the deepest registered one on its
/// machine, and the one machine whose projects hold it.
struct Target {
    device_id: String,
    workspace_id: String,
    checkout_id: String,
    root: String,
}

impl Runtime {
    /// The checkout of a connected machine whose folder holds `path`, the
    /// deepest on each machine. A path two machines' projects hold is
    /// refused rather than read as either: the same folder on two machines
    /// is two files.
    fn desk_open_target(&self, path: &str) -> Result<Target, Refusal> {
        if !hide_platform::path::is_wire_absolute(path) {
            return Err(Refusal {
                reason: "invalid_path",
                next_action: "Give the file's absolute path and retry",
            });
        }
        let mut found: Vec<Target> = Vec::new();
        for workspace in self.catalog_workspaces() {
            if !self.workspace_connected(&workspace.device_id) {
                continue;
            }
            for checkout in &workspace.checkouts {
                let root = checkout.path.trim_end_matches('/');
                if root.is_empty() || !path_within(path, root) {
                    continue;
                }
                match found
                    .iter_mut()
                    .find(|target| target.device_id == workspace.device_id)
                {
                    Some(best) if best.root.trim_end_matches('/').len() >= root.len() => {}
                    Some(best) => {
                        best.workspace_id.clone_from(&workspace.id);
                        best.checkout_id.clone_from(&checkout.id);
                        best.root.clone_from(&checkout.path);
                    }
                    None => found.push(Target {
                        device_id: workspace.device_id.clone(),
                        workspace_id: workspace.id.clone(),
                        checkout_id: checkout.id.clone(),
                        root: checkout.path.clone(),
                    }),
                }
            }
        }
        match found.len() {
            0 => Err(OUTSIDE_PROJECTS),
            1 => Ok(found.remove(0)),
            _ => Err(AMBIGUOUS_DEVICE),
        }
    }

    fn desk_open_result(&self, request_id: &str, id: u64) -> ActionResult {
        ActionResult {
            context: crate::desk::desk_context(self.node.as_str()),
            request_id: request_id.to_owned(),
            changed: true,
            view_id: format!("desk-open-{id}"),
            area_id: None,
            load: None,
        }
    }

    /// The read that checks Desk's open on its checkout's host, or the
    /// answer an open with the same request id already has.
    pub(in crate::runtime) fn desk_prepare_open(
        &mut self,
        device_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
    ) -> Result<ActionPreparation, Refusal> {
        self.desk_open_caller(device_id, expected)?;
        let path = file_of(action)?;
        if let Some(open) = self.desk_open_by_request(request_id) {
            return Ok(ActionPreparation::Cached(Ok(
                self.desk_open_result(request_id, open)
            )));
        }
        let target = self.desk_open_target(path)?;
        let local = target.device_id == self.node.as_str();
        let (root, channel) = self
            .document_source(&target.workspace_id, &target.checkout_id)
            .map_err(|_| CHECKOUT_UNAVAILABLE)?;
        Ok(ActionPreparation::Read(ActionSource::file(
            root,
            (target.workspace_id, target.checkout_id),
            channel,
            path.to_owned(),
            false,
            local,
        )))
    }

    /// Opens Desk's file in front of Desk's desk once its host has read it.
    /// The same request id asked again answers the open already made.
    pub(in crate::runtime) fn desk_commit_open(
        &mut self,
        device_id: &str,
        expected: &Context,
        request_id: &str,
        action: &Action,
        material: Result<Option<ActionMaterial>, Refusal>,
    ) -> Result<ActionResult, Refusal> {
        self.desk_open_caller(device_id, expected)?;
        let path = file_of(action)?;
        if let Some(open) = self.desk_open_by_request(request_id) {
            return Ok(self.desk_open_result(request_id, open));
        }
        let material = material?.ok_or(CHECKOUT_UNAVAILABLE)?;
        // The catalog may have moved while the host read the file.
        let target = self.desk_open_target(path)?;
        if !self.desk_views_on() {
            return Err(Refusal {
                reason: "desk_off",
                next_action: "Desk is switched off; the operator turns it on in Settings",
            });
        }
        // A refusal is the agent's to read, and neither counts as an open nor
        // stands for this request, so asking again once there is room opens.
        let Some(view) =
            self.desk_open_read_file(&target.workspace_id, &target.checkout_id, material)
        else {
            return Err(Refusal {
                reason: "desk_open_refused",
                next_action: "Desk's desk could not take the file (no room, or it could not be read); ask the operator to close a tab, then open it again",
            });
        };
        // The View it landed in is Desk's agent's open, for the operator to
        // look at (한눈에), as a pane agent's open marks its View.
        let key = self.desk_key();
        self.stamp_view_opener(&key, &view, Some(self.desk_opener()));
        self.mark_view_unseen(&key, &view);
        self.note_desk_agent_open();
        let id = self.desk.views_opened;
        let answered = &mut self.desk.opens.answered;
        if answered.len() >= OPENS_ANSWERED {
            answered.pop_front();
        }
        answered.push_back((request_id.to_owned(), id));
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk.open",
            "open": id,
            "device": target.device_id,
        }));
        Ok(self.desk_open_result(request_id, id))
    }

    fn desk_open_caller(&self, device_id: &str, expected: &Context) -> Result<(), Refusal> {
        let (context, _) = self.desk_authority(device_id).map_err(|_| Refusal {
            reason: "desk_not_running",
            next_action: "Desk's agent is not running; the operator starts it in Desk",
        })?;
        if context != *expected {
            return Err(DESK_REFUSAL);
        }
        Ok(())
    }

    fn desk_open_by_request(&self, request_id: &str) -> Option<u64> {
        self.desk
            .opens
            .answered
            .iter()
            .find(|(asked, _)| asked == request_id)
            .map(|(_, id)| *id)
    }
}
