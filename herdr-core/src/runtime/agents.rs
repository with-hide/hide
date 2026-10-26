use super::*;

/// Which of this machine's panes the lineage pass saw, and how each one's
/// connection stood, so a Reopen's published state ends with the need for it.
#[derive(Default)]
struct ReopenScope {
    panes: HashSet<String>,
    connected: HashSet<String>,
    not_connected: HashSet<String>,
}

impl Runtime {
    pub(super) fn advance_remote_file_generation(&mut self) -> u64 {
        self.next_remote_file_generation = self.next_remote_file_generation.saturating_add(1);
        self.next_remote_file_generation
    }

    pub(super) fn mark_remote_files_unavailable(
        &mut self,
        status_index: usize,
        root_path: String,
        message: String,
        generation: u64,
    ) {
        self.snapshot.status.remote[status_index].files = RemoteFileListSnapshot {
            root_path: Some(root_path),
            state: "unavailable".to_owned(),
            entries: Vec::new(),
            message: Some(message),
            generation,
        };
    }

    /// Lists the device's file panel root again once its helper is ready, when
    /// the last listing could not run for want of it.
    pub(super) fn relist_remote_files(&mut self, target_id: &str) {
        let Some(root_path) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == target_id)
            .filter(|status| matches!(status.files.state.as_str(), "unavailable" | "not_allowed"))
            .and_then(|status| status.files.root_path.clone())
        else {
            return;
        };
        self.request_remote_file_list(RemoteFileListPayload {
            target_id: target_id.to_owned(),
            root_path,
        });
    }

    pub(super) fn request_remote_file_list(&mut self, payload: RemoteFileListPayload) -> bool {
        let target_id = payload.target_id;
        let root_path = payload.root_path;
        let Some(status_index) = self
            .snapshot
            .status
            .remote
            .iter()
            .position(|status| status.target_id == target_id)
        else {
            self.set_error(
                "remote.files.unknown_target",
                format!("Remote file listing requested an unconfigured target {target_id}"),
                false,
            );
            return true;
        };
        if !Path::new(&root_path).is_absolute()
            || root_path.bytes().any(|byte| byte.is_ascii_control())
        {
            let generation = self.advance_remote_file_generation();
            self.mark_remote_files_unavailable(
                status_index,
                root_path.clone(),
                "Remote file root must be an absolute single-line path".to_owned(),
                generation,
            );
            self.set_error(
                "remote.files.invalid_root",
                format!("Remote file root is invalid for target {target_id}"),
                false,
            );
            return true;
        }
        let status = &self.snapshot.status.remote[status_index];
        if status.state != "connected" {
            let message = status
                .message
                .clone()
                .unwrap_or_else(|| format!("Remote target {target_id} is not connected"));
            let generation = self.advance_remote_file_generation();
            self.mark_remote_files_unavailable(status_index, root_path, message, generation);
            return true;
        }
        let root_is_authoritative = status.session.as_ref().is_some_and(|session| {
            session
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .any(|checkout| checkout.path == root_path)
        });
        if !root_is_authoritative {
            let generation = self.advance_remote_file_generation();
            self.mark_remote_files_unavailable(
                status_index,
                root_path.clone(),
                "Remote file root is not part of the authoritative Herdr session".to_owned(),
                generation,
            );
            self.set_error(
                "remote.files.unknown_root",
                format!("Remote file root {root_path} is not open on target {target_id}"),
                false,
            );
            return true;
        }
        if status.files.root_path.as_deref() == Some(root_path.as_str())
            && matches!(status.files.state.as_str(), "loading" | "ready")
        {
            return false;
        }
        // The device's helper lists it, as it lists the web Explorer's
        // folders: the same confinement to the checkout's opened root on
        // either shell (PRD S5.5 D-05). A device without a ready helper
        // lists nothing and says why.
        let channel = match self.node_link(&target_id) {
            Ok(channel) => channel,
            Err(message) => {
                let generation = self.advance_remote_file_generation();
                self.mark_remote_files_unavailable(status_index, root_path, message, generation);
                // A device without consent says what allowing it installs
                // and runs, so the shell can ask for it where the files are
                // (PRD S5.5 B50).
                let host = self.host_snapshot(&target_id);
                if host.state == "not_allowed" {
                    let helper_root = host.helper_root.unwrap_or_else(|| self.host_helper_root());
                    let cli_dir = host.cli_dir.unwrap_or_default();
                    let files = &mut self.snapshot.status.remote[status_index].files;
                    files.state = "not_allowed".to_owned();
                    files.message = Some(format!(
                        "Hide reads this device's files through a small helper it installs at {helper_root} and runs only while Hide is connected over SSH, with the hide command beside it linked in {cli_dir}. Allowing it lets Hide read and change files and Git in this device's checkouts and lets its panes open files and pages in Hide; later updates within the same scope install without asking again."
                    ));
                }
                return true;
            }
        };
        let Some(context) = self.worker_context.clone() else {
            let message = "The remote file worker is unavailable".to_owned();
            let generation = self.advance_remote_file_generation();
            self.mark_remote_files_unavailable(
                status_index,
                root_path,
                message.clone(),
                generation,
            );
            self.set_error("remote.files.worker_unavailable", message, true);
            return true;
        };

        let generation = self.advance_remote_file_generation();
        self.snapshot.status.remote[status_index].files = RemoteFileListSnapshot {
            root_path: Some(root_path.clone()),
            state: "loading".to_owned(),
            entries: Vec::new(),
            message: None,
            generation,
        };
        self.push_diagnostic(
            "remote.files.requested",
            format!("Listing remote files for {target_id} at {root_path}"),
        );
        let worker_target_id = target_id.clone();
        let worker_root_path = root_path.clone();
        match thread::Builder::new()
            .name(format!("herdr-core-remote-files-{target_id}"))
            .spawn(move || {
                let result =
                    crate::node_access::list_folder(channel.as_ref(), &worker_root_path, "")
                        .map_err(|error| error.to_string());
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_remote_file_list_result(
                        &worker_target_id,
                        &worker_root_path,
                        generation,
                        result,
                    ),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            }) {
            Ok(_) => true,
            Err(error) => self.ingest_remote_file_list_result(
                &target_id,
                &root_path,
                generation,
                Err(format!("Remote file worker could not be started: {error}")),
            ),
        }
    }

    pub(super) fn ingest_remote_file_list_result(
        &mut self,
        target_id: &str,
        root_path: &str,
        generation: u64,
        result: Result<hide_node_link::list::Listing, String>,
    ) -> bool {
        let Some(status_index) = self
            .snapshot
            .status
            .remote
            .iter()
            .position(|status| status.target_id == target_id)
        else {
            self.set_error(
                "remote.files.unknown_target",
                format!("Remote file result named an unconfigured target {target_id}"),
                false,
            );
            return true;
        };
        let files = &self.snapshot.status.remote[status_index].files;
        if files.generation != generation || files.root_path.as_deref() != Some(root_path) {
            self.push_diagnostic(
                "remote.files.stale",
                format!(
                    "Ignored stale remote file result for {target_id} at {root_path} generation {generation}"
                ),
            );
            return true;
        }
        match result {
            Ok(listing) => {
                // The helper's order: folders first, then the natural name
                // order the Explorer uses.
                let base = root_path.trim_end_matches('/');
                let entries = listing.entries;
                let entry_count = entries.len();
                self.snapshot.status.remote[status_index].files = RemoteFileListSnapshot {
                    root_path: Some(root_path.to_owned()),
                    state: "ready".to_owned(),
                    entries: entries
                        .into_iter()
                        .map(|entry| RemoteFileEntrySnapshot {
                            path: format!("{base}/{}", entry.name),
                            name: entry.name,
                            is_directory: entry.is_directory,
                        })
                        .collect(),
                    message: None,
                    generation,
                };
                self.push_diagnostic(
                    "remote.files.ready",
                    format!("Listed {entry_count} remote files for {target_id} at {root_path}"),
                );
            }
            Err(message) => {
                self.mark_remote_files_unavailable(
                    status_index,
                    root_path.to_owned(),
                    message.clone(),
                    generation,
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_files",
                    "kind": "remote.files_failed",
                    "target": target_id,
                    "root_path": root_path,
                    "generation": generation,
                    "message": message,
                }));
            }
        }
        true
    }

    pub(super) fn request_remote_control(&mut self, payload: RemoteControlPayload) -> bool {
        self.request_remote_control_inner(payload, false, FailureReport::Notice);
        true
    }

    pub(super) fn request_remote_terminal(&mut self, payload: RemoteControlPayload) -> bool {
        self.send_remote_terminal(payload, FailureReport::Notice);
        true
    }

    /// Asks for a terminal tab on a device and says whether the request went
    /// to it; a refusal before that is reported as `report` says.
    pub(super) fn send_remote_terminal(
        &mut self,
        payload: RemoteControlPayload,
        report: FailureReport,
    ) -> bool {
        self.request_remote_control_inner(payload, true, report)
    }

    /// Whether the request was handed to the device's worker.
    fn request_remote_control_inner(
        &mut self,
        payload: RemoteControlPayload,
        terminal_intent: bool,
        report: FailureReport,
    ) -> bool {
        let target_id = payload.target_id;
        let request_id = payload.request_id;
        let focus_device = payload.focus_device;
        let pane_focus_target = match (&payload.request, payload.report_pane_focus_outcome) {
            (RemoteControlRequest::FocusPane { pane_id }, true) => Some(pane_id.clone()),
            _ => None,
        };
        macro_rules! fail_request {
            ($kind:expr, $message:expr, $retryable:expr $(,)?) => {{
                let message = $message;
                if pane_focus_target.is_some() {
                    self.finish_pane_focus_request_by_id(
                        &request_id,
                        "failed",
                        Some(message.clone()),
                        $retryable,
                    );
                }
                self.report_remote_failure(report, &target_id, $kind, message, $retryable);
                return false;
            }};
        }
        if target_id.trim().is_empty() || request_id.trim().is_empty() {
            fail_request!(
                "remote.control.invalid_request",
                "Remote control requires non-empty target_id and request_id".to_owned(),
                false,
            );
        }
        if self
            .remote_control_requests
            .iter()
            .any(|known| known == &(target_id.clone(), request_id.clone()))
        {
            self.push_diagnostic(
                "remote.control.duplicate_ignored",
                format!("Ignored duplicate remote request {request_id} for {target_id}"),
            );
            return false;
        }
        if let Some(pane_id) = pane_focus_target.as_deref() {
            self.snapshot.status.pane_focus_request = Some(PaneFocusRequestSnapshot {
                request_id: request_id.clone(),
                target_pane_id: pane_id.to_owned(),
                phase: "pending".to_owned(),
                message: None,
                retryable: false,
            });
        }
        // Checkout navigation is core UI state. In particular an empty
        // checkout or draft must never turn into OpenOwner and start a PTY.
        if self.separate_view_areas()
            && let RemoteControlRequest::FocusWorkspace {
                workspace_id,
                checkout_id,
            } = &payload.request
        {
            let key = self
                .catalog_workspaces()
                .find(|workspace| workspace.device_id == target_id && workspace.id == *workspace_id)
                .and_then(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .find(|checkout| checkout_id.as_ref().is_none_or(|id| id == &checkout.id))
                })
                .map(|checkout| (target_id.clone(), checkout.path.clone()));
            let Some(key) = key else {
                fail_request!(
                    "remote.control.workspace_not_found",
                    "This checkout is no longer available. Select it again.".to_owned(),
                    false
                );
            };
            self.reconcile_node_agent_topology(&target_id);
            let live = self
                .workspace_views
                .as_ref()
                .and_then(|store| store.views.get(&key.0, &key.1))
                .and_then(|view| {
                    view.selected_agent_layout()
                        .active()
                        .filter(|id| !view.agent_groups.drafts.contains(*id))
                        .map(str::to_owned)
                });
            self.show_agent_workspace_tab(&key, live);
            self.remember_core_workspace_choice(&target_id, &request_id, key);
            self.supersede_start_navigation();
            if self.remote_control_requests.len() == 128 {
                self.remote_control_requests.pop_front();
            }
            self.remote_control_requests
                .push_back((target_id, request_id));
            return false;
        }
        let Some(context) = self.remote_controls.get(&target_id).cloned() else {
            fail_request!(
                "remote.control.unavailable",
                format!("Remote control is unavailable for target {target_id}"),
                true,
            );
        };
        let Some(remote) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|remote| remote.target_id == target_id)
        else {
            fail_request!(
                "remote.control.unknown_target",
                format!("Remote target {target_id} is not configured"),
                false,
            );
        };
        if remote.state != "connected" {
            fail_request!(
                "remote.control.not_connected",
                format!(
                    "Remote target {target_id} is {}; no command was sent",
                    remote.state
                ),
                true,
            );
        }
        let Some(session) = remote.session.clone() else {
            fail_request!(
                "remote.control.session_missing",
                format!("Remote target {target_id} has no authoritative session projection"),
                true,
            );
        };
        let connection_generation = *self
            .remote_connection_generations
            .entry(target_id.clone())
            .or_insert(0);
        let remote_mutation = remote_mutation_descriptor(&session, &payload.request);
        let mut source_pane_id = None;
        if let Some(pane_id) = payload.request.pane_id().map(str::to_owned) {
            if pane_id.trim().is_empty() {
                fail_request!(
                    "remote.control.invalid_pane",
                    "Remote pane control requires a non-empty pane_id".to_owned(),
                    false,
                );
            }
            let pane_exists = session.workspaces.iter().any(|workspace| {
                workspace.checkouts.iter().any(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                })
            });
            if !pane_exists {
                fail_request!(
                    "remote.control.pane_not_found",
                    format!("Pane {pane_id} does not belong to remote target {target_id}"),
                    false,
                );
            }
            source_pane_id = remote_pane_source_id(&target_id, &pane_id).map(str::to_owned);
            if source_pane_id.is_none() {
                fail_request!(
                    "remote.control.invalid_pane_scope",
                    format!("Pane {pane_id} is not scoped to remote target {target_id}"),
                    false,
                );
            }
            let status_unknown = matches!(&payload.request, RemoteControlRequest::ClosePane { .. })
                && session.agents.iter().any(|agent| {
                    agent.pane_id == pane_id
                        && (agent.requires_close_status_check
                            && self.failed_terminal_id(&agent.pane_id).is_none())
                });
            if status_unknown {
                self.report_remote_failure(report, &target_id,
                    "remote.control.close_status_unknown",
                    format!(
                        "Pane {pane_id} on {target_id} has an unknown activity status; refresh status before closing"
                    ),
                    true,
                );
                return false;
            }
            let needs_confirmation =
                matches!(&payload.request, RemoteControlRequest::ClosePane { .. })
                    && session.agents.iter().any(|agent| {
                        agent.pane_id == pane_id
                            && (agent.requires_close_confirmation
                                && self.failed_terminal_id(&agent.pane_id).is_none())
                    });
            if needs_confirmation && !payload.request.confirmed() {
                self.report_remote_failure(report, &target_id,
                    "remote.control.close_confirmation_required",
                    format!(
                        "Pane {pane_id} on {target_id} is working or needs attention; remote close requires confirmed=true"
                    ),
                    false,
                );
                return false;
            }
            if matches!(
                &payload.request,
                RemoteControlRequest::TogglePaneZoom { .. }
            ) && remote_pane_alone_unzoomed(&session, &pane_id)
            {
                self.push_diagnostic(
                    "remote.control.zoom_single_pane",
                    format!(
                        "Pane {pane_id} on {target_id} is its tab's only pane; zoom was not sent"
                    ),
                );
                return false;
            }
        }

        // The Workspace that holds the agent or tab the request chooses,
        // which is in front only once the device's Herdr has moved there.
        let agent_workspace_key = self
            .separate_view_areas()
            .then(|| self.remote_request_workspace_key(&target_id, &session, &payload.request))
            .flatten();
        // The Workspace this request chooses, remembered once the device's
        // front lands there (D-11): a Workspace opened from Main or an
        // Overview names its checkout; an agent or tab, the checkout holding
        // it.
        let chosen_key = match &payload.request {
            RemoteControlRequest::FocusWorkspace {
                workspace_id,
                checkout_id: Some(checkout_id),
            }
            | RemoteControlRequest::CreateTab {
                workspace_id,
                checkout_id: Some(checkout_id),
                ..
            } if self.separate_view_areas() => self.workspace_key(workspace_id, checkout_id),
            _ => agent_workspace_key,
        };
        let focused_tab = session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .find(|tab| match &payload.request {
                RemoteControlRequest::FocusTab { tab_id } => tab.id.as_ref() == Some(tab_id),
                RemoteControlRequest::FocusPane { pane_id } => {
                    tab.panes.iter().any(|pane| &pane.id == pane_id)
                }
                _ => false,
            })
            .and_then(|tab| tab.id.clone().map(|id| (id, tab.delegated)));
        if let Some(key) = chosen_key.as_ref()
            && let Some((tab, _)) = focused_tab.as_ref()
            && self.agent_tab_waiting_in(key, tab)
        {
            fail_request!(
                "agent_layout.display_limit",
                "This tab is waiting for an Agent area slot. Close a tab before selecting it."
                    .to_owned(),
                false
            );
        }
        let mut action = match payload.request {
            RemoteControlRequest::FocusPane { .. } => {
                RemoteControlAction::Pane(PaneControlAction::Focus {
                    pane_id: source_pane_id.expect("remote pane source id was validated"),
                })
            }
            RemoteControlRequest::SplitPane { direction, cwd, .. } => {
                RemoteControlAction::Pane(PaneControlAction::Split {
                    pane_id: source_pane_id.expect("remote pane source id was validated"),
                    direction,
                    cwd,
                })
            }
            RemoteControlRequest::TogglePaneZoom { .. } => {
                RemoteControlAction::Pane(PaneControlAction::ToggleZoom {
                    pane_id: source_pane_id.expect("remote pane source id was validated"),
                })
            }
            RemoteControlRequest::ClosePane { .. } => {
                RemoteControlAction::Pane(PaneControlAction::Close {
                    pane_id: source_pane_id.expect("remote pane source id was validated"),
                })
            }
            RemoteControlRequest::FocusWorkspace {
                workspace_id,
                checkout_id,
            } => {
                let Some((project, checkout)) =
                    remote_checkout(&session, &workspace_id, checkout_id.as_deref())
                else {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.workspace_not_found",
                        format!(
                            "Workspace {workspace_id} does not belong to remote target {target_id}"
                        ),
                        false,
                    );
                    return false;
                };
                // Opening a checkout brings its most recent tab forward from
                // whichever Herdr workspace holds it; only a checkout with no
                // tab opens its owner (D-13).
                let tab = session
                    .active_tab_ids
                    .get(&checkout.id)
                    .or_else(|| checkout.tabs.first().and_then(|tab| tab.id.as_ref()))
                    .and_then(|tab| remote_tab_source_id(&target_id, tab));
                match (tab, &checkout.owner_workspace_id) {
                    (Some(tab), _) => RemoteControlAction::FocusTab {
                        tab_id: tab.to_owned(),
                    },
                    (None, Some(owner)) => RemoteControlAction::FocusWorkspace {
                        workspace_id: owner.clone(),
                    },
                    (None, None) if checkout.unconfirmed => {
                        self.refuse_unconfirmed_owner(report, &target_id, checkout);
                        return false;
                    }
                    (None, None) => RemoteControlAction::OpenOwner {
                        owner: super::projects::owner_open(
                            project, checkout, &target_id, &self.node,
                        ),
                        cwd: checkout.path.clone(),
                        label: checkout.next_tab_label.clone(),
                        area_id: None,
                        group_id: None,
                        admission_id: None,
                    },
                }
            }
            RemoteControlRequest::FocusTab { tab_id } => {
                let exists = !tab_id.trim().is_empty()
                    && session.workspaces.iter().any(|workspace| {
                        workspace.checkouts.iter().any(|checkout| {
                            checkout
                                .tabs
                                .iter()
                                .any(|tab| tab.id.as_deref() == Some(tab_id.as_str()))
                        })
                    });
                if !exists {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.tab_not_found",
                        format!("Tab {tab_id} does not belong to remote target {target_id}"),
                        false,
                    );
                    return false;
                }
                let Some(source_id) = remote_tab_source_id(&target_id, &tab_id) else {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.invalid_tab_scope",
                        format!("Tab {tab_id} is not scoped to remote target {target_id}"),
                        false,
                    );
                    return false;
                };
                RemoteControlAction::FocusTab {
                    tab_id: source_id.to_owned(),
                }
            }
            RemoteControlRequest::CreateTab {
                workspace_id,
                checkout_id,
                cwd,
                label,
                area_id,
            } => {
                let Some((project, checkout)) =
                    remote_checkout(&session, &workspace_id, checkout_id.as_deref())
                else {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.workspace_not_found",
                        format!(
                            "Workspace {workspace_id} does not belong to remote target {target_id}"
                        ),
                        false,
                    );
                    return false;
                };
                if cwd.trim().is_empty() || label.trim().is_empty() {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.invalid_tab",
                        "Remote tab creation requires non-empty cwd and label",
                        false,
                    );
                    return false;
                }
                let key = (target_id.clone(), checkout.path.clone());
                let area_id = area_id.or_else(|| {
                    self.agent_layout_of(&key)
                        .map(|layout| layout.tree.active_area.clone())
                });
                if area_id.as_ref().is_some_and(|id| {
                    self.agent_layout_of(&key)
                        .is_none_or(|layout| layout.tree.area(id).is_none())
                }) {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "agent_layout.unknown_area",
                        "The Agent area is no longer available",
                        false,
                    );
                    return false;
                }
                let group_id = self
                    .workspace_views
                    .as_ref()
                    .and_then(|store| store.views.get(&key.0, &key.1))
                    .and_then(|view| view.agent_groups.selected.clone())
                    .map(String::into_boxed_str);
                // A new tab goes to the checkout's owner on the device, opened
                // first when none is (D-07, D-10).
                if checkout.owner_workspace_id.is_none() && checkout.unconfirmed {
                    self.refuse_unconfirmed_owner(report, &target_id, checkout);
                    return false;
                }
                match super::projects::tab_host(project, checkout, &target_id, &self.node) {
                    TabHost::Workspace(owner) => RemoteControlAction::CreateTab {
                        workspace_id: owner,
                        cwd,
                        label,
                        area_id,
                        group_id,
                        admission_id: None,
                    },
                    TabHost::Open(owner) => RemoteControlAction::OpenOwner {
                        cwd: owner.path().to_owned(),
                        owner,
                        label,
                        area_id,
                        group_id,
                        admission_id: None,
                    },
                }
            }
            RemoteControlRequest::CloseTab { tab_id, confirmed } => {
                let tab = session
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .flat_map(|checkout| checkout.tabs.iter())
                    .find(|tab| tab.id.as_deref() == Some(tab_id.as_str()));
                let Some(tab) = tab else {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.tab_not_found",
                        format!("Tab {tab_id} does not belong to remote target {target_id}"),
                        false,
                    );
                    return false;
                };
                let pane_ids = tab
                    .panes
                    .iter()
                    .map(|pane| pane.id.as_str())
                    .collect::<HashSet<_>>();
                let status_unknown = session.agents.iter().any(|agent| {
                    pane_ids.contains(agent.pane_id.as_str())
                        && (agent.requires_close_status_check
                            && self.failed_terminal_id(&agent.pane_id).is_none())
                });
                if status_unknown {
                    self.report_remote_failure(report, &target_id,
                        "remote.control.close_status_unknown",
                        format!(
                            "Tab {tab_id} on {target_id} has a pane whose activity status is unknown; refresh status before closing"
                        ),
                        true,
                    );
                    return false;
                }
                let needs_confirmation = session.agents.iter().any(|agent| {
                    pane_ids.contains(agent.pane_id.as_str())
                        && (agent.requires_close_confirmation
                            && self.failed_terminal_id(&agent.pane_id).is_none())
                });
                if needs_confirmation && !confirmed {
                    self.report_remote_failure(report, &target_id,
                        "remote.control.close_confirmation_required",
                        format!(
                            "Tab {tab_id} on {target_id} contains an agent that is working or needs attention; remote close requires confirmed=true"
                        ),
                        false,
                    );
                    return false;
                }
                let Some(source_id) = remote_tab_source_id(&target_id, &tab_id) else {
                    self.report_remote_failure(
                        report,
                        &target_id,
                        "remote.control.invalid_tab_scope",
                        format!("Tab {tab_id} is not scoped to remote target {target_id}"),
                        false,
                    );
                    return false;
                };
                RemoteControlAction::CloseTab {
                    tab_id: source_id.to_owned(),
                }
            }
        };

        let creation_admission = if terminal_intent {
            match &mut action {
                RemoteControlAction::CreateTab {
                    cwd, admission_id, ..
                }
                | RemoteControlAction::OpenOwner {
                    cwd, admission_id, ..
                } => {
                    let key = (target_id.clone(), cwd.clone());
                    let id = match self.try_reserve_agent_creation(&key) {
                        Ok(id) => id,
                        Err((kind, message)) => {
                            self.report_remote_failure(report, &target_id, kind, message, false);
                            return false;
                        }
                    };
                    *admission_id = Some(id);
                    Some((key, id))
                }
                _ => None,
            }
        } else {
            None
        };
        let chooses_focus = matches!(
            action,
            RemoteControlAction::Pane(PaneControlAction::Focus { .. })
                | RemoteControlAction::FocusTab { .. }
                | RemoteControlAction::FocusWorkspace { .. }
        );
        // A terminal tab chosen on the device's strip takes the surface from
        // a device file the editor shows, as a local tab does (`focus_tab`).
        if matches!(
            action,
            RemoteControlAction::FocusTab { .. }
                | RemoteControlAction::CreateTab { .. }
                | RemoteControlAction::FocusWorkspace { .. }
        ) && self.active_editor_tab_on_device(&target_id)
        {
            self.yield_surface_to_terminal();
        }
        let creation_key = remote_tab_creation_key(&target_id, &action);
        if let Some(key) = creation_key.as_ref()
            && !self.remote_tab_creations_in_flight.insert(key.clone())
        {
            self.push_diagnostic(
                "remote.control.duplicate_tab_ignored",
                format!(
                    "Ignored duplicate in-flight tab.create for workspace {} on {target_id}",
                    key.1
                ),
            );
            return false;
        }
        let remote_operation_key = if let Some(descriptor) = remote_mutation {
            self.remote_operations.retain(|_, operation| {
                !(operation.scope_id == descriptor.scope_id
                    && matches!(operation.phase.as_str(), "failed" | "refused"))
            });
            if self.geometry_tab_busy(&descriptor.scope_id)
                || self.remote_operations.values().any(|operation| {
                    operation.scope_id == descriptor.scope_id
                        && matches!(
                            operation.phase.as_str(),
                            "transmitting" | "awaiting_topology" | "unknown"
                        )
                })
            {
                fail_request!(
                    "remote.control.in_progress",
                    format!(
                        "{} is already running for {} on remote target {}",
                        descriptor.kind, descriptor.scope_id, target_id
                    ),
                    false,
                );
            }
            let operation_key = (target_id.clone(), request_id.clone());
            self.insert_remote_operation(
                &operation_key,
                descriptor,
                unix_milliseconds(),
                connection_generation,
            );
            Some(operation_key)
        } else {
            None
        };
        if self.remote_control_requests.len() == 128 {
            self.remote_control_requests.pop_front();
        }
        self.remote_control_requests
            .push_back((target_id.clone(), request_id.clone()));
        self.push_diagnostic(
            "remote.control.requested",
            format!("Sending {} to {target_id}", action.kind()),
        );
        let dispatched_request_id = request_id.clone();
        let failed_terminals = session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter().map(move |pane| (tab, pane)))
            .filter_map(|(tab, pane)| {
                let source = remote_pane_source_id(&target_id, &pane.id)?;
                let selected = match &action {
                    RemoteControlAction::Pane(PaneControlAction::Close { pane_id }) => {
                        pane_id == source
                    }
                    RemoteControlAction::CloseTab { tab_id } => {
                        tab.id
                            .as_deref()
                            .and_then(|id| remote_tab_source_id(&target_id, id))
                            == Some(tab_id.as_str())
                    }
                    _ => false,
                };
                selected
                    .then(|| {
                        pane.failed_terminal_id
                            .as_ref()
                            .map(|id| (source.to_owned(), id.clone()))
                    })
                    .flatten()
            })
            .collect();
        let scope_pane_ids: Vec<String> = session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .filter(|tab| match &action {
                RemoteControlAction::CloseTab { tab_id } => {
                    tab.id
                        .as_deref()
                        .and_then(|id| remote_tab_source_id(&target_id, id))
                        == Some(tab_id.as_str())
                }
                _ => false,
            })
            .flat_map(|tab| tab.panes.iter())
            .filter_map(|pane| remote_pane_source_id(&target_id, &pane.id).map(str::to_owned))
            .collect();
        let approved_peers = session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .filter(|pane| pane.requires_close_confirmation && pane.failed_terminal_id.is_none())
            .filter_map(|pane| {
                let source = remote_pane_source_id(&target_id, &pane.id)?;
                scope_pane_ids.contains(&source.to_owned()).then_some(())?;
                Some(live::ClosePeer {
                    pane_id: source.to_owned(),
                    terminal_id: pane.terminal_id.clone()?,
                    session: session
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane.id)
                        .and_then(|agent| agent.close_session.clone()),
                })
            })
            .collect();
        if let Err(message) = live::spawn_remote_control_checked(
            context,
            request_id,
            action,
            connection_generation,
            failed_terminals,
            live::CloseSafety {
                pane_ids: scope_pane_ids,
                approved_peers,
                deadline: std::time::Instant::now()
                    + std::time::Duration::from_millis(
                        remote_operation_key
                            .as_ref()
                            .and_then(|key| self.remote_operations.get(key))
                            .and_then(|operation| operation.deadline_at_unix_ms)
                            .unwrap_or_else(unix_milliseconds)
                            .saturating_sub(unix_milliseconds()),
                    ),
            },
        ) {
            if let Some(key) = remote_operation_key.as_ref() {
                self.fail_remote_operation_reported(report, key, message.clone(), false);
            }
            if let Some(key) = creation_key {
                self.remote_tab_creations_in_flight.remove(&key);
            }
            if let Some((key, id)) = creation_admission {
                self.finish_agent_creation(&key, id);
            }
            if pane_focus_target.is_some() {
                self.finish_pane_focus_request_by_id(
                    &dispatched_request_id,
                    "failed",
                    Some(message.clone()),
                    true,
                );
            }
            if remote_operation_key.is_none() {
                self.report_remote_failure(
                    report,
                    &target_id,
                    "remote.control.worker_failed",
                    message,
                    true,
                );
            }
            return false;
        }
        if chooses_focus {
            self.supersede_start_navigation();
        }
        if let Some(key) = chosen_key {
            if let Some((tab, delegated)) = focused_tab {
                self.reveal_remote_agent_tab(&key, &tab, delegated);
            } else {
                self.choose_when_in_front(&target_id, &dispatched_request_id, key);
                if terminal_intent {
                    self.await_terminal_choice(&target_id, &dispatched_request_id);
                }
            }
        }
        if focus_device && !self.device_in_front(&target_id) {
            self.bring_device_forward(target_id.clone());
        }
        true
    }

    /// The Workspace a device focus request lands on: the checkout holding
    /// the pane or tab it names.
    fn remote_request_workspace_key(
        &self,
        target_id: &str,
        session: &RemoteSessionSnapshot,
        request: &RemoteControlRequest,
    ) -> Option<workspace_view::WorkspaceKey> {
        let holds = |checkout: &CheckoutSnapshot| match request {
            RemoteControlRequest::FocusPane { pane_id } => checkout
                .tabs
                .iter()
                .any(|tab| tab.panes.iter().any(|pane| &pane.id == pane_id)),
            RemoteControlRequest::FocusTab { tab_id } => checkout
                .tabs
                .iter()
                .any(|tab| tab.id.as_deref() == Some(tab_id.as_str())),
            _ => false,
        };
        let (workspace, checkout) = session.workspaces.iter().find_map(|workspace| {
            workspace
                .checkouts
                .iter()
                .find(|checkout| holds(checkout))
                .map(|checkout| (workspace, checkout))
        })?;
        Some(
            self.workspace_key(&workspace.id, &checkout.id)
                .unwrap_or_else(|| (target_id.to_owned(), checkout.path.clone())),
        )
    }

    pub fn ingest_usage(&mut self, usage: Vec<crate::usage::model::AgentUsage>) -> bool {
        let mut changed = false;
        for reading in usage {
            if let Some(row) = self.snapshot.usage.nodes[0]
                .agents
                .iter_mut()
                .find(|row| row.agent == reading.agent)
                && row.limits != reading.limits
            {
                row.limits = reading.limits;
                changed = true;
            }
        }
        if !changed {
            return false;
        }
        self.usage_edit += 1;
        if self.snapshot.ui_state.usage.reconcile(&self.snapshot.usage) {
            self.persist_ui_state();
        }
        self.refresh_background_ai();
        true
    }

    pub(crate) fn usage_activity(&self) -> crate::usage::UsageActivity {
        crate::usage::UsageActivity {
            // Background Memory analysis reads the weekly figure with no
            // window attached (D-10 b).
            window_visible: self.usage_window_visible
                || self.memory_reads_usage()
                || self.limit_reads_usage(),
            popover_open_generation: self.usage_popover_open_generation,
            hidden: self.snapshot.ui_state.usage.hidden.clone(),
            required: if self.memory_reads_usage() || self.limit_reads_usage() {
                self.ai_settings
                    .as_ref()
                    .filter(|settings| settings.enabled && settings.chosen)
                    .map(|settings| {
                        // The usage rows are keyed by the agent's Herdr name,
                        // which is not the provider id for every agent (agy
                        // runs the `antigravity` provider).
                        settings
                            .router_config()
                            .priority
                            .iter()
                            .filter_map(|provider| {
                                hide_agent_adapter::adapter(provider.descriptor().agent)
                            })
                            .map(|row| row.herdr.name.to_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        }
    }

    /// Recomputes the pet's pose, badge row, and attention queue from the
    /// current agent list and connection state. Idempotent: the same inputs
    /// produce the same snapshot and report no change.
    pub(super) fn refresh_pet(&mut self) -> bool {
        let now = unix_milliseconds();
        let connected = self.snapshot.status.herdr.state == "connected";
        let agents = &self.snapshot.navigator.agents;
        let summary = crate::agent_state::summarize(agents, connected);
        if summary.needs_you + summary.working > 0 {
            self.pet_active_at_unix_ms = now;
            self.agent_worked_at_unix_ms = now;
        }
        let idle_ms = now.saturating_sub(self.pet_active_at_unix_ms);
        let waking = now < self.pet_waking_until_unix_ms;
        let subagents_active =
            crate::agent_state::subagents_active(agents, &self.pane_hook_tokens, connected);
        let attention_pane_ids = if connected {
            crate::agent_state::observe_unseen(&mut self.pet_unseen_observed, agents, now);
            crate::agent_state::attention_order(
                &self.snapshot.navigator.agents,
                &self.pet_unseen_observed,
            )
        } else {
            Vec::new()
        };

        let next = PetSnapshot {
            visible: self.snapshot.ui_state.pet_visible,
            connection: self.snapshot.status.herdr.state.clone(),
            connection_message: self.snapshot.status.herdr.message.clone(),
            pose: pet::pose(summary, idle_ms, waking, connected).to_owned(),
            sleep_phase: pet::sleep_phase_for_idle_ms(idle_ms).as_str().to_owned(),
            roam_allowed: connected && pet::is_roam_allowed(summary, idle_ms, self.pet_dragging),
            badges: PetBadgesSnapshot {
                needs_you: summary.needs_you,
                done: summary.done,
                working: summary.working,
                seen: summary.seen,
                disconnected: summary.disconnected,
                subagents_active,
            },
            attention_pane_ids,
            origin: self.snapshot.ui_state.pet_origin,
            shortcut: self.snapshot.ui_state.pet_shortcut.clone(),
            shortcut_error: self.snapshot.pet.shortcut_error.clone(),
            theme_id: self.snapshot.pet.theme_id.clone(),
        };
        if self.snapshot.pet == next {
            return false;
        }
        self.snapshot.pet = next;
        true
    }

    /// Applying the same visibility twice converges instead of flapping, so
    /// four surfaces sharing one state can all set it freely.
    pub(super) fn set_pet_visible(&mut self, visible: bool) -> bool {
        if self.snapshot.ui_state.pet_visible == visible {
            return false;
        }
        self.snapshot.ui_state.pet_visible = visible;
        self.persist_ui_state();
        self.note_pet_activity();
        self.refresh_pet();
        true
    }

    /// Pointer or toggle activity wakes a sleeping pet before the normal
    /// priority resumes.
    pub(super) fn note_pet_activity(&mut self) {
        let now = unix_milliseconds();
        let idle_ms = now.saturating_sub(self.pet_active_at_unix_ms);
        if pet::sleep_phase_for_idle_ms(idle_ms) != pet::SleepPhase::Awake {
            self.pet_waking_until_unix_ms = now.saturating_add(PET_WAKING_MS);
        }
        self.pet_active_at_unix_ms = now;
    }

    pub(super) fn apply_persisted_pet_state(&mut self) {
        self.snapshot.pet.visible = self.snapshot.ui_state.pet_visible;
        self.snapshot.pet.origin = self.snapshot.ui_state.pet_origin;
        self.snapshot.pet.shortcut = self.snapshot.ui_state.pet_shortcut.clone();
    }

    /// Starts the read axis's view of a new connection to one Herdr server:
    /// a mark no earlier connection wrote, and no pane settled yet. Every
    /// connect starts one, because a reconnect cannot tell whether the server
    /// it reaches is the one it lost; the panes' marks say that
    /// (`agent_state::ReadConnection`).
    pub(crate) fn begin_read_connection(&mut self, target_id: Option<&str>) {
        let connection =
            crate::agent_state::ReadConnection::new(super::read_mark(self.read_marks_made));
        self.read_marks_made = self.read_marks_made.saturating_add(1);
        crate::diagnostic!(serde_json::json!({
            "component": "read_state",
            "kind": "connection.started",
            "target": target_id.unwrap_or("local"),
            "mark": connection.mark,
        }));
        match target_id {
            None => self.local_read_connection = connection,
            Some(target_id) => {
                self.remote_read_connections
                    .insert(target_id.to_owned(), connection);
            }
        }
    }

    /// The connection the read axis judges one target's panes on.
    pub(super) fn read_connection(
        &mut self,
        target_id: Option<&str>,
    ) -> &mut crate::agent_state::ReadConnection {
        match target_id {
            None => &mut self.local_read_connection,
            Some(target_id) => {
                if !self.remote_read_connections.contains_key(target_id) {
                    self.begin_read_connection(Some(target_id));
                }
                self.remote_read_connections
                    .get_mut(target_id)
                    .expect("inserted above")
            }
        }
    }

    /// The panes on one target's server that should carry this connection's
    /// mark, by Herdr's own pane id: those whose saved read record or
    /// completion names it. The lineage writer writes the ones that do not.
    pub(crate) fn wanted_server_marks(&self, target_id: Option<&str>) -> BTreeMap<String, String> {
        let (connection, agents, prefix) = match target_id {
            None => (
                &self.local_read_connection,
                self.snapshot.navigator.agents.as_slice(),
                None,
            ),
            Some(target_id) => {
                let Some(connection) = self.remote_read_connections.get(target_id) else {
                    return BTreeMap::new();
                };
                let Some(session) = self
                    .snapshot
                    .status
                    .remote
                    .iter()
                    .find(|remote| remote.target_id == target_id)
                    .and_then(|remote| remote.session.as_ref())
                else {
                    return BTreeMap::new();
                };
                (
                    connection,
                    session.agents.as_slice(),
                    Some(remote_pane_id_prefix(target_id)),
                )
            }
        };
        let names = |server: &Option<String>| server.as_deref() == Some(connection.mark.as_str());
        let ui_state = &self.snapshot.ui_state;
        agents
            .iter()
            .filter(|agent| {
                ui_state
                    .pane_read_records
                    .get(&agent.pane_id)
                    .is_some_and(|record| names(&record.server))
                    || ui_state
                        .pane_completions
                        .get(&agent.pane_id)
                        .is_some_and(|completion| names(&completion.server))
            })
            .filter_map(|agent| {
                let pane_id = match &prefix {
                    None => agent.pane_id.as_str(),
                    Some(prefix) => agent.pane_id.strip_prefix(prefix.as_str())?,
                };
                Some((pane_id.to_owned(), connection.mark.clone()))
            })
            .collect()
    }

    /// Shows each finished pane's completion through a server that no
    /// longer reports it, before the lineage hands completions up, and saves
    /// the completions when they moved (`agent_state::apply_completions`).
    pub(super) fn apply_pane_completions(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        target_id: Option<&str>,
        scope: ReadRecordScope<'_>,
        live_pane_ids: Option<&HashSet<String>>,
    ) {
        let mut completions = std::mem::take(&mut self.snapshot.ui_state.pane_completions);
        let mut changed = crate::agent_state::apply_completions(
            agents,
            &mut completions,
            self.read_connection(target_id),
        );
        if let Some(live_pane_ids) = live_pane_ids {
            changed |=
                crate::agent_state::prune_completions(&mut completions, live_pane_ids, scope);
        }
        self.snapshot.ui_state.pane_completions = completions;
        if changed {
            self.persist_ui_state();
        }
    }

    /// Raises the operator-focused pane's read record and sets the read axis
    /// on every row, then persists the record when it actually moved.
    ///
    /// This is the only place the read axis is decided, and the pane it reads
    /// is the one the operator chose, not the one Herdr reports focused.
    /// Herdr marks every pane in a tab seen the moment the tab is focused, so
    /// three finished agents side by side would clear together; Hide keeps its
    /// own pane-level record instead and never derives unread from Herdr's
    /// `done` or `idle`, nor from a focus it merely inherited.
    ///
    /// The record moves on a real state change or an operator focus, not on
    /// every tick, so the save this triggers is not a per-tick disk write.
    pub(super) fn apply_pane_read_state(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        scope: ReadRecordScope<'_>,
        live_pane_ids: Option<&HashSet<String>>,
    ) -> bool {
        let focused = self.operator_focused_pane_id.clone();
        let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
        let mut changes = Vec::new();
        if let Some(live_pane_ids) = live_pane_ids {
            changes.extend(crate::agent_state::prune_read_records(
                &mut records,
                live_pane_ids,
                scope,
            ));
            self.local_read_connection
                .settled
                .retain(|pane_id| live_pane_ids.contains(pane_id));
        }
        changes.extend(crate::agent_state::apply_read_state(
            agents,
            &mut records,
            &mut self.local_read_connection,
            focused.as_deref(),
        ));
        self.snapshot.ui_state.pane_read_records = records;
        let synced = self.sync_pane_status_from_agents(agents);
        let pruned = prune_pane_text_scales(
            &mut self.snapshot.ui_state.pane_text_scales,
            &self.snapshot.navigator.workspaces,
            agents,
            scope,
        );
        if changes.is_empty() && !pruned {
            return synced;
        }
        self.record_read_record_changes(&changes);
        true
    }

    /// Re-runs the read axis over the agents already in the snapshot, for the
    /// moment the operator picks a pane without a new agent list arriving.
    pub(super) fn refresh_pane_read_state(&mut self) -> bool {
        let before = self.snapshot.navigator.agents.clone();
        let mut agents = std::mem::take(&mut self.snapshot.navigator.agents);
        self.apply_pane_read_state(&mut agents, ReadRecordScope::Retain, None);
        let changed = before != agents;
        self.snapshot.navigator.agents = agents;
        changed | self.refresh_inactive_groups() | self.sync_request_rows()
    }

    /// Applies the read axis to one remote target's agent rows.
    ///
    /// A pane is a pane: a remote row earns its read record the same way a
    /// local one does, from the focus its own server reports, because Hide
    /// never focuses a remote pane itself. Eviction is scoped to this target's
    /// pane id prefix, so a local sync cannot drop what this pass wrote and
    /// this pass cannot drop another target's records.
    ///
    /// The remote pane tree arrives freshly projected on every sync, with no
    /// read axis applied, so it is synced whether or not the ledger moved.
    pub(super) fn apply_remote_read_state(
        &mut self,
        target_id: &str,
        session: &mut RemoteSessionSnapshot,
    ) -> bool {
        let focused = session.focused_pane_id.clone().filter(|pane| {
            if self.workspace_views.is_none() {
                return true;
            }
            if !self.device_in_front(target_id) {
                return false;
            }
            self.node_shown_agent_tabs(target_id).is_none_or(|shown| {
                session
                    .workspaces
                    .iter()
                    .flat_map(|workspace| &workspace.checkouts)
                    .flat_map(|checkout| &checkout.tabs)
                    .any(|tab| {
                        tab.id.as_ref().is_some_and(|id| shown.contains(id))
                            && tab.panes.iter().any(|entry| &entry.id == pane)
                    })
            })
        });
        let prefix = remote_pane_id_prefix(target_id);
        let live_pane_ids = session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .map(|pane| pane.id.clone())
            .collect::<HashSet<_>>();
        // The lineage comes first because the read fingerprint carries what
        // each row's descendants are doing, and that is only known once the
        // tree is built (PRD B5).
        let lineage_pruned = crate::agent_state::prune_lineage_expansion(
            &mut self.snapshot.ui_state.expanded_agent_pane_ids,
            &session.agents,
            ReadRecordScope::Remote(&prefix),
        ) | crate::agent_state::prune_lineage_expansion(
            &mut self.snapshot.ui_state.sessions_expanded_agent_pane_ids,
            &session.agents,
            ReadRecordScope::Remote(&prefix),
        );
        self.apply_pane_completions(
            &mut session.agents,
            Some(target_id),
            ReadRecordScope::Remote(&prefix),
            Some(&live_pane_ids),
        );
        self.apply_pane_launches(
            &session.agents,
            ReadRecordScope::Remote(&prefix),
            Some(&live_pane_ids),
        );
        self.lay_start_prompts(
            &mut session.agents,
            ReadRecordScope::Remote(&prefix),
            Some(&live_pane_ids),
        );
        crate::agent_state::apply_lineage(
            &mut session.agents,
            &session.workspaces,
            &self.snapshot.ui_state.expanded_agent_pane_ids,
        );
        let mut records = std::mem::take(&mut self.snapshot.ui_state.pane_read_records);
        let mut changes = crate::agent_state::prune_read_records(
            &mut records,
            &live_pane_ids,
            ReadRecordScope::Remote(&prefix),
        );
        let connection = self.read_connection(Some(target_id));
        connection
            .settled
            .retain(|pane_id| live_pane_ids.contains(pane_id));
        changes.extend(crate::agent_state::apply_read_state(
            &mut session.agents,
            &mut records,
            connection,
            focused.as_deref(),
        ));
        self.snapshot.ui_state.pane_read_records = records;
        if lineage_pruned {
            self.persist_ui_state();
        }
        sync_pane_status(
            &mut session.workspaces,
            &session.agents,
            session.focused_pane_id.as_deref(),
            self.sleep_machine(target_id),
        );
        sync_remote_pane_relations(session);
        let pruned = prune_pane_text_scales(
            &mut self.snapshot.ui_state.pane_text_scales,
            &session.workspaces,
            &session.agents,
            ReadRecordScope::Remote(&prefix),
        );
        if changes.is_empty() && !pruned && !lineage_pruned {
            // These copies were just projected from Herdr, not the previous
            // published snapshot. The caller compares the completed session;
            // applying read/status fields alone is not a new runtime change.
            return false;
        }
        self.record_read_record_changes(&changes);
        true
    }

    /// Logs each read record move and saves the ledger.
    ///
    /// The record moves on a real state change, a focus move, or a pane going
    /// away, not on every tick, so the save this triggers is not a per-tick
    /// disk write.
    pub(super) fn record_read_record_changes(
        &mut self,
        changes: &[crate::agent_state::ReadRecordChange],
    ) {
        for change in changes {
            crate::diagnostic!(serde_json::json!({
                "component": "session",
                "kind": "pane.read_record",
                "pane_id": change.pane_id,
                "evicted": change.evicted,
                "state_change_seq": change.record.state_change_seq,
                "demand": change.record.demand,
                "activity": change.record.activity,
                "server": change.record.server,
                "unread": change.record.unread,
            }));
        }
        self.persist_ui_state();
    }

    /// Copies each local pane's status word and close-confirmation answer from
    /// the agent rows that just had the read axis applied.
    pub(super) fn sync_pane_status_from_agents(&mut self, agents: &[SidebarAgentSnapshot]) -> bool {
        sync_pane_status(
            &mut self.snapshot.navigator.workspaces,
            agents,
            self.snapshot.focused.pane_id.as_deref(),
            crate::agent_sleep::SleepMachine::Core,
        )
    }

    /// A sleeping project the operator opened stays open only until one of
    /// its agents wakes or starts; the next time it falls asleep it starts
    /// folded again (PRD sidebar-titlebar-refresh D-19). Only an awake agent
    /// row ends it: a key whose project is not listed, or is listed with no
    /// agent row at all, is kept, as a reconnect can publish no rows for a
    /// moment. Returns whether a key left, after asking for the save.
    fn forget_woken_asleep_folds(&mut self) -> bool {
        if !self
            .snapshot
            .ui_state
            .session_open_folds
            .iter()
            .any(|key| key.starts_with("asleep/"))
        {
            return false;
        }
        let by_pane = self.agent_rows_by_pane();
        let woken: HashSet<String> = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.workspaces),
            )
            .filter(|workspace| {
                let (presence, has_rows) = crate::project_context::agent_presence(
                    workspace,
                    &by_pane,
                    &self.snapshot.navigator.sleeping_sessions,
                    &[],
                );
                // Rows and none asleep: an agent there is awake.
                has_rows && presence.asleep.is_none()
            })
            .map(|workspace| crate::project_context::asleep_fold_key(&workspace.id))
            .collect();
        drop(by_pane);
        let folds = &mut self.snapshot.ui_state.session_open_folds;
        let before = folds.len();
        folds.retain(|key| !woken.contains(key));
        if folds.len() == before {
            return false;
        }
        self.persist_ui_state();
        true
    }

    /// Every agent row the core lists, here and on each device session, by pane.
    fn agent_rows_by_pane(&self) -> HashMap<&str, &SidebarAgentSnapshot> {
        self.snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.agents),
            )
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect()
    }

    /// Recomputes the two inactive folds from current core facts. No row is
    /// moved or copied: the full collections stay authoritative for search,
    /// focus, and non-sidebar consumers.
    pub(super) fn refresh_inactive_groups(&mut self) -> bool {
        // A bootstrap or reconnect can temporarily publish no catalog rows.
        // Like recent checkouts, remembered folds leave only on explicit removal.
        let mut changed = self.forget_woken_asleep_folds();
        changed |= crate::project_context::refresh_inactive_groups(
            &mut self.snapshot.navigator,
            &self.snapshot.ui_state,
            unix_milliseconds(),
        );
        // The time that moves a checkout under Inactive reaches device
        // sessions too, through the fold their derive uses.
        let nodes: BTreeSet<String> = self
            .node_sessions()
            .map(|(node, _)| node.to_owned())
            .collect();
        let now = unix_milliseconds();
        let window = self.snapshot.navigator.focused_checkout_id.clone();
        for remote in &mut self.snapshot.status.remote {
            let node = nodes.contains(&remote.target_id);
            if let Some(session) = &mut remote.session {
                let agents = session
                    .agents
                    .iter()
                    .map(|row| (row.pane_id.as_str(), row))
                    .collect();
                let focused = crate::project_context::device_fold_focus(window.as_deref(), session);
                for workspace in &mut session.workspaces {
                    let before = (
                        workspace.session_folds.clone(),
                        workspace.inactive_checkouts.clone(),
                        workspace.agent_presence.clone(),
                    );
                    crate::project_context::refresh_device_folds(
                        workspace,
                        &agents,
                        &self.snapshot.navigator.sleeping_sessions,
                        &self.snapshot.ui_state,
                        focused.as_deref(),
                        node,
                        now,
                    );
                    changed |= workspace.session_folds != before.0
                        || workspace.inactive_checkouts != before.1
                        || workspace.agent_presence != before.2;
                }
            }
        }
        changed | self.refresh_agent_scopes()
    }

    /// Drops the conversation choice of every pane that is no longer an
    /// eligible agent pane. Nothing is added here: a pane shows its terminal
    /// until the operator asks for the conversation, so the set only ever
    /// grows through `toggle_conversation`.
    pub(super) fn sync_conversation_modes(&mut self, agents: &[SidebarAgentSnapshot]) -> bool {
        let live: BTreeSet<String> = agents
            .iter()
            .filter(|agent| conversation_agent_kind(&agent.agent_kind))
            .map(|agent| agent.pane_id.clone())
            .collect();
        let conversation = &mut self.snapshot.ui_state.conversation_pane_ids;
        let before = conversation.len();
        conversation.retain(|pane_id| live.contains(pane_id));
        before != conversation.len()
    }

    /// Refills every pane's child summary and breadcrumb from the final agent
    /// list.
    ///
    /// It runs after the read axis and the lineage, not while the panes are
    /// built: a chip's mark and emphasis come from the group, the group comes
    /// from the read axis, and the children come from the lineage, so a pass
    /// that ran earlier would publish a chip row describing a state the
    /// sidebar had already moved past.
    pub(super) fn sync_pane_lineage(&mut self) -> bool {
        let agents = std::mem::take(&mut self.snapshot.navigator.agents);
        let diagnosis = self.hook_diagnosis.clone();
        let local_kit = self.kit_states.get(self.node.as_str());
        // Claude Code's and Codex's hooks are read from their files; OpenCode's
        // plugin and Grok's and Cursor's hooks from this Mac's kit, the one
        // place that judges them, read here once rather than holding the kit
        // across the pass.
        let from_kit: Vec<_> = hide_agent_adapter::HookDialect::ALL
            .into_iter()
            .filter(|dialect| hide_agent_hooks::AgentRuntime::from_dialect(*dialect).is_none())
            .map(|dialect| {
                let status =
                    local_kit.and_then(|kit| crate::agent_hooks::kit_hook_status(kit, dialect));
                (dialect, status)
            })
            .collect();
        let codex_daemon_on = local_kit.is_some_and(crate::model::KitSnapshot::shares_codex_server);
        let status_of = |dialect: hide_agent_adapter::HookDialect| {
            match hide_agent_hooks::AgentRuntime::from_dialect(dialect) {
                Some(runtime) => diagnosis
                    .as_ref()
                    .and_then(|diagnosis| diagnosis.status_of(runtime))
                    .cloned(),
                None => from_kit
                    .iter()
                    .find(|(known, _)| *known == dialect)
                    .and_then(|(_, status)| status.clone()),
            }
        };
        let mut changed = false;
        let mut reopen_scope = ReopenScope::default();
        let mut delegated_tabs_changed = false;
        // A tab is the operator's whenever it holds an agent they own. One
        // holding only delegated children is the pile this change exists to
        // take off the strip (PRD B1).
        for tab in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.checkouts.iter_mut())
            .flat_map(|checkout| checkout.tabs.iter_mut())
        {
            let delegated = crate::agent_state::tab_is_delegated(&tab.panes, &agents);
            if tab.delegated != delegated {
                tab.delegated = delegated;
                delegated_tabs_changed = true;
            }
        }
        for pane in self
            .snapshot
            .navigator
            .workspaces
            .iter_mut()
            .flat_map(|workspace| workspace.checkouts.iter_mut())
            .flat_map(|checkout| checkout.tabs.iter_mut())
            .flat_map(|tab| tab.panes.iter_mut())
        {
            // A remote pane's answer is fixed and was decided where it was
            // projected; the local hook state says nothing about it.
            if crate::agent_hooks::is_remote_pane(&pane.id) {
                continue;
            }
            let tokens = self
                .pane_hook_tokens
                .get(&pane.id)
                .copied()
                .unwrap_or_default();
            let mut children = crate::sidebar::project_pane_children_connected(
                &agents,
                &pane.id,
                tokens,
                &status_of,
                codex_daemon_on,
            );
            reopen_scope.panes.insert(pane.id.clone());
            match children
                .as_mut()
                .and_then(|children| children.connection.as_mut())
            {
                Some(connection) if connection.connected => {
                    reopen_scope.connected.insert(pane.id.clone());
                }
                Some(connection) => {
                    connection.reopen = self.pane_reopens.get(&pane.id).copied();
                    reopen_scope.not_connected.insert(pane.id.clone());
                }
                None => {}
            }
            let lineage_path = crate::sidebar::project_lineage_path(&agents, &pane.id);
            // The fork's source is named once the lineage pass has read it.
            let forked_from = agents
                .iter()
                .find(|agent| agent.pane_id == pane.id)
                .and_then(|agent| agent.forked_from.clone());
            if pane.fork.forked_from != forked_from {
                pane.fork.forked_from = forked_from;
                changed = true;
            }
            if pane.children != children {
                pane.children = children;
                changed = true;
            }
            if pane.lineage_path != lineage_path {
                pane.lineage_path = lineage_path;
                changed = true;
            }
        }
        self.snapshot.navigator.agents = agents;
        self.pane_reopens.retain(|pane_id, state| {
            !reopen_scope.connected.contains(pane_id)
                && (reopen_scope.not_connected.contains(pane_id)
                    || (matches!(state, crate::model::PaneReopenSnapshot::Pending)
                        && reopen_scope.panes.contains(pane_id)))
        });
        let hooks = crate::model::AgentHooksSnapshot {
            last_report_failure: self
                .hook_diagnosis
                .as_ref()
                .and_then(|diagnosis| diagnosis.last_report_failure.as_ref())
                .map(|failure| failure.message()),
        };
        if self.snapshot.status.agent_hooks != hooks {
            self.snapshot.status.agent_hooks = hooks;
            changed = true;
        }
        if delegated_tabs_changed {
            self.rebuild_tab_strips();
            self.sync_agent_lineage_layouts();
        }
        // The lineage decided `delegated` after the read pass named the
        // strip entries, so the chip they carry is refreshed here.
        changed |= sync_strip_agent_identity(
            &mut self.snapshot.navigator.workspaces,
            &self.snapshot.navigator.agents,
        );
        changed |= self.refresh_agent_sessions();
        changed | delegated_tabs_changed | self.refresh_inactive_groups()
    }

    /// Marks the tabs that exist only to hold delegated children, and asks
    /// Herdr to move any child still sharing its parent's tab into one.
    ///
    /// Detection is the same on every pass, so a child that arrives while
    /// Hide is running and a child already split when Hide started are the
    /// same case and take the same path (PRD B1, B3, D-44). Herdr keeps
    /// owning split geometry and the PTY size, so the pane is really moved
    /// rather than merely left undrawn (PRD D-15).
    pub(super) fn relocate_delegated_child_panes(&mut self) -> bool {
        // Where Herdr currently holds each pane. The layout is the only place
        // that carries Herdr's own workspace and tab ids for a pane.
        let placement = self
            .snapshot
            .pane_layouts
            .iter()
            .flat_map(|layout| {
                layout
                    .pane_ids()
                    .into_iter()
                    .map(|pane_id| {
                        (
                            pane_id.to_owned(),
                            (layout.workspace_id.clone(), layout.tab_id.clone()),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeMap<_, _>>();
        let now = unix_milliseconds();
        let mut requests = Vec::new();
        let mut settled = Vec::new();
        // Read from the ledger only once a delegated child reaches the check.
        let mut beside = None;
        for agent in &self.snapshot.navigator.agents {
            if !agent.delegated
                || crate::agent_hooks::is_remote_pane(&agent.pane_id)
                || self.manually_placed_pane(&agent.pane_id)
                || beside
                    .get_or_insert_with(|| self.beside_spawned_panes())
                    .contains(agent.pane_id.as_str())
            {
                continue;
            }
            let Some(parent_pane_id) = agent.lineage_parent_pane_id.as_deref() else {
                continue;
            };
            let (Some((workspace_id, tab_id)), Some((_, parent_tab_id))) =
                (placement.get(&agent.pane_id), placement.get(parent_pane_id))
            else {
                continue;
            };
            if tab_id != parent_tab_id {
                settled.push(agent.pane_id.clone());
                continue;
            }
            if self
                .pane_relocations_in_flight
                .get(&agent.pane_id)
                .is_some_and(|asked| now.saturating_sub(*asked) < RELOCATION_RETRY_INTERVAL_MS)
            {
                continue;
            }
            requests.push((
                agent.pane_id.clone(),
                workspace_id.clone(),
                agent.id.clone(),
            ));
        }
        // A move is done when Herdr's layout shows the child in its own tab,
        // not when Herdr acknowledges the request: the acknowledgement can
        // arrive before that layout, and a pass in between would move the
        // child a second time. A pane Herdr no longer reports can never
        // answer, so its record is dropped rather than held forever.
        self.pane_relocations_in_flight
            .retain(|pane_id, _| placement.contains_key(pane_id) && !settled.contains(pane_id));
        if requests.is_empty() {
            return false;
        }
        let Some(context) = self.live.as_ref().cloned() else {
            return false;
        };
        for (pane_id, workspace_id, label) in requests {
            self.pane_relocations_in_flight.insert(pane_id.clone(), now);
            if let Err(message) = live::spawn_pane_control(
                context.clone(),
                PaneControlAction::MoveToNewTab {
                    pane_id: pane_id.clone(),
                    workspace_id,
                    label,
                },
            ) {
                self.pane_relocations_in_flight.remove(&pane_id);
                self.push_diagnostic(
                    "lineage.relocate_failed",
                    format!("Could not move delegated pane {pane_id}: {message}"),
                );
            }
        }
        true
    }

    /// The panes of this machine's live agents that a spawn opened beside
    /// its caller (`hide agent spawn --beside`), which stay in the caller's
    /// tab. Keyed by the spawn's child record, so a restarted Hide or Herdr
    /// that rebinds the child to its pane keeps it there too.
    fn beside_spawned_panes(&self) -> std::collections::HashSet<&str> {
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return Default::default();
        };
        let children: std::collections::HashSet<&str> = ledger
            .spawns
            .iter()
            .filter(|spawn| spawn.beside && spawn.machine.is_none())
            .filter_map(|spawn| spawn.child.as_deref())
            .collect();
        if children.is_empty() {
            return Default::default();
        }
        ledger
            .agents
            .iter()
            .filter(|agent| {
                !agent.ended
                    && agent.machine == self.node.as_str()
                    && children.contains(agent.id.as_str())
            })
            .map(|agent| agent.pane.as_str())
            .collect()
    }

    /// The operator's provider choice once the settings file has been read.
    pub(crate) fn label_ai_settings(&self) -> Option<hide_ai::AiSettings> {
        self.ai_settings.clone()
    }

    pub(crate) fn install_label_services(
        &mut self,
        services: std::sync::Arc<crate::labels::LabelServices>,
    ) {
        self.label_services = Some(services);
    }

    /// Whether the readers that only feed a window should run.
    pub(crate) fn ui_attached(&self) -> bool {
        self.ui_attached
    }

    /// The pull request creation times the label workers judge against.
    pub(crate) fn pull_request_times(
        &self,
    ) -> std::sync::Arc<crate::labels::facts::PullRequestTimes> {
        std::sync::Arc::clone(&self.pull_request_times)
    }

    /// What a starting session-sync coordinator builds its label worker on.
    pub(crate) fn label_services(&self) -> Option<std::sync::Arc<crate::labels::LabelServices>> {
        self.label_services.clone()
    }

    /// Applies one Background AI settings event.
    ///
    /// The choice takes effect on the snapshot at once, so the control moves
    /// under the operator's hand rather than after a file write; the write
    /// itself is queued for the coordinator, because nothing that touches the
    /// disk runs under this mutex.
    pub(super) fn apply_ai_settings(&mut self, payload: AiSettingsPayload) -> bool {
        let mut changed = false;
        if let Some(observing) = payload.observing
            && self.ai_observing != observing
        {
            self.ai_observing = observing;
            changed = true;
        }
        if let Some(observing) = payload.start_observing
            && self.ai_start_observing != observing
        {
            self.ai_start_observing = observing;
            changed = true;
        }

        // A model without the provider it belongs to is not applied to
        // whichever provider happens to be selected: the event is refused and
        // says so.
        if payload.provider.is_none() && payload.model.is_some() {
            self.set_error(
                "ai_settings.model_without_provider",
                "A background AI model must name the provider it belongs to",
                false,
            );
            return true;
        }

        let mut next = self.ai_settings.clone().unwrap_or_default();
        if let Some(on) = payload.enabled {
            next.enabled = on;
        }
        if let Some(on) = payload.agent_summary {
            next.agent_summary = on;
        }
        if let Some(limit) = payload.usage_limit {
            next.usage_limit = limit;
        }
        if let Some(id) = payload.provider.as_deref() {
            let Some(provider) = self.ai_provider_named(id) else {
                return true;
            };
            match payload.model {
                // A model names the agent it is for: Runs on, or an agent in
                // the fallback list, which keeps its own model. Choosing a
                // model for the agent that is already selected is also
                // choosing it, which is what a first choice by model means.
                Some(model) => {
                    if next.fallback.iter().any(|entry| entry.provider == provider) {
                        next.set_fallback_model(provider, Some(model));
                    } else {
                        if provider == next.provider && !next.chosen {
                            next.set_provider(provider);
                        }
                        next.set_model(provider, model);
                    }
                }
                None => {
                    if provider != next.provider || !next.chosen {
                        if !self.ai_selectable(provider) {
                            self.refuse_unselectable(provider);
                            return true;
                        }
                        next.set_provider(provider);
                    }
                }
            }
        }
        if let Some(id) = payload.fallback_add.as_deref() {
            let Some(provider) = self.ai_provider_named(id) else {
                return true;
            };
            if !self.ai_selectable(provider) {
                self.refuse_unselectable(provider);
                return true;
            }
            if let Err(refusal) = next.add_fallback(provider, None) {
                let (kind, message) = match refusal {
                    hide_ai::FallbackRefusal::IsRunsOn => (
                        "ai_settings.fallback_is_runs_on",
                        "The agent Hide AI runs on is not its own fallback",
                    ),
                    hide_ai::FallbackRefusal::AlreadyListed => (
                        "ai_settings.fallback_listed",
                        "That agent is already in the fallback list",
                    ),
                };
                self.set_error(kind, message, false);
                return true;
            }
        }
        if let Some(id) = payload.fallback_remove.as_deref() {
            let Some(provider) = self.ai_provider_named(id) else {
                return true;
            };
            next.remove_fallback(provider);
        }
        if self.ai_settings.as_ref() != Some(&next) {
            self.ai_settings = Some(next.clone());
            self.pending_ai_settings_save = Some(next);
            changed = true;
        }

        if changed {
            if !self.ai_settings.as_ref().is_some_and(|settings| {
                settings.enabled && settings.chosen && settings.agent_summary
            }) {
                self.drop_missed_backlog();
            }
            self.refresh_background_ai();
        }
        true
    }

    /// Whether agent labels are asked of a provider and shown (D-11): the
    /// switch is on, Hide AI is on and an agent is chosen to run on (B33,
    /// B47). A turn that is not asked is not lost: the row shows the session's
    /// own text and the label is made once this is true again.
    pub(crate) fn agent_summary(&self) -> bool {
        self.ai_settings
            .as_ref()
            .is_some_and(|settings| settings.agent_summary && self.ai_active())
            // The pause as the last refresh found it: this runs every
            // coordinator tick, and the refresh runs on each usage read,
            // settings change and wake that can move it.
            && !self.usage_pause_announced
    }

    /// Whether the label worker asks a provider now: agent summaries are on
    /// and the core does not wait for its move's link (`effects.rs`), which
    /// reads as the switch off until then.
    pub(crate) fn labels_asked(&self) -> bool {
        self.agent_summary() && !self.effects_held()
    }

    /// Whether Hide makes any model call: Use Hide AI is on and an agent is
    /// chosen (B33, B47). Worktree names, Memory analysis and agent labels
    /// each fall back to their own non-AI behavior while this is false.
    pub(crate) fn ai_active(&self) -> bool {
        self.ai_settings
            .as_ref()
            .is_some_and(|settings| settings.enabled && settings.chosen)
    }

    /// The agent a first-run or `provider` event names, or the refusal that
    /// says Hide has no such agent.
    fn ai_provider_named(&mut self, id: &str) -> Option<hide_ai::ProviderId> {
        let provider = hide_ai::ProviderId::from_id(id);
        if provider.is_none() {
            self.set_error(
                "ai_settings.unknown_provider",
                format!("Hide has no background AI provider called {id}"),
                false,
            );
        }
        provider
    }

    /// Whether the last read says `provider` can be chosen (D-15): its CLI
    /// is installed, it is signed in, it has a backend and its call cannot
    /// change files. A provider not read yet cannot.
    fn ai_selectable(&self, provider: hide_ai::ProviderId) -> bool {
        self.background_ai_providers
            .iter()
            .any(|row| row.id == provider.as_str() && row.selectable)
    }

    fn refuse_unselectable(&mut self, provider: hide_ai::ProviderId) {
        self.set_error(
            "ai_settings.provider_not_selectable",
            format!(
                "{} cannot be chosen for Hide AI: it is not installed, not signed in, or cannot be asked without risk to files",
                provider.label()
            ),
            false,
        );
    }

    /// What the provider probe should ask, and whether it should ask at all.
    ///
    /// An empty answer while the tab is off screen and an agent is chosen is
    /// intentional, the same way an empty disk request is: an idle Hide must
    /// never start a provider process. With nobody chosen it asks only the
    /// agents that are switched on, for their sign-in, so the first one that
    /// is signed in can be chosen by itself (D-18, D-27).
    pub fn ai_request(&self) -> crate::ai::AiRequest {
        let settings = self.ai_settings.clone().unwrap_or_default();
        let selecting = if self.ai_settings.is_some() && settings.enabled && !settings.chosen {
            self.kit_state(self.node.as_str())
                .agents
                .iter()
                .filter(|row| {
                    row.enabled && !matches!(row.availability, hide_kit::Availability::NotInstalled)
                })
                .filter_map(|row| {
                    hide_ai::PROVIDERS
                        .iter()
                        .find(|provider| provider.descriptor().agent == row.id)
                        .copied()
                })
                .collect()
        } else {
            std::collections::BTreeSet::new()
        };
        let kit = self.kit_state(self.node.as_str());
        // The kit's own answer to "is this agent's program on this Mac"; none
        // until it has read, so no agent is called missing from no reading.
        let cli_found = (!kit.agents.is_empty()).then(|| {
            kit.agents
                .iter()
                .filter(|row| !matches!(row.availability, hide_kit::Availability::NotInstalled))
                .filter_map(|row| {
                    hide_ai::PROVIDERS
                        .iter()
                        .find(|provider| provider.descriptor().agent == row.id)
                        .copied()
                })
                .collect()
        });
        crate::ai::AiRequest {
            observing: self.ai_observing
                || self.ai_start_observing
                || self.ai_catalog_read_pending(),
            models: settings.models_by_provider(),
            selecting,
            cli_found,
        }
    }

    /// Whether `hide agent catalog` is waiting for one read of the model
    /// lists: asked, and no read has landed since. It observes the providers
    /// as the start panel does, and only until that read.
    pub(crate) fn ai_catalog_read_pending(&self) -> bool {
        self.ai_catalog_read_asked == Some(self.ai_reads)
    }

    /// Asks for one read of the model lists for the catalog, unless one is
    /// already on its way.
    pub(crate) fn ask_ai_catalog_read(&mut self) {
        if !self.ai_catalog_read_pending() {
            self.ai_catalog_read_asked = Some(self.ai_reads);
        }
    }

    /// Whether the Settings agents tab is on screen. The Background AI group
    /// reports its own appearance through `ai_settings.observing`, and the
    /// hook diagnosis shares that tab, so the one flag answers for both
    /// readers that only work while the operator is looking.
    pub(crate) fn settings_observed(&self) -> bool {
        self.ai_observing
    }

    /// Hands a queued settings write to the caller that can perform it.
    pub(crate) fn take_ai_settings_save(&mut self) -> Option<hide_ai::AiSettings> {
        self.pending_ai_settings_save.take()
    }

    /// Stores the choice the coordinator read from the settings file, and
    /// whether reading it failed.
    ///
    /// A failed read is not taken as the defaults in silence: the defaults
    /// are used and the reason travels to the screen with them.
    pub(crate) fn ingest_ai_settings(
        &mut self,
        settings: hide_ai::AiSettings,
        unavailable_reason: Option<String>,
    ) -> bool {
        let same = self.ai_settings.as_ref() == Some(&settings)
            && self.snapshot.status.background_ai.unavailable_reason == unavailable_reason;
        if same {
            return false;
        }
        self.ai_settings = Some(settings);
        self.snapshot.status.background_ai.unavailable_reason = unavailable_reason;
        self.refresh_background_ai();
        true
    }

    /// Stores what the providers answered.
    pub(crate) fn ingest_background_ai(
        &mut self,
        read: crate::model::BackgroundAiSnapshot,
    ) -> bool {
        // Counted before anything else, so a read that changed nothing still
        // ends the catalog's wait for it.
        self.ai_reads = self.ai_reads.wrapping_add(1);
        // A read that asked only the agents being selected from knows
        // nothing of the others: their rows stay as they were.
        let mut providers = if self.background_ai_providers.is_empty() {
            crate::model::BackgroundAiSnapshot::unread().providers
        } else {
            self.background_ai_providers.clone()
        };
        for row in read.providers {
            if row.state != "unread"
                && let Some(old) = providers.iter_mut().find(|old| old.id == row.id)
            {
                *old = row;
            }
        }
        let selected = self.choose_first_run_provider(&providers);
        if self.background_ai_providers == providers && !selected {
            return false;
        }
        self.background_ai_providers = providers;
        self.refresh_background_ai();
        true
    }

    /// The first run's choice (D-18, D-27): while nobody has chosen, the first
    /// agent of the fixed order that is switched on in this Mac's kit and
    /// signed in becomes Runs on, with its default model, and the choice is
    /// stored. A stored choice is never replaced, and with no agent signed in
    /// nothing is chosen and Hide features run without a model (B47).
    fn choose_first_run_provider(
        &mut self,
        providers: &[crate::model::BackgroundAiProviderSnapshot],
    ) -> bool {
        let Some(settings) = self.ai_settings.as_ref() else {
            return false;
        };
        if settings.chosen || !settings.enabled {
            return false;
        }
        let on: Vec<String> = self
            .kit_state(self.node.as_str())
            .agents
            .iter()
            .filter(|row| row.enabled)
            .map(|row| row.id.clone())
            .collect();
        let ready: Vec<(hide_ai::ProviderId, hide_ai::Availability)> = providers
            .iter()
            .filter(|row| row.state == "ready")
            .filter_map(|row| {
                let provider = hide_ai::ProviderId::from_id(&row.id)?;
                on.iter()
                    .any(|agent| agent == provider.descriptor().agent)
                    .then_some((provider, hide_ai::Availability::Ready))
            })
            .collect();
        let Some(provider) = hide_ai::AiSettings::provider_for_first_run(&ready) else {
            return false;
        };
        let mut next = settings.clone();
        next.set_provider(provider);
        self.ai_settings = Some(next.clone());
        self.pending_ai_settings_save = Some(next);
        true
    }

    /// Reports a settings write that did not happen, so a choice the operator
    /// made and the file on disk cannot silently disagree.
    pub(crate) fn report_ai_settings_failure(&mut self, reason: String) -> bool {
        if self
            .snapshot
            .status
            .background_ai
            .unavailable_reason
            .as_deref()
            == Some(reason.as_str())
        {
            return false;
        }
        self.snapshot.status.background_ai.unavailable_reason = Some(reason);
        true
    }

    /// Rebuilds the Background AI section from the choice and the last
    /// provider answers. The model each row reports is the configured one,
    /// which is what the probe was run with.
    pub(super) fn refresh_background_ai(&mut self) {
        let settings = self.ai_settings.clone().unwrap_or_default();
        self.memory_ai_settings_changed(&settings);
        let mut providers = if self.background_ai_providers.is_empty() {
            crate::model::BackgroundAiSnapshot::unread().providers
        } else {
            self.background_ai_providers.clone()
        };
        let models = settings.models_by_provider();
        for row in &mut providers {
            if let Some(provider) = hide_ai::ProviderId::from_id(&row.id) {
                row.model = models
                    .get(&provider)
                    .cloned()
                    .unwrap_or_else(|| settings.model(provider).to_owned());
            }
        }
        let snapshot = &mut self.snapshot.status.background_ai;
        snapshot.enabled = settings.enabled;
        snapshot.chosen = settings.chosen;
        snapshot.provider = settings
            .chosen
            .then(|| settings.provider.as_str().to_owned());
        snapshot.agent_summary = settings.agent_summary;
        snapshot.fallback = settings
            .fallback
            .iter()
            .map(|entry| crate::model::BackgroundAiFallbackSnapshot {
                provider: entry.provider.as_str().to_owned(),
                model: models.get(&entry.provider).cloned().unwrap_or_default(),
            })
            .collect();
        snapshot.providers = providers;
        // The figures and the refusal read the rows just stored.
        self.refresh_usage_limit();
        let refusal = self.ai_refusal(&settings);
        self.snapshot.status.background_ai.refusal = refusal;
    }

    /// What the label analyzer last found about Runs on (B41, B42): shown
    /// only while it is the chosen agent that cannot answer.
    fn ai_refusal(
        &self,
        settings: &hide_ai::AiSettings,
    ) -> Option<crate::model::BackgroundAiRefusalSnapshot> {
        if !settings.chosen || !settings.enabled {
            return None;
        }
        // Every agent past the limit is what the screen must say, whatever the
        // analyzer last recorded: labels are not asked while it holds, so that
        // record is not refreshed.
        if self.usage_pause_announced
            && let Some(refusal) = self.usage_refusal(settings)
        {
            return Some(refusal);
        }
        let standing = self
            .label_services
            .as_ref()
            .and_then(|services| services.standing.current())
            .filter(|standing| standing.selected == settings.provider);
        let Some(standing) = standing else {
            // Nothing is wrong with Runs on except, maybe, the usage limit.
            return self.usage_refusal(settings);
        };
        // The analyzer's record is as old as its last job: an agent it named
        // as answering that is past the limit now does not answer.
        let using = standing.using.filter(|using| {
            self.usage_gate()
                .past_limit(*using, unix_milliseconds())
                .is_none()
        });
        Some(crate::model::BackgroundAiRefusalSnapshot {
            provider: standing.selected.as_str().to_owned(),
            reason: standing.reason.to_owned(),
            retry_at_ms: standing.retry_at_ms,
            using: using.map(|using| using.as_str().to_owned()),
            used_percent: None,
            limit: None,
        })
    }

    /// Republishes the refusal when the analyzer's standing moved since the
    /// snapshot last said. One mutex read per coordinator wake, no provider.
    pub(crate) fn refresh_ai_standing(&mut self) -> bool {
        let settings = self.ai_settings.clone().unwrap_or_default();
        // A window that reset or a figure that crossed the limit moves these
        // with the clock, not with a usage read.
        let figures = self.refresh_usage_limit();
        let next = self.ai_refusal(&settings);
        if self.snapshot.status.background_ai.refusal == next {
            return figures;
        }
        self.snapshot.status.background_ai.refusal = next;
        true
    }

    pub(crate) fn ingest_hook_diagnosis(
        &mut self,
        mut diagnosis: hide_agent_hooks::Diagnosis,
    ) -> bool {
        // The files cannot say an agent was switched off; this Mac's kit
        // can, so a hook absent on purpose reads Off and not "not installed".
        for runtime in &mut diagnosis.runtimes {
            let agent = runtime.runtime.id();
            let off = self
                .kit_state(self.node.as_str())
                .agents
                .iter()
                .any(|row| row.id == agent && !row.enabled);
            if off && matches!(runtime.status, hide_agent_hooks::HookStatus::NotInstalled) {
                runtime.status = hide_agent_hooks::HookStatus::Off;
            }
        }
        if self.hook_diagnosis.as_ref() == Some(&diagnosis) {
            return false;
        }
        self.hook_diagnosis = Some(diagnosis);
        self.sync_pane_lineage();
        self.enable_memory_after_hook_update();
        self.sync_project_memory();
        true
    }

    /// Steps one text scale by a direction the shell sent, or names the
    /// direction it could not read and answers `None`.
    pub(super) fn stepped_text_scale(&mut self, current: f32, direction: &str) -> Option<f32> {
        match direction {
            "in" => Some(clamp_pane_text_scale(current + PANE_TEXT_SCALE_STEP)),
            "out" => Some(clamp_pane_text_scale(current - PANE_TEXT_SCALE_STEP)),
            "reset" => Some(DEFAULT_PANE_TEXT_SCALE),
            other => {
                self.set_error(
                    "pane.text_scale_unknown_direction",
                    format!("{other} is not a text scale direction; expected in, out, or reset"),
                    true,
                );
                None
            }
        }
    }

    /// Saves the current UI state and surfaces a write failure instead of
    /// dropping it.
    /// Writes the operator's UI state and the pane sizes the next launch
    /// attaches with. The sizes are not part of the UI state the shell draws,
    /// so they are collected here rather than carried on the snapshot.
    pub(super) fn write_ui_state(&mut self) -> Result<(), String> {
        let Some(context) = self.worker_context.clone() else {
            // Standalone runtimes have no shared mutex or worker context.
            let state = self.ui_state_to_save();
            let result = persistence::save(
                &self.state_path,
                &state,
                &self
                    .terminal_sizes
                    .iter()
                    .map(|(id, size)| (id.clone(), *size))
                    .collect(),
            );
            self.acknowledge_session_save(&state, result.is_ok());
            self.ingest_dormant_saved(&state.agent_sleep, result.is_ok());
            self.ingest_archive_saved(&state.session_archive, result.is_ok());
            self.acknowledge_project_settings_save(&state, result.is_ok());
            return result;
        };
        self.state_save_pending = true;
        if self.state_save_active {
            return Ok(());
        }
        self.state_save_active = true;
        match thread::Builder::new()
            .name("hide-state-save".into())
            .spawn(move || {
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                loop {
                    let (path, state, sizes) = {
                        let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
                        if !guard.state_save_pending {
                            guard.state_save_active = false;
                            return;
                        }
                        guard.state_save_pending = false;
                        (
                            guard.state_path.clone(),
                            guard.ui_state_to_save(),
                            guard
                                .terminal_sizes
                                .iter()
                                .map(|(id, size)| (id.clone(), *size))
                                .collect(),
                        )
                    };
                    // The existing save function serializes and writes outside the
                    // runtime mutex. One pending flag coalesces newer UI state.
                    let result = persistence::save(&path, &state, &sizes);
                    let changed = {
                        let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
                        let resolution = state.resolved_sessions.iter().any(|(pane, record)| {
                            guard.pending_session_resolutions.get(pane) == Some(record)
                        });
                        let resolving = resolution
                            || guard.pending_revisit.iter().any(|(pane, pending)| {
                                state.revisit_sessions.get(pane) == pending.as_ref()
                            });
                        let resolved_changed =
                            guard.acknowledge_session_save(&state, result.is_ok());
                        let archive_changed =
                            guard.ingest_archive_saved(&state.session_archive, result.is_ok());
                        let dormant_changed = guard
                            .ingest_dormant_saved(&state.agent_sleep, result.is_ok())
                            | archive_changed;
                        let settings_changed =
                            guard.acknowledge_project_settings_save(&state, result.is_ok());
                        if let Err(message) = &result {
                            if settings_changed {
                                guard.push_diagnostic(
                                    "project_settings.save_failed",
                                    message.clone(),
                                );
                            } else if resolving && !dormant_changed {
                                guard.push_diagnostic(
                                    if resolution {
                                        "session.resolve.save_failed"
                                    } else {
                                        "revisit.save_failed"
                                    },
                                    message.clone(),
                                );
                            } else if archive_changed {
                                // The archive said what the failed save left
                                // closed and open; that notice stays.
                                guard.push_diagnostic("ui_state.save_failed", message.clone());
                            } else {
                                guard.set_error("ui_state.save_failed", message.clone(), true);
                            }
                        }
                        resolved_changed || dormant_changed || settings_changed || result.is_err()
                    };
                    if changed {
                        context.notifier.notify();
                    }
                }
            }) {
            Ok(worker) => {
                self.state_save_worker = Some(worker);
                Ok(())
            }
            Err(error) => {
                self.state_save_active = false;
                Err(format!("UI state save worker could not start: {error}"))
            }
        }
    }

    pub(super) fn persist_ui_state(&mut self) {
        // Every registration change is persisted here, which is when a Home's
        // links follow it (PRD home-device-rail D-06).
        self.request_home_link_syncs();
        // ...and when a machine's protections follow it (PRD agent-guardrails B11).
        self.refresh_guardrail_policies();
        if let Err(message) = self.write_ui_state() {
            self.set_error("ui_state.save_failed", message, true);
        }
    }

    /// A native-file fork is bound to its admitted intent and source session.
    /// Ordinary activity changes do not replace that execution.
    /// The worker checks this without doing file I/O under Runtime.
    pub(crate) fn fork_request_is_current(
        &self,
        request: &ForkRequest,
        connector: &Arc<dyn hide_herdr_client::ApiConnector>,
    ) -> bool {
        request.connection_generation == self.live_generation
            && self
                .live
                .as_ref()
                .is_some_and(|live| Arc::ptr_eq(&live.api_connector, connector))
            // A later press for the same pane/session cannot authorize this
            // worker: each admitted fork has its own name.
            && self
                .forks_in_flight
                .get(&request.parent_pane_id)
                .is_some_and(|pending| pending.name == request.name)
            && self.snapshot.navigator.agents.iter().any(|agent| {
                agent.pane_id == request.parent_pane_id
                    && hide_agent_adapter::canonical_kind(&agent.agent_kind) == request.agent.kind()
                    && agent.session_id.as_deref() == Some(request.session_id.as_str())
                    && agent.row_facts.is_some()
                    && agent
                        .row_facts
                        .as_ref()
                        .and_then(|facts| facts.native_reference.as_ref())
                        == request.source_reference.as_ref()
            })
            && self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .flat_map(|tab| &tab.panes)
                .any(|pane| {
                    pane.id == request.parent_pane_id
                        && Some(pane.cwd.as_str()) == request.cwd.as_deref()
                })
    }

    /// Herdr made the fork's pane, in its own tab (`Fork agent`) or beside
    /// the source (`Fork agent to the side`). The keyboard goes to it now, as
    /// to a starting agent tab, unless the operator chose a view after the
    /// press: that is where they are, and the fork waits where it was put
    /// (B5, D-10). Keys typed meanwhile wait for the agent's start either way.
    pub fn ingest_fork_pane(
        &mut self,
        source_pane_id: &str,
        tab_id: Option<&str>,
        pane_id: &str,
    ) -> bool {
        let Some(pending) = self.forks_in_flight.get_mut(source_pane_id) else {
            return false;
        };
        // A pane the replaced Herdr made is not one this connection can place
        // or focus; the worker's own checks end that fork.
        if pending.generation != self.live_generation {
            self.push_diagnostic(
                "pane.fork.connection_changed",
                format!("Herdr was replaced while {source_pane_id} was being forked"),
            );
            return false;
        }
        pending.tab_id = tab_id.map(str::to_owned);
        pending.pane_id = Some(pane_id.to_owned());
        let (issued_serial, press, tab_label, fork_origin) = (
            pending.issued_serial,
            pending.press,
            pending.tab_label.clone(),
            pending.origin,
        );
        // Another fork's own focus is not the operator choosing a view: it
        // supersedes this one only when it was pressed later, or when the
        // operator chose a view after this press and before that focus.
        let chosen_since = match self
            .fork_focus
            .filter(|focus| focus.intent_serial == self.last_view_intent_serial)
        {
            Some(focus) => focus.press > press || focus.prior_intent_serial > issued_serial,
            None => self.last_view_intent_serial > issued_serial,
        };
        let source_workspace_id = self
            .layout_holding_pane(source_pane_id)
            .map(|layout| layout.workspace_id.clone());
        let checkout_path = tab_id.and_then(|tab_id| self.place_fork_tab(source_pane_id, tab_id));
        // The fork's pane is drawn from Herdr's answer, its own tab as a
        // created tab is and a split as a split Hide asked for is, so its
        // checkout holds it before Herdr's layout lists it
        // (`created_pane_place`). A device's pane is drawn by that device.
        if !crate::agent_hooks::is_remote_pane(source_pane_id) {
            match (tab_id, checkout_path.as_deref(), source_workspace_id) {
                (Some(tab_id), Some(path), Some(workspace_id)) => {
                    self.draw_created_tab(super::drawn_ahead::ProvisionalTab {
                        workspace_id: Some(workspace_id),
                        tab_id: tab_id.to_owned(),
                        pane_id: pane_id.to_owned(),
                        cwd: path.to_owned(),
                        label: tab_label.unwrap_or_default(),
                        timing_id: None,
                        recorded_at_unix_ms: unix_milliseconds(),
                    });
                }
                (None, ..) => self.draw_answered_split(source_pane_id, pane_id),
                _ => {}
            }
        }
        let origin = super::terminal_input::InputOrigin::Fork(source_pane_id.to_owned());
        // Only the operator's own press brings the fork forward with the
        // keyboard; any other caller's fork waits where it was put (#1271).
        if fork_origin != super::events::ForkOrigin::Operator {
            self.hold_background_fork_input(&origin, press, pane_id);
            return true;
        }
        // A tab the Agent area has no room for waits in the strip, as a new
        // tab does, and is not where the operator's keys go.
        let waiting = tab_id
            .zip(checkout_path.as_deref())
            .is_some_and(|(tab_id, path)| !self.agent_can_show_created(path, tab_id));
        if chosen_since || waiting {
            self.push_diagnostic(
                "pane.fork.focus_superseded",
                if waiting {
                    format!("The Agent area has no room to show the fork of {source_pane_id}; it waits in the strip")
                } else {
                    format!("A view was chosen after {source_pane_id} was forked; the fork waits where it was put")
                },
            );
            self.hold_background_fork_input(&origin, press, pane_id);
            return true;
        }
        let prior_intent_serial = self.last_view_intent_serial;
        self.focus_pane(pane_id.to_owned(), PaneFocusOrigin::Operator, None);
        // A focus the core refused (a waiting tab, a closing pane) moved
        // nothing: the fork does not hold the keys or count as a move.
        if self.snapshot.focused.pane_id.as_deref() != Some(pane_id) {
            self.hold_background_fork_input(&origin, press, pane_id);
            return true;
        }
        if let Some(pending) = self.forks_in_flight.get_mut(source_pane_id) {
            pending.focused = Some(pane_id.to_owned());
        }
        // Every client's keys wait on the node while the agent starts.
        self.hold_input_request_for(&origin, pane_id);
        self.fork_focus = Some(ForkFocus {
            intent_serial: self.last_view_intent_serial,
            prior_intent_serial,
            press,
        });
        true
    }

    fn hold_background_fork_input(
        &mut self,
        origin: &super::terminal_input::InputOrigin,
        press: u64,
        pane_id: &str,
    ) {
        // Keys from the abandoned press stay discarded, but later clients
        // addressing the pane still wait for its agent.
        self.discard_input_request(origin, "not_shown");
        self.input_requests.open(
            &format!("fork-hold-{press}"),
            origin.clone(),
            std::time::Instant::now(),
        );
        self.hold_input_request_for(origin, pane_id);
    }

    /// `Fork agent`'s tab goes right of the source's tab in the source's Agent
    /// area. A source tab that has closed meanwhile leaves the new tab where a
    /// new tab goes (PRD fork-operator-root B2). Returns the checkout the tab
    /// belongs to, unknown once the source's tab has closed.
    fn place_fork_tab(&mut self, source_pane_id: &str, tab_id: &str) -> Option<String> {
        let (checkout_path, source_tab) = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .find_map(|checkout| {
                let tab = checkout
                    .tabs
                    .iter()
                    .find(|tab| tab.panes.iter().any(|pane| pane.id == source_pane_id))?;
                Some((checkout.path.clone(), tab.id.clone()?))
            })?;
        // The pane is born in the source's folder, which can sit below the
        // checkout; the tab is the checkout's (`record_created_tab_checkout`).
        self.record_created_tab_checkout(tab_id, &checkout_path);
        let placement_capacity = self.agent_placement_capacity(None);
        let Some(store) = self.workspace_views.as_mut() else {
            return Some(checkout_path);
        };
        let key = (self.node.as_str().to_owned(), checkout_path.clone());
        let Some((group, area, index)) = store.views.get(&key.0, &key.1).and_then(|view| {
            let group = view.agent_tab_group(&source_tab)?;
            let area = view.group_layout(group)?.tree.area_of(&source_tab)?;
            let index = area.displays.iter().position(|tab| tab.id == source_tab)?;
            Some((group.map(str::to_owned), area.id.clone(), index + 1))
        }) else {
            return Some(checkout_path);
        };
        let view = store.views.entry(&key.0, &key.1);
        if view.agent_tab_group(tab_id).is_some() {
            let selected = view.agent_groups.selected.clone();
            let result = view.move_agent_to_group(tab_id, group.as_deref(), unix_milliseconds());
            view.agent_groups.selected = selected;
            if let Err(error) = result {
                self.push_diagnostic("agent_group.fork_placement_failed", format!("{error:?}"));
                return Some(checkout_path);
            }
        }
        let layout = view
            .group_layout_mut(group.as_deref())
            .expect("source group exists");
        // Herdr's session can carry the tab before this answer does; the
        // reconcile appended it then, and it moves next to its source now.
        if layout.tree.display(tab_id).is_some() {
            let stamp = layout.tree.next_stamp(unix_milliseconds());
            match layout.tree.move_display(tab_id, &area, index, stamp) {
                Ok(true) => self.persist_workspace_views(),
                Ok(false) => {}
                Err(error) => self.push_diagnostic(
                    "agent_layout.reconcile_refused",
                    format!("Forked tab {tab_id}: {error:?}"),
                ),
            }
            return Some(checkout_path);
        }
        if !placement_capacity && !store.agent_placements.contains_key(tab_id) {
            self.push_diagnostic(
                "agent_layout.placement_limit",
                "Too many pending Agent tab placements",
            );
            return Some(checkout_path);
        }
        store
            .agent_placements
            .insert(tab_id.to_owned(), (key, group, area, Some(index)));
        Some(checkout_path)
    }

    /// Readiness affects held keys only. A slow or unreadable agent keeps
    /// its successful fork and pane; the node reports actual lost bytes.
    pub(crate) fn discard_fork_input(&mut self, parent_pane_id: &str) {
        let origin = super::terminal_input::InputOrigin::Fork(parent_pane_id.to_owned());
        self.discard_input_request(&origin, "unconfirmed");
    }

    pub fn ingest_fork_result(
        &mut self,
        parent_pane_id: &str,
        result: Result<String, String>,
        elapsed_ms: u128,
    ) -> bool {
        let pending = self.forks_in_flight.remove(parent_pane_id);
        let origin = super::terminal_input::InputOrigin::Fork(parent_pane_id.to_owned());
        // Either outcome leaves the process, because a fork that produced no
        // pane and no message is the report the operator brought: a modal
        // appeared and nothing else happened.
        match result {
            Ok(forked_pane_id) => {
                if let Some(pending) = &pending {
                    self.record_pane_launch(
                        &forked_pane_id,
                        &pending.kind,
                        &pending.name,
                        &pending.launch,
                        None,
                    );
                }
                self.push_diagnostic(
                    "pane.fork.created",
                    format!("Forked pane {parent_pane_id} into {forked_pane_id} in {elapsed_ms}ms"),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_fork",
                    "kind": "pane.fork.created",
                    "pane_id": parent_pane_id,
                    "forked_pane_id": forked_pane_id,
                    "duration_ms": elapsed_ms,
                }));
                // Keys typed since the press reach the fork's agent now that
                // it has started, never its launch command; a fork that did
                // not take the keyboard was not where they were going.
                self.resolve_input_request(&origin, &forked_pane_id);
                true
            }
            Err(message) => {
                self.discard_input_request(&origin, "failed");
                let (tab_id, pane_id, focused, generation) = pending
                    .map(|pending| {
                        (
                            pending.tab_id,
                            pending.pane_id,
                            pending.focused,
                            Some(pending.generation),
                        )
                    })
                    .unwrap_or_default();
                // The worker closed the pane it made, so what was drawn for
                // it goes now rather than at its bound.
                match (tab_id, pane_id) {
                    (Some(tab_id), _) => {
                        self.take_back_created_tab(&tab_id);
                        if let Some(store) = self.workspace_views.as_mut() {
                            store.agent_placements.remove(&tab_id);
                        }
                    }
                    (None, Some(pane_id)) => self.take_back_answered_split(&pane_id),
                    (None, None) => {}
                }
                // The worker closed the pane the keyboard went to; it goes
                // back to the source the operator forked, unless they have
                // moved on since.
                if let Some(pane) = focused
                    && generation == Some(self.live_generation)
                    && self.snapshot.focused.pane_id.as_deref() == Some(pane.as_str())
                {
                    self.focus_pane(parent_pane_id.to_owned(), PaneFocusOrigin::Operator, None);
                }
                self.set_error(
                    "pane.fork_failed",
                    format!("Pane {parent_pane_id} could not be forked: {message}"),
                    true,
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_fork",
                    "kind": "pane.fork_failed",
                    "pane_id": parent_pane_id,
                    "message": message,
                    "duration_ms": elapsed_ms,
                }));
                true
            }
        }
    }

    pub fn ingest_remote_control_result(
        &mut self,
        target_id: &str,
        request_id: &str,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, String>,
        elapsed_ms: u128,
    ) -> bool {
        self.ingest_remote_control_result_with_generation(
            target_id, request_id, action, result, elapsed_ms, None,
        )
    }

    pub(crate) fn ingest_remote_control_result_with_generation(
        &mut self,
        target_id: &str,
        request_id: &str,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, String>,
        elapsed_ms: u128,
        connection_generation: Option<u64>,
    ) -> bool {
        self.ingest_remote_control_result_with_failure(
            target_id,
            request_id,
            action,
            result.map_err(live::ControlFailure::Definite),
            elapsed_ms,
            connection_generation,
        )
    }

    pub(crate) fn ingest_remote_control_failure_with_generation(
        &mut self,
        target_id: &str,
        request_id: &str,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
        connection_generation: Option<u64>,
    ) -> bool {
        self.ingest_remote_control_result_with_failure(
            target_id,
            request_id,
            action,
            result,
            elapsed_ms,
            connection_generation,
        )
    }

    // Account for a confirmed creation even when its connection has since left.
    // This records placement only; focus and the device catalog need a current answer.
    fn settle_remote_terminal_creation(
        &mut self,
        target_id: &str,
        action: &RemoteControlAction,
        result: &Result<RemoteControlOutcome, live::ControlFailure>,
    ) -> bool {
        let (cwd, admission_id, group, area) = match action {
            RemoteControlAction::CreateTab {
                cwd,
                admission_id,
                group_id,
                area_id,
                ..
            }
            | RemoteControlAction::OpenOwner {
                cwd,
                admission_id,
                group_id,
                area_id,
                ..
            } => (cwd, admission_id, group_id, area_id),
            _ => return false,
        };
        if result.as_ref().is_err_and(|error| error.is_ambiguous()) {
            return false;
        }
        let key = (target_id.to_owned(), cwd.clone());
        if let Some(id) = admission_id {
            // Forgetting a device also ends its pending creations.
            if self.workspace_views.as_ref().is_none_or(|store| {
                store
                    .agent_admissions
                    .get(&key)
                    .is_none_or(|claims| !claims.contains(&format!("create:{id}")))
            }) {
                return false;
            }
            self.finish_agent_creation(&key, *id);
        }
        if let Some(key) = remote_tab_creation_key(target_id, action) {
            self.remote_tab_creations_in_flight.remove(&key);
        }
        let placement_capacity = self.agent_placement_capacity(None);
        if let Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(tab),
            ..
        }) = result
            && let Some(area) = area
            && let Some(store) = self.workspace_views.as_mut()
        {
            if placement_capacity {
                store.agent_placements.insert(
                    crate::session_sync::remote_tab_id(target_id, tab),
                    (key, group.as_deref().map(str::to_owned), area.clone(), None),
                );
            } else {
                self.push_diagnostic(
                    "agent_layout.placement_limit",
                    "Too many pending Agent tab placements",
                );
            }
        }
        true
    }

    fn ingest_remote_control_result_with_failure(
        &mut self,
        target_id: &str,
        request_id: &str,
        action: RemoteControlAction,
        result: Result<RemoteControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
        connection_generation: Option<u64>,
    ) -> bool {
        let report = self.registration_terminal_report(target_id, request_id);
        // A device's tab move is held per checkout like this machine's, and
        // its own record already names the connection that carried it.
        if let RemoteControlAction::RenameTab { request_id, .. } = &action {
            return self.ingest_tab_rename_result(
                request_id,
                result
                    .map(|_| ())
                    .map_err(|error| error.message().to_owned()),
            );
        }
        if let RemoteControlAction::MoveTab {
            checkout_id,
            tab_id,
            expected_order,
            generation,
            connection_generation,
            ..
        } = &action
        {
            return self.ingest_tab_move_result(
                TabMoveResultContext {
                    checkout_id,
                    tab_id,
                    expected_order,
                    generation: *generation,
                    connection_generation: *connection_generation,
                    elapsed_ms,
                },
                result,
            );
        }
        let action_kind = action.kind();
        let remote_operation_key = (target_id.to_owned(), request_id.to_owned());
        if let Some(generation) = connection_generation {
            if self
                .remote_connection_generations
                .get(target_id)
                .copied()
                .unwrap_or(0)
                != generation
            {
                // Its navigation is stale; a definite terminal creation still owns
                // an admission and a group placement that must be settled.
                self.drop_pending_choice(target_id, request_id);
                let settled = if matches!(
                    &action,
                    RemoteControlAction::CreateTab {
                        admission_id: Some(_),
                        ..
                    } | RemoteControlAction::OpenOwner {
                        admission_id: Some(_),
                        ..
                    }
                ) {
                    self.settle_remote_terminal_creation(target_id, &action, &result)
                } else {
                    false
                };
                self.push_diagnostic(
                    "remote.control.stale_result",
                    format!(
                        "Ignored remote {action_kind} result for {target_id} from connection generation {generation}"
                    ),
                );
                return settled;
            }
            if self
                .remote_operations
                .get(&remote_operation_key)
                .is_some_and(|operation| operation.connection_generation != generation)
            {
                self.drop_pending_choice(target_id, request_id);
                return false;
            }
        }
        let tracked_remote_operation = self.remote_operations.contains_key(&remote_operation_key);
        let is_pane_focus = matches!(
            &action,
            RemoteControlAction::Pane(PaneControlAction::Focus { .. })
        );
        if self
            .remote_operations
            .get(&remote_operation_key)
            .and_then(|operation| operation.deadline_at_unix_ms)
            .is_some_and(|deadline| deadline <= unix_milliseconds())
        {
            let message =
                "Remote result arrived after its deadline; no mutation was resent".to_owned();
            self.drop_pending_choice(target_id, request_id);
            self.fail_remote_operation_reported(
                report,
                &remote_operation_key,
                message.clone(),
                true,
            );
            if is_pane_focus {
                self.finish_pane_focus_request_by_id(request_id, "failed", Some(message), true);
            }
            return true;
        }
        if let Some(key) = remote_tab_creation_key(target_id, &action) {
            self.remote_tab_creations_in_flight.remove(&key);
        }
        self.settle_remote_terminal_creation(target_id, &action, &result);
        if result.is_err() {
            self.drop_pending_choice(target_id, request_id);
        }
        match result {
            Ok(RemoteControlOutcome::Acknowledged {
                created_tab_id,
                created_pane_id,
            }) => {
                if tracked_remote_operation {
                    self.acknowledge_remote_operation(
                        target_id,
                        request_id,
                        created_pane_id.clone(),
                    );
                }
                if is_pane_focus {
                    self.finish_pane_focus_request_by_id(request_id, "succeeded", None, false);
                }
                if let (
                    Some(tab_id),
                    RemoteControlAction::CreateTab { cwd, .. }
                    | RemoteControlAction::OpenOwner { cwd, .. },
                ) = (created_tab_id.as_deref(), &action)
                {
                    self.note_terminal_choice(
                        target_id,
                        request_id,
                        crate::session_sync::remote_tab_id(target_id, tab_id),
                    );
                    self.record_created_device_tab(target_id, tab_id, cwd);
                }
                self.acknowledge_choice(target_id, request_id);
                let mut receipt = String::new();
                if let Some(tab_id) = created_tab_id.as_deref() {
                    receipt.push_str(&format!("; created tab {tab_id}"));
                }
                if let Some(pane_id) = created_pane_id.as_deref() {
                    receipt.push_str(&format!("; created pane {pane_id}"));
                }
                self.push_diagnostic(
                    "remote.control.ready",
                    format!(
                        "{action_kind} for {target_id} acknowledged in {elapsed_ms} ms{receipt}; awaiting authoritative event"
                    ),
                );
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_control",
                    "kind": "remote.control.ready",
                    "target": target_id,
                    "request_id": request_id,
                    "action": action_kind,
                    "created_tab_id": created_tab_id,
                    "created_pane_id": created_pane_id,
                    "duration_ms": elapsed_ms,
                }));
            }
            // A tab move is settled above; no other action reports an order.
            Ok(RemoteControlOutcome::TabsOrdered { .. }) => {
                self.drop_pending_choice(target_id, request_id);
                if tracked_remote_operation {
                    self.fail_remote_operation_reported(
                        report,
                        &remote_operation_key,
                        format!("{action_kind} returned a tab-order outcome"),
                        false,
                    );
                }
                if is_pane_focus {
                    self.finish_pane_focus_request_by_id(
                        request_id,
                        "failed",
                        Some(format!(
                            "{action_kind} for {target_id} returned an invalid outcome"
                        )),
                        true,
                    );
                }
                if !tracked_remote_operation {
                    self.report_remote_failure(
                        report,
                        target_id,
                        "remote.control.failed",
                        format!("{action_kind} for {target_id} returned a tab order remotely"),
                        false,
                    );
                }
            }
            Err(error) => {
                let message = error.message().to_owned();
                if tracked_remote_operation {
                    let ambiguous = error.is_ambiguous();
                    self.fail_remote_operation_reported(
                        report,
                        &remote_operation_key,
                        if ambiguous {
                            format!(
                                "{message}; no mutation was resent and a fresh topology is required"
                            )
                        } else {
                            message.clone()
                        },
                        ambiguous,
                    );
                }
                if is_pane_focus {
                    self.finish_pane_focus_request_by_id(
                        request_id,
                        "failed",
                        Some(message.clone()),
                        true,
                    );
                }
                if !tracked_remote_operation {
                    self.report_remote_failure(
                        report,
                        target_id,
                        "remote.control.failed",
                        format!("{action_kind} for {target_id} failed: {message}"),
                        true,
                    );
                }
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_control",
                    "kind": "remote.control.failed",
                    "target": target_id,
                    "request_id": request_id,
                    "action": action_kind,
                    "message": message,
                    "duration_ms": elapsed_ms,
                }));
            }
        }
        true
    }

    /// Names each agent by the project and checkout its pane sits in, as the
    /// sidebar tree shows them. Herdr's own workspace label ("hide main") is
    /// a launcher artifact that can span two repositories, so it is kept only
    /// for a pane the navigator has not placed.
    pub(super) fn place_agents_in_navigator(&self, agents: &mut [SidebarAgentSnapshot]) {
        for agent in agents {
            let placed = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .find_map(|workspace| {
                    workspace.checkouts.iter().find_map(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .flat_map(|tab| tab.panes.iter())
                            .any(|pane| pane.id == agent.pane_id)
                            .then(|| (workspace.label.clone(), checkout.label.clone()))
                    })
                });
            if let Some((workspace_label, checkout_label)) = placed {
                agent.workspace_label = workspace_label;
                agent.checkout_label = Some(checkout_label);
            }
        }
    }

    /// Lays on each row the answers its session still waits for and who
    /// will give each. The ledger is the only source of that fact, so it is
    /// laid here, before the lineage decides from the recipients whether an
    /// answer is still being made (docs/status-model.md, A quiet row that
    /// waits). A recipient the projection cannot vouch for is left out: a
    /// code-owned one has no row, and one on a server that is not answering
    /// (`unreachable`) shows a state that may no longer hold.
    fn lay_awaited_replies(
        &self,
        agents: &mut [SidebarAgentSnapshot],
        unreachable: &HashSet<String>,
    ) {
        let ledger = self.delivery_ledger.as_ref().ok();
        let now = unix_milliseconds();
        for agent in agents {
            let awaited = ledger
                .map(|ledger| {
                    ledger
                        .awaited_answers(&agent.pane_id, agent.lineage_session.as_deref(), now)
                        // A turn the session began after the letter went out
                        // took the request up without the reply, so that
                        // wait is over.
                        .filter(|letter| {
                            !crate::agent_state::escalation::took_up_request_after(
                                agent,
                                letter.created_at_unix_ms,
                            ) && !letter.recipient.code_owned()
                                && !unreachable.contains(&letter.recipient.pane_id)
                        })
                        .map(|letter| crate::model::AwaitedReply {
                            recipient_pane_id: letter.recipient.pane_id.clone(),
                            recipient_session: letter.recipient.session.clone(),
                            taken_in: !letter.awaiting_intake(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            agent.awaited_replies = awaited;
        }
    }

    /// Marks the rows Hide itself started. The ledger is the only source of
    /// that fact, so it is laid here with the other ledger facts, and never
    /// from a branch or worktree name (PRD hide-ai-origin-mark D-07). A row
    /// matches while its registration is live and its session is the one the
    /// registration holds, so a later agent in a reused pane is not marked.
    /// A device's pane is registered under the id the core names it by
    /// (`remote:<device>:pane:<pane>`), so one comparison serves both.
    fn lay_hide_ai(&self, agents: &mut [SidebarAgentSnapshot]) {
        for agent in agents {
            agent.hide_ai = self
                .hide_started
                .agents
                .iter()
                .find(|started| {
                    started.pane_id == agent.pane_id
                        && started.session.is_some()
                        && started.session == agent.lineage_session
                })
                .map(|started| {
                    let home = started.task.as_ref().and_then(|task| task.kind.home());
                    // Only a kind with a home has an attempt to read; a kind
                    // this build does not know stays an operator row (D-19).
                    let attempt = home.and_then(|_| match started.task.as_ref()?.kind {
                        crate::coordination::TaskKind::MemoryPromote => self
                            .project_memory
                            .promotions
                            .home_attempt(&started.agent_id),
                        crate::coordination::TaskKind::SuggestionApply => {
                            self.suggestions_work.apply.home_attempt(&started.agent_id)
                        }
                        crate::coordination::TaskKind::Unknown(_) => None,
                    });
                    crate::model::HideAiSnapshot {
                        task: started.task.clone(),
                        home,
                        workspace_id: attempt.and_then(|attempt| {
                            self.catalog_workspaces()
                                .find(|workspace| {
                                    workspace.device_id == attempt.node.as_str()
                                        && workspace.path == attempt.root
                                })
                                .map(|workspace| workspace.id.clone())
                        }),
                        turn: attempt.and_then(|attempt| {
                            super::memory_promotion::home_turn(
                                attempt.pull,
                                agent.activity == "working",
                            )
                        }),
                    }
                });
        }
    }

    /// The checkouts Hide made for the agents it started. A spawn's machine
    /// is the id of the device the shell names its Projects by (this node's
    /// own id for a local one), so it is the checkout's device as it stands.
    fn hide_ai_checkouts(&self) -> Vec<crate::model::HideAiCheckoutSnapshot> {
        self.hide_started
            .checkouts
            .iter()
            .map(|started| crate::model::HideAiCheckoutSnapshot {
                device_id: started.machine.clone(),
                path: started.path.clone(),
                task: started.task.clone(),
            })
            .collect()
    }

    pub(super) fn refresh_agent_lineage(&mut self) -> bool {
        let before_local = self.snapshot.navigator.agents.clone();
        let before_remote = self
            .snapshot
            .status
            .remote
            .iter()
            .map(|remote| {
                (
                    remote.target_id.clone(),
                    remote
                        .session
                        .as_ref()
                        .map(|session| session.agents.clone()),
                )
            })
            .collect::<Vec<_>>();
        let machine_targets = self
            .device_machine_ids
            .iter()
            .map(|(target, machine)| (machine.clone(), target.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut unresolved = HashSet::new();
        let mut agents = Vec::new();
        let mut workspaces = self.snapshot.navigator.workspaces.clone();

        let resolve = |agent: &mut SidebarAgentSnapshot,
                       origin: Option<&str>,
                       unresolved: &mut HashSet<String>| {
            let Some(parent) = agent.declared_parent_pane_id.as_deref() else {
                agent.spawned_from_pane_id = None;
                return;
            };
            agent.spawned_from_pane_id = match agent.spawned_from_machine_id.as_deref() {
                None => Some(match origin {
                    Some(target) => crate::session_sync::remote_pane_id(target, parent),
                    None => parent.to_owned(),
                }),
                Some(machine) if self.node == machine => Some(parent.to_owned()),
                Some(machine) => machine_targets
                    .get(machine)
                    .map(|target| crate::session_sync::remote_pane_id(target, parent)),
            };
            if agent.spawned_from_machine_id.is_some() && agent.spawned_from_pane_id.is_none() {
                unresolved.insert(agent.pane_id.clone());
            }
        };

        for mut agent in self.snapshot.navigator.agents.clone() {
            resolve(&mut agent, None, &mut unresolved);
            agents.push(agent);
        }
        for remote in &self.snapshot.status.remote {
            if let Some(session) = &remote.session {
                workspaces.extend(session.workspaces.clone());
                for mut agent in session.agents.clone() {
                    resolve(&mut agent, Some(&remote.target_id), &mut unresolved);
                    agents.push(agent);
                }
            }
        }
        for pane in unresolved.difference(&self.unresolved_machine_lineage) {
            crate::diagnostic!(serde_json::json!({
                "component": "lineage",
                "kind": "parent_machine_unresolved",
                "pane_id": pane,
                "message": "The parent machine is disconnected or has no matching machine identity",
            }));
        }
        self.unresolved_machine_lineage = unresolved;
        // A device that is not connected keeps its last session for the
        // lineage below, but its rows are neither shown nor closable, so no
        // close ever names them (PRD close-agent-subtree D-16), and nothing
        // there is known to be making an answer.
        let unreachable = self
            .snapshot
            .status
            .remote
            .iter()
            .filter(|remote| remote.state != "connected")
            .filter_map(|remote| remote.session.as_ref())
            .flat_map(|session| session.agents.iter().map(|agent| agent.pane_id.clone()))
            .collect::<HashSet<_>>();
        self.lay_awaited_replies(&mut agents, &unreachable);
        self.lay_checks(&mut agents);
        self.lay_hide_ai(&mut agents);
        crate::agent_state::apply_lineage(
            &mut agents,
            &workspaces,
            &self.snapshot.ui_state.expanded_agent_pane_ids,
        );
        if !unreachable.is_empty() {
            for agent in &mut agents {
                agent
                    .close_descendant_pane_ids
                    .retain(|pane| !unreachable.contains(pane));
            }
        }
        let resolved = agents
            .into_iter()
            .map(|agent| (agent.pane_id.clone(), agent))
            .collect::<BTreeMap<_, _>>();
        for agent in &mut self.snapshot.navigator.agents {
            if let Some(row) = resolved.get(&agent.pane_id) {
                *agent = row.clone();
            }
        }
        for remote in &mut self.snapshot.status.remote {
            if let Some(session) = &mut remote.session {
                for agent in &mut session.agents {
                    if let Some(row) = resolved.get(&agent.pane_id) {
                        *agent = row.clone();
                    }
                }
            }
        }
        let changed = before_local != self.snapshot.navigator.agents
            || before_remote.iter().any(|(target, before)| {
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .find(|remote| &remote.target_id == target)
                    .and_then(|remote| remote.session.as_ref())
                    .map(|session| &session.agents)
                    != before.as_ref()
            });
        let hide_ai_checkouts = self.hide_ai_checkouts();
        let changed = changed | (self.snapshot.navigator.hide_ai_checkouts != hide_ai_checkouts);
        self.snapshot.navigator.hide_ai_checkouts = hide_ai_checkouts;
        // Machine identity, remote arrival and local arrival all publish the
        // same ownership transition, including normal tab placement.
        let changed = changed | self.sync_pane_lineage();
        // The request block reads the lineage (a delegated child's first
        // request is its parent's), so it is built after it.
        changed | self.sync_request_rows()
    }

    /// Logs the descendants whose activity Herdr cannot classify, once per
    /// change in that count per row: they are left off the badge because a
    /// count the projection cannot vouch for is not drawn (PRD B7).
    pub(super) fn log_unknown_descendants(
        previous: &[SidebarAgentSnapshot],
        current: &[SidebarAgentSnapshot],
    ) {
        for agent in current {
            let unknown = agent.descendant_counts.unknown;
            let before = previous
                .iter()
                .find(|row| row.pane_id == agent.pane_id)
                .map(|row| row.descendant_counts.unknown)
                .unwrap_or(0);
            if unknown > 0 && unknown != before {
                crate::diagnostic!(serde_json::json!({
                    "component": "session",
                    "kind": "lineage.unknown_descendants",
                    "pane_id": agent.pane_id,
                    "unknown": unknown,
                }));
            }
        }
    }
}

impl Runtime {
    /// A device folder the helper has not confirmed has no known kind, so
    /// which owner it gets (a bound worktree workspace or a marked one) is
    /// unknown; nothing is opened on a guess, and the device's catalog notice
    /// says why it is unconfirmed.
    fn refuse_unconfirmed_owner(
        &mut self,
        report: FailureReport,
        target_id: &str,
        checkout: &CheckoutSnapshot,
    ) {
        self.report_remote_failure(
            report,
            target_id,
            "remote.control.checkout_unconfirmed",
            format!(
                "{} on {target_id} is not confirmed yet; no tab was opened",
                checkout.path
            ),
            true,
        );
    }

    /// Tells a remote request's failure to whoever `report` names.
    fn report_remote_failure(
        &mut self,
        report: FailureReport,
        target_id: &str,
        kind: &str,
        message: impl Into<String>,
        retryable: bool,
    ) {
        match report {
            FailureReport::Notice => self.set_error(kind, message, retryable),
            FailureReport::Log { kind: logged } => {
                self.push_diagnostic(logged, format!("{target_id} ({kind}): {}", message.into()))
            }
        }
    }

    /// Ends a remote operation that failed (`ambiguous`: whose effect is
    /// unknown). One the operator asked for stays shown, to retry or to
    /// check; one the core made on its own is dropped and logged.
    fn fail_remote_operation_reported(
        &mut self,
        report: FailureReport,
        key: &(String, String),
        message: String,
        ambiguous: bool,
    ) {
        match report {
            FailureReport::Notice if ambiguous => {
                self.mark_remote_operation_unknown(key, message);
            }
            FailureReport::Notice => self.fail_remote_operation(&key.0, &key.1, message),
            FailureReport::Log { .. } => {
                if let Some(operation) = self.remote_operations.remove(key) {
                    self.sync_async_operations();
                    self.report_remote_failure(report, &key.0, &operation.kind, message, true);
                }
            }
        }
    }
}

/// Who is told when a remote request fails. A request the operator made
/// tells them (`last_error`, and a failed operation to retry); one the core
/// makes on its own, such as a device project's first terminal, tells the
/// diagnostic log under `kind` and leaves nothing on screen (design
/// principle 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FailureReport {
    Notice,
    Log { kind: &'static str },
}

/// The checkout a device row names: the named checkout of that project, or
/// with no checkout named, the project's first.
pub(super) fn remote_checkout<'a>(
    session: &'a RemoteSessionSnapshot,
    workspace_id: &str,
    checkout_id: Option<&str>,
) -> Option<(&'a WorkspaceSnapshot, &'a CheckoutSnapshot)> {
    let project = session
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)?;
    let checkout = match checkout_id {
        Some(checkout_id) => project
            .checkouts
            .iter()
            .find(|checkout| checkout.id == checkout_id)?,
        None => project.checkouts.first()?,
    };
    Some((project, checkout))
}

/// A device pane's children and ancestors, from that device's own lineage,
/// so its header carries the same child row and Return as a pane on this
/// machine (S6 B14-B16, B21). A device pane has no local hook tokens; the
/// projection reports it as uninstrumented for in-process counts, while its
/// Herdr-declared children are still known.
fn sync_remote_pane_relations(session: &mut RemoteSessionSnapshot) -> bool {
    let agents = &session.agents;
    let mut changed = false;
    for pane in session
        .workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        let children = crate::sidebar::project_pane_children(
            agents,
            &pane.id,
            crate::agent_hooks::PaneHookTokens::default(),
            &|_| None,
        );
        let lineage_path = crate::sidebar::project_lineage_path(agents, &pane.id);
        // A device's pane offers no fork, but a fork there still names its
        // source, as its rows do.
        let forked_from = agents
            .iter()
            .find(|agent| agent.pane_id == pane.id)
            .and_then(|agent| agent.forked_from.clone());
        if pane.fork.forked_from != forked_from {
            pane.fork.forked_from = forked_from;
            changed = true;
        }
        if pane.children != children {
            pane.children = children;
            changed = true;
        }
        if pane.lineage_path != lineage_path {
            pane.lineage_path = lineage_path;
            changed = true;
        }
    }
    changed
}

/// The keyboard move the core made for a fork, so a fork still starting can
/// tell it from a view the operator chose.
#[derive(Clone, Copy, Debug)]
pub(super) struct ForkFocus {
    /// `last_view_intent_serial` right after the move.
    pub(super) intent_serial: u64,
    /// `last_view_intent_serial` just before it: the operator's last choice.
    pub(super) prior_intent_serial: u64,
    /// The press that fork answered (`PendingFork::press`).
    pub(super) press: u64,
}

/// A fork the operator asked for, until its worker answers.
#[derive(Debug)]
pub(super) struct PendingFork {
    /// The order of the press among forks, so a later press keeps the
    /// keyboard from an earlier one that is still starting.
    pub(super) press: u64,
    /// The Herdr connection the press was made on.
    pub(super) generation: u64,
    /// The control lane's last serial at the press. A view chosen after it is
    /// the operator's, and the fork does not take the keyboard from it.
    pub(super) issued_serial: u64,
    /// Who asked for it: only the operator's press takes the keyboard.
    pub(super) origin: super::events::ForkOrigin,
    /// The fork's tab once Herdr named it, so a fork that fails leaves no
    /// placement or drawing waiting for a tab that was closed.
    pub(super) tab_id: Option<String>,
    /// The fork's pane once Herdr named it, so a failed split is taken back.
    pub(super) pane_id: Option<String>,
    /// The name the fork's tab is made with, which its drawing carries.
    pub(super) tab_label: Option<String>,
    /// The fork's pane, once Herdr made it and it took the keyboard.
    pub(super) focused: Option<String>,
    /// The source's agent kind and launch choice, and the name the fork
    /// starts under, which the fork's pane keeps once the fork started.
    pub(super) kind: String,
    pub(super) name: String,
    pub(super) launch: hide_agent_adapter::LaunchChoice,
}
