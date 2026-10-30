//! The Project tool's 흘린 것 (PRD project-panel-results B6-B16): which
//! worktrees stand as rows, how 놔두기 holds one down until something new
//! happens there, and how 마무리 맡기기 sends once per press.
use super::*;
use crate::agent_state::tally::project_status::{MAX_STRAY_DISMISSALS, Stray, StrayDismissal};

fn event(runtime: &mut Runtime, kind: &str, payload: serde_json::Value) -> bool {
    runtime.dispatch_json(&explorer_event(kind, payload))
}

/// A worktree on its own branch with `files` changed and the commit `head`.
fn worktree(
    name: &str,
    files: u32,
    head: &str,
    pane_id: Option<&str>,
) -> crate::model::CheckoutSnapshot {
    let path = format!("/fixture/{name}");
    let mut row = checkout("project", name, &path, pane_id.map(|id| pane(id, &path)));
    row.is_worktree = true;
    row.branch = Some(name.to_owned());
    row.base_branch = Some("main".to_owned());
    row.worktree = Some(crate::model::WorktreeSnapshot {
        dirty: files > 0,
        changed_file_count: files,
        head_sha: Some(head.to_owned()),
        ..Default::default()
    });
    row
}

fn project(runtime: &mut Runtime, checkouts: Vec<crate::model::CheckoutSnapshot>) {
    let mut main = checkout("project", "main", "/fixture", None);
    main.is_primary = true;
    main.branch = Some("main".to_owned());
    main.base_branch = Some("main".to_owned());
    // The default branch's own changes are the work under way, never a row (B7).
    main.worktree = Some(crate::model::WorktreeSnapshot {
        dirty: true,
        changed_file_count: 2,
        is_main: true,
        ..Default::default()
    });
    let mut project = workspace(
        "project",
        "Project",
        "/fixture",
        [vec![main], checkouts].concat(),
    );
    project.is_git = true;
    runtime.snapshot.navigator.workspaces = vec![project];
    runtime.refresh_agent_scopes();
}

fn strays(runtime: &Runtime) -> Vec<Stray> {
    runtime.snapshot.navigator.workspaces[0]
        .agent_scope
        .status
        .strays
        .clone()
}

fn agent(pane_id: &str, activity: &str) -> SidebarAgentSnapshot {
    let mut row = project_agents(
        serde_json::from_value(serde_json::json!({"agents": [
            {"pane_id": pane_id, "agent": "codex", "agent_status": activity, "state_change_seq": 1}
        ]}))
        .unwrap(),
    )
    .agents
    .remove(0);
    row.activity = activity.into();
    row.identity_label = "Retry failed emails".into();
    row.changed_at_unix_ms = Some(5_000);
    row.state = crate::agent_state::turn::row_state(&row);
    row
}

/// B6, B7: a worktree whose agent stopped with changes left is a row named
/// by that agent's task; while the agent works again it is not one, and
/// the default branch's checkout never is.
#[test]
fn a_worktree_left_with_changes_stands_until_its_agent_works_there_again() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents = vec![agent("p1", "idle")];
    project(&mut runtime, vec![worktree("emails", 4, "aaa", Some("p1"))]);
    let rows = strays(&runtime);
    assert_eq!(rows.len(), 1, "the default branch's changes are no row");
    let row = &rows[0];
    assert_eq!(
        (
            row.checkout_id.as_str(),
            row.name.as_deref(),
            row.pane_id.as_deref(),
            row.changed_files,
            row.agent.as_deref()
        ),
        (
            "emails",
            Some("Retry failed emails"),
            Some("p1"),
            4,
            Some("codex")
        )
    );
    assert_eq!(
        runtime.snapshot.navigator.workspaces[0]
            .agent_scope
            .status
            .strays_total,
        1
    );

    runtime.snapshot.navigator.agents = vec![agent("p1", "working")];
    runtime.refresh_agent_scopes();
    assert!(
        strays(&runtime).is_empty(),
        "a working agent's worktree is its own"
    );
}

/// B6, B7: a pull request stands for its failed CI alone; a draft or one
/// whose checks pass leaves nothing, and a worktree with nothing left is
/// no row.
#[test]
fn a_pull_request_stands_only_for_its_failed_checks() {
    let pr = |checks: &str, draft: bool| -> crate::model::PullRequestSnapshot {
        serde_json::from_value(serde_json::json!({
            "number": 1815, "title": "Ship", "url": "https://github.com/acme/app/pull/1815",
            "head_branch": "ship", "base_branch": "main", "badge": "open", "checks": checks, "is_draft": draft,
            "closing_issues": [],
        }))
        .unwrap()
    };
    let mut runtime = runtime();
    let mut failed = worktree("ship", 0, "aaa", None);
    failed.pull_request = Some(pr("failed", false));
    let mut draft = worktree("draft", 0, "bbb", None);
    draft.pull_request = Some(pr("failed", true));
    let mut passing = worktree("passing", 0, "ccc", None);
    passing.pull_request = Some(pr("passing", false));
    project(
        &mut runtime,
        vec![failed, draft, passing, worktree("clean", 0, "ddd", None)],
    );
    let rows = strays(&runtime);
    assert_eq!(
        rows.iter()
            .map(|row| (row.checkout_id.as_str(), row.failed_pr))
            .collect::<Vec<_>>(),
        [("ship", Some(1815))]
    );
    assert_eq!(
        crate::runtime::strays::hand_off_words(&rows[0]),
        "#1815 CI 실패 원인 고쳐줘"
    );
}

/// B15: 놔두기 takes the row out and keeps it out across a restart until
/// the worktree has a new commit; 되돌리기 brings it back at once.
#[test]
fn leaving_a_row_holds_until_a_new_commit_and_undo_brings_it_back() {
    let mut runtime = runtime();
    project(&mut runtime, vec![worktree("emails", 4, "aaa", None)]);
    let dismiss = serde_json::json!({"workspace_id": "project", "checkout_id": "emails"});
    assert!(event(&mut runtime, "stray_dismiss", dismiss.clone()));
    assert!(strays(&runtime).is_empty());
    assert_eq!(
        runtime
            .snapshot
            .ui_state
            .stray_dismissals
            .get("emails")
            .and_then(|d| d.head.as_deref()),
        Some("aaa")
    );

    assert!(event(&mut runtime, "stray_restore", dismiss.clone()));
    assert_eq!(strays(&runtime).len(), 1);

    assert!(event(&mut runtime, "stray_dismiss", dismiss));
    project(&mut runtime, vec![worktree("emails", 4, "bbb", None)]);
    assert_eq!(
        strays(&runtime).len(),
        1,
        "a new commit stands the row again"
    );
}

/// B16: past the cap the oldest 놔두기 goes, so its row stands again, and
/// the count never grows past it.
#[test]
fn the_oldest_leave_goes_past_the_cap() {
    let mut runtime = runtime();
    project(
        &mut runtime,
        vec![
            worktree("emails", 4, "aaa", None),
            worktree("oldest", 1, "ooo", None),
        ],
    );
    let dismissals = &mut runtime.snapshot.ui_state.stray_dismissals;
    dismissals.insert(
        "oldest".into(),
        StrayDismissal {
            head: Some("ooo".into()),
            failed_pr: None,
            at_unix_ms: 1,
        },
    );
    for index in 0..MAX_STRAY_DISMISSALS - 1 {
        dismissals.insert(
            format!("gone{index}"),
            StrayDismissal {
                head: None,
                failed_pr: None,
                at_unix_ms: 1_000 + index as u64,
            },
        );
    }
    runtime.refresh_agent_scopes();
    assert_eq!(
        strays(&runtime)
            .iter()
            .map(|row| row.checkout_id.as_str())
            .collect::<Vec<_>>(),
        ["emails"]
    );

    assert!(event(
        &mut runtime,
        "stray_dismiss",
        serde_json::json!({"workspace_id": "project", "checkout_id": "emails"})
    ));
    assert_eq!(
        runtime.snapshot.ui_state.stray_dismissals.len(),
        MAX_STRAY_DISMISSALS
    );
    assert_eq!(
        strays(&runtime)
            .iter()
            .map(|row| row.checkout_id.as_str())
            .collect::<Vec<_>>(),
        ["oldest"]
    );
}

/// B12, B13: one press is one request: the same request again sends
/// nothing, and a worktree with nothing left is refused with a reason in
/// the receipt, the row untouched.
#[test]
fn a_hand_off_goes_once_per_request() {
    let mut runtime = runtime();
    project(&mut runtime, vec![]);
    let payload =
        serde_json::json!({"workspace_id": "project", "checkout_id": "emails", "request_id": "r1"});
    assert!(event(&mut runtime, "stray_hand_off", payload.clone()));
    assert!(!event(&mut runtime, "stray_hand_off", payload));
    let receipts = &runtime.snapshot.status.stray_hand_offs;
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| (receipt.request_id.as_str(), receipt.state))
            .collect::<Vec<_>>(),
        [("r1", "failed")]
    );
}

/// B17, D-21: an agent that stops working has the worktrees of its project
/// read again once, so changes it left unstaged reach the 흘린 것; an agent
/// that keeps working, or one already stopped, reads nothing.
#[test]
fn an_agent_that_stops_working_has_its_project_s_worktrees_read_again() {
    let mut runtime = runtime();
    runtime.snapshot.navigator.agents = vec![agent("p1", "working")];
    project(&mut runtime, vec![worktree("emails", 0, "aaa", Some("p1"))]);
    let generation = |runtime: &Runtime| {
        runtime
            .worktree_project_generations
            .get("/fixture")
            .copied()
            .unwrap_or(0)
    };
    let before = generation(&runtime);
    runtime.refresh_agent_scopes();
    assert_eq!(
        generation(&runtime),
        before,
        "a working agent reads nothing"
    );
    runtime.snapshot.navigator.agents = vec![agent("p1", "idle")];
    runtime.refresh_agent_scopes();
    assert_eq!(generation(&runtime), before + 1);
    assert!(runtime.snapshot.git_worktrees_loading);
    runtime.refresh_agent_scopes();
    assert_eq!(generation(&runtime), before + 1, "once per stop");
}

/// B9: `남은 것` reads the worktree it shows by that worktree's own path,
/// beside the checkout in front, whatever else is on screen; an answer for a
/// worktree the operator has left is dropped, and closing it empties it.
#[test]
fn the_remains_view_reads_its_worktree_apart_from_the_checkout_in_front() {
    let mut runtime = runtime();
    project(&mut runtime, vec![worktree("emails", 4, "aaa", None)]);
    runtime.snapshot.navigator.focused_workspace_id = Some("project".into());
    runtime.snapshot.navigator.focused_checkout_id = Some("main".into());
    runtime.ui_attached = true;
    runtime.snapshot.ui_state.right_panel_visible = true;
    runtime.snapshot.ui_state.right_panel_section = RightPanelSection::Changes;
    assert!(
        runtime.remains_request().0.is_none(),
        "nothing shows it yet"
    );
    assert!(event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": {"kind": "remains", "workspace_id": "project", "checkout_id": "emails"}})
    ));
    let front = runtime
        .changes_request()
        .expect("the Changes section reads the front");
    let remains = runtime
        .remains_request()
        .0
        .expect("the remains view reads its own");
    assert_eq!(front.root_path, "/fixture");
    assert_eq!(remains.root_path, "/fixture/emails");
    assert_eq!(
        (remains.selected_path.as_deref(), remains.diffs.len()),
        (None, 0)
    );

    let answer =
        |key: Option<crate::changes::ChangesKey>, root: &str| crate::changes::ChangesAnswer {
            key,
            detail: None,
            selection: (None, false),
            changes: crate::model::ChangesSnapshot {
                root_path: Some(root.into()),
                ..Default::default()
            },
        };
    assert!(!runtime.ingest_remains(answer(Some(front.key()), "/fixture")));
    assert!(
        runtime.snapshot.remains.is_none(),
        "another folder's answer is dropped"
    );
    assert!(runtime.ingest_remains(answer(Some(remains.key()), "/fixture/emails")));
    let held = runtime.snapshot.remains.as_ref().expect("the read is held");
    assert_eq!(
        (held.checkout_id.as_str(), held.changes.root_path.as_deref()),
        ("emails", Some("/fixture/emails")),
        "the read names its checkout, not only a folder inside the primary's"
    );
    assert!(event(
        &mut runtime,
        "detail_show",
        serde_json::json!({"shown": false})
    ));
    assert_eq!(
        runtime.remains_request(),
        (None, true),
        "nothing shown clears the held read"
    );
    assert!(!runtime.ingest_remains(answer(None, "")));
    assert!(runtime.snapshot.remains.is_none());
}

/// A worktree this daemon did not open cannot be read; `남은 것` says so
/// instead of waiting, and a failed read of a listed worktree keeps its list
/// marked stale rather than showing nothing left.
#[test]
fn the_remains_view_states_a_failed_read_instead_of_nothing_left() {
    let mut runtime = runtime();
    project(&mut runtime, vec![worktree("emails", 4, "aaa", None)]);
    runtime.ui_attached = true;
    assert!(event(
        &mut runtime,
        "detail_open",
        serde_json::json!({"item": {"kind": "remains", "workspace_id": "project", "checkout_id": "emails"}})
    ));
    let request = runtime.remains_request().0.expect("readable");
    let answer = |reason: Option<&str>, files: usize| crate::changes::ChangesAnswer {
        key: Some(request.key()),
        detail: None,
        selection: (None, false),
        changes: crate::model::ChangesSnapshot {
            root_path: Some("/fixture/emails".into()),
            entries: (0..files)
                .map(|n| crate::model::ChangedFileSnapshot {
                    path: format!("/fixture/emails/f{n}.rs"),
                    relative_path: format!("f{n}.rs"),
                    previous_relative_path: None,
                    status: crate::model::ChangedFileStatus::Untracked,
                    added_lines: None,
                    removed_lines: None,
                })
                .collect(),
            unavailable_reason: reason.map(str::to_owned),
            ..Default::default()
        },
    };
    assert!(runtime.ingest_remains(answer(None, 2)));
    assert!(runtime.ingest_remains(answer(Some("link.not_connected"), 0)));
    let held = &runtime.snapshot.remains.as_ref().expect("held").changes;
    assert_eq!(
        (
            held.entries.len(),
            held.stale_reason.as_deref(),
            held.unavailable_reason.as_deref()
        ),
        (2, Some("link.not_connected"), None)
    );

    runtime.file_roots = Some(crate::files::FileRoots::from_identities(Vec::new()));
    assert_eq!(runtime.remains_request(), (None, true));
    let refused = &runtime
        .snapshot
        .remains
        .as_ref()
        .expect("published")
        .changes;
    assert_eq!(
        refused.unavailable_reason.as_deref(),
        Some("host.root_not_opened")
    );
    assert_eq!(
        runtime.remains_request(),
        (None, false),
        "a refusal already shown is not published again"
    );
}
