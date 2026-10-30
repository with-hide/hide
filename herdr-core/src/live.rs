//! Live Herdr commands and pane byte transport. Session state synchronization
//! lives in `session_sync` and uses the sequenced socket event stream.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::engine::{Engine, EngineError, SharedEngine, Wait};
use serde::Deserialize;
use serde_json::json;

use crate::checkout_owner::OwnerOpen;
use crate::checkout_owner::open::open_owner_tab;
use crate::fork::worker::read_past_writes;
use crate::handle::ChangeNotifier;
use crate::model::{PaneLayoutDirection, PaneLayoutNodeSnapshot, PaneLayoutSnapshot};
use crate::recent_closed::capture::{
    CloseCaptureTarget, CloseEffectRequest, CloseSafety, run_close_effect,
};
use crate::recent_closed::reopen::{
    ReopenIntentStage, export_reopen_layout, pane_ids_with_reopen_marker, reopen_intent_env,
    reopen_intent_marker,
};
use crate::runtime::{LaneStart, PendingPaneFocusControl, Runtime};
use crate::sidebar::{
    SessionLayoutPanePayload, SessionLayoutPayload, SessionLayoutRect, SessionSnapshotPayload,
};

/// What live commands and session sync need from the live configuration.
#[derive(Clone)]
pub struct LiveContext {
    pub socket_path: PathBuf,
    pub runtime: Weak<Mutex<Runtime>>,
    pub notifier: ChangeNotifier,
    /// This machine's engine.
    pub(crate) engine: SharedEngine,
    /// The core's own node, which does this machine's work (PRD
    /// core-host-node D-21).
    pub(crate) node: Arc<dyn crate::node_access::NodeLink>,
}

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
    pub(crate) engine: SharedEngine,
    pub(crate) runtime: Weak<Mutex<Runtime>>,
    pub(crate) notifier: ChangeNotifier,
}

impl RemoteControlContext {
    pub(crate) fn new(
        target_id: impl Into<String>,
        engine: SharedEngine,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) -> Self {
        Self {
            target_id: target_id.into(),
            engine,
            runtime,
            notifier,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    /// The engine this device's control requests use.
    pub(crate) fn engine(&self) -> SharedEngine {
        Arc::clone(&self.engine)
    }
}

pub(crate) fn execute_remote_control(
    engine: &dyn Engine,
    action: &RemoteControlAction,
) -> Result<RemoteControlOutcome, ControlFailure> {
    let (created_tab_id, created_pane_id) = match action {
        RemoteControlAction::Pane(action) => match action {
            PaneControlAction::Focus { .. }
            | PaneControlAction::Split { .. }
            | PaneControlAction::ToggleZoom { .. }
            | PaneControlAction::Close { .. } => match execute_pane_control(engine, action)? {
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
            engine
                .focus_workspace(CONTROL_WAIT, workspace_id)
                .map_err(mutation_failure("workspace.focus"))?;
            (None, None)
        }
        RemoteControlAction::FocusTab { tab_id } => {
            engine
                .focus_tab(CONTROL_WAIT, tab_id)
                .map_err(mutation_failure("tab.focus"))?;
            (None, None)
        }
        RemoteControlAction::CreateTab {
            workspace_id,
            cwd,
            label,
            admission_id,
            ..
        } => {
            let (tab_id, pane_id) = engine
                .create_tab(
                    CONTROL_WAIT,
                    workspace_id,
                    cwd,
                    label,
                    true,
                    local_create_env(*admission_id),
                )
                .map_err(mutation_failure("tab.create"))?;
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
                engine,
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
            engine
                .close_tab(CONTROL_WAIT, tab_id)
                .map_err(mutation_failure("tab.close"))?;
            (None, None)
        }
        RemoteControlAction::RenameTab { tab_id, label, .. } => {
            engine
                .rename_tab(CONTROL_WAIT, tab_id, label)
                .map_err(mutation_failure("tab.rename"))?;
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
            let tab_ids = engine
                .move_tab(CONTROL_WAIT, tab_id, *insert_index)
                .map_err(mutation_failure("tab.move"))?;
            return Ok(RemoteControlOutcome::TabsOrdered { tab_ids });
        }
    };
    Ok(RemoteControlOutcome::Acknowledged {
        created_tab_id,
        created_pane_id,
    })
}

fn execute_pane_control(
    engine: &dyn Engine,
    action: &PaneControlAction,
) -> Result<PaneControlOutcome, ControlFailure> {
    if let PaneControlAction::Project { pane_id } = action {
        return fetch_pane_layout(engine, pane_id)
            .map(|layout| PaneControlOutcome::Projected { layout })
            .map_err(ControlFailure::Definite);
    }
    if let PaneControlAction::Focus { pane_id } = action {
        engine
            .focus_pane(CONTROL_WAIT, pane_id)
            .map_err(mutation_failure("pane.focus"))?;
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
        let changed = engine
            .resize_pane(CONTROL_WAIT, pane_id, *direction, *amount)
            .map_err(mutation_failure("pane.resize"))?;
        return Ok(if changed {
            PaneControlOutcome::Acknowledged {
                created_pane_id: None,
            }
        } else {
            PaneControlOutcome::Unchanged
        });
    }
    if let PaneControlAction::ToggleZoom { pane_id } = action {
        let changed = engine
            .toggle_zoom(CONTROL_WAIT, pane_id)
            .map_err(mutation_failure("pane.zoom"))?;
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
            Some(
                engine
                    .split_pane(CONTROL_WAIT, pane_id, *direction, cwd.as_deref())
                    .map_err(mutation_failure("pane.split"))?,
            )
        }
        PaneControlAction::Close { pane_id } => {
            engine
                .close_pane(CONTROL_WAIT, pane_id)
                .map_err(mutation_failure("pane.close"))?;
            None
        }
        PaneControlAction::MoveToNewTab {
            pane_id,
            workspace_id,
            label,
        } => {
            // The created tab is checked so a refusal Herdr reported as an
            // unchanged move fails the action rather than reading as done.
            engine
                .move_pane_to_new_tab(CONTROL_WAIT, pane_id, workspace_id, label)
                .map_err(mutation_failure("pane.move"))?;
            None
        }
        PaneControlAction::Project { .. }
        | PaneControlAction::Focus { .. }
        | PaneControlAction::Resize { .. }
        | PaneControlAction::ToggleZoom { .. } => unreachable!("handled above"),
    };
    Ok(PaneControlOutcome::Acknowledged { created_pane_id })
}

/// The bound of one control call: each step within five seconds.
pub(crate) const CONTROL_WAIT: Wait = Wait::Each(Duration::from_secs(5));

/// A failed read or change `operation` as a control step reports it.
pub(crate) fn failed(operation: &str) -> impl FnOnce(EngineError) -> String + '_ {
    move |error| error.within(&format!("{operation} failed"))
}

/// A failed change `operation` as the control lane reports it: a refusal
/// or a request the engine never got is definite, and a call whose answer
/// never came or could not be read may have happened.
pub(crate) fn mutation_failure(operation: &str) -> impl FnOnce(EngineError) -> ControlFailure + '_ {
    move |error| match error {
        EngineError::Refused { code, message } => {
            ControlFailure::Definite(format!("{operation} was refused: {code}: {message}"))
        }
        EngineError::Request(message) => ControlFailure::Definite(message),
        EngineError::Answer(message) => ControlFailure::Ambiguous(message),
        other => ControlFailure::Ambiguous(format!("{operation} result is unknown: {other}")),
    }
}

/// `node` runs the pane (`agent_start::start_at_shell_checked`); none
/// when it cannot be reached. A start whose shell had not finished starting
/// is refused as `agent_start::SHELL_STARTING` with the screen's last line,
/// so the spawn that asked can say what the shell waits for.
#[allow(clippy::too_many_arguments)] // the start identity and the pane's node
pub(crate) fn start_agent(
    engine: &dyn Engine,
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
        engine,
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
        crate::agent_start::StartError::Herdr(EngineError::Refused { code, .. })
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
pub(crate) fn start_agent_checked(
    engine: &dyn Engine,
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
    let start = engine
        .prepare_start(crate::engine::AgentLaunch {
            pane_id: pane_id.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            args,
            codex_daemon,
        })
        .map_err(|detail| crate::agent_start::StartError::NotStarted {
            reason: "agent_start.failed".into(),
            detail,
        })?;
    match crate::agent_start::start_at_shell_checked(
        engine,
        node,
        request_id,
        shell,
        &start,
        Duration::from_secs(5),
        check,
    ) {
        Ok(started) => Ok(started.pane_id),
        Err(crate::agent_start::StartError::Herdr(EngineError::Answer(detail))) => {
            Err(crate::agent_start::StartError::NotStarted {
                reason: "agent_start.unconfirmed".into(),
                detail,
            })
        }
        Err(error) => Err(error),
    }
}

/// A start's failure as an operator reads it.
pub(crate) fn start_error_message(error: crate::agent_start::StartError) -> String {
    match error {
        crate::agent_start::StartError::NotStarted { detail, .. } => detail,
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

fn fetch_pane_layout(engine: &dyn Engine, pane_id: &str) -> Result<PaneLayoutSnapshot, String> {
    let layout = engine
        .pane_layout(CONTROL_WAIT, pane_id)
        .map_err(failed("pane.layout"))?;
    project_layout(&layout)
}

fn execute_pane_focus(
    engine: &dyn Engine,
    pane_id: &str,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    execute_pane_focus_with_timeout(engine, pane_id, Duration::from_secs(5))
}

fn execute_pane_focus_with_timeout(
    engine: &dyn Engine,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    // These three responses share the established absolute read deadline and
    // 64 KiB frame cap. A lost mutation response has a different outcome from
    // an authoritative read failing after Herdr already acknowledged it.
    engine
        .focus_pane(Wait::Small(response_timeout), pane_id)
        .map_err(mutation_failure("pane.focus"))?;
    confirm_pane_focus_with_timeout(engine, pane_id, response_timeout)
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
pub(crate) fn confirm_pane_focus(engine: &dyn Engine, pane_id: &str) -> FocusReadback {
    match read_pane_focus(engine, pane_id, Duration::from_secs(5)) {
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
    engine: &dyn Engine,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<PaneLayoutSnapshot, ControlFailure> {
    let (layout, active_tab_id) = read_pane_focus(engine, pane_id, response_timeout)?;
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
    engine: &dyn Engine,
    pane_id: &str,
    response_timeout: Duration,
) -> Result<(PaneLayoutSnapshot, String), ControlFailure> {
    let layout = engine
        .pane_layout(Wait::Small(response_timeout), pane_id)
        .map_err(failed("pane.layout"))?;
    let layout = project_layout(&layout)?;
    let active_tab_id = engine
        .workspace_active_tab(Wait::Small(response_timeout), &layout.workspace_id)
        .map_err(failed("workspace.get"))?;
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
                            execute_local_control(context.engine.as_ref(), &action)
                        }))
                        .unwrap_or_else(|_| Err(lane_worker_panicked(action.kind())));
                        (context, Finished::Tab(action, result))
                    }
                    LaneStart::PaneFocus { context, control } => {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute_pane_focus(context.engine.as_ref(), &control.target_id)
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
            let result = execute_pane_control(context.engine.as_ref(), &action);
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

/// `engine`, with `current` checked on both sides of reaching it for every
/// request. Topology reads and opening an SSH stream can wait, so the same
/// control, file node and admitted intent is checked again once connected.
/// `current` checks current runtime facts and, for a saved group start, its
/// owning node's terminal incarnation outside Runtime's mutex; its refusal
/// answers in Herdr's place with its own code and [`SESSION_CHANGED`].
pub(crate) fn current_session<'a>(
    engine: &'a dyn Engine,
    current: &'a (dyn Fn() -> Result<(), String> + Sync),
) -> Box<dyn Engine + 'a> {
    engine.gated(crate::engine::Gate::both(move || {
        current().map_err(|code| EngineError::Refused {
            code,
            message: SESSION_CHANGED.into(),
        })
    }))
}

/// The message of the refusal [`current_session`] answers in Herdr's place,
/// before anything reached Herdr: its `code` is Hide's own check's.
pub(crate) const SESSION_CHANGED: &str = "The session execution changed; refresh before retrying";

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
                    context.engine.as_ref(),
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
                    EngineError::Refused { .. } => ControlFailure::Definite(error.to_string()),
                    _ => ControlFailure::Ambiguous(error.to_string()),
                })
            } else {
                execute_remote_control(context.engine.as_ref(), &action)
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
pub(crate) fn execute_local_control(
    engine: &dyn Engine,
    action: &RemoteControlAction,
) -> Result<RemoteControlOutcome, ControlFailure> {
    let result = execute_remote_control(engine, action);
    if !result.as_ref().is_err_and(ControlFailure::is_ambiguous) {
        return result;
    }
    match recover_local_creation(engine, action) {
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
    engine: &dyn Engine,
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
    let snapshot = fetch_session(engine).map_err(|error| error.message().to_owned())?;
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
        let layout = export_reopen_layout(engine, &tab.tab_id)?;
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
                message: crate::engine::herdr::device_protocol_behind(
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
    own_engine: SharedEngine,
    socket_path: &str,
    usage_paths: crate::usage::UsagePaths,
) -> Option<crate::session_sync::SessionSyncHandle> {
    let context = LiveContext {
        socket_path: PathBuf::from(socket_path),
        runtime: Arc::downgrade(runtime),
        notifier: notifier.clone(),
        engine: own_engine,
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

/// The whole session as the sidebar projects it, read once.
pub(crate) fn fetch_session(
    engine: &dyn Engine,
) -> Result<SessionSnapshotPayload, SessionFetchError> {
    engine
        .session_payload(CONTROL_WAIT)
        .map_err(|error| match error {
            EngineError::NotRunning(message) => SessionFetchError::SocketMissing(message),
            EngineError::Protocol {
                message,
                expected_protocol,
                received_protocol,
                received_version,
            } => SessionFetchError::Protocol {
                message,
                expected_protocol,
                received_protocol,
                received_version,
            },
            EngineError::Answer(message) => SessionFetchError::Malformed(message),
            error => SessionFetchError::Unreachable(error.to_string()),
        })
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

    use hide_herdr_client::{ApiConnector, HERDR_PROTOCOL_REVISION, request_with_connector};
    use serde_json::Value;

    use super::*;
    use crate::fake_herdr::FakeHerdr;

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
                host.engine().as_ref(),
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
    fn protocol_revision_matches_the_canonical_contract() {
        const CONTRACT: &str = include_str!("../../contracts/herdr-api.schema.json");

        let schema: Value = serde_json::from_str(CONTRACT).expect("contract is valid JSON");
        assert_eq!(schema["protocol"], HERDR_PROTOCOL_REVISION);
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
        let connector = herdr.engine();
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
            execute_pane_control(connector.as_ref(), &action).expect("control request");
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
        let connector = herdr.engine();
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
            let outcome =
                execute_pane_control(connector.as_ref(), &action).expect("control request");
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
            herdr.engine().as_ref(),
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
        let connector = herdr.engine();
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
        let mut outcomes = actions.iter().map(|action| {
            execute_remote_control(connector.as_ref(), action).expect("control request")
        });
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
        let engine = crate::engine::herdr::HerdrEngine::shared(Arc::clone(&connector));
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
            engine.as_ref(),
            &RemoteControlAction::FocusWorkspace {
                workspace_id: workspace_id.clone(),
            },
        )
        .expect("focus owned fixture workspace");
        execute_remote_control(
            engine.as_ref(),
            &RemoteControlAction::FocusTab {
                tab_id: original_tab_id,
            },
        )
        .expect("focus owned fixture tab");

        let RemoteControlOutcome::Acknowledged {
            created_tab_id: Some(created_tab_id),
            created_pane_id: Some(created_root_pane_id),
        } = execute_remote_control(
            engine.as_ref(),
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
            engine.as_ref(),
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
            execute_remote_control(engine.as_ref(), &action).expect("mutate only the fixture pane");
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
            engine: herdr.engine(),
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

            let result = execute_pane_focus(herdr.engine().as_ref(), "fixture:p2");
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
            let readback = confirm_pane_focus(herdr.engine().as_ref(), "fixture:p2");
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
                crate::engine::herdr::local(&socket).as_ref(),
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
        let payload = crate::engine::herdr::wire::project_snapshot(&snapshot).expect("projects");
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

        let payload =
            crate::engine::herdr::wire::project_snapshot(&snapshot).expect("projects tab metadata");
        assert_eq!(payload.tabs.len(), 1);
        assert_eq!(payload.tabs[0].tab_id, "w1:t1");
        assert_eq!(payload.tabs[0].label, "2");
    }

    #[test]
    fn protocol_mismatch_is_an_explicit_failure() {
        let snapshot = json!({"protocol": 20, "workspaces": [], "agents": []});
        let error = crate::engine::herdr::wire::project_snapshot(&snapshot).expect_err("must fail");
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

        let payload =
            crate::engine::herdr::wire::project_snapshot(&snapshot).expect("session projection");
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
        let error = fetch_session(
            crate::engine::herdr::local("/nonexistent/herdr-core-test.sock").as_ref(),
        )
        .expect_err("must fail");
        assert_eq!(error.state(), "socket_missing");
    }
}
