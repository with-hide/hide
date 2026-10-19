//! Add device's host list: one listing at a time, run by a worker off the
//! lock and reported in `status.ssh_hosts`.
//!
//! The worker is the listing's only owner: its `ssh -G` children are owned
//! children of `hide_platform::process::run_to_end`, and the stop flag the job
//! carries is raised when the job is dropped, so a runtime that goes away
//! ends the listing with it. A request while one runs joins it, so the answer
//! is published once (engineering rule 11).

use super::*;
use crate::model::{SshHostSnapshot, SshHostsSnapshot};
use crate::remote::SshHostListing;
use std::sync::atomic::{AtomicBool, Ordering};

/// The listing the worker is running.
pub(super) struct SshHostsJob {
    id: u64,
    stop: Arc<AtomicBool>,
}

impl Drop for SshHostsJob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Runtime {
    /// Starts a listing of the account's Host entries, or joins the one that
    /// is running.
    pub(super) fn request_ssh_hosts(&mut self) -> bool {
        if self.ssh_hosts_job.is_some() {
            return false;
        }
        let (Some(home), Some(context)) = (self.home_path.clone(), self.worker_context.clone())
        else {
            // No account home or no worker: there is no config to read, so
            // the answer is an empty list, the same as a missing config.
            crate::diagnostic!(serde_json::json!({
                "component": "ssh_hosts",
                "kind": "ssh_hosts.unavailable",
            }));
            self.snapshot.status.ssh_hosts = SshHostsSnapshot {
                state: "ready".to_owned(),
                hosts: Vec::new(),
                truncated: false,
            };
            return true;
        };
        let registered: Vec<String> = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter_map(|registration| registration.origin.ssh_alias().map(str::to_owned))
            .collect();
        self.next_ssh_hosts_id = self.next_ssh_hosts_id.wrapping_add(1).max(1);
        let id = self.next_ssh_hosts_id;
        let stop = Arc::new(AtomicBool::new(false));
        self.ssh_hosts_job = Some(SshHostsJob {
            id,
            stop: Arc::clone(&stop),
        });
        self.snapshot.status.ssh_hosts.state = "loading".to_owned();
        let devices = Arc::clone(&self.devices);
        let spawned = thread::Builder::new()
            .name("herdr-core-ssh-hosts".to_owned())
            .spawn(move || {
                // A panic still settles the slot, which is the only one: a
                // listing left `loading` would join every later request.
                let listing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    devices.ssh_hosts(&home, &registered, &stop)
                }))
                .unwrap_or_default();
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = runtime
                    .lock()
                    .map(|mut guard| guard.settle_ssh_hosts(id, &listing))
                    .unwrap_or(false);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            crate::diagnostic!(serde_json::json!({
                "component": "ssh_hosts",
                "kind": "ssh_hosts.not_started",
                "message": error.to_string(),
            }));
            self.settle_ssh_hosts(id, &SshHostListing::default());
        }
        true
    }

    /// Publishes a listing's answer, naming the device that already holds each
    /// alias or reaches the same address. Only the running listing settles.
    pub(crate) fn settle_ssh_hosts(&mut self, id: u64, listing: &SshHostListing) -> bool {
        if self.ssh_hosts_job.as_ref().is_none_or(|job| job.id != id) {
            return false;
        }
        self.ssh_hosts_job = None;
        let registrations = &self.snapshot.ui_state.device_registrations;
        let address_of = |alias: &str| {
            listing
                .registered
                .iter()
                .find(|(registered, _)| registered == alias)
                .map(|(_, address)| address)
        };
        let hosts = listing
            .entries
            .iter()
            .map(|entry| {
                let added_as = registrations
                    .iter()
                    .find(|registration| {
                        // Add device's SSH aliases select the default server;
                        // another named session on the host is a separate row.
                        if registration.herdr_socket_path.is_some()
                            || registration
                                .herdr_session
                                .as_deref()
                                .is_some_and(|session| session != "default")
                        {
                            return false;
                        }
                        let Some(alias) = registration.origin.ssh_alias() else {
                            return false;
                        };
                        alias == entry.alias
                            || entry
                                .address
                                .as_ref()
                                .ok()
                                .is_some_and(|address| address_of(alias) == Some(address))
                    })
                    .map(|registration| registration.label.clone());
                SshHostSnapshot {
                    alias: entry.alias.clone(),
                    address: entry.address.as_ref().ok().map(|address| address.display()),
                    added_as,
                    problem: entry
                        .address
                        .as_ref()
                        .err()
                        .map(|problem| problem.code().to_owned()),
                }
            })
            .collect();
        self.snapshot.status.ssh_hosts = SshHostsSnapshot {
            state: "ready".to_owned(),
            hosts,
            truncated: listing.truncated,
        };
        true
    }
}
