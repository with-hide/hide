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
    assert!(event(
        &mut runtime,
        "task_reveal",
        serde_json::json!({"pane_id": "leaf", "session": session})
    ));
    let state = &runtime.snapshot.ui_state;
    assert!(state.left_sidebar_visible);
    assert_eq!(state.sidebar_lens, SidebarLens::Tasks);
    assert_eq!(state.sessions_expanded_agent_pane_ids, ["middle", "root"]);
    assert_eq!(state.detail.items, [DetailItem::Record { session }]);
    assert!(state.detail.shown);
}

/// The slot steps back through at most 20 items, the oldest leaving first,
/// and opening one it already holds moves it to the front rather than adding
/// a second entry (B47).
#[test]
fn the_detail_slot_keeps_the_newest_twenty_items_and_reopening_one_moves_it_forward() {
    let mut runtime = runtime();
    let record = |index: usize| DetailItem::Record {
        session: crate::links::ledger::SessionIdentity {
            device_id: "local".into(),
            agent: "claude".into(),
            session_id: format!("s-{index}"),
        },
    };
    for index in 0..22 {
        assert!(event(
            &mut runtime,
            "detail_open",
            serde_json::json!({"item": record(index)})
        ));
    }
    let items = &runtime.snapshot.ui_state.detail.items;
    assert_eq!(items.len(), crate::model::DETAIL_HISTORY_LIMIT);
    assert_eq!(items.first(), Some(&record(2)), "the two oldest left first");
    assert_eq!(items.last(), Some(&record(21)));
    event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": record(5)}),
    );
    let items = &runtime.snapshot.ui_state.detail.items;
    assert_eq!(items.len(), crate::model::DETAIL_HISTORY_LIMIT);
    assert_eq!(items.last(), Some(&record(5)));
    assert_eq!(items.iter().filter(|item| **item == record(5)).count(), 1);
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

/// The slot belongs to the pane it was opened for (operator decision
/// 2026-11-09): another pane in front shows its own slot or none, coming back
/// finds the card as it was, and a pane that closes takes its slot with it,
/// so an agent that sleeps and wakes in a new pane finds that pane's slot
/// empty.
#[test]
fn the_detail_slot_stays_with_the_pane_it_was_opened_for() {
    let checkout_path = "/private/tmp/hide-slot-per-agent";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    let agents = [("w-order:t1:p", 1), ("w-order:t2:p", 2), ("w-order:t3:p", 3)];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &agents,
    )));
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    let worktrees = DetailItem::Worktrees {
        workspace_id: "workspace:order".into(),
    };
    let front = |runtime: &mut Runtime, tab: &str| {
        assert!(runtime.dispatch_json(&operator_focus_event(&format!("{tab}:p"))));
    };

    front(&mut runtime, "w-order:t1");
    assert!(event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": cost})
    ));
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        std::slice::from_ref(&cost)
    );
    assert_eq!(detail_pane(&runtime), Some("w-order:t1:p"));

    // Another pane in front: no card of its own, so none shows; the first
    // pane's card is kept for it.
    front(&mut runtime, "w-order:t2");
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the other pane's card never follows"
    );
    assert!(
        !event(
            &mut runtime,
            "detail_show",
            serde_json::json!({"shown": true})
        ),
        "the rail's 보기 has nothing of this pane's to bring forward"
    );
    assert!(event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": worktrees})
    ));
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        std::slice::from_ref(&worktrees)
    );

    // Back on the first pane its card is there as it was; a second click on
    // the same pane keeps it.
    front(&mut runtime, "w-order:t1");
    front(&mut runtime, "w-order:t1");
    let slot = &runtime.snapshot.ui_state.detail;
    assert_eq!(
        (slot.items.as_slice(), slot.shown),
        ([cost.clone()].as_slice(), true)
    );
    front(&mut runtime, "w-order:t2");
    assert_eq!(runtime.snapshot.ui_state.detail.items, [worktrees]);

    // A pane that closes takes its slot, and nothing is re-homed to
    // another.
    front(&mut runtime, "w-order:t1");
    let remaining = ["w-order:t1", "w-order:t3"];
    let mut closed = with_agents(
        tab_order_payload(checkout_path, &remaining, &remaining, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t3:p", 3)],
    );
    closed.focused_pane_id = None;
    runtime.ingest_session(Ok(closed));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert!(
        runtime.snapshot.ui_state.detail_panes.is_empty(),
        "the closed pane's slot is gone: {:?}",
        runtime.snapshot.ui_state.detail_panes
    );
}

/// `tab_order_payload` with a second pane, `w-order:t1:q`, beside
/// `w-order:t1:p` in the first tab, each running an agent of its own
/// (sessions 1 and 2), and agent 3 in `w-order:t2:p`.
fn two_agents_in_one_tab(checkout_path: &str) -> SessionSnapshotPayload {
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut payload = tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1");
    payload.panes.push(
        serde_json::from_value(serde_json::json!({"pane_id": "w-order:t1:q", "cwd": checkout_path}))
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
        &[("w-order:t1:p", 1), ("w-order:t1:q", 2), ("w-order:t2:p", 3)],
    )
}

/// Operator report 2026-11-09: two agents shared one Agent tab, a mission's
/// card opened for one of them, and the other agent, pressed, showed it too.
/// The slot is the pane's, so the other pane of the same tab shows its own
/// slot (none), and the card comes back with its own pane.
#[test]
fn a_card_opened_for_one_pane_is_not_shown_on_another_pane_of_the_same_tab() {
    let checkout_path = "/private/tmp/hide-slot-two-agents";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    runtime.ingest_session(Ok(two_agents_in_one_tab(checkout_path)));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    let map = DetailItem::Mission {
        mission: "mission-41".into(),
    };
    assert!(event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": map})
    ));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [map.clone()]);

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:q")));
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:q")
    );
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the other pane of the same tab shows no mission card"
    );
    let wire = serde_json::to_value(&runtime.snapshot.ui_state).unwrap();
    assert_eq!(wire["detail"]["items"], serde_json::json!([]));

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [map]);
}

/// A pane that runs no agent (a shell) has a slot of its own like any other
/// pane: what was opened there comes back with it.
#[test]
fn a_pane_without_an_agent_keeps_its_own_slot() {
    let checkout_path = "/private/tmp/hide-slot-shell";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1)],
    )));
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    open_at(&mut runtime, &cost, serde_json::json!("here"));

    let shell = "w-order:t2:p";
    assert!(runtime.dispatch_json(&operator_focus_event(shell)));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default()
    );
    let worktrees = DetailItem::Worktrees {
        workspace_id: "workspace:order".into(),
    };
    open_at(&mut runtime, &worktrees, serde_json::json!("here"));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [worktrees.clone()]);

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert!(runtime.dispatch_json(&operator_focus_event(shell)));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [worktrees]);
}

/// The slot loaded at launch (one an older build kept for its front tab, or
/// window-wide) attaches to the pane in front when the first event arrives,
/// and stays with it; a kept slot of a pane that closed while Hide was not
/// running is dropped by the first session, and a device's waits for that
/// device's own session.
#[test]
fn a_slot_loaded_at_launch_attaches_to_the_front_pane_and_closed_panes_slots_go() {
    let checkout_path = "/private/tmp/hide-slot-launch";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    let held = || {
        let mut slot = crate::model::DetailSlot::default();
        slot.open(DetailItem::Worktrees {
            workspace_id: "workspace:order".into(),
        });
        slot
    };
    // As a launch loads it: a slot, no pane it belongs to yet, and the slots
    // kept for a device's pane, for a pane that closed, and for pane t1.
    runtime.snapshot.ui_state.detail.open(cost.clone());
    assert_eq!(detail_pane(&runtime), None);
    let device = crate::address::scoped_pane_id("mini", "w1:p2");
    let kept = &mut runtime.snapshot.ui_state.detail_panes;
    kept.keep(device.clone(), held());
    kept.keep("w-order:t9:p".into(), held());
    kept.keep("w-order:t1:p".into(), held());

    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));
    let kept: Vec<_> = runtime
        .snapshot
        .ui_state
        .detail_panes
        .0
        .iter()
        .map(|held| held.pane.clone())
        .collect();
    assert_eq!(
        kept,
        [device, "w-order:t1:p".to_owned()],
        "the closed pane's slot went"
    );

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert_eq!(detail_pane(&runtime), Some("w-order:t2:p"));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(runtime.snapshot.ui_state.detail, held());
}

fn detail_pane(runtime: &Runtime) -> Option<&str> {
    runtime.snapshot.ui_state.detail_pane.as_deref()
}

fn open_at(runtime: &mut Runtime, item: &DetailItem, at: serde_json::Value) {
    assert!(event(
        runtime,
        "detail_open",
        serde_json::json!({"item": item, "at": at})
    ));
    assert_eq!(runtime.snapshot.status.last_error, None);
}

/// Operator decision 2026-11-09: a record pressed where no agent is opens in
/// the slot of the pane that runs the session it records, which comes
/// forward; a session with no pane here opens in the slot in front, and a
/// record opened inside the slot (a child's, `here`) stays in the slot it was
/// pressed in.
#[test]
fn a_record_pressed_where_no_agent_is_opens_in_its_sessions_pane() {
    let checkout_path = "/private/tmp/hide-slot-owner-session";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t2:p", 2)],
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    let session = session(&runtime, 2);
    let record = |session: &crate::links::ledger::SessionIdentity| DetailItem::Record {
        session: session.clone(),
    };

    open_at(&mut runtime, &record(&session), serde_json::json!("owner"));
    assert_eq!(detail_pane(&runtime), Some("w-order:t2:p"));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [record(&session)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p"),
        "the session's pane takes the keyboard"
    );
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the pane that was in front holds no card"
    );

    // No pane runs this session here: the slot in front takes it.
    let gone = crate::links::ledger::SessionIdentity {
        session_id: "ended".into(),
        ..session.clone()
    };
    open_at(&mut runtime, &record(&gone), serde_json::json!("owner"));
    assert_eq!(
        detail_pane(&runtime),
        Some("w-order:t1:p")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [record(&gone)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p")
    );

    // Opened inside the slot, the record stays where it was pressed.
    open_at(&mut runtime, &record(&session), serde_json::json!("here"));
    assert_eq!(
        detail_pane(&runtime),
        Some("w-order:t1:p")
    );
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        [record(&gone), record(&session)]
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
/// (the PR list) opens in the slot of the agent that made it, whose pane
/// comes forward; one whose agent has no pane here opens in the pane in
/// front.
#[test]
fn a_pull_request_pressed_where_no_agent_is_opens_at_its_makers_pane() {
    let checkout_path = "/private/tmp/hide-slot-owner-pr";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "slot-owner-pr");
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t2:p", 2)],
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    with_pull_requests(&mut runtime, &[(42, "w-order:t2:p"), (43, "gone")]);

    open_pull_request(&mut runtime, 42, serde_json::json!("owner"));
    assert_eq!(
        detail_pane(&runtime),
        Some("w-order:t2:p")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [card(42)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p")
    );

    // Its agent has no pane here: the slot in front takes the card.
    open_pull_request(&mut runtime, 43, serde_json::json!("owner"));
    assert_eq!(
        detail_pane(&runtime),
        Some("w-order:t2:p")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [card(42), card(43)]);
}

/// Operator report 2026-11-09: on a parent's row, one pull request its child
/// made went to the child's tab and another stayed. A pull request pressed
/// on an agent's row opens in that row's pane's slot, whoever made it, and
/// that pane takes the keyboard; the maker's pane keeps its own slot.
#[test]
fn a_pull_request_pressed_on_a_parents_row_opens_in_the_parents_slot() {
    let checkout_path = "/private/tmp/hide-slot-pressed-row";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "slot-pressed-row");
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t3"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t2:p", 2), ("w-order:t3:p", 3)],
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t3:p")));
    // The parent works in t1, its child in t2; the child made both.
    let parent = "w-order:t1:p";
    let child = "w-order:t2:p";
    with_pull_requests(&mut runtime, &[(42, child), (43, child)]);

    for number in [42, 43] {
        open_pull_request(&mut runtime, number, serde_json::json!({"pane": parent}));
        assert_eq!(
            detail_pane(&runtime),
            Some(parent),
            "#{number} opens where it was pressed"
        );
        assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(parent));
        assert_eq!(runtime.snapshot.ui_state.detail.items.last(), Some(&card(number)));
    }
    assert_eq!(runtime.snapshot.ui_state.detail.items, [card(42), card(43)]);
    assert!(runtime.dispatch_json(&operator_focus_event(child)));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the child's own slot took neither"
    );

    // A record pressed on the child's row opens in the child's slot.
    let record = DetailItem::Record {
        session: session(&runtime, 2),
    };
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t3:p")));
    open_at(&mut runtime, &record, serde_json::json!({"pane": child}));
    assert_eq!(detail_pane(&runtime), Some(child));
    assert_eq!(runtime.snapshot.focused.pane_id.as_deref(), Some(child));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [record]);

    // A row whose pane closed (its agent sleeps): the slot in front takes it.
    open_pull_request(&mut runtime, 42, serde_json::json!({"pane": "w-order:t9:p"}));
    assert_eq!(detail_pane(&runtime), Some(child));
    assert_eq!(runtime.snapshot.ui_state.detail.items.last(), Some(&card(42)));
}

/// Operator report 2026-11-09: a pull request pressed inside a session's
/// record jumped to the tab of the agent that made it. Pressed inside a
/// card it stacks in the same slot, the tab and the keyboard stay, and ←
/// comes back to the record, as a child's record does.
#[test]
fn a_pull_request_pressed_inside_a_record_stacks_in_the_same_slot_and_back_returns() {
    let checkout_path = "/private/tmp/hide-slot-inside-card";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "slot-inside-card");
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1), ("w-order:t2:p", 2)],
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    // The maker works in t2.
    with_pull_requests(&mut runtime, &[(42, "w-order:t2:p")]);
    let record = DetailItem::Record {
        session: session(&runtime, 2),
    };
    open_at(&mut runtime, &record, serde_json::json!({"pane": "w-order:t1:p"}));

    open_pull_request(&mut runtime, 42, serde_json::json!("here"));
    assert_eq!(
        detail_pane(&runtime),
        Some("w-order:t1:p")
    );
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p"),
        "the keyboard stays"
    );
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        [record.clone(), card(42)]
    );
    assert!(event(&mut runtime, "detail_back", serde_json::json!({})));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [record]);
    assert!(runtime.snapshot.ui_state.detail.shown);
}

/// An agent's row on a connected device opens the card in that pane's slot:
/// the device's pane comes forward only when the device answers, so the card
/// waits in that pane's slot while this machine's pane keeps its own, and
/// shows once the device reports the pane in front. A device's kept slot
/// goes only when that device's session no longer lists the pane, not while
/// the device is offline.
#[test]
fn a_device_agents_row_opens_its_card_when_the_device_brings_the_pane_forward() {
    let checkout_path = "/private/tmp/hide-slot-device-row";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1"];
    runtime.ingest_session(Ok(with_agents(
        tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1"),
        checkout_path,
        &[("w-order:t1:p", 1)],
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));

    let target_id = "mini";
    let device_pane = crate::address::scoped_pane_id(target_id, "w1:p2");
    let workspace_id = "remote:mini:workspace:w1";
    let checkout_id = "remote:mini:checkout:w1";
    let tab_id = "remote:mini:tab:w1:t1";
    let mut remote_checkout = checkout(
        workspace_id,
        checkout_id,
        "/home/me/project",
        Some(pane(&device_pane, "/home/me/project")),
    );
    remote_checkout.tabs[0].id = Some(tab_id.to_owned());
    let mut remote_workspace = workspace(
        workspace_id,
        "Remote",
        "/home/me/project",
        vec![remote_checkout],
    );
    remote_workspace.remote_target_id = Some(target_id.to_owned());
    remote_workspace.device_id = target_id.to_owned();
    // The device's agent row, as the device's session lists it.
    let mut agent = runtime.snapshot.navigator.agents[0].clone();
    agent.pane_id = device_pane.clone();
    let mut session = RemoteSessionSnapshot {
        workspaces: vec![remote_workspace],
        agents: vec![agent],
        active_tab_ids: Default::default(),
        focused_workspace_id: None,
        focused_checkout_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        pane_layouts: Vec::new(),
        pane_hook_tokens: Default::default(),
        issue_tokens: Default::default(),
        issue_links: Default::default(),
    };
    runtime.snapshot.status.remote.push(RemoteStatusSnapshot {
        target_id: target_id.to_owned(),
        state: "connected".to_owned(),
        message: None,
        session: Some(session.clone()),
        files: RemoteFileListSnapshot::idle(),
        catalog: Default::default(),
        unreachable_since_unix_ms: None,
    });
    let mut device = crate::workspace::local_device(&crate::node::test_node(), None, None);
    device.id = target_id.into();
    device.kind = "remote".into();
    runtime.snapshot.navigator.devices.push(device);
    runtime.install_remote_control(RemoteControlContext::new(
        target_id,
        FakeEngine::absent().engine(),
        Weak::new(),
        ChangeNotifier::noop(),
    ));
    let cost = DetailItem::Cost {
        workspace_id: workspace_id.into(),
    };
    let kept = |runtime: &Runtime| {
        runtime
            .snapshot
            .ui_state
            .detail_panes
            .0
            .iter()
            .map(|held| (held.pane.clone(), held.slot.items.clone()))
            .collect::<Vec<_>>()
    };

    open_at(&mut runtime, &cost, serde_json::json!({"pane": device_pane}));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "this machine's pane keeps its own slot until the device answers"
    );
    assert_eq!(kept(&runtime), [(device_pane.clone(), vec![cost.clone()])]);

    // Offline, the device says nothing about its panes: the slot stays.
    runtime.snapshot.status.remote[0].session = None;
    assert!(!runtime.prune_detail_panes());
    // Its session no longer lists the pane: the slot goes.
    let mut closed = session.clone();
    closed.workspaces[0].checkouts[0].tabs[0].panes.clear();
    runtime.snapshot.status.remote[0].session = Some(closed);
    assert!(runtime.prune_detail_panes());
    assert!(kept(&runtime).is_empty());

    runtime.snapshot.status.remote[0].session = Some(session.clone());
    open_at(&mut runtime, &cost, serde_json::json!({"pane": device_pane}));
    // The device brings the pane forward.
    session.focused_workspace_id = Some(workspace_id.to_owned());
    session.focused_checkout_id = Some(checkout_id.to_owned());
    session.focused_tab_id = Some(tab_id.to_owned());
    session.focused_pane_id = Some(device_pane.clone());
    runtime.snapshot.status.remote[0].session = Some(session);
    if !runtime.device_in_front(target_id) {
        runtime.bring_device_forward(target_id.to_owned());
    }
    runtime.sync_detail_pane();
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert_eq!(detail_pane(&runtime), Some(device_pane.as_str()));
}
