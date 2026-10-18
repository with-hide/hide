//! A checkout start holds its tab through every effect, not just the first
//! changed layout. The worker boundary receives one grant and explicitly
//! confirms the exact joined topology; these tests drive those answers.

use super::super::operations::{CHECKOUT_START_GEOMETRY_TIMEOUT_MS, GEOMETRY_QUEUE_LIMIT};
use super::*;
use std::sync::mpsc::TryRecvError;

const PATH: &str = "/checkout-start-geometry";
const TAB: &str = "w-order:t1";
const PANE: &str = "w-order:t1:p";

fn session(zoomed: bool) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(PATH, &[TAB], &[TAB], TAB);
    payload.layouts[0].zoomed = zoomed;
    payload
}

fn setup() -> (Runtime, FakeHerdr, u64) {
    let herdr = FakeHerdr::start_with_errors("checkout-start-geometry", |method, _| match method {
        "pane.zoom" => Ok(serde_json::json!({"type":"ok"})),
        other => Err(("unsupported".into(), format!("unexpected {other}"))),
    });
    let (mut runtime, _) = tab_order_runtime(PATH);
    record_terminals(&mut runtime);
    runtime.live = Some(live::LiveContext {
        socket_path: herdr.socket_path().to_path_buf(),
        runtime: Weak::new(),
        notifier: ChangeNotifier::noop(),
        api_connector: Arc::new(herdr.connector()),
        node: Arc::new(hide_node::Local::of_process()),
    });
    runtime.ingest_session(Ok(session(true)));
    let task = runtime
        .begin_task_operation("checkout_start", None, None, None, None)
        .unwrap();
    (runtime, herdr, task)
}

fn zoom(runtime: &mut Runtime) -> String {
    runtime.begin_pane_operation(
        PaneControlAction::ToggleZoom {
            pane_id: PANE.into(),
        },
        None,
    );
    runtime
        .pane_operations
        .values()
        .max_by_key(|operation| operation.sequence)
        .unwrap()
        .id
        .clone()
}

fn confirm_unzoom(runtime: &mut Runtime, id: &str) {
    assert!(runtime.ingest_pane_mutation_result(
        id,
        Ok(PaneControlOutcome::Acknowledged {
            created_pane_id: None
        }),
        0
    ));
    runtime.ingest_session(Ok(session(false)));
}

#[test]
fn checkout_start_waits_its_turn_and_holds_followers_through_intermediate_topology() {
    let (mut runtime, _herdr, task) = setup();
    let first = zoom(&mut runtime);
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    let follower = zoom(&mut runtime);
    assert_eq!(ready.try_recv(), Err(TryRecvError::Empty));
    confirm_unzoom(&mut runtime, &first);
    let lease = ready.try_recv().unwrap().unwrap();
    assert!(runtime.checkout_start_geometry_is_current(task, &lease));
    assert_eq!(runtime.pane_operations[&follower].phase, "queued");

    // Neither a transport ACK nor the intermediate unzoom can stand in for
    // the worker's proof that the exact newly created pane joined the tab.
    assert!(!runtime.ingest_pane_mutation_result(&lease, Ok(PaneControlOutcome::Unchanged), 0));
    runtime.ingest_session(Ok(session(true)));
    runtime.ingest_session(Ok(session(false)));
    assert!(runtime.checkout_start_geometry_is_current(task, &lease));
    assert_eq!(runtime.pane_operations[&follower].phase, "queued");
    runtime.finish_checkout_start_geometry(task + 1, &lease, Ok(()));
    assert!(runtime.checkout_start_geometry_is_current(task, &lease));
    runtime.finish_checkout_start_geometry(task, &lease, Ok(()));
    assert!(!runtime.checkout_start_geometry_is_current(task, &lease));
    assert_eq!(runtime.pane_operations[&follower].phase, "transmitting");
    runtime.finish_checkout_start_geometry(task, &lease, Err("late reply".into()));
    assert_eq!(runtime.pane_operations[&follower].phase, "transmitting");
}

#[test]
fn checkout_start_shares_the_existing_eight_waiter_limit() {
    let (mut runtime, _herdr, task) = setup();
    zoom(&mut runtime);
    for _ in 0..GEOMETRY_QUEUE_LIMIT {
        zoom(&mut runtime);
    }
    assert!(runtime.queue_checkout_start_geometry(task, PANE).is_err());
    assert_eq!(runtime.pane_operations.len(), GEOMETRY_QUEUE_LIMIT + 1);
}

#[test]
fn checkout_start_receives_refusal_when_the_operation_ahead_fails() {
    let (mut runtime, _herdr, task) = setup();
    let first = zoom(&mut runtime);
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    let follower = zoom(&mut runtime);
    runtime.fail_pane_operation(&first, "Herdr refused zoom".into());
    assert!(ready.try_recv().unwrap().is_err());
    assert!(!runtime.pane_operations.contains_key(&follower));
    assert!(!runtime.geometry_tab_busy(TAB));
}

#[test]
fn checkout_start_cancel_covers_both_waiting_and_grant_racing_with_timeout() {
    let (mut runtime, _herdr, task) = setup();
    let first = zoom(&mut runtime);
    let predecessor = zoom(&mut runtime);
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    let follower = zoom(&mut runtime);
    runtime.cancel_checkout_start_geometry(task);
    assert!(ready.try_recv().unwrap().is_err());
    assert!(!runtime.pane_operations.contains_key(&follower));
    assert_eq!(runtime.pane_operations[&first].phase, "transmitting");
    assert_eq!(runtime.pane_operations[&predecessor].phase, "queued");
    confirm_unzoom(&mut runtime, &first);
    assert_eq!(runtime.pane_operations[&predecessor].phase, "transmitting");
    runtime.ingest_pane_mutation_result(&predecessor, Ok(PaneControlOutcome::Unchanged), 0);

    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    // The grant is in the channel while the worker's acquire wait expires.
    runtime.cancel_checkout_start_geometry(task);
    let late_grant = ready.try_recv().unwrap().unwrap();
    assert!(!runtime.checkout_start_geometry_is_current(task, &late_grant));
    assert!(!runtime.geometry_tab_busy(TAB));
}

#[test]
fn checkout_start_abandoned_receiver_cannot_lock_the_tab() {
    let (mut runtime, _herdr, task) = setup();
    let first = zoom(&mut runtime);
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    let follower = zoom(&mut runtime);
    drop(ready);
    confirm_unzoom(&mut runtime, &first);
    assert!(!runtime.geometry_tab_busy(TAB));
    assert!(!runtime.pane_operations.contains_key(&follower));
}

#[test]
fn checkout_start_deadline_revokes_the_lease_and_drops_followers_without_resending() {
    let (mut runtime, _herdr, task) = setup();
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    let lease = ready.try_recv().unwrap().unwrap();
    let follower = zoom(&mut runtime);
    runtime.expire_pane_operations(unix_milliseconds() + CHECKOUT_START_GEOMETRY_TIMEOUT_MS + 1);
    assert!(!runtime.checkout_start_geometry_is_current(task, &lease));
    assert!(!runtime.pane_operations.contains_key(&follower));
    assert!(!runtime.geometry_tab_busy(TAB));
}

#[test]
fn checkout_start_does_not_grant_a_replaced_task_or_connection() {
    for replace_task in [false, true] {
        let (mut runtime, _herdr, task) = setup();
        let first = zoom(&mut runtime);
        let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
        if replace_task {
            runtime.snapshot.task_operation.as_mut().unwrap().id += 1;
        } else {
            runtime.live_generation += 1;
        }
        confirm_unzoom(&mut runtime, &first);
        assert!(ready.try_recv().unwrap().is_err());
        assert!(!runtime.geometry_tab_busy(TAB));
    }
}

#[test]
fn checkout_start_revalidates_its_group_target_after_waiting() {
    let (runtime, _herdr, task) = setup();
    let mut runtime = super::workspace_view::with_new_views(runtime, "start-geometry-target");
    runtime.ingest_session(Ok(session(true)));
    runtime.sync_workspace_view();
    runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, PATH),
            Some(super::super::group_starts::StartTarget {
                group_id: None,
                area_id: "a1".into(),
                tab_id: Some(TAB.into()),
                add_pane: true,
            }),
            Some("geometry-target"),
            None,
            false,
        )
        .unwrap();
    let first = zoom(&mut runtime);
    let ready = runtime.queue_checkout_start_geometry(task, PANE).unwrap();
    // The checkout presentation was removed while the worker waited. Its
    // unchanged Herdr pane is insufficient authority to start in that group.
    runtime.workspace_views = None;
    confirm_unzoom(&mut runtime, &first);
    assert!(ready.try_recv().unwrap().is_err());
    assert!(!runtime.geometry_tab_busy(TAB));
}

const DEVICE: &str = "geometry-device";

fn remote_setup() -> (Runtime, FakeHerdr, u64, String, String) {
    let (mut runtime, herdr, task) = setup();
    let tab = format!("remote:{DEVICE}:tab:{TAB}");
    let pane_id = remote_pane_id(DEVICE, PANE);
    let workspace_id = format!("remote:{DEVICE}:workspace:w-order");
    let mut owned = checkout(
        &workspace_id,
        "remote-checkout",
        PATH,
        Some(pane(&pane_id, PATH)),
    );
    owned.tabs[0].id = Some(tab.clone());
    let other_pane = remote_pane_id(DEVICE, "w-order:p2");
    owned.tabs[0].panes.push(pane(&other_pane, PATH));
    let mut project = workspace(&workspace_id, "Geometry", PATH, vec![owned]);
    project.device_id = DEVICE.into();
    project.remote_target_id = Some(DEVICE.into());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: DEVICE.into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: Vec::new(),
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: Some(tab.clone()),
            focused_pane_id: Some(pane_id.clone()),
            pane_layouts: vec![crate::model::RemotePaneLayoutSnapshot {
                workspace_id,
                tab_id: tab.clone(),
                focused_pane_id: pane_id.clone(),
                zoomed: true,
                frames: vec![
                    crate::model::RemotePaneLayoutFrame {
                        pane_id: pane_id.clone(),
                        x: 0.0,
                        y: 0.0,
                        width: 0.5,
                        height: 1.0,
                    },
                    crate::model::RemotePaneLayoutFrame {
                        pane_id: other_pane,
                        x: 0.5,
                        y: 0.0,
                        width: 0.5,
                        height: 1.0,
                    },
                ],
            }],
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
    });
    runtime.install_remote_control(RemoteControlContext::new(
        DEVICE,
        Arc::new(herdr.connector()),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    runtime.snapshot.task_operation.as_mut().unwrap().device_id = Some(DEVICE.into());
    (runtime, herdr, task, tab, pane_id)
}

fn remote_zoom(runtime: &mut Runtime, pane_id: &str, request: &str) {
    runtime.request_remote_control(RemoteControlPayload {
        target_id: DEVICE.into(),
        request_id: request.into(),
        report_pane_focus_outcome: false,
        focus_device: false,
        request: RemoteControlRequest::TogglePaneZoom {
            pane_id: pane_id.into(),
        },
    });
}

#[test]
fn checkout_start_waits_for_remote_control_and_blocks_later_remote_controls() {
    let (mut runtime, _herdr, task, tab, pane_id) = remote_setup();
    remote_zoom(&mut runtime, &pane_id, "first");
    assert_eq!(runtime.remote_operations.len(), 1);
    let ready = runtime
        .queue_checkout_start_geometry(task, &pane_id)
        .unwrap();
    assert_eq!(ready.try_recv(), Err(TryRecvError::Empty));
    remote_zoom(&mut runtime, &pane_id, "while-queued");
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "remote.control.in_progress"
    );
    runtime.acknowledge_remote_operation(DEVICE, "first", None);
    let session = runtime.snapshot.status.remote[0].session.as_mut().unwrap();
    session.pane_layouts[0].zoomed = false;
    let session = session.clone();
    assert!(runtime.observe_remote_operations(DEVICE, &session));
    let lease = ready.try_recv().unwrap().unwrap();
    assert!(runtime.checkout_start_geometry_is_current(task, &lease));
    remote_zoom(&mut runtime, &pane_id, "while-granted");
    assert!(runtime.remote_operations.is_empty());
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "remote.control.in_progress"
    );
    runtime.finish_checkout_start_geometry(task, &lease, Ok(()));
    assert!(!runtime.geometry_tab_busy(&tab));
    remote_zoom(&mut runtime, &pane_id, "after-finish");
    assert_eq!(runtime.remote_operations.len(), 1);
}

#[test]
fn checkout_start_remote_unknown_does_not_grant_and_reconnect_revokes_held_lease() {
    let (mut runtime, _herdr, task, tab, pane_id) = remote_setup();
    remote_zoom(&mut runtime, &pane_id, "first");
    runtime.expire_remote_operations(unix_milliseconds() + CLOSE_STAGE_TIMEOUT_MS + 1);
    let ready = runtime
        .queue_checkout_start_geometry(task, &pane_id)
        .unwrap();
    assert_eq!(ready.try_recv(), Err(TryRecvError::Empty));
    runtime.expire_pane_operations(unix_milliseconds() + CHECKOUT_START_GEOMETRY_TIMEOUT_MS + 1);
    assert!(ready.try_recv().unwrap().is_err());
    assert!(
        runtime.geometry_tab_busy(&tab),
        "unknown remote control still owns the tab"
    );
    runtime.remote_operations.clear();
    let ready = runtime
        .queue_checkout_start_geometry(task, &pane_id)
        .unwrap();
    let lease = ready.try_recv().unwrap().unwrap();
    *runtime
        .remote_connection_generations
        .entry(DEVICE.into())
        .or_default() += 1;
    assert!(!runtime.checkout_start_geometry_is_current(task, &lease));
    runtime.finish_checkout_start_geometry(task, &lease, Ok(()));
    assert!(!runtime.geometry_tab_busy(&tab));
}
