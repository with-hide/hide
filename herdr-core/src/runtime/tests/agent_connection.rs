//! How many Claude Code and Codex sessions run on each machine, and whether
//! Hide hears each one's pane (PRD settings-cleanup B16, B19, B26 to B28,
//! D-23). Expected answers come from the PRD, not from the projector.

use super::*;
use crate::model::{
    PaneConnectionReason, PaneConnectionSnapshot, PaneReopenFailure, PaneReopenSnapshot,
};
use crate::pane_reopen::ReopenFailure;
use hide_agent_hooks::{AgentRuntime, HookStatus};

/// One Herdr session with one pane per `(pane id, agent, hook token)`.
fn session(panes: &[(&str, &str, bool)]) -> serde_json::Value {
    let agents = panes
        .iter()
        .enumerate()
        .map(|(index, (pane, agent, _))| {
            serde_json::json!({
                "id": format!("agent-{pane}"), "pane_id": pane, "agent": agent,
                "agent_status": "idle", "state_change_seq": index + 1,
                "cwd": "/fixture", "workspace_label": "Fixture",
                "agent_session": {"kind": "id", "value": format!("session-{pane}")},
                "tokens": {"task": format!("Task of {pane}")},
            })
        })
        .collect::<Vec<_>>();
    let pane_rows = panes
        .iter()
        .map(|(pane, _, hooked)| {
            let tokens = if *hooked {
                serde_json::json!({"hide_hooks": hide_agent_hooks::HOOK_VERSION.to_string()})
            } else {
                serde_json::json!({})
            };
            serde_json::json!({"pane_id": pane, "cwd": "/fixture", "tokens": tokens})
        })
        .collect::<Vec<_>>();
    // One pane per tab: a tab of several panes needs an authoritative split.
    let tabs = (0..panes.len())
        .map(|index| serde_json::json!({"workspace_id": "w1", "tab_id": format!("t{index}"), "label": ""}))
        .collect::<Vec<_>>();
    let layouts = panes
        .iter()
        .enumerate()
        .map(|(index, (pane, _, _))| {
            serde_json::json!({
                "workspace_id": "w1", "tab_id": format!("t{index}"), "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": pane,
                "panes": [{"pane_id": pane, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
                "splits": [],
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "agents": agents,
        "panes": pane_rows,
        "tabs": tabs,
        "layouts": layouts,
    })
}

pub(super) fn diagnosis(claude: HookStatus, codex: HookStatus) -> hide_agent_hooks::Diagnosis {
    let row = |runtime: AgentRuntime, status| hide_agent_hooks::diagnosis::RuntimeDiagnosis {
        runtime,
        label: runtime.label().to_owned(),
        path: "/fixture/hooks".to_owned(),
        status,
        current_version: hide_agent_hooks::HOOK_VERSION,
        memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
            version: "999.0.0".to_owned(),
        },
    };
    hide_agent_hooks::Diagnosis {
        runtimes: vec![
            row(AgentRuntime::ClaudeCode, claude),
            row(AgentRuntime::Codex, codex),
        ],
        last_report_failure: None,
    }
}

pub(super) fn installed() -> HookStatus {
    HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }
}

/// This Mac's kit as the first check reported it: every adapter's row, on.
pub(super) fn kit_rows(runtime: &mut Runtime, codex_daemon_on: Option<bool>) {
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    kit.codex_daemon = Some(true);
    kit.codex_daemon_on = codex_daemon_on;
    kit.agents = hide_kit::agents::ADAPTERS
        .iter()
        .map(|adapter| {
            let piece = crate::model::KitPieceSnapshot {
                state: hide_kit::ComponentState::Installed,
                why: None,
                location: None,
            };
            crate::model::KitAgentSnapshot {
                id: adapter.id.to_owned(),
                label: adapter.label.to_owned(),
                availability: hide_kit::Availability::Available,
                enabled: true,
                chosen: false,
                skill: piece,
                hook: None,
                herdr: None,
                partial: adapter.partial(),
                features: Vec::new(),
                sessions: None,
                update: None,
                doc_url: String::new(),
            }
        })
        .collect();
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
}

pub(super) fn sessions_of(runtime: &Runtime, agent: &str) -> Option<u32> {
    runtime.snapshot.navigator.devices[0]
        .kit
        .agents
        .iter()
        .find(|row| row.id == agent)
        .unwrap_or_else(|| panic!("no kit row for {agent}"))
        .sessions
}

pub(super) fn connection_of(runtime: &Runtime, pane_id: &str) -> Option<PaneConnectionSnapshot> {
    runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .find(|pane| pane.id == pane_id)
        .and_then(|pane| pane.children.as_ref())
        .and_then(|children| children.connection.clone())
}

pub(super) fn feed(runtime: &mut Runtime, panes: &[(&str, &str, bool)]) {
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(session(panes)).expect("session payload")
    ));
}

#[test]
fn the_counts_follow_the_sessions_open_now_and_never_accumulate() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));

    feed(
        &mut runtime,
        &[
            ("w1:p1", "claude", true),
            ("w1:p2", "claude", false),
            ("w1:p3", "claude", false),
        ],
    );
    assert_eq!(
        sessions_of(&runtime, "claude-code"),
        Some(3),
        "every running session counts, heard or not"
    );

    // Two sessions close: the count shrinks with them (D-23).
    feed(
        &mut runtime,
        &[("w1:p1", "claude", true), ("w1:p2", "claude", false)],
    );
    assert_eq!(sessions_of(&runtime, "claude-code"), Some(2));
    feed(&mut runtime, &[("w1:p1", "claude", true)]);
    assert_eq!(sessions_of(&runtime, "claude-code"), Some(1));
    feed(&mut runtime, &[("w1:p4", "claude", true)]);
    assert_eq!(
        sessions_of(&runtime, "claude-code"),
        Some(1),
        "a different open session replaces the closed one"
    );
}

#[test]
fn a_retired_cursor_session_does_not_create_a_supported_agent_or_hook_connection() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(
        &mut runtime,
        &[("w1:p1", "cursor", false), ("w1:p2", "claude", true)],
    );

    assert!(
        runtime.snapshot.navigator.devices[0]
            .kit
            .agents
            .iter()
            .all(|agent| agent.id != "cursor")
    );
    assert_eq!(connection_of(&runtime, "w1:p1"), None);
    // An on agent with no session is "Ready": a count of zero, not an absent one.
    assert_eq!(sessions_of(&runtime, "codex"), Some(0));
}

#[test]
fn an_agent_without_a_native_session_waits_without_a_connection_warning() {
    for kind in ["codex", "claude"] {
        let mut runtime = runtime();
        runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
        kit_rows(&mut runtime, Some(true));
        let mut starting = session(&[("w1:p1", kind, false)]);
        starting["agents"][0]["agent_session"] = serde_json::Value::Null;
        runtime.ingest_session(Ok(
            crate::sidebar::owned_label_fixture(starting).expect("starting agent")
        ));
        assert_eq!(
            connection_of(&runtime, "w1:p1"),
            None,
            "detecting {kind}'s process before its session is not a failed integration"
        );

        feed(&mut runtime, &[("w1:p1", kind, false)]);
        assert!(connection_of(&runtime, "w1:p1").is_some());
        feed(&mut runtime, &[("w1:p1", kind, true)]);
        assert_eq!(
            connection_of(&runtime, "w1:p1").map(|connection| connection.connected),
            Some(true)
        );
    }
}

#[test]
fn a_codex_pane_on_the_shared_server_says_so_and_one_started_earlier_says_that() {
    let mut runtime = runtime();
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    kit_rows(&mut runtime, Some(true));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1"),
        Some(PaneConnectionSnapshot {
            connected: false,
            can_reopen: true,
            reason: Some(PaneConnectionReason::CodexSharedServer),
            reopen: None,
            update: None,
        })
    );

    // The same pane with the shared server off is a session that started
    // before Hide's hook.
    kit_rows(&mut runtime, Some(false));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1").and_then(|connection| connection.reason),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
}

#[test]
fn a_missing_hook_asks_for_setup_and_a_switched_off_agent_claims_nothing() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(HookStatus::NotInstalled, installed()));
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1").and_then(|connection| connection.reason),
        Some(PaneConnectionReason::SetupNeeded),
        "the fix is the agent's row in Settings, not Reopen"
    );

    runtime.ingest_hook_diagnosis(diagnosis(HookStatus::Off, installed()));
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1"),
        None,
        "B16: off has no state"
    );
}

fn with_live(runtime: &mut Runtime) {
    // A worker started against this socket finds no Herdr and no runtime to
    // answer, so each answer below is fed in the way a worker would.
    let socket = std::env::temp_dir()
        .join(format!("herdr-core-reopen-{}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned();
    runtime.live = Some(crate::live::LiveContext {
        socket_path: socket.clone().into(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket)),
        node: Arc::new(hide_node::Local::of_process()),
    });
}

fn reopen(runtime: &mut Runtime, pane_id: &str) -> bool {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "pane_reopen",
            "payload": {"pane_id": pane_id}
        }))
        .unwrap(),
    )
}

fn reopen_of(runtime: &Runtime, pane_id: &str) -> Option<PaneReopenSnapshot> {
    connection_of(runtime, pane_id).and_then(|connection| connection.reopen)
}

/// B29: Reopen is pending while it runs, a second press is the same intent,
/// a refusal leaves the pane as it was with a code, and a start that landed
/// publishes nothing more once the session's own hook connects.
#[test]
fn reopen_runs_once_per_pane_and_a_refusal_leaves_the_pane_as_it_was() {
    let mut runtime = runtime();
    with_live(&mut runtime);
    kit_rows(&mut runtime, Some(true));
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    assert_eq!(reopen_of(&runtime, "w1:p1"), None);

    assert!(reopen(&mut runtime, "w1:p1"));
    assert_eq!(
        reopen_of(&runtime, "w1:p1"),
        Some(PaneReopenSnapshot::Pending)
    );
    assert!(
        !reopen(&mut runtime, "w1:p1"),
        "a second press while one runs starts nothing"
    );

    assert!(runtime.ingest_pane_reopen(
        "w1:p1",
        Err(ReopenFailure {
            code: PaneReopenFailure::EndRefused,
            detail: "the agent did not hand the terminal back".into(),
            ended: false,
        }),
    ));
    assert_eq!(
        reopen_of(&runtime, "w1:p1"),
        Some(PaneReopenSnapshot::Failed {
            reason: PaneReopenFailure::EndRefused
        })
    );
    assert_eq!(
        connection_of(&runtime, "w1:p1").map(|connection| connection.connected),
        Some(false),
        "the pane stays as it was"
    );

    // Pressing again after a refusal is a new attempt; a start that landed
    // ends the attempt, and the pane's own hook ends the chip.
    assert!(reopen(&mut runtime, "w1:p1"));
    assert_eq!(
        reopen_of(&runtime, "w1:p1"),
        Some(PaneReopenSnapshot::Pending)
    );
    assert!(runtime.ingest_pane_reopen("w1:p1", Ok(())));
    assert_eq!(reopen_of(&runtime, "w1:p1"), None);
    feed(&mut runtime, &[("w1:p1", "codex", true)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1"),
        Some(PaneConnectionSnapshot {
            connected: true,
            can_reopen: false,
            reason: None,
            reopen: None,
            update: None,
        })
    );
    assert!(
        !reopen(&mut runtime, "w1:p1"),
        "connected: nothing to reopen"
    );
    assert_eq!(runtime.pane_reopens.len(), 0, "no entry outlives its need");
}

/// An agent that was ended and could not be started again has no row for the
/// pane's chip to hang the failure on, so the failure is published once as
/// the snapshot's error; a refusal that left the agent in place is not.
#[test]
fn a_reopen_that_ended_the_agent_and_could_not_start_it_is_still_published() {
    let mut runtime = runtime();
    with_live(&mut runtime);
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert!(reopen(&mut runtime, "w1:p1"));

    // Herdr ended the agent: the pane is still there with no agent row.
    let mut ended = session(&[("w1:p1", "claude", false)]);
    ended["agents"] = serde_json::json!([]);
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(ended).expect("session payload")
    ));
    assert_eq!(
        reopen_of(&runtime, "w1:p1"),
        None,
        "a pane without an agent has no chip to carry anything"
    );
    assert!(runtime.ingest_pane_reopen(
        "w1:p1",
        Err(ReopenFailure {
            code: PaneReopenFailure::StartRefused,
            detail: "agent_name_taken: name is used".into(),
            ended: true,
        }),
    ));

    let error = runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .expect("the failure stays published");
    assert_eq!(error.kind, "pane_reopen.not_restarted");
    assert!(
        !error.message.contains("agent_name_taken"),
        "Herdr's words stay in the log: {}",
        error.message
    );

    // Pressing Reopen on a pane that kept its agent and was refused before
    // anything was ended publishes the pane's chip, not this.
    let mut runtime = self::runtime();
    with_live(&mut runtime);
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert!(reopen(&mut runtime, "w1:p1"));
    assert!(runtime.ingest_pane_reopen(
        "w1:p1",
        Err(ReopenFailure {
            code: PaneReopenFailure::EndRefused,
            detail: "refused".into(),
            ended: false,
        }),
    ));
    assert!(runtime.snapshot.status.last_error.is_none());
}

/// A refusal known under the lock is published at once and starts no worker;
/// each one is a code, never Herdr's words.
#[test]
fn reopen_refuses_what_it_cannot_do_with_a_code_and_never_for_a_missing_hook() {
    let mut runtime = runtime();
    with_live(&mut runtime);
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));

    // An agent working now is not ended.
    let mut working = session(&[("w1:p1", "claude", false)]);
    working["agents"][0]["agent_status"] = "working".into();
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(working).expect("session payload")
    ));
    assert!(reopen(&mut runtime, "w1:p1"));
    assert_eq!(
        reopen_of(&runtime, "w1:p1"),
        Some(PaneReopenSnapshot::Failed {
            reason: PaneReopenFailure::AgentBusy
        })
    );

    // A Codex whose capability was never read does not know whether to leave
    // the shared server, so it starts nothing.
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    kit.codex_daemon = None;
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
    feed(&mut runtime, &[("w1:p2", "codex", false)]);
    assert!(reopen(&mut runtime, "w1:p2"));
    assert_eq!(
        reopen_of(&runtime, "w1:p2"),
        Some(PaneReopenSnapshot::Failed {
            reason: PaneReopenFailure::CodexUnread
        })
    );

    // Before Herdr reports a native session there is no warning or recovery
    // control; even a stale request must not start a reopen.
    let mut anonymous = session(&[("w1:p3", "claude", false)]);
    anonymous["agents"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_session");
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(anonymous).expect("session payload")
    ));
    assert!(!reopen(&mut runtime, "w1:p3"));
    assert_eq!(connection_of(&runtime, "w1:p3"), None);
    assert!(runtime.pane_reopens.is_empty());

    // A hook that is missing is Settings' to fix; Reopen would change nothing.
    runtime.ingest_hook_diagnosis(diagnosis(
        HookStatus::NotInstalled,
        HookStatus::NotInstalled,
    ));
    feed(&mut runtime, &[("w1:p4", "claude", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p4"),
        Some(PaneConnectionSnapshot {
            connected: false,
            can_reopen: false,
            reason: Some(PaneConnectionReason::SetupNeeded),
            reopen: None,
            update: None,
        })
    );
    reopen(&mut runtime, "w1:p4");
    assert_eq!(reopen_of(&runtime, "w1:p4"), None);
    assert!(runtime.pane_reopens.is_empty());
}

fn disable(runtime: &mut Runtime, device: &str) -> bool {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "codex_daemon_disable",
            "payload": {"device_id": device}
        }))
        .unwrap(),
    )
}

fn off_of(runtime: &Runtime) -> Option<crate::model::CodexDaemonOffSnapshot> {
    runtime.snapshot.navigator.devices[0].kit.codex_daemon_off
}

fn report_with(on: bool, off: Option<hide_kit::CodexDaemonOff>) -> hide_kit::KitReport {
    hide_kit::KitReport {
        codex_daemon: Some(true),
        codex_daemon_on: Some(on),
        codex_daemon_running: None,
        codex_daemon_unreadable: None,
        codex_capability_unread: None,
        codex_daemon_off: off,
        ..hide_kit::KitReport::default()
    }
}

/// B27: the link on a Codex pane's popover turns the shared server off on
/// this Mac through the kit worker. It is pending until the pass that
/// carried it answers, nothing is queued twice, a failure is a code and
/// leaves the setting, and a success re-judges the panes at once.
#[test]
fn the_shared_server_off_request_is_one_queued_pass_and_its_answer_is_a_code() {
    use crate::model::CodexDaemonOffSnapshot as Off;
    let mut runtime = runtime();
    kit_rows(&mut runtime, Some(true));
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    assert_eq!(
        connection_of(&runtime, "w1:p1").and_then(|connection| connection.reason),
        Some(PaneConnectionReason::CodexSharedServer)
    );

    assert!(disable(&mut runtime, crate::node::TEST_NODE));
    assert_eq!(off_of(&runtime), Some(Off::Pending));
    assert_eq!(
        runtime.local_kit_pending,
        Some(hide_kit::Scope::codex_daemon_off())
    );
    assert!(
        !disable(&mut runtime, crate::node::TEST_NODE),
        "the same intent runs once"
    );

    // A read that lands first says nothing about the request.
    runtime.ingest_kit_report(crate::node::TEST_NODE, &report_with(true, None));
    assert_eq!(off_of(&runtime), Some(Off::Pending));

    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &report_with(
            true,
            Some(hide_kit::CodexDaemonOff::Failed {
                reason: hide_kit::CodexDaemonOffFailure::TimedOut,
                detail: "codex features disable did not answer".to_owned(),
            }),
        ),
    );
    assert_eq!(
        off_of(&runtime),
        Some(Off::Failed {
            reason: hide_kit::CodexDaemonOffFailure::TimedOut
        })
    );
    assert_eq!(
        connection_of(&runtime, "w1:p1").and_then(|connection| connection.reason),
        Some(PaneConnectionReason::CodexSharedServer),
        "a failure leaves the pane as it was"
    );

    // Trying again is a new attempt; a success turns the pane's reason into
    // the one a session started before the hook has.
    assert!(disable(&mut runtime, crate::node::TEST_NODE));
    assert_eq!(off_of(&runtime), Some(Off::Pending));
    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &report_with(
            false,
            Some(hide_kit::CodexDaemonOff::Done { no_daemon: None }),
        ),
    );
    assert_eq!(off_of(&runtime), Some(Off::Done));
    assert_eq!(
        connection_of(&runtime, "w1:p1").and_then(|connection| connection.reason),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
    assert!(
        !disable(&mut runtime, crate::node::TEST_NODE),
        "already off: nothing to turn off"
    );
}

/// PRD codex-daemon-apply D-07, B7, B9: with autostart already off, a daemon
/// that still answers keeps a Codex pane on the shared server and keeps the
/// turn-off on offer; a stop that did not take effect is its own code and
/// can be asked again; once no daemon answers the pane reads as a session
/// started before the hook.
#[test]
fn a_daemon_still_answering_with_autostart_off_keeps_the_shared_server_and_its_turn_off() {
    use crate::model::CodexDaemonOffSnapshot as Off;
    let local = crate::node::TEST_NODE;
    let read = |running: Option<bool>, off: Option<hide_kit::CodexDaemonOff>| hide_kit::KitReport {
        codex_daemon_running: running,
        ..report_with(false, off)
    };
    let mut runtime = runtime();
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    kit_rows(&mut runtime, Some(false));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    let reason = |runtime: &Runtime| {
        connection_of(runtime, "w1:p1").and_then(|connection| connection.reason)
    };
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
    assert!(
        !disable(&mut runtime, crate::node::TEST_NODE),
        "nothing answers: nothing to turn off"
    );

    runtime.ingest_kit_report(local, &read(Some(true), None));
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::CodexSharedServer)
    );
    assert!(disable(&mut runtime, crate::node::TEST_NODE));
    assert_eq!(off_of(&runtime), Some(Off::Pending));

    runtime.ingest_kit_report(
        local,
        &read(
            Some(true),
            Some(hide_kit::CodexDaemonOff::Failed {
                reason: hide_kit::CodexDaemonOffFailure::StopFailed,
                detail: "the shared Codex daemon still answers".to_owned(),
            }),
        ),
    );
    assert_eq!(
        off_of(&runtime),
        Some(Off::Failed {
            reason: hide_kit::CodexDaemonOffFailure::StopFailed
        })
    );
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::CodexSharedServer)
    );
    assert!(
        disable(&mut runtime, crate::node::TEST_NODE),
        "a failed stop can be asked again"
    );

    runtime.ingest_kit_report(
        local,
        &read(
            Some(false),
            Some(hide_kit::CodexDaemonOff::Done { no_daemon: None }),
        ),
    );
    assert_eq!(off_of(&runtime), Some(Off::Done));
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
    assert!(!disable(&mut runtime, crate::node::TEST_NODE));
}

/// B7: a stop that failed because the daemon's answer could not be read
/// leaves nothing known about the daemon; the retry stays on offer until a
/// read says no daemon answers.
#[test]
fn a_stop_that_failed_on_an_unreadable_answer_keeps_the_retry_until_no_daemon_answers() {
    let local = crate::node::TEST_NODE;
    let read = |running: Option<bool>, off: Option<hide_kit::CodexDaemonOff>| hide_kit::KitReport {
        codex_daemon_running: running,
        ..report_with(false, off)
    };
    let mut runtime = runtime();
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    kit_rows(&mut runtime, Some(false));
    feed(&mut runtime, &[("w1:p1", "codex", false)]);
    let reason = |runtime: &Runtime| {
        connection_of(runtime, "w1:p1").and_then(|connection| connection.reason)
    };
    runtime.ingest_kit_report(local, &read(Some(true), None));
    assert!(disable(&mut runtime, crate::node::TEST_NODE));

    runtime.ingest_kit_report(
        local,
        &read(
            None,
            Some(hide_kit::CodexDaemonOff::Failed {
                reason: hide_kit::CodexDaemonOffFailure::StopFailed,
                detail: "codex app-server daemon version printed an answer Hide cannot read"
                    .to_owned(),
            }),
        ),
    );
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::CodexSharedServer)
    );
    runtime.ingest_kit_report(local, &read(None, None));
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::CodexSharedServer),
        "a later read that still cannot tell keeps the retry"
    );
    assert!(
        disable(&mut runtime, crate::node::TEST_NODE),
        "the retry is accepted"
    );

    runtime.ingest_kit_report(local, &read(Some(false), None));
    assert_eq!(
        reason(&runtime),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
}

/// An answer about the daemon the kit could not read reads as no answer and
/// goes to the log once, not on every read (B13).
#[test]
fn an_unreadable_daemon_answer_is_no_answer_and_is_logged_once() {
    let local = crate::node::TEST_NODE;
    let mut runtime = runtime();
    let unreadable = hide_kit::KitReport {
        codex_daemon_unreadable: Some("codex answered `daemon: ok?`".to_owned()),
        ..report_with(false, None)
    };
    let ((), records) = crate::diagnostics::capture(|| {
        for _ in 0..3 {
            runtime.ingest_kit_report(local, &unreadable);
        }
    });
    let logged: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "codex_daemon.unreadable")
        .collect();
    assert_eq!(logged.len(), 1, "{records:?}");
    assert_eq!(logged[0]["device_id"], local);
    assert!(!runtime.kit_state(local).shares_codex_server());
}

/// A Codex the kit could not read goes to the log once per cause, with the
/// machine it was on and the cause and exit code alone (engineering rule 9).
#[test]
fn an_unread_codex_is_logged_once_with_its_machine_cause_and_exit_code() {
    let local = crate::node::TEST_NODE;
    let mut runtime = runtime();
    let unread = hide_kit::KitReport {
        codex_daemon: None,
        codex_capability_unread: Some(hide_kit::CodexReadFailure::Exited { code: 134 }),
        ..report_with(false, None)
    };
    let ((), records) = crate::diagnostics::capture(|| {
        for _ in 0..3 {
            runtime.ingest_kit_report(local, &unread);
        }
    });
    let logged: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "codex_capability_unread")
        .collect();
    assert_eq!(logged.len(), 1, "{records:?}");
    assert_eq!(logged[0]["device_id"], local);
    assert_eq!(logged[0]["agent"], "codex");
    assert_eq!(
        logged[0]["failure"],
        serde_json::json!({"cause": "exited", "code": 134})
    );
}

#[test]
fn a_machine_with_no_kit_or_no_such_device_refuses_the_request_with_an_error() {
    let mut runtime = runtime();
    runtime.set_local_kit_unavailable(
        crate::model::KitUnavailable::Standalone,
        "standalone daemon",
    );
    assert!(disable(&mut runtime, crate::node::TEST_NODE));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("kit.unavailable")
    );
    assert!(disable(&mut runtime, "nowhere"));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("kit.unknown_machine")
    );
}

/// B29 end to end through the runtime and its worker: the agent's process
/// ends, Herdr still holds its name for two answers, and the reopen settles
/// as done, with nothing left published and the calls in the order that
/// never mutates before Herdr has confirmed.
#[test]
fn a_reopen_through_the_worker_waits_for_herdr_to_release_the_name() {
    use crate::fake_herdr::FakeHerdr;
    use hide_platform::process;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
    let mut child = process::OwnedChild::spawn(std::process::Command::new("/bin/sleep").arg("60"))
        .expect("sleep starts");
    let pid = child.id();
    let exited = Arc::new(AtomicBool::new(false));
    let gone_flag = Arc::clone(&exited);
    let starts = Arc::new(AtomicUsize::new(0));
    let attempts = Arc::clone(&starts);
    let herdr = FakeHerdr::start_with_errors("runtime-reopen-name", move |method, params| {
        let info = |shell: u32, foreground: u32| {
            json!({"type": "pane_process_info", "process_info": {
                "pane_id": "w1:p2", "shell_pid": shell, "foreground_process_group_id": foreground,
                "foreground_processes": [{"pid": shell, "name": "zsh"}]
            }})
        };
        match method {
            "agent.get" => Ok(json!({"type": "agent_info", "agent": {
                "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                "terminal_id": "term_1", "agent": "claude", "agent_status": "idle",
                "state_change_seq": 2, "focused": false, "interactive_ready": true, "revision": 0
            }})),
            "pane.process_info" => {
                let shell = std::process::id();
                let gone = !process::is_alive(pid) || gone_flag.load(Ordering::SeqCst);
                Ok(info(shell, if gone { shell } else { pid }))
            }
            // The agent quits on its own keys, so the node never ends it.
            "pane.send_keys" => {
                let _ = process::terminate(pid);
                Ok(json!({"type": "ok"}))
            }
            "agent.start" => {
                assert_eq!(params["args"], json!(["--resume", SESSION_ID]));
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err((
                        "agent_name_taken".into(),
                        "agent name agent-w1:p2 is already used".into(),
                    ))
                } else {
                    Ok(json!({"type": "agent_started", "argv": [], "agent": {
                        "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                        "terminal_id": "term_1", "agent_status": "idle", "focused": false, "revision": 1
                    }}))
                }
            }
            other => panic!("unexpected {other}"),
        }
    });
    let reaper = std::thread::spawn(move || {
        let _ = child.wait();
        exited.store(true, Ordering::SeqCst);
    });

    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    let mut payload = session(&[("w1:p1", "claude", true), ("w1:p2", "claude", false)]);
    payload["agents"][1]["agent_session"]["value"] = json!(SESSION_ID);
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(payload).expect("native session fixture")
    ));
    // The reopen starts in the pane's own folder, which must exist.
    for pane in runtime
        .snapshot
        .navigator
        .workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        pane.cwd = std::env::temp_dir().to_string_lossy().into_owned();
    }
    let shared = Arc::new(Mutex::new(runtime));
    {
        let mut runtime = shared.lock().unwrap();
        runtime.live = Some(crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: Arc::downgrade(&shared),
            notifier: crate::handle::ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        });
        assert!(reopen(&mut runtime, "w1:p2"));
        assert_eq!(
            reopen_of(&runtime, "w1:p2"),
            Some(PaneReopenSnapshot::Pending)
        );
    }
    wait(&shared, "the reopen to settle", |runtime| {
        runtime.pane_reopens.is_empty() || {
            matches!(
                reopen_of(runtime, "w1:p2"),
                Some(PaneReopenSnapshot::Failed { .. })
            )
        }
    });
    reaper.join().expect("reaped");
    assert_eq!(
        reopen_of(&shared.lock().unwrap(), "w1:p2"),
        None,
        "the name was released, so the session started and nothing is left to say"
    );
    assert_eq!(starts.load(Ordering::SeqCst), 3);
    let methods = herdr.methods();
    assert_eq!(
        methods[0], "agent.get",
        "the agent is read before it is touched"
    );
    let first_start = methods.iter().position(|m| m == "agent.start").unwrap();
    assert!(methods[..first_start].iter().all(|m| m != "agent.start"));
}

/// A reopen in the pane resumes the conversation with the model and effort
/// Hide started the agent with, which a bare resume would leave to the CLI's
/// configured default (`launch_choice.rs` holds the other restarts).
#[test]
fn a_reopen_in_its_pane_resumes_with_the_choice_the_agent_was_started_with() {
    use crate::fake_herdr::FakeHerdr;
    use hide_platform::process;
    use serde_json::json;

    const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
    let mut child = process::OwnedChild::spawn(std::process::Command::new("/bin/sleep").arg("60"))
        .expect("sleep starts");
    let pid = child.id();
    let (sent, starts) = std::sync::mpsc::channel();
    let herdr = FakeHerdr::start(
        "runtime-reopen-choice",
        move |method, params| match method {
            "agent.get" => json!({"type": "agent_info", "agent": {
                "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                "terminal_id": "term_1", "agent": "claude", "agent_status": "idle",
                "state_change_seq": 2, "focused": false, "interactive_ready": true, "revision": 0
            }}),
            "pane.process_info" => {
                let shell = std::process::id();
                let foreground = if process::is_alive(pid) { pid } else { shell };
                json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "w1:p2", "shell_pid": shell, "foreground_process_group_id": foreground,
                    "foreground_processes": [{"pid": shell, "name": "zsh"}]
                }})
            }
            "pane.send_keys" => {
                let _ = process::terminate(pid);
                json!({"type": "ok"})
            }
            "agent.start" => {
                sent.send(params["args"].clone()).unwrap();
                json!({"type": "agent_started", "argv": [], "agent": {
                    "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                    "terminal_id": "term_1", "agent_status": "idle", "focused": false, "revision": 1
                }})
            }
            other => panic!("unexpected {other}"),
        },
    );
    let reaper = std::thread::spawn(move || {
        let _ = child.wait();
    });

    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    let launch = hide_agent_adapter::LaunchChoice {
        model: Some("sonnet".into()),
        effort: Some("high".into()),
    };
    runtime.record_pane_launch("w1:p2", "claude", "agent-w1:p2", &launch, None);
    let mut payload = session(&[("w1:p1", "claude", true), ("w1:p2", "claude", false)]);
    payload["agents"][1]["agent_session"]["value"] = json!(SESSION_ID);
    payload["agents"][1]["name"] = json!("agent-w1:p2");
    runtime.ingest_session(Ok(
        crate::sidebar::owned_label_fixture(payload).expect("native session fixture")
    ));
    for pane in runtime
        .snapshot
        .navigator
        .workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        pane.cwd = std::env::temp_dir().to_string_lossy().into_owned();
    }
    let shared = SharedRuntime::new(runtime);
    {
        let mut runtime = shared.lock().unwrap();
        runtime.live = Some(crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: shared.weak(),
            notifier: crate::handle::ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        });
        assert!(reopen(&mut runtime, "w1:p2"));
    }
    assert_eq!(
        starts
            .recv_timeout(Duration::from_secs(10))
            .expect("the reopen started the agent"),
        json!([
            "--resume", SESSION_ID, "--model", "sonnet", "--effort", "high"
        ])
    );
    wait(&shared, "the reopen to settle", |runtime| {
        runtime.pane_reopens.is_empty()
    });
    reaper.join().expect("reaped");
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .snapshot()
            .ui_state
            .pane_launches
            .get("w1:p2")
            .map(|record| &record.choice),
        Some(&launch),
        "the same conversation keeps its choice for the next restart"
    );
}
