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
    assert!(event(&mut runtime, "practice_invite_close", serde_json::json!({})));
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
            home: false,
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
