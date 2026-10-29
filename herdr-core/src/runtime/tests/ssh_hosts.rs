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
