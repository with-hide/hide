//! The one place agent sleep relies on Herdr and on the operating system
//! (PRD agent-sleep D-20).
//!
//! Herdr has no hibernate: the pinned release ends nothing on request and
//! forgets an agent whose process ended. So sleeping is Hide ending the
//! pane's foreground process group with SIGTERM, which returns the pane to
//! its shell, and waking is Herdr's own `agent.start` in that same pane with
//! the provider's resume arguments; the signal is sent by the pane's node.
//! When Herdr offers a hibernate or an
//! exit mark of its own, this file is what changes; the decision, the
//! records and the screens in `agent_sleep.rs` and the runtime stay.
//!
//! Every call here runs on a worker thread, never under `Mutex<Runtime>`.

use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use hide_herdr_client::{ApiConnector, ApiError, request_with_connector};
use hide_node_link::protocol::Call;

use crate::node_access::{NodeLink, call_as};

use crate::agent_sleep::WakeMode;
use crate::agent_start::StartError;
use crate::handle::ChangeNotifier;
use crate::runtime::Runtime;
use crate::wire;

mod dormant;
pub(crate) use dormant::{
    DormantStartOutcome, DormantStatus, DormantTabOutcome, DormantWork,
    fenced_dormant_close_connector, spawn_dormant_start, spawn_dormant_status, spawn_dormant_tab,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an ended agent has to hand the terminal back to its shell before
/// the sleep is given up and the agent is left awake (B8).
const END_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest the end waits for a write already admitted to the pane; a
/// write has its own request timeout (`REQUEST_TIMEOUT`), so this is the hang guard.
const WRITE_DRAIN_LIMIT: Duration = Duration::from_secs(6);
const END_POLL_INTERVAL: Duration = Duration::from_millis(200);
/// `agent.start` waits this long for the agent to be ready; the read waits a
/// little longer so a slow start is not reported as an unanswered one.
const AGENT_START_TIMEOUT_MS: u64 = 120_000;

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

/// Ends the agent in `pane_id` and returns the state change sequence Herdr
/// reported for it just before, which is how a later listing of the same
/// agent is told from a new one.
///
/// It reads the agent again first, because the decision was made on a list
/// up to a second old and a parent's prompt may have set it working since.
/// The signal goes to the pane's foreground process group only, and only
/// when that group is not the shell's: the agent and what it started in its
/// own group (MCP servers, tool processes) end, the shell and every other
/// pane stay (B7).
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub(crate) fn end_agent(
    connector: &dyn ApiConnector,
    node: &dyn NodeLink,
    pane_id: &str,
    kind: &str,
) -> Result<u64, EndError> {
    let state = request_with_connector(
        connector,
        "agent.get",
        wire::agent_target_params(pane_id).map_err(EndError::Failed)?,
        REQUEST_TIMEOUT,
    )
    .map_err(|error| format!("agent.get failed: {error}"))
    .and_then(wire::agent_state)
    .map_err(EndError::Failed)?;
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
    let group = process_group(connector, pane_id).map_err(EndError::Failed)?;
    let (Some(shell), Some(foreground)) = (group.shell_pid, group.foreground_process_group_id)
    else {
        return Err(EndError::Failed(
            "Herdr reported no shell or foreground process group for the pane".into(),
        ));
    };
    // kill(-0) signals Hide's own group and kill(-1) every process the user
    // owns, so a group id that is not a real group never reaches the signal.
    if foreground <= 1 || shell <= 1 {
        return Err(EndError::Failed(format!(
            "Herdr reported process group {foreground} and shell {shell}, which are not a pane's"
        )));
    }
    if foreground == shell {
        return Err(EndError::Failed(
            "the pane's shell already holds the terminal".into(),
        ));
    }
    // The group Herdr reported as the pane's foreground gets SIGTERM.
    call_as::<()>(
        node,
        Call::TerminateGroup { leader: foreground },
        REQUEST_TIMEOUT,
    )
    .map_err(|error| {
        EndError::Failed(format!("ending process group {foreground} failed: {error}"))
    })?;
    let deadline = Instant::now() + END_TIMEOUT;
    loop {
        thread::sleep(END_POLL_INTERVAL);
        let group = process_group(connector, pane_id).map_err(EndError::Failed)?;
        if group.foreground_process_group_id == Some(shell) {
            return Ok(state.state_change_seq);
        }
        if Instant::now() >= deadline {
            return Err(EndError::Failed(format!(
                "the agent did not hand the terminal back to its shell within {} s",
                END_TIMEOUT.as_secs()
            )));
        }
    }
}

fn process_group(
    connector: &dyn ApiConnector,
    pane_id: &str,
) -> Result<wire::PaneProcessGroup, String> {
    request_with_connector(
        connector,
        "pane.process_info",
        wire::pane_process_info_params(pane_id)?,
        REQUEST_TIMEOUT,
    )
    .map_err(|error| format!("pane.process_info failed: {error}"))
    .and_then(wire::pane_process_group)
}

/// A resumed agent whose CLI runs no session hook on resume leaves Herdr with
/// no session for the woken pane, so the wake could never be told from
/// another conversation. The core chose the id the pane resumed, and says so
/// the way the agent's own integration would have. Nothing is sent for an
/// agent that reports it itself.
pub(crate) fn report_resumed_session(
    connector: &dyn ApiConnector,
    pane: &str,
    kind: &str,
    session_id: &str,
) -> Result<(), ApiError> {
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
    let params =
        wire::report_agent_session_params(pane, kind, session_id, seq).map_err(|message| {
            ApiError::Remote {
                code: "session_report_unavailable".into(),
                message,
            }
        })?;
    request_with_connector(
        connector,
        "pane.report_agent_session",
        params,
        REQUEST_TIMEOUT,
    )
    .map(|_| ())
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
    /// The conversation a resume opens, which the core reports to Herdr for
    /// an agent that does not report it itself on resume.
    pub(crate) session_id: Option<String>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
}

/// How a wake ended. `reason` is what the pane says (B14); `detail` is what
/// only the diagnostic carries, Herdr's own code and message among it (B19).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WakeOutcome {
    Started,
    Failed { reason: String, detail: String },
}

/// Starts the agent again in the same pane, through Herdr's `agent.start`
/// once the pane's shell holds its terminal (`agent_start`); Herdr answers
/// once the agent is ready.
pub(crate) fn start_agent(
    connector: &dyn ApiConnector,
    node: &dyn NodeLink,
    request: &WakeRequest,
) -> WakeOutcome {
    if let Some(cwd) = request.cwd.as_deref()
        && !crate::node_access::is_directory(node, cwd)
    {
        return WakeOutcome::Failed {
            reason: format!("The working folder {cwd} no longer exists."),
            detail: format!("{cwd} is not a directory"),
        };
    }
    let params = match wire::agent_start_params(
        &request.pane_id,
        &request.name,
        &request.kind,
        request.args.clone(),
        request.codex_daemon,
    ) {
        Ok(params) => params,
        Err(detail) => {
            return WakeOutcome::Failed {
                reason: detail.clone(),
                detail,
            };
        }
    };
    match crate::agent_start::start_at_shell_reusing_name(
        connector,
        Some(node),
        &format!("herdr-core:agent-sleep:{}:wake", request.pane_id),
        &request.pane_id,
        crate::agent_start::ShellStart::Plain,
        params,
        Duration::from_millis(AGENT_START_TIMEOUT_MS + 5_000),
        &|| Ok(()),
    ) {
        Ok(value) => match wire::started_agent(value) {
            Ok(_) => {
                if request.mode == WakeMode::Resume
                    && let Some(session) = request.session_id.as_deref()
                    && let Err(error) =
                        report_resumed_session(connector, &request.pane_id, &request.kind, session)
                {
                    // The agent runs its conversation; Herdr learns it at the
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
            Err(detail) => WakeOutcome::Failed {
                reason: "The agent did not become ready in time.".to_owned(),
                detail,
            },
        },
        Err(StartError::NotStarted(detail)) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail,
        },
        Err(StartError::ShellStarting(screen)) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail: crate::agent_start::shell_starting_message(screen.as_deref()),
        },
        Err(StartError::Herdr(ApiError::Remote { code, message })) => WakeOutcome::Failed {
            reason: refused_reason(request.mode).to_owned(),
            detail: format!("{code}: {message}"),
        },
        Err(StartError::Herdr(error)) => WakeOutcome::Failed {
            reason: "The agent did not become ready in time.".to_owned(),
            detail: error.to_string(),
        },
    }
}

fn refused_reason(mode: WakeMode) -> &'static str {
    match mode {
        WakeMode::Resume => "The conversation couldn\u{2019}t be resumed.",
        WakeMode::Fresh => "A new session couldn\u{2019}t be started.",
    }
}

/// The machine a sleep or a wake runs on, gathered under the lock: the core's
/// own Herdr and node, or those of a node that dials the core (PRD
/// core-host-node-remote-core B13).
pub(crate) struct SleepTarget {
    pub(crate) api_connector: Arc<dyn ApiConnector>,
    pub(crate) node: Arc<dyn NodeLink>,
    pub(crate) runtime: Weak<Mutex<Runtime>>,
    pub(crate) notifier: ChangeNotifier,
    /// The pane as the machine's Herdr names it; a node's pane is kept in
    /// the core under its `remote:` scope.
    pub(crate) herdr_pane_id: String,
    /// Closed to the pane before the end began; the end waits for the writes
    /// that were admitted before that.
    pub(crate) input_gate: Arc<crate::input_gate::PaneInputGate>,
}

pub(crate) fn spawn_end(context: SleepTarget, pane_id: String, kind: String) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-agent-sleep".into())
        .spawn(move || {
            // A reply admitted before the pane closed is on its way to the
            // agent: let it land there before the agent is ended.
            context.input_gate.wait_drained(&pane_id, WRITE_DRAIN_LIMIT);
            let result = end_agent(
                context.api_connector.as_ref(),
                context.node.as_ref(),
                &context.herdr_pane_id,
                &kind,
            )
            .map_err(|error| error.to_string());
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
        .name("herdr-core-agent-wake".into())
        .spawn(move || {
            let outcome = start_agent(
                context.api_connector.as_ref(),
                context.node.as_ref(),
                &WakeRequest {
                    pane_id: context.herdr_pane_id.clone(),
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
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::{Value, json};

    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use hide_platform::process;

    fn agent_info(status: &str, seq: u64) -> Value {
        json!({"type": "agent_info", "agent": {
            "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "terminal_id": "term_1",
            "agent": "claude", "agent_status": status, "state_change_seq": seq,
            "focused": false, "interactive_ready": true, "revision": 0
        }})
    }

    fn process_info(shell: u32, foreground: u32) -> Value {
        json!({"type": "pane_process_info", "process_info": {
            "pane_id": "w1:p1", "shell_pid": shell, "foreground_process_group_id": foreground,
            "foreground_processes": []
        }})
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
            let herdr = FakeHerdr::start("wake-alias", move |method, params| match method {
                "pane.process_info" => process_info(42, 42),
                "agent.start" => {
                    assert_eq!(params["kind"], canonical);
                    assert_eq!(params["args"], json!(expected));
                    assert_eq!(params["pane_id"], "w1:p1");
                    let mut agent = agent_info("idle", 3)["agent"].clone();
                    agent["agent"] = json!(canonical);
                    json!({"type": "agent_started", "argv": [], "agent": agent})
                }
                other => panic!("unexpected {other}"),
            });
            assert_eq!(
                start_agent(
                    &herdr.connector(),
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
            assert_eq!(herdr.methods(), ["pane.process_info", "agent.start"]);
        }
    }

    #[test]
    fn sleep_end_accepts_known_aliases_but_keeps_process_group_and_agent_guards() {
        for kind in ["claude-code", " CLAUDE ", " CLAUDE_CODE "] {
            let herdr = FakeHerdr::start("end-alias", |method, _| match method {
                "agent.get" => agent_info("idle", 3),
                "pane.process_info" => process_info(42, 42),
                other => panic!("unexpected {other}"),
            });
            let error = end_agent(
                &herdr.connector(),
                &hide_node::Local::of_process(),
                "w1:p1",
                kind,
            )
            .expect_err("a canonical identity still cannot signal the shell")
            .to_string();
            assert!(error.contains("shell already holds"), "{kind}: {error}");
            assert_eq!(herdr.methods(), ["agent.get", "pane.process_info"]);
        }
        for kind in ["codex", "future-agent"] {
            let herdr = FakeHerdr::start("end-other-agent", |method, _| match method {
                "agent.get" => agent_info("idle", 3),
                other => panic!("identity refusal must not query or signal a group: {other}"),
            });
            assert!(
                end_agent(
                    &herdr.connector(),
                    &hide_node::Local::of_process(),
                    "w1:p1",
                    kind
                )
                .expect_err("another agent cannot be ended")
                .to_string()
                .contains("rather than")
            );
            assert_eq!(herdr.methods(), ["agent.get"]);
        }
    }

    /// A reply a phone admitted just before the pane closed is on its way to
    /// the agent. The worker must not touch the agent until it has landed, or
    /// the reply arrives behind the end and runs in the shell.
    #[test]
    fn the_end_waits_for_a_write_admitted_before_the_pane_closed() {
        let released = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&released);
        let herdr = FakeHerdr::start("agent-sleep-drain", move |method, _| match method {
            "agent.get" => {
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
        spawn_end(
            SleepTarget {
                api_connector: Arc::new(herdr.connector()),
                node: Arc::new(hide_node::Local::of_process()),
                runtime: Weak::new(),
                notifier: ChangeNotifier::new(),
                herdr_pane_id: "w1:p1".into(),
                input_gate: Arc::clone(&gate),
            },
            "w1:p1".into(),
            "claude".into(),
        )
        .expect("the worker starts");
        assert!(gate.admit("w1:p1").is_err(), "a later write is refused");
        released.store(true, Ordering::SeqCst);
        drop(admission);
        herdr.wait_for_requests(1, Duration::from_secs(30));
    }

    #[test]
    fn ending_signals_the_foreground_group_and_waits_for_the_shell() {
        let mut child =
            process::OwnedChild::spawn(Command::new("/bin/sleep").arg("60")).expect("sleep starts");
        let pid = child.id();
        let exited = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&exited);
        let herdr = FakeHerdr::start("agent-sleep-end", move |method, _| match method {
            "agent.get" => agent_info("idle", 7),
            "pane.process_info" => {
                // The shell (this test process stands in for it) takes the
                // terminal back only once the agent's group is really gone.
                let shell = std::process::id();
                let gone = !process::is_alive(pid) || seen.load(Ordering::SeqCst);
                process_info(shell, if gone { shell } else { pid })
            }
            other => panic!("unexpected {other}"),
        });
        let reaper = thread::spawn(move || {
            let _ = child.wait();
            exited.store(true, Ordering::SeqCst);
        });
        assert_eq!(
            end_agent(
                &herdr.connector(),
                &hide_node::Local::of_process(),
                "w1:p1",
                "claude"
            ),
            Ok(7)
        );
        reaper.join().expect("reaped");
        assert!(!process::is_alive(pid));
    }

    #[test]
    fn a_pane_whose_shell_holds_the_terminal_is_left_alone() {
        let herdr = FakeHerdr::start("agent-sleep-shell", |method, _| match method {
            "agent.get" => agent_info("idle", 3),
            "pane.process_info" => process_info(42, 42),
            other => panic!("unexpected {other}"),
        });
        let error = end_agent(
            &herdr.connector(),
            &hide_node::Local::of_process(),
            "w1:p1",
            "claude",
        )
        .expect_err("nothing to end")
        .to_string();
        assert!(error.contains("shell already holds"), "{error}");
        assert_eq!(herdr.methods(), ["agent.get", "pane.process_info"]);
    }

    #[test]
    fn a_group_that_would_signal_every_process_is_refused() {
        for (shell, foreground) in [(42, 0), (42, 1), (0, 42), (1, 42)] {
            let herdr = FakeHerdr::start("agent-sleep-broadcast", move |method, _| match method {
                "agent.get" => agent_info("idle", 3),
                "pane.process_info" => process_info(shell, foreground),
                other => panic!("unexpected {other}"),
            });
            let error = end_agent(
                &herdr.connector(),
                &hide_node::Local::of_process(),
                "w1:p1",
                "claude",
            )
            .expect_err("refused")
            .to_string();
            assert!(error.contains("not a pane's"), "{error}");
            assert_eq!(herdr.methods(), ["agent.get", "pane.process_info"]);
        }
    }

    #[test]
    fn an_agent_that_started_working_is_not_ended() {
        let herdr = FakeHerdr::start("agent-sleep-working", |method, _| match method {
            "agent.get" => agent_info("working", 3),
            other => panic!("unexpected {other}"),
        });
        let error = end_agent(
            &herdr.connector(),
            &hide_node::Local::of_process(),
            "w1:p1",
            "claude",
        )
        .expect_err("working");
        assert!(matches!(error, EndError::Busy(ref message) if message.contains("working")));
        assert_eq!(herdr.methods(), ["agent.get"]);
    }

    #[test]
    fn an_unknown_codex_wake_keeps_the_next_action_and_sends_no_start() {
        let herdr = FakeHerdr::start("codex-wake-capability", |method, _| {
            panic!("unexpected {method}")
        });
        let outcome = start_agent(
            &herdr.connector(),
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
        assert!(reason.contains("Settings"), "{reason}");
        assert!(herdr.methods().is_empty());
    }

    #[test]
    fn a_refused_wake_names_a_plain_reason_and_keeps_herdrs_words_for_the_log() {
        let herdr = FakeHerdr::start_with_errors("agent-wake-refused", |method, _| match method {
            "pane.process_info" => Ok(process_info(42, 42)),
            "agent.start" => Err((
                "agent_name_taken".into(),
                "agent name one is already used".into(),
            )),
            // Another agent holds the name, so reading it changes nothing.
            "agent.get" => Ok(serde_json::json!({"type": "agent_info", "agent": {
                "pane_id": "w1:p2", "terminal_id": "term_2", "workspace_id": "w1",
                "tab_id": "w1:t2", "name": "one", "focused": false, "agent": "claude",
                "agent_status": "working", "revision": 0}})),
            other => panic!("unexpected {other}"),
        });
        let outcome = start_agent(
            &herdr.connector(),
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
                reason: "The conversation couldn\u{2019}t be resumed.".into(),
                detail: "agent_name_taken: agent name one is already used".into(),
            }
        );
    }

    #[test]
    fn a_wake_into_a_folder_that_is_gone_asks_herdr_nothing() {
        let herdr = FakeHerdr::start("agent-wake-cwd", |method, _| panic!("unexpected {method}"));
        let outcome = start_agent(
            &herdr.connector(),
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
            WakeOutcome::Failed { ref reason, .. } if reason == "The working folder /nonexistent/agent-sleep-test no longer exists."
        ));
        assert!(herdr.methods().is_empty());
    }
}
