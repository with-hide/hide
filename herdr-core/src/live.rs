//! Live Herdr commands and pane byte transport. Session state synchronization
//! lives in `session_sync` and uses the sequenced socket event stream.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::wire;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent_find::AgentFind;
use crate::checkout_owner::OwnerOpen;
use crate::find::PaneFindOptions;
use crate::fork::{ForkPlacement, ForkRequest};
use crate::handle::ChangeNotifier;
use crate::model::{
    EditorDocumentSnapshot, PaneFindRoute, PaneLayoutDirection, PaneLayoutNodeSnapshot,
    PaneLayoutSnapshot, WorkspaceRegistration, WorkspaceSnapshot,
};
use crate::node_access::call_as;
use crate::recent_closed::{
    ClosedAgent, ClosedContext, ClosedItem, ClosedLayoutBranch, ClosedLayoutNode, ClosedPane,
    PanePlacement, resume_arguments,
};
use crate::runtime::{LaneStart, PendingPaneFocusControl, Runtime};
use crate::sidebar::{
    SessionLayoutPanePayload, SessionLayoutPayload, SessionLayoutRect, SessionSnapshotPayload,
};
use crate::workspace;
#[cfg(test)]
use hide_herdr_client::HERDR_PROTOCOL_REVISION;
use hide_herdr_client::{
    ApiConnector, ApiError, request_small_response, request_with_connector,
    request_with_correlation_id,
};
use hide_node_link::project::ProjectCreated;
use hide_node_link::protocol::Call;

#[path = "worktree_cleanup.rs"]
pub(crate) mod cleanup;

#[path = "checkout_start.rs"]
mod checkout_start;
#[path = "worktree_control.rs"]
mod worktree_control;
pub(crate) use worktree_control::{
    CONFIRM_TIMEOUT, ProcessWait, close_checkout_panes, prompt_argument,
};
pub(crate) use worktree_control::{CheckoutStart, spawn_targeted_checkout_start};
pub use worktree_control::{
    CheckoutTabRequest, HomeStartRequest, IssueWriteFailure, PendingAgentStart, PurposeMirror,
    PurposeTaskOutcome, PurposeTaskRequest, TabTarget, TaskAgentOutcome, WorktreeTarget,
    WorktreeTaskOutcome, WorktreeTaskRequest, spawn_branch_migration, spawn_checkout_tab_create,
    spawn_existing_branch_worktree, spawn_home_link_sync, spawn_home_start, spawn_purpose_write,
    spawn_remote_purpose_write, spawn_task_agent_start, spawn_workspace_close,
    spawn_worktree_close, spawn_worktree_create, spawn_worktree_open, spawn_worktree_preflight,
};
pub(crate) use worktree_control::{GhPlace, spawn_issue_write};

/// What live commands and session sync need from the live configuration.
#[derive(Clone)]
pub struct LiveContext {
    pub socket_path: PathBuf,
    pub runtime: Weak<Mutex<Runtime>>,
    pub notifier: ChangeNotifier,
    pub(crate) api_connector: Arc<dyn ApiConnector>,
    /// The core's own node, which does this machine's work (PRD
    /// core-host-node D-21).
    pub(crate) node: Arc<dyn crate::node_access::NodeLink>,
}

pub struct WorkspaceCreationOutcome {
    pub registration: WorkspaceRegistration,
    pub base_registrations: Vec<WorkspaceRegistration>,
    pub registrations: Vec<WorkspaceRegistration>,
    pub workspaces: Vec<WorkspaceSnapshot>,
    /// What the node said of the paths `workspaces` was built from.
    pub paths: Arc<workspace::PathIndex>,
    pub session: SessionSnapshotPayload,
    pub created_pane_id: Option<String>,
    pub git_init_error: Option<String>,
}

pub fn spawn_workspace_creation(
    context: LiveContext,
    node: crate::node::NodeId,
    path: String,
    label: String,
    initialize_git: bool,
    new_folder: bool,
    base_registrations: Vec<WorkspaceRegistration>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-workspace-create".to_owned())
        .spawn(move || {
            let started = Instant::now();
            let request_path = path.clone();
            // The catalog this worker builds replaces the navigator's, so it
            // must carry the worktree rows the reader has already found.
            // Building it from an empty catalog would drop every worktree
            // without a pane until the next read.
            let worktrees = context
                .runtime
                .upgrade()
                .and_then(|runtime| {
                    let read = runtime.lock().ok().map(|guard| guard.worktree_catalog());
                    drop(runtime);
                    read
                })
                .unwrap_or_default();
            // The node makes the folder and the repository, and answers the
            // real path the project is registered by.
            let made: Result<ProjectCreated, String> = call_as(
                context.node.as_ref(),
                Call::ProjectCreate {
                    path: path.clone(),
                    new_folder,
                    initialize_git,
                },
                PROJECT_CREATE_TIMEOUT,
            )
            .map_err(|error| error.to_string());
            let result = made
                .and_then(|created| {
                    workspace::registration(&created.path, &label, node.as_str())
                        .map(|registration| (registration, created.git_error))
                })
                .and_then(|(registration, git_init_error)| {
                    let mut registrations = base_registrations.clone();
                    if !registrations
                        .iter()
                        .any(|existing| existing.id == registration.id)
                    {
                        registrations.push(registration.clone());
                    }
                    let ask = |session: &SessionSnapshotPayload| {
                        let mut wanted =
                            workspace::PathIndex::wanted(&node, &registrations, &[], &worktrees);
                        wanted.extend(Runtime::session_cwds(session));
                        workspace::PathIndex::ask(context.node.as_ref(), wanted)
                    };
                    let before = fetch_session_with_connector(context.api_connector.as_ref())
                        .map_err(|error| {
                            format!(
                                "session.snapshot before workspace creation failed: {}",
                                error.message()
                            )
                        })?;
                    let before_paths = ask(&before)?;
                    let before_spaces = Runtime::session_spaces(&before, &before_paths);
                    let before_catalog = workspace::build_catalog(
                        &node,
                        &registrations,
                        &before_spaces,
                        &worktrees,
                        &before_paths,
                    );
                    let needs_herdr_workspace = before_catalog
                        .iter()
                        .any(|workspace| workspace.id == registration.id);
                    // The project's first workspace is its checkout's owner
                    // (PRD checkout-workspace-binding B12), never a plain
                    // workspace a later tab would pass over.
                    let created = needs_herdr_workspace
                        .then(|| {
                            ensure_owner(
                                context.api_connector.as_ref(),
                                &registered_owner(
                                    &node,
                                    &before_paths,
                                    &registration.path,
                                    &registration.label,
                                ),
                                true,
                                Default::default(),
                            )
                            .map_err(|error| error.message().to_owned())
                        })
                        .transpose()?;
                    let session = if created.is_some() {
                        fetch_session_with_connector(context.api_connector.as_ref()).map_err(
                            |error| {
                                format!(
                                    "session.snapshot after workspace creation failed: {}",
                                    error.message()
                                )
                            },
                        )?
                    } else {
                        before
                    };
                    let paths = if created.is_some() {
                        ask(&session)?
                    } else {
                        before_paths
                    };
                    let spaces = Runtime::session_spaces(&session, &paths);
                    let workspaces = workspace::build_catalog(
                        &node,
                        &registrations,
                        &spaces,
                        &worktrees,
                        &paths,
                    );
                    Ok(WorkspaceCreationOutcome {
                        registration,
                        base_registrations,
                        registrations,
                        workspaces,
                        paths: Arc::new(paths),
                        session,
                        created_pane_id: created
                            .and_then(|(_, first_tab)| first_tab.map(|(_, pane_id, _)| pane_id)),
                        git_init_error,
                    })
                });
            let elapsed_ms = started.elapsed().as_millis();
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_workspace_creation(&request_path, result, elapsed_ms),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("workspace creation worker could not be started: {error}"))
}

/// The owner a newly registered project's checkout gets: a Git checkout is
/// opened from its repository's main worktree, a plain folder is created
/// and marked at the registered path, the same path the catalog keys it by.
fn registered_owner(
    node: &crate::node::NodeId,
    paths: &workspace::PathIndex,
    path: &str,
    label: &str,
) -> OwnerOpen {
    match paths.place(path) {
        Some(place) => {
            OwnerOpen::for_checkout(node.as_str(), &place.root, &place.main_root, true, label)
        }
        None => OwnerOpen::for_checkout(node.as_str(), path, path, false, label).on_node(),
    }
}

/// How long the node may take to make a project's folder and repository.
const PROJECT_CREATE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaneSplitDirection {
    Right,
    Down,
}

impl PaneSplitDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaneResizeDirection {
    Left,
    Right,
    Up,
    Down,
}

impl PaneResizeDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Debug)]
pub enum PaneControlAction {
    /// Fetches the authoritative layout containing a pane without changing
    /// Herdr's global focus. Checkout navigation uses this to update directly
    /// instead of waiting for the corresponding event-stream projection.
    Project {
        pane_id: String,
    },
    Focus {
        pane_id: String,
    },
    Split {
        pane_id: String,
        direction: PaneSplitDirection,
        cwd: Option<String>,
    },
    Resize {
        pane_id: String,
        direction: PaneResizeDirection,
        amount: f32,
    },
    ToggleZoom {
        pane_id: String,
    },
    Close {
        pane_id: String,
    },
    /// Takes a delegated child out of the tab it was split into and gives it
    /// a tab of its own, so the canvas stays one pane (PRD B1, D-15).
    MoveToNewTab {
        pane_id: String,
        workspace_id: String,
        label: String,
    },
}

impl PaneControlAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Project { .. } => "pane.project",
            Self::Focus { .. } => "pane.focus",
            Self::Split { .. } => "pane.split",
            Self::Resize { .. } => "pane.resize",
            Self::ToggleZoom { .. } => "pane.zoom",
            Self::Close { .. } => "pane.close",
            Self::MoveToNewTab { .. } => "pane.move",
        }
    }

    /// The pane the action is about, for the caller that has to match a
    /// result back to what it asked for.
    pub fn pane_id(&self) -> &str {
        match self {
            Self::Project { pane_id }
            | Self::Focus { pane_id }
            | Self::Split { pane_id, .. }
            | Self::Resize { pane_id, .. }
            | Self::ToggleZoom { pane_id }
            | Self::Close { pane_id }
            | Self::MoveToNewTab { pane_id, .. } => pane_id,
        }
    }
}

#[derive(Debug)]
pub enum PaneControlOutcome {
    Projected {
        layout: PaneLayoutSnapshot,
    },
    Acknowledged {
        created_pane_id: Option<String>,
    },
    /// Herdr accepted the request and moved nothing: a zoom answered
    /// `zoom_changed: false` or a resize answered `changed: false`. No
    /// layout event follows, so the operation ends at this answer (PRD
    /// instant-pane-topology D-12).
    Unchanged,
}

#[derive(Clone, Debug)]
pub enum RemoteControlAction {
    RenameTab {
        tab_id: String,
        label: String,
        request_id: String,
    },
    Pane(PaneControlAction),
    FocusWorkspace {
        workspace_id: String,
    },
    FocusTab {
        tab_id: String,
    },
    CreateTab {
        workspace_id: String,
        cwd: String,
        label: String,
        area_id: Option<String>,
        group_id: Option<Box<str>>,
        admission_id: Option<u64>,
    },
    /// A new tab in a checkout whose owner Herdr workspace is not open: the
    /// owner is opened first and the tab lands in it (`checkout_owner`).
    OpenOwner {
        owner: OwnerOpen,
        cwd: String,
        label: String,
        area_id: Option<String>,
        group_id: Option<Box<str>>,
        admission_id: Option<u64>,
    },
    CloseTab {
        tab_id: String,
    },
    /// Asks Herdr to put one of its tabs at a new place in its workspace.
    ///
    /// Herdr owns the order of its own tabs, so this is a request, not a
    /// local edit: the strip keeps the order Herdr last reported until Herdr
    /// reports the new one. `expected_order` is the tab order the operator
    /// asked for *within the single Herdr workspace that owns the moved tab*,
    /// not across the checkout: a checkout can hold tabs from several
    /// workspaces, and `tab.move` answers with one workspace's list. It is
    /// carried so the acknowledgement can be checked against what was
    /// requested rather than assumed.
    MoveTab {
        checkout_id: String,
        tab_id: String,
        /// Herdr's insertion index, which counts positions in the tab list
        /// *before* the moved tab is taken out of it.
        insert_index: usize,
        expected_order: Vec<String>,
        /// Which reorder this request belongs to, so a result that a later
        /// drag has already superseded cannot cancel the newer one.
        generation: u64,
        /// The Herdr connection that carried the request. A late response
        /// from an older socket cannot settle a move on a reconnected host.
        connection_generation: u64,
    },
}

impl RemoteControlAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Pane(action) => action.kind(),
            Self::FocusWorkspace { .. } => "workspace.focus",
            Self::FocusTab { .. } => "tab.focus",
            Self::CreateTab { .. } => "tab.create",
            Self::OpenOwner { owner, .. } => owner.kind(),
            Self::CloseTab { .. } => "tab.close",
            Self::MoveTab { .. } => "tab.move",
            Self::RenameTab { .. } => "tab.rename",
        }
    }
}

#[derive(Debug)]
pub enum RemoteControlOutcome {
    Acknowledged {
        created_tab_id: Option<String>,
        created_pane_id: Option<String>,
    },
    /// The workspace tab order Herdr reported back, in Herdr's order.
    TabsOrdered { tab_ids: Vec<String> },
}

/// A control result is either a known refusal or an answer that may have been
/// lost after Herdr accepted the request. The runtime must never retry the
/// latter mutation without first reading fresh topology.
#[derive(Clone, Debug)]
pub(crate) enum ControlFailure {
    Definite(String),
    Ambiguous(String),
}

impl ControlFailure {
    pub(crate) fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous(_))
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Definite(message) | Self::Ambiguous(message) => message,
        }
    }
}

impl From<String> for ControlFailure {
    fn from(message: String) -> Self {
        Self::Definite(message)
    }
}

#[derive(Clone)]
pub struct RemoteControlContext {
    target_id: String,
    api_connector: Arc<dyn ApiConnector>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl RemoteControlContext {
    pub(crate) fn new(
        target_id: impl Into<String>,
        api_connector: Arc<dyn ApiConnector>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) -> Self {
        Self {
            target_id: target_id.into(),
            api_connector,
            runtime,
            notifier,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    /// The Herdr API connection this device's control requests use.
    pub(crate) fn api_connector(&self) -> Arc<dyn ApiConnector> {
        Arc::clone(&self.api_connector)
    }
}

fn execute_remote_control(
    connector: &dyn ApiConnector,
    action: &RemoteControlAction,
) -> Result<RemoteControlOutcome, ControlFailure> {
    let (created_tab_id, created_pane_id) = match action {
        RemoteControlAction::Pane(action) => match action {
            PaneControlAction::Focus { .. }
            | PaneControlAction::Split { .. }
            | PaneControlAction::ToggleZoom { .. }
            | PaneControlAction::Close { .. } => match execute_pane_control(connector, action)? {
                PaneControlOutcome::Acknowledged { created_pane_id } => (None, created_pane_id),
                PaneControlOutcome::Unchanged => (None, None),
                PaneControlOutcome::Projected { .. } => {
                    return Err(ControlFailure::Definite(
                        "remote pane mutation returned a layout projection".to_owned(),
                    ));
                }
            },
            // A remote pane is never relocated: Hide leaves another
            // machine's layout alone (PRD D-28, D-49).
            PaneControlAction::Project { .. }
            | PaneControlAction::Resize { .. }
            | PaneControlAction::MoveToNewTab { .. } => {
                return Err(ControlFailure::Definite(
                    "unsupported remote pane control action".to_owned(),
                ));
            }
        },
        RemoteControlAction::FocusWorkspace { workspace_id } => {
            mutation_request(
                connector,
                "workspace.focus",
                wire::workspace_target_params(workspace_id)?,
            )?;
            (None, None)
        }
        RemoteControlAction::FocusTab { tab_id } => {
            mutation_request(connector, "tab.focus", wire::tab_target_params(tab_id)?)?;
            (None, None)
        }
        RemoteControlAction::CreateTab {
            workspace_id,
            cwd,
            label,
            admission_id,
            ..
        } => {
            let result = mutation_request(
                connector,
                "tab.create",
                wire::tab_create_with_env_params(
                    workspace_id,
                    cwd,
                    label,
                    true,
                    local_create_env(*admission_id),
                )?,
            )?;
            let (tab_id, pane_id) = wire::created_tab(result).map_err(ControlFailure::Ambiguous)?;
            (Some(tab_id), Some(pane_id))
        }
        RemoteControlAction::OpenOwner {
            owner,
            cwd,
            label,
            admission_id,
            ..
        } => {
            let tab = open_owner_tab(
                connector,
                owner,
                cwd,
                label,
                true,
                local_create_env(*admission_id),
                None,
            )?;
            (Some(tab.tab_id), Some(tab.pane_id))
        }
        RemoteControlAction::CloseTab { tab_id } => {
            mutation_request(connector, "tab.close", wire::tab_target_params(tab_id)?)?;
            (None, None)
        }
        RemoteControlAction::RenameTab { tab_id, label, .. } => {
            mutation_request(
                connector,
                "tab.rename",
                wire::tab_rename_params(tab_id, label)?,
            )?;
            (None, None)
        }
        RemoteControlAction::MoveTab {
            tab_id,
            insert_index,
            ..
        } => {
            // `tab.move` answers with the workspace's whole tab list in its
            // new order, so the caller can check what Herdr actually did
            // instead of assuming the request landed as asked.
            let result = mutation_request(
                connector,
                "tab.move",
                wire::tab_move_params(tab_id, *insert_index)?,
            )?;
            let tab_ids = wire::moved_tabs(result).map_err(ControlFailure::Ambiguous)?;
            return Ok(RemoteControlOutcome::TabsOrdered { tab_ids });
        }
    };
    Ok(RemoteControlOutcome::Acknowledged {
        created_tab_id,
        created_pane_id,
    })
}

/// A tab Hide created in a checkout's owner Herdr workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedTab {
    /// Label returned by creation, only while retaining that exact first tab.
    pub(crate) initial_label: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    /// Whether the pane was started the way it was asked for, its `env` and
    /// its command: a workspace's first tab that `worktree.open` made has
    /// neither, and one `workspace.create` made has no command.
    pub(crate) launched: bool,
}

/// Plain-folder owners are found and made one at a time, so two requests
/// racing for a folder with no owner converge on one workspace; a Git
/// checkout converges through `worktree.open` itself (`already_open`). The
/// lock spans every folder and device rather than one folder: an open here
/// happens once per folder per Herdr server life (D-11), and each call it
/// holds across is bounded by its own timeout, so the rare wait this costs
/// an unrelated folder is not worth a keyed lock registry.
static FOLDER_OWNER_OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Opens a checkout's owner and creates one tab in it (PRD
/// checkout-workspace-binding B2, B12, B13): a newly opened owner's first tab
/// is the new tab, and an owner that was already open gets a new tab. A
/// refusal fails the request; nothing is created in any other workspace (B3).
/// With a `command`, the tab's pane runs it (`layout.apply`); a newly opened
/// owner's first tab, which Herdr starts with its own shell, is replaced by
/// one that does (PRD agent-start-shell-ready D-10).
pub(crate) fn open_owner_tab(
    connector: &dyn ApiConnector,
    owner: &OwnerOpen,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> Result<OwnedTab, ControlFailure> {
    let (workspace_id, first_tab) = ensure_owner(connector, owner, focus, env.clone())?;
    let owned = match first_tab {
        Some((tab_id, pane_id, initial_label)) if command.is_some() => replace_tab(
            connector,
            OwnedTab {
                initial_label,
                workspace_id,
                tab_id,
                pane_id,
                launched: false,
            },
            cwd,
            label,
            focus,
            env,
            command,
        ),
        Some((tab_id, pane_id, initial_label)) => {
            // The owner's first tab is the new tab, so it takes the label the
            // request carries, as `tab.create` would give it; the tab is real
            // either way.
            if !label.trim().is_empty()
                && let Err(error) = mutation_request(
                    connector,
                    "tab.rename",
                    wire::tab_rename_params(&tab_id, label)?,
                )
            {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.tab_label_failed",
                    "workspace_id": workspace_id,
                    "tab_id": tab_id,
                    "message": error.message(),
                }));
            }
            OwnedTab {
                initial_label,
                workspace_id,
                tab_id,
                pane_id,
                launched: matches!(owner, OwnerOpen::Folder { .. }),
            }
        }
        None => create_tab_in(connector, &workspace_id, cwd, label, focus, env, command)?,
    };
    crate::diagnostic!(json!({
        "component": "checkout_owner",
        "kind": "owner.tab_created",
        "path": owner.path(),
        "method": owner.kind(),
        "workspace_id": owned.workspace_id,
        "tab_id": owned.tab_id,
    }));
    Ok(owned)
}

/// The checkout's owner workspace, opened when none is open, and the tab and
/// pane Herdr made with it when this call opened it (`None` when it was
/// already open). `env` reaches the first pane only where Herdr takes it
/// (`workspace.create`); `worktree.open` takes none.
type FirstTab = (String, String, Option<String>);

fn ensure_owner(
    connector: &dyn ApiConnector,
    owner: &OwnerOpen,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
) -> Result<(String, Option<FirstTab>), ControlFailure> {
    match owner {
        OwnerOpen::Worktree {
            path,
            repository_root,
            label,
        } => {
            let opened = wire::opened_worktree(mutation_request(
                connector,
                "worktree.open",
                wire::worktree_open_params(path, repository_root, focus)?,
            )?)
            .map_err(ControlFailure::Ambiguous)?;
            if opened.already_open {
                return Ok((opened.workspace_id, None));
            }
            // Only a workspace Herdr opened for this request takes the
            // checkout's name (D-14); the tab is real either way.
            if !label.trim().is_empty()
                && opened.label != *label
                && let Err(error) = mutation_request(
                    connector,
                    "workspace.rename",
                    wire::workspace_rename_params(&opened.workspace_id, label)?,
                )
            {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.label_failed",
                    "workspace_id": opened.workspace_id,
                    "path": path,
                    "message": error.message(),
                }));
            }
            Ok((
                opened.workspace_id,
                Some((opened.tab_id, opened.pane_id, Some(opened.tab_label))),
            ))
        }
        OwnerOpen::Folder {
            path, label, mark, ..
        } => {
            let _serialized = FOLDER_OWNER_OPEN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let listed = request_with_connector(
                connector,
                "workspace.list",
                wire::workspace_list_params()?,
                Duration::from_secs(5),
            )
            .map_err(|error| ControlFailure::Definite(format!("workspace.list failed: {error}")))
            .and_then(|value| {
                wire::listed_workspace_tokens(value, crate::checkout_owner::OWNER_TOKEN)
                    .map_err(ControlFailure::Definite)
            })?;
            if let Some((workspace_id, _)) = listed
                .into_iter()
                .find(|(_, value)| value.as_deref().is_some_and(|value| owner.marks(value)))
            {
                return Ok((workspace_id, None));
            }
            let created = mutation_request(
                connector,
                "workspace.create",
                wire::workspace_create_with_env_params(path, label, focus, env)?,
            )?;
            let (workspace_id, tab_id, pane_id) =
                wire::created_workspace(created).map_err(ControlFailure::Ambiguous)?;
            // The tab is real either way; without its mark only the next tab
            // in this folder makes another owner.
            if let Err(error) = mutation_request(
                connector,
                "workspace.report_metadata",
                wire::workspace_owner_mark_params(&workspace_id, mark)?,
            ) {
                crate::diagnostic!(json!({
                    "component": "checkout_owner",
                    "kind": "owner.mark_failed",
                    "workspace_id": workspace_id,
                    "path": path,
                    "message": error.message(),
                }));
            }
            Ok((workspace_id, Some((tab_id, pane_id, None))))
        }
    }
}

/// A new tab in `workspace_id` whose pane is started with `env`, running
/// `command` when there is one (`layout.apply`) and Herdr's own shell
/// otherwise (`tab.create`).
pub(crate) fn create_tab_in(
    connector: &dyn ApiConnector,
    workspace_id: &str,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> Result<OwnedTab, ControlFailure> {
    let (tab_id, pane_id) = match command {
        Some(command) => wire::launched_tab(mutation_request(
            connector,
            "layout.apply",
            wire::launched_tab_params(workspace_id, None, label, focus, cwd, env, command)?,
        )?),
        None => wire::created_tab(mutation_request(
            connector,
            "tab.create",
            wire::tab_create_with_env_params(workspace_id, cwd, label, focus, env)?,
        )?),
    }
    .map_err(ControlFailure::Ambiguous)?;
    Ok(OwnedTab {
        initial_label: None,
        workspace_id: workspace_id.to_owned(),
        tab_id,
        pane_id,
        launched: true,
    })
}

/// `tab`, a workspace's first tab Herdr started with its own shell, replaced
/// by one whose pane runs `command` with `env` (`layout.apply` with the
/// tab's id): `worktree.create`, `worktree.open` and `workspace.create` take
/// no command. Nothing is shown of it, since the workspace was made for this
/// start. When the replacement fails, `tab` itself is answered, not
/// launched, and the failure is logged.
#[allow(clippy::too_many_arguments)] // the tab to replace and what its pane runs
pub(crate) fn replace_tab(
    connector: &dyn ApiConnector,
    tab: OwnedTab,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
) -> OwnedTab {
    let Some(command) = command else {
        return tab;
    };
    let replaced = wire::launched_tab_params(
        &tab.workspace_id,
        Some(&tab.tab_id),
        label,
        focus,
        cwd,
        env,
        command,
    )
    .map_err(ControlFailure::Definite)
    .and_then(|params| mutation_request(connector, "layout.apply", params))
    .and_then(|answer| wire::launched_tab(answer).map_err(ControlFailure::Ambiguous));
    crate::diagnostic!(json!({
        "component": "agent_start",
        "kind": "launched_tab.replaced",
        "workspace_id": tab.workspace_id,
        "replaced": tab.tab_id,
        "tab_id": replaced.as_ref().ok().map(|(tab_id, _)| tab_id),
        "reason": replaced.as_ref().err().map(ControlFailure::message),
    }));
    match replaced {
        Ok((tab_id, pane_id)) => OwnedTab {
            initial_label: None,
            workspace_id: tab.workspace_id,
            tab_id,
            pane_id,
            launched: true,
        },
        Err(_) => tab,
    }
}

fn execute_pane_control(
    connector: &dyn ApiConnector,
    action: &PaneControlAction,
) -> Result<PaneControlOutcome, ControlFailure> {
    if let PaneControlAction::Project { pane_id } = action {
        return fetch_pane_layout(connector, pane_id)
            .map(|layout| PaneControlOutcome::Projected { layout })
            .map_err(ControlFailure::Definite);
    }
    if let PaneControlAction::Focus { pane_id } = action {
        mutation_request(connector, "pane.focus", wire::pane_target_params(pane_id)?)?;
        return Ok(PaneControlOutcome::Acknowledged {
            created_pane_id: None,
        });
    }
    if let PaneControlAction::Resize {
        pane_id,
        direction,
        amount,
    } = action
    {
        let result = mutation_request(
            connector,
            "pane.resize",
            wire::pane_resize_params(pane_id, *direction, *amount)?,
        )?;
        let changed = wire::pane_resize_changed(result).map_err(ControlFailure::Ambiguous)?;
        return Ok(if changed {
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            }
        } else {
            PaneControlOutcome::Unchanged
        });
    }
    if let PaneControlAction::ToggleZoom { pane_id } = action {
        let result = mutation_request(connector, "pane.zoom", wire::pane_zoom_params(pane_id)?)?;
        let changed = wire::pane_zoom_changed(result).map_err(ControlFailure::Ambiguous)?;
        return Ok(if changed {
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            }
        } else {
            PaneControlOutcome::Unchanged
        });
    }

    let created_pane_id = match action {
        PaneControlAction::Split {
            pane_id,
            direction,
            cwd,
        } => {
            // The user split to work in the new pane, so Herdr focuses it as
            // part of the split and the pane_focused event lands the shell's
            // selection there with no second round trip.
            let params = wire::pane_split_params(pane_id, *direction, cwd.as_deref())?;
            let result = mutation_request(connector, "pane.split", params)?;
            Some(wire::split_pane(result).map_err(ControlFailure::Ambiguous)?)
        }
        PaneControlAction::Close { pane_id } => {
            mutation_request(connector, "pane.close", wire::pane_target_params(pane_id)?)?;
            None
        }
        PaneControlAction::MoveToNewTab {
            pane_id,
            workspace_id,
            label,
        } => {
            let params = wire::pane_move_to_new_tab_params(pane_id, workspace_id, label)?;
            let result = mutation_request(connector, "pane.move", params)?;
            // The created tab is checked here so a refusal Herdr reported as
            // an unchanged move fails the action rather than reading as done.
            wire::moved_pane_tab(result).map_err(ControlFailure::Ambiguous)?;
            None
        }
        PaneControlAction::Project { .. }
        | PaneControlAction::Focus { .. }
        | PaneControlAction::Resize { .. }
        | PaneControlAction::ToggleZoom { .. } => unreachable!("handled above"),
    };
    Ok(PaneControlOutcome::Acknowledged { created_pane_id })
}

fn control_request(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    request_with_connector(connector, method, params, Duration::from_secs(5))
        .map_err(|error| format!("{method} failed: {error}"))
}

fn mutation_request(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
) -> Result<Value, ControlFailure> {
    request_with_connector(connector, method, params, Duration::from_secs(5)).map_err(|error| {
        match error {
            ApiError::Remote { code, message } => {
                ControlFailure::Definite(format!("{method} was refused: {code}: {message}"))
            }
            ApiError::NotRunning(message)
            | ApiError::Transport(message)
            | ApiError::Malformed(message) => {
                ControlFailure::Ambiguous(format!("{method} result is unknown: {message}"))
            }
        }
    })
}

fn reopen_request(
    connector: &dyn ApiConnector,
    request_id: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    request_with_correlation_id(
        connector,
        request_id,
        method,
        params,
        Duration::from_secs(5),
    )
    .map_err(|error| format!("{method} failed: {error}"))
}

#[derive(Clone, Debug)]
pub enum CloseCaptureTarget {
    Pane { pane_id: String },
    Tab { tab_id: String },
}

#[derive(Clone, Debug)]
pub struct CloseCaptureRequest {
    pub failed_terminals: Vec<(String, String)>,
    /// A sleep close must never acquire the independent ordinary undo route.
    pub retain_for_reopen: bool,
    pub key: String,
    /// A response from an older live connection must never settle a newer
    /// close intent after reconnect.
    pub connection_generation: u64,
    pub context: ClosedContext,
    pub panes: Vec<ClosedPane>,
    pub target: CloseCaptureTarget,
}

#[derive(Debug)]
pub struct CloseCaptureOutcome {
    pub item: Option<ClosedItem>,
}

/// The work included in the operator's existing close confirmation.
#[derive(Clone, Debug)]
pub struct ClosePeer {
    pub pane_id: String,
    pub terminal_id: String,
    pub session: Option<crate::sidebar::SessionAgentSessionPayload>,
}

/// Fresh terminal-close authorization and the admission's shared deadline.
#[derive(Clone, Debug)]
pub struct CloseSafety {
    /// Only the working terminals included in the admitted confirmation.
    pub approved_peers: Vec<ClosePeer>,
    pub pane_ids: Vec<String>,
    pub deadline: Instant,
}

#[derive(Clone, Debug)]
pub struct CloseEffectRequest {
    pub safety: Option<CloseSafety>,
    pub failed_terminals: Vec<(String, String)>,
    pub retain_for_reopen: bool,
    pub allow_replacement_create: bool,
    pub replacement: Option<ClosedContext>,
    pub key: String,
    pub connection_generation: u64,
    pub target: CloseCaptureTarget,
}

pub fn spawn_close_capture(
    context: LiveContext,
    request: CloseCaptureRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-close-capture".to_owned())
        .spawn(move || {
            let result = capture_close_item(context.api_connector.as_ref(), &request);
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let (changed, effects) = match runtime.lock() {
                Ok(mut guard) => guard.ingest_close_capture_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
            run_close_effects(&context, effects);
        })
        .map(|_| ())
        .map_err(|error| format!("close capture worker could not be started: {error}"))
}

/// Continues the same close operation after a durable sleeping-session
/// receipt. It neither captures another item nor sends an extra close.
pub(crate) fn spawn_saved_close_effect(
    mut context: LiveContext,
    effect: CloseEffectRequest,
) -> Result<(), String> {
    context.api_connector = Arc::from(crate::agent_sleep_herdr::fenced_dormant_close_connector(
        &context, &effect,
    ));
    thread::Builder::new()
        .name("herdr-core-saved-close".into())
        .spawn(move || run_close_effects(&context, vec![effect]))
        .map(|_| ())
        .map_err(|_| "The saved close worker could not start".into())
}

fn run_close_effects(context: &LiveContext, effects: Vec<CloseEffectRequest>) {
    for effect in effects {
        let prepared = if let Some(replacement) = &effect.replacement {
            prepare_close_replacement(
                context.api_connector.as_ref(),
                &effect.key,
                replacement,
                &effect.target,
                effect.allow_replacement_create,
            )
            .map_err(|failure| {
                if let Some(shell) = &failure.shell_in_tab
                    && let Some(runtime) = context.runtime.upgrade()
                    && let Ok(mut guard) = runtime.lock()
                {
                    guard.note_shell_in_tab(&effect.key, shell);
                }
                failure.message
            })
            .and_then(|(shell, payload)| {
                let runtime = context.runtime.upgrade().ok_or("runtime stopped")?;
                let mut guard = runtime.lock().map_err(|_| "runtime unavailable")?;
                guard.ingest_close_replacement(&effect, &shell, payload)?;
                context.notifier.notify();
                Ok(())
            })
            .map_err(|message| ApiError::Remote {
                code: "replacement_failed".into(),
                message,
            })
        } else {
            Ok(())
        };
        let result =
            prepared.and_then(|()| run_close_effect(context.api_connector.as_ref(), &effect));
        let Some(runtime) = context.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => guard.ingest_close_effect_result(&effect, result),
            Err(_) => return,
        };
        drop(runtime);
        if changed {
            context.notifier.notify();
        }
    }
}

fn capture_close_item(
    connector: &dyn ApiConnector,
    request: &CloseCaptureRequest,
) -> Result<CloseCaptureOutcome, String> {
    // A terminal Herdr could not recreate has no live layout to export.
    // Keep the close intent, but do not promise an undo that cannot restore it.
    let targets = match &request.target {
        CloseCaptureTarget::Pane { pane_id } => vec![pane_id.as_str()],
        CloseCaptureTarget::Tab { .. } => request
            .panes
            .iter()
            .map(|pane| pane.pane_id.as_str())
            .collect(),
    };
    if targets
        .iter()
        .any(|id| request.failed_terminals.iter().any(|(pane, _)| pane == id))
    {
        return Ok(CloseCaptureOutcome { item: None });
    }
    let layout_value = reopen_request(
        connector,
        &format!("herdr-core:{}:export", request.key),
        "layout.export",
        wire::layout_export_params(&request.context.tab_id)?,
    )?;
    let layout = wire::exported_layout(layout_value)?;
    let item = match &request.target {
        CloseCaptureTarget::Pane { pane_id } => {
            let pane = request
                .panes
                .iter()
                .find(|pane| pane.pane_id == *pane_id)
                .cloned()
                .ok_or_else(|| format!("pane {pane_id} disappeared before close capture"))?;
            let placement = layout.root.placement_for(pane_id).unwrap_or(PanePlacement {
                neighbor_pane_id: None,
                parent_path: Vec::new(),
                direction: crate::recent_closed::ClosedSplitDirection::Right,
                ratio: 0.5,
                target_was_first: false,
            });
            Some(ClosedItem::Pane {
                key: request.key.clone(),
                context: request.context.clone(),
                pane,
                placement,
            })
        }
        CloseCaptureTarget::Tab { .. } => Some(ClosedItem::Tab {
            key: request.key.clone(),
            context: request.context.clone(),
            layout,
            panes: request.panes.clone(),
        }),
    };
    Ok(CloseCaptureOutcome { item })
}

/// Why a close's shell could not be handed over. A shell already standing in
/// the closing tab is named: it is this close's own, so the runtime keeps it
/// among the tab's panes and a retry closes beside it.
#[derive(Debug)]
pub(crate) struct ReplacementFailure {
    pub(crate) message: String,
    pub(crate) shell_in_tab: Option<String>,
}

impl From<String> for ReplacementFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            shell_in_tab: None,
        }
    }
}

impl From<&str> for ReplacementFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

/// Makes, or on a retry finds by its marker, the shell a close needs first,
/// and answers it with a fresh session that holds it: a new tab in the
/// primary workspace, or a pane in the closing tab beside the pane `target`
/// closes when `replacement_in_tab`.
fn prepare_close_replacement(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    target: &CloseCaptureTarget,
    allow_create: bool,
) -> Result<(String, SessionSnapshotPayload), ReplacementFailure> {
    if context.replacement_in_tab {
        let CloseCaptureTarget::Pane { pane_id } = target else {
            return Err("a shell inside the tab needs the pane it stands beside".into());
        };
        return prepare_shell_in_tab(connector, key, context, pane_id, allow_create);
    }
    let started = Instant::now();
    let snapshot =
        fetch_session_with_connector(connector).map_err(|error| error.message().to_owned())?;
    if !snapshot
        .workspaces
        .iter()
        .any(|workspace| workspace.workspace_id == context.workspace_id)
    {
        return Err("primary workspace disappeared before replacement".into());
    }
    let candidates = snapshot
        .tabs
        .iter()
        .filter(|tab| {
            tab.workspace_id == context.workspace_id
                && !context.tab_ids_before_close.contains(&tab.tab_id)
        })
        .collect::<Vec<_>>();
    if candidates.len() > 64 {
        return Err("too many tabs to inspect for replacement intent".into());
    }
    let marker = reopen_intent_marker(key, ReopenIntentStage::CloseReplacement);
    let mut owned = None;
    for tab in candidates {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("replacement inspection timed out".into());
        }
        let layout = export_reopen_layout(connector, key, &tab.tab_id)?;
        if layout.tab_id != tab.tab_id || layout.workspace_id != tab.workspace_id {
            return Err("The tab changed during start confirmation. Retry confirmation.".into());
        }
        if layout_has_reopen_marker(&layout, &marker) {
            if owned.is_some() {
                return Err("multiple shells carry the same close intent".into());
            }
            owned = Some(tab.tab_id.clone());
        }
    }
    let tab_id = match owned {
        Some(id) => id,
        None if !allow_create => {
            return Err(
                "The replacement shell disappeared; retry after making room for a tab".into(),
            );
        }
        None => {
            let value = reopen_request(
                connector,
                &format!("herdr-core:{key}:replacement"),
                "tab.create",
                wire::replacement_tab_params(
                    &context.workspace_id,
                    &context.checkout_path,
                    reopen_intent_env(key, ReopenIntentStage::CloseReplacement),
                )?,
            )?;
            wire::created_tab(value)?.0
        }
    };
    let mut snapshot =
        fetch_session_with_connector(connector).map_err(|error| error.message().to_owned())?;
    if !snapshot
        .tabs
        .iter()
        .any(|tab| tab.tab_id == tab_id && tab.workspace_id == context.workspace_id)
    {
        return Err("replacement shell is not confirmed in the primary workspace".into());
    }
    place_replacement_at_its_checkout(&mut snapshot, &tab_id, &context.checkout_path);
    Ok((tab_id, snapshot))
}

/// The shell a named tab keeps when its last agent pane closes: split from
/// that pane at the checkout, unfocused, and stamped with the close's marker,
/// which is the only proof a retry adopts it by.
fn prepare_shell_in_tab(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    closing_pane: &str,
    allow_create: bool,
) -> Result<(String, SessionSnapshotPayload), ReplacementFailure> {
    let pane_id = match marked_shells_in_tab(connector, key, &context.tab_id)?.as_slice() {
        [] if !allow_create => {
            return Err("The replacement shell disappeared; retry to make another".into());
        }
        [] => split_shell_in_tab(connector, key, context, closing_pane)?,
        [id] => id.clone(),
        _ => return Err("multiple shells carry the same close intent".into()),
    };
    let held = |message: String| ReplacementFailure {
        message,
        shell_in_tab: Some(pane_id.clone()),
    };
    let mut snapshot = fetch_session_with_connector(connector)
        .map_err(|error| held(error.message().to_owned()))?;
    if !snapshot.layouts.iter().any(|layout| {
        layout.tab_id == context.tab_id && layout.panes.iter().any(|pane| pane.pane_id == pane_id)
    }) {
        return Err(held("replacement shell is not confirmed in its tab".into()));
    }
    // As for a new tab's shell (below), its folder is the one it was made at.
    if let Some(pane) = snapshot
        .panes
        .iter_mut()
        .find(|pane| pane.pane_id == pane_id)
    {
        pane.cwd = Some(context.checkout_path.clone());
    }
    Ok((pane_id, snapshot))
}

fn marked_shells_in_tab(
    connector: &dyn ApiConnector,
    key: &str,
    tab_id: &str,
) -> Result<Vec<String>, String> {
    let layout = export_reopen_layout(connector, key, tab_id)?;
    let marker = reopen_intent_marker(key, ReopenIntentStage::CloseReplacement);
    let mut owned = Vec::new();
    pane_ids_with_reopen_marker(&layout.root, &marker, &mut owned);
    Ok(owned)
}

/// A lost or malformed answer can follow a split Herdr made, so a failed
/// split reads the tab once more for the marked pane a retry would adopt.
fn split_shell_in_tab(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    closing_pane: &str,
) -> Result<String, ReplacementFailure> {
    let params = wire::replacement_pane_params(
        closing_pane,
        &context.checkout_path,
        reopen_intent_env(key, ReopenIntentStage::CloseReplacement),
    )?;
    reopen_request(
        connector,
        &format!("herdr-core:{key}:replacement"),
        "pane.split",
        params,
    )
    .and_then(wire::split_pane)
    .map_err(|message| ReplacementFailure {
        message,
        shell_in_tab: marked_shells_in_tab(connector, key, &context.tab_id)
            .ok()
            .and_then(|owned| match owned.as_slice() {
                [id] => Some(id.clone()),
                _ => None,
            }),
    })
}

/// Herdr reads a new pane's cwd off its process, which still has the
/// server's own until the shell has started in the folder it was asked for,
/// so a snapshot read right after `tab.create` can name the wrong folder
/// (`SessionReplica::await_cwd` waits it out for the event stream). This
/// snapshot is ingested at once to place the shell beside the tab it
/// replaces, and a pane at the server's folder lands outside the checkout,
/// where nothing can place it; the folder the shell was created at is the
/// one it is placed by.
fn place_replacement_at_its_checkout(
    snapshot: &mut SessionSnapshotPayload,
    tab_id: &str,
    checkout_path: &str,
) {
    let replacement_panes = snapshot
        .layouts
        .iter()
        .filter(|layout| layout.tab_id == tab_id)
        .flat_map(|layout| layout.panes.iter().map(|pane| pane.pane_id.as_str()))
        .collect::<Vec<_>>();
    for pane in &mut snapshot.panes {
        if replacement_panes.contains(&pane.pane_id.as_str()) {
            pane.cwd = Some(checkout_path.to_owned());
        }
    }
}

fn run_close_effect(
    connector: &dyn ApiConnector,
    request: &CloseEffectRequest,
) -> Result<(), ApiError> {
    run_close_effect_with_clock(connector, request, &Instant::now)
}

fn run_close_effect_with_clock(
    connector: &dyn ApiConnector,
    request: &CloseEffectRequest,
    now: &dyn Fn() -> Instant,
) -> Result<(), ApiError> {
    check_failed_terminals(
        connector,
        &request.failed_terminals,
        &request.target,
        request.safety.as_ref(),
        now,
    )?;
    let (method, params) = match &request.target {
        CloseCaptureTarget::Pane { pane_id } => (
            "pane.close",
            wire::pane_target_params(pane_id).map_err(ApiError::Malformed)?,
        ),
        CloseCaptureTarget::Tab { tab_id } => (
            "tab.close",
            wire::tab_target_params(tab_id).map_err(ApiError::Malformed)?,
        ),
    };
    request_with_correlation_id(
        connector,
        &format!("herdr-core:{}:close", request.key),
        method,
        params,
        close_remaining(request.safety.as_ref(), now())?,
    )
    .map(|_| ())
}

/// One fresh read per close, on its existing worker, with no runtime lock held.
/// Refuse the old authorization if any failed terminal was replaced or recovered.
fn check_failed_terminals(
    connector: &dyn ApiConnector,
    expected: &[(String, String)],
    target: &CloseCaptureTarget,
    safety: Option<&CloseSafety>,
    now: &dyn Fn() -> Instant,
) -> Result<(), ApiError> {
    if expected.is_empty() {
        return Ok(());
    }
    let value = request_with_connector(
        connector,
        "session.snapshot",
        wire::empty_params(),
        close_remaining(safety, now())?,
    )?;
    let snapshot = wire::snapshot_response(value)
        .map_err(|error| ApiError::Malformed(error.message().to_owned()))?;
    let scope_valid = match target {
        CloseCaptureTarget::Pane { pane_id } => expected.len() == 1 && expected[0].0 == *pane_id,
        CloseCaptureTarget::Tab { tab_id } => safety.is_some_and(|safety| {
            let mut current = snapshot
                .panes
                .iter()
                .filter(|pane| &pane.tab_id == tab_id)
                .map(|pane| pane.pane_id.clone())
                .collect::<Vec<_>>();
            current.sort();
            let mut approved = safety.pane_ids.clone();
            approved.sort();
            current == approved
                && expected.iter().all(|(id, _)| current.contains(id))
                && snapshot
                    .panes
                    .iter()
                    .filter(|pane| &pane.tab_id == tab_id)
                    .filter(|pane| !expected.iter().any(|(id, _)| id == &pane.pane_id))
                    .all(|pane| {
                        let approved = safety.approved_peers.iter().any(|peer| {
                            peer.pane_id == pane.pane_id
                                && peer.terminal_id == pane.terminal_id
                                && peer.session.as_ref()
                                    == snapshot
                                        .agents
                                        .iter()
                                        .find(|agent| agent.pane_id == pane.pane_id)
                                        .and_then(|agent| agent.agent_session.as_ref())
                        });
                        let allowed = |status: &str| {
                            matches!(status, "idle" | "done")
                                || (approved && matches!(status, "working" | "blocked"))
                        };
                        allowed(&pane.agent_status)
                            && snapshot
                                .agents
                                .iter()
                                .filter(|agent| agent.pane_id == pane.pane_id)
                                .all(|agent| agent.agent_status.as_deref().is_some_and(allowed))
                    })
        }),
    };
    if scope_valid
        && expected.iter().all(|(id, terminal)| {
            snapshot.panes.iter().any(|pane| {
                &pane.pane_id == id && &pane.terminal_id == terminal && pane.restore_error.is_some()
            })
        })
    {
        Ok(())
    } else {
        Err(ApiError::Remote {
            code: "close_target_changed".into(),
            message:
                "The terminal changed before closing. Check its current state and close it again."
                    .into(),
        })
    }
}

fn close_remaining(safety: Option<&CloseSafety>, now: Instant) -> Result<Duration, ApiError> {
    match safety {
        None => Ok(Duration::from_secs(5)),
        Some(safety) => safety.deadline.checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| ApiError::Remote {
                code: "close_deadline_expired".into(),
                message: "The close expired before it was sent. Check its current state and close it again.".into(),
            }),
    }
}

/// A read-only fresh session check for a close whose transport result was
/// ambiguous. It does not repeat the mutation and carries the connection
/// generation so a late answer from an older socket cannot change current
/// state.
#[derive(Clone, Debug)]
pub struct CloseStatusCheckRequest {
    pub key: String,
    pub target: CloseCaptureTarget,
}

pub fn spawn_close_status_check(
    context: LiveContext,
    key: String,
    connection_generation: u64,
    target: CloseCaptureTarget,
) -> Result<(), String> {
    spawn_close_status_checks(
        context,
        connection_generation,
        vec![CloseStatusCheckRequest { key, target }],
    )
}

/// Runs one read for all close operations waiting on the same authoritative
/// session. The result is fanned out while the runtime lock is held, so a
/// single fresh snapshot cannot settle one close and leave its siblings stale.
pub fn spawn_close_status_checks(
    context: LiveContext,
    connection_generation: u64,
    requests: Vec<CloseStatusCheckRequest>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-close-status-check".to_owned())
        .spawn(move || {
            let result = fetch_session_with_connector(context.api_connector.as_ref());
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => {
                    guard.ingest_close_status_results(connection_generation, &requests, result)
                }
                Err(_) => false,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("close status worker could not be started: {error}"))
}

#[derive(Clone, Debug)]
pub struct ReopenRequest {
    pub item: ClosedItem,
    pub workspace_exists: bool,
    pub tab_exists: bool,
    pub fallback_pane_id: Option<String>,
    /// The owner of the checkout the closed item belongs to: a tab that has to
    /// be made again lands there, never in the workspace it was closed from
    /// (PRD checkout-workspace-binding D-16). `None` for a file.
    pub owner: Option<OwnerOpen>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
}

const REOPEN_INTENT_ENV: &str = "HIDE_REOPEN_INTENT";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReopenIntentStage {
    LocalCreate,
    Workspace,
    Layout,
    Pane,
    CloseReplacement,
}

fn reopen_intent_marker(key: &str, stage: ReopenIntentStage) -> String {
    let stage = match stage {
        ReopenIntentStage::LocalCreate => "local-create",
        ReopenIntentStage::Workspace => "workspace",
        ReopenIntentStage::Layout => "layout",
        ReopenIntentStage::Pane => "pane",
        ReopenIntentStage::CloseReplacement => "close-replacement",
    };
    format!("{key}:{stage}")
}

fn reopen_intent_env(
    key: &str,
    stage: ReopenIntentStage,
) -> std::collections::BTreeMap<String, String> {
    [(
        REOPEN_INTENT_ENV.to_owned(),
        reopen_intent_marker(key, stage),
    )]
    .into_iter()
    .collect()
}

fn tag_reopen_layout(
    node: &ClosedLayoutNode,
    key: &str,
    stage: ReopenIntentStage,
) -> ClosedLayoutNode {
    match node {
        ClosedLayoutNode::Pane {
            pane_id,
            label,
            cwd,
            command,
            env,
        } => {
            let mut env = env.clone();
            env.insert(
                REOPEN_INTENT_ENV.to_owned(),
                reopen_intent_marker(key, stage),
            );
            ClosedLayoutNode::Pane {
                pane_id: pane_id.clone(),
                label: crate::start_claim::public_label(label.as_deref()),
                cwd: cwd.clone(),
                command: command.clone(),
                env,
            }
        }
        ClosedLayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => ClosedLayoutNode::Split {
            direction: *direction,
            ratio: *ratio,
            first: Box::new(tag_reopen_layout(first, key, stage)),
            second: Box::new(tag_reopen_layout(second, key, stage)),
        },
    }
}

fn pane_ids_with_reopen_marker(node: &ClosedLayoutNode, marker: &str, output: &mut Vec<String>) {
    match node {
        ClosedLayoutNode::Pane { pane_id, env, .. } => {
            if env
                .get(REOPEN_INTENT_ENV)
                .is_some_and(|value| value == marker)
            {
                output.extend(pane_id.iter().cloned());
            }
        }
        ClosedLayoutNode::Split { first, second, .. } => {
            pane_ids_with_reopen_marker(first, marker, output);
            pane_ids_with_reopen_marker(second, marker, output);
        }
    }
}

fn layout_has_reopen_marker(layout: &crate::recent_closed::ClosedLayout, marker: &str) -> bool {
    let mut pane_ids = Vec::new();
    pane_ids_with_reopen_marker(&layout.root, marker, &mut pane_ids);
    !pane_ids.is_empty()
}

fn export_reopen_layout(
    connector: &dyn ApiConnector,
    key: &str,
    tab_id: &str,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    let exported = reopen_request(
        connector,
        &format!("herdr-core:{key}:inspect:{tab_id}"),
        "layout.export",
        wire::layout_export_params(tab_id)?,
    )?;
    wire::exported_layout(exported)
}

fn restore_tab_position(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    layout: crate::recent_closed::ClosedLayout,
    notices: &mut Vec<String>,
) -> crate::recent_closed::ClosedLayout {
    // The position is an index among the tabs of the workspace it was closed
    // from; a tab that reopened in the checkout's owner elsewhere keeps the
    // place Herdr gave it. That is the usual case for a tab closed outside
    // the owner, and for a plain folder whose owner closed with its last tab.
    if layout.workspace_id != context.workspace_id {
        return layout;
    }
    let params = match wire::tab_move_params(&layout.tab_id, context.tab_index) {
        Ok(params) => params,
        Err(message) => {
            notices.push(format!(
                "The tab reopened but its original position could not be restored: {message}"
            ));
            return layout;
        }
    };
    let move_result = reopen_request(
        connector,
        &format!("herdr-core:{key}:tab-position"),
        "tab.move",
        params,
    )
    .and_then(wire::moved_tabs);
    if let Err(message) = move_result {
        notices.push(format!(
            "The tab reopened but its original position could not be restored: {message}"
        ));
    }
    layout
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenNotice {
    pub pane_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenOutcome {
    pub tab_id: Option<String>,
    pub consumed: bool,
    pub focused_pane_id: Option<String>,
    pub notices: Vec<ReopenNotice>,
    /// The agents the reopen started, so the core keeps each one's launch
    /// choice with its new pane.
    pub agents: Vec<ReopenedAgent>,
}

/// An agent a reopen started in a restored pane: the pane, its kind, the
/// choice it was started with, and the conversation it resumed (none for a
/// new one, when the resume did not take).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReopenedAgent {
    pub pane_id: String,
    pub kind: String,
    /// The name the reopen started it under.
    pub name: String,
    pub launch: hide_agent_adapter::LaunchChoice,
    pub session_id: Option<String>,
}

#[derive(Debug)]
pub enum FileReopenResult {
    Opened(Box<(EditorDocumentSnapshot, crate::files::DocumentPlace)>),
    Missing,
    Failed(String),
}

pub fn spawn_reopen(context: LiveContext, request: ReopenRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-reopen-closed".to_owned())
        .spawn(move || {
            let node = context.node.as_ref();
            let folders = |path: &str| crate::node_access::is_directory(node, path);
            let result = run_herdr_reopen(context.api_connector.as_ref(), &folders, &request)
                .map(FileReopenResultOrHerdr::Herdr);
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_reopen_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("reopen worker could not be started: {error}"))
}

pub fn spawn_file_reopen(
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    request: ReopenRequest,
    root: crate::files::DocumentRoot,
    channel: std::sync::Arc<dyn crate::node_access::NodeLink>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-file-reopen".to_owned())
        .spawn(move || {
            let ClosedItem::File { path, .. } = &request.item else {
                return;
            };
            let result = Ok(run_file_reopen(channel.as_ref(), &root, path));
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_reopen_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("file reopen worker could not be started: {error}"))
}

#[derive(Debug)]
pub enum FileReopenResultOrHerdr {
    File(FileReopenResult),
    Herdr(ReopenOutcome),
}

fn run_file_reopen(
    channel: &dyn crate::node_access::NodeLink,
    root: &crate::files::DocumentRoot,
    path: &str,
) -> FileReopenResultOrHerdr {
    FileReopenResultOrHerdr::File(match crate::files::open_document(channel, root, path) {
        Ok(opened) => FileReopenResult::Opened(Box::new(opened)),
        Err(crate::files::OpenFailure::Missing) => FileReopenResult::Missing,
        Err(failure) => FileReopenResult::Failed(failure.message()),
    })
}

/// `folders` says whether a closed pane's folder is still a directory on the
/// node that held it.
fn run_herdr_reopen(
    connector: &dyn ApiConnector,
    folders: &dyn Fn(&str) -> bool,
    request: &ReopenRequest,
) -> Result<ReopenOutcome, String> {
    match &request.item {
        ClosedItem::Pane {
            key,
            context,
            pane,
            placement,
        } => reopen_pane(connector, folders, key, context, pane, placement, request),
        ClosedItem::Tab {
            key,
            context,
            layout,
            panes,
        } => reopen_tab(
            connector,
            folders,
            key,
            context,
            reopen_owner(request)?,
            &layout.root,
            panes,
            request.codex_daemon,
        ),
        ClosedItem::File { .. } => unreachable!("file reopen uses the filesystem worker"),
        ClosedItem::Draft { .. } => unreachable!("draft reopen is local"),
    }
}

/// Creates or adopts exactly one intent-marked wake tab in the current
/// checkout owner. No folder substitution or fresh-agent fallback is allowed.
pub(crate) fn create_sleep_tab(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    cwd: &str,
) -> Result<(String, String, SessionSnapshotPayload), String> {
    if !crate::node_access::is_directory(node, cwd) {
        return Err("The sleeping session's working folder is unavailable".into());
    }
    let root = ClosedLayoutNode::Pane {
        pane_id: None,
        label: None,
        cwd: Some(cwd.into()),
        command: None,
        env: Default::default(),
    };
    let mut notices = Vec::new();
    let layout =
        ensure_workspace_and_tab(connector, key, context, owner, false, &root, &mut notices)?;
    // Position notices do not change the owned tab/pane identity or cwd.
    for _ in notices {
        crate::diagnostic!(
            serde_json::json!({"component":"agent_sleep", "kind":"agent_sleep.wake_tab_position_unconfirmed", "sleep_id":key})
        );
    }
    let snapshot = fetch_session_with_connector(connector)
        .map_err(|_| "The wake tab exists but its current topology could not be read".to_owned())?;
    Ok((layout.tab_id, layout.focused_pane_id, snapshot))
}

struct ReopenIntentMatch {
    open_owner: Option<String>,
    layout: Option<crate::recent_closed::ClosedLayout>,
    workspace_seed: Option<(String, String)>,
}

/// Searches only current-owner tabs carrying the exact intent marker.
/// This part of reopen is read-only and is also used to inspect uncertain wakes.
fn find_reopen_intent(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    snapshot: &SessionSnapshotPayload,
) -> Result<ReopenIntentMatch, String> {
    let started = Instant::now();
    // The tab goes to the checkout's owner. A retry adopts only a layout or a
    // workspace seed carrying this intent's marker: in the owner when it is
    // open, else in a workspace that appeared since the close.
    let open_owner = owner
        .find_in(
            snapshot
                .workspaces
                .iter()
                .map(crate::checkout_owner::WorkspaceFacts::from),
        )
        .map(str::to_owned);
    let workspace_marker = reopen_intent_marker(key, ReopenIntentStage::Workspace);
    let layout_marker = reopen_intent_marker(key, ReopenIntentStage::Layout);
    let candidate_workspace_ids = match &open_owner {
        Some(owner) => vec![owner.clone()],
        None => snapshot
            .workspaces
            .iter()
            .filter(|workspace| {
                !context
                    .workspace_ids_before_close
                    .contains(&workspace.workspace_id)
            })
            .map(|workspace| workspace.workspace_id.clone())
            .collect(),
    };
    let mut recovered_layouts = Vec::new();
    let mut recovered_workspace_seeds = Vec::new();
    let candidates = snapshot
        .tabs
        .iter()
        .filter(|tab| {
            candidate_workspace_ids.contains(&tab.workspace_id)
                && !context.tab_ids_before_close.contains(&tab.tab_id)
        })
        .collect::<Vec<_>>();
    if candidates.len() > 64 {
        return Err("Too many tabs to inspect for the saved intent".into());
    }
    for tab in candidates {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("Saved-intent inspection timed out".into());
        }
        let layout = export_reopen_layout(connector, key, &tab.tab_id)?;
        if layout_has_reopen_marker(&layout, &layout_marker) {
            recovered_layouts.push(layout);
        } else if layout_has_reopen_marker(&layout, &workspace_marker) {
            recovered_workspace_seeds.push((tab.workspace_id.clone(), tab.tab_id.clone()));
        }
    }
    if recovered_layouts.len() > 1 {
        return Err(format!(
            "reopen retry found {} layouts owned by {key}; refusing to choose one",
            recovered_layouts.len()
        ));
    }
    if recovered_workspace_seeds.len() > 1 {
        return Err(format!(
            "reopen retry found {} workspace seeds owned by {key}; refusing to choose one",
            recovered_workspace_seeds.len()
        ));
    }
    Ok(ReopenIntentMatch {
        open_owner,
        layout: recovered_layouts.pop(),
        workspace_seed: recovered_workspace_seeds.pop(),
    })
}

pub(crate) fn inspect_sleep_tab(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    snapshot: &SessionSnapshotPayload,
) -> Result<Option<(String, String)>, String> {
    let found = find_reopen_intent(connector, key, context, owner, snapshot)?;
    match found.layout {
        Some(layout) if layout.root.pane_count() == 1 => {
            Ok(Some((layout.tab_id, layout.focused_pane_id)))
        }
        Some(_) => Err("The wake tab's layout changed; it was not adopted".into()),
        None => Ok(None),
    }
}

fn ensure_workspace_and_tab(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    tab_exists: bool,
    root: &ClosedLayoutNode,
    notices: &mut Vec<String>,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    if tab_exists {
        return Err("the requested tab already exists".into());
    }
    let snapshot = fetch_session_with_connector(connector)
        .map_err(|error| format!("session.snapshot before reopen failed: {}", error.message()))?;
    let found = find_reopen_intent(connector, key, context, owner, &snapshot)?;
    if let Some(layout) = found.layout {
        let layout = repair_incomplete_tab_layout(connector, key, context, root, layout, notices)?;
        return Ok(restore_tab_position(
            connector, key, context, layout, notices,
        ));
    }
    let (workspace_id, seed_tab_id) = if let Some(seed) = found.workspace_seed {
        (seed.0, Some(seed.1))
    } else if let Some(owner) = found.open_owner {
        (owner, None)
    } else {
        // The owner's first tab is the seed the layout replaces.
        let (workspace_id, first_tab) = ensure_owner(
            connector,
            owner,
            true,
            reopen_intent_env(key, ReopenIntentStage::Workspace),
        )
        .map_err(|failure| failure.message().to_owned())?;
        (workspace_id, first_tab.map(|(tab_id, _, _)| tab_id))
    };
    let tagged_root = tag_reopen_layout(root, key, ReopenIntentStage::Layout);
    let applied = reopen_request(
        connector,
        &format!("herdr-core:{key}:layout"),
        "layout.apply",
        wire::layout_apply_params(
            &workspace_id,
            seed_tab_id.as_deref(),
            context.herdr_tab_label(),
            &tagged_root,
        )?,
    )?;
    let layout = wire::applied_layout(applied)?;
    let layout = repair_incomplete_tab_layout(connector, key, context, root, layout, notices)?;
    Ok(restore_tab_position(
        connector, key, context, layout, notices,
    ))
}

fn repair_incomplete_tab_layout(
    connector: &dyn ApiConnector,
    key: &str,
    context: &ClosedContext,
    root: &ClosedLayoutNode,
    mut layout: crate::recent_closed::ClosedLayout,
    notices: &mut Vec<String>,
) -> Result<crate::recent_closed::ClosedLayout, String> {
    let expected = root.pane_count();
    let first_actual = layout.root.pane_count();
    if first_actual == expected {
        return Ok(layout);
    }
    if first_actual > expected {
        return Err(format!(
            "the reopened tab returned {first_actual} panes, expected {expected}; extra panes were not adopted or overwritten and retry is available"
        ));
    }
    let repaired = reopen_request(
        connector,
        &format!("herdr-core:{key}:layout-repair"),
        "layout.apply",
        wire::layout_apply_params(
            &layout.workspace_id,
            Some(&layout.tab_id),
            context.herdr_tab_label(),
            &tag_reopen_layout(root, key, ReopenIntentStage::Layout),
        )?,
    )?;
    layout = wire::applied_layout(repaired)?;
    let repaired_actual = layout.root.pane_count();
    if repaired_actual > expected {
        return Err(format!(
            "the reopened tab restored {first_actual} of {expected} panes, then returned {repaired_actual} after repair; extra panes were not adopted or overwritten and retry is available"
        ));
    }
    if repaired_actual < expected {
        return Err(format!(
            "the reopened tab restored {first_actual} of {expected} panes, then {repaired_actual} after repair; retry is available"
        ));
    }
    notices.push("The incomplete tab restore was repaired before reopening sessions".into());
    Ok(layout)
}

fn reopen_pane(
    connector: &dyn ApiConnector,
    folders: &dyn Fn(&str) -> bool,
    key: &str,
    context: &ClosedContext,
    pane: &ClosedPane,
    placement: &PanePlacement,
    request: &ReopenRequest,
) -> Result<ReopenOutcome, String> {
    let mut notices = Vec::new();
    let cwd = restored_cwd(folders, &pane.cwd, &context.checkout_path, &mut notices);
    let (restored_pane_id, restored_tab_id) = if request.tab_exists {
        let current_layout = export_reopen_layout(connector, key, &context.tab_id)?;
        let marker = reopen_intent_marker(key, ReopenIntentStage::Pane);
        let mut recovered_panes = Vec::new();
        pane_ids_with_reopen_marker(&current_layout.root, &marker, &mut recovered_panes);
        if recovered_panes.len() > 1 {
            return Err(format!(
                "reopen retry found {} panes owned by {key}; refusing to choose one",
                recovered_panes.len(),
            ));
        }
        let (new_pane_id, current_target_was_first) = if let Some(pane_id) = recovered_panes.pop() {
            let current = current_layout.root.placement_for(&pane_id).ok_or_else(|| {
                format!("reopen retry could not locate owned pane {pane_id} in its layout")
            })?;
            (pane_id, current.target_was_first)
        } else if let Some(target) = placement.neighbor_pane_id.as_ref() {
            if request.fallback_pane_id.as_deref() != Some(target.as_str()) {
                return Err(format!(
                    "the original sibling pane {target} is no longer available"
                ));
            }
            let value = reopen_request(
                connector,
                &format!("herdr-core:{key}:pane"),
                "pane.split",
                wire::pane_split_with_ratio_params(
                    target,
                    placement.direction,
                    &cwd,
                    placement.ratio,
                    reopen_intent_env(key, ReopenIntentStage::Pane),
                )?,
            )?;
            (wire::split_pane(value)?, false)
        } else {
            let pane_root = tag_reopen_layout(
                &ClosedLayoutNode::Pane {
                    pane_id: None,
                    label: crate::start_claim::public_label(pane.label.as_deref()),
                    cwd: Some(cwd.clone()),
                    command: None,
                    env: Default::default(),
                },
                key,
                ReopenIntentStage::Pane,
            );
            let root = wrap_surviving_subtree(
                current_layout.root.clone(),
                &placement.parent_path,
                pane_root,
                placement.direction,
                placement.ratio,
                placement.target_was_first,
            )?;
            let value = reopen_request(
                connector,
                &format!("herdr-core:{key}:pane-layout"),
                "layout.apply",
                // The tab is still there under the name it has now, which
                // may be a rename made after the close.
                wire::layout_apply_params(
                    &current_layout.workspace_id,
                    Some(&context.tab_id),
                    None,
                    &root,
                )?,
            )?;
            let applied = wire::applied_layout(value)?;
            let mut owned = Vec::new();
            pane_ids_with_reopen_marker(&applied.root, &marker, &mut owned);
            if owned.len() != 1 {
                return Err(format!(
                    "reopen layout created {} owned panes instead of one",
                    owned.len()
                ));
            }
            (owned.remove(0), placement.target_was_first)
        };
        if current_target_was_first != placement.target_was_first
            && let Some(target) = placement.neighbor_pane_id.as_ref()
            && let Err(message) = reopen_request(
                connector,
                &format!("herdr-core:{key}:swap"),
                "pane.swap",
                wire::pane_swap_params(&new_pane_id, target)?,
            )
        {
            notices.push(format!(
                "The pane reopened but its original side could not be restored: {message}"
            ));
        }
        (new_pane_id, context.tab_id.clone())
    } else {
        let root = ClosedLayoutNode::Pane {
            pane_id: None,
            label: crate::start_claim::public_label(pane.label.as_deref()),
            cwd: Some(cwd.clone()),
            command: None,
            env: Default::default(),
        };
        let layout = ensure_workspace_and_tab(
            connector,
            key,
            context,
            reopen_owner(request)?,
            false,
            &root,
            &mut notices,
        )?;
        (layout.focused_pane_id, layout.tab_id)
    };
    let agents = pane
        .agent
        .as_ref()
        .and_then(|agent| {
            start_or_degrade_agent(
                connector,
                key,
                0,
                &restored_pane_id,
                agent,
                request.codex_daemon,
                &mut notices,
            )
        })
        .into_iter()
        .collect();
    Ok(ReopenOutcome {
        tab_id: Some(restored_tab_id),
        consumed: true,
        focused_pane_id: Some(restored_pane_id.clone()),
        notices: notices
            .into_iter()
            .map(|message| ReopenNotice {
                pane_id: Some(restored_pane_id.clone()),
                message,
            })
            .collect(),
        agents,
    })
}

fn wrap_surviving_subtree(
    surviving: ClosedLayoutNode,
    parent_path: &[ClosedLayoutBranch],
    reopened: ClosedLayoutNode,
    direction: crate::recent_closed::ClosedSplitDirection,
    ratio: f32,
    target_was_first: bool,
) -> Result<ClosedLayoutNode, String> {
    if parent_path.is_empty() {
        let (first, second) = if target_was_first {
            (reopened, surviving)
        } else {
            (surviving, reopened)
        };
        return Ok(ClosedLayoutNode::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        });
    }
    let ClosedLayoutNode::Split {
        direction: surviving_direction,
        ratio: surviving_ratio,
        first,
        second,
    } = surviving
    else {
        return Err("the surviving layout no longer matches the closed pane's parent path".into());
    };
    let (first, second) = match parent_path[0] {
        ClosedLayoutBranch::First => (
            Box::new(wrap_surviving_subtree(
                *first,
                &parent_path[1..],
                reopened,
                direction,
                ratio,
                target_was_first,
            )?),
            second,
        ),
        ClosedLayoutBranch::Second => (
            first,
            Box::new(wrap_surviving_subtree(
                *second,
                &parent_path[1..],
                reopened,
                direction,
                ratio,
                target_was_first,
            )?),
        ),
    };
    Ok(ClosedLayoutNode::Split {
        direction: surviving_direction,
        ratio: surviving_ratio,
        first,
        second,
    })
}

fn reopen_owner(request: &ReopenRequest) -> Result<&OwnerOpen, String> {
    request
        .owner
        .as_ref()
        .ok_or_else(|| "the closed item names no checkout to reopen in".to_owned())
}

#[allow(clippy::too_many_arguments)] // one closed tab with its owner, layout and folders
fn reopen_tab(
    connector: &dyn ApiConnector,
    folders: &dyn Fn(&str) -> bool,
    key: &str,
    context: &ClosedContext,
    owner: &OwnerOpen,
    root: &ClosedLayoutNode,
    panes: &[ClosedPane],
    codex_daemon: crate::codex_launch::CodexDaemon,
) -> Result<ReopenOutcome, String> {
    let pane_map = panes
        .iter()
        .map(|pane| (pane.pane_id.clone(), pane.clone()))
        .collect();
    let mut terminal_ids = Vec::new();
    root.known_pane_ids(&pane_map, &mut terminal_ids);
    let mut common_notices = Vec::new();
    let Some(root) = root.resolve_panes(
        &pane_map,
        &context.checkout_path,
        folders,
        &mut common_notices,
    ) else {
        return Err("the closed tab contained no panes".into());
    };
    let layout = ensure_workspace_and_tab(
        connector,
        key,
        context,
        owner,
        false,
        &root,
        &mut common_notices,
    )?;
    let mut new_ids = Vec::new();
    layout.root.pane_ids(&mut new_ids);
    let terminal_panes = terminal_ids
        .iter()
        .filter_map(|id| pane_map.get(id))
        .collect::<Vec<_>>();
    ensure_complete_tab_restore(terminal_panes.len(), new_ids.len())?;
    let mut notices = Vec::new();
    let mut agents = Vec::new();
    for (index, (pane, new_id)) in terminal_panes.iter().zip(new_ids.iter()).enumerate() {
        let mut pane_notices = Vec::new();
        if !folders(&pane.cwd) {
            pane_notices.push(format!(
                "{} no longer exists; reopened in the checkout root",
                pane.cwd
            ));
        }
        if let Some(agent) = &pane.agent {
            agents.extend(start_or_degrade_agent(
                connector,
                key,
                index,
                new_id,
                agent,
                codex_daemon,
                &mut pane_notices,
            ));
        }
        notices.extend(pane_notices.into_iter().map(|message| ReopenNotice {
            pane_id: Some(new_id.clone()),
            message,
        }));
    }
    let first = new_ids.first().cloned();
    notices.extend(common_notices.into_iter().map(|message| ReopenNotice {
        pane_id: first.clone(),
        message,
    }));
    Ok(ReopenOutcome {
        tab_id: Some(layout.tab_id),
        consumed: true,
        focused_pane_id: first,
        notices,
        agents,
    })
}

fn ensure_complete_tab_restore(expected: usize, actual: usize) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "the reopened tab restored {actual} of {expected} terminal panes; retry is available"
        ))
    }
}

fn restored_cwd(
    folders: &dyn Fn(&str) -> bool,
    cwd: &str,
    checkout_root: &str,
    notices: &mut Vec<String>,
) -> String {
    if folders(cwd) {
        cwd.to_owned()
    } else {
        notices.push(format!(
            "{cwd} no longer exists; reopened in the checkout root"
        ));
        checkout_root.to_owned()
    }
}

/// Starts `agent` in a restored pane with its conversation, or a new one when
/// the resume does not take, each with the launch choice it was started
/// with; answers what started, or none when only the shell was kept.
fn start_or_degrade_agent(
    connector: &dyn ApiConnector,
    key: &str,
    index: usize,
    pane_id: &str,
    agent: &ClosedAgent,
    codex_daemon: crate::codex_launch::CodexDaemon,
    notices: &mut Vec<String>,
) -> Option<ReopenedAgent> {
    let name = format!("reopen-{key}-{index}");
    let launch = match crate::runtime::agent_choice::launch_arguments(&agent.kind, &agent.launch) {
        Ok(launch) => launch,
        Err(message) => {
            notices.push(format!(
                "Agent could not be started; the shell was kept: {message}"
            ));
            return None;
        }
    };
    let started = |session_id: Option<&String>| ReopenedAgent {
        pane_id: pane_id.to_owned(),
        kind: agent.kind.clone(),
        name: name.clone(),
        launch: agent.launch.clone(),
        session_id: session_id.cloned(),
    };
    let interrupted_reused_agent = interrupt_reused_agent(connector, key, index, pane_id);
    let pane_was_clear = matches!(&interrupted_reused_agent, Ok(false));
    let resume = resume_arguments(agent);
    if let Some(mut args) = resume {
        args.extend(launch.iter().cloned());
        // A resume line is a session id, far under every terminal's line
        // limit, so the restore asks no node.
        let result = start_agent(
            connector,
            None,
            &format!("herdr-core:{key}:agent:{index}:resume"),
            pane_id,
            crate::agent_start::ShellStart::Plain,
            &name,
            &agent.kind,
            args,
            codex_daemon,
        );
        if result.is_ok() || (pane_was_clear && agent_is_running(connector, pane_id, &agent.kind)) {
            return Some(started(agent.session_id.as_ref()));
        }
        let preparation = interrupted_reused_agent
            .as_ref()
            .err()
            .map(|message| format!("; restored pane preparation failed: {message}"))
            .unwrap_or_default();
        notices.push(format!(
            "Session {} could not be resumed{preparation}; started a new conversation",
            agent.session_id.as_deref().unwrap_or("unknown"),
        ));
    } else {
        notices
            .push("Previous conversation could not be resumed; started a new conversation".into());
    }
    if let Err(message) = start_agent(
        connector,
        None,
        &format!("herdr-core:{key}:agent:{index}:fresh"),
        pane_id,
        crate::agent_start::ShellStart::Plain,
        &name,
        &agent.kind,
        launch,
        codex_daemon,
    ) && !(pane_was_clear && agent_is_running(connector, pane_id, &agent.kind))
    {
        notices.push(format!(
            "Agent could not be started; the shell was kept: {message}"
        ));
        return None;
    }
    Some(started(None))
}

/// Herdr can reuse the just-closed pane id while its old PTY is still alive.
/// A layout-only restore then surfaces that old process in the new tab before
/// `agent.start` can apply the explicit resume arguments. Closing the process
/// here returns the restored pane to its shell prompt; the start waits for
/// that prompt (`agent_start`) and `agent.start` remains the single owner of
/// session resumption.
fn interrupt_reused_agent(
    connector: &dyn ApiConnector,
    key: &str,
    index: usize,
    pane_id: &str,
) -> Result<bool, String> {
    let snapshot = fetch_session_with_connector(connector).map_err(|error| {
        format!(
            "session.snapshot before agent resume failed: {}",
            error.message()
        )
    })?;
    if !snapshot
        .agents
        .iter()
        .any(|candidate| candidate.pane_id.as_deref() == Some(pane_id))
    {
        return Ok(false);
    }
    reopen_request(
        connector,
        &format!("herdr-core:{key}:agent:{index}:interrupt-reused"),
        "pane.send_text",
        wire::pane_send_text_params(pane_id, "\u{3}")?,
    )?;
    Ok(true)
}

fn agent_is_running(connector: &dyn ApiConnector, pane_id: &str, kind: &str) -> bool {
    fetch_session_with_connector(connector).is_ok_and(|snapshot| {
        snapshot.agents.iter().any(|agent| {
            agent.pane_id.as_deref() == Some(pane_id)
                && agent.agent.as_deref().is_some_and(|agent_kind| {
                    hide_agent_adapter::canonical_kind(agent_kind)
                        .eq_ignore_ascii_case(hide_agent_adapter::canonical_kind(kind))
                })
        })
    })
}

/// `node` runs the pane (`agent_start::start_at_shell_checked`); none
/// when it cannot be reached. A start whose shell had not finished starting
/// is refused as `agent_start::SHELL_STARTING` with the screen's last line,
/// so the spawn that asked can say what the shell waits for.
#[allow(clippy::too_many_arguments)] // the start identity and the pane's node
pub(crate) fn start_agent(
    connector: &dyn ApiConnector,
    node: Option<&dyn crate::node_access::NodeLink>,
    request_id: &str,
    pane_id: &str,
    shell: crate::agent_start::ShellStart,
    name: &str,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
) -> Result<String, String> {
    start_agent_checked(
        connector,
        node,
        request_id,
        pane_id,
        shell,
        name,
        kind,
        args,
        codex_daemon,
        &|| Ok(()),
    )
    .map_err(|error| match error {
        crate::agent_start::StartError::ShellStarting(screen) => crate::coordination::refusal(
            crate::agent_start::SHELL_STARTING,
            screen.as_deref().unwrap_or_default(),
        ),
        // Herdr refused the pane as busy until the start's deadline: nothing
        // was typed, and the same intent asks again in the same pane.
        crate::agent_start::StartError::Herdr(ApiError::Remote { code, .. })
            if code == crate::agent_start::PANE_BUSY =>
        {
            PANE_BUSY_REFUSAL.to_owned()
        }
        other => start_error_message(other),
    })
}

/// A spawn's start that Herdr refused as busy for its whole wait.
pub(crate) const PANE_BUSY_REFUSAL: &str = "pane_busy";

#[allow(clippy::too_many_arguments)] // the existing start identity plus its effect admission
fn start_agent_checked(
    connector: &dyn ApiConnector,
    node: Option<&dyn crate::node_access::NodeLink>,
    request_id: &str,
    pane_id: &str,
    shell: crate::agent_start::ShellStart,
    name: &str,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<String, crate::agent_start::StartError> {
    let answer = crate::agent_start::start_at_shell_checked(
        connector,
        node,
        request_id,
        pane_id,
        shell,
        wire::agent_start_params(pane_id, name, kind, args, codex_daemon)
            .map_err(crate::agent_start::StartError::NotStarted)?,
        Duration::from_secs(5),
        check,
    )?;
    wire::started_agent(answer).map_err(crate::agent_start::StartError::NotStarted)
}

/// A start's failure as an operator reads it.
fn start_error_message(error: crate::agent_start::StartError) -> String {
    match error {
        crate::agent_start::StartError::NotStarted(message) => message,
        crate::agent_start::StartError::ShellStarting(screen) => {
            crate::agent_start::shell_starting_message(screen.as_deref())
        }
        crate::agent_start::StartError::Herdr(error) => format!("agent.start failed: {error}"),
    }
}

/// One action-only node proof, never called under Runtime. Authenticated
/// Identity and Activity facts are checked before and after the bounded read.
pub(crate) fn confirm_session_launch(
    node: &dyn crate::node_access::NodeLink,
    kind: &str,
    id: &str,
    cwd: Option<&str>,
    source_reference: Option<&crate::sidebar::SessionAgentSessionPayload>,
) -> Result<(), String> {
    let Some(agent) =
        hide_session::Agent::from_kind(kind).filter(|agent| agent.requires_native_proof())
    else {
        return Ok(());
    };
    use hide_node_link::sessions::ReaderFeature;
    let check = || {
        for feature in [ReaderFeature::Identity, ReaderFeature::Activity] {
            hide_node_link::link::check_reader_support(node, agent, feature)
                .map_err(|_| "session_route_reader_unavailable".to_owned())?;
        }
        Ok(())
    };
    check()?;
    let source_reference = source_reference.ok_or("session_route_reference_unconfirmed")?;
    if source_reference.kind != agent.proof_reference_kind() {
        return Err("session_route_reference_unconfirmed".into());
    }
    let request = hide_session::session_activity::SessionActivityRequest {
        agent,
        reference_kind: source_reference.kind.clone(),
        reference_value: source_reference.value.clone(),
        cwd: Some(cwd.ok_or("session_route_cwd_unconfirmed")?.into()),
        exact_route: true,
        expected_id: Some(id.into()),
    };
    // A proof that met the agent writing its records is read again: agy
    // writes its index after every turn, and the route stands or falls on
    // what the next read finds.
    let _: hide_session::session_activity::SessionActivity = read_past_writes(
        node,
        || hide_node_link::protocol::Call::SessionActivity {
            request: request.clone(),
        },
        Duration::from_secs(10),
        Instant::now() + Duration::from_secs(10),
    )
    .map_err(|error| {
        let reason = match &error {
            crate::node_access::LinkError::Refused(error)
                if error.code == hide_node_link::ErrorCode::Busy =>
            {
                "session_route_in_motion"
            }
            crate::node_access::LinkError::Refused(error) => match error
                .message
                .strip_prefix("session_checkpoint_invalid:")
                .unwrap_or(&error.message)
            {
                "cursor_launch_root_unsupported"
                | "cursor_launch_root_ambiguous"
                | "cursor_launch_environment_invalid"
                | "cursor_launch_root_unconfirmed"
                | "cursor_launch_root_changed" => error
                    .message
                    .strip_prefix("session_checkpoint_invalid:")
                    .unwrap_or(&error.message),
                _ => "session_route_unconfirmed",
            },
            _ => "session_route_unconfirmed",
        };
        crate::diagnostic!(json!({
            "component": "session_route", "kind": "session_route.refused",
            "agent_kind": kind, "native_session_id": id, "reason": reason,
        }));
        "session_route_unconfirmed".to_owned()
    })?;
    check()
}

fn fetch_pane_layout(
    connector: &dyn ApiConnector,
    pane_id: &str,
) -> Result<PaneLayoutSnapshot, String> {
    let result = control_request(connector, "pane.layout", wire::pane_layout_params(pane_id)?)?;
    let layout = wire::pane_layout(result)?;
    project_layout(&layout)
}

fn execute_pane_focus(
    connector: &dyn ApiConnector,
    pane_id: &str,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    execute_pane_focus_with_timeout(connector, pane_id, Duration::from_secs(5))
}

fn execute_pane_focus_with_timeout(
    connector: &dyn ApiConnector,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    // These three responses share the established absolute read deadline and
    // 64 KiB frame cap. A lost mutation response has a different outcome from
    // an authoritative read failing after Herdr already acknowledged it.
    request_small_response(
        connector,
        "pane.focus",
        wire::pane_target_params(pane_id)?,
        response_timeout,
    )
    .map_err(|error| match error {
        ApiError::Remote { code, message } => {
            ControlFailure::Definite(format!("pane.focus was refused: {code}: {message}"))
        }
        ApiError::NotRunning(message)
        | ApiError::Transport(message)
        | ApiError::Malformed(message) => {
            ControlFailure::Ambiguous(format!("pane.focus result is unknown: {message}"))
        }
    })?;
    confirm_pane_focus_with_timeout(connector, pane_id, response_timeout)
}

/// Herdr's answer to whether a pane holds its focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FocusReadback {
    /// The pane is focused in the tab its workspace shows.
    Confirmed,
    /// Herdr's focus is on another pane or tab, so the move the stream showed
    /// is not Herdr's focus now.
    Elsewhere,
    /// Herdr could not be read; its reason.
    Unread(String),
}

/// Read current focus without producing another focus effect. Herdr's pinned
/// event stream has no ordering cursor, so an external move needs this same
/// confirmation before an older queued layout can replace Hide's selection.
pub(crate) fn confirm_pane_focus(connector: &dyn ApiConnector, pane_id: &str) -> FocusReadback {
    match read_pane_focus(connector, pane_id, Duration::from_secs(5)) {
        Ok((layout, active_tab_id))
            if layout.focused_pane_id == pane_id && active_tab_id == layout.tab_id =>
        {
            FocusReadback::Confirmed
        }
        Ok(_) => FocusReadback::Elsewhere,
        Err(error) => FocusReadback::Unread(error.message().to_owned()),
    }
}

fn confirm_pane_focus_with_timeout(
    connector: &dyn ApiConnector,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    let (layout, active_tab_id) = read_pane_focus(connector, pane_id, response_timeout)?;
    if layout.focused_pane_id != pane_id || active_tab_id != layout.tab_id {
        return Err(ControlFailure::Definite(format!(
            "Herdr did not confirm pane {pane_id}: pane {} is focused in tab {}, and workspace {} shows tab {active_tab_id}",
            layout.focused_pane_id, layout.tab_id, layout.workspace_id
        )));
    }
    Ok(layout)
}

/// Herdr's layout of the tab holding `pane_id` and the tab its workspace
/// shows.
fn read_pane_focus(
    connector: &dyn ApiConnector,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<(PaneLayoutSnapshot, String), ControlFailure> {
    let layout = request_small_response(
        connector,
        "pane.layout",
        wire::pane_layout_params(pane_id)?,
        response_timeout,
    )
    .map_err(|error| format!("pane.layout failed: {error}"))?;
    let layout = project_layout(&wire::pane_layout(layout)?)?;
    let workspace = request_small_response(
        connector,
        "workspace.get",
        wire::workspace_target_params(&layout.workspace_id)?,
        response_timeout,
    )
    .map_err(|error| format!("workspace.get failed: {error}"))?;
    let active_tab_id = wire::workspace_active_tab(workspace)?;
    Ok((layout, active_tab_id))
}

/// One worker runs the control lane: it sends a job, settles its result under
/// the runtime lock, and takes the next job the lane hands back, until the
/// lane is idle. Every socket read is outside the runtime lock, and a job's
/// result can only settle the request carrying its own identity.
pub(crate) fn spawn_control_lane(first: LaneStart) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-control-lane".to_owned())
        .spawn(move || {
            let mut current = first;
            loop {
                let started = Instant::now();
                let (context, finished) = match current {
                    LaneStart::Tab { context, action } => {
                        // A panic here must still settle the job: the lane
                        // goes idle only when this worker reports back, so a
                        // dead worker would hold every later control behind
                        // it. The effect is unknown, so it is ambiguous.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute_local_control(context.api_connector.as_ref(), &action)
                        }))
                        .unwrap_or_else(|_| Err(lane_worker_panicked(action.kind())));
                        (context, Finished::Tab(action, result))
                    }
                    LaneStart::PaneFocus { context, control } => {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute_pane_focus(context.api_connector.as_ref(), &control.target_id)
                        }))
                        .unwrap_or_else(|_| Err(lane_worker_panicked("pane.focus")));
                        (context, Finished::PaneFocus(control, result))
                    }
                };
                let elapsed_ms = started.elapsed().as_millis();
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let (changed, next) = match runtime.lock() {
                    Ok(mut guard) => match finished {
                        Finished::Tab(action, result) => {
                            guard.complete_lane_tab(action, result, elapsed_ms)
                        }
                        Finished::PaneFocus(control, result) => {
                            guard.complete_lane_pane_focus(control, result, elapsed_ms)
                        }
                    },
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
                let Some(next) = next else {
                    return;
                };
                current = next;
            }
        })
        .map(|_| ())
        .map_err(|error| format!("control lane worker could not be started: {error}"))
}

fn lane_worker_panicked(kind: &str) -> ControlFailure {
    crate::diagnostic!(serde_json::json!({
        "component": "control_lane",
        "kind": "control.worker_panicked",
        "action": kind,
    }));
    ControlFailure::Ambiguous(format!(
        "{kind} result is unknown: the control worker failed"
    ))
}

/// What a lane job produced, waiting to be settled under the runtime lock.
enum Finished {
    Tab(
        RemoteControlAction,
        Result<RemoteControlOutcome, ControlFailure>,
    ),
    PaneFocus(
        PendingPaneFocusControl,
        Result<PaneLayoutSnapshot, ControlFailure>,
    ),
}

pub fn spawn_pane_control(context: LiveContext, action: PaneControlAction) -> Result<(), String> {
    spawn_pane_control_with_generation(context, action, None)
}

/// Starts a pane control worker with the connection generation that created it.
/// The generation is part of the operation identity, so a late answer from an
/// older Herdr socket cannot settle a newer request for the same pane.
pub fn spawn_pane_control_with_generation(
    context: LiveContext,
    action: PaneControlAction,
    connection_generation: Option<u64>,
) -> Result<(), String> {
    let worker_name = match &action {
        PaneControlAction::Project { .. } => "herdr-core-pane-project".to_owned(),
        PaneControlAction::Focus { .. } => "herdr-core-pane-focus".to_owned(),
        PaneControlAction::Split { direction, .. } => {
            format!("herdr-core-pane-split-{}", direction.as_str())
        }
        PaneControlAction::Resize { direction, .. } => {
            format!("herdr-core-pane-resize-{}", direction.as_str())
        }
        PaneControlAction::ToggleZoom { .. } => "herdr-core-pane-zoom".to_owned(),
        PaneControlAction::Close { .. } => "herdr-core-pane-close".to_owned(),
        PaneControlAction::MoveToNewTab { .. } => "herdr-core-pane-move".to_owned(),
    };
    thread::Builder::new()
        .name(worker_name)
        .spawn(move || {
            let started = Instant::now();
            let result = execute_pane_control(context.api_connector.as_ref(), &action);
            let elapsed_ms = started.elapsed().as_millis();
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_pane_control_failure_with_generation(
                    action,
                    result,
                    elapsed_ms,
                    connection_generation,
                ),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("pane control worker could not be started: {error}"))
}

/// Runs one `herdr agent new` and reports what it produced.
///
/// This is a CLI wrapper rather than a socket request because agent lifecycle
/// is CLI-owned: the command starts a process, waits for the agent to come up,
/// and reports a startup failure as its own exit status. `HERDR_SOCKET_PATH` is
/// set from the live context for the same reason every other spawned herdr
/// process sets it - it is what keeps this build's commands on this build's
/// server.
/// How many lines of history a pane search asks Herdr for.
///
/// Herdr caps what it keeps; this is the ceiling on what is searched, and it
/// is reported alongside the count so a truncated buffer is visible rather
/// than quietly reported as the whole thing.
const PANE_FIND_LINE_LIMIT: u32 = 10_000;

/// What a pane search needs, gathered under the runtime mutex so the worker
/// carries no reference back into runtime state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneFindRequest {
    pub term: String,
    pub options: PaneFindOptions,
    /// Which match to move to once the search lands: 0 keeps the current one,
    /// and stepping is relative so a search and a step share one path.
    pub step: i64,
    /// The index the shell is on now, so a step continues from it rather than
    /// restarting at the top every time.
    pub current_index: usize,
}

/// What a pane search found, in the shape the runtime stores.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PaneFindOutcome {
    pub term: String,
    pub total: usize,
    /// 1-based position of the current match, or 0 when there is none.
    pub index: usize,
    pub truncated: bool,
    /// The viewport move that puts the current match on screen, as a direction
    /// and a line count. `None` when it is already there.
    pub scroll: Option<(String, u16)>,
}

/// Where one pane's socket requests go: this machine's Herdr, or a device's,
/// with the id that Herdr knows the pane by beside the id the core projects.
#[derive(Clone)]
pub struct PaneApiRoute {
    pane_id: String,
    herdr_pane_id: String,
    api_connector: Arc<dyn ApiConnector>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl PaneApiRoute {
    pub(crate) fn local(context: &LiveContext, pane_id: &str) -> Self {
        Self {
            pane_id: pane_id.to_owned(),
            herdr_pane_id: pane_id.to_owned(),
            api_connector: Arc::clone(&context.api_connector),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    pub(crate) fn remote(
        context: &RemoteControlContext,
        pane_id: &str,
        herdr_pane_id: &str,
    ) -> Self {
        Self {
            pane_id: pane_id.to_owned(),
            herdr_pane_id: herdr_pane_id.to_owned(),
            api_connector: Arc::clone(&context.api_connector),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
        }
    }

    /// Runs `work` on its own thread and hands its result to `ingest` under
    /// the runtime lock, notifying when that changed the snapshot.
    fn spawn<T: Send + 'static>(
        self,
        name: &str,
        work: impl FnOnce(&dyn ApiConnector, &str) -> T + Send + 'static,
        ingest: impl FnOnce(&mut Runtime, &str, T) -> bool + Send + 'static,
    ) -> Result<(), String> {
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let result = work(self.api_connector.as_ref(), &self.herdr_pane_id);
                let Some(runtime) = self.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => ingest(&mut guard, &self.pane_id, result),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    self.notifier.notify();
                }
            })
            .map(|_| ())
            .map_err(|error| format!("{name} worker could not be started: {error}"))
    }
}

/// Searches a pane's whole scrollback and moves the viewport to the match.
///
/// The two reads and the search happen on this thread, never under the runtime
/// mutex: the buffer is thousands of lines and every shell snapshot read blocks
/// on that mutex.
pub fn spawn_pane_find(route: PaneApiRoute, request: PaneFindRequest) -> Result<(), String> {
    route.spawn(
        "herdr-core-pane-find",
        move |connector, herdr_pane_id| run_pane_find(connector, herdr_pane_id, &request),
        |runtime, pane_id, result| runtime.ingest_pane_find(pane_id, result),
    )
}

/// Opens an agent's own search in its pane, unless Herdr holds the pane's
/// history, which Hide's find bar searches instead (`agent_find.rs`).
///
/// The requests run on this thread, never under the runtime mutex.
pub(crate) fn spawn_agent_find(
    route: PaneApiRoute,
    request_id: String,
    find: AgentFind,
    gate: Arc<crate::input_gate::PaneInputGate>,
) -> Result<(), String> {
    let core_pane_id = route.pane_id.clone();
    route.spawn(
        "herdr-core-agent-find",
        move |connector, herdr_pane_id| {
            open_agent_find(connector, herdr_pane_id, find, &gate, &core_pane_id)
        },
        move |runtime, pane_id, result| runtime.ingest_agent_find(pane_id, request_id, result),
    )
}

fn open_agent_find(
    connector: &dyn ApiConnector,
    pane_id: &str,
    find: AgentFind,
    gate: &Arc<crate::input_gate::PaneInputGate>,
    core_pane_id: &str,
) -> Result<PaneFindRoute, String> {
    // An agent that draws inline leaves its conversation in Herdr's history,
    // where the find bar reaches all of it; its own search may not exist in
    // that mode (Claude Code's default renderer has none).
    let pane = control_request(connector, "pane.get", wire::pane_target_params(pane_id)?)?;
    if wire::pane_scroll(pane)?.is_some_and(|scroll| scroll.max_offset_from_bottom > 0) {
        return Ok(PaneFindRoute::Bar);
    }
    let visible = match find.already_open {
        Some(_) => Some(read_pane_text(connector, pane_id, "visible")?.text),
        None => None,
    };
    // The keys go into the agent's own search box; into the shell under a
    // sleeping agent they would be typed as a command.
    let _admission = gate
        .admit(core_pane_id)
        .map_err(|_| "The agent is asleep, so its own search is closed".to_owned())?;
    control_request(
        connector,
        "pane.send_keys",
        wire::pane_send_keys_params(pane_id, find.keys(visible.as_deref()))?,
    )?;
    Ok(PaneFindRoute::Agent)
}

/// Herdr's scroll position for one pane, in lines above the bottom.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneScroll {
    pub offset_from_bottom: u64,
    pub max_offset_from_bottom: u64,
}

/// Moves an observed pane's viewport by `lines` (positive shows older lines).
///
/// `pane.scroll` takes an absolute offset and any socket client may send it,
/// which is what an observer without a terminal writer has; the offset comes
/// from the metrics `pane.get` reports, and Herdr clamps an overshoot at the
/// top. Two requests off the runtime mutex; the core keeps one in flight per
/// pane and sums the wheels that arrive meanwhile.
pub fn spawn_viewport_scroll(route: PaneApiRoute, lines: i32) -> Result<(), String> {
    route.spawn(
        "herdr-core-viewport-scroll",
        move |connector, herdr_pane_id| scroll_viewport(connector, herdr_pane_id, lines),
        |runtime, pane_id, result| runtime.ingest_viewport_scroll(pane_id, result),
    )
}

/// Why an observed pane's viewport did not move.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewportScrollError {
    /// Herdr answered and refused the request (an older device Herdr without
    /// `pane.scroll`, say): this client cannot scroll the pane.
    Refused(String),
    /// Herdr was not reached or answered something unreadable; nothing is
    /// known about the pane's scrolling.
    Unreachable(String),
}

fn scroll_request(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
) -> Result<Option<PaneScroll>, ViewportScrollError> {
    let answer = request_with_connector(connector, method, params, Duration::from_secs(5))
        .map_err(|error| match error {
            ApiError::Remote { code, message } => {
                ViewportScrollError::Refused(format!("{method} was refused: {code}: {message}"))
            }
            ApiError::NotRunning(message)
            | ApiError::Transport(message)
            | ApiError::Malformed(message) => {
                ViewportScrollError::Unreachable(format!("{method} failed: {message}"))
            }
        })?;
    wire::pane_scroll(answer).map_err(ViewportScrollError::Unreachable)
}

fn scroll_viewport(
    connector: &dyn ApiConnector,
    pane_id: &str,
    lines: i32,
) -> Result<Option<PaneScroll>, ViewportScrollError> {
    let target = wire::pane_target_params(pane_id).map_err(ViewportScrollError::Unreachable)?;
    let Some(current) = scroll_request(connector, "pane.get", target)? else {
        return Ok(None);
    };
    let distance = u64::from(lines.unsigned_abs());
    let target = if lines > 0 {
        current.offset_from_bottom.saturating_add(distance)
    } else {
        current.offset_from_bottom.saturating_sub(distance)
    };
    if target == current.offset_from_bottom {
        return Ok(Some(current));
    }
    let params =
        wire::pane_scroll_params(pane_id, target).map_err(ViewportScrollError::Unreachable)?;
    scroll_request(connector, "pane.scroll", params)
}

fn run_pane_find(
    connector: &dyn ApiConnector,
    pane_id: &str,
    request: &PaneFindRequest,
) -> Result<PaneFindOutcome, String> {
    if request.term.is_empty() {
        return Ok(PaneFindOutcome {
            term: String::new(),
            ..PaneFindOutcome::default()
        });
    }
    let buffer = read_pane_text(connector, pane_id, "recent")?;
    let matches = crate::find::find_matches(&buffer.text, &request.term, &request.options)?;
    let total = matches.len();
    if total == 0 {
        return Ok(PaneFindOutcome {
            term: request.term.clone(),
            total: 0,
            index: 0,
            truncated: buffer.truncated,
            scroll: None,
        });
    }

    // Stepping wraps, because a search that stops at the end of the buffer
    // makes the reader guess whether there is more or they have gone round.
    let count = total as i64;
    let current = request.current_index as i64;
    let next = (current - 1 + request.step).rem_euclid(count);
    let target = matches[next as usize];

    let visible = read_pane_text(connector, pane_id, "visible")?;
    let viewport_rows = visible.text.lines().count();
    let scroll = crate::find::viewport_anchor(&buffer.text, &visible.text)
        .map(|top| crate::find::scroll_delta(target.line, top, viewport_rows))
        .filter(|delta| *delta != 0)
        .and_then(|delta| {
            let lines = u16::try_from(delta.unsigned_abs()).ok()?;
            Some((if delta > 0 { "up" } else { "down" }.to_owned(), lines))
        });

    Ok(PaneFindOutcome {
        term: request.term.clone(),
        total,
        index: next as usize + 1,
        truncated: buffer.truncated,
        scroll,
    })
}

pub(crate) struct PaneText {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

fn read_pane_text(
    connector: &dyn ApiConnector,
    pane_id: &str,
    source: &str,
) -> Result<PaneText, String> {
    let response = control_request(
        connector,
        "pane.read",
        wire::pane_read_params(pane_id, source, PANE_FIND_LINE_LIMIT)?,
    )?;
    wire::pane_text(response)
}

/// Topology reads and opening an SSH stream can wait. Recheck the same
/// control, file node and admitted intent on both sides of every connection.
/// The callback checks current runtime facts and, for a saved group start,
/// its owning node's terminal incarnation outside Runtime's mutex.
pub(crate) struct CurrentSessionConnector<'a> {
    pub(crate) connector: &'a dyn ApiConnector,
    pub(crate) current: &'a (dyn Fn() -> Result<(), String> + Sync),
}

impl CurrentSessionConnector<'_> {
    fn check(&self) -> Result<(), ApiError> {
        (self.current)().map_err(|code| ApiError::Remote {
            code,
            message: "The session execution changed; refresh before retrying".into(),
        })
    }
}

impl ApiConnector for CurrentSessionConnector<'_> {
    fn connect(&self) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        self.check()?;
        let stream = self.connector.connect()?;
        self.check()?;
        Ok(stream)
    }
}

pub fn spawn_agent_fork(context: LiveContext, request: ForkRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-agent-fork".to_owned())
        .spawn(move || {
            // Includes pane creation and the start acknowledgment, so this
            // input wait never extends Herdr's 120-second start reservation.
            let input_deadline =
                Instant::now() + crate::agent_start::AGENT_START_DEADLINE - Duration::from_secs(5);
            finish_fork_worker(&context, &request.parent_pane_id, || {
                let pane = run_agent_fork_with_registration(
                    context.api_connector.as_ref(),
                    context.node.as_ref(),
                    &request,
                    &|| fork_execution_current(&context, &request),
                    |tab_id, pane_id| {
                        let Some(runtime) = context.runtime.upgrade() else {
                            return;
                        };
                        let changed = match runtime.lock() {
                            Ok(mut guard) => {
                                guard.ingest_fork_pane(&request.parent_pane_id, tab_id, pane_id)
                            }
                            Err(_) => return,
                        };
                        drop(runtime);
                        if changed {
                            context.notifier.notify();
                        }
                    },
                    |parent, child| {
                        fork_execution_current(&context, &request)?;
                        crate::coordination::link_fork(&context, parent, child, &request.name)
                    },
                )?;
                // The fork already succeeded. Readiness owns only its held
                // input and must never close a slow but working pane.
                wait_for_fork_input(&context, &request, &pane, input_deadline);
                Ok(pane)
            });
        })
        .map(|_| ())
        .map_err(|error| format!("fork worker could not be started: {error}"))
}

fn wait_for_fork_input(
    context: &LiveContext,
    request: &ForkRequest,
    pane: &str,
    deadline: Instant,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::agent_start::confirm_fork_started(
            context.api_connector.as_ref(),
            pane,
            &request.name,
            request.agent.kind(),
            &|| fork_input_owner_alive(&context.runtime),
            deadline,
        )
    }))
    .unwrap_or_else(|_| Err("fork input confirmation panicked".into()));
    if let Err(reason) = outcome {
        crate::diagnostic!(serde_json::json!({
            "component": "terminal_input", "kind": "terminal.fork_input_unconfirmed",
            "pane_id": pane, "reason": reason,
        }));
        if let Some(runtime) = context.runtime.upgrade() {
            runtime
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .discard_fork_input(&request.parent_pane_id);
            context.notifier.notify();
        }
    }
}

fn fork_input_owner_alive(owner: &Weak<Mutex<Runtime>>) -> Result<(), String> {
    if owner.strong_count() == 0 {
        Err("The fork's daemon stopped.".into())
    } else {
        Ok(())
    }
}

/// Every worker exit settles its input holder, including an unwinding provider
/// or connector. Runtime teardown settles it if the weak owner has ended.
pub(crate) fn finish_fork_worker(
    context: &LiveContext,
    parent_pane_id: &str,
    work: impl FnOnce() -> Result<String, String>,
) {
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err("fork worker panicked".into()));
    let Some(runtime) = context.runtime.upgrade() else {
        return;
    };
    let changed = runtime
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .ingest_fork_result(parent_pane_id, result, started.elapsed().as_millis());
    if changed {
        context.notifier.notify();
    }
}

fn fork_execution_current(context: &LiveContext, request: &ForkRequest) -> Result<(), String> {
    let runtime = context.runtime.upgrade().ok_or("runtime stopped")?;
    if !hide_session::Agent::from_kind(request.agent.kind())
        .is_some_and(hide_session::Agent::requires_native_proof)
    {
        return Ok(());
    }
    let runtime = runtime.lock().map_err(|_| "runtime unavailable")?;
    if !runtime.fork_request_is_current(request, &context.api_connector)
        || !Arc::ptr_eq(&runtime.own_node(), &context.node)
    {
        return Err("session_fork_execution_changed".into());
    }
    Ok(())
}

/// Starts a fork where the operator asked for it, in its own tab or beside
/// the source. Herdr makes the pane without moving the keyboard; `created`
/// hands it to the core, which moves the keyboard to it and places its tab.
/// A pane whose agent never started is closed; registration never turns a
/// working fork into a failure.
fn run_agent_fork_with_registration(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
    current: &(dyn Fn() -> Result<(), String> + Sync),
    created: impl FnOnce(Option<&str>, &str),
    register: impl FnOnce(&str, &str) -> Result<(), String>,
) -> Result<String, String> {
    let checked = CurrentSessionConnector { connector, current };
    let proven = hide_session::Agent::from_kind(request.agent.kind())
        .is_some_and(hide_session::Agent::requires_native_proof);
    let mutation_connector: &dyn ApiConnector = if proven { &checked } else { connector };
    let check = || {
        current()?;
        confirm_session_launch(
            node,
            request.agent.kind(),
            &request.session_id,
            request.cwd.as_deref(),
            request.source_reference.as_ref(),
        )?;
        current()
    };
    check()?;
    // The arguments are settled before any pane exists, so a choice the agent
    // does not take never leaves a pane to close. The choice is checked before
    // the fork is made, so a refused one never leaves a session an OpenCode 2
    // server made either.
    let choice =
        crate::runtime::agent_choice::launch_arguments(request.agent.kind(), &request.launch)?;
    let (mut args, made) = fork_arguments(node, request)?;
    args.extend(choice);
    // Everything after an OpenCode 2 fork can still fail (the parent's tab,
    // the new tab, the agent's start, or the parent closed meanwhile), and the
    // session OpenCode's server made stays in OpenCode's list: it is named in
    // the reason and a diagnostic for the operator to find, never deleted.
    let mut made_pane = None;
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        // A split takes no command, so only a fork in its own tab runs the
        // integration's launch.
        let launch = matches!(request.placement, ForkPlacement::Tab)
            .then(|| {
                crate::agent_start::shell_launch(
                    Some(node),
                    &format!("herdr-core:fork:{}:start", request.name),
                )
            })
            .flatten();
        let shell = crate::agent_start::ShellStart::of_tab(true, launch.as_ref());
        let child_pane_id = match request.placement {
            // `Fork agent`: the fork's own tab, which the core puts right of the
            // source's tab once Herdr has named it (`created`).
            ForkPlacement::Tab => {
                let cwd = request
                    .cwd
                    .as_deref()
                    .filter(|cwd| !cwd.trim().is_empty())
                    .ok_or("Session fork has no confirmed checkout")?;
                let snapshot = fetch_session_with_connector(mutation_connector)
                    .map_err(|error| error.message().to_owned())?;
                let mut parents = snapshot.layouts.iter().filter(|layout| {
                    layout
                        .panes
                        .iter()
                        .any(|pane| pane.pane_id == request.parent_pane_id)
                });
                let parent = parents
                    .next()
                    .ok_or("Session fork parent is no longer in its workspace")?;
                if parents.next().is_some() {
                    return Err("Session fork parent has ambiguous workspace ownership".into());
                }
                let label = request
                    .tab_label
                    .as_deref()
                    .ok_or("Session fork has no checkout to name its tab in")?;
                check()?;
                let tab = create_tab_in(
                    mutation_connector,
                    &parent.workspace_id,
                    cwd,
                    label,
                    false,
                    launch
                        .as_ref()
                        .map(|launch| launch.env.clone())
                        .unwrap_or_default(),
                    launch.as_ref().map(|launch| launch.command.as_slice()),
                )
                .map_err(|error| error.message().to_owned())?;
                made_pane = Some(tab.pane_id.clone());
                created(Some(&tab.tab_id), &tab.pane_id);
                tab.pane_id
            }
            // `Fork agent to the side`: a split right of the source, in its tab.
            ForkPlacement::Side => {
                let pane_id = control_request(
                    mutation_connector,
                    "pane.split",
                    wire::fork_split_params(
                        &request.parent_pane_id,
                        request.cwd.as_deref(),
                        Default::default(),
                    )?,
                )
                .and_then(wire::split_pane)?;
                made_pane = Some(pane_id.clone());
                created(None, &pane_id);
                pane_id
            }
        };
        let started = start_agent_checked(
            mutation_connector,
            Some(node),
            &format!("herdr-core:fork:{}:start", request.name),
            &child_pane_id,
            shell,
            &request.name,
            request.agent.kind(),
            args,
            request.codex_daemon,
            &check,
        )
        .map_err(start_error_message);
        let started = started.and_then(|pane_id| {
            if request.agent == crate::fork::ForkableAgent::Antigravity {
                fork_antigravity(mutation_connector, node, request, &pane_id)?;
            }
            Ok(pane_id)
        });
        match started {
            Ok(pane_id) => {
                if let Err(message) = register(&request.parent_pane_id, &pane_id) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "fork",
                        "kind": "registration_failed",
                        "parent_pane_id": request.parent_pane_id,
                        "child_pane_id": pane_id,
                        "message": message,
                    }));
                }
                Ok(pane_id)
            }
            Err(error) => {
                // This child is already ours on the original server. Cleanup is
                // not a new fork effect and must not target its replacement.
                let closed = control_request(
                    connector,
                    "pane.close",
                    wire::pane_target_params(&child_pane_id)?,
                );
                Err(match closed {
                    Ok(_) => error,
                    Err(close_error) => format!(
                        "{error}; the created pane {child_pane_id} could not be closed: {close_error}"
                    ),
                })
            }
        }
    })).unwrap_or_else(|_| {
        if let Some(pane) = made_pane {
            // Closing can fail too, but must never prevent settling the hold.
            let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                control_request(connector, "pane.close", wire::pane_target_params(&pane)?)
            }));
            if !matches!(closed, Ok(Ok(_))) {
                crate::diagnostic!(json!({"component": "pane_fork", "kind": "pane.fork.close_failed", "pane_id": pane}));
            }
        }
        Err("fork worker panicked".into())
    });
    opened.map_err(|error| match made {
        Some(session) => opencode_fork_left(request, &session, error),
        None => error,
    })
}

/// The reason a fork whose OpenCode 2 session was made did not open, naming
/// that session, and its diagnostic.
fn opencode_fork_left(request: &ForkRequest, session: &str, error: String) -> String {
    crate::diagnostic!(serde_json::json!({
        "component": "fork",
        "kind": "opencode_fork_left",
        "parent_pane_id": request.parent_pane_id,
        "session_id": session,
        "message": error,
    }));
    format!(
        "{error}; OpenCode made the fork's session {session}, which stays in OpenCode's session list"
    )
}

/// The agent's arguments that open the fork. Every agent but OpenCode spells
/// its own fork ([`crate::fork::ForkableAgent::resume_arguments`]). OpenCode's
/// depends on the version holding the session, which its node reads from the
/// database (`hide_session::opencode::fork_route`): 1.x forks with its own
/// flag, and 2.x's server makes the fork here, once, before the fork's pane
/// exists, so every later step of this fork, a retried start included, opens
/// that one session; the session it made comes back beside the arguments. A
/// node that cannot fork it ends the fork with its reason before any pane is
/// made, and one whose answer never came says a fork may exist.
fn fork_arguments(
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
) -> Result<(Vec<String>, Option<String>), String> {
    if request.agent != crate::fork::ForkableAgent::OpenCode {
        return Ok((request.agent.resume_arguments(&request.session_id), None));
    }
    let cwd = request
        .cwd
        .as_deref()
        .ok_or("Session fork has no confirmed checkout")?;
    // Busy answers only a route read that met OpenCode writing its
    // database, before the node forked anything, so it is asked again.
    let fork: hide_node_link::protocol::OpenCodeFork = read_past_writes(
        node,
        || hide_node_link::protocol::Call::OpenCodeFork {
            source: request.session_id.clone(),
            cwd: cwd.to_owned(),
        },
        hide_node_link::protocol::OPENCODE_FORK_TIMEOUT,
        Instant::now() + Duration::from_secs(10),
    )
    .map_err(|error| match error {
        crate::node_access::LinkError::Unknown(_) => format!(
            "OpenCode could not confirm the fork: {error}; a fork may exist in OpenCode's session list"
        ),
        error => format!("OpenCode could not fork the session: {error}"),
    })?;
    match fork {
        hide_node_link::protocol::OpenCodeFork::Flag => {
            Ok((request.agent.resume_arguments(&request.session_id), None))
        }
        hide_node_link::protocol::OpenCodeFork::Forked { id } => {
            Ok((crate::fork::opencode_forked_arguments(&id)?, Some(id)))
        }
    }
}

/// How long agy gets to come up in the fork's pane, and then to write the
/// conversation its `/fork` made.
const ANTIGRAVITY_FORK_WAIT: Duration = Duration::from_secs(30);
const ANTIGRAVITY_FORK_POLL: Duration = Duration::from_millis(250);
/// How many reads of an agent's records in a row may meet the agent writing
/// them before the reader gives up: 5 s of polls, far past one write.
const NATIVE_READ_IN_MOTION_LIMIT: usize = 20;
const NATIVE_READ_IN_MOTION_POLL: Duration = Duration::from_millis(250);

/// One read of an agent's native records on a node, read again while it
/// only met the agent writing them, which the node answers as busy
/// (`hide_session::ReadRefusal::InMotion`). A read that answers is one
/// consistent snapshot, so it needs no second read to agree with; the wait
/// ends at `deadline` or after [`NATIVE_READ_IN_MOTION_LIMIT`] reads in a
/// row, whichever is first, and answers the last read's busy refusal.
#[allow(clippy::disallowed_methods)] // a production wait, not test code: bounded by count and deadline
fn read_past_writes<T: serde::de::DeserializeOwned>(
    node: &dyn crate::node_access::NodeLink,
    call: impl Fn() -> hide_node_link::protocol::Call,
    timeout: Duration,
    deadline: Instant,
) -> Result<T, crate::node_access::LinkError> {
    let mut in_motion = 0;
    loop {
        match crate::node_access::call_as(node, call(), timeout) {
            Err(crate::node_access::LinkError::Refused(error))
                if error.code == hide_node_link::ErrorCode::Busy
                    && in_motion + 1 < NATIVE_READ_IN_MOTION_LIMIT
                    && Instant::now() < deadline =>
            {
                in_motion += 1;
                thread::sleep(NATIVE_READ_IN_MOTION_POLL);
            }
            answer => return answer,
        }
    }
}

/// agy forks only through its `/fork` command (agy 1.4.0, PRD
/// antigravity-first-class D-07). The fork's pane resumed the source
/// conversation; this types `/fork` once Herdr reads agy as idle there, finds
/// the conversation `/fork` made in agy's index on the pane's node, and
/// reports it to Herdr, which would keep the source's id until the next
/// prompt. Everything the fork assumes of agy's screen is in this function;
/// a failure leaves the source conversation as it was and the caller closes
/// the pane.
#[allow(clippy::disallowed_methods)] // a production wait, not test code: polls Herdr and agy's index, bounded by a deadline
fn fork_antigravity(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    request: &ForkRequest,
    pane_id: &str,
) -> Result<(), String> {
    let cwd = request
        .cwd
        .as_deref()
        .ok_or("Session fork has no confirmed checkout")?;
    let deadline = Instant::now() + ANTIGRAVITY_FORK_WAIT;
    while control_request(connector, "pane.get", wire::pane_target_params(pane_id)?)
        .and_then(wire::pane_agent_status)?
        != "idle"
    {
        if Instant::now() >= deadline {
            return Err("agy did not open the conversation to fork in time".into());
        }
        thread::sleep(ANTIGRAVITY_FORK_POLL);
    }
    // What the checkout holds before /fork, read on the pane's node, so the
    // fork is the row that was not there: no clock is compared, and the
    // source may move on while agy writes it.
    let mark: hide_node_link::protocol::AntigravityForkMark = read_past_writes(
        node,
        || hide_node_link::protocol::Call::AntigravityForkMark {
            source: request.session_id.clone(),
            cwd: cwd.to_owned(),
        },
        Duration::from_secs(5),
        Instant::now() + ANTIGRAVITY_FORK_WAIT,
    )
    .map_err(|error| format!("agy's conversation to fork could not be read: {error}"))?;
    // The mark's read may have waited on agy writing its index: `/fork` is
    // typed only into a pane still idle, never into a turn that began since.
    if control_request(connector, "pane.get", wire::pane_target_params(pane_id)?)
        .and_then(wire::pane_agent_status)?
        != "idle"
    {
        return Err("agy started working before /fork could be typed".into());
    }
    control_request(
        connector,
        "pane.send_input",
        wire::delivery_input_params(pane_id, "/fork")?,
    )?;
    let deadline = Instant::now() + ANTIGRAVITY_FORK_WAIT;
    let forked = loop {
        let found: Option<String> = read_past_writes(
            node,
            || hide_node_link::protocol::Call::AntigravityFork {
                source: request.session_id.clone(),
                cwd: cwd.to_owned(),
                mark: mark.clone(),
            },
            Duration::from_secs(5),
            deadline,
        )
        .map_err(|error| format!("agy's fork could not be confirmed: {error}"))?;
        if let Some(id) = found {
            break id;
        }
        if Instant::now() >= deadline {
            return Err("agy's /fork made no new conversation".into());
        }
        thread::sleep(ANTIGRAVITY_FORK_POLL);
    };
    let seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        });
    control_request(
        connector,
        "pane.report_agent_session",
        wire::report_agent_session_params(pane_id, request.agent.kind(), &forked, seq)?,
    )
    .map(drop)
}

pub fn spawn_remote_control(
    context: RemoteControlContext,
    request_id: String,
    action: RemoteControlAction,
    connection_generation: u64,
) -> Result<(), String> {
    spawn_remote_control_checked(
        context,
        request_id,
        action,
        connection_generation,
        Vec::new(),
        CloseSafety {
            approved_peers: Vec::new(),
            pane_ids: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(5),
        },
    )
}

pub(crate) fn spawn_remote_control_checked(
    context: RemoteControlContext,
    request_id: String,
    action: RemoteControlAction,
    connection_generation: u64,
    failed_terminals: Vec<(String, String)>,
    safety: CloseSafety,
) -> Result<(), String> {
    let target_id = context.target_id.clone();
    let worker_name = match &action {
        RemoteControlAction::Pane(PaneControlAction::Focus { .. }) => {
            format!("herdr-core-remote-{target_id}-pane-focus")
        }
        RemoteControlAction::Pane(PaneControlAction::Split { direction, .. }) => format!(
            "herdr-core-remote-{target_id}-pane-split-{}",
            direction.as_str()
        ),
        RemoteControlAction::Pane(PaneControlAction::ToggleZoom { .. }) => {
            format!("herdr-core-remote-{target_id}-pane-zoom")
        }
        RemoteControlAction::Pane(PaneControlAction::Close { .. }) => {
            format!("herdr-core-remote-{target_id}-pane-close")
        }
        RemoteControlAction::Pane(PaneControlAction::MoveToNewTab { .. }) => {
            format!("herdr-core-remote-{target_id}-pane-move")
        }
        RemoteControlAction::Pane(
            PaneControlAction::Project { .. } | PaneControlAction::Resize { .. },
        ) => return Err("unsupported remote pane control action".to_owned()),
        RemoteControlAction::FocusWorkspace { .. } => {
            format!("herdr-core-remote-{target_id}-workspace-focus")
        }
        RemoteControlAction::FocusTab { .. } => {
            format!("herdr-core-remote-{target_id}-tab-focus")
        }
        RemoteControlAction::CreateTab { .. } => {
            format!("herdr-core-remote-{target_id}-tab-create")
        }
        RemoteControlAction::OpenOwner { .. } => {
            format!("herdr-core-remote-{target_id}-owner-open")
        }
        RemoteControlAction::CloseTab { .. } => {
            format!("herdr-core-remote-{target_id}-tab-close")
        }
        RemoteControlAction::RenameTab { .. } => {
            format!("herdr-core-remote-{target_id}-tab-rename")
        }
        RemoteControlAction::MoveTab { .. } => {
            format!("herdr-core-remote-{target_id}-tab-move")
        }
    };
    thread::Builder::new()
        .name(worker_name)
        .spawn(move || {
            let started = Instant::now();
            let target = match &action {
                RemoteControlAction::Pane(PaneControlAction::Close { pane_id }) => {
                    Some(CloseCaptureTarget::Pane {
                        pane_id: pane_id.clone(),
                    })
                }
                RemoteControlAction::CloseTab { tab_id } => Some(CloseCaptureTarget::Tab {
                    tab_id: tab_id.clone(),
                }),
                _ => None,
            };
            let result = if !failed_terminals.is_empty()
                && let Some(target) = target
            {
                run_close_effect(
                    context.api_connector.as_ref(),
                    &CloseEffectRequest {
                        safety: Some(safety),
                        failed_terminals,
                        target,
                        retain_for_reopen: false,
                        allow_replacement_create: false,
                        replacement: None,
                        key: request_id.clone(),
                        connection_generation,
                    },
                )
                .map(|()| RemoteControlOutcome::Acknowledged {
                    created_tab_id: None,
                    created_pane_id: None,
                })
                .map_err(|error| match error {
                    ApiError::Remote { .. } => ControlFailure::Definite(error.to_string()),
                    _ => ControlFailure::Ambiguous(error.to_string()),
                })
            } else {
                execute_remote_control(context.api_connector.as_ref(), &action)
            };
            let elapsed_ms = started.elapsed().as_millis();
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_remote_control_failure_with_generation(
                    &target_id,
                    &request_id,
                    action,
                    result,
                    elapsed_ms,
                    Some(connection_generation),
                ),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("remote control worker could not be started: {error}"))
}

// A local create is tagged once. A lost reply gets one bounded read-only
// recovery, never a repeated mutation; a still-unknown result retains its claim.
fn local_create_key(id: u64) -> String {
    format!("create-{}-{id}", std::process::id())
}
fn local_create_env(id: Option<u64>) -> std::collections::BTreeMap<String, String> {
    id.map(|id| reopen_intent_env(&local_create_key(id), ReopenIntentStage::LocalCreate))
        .unwrap_or_default()
}
fn execute_local_control(
    connector: &dyn ApiConnector,
    action: &RemoteControlAction,
) -> Result<RemoteControlOutcome, ControlFailure> {
    let result = execute_remote_control(connector, action);
    if !result.as_ref().is_err_and(ControlFailure::is_ambiguous) {
        return result;
    }
    match recover_local_creation(connector, action) {
        Ok(outcome) => Ok(outcome),
        Err(message) => {
            crate::diagnostic!(
                json!({"component":"tab_control", "kind":"tab.create.recovery_unknown", "message":message})
            );
            result
        }
    }
}
fn recover_local_creation(
    connector: &dyn ApiConnector,
    action: &RemoteControlAction,
) -> Result<RemoteControlOutcome, String> {
    let (id, workspace) = match action {
        RemoteControlAction::CreateTab {
            admission_id: Some(id),
            workspace_id,
            ..
        } => (*id, Some(workspace_id.as_str())),
        RemoteControlAction::OpenOwner {
            admission_id: Some(id),
            ..
        } => (*id, None),
        _ => return Err("No local creation claim to recover".into()),
    };
    let started = Instant::now();
    let snapshot =
        fetch_session_with_connector(connector).map_err(|error| error.message().to_owned())?;
    let tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| workspace.is_none_or(|id| tab.workspace_id == id))
        .collect::<Vec<_>>();
    if tabs.len() > 64 {
        return Err("Too many tabs for bounded creation recovery".into());
    }
    let key = local_create_key(id);
    let marker = reopen_intent_marker(&key, ReopenIntentStage::LocalCreate);
    let mut found = None;
    for tab in tabs {
        if started.elapsed() > Duration::from_secs(5) {
            return Err("Creation recovery timed out".into());
        }
        let layout = export_reopen_layout(connector, &key, &tab.tab_id)?;
        let mut panes = Vec::new();
        pane_ids_with_reopen_marker(&layout.root, &marker, &mut panes);
        if let Some(pane) = panes.first() {
            if found.is_some() {
                return Err("Multiple tabs carry the creation claim".into());
            }
            found = Some(RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(tab.tab_id.clone()),
                created_pane_id: Some(pane.clone()),
            });
        }
    }
    found.ok_or_else(|| "Creation is not yet confirmed".into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionFetchError {
    /// The socket file itself does not exist: the herdr server is not running.
    SocketMissing(String),
    /// The socket exists but the request failed (connect, timeout, IO).
    Unreachable(String),
    /// The server answered with an incompatible protocol revision.
    Protocol {
        message: String,
        expected_protocol: u64,
        received_protocol: u64,
        received_version: Option<String>,
    },
    /// A previously valid projection is retained while the event stream
    /// reconnects or performs an explicit snapshot resynchronization.
    Stale(String),
    /// The server answered but the payload did not match the expected shape.
    Malformed(String),
}

impl SessionFetchError {
    pub fn state(&self) -> &'static str {
        match self {
            Self::SocketMissing(_) => "socket_missing",
            Self::Unreachable(_) => "unreachable",
            Self::Protocol { .. } => "protocol_mismatch",
            Self::Stale(_) => "stale",
            Self::Malformed(_) => "malformed",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::SocketMissing(message)
            | Self::Unreachable(message)
            | Self::Stale(message)
            | Self::Malformed(message) => message,
            Self::Protocol { message, .. } => message,
        }
    }

    /// The same failure as a device's row reads it: a device's Herdr on an
    /// older protocol gets the device's step rather than this machine's
    /// advice to stop its session and reopen Hide (PRD device-herdr-update
    /// D-15).
    pub(crate) fn for_device(self) -> Self {
        match self {
            Self::Protocol {
                expected_protocol,
                received_protocol,
                received_version,
                ..
            } if received_protocol < expected_protocol => Self::Protocol {
                message: crate::wire::device_protocol_behind(
                    received_protocol,
                    received_version.as_deref(),
                ),
                expected_protocol,
                received_protocol,
                received_version,
            },
            other => other,
        }
    }

    pub fn protocol_details(&self) -> Option<(u64, u64, Option<&str>)> {
        match self {
            Self::Protocol {
                expected_protocol,
                received_protocol,
                received_version,
                ..
            } => Some((
                *expected_protocol,
                *received_protocol,
                received_version.as_deref(),
            )),
            _ => None,
        }
    }
}

/// Installs live command context and starts event-driven session sync.
pub(crate) fn install(
    runtime: &Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    own_node: Arc<dyn crate::node_access::NodeLink>,
    own_herdr: Arc<dyn ApiConnector>,
    socket_path: &str,
    usage_paths: crate::usage::UsagePaths,
) -> Option<crate::session_sync::SessionSyncHandle> {
    let context = LiveContext {
        socket_path: PathBuf::from(socket_path),
        runtime: Arc::downgrade(runtime),
        notifier: notifier.clone(),
        api_connector: own_herdr,
        node: Arc::clone(&own_node),
    };
    if let Ok(mut guard) = runtime.lock() {
        guard.set_live(context.clone());
    }
    match crate::session_sync::spawn(
        crate::session_sync::SessionSyncContext::local(&context, own_node),
        Some(usage_paths),
    ) {
        Ok(handle) => {
            if let Ok(mut guard) = runtime.lock() {
                guard.set_republish_waker(handle.republish_waker());
            }
            Some(handle)
        }
        Err(message) => {
            crate::diagnostic!(json!({
                "component": "session_sync",
                "kind": "coordinator.spawn_failed",
                "message": message,
            }));
            let changed = runtime.lock().ok().is_some_and(|mut guard| {
                guard.ingest_session(Err(SessionFetchError::Unreachable(message)))
            });
            if changed {
                context.notifier.notify();
            }
            None
        }
    }
}

pub(crate) fn fetch_session_with_connector(
    connector: &dyn ApiConnector,
) -> Result<SessionSnapshotPayload, SessionFetchError> {
    let result = request_with_connector(
        connector,
        "session.snapshot",
        wire::empty_params(),
        Duration::from_secs(5),
    )
    .map_err(|error| match error {
        ApiError::NotRunning(message) => SessionFetchError::SocketMissing(message),
        error => SessionFetchError::Unreachable(error.to_string()),
    })?;
    wire::live_session_response(result)
}

/// Maps the herdr wire snapshot into the sidebar session payload. Tokens are
/// passed through verbatim so the unseen-vs-acknowledged state rules
/// (INV-herdr-unseen-token) stay owned by the sidebar projection.
pub fn project_session(snapshot: &Value) -> Result<SessionSnapshotPayload, SessionFetchError> {
    crate::session_sync::project_snapshot(snapshot)
}

pub fn project_layout_for_pane(
    payload: &SessionSnapshotPayload,
    pane_id: &str,
) -> Result<PaneLayoutSnapshot, String> {
    let layout = payload
        .layouts
        .iter()
        .find(|layout| layout.panes.iter().any(|pane| pane.pane_id == pane_id))
        .ok_or_else(|| format!("Herdr session has no layout containing pane {pane_id}"))?;
    project_layout(layout)
}

/// Every tab's layout in the session, in the order Herdr sent them, plus the
/// tabs whose layout could not be read.
///
/// One unreadable layout excludes only its own tab. Refusing the whole
/// session for it would empty every other tab's canvas over a fault in one,
/// which is the failure shape the caller is being given all of them to avoid.
pub fn project_layouts(
    payload: &SessionSnapshotPayload,
) -> (Vec<PaneLayoutSnapshot>, Vec<(String, String)>) {
    let mut layouts = Vec::with_capacity(payload.layouts.len());
    let mut rejected = Vec::new();
    for layout in &payload.layouts {
        match project_layout(layout) {
            Ok(projected) => layouts.push(projected),
            Err(reason) => rejected.push((layout.tab_id.clone(), reason)),
        }
    }
    (layouts, rejected)
}

pub(crate) fn project_layout(layout: &SessionLayoutPayload) -> Result<PaneLayoutSnapshot, String> {
    if layout.panes.is_empty() {
        return Err(format!("Herdr tab {} has no panes", layout.tab_id));
    }
    let root = project_layout_node(layout.area, &layout.panes, &layout.splits)?;
    Ok(PaneLayoutSnapshot {
        workspace_id: layout.workspace_id.clone(),
        tab_id: layout.tab_id.clone(),
        focused_pane_id: layout.focused_pane_id.clone(),
        zoomed: layout.zoomed,
        root,
    })
}

fn project_layout_node(
    area: SessionLayoutRect,
    panes: &[SessionLayoutPanePayload],
    splits: &[crate::sidebar::SessionLayoutSplitPayload],
) -> Result<PaneLayoutNodeSnapshot, String> {
    let matching_splits = splits
        .iter()
        .filter(|split| split.rect == area)
        .collect::<Vec<_>>();
    if matching_splits.len() > 1 {
        return Err("Herdr layout contains duplicate splits for one area".to_owned());
    }
    let Some(split) = matching_splits.first().copied() else {
        return match panes {
            [pane] => Ok(PaneLayoutNodeSnapshot::Pane {
                pane_id: pane.pane_id.clone(),
            }),
            [] => Err("Herdr layout produced an empty leaf".to_owned()),
            _ => Err("Herdr layout has multiple panes without an authoritative split".to_owned()),
        };
    };
    if !split.ratio.is_finite() || split.ratio <= 0.0 || split.ratio >= 1.0 {
        return Err(format!(
            "Herdr layout split ratio {} is invalid",
            split.ratio
        ));
    }
    let (first_area, second_area, boundary) = split_areas(area, split.direction, split.ratio)?;
    let (first_panes, second_panes): (Vec<_>, Vec<_>) = panes.iter().partition(|pane| {
        let center = match split.direction {
            PaneLayoutDirection::Right => f32::from(pane.rect.x) + f32::from(pane.rect.width) / 2.0,
            PaneLayoutDirection::Down => f32::from(pane.rect.y) + f32::from(pane.rect.height) / 2.0,
        };
        center < boundary
    });
    if first_panes.is_empty() || second_panes.is_empty() {
        return Err("Herdr layout split does not divide panes into two children".to_owned());
    }
    let first = project_layout_node(
        first_area,
        &first_panes.into_iter().cloned().collect::<Vec<_>>(),
        splits,
    )?;
    let second = project_layout_node(
        second_area,
        &second_panes.into_iter().cloned().collect::<Vec<_>>(),
        splits,
    )?;
    Ok(PaneLayoutNodeSnapshot::Split {
        direction: split.direction,
        ratio: split.ratio,
        first: Box::new(first),
        second: Box::new(second),
    })
}

pub(crate) fn split_areas(
    area: SessionLayoutRect,
    direction: PaneLayoutDirection,
    ratio: f32,
) -> Result<(SessionLayoutRect, SessionLayoutRect, f32), String> {
    match direction {
        PaneLayoutDirection::Right => {
            if area.width < 2 {
                return Err("Herdr right split area is too narrow".to_owned());
            }
            let first_width =
                ((f32::from(area.width) * ratio).round() as u16).clamp(1, area.width - 1);
            let second_width = area.width - first_width;
            let second_x = area.x.saturating_add(first_width);
            Ok((
                SessionLayoutRect {
                    width: first_width,
                    ..area
                },
                SessionLayoutRect {
                    x: second_x,
                    width: second_width,
                    ..area
                },
                f32::from(second_x),
            ))
        }
        PaneLayoutDirection::Down => {
            if area.height < 2 {
                return Err("Herdr down split area is too short".to_owned());
            }
            let first_height =
                ((f32::from(area.height) * ratio).round() as u16).clamp(1, area.height - 1);
            let second_height = area.height - first_height;
            let second_y = area.y.saturating_add(first_height);
            Ok((
                SessionLayoutRect {
                    height: first_height,
                    ..area
                },
                SessionLayoutRect {
                    y: second_y,
                    height: second_height,
                    ..area
                },
                f32::from(second_y),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::path::Path;

    use hide_herdr_client::LocalSocketConnector;

    use super::*;
    use crate::fake_herdr::FakeHerdr;

    /// A restored agent resumes with the model and effort it was started
    /// with, and the new conversation that replaces a refused resume keeps
    /// them too.
    #[test]
    fn a_restored_agent_starts_with_its_launch_choice_resumed_or_new() {
        let starts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("reopen-choice", move |method, params| match method {
                "session.snapshot" => Ok(json!({"type": "session_snapshot", "snapshot": {
                    "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
                }})),
                "pane.process_info" => Ok(json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "pane", "shell_pid": 42, "foreground_process_group_id": 42,
                    "foreground_processes": []
                }})),
                "agent.start" => {
                    let mut seen = seen.lock().unwrap();
                    seen.push(params["args"].clone());
                    if seen.len() == 1 {
                        return Err(("agent_start_failed".into(), "resume refused".into()));
                    }
                    Ok(json!({"type": "agent_started", "argv": [], "agent": {
                        "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1",
                        "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1
                    }}))
                }
                other => panic!("unexpected {other}"),
            });
        let launch = hide_agent_adapter::LaunchChoice {
            model: Some("gpt-6-luna".into()),
            effort: Some("low".into()),
        };
        let mut notices = Vec::new();
        let started = start_or_degrade_agent(
            &herdr.connector(),
            "key",
            0,
            "pane",
            &ClosedAgent {
                kind: "codex".into(),
                session_id: Some("session".into()),
                launch: launch.clone(),
            },
            crate::codex_launch::CodexDaemon::Unsupported,
            &mut notices,
        );
        let choice = ["--model", "gpt-6-luna", "-c", "model_reasoning_effort=low"];
        assert_eq!(
            *starts.lock().unwrap(),
            [
                json!([["resume", "session"].as_slice(), &choice].concat()),
                json!(choice)
            ]
        );
        assert_eq!(
            started,
            Some(ReopenedAgent {
                pane_id: "pane".into(),
                kind: "codex".into(),
                name: "reopen-key-0".into(),
                launch,
                session_id: None,
            })
        );
        assert_eq!(
            notices.len(),
            1,
            "the lost conversation is said: {notices:?}"
        );
    }

    #[test]
    fn reopen_canonicalizes_alias_starts_and_confirms_only_the_same_pane_agent() {
        for (kind, canonical, args) in [
            (" CLAUDE-CODE ", "claude", vec!["--resume", "session"]),
            (" CLAUDE ", "claude", vec!["--resume", "session"]),
            (" CLAUDE_CODE ", "claude", vec!["--resume", "session"]),
            (" CODEX ", "codex", vec!["resume", "session"]),
        ] {
            let herdr = FakeHerdr::start("reopen-alias", move |method, params| match method {
                "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                    "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
                }}),
                "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                    "pane_id": "pane", "shell_pid": 42, "foreground_process_group_id": 42,
                    "foreground_processes": []
                }}),
                "agent.start" => {
                    assert_eq!(params["kind"], canonical);
                    assert_eq!(params["args"], json!(args));
                    json!({"type": "agent_started", "argv": [], "agent": {
                        "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1",
                        "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1
                    }})
                }
                other => panic!("unexpected {other}"),
            });
            let mut notices = Vec::new();
            start_or_degrade_agent(
                &herdr.connector(),
                "key",
                0,
                "pane",
                &ClosedAgent {
                    kind: kind.into(),
                    session_id: Some("session".into()),
                    ..Default::default()
                },
                crate::codex_launch::CodexDaemon::Unsupported,
                &mut notices,
            );
            assert!(notices.is_empty(), "{kind}: {notices:?}");
            assert_eq!(
                herdr.methods(),
                ["session.snapshot", "pane.process_info", "agent.start"]
            );
        }
        let herdr = FakeHerdr::start("reopen-alias-confirm", |method, _| match method {
            "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": [{
                    "pane_id": "pane", "terminal_id": "term", "workspace_id": "w1", "tab_id": "w1:t1",
                    "name": "agent", "agent": "claude", "agent_status": "idle", "focused": false, "revision": 1
                }]
            }}),
            other => panic!("unexpected {other}"),
        });
        assert!(agent_is_running(
            &herdr.connector(),
            "pane",
            " CLAUDE_CODE "
        ));
        assert!(!agent_is_running(
            &herdr.connector(),
            "other-pane",
            "claude_code"
        ));
        assert!(!agent_is_running(&herdr.connector(), "pane", "codex"));
        assert!(!agent_is_running(
            &herdr.connector(),
            "pane",
            "future-agent"
        ));
    }

    #[test]
    fn a_native_fork_gets_its_own_unfocused_tab_and_failed_start_closes_only_its_pane() {
        for agent in [
            crate::fork::ForkableAgent::Pi,
            crate::fork::ForkableAgent::Omp,
        ] {
            native_fork_tab_journey(agent);
        }
    }

    fn native_fork_tab_journey(agent: crate::fork::ForkableAgent) {
        let kind = agent.kind();
        for case in [
            "success",
            "refused",
            "panic-created",
            "duplicate-before",
            "duplicate-after",
            "execution-after",
            "source-missing-before",
            "source-missing-after",
        ] {
            let refused = case == "refused";
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let folder = crate::fixture::native_session_folder(home.path(), kind, &cwd);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join("native.jsonl"),
                format!(
                    "{}\n",
                    json!({"type":"session", "version":3, "id":"native-pi", "cwd":cwd})
                ),
            )
            .unwrap();
            let source = folder.join("native.jsonl");
            let before = std::fs::read(&source).unwrap();
            let other_before = [before.as_slice(), b"{\"type\":\"message\",\"id\":\"other-message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"different history\"}]}}\n"].concat();
            if case == "duplicate-before" {
                std::fs::copy(&source, folder.join("duplicate.jsonl")).unwrap();
            }
            if case == "source-missing-before" {
                std::fs::write(folder.join("other.jsonl"), &other_before).unwrap();
                std::fs::remove_file(&source).unwrap();
            }
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let expected_cwd = cwd.display().to_string();
            let tab_source = source.clone();
            let tab_other = other_before.clone();
            let herdr = FakeHerdr::start_with_errors("pi-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "source-missing-after" {
                            std::fs::write(tab_source.with_file_name("other.jsonl"), &tab_other)
                                .unwrap();
                            std::fs::remove_file(&tab_source).unwrap();
                        }
                        if case == "duplicate-after" {
                            std::fs::copy(
                                &tab_source,
                                tab_source.with_file_name("duplicate.jsonl"),
                            )
                            .unwrap();
                        }
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["workspace_id"], "w1");
                        assert_eq!(params["root"]["cwd"], expected_cwd);
                        assert_eq!(params["focus"], false);
                        // Named as a new tab, so the strip shows the agent;
                        // it runs the node's shell with the integration's
                        // launch.
                        assert_eq!(params["tab_label"], "Tab 2");
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        assert_eq!(params["kind"], kind);
                        assert_eq!(params["args"], json!(["--fork", "native-pi"]));
                        if refused {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"ok"})
                    }
                    other => panic!("unexpected {other}"),
                })
            });
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "path".into(),
                    value: source.display().to_string(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent,
                session_id: "native-pi".into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-pi-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let created = std::cell::RefCell::new(Vec::new());
            let result = run_agent_fork_with_registration(
                &herdr.connector(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |tab, pane| {
                    created
                        .borrow_mut()
                        .push((tab.map(str::to_owned), pane.to_owned()));
                    assert_ne!(case, "panic-created", "fixture callback panicked");
                },
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}");
            // The core hears of the fork's tab once, as soon as Herdr made it,
            // whatever happens to its start afterwards.
            let made_tab = herdr
                .methods()
                .iter()
                .any(|method| method == "layout.apply");
            assert_eq!(
                created.into_inner(),
                if made_tab {
                    vec![(Some("w1:t2".to_owned()), "child-pane".to_owned())]
                } else {
                    Vec::new()
                },
                "{case}"
            );
            assert_eq!(
                std::fs::read(if source.exists() {
                    source.clone()
                } else {
                    folder.join("other.jsonl")
                })
                .unwrap(),
                if case.starts_with("source-missing") {
                    other_before
                } else {
                    before
                },
                "fork never rewrites either conversation"
            );
            if case == "success" {
                assert_eq!(result.unwrap(), "child-pane");
            }
            let mut expected = vec![
                "session.snapshot",
                "layout.apply",
                "pane.process_info",
                "pane.get",
                "agent.start",
            ];
            if case == "duplicate-before" || case == "source-missing-before" {
                expected.clear();
            } else if case == "duplicate-after"
                || case == "execution-after"
                || case == "source-missing-after"
            {
                expected.pop();
                if case == "execution-after" {
                    // Retired execution is now refused before even the
                    // shell-read connection; the owned child still closes.
                    expected.pop();
                    expected.pop();
                }
                expected.push("pane.close");
            } else if case == "panic-created" {
                expected = vec!["session.snapshot", "layout.apply", "pane.close"];
            } else if refused {
                expected.push("pane.close");
            }
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    #[test]
    fn an_opencode_fork_proves_its_root_session_and_checkout_before_and_after_its_tab() {
        for case in [
            "success",
            "refused",
            "other-checkout",
            "child-session",
            "removed-after",
            "execution-after",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let checkout = cwd.display().to_string();
            let directory = if case == "other-checkout" {
                "/elsewhere/app".to_owned()
            } else {
                checkout.clone()
            };
            crate::fixture::opencode_database(
                home.path(),
                &[
                    ("ses_root", None, directory.as_str()),
                    ("ses_child", Some("ses_root"), directory.as_str()),
                ],
            );
            let session = if case == "child-session" {
                "ses_child"
            } else {
                "ses_root"
            };
            let database = home.path().join(".local/share/opencode/opencode.db");
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let expected_cwd = checkout.clone();
            let herdr = FakeHerdr::start_with_errors("opencode-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "removed-after" {
                            rusqlite::Connection::open(&database)
                                .unwrap()
                                .execute("DELETE FROM session WHERE id = 'ses_root'", [])
                                .unwrap();
                        }
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["root"]["cwd"], expected_cwd);
                        assert_eq!(params["focus"], false);
                        // The fork's tab runs the node's shell with the
                        // integration's launch.
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["kind"], "opencode");
                        assert_eq!(params["args"], json!(["-s", "ses_root", "--fork"]));
                        if case == "refused" {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"ok"})
                    }
                    other => panic!("unexpected {other}"),
                })
            });
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: session.into(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: session.into(),
                cwd: Some(checkout.clone()),
                name: "fork-opencode-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                &herdr.connector(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |_, _| {},
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}");
            let expected: &[&str] = match case {
                "success" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                ],
                "refused" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                    "pane.close",
                ],
                "other-checkout" | "child-session" => &[],
                "removed-after" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "pane.close",
                ],
                _ => &["session.snapshot", "layout.apply", "pane.close"],
            };
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    /// A node that forks OpenCode sessions as `answer` says and records each
    /// ask in `log`, beside the Herdr requests the test records there.
    struct OpenCodeForking {
        inner: hide_node::Local,
        answer: Result<hide_node_link::protocol::OpenCodeFork, &'static str>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl crate::node_access::NodeLink for OpenCodeForking {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let hide_node_link::protocol::Call::OpenCodeFork { source, cwd } = &call else {
                return self.inner.call(call, timeout);
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("opencode_fork {source} {cwd}"));
            match &self.answer {
                Ok(fork) => Ok(serde_json::to_value(fork).unwrap().into()),
                Err(reason) => Err(crate::node_access::LinkError::Refused(
                    hide_node_link::HostError::new(hide_node_link::ErrorCode::Io, *reason),
                )),
            }
        }

        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            self.inner.reader_features()
        }

        fn in_process(&self) -> bool {
            self.inner.in_process()
        }
    }

    /// OpenCode 2.0.29's interactive command has no `--fork`: the node's
    /// server forks the session once, before the fork's tab exists, and the
    /// tab opens the session it made; a node that cannot fork it ends the fork
    /// with its reason and no tab, and a fork whose start then fails names the
    /// session OpenCode made, which stays in OpenCode's list.
    #[test]
    fn an_opencode_2_fork_is_made_by_its_server_before_its_tab_and_the_tab_opens_it() {
        for case in ["forked", "refused", "start_failed"] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let checkout = cwd.display().to_string();
            crate::fixture::opencode_2_database(
                home.path(),
                &[("ses_root", None, checkout.as_str())],
            );
            let log = Arc::new(Mutex::new(Vec::new()));
            let node = OpenCodeForking {
                inner: hide_node::Local::new(Some(home.path().to_path_buf()))
                    .with_shell(crate::fixture::integrated_shell(home.path())),
                answer: if case == "refused" {
                    Err("opencode_fork_timed_out")
                } else {
                    Ok(hide_node_link::protocol::OpenCodeFork::Forked {
                        id: "ses_fork".into(),
                    })
                },
                log: Arc::clone(&log),
            };
            let herdr_log = Arc::clone(&log);
            let herdr = FakeHerdr::start_with_errors(
                "opencode-2-fork-tab",
                move |method, params| {
                    herdr_log.lock().unwrap().push(method.to_owned());
                    Ok(match method {
                        "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                            "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                            "workspaces": [], "tabs": [], "panes": [], "agents": [],
                            "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                                "area": {"x":0,"y":0,"width":80,"height":24},
                                "focused_pane_id": "parent-pane",
                                "panes": [{"pane_id":"parent-pane", "focused":true,
                                    "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                        }}),
                        "layout.apply" => {
                            json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                        "zoomed":false,"focused_pane_id":"child-pane",
                        "root":{"type":"pane","pane_id":"child-pane"}}})
                        }
                        "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                        "pane.get" => json!({"type":"pane_info","pane":{
                        "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                        "tokens":{"hide_prompt":"1"}}}),
                        "agent.start" => {
                            assert_eq!(params["kind"], "opencode");
                            assert_eq!(params["args"], json!(["-s", "ses_fork"]));
                            if case == "start_failed" {
                                return Err(("start_refused".into(), "fixture refused".into()));
                            }
                            json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                        }
                        "pane.close" => {
                            assert_eq!(params["pane_id"], "child-pane");
                            json!({"type":"ok"})
                        }
                        other => panic!("unexpected {other}"),
                    })
                },
            );
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: "ses_root".into(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: "ses_root".into(),
                cwd: Some(checkout.clone()),
                name: "fork-opencode-2".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                &herdr.connector(),
                &node,
                &request,
                &|| Ok(()),
                |_, _| {},
                |_, _| Ok(()),
            );
            let fork = format!("opencode_fork ses_root {checkout}");
            let opened = [
                fork.as_str(),
                "session.snapshot",
                "layout.apply",
                "pane.process_info",
                "pane.get",
                "agent.start",
            ];
            match case {
                "forked" => {
                    assert_eq!(result.unwrap(), "child-pane");
                    assert_eq!(*log.lock().unwrap(), opened);
                }
                "refused" => {
                    let error = result.unwrap_err();
                    assert!(error.contains("opencode_fork_timed_out"), "{error}");
                    assert_eq!(*log.lock().unwrap(), [fork.as_str()]);
                }
                _ => {
                    let error = result.unwrap_err();
                    assert!(
                        error.ends_with(
                            "OpenCode made the fork's session ses_fork, which stays in OpenCode's session list"
                        ),
                        "{error}"
                    );
                    let mut closed = opened.to_vec();
                    closed.push("pane.close");
                    assert_eq!(*log.lock().unwrap(), closed);
                }
            }
        }
    }

    #[test]
    fn a_grok_fork_resumes_its_proven_session_as_a_new_one_in_its_own_tab() {
        let id = "0199b000-0000-7000-8000-0000000000f0";
        for case in [
            "success",
            "refused",
            "summary-changed-before",
            "execution-after",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            let source = crate::fixture::grok_session(home.path(), &cwd, id);
            let summary = source.with_file_name("summary.json");
            if case == "summary-changed-before" {
                let other = std::fs::read_to_string(&summary)
                    .unwrap()
                    .replace(id, "0199b000-0000-7000-8000-0000000000f1");
                std::fs::write(&summary, other).unwrap();
            }
            let before = std::fs::read(&summary).unwrap();
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let observed_changed = Arc::clone(&changed);
            // The fork's tab runs this account's integrated shell and
            // waits for its prompt token, independently of installed shells.
            let shell = crate::fixture::integrated_shell(home.path());
            let expected_shell = shell.shell.clone().unwrap();
            let node = hide_node::Local::new(Some(home.path().to_path_buf())).with_shell(shell);
            let herdr = FakeHerdr::start_with_errors("grok-fork-tab", move |method, params| {
                Ok(match method {
                    "session.snapshot" => json!({"type": "session_snapshot", "snapshot": {
                        "version": "fixture", "protocol": HERDR_PROTOCOL_REVISION,
                        "workspaces": [], "tabs": [], "panes": [], "agents": [],
                        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                            "area": {"x":0,"y":0,"width":80,"height":24},
                            "focused_pane_id": "parent-pane",
                            "panes": [{"pane_id":"parent-pane", "focused":true,
                                "rect":{"x":0,"y":0,"width":80,"height":24}}], "splits": []}]
                    }}),
                    "layout.apply" => {
                        if case == "execution-after" {
                            observed_changed.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                        assert_eq!(params["focus"], false);
                        // The fork's tab runs the node's shell with the
                        // integration's launch.
                        assert_eq!(params["root"]["command"][0], expected_shell);
                        json!({"type":"layout_apply","layout":{"workspace_id":"w1","tab_id":"w1:t2",
                            "zoomed":false,"focused_pane_id":"child-pane",
                            "root":{"type":"pane","pane_id":"child-pane"}}})
                    }
                    "pane.process_info" => json!({"type":"pane_process_info","process_info":{
                        "pane_id":"child-pane","shell_pid":42,"foreground_process_group_id":42,
                        "foreground_processes":[{"pid":42,"name":"zsh"}]}}),
                    "pane.get" => {
                        assert_eq!(params["pane_id"], "child-pane");
                        json!({"type":"pane_info","pane":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"unknown","revision":1,
                            "tokens":{"hide_prompt":"1"}}})
                    }
                    "agent.start" => {
                        assert_eq!(params["kind"], "grok");
                        assert_eq!(params["args"], json!(["--resume", id, "--fork-session"]));
                        if case == "refused" {
                            return Err(("start_refused".into(), "fixture refused".into()));
                        }
                        json!({"type":"agent_started","argv":[],"agent":{
                            "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1","tab_id":"w1:t2","focused":false,"agent_status":"idle","revision":2}})
                    }
                    "pane.close" => json!({"type":"ok"}),
                    other => panic!("unexpected {other}"),
                })
            });
            // The label overlay hands a native-file agent its proven file.
            let request = ForkRequest {
                source_reference: Some(crate::sidebar::SessionAgentSessionPayload {
                    kind: "path".into(),
                    value: source.display().to_string(),
                }),
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::Grok,
                session_id: id.into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-grok-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let result = run_agent_fork_with_registration(
                &herdr.connector(),
                &node,
                &request,
                &|| {
                    if changed.load(std::sync::atomic::Ordering::SeqCst) {
                        Err("session_fork_execution_changed".into())
                    } else {
                        Ok(())
                    }
                },
                |_, _| {},
                |_, _| Ok(()),
            );
            assert_eq!(result.is_err(), case != "success", "{case}: {result:?}");
            assert_eq!(std::fs::read(&summary).unwrap(), before, "{case}");
            let expected: &[&str] = match case {
                "success" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                ],
                "refused" => &[
                    "session.snapshot",
                    "layout.apply",
                    "pane.process_info",
                    "pane.get",
                    "agent.start",
                    "pane.close",
                ],
                "summary-changed-before" => &[],
                _ => &["session.snapshot", "layout.apply", "pane.close"],
            };
            assert_eq!(herdr.methods(), expected, "{case}");
        }
    }

    #[test]
    fn an_agy_fork_types_fork_once_and_reports_the_one_conversation_it_made() {
        let source = "a9c0e2f4-0000-4000-8000-000000000001";
        let fork = "a9c0e2f4-0000-4000-8000-000000000002";
        // `moved`: the node's clock is behind the core's, so agy stamps the
        // fork's row in the past, and the source takes another step while
        // agy writes it; the fork is still the one row that was not there.
        // `untrusted`: agy does not trust the checkout, so the resumed pane
        // would open on agy's trust question, which `/fork` must not answer.
        // `in-motion`: the first index reads meet agy writing its index
        // (1 fork in 3 failed on a live agy 1.4.0) and are read again.
        // `unreadable`: a refusal that is not agy writing ends the fork at
        // its first read, with no second one.
        for case in [
            "success",
            "ambiguous",
            "moved",
            "untrusted",
            "in-motion",
            "unreadable",
        ] {
            let home = tempfile::tempdir().unwrap();
            let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
            if case != "untrusted" {
                let settings = home.path().join(".gemini/antigravity-cli/settings.json");
                std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
                std::fs::write(
                    &settings,
                    json!({ "trustedWorkspaces": [cwd.to_str().unwrap()] }).to_string(),
                )
                .unwrap();
            }
            crate::fixture::antigravity_conversation(
                home.path(),
                &cwd,
                source,
                4,
                Some("2026-10-03 01:00:00+00:00"),
            );
            let node = DatabaseInMotion {
                inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                left: std::sync::atomic::AtomicUsize::new(match case {
                    "in-motion" => 3,
                    "unreadable" => 1,
                    _ => 0,
                }),
                // What the node answers for each: agy writing its index is
                // busy, an index Hide cannot read is not.
                refusal: if case == "unreadable" {
                    (hide_node_link::ErrorCode::Io, "antigravity_db_unreadable")
                } else {
                    (hide_node_link::ErrorCode::Busy, "antigravity_db_changed")
                },
            };
            let index_home = home.path().to_path_buf();
            let index_cwd = cwd.clone();
            let statuses = Arc::new(Mutex::new(vec!["working", "idle"]));
            let herdr = FakeHerdr::start("agy-fork", move |method, params| match method {
                "pane.get" => {
                    let mut statuses = statuses.lock().unwrap();
                    let status = if statuses.len() > 1 {
                        statuses.remove(0)
                    } else {
                        statuses[0]
                    };
                    json!({"type":"pane_info","pane":{
                        "pane_id":"child-pane","terminal_id":"child-terminal","workspace_id":"w1",
                        "tab_id":"w1:t2","focused":false,"agent_status":status,"revision":1}})
                }
                "pane.send_input" => {
                    assert_eq!(params["text"], "/fork");
                    assert_eq!(params["keys"], json!(["enter"]));
                    // agy writes the fork's row at the source's step count.
                    crate::fixture::antigravity_conversation(
                        &index_home,
                        &index_cwd,
                        fork,
                        4,
                        (case == "moved").then_some("2026-10-01 00:00:00+00:00"),
                    );
                    if case == "moved" {
                        rusqlite::Connection::open(
                            index_home.join(".gemini/antigravity-cli/conversation_summaries.db"),
                        )
                        .unwrap()
                        .execute(
                            "UPDATE conversation_summaries SET step_count = 5 \
                             WHERE conversation_id = ?1",
                            [source],
                        )
                        .unwrap();
                    }
                    if case == "ambiguous" {
                        crate::fixture::antigravity_conversation(
                            &index_home,
                            &index_cwd,
                            "a9c0e2f4-0000-4000-8000-000000000003",
                            4,
                            None,
                        );
                    }
                    json!({"type":"ok"})
                }
                "pane.report_agent_session" => {
                    assert_eq!(params["agent_session_id"], fork);
                    assert_eq!(params["agent"], "agy");
                    assert_eq!(params["source"], "herdr:antigravity_cli");
                    json!({"type":"ok"})
                }
                other => panic!("unexpected {other}"),
            });
            let request = ForkRequest {
                source_reference: None,
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::Antigravity,
                session_id: source.into(),
                cwd: Some(cwd.display().to_string()),
                name: "fork-agy-1".into(),
                tab_label: Some("Tab 2".into()),
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let result = fork_antigravity(&herdr.connector(), &node, &request, "child-pane");
            let expected: &[&str] = match case {
                "success" | "moved" | "in-motion" => {
                    assert_eq!(result, Ok(()), "{case}");
                    &[
                        "pane.get",
                        "pane.get",
                        "pane.get",
                        "pane.send_input",
                        "pane.report_agent_session",
                    ]
                }
                "unreadable" => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_db_unreadable"),
                        "{result:?}"
                    );
                    &["pane.get", "pane.get"]
                }
                "untrusted" => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_fork_untrusted"),
                        "{result:?}"
                    );
                    &["pane.get", "pane.get"]
                }
                _ => {
                    assert!(
                        result
                            .as_ref()
                            .unwrap_err()
                            .contains("antigravity_fork_ambiguous"),
                        "{result:?}"
                    );
                    &["pane.get", "pane.get", "pane.get", "pane.send_input"]
                }
            };
            assert_eq!(herdr.methods(), expected, "{case}");
            assert_eq!(
                node.left.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{case}"
            );
        }
    }

    /// The route Fork, Sleep's wake and resume prove before they start an
    /// agent that keeps its records in a database, agy's index or
    /// OpenCode's database (Windows run 39104633599 refused an OpenCode
    /// wake this way): a proof that meets the agent writing is read again,
    /// and one that cannot read the database is refused at its first read.
    #[test]
    fn a_database_route_proof_is_read_again_only_while_it_meets_the_agent_writing() {
        for agent in ["agy", "opencode"] {
            for (left, code, proven) in [
                (3, hide_node_link::ErrorCode::Busy, true),
                (1, hide_node_link::ErrorCode::Io, false),
            ] {
                let home = tempfile::tempdir().unwrap();
                let cwd = hide_platform::fs::identity::canonical(home.path()).unwrap();
                let (id, reasons) = if agent == "agy" {
                    let id = "a9c0e2f4-0000-4000-8000-000000000001";
                    crate::fixture::antigravity_conversation(home.path(), &cwd, id, 4, None);
                    let transcript = hide_session::antigravity::transcript(home.path(), id);
                    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
                    std::fs::write(
                        &transcript,
                        include_str!(
                            "../../hide-session/tests/fixtures/adapters/antigravity-1.3.6/transcript_full.jsonl"
                        ),
                    )
                    .unwrap();
                    (id, ("antigravity_db_busy", "antigravity_db_unreadable"))
                } else {
                    let id = "ses_root";
                    crate::fixture::opencode_database(
                        home.path(),
                        &[(id, None, cwd.to_str().unwrap())],
                    );
                    (id, ("opencode_db_busy", "opencode_db_unreadable"))
                };
                let node = DatabaseInMotion {
                    inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                    left: std::sync::atomic::AtomicUsize::new(left),
                    refusal: (code, if proven { reasons.0 } else { reasons.1 }),
                };
                let reference = crate::sidebar::SessionAgentSessionPayload {
                    kind: "id".into(),
                    value: id.into(),
                };
                let result =
                    confirm_session_launch(&node, agent, id, cwd.to_str(), Some(&reference));
                assert_eq!(result.is_ok(), proven, "{agent} {code:?}: {result:?}");
                assert_eq!(
                    node.left.load(std::sync::atomic::Ordering::SeqCst),
                    0,
                    "{agent} {code:?}"
                );
            }
        }
    }

    /// An OpenCode fork whose route read met OpenCode writing its database
    /// forked nothing, so it is asked again; one that cannot read the
    /// database is refused at its first ask.
    #[test]
    fn an_opencode_fork_is_asked_again_only_while_its_route_meets_opencode_writing() {
        for (left, code, forked) in [
            (3, hide_node_link::ErrorCode::Busy, true),
            (1, hide_node_link::ErrorCode::Io, false),
        ] {
            let home = tempfile::tempdir().unwrap();
            let checkout = hide_platform::fs::identity::canonical(home.path())
                .unwrap()
                .display()
                .to_string();
            crate::fixture::opencode_database(home.path(), &[("ses_root", None, &checkout)]);
            let node = DatabaseInMotion {
                inner: hide_node::Local::new(Some(home.path().to_path_buf())),
                left: std::sync::atomic::AtomicUsize::new(left),
                refusal: (code, "opencode_db_busy"),
            };
            let request = ForkRequest {
                source_reference: None,
                connection_generation: 0,
                codex_daemon: Default::default(),
                parent_pane_id: "parent-pane".into(),
                agent: crate::fork::ForkableAgent::OpenCode,
                session_id: "ses_root".into(),
                cwd: Some(checkout),
                name: "fork-opencode-1".into(),
                tab_label: None,
                placement: ForkPlacement::Tab,
                launch: Default::default(),
            };
            let result = fork_arguments(&node, &request);
            assert_eq!(result.is_ok(), forked, "{code:?}: {result:?}");
            assert_eq!(
                node.left.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "{code:?}"
            );
        }
    }

    /// A node whose next `left` reads of an agent's database are refused
    /// with `refusal`.
    struct DatabaseInMotion {
        inner: hide_node::Local,
        left: std::sync::atomic::AtomicUsize,
        refusal: (hide_node_link::ErrorCode, &'static str),
    }

    impl crate::node_access::NodeLink for DatabaseInMotion {
        fn reader_features(&self) -> Option<&hide_node_link::sessions::ReaderFeatures> {
            self.inner.reader_features()
        }

        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            if matches!(
                call,
                hide_node_link::protocol::Call::AntigravityForkMark { .. }
                    | hide_node_link::protocol::Call::AntigravityFork { .. }
                    | hide_node_link::protocol::Call::SessionActivity { .. }
                    | hide_node_link::protocol::Call::OpenCodeFork { .. }
            ) && self
                .left
                .try_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |left| left.checked_sub(1),
                )
                .is_ok()
            {
                return Err(crate::node_access::LinkError::Refused(
                    hide_node_link::HostError::new(self.refusal.0, self.refusal.1),
                ));
            }
            self.inner.call(call, timeout)
        }
    }

    #[test]
    fn internal_registration_failure_keeps_the_started_fork() {
        let herdr = FakeHerdr::start("fork-registration-failure", |method, _| match method {
            "pane.split" => json!({"type": "pane_info", "pane": {
                "pane_id": "child-pane", "terminal_id": "child-terminal",
                "workspace_id": "w1", "tab_id": "w1:t1", "focused": true,
                "agent_status": "idle", "revision": 1
            }}),
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                "pane_id": "child-pane", "shell_pid": 42, "foreground_process_group_id": 42,
                "foreground_processes": [{"pid": 42, "name": "zsh"}]
            }}),
            "agent.start" => json!({"type": "agent_started", "argv": [], "agent": {
                "pane_id": "child-pane", "terminal_id": "child-terminal",
                "workspace_id": "w1", "tab_id": "w1:t1", "focused": true,
                "agent_status": "idle", "revision": 2
            }}),
            other => panic!("unexpected {other}"),
        });
        let request = ForkRequest {
            source_reference: None,
            connection_generation: 0,
            codex_daemon: crate::codex_launch::CodexDaemon::Present,
            parent_pane_id: "parent-pane".to_owned(),
            agent: crate::fork::ForkableAgent::Codex,
            session_id: "3f2b1c00-0000-4000-8000-000000000001".to_owned(),
            cwd: Some("/checkout".to_owned()),
            name: "fork-parent-pane-1".to_owned(),
            tab_label: None,
            placement: ForkPlacement::Side,
            launch: Default::default(),
        };

        let pane_id = run_agent_fork_with_registration(
            &herdr.connector(),
            &hide_node::Local::new(None),
            &request,
            &|| Ok(()),
            |_, _| {},
            |_, _| Err("ledger_unavailable".into()),
        )
        .expect("registration cannot close a started fork");
        assert_eq!(pane_id, "child-pane");
        assert_eq!(
            herdr.methods(),
            ["pane.split", "pane.process_info", "agent.start",]
        );
    }

    #[test]
    fn tab_rename_uses_the_selected_host_and_reports_refusal() {
        let host = FakeHerdr::start_with_errors("tab-rename", |method, params| {
            assert_eq!(method, "tab.rename");
            assert_eq!(params["tab_id"], "w1:t1");
            if params["label"] == "refuse" {
                Err(("not_found".into(), "tab unavailable".into()))
            } else {
                Ok(json!({"type":"ok"}))
            }
        });
        for label in ["이름", "", "refuse"] {
            let result = execute_remote_control(
                &host.connector(),
                &RemoteControlAction::RenameTab {
                    tab_id: "w1:t1".into(),
                    label: label.into(),
                    request_id: "r1".into(),
                },
            );
            assert_eq!(result.is_ok(), label != "refuse");
        }
        assert_eq!(host.methods(), ["tab.rename", "tab.rename", "tab.rename"]);
    }

    #[test]
    fn nested_split_reopen_applies_the_original_outer_and_inner_geometry() {
        let herdr = FakeHerdr::start("reopen-nested-layout", |method, params| match method {
            "layout.export" => json!({"type":"layout_export","layout": {
                "workspace_id":"w1", "tab_id":"w1:t1", "zoomed":false,
                "focused_pane_id":"outer-left",
                "root":{"type":"split", "direction":"right", "ratio":0.4,
                    "first":{"type":"pane", "pane_id":"outer-left", "cwd":"/tmp", "env":{}},
                    "second":{"type":"split", "direction":"right", "ratio":0.65,
                        "first":{"type":"pane", "pane_id":"inner-left", "cwd":"/tmp", "env":{}},
                        "second":{"type":"pane", "pane_id":"inner-right", "cwd":"/tmp", "env":{}}}}
            }}),
            "layout.apply" => {
                // The tab keeps its own label: Herdr would keep any label
                // sent as the tab's name.
                assert!(params["tab_label"].is_null());
                let root = &params["root"];
                assert_eq!(root["direction"], "right");
                assert!((root["ratio"].as_f64().unwrap() - 0.4).abs() < 0.000_001);
                assert_eq!(root["first"]["pane_id"], "outer-left");
                assert_eq!(root["second"]["direction"], "down");
                assert!((root["second"]["ratio"].as_f64().unwrap() - 0.3).abs() < 0.000_001);
                assert_eq!(
                    root["second"]["first"]["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("nested-intent", ReopenIntentStage::Pane)
                );
                assert_eq!(root["second"]["second"]["direction"], "right");
                assert!(
                    (root["second"]["second"]["ratio"].as_f64().unwrap() - 0.65).abs() < 0.000_001
                );
                assert_eq!(root["second"]["second"]["first"]["pane_id"], "inner-left");
                assert_eq!(root["second"]["second"]["second"]["pane_id"], "inner-right");
                json!({"type":"layout_apply","layout": {
                    "workspace_id":"w1", "tab_id":"w1:t1", "zoomed":false,
                    "focused_pane_id":"reopened",
                    "root":{"type":"split", "direction":"right", "ratio":0.4,
                        "first":{"type":"pane", "pane_id":"outer-left", "cwd":"/tmp", "env":{}},
                        "second":{"type":"split", "direction":"down", "ratio":0.3,
                            "first":{"type":"pane", "pane_id":"reopened", "cwd":"/tmp", "env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("nested-intent", ReopenIntentStage::Pane)
                            }},
                            "second":{"type":"split", "direction":"right", "ratio":0.65,
                                "first":{"type":"pane", "pane_id":"inner-left", "cwd":"/tmp", "env":{}},
                                "second":{"type":"pane", "pane_id":"inner-right", "cwd":"/tmp", "env":{}}}}}
                }})
            }
            other => panic!("unexpected {other}"),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec![
                "outer-left".into(),
                "closed".into(),
                "inner-left".into(),
                "inner-right".into(),
            ],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Agent one".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: crate::recent_closed::ClosedTabName::Unnamed,
        };
        let pane = ClosedPane {
            pane_id: "closed".into(),
            label: None,
            cwd: "/tmp".into(),
            agent: None,
        };
        let placement = PanePlacement {
            neighbor_pane_id: None,
            parent_path: vec![ClosedLayoutBranch::Second],
            direction: crate::recent_closed::ClosedSplitDirection::Down,
            ratio: 0.3,
            target_was_first: true,
        };
        let request = ReopenRequest {
            codex_daemon: Default::default(),
            item: ClosedItem::Pane {
                key: "nested-intent".into(),
                context: context.clone(),
                pane: pane.clone(),
                placement: placement.clone(),
            },
            workspace_exists: true,
            tab_exists: true,
            fallback_pane_id: Some("outer-left".into()),
            owner: None,
        };

        let outcome = reopen_pane(
            &herdr.connector(),
            &|path| Path::new(path).is_dir(),
            "nested-intent",
            &context,
            &pane,
            &placement,
            &request,
        )
        .expect("nested pane reopens through layout.apply");

        assert_eq!(outcome.focused_pane_id.as_deref(), Some("reopened"));
        assert_eq!(herdr.methods(), ["layout.export", "layout.apply"]);
    }

    #[test]
    fn over_count_tab_layout_is_not_adopted_or_overwritten() {
        let pane = |id: &str| ClosedLayoutNode::Pane {
            pane_id: Some(id.to_owned()),
            label: None,
            cwd: Some("/tmp".to_owned()),
            command: None,
            env: Default::default(),
        };
        let expected_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            first: Box::new(pane("expected-a")),
            second: Box::new(pane("expected-b")),
        };
        let actual_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            first: Box::new(expected_root.clone()),
            second: Box::new(pane("unknown-extra")),
        };
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into(), "w1:p2".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let error = repair_incomplete_tab_layout(
            &LocalSocketConnector::new("/tmp/hide-reopen-over-count-must-not-connect.sock"),
            "over-count",
            &context,
            &expected_root,
            crate::recent_closed::ClosedLayout {
                workspace_id: "w1".into(),
                tab_id: "w1:t2".into(),
                zoomed: false,
                focused_pane_id: "unknown-extra".into(),
                root: actual_root,
            },
            &mut Vec::new(),
        )
        .expect_err("an extra pane is not owned by the reopen intent");

        assert!(error.contains("returned 3 panes, expected 2"));
        assert!(error.contains("not adopted or overwritten"));
        assert!(error.contains("retry is available"));
    }

    #[test]
    fn partial_tab_restore_is_not_consumed_as_success() {
        let error = ensure_complete_tab_restore(3, 2).expect_err("one pane is still missing");
        assert!(error.contains("restored 2 of 3"));
        assert!(error.contains("retry is available"));
        assert!(ensure_complete_tab_restore(3, 3).is_ok());
    }

    // A folder is made untraversable through its mode bits.
    #[cfg(unix)]
    #[test]
    fn file_reopen_distinguishes_metadata_failure_from_a_missing_file() {
        use std::os::unix::fs::PermissionsExt;
        let folder = tempfile::tempdir().expect("fixture folder");
        let root = folder.path().to_path_buf();
        let denied = root.join("denied");
        let file = denied.join("document.txt");
        std::fs::create_dir_all(&denied).expect("create denied fixture");
        std::fs::write(&file, "saved").expect("write fixture");
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000))
            .expect("deny traversal");

        let result = run_file_reopen(
            &hide_node::Local::of_process(),
            &crate::files::DocumentRoot {
                device_id: crate::node::TEST_NODE.to_owned(),
                path: root.to_string_lossy().into_owned(),
                identity: None,
            },
            file.to_str().unwrap(),
        );

        // Traversal comes back before the folder is dropped, or it could not
        // be removed.
        std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o700))
            .expect("restore traversal");
        match result {
            FileReopenResultOrHerdr::File(FileReopenResult::Failed(message)) => {
                assert!(message.contains("could not be opened"), "{message}");
            }
            other => panic!("permission failure must stay retryable, got {other:?}"),
        }
    }

    #[test]
    fn pane_reopen_ignores_an_unowned_same_cwd_pane() {
        let herdr = FakeHerdr::start("reopen-owned-pane", |method, _| match method {
            "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                "workspaces":[], "tabs":[], "agents":[],
                "panes":[{"pane_id":"w1:p3", "terminal_id": "fixture-terminal","workspace_id":"w1","tab_id":"w1:t1","cwd":"/tmp","focused":false,"agent_status":"idle","revision":0}],
                "layouts":[{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,
                    "area":{"x":0,"y":0,"width":80,"height":24}, "focused_pane_id":"w1:p1",
                    "panes":[{"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":40,"height":24}},
                             {"pane_id":"w1:p3","focused":false,"rect":{"x":40,"y":0,"width":40,"height":24}}],
                    "splits":[] }]
            }}),
            "layout.export" => json!({
                "type": "layout_export",
                "layout": {
                    "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
                    "focused_pane_id": "w1:p1",
                    "root": {"type":"split", "direction":"right", "ratio":0.5,
                        "first":{"type":"pane","pane_id":"w1:p1","cwd":"/tmp","env":{}},
                        "second":{"type":"pane","pane_id":"w1:p3","cwd":"/tmp","env":{}}}
                }
            }),
            "pane.split" => json!({"type": "pane_info", "pane": {
                "pane_id": "w1:p4", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1",
                "focused": true, "agent_status": "idle", "revision": 1
            }}),
            method => panic!("unexpected method {method}"),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into(), "w1:p2".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let pane = ClosedPane {
            pane_id: "w1:p2".into(),
            label: None,
            cwd: "/tmp".into(),
            agent: None,
        };
        let placement = PanePlacement {
            neighbor_pane_id: Some("w1:p1".into()),
            parent_path: Vec::new(),
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            target_was_first: false,
        };
        let request = ReopenRequest {
            codex_daemon: Default::default(),
            item: ClosedItem::Pane {
                key: "first-attempt".into(),
                context: context.clone(),
                pane: pane.clone(),
                placement: placement.clone(),
            },
            workspace_exists: true,
            tab_exists: true,
            fallback_pane_id: Some("w1:p1".into()),
            owner: None,
        };

        let outcome = reopen_pane(
            &herdr.connector(),
            &|path| Path::new(path).is_dir(),
            "first-attempt",
            &context,
            &pane,
            &placement,
            &request,
        )
        .expect("fixture reopen succeeds");

        assert_eq!(outcome.focused_pane_id.as_deref(), Some("w1:p4"));
        let requests = herdr.requests();
        let split = requests
            .iter()
            .find(|request| request["method"] == "pane.split")
            .expect("the reopen creates its own pane");
        assert_eq!(
            split["params"]["env"][REOPEN_INTENT_ENV],
            reopen_intent_marker("first-attempt", ReopenIntentStage::Pane)
        );
        assert!(
            requests
                .iter()
                .all(|request| request["method"] != "pane.send_text")
        );
    }

    #[test]
    fn tab_reopen_ignores_unowned_same_label_workspace_and_tab() {
        let mut layout_apply_count = 0;
        let herdr = FakeHerdr::start("reopen-owned-layout", move |method, params| match method {
            "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                "workspaces":[{"workspace_id":"w2","label":"Fixture","active_tab_id":"w2:t1","number":2,"focused":true,"pane_count":1,"tab_count":1,"agent_status":"idle"}],
                "tabs":[{"workspace_id":"w2","tab_id":"w2:t1","label":"Tab","number":1,"focused":true,"pane_count":1,"agent_status":"idle"}],
                "panes":[], "layouts":[], "agents":[]
            }}),
            "layout.export" => json!({"type":"layout_export","layout": {
                "workspace_id":"w2", "tab_id":"w2:t1", "zoomed":false,
                "focused_pane_id":"w2:p9",
                "root":{"type":"pane","pane_id":"w2:p9","cwd":"/tmp","env":{}}
            }}),
            "workspace.create" => {
                assert_eq!(
                    params["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("layout-intent", ReopenIntentStage::Workspace)
                );
                json!({"type":"workspace_created",
                    "workspace":{"workspace_id":"w3","number":3,"label":"Fixture","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w3:t1","agent_status":"idle"},
                    "tab":{"tab_id":"w3:t1","workspace_id":"w3","number":1,"label":"1","focused":true,"pane_count":1,"agent_status":"idle"},
                    "root_pane":{"pane_id":"w3:p3", "terminal_id": "fixture-terminal","workspace_id":"w3","tab_id":"w3:t1","focused":true,"agent_status":"idle","revision":1}
                })
            }
            "layout.apply" => {
                assert_eq!(params["tab_id"], "w3:t1");
                assert!(params.get("workspace_id").is_none());
                assert_eq!(
                    params["root"]["first"]["env"][REOPEN_INTENT_ENV],
                    reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                );
                layout_apply_count += 1;
                if layout_apply_count == 1 {
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w3", "tab_id":"w3:t1", "zoomed":false,
                        "focused_pane_id":"w3:p3",
                        "root":{"type":"pane","pane_id":"w3:p3","cwd":"/tmp","env":{
                            (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                        }}
                    }})
                } else {
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w3", "tab_id":"w3:t1", "zoomed":false,
                        "focused_pane_id":"w3:p3",
                        "root":{"type":"split","direction":"right","ratio":0.6,
                            "first":{"type":"pane","pane_id":"w3:p3","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                            }},
                            "second":{"type":"pane","pane_id":"w3:p4","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("layout-intent", ReopenIntentStage::Layout)
                            }}}
                    }})
                }
            }
            "workspace.list" => json!({"type":"workspace_list","workspaces":[
                {"workspace_id":"w2","number":2,"label":"other","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w2:t1","agent_status":"idle"}
            ]}),
            "workspace.report_metadata" => json!({"type":"ok"}),
            _ => unreachable!(),
        });
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let layout_root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.6,
            first: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
            second: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
        };
        let mut notices = Vec::new();
        let restored = ensure_workspace_and_tab(
            &herdr.connector(),
            "layout-intent",
            &context,
            &OwnerOpen::Folder {
                path: "/tmp".into(),
                label: "Fixture".into(),
                mark: crate::checkout_owner::owner_mark(crate::node::TEST_NODE, "/tmp"),
                legacy_mark: None,
            },
            false,
            &layout_root,
            &mut notices,
        )
        .expect("owned layout is created");

        assert_eq!(restored.workspace_id, "w3");
        assert_eq!(restored.tab_id, "w3:t1");
        assert_eq!(restored.focused_pane_id, "w3:p3");
        assert_eq!(restored.root.pane_count(), 2);
        assert_eq!(
            notices,
            ["The incomplete tab restore was repaired before reopening sessions"]
        );
        assert_eq!(
            herdr.methods(),
            // The closed tab's workspace is gone, so it reopens in the
            // folder's owner, made and marked here; the old index names a
            // place in another workspace and is not restored.
            [
                "session.snapshot",
                "layout.export",
                "workspace.list",
                "workspace.create",
                "workspace.report_metadata",
                "layout.apply",
                "layout.apply"
            ]
        );
    }

    #[test]
    fn recovered_partial_tab_layout_is_repaired_and_revalidated() {
        let herdr = FakeHerdr::start(
            "reopen-repair-recovered",
            move |method, params| match method {
                "session.snapshot" => json!({"type":"session_snapshot","snapshot": {
                    "version":"fixture", "protocol":HERDR_PROTOCOL_REVISION,
                    "workspaces":[{"workspace_id":"w1","label":"Fixture","active_tab_id":"w1:t2","number":1,"focused":true,"pane_count":1,"tab_count":1,"agent_status":"idle",
                        "worktree":{"repo_key":"/tmp","repo_name":"tmp","repo_root":"/tmp","checkout_path":"/tmp","is_linked_worktree":false}}],
                    "tabs":[{"workspace_id":"w1","tab_id":"w1:t2","label":"Tab","number":1,"focused":true,"pane_count":1,"agent_status":"idle"}],
                    "panes":[], "layouts":[], "agents":[]
                }}),
                "layout.export" => json!({"type":"layout_export","layout": {
                    "workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false,
                    "focused_pane_id":"w1:p2",
                    "root":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{
                        (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                    }}
                }}),
                "layout.apply" => {
                    assert_eq!(params["tab_id"], "w1:t2");
                    assert_eq!(
                        params["root"]["second"]["env"][REOPEN_INTENT_ENV],
                        reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                    );
                    json!({"type":"layout_apply","layout": {
                        "workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false,
                        "focused_pane_id":"w1:p2",
                        "root":{"type":"split","direction":"right","ratio":0.6,
                            "first":{"type":"pane","pane_id":"w1:p2","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                            }},
                            "second":{"type":"pane","pane_id":"w1:p3","cwd":"/tmp","env":{
                                (REOPEN_INTENT_ENV): reopen_intent_marker("recovered-intent", ReopenIntentStage::Layout)
                            }}}
                    }})
                }
                "tab.move" => json!({"type":"tab_list","tabs":[]}),
                _ => unreachable!(),
            },
        );
        let context = ClosedContext {
            workspace_id: "w1".into(),
            workspace_label: "Fixture".into(),
            workspace_ids_before_close: vec!["w1".into()],
            tab_ids_before_close: vec!["w1:t1".into()],
            pane_ids_before_close: vec!["w1:p1".into()],
            checkout_id: "checkout".into(),
            checkout_path: "/tmp".into(),
            tab_id: "w1:t1".into(),
            tab_label: "Tab".into(),
            tab_index: 0,
            agent_area: None,
            agent_group: None,
            replacement_shell: false,
            replacement_in_tab: false,
            tab_name: Default::default(),
        };
        let root = ClosedLayoutNode::Split {
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.6,
            first: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
            second: Box::new(ClosedLayoutNode::Pane {
                pane_id: None,
                label: None,
                cwd: Some("/tmp".into()),
                command: None,
                env: Default::default(),
            }),
        };
        let mut notices = Vec::new();
        let restored = ensure_workspace_and_tab(
            &herdr.connector(),
            "recovered-intent",
            &context,
            &OwnerOpen::Worktree {
                path: "/tmp".into(),
                repository_root: "/tmp".into(),
                label: "Fixture".into(),
            },
            false,
            &root,
            &mut notices,
        )
        .expect("the recovered layout is repaired");

        assert_eq!(restored.tab_id, "w1:t2");
        assert_eq!(restored.root.pane_count(), 2);
        assert_eq!(
            notices,
            ["The incomplete tab restore was repaired before reopening sessions"]
        );
        assert_eq!(
            herdr.methods(),
            [
                "session.snapshot",
                "layout.export",
                "layout.apply",
                "tab.move"
            ]
        );
    }

    #[test]
    fn reused_agent_process_is_interrupted_before_resume() {
        let herdr = FakeHerdr::start("reopen-interrupt", |method, params| match method {
            "session.snapshot" => json!({
                "type": "session_snapshot",
                "snapshot": {
                    "version": "fixture",
                    "protocol": HERDR_PROTOCOL_REVISION,
                    "workspaces": [],
                    "tabs": [],
                    "panes": [],
                    "layouts": [],
                    "agents": [{
                        "terminal_id": "term-1",
                        "name": "old-agent",
                        "agent": "claude",
                        "agent_status": "idle",
                        "workspace_id": "w1",
                        "tab_id": "w1:t1",
                        "pane_id": "w1:p1",
                        "focused": false,
                        "interactive_ready": true,
                        "state_change_seq": 1,
                        "cwd": "/tmp",
                        "foreground_cwd": "/tmp",
                        "revision": 0
                    }]
                }
            }),
            "pane.send_text" => {
                assert_eq!(*params, json!({"pane_id": "w1:p1", "text": "\u{3}"}));
                json!({"type": "ok"})
            }
            other => panic!("unexpected {other}"),
        });

        assert!(
            interrupt_reused_agent(&herdr.connector(), "fixture", 0, "w1:p1")
                .expect("reused agent is interrupted")
        );

        assert_eq!(herdr.methods(), ["session.snapshot", "pane.send_text"]);
    }

    #[test]
    fn protocol_revision_matches_the_canonical_contract() {
        const CONTRACT: &str = include_str!("../../contracts/herdr-api.schema.json");

        let schema: Value = serde_json::from_str(CONTRACT).expect("contract is valid JSON");
        assert_eq!(schema["protocol"], HERDR_PROTOCOL_REVISION);
    }

    #[test]
    fn a_registered_project_gets_its_checkouts_owner_not_a_plain_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let folder = root.join("notes");
        let repo = root.join("repo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let folder = folder.to_string_lossy().into_owned();
        let repo = repo.to_string_lossy().into_owned();

        let paths = workspace::paths_here([folder.clone(), repo.clone()]);
        assert_eq!(
            registered_owner(&crate::node::test_node(), &paths, &folder, "Notes"),
            OwnerOpen::for_checkout(crate::node::TEST_NODE, &folder, &folder, false, "Notes")
                .on_node()
        );
        assert_eq!(
            registered_owner(&crate::node::test_node(), &paths, &repo, "Repo"),
            OwnerOpen::Worktree {
                path: repo.clone(),
                repository_root: repo.clone(),
                label: "Repo".to_owned(),
            }
        );
    }

    #[test]
    fn local_tab_control_opens_the_checkouts_bound_workspace_when_none_is_open() {
        let herdr = FakeHerdr::start("owner-open-worktree", |method, _| match method {
            "worktree.open" => json!({
                "type": "worktree_opened",
                "already_open": false,
                "workspace": {"workspace_id": "w9", "number": 9, "label": "repo-feat", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"},
                "worktree": {"path": "/repo-feat", "is_bare": false, "is_detached": false, "is_prunable": false, "is_linked_worktree": true, "label": "repo-feat"},
                "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                "root_pane": {"pane_id": "w9:p1", "terminal_id": "fixture-terminal", "workspace_id": "w9", "tab_id": "w9:t1", "focused": true, "agent_status": "idle", "revision": 1}
            }),
            "tab.rename" => json!({
                "type": "tab_info",
                "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"}
            }),
            "workspace.rename" => json!({
                "type": "workspace_info",
                "workspace": {"workspace_id": "w9", "number": 9, "label": "feat", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"}
            }),
            other => panic!("unexpected {other}"),
        });

        let outcome = execute_remote_control(
            &herdr.connector(),
            &RemoteControlAction::OpenOwner {
                owner: OwnerOpen::Worktree {
                    path: "/repo-feat".to_owned(),
                    repository_root: "/repo".to_owned(),
                    label: "feat".to_owned(),
                },
                cwd: "/repo-feat".to_owned(),
                label: "Tab 2".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
        )
        .expect("owner open");

        // A newly opened owner's first tab is the new tab.
        assert!(matches!(
            outcome,
            RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(ref tab_id),
                created_pane_id: Some(ref pane_id),
            } if tab_id == "w9:t1" && pane_id == "w9:p1"
        ));
        // Herdr opens a linked worktree only from its repository's main
        // worktree, and a label on the open would rename a workspace that was
        // already there, so the new one is named after it opens (D-14).
        assert_eq!(
            herdr.calls(),
            [
                (
                    "worktree.open".to_owned(),
                    json!({"cwd": "/repo", "path": "/repo-feat", "focus": true})
                ),
                (
                    "workspace.rename".to_owned(),
                    json!({"workspace_id": "w9", "label": "feat"})
                ),
                (
                    "tab.rename".to_owned(),
                    json!({"tab_id": "w9:t1", "label": "Tab 2"})
                ),
            ]
        );
    }

    #[test]
    fn a_second_request_for_an_open_owner_adds_a_tab_there_instead_of_another_workspace() {
        for focus in [true, false] {
            let herdr = FakeHerdr::start("owner-already-open", |method, _| match method {
                "worktree.open" => json!({
                    "type": "worktree_opened",
                    "already_open": true,
                    "workspace": {"workspace_id": "w9", "number": 9, "label": "hide", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w9:t1", "agent_status": "idle"},
                    "worktree": {"path": "/repo", "is_bare": false, "is_detached": false, "is_prunable": false, "is_linked_worktree": false, "label": "hide"},
                    "tab": {"tab_id": "w9:t1", "workspace_id": "w9", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w9:p1", "terminal_id": "fixture-terminal", "workspace_id": "w9", "tab_id": "w9:t1", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                "tab.create" => json!({
                    "type": "tab_created",
                    "tab": {"tab_id": "w9:t2", "workspace_id": "w9", "number": 2, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w9:p2", "terminal_id": "fixture-terminal-2", "workspace_id": "w9", "tab_id": "w9:t2", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                other => panic!("unexpected {other}"),
            });

            let tab = open_owner_tab(
                &herdr.connector(),
                &OwnerOpen::Worktree {
                    path: "/repo".to_owned(),
                    repository_root: "/repo".to_owned(),
                    label: "hide".to_owned(),
                },
                "/repo",
                "Tab 2",
                focus,
                Default::default(),
                None,
            )
            .expect("owner tab");

            assert_eq!(
                tab,
                OwnedTab {
                    initial_label: None,
                    workspace_id: "w9".to_owned(),
                    tab_id: "w9:t2".to_owned(),
                    pane_id: "w9:p2".to_owned(),
                    launched: true,
                }
            );
            assert_eq!(herdr.methods(), ["worktree.open", "tab.create"]);
            assert_eq!(herdr.calls()[0].1["focus"], focus);
            assert_eq!(herdr.calls()[1].1["focus"], focus);
        }
    }

    #[test]
    fn a_refused_owner_open_fails_the_request_and_creates_nothing_elsewhere() {
        let herdr = FakeHerdr::start_with_errors("owner-refused", |method, _| match method {
            "worktree.open" => Err(("not_found".to_owned(), "no such checkout".to_owned())),
            other => panic!("unexpected {other}"),
        });

        let failure = open_owner_tab(
            &herdr.connector(),
            &OwnerOpen::Worktree {
                path: "/gone".to_owned(),
                repository_root: "/gone".to_owned(),
                label: "gone".to_owned(),
            },
            "/gone",
            "Tab 1",
            true,
            Default::default(),
            None,
        )
        .expect_err("a refused open fails");

        assert!(!failure.is_ambiguous());
        assert!(failure.message().contains("no such checkout"));
        assert_eq!(herdr.methods(), ["worktree.open"]);
    }

    #[test]
    fn a_plain_folder_reuses_its_marked_owner_and_marks_a_new_one_only_when_none_is_live() {
        for focus in [true, false] {
            let mark = crate::checkout_owner::owner_mark(crate::node::TEST_NODE, "/notes");
            let listed_mark = mark.clone();
            let marked = Arc::new(Mutex::new(false));
            let marked_in_server = marked.clone();
            let herdr = FakeHerdr::start("owner-folder", move |method, params| match method {
                "workspace.list" => {
                    let tokens = if *marked_in_server.lock().unwrap() {
                        json!({"hide_owner": listed_mark})
                    } else {
                        json!({})
                    };
                    json!({"type": "workspace_list", "workspaces": [
                        {"workspace_id": "w1", "number": 1, "label": "other", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"},
                        {"workspace_id": "w4", "number": 4, "label": "notes", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w4:t1", "agent_status": "idle", "tokens": tokens}
                    ]})
                }
                "workspace.create" => json!({
                    "type": "workspace_created",
                    "workspace": {"workspace_id": "w4", "number": 4, "label": "notes", "focused": true, "pane_count": 1, "tab_count": 1, "active_tab_id": "w4:t1", "agent_status": "idle"},
                    "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "1", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w4:p1", "terminal_id": "fixture-terminal", "workspace_id": "w4", "tab_id": "w4:t1", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                "workspace.report_metadata" => {
                    assert_eq!(params["tokens"]["hide_owner"], json!(listed_mark));
                    *marked_in_server.lock().unwrap() = true;
                    json!({"type": "ok"})
                }
                "tab.rename" => json!({
                    "type": "tab_info",
                    "tab": {"tab_id": "w4:t1", "workspace_id": "w4", "number": 1, "label": "Tab 1", "focused": true, "pane_count": 1, "agent_status": "idle"}
                }),
                "tab.create" => json!({
                    "type": "tab_created",
                    "tab": {"tab_id": "w4:t2", "workspace_id": "w4", "number": 2, "label": "Tab 2", "focused": true, "pane_count": 1, "agent_status": "idle"},
                    "root_pane": {"pane_id": "w4:p2", "terminal_id": "fixture-terminal-2", "workspace_id": "w4", "tab_id": "w4:t2", "focused": true, "agent_status": "idle", "revision": 1}
                }),
                other => panic!("unexpected {other}"),
            });
            let owner = OwnerOpen::Folder {
                path: "/notes".to_owned(),
                label: "notes".to_owned(),
                mark,
                legacy_mark: None,
            };

            let first = open_owner_tab(
                &herdr.connector(),
                &owner,
                "/notes",
                "Tab 1",
                focus,
                Default::default(),
                None,
            )
            .expect("first tab");
            let second = open_owner_tab(
                &herdr.connector(),
                &owner,
                "/notes",
                "Tab 2",
                focus,
                Default::default(),
                None,
            )
            .expect("second tab");

            assert_eq!(
                (first.workspace_id.as_str(), first.tab_id.as_str()),
                ("w4", "w4:t1")
            );
            assert_eq!(
                (second.workspace_id.as_str(), second.tab_id.as_str()),
                ("w4", "w4:t2")
            );
            assert!(*marked.lock().unwrap());
            assert_eq!(herdr.calls()[1].1["focus"], focus);
            assert_eq!(herdr.calls()[5].1["focus"], focus);
            assert_eq!(
                herdr.methods(),
                [
                    "workspace.list",
                    "workspace.create",
                    "workspace.report_metadata",
                    "tab.rename",
                    "workspace.list",
                    "tab.create"
                ]
            );
        }
    }

    #[test]
    fn pane_control_uses_the_official_socket_contract_for_every_mutation() {
        let herdr = FakeHerdr::start("pane-control", |method, _| match method {
            "pane.split" => {
                json!({"type": "pane_info", "pane": {"pane_id": "w1:p2", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}})
            }
            "pane.zoom" => zoom_answer("w1:p1", true),
            "pane.close" => json!({"type": "ok"}),
            other => panic!("unexpected {other}"),
        });
        let connector = herdr.connector();
        for action in [
            PaneControlAction::Split {
                pane_id: "w1:p1".to_owned(),
                direction: PaneSplitDirection::Right,
                cwd: Some("/tmp/hide-verify-shortcuts".to_owned()),
            },
            PaneControlAction::Split {
                pane_id: "w1:p1".to_owned(),
                direction: PaneSplitDirection::Down,
                cwd: None,
            },
            PaneControlAction::ToggleZoom {
                pane_id: "w1:p1".to_owned(),
            },
            PaneControlAction::Close {
                pane_id: "w1:p1".to_owned(),
            },
        ] {
            execute_pane_control(&connector, &action).expect("control request");
        }
        assert_eq!(
            herdr.calls(),
            [
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "w1:p1",
                        "direction": "right",
                        "right_click": "herdr",
                        "focus": true,
                        "cwd": "/tmp/hide-verify-shortcuts"
                    }),
                ),
                (
                    "pane.split".to_owned(),
                    json!({
                        "target_pane_id": "w1:p1",
                        "direction": "down",
                        "right_click": "herdr",
                        "focus": true
                    }),
                ),
                (
                    "pane.zoom".to_owned(),
                    json!({"pane_id": "w1:p1", "mode": "toggle"})
                ),
                ("pane.close".to_owned(), json!({"pane_id": "w1:p1"})),
            ]
        );
    }

    fn one_pane_layout(pane_id: &str) -> Value {
        json!({
            "workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": pane_id,
            "panes": [{"pane_id": pane_id, "focused": true,
                       "rect": {"x": 0, "y": 0, "width": 120, "height": 60}}],
            "splits": []
        })
    }

    fn zoom_answer(pane_id: &str, zoom_changed: bool) -> Value {
        json!({"type": "pane_zoom", "zoom": {
            "changed": true, "zoom_changed": zoom_changed, "focus_changed": !zoom_changed,
            "pane_id": pane_id, "focused_pane_id": pane_id, "zoomed": zoom_changed,
            "reason": (!zoom_changed).then_some("single_pane"),
            "layout": one_pane_layout(pane_id)
        }})
    }

    fn resize_answer(pane_id: &str, changed: bool) -> Value {
        json!({"type": "pane_resize", "resize": {
            "changed": changed, "pane_id": pane_id, "focused_pane_id": pane_id,
            "layout": one_pane_layout(pane_id)
        }})
    }

    /// Herdr answers a zoom of a lone pane with `zoom_changed: false` (and
    /// `changed: true` when only focus moved) and a resize with nothing to
    /// move with `changed: false`. Neither sends a layout event, so both read
    /// as Unchanged and the operation can end at the answer (PRD
    /// instant-pane-topology D-12). The change flag decides, not `changed`.
    #[test]
    fn a_zoom_or_resize_that_moved_nothing_reads_as_unchanged() {
        let herdr = FakeHerdr::start("pane-control-unchanged", |method, params| {
            let moved = params["pane_id"] == "w1:p2";
            match method {
                "pane.zoom" => zoom_answer(params["pane_id"].as_str().unwrap(), moved),
                "pane.resize" => resize_answer(params["pane_id"].as_str().unwrap(), moved),
                other => panic!("unexpected {other}"),
            }
        });
        let connector = herdr.connector();
        let zoom = |pane: &str| PaneControlAction::ToggleZoom {
            pane_id: pane.to_owned(),
        };
        let resize = |pane: &str| PaneControlAction::Resize {
            pane_id: pane.to_owned(),
            direction: PaneResizeDirection::Right,
            amount: 0.05,
        };
        for (action, unchanged) in [
            (zoom("w1:p1"), true),
            (zoom("w1:p2"), false),
            (resize("w1:p1"), true),
            (resize("w1:p2"), false),
        ] {
            let outcome = execute_pane_control(&connector, &action).expect("control request");
            assert_eq!(
                matches!(outcome, PaneControlOutcome::Unchanged),
                unchanged,
                "{action:?} answered {outcome:?}"
            );
        }
    }

    /// An answer without the change flag cannot say whether a layout event
    /// will follow, so the operation's result is unknown, not a success.
    #[test]
    fn a_zoom_answer_without_its_result_is_ambiguous() {
        let herdr = FakeHerdr::start("pane-control-zoom-ok", |_, _| json!({"type": "ok"}));
        let failure = execute_pane_control(
            &herdr.connector(),
            &PaneControlAction::ToggleZoom {
                pane_id: "w1:p1".to_owned(),
            },
        )
        .expect_err("a bare ok is not a zoom result");
        assert!(
            matches!(failure, ControlFailure::Ambiguous(_)),
            "{failure:?}"
        );
    }

    #[test]
    fn remote_session_control_uses_the_official_socket_contract() {
        let herdr = FakeHerdr::start("remote-control", |method, _| match method {
            "workspace.focus" | "tab.focus" | "tab.close" => json!({"type": "ok"}),
            "tab.create" => json!({
                "type": "tab_created",
                "tab": {"tab_id": "w1:t3", "workspace_id": "w1", "number": 1, "label": "fixture", "focused": false, "pane_count": 1, "agent_status": "idle"},
                "root_pane": {"pane_id": "w1:p3", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}
            }),
            other => panic!("unexpected {other}"),
        });
        let connector = herdr.connector();
        let actions = [
            RemoteControlAction::FocusWorkspace {
                workspace_id: "w1".to_owned(),
            },
            RemoteControlAction::FocusTab {
                tab_id: "w1:t2".to_owned(),
            },
            RemoteControlAction::CreateTab {
                workspace_id: "w1".to_owned(),
                cwd: "/tmp/hide-remote-tab".to_owned(),
                label: "New tab".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
            RemoteControlAction::CloseTab {
                tab_id: "w1:t3".to_owned(),
            },
        ];
        let mut outcomes = actions
            .iter()
            .map(|action| execute_remote_control(&connector, action).expect("control request"));
        assert!(matches!(
            outcomes.next(),
            Some(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            })
        ));
        assert!(matches!(
            outcomes.next(),
            Some(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            })
        ));
        assert!(matches!(
            outcomes.next(),
            Some(RemoteControlOutcome::Acknowledged {
                created_tab_id: Some(tab_id),
                created_pane_id: Some(pane_id),
            }) if tab_id == "w1:t3" && pane_id == "w1:p3"
        ));
        assert!(matches!(
            outcomes.next(),
            Some(RemoteControlOutcome::Acknowledged {
                created_tab_id: None,
                created_pane_id: None,
            })
        ));
        assert_eq!(
            herdr.calls(),
            [
                ("workspace.focus".to_owned(), json!({"workspace_id": "w1"})),
                ("tab.focus".to_owned(), json!({"tab_id": "w1:t2"})),
                (
                    "tab.create".to_owned(),
                    json!({
                        "workspace_id": "w1",
                        "cwd": "/tmp/hide-remote-tab",
                        "focus": true,
                        "label": "New tab"
                    }),
                ),
                ("tab.close".to_owned(), json!({"tab_id": "w1:t3"})),
            ]
        );
    }

    #[test]
    #[ignore = "requires an owned remote fixture and HERDR_TEST_REMOTE_CONTROL_* variables"]
    fn official_remote_control_fixture_probe() {
        let alias_name = std::env::var("HERDR_TEST_SSH_ALIAS")
            .expect("HERDR_TEST_SSH_ALIAS names a configured SSH host");
        let workspace_id = std::env::var("HERDR_TEST_REMOTE_CONTROL_WORKSPACE_ID")
            .expect("HERDR_TEST_REMOTE_CONTROL_WORKSPACE_ID names the owned fixture workspace");
        let cwd = std::env::var("HERDR_TEST_REMOTE_CONTROL_CWD")
            .expect("HERDR_TEST_REMOTE_CONTROL_CWD names the owned fixture directory");
        assert!(
            cwd.starts_with("/tmp/hide-verify-"),
            "remote control fixture must use the owned fixture namespace"
        );

        let home = std::env::var_os("HOME").expect("HOME is configured");
        use crate::remote::DeviceConnector as _;
        let connector = hide_node::ssh::Connector::new(None)
            .transport(std::path::Path::new(&home), "probe", &alias_name, None)
            .expect("SSH alias resolves")
            .herdr_api_connector();
        let connector = &*connector;

        let response = request_with_connector(
            connector,
            "session.snapshot",
            json!({}),
            Duration::from_secs(5),
        )
        .expect("fixture session snapshot");
        let snapshot = response["snapshot"]
            .as_object()
            .map(|_| &response["snapshot"])
            .expect("session.snapshot response contains a snapshot");
        let workspace = snapshot["workspaces"]
            .as_array()
            .and_then(|workspaces| {
                workspaces.iter().find(|workspace| {
                    workspace["workspace_id"].as_str() == Some(workspace_id.as_str())
                })
            })
            .expect("owned fixture workspace is present");
        assert!(
            workspace["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("hide-verify-")),
            "remote control refused a workspace outside the owned fixture namespace"
        );
        let original_tab_id = snapshot["tabs"]
            .as_array()
            .and_then(|tabs| {
                tabs.iter()
                    .find(|tab| tab["workspace_id"].as_str() == Some(workspace_id.as_str()))
            })
            .and_then(|tab| tab["tab_id"].as_str())
            .expect("owned fixture workspace has a tab")
            .to_owned();

        execute_remote_control(
            connector,
            &RemoteControlAction::FocusWorkspace {
                workspace_id: workspace_id.clone(),
            },
        )
        .expect("focus owned fixture workspace");
        execute_remote_control(
            connector,
            &RemoteControlAction::FocusTab {
                tab_id: original_tab_id,
            },
        )
        .expect("focus owned fixture tab");

        let RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(created_tab_id),
            created_pane_id: Some(created_root_pane_id),
        } = execute_remote_control(
            connector,
            &RemoteControlAction::CreateTab {
                workspace_id: workspace_id.clone(),
                cwd: cwd.clone(),
                label: "Hide remote control probe".to_owned(),
                area_id: None,
                group_id: None,
                admission_id: None,
            },
        )
        .expect("create fixture tab")
        else {
            panic!("tab.create did not return the created tab and root pane ids");
        };

        let RemoteControlOutcome::Acknowledged {
            created_tab_id: None,
            created_pane_id: Some(created_split_pane_id),
        } = execute_remote_control(
            connector,
            &RemoteControlAction::Pane(PaneControlAction::Split {
                pane_id: created_root_pane_id.clone(),
                direction: PaneSplitDirection::Right,
                cwd: Some(cwd),
            }),
        )
        .expect("split fixture pane")
        else {
            panic!("pane.split did not return the created pane id");
        };

        for action in [
            RemoteControlAction::Pane(PaneControlAction::Focus {
                pane_id: created_split_pane_id.clone(),
            }),
            RemoteControlAction::Pane(PaneControlAction::ToggleZoom {
                pane_id: created_split_pane_id.clone(),
            }),
            RemoteControlAction::Pane(PaneControlAction::ToggleZoom {
                pane_id: created_split_pane_id.clone(),
            }),
            RemoteControlAction::Pane(PaneControlAction::Close {
                pane_id: created_split_pane_id.clone(),
            }),
        ] {
            execute_remote_control(connector, &action).expect("mutate only the fixture pane");
        }

        remote_snapshot_until(
            connector,
            "authoritative snapshot did not converge after remote controls",
            |snapshot| {
                let created_tab_visible = snapshot["tabs"].as_array().is_some_and(|tabs| {
                    tabs.iter()
                        .any(|tab| tab["tab_id"].as_str() == Some(created_tab_id.as_str()))
                });
                let created_root_visible = snapshot["panes"].as_array().is_some_and(|panes| {
                    panes
                        .iter()
                        .any(|pane| pane["pane_id"].as_str() == Some(created_root_pane_id.as_str()))
                });
                let closed_split_absent = snapshot["panes"].as_array().is_some_and(|panes| {
                    panes.iter().all(|pane| {
                        pane["pane_id"].as_str() != Some(created_split_pane_id.as_str())
                    })
                });
                created_tab_visible && created_root_visible && closed_split_absent
            },
        );
    }

    /// Asks the remote Herdr for its snapshot until `ready` accepts it; the
    /// remote server applies a control on its own time.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    fn remote_snapshot_until(
        connector: &dyn ApiConnector,
        what: &str,
        ready: impl Fn(&Value) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let response = request_with_connector(
                connector,
                "session.snapshot",
                json!({}),
                Duration::from_secs(5),
            )
            .expect("post-control session snapshot");
            let snapshot = response["snapshot"]
                .as_object()
                .map(|_| &response["snapshot"])
                .expect("session.snapshot response contains a snapshot");
            if ready(snapshot) {
                return;
            }
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn pane_control_worker_returns_before_the_socket_receipt() {
        // Herdr holds its answer until the spawn has returned, so a spawn that
        // waited for the receipt would never return rather than return late.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let herdr = FakeHerdr::start("pane-worker", move |method, _| {
            assert_eq!(method, "pane.split");
            held.recv_timeout(Duration::from_secs(30))
                .expect("the test lets Herdr answer");
            json!({"type": "pane_info", "pane": {"pane_id": "w1:p2", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}})
        });
        let context = LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: Weak::new(),
            notifier: ChangeNotifier::noop(),
            api_connector: Arc::new(herdr.connector()),
            node: Arc::new(hide_node::Local::of_process()),
        };

        spawn_pane_control(
            context,
            PaneControlAction::Split {
                pane_id: "w1:p1".to_owned(),
                direction: PaneSplitDirection::Right,
                cwd: Some("/tmp".to_owned()),
            },
        )
        .expect("worker starts");

        // Returning at all is the proof: Herdr is still holding its answer.
        // The worker still delivers the request; the spawn only stopped
        // waiting for its answer.
        release.send(()).unwrap();
        herdr.wait_for_requests(1, Duration::from_secs(5));
        assert_eq!(herdr.methods(), ["pane.split"]);
    }

    #[test]
    fn focus_requires_authoritative_pane_and_owning_tab_confirmation() {
        for (focused_pane, active_tab, confirmed) in [
            ("fixture:p2", "fixture:t1", true),
            ("fixture:p1", "fixture:t1", false),
            ("fixture:p2", "fixture:t2", false),
        ] {
            let herdr = FakeHerdr::start("focus-contract", move |method, params| {
                if method == "workspace.get" {
                    assert_eq!(params["workspace_id"], "fixture");
                } else {
                    assert_eq!(params["pane_id"], "fixture:p2");
                }
                match method {
                    "pane.focus" => {
                        json!({"type": "pane_info", "pane": {"pane_id": "fixture:p2", "terminal_id": "fixture-terminal", "workspace_id": "fixture", "tab_id": "fixture:t1", "focused": false, "agent_status": "idle", "revision": 1}})
                    }
                    "pane.layout" => json!({
                        "type": "pane_layout",
                        "layout": {
                            "workspace_id": "fixture",
                            "tab_id": "fixture:t1",
                            "zoomed": false,
                            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                            "focused_pane_id": focused_pane,
                            "panes": [
                                {"pane_id": "fixture:p1", "focused": focused_pane == "fixture:p1", "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                                {"pane_id": "fixture:p2", "focused": focused_pane == "fixture:p2", "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
                            ],
                            "splits": [
                                {"id": "fixture:split1", "direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 120, "height": 60}}
                            ]
                        }
                    }),
                    "workspace.get" => json!({
                        "type": "workspace_info",
                        "workspace": {"workspace_id": "fixture", "number": 1, "label": "fixture", "focused": false, "pane_count": 2, "tab_count": 2, "active_tab_id": active_tab, "agent_status": "idle"}
                    }),
                    other => panic!("unexpected {other}"),
                }
            });

            let result = execute_pane_focus(&herdr.connector(), "fixture:p2");
            assert_eq!(
                result.is_ok(),
                confirmed,
                "pane={focused_pane}, active tab={active_tab}"
            );
            if let Ok(layout) = result {
                assert_eq!(layout.focused_pane_id, "fixture:p2");
                assert_eq!(layout.pane_ids(), ["fixture:p1", "fixture:p2"]);
            }
            assert_eq!(
                herdr.methods(),
                ["pane.focus", "pane.layout", "workspace.get"]
            );
            // Checking a proposed stream move shares the confirmation
            // contract and must not produce another focus mutation.
            let readback = confirm_pane_focus(&herdr.connector(), "fixture:p2");
            assert_eq!(
                readback,
                if confirmed {
                    FocusReadback::Confirmed
                } else {
                    FocusReadback::Elsewhere
                }
            );
            assert_eq!(
                herdr.methods(),
                [
                    "pane.focus",
                    "pane.layout",
                    "workspace.get",
                    "pane.layout",
                    "workspace.get"
                ]
            );
        }
    }

    #[test]
    fn pane_focus_rejects_trickling_and_over_cap_mutation_responses_as_unknown() {
        use hide_platform::ipc::LocalListener;
        use std::io::{BufRead, BufReader, Write};

        for over_cap in [false, true] {
            let root = tempfile::Builder::new().prefix("hpf-").tempdir().unwrap();
            let socket = root.path().join("peer.sock");
            let listener = LocalListener::bind(&socket).unwrap();
            let closer = listener.closer();
            let server = thread::spawn(move || {
                let Ok(mut stream) = listener.accept() else {
                    return None;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request).unwrap();
                let request: Value = serde_json::from_str(&request).unwrap();
                if over_cap {
                    let _ = stream.write_all(&vec![b' '; 64 * 1024 + 1]);
                } else {
                    // Progress defeats a per-read timeout. The peer itself is
                    // bounded so a regressed client still ends and fails.
                    let end = Instant::now() + Duration::from_millis(500);
                    while Instant::now() < end && stream.write_all(b" ").is_ok() {
                        thread::park_timeout(Duration::from_millis(5));
                    }
                }
                Some(request)
            });
            let timeout = if over_cap {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(50)
            };
            let result = execute_pane_focus_with_timeout(
                &LocalSocketConnector::new(&socket),
                "w1:p2",
                timeout,
            );
            closer.close();
            let request = server.join().unwrap().expect("focus request reached peer");
            assert_eq!(request["method"], "pane.focus");
            let error = result.expect_err("unfinished response must be rejected");
            assert!(error.is_ambiguous());
            assert!(error.message().contains(if over_cap {
                "exceeds 64 KiB"
            } else {
                "timed out"
            }));
        }
    }

    #[test]
    fn wire_snapshot_projects_agents_with_workspace_labels_and_verbatim_tokens() {
        let snapshot = json!({
            "protocol": HERDR_PROTOCOL_REVISION,
            "version": "fixture",
            "panes": [],
            "tabs": [],
            "workspaces": [
                {"workspace_id": "w1", "label": "hide", "active_tab_id": "w1:t1", "number": 1, "focused": true, "pane_count": 1, "tab_count": 1, "agent_status": "idle"},
            ],
            "layouts": [],
            "agents": [
                {
                    "pane_id": "w1:p1",
                    "terminal_id": "terminal-1", "tab_id": "w1:t1", "focused": true, "revision": 1,
                    "workspace_id": "w1",
                    "agent": "claude",
                    "agent_status": "working",
                    "cwd": "/tmp/project",
                    "tokens": {
                        "status_working": "●",
                        "activity": "1787963036671",
                        "summary": "doing things",
                        "elapsed": "6h"
                    }
                },
                {
                    "pane_id": "w9:p2",
                    "terminal_id": "terminal-2", "tab_id": "w9:t1", "focused": false, "revision": 1, "agent_status": "idle",
                    "workspace_id": "w9",
                    "tokens": {"status_idle": "○", "activity": "1787963036672"}
                }
            ]
        });
        let payload = project_session(&snapshot).expect("projects");
        assert_eq!(payload.agents.len(), 2);
        assert_eq!(payload.agents[0].pane_id.as_deref(), Some("w1:p1"));
        assert_eq!(payload.agents[0].workspace_label.as_deref(), Some("hide"));
        assert_eq!(
            payload.agents[0].tokens.get("status_working"),
            Some(&json!("●"))
        );
        // A workspace without a label entry falls back to its id.
        assert_eq!(payload.agents[1].workspace_label.as_deref(), Some("w9"));

        let projected = crate::sidebar::project_agents(payload).agents;
        assert_eq!(
            (projected[0].demand.as_str(), projected[0].activity.as_str()),
            ("none", "working")
        );
        assert_eq!(
            (projected[1].demand.as_str(), projected[1].activity.as_str()),
            ("none", "stopped")
        );
    }

    #[test]
    fn wire_snapshot_preserves_herdr_tab_labels() {
        let snapshot = json!({
            "protocol": HERDR_PROTOCOL_REVISION,
            "version": "fixture",
            "panes": [],
            "workspaces": [{"workspace_id": "w1", "label": "verify", "active_tab_id": "w1:t1", "number": 1, "focused": true, "pane_count": 1, "tab_count": 1, "agent_status": "idle"}],
            "tabs": [{
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "label": "2",
                "number": 2,
                "focused": true,
                "pane_count": 1,
                "agent_status": "idle"
            }],
            "layouts": [],
            "agents": []
        });

        let payload = project_session(&snapshot).expect("projects tab metadata");
        assert_eq!(payload.tabs.len(), 1);
        assert_eq!(payload.tabs[0].tab_id, "w1:t1");
        assert_eq!(payload.tabs[0].label, "2");
    }

    #[test]
    fn protocol_mismatch_is_an_explicit_failure() {
        let snapshot = json!({"protocol": 20, "workspaces": [], "agents": []});
        let error = project_session(&snapshot).expect_err("must fail");
        assert_eq!(error.state(), "protocol_mismatch");
        assert!(error.message().contains("20"));
    }

    #[test]
    fn session_layout_projects_authoritative_nested_tree_and_zoom() {
        let snapshot = json!({
            "protocol": HERDR_PROTOCOL_REVISION,
            "version": "fixture",
            "panes": [],
            "tabs": [],
            "workspaces": [{"workspace_id": "w1", "label": "verify", "active_tab_id": "w1:t1", "number": 1, "focused": true, "pane_count": 1, "tab_count": 1, "agent_status": "idle"}],
            "agents": [],
            "focused_pane_id": "w1:p3",
            "layouts": [{
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "zoomed": true,
                "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                "focused_pane_id": "w1:p3",
                "panes": [
                    {"pane_id": "w1:p1", "focused": false, "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                    {"pane_id": "w1:p2", "focused": false, "rect": {"x": 60, "y": 0, "width": 60, "height": 30}},
                    {"pane_id": "w1:p3", "focused": true, "rect": {"x": 60, "y": 30, "width": 60, "height": 30}}
                ],
                "splits": [
                    {"id": "split_0_root", "direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 120, "height": 60}},
                    {"id": "split_1_1", "direction": "down", "ratio": 0.5, "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
                ]
            }]
        });

        let payload = project_session(&snapshot).expect("session projection");
        let layout = project_layout_for_pane(&payload, "w1:p3").expect("layout projection");
        assert_eq!(layout.workspace_id, "w1");
        assert_eq!(layout.tab_id, "w1:t1");
        assert_eq!(layout.focused_pane_id, "w1:p3");
        assert!(layout.zoomed);
        assert_eq!(layout.pane_ids(), ["w1:p1", "w1:p2", "w1:p3"]);
        assert_eq!(
            serde_json::to_value(&layout.root).expect("layout JSON"),
            json!({
                "type": "split",
                "direction": "right",
                "ratio": 0.5,
                "first": {"type": "pane", "pane_id": "w1:p1"},
                "second": {
                    "type": "split",
                    "direction": "down",
                    "ratio": 0.5,
                    "first": {"type": "pane", "pane_id": "w1:p2"},
                    "second": {"type": "pane", "pane_id": "w1:p3"}
                }
            })
        );
    }

    #[test]
    fn missing_socket_file_is_distinguished_from_unreachable() {
        let error = fetch_session_with_connector(&LocalSocketConnector::new(
            "/nonexistent/herdr-core-test.sock",
        ))
        .expect_err("must fail");
        assert_eq!(error.state(), "socket_missing");
    }
}

#[cfg(test)]
#[path = "live_agent_close_tests.rs"]
mod agent_close_tests;
