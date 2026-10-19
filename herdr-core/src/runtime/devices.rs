//! The registered SSH devices: each one's connection is opened when it is
//! registered or when the core starts, torn down when it is removed, and
//! probed stage by stage when the operator asks for a test.
//!
//! Settings holds only a label and an SSH alias for a device. Everything else
//! is asked of the host: the alias resolves through `~/.ssh/config`, and the
//! Herdr socket is whatever `herdr status server --json` reports there, so no
//! path of another machine is ever written down here.

use std::sync::Arc;

use super::*;
use crate::model::{
    DeviceRegistration, DeviceTestSnapshot, DeviceTestStageSnapshot, RemoteFileListSnapshot,
    RemoteStatusSnapshot,
};
use crate::remote::{Arrived, CapabilityReport, CapabilityState, DeviceTransport, DialedTransport};

/// How the core reaches a connected device: over a dial of its own, or over
/// the link a node that dials in brought.
#[derive(Clone)]
pub(crate) enum DeviceReach {
    Dialed(Arc<dyn DialedTransport>),
    Inbound(Arc<dyn DeviceTransport>),
}

impl DeviceReach {
    pub(crate) fn device(&self) -> &dyn DeviceTransport {
        match self {
            Self::Dialed(transport) => &**transport,
            Self::Inbound(transport) => &**transport,
        }
    }

    /// Stages `files` on the device for a pane there. Files reach a node
    /// that dials in from its own screen (D-06); a window on another machine
    /// stages nothing there.
    pub(crate) fn stage_attachments(
        &self,
        request_id: &str,
        files: &[hide_node_link::attachments::AttachmentFile],
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<String>, String> {
        match self {
            Self::Dialed(transport) => transport.stage_attachments(request_id, files, cancelled),
            Self::Inbound(_) => Err(
                "A file reaches a pane of this machine from a window on this machine only"
                    .to_owned(),
            ),
        }
    }

    /// Removes what `request_id` staged; nothing was staged on a node that
    /// dials in.
    pub(crate) fn remove_attachments(
        &self,
        request_id: &str,
        files: &[hide_node_link::attachments::AttachmentFile],
    ) {
        if let Self::Dialed(transport) = self {
            transport.remove_attachments(request_id, files);
        }
    }
}

/// Why a device's connection is retired. A removed device keeps nothing of
/// the core's; a retried one is the same device on a new attempt, so what is
/// about the device and not about the attempt stays: the state it last
/// reported, when it first stopped answering, and the kit work queued for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Retiring {
    Removed,
    Retried,
}

pub(super) struct RemoteDeviceConnection {
    pub(super) transport: DeviceReach,
    /// `None` when the coordinator could not be started; the status entry
    /// then carries why.
    sync: Option<session_sync::SessionSyncHandle>,
    /// True while a connection test runs; one test per device at a time, so
    /// a second click cannot stack SSH handshakes behind the first.
    test_in_flight: bool,
}

impl Runtime {
    /// Makes a registered device the one in front: its checkouts, panes and
    /// files are what the shell shows next.
    pub(super) fn bring_device_forward(&mut self, device_id: String) {
        if !self
            .snapshot
            .navigator
            .devices
            .iter()
            .any(|device| device.id == device_id)
        {
            self.set_error(
                "device.unknown",
                format!("Device {device_id} is not registered"),
                false,
            );
            return;
        }
        let local = device_id == self.node.as_str();
        self.snapshot.navigator.focused_device_id = Some(device_id);
        self.yield_surface_to_terminal();
        if local {
            self.return_keyboard_to_local_pane();
        }
        self.reconcile_remote_terminal_selection();
        self.sync_recent_closed_snapshot();
        self.persist_current_ui_state();
    }

    /// Whether `device_id` is already the device in front.
    pub(super) fn device_in_front(&self, device_id: &str) -> bool {
        self.snapshot
            .navigator
            .focused_device_id
            .as_deref()
            .unwrap_or(self.node.as_str())
            == device_id
    }

    /// Connects every device the persisted registrations name. Called once,
    /// after the worker context exists, because a coordinator reports back
    /// through it, and again when a pending core's hold is released.
    pub fn connect_registered_devices(&mut self) -> bool {
        let registrations = self.snapshot.ui_state.device_registrations.clone();
        let mut changed = false;
        for registration in &registrations {
            changed |= self.connect_remote_device(registration);
        }
        changed
    }

    /// Opens the remote status entry for a device and starts its coordinator.
    /// A device that cannot be started still gets its entry, carrying the
    /// reason, so the row never reads as "not attempted".
    pub(super) fn connect_remote_device(&mut self, registration: &DeviceRegistration) -> bool {
        let device_id = registration.id.clone();
        if self.remote_connections.contains_key(&device_id) {
            return false;
        }
        // A node that dials this core has a connection only once it has
        // brought its link; until then its row reads not connected.
        let source = match &registration.origin {
            LinkOrigin::Inbound => match self.inbound_arrivals.remove(&device_id) {
                Some(arrived) => Ok(arrived),
                None => return false,
            },
            // A core waiting for its move's link dials nothing; releasing
            // the hold connects every registration again.
            LinkOrigin::Dialed { .. } if self.effects_held() => return false,
            LinkOrigin::Dialed { ssh_alias } => Err(ssh_alias.clone()),
        };
        if !self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == device_id)
        {
            self.snapshot.status.remote.push(RemoteStatusSnapshot {
                target_id: device_id.clone(),
                state: "not_connected".to_owned(),
                message: Some("Waiting for the first remote connection attempt".to_owned()),
                herdr_version: None,
                session: None,
                files: RemoteFileListSnapshot::idle(),
                catalog: Default::default(),
                unreachable_since_unix_ms: Some(unix_milliseconds()),
            });
        }
        let started = (|| {
            // A registration from before ids were checked, or one a shell
            // wrote into the UI state, is refused here too: its panes' ids
            // would overlap another device's.
            if !hide_node_link::terminal::device_id_is_unambiguous(&device_id) {
                return Err("This device's id contains \":\", so its panes cannot be told apart from another device's; remove it and register it under another id".to_owned());
            }
            let (transport, arrived) = match source {
                Ok(Arrived {
                    transport,
                    node,
                    hear_close,
                }) => (DeviceReach::Inbound(transport), Some((node, hear_close))),
                Err(ssh_alias) => {
                    let home_path = self.home_path.as_ref().ok_or_else(|| {
                        "HOME is unavailable, so the SSH config cannot be resolved".to_owned()
                    })?;
                    let transport = self.devices.transport_session(
                        home_path,
                        &device_id,
                        &ssh_alias,
                        registration.herdr_socket_path.clone(),
                        registration.herdr_session.clone(),
                    )?;
                    (DeviceReach::Dialed(transport), None)
                }
            };
            let connector = transport.device().herdr_api_connector();
            Ok::<_, String>((transport, arrived, connector))
        })();
        let (transport, arrived, connector) = match started {
            Ok(parts) => parts,
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_session_sync",
                    "kind": "device.connect_failed",
                    "target": device_id,
                    "message": message,
                }));
                return self.ingest_remote_session(
                    &device_id,
                    Err(live::SessionFetchError::Unreachable(message)),
                );
            }
        };
        let Some(context) = self.worker_context.clone() else {
            return self.ingest_remote_session(
                &device_id,
                Err(live::SessionFetchError::Unreachable(
                    "The remote session worker is unavailable".to_owned(),
                )),
            );
        };
        self.install_remote_control(live::RemoteControlContext::new(
            device_id.clone(),
            Arc::clone(&connector),
            context.runtime.clone(),
            context.notifier.clone(),
        ));
        self.install_remote_file_transport(device_id.clone(), transport.clone());
        let mut sync_context = session_sync::SessionSyncContext::remote(
            device_id.clone(),
            registration.label.clone(),
            connector,
            context.runtime.clone(),
            context.notifier.clone(),
        );
        if registration.origin == LinkOrigin::Inbound {
            sync_context = sync_context.dialed_in(
                super::inbound::InboundLink::for_device(&device_id, context.runtime.clone()),
                self.own_node(),
            );
        }
        let (sync, changed) = match session_sync::spawn(sync_context, None) {
            Ok(handle) => (Some(handle), false),
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_session_sync",
                    "kind": "coordinator.spawn_failed",
                    "target": device_id,
                    "message": message,
                }));
                let changed = self.ingest_remote_session(
                    &device_id,
                    Err(live::SessionFetchError::Unreachable(message)),
                );
                (None, changed)
            }
        };
        self.remote_connections.insert(
            device_id.clone(),
            RemoteDeviceConnection {
                transport,
                sync,
                test_in_flight: false,
            },
        );
        let host_changed = match arrived {
            Some((node, hear_close)) => self.start_arrived_host(&device_id, node, hear_close),
            None => self.start_device_host(&device_id),
        };
        changed || host_changed || self.refresh_device_snapshots()
    }

    pub(super) fn ingest_device_machine_id(
        &mut self,
        target_id: &str,
        result: Result<String, String>,
    ) -> bool {
        let identity_changed = match result {
            Ok(machine_id) => {
                let changed = self.device_machine_ids.get(target_id) != Some(&machine_id);
                self.device_machine_ids
                    .insert(target_id.to_owned(), machine_id);
                changed
            }
            Err(message) => {
                let removed = self.device_machine_ids.remove(target_id).is_some();
                crate::diagnostic!(serde_json::json!({
                    "component": "lineage",
                    "kind": "machine_identity_unavailable",
                    "target": target_id,
                    "message": message,
                }));
                removed
            }
        };
        identity_changed | self.refresh_agent_lineage()
    }

    /// Forgets what the core holds for a device's connection: its
    /// coordinator, transports, status entry, pending operations and
    /// projected panes. What its helper answered about its directories stays
    /// (`forget_device_catalog`). The coordinator is joined later, off the
    /// lock, by whoever drains `take_retired_remote_syncs`.
    pub(super) fn disconnect_remote_device(&mut self, device_id: &str, why: Retiring) {
        self.device_machine_ids.remove(device_id);
        self.device_ports.remove(device_id);
        self.device_disk.remove(device_id);
        self.forget_disk_sweep(device_id);
        self.device_github.remove(device_id);
        self.device_github_reads
            .retain(|(device, _), _| device != device_id);
        self.device_github_read_generation
            .retain(|(device, _), _| device != device_id);
        self.device_github_over_limit.remove(device_id);
        self.forget_device_host(device_id, why);
        if let Some(connection) = self.remote_connections.remove(device_id)
            && let Some(sync) = connection.sync
        {
            self.retired_remote_syncs.push(sync);
        }
        self.remote_controls.remove(device_id);
        self.terminals.remove_device(device_id);
        self.remote_file_transports.remove(device_id);
        self.remote_connection_generations.remove(device_id);
        self.remote_operations
            .retain(|(target_id, _), _| target_id != device_id);
        self.remote_tab_creations_in_flight
            .retain(|(target_id, _, _, _)| target_id != device_id);
        self.remote_device_tests.remove(device_id);
        self.device_herdr_behind.remove(device_id);
        match why {
            Retiring::Removed => self
                .snapshot
                .status
                .remote
                .retain(|status| status.target_id != device_id),
            // The row reports the new attempt, not the failure of the last
            // one, and keeps the last session and the age of the outage.
            Retiring::Retried => {
                if let Some(status) = self
                    .snapshot
                    .status
                    .remote
                    .iter_mut()
                    .find(|status| status.target_id == device_id)
                {
                    status.state = "not_connected".to_owned();
                    status.message = Some("Reconnecting to the device".to_owned());
                }
            }
        }
        self.reconcile_remote_terminal_panes(device_id, &HashSet::new(), &HashSet::new());
        if self.snapshot.navigator.focused_device_id.as_deref() == Some(device_id) {
            self.snapshot.navigator.focused_device_id = Some(self.node.as_str().to_owned());
            self.snapshot.ui_state.focused_device_id = None;
            self.return_keyboard_to_local_pane();
            self.sync_recent_closed_snapshot();
        }
        self.refresh_agent_lineage();
    }

    /// The device rows after a registration changed. The projects, their
    /// tabs and the focus do not depend on device registrations, so they stay
    /// as the last session projection drew them: rebuilding the whole catalog
    /// here dropped every checkout's tabs until the next session publish, and
    /// the canvas read "no tab" after a device was added or removed. The agent
    /// counts are the projection's too and carry over.
    pub(super) fn rebuild_device_rows(&mut self) {
        let counts = self
            .snapshot
            .navigator
            .devices
            .iter()
            .map(|device| (device.id.clone(), device.agent_count))
            .collect::<HashMap<_, _>>();
        let mut devices = workspace::devices(
            &self.node,
            self.machine_name.as_deref(),
            &self.snapshot.ui_state,
        );
        for device in &mut devices {
            if let Some(count) = counts.get(&device.id) {
                device.agent_count = *count;
            }
        }
        self.snapshot.navigator.devices = devices;
        self.refresh_device_snapshots();
        self.refresh_agent_scopes();
    }

    /// A fresh connection attempt for a registered device that is not
    /// connected: the old coordinator and transports are retired and a new
    /// one starts, so the row reports this attempt rather than a cached one.
    /// Nothing on the host is touched, and the operator's device focus stays.
    pub(super) fn retry_remote_device(&mut self, device_id: &str) -> bool {
        let Some(registration) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device_id)
            .cloned()
        else {
            self.set_error(
                "remote.unknown_target",
                format!("Device {device_id} is not registered"),
                false,
            );
            return true;
        };
        if !registration.origin.core_redials() {
            self.set_error(
                "remote.retry_inbound",
                "This machine connects to the core itself; it reconnects when it can reach the core",
                false,
            );
            return true;
        }
        if self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == device_id && status.state == "connected")
        {
            self.set_error(
                "remote.retry_connected",
                format!("Device {device_id} is already connected"),
                false,
            );
            return true;
        }
        let focused_device = self.snapshot.navigator.focused_device_id.clone();
        let persisted_focus = self.snapshot.ui_state.focused_device_id.clone();
        self.disconnect_remote_device(device_id, Retiring::Retried);
        self.snapshot.navigator.focused_device_id = focused_device;
        self.snapshot.ui_state.focused_device_id = persisted_focus;
        self.push_diagnostic(
            "device.retry",
            format!("Reconnecting SSH device {device_id}"),
        );
        self.connect_remote_device(&registration);
        self.refresh_device_snapshots();
        true
    }

    pub(crate) fn take_retired_remote_syncs(&mut self) -> Vec<session_sync::SessionSyncHandle> {
        std::mem::take(&mut self.retired_remote_syncs)
    }

    /// Every coordinator the runtime still owns, for the FFI layer to join
    /// before the runtime itself is dropped.
    pub(crate) fn take_remote_syncs(&mut self) -> Vec<session_sync::SessionSyncHandle> {
        let mut handles = self.take_retired_remote_syncs();
        handles.extend(
            self.remote_connections
                .values_mut()
                .filter_map(|connection| connection.sync.take()),
        );
        handles
    }

    /// Runs the staged capability test for a device on a worker thread and
    /// shows it as `running` until the report lands.
    pub(super) fn start_device_test(&mut self, device_id: &str) -> bool {
        let Some(connection) = self.remote_connections.get_mut(device_id) else {
            self.set_error(
                "device.not_connected",
                format!("Device {device_id} has no connection to test; remove and add it again"),
                false,
            );
            return true;
        };
        if connection.test_in_flight {
            self.set_error(
                "device.test_in_flight",
                format!("A connection test for {device_id} is already running"),
                false,
            );
            return true;
        }
        let Some(context) = self.worker_context.clone() else {
            self.set_error(
                "device.test_worker_unavailable",
                "The connection test worker is unavailable",
                true,
            );
            return true;
        };
        // A node that dials in has no connection of the core's to test; its
        // row reports its link as it is.
        let DeviceReach::Dialed(transport) = &connection.transport else {
            self.set_error(
                "device.test_inbound",
                "This machine connects to the core itself; test it from its own side",
                false,
            );
            return true;
        };
        let transport = Arc::clone(transport);
        connection.test_in_flight = true;
        self.remote_device_tests.insert(
            device_id.to_owned(),
            DeviceTestSnapshot {
                state: "running".to_owned(),
                checked_at_unix_ms: None,
                stages: Vec::new(),
            },
        );
        self.push_diagnostic(
            "device.connection_test_requested",
            format!(
                "Connection test requested for {device_id}; SSH credentials remain outside hide"
            ),
        );
        let worker_device_id = device_id.to_owned();
        let spawned = thread::Builder::new()
            .name(format!("herdr-core-device-test-{device_id}"))
            .spawn(move || {
                let report = transport.capability_test(
                    "device-connection-test",
                    &crate::remote::check_test_snapshot,
                );
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_device_test(&worker_device_id, &report),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            if let Some(connection) = self.remote_connections.get_mut(device_id) {
                connection.test_in_flight = false;
            }
            self.remote_device_tests.remove(device_id);
            self.set_error(
                "device.test_worker_unavailable",
                format!("The connection test could not start: {error}"),
                true,
            );
        }
        self.refresh_device_snapshots();
        true
    }

    pub(crate) fn ingest_device_test(
        &mut self,
        device_id: &str,
        report: &CapabilityReport,
    ) -> bool {
        let Some(connection) = self.remote_connections.get_mut(device_id) else {
            // Removed while the test ran; the answer has no row to land on.
            return false;
        };
        connection.test_in_flight = false;
        let stages = report
            .stages
            .iter()
            // The tunnel stage is an explicit probe the test does not run.
            .filter(|result| result.stage != crate::remote::RemoteStage::Tunnel)
            .map(|result| DeviceTestStageSnapshot {
                stage: result.stage.to_string(),
                state: match result.state {
                    CapabilityState::Pending => "pending",
                    CapabilityState::Passed => "passed",
                    CapabilityState::Failed { .. } => "failed",
                }
                .to_owned(),
                detail: result.detail.clone(),
            })
            .collect::<Vec<_>>();
        let passed = stages.iter().all(|stage| stage.state == "passed");
        self.remote_device_tests.insert(
            device_id.to_owned(),
            DeviceTestSnapshot {
                state: if passed { "passed" } else { "failed" }.to_owned(),
                checked_at_unix_ms: Some(unix_milliseconds()),
                stages,
            },
        );
        self.push_diagnostic(
            if passed {
                "device.connection_test_passed"
            } else {
                "device.connection_test_failed"
            },
            format!("Connection test for {device_id} finished"),
        );
        self.refresh_device_snapshots();
        true
    }

    /// Reads each remote device row off its remote status and last test. The
    /// rows are rebuilt with every catalog, so this is the one place their
    /// state comes from.
    pub(crate) fn refresh_device_snapshots(&mut self) -> bool {
        let kits = self
            .snapshot
            .navigator
            .devices
            .iter()
            .map(|device| (device.id.clone(), self.kit_view(&device.id)))
            .collect::<HashMap<_, _>>();
        let statuses = &self.snapshot.status.remote;
        let tests = &self.remote_device_tests;
        let mut changed = false;
        for device in &mut self.snapshot.navigator.devices {
            let kit = kits.get(&device.id).cloned().unwrap_or_default();
            if device.kit != kit {
                device.kit = kit;
                changed = true;
            }
            if device.kind != "remote" {
                continue;
            }
            let status = statuses.iter().find(|status| status.target_id == device.id);
            let (state, message) = match status {
                Some(status) if status.state == "connected" => ("ready", None),
                Some(status) => ("unavailable", status.message.clone()),
                None => ("unavailable", None),
            };
            let problem = message
                .as_deref()
                .and_then(crate::remote::connection_problem)
                .map(str::to_owned);
            let test = tests.get(&device.id).cloned();
            let herdr_behind = self.device_herdr_behind.get(&device.id).cloned();
            if device.state != state
                || device.message != message
                || device.problem != problem
                || device.test != test
                || device.herdr_behind != herdr_behind
            {
                device.state = state.to_owned();
                device.message = message;
                device.problem = problem;
                device.test = test;
                device.herdr_behind = herdr_behind;
                changed = true;
            }
        }
        let helper_root = Some(self.host_helper_root());
        if let Some(local) = self
            .snapshot
            .navigator
            .devices
            .iter_mut()
            .find(|device| device.kind != "remote")
            && local.host.helper_root != helper_root
        {
            local.host.helper_root = helper_root;
            changed = true;
        }
        let ids = self
            .snapshot
            .navigator
            .devices
            .iter()
            .filter(|device| device.kind == "remote")
            .map(|device| device.id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            let host = self.host_snapshot(&id);
            if let Some(device) = self
                .snapshot
                .navigator
                .devices
                .iter_mut()
                .find(|device| device.id == id)
                && device.host != host
            {
                device.host = host;
                changed = true;
            }
        }
        changed
    }
}
