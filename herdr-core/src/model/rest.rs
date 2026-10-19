//! Immutable semantic sections shared by transports and internal readers.
use super::*;
use serde::ser::SerializeMap;

#[derive(Clone, Debug)]
pub struct Retained<T> {
    pub value: Arc<T>,
    pub revision: u64,
    generation: Option<u64>,
}

impl<T: Clone + PartialEq> Retained<T> {
    fn capture(value: &T, previous: Option<&Self>, revision: u64, generation: Option<u64>) -> Self {
        if let Some(previous) = previous
            && (generation.is_some() && previous.generation == generation
                || generation.is_none()
                    && previous.generation.is_none()
                    && previous.value.as_ref() == value)
        {
            return previous.clone();
        }
        Self {
            value: Arc::new(value.clone()),
            revision,
            generation,
        }
    }
}

macro_rules! generation {
    () => {
        None
    };
    ($value:expr) => {
        $value
    };
}

impl<T> std::ops::Deref for Retained<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Serialize> Serialize for Retained<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

/// Each field has one semantic replacement and one stamp. This is a fixed
/// projection of the supported snapshot, not a row-patch protocol.
macro_rules! section {
    ($name:ident, $source:ty, $s:ident,
     { $($field:ident: $ty:ty => $value:expr $(; generation $generation:expr)?),* $(,)? },
     { $($group:ident: $group_ty:ident => $group_source:expr),* $(,)? }) => {
        #[derive(Clone, Debug)]
        pub struct $name {
            $(pub $field: Retained<$ty>,)*
            $(pub $group: $group_ty,)*
        }
        impl $name {
            pub fn capture($s: &$source, previous: Option<&Self>, revision: u64) -> Self {
                Self {
                    $($field: Retained::capture(&$value, previous.map(|old| &old.$field), revision, generation!($($generation)?)),)*
                    $($group: $group_ty::capture(&$group_source, previous.map(|old| &old.$group), revision),)*
                }
            }
            pub fn revision(&self) -> u64 {
                let mut revision = 0;
                $(revision = revision.max(self.$field.revision);)*
                $(revision = revision.max(self.$group.revision());)*
                revision
            }
            fn write<S: serde::Serializer>(&self, serializer: S, have: u64, legacy: bool) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                $(if self.$field.revision > have {
                    // A legacy complete rest omitted an absent Workspace.
                    if !(legacy && self.legacy_omit(stringify!($field))) {
                        map.serialize_entry(stringify!($field), self.$field.value.as_ref())?;
                    }
                })*
                $(if self.$group.revision() > have {
                    map.serialize_entry(stringify!($group), &SectionWire { section: &self.$group, have, legacy })?;
                })*
                map.end()
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.write(serializer, 0, false)
            }
        }
        impl Serialize for SectionWire<'_, $name> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.section.write(serializer, self.have, self.legacy)
            }
        }
    }
}

struct SectionWire<'a, T> {
    section: &'a T,
    have: u64,
    legacy: bool,
}

section!(NavigatorSections, NavigatorSnapshot, s, {
    agent_scope: crate::agent_state::Scope => s.agent_scope,
    root_path: Option<String> => s.root_path,
    changes_root_path: Option<String> => s.changes_root_path,
    focused_device_id: Option<String> => s.focused_device_id,
    focused_workspace_id: Option<String> => s.focused_workspace_id,
    focused_checkout_id: Option<String> => s.focused_checkout_id,
    devices: Vec<DeviceSnapshot> => s.devices,
    workspaces: Vec<WorkspaceSnapshot> => s.workspaces,
    inactive_projects: Vec<InactiveProjectGroupSnapshot> => s.inactive_projects,
    no_agent_projects: Vec<InactiveProjectGroupSnapshot> => s.no_agent_projects,
    agents: Vec<SidebarAgentSnapshot> => s.agents,
    hide_ai_checkouts: Vec<HideAiCheckoutSnapshot> => s.hide_ai_checkouts,
    sleeping_sessions: Vec<crate::agent_sleep::SleepingSessionSnapshot> => s.sleeping_sessions,
    archived_sessions: Vec<crate::session_archive::ArchivedSessionSnapshot> => s.archived_sessions,
    factory_worker_panes: Vec<String> => s.factory_worker_panes,
}, {});

section!(TerminalSections, TerminalSnapshot, s, {
    pane_id: Option<String> => s.pane_id,
    closed: bool => s.closed,
    exit_code: Option<i32> => s.exit_code,
    panes: Vec<TerminalPaneSnapshot> => s.panes,
    input_requests: Vec<InputRequestSnapshot> => s.input_requests,
    headers: BTreeMap<String, crate::agent_state::header::Header> => s.headers,
}, {});

section!(StatusSections, StatusSnapshot, s, {
    server_discovery: ServerDiscoverySnapshot => s.server_discovery,
    tab_rename: Option<TabRenameSnapshot> => s.tab_rename,
    herdr: ProviderStatusSnapshot => s.herdr,
    remote: Vec<RemoteStatusSnapshot> => s.remote,
    environment: Vec<EnvironmentStatusSnapshot> => s.environment,
    agent_hooks: AgentHooksSnapshot => s.agent_hooks,
    background_ai: BackgroundAiSnapshot => s.background_ai,
    ssh_hosts: SshHostsSnapshot => s.ssh_hosts,
    herdr_profiles: HerdrProfilesSnapshot => s.herdr_profiles,
    diagnostics: Vec<DiagnosticSnapshot> => s.diagnostics,
    last_error: Option<LastErrorSnapshot> => s.last_error,
    async_operations: Vec<AsyncOperationSnapshot> => s.async_operations,
    pane_focus_request: Option<PaneFocusRequestSnapshot> => s.pane_focus_request,
    browser_opens: Vec<BrowserOpenReceiptSnapshot> => s.browser_opens,
}, {});

section!(RestSections, Snapshot, s, {
    delivery_watches: Vec<crate::delivery::watch::View> => s.delivery_watches,
    overview_graph: OverviewGraphSnapshot => s.overview_graph,
    sessions: SessionsSnapshot => s.sessions,
    card: CheckoutCardSnapshot => s.card,
    git_worktrees: Option<ProjectWorktreesSnapshot> => s.git_worktrees,
    git_worktrees_loading: bool => s.git_worktrees_loading,
    git_worktrees_remote: bool => s.git_worktrees_remote,
    worktree_removal: Option<WorktreeRemovalSnapshot> => s.worktree_removal,
    task_operation: Option<TaskOperationSnapshot> => s.task_operation,
    repository_clone: Option<RepositoryCloneSnapshot> => s.repository_clone,
    explorer_operation: Option<ExplorerOperationSnapshot> => s.explorer_operation,
    issue_work: IssueWorkSnapshot => s.issue_work,
    pr_work: PrWorkSnapshot => s.pr_work,
    overlay: OverlaySnapshot => s.overlay,
    tab: TabSnapshot => s.tab,
    connection: ConnectionSnapshot => s.connection,
    zoomed: Option<String> => s.zoomed,
    focused: FocusedSnapshot => s.focused,
    pane_layouts: Vec<PaneLayoutSnapshot> => s.pane_layouts,
    ui_state: UiStateSnapshot => s.ui_state,
    ime: ImeSnapshot => s.ime,
    pet: PetSnapshot => s.pet,
    recent_closed: RecentClosedSnapshot => s.recent_closed,
    workspace_view: Option<WorkspaceViewSnapshot> => s.workspace_view,
    browser_views: Vec<BrowserViewInventoryRow> => s.browser_views; generation s.browser_views_revision,
    browser_scopes: Vec<BrowserAreaScopeRow> => s.browser_scopes; generation s.browser_views_revision,
}, {
    navigator: NavigatorSections => s.navigator,
    terminal: TerminalSections => s.terminal,
    status: StatusSections => s.status,
});

pub struct RestWire<'a> {
    pub rest: &'a RestSections,
    pub have_revision: u64,
    pub legacy: bool,
}

impl Serialize for RestWire<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.rest.write(serializer, self.have_revision, self.legacy)
    }
}

impl RestSections {
    fn legacy_omit(&self, field: &str) -> bool {
        field == "workspace_view" && self.workspace_view.is_none()
    }
}
impl NavigatorSections {
    fn legacy_omit(&self, _: &str) -> bool {
        false
    }
}
impl TerminalSections {
    fn legacy_omit(&self, _: &str) -> bool {
        false
    }
}
impl StatusSections {
    fn legacy_omit(&self, _: &str) -> bool {
        false
    }
}
