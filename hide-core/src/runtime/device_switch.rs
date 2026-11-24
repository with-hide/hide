//! A helper device's switch to the connection of the Hide on its machine
//! (PRD device-connect-iroh D-49, B36).
//!
//! One rule decides it (operator decision 2026-11-24): a device the core
//! dials over SSH, or a WSL distribution of this PC, moves by itself when
//! nothing is lost by it and its machine's own Hide is back after a restart
//! with nobody there; any other is used over its helper as before. Nothing
//! on the screen asks for a switch or tells its steps.
//!
//! The steps run in hided, which reaches the machine over the device's SSH
//! login, or through `wsl.exe` (D-27), has a Hide put there when it has
//! none and pairs the two. Each time a device's helper connected and this
//! build's `hide` is in place there, the core asks the daemon for its
//! switch ([`DeviceMover`]); hided looks first whether the machine comes
//! back by itself and says what it still needs when it does not, which the
//! device's row shows as one faint line. Once the paired machine's grant is
//! active the core changes the registration's link in place: the device
//! keeps its id and its name, and so its engine folder, its projects, its
//! tabs and every record that names it. Until then the registration is the
//! helper's and its link is untouched, so a switch that stops leaves the
//! device as it was and only the log says why.
//!
//! A device another computer switched first keeps its helper folder for
//! that computer, so this one goes on with a folder of its own, which
//! takes up none of the terminals it ran there: the core asks for that
//! switch only once the device lists no terminal, and refuses the link
//! change if one opened meanwhile. No switch closes a terminal.

use serde::Deserialize;

use super::devices::Retiring;
use super::*;
use crate::model::LinkOrigin;

/// Where one device's switch stands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DeviceSwitch {
    /// The connection key of the machine the device is: the grant that
    /// becomes active under it takes the device.
    key: String,
    /// What the Hide this switch started on the machine still needs, which
    /// the device's row says once it is that machine's.
    needs: Option<crate::model::DeviceNeedsSnapshot>,
    /// The switch goes on with a folder of this computer's own, which
    /// takes up no terminal the helper ran: it lands only on a device that
    /// lists none.
    own_folder: bool,
}

/// What switches a helper device by itself: the daemon, which hands one in
/// at its start (`Core::set_device_mover`). A core without one switches a
/// device only when `hide device switch` asks.
pub trait DeviceMover: Send + Sync {
    /// The device `device_id` is due to move; with `own_folder`, another
    /// computer switched it first and it lists no terminal of this
    /// computer's. Called under the core's lock: it only signals.
    fn due(&self, device_id: &str, own_folder: bool);
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
    /// The name the operator knows the device by, which the machine it
    /// becomes keeps (PRD device-connect-iroh, [바꾸기]: the same name).
    pub label: String,
    pub via: SwitchVia,
    /// The helper root the operator allowed on the device, as the consent
    /// spells it.
    pub helper_root: String,
    /// The folders registered as projects on the device, as the device
    /// spells them, which its machine's own Hide lists from then on (PRD
    /// device-connect-iroh D-48); at most [`SWITCHED_PROJECTS`].
    pub projects: Vec<String>,
}

/// Who asks for a device's switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchBy {
    /// The operator, with `hide device switch`.
    Operator,
    /// The core's rule, with nobody asked; with `own_folder`, for a device
    /// another computer switched first, which goes on with a folder of
    /// this computer's own.
    Rule { own_folder: bool },
}

/// What hided reaches the machine a helper device is through.
pub enum SwitchVia {
    /// Over the SSH alias the device is dialed by, by a login that asks
    /// nobody: the read every switch starts with signs in without asking,
    /// so a login that needs an answer never reaches the switch.
    Ssh { alias: String },
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
    /// With `looked`: what the device's machine needs before it is moved,
    /// none when it is moved or nothing the operator does would move it.
    /// With `connecting`: what the Hide this switch started there still
    /// needs to stay reachable (B30).
    #[serde(default)]
    pub(super) needs: Option<crate::model::DeviceNeedsSnapshot>,
    /// With `connecting`: the switch goes on with a folder of this
    /// computer's own.
    #[serde(default)]
    pub(super) own_folder: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum SwitchStep {
    /// hided looked whether the device moves by itself.
    Looked,
    /// Another computer switched the device first (D-75): it is moved with
    /// a folder of this computer's own once it lists no terminal.
    Held,
    /// The two machines are paired and the connection is being opened.
    Connecting,
    /// The switch is over and the device was not switched.
    Ended,
}

impl DeviceSwitchPayload {
    pub(super) fn valid(&self) -> bool {
        let key_spelled = self
            .key
            .as_deref()
            .is_none_or(crate::model::peer_key_is_spelled);
        let needs_spelled = self
            .needs
            .as_ref()
            .is_none_or(crate::model::DeviceNeedsSnapshot::valid);
        key_spelled && needs_spelled && (self.step != SwitchStep::Connecting || self.key.is_some())
    }
}

impl Runtime {
    /// What hided switches `device` from, or the code of why it is not a
    /// device that can be switched: only a device the core dials over SSH
    /// under its machine's own id, which the paired machine's device then
    /// has too, or a WSL distribution of this PC. `device` is its id, or
    /// the SSH alias or the distribution it is dialed by, which is what a
    /// terminal knows of a device it has just added. Its login asks nobody,
    /// whoever asked for the switch, as Hide's own connection attempts do
    /// not (PRD system-ssh-device-link B14).
    pub(crate) fn device_switch_source(
        &self,
        device: &str,
        by: SwitchBy,
    ) -> Result<DeviceSwitchSource, &'static str> {
        let pressed = by == SwitchBy::Operator;
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
        // Asked by the core a while ago, and run now: the operator may have
        // taken their consent back in between.
        if !pressed && !self.due_to_move(device_id) {
            return Err("switch_not_due");
        }
        // And a terminal may have opened there: a folder of this
        // computer's own takes up none, so the switch is refused before
        // its login pairs the two machines.
        if by == (SwitchBy::Rule { own_folder: true })
            && self.device_terminals(device_id) != Some(0)
        {
            return Err("terminals_open");
        }
        let via = match registration.origin.dialed_by() {
            Some(crate::model::DialedBy::Ssh(alias)) => SwitchVia::Ssh { alias },
            Some(crate::model::DialedBy::Wsl(distro)) => SwitchVia::Wsl { distro },
            None => return Err("switch_unsupported"),
        };
        Ok(DeviceSwitchSource {
            device_id: device_id.to_owned(),
            label: registration.label.clone(),
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

    /// Gives the core what switches a device by itself, once, at the
    /// daemon's start; the devices whose helper connected before it are
    /// asked for now.
    pub(crate) fn set_device_mover(&mut self, mover: Arc<dyn DeviceMover>) {
        for device in std::mem::take(&mut self.device_moves_ready) {
            mover.due(&device, false);
        }
        self.device_mover = Some(mover);
    }

    /// A pass that installs this build's kit on `device_id` answered: every
    /// connection runs one, itself or on the device's kit worker, and a
    /// repair is one too. Once it says this build's `hide` is in place
    /// there, which a pass that was refused or could not write it does not,
    /// the `hide` a switch runs is this build's, so a device that is due to
    /// move is switched now.
    pub(crate) fn kit_applied(&mut self, device_id: &str, report: &hide_kit::KitReport) {
        let hide_in_place = report.components.iter().any(|component| {
            component.id == hide_kit::ComponentId::Cli
                && component.state == hide_kit::ComponentState::Installed
        });
        if hide_in_place {
            self.move_when_ready(device_id);
        }
    }

    /// Whether `device_id` is looked at for a move by itself: a device the
    /// core dials itself, on which the operator let this machine's programs
    /// run. Read when its helper connects and again when hided takes its
    /// switch up.
    fn due_to_move(&self, device_id: &str) -> bool {
        self.device_registration(device_id)
            .is_some_and(|registration| {
                registration.origin.switches() && registration.host_consent.is_some()
            })
    }

    /// This build's kit is in place on `device_id`: hided is asked to look
    /// at the device and switch it, with nobody asked. Asked for after each
    /// such pass, so a device that was left as it is, or whose move
    /// stopped, is looked at again when its helper next connects; never
    /// while its switch is landing.
    pub(super) fn move_when_ready(&mut self, device_id: &str) {
        let landing = self.device_switches.contains_key(device_id)
            || self.device_switch_saving.contains_key(device_id);
        if landing || !self.due_to_move(device_id) {
            return;
        }
        self.ask_move(device_id, false);
    }

    fn ask_move(&mut self, device_id: &str, own_folder: bool) {
        crate::diagnostic!(serde_json::json!({
            "component": "device_switch",
            "kind": "device_switch.due",
            "target": device_id,
            "own_folder": own_folder,
        }));
        match &self.device_mover {
            Some(mover) => mover.due(device_id, own_folder),
            // The daemon has not handed its mover in yet: asked then.
            None => {
                self.device_moves_ready.insert(device_id.to_owned());
            }
        }
    }

    /// How many terminals `device_id` lists, or none while the core has
    /// not read its session on the connection it has: what a switch with a
    /// folder of this computer's own would not take up.
    fn device_terminals(&self, device_id: &str) -> Option<usize> {
        let connected = self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == device_id && status.state == "connected");
        self.device_raw_sessions
            .get(device_id)
            .filter(|_| connected)
            .map(|session| super::remote_live_pane_ids(session).len())
    }

    /// The session of `device_id` was read: a device that waits for its
    /// terminals to close before it moves (`SwitchStep::Held`) is asked for
    /// once it lists none. One set lookup for each read, and nothing more
    /// for a device that does not wait.
    pub(super) fn move_when_closed(&mut self, device_id: &str) {
        if !self.device_moves_held.contains(device_id)
            || self.device_terminals(device_id) != Some(0)
        {
            return;
        }
        self.device_moves_held.remove(device_id);
        if self.due_to_move(device_id) {
            self.ask_move(device_id, true);
        }
    }

    /// Where the switch of `device_id` to the machine under `key` stands,
    /// for the daemon that waits on it: `Some(true)` once the registration
    /// is that machine's and saved, `Some(false)` once the core refused it
    /// or the device is gone, and `None` while it is under way. A paired
    /// machine alone is no switched device: the daemon answers a switch
    /// done, and hands the machine its projects, only on `Some(true)`.
    pub(crate) fn device_switch_landed(&self, device_id: &str, key: &str) -> Option<bool> {
        if self.device_switch_saving.contains_key(device_id) {
            return None;
        }
        let landed = self
            .device_registration(device_id)
            .is_some_and(|registration| registration.origin.peer_key() == Some(key));
        match landed || !self.device_switches.contains_key(device_id) {
            true => Some(landed),
            false => None,
        }
    }

    /// The helper device that is being switched to the machine under `key`.
    pub(super) fn switching_to(&self, key: &str) -> Option<String> {
        self.device_switches
            .iter()
            .find(|(_, switch)| switch.key == key)
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
        let key = match payload.step {
            SwitchStep::Looked => {
                // What the machine needs before it moves is the row's one
                // line; a device that moves, or that nothing the operator
                // does would move, shows none.
                let changed = match payload.needs {
                    Some(needs) => {
                        self.device_needs.insert(device_id.clone(), needs.clone()) != Some(needs)
                    }
                    None => self.device_needs.remove(&device_id).is_some(),
                };
                return changed && self.refresh_device_snapshots();
            }
            SwitchStep::Held => {
                self.device_moves_held.insert(device_id.clone());
                self.move_when_closed(&device_id);
                return false;
            }
            SwitchStep::Ended => {
                self.device_switches.remove(&device_id);
                return false;
            }
            SwitchStep::Connecting => payload.key.unwrap_or_default(),
        };
        self.device_switches.insert(
            device_id.clone(),
            DeviceSwitch {
                key: key.clone(),
                needs: payload.needs,
                own_folder: payload.own_folder,
            },
        );
        // A machine that was paired before the switch was asked for is
        // active already: no change of the paired machines will say so.
        self.peer_opens(&key) && self.switch_device_link(&device_id, &key)
    }

    /// The paired machine under `key` may be opened: the helper device
    /// `device_id` becomes that machine's device. The registration is
    /// rewritten and saved here, and nothing of the link changes until that
    /// save is acknowledged ([`Self::acknowledge_device_switch`]): a core
    /// that ended between the two would read the helper registration again
    /// and dial a folder the machine's own Hide already runs this core's
    /// node from.
    /// A node that dialed this core is taken over the same way once its
    /// machine runs a core of its own (D-46): what it ran for this core
    /// goes on in the folder its node kept. Such a node ran this core's
    /// node itself and holds no consent of this core's; the pairing its
    /// split made is the consent this core's node runs there under, as for
    /// any paired machine (`register_peer_device`), so the core dials it.
    pub(super) fn switch_device_link(&mut self, device_id: &str, key: &str) -> bool {
        // Rewritten already and waiting for its save: the paired machines
        // changing again meanwhile asks for nothing more.
        if self.device_switch_saving.contains_key(device_id) {
            return false;
        }
        // A folder of this computer's own takes up no terminal the helper
        // ran: a device that lists one, or whose session is not read, waits
        // on as a helper device until it lists none.
        let own_folder = self
            .device_switches
            .get(device_id)
            .is_some_and(|switch| switch.own_folder);
        if own_folder && self.device_terminals(device_id) != Some(0) {
            self.device_moves_held.insert(device_id.to_owned());
            self.not_moved(device_id, "terminals_open");
            return false;
        }
        let taken = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .any(|registration| registration.origin.peer_key() == Some(key));
        let paired = self.new_host_consent();
        let Some(registration) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == device_id && registration.origin.becomes_peer())
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
            self.not_moved(device_id, "device_already_connected");
            return false;
        };
        let was = std::mem::replace(
            &mut registration.origin,
            LinkOrigin::Peer {
                key: key.to_owned(),
                switched: true,
            },
        );
        if was == LinkOrigin::Inbound {
            registration.host_consent = Some(paired);
        }
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
    /// goes back to its helper when it did not. A save taken before the
    /// switch holds no such registration and settles nothing; the save the
    /// switch asked for follows it.
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
        self.device_moves_held.remove(device_id);
        // The line that said what the device needed before it moved goes;
        // what the Hide this switch started still needs takes its place.
        match saving.needs {
            Some(needs) => self.device_needs.insert(device_id.to_owned(), needs),
            None => self.device_needs.remove(device_id),
        };
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
    /// registration is the helper's again, as the store still has it, and
    /// its link was never touched. The next connection of its helper asks
    /// for the move again.
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
        if saving.was == LinkOrigin::Inbound {
            registration.host_consent = None;
        }
        registration.origin = saving.was;
        self.snapshot.ui_state.device_switch_opens.remove(device_id);
        crate::diagnostic!(serde_json::json!({
            "component": "device_switch",
            "kind": "device_switch.not_saved",
            "target": device_id,
        }));
        self.not_moved(device_id, "switch_not_saved");
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

    /// The switch of `device_id` stopped in the core for `reason`: the
    /// device stays on its helper with nothing on its row, the log says
    /// why, and its helper's next connection asks for the move again.
    fn not_moved(&mut self, device_id: &str, reason: &str) {
        self.device_switches.remove(device_id);
        crate::diagnostic!(serde_json::json!({
            "component": "device_switch",
            "kind": "device_switch.not_moved",
            "target": device_id,
            "reason": reason,
        }));
    }
}
