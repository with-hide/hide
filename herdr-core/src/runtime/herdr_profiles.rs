//! Saved machine discovery reuses the device connector's machine boundary.
//! One request joins one owned, cancelable worker; no polling or SSH probes.
//! Beside the saved profiles the first-run step lists this PC's default WSL
//! distribution (PRD wsl-device B1, D-13), read from the account's registry
//! as Add device reads it, so listing never starts WSL; it connects as a WSL
//! device and comes to the front (B2, D-12).

use super::*;
use crate::model::{HerdrProfileRefusal, HerdrProfileSnapshot, HerdrProfilesSnapshot};
use crate::remote::WslListing;
use hide_node_link::device::HerdrProfileListing;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct HerdrProfilesJob {
    id: u64,
    stop: Arc<AtomicBool>,
}

impl Drop for HerdrProfilesJob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Runtime {
    pub(super) fn request_herdr_profiles(&mut self) -> bool {
        if self.herdr_profiles_job.is_some() {
            return false;
        }
        let (Some(home), Some(context)) = (self.home_path.clone(), self.worker_context.clone())
        else {
            self.snapshot.status.herdr_profiles.state = "failed".to_owned();
            self.snapshot.status.herdr_profiles.error =
                Some("Saved Herdr machines are unavailable. Reconnect and retry".to_owned());
            return true;
        };
        let registered: Vec<String> = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter_map(|r| r.origin.ssh_alias().map(str::to_owned))
            .collect();
        self.next_ssh_hosts_id = self.next_ssh_hosts_id.wrapping_add(1).max(1);
        let id = self.next_ssh_hosts_id;
        let stop = Arc::new(AtomicBool::new(false));
        self.herdr_profiles_job = Some(HerdrProfilesJob {
            id,
            stop: Arc::clone(&stop),
        });
        self.snapshot.status.herdr_profiles.state = "loading".to_owned();
        self.snapshot.status.herdr_profiles.error = None;
        let devices = Arc::clone(&self.devices);
        let spawned = thread::Builder::new()
            .name("herdr-core-saved-machines".to_owned())
            .spawn(move || {
                let listing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    devices.herdr_profiles(&home, &registered, &stop)
                }))
                .unwrap_or_else(|_| {
                    Err("Saved machine discovery stopped unexpectedly. Retry".to_owned())
                });
                let wsl = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    devices.wsl_distributions()
                }))
                .unwrap_or_else(|_| Err("The WSL listing ended unexpectedly".to_owned()));
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = runtime
                    .lock()
                    .map(|mut guard| guard.settle_herdr_profiles(id, listing, &wsl))
                    .unwrap_or(false);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"herdr_profiles","kind":"herdr_profiles.not_started","request":id,"reason":error.to_string()})
            );
            self.settle_herdr_profiles(
                id,
                Err("Saved machine discovery could not start. Retry".to_owned()),
                &Ok(Vec::new()),
            );
        }
        true
    }

    fn settle_herdr_profiles(
        &mut self,
        id: u64,
        answer: Result<HerdrProfileListing, String>,
        wsl: &WslListing,
    ) -> bool {
        if self
            .herdr_profiles_job
            .as_ref()
            .is_none_or(|job| job.id != id)
        {
            return false;
        }
        self.herdr_profiles_job = None;
        match answer {
            Err(error) => {
                self.snapshot.status.herdr_profiles.state = "failed".to_owned();
                self.snapshot.status.herdr_profiles.error = Some(error);
            }
            Ok(listing) => {
                self.herdr_profile_endpoints = listing.endpoints;
                let profiles = listing
                    .profiles
                    .into_iter()
                    .map(|profile| {
                        let existing = self.profile_registration(&profile.target, &profile.session);
                        let (device_id, added_as) = existing
                            .map(|r| (Some(r.id.clone()), Some(r.label.clone())))
                            .unwrap_or_default();
                        let problem = if !profile.enabled {
                            Some("disabled".to_owned())
                        } else if !hide_node_link::device::valid_herdr_session(&profile.session) {
                            Some("invalid_session".to_owned())
                        } else {
                            self.herdr_profile_endpoints
                                .get(&profile.target)
                                .and_then(|endpoint| endpoint.as_ref().err())
                                .map(|_| "invalid_target".to_owned())
                        };
                        let connect_state = device_id
                            .as_deref()
                            .map(|device| self.profile_connection(device))
                            .unwrap_or_else(|| "saved".to_owned());
                        HerdrProfileSnapshot {
                            id: profile.id,
                            label: profile.label,
                            target: profile.target,
                            session: profile.session,
                            wsl_distro: None,
                            enabled: profile.enabled,
                            added_as,
                            problem,
                            device_id,
                            connect_state,
                            connect_refusal: None,
                        }
                    })
                    .collect::<Vec<_>>();
                let profiles = self
                    .default_wsl_row(wsl)
                    .into_iter()
                    .chain(profiles)
                    .collect();
                self.snapshot.status.herdr_profiles = HerdrProfilesSnapshot {
                    state: "ready".to_owned(),
                    error: None,
                    profiles,
                    truncated: listing.truncated,
                };
                self.settle_first_run_devices();
            }
        }
        true
    }

    /// A custom socket cannot be equated to a named server without evidence.
    fn profile_registration(
        &self,
        target: &str,
        session: &str,
    ) -> Option<&crate::model::DeviceRegistration> {
        let endpoint = self.herdr_profile_endpoints.get(target)?.as_ref().ok()?;
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| {
                registration.herdr_socket_path.is_none()
                    && registration.herdr_session.as_deref().unwrap_or("default") == session
                    && registration
                        .origin
                        .ssh_alias()
                        .and_then(|alias| self.herdr_profile_endpoints.get(alias))
                        .and_then(|endpoint| endpoint.as_ref().ok())
                        == Some(endpoint)
            })
    }

    fn profile_connection(&self, device: &str) -> String {
        match self
            .snapshot
            .status
            .remote
            .iter()
            .find(|r| r.target_id == device)
        {
            Some(status) if status.state == "connected" => "connected".to_owned(),
            Some(status)
                if status.state == "error"
                    || status.state == "action_required"
                    || status.state == "unreachable"
                    || status.state == "stale" =>
            {
                "failed".to_owned()
            }
            Some(status) if status.state == "connecting" => "connecting".to_owned(),
            _ => "added".to_owned(),
        }
    }

    /// The step's row for this PC's default WSL distribution (PRD wsl-device
    /// B1, B29), or none without WSL. One that cannot connect (WSL 1, its
    /// setup unfinished) is a row with that problem. Another distribution is
    /// no row: Add device lists it (D-13). A listing that could not be read
    /// shows no row and is logged; Add device says so where it can be read
    /// again (B25).
    fn default_wsl_row(&self, wsl: &WslListing) -> Option<HerdrProfileSnapshot> {
        let distributions = match wsl {
            Ok(distributions) => distributions,
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "herdr_profiles",
                    "kind": "wsl.listing_failed",
                    "message": reason,
                }));
                return None;
            }
        };
        let distribution = distributions.iter().find(|found| found.default)?;
        let registration = self.wsl_registration(&distribution.name);
        let device_id = registration.map(|registration| registration.id.clone());
        let problem = if distribution.setup_unfinished {
            Some("setup_unfinished")
        } else if distribution.wsl_version != 2 {
            Some("wsl1")
        } else {
            None
        };
        Some(HerdrProfileSnapshot {
            id: format!("wsl:{}", distribution.name),
            label: super::devices::wsl_device_label(&distribution.name),
            target: distribution.name.clone(),
            session: "default".to_owned(),
            wsl_distro: Some(distribution.name.clone()),
            enabled: true,
            added_as: registration.map(|registration| registration.label.clone()),
            problem: problem.map(str::to_owned),
            connect_state: device_id
                .as_deref()
                .map(|device| self.profile_connection(device))
                .unwrap_or_else(|| "saved".to_owned()),
            device_id,
            connect_refusal: None,
        })
    }

    fn wsl_registration(&self, distro: &str) -> Option<&crate::model::DeviceRegistration> {
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.origin.wsl_distro() == Some(distro))
    }

    pub(super) fn connect_herdr_profile(&mut self, profile_id: &str, consent: bool) -> bool {
        let Some(index) = self
            .snapshot
            .status
            .herdr_profiles
            .profiles
            .iter()
            .position(|p| p.id == profile_id)
        else {
            self.set_error(
                "device.profile_unknown",
                "Saved machine profile is unavailable. Refresh the machine list and retry",
                false,
            );
            return true;
        };
        let profile = self.snapshot.status.herdr_profiles.profiles[index].clone();
        let refusal = if !consent {
            Some(HerdrProfileRefusal::NoConsent)
        } else if !profile.enabled {
            Some(HerdrProfileRefusal::Disabled)
        } else if profile.problem.is_some() {
            Some(HerdrProfileRefusal::InvalidTarget)
        } else if self.snapshot.status.herdr_profiles.state != "ready" {
            Some(HerdrProfileRefusal::DiscoveryPending)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
            row.connect_state = "failed".to_owned();
            row.connect_refusal = Some(refusal);
            return true;
        }
        let registration = match profile.wsl_distro.as_deref() {
            Some(distro) => self.wsl_registration(distro),
            None => self.profile_registration(&profile.target, &profile.session),
        }
        .cloned();
        if registration.is_none() && self.snapshot.ui_state.device_registrations.len() >= 64 {
            let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
            row.connect_state = "failed".to_owned();
            row.connect_refusal = Some(HerdrProfileRefusal::DeviceLimit);
            return true;
        }
        let registration = match (registration, profile.wsl_distro.as_deref()) {
            (Some(registration), _) => {
                // Re-entry is inert. Only a failed connection or an absent
                // consent is an explicit retry of the selected device.
                let state = self.profile_connection(&registration.id);
                // Explicit import also protects a pre-existing default-session
                // registration on subsequent automatic kit work and reconnects.
                // A WSL device reaches its account's own Herdr, so it names none.
                let import = profile.wsl_distro.is_none() && registration.herdr_session.is_none();
                if import {
                    if let Some(stored) = self
                        .snapshot
                        .ui_state
                        .device_registrations
                        .iter_mut()
                        .find(|r| r.id == registration.id)
                    {
                        stored.herdr_session = Some(profile.session.clone());
                    }
                    self.persist_current_ui_state();
                    self.disconnect_remote_device(
                        &registration.id,
                        super::devices::Retiring::Retried,
                    );
                }
                if registration.host_consent.is_none() {
                    self.set_host_consent(&registration.id, true);
                }
                if state == "failed" || state == "added" || import {
                    self.retry_remote_device(&registration.id);
                } else if state == "connected" {
                    self.retry_device_host_now(
                        &registration.id,
                        super::hosts::HostAttempt::Operator,
                    );
                    let kit = self.kit_view(&registration.id);
                    if kit.unavailable.is_some() {
                        self.queue_device_kit(
                            &registration.id,
                            super::kit::KitJob::Apply(hide_kit::Scope::automatic()),
                        );
                    } else if kit.refused.is_some() || kit.offers_reinstall {
                        self.request_kit_reinstall(&registration.id, None, None);
                    }
                }
                registration
            }
            // The press is the install consent, as Add device's is (B2).
            (None, Some(distro)) => match self.register_wsl_device(distro, true) {
                Ok(registration) => registration,
                Err(refusal) => {
                    self.set_error(refusal.code, refusal.message, false);
                    let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
                    row.connect_state = "failed".to_owned();
                    return true;
                }
            },
            (None, None) => {
                use sha2::{Digest, Sha256};
                let digest = format!("{:x}", Sha256::digest(profile.id.as_bytes()));
                let id = format!("herdr-{}", &digest[..32]);
                if self
                    .snapshot
                    .ui_state
                    .device_registrations
                    .iter()
                    .any(|r| r.id == id)
                {
                    let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
                    row.connect_state = "failed".to_owned();
                    row.connect_refusal = Some(HerdrProfileRefusal::ProfileMoved);
                    return true;
                }
                let registration = crate::model::DeviceRegistration {
                    id,
                    label: profile.label.clone(),
                    origin: crate::model::LinkOrigin::Dialed {
                        ssh_alias: profile.target.clone(),
                    },
                    herdr_socket_path: None,
                    herdr_session: Some(profile.session.clone()),
                    host_consent: Some(self.new_host_consent()),
                };
                self.snapshot
                    .ui_state
                    .device_registrations
                    .push(registration.clone());
                self.rebuild_device_rows();
                self.persist_current_ui_state();
                self.connect_remote_device(&registration, super::hosts::HostAttempt::Operator);
                registration
            }
        };
        // The distribution's tile comes to the front with its press, so its
        // projects and agents are the ones the next steps show (D-12).
        if profile.wsl_distro.is_some() {
            self.bring_device_forward(registration.id.clone());
        }
        if self.remote_connections.contains_key(&registration.id)
            && let Some(status) = self.snapshot.status.remote.iter_mut().find(|status| {
                status.target_id == registration.id && status.state == "not_connected"
            })
        {
            status.state = "connecting".to_owned();
            status.message = None;
        }
        // All aliases of this endpoint/session converge in the same frame.
        let same: Vec<usize> = self
            .snapshot
            .status
            .herdr_profiles
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, row)| match row.wsl_distro.as_deref() {
                Some(distro) => registration.origin.wsl_distro() == Some(distro),
                None => self
                    .profile_registration(&row.target, &row.session)
                    .is_some_and(|r| r.id == registration.id),
            })
            .map(|(index, _)| index)
            .collect();
        let state = self.profile_connection(&registration.id);
        for index in same {
            let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
            row.device_id = Some(registration.id.clone());
            row.added_as = Some(registration.label.clone());
            row.connect_state = state.clone();
            row.connect_refusal = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::device::{HerdrProfile, WslDistribution};

    fn discovered() -> Runtime {
        discovered_with(&Ok(Vec::new()))
    }

    /// The saved profiles beside `wsl`, this PC's distributions.
    fn discovered_with(wsl: &WslListing) -> Runtime {
        let mut runtime = super::super::tests::runtime();
        runtime.herdr_profiles_job = Some(HerdrProfilesJob {
            id: 1,
            stop: Arc::new(AtomicBool::new(false)),
        });
        runtime.settle_herdr_profiles(
            1,
            Ok(HerdrProfileListing {
                profiles: [
                    ("first", "grab@box", "review", true),
                    ("alias", "other-alias", "review", true),
                    ("second", "grab@box", "default", true),
                    ("disabled", "off", "review", false),
                ]
                .into_iter()
                .map(|(id, target, session, enabled)| HerdrProfile {
                    id: id.to_owned(),
                    label: "Shared name".to_owned(),
                    target: target.to_owned(),
                    session: session.to_owned(),
                    enabled,
                })
                .collect(),
                endpoints: [
                    ("grab@box", "grab@box:22"),
                    ("other-alias", "grab@box:22"),
                    ("off", "grab@off:22"),
                ]
                .into_iter()
                .map(|(target, endpoint)| (target.to_owned(), Ok(endpoint.to_owned())))
                .collect(),
                truncated: false,
            }),
            wsl,
        );
        runtime
    }

    fn distro(name: &str, wsl_version: u8, default: bool) -> WslDistribution {
        WslDistribution {
            name: name.to_owned(),
            wsl_version,
            setup_unfinished: false,
            default,
        }
    }

    fn connect(runtime: &mut Runtime, profile: &str, consent: bool) {
        assert!(
            runtime.dispatch_json(
                &serde_json::to_vec(&serde_json::json!({
                    "schema_version": SCHEMA_VERSION, "kind":"herdr_profile_connect",
                    "payload":{"profile_id":profile,"host_consent":consent}
                }))
                .unwrap()
            )
        );
    }

    #[test]
    fn discovery_and_refused_consent_register_nothing() {
        let mut runtime = discovered();
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert!(runtime.snapshot.status.remote.is_empty());
        connect(&mut runtime, "first", false);
        connect(&mut runtime, "disabled", true);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert!(runtime.snapshot.status.remote.is_empty());
        assert_eq!(
            runtime.snapshot.status.herdr_profiles.profiles[0].connect_state,
            "failed"
        );
        assert_eq!(
            runtime.snapshot.status.herdr_profiles.profiles[3]
                .problem
                .as_deref(),
            Some("disabled")
        );
    }

    #[test]
    fn imports_keep_original_routes_deduplicate_aliases_and_separate_sessions() {
        let mut runtime = discovered();
        connect(&mut runtime, "first", true);
        connect(&mut runtime, "alias", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
        connect(&mut runtime, "second", true);
        let registrations = &runtime.snapshot.ui_state.device_registrations;
        assert_eq!(registrations.len(), 2);
        assert_eq!(registrations[0].origin.ssh_alias(), Some("grab@box"));
        assert_eq!(registrations[0].herdr_session.as_deref(), Some("review"));
        assert_eq!(registrations[1].herdr_session.as_deref(), Some("default"));
        assert_ne!(registrations[0].id, registrations[1].id);
        let rows = &runtime.snapshot.status.herdr_profiles.profiles;
        assert_eq!(rows[0].device_id, rows[1].device_id);
        assert_ne!(rows[0].device_id, rows[2].device_id);
        let stored = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
        assert_eq!(stored["device_registrations"][0]["herdr_session"], "review");
    }

    /// PRD wsl-device B1, D-13: the step lists this PC's default WSL
    /// distribution first, as the device it would be, and no other.
    #[test]
    fn the_default_wsl_distribution_is_the_steps_first_row_and_no_other_is() {
        let runtime = discovered_with(&Ok(vec![
            distro("Debian", 2, false),
            distro("Ubuntu", 2, true),
        ]));
        let rows = &runtime.snapshot.status.herdr_profiles.profiles;
        assert_eq!(
            rows.len(),
            5,
            "the four saved profiles and one distribution"
        );
        let wsl = &rows[0];
        assert_eq!(
            (
                wsl.id.as_str(),
                wsl.label.as_str(),
                wsl.wsl_distro.as_deref()
            ),
            ("wsl:Ubuntu", "WSL Ubuntu", Some("Ubuntu"))
        );
        assert_eq!(
            (wsl.problem.as_deref(), wsl.connect_state.as_str()),
            (None, "saved")
        );
        assert!(rows[1..].iter().all(|row| row.wsl_distro.is_none()));
        // Listing installs and registers nothing (B2).
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        let wire = serde_json::to_value(&rows[1]).unwrap();
        assert!(
            wire.get("wsl_distro").is_none(),
            "a saved profile's wire is unchanged"
        );
    }

    /// B29, D-14: a default distribution that cannot connect is its row with
    /// why; no WSL, no default or an unreadable listing is no row.
    #[test]
    fn a_default_wsl_1_or_unfinished_distribution_says_why_and_no_wsl_is_no_row() {
        let first = |wsl: WslListing| {
            discovered_with(&wsl)
                .snapshot
                .status
                .herdr_profiles
                .profiles[0]
                .clone()
        };
        assert_eq!(
            first(Ok(vec![distro("Legacy", 1, true)]))
                .problem
                .as_deref(),
            Some("wsl1")
        );
        let mut fresh = distro("Fresh", 2, true);
        fresh.setup_unfinished = true;
        assert_eq!(
            first(Ok(vec![fresh])).problem.as_deref(),
            Some("setup_unfinished")
        );
        for wsl in [
            Ok(Vec::new()),
            Ok(vec![distro("Debian", 2, false)]),
            Err("the registry could not be read".to_owned()),
        ] {
            assert_eq!(first(wsl).wsl_distro, None);
        }
        // A WSL 1 row sent to connect anyway is refused and registers nothing.
        let mut runtime = discovered_with(&Ok(vec![distro("Legacy", 1, true)]));
        connect(&mut runtime, "wsl:Legacy", true);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert_eq!(
            runtime.snapshot.status.herdr_profiles.profiles[0].connect_refusal,
            Some(HerdrProfileRefusal::InvalidTarget)
        );
    }

    /// macOS, Linux and a PC without WSL: the step is the saved profiles,
    /// exactly as before.
    #[test]
    fn without_wsl_the_step_is_the_saved_profiles_alone() {
        let runtime = discovered();
        let rows = &runtime.snapshot.status.herdr_profiles.profiles;
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["first", "alias", "second", "disabled"]
        );
        assert!(rows.iter().all(|row| row.wsl_distro.is_none()));
    }

    /// B2, D-08, D-12: pressing connect with the row checked registers the
    /// distribution as its WSL device with the install consent and brings
    /// its tile to the front; without consent nothing is registered.
    #[test]
    fn connecting_the_wsl_row_registers_its_device_with_consent_and_selects_it() {
        let mut runtime = discovered_with(&Ok(vec![distro("Ubuntu", 2, true)]));
        connect(&mut runtime, "wsl:Ubuntu", false);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        connect(&mut runtime, "wsl:Ubuntu", true);
        let id = super::super::devices::wsl_device_id("Ubuntu");
        let registrations = &runtime.snapshot.ui_state.device_registrations;
        assert_eq!(registrations.len(), 1);
        let registration = &registrations[0];
        assert_eq!(
            (
                registration.id.as_str(),
                registration.label.as_str(),
                registration.origin.wsl_distro()
            ),
            (id.as_str(), "WSL Ubuntu", Some("Ubuntu"))
        );
        assert!(registration.host_consent.is_some());
        assert_eq!(registration.herdr_session, None);
        assert_eq!(
            runtime.snapshot.navigator.focused_device_id.as_deref(),
            Some(id.as_str())
        );
        let row = &runtime.snapshot.status.herdr_profiles.profiles[0];
        assert_eq!(
            (row.device_id.as_deref(), row.added_as.as_deref()),
            (Some(id.as_str()), Some("WSL Ubuntu"))
        );
        // The saved profiles keep their own routes.
        assert!(
            runtime.snapshot.status.herdr_profiles.profiles[1..]
                .iter()
                .all(|row| row.device_id.is_none())
        );
        // Pressing again is the same device, not a second one.
        connect(&mut runtime, "wsl:Ubuntu", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
    }
}
