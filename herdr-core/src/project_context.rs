//! Derived project reading order and current checkout context. No I/O or clock.
use std::collections::{HashMap, HashSet};

use crate::agent_sleep::SleepingSessionSnapshot;
use crate::agent_state::RowMark;
use crate::model::{
    CheckoutPaneContext, CheckoutSnapshot, InactiveCheckoutGroupSnapshot,
    InactiveProjectGroupSnapshot, NavigatorSnapshot, ProjectAgentPresenceSnapshot,
    ProjectAsleepSnapshot, SidebarAgentSnapshot, UiStateSnapshot, WorkspaceSnapshot,
};

const INACTIVE_AFTER_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// Wall time and server sequence are separate domains. A sequence is only a
/// tie-breaker, never interpreted as milliseconds. Missing facts remain absent.
#[derive(Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
struct Activity {
    unix_ms: Option<u64>,
    sequence: Option<u64>,
}

fn checkout_activity(
    checkout: &CheckoutSnapshot,
    by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
) -> Activity {
    let mut activity = Activity {
        unix_ms: checkout
            .worktree
            .as_ref()
            .and_then(|worktree| worktree.last_commit_unix_seconds)
            .and_then(|seconds| seconds.checked_mul(1_000)),
        sequence: None,
    };
    for pane in checkout.tabs.iter().flat_map(|tab| &tab.panes) {
        if let Some(agent) = by_pane.get(pane.id.as_str()) {
            // The label worker's wall timestamp has 13 digits. Its padded
            // sequence is deliberately not interpreted as a date.
            let timestamp = (agent.last_activity.len() == 13)
                .then(|| agent.last_activity.parse::<u64>().ok())
                .flatten();
            activity.unix_ms = activity.unix_ms.max(timestamp);
            activity.sequence = activity.sequence.max(agent.state_change_seq);
        }
    }
    activity
}

pub(crate) fn sort_projects(
    projects: &mut [WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
) -> bool {
    let by_pane: HashMap<_, _> = agents
        .iter()
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect();
    let before: Vec<_> = projects
        .iter()
        .map(|project| {
            (
                project.id.clone(),
                project.last_activity_unix_ms,
                project
                    .checkouts
                    .iter()
                    .map(|c| c.id.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let mut project_activity = HashMap::new();
    for project in projects.iter_mut() {
        let mut keys = HashMap::new();
        let mut recent = Activity::default();
        for checkout in &project.checkouts {
            let key = checkout_activity(checkout, &by_pane);
            recent = recent.max(key);
            keys.insert(checkout.id.clone(), key);
        }
        // The primary checkout leads its project whatever its activity: it is
        // the one checkout that never folds, so the list starts at the same
        // place every time. The rest follow by activity.
        project.checkouts.sort_by(|left, right| {
            right
                .is_primary
                .cmp(&left.is_primary)
                .then_with(|| keys[&right.id].cmp(&keys[&left.id]))
                .then_with(|| left.id.cmp(&right.id))
        });
        // The order's own reason, carried to the shell so the row can show it.
        // Only the wall time travels: a server sequence is a tie-breaker, not
        // a date, and rendering it as one would invent recency.
        project.last_activity_unix_ms = recent.unix_ms;
        project_activity.insert(project.id.clone(), recent);
    }
    // A pinned project stays ahead of its device's unpinned ones whatever
    // its activity, so the shell can split the list into the `Pinned`
    // section and the activity list without repeating either key (D-02,
    // D-03). The device is still the first key: pins never cross devices.
    projects.sort_by(|left, right| {
        left.device_id
            .cmp(&right.device_id)
            .then_with(|| right.pinned.cmp(&left.pinned))
            .then_with(|| project_activity[&right.id].cmp(&project_activity[&left.id]))
            .then_with(|| left.id.cmp(&right.id))
    });
    projects
        .iter()
        .zip(before)
        .any(|(project, (id, last_activity_unix_ms, checkouts))| {
            project.id != id
                || project.last_activity_unix_ms != last_activity_unix_ms
                || project.checkouts.iter().map(|c| &c.id).ne(checkouts.iter())
        })
}

fn checkout_has_live_exception(
    checkout: &CheckoutSnapshot,
    focused_checkout_id: Option<&str>,
) -> bool {
    crate::agent_state::checkout_has_active_agents(&checkout.agent_summary)
        || checkout.dirty
        || checkout
            .unpushed
            .as_ref()
            .is_some_and(|unpushed| unpushed.count > 0)
        || Some(checkout.id.as_str()) == focused_checkout_id
}

fn checkout_is_inactive(
    checkout: &CheckoutSnapshot,
    by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
    focused_checkout_id: Option<&str>,
    now_unix_ms: u64,
) -> bool {
    if checkout_has_live_exception(checkout, focused_checkout_id) {
        return false;
    }
    let settled = checkout.landed
        || checkout
            .pull_request
            .as_ref()
            .is_some_and(|pull_request| pull_request.badge.is_settled());
    let stale = checkout_activity(checkout, by_pane)
        .unix_ms
        .is_some_and(|last| now_unix_ms.saturating_sub(last) > INACTIVE_AFTER_MS);
    settled || stale
}

/// The `session_open_folds` key that holds a device's No agents fold open.
pub(crate) fn no_agents_fold_key(device_id: &str) -> String {
    format!("no_agents/{device_id}")
}

/// The `session_open_folds` key that holds a sleeping project open until
/// one of its agents wakes (D-19).
pub(crate) fn asleep_fold_key(project_id: &str) -> String {
    format!("asleep/{project_id}")
}

/// What a project's agents are doing, read from its checkouts' panes and
/// the sleeping conversations kept for its checkouts' folders (PRD
/// sidebar-titlebar-refresh D-02, D-03). An awake agent whose mark says
/// something is moving (`RowMark::of` is neither quiet nor unknown) makes the
/// project working; a resting agent, or one Hide cannot read, does not. A live row Hide holds asleep counts as sleeping, not as awake,
/// because its badge does not count it.
pub(crate) fn agent_presence(
    workspace: &WorkspaceSnapshot,
    by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
    sleeping: &[SleepingSessionSnapshot],
    open_folds: &[String],
) -> (ProjectAgentPresenceSnapshot, bool) {
    let mut awake = 0usize;
    let mut working = false;
    let mut asleep_since: Option<u64> = None;
    for agent in workspace
        .checkouts
        .iter()
        .flat_map(|checkout| &checkout.tabs)
        .flat_map(|tab| &tab.panes)
        .filter_map(|pane| by_pane.get(pane.id.as_str()))
    {
        if let Some(sleep) = &agent.sleep {
            asleep_since = asleep_since.max(Some(sleep.since_unix_ms));
            continue;
        }
        awake += 1;
        working |= !matches!(RowMark::of(agent), RowMark::Quiet | RowMark::Unknown);
    }
    for session in sleeping.iter().filter(|session| {
        session.node_id == workspace.device_id
            && workspace
                .checkouts
                .iter()
                .any(|checkout| checkout.path == session.checkout_path)
    }) {
        asleep_since = asleep_since.max(Some(session.since_unix_ms));
    }
    let has_rows = awake > 0 || asleep_since.is_some();
    let asleep = asleep_since
        .filter(|_| awake == 0)
        .map(|since_unix_ms| ProjectAsleepSnapshot {
            since_unix_ms,
            expanded: open_folds.contains(&asleep_fold_key(&workspace.id)),
        });
    (ProjectAgentPresenceSnapshot { working, asleep }, has_rows)
}

/// A project's Inactive fold: its checkouts, other than the primary one,
/// that nothing works in now (`checkout_is_inactive`), open as the operator
/// left it. This machine's projects and those of a node that dials the core
/// share it (PRD core-host-node-move D-29).
pub(crate) fn inactive_checkout_group(
    workspace: &WorkspaceSnapshot,
    by_pane: &HashMap<&str, &SidebarAgentSnapshot>,
    focused_checkout_id: Option<&str>,
    expanded: bool,
    now_unix_ms: u64,
) -> InactiveCheckoutGroupSnapshot {
    InactiveCheckoutGroupSnapshot {
        expanded,
        checkout_ids: workspace
            .checkouts
            .iter()
            .filter(|checkout| {
                !checkout.is_primary
                    && checkout_is_inactive(checkout, by_pane, focused_checkout_id, now_unix_ms)
            })
            .map(|checkout| checkout.id.clone())
            .collect(),
    }
}

/// Rebuilds core-owned inactive membership without moving the authoritative
/// project or checkout rows. The shell maps these ordered IDs back to the
/// existing rows, while search and focus continue to see the full collections.
pub(crate) fn refresh_inactive_groups(
    navigator: &mut NavigatorSnapshot,
    ui_state: &UiStateSnapshot,
    now_unix_ms: u64,
) -> bool {
    let before_projects = navigator.inactive_projects.clone();
    let before_no_agents = navigator.no_agent_projects.clone();
    let before_checkouts = navigator
        .workspaces
        .iter()
        .map(|workspace| {
            (
                workspace.id.clone(),
                workspace.inactive_checkouts.clone(),
                workspace.session_folds.clone(),
                workspace.agent_presence.clone(),
            )
        })
        .collect::<Vec<_>>();
    let by_pane: HashMap<_, _> = navigator
        .agents
        .iter()
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect();
    let focused_checkout_id = navigator.focused_checkout_id.as_deref();
    let expanded_checkout_projects = ui_state
        .expanded_inactive_checkout_project_paths
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expanded_project_devices = ui_state
        .expanded_inactive_project_device_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    let mut inactive_projects = Vec::<InactiveProjectGroupSnapshot>::new();
    let mut no_agent_projects = Vec::<InactiveProjectGroupSnapshot>::new();
    let sleeping = &navigator.sleeping_sessions;
    for workspace in &mut navigator.workspaces {
        workspace.inactive_checkouts = inactive_checkout_group(
            workspace,
            &by_pane,
            focused_checkout_id,
            expanded_checkout_projects.contains(workspace.path.as_str()),
            now_unix_ms,
        );
        refresh_session_folds(workspace, &by_pane, ui_state, focused_checkout_id);
        workspace
            .inactive_checkouts
            .checkout_ids
            .retain(|id| !workspace.session_folds.cleanup.contains(id));

        // A pinned project is exempt from the device fold: the operator asked
        // to see it whatever its activity (D-04). Its own checkouts still
        // fold above.
        let project_is_inactive = !workspace.pinned
            && !workspace.checkouts.is_empty()
            && workspace.checkouts.iter().all(|checkout| {
                checkout_is_inactive(checkout, &by_pane, focused_checkout_id, now_unix_ms)
            });
        let (presence, has_rows) =
            agent_presence(workspace, &by_pane, sleeping, &ui_state.session_open_folds);
        workspace.agent_presence = presence;
        let in_front = focused_checkout_id.is_some_and(|focused| {
            workspace
                .checkouts
                .iter()
                .any(|checkout| checkout.id == focused)
        });
        if project_is_inactive {
            let expanded = expanded_project_devices.contains(workspace.device_id.as_str());
            add_to_group(&mut inactive_projects, workspace, expanded);
        } else if !has_rows
            && !workspace.checkouts.iter().any(holds_named_tab)
            && !workspace.pinned
            && !workspace.is_home
            && !in_front
        {
            // A project with no agent and no sleeping conversation waits in
            // its device's No agents fold (D-01); the Inactive fold keeps
            // the projects nothing has touched for a week. A tab the
            // operator named and left without agents is one they kept.
            let expanded = ui_state
                .session_open_folds
                .contains(&no_agents_fold_key(&workspace.device_id));
            add_to_group(&mut no_agent_projects, workspace, expanded);
        }
    }
    navigator.inactive_projects = inactive_projects;
    navigator.no_agent_projects = no_agent_projects;

    navigator.inactive_projects != before_projects
        || navigator.no_agent_projects != before_no_agents
        || navigator.workspaces.iter().zip(before_checkouts).any(
            |(workspace, (id, group, session_folds, presence))| {
                workspace.id != id
                    || workspace.inactive_checkouts != group
                    || workspace.session_folds != session_folds
                    || workspace.agent_presence != presence
            },
        )
}

/// Adds a project to its device's group, in the list's order.
fn add_to_group(
    groups: &mut Vec<InactiveProjectGroupSnapshot>,
    workspace: &WorkspaceSnapshot,
    expanded: bool,
) {
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group.device_id == workspace.device_id)
    {
        group.project_ids.push(workspace.id.clone());
    } else {
        groups.push(InactiveProjectGroupSnapshot {
            device_id: workspace.device_id.clone(),
            expanded,
            project_ids: vec![workspace.id.clone()],
        });
    }
}

/// The folds of a device session's project, the one place they are
/// decided for a device: the session folds, and the Inactive fold when the
/// device is a node that dials this core (`node`), whose pane-less
/// worktrees are rows as this machine's are (D-29). A device this core
/// dials shows only the checkouts its session opens, so it has no Inactive
/// fold. The derive and the time-based pass both call it, so a published
/// session and a newly derived one compare like with like.
/// The checkout a device session's folds treat as in front: the window's
/// selection when it is one of the session's checkouts, as a local
/// project's folds read it, else the device Herdr's own focus.
pub(crate) fn device_fold_focus(
    window: Option<&str>,
    session: &crate::model::RemoteSessionSnapshot,
) -> Option<String> {
    window
        .filter(|id| {
            session.workspaces.iter().any(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.id == *id)
            })
        })
        .or(session.focused_checkout_id.as_deref())
        .map(str::to_owned)
}

pub(crate) fn refresh_device_folds(
    workspace: &mut WorkspaceSnapshot,
    agents: &HashMap<&str, &SidebarAgentSnapshot>,
    sleeping: &[SleepingSessionSnapshot],
    ui_state: &UiStateSnapshot,
    focused_checkout_id: Option<&str>,
    node: bool,
    now_unix_ms: u64,
) {
    workspace.agent_presence =
        agent_presence(workspace, agents, sleeping, &ui_state.session_open_folds).0;
    if node {
        workspace.inactive_checkouts = inactive_checkout_group(
            workspace,
            agents,
            focused_checkout_id,
            ui_state
                .expanded_inactive_checkout_project_paths
                .contains(&workspace.path),
            now_unix_ms,
        );
    }
    refresh_session_folds(workspace, agents, ui_state, focused_checkout_id);
}

/// The checkout holds a tab the operator named, which stays on its line
/// with or without agents (docs/UI_BEHAVIOR.md, Sidebar hierarchy).
fn holds_named_tab(checkout: &CheckoutSnapshot) -> bool {
    checkout
        .tabs
        .iter()
        .any(|tab| tab.naming.operator_name().is_some() && !tab.delegated)
}

pub(crate) fn refresh_session_folds(
    workspace: &mut WorkspaceSnapshot,
    agents: &HashMap<&str, &SidebarAgentSnapshot>,
    ui_state: &UiStateSnapshot,
    focused_checkout_id: Option<&str>,
) {
    let mut folds = crate::model::SessionCheckoutFolds {
        empty_open: ui_state.session_open_folds.contains(&workspace.id),
        cleanup_open: ui_state
            .session_open_folds
            .contains(&format!("cleanup/{}", workspace.device_id)),
        ..Default::default()
    };
    for checkout in &workspace.checkouts {
        let subagent = std::path::Path::new(&checkout.path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("worktree-agent-"));
        if !checkout.exists || subagent {
            folds.cleanup.push(checkout.id.clone());
            continue;
        }
        let unresolved = || {
            checkout
                .tabs
                .iter()
                .flat_map(|tab| &tab.panes)
                .filter_map(|pane| agents.get(pane.id.as_str()))
                .filter(|agent| agent.resolved.is_none())
        };
        let live = unresolved().next().is_some();
        // A worktree that only delegated children work in is the parent's
        // tree, not a line of its own: the children already reach the screen
        // as the descendant mark on the parent root's row. A Factory worker
        // is not delegated here, because its spawn parent is the Factory's
        // code-owned identity, which is no pane in the list, so the worker
        // resolves as an orphan root.
        let child_only = live && !unresolved().any(|agent| agent.state.root);
        if checkout.is_worktree
            && !checkout.is_primary
            && !workspace
                .inactive_checkouts
                .checkout_ids
                .contains(&checkout.id)
            && !checkout.dirty
            && checkout
                .unpushed
                .as_ref()
                .is_none_or(|value| value.count == 0)
            && focused_checkout_id != Some(checkout.id.as_str())
        {
            if !live && !holds_named_tab(checkout) {
                folds.empty.push(checkout.id.clone());
                folds.open_prs += usize::from(
                    checkout
                        .pull_request
                        .as_ref()
                        .is_some_and(|pr| !pr.badge.is_settled()),
                );
            } else if child_only {
                folds.child_only.push(checkout.id.clone());
            }
        }
    }
    workspace.session_folds = folds;
}

pub(crate) fn checkout_panes(
    checkout: &CheckoutSnapshot,
    projects: &[WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
) -> Vec<CheckoutPaneContext> {
    let live: HashSet<_> = projects
        .iter()
        .flat_map(|w| &w.checkouts)
        .flat_map(|c| &c.tabs)
        .flat_map(|t| &t.panes)
        .map(|p| p.id.as_str())
        .collect();
    let agents: HashMap<_, _> = agents.iter().map(|a| (a.pane_id.as_str(), a)).collect();
    checkout
        .tabs
        .iter()
        .flat_map(|tab| {
            tab.panes.iter().map(|pane| {
                let agent = agents.get(pane.id.as_str());
                CheckoutPaneContext {
                    pane_id: pane.id.clone(),
                    title: pane
                        .herdr_label
                        .as_ref()
                        .or(pane.terminal_title.as_ref())
                        .or(pane.identity_label.as_ref())
                        .cloned()
                        .unwrap_or_else(|| pane.id.clone()),
                    status: pane.status_code.english().to_owned(),
                    session_id: agent.and_then(|a| a.session_id.clone()),
                    parent_pane_id: agent
                        .and_then(|a| a.spawned_from_pane_id.as_ref())
                        .filter(|id| live.contains(id.as_str()))
                        .cloned(),
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::model::{
        CoreOptions, NavigatorSnapshot, PaneSnapshot, PullRequestBadge, PullRequestSnapshot,
        SCHEMA_VERSION, Snapshot, TabSnapshot, UnpushedSnapshot, WorktreeSnapshot,
    };

    /// A project row is a project id, the commit time of its one checkout, and
    /// the panes an agent may be running in. Everything else the ordering does
    /// not read stays at its default.
    fn project(
        id: &str,
        device_id: &str,
        last_commit_unix_seconds: Option<u64>,
        pane_ids: &[&str],
    ) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            agent_presence: Default::default(),
            agent_scope: Default::default(),
            home_issues: Default::default(),
            pull_requests: Vec::new(),
            tasks: Default::default(),
            id: id.to_owned(),
            label: id.to_owned(),
            path: format!("/fixture/{id}"),
            remote_target_id: None,
            expanded: true,
            device_id: device_id.to_owned(),
            repo_name: id.to_owned(),
            is_git: true,
            default_branch: None,
            branches: Vec::new(),
            registered: true,
            temporary: false,
            session_workspace_ids: Vec::new(),
            last_activity_unix_ms: None,
            checkouts: vec![checkout(id, last_commit_unix_seconds, pane_ids)],
            pinned: false,
            is_home: false,
            inactive_checkouts: InactiveCheckoutGroupSnapshot::default(),
            session_folds: Default::default(),
            removal: Default::default(),
            disk: Default::default(),
            cleanup: None,
        }
    }

    fn checkout(
        id: &str,
        last_commit_unix_seconds: Option<u64>,
        pane_ids: &[&str],
    ) -> CheckoutSnapshot {
        CheckoutSnapshot {
            agent_scope: Default::default(),
            id: id.to_owned(),
            is_primary: true,
            exists: true,
            worktree: last_commit_unix_seconds.map(|seconds| WorktreeSnapshot {
                last_commit_unix_seconds: Some(seconds),
                ..Default::default()
            }),
            tabs: vec![TabSnapshot {
                agent: None,
                naming: Default::default(),
                id: None,
                workspace_id: None,
                checkout_id: None,
                label: None,
                empty: pane_ids.is_empty(),
                delegated: false,
                panes: pane_ids
                    .iter()
                    .map(|pane_id| PaneSnapshot {
                        terminal_id: None,
                        failed_terminal_id: None,
                        id: (*pane_id).to_owned(),
                        herdr_label: None,
                        terminal_title: None,
                        cwd: "/fixture".to_owned(),
                        status_code: crate::model::AgentStatusCode::Unknown,
                        requires_close_confirmation: false,
                        requires_close_status_check: false,
                        identity_label: None,
                        activity_at_unix_ms: None,
                        fork: Default::default(),
                        ports: vec![],
                        servers: vec![],
                        children: None,
                        lineage_path: Vec::new(),
                        sleep: None,
                        sleep_action: None,
                    })
                    .collect(),
            }],
            ..Default::default()
        }
    }

    /// Agents come through the real projection rather than a handwritten
    /// struct, so the tests read the same 13-digit activity token the sidebar
    /// validates.
    fn agents(rows: Value) -> Vec<SidebarAgentSnapshot> {
        crate::sidebar::project_agents(
            crate::sidebar::owned_label_fixture(json!({ "agents": rows })).expect("valid fixture"),
        )
        .agents
    }

    fn ids(projects: &[WorkspaceSnapshot]) -> Vec<&str> {
        projects.iter().map(|p| p.id.as_str()).collect()
    }

    fn navigator(workspaces: Vec<WorkspaceSnapshot>) -> NavigatorSnapshot {
        let mut navigator = Snapshot::initial(&CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            herdr_socket_path: None,
            herdr_bin_path: None,
            app_state_path: "/tmp/hide-project-context-test-state.json".to_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        })
        .navigator;
        navigator.workspaces = workspaces;
        navigator
    }

    fn make_worktree_checkout(
        workspace_id: &str,
        id: &str,
        last_commit_unix_seconds: Option<u64>,
    ) -> CheckoutSnapshot {
        CheckoutSnapshot {
            agent_scope: Default::default(),
            id: id.to_owned(),
            workspace_id: workspace_id.to_owned(),
            label: id.to_owned(),
            path: format!("/fixture/{workspace_id}/{id}"),
            is_worktree: true,
            exists: true,
            worktree: Some(WorktreeSnapshot {
                last_commit_unix_seconds,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn settled_pull_request(badge: PullRequestBadge) -> PullRequestSnapshot {
        PullRequestSnapshot {
            closing_issues: Default::default(),
            title: "Settled".to_owned(),
            checks: Default::default(),
            number: 1,
            head_branch: "topic".to_owned(),
            base_branch: "main".to_owned(),
            url: "https://example.invalid/pull/1".to_owned(),
            badge,
            review: None,
            is_draft: false,
            merged_at_unix_ms: None,
            updated_at_unix_ms: None,
            created_at_unix_ms: None,
            closed_at_unix_ms: None,
            head_oid: None,
            cross_repository: false,
            work: Default::default(),
        }
    }

    /// B1, B2. The newest activity in a project comes first inside its device
    /// group, whether that activity is a commit or an agent, and the carried
    /// timestamp is the same one the order was decided by.
    #[test]
    fn projects_order_by_newest_activity_within_a_device() {
        let mut projects = vec![
            project("stale", "local", Some(1_000), &[]),
            project("committed", "local", Some(3_000), &[]),
            project("agent", "local", Some(2_000), &["pane-agent"]),
        ];
        let agents = agents(json!([
            {"pane_id": "pane-agent", "state_change_seq": 1, "tokens": {"activity": "0000004000000"}},
        ]));

        assert!(sort_projects(&mut projects, &agents));

        assert_eq!(ids(&projects), ["agent", "committed", "stale"]);
        assert_eq!(projects[0].last_activity_unix_ms, Some(4_000_000));
        assert_eq!(projects[1].last_activity_unix_ms, Some(3_000_000));
        assert_eq!(projects[2].last_activity_unix_ms, Some(1_000_000));
    }

    /// B1. Equal activity keeps the existing order, which is the project id.
    /// A tie is not an invitation to shuffle the list on every projection.
    #[test]
    fn equal_activity_falls_back_to_the_project_id() {
        let mut projects = vec![
            project("beta", "local", Some(2_000), &[]),
            project("alpha", "local", Some(2_000), &[]),
        ];

        assert!(sort_projects(&mut projects, &[]));
        assert_eq!(ids(&projects), ["alpha", "beta"]);

        // Already ordered: the same input reports no change, so an idle tick
        // publishes nothing.
        assert!(!sort_projects(&mut projects, &[]));
    }

    /// B1, B4. A project with neither a commit nor an agent has no activity to
    /// claim. It sorts below every project that has one and carries no
    /// timestamp, which is what lets the row leave its time blank.
    #[test]
    fn projects_without_activity_sort_last_and_carry_no_time() {
        let mut projects = vec![
            project("quiet", "local", None, &[]),
            project("active", "local", Some(5_000), &[]),
        ];

        assert!(sort_projects(&mut projects, &[]));

        assert_eq!(ids(&projects), ["active", "quiet"]);
        assert_eq!(projects[0].last_activity_unix_ms, Some(5_000_000));
        assert_eq!(projects[1].last_activity_unix_ms, None);
    }

    /// The primary checkout leads its project even when a linked worktree
    /// moved more recently; the linked ones keep their activity order, and
    /// the project still carries the newest activity of any of them.
    #[test]
    fn primary_checkout_leads_its_project_whatever_its_activity() {
        let mut herdr = project("herdr", "local", Some(1_000), &[]);
        herdr.checkouts[0].path = herdr.path.clone();
        herdr
            .checkouts
            .push(make_worktree_checkout("herdr", "older", Some(2_000)));
        herdr
            .checkouts
            .push(make_worktree_checkout("herdr", "newer", Some(3_000)));
        let mut projects = vec![herdr];

        assert!(sort_projects(&mut projects, &[]));

        let order: Vec<_> = projects[0]
            .checkouts
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(order, ["herdr", "newer", "older"]);
        assert_eq!(projects[0].last_activity_unix_ms, Some(3_000_000));
    }

    /// Session B7 preserves existing inactive membership. Only the remaining
    /// agentless worktrees enter the new fold, shown above Inactive.
    #[test]
    fn agentless_worktrees_preserve_legacy_inactivity_membership() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let old_seconds = (now_ms - (7 * 24 + 1) * 60 * 60 * 1_000) / 1_000;
        let recent_seconds = (now_ms - (6 * 24 + 23) * 60 * 60 * 1_000) / 1_000;
        let mut project = project("alpha", "local", Some(old_seconds), &[]);
        project.checkouts[0].path = project.path.clone();
        project.checkouts[0].worktree.as_mut().unwrap().merged = Some(true);

        let mut merged = make_worktree_checkout("alpha", "merged", Some(recent_seconds));
        merged.worktree.as_mut().unwrap().merged = Some(true);
        merged.landed = true;
        let mut untouched = make_worktree_checkout("alpha", "untouched", Some(recent_seconds));
        untouched.worktree.as_mut().unwrap().merged = Some(true);
        let stale = make_worktree_checkout("alpha", "stale", Some(old_seconds));
        let recent = make_worktree_checkout("alpha", "recent", Some(recent_seconds));
        let unknown = make_worktree_checkout("alpha", "unknown", None);
        let mut closed = make_worktree_checkout("alpha", "closed", Some(recent_seconds));
        closed.pull_request = Some(settled_pull_request(PullRequestBadge::Closed));
        project
            .checkouts
            .extend([merged, untouched, stale, recent, unknown, closed]);
        let mut navigator = navigator(vec![project]);

        assert!(refresh_inactive_groups(
            &mut navigator,
            &UiStateSnapshot::default(),
            now_ms
        ));

        assert_eq!(
            navigator.workspaces[0].session_folds.empty,
            ["untouched", "recent", "unknown"]
        );
        assert_eq!(
            navigator.workspaces[0].inactive_checkouts.checkout_ids,
            ["merged", "stale", "closed"]
        );
        assert!(
            !navigator.workspaces[0]
                .inactive_checkouts
                .checkout_ids
                .contains(&"alpha".to_owned()),
            "the primary never folds inside its project"
        );
        assert!(navigator.inactive_projects.is_empty());
    }

    /// D-06, B13. A recent agent timestamp keeps a checkout active even when
    /// its commit is old; the predicate uses the same newest-activity answer
    /// as project ordering.
    #[test]
    fn recent_agent_activity_overrides_an_old_commit() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let old_seconds = (now_ms - 8 * 24 * 60 * 60 * 1_000) / 1_000;
        let recent_ms = now_ms - 2 * 24 * 60 * 60 * 1_000;
        let mut project = project("alpha", "local", Some(old_seconds), &[]);
        project.checkouts[0].path = project.path.clone();
        project.checkouts.push(make_worktree_checkout(
            "alpha",
            "recent-agent",
            Some(old_seconds),
        ));
        project.checkouts[1].tabs = checkout("ignored", None, &["pane-active"]).tabs;
        let mut navigator = navigator(vec![project]);
        navigator.agents = agents(json!([
            {"pane_id": "pane-active", "state_change_seq": 1, "tokens": {"activity": format!("{recent_ms:013}")}},
        ]));

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        assert!(
            navigator.workspaces[0]
                .inactive_checkouts
                .checkout_ids
                .is_empty()
        );
    }

    /// B3, B7. Every live-work exception wins over a settled branch, including
    /// the operator's current selection.
    #[test]
    fn dirty_unpushed_and_focused_checkouts_stay_out_of_the_agentless_fold() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let mut project = project("alpha", "local", None, &[]);
        let ids = [
            "working",
            "needs-you",
            "dirty",
            "unpushed",
            "focused",
            "control",
        ];
        for id in ids {
            let mut checkout = make_worktree_checkout("alpha", id, None);
            checkout.worktree.as_mut().unwrap().merged = Some(true);
            checkout.landed = true;
            project.checkouts.push(checkout);
        }
        project.checkouts[1].agent_summary.working = 1;
        project.checkouts[2].agent_summary.needs_you = 1;
        project.checkouts[1].tabs = checkout("working", None, &["working-pane"]).tabs;
        project.checkouts[2].tabs = checkout("needs-you", None, &["blocked-pane"]).tabs;
        project.checkouts[3].dirty = true;
        project.checkouts[4].unpushed = Some(UnpushedSnapshot {
            remote: "origin".to_owned(),
            count: 1,
        });
        let mut navigator = navigator(vec![project]);
        navigator.focused_checkout_id = Some("focused".to_owned());
        navigator.agents = agents(json!([
            {"pane_id": "working-pane", "agent_status": "working", "state_change_seq": 1},
            {"pane_id": "blocked-pane", "agent_status": "blocked", "state_change_seq": 1}
        ]));

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        assert!(navigator.workspaces[0].session_folds.empty.is_empty());
        assert_eq!(
            navigator.workspaces[0].inactive_checkouts.checkout_ids,
            ["control"]
        );
    }

    /// A tab the operator named and left without agents is one they kept:
    /// its worktree keeps its line and its project stays out of No agents.
    /// The same name given by a spawn keeps neither.
    #[test]
    fn a_tab_the_operator_named_keeps_its_worktree_and_project_out_of_no_agents() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let recent = Some(now_ms / 1_000);
        let named = |by_operator| crate::model::TabNaming {
            raw: "Planning".to_owned(),
            by_operator,
            ..Default::default()
        };
        let mut kept = project("kept", "local", recent, &["kept-shell"]);
        kept.checkouts[0].tabs[0].naming = named(true);
        let mut spawned = project("spawned", "local", recent, &["spawned-shell"]);
        spawned.checkouts[0].tabs[0].naming = named(false);
        let mut alpha = project("alpha", "local", recent, &[]);
        alpha.pinned = true;
        for (id, by_operator) in [("kept-tree", true), ("spawned-tree", false)] {
            let mut tree = make_worktree_checkout("alpha", id, None);
            tree.tabs = checkout(id, None, &["shell"]).tabs;
            tree.tabs[0].naming = named(by_operator);
            alpha.checkouts.push(tree);
        }
        let mut navigator = navigator(vec![kept, spawned, alpha]);

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        let no_agents: Vec<_> = navigator
            .no_agent_projects
            .iter()
            .flat_map(|group| group.project_ids.clone())
            .collect();
        assert_eq!(no_agents, ["spawned"]);
        assert_eq!(
            navigator.workspaces[2].session_folds.empty,
            ["spawned-tree"]
        );
    }

    /// A worktree that only delegated children work in gets no checkout line
    /// and no fold: the children reach the screen as the descendant mark on
    /// the root that owns them. Every other case keeps its line - a root of
    /// its own, an orphan whose parent ended, a Factory worker (its spawn
    /// parent is no pane in the list) and the live-work exceptions.
    #[test]
    fn worktrees_with_only_delegated_children_are_hidden() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let mut project = project("alpha", "local", None, &[]);
        let panes = [
            ("owner", "root-pane"),
            ("kids", "child-pane"),
            ("orphan", "orphan-pane"),
            ("factory", "worker-pane"),
            ("kids-dirty", "dirty-child-pane"),
            ("kids-focused", "focused-child-pane"),
        ];
        for (id, pane) in panes {
            let mut worktree = make_worktree_checkout("alpha", id, None);
            worktree.tabs = checkout(id, None, &[pane]).tabs;
            project.checkouts.push(worktree);
        }
        project.checkouts[5].dirty = true;
        let mut navigator = navigator(vec![project]);
        navigator.focused_checkout_id = Some("kids-focused".to_owned());
        navigator.agents = agents(json!([
            {"pane_id": "root-pane", "agent_status": "working", "state_change_seq": 1},
            {"pane_id": "child-pane", "agent_status": "working", "state_change_seq": 1, "spawned_from_pane_id": "root-pane"},
            {"pane_id": "orphan-pane", "agent_status": "working", "state_change_seq": 1, "spawned_from_pane_id": "gone-pane"},
            {"pane_id": "worker-pane", "agent_status": "working", "state_change_seq": 1, "spawned_from_pane_id": "factory:one"},
            {"pane_id": "dirty-child-pane", "agent_status": "working", "state_change_seq": 1, "spawned_from_pane_id": "root-pane"},
            {"pane_id": "focused-child-pane", "agent_status": "working", "state_change_seq": 1, "spawned_from_pane_id": "root-pane"}
        ]));
        let workspaces = navigator.workspaces.clone();
        crate::agent_state::apply_lineage(&mut navigator.agents, &workspaces, &[]);

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        let folds = &navigator.workspaces[0].session_folds;
        assert_eq!(folds.child_only, ["kids"]);
        assert!(!folds.empty.contains(&"kids".to_owned()));
        assert!(
            navigator.workspaces[0]
                .inactive_checkouts
                .checkout_ids
                .is_empty()
        );
    }

    /// B11, B12, B16. A primary can make its whole project inactive even
    /// though it never enters the checkout fold. Device groups preserve the
    /// already sorted project order and read independent persisted expansion.
    #[test]
    fn fully_inactive_projects_group_by_device_in_existing_order() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let old_seconds = (now_ms - 8 * 24 * 60 * 60 * 1_000) / 1_000;
        let active = project("active", "local", Some(now_ms / 1_000), &[]);
        let mut local_one = project("local-one", "local", Some(old_seconds), &[]);
        local_one.checkouts[0].path = local_one.path.clone();
        let mut local_two = project("local-two", "local", Some(old_seconds), &[]);
        local_two.checkouts[0].path = local_two.path.clone();
        let mut remote = project("remote", "mini", Some(old_seconds), &[]);
        remote.checkouts[0].path = remote.path.clone();
        let mut navigator = navigator(vec![active, local_one, local_two, remote]);
        let ui_state = UiStateSnapshot {
            expanded_inactive_checkout_project_paths: vec!["/fixture/local-one".to_owned()],
            expanded_inactive_project_device_ids: vec!["local".to_owned()],
            ..UiStateSnapshot::default()
        };

        refresh_inactive_groups(&mut navigator, &ui_state, now_ms);

        assert_eq!(
            ids(&navigator.workspaces),
            ["active", "local-one", "local-two", "remote"]
        );
        assert_eq!(navigator.inactive_projects.len(), 2);
        assert_eq!(
            navigator.inactive_projects[0].project_ids,
            ["local-one", "local-two"]
        );
        assert!(navigator.inactive_projects[0].expanded);
        assert_eq!(navigator.inactive_projects[1].project_ids, ["remote"]);
        assert!(!navigator.inactive_projects[1].expanded);
        assert!(navigator.workspaces[1].inactive_checkouts.expanded);
        assert!(
            navigator.workspaces[1]
                .inactive_checkouts
                .checkout_ids
                .is_empty(),
            "the primary contributes only to the project-level fold"
        );
    }

    fn sleeping(device: &str, checkout_path: &str, since_unix_ms: u64) -> SleepingSessionSnapshot {
        SleepingSessionSnapshot {
            sleep_id: crate::agent_sleep::SleepId::new().expect("sleep id"),
            node_id: device.to_owned(),
            checkout_path: checkout_path.to_owned(),
            kind: "claude".to_owned(),
            identity_label: "asleep".to_owned(),
            phase: crate::agent_sleep::DormantPhase::Sleeping,
            since_unix_ms,
            reason: None,
            group: "idle",
            wake_available: true,
            checking: false,
        }
    }

    /// PRD sidebar-titlebar-refresh B1, B3. A project with no agent row and
    /// no sleeping conversation waits in its device's No agents fold, in the
    /// list's order and with the fold's open key; a pinned project, the
    /// project in front and a week-old project stay out of it. Only an agent
    /// with a mark other than the resting ring makes a project working.
    #[test]
    fn projects_without_agents_fold_and_only_marked_agents_weigh() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let recent = Some(now_ms / 1_000);
        let old = Some((now_ms - 8 * 24 * 60 * 60 * 1_000) / 1_000);
        let mut pinned = project("pinned", "local", recent, &[]);
        pinned.pinned = true;
        let mut navigator = navigator(vec![
            project("working", "local", recent, &["working-pane"]),
            project("resting", "local", recent, &["idle-pane"]),
            project("empty-one", "local", recent, &[]),
            pinned,
            project("front", "local", recent, &[]),
            project("empty-two", "local", recent, &[]),
            project("week-old", "local", old, &[]),
            project("device-empty", "mini", recent, &[]),
        ]);
        navigator.agents = agents(json!([
            {"pane_id": "working-pane", "agent_status": "working", "state_change_seq": 1},
            {"pane_id": "idle-pane", "agent_status": "idle", "state_change_seq": 1}
        ]));
        navigator.focused_checkout_id = Some("front".to_owned());
        let ui_state = UiStateSnapshot {
            session_open_folds: vec![no_agents_fold_key("local")],
            ..UiStateSnapshot::default()
        };

        assert!(refresh_inactive_groups(&mut navigator, &ui_state, now_ms));

        let groups: Vec<_> = navigator
            .no_agent_projects
            .iter()
            .map(|group| {
                (
                    group.device_id.as_str(),
                    group.expanded,
                    group.project_ids.clone(),
                )
            })
            .collect();
        assert_eq!(
            groups,
            [
                (
                    "local",
                    true,
                    vec!["empty-one".to_owned(), "empty-two".to_owned()]
                ),
                ("mini", false, vec!["device-empty".to_owned()]),
            ]
        );
        assert_eq!(navigator.inactive_projects[0].project_ids, ["week-old"]);
        let working: Vec<_> = navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.agent_presence.working)
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(working, ["working"]);
    }

    /// B4, B5. A project whose every agent sleeps carries the newest sleep
    /// time and starts folded; its open key opens it, an awake agent ends
    /// the sleeping state, and a sleeping project is not a No agents one.
    #[test]
    fn a_project_whose_agents_all_sleep_carries_its_newest_sleep() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let recent = Some(now_ms / 1_000);
        let mut asleep = project("asleep", "local", recent, &[]);
        asleep.checkouts[0].path = "/fixture/asleep".to_owned();
        let mut stirring = project("stirring", "local", recent, &["awake-pane"]);
        stirring.checkouts[0].path = "/fixture/stirring".to_owned();
        let mut navigator = navigator(vec![asleep, stirring]);
        navigator.agents = agents(json!([
            {"pane_id": "awake-pane", "agent_status": "idle", "state_change_seq": 1}
        ]));
        navigator.sleeping_sessions = vec![
            sleeping("local", "/fixture/asleep", 1_000),
            sleeping("local", "/fixture/asleep", 5_000),
            sleeping("mini", "/fixture/asleep", 9_000),
            sleeping("local", "/fixture/stirring", 2_000),
        ];

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        assert_eq!(
            navigator.workspaces[0].agent_presence.asleep,
            Some(ProjectAsleepSnapshot {
                since_unix_ms: 5_000,
                expanded: false
            })
        );
        assert_eq!(navigator.workspaces[1].agent_presence.asleep, None);
        assert!(navigator.no_agent_projects.is_empty());

        let opened = UiStateSnapshot {
            session_open_folds: vec![asleep_fold_key("asleep")],
            ..UiStateSnapshot::default()
        };
        assert!(refresh_inactive_groups(&mut navigator, &opened, now_ms));
        assert!(
            navigator.workspaces[0]
                .agent_presence
                .asleep
                .as_ref()
                .is_some_and(|asleep| asleep.expanded)
        );
    }

    /// B1. Device groups stay whole: activity orders projects inside a device,
    /// never across two of them.
    #[test]
    fn device_groups_are_ordered_before_activity() {
        let mut projects = vec![
            project("remote-old", "mini", Some(1_000), &[]),
            project("local-old", "local", Some(2_000), &[]),
            project("remote-new", "mini", Some(9_000), &[]),
            project("local-new", "local", Some(8_000), &[]),
        ];

        sort_projects(&mut projects, &[]);

        assert_eq!(
            ids(&projects),
            ["local-new", "local-old", "remote-new", "remote-old"]
        );
    }

    /// B2, B3. A pinned project leads its device whatever its activity, pins
    /// keep the activity order among themselves, and a remote pin still sits
    /// after every local row because the device is the first key.
    #[test]
    fn pinned_projects_lead_their_device_in_activity_order() {
        let mut pinned_stale = project("pinned-stale", "local", Some(1_000), &[]);
        pinned_stale.pinned = true;
        let mut pinned_busy = project("pinned-busy", "local", Some(2_000), &[]);
        pinned_busy.pinned = true;
        let mut remote_pinned = project("remote-pinned", "mini", Some(9_000), &[]);
        remote_pinned.pinned = true;
        let mut projects = vec![
            project("active", "local", Some(8_000), &[]),
            pinned_stale,
            remote_pinned,
            project("remote", "mini", Some(9_500), &[]),
            pinned_busy,
        ];

        assert!(sort_projects(&mut projects, &[]));

        assert_eq!(
            ids(&projects),
            [
                "pinned-busy",
                "pinned-stale",
                "active",
                "remote-pinned",
                "remote"
            ]
        );
    }

    /// B6. A pinned project whose every checkout is inactive stays out of the
    /// device fold; its own inactive worktrees still fold inside it.
    #[test]
    fn pinned_projects_stay_out_of_the_device_fold() {
        let now_ms = 20 * 24 * 60 * 60 * 1_000;
        let old_seconds = (now_ms - 8 * 24 * 60 * 60 * 1_000) / 1_000;
        let mut pinned = project("pinned", "local", Some(old_seconds), &[]);
        pinned.pinned = true;
        pinned.checkouts[0].path = pinned.path.clone();
        pinned.checkouts.push(make_worktree_checkout(
            "pinned",
            "old-topic",
            Some(old_seconds),
        ));
        let mut folded = project("folded", "local", Some(old_seconds), &[]);
        folded.checkouts[0].path = folded.path.clone();
        let mut navigator = navigator(vec![pinned, folded]);

        refresh_inactive_groups(&mut navigator, &UiStateSnapshot::default(), now_ms);

        assert_eq!(navigator.inactive_projects.len(), 1);
        assert_eq!(navigator.inactive_projects[0].project_ids, ["folded"]);
        assert!(navigator.workspaces[0].session_folds.empty.is_empty());
        assert_eq!(
            navigator.workspaces[0].inactive_checkouts.checkout_ids,
            ["old-topic"]
        );
    }

    /// B2. A pane's agent moving is enough to raise its project, and the move
    /// is reported so the snapshot publishes.
    #[test]
    fn new_agent_activity_raises_its_project_and_reports_the_change() {
        let mut projects = vec![
            project("busy", "local", Some(9_000), &[]),
            project("waking", "local", Some(1_000), &["pane-waking"]),
        ];
        sort_projects(&mut projects, &[]);
        assert_eq!(ids(&projects), ["busy", "waking"]);

        let awake = agents(json!([
            {"pane_id": "pane-waking", "state_change_seq": 2, "tokens": {"activity": "0000010000000"}},
        ]));

        assert!(sort_projects(&mut projects, &awake));
        assert_eq!(ids(&projects), ["waking", "busy"]);
        assert_eq!(projects[0].last_activity_unix_ms, Some(10_000_000));
    }
}
