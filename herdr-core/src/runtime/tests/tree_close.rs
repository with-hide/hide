use super::*;

// PRD close-agent-subtree: a parent closed with its descendants goes deepest
// first, the target last, and closes only what the sheet listed.
//
// The fixture: project A holds the parent p1, its child p2 with grandchild
// p3, and its child p4, each in its own tab; project B, unregistered, holds
// only p5, a child of p4.

pub(super) const A: &str = "/private/tmp/hide-tree-close-a";
pub(super) const B: &str = "/private/tmp/hide-tree-close-b";

/// The session with every pane in `present`; `status` overrides an agent's
/// Herdr status by pane. With `shared`, the siblings p2 and p4 share tab t2.
pub(super) fn tree_payload(present: &[&str], status: &[(&str, &str)]) -> SessionSnapshotPayload {
    tree_payload_in(present, status, false)
}

fn tree_payload_in(
    present: &[&str],
    status: &[(&str, &str)],
    shared: bool,
) -> SessionSnapshotPayload {
    let all = [
        ("w1", "w1:t1", "w1:p1", None),
        ("w1", "w1:t2", "w1:p2", Some("w1:p1")),
        ("w1", "w1:t3", "w1:p3", Some("w1:p2")),
        (
            "w1",
            if shared { "w1:t2" } else { "w1:t4" },
            "w1:p4",
            Some("w1:p1"),
        ),
        ("w5", "w5:t5", "w5:p5", Some("w1:p4")),
    ];
    let (mut tabs, mut panes, mut layouts, mut agents) = (vec![], vec![], vec![], vec![]);
    let mut seen_tabs: Vec<&str> = Vec::new();
    for (index, (workspace, tab, pane, parent)) in all.iter().enumerate() {
        if !present.contains(pane) {
            continue;
        }
        let cwd = if *workspace == "w1" { A } else { B };
        panes.push(serde_json::json!({"pane_id": pane, "cwd": cwd}));
        if !seen_tabs.contains(tab) {
            seen_tabs.push(tab);
            tabs.push(serde_json::json!({"workspace_id": workspace, "tab_id": tab, "label": ""}));
            let in_tab = all
                .iter()
                .filter(|(_, other, id, _)| other == tab && present.contains(id))
                .enumerate()
                .map(|(slot, (_, _, id, _))| {
                    serde_json::json!({"pane_id": id, "rect": {"x": slot * 40, "y": 0, "width": 40, "height": 24}})
                })
                .collect::<Vec<_>>();
            layouts.push(serde_json::json!({
                "workspace_id": workspace,
                "tab_id": tab,
                "zoomed": false,
                "area": {"x": 0, "y": 0, "width": 80, "height": 24},
                "focused_pane_id": pane,
                "panes": in_tab,
                "splits": []
            }));
        }
        let agent_status = status
            .iter()
            .find(|(id, _)| id == pane)
            .map(|(_, status)| *status)
            .unwrap_or("idle");
        agents.push(serde_json::json!({
            "id": format!("agent-{pane}"),
            "pane_id": pane,
            "agent": "claude",
            "agent_status": agent_status,
            "spawned_from_pane_id": parent,
            "state_change_seq": index + 1,
            "tokens": {"task": format!("Task {pane}")}
        }));
    }
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents,
        "focused_workspace_id": "w1",
        "focused_pane_id": "w1:p1",
        "workspaces": [
            {"workspace_id": "w1", "label": "a", "active_tab_id": "w1:t1"},
            {"workspace_id": "w5", "label": "b", "active_tab_id": "w5:t5"}
        ],
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("tree session payload")
}

pub(super) fn tree_catalog() -> session_sync::PrecomputedCatalog {
    let spaces = vec![
        workspace::SessionSpace {
            checkout: None,
            id: "w1".to_owned(),
            label: "a".to_owned(),
            purpose: None,
            cwds: vec![A.to_owned()],
        },
        workspace::SessionSpace {
            checkout: None,
            id: "w5".to_owned(),
            label: "b".to_owned(),
            purpose: None,
            cwds: vec![B.to_owned()],
        },
    ];
    session_sync::PrecomputedCatalog {
        worktrees: no_worktrees(),
        registrations: Vec::new(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &crate::model::WorktreeCatalogSnapshot::default(),
            &workspace::catalog_paths_here(
                &[],
                &spaces,
                &crate::model::WorktreeCatalogSnapshot::default(),
            ),
        ),
        paths: std::sync::Arc::new(workspace::paths_here(
            spaces.iter().flat_map(|space| space.cwds.clone()),
        )),
    }
}

pub(super) const EVERY: [&str; 5] = ["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w5:p5"];

pub(super) fn tree_runtime(status: &[(&str, &str)]) -> Runtime {
    live_runtime(tree_payload(&EVERY, status))
}

/// A runtime connected to a Herdr that takes every request, showing `payload`.
fn live_runtime(payload: SessionSnapshotPayload) -> Runtime {
    let mut runtime = runtime();
    runtime.restore_hint_pending = false;
    let socket_path = std::env::temp_dir()
        .join(format!(
            "herdr-core-tree-close-{}-{}.sock",
            std::process::id(),
            NEXT_RUNTIME_STATE_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    runtime.live = Some(live::LiveContext {
        socket_path: socket_path.clone().into(),
        runtime: std::sync::Weak::new(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    });
    assert!(runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog())));
    runtime
}

fn close_tree(
    runtime: &mut Runtime,
    target: serde_json::Value,
    pane_ids: &[&str],
    confirmed: bool,
) {
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "close_tree",
        "payload": {"target": target, "pane_ids": pane_ids, "confirmed": confirmed}
    }))
    .unwrap();
    runtime.dispatch_json(&event);
}

fn pane_target(pane: &str) -> serde_json::Value {
    serde_json::json!({"kind": "pane", "pane_id": pane})
}

/// Panes whose ordinary close has been started, sorted.
pub(super) fn started(runtime: &Runtime) -> Vec<String> {
    let mut started = runtime
        .close_operations
        .values()
        .map(|operation| operation.target_id.clone())
        .collect::<Vec<_>>();
    started.sort();
    started
}

pub(super) fn leave(runtime: &mut Runtime, present: &[&str]) {
    runtime.ingest_session_with_catalog(Ok(tree_payload(present, &[])), Some(tree_catalog()));
}

fn tree_phase(runtime: &Runtime, pane: &str) -> Option<String> {
    runtime
        .snapshot()
        .status
        .async_operations
        .iter()
        .find(|operation| operation.kind == "tree.close" && operation.target_id == pane)
        .map(|operation| operation.phase.clone())
}

#[test]
fn a_tree_close_starts_the_deepest_listed_panes_first_and_the_target_last() {
    let mut runtime = tree_runtime(&[]);
    let parent = runtime
        .snapshot()
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == "w1:p1")
        .unwrap()
        .close_descendant_pane_ids
        .clone();
    assert_eq!(parent.len(), 4);

    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4", "w5:p5"],
        true,
    );
    assert_eq!(
        started(&runtime),
        ["w1:p3", "w5:p5"],
        "only the leaves start"
    );
    assert_eq!(tree_phase(&runtime, "w1:p3").as_deref(), Some("closing"));
    assert_eq!(tree_phase(&runtime, "w1:p1").as_deref(), Some("waiting"));

    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4"]);
    assert_eq!(started(&runtime), ["w1:p2", "w1:p4"]);

    leave(&mut runtime, &["w1:p1"]);
    assert!(
        started(&runtime).contains(&"w1:p1".to_owned()),
        "the target goes last"
    );

    leave(&mut runtime, &[]);
    assert!(runtime.tree_closes.is_empty());
    assert_eq!(
        tree_phase(&runtime, "w1:p1"),
        None,
        "a finished tree leaves no record"
    );
    assert!(
        !runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .is_some_and(|error| error.kind.starts_with("tree_close")),
        "a complete tree reports no failure"
    );
}

#[test]
fn a_descendant_that_is_its_projects_last_pane_keeps_the_project_like_an_ordinary_close() {
    let mut runtime = tree_runtime(&[]);
    assert!(
        runtime
            .snapshot()
            .ui_state
            .workspace_registrations
            .is_empty()
    );
    close_tree(&mut runtime, pane_target("w1:p4"), &["w5:p5"], true);
    assert_eq!(started(&runtime), ["w5:p5"]);
    let registrations = &runtime.snapshot().ui_state.workspace_registrations;
    assert_eq!(
        registrations.len(),
        1,
        "project B stays listed after its last pane"
    );
    assert_eq!(registrations[0].path, B);
}

#[test]
fn a_pane_already_gone_counts_as_closed_and_an_unlisted_descendant_stays_open() {
    let mut runtime = tree_runtime(&[]);
    // The sheet listed p2 and p3 only; p4 and p5 are not the operator's to
    // close in this request.
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3"],
        true,
    );
    assert_eq!(started(&runtime), ["w1:p3"]);
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert_eq!(started(&runtime), ["w1:p2"]);
    leave(&mut runtime, &["w1:p1", "w1:p4", "w5:p5"]);
    assert!(started(&runtime).contains(&"w1:p1".to_owned()));
    assert!(!started(&runtime).contains(&"w1:p4".to_owned()));
    assert!(!started(&runtime).contains(&"w5:p5".to_owned()));

    // A listed pane that is already gone does not hold its parent back.
    let mut runtime = tree_runtime(&[]);
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    close_tree(&mut runtime, pane_target("w1:p2"), &["w1:p3"], true);
    assert_eq!(started(&runtime), ["w1:p2"]);
}

#[test]
fn a_listed_pane_that_needs_a_status_check_or_a_confirmation_refuses_the_whole_close() {
    let mut runtime = tree_runtime(&[("w1:p3", "unknown")]);
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4", "w5:p5"],
        true,
    );
    assert!(started(&runtime).is_empty(), "nothing closes");
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tree_close.status_unknown")
    );
    assert!(runtime.tree_closes.is_empty());

    let mut runtime = tree_runtime(&[("w5:p5", "working")]);
    close_tree(&mut runtime, pane_target("w1:p4"), &["w5:p5"], false);
    assert!(started(&runtime).is_empty());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tree_close.confirmation_required")
    );
    close_tree(&mut runtime, pane_target("w1:p4"), &["w5:p5"], true);
    assert_eq!(
        started(&runtime),
        ["w5:p5"],
        "confirmed covers the whole list"
    );
}

#[test]
fn a_refused_descendant_keeps_its_ancestors_open_while_the_other_branch_closes() {
    let mut runtime = tree_runtime(&[]);
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4", "w5:p5"],
        true,
    );
    let key = runtime
        .close_operations
        .iter()
        .find(|(_, operation)| operation.target_id == "w1:p3")
        .map(|(key, _)| key.clone())
        .unwrap();
    runtime.fail_close_operation(&key, "refused by Herdr".to_owned());
    runtime.tick_async_operations(unix_milliseconds());
    assert_eq!(tree_phase(&runtime, "w1:p3").as_deref(), Some("failed"));
    assert_eq!(tree_phase(&runtime, "w1:p2").as_deref(), Some("failed"));
    assert_eq!(tree_phase(&runtime, "w1:p1").as_deref(), Some("failed"));

    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p3", "w1:p4"]);
    assert!(
        started(&runtime).contains(&"w1:p4".to_owned()),
        "the other branch runs on"
    );
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p3"]);
    assert!(runtime.tree_closes.is_empty());
    assert!(!started(&runtime).contains(&"w1:p1".to_owned()));
    assert!(!started(&runtime).contains(&"w1:p2".to_owned()));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tree_close.incomplete")
    );

    // Closing again takes only what remains; the refused close it retries
    // is no longer in its way.
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3"],
        true,
    );
    assert!(started(&runtime).contains(&"w1:p3".to_owned()));
    assert!(!started(&runtime).contains(&"w1:p2".to_owned()));
}

/// A parent `w1:p1` with `children` delegated children, each in its own tab
/// as a delegated child opens, and an unrelated root `w1:q`, all in project A;
/// only the panes in `present` are open.
fn wide_payload(children: usize, present: &[String]) -> SessionSnapshotPayload {
    let mut all = vec![("w1:t1".to_owned(), "w1:p1".to_owned(), None)];
    all.push(("w1:tq".to_owned(), "w1:q".to_owned(), None));
    for index in 0..children {
        all.push((
            format!("w1:tc{index}"),
            format!("w1:c{index}"),
            Some("w1:p1"),
        ));
    }
    let (mut tabs, mut panes, mut layouts, mut agents) = (vec![], vec![], vec![], vec![]);
    for (index, (tab, pane, parent)) in all.iter().enumerate() {
        if !present.contains(pane) {
            continue;
        }
        panes.push(serde_json::json!({"pane_id": pane, "cwd": A}));
        tabs.push(serde_json::json!({"workspace_id": "w1", "tab_id": tab, "label": ""}));
        layouts.push(serde_json::json!({
            "workspace_id": "w1",
            "tab_id": tab,
            "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": pane,
            "panes": [{"pane_id": pane, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }));
        agents.push(serde_json::json!({
            "id": format!("agent-{pane}"),
            "pane_id": pane,
            "agent": "claude",
            "agent_status": "idle",
            "spawned_from_pane_id": parent,
            "state_change_seq": index + 1,
            "tokens": {"task": format!("Task {pane}")}
        }));
    }
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents,
        "focused_workspace_id": "w1",
        "focused_pane_id": "w1:p1",
        "workspaces": [
            {"workspace_id": "w1", "label": "a", "active_tab_id": "w1:t1"},
            {"workspace_id": "w5", "label": "b", "active_tab_id": null}
        ],
        "tabs": tabs,
        "panes": panes,
        "layouts": layouts
    }))
    .expect("wide session payload")
}

fn wide_children(children: usize) -> Vec<String> {
    (0..children).map(|index| format!("w1:c{index}")).collect()
}

fn ingest_wide(runtime: &mut Runtime, children: usize, present: &[String]) {
    runtime.ingest_session_with_catalog(Ok(wide_payload(children, present)), Some(tree_catalog()));
}

fn close_pane(runtime: &mut Runtime, pane: &str) {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "close_pane",
            "payload": {"pane_id": pane, "confirmed": true}
        }))
        .unwrap(),
    );
}

fn last_error_kind(runtime: &Runtime) -> Option<String> {
    runtime
        .snapshot()
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.clone())
}

#[test]
fn a_tree_wider_than_the_close_slots_is_admitted_and_closes_every_pane() {
    // The operator's case: 33 children whose agents ended, each pane left
    // open in its own tab, so the close names 34 local panes, more than the
    // 20 close records Hide keeps at once.
    const CHILDREN: usize = 33;
    let mut present = wide_children(CHILDREN);
    present.extend(["w1:p1".to_owned(), "w1:q".to_owned()]);
    let mut runtime = live_runtime(wide_payload(CHILDREN, &present));
    let children = wide_children(CHILDREN);
    let listed = children.iter().map(String::as_str).collect::<Vec<_>>();

    close_tree(&mut runtime, pane_target("w1:p1"), &listed, true);
    assert_eq!(runtime.tree_closes.len(), 1, "the whole tree is admitted");
    assert_eq!(last_error_kind(&runtime), None);
    let running = started(&runtime).len();
    assert!(
        (1..crate::recent_closed::RECENT_CLOSED_LIMIT).contains(&running),
        "the children start a slot at a time, leaving room: {running}"
    );

    // An ordinary close beside the tree still finds a slot.
    close_pane(&mut runtime, "w1:q");
    assert!(started(&runtime).contains(&"w1:q".to_owned()));
    present.retain(|pane| pane != "w1:q");

    let mut rounds = 0;
    while !runtime.tree_closes.is_empty() {
        rounds += 1;
        assert!(rounds <= CHILDREN + 1, "the tree keeps moving");
        let closing = started(&runtime);
        assert!(
            !closing.is_empty(),
            "a node holds a slot until the tree ends"
        );
        present.retain(|pane| !closing.contains(pane));
        ingest_wide(&mut runtime, CHILDREN, &present);
    }
    assert!(
        present.is_empty(),
        "every listed pane and the target closed"
    );
    assert_eq!(
        last_error_kind(&runtime),
        None,
        "nothing was refused halfway"
    );
}

#[test]
fn a_tree_with_no_free_close_slot_is_refused_with_the_limit_and_one_free_slot_runs_it() {
    let mut runtime = tree_runtime(&[]);
    hold_closes(&mut runtime, 19);
    // p4's own close runs at the front of the queue throughout.
    close_pane(&mut runtime, "w1:p4");
    close_tree(&mut runtime, pane_target("w1:p2"), &["w1:p3"], true);
    assert!(!started(&runtime).contains(&"w1:p3".to_owned()));
    assert!(runtime.tree_closes.is_empty());
    let error = runtime.snapshot().status.last_error.clone().unwrap();
    assert_eq!(error.kind, "tree_close.capacity");
    assert!(
        !error.message.contains("earlier close"),
        "the refusal names the limit, not an earlier close: {}",
        error.message
    );

    // With one slot free the same close runs through it, a node at a time.
    settle_held(&mut runtime, 0..1);
    close_tree(&mut runtime, pane_target("w1:p2"), &["w1:p3"], true);
    assert_eq!(runtime.tree_closes.len(), 1);
    assert!(started(&runtime).contains(&"w1:p3".to_owned()));
    sent(&mut runtime, "w1:p3");
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert!(
        !started(&runtime).contains(&"w1:p2".to_owned()),
        "p3's finished record keeps the slot while it queues behind p4's close"
    );
    leave(&mut runtime, &["w1:p1", "w1:p2", "w5:p5"]);
    assert!(
        started(&runtime).contains(&"w1:p2".to_owned()),
        "the target takes the slot its child's close left"
    );
}

#[test]
fn an_admitted_tree_keeps_its_slots_while_ordinary_closes_take_the_rest() {
    let mut runtime = tree_runtime(&[]);
    hold_closes(&mut runtime, 16);
    close_pane(&mut runtime, "w1:p4");
    // Three slots are free and the tree takes two of them.
    close_tree(&mut runtime, pane_target("w1:p2"), &["w1:p3"], true);
    assert!(started(&runtime).contains(&"w1:p3".to_owned()));
    close_pane(&mut runtime, "w5:p5");
    assert!(
        started(&runtime).contains(&"w5:p5".to_owned()),
        "an ordinary close gets the free slot"
    );
    close_pane(&mut runtime, "w1:p1");
    assert!(
        !started(&runtime).contains(&"w1:p1".to_owned()),
        "the slot the tree still needs is not taken"
    );
    // p3's finished record waits behind p4's close in the queue, so the
    // target starts on the tree's second slot.
    sent(&mut runtime, "w1:p3");
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert!(
        runtime
            .close_operations
            .values()
            .any(|operation| operation.target_id == "w1:p3"),
        "p3's record is still queued"
    );
    assert!(started(&runtime).contains(&"w1:p2".to_owned()));
}

#[test]
fn a_node_waiting_for_a_slot_held_behind_an_unresolved_close_fails_at_once() {
    let mut runtime = tree_runtime(&[]);
    hold_closes(&mut runtime, 19);
    // One slot: p3 takes it, and its record cannot leave the queue while the
    // close in front waits for the operator (a failed replacement shell).
    close_tree(&mut runtime, pane_target("w1:p2"), &["w1:p3"], true);
    let front = runtime.close_operations.get_mut("held-0").unwrap();
    front.phase = "failed".to_owned();
    front.request.context.replacement_shell = true;
    sent(&mut runtime, "w1:p3");
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert!(
        runtime.tree_closes.is_empty(),
        "the target fails instead of waiting forever"
    );
    assert!(!started(&runtime).contains(&"w1:p2".to_owned()));
    assert_eq!(
        last_error_kind(&runtime).as_deref(),
        Some("tree_close.incomplete")
    );
}

/// `count` ordinary closes of other tabs, each still preparing.
fn hold_closes(runtime: &mut Runtime, count: usize) {
    for index in 0..count {
        runtime.ensure_pending_close_from_request(&close_capture_request(&format!("held-{index}")));
    }
}

/// The held closes `range` finish and leave the close queue.
fn settle_held(runtime: &mut Runtime, range: std::ops::Range<usize>) {
    for index in range {
        runtime
            .close_operations
            .get_mut(&format!("held-{index}"))
            .unwrap()
            .phase = "completed".to_owned();
    }
    runtime.promote_close_reservations();
}

/// Herdr has taken `pane`'s close, so the topology confirms it rather than
/// cancelling a close that never left.
fn sent(runtime: &mut Runtime, pane: &str) {
    runtime
        .close_operations
        .values_mut()
        .find(|operation| operation.target_id == pane)
        .unwrap()
        .phase = "awaiting_topology".to_owned();
}

#[test]
fn the_target_record_carries_the_trees_progress() {
    let mut runtime = tree_runtime(&[]);
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4", "w5:p5"],
        true,
    );
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4"]);
    let operations = runtime.snapshot().status.async_operations.clone();
    let target = operations
        .iter()
        .find(|operation| operation.kind == "tree.close" && operation.stage == "target")
        .expect("the target's record");
    assert_eq!(target.target_id, "w1:p1");
    let tree = operations
        .iter()
        .filter(|operation| operation.scope_id == target.scope_id)
        .collect::<Vec<_>>();
    assert_eq!(tree.len(), 5);
    assert_eq!(
        tree.iter()
            .filter(|operation| operation.phase == "completed")
            .count(),
        2
    );
}

#[test]
fn a_device_descendant_closes_through_the_device_and_fails_when_it_disconnects() {
    let mut runtime = tree_runtime(&[]);
    runtime.node = crate::node::NodeId::parse("machine-local").unwrap();
    let remote_pane = "remote:mini:pane:r1";
    let mut child = runtime.snapshot.navigator.agents[1].clone();
    child.pane_id = remote_pane.to_owned();
    child.id = "remote-agent".to_owned();
    child.declared_parent_pane_id = Some("w1:p4".to_owned());
    child.spawned_from_machine_id = Some("machine-local".to_owned());
    let mut remote_checkout = checkout(
        "remote:mini:project",
        "remote:mini:checkout",
        "/remote/c",
        Some(pane(remote_pane, "/remote/c")),
    );
    remote_checkout.tabs[0].id = Some("remote:mini:tab:t1".to_owned());
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![workspace(
                "remote:mini:project",
                "c",
                "/remote/c",
                vec![remote_checkout],
            )],
            agents: vec![child],
            active_tab_ids: Default::default(),
            focused_workspace_id: None,
            focused_checkout_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            pane_layouts: Vec::new(),
            pane_hook_tokens: Default::default(),
            issue_tokens: Default::default(),
            issue_links: Default::default(),
            herdr_version: None,
        }),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.remote_controls.insert(
        "mini".to_owned(),
        live::RemoteControlContext::new(
            "mini",
            Arc::new(hide_herdr_client::LocalSocketConnector::new(
                "/tmp/hide-tree-close-none.sock",
            )),
            std::sync::Weak::new(),
            crate::handle::ChangeNotifier::noop(),
        ),
    );
    runtime.refresh_agent_lineage();
    close_tree(
        &mut runtime,
        pane_target("w1:p4"),
        &["w5:p5", remote_pane],
        true,
    );
    let remote = runtime
        .remote_operations
        .values()
        .find(|operation| operation.target_id == remote_pane)
        .expect("the device's own close was sent");
    assert_eq!(remote.kind, "pane.close");
    assert_eq!(
        started(&runtime),
        ["w5:p5"],
        "the local branch runs beside it"
    );

    // The device leaves the status list altogether (removed or reconnected
    // without a session): its pane's absence is not a confirmed close.
    runtime.snapshot.status.remote.clear();
    runtime.tick_async_operations(unix_milliseconds());
    assert_eq!(tree_phase(&runtime, remote_pane).as_deref(), Some("failed"));
    assert_eq!(tree_phase(&runtime, "w1:p4").as_deref(), Some("failed"));
}

#[test]
fn removing_a_project_closes_its_outside_descendants_first_and_a_failure_keeps_it() {
    let mut runtime = tree_runtime(&[]);
    let project = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.path == A)
        .unwrap()
        .id
        .clone();
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "remove_workspace",
        "payload": {"workspace_id": project, "close_descendant_pane_ids": ["w5:p5"]}
    }))
    .unwrap();
    runtime.dispatch_json(&event);
    assert_eq!(
        started(&runtime),
        ["w5:p5"],
        "only the outside descendant closes first"
    );
    assert!(runtime.workspace_removals_in_flight.contains(&project));
    let key = runtime
        .close_operations
        .iter()
        .find(|(_, operation)| operation.target_id == "w5:p5")
        .map(|(key, _)| key.clone())
        .unwrap();
    runtime.fail_close_operation(&key, "refused by Herdr".to_owned());
    runtime.tick_async_operations(unix_milliseconds());
    assert!(runtime.tree_closes.is_empty());
    assert!(!runtime.workspace_removals_in_flight.contains(&project));
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("workspace.remove_failed"),
        "the removal fails through its own banner and nothing of it started"
    );
    assert!(!started(&runtime).contains(&"w1:p1".to_owned()));
}

#[test]
fn an_unresolved_close_fails_its_same_tab_sibling_at_once_and_a_retry_waits_for_it() {
    let mut runtime = tree_runtime(&[]);
    let shared = |runtime: &mut Runtime, present: &[&str]| {
        runtime.ingest_session_with_catalog(
            Ok(tree_payload_in(present, &[], true)),
            Some(tree_catalog()),
        );
    };
    shared(&mut runtime, &EVERY);
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4", "w5:p5"],
        true,
    );
    shared(&mut runtime, &["w1:p1", "w1:p2", "w1:p4"]);
    // p2 and p4 share a tab, so one close starts there and the other waits.
    let first = ["w1:p2", "w1:p4"]
        .into_iter()
        .find(|pane| started(&runtime).contains(&(*pane).to_owned()))
        .expect("one sibling starts");
    let second = if first == "w1:p2" { "w1:p4" } else { "w1:p2" };
    assert_eq!(tree_phase(&runtime, second).as_deref(), Some("waiting"));

    // Herdr's answer is lost and the status check cannot tell: the record
    // stays unknown until the operator checks again, which refuses any
    // second close in that tab.
    let operation = runtime
        .close_operations
        .values_mut()
        .find(|operation| operation.target_id == first)
        .unwrap();
    operation.phase = "unknown".to_owned();
    operation.deadline_at_unix_ms = None;
    runtime.tick_async_operations(unix_milliseconds());
    assert!(
        runtime.tree_closes.is_empty(),
        "the sibling fails instead of waiting behind the unresolved close"
    );
    assert!(!started(&runtime).contains(&second.to_owned()));
    assert!(!started(&runtime).contains(&"w1:p1".to_owned()));

    // Asking again closes nothing while that close is unresolved.
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p4"],
        true,
    );
    assert!(runtime.tree_closes.is_empty());
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tree_close.unresolved_close")
    );

    // Once the topology shows the pane gone, the retry closes the rest.
    shared(&mut runtime, &["w1:p1", second]);
    close_tree(&mut runtime, pane_target("w1:p1"), &[second], true);
    assert!(started(&runtime).contains(&second.to_owned()));
    shared(&mut runtime, &["w1:p1"]);
    assert!(started(&runtime).contains(&"w1:p1".to_owned()));
    shared(&mut runtime, &[]);
    assert!(runtime.tree_closes.is_empty());
}

#[test]
fn a_listed_pane_that_is_not_a_live_descendant_is_never_closed() {
    let mut runtime = tree_runtime(&[]);
    // p4 and p5 are another branch of p1, not below p2.
    close_tree(
        &mut runtime,
        pane_target("w1:p2"),
        &["w1:p3", "w1:p4", "w5:p5", "w1:p1"],
        true,
    );
    assert_eq!(started(&runtime), ["w1:p3"]);
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert_eq!(started(&runtime), ["w1:p2"]);
    leave(&mut runtime, &["w1:p1", "w1:p4", "w5:p5"]);
    assert!(runtime.tree_closes.is_empty());

    // A removal naming panes outside its checkout's descendants closes none
    // of them before it goes on.
    let project = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .find(|workspace| workspace.path == B)
        .unwrap()
        .id
        .clone();
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "remove_workspace",
        "payload": {"workspace_id": project, "close_descendant_pane_ids": ["w1:p1", "w1:p4"]}
    }))
    .unwrap();
    runtime.dispatch_json(&event);
    assert!(!started(&runtime).contains(&"w1:p1".to_owned()));
    assert!(!started(&runtime).contains(&"w1:p4".to_owned()));
}

#[test]
fn a_same_tab_sibling_waits_while_a_finished_close_queues_behind_a_running_one() {
    let mut runtime = tree_runtime(&[]);
    let shared = |runtime: &mut Runtime, present: &[&str]| {
        runtime.ingest_session_with_catalog(
            Ok(tree_payload_in(present, &[], true)),
            Some(tree_catalog()),
        );
    };
    shared(&mut runtime, &EVERY);
    // An ordinary close the operator started earlier holds the front of the
    // reservation queue while it runs.
    let event = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "close_pane",
        "payload": {"pane_id": "w5:p5", "confirmed": true}
    }))
    .unwrap();
    runtime.dispatch_json(&event);
    assert_eq!(started(&runtime), ["w5:p5"]);
    // p4 lists nothing below it here, so it starts beside p3; p2 shares p4's tab.
    close_tree(
        &mut runtime,
        pane_target("w1:p1"),
        &["w1:p2", "w1:p3", "w1:p4"],
        true,
    );
    assert!(started(&runtime).contains(&"w1:p4".to_owned()));
    shared(&mut runtime, &["w1:p1", "w1:p2", "w5:p5"]);
    // p4's close is done but queued behind p5's: p2 waits, it does not fail.
    runtime.tick_async_operations(unix_milliseconds());
    assert_eq!(tree_phase(&runtime, "w1:p2").as_deref(), Some("waiting"));
    shared(&mut runtime, &["w1:p1", "w1:p2"]);
    assert!(started(&runtime).contains(&"w1:p2".to_owned()));
    shared(&mut runtime, &["w1:p1"]);
    shared(&mut runtime, &[]);
    assert!(runtime.tree_closes.is_empty());
    assert_ne!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("tree_close.incomplete")
    );
}
