//! Hide AI on a connected device's agents (PRD wsl-device B11 to B13): the
//! device's agents are offered in Runs on, a first run prefers an agent
//! signed in here, the operator's choice holds, and a chosen device that is
//! not connected reads as unavailable without being woken and answers again
//! once its link is back.

use super::agents_settings_remote::ready_read;
use super::inbound::{held_link, shared_runtime};
use super::*;

const DEVICE: &str = "studio-node-1";

/// Switches the named agents on, and the others off, in `machine`'s kit.
fn kit_on(runtime: &mut Runtime, machine: &str, ids: &[&str]) {
    let mut kit = runtime.kit_state(machine);
    kit.agents = hide_kit::agents::ADAPTERS
        .iter()
        .map(|adapter| crate::model::KitAgentSnapshot {
            id: adapter.id.to_owned(),
            label: adapter.label.to_owned(),
            availability: hide_kit::Availability::Available,
            enabled: ids.contains(&adapter.id),
            chosen: false,
            skill: crate::model::KitPieceSnapshot {
                state: hide_kit::ComponentState::Installed,
                why: None,
                location: None,
            },
            hook: None,
            herdr: None,
            partial: false,
            features: Vec::new(),
            sessions: None,
            update: None,
            doc_url: String::new(),
        })
        .collect();
    runtime.set_kit_state(machine, kit);
}

/// A read in which `ready` are signed in and `out` are not, here or on
/// `device`.
fn read(device: Option<&str>, ready: &[&str], out: &[&str]) -> crate::model::BackgroundAiSnapshot {
    let mut snapshot = ready_read(ready);
    for id in out {
        let provider = hide_ai::ProviderId::from_id(id).expect("a registered agent");
        snapshot
            .providers
            .push(crate::model::BackgroundAiProviderSnapshot {
                state: "needs_login".to_owned(),
                headline: "Sign in required".to_owned(),
                ..crate::model::BackgroundAiProviderSnapshot::unread(provider)
            });
    }
    for row in &mut snapshot.providers {
        row.device_id = device.map(str::to_owned);
    }
    snapshot
}

fn joined(
    mut here: crate::model::BackgroundAiSnapshot,
    there: crate::model::BackgroundAiSnapshot,
) -> crate::model::BackgroundAiSnapshot {
    here.providers.extend(there.providers);
    here
}

fn row<'a>(
    runtime: &'a Runtime,
    id: &str,
    device: Option<&str>,
) -> Option<&'a crate::model::BackgroundAiProviderSnapshot> {
    runtime
        .snapshot
        .status
        .background_ai
        .providers
        .iter()
        .find(|row| row.id == id && row.device_id.as_deref() == device)
}

/// A runtime with `DEVICE` connected over a link that holds until the
/// sender is dropped.
fn connected() -> (SharedRuntime, std::sync::mpsc::Sender<()>) {
    let shared = shared_runtime();
    let (link, release) = held_link();
    shared
        .lock()
        .unwrap()
        .accept_inbound_node(DEVICE, "Studio", link)
        .expect("the link is taken");
    wait(&shared, "the device's link", |runtime| {
        runtime.linked_node(DEVICE).is_ok()
    });
    (shared, release)
}

fn ai_event(payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": "ai_settings", "payload": payload
    }))
    .unwrap()
}

/// B11: a device's agents are asked and offered beside this machine's; a
/// first run chooses the device's only once every agent switched on here
/// has answered and none is signed in, and a later sign-in here does not
/// move the choice.
#[test]
fn a_device_agent_is_offered_and_chosen_first_only_when_none_here_is_signed_in() {
    let (shared, release) = connected();
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    kit_on(&mut runtime, crate::node::TEST_NODE, &["codex"]);
    kit_on(&mut runtime, DEVICE, &["codex"]);

    let asked = runtime.ai_request();
    assert_eq!(
        asked.devices,
        vec![crate::ai::DeviceAsk {
            id: DEVICE.to_owned(),
            selecting: [hide_ai::ProviderId::CODEX].into(),
            cli_found: Some(hide_ai::PROVIDERS.iter().copied().collect()),
        }],
        "the connected device's switched-on agents are asked too"
    );

    // The device answers first: nothing here has, so nothing is chosen.
    runtime.ingest_background_ai(read(Some(DEVICE), &["codex"], &[]));
    assert!(runtime.snapshot.status.background_ai.provider.is_none());
    assert!(runtime.take_ai_settings_save().is_none());
    assert!(
        row(&runtime, "codex", Some(DEVICE)).is_some_and(|row| row.selectable),
        "the device's signed-in agent is offered in Runs on"
    );

    // Here it needs a sign-in: the device's agent is chosen, on the device.
    runtime.ingest_background_ai(joined(
        read(None, &[], &["codex"]),
        read(Some(DEVICE), &["codex"], &[]),
    ));
    let section = &runtime.snapshot.status.background_ai;
    assert_eq!(section.provider.as_deref(), Some("codex"));
    assert_eq!(section.device.as_deref(), Some(DEVICE));
    let saved = runtime
        .take_ai_settings_save()
        .expect("the choice is stored");
    assert_eq!(saved.machine(hide_ai::ProviderId::CODEX), Some(DEVICE));

    // Signing in here later leaves the stored choice where it is.
    runtime.ingest_background_ai(joined(
        read(None, &["codex"], &[]),
        read(Some(DEVICE), &["codex"], &[]),
    ));
    assert_eq!(
        runtime.snapshot.status.background_ai.device.as_deref(),
        Some(DEVICE)
    );
    assert!(runtime.take_ai_settings_save().is_none());

    // The operator moves it here, and back to the device, by choosing rows.
    assert!(runtime.dispatch_json(&ai_event(
        serde_json::json!({"provider": "codex", "device": null})
    )));
    assert_eq!(runtime.snapshot.status.background_ai.device, None);
    assert!(runtime.dispatch_json(&ai_event(
        serde_json::json!({"provider": "codex", "device": DEVICE})
    )));
    assert_eq!(
        runtime.snapshot.status.background_ai.device.as_deref(),
        Some(DEVICE)
    );
    let saved = runtime.take_ai_settings_save().expect("the move is stored");
    assert_eq!(saved.machine(hide_ai::ProviderId::CODEX), Some(DEVICE));
    drop(runtime);
    drop(release);
}

/// B11: with an agent signed in here and on a device in the same read, the
/// first run chooses the one here.
#[test]
fn a_first_run_prefers_an_agent_signed_in_on_this_machine() {
    let (shared, release) = connected();
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
    kit_on(&mut runtime, crate::node::TEST_NODE, &["codex", "opencode"]);
    kit_on(&mut runtime, DEVICE, &["codex"]);
    runtime.ingest_background_ai(joined(
        read(None, &["opencode"], &["codex"]),
        read(Some(DEVICE), &["codex"], &[]),
    ));
    let section = &runtime.snapshot.status.background_ai;
    assert_eq!(section.provider.as_deref(), Some("opencode"));
    assert_eq!(section.device, None, "the agent here is preferred");
    drop(runtime);
    drop(release);
}

/// B13: a chosen device whose link ended reads as unavailable and stays
/// chosen, is not asked and not woken, cannot be chosen again until it is
/// back, and answers through its next link once it connects.
#[test]
fn a_chosen_device_that_is_not_connected_is_unavailable_and_answers_again_once_back() {
    let (shared, release) = connected();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
        runtime.ingest_background_ai(joined(
            read(None, &["opencode"], &[]),
            read(Some(DEVICE), &["codex"], &[]),
        ));
        assert!(runtime.dispatch_json(&ai_event(
            serde_json::json!({"provider": "codex", "device": DEVICE})
        )));
        assert_eq!(
            runtime.snapshot.status.background_ai.device.as_deref(),
            Some(DEVICE)
        );
    }
    let machines = shared.lock().unwrap().ai_machines();
    assert!((machines.devices)(DEVICE).is_some(), "asked over its link");

    drop(release);
    wait(&shared, "the device's link ending", |runtime| {
        runtime.linked_node(DEVICE).is_err()
    });
    {
        let mut runtime = shared.lock().unwrap();
        let away = row(&runtime, "codex", Some(DEVICE)).expect("Runs on keeps its row");
        assert_eq!(away.state, "unavailable");
        assert_eq!(away.message.as_deref(), Some("device_not_connected"));
        assert!(!away.selectable);
        assert_eq!(
            runtime.snapshot.status.background_ai.device.as_deref(),
            Some(DEVICE),
            "the operator's choice stays"
        );
        assert!(
            runtime.ai_request().devices.is_empty(),
            "a device that is not connected is not asked"
        );
        assert!(runtime.dispatch_json(&ai_event(serde_json::json!({"provider": "opencode"}))));
        assert_eq!(runtime.snapshot.status.background_ai.device, None);
        runtime.dispatch_json(&ai_event(
            serde_json::json!({"provider": "codex", "device": DEVICE}),
        ));
        assert_eq!(
            runtime
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("ai_settings.provider_not_selectable"),
            "an agent on a device that is not connected cannot be chosen"
        );
    }
    assert!((machines.devices)(DEVICE).is_none(), "nothing to ask");
    assert!(
        !shared.lock().unwrap().device_host_connecting(DEVICE),
        "and nothing woke it"
    );

    let (next, release) = held_link();
    shared
        .lock()
        .unwrap()
        .accept_inbound_node(DEVICE, "Studio", next)
        .expect("the next link is taken");
    wait(&shared, "the device's next link", |runtime| {
        runtime.linked_node(DEVICE).is_ok()
    });
    let current = shared.lock().unwrap().linked_node(DEVICE).unwrap().1;
    let asked = (machines.devices)(DEVICE).expect("asked again once back");
    assert!(Arc::ptr_eq(&asked, &current), "through its next link");
    {
        let mut runtime = shared.lock().unwrap();
        assert_eq!(runtime.ai_request().devices.len(), 1);
        assert!(runtime.dispatch_json(&ai_event(
            serde_json::json!({"provider": "codex", "device": DEVICE})
        )));
        assert_eq!(
            runtime.snapshot.status.background_ai.device.as_deref(),
            Some(DEVICE)
        );
    }
    drop(release);
}

/// A request of `feature`, as Hide AI's work asks one.
fn request(feature: &'static str) -> hide_ai::AiRequest {
    hide_ai::AiRequest {
        feature_id: feature.into(),
        request_id: hide_ai::RequestId(format!("{feature}-1")),
        subject_id: "1".to_owned(),
        system: String::new(),
        input: String::new(),
        output_schema: serde_json::json!({ "type": "object" }),
        deadline: Duration::from_secs(5),
        schema_version: "1".into(),
        pick: None,
        output_language: None,
        answer: hide_ai::AnswerMode::PlainJson,
        provider_efforts: Vec::new(),
    }
}

/// B11: the operator's choice of an agent on a device holds across a
/// restart. The settings file one run saves brings the next run up on the
/// same agent on the same device, its row shown there, and nothing is
/// chosen again once the device's agents answer.
#[test]
fn a_device_agent_chosen_before_a_restart_is_chosen_after_it() {
    let home = tempfile::tempdir().unwrap();
    {
        let (shared, release) = connected();
        let mut runtime = shared.lock().unwrap();
        runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
        runtime.ingest_background_ai(joined(
            read(None, &["opencode"], &[]),
            read(Some(DEVICE), &["codex"], &[]),
        ));
        assert!(runtime.dispatch_json(&ai_event(
            serde_json::json!({"provider": "codex", "device": DEVICE})
        )));
        let saved = runtime
            .take_ai_settings_save()
            .expect("the choice is stored");
        hide_ai::settings::save(home.path(), &saved).unwrap();
        drop(runtime);
        drop(release);
    }

    // The next run reads the file before any agent answers, as the daemon
    // does when it starts.
    let (shared, release) = connected();
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_ai_settings(hide_ai::settings::load(home.path()).unwrap(), None);
    let section = &runtime.snapshot.status.background_ai;
    assert_eq!(section.provider.as_deref(), Some("codex"));
    assert_eq!(section.device.as_deref(), Some(DEVICE));
    assert!(
        row(&runtime, "codex", Some(DEVICE)).is_some(),
        "Runs on shows the chosen agent on its device before it answers"
    );
    runtime.ingest_background_ai(joined(
        read(None, &["opencode"], &[]),
        read(Some(DEVICE), &["codex"], &[]),
    ));
    assert!(row(&runtime, "codex", Some(DEVICE)).is_some_and(|row| row.selectable));
    assert_eq!(
        runtime.snapshot.status.background_ai.device.as_deref(),
        Some(DEVICE)
    );
    assert!(
        runtime.take_ai_settings_save().is_none(),
        "nothing is chosen again"
    );
    drop(runtime);
    drop(release);
}

/// B13: Hide AI's work for an agent on a device whose link is down never
/// starts that link. Labels, suggestions and Memory (which suggestions ask
/// through), and a worktree name each answer unavailable, and no attempt is
/// made: every attempt to start a device's link advances its host
/// generation before it reaches the device's transport, and the generation
/// stays where it was. A read that does start links starts this one at
/// once, so the count would show a wake.
#[test]
fn hide_ai_work_never_starts_the_link_of_a_chosen_device_that_is_down() {
    let ssh = super::device_kit::DEVICE;
    let shared = super::device_kit::with_consent(Some(super::device_kit::KitDevice::answering(
        Err("not asked".to_owned()),
    )));
    let settings = {
        let mut runtime = shared.lock().unwrap();
        runtime.ingest_ai_settings(hide_ai::AiSettings::default(), None);
        runtime.ingest_background_ai(joined(
            read(None, &["opencode"], &[]),
            read(Some(ssh), &["codex"], &[]),
        ));
        assert!(runtime.dispatch_json(&ai_event(
            serde_json::json!({"provider": "codex", "device": ssh})
        )));
        let settings = runtime
            .take_ai_settings_save()
            .expect("the choice is stored");
        // The link is lost, as its node's close reports it, and no retry
        // waits: whatever starts links would start this one now.
        let generation = runtime.device_host_generation(ssh);
        assert!(runtime.ingest_host_closed(ssh, generation, "lost".to_owned()));
        runtime.device_host_retries.remove(ssh);
        settings
    };
    let (machines, before) = {
        let runtime = shared.lock().unwrap();
        (runtime.ai_machines(), runtime.device_host_generation(ssh))
    };
    let usage = crate::usage_ledger::ai_usage::AiUsageQueue::new() as Arc<dyn hide_ai::AiUsageSink>;
    let gate = Arc::new(hide_ai::UsageLimits::new());
    for (feature, router) in [
        (
            "label",
            crate::ai::labels_router(&machines, &settings, &gate, &usage),
        ),
        (
            "suggestion or Memory analysis",
            crate::ai::memory_router(&machines, &settings, &gate, &usage),
        ),
        (
            "Memory add",
            crate::ai::memory_add_router(&machines, &settings, &usage),
        ),
    ] {
        assert!(
            router
                .execute(&request(feature), &hide_ai::CancelToken::new())
                .is_err(),
            "{feature} was answered while its device was down"
        );
    }
    assert!(
        crate::ai::suggest_worktree_name(&machines, &settings, &usage, "7", "", "Title", "")
            .is_err()
    );
    {
        let runtime = shared.lock().unwrap();
        assert_eq!(
            runtime.device_host_generation(ssh),
            before,
            "Hide AI started the device's link"
        );
        assert!(!runtime.device_host_connecting(ssh));
    }
    let _ = shared.lock().unwrap().node_link(ssh);
    assert_eq!(
        shared.lock().unwrap().device_host_generation(ssh),
        before + 1,
        "a read that starts links moves the count"
    );
}
