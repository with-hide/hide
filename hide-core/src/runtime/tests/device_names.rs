//! A dialed device's name (`runtime/device_names.rs`): a device added by its
//! SSH alias takes its machine's own id on its first connection, opens
//! under it only once the core state holding it is saved, and opens only on
//! the machine its id names; its alias keeps selecting it.

use super::device_kit::{KitDevice, established_on};
use super::*;
use crate::model::{DeviceNaming, LinkOrigin};
use hide_node_link::protocol::MachineIdentity;

const ALIAS: &str = "mini";
const MACHINE: &str = "b06a260a-1c1f-5d2e-9c4a-6f0e1d2c3b4a";
const OTHER: &str = "ebcd0d83-7f6a-4e1b-8c2d-3a4b5c6d7e8f";

/// Adds `alias` as Add device does, with Hide's helper allowed; the id the
/// core gave it.
fn added(runtime: &mut Runtime, alias: &str) -> String {
    let event = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "register_device",
        "payload": { "label": alias, "ssh_alias": alias, "host_consent": true },
    });
    runtime.dispatch_json(&serde_json::to_vec(&event).unwrap());
    let registration = runtime
        .snapshot
        .ui_state
        .device_registrations
        .iter()
        .rev()
        .find(|registration| registration.origin.ssh_alias() == Some(alias))
        .expect("registered");
    assert_eq!(
        registration.origin.naming(),
        Some(DeviceNaming::Pending),
        "a device this build adds waits for its name"
    );
    registration.id.clone()
}

/// One connection to `device_id` whose bootstrap probe read `probe` and
/// whose node reports `hello`.
fn connected(
    runtime: &mut Runtime,
    device_id: &str,
    probe: Option<&str>,
    hello: Result<&str, &str>,
) -> bool {
    runtime.start_device_host(device_id, hosts::HostAttempt::Background);
    let generation = runtime.device_host_generation(device_id);
    let mut established = established_on(
        probe.unwrap_or(MACHINE),
        KitDevice::answering(Err("not asked".to_owned())),
        None,
    );
    established.identity.machine_id = probe.map(str::to_owned);
    established.node.hello.machine_identity = match hello {
        Ok(id) => MachineIdentity::Available { id: id.to_owned() },
        Err(reason) => MachineIdentity::Unavailable {
            reason: reason.to_owned(),
        },
    };
    runtime.ingest_host_established(
        device_id,
        generation,
        Ok(hosts::HostReady::Dialed(Box::new(established))),
    )
}

fn ids(runtime: &Runtime) -> Vec<(String, Option<DeviceNaming>)> {
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .iter()
        .map(|registration| (registration.id.clone(), registration.origin.naming()))
        .collect()
}

fn row(runtime: &mut Runtime, device_id: &str) -> DeviceSnapshot {
    runtime
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == device_id)
        .expect("device row")
        .clone()
}

/// Why the device did not connect, before any link.
fn row_message(runtime: &mut Runtime, device_id: &str) -> Option<String> {
    row(runtime, device_id).message
}

/// Why the device's link was closed.
fn link_message(runtime: &mut Runtime, device_id: &str) -> Option<String> {
    row(runtime, device_id).host.message
}

fn no_retry_waits(runtime: &Runtime, device_id: &str) -> bool {
    runtime
        .device_host_retries
        .get(device_id)
        .is_none_or(|retry| retry.at_unix_ms.is_none())
}

/// A registration put in the UI state as an earlier build or a finished
/// naming left it.
fn stored(runtime: &mut Runtime, id: &str, naming: DeviceNaming, bound: Option<&str>) {
    let mut consent = runtime.new_host_consent();
    consent.identity = Some(hide_node_link::device::HostIdentity {
        user: "me".to_owned(),
        hostname: "mini.local".to_owned(),
        port: 22,
        machine_id: bound.map(str::to_owned),
        host_key_sha256: "key".to_owned(),
    });
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: id.to_owned(),
            label: ALIAS.to_owned(),
            origin: LinkOrigin::Dialed {
                ssh_alias: ALIAS.to_owned(),
                naming,
            },
            host_consent: Some(consent),
        });
    runtime.rebuild_device_rows();
}

#[test]
fn a_new_device_takes_its_machine_id_on_its_first_connection_and_keeps_its_alias() {
    let mut runtime = runtime();
    let provisional = added(&mut runtime, ALIAS);
    assert_eq!(provisional, ALIAS);
    let focus = serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_device",
        "payload": { "device_id": ALIAS },
    });
    runtime.dispatch_json(&serde_json::to_vec(&focus).unwrap());
    assert_eq!(runtime.look.front(), Some(ALIAS));

    let (_, records) = crate::diagnostics::capture(|| {
        assert!(connected(
            &mut runtime,
            &provisional,
            Some(MACHINE),
            Ok(MACHINE)
        ));
    });

    assert_eq!(
        ids(&runtime),
        [(MACHINE.to_owned(), Some(DeviceNaming::Named))]
    );
    let registration = runtime.device_registration(MACHINE).unwrap();
    assert_eq!(registration.origin.ssh_alias(), Some(ALIAS));
    assert_eq!(registration.label, ALIAS);
    assert!(records.iter().any(|record| record["kind"] == "device.named"
        && record["from"] == ALIAS
        && record["to"] == MACHINE));
    // The saved state names it by its machine, and nothing under the alias
    // is left open.
    let (saved, _, _) = crate::persistence::load(&runtime.state_path);
    assert_eq!(saved.device_registrations[0].id, MACHINE);
    assert!(runtime.device_names_saving.is_empty());
    assert!(
        runtime
            .snapshot()
            .status
            .remote
            .iter()
            .all(|remote| remote.target_id == MACHINE),
        "{:?}",
        runtime.snapshot().status.remote
    );
    assert!(!runtime.device_hosts.contains_key(ALIAS));
    // In front before, in front under its new id.
    assert_eq!(runtime.look.front(), Some(MACHINE));
    assert_eq!(
        runtime.snapshot.navigator.focused_device_id.as_deref(),
        Some(MACHINE)
    );
    // The alias still selects it, as `--machine` and `hide device` name it.
    assert_eq!(runtime.machine_named(ALIAS), Ok(MACHINE.to_owned()));
    assert_eq!(runtime.machine_named(MACHINE), Ok(MACHINE.to_owned()));
}

/// The same machine is called by one id as a core and as a device: the id a
/// device's helper reports is the one `NodeId::of_this_machine` gives the
/// same machine's core, so every key made from it (a Memory project, a
/// project's settings) is the same in both.
#[test]
fn a_machine_named_as_a_device_has_the_id_it_has_as_a_core() {
    let as_core = crate::node::NodeId::of_this_machine().unwrap();
    // What the device's helper puts in its Hello (`hide-host` serve).
    let reported = hide_platform::host::machine_id().unwrap();
    let mut runtime = runtime();
    let provisional = added(&mut runtime, ALIAS);
    let probe = reported.to_lowercase();
    assert!(connected(
        &mut runtime,
        &provisional,
        Some(&probe),
        Ok(&reported)
    ));

    let device = runtime.snapshot.ui_state.device_registrations[0].id.clone();
    assert_eq!(device, as_core.as_str());
    assert_eq!(
        hide_project::project_id(&device, std::path::Path::new("/srv/p")),
        hide_project::project_id(as_core.as_str(), std::path::Path::new("/srv/p"))
    );
    assert_eq!(
        crate::project_settings::key(&device, "/srv/p"),
        crate::project_settings::key(as_core.as_str(), "/srv/p")
    );
}

/// Until the core state that names the device by its machine is saved,
/// nothing opens under the new id: a connection is held with the reason on
/// its row, and the save's end connects it.
#[test]
fn a_renamed_device_connects_only_once_the_state_naming_it_is_saved() {
    let shared = SharedRuntime::new(runtime());
    let mut runtime = shared.lock().unwrap();
    runtime.install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
    let provisional = added(&mut runtime, ALIAS);
    assert!(connected(
        &mut runtime,
        &provisional,
        Some(MACHINE),
        Ok(MACHINE)
    ));
    // The save runs on its own thread, which waits for this lock.
    assert!(runtime.device_names_saving.contains_key(MACHINE));
    let registration = runtime.device_registration(MACHINE).cloned().unwrap();
    runtime.connect_remote_device(&registration, hosts::HostAttempt::Operator);
    let held = row_message(&mut runtime, MACHINE).expect("reason");
    assert!(held.starts_with("Saving this device's new name"), "{held}");
    assert!(!runtime.remote_connections.contains_key(MACHINE));
    drop(runtime);

    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.lock().unwrap().device_names_saving.is_empty() {
        assert!(Instant::now() < deadline, "the save never ended");
        #[allow(clippy::disallowed_methods)] // polls the save thread, bounded by the deadline
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut runtime = shared.lock().unwrap();
    let message = row_message(&mut runtime, MACHINE).unwrap_or_default();
    assert!(
        !message.starts_with("Saving this device's new name"),
        "{message}"
    );
    let (saved, _, _) = crate::persistence::load(&runtime.state_path);
    assert_eq!(saved.device_registrations[0].id, MACHINE);
}

/// A save that fails keeps the device waiting; the operator's Retry, which
/// may name the device by its alias, asks for the save again, and the save
/// connects it.
#[test]
fn a_failed_save_keeps_the_device_waiting_until_a_retry_saves_it() {
    let mut runtime = runtime();
    let state_path = runtime.state_path.clone();
    let blocker = tempfile::NamedTempFile::new().unwrap();
    runtime.state_path = blocker.path().join("state.json");
    let provisional = added(&mut runtime, ALIAS);
    assert!(connected(
        &mut runtime,
        &provisional,
        Some(MACHINE),
        Ok(MACHINE)
    ));
    assert!(runtime.device_names_saving.contains_key(MACHINE));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("ui_state.save_failed")
    );

    runtime.state_path = state_path;
    runtime
        .device_command(
            crate::node::TEST_NODE,
            crate::device_command::DeviceCommand::Retry {
                device_id: ALIAS.into(),
            },
        )
        .expect("retried by its alias");
    assert!(runtime.device_names_saving.is_empty());
    let (saved, _, _) = crate::persistence::load(&runtime.state_path);
    assert_eq!(saved.device_registrations[0].id, MACHINE);
}

/// The core state as a save takes it: the state and the naming round.
fn taken(runtime: &Runtime) -> (crate::model::UiStateSnapshot, u64) {
    (runtime.ui_state_to_save(), runtime.device_naming_round)
}

/// Saves fail until the returned path is put back as the state path.
fn saves_failing(runtime: &mut Runtime) -> (std::path::PathBuf, tempfile::NamedTempFile) {
    let state_path = runtime.state_path.clone();
    let blocker = tempfile::NamedTempFile::new().unwrap();
    runtime.state_path = blocker.path().join("state.json");
    (state_path, blocker)
}

fn removed(runtime: &mut Runtime, name: &str) {
    runtime
        .device_command(
            crate::node::TEST_NODE,
            crate::device_command::DeviceCommand::Remove {
                device_id: name.into(),
            },
        )
        .expect("removed");
}

fn saving_held(runtime: &mut Runtime) -> bool {
    row_message(runtime, MACHINE)
        .is_some_and(|message| message.starts_with("Saving this device's new name"))
}

/// A save the worker took before the device was removed, added again and
/// named again ends after that: it holds the machine's id named, but not
/// this naming, so the device keeps waiting; a failed save of this naming
/// keeps it waiting too, and only a save of it connects it.
#[test]
fn a_save_taken_before_a_device_was_named_again_does_not_connect_it() {
    let mut runtime = runtime();
    let (_state_path, _blocker) = saves_failing(&mut runtime);
    let provisional = added(&mut runtime, ALIAS);
    connected(&mut runtime, &provisional, Some(MACHINE), Ok(MACHINE));
    let (first, first_round) = taken(&runtime);
    assert!(first.device_registrations.iter().any(|registration| {
        registration.id == MACHINE && registration.origin.naming() == Some(DeviceNaming::Named)
    }));

    removed(&mut runtime, ALIAS);
    assert!(runtime.device_names_saving.is_empty());
    assert!(!runtime.acknowledge_device_naming(&first, first_round, true));
    assert!(runtime.device_registration(MACHINE).is_none());

    let again = added(&mut runtime, ALIAS);
    connected(&mut runtime, &again, Some(MACHINE), Ok(MACHINE));
    assert!(runtime.device_names_saving.contains_key(MACHINE));
    let (second, second_round) = taken(&runtime);
    assert!(second_round > first_round, "rounds are never reused");

    assert!(!runtime.acknowledge_device_naming(&first, first_round, true));
    assert!(runtime.device_names_saving.contains_key(MACHINE));
    let registration = runtime.device_registration(MACHINE).cloned().unwrap();
    runtime.connect_remote_device(&registration, hosts::HostAttempt::Operator);
    assert!(saving_held(&mut runtime));

    assert!(!runtime.acknowledge_device_naming(&second, second_round, false));
    assert!(runtime.device_names_saving.contains_key(MACHINE));

    runtime.acknowledge_device_naming(&second, second_round, true);
    assert!(runtime.device_names_saving.is_empty());
    assert!(!saving_held(&mut runtime));
}

/// Taking Hide's helper's consent back while the name saves leaves the
/// name waiting for its save; the save then lets the device connect, and
/// its consent, gone, opens no helper until it is given again.
#[test]
fn a_device_whose_consent_was_taken_back_while_its_name_saved_opens_no_helper() {
    let mut runtime = runtime();
    let (state_path, _blocker) = saves_failing(&mut runtime);
    let provisional = added(&mut runtime, ALIAS);
    connected(&mut runtime, &provisional, Some(MACHINE), Ok(MACHINE));
    assert!(runtime.set_host_consent(MACHINE, false));
    assert!(runtime.device_names_saving.contains_key(MACHINE));

    runtime.state_path = state_path;
    runtime.persist_current_ui_state();
    assert!(runtime.device_names_saving.is_empty());
    let host = row(&mut runtime, MACHINE).host;
    assert_eq!(host.state, "not_allowed", "{host:?}");
    assert!(
        !runtime
            .device_hosts
            .get(MACHINE)
            .is_some_and(|host| matches!(host.phase, hosts::HostPhase::Ready { .. }))
    );

    assert!(runtime.set_host_consent(MACHINE, true));
    let host = row(&mut runtime, MACHINE).host;
    assert_ne!(host.state, "not_allowed", "{host:?}");
}

/// A new device is named only by a machine both readings agree on that is
/// not already this core or another device; otherwise it keeps its alias's
/// id, opens nothing and says why.
#[test]
fn a_new_device_is_held_when_its_machine_cannot_name_it() {
    let own = runtime().node.as_str().to_owned();
    let cases: [(Option<&str>, Result<&str, &str>, &str); 5] = [
        (
            Some(MACHINE),
            Err("protocol 24"),
            "could not read its machine id",
        ),
        (Some(MACHINE), Ok(OTHER), "but the sign-in reached"),
        (None, Ok(MACHINE), "but the sign-in reached"),
        (Some("Not An Id!"), Ok("Not An Id!"), "which is not an id"),
        (Some(&own), Ok(&own), "the machine this core runs on"),
    ];
    for (probe, hello, reason) in cases {
        let mut runtime = runtime();
        let provisional = added(&mut runtime, ALIAS);
        connected(&mut runtime, &provisional, probe, hello);
        assert_eq!(
            ids(&runtime),
            [(ALIAS.to_owned(), Some(DeviceNaming::Pending))],
            "{reason}"
        );
        let message = link_message(&mut runtime, ALIAS).expect("reason");
        assert!(message.contains(reason), "{message}");
        assert!(no_retry_waits(&runtime, ALIAS), "{reason}");
    }

    // The machine is already added under its own id.
    let mut runtime = runtime();
    stored(&mut runtime, MACHINE, DeviceNaming::Named, Some(MACHINE));
    let second = added(&mut runtime, ALIAS);
    assert_ne!(second, MACHINE);
    connected(&mut runtime, &second, Some(MACHINE), Ok(MACHINE));
    assert!(runtime.device_registration(&second).is_some());
    let message = link_message(&mut runtime, &second).expect("reason");
    assert!(message.contains("already added as"), "{message}");
}

/// A named device opens only on the machine its id names: an alias that now
/// reaches another machine is held, never retried, and asks for that
/// machine to be added as a device of its own.
#[test]
fn a_named_device_opens_only_on_its_own_machine() {
    let mut runtime = runtime();
    stored(&mut runtime, MACHINE, DeviceNaming::Named, Some(MACHINE));
    connected(&mut runtime, MACHINE, Some(OTHER), Ok(OTHER));
    let message = link_message(&mut runtime, MACHINE).expect("reason");
    assert!(
        message.contains(&format!("another machine ({OTHER})")),
        "{message}"
    );
    assert!(no_retry_waits(&runtime, MACHINE));

    // A helper that cannot read its machine id is let through only when
    // this connection's probe read the device's own machine.
    let mut runtime = super::runtime();
    stored(&mut runtime, MACHINE, DeviceNaming::Named, Some(MACHINE));
    connected(&mut runtime, MACHINE, Some(MACHINE), Err("protocol 24"));
    assert!(matches!(
        runtime.device_hosts[MACHINE].phase,
        hosts::HostPhase::Ready { .. }
    ));
}

/// A device an earlier build added under its alias's id opens nothing once
/// its consent holds its machine id: the start's renaming did not take it
/// (`node_migration::name_devices`), or this connection just bound the
/// machine and the next start names it.
#[test]
fn a_device_from_an_earlier_build_waits_for_the_start_to_name_it() {
    let mut runtime = runtime();
    stored(&mut runtime, ALIAS, DeviceNaming::Legacy, Some(MACHINE));
    let (_, records) = crate::diagnostics::capture(|| {
        runtime.connect_registered_devices();
    });
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "device.name_held" && record["target"] == ALIAS)
    );
    assert!(!runtime.remote_connections.contains_key(ALIAS));
    let message = row_message(&mut runtime, ALIAS).expect("reason");
    assert!(message.contains("could not be moved"), "{message}");

    // One whose consent never held a machine id binds it on this
    // connection, then waits for the next start.
    let mut runtime = super::runtime();
    stored(&mut runtime, ALIAS, DeviceNaming::Legacy, None);
    connected(&mut runtime, ALIAS, Some(MACHINE), Ok(MACHINE));
    assert_eq!(
        ids(&runtime),
        [(ALIAS.to_owned(), Some(DeviceNaming::Legacy))]
    );
    let bound = runtime.device_registration(ALIAS).unwrap();
    assert_eq!(
        bound
            .host_consent
            .as_ref()
            .and_then(|consent| consent.identity.as_ref())
            .and_then(|identity| identity.machine_id.as_deref()),
        Some(MACHINE)
    );
    assert!(no_retry_waits(&runtime, ALIAS));
    let registration = bound.clone();
    runtime.connect_remote_device(&registration, hosts::HostAttempt::Operator);
    let message = row_message(&mut runtime, ALIAS).expect("reason");
    assert!(message.starts_with("Restart Hide"), "{message}");
}

/// An alias two devices share names neither: the refusal lists both ids.
/// A name that is no node, device or alias is answered as given, for its
/// caller to refuse.
#[test]
fn an_alias_two_devices_share_is_refused_with_their_ids() {
    let mut runtime = runtime();
    stored(&mut runtime, MACHINE, DeviceNaming::Named, Some(MACHINE));
    stored(&mut runtime, OTHER, DeviceNaming::Named, Some(OTHER));
    let refusal = runtime.machine_named(ALIAS).unwrap_err();
    assert!(refusal.starts_with("machine_unknown"), "{refusal}");
    assert!(
        refusal.contains(MACHINE) && refusal.contains(OTHER),
        "{refusal}"
    );
    assert_eq!(runtime.machine_named("nowhere"), Ok("nowhere".to_owned()));
    let own = runtime.node.as_str().to_owned();
    assert_eq!(runtime.machine_named(&own), Ok(own.clone()));
}

/// `--machine <alias>` reaches the device the alias names wherever an agent
/// names a machine: a catalog asked of a device that is not connected says
/// so rather than that the machine is unknown.
#[test]
fn an_agents_machine_given_as_an_alias_is_the_device() {
    let mut runtime = runtime();
    stored(&mut runtime, MACHINE, DeviceNaming::Named, Some(MACHINE));
    runtime.connect_registered_devices();
    let node = runtime.node.as_str().to_owned();
    assert_eq!(
        runtime.agent_catalog(&node, Some(ALIAS), false),
        Err("machine_unavailable".to_owned())
    );
    assert_eq!(
        runtime.agent_catalog(&node, Some("nowhere"), false),
        Err("machine_unknown".to_owned())
    );
}
