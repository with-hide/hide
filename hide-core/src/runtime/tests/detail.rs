//! The sidebar's lens, the 태스크 view option and a task revealed from
//! outside the list (PRD sidebar-task-lens B2, B3, B40, D-47, D-48).
use super::*;
use crate::model::{DetailItem, PracticeProgress, SidebarLens, TaskView};

fn event(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event(kind, payload))
}

/// A 현황 tile narrows both lists and turns the sidebar to one in one event;
/// the chip's ✕ lifts the narrowing and leaves the lens where it is.
#[test]
fn the_sidebar_project_narrows_both_lists_until_lifted() {
    let mut runtime = runtime();
    assert_eq!(runtime.snapshot.ui_state.sidebar_project, None);
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    let project = serde_json::json!({"device_id": "mini", "workspace_id": "remote:mini:project:p"});
    assert!(event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": project, "lens": "prs", "show": true})
    ));
    let state = &runtime.snapshot.ui_state;
    assert_eq!(
        state
            .sidebar_project
            .as_ref()
            .map(|p| p.workspace_id.as_str()),
        Some("remote:mini:project:p")
    );
    assert_eq!(state.sidebar_lens, SidebarLens::Prs);
    assert!(state.left_sidebar_visible);
    assert!(!event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": project})
    ));
    assert!(event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": null})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_project, None);
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Prs);
}

fn practice_event(runtime: &mut Runtime, chapter: &str, step: u32, finished: bool) -> bool {
    event(
        runtime,
        "practice_progress_set",
        serde_json::json!({"chapter": chapter, "version": 2, "step": step, "finished": finished}),
    )
}

/// The sidebar's practice invitation closes once and stays closed: a second
/// close is no change, and a whole-state save from a shell that still read it
/// open does not bring it back.
#[test]
fn the_practice_invite_closes_once_and_a_stale_save_keeps_it_closed() {
    let mut runtime = runtime();
    assert!(!runtime.snapshot.ui_state.practice_invite_closed);
    let stale = super::appearance::ui_state_update(&runtime, serde_json::json!({}));
    assert!(event(
        &mut runtime,
        "practice_invite_close",
        serde_json::json!({})
    ));
    assert!(runtime.snapshot.ui_state.practice_invite_closed);
    assert!(
        !event(&mut runtime, "practice_invite_close", serde_json::json!({})),
        "closing it again is no change"
    );
    runtime.dispatch_json(&stale);
    assert!(runtime.snapshot.ui_state.practice_invite_closed);
    let snapshot = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
    assert_eq!(snapshot["practice_invite_closed"], serde_json::json!(true));
}

fn last_error_kind(runtime: &Runtime) -> Option<String> {
    runtime
        .snapshot
        .status
        .last_error
        .as_ref()
        .map(|error| error.kind.clone())
}

/// A learner's place in a practice chapter is written per chapter id, an
/// identical write changes nothing, and a later write replaces the entry.
#[test]
fn practice_progress_is_kept_per_chapter_and_an_identical_write_changes_nothing() {
    let mut runtime = runtime();
    assert!(runtime.snapshot.ui_state.practice.is_empty());
    assert!(practice_event(&mut runtime, "first-agent", 3, false));
    assert!(practice_event(&mut runtime, "sidebar-tour", 0, false));
    let practice = &runtime.snapshot.ui_state.practice;
    assert_eq!(practice.len(), 2);
    assert_eq!(
        practice["first-agent"],
        PracticeProgress {
            version: 2,
            step: 3,
            finished: false
        }
    );
    assert!(
        !practice_event(&mut runtime, "first-agent", 3, false),
        "the value it already holds is no change"
    );
    assert!(practice_event(&mut runtime, "first-agent", 7, true));
    let progress = runtime.snapshot.ui_state.practice["first-agent"];
    assert_eq!((progress.step, progress.finished), (7, true));
    assert_eq!(runtime.snapshot.ui_state.practice.len(), 2);
    assert_eq!(last_error_kind(&runtime), None);

    // The wire form a shell reads.
    let snapshot = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
    assert_eq!(
        snapshot["practice"]["first-agent"],
        serde_json::json!({"version": 2, "step": 7, "finished": true})
    );
}

/// A practice chapter counts its start and its end once each per
/// installation: later steps, a replay that keeps the chapter finished, and
/// a chapter the statistics do not name send nothing.
#[test]
fn a_practice_chapter_counts_its_start_and_end_once() {
    use crate::product_analytics::{Event, PracticeChapter, PracticeStage};
    let practice = |stage| Event::Practice {
        chapter: PracticeChapter::Mission,
        stage,
    };
    let mut runtime = runtime();
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());
    assert!(practice_event(&mut runtime, "mission", 0, false));
    assert!(practice_event(&mut runtime, "mission", 4, false));
    assert_eq!(statistics.take(), [practice(PracticeStage::Started)]);
    assert!(practice_event(&mut runtime, "mission", 9, true));
    assert!(practice_event(&mut runtime, "mission", 2, true));
    assert_eq!(statistics.take(), [practice(PracticeStage::Finished)]);
    assert!(practice_event(&mut runtime, "first-agent", 9, true));
    assert_eq!(statistics.take(), []);
    // A flow that was folded into another sends nothing; the one it was folded into counts.
    assert!(practice_event(&mut runtime, "screen", 9, true));
    assert_eq!(statistics.take(), []);
    // A flow that was dropped keeps what a learner already saved under its id, and sends nothing.
    assert!(practice_event(&mut runtime, "plan", 9, true));
    assert_eq!(statistics.take(), []);
    assert!(practice_event(&mut runtime, "agents", 0, false));
    assert_eq!(
        statistics.take(),
        [Event::Practice {
            chapter: PracticeChapter::Agents,
            stage: PracticeStage::Started
        }]
    );
}

/// A chapter id outside `^[a-z0-9-]{1,32}$` and a step past 255 are refused
/// with their codes and leave the progress as it was.
#[test]
fn practice_progress_refuses_a_bad_chapter_id_and_a_step_past_the_limit() {
    let mut runtime = runtime();
    assert!(practice_event(&mut runtime, "a-1", 255, false));
    for chapter in ["", "Upper", "has space", "under_score", &"a".repeat(33)] {
        practice_event(&mut runtime, chapter, 1, false);
        assert_eq!(
            last_error_kind(&runtime).as_deref(),
            Some("practice.chapter_invalid"),
            "{chapter:?}"
        );
        runtime.snapshot.status.last_error = None;
    }
    assert!(practice_event(&mut runtime, &"a".repeat(32), 1, false));
    practice_event(&mut runtime, "a-1", 256, false);
    assert_eq!(
        last_error_kind(&runtime).as_deref(),
        Some("practice.step_invalid")
    );
    let practice = &runtime.snapshot.ui_state.practice;
    assert_eq!(practice.len(), 2);
    assert_eq!(practice["a-1"].step, 255);
}

/// Progress is kept for 64 chapters; a 65th is refused, while a chapter
/// already kept can still be updated.
#[test]
fn practice_progress_keeps_at_most_sixty_four_chapters() {
    let mut runtime = runtime();
    for index in 0..64 {
        assert!(practice_event(&mut runtime, &format!("c{index}"), 0, false));
    }
    assert_eq!(last_error_kind(&runtime), None);
    practice_event(&mut runtime, "one-more", 0, false);
    assert_eq!(
        last_error_kind(&runtime).as_deref(),
        Some("practice.too_many")
    );
    let practice = &runtime.snapshot.ui_state.practice;
    assert_eq!(practice.len(), 64);
    assert!(!practice.contains_key("one-more"));
    assert!(practice_event(&mut runtime, "c0", 4, true));
    assert_eq!(runtime.snapshot.ui_state.practice["c0"].step, 4);
}

/// Removing the chosen project lifts the narrowing, so a restart never opens
/// the lists on a project that is gone.
#[test]
fn removing_the_chosen_project_shows_every_project() {
    let mut runtime = runtime();
    let project = "remote:mini:project:p1";
    runtime
        .snapshot
        .ui_state
        .workspace_registrations
        .push(crate::model::WorkspaceRegistration {
            primary_checkout_id: None,
            id: project.to_owned(),
            label: "api".to_owned(),
            path: "/srv/api".to_owned(),
            device_id: "mini".to_owned(),
            pinned: false,
        });
    assert!(event(
        &mut runtime,
        "sidebar_project_set",
        serde_json::json!({"project": {"device_id": "mini", "workspace_id": project}})
    ));
    assert!(runtime.retire_workspace_registration(project));
    assert_eq!(runtime.snapshot.ui_state.sidebar_project, None);
}

#[test]
fn the_lens_and_view_option_are_kept_and_a_hidden_sidebar_opens_on_the_other_lens() {
    let mut runtime = runtime();
    assert_eq!(
        runtime.snapshot.ui_state.sidebar_lens,
        SidebarLens::Projects
    );
    assert_eq!(runtime.snapshot.ui_state.task_view, TaskView::Status);
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    assert!(event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "tasks", "show": true})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Tasks);
    assert!(
        runtime.snapshot.ui_state.left_sidebar_visible,
        "⇧⌘B with the sidebar hidden shows it in the same event"
    );
    assert!(!event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "tasks"})
    ));
    assert!(event(
        &mut runtime,
        "task_view_set",
        serde_json::json!({"view": "project"})
    ));
    assert_eq!(runtime.snapshot.ui_state.task_view, TaskView::Project);
    // The PR list is the third lens (PRD lane-two-columns B41).
    assert!(event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "prs"})
    ));
    assert_eq!(runtime.snapshot.ui_state.sidebar_lens, SidebarLens::Prs);
    event(
        &mut runtime,
        "sidebar_lens_set",
        serde_json::json!({"lens": "sessions"}),
    );
    assert!(runtime.snapshot.status.last_error.is_some());
    assert_eq!(
        runtime.snapshot.ui_state.sidebar_lens,
        SidebarLens::Prs,
        "a lens this build does not know changes nothing"
    );
}

#[test]
fn a_revealed_task_unfolds_its_ancestors_and_opens_the_agent_s_record() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents = project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id":"root", "agent":"claude", "agent_status":"working", "state_change_seq":1},
            {"pane_id":"middle", "agent":"claude", "agent_status":"working", "state_change_seq":2},
            {"pane_id":"leaf", "agent":"claude", "agent_status":"idle", "state_change_seq":3}
        ]}))
        .unwrap(),
    )
    .agents;
    runtime.snapshot.navigator.agents[1].lineage_parent_pane_id = Some("root".into());
    runtime.snapshot.navigator.agents[2].lineage_parent_pane_id = Some("middle".into());
    runtime.snapshot.ui_state.left_sidebar_visible = false;
    let session = crate::links::ledger::SessionIdentity {
        device_id: "local".into(),
        agent: "claude".into(),
        session_id: "s-leaf".into(),
    };
    // With no Workspace in front the record has no desk to open on; the
    // sidebar still turns (`a_record_pressed_*` covers the card).
    assert!(event(
        &mut runtime,
        "task_reveal",
        serde_json::json!({"pane_id": "leaf", "session": session})
    ));
    let state = &runtime.snapshot.ui_state;
    assert!(state.left_sidebar_visible);
    assert_eq!(state.sidebar_lens, SidebarLens::Tasks);
    assert_eq!(state.sessions_expanded_agent_pane_ids, ["middle", "root"]);
}

/// The session id of the agent the fixtures run as agent `n`.
fn session_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

fn session(runtime: &Runtime, n: u32) -> crate::links::ledger::SessionIdentity {
    crate::links::ledger::SessionIdentity {
        device_id: runtime.node.as_str().to_owned(),
        agent: "claude".into(),
        session_id: session_id(n),
    }
}

/// `payload` with a Claude agent in each pane `agents` names, running the
/// session [`session_id`] makes of its number.
fn with_agents(
    mut payload: SessionSnapshotPayload,
    checkout_path: &str,
    agents: &[(&str, u32)],
) -> SessionSnapshotPayload {
    let rows: Vec<serde_json::Value> = agents
        .iter()
        .map(|(pane, n)| {
            serde_json::json!({
                "id": format!("agent-{n}"), "pane_id": pane, "agent": "claude",
                "agent_status": "working", "state_change_seq": 1, "cwd": checkout_path,
                "agent_session": {"kind": "id", "value": session_id(*n)}
            })
        })
        .collect();
    let owned: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({ "agents": rows })).unwrap();
    payload.agents.extend(owned.agents);
    payload
}

/// A live runtime on `checkout_path` with a View store, tabs `tabs` each
/// running agent `n` in pane `<tab>:p`, and `tabs[0]` in front.
fn desk_runtime(checkout_path: &str, name: &str, tabs: &[&str]) -> Runtime {
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, name);
    let panes: Vec<String> = tabs.iter().map(|tab| format!("{tab}:p")).collect();
    let agents: Vec<(&str, u32)> = panes
        .iter()
        .enumerate()
        .map(|(index, pane)| (pane.as_str(), index as u32 + 1))
        .collect();
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, tabs, tabs, tabs[0]),
        checkout_path,
        &agents,
    )));
    assert!(runtime.dispatch_json(&operator_focus_event(&panes[0])));
    runtime
}

/// The cards on the desk of `tab`, in tree order.
fn desk(runtime: &Runtime, checkout_path: &str, tab: &str) -> Vec<DetailItem> {
    runtime
        .workspace_views
        .as_ref()
        .and_then(|store| store.views.get(crate::node::TEST_NODE, checkout_path))
        .and_then(|view| view.tray(tab))
        .map(|tray| {
            tray.layout
                .displays()
                .filter_map(|display| display.card.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The desk the snapshot draws: its tab, whether it is shown, its count.
fn drawn_desk(runtime: &mut Runtime) -> (String, bool, usize) {
    runtime.sync_workspace_view();
    let view = runtime.snapshot.workspace_view.as_ref().unwrap();
    (view.tray.tab_id.clone(), view.tray.shown, view.tray.count)
}

fn open_at(runtime: &mut Runtime, item: &DetailItem, at: serde_json::Value) {
    assert!(event(
        runtime,
        "detail_open",
        serde_json::json!({"item": item, "at": at})
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// PRD lane-desk-pair-view D-06, B4, B9, B11: a card opens on the desk of
/// the tab in front and stays there; another tab in front draws its own desk
/// (empty until something opens there); coming back finds the card as it
/// was; and a tab that closes takes its desk, its cards going to the desk in
/// front.
#[test]
fn a_card_stays_on_the_desk_of_the_tab_it_opened_on() {
    let checkout_path = "/private/tmp/hide-desk-per-tab";
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let mut runtime = desk_runtime(checkout_path, "desk-per-tab", &tabs);
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    let worktrees = DetailItem::Worktrees {
        workspace_id: "workspace:order".into(),
    };
    open_at(&mut runtime, &cost, serde_json::json!("here"));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        std::slice::from_ref(&cost)
    );
    assert_eq!(drawn_desk(&mut runtime), ("w-order:t1".into(), true, 1));

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    assert_eq!(
        drawn_desk(&mut runtime),
        ("w-order:t2".into(), false, 0),
        "the other tab's card never follows"
    );
    open_at(&mut runtime, &worktrees, serde_json::json!("here"));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t2"),
        std::slice::from_ref(&worktrees)
    );

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(drawn_desk(&mut runtime), ("w-order:t1".into(), true, 1));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        std::slice::from_ref(&cost)
    );

    let remaining = ["w-order:t1", "w-order:t3"];
    let mut closed = with_agents(
        tab_order_payload(checkout_path, &remaining, &remaining, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t3:p", 3)],
    );
    closed.focused_pane_id = None;
    runtime.ingest_session(Ok(closed));
    let store = runtime.workspace_views.as_ref().unwrap();
    let view = store
        .views
        .get(crate::node::TEST_NODE, checkout_path)
        .unwrap();
    assert!(
        view.tray("w-order:t2").is_none(),
        "the closed tab's desk went"
    );
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        [cost, worktrees],
        "what the operator opened there goes to the desk in front"
    );
}

/// `tab_order_payload` with a second pane, `w-order:t1:q`, beside
/// `w-order:t1:p` in the first tab, each running an agent of its own
/// (sessions 1 and 2), and agent 3 in `w-order:t2:p`.
fn two_agents_in_one_tab(checkout_path: &str) -> SessionSnapshotPayload {
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut payload = tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1");
    payload.panes.push(
        serde_json::from_value(
            serde_json::json!({"pane_id": "w-order:t1:q", "cwd": checkout_path}),
        )
        .unwrap(),
    );
    let layout = payload
        .layouts
        .iter_mut()
        .find(|layout| layout.tab_id == "w-order:t1")
        .unwrap();
    layout.panes = serde_json::from_value(serde_json::json!([
        {"pane_id": "w-order:t1:p", "rect": {"x": 0, "y": 0, "width": 40, "height": 24}},
        {"pane_id": "w-order:t1:q", "rect": {"x": 40, "y": 0, "width": 40, "height": 24}}
    ]))
    .unwrap();
    with_agents(
        payload,
        checkout_path,
        &[
            ("w-order:t1:p", 1),
            ("w-order:t1:q", 2),
            ("w-order:t2:p", 3),
        ],
    )
}

/// PRD lane-desk-pair-view D-06: the desk is the Agent tab's, so two agents
/// sharing one tab share its desk: a card opened for one pane stays drawn
/// when the other pane of the tab takes the keyboard.
#[test]
fn two_panes_of_one_tab_share_its_desk() {
    let checkout_path = "/private/tmp/hide-desk-two-agents";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "desk-two-agents");
    runtime.ingest_session(Ok(two_agents_in_one_tab(checkout_path)));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    let map = DetailItem::Mission {
        mission: "mission-41".into(),
    };
    open_at(
        &mut runtime,
        &map,
        serde_json::json!({"pane": "w-order:t1:p"}),
    );
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:q")));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:q")
    );
    assert_eq!(drawn_desk(&mut runtime), ("w-order:t1".into(), true, 1));
    assert_eq!(desk(&runtime, checkout_path, "w-order:t1"), [map]);
}

/// PRD lane-desk-pair-view D-18, B34: the 보기 slots an older build kept
/// move once onto the desk of their pane's tab, in the order they stacked,
/// when that pane's tab is known: the slot kept for no pane goes to the pane
/// in front, a card two slots held lands once (D-20), a pane that closed
/// while Hide was not running takes its cards with it, a device's wait for
/// that device, and nothing is moved twice.
#[test]
fn the_slots_an_older_build_kept_move_onto_their_panes_desks_once() {
    let checkout_path = "/private/tmp/hide-desk-slot-move";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "desk-slot-move");
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    let worktrees = DetailItem::Worktrees {
        workspace_id: "workspace:order".into(),
    };
    let front = DetailItem::Mission {
        mission: "mission-9".into(),
    };
    let device = crate::address::scoped_pane_id("mini", "w1:p2");
    let moving = |pane: Option<&str>, cards: Vec<DetailItem>| crate::model::MovingCards {
        pane: pane.map(str::to_owned),
        cards,
        shown: true,
    };
    runtime.snapshot.ui_state.moving_cards = vec![
        moving(None, vec![front.clone()]),
        moving(Some(&device), vec![cost.clone()]),
        moving(Some("w-order:t9:p"), vec![cost.clone()]),
        moving(Some("w-order:t1:p"), vec![cost.clone(), worktrees.clone()]),
    ];

    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    let desks = [
        desk(&runtime, checkout_path, "w-order:t1"),
        desk(&runtime, checkout_path, "w-order:t2"),
    ];
    let all: Vec<&DetailItem> = desks.iter().flatten().collect();
    assert_eq!(
        all.iter().filter(|card| ***card == cost).count(),
        1,
        "{desks:?}"
    );
    assert!(
        desks[0].contains(&worktrees) && desks[0].contains(&cost),
        "{desks:?}"
    );
    assert!(
        all.contains(&&front),
        "the slot kept for no pane lands: {desks:?}"
    );
    let waiting: Vec<_> = runtime
        .snapshot
        .ui_state
        .moving_cards
        .iter()
        .map(|held| held.pane.clone())
        .collect();
    assert_eq!(waiting, [Some(device)], "only the device's slot waits");
    assert!(
        !serde_json::to_string(&runtime.snapshot.ui_state)
            .unwrap()
            .contains("moving_cards"),
        "the shell never reads the slot"
    );
    assert!(!runtime.move_slot_cards(), "nothing moves twice");
}

/// PRD lane-desk-pair-view B34: moving an older build's slot onto a desk
/// that is nearly full closes none of the documents on it; the cards the
/// slot showed last take the room left, and the older ones stay out.
#[test]
fn a_slot_moving_onto_a_full_desk_closes_no_document_and_keeps_its_newest_cards() {
    let checkout_path = "/private/tmp/hide-desk-slot-full";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "desk-slot-full");
    let documents = crate::view_layout::MAX_VIEW_DISPLAYS - 2;
    {
        let desk = runtime
            .workspace_views
            .as_mut()
            .unwrap()
            .views
            .entry(crate::node::TEST_NODE, checkout_path)
            .tray_entry("w-order:t1");
        let area = desk.layout.active_area.clone();
        for index in 0..documents {
            let document = desk.layout.new_display(
                &format!("{checkout_path}/doc-{index}.md"),
                crate::view_layout::DisplayKind::File,
                None,
                false,
            );
            desk.layout.append(&area, document).unwrap();
        }
    }
    let cards = [
        DetailItem::Cost {
            workspace_id: "workspace:order".into(),
        },
        DetailItem::Worktrees {
            workspace_id: "workspace:order".into(),
        },
        DetailItem::Mission {
            mission: "mission-9".into(),
        },
    ];
    runtime.snapshot.ui_state.moving_cards = vec![crate::model::MovingCards {
        pane: Some("w-order:t1:p".into()),
        cards: cards.to_vec(),
        shown: true,
    }];

    let tabs = ["w-order:t1"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert_eq!(desk(&runtime, checkout_path, "w-order:t1"), cards[1..]);
    let files = runtime
        .workspace_views
        .as_ref()
        .and_then(|store| store.views.get(crate::node::TEST_NODE, checkout_path))
        .and_then(|view| view.tray("w-order:t1"))
        .map_or(0, |tray| {
            tray.layout
                .displays()
                .filter(|display| display.card.is_none())
                .count()
        });
    assert_eq!(files, documents, "every document stays open");
    assert!(runtime.snapshot.ui_state.moving_cards.is_empty());
}

/// Operator decision 2026-11-09, PRD lane-desk-pair-view B6, B7: a record
/// pressed where no agent is opens on the desk of the tab of the pane that
/// runs its session, which comes forward; a session with no pane here opens
/// on the desk in front; and a record opened inside a card (`here`) is a
/// new tab of the desk it was pressed on.
#[test]
fn a_record_pressed_where_no_agent_is_opens_in_its_sessions_pane() {
    let checkout_path = "/private/tmp/hide-desk-owner-session";
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut runtime = desk_runtime(checkout_path, "desk-owner-session", &tabs);
    let session = session(&runtime, 2);
    let record = |session: &crate::links::ledger::SessionIdentity| DetailItem::Record {
        session: session.clone(),
    };

    open_at(&mut runtime, &record(&session), serde_json::json!("owner"));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t2"),
        [record(&session)]
    );
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p"),
        "the session's pane takes the keyboard"
    );
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert!(desk(&runtime, checkout_path, "w-order:t1").is_empty());

    let gone = crate::links::ledger::SessionIdentity {
        session_id: "ended".into(),
        ..session.clone()
    };
    open_at(&mut runtime, &record(&gone), serde_json::json!("owner"));
    assert_eq!(desk(&runtime, checkout_path, "w-order:t1"), [record(&gone)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p")
    );

    open_at(&mut runtime, &record(&session), serde_json::json!("here"));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        [record(&gone), record(&session)]
    );
    assert!(
        desk(&runtime, checkout_path, "w-order:t2").is_empty(),
        "one card is on one desk: it came to the desk it was pressed on (D-20)"
    );
}

/// A runtime whose project holds pull requests #42 and #43, each made by the
/// agent `maker` names (a pane id).
fn with_pull_requests(runtime: &mut Runtime, makers: &[(u32, &str)]) {
    let pull = |number: u32| -> crate::model::PullRequestSnapshot {
        serde_json::from_value(serde_json::json!({
            "number":number,"title":"Ship","url":format!("https://github.com/acme/app/pull/{number}"),
            "head_branch":format!("feature-{number}"),"base_branch":"main","badge":"open","checks":"passing",
            "is_draft":false,"closing_issues":[]
        }))
        .unwrap()
    };
    let row = |number: u32, pane: &str| crate::agent_state::work::board::Row {
        number,
        group: "open",
        fold: None,
        reason: Default::default(),
        agents: vec![crate::agent_state::tally::scope::RowRef {
            pane_id: pane.into(),
            occurrence: 0,
        }],
        agent_pane: None,
        checkout_id: None,
        checkout_ids: Vec::new(),
        issue: None,
    };
    let project = &mut runtime.snapshot.navigator.workspaces[0];
    project.is_git = true;
    project.pull_requests = makers.iter().map(|(number, _)| pull(*number)).collect();
    project.agent_scope.prs.rows = makers
        .iter()
        .map(|(number, maker)| row(*number, maker))
        .collect();
}

fn open_pull_request(runtime: &mut Runtime, number: u32, at: serde_json::Value) {
    assert!(event(
        runtime,
        "pr_open",
        serde_json::json!({"workspace_id": "workspace:order", "pr_number": number, "at": at})
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

fn card(number: u32) -> DetailItem {
    DetailItem::PullRequest {
        workspace_id: "workspace:order".into(),
        number,
    }
}

/// Operator decision 2026-11-09: a pull request pressed where no agent is
/// (the PR list) opens on the desk of the tab of the agent that made it,
/// whose pane comes forward; one whose agent has no pane here opens on the
/// desk in front.
#[test]
fn a_pull_request_pressed_where_no_agent_is_opens_at_its_makers_pane() {
    let checkout_path = "/private/tmp/hide-desk-owner-pr";
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut runtime = desk_runtime(checkout_path, "desk-owner-pr", &tabs);
    with_pull_requests(&mut runtime, &[(42, "w-order:t2:p"), (43, "gone")]);

    open_pull_request(&mut runtime, 42, serde_json::json!("owner"));
    assert_eq!(desk(&runtime, checkout_path, "w-order:t2"), [card(42)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p")
    );

    open_pull_request(&mut runtime, 43, serde_json::json!("owner"));
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t2"),
        [card(42), card(43)]
    );
}

/// Operator report 2026-11-09: a pull request pressed on an agent's row
/// opens on the desk of that row's tab, whoever made it, and that pane
/// takes the keyboard; the maker's desk takes nothing. A row whose pane
/// closed leaves the card on the desk in front.
#[test]
fn a_pull_request_pressed_on_a_parents_row_opens_on_the_parents_desk() {
    let checkout_path = "/private/tmp/hide-desk-pressed-row";
    let tabs = ["w-order:t3", "w-order:t1", "w-order:t2"];
    let mut runtime = desk_runtime(checkout_path, "desk-pressed-row", &tabs);
    let parent = "w-order:t1:p";
    let child = "w-order:t2:p";
    with_pull_requests(&mut runtime, &[(42, child), (43, child)]);

    for number in [42, 43] {
        open_pull_request(&mut runtime, number, serde_json::json!({"pane": parent}));
        assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(parent));
    }
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        [card(42), card(43)]
    );
    assert!(desk(&runtime, checkout_path, "w-order:t2").is_empty());

    open_pull_request(
        &mut runtime,
        42,
        serde_json::json!({"pane": "w-order:t9:p"}),
    );
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        [card(42), card(43)]
    );
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(parent));
}

/// Operator report 2026-11-09: a pull request pressed inside a session's
/// record jumped to the tab of the agent that made it. Pressed inside a
/// card it is a new tab of the same desk, and the tab and the keyboard stay.
#[test]
fn a_pull_request_pressed_inside_a_record_opens_beside_it_on_the_same_desk() {
    let checkout_path = "/private/tmp/hide-desk-inside-card";
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut runtime = desk_runtime(checkout_path, "desk-inside-card", &tabs);
    with_pull_requests(&mut runtime, &[(42, "w-order:t2:p")]);
    let record = DetailItem::Record {
        session: session(&runtime, 2),
    };
    open_at(
        &mut runtime,
        &record,
        serde_json::json!({"pane": "w-order:t1:p"}),
    );
    open_pull_request(&mut runtime, 42, serde_json::json!("here"));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p"),
        "the keyboard stays"
    );
    assert_eq!(
        desk(&runtime, checkout_path, "w-order:t1"),
        [record, card(42)]
    );
}
