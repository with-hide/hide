//! The engine boundary: the one way the core speaks to the program that runs
//! its panes (PRD engine-bundle-1 D-2). Pane existence, split geometry, zoom,
//! cwd, agent lifecycle and the PTY are the engine's; the core asks for them
//! through [`Engine`] and never names the engine's protocol. The core's own
//! engine ([`own::OwnEngine`]), which runs panes in the node's holders, is
//! the only one (PRD engine-bundle-4-herdr-removal D-2).
//!
//! Terminal bytes (`TerminalRoutes`) and a machine's own work (`NodeLink`)
//! are already outside this boundary and do not pass through it.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) mod answers;
#[cfg(test)]
pub(crate) mod contract;
#[cfg(test)]
pub(crate) mod fake;
pub mod keys;
pub(crate) mod own;
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
    /// The engine answered with a refusal, under its stable code. A refusal
    /// is made before the engine does what was asked: an input it refuses,
    /// its own or its node's, typed nothing (`delivery::ladder`).
    Refused { code: String, message: String },
    /// The engine's answer could not be read as an answer at all.
    Malformed(String),
    /// The request could not be put into the engine's terms; nothing was
    /// sent.
    Request(String),
    /// The engine answered, but not with what the call's contract promises.
    Answer(String),
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
        matches!(self, Self::Refused { .. } | Self::Answer(_))
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
            | Self::Answer(message) => formatter.write_str(message),
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
pub use answers::{PaneProof, PaneRows, PaneSession, PaneSessionKind, PaneText, ServerPong};

/// What `hide agent exec` reported of a profile's start
/// (`HookCommand::AgentExec`), with the process that sent it as the pane's
/// machine says it is that attempt's executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentExecReport {
    pub attempt: String,
    pub event: hide_node_link::panes::AgentExecEvent,
    pub executor: hide_node_link::panes::ExecProcess,
}

/// The refusal of an [`AgentExecReport`] for an attempt that is not, or no
/// longer, running in its pane.
pub const ATTEMPT_UNKNOWN: &str = "attempt_unknown";

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
    /// The engine's name where a diagnostic says which engine moved
    /// something: `own_engine`.
    fn name(&self) -> &'static str;
    /// This engine, with every call first passing `gate`.
    fn gated<'a>(&'a self, gate: Gate<'a>) -> Box<dyn Engine + 'a>;
    /// How the session's watcher learns it changed: `wake`, called once a
    /// change has been made, after which it reads the whole session again.
    /// The session is the engine's own state, so nothing is replayed (PRD
    /// engine-bundle-3-core-owns D-3). Asked once per watcher; a later call
    /// replaces the earlier `wake`.
    fn changes(&self, wake: Wake);
    /// The agent definitions the operator's folder holds now, read by the
    /// core: what an engine that starts and reads agents itself starts,
    /// recognizes and reads them by.
    fn set_agent_definitions(&self, _catalog: Arc<hide_agent_adapter::definition::Catalog>) {}
    /// The program a start of the `kind` agent types: its definition's first
    /// executable, as this engine reads the definitions now, which is also
    /// what the pane's node is asked how it starts (`start_prompt::program_of`).
    /// `None` where no definition names `kind`.
    fn start_program_name(&self, kind: &str) -> Option<String>;
    /// The session the hook of the `kind` agent in `pane` reported, proved
    /// to come from that pane (PRD engine-bundle-3-core-owns D-9, D-16): an
    /// engine that starts and resumes agents itself keeps it as the pane's
    /// agent's session, saved with the pane, so a reboot resumes that
    /// conversation. Answers true once for the first session of an agent
    /// the engine resumed after its turn was cut off under it (PRD
    /// engine-bundle-4-herdr-removal D-22), which the core then tells.
    fn hook_session(&self, _pane: &str, _kind: &str, _session: &str) -> Answer<bool> {
        Ok(false)
    }

    /// What `hide agent exec` reported of the profile's start this engine
    /// typed in `pane` (PRD agent-profiles B22), kept with that start's
    /// attempt, whose result the engine reads from the executor's process
    /// ([`Self::start_agent`]). Refused when the pane has no such attempt
    /// running, which tells a `Ready` executor not to start its program;
    /// an engine that types no profile's start has none.
    fn agent_exec(&self, _pane: &str, report: AgentExecReport) -> Answer<()> {
        Err(EngineError::Refused {
            code: ATTEMPT_UNKNOWN.to_owned(),
            message: format!("no start with attempt {} runs here", report.attempt),
        })
    }

    /// What the executor of the attempt `attempt` in `pane` is answered from
    /// when it asks for its profile (`AgentExecEvent::Profile`): the profile
    /// and version the start was checked with, which the caller asks the
    /// pane's machine for off every lock (`launch::exec_start`). Refused,
    /// like a report, unless that attempt runs there and has no `Ready`.
    fn exec_attempt(&self, _pane: &str, attempt: &str) -> Answer<crate::launch::ExecAttempt> {
        Err(EngineError::Refused {
            code: ATTEMPT_UNKNOWN.to_owned(),
            message: format!("no start with attempt {attempt} runs here"),
        })
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
    /// The last `lines` rows of a pane with soft wraps joined, escape
    /// sequences kept (a holder built before that answers the rows as
    /// drawn).
    fn read_rows(&self, wait: Wait, pane: &str, lines: u32) -> Answer<PaneRows>;
    /// Types `text` into a pane as it is.
    fn send_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()>;
    /// Presses `keys` in a pane, in order.
    fn send_keys(&self, wait: Wait, pane: &str, keys: &[&str]) -> Answer<()>;
    /// Presses `keys` in a pane as the sleep of its agent does, to end it:
    /// the only keys, beside a wake's start, that a pane whose agent is
    /// asleep takes.
    fn send_sleep_keys(&self, wait: Wait, pane: &str, keys: &[&str]) -> Answer<()>;
    /// Types `text` into a pane and submits it.
    fn submit_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()>;
    /// Presses one key in a pane.
    fn press_key(&self, wait: Wait, pane: &str, key: &str) -> Answer<()>;
    /// Presses Hide's `keys` in a pane, in order, in one call.
    fn press_pane_keys(&self, wait: Wait, pane: &str, keys: &[keys::PaneKey]) -> Answer<()>;
    /// Puts `text` into a pane as one paste, bracketed when the program in
    /// the pane asked for bracketed paste, with no Enter after it.
    fn paste_text(&self, wait: Wait, pane: &str, text: &str) -> Answer<()>;

    // A pane's processes.

    /// Returns once the engine has laid its saved session out again around
    /// the holders its node found, the wait every call that makes a pane
    /// takes, or refuses when that has not happened `within`. Until then a
    /// saved pane is drawn but its holder is not yet taken back, so a start
    /// into it reads nothing. An engine that restores nothing answers at once.
    fn wait_restored(&self, _within: Duration) -> Answer<()> {
        Ok(())
    }
    /// The pane's shell and its foreground processes.
    fn pane_processes(&self, wait: Wait, pane: &str) -> Answer<PaneProcessGroup>;
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
    /// The pane's terminal and shell as this engine itself proves them, what
    /// a start checks it types into the pane it chose (PRD
    /// engine-bundle-3-core-owns D-9): the engine reads the pane's record.
    fn inspect_pane(&self, wait: Wait, pane: &str) -> Answer<hide_node_link::panes::PaneIdentity>;

    // Agents.

    /// `launch` as a start this engine can send, or why it cannot: checked
    /// before any wait for the pane's shell, so a start that could never be
    /// sent is refused at once.
    fn prepare_start(&self, launch: AgentLaunch) -> Result<PreparedStart, String>;
    /// Starts `start` in its pane's shell: the agent's pane.
    fn start_agent(&self, wait: Wait, start: &PreparedStart) -> Answer<StartedAgent>;
    /// Brings the pane's holder to this build before an agent starts again
    /// in it (with-hide/hide-private#646): a shell pane whose holder runs from
    /// another build is started again under the same id, in `cwd` when given
    /// (else the folder it was started in), from its machine's current
    /// holder. Every answer names the holder that runs the pane, which an
    /// agent may start in; a refusal is a pane nothing may be typed into,
    /// since no holder is known to run it.
    fn renew_holder(
        &self,
        wait: Wait,
        pane: &str,
        cwd: Option<&str>,
    ) -> Answer<hide_node_link::holders::HolderRenewal>;
    /// An agent's kind, status and state sequence, by name or pane.
    fn agent_state(&self, wait: Wait, target: &str) -> Answer<AgentState>;
    /// An agent as a letter's delivery checks it, by name or pane.
    fn delivery_agent(&self, wait: Wait, target: &str) -> Answer<DeliveryAgent>;
    /// Types `text` into the agent's input and submits it. A refusal typed
    /// nothing; an answer that never came may follow a submission that
    /// arrived.
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
    /// Told the id of a pane a call makes before anything runs in it, on
    /// an engine that names a pane before it starts one ([`Placement`]).
    pub(crate) placement: Option<Arc<dyn Placement>>,
}

type GateCheck<'a> = dyn Fn() -> Result<(), EngineError> + Send + Sync + 'a;

/// What a caller records against a pane a gated call makes, before the pane
/// runs anything (PRD engine-bundle-3-core-owns D-11): a spawn's record names
/// its pane before the pane's program starts, so no pane runs that its
/// record does not name, and none is named that never ran.
///
/// The core's own engine names a pane when it reserves its id and calls
/// [`Placement::place`] after it let go of its topology lock and before it
/// starts the pane's holder, outside every lock: the topology lock reserves
/// the id alone, `place` takes whatever lock its record needs (the runtime's,
/// for a spawn's record) and lets it go, and only then does the holder
/// start. A holder that does not start is [`Placement::withdraw`]n in the
/// same call. An engine that names a pane only once it runs never places one,
/// and the caller records the pane after the call, as before (D-2).
pub(crate) trait Placement: Send + Sync {
    /// Records `pane`; a refusal leaves the pane unmade and is the call's.
    fn place(&self, pane: &str) -> Result<(), EngineError>;
    /// Takes back what [`Placement::place`] recorded for `pane`, whose
    /// program did not start.
    fn withdraw(&self, pane: &str);
}

impl<'a> Gate<'a> {
    pub(crate) fn new(
        before: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a,
        at_send: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a,
    ) -> Self {
        Self {
            before: Arc::new(before),
            at_send: Arc::new(at_send),
            placement: None,
        }
    }

    /// The same check on both sides of reaching the engine.
    pub(crate) fn both(check: impl Fn() -> Result<(), EngineError> + Send + Sync + 'a) -> Self {
        let check: Arc<GateCheck<'a>> = Arc::new(check);
        Self {
            before: Arc::clone(&check),
            at_send: check,
            placement: None,
        }
    }

    /// No check, and `placement` told each pane a call makes.
    pub(crate) fn placing(placement: impl Placement + 'static) -> Self {
        Self {
            before: Arc::new(|| Ok(())),
            at_send: Arc::new(|| Ok(())),
            placement: Some(Arc::new(placement)),
        }
    }
}

/// What an engine calls once its session changed ([`Engine::changes`]).
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

/// Whether a tab is to be zoomed on a pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Zoom {
    On,
    Off,
}

/// The engine hided hands the core and asks itself for the few things that
/// cannot wait for the core's owner thread (a pane's proof, a phone's read
/// of a pane). It holds the one engine both use.
#[derive(Clone)]
pub struct EngineHandle {
    engine: SharedEngine,
    /// The folder the core's own engine stands for, which the core keys this
    /// machine's session by; `None` for a test's engine.
    own_scope: Option<std::path::PathBuf>,
    /// The Hide installation whose panes this engine starts, as the embedder
    /// that owns the machine read it (`hide_host::kit::running_install`), so
    /// the core composes the hooks' records folder by the one rule those
    /// panes' hooks apply; `None` for a pane that names none.
    own_install: Option<String>,
}

impl EngineHandle {
    pub(crate) fn new(engine: SharedEngine) -> Self {
        Self {
            engine,
            own_scope: None,
            own_install: None,
        }
    }

    /// The core's own engine for this machine, whose panes the holders of
    /// `node` run, for the state folder `state_dir`: it stands for that
    /// folder's engine folder (`hide_kit::layout::engine_dir`), saves its
    /// session there and lays the saved session out again as it opens,
    /// `home` is where it opens worktrees and a restored pane whose folder is
    /// gone, and `node` is this machine's node, which reads its folders, runs
    /// its Git and answers its pane holders through the same requests a
    /// device's node answers ([`hide_node_link::link_panes::LinkPanes`]).
    /// Every daemon builds it (PRD engine-bundle-4-herdr-removal D-2).
    ///
    /// `install` is the Hide installation whose panes this engine starts, as
    /// the embedder that owns this machine names it
    /// (`hide_host::kit::running_install`); the core never reads it from this
    /// process itself (PRD core-host-node D-21). The hooks' records folder is
    /// the state folder this engine was handed (never this process's
    /// environment's) composed with it by the one rule those panes' hooks
    /// apply, so `~/.hide/agent-hooks` for the desktop app on the account's
    /// own state folder and that folder's `installs/<mark>` for a build
    /// started from elsewhere.
    pub fn own(
        node: Arc<dyn hide_node_link::NodeLink>,
        state_dir: std::path::PathBuf,
        home: std::path::PathBuf,
        definitions: hide_agent_adapter::definition::Catalog,
        install: Option<String>,
    ) -> Result<Self, String> {
        let scope = hide_kit::layout::engine_dir(&state_dir);
        let hooks_records =
            hide_agent_hooks::route::records_of_state(&home, Some(&state_dir), install.as_deref())
                .map_err(|error| {
                    format!(
                        "the hooks' records folder of {} could not be read: {error}",
                        state_dir.display()
                    )
                })?;
        let home = hide_platform::path::to_wire(&home)
            .map_err(|error| format!("{} has no wire spelling: {error}", home.display()))?;
        // The choices this machine's agents were started with, which the
        // core keeps and reads again when it starts; the restore that starts
        // with the engine resumes its agents with them (PRD
        // engine-bundle-4-herdr-removal D-24). This machine's panes are keyed
        // by their own ids there, as the engine's are.
        let launches = crate::persistence::load(&state_dir.join(crate::node_migration::CORE_STATE))
            .0
            .pane_launches
            .into_iter()
            .filter(|(pane, _)| !crate::address::is_device_scoped(pane))
            .collect();
        let engine = own::OwnEngine::open(
            own::link_panes(Arc::clone(&node)),
            own::Place {
                engine_dir: scope.clone(),
                home,
                hooks_records: Some(hooks_records),
                node,
                // This machine's own node.
                windows: cfg!(windows),
                definitions: Arc::new(definitions),
                launches: Arc::new(launches),
            },
        )?;
        Ok(Self {
            engine: Arc::new(engine),
            own_scope: Some(scope),
            own_install: install,
        })
    }

    /// The folder the core's own engine stands for; `None` for a test's engine.
    pub fn own_scope(&self) -> Option<&std::path::Path> {
        self.own_scope.as_deref()
    }

    /// The Hide installation whose panes this engine starts, as its embedder
    /// named it; `None` for panes that name none.
    pub fn own_install(&self) -> Option<&str> {
        self.own_install.as_deref()
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
    ) -> Result<PaneRows, EngineError> {
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
