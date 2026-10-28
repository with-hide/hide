//! An agent's open the operator has not looked at (PRD unseen-agent-views):
//! the row it puts in Needs You, what looking at it takes, and what a
//! delegated child's open does to its ancestors. Each test drives the
//! runtime's own opens, events and reads, and reads the rows it publishes.

use super::*;
use crate::agent_state::RowMark;
use crate::view_layout::DisplayKind;
use crate::workspace_control::Action;
use serde_json::json;

const CHECKOUT: &str = "/checkouts/fixture";

fn key() -> (String, String) {
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
fn fixture(child: bool) -> (tempfile::TempDir, Arc<Mutex<Runtime>>) {
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
        let store = guard.workspace_views.as_mut().unwrap();
        let entry = store.views.entry(crate::node::TEST_NODE, CHECKOUT);
        entry.open = true;
        entry.documents = true;
        guard.set_window_focused(true);
        guard.refresh_agent_lineage();
        guard.refresh_pane_read_state();
    }
    (root, runtime)
}

/// The operator's keyboard on `pane`, or on no pane, as the strip settles
/// it before the next frame.
fn keyboard(runtime: &mut Runtime, pane: Option<&str>) {
    runtime.snapshot.terminal.pane_id = pane.map(str::to_owned);
    runtime.operator_focused_pane_id = pane.map(str::to_owned);
    runtime.refresh_pane_read_state();
    runtime.sync_workspace_view();
}

/// `hide file open <path>` (or a page) from `caller`, as the daemon applies
/// it once the file is read: the View lands in the active area and is marked
/// by its caller.
fn open(runtime: &mut Runtime, caller: &str, path: &str) -> String {
    let key = key();
    let view = runtime
        .change_view_layout(&key, |layout, stamp| {
            let existing = layout
                .displays()
                .find(|display| display.shows(path, DisplayKind::File, None))
                .map(|display| display.id.clone());
            let id = match existing {
                Some(id) => {
                    layout.focus(&id, stamp)?;
                    id
                }
                None => {
                    let display = layout.new_display(path, DisplayKind::File, None, false);
                    let id = display.id.clone();
                    let area = layout.active_area().id.clone();
                    layout.insert(&area, display, stamp)?;
                    id
                }
            };
            Ok((id, true))
        })
        .unwrap();
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
        .view_layout_of(&key())
        .and_then(|layout| layout.display(view))
        .is_some_and(|display| display.unseen.is_some())
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

/// The View comes forward as the operator's press on it does.
fn look_at(runtime: &mut Runtime, view: &str) {
    runtime
        .change_view_layout(&key(), |layout, stamp| {
            layout.focus(view, stamp).map(|changed| ((), changed))
        })
        .unwrap();
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
    assert_eq!(look.group, "working", "notices follow the row without it");
    // B12: it is counted as the operator's turn.
    let mut counts = crate::model::MarkCountsSnapshot::default();
    recipient.mark.count_into(&mut counts);
    assert_eq!(counts.turn, 1);
    // B4, D-20: seeing it moves the row back without reading the pane.
    keyboard(&mut guard, Some("recipient"));
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
    assert_eq!(row(&guard, "recipient").group, "needs_you");
    // B7: the lane shows no documents.
    {
        let store = guard.workspace_views.as_mut().unwrap();
        store.views.entry(crate::node::TEST_NODE, CHECKOUT).documents = false;
    }
    guard.set_window_focused(true);
    guard.sync_workspace_view();
    assert!(unseen(&guard, &view));
    // Back on the documents, with a window holding the keyboard, it is seen.
    {
        let store = guard.workspace_views.as_mut().unwrap();
        store.views.entry(crate::node::TEST_NODE, CHECKOUT).documents = true;
    }
    guard.sync_workspace_view();
    assert!(!unseen(&guard, &view));
}

#[test]
fn a_view_in_an_area_that_is_not_active_stays_unseen() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let first = open(&mut guard, "recipient", "src/a.ts");
    let second = open(&mut guard, "recipient", "src/b.ts");
    // The first View moves to an area of its own; the second stays active.
    guard
        .change_view_layout(&key(), |layout, stamp| {
            let area = layout.active_area().id.clone();
            layout
                .split(&first, &area, crate::view_layout::Edge::Right, stamp)
                .map(|_| ((), true))
        })
        .unwrap();
    guard
        .change_view_layout(&key(), |layout, stamp| {
            let area = layout.area_of(&second).unwrap().id.clone();
            layout.focus_area(&area, stamp).map(|changed| ((), changed))
        })
        .unwrap();
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
    guard
        .change_view_layout(&key(), |layout, _| {
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
    keyboard(&mut guard, Some("sender"));
    // B14, D-11: the same path brought forward again by the agent.
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
        .change_view_layout(&key(), |layout, stamp| {
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
fn a_view_to_look_at_sends_no_phone_or_desktop_notice() {
    let (_root, runtime) = fixture(false);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, Some("sender"));
    let rows = |runtime: &Runtime| {
        json!({"navigator": {"agents": serde_json::to_value(&runtime.snapshot.navigator.agents).unwrap()}})
    };
    let mut transitions = crate::agent_state::push::Transitions::default();
    let seeded = crate::agent_state::phone::project(&rows(&guard), crate::node::TEST_NODE);
    transitions.observe(&seeded);
    open(&mut guard, "recipient", "src/login.ts");
    assert_eq!(row(&guard, "recipient").group, "needs_you");
    // B12, D-13: the notices follow the row as it would be without the View.
    let (notices, _) = transitions.observe(&crate::agent_state::phone::project(
        &rows(&guard),
        crate::node::TEST_NODE,
    ));
    assert!(notices.is_empty(), "{notices:?}");
    // A block on the same row still calls the operator.
    blocked(&mut guard, true);
    let (notices, _) = transitions.observe(&crate::agent_state::phone::project(
        &rows(&guard),
        crate::node::TEST_NODE,
    ));
    assert_eq!(notices.len(), 1);
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
    guard.note_ancestor_read(&key(), &view, "recipient");
    guard.note_ancestor_read(&key(), &view, "stranger");
    assert!(row(&guard, "sender").unread);
    // (3) Its parent reads it: the signal goes, the View stays unseen (B22).
    guard.note_ancestor_read(&key(), &view, "sender");
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
    assert!(!unseen(&guard, &view));
    assert!(row(&guard, "sender").look.is_none());
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
    guard
        .change_view_layout(&key(), |layout, _| {
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
    assert!(body.starts_with("Done.\n\nViews this agent opened"), "{body}");
    assert!(
        body.contains(&format!("- {view} src/report.md (not seen by the operator)")),
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
        store.views.entry(crate::node::TEST_NODE, CHECKOUT).documents = false;
    }
    // B3: the View comes forward on its pane, and is seen once in front.
    keyboard(&mut guard, Some("recipient"));
    guard.reveal_unseen_view("recipient");
    guard.sync_workspace_view();
    assert!(!unseen(&guard, &view));
    assert!(row(&guard, "recipient").look.is_none());
}
