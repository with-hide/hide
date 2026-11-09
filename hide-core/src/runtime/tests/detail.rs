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

/// The first-run chapter counts its start and its end once each per
/// installation: later steps, a replay that keeps it finished, and a chapter
/// the statistics do not name (the flows that were removed included) send nothing.
#[test]
fn a_practice_chapter_counts_its_start_and_end_once() {
    use crate::product_analytics::{Event, PracticeChapter, PracticeStage};
    let practice = |stage| Event::Practice {
        chapter: PracticeChapter::Overview,
        stage,
    };
    let mut runtime = runtime();
    let statistics = Arc::new(RecordedStatistics::default());
    runtime.install_product_analytics(statistics.clone());
    assert!(practice_event(&mut runtime, "overview", 0, false));
    assert!(practice_event(&mut runtime, "overview", 4, false));
    assert_eq!(statistics.take(), [practice(PracticeStage::Started)]);
    assert!(practice_event(&mut runtime, "overview", 6, true));
    assert!(practice_event(&mut runtime, "overview", 2, true));
    assert_eq!(statistics.take(), [practice(PracticeStage::Finished)]);
    for removed in ["mission", "agents", "devices", "day", "first-agent", "plan"] {
        assert!(practice_event(&mut runtime, removed, 9, true));
    }
    assert_eq!(statistics.take(), []);
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

/// The slot belongs to the Agent tab it was opened in (operator decision
/// 2026-11-08): another tab in front shows its own slot or none, coming back
/// finds the card as it was, and a closed tab takes its slot with it.
#[test]
fn the_detail_slot_stays_with_the_agent_tab_it_was_opened_in() {
    let checkout_path = "/private/tmp/hide-slot-per-tab";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2", "w-order:t3"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
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
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t1")
    );

    // Another tab in front: no card of its own, so none shows; the first
    // tab's card is kept for it.
    front(&mut runtime, "w-order:t2");
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the other tab's card never follows"
    );
    assert!(
        !event(
            &mut runtime,
            "detail_show",
            serde_json::json!({"shown": true})
        ),
        "the rail's 보기 has nothing of this tab's to bring forward"
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

    // Back in the first tab its card is there as it was; a click on the
    // tab's own pane keeps it.
    front(&mut runtime, "w-order:t1");
    front(&mut runtime, "w-order:t1");
    let slot = &runtime.snapshot.ui_state.detail;
    assert_eq!(
        (slot.items.as_slice(), slot.shown),
        ([cost.clone()].as_slice(), true)
    );
    front(&mut runtime, "w-order:t2");
    assert_eq!(runtime.snapshot.ui_state.detail.items, [worktrees]);

    // A tab that closes takes its slot, and nothing is re-homed to another.
    let remaining = ["w-order:t1", "w-order:t3"];
    let mut closed = tab_order_payload(checkout_path, &remaining, &remaining, "w-order:t1");
    closed.focused_pane_id = None;
    runtime.ingest_session(Ok(closed));
    front(&mut runtime, "w-order:t1");
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert!(
        runtime.snapshot.ui_state.detail_tabs.is_empty(),
        "the closed tab's slot is gone: {:?}",
        runtime.snapshot.ui_state.detail_tabs
    );
}

/// A slot an older build kept window-wide attaches to the tab in front when
/// the first event arrives, and stays there.
#[test]
fn a_window_wide_detail_slot_from_an_older_build_attaches_to_the_front_tab() {
    let checkout_path = "/private/tmp/hide-slot-per-tab-old";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let cost = DetailItem::Cost {
        workspace_id: "workspace:order".into(),
    };
    // As a launch loads it: a slot, and no tab it belongs to yet.
    runtime.snapshot.ui_state.detail.open(cost.clone());
    assert_eq!(runtime.snapshot.ui_state.detail_home, None);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t2",
    )));

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    assert_eq!(runtime.snapshot.ui_state.detail.items, [cost]);
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t2")
    );
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default()
    );
}

/// A device's tab is judged closed only against that device's own session:
/// before it arrives, or while the device is offline, its slot is kept
/// through events on this machine.
#[test]
fn a_device_tab_slot_is_kept_until_that_devices_session_says_the_tab_closed() {
    let checkout_path = "/private/tmp/hide-slot-per-tab-device";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    let mut kept = crate::model::DetailSlot::default();
    kept.open(DetailItem::Cost {
        workspace_id: "remote:mini:project:p".into(),
    });
    let device_tab = "remote:mini:tab:w9:t1";
    runtime
        .snapshot
        .ui_state
        .detail_tabs
        .insert(device_tab.to_owned(), kept.clone());
    runtime
        .snapshot
        .ui_state
        .detail_tabs
        .insert("w-order:t9".to_owned(), kept.clone());

    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t2:p")));
    assert_eq!(
        runtime.snapshot.ui_state.detail_tabs.get(device_tab),
        Some(&kept),
        "no session of mini is here to say its tab closed"
    );
    assert!(
        !runtime
            .snapshot
            .ui_state
            .detail_tabs
            .contains_key("w-order:t9"),
        "this machine's own catalog lists no such tab"
    );
}

/// Operator decision 2026-11-09: a record pressed outside the slot opens in
/// the tab of the session it records, which comes forward with the
/// session's pane, so the card stays with that agent; a session with no pane
/// here opens in the tab in front, and a record opened inside the slot (a
/// child's, with no `in_owner_tab`) stays in the slot it was pressed in.
#[test]
fn a_record_opens_in_its_own_sessions_tab() {
    let checkout_path = "/private/tmp/hide-slot-owner-tab";
    let (mut runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let tabs = ["w-order:t1", "w-order:t2"];
    let mut payload = tab_order_payload(checkout_path, &tabs, &tabs, "w-order:t1");
    let owned: SessionSnapshotPayload =
        crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
            "id": "worker", "pane_id": "w-order:t2:p", "agent": "claude",
            "agent_status": "working", "state_change_seq": 1, "cwd": checkout_path,
            "agent_session": {"kind": "id", "value": "11111111-2222-3333-4444-555555555555"}
        }]}))
        .unwrap();
    payload.agents.extend(owned.agents);
    runtime.ingest_session(Ok(payload));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    let session = crate::links::ledger::SessionIdentity {
        device_id: runtime.node.as_str().to_owned(),
        agent: "claude".into(),
        session_id: "11111111-2222-3333-4444-555555555555".into(),
    };
    let record = |session: &crate::links::ledger::SessionIdentity| DetailItem::Record {
        session: session.clone(),
    };
    let open = |runtime: &mut Runtime, item: &DetailItem, in_owner_tab: bool| {
        assert!(event(
            runtime,
            "detail_open",
            serde_json::json!({"item": item, "in_owner_tab": in_owner_tab})
        ));
    };

    open(&mut runtime, &record(&session), true);
    let state = &runtime.snapshot.ui_state;
    assert_eq!(state.detail_home.as_deref(), Some("w-order:t2"));
    assert_eq!(state.detail.items, [record(&session)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p"),
        "the session's pane takes the keyboard"
    );
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
    assert_eq!(
        runtime.snapshot.ui_state.detail,
        crate::model::DetailSlot::default(),
        "the tab that was in front holds no card"
    );

    // No pane runs this session here: the tab in front takes it.
    let gone = crate::links::ledger::SessionIdentity {
        session_id: "ended".into(),
        ..session.clone()
    };
    open(&mut runtime, &record(&gone), true);
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [record(&gone)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t1:p")
    );

    // Opened inside the slot, the record stays where it was pressed.
    open(&mut runtime, &record(&session), false);
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t1")
    );
    assert_eq!(
        runtime.snapshot.ui_state.detail.items,
        [record(&gone), record(&session)]
    );
}

/// Operator decision 2026-11-09: a pull request's card opens in the tab of
/// the agent that made it, which comes forward with its pane; a pull request
/// whose agent has no pane here opens in the tab in front.
#[test]
fn a_pull_request_card_opens_in_its_agents_tab() {
    let checkout_path = "/private/tmp/hide-slot-owner-pr";
    let (runtime, _checkout_id) = live_tab_order_runtime(checkout_path);
    let mut runtime = super::workspace_view::with_new_views(runtime, "slot-owner-pr");
    let tabs = ["w-order:t1", "w-order:t2"];
    runtime.ingest_session(Ok(tab_order_payload(
        checkout_path,
        &tabs,
        &tabs,
        "w-order:t1",
    )));
    assert!(runtime.dispatch_json(&operator_focus_event("w-order:t1:p")));
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
    project.pull_requests = vec![pull(42), pull(43)];
    project.agent_scope.prs.rows = vec![row(42, "w-order:t2:p"), row(43, "gone")];
    let open = |runtime: &mut Runtime, number: u32| {
        assert!(event(
            runtime,
            "pr_open",
            serde_json::json!({"workspace_id": "workspace:order", "pr_number": number})
        ));
        assert_eq!(runtime.snapshot.status.last_error, None);
    };
    let card = |number: u32| DetailItem::PullRequest {
        workspace_id: "workspace:order".into(),
        number,
    };

    open(&mut runtime, 42);
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [card(42)]);
    assert_eq!(
        runtime.snapshot.focused.pane_id.as_deref(),
        Some("w-order:t2:p")
    );

    // Its agent has no pane here: the tab in front takes the card.
    open(&mut runtime, 43);
    assert_eq!(
        runtime.snapshot.ui_state.detail_home.as_deref(),
        Some("w-order:t2")
    );
    assert_eq!(runtime.snapshot.ui_state.detail.items, [card(42), card(43)]);
}
