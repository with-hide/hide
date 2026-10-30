//! The engine boundary: the one way the core speaks to the program that runs
//! its panes (PRD engine-bundle-1 D-2). Pane existence, split geometry, zoom,
//! cwd, agent lifecycle and the PTY are the engine's; the core asks for them
//! through [`Engine`] and never names the engine's protocol. The one engine
//! today is Herdr ([`herdr::HerdrEngine`]), and only that module speaks
//! Herdr's socket API and its generated wire types; the CI check
//! `scripts/check-engine-boundary.py` keeps it so.
//!
//! Terminal bytes (`TerminalRoutes`) and a machine's own work (`NodeLink`)
//! are already outside this boundary and do not pass through it.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) mod answers;
#[cfg(test)]
mod contract;
pub mod herdr;
pub(crate) mod tokens;

/// How long a call may wait for the engine and how much of an answer it
/// takes. Each caller keeps the bound its operation was measured with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Wait {
    /// Connecting, sending and reading the answer each wait up to this long;
    /// the answer may be any size.
    Each(Duration),
    /// A small answer (64 KiB at most), sent and read within this long;
    /// connecting keeps the transport's own bound.
    Small(Duration),
    /// A small answer whose connection, sending and reading all end by this
    /// instant. A transport that cannot bound its connection refuses before
    /// it starts one.
    By(Instant),
}

/// Why a call to the engine did not answer as asked. The kinds are the ones
/// callers branch on: nothing ran, the engine refused, or the call was sent
/// and its outcome is unknown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// No engine runs where the core looked for it.
    NotRunning(String),
    /// The engine could not be reached or the call could not be sent:
    /// nothing was done.
    Transport(String),
    /// The call was sent and its answer never came: a read that timed out or
    /// a connection that closed unanswered. The engine may have done what was
    /// asked, so a caller that made a change reconciles before it asks again.
    Unanswered(String),
    /// The engine answered with a refusal, under its stable code.
    Refused { code: String, message: String },
    /// The engine's answer could not be read as an answer at all.
    Malformed(String),
    /// The request could not be put into the engine's terms; nothing was
    /// sent.
    Request(String),
    /// The engine answered, but not with what the call's contract promises.
    Answer(String),
    /// The engine speaks a protocol this build does not; the message says
    /// what the operator can do about it.
    Protocol {
        message: String,
        expected_protocol: u64,
        received_protocol: u64,
        received_version: Option<String>,
    },
}

impl EngineError {
    /// The engine's refusal code, when it refused.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code),
            _ => None,
        }
    }

    /// Whether the call was sent and its answer never came
    /// ([`EngineError::Unanswered`]); a refused connect or send did nothing.
    pub fn reply_unknown(&self) -> bool {
        matches!(self, Self::Unanswered(_))
    }

    /// Whether the engine was reached and said something: a refusal, or an
    /// answer this call could not use. Nothing about the transport failed.
    pub fn answered(&self) -> bool {
        matches!(
            self,
            Self::Refused { .. } | Self::Answer(_) | Self::Protocol { .. }
        )
    }

    /// The text of the failure, as `Display` writes it.
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// `context` before a failure of the call itself, and nothing before a
    /// request or answer that did not fit the call's contract: the wording
    /// the core used before the engine boundary, kept so a reader of the log
    /// or the screen sees the same sentence.
    pub(crate) fn within(self, context: &str) -> String {
        match self {
            Self::Request(message) | Self::Answer(message) => message,
            other => format!("{context}: {other}"),
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning(message)
            | Self::Transport(message)
            | Self::Unanswered(message)
            | Self::Malformed(message)
            | Self::Request(message)
            | Self::Answer(message)
            | Self::Protocol { message, .. } => formatter.write_str(message),
            Self::Refused { code, message } => write!(formatter, "{code}: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<EngineError> for String {
    fn from(error: EngineError) -> Self {
        error.to_string()
    }
}

/// The engine the core holds for one machine, shared by every worker that
/// asks it something. Two handles are the same engine when they share the
/// allocation: a coordinator compares its own with the one the runtime holds
/// now, so a replaced connection retires the readers of the old one.
pub(crate) type SharedEngine = Arc<dyn Engine>;

use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{ProjectedAgent, ProjectionState};
use crate::sidebar::{SessionLayoutPayload, SessionSnapshotPayload};

/// Whether two handles name the same engine connection.
pub(crate) fn same_engine(left: &SharedEngine, right: &SharedEngine) -> bool {
    Arc::ptr_eq(left, right)
}

pub(crate) use answers::*;
pub use answers::{PaneProof, PaneSession, PaneSessionKind, PaneText, ServerPong};

/// The environment a new pane's shell starts with, beside its own.
pub(crate) type Env = BTreeMap<String, String>;

/// One call's answer.
pub(crate) type Answer<T> = Result<T, EngineError>;

/// What the core asks the engine. Every method is one call: it blocks until
/// the engine answers or `wait` ends, and none is ever made under the
/// runtime lock. A method that changes something says what changed in its
/// answer. A call that was sent and never answered fails with
/// [`EngineError::Unanswered`], never with another kind, and a caller that
/// met it reads the engine again before it asks for the change twice.
pub(crate) trait Engine: Send + Sync {
    /// This engine, with every call first passing `gate`.
    fn gated<'a>(&'a self, gate: Gate<'a>) -> Box<dyn Engine + 'a>;

    /// The connection the hook token restorer (`coordination::hook_tokens`)
    /// still reports a hook's tokens on itself, for an engine that has one.
    /// Temporary: hooks-status deletes the restorer and this with it (PRD
    /// engine-bundle-1 D-8).
    fn hook_token_link(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        None
    }

    // The session as a whole.

    /// Every workspace, tab, pane, layout and agent, with the focus.
    fn session(&self, wait: Wait) -> Answer<ProjectionState>;
    /// The same session as the sidebar's payload.
    fn session_payload(&self, wait: Wait) -> Answer<SessionSnapshotPayload>;
    /// Each pane with the folders it works in: its own and its foreground
    /// program's, or none when the engine reports neither.
    fn pane_folders(&self, wait: Wait) -> Answer<Vec<(String, Option<String>)>>;
    /// Every pane with its folders as reported, unconverted: what a close
    /// is confirmed against, where a missing pane id is never read as a pane
    /// gone.
    fn pane_places(&self, wait: Wait) -> Answer<Vec<PanePlace>>;
    /// Every agent and its state.
    fn agents(&self, wait: Wait) -> Answer<Vec<ProjectedAgent>>;
    /// The engine's version and protocol.
    fn ping(&self, wait: Wait) -> Answer<ServerPong>;
    /// The changes to the session from now on, each workspace, tab, pane,
    /// layout and agent event as it happens. `timeout` bounds the
    /// subscription's acknowledgement; the stream itself waits.
    fn subscribe(&self, timeout: Duration) -> Answer<EventStream>;

    // Workspaces.

    /// A new workspace in `cwd`: its id, its first tab and that tab's pane.
    fn create_workspace(
        &self,
        wait: Wait,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String, String)>;
    /// The tab a workspace shows.
    fn workspace_active_tab(&self, wait: Wait, workspace: &str) -> Answer<String>;
    /// The workspace that holds the keyboard focus, if any.
    fn focused_workspace(&self, wait: Wait) -> Answer<Option<String>>;
    /// Each workspace with the value of its metadata `token`.
    fn workspace_tokens(&self, wait: Wait, token: &str) -> Answer<Vec<(String, Option<String>)>>;
    fn rename_workspace(&self, wait: Wait, workspace: &str, label: &str) -> Answer<()>;
    fn focus_workspace(&self, wait: Wait, workspace: &str) -> Answer<()>;
    /// Sets Hide's metadata `token` on a workspace; `None` clears it.
    fn report_workspace_token(
        &self,
        wait: Wait,
        workspace: &str,
        token: &str,
        value: Option<&str>,
    ) -> Answer<()>;

    // Tabs.

    /// A new tab in `workspace`: its id and its pane.
    fn create_tab(
        &self,
        wait: Wait,
        workspace: &str,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String)>;
    /// A new unfocused, unnamed tab in `workspace` that stands in for one
    /// closed: its id and its pane.
    fn create_replacement_tab(
        &self,
        wait: Wait,
        workspace: &str,
        cwd: &str,
        env: Env,
    ) -> Answer<(String, String)>;
    /// A tab of one pane running `command`, created in `workspace` or
    /// replacing `replace`: its id and its pane.
    #[allow(clippy::too_many_arguments)] // one tab's place, name, focus and program
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
    ) -> Answer<(String, String)>;
    /// A checkout's start tab, its one pane labelled `claim` until the start
    /// names it, replacing `seed_tab` when given: its id and its pane.
    #[allow(clippy::too_many_arguments)] // one tab's place, name, program and claim
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
    ) -> Answer<(String, String)>;
    fn close_tab(&self, wait: Wait, tab: &str) -> Answer<()>;
    fn focus_tab(&self, wait: Wait, tab: &str) -> Answer<()>;
    fn rename_tab(&self, wait: Wait, tab: &str, label: &str) -> Answer<()>;
    /// Moves a tab to `index` among its workspace's tabs: the tabs in order.
    fn move_tab(&self, wait: Wait, tab: &str, index: usize) -> Answer<Vec<String>>;

    // Layouts.

    /// The layout of the tab holding `pane`.
    fn pane_layout(&self, wait: Wait, pane: &str) -> Answer<SessionLayoutPayload>;
    /// A tab's whole layout, to give back later.
    fn export_layout(&self, wait: Wait, tab: &str) -> Answer<ClosedLayout>;
    /// Lays `root` out in `tab`, or in a new tab of `workspace` named
    /// `tab_label`, and focuses it: the layout made.
    fn apply_layout(
        &self,
        wait: Wait,
        workspace: &str,
        tab: Option<&str>,
        tab_label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout>;

    // Panes.

    /// A new pane beside `pane`, focused: its id.
    fn split_pane(
        &self,
        wait: Wait,
        pane: &str,
        direction: crate::live::PaneSplitDirection,
        cwd: Option<&str>,
    ) -> Answer<String>;
    /// A new focused pane beside `pane` at `ratio`: its id.
    fn split_pane_at(
        &self,
        wait: Wait,
        pane: &str,
        direction: ClosedSplitDirection,
        cwd: &str,
        ratio: f32,
        env: Env,
    ) -> Answer<String>;
    /// A new unfocused pane to the right of `parent` for a fork or a
    /// spawn beside it: its id.
    fn split_beside(&self, wait: Wait, parent: &str, cwd: Option<&str>, env: Env)
    -> Answer<String>;
    /// A new unfocused pane beside `pane` that stands in for one closed: its
    /// id.
    fn split_replacement(&self, wait: Wait, pane: &str, cwd: &str, env: Env) -> Answer<String>;
    fn close_pane(&self, wait: Wait, pane: &str) -> Answer<()>;
    fn focus_pane(&self, wait: Wait, pane: &str) -> Answer<()>;
    /// Names a pane; `None` clears its name.
    fn rename_pane(&self, wait: Wait, pane: &str, label: Option<&str>) -> Answer<()>;
    /// Zooms the pane, or unzooms it when zoomed: whether its zoom changed.
    fn toggle_zoom(&self, wait: Wait, pane: &str) -> Answer<bool>;
    /// Zooms or unzooms the pane's tab around it. Asked for the effect only;
    /// the layout read after it is what confirms it.
    fn set_zoom(&self, wait: Wait, pane: &str, zoom: Zoom) -> Answer<()>;
    /// Moves a pane's edge: whether anything moved.
    fn resize_pane(
        &self,
        wait: Wait,
        pane: &str,
        direction: crate::live::PaneResizeDirection,
        amount: f32,
    ) -> Answer<bool>;
    fn swap_panes(&self, wait: Wait, first: &str, second: &str) -> Answer<()>;
    /// Moves a pane, unfocused, into a new tab of `workspace`: where it is
    /// now, where it came from, and the tab made for it. A move the engine
    /// declined answers `changed: false` with its reason, not an error.
    fn move_pane_to_new_tab(
        &self,
        wait: Wait,
        pane: &str,
        workspace: &str,
        label: &str,
    ) -> Answer<PaneJoinResult>;
    /// Moves a pane, unfocused, beside `target_pane` in `target_tab`, to its
    /// right or `down`: where it is now and where it came from.
    fn move_pane_beside(
        &self,
        wait: Wait,
        pane: &str,
        target_tab: &str,
        target_pane: &str,
        down: bool,
    ) -> Answer<PaneJoinResult>;
    /// A pane's folder and the session its agent reports.
    fn pane_session(&self, wait: Wait, pane: &str)
    -> Answer<(Option<String>, Option<PaneSession>)>;
    /// The terminal a pane runs now.
    fn pane_terminal(&self, wait: Wait, pane: &str) -> Answer<String>;
    /// The value of a pane's metadata `token`.
    fn pane_token(&self, wait: Wait, pane: &str, token: &str) -> Answer<Option<String>>;
    /// The agent status the engine shows for a pane.
    fn pane_agent_status(&self, wait: Wait, pane: &str) -> Answer<String>;
    /// How far a pane is scrolled, when the engine reports it.
    fn pane_scroll(&self, wait: Wait, pane: &str) -> Answer<Option<crate::find::pane::PaneScroll>>;
    /// Scrolls a pane to `offset_from_bottom` lines: where it is now.
    fn scroll_pane(
        &self,
        wait: Wait,
        pane: &str,
        offset_from_bottom: u64,
    ) -> Answer<Option<crate::find::pane::PaneScroll>>;
    /// Sets Hide's metadata tokens on a pane; a `None` value clears one.
    fn report_pane_tokens(
        &self,
        wait: Wait,
        pane: &str,
        tokens: &BTreeMap<String, Option<String>>,
    ) -> Answer<()>;
    /// Tells the engine which session the `kind` agent in `pane` runs, as
    /// of `seq`.
    fn report_agent_session(
        &self,
        wait: Wait,
        pane: &str,
        kind: &str,
        session: &str,
        seq: u64,
    ) -> Answer<()>;

    // What a pane shows, and input to it.

    /// The last `lines` of a pane's text from `source` (`recent`,
    /// `visible`, ...), unwrapped and without styling.
    fn read_pane(&self, wait: Wait, pane: &str, source: &str, lines: u32) -> Answer<PaneText>;
    /// The visible screen of a pane with its styling kept.
    fn read_screen(&self, wait: Wait, pane: &str) -> Answer<PaneText>;
    /// The last `lines` rows of a pane as the terminal drew them, escape
    /// sequences kept.
    fn read_rows(&self, wait: Wait, pane: &str, lines: u32) -> Answer<PaneText>;
    /// Types `text` into a pane as it is.
    fn send_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()>;
    /// Presses `keys` in a pane, in order.
    fn send_keys(&self, wait: Wait, pane: &str, keys: &[&str]) -> Answer<()>;
    /// Types `text` into a pane and submits it.
    fn submit_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()>;
    /// Presses one key in a pane.
    fn press_key(&self, wait: Wait, pane: &str, key: &str) -> Answer<()>;
    /// Shows the operator a notification.
    fn notify(&self, wait: Wait, title: &str, body: &str) -> Answer<Notified>;

    // A pane's processes.

    /// The pane's shell and its foreground processes.
    fn pane_processes(&self, wait: Wait, pane: &str) -> Answer<PaneProcessGroup>;
    /// The name of the program in the pane's foreground, if any.
    fn foreground_program(&self, wait: Wait, pane: &str) -> Answer<Option<String>>;
    /// The session a foreground `agent` was explicitly resumed with, as its
    /// kind (`id` or `path`) and value.
    fn resumed_session(
        &self,
        wait: Wait,
        pane: &str,
        agent: hide_session::Agent,
    ) -> Answer<Option<(String, String)>>;
    /// The pane an inherited caller reference names now.
    fn current_pane(&self, wait: Wait, pane: &str) -> Answer<String>;
    /// Whose pane an inherited caller reference names now, and that pane's
    /// shell and terminal: the proof a pane's process asks a credential on.
    fn pane_identity(&self, wait: Wait, pane: &str) -> Answer<PaneProof>;

    // Agents.

    /// `launch` as a start this engine can send, or why it cannot: checked
    /// before any wait for the pane's shell, so a start that could never be
    /// sent is refused at once.
    fn prepare_start(&self, launch: AgentLaunch) -> Result<PreparedStart, String>;
    /// Starts `start` in its pane's shell: the agent's pane.
    fn start_agent(&self, wait: Wait, start: &PreparedStart) -> Answer<StartedAgent>;
    /// An agent's kind, status and state sequence, by name or pane.
    fn agent_state(&self, wait: Wait, target: &str) -> Answer<AgentState>;
    /// An agent as a letter's delivery checks it, by name or pane.
    fn delivery_agent(&self, wait: Wait, target: &str) -> Answer<DeliveryAgent>;
    /// [`Self::delivery_agent`] with the native session the agent reports.
    fn observed_agent(
        &self,
        wait: Wait,
        target: &str,
    ) -> Answer<(
        DeliveryAgent,
        Option<hide_agent_hooks::observation::ObservationNativeSession>,
    )>;
    /// Types `text` into the agent's input and submits it.
    fn prompt_agent(&self, wait: Wait, target: &str, text: &str) -> Answer<()>;

    // Worktrees.

    /// A new worktree of the repository at `cwd` on `branch`, from `base`,
    /// opened in a new workspace named `label` and placed at `path` when
    /// given, where the engine names and places it otherwise.
    #[allow(clippy::too_many_arguments)] // one worktree's branch, start, name and place
    fn create_worktree(
        &self,
        wait: Wait,
        cwd: &str,
        branch: &str,
        base: Option<&str>,
        label: Option<&str>,
        path: Option<&str>,
        focus: bool,
    ) -> Answer<CreatedWorktree>;
    /// The path of the repository's worktree on `branch`, if any.
    fn worktree_path(&self, wait: Wait, cwd: &str, branch: &str) -> Answer<Option<String>>;
    /// The workspace bound to the checkout at `path`, opened if none is.
    fn open_worktree(
        &self,
        wait: Wait,
        path: &str,
        repository_root: &str,
        focus: bool,
    ) -> Answer<OpenedWorktree>;
    /// Removes the worktree a workspace is bound to, and the workspace.
    fn remove_worktree(&self, wait: Wait, workspace: &str) -> Answer<()>;
}

/// The checks a gated call passes: `before` as the call starts, and
/// `at_send` again once the engine is reached and just before the request
/// goes, since reaching a device's engine can take a while. A refusal of
/// either is the call's, and nothing was sent.
#[derive(Clone)]
pub(crate) struct Gate<'a> {
    pub(crate) before: Arc<GateCheck<'a>>,
    pub(crate) at_send: Arc<GateCheck<'a>>,
}

type GateCheck<'a> = dyn Fn() -> Result<(), EngineError> + Send + Sync + 'a;

impl<'a> Gate<'a> {
    pub(crate) fn new(
        before: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a,
        at_send: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a,
    ) -> Self {
        Self {
            before: Arc::new(before),
            at_send: Arc::new(at_send),
        }
    }

    /// The same check on both sides of reaching the engine.
    pub(crate) fn both(check: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a) -> Self {
        let check: Arc<GateCheck<'a>> = Arc::new(check);
        Self {
            before: Arc::clone(&check),
            at_send: check,
        }
    }
}

/// Whether a tab is to be zoomed on a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Zoom {
    On,
    Off,
}

/// The session's changes as the engine reports them, read one at a time
/// until the stream ends or [`EventStream::stop`] wakes the reader.
pub(crate) struct EventStream {
    pub(crate) reader: Box<dyn EventReader>,
    pub(crate) stopper: Box<dyn StopEvents>,
}

/// Reads the next event; `None` once the stream ended, with why.
pub(crate) trait EventReader: Send {
    fn next_event(&mut self) -> Result<Option<EventLine>, String>;
}

/// Ends a stream: its blocked reader wakes with the end.
pub(crate) trait StopEvents: Send {
    fn stop(&self);
}

/// One line of the event stream, read as the replica takes it.
pub(crate) type EventLine =
    Result<crate::session_sync::SubscriptionLine, crate::live::SessionFetchError>;

/// The engine hided hands the core and asks itself for the few things that
/// cannot wait for the core's owner thread (a pane's proof, a phone's read
/// of a pane). It holds the one engine both use.
#[derive(Clone)]
pub struct EngineHandle {
    engine: SharedEngine,
}

impl EngineHandle {
    pub(crate) fn new(engine: SharedEngine) -> Self {
        Self { engine }
    }

    pub(crate) fn shared(&self) -> SharedEngine {
        Arc::clone(&self.engine)
    }
}

/// What hided asks an engine itself, off the core's owner thread and the
/// runtime lock: a phone's read of one pane and its input there, a pane's
/// proof before a credential, and whether the engine answers at all.
impl EngineHandle {
    /// The last `lines` rows of a pane as the terminal drew them, escape
    /// sequences kept; each step of the call waits up to `timeout`.
    pub fn read_rows(
        &self,
        pane: &str,
        lines: u32,
        timeout: Duration,
    ) -> Result<PaneText, EngineError> {
        self.engine.read_rows(Wait::Each(timeout), pane, lines)
    }

    /// Types `text` into a pane and submits it.
    pub fn submit_text(
        &self,
        pane: &str,
        text: &str,
        timeout: Duration,
    ) -> Result<(), EngineError> {
        self.engine.submit_text(Wait::Each(timeout), pane, text)
    }

    /// Presses one key in a pane.
    pub fn press_key(&self, pane: &str, key: &str, timeout: Duration) -> Result<(), EngineError> {
        self.engine.press_key(Wait::Each(timeout), pane, key)
    }

    /// A pane's folder and the session its agent reports.
    pub fn pane_session(
        &self,
        pane: &str,
        timeout: Duration,
    ) -> Result<(Option<String>, Option<PaneSession>), EngineError> {
        self.engine.pane_session(Wait::Each(timeout), pane)
    }

    /// The engine's version and protocol.
    pub fn ping(&self, timeout: Duration) -> Result<ServerPong, EngineError> {
        self.engine.ping(Wait::Each(timeout))
    }

    /// The pane an inherited caller reference names now, by `deadline`: routing
    /// evidence only, which a caller attests separately. Refused under the
    /// pane bootstrap's codes.
    pub fn current_pane(&self, pane: &str, deadline: Instant) -> Result<String, &'static str> {
        self.engine
            .current_pane(Wait::By(deadline), pane)
            .map_err(|error| proof_refusal(&error, deadline))
    }

    /// The pane an inherited caller reference names now, with its shell and
    /// terminal, all read by `deadline`: what a pane's process is checked
    /// against before it gets a credential. Refused under the pane
    /// bootstrap's codes.
    pub fn pane_identity(&self, pane: &str, deadline: Instant) -> Result<PaneProof, &'static str> {
        self.engine
            .pane_identity(Wait::By(deadline), pane)
            .map_err(|error| proof_refusal(&error, deadline))
    }
}

/// A pane proof's refusal: a reference that cannot be asked about, a
/// deadline that passed, or a pane the engine did not prove.
fn proof_refusal(error: &EngineError, deadline: Instant) -> &'static str {
    match error {
        EngineError::Request(_) => "invalid_request",
        _ if Instant::now() >= deadline => "pane_inspection_expired",
        _ => "pane_unavailable",
    }
}

impl fmt::Debug for EngineHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EngineHandle")
    }
}
