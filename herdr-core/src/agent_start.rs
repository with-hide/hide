//! Starting an agent in a pane, the one way every Hide start reaches Herdr.
//!
//! The pinned Herdr's `agent.start` types the agent's command into the pane
//! only while the pane's shell alone holds the terminal: the foreground
//! process group is the shell's own and has no other member (v0.9.1
//! `available_pane_shell_from_job`). It does not wait for that state, it
//! refuses with `agent_pane_busy`, and no event announces the moment the
//! state is reached. A pane Hide has just made, a new worktree's, a fork's
//! split or a restored tab, is still running its shell's startup files then,
//! so a start reads that state through `pane.process_info` and sends
//! `agent.start` only once the shell holds the terminal. The wait is bounded;
//! a shell that never reaches its prompt is a start that did not happen, with
//! nothing typed into the pane.
//!
//! The shell alone holds the terminal between its startup files' commands
//! too, before its line editor runs, and until then the terminal is in
//! canonical mode: the kernel keeps an unfinished typed line and drops what
//! passes its limit (1,024 bytes on macOS), Enter included, so a long start
//! line (a first prompt) would be cut and never run. A start whose line could
//! be cut therefore also waits, inside the same bound, until the pane's node
//! reads the shell's terminal taking keys rather than lines (`line_input`),
//! which is the line editor reading. A line shorter than every supported
//! terminal's limit reaches the shell whole either way and asks no node.
//!
//! A start that puts an agent back under the name of the one just ended in
//! the same pane (a wake, a Reopen) also meets `agent_name_taken` until
//! Herdr has forgotten the ended agent; [`start_at_shell_reusing_name`] sends
//! that start again on exactly that refusal, inside a bounded wait. The other
//! starts take that refusal as the answer, since their name is someone else's.
//!
//! The shell holding the terminal does not say its startup files are done:
//! a startup file that asks a question (oh-my-zsh's `[Y/n]` update prompt
//! reads one key with `read -k 1`) leaves the shell alone in the foreground
//! too, and a start typed then answers the question with the command's first
//! letter and runs the rest. A pane Hide makes for a start therefore runs
//! the account's shell from Hide's own command (`hide_host::shell_integration`,
//! through Herdr's `layout.apply`), which marks the shell's first prompt on
//! the pane once the user's own startup files are done, and a start in such
//! a pane ([`ShellStart::Integrated`]) waits for that mark too. A shell that
//! never shows it within the bound, stopped at a question, is a start that
//! did not happen, and the screen's last line says what it waits for. A
//! shell the integration does not reach (sh, nu, cmd, or one a startup file
//! replaced the launched shell with), or a pane made another way (a split),
//! is started on the shell holding the terminal, and
//! the start is confirmed after it was typed, by its spawn
//! (`coordination::executor`).
//!
//! Every call here runs on a worker thread, never under `Mutex<Runtime>`.

use std::thread;
use std::time::{Duration, Instant};

use hide_herdr_client::{
    ApiConnector, ApiError, request_with_connector, request_with_correlation_id,
};
use hide_node_link::process::LineInput;
use hide_node_link::protocol::{Call, ShellLaunch};
use serde_json::Value;

use crate::node_access::NodeLink;
use crate::wire;

/// How long a pane's shell gets to reach its prompt: the pinned Herdr's own
/// default wait for an agent's readiness.
pub(crate) const SHELL_WAIT: Duration = Duration::from_secs(30);
/// How long Herdr waits for the agent of a start it typed (`timeout_ms`), and
/// so how long it holds the start's name while no agent shows. Long, because
/// typing a long first prompt into a slow shell takes most of it.
pub(crate) const AGENT_START_DEADLINE: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A typed start is not proof that its program exists. Keep the fork's node
/// holder until Herdr observes this named agent ready in the created pane.
/// This determines only input release, never whether the fork succeeded.
/// Reads also settle Herdr's pending start (#3853); nothing is typed again.
/// Runs on the existing fork worker, with no Runtime lock or new timer.
#[allow(clippy::disallowed_methods)] // bounded production polling, not a test sleep
pub(crate) fn confirm_fork_started(
    connector: &dyn ApiConnector,
    pane: &str,
    name: &str,
    kind: &str,
    current: &dyn Fn() -> Result<(), String>,
    deadline: Instant,
) -> Result<(), String> {
    let params = wire::agent_target_params(name)?;
    let mut last_read_error = None;
    loop {
        current()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(match last_read_error {
                Some(error) => format!("The fork's input readiness deadline elapsed: {error}"),
                None => "The fork's input readiness deadline elapsed.".into(),
            });
        }
        let agent = request_with_connector(
            connector,
            "agent.get",
            params.clone(),
            remaining.min(READ_TIMEOUT),
        )
        .map_err(|error| format!("The fork's agent could not be confirmed: {error}"))
        .and_then(wire::delivery_agent);
        // A transient read failure proves neither readiness nor a closed
        // pane. Keep refusing input until a positive answer or the deadline.
        if let Ok(agent) = agent {
            last_read_error = None;
            if agent.pane_id != pane || agent.name != name {
                return Err("The fork's agent identity changed.".into());
            }
            if let Some(observed) = agent.kind {
                if hide_agent_adapter::canonical_kind(&observed)
                    != hide_agent_adapter::canonical_kind(kind)
                {
                    return Err("The fork's agent kind changed.".into());
                }
                if agent.readiness.accepts_input() {
                    return current();
                }
            }
        } else {
            last_read_error = agent.err();
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(POLL_INTERVAL),
        );
    }
}
const PANE_BUSY: &str = "agent_pane_busy";
const NAME_TAKEN: &str = "agent_name_taken";
const AGENT_NOT_FOUND: &str = "agent_not_found";
/// The fewest bytes of an unfinished line a supported system's terminal
/// keeps before the shell reads it: macOS's `MAX_INPUT` (Linux keeps 4,096,
/// and a Windows console cuts nothing). A start line shorter than this
/// reaches the shell whole once the shell holds the terminal.
const SHORTEST_LINE_LIMIT: usize = 1024;
/// How long Herdr gets to let go of the name of an agent that was just
/// ended: it forgets the agent a moment after its process is gone, and
/// refuses `agent.start` under that name until it has.
pub(crate) const NAME_RELEASE_WAIT: Duration = Duration::from_secs(10);

/// The refusal of a start whose shell had not finished starting: the code,
/// then the screen's last line after [`crate::coordination::DETAIL`] when it
/// could be read.
pub(crate) const SHELL_STARTING: &str = "shell_starting";
/// The pane token the integration's first prompt reports
/// (`hide_host::shell_integration::PROMPT_TOKEN`).
const PROMPT_TOKEN: &str = "hide_prompt";
/// The most of a screen line a refusal carries.
const SCREEN_LINE_CHARS: usize = 120;

/// What says a pane's shell has finished starting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShellStart {
    /// The caller made the pane run Hide's [`ShellLaunch`] of the shell
    /// `program`, which marks its first prompt, and the start waits for that
    /// mark while that shell holds the terminal.
    Integrated { program: String },
    /// No mark is coming (a pane made before, by someone else, or where the
    /// integration could not be written): the shell holding the terminal is
    /// the only sign.
    Plain,
}

impl ShellStart {
    /// How the shell of a tab says it has finished starting: by its mark
    /// when the tab runs `launch`, else by holding the terminal.
    pub(crate) fn of_tab(launched: bool, launch: Option<&ShellLaunch>) -> Self {
        match launch.and_then(|launch| launch.command.first()) {
            Some(shell)
                if launched
                    && launch.is_some_and(|launch| {
                        launch.readiness == hide_node_link::protocol::ShellReadiness::Integrated
                    }) =>
            {
                Self::Integrated {
                    program: program_name(shell),
                }
            }
            _ => Self::Plain,
        }
    }

    /// How the shell of a pane recorded as launched in `program` says it has
    /// finished starting.
    pub(crate) fn launched(program: Option<String>) -> Self {
        program.map_or(Self::Plain, |program| Self::Integrated { program })
    }

    /// The shell this start waits on for a mark, if any.
    pub(crate) fn program(&self) -> Option<&str> {
        match self {
            Self::Integrated { program } => Some(program),
            Self::Plain => None,
        }
    }
}

/// Characters that change how text around them reads without showing
/// themselves: zero-width ones, bidirectional overrides and isolates.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// A shell's program name as a pane's process list shows it: the last part
/// of its path, without a login shell's leading `-` or Windows' `.exe`,
/// lower case.
fn program_name(shell: &str) -> String {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .trim_start_matches('-')
        .to_ascii_lowercase();
    match name.strip_suffix(".exe") {
        Some(name) => name.to_owned(),
        None => name,
    }
}

/// Why a start did not settle with Herdr's answer to `agent.start`.
#[derive(Debug)]
pub(crate) enum StartError {
    /// Nothing was typed into the pane: its shell never held the terminal in
    /// time, or its state could not be read.
    NotStarted(String),
    /// Nothing was typed into the pane: its shell held the terminal but had
    /// not marked its first prompt within the wait, stopped in its startup
    /// files (a question waiting for a key). Carries the screen's last line.
    ShellStarting(Option<String>),
    /// Herdr's own answer to `agent.start`: a refusal, or no answer at all.
    Herdr(ApiError),
}

/// Records, on the node that will run it, that `kind` may work in `path`, a
/// worktree Hide made, before Hide starts it there (the agent's own folder
/// trust: docs/agent-hooks.md, Folder trust for the worktrees Hide makes).
/// Nothing is asked for a kind that never asks; a trust that cannot be
/// recorded refuses the start with its reason, so no agent waits unseen on
/// its trust question (PRD antigravity-first-class B20).
pub(crate) fn trust_worktree(
    node: Option<&dyn crate::node_access::NodeLink>,
    kind: &str,
    path: &str,
) -> Result<(), String> {
    let Some((adapter, agent)) = hide_agent_adapter::adapter(kind).and_then(|adapter| {
        let agent = hide_agent_hooks::folder_trust::Agent::of(adapter.start?)?;
        Some((adapter, agent))
    }) else {
        return Ok(());
    };
    let node = node.ok_or(
        "agent_trust_unavailable: the machine that runs the worktree's pane cannot be reached.",
    )?;
    crate::node_access::call_as::<()>(
        node,
        hide_node_link::protocol::Call::AgentTrust {
            agent: adapter.id.to_owned(),
            path: path.to_owned(),
            trusted: true,
        },
        // The node may wait for another start's change of the same agent's
        // trust before its own (a Codex app-server each), and the link
        // carries the answer back.
        agent.longest_change() + Duration::from_secs(5),
    )
    .map_err(|error| {
        format!(
            "agent_trust_failed: {}'s trust of {path} could not be recorded ({error})",
            adapter.label
        )
    })
}

/// How a pane Hide makes for a start runs the account's shell so that it
/// marks its first prompt, written by `node`, the node of the machine that
/// will run the pane. The node supplies an explicit Plain recipe when it
/// cannot install prompt integration. `None`, logged, means the node or its
/// normal shell cannot be confirmed; group starts refuse before creation.
pub(crate) fn shell_launch(
    node: Option<&dyn NodeLink>,
    correlation_id: &str,
) -> Option<ShellLaunch> {
    let answer = match node {
        Some(node) => crate::node_access::call_as::<Option<ShellLaunch>>(
            node,
            Call::ShellIntegration,
            READ_TIMEOUT,
        )
        .map_err(|error| error.to_string()),
        None => Err("the machine that runs the pane cannot be reached".to_owned()),
    };
    answer
        .inspect_err(|reason| {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_start",
                "kind": "shell_integration.unavailable",
                "request": correlation_id,
                "reason": reason,
            }));
        })
        .ok()
        .flatten()
}

/// What an operator reads for a start whose shell had not finished starting,
/// with the screen's last line when there was one.
pub(crate) fn shell_starting_message(screen: Option<&str>) -> String {
    match screen {
        Some(line) => format!(
            "The pane's shell had not finished starting after {} s, so the agent was not started. Its screen ends with \"{line}\": answer what it asks in that pane, then retry.",
            SHELL_WAIT.as_secs()
        ),
        None => format!(
            "The pane's shell had not finished starting after {} s, so the agent was not started. Check what its startup files wait for in that pane, then retry.",
            SHELL_WAIT.as_secs()
        ),
    }
}

/// Action proof after shell readiness, repeated before every actual start
/// attempt. No proof is cached across startup waits or a busy-pane refusal.
/// `node` is the node that runs the pane, asked how its shell's terminal
/// takes input when the start line could be cut; none when that node cannot
/// be reached, which only a start with such a line minds.
#[allow(clippy::too_many_arguments)] // one start: its pane, how its shell says it is ready, and its effect admission
pub(crate) fn start_at_shell_checked(
    connector: &dyn ApiConnector,
    node: Option<&dyn NodeLink>,
    correlation_id: &str,
    pane_id: &str,
    shell: ShellStart,
    params: Value,
    answer_timeout: Duration,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Value, StartError> {
    start_within(
        connector,
        node,
        correlation_id,
        pane_id,
        shell,
        params,
        answer_timeout,
        SHELL_WAIT,
        Duration::ZERO,
        check,
    )
}

/// A start for an agent that goes back under the name of the one
/// just ended in this pane: `agent_name_taken` is answered by sending the
/// start again until Herdr has released the name, within [`NAME_RELEASE_WAIT`].
/// `check` is the action proof [`start_at_shell_checked`] repeats before every
/// attempt.
#[allow(clippy::too_many_arguments)] // one start: its pane, how its shell says it is ready, and its effect admission
pub(crate) fn start_at_shell_reusing_name(
    connector: &dyn ApiConnector,
    node: Option<&dyn NodeLink>,
    correlation_id: &str,
    pane_id: &str,
    shell: ShellStart,
    params: Value,
    answer_timeout: Duration,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Value, StartError> {
    start_within(
        connector,
        node,
        correlation_id,
        pane_id,
        shell,
        params,
        answer_timeout,
        SHELL_WAIT,
        NAME_RELEASE_WAIT,
        check,
    )
}

#[allow(clippy::disallowed_methods)] // a production wait, not test code
#[allow(clippy::too_many_arguments)] // one bounded start with shell/name waits and effect admission
fn start_within(
    connector: &dyn ApiConnector,
    node: Option<&dyn NodeLink>,
    correlation_id: &str,
    pane_id: &str,
    start: ShellStart,
    params: Value,
    answer_timeout: Duration,
    wait: Duration,
    name_release_wait: Duration,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Value, StartError> {
    let started = Instant::now();
    let deadline = started + wait;
    let name_deadline = started + name_release_wait;
    let shell = Shell {
        connector,
        node,
        correlation_id,
        pane_id,
        line: wire::agent_start_line_bytes(&params),
        awaits_mark: std::cell::Cell::new(matches!(start, ShellStart::Integrated { .. })),
        program: match start {
            ShellStart::Integrated { program } => Some(program),
            ShellStart::Plain => None,
        },
    };
    let mut settled = false;
    loop {
        shell.wait(started, deadline)?;
        check().map_err(StartError::NotStarted)?;
        match request_with_correlation_id(
            connector,
            correlation_id,
            "agent.start",
            params.clone(),
            answer_timeout,
        ) {
            // A prompt hook can take the terminal between the read and the
            // start. Herdr refused before typing anything, so the same start
            // is sent again once the shell holds the terminal, inside the
            // same deadline.
            Err(ApiError::Remote { code, .. })
                if code == PANE_BUSY && Instant::now() < deadline => {}
            // Herdr has not yet forgotten the agent this start replaces. It
            // refused before typing anything, so the same start is sent again
            // until it has, inside the name wait.
            Err(ApiError::Remote { code, .. })
                if code == NAME_TAKEN && Instant::now() < name_deadline =>
            {
                thread::sleep(POLL_INTERVAL);
            }
            // Herdr may still hold the name for an earlier start whose agent
            // never showed and whose deadline has passed, which it settles
            // only when the name is read: read it once, and send the start
            // again when that let it go.
            Err(ApiError::Remote { code, message }) if code == NAME_TAKEN && !settled => {
                settled = true;
                if !shell.settle_name(&params) {
                    return Err(StartError::Herdr(ApiError::Remote { code, message }));
                }
            }
            answer => {
                // The session sync reads a start Herdr then holds open only
                // when it knows Hide made it and when (herdrdev/herdr#3853).
                if let Some(terminal) = answer.as_ref().ok().and_then(wire::started_agent_terminal)
                {
                    crate::session_sync::record_start(&terminal, Instant::now());
                }
                return answer.map_err(StartError::Herdr);
            }
        }
    }
}

/// The pane a start waits on, how many bytes its start line types, whether
/// its first prompt's mark is still awaited, and from which shell.
struct Shell<'a> {
    connector: &'a dyn ApiConnector,
    node: Option<&'a dyn NodeLink>,
    correlation_id: &'a str,
    pane_id: &'a str,
    line: usize,
    awaits_mark: std::cell::Cell<bool>,
    program: Option<String>,
}

/// What a start still waited for when its wait ran out.
enum Waiting {
    /// The shell to hold the terminal.
    Terminal,
    /// The shell's first prompt, marked by the integration.
    Prompt,
    /// The shell's line editor, while the terminal keeps only this many bytes.
    LineEditor(usize),
}

impl Shell<'_> {
    /// Returns once the shell alone holds the terminal, has marked its first
    /// prompt when it was started to, and its terminal would hand the start
    /// line over whole.
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    fn wait(&self, started: Instant, deadline: Instant) -> Result<(), StartError> {
        let params =
            wire::pane_process_info_params(self.pane_id).map_err(StartError::NotStarted)?;
        loop {
            let group = request_with_connector(
                self.connector,
                "pane.process_info",
                params.clone(),
                READ_TIMEOUT,
            )
            .map_err(|error| format!("pane.process_info failed: {error}"))
            .and_then(wire::pane_process_group)
            .map_err(|message| {
                StartError::NotStarted(format!(
                    "The pane's shell could not be read, so the agent was not started: {message}"
                ))
            })?;
            // An unmarked shell is still starting whoever else holds the
            // terminal with it (a startup file's own program, or fish's
            // completion generator beside a question); the limit of a
            // terminal that still holds lines is read only while the shell
            // holds it alone.
            let waiting = match group.shell_pid {
                Some(_) if self.awaits_prompt(&group)? => Waiting::Prompt,
                Some(shell) if group.shell_holds_terminal() => match self.line_cut_at(shell)? {
                    None => return Ok(()),
                    Some(limit) => Waiting::LineEditor(limit),
                },
                _ => Waiting::Terminal,
            };
            if Instant::now() >= deadline {
                let held = match waiting {
                    Waiting::LineEditor(limit) => Some(limit),
                    _ => None,
                };
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_start",
                    "kind": "shell_wait.timeout",
                    "request": self.correlation_id,
                    "pane_id": self.pane_id,
                    "waited_ms": started.elapsed().as_millis() as u64,
                    "shell_pid": group.shell_pid,
                    "foreground_process_group_id": group.foreground_process_group_id,
                    "foreground_pids": group.foreground_pids,
                    "line_bytes": self.line,
                    "line_limit": held,
                    "prompt_unmarked": matches!(waiting, Waiting::Prompt),
                }));
                if matches!(waiting, Waiting::Prompt) {
                    return Err(StartError::ShellStarting(self.last_screen_line()));
                }
                return Err(StartError::NotStarted(match held {
                    Some(limit) => format!(
                        "The pane's shell did not start reading its command line within {} s, so the agent was not started: until it does, its terminal keeps only {limit} bytes of a typed line, and this start's line is up to {} bytes. Check what the shell's startup files wait on, then retry.",
                        SHELL_WAIT.as_secs(),
                        self.line,
                    ),
                    None => format!(
                        "The pane's shell did not reach its prompt within {} s, so the agent was not started.",
                        SHELL_WAIT.as_secs()
                    ),
                }));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Whether the start still waits for the first prompt's mark: only for a
    /// shell the integration reaches, and only until the mark is seen once.
    fn awaits_prompt(&self, group: &wire::PaneProcessGroup) -> Result<bool, StartError> {
        if !self.awaits_mark.get() {
            return Ok(false);
        }
        // A shell other than the one launched (a startup file that runs
        // `exec fish`) marks nothing, so nothing is waited for; its start is
        // confirmed after it was typed. Its name is read only while it is in
        // the foreground, so until then it is waited for.
        if let Some(name) = group
            .shell_name()
            .map(program_name)
            .filter(|name| Some(name) != self.program.as_ref())
        {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_start",
                "kind": "shell_wait.other_shell",
                "request": self.correlation_id,
                "pane_id": self.pane_id,
                "launched": self.program,
                "shell": name,
            }));
            self.awaits_mark.set(false);
            return Ok(false);
        }
        let marked = wire::pane_target_params(self.pane_id)
            .and_then(|params| {
                request_with_connector(self.connector, "pane.get", params, READ_TIMEOUT)
                    .map_err(|error| format!("pane.get failed: {error}"))
            })
            .and_then(|pane| wire::pane_token(pane, PROMPT_TOKEN))
            .map_err(|message| {
                StartError::NotStarted(format!(
                    "The pane's shell could not be read, so the agent was not started: {message}"
                ))
            })?
            .is_some();
        self.awaits_mark.set(!marked);
        Ok(!marked)
    }

    /// The last line the pane's screen shows, at most [`SCREEN_LINE_CHARS`]
    /// characters: what a shell stopped in its startup files is asking.
    /// `None` when the screen could not be read or shows nothing.
    fn last_screen_line(&self) -> Option<String> {
        let text = wire::pane_read_params(self.pane_id, "visible", 40)
            .and_then(|params| {
                request_with_connector(self.connector, "pane.read", params, READ_TIMEOUT)
                    .map_err(|error| error.to_string())
            })
            .and_then(wire::pane_text)
            .ok()?
            .text;
        let line = text
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())?;
        // The line is the operator's terminal output, shown on a card, kept
        // with the attempt and told to the agent that asked: a credential a
        // startup file printed is removed, and so is what reorders or hides
        // text.
        // Redacted whole before it is cut, so a cut never splits a
        // credential into a piece too short to be recognised.
        let line: String = line
            .chars()
            .filter(|c| !c.is_control() && !is_format_char(*c))
            .collect();
        Some(
            hide_memory::redact(&line)
                .text
                .chars()
                .take(SCREEN_LINE_CHARS)
                .collect(),
        )
    }

    /// Whether Herdr lets go of the start's name once asked about it.
    /// Herdr holds a name from its `agent.start` until that agent shows or
    /// the start's own deadline passes, but a server with no screen settles
    /// a passed deadline only when the agent is next read: `agent.get` is that
    /// read, and an answer that the name names no agent any more means the
    /// start can be sent again. A name still held is an agent's, or a start
    /// that may still show one, and Herdr's refusal is the answer.
    fn settle_name(&self, params: &Value) -> bool {
        let Some(name) = params["name"].as_str() else {
            return false;
        };
        let read = wire::agent_target_params(name)
            .map_err(ApiError::Transport)
            .and_then(|params| {
                request_with_connector(self.connector, "agent.get", params, READ_TIMEOUT)
            });
        let released =
            matches!(&read, Err(ApiError::Remote { code, .. }) if code == AGENT_NOT_FOUND);
        crate::diagnostic!(serde_json::json!({
            "component": "agent_start",
            "kind": "held_name.read",
            "request": self.correlation_id,
            "pane_id": self.pane_id,
            "released": released,
            "reason": read.err().filter(|_| !released).map(|error| error.to_string()),
        }));
        released
    }

    /// `None` when the start line reaches `shell` whole now; the limit of a
    /// terminal that would cut it, while the shell's line editor is not
    /// reading yet.
    fn line_cut_at(&self, shell: u32) -> Result<Option<usize>, StartError> {
        if self.line < SHORTEST_LINE_LIMIT {
            return Ok(None);
        }
        let refused = |reason: String| {
            StartError::NotStarted(format!(
                "The agent was not started: its start line is up to {} bytes, more than a terminal keeps before the shell's line editor reads it, and {reason}.",
                self.line
            ))
        };
        let node = self.node.ok_or_else(|| {
            refused(
                "the machine that runs the pane cannot be asked whether its shell is reading"
                    .into(),
            )
        })?;
        match crate::node_access::call_as::<LineInput>(
            node,
            Call::LineInput { pid: shell },
            READ_TIMEOUT,
        ) {
            Ok(LineInput::Keys | LineInput::Console) => Ok(None),
            Ok(LineInput::Lines { limit }) if self.line < limit as usize => Ok(None),
            Ok(LineInput::Lines { limit }) => Ok(Some(limit as usize)),
            Err(error) => Err(refused(format!(
                "the pane shell's terminal could not be read ({error})"
            ))),
        }
    }
}

#[cfg(test)]
mod real_agent_trust_tests;
#[cfg(test)]
mod real_herdr_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::*;
    use crate::fake_herdr::FakeHerdr;

    const SHELL: u32 = 4100;

    #[test]
    fn every_fork_kind_uses_delivery_readiness_and_checks_its_owner_after_the_read() {
        use crate::fork::ForkableAgent::*;
        for agent in [Claude, Codex, Grok, Pi, Omp, OpenCode, Antigravity] {
            for reported in [true, false] {
                for owner_ends in [true, false] {
                    let reads = Arc::new(AtomicUsize::new(0));
                    let observed = reads.clone();
                    let herdr = FakeHerdr::start("fork-ready-kinds", move |method, _| {
                        assert_eq!(method, "agent.get");
                        observed.fetch_add(1, Ordering::SeqCst);
                        let mut reply = json!({"type":"agent_info", "agent": {
                            "pane_id":"w1:p1", "terminal_id":"terminal-one", "workspace_id":"w1", "tab_id":"w1:t1",
                            "name":"fork-one", "agent":agent.kind(), "launch_pending":false,
                            "focused":false, "agent_status":"idle", "revision":1
                        }});
                        if reported {
                            reply["agent"]["interactive_ready"] = json!(true);
                        }
                        reply
                    });
                    let result = confirm_fork_started(
                        &herdr.connector(),
                        "w1:p1",
                        "fork-one",
                        agent.kind(),
                        &|| {
                            if owner_ends && reads.load(Ordering::SeqCst) > 0 {
                                Err("daemon ended".into())
                            } else {
                                Ok(())
                            }
                        },
                        Instant::now() + Duration::from_secs(5),
                    );
                    assert_eq!(
                        result.is_ok(),
                        !owner_ends,
                        "{agent:?}, reported={reported}"
                    );
                    assert_eq!(reads.load(Ordering::SeqCst), 1);
                }
            }
        }
    }

    #[test]
    fn a_still_pending_fork_reaches_its_deadline_without_typing() {
        let herdr = FakeHerdr::start("fork-remains-pending", |method, _| {
            assert_eq!(method, "agent.get", "input never reaches the shell");
            json!({"type":"agent_info", "agent": {
                "pane_id":"w1:p1", "terminal_id":"terminal-one", "workspace_id":"w1", "tab_id":"w1:t1",
                "name":"fork-one", "agent":"claude", "launch_pending":true,
                "interactive_ready":false, "focused":false, "agent_status":"idle", "revision":1
            }})
        });
        let result = confirm_fork_started(
            &herdr.connector(),
            "w1:p1",
            "fork-one",
            "claude",
            &|| Ok(()),
            Instant::now() + Duration::from_millis(200),
        );
        assert!(result.unwrap_err().contains("deadline elapsed"));
        assert!(!herdr.methods().is_empty());
    }

    #[test]
    fn a_fork_waits_for_its_named_agent_instead_of_releasing_keys_to_a_shell() {
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        let herdr = FakeHerdr::start("fork-start-observed", move |method, params| {
            assert_eq!(method, "agent.get");
            assert_eq!(params["target"], "fork-one");
            let read = observed.fetch_add(1, Ordering::SeqCst);
            json!({"type":"agent_info", "agent": {
                "pane_id":"w1:p1", "terminal_id":"terminal-one", "workspace_id":"w1", "tab_id":"w1:t1",
                "name":"fork-one", "agent":if read == 0 { Value::Null } else { json!("claude") },
                "launch_pending":read < 2, "interactive_ready":read >= 2,
                "focused":false, "agent_status":"idle", "revision":1
            }})
        });
        confirm_fork_started(
            &herdr.connector(),
            "w1:p1",
            "fork-one",
            "claude",
            &|| Ok(()),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            reads.load(Ordering::SeqCst),
            3,
            "a reserved name and a still-starting process are not ready"
        );
    }

    #[test]
    fn a_fork_with_no_confirmed_agent_refuses_input() {
        for case in [
            "gone",
            "pane-changed",
            "name-changed",
            "kind-changed",
            "shell",
            "owner-ended",
            "deadline",
        ] {
            let reads = Arc::new(AtomicUsize::new(0));
            let observed = reads.clone();
            let herdr = FakeHerdr::start_with_errors("fork-start-refused", move |method, _| {
                assert_eq!(method, "agent.get", "confirmation never types keys");
                let read = observed.fetch_add(1, Ordering::SeqCst);
                if case == "gone" || read > 0 {
                    return Err(("agent_not_found".into(), "start failed".into()));
                }
                let agent = json!({
                    "pane_id":if case == "pane-changed" { "w1:p2" } else { "w1:p1" },
                    "name":if case == "name-changed" { "replacement" } else { "fork-one" },
                    "agent":if case == "kind-changed" { json!("pi") } else if case == "shell" { Value::Null } else { json!("claude") },
                    "terminal_id":"terminal-one", "workspace_id":"w1", "tab_id":"w1:t1",
                    "interactive_ready":true, "focused":false, "agent_status":"idle", "revision":1
                });
                Ok(json!({"type":"agent_info", "agent":agent}))
            });
            let deadline = Instant::now()
                + if case == "deadline" {
                    Duration::ZERO
                } else {
                    Duration::from_millis(200)
                };
            let result = confirm_fork_started(
                &herdr.connector(),
                "w1:p1",
                "fork-one",
                "claude",
                &|| {
                    if case == "owner-ended" {
                        Err("fork ended".into())
                    } else {
                        Ok(())
                    }
                },
                deadline,
            );
            assert!(result.is_err(), "{case}");
            if matches!(case, "owner-ended" | "deadline") {
                assert_eq!(reads.load(Ordering::SeqCst), 0);
            }
        }
    }

    fn process_info(foreground: u32, pids: &[u32]) -> Value {
        let processes: Vec<Value> = pids
            .iter()
            .map(|pid| json!({"pid": pid, "name": if *pid == SHELL { "zsh" } else { "direnv" }}))
            .collect();
        json!({"type": "pane_process_info", "process_info": {
            "pane_id": "w1:p1", "shell_pid": SHELL, "foreground_process_group_id": foreground,
            "foreground_processes": processes
        }})
    }

    fn started() -> Value {
        json!({"type": "agent_started", "argv": [], "agent": {
            "pane_id": "w1:p1", "terminal_id": "term_1", "workspace_id": "w1",
            "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1
        }})
    }

    fn start(herdr: &FakeHerdr, wait: Duration) -> Result<Value, StartError> {
        start_within(
            &herdr.connector(),
            None,
            "test:start",
            "w1:p1",
            ShellStart::Plain,
            json!({"pane_id": "w1:p1"}),
            Duration::from_secs(5),
            wait,
            Duration::ZERO,
            &|| Ok(()),
        )
    }

    fn start_reusing_name(herdr: &FakeHerdr, name_wait: Duration) -> Result<Value, StartError> {
        start_within(
            &herdr.connector(),
            None,
            "test:start",
            "w1:p1",
            ShellStart::Plain,
            json!({"pane_id": "w1:p1"}),
            Duration::from_secs(5),
            Duration::from_secs(5),
            name_wait,
            &|| Ok(()),
        )
    }

    #[test]
    fn the_start_waits_until_the_shell_alone_holds_the_terminal() {
        let reads = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&reads);
        let herdr = FakeHerdr::start("agent-start-wait", move |method, _| match method {
            // Startup files first run a command in their own group, then one
            // in the shell's group, and only then leave the shell alone.
            "pane.process_info" => match seen.fetch_add(1, Ordering::SeqCst) {
                0 => process_info(4200, &[4200]),
                1 => process_info(SHELL, &[SHELL, 4201]),
                _ => process_info(SHELL, &[SHELL]),
            },
            "agent.start" => started(),
            other => panic!("unexpected {other}"),
        });
        assert!(start(&herdr, Duration::from_secs(5)).is_ok());
        assert_eq!(
            herdr.methods(),
            [
                "pane.process_info",
                "pane.process_info",
                "pane.process_info",
                "agent.start"
            ]
        );
    }

    #[test]
    fn a_start_refused_as_busy_waits_for_the_shell_and_is_sent_again() {
        let starts = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("agent-start-busy", move |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" if seen.fetch_add(1, Ordering::SeqCst) == 0 => Err((
                    PANE_BUSY.into(),
                    "agent target pane w1:p1 is not an available shell".into(),
                )),
                "agent.start" => Ok(started()),
                other => panic!("unexpected {other}"),
            });
        assert!(start(&herdr, Duration::from_secs(5)).is_ok());
        assert_eq!(
            herdr.methods(),
            [
                "pane.process_info",
                "agent.start",
                "pane.process_info",
                "agent.start"
            ]
        );
    }

    #[test]
    fn a_changed_session_after_readiness_or_a_busy_refusal_never_types_another_start() {
        for previously_admitted in [false, true] {
            let herdr = FakeHerdr::start_with_errors("checked-start", |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" => Err((PANE_BUSY.into(), "busy before typing".into())),
                other => panic!("unexpected {other}"),
            });
            let checks = std::cell::Cell::new(0);
            let answer = start_within(
                &herdr.connector(),
                None,
                "checked",
                "w1:p1",
                ShellStart::Plain,
                json!({"pane_id":"w1:p1"}),
                Duration::from_secs(5),
                Duration::from_secs(5),
                Duration::ZERO,
                &|| {
                    let count = checks.get();
                    checks.set(count + 1);
                    if previously_admitted && count == 0 {
                        Ok(())
                    } else {
                        Err("session_route_unconfirmed".into())
                    }
                },
            );
            assert!(
                matches!(answer, Err(StartError::NotStarted(reason)) if reason == "session_route_unconfirmed")
            );
            let typed_attempts = herdr
                .methods()
                .iter()
                .filter(|method| *method == "agent.start")
                .count();
            assert_eq!(typed_attempts, usize::from(previously_admitted));
        }
    }

    #[test]
    fn a_shell_that_never_reaches_its_prompt_types_nothing_and_says_so() {
        let herdr = FakeHerdr::start("agent-start-never", |method, _| match method {
            "pane.process_info" => process_info(4200, &[4200]),
            other => panic!("unexpected {other}"),
        });
        let Err(StartError::NotStarted(message)) = start(&herdr, Duration::from_millis(300)) else {
            panic!("a shell that never holds the terminal is not a start");
        };
        assert!(message.contains("did not reach its prompt"), "{message}");
        assert!(!herdr.methods().iter().any(|method| method == "agent.start"));
    }

    #[test]
    fn another_refusal_is_herdrs_answer_at_once() {
        let herdr = FakeHerdr::start_with_errors("agent-start-refused", |method, _| match method {
            "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
            "agent.start" => Err((
                "agent_name_taken".into(),
                "agent name one is already used".into(),
            )),
            other => panic!("unexpected {other}"),
        });
        let Err(StartError::Herdr(ApiError::Remote { code, .. })) =
            start(&herdr, Duration::from_secs(5))
        else {
            panic!("a refusal other than a busy pane is not retried");
        };
        assert_eq!(code, "agent_name_taken");
        assert_eq!(herdr.methods(), ["pane.process_info", "agent.start"]);
    }

    /// Herdr forgets an ended agent a moment after its process is gone, so a
    /// start under the same name is refused until then and must be sent
    /// again, once the refusal is gone, and not before.
    #[test]
    fn a_start_under_the_ended_agents_name_is_sent_again_until_herdr_lets_go() {
        let starts = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("agent-start-name", move |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" if seen.fetch_add(1, Ordering::SeqCst) < 2 => {
                    Err((NAME_TAKEN.into(), "agent name two is already used".into()))
                }
                "agent.start" => Ok(started()),
                other => panic!("unexpected {other}"),
            });
        assert!(start_reusing_name(&herdr, Duration::from_secs(5)).is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 3);
    }

    fn start_named(herdr: &FakeHerdr) -> Result<Value, StartError> {
        start_within(
            &herdr.connector(),
            None,
            "test:start",
            "w1:p1",
            ShellStart::Plain,
            json!({"pane_id": "w1:p1", "name": "factory-hide-t1", "kind": "claude"}),
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::ZERO,
            &|| Ok(()),
        )
    }

    /// Herdr holds a name for a start whose agent never showed (its line
    /// was cut, or its program exited at once) until that start's deadline,
    /// and a server with no screen lets go only when the name is read. A
    /// start refused under it reads the name once and, when Herdr let go,
    /// starts rather than being refused for good.
    #[test]
    fn a_name_herdr_lets_go_when_read_is_started_again() {
        let starts = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("agent-start-settled", move |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" if seen.fetch_add(1, Ordering::SeqCst) == 0 => Err((
                    NAME_TAKEN.into(),
                    "agent name factory-hide-t1 is already used".into(),
                )),
                "agent.start" => Ok(started()),
                "agent.get" => Err((
                    AGENT_NOT_FOUND.into(),
                    "agent target factory-hide-t1 not found".into(),
                )),
                other => panic!("unexpected {other}"),
            });
        assert!(start_named(&herdr).is_ok());
        let calls = herdr.calls();
        let (_, read) = calls
            .iter()
            .find(|(method, _)| method == "agent.get")
            .expect("the held name is read");
        assert_eq!(read["target"], "factory-hide-t1");
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    /// A name Herdr still holds after the read is an agent's, or a start
    /// that may still show one: the refusal is the answer.
    #[test]
    fn a_name_still_held_after_the_read_is_herdrs_refusal() {
        let herdr =
            FakeHerdr::start_with_errors("agent-start-held", move |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" => Err((
                    NAME_TAKEN.into(),
                    "agent name factory-hide-t1 is already used".into(),
                )),
                "agent.get" => Ok(json!({"type": "agent_info", "agent": {
                    "pane_id": "w1:p2", "terminal_id": "term_2", "workspace_id": "w1",
                    "tab_id": "w1:t2", "name": "factory-hide-t1", "focused": false,
                    "agent_status": "unknown", "revision": 0, "launch_pending": true}})),
                other => panic!("unexpected {other}"),
            });
        let Err(StartError::Herdr(ApiError::Remote { code, .. })) = start_named(&herdr) else {
            panic!("a name someone else holds is refused");
        };
        assert_eq!(code, NAME_TAKEN);
        assert_eq!(
            herdr.methods(),
            ["pane.process_info", "agent.start", "agent.get"]
        );
    }

    /// The wait is bounded, and an ordinary start never waits for a name: it
    /// belongs to someone else.
    #[test]
    fn a_name_that_is_never_released_is_herdrs_answer_and_an_ordinary_start_does_not_wait() {
        let herdr =
            FakeHerdr::start_with_errors("agent-start-name-held", |method, _| match method {
                "pane.process_info" => Ok(process_info(SHELL, &[SHELL])),
                "agent.start" => Err((NAME_TAKEN.into(), "agent name two is already used".into())),
                other => panic!("unexpected {other}"),
            });
        let Err(StartError::Herdr(ApiError::Remote { code, .. })) =
            start_reusing_name(&herdr, Duration::from_millis(300))
        else {
            panic!("a name that is never released is refused");
        };
        assert_eq!(code, NAME_TAKEN);
        let before = herdr.methods().len();
        let Err(StartError::Herdr(ApiError::Remote { code, .. })) =
            start(&herdr, Duration::from_secs(5))
        else {
            panic!("an ordinary start takes the refusal as the answer");
        };
        assert_eq!(code, NAME_TAKEN);
        assert_eq!(
            herdr.methods().len() - before,
            2,
            "one process read and one start, nothing sent again"
        );
    }

    /// The pane's node, answering how the shell's terminal takes input with
    /// the next of `answers` (the last one repeats), and counting the reads.
    struct Terminal {
        answers: Vec<Result<LineInput, String>>,
        reads: AtomicUsize,
    }

    impl Terminal {
        fn answering(answers: Vec<Result<LineInput, String>>) -> Self {
            Self {
                answers,
                reads: AtomicUsize::new(0),
            }
        }
    }

    impl NodeLink for Terminal {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            let Call::LineInput { pid } = call else {
                panic!("a start asks its node only how the shell's terminal reads");
            };
            assert_eq!(pid, SHELL, "the shell Herdr named is the one read");
            let read = self.reads.fetch_add(1, Ordering::SeqCst);
            match &self.answers[read.min(self.answers.len() - 1)] {
                Ok(input) => Ok(crate::node_access::LinkAnswer::Parsed(
                    serde_json::to_value(input).unwrap(),
                )),
                Err(reason) => Err(crate::node_access::LinkError::NotConnected(reason.clone())),
            }
        }
    }

    fn shell_alone(name: &str) -> FakeHerdr {
        FakeHerdr::start(name, |method, _| match method {
            "pane.process_info" => process_info(SHELL, &[SHELL]),
            "agent.start" => started(),
            other => panic!("unexpected {other}"),
        })
    }

    /// A first prompt of `bytes` bytes, the argument Herdr types last.
    fn start_with_prompt(
        herdr: &FakeHerdr,
        node: Option<&dyn NodeLink>,
        bytes: usize,
        wait: Duration,
    ) -> Result<Value, StartError> {
        start_within(
            &herdr.connector(),
            node,
            "test:start",
            "w1:p1",
            ShellStart::Plain,
            json!({"pane_id": "w1:p1", "args": ["--", "가".repeat(bytes / 3)]}),
            Duration::from_secs(5),
            wait,
            Duration::ZERO,
            &|| Ok(()),
        )
    }

    fn typed(herdr: &FakeHerdr) -> usize {
        herdr
            .methods()
            .iter()
            .filter(|method| *method == "agent.start")
            .count()
    }

    /// A shell that holds the terminal while its startup files still run
    /// keeps a typed line in canonical mode, where macOS drops what passes
    /// 1,024 bytes; the start waits until the line editor reads keys.
    #[test]
    fn a_long_start_line_waits_for_the_shells_line_editor() {
        let herdr = shell_alone("agent-start-editor");
        let terminal = Terminal::answering(vec![
            Ok(LineInput::Lines { limit: 1024 }),
            Ok(LineInput::Lines { limit: 1024 }),
            Ok(LineInput::Keys),
        ]);
        assert!(start_with_prompt(&herdr, Some(&terminal), 4800, Duration::from_secs(5)).is_ok());
        assert_eq!(terminal.reads.load(Ordering::SeqCst), 3);
        assert_eq!(typed(&herdr), 1);
        assert_eq!(
            herdr.methods().last().map(String::as_str),
            Some("agent.start")
        );
    }

    /// A line that fits the terminal's own limit reaches the shell whole
    /// even before the line editor reads (a Linux terminal keeps 4,096
    /// bytes), and a console cuts no line.
    #[test]
    fn a_long_line_the_terminal_keeps_whole_starts_without_waiting() {
        for input in [LineInput::Lines { limit: 4096 }, LineInput::Console] {
            let herdr = shell_alone("agent-start-fits");
            let terminal = Terminal::answering(vec![Ok(input)]);
            assert!(
                start_with_prompt(&herdr, Some(&terminal), 2400, Duration::from_secs(5)).is_ok()
            );
            assert_eq!(terminal.reads.load(Ordering::SeqCst), 1);
            assert_eq!(typed(&herdr), 1);
        }
    }

    /// A line shorter than every terminal's limit is typed as soon as the
    /// shell holds the terminal, with no node to ask: a shell that has no
    /// line editor still starts a plain command.
    #[test]
    fn a_short_start_line_asks_no_node() {
        let herdr = shell_alone("agent-start-short");
        let terminal = Terminal::answering(vec![Ok(LineInput::Lines { limit: 1024 })]);
        assert!(start_with_prompt(&herdr, Some(&terminal), 600, Duration::from_secs(5)).is_ok());
        assert_eq!(terminal.reads.load(Ordering::SeqCst), 0);
        assert!(start_with_prompt(&herdr, None, 600, Duration::from_secs(5)).is_ok());
        assert_eq!(typed(&herdr), 2);
    }

    /// The wait is the shell wait: past it a terminal still holding lines
    /// gets nothing typed, never a cut line, and the reason names the limit.
    #[test]
    fn a_terminal_that_never_leaves_line_mode_gets_nothing_typed() {
        let herdr = shell_alone("agent-start-canonical");
        let terminal = Terminal::answering(vec![Ok(LineInput::Lines { limit: 1024 })]);
        let Err(StartError::NotStarted(message)) =
            start_with_prompt(&herdr, Some(&terminal), 4800, Duration::from_millis(300))
        else {
            panic!("a line the terminal would cut is not typed");
        };
        assert!(message.contains("1024 bytes"), "{message}");
        assert_eq!(typed(&herdr), 0);
    }

    /// Without a node to ask, or with one that cannot read the terminal, a
    /// long line is not typed on a guess.
    #[test]
    fn a_long_line_whose_terminal_cannot_be_read_gets_nothing_typed() {
        let herdr = shell_alone("agent-start-unread");
        let Err(StartError::NotStarted(message)) =
            start_with_prompt(&herdr, None, 4800, Duration::from_secs(5))
        else {
            panic!("no node, no long line");
        };
        assert!(message.contains("cannot be asked"), "{message}");
        let terminal = Terminal::answering(vec![Err("the helper is gone".into())]);
        let Err(StartError::NotStarted(message)) =
            start_with_prompt(&herdr, Some(&terminal), 4800, Duration::from_secs(5))
        else {
            panic!("an unread terminal is not a reading one");
        };
        assert!(message.contains("the helper is gone"), "{message}");
        assert_eq!(typed(&herdr), 0);
    }

    fn pane_with(tokens: Value) -> Value {
        json!({"type": "pane_info", "pane": {"workspace_id": "w1", "tab_id": "w1:t1",
            "pane_id": "w1:p1", "terminal_id": "term_1", "focused": false,
            "agent_status": "idle", "revision": 0, "tokens": tokens}})
    }

    fn start_integrated(herdr: &FakeHerdr, wait: Duration) -> Result<Value, StartError> {
        start_within(
            &herdr.connector(),
            None,
            "test:start",
            "w1:p1",
            ShellStart::Integrated {
                program: "zsh".into(),
            },
            json!({"pane_id": "w1:p1"}),
            Duration::from_secs(5),
            wait,
            Duration::ZERO,
            &|| Ok(()),
        )
    }

    /// A startup question holds the terminal with the shell alone in the
    /// foreground, which Herdr reads as a shell ready for the command; the
    /// first prompt's mark is what says the startup files are done.
    #[test]
    fn an_integrated_shell_is_typed_into_only_after_its_first_prompt_is_marked() {
        let reads = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&reads);
        let herdr = FakeHerdr::start("agent-start-mark", move |method, _| match method {
            "pane.process_info" => process_info(SHELL, &[SHELL]),
            "pane.get" if seen.fetch_add(1, Ordering::SeqCst) < 2 => pane_with(json!({})),
            "pane.get" => pane_with(json!({PROMPT_TOKEN: "1"})),
            "agent.start" => started(),
            other => panic!("unexpected {other}"),
        });
        assert!(start_integrated(&herdr, Duration::from_secs(5)).is_ok());
        let methods = herdr.methods();
        assert_eq!(methods.iter().filter(|m| *m == "pane.get").count(), 3);
        assert_eq!(methods.last().map(String::as_str), Some("agent.start"));
    }

    /// The question holds the terminal with the shell alone, or with a
    /// program of the shell's own beside it (fish starts its completion
    /// generator when the question reads): either way the shell is starting.
    #[test]
    fn a_shell_held_by_a_startup_question_gets_nothing_typed_and_names_the_question() {
        for foreground in [&[SHELL][..], &[SHELL, SHELL + 1]] {
            let foreground = foreground.to_vec();
            let herdr = FakeHerdr::start("agent-start-question", move |method, _| match method {
                "pane.process_info" => process_info(SHELL, &foreground),
                "pane.get" => pane_with(json!({})),
                "pane.read" => json!({"type": "pane_read", "read": {"pane_id": "w1:p1",
                    "workspace_id": "w1", "tab_id": "w1:t1", "source": "visible",
                    "format": "text", "revision": 1, "truncated": false,
                    "text": "Last login: today\n[oh-my-zsh] Would you like to update? [Y/n] \n\n"}}),
                other => panic!("unexpected {other}"),
            });
            let Err(StartError::ShellStarting(line)) =
                start_integrated(&herdr, Duration::from_millis(300))
            else {
                panic!("an unmarked shell is not a start");
            };
            assert_eq!(
                line.as_deref(),
                Some("[oh-my-zsh] Would you like to update? [Y/n]")
            );
            assert!(!herdr.methods().iter().any(|method| method == "agent.start"));
        }
    }

    /// The question line is the operator's terminal output: a credential a
    /// startup file printed, and characters that hide or reorder text, never
    /// leave the pane with it.
    #[test]
    fn a_question_line_carries_no_credential_and_no_hidden_characters() {
        let herdr = FakeHerdr::start("agent-start-secret", move |method, _| match method {
            "pane.process_info" => process_info(SHELL, &[SHELL]),
            "pane.get" => pane_with(json!({})),
            "pane.read" => json!({"type": "pane_read", "read": {"pane_id": "w1:p1",
                "workspace_id": "w1", "tab_id": "w1:t1", "source": "visible",
                "format": "text", "revision": 1, "truncated": false,
                "text": "export api_key=abcd1234efgh5678\u{202E}\u{200B} Continue? [y/N]\n"}}),
            other => panic!("unexpected {other}"),
        });
        let Err(StartError::ShellStarting(Some(line))) =
            start_integrated(&herdr, Duration::from_millis(300))
        else {
            panic!("an unmarked shell is not a start");
        };
        assert_eq!(line, "export [REDACTED] Continue? [y/N]");
    }

    /// A credential that runs past the most of a line a refusal carries is
    /// still recognised, so no piece of it is left.
    #[test]
    fn a_credential_at_the_end_of_a_long_question_line_leaves_no_piece() {
        let text = format!("{} sk-{} [y/N]\n", "x".repeat(105), "A".repeat(30));
        let herdr = FakeHerdr::start("agent-start-cut", move |method, _| match method {
            "pane.process_info" => process_info(SHELL, &[SHELL]),
            "pane.get" => pane_with(json!({})),
            "pane.read" => json!({"type": "pane_read", "read": {"pane_id": "w1:p1",
                "workspace_id": "w1", "tab_id": "w1:t1", "source": "visible",
                "format": "text", "revision": 1, "truncated": false, "text": text}}),
            other => panic!("unexpected {other}"),
        });
        let Err(StartError::ShellStarting(Some(line))) =
            start_integrated(&herdr, Duration::from_millis(300))
        else {
            panic!("an unmarked shell is not a start");
        };
        assert!(!line.contains("sk-"), "{line}");
        assert_eq!(line.chars().count(), SCREEN_LINE_CHARS);
    }

    /// Herdr names a Windows shell by its full path: the launched Windows
    /// PowerShell is still the shell whose mark is waited for.
    #[test]
    fn a_windows_shell_named_by_its_full_path_is_the_launched_one() {
        let herdr = FakeHerdr::start("agent-start-windows", |method, _| match method {
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                "pane_id": "w1:p1", "shell_pid": SHELL, "foreground_process_group_id": SHELL,
                "foreground_processes": [{"pid": SHELL, "name": "powershell.exe",
                    "argv0": "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"}]}}),
            "pane.get" => pane_with(json!({})),
            "pane.read" => json!({"type": "pane_read", "read": {"pane_id": "w1:p1",
                "workspace_id": "w1", "tab_id": "w1:t1", "source": "visible",
                "format": "text", "revision": 1, "truncated": false,
                "text": "Update the framework now? [Y/n]\n"}}),
            other => panic!("unexpected {other}"),
        });
        let answer = start_within(
            &herdr.connector(),
            None,
            "test:start",
            "w1:p1",
            ShellStart::Integrated {
                program: "powershell".into(),
            },
            json!({"pane_id": "w1:p1"}),
            Duration::from_secs(5),
            Duration::from_millis(300),
            Duration::ZERO,
            &|| Ok(()),
        );
        assert!(
            matches!(answer, Err(StartError::ShellStarting(_))),
            "{answer:?}"
        );
        assert!(!herdr.methods().iter().any(|method| method == "agent.start"));
    }

    /// A startup file that replaces the launched shell with another one
    /// (`exec fish` in a `.zshrc`) leaves a shell that never marks a prompt,
    /// so nothing waits for one.
    #[test]
    fn a_shell_other_than_the_launched_one_is_started_without_waiting_for_a_mark() {
        let herdr = FakeHerdr::start("agent-start-exec", |method, _| match method {
            "pane.process_info" => json!({"type": "pane_process_info", "process_info": {
                "pane_id": "w1:p1", "shell_pid": SHELL, "foreground_process_group_id": SHELL,
                "foreground_processes": [{"pid": SHELL, "name": "-fish"}]}}),
            "agent.start" => started(),
            other => panic!("unexpected {other}"),
        });
        assert!(start_integrated(&herdr, Duration::from_secs(5)).is_ok());
        assert_eq!(herdr.methods(), ["pane.process_info", "agent.start"]);
    }

    #[test]
    fn a_launched_shell_is_named_as_its_pane_lists_it() {
        let launch = |shell: &str| ShellLaunch {
            command: vec![shell.into(), "-l".into()],
            readiness: hide_node_link::protocol::ShellReadiness::Integrated,
            env: Default::default(),
        };
        for (shell, program) in [
            ("/bin/zsh", "zsh"),
            ("/opt/homebrew/bin/fish", "fish"),
            (
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                "powershell",
            ),
            ("powershell.exe", "powershell"),
        ] {
            assert_eq!(
                ShellStart::of_tab(true, Some(&launch(shell))),
                ShellStart::Integrated {
                    program: program.into()
                }
            );
        }
        assert_eq!(
            ShellStart::of_tab(false, Some(&launch("/bin/zsh"))),
            ShellStart::Plain
        );
        assert_eq!(ShellStart::of_tab(true, None), ShellStart::Plain);
    }

    #[test]
    fn claude_and_agy_have_a_worktree_trusted_on_its_node_before_they_start() {
        let home = tempfile::tempdir().unwrap();
        let agy = home.path().join(".gemini/antigravity-cli/settings.json");
        let claude = home.path().join(".claude.json");
        std::fs::create_dir_all(agy.parent().unwrap()).unwrap();
        let node = hide_node::Local::new(Some(home.path().to_path_buf()));
        // The core names the worktree in its wire spelling; each agent's
        // settings hold the native one, resolved.
        let tree = |name: &str| home.path().join(name);
        let wire = |name: &str| hide_platform::path::to_wire(&tree(name)).unwrap();
        let resolved = |name: &str| {
            hide_platform::fs::identity::canonical(&tree(name))
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };
        for name in ["tree-1", "tree-2"] {
            std::fs::create_dir_all(tree(name).join(".git")).unwrap();
        }
        std::fs::create_dir_all(tree("plain")).unwrap();
        trust_worktree(Some(&node), "grok", &wire("tree-1")).unwrap();
        assert!(
            !agy.exists() && !claude.exists(),
            "Grok asks no folder trust"
        );
        trust_worktree(Some(&node), "antigravity", &wire("tree-1")).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&agy).unwrap()).unwrap();
        assert_eq!(
            written["trustedWorkspaces"],
            serde_json::json!([resolved("tree-1")])
        );
        trust_worktree(Some(&node), "claude", &wire("tree-1")).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&claude).unwrap()).unwrap();
        let key = if cfg!(windows) {
            resolved("tree-1").replace('\\', "/")
        } else {
            resolved("tree-1")
        };
        assert_eq!(
            written["projects"][key.as_str()]["hasTrustDialogAccepted"],
            true
        );
        // A folder that is not a worktree is never trusted.
        for kind in ["antigravity", "claude"] {
            let refused = trust_worktree(Some(&node), kind, &wire("plain")).unwrap_err();
            assert!(refused.starts_with("agent_trust_failed"), "{refused}");
        }
        // A trust that cannot be recorded stops the start with its reason,
        // naming the agent.
        std::fs::write(&agy, "{ not json").unwrap();
        let refused = trust_worktree(Some(&node), "antigravity", &wire("tree-2")).unwrap_err();
        assert!(
            refused.starts_with("agent_trust_failed: Antigravity's trust of"),
            "{refused}"
        );
        assert!(
            trust_worktree(None, "claude", &wire("tree-2"))
                .unwrap_err()
                .starts_with("agent_trust_unavailable")
        );
    }
}
