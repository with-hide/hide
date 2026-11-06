//! A dialed device on the core's own engine (PRD engine-bundle-3-core-owns
//! D-22): its engine opens on a ready link whose node runs its holders, only
//! for the attempt that is still the device's, and goes with the link.

use super::device_kit::{DEVICE, established};
use super::*;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_node_link::holders::{HolderRecovery, PaneCall};
use hide_node_link::protocol::Call;
use std::sync::{Arc, Mutex};

/// A device's node that runs holders and has no pane: it answers what an
/// engine asks while it opens and refuses its watch, which no test needs.
#[derive(Default)]
struct HolderNode {
    asked: Mutex<Vec<String>>,
    closed: Mutex<Option<String>>,
}

impl NodeLink for HolderNode {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        if let Call::Pane { call } = &call {
            self.asked.lock().unwrap().push(format!("{call:?}"));
        }
        let answer: hide_node_link::holders::PaneAnswer = match call {
            Call::Pane {
                call: PaneCall::Recovered { .. },
            } => Ok(serde_json::to_value(HolderRecovery {
                owner: 1,
                closed: Vec::new(),
                ended: Vec::new(),
                present: Vec::new(),
                recoverable: Vec::new(),
            })
            .unwrap()),
            Call::Pane {
                call: PaneCall::ShellLaunch,
            } => Ok(serde_json::Value::Null),
            _ => {
                return Err(LinkError::Refused(hide_node_link::error::HostError::new(
                    hide_node_link::error::ErrorCode::Unsupported,
                    "the test's node answers no such call",
                )));
            }
        };
        Ok(LinkAnswer::Parsed(serde_json::to_value(answer).unwrap()))
    }

    fn closed_reason(&self) -> Option<String> {
        self.closed.lock().unwrap().clone()
    }

    fn close(&self, reason: &str) {
        *self.closed.lock().unwrap() = Some(reason.to_owned());
    }
}

/// A core whose devices run on its own engine, with the dialed device
/// `DEVICE` registered, keeping their sessions under `state`.
fn own_runtime(state: &std::path::Path) -> SharedRuntime {
    let shared = super::device_kit::with_consent(None);
    shared
        .lock()
        .unwrap()
        .run_devices_on_own_engine(state.to_path_buf());
    shared
}

/// The device's ready connection over `node`, its node answering for a home
/// and running its holders when `holders` is `Ok`.
fn ready(
    node: Arc<HolderNode>,
    holders: Result<(), String>,
) -> hide_node_link::device::Established {
    let terminals = holders
        .is_ok()
        .then(|| Arc::new(super::RecordedTerminals::default()) as Arc<_>);
    let mut connected = established(node, terminals);
    connected.node.hello.home = Some("/srv/account".to_owned());
    if let Err(reason) = holders {
        connected.node.terminals = Err(reason);
    }
    connected
}

fn connect(shared: &Mutex<Runtime>, connected: hide_node_link::device::Established) -> u64 {
    let mut runtime = shared.lock().unwrap();
    runtime.start_device_host(DEVICE, hosts::HostAttempt::Background);
    let generation = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Ok(hosts::HostReady::Dialed(Box::new(connected))),
    );
    generation
}

/// Waits, bounded, until the device's own engine is installed or not.
#[allow(clippy::disallowed_methods)] // a polling helper: the engine opens on a worker the test cannot join
fn wait_engine(shared: &Mutex<Runtime>, installed: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while shared.lock().unwrap().own_device_engines().len() != usize::from(installed) {
        assert!(
            Instant::now() < deadline,
            "the device's own engine was never {}",
            if installed { "installed" } else { "let go" }
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Waits, bounded, until the device takes deliveries: its session reads
/// connected, which the link's session sync reports after the engine opened.
#[allow(clippy::disallowed_methods)] // a polling helper: the session sync runs on a worker the test cannot join
fn wait_delivery(shared: &Mutex<Runtime>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while shared.lock().unwrap().delivery_engine(DEVICE).is_none() {
        assert!(Instant::now() < deadline, "the device never read connected");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn row_message(shared: &Mutex<Runtime>) -> Option<String> {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .status
        .remote
        .iter()
        .find(|row| row.target_id == DEVICE)
        .and_then(|row| row.message.clone())
}

#[test]
fn a_ready_device_gets_its_own_engine_which_goes_with_its_link() {
    let state = tempfile::tempdir().unwrap();
    let shared = own_runtime(state.path());
    let node = Arc::new(HolderNode::default());
    let generation = connect(&shared, ready(Arc::clone(&node), Ok(())));
    wait_engine(&shared, true);
    assert!(
        hide_kit::layout::device_engine_dir(state.path(), DEVICE)
            .unwrap()
            .starts_with(state.path()),
        "the device's session is kept in the core's state folder"
    );

    let changed = shared.lock().unwrap().ingest_host_closed(
        DEVICE,
        generation,
        "the network dropped".to_owned(),
    );
    assert!(changed);
    wait_engine(&shared, false);
}

#[test]
fn a_device_whose_holders_did_not_start_opens_no_engine_and_says_why() {
    let state = tempfile::tempdir().unwrap();
    let shared = own_runtime(state.path());
    let node = Arc::new(HolderNode::default());
    connect(
        &shared,
        ready(
            Arc::clone(&node),
            Err("the holder program could not start".to_owned()),
        ),
    );
    assert!(shared.lock().unwrap().own_device_engines().is_empty());
    let message = row_message(&shared).expect("the row says why");
    assert!(
        message.contains("pane holders did not start")
            && message.contains("the holder program could not start"),
        "{message}"
    );
    let asked = node.asked.lock().unwrap().clone();
    assert!(asked.is_empty(), "no engine asked the node: {asked:?}");
}

/// An engine that finished opening after its attempt was replaced is
/// handed back to be let go off the lock, not installed on the newer link.
#[test]
fn an_engine_opened_for_a_replaced_attempt_is_not_installed() {
    let state = tempfile::tempdir().unwrap();
    let shared = own_runtime(state.path());
    let generation = connect(&shared, ready(Arc::new(HolderNode::default()), Ok(())));
    wait_engine(&shared, true);
    let late = crate::engine::own::open_device(
        DEVICE,
        Arc::new(HolderNode::default()),
        state.path(),
        "/srv/account".to_owned(),
        false,
        Arc::new(hide_agent_adapter::definition::Catalog::read(Vec::new())),
        Arc::default(),
    )
    .map(|engine| Arc::new(engine) as crate::engine::SharedEngine);
    let (changed, unused) =
        shared
            .lock()
            .unwrap()
            .install_own_device_engine(DEVICE, generation + 1, late);
    assert!(!changed);
    assert!(unused.is_some(), "the late engine was not handed back");
    assert_eq!(shared.lock().unwrap().own_device_engines().len(), 1);
    drop(unused);
}

/// With the switch off a dialed device's panes stay on its engine.
#[test]
fn a_core_with_the_switch_off_opens_no_device_engine() {
    let shared = super::device_kit::with_consent(None);
    connect(&shared, ready(Arc::new(HolderNode::default()), Ok(())));
    assert!(shared.lock().unwrap().own_device_engines().is_empty());
    assert!(shared.lock().unwrap().own_devices.is_none());
}

/// A spawned agent's identity on a device binds by the session its hook
/// reports, which only the device's own engine can keep: the core once
/// handed a hook's session to this machine's engine alone, so an agent on a
/// device never had one and its spawn waited for an identity for good.
#[test]
fn a_hooks_session_on_a_device_goes_to_that_devices_own_engine() {
    let state = tempfile::tempdir().unwrap();
    let shared = own_runtime(state.path());
    let node = Arc::new(HolderNode::default());
    connect(&shared, ready(Arc::clone(&node), Ok(())));
    wait_engine(&shared, true);
    wait_delivery(&shared);
    let report = |session: Option<&str>| hide_node_link::panes::HookStatusReport {
        runtime: "claude-code".to_owned(),
        version: hide_agent_hooks::HOOK_VERSION,
        session: session.map(str::to_owned),
        turn: None,
        counts: None,
        session_start: true,
        waiting: None,
    };
    let runtime = shared.lock().unwrap();
    let device = runtime.own_device_engines().remove(0);
    // The pane's credential names it as the core does, with its device; the
    // device's engine knows it by its holder's own id.
    let scoped = crate::address::scoped_pane_id(DEVICE, "w4:p2");
    for caller in [scoped.as_str(), "w4:p2"] {
        let (taken, pane) = runtime
            .hook_session_target(DEVICE, caller, &report(Some("s-1")))
            .expect("the device's engine takes the session");
        assert!(Arc::ptr_eq(&taken, &device));
        assert_eq!(pane, "w4:p2", "{caller}");
    }
    assert!(
        runtime
            .hook_session_target(DEVICE, &scoped, &report(None))
            .is_none()
    );
}
