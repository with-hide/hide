//! A device's install kit as the runtime drives it (PRD device-parity B13,
//! B17, B19, B21-B24): the connection pass, the operator's Reinstall, the
//! consent a device needs first, a call that fails, and removal.

use super::*;
use crate::node_access::{LinkAnswer, LinkError, NodeLink};
use hide_kit::{ComponentId, ComponentReport, ComponentState, KitReport};
use hide_node_link::protocol::{Call, KitAction};
use std::sync::{Arc, Mutex};

const DEVICE: &str = "studio";

/// A device helper that answers only kit calls: a set report, and a set
/// removal outcome for `remove`.
pub(super) struct KitDevice {
    calls: Mutex<Vec<(KitAction, String, Option<String>)>>,
    retirement_projects: Mutex<Vec<Vec<String>>>,
    answer: Mutex<Result<KitReport, String>>,
    closed: Mutex<Option<String>>,
    /// When set, an apply or status call waits here until the test lets it
    /// answer, as a slow device would.
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl KitDevice {
    pub(super) fn answering(answer: Result<KitReport, String>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            retirement_projects: Mutex::new(Vec::new()),
            answer: Mutex::new(answer),
            closed: Mutex::new(None),
            gate: Mutex::new(None),
        })
    }

    /// A device whose next non-removal call answers only once the returned
    /// sender is used.
    fn held(answer: Result<KitReport, String>) -> (Arc<Self>, std::sync::mpsc::Sender<()>) {
        let (release, gate) = std::sync::mpsc::channel();
        let device = Self::answering(answer);
        *device.gate.lock().unwrap() = Some(gate);
        (device, release)
    }

    pub(super) fn calls(&self) -> Vec<(KitAction, String, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl NodeLink for KitDevice {
    fn call(&self, call: Call, _timeout: Duration) -> Result<LinkAnswer, LinkError> {
        let Call::Kit {
            action,
            cli_dir,
            herdr_socket,
            retirement_projects,
            ..
        } = call
        else {
            return Err(LinkError::NotConnected("kit calls only".to_owned()));
        };
        let removing = action == KitAction::Remove;
        self.retirement_projects
            .lock()
            .unwrap()
            .push(retirement_projects);
        self.calls
            .lock()
            .unwrap()
            .push((action, cli_dir, herdr_socket));
        if !removing && let Some(gate) = self.gate.lock().unwrap().take() {
            let _ = gate.recv();
        }
        if removing {
            let removed = hide_node_link::protocol::KitRemoved {
                kit: hide_kit::RemoveReport {
                    components: vec![(ComponentId::Cli, hide_kit::RemoveOutcome::Removed)],
                    agents: Vec::new(),
                },
                helper_root: hide_kit::RemoveOutcome::Removed,
            };
            return Ok(LinkAnswer::Parsed(serde_json::to_value(removed).unwrap()));
        }
        match &*self.answer.lock().unwrap() {
            Ok(report) => Ok(LinkAnswer::Parsed(serde_json::to_value(report).unwrap())),
            Err(reason) => Err(LinkError::Unknown(reason.clone())),
        }
    }

    fn closed_reason(&self) -> Option<String> {
        self.closed.lock().unwrap().clone()
    }

    fn close(&self, reason: &str) {
        *self.closed.lock().unwrap() = Some(reason.to_owned());
    }
}

pub(super) fn report(states: &[(ComponentId, ComponentState)]) -> KitReport {
    KitReport {
        components: states
            .iter()
            .map(|(id, state)| ComponentReport {
                id: *id,
                state: *state,
                reason: None,
                why: None,
                location: Some(format!("/home/me/{}", id.code())),
            })
            .collect(),
        agents: Vec::new(),
        held_for_onboarding: false,
        labels_retirement: Default::default(),
        legacy_retirement: Default::default(),
        codex_daemon: None,
        codex_daemon_on: None,
        codex_daemon_running: None,
        codex_daemon_unreadable: None,
        codex_capability_unread: None,
        codex_daemon_off: None,
        herdr: None,
        refused: None,
    }
}

/// Dispatches one event and answers whether the core would announce a change
/// to the shell for it.
fn dispatch(shared: &Mutex<Runtime>, kind: &str, payload: serde_json::Value) -> bool {
    let event =
        serde_json::json!({"schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload});
    shared
        .lock()
        .unwrap()
        .dispatch_json(&serde_json::to_vec(&event).unwrap())
}

/// A registered device with the given consent, optionally connected to
/// `helper`, with the kit workers able to run.
fn device_runtime(
    consent: Option<crate::model::HostConsent>,
    helper: Option<Arc<KitDevice>>,
) -> SharedRuntime {
    let shared = SharedRuntime::new(runtime());
    dispatch(
        &shared,
        "register_device",
        serde_json::json!({ "id": DEVICE, "label": DEVICE, "ssh_alias": DEVICE }),
    );
    {
        let mut runtime = shared.lock().unwrap();
        let registration = runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == DEVICE)
            .unwrap();
        registration.host_consent = consent;
        registration.herdr_socket_path = Some("~/.config/herdr/other.sock".to_owned());
        if let Some(helper) = helper {
            runtime.device_hosts.insert(
                DEVICE.to_owned(),
                hosts::DeviceHost {
                    phase: hosts::HostPhase::Ready {
                        host: helper,
                        platform: "macos aarch64".to_owned(),
                        helper_path: Some(
                            "/home/me/.local/share/hide/host-helper/0123456789abcdef/hided"
                                .to_owned(),
                        ),
                    },
                    generation: 1,
                },
            );
        }
        runtime.install_worker_context(shared.weak(), crate::handle::ChangeNotifier::noop());
        runtime.refresh_device_snapshots();
    }
    shared
}

fn granted(runtime: &Mutex<Runtime>) -> crate::model::HostConsent {
    runtime.lock().unwrap().new_host_consent()
}

fn kit(shared: &Mutex<Runtime>) -> crate::model::KitSnapshot {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == DEVICE)
        .expect("device row")
        .kit
        .clone()
}

/// Waits for the device's kit worker to finish what is queued.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn settle(shared: &Mutex<Runtime>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.lock().unwrap().device_kit_running.contains(DEVICE) {
        assert!(
            Instant::now() < deadline,
            "the device kit worker never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn with_consent(helper: Option<Arc<KitDevice>>) -> SharedRuntime {
    let probe = device_runtime(None, None);
    let consent = granted(&probe);
    device_runtime(Some(consent), helper)
}

/// B13: a connected device's kit is installed by the connection pass, with
/// the consent's command folder and the registration's Herdr socket, and its
/// row shows each part the way This Mac's does.
#[test]
fn a_connected_device_shows_its_kit_and_retirement_failure_with_recovery() {
    let mut answer = report(&[
        (ComponentId::Cli, ComponentState::Installed),
        (ComponentId::ClaudeCodeHook, ComponentState::Installed),
        (ComponentId::CodexHook, ComponentState::Absent),
        (ComponentId::CoordinationRetirement, ComponentState::Failed),
    ]);
    let recovery = "remove owned links failed: inspect the link, then retry retirement";
    answer.components.last_mut().unwrap().reason = Some(recovery.to_owned());
    answer.components.last_mut().unwrap().why = Some(hide_kit::PartWhy {
        cause: hide_kit::PartCause::NotChanged,
        value: None,
    });
    let helper = KitDevice::answering(Ok(answer));
    let shared = with_consent(Some(Arc::clone(&helper)));

    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Apply(hide_kit::Scope::automatic()));
    assert!(kit(&shared).busy, "the row says the install is running");
    settle(&shared);

    assert_eq!(
        helper.calls(),
        vec![(
            KitAction::Apply,
            crate::remote::DEFAULT_CLI_DIR.to_owned(),
            Some("~/.config/herdr/other.sock".to_owned()),
        )]
    );
    let kit = kit(&shared);
    assert!(!kit.busy);
    assert_eq!(kit.unavailable, None);
    assert_eq!(
        kit.components
            .iter()
            .map(|part| (part.id, part.state))
            .collect::<Vec<_>>(),
        vec![
            (ComponentId::Cli, ComponentState::Installed),
            (ComponentId::ClaudeCodeHook, ComponentState::Installed),
            (ComponentId::CodexHook, ComponentState::Absent),
            (ComponentId::CoordinationRetirement, ComponentState::Failed),
        ]
    );
    // The screen gets the cause; the kit's sentence is the log's.
    assert_eq!(
        kit.components
            .last()
            .unwrap()
            .why
            .as_ref()
            .map(|why| why.cause),
        Some(hide_kit::PartCause::NotChanged)
    );
    let wire = serde_json::to_string(&kit).unwrap();
    assert!(!wire.contains(recovery), "{wire}");
    assert!(kit.offers_reinstall);
}

/// B8: Reinstall on a device's row sends only the parts that need it.
#[test]
fn reinstall_on_a_device_retries_failed_retirement_and_restores_removed_hooks() {
    let helper = KitDevice::answering(Ok(report(&[
        (ComponentId::Cli, ComponentState::Installed),
        (ComponentId::ClaudeCodeHook, ComponentState::Removed),
        (ComponentId::CoordinationRetirement, ComponentState::Failed),
    ])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);

    dispatch(
        &shared,
        "kit_reinstall",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);
    let calls = helper.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].0, KitAction::Status);
    assert_eq!(
        calls[1].0,
        KitAction::Reinstall {
            components: vec![
                ComponentId::ClaudeCodeHook,
                ComponentId::CoordinationRetirement
            ],
            agents_on: Vec::new(),
            agents_off: Vec::new(),
            codex_daemon_off: false,
        }
    );
}

fn agent_report(
    id: &str,
    availability: hide_kit::Availability,
    enabled: bool,
    skill: ComponentState,
) -> hide_kit::AgentReport {
    let piece = |state| hide_kit::PieceReport {
        state,
        reason: None,
        why: None,
        location: None,
    };
    hide_kit::AgentReport {
        id: id.to_owned(),
        label: id.to_owned(),
        availability,
        enabled,
        chosen: false,
        skill: piece(skill),
        hook: None,
        herdr: None,
        doc_url: "https://example.test/skills".to_owned(),
    }
}

/// Issue #517: an agent's switch on a device's row sends that agent alone,
/// the same press again is already met, an agent that is not set up there has
/// nothing to switch, and Reinstall names only the agents that are on and
/// need it.
#[test]
fn an_agent_switch_on_a_device_sends_the_agent_and_repeats_nothing() {
    use hide_kit::Availability::{Available, NotInstalled};
    let mut answer = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    answer.agents = vec![
        agent_report("omp", Available, false, ComponentState::Off),
        agent_report("grok", NotInstalled, false, ComponentState::Off),
        agent_report("codex", Available, true, ComponentState::Removed),
    ];
    let helper = KitDevice::answering(Ok(answer));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);
    let kit = kit(&shared);
    assert_eq!(kit.agents.len(), 3);
    assert!(
        kit.offers_reinstall,
        "an agent that is on has a piece removed"
    );

    dispatch(
        &shared,
        "kit_agent_set",
        serde_json::json!({ "device_id": DEVICE, "agent": "omp", "enabled": true }),
    );
    settle(&shared);
    dispatch(
        &shared,
        "kit_agent_set",
        serde_json::json!({ "device_id": DEVICE, "agent": "grok", "enabled": true }),
    );
    dispatch(
        &shared,
        "kit_agent_set",
        serde_json::json!({ "device_id": DEVICE, "agent": "omp", "enabled": false }),
    );
    settle(&shared);
    dispatch(
        &shared,
        "kit_reinstall",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);

    let actions: Vec<KitAction> = helper.calls().into_iter().map(|call| call.0).collect();
    let agents = |on: &[&str]| KitAction::Reinstall {
        components: Vec::new(),
        agents_on: on.iter().map(|id| (*id).to_owned()).collect(),
        agents_off: Vec::new(),
        codex_daemon_off: false,
    };
    assert_eq!(
        actions,
        vec![
            KitAction::Status,
            agents(&["omp"]),
            // The agent not set up there sent nothing, and this device's
            // report still says omp is off, so switching it off is met.
            agents(&["codex"]),
        ],
        "{actions:?}"
    );
}

/// PRD settings-cleanup D-13: a device's Herdr integration for an agent is a
/// piece of that agent's row, a failed one offers Reinstall for that agent
/// alone, and a row from a helper whose build predates the piece carries none.
#[test]
fn a_failed_herdr_integration_shows_on_its_agent_and_offers_reinstall() {
    use hide_kit::Availability::Available;
    let mut pi = agent_report("pi", Available, true, ComponentState::Installed);
    pi.herdr = Some(hide_kit::PieceReport {
        state: ComponentState::Failed,
        reason: Some("`herdr integration install pi` failed: disk full".to_owned()),
        why: Some(hide_kit::PartWhy {
            cause: hide_kit::PartCause::NotChanged,
            value: None,
        }),
        location: Some("/home/me/.pi/agent".to_owned()),
    });
    let grok = agent_report("grok", Available, true, ComponentState::Installed);
    let mut answer = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    answer.agents = vec![pi, grok];
    let shared = with_consent(Some(KitDevice::answering(Ok(answer))));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);

    let kit = kit(&shared);

    let piece = kit.agents[0].herdr.as_ref().expect("pi has an integration");
    assert_eq!(piece.state, ComponentState::Failed);
    assert_eq!(
        piece.why.as_ref().map(|why| why.cause),
        Some(hide_kit::PartCause::NotChanged)
    );
    assert!(kit.agents[0].needs_attention());
    assert!(kit.agents[1].herdr.is_none());
    assert!(!kit.agents[1].needs_attention());
    assert!(kit.offers_reinstall);
}

/// A pass that ran: it lists the agents, which a refused or failed pass never does.
fn ran(mut report: KitReport) -> KitReport {
    report.agents = vec![agent_report(
        "codex",
        hide_kit::Availability::Available,
        false,
        ComponentState::Off,
    )];
    report
}

fn held_report() -> KitReport {
    let mut report = ran(report(&[(ComponentId::Cli, ComponentState::Installed)]));
    report.held_for_onboarding = true;
    report
}

fn onboarding(shared: &Mutex<Runtime>) -> Option<crate::model::AgentOnboarding> {
    shared.lock().unwrap().snapshot.ui_state.agent_onboarding
}

/// First-run agent choice: this Mac's record decides whether to ask, Apply
/// switches the chosen agents on here and on every device that waits for it,
/// a second Apply is the same intent already met, and a device whose own
/// record waits later gets the choice once on its first report.
#[test]
fn the_first_run_choice_is_asked_once_applied_everywhere_and_remembered_for_later_devices() {
    use crate::model::AgentOnboarding::{Done, Pending};
    let helper = KitDevice::answering(Ok(ran(report(&[(
        ComponentId::Cli,
        ComponentState::Installed,
    )]))));
    let shared = with_consent(Some(Arc::clone(&helper)));
    assert_eq!(onboarding(&shared), None);

    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    assert_eq!(onboarding(&shared), Some(Pending));
    // A device whose record waits too is not installed to while the Mac asks.
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    settle(&shared);
    assert!(helper.calls().is_empty());

    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["codex", "claude-code", "codex"] }),
    );
    settle(&shared);
    assert_eq!(onboarding(&shared), Some(Done));
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .ui_state
            .agent_onboarding_agents,
        ["claude-code", "codex"]
    );
    let device_calls: Vec<KitAction> = helper.calls().into_iter().map(|call| call.0).collect();
    assert_eq!(
        device_calls,
        vec![KitAction::Reinstall {
            components: Vec::new(),
            agents_on: vec!["claude-code".to_owned(), "codex".to_owned()],
            // agy is on by default and was not chosen, so the answer names it off.
            agents_off: vec!["antigravity".to_owned()],
            codex_daemon_off: false,
        }]
    );
    let local = shared.lock().unwrap().local_kit_pending.clone().unwrap();
    assert_eq!(
        local.agent_on.iter().collect::<Vec<_>>(),
        ["claude-code", "codex"]
    );

    // The device's answered record clears its wait.
    assert!(shared.lock().unwrap().device_first_run_choice.is_empty());

    // The same press again, or a stale client, changes nothing.
    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["omp"] }),
    );
    settle(&shared);
    assert_eq!(helper.calls().len(), 1);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .ui_state
            .agent_onboarding_agents,
        ["claude-code", "codex"]
    );

    // A device whose own first pass held its agents gets the choice once.
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    settle(&shared);
    assert_eq!(helper.calls().len(), 2);
}

#[test]
fn a_machine_that_already_had_the_kit_never_asks() {
    use crate::model::AgentOnboarding::Done;
    let shared = with_consent(None);
    shared.lock().unwrap().ingest_kit_report(
        crate::node::TEST_NODE,
        &ran(report(&[(ComponentId::Cli, ComponentState::Installed)])),
    );
    assert_eq!(onboarding(&shared), Some(Done));
}

/// The question is the kit record's: a launch after a quit between the hold
/// and the answer, or after the core's own state was reset, still asks, and
/// an Apply whose record did not save asks again; an Apply whose install is
/// still queued is not undone by a report that was already on its way.
#[test]
fn the_question_comes_back_while_this_macs_record_still_waits() {
    use crate::model::AgentOnboarding::{Done, Pending};
    let shared = with_consent(None);
    let local = crate::node::TEST_NODE;
    // The core thinks the choice was made; the record says it was not.
    shared.lock().unwrap().snapshot.ui_state.agent_onboarding = Some(Done);
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(local, &held_report());
    assert_eq!(onboarding(&shared), Some(Pending));

    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["codex"] }),
    );
    assert_eq!(onboarding(&shared), Some(Done));
    // A report that was already on its way, while the install is queued.
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(local, &held_report());
    assert_eq!(onboarding(&shared), Some(Done));
    // The install ran and the record did not save.
    shared.lock().unwrap().local_kit_pending = None;
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(local, &held_report());
    assert_eq!(onboarding(&shared), Some(Pending));
}

/// Apply with every tile off is still an answer: the Mac and a waiting device
/// are sent the choice by naming the default agents off, so their records stop
/// waiting, and nothing is installed.
#[test]
fn applying_with_nothing_chosen_answers_the_question_here_and_on_a_waiting_device() {
    use crate::model::AgentOnboarding::Done;
    let shared = with_consent(None);
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": [] }),
    );
    assert_eq!(onboarding(&shared), Some(Done));
    let runtime = shared.lock().unwrap();
    assert!(runtime.snapshot.ui_state.agent_onboarding_agents.is_empty());
    let off = |scope: &hide_kit::Scope| scope.agent_off.iter().cloned().collect::<Vec<_>>();
    let local = runtime.local_kit_pending.clone().unwrap();
    assert!(local.agent_on.is_empty());
    assert_eq!(off(&local), ["antigravity", "claude-code", "codex"]);
    let Some(KitJob::Apply(device)) = runtime.device_kit_pending.get(DEVICE) else {
        panic!("the waiting device is sent the answer");
    };
    assert_eq!(off(device), ["antigravity", "claude-code", "codex"]);
}

/// A device that is not connected when Apply is pressed keeps waiting: the
/// choice is queued for it, and when the queued work is lost the next report
/// that still waits sends it again.
#[test]
fn a_device_that_was_not_ready_at_apply_still_receives_the_choice() {
    let shared = with_consent(None);
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["codex"] }),
    );
    assert!(
        shared
            .lock()
            .unwrap()
            .device_kit_pending
            .contains_key(DEVICE)
    );

    // The queued work is dropped (consent withdrawn, host gone); the device's
    // record still waits, so its next report brings the choice back.
    shared.lock().unwrap().forget_device_kit_work(DEVICE);
    assert!(
        !shared
            .lock()
            .unwrap()
            .device_kit_pending
            .contains_key(DEVICE)
    );
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    let runtime = shared.lock().unwrap();
    let Some(KitJob::Apply(scope)) = runtime.device_kit_pending.get(DEVICE) else {
        panic!("the device is sent the choice again");
    };
    assert!(scope.agent_on.contains("codex"), "{scope:?}");
}

/// A device whose record will not save keeps reporting that it waits; the
/// choice is sent once and the rest is logged, not queued on every report.
#[test]
fn a_device_that_keeps_waiting_is_sent_the_choice_once() {
    let helper = KitDevice::answering(Ok(held_report()));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["codex"] }),
    );
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    settle(&shared);
    for _ in 0..3 {
        shared
            .lock()
            .unwrap()
            .ingest_kit_report(DEVICE, &held_report());
        settle(&shared);
    }
    let sent = helper
        .calls()
        .into_iter()
        .filter(|call| matches!(call.0, KitAction::Reinstall { .. }))
        .count();
    assert_eq!(sent, 1, "{:?}", helper.calls());
}

#[test]
fn applying_an_agent_hide_does_not_know_is_refused() {
    let shared = with_consent(None);
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    dispatch(
        &shared,
        "agent_onboarding_apply",
        serde_json::json!({ "agents": ["not-an-agent"] }),
    );
    assert_eq!(
        onboarding(&shared),
        Some(crate::model::AgentOnboarding::Pending)
    );
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("kit.unknown_agent")
    );
}

/// B21: a device registered without consent installs nothing, its row says
/// what would install the kit, and Reinstall is refused rather than queued.
#[test]
fn a_device_without_consent_installs_nothing_and_says_why() {
    let shared = device_runtime(None, None);
    let kit = kit(&shared);
    assert_eq!(
        kit.unavailable,
        Some(crate::model::KitUnavailable::NoConsent),
        "{kit:?}"
    );
    dispatch(
        &shared,
        "kit_reinstall",
        serde_json::json!({ "device_id": DEVICE }),
    );
    let runtime = shared.lock().unwrap();
    assert!(!runtime.device_kit_pending.contains_key(DEVICE));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("kit.unavailable")
    );
}

/// B17: a device whose platform this build does not carry shows why and
/// installs nothing.
#[test]
fn a_device_on_an_unsupported_platform_shows_why() {
    let shared = with_consent(None);
    {
        let mut runtime = shared.lock().unwrap();
        runtime.device_hosts.insert(
            DEVICE.to_owned(),
            hosts::DeviceHost {
                phase: hosts::HostPhase::Unsupported(
                    "This Hide build does not include the device helper for Linux x86_64"
                        .to_owned(),
                ),
                generation: 1,
            },
        );
        runtime.refresh_device_snapshots();
    }
    assert_eq!(
        kit(&shared).unavailable,
        Some(crate::model::KitUnavailable::UnsupportedPlatform)
    );
}

/// D-13, B19: a device allowed under contract 2 for the same folders is
/// carried to the whole kit without asking when its connection starts, and
/// keeps the identity it was bound to; one allowed for other folders asks
/// again.
#[test]
fn a_contract_2_consent_is_carried_to_the_kit_on_its_next_connection() {
    let probe = device_runtime(None, None);
    let identity = hide_node_link::device::HostIdentity {
        user: "me".to_owned(),
        hostname: "studio.local".to_owned(),
        port: 22,
        host_key_sha256: "key".to_owned(),
    };
    let mut consent = granted(&probe);
    consent.contract = 2;
    consent.identity = Some(identity.clone());
    let shared = device_runtime(Some(consent.clone()), None);
    assert_eq!(
        shared.lock().unwrap().host_snapshot(DEVICE).consent,
        "granted"
    );
    shared.lock().unwrap().start_device_host(DEVICE);
    let carried = shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_registrations
        .iter()
        .find(|registration| registration.id == DEVICE)
        .and_then(|registration| registration.host_consent.clone())
        .unwrap();
    assert_eq!(carried.contract, crate::remote::HOST_CONSENT_CONTRACT);
    assert_eq!(carried.identity, Some(identity));

    let mut elsewhere = consent;
    elsewhere.helper_root = "~/elsewhere".to_owned();
    let shared = device_runtime(Some(elsewhere), None);
    shared.lock().unwrap().start_device_host(DEVICE);
    let runtime = shared.lock().unwrap();
    assert_eq!(runtime.host_snapshot(DEVICE).consent, "outdated");
    assert_eq!(
        runtime
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == DEVICE)
            .and_then(|registration| registration.host_consent.as_ref())
            .map(|consent| consent.contract),
        Some(2)
    );
}

/// PRD hide-home-layout D-12, B20: a device allowed for the old default
/// helper root is not asked again; the connection installs at the new
/// default root and keeps the bound identity. A consent for another root,
/// or a daemon told another root, carries nothing.
#[test]
fn a_consent_for_the_old_default_root_moves_to_the_new_one_without_asking() {
    use hide_kit::layout::{HELPER_ROOT, LEGACY_HELPER_ROOT};
    let probe = device_runtime(None, None);
    let identity = hide_node_link::device::HostIdentity {
        user: "me".to_owned(),
        hostname: "studio.local".to_owned(),
        port: 22,
        host_key_sha256: "key".to_owned(),
    };
    let mut consent = granted(&probe);
    assert_eq!(consent.helper_root, HELPER_ROOT);
    consent.helper_root = LEGACY_HELPER_ROOT.to_owned();
    consent.identity = Some(identity.clone());
    let shared = device_runtime(Some(consent.clone()), None);
    {
        let runtime = shared.lock().unwrap();
        let snapshot = runtime.host_snapshot(DEVICE);
        assert_eq!(snapshot.consent, "granted");
        assert_eq!(snapshot.helper_root.as_deref(), Some(HELPER_ROOT));
    }
    let connecting = shared
        .lock()
        .unwrap()
        .carried_consent(DEVICE, consent.clone());
    assert_eq!(
        connecting.helper_root, HELPER_ROOT,
        "this connection installs at the new root"
    );
    assert_eq!(connecting.identity, Some(identity));
    let saved = shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_registrations
        .iter()
        .find(|registration| registration.id == DEVICE)
        .and_then(|registration| registration.host_consent.clone())
        .unwrap();
    assert_eq!(saved.helper_root, HELPER_ROOT);

    let mut elsewhere = consent.clone();
    elsewhere.helper_root = "~/elsewhere".to_owned();
    let shared = device_runtime(Some(elsewhere.clone()), None);
    let kept = shared.lock().unwrap().carried_consent(DEVICE, elsewhere);
    assert_eq!(kept.helper_root, "~/elsewhere");
    assert_eq!(
        shared.lock().unwrap().host_snapshot(DEVICE).consent,
        "outdated"
    );

    // An isolated daemon told its own root asks again for the old default.
    let shared = device_runtime(Some(consent.clone()), None);
    shared.lock().unwrap().host_helper_root = "~/.cache/hide-test/helper".to_owned();
    assert_eq!(
        shared.lock().unwrap().host_snapshot(DEVICE).consent,
        "outdated"
    );
    let kept = shared.lock().unwrap().carried_consent(DEVICE, consent);
    assert_eq!(kept.helper_root, LEGACY_HELPER_ROOT);
}

/// A kit call that fails on a device never read says why on its row, and
/// the row stops saying it is working.
#[test]
fn a_failed_kit_call_on_an_unread_device_says_why() {
    let helper = KitDevice::answering(Err("The device did not answer in time".to_owned()));
    let shared = with_consent(Some(helper));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Apply(hide_kit::Scope::automatic()));
    settle(&shared);
    let kit = kit(&shared);
    assert!(!kit.busy);
    assert_eq!(
        kit.unavailable,
        Some(crate::model::KitUnavailable::Unreadable),
        "{kit:?}"
    );
}

/// B10: Check again shows a read under way until an answer lands, and the
/// core announces that flip so the shell pulls it before the answer; an answer
/// that is a failure leaves the report the device last gave and says only a
/// code, and the next read that lands clears it.
#[test]
fn check_again_shows_a_read_under_way_and_a_failed_one_as_a_code_beside_the_last_report() {
    let (helper, release) =
        KitDevice::held(Ok(report(&[(ComponentId::Cli, ComponentState::Installed)])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    let announced = dispatch(&shared, "kit_check", serde_json::json!({}));
    assert!(
        announced,
        "the shell pulls a snapshot only when the core announces one, so the flip must be announced"
    );
    let queued = kit(&shared);
    assert!(queued.checking, "the read is queued");
    assert_eq!(serde_json::to_value(&queued).unwrap()["checking"], true);
    wait_for("the device read to start", || !helper.calls().is_empty());
    assert!(kit(&shared).checking, "the read is still under way");
    release.send(()).unwrap();
    settle(&shared);
    let read = kit(&shared);
    assert!(!read.checking && read.check_failed.is_none(), "{read:?}");
    assert_eq!(read.components.len(), 1);

    *helper.answer.lock().unwrap() = Err("The device did not answer in time".to_owned());
    dispatch(&shared, "kit_check", serde_json::json!({}));
    settle(&shared);
    let failed = kit(&shared);
    assert!(!failed.checking, "{failed:?}");
    assert_eq!(
        failed.check_failed,
        Some(crate::model::KitCheckFailure::ReadFailed)
    );
    assert_eq!(failed.components, read.components, "the last report stays");
    assert!(
        failed.unavailable.is_none(),
        "a list exists, so no blanket reason"
    );
    let wire = serde_json::to_value(&failed).unwrap();
    assert_eq!(wire["check_failed"], "read_failed");
    assert!(
        !wire.to_string().contains("did not answer"),
        "words stay in the log"
    );

    *helper.answer.lock().unwrap() = Ok(report(&[(ComponentId::Cli, ComponentState::Installed)]));
    dispatch(&shared, "kit_check", serde_json::json!({}));
    settle(&shared);
    assert!(kit(&shared).check_failed.is_none());
}

fn registered(shared: &Mutex<Runtime>) -> bool {
    shared
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .device_registrations
        .iter()
        .any(|registration| registration.id == DEVICE)
}

/// B23: removing a connected device takes Hide's kit off it on its own
/// helper connection, which closes once that is done.
#[test]
fn removing_a_connected_device_takes_its_kit_off_and_then_closes_the_helper() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    assert!(!registered(&shared), "the registration goes at once");
    settle(&shared);

    assert_eq!(
        helper
            .calls()
            .into_iter()
            .map(|(action, ..)| action)
            .collect::<Vec<_>>(),
        vec![KitAction::Remove]
    );
    assert_eq!(helper.closed_reason().as_deref(), Some("device removed"));
    assert!(!shared.lock().unwrap().device_kit_removing(DEVICE));
}

/// Two registrations that reach one account on one machine (two Herdr
/// servers there) share its hooks, `hide` link and helper root, so removing
/// one leaves the kit for the other and only closes its own helper.
#[test]
fn removing_one_of_two_registrations_of_the_same_account_keeps_the_kit() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    {
        let mut runtime = shared.lock().unwrap();
        let registrations = &mut runtime.snapshot.ui_state.device_registrations;
        let device = registrations
            .iter_mut()
            .find(|registration| registration.id == DEVICE)
            .unwrap();
        device.host_consent.as_mut().unwrap().identity =
            Some(hide_node_link::device::HostIdentity {
                user: "me".to_owned(),
                hostname: "studio.local".to_owned(),
                port: 22,
                host_key_sha256: "SHA256:studio".to_owned(),
            });
        let mut twin = device.clone();
        twin.id = "studio-second-herdr".to_owned();
        twin.origin = crate::model::LinkOrigin::Dialed {
            ssh_alias: "studio-by-address".to_owned(),
        };
        twin.host_consent
            .as_mut()
            .unwrap()
            .identity
            .as_mut()
            .unwrap()
            .hostname = "10.0.0.7".to_owned();
        twin.label = "Studio, second Herdr".to_owned();
        registrations.push(twin);
        runtime.refresh_device_snapshots();
    }
    // The removal confirmation reads this to say the kit stays.
    assert_eq!(
        kit(&shared).shares_account_with.as_deref(),
        Some("Studio, second Herdr")
    );
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);

    assert!(!registered(&shared));
    assert!(helper.calls().is_empty(), "{:?}", helper.calls());
    assert_eq!(helper.closed_reason().as_deref(), Some("device removed"));
    assert!(!shared.lock().unwrap().device_kit_removing(DEVICE));
}

/// A device removed while its install call is still running on the device:
/// the removal runs after that call, and the call's late answer brings no
/// kit state back, so the same id added again starts unread.
#[test]
#[allow(clippy::disallowed_methods)] // a bounded poll inside the test: it sleeps between observations of a state, bounded by a deadline
fn an_install_answer_that_arrives_after_removal_brings_nothing_back() {
    let (helper, release) =
        KitDevice::held(Ok(report(&[(ComponentId::Cli, ComponentState::Installed)])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Apply(hide_kit::Scope::automatic()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while helper.calls().is_empty() {
        assert!(Instant::now() < deadline, "the install call never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    release.send(()).unwrap();
    settle(&shared);

    assert_eq!(
        helper
            .calls()
            .into_iter()
            .map(|(action, ..)| action)
            .collect::<Vec<_>>(),
        vec![KitAction::Apply, KitAction::Remove]
    );
    let runtime = shared.lock().unwrap();
    assert!(!runtime.device_kit_removing(DEVICE));
    assert_eq!(
        runtime.kit_state(DEVICE),
        crate::model::KitSnapshot::default()
    );
}

/// B24: a device removed while its helper is not connected loses only its
/// registration; nothing is sent, and adding it again later installs the
/// whole kit on its next connection.
#[test]
fn removing_a_device_whose_helper_is_not_connected_sends_nothing() {
    let shared = with_consent(None);
    dispatch(
        &shared,
        "remove_device",
        serde_json::json!({ "device_id": DEVICE }),
    );
    assert!(!registered(&shared));
    let runtime = shared.lock().unwrap();
    assert!(!runtime.device_kit_removing(DEVICE));
    assert!(!runtime.device_kit_running.contains(DEVICE));
}

/// A device added again while its kit is still coming off waits to connect,
/// so the removal cannot delete what the new connection installs.
#[test]
fn a_device_added_again_during_its_removal_waits_to_connect() {
    let shared = with_consent(None);
    let mut runtime = shared.lock().unwrap();
    runtime.device_kit_removing.insert(DEVICE.to_owned());
    runtime.start_device_host(DEVICE);
    let host = runtime.host_snapshot(DEVICE);
    assert_eq!(host.state, "unavailable");
    assert!(
        host.message
            .as_deref()
            .is_some_and(|message| message.contains("still taking its kit off")),
        "{host:?}"
    );
}

#[test]
fn retirement_inspection_takes_registered_checkouts_on_their_own_device() {
    let mut runtime = runtime();
    for (device, path) in [
        (crate::node::TEST_NODE, "/local-checkout"),
        (DEVICE, "/device-checkout"),
        ("other", "/other-checkout"),
    ] {
        runtime.snapshot.ui_state.workspace_registrations.push(
            crate::model::WorkspaceRegistration {
                id: format!("{device}-project"),
                device_id: device.into(),
                path: path.into(),
                ..Default::default()
            },
        );
    }
    runtime.snapshot.navigator.workspaces.push(workspace(
        "local-project",
        crate::node::TEST_NODE,
        "/local-checkout",
        vec![checkout(
            "local-project",
            "linked",
            "/local-linked-checkout",
            None,
        )],
    ));
    let mut unregistered = workspace(
        "incidental",
        "incidental",
        "/pane-only",
        vec![checkout("incidental", "pane", "/pane-only", None)],
    );
    unregistered.registered = false;
    runtime.snapshot.navigator.workspaces.push(unregistered);
    assert_eq!(
        runtime.retirement_projects(crate::node::TEST_NODE),
        ["/local-checkout", "/local-linked-checkout"]
    );
    assert_eq!(runtime.retirement_projects(DEVICE), ["/device-checkout"]);
    assert_eq!(
        runtime.retirement_projects("unregistered"),
        Vec::<String>::new()
    );
}

#[test]
fn device_kit_worker_sends_only_its_registered_checkout_paths() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(Arc::clone(&helper)));
    {
        let mut runtime = shared.lock().unwrap();
        for (device, path) in [
            (crate::node::TEST_NODE, "/local-checkout"),
            (DEVICE, "/device-checkout"),
            ("other", "/other-checkout"),
        ] {
            runtime.snapshot.ui_state.workspace_registrations.push(
                crate::model::WorkspaceRegistration {
                    id: format!("{device}-project"),
                    device_id: device.into(),
                    path: path.into(),
                    ..Default::default()
                },
            );
        }
        runtime.queue_device_kit(DEVICE, KitJob::Status);
    }
    settle(&shared);
    assert_eq!(
        *helper.retirement_projects.lock().unwrap(),
        [vec!["/device-checkout".to_owned()]]
    );
}

/// Reinstall repairs what is on: an out-of-date hook part of an agent that is
/// switched off is not offered one, because the pass would refuse it.
#[test]
fn reinstall_is_not_offered_for_the_hook_part_of_an_agent_that_is_off() {
    use hide_kit::Availability::Available;
    let snapshot = |enabled| {
        let mut answer = report(&[(ComponentId::ClaudeCodeHook, ComponentState::Outdated)]);
        answer.agents = vec![agent_report(
            "claude-code",
            Available,
            enabled,
            ComponentState::Installed,
        )];
        crate::model::KitSnapshot::from_report(&answer)
    };
    assert!(snapshot(true).offers_reinstall);
    assert!(!snapshot(false).offers_reinstall);
}

/// The latest intent wins (engineering rule 11): a switch pressed while an
/// earlier press is still queued replaces it, and the same press twice is one.
/// The device's worker is held in a read while the switches are pressed, so
/// what they queue waits behind it; a free worker took the queue before the
/// test read it (issue 689).
#[test]
fn the_latest_agent_switch_wins_over_one_still_queued() {
    use hide_kit::Availability::Available;
    let mut answer = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    answer.agents = vec![agent_report("omp", Available, false, ComponentState::Off)];
    let helper = KitDevice::answering(Ok(answer));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);
    let (release, gate) = std::sync::mpsc::channel();
    *helper.gate.lock().unwrap() = Some(gate);
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    wait_for("the held read to start", || helper.calls().len() == 2);

    let press = |enabled| {
        dispatch(
            &shared,
            "kit_agent_set",
            serde_json::json!({ "device_id": DEVICE, "agent": "omp", "enabled": enabled }),
        );
    };
    // The device reports omp off, so a lone "off" would be met; with "on"
    // queued first it is not, and the later "off" replaces the queued "on".
    press(true);
    press(true);
    press(false);
    {
        let runtime = shared.lock().unwrap();
        let Some(KitJob::Apply(scope)) = runtime.device_kit_pending.get(DEVICE) else {
            panic!("work stays queued");
        };
        assert!(scope.agent_off.contains("omp"), "{scope:?}");
        assert!(!scope.agent_on.contains("omp"), "{scope:?}");
    }
    release.send(()).unwrap();
    settle(&shared);
}

/// A first pass that could not run (the retirement preflight refused, the
/// account lock timed out) says nothing about the record, so it must not end
/// the choice: the next pass that runs is the one that decides.
#[test]
fn a_first_pass_that_did_not_run_does_not_decide_the_first_run_choice() {
    use crate::model::AgentOnboarding::Pending;
    let shared = with_consent(None);
    shared.lock().unwrap().ingest_kit_report(
        crate::node::TEST_NODE,
        &KitReport::refused(hide_kit::KitRefusal {
            cause: hide_kit::RefusalCause::Busy,
            path: None,
            variable: None,
            stop: None,
            detail: "another Hide was still changing this account's kit".into(),
        }),
    );
    assert_eq!(onboarding(&shared), None);

    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &held_report());
    assert_eq!(onboarding(&shared), Some(Pending));
}

/// An existing Mac is never asked, and a device added later gets what the Mac
/// has on today (Claude Code, Codex and agy by default), as it did before the
/// first-run choice existed.
#[test]
fn an_existing_macs_agents_become_the_saved_choice_a_later_device_receives() {
    use crate::model::AgentOnboarding::Done;
    use hide_kit::Availability::Available;
    let shared = with_consent(None);
    let mut existing = ran(report(&[(ComponentId::Cli, ComponentState::Installed)]));
    existing.agents = vec![
        agent_report("claude-code", Available, true, ComponentState::Installed),
        agent_report("codex", Available, true, ComponentState::Installed),
        agent_report("omp", Available, false, ComponentState::Off),
    ];
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(crate::node::TEST_NODE, &existing);
    assert_eq!(onboarding(&shared), Some(Done));
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot
            .ui_state
            .agent_onboarding_agents,
        ["claude-code", "codex"]
    );

    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &held_report());
    let runtime = shared.lock().unwrap();
    let Some(KitJob::Apply(scope)) = runtime.device_kit_pending.get(DEVICE) else {
        panic!("the device is given the Mac's agents");
    };
    assert_eq!(
        scope
            .agent_on
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["claude-code", "codex"]
    );
}

fn with_daemon(on: bool) -> KitReport {
    let mut answer = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    answer.codex_daemon = Some(true);
    answer.codex_daemon_on = Some(on);
    answer
}

/// B27: Turn off Codex's shared server on a device rides one `reinstall`
/// call to its helper, is pending until that call answers, and the answer
/// is what the pane's popover shows; a second request while it runs is the
/// same intent and a machine already off has nothing to turn off.
#[test]
fn turning_a_devices_codex_shared_server_off_is_one_helper_call_with_its_own_answer() {
    let helper = KitDevice::answering(Ok(with_daemon(true)));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);
    assert_eq!(kit(&shared).codex_daemon_on, Some(true));

    let mut done = with_daemon(false);
    done.codex_daemon_off = Some(hide_kit::CodexDaemonOff::Done { no_daemon: None });
    *helper.answer.lock().unwrap() = Ok(done);
    dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": DEVICE }),
    );
    assert_eq!(
        kit(&shared).codex_daemon_off,
        Some(crate::model::CodexDaemonOffSnapshot::Pending)
    );
    settle(&shared);

    let calls = helper.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(
        calls[1].0,
        KitAction::Reinstall {
            components: Vec::new(),
            agents_on: Vec::new(),
            agents_off: Vec::new(),
            codex_daemon_off: true,
        }
    );
    let after = kit(&shared);
    assert_eq!(
        after.codex_daemon_off,
        Some(crate::model::CodexDaemonOffSnapshot::Done)
    );
    assert_eq!(after.codex_daemon_on, Some(false));

    // Already off: nothing to turn off, so no second call.
    dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);
    assert_eq!(helper.calls().len(), 2);
}

#[test]
fn a_refused_or_unanswered_request_ends_as_a_code_and_a_later_read_does_not_erase_it() {
    let helper = KitDevice::answering(Ok(with_daemon(true)));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);

    let mut refused = with_daemon(true);
    refused.codex_daemon_off = Some(hide_kit::CodexDaemonOff::Failed {
        reason: hide_kit::CodexDaemonOffFailure::CodexRefused,
        detail: "config.toml is locked".to_owned(),
    });
    *helper.answer.lock().unwrap() = Ok(refused);
    dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);
    let failed = crate::model::CodexDaemonOffSnapshot::Failed {
        reason: hide_kit::CodexDaemonOffFailure::CodexRefused,
    };
    assert_eq!(kit(&shared).codex_daemon_off, Some(failed));
    assert_eq!(kit(&shared).codex_daemon_on, Some(true), "left as it was");

    // A plain read says nothing about the request, so it keeps the answer.
    *helper.answer.lock().unwrap() = Ok(with_daemon(true));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    settle(&shared);
    assert_eq!(kit(&shared).codex_daemon_off, Some(failed));

    // A call that never ran to a report is the machine not being reachable.
    *helper.answer.lock().unwrap() = Err("the helper went away".to_owned());
    dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": DEVICE }),
    );
    settle(&shared);
    assert_eq!(
        kit(&shared).codex_daemon_off,
        Some(crate::model::CodexDaemonOffSnapshot::Failed {
            reason: hide_kit::CodexDaemonOffFailure::Unreachable
        })
    );
}

/// Waits until the device helper has received `count` kit calls.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for_calls(helper: &KitDevice, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while helper.calls().len() < count {
        assert!(Instant::now() < deadline, "the helper never got the call");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// This machine's row: the kit of the core's own node, read from the
/// snapshot as the screen reads it.
fn own_kit(shared: &Mutex<Runtime>) -> crate::model::KitSnapshot {
    let runtime = shared.lock().unwrap();
    let node = runtime.node.clone();
    runtime
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == node.as_str())
        .expect("this machine's row")
        .kit
        .clone()
}

/// Waits until this machine's kit is no longer busy.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn settle_own(shared: &Mutex<Runtime>) -> crate::model::KitSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let kit = own_kit(shared);
        if !kit.busy {
            return kit;
        }
        assert!(
            Instant::now() < deadline,
            "this machine's kit worker never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A device's kit read with its Codex shared server on and, when given, the
/// answer to a turn-off.
fn shared_server(running: bool, off: Option<hide_kit::CodexDaemonOff>) -> KitReport {
    KitReport {
        codex_daemon: Some(true),
        codex_daemon_on: Some(!matches!(off, Some(hide_kit::CodexDaemonOff::Done { .. }))),
        codex_daemon_running: Some(running),
        codex_daemon_off: off,
        ..KitReport::default()
    }
}

/// How many of the helper's calls carried the shared-server turn-off.
fn turn_offs(helper: &KitDevice) -> usize {
    helper
        .calls()
        .iter()
        .filter(|call| {
            matches!(
                call.0,
                KitAction::Reinstall {
                    codex_daemon_off: true,
                    ..
                }
            )
        })
        .count()
}

fn records_of(records: &[serde_json::Value], kind: &str) -> usize {
    records
        .iter()
        .filter(|record| record["kind"] == kind)
        .count()
}

/// PRD codex-daemon-apply B11: the confirmation named what disconnects now,
/// so a device takes the turn-off, which stops its running daemon, only over
/// the helper connection it was confirmed on; a request while the helper is
/// away is refused into the log and changes nothing.
#[test]
fn a_turn_off_for_a_device_whose_helper_is_away_is_refused_into_the_log() {
    let shared = with_consent(None);
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &shared_server(true, None));
    assert!(kit(&shared).shares_codex_server());

    let ((), records) = crate::diagnostics::capture(|| {
        assert!(!dispatch(
            &shared,
            "codex_daemon_disable",
            serde_json::json!({ "device_id": DEVICE }),
        ));
    });
    assert_eq!(
        records_of(&records, "codex_daemon_off.refused"),
        1,
        "{records:?}"
    );
    assert_eq!(kit(&shared).codex_daemon_off, None);
    assert!(
        shared
            .lock()
            .unwrap()
            .snapshot()
            .status
            .last_error
            .is_none()
    );
    assert!(
        !shared
            .lock()
            .unwrap()
            .device_kit_pending
            .contains_key(DEVICE)
    );
}

/// B11: a connection that ends before the worker takes the turn-off drops
/// it: the request ends as unreachable, the log says so, and the next
/// connection never runs a stop nobody confirmed against it.
#[test]
fn a_turn_off_confirmed_on_one_connection_never_runs_on_the_next() {
    use crate::model::CodexDaemonOffSnapshot as Off;
    let (helper, release) = KitDevice::held(Ok(shared_server(true, None)));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &shared_server(true, None));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Status);
    // The turn-off waits in the queue behind the call the helper holds.
    wait_for_calls(&helper, 1);
    let ((), records) = crate::diagnostics::capture(|| {
        assert!(dispatch(
            &shared,
            "codex_daemon_disable",
            serde_json::json!({ "device_id": DEVICE }),
        ));
    });
    assert_eq!(records_of(&records, "codex_daemon_off.requested"), 1);
    assert_eq!(kit(&shared).codex_daemon_off, Some(Off::Pending));

    let ((), records) = crate::diagnostics::capture(|| {
        let mut runtime = shared.lock().unwrap();
        let generation = runtime.device_host_generation(DEVICE);
        runtime.ingest_host_closed(DEVICE, generation, "connection lost".to_owned());
    });
    assert_eq!(
        records_of(&records, "codex_daemon_off.dropped"),
        1,
        "{records:?}"
    );
    assert_eq!(
        kit(&shared).codex_daemon_off,
        Some(Off::Failed {
            reason: hide_kit::CodexDaemonOffFailure::Unreachable
        })
    );
    assert!(!matches!(
        shared.lock().unwrap().device_kit_pending.get(DEVICE),
        Some(KitJob::Apply(scope)) if scope.codex_daemon_off
    ));
    release.send(()).unwrap();
    settle(&shared);
    assert_eq!(turn_offs(&helper), 0, "the stop never reached the helper");
}

/// B11: a device's turn-off is its own helper call, carrying the request;
/// only the answer for the run still current settles it, so a late answer
/// from a run that already settled changes nothing.
#[test]
fn a_late_answer_for_a_turn_off_run_that_settled_changes_nothing() {
    use crate::model::CodexDaemonOffSnapshot as Off;
    let helper = KitDevice::answering(Ok(shared_server(
        false,
        Some(hide_kit::CodexDaemonOff::Done { no_daemon: None }),
    )));
    let shared = with_consent(Some(Arc::clone(&helper)));
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(DEVICE, &shared_server(true, None));
    assert!(dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": DEVICE }),
    ));
    settle(&shared);
    assert_eq!(turn_offs(&helper), 1);
    assert_eq!(kit(&shared).codex_daemon_off, Some(Off::Done));
    let run = shared.lock().unwrap().codex_daemon_off_runs;
    assert!(shared.lock().unwrap().codex_daemon_off_running.is_empty());

    // The same run answering again, late, as a failure or a report.
    shared.lock().unwrap().ingest_device_kit_answer(
        DEVICE,
        crate::runtime::DeviceKitAnswer::Report(Err("connection lost".to_owned())),
        Some(run),
    );
    shared.lock().unwrap().ingest_device_kit_answer(
        DEVICE,
        crate::runtime::DeviceKitAnswer::Report(Ok(shared_server(
            true,
            Some(hide_kit::CodexDaemonOff::Failed {
                reason: hide_kit::CodexDaemonOffFailure::StopFailed,
                detail: "late".to_owned(),
            }),
        ))),
        Some(run),
    );
    assert_eq!(kit(&shared).codex_daemon_off, Some(Off::Done));
}

/// PRD core-host-node D-21: this machine's kit runs on the core's own node,
/// asked with the command folder in its home and the embedder's Herdr
/// socket, and its row shows what the node answered.
#[test]
fn this_machines_kit_runs_on_its_own_node() {
    let shared = device_runtime(None, None);
    let node = KitDevice::answering(Ok(report(&[(ComponentId::Cli, ComponentState::Installed)])));
    shared.lock().unwrap().queue_local_kit_launch();
    let pump = crate::kit::KitPump::spawn(
        shared.weak(),
        crate::handle::ChangeNotifier::noop(),
        Arc::clone(&node) as Arc<dyn NodeLink>,
        Some("/run/herdr.sock".to_owned()),
        shared.lock().unwrap().node.clone(),
    )
    .unwrap();

    let kit = settle_own(&shared);
    drop(pump);
    assert_eq!(
        node.calls(),
        vec![(
            KitAction::Apply,
            "~/.local/bin".to_owned(),
            Some("/run/herdr.sock".to_owned())
        )]
    );
    assert_eq!(kit.unavailable, None);
    assert!(
        kit.components
            .iter()
            .any(|part| part.id == ComponentId::Cli && part.state == ComponentState::Installed),
        "{kit:?}"
    );
}

/// The node protocol is held until the test supplies each reply. No host
/// filesystem, provider or operator hook is reached by these lifecycle tests.
type HeldKitCall = (
    Call,
    std::sync::mpsc::Sender<Result<serde_json::Value, String>>,
);

struct HeldKitHost(std::sync::mpsc::Sender<HeldKitCall>);

impl NodeLink for HeldKitHost {
    fn call(&self, call: Call, timeout: Duration) -> Result<LinkAnswer, LinkError> {
        let (reply, answer) = std::sync::mpsc::channel();
        self.0
            .send((call, reply))
            .map_err(|_| LinkError::NotConnected("fixture ended".into()))?;
        answer
            .recv_timeout(timeout.min(Duration::from_secs(10)))
            .map_err(|error| LinkError::Unknown(error.to_string()))?
            .map(LinkAnswer::Parsed)
            .map_err(LinkError::Unknown)
    }
}

fn diagnosis_status(status: hide_agent_hooks::HookStatus) -> hide_agent_hooks::Diagnosis {
    super::agent_connection::diagnosis(status, hide_agent_hooks::HookStatus::NotInstalled)
}

#[test]
fn local_hook_diagnosis_and_merged_installs_have_one_ordered_owner() {
    let shared = device_runtime(None, None);
    let (requests, calls) = std::sync::mpsc::channel();
    let pump = crate::kit::KitPump::spawn(
        shared.weak(),
        crate::handle::ChangeNotifier::noop(),
        Arc::new(HeldKitHost(requests)),
        None,
        shared.lock().unwrap().node.clone(),
    )
    .unwrap();
    let (call, reply) = calls.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(call, Call::HookDiagnosis));
    for _ in 0..50 {
        shared.lock().unwrap().queue_local_kit_launch();
    }
    let before = diagnosis_status(hide_agent_hooks::HookStatus::NotInstalled);
    reply
        .send(Ok(serde_json::to_value(before).unwrap()))
        .unwrap();
    let (call, reply) = calls.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(
        call,
        Call::Kit {
            action: KitAction::Apply,
            ..
        }
    ));
    reply
        .send(Ok(serde_json::to_value(report(&[])).unwrap()))
        .unwrap();
    let (call, reply) = calls.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(call, Call::HookDiagnosis));
    let after = diagnosis_status(hide_agent_hooks::HookStatus::Off);
    reply
        .send(Ok(serde_json::to_value(&after).unwrap()))
        .unwrap();
    wait(&shared, "post-install hook diagnosis", |runtime| {
        runtime.hook_diagnosis.as_ref() == Some(&after)
    });
    drop(pump);
    assert!(
        calls.try_recv().is_err(),
        "no duplicate install or off-screen refresh remains"
    );
    assert_eq!(shared.lock().unwrap().hook_diagnosis.as_ref(), Some(&after));
}

#[test]
fn stopping_local_kit_joins_the_held_diagnosis_without_publishing_or_starting_pending_work() {
    let shared = device_runtime(None, None);
    let (requests, calls) = std::sync::mpsc::channel();
    let pump = crate::kit::KitPump::spawn(
        shared.weak(),
        crate::handle::ChangeNotifier::noop(),
        Arc::new(HeldKitHost(requests)),
        None,
        shared.lock().unwrap().node.clone(),
    )
    .unwrap();
    let (call, reply) = calls.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(call, Call::HookDiagnosis));
    shared.lock().unwrap().queue_local_kit_launch();
    pump.stop();
    let dropping = std::thread::spawn(move || drop(pump));
    // Shutdown must not hold Runtime while the host is still answering.
    assert!(shared.try_lock().is_ok());
    reply
        .send(Ok(serde_json::to_value(diagnosis_status(
            hide_agent_hooks::HookStatus::Off,
        ))
        .unwrap()))
        .unwrap();
    dropping.join().unwrap();
    assert!(
        shared.lock().unwrap().hook_diagnosis.is_none(),
        "a retiring owner's reply is stale"
    );
    assert!(
        calls.try_recv().is_err(),
        "stopping cancels the pending install"
    );
}

/// PRD codex-daemon-apply B11 on this machine: the turn-off needs no helper
/// connection here and goes to the core's own node as the kit's reinstall
/// with `codex_daemon_off`, whose answer settles this machine's row.
#[test]
fn this_machines_shared_server_turn_off_goes_to_its_own_node() {
    let shared = device_runtime(None, None);
    let own = shared.lock().unwrap().node.clone();
    shared
        .lock()
        .unwrap()
        .ingest_kit_report(own.as_str(), &shared_server(true, None));
    assert!(dispatch(
        &shared,
        "codex_daemon_disable",
        serde_json::json!({ "device_id": own.as_str() }),
    ));
    let node = KitDevice::answering(Ok(shared_server(
        false,
        Some(hide_kit::CodexDaemonOff::Done { no_daemon: None }),
    )));
    let pump = crate::kit::KitPump::spawn(
        shared.weak(),
        crate::handle::ChangeNotifier::noop(),
        Arc::clone(&node) as Arc<dyn NodeLink>,
        Some("/run/herdr.sock".to_owned()),
        own.clone(),
    )
    .unwrap();

    let kit = settle_own(&shared);
    drop(pump);
    assert_eq!(turn_offs(&node), 1, "{:?}", node.calls());
    assert_eq!(
        kit.codex_daemon_off,
        Some(crate::model::CodexDaemonOffSnapshot::Done)
    );
}

/// B11: a node outside the desktop package installs nothing, and this
/// machine's row says why with the node's own reason.
#[test]
fn a_standalone_own_node_installs_nothing_and_says_why() {
    let shared = device_runtime(None, None);
    let home = tempfile::tempdir().unwrap();
    shared.lock().unwrap().queue_local_kit_launch();
    let pump = crate::kit::KitPump::spawn(
        shared.weak(),
        crate::handle::ChangeNotifier::noop(),
        Arc::new(hide_node::Local::new(Some(home.path().to_path_buf()))),
        None,
        shared.lock().unwrap().node.clone(),
    )
    .unwrap();

    let kit = settle_own(&shared);
    drop(pump);
    assert_eq!(
        kit.unavailable,
        Some(crate::model::KitUnavailable::Standalone)
    );
    assert_eq!(
        std::fs::read_dir(home.path()).unwrap().count(),
        0,
        "nothing is written into the node's home"
    );
}

/// Letter-720: a device link that was lost or could not start is tried
/// again on its own after two seconds, the wait doubling to a minute; a read
/// before then starts nothing, and the operator's Retry tries at once and
/// begins the schedule over.
#[test]
fn a_failed_device_link_is_tried_again_after_a_doubling_wait() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    // Held throughout, so an attempt's own answer never lands; each failure
    // is ingested here instead.
    let mut runtime = shared.lock().unwrap();
    runtime.start_device_host(DEVICE);
    let mut waits = Vec::new();
    for _ in 0..7 {
        let generation = runtime.device_host_generation(DEVICE);
        let before = super::unix_milliseconds();
        runtime.ingest_host_established(
            DEVICE,
            generation,
            Err(hide_node_link::device::EstablishError::Helper(
                "the device did not answer".to_owned(),
            )),
        );
        let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
        waits.push((at - before) / 1_000);
        // A read before the wait is over starts nothing.
        assert!(runtime.node_link(DEVICE).is_err());
        assert_eq!(runtime.device_host_generation(DEVICE), generation);
        assert!(!runtime.tick_device_hosts(at - 1));
        assert_eq!(runtime.device_host_generation(DEVICE), generation);
        runtime.tick_device_hosts(at);
        assert_eq!(runtime.device_host_generation(DEVICE), generation + 1);
    }
    assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60]);

    let generation = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Err(hide_node_link::device::EstablishError::Helper(
            "again".to_owned(),
        )),
    );
    runtime.retry_device_host_now(DEVICE);
    assert_eq!(runtime.device_host_generation(DEVICE), generation + 1);
    let generation = runtime.device_host_generation(DEVICE);
    let before = super::unix_milliseconds();
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Err(hide_node_link::device::EstablishError::Helper(
            "again".to_owned(),
        )),
    );
    let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
    assert_eq!((at - before) / 1_000, 2);
}

/// PRD core-host-node-terminal D-18, D-19, B17: a device's terminals ride its
/// node link. A device without consent opens no link, so none are installed
/// and its panes have no terminal; a link installs them, and its end removes
/// them and leaves the device's panes unavailable until a link is back.
#[test]
fn a_devices_terminals_ride_its_link_and_end_with_it() {
    let shared = device_runtime(None, None);
    {
        let mut runtime = shared.lock().unwrap();
        let terminals = record_terminals(&mut runtime);
        runtime.start_device_host(DEVICE);
        assert!(matches!(
            runtime.device_hosts[DEVICE].phase,
            hosts::HostPhase::NotAllowed
        ));
        assert!(terminals.devices().is_empty());
    }

    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    let terminals = record_terminals(&mut runtime);
    runtime.start_device_host(DEVICE);
    let generation = runtime.device_host_generation(DEVICE);
    let established = established_with_terminals();
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Ok(hosts::HostReady::Dialed(established)),
    );
    assert_eq!(terminals.devices(), ["install:studio"]);

    let pane = "remote:studio:pane:w1:p1";
    runtime.ensure_terminal_pane(pane);
    report_terminal(&mut runtime, pane, terminal_state("controlling", 1));
    runtime.ingest_host_closed(DEVICE, generation, "the link dropped".to_owned());
    assert_eq!(terminals.devices(), ["install:studio", "remove:studio"]);
    let projected = runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .find(|row| row.pane_id == pane)
        .expect("the device's pane stays projected")
        .clone();
    assert_eq!(projected.transport_state, "unavailable");
    assert_eq!(
        projected.transport_exit_category.as_deref(),
        Some("device_unavailable")
    );
}

/// A device's connection whose node runs a terminal service.
fn established_with_terminals() -> hide_node_link::device::Established {
    established(
        KitDevice::answering(Err("not asked".to_owned())),
        Some(Arc::new(RecordedTerminals::default())),
    )
}

/// A device's connection over `host`, with `terminals` when its node runs a
/// terminal service.
pub(super) fn established(
    host: Arc<dyn crate::node_access::NodeLink>,
    terminals: Option<Arc<dyn hide_node_link::terminal::TerminalNode>>,
) -> hide_node_link::device::Established {
    hide_node_link::device::Established {
        node: node_ready(host, terminals),
        identity: hide_node_link::device::HostIdentity {
            user: "me".to_owned(),
            hostname: "studio.local".to_owned(),
            port: 22,
            host_key_sha256: "key".to_owned(),
        },
        installed: false,
        helper_path: "/home/me/.local/share/hide/host-helper/0123456789abcdef/hided".to_owned(),
        upload: Default::default(),
    }
}

/// A device's node answering over `host`, with `terminals` when it runs a
/// terminal service.
pub(super) fn node_ready(
    host: Arc<dyn crate::node_access::NodeLink>,
    terminals: Option<Arc<dyn hide_node_link::terminal::TerminalNode>>,
) -> hide_node_link::device::NodeReady {
    hide_node_link::device::NodeReady {
        host,
        hello: hide_node_link::protocol::Hello {
            protocol: hide_node_link::protocol::PROTOCOL_VERSION,
            version: "test".to_owned(),
            os: "macos".to_owned(),
            arch: "aarch64".to_owned(),
            home: None,
            machine_identity: hide_node_link::protocol::MachineIdentity::Unavailable {
                reason: "test".to_owned(),
            },
            reader_features: None,
        },
        terminals: terminals.ok_or_else(|| "the test's node runs no terminal service".to_owned()),
    }
}

const PANE: &str = "remote:studio:pane:w1:p1";

/// The device's Herdr session showing one pane in front, as its session sync
/// would publish it.
fn session_with_pane() -> RemoteSessionSnapshot {
    let pane_id = PANE;
    let workspace_id = "remote:studio:workspace:w1";
    let checkout_id = "remote:studio:checkout:w1";
    let tab_id = "remote:studio:tab:w1:t1";
    let mut device_checkout = checkout(
        workspace_id,
        checkout_id,
        "/tmp/hide-studio-shown",
        Some(pane(pane_id, "/tmp/hide-studio-shown")),
    );
    device_checkout.tabs[0].id = Some(tab_id.to_owned());
    let mut device_workspace = workspace(
        workspace_id,
        "Studio",
        "/tmp/hide-studio-shown",
        vec![device_checkout],
    );
    device_workspace.remote_target_id = Some(DEVICE.to_owned());
    device_workspace.device_id = DEVICE.to_owned();
    RemoteSessionSnapshot {
        workspaces: vec![device_workspace],
        agents: Vec::new(),
        active_tab_ids: [(checkout_id.to_owned(), tab_id.to_owned())]
            .into_iter()
            .collect(),
        focused_workspace_id: Some(workspace_id.to_owned()),
        focused_checkout_id: Some(checkout_id.to_owned()),
        focused_tab_id: Some(tab_id.to_owned()),
        focused_pane_id: Some(pane_id.to_owned()),
        pane_layouts: Vec::new(),
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
        herdr_version: None,
    }
}

/// D-19, B17: the device's pane on screen attaches again inside the link
/// that comes back after one dropped, and its keys go nowhere meanwhile.
#[test]
fn a_devices_shown_pane_attaches_again_when_its_link_returns() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    let terminals = record_terminals(&mut runtime);
    let pane_id = PANE;
    let session = session_with_pane();
    runtime.snapshot.navigator.focused_device_id = Some(DEVICE.to_owned());
    let attaches = |terminals: &RecordedTerminals| {
        terminals
            .take()
            .into_iter()
            .filter(|control| {
                matches!(control, TerminalControl::Attach { pane, .. } if pane == pane_id)
            })
            .count()
    };

    runtime.start_device_host(DEVICE);
    let first = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        first,
        Ok(hosts::HostReady::Dialed(established_with_terminals())),
    );
    runtime.ingest_remote_session(DEVICE, Ok(session));
    assert_eq!(attaches(&terminals), 1, "the shown pane attached");
    report_terminal(&mut runtime, pane_id, terminal_state("controlling", 1));

    runtime.ingest_host_closed(DEVICE, first, "the link dropped".to_owned());
    assert_eq!(attaches(&terminals), 0, "nothing attaches without a link");
    assert_eq!(
        terminals.devices(),
        ["install:studio", "remove:studio"],
        "the dropped link took its terminals with it"
    );

    runtime.start_device_host(DEVICE);
    let second = runtime.device_host_generation(DEVICE);
    assert_ne!(first, second);
    runtime.ingest_host_established(
        DEVICE,
        second,
        Ok(hosts::HostReady::Dialed(established_with_terminals())),
    );
    assert_eq!(
        terminals.devices(),
        ["install:studio", "remove:studio", "install:studio"]
    );
    assert_eq!(attaches(&terminals), 1, "the shown pane attached again");
}

fn herdr_behind(shared: &Mutex<Runtime>) -> Option<crate::model::HerdrBehindSnapshot> {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .navigator
        .devices
        .iter()
        .find(|device| device.id == DEVICE)
        .expect("device row")
        .herdr_behind
        .clone()
}

fn with_herdr(update: Option<hide_kit::device_herdr::HerdrUpdate>) -> KitReport {
    let mut report = report(&[(ComponentId::Cli, ComponentState::Installed)]);
    report.herdr = update.map(Box::new);
    report
}

/// PRD device-herdr-update B8, B3: a device server its node left on an
/// older Herdr puts one line on the device row, with why, until a later
/// pass finds it on the pin; a read, or a pass that could not tell, keeps it.
#[test]
fn a_device_server_left_older_than_the_pin_keeps_its_line_until_a_pass_finds_it_moved() {
    use hide_kit::device_herdr::{BehindCause, HerdrUpdate};
    let behind = HerdrUpdate::Behind {
        server_version: "0.9.1".to_owned(),
        version: "0.9.3".to_owned(),
        cause: BehindCause::CannotHandOff,
        reason: "Herdr 0.9.1 cannot hand its panes over".to_owned(),
    };
    let helper = KitDevice::answering(Ok(with_herdr(Some(behind))));
    let shared = with_consent(Some(Arc::clone(&helper)));
    let pass = |job: KitJob| {
        shared.lock().unwrap().queue_device_kit(DEVICE, job);
        settle(&shared);
    };

    pass(KitJob::Apply(hide_kit::Scope::automatic()));
    let line = crate::model::HerdrBehindSnapshot {
        server_version: "0.9.1".to_owned(),
        version: "0.9.3".to_owned(),
        cause: BehindCause::CannotHandOff,
    };
    assert_eq!(herdr_behind(&shared), Some(line.clone()));

    *helper.answer.lock().unwrap() = Ok(with_herdr(None));
    pass(KitJob::Status);
    assert_eq!(herdr_behind(&shared), Some(line.clone()), "a read keeps it");
    *helper.answer.lock().unwrap() = Ok(with_herdr(Some(HerdrUpdate::Failed {
        stage: "ping".to_owned(),
        reason: "timed out".to_owned(),
        version: Some("0.9.3".to_owned()),
        server_version: None,
    })));
    pass(KitJob::Apply(hide_kit::Scope::automatic()));
    assert_eq!(
        herdr_behind(&shared),
        Some(line),
        "an unreadable server keeps it"
    );

    *helper.answer.lock().unwrap() = Ok(with_herdr(Some(HerdrUpdate::HandedOff {
        server_version: "0.9.1".to_owned(),
        version: "0.9.3".to_owned(),
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
    })));
    pass(KitJob::Apply(hide_kit::Scope::automatic()));
    assert_eq!(herdr_behind(&shared), None);
}

fn live_session(herdr_version: &str) -> RemoteSessionSnapshot {
    RemoteSessionSnapshot {
        workspaces: Vec::new(),
        agents: Vec::new(),
        active_tab_ids: Default::default(),
        focused_workspace_id: None,
        focused_checkout_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        pane_layouts: Vec::new(),
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
        herdr_version: Some(herdr_version.to_owned()),
    }
}

/// PRD device-herdr-update B8: the line is about the server it was left on,
/// so a server the operator restarted on another release ends it as soon as
/// that server answers, without waiting for the next kit pass.
#[test]
fn a_device_servers_line_ends_once_the_live_server_answers_with_another_release() {
    use hide_kit::device_herdr::{BehindCause, HerdrUpdate};
    let helper = KitDevice::answering(Ok(with_herdr(Some(HerdrUpdate::Behind {
        server_version: "0.9.1".to_owned(),
        version: "0.9.3".to_owned(),
        cause: BehindCause::HandoffFailed,
        reason: "the handoff timed out".to_owned(),
    }))));
    let shared = with_consent(Some(helper));
    shared
        .lock()
        .unwrap()
        .queue_device_kit(DEVICE, KitJob::Apply(hide_kit::Scope::automatic()));
    settle(&shared);
    assert!(herdr_behind(&shared).is_some());

    shared
        .lock()
        .unwrap()
        .ingest_remote_session(DEVICE, Ok(live_session("0.9.1")));
    assert!(herdr_behind(&shared).is_some(), "the same server keeps it");

    shared
        .lock()
        .unwrap()
        .ingest_remote_session(DEVICE, Ok(live_session("0.9.3")));
    assert_eq!(herdr_behind(&shared), None);
}

/// PRD device-herdr-update B3, B11: the device's Herdr version is the one
/// its server names in the session, so a handoff shows at once, and a
/// device on an older protocol is told the device's step, never this
/// machine's advice to reopen Hide.
#[test]
fn a_device_reads_its_servers_own_version_and_device_advice_for_an_older_protocol() {
    let shared = with_consent(None);
    let mut runtime = shared.lock().unwrap();
    let remote = |runtime: &Runtime| {
        runtime
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == DEVICE)
            .cloned()
            .expect("the device's remote status")
    };
    runtime.ingest_remote_session(DEVICE, Ok(live_session("0.9.3")));
    assert_eq!(remote(&runtime).herdr_version.as_deref(), Some("0.9.3"));

    runtime.ingest_remote_session(
        DEVICE,
        Err(crate::wire::protocol_mismatch(21, Some("0.8.0".to_owned()))),
    );
    let status = remote(&runtime);
    assert_eq!(status.state, "protocol_mismatch");
    assert_eq!(status.herdr_version.as_deref(), Some("0.8.0"));
    let message = status.message.expect("the device row says why");
    assert!(message.contains("restart Herdr on the device"), "{message}");
    assert!(!message.contains("reopen Hide"), "{message}");
}

fn status_of(runtime: &Runtime) -> crate::model::RemoteStatusSnapshot {
    runtime
        .snapshot
        .status
        .remote
        .iter()
        .find(|status| status.target_id == DEVICE)
        .cloned()
        .expect("the device's remote status")
}

fn helper_failed() -> Result<hosts::HostReady, hide_node_link::device::EstablishError> {
    Err(hide_node_link::device::EstablishError::Helper(
        "the device did not answer".to_owned(),
    ))
}

/// A device's Herdr session that answers again means the machine can be
/// reached, so the helper link's retry is due now instead of at the end of a
/// wait grown while the machine was away; the wait after it keeps doubling,
/// and a session that flaps while the helper keeps failing moves the retry
/// forward once per outage, never once per flap.
#[test]
fn a_device_session_that_answers_again_makes_the_helper_retry_due_once_per_outage() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    runtime.start_device_host(DEVICE);
    for _ in 0..4 {
        let generation = runtime.device_host_generation(DEVICE);
        runtime.ingest_host_established(DEVICE, generation, helper_failed());
        let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
        runtime.tick_device_hosts(at);
    }
    let generation = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(DEVICE, generation, helper_failed());
    assert!(
        runtime.device_host_retries[DEVICE].at_unix_ms.unwrap()
            > super::unix_milliseconds() + 10_000,
        "the wait has grown past ten seconds"
    );

    let flap = |runtime: &mut Runtime| {
        runtime.ingest_remote_session(
            DEVICE,
            Err(SessionFetchError::Unreachable("no answer".to_owned())),
        );
        runtime.ingest_remote_session(DEVICE, Ok(live_session("0.9.3")));
    };
    flap(&mut runtime);
    let attempt = runtime.device_host_generation(DEVICE);
    assert_ne!(
        attempt, generation,
        "the machine answers again, so the helper link is tried at once"
    );

    // That attempt fails too; the wait after it is the schedule's next, not
    // the first one again.
    let before = super::unix_milliseconds();
    runtime.ingest_host_established(DEVICE, attempt, helper_failed());
    let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
    assert_eq!(
        (at - before) / 1_000,
        60,
        "the doubling goes on to its cap, not back to two seconds"
    );

    for _ in 0..5 {
        flap(&mut runtime);
    }
    assert_eq!(
        runtime.device_host_generation(DEVICE),
        attempt,
        "five more flaps start no attempt"
    );
    assert_eq!(runtime.device_host_retries[DEVICE].at_unix_ms, Some(at));
}

/// A pane of a device whose helper link will be tried again by itself reads
/// reconnecting, not unavailable; one that waits for the operator still reads
/// unavailable, and so does one whose link is up without terminals.
#[test]
fn a_device_pane_waiting_for_its_helper_reads_reconnecting_only_while_it_will_retry() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    let _terminals = record_terminals(&mut runtime);
    runtime.start_device_host(DEVICE);
    let first = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        first,
        Ok(hosts::HostReady::Dialed(established_with_terminals())),
    );
    runtime.ingest_remote_session(DEVICE, Ok(session_with_pane()));
    runtime.snapshot.navigator.focused_device_id = Some(DEVICE.to_owned());
    report_terminal(&mut runtime, PANE, terminal_state("controlling", 1));
    let notice = |runtime: &Runtime| {
        runtime.snapshot.terminal.headers[PANE]
            .notice
            .as_ref()
            .map(|notice| notice.kind.clone())
    };
    assert_eq!(notice(&runtime), None);

    runtime.ingest_host_closed(DEVICE, first, "the link dropped".to_owned());
    assert_eq!(notice(&runtime).as_deref(), Some("reconnecting"));

    // The retry finds no SSH connection to dial and schedules no other: the
    // pane waits for something that is not coming, so it is not reconnecting.
    let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
    runtime.tick_device_hosts(at);
    assert_eq!(notice(&runtime).as_deref(), Some("unavailable"));

    runtime.start_device_host(DEVICE);
    let second = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        second,
        Err(hide_node_link::device::EstablishError::Unsupported(
            "this build cannot serve the device".to_owned(),
        )),
    );
    assert_eq!(
        notice(&runtime).as_deref(),
        Some("unavailable"),
        "a device waiting for the operator is not reconnecting"
    );
}

/// A Reconnect pressed while the device still does not answer is a new
/// attempt, not a new device: the last state it reported stays on screen, the
/// age says when it first stopped answering, and a queued Reinstall waits for
/// the link (UI_BEHAVIOR, the Overview's unreachable body).
#[test]
fn reconnecting_a_device_that_is_still_down_keeps_its_last_state_age_and_queued_work() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    runtime.ingest_remote_session(DEVICE, Ok(session_with_pane()));
    runtime.ingest_remote_session(
        DEVICE,
        Err(SessionFetchError::Unreachable("no answer".to_owned())),
    );
    let since = 1_000;
    runtime
        .snapshot
        .status
        .remote
        .iter_mut()
        .find(|status| status.target_id == DEVICE)
        .unwrap()
        .unreachable_since_unix_ms = Some(since);
    runtime.queue_kit_reinstall(DEVICE, vec![ComponentId::Cli]);
    assert!(runtime.device_kit_pending.contains_key(DEVICE));

    assert!(runtime.retry_remote_device(DEVICE));

    let status = status_of(&runtime);
    assert!(status.session.is_some(), "the last state stays on screen");
    assert_ne!(status.state, "connected");
    assert_eq!(status.unreachable_since_unix_ms, Some(since));
    assert!(
        runtime.device_kit_pending.contains_key(DEVICE),
        "a queued Reinstall is not dropped"
    );
}

/// A helper link that ended while its connection was still being set up has
/// no one to hear it end once the connection is stored, so it is read as a
/// lost link then: tried again on its own, never stored ready and dead.
#[test]
fn a_helper_link_that_closed_during_establish_is_retried_not_stored_ready() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    runtime.start_device_host(DEVICE);
    let generation = runtime.device_host_generation(DEVICE);
    let host = KitDevice::answering(Err("not asked".to_owned()));
    host.close("the link dropped during hello");
    // The close was heard while the attempt still ran, which ignores it.
    assert!(!runtime.ingest_host_closed(DEVICE, generation, "the link dropped".to_owned()));
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Ok(hosts::HostReady::Dialed(established(host, None))),
    );
    assert!(
        matches!(
            runtime.device_hosts[DEVICE].phase,
            hosts::HostPhase::Unavailable(_)
        ),
        "a dead link is not stored ready"
    );
    assert!(
        runtime.device_host_retries[DEVICE].at_unix_ms.is_some(),
        "its retry is scheduled"
    );
}

/// The session answering again while a helper attempt still runs leaves that
/// attempt alone; should it fail, the retry is due at once, the once, and the
/// wait after that is the schedule's next.
#[test]
fn a_session_back_while_the_helper_link_starts_makes_its_failure_retry_at_once_once() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    runtime.start_device_host(DEVICE);
    for _ in 0..4 {
        let generation = runtime.device_host_generation(DEVICE);
        runtime.ingest_host_established(DEVICE, generation, helper_failed());
        let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
        runtime.tick_device_hosts(at);
    }
    runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Connecting;
    let generation = runtime.device_host_generation(DEVICE);

    runtime.ingest_remote_session(DEVICE, Ok(live_session("0.9.3")));
    assert_eq!(
        runtime.device_host_generation(DEVICE),
        generation,
        "the attempt that is running is left alone"
    );

    let before = super::unix_milliseconds();
    runtime.ingest_host_established(DEVICE, generation, helper_failed());
    let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
    assert!(at - before < 1_000, "its failure is retried at once");
    runtime.tick_device_hosts(at);

    // The next attempt fails after another flap: no second shortcut.
    runtime.device_hosts.get_mut(DEVICE).unwrap().phase = hosts::HostPhase::Connecting;
    let generation = runtime.device_host_generation(DEVICE);
    runtime.ingest_remote_session(
        DEVICE,
        Err(SessionFetchError::Unreachable("no answer".to_owned())),
    );
    runtime.ingest_remote_session(DEVICE, Ok(live_session("0.9.3")));
    let before = super::unix_milliseconds();
    runtime.ingest_host_established(DEVICE, generation, helper_failed());
    let at = runtime.device_host_retries[DEVICE].at_unix_ms.unwrap();
    assert_eq!((at - before) / 1_000, 60);
}

/// Revoking the helper ends the device's terminals with its link: a pane the
/// operator could type into reads unavailable at once, naming consent as the
/// reason, instead of looking live while every key is dropped.
#[test]
fn revoking_the_helper_ends_the_devices_terminals_and_its_panes_say_why() {
    let probe = device_runtime(None, None);
    let shared = device_runtime(Some(granted(&probe)), None);
    let mut runtime = shared.lock().unwrap();
    let terminals = record_terminals(&mut runtime);
    runtime.start_device_host(DEVICE);
    let generation = runtime.device_host_generation(DEVICE);
    runtime.ingest_host_established(
        DEVICE,
        generation,
        Ok(hosts::HostReady::Dialed(established_with_terminals())),
    );
    runtime.ingest_remote_session(DEVICE, Ok(session_with_pane()));
    report_terminal(&mut runtime, PANE, terminal_state("controlling", 1));
    let notice = |runtime: &Runtime| {
        runtime.snapshot.terminal.headers[PANE]
            .notice
            .as_ref()
            .map(|notice| notice.kind.clone())
    };
    assert_eq!(notice(&runtime), None);

    runtime.set_host_consent(DEVICE, false);

    assert_eq!(terminals.devices(), ["install:studio", "remove:studio"]);
    let pane = runtime
        .snapshot()
        .terminal
        .panes
        .iter()
        .find(|row| row.pane_id == PANE)
        .expect("the pane stays projected")
        .clone();
    assert_eq!(pane.transport_state, "unavailable");
    assert_eq!(notice(&runtime).as_deref(), Some("helper_not_allowed"));

    runtime.set_host_consent(DEVICE, true);
    assert_ne!(
        notice(&runtime).as_deref(),
        Some("helper_not_allowed"),
        "allowing it again ends the consent wait"
    );
}
