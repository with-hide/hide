//! Projects the engine's session into the shell-facing session payload.

use super::*;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectedWorkspace {
    pub(crate) workspace_id: String,
    pub(crate) label: String,
    pub(crate) active_tab_id: String,
    pub(crate) tokens: BTreeMap<String, Value>,
    pub(crate) worktree: Option<ProjectedWorktree>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectedWorktree {
    pub(crate) repo_key: String,
    pub(crate) repo_name: String,
    pub(crate) repo_root: String,
    pub(crate) checkout_path: String,
    pub(crate) is_linked_worktree: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectedTab {
    pub(crate) number: u32,
    pub(crate) tab_id: String,
    pub(crate) workspace_id: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectedPane {
    pub terminal_id: String,
    pub restore_error: Option<String>,
    pub(crate) foreground_process: Option<String>,
    pub(crate) agent_status: String,
    pub(crate) pane_id: String,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) cwd: Option<String>,
    pub(crate) tokens: BTreeMap<String, Value>,
    pub(crate) label: Option<String>,
    /// The terminal's title without the activity glyph an agent turns in
    /// front of it, as the engine's stripped title reads it.
    pub(crate) terminal_title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectedAgent {
    pub(crate) pane_id: String,
    /// The terminal the agent runs in.
    pub(crate) terminal_id: String,
    pub(crate) name: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) cwd: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) agent_status: Option<String>,
    pub(crate) agent_session: Option<crate::sidebar::SessionAgentSessionPayload>,
    pub(crate) spawned_from_pane_id: Option<String>,
    pub(crate) spawned_from_machine_id: Option<String>,
    pub(crate) declared_parent_session: Option<String>,
    pub(crate) lineage_session: Option<String>,
    pub(crate) state_change_seq: u64,
    /// Present while the agent's current stop finished work, whoever has
    /// looked at it since: the engine's `done` turning `idle` when it marks the
    /// tab seen leaves it in place.
    pub(crate) completion_seq: Option<u64>,
    /// The engine still holds open the start it made of this agent
    /// (`agent.start`). It says, with no state reported yet, that the agent
    /// is starting (`agent_state::agent_starting`); it never reaches the shell.
    pub(crate) launch_pending: bool,
    /// The engine typed the start and its program has not shown yet
    /// (`SessionAgentPayload::start_unshown`).
    pub(crate) start_unshown: bool,
    pub(crate) tokens: BTreeMap<String, Value>,
}

/// An agent a restore did not bring back: its pane is a shell now, because
/// no resume could be typed or the one typed ended without the agent running
/// (its start's deadline passed with no program shown, or its program ended
/// before the start settled). The core ends the agent's registration
/// (`Runtime::end_unresumed`), so the row reads ended, never running with
/// nothing behind it.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct NotResumed {
    pub(crate) pane_id: String,
    /// The definition's id.
    pub(crate) agent: String,
    /// The conversation it ran, when the save knew it.
    pub(crate) session: Option<String>,
    /// Why it was not resumed, as the diagnostic says it.
    pub(crate) why: String,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(Default, serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ProjectionState {
    #[cfg_attr(test, serde(skip))]
    pub(crate) tab_focus: Option<crate::sidebar::SessionTabFocus>,
    /// Kept by the engine that published the session; a snapshot read on its own has none.
    #[cfg_attr(test, serde(skip))]
    pub(crate) tab_moves: Option<crate::sidebar::SessionTabMoves>,
    pub(crate) focused_pane_id: Option<String>,
    /// The engine's focused workspace, read from `session.snapshot` and kept
    /// current from the focus events. A `tab_focused` or `pane_focused` in
    /// another workspace moves it too, because the engine focuses the workspace
    /// along with the tab and does not always send `workspace_focused` first.
    pub(crate) focused_workspace_id: Option<String>,
    pub(crate) workspaces: Vec<ProjectedWorkspace>,
    pub(crate) tabs: Vec<ProjectedTab>,
    pub(crate) panes: Vec<ProjectedPane>,
    pub(crate) layouts: Vec<SessionLayoutPayload>,
    pub(crate) agents: Vec<ProjectedAgent>,
    /// The agents a restore left resting in their panes rather than start
    /// again, until an agent shows in the pane; the core keeps each as a
    /// sleeping agent (`Runtime::adopt_resting`). None is running, so none
    /// is among `agents`.
    pub(crate) resting: Vec<ProjectedAgent>,
    /// The agents a restore did not bring back and will not, each until an
    /// agent shows in its pane, by a start, its hook or its foreground; the
    /// core ends the registration of each while it is named
    /// (`Runtime::end_unresumed`). None runs, so none is among `agents`.
    pub(crate) not_resumed: Vec<NotResumed>,
    /// The next moment an agent start this read has pending (`launch_pending`)
    /// can end with nothing the engine reports changing: its settle passing
    /// while its screen already rests, or its deadline. The coordinator that
    /// publishes this read reads the session again then; no other read acts
    /// on it.
    #[cfg_attr(test, serde(skip))]
    pub(crate) settles_at: Option<std::time::Instant>,
}

impl ProjectionState {
    /// The agents a restore left resting, as the session's own agents
    /// project, so the core reads each the way it reads a listed one.
    pub(crate) fn resting_payloads(&self) -> Vec<SessionAgentPayload> {
        self.agent_payloads(&self.resting)
    }

    fn agent_payloads(&self, agents: &[ProjectedAgent]) -> Vec<SessionAgentPayload> {
        let workspace_labels = self
            .workspaces
            .iter()
            .map(|workspace| (workspace.workspace_id.as_str(), workspace.label.as_str()))
            .collect::<BTreeMap<_, _>>();
        agents
            .iter()
            .map(|agent| {
                let workspace_label = workspace_labels
                    .get(agent.workspace_id.as_str())
                    .copied()
                    .or_else(|| {
                        (!agent.workspace_id.trim().is_empty())
                            .then_some(agent.workspace_id.as_str())
                    })
                    .map(str::to_owned);
                SessionAgentPayload {
                    record: None,
                    hook: None,
                    telemetry_stale: false,
                    // Laid on from the pane by `project_agents`.
                    terminal_failed: false,
                    id: agent.name.clone().or_else(|| Some(agent.pane_id.clone())),
                    name: agent.name.clone(),
                    pane_id: Some(agent.pane_id.clone()),
                    workspace_label,
                    cwd: agent.cwd.clone(),
                    agent: agent.agent.clone(),
                    agent_status: agent.agent_status.clone(),
                    agent_session: agent.agent_session.clone(),
                    spawned_from_pane_id: agent.spawned_from_pane_id.clone(),
                    spawned_from_machine_id: agent.spawned_from_machine_id.clone(),
                    declared_parent_session: agent.declared_parent_session.clone(),
                    lineage_session: agent.lineage_session.clone(),
                    state_change_seq: Some(agent.state_change_seq),
                    completion_seq: agent.completion_seq,
                    launch_pending: agent.launch_pending,
                    start_unshown: agent.start_unshown,
                    tokens: agent.tokens.clone(),
                    // The coordinator's label worker lays these on.
                    label: None,
                    changed_at_unix_ms: None,
                    facts: None,
                    notification_pending: false,
                    notification_finalized: false,
                }
            })
            .collect()
    }

    pub(crate) fn project(&self) -> SessionSnapshotPayload {
        let agents = self.agent_payloads(&self.agents);
        let tabs = self
            .tabs
            .iter()
            .map(|tab| SessionTabPayload {
                number: tab.number,
                tab_id: tab.tab_id.clone(),
                workspace_id: tab.workspace_id.clone(),
                label: tab.label.clone(),
            })
            .collect();
        let panes = self
            .panes
            .iter()
            .map(|pane| SessionPanePayload {
                start_claim: pane
                    .label
                    .as_deref()
                    .filter(|label| crate::start_claim::is_reserved(label))
                    .map(str::to_owned),
                terminal_id: Some(pane.terminal_id.clone()),
                failed_terminal_id: pane
                    .restore_error
                    .as_ref()
                    .map(|_| pane.terminal_id.clone()),
                foreground_process: pane.foreground_process.clone(),
                pane_id: pane.pane_id.clone(),
                tokens: pane.tokens.clone(),
                cwd: pane.cwd.clone(),
                label: crate::start_claim::public_label(pane.label.as_deref()),
                terminal_title: crate::start_claim::public_label(pane.terminal_title.as_deref()),
            })
            .collect();
        // The engine order, not map order: the active tab id rides with the
        // workspace it belongs to, and the navigator reads it as the only
        // authority for which tab is active.
        let workspaces = self
            .workspaces
            .iter()
            .map(|workspace| SessionWorkspacePayload {
                workspace_id: workspace.workspace_id.clone(),
                worktree: workspace.worktree.as_ref().map(|tree| {
                    crate::domain::WorktreeProjection {
                        repo_key: tree.repo_key.clone(),
                        repo_name: tree.repo_name.clone(),
                        repo_root: tree.repo_root.clone(),
                        checkout_path: tree.checkout_path.clone(),
                        is_linked_worktree: tree.is_linked_worktree,
                    }
                }),
                label: workspace.label.clone(),
                active_tab_id: non_blank(Some(workspace.active_tab_id.as_str())),
                tokens: workspace.tokens.clone(),
            })
            .collect();
        SessionSnapshotPayload {
            tab_focus: self.tab_focus.clone(),
            tab_moves: self.tab_moves.clone(),
            focused_pane_id: self.focused_pane_id.clone(),
            focused_workspace_id: self.focused_workspace_id.clone(),
            tabs,
            layouts: self.layouts.clone(),
            agents,
            panes,
            workspaces,
        }
    }
}

/// An engine label or terminal title that is present but blank carries no more
/// information than an absent one, and a header ladder that treated the two
/// differently would show an empty title instead of falling through.
pub(crate) fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

impl ProjectionState {
    /// The remote session of the device `target_id` built from `payload`,
    /// which is [`Self::project`] with the target's labels laid on.
    pub(crate) fn project_remote(
        &self,
        target_id: &str,
        payload: SessionSnapshotPayload,
    ) -> Result<(RemoteSessionSnapshot, Vec<crate::sidebar::AgentExclusion>), SessionFetchError>
    {
        let state = self;
        let issue_tokens = crate::engine::tokens::issue_tokens(&payload);
        let agent_projection = crate::sidebar::project_agents(payload);
        let mut agents = agent_projection.agents;

        let mut pane_layouts = Vec::with_capacity(state.layouts.len());
        for layout in &state.layouts {
            let area_width = f64::from(layout.area.width);
            let area_height = f64::from(layout.area.height);
            if area_width <= 0.0 || area_height <= 0.0 {
                return Err(SessionFetchError::Malformed(format!(
                    "remote engine layout {} has an empty area",
                    layout.tab_id
                )));
            }
            let mut frames = Vec::with_capacity(layout.panes.len());
            for pane in &layout.panes {
                let frame = RemotePaneLayoutFrame {
                    pane_id: pane.pane_id.clone(),
                    x: (f64::from(pane.rect.x) - f64::from(layout.area.x)) / area_width,
                    y: (f64::from(pane.rect.y) - f64::from(layout.area.y)) / area_height,
                    width: f64::from(pane.rect.width) / area_width,
                    height: f64::from(pane.rect.height) / area_height,
                };
                if frame.x < 0.0
                    || frame.y < 0.0
                    || frame.width <= 0.0
                    || frame.height <= 0.0
                    || frame.x + frame.width > 1.0
                    || frame.y + frame.height > 1.0
                {
                    return Err(SessionFetchError::Malformed(format!(
                        "remote engine layout {} contains an out-of-bounds pane {}",
                        layout.tab_id, pane.pane_id
                    )));
                }
                frames.push(frame);
            }
            pane_layouts.push(RemotePaneLayoutSnapshot {
                workspace_id: crate::address::scoped_id(
                    target_id,
                    crate::address::Kind::Workspace,
                    &layout.workspace_id,
                ),
                tab_id: crate::address::scoped_tab_id(target_id, &layout.tab_id),
                focused_pane_id: crate::address::scoped_pane_id(target_id, &layout.focused_pane_id),
                zoomed: layout.zoomed,
                frames: frames
                    .into_iter()
                    .map(|mut frame| {
                        frame.pane_id = crate::address::scoped_pane_id(target_id, &frame.pane_id);
                        frame
                    })
                    .collect(),
            });
        }

        let mut workspaces = state
            .workspaces
            .iter()
            .map(|workspace| {
                let workspace_id = crate::address::scoped_id(
                    target_id,
                    crate::address::Kind::Workspace,
                    &workspace.workspace_id,
                );
                let checkout_id = crate::address::scoped_id(
                    target_id,
                    crate::address::Kind::Checkout,
                    &workspace.workspace_id,
                );
                let workspace_panes = state
                    .panes
                    .iter()
                    .filter(|pane| pane.workspace_id == workspace.workspace_id)
                    .collect::<Vec<_>>();
                let pane_path = workspace_panes
                    .iter()
                    .find_map(|pane| pane.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()))
                    .map(str::to_owned);
                let path = workspace
                    .worktree
                    .as_ref()
                    .map(|worktree| worktree.checkout_path.clone())
                    .or(pane_path)
                    .unwrap_or_default();
                let repo_name = workspace
                    .worktree
                    .as_ref()
                    .map(|worktree| worktree.repo_name.clone())
                    .unwrap_or_else(|| workspace.label.clone());
                let next_tab_label = crate::model::next_tab_label(
                    state
                        .tabs
                        .iter()
                        .filter(|tab| tab.workspace_id == workspace.workspace_id)
                        .map(|tab| tab.label.as_str()),
                );
                let tabs = state
                    .tabs
                    .iter()
                    .filter(|tab| tab.workspace_id == workspace.workspace_id)
                    .map(|tab| {
                        let panes = workspace_panes
                            .iter()
                            .filter(|pane| pane.tab_id == tab.tab_id)
                            .map(|pane| {
                                let agent =
                                    agents.iter().find(|agent| agent.pane_id == pane.pane_id);
                                PaneSnapshot {
                                    device_id: Default::default(),
                                    device_pane_id: Default::default(),
                                    start_claim: pane
                                        .label
                                        .as_deref()
                                        .filter(|label| crate::start_claim::is_reserved(label))
                                        .map(str::to_owned),
                                    terminal_id: Some(pane.terminal_id.clone()),
                                    failed_terminal_id: pane
                                        .restore_error
                                        .as_ref()
                                        .map(|_| pane.terminal_id.clone()),
                                    id: crate::address::scoped_pane_id(target_id, &pane.pane_id),
                                    engine_label: crate::start_claim::public_label(
                                        pane.label.as_deref(),
                                    ),
                                    terminal_title: crate::start_claim::public_label(
                                        pane.terminal_title.as_deref(),
                                    ),
                                    cwd: pane.cwd.clone().unwrap_or_else(|| path.clone()),
                                    status_code: agent
                                        .map(|agent| agent.status_code)
                                        .unwrap_or(crate::model::AgentStatusCode::Attached),
                                    requires_close_confirmation: agent
                                        .is_some_and(|agent| agent.requires_close_confirmation),
                                    requires_close_status_check: agent
                                        .is_some_and(|agent| agent.requires_close_status_check),
                                    identity_label: agent.map(|agent| agent.identity_label.clone()),
                                    activity_at_unix_ms: agent
                                        .and_then(|agent| agent.changed_at_unix_ms),
                                    // Fork agent runs on this machine's panes
                                    // only, so a device pane offers none.
                                    fork: Default::default(),
                                    // Ports describe this machine's listeners,
                                    // so a remote pane reports none rather than
                                    // claiming the local machine's.
                                    ports: Vec::new(),
                                    servers: Vec::new(),
                                    // Judged by the core from this pane's
                                    // hook tokens and the device's kit, the
                                    // same way a local pane is (PRD
                                    // device-parity D-21).
                                    children: None,
                                    lineage_path: Vec::new(),
                                    sleep: None,
                                    sleep_action: None,
                                }
                            })
                            .collect::<Vec<_>>();
                        let focused = state
                            .layouts
                            .iter()
                            .find(|layout| layout.tab_id == tab.tab_id)
                            .map(|layout| layout.focused_pane_id.as_str());
                        let representative = agents
                            .iter()
                            .find(|agent| Some(agent.pane_id.as_str()) == focused);
                        let process = state
                            .panes
                            .iter()
                            .find(|pane| Some(pane.pane_id.as_str()) == focused)
                            .and_then(|pane| pane.foreground_process.as_deref());
                        TabSnapshot {
                            agent: representative.map(Into::into),
                            naming: crate::model::TabNaming {
                                // Rename is recorded for this machine's tabs.
                                by_operator: false,
                                focused_pane_id: focused
                                    .map(|pane| crate::address::scoped_pane_id(target_id, pane))
                                    .unwrap_or_default(),
                                number: crate::model::tab_number(&tab.label, tab.number),
                                processes: state
                                    .panes
                                    .iter()
                                    .filter(|pane| pane.tab_id == tab.tab_id)
                                    .filter_map(|pane| {
                                        pane.foreground_process.clone().map(|name| {
                                            (
                                                crate::address::scoped_pane_id(
                                                    target_id,
                                                    &pane.pane_id,
                                                ),
                                                name,
                                            )
                                        })
                                    })
                                    .collect(),
                                raw: tab.label.clone(),
                                automatic: crate::model::display_tab_label(
                                    "",
                                    tab.number,
                                    representative.map(|agent| agent.identity_label.as_str()),
                                    process,
                                ),
                            },
                            id: Some(crate::address::scoped_tab_id(target_id, &tab.tab_id)),
                            workspace_id: Some(workspace_id.clone()),
                            checkout_id: Some(checkout_id.clone()),
                            label: Some(crate::model::display_tab_label(
                                &tab.label,
                                tab.number,
                                representative.map(|agent| agent.identity_label.as_str()),
                                process,
                            )),
                            empty: panes.is_empty(),
                            delegated: false,
                            panes,
                        }
                    })
                    .collect::<Vec<_>>();
                // The engine half of the strip in the engine's order; the runtime
                // places the device's file tabs among them (`place_device_strips`).
                let strip = StripTabSnapshot::from_engine_tabs(&tabs);
                let active_tab_id = Some(crate::address::scoped_tab_id(
                    target_id,
                    &workspace.active_tab_id,
                ))
                .filter(|active| tabs.iter().any(|tab| tab.id.as_ref() == Some(active)));
                WorkspaceSnapshot {
                    agent_presence: Default::default(),
                    agent_scope: Default::default(),
                    home_issues: Default::default(),
                    pull_requests: Vec::new(),
                    tasks: Default::default(),
                    id: workspace_id.clone(),
                    label: workspace.label.clone(),
                    path: path.clone(),
                    remote_target_id: Some(target_id.to_owned()),
                    expanded: true,
                    device_id: target_id.to_owned(),
                    repo_name,
                    is_git: workspace.worktree.is_some(),
                    default_branch: None,
                    branches: Vec::new(),
                    registered: true,
                    temporary: false,
                    session_workspace_ids: vec![workspace.workspace_id.clone()],
                    last_activity_unix_ms: None,
                    checkouts: vec![CheckoutSnapshot {
                        id: checkout_id,
                        workspace_id,
                        label: workspace.label.clone(),
                        path: path.clone(),
                        branch: None,
                        is_worktree: workspace
                            .worktree
                            .as_ref()
                            .is_some_and(|worktree| worktree.is_linked_worktree),
                        engine_repo_root: workspace
                            .worktree
                            .as_ref()
                            .map(|worktree| worktree.repo_root.clone()),
                        exists: !path.is_empty(),
                        temporary: false,
                        // A remote checkout carries no worktree, pull-request,
                        // or disk facts: those readers describe this machine.
                        has_panes: !tabs.is_empty(),
                        purpose: workspace
                            .tokens
                            .get("purpose")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|purpose| !purpose.is_empty())
                            .map(|purpose| crate::model::CheckoutPurposeSnapshot {
                                text: purpose.chars().take(80).collect(),
                                origin: crate::model::CheckoutPurposeOrigin::Token,
                            }),
                        tabs,
                        active_tab_id,
                        strip,
                        next_tab_label,
                        // A worktree workspace owns the checkout the engine binds
                        // it to, which is this row's path; a marked one is
                        // checked against its folder once grouped.
                        owner_workspace_id: workspace
                            .worktree
                            .is_some()
                            .then(|| workspace.workspace_id.clone()),
                        owner_mark: workspace
                            .tokens
                            .get(crate::checkout_owner::OWNER_TOKEN)
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        ..CheckoutSnapshot::default()
                    }],
                    pinned: false,
                    is_home: false,
                    inactive_checkouts: Default::default(),
                    session_folds: Default::default(),
                    removal: Default::default(),
                    disk: Default::default(),
                    cleanup: None,
                }
            })
            .collect::<Vec<_>>();
        for agent in &mut agents {
            let source_pane_id = agent.pane_id.clone();
            agent.id =
                crate::address::scoped_id(target_id, crate::address::Kind::Agent, &source_pane_id);
            agent.pane_id = crate::address::scoped_pane_id(target_id, &source_pane_id);
            // A declaration without a machine stays within this device. A
            // machine-qualified declaration is resolved after every device's
            // stable identity is known, so keep its raw pane token here.
            if agent.spawned_from_machine_id.is_none() {
                agent.spawned_from_pane_id = agent
                    .spawned_from_pane_id
                    .as_deref()
                    .map(|parent| crate::address::scoped_pane_id(target_id, parent));
            }
        }

        // Remote rows use the same representative-agent and purpose fallback
        // ladder as local rows. Run this after target-scoping pane ids so the
        // summary can join each agent to its projected checkout.
        crate::agent_state::sync_checkout_agent_summaries(&mut workspaces, &agents);

        // A remote project list follows the same activity order as a local
        // one, so a user reading two devices reads one rule. The agent pane
        // ids are rewritten first because the ordering matches agents to the
        // projected panes by id, and the projection carries the remote form.
        crate::project_context::sort_projects(&mut workspaces, &agents);

        // Keyed by checkout: the runtime groups a device's engine workspaces
        // into projects (`device_catalog`), and a checkout is the one id that
        // still names a single engine workspace after that.
        let active_tab_ids = state
            .workspaces
            .iter()
            .map(|workspace| {
                (
                    crate::address::scoped_id(
                        target_id,
                        crate::address::Kind::Checkout,
                        &workspace.workspace_id,
                    ),
                    crate::address::scoped_tab_id(target_id, &workspace.active_tab_id),
                )
            })
            .collect();

        let focused = state
            .focused_pane_id
            .as_deref()
            .and_then(|pane_id| state.panes.iter().find(|pane| pane.pane_id == pane_id));
        let focused_workspace_id = focused.map(|pane| {
            crate::address::scoped_id(
                target_id,
                crate::address::Kind::Workspace,
                &pane.workspace_id,
            )
        });
        let focused_checkout_id = focused.map(|pane| {
            crate::address::scoped_id(
                target_id,
                crate::address::Kind::Checkout,
                &pane.workspace_id,
            )
        });
        let focused_tab_id =
            focused.map(|pane| crate::address::scoped_tab_id(target_id, &pane.tab_id));

        Ok((
            RemoteSessionSnapshot {
                workspaces,
                agents,
                active_tab_ids,
                focused_workspace_id,
                focused_checkout_id,
                focused_tab_id,
                focused_pane_id: state
                    .focused_pane_id
                    .as_deref()
                    .map(|pane_id| crate::address::scoped_pane_id(target_id, pane_id)),
                pane_layouts,
                // The core's hook facts, laid by the device's coordinator;
                // the engine's pane tokens carry none of them.
                pane_hook_tokens: BTreeMap::new(),
                issue_tokens: crate::engine::tokens::IssueTokens {
                    panes: issue_tokens
                        .panes
                        .into_iter()
                        .map(|(pane, token)| {
                            (crate::address::scoped_pane_id(target_id, &pane), token)
                        })
                        .collect(),
                    workspaces: issue_tokens.workspaces,
                },
                issue_links: BTreeMap::new(),
            },
            agent_projection.excluded,
        ))
    }
}
