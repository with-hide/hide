//! What Desk's agent opened for the operator with `hide file open`, waiting
//! for a window to show it in Desk's tab panel. `hide diff open` is refused
//! Desk for now: the tab panel has no diff tab, and the core reads a diff
//! only for the checkout in front (`changes_request`).
//!
//! The tab panel's tabs are the window's (`web/src/deskRoom.ts`), so the
//! core never adds a tab: it keeps a short list of opens the snapshot
//! carries in the `desk` section, and a window showing Desk opens each one
//! as a click would, then sends `desk_open_taken` with its id, which takes
//! it off the list. With no window showing Desk an open waits; past
//! [`MAX_DESK_OPENS`] the oldest is dropped (engineering principle 15).
//!
//! The path is checked as an agent's `file_open` is: it must sit inside a
//! checkout Hide shows on a connected machine, by whole path segments, and
//! that checkout's own host reads it there (`ActionSource::read`) before the
//! open is kept, so a link out of the checkout or a missing file is refused
//! before anything waits.

use std::collections::VecDeque;

use serde::Deserialize;

use crate::model::DeskOpenSnapshot;
use crate::runtime::Runtime;
use crate::runtime::workspace_control::{DESK_REFUSAL, path_within};
use crate::workspace_control::{
    Action, ActionMaterial, ActionPreparation, ActionResult, ActionSource, Context, Refusal,
};

/// The most opens that wait for a window; a newer one drops the oldest.
pub(crate) const MAX_DESK_OPENS: usize = 16;

/// One open waiting for a window, with the request id that made it, so the
/// same command run again answers the same open.
#[derive(Clone, Debug)]
pub(super) struct DeskOpen {
    request_id: String,
    open: DeskOpenSnapshot,
}

#[derive(Default)]
pub(super) struct DeskOpens {
    waiting: VecDeque<DeskOpen>,
    next_id: u64,
}

impl DeskOpens {
    pub(super) fn snapshot(&self) -> Vec<DeskOpenSnapshot> {
        self.waiting.iter().map(|open| open.open.clone()).collect()
    }

    pub(super) fn clear(&mut self) -> bool {
        let had = !self.waiting.is_empty();
        self.waiting.clear();
        had
    }
}

/// `desk_open_taken`, from a window that opened the entry in its room.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime) struct TakenPayload {
    id: u64,
}

/// `hide diff open` from Desk: its tab panel has no diff tab yet.
const DIFF_UNSUPPORTED: Refusal = Refusal {
    reason: "desk_diff_unsupported",
    next_action: "Desk's tab panel shows files, not diffs, for now: open the changed file with hide file open <path>",
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

    /// Keeps Desk's open once its host has read it, for a window to take.
    /// The same request id, or the same file still waiting, answers the open
    /// already kept.
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
        let open = DeskOpenSnapshot {
            id: 0,
            device_id: target.device_id,
            root: target.root,
            path: material.path,
        };
        let opens = &mut self.desk.opens;
        let id = match opens.waiting.iter_mut().find(|waiting| {
            waiting.open.device_id == open.device_id
                && waiting.open.root == open.root
                && waiting.open.path == open.path
        }) {
            Some(waiting) => {
                waiting.request_id = request_id.to_owned();
                waiting.open.id
            }
            None => {
                opens.next_id += 1;
                let id = opens.next_id;
                if opens.waiting.len() >= MAX_DESK_OPENS {
                    opens.waiting.pop_front();
                    crate::diagnostic!(serde_json::json!({
                        "component": "desk",
                        "kind": "desk.open_dropped",
                        "cap": MAX_DESK_OPENS,
                    }));
                }
                opens.waiting.push_back(DeskOpen {
                    request_id: request_id.to_owned(),
                    open: DeskOpenSnapshot { id, ..open },
                });
                id
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk.open",
            "open": id,
            "waiting": self.desk.opens.waiting.len(),
        }));
        self.compose_desk();
        Ok(self.desk_open_result(request_id, id))
    }

    /// `desk_open_taken`: a window opened the entry in its room.
    pub(in crate::runtime) fn desk_open_taken(&mut self, payload: TakenPayload) -> bool {
        let before = self.desk.opens.waiting.len();
        self.desk
            .opens
            .waiting
            .retain(|waiting| waiting.open.id != payload.id);
        before != self.desk.opens.waiting.len()
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
            .waiting
            .iter()
            .find(|waiting| waiting.request_id == request_id)
            .map(|waiting| waiting.open.id)
    }
}
