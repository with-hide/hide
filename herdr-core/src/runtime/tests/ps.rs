//! `hide ps`: the live panes of this machine and of each device, the agent
//! each holds with its lineage parent, registration and watch, and the
//! registrations no pane holds.

use super::*;
use crate::delivery::answer::{Ps, PsAgent, PsState};
use crate::delivery::{Actor, ledger::Ledger};

fn actor(pane: &str, device: &str, session: &str) -> Actor {
    Actor {
        pane_id: pane.into(),
        name: pane.into(),
        kind: "claude".into(),
        device_id: device.into(),
        session: crate::wire::session_digest(session),
    }
}

fn record(
    id: &str,
    actor: &Actor,
    pane: &str,
    parent: Option<&str>,
    ended: bool,
) -> crate::coordination::AgentRecord {
    crate::coordination::AgentRecord {
        id: id.into(),
        name: Some(actor.name.clone()),
        machine: actor.device_id.clone(),
        host_scope: "fixture-scope".into(),
        native_machine: "fixture-machine".into(),
        session: format!("{}-native", actor.name),
        instance: pane.into(),
        pane: pane.into(),
        parent: parent.map(str::to_owned),
        origin: None,
        project: Some("/fixture/main".into()),
        actor: actor.clone(),
        ended,
        terminal_id: None,
        forked: false,
    }
}

/// A lead and a shell in the main checkout's tab, the lead's delegated child
/// in a worktree, a lead on the device `mini` that does not answer, an
/// ended registration and a live one whose pane is gone.
fn fixture() -> Runtime {
    let mut runtime = runtime();
    let mut main = checkout(
        "project",
        "main",
        "/fixture/main",
        Some(pane("lead", "/fixture/main")),
    );
    main.branch = Some("main".into());
    main.tabs[0].panes.push(pane("shell", "/fixture/main"));
    let mut feature = checkout(
        "project",
        "feature",
        "/fixture/feature",
        Some(pane("child", "/fixture/feature")),
    );
    feature.branch = Some("feat/x".into());
    feature.is_worktree = true;
    runtime.snapshot.navigator.workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture/main",
        vec![main, feature],
    )];
    let rows = |agents: serde_json::Value| {
        project_agents(
            crate::sidebar::owned_label_fixture(serde_json::json!({ "agents": agents })).unwrap(),
        )
        .agents
    };
    runtime.snapshot.navigator.agents = rows(serde_json::json!([
        {"id":"lead-control","pane_id":"lead","agent":"claude","agent_status":"working","state_change_seq":1,"tokens":{"task":"Lead"}},
        {"id":"child-control","pane_id":"child","agent":"codex","spawned_from_pane_id":"lead","agent_status":"working","state_change_seq":2,"tokens":{"task":"Child"}},
    ]));
    runtime.refresh_agent_lineage();
    for agent in &mut runtime.snapshot.navigator.agents {
        agent.lineage_session = crate::wire::session_digest(&format!("{}-native", agent.pane_id));
    }
    let mut far = checkout(
        "mini-project",
        "mini-main",
        "/far/main",
        Some(pane("remote:mini:pane:w1:p1", "/far/main")),
    );
    far.branch = Some("main".into());
    let mut far_rows = rows(serde_json::json!([
        {"id":"far-control","pane_id":"remote:mini:pane:w1:p1","agent":"claude","agent_status":"working","state_change_seq":1,"tokens":{"task":"Far"}},
    ]));
    far_rows[0].lineage_session = crate::wire::session_digest("far-native");
    let mut far = workspace("mini-project", "Far", "/far/main", vec![far]);
    far.device_id = "mini".into();
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: "mini".to_owned(),
        state: "unavailable".to_owned(),
        message: None,
        herdr_version: Some("0.9.5".to_owned()),
        session: Some(RemoteSessionSnapshot {
            workspaces: vec![far],
            agents: far_rows,
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
    let node = crate::node::TEST_NODE;
    let lead = actor("lead", node, "lead-native");
    let child = actor("child", node, "child-native");
    let mut ledger = Ledger {
        agents: vec![
            // An older registration of the lead's pane, from a session it no
            // longer runs.
            record(
                "agent-0",
                &actor("lead", node, "before"),
                "lead",
                None,
                false,
            ),
            record("agent-1", &lead, "lead", None, false),
            record("agent-2", &child, "child", Some("agent-1"), false),
            record(
                "agent-3",
                &actor("gone", node, "gone-native"),
                "gone",
                None,
                true,
            ),
            record(
                "agent-4",
                &actor("far", "mini", "far-native"),
                "w1:p1",
                None,
                false,
            ),
        ],
        ..Ledger::default()
    };
    crate::delivery::watch::start(&mut ledger, &lead, &child, 1).unwrap();
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    runtime
}

fn answer(runtime: &Runtime, all: bool) -> Ps {
    runtime
        .ps_rows(crate::node::TEST_NODE, "shell")
        .unwrap()
        .answer(all)
}

fn agent<'a>(ps: &'a Ps, pane: &str) -> &'a PsAgent {
    ps.panes
        .iter()
        .find(|row| row.pane == pane)
        .and_then(|row| row.agent.as_ref())
        .unwrap()
}

#[test]
fn every_live_pane_is_listed_with_its_place_agent_parent_and_watch() {
    let runtime = fixture();
    let ps = answer(&runtime, false);
    let places: Vec<_> = ps
        .panes
        .iter()
        .map(|pane| {
            (
                pane.machine.as_str(),
                pane.checkout.as_str(),
                pane.pane.as_str(),
                pane.you,
            )
        })
        .collect();
    assert_eq!(
        places,
        [
            (crate::node::TEST_NODE, "main", "lead", false),
            (crate::node::TEST_NODE, "main", "shell", true),
            (crate::node::TEST_NODE, "feat/x", "child", false),
            ("mini", "main", "w1:p1", false),
        ]
    );
    let shell = ps.panes.iter().find(|pane| pane.pane == "shell").unwrap();
    assert!(shell.agent.is_none());
    let lead = agent(&ps, "lead");
    assert_eq!(
        lead.id.as_deref(),
        Some("agent-1"),
        "the registration of the session the pane runs"
    );
    assert_eq!(lead.parent_pane, None);
    let child = agent(&ps, "child");
    assert_eq!(
        (child.id.as_deref(), child.parent.as_deref()),
        (Some("agent-2"), Some("agent-1"))
    );
    assert_eq!(
        (
            child.parent_machine.as_deref(),
            child.parent_pane.as_deref()
        ),
        (Some(crate::node::TEST_NODE), Some("lead"))
    );
    assert_eq!(child.watched_by.as_deref(), Some("lead"));
    assert_eq!(child.kind, "codex");
    let far = agent(&ps, "w1:p1");
    assert_eq!(
        (far.id.as_deref(), far.state),
        (Some("agent-4"), PsState::Disconnected)
    );
    assert_eq!(ps.panes[3].machine_label.as_deref(), Some("mini"));
    assert_eq!(ps.panes[0].machine_label, None);
}

#[test]
fn registrations_no_pane_holds_are_listed_and_ended_ones_only_with_all() {
    let runtime = fixture();
    let ids = |ps: &Ps| {
        ps.agents
            .iter()
            .map(|agent| (agent.id.clone().unwrap(), agent.state))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&answer(&runtime, false)),
        [("agent-0".to_owned(), PsState::Unknown)]
    );
    assert_eq!(
        ids(&answer(&runtime, true)),
        [
            ("agent-0".to_owned(), PsState::Unknown),
            ("agent-3".to_owned(), PsState::Ended)
        ]
    );
}

fn mission(
    id: &str,
    conductor: &str,
    present: bool,
    worker: &str,
) -> crate::runtime::missions::MissionView {
    use crate::missions::{MissionScope, MissionStatus, NodeKind, NodeStatus};
    use crate::runtime::missions::{ConductorView, MissionView, NodeView};
    MissionView {
        id: id.into(),
        title: format!("{id} title"),
        workspace_id: "project".into(),
        project: "/fixture/main".into(),
        checkout_path: "/fixture/main".into(),
        device_id: crate::node::TEST_NODE.into(),
        status: MissionStatus::Running,
        scope: MissionScope::Pr,
        cap: 2,
        exempt: false,
        conductor: ConductorView {
            pane_id: conductor.into(),
            name: conductor.into(),
            agent_kind: "claude".into(),
            session: None,
            present,
            takes_answers: true,
        },
        nodes: vec![NodeView {
            key: "fix".into(),
            title: "Fix it".into(),
            kind: NodeKind::Change,
            depends_on: Vec::new(),
            status: NodeStatus::Working,
            blocked: None,
            delivery: None,
            worker: None,
            agent_id: Some(worker.into()),
            pane_id: None,
            machine: None,
            pr: None,
            base_pr: None,
            branch: None,
            pr_state: None,
            pr_url: None,
            stage: None,
            attempts: 0,
            line: None,
            change: None,
            review: None,
            criteria: Vec::new(),
            criteria_met: 0,
            criteria_skipped: 0,
            external: None,
            reopened: 0,
            started_at_unix_ms: None,
            finished_at_unix_ms: None,
            commit: None,
            verdict: None,
            checks: None,
            plan: None,
        }],
        merge_order: Vec::new(),
        check: None,
        github: None,
        decision: None,
        question: None,
        answer: None,
        answer_failed: None,
        delay: None,
        report: None,
        created_at_unix_ms: 0,
        started_at_unix_ms: None,
        closed_at_unix_ms: None,
        running: 1,
        stale: false,
        github_stale: false,
        github_read_at_unix_ms: None,
    }
}

#[test]
fn a_mission_names_its_present_conductor_and_its_workers_and_an_unread_record_says_so() {
    let mut runtime = fixture();
    runtime.snapshot.missions.items = vec![mission("m1", "lead", true, "agent-2")];
    let ps = answer(&runtime, false);
    let role = |pane: &str| {
        agent(&ps, pane)
            .mission
            .as_ref()
            .map(|m| (m.id.clone(), m.role.clone()))
    };
    assert_eq!(
        role("lead"),
        Some(("m1".to_owned(), "conductor".to_owned()))
    );
    assert_eq!(role("child"), Some(("m1".to_owned(), "fix".to_owned())));
    assert!(!ps.missions_unreadable);
    // Another session in the Conductor's former pane is not the Conductor.
    runtime.snapshot.missions.items = vec![mission("m1", "lead", false, "agent-2")];
    let ps = answer(&runtime, false);
    assert_eq!(
        agent(&ps, "lead").mission.as_ref().map(|m| m.id.as_str()),
        None
    );
    runtime.snapshot.missions.failed = Some("unreadable".into());
    assert!(answer(&runtime, false).missions_unreadable);
}

#[test]
fn the_watcher_is_named_as_its_own_row_names_it() {
    let mut runtime = fixture();
    let mut ledger = (**runtime.delivery_ledger.as_ref().unwrap()).clone();
    // A registration with no name of its own goes by its registration id.
    ledger
        .agents
        .iter_mut()
        .find(|record| record.id == "agent-1")
        .unwrap()
        .name = None;
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let ps = answer(&runtime, false);
    assert_eq!(agent(&ps, "lead").name.as_deref(), Some("agent-1"));
    assert_eq!(agent(&ps, "child").watched_by.as_deref(), Some("agent-1"));
}
