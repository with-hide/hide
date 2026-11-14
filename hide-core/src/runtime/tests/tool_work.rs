//! Whose work a row is decides which of the operator's lists it reaches
//! (docs/status-model.md, Ownership). The operator's own rows and Hide AI's
//! rows reach the same lists as before external tools existed (PRD
//! hide-external-control B37); an external tool's work reaches them only to
//! ask a person or show a View (B26-B29).

use super::*;
use crate::model::{DeclaredHumanTurn, HideAiSnapshot, ToolWorkSnapshot, UnseenViewFact};

/// Who started the row's agent.
#[derive(Clone, Copy, Debug)]
enum Owner {
    Operator,
    HideAi,
    /// An external tool, whose connection stands at Act (`true`) or no
    /// longer does: View only, disconnected or left.
    Tool(bool),
}

/// What the row's agent is doing.
#[derive(Clone, Copy, Debug)]
enum Doing {
    Working,
    Resting,
    /// Its own question, unread.
    Question,
    /// Finished a turn the operator has not read.
    Done,
    /// `hide agent ask-human auth github.com/login`.
    HumanTurn,
    /// Opened a View for the operator with `hide file open`, unseen.
    Look,
}

/// Where one row shows, surface by surface, keyed by the inventory's cells.
#[derive(Debug, Default, Eq, PartialEq)]
struct Reach {
    /// L3: its checkout's tree in the sidebar.
    tree: bool,
    /// L5, L6, L16: the raised group it stands in (Needs You or Done), which
    /// the app badge sums.
    raised: Option<&'static str>,
    /// L7: the project's Sessions rows.
    sessions: bool,
    /// L8: a card of the device's 태스크 list.
    task: bool,
    /// L9: the project's Overview count and its Needs You count.
    overview: (usize, usize),
    /// L10: the Project tool's session count.
    project_sessions: usize,
    /// L11: the checkout's summary count, which the sidebar badge reads.
    summary: usize,
    /// L13-L15: its group on the phone, whose notices the phone push and the
    /// desktop notices both follow.
    phone: Option<String>,
    /// L17: the pool ⌘K, links and tabs find an agent in.
    pool: bool,
    /// L18: whether the usage clock counts it as active.
    active: bool,
}

fn row(owner: Owner, doing: Doing) -> SidebarAgentSnapshot {
    let mut agent = crate::sidebar::project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id": "subject", "agent": "claude", "agent_status": "idle", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0);
    match doing {
        Doing::Working => agent.activity = "working".into(),
        Doing::Resting => agent.activity = "stopped".into(),
        Doing::Question => {
            agent.activity = "stopped".into();
            agent.demand = "question".into();
            agent.unread = true;
        }
        Doing::Done => {
            agent.activity = "stopped".into();
            agent.completed = true;
            agent.unread = true;
        }
        Doing::Look => {
            agent.activity = "stopped".into();
            agent.unseen_views = vec![UnseenViewFact {
                device_id: crate::node::TEST_NODE.into(),
                path: "/fixture/work".into(),
                view_id: "v1".into(),
                label: "report.html".into(),
                workspace_id: Some("project".into()),
                checkout_id: Some("work".into()),
                opened_unix_ms: 1,
                ancestors_read: false,
            }];
        }
        Doing::HumanTurn => {
            agent.activity = "working".into();
            agent.declared_human_turn = Some(DeclaredHumanTurn {
                id: "h1".into(),
                kind: crate::coordination::human_turn::HumanKind::Auth,
                place: "github.com/login".into(),
                device: None,
                since_unix_ms: 1,
            });
        }
    }
    match owner {
        Owner::Operator => {}
        Owner::HideAi => {
            agent.hide_ai = Some(HideAiSnapshot {
                task: None,
                home: Some(crate::coordination::TaskHome::Suggestions),
                workspace_id: None,
                turn: None,
            });
        }
        Owner::Tool(owns) => agent.tool = Some(tool(owns)),
    }
    agent
}

fn tool(owns: bool) -> ToolWorkSnapshot {
    ToolWorkSnapshot {
        connection: "c0ffee00c0ffee00".into(),
        name: "nightly".into(),
        owns,
    }
}

fn reach(agent: SidebarAgentSnapshot) -> Reach {
    reach_among(vec![agent])
}

/// Where the row `subject` shows among `agents`.
fn reach_among(mut agents: Vec<SidebarAgentSnapshot>) -> Reach {
    let mut runtime = runtime();
    let workspaces = vec![workspace(
        "project",
        "Project",
        "/fixture",
        vec![checkout(
            "project",
            "work",
            "/fixture/work",
            Some(pane("subject", "/fixture/work")),
        )],
    )];
    crate::agent_state::apply_lineage(&mut agents, &workspaces, &[]);
    runtime.snapshot.navigator.agents = agents;
    runtime.snapshot.navigator.workspaces = workspaces;
    // A child's escalation rises to its root here, as on every projection.
    runtime.sync_session_state();
    crate::agent_state::sync_checkout_agent_summaries(
        &mut runtime.snapshot.navigator.workspaces,
        &runtime.snapshot.navigator.agents,
    );
    runtime.refresh_agent_scopes();
    let project = &runtime.snapshot.navigator.workspaces[0];
    let scope = &project.agent_scope;
    let device = &runtime.snapshot.navigator.devices[0].agent_scope;
    let summary = &project.checkouts[0].agent_summary;
    let rest = serde_json::json!({"navigator": {
        "agents": &runtime.snapshot.navigator.agents,
        "workspaces": &runtime.snapshot.navigator.workspaces,
    }});
    let phone = crate::agent_state::phone::project(&rest, crate::node::TEST_NODE);
    Reach {
        tree: !project.checkouts[0]
            .agent_scope
            .sidebar_tree
            .rows
            .is_empty(),
        raised: device
            .raised
            .iter()
            .find(|raised| raised.shown.iter().any(|row| row.pane_id == "subject"))
            .map(|raised| match raised.group.as_str() {
                "needs_you" => "needs_you",
                "done" => "done",
                other => panic!("raised group {other}"),
            }),
        sessions: scope.rows.iter().any(|row| row.pane_id == "subject"),
        task: device.tasks.cards.iter().any(|card| card.root == "subject"),
        overview: (scope.overview_total, scope.overview_needs_you),
        project_sessions: scope.buckets.turn
            + scope.buckets.working
            + scope.buckets.delegating
            + scope.buckets.resting,
        summary: summary.needs_you + summary.done + summary.working + summary.seen,
        phone: phone
            .agents()
            .find(|listed| listed.pane_id == "subject")
            .map(|listed| listed.group.clone()),
        pool: device.listed.iter().any(|entry| entry.pane_id == "subject"),
        active: crate::agent_state::any_agent_active(&runtime.snapshot.navigator.agents, true),
    }
}

/// Everywhere: an operator's row in `group`, standing in `raised`.
fn everywhere(group: &str, raised: Option<&'static str>) -> Reach {
    Reach {
        tree: true,
        raised,
        sessions: true,
        task: true,
        overview: (1, usize::from(raised == Some("needs_you"))),
        project_sessions: 1,
        summary: 1,
        phone: Some(group.into()),
        pool: true,
        active: matches!(group, "needs_you" | "working"),
    }
}

/// Only where search, links and tabs look.
fn pool_only() -> Reach {
    Reach {
        pool: true,
        ..Reach::default()
    }
}

/// A row its owner keeps that calls the operator: in Needs You and every
/// list that counts it, never in a checkout's tree (PRD hide-ai-work-home
/// D-17). The usage clock reads the row's own axes, which a person's turn
/// is not one of, for every owner alike (L18), so `active` is the case's.
fn called_out(active: bool) -> Reach {
    Reach {
        tree: false,
        active,
        ..everywhere("needs_you", Some("needs_you"))
    }
}

/// B37 (inventory L3-L18): the operator's own row reaches every list in the
/// group its axes give; Hide AI's reaches them only while it is in Needs
/// You, and no checkout's tree draws it even then. A person's turn a Hide AI
/// agent declares calls the operator out like its own question does, so
/// every list that counts Needs You counts it (inventory L2, the note on
/// raised rows).
#[test]
fn each_row_reaches_the_operators_lists_by_whose_work_it_is() {
    let cases = [
        (Owner::Operator, Doing::Working, everywhere("working", None)),
        (Owner::Operator, Doing::Resting, everywhere("seen", None)),
        (
            Owner::Operator,
            Doing::Question,
            everywhere("needs_you", Some("needs_you")),
        ),
        (Owner::Operator, Doing::Done, everywhere("done", Some("done"))),
        (
            Owner::Operator,
            Doing::HumanTurn,
            everywhere("needs_you", Some("needs_you")),
        ),
        (Owner::HideAi, Doing::Working, pool_only()),
        (Owner::HideAi, Doing::Resting, pool_only()),
        (Owner::HideAi, Doing::Question, called_out(true)),
        (Owner::HideAi, Doing::Done, pool_only()),
        (Owner::HideAi, Doing::HumanTurn, called_out(false)),
        (Owner::Operator, Doing::Look, everywhere("needs_you", Some("needs_you"))),
        (Owner::HideAi, Doing::Look, pool_only()),
    ];
    for (owner, doing, expected) in cases {
        assert_eq!(
            reach(row(owner, doing)),
            expected,
            "{owner:?} {doing:?}"
        );
    }
}

/// B26, B28, B29 (D-09, D-37, D-38): a tool's agent reaches the operator's
/// lists only to ask a person or to show a View; its own question, its done
/// turn and its work are the tool's to read. Once the tool no longer keeps
/// it (View only, disconnected, left) it is the operator's row in every way
/// (D-14).
#[test]
fn a_tools_agent_reaches_the_operator_only_for_a_person_or_a_view() {
    let cases = [
        (Doing::Working, pool_only()),
        (Doing::Resting, pool_only()),
        (Doing::Question, pool_only()),
        (Doing::Done, pool_only()),
        (Doing::HumanTurn, called_out(false)),
        (Doing::Look, called_out(true)),
    ];
    for (doing, expected) in cases {
        assert_eq!(reach(row(Owner::Tool(true), doing)), expected, "kept {doing:?}");
        assert_eq!(
            reach(row(Owner::Tool(false), doing)),
            reach(row(Owner::Operator, doing)),
            "let go {doing:?}"
        );
    }
    let mut asking = row(Owner::Tool(true), Doing::Question);
    crate::agent_state::rederive(&mut asking);
    assert_eq!(
        asking.mark,
        crate::agent_state::RowMark::Waiting,
        "its question waits on the tool, not on the operator"
    );
    assert!(asking.unlisted, "the phone reads the core's one decision");
}

/// B27, B29: a child a tool's agent started with `--parent here` stays off
/// the operator's lists, whatever it asks; only its person's turn rises, to
/// the tool's agent, which then stands in Needs You.
#[test]
fn a_tools_delegated_child_rises_to_the_operator_only_for_a_person() {
    let child = |doing: Doing| {
        let mut child = row(Owner::Tool(true), doing);
        child.pane_id = "child".into();
        child.lineage_session = Some("child-session".into());
        child.spawned_from_pane_id = Some("subject".into());
        child.declared_parent_session = Some("tool-session".into());
        child
    };
    let child_reach = |doing: Doing| {
        let mut parent = row(Owner::Tool(true), Doing::Working);
        parent.lineage_session = Some("tool-session".into());
        let mut agents = vec![parent, child(doing)];
        let workspaces = vec![workspace(
            "project",
            "Project",
            "/fixture",
            vec![checkout(
                "project",
                "work",
                "/fixture/work",
                Some(pane("subject", "/fixture/work")),
            )],
        )];
        crate::agent_state::apply_lineage(&mut agents, &workspaces, &[]);
        assert!(agents[1].delegated);
        assert!(agents[1].unlisted, "{doing:?}: the child is never listed");
        reach_among(agents)
    };
    for doing in [Doing::Working, Doing::Question, Doing::Done, Doing::Look] {
        assert_eq!(child_reach(doing), pool_only(), "{doing:?}");
    }
    assert_eq!(child_reach(Doing::HumanTurn), called_out(false));
}
