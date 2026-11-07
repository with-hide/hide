//! Add a project's folder list remembers the last folder it showed on each
//! machine, so the next opening there starts in it (PRD device-add-project
//! D-04, B4): this machine's in `add_project_folder`, a device's in its own
//! entry of `device_add_project_folders`, both kept across a restart.

use super::*;
use serde::Deserialize;

/// The longest folder path remembered; a longer one is refused rather than
/// kept (engineering 15).
const FOLDER_PATH_CAP: usize = 4096;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AddProjectFolderPayload {
    /// The machine the folder is on; empty is the core's own.
    #[serde(default)]
    pub(super) device_id: String,
    /// The folder as the machine's node answered it (`HomeListing::path`).
    pub(super) path: String,
}

impl Runtime {
    /// Remembers `path` as the folder the list last showed on its machine.
    /// A machine that is neither this one nor a registered device, an empty
    /// path and one past the cap are refused to the log and change nothing.
    pub(super) fn set_add_project_folder(&mut self, payload: AddProjectFolderPayload) -> bool {
        let own = payload.device_id.is_empty() || payload.device_id == self.node.as_str();
        let known = own
            || self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .any(|device| device.id == payload.device_id);
        if !known || payload.path.is_empty() || payload.path.len() > FOLDER_PATH_CAP {
            crate::diagnostic!(serde_json::json!({
                "component": "add_project", "kind": "folder_refused",
                "device": payload.device_id, "known": known, "path_len": payload.path.len(),
            }));
            return false;
        }
        let ui = &mut self.snapshot.ui_state;
        let current = if own {
            ui.add_project_folder.as_deref()
        } else {
            ui.device_add_project_folders
                .get(&payload.device_id)
                .map(String::as_str)
        };
        if current == Some(payload.path.as_str()) {
            return false;
        }
        if own {
            ui.add_project_folder = Some(payload.path);
        } else {
            ui.device_add_project_folders
                .insert(payload.device_id, payload.path);
        }
        self.persist_ui_state();
        true
    }

    /// The paths of the projects registered on `device_id`, in the wire
    /// spelling the node that holds them answered.
    pub(crate) fn registered_paths(&self, device_id: &str) -> Vec<String> {
        self.snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|registration| registration.device_id == device_id)
            .map(|registration| registration.path.clone())
            .collect()
    }
}
