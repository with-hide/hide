use super::workspace_view::{views_path, with_new_views, with_views};
use super::*;
use crate::model::AgentLayoutSnapshot;
use crate::split_tree::Node;
use serde_json::{Value, json};
use std::fs;

// Expected outcomes come from group-tabs B2/B3/B14-B17/B21/B22/B24/B29.
// These tests dispatch the same intents as the shell and observe its snapshot.
const PATH: &str = "/named-group-tabs";
const TABS: [&str; 3] = ["w-order:t1", "w-order:t2", "w-order:t3"];

#[test]
fn a_global_start_can_open_another_checkout_until_a_selection_supersedes_it() {
    let (mut runtime, _) = setup();
    let task = runtime
        .begin_task_operation("agent_start", None, None, None, None)
        .unwrap();
    runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, "/another-checkout"),
            None,
            Some("global"),
            None,
            false,
        )
        .unwrap();
    assert!(runtime.group_start_should_focus(task));
    group(&mut runtime, "leave-origin", json!({"action":"new_tab"}));
    assert!(!runtime.group_start_should_focus(task));
}

#[test]
fn a_global_start_in_the_front_checkout_keeps_its_navigation_through_snapshot_sync() {
    for stays_behind in [false, true] {
        let (mut runtime, _) = setup();
        let task = runtime
            .begin_task_operation("agent_start", None, None, None, None)
            .unwrap();
        runtime
            .snapshot
            .task_operation
            .as_mut()
            .unwrap()
            .stays_behind = stays_behind;
        runtime
            .prepare_group_start(
                task,
                (crate::node::TEST_NODE, PATH),
                None,
                Some("global-front-checkout"),
                None,
                false,
            )
            .unwrap();
        // Dispatch and snapshot publication synchronize the old pane with
        // the chosen canvas while the real start's answer is still pending.
        runtime.sync_workspace_view();
        ingest(&mut runtime, &TABS);
        assert_eq!(geometry(&published(&runtime).root)["active"], TABS[0]);
        assert_eq!(runtime.group_start_should_focus(task), !stays_behind);
        runtime.ingest_task_operation_result(
            task,
            Ok(live::WorktreeTaskOutcome {
                path: PATH.into(),
                pane_id: "w-order:t4:p".into(),
                created_tab_id: Some("w-order:t4".into()),
                purpose_error: None,
                unconfirmed_purpose_token: None,
                issue_error: None,
                shell: crate::agent_start::ShellStart::Plain,
            }),
        );
        runtime.sync_workspace_view();
        assert_eq!(
            geometry(&published(&runtime).root)["active"],
            if stays_behind { TABS[0] } else { "w-order:t4" }
        );
    }
}

#[test]
fn a_settled_global_start_reuses_its_tab_after_a_group_move() {
    settled_global_start_after_group_change(false);
}

#[test]
fn a_settled_global_start_reuses_its_tab_after_its_group_is_dissolved() {
    settled_global_start_after_group_change(true);
}

fn settled_global_start_after_group_change(dissolve: bool) {
    let (mut runtime, checkout) = setup();
    let original = create(&mut runtime, "original", "Original");
    let task = runtime
        .begin_task_operation("agent_start", None, None, None, None)
        .unwrap();
    let first = runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, PATH),
            None,
            Some("settled-global"),
            None,
            false,
        )
        .unwrap();
    let outcome = || live::WorktreeTaskOutcome {
        path: PATH.into(),
        pane_id: "w-order:t4:p".into(),
        created_tab_id: Some("w-order:t4".into()),
        purpose_error: None,
        unconfirmed_purpose_token: None,
        issue_error: None,
        shell: crate::agent_start::ShellStart::Plain,
    };
    runtime.ingest_task_operation_result(task, Ok(outcome()));
    ingest(&mut runtime, &[TABS[0], TABS[1], TABS[2], "w-order:t4"]);
    let destination = if dissolve {
        group(
            &mut runtime,
            "dissolve-original",
            json!({"action":"dissolve","group_id":original}),
        );
        None
    } else {
        let destination = create(&mut runtime, "destination", "Destination");
        group(
            &mut runtime,
            "move-settled",
            json!({"action":"move","group_id":destination,"tab_id":"w-order:t4"}),
        );
        Some(destination)
    };
    assert!(runtime.snapshot().status.last_error.is_none());
    let before = ordered_tab_ids(&runtime, &checkout);
    let repeated_task = runtime
        .begin_task_operation("agent_start", None, None, None, None)
        .unwrap();
    let repeated = runtime
        .prepare_group_start(
            repeated_task,
            (crate::node::TEST_NODE, PATH),
            None,
            Some("settled-global"),
            None,
            false,
        )
        .unwrap();
    assert!(repeated.request.recovery_only);
    assert_eq!(repeated.request.marker, first.request.marker);
    assert_eq!(
        repeated.request.settled_tab_id.as_deref(),
        Some("w-order:t4")
    );
    assert!(runtime.group_start_should_focus(repeated_task));
    runtime.ingest_task_operation_result(repeated_task, Ok(outcome()));
    runtime.sync_workspace_view();
    assert_eq!(published(&runtime).group_id, destination);
    assert_eq!(geometry(&published(&runtime).root)["active"], "w-order:t4");
    assert_eq!(ordered_tab_ids(&runtime, &checkout), before);
    assert!(published(&runtime).drafts.is_empty());
    assert_eq!(
        runtime
            .snapshot
            .task_operation
            .as_ref()
            .unwrap()
            .pane_id
            .as_deref(),
        Some("w-order:t4:p")
    );
}

#[test]
fn a_global_start_stays_behind_after_an_empty_selection_round_trip() {
    for draft in [false, true] {
        let (mut runtime, _) = setup();
        let other = create(&mut runtime, "other-group", "Other");
        let origin = create(&mut runtime, "origin-group", "Origin");
        if draft {
            group(
                &mut runtime,
                "origin-draft",
                json!({"action":"new_tab", "group_id":origin}),
            );
        }
        let task = runtime
            .begin_task_operation("agent_start", None, None, None, None)
            .unwrap();
        runtime
            .prepare_group_start(
                task,
                (crate::node::TEST_NODE, "/another-checkout"),
                None,
                Some("global"),
                None,
                false,
            )
            .unwrap();
        assert!(runtime.group_start_should_focus(task));
        select(&mut runtime, "visit-other", Some(&other));
        select(&mut runtime, "return-origin", Some(&origin));
        assert!(
            !runtime.group_start_should_focus(task),
            "an intervening selection stays newer after returning to the original empty screen"
        );
    }
}

fn setup() -> (Runtime, String) {
    let (runtime, checkout) = tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "named-group-tabs");
    ingest(&mut runtime, &TABS);
    (runtime, checkout)
}

fn ingest(runtime: &mut Runtime, tabs: &[&str]) {
    runtime.ingest_session(Ok(tab_order_payload(PATH, tabs, tabs, "w-order:t1")));
    runtime.sync_workspace_view();
}

fn dispatch(runtime: &mut Runtime, event: &str, mut payload: Value) {
    payload["workspace"] = json!({"device_id": crate::node::TEST_NODE, "path": PATH});
    runtime.dispatch_json(&explorer_event(event, payload));
    runtime.sync_workspace_view();
}

fn group(runtime: &mut Runtime, request: &str, mut payload: Value) {
    payload["request_id"] = json!(request);
    dispatch(runtime, "agent_group", payload);
}

fn create(runtime: &mut Runtime, request: &str, name: &str) -> String {
    group(runtime, request, json!({"action":"create", "name":name}));
    published(runtime)
        .group_id
        .clone()
        .expect("new group selected")
}

fn select(runtime: &mut Runtime, request: &str, id: Option<&str>) {
    group(runtime, request, json!({"action":"select", "group_id":id}));
}

fn published(runtime: &Runtime) -> &AgentLayoutSnapshot {
    runtime
        .snapshot()
        .workspace_view
        .as_ref()
        .unwrap()
        .agent_layout
        .as_ref()
        .unwrap()
}

fn tabs(runtime: &Runtime, id: Option<&str>) -> Vec<String> {
    published(runtime)
        .groups
        .iter()
        .find(|g| g.id.as_deref() == id)
        .unwrap()
        .tabs
        .clone()
}

// Focus timestamps may change on a visit; the user-visible geometry must not.
fn geometry(node: &Node<crate::agent_layout::Tab>) -> Value {
    match node {
        Node::Area(area) => json!({
            "id":area.id, "active":area.active,
            "tabs":area.displays.iter().map(|tab| &tab.id).collect::<Vec<_>>()
        }),
        Node::Split(split) => json!({
            "id":split.id, "axis":split.axis, "ratio":split.ratio,
            "first":geometry(&split.first), "second":geometry(&split.second)
        }),
    }
}

#[test]
fn a_created_terminal_keeps_its_group_before_or_after_the_catalog() {
    for opens_owner in [false, true] {
        for catalog_first in [false, true] {
            created_terminal_group(opens_owner, catalog_first, false);
        }
    }
}

#[test]
fn a_created_terminal_preserves_a_group_chosen_while_its_answer_was_pending() {
    for opens_owner in [false, true] {
        for catalog_first in [false, true] {
            created_terminal_group(opens_owner, catalog_first, true);
        }
    }
}

#[test]
fn an_external_terminal_keeps_the_selected_group_and_joins_ungrouped() {
    let (mut runtime, _) = setup();
    let selected = create(&mut runtime, "selected", "My work");
    let all = [TABS[0], TABS[1], TABS[2], "w-order:t4"];
    let mut catalog = tab_order_payload(PATH, &all, &all, "w-order:t4");
    catalog.tab_focus = Some(crate::sidebar::SessionTabFocus {
        generation: 1,
        workspace_id: "w-order".into(),
        tab_id: "w-order:t4".into(),
        revision: 1,
        creation: true,
    });
    runtime.ingest_session(Ok(catalog));
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(selected.as_str())
    );
    assert_eq!(tabs(&runtime, None), all);
    assert!(runtime.shown_agent_tabs().is_empty());
    assert!(runtime.snapshot().terminal.pane_id.is_none());
}

#[test]
fn an_unanswered_catalog_cannot_prune_groups_on_a_creation_answer() {
    let (mut runtime, _) = setup();
    let selected = create(&mut runtime, "selected", "My work");
    runtime.catalog_paths = Default::default();
    runtime.begin_catalog_coordinator();
    runtime.ingest_local_control_result(
        RemoteControlAction::CreateTab {
            workspace_id: "w-order".into(),
            cwd: PATH.into(),
            label: "New terminal".into(),
            area_id: Some("a1".into()),
            group_id: Some(selected.clone().into_boxed_str()),
            admission_id: None,
        },
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some("w-order:t4".into()),
            created_pane_id: Some("w-order:t4:p".into()),
        }),
        1,
    );
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(selected.as_str())
    );
    assert_eq!(tabs(&runtime, None), TABS);
}

fn created_terminal_group(opens_owner: bool, catalog_first: bool, choose_another: bool) {
    let (mut runtime, _) = setup();
    let destination = create(&mut runtime, "destination", "Terminal work");
    let other = create(&mut runtime, "other", "Other work");
    select(&mut runtime, "choose-destination", Some(&destination));
    let area = published(&runtime).active_area.clone();
    let action = if opens_owner {
        RemoteControlAction::OpenOwner {
            owner: crate::checkout_owner::OwnerOpen::for_checkout(
                crate::node::TEST_NODE,
                PATH,
                PATH,
                false,
                "Terminal work",
            ),
            cwd: PATH.into(),
            label: "New terminal".into(),
            area_id: Some(area.clone()),
            group_id: Some(destination.clone().into_boxed_str()),
            admission_id: None,
        }
    } else {
        RemoteControlAction::CreateTab {
            workspace_id: "w-order".into(),
            cwd: PATH.into(),
            label: "New terminal".into(),
            area_id: Some(area.clone()),
            group_id: Some(destination.clone().into_boxed_str()),
            admission_id: None,
        }
    };
    let created = if opens_owner {
        "w-new:t1"
    } else {
        "w-order:t4"
    };
    let all = [TABS[0], TABS[1], TABS[2], created];
    let mut catalog = if opens_owner {
        split_checkout_payload(
            PATH,
            &[("w-order", &TABS, TABS[0]), ("w-new", &[created], created)],
            "w-new",
        )
    } else {
        tab_order_payload(PATH, &all, &all, created)
    };
    catalog.tab_focus = Some(crate::sidebar::SessionTabFocus {
        generation: 1,
        workspace_id: if opens_owner { "w-new" } else { "w-order" }.into(),
        tab_id: created.into(),
        revision: 1,
        creation: true,
    });
    if catalog_first {
        runtime.ingest_session(Ok(catalog.clone()));
        runtime.sync_workspace_view();
    }
    if choose_another {
        select(&mut runtime, "choose-other-while-pending", Some(&other));
    }
    runtime.ingest_local_control_result(
        action,
        Ok(RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(created.into()),
            created_pane_id: Some(format!("{created}:p")),
        }),
        1,
    );
    runtime.sync_workspace_view();
    let selected = if choose_another { &other } else { &destination };
    assert_eq!(
        published(&runtime).group_id.as_ref(),
        Some(selected),
        "creation answer: owner={opens_owner}, catalog_first={catalog_first}"
    );
    if catalog_first {
        assert_eq!(tabs(&runtime, Some(&destination)), [created]);
        assert_eq!(tabs(&runtime, None), TABS);
    }
    runtime.ingest_session(Ok(catalog));
    runtime.sync_workspace_view();
    assert_eq!(published(&runtime).group_id.as_ref(), Some(selected));
    assert_eq!(tabs(&runtime, Some(&destination)), [created]);
    assert_eq!(tabs(&runtime, None), TABS);
    if choose_another {
        assert!(runtime.shown_agent_tabs().is_empty());
        assert!(runtime.snapshot().terminal.pane_id.is_none());
    } else {
        assert_eq!(published(&runtime).active_area, area);
        assert_eq!(geometry(&published(&runtime).root)["active"], created);
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some(format!("{created}:p").as_str())
        );
    }
}

#[test]
fn empty_group_preserves_the_old_split_and_shows_no_hidden_terminal() {
    let (mut runtime, checkout) = setup();
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split", "tab_id":TABS[1], "area_id":"a1",
            "edge":"right", "request_id":"old-split"
        }),
    );
    let split = match &published(&runtime).root {
        Node::Split(split) => split.id.clone(),
        _ => panic!("split is visible"),
    };
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({"action":"resize", "split_id":split,"ratio":0.35}),
    );
    let original_geometry = geometry(&published(&runtime).root);
    let original_area = published(&runtime).active_area.clone();
    let live_tabs = ordered_tab_ids(&runtime, &checkout);
    let empty = create(&mut runtime, "empty", "  문서 Draft  ");

    assert_eq!(
        published(&runtime).groups[1].name.as_deref(),
        Some("문서 Draft")
    );
    assert!(tabs(&runtime, Some(&empty)).is_empty());
    assert!(
        runtime.shown_agent_tabs().is_empty(),
        "an empty group must not fall back to a hidden live tab"
    );
    assert!(runtime.snapshot().terminal.pane_id.is_none());
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout),
        live_tabs,
        "creating a group creates no topology"
    );
    ingest(&mut runtime, &TABS);
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(empty.as_str())
    );
    assert!(
        runtime.shown_agent_tabs().is_empty(),
        "an unchanged authoritative focus must not leave the empty group"
    );

    select(&mut runtime, "back", None);
    assert_eq!(geometry(&published(&runtime).root), original_geometry);
    assert_eq!(published(&runtime).active_area, original_area);
    assert_eq!(ordered_tab_ids(&runtime, &checkout), live_tabs);
}

#[test]
fn moving_a_tab_preserves_its_pane_and_collapses_only_the_empty_source_area() {
    let (mut runtime, checkout) = setup();
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split", "tab_id":TABS[1], "area_id":"a1",
            "edge":"down", "request_id":"source-split"
        }),
    );
    let original_tabs = ordered_tab_ids(&runtime, &checkout);
    let target = create(&mut runtime, "target", "Implementation");
    group(
        &mut runtime,
        "move-second",
        json!({"action":"move", "group_id":target,"tab_id":TABS[1]}),
    );
    assert_eq!(tabs(&runtime, Some(&target)), [TABS[1]]);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), original_tabs);

    select(&mut runtime, "source", None);
    assert!(
        matches!(&published(&runtime).root, Node::Area(_)),
        "the empty non-root area collapses"
    );
    assert_eq!(tabs(&runtime, None), [TABS[0], TABS[2]]);
    group(
        &mut runtime,
        "move-first",
        json!({"action":"move", "group_id":target,"tab_id":TABS[0]}),
    );
    assert_eq!(
        tabs(&runtime, Some(&target)),
        [TABS[1], TABS[0]],
        "a move appends in the destination"
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t1:p")
    );

    group(
        &mut runtime,
        "remove-second",
        json!({"action":"move","tab_id":TABS[1]}),
    );
    group(
        &mut runtime,
        "remove-first",
        json!({"action":"move","tab_id":TABS[0]}),
    );
    select(&mut runtime, "empty-target", Some(&target));
    assert!(tabs(&runtime, Some(&target)).is_empty());
    assert!(matches!(&published(&runtime).root, Node::Area(_)));
    assert!(runtime.shown_agent_tabs().is_empty());
    assert_eq!(ordered_tab_ids(&runtime, &checkout), original_tabs);
}

#[test]
fn dissolve_appends_in_order_and_undo_expires_after_the_next_group_mutation() {
    let (mut runtime, checkout) = setup();
    let original_tabs = ordered_tab_ids(&runtime, &checkout);
    let target = create(&mut runtime, "target", "Review");
    for (request, tab) in [("move-two", TABS[1]), ("move-three", TABS[2])] {
        group(
            &mut runtime,
            request,
            json!({"action":"move", "group_id":target,"tab_id":tab}),
        );
    }
    let grouped = geometry(&published(&runtime).root);
    group(
        &mut runtime,
        "dissolve",
        json!({"action":"dissolve", "group_id":target}),
    );
    assert_eq!(tabs(&runtime, None), TABS);
    assert!(published(&runtime).can_undo_group);
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout),
        original_tabs,
        "dissolve does not close sessions"
    );
    group(&mut runtime, "undo", json!({"action":"undo"}));
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(geometry(&published(&runtime).root), grouped);
    assert_eq!(tabs(&runtime, None), [TABS[0]]);
    assert!(!published(&runtime).can_undo_group);

    group(
        &mut runtime,
        "dissolve-again",
        json!({"action":"dissolve", "group_id":target}),
    );
    let next = create(&mut runtime, "next", "Next");
    assert!(!published(&runtime).can_undo_group);
    group(&mut runtime, "expired-undo", json!({"action":"undo"}));
    assert_eq!(published(&runtime).group_id.as_deref(), Some(next.as_str()));
    assert_eq!(tabs(&runtime, None), TABS);
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("agent_group.refused")
    );
}

#[test]
fn names_and_group_limit_refuse_without_replacing_existing_work() {
    let (mut runtime, _) = setup();
    let longest = "한".repeat(80);
    let first = create(&mut runtime, "eighty", &longest);
    assert_eq!(
        published(&runtime).groups[1].name.as_deref(),
        Some(longest.as_str())
    );
    for (index, name) in [
        " ".to_owned(),
        "한".repeat(81),
        "one\ntwo".into(),
        "one\u{0007}two".into(),
        "one\u{2028}two".into(),
        "one\u{2029}two".into(),
        longest,
    ]
    .into_iter()
    .enumerate()
    {
        group(
            &mut runtime,
            &format!("bad-name-{index}"),
            json!({"action":"create", "name":name}),
        );
        assert_eq!(
            published(&runtime).groups.len(),
            2,
            "invalid names cannot create a group"
        );
        assert_eq!(
            published(&runtime).group_id.as_deref(),
            Some(first.as_str())
        );
        assert_eq!(
            runtime
                .snapshot()
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("agent_group.refused")
        );
    }
    for index in 1..32 {
        create(
            &mut runtime,
            &format!("group-{index}"),
            &format!("Group {index}"),
        );
    }
    let before = published(&runtime).groups.clone();
    group(
        &mut runtime,
        "thirty-three",
        json!({"action":"create", "name":"Overflow"}),
    );
    assert_eq!(published(&runtime).groups, before);
    assert_eq!(tabs(&runtime, None), TABS);
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("agent_group.refused")
    );
}

#[test]
fn all_groups_share_the_admission_budget_with_pending_starts() {
    let (mut runtime, checkout) = setup();
    let ids: Vec<_> = (1..=62).map(|n| format!("w-order:t{n}")).collect();
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let target = create(&mut runtime, "target", "Drafts");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab", "group_id":target}),
    );
    assert_eq!(tabs(&runtime, Some(&target)).len(), 1);
    // A request already admitted but not present in topology reserves slot 64.
    runtime
        .workspace_views
        .as_mut()
        .unwrap()
        .agent_admissions
        .insert(
            (crate::node::TEST_NODE.into(), PATH.into()),
            HashSet::from(["create:1".into()]),
        );
    group(
        &mut runtime,
        "over-budget",
        json!({"action":"new_tab", "group_id":target}),
    );
    assert_eq!(tabs(&runtime, Some(&target)).len(), 1);
    assert_eq!(ordered_tab_ids(&runtime, &checkout).len(), 62);
    assert!(runtime.snapshot().status.last_error.is_some());
    runtime.finish_agent_admission(PATH, 1);
    group(
        &mut runtime,
        "last-slot",
        json!({"action":"new_tab", "group_id":target}),
    );
    assert_eq!(tabs(&runtime, Some(&target)).len(), 2);
    group(
        &mut runtime,
        "sixty-five",
        json!({"action":"new_tab", "group_id":target}),
    );
    assert_eq!(tabs(&runtime, Some(&target)).len(), 2);
    assert_eq!(tabs(&runtime, None).len(), 62);
}

#[test]
fn drafts_and_empty_groups_survive_authoritative_topology_and_restart() {
    let (_folder, state) = views_path("group-draft-restart");
    let (runtime, checkout) = tab_order_runtime(PATH);
    let mut runtime = with_views(runtime, &state);
    ingest(&mut runtime, &TABS);
    let empty = create(&mut runtime, "empty", "Empty");
    let draft_group = create(&mut runtime, "draft-group", "Draft");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab", "group_id":draft_group}),
    );
    let draft = tabs(&runtime, Some(&draft_group))[0].clone();
    ingest(&mut runtime, &TABS);
    assert_eq!(
        tabs(&runtime, Some(&draft_group)),
        std::slice::from_ref(&draft)
    );
    assert!(tabs(&runtime, Some(&empty)).is_empty());
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(draft_group.as_str())
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
    drop(runtime);

    let (runtime, checkout) = tab_order_runtime(PATH);
    let mut runtime = with_views(runtime, &state);
    ingest(&mut runtime, &TABS);
    assert_eq!(
        published(&runtime)
            .groups
            .iter()
            .filter_map(|group| group.name.as_deref())
            .collect::<Vec<_>>(),
        ["Empty", "Draft"]
    );
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(draft_group.as_str())
    );
    assert_eq!(
        tabs(&runtime, Some(&draft_group)),
        std::slice::from_ref(&draft)
    );
    assert!(published(&runtime).drafts.contains(&draft));
    assert!(
        runtime.snapshot().terminal.pane_id.is_none(),
        "restoring a draft must not attach an old terminal"
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

#[test]
fn existing_focus_event_reveals_a_tab_in_another_group() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Code");
    group(
        &mut runtime,
        "move",
        json!({"action":"move", "group_id":target,"tab_id":TABS[1]}),
    );
    select(&mut runtime, "ungrouped", None);
    runtime.dispatch_json(&focus_tab_event(&checkout, TABS[1]));
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some(TABS[1])
    );
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
}

#[test]
fn stale_group_area_intent_cannot_split_another_group() {
    let (mut runtime, _) = setup();
    let first = create(&mut runtime, "first", "First");
    group(
        &mut runtime,
        "first-tab",
        json!({"action":"move", "group_id":first,"tab_id":TABS[1]}),
    );
    let second = create(&mut runtime, "second", "Second");
    group(
        &mut runtime,
        "second-tab",
        json!({"action":"move", "group_id":second,"tab_id":TABS[2]}),
    );
    group(
        &mut runtime,
        "second-tab-again",
        json!({"action":"move", "group_id":second,"tab_id":TABS[0]}),
    );
    let before = geometry(&published(&runtime).root);
    // Every group's root can be a1. A delayed intent must carry its group too.
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split", "group_id":first, "tab_id":TABS[2], "area_id":"a1",
            "edge":"right", "request_id":"stale-split"
        }),
    );
    assert_eq!(geometry(&published(&runtime).root), before);
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(second.as_str())
    );
    assert_eq!(
        runtime
            .snapshot()
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("agent_layout.stale_group")
    );
    group(
        &mut runtime,
        "stale-draft",
        json!({"action":"new_tab", "group_id":first,"area_id":"a1"}),
    );
    assert_eq!(tabs(&runtime, Some(&second)), [TABS[2], TABS[0]]);
    assert_eq!(tabs(&runtime, Some(&first)), [TABS[1]]);
}

#[test]
fn a_pre_group_workspace_file_restores_the_entire_ungrouped_tree() {
    let (_folder, state) = views_path("legacy-agent-tree");
    let (runtime, _) = tab_order_runtime(PATH);
    let mut runtime = with_views(runtime, &state);
    ingest(&mut runtime, &TABS);
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split", "tab_id":TABS[1], "area_id":"a1",
            "edge":"right", "request_id":"old-split"
        }),
    );
    let split = match &published(&runtime).root {
        Node::Split(split) => split.id.clone(),
        _ => panic!("split is visible"),
    };
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({"action":"resize","split_id":split,"ratio":0.7}),
    );
    let before = geometry(&published(&runtime).root);
    let active_area = published(&runtime).active_area.clone();
    drop(runtime);

    // The existing schema-3 format has agent_layout but no named collection.
    let mut legacy: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    assert_eq!(legacy["schema_version"], 4);
    for workspace in legacy["workspaces"].as_array_mut().unwrap() {
        workspace.as_object_mut().unwrap().remove("agent_groups");
    }
    fs::write(&state, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let (runtime, checkout) = tab_order_runtime(PATH);
    let mut runtime = with_views(runtime, &state);
    ingest(&mut runtime, &TABS);
    assert_eq!(published(&runtime).group_id, None);
    assert_eq!(published(&runtime).groups.len(), 1);
    assert_eq!(geometry(&published(&runtime).root), before);
    assert_eq!(published(&runtime).active_area, active_area);
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

#[test]
fn each_group_restores_its_own_split_ratio_selection_and_active_area() {
    let (mut runtime, _) = setup();
    let ids: Vec<_> = (1..=4).map(|n| format!("w-order:t{n}")).collect();
    ingest(
        &mut runtime,
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let first = create(&mut runtime, "first", "First");
    for (request, tab) in [("first-one", "w-order:t1"), ("first-two", "w-order:t2")] {
        group(
            &mut runtime,
            request,
            json!({"action":"move","group_id":first,"tab_id":tab}),
        );
    }
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split","group_id":first,"tab_id":"w-order:t2","area_id":"a1",
            "edge":"right","request_id":"first-split"
        }),
    );
    let first_geometry = geometry(&published(&runtime).root);
    let first_area = published(&runtime).active_area.clone();
    let second = create(&mut runtime, "second", "Second");
    for (request, tab) in [
        ("second-three", "w-order:t3"),
        ("second-four", "w-order:t4"),
    ] {
        group(
            &mut runtime,
            request,
            json!({"action":"move","group_id":second,"tab_id":tab}),
        );
    }
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({
            "action":"split","group_id":second,"tab_id":"w-order:t4","area_id":"a1",
            "edge":"down","request_id":"second-split"
        }),
    );
    let split = match &published(&runtime).root {
        Node::Split(split) => split.id.clone(),
        _ => panic!("second group split is visible"),
    };
    dispatch(
        &mut runtime,
        "agent_layout",
        json!({"action":"resize","group_id":second,"split_id":split,"ratio":0.65}),
    );
    let second_geometry = geometry(&published(&runtime).root);
    let second_area = published(&runtime).active_area.clone();
    select(&mut runtime, "first-back", Some(&first));
    assert_eq!(geometry(&published(&runtime).root), first_geometry);
    assert_eq!(published(&runtime).active_area, first_area);
    assert_eq!(runtime.shown_agent_tabs(), ["w-order:t1", "w-order:t2"]);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    select(&mut runtime, "second-back", Some(&second));
    assert_eq!(geometry(&published(&runtime).root), second_geometry);
    assert_eq!(published(&runtime).active_area, second_area);
    assert_eq!(runtime.shown_agent_tabs(), ["w-order:t3", "w-order:t4"]);
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t4:p")
    );
}

#[test]
fn a_closed_draft_returns_to_its_original_slot_without_creating_a_terminal() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Drafts");
    for request in ["draft-one", "draft-two", "draft-three"] {
        group(
            &mut runtime,
            request,
            json!({"action":"new_tab","group_id":target}),
        );
    }
    let original = tabs(&runtime, Some(&target));
    group(
        &mut runtime,
        "close-middle",
        json!({"action":"close_draft","group_id":target,"tab_id":original[1]}),
    );
    assert_eq!(
        tabs(&runtime, Some(&target)),
        [original[0].clone(), original[2].clone()]
    );
    assert!(runtime.snapshot().recent_closed.can_reopen);
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
    dispatch(&mut runtime, "reopen_closed", json!({}));
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(tabs(&runtime, Some(&target)), original);
    assert!(published(&runtime).drafts.contains(&original[1]));
    assert!(!runtime.snapshot().recent_closed.can_reopen);
    assert!(runtime.snapshot().terminal.pane_id.is_none());
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

#[test]
fn a_draft_closed_in_a_removed_group_reopens_at_the_end_of_ungrouped() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Temporary");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab","group_id":target}),
    );
    let draft = tabs(&runtime, Some(&target))[0].clone();
    group(
        &mut runtime,
        "close",
        json!({"action":"close_draft","group_id":target,"tab_id":draft}),
    );
    group(
        &mut runtime,
        "dissolve",
        json!({"action":"dissolve","group_id":target}),
    );
    dispatch(&mut runtime, "reopen_closed", json!({}));
    assert_eq!(published(&runtime).group_id, None);
    assert_eq!(
        tabs(&runtime, None),
        [TABS[0], TABS[1], TABS[2], draft.as_str()]
    );
    assert!(published(&runtime).drafts.contains(&draft));
    assert!(runtime.snapshot().terminal.pane_id.is_none());
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

#[test]
fn undoing_dissolve_preserves_external_tabs_arriving_after_the_dissolve() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Review");
    group(
        &mut runtime,
        "move",
        json!({"action":"move","group_id":target,"tab_id":TABS[1]}),
    );
    group(
        &mut runtime,
        "dissolve",
        json!({"action":"dissolve","group_id":target}),
    );
    let current = [TABS[0], TABS[1], TABS[2], "w-order:t4"];
    ingest(&mut runtime, &current);
    assert!(
        published(&runtime).can_undo_group,
        "topology arrival is not a group mutation"
    );
    group(&mut runtime, "undo", json!({"action":"undo"}));
    assert_eq!(tabs(&runtime, Some(&target)), [TABS[1]]);
    assert_eq!(tabs(&runtime, None), [TABS[0], TABS[2], "w-order:t4"]);
    assert_eq!(ordered_tab_ids(&runtime, &checkout), current);
}

#[test]
fn a_start_awaiting_confirmation_refuses_move_dissolve_and_draft_close() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Starting");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab","group_id":target}),
    );
    let draft = tabs(&runtime, Some(&target))[0].clone();
    let prepared = runtime
        .prepare_group_start(
            71,
            (crate::node::TEST_NODE, PATH),
            Some(super::super::group_starts::StartTarget {
                group_id: Some(target.clone()),
                area_id: "a1".into(),
                tab_id: Some(draft.clone()),
                add_pane: false,
            }),
            Some("pending-start"),
            None,
            false,
        )
        .expect("start reserved");
    assert_eq!(
        prepared
            .saved
            .try_recv()
            .expect("the write was acknowledged"),
        Ok(())
    );
    assert!(!prepared.request.recovery_only);
    for (request, payload) in [
        ("move-pending", json!({"action":"move","tab_id":draft})),
        (
            "dissolve-pending",
            json!({"action":"dissolve","group_id":target}),
        ),
        (
            "close-pending",
            json!({"action":"close_draft","group_id":target,"tab_id":draft}),
        ),
    ] {
        group(&mut runtime, request, payload);
        assert_eq!(tabs(&runtime, Some(&target)), std::slice::from_ref(&draft));
        assert_eq!(
            published(&runtime).group_id.as_deref(),
            Some(target.as_str())
        );
        assert!(runtime.snapshot().status.last_error.is_some());
        assert!(!runtime.snapshot().recent_closed.can_reopen);
    }
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

#[test]
fn a_late_start_fills_its_original_draft_once_without_changing_the_visible_group() {
    let (mut runtime, _) = setup();
    let target = create(&mut runtime, "target", "Starting");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab","group_id":target}),
    );
    let draft = tabs(&runtime, Some(&target))[0].clone();
    let destination = super::super::group_starts::StartTarget {
        group_id: Some(target.clone()),
        area_id: "a1".into(),
        tab_id: Some(draft.clone()),
        add_pane: false,
    };
    let initial = runtime
        .prepare_group_start(
            81,
            (crate::node::TEST_NODE, PATH),
            Some(destination.clone()),
            Some("late-start"),
            None,
            false,
        )
        .expect("initial start");
    assert!(!initial.request.recovery_only);
    assert_eq!(initial.saved.try_recv().expect("start persisted"), Ok(()));
    runtime.group_start_failed(81, true);
    let repeated = runtime
        .prepare_group_start(
            82,
            (crate::node::TEST_NODE, PATH),
            Some(destination.clone()),
            Some("late-start"),
            None,
            false,
        )
        .expect("recover lost response");
    assert!(
        repeated.request.recovery_only,
        "an uncertain effect is confirmed before another create"
    );
    assert_eq!(repeated.request.marker, initial.request.marker);
    assert_eq!(tabs(&runtime, Some(&target)), std::slice::from_ref(&draft));
    let elsewhere = create(&mut runtime, "elsewhere", "Elsewhere");
    assert!(!runtime.group_start_should_focus(82));
    // Herdr's authoritative topology may precede the effect's own answer.
    ingest(&mut runtime, &[TABS[0], TABS[1], TABS[2], "w-order:t4"]);
    runtime.complete_group_start(82, Some("w-order:t4"), "w-order:t4:p", false);
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(elsewhere.as_str())
    );
    assert_eq!(tabs(&runtime, Some(&target)), ["w-order:t4"]);
    assert_eq!(tabs(&runtime, None), TABS);
    assert!(!published(&runtime).drafts.contains(&draft));
    runtime.complete_group_start(82, Some("w-order:t4"), "w-order:t4:p", false);
    runtime.sync_workspace_view();
    assert_eq!(tabs(&runtime, Some(&target)), ["w-order:t4"]);
    let settled = runtime
        .prepare_group_start(
            83,
            (crate::node::TEST_NODE, PATH),
            Some(destination),
            Some("late-start"),
            None,
            false,
        )
        .expect("settled retry");
    assert!(settled.request.recovery_only);
    assert_eq!(
        settled.request.settled_tab_id.as_deref(),
        Some("w-order:t4")
    );
    assert_eq!(settled.request.marker, initial.request.marker);
}

#[test]
fn an_acknowledged_start_survives_stale_topology_then_closes_after_being_observed() {
    let (mut runtime, checkout) = setup();
    let target = create(&mut runtime, "target", "Starting");
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab","group_id":target}),
    );
    let draft = tabs(&runtime, Some(&target))[0].clone();
    let prepared = runtime
        .prepare_group_start(
            91,
            (crate::node::TEST_NODE, PATH),
            Some(super::super::group_starts::StartTarget {
                group_id: Some(target.clone()),
                area_id: "a1".into(),
                tab_id: Some(draft.clone()),
                add_pane: false,
            }),
            Some("ack-before-topology"),
            None,
            false,
        )
        .expect("start reserved");
    assert_eq!(prepared.saved.try_recv().expect("start persisted"), Ok(()));

    // The effect answers first. The session stream still contains the old tabs.
    runtime.complete_group_start(91, Some("w-order:t4"), "w-order:t4:p", false);
    runtime.sync_workspace_view();
    assert_eq!(tabs(&runtime, Some(&target)), ["w-order:t4"]);
    assert!(!published(&runtime).drafts.contains(&draft));
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str()),
        "acknowledgement retains its selected group"
    );
    ingest(&mut runtime, &TABS);
    assert_eq!(
        tabs(&runtime, Some(&target)),
        ["w-order:t4"],
        "the acknowledged slot waits for its first authoritative appearance"
    );
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str()),
        "stale topology retains the selected group"
    );
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout),
        TABS,
        "a retained placement does not invent a live catalog tab"
    );

    // Only actual topology confirms the live identity. A later omission then
    // means it closed, and the receipt must no longer keep a phantom tab alive.
    let live = [TABS[0], TABS[1], TABS[2], "w-order:t4"];
    ingest(&mut runtime, &live);
    assert_eq!(tabs(&runtime, Some(&target)), ["w-order:t4"]);
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str()),
        "first appearance retains the selected group"
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), live);
    ingest(&mut runtime, &TABS);
    assert!(tabs(&runtime, Some(&target)).is_empty());
    assert!(matches!(&published(&runtime).root, Node::Area(_)));
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), TABS);
}

fn first_owner_before_catalog(named: bool) -> Runtime {
    let (mut runtime, _) = setup();
    let target = named.then(|| create(&mut runtime, "target", "Starting"));
    if let Some(target) = &target {
        group(
            &mut runtime,
            "existing-in-target",
            json!({"action":"move","group_id":target,"tab_id":TABS[0]}),
        );
    }
    group(
        &mut runtime,
        "draft",
        json!({"action":"new_tab","group_id":target}),
    );
    let draft = tabs(&runtime, target.as_deref()).last().unwrap().clone();
    let task = runtime
        .begin_task_operation("checkout_start", None, None, None, None)
        .unwrap();
    let prepared = runtime
        .prepare_group_start(
            task,
            (crate::node::TEST_NODE, PATH),
            Some(super::super::group_starts::StartTarget {
                group_id: target.clone(),
                area_id: "a1".into(),
                tab_id: Some(draft),
                add_pane: false,
            }),
            Some("first-owner-before-catalog"),
            None,
            false,
        )
        .unwrap();
    assert_eq!(prepared.saved.try_recv().unwrap(), Ok(()));
    assert!(runtime.group_start_should_focus(task));
    assert!(runtime.owner_workspace_of_path(PATH).is_none());
    assert!(runtime.ingest_task_operation_result(
        task,
        Ok(live::WorktreeTaskOutcome {
            path: PATH.into(),
            pane_id: "w-new:t1:p".into(),
            created_tab_id: Some("w-new:t1".into()),
            purpose_error: None,
            unconfirmed_purpose_token: None,
            issue_error: None,
            shell: crate::agent_start::ShellStart::Plain,
        }),
    ));
    runtime.sync_workspace_view();
    assert_eq!(geometry(&published(&runtime).root)["active"], "w-new:t1");
    runtime
}

#[test]
fn a_first_owner_start_keeps_its_selected_slot_before_the_catalog_lists_it() {
    for named in [false, true] {
        let mut runtime = first_owner_before_catalog(named);
        ingest(&mut runtime, &TABS);
        assert_eq!(geometry(&published(&runtime).root)["active"], "w-new:t1");

        let mut creation = tab_order_payload(PATH, &TABS, &TABS, TABS[0]);
        creation.tab_focus = Some(crate::sidebar::SessionTabFocus {
            generation: 1,
            workspace_id: "w-order".into(),
            tab_id: TABS[0].into(),
            revision: 1,
            creation: true,
        });
        runtime.ingest_session(Ok(creation));
        runtime.sync_workspace_view();
        assert_eq!(geometry(&published(&runtime).root)["active"], "w-new:t1");

        let live = [TABS[0], TABS[1], TABS[2], "w-new:t1"];
        let payload = tab_order_payload(PATH, &live, &live, TABS[0]);
        runtime.ingest_session(Ok(payload));
        runtime.sync_workspace_view();
        assert_eq!(geometry(&published(&runtime).root)["active"], "w-new:t1");
        assert_eq!(
            runtime.snapshot().focused.pane_id.as_deref(),
            Some("w-new:t1:p")
        );
    }
}

#[test]
fn explicit_external_focus_can_replace_a_started_slot_before_its_catalog_arrives() {
    for named in [false, true] {
        let mut runtime = first_owner_before_catalog(named);
        let mut payload = tab_order_payload(PATH, &TABS, &TABS, TABS[0]);
        payload.tab_focus = Some(crate::sidebar::SessionTabFocus {
            generation: 1,
            workspace_id: "w-order".into(),
            tab_id: TABS[0].into(),
            revision: 1,
            creation: false,
        });
        runtime.ingest_session(Ok(payload));
        runtime.sync_workspace_view();
        assert_eq!(geometry(&published(&runtime).root)["active"], TABS[0]);
        assert_eq!(
            runtime.snapshot().focused.pane_id.as_deref(),
            Some("w-order:t1:p")
        );
    }
}

const CHILDREN: [&str; 2] = ["w-order:t4", "w-order:t5"];

fn with_delegated_canvases() -> (Runtime, String, String) {
    let (mut runtime, checkout) = setup();
    let ids = [TABS[0], TABS[1], TABS[2], CHILDREN[0], CHILDREN[1]];
    let mut payload = tab_order_payload(PATH, &ids, &ids, TABS[0]);
    payload.agents = serde_json::from_value(
        crate::sidebar::owned_label_fixture::<Value>(json!({"agents":[
            {"id":"first-parent","pane_id":"w-order:t1:p","agent_status":"working","state_change_seq":1,"tokens":{"task":"First parent"}},
            {"id":"second-parent","pane_id":"w-order:t2:p","agent_status":"working","state_change_seq":1,"tokens":{"task":"Second parent"}},
            {"id":"first-child","pane_id":"w-order:t4:p","spawned_from_pane_id":"w-order:t1:p","agent_status":"working","state_change_seq":1,"tokens":{"task":"First child"}},
            {"id":"second-child","pane_id":"w-order:t5:p","spawned_from_pane_id":"w-order:t2:p","agent_status":"working","state_change_seq":1,"tokens":{"task":"Second child"}}
        ]})).unwrap()["agents"].clone(),
    ).unwrap();
    runtime.ingest_session(Ok(payload));
    runtime.sync_workspace_view();
    assert_eq!(tabs(&runtime, None), TABS);
    let target = create(&mut runtime, "delegated-group", "Delegated work");
    group(
        &mut runtime,
        "move-parent",
        json!({"action":"move","group_id":target,"tab_id":TABS[0]}),
    );
    runtime.focus_pane("w-order:t4:p".into(), PaneFocusOrigin::Operator, None);
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[0])
    );
    (runtime, checkout, target)
}

#[test]
fn a_new_draft_replaces_the_delegated_canvas_without_creating_a_pane() {
    let (mut runtime, checkout, target) = with_delegated_canvases();
    let before = ordered_tab_ids(&runtime, &checkout);
    group(
        &mut runtime,
        "draft-over-child",
        json!({"action":"new_tab","group_id":target}),
    );
    let layout = published(&runtime);
    assert!(layout.canvases.is_empty());
    let Node::Area(area) = &layout.root else {
        panic!("one area");
    };
    assert!(
        area.active
            .as_ref()
            .is_some_and(|id| layout.drafts.contains(id))
    );
    assert!(runtime.snapshot().terminal.pane_id.is_none());
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout),
        before,
        "the child keeps its existing pane; the draft creates none"
    );
}

#[test]
fn a_live_tab_moved_into_a_delegated_canvas_becomes_visible_and_keeps_both_panes() {
    let (mut runtime, checkout, target) = with_delegated_canvases();
    let before = ordered_tab_ids(&runtime, &checkout);
    group(
        &mut runtime,
        "move-over-child",
        json!({"action":"move","group_id":target,"tab_id":TABS[1]}),
    );
    assert!(published(&runtime).canvases.is_empty());
    assert_eq!(
        runtime.snapshot().terminal.pane_id.as_deref(),
        Some("w-order:t2:p")
    );
    assert_eq!(
        checkout_active_tab_id(&runtime, &checkout).as_deref(),
        Some(TABS[1])
    );
    assert_eq!(
        ordered_tab_ids(&runtime, &checkout),
        before,
        "moving does not close or restart the child or moved pane"
    );
}

#[test]
fn dissolving_and_undoing_a_group_preserves_each_collections_delegated_canvas() {
    let (mut runtime, checkout, target) = with_delegated_canvases();
    let before = ordered_tab_ids(&runtime, &checkout);
    runtime.focus_pane("w-order:t5:p".into(), PaneFocusOrigin::Operator, None);
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[1])
    );
    select(&mut runtime, "show-group-child", Some(&target));
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[0])
    );
    group(
        &mut runtime,
        "dissolve-children",
        json!({"action":"dissolve","group_id":target}),
    );
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[1])
    );
    group(&mut runtime, "restore-children", json!({"action":"undo"}));
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[0])
    );
    select(&mut runtime, "show-ungrouped-child", None);
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[1])
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), before);
}

#[test]
fn add_pane_accepts_the_shown_delegated_tab_and_refuses_a_stale_canvas_target() {
    let (mut runtime, checkout, target) = with_delegated_canvases();
    let before = ordered_tab_ids(&runtime, &checkout);
    let destination = |tab: &str| super::super::group_starts::StartTarget {
        group_id: Some(target.clone()),
        area_id: "a1".into(),
        tab_id: Some(tab.into()),
        add_pane: true,
    };
    assert!(
        runtime
            .prepare_group_start(
                101,
                (crate::node::TEST_NODE, PATH),
                Some(destination(CHILDREN[1])),
                Some("stale-child"),
                Some("claude"),
                false
            )
            .is_err()
    );
    let prepared = runtime
        .prepare_group_start(
            102,
            (crate::node::TEST_NODE, PATH),
            Some(destination(CHILDREN[0])),
            Some("shown-child"),
            Some("claude"),
            false,
        )
        .expect("shown child admits a pane start");
    assert_eq!(prepared.request.tab_id.as_deref(), Some(CHILDREN[0]));
    assert_eq!(prepared.saved.try_recv().unwrap(), Ok(()));
    assert!(runtime.group_start_target_is_current(102));
    runtime.dispatch_json(&focus_tab_event(&checkout, TABS[0]));
    runtime.sync_workspace_view();
    assert!(
        runtime.group_start_target_is_current(102),
        "an admitted start can finish after the user leaves its canvas"
    );
    runtime.complete_group_start(102, Some(CHILDREN[0]), "w-order:t4:new-pane", false);
    assert!(
        runtime.group_start_target_is_current(102),
        "agent launch keeps the accepted live tab"
    );
    assert_eq!(
        tabs(&runtime, Some(&target)),
        [TABS[0]],
        "delegated tabs never gain an ordinary strip slot"
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), before);
}

#[test]
fn moving_a_root_prunes_only_its_old_groups_child_canvas_and_reveals_the_child_in_its_new_group() {
    for create_with_tab in [false, true] {
        let (mut runtime, checkout, source) = with_delegated_canvases();
        let before = ordered_tab_ids(&runtime, &checkout);
        runtime.focus_pane("w-order:t5:p".into(), PaneFocusOrigin::Operator, None);
        runtime.sync_workspace_view();
        select(&mut runtime, "root-source", Some(&source));
        let target = if create_with_tab {
            group(
                &mut runtime,
                "new-root-group",
                json!({"action":"create","name":"Moved root","tab_id":TABS[0]}),
            );
            published(&runtime).group_id.clone().unwrap()
        } else {
            let target = create(&mut runtime, "new-root-group", "Moved root");
            group(
                &mut runtime,
                "move-root",
                json!({"action":"move","group_id":target,"tab_id":TABS[0]}),
            );
            target
        };
        assert_eq!(
            runtime.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t1:p")
        );
        select(&mut runtime, "old-root-group", Some(&source));
        assert!(published(&runtime).canvases.is_empty());
        assert!(
            runtime.snapshot().terminal.pane_id.is_none(),
            "the empty source cannot show a descendant of the moved root"
        );
        select(&mut runtime, "unrelated-root", None);
        assert_eq!(
            published(&runtime).canvases.get("a1").map(String::as_str),
            Some(CHILDREN[1])
        );
        runtime.focus_pane("w-order:t4:p".into(), PaneFocusOrigin::Operator, None);
        runtime.sync_workspace_view();
        assert_eq!(
            published(&runtime).group_id.as_deref(),
            Some(target.as_str())
        );
        assert_eq!(
            published(&runtime).canvases.get("a1").map(String::as_str),
            Some(CHILDREN[0])
        );
        assert_eq!(ordered_tab_ids(&runtime, &checkout), before);
    }
}

#[test]
fn restored_draft_keeps_its_group_without_a_saved_checkout_or_pane() {
    restored_group_without_saved_focus(true);
}

#[test]
fn restored_live_group_replaces_the_initial_ungrouped_keyboard() {
    restored_group_without_saved_focus(false);
}

fn restored_group_without_saved_focus(draft: bool) {
    let (folder, state) = views_path("group-first-session");
    let path = folder.path().join("checkout");
    fs::create_dir(&path).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut stored = WorkspaceViewStore::open(state.clone(), (false, RightPanelSection::Changes)).0;
    let saved_path = fs::canonicalize(&path)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let view = stored.views.entry(crate::node::TEST_NODE, &saved_path);
    view.last_used_unix_ms = 1;
    view.reconcile_agent_groups(
        &TABS
            .iter()
            .map(|id| ((*id).to_owned(), false))
            .collect::<Vec<_>>(),
        0,
        &HashSet::new(),
        &HashSet::new(),
    )
    .unwrap();
    let target = view.create_agent_group("문서 검토").unwrap();
    let active = if draft {
        view.create_agent_draft("a1", 1).unwrap()
    } else {
        view.move_agent_to_group(TABS[1], Some(&target), 1).unwrap();
        TABS[1].to_owned()
    };
    crate::workspace_views::save(&state, &stored.views).unwrap();
    let mut restored = runtime();
    restored.workspace_views =
        Some(WorkspaceViewStore::open(state, (false, RightPanelSection::Changes)).0);
    restored.rebuild_catalog();
    restored.sync_workspace_view();
    restored.catalog_paths = Default::default();
    restored.begin_catalog_coordinator();
    let payload = tab_order_payload(&path, &TABS, &TABS, TABS[0]);
    // The coordinator excludes unknown cwds before building its first catalog.
    // The saved group belongs to the canonical path discovered in the next answer.
    restored.ingest_session_with_catalog(
        Ok(payload.clone()),
        Some(session_sync::PrecomputedCatalog {
            registrations: Vec::new(),
            worktrees: restored.worktree_catalog(),
            workspaces: Vec::new(),
            paths: Default::default(),
        }),
    );
    restored.sync_workspace_view();
    assert_eq!(
        restored
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(crate::node::TEST_NODE, &saved_path)
            .unwrap()
            .agent_groups
            .selected
            .as_deref(),
        Some(target.as_str()),
        "before path facts"
    );
    let paths = restored.ask_paths(BTreeSet::from([path.clone()]));
    let spaces = Runtime::session_spaces(&payload, &paths);
    let catalog = session_sync::PrecomputedCatalog {
        registrations: restored.snapshot.ui_state.workspace_registrations.clone(),
        worktrees: restored.worktree_catalog(),
        workspaces: workspace::build_catalog(
            &restored.node,
            &restored.snapshot.ui_state.workspace_registrations,
            &spaces,
            &restored.worktree_catalog(),
            &paths,
        ),
        paths,
    };
    restored.ingest_session_with_catalog(Ok(payload), Some(catalog));
    restored.sync_workspace_view();
    assert_eq!(
        published(&restored).group_id.as_deref(),
        Some(target.as_str()),
        "first session"
    );
    let (workspace, checkout) = restored.front_checkout_owned().unwrap();
    dispatch(
        &mut restored,
        "focus_checkout",
        json!({"workspace_id":workspace,"checkout_id":checkout}),
    );
    assert_eq!(
        published(&restored).group_id.as_deref(),
        Some(target.as_str()),
        "opening the project"
    );
    assert_eq!(tabs(&restored, Some(&target)), [active]);
    if draft {
        assert!(restored.snapshot().terminal.pane_id.is_none());
    } else {
        assert_eq!(
            restored.snapshot().terminal.pane_id.as_deref(),
            Some("w-order:t2:p")
        );
    }
}

#[test]
fn a_stored_delegated_canvas_waits_for_its_canonical_lineage() {
    let (mut runtime, checkout, target) = with_delegated_canvases();
    let before = ordered_tab_ids(&runtime, &checkout);
    // The saved view and catalog can be available before agent lineage is
    // published, as on startup or while a remote catalog is reconnecting.
    let agents = std::mem::take(&mut runtime.snapshot.navigator.agents);
    group(
        &mut runtime,
        "rename-before-lineage",
        json!({"action":"rename","group_id":target,"name":"Renamed"}),
    );
    select(&mut runtime, "select-before-lineage", Some(&target));
    assert_eq!(
        published(&runtime).group_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[0])
    );
    runtime.snapshot.navigator.agents = agents;
    runtime.sync_workspace_view();
    assert_eq!(
        published(&runtime).canvases.get("a1").map(String::as_str),
        Some(CHILDREN[0])
    );
    assert_eq!(ordered_tab_ids(&runtime, &checkout), before);
}

#[test]
fn pending_creation_occupies_its_draft_once_even_when_topology_beats_the_answer() {
    for ordinary_count in [3, 63] {
        let (mut runtime, checkout) = setup();
        let existing: Vec<String> = (1..=ordinary_count)
            .map(|n| format!("w-order:t{n}"))
            .collect();
        let existing_ids: Vec<&str> = existing.iter().map(String::as_str).collect();
        ingest(&mut runtime, &existing_ids);
        let target = create(&mut runtime, "pending-placement", "Starting");
        group(
            &mut runtime,
            "pending-draft",
            json!({"action":"new_tab","group_id":target}),
        );
        let draft = tabs(&runtime, Some(&target))[0].clone();
        let prepared = runtime
            .prepare_group_start(
                130,
                (crate::node::TEST_NODE, PATH),
                Some(super::super::group_starts::StartTarget {
                    group_id: Some(target.clone()),
                    area_id: "a1".into(),
                    tab_id: Some(draft.clone()),
                    add_pane: false,
                }),
                Some("creation-before-answer"),
                None,
                false,
            )
            .expect("draft has exactly one admitted start");
        assert_eq!(prepared.saved.try_recv().unwrap(), Ok(()));
        let saved = runtime
            .save_group_start_effect(
                130,
                crate::start_claim::Effect {
                    workspace_id: "w-order".into(),
                    seed_tab_id: None,
                    target: None,
                    expected_command: Some(vec!["/bin/sh".into(), "-l".into()]),
                    shell_env: Default::default(),
                    shell_readiness: hide_node_link::protocol::ShellReadiness::Plain,
                    binding: None,
                },
            )
            .unwrap();
        assert_eq!(saved.try_recv().unwrap(), Ok(()));
        let stage = "w-order:t100";
        let mut all = existing_ids.clone();
        all.push(stage);
        let mut payload = tab_order_payload(PATH, &all, &all, TABS[0]);
        payload.workspaces[0].tokens.insert(
            crate::checkout_owner::OWNER_TOKEN.into(),
            json!(crate::checkout_owner::owner_mark(
                crate::node::TEST_NODE,
                PATH
            )),
        );
        payload
            .panes
            .iter_mut()
            .find(|pane| pane.pane_id == format!("{stage}:p"))
            .unwrap()
            .start_claim = Some(crate::start_claim::label(&prepared.request.marker));
        runtime.ingest_session(Ok(payload.clone()));
        runtime.sync_workspace_view();
        assert_eq!(
            ordered_tab_ids(&runtime, &checkout),
            existing,
            "the staging tab is neither a second visible tab nor overflow"
        );
        assert_eq!(tabs(&runtime, None), existing);
        assert_eq!(tabs(&runtime, Some(&target)), std::slice::from_ref(&draft));
        runtime.complete_group_start(130, Some(stage), &format!("{stage}:p"), false);
        runtime.ingest_session(Ok(payload));
        runtime.sync_workspace_view();
        assert_eq!(tabs(&runtime, Some(&target)), [stage]);
        assert_eq!(tabs(&runtime, None), existing);
        assert_eq!(
            ordered_tab_ids(&runtime, &checkout).len(),
            ordinary_count + 1
        );
        assert!(!published(&runtime).drafts.contains(&draft));
    }
}
