//! The engine as Herdr: every [`Engine`] call as one request on Herdr's
//! socket API (`hide-herdr-client`), its parameters built and its answer
//! read by [`wire`], the only module that names the types generated from
//! the pinned Herdr schema. Nothing outside this module sends a Herdr
//! request.

use std::io::BufRead;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hide_herdr_client::{
    ApiConnector, ApiError, request_small_response, request_small_response_strict_until,
    request_with_connector,
};
use serde_json::Value;

use super::{
    AgentLaunch, AgentState, Answer, CreatedWorktree, DeliveryAgent, Engine, EngineError,
    EngineHandle, Env, EventLine, EventReader, EventStream, Gate, Notified, OpenedWorktree,
    PaneJoinResult, PanePlace, PaneProcessGroup, PaneProof, PaneSession, PaneText, PreparedStart,
    ServerPong, SharedEngine, StartedAgent, StopEvents, Wait, Zoom,
};
use crate::find::pane::PaneScroll;
use crate::live::{PaneResizeDirection, PaneSplitDirection};
use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{ProjectedAgent, ProjectionState};
use crate::sidebar::{SessionLayoutPayload, SessionSnapshotPayload};

pub(crate) mod contract;
// Only this module names Herdr's wire terms; a test elsewhere in the crate
// may build an answer with them.
#[cfg(not(test))]
mod wire;
#[cfg(test)]
pub(crate) mod wire;

/// Herdr's protocol revision this build speaks.
pub(crate) use hide_herdr_client::HERDR_PROTOCOL_REVISION;
pub(crate) use wire::device_protocol_behind;

/// The Herdr listening at `socket`, for a test.
#[cfg(test)]
pub(crate) fn local(socket: impl AsRef<std::path::Path>) -> SharedEngine {
    HerdrEngine::shared(Arc::new(hide_herdr_client::LocalSocketConnector::new(
        socket.as_ref(),
    )))
}

/// What `hided/src/herdr_handoff.rs` asks a Herdr server it is about to
/// replace, before any core or engine exists for it: the one reach into
/// Herdr's wire terms from outside this crate. Bundle 4 (the switch release)
/// deletes the handoff and this with it.
pub mod handoff {
    pub use super::wire::{live_handoff_done, live_handoff_params, ping_params, server_pong};
}

/// Why a server that answered `pong` cannot serve this build: its protocol
/// is not the one this build speaks.
pub(crate) fn protocol_refusal(pong: ServerPong) -> Option<crate::live::SessionFetchError> {
    let protocol = u64::from(pong.protocol);
    (protocol != HERDR_PROTOCOL_REVISION)
        .then(|| wire::protocol_mismatch(protocol, Some(pong.version)))
}

/// Decodes the snapshot a device's connection test fetched
/// (`hide_node_link::device::SnapshotCheck`): Herdr names no host in its
/// snapshot, so the device's host id and the socket it answered on are the
/// identity the envelope carries.
pub(crate) fn check_test_snapshot(
    value: &Value,
    host_id: &str,
    socket: &str,
) -> crate::remote::RemoteResult<u32> {
    let host = crate::domain::HostScope {
        host_id: host_id.to_owned(),
        session_id: socket.to_owned(),
    };
    wire::remote_snapshot(value, "remote-herdr-snapshot", &host).map(|snapshot| snapshot.protocol)
}

/// The events the core's session sync follows: every change to workspaces,
/// tabs, panes, layouts and agents.
const TOPOLOGY_EVENTS: &[&str] = &[
    "workspace.created",
    "workspace.updated",
    "workspace.metadata_updated",
    "workspace.renamed",
    "workspace.moved",
    "workspace.reordered",
    "workspace.closed",
    "workspace.focused",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.renamed",
    "tab.moved",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.focused",
    "pane.moved",
    "pane.exited",
    "pane.agent_detected",
    "layout.updated",
];

/// How long one stage of a pane's proof may take within the caller's
/// deadline.
const PROOF_STAGE: Duration = Duration::from_secs(2);

/// One Herdr server, reached through `connector`: this machine's socket, or
/// a device's through its link.
pub(crate) struct HerdrEngine<'a> {
    connector: Arc<dyn ApiConnector + 'a>,
    /// The same connection where it outlives every borrow, for the hook
    /// token restorer; none on a gated engine. Temporary: hooks-status
    /// deletes it with the restorer.
    link: Option<Arc<dyn ApiConnector>>,
}

impl HerdrEngine<'static> {
    pub(crate) fn new(connector: Arc<dyn ApiConnector>) -> Self {
        Self {
            connector: Arc::clone(&connector),
            link: Some(connector),
        }
    }

    pub(crate) fn shared(connector: Arc<dyn ApiConnector>) -> SharedEngine {
        Arc::new(Self::new(connector))
    }
}

impl HerdrEngine<'_> {
    /// Sends `method` with the parameters `params` built, and reads the
    /// answer within `wait`.
    fn ask(
        &self,
        method: &str,
        params: Result<Value, String>,
        wait: Wait,
    ) -> Result<Value, EngineError> {
        let params = params.map_err(EngineError::Request)?;
        let connector = self.connector.as_ref();
        match wait {
            Wait::Each(timeout) => request_with_connector(connector, method, params, timeout),
            Wait::Small(timeout) => request_small_response(connector, method, params, timeout),
            Wait::By(deadline) => {
                request_small_response_strict_until(connector, method, params, deadline)
            }
        }
        .map_err(from_api)
    }

    /// [`Self::ask`], with the answer read by `read`.
    fn call<T>(
        &self,
        method: &str,
        params: Result<Value, String>,
        wait: Wait,
        read: impl FnOnce(Value) -> Result<T, String>,
    ) -> Answer<T> {
        read(self.ask(method, params, wait)?).map_err(EngineError::Answer)
    }

    /// [`Self::ask`] for a call whose answer says nothing beyond success.
    fn act(&self, method: &str, params: Result<Value, String>, wait: Wait) -> Answer<()> {
        self.ask(method, params, wait).map(drop)
    }
}

fn start_params(launch: &AgentLaunch) -> Result<Value, String> {
    wire::agent_start_params(
        &launch.pane_id,
        &launch.name,
        &launch.kind,
        launch.args.clone(),
        launch.codex_daemon,
    )
}

/// A connection whose every request first passes a [`Gate`]: before it
/// connects, and again once connected, before the request is written.
struct GatedConnector<'a> {
    inner: Arc<dyn ApiConnector + 'a>,
    gate: Gate<'a>,
}

impl ApiConnector for GatedConnector<'_> {
    fn connect(&self) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        (self.gate.before)().map_err(into_api)?;
        let stream = self.inner.connect()?;
        (self.gate.at_send)().map_err(into_api)?;
        Ok(stream)
    }

    fn connect_until(
        &self,
        deadline: Instant,
    ) -> Result<Box<dyn hide_herdr_client::ApiStream>, ApiError> {
        (self.gate.before)().map_err(into_api)?;
        let stream = self.inner.connect_until(deadline)?;
        (self.gate.at_send)().map_err(into_api)?;
        Ok(stream)
    }
}

/// A gate's refusal in the transport's words, which [`from_api`] reads back
/// as the same refusal; one with no word of its own there is a transport
/// failure with its message.
fn into_api(error: EngineError) -> ApiError {
    match error {
        EngineError::NotRunning(message) => ApiError::NotRunning(message),
        EngineError::Refused { code, message } => ApiError::Remote { code, message },
        EngineError::Malformed(message) => ApiError::Malformed(message),
        other => ApiError::Transport(other.to_string()),
    }
}

/// The engine boundary's words for a failure of Herdr's transport.
pub(crate) fn from_api(error: ApiError) -> EngineError {
    match error {
        ApiError::NotRunning(message) => EngineError::NotRunning(message),
        ApiError::Transport(message) => EngineError::Transport(message),
        ApiError::Remote { code, message } => EngineError::Refused { code, message },
        ApiError::Malformed(message) => EngineError::Malformed(message),
    }
}

/// A snapshot that could not be read as one, in the engine boundary's
/// words: the protocol, or a shape the call does not promise.
fn from_snapshot(error: crate::live::SessionFetchError) -> EngineError {
    match error {
        crate::live::SessionFetchError::Protocol {
            message,
            expected_protocol,
            received_protocol,
            received_version,
        } => EngineError::Protocol {
            message,
            expected_protocol,
            received_protocol,
            received_version,
        },
        other => EngineError::Answer(other.message().to_owned()),
    }
}

/// A stage of a pane's proof ends by the caller's deadline and within
/// [`PROOF_STAGE`].
fn proof_stage(wait: Wait) -> Answer<Wait> {
    let deadline = match wait {
        Wait::By(deadline) => deadline,
        other => {
            return Err(EngineError::Request(format!(
                "a pane's proof takes one deadline, not {other:?}"
            )));
        }
    };
    let now = Instant::now();
    if deadline <= now {
        return Err(EngineError::Transport("pane_inspection_expired".into()));
    }
    Ok(Wait::By(deadline.min(now + PROOF_STAGE)))
}

struct HerdrEvents {
    reader: Box<dyn BufRead + Send>,
}

impl EventReader for HerdrEvents {
    fn next_event(&mut self) -> Result<Option<EventLine>, String> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(wire::parse_subscription_line(&line))),
            Err(error) => Err(format!("socket read failed: {error}")),
        }
    }
}

struct HerdrStop(Box<dyn hide_herdr_client::ConnectionShutdown>);

impl StopEvents for HerdrStop {
    fn stop(&self) {
        self.0.shutdown();
    }
}

impl Engine for HerdrEngine<'_> {
    fn gated<'g>(&'g self, gate: Gate<'g>) -> Box<dyn Engine + 'g> {
        Box::new(HerdrEngine {
            connector: Arc::new(GatedConnector {
                inner: Arc::clone(&self.connector),
                gate,
            }),
            link: None,
        })
    }

    fn hook_token_link(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.link
            .clone()
            .map(|link| Arc::new(link) as Arc<dyn std::any::Any + Send + Sync>)
    }

    fn session(&self, wait: Wait) -> Answer<ProjectionState> {
        let answer = self.ask("session.snapshot", Ok(wire::empty_params()), wait)?;
        wire::snapshot_response(answer).map_err(from_snapshot)
    }

    fn session_payload(&self, wait: Wait) -> Answer<SessionSnapshotPayload> {
        let answer = self.ask("session.snapshot", Ok(wire::empty_params()), wait)?;
        wire::live_session_response(answer).map_err(from_snapshot)
    }

    fn pane_folders(&self, wait: Wait) -> Answer<Vec<(String, Option<String>)>> {
        let answer = self.ask("session.snapshot", Ok(wire::empty_params()), wait)?;
        wire::cleanup_usage_paths(answer).map_err(from_snapshot)
    }

    fn pane_places(&self, wait: Wait) -> Answer<Vec<PanePlace>> {
        self.call(
            "session.snapshot",
            Ok(wire::empty_params()),
            wait,
            wire::pane_places,
        )
    }

    fn agents(&self, wait: Wait) -> Answer<Vec<ProjectedAgent>> {
        let answer = self.ask("agent.list", Ok(wire::empty_params()), wait)?;
        wire::agents_response(answer).map_err(from_snapshot)
    }

    fn ping(&self, wait: Wait) -> Answer<ServerPong> {
        self.call("ping", wire::ping_params(), wait, wire::server_pong)
    }

    fn subscribe(&self, timeout: Duration) -> Answer<EventStream> {
        let params = wire::subscription_params(TOPOLOGY_EVENTS).map_err(EngineError::Request)?;
        let subscription =
            hide_herdr_client::subscribe_with_connector(self.connector.as_ref(), params, timeout)
                .map_err(from_api)?;
        let (reader, shutdown) = subscription.into_parts();
        Ok(EventStream {
            reader: Box::new(HerdrEvents { reader }),
            stopper: Box::new(HerdrStop(shutdown)),
        })
    }

    fn create_workspace(
        &self,
        wait: Wait,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String, String)> {
        self.call(
            "workspace.create",
            wire::workspace_create_with_env_params(cwd, label, focus, env),
            wait,
            wire::created_workspace,
        )
    }

    fn workspace_active_tab(&self, wait: Wait, workspace: &str) -> Answer<String> {
        self.call(
            "workspace.get",
            wire::workspace_target_params(workspace),
            wait,
            wire::workspace_active_tab,
        )
    }

    fn focused_workspace(&self, wait: Wait) -> Answer<Option<String>> {
        self.call(
            "workspace.list",
            wire::workspace_list_params(),
            wait,
            wire::listed_focused_workspace,
        )
    }

    fn workspace_tokens(&self, wait: Wait, token: &str) -> Answer<Vec<(String, Option<String>)>> {
        self.call(
            "workspace.list",
            wire::workspace_list_params(),
            wait,
            |answer| wire::listed_workspace_tokens(answer, token),
        )
    }

    fn rename_workspace(&self, wait: Wait, workspace: &str, label: &str) -> Answer<()> {
        self.act(
            "workspace.rename",
            wire::workspace_rename_params(workspace, label),
            wait,
        )
    }

    fn focus_workspace(&self, wait: Wait, workspace: &str) -> Answer<()> {
        self.act(
            "workspace.focus",
            wire::workspace_target_params(workspace),
            wait,
        )
    }

    fn report_workspace_token(
        &self,
        wait: Wait,
        workspace: &str,
        token: &str,
        value: Option<&str>,
    ) -> Answer<()> {
        self.act(
            "workspace.report_metadata",
            wire::workspace_metadata_params(workspace, token, value),
            wait,
        )
    }

    fn create_tab(
        &self,
        wait: Wait,
        workspace: &str,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String)> {
        self.call(
            "tab.create",
            wire::tab_create_with_env_params(workspace, cwd, label, focus, env),
            wait,
            wire::created_tab,
        )
    }

    fn create_replacement_tab(
        &self,
        wait: Wait,
        workspace: &str,
        cwd: &str,
        env: Env,
    ) -> Answer<(String, String)> {
        self.call(
            "tab.create",
            wire::replacement_tab_params(workspace, cwd, env),
            wait,
            wire::created_tab,
        )
    }

    fn launch_tab(
        &self,
        wait: Wait,
        workspace: &str,
        replace: Option<&str>,
        label: &str,
        focus: bool,
        cwd: &str,
        env: Env,
        command: &[String],
    ) -> Answer<(String, String)> {
        self.call(
            "layout.apply",
            wire::launched_tab_params(workspace, replace, label, focus, cwd, env, command),
            wait,
            wire::launched_tab,
        )
    }

    fn launch_checkout_tab(
        &self,
        wait: Wait,
        workspace: &str,
        seed_tab: Option<&str>,
        cwd: &str,
        label: &str,
        env: Env,
        command: Option<&[String]>,
        claim: &str,
    ) -> Answer<(String, String)> {
        self.call(
            "layout.apply",
            wire::checkout_start_tab_params(workspace, seed_tab, cwd, label, env, command, claim),
            wait,
            wire::launched_tab,
        )
    }

    fn close_tab(&self, wait: Wait, tab: &str) -> Answer<()> {
        self.act("tab.close", wire::tab_target_params(tab), wait)
    }

    fn focus_tab(&self, wait: Wait, tab: &str) -> Answer<()> {
        self.act("tab.focus", wire::tab_target_params(tab), wait)
    }

    fn rename_tab(&self, wait: Wait, tab: &str, label: &str) -> Answer<()> {
        self.act("tab.rename", wire::tab_rename_params(tab, label), wait)
    }

    fn move_tab(&self, wait: Wait, tab: &str, index: usize) -> Answer<Vec<String>> {
        self.call(
            "tab.move",
            wire::tab_move_params(tab, index),
            wait,
            wire::moved_tabs,
        )
    }

    fn pane_layout(&self, wait: Wait, pane: &str) -> Answer<SessionLayoutPayload> {
        self.call(
            "pane.layout",
            wire::pane_layout_params(pane),
            wait,
            wire::pane_layout,
        )
    }

    fn export_layout(&self, wait: Wait, tab: &str) -> Answer<ClosedLayout> {
        self.call(
            "layout.export",
            wire::layout_export_params(tab),
            wait,
            wire::exported_layout,
        )
    }

    fn apply_layout(
        &self,
        wait: Wait,
        workspace: &str,
        tab: Option<&str>,
        tab_label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout> {
        self.call(
            "layout.apply",
            wire::layout_apply_params(workspace, tab, tab_label, root),
            wait,
            wire::applied_layout,
        )
    }

    fn split_pane(
        &self,
        wait: Wait,
        pane: &str,
        direction: PaneSplitDirection,
        cwd: Option<&str>,
    ) -> Answer<String> {
        self.call(
            "pane.split",
            wire::pane_split_params(pane, direction, cwd),
            wait,
            wire::split_pane,
        )
    }

    fn split_pane_at(
        &self,
        wait: Wait,
        pane: &str,
        direction: ClosedSplitDirection,
        cwd: &str,
        ratio: f32,
        env: Env,
    ) -> Answer<String> {
        self.call(
            "pane.split",
            wire::pane_split_with_ratio_params(pane, direction, cwd, ratio, env),
            wait,
            wire::split_pane,
        )
    }

    fn split_beside(
        &self,
        wait: Wait,
        parent: &str,
        cwd: Option<&str>,
        env: Env,
    ) -> Answer<String> {
        self.call(
            "pane.split",
            wire::fork_split_params(parent, cwd, env),
            wait,
            wire::split_pane,
        )
    }

    fn split_replacement(&self, wait: Wait, pane: &str, cwd: &str, env: Env) -> Answer<String> {
        self.call(
            "pane.split",
            wire::replacement_pane_params(pane, cwd, env),
            wait,
            wire::split_pane,
        )
    }

    fn close_pane(&self, wait: Wait, pane: &str) -> Answer<()> {
        self.act("pane.close", wire::pane_target_params(pane), wait)
    }

    fn focus_pane(&self, wait: Wait, pane: &str) -> Answer<()> {
        self.act("pane.focus", wire::pane_target_params(pane), wait)
    }

    fn rename_pane(&self, wait: Wait, pane: &str, label: Option<&str>) -> Answer<()> {
        self.act("pane.rename", wire::pane_rename_params(pane, label), wait)
    }

    fn toggle_zoom(&self, wait: Wait, pane: &str) -> Answer<bool> {
        self.call(
            "pane.zoom",
            wire::pane_zoom_params(pane),
            wait,
            wire::pane_zoom_changed,
        )
    }

    fn set_zoom(&self, wait: Wait, pane: &str, zoom: Zoom) -> Answer<()> {
        let params = match zoom {
            Zoom::On => wire::pane_restore_zoom_params(pane),
            Zoom::Off => wire::pane_unzoom_params(pane),
        };
        self.act("pane.zoom", params, wait)
    }

    fn resize_pane(
        &self,
        wait: Wait,
        pane: &str,
        direction: PaneResizeDirection,
        amount: f32,
    ) -> Answer<bool> {
        self.call(
            "pane.resize",
            wire::pane_resize_params(pane, direction, amount),
            wait,
            wire::pane_resize_changed,
        )
    }

    fn swap_panes(&self, wait: Wait, first: &str, second: &str) -> Answer<()> {
        self.act("pane.swap", wire::pane_swap_params(first, second), wait)
    }

    fn move_pane_to_new_tab(
        &self,
        wait: Wait,
        pane: &str,
        workspace: &str,
        label: &str,
    ) -> Answer<PaneJoinResult> {
        self.call(
            "pane.move",
            wire::pane_move_to_new_tab_params(pane, workspace, label),
            wait,
            wire::joined_pane,
        )
    }

    fn move_pane_beside(
        &self,
        wait: Wait,
        pane: &str,
        target_tab: &str,
        target_pane: &str,
        down: bool,
    ) -> Answer<PaneJoinResult> {
        self.call(
            "pane.move",
            wire::pane_insert_params(pane, target_tab, target_pane, down),
            wait,
            wire::joined_pane,
        )
    }

    fn pane_session(
        &self,
        wait: Wait,
        pane: &str,
    ) -> Answer<(Option<String>, Option<PaneSession>)> {
        self.call(
            "pane.get",
            wire::pane_target_params(pane),
            wait,
            wire::pane_session,
        )
    }

    fn pane_terminal(&self, wait: Wait, pane: &str) -> Answer<String> {
        self.call(
            "pane.get",
            wire::pane_target_params(pane),
            wait,
            wire::pane_terminal,
        )
    }

    fn pane_token(&self, wait: Wait, pane: &str, token: &str) -> Answer<Option<String>> {
        self.call("pane.get", wire::pane_target_params(pane), wait, |answer| {
            wire::pane_token(answer, token)
        })
    }

    fn pane_agent_status(&self, wait: Wait, pane: &str) -> Answer<String> {
        self.call(
            "pane.get",
            wire::pane_target_params(pane),
            wait,
            wire::pane_agent_status,
        )
    }

    fn pane_scroll(&self, wait: Wait, pane: &str) -> Answer<Option<PaneScroll>> {
        self.call(
            "pane.get",
            wire::pane_target_params(pane),
            wait,
            wire::pane_scroll,
        )
    }

    fn scroll_pane(
        &self,
        wait: Wait,
        pane: &str,
        offset_from_bottom: u64,
    ) -> Answer<Option<PaneScroll>> {
        self.call(
            "pane.scroll",
            wire::pane_scroll_params(pane, offset_from_bottom),
            wait,
            wire::pane_scroll,
        )
    }

    fn report_pane_tokens(
        &self,
        wait: Wait,
        pane: &str,
        tokens: &std::collections::BTreeMap<String, Option<String>>,
    ) -> Answer<()> {
        self.act(
            "pane.report_metadata",
            wire::pane_metadata_params(pane, tokens),
            wait,
        )
    }

    fn report_agent_session(
        &self,
        wait: Wait,
        pane: &str,
        kind: &str,
        session: &str,
        seq: u64,
    ) -> Answer<()> {
        self.act(
            "pane.report_agent_session",
            wire::report_agent_session_params(pane, kind, session, seq),
            wait,
        )
    }

    fn read_pane(&self, wait: Wait, pane: &str, source: &str, lines: u32) -> Answer<PaneText> {
        self.call(
            "pane.read",
            wire::pane_read_params(pane, source, lines),
            wait,
            wire::pane_text,
        )
    }

    fn read_screen(&self, wait: Wait, pane: &str) -> Answer<PaneText> {
        self.call(
            "pane.read",
            wire::composer_screen_params(pane),
            wait,
            wire::pane_text,
        )
    }

    fn read_rows(&self, wait: Wait, pane: &str, lines: u32) -> Answer<PaneText> {
        self.call(
            "pane.read",
            wire::pane_rows_params(pane, lines),
            wait,
            wire::pane_text,
        )
    }

    fn send_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.act(
            "pane.send_text",
            wire::pane_send_text_params(pane, text),
            wait,
        )
    }

    fn send_keys(&self, wait: Wait, pane: &str, keys: &[&str]) -> Answer<()> {
        self.act(
            "pane.send_keys",
            wire::pane_send_keys_params(pane, keys),
            wait,
        )
    }

    fn submit_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.act(
            "pane.send_input",
            wire::delivery_input_params(pane, text),
            wait,
        )
    }

    fn press_key(&self, wait: Wait, pane: &str, key: &str) -> Answer<()> {
        self.act("pane.send_input", wire::pane_key_params(pane, key), wait)
    }

    fn notify(&self, wait: Wait, title: &str, body: &str) -> Answer<Notified> {
        let answer = self.ask(
            "notification.show",
            Ok(wire::notification_params(title, body)),
            wait,
        )?;
        Ok(wire::notification_shown(&answer))
    }

    fn pane_processes(&self, wait: Wait, pane: &str) -> Answer<PaneProcessGroup> {
        self.call(
            "pane.process_info",
            wire::pane_process_info_params(pane),
            wait,
            wire::pane_process_group,
        )
    }

    fn foreground_program(&self, wait: Wait, pane: &str) -> Answer<Option<String>> {
        self.call(
            "pane.process_info",
            wire::pane_process_info_params(pane),
            wait,
            |answer| wire::foreground_process(answer, pane),
        )
    }

    fn resumed_session(
        &self,
        wait: Wait,
        pane: &str,
        agent: hide_session::Agent,
    ) -> Answer<Option<(String, String)>> {
        self.call(
            "pane.process_info",
            wire::pane_process_info_params(pane),
            wait,
            |answer| wire::resumed_session_reference(answer, pane, agent),
        )
    }

    fn current_pane(&self, wait: Wait, pane: &str) -> Answer<String> {
        self.call(
            "pane.current",
            wire::pane_current_params(pane),
            proof_stage(wait)?,
            wire::current_pane,
        )
    }

    fn pane_identity(&self, wait: Wait, pane: &str) -> Answer<PaneProof> {
        let current = self.current_pane(wait, pane)?;
        let shell_pid = self.call(
            "pane.process_info",
            wire::pane_process_info_params(&current),
            proof_stage(wait)?,
            wire::pane_shell_pid,
        )?;
        let terminal_id = self.pane_terminal(proof_stage(wait)?, &current)?;
        Ok(PaneProof {
            pane_id: current,
            terminal_id,
            shell_pid,
        })
    }

    fn prepare_start(&self, launch: AgentLaunch) -> Result<PreparedStart, String> {
        let line = wire::agent_start_line_bytes(&start_params(&launch)?);
        Ok(PreparedStart::new(launch, line))
    }

    fn start_agent(&self, wait: Wait, start: &PreparedStart) -> Answer<StartedAgent> {
        self.call(
            "agent.start",
            start_params(start.launch()),
            wait,
            |answer| {
                Ok(StartedAgent {
                    terminal_id: wire::started_agent_terminal(&answer),
                    pane_id: wire::started_agent(answer)?,
                })
            },
        )
    }

    fn agent_state(&self, wait: Wait, target: &str) -> Answer<AgentState> {
        self.call(
            "agent.get",
            wire::agent_target_params(target),
            wait,
            wire::agent_state,
        )
    }

    fn delivery_agent(&self, wait: Wait, target: &str) -> Answer<DeliveryAgent> {
        self.call(
            "agent.get",
            wire::agent_target_params(target),
            wait,
            wire::delivery_agent,
        )
    }

    fn observed_agent(
        &self,
        wait: Wait,
        target: &str,
    ) -> Answer<(
        DeliveryAgent,
        Option<hide_agent_hooks::observation::ObservationNativeSession>,
    )> {
        self.call(
            "agent.get",
            wire::agent_target_params(target),
            wait,
            wire::direct_observation_agent,
        )
    }

    fn prompt_agent(&self, wait: Wait, target: &str, text: &str) -> Answer<()> {
        self.call(
            "agent.prompt",
            wire::delivery_prompt_params(target, text),
            wait,
            wire::delivery_prompted,
        )
    }

    fn create_worktree(
        &self,
        wait: Wait,
        cwd: &str,
        branch: &str,
        base: Option<&str>,
        label: Option<&str>,
        path: Option<&str>,
        focus: bool,
    ) -> Answer<CreatedWorktree> {
        self.call(
            "worktree.create",
            wire::worktree_create_params(cwd, branch, base, label, path, focus),
            wait,
            wire::created_worktree,
        )
    }

    fn worktree_path(&self, wait: Wait, cwd: &str, branch: &str) -> Answer<Option<String>> {
        self.call(
            "worktree.list",
            wire::worktree_list_params(cwd),
            wait,
            |answer| wire::listed_worktree_path(answer, branch),
        )
    }

    fn open_worktree(
        &self,
        wait: Wait,
        path: &str,
        repository_root: &str,
        focus: bool,
    ) -> Answer<OpenedWorktree> {
        self.call(
            "worktree.open",
            wire::worktree_open_params(path, repository_root, focus),
            wait,
            wire::opened_worktree,
        )
    }

    fn remove_worktree(&self, wait: Wait, workspace: &str) -> Answer<()> {
        self.act(
            "worktree.remove",
            wire::worktree_remove_params(workspace),
            wait,
        )
    }
}

/// The Herdr connection under `engine`, for the hook token restorer
/// (`coordination::hook_tokens`), which still reports a hook's tokens itself.
/// Temporary: hooks-status deletes the restorer and this with it (PRD
/// engine-bundle-1 D-8).
pub(crate) fn hook_token_connector(engine: &SharedEngine) -> Option<Arc<dyn ApiConnector>> {
    engine
        .hook_token_link()?
        .downcast::<Arc<dyn ApiConnector>>()
        .ok()
        .map(|link| Arc::clone(link.as_ref()))
}

impl EngineHandle {
    /// The Herdr server `connector` reaches: this machine's through its node
    /// (`hide_node::herdr`), which owns the socket.
    pub fn herdr(connector: Arc<dyn ApiConnector>) -> Self {
        Self::new(HerdrEngine::shared(connector))
    }
}
