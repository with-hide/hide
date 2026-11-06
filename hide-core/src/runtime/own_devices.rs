//! The core's own engine on devices (PRD engine-bundle-3-core-owns D-22,
//! B35, engine-bundle-4-herdr-removal D-10): a device (one the core dials
//! over SSH, a WSL distribution of this PC, or a node that dials the core)
//! runs no engine; its node runs pane holders, and the core keeps the
//! device's session itself, in an engine of its own for that device, as it
//! keeps this machine's.
//!
//! The engine lives as long as one link: it is opened, off the runtime lock,
//! once the device's node is ready, and let go when that link ends, which
//! saves its session. A node that starts again (every new link starts its
//! node) is laid out again from that save around the holders it finds, so
//! a pane whose holder ran on keeps its id and its process.

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

    /// Whether this core runs on its own engine, so its devices run on it
    /// too (`run_devices_on_own_engine`).
    pub(super) fn runs_own_engine(&self) -> bool {
        self.own_devices.is_some()
    }

    /// Whether `registration`'s panes run on the core's own engine: the core
    /// runs on it.
    pub(super) fn device_on_own_engine(&self, _registration: &DeviceRegistration) -> bool {
        self.own_devices.is_some()
    }

    /// The device's node is ready on `link` for attempt `generation`: opens
    /// the device's engine on a worker, off the lock, since it reads its
    /// saved session, and installs it once open if that link is still the
    /// device's. `home` is the account the node answers for, and `windows`
    /// whether its machine runs Windows, as the node's hello says.
    pub(super) fn open_own_device_engine(
        &mut self,
        device_id: &str,
        generation: u64,
        link: Arc<dyn NodeLink>,
        home: Option<String>,
        windows: bool,
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
        // The device's agents resume with the choices they were started
        // with (D-24), keyed by the device's own pane ids as its engine's are.
        let launches: std::collections::BTreeMap<String, crate::model::PaneLaunch> = self
            .snapshot
            .ui_state
            .pane_launches
            .iter()
            .filter_map(|(pane, launch)| {
                Some((
                    crate::address::pane_of(pane, device_id)?.to_owned(),
                    launch.clone(),
                ))
            })
            .collect();
        let launches = Arc::new(launches);
        let device = device_id.to_owned();
        // The device's engine save is looked at before its engine opens, by
        // its node (B18); a device whose machine the core cannot name is
        // not, since its registrations could not be told apart.
        let local = self.herdr_import_local();
        let machine = self.device_machine_ids.get(device_id).cloned();
        if local.is_some() && machine.is_none() {
            crate::diagnostic!(serde_json::json!({
                "component": "own_devices",
                "kind": "herdr_import.device_unnamed",
                "target": device_id,
            }));
        }
        let spawned = thread::Builder::new()
            .name("hide-core-device-engine".to_owned())
            .spawn(move || {
                let mover = local.zip(machine).and_then(|(local, machine)| {
                    let engine_dir =
                        hide_kit::layout::device_engine_dir(&state_dir, &device).ok()?;
                    Some(crate::herdr_import::device::Device {
                        link: Arc::clone(&link),
                        engine_dir,
                        machine,
                        hide_used: crate::herdr_import::hide_used(&state_dir, &device),
                        local,
                    })
                });
                let seen = match mover.as_ref().map(|mover| mover.look_first()) {
                    // The core role starts again and moves it before any
                    // link to the device is up (D-7).
                    Some((_, true)) => return,
                    Some((seen, false)) => Some(seen),
                    None => None,
                };
                let opened = home
                    .ok_or_else(|| "the device's node named no home".to_owned())
                    .and_then(|home| {
                        crate::engine::own::open_device(
                            &device,
                            Arc::clone(&link),
                            &state_dir,
                            home,
                            windows,
                            definitions,
                            launches,
                        )
                    })
                    .map(|engine| Arc::new(engine) as crate::engine::SharedEngine);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                // An engine not installed is let go after the lock, where its
                // last save is written, and so are the coordinators the
                // install retired, whose end takes the lock.
                let (changed, unused, retired) = match runtime.lock() {
                    Ok(mut guard) => {
                        let (mut changed, unused) =
                            guard.install_own_device_engine(&device, generation, opened);
                        if let (Some(mover), Some(seen)) = (mover, seen)
                            && guard.device_link_current(&device, generation)
                        {
                            changed |=
                                guard.install_device_herdr_import(&device, Arc::new(mover), seen);
                        }
                        (changed, unused, guard.take_retired_remote_syncs())
                    }
                    Err(_) => return,
                };
                drop(runtime);
                drop(retired);
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
    pub(super) fn install_own_device_engine(
        &mut self,
        device_id: &str,
        generation: u64,
        opened: Result<crate::engine::SharedEngine, String>,
    ) -> (bool, Option<crate::engine::SharedEngine>) {
        let current = self.device_link_current(device_id, generation);
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
        // A coordinator is never dropped here: its end joins a thread that
        // takes this lock, so one with no connection to hold it, or the one
        // it replaces, is retired and ended off the lock.
        let retired = match self.remote_connections.get_mut(device_id) {
            Some(connection) => std::mem::replace(&mut connection.sync, sync),
            None => sync,
        };
        self.retired_remote_syncs.extend(retired);
        // The engine's definitions follow the core's later reads too.
        if let Some(state) = self.agent_definitions.as_ref() {
            engine.set_agent_definitions(Arc::new(state.catalog.clone()));
        }
        (changed, None)
    }

    /// Whether attempt `generation` is still `device_id`'s ready link.
    fn device_link_current(&self, device_id: &str, generation: u64) -> bool {
        self.device_hosts.get(device_id).is_some_and(|host| {
            host.generation == generation && matches!(host.phase, hosts::HostPhase::Ready { .. })
        })
    }

    /// The device's node is ready but its holders did not start (`reason`):
    /// no engine opens on that link, and the device's row reads why until a
    /// new link starts them.
    pub(super) fn own_device_holders_unstarted(&mut self, device_id: &str, reason: &str) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "own_devices",
            "kind": "device_engine.holders_unstarted",
            "target": device_id,
            "message": reason,
        }));
        self.ingest_remote_session(
            device_id,
            Err(live::SessionFetchError::Unreachable(format!(
                "This device's pane holders did not start: {reason}"
            ))),
        )
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
        let import_changed = self.retire_device_herdr_import(device_id);
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
        ) | import_changed
    }

    /// The engines of the devices on the core's own engine, whose
    /// definitions follow the core's reads as this machine's do.
    pub(crate) fn own_device_engines(&self) -> Vec<crate::engine::SharedEngine> {
        if self.own_devices.is_none() {
            return Vec::new();
        }
        self.remote_controls
            .iter()
            .filter(|(device_id, _)| {
                self.device_registration(device_id)
                    .is_some_and(|registration| self.device_on_own_engine(registration))
            })
            .map(|(_, control)| live::RemoteControlContext::engine(control))
            .collect()
    }
}
