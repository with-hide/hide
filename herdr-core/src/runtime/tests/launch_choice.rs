//! Every restart Hide makes of an agent (a wake, a fork, a reopen) starts it
//! with the model and effort it was started with, from the pane's launch
//! record, and a record from a build that kept none restarts on the CLI's
//! defaults as before. The restarts run through their real workers against a
//! fake Herdr; the expected command lines are each CLI's own spelling
//! (`claude --model <m> --effort <e>`, `codex --model <m> -c
//! model_reasoning_effort=<e>`), measured against Claude Code 2.1.302 and
//! Codex 0.163.0, where a bare resume or fork fell back to the configured
//! default.

use super::*;
use crate::fake_herdr::FakeHerdr;
use crate::model::PaneLaunch;
use hide_agent_adapter::LaunchChoice;
use serde_json::json;

const TABS: [&str; 2] = ["w-order:t1", "w-order:t2"];
const AGENT: &str = "w-order:t2:p";
const SESSION: &str = "11111111-2222-3333-4444-555555555555";
/// The Herdr name Hide started the t2 agent under.
const NAME: &str = "reviewer";

fn choice(model: &str, effort: &str) -> LaunchChoice {
    LaunchChoice {
        model: Some(model.to_owned()),
        effort: Some(effort.to_owned()),
    }
}

/// Each kind's resume line and the words its launch choice adds after it.
fn resumed(kind: &str, launch: &[&str]) -> serde_json::Value {
    let resume: &[&str] = match kind {
        "codex" => &["resume", SESSION],
        _ => &["--resume", SESSION],
    };
    json!([resume, launch].concat())
}

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload
    }))
    .unwrap()
}

/// The two-tab session with t1 in front and, when `listed`, the idle `kind`
/// agent Hide started in t2 as [`NAME`], on [`SESSION`].
fn session(checkout: &str, kind: &str, listed: bool) -> SessionSnapshotPayload {
    hosting(
        checkout,
        listed.then_some((kind, Some(NAME), Some(SESSION))),
    )
}

/// The same session with t2 hosting `agent`: its kind, the name Herdr
/// reports it by (none for one started by hand) and its conversation.
fn hosting(
    checkout: &str,
    agent: Option<(&str, Option<&str>, Option<&str>)>,
) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(checkout, &TABS, &TABS, "w-order:t1");
    if let Some((kind, name, session)) = agent {
        let mut row = json!({
            "id": name.unwrap_or(AGENT), "pane_id": AGENT, "agent": kind,
            "agent_status": "idle", "state_change_seq": 4, "cwd": checkout,
            "tokens": {"task": "Review the parser"}
        });
        if let Some(name) = name {
            row["name"] = json!(name);
        }
        if let Some(session) = session {
            row["agent_session"] = json!({"kind": "id", "value": session});
        }
        let owned: SessionSnapshotPayload =
            crate::sidebar::owned_label_fixture(json!({"agents": [row]})).unwrap();
        payload.agents.extend(owned.agents);
    }
    payload
}

/// A live runtime on a real checkout folder whose t2 agent Hide started
/// with `launch` (none: an agent of an older build, or one started on the
/// CLI's defaults), on a machine whose Codex has no shared daemon.
fn started(kind: &str, launch: Option<&LaunchChoice>) -> (Runtime, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    let checkout = checkout_of(&folder);
    let (mut runtime, _) = live_tab_order_runtime(&checkout);
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    kit.codex_daemon = Some(false);
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
    if let Some(launch) = launch {
        // What the task start, the spawn executor and every restart do once
        // the agent is running in the pane.
        runtime.record_pane_launch(AGENT, kind, NAME, launch, None);
    }
    runtime.ingest_session(Ok(session(&checkout, kind, true)));
    (runtime, folder)
}

fn checkout_of(folder: &tempfile::TempDir) -> String {
    hide_platform::fs::identity::canonical(folder.path())
        .unwrap()
        .display()
        .to_string()
}

/// A fake Herdr that starts any agent in a pane whose shell holds the
/// terminal, and hands each `agent.start`'s arguments to the test.
fn starting_herdr(
    name: &str,
) -> (
    FakeHerdr,
    std::sync::mpsc::Receiver<(String, serde_json::Value)>,
) {
    let (sent, starts) = std::sync::mpsc::channel();
    let herdr = FakeHerdr::start_with_errors(name, move |method, params| match method {
        "pane.process_info" => Ok(json!({"type": "pane_process_info", "process_info": {
            "pane_id": params["pane_id"], "shell_pid": 42, "foreground_process_group_id": 42,
            "foreground_processes": [{"pid": 42, "name": "zsh"}]
        }})),
        "pane.split" => Ok(json!({"type": "pane_info", "pane": {
            "pane_id": "w-order:t2:fork", "terminal_id": "fork-terminal",
            "workspace_id": "w-order", "tab_id": "w-order:t2", "focused": false,
            "agent_status": "idle", "revision": 1
        }})),
        "agent.start" => {
            sent.send((
                params["pane_id"].as_str().unwrap().to_owned(),
                params["args"].clone(),
            ))
            .unwrap();
            Ok(json!({"type": "agent_started", "argv": [], "agent": {
                "pane_id": params["pane_id"], "terminal_id": "term", "workspace_id": "w-order",
                "tab_id": "w-order:t2", "focused": false, "agent_status": "idle", "revision": 2
            }}))
        }
        // What the fork's registration asks after its start is no part of
        // the start; refusing it leaves a working fork.
        other => Err(("unsupported".into(), format!("fixture answers no {other}"))),
    });
    (herdr, starts)
}

fn next_start(
    starts: &std::sync::mpsc::Receiver<(String, serde_json::Value)>,
) -> (String, serde_json::Value) {
    starts
        .recv_timeout(Duration::from_secs(10))
        .expect("the worker started the agent")
}

/// Puts the t2 agent to sleep and has Herdr forget it, as the end does.
fn sleep(runtime: &mut Runtime, folder: &tempfile::TempDir, kind: &str) {
    runtime.dispatch_json(&event("agent_sleep", json!({"pane_id": AGENT})));
    assert!(runtime.snapshot().status.last_error.is_none());
    assert!(runtime.ingest_agent_sleep_end(AGENT, Ok(4)));
    runtime.ingest_session(Ok(session(&checkout_of(folder), kind, false)));
}

fn wake(runtime: &mut Runtime, herdr: &FakeHerdr) {
    runtime.live.as_mut().unwrap().api_connector = Arc::new(herdr.connector());
    assert!(runtime.dispatch_json(&event("agent_wake", json!({"pane_id": AGENT}))));
}

#[test]
fn a_wake_resumes_the_agent_with_the_model_and_effort_it_was_started_with() {
    for (kind, launch, words) in [
        (
            "claude",
            choice("sonnet", "high"),
            ["--model", "sonnet", "--effort", "high"],
        ),
        (
            "codex",
            choice("gpt-6-luna", "low"),
            ["--model", "gpt-6-luna", "-c", "model_reasoning_effort=low"],
        ),
    ] {
        let (mut runtime, folder) = started(kind, Some(&launch));
        sleep(&mut runtime, &folder, kind);
        let (herdr, starts) = starting_herdr("launch-choice-wake");
        wake(&mut runtime, &herdr);
        assert_eq!(
            next_start(&starts),
            (AGENT.to_owned(), resumed(kind, &words)),
            "{kind}"
        );
        // The woken agent is the same conversation on the same choice, so
        // the next restart carries it again.
        assert_eq!(
            runtime.snapshot().ui_state.pane_launches.get(AGENT),
            Some(&PaneLaunch {
                kind: kind.to_owned(),
                name: NAME.to_owned(),
                choice: launch,
                session: Some(SESSION.to_owned()),
            }),
            "{kind}"
        );
    }
}

/// A spawn keeps its child's choice before the agent shows a session, which
/// Codex CLI reports only at its first turn (the spawn's identity wait), so
/// the record starts unbound; it binds to the session when that arrives, and
/// a wake then resumes that conversation on the choice.
#[test]
fn a_choice_kept_before_the_session_arrives_reaches_the_wake() {
    let launch = choice("gpt-6-luna", "low");
    let folder = tempfile::tempdir().unwrap();
    let checkout = checkout_of(&folder);
    let (mut runtime, _) = live_tab_order_runtime(&checkout);
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    kit.codex_daemon = Some(false);
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
    // What the spawn writes once its start is typed, before any session.
    runtime.record_pane_launch(AGENT, "codex", NAME, &launch, None);
    runtime.ingest_session(Ok(hosting(&checkout, Some(("codex", Some(NAME), None)))));
    assert_eq!(
        runtime.snapshot().ui_state.pane_launches[AGENT].session,
        None
    );
    // The agent's first turn reports its session.
    runtime.ingest_session(Ok(session(&checkout, "codex", true)));
    assert_eq!(
        runtime.snapshot().ui_state.pane_launches[AGENT]
            .session
            .as_deref(),
        Some(SESSION)
    );
    sleep(&mut runtime, &folder, "codex");
    let (herdr, starts) = starting_herdr("launch-choice-late-wake");
    wake(&mut runtime, &herdr);
    assert_eq!(
        next_start(&starts),
        (
            AGENT.to_owned(),
            resumed(
                "codex",
                &["--model", "gpt-6-luna", "-c", "model_reasoning_effort=low"]
            )
        )
    );
}

/// An agent of an older build left no launch record and a sleep record
/// without the field; it wakes on the resume alone, as before.
#[test]
fn a_wake_of_an_agent_with_no_recorded_choice_resumes_as_before() {
    let (mut runtime, folder) = started("claude", None);
    sleep(&mut runtime, &folder, "claude");
    let record = runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .get_mut(AGENT)
        .unwrap();
    let mut saved = serde_json::to_value(&*record).unwrap();
    assert!(
        saved.get("launch").is_none(),
        "no choice is written as none"
    );
    saved.as_object_mut().unwrap().remove("launch");
    *record = serde_json::from_value(saved).unwrap();
    let (herdr, starts) = starting_herdr("launch-choice-legacy-wake");
    wake(&mut runtime, &herdr);
    assert_eq!(next_start(&starts).1, resumed("claude", &[]));
    assert!(runtime.snapshot().ui_state.pane_launches.is_empty());
}

#[test]
fn a_fork_starts_with_the_model_and_effort_its_source_was_started_with() {
    for (kind, launch, resume, words) in [
        (
            "claude",
            choice("sonnet", "high"),
            vec!["--resume", SESSION, "--fork-session"],
            vec!["--model", "sonnet", "--effort", "high"],
        ),
        (
            "codex",
            choice("gpt-6-luna", "low"),
            vec!["fork", SESSION],
            vec!["--model", "gpt-6-luna", "-c", "model_reasoning_effort=low"],
        ),
    ] {
        let (runtime, _folder) = started(kind, Some(&launch));
        let (herdr, starts) = starting_herdr("launch-choice-fork");
        let shared = SharedRuntime::new(runtime);
        {
            let mut runtime = shared.lock().unwrap();
            let live = runtime.live.as_mut().unwrap();
            live.api_connector = Arc::new(herdr.connector());
            live.runtime = shared.weak();
            assert!(runtime.dispatch_json(&event(
                "fork_pane",
                json!({"pane_id": AGENT, "placement": "side", "origin": "operator"})
            )));
        }
        assert_eq!(
            next_start(&starts),
            (
                "w-order:t2:fork".to_owned(),
                json!([resume, words].concat())
            ),
            "{kind}"
        );
        wait(&shared, "the fork to settle", |runtime| {
            runtime.forks_in_flight.is_empty()
        });
        // The fork is a new agent on its source's choice, under the name
        // the fork started it with.
        let record = shared
            .lock()
            .unwrap()
            .snapshot()
            .ui_state
            .pane_launches
            .get("w-order:t2:fork")
            .cloned()
            .expect("the fork's pane keeps the choice");
        assert_eq!(
            (record.kind.as_str(), &record.choice, record.session),
            (kind, &launch, None),
            "{kind}"
        );
        assert!(record.name.starts_with("fork-"), "{}", record.name);
    }
}

/// A saved choice the agent does not take (a store edited by hand) refuses
/// the fork with the reason before any pane is made for it.
#[test]
fn a_fork_on_a_choice_its_agent_does_not_take_is_refused_before_a_pane_is_made() {
    let launch = LaunchChoice {
        model: None,
        effort: Some("ultra".into()),
    };
    let (mut runtime, _folder) = started("codex", Some(&launch));
    let (herdr, starts) = starting_herdr("launch-choice-fork-refused");
    runtime.live.as_mut().unwrap().api_connector = Arc::new(herdr.connector());
    runtime.dispatch_json(&event(
        "fork_pane",
        json!({"pane_id": AGENT, "placement": "side", "origin": "operator"}),
    ));
    let error = runtime.snapshot().status.last_error.clone().unwrap();
    assert_eq!(error.kind, "pane.fork_invalid_launch_choice");
    assert!(error.message.contains("ultra"), "{}", error.message);
    assert!(runtime.forks_in_flight.is_empty());
    assert!(starts.recv_timeout(Duration::from_millis(200)).is_err());
    assert!(herdr.methods().is_empty(), "{:?}", herdr.methods());
}

/// Closing a pane keeps its agent's choice with the closed pane, and the
/// reopen's answer records it on the pane the reopen made.
#[test]
fn a_closed_agent_takes_its_choice_to_the_pane_its_reopen_makes() {
    let launch = choice("sonnet", "high");
    let (mut runtime, _folder) = started("claude", Some(&launch));
    let tab = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .find(|tab| tab.id.as_deref() == Some("w-order:t2"))
        .cloned()
        .unwrap();
    let closed = runtime.closed_panes(&tab);
    assert_eq!(
        closed[0].agent,
        Some(ClosedAgent {
            kind: "claude".into(),
            session_id: Some(SESSION.into()),
            launch: launch.clone(),
        })
    );

    let item = closed_file("closed", "/repo/closed.rs");
    runtime.push_recent_closed(item.clone());
    runtime.reopen_in_flight = Some("closed".to_owned());
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item,
        workspace_exists: true,
        tab_exists: true,
        fallback_pane_id: None,
        owner: None,
    };
    assert!(runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: None,
            consumed: true,
            focused_pane_id: None,
            notices: vec![],
            agents: vec![live::ReopenedAgent {
                pane_id: "w-order:t3:p".into(),
                kind: "claude".into(),
                name: "reopen-closed-0".into(),
                launch: launch.clone(),
                session_id: Some(SESSION.into()),
            }],
        })),
    ));
    assert_eq!(
        runtime
            .snapshot()
            .ui_state
            .pane_launches
            .get("w-order:t3:p"),
        Some(&PaneLaunch {
            kind: "claude".into(),
            name: "reopen-closed-0".into(),
            choice: launch,
            session: Some(SESSION.into()),
        })
    );
}

/// A record describes only the agent Hide started, by the name it started it
/// under: an agent started in the same pane by hand (no name, or another),
/// one of another kind, or a pane that is gone drops it, and an unnamed agent
/// matches only by the conversation the named one last reported, as one back
/// from a Herdr restart without its name does.
#[test]
fn a_launch_record_describes_only_the_agent_hide_started_under_its_name() {
    let launch = choice("sonnet", "high");
    let (mut runtime, folder) = started("claude", Some(&launch));
    let checkout = checkout_of(&folder);
    let launches = |runtime: &Runtime| runtime.snapshot().ui_state.pane_launches.clone();
    let described = |runtime: &Runtime| {
        let agent = runtime
            .snapshot()
            .navigator
            .agents
            .iter()
            .find(|agent| agent.pane_id == AGENT)
            .cloned()
            .expect("the t2 agent");
        runtime.launch_choice_of(&agent)
    };
    assert_eq!(
        launches(&runtime).get(AGENT),
        Some(&PaneLaunch {
            kind: "claude".into(),
            name: NAME.into(),
            choice: launch.clone(),
            session: Some(SESSION.into()),
        })
    );
    assert_eq!(described(&runtime), launch);

    // The named agent's own new conversation (Claude Code `/clear`) is
    // still the agent Hide started with the choice.
    let cleared = "22222222-2222-3333-4444-555555555555";
    runtime.ingest_session(Ok(hosting(
        &checkout,
        Some(("claude", Some(NAME), Some(cleared))),
    )));
    assert_eq!(described(&runtime), launch);
    assert_eq!(launches(&runtime)[AGENT].session.as_deref(), Some(cleared));

    // Back from a Herdr restart without its name, on the same conversation.
    runtime.ingest_session(Ok(hosting(
        &checkout,
        Some(("claude", None, Some(cleared))),
    )));
    assert_eq!(described(&runtime), launch);

    // Started by hand in the pane before it reported a conversation: never
    // described, and dropped once its own conversation shows.
    runtime.ingest_session(Ok(hosting(&checkout, Some(("claude", None, None)))));
    assert_eq!(described(&runtime), LaunchChoice::default());
    assert!(launches(&runtime).contains_key(AGENT));
    let by_hand = "99999999-2222-3333-4444-555555555555";
    runtime.ingest_session(Ok(hosting(
        &checkout,
        Some(("claude", None, Some(by_hand))),
    )));
    assert_eq!(described(&runtime), LaunchChoice::default());
    assert!(launches(&runtime).is_empty());

    // Started under another name (another Hide start, or a reused pane id).
    runtime.record_pane_launch(AGENT, "claude", NAME, &launch, None);
    runtime.ingest_session(Ok(hosting(
        &checkout,
        Some(("claude", Some("other"), None)),
    )));
    assert!(launches(&runtime).is_empty());

    runtime.record_pane_launch(AGENT, "claude", NAME, &launch, None);
    runtime.ingest_session(Ok(session(&checkout, "codex", true)));
    assert!(launches(&runtime).is_empty());

    runtime.record_pane_launch(AGENT, "claude", NAME, &launch, Some(SESSION));
    runtime.ingest_session(Ok(tab_order_payload(
        &checkout,
        &TABS[..1],
        &TABS[..1],
        "w-order:t1",
    )));
    assert!(launches(&runtime).is_empty());
}

/// The records are bounded: one per pane a registered machine's Herdr holds,
/// a removed device takes its own with it, and past the cap a start is
/// recorded nowhere and logged.
#[test]
fn launch_records_leave_with_their_device_and_stop_at_the_cap() {
    let launch = choice("sonnet", "high");
    let (mut runtime, _folder) = started("claude", Some(&launch));
    let device = "device-a";
    runtime
        .snapshot
        .ui_state
        .device_registrations
        .push(crate::model::DeviceRegistration {
            id: device.to_owned(),
            label: device.to_owned(),
            origin: crate::model::LinkOrigin::Dialed {
                ssh_alias: device.to_owned(),
            },
            herdr_socket_path: None,
            host_consent: None,
        });
    let remote = format!("remote:{device}:pane:w1:p1");
    runtime.record_pane_launch(&remote, "claude", "hide-claude-w1-p1", &launch, None);
    assert!(runtime.dispatch_json(&event("remove_device", json!({"device_id": device}))));
    let launches = &runtime.snapshot().ui_state.pane_launches;
    assert!(!launches.contains_key(&remote));
    assert!(launches.contains_key(AGENT), "this machine's record stays");

    for index in launches.len()..crate::runtime::agent_choice::PANE_LAUNCH_LIMIT {
        runtime.record_pane_launch(&format!("w9:p{index}"), "claude", "name", &launch, None);
    }
    let ((), records) = crate::diagnostics::capture(|| {
        runtime.record_pane_launch("w9:over", "claude", "name", &launch, None);
    });
    let launches = &runtime.snapshot().ui_state.pane_launches;
    assert_eq!(
        launches.len(),
        crate::runtime::agent_choice::PANE_LAUNCH_LIMIT
    );
    assert!(!launches.contains_key("w9:over"));
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "pane_launch.limit"),
        "{records:?}"
    );
}

/// The record outlives the daemon, so a wake after a restart still has it.
#[test]
fn a_launch_record_survives_a_restart() {
    let launch = choice("gpt-6-luna", "low");
    let (mut runtime, _folder) = started("codex", Some(&launch));
    let path = runtime.state_path.clone();
    let _state = hold_dirs(&mut runtime);
    runtime.write_ui_state().unwrap();
    let saved = runtime.snapshot().ui_state.pane_launches.clone();
    assert_eq!(saved.len(), 1);
    drop(runtime);
    let restarted = Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: Some("/tmp/herdr-core-pet-runtime.sock".to_owned()),
            herdr_bin_path: None,
            app_state_path: path.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        },
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: None,
            codex_home: None,
        },
        Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    );
    assert_eq!(restarted.snapshot().ui_state.pane_launches, saved);
}
