//! A tab the operator named outlives its last agent (docs/UI_BEHAVIOR.md,
//! Agent areas): closing that agent's pane leaves a shell in the tab.
use super::workspace_view::with_new_views;
use super::*;

const PATH: &str = "/agent-groups";
const AGENT: &str = "w-order:t1:p";
const SHELL: &str = "w-order:t1:shell";

/// Tabs t1 and t2 of one checkout. t1 carries `label` and holds `t1_panes`
/// side by side, the first holding an idle Claude when `agent`; t2 holds
/// its one shell. The operator is on t1.
fn session(label: &str, agent: bool, t1_panes: &[&str]) -> SessionSnapshotPayload {
    let rect =
        |x: u32, width: u32| serde_json::json!({"x": x, "y": 0, "width": width, "height": 24});
    let width = 80 / t1_panes.len() as u32;
    let t1_layout_panes = t1_panes
        .iter()
        .enumerate()
        .map(|(index, pane)| serde_json::json!({"pane_id": pane, "rect": rect(index as u32 * width, width)}))
        .collect::<Vec<_>>();
    let t1_splits = if t1_panes.len() == 2 {
        serde_json::json!([{"direction": "right", "ratio": 0.5, "rect": rect(0, 80),
            "first": {"type": "pane", "pane_id": t1_panes[0]},
            "second": {"type": "pane", "pane_id": t1_panes[1]}}])
    } else {
        serde_json::json!([])
    };
    let mut panes = t1_panes
        .iter()
        .map(|pane| serde_json::json!({"pane_id": pane, "cwd": PATH}))
        .collect::<Vec<_>>();
    panes.push(serde_json::json!({"pane_id": "w-order:t2:p", "cwd": PATH}));
    let agents = if agent {
        serde_json::json!([{"id": "a1", "pane_id": AGENT, "agent": "claude", "agent_status": "idle",
            "state_change_seq": 1, "lineage_session": "s1"}])
    } else {
        serde_json::json!([])
    };
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": agents,
        "focused_workspace_id": "w-order",
        "focused_pane_id": t1_panes[0],
        "workspaces": [{"workspace_id": "w-order", "label": "order", "active_tab_id": "w-order:t1"}],
        "tabs": [
            {"workspace_id": "w-order", "tab_id": "w-order:t1", "label": label, "number": 1},
            {"workspace_id": "w-order", "tab_id": "w-order:t2", "label": "", "number": 2}
        ],
        "panes": panes,
        "layouts": [
            {"workspace_id": "w-order", "tab_id": "w-order:t1", "zoomed": false, "area": rect(0, 80),
             "focused_pane_id": t1_panes[0], "panes": t1_layout_panes, "splits": t1_splits},
            {"workspace_id": "w-order", "tab_id": "w-order:t2", "zoomed": false, "area": rect(0, 80),
             "focused_pane_id": "w-order:t2:p",
             "panes": [{"pane_id": "w-order:t2:p", "rect": rect(0, 80)}], "splits": []}
        ]
    }))
    .expect("named tab session")
}

/// t1 carries `label`, which the operator gave it through Rename when the
/// label is a name.
fn started(label: &str, agent: bool) -> Runtime {
    let mut runtime = started_unrenamed(label, agent);
    rename(&mut runtime, label);
    runtime
}

/// t1 carries `label` as Herdr holds it, given by someone other than the
/// operator's Rename: a spawn, or a Rename before Hide recorded them.
fn started_unrenamed(label: &str, agent: bool) -> Runtime {
    let (runtime, _) = live_tab_order_runtime(PATH);
    let mut runtime = with_new_views(runtime, "named-tab");
    runtime.ingest_session(Ok(session(label, agent, &[AGENT])));
    runtime.sync_workspace_view();
    runtime.select_terminal_pane(Some(AGENT.to_owned()));
    runtime
}

/// The operator renames t1 to `label` and Herdr accepts it.
fn rename(runtime: &mut Runtime, label: &str) {
    runtime.rename_tab(RenameTabPayload {
        request_id: format!("rename {label}"),
        tab_id: "w-order:t1".into(),
        label: label.into(),
    });
    runtime.ingest_tab_rename_result(&format!("rename {label}"), Ok(()));
}

fn close_pane(runtime: &mut Runtime, pane: &str) {
    runtime.dispatch_json(&explorer_event(
        "close_pane",
        serde_json::json!({"pane_id": pane, "confirmed": false}),
    ));
}

fn the_close(runtime: &Runtime) -> &PendingClose {
    let [operation] = runtime.close_operations.values().collect::<Vec<_>>()[..] else {
        panic!("one close in progress");
    };
    operation
}

/// The tabs the Agent areas show, in order.
fn shown(runtime: &mut Runtime) -> Vec<String> {
    runtime.sync_workspace_view();
    let layout = runtime
        .snapshot()
        .workspace_view
        .as_ref()
        .and_then(|view| view.agent_layout.as_ref())
        .expect("this machine's Agent areas")
        .clone();
    let crate::split_tree::Node::Area(area) = &layout.root else {
        panic!("one area");
    };
    area.displays.iter().map(|tab| tab.id.clone()).collect()
}

fn tab_t1(runtime: &Runtime) -> Option<TabSnapshot> {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .flat_map(|checkout| &checkout.tabs)
        .find(|tab| tab.id.as_deref() == Some("w-order:t1"))
        .cloned()
}

/// The capture Herdr answers, as a pane close records it for Reopen.
fn captured(runtime: &mut Runtime) -> live::CloseEffectRequest {
    let request = the_close(runtime).request.clone();
    let item = ClosedItem::Pane {
        key: request.key.clone(),
        context: request.context.clone(),
        pane: request.panes[0].clone(),
        placement: crate::recent_closed::PanePlacement {
            neighbor_pane_id: None,
            parent_path: Vec::new(),
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            target_was_first: false,
        },
    };
    let (_, effects) = runtime
        .ingest_close_capture_result(&request, Ok(live::CloseCaptureOutcome { item: Some(item) }));
    let [effect] = effects.as_slice() else {
        panic!("one close effect, got {effects:?}");
    };
    effect.clone()
}

#[test]
fn closing_a_named_tabs_last_agent_keeps_the_tab_its_name_and_its_place() {
    let mut runtime = started("Planning", true);
    close_pane(&mut runtime, AGENT);

    let context = &the_close(&runtime).request.context;
    assert!(context.replacement_shell && context.replacement_in_tab);
    assert_eq!(
        context.tab_name,
        ClosedTabName::Named("Planning".into()),
        "a reopen names the tab only with the operator's name"
    );
    // Nothing moves while the shell is made: the tab stays shown, with the
    // keyboard, and no new tab is reserved for it.
    assert_eq!(shown(&mut runtime), ["w-order:t1", "w-order:t2"]);
    assert_eq!(runtime.snapshot().terminal.pane_id.as_deref(), Some(AGENT));
    assert!(runtime.admit_agent_tab(PATH));

    let effect = captured(&mut runtime);
    let shell_context = effect.replacement.as_ref().expect("a shell first");
    assert!(shell_context.replacement_in_tab);
    runtime
        .ingest_close_replacement(&effect, SHELL, session("Planning", true, &[AGENT, SHELL]))
        .expect("the shell is this close's own");
    assert_eq!(runtime.snapshot().terminal.pane_id.as_deref(), Some(SHELL));
    assert!(runtime.ingest_close_effect_result(&effect, Ok(())));

    runtime.ingest_session(Ok(session("Planning", false, &[SHELL])));
    let tab = tab_t1(&runtime).expect("the named tab stays");
    assert_eq!(tab.naming.manual_name(), Some("Planning"));
    assert_eq!(
        tab.panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>(),
        [SHELL]
    );
    assert_eq!(shown(&mut runtime), ["w-order:t1", "w-order:t2"]);
    assert!(
        runtime.close_operations.is_empty(),
        "the close is confirmed"
    );
    assert!(
        runtime.snapshot().recent_closed.can_reopen,
        "the agent pane can be reopened"
    );
}

/// Herdr's event stream can lag the read that showed the shell: an update
/// from before it still has the agent pane, focused, and no shell. The
/// keyboard stays on the shell rather than go back to the pane the close is
/// taking, where it would stay once that pane is gone.
#[test]
fn an_update_older_than_the_shell_keeps_the_keyboard_on_the_shell() {
    let mut runtime = started("Planning", true);
    close_pane(&mut runtime, AGENT);
    let effect = captured(&mut runtime);
    runtime
        .ingest_close_replacement(&effect, SHELL, session("Planning", true, &[AGENT, SHELL]))
        .expect("the shell is this close's own");
    assert!(runtime.ingest_close_effect_result(&effect, Ok(())));

    runtime.ingest_session(Ok(session("Planning", true, &[AGENT])));
    assert_eq!(runtime.snapshot().terminal.pane_id.as_deref(), Some(SHELL));
    assert!(runtime.snapshot().status.last_error.is_none());

    runtime.ingest_session(Ok(session("Planning", false, &[SHELL])));
    assert_eq!(runtime.snapshot().terminal.pane_id.as_deref(), Some(SHELL));
}

/// Today's close, for every tab the operator did not name and for every
/// pane that holds no agent, the leftover shell included.
#[test]
fn an_unnamed_tab_and_a_named_tabs_leftover_shell_close_their_tab_as_before() {
    for (label, agent) in [("", true), ("3", true), ("Planning", false)] {
        let mut runtime = started(label, agent);
        close_pane(&mut runtime, AGENT);
        let context = &the_close(&runtime).request.context;
        assert!(!context.replacement_in_tab, "{label:?} with agent {agent}");
        assert!(!context.replacement_shell);
        let named = (label == "Planning").then(|| ClosedTabName::Named(label.into()));
        assert_eq!(
            context.tab_name,
            named.unwrap_or(ClosedTabName::Unnamed),
            "a reopen sends Herdr no label for a tab the operator never named"
        );
        assert_eq!(
            shown(&mut runtime),
            ["w-order:t2"],
            "the tab leaves at once"
        );
    }
}

/// A name a spawn gave, or one given before Hide recorded the operator's
/// Rename, is shown but does not make the tab named: it closes with its last
/// agent and comes back under that name, unnamed. Renaming it makes it
/// named; clearing the name makes it unnamed again.
#[test]
fn only_the_operators_rename_makes_a_tab_named() {
    let mut runtime = started_unrenamed("agent-6651", true);
    let tab = tab_t1(&runtime).unwrap();
    assert_eq!(tab.naming.manual_name(), Some("agent-6651"));
    assert_eq!(tab.naming.operator_name(), None);
    close_pane(&mut runtime, AGENT);
    let context = &the_close(&runtime).request.context;
    assert!(!context.replacement_in_tab);
    assert_eq!(
        context.tab_name,
        ClosedTabName::Labeled("agent-6651".into())
    );
    assert_eq!(
        shown(&mut runtime),
        ["w-order:t2"],
        "the tab leaves at once"
    );

    let mut runtime = started_unrenamed("Planning", true);
    rename(&mut runtime, "Planning");
    assert_eq!(
        tab_t1(&runtime).unwrap().naming.operator_name(),
        Some("Planning")
    );
    rename(&mut runtime, "");
    runtime.ingest_session(Ok(session("", true, &[AGENT])));
    assert_eq!(tab_t1(&runtime).unwrap().naming.operator_name(), None);
    assert!(runtime.snapshot.ui_state.operator_tab_names.is_empty());
}

/// A tab the operator named comes back named from Reopen closed tab, under
/// the tab id Herdr gives it.
#[test]
fn a_reopened_tab_the_operator_named_is_named_again() {
    let mut runtime = started("Planning", true);
    let context = runtime
        .close_context(&tab_t1(&runtime).unwrap())
        .expect("the tab's context");
    let request = live::ReopenRequest {
        codex_daemon: Default::default(),
        item: ClosedItem::Tab {
            key: "reopen".into(),
            context,
            layout: crate::recent_closed::ClosedLayout {
                workspace_id: "w-order".into(),
                tab_id: "w-order:t1".into(),
                zoomed: false,
                focused_pane_id: AGENT.into(),
                root: crate::recent_closed::ClosedLayoutNode::Pane {
                    pane_id: Some(AGENT.into()),
                    label: None,
                    cwd: Some(PATH.into()),
                    command: None,
                    env: Default::default(),
                },
            },
            panes: Vec::new(),
        },
        workspace_exists: true,
        tab_exists: false,
        fallback_pane_id: None,
        owner: None,
    };
    runtime.reopen_in_flight = Some("reopen".into());
    runtime.ingest_reopen_result(
        &request,
        Ok(live::FileReopenResultOrHerdr::Herdr(live::ReopenOutcome {
            tab_id: Some("w-order:t9".into()),
            consumed: true,
            focused_pane_id: None,
            notices: Vec::new(),
            agents: Vec::new(),
        })),
    );
    assert_eq!(
        runtime
            .snapshot
            .ui_state
            .operator_tab_names
            .get("w-order:t9")
            .map(String::as_str),
        Some("Planning")
    );
}

#[test]
fn close_tab_closes_a_named_tab_and_its_name() {
    let mut runtime = started("Planning", true);
    runtime.dispatch_json(&explorer_event(
        "close_tab",
        serde_json::json!({"tab_id": "w-order:t1", "confirmed": false}),
    ));
    assert!(!the_close(&runtime).request.context.replacement_in_tab);
    assert_eq!(shown(&mut runtime), ["w-order:t2"]);
}

/// A delegated child's tab and a failed terminal, which must stay closable,
/// keep no shell even when the operator named the tab.
#[test]
fn a_delegated_tab_and_a_failed_terminal_are_not_kept() {
    let runtime = started("Planning", true);
    let target = live::CloseCaptureTarget::Pane {
        pane_id: AGENT.into(),
    };
    let tab = tab_t1(&runtime).unwrap();
    assert!(runtime.keeps_named_tab(&target, &tab));
    let mut delegated = tab.clone();
    delegated.delegated = true;
    assert!(!runtime.keeps_named_tab(&target, &delegated));
    let mut failed = tab;
    failed.panes[0].failed_terminal_id = Some("terminal".into());
    assert!(!runtime.keeps_named_tab(&target, &failed));
}

/// A refused close keeps the shell, and Retry closes beside the same shell
/// rather than making a second; a shell the operator closed meanwhile is
/// made again.
#[test]
fn retry_reuses_the_shell_in_the_tab_and_makes_another_only_when_it_is_gone() {
    let mut runtime = started("Planning", true);
    close_pane(&mut runtime, AGENT);
    let effect = captured(&mut runtime);
    runtime
        .ingest_close_replacement(&effect, SHELL, session("Planning", true, &[AGENT, SHELL]))
        .unwrap();
    runtime.ingest_close_effect_result(
        &effect,
        Err(hide_herdr_client::ApiError::Remote {
            code: "fixture_refused".into(),
            message: "refused".into(),
        }),
    );
    let key = effect.key.clone();
    assert_eq!(runtime.close_operations[&key].phase, "refused");
    assert!(
        tab_t1(&runtime)
            .unwrap()
            .panes
            .iter()
            .any(|pane| pane.id == SHELL)
    );

    assert!(runtime.retry_agent_close(&key));
    let operation = &runtime.close_operations[&key];
    assert!(
        !operation.allow_replacement_create,
        "the shell is still there"
    );
    assert!(runtime.close_precondition_failure(operation).is_none());

    // The operator closed the shell before the next retry.
    runtime.close_operations.get_mut(&key).unwrap().phase = "refused".into();
    runtime.ingest_session(Ok(session("Planning", true, &[AGENT])));
    assert!(runtime.retry_agent_close(&key));
    let operation = &runtime.close_operations[&key];
    assert!(operation.allow_replacement_create);
    assert_eq!(operation.replacement_id, None);
    assert!(runtime.close_precondition_failure(operation).is_none());
}

/// The worker made the shell but could not hand it over (Herdr's answer was
/// lost): the failure names the shell, and Retry closes beside it instead of
/// refusing the tab as changed or making a second one.
#[test]
fn retry_closes_beside_a_shell_the_worker_made_but_could_not_hand_over() {
    let mut runtime = started("Planning", true);
    close_pane(&mut runtime, AGENT);
    let effect = captured(&mut runtime);
    runtime.note_shell_in_tab(&effect.key, SHELL);
    runtime.ingest_close_effect_result(
        &effect,
        Err(hide_herdr_client::ApiError::Remote {
            code: "replacement_failed".into(),
            message: "pane.split response is missing pane.pane_id".into(),
        }),
    );
    let key = effect.key.clone();
    assert_eq!(runtime.close_operations[&key].phase, "refused");
    runtime.ingest_session(Ok(session("Planning", true, &[AGENT, SHELL])));

    assert!(runtime.retry_agent_close(&key));
    let operation = &runtime.close_operations[&key];
    assert_eq!(operation.phase, "preparing", "the close goes on");
    assert!(
        !operation.allow_replacement_create,
        "the worker adopts the shell already in the tab"
    );
}

/// A shell the worker made in time but handed over after the close moved on
/// (its deadline passed, or the connection changed) is still the close's own,
/// so Retry closes beside it rather than refusing the tab as changed.
#[test]
fn a_shell_handed_over_after_the_close_moved_on_still_counts_for_retry() {
    let mut runtime = started("Planning", true);
    close_pane(&mut runtime, AGENT);
    let effect = captured(&mut runtime);
    let key = effect.key.clone();
    runtime.close_operations.get_mut(&key).unwrap().phase = "unknown".into();
    assert!(
        runtime
            .ingest_close_replacement(&effect, SHELL, session("Planning", true, &[AGENT, SHELL]))
            .is_err()
    );

    // Herdr's events show the shell; the status check found the agent pane
    // still open.
    runtime.ingest_session(Ok(session("Planning", true, &[AGENT, SHELL])));
    runtime.close_operations.get_mut(&key).unwrap().phase = "refused".into();
    assert!(runtime.retry_agent_close(&key));
    let operation = &runtime.close_operations[&key];
    assert_eq!(operation.phase, "preparing", "the close goes on");
    assert!(!operation.allow_replacement_create);
}
