//! The core's own engine on devices (PRD engine-bundle-3-core-owns D-22,
//! B35): with the development switch `HIDE_DEV_PANE_HOLDER` on, a device the
//! core dials (over SSH, or a WSL distribution of this PC) runs no Herdr; its
//! node runs pane holders, and the core keeps the device's session itself,
//! in an engine of its own for that device, as it keeps this machine's.
//!
//! The engine lives as long as one link: it is opened, off the runtime lock,
//! once the device's node is ready, and let go when that link ends, which
//! saves its session. A node that starts again (every new link starts its
//! node) is laid out again from that save around the holders it finds, so
//! a pane whose holder ran on keeps its id and its process. A node that
//! dials this core keeps its Herdr: only a dialed device is the core's to
//! run.
//!
//! With the switch off nothing here runs, and a device's session is its
//! Herdr's, as it always was.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::model::DeviceRegistration;
use hide_node_link::NodeLink;

/// What a core whose devices run on its own engine needs to open one.
pub(super) struct OwnDevices {
    /// The core's state folder, which keeps each device's engine folder.
    state_dir: PathBuf,
}

impl Runtime {
    /// Runs every device this core dials on the core's own engine, keeping
    /// their sessions under `state_dir`: a core created with the own engine
    /// for this machine.
    pub(crate) fn run_devices_on_own_engine(&mut self, state_dir: PathBuf) {
        self.own_devices = Some(OwnDevices { state_dir });
    }

    /// Whether `registration`'s panes run on the core's own engine: the
    /// switch is on and the core dials the device.
    pub(super) fn device_on_own_engine(&self, registration: &DeviceRegistration) -> bool {
        self.own_devices.is_some() && registration.origin.core_redials()
    }

    /// The device's node is ready on `link` for attempt `generation`: opens
    /// the device's engine on a worker, off the lock, since it reads its
    /// saved session, and installs it once open if that link is still the
    /// device's. `home` is the account the node answers for.
    pub(super) fn open_own_device_engine(
        &mut self,
        device_id: &str,
        generation: u64,
        link: Arc<dyn NodeLink>,
        home: Option<String>,
    ) {
        let Some(own) = self.own_devices.as_ref() else {
            return;
        };
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        let state_dir = own.state_dir.clone();
        let definitions = Arc::new(
            self.agent_definitions
                .as_ref()
                .map(|state| state.catalog.clone())
                .unwrap_or_else(|| hide_agent_adapter::definition::Catalog::read(Vec::new())),
        );
        let device = device_id.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-device-engine".to_owned())
            .spawn(move || {
                let opened = home
                    .ok_or_else(|| "the device's node named no home".to_owned())
                    .and_then(|home| {
                        crate::engine::own::open_device(
                            &device,
                            Arc::clone(&link),
                            &state_dir,
                            home,
                            definitions,
                        )
                    })
                    .map(|engine| Arc::new(engine) as crate::engine::SharedEngine);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                // An engine not installed is let go after the lock, where its
                // last save is written.
                let (changed, unused) = match runtime.lock() {
                    Ok(mut guard) => guard.install_own_device_engine(&device, generation, opened),
                    Err(_) => return,
                };
                drop(runtime);
                drop(unused);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            crate::diagnostic!(serde_json::json!({
                "component": "own_devices",
                "kind": "device_engine.open_unstarted",
                "target": device_id,
                "message": error.to_string(),
            }));
        }
    }

    /// Takes the engine opened for attempt `generation` of `device_id`, and
    /// starts its session's coordinator, unless the device moved on since:
    /// removed, or another attempt began. Answers whether the screen changed
    /// and the engine when it was not taken.
    fn install_own_device_engine(
        &mut self,
        device_id: &str,
        generation: u64,
        opened: Result<crate::engine::SharedEngine, String>,
    ) -> (bool, Option<crate::engine::SharedEngine>) {
        let current = self.device_hosts.get(device_id).is_some_and(|host| {
            host.generation == generation && matches!(host.phase, hosts::HostPhase::Ready { .. })
        });
        let registration = self.device_registration(device_id).cloned();
        let (Some(registration), true) = (registration, current) else {
            return (false, opened.ok());
        };
        let engine = match opened {
            Ok(engine) => engine,
            Err(message) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_devices",
                    "kind": "device_engine.open_failed",
                    "target": device_id,
                    "message": message,
                }));
                return (
                    self.ingest_remote_session(
                        device_id,
                        Err(live::SessionFetchError::Unreachable(message)),
                    ),
                    None,
                );
            }
        };
        let Some(context) = self.worker_context.clone() else {
            return (false, Some(engine));
        };
        self.install_remote_control(live::RemoteControlContext::new(
            device_id.to_owned(),
            Arc::clone(&engine),
            context.runtime.clone(),
            context.notifier.clone(),
        ));
        let (sync, changed) = self.spawn_remote_sync(&registration, Arc::clone(&engine));
        if let Some(connection) = self.remote_connections.get_mut(device_id)
            && let Some(retired) = std::mem::replace(&mut connection.sync, sync)
        {
            self.retired_remote_syncs.push(retired);
        }
        // The engine's definitions follow the core's later reads too.
        if let Some(state) = self.agent_definitions.as_ref() {
            engine.set_agent_definitions(Arc::new(state.catalog.clone()));
        }
        (changed, None)
    }

    /// The device's link ended: its engine and the coordinator of its
    /// session go with it, the engine saving its session as it is let go
    /// once the coordinator has stopped, and its row reads why until a new
    /// link brings a new engine.
    pub(super) fn retire_own_device_engine(&mut self, device_id: &str, reason: &str) -> bool {
        let on_own = self
            .device_registration(device_id)
            .is_some_and(|registration| self.device_on_own_engine(registration));
        if !on_own {
            return false;
        }
        self.remote_controls.remove(device_id);
        if let Some(sync) = self
            .remote_connections
            .get_mut(device_id)
            .and_then(|connection| connection.sync.take())
        {
            self.retired_remote_syncs.push(sync);
        }
        self.ingest_remote_session(
            device_id,
            Err(live::SessionFetchError::Unreachable(reason.to_owned())),
        )
    }

    /// The engines whose definitions follow the core's reads: this
    /// machine's and every device's on the core's own engine.
    pub(crate) fn own_device_engines(&self) -> Vec<crate::engine::SharedEngine> {
        if self.own_devices.is_none() {
            return Vec::new();
        }
        self.remote_controls
            .values()
            .map(live::RemoteControlContext::engine)
            .collect()
    }
}
