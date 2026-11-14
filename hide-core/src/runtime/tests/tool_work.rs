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
fn reach_among(agents: Vec<SidebarAgentSnapshot>) -> Reach {
    let runtime = settled(agents);
    let project = &runtime.snapshot.navigator.workspaces[0];
    let scope = &project.agent_scope;
    let device = &runtime.snapshot.navigator.devices[0].agent_scope;
    let summary = &project.checkouts[0].agent_summary;
    let phone = phone_of(&runtime);
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

/// The phone's projection of a settled runtime, as `hided` sends it.
fn phone_of(runtime: &Runtime) -> crate::agent_state::phone::Projection {
    let rest = serde_json::json!({"navigator": {
        "agents": &runtime.snapshot.navigator.agents,
        "workspaces": &runtime.snapshot.navigator.workspaces,
    }});
    crate::agent_state::phone::project(&rest, crate::node::TEST_NODE)
}

/// A runtime holding `agents` in one project, settled as a projection settles it.
fn settled(mut agents: Vec<SidebarAgentSnapshot>) -> Runtime {
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
    runtime
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
        (
            Owner::Operator,
            Doing::Done,
            everywhere("done", Some("done")),
        ),
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
        (
            Owner::Operator,
            Doing::Look,
            everywhere("needs_you", Some("needs_you")),
        ),
        (Owner::HideAi, Doing::Look, pool_only()),
    ];
    for (owner, doing, expected) in cases {
        assert_eq!(reach(row(owner, doing)), expected, "{owner:?} {doing:?}");
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
        assert_eq!(
            reach(row(Owner::Tool(true), doing)),
            expected,
            "kept {doing:?}"
        );
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

/// B46, D-52: the phone has no tool's cell, so a tool's row that reaches its
/// list, a person's turn or a View, names the tool, on the wire and so on its
/// second line; the operator's own rows send no name, which an older phone
/// therefore never meets.
#[test]
fn a_tools_row_on_the_phone_names_the_tool() {
    let sent = |owner: Owner, doing: Doing| {
        let phone = phone_of(&settled(vec![row(owner, doing)]));
        let listed = phone
            .agents()
            .find(|listed| listed.pane_id == "subject")
            .unwrap_or_else(|| panic!("{owner:?} {doing:?} is on the phone"));
        serde_json::to_value(listed).unwrap().get("tool").cloned()
    };
    for doing in [Doing::HumanTurn, Doing::Look] {
        assert_eq!(
            sent(Owner::Tool(true), doing),
            Some(serde_json::json!("nightly")),
            "kept {doing:?}"
        );
        assert_eq!(
            sent(Owner::Operator, doing),
            None,
            "the operator's {doing:?}"
        );
    }
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
    // A child's View goes through the runtime's own open in
    // `unseen_views::a_tools_delegated_childs_view_follows_every_delegated_childs_rule`.
    for doing in [Doing::Working, Doing::Question, Doing::Done] {
        assert_eq!(child_reach(doing), pool_only(), "{doing:?}");
    }
    assert_eq!(child_reach(Doing::HumanTurn), called_out(false));
}

/// The tool `nightly` asks to connect under `id` with a key of `digit`s,
/// and the operator approves it at `level`; its connection's id. A revoked
/// key is refused, so a new connection comes with a new key.
pub(super) fn connect(
    runtime: &mut Runtime,
    id: &str,
    digit: &str,
    level: crate::access::Level,
) -> String {
    connect_as(runtime, "nightly", id, digit, level)
}

/// `connect` under another tool's name.
fn connect_as(
    runtime: &mut Runtime,
    name: &str,
    id: &str,
    digit: &str,
    level: crate::access::Level,
) -> String {
    let (asked, _) = runtime.access_call(crate::access::Call::Request {
        name: name.into(),
        key_sha256: crate::access::key_hash(&digit.repeat(64)),
        program: None,
        id: id.into(),
    });
    let connection = asked.unwrap()["id"].as_str().unwrap().to_owned();
    assert!(
        runtime.access_event(crate::runtime::access::AccessEvent::Approve {
            id: connection.clone(),
            level,
        })
    );
    connection
}

/// B16, D-14, D-40: whose the work is follows the tool's connection on the
/// event that moved it. View only or a disconnect hands the tool's agent
/// to the operator at once, Act again takes it back, and a new connection
/// under the same name is another connection, which takes nothing.
#[test]
fn the_tools_work_is_the_operators_the_moment_its_connection_no_longer_acts() {
    use crate::access::Level;
    use crate::runtime::access::AccessEvent;
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
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
    let mut agent = row(Owner::Operator, Doing::Working);
    agent.lineage_session = Some("tool-session".into());
    runtime.snapshot.navigator.agents = vec![agent];
    let connection = connect(&mut runtime, "00000000000000a7", "7", Level::Act);
    runtime.hide_started.tool_work = vec![crate::coordination::started::ToolWork {
        connection: connection.clone(),
        name: "nightly".into(),
        agent_id: "agent-1".into(),
        pane_id: "subject".into(),
        session: Some("tool-session".into()),
        ended: false,
    }];
    runtime.refresh_agent_lineage();
    runtime.sync_session_state();
    runtime.refresh_agent_scopes();
    let listed = |runtime: &Runtime| {
        let subject = &runtime.snapshot.navigator.agents[0];
        let sessions = runtime.snapshot.navigator.workspaces[0]
            .agent_scope
            .rows
            .iter()
            .any(|row| row.pane_id == "subject");
        assert_eq!(sessions, !subject.unlisted, "one decision for every list");
        (subject.tool.as_ref().map(|tool| tool.owns), sessions)
    };
    assert_eq!(listed(&runtime), (Some(true), false), "kept at Act");

    assert!(runtime.access_event(AccessEvent::Level {
        id: connection.clone(),
        level: Level::View,
    }));
    assert_eq!(
        listed(&runtime),
        (Some(false), true),
        "View only lets it go"
    );
    assert!(runtime.access_event(AccessEvent::Level {
        id: connection.clone(),
        level: Level::Act,
    }));
    assert_eq!(listed(&runtime), (Some(true), false), "Act takes it back");

    assert!(runtime.access_event(AccessEvent::Revoke {
        id: connection.clone(),
    }));
    assert_eq!(
        listed(&runtime),
        (Some(false), true),
        "a disconnect lets it go"
    );
    let again = connect(&mut runtime, "00000000000000a8", "8", Level::Act);
    assert_ne!(again, connection);
    runtime.refresh_agent_lineage();
    runtime.sync_session_state();
    runtime.refresh_agent_scopes();
    assert_eq!(
        listed(&runtime),
        (Some(false), true),
        "a new connection of the same name takes nothing"
    );
}

/// B28, B29: a tool's agent that asks its tool something, such as the
/// decision of a mission it conducts, and also opened a View or waits on a
/// person shows the operator only the View or the person's turn: its mark,
/// its line and `외 N건` leave the tool's question out. The operator's own
/// row keeps counting its question behind its person's turn.
#[test]
fn a_tools_own_question_never_leads_or_counts_on_the_operators_row() {
    let asking = |owner: Owner, doing: Doing| {
        let mut agent = row(owner, doing);
        agent.demand = "question".into();
        agent.unread = true;
        agent.detail = Some("Which order?".into());
        crate::agent_state::rederive(&mut agent);
        agent
    };
    let looking = asking(Owner::Tool(true), Doing::Look);
    assert_eq!(looking.group, "needs_you");
    assert_eq!(looking.mark, crate::agent_state::RowMark::Look);
    assert_eq!(
        looking.state.ask, None,
        "the row says the View, not the question"
    );
    let person = asking(Owner::Tool(true), Doing::HumanTurn);
    assert_eq!(person.mark, crate::agent_state::RowMark::Turn);
    let ask = person.state.ask.unwrap();
    assert_eq!(
        (ask.what.as_deref(), ask.more),
        (Some("github.com/login"), 0),
        "no 외 N건 for the tool's question"
    );
    let own = asking(Owner::Operator, Doing::HumanTurn);
    assert_eq!(own.state.ask.unwrap().more, 1);

    // A question the operator read, on the tool's pane, is still the tool's:
    // it does not lift the row into the 태스크 list's 내 차례 as the
    // operator's own read question does.
    let read = |owner: Owner| {
        let mut agent = row(owner, Doing::Resting);
        agent.demand = "question".into();
        agent.changed_at_unix_ms = Some(1);
        agent
    };
    assert_eq!(reach(read(Owner::Tool(true))), pool_only());
    assert!(reach(read(Owner::Operator)).task);
}

/// D-45, D-46, D-47, B40, B41, B43, B44: each tool connected at Act has its
/// own cell beside the devices, empty or not, holding only its work; its
/// agent's question is the tool's turn, the cell's grey number, and never
/// the operator's Needs You; an answer typed into the pane takes it out of
/// the tool's turn, and View only or a disconnect takes the cell away.
#[test]
fn a_tools_cell_holds_its_work_and_counts_only_the_tools_turn() {
    use crate::access::Level;
    use crate::runtime::access::AccessEvent;
    let mut runtime = runtime();
    runtime.snapshot.navigator.workspaces = vec![workspace(
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
    let mut agent = row(Owner::Operator, Doing::Working);
    agent.lineage_session = Some("tool-session".into());
    runtime.snapshot.navigator.agents = vec![agent];
    let connection = connect(&mut runtime, "00000000000000a7", "7", Level::Act);
    let idle = connect_as(&mut runtime, "weekly", "00000000000000a8", "8", Level::Act);
    runtime.hide_started.tool_work = vec![crate::coordination::started::ToolWork {
        connection: connection.clone(),
        name: "nightly".into(),
        agent_id: "agent-1".into(),
        pane_id: "subject".into(),
        session: Some("tool-session".into()),
        ended: false,
    }];
    let project = |runtime: &mut Runtime| {
        runtime.refresh_agent_lineage();
        runtime.sync_session_state();
        runtime.publish_mission_asks();
        runtime.refresh_agent_scopes();
        let cells = runtime
            .snapshot
            .navigator
            .tools
            .iter()
            .map(|cell| {
                let cards = cell
                    .devices
                    .iter()
                    .flat_map(|device| &device.tasks.cards)
                    .map(|card| (card.root.clone(), card.state))
                    .collect::<Vec<_>>();
                (cell.connection.clone(), cell.turn, cards)
            })
            .collect::<Vec<_>>();
        let operator = &runtime.snapshot.navigator.devices[0].agent_scope;
        // The operator's cell: its 태스크 cards and its Needs You number.
        (
            cells,
            (operator.tasks.cards.len(), operator.groups.needs_you),
        )
    };
    use crate::agent_state::tally::tasks::TaskState;
    assert_eq!(
        project(&mut runtime),
        (
            vec![
                (
                    connection.clone(),
                    0,
                    vec![("subject".to_owned(), TaskState::Working)]
                ),
                (idle.clone(), 0, Vec::new()),
            ],
            (0, 0)
        ),
        "every Act tool has a cell, the one with no work too, and the operator's lists none of it"
    );

    let subject = &mut runtime.snapshot.navigator.agents[0];
    subject.activity = "stopped".into();
    subject.demand = "question".into();
    subject.unread = true;
    assert_eq!(
        project(&mut runtime),
        (
            vec![
                (
                    connection.clone(),
                    1,
                    vec![("subject".to_owned(), TaskState::NeedsYou)]
                ),
                (idle.clone(), 0, Vec::new()),
            ],
            (0, 0)
        ),
        "its question is the tool's turn, never the operator's"
    );

    // The operator answers in the agent's pane from the tool's cell.
    let subject = &mut runtime.snapshot.navigator.agents[0];
    subject.activity = "working".into();
    subject.demand = String::new();
    subject.unread = false;
    assert_eq!(
        project(&mut runtime).0[0],
        (
            connection.clone(),
            0,
            vec![("subject".to_owned(), TaskState::Working)]
        ),
        "the answer takes it out of the tool's turn and leaves it the tool's"
    );

    assert!(runtime.access_event(AccessEvent::Level {
        id: connection.clone(),
        level: Level::View,
    }));
    assert!(runtime.access_event(AccessEvent::Revoke { id: idle.clone() }));
    assert_eq!(
        project(&mut runtime),
        (Vec::new(), (1, 0)),
        "View only and a disconnect end the cells, and the work let go is the operator's"
    );
}
