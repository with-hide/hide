//! Saved machine discovery reuses the device connector's machine boundary.
//! One request joins one owned, cancelable worker; no polling or SSH probes.

use super::*;
use crate::model::{HerdrProfileSnapshot, HerdrProfilesSnapshot};
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
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = runtime
                    .lock()
                    .map(|mut guard| guard.settle_herdr_profiles(id, listing))
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
            );
        }
        true
    }

    fn settle_herdr_profiles(
        &mut self,
        id: u64,
        answer: Result<HerdrProfileListing, String>,
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
                        let (connect_state, connect_error) = device_id
                            .as_deref()
                            .map(|device| self.profile_connection(device))
                            .unwrap_or_else(|| ("saved".to_owned(), None));
                        HerdrProfileSnapshot {
                            id: profile.id,
                            label: profile.label,
                            target: profile.target,
                            session: profile.session,
                            enabled: profile.enabled,
                            added_as,
                            problem,
                            device_id,
                            connect_state,
                            connect_error,
                        }
                    })
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

    fn profile_connection(&self, device: &str) -> (String, Option<String>) {
        match self
            .snapshot
            .status
            .remote
            .iter()
            .find(|r| r.target_id == device)
        {
            Some(status) if status.state == "connected" => ("connected".to_owned(), None),
            Some(status)
                if status.state == "error"
                    || status.state == "action_required"
                    || status.state == "unreachable"
                    || status.state == "stale" =>
            {
                ("failed".to_owned(), status.message.clone())
            }
            Some(status) if status.state == "connecting" => ("connecting".to_owned(), None),
            _ => ("added".to_owned(), None),
        }
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
            Some("Review and allow the helper installation before connecting")
        } else if !profile.enabled {
            Some("This profile is disabled in Herdr. Enable it there and retry discovery")
        } else if profile.problem.is_some() {
            Some("Fix the saved SSH target in Herdr or SSH configuration and retry discovery")
        } else if self.snapshot.status.herdr_profiles.state != "ready" {
            Some("Wait for saved machine discovery, then retry")
        } else {
            None
        };
        if let Some(reason) = refusal {
            let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
            row.connect_state = "failed".to_owned();
            row.connect_error = Some(reason.to_owned());
            return true;
        }
        let registration = self
            .profile_registration(&profile.target, &profile.session)
            .cloned();
        let registration = match registration {
            Some(registration) => {
                // Re-entry is inert. Only a failed connection or an absent
                // consent is an explicit retry of the selected device.
                let (state, _) = self.profile_connection(&registration.id);
                // Explicit import also protects a pre-existing default-session
                // registration on subsequent automatic kit work and reconnects.
                if registration.herdr_session.is_none() {
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
                if state == "failed" || state == "added" || registration.herdr_session.is_none() {
                    self.retry_remote_device(&registration.id);
                } else if state == "connected" {
                    self.retry_device_host_now(&registration.id);
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
            None => {
                if self.snapshot.ui_state.device_registrations.len() >= 64 {
                    let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
                    row.connect_state = "failed".to_owned();
                    row.connect_error = Some(
                        "The device limit is reached. Remove an unused device before connecting"
                            .to_owned(),
                    );
                    return true;
                }
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
                    row.connect_error = Some("This saved profile now points to another server. Remove its previous device registration before importing it again".to_owned());
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
                self.connect_remote_device(&registration);
                registration
            }
        };
        if self.remote_connections.contains_key(&registration.id) {
            if let Some(status) = self.snapshot.status.remote.iter_mut().find(|status| {
                status.target_id == registration.id && status.state == "not_connected"
            }) {
                status.state = "connecting".to_owned();
                status.message = None;
            }
        }
        // All aliases of this endpoint/session converge in the same frame.
        let same: Vec<usize> = self
            .snapshot
            .status
            .herdr_profiles
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                self.profile_registration(&row.target, &row.session)
                    .is_some_and(|r| r.id == registration.id)
            })
            .map(|(index, _)| index)
            .collect();
        let (state, error) = self.profile_connection(&registration.id);
        for index in same {
            let row = &mut self.snapshot.status.herdr_profiles.profiles[index];
            row.device_id = Some(registration.id.clone());
            row.added_as = Some(registration.label.clone());
            row.connect_state = state.clone();
            row.connect_error = error.clone();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::device::HerdrProfile;

    fn discovered() -> Runtime {
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
        );
        runtime
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
}
