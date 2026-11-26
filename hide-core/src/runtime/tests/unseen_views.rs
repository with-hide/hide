//! An agent's open the operator has not looked at (PRD unseen-agent-views):
//! the row it puts in Needs You, what looking at it takes, and what a
//! delegated child's open does to its ancestors. Each test drives the
//! runtime's own opens, events and reads, and reads the rows it publishes.

use super::*;
use crate::agent_state::RowMark;
use crate::view_layout::DisplayKind;
use crate::workspace_control::Action;
use serde_json::json;

pub(super) const CHECKOUT: &str = "/checkouts/fixture";
/// The fixture's one Agent tab, which holds both panes.
const TAB: &str = "tab";

pub(super) fn key() -> (String, String) {
    (crate::node::TEST_NODE.into(), CHECKOUT.into())
}

fn agent(pane: &str, session: &str) -> crate::model::SidebarAgentSnapshot {
    let mut agent = crate::sidebar::project_agents(
        crate::sidebar::owned_label_fixture(json!({"agents": [
            {"id": pane, "pane_id": pane, "agent_status": "working", "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0);
    agent.agent_kind = "codex".into();
    agent.lineage_session = Some(session.into());
    agent.session_id = Some(format!("native-{pane}"));
    agent.identity_label = format!("Agent {pane}");
    agent
}

/// The delivery fixture's two panes, `sender` and `recipient`, each running
/// a working Codex agent in its own terminal, in the one front checkout,
/// with the Files lane open, a window holding the keyboard and the operator
/// in neither pane. With `child`, `recipient` is `sender`'s delegated child.
pub(super) fn fixture(child: bool) -> (tempfile::TempDir, Arc<Mutex<Runtime>>) {
    let root = tempfile::tempdir().unwrap();
    let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
    {
        let mut guard = runtime.lock().unwrap();
        for pane in guard.snapshot.navigator.workspaces[0].checkouts[0].tabs[0]
            .panes
            .iter_mut()
        {
            pane.terminal_id = Some(format!("term-{}", pane.id));
        }
        let sender = agent("sender", "sender-session");
        let mut recipient = agent("recipient", "recipient-session");
        if child {
            recipient.declared_parent_pane_id = Some("sender".into());
            recipient.declared_parent_session = Some("sender-session".into());
        }
        guard.snapshot.navigator.agents = vec![sender, recipient];
        guard.snapshot.navigator.focused_workspace_id = Some("workspace".into());
        guard.snapshot.navigator.focused_checkout_id = Some("checkout".into());
        guard.snapshot.navigator.workspaces[0].checkouts[0].active_tab_id = Some(TAB.into());
        let store = guard.workspace_views.as_mut().unwrap();
        let entry = store.views.entry(crate::node::TEST_NODE, CHECKOUT);
        entry.open = true;
        entry.tray_entry(TAB).shown = true;
        guard.set_window_focused(true);
        guard.refresh_agent_lineage();
        guard.refresh_pane_read_state();
    }
    (root, runtime)
}

/// The operator's keyboard on `pane`, or on no pane, as the strip settles
/// it before the next frame.
pub(super) fn keyboard(runtime: &mut Runtime, pane: Option<&str>) {
    runtime.snapshot.terminal.pane_id = pane.map(str::to_owned);
    runtime.set_operator_focus(pane.map(str::to_owned));
    runtime.refresh_pane_read_state();
    runtime.sync_workspace_view();
}

/// `hide file open <path>` (or a page) from `caller`, as the daemon applies
/// it once the file is read: on the desk of the caller's tab when the
/// operator watches the caller, else on no desk (PRD lane-desk-pair-view
/// B1), a View that already shows the file taken where it is (D-20), and
/// marked by its caller.
pub(super) fn open(runtime: &mut Runtime, caller: &str, path: &str) -> String {
    let key = key();
    let place = runtime.control_place(&key, caller);
    let watched = place != crate::workspace_views::Place::Shelf;
    let view = match runtime.claim_existing_view(&key, &place, |display| {
        display.shows(path, DisplayKind::File, None)
    }) {
        // An open the operator does not watch leaves every front as it was.
        Some((id, _)) if !watched => id,
        Some((id, at)) => runtime
            .change_view_layout(&key, &at, |layout, stamp| {
                Ok((id.clone(), layout.focus(&id, stamp)?))
            })
            .unwrap(),
        None => runtime
            .change_view_layout(&key, &place, |layout, stamp| {
                let display = layout.new_display(path, DisplayKind::File, None, false);
                let id = display.id.clone();
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
                Ok((id, true))
            })
            .unwrap(),
    };
    let action = Action::OpenFile { path: path.into() };
    runtime.stamp_control_opener(&key, caller, &action, &view);
    runtime.sync_workspace_view();
    view
}

fn row<'a>(runtime: &'a Runtime, pane: &str) -> &'a crate::model::SidebarAgentSnapshot {
    runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane)
        .unwrap()
}

fn unseen(runtime: &Runtime, view: &str) -> bool {
    runtime
        .view_display(&key(), view)
        .is_some_and(|display| display.unseen.is_some())
}

/// `hide view status <view>` or a `hide browser` read from `caller`'s pane,
/// as the CLI reports it after the read (`Action::Read`).
fn read(runtime: &mut Runtime, caller: &str, view: &str) {
    let expected = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            caller,
            crate::workspace_control::Query::Info,
        )
        .unwrap()
        .context;
    let answer = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &format!("{}-read-{caller}-{view}", unix_milliseconds()),
            Action::Read {
                view_id: view.to_owned(),
            },
            Ok(None),
        )
        .unwrap();
    assert!(!answer.changed);
}

/// The recipient's prompt blocked on the operator, or answered.
fn blocked(runtime: &mut Runtime, blocked: bool) {
    let recipient = runtime
        .snapshot
        .navigator
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id == "recipient")
        .unwrap();
    recipient.blocked = blocked;
    runtime.refresh_agent_lineage();
}

/// The operator's press on the View: from its pane's list or a tab, it
/// comes onto the desk in front and forward there.
fn look_at(runtime: &mut Runtime, view: &str) {
    assert!(runtime.dispatch_json(&explorer_event(
        "view_layout",
        json!({
            "workspace": {"device_id": crate::node::TEST_NODE, "path": CHECKOUT},
            "action": "focus", "display_id": view,
        }),
    )));
    runtime.sync_workspace_view();
}

#[test]
fn an_agents_open_while_the_operator_is_elsewhere_puts_its_row_in_needs_you_to_look_at() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let view = open(&mut guard, "recipient", "src/login.ts");
    let recipient = row(&guard, "recipient");
    // B1, D-08: at once, while the agent still works.
    assert_eq!(recipient.activity, "working");
    assert_eq!(recipient.group, "needs_you");
    assert_eq!(recipient.mark, RowMark::Look);
    let look = recipient.look.as_ref().unwrap();
    assert_eq!((look.count, look.view_id.as_str()), (1, view.as_str()));
    assert_eq!(look.label, "login.ts");
    assert_eq!(look.group, "working", "the row without it");
    // B12: it is counted as the operator's turn.
    let mut counts = crate::model::MarkCountsSnapshot::default();
    recipient.mark.count_into(&mut counts);
    assert_eq!(counts.turn, 1);
    // B4, D-20: seeing it moves the row back without reading the pane; the
    // keyboard reaching the pane brings nothing onto the desk (PRD
    // lane-desk-pair-view B1), the operator's press does.
    keyboard(&mut guard, Some("recipient"));
    assert!(unseen(&guard, &view));
    look_at(&mut guard, &view);
    assert!(!unseen(&guard, &view));
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.group, "working");
    assert_eq!(recipient.mark, RowMark::Going);
    assert!(recipient.look.is_none());
}

#[test]
fn an_open_the_operator_is_already_looking_at_is_never_unseen() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("recipient"));
    open(&mut guard, "recipient", "src/a.ts");
    let view = open(&mut guard, "recipient", "src/login.ts");
    // B5, B14: decided in the same event as the open, so no frame shows it.
    assert!(!unseen(&guard, &view));
    assert_eq!(row(&guard, "recipient").group, "working");
}

#[test]
fn a_view_stays_unseen_while_no_window_holds_the_keyboard_or_the_lane_shows_something_else() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("recipient"));
    // B6: the window lost the keyboard; the View is in front all the same.
    guard.set_window_focused(false);
    let view = open(&mut guard, "recipient", "src/login.ts");
    assert!(unseen(&guard, &view));
    // The operator is at its pane, so the row is read and the View waits as
    // its chip, calling no one (operator decision 2026-11-23).
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.group, "working");
    assert_eq!(recipient.look.as_ref().unwrap().view_id, view);
    // B7: the lane shows no documents.
    {
        let store = guard.workspace_views.as_mut().unwrap();
        for tray in &mut store.views.entry(crate::node::TEST_NODE, CHECKOUT).trays {
            tray.shown = false;
        }
    }
    guard.set_window_focused(true);
    guard.sync_workspace_view();
    assert!(unseen(&guard, &view));
    // Back on the documents, with a window holding the keyboard, it is seen.
    {
        let store = guard.workspace_views.as_mut().unwrap();
        for tray in &mut store.views.entry(crate::node::TEST_NODE, CHECKOUT).trays {
            tray.shown = true;
        }
    }
    guard.sync_workspace_view();
    assert!(!unseen(&guard, &view));
}

#[test]
fn a_view_in_an_area_that_is_not_active_stays_unseen() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    guard.set_window_focused(false);
    keyboard(&mut guard, Some("recipient"));
    let first = open(&mut guard, "recipient", "src/a.ts");
    let second = open(&mut guard, "recipient", "src/b.ts");
    // The first View moves to an area of its own; the second stays active.
    guard
        .change_front_layout(&key(), |layout, stamp| {
            let area = layout.active_area().id.clone();
            layout
                .split(&first, &area, crate::view_layout::Edge::Right, stamp)
                .map(|_| ((), true))
        })
        .unwrap();
    guard
        .change_front_layout(&key(), |layout, stamp| {
            let area = layout.area_of(&second).unwrap().id.clone();
            layout.focus_area(&area, stamp).map(|changed| ((), changed))
        })
        .unwrap();
    guard.set_window_focused(true);
    keyboard(&mut guard, Some("recipient"));
    // B7: shown in an area that is not the active one is not looked at.
    assert!(unseen(&guard, &first));
    assert!(!unseen(&guard, &second));
}

#[test]
fn several_unseen_views_name_the_newest_and_count_down_as_each_is_seen() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let a = open(&mut guard, "recipient", "src/a.ts");
    let b = open(&mut guard, "recipient", "src/b.ts");
    let c = open(&mut guard, "recipient", "src/c.ts");
    let look = row(&guard, "recipient").look.clone().unwrap();
    // B9, D-14: the newest leads, and the rest are counted.
    assert_eq!((look.count, look.view_id.as_str()), (3, c.as_str()));
    // The keyboard reaches their pane while the window is away, so only
    // the View brought forward below is looked at.
    guard.set_window_focused(false);
    keyboard(&mut guard, Some("recipient"));
    look_at(&mut guard, &a);
    guard.set_window_focused(true);
    let look = row(&guard, "recipient").look.clone().unwrap();
    assert_eq!((look.count, look.view_id.as_str()), (2, c.as_str()));
    look_at(&mut guard, &c);
    let look = row(&guard, "recipient").look.clone().unwrap();
    assert_eq!((look.count, look.view_id.as_str()), (1, b.as_str()));
    look_at(&mut guard, &b);
    assert!(row(&guard, "recipient").look.is_none());
}

#[test]
fn a_view_closed_before_it_is_seen_takes_its_look_with_it() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let view = open(&mut guard, "recipient", "src/login.ts");
    assert_eq!(row(&guard, "recipient").group, "needs_you");
    // B13, B32: the record lives on the View and goes with it.
    let place = guard.view_place(&key(), &view).unwrap();
    guard
        .change_view_layout(&key(), &place, |layout, _| {
            layout.remove(&view);
            Ok(((), true))
        })
        .unwrap();
    guard.sync_workspace_view();
    assert!(row(&guard, "recipient").look.is_none());
    assert_eq!(row(&guard, "recipient").group, "working");
}

#[test]
fn an_open_again_of_a_seen_view_is_to_look_at_again() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("recipient"));
    let view = open(&mut guard, "recipient", "src/login.ts");
    assert!(!unseen(&guard, &view));
    open(&mut guard, "recipient", "src/a.ts");
    keyboard(&mut guard, Some("sender"));
    // B14, D-11: the same path opened again by the agent, now behind.
    assert_eq!(open(&mut guard, "recipient", "src/login.ts"), view);
    assert!(unseen(&guard, &view));
    assert_eq!(row(&guard, "recipient").mark, RowMark::Look);
}

#[test]
fn only_an_agents_open_makes_a_view_to_look_at() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    // B15: a checkout caller is nobody's, so the operator's.
    let checkout = crate::workspace_control::checkout_caller_id(&"a".repeat(32), CHECKOUT);
    let mine = open(&mut guard, &checkout, "src/mine.ts");
    assert!(!unseen(&guard, &mine));
    // A terminal link the operator clicked in the agent's pane names the
    // agent as opener, and is still the operator's open.
    guard.expect_view_opener(&key(), "src/link.ts", DisplayKind::File, Some("recipient"));
    let link = guard
        .change_front_layout(&key(), |layout, stamp| {
            let display = layout.new_display("src/link.ts", DisplayKind::File, None, false);
            let id = display.id.clone();
            let area = layout.active_area().id.clone();
            layout.insert(&area, display, stamp)?;
            Ok((id, true))
        })
        .unwrap();
    assert!(guard.claim_view_opener(&key(), "src/link.ts", DisplayKind::File, None));
    assert!(!unseen(&guard, &link));
    // A page's own new target, opened by an agent's `hide browser` action.
    let target = Action::OpenBrowser {
        url: "https://example.test/popup".into(),
        area_id: Some("a1".into()),
        new_target: true,
    };
    guard.stamp_control_opener(&key(), "recipient", &target, &link);
    assert!(!unseen(&guard, &link));
    assert!(row(&guard, "recipient").look.is_none());
}

#[test]
fn the_last_agent_to_open_a_view_is_the_one_whose_row_has_it() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    let view = open(&mut guard, "recipient", "src/login.ts");
    assert!(row(&guard, "recipient").look.is_some());
    // B16, D-19.
    assert_eq!(open(&mut guard, "sender", "src/login.ts"), view);
    assert!(row(&guard, "recipient").look.is_none());
    assert_eq!(row(&guard, "sender").look.as_ref().unwrap().view_id, view);
}

#[test]
fn a_question_takes_the_mark_and_the_view_keeps_the_row_in_needs_you_after_it() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    open(&mut guard, "recipient", "src/login.ts");
    // B11, D-18: a block outranks the View for the mark.
    blocked(&mut guard, true);
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.mark, RowMark::Turn);
    assert!(recipient.look.is_some());
    assert_eq!(recipient.look.as_ref().unwrap().group, "needs_you");
    blocked(&mut guard, false);
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.group, "needs_you");
    assert_eq!(recipient.mark, RowMark::Look);
}

#[test]
fn a_read_row_leaves_needs_you_with_its_view_kept_and_a_later_open_calls_again() {
    // Operator decision 2026-11-23: a View holds the row in Needs You only
    // while the row is unread; read, the row stands where its own axes put
    // it, the View still unseen as its chip, and the notice is taken back.
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let rows = |runtime: &Runtime| json!({"navigator": {"agents": serde_json::to_value(&runtime.snapshot.navigator.agents).unwrap()}});
    let mut transitions = crate::agent_state::push::Transitions::default();
    let mut judge = |runtime: &Runtime| {
        transitions.observe(&crate::agent_state::phone::project(
            &rows(runtime),
            crate::node::TEST_NODE,
        ))
    };
    judge(&guard);
    let view = open(&mut guard, "recipient", "src/login.ts");
    assert_eq!(row(&guard, "recipient").group, "needs_you");
    assert_eq!(judge(&guard).0.len(), 1);
    // The operator goes to the pane and away again, without the View.
    keyboard(&mut guard, Some("recipient"));
    keyboard(&mut guard, Some("sender"));
    assert!(unseen(&guard, &view));
    let recipient = row(&guard, "recipient");
    assert_eq!(
        (recipient.group.as_str(), recipient.mark),
        ("working", RowMark::Going)
    );
    assert_eq!(recipient.look.as_ref().unwrap().view_id, view);
    let mut counts = crate::model::MarkCountsSnapshot::default();
    recipient.mark.count_into(&mut counts);
    assert_eq!(counts.turn, 0);
    let (notices, cleared) = judge(&guard);
    assert!(notices.is_empty());
    assert_eq!(cleared.len(), 1);
    // Another open is news again, the first View counted under its chip.
    let report = open(&mut guard, "recipient", "src/report.md");
    let recipient = row(&guard, "recipient");
    assert_eq!(
        (recipient.group.as_str(), recipient.mark),
        ("needs_you", RowMark::Look)
    );
    let look = recipient.look.as_ref().unwrap();
    assert_eq!((look.view_id.as_str(), look.count), (report.as_str(), 2));
    assert_eq!(judge(&guard).0.len(), 1);
}

#[test]
fn a_view_to_look_at_is_one_notice_until_seen_and_a_block_keeps_its_own() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let rows = |runtime: &Runtime| json!({"navigator": {"agents": serde_json::to_value(&runtime.snapshot.navigator.agents).unwrap()}});
    let mut transitions = crate::agent_state::push::Transitions::default();
    let mut judge = |runtime: &Runtime| {
        transitions.observe(&crate::agent_state::phone::project(
            &rows(runtime),
            crate::node::TEST_NODE,
        ))
    };
    judge(&guard);
    // Operator decision 2026-11-08: the open notifies as `볼 것` with the
    // View's name, once.
    let view = open(&mut guard, "recipient", "src/login.ts");
    let (notices, _) = judge(&guard);
    assert_eq!(
        notices
            .iter()
            .map(|n| (n.status_code.as_deref(), n.detail.as_deref()))
            .collect::<Vec<_>>(),
        [(Some("look"), Some("login.ts"))]
    );
    let second = open(&mut guard, "recipient", "src/report.md");
    assert!(judge(&guard).0.is_empty(), "a second open while unseen");
    // A block on the same row calls the operator with its own notice.
    blocked(&mut guard, true);
    let (notices, _) = judge(&guard);
    assert_eq!(notices.len(), 1);
    assert_ne!(notices[0].status_code.as_deref(), Some("look"));
    blocked(&mut guard, false);
    assert_eq!(judge(&guard), (Vec::new(), Default::default()));
    // Seeing both Views takes the notice back.
    keyboard(&mut guard, Some("recipient"));
    look_at(&mut guard, &view);
    look_at(&mut guard, &second);
    assert!(!unseen(&guard, &view) && !unseen(&guard, &second));
    let (notices, cleared) = judge(&guard);
    assert!(notices.is_empty());
    assert_eq!(cleared.len(), 1);
}

#[test]
fn a_childs_unseen_view_turns_its_ancestors_unread_until_one_reads_it_or_the_operator_looks() {
    let (_root, runtime) = fixture(true);
    let mut guard = runtime.lock().unwrap();
    // The operator read the parent before the child opened anything.
    keyboard(&mut guard, Some("sender"));
    keyboard(&mut guard, None);
    assert!(!row(&guard, "sender").unread);
    // (1) The child opens: it stays where it was and its parent is to read.
    let view = open(&mut guard, "recipient", "src/report.md");
    let child = row(&guard, "recipient");
    assert!(child.delegated);
    assert_eq!(child.group, "working", "B20, D-10");
    assert!(child.look.is_none());
    assert!(row(&guard, "sender").unread);
    assert_ne!(row(&guard, "sender").group, "needs_you");
    // The child's own read and a caller that is no ancestor change nothing
    // (B23).
    read(&mut guard, "recipient", &view);
    guard.note_ancestor_read(&key(), &view, "stranger");
    assert!(row(&guard, "sender").unread);
    // (3) Its parent reads it: the signal goes, the View stays unseen (B22).
    read(&mut guard, "sender", &view);
    assert!(!row(&guard, "sender").unread);
    assert!(unseen(&guard, &view));
    // (5) The parent opens it again from its own pane: it is the parent's
    // to show now (B24).
    open(&mut guard, "sender", "src/report.md");
    let parent = row(&guard, "sender");
    assert_eq!(parent.group, "needs_you");
    assert_eq!(parent.look.as_ref().unwrap().view_id, view);
    // (2) The operator looks at it: seen, and nothing is left on either.
    keyboard(&mut guard, Some("sender"));
    look_at(&mut guard, &view);
    assert!(!unseen(&guard, &view));
    assert!(row(&guard, "sender").look.is_none());
}

/// PRD hide-external-control B29 and operator decision 2026-11-14: a
/// delegated child of an external tool's agent opening a View follows the
/// rule every delegated child does. The child stays where it was, its root
/// turns unread and stands in no Needs You, so the tool's work does not call
/// the operator; once the tool's agent opens it again itself, its row draws
/// the eye in the tool's cell, as on any root, and still calls no one.
#[test]
fn a_tools_delegated_childs_view_follows_every_delegated_childs_rule() {
    let (_root, runtime) = fixture(true);
    let mut guard = runtime.lock().unwrap();
    let connection = super::tool_work::connect(
        &mut guard,
        "00000000000000b8",
        "8",
        crate::access::Mode::Own,
    );
    // The tool started `sender`, and `recipient` under it, as the started
    // record keeps every agent delegated under the tool's.
    guard.hide_started.tool_work = [("agent-1", "sender"), ("agent-2", "recipient")]
        .map(|(agent, pane)| crate::coordination::started::ToolWork {
            connection: connection.clone(),
            name: "nightly".into(),
            agent_id: agent.into(),
            pane_id: pane.into(),
            session: Some(format!("{pane}-session")),
            ended: false,
        })
        .into();
    guard.refresh_agent_lineage();
    keyboard(&mut guard, Some("sender"));
    keyboard(&mut guard, None);
    let view = open(&mut guard, "recipient", "src/report.md");
    let child = row(&guard, "recipient");
    assert!(child.delegated && child.unlisted);
    assert!(
        child.look.is_none(),
        "the child's View is a descendant signal"
    );
    let root = row(&guard, "sender");
    assert!(root.tool.as_ref().is_some_and(|tool| tool.owns));
    assert!(root.unread, "its root turns unread, as every root does");
    assert_ne!(root.group, "needs_you");
    assert!(root.unlisted, "and the tool's root still calls no one");
    open(&mut guard, "sender", "src/report.md");
    let root = row(&guard, "sender");
    assert_eq!(root.look.as_ref().unwrap().view_id, view);
    assert_ne!(root.group, "needs_you", "the eye calls no one");
    assert!(root.unlisted, "and the operator's lists still leave it out");
}

/// PRD hide-external-control D-38, D-55, B29, D-52: a View an agent the tool
/// keeps opened shows as any agent's does, inside the tool's cell: the row
/// carries the eye and the View is in that session's Files list. It reaches
/// nothing of the operator's, the phone's Views list included, and it neither
/// moves the row into the tool's turn nor adds to the cell's number.
#[test]
fn a_view_a_tools_agent_opened_is_the_eye_on_its_row_in_the_tools_cell_only() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    let connection = super::tool_work::connect(
        &mut guard,
        "00000000000000b9",
        "9",
        crate::access::Mode::Own,
    );
    guard.hide_started.tool_work = vec![crate::coordination::started::ToolWork {
        connection: connection.clone(),
        name: "nightly".into(),
        agent_id: "agent-1".into(),
        pane_id: "recipient".into(),
        session: Some("recipient-session".into()),
        ended: false,
    }];
    guard.refresh_agent_lineage();
    keyboard(&mut guard, Some("sender"));
    keyboard(&mut guard, None);
    let view = open(&mut guard, "recipient", "src/report.md");
    guard.sync_session_state();
    guard.publish_mission_asks();
    guard.refresh_agent_scopes();

    // The row draws the eye, and the View is in its session's list.
    let agent = row(&guard, "recipient");
    assert_eq!(agent.mark, RowMark::Look);
    assert_eq!(agent.look.as_ref().unwrap().view_id, view);
    assert!(
        guard
            .view_display(&key(), &view)
            .is_some_and(|display| !display.openers.is_empty() && display.unseen.is_some())
    );
    // The tool's cell holds the row where its state puts it, and the eye
    // does not make it the tool's turn.
    let cell = &guard.snapshot.navigator.tools[0];
    assert_eq!(cell.turn, 0);
    let cards: Vec<_> = cell
        .devices
        .iter()
        .flat_map(|device| &device.tasks.cards)
        .collect();
    assert_eq!(cards.len(), 1);
    assert_ne!(
        cards[0].state,
        crate::agent_state::tally::tasks::TaskState::NeedsYou
    );
    // None of the operator's.
    assert_ne!(agent.group, "needs_you");
    assert!(agent.unlisted);
    assert_eq!(
        guard.snapshot.navigator.devices[0]
            .agent_scope
            .groups
            .needs_you,
        0
    );
    let phone = guard.phone_looks("recipient");
    assert!(phone.views.is_empty() && phone.omitted == 0);
}

/// PRD hide-external-control D-55 (operator decision, session based): a View
/// the tool itself opens with `hide file open`, from its connection and with
/// no Hide session behind it, is the operator's own open as on any caller
/// that is no agent: a tab for every strip, no mark and no stamp.
#[test]
fn a_tools_direct_open_is_the_operators_own_open() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    let connection = super::tool_work::connect(
        &mut guard,
        "00000000000000ba",
        "a",
        crate::access::Mode::Own,
    );
    keyboard(&mut guard, Some("sender"));
    let caller = crate::workspace_control::access_caller_id(&connection, None);
    let view = open(&mut guard, &caller, "src/hello.html");
    let display = guard.view_display(&key(), &view).unwrap();
    assert!(display.operator && display.openers.is_empty() && display.opener.is_none());
    assert!(
        display.unseen.is_none(),
        "no mark: nobody opened it for them"
    );
    assert!(
        guard
            .snapshot
            .navigator
            .agents
            .iter()
            .all(|agent| agent.look.is_none())
    );
}

#[test]
fn reading_the_parent_takes_in_the_childs_view_signal_and_a_close_drops_it() {
    let (_root, runtime) = fixture(true);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    let view = open(&mut guard, "recipient", "src/report.md");
    assert!(row(&guard, "sender").unread);
    // (4) The operator reads the parent pane: the signal is consumed by the
    // descendant rule, and the View is still unseen in the child's list (B25).
    keyboard(&mut guard, Some("sender"));
    keyboard(&mut guard, None);
    assert!(!row(&guard, "sender").unread);
    assert!(unseen(&guard, &view));
    // (6) Closed, it is gone everywhere.
    let place = guard.view_place(&key(), &view).unwrap();
    guard
        .change_view_layout(&key(), &place, |layout, _| {
            layout.remove(&view);
            Ok(((), true))
        })
        .unwrap();
    guard.sync_workspace_view();
    assert!(row(&guard, "recipient").unseen_views.is_empty());
    assert!(!row(&guard, "sender").unread);
}

#[test]
fn a_report_names_the_views_its_sender_opened_and_whether_they_were_seen() {
    let (_root, runtime) = fixture(true);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    let view = open(&mut guard, "recipient", "src/report.md");
    let mut body = "Done.".to_owned();
    guard.list_opened_views("recipient", &mut body);
    assert!(
        body.starts_with("Done.\n\nViews this agent opened"),
        "{body}"
    );
    assert!(
        body.contains(&format!(
            "- {view} src/report.md (not seen by the operator)"
        )),
        "{body}"
    );
    // An agent that opened nothing adds nothing.
    let mut quiet = "Done.".to_owned();
    guard.list_opened_views("sender", &mut quiet);
    assert_eq!(quiet, "Done.");
}

#[test]
fn a_row_pressed_in_a_list_brings_its_newest_unseen_view_forward_and_sees_it() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let view = open(&mut guard, "recipient", "src/login.ts");
    {
        let store = guard.workspace_views.as_mut().unwrap();
        for tray in &mut store.views.entry(crate::node::TEST_NODE, CHECKOUT).trays {
            tray.shown = false;
        }
    }
    // B3: the View comes forward on its pane, and is seen once in front.
    keyboard(&mut guard, Some("recipient"));
    guard.reveal_unseen_view("recipient");
    guard.sync_workspace_view();
    assert!(!unseen(&guard, &view));
    assert!(row(&guard, "recipient").look.is_none());
}

#[test]
fn a_restart_keeps_an_unseen_view_unseen_and_a_seen_one_seen() {
    let (root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let unread = open(&mut guard, "recipient", "src/a.ts");
    let read = open(&mut guard, "recipient", "src/b.ts");
    guard.set_window_focused(false);
    keyboard(&mut guard, Some("recipient"));
    look_at(&mut guard, &read);
    guard.set_window_focused(true);
    assert!(unseen(&guard, &unread) && !unseen(&guard, &read));
    keyboard(&mut guard, Some("sender"));

    // B17: the views file the saver writes, read again as a restart does.
    let path = root.path().join("views.json");
    let views = guard.workspace_views.as_ref().unwrap().views.clone();
    crate::workspace_views::save(&path, &views).unwrap();
    let panel = (
        guard.snapshot.ui_state.right_panel_visible,
        guard.snapshot.ui_state.right_panel_section,
    );
    guard.workspace_views = Some(WorkspaceViewStore::open(path, panel).0);
    guard.sync_workspace_view();

    assert!(unseen(&guard, &unread) && !unseen(&guard, &read));
    // The operator read the row with `unread` open, so it keeps its chip
    // where its own axes put it (operator decision 2026-11-23).
    let recipient = row(&guard, "recipient");
    assert_eq!(
        (recipient.group.as_str(), recipient.mark),
        ("working", RowMark::Going)
    );
    assert_eq!(recipient.look.as_ref().unwrap().view_id, unread);
}

/// `hide browser open <url>` from `caller`, through the action the daemon
/// commits.
fn open_page(runtime: &mut Runtime, caller: &str, url: &str) -> String {
    let expected = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            caller,
            crate::workspace_control::Query::Info,
        )
        .unwrap()
        .context;
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &format!("{}-page-{caller}-{url}", unix_milliseconds()),
            Action::OpenBrowser {
                url: url.to_owned(),
                area_id: None,
                new_target: false,
            },
            Ok(None),
        )
        .unwrap()
        .view_id
}

#[test]
fn an_open_again_from_behind_is_unseen_even_when_it_passed_through_the_front() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let page = open_page(&mut guard, "recipient", "https://example.test/a");
    let other = open_page(&mut guard, "recipient", "https://example.test/b");
    // The operator brings both onto the desk and sees them, the second in
    // front.
    look_at(&mut guard, &page);
    look_at(&mut guard, &other);
    assert!(!unseen(&guard, &page) && !unseen(&guard, &other));
    // B14: the agent behind opens the first page again; the operator's front
    // is put back, so the page is news, though it was in front mid-action.
    assert_eq!(
        open_page(&mut guard, "recipient", "https://example.test/a"),
        page
    );
    guard.sync_workspace_view();
    assert!(unseen(&guard, &page));
    assert_eq!(
        row(&guard, "recipient").look.as_ref().unwrap().view_id,
        page
    );
    // B5: an open from the pane the operator is in is seen in the same event.
    keyboard(&mut guard, Some("recipient"));
    let shown = open_page(&mut guard, "recipient", "https://example.test/c");
    assert!(!unseen(&guard, &shown));
}

#[test]
fn reads_keep_no_receipt_so_any_number_of_them_leaves_room_for_an_open() {
    let (_root, runtime) = fixture(true);
    let mut guard = runtime.lock().unwrap();
    // The operator read the parent before the child opened anything.
    keyboard(&mut guard, Some("sender"));
    keyboard(&mut guard, None);
    let view = open_page(&mut guard, "recipient", "https://example.test/report");
    assert!(row(&guard, "sender").unread);
    // More reads than the kept results hold, each still taken in (D-22).
    for _ in 0..200 {
        read(&mut guard, "sender", &view);
    }
    assert!(!row(&guard, "sender").unread);
    open_page(&mut guard, "recipient", "https://example.test/next");
}

/// An agent's `hide file open` or `hide browser open` of a file outside
/// every checkout reaches the core as the window's grant alone
/// (`open_external`, made by hided), and is tracked as any agent's open: it
/// waits in Needs You under its file name, `hide view list` names who opened
/// it and nothing of the file, a second open brings the same tab forward,
/// and looking at it ends the wait.
/// `hide browser open <outside file>` from `caller`, as hided hands it on
/// once the window granted the file: an outside file named by `grant`.
fn open_outside(
    runtime: &mut Runtime,
    caller: &str,
    grant: &str,
    name: &str,
    request: &str,
) -> String {
    let action = Action::OpenExternal {
        grant: grant.into(),
        name: name.into(),
        kind: crate::view_layout::ExternalKind::Html,
    };
    let expected = runtime
        .workspace_control_query(
            crate::node::TEST_NODE,
            caller,
            crate::workspace_control::Query::Info,
        )
        .unwrap()
        .context;
    let request = format!("{}-{request}", unix_milliseconds());
    let prepared = runtime
        .workspace_control_prepare_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &request,
            &action,
        )
        .unwrap();
    assert!(matches!(
        prepared,
        crate::workspace_control::ActionPreparation::Ready
    ));
    let answer = runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            caller,
            &expected,
            &request,
            action,
            Ok(None),
        )
        .unwrap();
    runtime.sync_workspace_view();
    answer.view_id
}

/// The rows of `pane`'s `N views` as the front Workspace publishes them.
fn pane_views(runtime: &Runtime, pane: &str) -> Vec<crate::model::OpenedViewSnapshot> {
    runtime
        .snapshot
        .workspace_view
        .as_ref()
        .and_then(|view| view.feedback.as_ref())
        .map(|feedback| {
            feedback
                .opened
                .iter()
                .filter(|row| row.pane_id == pane)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The same file in two areas is one row of the opener's `N views`; while
/// the tab the operator has not looked at is the second, the row wears its
/// 볼 것 mark and names that tab, so pressing the row brings it forward.
#[test]
fn a_row_for_a_file_in_two_tabs_names_the_tab_still_to_look_at() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("recipient"));
    let first = open(&mut guard, "recipient", "src/a.ts");
    let other = open(&mut guard, "recipient", "src/b.ts");
    let second_area = guard
        .change_front_layout(&key(), |layout, stamp| {
            let area = layout.active_area().id.clone();
            layout
                .split(&other, &area, crate::view_layout::Edge::Right, stamp)
                .map(|area| (area, true))
        })
        .unwrap();
    look_at(&mut guard, &first);
    keyboard(&mut guard, Some("recipient"));
    assert!(!unseen(&guard, &first));
    keyboard(&mut guard, Some("sender"));
    // The agent, no longer watched, opens the file again in the second area;
    // its open leaves the operator's front where it was (B1).
    let again = guard
        .change_front_layout(&key(), |layout, stamp| {
            let display = layout.new_display("src/a.ts", DisplayKind::File, None, false);
            let id = display.id.clone();
            layout.insert(&second_area, display, stamp)?;
            layout.focus(&first, stamp)?;
            Ok((id, true))
        })
        .unwrap();
    let action = Action::OpenFile {
        path: "src/a.ts".into(),
    };
    guard.stamp_control_opener(&key(), "recipient", &action, &again);
    guard.sync_workspace_view();
    assert!(unseen(&guard, &again));
    let rows = pane_views(&guard, "recipient");
    let row = rows.iter().find(|row| row.path == "src/a.ts").unwrap();
    assert_eq!(rows.iter().filter(|row| row.path == "src/a.ts").count(), 1);
    assert_eq!((row.view_id.as_str(), row.unseen), (again.as_str(), true));
}

/// Outside files carry no path, so only their grant tells them apart: each
/// is a row of its own in the opener's `N views`, and the one not looked at
/// keeps its 볼 것 mark there (UI_BEHAVIOR, View openers and comments).
#[test]
fn each_outside_file_an_agent_opens_is_its_own_row_and_keeps_its_unseen_mark() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let first = open_outside(
        &mut guard,
        "recipient",
        &"a1".repeat(16),
        "first.html",
        "first",
    );
    look_at(&mut guard, &first);
    keyboard(&mut guard, Some("recipient"));
    assert!(!unseen(&guard, &first));
    keyboard(&mut guard, Some("sender"));
    let second = open_outside(
        &mut guard,
        "recipient",
        &"b2".repeat(16),
        "second.html",
        "second",
    );
    assert_ne!(first, second);
    assert!(unseen(&guard, &second));
    let rows = pane_views(&guard, "recipient");
    let listed: Vec<(&str, bool)> = rows
        .iter()
        .map(|row| (row.label.as_str(), row.unseen))
        .collect();
    assert_eq!(listed, [("second.html", true), ("first.html", false)]);
}

/// A pane's `N views` lists its Views newest open first, whether or not the
/// operator has looked at them, and an agent's open again of an older View
/// brings it back to the top (UI_BEHAVIOR, View openers and comments).
#[test]
fn a_panes_views_list_the_newest_open_first_and_an_open_again_leads_it() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let first = open(&mut guard, "recipient", "src/a.ts");
    open(&mut guard, "recipient", "src/b.ts");
    let newest = open(&mut guard, "recipient", "src/c.ts");
    // The operator looks at the oldest and the newest, leaving the one
    // between to look at.
    keyboard(&mut guard, Some("recipient"));
    look_at(&mut guard, &first);
    look_at(&mut guard, &newest);
    keyboard(&mut guard, Some("sender"));
    let listed = |runtime: &Runtime| -> Vec<(String, bool)> {
        pane_views(runtime, "recipient")
            .into_iter()
            .map(|row| (row.path, row.unseen))
            .collect()
    };
    assert_eq!(
        listed(&guard),
        [
            ("src/c.ts".to_owned(), false),
            ("src/b.ts".to_owned(), true),
            ("src/a.ts".to_owned(), false),
        ]
    );
    // An open again of the oldest, on the desk behind the newest, marks it
    // To look at again and leads the list.
    assert_eq!(open(&mut guard, "recipient", "src/a.ts"), first);
    assert_eq!(
        listed(&guard),
        [
            ("src/a.ts".to_owned(), true),
            ("src/c.ts".to_owned(), false),
            ("src/b.ts".to_owned(), true),
        ]
    );
}

#[test]
fn an_agents_open_of_an_outside_file_is_tracked_as_its_open_by_grant_alone() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let grant = "a1".repeat(16);
    let open = |runtime: &mut Runtime, request: &str| {
        open_outside(runtime, "recipient", &grant, "report.html", request)
    };
    let view = open(&mut guard, "first");
    let display = guard.view_display(&key(), &view).unwrap().clone();
    assert_eq!(display.kind, DisplayKind::External);
    assert!(display.path.is_empty() && display.url.is_none());
    assert!(guard.snapshot.external_grants.contains(&grant));
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.group, "needs_you");
    assert_eq!(recipient.mark, RowMark::Look);
    assert_eq!(recipient.look.as_ref().unwrap().label, "report.html");
    let listed = guard
        .workspace_control_query(
            crate::node::TEST_NODE,
            "sender",
            crate::workspace_control::Query::ViewList,
        )
        .unwrap()
        .views
        .unwrap()
        .into_iter()
        .find(|row| row.view_id == view)
        .unwrap();
    assert_eq!((listed.kind, listed.target.as_str()), ("external", ""));
    assert!(listed.unseen && !listed.operator);
    assert_eq!(listed.openers.len(), 1);
    assert_eq!(
        listed.openers[0].session.as_deref(),
        Some("native-recipient")
    );
    // B10: the same file again is the same tab, brought forward.
    assert_eq!(open(&mut guard, "again"), view);
    let externals = guard
        .view_displays(&key())
        .filter(|display| display.kind == DisplayKind::External)
        .count();
    assert_eq!(externals, 1);
    look_at(&mut guard, &view);
    keyboard(&mut guard, Some("recipient"));
    assert!(!unseen(&guard, &view));
}

/// `open_external` exists only as hided makes it: no pane's request decodes
/// one, so a caller cannot name a grant it was never given.
#[test]
fn no_pane_request_decodes_an_outside_file_open() {
    let wire = json!({
        "action": "open_external",
        "grant": "a1".repeat(16),
        "name": "report.html",
        "kind": "html",
    });
    assert!(serde_json::from_value::<Action>(wire).is_err());
}

#[test]
fn an_output_looked_at_in_desks_lane_is_seen_and_its_row_leaves_needs_you() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    guard.start_test_desk();
    keyboard(&mut guard, Some("sender"));
    let view = open(&mut guard, "recipient", "notes.md");
    assert_eq!(row(&guard, "recipient").mark, RowMark::Look);
    // PRD session-docs-and-assistant B31: Desk's 보기 column shows the
    // output with the file viewers, outside the checkout's View areas.
    guard.dispatch_json(&explorer_event(
        "desk_output_seen",
        json!({"device_id": crate::node::TEST_NODE, "checkout": CHECKOUT, "id": view}),
    ));
    assert!(!unseen(&guard, &view));
    let recipient = row(&guard, "recipient");
    assert_eq!(recipient.group, "working");
    assert!(recipient.look.is_none());
}

/// PRD mobile-companion-ui D-15: the phone marks a View it showed seen with
/// the event Desk's lane sends, and Desk being off or not installed (the
/// fixture runs none) does not refuse it.
#[test]
fn a_view_seen_on_the_phone_is_seen_with_desk_off_and_the_row_leaves_needs_you() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    assert!(!guard.desk_installed(), "the fixture runs no Desk");
    keyboard(&mut guard, Some("sender"));
    let view = open(&mut guard, "recipient", "notes.md");
    assert_eq!(row(&guard, "recipient").mark, RowMark::Look);
    guard.dispatch_json(&explorer_event(
        "desk_output_seen",
        json!({"device_id": crate::node::TEST_NODE, "checkout": CHECKOUT, "id": view}),
    ));
    assert!(!unseen(&guard, &view));
    assert_eq!(row(&guard, "recipient").group, "working");
    // B25: opened again, it is to look at again.
    keyboard(&mut guard, Some("sender"));
    open(&mut guard, "recipient", "notes.md");
    assert!(unseen(&guard, &view));
}

/// PRD mobile-companion-ui B12, B24, B27: the phone's list is what the
/// agent opened and has open, seen or not; another agent's, and a closed
/// View, are not in it.
#[test]
fn the_phones_list_is_the_views_the_agent_opened_that_are_open() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let notes = open(&mut guard, "recipient", "docs/notes.md");
    let login = open(&mut guard, "recipient", "src/login.ts");
    let other = open(&mut guard, "sender", "src/sender.ts");
    let listed = guard.phone_looks("recipient");
    let ids: Vec<&str> = listed.views.iter().map(|v| v.display_id.as_str()).collect();
    assert_eq!(listed.omitted, 0);
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&notes.as_str()) && ids.contains(&login.as_str()));
    assert!(
        !ids.contains(&other.as_str()),
        "another agent's View is not listed"
    );
    assert!(listed.views.iter().all(|view| view.unseen));
    let notes_row = listed.views.iter().find(|v| v.display_id == notes).unwrap();
    assert_eq!(
        (
            notes_row.device_id.as_str(),
            notes_row.checkout_path.as_str()
        ),
        (crate::node::TEST_NODE, CHECKOUT)
    );
    assert_eq!(notes_row.label, "notes.md");
    // The phone's treatment of a file comes from the one table of outputs.
    assert_eq!(notes_row.output, Some("document"));
    let login_row = listed.views.iter().find(|v| v.display_id == login).unwrap();
    assert_eq!(login_row.output, None);
    // The same set the desktop's `N views` lists for that pane.
    let desktop: Vec<(String, bool)> = pane_views(&guard, "recipient")
        .into_iter()
        .map(|row| (row.view_id, row.unseen))
        .collect();
    let phone: Vec<(String, bool)> = listed
        .views
        .iter()
        .map(|v| (v.display_id.clone(), v.unseen))
        .collect();
    for pair in &desktop {
        assert!(phone.contains(pair), "{pair:?} in {phone:?}");
    }
    // Seen it stays listed, now as seen.
    guard.dispatch_json(&explorer_event(
        "desk_output_seen",
        json!({"device_id": crate::node::TEST_NODE, "checkout": CHECKOUT, "id": notes}),
    ));
    let listed = guard.phone_looks("recipient");
    let seen: Vec<(&str, bool)> = listed
        .views
        .iter()
        .map(|v| (v.display_id.as_str(), v.unseen))
        .collect();
    assert!(seen.contains(&(notes.as_str(), false)));
    assert!(seen.contains(&(login.as_str(), true)));
    // Closed on the desktop, it leaves the list (B12, B27).
    let place = guard.view_place(&key(), &login).unwrap();
    guard
        .change_view_layout(&key(), &place, |layout, _| {
            layout.remove(&login);
            Ok(((), true))
        })
        .unwrap();
    guard.sync_workspace_view();
    let listed = guard.phone_looks("recipient");
    assert!(listed.views.iter().all(|view| view.display_id != login));
    assert_eq!(listed.views.len(), 1);
    assert!(guard.phone_looks("no-such-pane").views.is_empty());
}
