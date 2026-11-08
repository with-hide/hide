//! Retiring each device's old Home (`~/hide`, PRD retire-hide-home D-01..D-06).
//!
//! Hide no longer keeps a Home. A store the release before wrote still has a
//! registration flagged as a device's Home; `persistence` reads that flag once
//! and leaves an ordinary pinned project plus a `HomeRetirement` per device.
//! The device's helper is asked once to remove what Hide's marker lists in its
//! `~/hide`, and its answer settles the registration: the folder still there
//! (the operator's files, or a session or pane recorded in it) keeps the
//! project, named `hide`; an empty folder takes the registration with it.
//!
//! A device whose helper has not updated is not asked (D-06); it is asked when
//! its updated helper becomes ready. A failure reaches the diagnostic log only
//! (design principle 13): the project stays, and a refusal is not asked again
//! until the next launch. The helper's retirement converges on a rerun, so a
//! restart in the middle of one ends in the same state.

use super::*;
use crate::node_access::LinkError;
use hide_node_link::home::HomeRetired;

impl Runtime {
    /// Asks the helper of every device with a pending retirement whose helper
    /// is ready and whose projects the core has seen, once. Every UI state
    /// write, every applied session and every helper that becomes ready come
    /// here, so it costs nothing while no retirement is pending.
    pub(super) fn request_home_retirements(&mut self) {
        if self.snapshot.ui_state.home_retire.is_empty() {
            return;
        }
        let Some((runtime, notifier)) = self
            .worker_context
            .as_ref()
            .map(|context| (context.runtime.clone(), context.notifier.clone()))
        else {
            return;
        };
        let mut devices: Vec<String> = self
            .snapshot
            .ui_state
            .home_retire
            .iter()
            .map(|retirement| retirement.device_id.clone())
            .collect();
        devices.dedup();
        for device in devices {
            if self.home_retiring.contains(&device) {
                continue;
            }
            let Some(keep_empty) = self.home_kept_open(&device) else {
                continue;
            };
            // Asking starts no helper: a device that is not linked, or whose
            // helper predates this release, is left as it is.
            let Ok((_, host)) = self.linked_node(&device) else {
                continue;
            };
            if host.predates_current_protocol() {
                continue;
            }
            self.home_retiring.insert(device.clone());
            if let Err(message) = crate::worktree_control::spawn_home_retire(
                runtime.clone(),
                notifier.clone(),
                device.clone(),
                keep_empty,
                host,
            ) {
                self.home_retiring.remove(&device);
                self.log_home_retire_failure(&device, "home.retire_failed", &message);
            }
        }
    }

    /// Whether the device's old Home holds an open pane, so its folder must
    /// stay even when nothing else does; `None` while the core has not yet
    /// seen the device's projects, which is when a pane could be missed.
    fn home_kept_open(&self, device: &str) -> Option<bool> {
        let local = device == self.node.as_str();
        let rows = if local {
            self.snapshot.navigator.workspaces.as_slice()
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())?
                .workspaces
                .as_slice()
        };
        let mut keep = false;
        for retirement in &self.snapshot.ui_state.home_retire {
            if retirement.device_id != device {
                continue;
            }
            // A registration the operator removed since has nothing to hold.
            if !self
                .snapshot
                .ui_state
                .workspace_registrations
                .iter()
                .any(|registration| registration.id == retirement.registration_id)
            {
                continue;
            }
            if !rows.iter().any(|row| row.id == retirement.registration_id) {
                return None;
            }
            let remote = (!local).then_some(device);
            keep |= !self
                .project_panes(&retirement.registration_id, remote)
                .1
                .is_empty();
        }
        Some(keep)
    }

    /// `device`'s helper became ready: a retirement it had to wait for is
    /// asked now (D-06, B13).
    pub(super) fn home_helper_ready(&mut self, device: &str) {
        if self
            .snapshot
            .ui_state
            .home_retire
            .iter()
            .any(|retirement| retirement.device_id == device)
        {
            self.request_home_retirements();
        }
    }

    /// The helper's answer. A kept folder leaves the registration as the
    /// ordinary `hide` project it already is; a folder that is gone takes the
    /// registration with it. A failure keeps the project and only the log
    /// hears of it (B11).
    pub(crate) fn ingest_home_retired(
        &mut self,
        device: &str,
        retired: Result<HomeRetired, LinkError>,
    ) {
        let retired = match retired {
            Ok(retired) => retired,
            Err(error) => {
                // A link that dropped is asked again when it is back; a
                // helper that refused is not asked again this launch.
                if !matches!(error, LinkError::Refused(_)) {
                    self.home_retiring.remove(device);
                }
                self.log_home_retire_failure(device, "home.retire_failed", &error.to_string());
                return;
            }
        };
        crate::diagnostic!(serde_json::json!({
            "component": "home", "kind": "home.retired", "target": device,
            "marked": retired.marked, "kept": retired.kept,
            "removed": retired.removed.len(), "changed": retired.changed.len(),
        }));
        for name in &retired.changed {
            crate::diagnostic!(serde_json::json!({
                "component": "home", "kind": "home.entry_left", "target": device,
                "name": name,
            }));
        }
        let settled: Vec<String> = self
            .snapshot
            .ui_state
            .home_retire
            .iter()
            .filter(|retirement| retirement.device_id == device)
            .map(|retirement| retirement.registration_id.clone())
            .collect();
        self.snapshot
            .ui_state
            .home_retire
            .retain(|retirement| retirement.device_id != device);
        if retired.kept {
            self.persist_ui_state();
            return;
        }
        let mut removed = false;
        for registration_id in settled {
            removed |= self.retire_workspace_registration(&registration_id);
        }
        if !removed {
            self.persist_ui_state();
        }
    }

    fn log_home_retire_failure(&self, device: &str, kind: &str, message: &str) {
        crate::diagnostic!(serde_json::json!({
            "component": "home", "kind": kind, "target": device, "message": message,
        }));
    }
}
