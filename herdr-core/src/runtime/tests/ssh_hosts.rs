//! Add device's host list through the core: the event starts one listing, a
//! second request joins it, and the answer names each alias's address and the
//! device that already holds it. The test decides each alias's answer and
//! records what was asked; nothing connects anywhere, and no child races the
//! `ssh -G` deadline (issue 728).

use super::*;
use crate::model::DeviceRegistration;
use hide_node::ssh::hosts::Resolve;
use hide_node_link::device::{SshAddress as Address, SshHostProblem as Problem};

/// Answers each alias the way `ssh -G` would for the test's config, and
/// records the home and alias of every call.
fn answers(asked: Arc<Mutex<Vec<(PathBuf, String)>>>) -> Resolve {
    Arc::new(move |home, alias, _stop| {
        asked
            .lock()
            .unwrap()
            .push((home.to_owned(), alias.to_owned()));
        let address = |host: &str, port| {
            Ok(Address {
                user: "grab".to_owned(),
                host: host.to_owned(),
                port,
            })
        };
        match alias {
            "broken" => Err(Problem::SshFailed),
            "studio" | "studio-ip" => address("10.0.0.2", 22),
            "fresh" | "old-alias" => address("10.0.0.9", 2222),
            other => address(&format!("{other}.example"), 22),
        }
    })
}

fn registration(id: &str, label: &str, alias: &str) -> DeviceRegistration {
    DeviceRegistration {
        id: id.to_owned(),
        label: label.to_owned(),
        origin: crate::model::LinkOrigin::Dialed {
            ssh_alias: alias.to_owned(),
        },
        herdr_socket_path: None,
        herdr_session: None,
        host_consent: None,
    }
}

fn list_event() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "ssh_hosts_list",
        "payload": {}
    }))
    .unwrap()
}

#[test]
fn the_host_list_names_each_alias_its_address_and_the_device_that_holds_it() {
    let mut runtime = super::devices::runtime_with_home();
    let home = runtime.home_path.clone().expect("fixture home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(
        home.join(".ssh/config"),
        "Host *\n  ServerAliveInterval 30\nHost studio studio-ip fresh broken\n",
    )
    .unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    runtime.devices =
        Arc::new(hide_node::ssh::Connector::new(None).with_resolve(answers(Arc::clone(&asked))));
    // "Studio Mac" is added under `studio`; "Old" under an alias the config
    // no longer names that reaches the same address as `fresh`.
    runtime.snapshot.ui_state.device_registrations = vec![
        registration("studio", "Studio Mac", "studio"),
        registration("old", "Old", "old-alias"),
    ];
    let shared = Arc::new(Mutex::new(runtime));
    shared.lock().unwrap().install_worker_context(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
    );

    assert_eq!(
        shared.lock().unwrap().snapshot().status.ssh_hosts.state,
        "idle"
    );
    {
        // Under the event's own lock: the listing's answers come at once, so
        // after a release it may already have ended.
        let mut runtime = shared.lock().unwrap();
        assert!(runtime.dispatch_json(&list_event()));
        assert_eq!(runtime.snapshot().status.ssh_hosts.state, "loading");
        // A request while one runs is the same listing, not a second one.
        assert!(!runtime.dispatch_json(&list_event()));
    }
    wait(&shared, "the host list", |runtime| {
        runtime.snapshot().status.ssh_hosts.state == "ready"
    });

    let listing = shared.lock().unwrap().snapshot().status.ssh_hosts.clone();
    let row = |alias: &str| {
        listing
            .hosts
            .iter()
            .find(|host| host.alias == alias)
            .unwrap_or_else(|| panic!("{alias} is listed: {listing:?}"))
            .clone()
    };
    assert_eq!(
        listing
            .hosts
            .iter()
            .map(|host| host.alias.as_str())
            .collect::<Vec<_>>(),
        ["studio", "studio-ip", "fresh", "broken"],
        "the wildcard Host is never listed"
    );
    assert_eq!(row("studio").address.as_deref(), Some("grab@10.0.0.2:22"));
    assert_eq!(row("studio").added_as.as_deref(), Some("Studio Mac"));
    // Another alias of the same machine reads as added too.
    assert_eq!(row("studio-ip").added_as.as_deref(), Some("Studio Mac"));
    assert_eq!(row("fresh").address.as_deref(), Some("grab@10.0.0.9:2222"));
    assert_eq!(row("fresh").added_as.as_deref(), Some("Old"));
    assert_eq!(row("broken").address, None);
    assert_eq!(row("broken").problem.as_deref(), Some("ssh_failed"));
    assert_eq!(row("broken").added_as, None);
    assert!(!listing.truncated);

    // One question per alias, under the account's own home.
    let mut asked = asked.lock().unwrap().clone();
    asked.sort_unstable();
    let under_home = |alias: &str| (home.clone(), alias.to_owned());
    assert_eq!(
        asked,
        [
            under_home("broken"),
            under_home("fresh"),
            under_home("old-alias"),
            under_home("studio"),
            under_home("studio-ip"),
        ]
    );
}

#[test]
fn a_missing_config_or_program_lists_nothing_or_names_the_problem() {
    let mut runtime = super::devices::runtime_with_home();
    // The real `ssh -G` path, with a program that cannot start: that fails at
    // once, so no deadline is involved.
    runtime.devices = Arc::new(
        hide_node::ssh::Connector::new(None).with_ssh_program(PathBuf::from("/nonexistent/ssh")),
    );
    let home = runtime.home_path.clone().expect("fixture home");
    let shared = Arc::new(Mutex::new(runtime));
    shared.lock().unwrap().install_worker_context(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
    );
    // No ~/.ssh/config at all: ready, with no host to offer (B52).
    assert!(shared.lock().unwrap().dispatch_json(&list_event()));
    wait(&shared, "the empty list", |runtime| {
        runtime.snapshot().status.ssh_hosts.state == "ready"
    });
    assert!(
        shared
            .lock()
            .unwrap()
            .snapshot()
            .status
            .ssh_hosts
            .hosts
            .is_empty()
    );

    // A config whose ssh cannot start shows the host with the problem.
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), "Host lonely\n").unwrap();
    assert!(shared.lock().unwrap().dispatch_json(&list_event()));
    wait(&shared, "the host list", |runtime| {
        runtime.snapshot().status.ssh_hosts.hosts.len() == 1
    });
    let hosts = shared
        .lock()
        .unwrap()
        .snapshot()
        .status
        .ssh_hosts
        .hosts
        .clone();
    assert_eq!(hosts[0].alias, "lonely");
    assert_eq!(hosts[0].problem.as_deref(), Some("ssh_missing"));
}

/// The real ssh connector for Hosts, beside a Herdr that saved no machine:
/// the first-run step of a person who never used Herdr.
struct NoSavedMachines(hide_node::ssh::Connector);

impl crate::remote::DeviceConnector for NoSavedMachines {
    fn transport(
        &self,
        home: &std::path::Path,
        node: &str,
        alias: &str,
        herdr_socket: Option<String>,
    ) -> Result<Arc<dyn crate::remote::DialedTransport>, String> {
        self.0.transport(home, node, alias, herdr_socket)
    }

    fn herdr_profiles(
        &self,
        _home: &std::path::Path,
        _registered: &[String],
        _stop: &std::sync::atomic::AtomicBool,
    ) -> Result<hide_node_link::device::HerdrProfileListing, String> {
        Ok(hide_node_link::device::HerdrProfileListing::default())
    }

    fn ssh_hosts(
        &self,
        home: &std::path::Path,
        registered: &[String],
        stop: &std::sync::atomic::AtomicBool,
    ) -> crate::remote::SshHostListing {
        self.0.ssh_hosts(home, registered, stop)
    }

    fn wsl_transport(
        &self,
        node: &str,
        distro: &str,
    ) -> Result<Arc<dyn crate::remote::DialedTransport>, String> {
        self.0.wsl_transport(node, distro)
    }

    fn wsl_distributions(&self) -> crate::remote::WslListing {
        self.0.wsl_distributions()
    }
}

/// A runtime whose account has `config` as its ssh config and whose `ssh -G`
/// answers as `resolve` does.
fn first_run_runtime(config: &str, resolve: Resolve) -> Arc<Mutex<Runtime>> {
    let mut runtime = super::devices::runtime_with_home();
    let home = runtime.home_path.clone().expect("fixture home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), config).unwrap();
    runtime.devices = Arc::new(NoSavedMachines(
        hide_node::ssh::Connector::new(None).with_resolve(resolve),
    ));
    let shared = Arc::new(Mutex::new(runtime));
    shared.lock().unwrap().install_worker_context(
        Arc::downgrade(&shared),
        crate::handle::ChangeNotifier::noop(),
    );
    shared
}

fn event(kind: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": {}
    }))
    .unwrap()
}

/// Add device's list and the first-run step read one config and say the
/// same thing about each Host: a Host whose server is a registered device,
/// spelled in another case, reads as that device on both, and a different
/// port, a named session or a custom socket on the same host reads as not
/// added on both.
#[test]
fn add_device_and_the_first_run_step_agree_on_which_host_is_added() {
    let shared = first_run_runtime(
        "Host lower other-port named socket\n",
        Arc::new(|_, alias, _| {
            let (user, host, port) = match alias {
                "upper" => ("alice", "BOX.example", 22),
                "lower" => ("alice", "box.example", 22),
                "other-port" => ("alice", "box.example", 2222),
                "named" => ("carol", "box.example", 22),
                "socket" => ("dave", "box.example", 22),
                _ => return Err(Problem::SshFailed),
            };
            Ok(Address {
                user: user.to_owned(),
                host: host.to_owned(),
                port,
            })
        }),
    );
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_registrations = vec![
        // Added under an alias the config no longer names, in capitals.
        registration("box", "Box", "upper"),
        DeviceRegistration {
            herdr_session: Some("work".to_owned()),
            ..registration("box-work", "Box work", "named")
        },
        DeviceRegistration {
            herdr_socket_path: Some("/run/herdr.sock".to_owned()),
            ..registration("box-socket", "Box socket", "socket")
        },
    ];
    for kind in ["ssh_hosts_list", "herdr_profiles_list"] {
        assert!(shared.lock().unwrap().dispatch_json(&event(kind)));
    }
    wait(&shared, "both listings", |runtime| {
        runtime.snapshot().status.ssh_hosts.state == "ready"
            && runtime.snapshot().status.herdr_profiles.state == "ready"
    });
    let runtime = shared.lock().unwrap();
    let add_device: Vec<(String, Option<String>)> = runtime
        .snapshot()
        .status
        .ssh_hosts
        .hosts
        .iter()
        .map(|host| (host.alias.clone(), host.added_as.clone()))
        .collect();
    let first_run: Vec<(String, Option<String>)> = runtime
        .snapshot()
        .status
        .herdr_profiles
        .profiles
        .iter()
        .map(|row| (row.ssh_host.clone().unwrap(), row.added_as.clone()))
        .collect();
    let expected = [
        ("lower", Some("Box")),
        ("other-port", None),
        ("named", None),
        ("socket", None),
    ]
    .map(|(alias, added)| (alias.to_owned(), added.map(str::to_owned)));
    assert_eq!(add_device, expected);
    assert_eq!(first_run, expected);
}

/// A wildcard Include whose folder cannot be opened says nothing about
/// the Hosts it names: the first-run step reads it as a failed read with
/// Retry, never as a list that could be skipped. The folder is a file,
/// which no account can list; Windows answers that path with another
/// error, so the case runs on Unix.
#[cfg(unix)]
#[test]
fn an_include_folder_that_cannot_be_opened_fails_the_first_run_read_with_retry() {
    let shared = first_run_runtime(
        "Host first\nInclude fragments/*\n",
        Arc::new(|_, _, _| panic!("an unreadable config resolves nothing")),
    );
    let home = shared.lock().unwrap().home_path.clone().unwrap();
    std::fs::write(home.join(".ssh/fragments"), "Host not-a-folder\n").unwrap();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.snapshot.ui_state.first_run = Some(crate::model::FirstRun {
            version: crate::model::FIRST_RUN_VERSION,
            step: crate::model::FirstRunStep::Devices,
        });
        assert!(runtime.dispatch_json(&event("herdr_profiles_list")));
    }
    wait(&shared, "the device listing", |runtime| {
        runtime.snapshot().status.herdr_profiles.state != "loading"
    });
    let runtime = shared.lock().unwrap();
    let listing = &runtime.snapshot().status.herdr_profiles;
    assert_eq!(listing.state, "failed");
    assert!(listing.error.is_some());
    assert!(listing.profiles.is_empty());
    assert_eq!(
        runtime.first_run_step(),
        Some(crate::model::FirstRunStep::Devices)
    );
}

/// A file the config includes that could not be read keeps the read
/// unreadable even when a later wildcard Include is then cut at the file
/// bound: the step still answers with Retry, not with an incomplete list.
#[test]
fn an_unreadable_include_stays_unreadable_past_a_later_include_bound() {
    let shared = first_run_runtime(
        "Host first\nInclude unreadable\nInclude many/*\n",
        Arc::new(|_, _, _| panic!("an unreadable config resolves nothing")),
    );
    let home = shared.lock().unwrap().home_path.clone().unwrap();
    // A folder where a config file should be cannot be read as one.
    std::fs::create_dir_all(home.join(".ssh/unreadable")).unwrap();
    std::fs::create_dir_all(home.join(".ssh/many")).unwrap();
    for index in 0..=64 {
        std::fs::write(home.join(format!(".ssh/many/{index:03}")), "Host extra\n").unwrap();
    }
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&event("herdr_profiles_list"))
    );
    wait(&shared, "the device listing", |runtime| {
        runtime.snapshot().status.herdr_profiles.state != "loading"
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.snapshot().status.herdr_profiles.state, "failed");
    assert!(runtime.snapshot().status.herdr_profiles.profiles.is_empty());
}
