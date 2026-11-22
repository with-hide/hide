//! A helper device's switch to the connection of the Hide on its machine
//! (PRD device-connect-iroh D-49, B35, B36, B41).
//!
//! The steps run in hided, which reaches the machine over the device's SSH
//! login, or through `wsl.exe` when the device is a WSL distribution of
//! this PC (D-27), has a Hide put there when it has none and pairs the two. The
//! core hears each step as a `device_switch` event, sent by the daemon
//! only: it draws the step on the device's row and, once the paired
//! machine's grant is active, changes the registration's link in place.
//! The device keeps its id and its name, and so its engine folder, its
//! projects, its tabs and every record that names it; only the transport
//! under it is made again. Until that moment the registration is the
//! helper's and its link is untouched, so a switch that stops leaves the
//! device as it was, with the reason on its row.

use serde::Deserialize;

use super::devices::Retiring;
use super::*;
use crate::model::{DeviceSwitchSnapshot, DeviceSwitchState, LinkOrigin};

/// Where one device's switch stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DeviceSwitch {
    state: DeviceSwitchState,
    reason: Option<String>,
    /// The connection key of the machine the device is, known from the
    /// pairing on: the grant that becomes active under it takes the device.
    key: Option<String>,
    /// What the Hide this switch started on the machine still needs, which
    /// the device's row says once it is that machine's.
    needs: Option<crate::model::DeviceNeedsSnapshot>,
}

/// A registration rewritten to its machine's connection whose save is not
/// acknowledged yet: the connection it was rewritten to, and what it was.
#[derive(Clone, Debug)]
pub(super) struct SwitchSaving {
    key: String,
    was: LinkOrigin,
    /// What the Hide the switch started there still needs, which the
    /// device's row says once it is that machine's.
    needs: Option<crate::model::DeviceNeedsSnapshot>,
}

/// What a helper device is switched from: how the core reaches it, and
/// where its helper is.
pub struct DeviceSwitchSource {
    /// The device's id, which the one who asked may have named by its
    /// alias.
    pub device_id: String,
    pub via: SwitchVia,
    /// The helper root the operator allowed on the device, as the consent
    /// spells it.
    pub helper_root: String,
    /// The folders registered as projects on the device, as the device
    /// spells them, which its machine's own Hide lists from then on (PRD
    /// device-connect-iroh D-48); at most [`SWITCHED_PROJECTS`].
    pub projects: Vec<String>,
}

/// What hided reaches the machine a helper device is through.
pub enum SwitchVia {
    /// Over the SSH alias the device is dialed by.
    Ssh {
        alias: String,
        /// Where the questions of the switch's SSH login go: the device's
        /// row.
        sign_in: Option<Arc<dyn hide_node_link::device::SignIn>>,
    },
    /// Through `wsl.exe`, into this PC's distribution of that name; it asks
    /// nothing.
    Wsl { distro: String },
}

/// The most projects a switch hands the device's machine, which is as many
/// as that machine's `hide device accept-ssh` takes: as many as a core adds
/// at once.
pub const SWITCHED_PROJECTS: usize = 32;

#[derive(Debug, Deserialize)]
pub(super) struct DeviceSwitchPayload {
    pub(super) device_id: String,
    pub(super) step: SwitchStep,
    /// With `connecting`: the paired machine's connection key.
    #[serde(default)]
    pub(super) key: Option<String>,
    /// With `failed`: why, as a code.
    #[serde(default)]
    pub(super) reason: Option<String>,
    /// With `connecting`: what the Hide this switch started there still
    /// needs to stay reachable (B30).
    #[serde(default)]
    pub(super) needs: Option<crate::model::DeviceNeedsSnapshot>,
    /// With `failed`: the switch started nothing on the device's machine
    /// and no grant was saved there, so a device added for this switch can
    /// be taken off again (B41).
    #[serde(default)]
    pub(super) clean: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum SwitchStep {
    Checking,
    Installing,
    Connecting,
    Failed,
    /// The switch is over with nothing to show: the operator was asked
    /// something first, or cancelled.
    Ended,
}

impl DeviceSwitchPayload {
    pub(super) fn valid(&self) -> bool {
        let key_spelled = self
            .key
            .as_deref()
            .is_none_or(crate::model::peer_key_is_spelled);
        let reason_plain = self.reason.as_deref().is_none_or(|reason| {
            !reason.is_empty()
                && reason.len() <= 64
                && reason
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        });
        let needs_spelled = self
            .needs
            .as_ref()
            .is_none_or(crate::model::DeviceNeedsSnapshot::valid);
        key_spelled
            && reason_plain
            && needs_spelled
            && (self.step != SwitchStep::Connecting || self.key.is_some())
    }
}

impl Runtime {
    /// What hided switches `device` from, or the code of why it is not a
    /// device that can be switched: only a device the core dials over SSH
    /// under its machine's own id, which the paired machine's device then
    /// has too, or a WSL distribution of this PC. `device` is its id, or
    /// the SSH alias or the distribution it is dialed by, which is what a
    /// terminal knows of a device it has just added.
    pub(crate) fn device_switch_source(
        &self,
        device: &str,
    ) -> Result<DeviceSwitchSource, &'static str> {
        let registration = self
            .device_registration(device)
            .or_else(|| {
                self.snapshot
                    .ui_state
                    .device_registrations
                    .iter()
                    .find(|registration| {
                        registration.origin.ssh_alias() == Some(device)
                            || registration.origin.wsl_distro() == Some(device)
                    })
            })
            .ok_or("device_unknown")?;
        let device_id = registration.id.as_str();
        if !registration.origin.switches() {
            return Err("switch_unsupported");
        }
        // The operator's press on the device's row started the switch, so
        // its login asks there, as the device's own connection attempts do
        // (D-45). A device that never had an attempt has no row state to
        // put a question in: its login then asks nobody.
        let sign_in = self
            .worker_context
            .clone()
            .zip(self.device_hosts.get(device_id))
            .map(|(context, host)| {
                Arc::new(super::device_sign_in::RowSignIn {
                    context,
                    device_id: device_id.to_owned(),
                    generation: host.generation,
                    wake: crate::remote::Wake::Allowed,
                    withdrawn: Arc::default(),
                }) as Arc<dyn hide_node_link::device::SignIn>
            });
        let via = match registration.origin.dialed_by() {
            Some(crate::model::DialedBy::Ssh(alias)) => SwitchVia::Ssh { alias, sign_in },
            Some(crate::model::DialedBy::Wsl(distro)) => SwitchVia::Wsl { distro },
            None => return Err("switch_unsupported"),
        };
        Ok(DeviceSwitchSource {
            device_id: device_id.to_owned(),
            via,
            helper_root: registration
                .host_consent
                .as_ref()
                .map(|consent| consent.helper_root.clone())
                .unwrap_or_else(|| self.host_helper_root()),
            projects: self
                .snapshot
                .ui_state
                .workspace_registrations
                .iter()
                .filter(|project| project.device_id == device_id)
                .map(|project| project.path.clone())
                .take(SWITCHED_PROJECTS)
                .collect(),
        })
    }

    /// The switch of `device_id` as its row draws it.
    pub(super) fn device_switch_snapshot(&self, device_id: &str) -> Option<DeviceSwitchSnapshot> {
        self.device_switches
            .get(device_id)
            .map(|switch| DeviceSwitchSnapshot {
                state: switch.state,
                reason: switch.reason.clone(),
            })
    }

    /// The helper device that is being switched to the machine under `key`.
    pub(super) fn switching_to(&self, key: &str) -> Option<String> {
        self.device_switches
            .iter()
            .find(|(_, switch)| switch.key.as_deref() == Some(key))
            .map(|(device, _)| device.clone())
    }

    /// One step of a device's switch, as hided said it.
    pub(super) fn device_switch(&mut self, payload: DeviceSwitchPayload) -> bool {
        let device_id = payload.device_id;
        let helper = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .any(|registration| registration.id == device_id && registration.origin.dials_helper());
        if !helper {
            // Switched already, or removed meanwhile: no step is drawn.
            let dropped = self.device_switches.remove(&device_id).is_some();
            return dropped && self.refresh_device_snapshots();
        }
        // A device added over SSH, or as a WSL distribution, to be switched
        // is not left on the helper
        // when that switch stops with nothing started on its machine (B41):
        // it is taken off again, and one line says why. Past that point its
        // machine runs a Hide from the helper's folder, or holds a grant
        // that may still become active, so it stays, with its reason and a
        // retry, like any helper device.
        if payload.step == SwitchStep::Failed {
            let fresh = self
                .device_registration(&device_id)
                .and_then(|registration| registration.origin.dialed_by())
                .filter(|by| self.device_fresh_adds.contains(by));
            if let Some(by) = fresh {
                self.device_fresh_adds.remove(&by);
                if payload.clean {
                    return self.take_off_fresh_add(&device_id, payload.reason.as_deref());
                }
            }
        }
        let state = match payload.step {
            SwitchStep::Ended => {
                let dropped = self.device_switches.remove(&device_id).is_some();
                return dropped && self.refresh_device_snapshots();
            }
            SwitchStep::Checking => DeviceSwitchState::Checking,
            SwitchStep::Installing => DeviceSwitchState::Installing,
            SwitchStep::Connecting => DeviceSwitchState::Connecting,
            SwitchStep::Failed => DeviceSwitchState::Failed,
        };
        let key = payload
            .key
            .filter(|_| payload.step == SwitchStep::Connecting);
        self.device_switches.insert(
            device_id.clone(),
            DeviceSwitch {
                state,
                reason: payload
                    .reason
                    .filter(|_| payload.step == SwitchStep::Failed),
                key: key.clone(),
                needs: payload
                    .needs
                    .filter(|_| payload.step == SwitchStep::Connecting),
            },
        );
        // A machine that was paired before the switch was asked for is
        // active already: no change of the paired machines will say so.
        if let Some(key) = key
            && self.peer_opens(&key)
        {
            self.switch_device_link(&device_id, &key);
        }
        self.refresh_device_snapshots();
        true
    }

    /// The paired machine under `key` may be opened: the helper device
    /// `device_id` becomes that machine's device. The registration is
    /// rewritten and saved here, and nothing of the link changes until that
    /// save is acknowledged ([`Self::acknowledge_device_switch`]): a core
    /// that ended between the two would read the helper registration again
    /// and dial a folder the machine's own Hide already runs this core's
    /// node from.
    pub(super) fn switch_device_link(&mut self, device_id: &str, key: &str) -> bool {
        // Rewritten already and waiting for its save: the paired machines
        // changing again meanwhile asks for nothing more.
        if self.device_switch_saving.contains_key(device_id) {
            return false;
        }
        let taken = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .any(|registration| registration.origin.peer_key() == Some(key));
        let Some(registration) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == device_id && registration.origin.dials_helper())
            .filter(|_| !taken)
        else {
            // Another registration is that machine's device already: the
            // helper device stays as it is until that one is removed.
            crate::diagnostic!(serde_json::json!({
                "component": "device_switch",
                "kind": "device_switch.not_switched",
                "target": device_id,
                "key_taken": taken,
            }));
            self.fail_device_switch(device_id, "device_already_connected");
            return self.refresh_device_snapshots();
        };
        let was = std::mem::replace(
            &mut registration.origin,
            LinkOrigin::Peer {
                key: key.to_owned(),
                switched: true,
            },
        );
        let needs = self
            .device_switches
            .get_mut(device_id)
            .and_then(|switch| switch.needs.take());
        self.device_switch_saving.insert(
            device_id.to_owned(),
            SwitchSaving {
                key: key.to_owned(),
                was,
                needs,
            },
        );
        // Its first open on the new connection closes the panes that
        // connection does not hold; saved with the rewrite, so a core that
        // stops before that open still does it.
        self.snapshot
            .ui_state
            .device_switch_opens
            .insert(device_id.to_owned());
        // Saved before the link changes; a standalone runtime acknowledges
        // within this call.
        self.persist_current_ui_state();
        self.refresh_device_snapshots();
        true
    }

    /// A save of the core's state ended: every switched registration it
    /// holds takes its machine's connection when the save succeeded, and
    /// goes back to its helper, with the reason on its row, when it did
    /// not. A save taken before the switch holds no such registration and
    /// settles nothing; the save the switch asked for follows it.
    pub(super) fn acknowledge_device_switch(
        &mut self,
        saved: &UiStateSnapshot,
        success: bool,
    ) -> bool {
        if self.device_switch_saving.is_empty() {
            return false;
        }
        let held: Vec<String> = self
            .device_switch_saving
            .iter()
            .filter(|(id, saving)| {
                saved.device_registrations.iter().any(|registration| {
                    &registration.id == *id
                        && registration.origin.peer_key() == Some(saving.key.as_str())
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for id in held {
            let Some(saving) = self.device_switch_saving.remove(&id) else {
                continue;
            };
            changed |= if success {
                self.land_device_switch(&id, saving)
            } else {
                self.undo_device_switch(&id, saving)
            };
        }
        changed
    }

    /// The switched registration of `device_id` is saved: its helper link
    /// is closed and its machine's connection is dialed in its place, under
    /// the same registration. The machine's own Hide then starts this
    /// core's node on the folder the helper ran in, and the panes are taken
    /// up again as on any new link.
    fn land_device_switch(&mut self, device_id: &str, saving: SwitchSaving) -> bool {
        // The device may have been removed since the save was asked.
        let Some(registration) = self
            .device_registration(device_id)
            .filter(|registration| registration.origin.peer_key().is_some())
            .cloned()
        else {
            return false;
        };
        self.device_switches.remove(device_id);
        if let Some(by) = saving.was.dialed_by() {
            self.device_fresh_adds.remove(&by);
        }
        if let Some(needs) = saving.needs {
            self.device_needs.insert(device_id.to_owned(), needs);
        }
        self.rebuild_device_rows();
        let focused_device = self.look.front().map(str::to_owned);
        let persisted_focus = self.snapshot.ui_state.focused_device_id.clone();
        // One selection, as a retry keeps it: the device in front stays
        // there while its link is made again (`retry_remote_device`).
        self.change_look(|runtime| {
            runtime.disconnect_remote_device(device_id, Retiring::Retried);
            runtime.set_front_device(focused_device);
            runtime.snapshot.ui_state.focused_device_id = persisted_focus;
        });
        self.push_diagnostic(
            "device.switched",
            format!("Device {device_id} now uses its machine's own connection"),
        );
        // The machine keeps its own Settings from here on: Hide AI placed
        // on the device runs on this machine again (Settings per machine).
        if self.drop_placements_on_own_settings_machines() {
            self.refresh_background_ai();
        }
        self.connect_remote_device(&registration, super::hosts::HostAttempt::Operator);
        self.refresh_device_snapshots();
        true
    }

    /// The save that held `device_id`'s switched registration failed: the
    /// registration is the helper's again, as the store still has it, its
    /// link was never touched, and its row says why (B41).
    fn undo_device_switch(&mut self, device_id: &str, saving: SwitchSaving) -> bool {
        let Some(registration) =
            self.snapshot
                .ui_state
                .device_registrations
                .iter_mut()
                .find(|registration| {
                    registration.id == device_id
                        && registration.origin.peer_key() == Some(saving.key.as_str())
                })
        else {
            return false;
        };
        registration.origin = saving.was;
        self.snapshot.ui_state.device_switch_opens.remove(device_id);
        crate::diagnostic!(serde_json::json!({
            "component": "device_switch",
            "kind": "device_switch.not_saved",
            "target": device_id,
        }));
        self.fail_device_switch(device_id, "switch_not_saved");
        self.rebuild_device_rows();
        self.refresh_device_snapshots();
        true
    }

    /// Whether the next engine open of `device_id` is the first on the
    /// connection its switch gave it.
    pub(super) fn switch_open_pending(&self, device_id: &str) -> bool {
        self.snapshot
            .ui_state
            .device_switch_opens
            .contains(device_id)
            && !self.device_switch_saving.contains_key(device_id)
            && self
                .device_registration(device_id)
                .is_some_and(|registration| registration.origin.switched())
    }

    /// The first restore of `device_id` on the connection its switch gave
    /// it ended, having closed `left` for want of a holder there (PRD
    /// device-connect-iroh D-49, B36): each agent's conversation is kept
    /// under 보관됨 for 이어가기, the row says how many terminals were not
    /// taken up, and the device owes no such open any more. Said again by
    /// every read of that engine, and settled once.
    pub(crate) fn settle_switch_open(
        &mut self,
        device_id: &str,
        left: &[crate::session_sync::NotCarried],
    ) -> bool {
        if !self.snapshot.ui_state.device_switch_opens.remove(device_id) {
            return false;
        }
        let mut kept = 0usize;
        for pane in left {
            match self.archive_not_carried(device_id, pane) {
                Ok(true) => kept += 1,
                Ok(false) => {}
                Err(why) => crate::diagnostic!(serde_json::json!({
                    "component": "device_switch",
                    "kind": "device_switch.conversation_not_kept",
                    "target": device_id,
                    "pane_id": pane.pane_id,
                    "reason": why,
                })),
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "device_switch",
            "kind": "device_switch.opened",
            "target": device_id,
            "not_carried": left.len(),
            "conversations_kept": kept,
        }));
        if let Ok(count) = u32::try_from(left.len())
            && count > 0
        {
            self.device_not_carried.insert(device_id.to_owned(), count);
        }
        self.persist_ui_state();
        if self.refresh_device_snapshots()
            && let Some(context) = self.worker_context.as_ref()
        {
            context.notifier.notify();
        }
        true
    }

    /// Takes the device `device_id`, added over SSH for a switch that
    /// stopped before it started anything on its machine, off again, and
    /// keeps the one line that says why.
    fn take_off_fresh_add(&mut self, device_id: &str, reason: Option<&str>) -> bool {
        let Some(label) = self
            .device_registration(device_id)
            .map(|registration| registration.label.clone())
        else {
            return false;
        };
        if let Err(refusal) = self.remove_device(device_id) {
            crate::diagnostic!(serde_json::json!({
                "component": "device_switch",
                "kind": "device_switch.fresh_add_not_removed",
                "target": device_id,
                "code": refusal.code,
            }));
            // It stays a helper device, with its reason.
            self.fail_device_switch(device_id, reason.unwrap_or("switch_failed"));
            return self.refresh_device_snapshots();
        }
        self.snapshot.navigator.device_not_added = Some(crate::model::DeviceNotAddedSnapshot {
            label,
            reason: reason.unwrap_or("switch_failed").to_owned(),
        });
        true
    }

    /// The switch of `device_id` stopped for `reason`, which its row says.
    fn fail_device_switch(&mut self, device_id: &str, reason: &str) {
        self.device_switches.insert(
            device_id.to_owned(),
            DeviceSwitch {
                state: DeviceSwitchState::Failed,
                reason: Some(reason.to_owned()),
                key: None,
                needs: None,
            },
        );
    }
}
