//! Live engine commands and pane byte transport. Session state synchronization
//! lives in `session_sync`, which reads the engine's whole session on each wake.

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
    /// the engine's global focus. Checkout navigation uses this to update directly
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
    /// The engine accepted the request and moved nothing: a zoom answered
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
    /// A new tab in a checkout whose owner the engine workspace is not open: the
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
    /// Asks the engine to put one of its tabs at a new place in its workspace.
    ///
    /// The engine owns the order of its own tabs, so this is a request, not a
    /// local edit: the strip keeps the order the engine last reported until the engine
    /// reports the new one. `expected_order` is the tab order the operator
    /// asked for *within the single engine workspace that owns the moved tab*,
    /// not across the checkout: a checkout can hold tabs from several
    /// workspaces, and `tab.move` answers with one workspace's list. It is
    /// carried so the acknowledgement can be checked against what was
    /// requested rather than assumed.
    MoveTab {
        checkout_id: String,
        tab_id: String,
        /// The engine's insertion index, which counts positions in the tab list
        /// *before* the moved tab is taken out of it.
        insert_index: usize,
        expected_order: Vec<String>,
        /// Which reorder this request belongs to, so a result that a later
        /// drag has already superseded cannot cancel the newer one.
        generation: u64,
        /// The connection that carried the request. A late response
        /// from an older connection cannot settle a move on a reconnected host.
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
    /// The workspace tab order the engine reported back, in the engine's order.
    TabsOrdered { tab_ids: Vec<String> },
}

/// A control result is either a known refusal or an answer that may have been
/// lost after the engine accepted the request. The runtime must never retry the
/// latter mutation without first reading fresh topology.
#[derive(Clone, Debug)]
pub(crate) enum ControlFailure {
    Definite(String),
    Ambiguous(String),
    /// The engine refused under its stable `code`; `message` says which
    /// operation and why.
    Refused {
        code: String,
        message: String,
    },
}

impl ControlFailure {
    pub(crate) fn is_ambiguous(&self) -> bool {
        matches!(self, Self::Ambiguous(_))
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Definite(message) | Self::Ambiguous(message) | Self::Refused { message, .. } => {
                message
            }
        }
    }

    /// The engine's refusal code, when the engine refused.
    pub(crate) fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code),
            Self::Definite(_) | Self::Ambiguous(_) => None,
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
            // new order, so the caller can check what the engine actually did
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
            // The user split to work in the new pane, so the engine focuses it as
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
            // The created tab is checked so a refusal the engine reported as an
            // unchanged move fails the action rather than reading as done.
            engine
                .move_pane_to_new_tab(CONTROL_WAIT, pane_id, workspace_id, label)
                .map_err(mutation_failure("pane.move"))?
                .created_tab()
                .map_err(ControlFailure::Ambiguous)?;
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
        EngineError::Refused { code, message } => ControlFailure::Refused {
            message: format!("{operation} was refused: {code}: {message}"),
            code,
        },
        EngineError::Request(message) => ControlFailure::Definite(message),
        EngineError::Answer(message) => ControlFailure::Ambiguous(message),
        other => ControlFailure::Ambiguous(format!("{operation} result is unknown: {other}")),
    }
}

/// `node` runs the pane (`agent_start::start_at_shell_checked`); none
/// when it cannot be reached. The pane's shell gets `shell_wait`, at most
/// `agent_start::SHELL_WAIT`, to finish starting; one that had not (stopped
/// at a question, not at its prompt, or not reading its line yet) is refused
/// as `agent_start::SHELL_STARTING` with the screen's last line when there
/// is one, so the spawn that asked can say what the shell waits for.
#[allow(clippy::too_many_arguments)] // the start identity and the pane's node
pub(crate) fn start_agent(
    engine: &dyn Engine,
    node: Option<&dyn crate::node_access::NodeLink>,
    request_id: &str,
    pane_id: &str,
    shell: crate::agent_start::ShellStart,
    shell_wait: Duration,
    name: &str,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
    program: Option<crate::engine::StartProgram>,
) -> Result<String, String> {
    start_agent_checked(
        engine,
        node,
        request_id,
        pane_id,
        shell,
        shell_wait,
        name,
        kind,
        args,
        codex_daemon,
        program,
        &|| Ok(()),
    )
    .map_err(|error| match error {
        crate::agent_start::StartError::ShellStarting { waited, screen } => {
            crate::coordination::shell_starting_refusal(waited, screen.as_deref())
        }
        // The engine refused the pane as busy until the start's deadline: nothing
        // was typed, and the same intent asks again in the same pane.
        crate::agent_start::StartError::Engine(EngineError::Refused { code, .. })
            if code == crate::agent_start::PANE_BUSY =>
        {
            PANE_BUSY_REFUSAL.to_owned()
        }
        other => start_error_message(other),
    })
}

/// A spawn's start that engine refused as busy for its whole wait.
pub(crate) const PANE_BUSY_REFUSAL: &str = "pane_busy";

#[allow(clippy::too_many_arguments)] // the existing start identity plus its effect admission
pub(crate) fn start_agent_checked(
    engine: &dyn Engine,
    node: Option<&dyn crate::node_access::NodeLink>,
    request_id: &str,
    pane_id: &str,
    shell: crate::agent_start::ShellStart,
    shell_wait: Duration,
    name: &str,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
    program: Option<crate::engine::StartProgram>,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<String, crate::agent_start::StartError> {
    let start = engine
        .prepare_start(crate::engine::AgentLaunch {
            pane_id: pane_id.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            args,
            codex_daemon,
            wakes: false,
            program,
        })
        .map_err(|detail| crate::agent_start::StartError::NotStarted {
            reason: "agent_start.failed".into(),
            detail,
        })?;
    match crate::agent_start::start_at_shell_within(
        engine,
        node,
        request_id,
        shell,
        &start,
        Duration::from_secs(5),
        shell_wait,
        check,
    ) {
        Ok(started) => Ok(started.pane_id),
        Err(crate::agent_start::StartError::Engine(EngineError::Answer(detail))) => {
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
        crate::agent_start::StartError::ShellStarting { waited, screen } => {
            crate::agent_start::shell_starting_message(waited, screen.as_deref())
        }
        crate::agent_start::StartError::Engine(error) => format!("agent.start failed: {error}"),
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
    let response_timeout = Duration::from_secs(5);
    // These three responses share the established absolute read deadline and
    // 64 KiB frame cap. A lost mutation response has a different outcome from
    // an authoritative read failing after the engine already acknowledged it.
    engine
        .focus_pane(Wait::Small(response_timeout), pane_id)
        .map_err(mutation_failure("pane.focus"))?;
    confirm_pane_focus_with_timeout(engine, pane_id, response_timeout)
}

/// The engine's answer to whether a pane holds its focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FocusReadback {
    /// The pane is focused in the tab its workspace shows.
    Confirmed,
    /// The engine's focus is on another pane or tab, so the move the stream showed
    /// is not the engine's focus now.
    Elsewhere,
    /// The engine could not be read; its reason.
    Unread(String),
}

/// Read current focus without producing another focus effect. Herdr's event
/// stream had no ordering cursor, so an external move needs this same
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
            "The engine did not confirm pane {pane_id}: pane {} is focused in tab {}, and workspace {} shows tab {active_tab_id}",
            layout.focused_pane_id, layout.tab_id, layout.workspace_id
        )));
    }
    Ok(layout)
}

/// The engine's layout of the tab holding `pane_id` and the tab its workspace
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
        .name("hide-core-control-lane".to_owned())
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
/// older connection cannot settle a newer request for the same pane.
pub fn spawn_pane_control_with_generation(
    context: LiveContext,
    action: PaneControlAction,
    connection_generation: Option<u64>,
) -> Result<(), String> {
    let worker_name = match &action {
        PaneControlAction::Project { .. } => "hide-core-pane-project".to_owned(),
        PaneControlAction::Focus { .. } => "hide-core-pane-focus".to_owned(),
        PaneControlAction::Split { direction, .. } => {
            format!("hide-core-pane-split-{}", direction.as_str())
        }
        PaneControlAction::Resize { direction, .. } => {
            format!("hide-core-pane-resize-{}", direction.as_str())
        }
        PaneControlAction::ToggleZoom { .. } => "hide-core-pane-zoom".to_owned(),
        PaneControlAction::Close { .. } => "hide-core-pane-close".to_owned(),
        PaneControlAction::MoveToNewTab { .. } => "hide-core-pane-move".to_owned(),
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
/// answers in the engine's place with its own code and [`SESSION_CHANGED`].
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

/// The message of the refusal [`current_session`] answers in the engine's place,
/// before anything reached the engine: its `code` is Hide's own check's.
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
            format!("hide-core-remote-{target_id}-pane-focus")
        }
        RemoteControlAction::Pane(PaneControlAction::Split { direction, .. }) => format!(
            "hide-core-remote-{target_id}-pane-split-{}",
            direction.as_str()
        ),
        RemoteControlAction::Pane(PaneControlAction::ToggleZoom { .. }) => {
            format!("hide-core-remote-{target_id}-pane-zoom")
        }
        RemoteControlAction::Pane(PaneControlAction::Close { .. }) => {
            format!("hide-core-remote-{target_id}-pane-close")
        }
        RemoteControlAction::Pane(PaneControlAction::MoveToNewTab { .. }) => {
            format!("hide-core-remote-{target_id}-pane-move")
        }
        RemoteControlAction::Pane(
            PaneControlAction::Project { .. } | PaneControlAction::Resize { .. },
        ) => return Err("unsupported remote pane control action".to_owned()),
        RemoteControlAction::FocusWorkspace { .. } => {
            format!("hide-core-remote-{target_id}-workspace-focus")
        }
        RemoteControlAction::FocusTab { .. } => {
            format!("hide-core-remote-{target_id}-tab-focus")
        }
        RemoteControlAction::CreateTab { .. } => {
            format!("hide-core-remote-{target_id}-tab-create")
        }
        RemoteControlAction::OpenOwner { .. } => {
            format!("hide-core-remote-{target_id}-owner-open")
        }
        RemoteControlAction::CloseTab { .. } => {
            format!("hide-core-remote-{target_id}-tab-close")
        }
        RemoteControlAction::RenameTab { .. } => {
            format!("hide-core-remote-{target_id}-tab-rename")
        }
        RemoteControlAction::MoveTab { .. } => {
            format!("hide-core-remote-{target_id}-tab-move")
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
    /// No engine runs where the core looked for it (`EngineError::NotRunning`).
    SocketMissing(String),
    /// The socket exists but the request failed (connect, timeout, IO).
    Unreachable(String),
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
        .ok_or_else(|| format!("Engine session has no layout containing pane {pane_id}"))?;
    project_layout(layout)
}

/// Every tab's layout in the session, in the order the engine sent them, plus the
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
        return Err(format!("Engine tab {} has no panes", layout.tab_id));
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
        return Err("Engine layout contains duplicate splits for one area".to_owned());
    }
    let Some(split) = matching_splits.first().copied() else {
        return match panes {
            [pane] => Ok(PaneLayoutNodeSnapshot::Pane {
                pane_id: pane.pane_id.clone(),
            }),
            [] => Err("Engine layout produced an empty leaf".to_owned()),
            _ => Err("Engine layout has multiple panes without an authoritative split".to_owned()),
        };
    };
    if !split.ratio.is_finite() || split.ratio <= 0.0 || split.ratio >= 1.0 {
        return Err(format!(
            "Engine layout split ratio {} is invalid",
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
        return Err("Engine layout split does not divide panes into two children".to_owned());
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
                return Err("The right split area is too narrow".to_owned());
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
                return Err("The down split area is too short".to_owned());
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
    use serde_json::Value;

    use super::*;
    use crate::engine::fake::{FakeEngine, refused, reply};

    #[test]
    fn tab_rename_uses_the_selected_host_and_reports_refusal() {
        let host = FakeEngine::start(|method, params| {
            assert_eq!(method, "rename_tab");
            assert_eq!(params["tab"], "w1:t1");
            if params["label"] == "refuse" {
                refused("not_found", "tab unavailable")
            } else {
                reply(())
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
        assert_eq!(host.methods(), ["rename_tab", "rename_tab", "rename_tab"]);
    }

    #[test]
    fn pane_control_asks_the_engine_for_every_mutation() {
        let engine = FakeEngine::start(|method, _| match method {
            "split_pane" => reply("w1:p2".to_owned()),
            "toggle_zoom" => reply(true),
            "close_pane" => reply(()),
            other => panic!("unexpected {other}"),
        });
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
            execute_pane_control(engine.engine().as_ref(), &action).expect("control request");
        }
        assert_eq!(
            engine.calls(),
            [
                (
                    "split_pane",
                    json!({"pane": "w1:p1", "direction": "Right", "cwd": "/tmp/hide-verify-shortcuts"}),
                ),
                (
                    "split_pane",
                    json!({"pane": "w1:p1", "direction": "Down", "cwd": null}),
                ),
                ("toggle_zoom", json!({"pane": "w1:p1"})),
                ("close_pane", json!({"pane": "w1:p1"})),
            ]
        );
    }

    /// A zoom of a lone pane and a resize with nothing to move change no
    /// layout, so both read as Unchanged and the operation can end at the
    /// answer (PRD instant-pane-topology D-12).
    #[test]
    fn a_zoom_or_resize_that_moved_nothing_reads_as_unchanged() {
        let engine = FakeEngine::start(|method, params| {
            let moved = params["pane"] == "w1:p2";
            match method {
                "toggle_zoom" | "resize_pane" => reply(moved),
                other => panic!("unexpected {other}"),
            }
        });
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
                execute_pane_control(engine.engine().as_ref(), &action).expect("control request");
            assert_eq!(
                matches!(outcome, PaneControlOutcome::Unchanged),
                unchanged,
                "{action:?} answered {outcome:?}"
            );
        }
    }

    #[test]
    fn remote_session_control_asks_the_engine_for_each_action() {
        let engine = FakeEngine::start(|method, _| match method {
            "focus_workspace" | "focus_tab" | "close_tab" => reply(()),
            "create_tab" => reply(("w1:t3".to_owned(), "w1:p3".to_owned())),
            other => panic!("unexpected {other}"),
        });
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
            execute_remote_control(engine.engine().as_ref(), action).expect("control request")
        });
        for created in [None, None, Some(("w1:t3", "w1:p3")), None] {
            match (outcomes.next(), created) {
                (
                    Some(RemoteControlOutcome::Acknowledged {
                        created_tab_id: None,
                        created_pane_id: None,
                    }),
                    None,
                ) => {}
                (
                    Some(RemoteControlOutcome::Acknowledged {
                        created_tab_id: Some(tab_id),
                        created_pane_id: Some(pane_id),
                    }),
                    Some((tab, pane)),
                ) if tab_id == tab && pane_id == pane => {}
                (other, _) => panic!("unexpected outcome {other:?}"),
            }
        }
        assert_eq!(
            engine.methods(),
            ["focus_workspace", "focus_tab", "create_tab", "close_tab"]
        );
        assert_eq!(engine.calls_of("focus_tab"), [json!({"tab": "w1:t2"})]);
        let created = &engine.calls_of("create_tab")[0];
        assert_eq!(created["workspace"], "w1");
        assert_eq!(created["cwd"], "/tmp/hide-remote-tab");
        assert_eq!(created["label"], "New tab");
        assert_eq!(created["focus"], true);
    }

    #[test]
    fn pane_control_worker_returns_before_the_engine_answers() {
        // The engine holds its answer until the spawn has returned, so a
        // spawn that waited for the answer would never return rather than
        // return late.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let engine = FakeEngine::start(move |method, _| {
            assert_eq!(method, "split_pane");
            held.recv_timeout(Duration::from_secs(30))
                .expect("the test lets the engine answer");
            reply("w1:p2".to_owned())
        });

        spawn_pane_control(
            engine.live(),
            PaneControlAction::Split {
                pane_id: "w1:p1".to_owned(),
                direction: PaneSplitDirection::Right,
                cwd: Some("/tmp".to_owned()),
            },
        )
        .expect("worker starts");

        // Returning at all is the proof: the engine is still holding its
        // answer. The worker still makes the call; the spawn only stopped
        // waiting for its answer.
        release.send(()).unwrap();
        engine.wait_for_calls(1, Duration::from_secs(5));
        assert_eq!(engine.methods(), ["split_pane"]);
    }

    fn layout(value: Value) -> SessionLayoutPayload {
        serde_json::from_value(value).expect("a layout")
    }

    #[test]
    fn focus_requires_authoritative_pane_and_owning_tab_confirmation() {
        for (focused_pane, active_tab, confirmed) in [
            ("fixture:p2", "fixture:t1", true),
            ("fixture:p1", "fixture:t1", false),
            ("fixture:p2", "fixture:t2", false),
        ] {
            let engine = FakeEngine::start(move |method, params| {
                if method == "workspace_active_tab" {
                    assert_eq!(params["workspace"], "fixture");
                } else {
                    assert_eq!(params["pane"], "fixture:p2");
                }
                match method {
                    "focus_pane" => reply(()),
                    "pane_layout" => reply(layout(json!({
                        "workspace_id": "fixture",
                        "tab_id": "fixture:t1",
                        "zoomed": false,
                        "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                        "focused_pane_id": focused_pane,
                        "panes": [
                            {"pane_id": "fixture:p1", "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                            {"pane_id": "fixture:p2", "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
                        ],
                        "splits": [
                            {"direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 120, "height": 60}}
                        ]
                    }))),
                    "workspace_active_tab" => reply(active_tab.to_owned()),
                    other => panic!("unexpected {other}"),
                }
            });

            let result = execute_pane_focus(engine.engine().as_ref(), "fixture:p2");
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
                engine.methods(),
                ["focus_pane", "pane_layout", "workspace_active_tab"]
            );
            // Checking a proposed stream move shares the confirmation
            // contract and must not produce another focus mutation.
            let readback = confirm_pane_focus(engine.engine().as_ref(), "fixture:p2");
            assert_eq!(
                readback,
                if confirmed {
                    FocusReadback::Confirmed
                } else {
                    FocusReadback::Elsewhere
                }
            );
            assert_eq!(
                engine.methods(),
                [
                    "focus_pane",
                    "pane_layout",
                    "workspace_active_tab",
                    "pane_layout",
                    "workspace_active_tab"
                ]
            );
        }
    }

    #[test]
    fn a_layout_projects_its_nested_tree_and_zoom() {
        let layout = project_layout(&layout(json!({
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "zoomed": true,
            "area": {"x": 0, "y": 0, "width": 120, "height": 60},
            "focused_pane_id": "w1:p3",
            "panes": [
                {"pane_id": "w1:p1", "rect": {"x": 0, "y": 0, "width": 60, "height": 60}},
                {"pane_id": "w1:p2", "rect": {"x": 60, "y": 0, "width": 60, "height": 30}},
                {"pane_id": "w1:p3", "rect": {"x": 60, "y": 30, "width": 60, "height": 30}}
            ],
            "splits": [
                {"direction": "right", "ratio": 0.5, "rect": {"x": 0, "y": 0, "width": 120, "height": 60}},
                {"direction": "down", "ratio": 0.5, "rect": {"x": 60, "y": 0, "width": 60, "height": 60}}
            ]
        })))
        .expect("layout projection");
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
    fn an_engine_that_is_not_running_is_distinguished_from_unreachable() {
        let error = fetch_session(FakeEngine::absent().engine().as_ref()).expect_err("must fail");
        assert_eq!(error.state(), "socket_missing");
    }
}
