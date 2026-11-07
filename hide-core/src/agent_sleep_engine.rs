//! The one place agent sleep relies on the engine and on the operating system
//! (PRD agent-sleep D-20).
//!
//! The engine has no hibernate: it ends nothing on request and
//! forgets an agent whose process ended. So sleeping is Hide ending the
//! agent, which returns the pane to its shell: first with the agent's own
//! quit key, then, for an agent still running, by the pane's node ending
//! its foreground job (SIGTERM to its process group on Unix, the agent's
//! process tree at once on Windows). Waking is the engine's own `agent.start` in
//! that same pane with the provider's resume arguments.
//! When the engine offers a hibernate or an
//! exit mark of its own, this file is what changes; the decision, the
//! records and the screens in `agent_sleep.rs` and the runtime stay.
//!
//! Every call here runs on a worker thread, never under `Mutex<Runtime>`.

use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use hide_node_link::holders::HolderRenewal;
use hide_node_link::process::GroupEnd;
use hide_node_link::protocol::Call;

use crate::node_access::{NodeLink, call_as};

use crate::agent_sleep::WakeMode;
use crate::agent_start::StartError;
use crate::engine::{AgentLaunch, Engine, EngineError, PaneProcessGroup, SharedEngine, Wait};
use crate::handle::ChangeNotifier;
use crate::input_gate::Drain;
use crate::live::failed;
use crate::runtime::Runtime;

mod dormant;
pub(crate) use dormant::{
    DormantStartOutcome, DormantStatus, DormantTabOutcome, DormantWork, dormant_close_gate,
    spawn_dormant_start, spawn_dormant_status, spawn_dormant_tab, wake_name as dormant_wake_name,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a wake waits for its pane's holder to be renewed: the earlier
/// holder's end, then the new one's start, as the node bounds them.
const RENEW_TIMEOUT: Duration = Duration::from_secs(90);
/// How long an ended agent has to hand the terminal back to its shell before
/// the sleep is given up and the agent is left awake (B8).
const END_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest the end waits for a write already admitted to the pane; a
/// write has its own request timeout (`REQUEST_TIMEOUT`), so this is the hang guard.
const WRITE_DRAIN_LIMIT: Duration = Duration::from_secs(6);
const END_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long an agent asked to quit with its own key has to hand the
/// terminal back before it is ended instead.
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// The most presses of the quit key one end makes: one that clears a draft,
/// then the quit and the press that confirms it.
const QUIT_PRESSES: usize = 3;
/// How often a quit reads the pane while it waits: the agent's next press
/// follows its hint inside the shortest double-press window declared (Claude
/// Code's 800 ms, `LaunchDialect::quit_again`).
const QUIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Why an agent was not ended. `Busy` is the agent having started working, or
/// waiting for the operator, since the decision was made: nothing was
/// signalled, and a Reopen says so as such. Everything else is `Failed`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EndError {
    Busy(String),
    Failed(String),
}

impl std::fmt::Display for EndError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

/// Ends the agent in `pane_id` and returns the state change sequence the engine
/// reported for it just before, which is how a later listing of the same
/// agent is told from a new one.
///
/// It reads the agent again first, because the decision was made on a list
/// up to a second old and a parent's prompt may have set it working since.
/// The end reaches the pane's foreground job only, and only when that job is
/// not the shell's: the agent and what it started (MCP servers, tool
/// processes; its group on Unix, its process tree on Windows, where the engine
/// names the agent's process as the group) end, the shell and every other
/// pane stay (B7).
///
/// The agent is first asked to quit with its own key, as the operator quits
/// it, so it puts back the terminal modes it set: the engine wraps the wake's
/// start line in bracketed paste while the pane's terminal says the mode is
/// on, and a PowerShell prompt reads that wrapping as keys, so an agent ended
/// without its own quit left a Windows pane whose next start never ran
/// (Herdr's behavior, herdrdev/herdr#5371, which the engine follows), and an
/// agent ended by a signal leaves its keyboard protocol, alternate screen and mouse reports on for the shell
/// (`quit_on_key`). Only an agent still running after `EXIT_GRACE` is
/// ended by the node.
pub(crate) fn end_agent(
    engine: &dyn Engine,
    node: &dyn NodeLink,
    pane_id: &str,
    kind: &str,
) -> Result<u64, EndError> {
    let state = engine
        .agent_state(Wait::Each(REQUEST_TIMEOUT), pane_id)
        .map_err(|error| EndError::Failed(error.within("agent.get failed")))?;
    if !matches!(state.status.as_str(), "idle" | "done") {
        return Err(EndError::Busy(format!("the agent is {} now", state.status)));
    }
    if !state.agent.as_deref().is_some_and(|agent| {
        hide_agent_adapter::canonical_kind(agent)
            .eq_ignore_ascii_case(hide_agent_adapter::canonical_kind(kind))
    }) {
        return Err(EndError::Failed(format!(
            "the pane now runs {} rather than {kind}",
            state.agent.as_deref().unwrap_or("no agent")
        )));
    }
    let group = process_group(engine, pane_id).map_err(EndError::Failed)?;
    let (Some(shell), Some(foreground)) = (group.shell_pid, group.foreground_process_group_id)
    else {
        return Err(EndError::Failed(
            "The engine reported no shell or foreground process group for the pane".into(),
        ));
    };
    // kill(-0) signals Hide's own group and kill(-1) every process the user
    // owns, so a group id that is not a real group never reaches the signal.
    if foreground <= 1 || shell <= 1 {
        return Err(EndError::Failed(format!(
            "The engine reported process group {foreground} and shell {shell}, which are not a pane's"
        )));
    }
    if foreground == shell {
        return Err(EndError::Failed(
            "the pane's shell already holds the terminal".into(),
        ));
    }
    if let Some(quit) = Quit::of(kind) {
        match quit.run(engine, pane_id, shell)? {
            Quitting::Quit => return Ok(state.state_change_seq),
            Quitting::Unanswered { reason, presses } => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.exit_keys_unanswered",
                    "pane_id": pane_id,
                    "provider": kind,
                    "reason": reason,
                    "presses": presses,
                }));
            }
        }
    }
    // The job the engine reported as the pane's foreground ends. One that ended on
    // its own since is gone, and the shell is waited for all the same.
    let ended = call_as::<GroupEnd>(
        node,
        Call::TerminateGroup { leader: foreground },
        REQUEST_TIMEOUT,
    )
    .map_err(|error| {
        EndError::Failed(format!("ending process group {foreground} failed: {error}"))
    })?;
    if let GroupEnd::Ended { left } = &ended
        && !left.is_empty()
    {
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.end_left_running",
            "pane_id": pane_id,
            "provider": kind,
            "leader": foreground,
            "left": left
                .iter()
                .map(|process| serde_json::json!({"pid": process.pid, "reason": process.reason}))
                .collect::<Vec<_>>(),
        }));
    }
    if shell_takes_terminal_within(engine, pane_id, shell, END_TIMEOUT)? {
        return Ok(state.state_change_seq);
    }
    Err(EndError::Failed(format!(
        "the agent did not hand the terminal back to its shell within {} s",
        END_TIMEOUT.as_secs()
    )))
}

/// An agent's own quit, as its sleep dialect declares it.
struct Quit {
    key: &'static str,
    again: Option<&'static str>,
    composer: Option<hide_agent_adapter::composer::Composer>,
}

/// How a quit ended.
enum Quitting {
    /// The shell holds the terminal again.
    Quit,
    /// The agent still runs: why, and how many presses it was sent.
    Unanswered { reason: String, presses: usize },
}

impl Quit {
    fn of(kind: &str) -> Option<Self> {
        let adapter = hide_agent_adapter::adapter(kind)?;
        let dialect = adapter.sleep?;
        Some(Self {
            key: dialect.quit_key()?,
            again: dialect.quit_again(),
            composer: adapter.composer.supported(),
        })
    }

    /// Presses the quit key once, and again only on what the agent shows
    /// after a press: the hint that it waits for the key a second time, or a
    /// composer the press emptied of a draft. A press sent to an agent that
    /// had already begun to quit reaches its shell instead, encoded in the
    /// keyboard protocol the agent turned on: Codex CLI 0.165.0 quits on the
    /// first press, and the second one, sent at a fixed 150 ms, left
    /// `ESC[99;5u` in zsh's line as `9;5u` in three runs of four
    /// (with-hide/hide-private#37). An agent that shows neither, a quit in
    /// progress included, is given until `EXIT_GRACE` to hand the terminal
    /// back. A read or a press the engine refuses is an unanswered quit, which the
    /// node's end follows.
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    fn run(&self, engine: &dyn Engine, pane_id: &str, shell: u32) -> Result<Quitting, EndError> {
        let deadline = Instant::now() + EXIT_GRACE;
        let mut presses = 0;
        let unanswered = |reason: String, presses: usize| Quitting::Unanswered { reason, presses };
        let mut before = match screen(engine, pane_id) {
            Ok(screen) => screen,
            Err(reason) => return Ok(unanswered(reason, presses)),
        };
        loop {
            if presses == QUIT_PRESSES {
                return Ok(
                    if shell_takes_terminal_within(
                        engine,
                        pane_id,
                        shell,
                        deadline.saturating_duration_since(Instant::now()),
                    )? {
                        Quitting::Quit
                    } else {
                        unanswered(
                            format!("still running after {presses} presses of its quit key"),
                            presses,
                        )
                    },
                );
            }
            match engine.send_sleep_keys(Wait::Each(REQUEST_TIMEOUT), pane_id, &[self.key]) {
                Ok(()) => {}
                Err(EngineError::Request(message)) => return Err(EndError::Failed(message)),
                Err(reason) => {
                    return Ok(unanswered(
                        format!("pane.send_sleep_keys failed: {reason}"),
                        presses,
                    ));
                }
            }
            presses += 1;
            loop {
                thread::sleep(QUIT_POLL_INTERVAL);
                if process_group(engine, pane_id)
                    .map_err(EndError::Failed)?
                    .foreground_process_group_id
                    == Some(shell)
                {
                    return Ok(Quitting::Quit);
                }
                let now = match screen(engine, pane_id) {
                    Ok(screen) => screen,
                    Err(reason) => return Ok(unanswered(reason, presses)),
                };
                if self.asks_again(&before, &now) {
                    before = now;
                    break;
                }
                if Instant::now() >= deadline {
                    return Ok(unanswered(
                        format!(
                            "still running {} s after its quit key",
                            EXIT_GRACE.as_secs()
                        ),
                        presses,
                    ));
                }
            }
        }
    }

    /// Whether the screen after a press says the agent waits for another:
    /// its hint shows where it did not before the press, or its composer
    /// held a draft before the press and reads empty after it.
    fn asks_again(&self, before: &str, now: &str) -> bool {
        use hide_agent_adapter::composer::{Verdict, screen_shows};
        let hinted = self
            .again
            .is_some_and(|hint| screen_shows(now, hint) && !screen_shows(before, hint));
        let cleared = self.composer.is_some_and(|composer| {
            composer.judge(before) == Verdict::Holds && composer.judge(now) == Verdict::Empty
        });
        hinted || cleared
    }
}

/// The pane's visible screen with its styling, as a composer is judged.
fn screen(engine: &dyn Engine, pane_id: &str) -> Result<String, String> {
    engine
        .read_screen(Wait::Each(REQUEST_TIMEOUT), pane_id)
        .map_err(failed("pane.read"))
        .map(|screen| screen.text)
}

/// Whether the pane's shell holds the terminal again within `within`.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn shell_takes_terminal_within(
    engine: &dyn Engine,
    pane_id: &str,
    shell: u32,
    within: Duration,
) -> Result<bool, EndError> {
    let deadline = Instant::now() + within;
    loop {
        thread::sleep(END_POLL_INTERVAL);
        let group = process_group(engine, pane_id).map_err(EndError::Failed)?;
        if group.foreground_process_group_id == Some(shell) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
    }
}

fn process_group(engine: &dyn Engine, pane_id: &str) -> Result<PaneProcessGroup, String> {
    engine
        .pane_processes(Wait::Each(REQUEST_TIMEOUT), pane_id)
        .map_err(failed("pane.process_info"))
}

/// A resumed agent whose CLI runs no session hook on resume leaves the engine with
/// no session for the woken pane, so the wake could never be told from
/// another conversation. The core chose the id the pane resumed, and says so
/// the way the agent's own integration would have. Nothing is sent for an
/// agent that reports it itself.
pub(crate) fn report_resumed_session(
    engine: &dyn Engine,
    pane: &str,
    kind: &str,
    session_id: &str,
) -> Result<(), EngineError> {
    let reports = hide_agent_adapter::adapter(kind)
        .and_then(|adapter| adapter.resume)
        .is_none_or(hide_agent_adapter::LaunchDialect::resume_reports_session);
    if reports {
        return Ok(());
    }
    let seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        });
    engine
        .report_agent_session(Wait::Each(REQUEST_TIMEOUT), pane, kind, session_id, seq)
        .map_err(|error| match error {
            EngineError::Request(message) => EngineError::Refused {
                code: "session_report_unavailable".into(),
                message,
            },
            error => error,
        })
}

/// What a wake needs, gathered under the lock before the worker starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WakeRequest {
    pub(crate) pane_id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) mode: WakeMode,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: Option<String>,
    /// The conversation a resume opens, which the core reports to the engine for
    /// an agent that does not report it itself on resume.
    pub(crate) session_id: Option<String>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
}

/// Why a wake failed, as the codes the screen words (`web/src/startReason.ts`);
/// the folder, the engine's answer and the rest stay in the wake's `detail`.
pub(crate) const WAKE_FOLDER_MISSING: &str = "agent_wake.folder_missing";
pub(crate) const WAKE_NOT_RESUMED: &str = "agent_wake.not_resumed";
pub(crate) const WAKE_NOT_STARTED: &str = "agent_wake.not_started";
pub(crate) const WAKE_NOT_READY: &str = "agent_wake.not_ready";
pub(crate) const WAKE_LAUNCH_CHOICE: &str = "agent_wake.launch_choice_refused";

/// How a wake ended. `reason` is the code the pane words (B14); `detail` is
/// what only the diagnostic carries, the engine's own code and message among it
/// (B19).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WakeOutcome {
    Started,
    Failed { reason: String, detail: String },
}

/// Starts the agent again in the same pane, through the engine's `agent.start`
/// once the pane's shell holds its terminal (`agent_start`); the engine answers
/// when it has taken the start.
pub(crate) fn start_agent(
    engine: &dyn Engine,
    node: &dyn NodeLink,
    request: &WakeRequest,
) -> WakeOutcome {
    if let Some(cwd) = request.cwd.as_deref()
        && !crate::node_access::is_directory(node, cwd)
    {
        return WakeOutcome::Failed {
            reason: WAKE_FOLDER_MISSING.to_owned(),
            detail: format!("{cwd} is not a directory"),
        };
    }
    let start = match engine.prepare_start(AgentLaunch {
        pane_id: request.pane_id.clone(),
        name: request.name.clone(),
        kind: request.kind.clone(),
        args: request.args.clone(),
        codex_daemon: request.codex_daemon,
        wakes: true,
        program: crate::start_prompt::start_program_of(engine, node, &request.kind),
    }) {
        Ok(start) => start,
        // A Codex whose daemon Hide has not read answers its own code
        // (`codex_launch::CAPABILITY_UNKNOWN`), which the screen words.
        Err(detail) => {
            return WakeOutcome::Failed {
                reason: detail.clone(),
                detail,
            };
        }
    };
    let correlation = format!("hide-core:agent-sleep:{}:wake", request.pane_id);
    // An agent asleep in its pane left only its shell there, so the pane
    // wakes on this build's holder rather than the one an earlier build
    // started (with-hide/hide-private#646). Every answer is a holder that
    // runs the pane, and the wake goes on in it; a refusal is a pane no
    // holder may be typed into, or one whose holder nobody knows (its new
    // holder did not start, its earlier one may still be ending, its
    // machine's answer was lost), and nothing is typed. A renewed pane's
    // shell has just started, so the start waits for its first prompt, as a
    // start into a new tab does.
    let shell = match engine.renew_holder(
        Wait::Each(RENEW_TIMEOUT),
        &request.pane_id,
        request.cwd.as_deref(),
    ) {
        Ok(HolderRenewal::Renewed { .. }) => crate::agent_start::ShellStart::of_tab(
            true,
            crate::agent_start::shell_launch(Some(node), &correlation).as_ref(),
        ),
        Ok(HolderRenewal::Current | HolderRenewal::Kept { .. }) => {
            crate::agent_start::ShellStart::Plain
        }
        Err(error) => {
            return WakeOutcome::Failed {
                reason: refused_reason(request.mode).to_owned(),
                detail: error.to_string(),
            };
        }
    };
    match crate::agent_start::start_at_shell_reusing_name(
        engine,
        Some(node),
        &correlation,
        shell,
        &start,
        crate::agent_start::START_ANSWER_WAIT,
        &|| Ok(()),
    ) {
        Ok(_) => {
            if request.mode == WakeMode::Resume
                && let Some(session) = request.session_id.as_deref()
                && let Err(error) =
                    report_resumed_session(engine, &request.pane_id, &request.kind, session)
            {
                // The agent runs its conversation; the engine learns it at the
                // next prompt instead, as it would have without Hide.
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_sleep",
                    "kind": "agent_sleep.session_report_failed",
                    "pane_id": request.pane_id,
                    "provider": request.kind,
                    "message": error.to_string(),
                }));
            }
            WakeOutcome::Started
        }
        Err(StartError::Engine(EngineError::Answer(detail))) => WakeOutcome::Failed {
            reason: WAKE_NOT_READY.to_owned(),
            detail,
        },
        Err(StartError::NotStarted { detail, .. }) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail,
        },
        Err(StartError::ShellStarting { waited, screen }) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail: crate::agent_start::shell_starting_message(waited, screen.as_deref()),
        },
        Err(StartError::Engine(EngineError::Refused { code, message })) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail: format!("{code}: {message}"),
        },
        Err(StartError::Engine(error)) => WakeOutcome::Failed {
            reason: WAKE_NOT_READY.to_owned(),
            detail: error.to_string(),
        },
    }
}

fn refused_reason(mode: WakeMode) -> &'static str {
    match mode {
        WakeMode::Resume => WAKE_NOT_RESUMED,
        WakeMode::Fresh => WAKE_NOT_STARTED,
    }
}

/// The machine a sleep or a wake runs on, gathered under the lock: the core's
/// own engine and node, or those of a node that dials the core (PRD
/// core-host-node-remote-core B13).
pub(crate) struct SleepTarget {
    pub(crate) engine: SharedEngine,
    pub(crate) node: Arc<dyn NodeLink>,
    pub(crate) runtime: Weak<Mutex<Runtime>>,
    pub(crate) notifier: ChangeNotifier,
    /// The pane as the machine's engine names it; a node's pane is kept in
    /// the core under its `remote:` scope.
    pub(crate) engine_pane_id: String,
    /// Closed to the pane before the end began; the end waits for the writes
    /// that were admitted before that.
    pub(crate) input_gate: Arc<crate::input_gate::PaneInputGate>,
}

pub(crate) fn spawn_end(context: SleepTarget, pane_id: String, kind: String) -> Result<(), String> {
    spawn_end_within(context, pane_id, kind, WRITE_DRAIN_LIMIT)
}

/// Waits for the writes admitted before the pane closed, and ends the agent
/// only once they have landed. A write that outlives `drain_limit` is not
/// waited out: the agent stays awake (nothing is signalled), so that write
/// lands in the agent, and the sleep fails like any other the agent refused.
fn end_after_drain(
    context: &SleepTarget,
    pane_id: &str,
    kind: &str,
    drain_limit: Duration,
) -> Result<u64, String> {
    if context.input_gate.wait_drained(pane_id, drain_limit) == Drain::Stalled {
        crate::diagnostic!(serde_json::json!({
            "component": "agent_sleep",
            "kind": "agent_sleep.drain_stalled",
            "pane_id": pane_id,
            "limit_ms": u64::try_from(drain_limit.as_millis()).unwrap_or(u64::MAX),
        }));
        return Err("a write to the pane was still in flight".to_owned());
    }
    end_agent(
        context.engine.as_ref(),
        context.node.as_ref(),
        &context.engine_pane_id,
        kind,
    )
    .map_err(|error| error.to_string())
}

fn spawn_end_within(
    context: SleepTarget,
    pane_id: String,
    kind: String,
    drain_limit: Duration,
) -> Result<(), String> {
    thread::Builder::new()
        .name("hide-core-agent-sleep".into())
        .spawn(move || {
            let result = end_after_drain(&context, &pane_id, &kind, drain_limit);
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_agent_sleep_end(&pane_id, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("agent sleep worker could not be started: {error}"))
}

pub(crate) fn spawn_wake(context: SleepTarget, request: WakeRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("hide-core-agent-wake".into())
        .spawn(move || {
            let outcome = start_agent(
                context.engine.as_ref(),
                context.node.as_ref(),
                &WakeRequest {
                    pane_id: context.engine_pane_id.clone(),
                    ..request.clone()
                },
            );
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_agent_wake(&request.pane_id, request.mode, outcome),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("agent wake worker could not be started: {error}"))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::{Value, json};

    use super::*;
    use crate::engine::fake::{FakeEngine, Reply, delivery_agent, processes, refused, reply};
    use hide_platform::process;

    type Fake = Arc<FakeEngine<'static>>;

    /// The pane's Claude agent, as the check before its end reads it.
    fn agent_info(status: &str, seq: u64) -> Reply {
        agent_of("claude", status, seq)
    }

    fn agent_of(kind: &str, status: &str, seq: u64) -> Reply {
        reply(crate::engine::fake::agent_state(&json!({
            "agent": kind, "agent_status": status, "state_change_seq": seq
        })))
    }

    fn process_info(shell: u32, foreground: u32) -> Reply {
        reply(processes(&json!({
            "shell_pid": shell, "foreground_process_group_id": foreground,
            "foreground_processes": []
        })))
    }

    fn started() -> Reply {
        reply(crate::engine::StartedAgent {
            pane_id: "w1:p1".into(),
            terminal_id: Some("term_1".into()),
        })
    }

    #[test]
    fn wake_sends_canonical_kinds_and_preserves_resume_arguments_for_aliases() {
        for (kind, canonical, args) in [
            ("claude-code", "claude", vec!["--resume", "session"]),
            (" CLAUDE ", "claude", vec!["--resume", "session"]),
            (" CLAUDE_CODE ", "claude", vec!["--resume", "session"]),
            (" CODEX ", "codex", vec!["resume", "session"]),
        ] {
            let expected = args.clone();
            let fake = FakeEngine::start(move |method, params| match method {
                "renew_holder" => reply(HolderRenewal::Current),
                "pane_processes" => process_info(42, 42),
                "start_agent" => {
                    assert_eq!(params["kind"], canonical);
                    assert_eq!(params["args"], json!(expected));
                    assert_eq!(params["pane"], "w1:p1");
                    started()
                }
                other => panic!("unexpected {other}"),
            });
            assert_eq!(
                start_agent(
                    fake.engine().as_ref(),
                    &hide_node::Local::of_process(),
                    &WakeRequest {
                        pane_id: "w1:p1".into(),
                        name: "wake".into(),
                        kind: kind.into(),
                        mode: WakeMode::Resume,
                        args: args.into_iter().map(str::to_owned).collect(),
                        cwd: None,
                        session_id: None,
                        codex_daemon: crate::codex_launch::CodexDaemon::Unsupported,
                    }
                ),
                WakeOutcome::Started,
                "{kind}"
            );
            assert_eq!(
                fake.methods(),
                ["renew_holder", "pane_processes", "start_agent"]
            );
        }
    }

    #[test]
    fn sleep_end_accepts_known_aliases_but_keeps_process_group_and_agent_guards() {
        for kind in ["claude-code", " CLAUDE ", " CLAUDE_CODE "] {
            let fake = FakeEngine::start(|method, _| match method {
                "agent_state" => agent_info("idle", 3),
                "pane_processes" => process_info(42, 42),
                other => panic!("unexpected {other}"),
            });
            let error = end_agent(
                fake.engine().as_ref(),
                &hide_node::Local::of_process(),
                "w1:p1",
                kind,
            )
            .expect_err("a canonical identity still cannot signal the shell")
            .to_string();
            assert!(error.contains("shell already holds"), "{kind}: {error}");
            assert_eq!(fake.methods(), ["agent_state", "pane_processes"]);
        }
        for kind in ["codex", "future-agent"] {
            let fake = FakeEngine::start(|method, _| match method {
                "agent_state" => agent_info("idle", 3),
                other => panic!("identity refusal must not query or signal a group: {other}"),
            });
            assert!(
                end_agent(
                    fake.engine().as_ref(),
                    &hide_node::Local::of_process(),
                    "w1:p1",
                    kind
                )
                .expect_err("another agent cannot be ended")
                .to_string()
                .contains("rather than")
            );
            assert_eq!(fake.methods(), ["agent_state"]);
        }
    }

    fn sleep_target(fake: &Fake, gate: &Arc<crate::input_gate::PaneInputGate>) -> SleepTarget {
        SleepTarget {
            engine: fake.engine(),
            node: Arc::new(hide_node::Local::of_process()),
            runtime: Weak::new(),
            notifier: ChangeNotifier::new(),
            engine_pane_id: "w1:p1".into(),
            input_gate: Arc::clone(gate),
        }
    }

    /// A reply a phone admitted just before the pane closed is on its way to
    /// the agent. The worker must not touch the agent until it has landed, or
    /// the reply arrives behind the end and runs in the shell.
    #[test]
    fn the_end_waits_for_a_write_admitted_before_the_pane_closed() {
        let released = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&released);
        let fake = FakeEngine::start(move |method, _| match method {
            "agent_state" => {
                assert!(
                    seen.load(Ordering::SeqCst),
                    "the end began while a write was still landing"
                );
                // Busy again, so nothing is signalled: the order is the subject.
                agent_info("working", 3)
            }
            other => panic!("unexpected {other}"),
        });
        let gate = Arc::new(crate::input_gate::PaneInputGate::default());
        let admission = gate.admit("w1:p1").expect("awake");
        gate.close("w1:p1");
        spawn_end(sleep_target(&fake, &gate), "w1:p1".into(), "claude".into())
            .expect("the worker starts");
        // The write is released once the worker is at its wait, not at its spawn.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !gate.is_waiting("w1:p1") {
            assert!(Instant::now() < deadline, "the worker never waited");
            thread::yield_now();
        }
        assert!(gate.admit("w1:p1").is_err(), "a later write is refused");
        released.store(true, Ordering::SeqCst);
        drop(admission);
        fake.wait_for_calls(1, Duration::from_secs(30));
    }

    /// A write stalled past the drain bound is not waited out and not ended
    /// under: nothing reaches the engine, so the agent stays awake and the late
    /// write lands in it; the sleep fails with a reason the runtime takes as
    /// any refused end (it reopens the pane and backs off).
    #[test]
    fn a_write_stalled_past_the_drain_bound_leaves_the_agent_awake() {
        let fake = FakeEngine::start(|method, _| {
            panic!("a stalled write must not let the end begin: {method}")
        });
        let gate = Arc::new(crate::input_gate::PaneInputGate::default());
        let admission = gate.admit("w1:p1").expect("awake");
        gate.close("w1:p1");
        // The write is held until the worker's answer is in, so it cannot
        // finish inside the bound by luck.
        let error = end_after_drain(
            &sleep_target(&fake, &gate),
            "w1:p1",
            "claude",
            Duration::from_millis(50),
        )
        .expect_err("the drain did not finish");
        assert!(error.contains("still in flight"), "{error}");
        assert!(fake.methods().is_empty(), "the agent was not touched");
        drop(admission);
    }

    const AGENT_TREE: &str = "HIDE_TEST_AGENT_TREE";

    fn agent_tree_command(role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("the test binary"));
        command
            .args([
                "--exact",
                "agent_sleep_engine::tests::agent_tree_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(AGENT_TREE, role)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Plays the agent of the test below when `AGENT_TREE` is set: `agent`
    /// starts a `tool`, as an agent starts an MCP server, says its pid and
    /// waits on it; `tool` waits to be ended.
    #[test]
    #[allow(clippy::disallowed_methods)] // a stand-in agent: it sleeps to stay alive until it is ended
    fn agent_tree_child() {
        let Some(role) = std::env::var_os(AGENT_TREE) else {
            return;
        };
        if role == "agent" {
            let mut tool = agent_tree_command("tool")
                .stdout(Stdio::null())
                .spawn()
                .expect("the tool starts");
            println!("READY {}", tool.id());
            std::io::stdout().flush().expect("the pid is said");
            let _ = tool.wait();
        } else {
            thread::sleep(Duration::from_secs(60));
        }
    }

    /// A node that answers nothing: an end that reaches it fails the test.
    struct NoNode;

    impl NodeLink for NoNode {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            panic!("the node was asked {call:?}");
        }
    }

    /// The pane's visible screen, rows `text`.
    fn screen(text: &str) -> Reply {
        reply(crate::engine::PaneText {
            text: text.to_owned(),
            truncated: false,
        })
    }

    /// A composer as Codex CLI draws it: its marker and what follows, dim
    /// when it is the placeholder.
    const CODEX_EMPTY: &str = "\u{203a} \u{1b}[2mAsk Codex to do anything\u{1b}[0m\n";
    const CODEX_DRAFT: &str = "\u{203a} half a thought\n";
    const CODEX_QUITTING: &str = "\u{203a} Shutting down...\n";

    /// Ends the stand-in agent of `kind` that answers each press of its quit
    /// key by `press` (the presses before it, the agent's pid), shows the
    /// screen `shown` returns for the presses so far and answers each read
    /// of it by `read` (the same count, the pid); returns the keys the pane
    /// was sent. The node is never asked to end anything.
    fn quit_by_keys(
        kind: &'static str,
        shown: fn(usize) -> &'static str,
        press: fn(usize, u32),
        read: fn(usize, u32),
    ) -> Vec<Value> {
        let mut agent =
            process::OwnedChild::spawn(&mut agent_tree_command("tool")).expect("the agent starts");
        let pid = agent.id();
        let presses = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pressed = Arc::clone(&presses);
        let fake = FakeEngine::start(move |method, _| match method {
            "agent_state" => agent_of(kind, "idle", 7),
            "pane_processes" => {
                let shell = std::process::id();
                process_info(shell, if process::is_alive(pid) { pid } else { shell })
            }
            "read_screen" => {
                let presses = pressed.load(Ordering::SeqCst);
                read(presses, pid);
                screen(shown(presses))
            }
            "send_keys" => {
                press(pressed.fetch_add(1, Ordering::SeqCst), pid);
                reply(())
            }
            other => panic!("unexpected {other}"),
        });
        assert_eq!(
            end_agent(fake.engine().as_ref(), &NoNode, "w1:p1", kind),
            Ok(7)
        );
        agent.wait().expect("the agent is reaped");
        fake.calls()
            .into_iter()
            .filter(|(method, _)| *method == "send_keys")
            .map(|(_, params)| params["keys"].clone())
            .collect()
    }

    /// An agent that quits on its own key, as Claude Code does on a second
    /// Ctrl+C, puts its terminal back itself: the second press follows the
    /// hint it shows after the first, and the node is never asked to end
    /// anything.
    #[test]
    fn an_agent_that_asks_for_its_quit_key_again_gets_a_second_press() {
        let keys = quit_by_keys(
            "claude",
            |presses| match presses {
                0 => "\u{276f} \n",
                _ => "\u{276f} \nPress Ctrl-C again to exit\n",
            },
            |before, pid| {
                if before == 1 {
                    process::terminate(pid).expect("the agent quits");
                }
            },
            |_, _| {},
        );
        assert_eq!(keys, [json!(["ctrl+c"]), json!(["ctrl+c"])]);
    }

    /// Codex CLI 0.165.0 quits on the first Ctrl+C into an empty composer
    /// and takes a moment to hand the terminal back: a second press then
    /// reaches the shell, in the keyboard protocol Codex had turned on, and
    /// the wake typed behind it (with-hide/hide-private#37). The first press
    /// is the only one: the agent ends only after the sleep has read it
    /// quitting, so the sleep saw that state and did not press again.
    #[test]
    fn an_agent_quitting_on_its_first_press_is_sent_no_second_one() {
        let keys = quit_by_keys(
            "codex",
            |presses| {
                if presses == 0 {
                    CODEX_EMPTY
                } else {
                    CODEX_QUITTING
                }
            },
            |_, _| {},
            |presses, pid| {
                if presses == 1 {
                    let _ = process::terminate(pid);
                }
            },
        );
        assert_eq!(keys, [json!(["ctrl+c"])]);
    }

    /// A press into a composer that holds a draft clears it instead of
    /// quitting; the composer reads empty after it, and the next press quits.
    #[test]
    fn a_press_that_clears_a_draft_is_followed_by_the_quit() {
        let keys = quit_by_keys(
            "codex",
            |presses| match presses {
                0 => CODEX_DRAFT,
                1 => CODEX_EMPTY,
                _ => CODEX_QUITTING,
            },
            |before, pid| {
                if before == 1 {
                    process::terminate(pid).expect("the agent quits");
                }
            },
            |_, _| {},
        );
        assert_eq!(keys, [json!(["ctrl+c"]), json!(["ctrl+c"])]);
    }

    /// Sleep ends the pane's foreground job on every system when the agent
    /// does not quit on its keys: the engine names the agent's process as the
    /// pane's foreground group (on Windows, which keeps no group, the job's
    /// first process stands in for it), the agent and the tool it started
    /// end, and the end waits for the shell to hold the terminal again. On
    /// Windows the node refused the end as unsupported, so every Claude Code,
    /// Codex and Antigravity agent stayed awake.
    #[test]
    #[allow(clippy::disallowed_methods)] // a polling wait for the tool's end, bounded by a deadline
    fn an_agent_that_does_not_quit_on_its_keys_is_ended_with_what_it_started() {
        // Owned, so a failing test still ends the tree; on Unix it leads its
        // own group, as a terminal's foreground job does.
        let mut agent =
            process::OwnedChild::spawn(agent_tree_command("agent").stdout(Stdio::piped()))
                .expect("the agent starts");
        let said = std::io::BufRead::lines(std::io::BufReader::new(
            agent.take_stdout().expect("the agent's output"),
        ))
        .map_while(Result::ok)
        .find_map(|line| line.split_once("READY ")?.1.trim().parse::<u32>().ok());
        let tool = said.expect("the agent said its tool's pid");
        let pid = agent.id();
        let fake = FakeEngine::start(move |method, _| match method {
            "agent_state" => agent_info("idle", 7),
            "pane_processes" => {
                // This test process stands in for the shell, which takes the
                // terminal back once the agent has ended.
                let shell = std::process::id();
                process_info(shell, if process::is_alive(pid) { pid } else { shell })
            }
            // The stand-in reads no keys and shows nothing that asks for
            // another, as an agent in a dialog does not quit.
            "send_keys" => reply(()),
            "read_screen" => screen("A dialog the quit key does not close\n"),
            other => panic!("unexpected {other}"),
        });
        assert_eq!(
            end_agent(
                fake.engine().as_ref(),
                &hide_node::Local::of_process(),
                "w1:p1",
                "claude"
            ),
            Ok(7)
        );
        assert!(!agent.wait().expect("the agent is reaped").success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while process::is_alive(tool) {
            assert!(Instant::now() < deadline, "the agent's tool kept running");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// A node that answers each end with what it is given, and says it was
    /// asked.
    struct EndingNode(Value, Arc<AtomicBool>);

    impl NodeLink for EndingNode {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            assert!(
                matches!(call, Call::TerminateGroup { leader: 9 }),
                "{call:?}"
            );
            self.1.store(true, Ordering::SeqCst);
            Ok(crate::node_access::LinkAnswer::Parsed(self.0.clone()))
        }
    }

    /// The agent may quit between the last look at its pane and the node's
    /// end, or leave a process the node could not end once the agent itself
    /// is down. Neither is a failed sleep: the shell holds the terminal, so
    /// the agent sleeps.
    #[test]
    fn an_end_that_finds_the_agent_gone_or_leaves_a_process_still_sleeps_it() {
        for answer in [
            json!({"outcome": "gone"}),
            json!({"outcome": "ended", "left": [{"pid": 11, "reason": "Access is denied."}]}),
        ] {
            let asked = Arc::new(AtomicBool::new(false));
            let node = EndingNode(answer.clone(), Arc::clone(&asked));
            let fake = FakeEngine::start(move |method, _| {
                match method {
                    "agent_state" => agent_info("idle", 7),
                    "pane_processes" if asked.load(Ordering::SeqCst) => process_info(42, 42),
                    "pane_processes" => process_info(42, 9),
                    // Refused keys go to the node at once, so no grace is
                    // waited out here.
                    "read_screen" => screen("\u{276f} \n"),
                    "send_keys" => refused("pane_send_failed", "refused"),
                    other => panic!("unexpected {other}"),
                }
            });
            assert_eq!(
                end_agent(fake.engine().as_ref(), &node, "w1:p1", "claude"),
                Ok(7),
                "{answer}"
            );
        }
    }

    #[test]
    fn a_pane_whose_shell_holds_the_terminal_is_left_alone() {
        let fake = FakeEngine::start(|method, _| match method {
            "agent_state" => agent_info("idle", 3),
            "pane_processes" => process_info(42, 42),
            other => panic!("unexpected {other}"),
        });
        let error = end_agent(
            fake.engine().as_ref(),
            &hide_node::Local::of_process(),
            "w1:p1",
            "claude",
        )
        .expect_err("nothing to end")
        .to_string();
        assert!(error.contains("shell already holds"), "{error}");
        assert_eq!(fake.methods(), ["agent_state", "pane_processes"]);
    }

    #[test]
    fn a_group_that_would_signal_every_process_is_refused() {
        for (shell, foreground) in [(42, 0), (42, 1), (0, 42), (1, 42)] {
            let fake = FakeEngine::start(move |method, _| match method {
                "agent_state" => agent_info("idle", 3),
                "pane_processes" => process_info(shell, foreground),
                other => panic!("unexpected {other}"),
            });
            let error = end_agent(
                fake.engine().as_ref(),
                &hide_node::Local::of_process(),
                "w1:p1",
                "claude",
            )
            .expect_err("refused")
            .to_string();
            assert!(error.contains("not a pane's"), "{error}");
            assert_eq!(fake.methods(), ["agent_state", "pane_processes"]);
        }
    }

    #[test]
    fn an_agent_that_started_working_is_not_ended() {
        let fake = FakeEngine::start(|method, _| match method {
            "agent_state" => agent_info("working", 3),
            other => panic!("unexpected {other}"),
        });
        let error = end_agent(
            fake.engine().as_ref(),
            &hide_node::Local::of_process(),
            "w1:p1",
            "claude",
        )
        .expect_err("working");
        assert!(matches!(error, EndError::Busy(ref message) if message.contains("working")));
        assert_eq!(fake.methods(), ["agent_state"]);
    }

    #[test]
    fn an_unknown_codex_wake_keeps_the_next_action_and_sends_no_start() {
        let fake = FakeEngine::start(|method, _| panic!("unexpected {method}"));
        let outcome = start_agent(
            fake.engine().as_ref(),
            &hide_node::Local::of_process(),
            &WakeRequest {
                codex_daemon: crate::codex_launch::CodexDaemon::Unknown,
                pane_id: "w1:p1".into(),
                kind: "codex".into(),
                name: "one".into(),
                mode: WakeMode::Resume,
                args: vec!["resume".into(), "abc".into()],
                cwd: None,
                session_id: None,
            },
        );
        let WakeOutcome::Failed { reason, .. } = outcome else {
            panic!("wake started")
        };
        // The screen words the code (`web/src/startReason.ts`).
        assert_eq!(reason, crate::codex_launch::CAPABILITY_UNKNOWN);
        assert!(fake.methods().is_empty());
    }

    #[test]
    fn a_refused_wake_names_a_plain_reason_and_keeps_the_engine_s_words_for_the_log() {
        let fake = FakeEngine::start(|method, _| match method {
            "renew_holder" => reply(HolderRenewal::Current),
            "pane_processes" => process_info(42, 42),
            "start_agent" => refused("agent_name_taken", "agent name one is already used"),
            // Another agent holds the name, so reading it changes nothing.
            "delivery_agent" => reply(delivery_agent(&json!({
                "pane_id": "w1:p2", "name": "one", "agent": "claude",
                "agent_status": "working"}))),
            other => panic!("unexpected {other}"),
        });
        let outcome = start_agent(
            fake.engine().as_ref(),
            &hide_node::Local::of_process(),
            &WakeRequest {
                codex_daemon: Default::default(),
                pane_id: "w1:p1".into(),
                kind: "claude".into(),
                name: "one".into(),
                mode: WakeMode::Resume,
                args: vec!["--resume".into(), "abc".into()],
                cwd: None,
                session_id: None,
            },
        );
        assert_eq!(
            outcome,
            WakeOutcome::Failed {
                reason: WAKE_NOT_RESUMED.into(),
                detail: "agent_name_taken: agent name one is already used".into(),
            }
        );
    }

    /// A Wake names the file the pane's node says its start runs, as a
    /// start does: on Windows PowerShell would find npm's `.ps1` by the
    /// name, and Windows PowerShell 5.1 hands Claude Code's
    /// `--settings '{"...":true}'` on without its quotes. A node that starts
    /// the program by its name leaves the Wake by its name.
    #[test]
    fn a_wake_names_the_file_its_node_starts_the_program_from() {
        struct Starts(Value);
        impl NodeLink for Starts {
            fn call(
                &self,
                call: Call,
                _: Duration,
            ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
                match call {
                    Call::AgentInstalled { name } if name == "claude" => {
                        Ok(crate::node_access::LinkAnswer::Parsed(self.0.clone()))
                    }
                    other => panic!("the node was asked {other:?}"),
                }
            }
        }
        let exe = "C:\\npm\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe";
        for (form, named) in [
            (
                json!({"direct": {"program": exe, "leading": []}}),
                json!([exe, []]),
            ),
            (json!("program"), Value::Null),
        ] {
            let fake = FakeEngine::start(|method, _| match method {
                "renew_holder" => reply(HolderRenewal::Current),
                "pane_processes" => process_info(42, 42),
                "start_agent" => started(),
                other => panic!("unexpected {other}"),
            });
            let outcome = start_agent(
                fake.engine().as_ref(),
                &Starts(form),
                &WakeRequest {
                    codex_daemon: Default::default(),
                    pane_id: "w1:p1".into(),
                    kind: "claude".into(),
                    name: "one".into(),
                    mode: WakeMode::Resume,
                    args: vec!["--resume".into(), "abc".into()],
                    cwd: None,
                    session_id: None,
                },
            );
            assert_eq!(outcome, WakeOutcome::Started);
            let starts = fake.calls_of("start_agent");
            assert_eq!(starts.len(), 1);
            assert_eq!(starts[0]["program"], named);
        }
    }

    /// The node a renewal test's wake asks: the agent starts by its name,
    /// and the pane's shell marks no prompt, so a renewed pane's start waits
    /// for its shell to hold the terminal.
    struct PlainNode;
    impl NodeLink for PlainNode {
        fn call(
            &self,
            call: Call,
            _: Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            match call {
                Call::AgentInstalled { .. } => {
                    Ok(crate::node_access::LinkAnswer::Parsed(json!("program")))
                }
                Call::ShellIntegration => Ok(crate::node_access::LinkAnswer::Parsed(Value::Null)),
                Call::Directory { path } => Ok(crate::node_access::LinkAnswer::Parsed(json!(path))),
                other => panic!("the node was asked {other:?}"),
            }
        }
    }

    fn resume_in(cwd: &str) -> WakeRequest {
        WakeRequest {
            codex_daemon: Default::default(),
            pane_id: "w1:p1".into(),
            kind: "claude".into(),
            name: "one".into(),
            mode: WakeMode::Resume,
            args: vec!["--resume".into(), "abc".into()],
            cwd: Some(cwd.to_owned()),
            session_id: None,
        }
    }

    /// with-hide/hide-private#646: a wake brings its pane's holder to this
    /// build, in the conversation's folder, before it types the resume, and
    /// a pane its node kept on the holder it has (something runs beside the
    /// shell, a node that cannot renew) wakes there, as before. Each wake
    /// asks again, so a pane kept once is renewed by a later wake.
    #[test]
    fn a_wake_renews_its_pane_s_holder_before_it_types_the_resume() {
        let cwd = "/work/agent";
        for renewal in [
            reply(HolderRenewal::Renewed {
                from: "v1-earlier".into(),
                to: "v1-current".into(),
            }),
            reply(HolderRenewal::Current),
            reply(HolderRenewal::Kept {
                code: "pane_busy".into(),
                message: "the shell runs a program".into(),
            }),
            reply(HolderRenewal::Kept {
                code: "holder_renewal_unsupported".into(),
                message: "a node that cannot renew".into(),
            }),
        ] {
            let renewal = std::sync::Mutex::new(Some(renewal));
            let fake = FakeEngine::start(move |method, _| match method {
                "renew_holder" => renewal.lock().unwrap().take().expect("one renewal"),
                "pane_processes" => process_info(42, 42),
                "start_agent" => started(),
                other => panic!("unexpected {other}"),
            });
            let outcome = start_agent(fake.engine().as_ref(), &PlainNode, &resume_in(cwd));
            assert_eq!(outcome, WakeOutcome::Started);
            assert_eq!(
                fake.methods(),
                ["renew_holder", "pane_processes", "start_agent"]
            );
            let renewals = fake.calls_of("renew_holder");
            assert_eq!(renewals[0]["pane"], "w1:p1");
            assert_eq!(renewals[0]["cwd"], json!(cwd));
        }
    }

    /// A renewal after which no holder is known to run the pane leaves no
    /// shell to type into: its new holder did not start, its earlier one
    /// was asked to end and may still be ending, or its node's answer never
    /// came. The wake fails with the engine's words and types nothing.
    #[test]
    fn a_wake_whose_renewal_left_no_known_holder_fails_and_types_nothing() {
        for (error, detail) in [
            (
                EngineError::Refused {
                    code: hide_node_link::holders::RENEWAL_LOST.into(),
                    message: "the holder did not start".into(),
                },
                "holder_renewal_lost: the holder did not start",
            ),
            (
                EngineError::Refused {
                    code: hide_node_link::holders::RENEWAL_UNSETTLED.into(),
                    message: "the close did not finish".into(),
                },
                "holder_renewal_unsettled: the close did not finish",
            ),
            (
                EngineError::Unanswered("the device's answer never came".into()),
                "the device's answer never came",
            ),
        ] {
            let error = std::sync::Mutex::new(Some(error));
            let fake = FakeEngine::start(move |method, _| match method {
                "renew_holder" => Err(error.lock().unwrap().take().expect("one renewal")),
                other => panic!("unexpected {other}"),
            });
            let outcome = start_agent(
                fake.engine().as_ref(),
                &PlainNode,
                &resume_in("/work/agent"),
            );
            assert_eq!(
                outcome,
                WakeOutcome::Failed {
                    reason: WAKE_NOT_RESUMED.into(),
                    detail: detail.into(),
                }
            );
            assert_eq!(fake.methods(), ["renew_holder"]);
        }
    }

    /// Issue 628: the node looks the program up on its `PATH`, so a Wake of an
    /// agent whose kind is not its program (Cursor `cursor`, `cursor-agent`;
    /// Kiro `kiro`, `kiro-cli`) asks about the program, and names the file it
    /// answers with, where the kind would have named a program the node does
    /// not have.
    #[test]
    fn a_wake_asks_its_node_about_the_definitions_first_executable() {
        struct Starts(Mutex<Vec<String>>);
        impl NodeLink for Starts {
            fn call(
                &self,
                call: Call,
                _: Duration,
            ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
                let Call::AgentInstalled { name } = call else {
                    panic!("the node was asked {call:?}");
                };
                let answer = match name.as_str() {
                    "cursor-agent" => {
                        json!({"direct": {"program": "C:\\cursor-agent.exe", "leading": []}})
                    }
                    "kiro-cli" => json!({"direct": {"program": "C:\\kiro-cli.exe", "leading": []}}),
                    // Only what the agents are called by on the node.
                    _ => Value::Null,
                };
                self.0.lock().unwrap().push(name);
                Ok(crate::node_access::LinkAnswer::Parsed(answer))
            }
        }
        for (kind, program, exe) in [
            ("cursor", "cursor-agent", "C:\\cursor-agent.exe"),
            ("kiro", "kiro-cli", "C:\\kiro-cli.exe"),
        ] {
            let fake = FakeEngine::start(|method, _| match method {
                "renew_holder" => reply(HolderRenewal::Current),
                "pane_processes" => process_info(42, 42),
                "start_agent" => started(),
                other => panic!("unexpected {other}"),
            });
            let node = Starts(Mutex::default());
            let outcome = start_agent(
                fake.engine().as_ref(),
                &node,
                &WakeRequest {
                    codex_daemon: Default::default(),
                    pane_id: "w1:p1".into(),
                    kind: kind.into(),
                    name: "one".into(),
                    mode: WakeMode::Resume,
                    args: vec!["--resume".into(), "abc".into()],
                    cwd: None,
                    session_id: None,
                },
            );
            assert_eq!(outcome, WakeOutcome::Started, "{kind}");
            assert_eq!(*node.0.lock().unwrap(), [program], "{kind}");
            let starts = fake.calls_of("start_agent");
            assert_eq!(starts[0]["program"], json!([exe, []]), "{kind}");
        }
    }

    #[test]
    fn a_wake_into_a_folder_that_is_gone_asks_the_engine_nothing() {
        let fake = FakeEngine::start(|method, _| panic!("unexpected {method}"));
        let outcome = start_agent(
            fake.engine().as_ref(),
            &hide_node::Local::of_process(),
            &WakeRequest {
                codex_daemon: Default::default(),
                pane_id: "w1:p1".into(),
                kind: "codex".into(),
                name: "wake-w1-p1".into(),
                mode: WakeMode::Resume,
                args: Vec::new(),
                cwd: Some("/nonexistent/agent-sleep-test".into()),
                session_id: None,
            },
        );
        assert!(matches!(
            outcome,
            WakeOutcome::Failed { ref reason, ref detail } if reason == WAKE_FOLDER_MISSING && detail.contains("/nonexistent/agent-sleep-test")
        ));
        assert!(fake.methods().is_empty());
    }
}
