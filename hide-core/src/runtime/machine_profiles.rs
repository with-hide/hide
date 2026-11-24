//! The first-run Devices step lists the machines this computer already
//! reaches: the account's ssh config Hosts, read as Add device reads them
//! (`ssh -G`, which never connects), and this PC's default WSL distribution
//! (PRD wsl-device B1, D-13), read from the account's registry as Add device
//! reads it, so listing never starts WSL; it connects as a WSL device and
//! comes to the front (B2, D-12). One request joins one owned, cancelable
//! worker; no polling or SSH probes. Herdr's own saved machines are not read:
//! Hide runs no Herdr (PRD engine-bundle-4-herdr-removal), and what moves
//! from Herdr is its session, through the import.

use super::*;
use crate::model::{MachineProfileRefusal, MachineProfileSnapshot, MachineProfilesSnapshot};
use crate::remote::{SshHostListing, WslListing};
use std::sync::atomic::{AtomicBool, Ordering};

/// The id prefix of a Host row.
const SSH_HOST_ROW: &str = "ssh:";

pub(super) struct MachineProfilesJob {
    id: u64,
    stop: Arc<AtomicBool>,
}

impl Drop for MachineProfilesJob {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Runtime {
    pub(super) fn request_machine_profiles(&mut self) -> bool {
        if self.machine_profiles_job.is_some() {
            return false;
        }
        let (Some(home), Some(context)) = (self.home_path.clone(), self.worker_context.clone())
        else {
            self.snapshot.status.machine_profiles.state = "failed".to_owned();
            self.snapshot.status.machine_profiles.error =
                Some("This computer's machines are unavailable. Reconnect and retry".to_owned());
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
        self.machine_profiles_job = Some(MachineProfilesJob {
            id,
            stop: Arc::clone(&stop),
        });
        self.snapshot.status.machine_profiles.state = "loading".to_owned();
        self.snapshot.status.machine_profiles.error = None;
        let devices = Arc::clone(&self.devices);
        let spawned = thread::Builder::new()
            .name("hide-core-saved-machines".to_owned())
            .spawn(move || {
                let hosts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    devices.ssh_hosts(&home, &registered, &stop)
                }))
                .map_err(|_| "The ssh config listing ended unexpectedly. Retry".to_owned());
                let wsl = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    devices.wsl_distributions()
                }))
                .unwrap_or_else(|_| Err("The WSL listing ended unexpectedly".to_owned()));
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = runtime
                    .lock()
                    .map(|mut guard| guard.settle_machine_profiles(id, hosts, &wsl))
                    .unwrap_or(false);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            crate::diagnostic!(
                serde_json::json!({"component":"machine_profiles","kind":"machine_profiles.not_started","request":id,"reason":error.to_string()})
            );
            self.settle_machine_profiles(
                id,
                Err("Saved machine discovery could not start. Retry".to_owned()),
                &Ok(Vec::new()),
            );
        }
        true
    }

    fn settle_machine_profiles(
        &mut self,
        id: u64,
        hosts: Result<SshHostListing, String>,
        wsl: &WslListing,
    ) -> bool {
        if self
            .machine_profiles_job
            .as_ref()
            .is_none_or(|job| job.id != id)
        {
            return false;
        }
        self.machine_profiles_job = None;
        // A listing that could not be read proves nothing about WSL, so the
        // step keeps its failed state with Retry rather than reading empty
        // and being skipped (B1, B25); only a listing read empty is none.
        // A config that exists but could not be read says nothing about
        // its Hosts, so the step keeps Retry rather than reading empty.
        let answer = hosts.and_then(|hosts| {
            if hosts.unreadable {
                crate::diagnostic!(serde_json::json!({
                    "component": "machine_profiles",
                    "kind": "ssh_hosts.unreadable",
                    "request": id,
                }));
                return Err("This computer's ssh config could not be read. Retry".to_owned());
            }
            Ok(hosts)
        });
        let answer = answer.and_then(|hosts| match wsl {
            Ok(distributions) => Ok((hosts, distributions)),
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "machine_profiles",
                    "kind": "wsl.listing_failed",
                    "request": id,
                    "message": reason,
                }));
                Err("This PC's WSL distributions could not be read. Retry".to_owned())
            }
        });
        match answer {
            Err(error) => {
                self.snapshot.status.machine_profiles.state = "failed".to_owned();
                self.snapshot.status.machine_profiles.error = Some(error);
            }
            Ok((hosts, distributions)) => {
                // Every Host alias leads where `ssh -G` said, spelled as a
                // registration's endpoint is, so a device registered under
                // any alias of one server is found by the same key.
                self.ssh_host_endpoints = hosts
                    .entries
                    .iter()
                    .filter_map(|entry| Some((&entry.alias, entry.address.as_ref().ok()?)))
                    .chain(
                        hosts
                            .registered
                            .iter()
                            .map(|(alias, address)| (alias, address)),
                    )
                    .map(|(alias, address)| (alias.clone(), address.endpoint_key()))
                    .collect();
                // One row per server, by the user, host and port ssh
                // resolves to, in the config's order: a second alias of a
                // server is named by the first. A Host that cannot resolve
                // is its own row.
                let mut rows: Vec<(String, MachineProfileSnapshot)> = Vec::new();
                for entry in &hosts.entries {
                    let key = match &entry.address {
                        Ok(address) => address.endpoint_key(),
                        Err(_) => format!("alias {}", entry.alias),
                    };
                    if rows.iter().any(|(found, _)| *found == key) {
                        continue;
                    }
                    let display = entry.address.as_ref().ok().map(|address| address.display());
                    rows.push((key, self.ssh_host_row(entry, display)));
                }
                let profiles = self
                    .default_wsl_row(distributions)
                    .into_iter()
                    .chain(rows.into_iter().map(|(_, row)| row))
                    .collect();
                self.snapshot.status.machine_profiles = MachineProfilesSnapshot {
                    state: "ready".to_owned(),
                    error: None,
                    profiles,
                    truncated: hosts.truncated,
                };
                self.settle_first_run_devices();
            }
        }
        true
    }

    /// The registered device on the default server `alias` leads to, by Add
    /// device's own decision (`ssh_registration`), where an alias leads where
    /// this listing resolved it.
    fn endpoint_registration(&self, alias: &str) -> Option<&crate::model::DeviceRegistration> {
        self.ssh_registration(alias, |alias| self.ssh_host_endpoints.get(alias).cloned())
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
    /// no row: Add device lists it (D-13).
    fn default_wsl_row(
        &self,
        distributions: &[hide_node_link::device::WslDistribution],
    ) -> Option<MachineProfileSnapshot> {
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
        Some(MachineProfileSnapshot {
            id: format!("wsl:{}", distribution.name),
            label: super::devices::wsl_device_label(&distribution.name),
            target: distribution.name.clone(),
            ssh_host: None,
            address: None,
            wsl_distro: Some(distribution.name.clone()),
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

    /// The step's row for a Host: it connects as Add device's Host does, on
    /// the default server. One a device already uses
    /// is that device; one `ssh -G` could not resolve says why, as Add device
    /// does.
    fn ssh_host_row(
        &self,
        entry: &hide_node_link::device::SshHostEntry,
        address: Option<String>,
    ) -> MachineProfileSnapshot {
        let mut row = MachineProfileSnapshot {
            id: format!("{SSH_HOST_ROW}{}", entry.alias),
            label: entry.alias.clone(),
            target: entry.alias.clone(),
            ssh_host: Some(entry.alias.clone()),
            address,
            wsl_distro: None,
            added_as: None,
            problem: entry
                .address
                .as_ref()
                .err()
                .map(|problem| problem.code().to_owned()),
            device_id: None,
            connect_state: "saved".to_owned(),
            connect_refusal: None,
        };
        if let Some(registration) = self.row_registration(&row) {
            row.connect_state = self.profile_connection(&registration.id);
            row.device_id = Some(registration.id.clone());
            row.added_as = Some(registration.label.clone());
        }
        row
    }

    /// The registered device a row is: its distribution's, or the one on
    /// the same server, found by where its alias leads, so a device
    /// registered under another alias of that server, during this Connect
    /// too, is this row's (engineering rule 11). An alias nothing resolved is
    /// matched by its own name.
    fn row_registration(
        &self,
        row: &MachineProfileSnapshot,
    ) -> Option<&crate::model::DeviceRegistration> {
        if let Some(distro) = row.wsl_distro.as_deref() {
            return self.wsl_registration(distro);
        }
        self.endpoint_registration(&row.target)
    }

    fn wsl_registration(&self, distro: &str) -> Option<&crate::model::DeviceRegistration> {
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.origin.wsl_distro() == Some(distro))
    }

    /// Connects the step's row `profile_id`, and remembers the attempt it
    /// started, which the step's sign-in withdrawal may end.
    pub(super) fn connect_machine_profile(&mut self, profile_id: &str, consent: bool) -> bool {
        let before: BTreeMap<String, u64> = self
            .device_hosts
            .iter()
            .map(|(id, host)| (id.clone(), host.generation))
            .collect();
        let changed = self.connect_machine_profile_row(profile_id, consent);
        for (id, host) in &self.device_hosts {
            if before.get(id) != Some(&host.generation) && host.wake == crate::remote::Wake::Allowed
            {
                self.first_run_attempts.insert(id.clone(), host.generation);
            }
        }
        changed
    }

    fn connect_machine_profile_row(&mut self, profile_id: &str, consent: bool) -> bool {
        let (changed, read) = self
            .change_look(|runtime| runtime.connect_machine_profile_row_steps(profile_id, consent));
        changed || read
    }

    /// The steps of `connect_machine_profile_row`, which runs them as one selection (`look.rs`).
    fn connect_machine_profile_row_steps(&mut self, profile_id: &str, consent: bool) -> bool {
        let Some(index) = self
            .snapshot
            .status
            .machine_profiles
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
        let profile = self.snapshot.status.machine_profiles.profiles[index].clone();
        let refusal = if !consent {
            Some(MachineProfileRefusal::NoConsent)
        } else if profile.problem.is_some() {
            Some(MachineProfileRefusal::InvalidTarget)
        } else if self.snapshot.status.machine_profiles.state != "ready" {
            Some(MachineProfileRefusal::DiscoveryPending)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            let row = &mut self.snapshot.status.machine_profiles.profiles[index];
            row.connect_state = "failed".to_owned();
            row.connect_refusal = Some(refusal);
            return true;
        }
        let registration = self.row_registration(&profile).cloned();
        if registration.is_none() && self.snapshot.ui_state.device_registrations.len() >= 64 {
            let row = &mut self.snapshot.status.machine_profiles.profiles[index];
            row.connect_state = "failed".to_owned();
            row.connect_refusal = Some(MachineProfileRefusal::DeviceLimit);
            return true;
        }
        let registration = match (registration, profile.wsl_distro.as_deref()) {
            (Some(registration), _) => {
                // Re-entry is inert. Only a failed connection or an absent
                // consent is an explicit retry of the selected device.
                let state = self.profile_connection(&registration.id);
                if registration.host_consent.is_none() {
                    self.set_host_consent(&registration.id, true);
                }
                if state == "failed" || state == "added" {
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
                    let row = &mut self.snapshot.status.machine_profiles.profiles[index];
                    row.connect_state = "failed".to_owned();
                    return true;
                }
            },
            // A Host registers as Add device registers it (B50): the alias
            // and its name, the default server, and the press's consent.
            (None, None) => {
                match self.register_device(super::events::RegisterDevicePayload {
                    label: profile.label.clone(),
                    ssh_alias: profile.target.clone(),
                    wsl_distro: None,
                    host_consent: true,
                }) {
                    Ok(id) => {
                        let registrations = &self.snapshot.ui_state.device_registrations;
                        match registrations.iter().find(|r| r.id == id) {
                            Some(registration) => registration.clone(),
                            None => return true,
                        }
                    }
                    Err(refusal) => {
                        self.set_error(refusal.code, refusal.message, false);
                        let row = &mut self.snapshot.status.machine_profiles.profiles[index];
                        row.connect_state = "failed".to_owned();
                        return true;
                    }
                }
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
            .machine_profiles
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                self.row_registration(row)
                    .is_some_and(|r| r.id == registration.id)
            })
            .map(|(index, _)| index)
            .collect();
        let state = self.profile_connection(&registration.id);
        for index in same {
            let row = &mut self.snapshot.status.machine_profiles.profiles[index];
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
    use hide_node_link::device::{SshHostEntry, WslDistribution};

    fn host(alias: &str, user: &str, host: &str) -> SshHostEntry {
        SshHostEntry {
            alias: alias.to_owned(),
            address: Ok(hide_node_link::device::SshAddress {
                user: user.to_owned(),
                host: host.to_owned(),
                port: 22,
            }),
        }
    }

    fn distro(name: &str, wsl_version: u8, default: bool) -> WslDistribution {
        WslDistribution {
            name: name.to_owned(),
            wsl_version,
            setup_unfinished: false,
            default,
        }
    }

    /// The step's listing answered with the config's `hosts` and this PC's
    /// `wsl`, on `runtime` as it now is, on the Devices step.
    fn settle(runtime: &mut Runtime, hosts: SshHostListing, wsl: &WslListing) {
        runtime.snapshot.ui_state.first_run = Some(crate::model::FirstRun {
            version: crate::model::FIRST_RUN_VERSION,
            step: crate::model::FirstRunStep::Devices,
        });
        runtime.machine_profiles_job = Some(MachineProfilesJob {
            id: 1,
            stop: Arc::new(AtomicBool::new(false)),
        });
        runtime.settle_machine_profiles(1, Ok(hosts), wsl);
    }

    fn listing(hosts: Vec<SshHostEntry>, truncated: bool) -> SshHostListing {
        SshHostListing {
            entries: hosts,
            registered: Vec::new(),
            truncated,
            unreadable: false,
        }
    }

    /// A runtime on the Devices step whose listing found the config's
    /// `hosts` and this PC's `wsl`.
    fn at_devices_with(hosts: Vec<SshHostEntry>, wsl: &WslListing) -> Runtime {
        let mut runtime = super::super::tests::runtime();
        settle(&mut runtime, listing(hosts, false), wsl);
        runtime
    }

    fn at_devices(hosts: Vec<SshHostEntry>, truncated: bool) -> Runtime {
        let mut runtime = super::super::tests::runtime();
        settle(&mut runtime, listing(hosts, truncated), &Ok(Vec::new()));
        runtime
    }

    fn two_hosts() -> Vec<SshHostEntry> {
        vec![
            host("config-ubuntu", "ubuntu", "config-ubuntu.example"),
            host("mini", "alice", "mini.example"),
        ]
    }

    fn connect(runtime: &mut Runtime, profile: &str, consent: bool) {
        assert!(
            runtime.dispatch_json(
                &serde_json::to_vec(&serde_json::json!({
                    "schema_version": SCHEMA_VERSION, "kind":"machine_profile_connect",
                    "payload":{"profile_id":profile,"host_consent":consent}
                }))
                .unwrap()
            )
        );
    }

    fn step(runtime: &Runtime) -> Option<crate::model::FirstRunStep> {
        runtime.first_run_step()
    }

    #[test]
    fn listing_and_refused_consent_register_nothing() {
        let mut runtime = at_devices(two_hosts(), false);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert!(runtime.snapshot.status.remote.is_empty());
        connect(&mut runtime, "ssh:config-ubuntu", false);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert!(runtime.snapshot.status.remote.is_empty());
        let row = &runtime.snapshot.status.machine_profiles.profiles[0];
        assert_eq!(
            (row.connect_state.as_str(), row.connect_refusal),
            ("failed", Some(MachineProfileRefusal::NoConsent))
        );
    }

    /// PRD wsl-device B1, D-13: the step lists this PC's default WSL
    /// distribution first, as the device it would be, and no other.
    #[test]
    fn the_default_wsl_distribution_is_the_steps_first_row_and_no_other_is() {
        let runtime = at_devices_with(
            two_hosts(),
            &Ok(vec![distro("Debian", 2, false), distro("Ubuntu", 2, true)]),
        );
        let rows = &runtime.snapshot.status.machine_profiles.profiles;
        assert_eq!(rows.len(), 3, "the two Hosts and one distribution");
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
            "a Host row's wire names no distribution"
        );
    }

    /// B29, D-14: a default distribution that cannot connect is its row with
    /// why; no WSL or no default is no row.
    #[test]
    fn a_default_wsl_1_or_unfinished_distribution_says_why_and_no_wsl_is_no_row() {
        let first = |wsl: WslListing| {
            at_devices_with(two_hosts(), &wsl)
                .snapshot
                .status
                .machine_profiles
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
        for wsl in [Ok(Vec::new()), Ok(vec![distro("Debian", 2, false)])] {
            assert_eq!(first(wsl).wsl_distro, None);
        }
        // A WSL 1 row sent to connect anyway is refused and registers nothing.
        let mut runtime = at_devices_with(two_hosts(), &Ok(vec![distro("Legacy", 1, true)]));
        connect(&mut runtime, "wsl:Legacy", true);
        assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
        assert_eq!(
            runtime.snapshot.status.machine_profiles.profiles[0].connect_refusal,
            Some(MachineProfileRefusal::InvalidTarget)
        );
    }

    /// B1, B25: on a PC whose ssh config names no Host, a WSL listing that
    /// could not be read keeps the Devices step, failed with Retry, instead
    /// of reading as empty; only a listing read empty skips it.
    #[test]
    fn an_unreadable_wsl_listing_keeps_the_step_and_only_a_read_empty_one_skips_it() {
        let unreadable = at_devices_with(
            Vec::new(),
            &Err("the registry could not be read".to_owned()),
        );
        let profiles = &unreadable.snapshot.status.machine_profiles;
        assert_eq!(profiles.state, "failed");
        assert!(profiles.error.is_some());
        assert!(profiles.profiles.is_empty());
        assert_eq!(step(&unreadable), Some(crate::model::FirstRunStep::Devices));
        let empty = at_devices_with(Vec::new(), &Ok(Vec::new()));
        assert_eq!(empty.snapshot.status.machine_profiles.state, "ready");
        assert_eq!(
            step(&empty),
            Some(crate::model::FirstRunStep::Notifications)
        );
    }

    /// macOS, Linux and a PC without WSL: the step is the config's Hosts.
    #[test]
    fn without_wsl_the_step_is_the_hosts_alone() {
        let runtime = at_devices(two_hosts(), false);
        let rows = &runtime.snapshot.status.machine_profiles.profiles;
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["ssh:config-ubuntu", "ssh:mini"]
        );
        assert!(rows.iter().all(|row| row.wsl_distro.is_none()));
    }

    /// B2, D-08, D-12: pressing connect with the row checked registers the
    /// distribution as its WSL device with the install consent and brings
    /// its tile to the front; without consent nothing is registered.
    #[test]
    fn connecting_the_wsl_row_registers_its_device_with_consent_and_selects_it() {
        let mut runtime = at_devices_with(two_hosts(), &Ok(vec![distro("Ubuntu", 2, true)]));
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
        assert_eq!(
            runtime.snapshot.navigator.focused_device_id.as_deref(),
            Some(id.as_str())
        );
        let row = &runtime.snapshot.status.machine_profiles.profiles[0];
        assert_eq!(
            (row.device_id.as_deref(), row.added_as.as_deref()),
            (Some(id.as_str()), Some("WSL Ubuntu"))
        );
        // The Hosts keep their own routes.
        assert!(
            runtime.snapshot.status.machine_profiles.profiles[1..]
                .iter()
                .all(|row| row.device_id.is_none())
        );
        // Pressing again is the same device, not a second one.
        connect(&mut runtime, "wsl:Ubuntu", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
    }

    /// The machines the step offers are the ones the ssh config names: the
    /// step stays, and Connect registers the Host as Add device does, under
    /// its alias, on the default server, with the press's consent.
    #[test]
    fn a_host_keeps_the_step_and_connects_as_add_device_does() {
        let mut runtime = at_devices(
            vec![host("Config.Ubuntu", "ubuntu", "config-ubuntu.example")],
            false,
        );
        assert_eq!(step(&runtime), Some(crate::model::FirstRunStep::Devices));
        let rows = &runtime.snapshot.status.machine_profiles.profiles;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ssh:Config.Ubuntu");
        assert_eq!(rows[0].ssh_host.as_deref(), Some("Config.Ubuntu"));
        assert_eq!(
            rows[0].address.as_deref(),
            Some("ubuntu@config-ubuntu.example:22")
        );
        connect(&mut runtime, "ssh:Config.Ubuntu", true);
        let registrations = &runtime.snapshot.ui_state.device_registrations;
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].id, "config-ubuntu");
        assert_eq!(registrations[0].label, "Config.Ubuntu");
        assert_eq!(registrations[0].origin.ssh_alias(), Some("Config.Ubuntu"));
        assert!(registrations[0].host_consent.is_some());
        let row = &runtime.snapshot.status.machine_profiles.profiles[0];
        assert_eq!(row.device_id.as_deref(), Some("config-ubuntu"));
        assert_eq!(row.added_as.as_deref(), Some("Config.Ubuntu"));
        // A second press is the same device, never a second one.
        connect(&mut runtime, "ssh:Config.Ubuntu", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
    }

    /// With nothing to offer the read settles as none and the step is
    /// skipped; a Host list an Include cut short proves nothing.
    #[test]
    fn only_a_complete_empty_read_skips_the_step() {
        let empty = at_devices(Vec::new(), false);
        assert_eq!(empty.snapshot.status.machine_profiles.state, "ready");
        assert_eq!(
            step(&empty),
            Some(crate::model::FirstRunStep::Notifications)
        );
        let cut = at_devices(Vec::new(), true);
        assert_eq!(step(&cut), Some(crate::model::FirstRunStep::Devices));
    }

    /// One key per server: two aliases of one endpoint are one row, named
    /// by the config's first, while another user and another port stay
    /// apart.
    #[test]
    fn aliases_of_one_server_are_one_row_and_others_stay_apart() {
        let runtime = at_devices(
            vec![
                host("a", "alice", "box.example"),
                host("b", "alice", "BOX.example"),
                host("bob", "bob", "box.example"),
                SshHostEntry {
                    alias: "other-port".to_owned(),
                    address: Ok(hide_node_link::device::SshAddress {
                        user: "alice".to_owned(),
                        host: "box.example".to_owned(),
                        port: 2222,
                    }),
                },
            ],
            false,
        );
        let rows: Vec<_> = runtime
            .snapshot
            .status
            .machine_profiles
            .profiles
            .iter()
            .map(|row| (row.id.as_str(), row.ssh_host.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                ("ssh:a", Some("a")),
                ("ssh:bob", Some("bob")),
                ("ssh:other-port", Some("other-port")),
            ]
        );
    }

    /// Connect is one device however often it is pressed and however the
    /// server is named: a Host row whose server was registered meanwhile
    /// under another alias is that device, and a second press registers
    /// nothing (engineering rule 11).
    #[test]
    fn a_host_row_converges_on_the_device_its_server_already_has() {
        let mut runtime = at_devices(vec![host("a", "alice", "box.example")], false);
        connect(&mut runtime, "ssh:a", true);
        connect(&mut runtime, "ssh:a", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
        // The config now names the server under another alias first.
        settle(
            &mut runtime,
            listing(
                vec![
                    host("b", "alice", "box.example"),
                    host("a", "alice", "box.example"),
                ],
                false,
            ),
            &Ok(Vec::new()),
        );
        let row = &runtime.snapshot.status.machine_profiles.profiles[0];
        assert_eq!(row.id, "ssh:b");
        assert_eq!(row.device_id.as_deref(), Some("a"));
        connect(&mut runtime, "ssh:b", true);
        assert_eq!(runtime.snapshot.ui_state.device_registrations.len(), 1);
    }

    /// A config that exists but could not be read is a failed read with
    /// Retry, never an empty list that skips the step.
    #[test]
    fn an_unreadable_ssh_config_keeps_the_step_with_retry() {
        let mut runtime = super::super::tests::runtime();
        settle(
            &mut runtime,
            SshHostListing {
                unreadable: true,
                ..SshHostListing::default()
            },
            &Ok(Vec::new()),
        );
        assert_eq!(runtime.snapshot.status.machine_profiles.state, "failed");
        assert_eq!(step(&runtime), Some(crate::model::FirstRunStep::Devices));
    }
}
