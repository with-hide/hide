//! A WSL distribution as a device, through the core: its registration, the
//! one rule for which attempts may start it when it is off (PRD wsl-device
//! D-11, B3, B22-B24), and Add device's list of distributions (B4, B28). The
//! transport is the test's: it records the leave each attempt was given and
//! answers as a distribution that is off would, so nothing starts anywhere.

use super::*;
use crate::remote::{
    Attempt, CapabilityReport, DeviceConnector, DeviceTransport, DialedTransport, EstablishError,
    Established, SshHostListing, Wake, WslListing,
};
use hide_node_link::device::{HostConsent, RemoteHostIdentity, SnapshotCheck, WslDistribution};
use std::sync::mpsc;

/// Every attempt's leave, in order, and a gate an attempt waits at when
/// the test holds it.
#[derive(Default)]
struct Attempts {
    wakes: Mutex<Vec<Wake>>,
    held: Mutex<Option<mpsc::Receiver<()>>>,
    /// Set when `wsl.exe` cannot say which distributions run: an attempt
    /// without leave fails rather than finding the distribution off.
    undetermined: std::sync::atomic::AtomicBool,
}

struct Distributions(Arc<Attempts>);

impl DeviceConnector for Distributions {
    fn transport(
        &self,
        _home: &std::path::Path,
        _node: &str,
        _alias: &str,
        _herdr_socket: Option<String>,
    ) -> Result<Arc<dyn DialedTransport>, String> {
        Err("this test has no SSH devices".to_owned())
    }

    fn ssh_hosts(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> SshHostListing {
        SshHostListing::default()
    }

    fn herdr_profiles(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> Result<hide_node_link::device::HerdrProfileListing, String> {
        Ok(hide_node_link::device::HerdrProfileListing::default())
    }

    fn wsl_transport(&self, _node: &str, distro: &str) -> Result<Arc<dyn DialedTransport>, String> {
        assert_eq!(distro, "Ubuntu");
        Ok(Arc::new(OffDistribution(Arc::clone(&self.0))))
    }

    fn wsl_distributions(&self) -> WslListing {
        Ok(vec![
            WslDistribution {
                name: "Ubuntu".to_owned(),
                wsl_version: 2,
                setup_unfinished: false,
                default: true,
            },
            WslDistribution {
                name: "Fresh".to_owned(),
                wsl_version: 2,
                setup_unfinished: true,
                default: false,
            },
            WslDistribution {
                name: "Legacy".to_owned(),
                wsl_version: 1,
                setup_unfinished: false,
                default: false,
            },
        ])
    }
}

/// A distribution that is off: an attempt that may not start it ends as
/// stopped, and one that may, as a helper that failed, so the test sees
/// which leave each attempt had without anything to connect.
struct OffDistribution(Arc<Attempts>);

impl DeviceTransport for OffDistribution {
    fn herdr_api_connector(&self) -> Arc<dyn hide_herdr_client::ApiConnector> {
        Arc::new(hide_herdr_client::LocalSocketConnector::new(
            "/tmp/herdr-core-never-connect.sock",
        ))
    }

    fn cached_herdr_version(&self) -> Option<String> {
        None
    }
}

impl DialedTransport for OffDistribution {
    fn capability_test(&self, operation_id: &str, _check: SnapshotCheck<'_>) -> CapabilityReport {
        CapabilityReport::new(
            operation_id,
            RemoteHostIdentity {
                host_id: "wsl:Ubuntu".to_owned(),
                alias: "Ubuntu".to_owned(),
                hostname: "Ubuntu".to_owned(),
                port: 0,
            },
        )
    }

    fn establish(
        &self,
        _consent: &HostConsent,
        _retirement_projects: &[String],
        attempt: Attempt,
    ) -> Result<Established, EstablishError> {
        let wake = attempt.wake;
        let held = self.0.held.lock().unwrap().take();
        if let Some(held) = held {
            let _ = held.recv_timeout(Duration::from_secs(10));
        }
        self.0.wakes.lock().unwrap().push(wake);
        let undetermined = self
            .0
            .undetermined
            .load(std::sync::atomic::Ordering::SeqCst);
        Err(match wake {
            Wake::NotAllowed if undetermined => EstablishError::Helper(
                "wsl.exe could not list the running distributions (exit code: 1)".to_owned(),
            ),
            Wake::NotAllowed => EstablishError::Stopped("Ubuntu is off".to_owned()),
            Wake::Allowed => EstablishError::Helper("the test starts no helper".to_owned()),
        })
    }
}

fn wsl_runtime() -> (Arc<Mutex<Runtime>>, Arc<Attempts>) {
    let mut runtime = super::devices::runtime_with_home();
    let attempts = Arc::new(Attempts::default());
    runtime.devices = Arc::new(Distributions(Arc::clone(&attempts)));
    let shared = Arc::new(Mutex::new(runtime));
    shared.lock().unwrap().install_worker_context(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
    );
    (shared, attempts)
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload,
    }))
    .unwrap()
}

fn wakes(attempts: &Attempts) -> Vec<Wake> {
    attempts.wakes.lock().unwrap().clone()
}

fn wait_for_attempts(attempts: &Attempts, count: usize) {
    super::wait_for(&format!("{count} attempts"), || {
        attempts.wakes.lock().unwrap().len() >= count
    });
}

fn host_settled(shared: &Arc<Mutex<Runtime>>) {
    super::wait(shared, "the attempt to settle", |runtime| {
        !runtime.device_host_connecting("wsl-ubuntu")
    });
}

/// B2, B4, D-20: Add device's distribution registers as `WSL Ubuntu`, and
/// its first connection, the operator's own, may start it.
#[test]
fn a_registered_distribution_is_named_for_it_and_its_first_attempt_may_start_it() {
    let (shared, attempts) = wsl_runtime();
    let registered = event(
        "register_device",
        serde_json::json!({"wsl_distro": "Ubuntu", "host_consent": true}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&registered));
    wait_for_attempts(&attempts, 1);
    assert_eq!(wakes(&attempts), [Wake::Allowed]);
    let runtime = shared.lock().unwrap();
    let id = crate::runtime::devices::wsl_device_id("Ubuntu");
    let registration = &runtime.snapshot().ui_state.device_registrations[0];
    assert_eq!(
        (registration.id.as_str(), registration.label.as_str()),
        (id.as_str(), "WSL Ubuntu")
    );
    let device = runtime
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == id)
        .expect("the device row");
    assert_eq!(device.wsl_distro.as_deref(), Some("Ubuntu"));
    drop(runtime);
    // The same distribution is one device.
    assert!(shared.lock().unwrap().dispatch_json(&registered));
    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.snapshot().ui_state.device_registrations.len(), 1);
}

/// B22, B24: the core starting dials a distribution that is off with no
/// leave to start it: it reads `unavailable`, no retry is scheduled, and
/// only the app opening, or the operator's `다시 연결`, tries it with leave.
/// A resume does not.
#[test]
fn only_the_app_opening_or_the_operators_reconnect_may_start_an_off_distribution() {
    let (shared, attempts) = wsl_runtime();
    {
        let mut runtime = shared.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: "wsl-ubuntu".to_owned(),
                label: "WSL Ubuntu".to_owned(),
                origin: crate::model::LinkOrigin::Wsl {
                    distro: "Ubuntu".to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: Some(consent),
            });
        runtime.rebuild_device_rows();
        runtime.connect_registered_devices();
    }
    wait_for_attempts(&attempts, 1);
    host_settled(&shared);
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(wakes(&attempts), [Wake::NotAllowed]);
        assert_eq!(runtime.host_snapshot("wsl-ubuntu").state, "unavailable");
        assert!(
            runtime.helper_link_wait("wsl-ubuntu").is_none(),
            "an off distribution is not 'reconnecting'"
        );
        assert!(
            runtime
                .device_host_retries
                .get("wsl-ubuntu")
                .is_none_or(|retry| retry.at_unix_ms.is_none())
        );
    }
    let resume = event("hide_ai_wake", serde_json::json!({"cause": "resume"}));
    shared.lock().unwrap().dispatch_json(&resume);
    let app_open = event("hide_ai_wake", serde_json::json!({"cause": "app_open"}));
    shared.lock().unwrap().dispatch_json(&app_open);
    wait_for_attempts(&attempts, 2);
    host_settled(&shared);
    assert_eq!(wakes(&attempts), [Wake::NotAllowed, Wake::Allowed]);

    let reconnect = event(
        "device_host_retry",
        serde_json::json!({"device_id": "wsl-ubuntu"}),
    );
    shared.lock().unwrap().dispatch_json(&reconnect);
    wait_for_attempts(&attempts, 3);
    host_settled(&shared);
    assert_eq!(
        wakes(&attempts),
        [Wake::NotAllowed, Wake::Allowed, Wake::Allowed]
    );
}

/// B3: the app opening while the core's own attempt still runs is not
/// lost: should that attempt find the distribution off, the next one may
/// start it.
#[test]
fn the_app_opening_during_an_attempt_without_leave_is_taken_by_the_next() {
    let (shared, attempts) = wsl_runtime();
    let (release, held) = mpsc::channel();
    *attempts.held.lock().unwrap() = Some(held);
    {
        let mut runtime = shared.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: "wsl-ubuntu".to_owned(),
                label: "WSL Ubuntu".to_owned(),
                origin: crate::model::LinkOrigin::Wsl {
                    distro: "Ubuntu".to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: Some(consent),
            });
        runtime.rebuild_device_rows();
        runtime.connect_registered_devices();
        assert!(runtime.device_host_connecting("wsl-ubuntu"));
    }
    let app_open = event("hide_ai_wake", serde_json::json!({"cause": "app_open"}));
    shared.lock().unwrap().dispatch_json(&app_open);
    release.send(()).unwrap();
    wait_for_attempts(&attempts, 2);
    assert_eq!(wakes(&attempts), [Wake::NotAllowed, Wake::Allowed]);
}

/// B3, D-15: the app opening while the core's own attempt runs is not lost
/// when that attempt fails without saying whether the distribution runs:
/// the retry it schedules takes the app's leave.
#[test]
fn the_app_opening_during_an_attempt_that_fails_is_taken_by_its_retry() {
    let (shared, attempts) = wsl_runtime();
    attempts
        .undetermined
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (release, held) = mpsc::channel();
    *attempts.held.lock().unwrap() = Some(held);
    {
        let mut runtime = shared.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: "wsl-ubuntu".to_owned(),
                label: "WSL Ubuntu".to_owned(),
                origin: crate::model::LinkOrigin::Wsl {
                    distro: "Ubuntu".to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: Some(consent),
            });
        runtime.rebuild_device_rows();
        runtime.connect_registered_devices();
        assert!(runtime.device_host_connecting("wsl-ubuntu"));
    }
    let app_open = event("hide_ai_wake", serde_json::json!({"cause": "app_open"}));
    shared.lock().unwrap().dispatch_json(&app_open);
    release.send(()).unwrap();
    wait_for_attempts(&attempts, 1);
    host_settled(&shared);
    {
        let mut runtime = shared.lock().unwrap();
        let at = runtime.device_host_retries["wsl-ubuntu"]
            .at_unix_ms
            .expect("the failed attempt is tried again");
        runtime.tick_device_hosts(at);
    }
    wait_for_attempts(&attempts, 2);
    host_settled(&shared);
    assert_eq!(wakes(&attempts), [Wake::NotAllowed, Wake::Allowed]);
}

/// B4, B28: Add device lists each distribution beside the SSH aliases,
/// says which cannot connect and why, and names the device that already is
/// one.
#[test]
fn add_device_lists_distributions_with_what_keeps_one_from_connecting() {
    let (shared, attempts) = wsl_runtime();
    let registered = event(
        "register_device",
        serde_json::json!({"wsl_distro": "Ubuntu", "host_consent": true}),
    );
    assert!(shared.lock().unwrap().dispatch_json(&registered));
    wait_for_attempts(&attempts, 1);
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&event("ssh_hosts_list", serde_json::json!({})))
    );
    super::wait(&shared, "the list", |runtime| {
        runtime.snapshot().status.ssh_hosts.state == "ready"
    });
    let runtime = shared.lock().unwrap();
    let listed: Vec<_> = runtime
        .snapshot()
        .status
        .ssh_hosts
        .wsl
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.default,
                row.added_as.as_deref(),
                row.problem.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("Ubuntu", true, Some("WSL Ubuntu"), None),
            ("Fresh", false, None, Some("setup_unfinished")),
            ("Legacy", false, None, Some("wsl1")),
        ]
    );
}

/// D-15, B24: when `wsl.exe` cannot say whether the distribution runs, the
/// attempt is a failure that is tried again, and every retry is Hide's own,
/// so none may start the distribution.
#[test]
fn a_distribution_whose_state_cannot_be_read_is_retried_without_leave() {
    let (shared, attempts) = wsl_runtime();
    attempts
        .undetermined
        .store(true, std::sync::atomic::Ordering::SeqCst);
    {
        let mut runtime = shared.lock().unwrap();
        let consent = runtime.new_host_consent();
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: "wsl-ubuntu".to_owned(),
                label: "WSL Ubuntu".to_owned(),
                origin: crate::model::LinkOrigin::Wsl {
                    distro: "Ubuntu".to_owned(),
                },
                herdr_socket_path: None,
                herdr_session: None,
                host_consent: Some(consent),
            });
        runtime.rebuild_device_rows();
        runtime.connect_registered_devices();
    }
    for attempt in 1..=3 {
        wait_for_attempts(&attempts, attempt);
        host_settled(&shared);
        let mut runtime = shared.lock().unwrap();
        let at = runtime.device_host_retries["wsl-ubuntu"]
            .at_unix_ms
            .expect("a failure that says nothing is tried again");
        runtime.tick_device_hosts(at);
    }
    wait_for_attempts(&attempts, 4);
    host_settled(&shared);
    assert_eq!(wakes(&attempts), [Wake::NotAllowed; 4]);
}

/// D-20: each distribution is its own device whatever its name: a name of
/// plain letters keeps its plain id, and names that differ only in letters
/// an id cannot carry still get two ids.
#[test]
fn every_distribution_name_gets_an_id_of_its_own() {
    let id = crate::runtime::devices::wsl_device_id;
    // The digests are SHA-256's of each exact name, worked out apart from
    // the code.
    assert_eq!(id("Ubuntu"), "wsl-ubuntu-710eb4f4");
    assert_eq!(id("Ubuntu.24"), "wsl-ubuntu-24-6d166a5b");
    assert_eq!(id("Ubuntu-24-6d166a5b"), "wsl-ubuntu-24-6d166a5b-8bfa2cb6");
    assert_ne!(id("우분투"), id("데비안"));
    assert_ne!(id("Ubuntu_22.04"), id("Ubuntu-22.04"));
    assert_eq!(
        id("Ubuntu-24.04"),
        id("Ubuntu-24.04"),
        "the same name, the same id"
    );
}

/// B1, B2, D-08: the first-run step lists the default distribution from the
/// registry alone, so no attempt reaches it and nothing is registered or
/// installed until the step's connect is pressed; the press is the
/// operator's own attempt, which may start it.
#[test]
fn the_first_run_row_reaches_the_distribution_only_once_connect_is_pressed() {
    let (shared, attempts) = wsl_runtime();
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&event("herdr_profiles_list", serde_json::json!({})))
    );
    super::wait(&shared, "the device listing", |runtime| {
        runtime.snapshot().status.herdr_profiles.state == "ready"
    });
    {
        let runtime = shared.lock().unwrap();
        let rows = &runtime.snapshot().status.herdr_profiles.profiles;
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["wsl:Ubuntu"],
            "the default distribution alone, beside no saved profile"
        );
        assert!(runtime.snapshot().ui_state.device_registrations.is_empty());
    }
    assert!(
        wakes(&attempts).is_empty(),
        "the listing reached the distribution"
    );
    assert!(shared.lock().unwrap().dispatch_json(&event(
        "herdr_profile_connect",
        serde_json::json!({"profile_id": "wsl:Ubuntu", "host_consent": true}),
    )));
    wait_for_attempts(&attempts, 1);
    assert_eq!(wakes(&attempts), [Wake::Allowed]);
    let runtime = shared.lock().unwrap();
    let id = crate::runtime::devices::wsl_device_id("Ubuntu");
    assert_eq!(
        runtime.snapshot().navigator.focused_device_id.as_deref(),
        Some(id.as_str())
    );
}
