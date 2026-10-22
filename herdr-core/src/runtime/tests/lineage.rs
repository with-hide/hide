use super::*;

// PRD AC1-4: assertions describe the operator's tree and persisted choice,
// independently of how the projector builds its parent index.
fn lineage_rows() -> Vec<SidebarAgentSnapshot> {
    project_agents(crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
        {"id":"observer-control","pane_id":"parent","agent_status":"working","state_change_seq":1,"tokens":{"task":"Parent"}},
        {"id":"implementor-control","pane_id":"child","spawned_from_pane_id":"parent","agent_status":"working","state_change_seq":2,"tokens":{"task":"Child"}},
        {"id":"reviewer-control","pane_id":"grandchild","spawned_from_pane_id":"child","agent_status":"working","state_change_seq":3,"tokens":{"task":"Grandchild"}},
        {"id":"sibling-control","pane_id":"sibling","spawned_from_pane_id":"parent","agent_status":"working","state_change_seq":4,"tokens":{"task":"Newer sibling"}}
    ]})).unwrap()).agents
}

fn lineage_workspaces() -> Vec<WorkspaceSnapshot> {
    let main = checkout(
        "project",
        "main",
        "/fixture/main",
        Some(pane("parent", "/fixture/main")),
    );
    let mut feature = checkout(
        "project",
        "feature",
        "/fixture/feature",
        Some(pane("child", "/fixture/feature")),
    );
    feature.tabs[0].panes.extend([
        pane("grandchild", "/fixture/feature"),
        pane("sibling", "/fixture/feature"),
    ]);
    vec![workspace(
        "project",
        "Project",
        "/fixture/main",
        vec![main, feature],
    )]
}

fn remote_lineage_session(agents: Vec<SidebarAgentSnapshot>) -> RemoteSessionSnapshot {
    RemoteSessionSnapshot {
        workspaces: Vec::new(),
        agents,
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
    }
}

fn local_lineage_agent<'a>(runtime: &'a Runtime, pane: &str) -> &'a SidebarAgentSnapshot {
    runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .unwrap()
}

fn remote_lineage_agent<'a>(runtime: &'a Runtime, pane: &str) -> &'a SidebarAgentSnapshot {
    runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .unwrap()
}

/// A local pair and a pair on the device `mini`, each child declared by the
/// other machine's parent, with every machine identity known.
fn cross_machine_runtime() -> Runtime {
    let mut runtime = runtime();
    runtime.node = crate::node::NodeId::parse("machine-local").unwrap();
    runtime
        .device_machine_ids
        .insert("mini".to_owned(), "machine-mini".to_owned());

    let mut rows = lineage_rows();
    let mut local_parent = rows.remove(0);
    local_parent.pane_id = "local-parent".to_owned();
    local_parent.id = "local-parent-agent".to_owned();
    local_parent.declared_parent_pane_id = None;
    local_parent.spawned_from_pane_id = None;

    let mut local_child = rows.remove(0);
    local_child.pane_id = "local-child".to_owned();
    local_child.id = "local-child-agent".to_owned();
    local_child.declared_parent_pane_id = Some("remote-parent".to_owned());
    local_child.spawned_from_machine_id = Some("machine-mini".to_owned());

    let mut remote_parent = rows.remove(0);
    remote_parent.pane_id = "remote:mini:pane:remote-parent".to_owned();
    remote_parent.id = "remote-parent-agent".to_owned();
    remote_parent.declared_parent_pane_id = None;
    remote_parent.spawned_from_pane_id = None;

    let mut remote_child = rows.remove(0);
    remote_child.pane_id = "remote:mini:pane:remote-child".to_owned();
    remote_child.id = "remote-child-agent".to_owned();
    remote_child.declared_parent_pane_id = Some("local-parent".to_owned());
    remote_child.spawned_from_machine_id = Some("machine-local".to_owned());

    runtime.snapshot.navigator.agents = vec![local_parent, local_child];
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(remote_lineage_session(vec![remote_parent, remote_child])),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime
}

#[test]
fn machine_identity_resolves_lineage_in_both_directions_and_restores_after_reconnect() {
    let mut runtime = cross_machine_runtime();
    assert!(runtime.refresh_agent_lineage());
    assert_eq!(
        local_lineage_agent(&runtime, "local-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("remote:mini:pane:remote-parent")
    );
    assert_eq!(
        remote_lineage_agent(&runtime, "remote:mini:pane:remote-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("local-parent")
    );
    assert!(local_lineage_agent(&runtime, "local-child").delegated);
    assert!(remote_lineage_agent(&runtime, "remote:mini:pane:remote-child").delegated);

    runtime.device_machine_ids.remove("mini");
    assert!(runtime.refresh_agent_lineage());
    assert_eq!(
        local_lineage_agent(&runtime, "local-child").lineage_parent_pane_id,
        None
    );
    assert!(!local_lineage_agent(&runtime, "local-child").delegated);
    assert!(runtime.unresolved_machine_lineage.contains("local-child"));

    runtime
        .device_machine_ids
        .insert("mini".to_owned(), "machine-mini".to_owned());
    assert!(runtime.refresh_agent_lineage());
    assert_eq!(
        local_lineage_agent(&runtime, "local-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("remote:mini:pane:remote-parent")
    );
    assert!(runtime.unresolved_machine_lineage.is_empty());
}

/// The local child declared its remote parent for `s-remote-parent`, and the
/// remote child its local parent for `s-local-parent`; each pane runs the
/// session the relationship was written for.
fn attest_cross_machine_sessions(runtime: &mut Runtime) {
    let digest = |session: &str| crate::wire::session_digest(session);
    for agent in &mut runtime.snapshot.navigator.agents {
        match agent.pane_id.as_str() {
            "local-parent" => agent.lineage_session = digest("s-local-parent"),
            _ => {
                agent.lineage_session = digest("s-local-child");
                agent.declared_parent_session = digest("s-remote-parent");
            }
        }
    }
    for agent in &mut runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .agents
    {
        match agent.pane_id.as_str() {
            "remote:mini:pane:remote-parent" => agent.lineage_session = digest("s-remote-parent"),
            _ => {
                agent.lineage_session = digest("s-remote-child");
                agent.declared_parent_session = digest("s-local-parent");
            }
        }
    }
}

#[test]
fn a_cross_machine_relationship_holds_while_both_panes_run_the_sessions_it_was_written_for() {
    let mut runtime = cross_machine_runtime();
    attest_cross_machine_sessions(&mut runtime);
    runtime.refresh_agent_lineage();
    assert_eq!(
        local_lineage_agent(&runtime, "local-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("remote:mini:pane:remote-parent")
    );
    assert_eq!(
        remote_lineage_agent(&runtime, "remote:mini:pane:remote-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("local-parent")
    );
}

#[test]
fn a_parent_pane_on_another_machine_reused_by_another_agent_adopts_no_local_child() {
    let mut runtime = cross_machine_runtime();
    attest_cross_machine_sessions(&mut runtime);
    for agent in &mut runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .agents
    {
        if agent.pane_id == "remote:mini:pane:remote-parent" {
            agent.lineage_session = crate::wire::session_digest("s-another-agent");
        }
    }
    runtime.refresh_agent_lineage();

    let child = local_lineage_agent(&runtime, "local-child");
    assert_eq!(child.lineage_parent_pane_id, None);
    assert!(!child.delegated);
    assert!(
        !child.lineage_orphan,
        "a reused pane is not an ended parent"
    );
    assert_eq!(child.lineage_hint, None);
    let parent = remote_lineage_agent(&runtime, "remote:mini:pane:remote-parent");
    assert!(parent.lineage_child_pane_ids.is_empty());
    assert!(
        parent.descendant_counts == crate::model::DescendantCountsSnapshot::default(),
        "no badge for a child it never had"
    );
    assert_eq!(
        remote_lineage_agent(&runtime, "remote:mini:pane:remote-child")
            .lineage_parent_pane_id
            .as_deref(),
        Some("local-parent"),
        "the other direction is untouched"
    );
}

#[test]
fn a_cross_checkout_badge_names_the_branch_the_sidebar_shows() {
    // A device checkout keeps its folder name as its label and learns its
    // branch later; the badge reads as the sidebar does, branch first.
    let mut workspaces = lineage_workspaces();
    let feature = &mut workspaces[0].checkouts[1];
    feature.label = "sessions-full-tree".to_owned();
    feature.branch = Some("feat/sessions-full-tree-branch".to_owned());
    let mut rows = lineage_rows();
    let badge = |rows: &[SidebarAgentSnapshot]| {
        let child = rows.iter().find(|row| row.pane_id == "child").unwrap();
        child.lineage_worktree_badge.clone()
    };
    crate::agent_state::apply_lineage(&mut rows, &workspaces, &[]);
    assert_eq!(
        badge(&rows).as_deref(),
        Some("feat/sessions-full-tree-branch")
    );
    // With no branch (a detached HEAD, a plain folder) the label stands.
    workspaces[0].checkouts[1].branch = None;
    crate::agent_state::apply_lineage(&mut rows, &workspaces, &[]);
    assert_eq!(badge(&rows).as_deref(), Some("sessions-full-tree"));
}

#[test]
fn lineage_cross_checkout_tree_and_orphan_keep_the_canonical_rows_and_axes() {
    let mut rows = lineage_rows();
    let mut records = std::collections::BTreeMap::new();
    crate::agent_state::apply_read_state(
        &mut rows,
        &mut records,
        &mut crate::agent_state::ReadConnection::new("server".to_owned()),
        Some("child"),
    );
    let original = rows
        .iter()
        .map(|row| {
            (
                row.pane_id.clone(),
                row.demand.clone(),
                row.unread,
                row.group.clone(),
            )
        })
        .collect::<Vec<_>>();
    let ledger = records.clone();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let row = |id: &str| rows.iter().find(|row| row.pane_id == id).unwrap();
    assert_eq!(row("parent").lineage_child_pane_ids, ["sibling", "child"]);
    assert_eq!(row("child").lineage_depth, 1);
    assert_eq!(row("grandchild").lineage_depth, 2);
    assert_eq!(
        row("child").lineage_root_checkout_id.as_deref(),
        Some("main")
    );
    assert_eq!(
        row("child").lineage_worktree_badge.as_deref(),
        Some("feature")
    );
    assert_eq!(row("grandchild").lineage_worktree_badge, None);
    assert_eq!(
        original,
        rows.iter()
            .map(|row| (
                row.pane_id.clone(),
                row.demand.clone(),
                row.unread,
                row.group.clone()
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(ledger, records);
    rows.retain(|row| row.pane_id != "parent");
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let child = rows.iter().find(|row| row.pane_id == "child").unwrap();
    assert_eq!(child.lineage_depth, 0);
    assert_eq!(child.lineage_root_checkout_id.as_deref(), Some("feature"));
    assert!(child.lineage_orphan);
    // No agent is listed at that pane, so there is no name to give and the
    // line says only that. It used to read "↳ from parent", which looked
    // like a name and was an internal handle.
    assert_eq!(
        child.lineage_hint.as_deref(),
        Some("↳ from an agent Hide can't see")
    );
    assert_eq!(child.spawn_origin_pane_id, None);
}

#[test]
fn lineage_depth_is_not_capped_and_cycles_are_visible_orphans() {
    let mut values = Vec::new();
    for index in 0..64 {
        values.push(
            serde_json::json!({"pane_id": format!("p{index}"), "state_change_seq": index,
            "spawned_from_pane_id": (index > 0).then(|| format!("p{}", index - 1))}),
        );
    }
    let mut rows = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents":values})).unwrap(),
    )
    .agents;
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(rows[63].lineage_depth, 63);
    rows[0].spawned_from_pane_id = Some("p1".into());
    rows[0].declared_parent_session = rows[1].declared_parent_session.clone();
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert!(rows[0].lineage_orphan && rows[1].lineage_orphan);
    assert_eq!(rows[2].lineage_depth, 1);
}

// PRD B9, D-06, D-11: a parent starts folded, the operator's opening of it
// is what persists, a child's question does not open it, and the entry
// leaves with the pane.
#[test]
fn lineage_expansion_persists_without_attention_opening_it_and_prunes_on_disappearance() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = lineage_workspaces();
    runtime.snapshot.navigator.agents = lineage_rows();
    runtime.refresh_agent_lineage();
    let collapsed = |runtime: &Runtime, id: &str| {
        runtime
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|row| row.pane_id == id)
            .unwrap()
            .lineage_collapsed
    };
    assert!(
        collapsed(&runtime, "parent") && collapsed(&runtime, "child"),
        "every parent starts folded"
    );
    assert!(runtime.dispatch_json(
        br#"{"schema_version":2,"kind":"agent_tree_toggle","payload":{"pane_id":"parent"}}"#
    ));
    assert!(!collapsed(&runtime, "parent"));
    assert!(
        collapsed(&runtime, "child"),
        "opening the parent opens one level"
    );
    assert_eq!(
        runtime.snapshot.ui_state.expanded_agent_pane_ids,
        ["parent"]
    );
    let (restored, _, disposition) = persistence::load(&runtime.state_path);
    assert_eq!(disposition, persistence::LoadDisposition::Loaded);
    assert_eq!(restored.expanded_agent_pane_ids, ["parent"]);
    let options = CoreOptions {
        schema_version: SCHEMA_VERSION,
        home: None,
        node_id: crate::node::test_node(),
        herdr_socket_path: None,
        herdr_bin_path: None,
        app_state_path: runtime.state_path.to_string_lossy().into_owned(),
        host_helper_root: None,
        host_cli_dir: None,
        workspace_views_path: None,
        shortcut_import_path: None,
        local_issues_path: None,
        effects_held: false,
        machine_name: None,
        account_state_folder: false,
    };
    let restarted = Runtime::new(
        options,
        environment::EnvironmentReport {
            statuses: vec![],
            home_path: None,
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    );
    assert_eq!(
        restarted.snapshot.ui_state.expanded_agent_pane_ids,
        ["parent"]
    );
    let mut attention = lineage_rows();
    attention
        .iter_mut()
        .find(|row| row.pane_id == "child")
        .unwrap()
        .demand = "question".into();
    crate::agent_state::apply_lineage(&mut attention, &lineage_workspaces(), &[]);
    crate::agent_state::apply_read_state(
        &mut attention,
        &mut runtime.snapshot.ui_state.pane_read_records,
        &mut crate::agent_state::ReadConnection::new("server".to_owned()),
        None,
    );
    assert!(
        attention
            .iter()
            .find(|row| row.pane_id == "parent")
            .unwrap()
            .lineage_collapsed,
        "a child's question does not open the parent; the badge reports it"
    );
    // PRD B15, D-35: the child's question is the parent's problem. It keeps
    // the hint that says whose child asked it, and it stays out of the
    // operator's Needs You, which is what makes the delegation real.
    assert!(
        attention.iter().all(|row| row.group != "needs_you"),
        "a delegated child's question never raises the operator's own group"
    );
    let asking = attention
        .iter()
        .find(|row| row.pane_id == "child")
        .expect("the child is still a row");
    assert_eq!(asking.demand, "question");
    assert_eq!(asking.raised_hint.as_deref(), Some("↳ from Parent"));
    assert!(asking.delegated && !asking.emphasized);
    assert_eq!(
        runtime.snapshot.ui_state.expanded_agent_pane_ids,
        ["parent"]
    );
    let snapshot_state = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
    assert!(
        runtime.dispatch_json(
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": 2, "kind": "ui_state_update", "payload": snapshot_state
            }))
            .unwrap()
        )
    );
    assert_eq!(
        runtime.snapshot.ui_state.expanded_agent_pane_ids,
        ["parent"]
    );
    let mut scoped = vec!["parent".to_owned(), "remote:mini:p1".to_owned()];
    assert!(!crate::agent_state::prune_lineage_expansion(
        &mut scoped,
        &[],
        ReadRecordScope::Retain
    ));
    assert!(crate::agent_state::prune_lineage_expansion(
        &mut scoped,
        &[],
        ReadRecordScope::Local
    ));
    assert_eq!(scoped, ["remote:mini:p1"]);
    // Fresh ingestion is the production cleanup boundary, not the toggle.
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(
        serde_json::json!({"agents":[]}),
    )
    .unwrap()));
    assert!(runtime.snapshot.ui_state.expanded_agent_pane_ids.is_empty());
    let (restored, _, _) = persistence::load(&runtime.state_path);
    assert!(restored.expanded_agent_pane_ids.is_empty());
    let _ = std::fs::remove_file(&runtime.state_path);
}

// PRD B11, B13, B14, D-36, D-52: ownership is the fourth derived axis, and it
// is read off the lineage rather than stored anywhere.
#[test]
fn ownership_marks_descendants_delegated_and_hands_an_orphan_back_to_the_operator() {
    let mut rows = lineage_rows();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let row = |rows: &[SidebarAgentSnapshot], id: &str| {
        rows.iter().find(|row| row.pane_id == id).unwrap().clone()
    };
    assert!(
        !row(&rows, "parent").delegated,
        "a lineage root is the operator's own work"
    );
    assert!(row(&rows, "child").delegated);
    assert!(row(&rows, "grandchild").delegated);
    // The delegation source the row already carried is what names the owner.
    assert_eq!(
        row(&rows, "child").raised_hint.as_deref(),
        Some("↳ from Parent")
    );

    rows.retain(|row| row.pane_id != "parent");
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let orphan = row(&rows, "child");
    assert!(
        !orphan.delegated,
        "a lost parent returns ownership to the operator"
    );
    assert!(orphan.lineage_orphan);
    assert!(
        row(&rows, "grandchild").delegated,
        "its own descendants stay delegated"
    );
}

#[test]
fn a_lost_origin_is_named_as_ended_rather_than_shown_as_a_pane_id() {
    let mut rows = lineage_rows();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let row = |rows: &[SidebarAgentSnapshot], id: &str| {
        rows.iter().find(|row| row.pane_id == id).unwrap().clone()
    };
    // While the origin is an agent Hide can see, the line names it and
    // carries the pane, which is what the shell turns into a jump.
    let child = row(&rows, "child");
    assert_eq!(child.raised_hint.as_deref(), Some("↳ from Parent"));
    assert_eq!(child.spawn_origin_pane_id.as_deref(), Some("parent"));

    // Once Hide cannot see that agent the line says only that, and not that
    // it ended - which the projection has no way to know. A pane id is an
    // internal handle, never rendered as a name; with nothing to go to, no
    // destination is offered either.
    rows.retain(|row| row.pane_id != "parent");
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let orphan = row(&rows, "child");
    assert_eq!(
        orphan.lineage_hint.as_deref(),
        Some("↳ from an agent Hide can't see")
    );
    assert_eq!(orphan.spawn_origin_pane_id, None);
    for row in &rows {
        let shown = row.lineage_hint.iter().chain(row.raised_hint.iter());
        for hint in shown {
            assert!(
                !hint.contains("parent"),
                "a pane id never reaches the operator as a name: {hint}"
            );
        }
    }
}

// PRD B7, B9, B10, D-18, D-19: the breadcrumb and its per-step sibling list
// are derived every time from the lineage, so nothing can go stale.
#[test]
fn the_breadcrumb_path_and_each_steps_siblings_come_out_of_the_lineage() {
    let mut rows = lineage_rows();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let row = |id: &str| rows.iter().find(|row| row.pane_id == id).unwrap();

    assert_eq!(row("parent").lineage_path_pane_ids, Vec::<String>::new());
    assert_eq!(row("child").lineage_path_pane_ids, ["parent"]);
    assert_eq!(row("grandchild").lineage_path_pane_ids, ["parent", "child"]);
    assert_eq!(
        row("grandchild").lineage_parent_pane_id.as_deref(),
        Some("child")
    );

    // A step's dropdown offers that layer, in the parent's own child order,
    // and includes the step itself so the current position is visible.
    assert_eq!(row("child").lineage_sibling_pane_ids, ["sibling", "child"]);
    assert_eq!(
        row("sibling").lineage_sibling_pane_ids,
        ["sibling", "child"]
    );
    assert_eq!(row("grandchild").lineage_sibling_pane_ids, ["grandchild"]);
    assert_eq!(
        row("parent").lineage_sibling_pane_ids,
        Vec::<String>::new(),
        "the layer above a root is the sidebar, not the breadcrumb"
    );
}

#[test]
fn lineage_identity_uses_tasks_even_when_herdr_names_exist() {
    let mut rows = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
            {
                "pane_id":"w1:p1",
                "agent_status":"working",
                "state_change_seq":1,
                "workspace_label":"Workspace",
                "tokens":{"task":"Project coordinator"}
            },
            {
                "pane_id":"w1:p2",
                "spawned_from_pane_id":"w1:p1",
                "agent_status":"working",
                "state_change_seq":2,
                "tokens":{"task":"Hide design QA"}
            }
        ]}))
        .unwrap(),
    )
    .agents;
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    let child = rows.iter().find(|row| row.pane_id == "w1:p2").unwrap();

    assert_eq!(
        child.raised_hint.as_deref(),
        Some("↳ from Project coordinator")
    );
    assert_eq!(child.spawn_origin_pane_id.as_deref(), Some("w1:p1"));
    let path = crate::sidebar::project_lineage_path(&rows, "w1:p2");
    assert_eq!(path[0].label, "Project coordinator");
    assert_eq!(path[1].label, "Hide design QA");
    assert_eq!(crate::sidebar::agent_chip(child).label, path[1].label);
    assert_eq!(child.identity_label, "Hide design QA");
    let parent = rows.iter().find(|row| row.pane_id == "w1:p1").unwrap();
    assert_eq!(parent.identity_label, "Project coordinator");
    assert_eq!(
        serde_json::to_value(parent).unwrap()["identity_label"],
        "Project coordinator"
    );
}

// PRD D-18: the path is a function of the current list, so a child exiting
// leaves every surviving row's breadcrumb correct with no stored state to fix.
#[test]
fn a_departed_ancestor_shortens_every_descendants_path_on_the_next_projection() {
    let mut rows = lineage_rows();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    assert_eq!(
        rows.iter()
            .find(|row| row.pane_id == "grandchild")
            .unwrap()
            .lineage_path_pane_ids,
        ["parent", "child"]
    );
    rows.retain(|row| row.pane_id != "child");
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let grandchild = rows.iter().find(|row| row.pane_id == "grandchild").unwrap();
    assert_eq!(grandchild.lineage_path_pane_ids, Vec::<String>::new());
    assert_eq!(grandchild.lineage_parent_pane_id, None);
    assert!(
        !grandchild.delegated,
        "an orphaned grandchild is a root of its own"
    );
}

// PRD B5, B21-B24, B32, D-30, D-63: what a pane header is allowed to say
// about the work its agent delegated.
fn hook_status_of(
    status: Option<hide_agent_hooks::HookStatus>,
) -> impl Fn(hide_agent_adapter::HookDialect) -> Option<hide_agent_hooks::HookStatus> {
    move |_| status.clone()
}

fn instrumented_rows(agent_kind: &str) -> Vec<SidebarAgentSnapshot> {
    let mut rows = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
            {"id":"Observer","pane_id":"parent","agent":agent_kind,"agent_status":"working","state_change_seq":1,
             "tokens":{"task":"Coordinate delegated work"}},
            {"id":"Worker","pane_id":"child","agent":agent_kind,"spawned_from_pane_id":"parent",
             "agent_status":"idle","state_change_seq":2,
             "tokens":{"status_question_new":"?","task":"Resolve hook failure"}},
            {"id":"Runner","pane_id":"sibling","agent":agent_kind,"spawned_from_pane_id":"parent",
             "agent_status":"working","state_change_seq":3,
             "tokens":{"task":"Run verification"}}
        ]}))
        .unwrap(),
    )
    .agents;
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    rows
}

#[test]
fn a_pane_with_children_lists_them_and_names_the_one_that_speaks_for_them() {
    let rows = instrumented_rows("claude");
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    let tokens = crate::agent_hooks::PaneHookTokens {
        version: Some(hide_agent_hooks::HOOK_VERSION),
        working: Some(2),
        done: Some(4),
        blocked: None,
        session_digest: None,
    };
    let children =
        crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed).unwrap();

    assert!(children.instrumented);
    assert_eq!(children.uninstrumented_reason, None);
    let labels: Vec<_> = children
        .chips
        .iter()
        .map(|chip| chip.label.as_str())
        .collect();
    assert_eq!(
        labels,
        ["Run verification", "Resolve hook failure"],
        "chips follow the lineage's own child order"
    );
    assert!(children.chips.iter().all(|chip| chip.delegated));
    // An unread question outranks a working sibling, by the same rule the
    // Workspace summary chip uses.
    assert_eq!(
        children.representative.as_ref().unwrap().label,
        "Resolve hook failure"
    );
    // The question is the parent's to answer, so the child draws none of
    // the operator's marks (status-marks B24).
    assert_eq!(
        children.representative.as_ref().unwrap().mark,
        crate::agent_state::tally::RowMark::Quiet
    );

    // In-process subagents are summarised separately and never folded into
    // the chip count.
    assert_eq!(children.subagents.working, Some(2));
    assert_eq!(children.subagents.done, Some(4));
    assert_eq!(children.subagents.blocked, None);
    assert_eq!(
        children.chips.len(),
        2,
        "two pane children, whatever the subagent count says"
    );
}

#[test]
fn internal_work_hook_totals_follow_the_reported_session_when_a_pane_is_reused() {
    use hide_agent_adapter::work::{WorkRead, WorkTotals};
    let mut rows = instrumented_rows("claude");
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    let digest = |session: &str| -> [u8; 64] {
        crate::wire::session_digest(session)
            .unwrap()
            .as_bytes()
            .try_into()
            .unwrap()
    };
    let mut tokens = crate::agent_hooks::PaneHookTokens {
        version: Some(hide_agent_hooks::HOOK_VERSION),
        working: Some(3),
        done: Some(9),
        blocked: None,
        session_digest: Some(digest("session-a")),
    };
    rows[0].lineage_session = crate::wire::session_digest("session-a");
    let before =
        crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed).unwrap();
    assert_eq!(
        before.internal_work.subagents.current(),
        Some(&WorkTotals {
            working: Some(3),
            done: Some(9)
        })
    );

    // Herdr has moved to B while A's pane tokens remain after an ignored start.
    rows[0].lineage_session = crate::wire::session_digest("session-b");
    let replaced =
        crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed).unwrap();
    assert_eq!(replaced.internal_work.subagents, WorkRead::Unknown);
    assert_eq!(
        (replaced.subagents.working, replaced.subagents.done),
        (Some(3), Some(9)),
        "legacy fields stay compatible"
    );
    assert_eq!(
        replaced.chips, before.chips,
        "a count never invents lineage"
    );
    tokens.session_digest = Some(digest("session-b"));
    tokens.working = Some(0);
    tokens.done = None;
    let current =
        crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed).unwrap();
    assert_eq!(
        current.internal_work.subagents.current(),
        Some(&WorkTotals {
            working: Some(0),
            done: None
        })
    );
    tokens.done = Some(0);
    let empty = crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed).unwrap();
    assert_eq!(
        empty.internal_work.subagents.current(),
        Some(&WorkTotals {
            working: Some(0),
            done: Some(0)
        })
    );
    tokens.session_digest = None;
    assert_eq!(
        crate::sidebar::project_pane_children(&rows, "parent", tokens, &installed)
            .unwrap()
            .internal_work
            .subagents,
        WorkRead::Unknown
    );
}

#[test]
fn internal_work_counts_do_not_invent_execution_records_or_lineage() {
    use hide_agent_adapter::work::{WorkRead, WorkSource, WorkTotals};
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    for kind in ["claude", "codex", "grok", "opencode", "omp", "agy", "pi"] {
        let rows = instrumented_rows(kind);
        let owner = (!matches!(kind, "grok" | "omp")).then(|| {
            crate::sidebar::fixture_lineage_session()
                .as_bytes()
                .try_into()
                .unwrap()
        });
        let counts = crate::sidebar::project_pane_children(
            &rows,
            "parent",
            crate::agent_hooks::PaneHookTokens {
                version: Some(hide_agent_hooks::HOOK_VERSION),
                working: Some(7),
                done: Some(3),
                blocked: None,
                session_digest: owner,
            },
            &installed,
        )
        .unwrap();
        if kind == "pi" {
            assert_eq!(counts.internal_work.subagents, WorkRead::Unsupported);
        } else if owner.is_none() {
            assert_eq!(counts.internal_work.subagents, WorkRead::Unknown);
        } else {
            assert_eq!(
                counts.internal_work.subagents,
                WorkRead::Current {
                    source: WorkSource::Hooks,
                    value: WorkTotals {
                        working: Some(7),
                        done: Some(3)
                    },
                },
                "{kind}"
            );
        }
        assert_eq!(
            counts.internal_work.executions,
            if kind == "claude" {
                WorkRead::Unknown
            } else {
                WorkRead::Unsupported
            },
            "{kind}"
        );
        assert_eq!(
            counts.chips.len(),
            2,
            "native work is never another pane child"
        );
        let absent = crate::sidebar::project_pane_children(
            &rows,
            "parent",
            crate::agent_hooks::PaneHookTokens::default(),
            &installed,
        )
        .unwrap();
        assert_eq!(
            absent.internal_work.subagents,
            if kind == "pi" {
                WorkRead::Unsupported
            } else {
                WorkRead::Unknown
            }
        );
        let zero = crate::sidebar::project_pane_children(
            &rows,
            "parent",
            crate::agent_hooks::PaneHookTokens {
                version: Some(hide_agent_hooks::HOOK_VERSION),
                working: Some(0),
                done: Some(0),
                blocked: None,
                session_digest: owner,
            },
            &installed,
        )
        .unwrap();
        if kind != "pi" && owner.is_some() {
            assert_eq!(
                zero.internal_work.subagents.current(),
                Some(&WorkTotals {
                    working: Some(0),
                    done: Some(0)
                })
            );
            let partial = crate::sidebar::project_pane_children(
                &rows,
                "parent",
                crate::agent_hooks::PaneHookTokens {
                    version: Some(hide_agent_hooks::HOOK_VERSION),
                    working: Some(0),
                    done: None,
                    blocked: None,
                    session_digest: owner,
                },
                &installed,
            )
            .unwrap();
            assert_eq!(
                partial.internal_work.subagents.current(),
                Some(&WorkTotals {
                    working: Some(0),
                    done: None
                })
            );
        }
    }
}

#[test]
fn an_instrumented_pane_with_no_children_is_a_different_answer_from_one_hide_cannot_see() {
    let rows = instrumented_rows("claude");
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    let reported = crate::agent_hooks::PaneHookTokens {
        version: Some(hide_agent_hooks::HOOK_VERSION),
        working: Some(0),
        done: Some(0),
        blocked: None,
        session_digest: None,
    };
    let alone =
        crate::sidebar::project_pane_children(&rows, "child", reported, &installed).unwrap();
    assert!(alone.instrumented);
    assert!(alone.chips.is_empty());
    assert!(alone.subagents.is_silent());
    assert_eq!(alone.uninstrumented_reason, None);

    // The same pane before its session ever ran the hook.
    let silent = crate::sidebar::project_pane_children(
        &rows,
        "child",
        crate::agent_hooks::PaneHookTokens::default(),
        &installed,
    )
    .unwrap();
    assert!(!silent.instrumented);
    assert!(
        silent
            .uninstrumented_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Restart the agent")),
        "the operator is told which of the reasons applies"
    );
    assert!(
        silent.subagents.working.is_none(),
        "an unknown count is never a zero"
    );

    // The hook only ever answered for in-process subagents. Pane children come
    // from Herdr's lineage, so an uninstrumented parent still lists them.
    let uninstrumented_parent = crate::sidebar::project_pane_children(
        &rows,
        "parent",
        crate::agent_hooks::PaneHookTokens::default(),
        &installed,
    )
    .unwrap();
    assert!(!uninstrumented_parent.instrumented);
    assert_eq!(uninstrumented_parent.chips.len(), 2);
    assert!(uninstrumented_parent.subagents.working.is_none());
}

#[test]
fn a_pane_with_no_agent_gets_neither_chips_nor_a_mark() {
    let rows = instrumented_rows("claude");
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    assert!(
        crate::sidebar::project_pane_children(
            &rows,
            "a-plain-shell",
            crate::agent_hooks::PaneHookTokens::default(),
            &installed,
        )
        .is_none()
    );
}

#[test]
fn an_agent_hide_has_no_adapter_for_says_so_instead_of_guessing_a_runtime() {
    let rows = instrumented_rows("gemini");
    let children = crate::sidebar::project_pane_children(
        &rows,
        "parent",
        crate::agent_hooks::PaneHookTokens::default(),
        &hook_status_of(None),
    )
    .unwrap();
    assert!(!children.instrumented);
    assert_eq!(
        children.uninstrumented_reason.as_deref(),
        Some("Child information is unavailable for this pane.")
    );
    assert!(
        children.uninstrumented_label.is_some(),
        "the mark carries an accessible name"
    );
}

#[test]
fn the_breadcrumb_is_the_ancestors_then_the_pane_with_each_layers_siblings() {
    let rows = instrumented_rows("claude");
    assert!(
        crate::sidebar::project_lineage_path(&rows, "parent").is_empty(),
        "a root has nowhere to go back to"
    );
    let path = crate::sidebar::project_lineage_path(&rows, "child");
    assert_eq!(
        path.iter()
            .map(|step| step.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["parent", "child"]
    );
    assert!(path[0].siblings.is_empty());
    assert_eq!(
        path[1]
            .siblings
            .iter()
            .map(|s| s.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["sibling", "child"],
        "the step's dropdown offers that layer, including where the operator is"
    );
}

// PRD B1, B3, B4, D-15, D-43, D-44: the canvas keeps one pane, and the tab
// holding the delegated child never reaches the strip.
fn split_lineage_payload() -> crate::sidebar::SessionSnapshotPayload {
    crate::sidebar::owned_label_fixture(split_lineage_json()).expect("session payload")
}

/// A parent and its delegated child split in one tab, as Herdr reports them
/// before Hide moves the child.
pub(super) fn split_lineage_json() -> serde_json::Value {
    crate::sidebar::lineage_fixture_value(serde_json::json!({
        "agents": [
            {"id":"Observer","pane_id":"w1:p1","agent":"claude","agent_status":"working",
             "state_change_seq":1,"cwd":"/fixture","workspace_label":"Fixture",
             "tokens":{"task":"Coordinate implementation"}},
            {"id":"Implementor","pane_id":"w1:p2","agent":"claude","agent_status":"working",
             "state_change_seq":2,"cwd":"/fixture","workspace_label":"Fixture",
             "spawned_from_pane_id":"w1:p1","tokens":{"task":"Implement approved change"}}
        ],
        "panes": [
            {"pane_id":"w1:p1","cwd":"/fixture"},
            {"pane_id":"w1:p2","cwd":"/fixture"}
        ],
        "tabs": [{"workspace_id":"w1","tab_id":"t1","label":""}],
        "layouts": [{
            "workspace_id":"w1","tab_id":"t1","zoomed":false,
            "area":{"x":0,"y":0,"width":80,"height":24},
            "focused_pane_id":"w1:p1",
            "panes":[
                {"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":40,"height":24}},
                {"pane_id":"w1:p2","rect":{"x":40,"y":0,"width":40,"height":24}}
            ],
            "splits":[{"direction":"right","ratio":0.5,
                       "rect":{"x":0,"y":0,"width":80,"height":24}}]
        }]
    }))
}

#[test]
fn a_tab_holding_only_delegated_children_is_kept_off_the_strip() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(split_lineage_payload()));

    let tabs = runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .collect::<Vec<_>>();
    assert!(!tabs.is_empty(), "the fixture placed its tab");
    // The parent and the child share one tab here, so that tab is still the
    // operator's and stays on the strip; the move is what separates them.
    assert!(
        tabs.iter().all(|tab| !tab.delegated),
        "a tab holding the operator's own agent is never hidden"
    );

    // Once the child has its own tab, that tab holds nothing but delegated
    // work and leaves the strip while staying in the checkout.
    for workspace in &mut runtime.snapshot.navigator.workspaces {
        for checkout in &mut workspace.checkouts {
            let child = checkout.tabs[0]
                .panes
                .iter()
                .position(|pane| pane.id == "w1:p2")
                .map(|index| checkout.tabs[0].panes.remove(index));
            if let Some(child) = child {
                let mut moved = checkout.tabs[0].clone();
                moved.id = Some("t2".to_owned());
                moved.panes = vec![child];
                checkout.tabs.push(moved);
            }
        }
    }
    runtime.sync_pane_lineage();

    let checkout = runtime.snapshot.navigator.workspaces[0].checkouts[0].clone();
    let child_tab = checkout
        .tabs
        .iter()
        .find(|tab| tab.id.as_deref() == Some("t2"))
        .expect("the child's own tab");
    assert!(child_tab.delegated);
    assert!(
        !checkout.strip.iter().any(|entry| entry.source_id == "t2"),
        "the delegated tab is off the strip"
    );
    assert!(
        checkout
            .tabs
            .iter()
            .any(|tab| tab.id.as_deref() == Some("t2")),
        "and still in the checkout, so the sidebar and breadcrumb reach it"
    );
}

/// PRD D-16: a strip entry is named after its one agent, and a tab with two
/// agents or none keeps the Herdr label alone.
#[test]
fn a_strip_entry_carries_its_one_agents_identity_and_mark() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(split_lineage_payload()));

    let entry = |runtime: &Runtime, tab_id: &str| {
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .strip
            .iter()
            .find(|entry| entry.source_id == tab_id)
            .cloned()
    };
    // Two agents share the tab: the label stays what Herdr said.
    assert_eq!(
        entry(&runtime, "t1")
            .expect("the tab is on the strip")
            .agent_identity,
        None
    );

    // Move the child to its own tab: the parent's tab now holds one agent.
    for workspace in &mut runtime.snapshot.navigator.workspaces {
        for checkout in &mut workspace.checkouts {
            let child = checkout.tabs[0]
                .panes
                .iter()
                .position(|pane| pane.id == "w1:p2")
                .map(|index| checkout.tabs[0].panes.remove(index));
            if let Some(child) = child {
                let mut moved = checkout.tabs[0].clone();
                moved.id = Some("t2".to_owned());
                moved.panes = vec![child];
                checkout.tabs.push(moved);
            }
        }
    }
    runtime.sync_pane_lineage();
    let identity = entry(&runtime, "t1")
        .expect("the parent's tab")
        .agent_identity
        .expect("one agent names the tab");
    assert_eq!(identity.label, "Coordinate implementation");
    assert_eq!(identity.pane_id, "w1:p1");
    assert_eq!(identity.mark, crate::agent_state::tally::RowMark::Going);
    assert_eq!(identity.activity, "working");

    // A tab with no agent pane at all says nothing about agents.
    for workspace in &mut runtime.snapshot.navigator.workspaces {
        for checkout in &mut workspace.checkouts {
            let mut shell = checkout.tabs[0].clone();
            shell.id = Some("t3".to_owned());
            shell.panes = vec![];
            checkout.tabs.push(shell);
        }
    }
    runtime.rebuild_tab_strips();
    assert_eq!(
        entry(&runtime, "t3").expect("the shell tab").agent_identity,
        None
    );
    assert_eq!(
        entry(&runtime, "t1")
            .unwrap()
            .agent_identity
            .map(|chip| chip.label),
        Some("Coordinate implementation".to_owned()),
        "a rebuilt strip is named again before it is published"
    );
}

#[test]
fn moving_a_delegated_child_asks_for_a_new_tab_in_the_workspace_it_is_already_in() {
    let params = crate::wire::pane_move_to_new_tab_params("w1:p2", "w1", "Implementor")
        .expect("params encode");
    assert_eq!(
        params,
        serde_json::json!({
            "pane_id": "w1:p2",
            "destination": {"type": "new_tab", "workspace_id": "w1", "label": "Implementor"},
            "focus": false
        }),
        "a new workspace would split one checkout into two rows for one path"
    );
}

#[test]
fn a_move_herdr_declined_is_an_error_rather_than_a_silent_success() {
    // Herdr reports a refusal as an unchanged move with a reason, not as an
    // error, so the decision has to read `changed` rather than assume that a
    // successful request moved anything.
    let refused = crate::wire::move_outcome(false, Some("SameTab".to_owned()), None)
        .expect_err("a refusal is not a move");
    assert!(
        refused.contains("declined") && refused.contains("SameTab"),
        "got {refused}"
    );
    assert!(
        crate::wire::move_outcome(false, None, Some("t2".to_owned())).is_err(),
        "an unchanged move is a refusal even when a tab id came back"
    );
    assert_eq!(
        crate::wire::move_outcome(true, None, Some("t2".to_owned())).unwrap(),
        "t2"
    );
    assert!(
        crate::wire::move_outcome(true, None, None).is_err(),
        "a move with no tab to show for it is not a success"
    );
}

// PRD B15, B16, D-35, D-38: delegation is only real if the operator stops
// being called for the delegated work.
#[test]
fn a_delegated_childs_demand_and_completion_stay_off_the_operators_groups() {
    let mut rows = lineage_rows();
    for row in rows.iter_mut() {
        match row.pane_id.as_str() {
            // Asking a question, unread.
            "child" => {
                row.demand = "question".to_owned();
                row.activity = "stopped".to_owned();
                row.unread = true;
            }
            // Finished, unread: the operator's own row here would be Done.
            "grandchild" => {
                row.demand = "none".to_owned();
                row.activity = "stopped".to_owned();
                row.unread = true;
            }
            _ => {}
        }
    }
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let row = |id: &str| {
        rows.iter()
            .find(|row| row.pane_id == id)
            .unwrap_or_else(|| panic!("{id} is a row"))
    };

    assert_eq!(
        row("child").group,
        "seen",
        "a child's question is its parent's"
    );
    assert_eq!(
        row("grandchild").group,
        "seen",
        "a delegated completion is not the operator's Done"
    );
    assert!(!row("child").emphasized && !row("grandchild").emphasized);
    // The axes themselves are untouched, so the parent's badge can still read
    // the question off the row it points at.
    assert_eq!(row("child").demand, "question");
    assert_eq!(row("child").mark, crate::agent_state::tally::RowMark::Quiet);
    // The operator's own root keeps its group and its brightness.
    assert_eq!(row("parent").group, "working");
    assert!(!row("parent").delegated);
}

/// One ingest of a root and its descendants, each with the state the test
/// names. The parent's tab and the children's tabs are all in one checkout,
/// which is the ordinary shape sasu's dispatch leaves behind.
fn ingest_lineage(runtime: &mut Runtime, rows: &[(&str, Option<&str>, &str, &str)]) {
    let agents = rows
        .iter()
        .map(|(pane, parent, status, token)| {
            let mut tokens = serde_json::json!({"task": format!("Task {pane}")});
            if !token.is_empty() {
                tokens[*token] = serde_json::json!("x");
            }
            let mut agent = serde_json::json!({
                "id": format!("agent-{pane}"), "pane_id": pane, "agent": "claude",
                "agent_status": status, "state_change_seq": 1, "cwd": "/fixture",
                "workspace_label": "hide", "tokens": tokens
            });
            if let Some(parent) = parent {
                agent["spawned_from_pane_id"] = serde_json::json!(parent);
            }
            agent
        })
        .collect::<Vec<_>>();
    let panes = rows
        .iter()
        .map(|(pane, _, _, _)| serde_json::json!({"pane_id": pane, "cwd": "/fixture"}))
        .collect::<Vec<_>>();
    let tabs = rows
        .iter()
        .enumerate()
        .map(|(index, _)| serde_json::json!({"workspace_id":"w1","tab_id":format!("t{index}"),"label":""}))
        .collect::<Vec<_>>();
    let layouts = rows
        .iter()
        .enumerate()
        .map(|(index, (pane, _, _, _))| {
            serde_json::json!({
                "workspace_id":"w1","tab_id":format!("t{index}"),"zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":pane,
                "panes":[{"pane_id":pane,"rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            })
        })
        .collect::<Vec<_>>();
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents, "panes": panes, "tabs": tabs, "layouts": layouts
    }))
    .expect("session payload")));
}

fn agent_row<'a>(runtime: &'a Runtime, pane: &str) -> &'a SidebarAgentSnapshot {
    runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|row| row.pane_id == pane)
        .unwrap_or_else(|| panic!("{pane} is a row"))
}

/// The operator looks at one pane: the same path a sidebar click takes.
fn look_at(runtime: &mut Runtime, pane: &str) {
    runtime.operator_focused_pane_id = Some(pane.to_owned());
    runtime.refresh_pane_read_state();
    runtime.operator_focused_pane_id = None;
}

// PRD B5, B6, B8, D-03: a descendant asking, changing its ask, or finishing
// turns every ancestor unread; starting work, being answered, or closing
// does not; and reading the ancestor settles it until the next such change.
#[test]
fn a_descendants_demand_or_completion_turns_every_ancestor_unread_and_nothing_else_does() {
    let mut runtime = runtime();
    // Root working, child working, grandchild working: nothing to report.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "working", ""),
        ],
    );
    look_at(&mut runtime, "w1:p1");
    look_at(&mut runtime, "w1:p2");
    assert!(!agent_row(&runtime, "w1:p1").unread && !agent_row(&runtime, "w1:p2").unread);

    // The grandchild asks: both ancestors light up while their own axes
    // stay exactly where they were, so the root is an unread Working row
    // and never Needs You, and the delegated middle row stays Working.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "idle", "status_question_new"),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.unread, "a grandchild's question is news to the root");
    assert_eq!(
        root.group, "working",
        "the root's own axes decide its group"
    );
    assert_eq!(root.descendant_counts.question, 1);
    let middle = agent_row(&runtime, "w1:p2");
    assert!(middle.unread && middle.delegated);
    assert_eq!(
        middle.group, "working",
        "a delegated ancestor never leaves Working/Seen"
    );
    look_at(&mut runtime, "w1:p1");
    assert!(!agent_row(&runtime, "w1:p1").unread);

    // The question becomes an approval: a change between the three demands
    // is news again.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "blocked", ""),
        ],
    );
    assert!(agent_row(&runtime, "w1:p1").unread);
    assert_eq!(agent_row(&runtime, "w1:p1").descendant_counts.approval, 1);
    look_at(&mut runtime, "w1:p1");

    // Answered and back at work: the badge changes, the read axis does not.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(!root.unread, "a resolved demand is not news");
    assert_eq!(
        (
            root.descendant_counts.approval,
            root.descendant_counts.working
        ),
        (0, 2)
    );

    // Finished: news, and the root's own completion axis is untouched, so
    // it is an unread Working row and never Done on a child's account.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "done", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.unread, "a completion is news");
    assert_eq!(
        root.group, "working",
        "a child's completion is not the root's Done"
    );
    assert_eq!(root.descendant_counts.done, 1);
    look_at(&mut runtime, "w1:p1");

    // A second turn and a second completion: news again, because the
    // earlier completion left the record when the child went back to work.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "working", ""),
        ],
    );
    assert!(
        !agent_row(&runtime, "w1:p1").unread,
        "starting work is not news"
    );
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "done", ""),
        ],
    );
    assert!(agent_row(&runtime, "w1:p1").unread);
    look_at(&mut runtime, "w1:p1");

    // The finished grandchild's pane closes: it leaves the badge and turns
    // nobody unread.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(!root.unread, "a closed pane is not news");
    assert_eq!(root.descendant_counts.done, 0);
    assert_eq!(root.descendant_counts.working, 1);
    let _ = std::fs::remove_file(&runtime.state_path);
}

// PRD B7, D-03: an ordinary child question never makes its ancestor Needs
// You. A working root stays Working on its own account.
#[test]
fn a_childs_question_leaves_its_working_root_in_working() {
    let mut runtime = runtime();
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "idle", "status_question_new"),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert_eq!(root.group, "working");
    assert!(
        root.wait.is_none(),
        "a root that is working itself is not waiting"
    );
    assert!(root.unread);
    assert!(
        !root.emphasized,
        "a working row is drawn as working, not raised"
    );
    assert_eq!(root.descendant_counts.question, 1);
    assert!(root.raised.is_empty());
    assert_eq!(agent_row(&runtime, "w1:p2").group, "seen");
    let _ = std::fs::remove_file(&runtime.state_path);
}

// agent-hierarchy-screens B1, B2, B17, D-10: a raised grandchild reaches the
// operator as its lineage root, once: the root is Needs You with the verb
// and what, the grandchild keeps its own group, and every ancestor between
// wears the raised mark.
#[test]
fn a_blocked_grandchild_raises_its_lineage_root_once_and_never_itself() {
    use crate::agent_state::escalation::{MarkKind, Verb};
    let mut runtime = runtime();
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "blocked", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert_eq!(root.group, "needs_you");
    assert_eq!(root.raised.len(), 1);
    assert_eq!(
        (root.raised[0].verb, root.raised[0].open_pane_id.as_str()),
        (Verb::Approval, "w1:p3")
    );
    assert_eq!(root.raised[0].path.len(), 3, "root, child, grandchild");
    let ask = root.state.ask.as_ref().expect("a Needs You row asks");
    assert_eq!((ask.verb, ask.more), (Verb::Approval, 0));
    assert_eq!(
        root.state.session.group,
        crate::agent_state::sessions::Group::NeedsYou
    );
    let grandchild = agent_row(&runtime, "w1:p3");
    assert!(grandchild.escalation.is_some());
    assert_ne!(
        grandchild.group, "needs_you",
        "the raised descendant is not a second Needs You row"
    );
    assert!(grandchild.state.ask.is_none());
    for pane in ["w1:p1", "w1:p2"] {
        let mark = agent_row(&runtime, pane).descendant_mark.unwrap();
        assert_eq!((mark.kind, mark.count), (MarkKind::Raised, 1), "{pane}");
    }
    let needs_you = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .filter(|row| row.group == "needs_you")
        .count();
    assert_eq!(needs_you, 1, "Needs You counts the root alone");

    // B7: the cause clears and the root leaves Needs You.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert_eq!(root.group, "working");
    assert!(root.raised.is_empty());
    let mark = root.descendant_mark.unwrap();
    assert_eq!((mark.kind, mark.count), (MarkKind::Working, 2));
    let _ = std::fs::remove_file(&runtime.state_path);
}

// sidebar-agent-status B1-B4, D-01: a root that is quiet itself while a
// descendant works is waiting on it: a Working row with the dashed ring,
// never Done and never Needs You, until no descendant works. A descendant
// that asks waits on the root, so the root does not wait on it. Its own
// question or its own work outranks the waiting state.
#[test]
fn a_quiet_root_waits_in_working_while_a_descendant_works_and_not_on_one_that_asks() {
    let mut runtime = runtime();
    // B1: the root finished its own turn and a child is working.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait == Some(crate::model::AgentWait::Children));
    assert_eq!(root.group, "working", "a waiting root is not Done");
    assert_eq!(root.mark, crate::agent_state::tally::RowMark::Waiting);
    assert_eq!(root.status_code, crate::model::AgentStatusCode::Waiting);
    assert!(!root.emphasized);
    assert_eq!(root.descendant_counts.working, 1);
    let wire = serde_json::to_value(root).unwrap();
    assert_eq!(wire["wait"], "children");
    assert_eq!(
        wire["group"], "working",
        "no new group value reaches the wire"
    );
    look_at(&mut runtime, "w1:p1");
    assert_eq!(
        agent_row(&runtime, "w1:p1").group,
        "working",
        "reading a waiting root does not make it Seen"
    );

    // B2: the child asks. It waits on the root, so the root draws its own
    // finished turn, unread because the question is news to it, the badge
    // carries the question, and the root is never Needs You.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "idle", "status_question_new"),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait.is_none(), "an asking child is not progress");
    assert_eq!(
        (root.group.as_str(), root.mark),
        ("done", crate::agent_state::tally::RowMark::Unread)
    );
    assert_eq!(root.descendant_counts.question, 1);
    assert!(root.unread, "a child's question is news to the root");
    assert!(
        runtime
            .snapshot
            .navigator
            .agents
            .iter()
            .all(|row| row.group != "needs_you")
    );
    look_at(&mut runtime, "w1:p1");

    // B4: the root's own question outranks waiting; its own work does too.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "idle", "status_question_new"),
            ("w1:p2", Some("w1:p1"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait.is_none());
    assert_eq!(
        (root.group.as_str(), root.mark),
        ("needs_you", crate::agent_state::tally::RowMark::Turn)
    );
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait.is_none());
    assert_eq!(
        (root.group.as_str(), root.mark),
        ("working", crate::agent_state::tally::RowMark::Going)
    );

    // B3: the root and every descendant are quiet: Done, and the ring goes.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "done", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait.is_none());
    assert_eq!(
        (root.group.as_str(), root.mark),
        ("done", crate::agent_state::tally::RowMark::Unread)
    );

    // A grandchild keeps the root waiting through a quiet middle row, and
    // closing its pane releases the root on the next projection.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "idle", ""),
            ("w1:p3", Some("w1:p2"), "working", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait == Some(crate::model::AgentWait::Children));
    assert_eq!(root.group, "working");
    assert!(
        agent_row(&runtime, "w1:p2").wait.is_none(),
        "only a lineage root waits; a delegated row keeps its own mark"
    );
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "idle", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    assert!(root.wait.is_none());
    assert_ne!(
        root.group, "working",
        "a closed child no longer holds the root"
    );

    // A child Herdr cannot classify does not make the root wait.
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "idle", ""),
            ("w1:p2", Some("w1:p1"), "unrecognized", ""),
        ],
    );
    assert!(agent_row(&runtime, "w1:p1").wait.is_none());
    let _ = std::fs::remove_file(&runtime.state_path);
}

/// A pane's agent as a letter names it, running the fixtures' one session.
fn letter_actor(runtime: &Runtime, pane: &str) -> crate::delivery::Actor {
    crate::delivery::Actor {
        pane_id: pane.into(),
        name: pane.into(),
        kind: "claude".into(),
        device_id: runtime.node.to_string(),
        session: Some(crate::sidebar::fixture_lineage_session()),
    }
}

/// A request `from` sent `to` a minute ago, still waiting for its answer.
fn ask(
    ledger: &mut crate::delivery::ledger::Ledger,
    from: &crate::delivery::Actor,
    to: &crate::delivery::Actor,
) -> String {
    crate::delivery::mailbox::send(
        ledger,
        from,
        to,
        &format!("ask-{}-{}", from.pane_id, to.pane_id),
        "Which design do we take?",
        "request",
        None,
        unix_milliseconds() - 60_000,
    )
    .unwrap()
    .id
}

/// The recipient's hook confirmed it took the letter in.
fn take_in(ledger: &mut crate::delivery::ledger::Ledger, id: &str) {
    let letter = ledger
        .letters
        .iter_mut()
        .find(|letter| letter.id == id)
        .unwrap();
    letter.state = crate::delivery::ledger::State::Delivered;
    letter.hook_confirmed = Some(true);
}

// A root whose children all finished waits on nothing, even while a request
// it sent one of them stays unanswered: the child took it in and stopped,
// so nothing is making the answer. Until the child takes it in, delivering
// the letter will wake it, and the root waits for that.
#[test]
fn a_root_with_finished_children_draws_its_own_state_past_a_request_a_stopped_child_took_in() {
    let mut runtime = runtime();
    let (root, child) = (
        letter_actor(&runtime, "w1:p1"),
        letter_actor(&runtime, "w1:p2"),
    );
    let mut ledger = crate::delivery::ledger::Ledger::default();
    let id = ask(&mut ledger, &root, &child);
    runtime.publish_delivery(Arc::new(ledger.clone()), false);
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "done", ""),
            ("w1:p3", Some("w1:p1"), "done", ""),
        ],
    );
    let row = agent_row(&runtime, "w1:p1");
    assert_eq!(row.wait, Some(crate::model::AgentWait::Reply));
    assert_eq!(row.group, "working");

    take_in(&mut ledger, &id);
    // The step reaches the rows through the ledger's publication alone.
    runtime.publish_delivery(Arc::new(ledger), false);
    let row = agent_row(&runtime, "w1:p1");
    assert_eq!(row.wait, None);
    assert_eq!(
        (row.group.as_str(), row.mark),
        ("done", crate::agent_state::tally::RowMark::Unread),
        "its own finished turn, unread"
    );
    look_at(&mut runtime, "w1:p1");
    let row = agent_row(&runtime, "w1:p1");
    assert_eq!(row.wait, None);
    assert_eq!(
        (row.group.as_str(), row.mark),
        ("seen", crate::agent_state::tally::RowMark::Quiet),
        "read, it is a gray ring"
    );
    let _ = std::fs::remove_file(&runtime.state_path);
}

// A child that asked its parent by letter waits on the parent, never the
// other way round: while the letter is undelivered the child waits for the
// parent to be woken and the parent keeps its own state, and once the
// parent took the letter in and stopped, neither row waits, even with the
// parent's own request to the child unanswered.
#[test]
fn a_child_asking_its_stopped_parent_by_letter_leaves_neither_row_waiting() {
    let mut runtime = runtime();
    let (root, child) = (
        letter_actor(&runtime, "w1:p1"),
        letter_actor(&runtime, "w1:p2"),
    );
    let mut ledger = crate::delivery::ledger::Ledger::default();
    let assigned = ask(&mut ledger, &root, &child);
    take_in(&mut ledger, &assigned);
    let question = ask(&mut ledger, &child, &root);
    runtime.publish_delivery(Arc::new(ledger.clone()), false);
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "done", ""),
            ("w1:p2", Some("w1:p1"), "done", ""),
        ],
    );
    assert_eq!(
        agent_row(&runtime, "w1:p2").wait,
        Some(crate::model::AgentWait::Reply),
        "the letter will wake the parent"
    );
    let parent = agent_row(&runtime, "w1:p1");
    assert_eq!(parent.wait, None, "a child's reply wait is not progress");
    assert_eq!(parent.group, "done");

    take_in(&mut ledger, &question);
    // The step reaches the rows through the ledger's publication alone.
    runtime.publish_delivery(Arc::new(ledger), false);
    for pane in ["w1:p1", "w1:p2"] {
        let row = agent_row(&runtime, pane);
        assert_eq!(row.wait, None, "{pane}");
        assert_ne!(row.group, "working", "{pane}");
    }
    let _ = std::fs::remove_file(&runtime.state_path);
}

// A waiting state the projection cannot vouch for is not drawn: a
// code-owned recipient has no row whose progress Hide can see, and an agent
// on a device that is not connected shows a state that may no longer hold,
// so a letter to either holds no reply wait, taken in or not.

// PRD B3, B4, B7, D-05, D-12: the badge sums every live descendant, not the
// direct children only; a merely ready child and an unknown one add nothing.
#[test]
fn the_descendant_badge_sums_every_live_descendant_and_skips_ready_and_unknown_ones() {
    let mut runtime = runtime();
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "working", ""),
            ("w1:p3", Some("w1:p2"), "idle", "status_question_new"),
            ("w1:p4", Some("w1:p2"), "done", ""),
            ("w1:p5", Some("w1:p1"), "idle", ""),
            ("w1:p6", Some("w1:p1"), "unrecognized", ""),
        ],
    );
    let root = agent_row(&runtime, "w1:p1");
    let counts = root.descendant_counts;
    assert_eq!(
        (
            counts.error,
            counts.approval,
            counts.question,
            counts.working,
            counts.done
        ),
        (0, 0, 1, 1, 1),
        "the grandchild's question reaches the root; a ready child and an unknown one are not drawn"
    );
    assert_eq!(counts.unknown, 1);
    let middle = agent_row(&runtime, "w1:p2");
    assert_eq!(
        (
            middle.descendant_counts.question,
            middle.descendant_counts.done
        ),
        (1, 1)
    );
    assert!(!agent_row(&runtime, "w1:p3").descendant_counts.any_drawn());
    let wire = serde_json::to_value(root).unwrap();
    assert_eq!(wire["descendant_counts"]["question"], 1);
    assert!(
        wire["descendant_counts"].get("unknown").is_none(),
        "a count the badge does not draw is not on the wire"
    );
    let _ = std::fs::remove_file(&runtime.state_path);
}

// PRD B1, D-03: no clock, no notice, no escalated ownership. The wire says
// nothing about a stall and a row's ownership is operator or delegated.
#[test]
fn escalation_is_explicit_while_ownership_remains_operator_or_delegated() {
    let mut runtime = runtime();
    ingest_lineage(
        &mut runtime,
        &[
            ("w1:p1", None, "working", ""),
            ("w1:p2", Some("w1:p1"), "blocked", ""),
        ],
    );
    let payload = runtime.snapshot_delta_payload(0);
    let bytes = crate::runtime::serialize_snapshot_delta(&payload).expect("snapshot encodes");
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("stall_"), "no stall field reaches the shell");
    assert!(text.contains("\"cause\":\"child_blocked\""));
    for row in &runtime.snapshot.navigator.agents {
        assert!(matches!(
            crate::agent_state::ownership_of(row),
            crate::agent_state::Ownership::Operator | crate::agent_state::Ownership::Delegated
        ));
    }
    let _ = std::fs::remove_file(&runtime.state_path);
}

// PRD B31, D-53: a dead session's numbers never stay on screen.
#[test]
fn a_pane_whose_session_ended_draws_no_counts_even_though_its_tokens_remain() {
    let rows = instrumented_rows("claude");
    let installed = hook_status_of(Some(hide_agent_hooks::HookStatus::Installed {
        version: hide_agent_hooks::HOOK_VERSION,
    }));
    // Herdr's pane metadata is durable, so the last session's tokens are
    // still on the pane after it exits.
    let leftover = crate::agent_hooks::PaneHookTokens {
        version: Some(hide_agent_hooks::HOOK_VERSION),
        working: Some(3),
        done: Some(9),
        blocked: None,
        session_digest: None,
    };
    assert!(
        crate::sidebar::project_pane_children(&rows, "no-such-agent", leftover, &installed)
            .is_none(),
        "the counts belong to a session, and there is no session in that pane"
    );
    // The same tokens on a pane that does have a session still read.
    let live = crate::sidebar::project_pane_children(&rows, "parent", leftover, &installed)
        .expect("a pane with an agent projects");
    assert_eq!(live.subagents.done, Some(9));
}

// PRD B27, D-31, D-48, D-61: the Settings diagnosis says what each runtime's
// hook is, and names the panes a restart would fix.
#[test]
fn the_settings_diagnosis_reports_each_runtime_and_the_sessions_that_predate_the_install() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [
            {"id":"instrumented-control","pane_id":"w1:p1","agent":"claude","agent_status":"working",
             "agent_session":{"kind":"id","value":"instrumented-session"},
             "state_change_seq":1,"cwd":"/fixture","workspace_label":"Fixture","tokens":{"task":"Check hook reports"}},
            {"id":"older-control","pane_id":"w1:p2","agent":"claude","agent_status":"working",
             "agent_session":{"kind":"id","value":"older-session"},
             "state_change_seq":2,"cwd":"/fixture","workspace_label":"Fixture","tokens":{"task":"Restore hook visibility"}}
        ],
        "panes": [
            {"pane_id":"w1:p1","cwd":"/fixture","tokens":{"hide_hooks":hide_agent_hooks::HOOK_VERSION.to_string(),"hide_sub_done":"2"}},
            {"pane_id":"w1:p2","cwd":"/fixture"}
        ],
        "tabs": [{"workspace_id":"w1","tab_id":"t1","label":""}],
        "layouts": [{
            "workspace_id":"w1","tab_id":"t1","zoomed":false,
            "area":{"x":0,"y":0,"width":80,"height":24},
            "focused_pane_id":"w1:p1",
            "panes":[
                {"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":40,"height":24}},
                {"pane_id":"w1:p2","rect":{"x":40,"y":0,"width":40,"height":24}}
            ],
            "splits":[{"direction":"right","ratio":0.5,
                       "rect":{"x":0,"y":0,"width":80,"height":24}}]
        }]
    }))
    .expect("session payload")));

    // Before anything has been read, the screen has nothing to claim.
    let connection = |runtime: &Runtime, pane_id: &str| {
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
            .and_then(|children| children.connection)
    };
    assert_eq!(connection(&runtime, "w1:p2"), None);

    assert!(runtime.ingest_hook_diagnosis(hide_agent_hooks::Diagnosis {
        runtimes: vec![
            hide_agent_hooks::diagnosis::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::ClaudeCode,
                label: "Claude Code".to_owned(),
                path: "/fixture/.claude/settings.json".to_owned(),
                status: hide_agent_hooks::HookStatus::Installed {
                    version: hide_agent_hooks::HOOK_VERSION,
                },
                current_version: hide_agent_hooks::HOOK_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                    version: "999.0.0".to_owned(),
                },
            },
            hide_agent_hooks::diagnosis::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::Codex,
                label: "Codex".to_owned(),
                path: "/fixture/.codex/hooks.json".to_owned(),
                status: hide_agent_hooks::HookStatus::NotInstalled,
                current_version: hide_agent_hooks::HOOK_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                    version: "999.0.0".to_owned(),
                },
            },
        ],
        last_report_failure: None,
    }));

    let hooks = &runtime.snapshot.status.agent_hooks;

    // The hook is installed and one pane still carries none of its tokens,
    // so that session started first: it is the one not connected, and the
    // instrumented pane is connected (B16, B26, B28).
    assert_eq!(
        connection(&runtime, "w1:p1"),
        Some(crate::model::PaneConnectionSnapshot {
            connected: true,
            can_reopen: false,
            reason: None,
            reopen: None,
        })
    );
    assert_eq!(
        connection(&runtime, "w1:p2"),
        Some(crate::model::PaneConnectionSnapshot {
            connected: false,
            can_reopen: true,
            reason: Some(crate::model::PaneConnectionReason::StartedBeforeHide),
            reopen: None,
        })
    );
    assert_eq!(
        hooks.last_report_failure, None,
        "a hook whose last report landed has no failure to show"
    );

    // A hook that ran and was refused by Herdr looks, pane by pane, exactly
    // like a session that started first. The recorded failure is what the
    // Settings screen shows instead of the restart advice.
    let mut refused = runtime
        .hook_diagnosis
        .clone()
        .expect("diagnosis was ingested");
    refused.last_report_failure = Some(hide_agent_hooks::report::ReportFailure {
        pane_id: "w1:p2".to_owned(),
        event: "SessionStart".to_owned(),
        socket_path: "/fixture/herdr.sock".to_owned(),
        error: "invalid_metadata_source: metadata source may contain only ASCII letters".to_owned(),
        at_unix_ms: 1,
    });
    assert!(runtime.ingest_hook_diagnosis(refused));
    let message = runtime
        .snapshot
        .status
        .agent_hooks
        .last_report_failure
        .clone()
        .expect("the refusal reaches the snapshot");
    assert!(
        message.contains("/fixture/herdr.sock")
            && message.contains("w1:p2")
            && message.contains("invalid_metadata_source"),
        "got {message:?}"
    );
}

// PRD device-parity B8: Reinstall repairs only the parts that need it, a
// second press after the repair is the same intent already met, and a
// machine Hide cannot act on says why rather than being dropped.
#[test]
fn a_reinstall_queues_only_the_parts_that_need_it() {
    let mut runtime = runtime();
    let reinstall = |device_id: &str| {
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "kind": "kit_reinstall",
            "payload": {"device_id": device_id}
        }))
        .unwrap()
    };
    let part = |id, state| hide_kit::ComponentReport {
        id,
        state,
        reason: None,
        location: None,
    };
    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &hide_kit::KitReport {
            components: vec![
                part(
                    hide_kit::ComponentId::Cli,
                    hide_kit::ComponentState::Installed,
                ),
                part(
                    hide_kit::ComponentId::ClaudeCodeHook,
                    hide_kit::ComponentState::Removed,
                ),
                part(
                    hide_kit::ComponentId::CodexHook,
                    hide_kit::ComponentState::Failed,
                ),
                part(
                    hide_kit::ComponentId::CoordinationRetirement,
                    hide_kit::ComponentState::Installed,
                ),
            ],
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
        },
    );
    let local = |runtime: &Runtime| {
        runtime
            .snapshot
            .navigator
            .devices
            .iter()
            .find(|device| device.id == crate::node::TEST_NODE)
            .unwrap()
            .kit
            .clone()
    };
    assert!(local(&runtime).offers_reinstall);
    assert!(!local(&runtime).busy);

    assert!(runtime.dispatch_json(&reinstall(crate::node::TEST_NODE)));
    assert!(runtime.dispatch_json(&reinstall(crate::node::TEST_NODE)));
    assert!(local(&runtime).busy, "the row shows the install running");
    assert_eq!(
        runtime.take_local_kit_job(std::time::Instant::now()),
        Some(crate::runtime::KitJob::Apply(hide_kit::Scope::reinstall([
            hide_kit::ComponentId::ClaudeCodeHook,
            hide_kit::ComponentId::CodexHook,
        ]))),
        "two presses are one install of the two parts that need it"
    );

    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &hide_kit::KitReport {
            components: vec![part(
                hide_kit::ComponentId::Cli,
                hide_kit::ComponentState::Installed,
            )],
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
        },
    );
    assert!(!local(&runtime).busy);
    assert!(!local(&runtime).offers_reinstall);
    assert!(
        !runtime.dispatch_json(&reinstall(crate::node::TEST_NODE)),
        "nothing is left to repair"
    );
    assert_eq!(runtime.take_local_kit_job(std::time::Instant::now()), None);

    runtime.set_local_kit_unavailable(hide_kit::STANDALONE_REASON);
    assert!(runtime.dispatch_json(&reinstall(crate::node::TEST_NODE)));
    let error = runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .expect("the refusal is visible rather than silent");
    assert_eq!(error.kind, "kit.unavailable");
    assert_eq!(runtime.take_local_kit_job(std::time::Instant::now()), None);
}

// PRD B34, B35, D-32, D-55, D-60: Overview says who is working here, and an
// empty line is not the same answer as one Hide cannot fill.
#[test]
fn the_overview_agent_line_separates_nobody_working_here_from_cannot_see() {
    let rows = instrumented_rows("claude");
    let chips = rows
        .iter()
        .map(|agent| (agent.pane_id.clone(), crate::sidebar::agent_chip(agent)))
        .collect::<HashMap<_, _>>();
    let reason_for = |code: hide_agent_hooks::diagnosis::UninstrumentedReason| {
        crate::model::PaneChildrenSnapshot {
            instrumented: false,
            uninstrumented_reason: Some(code.message().to_owned()),
            uninstrumented_label: Some(code.accessibility_label().to_owned()),
            uninstrumented_code: Some(code.code().to_owned()),
            ..Default::default()
        }
    };
    let panes = |ids: &[&str]| {
        ids.iter()
            .map(|id| (*id).to_owned())
            .collect::<HashSet<String>>()
    };

    // A worktree Hide has no pane rows for at all.
    let none = worktree_agent_line(None, &chips, &HashMap::new(), &rows);
    assert!(none.agents.is_empty() && none.uninstrumented_code.is_none());

    // Panes, but nobody working in them: an empty line and no claim.
    let quiet = worktree_agent_line(Some(&panes(&["idle"])), &chips, &HashMap::new(), &rows);
    assert!(quiet.agents.is_empty());
    assert_eq!(
        quiet.uninstrumented_code, None,
        "no agents is an answer, not a gap"
    );

    // Two agents here and one elsewhere, all instrumented.
    let instrumented = HashMap::from([
        (
            "parent".to_owned(),
            crate::model::PaneChildrenSnapshot {
                instrumented: true,
                ..Default::default()
            },
        ),
        (
            "child".to_owned(),
            crate::model::PaneChildrenSnapshot {
                instrumented: true,
                ..Default::default()
            },
        ),
    ]);
    let working = worktree_agent_line(
        Some(&panes(&["parent", "child"])),
        &chips,
        &instrumented,
        &rows,
    );
    assert_eq!(
        working
            .agents
            .iter()
            .map(|agent| agent.pane_id.as_str())
            .collect::<Vec<_>>(),
        rows.iter()
            .filter(|agent| agent.pane_id != "sibling")
            .map(|agent| agent.pane_id.as_str())
            .collect::<Vec<_>>(),
        "the line follows the sidebar's own order"
    );
    assert!(working.agents.iter().any(|agent| !agent.label.is_empty()));
    assert_eq!(working.uninstrumented_code, None);

    // One of them is uninstrumented, and two reasons are in play: the line
    // shows the first in the crate's resolution order, not the first pane.
    let mixed = HashMap::from([
        (
            "parent".to_owned(),
            reason_for(hide_agent_hooks::diagnosis::UninstrumentedReason::SessionPredatesInstall),
        ),
        (
            "child".to_owned(),
            reason_for(hide_agent_hooks::diagnosis::UninstrumentedReason::HooksNotInstalled),
        ),
    ]);
    let unseen = worktree_agent_line(Some(&panes(&["parent", "child"])), &chips, &mixed, &rows);
    assert_eq!(unseen.agents.len(), 2, "the agents are still named");
    assert_eq!(
        unseen.uninstrumented_code.as_deref(),
        Some("hooks_not_installed")
    );
    assert_eq!(
        unseen.uninstrumented_reason,
        Some(
            hide_agent_hooks::diagnosis::UninstrumentedReason::HooksNotInstalled
                .message()
                .to_owned()
        ),
        "the same sentence the pane header shows (PRD B21)"
    );
    assert!(unseen.uninstrumented_label.is_some(), "the mark is named");
}

/// One realistic delegation session, projected end to end.
///
/// It asserts what the operator's screen says in the four states this change
/// exists to separate - a parent with children, a pane Hide cannot see into,
/// an instrumented pane working alone, and a pane with no agent at all - and,
/// when `HIDE_SNAPSHOT_OUT` names a path, writes the snapshot the shell would
/// draw so the same states can be captured for human visual judgement
/// (PRD D-34) without a live server or the operator's own panes.
#[test]
fn a_delegation_session_projects_every_state_the_operator_has_to_tell_apart() {
    let mut runtime = runtime();
    runtime.ingest_session(Ok(crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [
            {"id":"Observer","pane_id":"w1:p1","agent":"claude","agent_status":"working",
             "state_change_seq":1,"cwd":"/fixture","workspace_label":"hide",
             "tokens":{"task":"Coordinate lineage work","progress":"delegating the orchestrator work"}},
            {"id":"Implementor","pane_id":"w1:p2","agent":"claude","agent_status":"working",
             "state_change_seq":2,"cwd":"/fixture","workspace_label":"hide",
             "spawned_from_pane_id":"w1:p1",
             "tokens":{"task":"Implement lineage projection","progress":"구현 중: 계보 투영과 위임 표시"}},
            {"id":"Reviewer","pane_id":"w1:p3","agent":"claude","agent_status":"idle",
             "state_change_seq":3,"cwd":"/fixture","workspace_label":"hide",
             "spawned_from_pane_id":"w1:p1",
             "tokens":{"task":"Review lineage behavior","status_question_new":"?","progress":"정체 임계값을 물어보는 중"}},
            {"id":"Uninstrumented","pane_id":"w1:p4","agent":"claude","agent_status":"working",
             "agent_session":{"kind":"id","value":"uninstrumented-session"},
             "state_change_seq":4,"cwd":"/fixture","workspace_label":"hide",
             "tokens":{"task":"Restore hook visibility","progress":"started before the hook was installed"}},
            {"id":"Alone","pane_id":"w1:p5","agent":"claude","agent_status":"working",
             "state_change_seq":5,"cwd":"/fixture","workspace_label":"hide",
             "tokens":{"task":"Verify solo agent state","progress":"working with no children"}}
        ],
        "panes": [
            {"pane_id":"w1:p1","cwd":"/fixture",
             "tokens":{"hide_hooks":hide_agent_hooks::HOOK_VERSION.to_string(),"hide_sub_working":"2","hide_sub_done":"4"}},
            {"pane_id":"w1:p2","cwd":"/fixture","tokens":{"hide_hooks":hide_agent_hooks::HOOK_VERSION.to_string()}},
            {"pane_id":"w1:p3","cwd":"/fixture","tokens":{"hide_hooks":hide_agent_hooks::HOOK_VERSION.to_string()}},
            {"pane_id":"w1:p4","cwd":"/fixture"},
            {"pane_id":"w1:p5","cwd":"/fixture","tokens":{"hide_hooks":hide_agent_hooks::HOOK_VERSION.to_string(),"hide_sub_working":"0","hide_sub_done":"0"}},
            {"pane_id":"w1:p6","cwd":"/fixture"}
        ],
        "tabs": [
            {"workspace_id":"w1","tab_id":"t1","label":""},
            {"workspace_id":"w1","tab_id":"t2","label":""},
            {"workspace_id":"w1","tab_id":"t3","label":""},
            {"workspace_id":"w1","tab_id":"t4","label":""},
            {"workspace_id":"w1","tab_id":"t5","label":""},
            {"workspace_id":"w1","tab_id":"t6","label":""}
        ],
        "layouts": [
            {
                "workspace_id":"w1","tab_id":"t1","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p1",
                "panes":[{"pane_id":"w1:p1","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            },
            {
                "workspace_id":"w1","tab_id":"t2","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p2",
                "panes":[{"pane_id":"w1:p2","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            },
            {
                "workspace_id":"w1","tab_id":"t6","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p3",
                "panes":[{"pane_id":"w1:p3","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            },
            {
                "workspace_id":"w1","tab_id":"t3","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p4",
                "panes":[{"pane_id":"w1:p4","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            },
            {
                "workspace_id":"w1","tab_id":"t4","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p5",
                "panes":[{"pane_id":"w1:p5","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            },
            {
                "workspace_id":"w1","tab_id":"t5","zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40},
                "focused_pane_id":"w1:p6",
                "panes":[{"pane_id":"w1:p6","rect":{"x":0,"y":0,"width":120,"height":40}}],
                "splits":[]
            }
        ]
    }))
    .expect("session payload")));

    // The hook is installed, which is what turns "no tokens on this pane"
    // into "this session started first" rather than "no hook anywhere".
    runtime.ingest_hook_diagnosis(hide_agent_hooks::Diagnosis {
        runtimes: vec![
            hide_agent_hooks::diagnosis::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::ClaudeCode,
                label: "Claude Code".to_owned(),
                path: "/fixture/.claude/settings.json".to_owned(),
                status: hide_agent_hooks::HookStatus::Installed {
                    version: hide_agent_hooks::HOOK_VERSION,
                },
                current_version: hide_agent_hooks::HOOK_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                    version: "999.0.0".to_owned(),
                },
            },
            hide_agent_hooks::diagnosis::RuntimeDiagnosis {
                runtime: hide_agent_hooks::AgentRuntime::Codex,
                label: "Codex".to_owned(),
                path: "/fixture/.codex/hooks.json".to_owned(),
                status: hide_agent_hooks::HookStatus::NotInstalled,
                current_version: hide_agent_hooks::HOOK_VERSION,
                memory_compatibility: hide_agent_hooks::MemoryCompatibility::Supported {
                    version: "999.0.0".to_owned(),
                },
            },
        ],
        last_report_failure: None,
    });

    let pane = |id: &str| {
        runtime
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == id)
            .unwrap_or_else(|| panic!("{id} is a pane"))
            .clone()
    };

    // A parent with children: chips for the pane children, the in-process
    // count separate from them, and a breadcrumb its children carry.
    let parent = pane("w1:p1").children.expect("the parent has a session");
    assert!(parent.instrumented);
    assert_eq!(
        parent
            .chips
            .iter()
            .map(|chip| chip.label.as_str())
            .collect::<Vec<_>>(),
        ["Review lineage behavior", "Implement lineage projection"],
        "chips follow the lineage's own child order and carry the rolling task (PRD D-01)"
    );
    assert_eq!(
        parent
            .chips
            .iter()
            .map(|chip| chip.detail.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("정체 임계값을 물어보는 중"),
            Some("구현 중: 계보 투영과 위임 표시")
        ],
        "a delegated question is Seen and still says what it asks; a delegated worker keeps its progress (PRD D-06, sidebar-agent-status B7)"
    );
    assert_eq!(
        parent.representative.as_ref().unwrap().label,
        "Review lineage behavior"
    );
    assert_eq!(
        (parent.subagents.working, parent.subagents.done),
        (Some(2), Some(4))
    );
    assert!(
        pane("w1:p2")
            .lineage_path
            .iter()
            .any(|step| step.label == "Coordinate lineage work")
    );

    // A pane Hide cannot see into, and the reason a restart would fix it.
    let unseen = pane("w1:p4").children.expect("it has a session");
    assert!(!unseen.instrumented);
    assert_eq!(
        unseen.uninstrumented_code.as_deref(),
        Some("session_predates_install")
    );

    // An instrumented session working alone: a confirmed answer, and a
    // different screen from the one above.
    let alone = pane("w1:p5").children.expect("it has a session");
    assert!(alone.instrumented && alone.chips.is_empty());
    assert_eq!(alone.uninstrumented_reason, None);
    assert_eq!(alone.subagents.working, Some(0));

    // A pane with no agent says nothing at all.
    assert!(pane("w1:p6").children.is_none());

    // The header's chip is the one place a session Hide cannot hear is named.
    assert_eq!(
        unseen.connection.and_then(|connection| connection.reason),
        Some(crate::model::PaneConnectionReason::StartedBeforeHide)
    );

    // Overview reads the same rows. A catalog is handed in directly because
    // the real one comes from `git`, which a projection test does not run,
    // and Overview draws the focused checkout's project.
    let checkout_id = runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .id
        .clone();
    runtime.snapshot.navigator.focused_checkout_id = Some(checkout_id.clone());
    runtime.snapshot.ui_state.focused_checkout_id = Some(checkout_id);
    let workspace_id = runtime.snapshot.navigator.workspaces[0].id.clone();
    runtime.snapshot.navigator.focused_workspace_id = Some(workspace_id);
    runtime.ingest_worktrees(
        crate::model::WorktreeCatalogSnapshot {
            projects: vec![crate::model::ProjectWorktreesSnapshot {
                root_path: "/fixture".to_owned(),
                base_source: "default_branch".to_owned(),
                default_branch: Some("main".to_owned()),
                worktrees: vec![
                    crate::model::WorktreeSnapshot {
                        path: "/fixture".to_owned(),
                        branch: Some("main".to_owned()),
                        is_main: true,
                        upstream_state: "pushed".to_owned(),
                        ..Default::default()
                    },
                    crate::model::WorktreeSnapshot {
                        path: "/fixture/quiet".to_owned(),
                        branch: Some("quiet".to_owned()),
                        upstream_state: "pushed".to_owned(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        },
        0,
    );
    let overview = runtime
        .snapshot
        .git_worktrees
        .as_ref()
        .expect("the catalog projects");
    let line = |path: &str| {
        overview
            .worktrees
            .iter()
            .find(|worktree| worktree.path == path)
            .map(|worktree| worktree.agent_line.clone())
            .unwrap_or_else(|| panic!("{path} is a worktree row"))
    };
    // The worktree the session runs in names its agents and carries the mark
    // of the one Hide cannot see into.
    let busy = line("/fixture");
    assert!(!busy.agents.is_empty());
    assert_eq!(
        busy.uninstrumented_code.as_deref(),
        Some("session_predates_install")
    );
    // A worktree nobody is working in says nothing, which is a different
    // screen from one Hide cannot see into (PRD B35, D-60).
    let quiet = line("/fixture/quiet");
    assert!(quiet.agents.is_empty());
    assert_eq!(quiet.uninstrumented_code, None);

    if let Ok(path) = std::env::var("HIDE_SNAPSHOT_OUT") {
        let payload = runtime.snapshot_delta_payload(0);
        let bytes = crate::runtime::serialize_snapshot_delta(&payload).expect("snapshot encodes");
        std::fs::write(&path, bytes).expect("snapshot is written");
    }
}

// PRD close-agent-subtree D-20, D-21: a row lists every live descendant in
// the order closing it takes them, each before its own parent.
#[test]
fn each_row_lists_its_live_descendants_deepest_first_and_a_leaf_lists_none() {
    let mut rows = lineage_rows();
    crate::agent_state::apply_lineage(&mut rows, &lineage_workspaces(), &[]);
    let listed = |id: &str| {
        rows.iter()
            .find(|row| row.pane_id == id)
            .unwrap()
            .close_descendant_pane_ids
            .clone()
    };
    let parent = listed("parent");
    assert_eq!(parent.len(), 3);
    let at = |id: &str| parent.iter().position(|pane| pane == id).unwrap();
    assert!(
        at("grandchild") < at("child"),
        "a child closes after its own child"
    );
    assert!(parent.contains(&"sibling".to_owned()));
    assert_eq!(listed("child"), ["grandchild"]);
    assert!(listed("grandchild").is_empty());
    assert!(listed("sibling").is_empty());
    let wire =
        serde_json::to_value(rows.iter().find(|row| row.pane_id == "sibling").unwrap()).unwrap();
    assert!(
        wire.get("close_descendant_pane_ids").is_none(),
        "a row with no descendant carries no field"
    );
}

// PRD close-agent-subtree D-16, B17: a device that is not connected keeps
// its lineage but none of its rows is ever named for a close.
#[test]
fn a_descendant_on_a_disconnected_device_is_never_listed_for_a_close() {
    let mut runtime = runtime();
    runtime.node = crate::node::NodeId::parse("machine-local").unwrap();
    runtime
        .device_machine_ids
        .insert("mini".to_owned(), "machine-mini".to_owned());
    let mut rows = lineage_rows();
    let mut parent = rows.remove(0);
    parent.pane_id = "local-parent".to_owned();
    parent.declared_parent_pane_id = None;
    parent.spawned_from_pane_id = None;
    let mut local_child = rows.remove(0);
    local_child.pane_id = "local-child".to_owned();
    local_child.declared_parent_pane_id = Some("local-parent".to_owned());
    local_child.spawned_from_machine_id = None;
    let mut remote_child = rows.remove(0);
    remote_child.pane_id = "remote:mini:pane:remote-child".to_owned();
    remote_child.declared_parent_pane_id = Some("local-parent".to_owned());
    remote_child.spawned_from_machine_id = Some("machine-local".to_owned());
    runtime.snapshot.navigator.agents = vec![parent, local_child];
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "connected".to_owned(),
        message: None,
        herdr_version: Some("0.9.1".to_owned()),
        session: Some(remote_lineage_session(vec![remote_child])),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    runtime.refresh_agent_lineage();
    let mut listed = local_lineage_agent(&runtime, "local-parent")
        .close_descendant_pane_ids
        .clone();
    listed.sort();
    assert_eq!(listed, ["local-child", "remote:mini:pane:remote-child"]);

    runtime.snapshot.status.remote[0].state = "stale".to_owned();
    runtime.refresh_agent_lineage();
    assert_eq!(
        local_lineage_agent(&runtime, "local-parent").close_descendant_pane_ids,
        ["local-child"]
    );
}

// A cleanup reads a checkout as in use while an agent it delegated to is
// working or waiting, wherever that agent runs, and not once they are quiet.
#[test]
fn a_checkout_stays_in_use_while_an_agent_it_delegated_to_works_in_another_checkout() {
    let facts_for = |mutate: &dyn Fn(&mut Vec<SidebarAgentSnapshot>)| {
        let mut runtime = runtime();
        let mut rows = lineage_rows();
        mutate(&mut rows);
        let mut workspaces = lineage_workspaces();
        workspaces[0].is_git = true;
        crate::agent_state::apply_lineage(&mut rows, &workspaces, &[]);
        runtime.snapshot.navigator.agents = rows;
        runtime.snapshot.navigator.workspaces = workspaces;
        let facts = runtime.cleanup_facts("project").unwrap();
        let of = |path: &str| {
            facts
                .iter()
                .find(|facts| facts.path == std::path::Path::new(path))
                .unwrap()
                .clone()
        };
        (of("/fixture/main"), of("/fixture/feature"))
    };
    let stop = |rows: &mut Vec<SidebarAgentSnapshot>, panes: &[&str]| {
        for row in rows
            .iter_mut()
            .filter(|row| panes.contains(&row.pane_id.as_str()))
        {
            row.activity = "stopped".to_owned();
        }
    };

    // The parent finished, but the three agents it spawned still work in the
    // feature checkout: main is in use through them.
    let (main, feature) = facts_for(&|rows| stop(rows, &["parent"]));
    assert_eq!((main.agent_working, main.descendants_busy), (0, 3));
    assert_eq!(feature.agent_working, 3);

    // Everything stopped: no checkout is in use, and each lists the panes a
    // confirmed removal would close.
    let (main, feature) =
        facts_for(&|rows| stop(rows, &["parent", "child", "grandchild", "sibling"]));
    for facts in [&main, &feature] {
        assert_eq!(
            (
                facts.agent_working,
                facts.agent_waiting,
                facts.agent_unknown,
                facts.descendants_busy
            ),
            (0, 0, 0, 0)
        );
    }
    assert_eq!(main.panes, ["parent"]);
    assert_eq!(feature.panes, ["child", "grandchild", "sibling"]);
    assert!(main.terminal_panes.is_empty());

    // A finished child that asks for an approval keeps the parent in use.
    let (main, _) = facts_for(&|rows| {
        stop(rows, &["parent", "child", "grandchild", "sibling"]);
        rows.iter_mut()
            .find(|row| row.pane_id == "grandchild")
            .unwrap()
            .blocked = true;
    });
    assert_eq!(main.descendants_busy, 1);
}

// A parent whose delegated child works on a device keeps its checkout in use,
// and the device going away does not read as idle.
#[test]
fn a_checkout_stays_in_use_while_its_delegated_child_works_on_a_device_even_after_it_goes_stale() {
    let mut runtime = cross_machine_runtime();
    let mut workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture/main",
        vec![checkout(
            "project",
            "main",
            "/fixture/main",
            Some(pane("local-parent", "/fixture/main")),
        )],
    )];
    workspaces[0].is_git = true;
    runtime.snapshot.navigator.workspaces = workspaces;
    runtime.refresh_agent_lineage();
    let busy = |runtime: &Runtime| runtime.cleanup_facts("project").unwrap()[0].descendants_busy;
    assert_eq!(busy(&runtime), 1, "a working child on a connected device");

    runtime.snapshot.status.remote[0].state = "stale".to_owned();
    runtime.refresh_agent_lineage();
    assert_eq!(
        busy(&runtime),
        1,
        "its last state stands while the device is away"
    );

    runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .agents
        .iter_mut()
        .filter(|agent| agent.pane_id.ends_with("remote-child"))
        .for_each(|agent| agent.activity = "stopped".to_owned());
    runtime.refresh_agent_lineage();
    assert_eq!(busy(&runtime), 0, "a finished child does not");
}

// PRD settings-cleanup D-14: Codex starts follow what the machine's own Codex
// answered about its shared daemon, never a switch of the kit's.
#[test]
fn codex_starts_follow_the_capability_the_machines_kit_read() {
    use crate::codex_launch::CodexDaemon;
    let mut runtime = runtime();
    let report = |codex_daemon| hide_kit::KitReport {
        codex_daemon,
        codex_daemon_on: None,
        codex_daemon_running: None,
        codex_daemon_unreadable: None,
        codex_capability_unread: None,
        codex_daemon_off: None,
        ..Default::default()
    };
    assert_eq!(
        runtime.codex_daemon(crate::node::TEST_NODE),
        CodexDaemon::Unknown,
        "a machine whose kit has not answered cannot start Codex yet"
    );
    runtime.ingest_kit_report(crate::node::TEST_NODE, &report(Some(true)));
    assert_eq!(
        runtime.codex_daemon(crate::node::TEST_NODE),
        CodexDaemon::Present
    );

    runtime.ingest_kit_report(crate::node::TEST_NODE, &report(Some(false)));
    assert_eq!(
        runtime.codex_daemon(crate::node::TEST_NODE),
        CodexDaemon::Unsupported
    );

    runtime.ingest_kit_report(crate::node::TEST_NODE, &report(None));
    assert_eq!(
        runtime.codex_daemon(crate::node::TEST_NODE),
        CodexDaemon::Unknown
    );

    // The retired switch is not an event any more.
    let retired = serde_json::to_vec(&serde_json::json!({
        "schema_version": 2, "kind": "kit_component_set",
        "payload": {"device_id": crate::node::TEST_NODE, "component": "codex_per_pane", "enabled": false}
    }))
    .unwrap();
    runtime.dispatch_json(&retired);
    assert_eq!(runtime.take_local_kit_job(std::time::Instant::now()), None);
}

/// B18, B19, D-25: a quiet row waits for its children first, then for a
/// background task its session proves, then for a reply someone is still
/// making. A delegated child that waits on a background task counts as
/// working for its root; one that waits on a reply does not, because the
/// reply is the answering agent's progress, not the child's.
#[test]
fn a_quiet_row_waits_on_children_then_a_proven_device_then_a_reply() {
    let mut rows = project_agents(
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
            {"pane_id":"parent","agent_status":"done","state_change_seq":1},
            {"pane_id":"child","spawned_from_pane_id":"parent","agent_status":"done","state_change_seq":2},
            {"pane_id":"peer","agent_status":"working","state_change_seq":3},
        ]}))
        .unwrap(),
    )
    .agents;
    let index = |rows: &[SidebarAgentSnapshot], id: &str| {
        rows.iter().position(|row| row.pane_id == id).unwrap()
    };
    let wait_of = |rows: &[SidebarAgentSnapshot], id: &str| rows[index(rows, id)].wait;
    let (child, peer) = (index(&rows, "child"), index(&rows, "peer"));
    let asked_peer = |taken_in: bool| crate::model::AwaitedReply {
        recipient_pane_id: "peer".into(),
        recipient_session: Some(crate::sidebar::fixture_lineage_session()),
        taken_in,
    };

    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(wait_of(&rows, "child"), None);
    assert_eq!(wait_of(&rows, "parent"), None);

    // The peer took the letter in and works on the answer.
    rows[child].row_facts = None;
    rows[child].awaited_replies = vec![asked_peer(true)];
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    let row = &rows[child];
    assert_eq!(row.wait, Some(crate::model::AgentWait::Reply));
    assert!(
        row.row_facts.is_none(),
        "a waiting letter does not stand in for a proven session read"
    );
    assert_eq!(row.group, "working", "a delegated row that waits works");
    assert_eq!(
        wait_of(&rows, "parent"),
        None,
        "a child's reply wait is not progress under its root"
    );

    rows[child].row_facts = Some(crate::request_view::RowFacts {
        executions: hide_agent_adapter::work::WorkRead::Current {
            source: hide_agent_adapter::work::WorkSource::Session,
            value: hide_agent_adapter::work::WorkRecords {
                records: ["one", "two"]
                    .map(|id| hide_agent_adapter::work::WorkRecord {
                        id: id.into(),
                        kind: None,
                        name: None,
                        state: hide_agent_adapter::work::WorkState::Running,
                        expires_at_unix_ms: None,
                    })
                    .into(),
                history_truncated: false,
            },
        },
        ..Default::default()
    });
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(
        wait_of(&rows, "child"),
        Some(crate::model::AgentWait::Background),
        "a proven device outranks a reply"
    );
    assert_eq!(
        wait_of(&rows, "parent"),
        Some(crate::model::AgentWait::Children),
        "the root waits on a child whose background task runs"
    );

    // The peer stopped after taking the letter in: nothing makes the answer.
    rows[child].row_facts = None;
    rows[peer].activity = "stopped".into();
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(wait_of(&rows, "child"), None);
    assert_eq!(wait_of(&rows, "parent"), None);

    // A letter the peer has not taken in yet will wake it.
    rows[child].awaited_replies = vec![asked_peer(false)];
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(
        wait_of(&rows, "child"),
        Some(crate::model::AgentWait::Reply)
    );

    // An agent that took the peer's pane over is not who was asked.
    rows[peer].lineage_session = crate::wire::session_digest("another-agent");
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(wait_of(&rows, "child"), None);
}

/// A source and its fork as Herdr lists them once the fork's pane names the
/// source (`lineage::desired`); `fork_session` is the fork's own session.
fn fork_rows(fork_session: &str) -> Vec<SidebarAgentSnapshot> {
    let session = crate::sidebar::fixture_lineage_session();
    project_agents(crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [
        {"id":"source-control","pane_id":"source","agent_status":"working","state_change_seq":1,"tokens":{"task":"Source"}},
        {"id":"fork-control","pane_id":"fork","agent_status":"working","state_change_seq":2,"tokens":{
            "task":"Fork","fork_pane":"source","fork_session":fork_session,"fork_source_session":session}}
    ]})).unwrap()).agents
}

// PRD fork-operator-root B11, B12, B19: the fork names its source and is the
// operator's own work, with nothing of the source's lineage on either row.
#[test]
fn a_fork_names_its_source_and_stays_the_operators_own_work() {
    let row = |rows: &[SidebarAgentSnapshot], id: &str| {
        rows.iter().find(|row| row.pane_id == id).unwrap().clone()
    };
    let mut rows = fork_rows(&crate::sidebar::fixture_lineage_session());
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    let fork = row(&rows, "fork");
    assert_eq!(
        fork.forked_from,
        Some(crate::model::ForkSourceSnapshot {
            pane_id: Some("source".into()),
            label: Some("Source".into()),
        })
    );
    assert!(!fork.delegated);
    assert_eq!(fork.lineage_parent_pane_id, None);
    assert_eq!(fork.spawn_origin_pane_id, None);
    let source = row(&rows, "source");
    assert_eq!(source.forked_from, None);
    assert_eq!(source, {
        let mut alone = fork_rows("unrelated");
        alone.retain(|row| row.pane_id == "source");
        crate::agent_state::apply_lineage(&mut alone, &[], &[]);
        alone.remove(0)
    });

    // A source that runs another conversation now, or has closed, is still
    // where the fork came from, but no longer a pane to name or go to (B9).
    rows.iter_mut()
        .find(|row| row.pane_id == "source")
        .unwrap()
        .lineage_session = Some("another".into());
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    let unnamed = Some(crate::model::ForkSourceSnapshot::default());
    assert_eq!(row(&rows, "fork").forked_from, unnamed);
    rows.retain(|row| row.pane_id != "source");
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(row(&rows, "fork").forked_from, unnamed);

    // A pane that runs a new conversation is no longer the fork (B10).
    let mut rows = fork_rows("an-earlier-conversation");
    crate::agent_state::apply_lineage(&mut rows, &[], &[]);
    assert_eq!(row(&rows, "fork").forked_from, None);
}

/// PRD hide-ai-origin-mark B1, B3, B11, B13: the mark and the checkout list
/// come from the ledger's record of who started the spawn, on this machine
/// and on a connected device, and only for the session the record holds.
#[test]
fn rows_and_checkouts_hide_started_are_marked_from_the_ledger_alone() {
    use crate::coordination::started::tests::{Child, hide_started_ledger, memory_task};
    let mut runtime = cross_machine_runtime();
    attest_cross_machine_sessions(&mut runtime);
    let node = runtime.node.to_string();
    let ledger = hide_started_ledger(
        &node,
        "hide-memory-m72",
        memory_task("m72"),
        Child {
            machine: &node,
            pane: "local-child",
            session: "s-local-child",
        },
    );
    runtime.publish_delivery(Arc::new(ledger), false);

    let child = local_lineage_agent(&runtime, "local-child");
    assert_eq!(
        child.hide_ai.as_ref().and_then(|mark| mark.task.clone()),
        memory_task("m72")
    );
    assert!(
        local_lineage_agent(&runtime, "local-parent")
            .hide_ai
            .is_none()
    );
    assert_eq!(
        runtime.snapshot.navigator.hide_ai_checkouts,
        [crate::model::HideAiCheckoutSnapshot {
            device_id: node.clone(),
            path: "/repo.worktrees/hide-memory-m72".into(),
            task: memory_task("m72"),
        }]
    );

    // An agent that took the pane later holds another session: no mark.
    for agent in &mut runtime.snapshot.navigator.agents {
        if agent.pane_id == "local-child" {
            agent.lineage_session = crate::wire::session_digest("s-another-agent");
        }
    }
    runtime.refresh_agent_lineage();
    assert!(
        local_lineage_agent(&runtime, "local-child")
            .hide_ai
            .is_none()
    );

    // A device's agent is registered under the id the core names its pane by,
    // and its spawn under the device's own id, as a spawn on a device leaves them.
    let ledger = hide_started_ledger(
        &node,
        "hide-memory-m43",
        memory_task("m43"),
        Child {
            machine: "mini",
            pane: "remote:mini:pane:remote-child",
            session: "s-remote-child",
        },
    );
    runtime.publish_delivery(Arc::new(ledger), false);
    assert!(
        remote_lineage_agent(&runtime, "remote:mini:pane:remote-child")
            .hide_ai
            .is_some()
    );
    assert_eq!(
        runtime.snapshot.navigator.hide_ai_checkouts[0].device_id,
        "mini"
    );
}

/// PRD hide-ai-origin-mark Risks: the new snapshot fields move only when an
/// agent or checkout Hide started appears or ends. A ledger publish that
/// moves neither (here another agent registering) stamps no new revision.
#[test]
fn publishing_the_ledger_moves_the_snapshot_only_when_what_hide_started_moves() {
    use crate::coordination::started::tests::{
        Child, hide_started_ledger, memory_task, pane_on, register,
    };
    let mut runtime = cross_machine_runtime();
    attest_cross_machine_sessions(&mut runtime);
    runtime.refresh_agent_lineage();
    let node = runtime.node.to_string();
    let mut revision = runtime.snapshot_delta_payload(0).revision;
    let mut moved = |runtime: &mut Runtime| {
        let next = runtime.snapshot_delta_payload(revision);
        let moved = next.rest.is_some();
        revision = next.revision;
        moved
    };

    let mut ledger = hide_started_ledger(
        &node,
        "hide-memory-m72",
        memory_task("m72"),
        Child {
            machine: &node,
            pane: "local-child",
            session: "s-local-child",
        },
    );
    runtime.publish_delivery(Arc::new(ledger.clone()), false);
    assert!(moved(&mut runtime), "an agent Hide started appears once");

    runtime.publish_delivery(Arc::new(ledger.clone()), false);
    assert!(!moved(&mut runtime), "the same ledger again moves nothing");

    let other = pane_on(&node, "someone-else", "other");
    register(&mut ledger, &other);
    runtime.publish_delivery(Arc::new(ledger.clone()), false);
    assert!(
        !moved(&mut runtime),
        "another agent registering moves nothing Hide started"
    );

    for record in &mut ledger.agents {
        if record.origin.is_some() {
            record.ended = true;
        }
    }
    runtime.publish_delivery(Arc::new(ledger), false);
    assert!(moved(&mut runtime), "the agent ending publishes once");
    assert!(
        local_lineage_agent(&runtime, "local-child")
            .hide_ai
            .is_none()
    );
    assert_eq!(
        runtime.snapshot.navigator.hide_ai_checkouts.len(),
        1,
        "its checkout stays listed after the agent ends"
    );
}

/// PRD hide-ai-work-home D-14, B8, B9, B10 through the core: the tracker's
/// facts about an attempt's pull request, the agent's activity and the
/// ledger's record of who started it make the row's group and turn, and a
/// change of the pull request moves the published row without a new session
/// read.
#[test]
fn a_hide_started_row_follows_its_attempts_pull_request_into_and_out_of_needs_you() {
    use crate::coordination::started::tests::{Child, hide_started_ledger, memory_task};
    use crate::model::{HomeTurn, PullRequestChecks};
    use crate::runtime::memory_promotion::{HomeAttempt, HomePull};
    let mut runtime = cross_machine_runtime();
    attest_cross_machine_sessions(&mut runtime);
    let node = runtime.node.to_string();
    // Hide's agent has no pane parent: the operator would have it as a root.
    for agent in &mut runtime.snapshot.navigator.agents {
        if agent.pane_id == "local-child" {
            agent.declared_parent_pane_id = None;
            agent.spawned_from_machine_id = None;
        }
    }
    let mut project = workspace("project", "Project", "/repo", Vec::new());
    project.device_id.clone_from(&node);
    runtime.snapshot.navigator.workspaces = vec![project];
    let ledger = hide_started_ledger(
        &node,
        "hide-memory-m72",
        memory_task("m72"),
        Child {
            machine: &node,
            pane: "local-child",
            session: "s-local-child",
        },
    );
    let agent_id = ledger.spawns[0].child.clone().unwrap();
    runtime.publish_delivery(Arc::new(ledger), false);

    let state = |runtime: &Runtime| {
        let row = local_lineage_agent(runtime, "local-child");
        (
            row.group.clone(),
            row.hide_ai.as_ref().and_then(|mark| mark.turn),
            row.hide_ai
                .as_ref()
                .and_then(|mark| mark.workspace_id.clone()),
        )
    };
    let machine = runtime.node.clone();
    let attempt = |pull: Option<HomePull>| {
        HashMap::from([(
            agent_id.clone(),
            HomeAttempt {
                node: machine.clone(),
                root: "/repo".into(),
                pull,
            },
        )])
    };
    let pull = |checks, draft, changes_requested| {
        Some(HomePull {
            number: 924,
            draft,
            checks,
            changes_requested,
        })
    };
    // Followed, but with no pull request yet: progress only, nothing calls the operator.
    runtime.apply_homes(attempt(None));
    assert_eq!(
        state(&runtime),
        ("seen".into(), None, Some("project".into()))
    );
    // Ready and passing: the pull request waits for the operator, and stays.
    runtime.apply_homes(attempt(pull(PullRequestChecks::Passing, false, false)));
    assert_eq!(
        state(&runtime),
        (
            "needs_you".into(),
            Some(HomeTurn::PrReview { pr_number: 924 }),
            Some("project".into())
        )
    );
    // Reading it changes nothing: the turn is the home's, not an unread flag.
    for agent in &mut runtime.snapshot.navigator.agents {
        agent.unread = false;
    }
    runtime.refresh_agent_lineage();
    assert_eq!(state(&runtime).0, "needs_you");
    // A requested change, a draft or checks still running hand the turn back.
    for (checks, draft, changes) in [
        (PullRequestChecks::Passing, false, true),
        (PullRequestChecks::Passing, true, false),
        (PullRequestChecks::Pending, false, false),
    ] {
        runtime.apply_homes(attempt(pull(checks, draft, changes)));
        assert_eq!(state(&runtime).0, "seen", "{checks:?} {draft} {changes}");
    }
    // Failed checks are the operator's once the agent has stopped working on
    // them, and the row follows the agent's own activity without a new fact.
    let activity = |runtime: &mut Runtime, value: &str| {
        for agent in &mut runtime.snapshot.navigator.agents {
            if agent.pane_id == "local-child" {
                agent.activity = value.into();
            }
        }
        runtime.refresh_agent_lineage();
    };
    activity(&mut runtime, "working");
    runtime.apply_homes(attempt(pull(PullRequestChecks::Failed, false, false)));
    assert_eq!(state(&runtime).1, None, "the agent is fixing them");
    activity(&mut runtime, "stopped");
    assert_eq!(
        state(&runtime).1,
        Some(HomeTurn::CiFailed { pr_number: 924 })
    );
    assert_eq!(state(&runtime).0, "needs_you");
    // The attempt leaving the table (merged, closed or failed) leaves Needs You.
    runtime.apply_homes(HashMap::new());
    assert_eq!(state(&runtime), ("seen".into(), None, None));
}
