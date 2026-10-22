use super::*;
use crate::agent_state::tally::tasks::TaskState;
use crate::model::{PullRequestChecks, PullRequestSnapshot};

fn rows() -> Vec<SidebarAgentSnapshot> {
    project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id":"root", "agent":"claude", "agent_status":"working", "state_change_seq":1},
            {"pane_id":"child", "agent":"claude", "agent_status":"idle", "state_change_seq":2}
        ]}))
        .unwrap(),
    )
    .agents
}

#[test]
fn a_task_is_its_root_and_every_descendant_with_their_projects_and_devices() {
    // PRD overview-device-tasks B10, B13-B15: the task belongs to its root's
    // project; a child in another project of the same device is `→ sasu`, a
    // child on another device is `→ mini` there and a dashed, uncounted
    // entry on that device's project.
    let mut runtime = runtime();
    let local_id = runtime.snapshot.navigator.devices[0].id.clone();
    let mut agents = rows();
    agents[1].pane_id = "spec".into();
    agents[1].lineage_parent_pane_id = Some("root".into());
    let mut helper = agents[1].clone();
    helper.pane_id = "helper".into();
    helper.lineage_parent_pane_id = None;
    agents.push(helper);
    runtime.snapshot.navigator.agents = agents;
    let project = |id: &str, names: &[&str]| {
        let mut project = workspace(
            id,
            id,
            "/fixture",
            names
                .iter()
                .map(|name| checkout(id, name, "/fixture", Some(pane(name, "/fixture"))))
                .collect(),
        );
        project.device_id.clone_from(&local_id);
        project
    };
    runtime.snapshot.navigator.workspaces = vec![
        project("ide", &["root", "helper"]),
        project("sasu", &["spec"]),
    ];
    assert!(runtime.refresh_agent_scopes());
    let ide = &runtime.snapshot.navigator.workspaces[0].agent_scope.tasks;
    assert_eq!(
        ide.cards
            .iter()
            .map(|c| c.root.as_str())
            .collect::<Vec<_>>(),
        ["root", "helper"],
        "working before resting"
    );
    assert_eq!(ide.cards[0].members, ["root", "spec"]);
    assert_eq!(ide.cards[0].out_projects[0].project_id, "sasu");
    assert!(
        runtime.snapshot.navigator.workspaces[1]
            .agent_scope
            .tasks
            .cards
            .is_empty(),
        "a child is no task of its own"
    );
    assert!(!runtime.refresh_agent_scopes());

    let mut remote_device = crate::workspace::local_device(&crate::node::test_node(), None, None);
    remote_device.id = "other-device".into();
    remote_device.label = Some("mini".into());
    remote_device.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(remote_device);
    let mut remote_project = project("remote-docs", &["remote:other-device:pane:p1"]);
    remote_project.device_id = "other-device".into();
    let mut remote_agent = runtime.snapshot.navigator.agents[1].clone();
    remote_agent.pane_id = "remote:other-device:pane:p1".into();
    remote_agent.lineage_parent_pane_id = Some("root".into());
    remote_agent.group = "needs_you".into();
    runtime.snapshot.status.remote = vec![RemoteStatusSnapshot {
        target_id: "other-device".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![remote_project],
            agents: vec![remote_agent],
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
        unreachable_since_unix_ms: None,
    }];
    assert!(runtime.refresh_agent_scopes());
    let root = &runtime.snapshot.navigator.workspaces[0]
        .agent_scope
        .tasks
        .cards[0];
    assert_eq!(
        root.members,
        ["root", "spec", "remote:other-device:pane:p1"]
    );
    assert_eq!(root.out_devices[0].device_id, "other-device");
    assert_eq!(root.out_devices[0].label.as_deref(), Some("mini"));
    let remote = &runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces[0]
        .agent_scope
        .tasks;
    assert!(remote.cards.is_empty());
    assert_eq!(
        remote.needs_you, 0,
        "a delegated child's demand is not counted twice"
    );
    assert_eq!(remote.incoming[0].root, "root");
    assert_eq!(remote.incoming[0].root_device_id, local_id);
    assert_eq!(
        remote.incoming[0].root_device_label, None,
        "the core's own machine is named by the shell"
    );

    // An unresolved parent machine leaves the child an ordinary task there (B15).
    runtime.snapshot.status.remote[0]
        .session
        .as_mut()
        .unwrap()
        .agents[0]
        .lineage_parent_pane_id = None;
    assert!(runtime.refresh_agent_scopes());
    let remote = &runtime.snapshot.status.remote[0]
        .session
        .as_ref()
        .unwrap()
        .workspaces[0]
        .agent_scope
        .tasks;
    assert!(remote.incoming.is_empty());
    assert_eq!(remote.cards[0].root, "remote:other-device:pane:p1");
    assert_eq!(remote.needs_you, 1);
    assert!(
        runtime.snapshot.navigator.workspaces[0]
            .agent_scope
            .tasks
            .cards[0]
            .out_devices
            .is_empty()
    );
}

#[test]
fn revisit_tasks_stand_oldest_first_and_the_current_state_wins() {
    // PRD revisit-group B6, B7, B13: a quiet ★ or read ask stands in
    // 다시 볼 것, oldest first; a working ★ stays in 일하는 중.
    use crate::agent_state::sessions::Revisit;
    let mut runtime = runtime();
    let mut base = rows().remove(1);
    base.lineage_parent_pane_id = None;
    let agent = |id: &str, group: &str, activity: &str, since: Option<u64>| {
        let mut agent = base.clone();
        agent.pane_id = id.into();
        agent.group = group.into();
        agent.demand = "none".into();
        agent.activity = activity.into();
        agent.revisit = since.map(|since_unix_ms| Revisit {
            starred: true,
            read_ask: false,
            since_unix_ms,
        });
        agent
    };
    runtime.snapshot.navigator.agents = vec![
        agent("newer", "seen", "stopped", Some(20)),
        agent("older", "seen", "stopped", Some(10)),
        agent("busy", "working", "working", Some(5)),
        agent("plain", "seen", "stopped", None),
    ];
    let names = ["newer", "older", "busy", "plain"];
    let checkouts: Vec<_> = names
        .iter()
        .map(|name| {
            checkout(
                "project",
                name,
                &format!("/{name}"),
                Some(pane(name, &format!("/{name}"))),
            )
        })
        .collect();
    runtime.snapshot.navigator.workspaces =
        vec![workspace("project", "Project", "/fixture", checkouts)];
    assert!(runtime.refresh_agent_scopes());
    let tasks = &runtime.snapshot.navigator.workspaces[0].agent_scope.tasks;
    assert_eq!(
        tasks
            .cards
            .iter()
            .map(|c| (c.root.as_str(), c.state))
            .collect::<Vec<_>>(),
        [
            ("busy", TaskState::Working),
            ("older", TaskState::Revisit),
            ("newer", TaskState::Revisit),
            ("plain", TaskState::Resting),
        ]
    );
}

#[test]
fn tasks_stand_by_state_and_fold_a_merged_worktree_as_cleanup() {
    // B7, B9: Needs You, blocked, working, waiting, done, then resting and
    // the cleanup fold; the stage bar follows the task's PRs and worktrees.
    let mut runtime = runtime();
    let mut base = rows().remove(1);
    base.lineage_parent_pane_id = None;
    let agent = |id: &str, group: &str, demand: &str, activity: &str| {
        let mut agent = base.clone();
        agent.pane_id = id.into();
        agent.group = group.into();
        agent.demand = demand.into();
        agent.activity = activity.into();
        agent
    };
    let mut waiting = agent("waiting", "working", "none", "stopped");
    waiting.wait = Some(crate::model::AgentWait::Children);
    runtime.snapshot.navigator.agents = vec![
        agent("resting", "seen", "none", "stopped"),
        agent("merged", "seen", "none", "stopped"),
        agent("done", "done", "none", "stopped"),
        waiting,
        agent("working", "working", "none", "working"),
        agent("blocked", "needs_you", "error", "stopped"),
        agent("asks", "needs_you", "question", "stopped"),
    ];
    let names = [
        "resting", "merged", "done", "waiting", "working", "blocked", "asks",
    ];
    let mut checkouts: Vec<_> = names
        .iter()
        .map(|name| {
            checkout(
                "project",
                name,
                &format!("/{name}"),
                Some(pane(name, &format!("/{name}"))),
            )
        })
        .collect();
    checkouts[0].is_primary = true;
    let merged = &mut checkouts[1];
    merged.is_worktree = true;
    let mut project = workspace("project", "Project", "/fixture", checkouts);
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    // The merged PR reaches the row through its session's link record.
    let pull = crate::request_view::AgentPullRequestSnapshot {
        number: 7,
        title: "Seven".into(),
        url: "https://github.com/acme/ide/pull/7".into(),
        badge: crate::model::PullRequestBadge::Merged,
        checks: PullRequestChecks::Passing,
        review: None,
        is_draft: false,
        head_branch: "seven".into(),
        closing_issues: vec![],
        live: true,
        duty: true,
        provisional: false,
        created: true,
        settled_at_unix_ms: None,
    };
    let row = &mut runtime.snapshot.navigator.agents[1];
    row.request = Some(crate::request_view::AgentRequestSnapshot {
        verb: crate::agent_state::RequestVerb::Idle,
        verb_since_unix_ms: 0,
        line: None,
        end: None,
        request: None,
        request_summary: None,
        recent_turns: Vec::new(),
        later_by: None,
        pull_requests: vec![pull],
    });
    row.state.pr = crate::agent_state::sessions::own_prs(row);
    assert!(runtime.refresh_agent_scopes());
    let tasks = &runtime.snapshot.navigator.workspaces[0].agent_scope.tasks;
    assert_eq!(
        tasks
            .cards
            .iter()
            .map(|c| (c.root.as_str(), c.state))
            .collect::<Vec<_>>(),
        [
            ("asks", TaskState::NeedsYou),
            ("blocked", TaskState::Blocked),
            ("working", TaskState::Working),
            ("waiting", TaskState::Waiting),
            ("done", TaskState::Done),
            ("resting", TaskState::Resting),
            ("merged", TaskState::Cleanup),
        ]
    );
    assert_eq!(tasks.needs_you, 2);
    let stage = |id: &str| tasks.cards.iter().find(|c| c.root == id).unwrap().stage;
    assert_eq!(stage("merged"), Some(4));
    assert_eq!(
        stage("resting"),
        None,
        "a conversation in the primary checkout has no bar"
    );
    let row = &mut runtime.snapshot.navigator.agents[1];
    let mut provisional = row.request.as_ref().unwrap().pull_requests[0].clone();
    provisional.number = 8;
    provisional.url = "https://github.com/acme/ide/pull/8".into();
    provisional.badge = crate::model::PullRequestBadge::Open;
    provisional.provisional = true;
    provisional.duty = false;
    provisional.created = false;
    row.request
        .as_mut()
        .unwrap()
        .pull_requests
        .push(provisional);
    row.state.pr = crate::agent_state::sessions::own_prs(row);
    assert_eq!(
        row.state.pr.as_ref().unwrap().count,
        2,
        "history is displayed"
    );
    runtime.refresh_agent_scopes();
    let card = runtime.snapshot.navigator.workspaces[0]
        .agent_scope
        .tasks
        .cards
        .iter()
        .find(|c| c.root == "merged")
        .unwrap();
    assert_eq!(
        card.stage,
        Some(4),
        "history cannot change current task progress"
    );
    assert_eq!(card.state, TaskState::Cleanup);
    // A PR merged before the operator's next request keeps its mark, but the
    // task's progress reads only the PRs of the current request.
    let set_live = |runtime: &mut crate::runtime::Runtime, live: bool| {
        let row = &mut runtime.snapshot.navigator.agents[1];
        row.request.as_mut().unwrap().pull_requests[0].live = live;
        row.state.pr = crate::agent_state::sessions::own_prs(row);
    };
    set_live(&mut runtime, false);
    assert_eq!(
        runtime.snapshot.navigator.agents[1]
            .state
            .pr
            .as_ref()
            .unwrap()
            .count,
        2
    );
    runtime.refresh_agent_scopes();
    let card = runtime.snapshot.navigator.workspaces[0]
        .agent_scope
        .tasks
        .cards
        .iter()
        .find(|c| c.root == "merged")
        .unwrap();
    assert_ne!(
        card.stage,
        Some(4),
        "an earlier merge is not this request's"
    );
    assert_ne!(card.state, TaskState::Cleanup);
    set_live(&mut runtime, true);
    runtime.refresh_agent_scopes();
    runtime.snapshot.navigator.workspaces[0].checkouts[1].exists = false;
    assert!(
        runtime.refresh_agent_scopes(),
        "a removed worktree leaves nothing to clean"
    );
    let tasks = &runtime.snapshot.navigator.workspaces[0].agent_scope.tasks;
    assert_eq!(tasks.cards.last().unwrap().state, TaskState::Resting);
}

#[test]
fn a_device_publishes_every_project_s_tasks_in_one_order() {
    // PRD sidebar-task-lens B4, B5, B9, D-40: the 태스크 list's 상태별 view is
    // the device's every task across its projects, in the same groups and
    // order as one project's; a resolved task rests whatever its own state.
    let mut runtime = runtime();
    let local_id = runtime.snapshot.navigator.devices[0].id.clone();
    let mut base = rows().remove(1);
    base.lineage_parent_pane_id = None;
    let agent = |id: &str, group: &str, activity: &str, at: &str| {
        let mut agent = base.clone();
        agent.pane_id = id.into();
        agent.group = group.into();
        agent.activity = activity.into();
        agent.last_activity = at.into();
        agent
    };
    let mut resolved = agent("resolved", "needs_you", "stopped", "5");
    resolved.demand = "question".into();
    resolved.resolved = Some(crate::agent_state::sessions::Resolution {
        at_unix_ms: 1,
        source: crate::agent_state::sessions::ResolveSource::Operator,
        session: None,
        activity: "stopped".into(),
    });
    runtime.snapshot.navigator.agents = vec![
        agent("old", "working", "working", "1"),
        agent("asks", "needs_you", "stopped", "2"),
        agent("new", "working", "working", "3"),
        resolved,
    ];
    let project = |id: &str, names: &[&str]| {
        let mut project = workspace(
            id,
            id,
            &format!("/{id}"),
            names
                .iter()
                .map(|name| {
                    checkout(
                        id,
                        name,
                        &format!("/{id}/{name}"),
                        Some(pane(name, &format!("/{id}/{name}"))),
                    )
                })
                .collect(),
        );
        project.device_id.clone_from(&local_id);
        project
    };
    runtime.snapshot.navigator.workspaces = vec![
        project("ide", &["old", "asks"]),
        project("site", &["new", "resolved"]),
    ];
    runtime.snapshot.navigator.agents[1].demand = "question".into();
    assert!(runtime.refresh_agent_scopes());
    let device = &runtime.snapshot.navigator.devices[0].agent_scope.tasks;
    assert_eq!(
        device
            .cards
            .iter()
            .map(|c| (c.root.as_str(), c.state))
            .collect::<Vec<_>>(),
        [
            ("asks", TaskState::NeedsYou),
            ("new", TaskState::Working),
            ("old", TaskState::Working),
            ("resolved", TaskState::Resting),
        ],
        "every project's tasks, by state, the most recent first"
    );
    assert_eq!(device.needs_you, 1);
    let site = &runtime.snapshot.navigator.workspaces[1].agent_scope.tasks;
    assert_eq!(
        site.cards
            .iter()
            .map(|c| c.root.as_str())
            .collect::<Vec<_>>(),
        ["new", "resolved"],
        "a project's own list keeps only its tasks"
    );
}

#[test]
fn close_consequences_keep_unknown_priority_and_outside_descendant_counts() {
    let mut runtime = runtime();
    let mut agents = rows();
    let mut neighbour = agents[0].clone();
    neighbour.pane_id = "neighbour".into();
    neighbour.requires_close_status_check = true;
    neighbour.state.subtree = "unknown";
    agents[0].close_descendant_pane_ids = vec!["child".into(), "gone".into()];
    agents[0].lineage_child_pane_ids = vec!["child".into()];
    agents[1].lineage_parent_pane_id = Some("root".into());
    agents[1].lineage_depth = 2;
    agents[1].state.subtree = "waiting";
    agents.push(neighbour);
    runtime.snapshot.navigator.agents = agents;
    let mut root = pane("root", "/fixture");
    root.requires_close_confirmation = true;
    let mut neighbour = pane("neighbour", "/fixture");
    neighbour.herdr_label = Some("second pane".into());
    let mut inside = checkout("project", "inside", "/fixture", Some(root));
    inside.tabs[0].panes.push(neighbour);
    let outside = checkout(
        "project",
        "outside",
        "/outside",
        Some(pane("child", "/outside")),
    );
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture",
        vec![inside, outside],
    )];
    assert!(runtime.refresh_agent_scopes());
    let close = &runtime.snapshot.navigator.agent_scope.closes["root\0neighbour"];
    assert_eq!(
        serde_json::to_value(&close.decision).unwrap(),
        serde_json::json!({"action":"status_unknown", "label":"second pane"})
    );
    assert_eq!(close.stop_work.unknown, Some(1));
    let subtree = close.subtree.as_ref().unwrap();
    assert_eq!(subtree.ids, ["child"]);
    assert_eq!(
        subtree
            .rows
            .iter()
            .map(|r| (r.pane_id.as_str(), r.depth, r.target))
            .collect::<Vec<_>>(),
        [("root", 0, true), ("child", 2, false)]
    );
    assert_eq!(subtree.counts.waiting, 1);
    assert!(!subtree.unknown);
    assert!(!subtree.target_unknown);
    assert!(close.subtree_all.as_ref().unwrap().target_unknown);
    assert_eq!(close.subtree_all.as_ref().unwrap().rows.len(), 3);
    assert!(
        runtime.snapshot.navigator.agent_scope.closes["root\0neighbour\0child"]
            .subtree
            .is_none()
    );
    assert!(!runtime.refresh_agent_scopes());
    runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0].panes[0]
        .requires_close_status_check = true;
    assert!(
        runtime.refresh_agent_scopes(),
        "a pane-only status change refreshes the close notice"
    );
    let close = &runtime.snapshot.navigator.agent_scope.closes["root\0neighbour"];
    assert_eq!(close.stop_work.unknown, Some(0));
}

#[test]
fn scopes_keep_first_overview_owner_last_badge_owner_and_restore_rebuilt_catalog_values() {
    let mut runtime = runtime();
    let mut rows = rows();
    let mut duplicate = rows[0].clone();
    duplicate.activity = "stopped".into();
    duplicate.group = "idle".into();
    duplicate.identity_label = "Later duplicate".into();
    duplicate.state = crate::agent_state::turn::row_state(&duplicate);
    rows.push(duplicate);
    runtime.snapshot.navigator.agents = rows;
    let first = checkout(
        "project",
        "first",
        "/fixture/first",
        Some(pane("root", "/fixture/first")),
    );
    let last = checkout(
        "project",
        "last",
        "/fixture/last",
        Some(pane("root", "/fixture/last")),
    );
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture",
        vec![first, last],
    )];
    crate::agent_state::sync_checkout_agent_summaries(
        &mut runtime.snapshot.navigator.workspaces,
        &runtime.snapshot.navigator.agents,
    );
    assert!(runtime.refresh_agent_scopes());
    let project = &runtime.snapshot.navigator.workspaces[0];
    assert_eq!(
        project.agent_scope.total, 2,
        "physical rows are not deduplicated"
    );
    assert_eq!(
        project
            .agent_scope
            .rows
            .iter()
            .map(|r| r.occurrence)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(
        project
            .agent_scope
            .sections
            .iter()
            .flat_map(|s| &s.rows)
            .map(|r| r.occurrence)
            .collect::<Vec<_>>(),
        [0, 1],
        "physical section rows keep their distinct source occurrence"
    );
    assert_eq!(project.agent_scope.overview_total, 1);
    assert_eq!(project.agent_scope.buckets.working, 1);
    assert_eq!(project.agent_scope.buckets.resting, 0);
    assert_eq!(
        project.agent_scope.requests.counts[&crate::agent_state::RequestVerb::Working],
        1
    );
    assert_eq!(project.agent_scope.tasks.cards[0].checkouts, ["first"]);
    assert_eq!(project.agent_scope.members[0].checkout_id, "first");
    assert_eq!(project.agent_scope.places["root"].checkout_id, "first");
    assert_eq!(runtime.snapshot.navigator.agent_scope.listed.len(), 3);
    assert_eq!(runtime.snapshot.navigator.agent_scope.listed[2].index, 2);
    assert_eq!(project.checkouts[0].agent_summary.working, 0);
    assert_eq!(project.checkouts[1].agent_summary.working, 1);
    assert_eq!(project.agent_scope.marks.going, 1);
    assert!(
        !runtime.refresh_agent_scopes(),
        "an unchanged projection publishes nothing"
    );
    let expected = runtime.snapshot.navigator.workspaces.clone();
    runtime.snapshot.navigator.workspaces[0].agent_scope = Default::default();
    runtime.snapshot.navigator.workspaces[0].checkouts[0].agent_scope = Default::default();
    assert!(
        runtime.refresh_agent_scopes(),
        "a catalog rebuilt from facts gets its cached scopes back"
    );
    assert_eq!(runtime.snapshot.navigator.workspaces, expected);
    assert!(!runtime.refresh_agent_scopes());
    runtime.snapshot.navigator.workspaces[0].checkouts[0]
        .tabs
        .clear();
    assert!(
        runtime.refresh_agent_scopes(),
        "pane ownership invalidates the scope"
    );
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].agent_scope.members[0].checkout_id,
        "last"
    );
}

#[test]
fn disconnected_devices_zero_the_physical_tile_but_keep_overview_members_from_the_last_session() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents.clear();
    runtime.snapshot.navigator.workspaces.clear();
    let mut device = crate::workspace::local_device(&crate::node::test_node(), None, None);
    device.id = "mini".into();
    device.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(device);
    let mut project = workspace(
        "remote-project",
        "Remote",
        "/fixture",
        vec![checkout(
            "remote-project",
            "remote-checkout",
            "/fixture",
            Some(pane("root", "/fixture")),
        )],
    );
    project.device_id = "mini".into();
    runtime.snapshot.status.remote = vec![RemoteStatusSnapshot {
        target_id: "mini".into(),
        state: "connected".into(),
        message: None,
        herdr_version: None,
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![project],
            agents: rows(),
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
        unreachable_since_unix_ms: None,
    }];
    assert!(runtime.refresh_agent_scopes());
    let scope = &runtime
        .snapshot
        .navigator
        .devices
        .iter()
        .find(|d| d.id == "mini")
        .unwrap()
        .agent_scope;
    assert_eq!((scope.total, scope.overview_total), (2, 1));
    assert_eq!(scope.listed.len(), 2);
    assert!(scope.places_live);
    runtime.snapshot.status.remote[0].state = "stale".into();
    assert!(runtime.refresh_agent_scopes());
    let scope = &runtime
        .snapshot
        .navigator
        .devices
        .iter()
        .find(|d| d.id == "mini")
        .unwrap()
        .agent_scope;
    assert_eq!(
        (scope.total, scope.groups.working, scope.overview_total),
        (0, 0, 1)
    );
    assert_eq!(runtime.snapshot.navigator.agent_scope.total, 0);
    assert!(scope.listed.is_empty());
    assert!(!scope.places_live);
    assert!(
        scope.places.contains_key("root"),
        "search context retains the catalog's place"
    );
    assert_eq!(runtime.snapshot.navigator.agent_scope.overview_total, 1);
    assert!(!runtime.refresh_agent_scopes());
}

#[test]
fn checkout_trees_and_folded_badges_preserve_cross_checkout_lineage_and_priority() {
    let mut runtime = runtime();
    let mut agents = rows();
    let mut done = agents[1].clone();
    done.pane_id = "done".into();
    done.group = "done".into();
    done.demand = "none".into();
    done.activity = "stopped".into();
    done.state = crate::agent_state::turn::row_state(&done);
    agents.push(done);
    agents[0].lineage_child_pane_ids = vec!["child".into(), "done".into(), "gone".into()];
    agents[0].lineage_collapsed = true;
    agents[1].lineage_parent_pane_id = Some("root".into());
    agents[1].delegated = true;
    agents[1].group = "seen".into();
    agents[1].demand = "question".into();
    agents[1].state = crate::agent_state::turn::row_state(&agents[1]);
    agents[2].lineage_parent_pane_id = Some("root".into());
    let mut first = checkout(
        "project",
        "first",
        "/fixture/first",
        Some(pane("root", "/fixture/first")),
    );
    first.tabs[0].panes.push(pane("child", "/fixture/first"));
    let second = checkout(
        "project",
        "second",
        "/fixture/second",
        Some(pane("done", "/fixture/second")),
    );
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture",
        vec![first, second],
    )];
    runtime.snapshot.navigator.agents = agents;
    assert!(runtime.refresh_agent_scopes());
    let project = &runtime.snapshot.navigator.workspaces[0];
    let tree = &project.checkouts[0].agent_scope.tree;
    assert_eq!(
        tree.rows
            .iter()
            .map(|r| (r.pane_id.as_str(), r.depth))
            .collect::<Vec<_>>(),
        vec![("root", 0), ("child", 1), ("done", 1)]
    );
    assert_eq!(
        tree.visible_rows
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["root"]
    );
    assert_eq!(tree.shown, vec!["done", "root"]);
    assert_eq!(tree.more, 1);
    assert!(
        !tree.needs_you,
        "delegated question and a done descendant do not turn the card yellow"
    );
    assert!(
        project.checkouts[1].agent_scope.tree.needs_you,
        "that done row is a root relative to its own checkout"
    );
    let scope = &runtime.snapshot.navigator.agent_scope;
    let folded = &scope.folded["root"];
    assert_eq!(folded.badge_descendants, 1);
    assert_eq!(folded.badge_counts.question, 1);
    assert_eq!(folded.badge_children, vec!["child"]);
    assert_eq!(scope.children["root"], vec!["child", "done"]);
    assert_eq!(scope.descendants["root"], 2);
    assert_eq!(folded.tiers.len(), 1);
    assert_eq!(folded.tiers[0][0].candidates, vec!["done"]);
    assert_eq!(folded.overflow, 0);
    runtime.snapshot.navigator.agents[0].lineage_collapsed = false;
    assert!(runtime.refresh_agent_scopes());
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0].checkouts[0]
            .agent_scope
            .tree
            .visible_rows
            .len(),
        3
    );
    assert!(!runtime.refresh_agent_scopes());
}

#[test]
fn pr_board_follows_the_record_s_live_session_and_tracks_github_and_issue_changes() {
    let mut runtime = runtime();
    let mut pr: PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature","base_branch":"main","badge":"open","checks":"failed","is_draft":false,
        "closing_issues":[{"repository":"acme/app","number":7}]
    })).unwrap();
    pr.work.mine = Some(true);
    pr.work.asked_of_me = crate::model::ReviewAsk::None;
    pr.work.queue = crate::model::MergeQueuePlace::NotQueued;
    pr.work.merge_state = crate::model::MergeState::Blocked;
    // The record says the maker's session worked on the branch; the maker
    // is in another checkout and still working there.
    pr.work.sessions = vec!["s-maker".into()];
    let mut agents = rows();
    agents[0].session_id = Some("s-maker".into());
    agents[0].activity = "working".into();
    agents[0].changed_at_unix_ms = Some(unix_milliseconds());
    agents[0].state = crate::agent_state::turn::row_state(&agents[0]);
    // A finished agent of the same folder is not the pull request's agent.
    agents[1].group = "done".into();
    agents[1].activity = "stopped".into();
    agents[1].state = crate::agent_state::turn::row_state(&agents[1]);
    let mut branch = checkout(
        "project",
        "feature",
        "/fixture/feature",
        Some(pane("child", "/fixture/feature")),
    );
    branch.pull_request = Some(pr.clone());
    branch.exists = true;
    let mut project = workspace("project", "Project", "/fixture", vec![branch]);
    project.home_issues.repository = Some("acme/app".into());
    project.pull_requests = vec![pr];
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.navigator.agents = agents;
    assert!(runtime.refresh_agent_scopes());
    let board = &runtime.snapshot.navigator.workspaces[0].agent_scope.prs;
    assert_eq!(
        board.rows[0]
            .agents
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["root"]
    );
    assert_eq!(board.rows[0].group, "working");
    assert_eq!(board.rows[0].issue.as_ref().unwrap().label, "#7");
    assert_eq!(board.groups[0].numbers, vec![42]);
    assert_eq!(board.needs_you, 0);
    runtime.snapshot.navigator.agents[0].activity = "stopped".into();
    runtime.snapshot.navigator.agents[0].group = "seen".into();
    runtime.snapshot.navigator.agents[0].state =
        crate::agent_state::turn::row_state(&runtime.snapshot.navigator.agents[0]);
    assert!(runtime.refresh_agent_scopes());
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0]
            .agent_scope
            .prs
            .rows[0]
            .group,
        "blocked",
        "no live working agent: the failed checks decide"
    );
    runtime.snapshot.navigator.workspaces[0].pull_requests[0].checks = PullRequestChecks::Passing;
    runtime.snapshot.navigator.workspaces[0].pull_requests[0]
        .work
        .merge_state = crate::model::MergeState::Clean;
    assert!(
        runtime.refresh_agent_scopes(),
        "GitHub changes invalidate work scope even when agents stay still"
    );
    let board = &runtime.snapshot.navigator.workspaces[0].agent_scope.prs;
    assert_eq!(board.rows[0].group, "needs_you");
    assert_eq!(board.rows[0].reason.kind, "merge");
    assert_eq!(board.needs_you, 1);
    assert!(!runtime.refresh_agent_scopes());
}

#[test]
fn a_working_agent_turns_stalled_and_a_quiet_pull_request_old_with_the_clock_alone() {
    let mut runtime = runtime();
    let mut pr: PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature","base_branch":"main","badge":"open","checks":"passing","is_draft":false,
        "closing_issues":[], "updated_at_unix_ms": 1_000_000
    })).unwrap();
    pr.work.mine = Some(true);
    pr.work.asked_of_me = crate::model::ReviewAsk::None;
    pr.work.queue = crate::model::MergeQueuePlace::NotQueued;
    pr.work.merge_state = crate::model::MergeState::Clean;
    pr.work.sessions = vec!["s-maker".into()];
    let mut agents = rows();
    agents[0].session_id = Some("s-maker".into());
    agents[0].activity = "working".into();
    agents[0].changed_at_unix_ms = Some(1_000_000);
    agents[0].state = crate::agent_state::turn::row_state(&agents[0]);
    let mut project = workspace(
        "project",
        "Project",
        "/fixture",
        vec![checkout("project", "feature", "/fixture/feature", None)],
    );
    project.pull_requests = vec![pr];
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.navigator.agents = agents;
    let quiet = crate::delivery::INACTIVITY_MS;
    assert!(runtime.tick_work_clock(1_000_000 + 60_000));
    fn row(runtime: &Runtime) -> crate::agent_state::work::board::Row {
        runtime.snapshot.navigator.workspaces[0]
            .agent_scope
            .prs
            .rows[0]
            .clone()
    }
    assert_eq!(
        (row(&runtime).group, row(&runtime).reason.kind),
        ("working", "working")
    );
    assert!(
        !runtime.tick_work_clock(1_000_000 + 60_000 + 1_000),
        "the same minute judges nothing"
    );
    assert!(runtime.tick_work_clock(1_000_000 + quiet + 60_000));
    assert_eq!(
        (row(&runtime).group, row(&runtime).reason.kind),
        ("needs_you", "check")
    );
    assert!(runtime.tick_work_clock(1_000_000 + crate::agent_state::work::board::OLD_MS + 120_000));
    assert_eq!(row(&runtime).fold, Some("old"));
}

/// A node's project lists its pull requests as this machine's does (PRD
/// core-host-node-move D-29), so the clock judges its PRs list too while this
/// machine lists none.
#[test]
fn the_clock_judges_a_nodes_pull_requests_while_this_machine_lists_none() {
    let mut runtime = runtime();
    let mut pr: PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature","base_branch":"main","badge":"open","checks":"passing","is_draft":false,
        "closing_issues":[], "updated_at_unix_ms": 1_000_000
    })).unwrap();
    pr.work.mine = Some(true);
    pr.work.asked_of_me = crate::model::ReviewAsk::None;
    pr.work.queue = crate::model::MergeQueuePlace::NotQueued;
    pr.work.merge_state = crate::model::MergeState::Clean;
    pr.work.sessions = vec!["s-maker".into()];
    let mut agents = rows();
    agents[0].session_id = Some("s-maker".into());
    agents[0].activity = "working".into();
    agents[0].changed_at_unix_ms = Some(1_000_000);
    agents[0].state = crate::agent_state::turn::row_state(&agents[0]);
    let mut project = workspace(
        "remote:mbp:project:1",
        "Project",
        "/fixture",
        vec![checkout(
            "remote:mbp:project:1",
            "feature",
            "/fixture/feature",
            None,
        )],
    );
    project.device_id = "mbp".into();
    project.pull_requests = vec![pr];
    let mut session = super::device_catalog::session(vec![project]);
    session.agents = agents;
    let mut node = super::device_catalog::connected_status("mbp");
    node.session = Some(session);
    runtime.snapshot.status.remote.push(node);
    fn row(runtime: &Runtime) -> crate::agent_state::work::board::Row {
        runtime.snapshot.status.remote[0]
            .session
            .as_ref()
            .unwrap()
            .workspaces[0]
            .agent_scope
            .prs
            .rows[0]
            .clone()
    }
    assert!(runtime.tick_work_clock(1_000_000 + 60_000));
    assert_eq!(
        (row(&runtime).group, row(&runtime).reason.kind),
        ("working", "working")
    );
    assert!(runtime.tick_work_clock(1_000_000 + crate::delivery::INACTIVITY_MS + 60_000));
    assert_eq!(
        (row(&runtime).group, row(&runtime).reason.kind),
        ("needs_you", "check")
    );
}

#[test]
fn raised_sections_keep_five_questions_three_completions_and_first_number_owner() {
    let mut runtime = runtime();
    let prototype = rows()[0].clone();
    let agents: Vec<_> = (0..13)
        .map(|i| {
            let mut row = prototype.clone();
            row.pane_id = format!("p{i}");
            row.group = if i < 7 { "needs_you" } else { "done" }.into();
            row.state = crate::agent_state::turn::row_state(&row);
            row
        })
        .collect();
    let mut first = checkout(
        "project",
        "first",
        "/fixture/first",
        Some(pane("p0", "/fixture/first")),
    );
    first.tabs[0].panes = (0..12)
        .map(|i| pane(&format!("p{i}"), "/fixture/first"))
        .collect();
    let duplicate = checkout(
        "project",
        "second",
        "/fixture/second",
        Some(pane("p0", "/fixture/second")),
    );
    runtime.snapshot.navigator.agents = agents;
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture",
        vec![first, duplicate],
    )];
    assert!(runtime.refresh_agent_scopes());
    let scope = &runtime
        .snapshot
        .navigator
        .devices
        .iter()
        .find(|d| d.kind != "remote")
        .unwrap()
        .agent_scope;
    assert_eq!(
        scope.raised[0]
            .shown
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["p0", "p1", "p2", "p3", "p4"]
    );
    assert_eq!(
        scope.raised[0]
            .more
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["p5", "p6"]
    );
    assert_eq!(
        scope.raised[1]
            .shown
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["p7", "p8", "p9"]
    );
    assert_eq!(
        scope.raised[1]
            .more
            .iter()
            .map(|r| r.pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["p10", "p11"]
    );
    assert_eq!(scope.owners["p0"], "first");
    assert!(
        !scope.owners.contains_key("p12"),
        "an agent outside every drawn checkout is not raised or numbered in Projects"
    );
}

#[test]
fn palette_lineage_keeps_first_anchor_and_last_ancestor_occurrences() {
    let mut runtime = runtime();
    let mut agents = rows();
    agents[1].lineage_parent_pane_id = Some("root".into());
    let mut later_child = agents[1].clone();
    later_child.identity_label = "Later child".into();
    later_child.lineage_parent_pane_id = None;
    let mut later_root = agents[0].clone();
    later_root.identity_label = "Later parent".into();
    agents.extend([later_child, later_root]);
    let pr: PullRequestSnapshot = serde_json::from_value(serde_json::json!({
        "number":42,"title":"Ship","url":"https://github.com/acme/app/pull/42",
        "head_branch":"feature","base_branch":"main","badge":"open","checks":"passing","is_draft":false,"closing_issues":[]
    })).unwrap();
    let mut branch = checkout(
        "project",
        "feature",
        "/fixture/feature",
        Some(pane("child", "/fixture/feature")),
    );
    branch.pull_request = Some(pr.clone());
    branch.exists = true;
    let mut project = workspace("project", "Project", "/fixture", vec![branch]);
    project.pull_requests = vec![pr];
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.snapshot.navigator.agents = agents;
    assert!(runtime.refresh_agent_scopes());
    let relation = &runtime.snapshot.navigator.devices[0].agent_scope.relations["child"][0];
    assert_eq!(
        relation
            .rows
            .iter()
            .map(|r| (r.pane_id.as_str(), r.occurrence, r.tag))
            .collect::<Vec<_>>(),
        [("child", 0, Some("here")), ("root", 1, Some("parent"))]
    );
    assert!(!runtime.refresh_agent_scopes());
}

/// PRD hide-ai-work-home B1-B4, D-13: a row its home owns is in no operator
/// list while the home shows its progress, and in Needs You, with the lists
/// that count it, once the operator has something to do: the home's turn, or
/// the agent's own unread question.
#[test]
fn a_home_owned_row_is_listed_only_while_the_operator_has_something_to_do() {
    use crate::coordination::TaskHome;
    use crate::model::{HideAiSnapshot, HomeTurn};
    let ask = |agent: &mut SidebarAgentSnapshot| {
        agent.demand = "question".into();
        agent.unread = true;
    };
    let review = |_: &mut SidebarAgentSnapshot| {};
    type Case = (
        &'static str,
        Option<HomeTurn>,
        fn(&mut SidebarAgentSnapshot),
        &'static str,
    );
    let cases: [Case; 4] = [
        ("progress only", None, review, "seen"),
        (
            "a pull request to review",
            Some(HomeTurn::PrReview { pr_number: 924 }),
            review,
            "needs_you",
        ),
        (
            "failed checks",
            Some(HomeTurn::CiFailed { pr_number: 924 }),
            review,
            "needs_you",
        ),
        ("its own unread question", None, ask, "needs_you"),
    ];
    for (case, turn, act, group) in cases {
        let mut runtime = runtime();
        let mut agents = rows();
        agents[1].hide_ai = Some(HideAiSnapshot {
            task: None,
            home: Some(TaskHome::Memory),
            workspace_id: Some("p".into()),
            turn,
        });
        act(&mut agents[1]);
        let workspaces = vec![workspace(
            "project",
            "Project",
            "/fixture",
            vec![
                checkout(
                    "project",
                    "main",
                    "/fixture",
                    Some(pane("root", "/fixture")),
                ),
                checkout(
                    "project",
                    "memory",
                    "/fixture/m",
                    Some(pane("child", "/fixture/m")),
                ),
            ],
        )];
        crate::agent_state::apply_lineage(&mut agents, &workspaces, &[]);
        assert_eq!(agents[1].group, group, "{case}");
        runtime.snapshot.navigator.agents = agents;
        runtime.snapshot.navigator.workspaces = workspaces;
        runtime.refresh_agent_scopes();
        let scope = &runtime.snapshot.navigator.workspaces[0].agent_scope;
        let listed = scope.rows.iter().any(|row| row.pane_id == "child");
        assert_eq!(
            listed,
            group == "needs_you",
            "{case}: listed in the project"
        );
        assert_eq!(
            scope.groups.needs_you,
            usize::from(group == "needs_you"),
            "{case}: counted in Needs You"
        );
        assert_eq!(
            scope.tasks.cards.iter().any(|card| card.root == "child"),
            group == "needs_you",
            "{case}: the 태스크 list holds it only while it calls the operator"
        );
        assert!(
            runtime.snapshot.navigator.workspaces[0].checkouts[1]
                .agent_scope
                .sidebar_tree
                .rows
                .is_empty(),
            "{case}: no checkout's tree draws it, the raised Needs You and Sessions do"
        );
        assert!(
            runtime.snapshot.navigator.agents[1].hide_ai.is_some(),
            "{case}: the row stays in the agent data for search and links"
        );
        assert!(
            runtime.snapshot.navigator.devices[0]
                .agent_scope
                .listed
                .iter()
                .any(|entry| entry.pane_id == "child"),
            "{case}: the pool search, links and tabs find agents in still has it"
        );
    }
}
