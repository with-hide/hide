//! Each device's Home (`~/hide`, PRD home-device-rail D-01..D-08): a pinned
//! registration flagged `home`, made by the first start or tab there, whose
//! folder links every other registered project of that device.
//!
//! The runtime decides what a Home holds and records the registration; the
//! device's file helper (this machine's in process) does the disk work on a
//! worker, never under this mutex. A start waits for the sync before its tab
//! opens; a registration change afterwards syncs in the background and
//! reports only to the diagnostic log (design principle 13).

use super::*;
use crate::node_access::LinkError;
use hide_node_link::home::HomeSynced;

/// Where one device's Home links stand. `requested` is the project set last
/// sent to its helper, by a start or a background sync, whatever it answered;
/// `None` until the first sync since launch, so the first registration change
/// is synced rather than taken as already done. `in_flight` holds back a second
/// background sync while one runs; a start's sync does not wait for it, because
/// the helper runs syncs of one Home one after the other (`hide_host::home`).
/// `deferred` marks a change that found the helper not ready: the device is
/// left alone, its helper not asked again, until the helper is ready.
/// `requested` is also set to the set a sync applied when it answers, since
/// the helper runs syncs in the order they take its lock, not the order they
/// were sent; a newer set that lost that race is then sent again.
#[derive(Clone, Debug, Default)]
pub(crate) struct HomeLinkState {
    requested: Option<Vec<String>>,
    in_flight: bool,
    deferred: bool,
}

/// The label Home's registration and its first tab carry.
const HOME_LABEL: &str = "Home";

impl Runtime {
    /// The paths `device`'s Home links: every project registered there but
    /// Home itself (D-04 counts from registrations, not from the folder).
    pub(super) fn home_projects(&self, device: &str) -> Vec<String> {
        let mut projects: Vec<String> = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|registration| registration.device_id == device && !registration.home)
            .map(|registration| registration.path.clone())
            .collect();
        projects.sort();
        projects.dedup();
        projects
    }

    /// A start or a new tab in `device`'s Home: the helper brings the folder
    /// in step first, and the tab opens once it has (D-04, B18, B21).
    pub(super) fn start_in_home(
        &mut self,
        device: &str,
        agent_kind: Option<String>,
        model: Option<String>,
        prompt: Option<String>,
        request_id: Option<String>,
    ) -> bool {
        let local = device == self.node.as_str();
        if !local
            && !self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .any(|registration| registration.id == device)
        {
            self.set_request_error(
                "agent_start.unknown_device",
                format!("No device named {device} is registered"),
                false,
                request_id.as_deref(),
            );
            return true;
        }
        let target = if local {
            self.live
                .as_ref()
                .map(|context| live::TabTarget::local(context, self.live_generation))
        } else {
            self.remote_controls
                .get(device)
                .map(live::TabTarget::device)
        };
        let Some(target) = target else {
            self.set_request_error(
                "home.unavailable",
                "The device's Herdr is not connected, so Home cannot open there",
                true,
                request_id.as_deref(),
            );
            return true;
        };
        let host = match self.node_link(device) {
            Ok(host) => host,
            Err(message) => {
                self.set_request_error("home.unavailable", message, true, request_id.as_deref());
                return true;
            }
        };
        let id =
            match self.begin_task_operation("agent_start", None, None, None, agent_kind.clone()) {
                Ok(id) => id,
                Err(message) => {
                    self.set_request_error(
                        "task_operation.busy",
                        message,
                        true,
                        request_id.as_deref(),
                    );
                    return true;
                }
            };
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.request_id = request_id;
            operation.device_id = (!local).then(|| device.to_owned());
        }
        self.remember_agent_choice(agent_kind.as_deref(), model.as_deref());
        self.set_task_agent_launch(
            id,
            prompt,
            agent_choice::agent_arguments(agent_kind.as_deref(), model.as_deref(), &[]),
            hide_agent_adapter::LaunchChoice {
                model,
                effort: None,
            },
        );
        let projects = self.home_projects(device);
        self.home_links
            .entry(device.to_owned())
            .or_default()
            .requested = Some(projects.clone());
        let request = live::HomeStartRequest {
            id,
            device_id: device.to_owned(),
            projects,
            host,
        };
        if let Err(message) = live::spawn_home_start(target, request) {
            self.forget_home_sync(device);
            return self.ingest_task_operation_result(id, Err(message));
        }
        true
    }

    /// The Home start's sync answered: record Home and name the tab to open,
    /// or fail the start where the operator asked for it.
    pub(crate) fn ingest_home_start_sync(
        &mut self,
        id: u64,
        device: &str,
        projects: &[String],
        synced: Result<HomeSynced, LinkError>,
    ) -> Option<live::CheckoutTabRequest> {
        if synced.is_err() {
            self.forget_home_sync(device);
        }
        let operation = self.snapshot.task_operation.as_ref()?;
        if operation.id != id || operation.phase != "working" {
            return None;
        }
        let request_id = operation.request_id.clone();
        let agent_kind = operation.agent_kind.clone();
        let synced = match synced {
            Ok(synced) => synced,
            Err(LinkError::Refused(error))
                if error.code == hide_node_link::error::ErrorCode::HomeConflict =>
            {
                self.log_home_sync_failure(device, "home.conflict", &error.message);
                self.set_request_error(
                    "home.conflict",
                    &error.message,
                    false,
                    request_id.as_deref(),
                );
                self.ingest_task_operation_result(id, Err(error.message));
                return None;
            }
            Err(error) => {
                let message = format!("Home could not be prepared: {error}");
                self.log_home_sync_failure(device, "home.sync_failed", &message);
                self.ingest_task_operation_result(id, Err(message));
                return None;
            }
        };
        self.log_home_sync(device, projects, &synced);
        self.record_home_applied(device, projects);
        self.register_home(device, &synced.home);
        // The agent may write every linked project through its link (D-02,
        // D-08); supported CLIs take the real folders as extra roots. The helper
        // links no folder whose path Herdr could not pass (`control_character`).
        let folders: Vec<String> = synced
            .links
            .iter()
            .map(|link| link.target.clone())
            .collect();
        let arguments = agent_choice::agent_arguments(agent_kind.as_deref(), None, &folders);
        self.extend_task_agent_args(id, arguments);
        if let Some(operation) = self.snapshot.task_operation.as_mut() {
            operation.repository_root = Some(synced.home.clone());
        }
        let label = self
            .home_workspace(device)
            .and_then(|workspace| workspace.checkouts.first())
            .map(|checkout| checkout.next_tab_label.clone())
            .unwrap_or_else(|| crate::model::next_tab_label(std::iter::empty()));
        // A Folder owner is found by its mark when Herdr already has it open,
        // so the same request serves the first tab and every later one.
        let host = TabHost::Open(crate::checkout_owner::OwnerOpen::for_checkout(
            device,
            &synced.home,
            &synced.home,
            false,
            HOME_LABEL,
        ));
        // A registration that changed while this sync ran is caught up now.
        self.request_home_link_syncs();
        Some(live::CheckoutTabRequest {
            resume_reference: None,
            id,
            agent_kind: None,
            resume_scope: None,
            checkout_path: synced.home,
            label,
            host,
        })
    }

    /// The navigator row of `device`'s Home, once the catalog carries it.
    fn home_workspace(&self, device: &str) -> Option<&WorkspaceSnapshot> {
        let workspaces = if device == self.node.as_str() {
            self.snapshot.navigator.workspaces.as_slice()
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())
                .map(|session| session.workspaces.as_slice())
                .unwrap_or_default()
        };
        workspaces.iter().find(|workspace| workspace.is_home)
    }

    /// Records `home` as `device`'s Home: a pinned registration flagged
    /// `home`. A registration already at that path becomes Home rather than a
    /// second row.
    fn register_home(&mut self, device: &str, home: &str) {
        let local = device == self.node.as_str();
        let registrations = &mut self.snapshot.ui_state.workspace_registrations;
        let existing = registrations
            .iter_mut()
            .find(|registration| registration.device_id == device && registration.path == home)
            .map(|registration| {
                let changed = !registration.home || !registration.pinned;
                registration.home = true;
                registration.pinned = true;
                changed
            });
        let changed = match existing {
            Some(changed) => changed,
            None => {
                // A Home that moved (another account's HOME) is one row still.
                registrations.retain(|registration| {
                    !(registration.device_id == device && registration.home)
                });
                let registration = if local {
                    match workspace::registration(home, HOME_LABEL, self.node.as_str()) {
                        Ok(registration) => registration,
                        Err(message) => {
                            self.log_home_sync_failure(device, "home.register_failed", &message);
                            return;
                        }
                    }
                } else {
                    crate::model::WorkspaceRegistration {
                        primary_checkout_id: None,
                        id: crate::device_catalog::project_id(device, Path::new(home)),
                        label: HOME_LABEL.to_owned(),
                        path: home.to_owned(),
                        device_id: device.to_owned(),
                        pinned: false,
                        home: false,
                    }
                };
                self.snapshot.ui_state.workspace_registrations.push(
                    crate::model::WorkspaceRegistration {
                        pinned: true,
                        home: true,
                        ..registration
                    },
                );
                true
            }
        };
        if !changed {
            return;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "home", "kind": "home.registered", "target": device,
        }));
        self.persist_ui_state();
        if !local {
            self.request_device_facts(device);
            self.refresh_device_catalog(device);
        }
    }

    /// Runs a background link sync for every device whose Home exists and
    /// whose registered projects changed since its last sync (D-06, B19).
    /// Every registration change is persisted, so `persist_ui_state` calls
    /// this; a device without a Home gets nothing written (D-04).
    pub(super) fn request_home_link_syncs(&mut self) {
        let Some(context) = self
            .worker_context
            .as_ref()
            .map(|context| context.runtime.clone())
        else {
            return;
        };
        let devices: Vec<String> = self
            .snapshot
            .ui_state
            .workspace_registrations
            .iter()
            .filter(|registration| registration.home)
            .map(|registration| registration.device_id.clone())
            .collect();
        for device in devices {
            let projects = self.home_projects(&device);
            let state = self.home_links.entry(device.clone()).or_default();
            if state.in_flight || state.deferred || state.requested.as_ref() == Some(&projects) {
                continue;
            }
            // Nothing is recorded as sent until it is: a device whose helper
            // is not ready is asked once and then left until it is
            // (`home_helper_ready`), since every UI state write lands here.
            let host = match self.node_link(&device) {
                Ok(host) => host,
                Err(message) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "home", "kind": "home.link_sync_deferred",
                        "target": device, "message": message, "projects": projects.len(),
                    }));
                    self.home_links.entry(device).or_default().deferred = true;
                    continue;
                }
            };
            let state = self.home_links.entry(device.clone()).or_default();
            state.requested = Some(projects.clone());
            state.in_flight = true;
            state.deferred = false;
            if let Err(message) =
                live::spawn_home_link_sync(context.clone(), device.clone(), projects, host)
            {
                if let Some(state) = self.home_links.get_mut(&device) {
                    state.in_flight = false;
                }
                self.forget_home_sync(&device);
                self.log_home_sync_failure(&device, "home.sync_failed", &message);
            }
        }
    }

    /// `device`'s helper became ready: a link change it missed is sent now.
    /// Launching Hide writes nothing, so a device with nothing deferred is
    /// left alone.
    pub(super) fn home_helper_ready(&mut self, device: &str) {
        let Some(state) = self.home_links.get_mut(device) else {
            return;
        };
        if std::mem::take(&mut state.deferred) {
            self.request_home_link_syncs();
        }
    }

    /// A background link sync answered; only the log hears of it.
    pub(crate) fn ingest_home_link_sync(
        &mut self,
        device: &str,
        projects: &[String],
        synced: Result<HomeSynced, LinkError>,
    ) {
        if let Some(state) = self.home_links.get_mut(device) {
            state.in_flight = false;
        }
        match synced {
            // A registration that changed while this sync ran, or a newer
            // set this one ran after, is caught up now.
            Ok(synced) => {
                self.log_home_sync(device, projects, &synced);
                self.record_home_applied(device, projects);
                self.request_home_link_syncs();
            }
            // The failed set stays `requested`, so it is not sent again until
            // the registrations change or a Home start brings it in step.
            Err(error) => {
                self.log_home_sync_failure(device, "home.sync_failed", &error.to_string())
            }
        }
    }

    /// The set a sync just applied is what `~/hide` holds now, and the helper
    /// answered, so nothing is deferred.
    fn record_home_applied(&mut self, device: &str, projects: &[String]) {
        let state = self.home_links.entry(device.to_owned()).or_default();
        state.requested = Some(projects.to_vec());
        state.deferred = false;
    }

    /// A sync that did not run or did not finish leaves the links unknown: the
    /// next registration change sends them again.
    fn forget_home_sync(&mut self, device: &str) {
        if let Some(state) = self.home_links.get_mut(device) {
            state.requested = None;
        }
    }

    /// B20: a link dropped because its project moved away is a log line, not
    /// a screen state.
    fn log_home_sync(&self, device: &str, projects: &[String], synced: &HomeSynced) {
        crate::diagnostic!(serde_json::json!({
            "component": "home", "kind": "home.synced", "target": device,
            "created": synced.created, "projects": projects.len(),
            "links": synced.links.len(),
        }));
        for dropped in &synced.dropped {
            crate::diagnostic!(serde_json::json!({
                "component": "home", "kind": "home.link_dropped", "target": device,
                "link": dropped.name, "reason": dropped.reason,
            }));
        }
        for skipped in &synced.skipped {
            crate::diagnostic!(serde_json::json!({
                "component": "home", "kind": "home.link_skipped", "target": device,
                "reason": skipped.reason,
            }));
        }
    }

    fn log_home_sync_failure(&self, device: &str, kind: &str, message: &str) {
        crate::diagnostic!(serde_json::json!({
            "component": "home", "kind": kind, "target": device, "message": message,
        }));
    }
}
