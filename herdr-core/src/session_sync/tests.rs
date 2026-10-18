use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::model::{CoreOptions, SCHEMA_VERSION};
use hide_herdr_client::HERDR_PROTOCOL_REVISION;
use hide_platform::ipc::{LocalListener, LocalStream};

fn snapshot() -> Value {
    json!({
        "version": "0.8.2",
        "protocol": HERDR_PROTOCOL_REVISION,
        "focused_pane_id": "w1:p1",
        "workspaces": [{
            "workspace_id": "w1",
            "label": "fixture",
            "agent_status": "idle", "focused": true, "number": 1, "pane_count": 1, "tab_count": 1,
            "active_tab_id": "w1:t1"
        }],
        "tabs": [{
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "agent_status": "idle", "focused": false, "number": 1, "pane_count": 1, "label": "1"
        }],
        "panes": [{
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "pane_id": "w1:p1", "terminal_id": "fixture-terminal", "focused": false, "revision": 0, "agent_status": "idle",
            "cwd": "/tmp/fixture"
        }],
        "layouts": [{
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": "w1:p1",
            "panes": [{
                "pane_id": "w1:p1", "focused": false,
                "rect": {"x": 0, "y": 0, "width": 120, "height": 60}
            }],
            "splits": []
        }],
        "agents": []
    })
}

#[test]
fn every_session_sync_snapshot_fixture_obeys_the_generated_contract() {
    for (fixture, panes) in [(snapshot(), 1), (two_tab_snapshot(), 2)] {
        let state = wire::snapshot(fixture).expect("generated snapshot contract");
        assert_eq!(state.panes.len(), panes);
    }
}

/// Coexistence B1/B7: terminal failure survives both projections and a pane
/// update, but a missing cwd alone never makes a live shell a failed terminal.
#[test]
fn restoration_failure_survives_snapshot_updates_and_remote_projection() {
    let mut value = snapshot();
    value["panes"][0]["cwd"] = json!("/missing/한글 폴더");
    let mut replica = SessionReplica::from_snapshot(&value).unwrap();
    assert!(replica.project().panes[0].failed_terminal_id.is_none());
    value["panes"][0]["restore_error"] = json!("internal ENOENT detail");
    let update = event(
        "pane_updated",
        json!({"type":"pane_updated", "pane":value["panes"][0]}),
    );
    replica.apply(update, ApplyMode::Strict).unwrap();
    assert_eq!(
        replica.project().panes[0].failed_terminal_id.as_deref(),
        Some("fixture-terminal")
    );
    for replica in [replica, SessionReplica::from_snapshot(&value).unwrap()] {
        let (remote, _) = replica.project_remote("device", replica.project()).unwrap();
        let pane = &remote.workspaces[0].checkouts[0].tabs[0].panes[0];
        assert_eq!(pane.failed_terminal_id.as_deref(), Some("fixture-terminal"));
        assert!(!serde_json::to_string(pane).unwrap().contains("ENOENT"));
        assert!(
            remote.agents.is_empty(),
            "a shell failure must not invent an agent"
        );
    }
}

fn two_tab_snapshot() -> Value {
    let mut value = snapshot();
    value["focused_workspace_id"] = json!("w1");
    value["tabs"]
        .as_array_mut()
        .expect("tabs array")
        .push(json!({
            "workspace_id": "w1",
            "tab_id": "w1:t2",
            "agent_status": "idle", "focused": false, "number": 2, "pane_count": 1, "label": "2"
        }));
    value["panes"]
        .as_array_mut()
        .expect("panes array")
        .push(json!({
            "workspace_id": "w1",
            "tab_id": "w1:t2",
            "pane_id": "w1:p2", "terminal_id": "fixture-terminal", "focused": false, "revision": 0, "agent_status": "idle",
            "cwd": "/tmp/fixture"
        }));
    value["layouts"]
        .as_array_mut()
        .expect("layouts array")
        .push(json!({
            "workspace_id": "w1",
            "tab_id": "w1:t2",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": "w1:p2",
            "panes": [{
                "pane_id": "w1:p2", "focused": false,
                "rect": {"x": 0, "y": 0, "width": 120, "height": 60}
            }],
            "splits": []
        }));
    value
}

fn event(kind: &str, data: Value) -> ReplicaEvent {
    let raw = json!({"event": kind, "data": data});
    match parse_subscription_line(&raw.to_string()).expect("event parses") {
        SubscriptionLine::Event(event) => event,
        SubscriptionLine::EventsLost { .. } | SubscriptionLine::Error { .. } => unreachable!(),
    }
}

fn remote_focus(replica: &SessionReplica) -> (Option<String>, Option<String>, Option<String>) {
    let (session, _) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    (
        session.focused_checkout_id,
        session.focused_tab_id,
        session.focused_pane_id,
    )
}

fn assert_remote_replacement_focus(replica: &SessionReplica) {
    assert_eq!(
        remote_focus(replica),
        (
            Some("remote:mini:checkout:w1".to_owned()),
            Some("remote:mini:tab:w1:t2".to_owned()),
            Some("remote:mini:pane:w1:p2".to_owned()),
        )
    );
}

/// A session can fold Herdr's moves into one state; the replica keeps each
/// `tab_focused` it applies as a move, so a reader can tell t2 from t2, t3,
/// t2. Only the latest `TAB_FOCUS_LIMIT` are kept, and the count says how
/// many there were. A snapshot read on its own carries none.
#[test]
fn every_applied_tab_focus_is_one_move_and_the_latest_are_kept() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).unwrap();
    let focus = |tab: &str| {
        event(
            "tab_focused",
            json!({"type":"tab_focused","workspace_id":"w1","tab_id":tab}),
        )
    };
    for tab in ["w1:t2", "w1:t1", "w1:t2"] {
        replica.apply(focus(tab), ApplyMode::Strict).unwrap();
    }
    let moves = replica
        .project()
        .tab_moves
        .expect("a replica keeps its moves");
    assert_eq!(moves.applied, 3);
    assert_eq!(moves.recent, ["w1:t2", "w1:t1", "w1:t2"]);

    let limit = crate::sidebar::TAB_FOCUS_LIMIT;
    for index in 0..limit {
        let tab = if index % 2 == 0 { "w1:t1" } else { "w1:t2" };
        replica.apply(focus(tab), ApplyMode::Strict).unwrap();
    }
    let moves = replica
        .project()
        .tab_moves
        .expect("a replica keeps its moves");
    assert_eq!(moves.applied, 3 + limit as u64);
    assert_eq!(moves.recent.len(), limit);
    assert_eq!(moves.recent.back().map(String::as_str), Some("w1:t2"));
    assert!(
        moves.since(moves.generation, 3).is_some(),
        "the moves after the first three are all kept"
    );
    assert!(
        moves.since(moves.generation, 2).is_none(),
        "the third move was dropped"
    );
    assert!(moves.since(moves.generation + 1, 3).is_none());

    assert!(
        crate::session_sync::project_snapshot(&two_tab_snapshot())
            .unwrap()
            .tab_moves
            .is_none()
    );
}

/// A move into a workspace still waiting for its layout publishes nothing by
/// itself: the held state lacks the workspace the creation worker already
/// projected, and publishing it retired that workspace's pane before its
/// terminal attached (Windows CI). The move arrives with the publish that
/// releases the workspace.
#[test]
fn a_move_into_a_held_workspace_rides_on_the_publish_that_releases_it() {
    let created: Value = serde_json::from_str(&snapshot().to_string().replace("w1", "w2")).unwrap();
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let mut apply = |kind: &str, data: Value| {
        replica
            .apply(event(kind, data), ApplyMode::Strict)
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"))
            .publish
    };
    apply(
        "workspace_created",
        json!({"type": "workspace_created", "workspace": created["workspaces"][0]}),
    );
    apply(
        "tab_created",
        json!({"type": "tab_created", "tab": created["tabs"][0]}),
    );
    assert!(
        !apply(
            "tab_focused",
            json!({"type": "tab_focused", "workspace_id": "w2", "tab_id": "w2:t1"}),
        ),
        "the move waits for the workspace it moved into"
    );
    apply(
        "pane_created",
        json!({"type": "pane_created", "workspace_id": "w2", "tab_id": "w2:t1", "pane": created["panes"][0]}),
    );
    assert!(apply(
        "layout_updated",
        json!({"type": "layout_updated", "layout": created["layouts"][0]}),
    ));
    let published = replica.project();
    assert!(
        published
            .workspaces
            .iter()
            .any(|workspace| workspace.workspace_id == "w2")
    );
    let moves = published.tab_moves.expect("a replica keeps its moves");
    assert_eq!(moves.applied, 1);
    assert_eq!(moves.recent, ["w2:t1"]);
}

#[test]
fn creation_focus_cause_is_retired_when_another_workspace_takes_focus() {
    let mut value = two_tab_snapshot();
    let second: Value = serde_json::from_str(&snapshot().to_string().replace("w1", "w2")).unwrap();
    for field in ["workspaces", "tabs", "panes", "layouts"] {
        value[field]
            .as_array_mut()
            .unwrap()
            .extend(second[field].as_array().unwrap().clone());
    }
    let mut replica = SessionReplica::from_snapshot(&value).unwrap();
    let mut tab = value["tabs"][1].clone();
    tab["focused"] = json!(true);
    replica
        .apply(
            event("tab_created", json!({"type":"tab_created","tab":tab})),
            ApplyMode::Reconcile,
        )
        .unwrap();
    replica
        .apply(
            event(
                "tab_focused",
                json!({"type":"tab_focused","workspace_id":"w1","tab_id":"w1:t2"}),
            ),
            ApplyMode::Strict,
        )
        .unwrap();
    assert!(replica.project().tab_focus.unwrap().creation);
    for workspace in ["w2", "w1"] {
        replica
            .apply(
                event(
                    "workspace_focused",
                    json!({"type":"workspace_focused","workspace_id":workspace}),
                ),
                ApplyMode::Strict,
            )
            .unwrap();
        assert_eq!(
            replica.project().focused_workspace_id.as_deref(),
            Some(workspace)
        );
        assert!(
            replica.project().tab_focus.is_none(),
            "a return cannot resurrect the old creation cause"
        );
    }
}

#[test]
fn creation_focus_pairing_survives_bootstrap_and_blocked_publication() {
    for bootstrap in [false, true] {
        for explicit_before_layout in [false, true] {
            let created = two_tab_snapshot();
            let mut replica = SessionReplica::from_snapshot(&if bootstrap {
                created.clone()
            } else {
                snapshot()
            })
            .unwrap();
            let mut tab = created["tabs"][1].clone();
            tab["focused"] = json!(true);
            replica
                .apply(
                    event("tab_created", json!({"type":"tab_created","tab":tab})),
                    if bootstrap {
                        ApplyMode::Reconcile
                    } else {
                        ApplyMode::Strict
                    },
                )
                .unwrap();
            let focus = || {
                event(
                    "tab_focused",
                    json!({"type":"tab_focused","workspace_id":"w1","tab_id":"w1:t2"}),
                )
            };
            replica.apply(focus(), ApplyMode::Strict).unwrap();
            if explicit_before_layout {
                replica.apply(focus(), ApplyMode::Strict).unwrap();
            }
            if !bootstrap {
                assert!(
                    replica.project().tab_focus.is_none(),
                    "blocked membership keeps its old focus provenance"
                );
                replica
                    .apply(
                        event(
                            "pane_created",
                            json!({"type":"pane_created","pane":created["panes"][1]}),
                        ),
                        ApplyMode::Strict,
                    )
                    .unwrap();
                replica
                    .apply(
                        event(
                            "layout_updated",
                            json!({"type":"layout_updated","layout":created["layouts"][1]}),
                        ),
                        ApplyMode::Strict,
                    )
                    .unwrap();
            }
            let before = replica.project().tab_focus.unwrap();
            assert_eq!(before.creation, !explicit_before_layout);
            assert_eq!(before.tab_id, "w1:t2");
            assert!(
                replica.apply(focus(), ApplyMode::Strict).unwrap().publish,
                "an explicit same-tab focus must publish its new intent"
            );
            let after = replica.project().tab_focus.unwrap();
            assert!(!after.creation);
            assert!(after.revision > before.revision);
        }
    }
}

#[test]
fn creation_focus_is_published_with_membership_before_its_later_focus_event() {
    for focused_at_creation in [false, true] {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let created = two_tab_snapshot();
        let mut tab = created["tabs"][1].clone();
        tab["focused"] = json!(focused_at_creation);
        let change = replica
            .apply(
                event("tab_created", json!({"type":"tab_created", "tab":tab})),
                ApplyMode::Strict,
            )
            .expect("created tab");
        assert!(!change.publish);
        replica
            .apply(
                event(
                    "pane_created",
                    json!({"type":"pane_created", "pane":created["panes"][1]}),
                ),
                ApplyMode::Strict,
            )
            .expect("created pane");
        let change = replica
            .apply(
                event(
                    "layout_updated",
                    json!({"type":"layout_updated", "layout":created["layouts"][1]}),
                ),
                ApplyMode::Strict,
            )
            .expect("created layout");
        assert!(change.publish);
        let before = replica.project();
        assert_eq!(
            before.workspaces[0].active_tab_id.as_deref(),
            Some(if focused_at_creation {
                "w1:t2"
            } else {
                "w1:t1"
            })
        );
        replica
            .apply(
                event(
                    "tab_focused",
                    json!({"type":"tab_focused", "workspace_id":"w1", "tab_id":"w1:t2"}),
                ),
                ApplyMode::Strict,
            )
            .expect("later focus");
        replica
            .apply(
                event(
                    "pane_focused",
                    json!({"type":"pane_focused", "workspace_id":"w1", "pane_id":"w1:p2"}),
                ),
                ApplyMode::Strict,
            )
            .expect("later pane focus");
        let after = replica.project();
        assert_eq!(
            after.tab_focus.as_ref().unwrap().creation,
            focused_at_creation
        );
        assert_eq!(after.workspaces[0].active_tab_id.as_deref(), Some("w1:t2"));
        assert_eq!(
            before.workspaces[0].active_tab_id == after.workspaces[0].active_tab_id,
            focused_at_creation,
            "creation focus is one transition; a later explicit focus after --no-focus remains a new transition"
        );
    }
}

#[test]
fn worktree_events_invalidate_the_change_driven_reader_once() {
    let value = snapshot();
    let workspace = value["workspaces"][0].clone();
    let mut replica = SessionReplica::from_snapshot(&value).expect("snapshot");
    let opened = event(
        "worktree_opened",
        json!({
            "type": "worktree_opened",
            "workspace": workspace,
            "worktree": {"path": "/tmp/fixture", "is_bare": false, "is_detached": false, "is_prunable": false, "is_linked_worktree": true, "label": "fixture"},
            "already_open": true
        }),
    );
    let outcome = replica
        .apply(opened, ApplyMode::Strict)
        .expect("worktree event");
    assert!(outcome.refresh_worktrees);
}

/// A workspace's first tab is named before its tab and pane events arrive.
/// Publishing that incomplete workspace can erase the complete pane snapshot
/// already delivered by the workspace creation worker, losing its view size.
#[test]
fn a_new_workspace_waits_for_its_first_tab_layout_before_publication() {
    let created: Value = serde_json::from_str(&snapshot().to_string().replace("w1", "w2")).unwrap();
    let mut empty = snapshot();
    empty["focused_pane_id"] = Value::Null;
    for field in ["workspaces", "tabs", "panes", "layouts"] {
        empty[field] = json!([]);
    }
    for initial in [empty, snapshot()] {
        let mut replica = SessionReplica::from_snapshot(&initial).unwrap();
        let before = replica.project();
        for next in [
            event(
                "workspace_created",
                json!({"type":"workspace_created", "workspace":created["workspaces"][0]}),
            ),
            event(
                "workspace_focused",
                json!({"type":"workspace_focused", "workspace_id":"w2"}),
            ),
            event(
                "tab_created",
                json!({"type":"tab_created", "tab":created["tabs"][0]}),
            ),
            event(
                "pane_created",
                json!({"type":"pane_created", "pane":created["panes"][0]}),
            ),
        ] {
            let outcome = replica.apply(next, ApplyMode::Strict).unwrap();
            assert!(!outcome.publish, "the first tab has no complete layout yet");
            let projected = replica.project();
            assert_eq!(projected.workspaces.len(), before.workspaces.len());
            assert_eq!(projected.panes.len(), before.panes.len());
            assert_eq!(projected.layouts.len(), before.layouts.len());
            assert_eq!(projected.focused_pane_id, before.focused_pane_id);
        }
        let outcome = replica
            .apply(
                event(
                    "layout_updated",
                    json!({"type":"layout_updated", "layout":created["layouts"][0]}),
                ),
                ApplyMode::Strict,
            )
            .unwrap();
        assert!(outcome.publish);
        let projected = replica.project();
        assert!(projected.panes.iter().any(|pane| pane.pane_id == "w2:p1"));
        assert!(
            projected
                .layouts
                .iter()
                .any(|layout| layout.tab_id == "w2:t1")
        );
        for pane in before.panes {
            assert!(
                projected
                    .panes
                    .iter()
                    .any(|current| current.pane_id == pane.pane_id)
            );
        }
    }
}

/// Where a test's socket folder goes: directly under `/tmp` on Unix, whose
/// socket paths have a hard length limit, and the temporary folder on
/// Windows, which names a pipe after the path.
fn socket_parent() -> std::path::PathBuf {
    if cfg!(unix) {
        Path::new("/tmp").to_path_buf()
    } else {
        std::env::temp_dir()
    }
}

fn accept_request(listener: &LocalListener) -> (LocalStream, Value) {
    let stream = listener.accept().expect("accept request");
    let mut line = String::new();
    // The client writes one line and then only reads, so nothing past the
    // line is left in the reader's buffer when the stream is taken back.
    let mut reader = std::io::BufReader::new(stream);
    reader.read_line(&mut line).expect("read request");
    let request = serde_json::from_str(&line).expect("request JSON");
    (reader.into_inner(), request)
}

fn write_result(stream: &mut LocalStream, request: &Value, result: Value) {
    writeln!(stream, "{}", json!({"id": request["id"], "result": result})).expect("write response");
}

/// Accepts requests until the coordinator asks for the named method.
///
/// The coordinator refreshes agent telemetry once a second, on its own
/// clock, so an `agent.list` can land between any two requests a test is
/// interested in. Asserting that the very next request is the awaited one
/// therefore asserts the scheduler: these tests failed on it in one run
/// out of three with no change to the code under test. Each interleaved
/// refresh is answered with an empty list, which is what the coordinator
/// expects and what leaves its projection untouched.
fn accept_request_for(listener: &LocalListener, method: &str) -> (LocalStream, Value) {
    for _ in 0..32 {
        let (mut stream, request) = accept_request(listener);
        if request["method"] == method {
            return (stream, request);
        }
        let answer = match request["method"].as_str() {
            Some("agent.list") => json!({"type": "agent_list", "agents": []}),
            Some("pane.process_info") => json!({"type": "pane_process_info", "process_info": {
                "pane_id": request["params"]["pane_id"], "foreground_processes": []
            }}),
            _ => panic!(
                "unexpected capability read while {method} is awaited: {}",
                request["method"]
            ),
        };
        write_result(&mut stream, &request, answer);
    }
    panic!("the coordinator never asked for {method}");
}

#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_until(deadline: Instant, mut predicate: impl FnMut() -> bool) {
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(predicate(), "condition did not become true before deadline");
}

fn runtime_for_fixture(socket_path: &Path, state_path: &Path) -> Arc<Mutex<Runtime>> {
    Arc::new(Mutex::new(Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: Some(socket_path.to_string_lossy().into_owned()),
            herdr_bin_path: None,
            app_state_path: state_path.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        },
        crate::environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: None,
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    )))
}

fn context_for_fixture(runtime: &Arc<Mutex<Runtime>>, socket_path: &Path) -> SessionSyncContext {
    let live = LiveContext {
        socket_path: socket_path.to_path_buf(),
        runtime: Arc::downgrade(runtime),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    };
    SessionSyncContext::local(&live, std::sync::Arc::new(hide_node::Local::of_process()))
}

fn remove_fixture(folder: tempfile::TempDir, runtime: Arc<Mutex<Runtime>>) {
    drop(runtime);
    folder.close().expect("remove fixture directory");
}

/// Hold the host protocol, not a helper inside the coordinator. Both the
/// startup and Settings paths must publish Herdr's layout before release.
#[test]
fn held_hook_diagnosis_does_not_hold_startup_or_settings_topology() {
    use crate::node_access::{LinkAnswer, LinkError, NodeLink};
    use hide_node_link::protocol::Call;
    use std::sync::{Condvar, mpsc};

    struct Host {
        held: Mutex<bool>,
        released: Condvar,
        started: mpsc::Sender<()>,
        finished: mpsc::Sender<()>,
    }
    impl NodeLink for Host {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            if matches!(call, Call::HookDiagnosis) {
                let mut held = self.held.lock().unwrap();
                self.started.send(()).unwrap();
                while *held {
                    held = self.released.wait(held).unwrap();
                }
                self.finished.send(()).unwrap();
                return Ok(json!({"runtimes": [], "last_report_failure": null}).into());
            }
            Err(LinkError::NotConnected(
                "private fixture denies other host work".into(),
            ))
        }
    }
    struct Release(Arc<Host>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.held.lock().unwrap() = false;
            self.0.released.notify_all();
        }
    }
    struct Server {
        closer: hide_platform::ipc::ListenerCloser,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.closer.close();
            self.worker.take().unwrap().join().unwrap();
        }
    }

    let folder = tempfile::Builder::new()
        .prefix("hook-sync-")
        .tempdir_in(socket_parent())
        .unwrap();
    let socket = folder.path().join("herdr.sock");
    let listener = LocalListener::bind(&socket).unwrap();
    let closer = listener.closer();
    let (streams, subscription) = mpsc::channel();
    let server = Server {
        closer,
        worker: Some(thread::spawn(move || {
            while let Ok(stream) = listener.accept() {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let mut stream = reader.into_inner();
                let result = match request["method"].as_str().unwrap() {
                    "events.subscribe" => json!({"type": "subscription_started"}),
                    "session.snapshot" => {
                        json!({"type": "session_snapshot", "snapshot": snapshot()})
                    }
                    "agent.list" => json!({"type": "agent_list", "agents": []}),
                    "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                        "pane_id": request["params"]["pane_id"], "foreground_processes": []
                    }}),
                    method => panic!("unexpected fixture method {method}"),
                };
                let response = wire::checked_response_fixture(&request["id"], result);
                writeln!(stream, "{response}").unwrap();
                if request["method"] == "events.subscribe" {
                    streams.send(stream).unwrap();
                }
            }
        })),
    };
    let runtime = runtime_for_fixture(&socket, &folder.path().join("state.json"));
    let (started, calls) = mpsc::channel();
    let (finished, answers) = mpsc::channel();
    let host = Arc::new(Host {
        held: Mutex::new(true),
        released: Condvar::new(),
        started,
        finished,
    });
    let mut context = context_for_fixture(&runtime, &socket);
    context.node = Some(host.clone());
    let pump = crate::kit::KitPump::spawn(
        Arc::downgrade(&runtime),
        ChangeNotifier::noop(),
        host.clone(),
        None,
        crate::node::test_node(),
    )
    .unwrap();
    let handle = spawn(context, None).unwrap();
    // This guard releases before the owners join, including on assertion failure.
    let release = Release(host.clone());
    calls
        .recv_timeout(Duration::from_secs(10))
        .expect("startup diagnosis entered host");
    let mut stream = subscription
        .recv_timeout(Duration::from_secs(10))
        .expect("Herdr subscribes while startup diagnosis is held");
    wait_until(Instant::now() + Duration::from_secs(10), || {
        !runtime.lock().unwrap().snapshot().pane_layouts.is_empty()
    });
    drop(release);
    answers
        .recv_timeout(Duration::from_secs(10))
        .expect("startup diagnosis released");
    {
        let mut guard = runtime.lock().unwrap();
        guard.dispatch_json(
            &serde_json::to_vec(&json!({"schema_version": SCHEMA_VERSION,
            "kind": "ui_attached", "payload": {"attached": true}}))
            .unwrap(),
        );
    }
    // Wait for the first read's result before arming the next one.
    wait_until(Instant::now() + Duration::from_secs(10), || {
        runtime.lock().unwrap().snapshot().status.herdr.state == "connected"
    });
    *host.held.lock().unwrap() = true;
    let release = Release(host.clone());
    runtime.lock().unwrap().dispatch_json(
        &serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION, "kind": "ai_settings", "payload": {"observing": true}
        }))
        .unwrap(),
    );
    calls
        .recv_timeout(Duration::from_secs(10))
        .expect("Settings diagnosis entered host");
    let mut layout = snapshot()["layouts"][0].clone();
    layout["zoomed"] = json!(true);
    writeln!(
        stream,
        "{}",
        json!({"event":"layout_updated", "data":{"type":"layout_updated", "layout":layout}})
    )
    .unwrap();
    wait_until(Instant::now() + Duration::from_secs(10), || {
        runtime
            .lock()
            .unwrap()
            .snapshot()
            .pane_layouts
            .iter()
            .any(|layout| layout.zoomed)
    });
    drop(release);
    drop(handle);
    drop(pump);
    drop(stream);
    drop(server);
    remove_fixture(folder, runtime);
}

#[test]
fn split_is_published_only_after_the_authoritative_layout_arrives() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let created = replica
        .apply(
            event(
                "pane_created",
                json!({
                    "type": "pane_created",
                    "pane": {
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "pane_id": "w1:p2", "terminal_id": "fixture-terminal", "focused": false, "revision": 0, "agent_status": "idle",
                        "cwd": "/tmp/fixture"
                    }
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("pane event");
    assert!(!created.publish);

    let updated = replica
            .apply(event("layout_updated",
                json!({
                    "type": "layout_updated",
                    "layout": {
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "zoomed": false,
                        "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                        "focused_pane_id": "w1:p2",
                        "panes": [
                            {"pane_id": "w1:p1", "focused": false, "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                            {"pane_id": "w1:p2", "focused": false, "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
                        ],
                        "splits": [{
                            "id": "split_0_root",
                            "direction": "right",
                            "ratio": 0.5,
                            "rect": {"x": 0, "y": 0, "width": 120, "height": 60}
                        }]
                    }
                }),
            ), ApplyMode::Strict)
            .expect("layout event");
    assert!(updated.publish);
    assert_eq!(replica.project().panes.len(), 2);
    assert_eq!(replica.project().layouts[0].panes.len(), 2);
}

#[test]
fn agent_projection_keeps_the_herdr_name_as_its_control_identifier() {
    let mut value = snapshot();
    value["agents"] = json!([{
        "pane_id": "w1:p1",
        "workspace_id": "w1",
        "tab_id": "w1:t1",
        "terminal_id": "fixture-terminal",
        "focused": false,
        "revision": 1,
        "name": "observer",
        "agent": "claude",
        "agent_status": "working",
        "state_change_seq": 1,
        "tokens": {}
    }]);
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot");

    assert_eq!(replica.project().agents[0].id.as_deref(), Some("observer"));
}

/// ⌘K finds an agent by its Herdr name, so the snapshot's agent row carries
/// it, on this machine and on a device, whose row replaces `id`; an agent
/// given no name carries none rather than its pane id.
#[test]
fn the_snapshot_agent_row_carries_the_herdr_name_when_herdr_has_one() {
    fn wire_rows(name: Option<&str>) -> (Value, Value) {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let mut agent = json!({
            "pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1",
            "terminal_id": "fixture-terminal", "focused": false, "revision": 1,
            "agent": "claude", "agent_status": "working", "state_change_seq": 1, "tokens": {}
        });
        if let Some(name) = name {
            agent["name"] = json!(name);
        }
        let agents = wire::agents_response(json!({"type": "agent_list", "agents": [agent]}))
            .expect("an agent list");
        replica.replace_agents(agents);
        replica.refresh_published_state().expect("published");
        let local = crate::sidebar::project_agents(replica.project()).agents;
        let (remote, _) = replica
            .project_remote("mini", replica.project())
            .expect("remote projection");
        (
            serde_json::to_value(&local[0]).expect("local row"),
            serde_json::to_value(&remote.agents[0]).expect("remote row"),
        )
    }

    let (local, remote) = wire_rows(Some("observer-instant-pane-topology"));
    assert_eq!(local["herdr_name"], json!("observer-instant-pane-topology"));
    assert_eq!(
        remote["herdr_name"],
        json!("observer-instant-pane-topology")
    );
    assert_eq!(remote["pane_id"], json!("remote:mini:pane:w1:p1"));

    // No name, and the names Hide makes up from a pane id (a task's, a
    // wake's, a fork's), which would only spell the pane id.
    for name in [
        None,
        Some(crate::fork::task_agent_name("claude", "w1:p1")),
        Some(crate::fork::wake_name("w1:p1")),
        Some(crate::fork::fork_name("w1:p9", "3-1788624371518")),
    ] {
        let (local, remote) = wire_rows(name.as_deref());
        assert!(local.get("herdr_name").is_none(), "{name:?}: {local}");
        assert!(remote.get("herdr_name").is_none(), "{name:?}: {remote}");
    }
}

/// B3, D-03. A remote device's project list follows the same activity
/// order the local list follows, not its own alphabetical one. The labels
/// here are deliberately in the opposite order to the activity, so a
/// return to sorting by label fails this.
#[test]
fn remote_projects_follow_activity_order_rather_than_label_order() {
    let mut value = snapshot();
    value["workspaces"][0]["label"] = json!("alpha");
    value["workspaces"]
        .as_array_mut()
        .expect("workspaces array")
        .push(json!({
            "workspace_id": "w2",
            "label": "zulu",
            "agent_status": "idle", "focused": false, "number": 2, "pane_count": 1, "tab_count": 1,
            "active_tab_id": "w2:t1"
        }));
    value["tabs"]
        .as_array_mut()
        .expect("tabs array")
        .push(json!({
            "workspace_id": "w2",
            "tab_id": "w2:t1",
            "agent_status": "idle", "focused": false, "number": 1, "pane_count": 1, "label": "1"
        }));
    value["panes"]
        .as_array_mut()
        .expect("panes array")
        .push(json!({
            "workspace_id": "w2",
            "tab_id": "w2:t1",
            "pane_id": "w2:p1", "terminal_id": "fixture-terminal", "focused": false, "revision": 0, "agent_status": "idle",
            "cwd": "/tmp/fixture-zulu"
        }));
    value["layouts"]
        .as_array_mut()
        .expect("layouts array")
        .push(json!({
            "workspace_id": "w2",
            "tab_id": "w2:t1",
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": "w2:p1",
            "panes": [{
                "pane_id": "w2:p1", "focused": false,
                "rect": {"x": 0, "y": 0, "width": 120, "height": 60}
            }],
            "splits": []
        }));
    value["agents"] = json!([
        {
            "pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1",
            "agent": "codex", "agent_status": "idle", "focused": false, "revision": 0,
            "terminal_id": "fixture-terminal", "state_change_seq": 1, "tokens": {}
        },
        {
            "pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
            "agent": "codex", "agent_status": "working", "focused": false, "revision": 0,
            "terminal_id": "fixture-terminal", "state_change_seq": 2, "tokens": {}
        },
    ]);
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot");
    // The label worker times each agent's last state change.
    let mut payload = replica.project();
    for agent in &mut payload.agents {
        agent.changed_at_unix_ms = match agent.pane_id.as_deref() {
            Some("w1:p1") => Some(1_000_000),
            _ => Some(9_000_000),
        };
    }

    let (projected, _) = replica
        .project_remote("mini", payload)
        .expect("remote projection");

    assert_eq!(
        projected
            .workspaces
            .iter()
            .map(|workspace| workspace.label.as_str())
            .collect::<Vec<_>>(),
        ["zulu", "alpha"]
    );
    assert_eq!(
        projected.workspaces[0].last_activity_unix_ms,
        Some(9_000_000)
    );
    assert_eq!(
        projected.workspaces[1].last_activity_unix_ms,
        Some(1_000_000)
    );
}

#[test]
fn remote_projection_uses_target_scoped_ids_and_normalized_layout_frames() {
    let mut value = snapshot();
    value["agents"] = json!([{
        "pane_id": "w1:p1",
        "workspace_id": "w1",
        "tab_id": "w1:t1",
        "agent": "codex",
        "agent_status": "working", "focused": false, "revision": 0, "terminal_id": "fixture-terminal",
        "state_change_seq": 1,
        "tokens": {}
    }]);
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot");

    let (projected, excluded) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");

    assert!(excluded.is_empty());
    assert_eq!(projected.workspaces.len(), 1);
    assert_eq!(projected.workspaces[0].id, "remote:mini:workspace:w1");
    assert_eq!(
        projected.workspaces[0].checkouts[0].id,
        "remote:mini:checkout:w1"
    );
    assert_eq!(
        projected.focused_workspace_id.as_deref(),
        Some("remote:mini:workspace:w1")
    );
    assert_eq!(
        projected.focused_tab_id.as_deref(),
        Some("remote:mini:tab:w1:t1")
    );
    assert_eq!(
        projected.focused_pane_id.as_deref(),
        Some("remote:mini:pane:w1:p1")
    );
    assert_eq!(
        projected.workspaces[0].checkouts[0].tabs[0].id.as_deref(),
        Some("remote:mini:tab:w1:t1")
    );
    assert_eq!(
        projected.workspaces[0].checkouts[0].tabs[0].panes[0].id,
        "remote:mini:pane:w1:p1"
    );
    assert_eq!(projected.agents[0].pane_id, "remote:mini:pane:w1:p1");
    assert_eq!(projected.agents[0].demand, "none");
    assert_eq!(projected.agents[0].activity, "working");
    assert_eq!(
        projected.workspaces[0].checkouts[0].agent_summary.working,
        1
    );
    assert_eq!(
        projected.workspaces[0].checkouts[0]
            .purpose
            .as_ref()
            .map(|purpose| purpose.origin),
        Some(crate::model::CheckoutPurposeOrigin::AgentTitle)
    );
    assert_eq!(projected.pane_layouts[0].frames[0].x, 0.0);
    assert_eq!(projected.pane_layouts[0].frames[0].width, 1.0);
    assert!(projected.workspaces[0].checkouts[0].exists);
    assert_eq!(
        projected
            .active_tab_ids
            .get("remote:mini:checkout:w1")
            .map(String::as_str),
        Some("remote:mini:tab:w1:t1")
    );

    let (other_target, _) = replica
        .project_remote("build-mini", replica.project())
        .expect("other target");
    assert_ne!(
        projected.focused_tab_id, other_target.focused_tab_id,
        "the same remote tab id must not collide across targets"
    );
    assert_ne!(
        projected.focused_pane_id, other_target.focused_pane_id,
        "the same remote pane id must not collide across targets"
    );
    assert_ne!(
        projected.agents[0].pane_id, other_target.agents[0].pane_id,
        "agent routing follows the target-scoped pane identity"
    );
}

#[test]
fn remote_projection_keeps_workspace_purpose_tokens() {
    let mut value = snapshot();
    value["workspaces"][0]["tokens"] = json!({"purpose": "Remote purpose"});
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot");

    let (projected, _) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    let purpose = projected.workspaces[0].checkouts[0]
        .purpose
        .as_ref()
        .expect("workspace purpose");
    assert_eq!(purpose.text, "Remote purpose");
    assert_eq!(purpose.origin, crate::model::CheckoutPurposeOrigin::Token);
}

#[test]
fn remote_projection_preserves_official_worktree_metadata() {
    let mut value = snapshot();
    value["workspaces"][0]["worktree"] = json!({
        "repo_key": "/tmp/repo/.git",
        "repo_name": "repo",
        "repo_root": "/tmp/repo",
        "checkout_path": "/tmp/repo-linked",
        "is_linked_worktree": true
    });
    value["panes"][0]["cwd"] = json!("/tmp/repo-linked/subdirectory");
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot");

    let (projected, _) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    let workspace = &projected.workspaces[0];
    let checkout = &workspace.checkouts[0];

    assert_eq!(workspace.path, "/tmp/repo-linked");
    assert_eq!(workspace.repo_name, "repo");
    assert!(workspace.is_git);
    assert_eq!(checkout.path, "/tmp/repo-linked");
    assert!(checkout.is_worktree);
}

#[test]
fn remote_projection_rejects_an_empty_layout_area() {
    let mut value = snapshot();
    value["layouts"][0]["area"]["width"] = json!(0);
    let replica = SessionReplica::from_snapshot(&value).expect("snapshot shape");

    let error = replica
        .project_remote("mini", replica.project())
        .expect_err("empty layout area must be visible");

    assert_eq!(error.state(), "malformed");
    assert!(error.message().contains("empty area"));
}

#[test]
#[ignore = "requires HERDR_TEST_SSH_ALIAS"]
fn official_remote_session_coordinator_probe() {
    let alias_name = std::env::var("HERDR_TEST_SSH_ALIAS")
        .expect("HERDR_TEST_SSH_ALIAS names a configured SSH host");
    let home = std::env::var_os("HOME").expect("HOME is configured");
    let state_path = PathBuf::from("/tmp/herdr-core-remote-coordinator-probe-state.json");
    let _ = std::fs::remove_file(&state_path);
    let runtime = Arc::new(Mutex::new(Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: None,
            herdr_bin_path: None,
            app_state_path: state_path.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        },
        crate::environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: Some(PathBuf::from(&home)),
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::new(Some(PathBuf::from(home)))),
        crate::node::test_devices(),
    )));
    // The same path the shell takes: register the device, and the runtime
    // resolves the alias, asks the host for its socket and starts the
    // coordinator itself.
    {
        let mut guard = runtime.lock().expect("runtime lock");
        guard.install_worker_context(
            Arc::downgrade(&runtime),
            crate::handle::ChangeNotifier::noop(),
        );
        let register_device = serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "register_device",
            "payload": { "id": "probe", "label": "Probe", "ssh_alias": alias_name }
        }))
        .expect("register device event");
        assert!(guard.dispatch_json(&register_device));
    }

    wait_until(Instant::now() + Duration::from_secs(15), || {
        runtime.lock().ok().is_some_and(|runtime| {
            let status = &runtime.snapshot().status.remote[0];
            status.state == "connected" && status.session.is_some()
        })
    });
    let mut guard = runtime.lock().expect("runtime lock");
    let status = &guard.snapshot().status.remote[0];
    assert_eq!(status.state, "connected", "{:?}", status.message);
    let session = status.session.as_ref().expect("remote session projected");
    assert!(!session.agents.is_empty());
    assert_eq!(
        guard.snapshot().navigator.devices[1].agent_count as usize,
        session.agents.len()
    );
    assert_eq!(guard.snapshot().navigator.devices[1].state, "ready");
    let handles = guard.take_remote_syncs();
    drop(guard);
    drop(handles);
}

#[test]
fn workspace_close_cascade_clears_pending_layout_and_nested_state() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let pane_closed = replica
        .apply(
            event(
                "pane_closed",
                json!({
                    "type": "pane_closed",
                    "pane_id": "w1:p1",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("pane close event");
    assert!(!pane_closed.publish);

    let workspace_closed = replica
        .apply(
            event(
                "workspace_closed",
                json!({
                    "type": "workspace_closed",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("workspace close event");
    assert!(workspace_closed.publish);
    assert!(replica.ready_to_publish());

    let projected = replica.project();
    assert!(projected.workspaces.is_empty());
    assert!(projected.tabs.is_empty());
    assert!(projected.panes.is_empty());
    assert!(projected.layouts.is_empty());
    assert_eq!(projected.focused_pane_id, None);
}

#[test]
fn last_pane_close_removes_an_implicitly_closed_inactive_tab() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");

    let pane_closed = replica
        .apply(
            event(
                "pane_closed",
                json!({
                    "type": "pane_closed",
                    "pane_id": "w1:p2",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("last pane close event");

    assert!(pane_closed.publish);
    assert!(replica.ready_to_publish());
    let projected = replica.project();
    assert_eq!(
        projected
            .tabs
            .iter()
            .map(|tab| tab.tab_id.as_str())
            .collect::<Vec<_>>(),
        vec!["w1:t1"]
    );
    assert!(projected.panes.iter().all(|pane| pane.pane_id != "w1:p2"));
    assert!(
        projected
            .layouts
            .iter()
            .all(|layout| layout.tab_id != "w1:t2")
    );
}

#[test]
fn last_pane_close_waits_for_the_authoritative_fallback_tab_focus() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");

    let pane_closed = replica
        .apply(
            event(
                "pane_closed",
                json!({
                    "type": "pane_closed",
                    "pane_id": "w1:p1",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("last pane close event");
    assert!(!pane_closed.publish);
    assert!(!replica.ready_to_publish());

    let workspace_focused = replica
        .apply(
            event(
                "workspace_focused",
                json!({
                    "type": "workspace_focused",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("workspace focus event");
    assert!(!workspace_focused.publish);

    let tab_focused = replica
        .apply(
            event(
                "tab_focused",
                json!({
                    "type": "tab_focused",
                    "tab_id": "w1:t2",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("fallback tab focus event");
    assert!(tab_focused.publish);
    assert!(replica.ready_to_publish());
    assert_eq!(replica.state.workspaces[0].active_tab_id, "w1:t2");
    assert_remote_replacement_focus(&replica);
    assert!(
        replica
            .project()
            .tabs
            .iter()
            .all(|tab| tab.tab_id != "w1:t1")
    );
}

/// The remote context browses Herdr's tabs and has no file tabs to mix in,
/// so its strip is the Herdr tab list in Herdr's order and nothing else.
#[test]
fn remote_projection_tab_list_and_order_are_herdr_only() {
    let replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");
    let (projected, _) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    let checkout = projected
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .next()
        .expect("the remote checkout");

    assert_eq!(
        checkout
            .tabs
            .iter()
            .map(|tab| tab.id.clone().expect("a remote tab id"))
            .collect::<Vec<_>>(),
        vec![
            remote_tab_id("mini", "w1:t1"),
            remote_tab_id("mini", "w1:t2")
        ]
    );
    assert_eq!(
        checkout
            .strip
            .iter()
            .map(|entry| (entry.kind, entry.source_id.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                crate::model::StripTabKind::Herdr,
                remote_tab_id("mini", "w1:t1")
            ),
            (
                crate::model::StripTabKind::Herdr,
                remote_tab_id("mini", "w1:t2")
            )
        ]
    );
    assert_eq!(checkout.active_tab_id, Some(remote_tab_id("mini", "w1:t1")));
}

#[test]
fn tab_order_from_a_move_event_reaches_the_projection() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");
    assert_eq!(
        replica
            .project()
            .tabs
            .iter()
            .map(|tab| tab.tab_id.clone())
            .collect::<Vec<_>>(),
        vec!["w1:t1".to_owned(), "w1:t2".to_owned()]
    );

    // Herdr moved the second tab in front of the first and reported the
    // resulting order. The projection the navigator reads must carry that
    // order, not the order the tabs were created in.
    let moved = replica
            .apply(event("tab_moved",
                json!({
                    "type": "tab_moved",
                    "workspace_id": "w1",
                    "tab_id": "w1:t2",
                    "insert_index": 0,
                    "tabs": [
                        {"workspace_id": "w1", "tab_id": "w1:t2", "agent_status": "idle", "focused": false, "number": 2, "pane_count": 1, "label": "2"},
                        {"workspace_id": "w1", "tab_id": "w1:t1", "agent_status": "idle", "focused": false, "number": 1, "pane_count": 1, "label": "1"}
                    ]
                }),
            ), ApplyMode::Strict)
            .expect("tab move event");
    assert!(moved.publish);
    assert_eq!(
        replica
            .project()
            .tabs
            .iter()
            .map(|tab| tab.tab_id.clone())
            .collect::<Vec<_>>(),
        vec!["w1:t2".to_owned(), "w1:t1".to_owned()]
    );
}

#[test]
fn herdr_active_tab_rides_the_projection_with_its_workspace() {
    let replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");
    let projected = replica.project();
    let workspace = projected
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == "w1")
        .expect("the fixture workspace");
    assert_eq!(workspace.active_tab_id.as_deref(), Some("w1:t1"));
}

#[test]
fn active_tab_close_waits_for_the_authoritative_fallback_tab_focus() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");

    let tab_closed = replica
        .apply(
            event(
                "tab_closed",
                json!({
                    "type": "tab_closed",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("active tab close event");
    assert!(!tab_closed.publish);
    assert!(!replica.ready_to_publish());

    let tab_focused = replica
        .apply(
            event(
                "tab_focused",
                json!({
                    "type": "tab_focused",
                    "tab_id": "w1:t2",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("fallback tab focus event");
    assert!(tab_focused.publish);
    assert!(replica.ready_to_publish());
    assert_eq!(replica.state.workspaces[0].active_tab_id, "w1:t2");
    assert_remote_replacement_focus(&replica);
}

/// Herdr moves a non-focused workspace's active tab without an event when a
/// close removes it, so the replica settles from a `workspace.get` answer.
/// The projection then drops the closed tab and names the replacement.
#[test]
fn active_tab_close_settles_from_a_workspace_read_when_no_focus_follows() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");
    replica
        .apply(
            event(
                "tab_closed",
                json!({"type": "tab_closed", "tab_id": "w1:t1", "workspace_id": "w1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("active tab close event");
    assert_eq!(
        replica.workspaces_awaiting_active_tab(),
        vec!["w1".to_owned()]
    );

    // A read that ran ahead of the stream names a tab it has not delivered.
    assert!(!replica.settle_active_tab("w1", "w1:t9"));
    assert!(!replica.ready_to_publish());

    assert!(replica.settle_active_tab("w1", "w1:t2"));
    assert!(replica.ready_to_publish());
    assert!(replica.workspaces_awaiting_active_tab().is_empty());
    assert!(replica.refresh_published_state().expect("valid projection"));
    let projected = replica.project();
    assert!(projected.tabs.iter().all(|tab| tab.tab_id != "w1:t1"));
    assert_eq!(
        projected.workspaces[0].active_tab_id.as_deref(),
        Some("w1:t2")
    );
    assert_remote_replacement_focus(&replica);
}

#[test]
fn a_replacement_tab_keeps_its_selected_split_pane_after_either_close() {
    let mut value = two_tab_snapshot();
    let mut pane = value["panes"][1].clone();
    pane["pane_id"] = json!("w1:p3");
    value["panes"].as_array_mut().unwrap().push(pane);
    let layout = &mut value["layouts"][1];
    layout["focused_pane_id"] = json!("w1:p3");
    layout["panes"][0]["rect"]["width"] = json!(60);
    layout["panes"].as_array_mut().unwrap().push(json!({
        "pane_id": "w1:p3", "focused": true,
        "rect": {"x": 60, "y": 0, "width": 60, "height": 60}
    }));

    for close in [
        event(
            "tab_closed",
            json!({"type": "tab_closed", "tab_id": "w1:t1", "workspace_id": "w1"}),
        ),
        event(
            "pane_closed",
            json!({"type": "pane_closed", "pane_id": "w1:p1", "workspace_id": "w1"}),
        ),
    ] {
        for from_read in [false, true] {
            let mut replica = SessionReplica::from_snapshot(&value).expect("split replacement");
            let before = remote_focus(&replica);
            assert!(
                !replica
                    .apply(close.clone(), ApplyMode::Strict)
                    .unwrap()
                    .publish
            );
            assert_eq!(
                remote_focus(&replica),
                before,
                "hold selection until confirmed"
            );
            if from_read {
                assert!(replica.settle_active_tab("w1", "w1:t2"));
                assert!(replica.refresh_published_state().unwrap());
            } else {
                let focused = replica
                    .apply(
                        event(
                            "tab_focused",
                            json!({"type": "tab_focused", "tab_id": "w1:t2", "workspace_id": "w1"}),
                        ),
                        ApplyMode::Strict,
                    )
                    .unwrap();
                assert!(focused.publish);
            }
            assert_eq!(
                remote_focus(&replica),
                (
                    Some("remote:mini:checkout:w1".to_owned()),
                    Some("remote:mini:tab:w1:t2".to_owned()),
                    Some("remote:mini:pane:w1:p3".to_owned()),
                )
            );
            assert!(
                replica
                    .project()
                    .tabs
                    .iter()
                    .all(|tab| tab.tab_id != "w1:t1")
            );
        }
    }
}

#[test]
fn a_background_layout_cannot_take_the_closed_front_panes_focus() {
    let mut value = two_tab_snapshot();
    let mut background: Value =
        serde_json::from_str(&snapshot().to_string().replace("w1", "w2")).unwrap();
    background["workspaces"][0]["focused"] = json!(false);
    for section in ["workspaces", "tabs", "panes", "layouts"] {
        value[section]
            .as_array_mut()
            .unwrap()
            .extend(background[section].as_array().unwrap().clone());
    }
    let mut replica = SessionReplica::from_snapshot(&value).expect("two workspaces");
    let before = remote_focus(&replica);
    replica
        .apply(
            event(
                "pane_closed",
                json!({"type": "pane_closed", "pane_id": "w1:p1", "workspace_id": "w1"}),
            ),
            ApplyMode::Strict,
        )
        .unwrap();
    replica
        .apply(
            event(
                "layout_updated",
                json!({"type": "layout_updated", "layout": background["layouts"][0]}),
            ),
            ApplyMode::Strict,
        )
        .unwrap();
    assert_eq!(remote_focus(&replica), before);
    assert!(replica.settle_active_tab("w1", "w1:t2"));
    assert!(replica.refresh_published_state().unwrap());
    assert_remote_replacement_focus(&replica);
}

#[test]
fn a_background_tab_close_keeps_the_front_checkout_and_pane() {
    let mut value = snapshot();
    value["focused_workspace_id"] = json!("w1");
    let mut background: Value =
        serde_json::from_str(&two_tab_snapshot().to_string().replace("w1", "w2")).unwrap();
    background["workspaces"][0]["focused"] = json!(false);
    for section in ["workspaces", "tabs", "panes", "layouts"] {
        value[section]
            .as_array_mut()
            .unwrap()
            .extend(background[section].as_array().unwrap().clone());
    }
    let mut replica = SessionReplica::from_snapshot(&value).expect("background two-tab workspace");
    let before = remote_focus(&replica);
    replica
        .apply(
            event(
                "tab_closed",
                json!({"type": "tab_closed", "tab_id": "w2:t1", "workspace_id": "w2"}),
            ),
            ApplyMode::Strict,
        )
        .unwrap();
    assert!(replica.settle_active_tab("w2", "w2:t2"));
    assert!(replica.refresh_published_state().unwrap());
    assert_eq!(remote_focus(&replica), before);
    let projected = replica.project();
    assert_eq!(
        projected
            .workspaces
            .iter()
            .find(|w| w.workspace_id == "w2")
            .unwrap()
            .active_tab_id
            .as_deref(),
        Some("w2:t2")
    );
    assert!(projected.tabs.iter().all(|tab| tab.tab_id != "w2:t1"));
}

/// The focused workspace still gets its replacement from `tab_focused`; a
/// read that lands after it changes nothing.
#[test]
fn a_workspace_read_after_the_focus_event_changes_nothing() {
    let mut replica = SessionReplica::from_snapshot(&two_tab_snapshot()).expect("two-tab snapshot");
    replica
        .apply(
            event(
                "tab_closed",
                json!({"type": "tab_closed", "tab_id": "w1:t1", "workspace_id": "w1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("active tab close event");
    replica
        .apply(
            event(
                "tab_focused",
                json!({"type": "tab_focused", "tab_id": "w1:t2", "workspace_id": "w1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("fallback tab focus event");
    assert!(replica.workspaces_awaiting_active_tab().is_empty());
    assert!(!replica.settle_active_tab("w1", "w1:t2"));
    assert_eq!(replica.state.workspaces[0].active_tab_id, "w1:t2");
}

#[test]
fn last_tab_close_waits_for_the_workspace_close_cascade() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let tab_closed = replica
        .apply(
            event(
                "tab_closed",
                json!({
                    "type": "tab_closed",
                    "tab_id": "w1:t1",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("tab close event");
    assert!(!tab_closed.publish);

    let workspace_closed = replica
        .apply(
            event(
                "workspace_closed",
                json!({
                    "type": "workspace_closed",
                    "workspace_id": "w1"
                }),
            ),
            ApplyMode::Strict,
        )
        .expect("workspace close event");
    assert!(workspace_closed.publish);
    assert!(replica.ready_to_publish());
    assert!(replica.project().workspaces.is_empty());
    assert_eq!(remote_focus(&replica), (None, None, None));
}

/// Herdr 0.9.1 announces a workspace that closes with its last tab or pane
/// before the tab or pane itself; the trailing event names what the cascade
/// already removed and must not rebuild the replica.
#[test]
fn a_close_event_trailing_its_workspace_close_is_already_applied() {
    for (trailing, body) in [
        (
            "tab_closed",
            json!({"type": "tab_closed", "tab_id": "w1:t1", "workspace_id": "w1"}),
        ),
        (
            "pane_closed",
            json!({"type": "pane_closed", "pane_id": "w1:p1", "workspace_id": "w1"}),
        ),
    ] {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let workspace_closed = replica
            .apply(
                event(
                    "workspace_closed",
                    json!({"type": "workspace_closed", "workspace_id": "w1"}),
                ),
                ApplyMode::Strict,
            )
            .expect("workspace close event");
        assert!(workspace_closed.publish);

        let trailing = replica
            .apply(event(trailing, body), ApplyMode::Strict)
            .unwrap_or_else(|error| panic!("{trailing} after workspace_closed: {error:?}"));
        assert!(!trailing.publish);
        assert!(replica.ready_to_publish());
        assert!(replica.project().workspaces.is_empty());
    }

    // A close for a workspace no event closed is still a divergence.
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let error = replica
        .apply(
            event(
                "tab_closed",
                json!({"type": "tab_closed", "tab_id": "w9:t1", "workspace_id": "w9"}),
            ),
            ApplyMode::Strict,
        )
        .expect_err("unknown tab");
    assert_eq!(error.state(), "malformed");
}

/// Herdr 0.9.1 emits a workspace's creation events after the call that
/// created it returns, with the focus of that moment, so a tab created with
/// focus right after its workspace is announced as focused before it is
/// announced at all. The focus waits for its tab and pane.
#[test]
fn a_focus_announced_before_its_tab_and_pane_applies_when_they_arrive() {
    let created = two_tab_snapshot();
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let events = [
        event(
            "tab_focused",
            json!({"type": "tab_focused", "workspace_id": "w1", "tab_id": "w1:t2"}),
        ),
        event(
            "pane_focused",
            json!({"type": "pane_focused", "workspace_id": "w1", "pane_id": "w1:p2"}),
        ),
        event(
            "tab_created",
            json!({"type": "tab_created", "tab": created["tabs"][1]}),
        ),
        event(
            "pane_created",
            json!({"type": "pane_created", "workspace_id": "w1", "tab_id": "w1:t2", "pane": created["panes"][1]}),
        ),
        event(
            "layout_updated",
            json!({"type": "layout_updated", "layout": created["layouts"][1]}),
        ),
    ];
    for (index, next) in events.into_iter().enumerate() {
        replica
            .apply(next, ApplyMode::Strict)
            .unwrap_or_else(|error| panic!("event {index}: {error:?}"));
    }
    assert!(replica.ready_to_publish());
    let projected = replica.project();
    assert_eq!(
        projected.workspaces[0].active_tab_id.as_deref(),
        Some("w1:t2")
    );
    assert_eq!(projected.focused_pane_id.as_deref(), Some("w1:p2"));

    // A focus in a workspace Herdr never announced is still a divergence.
    let error = replica
        .apply(
            event(
                "tab_focused",
                json!({"type": "tab_focused", "workspace_id": "w9", "tab_id": "w9:t1"}),
            ),
            ApplyMode::Strict,
        )
        .expect_err("unknown workspace");
    assert_eq!(error.state(), "malformed");
}

/// A tab's pane and layout can reach the stream before its `tab_created`
/// (CI, issues 754 and 756). Both wait for the tab, in either mode: dropped,
/// they left the tab waiting for a layout that had already come, and its
/// workspace never published another change.
#[test]
fn a_pane_and_layout_announced_before_their_tab_apply_when_it_arrives() {
    let created = two_tab_snapshot();
    for mode in [ApplyMode::Reconcile, ApplyMode::Strict] {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let events = [
            event(
                "pane_created",
                json!({"type": "pane_created", "pane": created["panes"][1]}),
            ),
            event(
                "layout_updated",
                json!({"type": "layout_updated", "layout": created["layouts"][1]}),
            ),
            event(
                "tab_created",
                json!({"type": "tab_created", "tab": created["tabs"][1]}),
            ),
        ];
        for (index, next) in events.into_iter().enumerate() {
            replica
                .apply(next, mode)
                .unwrap_or_else(|error| panic!("{mode:?} event {index}: {error:?}"));
        }
        assert!(replica.ready_to_publish(), "{mode:?}");
        let projected = replica.project();
        let tabs: Vec<_> = projected
            .tabs
            .iter()
            .map(|tab| tab.tab_id.as_str())
            .collect();
        assert_eq!(tabs, ["w1:t1", "w1:t2"], "{mode:?}");
        assert!(
            projected.panes.iter().any(|pane| pane.pane_id == "w1:p2"),
            "{mode:?}"
        );
        assert!(
            projected
                .layouts
                .iter()
                .any(|layout| layout.tab_id == "w1:t2"),
            "{mode:?}"
        );
    }
}

/// A held focus is Herdr's focus of an earlier moment; a focus Herdr applies
/// after it wins, so the held one never moves focus back when its pane lands.
#[test]
fn a_focus_applied_after_a_held_one_supersedes_it() {
    let created = two_tab_snapshot();
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let events = [
        event(
            "pane_focused",
            json!({"type": "pane_focused", "workspace_id": "w1", "pane_id": "w1:p2"}),
        ),
        event(
            "pane_focused",
            json!({"type": "pane_focused", "workspace_id": "w1", "pane_id": "w1:p1"}),
        ),
        event(
            "tab_created",
            json!({"type": "tab_created", "tab": created["tabs"][1]}),
        ),
        event(
            "pane_created",
            json!({"type": "pane_created", "workspace_id": "w1", "tab_id": "w1:t2", "pane": created["panes"][1]}),
        ),
        event(
            "layout_updated",
            json!({"type": "layout_updated", "layout": created["layouts"][1]}),
        ),
    ];
    for (index, next) in events.into_iter().enumerate() {
        replica
            .apply(next, ApplyMode::Strict)
            .unwrap_or_else(|error| panic!("event {index}: {error:?}"));
    }
    assert!(replica.ready_to_publish());
    assert_eq!(replica.project().focused_pane_id.as_deref(), Some("w1:p1"));
}

/// The event order Herdr 0.9.1 emits when Hide opens a workspace and
/// `layout.apply` replaces its seed tab (the Reopen of a tab whose workspace
/// closed with it): the seed tab is announced, closed, then focused, given a
/// pane and laid out, and only then does the restored tab arrive.
#[test]
fn a_seed_tab_announced_after_its_close_leaves_the_restored_tab_in_front() {
    let restored: Value =
        serde_json::from_str(&two_tab_snapshot().to_string().replace("w1", "w2")).unwrap();
    let mut workspace = restored["workspaces"][0].clone();
    workspace["focused"] = json!(true);
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let events = [
        event(
            "workspace_created",
            json!({"type": "workspace_created", "workspace": workspace}),
        ),
        event(
            "workspace_focused",
            json!({"type": "workspace_focused", "workspace_id": "w2"}),
        ),
        event(
            "tab_created",
            json!({"type": "tab_created", "tab": restored["tabs"][0]}),
        ),
        event(
            "tab_closed",
            json!({"type": "tab_closed", "workspace_id": "w2", "tab_id": "w2:t1"}),
        ),
        event(
            "tab_focused",
            json!({"type": "tab_focused", "workspace_id": "w2", "tab_id": "w2:t1"}),
        ),
        event(
            "pane_created",
            json!({"type": "pane_created", "workspace_id": "w2", "tab_id": "w2:t1", "pane": restored["panes"][0]}),
        ),
        event(
            "pane_focused",
            json!({"type": "pane_focused", "workspace_id": "w2", "pane_id": "w2:p1"}),
        ),
        event(
            "layout_updated",
            json!({"type": "layout_updated", "layout": restored["layouts"][0]}),
        ),
        event(
            "tab_created",
            json!({"type": "tab_created", "tab": restored["tabs"][1]}),
        ),
        event(
            "pane_created",
            json!({"type": "pane_created", "workspace_id": "w2", "tab_id": "w2:t2", "pane": restored["panes"][1]}),
        ),
        event(
            "layout_updated",
            json!({"type": "layout_updated", "layout": restored["layouts"][1]}),
        ),
    ];
    for (index, next) in events.into_iter().enumerate() {
        replica
            .apply(next, ApplyMode::Strict)
            .unwrap_or_else(|error| panic!("event {index}: {error:?}"));
    }
    // No event names the restored tab as active, so the coordinator reads it.
    assert_eq!(
        replica.workspaces_awaiting_active_tab(),
        vec!["w2".to_owned()]
    );
    assert!(replica.settle_active_tab("w2", "w2:t2"));
    assert!(replica.ready_to_publish());
    assert!(replica.refresh_published_state().expect("publishes"));
    let projected = replica.project();
    let tabs = projected
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == "w2")
        .map(|tab| tab.tab_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tabs, vec!["w2:t2"]);
    let restored_workspace = projected
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == "w2")
        .expect("the reopened workspace is published");
    assert_eq!(restored_workspace.active_tab_id.as_deref(), Some("w2:t2"));
}

/// The subscription is opened before the snapshot, so an event emitted just
/// before the snapshot was taken arrives as well and describes a change the
/// snapshot already holds. In the reconcile window the snapshot wins and the
/// event is dropped; the same event afterwards is a real divergence.
#[test]
fn an_event_the_snapshot_already_holds_is_dropped_while_reconciling_and_rejected_afterwards() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let before = replica.project().panes.len();
    let already_listed = event(
        "pane_created",
        json!({
            "type": "pane_created",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "pane": snapshot()["panes"][0]
        }),
    );
    let reconciled = replica
        .apply(already_listed.clone(), ApplyMode::Reconcile)
        .expect("the snapshot already holds this pane");
    assert!(!reconciled.publish);
    assert_eq!(replica.project().panes.len(), before);
    assert_eq!(replica.applied_events, 0);

    let error = replica
        .apply(already_listed, ApplyMode::Strict)
        .expect_err("after the window the replica has diverged");
    assert_eq!(error.state(), "malformed");
    assert_eq!(replica.project().panes.len(), before);

    // An event that does not contradict the snapshot applies in either mode.
    let focused = event(
        "pane_focused",
        json!({"type": "pane_focused", "workspace_id": "w1", "pane_id": "w1:p1"}),
    );
    assert!(
        replica
            .apply(focused, ApplyMode::Reconcile)
            .expect("a focus the snapshot agrees with")
            .publish
    );
    assert_eq!(replica.applied_events, 1);
}

#[test]
fn a_stream_error_is_an_explicit_stream_result() {
    let line = json!({
        "id": "herdr-core:events.subscribe",
        "error": {
            "code": "internal",
            "message": "event stream closed"
        }
    })
    .to_string();
    match parse_subscription_line(&line).expect("typed error") {
        SubscriptionLine::Error { code, message } => {
            assert_eq!(code, "internal");
            assert!(message.contains("closed"));
        }
        SubscriptionLine::Event(_) | SubscriptionLine::EventsLost { .. } => {
            panic!("expected subscription error")
        }
    }
}

#[test]
fn subscription_failure_is_stale_only_when_a_projection_already_exists() {
    let initial = connect_failure_from_api(ApiError::Transport("socket closed".to_owned()), false);
    assert_eq!(initial.state(), "unreachable");

    let reconnect = connect_failure_from_api(ApiError::Transport("socket closed".to_owned()), true);
    assert_eq!(reconnect.state(), "stale");
}

fn agent(pane_id: &str, elapsed: &str) -> ProjectedAgent {
    wire::agents_response(json!({"type": "agent_list", "agents": [{
        "pane_id": pane_id, "workspace_id": "w1", "tab_id": "w1:t1",
        "agent": "claude", "agent_status": "idle", "focused": false,
        "terminal_id": "fixture-terminal", "revision": 0,
        "tokens": {"status_idle": "\u{25cb}", "elapsed": elapsed, "activity": "1"}
    }]}))
    .expect("an agent list")
    .remove(0)
}

fn fresh_catalog() -> CatalogCache {
    CatalogCache {
        registrations: Vec::new(),
        spaces: Vec::new(),
        worktrees: crate::model::WorktreeCatalogSnapshot::default(),
        workspaces: Vec::new(),
        paths: Default::default(),
        built_at: Instant::now(),
        purposes_synced: None,
    }
}

#[test]
fn agent_refresh_reconciles_a_plain_new_panes_live_checkout_cwd() {
    let mut value = two_tab_snapshot();
    value["panes"][1]["cwd"] = json!("/tmp");
    let mut replica = SessionReplica::from_snapshot(&value).expect("snapshot");
    let mut plain = agent("w1:p2", "");
    plain.tab_id = "w1:t2".to_owned();
    plain.agent = None;
    plain.cwd = Some("/tmp/fixture".to_owned());

    replica.replace_agents(vec![plain]);
    replica.refresh_published_state().expect("publish live cwd");
    let projected = replica.project();
    assert_eq!(
        Runtime::session_spaces(&projected, &workspace::PathIndex::NONE)[0].cwds,
        vec!["/tmp/fixture".to_owned()],
        "the shell's live directory, not its inherited birth cwd, owns both tabs"
    );
    assert_eq!(projected.tabs.len(), 2);
    assert_eq!(projected.panes[1].cwd.as_deref(), Some("/tmp/fixture"));
}

#[test]
fn agent_refresh_does_not_move_a_pane_or_clear_its_known_cwd() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let mut other_scope = agent("w1:p1", "");
    other_scope.tab_id = "w1:t2".to_owned();
    other_scope.cwd = Some("/tmp/elsewhere".to_owned());
    replica.replace_agents(vec![other_scope]);
    assert_eq!(replica.state.panes[0].cwd.as_deref(), Some("/tmp/fixture"));
    assert_eq!(replica.state.panes[0].tab_id, "w1:t1");

    replica.replace_agents(vec![agent("w1:p1", "")]);
    assert_eq!(replica.state.panes[0].cwd.as_deref(), Some("/tmp/fixture"));
}

/// R6, AC12, SC5. `agent.list` is polled once a second whether or not it
/// moved, and every tick used to rebuild the whole projection and re-enter
/// the runtime lock twice for a sidebar that had not changed.
#[test]
fn snapshot_delivery_skips_the_projection_when_the_agent_list_is_unchanged() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let held = vec![agent("w1:p1", "4m")];
    replica.replace_agents(held.clone());
    let catalog = fresh_catalog();

    assert!(
        !agent_tick_needs_publish(&replica, &held, Some(&catalog)),
        "an identical list projects the same sidebar, so the tick publishes nothing"
    );
    assert!(
        agent_tick_needs_publish(&replica, &[agent("w1:p1", "5m")], Some(&catalog)),
        "a token the sidebar renders moved, so the projection has to be rebuilt"
    );
    assert!(
        agent_tick_needs_publish(&replica, &[], Some(&catalog)),
        "an agent that went away has to leave the sidebar"
    );

    // The catalog is rebuilt inside the publish on its own refresh window,
    // and on an idle session this tick is the only thing that calls it, so
    // the skip must not be what freezes every branch mark in the navigator.
    assert!(
        agent_tick_needs_publish(&replica, &held, None),
        "a catalog that was never built has to be built"
    );
    let stale = CatalogCache {
        built_at: Instant::now() - CATALOG_REFRESH_INTERVAL,
        ..fresh_catalog()
    };
    assert!(
        agent_tick_needs_publish(&replica, &held, Some(&stale)),
        "a catalog past its refresh window has to be rebuilt"
    );
}

/// Herdr's focused workspace is carried from the snapshot and moved by
/// every focus event, because a checkout can hold tabs from several
/// workspaces and only the focused one's active tab is a focus.
#[test]
fn focus_events_move_the_projected_focused_workspace() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    assert_eq!(
        replica.project().focused_workspace_id,
        None,
        "the fixture snapshot names no focused workspace"
    );

    replica
        .apply(
            event(
                "workspace_focused",
                json!({"type": "workspace_focused", "workspace_id": "w1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("workspace focus applies");
    assert_eq!(
        replica.project().focused_workspace_id.as_deref(),
        Some("w1")
    );

    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    replica
        .apply(
            event(
                "tab_focused",
                json!({"type": "tab_focused", "workspace_id": "w1", "tab_id": "w1:t1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("tab focus applies");
    assert_eq!(
        replica.project().focused_workspace_id.as_deref(),
        Some("w1"),
        "a tab focus names the workspace Herdr moved into"
    );

    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    replica
        .apply(
            event(
                "pane_focused",
                json!({"type": "pane_focused", "workspace_id": "w1", "pane_id": "w1:p1"}),
            ),
            ApplyMode::Strict,
        )
        .expect("pane focus applies");
    assert_eq!(
        replica.project().focused_workspace_id.as_deref(),
        Some("w1"),
        "a pane focus names the workspace the pane is in"
    );
}

/// Herdr's stream has no position to resume from, so a clean disconnect is
/// followed by a fresh subscription and a fresh snapshot, in that order:
/// listening first is what keeps an event between the two from being lost.
#[test]
fn coordinator_rebuilds_from_a_fresh_snapshot_after_a_clean_disconnect() {
    let folder = tempfile::Builder::new()
        .prefix("herdr-core-session-rebuild-contract-")
        .tempdir_in(socket_parent())
        .expect("create socket directory");
    let root = folder.path().to_path_buf();
    let socket_path = root.join("herdr.sock");
    let state_path = root.join("state.json");
    let listener = LocalListener::bind(&socket_path).expect("bind fake Herdr socket");
    let rebuilt = Arc::new(AtomicBool::new(false));
    let rebuilt_from_server = Arc::clone(&rebuilt);
    let server = thread::spawn(move || {
        let (mut first_subscription, first_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        assert_eq!(
            first_subscribe_request["params"].get("after_sequence"),
            None,
            "the stable contract has no resume cursor"
        );
        write_result(
            &mut first_subscription,
            &first_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut snapshot_stream, snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut snapshot_stream,
            &snapshot_request,
            json!({"type": "session_snapshot", "snapshot": snapshot()}),
        );
        writeln!(
            first_subscription,
            "{}",
            json!({
                "event": "workspace_focused",
                "data": {"type": "workspace_focused", "workspace_id": "w1"}
            })
        )
        .expect("write event");
        drop(first_subscription);

        let (mut second_subscription, second_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut second_subscription,
            &second_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut second_snapshot_stream, second_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut second_snapshot_stream,
            &second_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": snapshot()}),
        );
        rebuilt_from_server.store(true, Ordering::Release);
        let mut byte = [0_u8; 1];
        assert_eq!(
            second_subscription
                .read(&mut byte)
                .expect("wait for shutdown"),
            0
        );
    });

    let runtime = runtime_for_fixture(&socket_path, &state_path);
    let context = context_for_fixture(&runtime, &socket_path);
    let handle = spawn(context, None).expect("start session sync");
    wait_until(Instant::now() + Duration::from_secs(3), || {
        rebuilt.load(Ordering::Acquire)
            && runtime
                .lock()
                .expect("runtime lock")
                .snapshot()
                .status
                .herdr
                .state
                == "connected"
    });

    drop(handle);
    server.join().expect("fake server joins");
    remove_fixture(folder, runtime);
}

/// End to end through the coordinator: Herdr reports the active tab closed
/// and nothing else, the coordinator asks `workspace.get`, and the runtime's
/// navigator stops listing the closed tab without waiting for a deadline.
#[test]
fn coordinator_reads_the_replacement_active_tab_when_herdr_names_none() {
    let folder = tempfile::Builder::new()
        .prefix("herdr-core-session-active-tab-read-")
        .tempdir_in(socket_parent())
        .expect("create socket directory");
    let root = folder.path().to_path_buf();
    let socket_path = root.join("herdr.sock");
    let state_path = root.join("state.json");
    let listener = LocalListener::bind(&socket_path).expect("bind fake Herdr socket");
    let read_answered = Arc::new(AtomicBool::new(false));
    let read_answered_from_server = Arc::clone(&read_answered);
    let server = thread::spawn(move || {
        let (mut subscription, subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut subscription,
            &subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut snapshot_stream, snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut snapshot_stream,
            &snapshot_request,
            json!({"type": "session_snapshot", "snapshot": two_tab_snapshot()}),
        );
        writeln!(
            subscription,
            "{}",
            json!({
                "event": "tab_closed",
                "data": {"type": "tab_closed", "tab_id": "w1:t1", "workspace_id": "w1"}
            })
        )
        .expect("write tab_closed");

        let (mut read_stream, read_request) = accept_request_for(&listener, "workspace.get");
        assert_eq!(read_request["params"]["workspace_id"], "w1");
        write_result(
            &mut read_stream,
            &read_request,
            json!({
                "type": "workspace_info",
                "workspace": {
                    "workspace_id": "w1", "number": 1, "label": "fixture", "focused": false,
                    "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t2",
                    "agent_status": "idle"
                }
            }),
        );
        read_answered_from_server.store(true, Ordering::Release);
        let mut byte = [0_u8; 1];
        assert_eq!(subscription.read(&mut byte).expect("wait for shutdown"), 0);
    });

    let runtime = runtime_for_fixture(&socket_path, &state_path);
    let context = context_for_fixture(&runtime, &socket_path);
    let handle = spawn(context, None).expect("start session sync");
    let listed_tab_ids = || -> Vec<String> {
        runtime
            .lock()
            .expect("runtime lock")
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .filter_map(|tab| tab.id.clone())
            .collect()
    };
    wait_until(Instant::now() + Duration::from_secs(3), || {
        let tabs = listed_tab_ids();
        read_answered.load(Ordering::Acquire)
            && tabs.iter().any(|id| id == "w1:t2")
            && tabs.iter().all(|id| id != "w1:t1")
    });

    drop(handle);
    server.join().expect("fake server joins");
    remove_fixture(folder, runtime);
}

#[test]
fn coordinator_recovers_a_stream_error_with_one_fresh_snapshot_and_stops_its_reader() {
    let folder = tempfile::Builder::new()
        .prefix("herdr-core-session-stream-error-contract-")
        .tempdir_in(socket_parent())
        .expect("create socket directory");
    let root = folder.path().to_path_buf();
    let socket_path = root.join("herdr.sock");
    let state_path = root.join("state.json");
    let listener = LocalListener::bind(&socket_path).expect("bind fake Herdr socket");
    let server = thread::spawn(move || {
        let (mut first_subscription, first_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut first_subscription,
            &first_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut first_snapshot_stream, first_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut first_snapshot_stream,
            &first_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": snapshot()}),
        );
        writeln!(
            first_subscription,
            "{}",
            json!({
                "id": "herdr-core:events.subscribe",
                "error": {
                    "code": "internal",
                    "message": "event stream closed"
                }
            })
        )
        .expect("write stream error");
        drop(first_subscription);

        let (mut final_subscription, final_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut final_subscription,
            &final_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut second_snapshot_stream, second_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        let mut recovered = snapshot();
        // A project is named after its directory, not Herdr's workspace
        // label, so the recovery marker is a pane in a new directory.
        recovered["panes"][0]["cwd"] = json!("/tmp/fixture-recovered");
        write_result(
            &mut second_snapshot_stream,
            &second_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": recovered}),
        );
        let mut byte = [0_u8; 1];
        assert_eq!(
            final_subscription
                .read(&mut byte)
                .expect("wait for shutdown"),
            0
        );
    });

    let runtime = runtime_for_fixture(&socket_path, &state_path);
    let context = context_for_fixture(&runtime, &socket_path);
    let handle = spawn(context, None).expect("start session sync");
    wait_until(Instant::now() + Duration::from_secs(3), || {
        let snapshot = runtime.lock().expect("runtime lock").snapshot().clone();
        snapshot.status.herdr.state == "connected"
            && snapshot
                .navigator
                .workspaces
                .iter()
                .any(|workspace| workspace.label == "fixture-recovered")
    });

    drop(handle);
    server.join().expect("fake server joins");
    remove_fixture(folder, runtime);
}

// Herdr's Socket API: a subscriber that falls behind gets `events_lost` and
// its connection closed; the client resubscribes and replaces its cache with
// a fresh snapshot. Herdr is still answering, so that is not a failure the
// operator sees while the resync is in flight.
#[test]
fn coordinator_resyncs_after_events_lost_without_reporting_herdr_stale() {
    let folder = tempfile::Builder::new()
        .prefix("herdr-core-session-events-lost-")
        .tempdir_in(socket_parent())
        .expect("create socket directory");
    let root = folder.path().to_path_buf();
    let socket_path = root.join("herdr.sock");
    let state_path = root.join("state.json");
    let listener = LocalListener::bind(&socket_path).expect("bind fake Herdr socket");
    let (resync_asked, resync_seen) = std::sync::mpsc::channel::<()>();
    let (answer_resync, resync_answered) = std::sync::mpsc::channel::<()>();
    let (initial_ready, initial_seen) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (mut first_subscription, first_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut first_subscription,
            &first_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut first_snapshot_stream, first_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut first_snapshot_stream,
            &first_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": snapshot()}),
        );
        // This contract is retention during resync, so establish an accepted
        // initial catalog before disconnecting. Path facts now settle on a
        // worker after the session snapshot itself is applied.
        initial_seen
            .recv_timeout(Duration::from_secs(30))
            .expect("initial catalog accepted");
        writeln!(
            first_subscription,
            "{}",
            json!({
                "id": first_subscribe_request["id"],
                "error": {
                    "code": "events_lost",
                    "message": "subscriber fell behind retained event history"
                }
            })
        )
        .expect("write events_lost");
        drop(first_subscription);

        let (mut final_subscription, final_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut final_subscription,
            &final_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut second_snapshot_stream, second_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        // The coordinator handled the lost stream before it asked again, so
        // whatever it published for it is already in the runtime.
        resync_asked.send(()).expect("report the resync request");
        resync_answered
            .recv()
            .expect("the test releases the resync snapshot");
        let mut recovered = snapshot();
        recovered["panes"][0]["cwd"] = json!("/tmp/fixture-recovered");
        write_result(
            &mut second_snapshot_stream,
            &second_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": recovered}),
        );
        let mut byte = [0_u8; 1];
        assert_eq!(
            final_subscription
                .read(&mut byte)
                .expect("wait for shutdown"),
            0
        );
    });

    let runtime = runtime_for_fixture(&socket_path, &state_path);
    let context = context_for_fixture(&runtime, &socket_path);
    let handle = spawn(context, None).expect("start session sync");
    wait_until(Instant::now() + Duration::from_secs(30), || {
        runtime
            .lock()
            .unwrap()
            .snapshot()
            .navigator
            .workspaces
            .iter()
            .any(|workspace| workspace.label == "fixture")
    });
    initial_ready
        .send(())
        .expect("allow events_lost after initial catalog");
    resync_seen
        .recv_timeout(Duration::from_secs(10))
        .expect("events_lost leads to a fresh subscription and snapshot");
    {
        let snapshot = runtime.lock().expect("runtime lock").snapshot().clone();
        assert_eq!(snapshot.status.herdr.state, "connected");
        assert!(
            snapshot
                .navigator
                .workspaces
                .iter()
                .any(|workspace| workspace.label == "fixture"),
            "the last projection stays on screen during the resync"
        );
    }
    answer_resync.send(()).expect("release the resync snapshot");
    wait_until(Instant::now() + Duration::from_secs(3), || {
        let snapshot = runtime.lock().expect("runtime lock").snapshot().clone();
        snapshot.status.herdr.state == "connected"
            && snapshot
                .navigator
                .workspaces
                .iter()
                .any(|workspace| workspace.label == "fixture-recovered")
    });

    drop(handle);
    server.join().expect("fake server joins");
    remove_fixture(folder, runtime);
}

/// Registrations ended on what Herdr's own session reads prove: a pane missing
/// from the fresh snapshot of a connect, never one missing while the stream
/// that lost events is being replaced.
#[test]
fn registrations_end_only_on_a_fresh_snapshot_never_during_the_resync() {
    let folder = tempfile::Builder::new()
        .prefix("herdr-core-session-gone-")
        .tempdir_in(socket_parent())
        .expect("create socket directory");
    let root = folder.path().to_path_buf();
    let socket_path = root.join("herdr.sock");
    let state_path = root.join("state.json");
    let listener = LocalListener::bind(&socket_path).expect("bind fake Herdr socket");
    let (resync_asked, resync_seen) = std::sync::mpsc::channel::<()>();
    let (answer_resync, resync_answered) = std::sync::mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (mut first_subscription, first_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut first_subscription,
            &first_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut first_snapshot_stream, first_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        write_result(
            &mut first_snapshot_stream,
            &first_snapshot_request,
            json!({"type": "session_snapshot", "snapshot": snapshot()}),
        );
        writeln!(
            first_subscription,
            "{}",
            json!({
                "id": first_subscribe_request["id"],
                "error": {"code": "events_lost", "message": "subscriber fell behind"}
            })
        )
        .expect("write events_lost");
        drop(first_subscription);
        let (mut final_subscription, final_subscribe_request) =
            accept_request_for(&listener, "events.subscribe");
        write_result(
            &mut final_subscription,
            &final_subscribe_request,
            json!({"type": "subscription_started"}),
        );
        let (mut second_snapshot_stream, second_snapshot_request) =
            accept_request_for(&listener, "session.snapshot");
        resync_asked.send(()).expect("report the resync request");
        resync_answered
            .recv()
            .expect("the test releases the resync snapshot");
        // The pane closed while the stream was lost; another took its place.
        let replaced = snapshot().to_string().replace("\"w1:p1\"", "\"w1:p3\"");
        write_result(
            &mut second_snapshot_stream,
            &second_snapshot_request,
            json!({"type": "session_snapshot",
                "snapshot": serde_json::from_str::<Value>(&replaced).unwrap()}),
        );
        let mut byte = [0_u8; 1];
        assert_eq!(
            final_subscription
                .read(&mut byte)
                .expect("wait for shutdown"),
            0
        );
    });

    let runtime = runtime_for_fixture(&socket_path, &state_path);
    {
        let mut guard = runtime.lock().expect("runtime lock");
        let node = guard.node().to_string();
        let scope = socket_path.to_str().unwrap().to_owned();
        let record = |id: &str, pane: &str, scope: &str| crate::coordination::AgentRecord {
            id: id.into(),
            name: Some(id.into()),
            machine: node.clone(),
            host_scope: scope.into(),
            native_machine: "fixture-machine".into(),
            session: format!("{id}-native"),
            instance: format!("terminal-{id}"),
            pane: pane.into(),
            parent: None,
            origin: None,
            project: None,
            actor: crate::delivery::Actor {
                pane_id: pane.into(),
                name: id.into(),
                kind: "claude".into(),
                device_id: node.clone(),
                session: crate::wire::session_digest(&format!("{id}-native")),
            },
            ended: false,
            forked: false,
        };
        let ledger = crate::delivery::ledger::Ledger {
            next_id: 4,
            agents: vec![
                record("agent-1", "w1:p1", &scope),
                // Its pane closed while no Hide was reading this Herdr.
                record("agent-2", "w1:p9", &scope),
                // Another Herdr's pane of the same name.
                record("agent-3", "w1:p9", "/elsewhere/herdr.sock"),
            ],
            ..Default::default()
        };
        ledger.validate().expect("valid ledger");
        guard.publish_delivery(Arc::new(ledger), false);
    }
    let gone = || {
        runtime
            .lock()
            .expect("runtime lock")
            .delivery_registrations_gone()
            .into_iter()
            .map(|(id, reason)| (id, reason.reason()))
            .collect::<Vec<_>>()
    };
    let context = context_for_fixture(&runtime, &socket_path);
    let handle = spawn(context, None).expect("start session sync");
    resync_seen
        .recv_timeout(Duration::from_secs(10))
        .expect("events_lost leads to a fresh subscription and snapshot");
    // The first snapshot lacked only agent-2's pane. While the lost stream
    // is replaced nothing is read, so agent-1 is not ended.
    assert_eq!(gone(), [("agent-2".to_owned(), "pane_absent")]);
    answer_resync.send(()).expect("release the resync snapshot");
    wait_until(Instant::now() + Duration::from_secs(3), || {
        gone().len() == 2
    });
    assert_eq!(
        gone(),
        [
            ("agent-1".to_owned(), "pane_absent"),
            ("agent-2".to_owned(), "pane_absent")
        ]
    );

    drop(handle);
    server.join().expect("fake server joins");
    remove_fixture(folder, runtime);
}

/// S6 B21: a device's declared parent is one of its own panes, so it is
/// scoped to the device like the pane ids the lineage joins it with.
#[test]
fn a_device_agents_declared_parent_is_scoped_to_the_device() {
    let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
    let mut child = agent("w1:p1", "1m");
    child.spawned_from_pane_id = Some("w1:p0".to_owned());
    child.tokens.remove("activity");
    replica.replace_agents(vec![child]);
    replica.refresh_published_state().expect("published");
    let (remote, excluded) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    assert!(excluded.is_empty(), "{excluded:?}");
    let row = remote
        .agents
        .iter()
        .find(|row| row.pane_id == "remote:mini:pane:w1:p1")
        .expect("the device agent");
    assert_eq!(
        row.spawned_from_pane_id.as_deref(),
        Some("remote:mini:pane:w1:p0")
    );
    assert_eq!(row.declared_parent_pane_id.as_deref(), Some("w1:p0"));

    let mut cross_device = agent("w1:p1", "1m");
    cross_device.spawned_from_pane_id = Some("w9:p3".to_owned());
    cross_device.spawned_from_machine_id = Some("machine-parent".to_owned());
    cross_device.tokens.remove("activity");
    replica.replace_agents(vec![cross_device]);
    replica.refresh_published_state().expect("published");
    let (remote, excluded) = replica
        .project_remote("mini", replica.project())
        .expect("remote projection");
    assert!(excluded.is_empty(), "{excluded:?}");
    let row = &remote.agents[0];
    assert_eq!(
        row.spawned_from_pane_id.as_deref(),
        Some("w9:p3"),
        "a machine-qualified parent stays raw until the runtime matches that machine"
    );
    assert_eq!(row.declared_parent_pane_id.as_deref(), Some("w9:p3"));
}

/// What the operator sees of a Herdr `agent.list` that carries hided's lineage
/// tokens: the rows the replica projects for a device, joined into the lineage
/// the sidebar draws. Panes `w1:p1` (the parent) and `w1:p2` (the child) share
/// one device, so the declared parent is scoped to it like any pane id.
mod lineage_sessions {
    use super::*;
    use crate::model::SidebarAgentSnapshot;

    fn digest(session: &str) -> String {
        wire::session_digest(session).expect("a session names itself")
    }

    fn listed(pane: &str, session: Option<&str>, tokens: Value) -> Value {
        let mut agent = json!({
            "pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1",
            "agent": "claude", "agent_status": "working", "focused": false,
            "terminal_id": format!("terminal-{pane}"), "revision": 0,
            "tokens": tokens
        });
        if let Some(session) = session {
            agent["agent_session"] = json!({"source": "herdr:claude", "agent": "claude", "kind": "id", "value": session});
        }
        agent
    }

    /// The tokens hided writes on the child of `parent_session` that runs
    /// `child_session`.
    fn declared(child_session: &str, parent_session: &str) -> Value {
        json!({
            "parent_pane": "w1:p1",
            "child_session": digest(child_session),
            "parent_session": digest(parent_session),
        })
    }

    /// The lineage rows of one device's `agent.list`, parent first.
    fn lineage(parent: Value, child: Value) -> Vec<SidebarAgentSnapshot> {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let agents =
            wire::agents_response(json!({"type": "agent_list", "agents": [parent, child]}))
                .expect("an agent list");
        replica.replace_agents(agents);
        replica.refresh_published_state().expect("published");
        let (mut remote, excluded) = replica
            .project_remote("mini", replica.project())
            .expect("remote projection");
        assert!(excluded.is_empty(), "{excluded:?}");
        crate::agent_state::apply_lineage(&mut remote.agents, &[], &[]);
        remote.agents
    }

    fn row<'a>(rows: &'a [SidebarAgentSnapshot], pane: &str) -> &'a SidebarAgentSnapshot {
        rows.iter()
            .find(|row| row.pane_id == format!("remote:mini:pane:{pane}"))
            .expect("the agent is listed")
    }

    fn assert_root_with_no_trace_of_a_parent(rows: &[SidebarAgentSnapshot]) {
        let child = row(rows, "w1:p2");
        assert_eq!(child.lineage_parent_pane_id, None, "no line to a parent");
        assert!(!child.delegated, "the operator's own work again");
        assert!(
            !child.lineage_orphan,
            "not an orphan of a parent that never was"
        );
        assert_eq!(
            child.lineage_hint, None,
            "no 'from an agent Hide can't see' hint either"
        );
        let parent = row(rows, "w1:p1");
        assert!(
            parent.lineage_child_pane_ids.is_empty(),
            "the pane's agent adopts no one"
        );
        assert!(
            parent.descendant_counts == crate::model::DescendantCountsSnapshot::default(),
            "no descendant badge"
        );
    }

    #[test]
    fn a_child_and_a_parent_that_still_run_their_sessions_are_a_tree() {
        let rows = lineage(
            listed("w1:p1", Some("s-parent"), json!({})),
            listed("w1:p2", Some("s-child"), declared("s-child", "s-parent")),
        );
        let child = row(&rows, "w1:p2");
        assert_eq!(
            child.lineage_parent_pane_id.as_deref(),
            Some("remote:mini:pane:w1:p1")
        );
        assert!(child.delegated);
        assert_eq!(
            row(&rows, "w1:p1").lineage_child_pane_ids,
            ["remote:mini:pane:w1:p2"]
        );
        assert_eq!(row(&rows, "w1:p1").descendant_counts.working, 1);
    }

    #[test]
    fn a_pane_reused_by_another_agent_is_a_root_not_the_old_childs_row() {
        let rows = lineage(
            listed("w1:p1", Some("s-parent"), json!({})),
            listed(
                "w1:p2",
                Some("s-different-agent"),
                declared("s-child", "s-parent"),
            ),
        );
        assert_root_with_no_trace_of_a_parent(&rows);
    }

    #[test]
    fn a_parent_pane_reused_by_another_agent_adopts_none_of_the_old_children() {
        let rows = lineage(
            listed("w1:p1", Some("s-different-agent"), json!({})),
            listed("w1:p2", Some("s-child"), declared("s-child", "s-parent")),
        );
        assert_root_with_no_trace_of_a_parent(&rows);
    }

    #[test]
    fn a_pane_reporting_no_session_cannot_prove_a_relationship_on_either_side() {
        let silent_child = lineage(
            listed("w1:p1", Some("s-parent"), json!({})),
            listed("w1:p2", None, declared("s-child", "s-parent")),
        );
        assert_root_with_no_trace_of_a_parent(&silent_child);
        let silent_parent = lineage(
            listed("w1:p1", None, json!({})),
            listed("w1:p2", Some("s-child"), declared("s-child", "s-parent")),
        );
        assert_root_with_no_trace_of_a_parent(&silent_parent);
    }

    #[test]
    fn a_parent_declared_before_sessions_were_recorded_is_a_root_until_hided_rewrites_it() {
        let rows = lineage(
            listed("w1:p1", Some("s-parent"), json!({})),
            listed("w1:p2", Some("s-child"), json!({"parent_pane": "w1:p1"})),
        );
        assert_root_with_no_trace_of_a_parent(&rows);
    }

    #[test]
    fn a_parent_whose_pane_is_gone_still_leaves_its_child_an_orphan_with_its_hint() {
        let mut replica = SessionReplica::from_snapshot(&snapshot()).expect("snapshot");
        let agents = wire::agents_response(json!({"type": "agent_list", "agents": [
            listed("w1:p2", Some("s-child"), declared("s-child", "s-parent"))
        ]}))
        .expect("an agent list");
        replica.replace_agents(agents);
        replica.refresh_published_state().expect("published");
        let (mut remote, _) = replica
            .project_remote("mini", replica.project())
            .expect("remote projection");
        crate::agent_state::apply_lineage(&mut remote.agents, &[], &[]);
        let child = &remote.agents[0];
        assert!(
            child.lineage_orphan,
            "the parent ended, which is not the pane being reused"
        );
        assert_eq!(
            child.lineage_hint.as_deref(),
            Some("↳ from an agent Hide can't see")
        );
    }
}

/// herdrdev/herdr#3853, end to end through the coordinator: Herdr lists
/// agents at rest whose starts it still holds open, and settles a start only
/// when the agent is read with `agent.get`, as Herdr 0.9.5 does. The core
/// reads the start Hide made once, the next lists show it settled and it is
/// not read again; the start Hide has no record of is never read, since a
/// read past its unknown deadline would clear its name. Across a reconnect
/// the cold start stays unread, the settled one is not read again, and a
/// start Hide made before the reconnect is read after it. The starts' state
/// never reaches the shell.
#[test]
fn an_agent_at_rest_with_its_start_pending_is_read_until_herdr_settles_it() {
    use std::sync::mpsc;

    struct Server {
        closer: hide_platform::ipc::ListenerCloser,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.closer.close();
            self.worker.take().unwrap().join().unwrap();
        }
    }
    #[derive(Default)]
    struct Herdr {
        settled: BTreeSet<String>,
        reads: Vec<Value>,
        subscriptions: usize,
        lists: usize,
    }
    let pending = |pane: &str, terminal: &str| {
        json!({
            "pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1",
            "terminal_id": terminal, "focused": false, "revision": 1,
            "name": format!("{pane}-agent"), "agent": "pi", "agent_status": "idle",
            "state_change_seq": 4, "tokens": {}
        })
    };
    // Hide made the starts in w1:p1 and w1:p3 (four seconds ago, past Herdr's
    // settle delay); an operator made w1:p2's, which Hide has no record of.
    let made = Instant::now().checked_sub(Duration::from_secs(4)).unwrap();
    crate::session_sync::record_start("launch-settle-made", made);
    crate::session_sync::record_start("launch-settle-after-reconnect", made);

    let folder = tempfile::Builder::new()
        .prefix("launch-settle-")
        .tempdir_in(socket_parent())
        .unwrap();
    let socket = folder.path().join("herdr.sock");
    let listener = LocalListener::bind(&socket).unwrap();
    let closer = listener.closer();
    let herdr = Arc::new(Mutex::new(Herdr::default()));
    let server_herdr = Arc::clone(&herdr);
    let (streams, subscription) = mpsc::channel();
    let server = Server {
        closer,
        worker: Some(thread::spawn(move || {
            while let Ok(stream) = listener.accept() {
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                let mut stream = reader.into_inner();
                let mut herdr = server_herdr.lock().unwrap();
                let mut listed = vec![
                    pending("w1:p1", "launch-settle-made"),
                    pending("w1:p2", "launch-settle-cold"),
                ];
                if herdr.subscriptions > 1 {
                    listed.push(pending("w1:p3", "launch-settle-after-reconnect"));
                }
                for agent in &mut listed {
                    let pane = agent["pane_id"].as_str().unwrap().to_owned();
                    if herdr.settled.contains(&pane) {
                        agent["interactive_ready"] = json!(true);
                    } else {
                        agent["launch_pending"] = json!(true);
                    }
                }
                let result = match request["method"].as_str().unwrap() {
                    "events.subscribe" => {
                        herdr.subscriptions += 1;
                        json!({"type": "subscription_started"})
                    }
                    "session.snapshot" => {
                        json!({"type": "session_snapshot", "snapshot": snapshot()})
                    }
                    "agent.list" => {
                        herdr.lists += 1;
                        json!({"type": "agent_list", "agents": listed})
                    }
                    "agent.get" => {
                        let pane = request["params"]["target"].as_str().unwrap().to_owned();
                        herdr.reads.push(request["params"].clone());
                        herdr.settled.insert(pane.clone());
                        let mut agent = listed
                            .into_iter()
                            .find(|agent| agent["pane_id"] == pane)
                            .unwrap();
                        agent["interactive_ready"] = json!(true);
                        agent["launch_pending"] = Value::Null;
                        json!({"type": "agent_info", "agent": agent})
                    }
                    "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                        "pane_id": request["params"]["pane_id"], "foreground_processes": []
                    }}),
                    method => panic!("unexpected fixture method {method}"),
                };
                drop(herdr);
                let response = wire::checked_response_fixture(&request["id"], result);
                writeln!(stream, "{response}").unwrap();
                if request["method"] == "events.subscribe" {
                    streams.send(stream).unwrap();
                }
            }
        })),
    };
    let runtime = runtime_for_fixture(&socket, &folder.path().join("state.json"));
    let handle = spawn(context_for_fixture(&runtime, &socket), None).unwrap();
    let first = subscription
        .recv_timeout(Duration::from_secs(10))
        .expect("the core subscribes");
    // Two lists after the settle: the core has seen the start settled, and
    // had a list on which a second read would have gone.
    let lists_after = |count: usize| {
        let settled_at = herdr.lock().unwrap().lists;
        wait_until(Instant::now() + Duration::from_secs(10), || {
            herdr.lock().unwrap().lists >= settled_at + count
        });
    };
    wait_until(Instant::now() + Duration::from_secs(10), || {
        herdr.lock().unwrap().settled.contains("w1:p1")
    });
    lists_after(2);
    assert_eq!(
        herdr.lock().unwrap().reads,
        [json!({"target": "w1:p1"})],
        "Hide's start is read once, by its pane id, and the cold one never"
    );

    drop(first);
    let second = subscription
        .recv_timeout(Duration::from_secs(10))
        .expect("the core subscribes again");
    wait_until(Instant::now() + Duration::from_secs(10), || {
        herdr.lock().unwrap().settled.contains("w1:p3")
    });
    lists_after(2);
    assert_eq!(
        herdr.lock().unwrap().reads,
        [json!({"target": "w1:p1"}), json!({"target": "w1:p3"})],
        "after the reconnect only the start Hide made before it is read"
    );
    let shell = serde_json::to_string(&runtime.lock().unwrap().snapshot()).unwrap();
    assert!(
        !shell.contains("launch_pending"),
        "the starts' state stays in the core"
    );

    drop(handle);
    drop(second);
    drop(server);
    remove_fixture(folder, runtime);
}
