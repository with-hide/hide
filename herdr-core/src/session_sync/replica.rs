//! Applies ordered Herdr topology events to the authoritative replica.

use std::collections::VecDeque;

use super::*;

/// How many of the latest ids removed by events a replica remembers
/// (`retired_ids`); a workspace close retires its tabs and panes with it.
const RETIRED_ID_WINDOW: usize = 64;

/// How many focus events a replica holds for tabs and panes not announced
/// yet (`early_focuses`); one more is a divergence that rebuilds the replica.
const EARLY_FOCUS_LIMIT: usize = 16;

/// How many creation events a replica holds for a parent not announced yet
/// (`early_children`): a tab for its workspace, a pane and a layout for their
/// tab; a tab, a pane and a layout for each tab of a burst of 32. One more is
/// a divergence that rebuilds the replica.
const EARLY_CHILD_LIMIT: usize = 96;

/// How many panes a replica holds waiting for their cwd to be confirmed
/// (`unconfirmed_cwds`). A pane waits at most `CWD_READ_LIMIT` coordinator
/// ticks, so the map holds the panes created in the last two seconds; a pane
/// past the cap keeps the cwd it was announced with and is logged.
const UNCONFIRMED_CWD_LIMIT: usize = 64;

/// How many times the coordinator reads one pane's cwd before it takes the
/// last answer without a second one agreeing (`pane_cwd.unsettled`).
const CWD_READ_LIMIT: u32 = 8;

static NEXT_REPLICA_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone)]
pub(crate) struct SessionReplica {
    generation: u64,
    /// Query freshness is independent of the topology stream's lifetime.
    /// A successful snapshot or agent list confirms telemetry again.
    pub(crate) agents_confirmed: bool,
    pub(crate) state: ProjectionState,
    /// The last projection whose dependency ranges were confirmed. Pending
    /// topology is kept in `state`, while this copy is what the runtime is
    /// allowed to see. That lets an independent workspace move forward while
    /// a different workspace still waits for its layout or replacement focus.
    pub(crate) published_state: ProjectionState,
    pub(crate) pending_layouts: BTreeSet<String>,
    /// At most one pairing per live tab; consumed when its creation focus event
    /// is applied, even when that workspace cannot publish its layout yet.
    pending_creation_focuses: BTreeSet<String>,
    pub(crate) pending_workspace_closures: BTreeSet<String>,
    pub(crate) pending_active_tab_focuses: BTreeSet<String>,
    /// The workspace that held Herdr's focus when it closed, while other
    /// workspaces remain. Herdr 0.9.5 moves its focus to one of them without
    /// an event (`tab.close` of the focused workspace's only tab emits
    /// `workspace_closed` and `tab_closed`, nothing else), so the replica
    /// holds that close until a focus event or a `workspace.list` read names
    /// the workspace in focus (`settle_focus`). Published without it, the
    /// device has no focused checkout and its screen loses the Workspace.
    pending_focus_after_close: Option<String>,
    /// Workspace, tab and pane ids this replica removed on an event, newest
    /// last. Herdr 0.9.1 can name an id after the event that removed it:
    /// `tab.close` of a workspace's last tab emits `workspace_closed` then
    /// `tab_closed` (and `pane.close` then `pane_closed`), and a workspace's
    /// creation events arrive after the call that created it returns, so a
    /// seed tab that `layout.apply` replaced is announced closed and then
    /// focused, given a pane and laid out. An event about an id retired here
    /// and absent from the replica is already applied, not a divergence.
    /// Herdr never reuses an id, and such events follow within one burst,
    /// so a short window is enough.
    retired_ids: VecDeque<String>,
    /// Focus events that name a tab or pane Herdr has not announced yet.
    /// Herdr 0.9.1 emits a workspace's creation events after the call that
    /// created it returns, carrying the focus of that moment: a tab created
    /// with focus in a workspace just created emits `tab_focused` for it
    /// before its `tab_created`. Each is applied once its target arrives;
    /// only the latest per workspace and kind is kept.
    early_focuses: Vec<ReplicaEvent>,
    /// `pane_created` and `layout_updated` events that name a tab Herdr has
    /// not announced yet. A tab's pane and layout can reach the stream before
    /// its `tab_created` (CI, issues 754 and 756); applied when the tab
    /// arrives, in the order they came. Dropping them instead lost the tab's
    /// pane, and its layout, which its workspace waits for before it publishes
    /// again, so the whole workspace stopped changing on screen.
    early_children: Vec<ReplicaEvent>,
    /// Panes announced by `pane_created` whose cwd Herdr has not confirmed.
    /// Herdr 0.9.1 can announce a new pane with a cwd that is not the one it
    /// was created in (`/tmp` for a tab created in the project folder, seen
    /// on CI), and on a loaded machine a read right afterwards can still
    /// answer a folder above it. Herdr sends no event when the cwd settles,
    /// and `agent.list` lists agent terminals only, so `pane.get` reads are
    /// what confirm it: once right after the event, then once per
    /// coordinator tick until an answer repeats the one before it
    /// (`confirm_pane_cwd`).
    unconfirmed_cwds: BTreeMap<String, UnconfirmedCwd>,
    /// How many events this replica has applied since its snapshot. Herdr's
    /// stream carries no sequence, so this is the only position a diagnostic
    /// can name.
    pub(crate) applied_events: u64,
    /// Every `tab_focused` this replica applied, as moves. `state` is
    /// Herdr's topology and carries none; each publish lays them on
    /// `published_state`.
    tab_moves: crate::sidebar::SessionTabMoves,
}

/// A pane's cwd while Herdr's answers for it may still move.
#[derive(Clone)]
struct UnconfirmedCwd {
    /// The cwd the pane was announced with, then the one Herdr last answered;
    /// an answer equal to it confirms the pane.
    last: Option<String>,
    reads: u32,
    /// Whether a coordinator tick has passed since the pane was announced.
    /// The first read follows the event at once, so the next one waits a
    /// whole tick rather than landing in the same instant.
    ticked: bool,
}

/// How an event that disagrees with the replica is treated.
///
/// A subscription is opened before the snapshot it starts from, so the
/// stream cannot miss an event; the price is that an event emitted before
/// the snapshot was taken arrives as well, and it describes a change the
/// snapshot already holds. For a short window after the snapshot the
/// snapshot therefore wins: an event it contradicts (a pane created that it
/// already lists, a pane closed that it no longer lists) is dropped with a
/// diagnostic rather than declared malformed. After the window an event
/// that contradicts the replica is a real divergence, and the replica is
/// rebuilt from a fresh snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplyMode {
    Strict,
    Reconcile,
}

#[derive(Debug)]
pub(crate) struct ApplyOutcome {
    pub(crate) publish: bool,
    pub(crate) refresh_agents: bool,
    pub(crate) refresh_worktrees: bool,
}

/// Every top-level `session.snapshot` field the replica reads. The contract
/// test asserts the pinned Herdr declares each one required, so a pin whose
/// snapshot cannot feed the replica fails in CI rather than at the user's
/// first launch with "snapshot is missing agents".
#[cfg(test)]
pub(crate) const SNAPSHOT_FIELDS_THE_REPLICA_READS: [&str; 7] = [
    "version",
    "protocol",
    "workspaces",
    "tabs",
    "panes",
    "layouts",
    "agents",
];

impl SessionReplica {
    #[cfg(test)]
    pub(crate) fn from_snapshot(snapshot: &Value) -> Result<Self, SessionFetchError> {
        Self::from_decoded(wire::snapshot(snapshot.clone())?)
    }

    pub(crate) fn from_decoded(state: ProjectionState) -> Result<Self, SessionFetchError> {
        for pane in &state.panes {
            report_restore_failure(pane);
        }
        let generation = NEXT_REPLICA_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tab_moves = crate::sidebar::SessionTabMoves::new(generation);
        let replica = Self {
            generation,
            agents_confirmed: true,
            published_state: ProjectionState {
                tab_moves: Some(tab_moves.clone()),
                ..state.clone()
            },
            state,
            pending_layouts: BTreeSet::new(),
            pending_creation_focuses: BTreeSet::new(),
            pending_workspace_closures: BTreeSet::new(),
            pending_active_tab_focuses: BTreeSet::new(),
            pending_focus_after_close: None,
            retired_ids: VecDeque::new(),
            early_focuses: Vec::new(),
            early_children: Vec::new(),
            unconfirmed_cwds: BTreeMap::new(),
            applied_events: 0,
            tab_moves,
        };
        replica.validate()?;
        replica.validate_active_tabs()?;
        Ok(replica)
    }

    pub(crate) fn project(&self) -> SessionSnapshotPayload {
        let mut payload = self.published_state.project();
        for agent in &mut payload.agents {
            agent.telemetry_stale = !self.agents_confirmed;
        }
        payload
    }

    /// Returns true only on the transition, so repeated failed reads do not
    /// rebuild a catalog or publish the same Unknown rows again.
    pub(crate) fn invalidate_agents(&mut self) -> bool {
        std::mem::replace(&mut self.agents_confirmed, false)
    }

    fn pending_workspace_ids(&self) -> HashSet<String> {
        let mut blocked = HashSet::new();
        for workspace_id in self
            .pending_workspace_closures
            .iter()
            .chain(self.pending_active_tab_focuses.iter())
            .chain(self.pending_focus_after_close.iter())
        {
            blocked.insert(workspace_id.clone());
        }
        for tab_id in &self.pending_layouts {
            self.state
                .tabs
                .iter()
                .find(|tab| &tab.tab_id == tab_id)
                .or_else(|| {
                    self.published_state
                        .tabs
                        .iter()
                        .find(|tab| &tab.tab_id == tab_id)
                })
                .map(|tab| tab.workspace_id.as_str())
                .or_else(|| {
                    // workspace_created names its first active tab before
                    // tab_created supplies membership. Its workspace is
                    // already known and must wait for that tab's layout too.
                    self.state
                        .workspaces
                        .iter()
                        .chain(&self.published_state.workspaces)
                        .find(|workspace| &workspace.active_tab_id == tab_id)
                        .map(|workspace| workspace.workspace_id.as_str())
                })
                .map(|workspace_id| blocked.insert(workspace_id.to_owned()));
        }
        blocked
    }

    fn merge_by_workspace<T: Clone>(
        current: &[T],
        published: &[T],
        blocked: &HashSet<String>,
        workspace_id: impl Fn(&T) -> &str,
    ) -> Vec<T> {
        let mut merged = Vec::with_capacity(current.len() + published.len());
        for item in current {
            if blocked.contains(workspace_id(item)) {
                continue;
            } else {
                merged.push(item.clone());
            }
        }
        for item in published {
            if blocked.contains(workspace_id(item)) {
                merged.push(item.clone());
            }
        }
        merged
    }

    fn workspace_for_pane<'a>(state: &'a ProjectionState, pane_id: &str) -> Option<&'a str> {
        state
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .map(|pane| pane.workspace_id.as_str())
    }

    fn focused_pane_for_partial_publish(&self, blocked: &HashSet<String>) -> Option<String> {
        match self.state.focused_pane_id.as_deref() {
            Some(pane_id) => match Self::workspace_for_pane(&self.state, pane_id) {
                Some(workspace_id) if !blocked.contains(workspace_id) => Some(pane_id.to_owned()),
                Some(_) => self.published_state.focused_pane_id.clone(),
                None => None,
            },
            None => self
                .published_state
                .focused_pane_id
                .as_deref()
                .filter(|pane_id| {
                    Self::workspace_for_pane(&self.published_state, pane_id)
                        .is_some_and(|workspace_id| blocked.contains(workspace_id))
                })
                .map(str::to_owned),
        }
    }

    fn focused_workspace_for_partial_publish(&self, blocked: &HashSet<String>) -> Option<String> {
        match self.state.focused_workspace_id.as_deref() {
            Some(workspace_id) if !blocked.contains(workspace_id) => Some(workspace_id.to_owned()),
            Some(_) => self.published_state.focused_workspace_id.clone(),
            None => self
                .published_state
                .focused_workspace_id
                .as_deref()
                .filter(|workspace_id| blocked.contains(*workspace_id))
                .map(str::to_owned),
        }
    }

    fn partial_published_state(&self) -> ProjectionState {
        let blocked = self.pending_workspace_ids();
        let focused_workspace_id = self.focused_workspace_for_partial_publish(&blocked);
        let tab_focus = match self.state.focused_workspace_id.as_ref() {
            Some(workspace) if !blocked.contains(workspace) => self.state.tab_focus.clone(),
            _ => self.published_state.tab_focus.clone(),
        }
        .filter(|focus| focused_workspace_id.as_ref() == Some(&focus.workspace_id));
        ProjectionState {
            tab_focus,
            // Laid on by `refresh_published_state`.
            tab_moves: None,
            focused_pane_id: self.focused_pane_for_partial_publish(&blocked),
            focused_workspace_id,
            workspaces: Self::merge_by_workspace(
                &self.state.workspaces,
                &self.published_state.workspaces,
                &blocked,
                |workspace| workspace.workspace_id.as_str(),
            ),
            tabs: Self::merge_by_workspace(
                &self.state.tabs,
                &self.published_state.tabs,
                &blocked,
                |tab| tab.workspace_id.as_str(),
            ),
            panes: Self::merge_by_workspace(
                &self.state.panes,
                &self.published_state.panes,
                &blocked,
                |pane| pane.workspace_id.as_str(),
            ),
            layouts: Self::merge_by_workspace(
                &self.state.layouts,
                &self.published_state.layouts,
                &blocked,
                |layout| layout.workspace_id.as_str(),
            ),
            agents: Self::merge_by_workspace(
                &self.state.agents,
                &self.published_state.agents,
                &blocked,
                |agent| agent.workspace_id.as_str(),
            ),
            herdr_version: self.state.herdr_version.clone(),
        }
    }

    pub(crate) fn refresh_published_state(&mut self) -> Result<bool, SessionFetchError> {
        self.restore_missing_pane_focus();
        let ready = self.ready_to_publish();
        let mut next = if ready {
            self.validate()?;
            self.state.clone()
        } else {
            self.partial_published_state()
        };
        // Every move applied so far is published, held workspace or not: a
        // move answers a request Hide sent and says nothing about a topology
        // still settling. But a move alone never publishes a held state: it
        // would publish the topology from before a workspace the creation
        // worker has already projected, and retire that workspace's pane.
        // The move rides on the next publish.
        next.tab_moves = self.published_state.tab_moves.clone();
        if !ready && next == self.published_state {
            return Ok(false);
        }
        next.tab_moves = Some(self.tab_moves.clone());
        let changed = next != self.published_state;
        self.published_state = next;
        Ok(changed)
    }

    fn restore_missing_pane_focus(&mut self) {
        if self.state.focused_pane_id.is_some() {
            return;
        }
        let Some(workspace) = self.state.workspaces.iter().find(|workspace| {
            Some(&workspace.workspace_id) == self.state.focused_workspace_id.as_ref()
        }) else {
            return;
        };
        if self
            .pending_active_tab_focuses
            .contains(&workspace.workspace_id)
            || self
                .pending_workspace_closures
                .contains(&workspace.workspace_id)
            || self.pending_layouts.contains(&workspace.active_tab_id)
        {
            return;
        }
        // A close can select another tab without a pane focus or layout event.
        // Its confirmed layout already carries Herdr's selected pane. Only the
        // focused workspace may supply it; a background layout is no focus move.
        if let Some(layout) = self.state.layouts.iter().find(|layout| {
            layout.workspace_id == workspace.workspace_id
                && layout.tab_id == workspace.active_tab_id
        }) {
            self.state.focused_pane_id = Some(layout.focused_pane_id.clone());
        }
    }

    /// The remote session built from `payload`, which is [`Self::project`]
    /// with the target's labels laid on.
    pub(crate) fn project_remote(
        &self,
        target_id: &str,
        payload: SessionSnapshotPayload,
    ) -> Result<(RemoteSessionSnapshot, Vec<crate::sidebar::AgentExclusion>), SessionFetchError>
    {
        Self::project_remote_state(&self.published_state, target_id, payload)
    }

    pub(crate) fn project_remote_state(
        state: &ProjectionState,
        target_id: &str,
        payload: SessionSnapshotPayload,
    ) -> Result<(RemoteSessionSnapshot, Vec<crate::sidebar::AgentExclusion>), SessionFetchError>
    {
        let issue_tokens = crate::wire::issue_tokens(&payload);
        let agent_projection = crate::sidebar::project_agents(payload);
        let mut agents = agent_projection.agents;

        let mut pane_layouts = Vec::with_capacity(state.layouts.len());
        for layout in &state.layouts {
            let area_width = f64::from(layout.area.width);
            let area_height = f64::from(layout.area.height);
            if area_width <= 0.0 || area_height <= 0.0 {
                return Err(SessionFetchError::Malformed(format!(
                    "remote Herdr layout {} has an empty area",
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
                        "remote Herdr layout {} contains an out-of-bounds pane {}",
                        layout.tab_id, pane.pane_id
                    )));
                }
                frames.push(frame);
            }
            pane_layouts.push(RemotePaneLayoutSnapshot {
                workspace_id: remote_workspace_id(target_id, &layout.workspace_id),
                tab_id: remote_tab_id(target_id, &layout.tab_id),
                focused_pane_id: remote_pane_id(target_id, &layout.focused_pane_id),
                zoomed: layout.zoomed,
                frames: frames
                    .into_iter()
                    .map(|mut frame| {
                        frame.pane_id = remote_pane_id(target_id, &frame.pane_id);
                        frame
                    })
                    .collect(),
            });
        }

        let mut workspaces = state
            .workspaces
            .iter()
            .map(|workspace| {
                let workspace_id = remote_workspace_id(target_id, &workspace.workspace_id);
                let checkout_id = remote_checkout_id(target_id, &workspace.workspace_id);
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
                                    id: remote_pane_id(target_id, &pane.pane_id),
                                    herdr_label: crate::start_claim::public_label(
                                        pane.label.as_deref(),
                                    ),
                                    terminal_title: crate::start_claim::public_label(
                                        pane.terminal_title_stripped
                                            .as_deref()
                                            .or(pane.terminal_title.as_deref()),
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
                                    .map(|pane| remote_pane_id(target_id, pane))
                                    .unwrap_or_default(),
                                number: crate::model::tab_number(&tab.label, tab.number),
                                processes: state
                                    .panes
                                    .iter()
                                    .filter(|pane| pane.tab_id == tab.tab_id)
                                    .filter_map(|pane| {
                                        pane.foreground_process.clone().map(|name| {
                                            (remote_pane_id(target_id, &pane.pane_id), name)
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
                            id: Some(remote_tab_id(target_id, &tab.tab_id)),
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
                // The Herdr half of the strip in Herdr's order; the runtime
                // places the device's file tabs among them (`place_device_strips`).
                let strip = StripTabSnapshot::from_herdr_tabs(&tabs);
                let active_tab_id = Some(remote_tab_id(target_id, &workspace.active_tab_id))
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
                        herdr_repo_root: workspace
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
                        // A worktree workspace owns the checkout Herdr binds
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
            agent.id = format!("remote:{target_id}:agent:{source_pane_id}");
            agent.pane_id = remote_pane_id(target_id, &source_pane_id);
            // A declaration without a machine stays within this device. A
            // machine-qualified declaration is resolved after every device's
            // stable identity is known, so keep its raw pane token here.
            if agent.spawned_from_machine_id.is_none() {
                agent.spawned_from_pane_id = agent
                    .spawned_from_pane_id
                    .as_deref()
                    .map(|parent| remote_pane_id(target_id, parent));
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

        // Keyed by checkout: the runtime groups a device's Herdr workspaces
        // into projects (`device_catalog`), and a checkout is the one id that
        // still names a single Herdr workspace after that.
        let active_tab_ids = state
            .workspaces
            .iter()
            .map(|workspace| {
                (
                    remote_checkout_id(target_id, &workspace.workspace_id),
                    remote_tab_id(target_id, &workspace.active_tab_id),
                )
            })
            .collect();

        let focused = state
            .focused_pane_id
            .as_deref()
            .and_then(|pane_id| state.panes.iter().find(|pane| pane.pane_id == pane_id));
        let focused_workspace_id =
            focused.map(|pane| remote_workspace_id(target_id, &pane.workspace_id));
        let focused_checkout_id =
            focused.map(|pane| remote_checkout_id(target_id, &pane.workspace_id));
        let focused_tab_id = focused.map(|pane| remote_tab_id(target_id, &pane.tab_id));

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
                    .map(|pane_id| remote_pane_id(target_id, pane_id)),
                pane_layouts,
                pane_hook_tokens: state
                    .panes
                    .iter()
                    .map(|pane| {
                        (
                            remote_pane_id(target_id, &pane.pane_id),
                            crate::agent_hooks::PaneHookTokens::read(&pane.tokens),
                        )
                    })
                    .filter(|(_, tokens)| !tokens.is_empty())
                    .collect(),
                issue_tokens: crate::wire::IssueTokens {
                    panes: issue_tokens
                        .panes
                        .into_iter()
                        .map(|(pane, token)| (remote_pane_id(target_id, &pane), token))
                        .collect(),
                    workspaces: issue_tokens.workspaces,
                },
                issue_links: BTreeMap::new(),
                herdr_version: state.herdr_version.clone(),
            },
            agent_projection.excluded,
        ))
    }

    /// Workspaces whose active tab a close removed and whose replacement Herdr
    /// has not named. Herdr emits `tab_focused` for the replacement only when
    /// that workspace holds its keyboard focus; in every other workspace the
    /// new active tab is that workspace's memory and reaches Hide only through
    /// a `workspace.get` read, which the coordinator issues for each of these.
    pub(crate) fn workspaces_awaiting_active_tab(&self) -> Vec<String> {
        self.pending_active_tab_focuses.iter().cloned().collect()
    }

    /// Applies a `workspace.get` answer to a workspace waiting for its
    /// replacement active tab, and reports whether that wait ended. A
    /// workspace a focus event already settled changes nothing. A tab the
    /// event stream has not delivered yet means the read ran ahead of the
    /// cursor, so the wait stays and the coordinator reads again on its
    /// bounded tick once the stream catches up.
    pub(crate) fn settle_active_tab(&mut self, workspace_id: &str, active_tab_id: &str) -> bool {
        if !self.pending_active_tab_focuses.contains(workspace_id) {
            return false;
        }
        let tab_known = self
            .state
            .tabs
            .iter()
            .any(|tab| tab.tab_id == active_tab_id && tab.workspace_id == workspace_id);
        if !tab_known {
            return false;
        }
        let Some(workspace) = self
            .state
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.workspace_id == workspace_id)
        else {
            return false;
        };
        workspace.active_tab_id = active_tab_id.to_owned();
        self.pending_active_tab_focuses.remove(workspace_id);
        true
    }

    /// The closed workspace whose successor in Herdr's focus the replica
    /// waits to read (`pending_focus_after_close`).
    pub(crate) fn awaiting_focus(&self) -> Option<&str> {
        self.pending_focus_after_close.as_deref()
    }

    /// Applies a `workspace.list` answer, the workspace Herdr has in focus or
    /// none, to a replica waiting for it, and reports whether that wait
    /// ended. A wait a focus event already ended changes nothing. A workspace
    /// the event stream has not delivered means the read ran ahead of the
    /// cursor, so the wait stays for the next read. The focused pane follows
    /// from that workspace's active tab layout (`restore_missing_pane_focus`).
    pub(crate) fn settle_focus(&mut self, focused_workspace_id: Option<&str>) -> bool {
        if self.pending_focus_after_close.is_none() {
            return false;
        }
        match focused_workspace_id {
            Some(workspace_id)
                if !self
                    .state
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == workspace_id) =>
            {
                false
            }
            Some(workspace_id) => {
                self.focus_workspace(workspace_id.to_owned());
                true
            }
            None => {
                self.pending_focus_after_close = None;
                true
            }
        }
    }

    /// Herdr's focus is in `workspace_id`: an event or a read named it, which
    /// ends a wait for the successor of a closed focused workspace.
    fn focus_workspace(&mut self, workspace_id: String) {
        self.state.focused_workspace_id = Some(workspace_id);
        self.pending_focus_after_close = None;
    }

    /// Panes whose cwd came from their creation event and has not been
    /// read back from Herdr yet.
    pub(crate) fn panes_awaiting_cwd(&self) -> Vec<String> {
        self.unconfirmed_cwds
            .iter()
            .filter(|(_, wait)| wait.reads == 0)
            .map(|(pane_id, _)| pane_id.clone())
            .collect()
    }

    /// Panes a coordinator tick reads again: those whose last answer did not
    /// repeat the one before it, once a tick has passed since their event.
    pub(crate) fn take_cwd_rereads(&mut self) -> Vec<String> {
        let mut due = Vec::new();
        for (pane_id, wait) in &mut self.unconfirmed_cwds {
            if wait.ticked {
                due.push(pane_id.clone());
            }
            wait.ticked = true;
        }
        due
    }

    /// Holds a pane's cwd as unconfirmed until Herdr answers it twice alike.
    fn await_cwd(&mut self, pane_id: String) {
        if self.unconfirmed_cwds.len() < UNCONFIRMED_CWD_LIMIT {
            let last = self
                .state
                .panes
                .iter()
                .find(|pane| pane.pane_id == pane_id)
                .and_then(|pane| pane.cwd.clone());
            self.unconfirmed_cwds.insert(
                pane_id,
                UnconfirmedCwd {
                    last,
                    reads: 0,
                    ticked: false,
                },
            );
        } else {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "pane_cwd.unconfirmed_over_cap",
                "pane_id": pane_id,
                "limit": UNCONFIRMED_CWD_LIMIT,
                "applied_events": self.applied_events,
            }));
        }
    }

    /// Applies a `pane.get` answer to a pane waiting for its cwd, and reports
    /// whether the cwd changed. An answer equal to the one before it (the
    /// event's cwd for the first read) confirms the pane; a different one is
    /// published and the pane keeps waiting, until `CWD_READ_LIMIT` reads
    /// take the last answer. An answer with no cwd (the pane closed before
    /// the read, or Herdr knows none) ends the wait and keeps the cwd held.
    pub(crate) fn confirm_pane_cwd(&mut self, pane_id: &str, cwd: Option<String>) -> bool {
        let Some(wait) = self.unconfirmed_cwds.get_mut(pane_id) else {
            return false;
        };
        let Some(cwd) = cwd else {
            self.unconfirmed_cwds.remove(pane_id);
            return false;
        };
        wait.reads += 1;
        if wait.last.as_deref() == Some(cwd.as_str()) {
            self.unconfirmed_cwds.remove(pane_id);
        } else if wait.reads >= CWD_READ_LIMIT {
            self.unconfirmed_cwds.remove(pane_id);
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "pane_cwd.unsettled",
                "pane_id": pane_id,
                "reads": CWD_READ_LIMIT,
                "applied_events": self.applied_events,
            }));
        } else {
            wait.last = Some(cwd.clone());
        }
        let Some(pane) = self
            .state
            .panes
            .iter_mut()
            .find(|pane| pane.pane_id == pane_id)
        else {
            return false;
        };
        if pane.cwd.as_deref() == Some(cwd.as_str()) {
            return false;
        }
        pane.cwd = Some(cwd);
        true
    }

    /// The cwd the replica holds for a pane: the one Herdr announced or
    /// confirmed, never a stand-in.
    pub(crate) fn pane_cwd(&self, pane_id: &str) -> Option<&str> {
        self.state
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .and_then(|pane| pane.cwd.as_deref())
    }

    pub(crate) fn ready_to_publish(&self) -> bool {
        self.pending_layouts.is_empty()
            && self.pending_workspace_closures.is_empty()
            && self.pending_active_tab_focuses.is_empty()
            && self.pending_focus_after_close.is_none()
    }

    /// Replaces the agent list and reports where agents stopped working.
    ///
    /// An agent leaving `working` is the moment its checkout's pull request is
    /// most likely to have just changed - it is what a run that ends in a push
    /// looks like from here - so it is the one event that re-reads `gh`
    /// without waiting out the five-minute window. Detecting it here rather
    /// than in the runtime keeps the comparison on the thread that already
    /// holds both the old and the new list.
    ///
    /// Returns the working directory of each agent that stopped, taken from
    /// the list it was working in: a pane that has since closed is gone from
    /// the new list and its old record is the one that still says where.
    pub(crate) fn replace_agents(&mut self, agents: Vec<ProjectedAgent>) -> Vec<String> {
        self.agents_confirmed = true;
        let stopped_in: Vec<String> = self
            .state
            .agents
            .iter()
            .filter(|agent| agent.agent_status.as_deref() == Some("working"))
            .filter(|agent| {
                !agents.iter().any(|next| {
                    next.pane_id == agent.pane_id && next.agent_status.as_deref() == Some("working")
                })
            })
            .filter_map(|agent| agent.cwd.clone())
            .collect();
        // agent.list reads an agent terminal's live cwd. Refresh that fact on
        // the same pane, without allowing the list to create or move
        // topology; a plain terminal is not listed, and its creation cwd is
        // confirmed by `confirm_pane_cwd` instead.
        let by_pane: HashMap<_, _> = agents
            .iter()
            .map(|agent| (agent.pane_id.as_str(), agent))
            .collect();
        for pane in &mut self.state.panes {
            if let Some(agent) = by_pane.get(pane.pane_id.as_str())
                && agent.workspace_id == pane.workspace_id
                && agent.tab_id == pane.tab_id
                && let Some(cwd) = agent.cwd.as_ref()
            {
                pane.cwd = Some(cwd.clone());
            }
        }
        self.state.agents = agents;
        stopped_in
    }

    pub(crate) fn apply(
        &mut self,
        event: ReplicaEvent,
        mode: ApplyMode,
    ) -> Result<ApplyOutcome, SessionFetchError> {
        let mut candidate = self.clone();
        let created_pane = match &event {
            ReplicaEvent::PaneCreated { pane } => Some(pane.pane_id.clone()),
            _ => None,
        };
        let refresh_worktrees = matches!(
            event,
            ReplicaEvent::WorktreeCreated { .. }
                | ReplicaEvent::WorktreeOpened { .. }
                | ReplicaEvent::WorktreeRemoved { .. }
        );
        let applied = candidate
            .apply_new_event(event, mode)
            .and_then(|refresh_agents| {
                candidate.apply_early_children(mode)?;
                candidate.apply_early_focuses(mode)?;
                Ok(refresh_agents)
            });
        let refresh_agents = match (applied, mode) {
            (Ok(refresh_agents), _) => refresh_agents,
            (Err(SessionFetchError::Malformed(detail)), ApplyMode::Reconcile) => {
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": "event.reconciled",
                    "applied_events": self.applied_events,
                    "message": detail,
                }));
                // A creation the snapshot already holds means the snapshot
                // was read while Herdr created the pane, so it may carry the
                // same early cwd the event would have.
                if let Some(pane_id) = created_pane
                    .filter(|pane_id| self.state.panes.iter().any(|pane| &pane.pane_id == pane_id))
                {
                    self.await_cwd(pane_id);
                }
                return Ok(ApplyOutcome {
                    publish: false,
                    refresh_agents: false,
                    refresh_worktrees: false,
                });
            }
            (Err(error), _) => return Err(error),
        };
        // A different authoritative focus owner retires its predecessor's
        // cause, so returning to that tab cannot resurrect a creation marker.
        if candidate.state.tab_focus.as_ref().is_some_and(|focus| {
            candidate.state.focused_workspace_id.as_ref() != Some(&focus.workspace_id)
                || !candidate.state.workspaces.iter().any(|workspace| {
                    workspace.workspace_id == focus.workspace_id
                        && workspace.active_tab_id == focus.tab_id
                })
        }) {
            candidate.state.tab_focus = None;
        }
        candidate.applied_events = candidate.applied_events.saturating_add(1);
        let publish = candidate.refresh_published_state()?;
        *self = candidate;
        Ok(ApplyOutcome {
            publish,
            refresh_agents,
            refresh_worktrees,
        })
    }

    fn apply_new_event(
        &mut self,
        data: ReplicaEvent,
        mode: ApplyMode,
    ) -> Result<bool, SessionFetchError> {
        if let Some((event, id)) = self.names_retired(&data) {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "event.already_applied",
                "event": event,
                "retired_id": id,
                "applied_events": self.applied_events,
            }));
            // What a late event creates inside a retired scope is retired
            // with it, so the events that follow about it are no-ops too.
            match &data {
                ReplicaEvent::TabCreated { tab, .. } => self.retire(&tab.tab_id),
                ReplicaEvent::PaneCreated { pane } => self.retire(&pane.pane_id),
                _ => {}
            }
            return Ok(false);
        }
        match data {
            ReplicaEvent::WorkspaceCreated {
                workspace: input_workspace,
            } => {
                let event = "workspace_created";
                validate_workspace_wire(event, &input_workspace)?;
                if self
                    .state
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == input_workspace.workspace_id)
                {
                    return Err(malformed_event(event, "created workspace already exists"));
                }
                self.pending_layouts
                    .insert(input_workspace.active_tab_id.clone());
                self.state.workspaces.push(input_workspace);
            }
            ReplicaEvent::WorkspaceUpdated {
                workspace: input_workspace,
            } => {
                let event = "workspace_updated";
                validate_workspace_wire(event, &input_workspace)?;
                let workspace = self
                    .state
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.workspace_id == input_workspace.workspace_id)
                    .ok_or_else(|| malformed_event(event, "updated workspace does not exist"))?;
                *workspace = input_workspace;
            }
            ReplicaEvent::WorkspaceRenamed {
                workspace_id: input_workspace_id,
                label: input_label,
            } => {
                let event = "workspace_renamed";
                let workspace = self
                    .state
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.workspace_id == input_workspace_id)
                    .ok_or_else(|| malformed_event(event, "renamed workspace does not exist"))?;
                workspace.label = input_label;
            }
            ReplicaEvent::WorkspaceMoved {
                workspace_id: input_workspace_id,
                insert_index: input_insert_index,
                workspaces: input_workspaces,
            } => {
                let event = "workspace_moved";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                for workspace in &input_workspaces {
                    validate_workspace_wire(event, workspace)?;
                }
                if input_insert_index > input_workspaces.len()
                    || !input_workspaces
                        .iter()
                        .any(|workspace| workspace.workspace_id == input_workspace_id)
                {
                    return Err(malformed_event(
                        event,
                        "resulting workspace order does not contain the moved workspace at a valid index",
                    ));
                }
                self.state.workspaces = input_workspaces;
            }
            ReplicaEvent::WorkspaceReordered {
                workspace_ids: input_workspace_ids,
                workspaces: input_workspaces,
            } => {
                let event = "workspace_reordered";
                for workspace in &input_workspaces {
                    validate_workspace_wire(event, workspace)?;
                }
                if input_workspace_ids.is_empty()
                    || input_workspace_ids.iter().any(|workspace_id| {
                        workspace_id.trim().is_empty()
                            || !input_workspaces
                                .iter()
                                .any(|workspace| &workspace.workspace_id == workspace_id)
                    })
                {
                    return Err(malformed_event(
                        event,
                        "resulting workspace order does not contain every reordered workspace",
                    ));
                }
                self.state.workspaces = input_workspaces;
            }
            ReplicaEvent::WorkspaceClosed {
                workspace_id: input_workspace_id,
            } => {
                let event = "workspace_closed";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                if !self
                    .state
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == input_workspace_id)
                {
                    return Err(malformed_event(event, "closed workspace does not exist"));
                }
                self.remove_workspace(&input_workspace_id);
            }
            ReplicaEvent::WorkspaceFocused {
                workspace_id: input_workspace_id,
            } => {
                let event = "workspace_focused";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                if !self
                    .state
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == input_workspace_id)
                {
                    return Err(malformed_event(event, "focused workspace does not exist"));
                }
                self.focus_workspace(input_workspace_id);
            }
            ReplicaEvent::WorktreeCreated {
                workspace: input_workspace,
            } => {
                let event = "worktree_created";
                validate_workspace_wire(event, &input_workspace)?;
                upsert_workspace(&mut self.state.workspaces, input_workspace);
            }
            ReplicaEvent::WorktreeOpened {
                workspace: input_workspace,
            } => {
                let event = "worktree_opened";
                validate_workspace_wire(event, &input_workspace)?;
                upsert_workspace(&mut self.state.workspaces, input_workspace);
            }
            ReplicaEvent::WorktreeRemoved {
                workspace_id: input_workspace_id,
                workspace: input_workspace,
            } => {
                let event = "worktree_removed";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                if let Some(workspace) = input_workspace {
                    validate_workspace_wire(event, &workspace)?;
                    if workspace.workspace_id != input_workspace_id {
                        return Err(malformed_event(
                            event,
                            "workspace does not match workspace_id",
                        ));
                    }
                    upsert_workspace(&mut self.state.workspaces, workspace);
                }
            }
            ReplicaEvent::TabCreated {
                tab: input_tab,
                focused,
            } => {
                let event = "tab_created";
                validate_tab_wire(event, &input_tab)?;
                if !self.has_workspace(&input_tab.workspace_id) {
                    // Herdr drains each subscribed kind on its own cursor, so a
                    // workspace created while a drain runs can reach the stream
                    // as its tab before its `workspace_created`.
                    self.defer_child(
                        event,
                        ReplicaEvent::TabCreated {
                            tab: input_tab,
                            focused,
                        },
                    )?;
                    return Ok(false);
                }
                if self
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.tab_id == input_tab.tab_id)
                {
                    if mode == ApplyMode::Reconcile
                        && self.state.tabs.iter().any(|tab| {
                            tab.tab_id == input_tab.tab_id
                                && tab.workspace_id == input_tab.workspace_id
                        })
                    {
                        // Bootstrap already holds this topology. Preserve only
                        // the buffered event's cause, without rewinding the snapshot.
                        if focused {
                            self.record_creation_focus(&input_tab);
                        }
                        return Ok(false);
                    }
                    return Err(malformed_event(event, "created tab already exists"));
                }
                // Creation already reports its focus. Publish it with membership,
                // so a later tab_focused event cannot look like a separate action
                // after Hide has first admitted the new tab without stealing focus.
                if focused {
                    let workspace = self
                        .state
                        .workspaces
                        .iter_mut()
                        .find(|workspace| workspace.workspace_id == input_tab.workspace_id)
                        .expect("created tab workspace was validated");
                    workspace.active_tab_id = input_tab.tab_id.clone();
                    self.focus_workspace(input_tab.workspace_id.clone());
                    self.pending_active_tab_focuses
                        .remove(&input_tab.workspace_id);
                    self.record_creation_focus(&input_tab);
                }
                self.cancel_workspace_closure(&input_tab.workspace_id);
                self.pending_layouts.insert(input_tab.tab_id.clone());
                self.state.tabs.push(input_tab);
            }
            ReplicaEvent::TabClosed {
                workspace_id: input_workspace_id,
                tab_id: input_tab_id,
            } => {
                let event = "tab_closed";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                let workspace_tab_count = self
                    .state
                    .tabs
                    .iter()
                    .filter(|tab| tab.workspace_id == input_workspace_id)
                    .count();
                let tab = self
                    .state
                    .tabs
                    .iter()
                    .find(|tab| tab.tab_id == input_tab_id)
                    .ok_or_else(|| malformed_event(event, "closed tab does not exist"))?;
                if tab.workspace_id != input_workspace_id {
                    return Err(malformed_event(
                        event,
                        "closed tab belongs to another workspace",
                    ));
                }
                let active_tab_closed = self
                    .state
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.workspace_id == input_workspace_id)
                    .is_some_and(|workspace| workspace.active_tab_id == input_tab_id);
                if workspace_tab_count == 1 {
                    self.pending_workspace_closures
                        .insert(input_workspace_id.clone());
                } else if active_tab_closed {
                    self.pending_active_tab_focuses
                        .insert(input_workspace_id.clone());
                }
                self.remove_tab(&input_tab_id);
            }
            ReplicaEvent::TabRenamed {
                workspace_id: input_workspace_id,
                tab_id: input_tab_id,
                label: input_label,
            } => {
                let event = "tab_renamed";
                let tab = self
                    .state
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.tab_id == input_tab_id)
                    .ok_or_else(|| malformed_event(event, "renamed tab does not exist"))?;
                if tab.workspace_id != input_workspace_id {
                    return Err(malformed_event(
                        event,
                        "renamed tab belongs to another workspace",
                    ));
                }
                tab.label = input_label;
            }
            ReplicaEvent::TabMoved {
                workspace_id: input_workspace_id,
                tab_id: input_tab_id,
                insert_index: input_insert_index,
                tabs: input_tabs,
            } => {
                let event = "tab_moved";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                ensure_non_empty(event, "tab_id", &input_tab_id)?;
                for tab in &input_tabs {
                    validate_tab_wire(event, tab)?;
                }
                if input_insert_index > input_tabs.len()
                    || input_tabs
                        .iter()
                        .any(|tab| tab.workspace_id != input_workspace_id)
                    || !input_tabs.iter().any(|tab| tab.tab_id == input_tab_id)
                {
                    return Err(malformed_event(
                        event,
                        "resulting tab order is inconsistent with the moved tab",
                    ));
                }
                self.state
                    .tabs
                    .retain(|tab| tab.workspace_id != input_workspace_id);
                self.state.tabs.extend(input_tabs);
            }
            ReplicaEvent::TabFocused {
                workspace_id: input_workspace_id,
                tab_id: input_tab_id,
            } => {
                let event = "tab_focused";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                ensure_non_empty(event, "tab_id", &input_tab_id)?;
                if self.has_workspace(&input_workspace_id)
                    && !self.state.tabs.iter().any(|tab| tab.tab_id == input_tab_id)
                {
                    self.defer_focus(
                        event,
                        ReplicaEvent::TabFocused {
                            workspace_id: input_workspace_id,
                            tab_id: input_tab_id,
                        },
                    )?;
                    return Ok(false);
                }
                let workspace = self
                    .state
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.workspace_id == input_workspace_id)
                    .ok_or_else(|| malformed_event(event, "focused workspace does not exist"))?;
                let tab = self
                    .state
                    .tabs
                    .iter()
                    .find(|tab| tab.tab_id == input_tab_id)
                    .ok_or_else(|| malformed_event(event, "focused tab does not exist"))?;
                if tab.workspace_id != input_workspace_id {
                    return Err(malformed_event(
                        event,
                        "focused tab belongs to another workspace",
                    ));
                }
                workspace.active_tab_id = input_tab_id.clone();
                self.tab_moves.record(input_tab_id.clone());
                // A focus Herdr applied after a held one supersedes it.
                self.early_focuses.retain(|held| {
                    !matches!(held, ReplicaEvent::TabFocused { workspace_id, .. } if workspace_id == &input_workspace_id)
                });
                self.state.tab_focus = Some(crate::sidebar::SessionTabFocus {
                    generation: self.generation,
                    workspace_id: input_workspace_id.clone(),
                    creation: self.pending_creation_focuses.remove(&input_tab_id),
                    tab_id: input_tab_id,
                    revision: self.applied_events.saturating_add(1),
                });
                self.focus_workspace(input_workspace_id.clone());
                self.pending_active_tab_focuses.remove(&input_workspace_id);
            }
            ReplicaEvent::PaneCreated { pane: input_pane } => {
                let event = "pane_created";
                validate_pane_wire(event, &input_pane)?;
                if !self
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.tab_id == input_pane.tab_id)
                {
                    self.defer_child(event, ReplicaEvent::PaneCreated { pane: input_pane })?;
                    return Ok(false);
                }
                if !self.state.tabs.iter().any(|tab| {
                    tab.tab_id == input_pane.tab_id && tab.workspace_id == input_pane.workspace_id
                }) {
                    return Err(malformed_event(
                        event,
                        "created pane references a tab in another workspace",
                    ));
                }
                if self
                    .state
                    .panes
                    .iter()
                    .any(|pane| pane.pane_id == input_pane.pane_id)
                {
                    return Err(malformed_event(event, "created pane already exists"));
                }
                self.pending_layouts.insert(input_pane.tab_id.clone());
                let pane_id = input_pane.pane_id.clone();
                self.state.panes.push(input_pane);
                self.await_cwd(pane_id);
            }
            ReplicaEvent::PaneClosed {
                workspace_id: input_workspace_id,
                pane_id: input_pane_id,
            } => {
                let event = "pane_closed";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                let pane = self
                    .state
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == input_pane_id)
                    .ok_or_else(|| malformed_event(event, "closed pane does not exist"))?;
                if pane.workspace_id != input_workspace_id {
                    return Err(malformed_event(
                        event,
                        "closed pane belongs to another workspace",
                    ));
                }
                let tab_id = pane.tab_id.clone();
                let last_pane_in_tab = self
                    .state
                    .panes
                    .iter()
                    .filter(|candidate| candidate.tab_id == tab_id)
                    .count()
                    == 1;
                let workspace_tab_count = self
                    .state
                    .tabs
                    .iter()
                    .filter(|tab| tab.workspace_id == input_workspace_id)
                    .count();
                if last_pane_in_tab && workspace_tab_count > 1 {
                    let active_tab_closed = self
                        .state
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.workspace_id == input_workspace_id)
                        .is_some_and(|workspace| workspace.active_tab_id == tab_id);
                    if active_tab_closed {
                        self.pending_active_tab_focuses
                            .insert(input_workspace_id.clone());
                    }
                    // Herdr 0.8.2 removes an emptied tab as part of pane.close
                    // without emitting a separate tab.closed event. Mirror that
                    // authoritative cascade so the replica cannot wait forever
                    // for a layout.updated event for a tab that no longer exists.
                    self.remove_tab(&tab_id);
                } else {
                    self.pending_layouts.insert(tab_id);
                    self.remove_pane(&input_pane_id);
                }
            }
            ReplicaEvent::PaneUpdated { pane: input_pane } => {
                let event = "pane_updated";
                validate_pane_wire(event, &input_pane)?;
                let previous = self
                    .state
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == input_pane.pane_id)
                    .cloned()
                    .ok_or_else(|| malformed_event(event, "updated pane does not exist"))?;
                if previous.tab_id != input_pane.tab_id
                    || previous.workspace_id != input_pane.workspace_id
                {
                    self.pending_layouts.insert(previous.tab_id);
                    self.pending_layouts.insert(input_pane.tab_id.clone());
                }
                upsert_pane(&mut self.state.panes, input_pane);
            }
            ReplicaEvent::PaneFocused {
                workspace_id: input_workspace_id,
                pane_id: input_pane_id,
            } => {
                let event = "pane_focused";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                if self.has_workspace(&input_workspace_id)
                    && !self
                        .state
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == input_pane_id)
                {
                    self.defer_focus(
                        event,
                        ReplicaEvent::PaneFocused {
                            workspace_id: input_workspace_id,
                            pane_id: input_pane_id,
                        },
                    )?;
                    return Ok(false);
                }
                if !self.state.panes.iter().any(|pane| {
                    pane.pane_id == input_pane_id && pane.workspace_id == input_workspace_id
                }) {
                    return Err(malformed_event(
                        event,
                        "focused pane does not exist in the stated workspace",
                    ));
                }
                self.state.focused_pane_id = Some(input_pane_id.clone());
                self.early_focuses
                    .retain(|held| !matches!(held, ReplicaEvent::PaneFocused { .. }));
                self.focus_workspace(input_workspace_id.clone());
                if let Some(layout) = self.state.layouts.iter_mut().find(|layout| {
                    layout
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == input_pane_id)
                }) {
                    layout.focused_pane_id = input_pane_id;
                }
            }
            ReplicaEvent::PaneMoved(payload) => {
                self.apply_pane_moved(payload)?;
                return Ok(true);
            }
            ReplicaEvent::PaneExited {
                workspace_id: input_workspace_id,
                pane_id: input_pane_id,
            } => {
                let event = "pane_exited";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                ensure_non_empty(event, "pane_id", &input_pane_id)?;
            }
            ReplicaEvent::PaneAgentDetected {
                workspace_id: input_workspace_id,
                pane_id: input_pane_id,
            } => {
                let event = "pane_agent_detected";
                ensure_non_empty(event, "workspace_id", &input_workspace_id)?;
                ensure_non_empty(event, "pane_id", &input_pane_id)?;
                return Ok(true);
            }
            ReplicaEvent::LayoutUpdated {
                layout: input_layout,
            } => {
                let event = "layout_updated";
                ensure_non_empty(event, "workspace_id", &input_layout.workspace_id)?;
                ensure_non_empty(event, "tab_id", &input_layout.tab_id)?;
                ensure_non_empty(event, "focused_pane_id", &input_layout.focused_pane_id)?;
                if !self
                    .state
                    .tabs
                    .iter()
                    .any(|tab| tab.tab_id == input_layout.tab_id)
                {
                    self.defer_child(
                        event,
                        ReplicaEvent::LayoutUpdated {
                            layout: input_layout,
                        },
                    )?;
                    return Ok(false);
                }
                if !self.state.tabs.iter().any(|tab| {
                    tab.tab_id == input_layout.tab_id
                        && tab.workspace_id == input_layout.workspace_id
                }) {
                    return Err(malformed_event(
                        event,
                        "layout references a tab in another workspace",
                    ));
                }
                let tab_id = input_layout.tab_id.clone();
                upsert_layout(&mut self.state.layouts, input_layout);
                self.pending_layouts.remove(&tab_id);
            }
            ReplicaEvent::Unrequested(unknown) => {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr subscription emitted unrequested event {unknown:?}"
                )));
            }
        }
        Ok(false)
    }

    fn apply_pane_moved(&mut self, payload: PaneMove) -> Result<(), SessionFetchError> {
        ensure_non_empty("pane_moved", "previous_pane_id", &payload.previous_pane_id)?;
        ensure_non_empty(
            "pane_moved",
            "previous_workspace_id",
            &payload.previous_workspace_id,
        )?;
        ensure_non_empty("pane_moved", "previous_tab_id", &payload.previous_tab_id)?;
        validate_pane_wire("pane_moved", &payload.pane)?;
        if let Some(previous) = self
            .state
            .panes
            .iter()
            .find(|pane| pane.pane_id == payload.previous_pane_id)
            && (previous.workspace_id != payload.previous_workspace_id
                || previous.tab_id != payload.previous_tab_id)
        {
            return Err(malformed_event(
                "pane_moved",
                "previous pane scope does not match the replica",
            ));
        }
        let previous_agent = self
            .state
            .agents
            .iter()
            .find(|agent| agent.pane_id == payload.previous_pane_id)
            .cloned();
        let previous_tab_id = payload.previous_tab_id.clone();
        self.remove_pane(&payload.previous_pane_id);

        if let Some(workspace) = payload.created_workspace {
            validate_workspace_wire("pane_moved", &workspace)?;
            upsert_workspace(&mut self.state.workspaces, workspace);
        }
        if let Some(tab) = payload.created_tab {
            validate_tab_wire("pane_moved", &tab)?;
            self.cancel_workspace_closure(&tab.workspace_id);
            self.pending_layouts.insert(tab.tab_id.clone());
            upsert_tab(&mut self.state.tabs, tab);
        }
        if let Some(tab_id) = payload.closed_tab_id.as_deref() {
            self.remove_tab(tab_id);
        } else {
            self.pending_layouts.insert(previous_tab_id);
        }
        if let Some(workspace_id) = payload.closed_workspace_id.as_deref() {
            self.remove_workspace(workspace_id);
        }

        self.pending_layouts.insert(payload.pane.tab_id.clone());
        let pane = payload.pane;
        if let Some(mut agent) = previous_agent {
            agent.pane_id = pane.pane_id.clone();
            agent.workspace_id = pane.workspace_id.clone();
            agent.tab_id = pane.tab_id.clone();
            agent.cwd = pane.cwd.clone();
            self.state
                .agents
                .retain(|candidate| candidate.pane_id != agent.pane_id);
            self.state.agents.push(agent);
        }
        upsert_pane(&mut self.state.panes, pane);
        Ok(())
    }

    /// Whether `event` is about an id an earlier event removed (`retired_ids`).
    fn names_retired<'a>(&self, event: &'a ReplicaEvent) -> Option<(&'static str, &'a str)> {
        let workspace = |id: &String| self.retired_ids.contains(id) && !self.has_workspace(id);
        let tab = |id: &String| {
            self.retired_ids.contains(id) && !self.state.tabs.iter().any(|tab| &tab.tab_id == id)
        };
        let pane = |id: &String| {
            self.retired_ids.contains(id)
                && !self.state.panes.iter().any(|pane| &pane.pane_id == id)
        };
        let (event, id, retired) = match event {
            ReplicaEvent::WorkspaceFocused { workspace_id } => {
                ("workspace_focused", workspace_id, workspace(workspace_id))
            }
            ReplicaEvent::TabCreated { tab: created, .. } => (
                "tab_created",
                &created.workspace_id,
                workspace(&created.workspace_id),
            ),
            ReplicaEvent::TabClosed { tab_id, .. } => ("tab_closed", tab_id, tab(tab_id)),
            ReplicaEvent::TabFocused { tab_id, .. } => ("tab_focused", tab_id, tab(tab_id)),
            ReplicaEvent::TabRenamed { tab_id, .. } => ("tab_renamed", tab_id, tab(tab_id)),
            ReplicaEvent::PaneCreated { pane: created } => {
                ("pane_created", &created.tab_id, tab(&created.tab_id))
            }
            ReplicaEvent::PaneClosed { pane_id, .. } => ("pane_closed", pane_id, pane(pane_id)),
            ReplicaEvent::PaneFocused { pane_id, .. } => ("pane_focused", pane_id, pane(pane_id)),
            ReplicaEvent::PaneUpdated { pane: updated } => {
                ("pane_updated", &updated.pane_id, pane(&updated.pane_id))
            }
            ReplicaEvent::LayoutUpdated { layout } => {
                ("layout_updated", &layout.tab_id, tab(&layout.tab_id))
            }
            _ => return None,
        };
        retired.then_some((event, id.as_str()))
    }

    /// A tab created in a workspace whose last tab closed means the workspace
    /// survived (a seed tab that `layout.apply` replaced), so it no longer
    /// waits for `workspace_closed`; its active tab is read from Herdr unless
    /// an event names it first.
    fn cancel_workspace_closure(&mut self, workspace_id: &str) {
        if self.pending_workspace_closures.remove(workspace_id) {
            self.pending_active_tab_focuses
                .insert(workspace_id.to_owned());
        }
    }

    fn retire(&mut self, id: &str) {
        if self.retired_ids.len() == RETIRED_ID_WINDOW {
            self.retired_ids.pop_front();
        }
        self.retired_ids.push_back(id.to_owned());
    }

    fn has_workspace(&self, workspace_id: &str) -> bool {
        self.state
            .workspaces
            .iter()
            .any(|workspace| workspace.workspace_id == workspace_id)
    }

    /// Holds a focus event until its tab or pane is announced, replacing an
    /// earlier held focus of the same kind in the same workspace.
    fn defer_focus(&mut self, event: &str, focus: ReplicaEvent) -> Result<(), SessionFetchError> {
        let same = |held: &ReplicaEvent| match (held, &focus) {
            (
                ReplicaEvent::TabFocused {
                    workspace_id: a, ..
                },
                ReplicaEvent::TabFocused {
                    workspace_id: b, ..
                },
            )
            | (
                ReplicaEvent::PaneFocused {
                    workspace_id: a, ..
                },
                ReplicaEvent::PaneFocused {
                    workspace_id: b, ..
                },
            ) => a == b,
            _ => false,
        };
        self.early_focuses.retain(|held| !same(held));
        if self.early_focuses.len() == EARLY_FOCUS_LIMIT {
            return Err(malformed_event(
                event,
                "too many focus events ahead of their tabs and panes",
            ));
        }
        self.early_focuses.push(focus);
        Ok(())
    }

    /// Holds a tab event until its workspace is announced, and a pane or
    /// layout event until its tab is.
    fn defer_child(&mut self, event: &str, child: ReplicaEvent) -> Result<(), SessionFetchError> {
        if self.early_children.len() == EARLY_CHILD_LIMIT {
            return Err(malformed_event(
                event,
                "too many tab, pane and layout events ahead of their parents",
            ));
        }
        self.early_children.push(child);
        Ok(())
    }

    /// Applies, in arrival order, each held event whose parent has now been
    /// announced, until none is left whose parent is: a tab released by its
    /// workspace releases the panes and layouts held for it.
    fn apply_early_children(&mut self, mode: ApplyMode) -> Result<(), SessionFetchError> {
        loop {
            let (ready, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.early_children)
                .into_iter()
                .partition(|child| match child {
                    ReplicaEvent::TabCreated { tab, .. } => self.has_workspace(&tab.workspace_id),
                    _ => child.subject().0.is_some_and(|tab_id| {
                        self.state.tabs.iter().any(|tab| tab.tab_id == tab_id)
                    }),
                });
            self.early_children = held;
            if ready.is_empty() {
                return Ok(());
            }
            for child in ready {
                self.apply_new_event(child, mode)?;
            }
        }
    }

    /// Applies each held focus whose tab or pane has now been announced.
    fn apply_early_focuses(&mut self, mode: ApplyMode) -> Result<(), SessionFetchError> {
        let (ready, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.early_focuses)
            .into_iter()
            .partition(|focus| match focus {
                ReplicaEvent::TabFocused { tab_id, .. } => {
                    self.state.tabs.iter().any(|tab| &tab.tab_id == tab_id)
                }
                ReplicaEvent::PaneFocused { pane_id, .. } => {
                    self.state.panes.iter().any(|pane| &pane.pane_id == pane_id)
                }
                _ => false,
            });
        self.early_focuses = held;
        for focus in ready {
            self.apply_new_event(focus, mode)?;
        }
        Ok(())
    }

    fn remove_workspace(&mut self, workspace_id: &str) {
        self.retire(workspace_id);
        let retired = self
            .state
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .map(|tab| tab.tab_id.clone())
            .chain(
                self.state
                    .panes
                    .iter()
                    .filter(|pane| pane.workspace_id == workspace_id)
                    .map(|pane| pane.pane_id.clone()),
            )
            .collect::<Vec<_>>();
        for id in &retired {
            self.retire(id);
        }
        let tab_ids = self
            .state
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == workspace_id)
            .map(|tab| tab.tab_id.clone())
            .collect::<HashSet<_>>();
        self.state
            .workspaces
            .retain(|workspace| workspace.workspace_id != workspace_id);
        if self.state.focused_workspace_id.as_deref() == Some(workspace_id) {
            self.state.focused_workspace_id = None;
            if !self.state.workspaces.is_empty() {
                self.pending_focus_after_close = Some(workspace_id.to_owned());
            }
        }
        self.state
            .tabs
            .retain(|tab| tab.workspace_id != workspace_id);
        self.state
            .panes
            .retain(|pane| pane.workspace_id != workspace_id);
        self.unconfirmed_cwds
            .retain(|pane_id, _| self.state.panes.iter().any(|pane| &pane.pane_id == pane_id));
        self.state
            .layouts
            .retain(|layout| layout.workspace_id != workspace_id);
        self.state
            .agents
            .retain(|agent| agent.workspace_id != workspace_id);
        self.pending_layouts
            .retain(|tab_id| !tab_ids.contains(tab_id));
        self.pending_workspace_closures.remove(workspace_id);
        self.pending_active_tab_focuses.remove(workspace_id);
        self.early_children.retain(|child| match child {
            ReplicaEvent::TabCreated { tab, .. } => tab.workspace_id != workspace_id,
            ReplicaEvent::PaneCreated { pane } => pane.workspace_id != workspace_id,
            ReplicaEvent::LayoutUpdated { layout } => layout.workspace_id != workspace_id,
            _ => true,
        });
        self.early_focuses.retain(|focus| match focus {
            ReplicaEvent::TabFocused {
                workspace_id: held, ..
            }
            | ReplicaEvent::PaneFocused {
                workspace_id: held, ..
            } => held != workspace_id,
            _ => true,
        });
        self.clear_missing_focus();
    }

    fn remove_tab(&mut self, tab_id: &str) {
        self.retire(tab_id);
        let pane_ids = self
            .state
            .panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id)
            .map(|pane| pane.pane_id.clone())
            .collect::<Vec<_>>();
        for pane_id in &pane_ids {
            self.retire(pane_id);
        }
        self.early_focuses.retain(|focus| match focus {
            ReplicaEvent::TabFocused { tab_id: held, .. } => held != tab_id,
            ReplicaEvent::PaneFocused { pane_id: held, .. } => !pane_ids.contains(held),
            _ => true,
        });
        self.state.tabs.retain(|tab| tab.tab_id != tab_id);
        self.state.panes.retain(|pane| pane.tab_id != tab_id);
        for pane_id in &pane_ids {
            self.unconfirmed_cwds.remove(pane_id);
        }
        self.state.layouts.retain(|layout| layout.tab_id != tab_id);
        self.state.agents.retain(|agent| agent.tab_id != tab_id);
        self.pending_layouts.remove(tab_id);
        self.clear_missing_focus();
    }

    fn remove_pane(&mut self, pane_id: &str) {
        self.retire(pane_id);
        self.early_focuses.retain(|focus| {
            !matches!(focus, ReplicaEvent::PaneFocused { pane_id: held, .. } if held == pane_id)
        });
        self.state.panes.retain(|pane| pane.pane_id != pane_id);
        self.unconfirmed_cwds.remove(pane_id);
        self.state.agents.retain(|agent| agent.pane_id != pane_id);
        self.clear_missing_focus();
    }

    fn record_creation_focus(&mut self, tab: &ProjectedTab) {
        self.pending_creation_focuses.insert(tab.tab_id.clone());
        if self.state.focused_workspace_id.as_ref() == Some(&tab.workspace_id)
            && self.state.workspaces.iter().any(|workspace| {
                workspace.workspace_id == tab.workspace_id && workspace.active_tab_id == tab.tab_id
            })
        {
            self.state.tab_focus = Some(crate::sidebar::SessionTabFocus {
                generation: self.generation,
                workspace_id: tab.workspace_id.clone(),
                tab_id: tab.tab_id.clone(),
                revision: self.applied_events.saturating_add(1),
                creation: true,
            });
        }
    }

    fn clear_missing_focus(&mut self) {
        self.pending_creation_focuses
            .retain(|id| self.state.tabs.iter().any(|tab| &tab.tab_id == id));
        if self
            .state
            .tab_focus
            .as_ref()
            .is_some_and(|focus| !self.state.tabs.iter().any(|tab| tab.tab_id == focus.tab_id))
        {
            self.state.tab_focus = None;
        }
        if self
            .state
            .focused_pane_id
            .as_ref()
            .is_some_and(|focused| !self.state.panes.iter().any(|pane| &pane.pane_id == focused))
        {
            self.state.focused_pane_id = None;
        }
    }

    fn validate_active_tabs(&self) -> Result<(), SessionFetchError> {
        let tabs_by_id = self
            .state
            .tabs
            .iter()
            .map(|tab| (tab.tab_id.as_str(), tab))
            .collect::<HashMap<_, _>>();
        for workspace in &self.state.workspaces {
            let Some(active_tab) = tabs_by_id.get(workspace.active_tab_id.as_str()) else {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr workspace {} references missing active tab {}",
                    workspace.workspace_id, workspace.active_tab_id
                )));
            };
            if active_tab.workspace_id != workspace.workspace_id {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr workspace {} references active tab {} from another workspace",
                    workspace.workspace_id, workspace.active_tab_id
                )));
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), SessionFetchError> {
        for workspace in &self.state.workspaces {
            if let Some(worktree) = &workspace.worktree {
                for (field, value) in [
                    ("repo_key", worktree.repo_key.as_str()),
                    ("repo_name", worktree.repo_name.as_str()),
                    ("repo_root", worktree.repo_root.as_str()),
                    ("checkout_path", worktree.checkout_path.as_str()),
                ] {
                    if value.trim().is_empty() {
                        return Err(SessionFetchError::Malformed(format!(
                            "Herdr workspace {} has an empty worktree {field}",
                            workspace.workspace_id
                        )));
                    }
                }
            }
        }
        let workspace_ids = unique_ids(
            "workspace",
            self.state
                .workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.as_str()),
        )?;
        let tab_ids = unique_ids("tab", self.state.tabs.iter().map(|tab| tab.tab_id.as_str()))?;
        let pane_ids = unique_ids(
            "pane",
            self.state.panes.iter().map(|pane| pane.pane_id.as_str()),
        )?;
        let tabs_by_id = self
            .state
            .tabs
            .iter()
            .map(|tab| (tab.tab_id.as_str(), tab))
            .collect::<HashMap<_, _>>();
        for tab in &self.state.tabs {
            if !workspace_ids.contains(tab.workspace_id.as_str()) {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr tab {} references missing workspace {}",
                    tab.tab_id, tab.workspace_id
                )));
            }
        }
        let panes_by_id = self
            .state
            .panes
            .iter()
            .map(|pane| (pane.pane_id.as_str(), pane))
            .collect::<HashMap<_, _>>();
        for pane in &self.state.panes {
            if !workspace_ids.contains(pane.workspace_id.as_str())
                || !tab_ids.contains(pane.tab_id.as_str())
            {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr pane {} references missing workspace or tab",
                    pane.pane_id
                )));
            }
            if tabs_by_id
                .get(pane.tab_id.as_str())
                .is_some_and(|tab| tab.workspace_id != pane.workspace_id)
            {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr pane {} references a tab from another workspace",
                    pane.pane_id
                )));
            }
        }
        let mut layout_tabs = HashSet::new();
        let mut laid_out_panes = HashSet::new();
        for layout in &self.state.layouts {
            if !layout_tabs.insert(layout.tab_id.as_str()) {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr session contains duplicate layout for tab {}",
                    layout.tab_id
                )));
            }
            if !workspace_ids.contains(layout.workspace_id.as_str())
                || !tab_ids.contains(layout.tab_id.as_str())
            {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr layout {} references missing workspace or tab",
                    layout.tab_id
                )));
            }
            if tabs_by_id
                .get(layout.tab_id.as_str())
                .is_some_and(|tab| tab.workspace_id != layout.workspace_id)
            {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr layout {} belongs to the wrong workspace",
                    layout.tab_id
                )));
            }
            if !layout
                .panes
                .iter()
                .any(|pane| pane.pane_id == layout.focused_pane_id)
            {
                return Err(SessionFetchError::Malformed(format!(
                    "Herdr layout {} focuses a pane outside that layout",
                    layout.tab_id
                )));
            }
            for layout_pane in &layout.panes {
                let Some(pane) = panes_by_id.get(layout_pane.pane_id.as_str()) else {
                    return Err(SessionFetchError::Malformed(format!(
                        "Herdr layout {} references missing pane {}",
                        layout.tab_id, layout_pane.pane_id
                    )));
                };
                if pane.tab_id != layout.tab_id || pane.workspace_id != layout.workspace_id {
                    return Err(SessionFetchError::Malformed(format!(
                        "Herdr layout {} contains pane {} from another tab",
                        layout.tab_id, layout_pane.pane_id
                    )));
                }
                if !laid_out_panes.insert(layout_pane.pane_id.as_str()) {
                    return Err(SessionFetchError::Malformed(format!(
                        "Herdr pane {} appears in multiple layouts",
                        layout_pane.pane_id
                    )));
                }
            }
        }
        if let Some(missing_tab_id) = tab_ids
            .iter()
            .find(|tab_id| !layout_tabs.contains(**tab_id))
        {
            return Err(SessionFetchError::Malformed(format!(
                "Herdr tab {missing_tab_id} has no layout"
            )));
        }
        if let Some(missing_pane_id) = pane_ids
            .iter()
            .find(|pane_id| !laid_out_panes.contains(**pane_id))
        {
            return Err(SessionFetchError::Malformed(format!(
                "Herdr pane {missing_pane_id} is absent from every layout"
            )));
        }
        if let Some(focused) = self.state.focused_pane_id.as_deref()
            && !pane_ids.contains(focused)
        {
            return Err(SessionFetchError::Malformed(format!(
                "Herdr session focuses missing pane {focused}"
            )));
        }
        Ok(())
    }
}

fn remote_workspace_id(target_id: &str, workspace_id: &str) -> String {
    format!("remote:{target_id}:workspace:{workspace_id}")
}

fn remote_checkout_id(target_id: &str, workspace_id: &str) -> String {
    format!("remote:{target_id}:checkout:{workspace_id}")
}

pub(crate) fn remote_tab_id(target_id: &str, tab_id: &str) -> String {
    format!("remote:{target_id}:tab:{tab_id}")
}

pub(crate) fn remote_pane_id(target_id: &str, pane_id: &str) -> String {
    format!("remote:{target_id}:pane:{pane_id}")
}

fn unique_ids<'a>(
    kind: &str,
    ids: impl Iterator<Item = &'a str>,
) -> Result<HashSet<&'a str>, SessionFetchError> {
    let mut unique = HashSet::new();
    for id in ids {
        if id.trim().is_empty() {
            return Err(SessionFetchError::Malformed(format!(
                "Herdr {kind} id is empty"
            )));
        }
        if !unique.insert(id) {
            return Err(SessionFetchError::Malformed(format!(
                "Herdr session contains duplicate {kind} id {id}"
            )));
        }
    }
    Ok(unique)
}

fn ensure_non_empty(event: &str, field: &str, value: &str) -> Result<(), SessionFetchError> {
    if value.trim().is_empty() {
        Err(malformed_event(event, &format!("has empty {field}")))
    } else {
        Ok(())
    }
}

fn validate_workspace_wire(
    event: &str,
    workspace: &ProjectedWorkspace,
) -> Result<(), SessionFetchError> {
    ensure_non_empty(event, "workspace.workspace_id", &workspace.workspace_id)?;
    ensure_non_empty(event, "workspace.active_tab_id", &workspace.active_tab_id)?;
    if let Some(worktree) = &workspace.worktree {
        ensure_non_empty(event, "workspace.worktree.repo_key", &worktree.repo_key)?;
        ensure_non_empty(event, "workspace.worktree.repo_name", &worktree.repo_name)?;
        ensure_non_empty(event, "workspace.worktree.repo_root", &worktree.repo_root)?;
        ensure_non_empty(
            event,
            "workspace.worktree.checkout_path",
            &worktree.checkout_path,
        )?;
    }
    Ok(())
}

fn validate_tab_wire(event: &str, tab: &ProjectedTab) -> Result<(), SessionFetchError> {
    ensure_non_empty(event, "tab.tab_id", &tab.tab_id)?;
    ensure_non_empty(event, "tab.workspace_id", &tab.workspace_id)
}

fn validate_pane_wire(event: &str, pane: &ProjectedPane) -> Result<(), SessionFetchError> {
    ensure_non_empty(event, "pane.pane_id", &pane.pane_id)?;
    ensure_non_empty(event, "pane.workspace_id", &pane.workspace_id)?;
    ensure_non_empty(event, "pane.tab_id", &pane.tab_id)
}

fn upsert_workspace(workspaces: &mut Vec<ProjectedWorkspace>, workspace: ProjectedWorkspace) {
    if let Some(existing) = workspaces
        .iter_mut()
        .find(|existing| existing.workspace_id == workspace.workspace_id)
    {
        *existing = workspace;
    } else {
        workspaces.push(workspace);
    }
}

fn upsert_tab(tabs: &mut Vec<ProjectedTab>, tab: ProjectedTab) {
    if let Some(existing) = tabs
        .iter_mut()
        .find(|existing| existing.tab_id == tab.tab_id)
    {
        *existing = tab;
    } else {
        tabs.push(tab);
    }
}

fn upsert_pane(panes: &mut Vec<ProjectedPane>, mut pane: ProjectedPane) {
    if let Some(existing) = panes
        .iter_mut()
        .find(|existing| existing.pane_id == pane.pane_id)
    {
        if pane.restore_error != existing.restore_error || pane.terminal_id != existing.terminal_id
        {
            report_restore_failure(&pane);
        }
        pane.foreground_process = existing.foreground_process.clone();
        *existing = pane;
    } else {
        report_restore_failure(&pane);
        panes.push(pane);
    }
}

fn report_restore_failure(pane: &ProjectedPane) {
    if let Some(error) = &pane.restore_error {
        crate::diagnostic!(serde_json::json!({
            "component": "session_sync", "kind": "terminal_restore_failed",
            "pane_id": pane.pane_id, "terminal_id": pane.terminal_id, "detail": error,
        }));
    }
}

fn upsert_layout(layouts: &mut Vec<SessionLayoutPayload>, layout: SessionLayoutPayload) {
    if let Some(existing) = layouts
        .iter_mut()
        .find(|existing| existing.tab_id == layout.tab_id)
    {
        *existing = layout;
    } else {
        layouts.push(layout);
    }
}

/// Inputs to replica transitions, with transport details removed by the boundary.
// The generated event boundary projects owned values directly into this
// transition enum; changing its representation belongs with measured sync work.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub(crate) enum ReplicaEvent {
    WorkspaceCreated {
        workspace: ProjectedWorkspace,
    },
    WorkspaceUpdated {
        workspace: ProjectedWorkspace,
    },
    WorkspaceRenamed {
        workspace_id: String,
        label: String,
    },
    WorkspaceMoved {
        workspace_id: String,
        insert_index: usize,
        workspaces: Vec<ProjectedWorkspace>,
    },
    WorkspaceReordered {
        workspace_ids: Vec<String>,
        workspaces: Vec<ProjectedWorkspace>,
    },
    WorkspaceClosed {
        workspace_id: String,
    },
    WorkspaceFocused {
        workspace_id: String,
    },
    WorktreeCreated {
        workspace: ProjectedWorkspace,
    },
    WorktreeOpened {
        workspace: ProjectedWorkspace,
    },
    WorktreeRemoved {
        workspace_id: String,
        workspace: Option<ProjectedWorkspace>,
    },
    TabCreated {
        tab: ProjectedTab,
        focused: bool,
    },
    TabClosed {
        workspace_id: String,
        tab_id: String,
    },
    TabRenamed {
        workspace_id: String,
        tab_id: String,
        label: String,
    },
    TabMoved {
        workspace_id: String,
        tab_id: String,
        insert_index: usize,
        tabs: Vec<ProjectedTab>,
    },
    TabFocused {
        workspace_id: String,
        tab_id: String,
    },
    PaneCreated {
        pane: ProjectedPane,
    },
    PaneClosed {
        workspace_id: String,
        pane_id: String,
    },
    PaneUpdated {
        pane: ProjectedPane,
    },
    PaneFocused {
        workspace_id: String,
        pane_id: String,
    },
    PaneMoved(PaneMove),
    PaneExited {
        workspace_id: String,
        pane_id: String,
    },
    PaneAgentDetected {
        workspace_id: String,
        pane_id: String,
    },
    LayoutUpdated {
        layout: SessionLayoutPayload,
    },
    Unrequested(String),
}
impl ReplicaEvent {
    /// The tab and pane this event names, and whether it carries a layout:
    /// what the stage record of a Hide-started operation needs (PRD
    /// instant-pane-topology D-14).
    pub(crate) fn subject(&self) -> (Option<&str>, Option<&str>, bool) {
        match self {
            Self::TabCreated { tab, .. } => (Some(&tab.tab_id), None, false),
            Self::TabClosed { tab_id, .. }
            | Self::TabRenamed { tab_id, .. }
            | Self::TabMoved { tab_id, .. }
            | Self::TabFocused { tab_id, .. } => (Some(tab_id), None, false),
            Self::PaneCreated { pane } | Self::PaneUpdated { pane } => {
                (Some(&pane.tab_id), Some(&pane.pane_id), false)
            }
            Self::PaneClosed { pane_id, .. }
            | Self::PaneFocused { pane_id, .. }
            | Self::PaneExited { pane_id, .. }
            | Self::PaneAgentDetected { pane_id, .. } => (None, Some(pane_id), false),
            Self::PaneMoved(moved) => (Some(&moved.pane.tab_id), Some(&moved.pane.pane_id), false),
            Self::LayoutUpdated { layout } => (Some(&layout.tab_id), None, true),
            Self::WorkspaceCreated { .. }
            | Self::WorkspaceUpdated { .. }
            | Self::WorkspaceRenamed { .. }
            | Self::WorkspaceMoved { .. }
            | Self::WorkspaceReordered { .. }
            | Self::WorkspaceClosed { .. }
            | Self::WorkspaceFocused { .. }
            | Self::WorktreeCreated { .. }
            | Self::WorktreeOpened { .. }
            | Self::WorktreeRemoved { .. }
            | Self::Unrequested(_) => (None, None, false),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PaneMove {
    pub(crate) previous_pane_id: String,
    pub(crate) previous_workspace_id: String,
    pub(crate) previous_tab_id: String,
    pub(crate) pane: ProjectedPane,
    pub(crate) created_workspace: Option<ProjectedWorkspace>,
    pub(crate) created_tab: Option<ProjectedTab>,
    pub(crate) closed_workspace_id: Option<String>,
    pub(crate) closed_tab_id: Option<String>,
}

// Boxing Event would add an allocation to every subscription line merely to
// shrink the uncommon error variant's stack footprint.
#[allow(clippy::large_enum_variant)]
pub(crate) enum SubscriptionLine {
    Event(ReplicaEvent),
    /// Herdr dropped this subscriber because it fell behind the server's
    /// retained event history, and closes the connection after this line.
    /// Herdr is answering; the replica has only missed events, so the cure is
    /// a fresh subscription and snapshot, not a failure.
    EventsLost {
        message: String,
    },
    Error {
        code: String,
        message: String,
    },
}

fn malformed_event(event: &str, detail: &str) -> SessionFetchError {
    SessionFetchError::Malformed(format!("Herdr {event} event {detail}"))
}
