use super::agent_sleep::{delivery_agent, delivery_panes};
use super::tree_close::{A, B, leave, started, tree_catalog, tree_runtime};
use super::*;

// Reopen closed after a tree close: the parent w1:p1 closed with its
// children p2 and p4 and its grandchild p3 is one entry, and one reopen
// brings all four back, each registration continued in its new pane.
// p5, p4's child in an unregistered project, is not listed and stays open.

const CLOSED: [&str; 4] = ["w1:p1", "w1:p2", "w1:p3", "w1:p4"];

fn session_of(pane: &str) -> String {
    format!("s-{}", pane.replace(':', "-"))
}

/// The pane a closed pane comes back in.
fn reopened(pane: &str) -> String {
    pane.replace(":p", ":p1")
}

fn registration_of(pane: &str) -> &'static str {
    match pane {
        "w1:p1" => "agent-1",
        "w1:p2" => "agent-2",
        "w1:p3" => "agent-3",
        "w1:p4" => "agent-4",
        _ => "agent-5",
    }
}

fn observe(runtime: &mut Runtime, panes: &[(&str, &str)]) {
    let node = runtime.node.as_str().to_owned();
    let payload = delivery_panes(
        panes
            .iter()
            .map(|(pane, session)| delivery_agent(registration_of(pane), pane, "claude", session))
            .collect(),
    );
    runtime.observe_delivery(&node, &payload, Some("scope"), None);
}

/// Every agent registered as `hide agent spawn --parent` leaves it, with the
/// root watching p2 and a letter from p2 to p3 still pending.
fn registered_tree() -> Runtime {
    use crate::coordination::AgentRecord;
    let mut runtime = tree_runtime(&[]);
    let node = runtime.node.as_str().to_owned();
    let every = ["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w5:p5"]
        .map(|pane| (pane, session_of(pane)))
        .to_vec();
    let every = every
        .iter()
        .map(|(pane, session)| (*pane, session.as_str()))
        .collect::<Vec<_>>();
    observe(&mut runtime, &every);
    let mut ledger = crate::delivery::ledger::Ledger::default();
    for (pane, parent) in [
        ("w1:p1", None),
        ("w1:p2", Some("agent-1")),
        ("w1:p3", Some("agent-2")),
        ("w1:p4", Some("agent-1")),
        ("w5:p5", Some("agent-4")),
    ] {
        ledger.agents.push(AgentRecord {
            id: registration_of(pane).into(),
            name: Some(registration_of(pane).into()),
            machine: node.clone(),
            host_scope: "scope".into(),
            native_machine: "fixture-machine".into(),
            session: session_of(pane),
            instance: pane.into(),
            pane: pane.into(),
            parent: parent.map(Into::into),
            origin: None,
            project: None,
            actor: runtime.delivery_observations[pane].actor.clone(),
            ended: false,
            terminal_id: None,
            forked: false,
        });
    }
    ledger.next_id = 6;
    let actor = |index: usize| ledger.agents[index].actor.clone();
    let (root, child, grandchild) = (actor(0), actor(1), actor(2));
    let at = crate::delivery::worker::now();
    crate::delivery::watch::start(&mut ledger, &root, &child, at).unwrap();
    crate::delivery::mailbox::send(
        &mut ledger,
        &child,
        &grandchild,
        "ask",
        "review the parser",
        "request",
        None,
        at,
    )
    .unwrap();
    ledger.validate().unwrap();
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    observe(&mut runtime, &every);
    runtime
}

fn close_tree(runtime: &mut Runtime) {
    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "close_tree",
            "payload": {
                "target": {"kind": "pane", "pane_id": "w1:p1"},
                "pane_ids": ["w1:p2", "w1:p3", "w1:p4"],
                "confirmed": true
            }
        }))
        .unwrap(),
    );
}

/// Herdr answers the capture of every close that started, as a pane close
/// records it for Reopen closed.
fn capture_started(runtime: &mut Runtime) {
    let requests = runtime
        .close_operations
        .values()
        .filter(|operation| operation.phase == "preparing")
        .map(|operation| operation.request.clone())
        .collect::<Vec<_>>();
    for request in requests {
        let live::CloseCaptureTarget::Pane { pane_id } = &request.target else {
            panic!("a tree node here is a pane");
        };
        let pane = request
            .panes
            .iter()
            .find(|pane| pane.pane_id == *pane_id)
            .unwrap()
            .clone();
        let item = ClosedItem::Pane {
            key: request.key.clone(),
            context: request.context.clone(),
            pane: crate::recent_closed::ClosedPane {
                agent: Some(crate::recent_closed::ClosedAgent {
                    kind: "claude".into(),
                    session_id: Some(session_of(pane_id)),
                    ..Default::default()
                }),
                ..pane
            },
            placement: crate::recent_closed::PanePlacement {
                neighbor_pane_id: None,
                parent_path: Vec::new(),
                direction: crate::recent_closed::ClosedSplitDirection::Right,
                ratio: 0.5,
                target_was_first: false,
            },
        };
        runtime.ingest_close_capture_result(
            &request,
            Ok(live::CloseCaptureOutcome { item: Some(item) }),
        );
    }
}

/// Runs the tree close to its end the way Herdr answers it: each started
/// close is captured, then its pane leaves the session and the delivery
/// read.
fn closed_tree() -> Runtime {
    let mut runtime = registered_tree();
    close_tree(&mut runtime);
    for present in [
        &["w1:p1", "w1:p2", "w1:p4", "w5:p5"][..],
        &["w1:p1", "w5:p5"],
        &["w5:p5"],
    ] {
        capture_started(&mut runtime);
        leave(&mut runtime, present);
        let still = present
            .iter()
            .map(|pane| (*pane, session_of(pane)))
            .collect::<Vec<_>>();
        let still = still
            .iter()
            .map(|(pane, session)| (*pane, session.as_str()))
            .collect::<Vec<_>>();
        observe(&mut runtime, &still);
    }
    assert!(runtime.tree_closes.is_empty(), "the tree close finished");
    assert!(started(&runtime).is_empty(), "every close settled");
    runtime
}

fn the_entry(runtime: &Runtime) -> Vec<crate::recent_closed::TreeMember> {
    match runtime.recent_closed.iter().collect::<Vec<_>>().as_slice() {
        [ClosedItem::Tree { members, .. }] => members.clone(),
        other => panic!("one tree entry, got {other:?}"),
    }
}

fn member_pane(member: &crate::recent_closed::TreeMember) -> &str {
    let ClosedItem::Pane { pane, .. } = &member.item else {
        panic!("a pane member");
    };
    &pane.pane_id
}

/// Herdr answers each member's reopen in turn, `fail` refusing that pane's;
/// returns the members started, in order.
fn reopen_answering(runtime: &mut Runtime, fail: &[&str]) -> Vec<String> {
    let members = the_entry(runtime);
    assert!(runtime.reopen_closed());
    let mut order = Vec::new();
    while let Some(key) = runtime.reopen_in_flight.clone() {
        let member = members
            .iter()
            .find(|member| member.item.key() == key)
            .expect("the reopen in flight is a member");
        let old = member_pane(member).to_owned();
        order.push(old.clone());
        let request = live::ReopenRequest {
            codex_daemon: Default::default(),
            item: member.item.clone(),
            workspace_exists: true,
            tab_exists: false,
            fallback_pane_id: None,
            owner: None,
        };
        let result = if fail.contains(&old.as_str()) {
            Err("layout.apply timed out".to_owned())
        } else {
            let pane = reopened(&old);
            Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
                tab_id: Some(pane.replace(":p", ":t")),
                consumed: true,
                focused_pane_id: Some(pane.clone()),
                notices: Vec::new(),
                agents: vec![live::ReopenedAgent {
                    pane_id: pane,
                    kind: "claude".into(),
                    name: registration_of(&old).into(),
                    launch: Default::default(),
                    session_id: Some(session_of(&old)),
                }],
            }))
        };
        assert!(runtime.ingest_reopen_result(&request, result));
    }
    order
}

/// The session after the reopen: p5 still open, and each reopened pane in
/// its own tab, with the parent its registration names (what the lineage
/// worker writes on the pane: `parent_pane` is the parent registration's
/// pane, `coordination::lineage`).
fn reopened_session(runtime: &mut Runtime, back: &[&str]) {
    let ledger = runtime.delivery_state().unwrap();
    let pane_of = |id: &str| {
        ledger
            .agents
            .iter()
            .find(|record| record.id == id)
            .map(|record| record.pane.clone())
    };
    let mut panes = vec![serde_json::json!({"pane_id": "w5:p5", "cwd": B})];
    let mut tabs = vec![serde_json::json!({"workspace_id": "w5", "tab_id": "w5:t5", "label": ""})];
    let mut agents = vec![serde_json::json!({
        "id": "agent-w5:p5", "pane_id": "w5:p5", "agent": "claude", "agent_status": "idle",
        "spawned_from_pane_id": pane_of("agent-4"), "state_change_seq": 1,
    })];
    let mut layouts = vec![serde_json::json!({
        "workspace_id": "w5", "tab_id": "w5:t5", "zoomed": false,
        "area": {"x": 0, "y": 0, "width": 80, "height": 24}, "focused_pane_id": "w5:p5",
        "panes": [{"pane_id": "w5:p5", "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
        "splits": []
    })];
    for (index, old) in back.iter().enumerate() {
        let pane = reopened(old);
        let tab = pane.replace(":p", ":t");
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == registration_of(old))
            .unwrap();
        panes.push(serde_json::json!({"pane_id": pane, "cwd": A}));
        tabs.push(serde_json::json!({"workspace_id": "w1", "tab_id": tab, "label": ""}));
        agents.push(serde_json::json!({
            "id": format!("agent-{pane}"), "pane_id": pane, "agent": "claude",
            "agent_status": "idle", "state_change_seq": index + 2,
            "spawned_from_pane_id": record.parent.as_deref().and_then(pane_of),
            "agent_session": {"kind": "id", "value": session_of(old)},
        }));
        layouts.push(serde_json::json!({
            "workspace_id": "w1", "tab_id": tab, "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24}, "focused_pane_id": pane,
            "panes": [{"pane_id": pane, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": []
        }));
    }
    let payload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents,
        "focused_workspace_id": "w1",
        "workspaces": [
            {"workspace_id": "w1", "label": "a"},
            {"workspace_id": "w5", "label": "b", "active_tab_id": "w5:t5"}
        ],
        "tabs": tabs, "panes": panes, "layouts": layouts
    }))
    .unwrap();
    runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog()));
}

/// The delivery store applies the moves the core asked for, as its worker
/// does on each pass (`coordination::rebind`).
fn apply_rebinds(runtime: &mut Runtime) -> Vec<String> {
    let rebinds = runtime.delivery_registration_rebinds();
    let mut ledger = (*runtime.delivery_state().unwrap()).clone();
    let moved = crate::coordination::rebind(&mut ledger, &rebinds);
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let mut ids = moved.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
    ids.sort();
    ids
}

fn parent_pane(runtime: &Runtime, pane: &str) -> Option<String> {
    runtime
        .snapshot()
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .unwrap_or_else(|| panic!("{pane} is listed"))
        .lineage_parent_pane_id
        .clone()
}

#[test]
fn one_reopen_brings_a_closed_tree_back_with_its_registrations_and_parents() {
    let mut runtime = closed_tree();

    // While closed, nothing the tree held ends: the registrations, the
    // root's watch on p2 and the letter to p3 wait for the reopen.
    let gone = runtime.delivery_registrations_gone();
    assert!(
        CLOSED
            .iter()
            .all(|pane| !gone.contains_key(registration_of(pane))),
        "{gone:?}"
    );
    assert!(runtime.delivery_watch_work().iter().all(|work| !work.gone));

    // One entry for the whole tree, the target first and every parent
    // before the members under it.
    let members = the_entry(&runtime);
    assert_eq!(runtime.snapshot().recent_closed.count, 1);
    assert_eq!(member_pane(&members[0]), "w1:p1");
    assert_eq!(members[0].parent, None);
    let mut panes = members.iter().map(member_pane).collect::<Vec<_>>();
    panes.sort();
    assert_eq!(panes, CLOSED);
    for member in &members {
        let [held] = member.registrations.as_slice() else {
            panic!("one registration per pane");
        };
        assert_eq!(held.id, registration_of(member_pane(member)));
        if let Some(parent) = &member.parent {
            let index = members
                .iter()
                .position(|other| other.item.key() == parent)
                .unwrap();
            assert!(index < members.iter().position(|m| m == member).unwrap());
        }
    }

    let order = reopen_answering(&mut runtime, &[]);
    assert_eq!(order[0], "w1:p1", "the target comes back first");
    assert_eq!(order.len(), 4, "one reopen restores every member");
    assert!(runtime.recent_closed.is_empty(), "the entry is consumed");
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w1:p11"),
        "the keyboard stays with the reopened target"
    );

    // Each registration moves to the pane its conversation resumed in.
    reopened_session(&mut runtime, &CLOSED);
    let resumed = CLOSED
        .iter()
        .map(|pane| (reopened(pane), session_of(pane)))
        .chain([("w5:p5".to_owned(), session_of("w5:p5"))])
        .collect::<Vec<_>>();
    let resumed_view = resumed
        .iter()
        .map(|(pane, session)| (pane.as_str(), session.as_str()))
        .collect::<Vec<_>>();
    // Herdr lists each reopened agent under the name it was started with.
    let node = runtime.node.as_str().to_owned();
    runtime.observe_delivery(
        &node,
        &delivery_panes(
            resumed_view
                .iter()
                .zip(["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w5:p5"])
                .map(|((pane, session), old)| {
                    delivery_agent(registration_of(old), pane, "claude", session)
                })
                .collect(),
        ),
        Some("scope"),
        None,
    );
    runtime.advance_tree_reopens();
    let rebinds = runtime.delivery_registration_rebinds();
    for pane in CLOSED {
        assert_eq!(
            rebinds[registration_of(pane)].to.pane_id,
            reopened(pane),
            "{pane}'s registration moves to its new pane"
        );
    }
    assert_eq!(
        apply_rebinds(&mut runtime),
        ["agent-1", "agent-2", "agent-3", "agent-4"]
    );

    let ledger = runtime.delivery_state().unwrap();
    for pane in CLOSED {
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == registration_of(pane))
            .unwrap();
        assert!(!record.ended);
        assert_eq!(record.pane, reopened(pane));
    }
    assert_eq!(ledger.watches.len(), 1);
    assert_eq!(ledger.watches[0].parent.pane_id, "w1:p11");
    assert_eq!(ledger.watches[0].target.pane_id, "w1:p12");
    assert_eq!(ledger.letters[0].recipient.pane_id, "w1:p13");
    assert_eq!(
        ledger.letters[0].state,
        crate::delivery::ledger::State::Pending
    );

    // The lineage the registrations now name draws the tree again, the
    // child left open included.
    reopened_session(&mut runtime, &CLOSED);
    assert_eq!(parent_pane(&runtime, "w1:p11"), None);
    assert_eq!(parent_pane(&runtime, "w1:p12").as_deref(), Some("w1:p11"));
    assert_eq!(parent_pane(&runtime, "w1:p13").as_deref(), Some("w1:p12"));
    assert_eq!(parent_pane(&runtime, "w1:p14").as_deref(), Some("w1:p11"));
    assert_eq!(parent_pane(&runtime, "w5:p5").as_deref(), Some("w1:p14"));

    // Once the ledger shows the move, nothing is held any more.
    runtime.observe_delivery(
        &node,
        &delivery_panes(
            resumed_view
                .iter()
                .zip(["w1:p1", "w1:p2", "w1:p3", "w1:p4", "w5:p5"])
                .map(|((pane, session), old)| {
                    delivery_agent(registration_of(old), pane, "claude", session)
                })
                .collect(),
        ),
        Some("scope"),
        None,
    );
    runtime.advance_tree_reopens();
    assert!(runtime.tree_reopen_continuations.is_empty());
}

/// A member that does not come back keeps the members under it, unstarted,
/// so no child returns without its parent; the next reopen retries them.
#[test]
fn a_member_that_fails_to_reopen_keeps_its_children_for_the_next_reopen() {
    let mut runtime = closed_tree();
    let order = reopen_answering(&mut runtime, &["w1:p2"]);
    assert!(!order.contains(&"w1:p3".to_owned()), "p3 waits for p2");
    assert!(
        order.contains(&"w1:p4".to_owned()),
        "another branch goes on"
    );
    let mut left = the_entry(&runtime)
        .iter()
        .map(|member| member_pane(member).to_owned())
        .collect::<Vec<_>>();
    left.sort();
    assert_eq!(left, ["w1:p2", "w1:p3"]);
    assert!(
        runtime.snapshot().recent_closed.notices[0]
            .message
            .starts_with("2 of 4 closed agents could not be reopened")
    );
    // What stays in the entry stays held.
    assert!(
        !runtime
            .delivery_registrations_gone()
            .contains_key("agent-3")
    );

    let order = reopen_answering(&mut runtime, &[]);
    assert_eq!(order, ["w1:p2", "w1:p3"]);
    assert!(runtime.recent_closed.is_empty());
}

/// An entry pushed off the list unreopened lets its registrations go: they
/// end with the panes that closed, as any closed agent's do.
#[test]
fn a_tree_entry_pushed_off_the_list_lets_its_registrations_end() {
    let mut runtime = closed_tree();
    for index in 0..crate::recent_closed::RECENT_CLOSED_LIMIT {
        runtime.push_recent_closed(ClosedItem::File {
            key: format!("file-{index}"),
            device_id: runtime.node.as_str().to_owned(),
            workspace_id: "w1".into(),
            checkout_id: "checkout".into(),
            checkout_path: A.into(),
            path: format!("{A}/file-{index}"),
            label: format!("file-{index}"),
        });
    }
    assert!(
        !runtime
            .recent_closed
            .iter()
            .any(|item| matches!(item, ClosedItem::Tree { .. }))
    );
    let gone = runtime.delivery_registrations_gone();
    for pane in CLOSED {
        assert!(gone.contains_key(registration_of(pane)), "{pane}");
    }
    assert!(!gone.contains_key("agent-5"), "the child left open stays");
}

/// A reopen pressed while the tree is still closing waits for its entry
/// rather than reaching an older item.
#[test]
fn a_reopen_during_a_tree_close_waits_for_its_entry() {
    let mut runtime = registered_tree();
    close_tree(&mut runtime);
    capture_started(&mut runtime);
    leave(&mut runtime, &["w1:p1", "w1:p2", "w1:p4", "w5:p5"]);
    assert!(!runtime.snapshot().recent_closed.can_reopen);
    assert!(runtime.reopen_closed());
    assert!(runtime.reopen_in_flight.is_none());
    assert!(runtime.recent_closed.is_empty());
}
