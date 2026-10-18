//! Reopen on a pane Hide cannot hear (PRD settings-cleanup B29, D-11).
//!
//! The session in the pane keeps its conversation: the agent is ended the way
//! agent sleep ends it and started again in the same pane with the agent's own
//! resume arguments, a Codex on its own server (`--no-daemon`, see
//! `codex_launch`). Nothing is ended before everything a start needs is known,
//! so a refusal that is known first leaves the pane exactly as it was. A start
//! that is refused after the end does not: the pane is left without an agent
//! (`ReopenFailure::ended`), the conversation is kept, and the runtime tells
//! the operator once through `last_error` (`pane_reopen.not_restarted`), which
//! the shell turns into a notice. Every call here runs on a worker thread,
//! never under `Mutex<Runtime>`; the answer is a code the pane's popover turns
//! into one line, and Herdr's own words stay in the diagnostic.

use std::thread;

use hide_herdr_client::ApiConnector;

use crate::agent_sleep::WakeMode;
use crate::agent_sleep_herdr::{self, EndError, WakeOutcome, WakeRequest};
use crate::live::LiveContext;
use crate::model::PaneReopenFailure;
use crate::recent_closed::ClosedAgent;

/// What a reopen needs, gathered under the lock before the worker starts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReopenRequest {
    pub(crate) pane_id: String,
    pub(crate) kind: String,
    pub(crate) session_id: String,
    pub(crate) name: String,
    pub(crate) cwd: Option<String>,
    pub(crate) codex_daemon: crate::codex_launch::CodexDaemon,
    /// The model and effort the agent was started with, which the start
    /// after the end carries again.
    pub(crate) launch: hide_agent_adapter::LaunchChoice,
}

/// A reopen that did not happen: `code` is what the pane says, `detail` what
/// only the diagnostic carries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReopenFailure {
    pub(crate) code: PaneReopenFailure,
    pub(crate) detail: String,
    /// The agent was ended before the start was refused, so the pane no
    /// longer holds the conversation and its chip cannot carry the failure.
    pub(crate) ended: bool,
}

fn failure(code: PaneReopenFailure, detail: impl Into<String>) -> ReopenFailure {
    ReopenFailure {
        code,
        detail: detail.into(),
        ended: false,
    }
}

/// Ends the agent and starts the same conversation again in the same pane.
///
/// Order is the contract: every refusal that can be known first (an unread
/// Codex, a conversation without resume arguments, a folder that is gone) is
/// answered before the agent is touched; the agent is ended only then, and the
/// start follows only once the shell holds the terminal again.
pub(crate) fn reopen(
    connector: &dyn ApiConnector,
    node: &dyn crate::node_access::NodeLink,
    request: &ReopenRequest,
) -> Result<(), ReopenFailure> {
    if crate::codex_launch::start_arguments(&request.kind, request.codex_daemon, Vec::new())
        .is_err()
    {
        return Err(failure(
            PaneReopenFailure::CodexUnread,
            "the machine's Codex has not been read",
        ));
    }
    let Some(mut args) = crate::recent_closed::resume_arguments(&ClosedAgent {
        kind: request.kind.clone(),
        session_id: Some(request.session_id.clone()),
        ..Default::default()
    }) else {
        return Err(failure(
            PaneReopenFailure::SessionGone,
            format!("{} has no resume arguments", request.kind),
        ));
    };
    args.extend(
        crate::runtime::agent_choice::launch_arguments(&request.kind, &request.launch)
            .map_err(|detail| failure(PaneReopenFailure::StartRefused, detail))?,
    );
    if let Some(cwd) = request.cwd.as_deref()
        && !crate::node_access::is_directory(node, cwd)
    {
        return Err(failure(
            PaneReopenFailure::StartRefused,
            format!("{cwd} is not a directory"),
        ));
    }
    agent_sleep_herdr::end_agent(connector, node, &request.pane_id, &request.kind).map_err(
        |error| {
            match error {
                // The decision was made on a list up to a second old, so an agent
                // that started working since is told as busy, and nothing was
                // signalled.
                EndError::Busy(detail) => failure(PaneReopenFailure::AgentBusy, detail),
                EndError::Failed(detail) => failure(PaneReopenFailure::EndRefused, detail),
            }
        },
    )?;
    match agent_sleep_herdr::start_agent(
        connector,
        node,
        &WakeRequest {
            pane_id: request.pane_id.clone(),
            kind: request.kind.clone(),
            name: request.name.clone(),
            mode: WakeMode::Resume,
            args,
            cwd: request.cwd.clone(),
            session_id: Some(request.session_id.clone()),
            codex_daemon: request.codex_daemon,
        },
    ) {
        WakeOutcome::Started => Ok(()),
        WakeOutcome::Failed { detail, .. } => Err(ReopenFailure {
            ended: true,
            ..failure(PaneReopenFailure::StartRefused, detail)
        }),
    }
}

pub(crate) fn spawn_reopen(context: LiveContext, request: ReopenRequest) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-pane-reopen".into())
        .spawn(move || {
            let result = reopen(
                context.api_connector.as_ref(),
                context.node.as_ref(),
                &request,
            );
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => guard.ingest_pane_reopen(&request.pane_id, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("pane reopen worker could not be started: {error}"))
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use hide_platform::process;
    use serde_json::{Value, json};

    use super::*;
    use crate::codex_launch::CodexDaemon;
    use crate::fake_herdr::FakeHerdr;

    fn request(kind: &str, daemon: CodexDaemon) -> ReopenRequest {
        ReopenRequest {
            pane_id: "w1:p1".into(),
            kind: kind.into(),
            session_id: "11111111-2222-3333-4444-555555555555".into(),
            name: "reviewer".into(),
            cwd: None,
            codex_daemon: daemon,
            launch: Default::default(),
        }
    }

    fn agent_info(kind: &str, status: &str) -> Value {
        json!({"type": "agent_info", "agent": {
            "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "terminal_id": "term_1",
            "agent": kind, "agent_status": status, "state_change_seq": 7,
            "focused": false, "interactive_ready": true, "revision": 0
        }})
    }

    fn process_info(shell: u32, foreground: u32) -> Value {
        json!({"type": "pane_process_info", "process_info": {
            "pane_id": "w1:p1", "shell_pid": shell, "foreground_process_group_id": foreground,
            "foreground_processes": [{"pid": shell, "name": "zsh"}]
        }})
    }

    /// B29: a refusal known up front touches nothing, so the pane is as it was.
    #[test]
    fn a_refusal_known_before_the_end_asks_herdr_nothing() {
        for (case, kind, daemon, expected) in [
            (
                "unread",
                "codex",
                CodexDaemon::Unknown,
                PaneReopenFailure::CodexUnread,
            ),
            (
                "no resume",
                "gemini",
                CodexDaemon::Present,
                PaneReopenFailure::SessionGone,
            ),
        ] {
            let herdr = FakeHerdr::start("pane-reopen-refused", |method, _| {
                panic!("unexpected {method}")
            });
            let error = reopen(
                &herdr.connector(),
                &hide_node::Local::of_process(),
                &request(kind, daemon),
            )
            .expect_err(case);
            assert_eq!(error.code, expected, "{case}");
            assert!(!error.ended, "{case}");
            assert!(herdr.methods().is_empty(), "{case}");
        }
        let herdr = FakeHerdr::start("pane-reopen-cwd", |method, _| panic!("unexpected {method}"));
        let mut gone = request("claude", CodexDaemon::Unknown);
        gone.cwd = Some("/nonexistent/pane-reopen-test".into());
        let error = reopen(&herdr.connector(), &hide_node::Local::of_process(), &gone)
            .expect_err("folder gone");
        assert_eq!(error.code, PaneReopenFailure::StartRefused);
        assert!(herdr.methods().is_empty());
    }

    /// An agent that started working since the decision is not ended.
    #[test]
    fn an_agent_that_is_working_now_is_not_ended_or_started_again() {
        let herdr = FakeHerdr::start("pane-reopen-working", |method, _| match method {
            "agent.get" => agent_info("claude", "working"),
            other => panic!("unexpected {other}"),
        });
        let error = reopen(
            &herdr.connector(),
            &hide_node::Local::of_process(),
            &request("claude", CodexDaemon::Unknown),
        )
        .expect_err("working");
        assert_eq!(error.code, PaneReopenFailure::AgentBusy);
        assert_eq!(herdr.methods(), ["agent.get"]);
    }

    /// B29: the agent ends first, and only after the shell holds the terminal
    /// does `agent.start` run, resuming the same conversation; Codex goes
    /// first with `--no-daemon`.
    #[test]
    fn the_agent_ends_and_then_starts_again_with_its_own_conversation() {
        for (kind, daemon, resume_prefix) in [
            ("claude", CodexDaemon::Unknown, vec!["--resume"]),
            ("codex", CodexDaemon::Present, vec!["--no-daemon", "resume"]),
        ] {
            let mut child = process::OwnedChild::spawn(Command::new("/bin/sleep").arg("60"))
                .expect("sleep starts");
            let pid = child.id();
            let exited = Arc::new(AtomicBool::new(false));
            let seen = Arc::clone(&exited);
            let herdr = FakeHerdr::start("pane-reopen-ok", move |method, _| match method {
                "agent.get" => agent_info(kind, "idle"),
                "pane.process_info" => {
                    let shell = std::process::id();
                    let gone = !process::is_alive(pid) || seen.load(Ordering::SeqCst);
                    process_info(shell, if gone { shell } else { pid })
                }
                "agent.start" => json!({"type": "agent_started", "argv": [], "agent": {
                    "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1",
                    "terminal_id": "term_1", "agent_status": "idle", "focused": false,
                    "revision": 1
                }}),
                other => panic!("unexpected {other}"),
            });
            let reaper = thread::spawn(move || {
                let _ = child.wait();
                exited.store(true, Ordering::SeqCst);
            });
            reopen(
                &herdr.connector(),
                &hide_node::Local::of_process(),
                &request(kind, daemon),
            )
            .expect("reopened");
            reaper.join().expect("reaped");
            let methods = herdr.methods();
            let first_start = methods
                .iter()
                .position(|method| method == "agent.start")
                .expect("a start was sent");
            assert_eq!(methods[0], "agent.get", "{kind}");
            let calls = herdr.calls();
            let (_, start) = &calls[first_start];
            let args: Vec<&str> = start["args"]
                .as_array()
                .expect("args")
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert!(
                args.starts_with(&resume_prefix),
                "{kind} starts with {resume_prefix:?}, got {args:?}"
            );
            assert!(
                args.contains(&"11111111-2222-3333-4444-555555555555"),
                "{args:?}"
            );
            assert!(
                methods[..first_start]
                    .iter()
                    .all(|method| method != "agent.start"),
                "{kind}: {methods:?}"
            );
            assert_eq!(
                methods.iter().filter(|m| *m == "agent.start").count(),
                1,
                "{kind}: one start only"
            );
        }
    }

    /// A start Herdr refuses after the end is reported as one, with Herdr's
    /// words kept for the log.
    #[test]
    fn a_refused_start_is_a_start_refused_with_herdrs_words_in_the_detail() {
        let mut child =
            process::OwnedChild::spawn(Command::new("/bin/sleep").arg("60")).expect("sleep starts");
        let pid = child.id();
        let exited = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&exited);
        let herdr =
            FakeHerdr::start_with_errors(
                "pane-reopen-start-refused",
                move |method, _| match method {
                    "agent.get" => Ok(agent_info("claude", "idle")),
                    "pane.process_info" => {
                        let shell = std::process::id();
                        let gone = !process::is_alive(pid) || seen.load(Ordering::SeqCst);
                        Ok(process_info(shell, if gone { shell } else { pid }))
                    }
                    "agent.start" => Err(("agent_name_taken".into(), "name is used".into())),
                    other => panic!("unexpected {other}"),
                },
            );
        let reaper = thread::spawn(move || {
            let _ = child.wait();
            exited.store(true, Ordering::SeqCst);
        });
        let error = reopen(
            &herdr.connector(),
            &hide_node::Local::of_process(),
            &request("claude", CodexDaemon::Unknown),
        )
        .expect_err("refused");
        reaper.join().expect("reaped");
        assert_eq!(error.code, PaneReopenFailure::StartRefused);
        assert!(
            error.detail.contains("agent_name_taken"),
            "{}",
            error.detail
        );
        assert!(
            error.ended,
            "the agent was ended before the start was refused"
        );
        let methods = herdr.methods();
        let first_start = methods
            .iter()
            .position(|method| method == "agent.start")
            .expect("a start was sent");
        assert!(
            methods[..first_start].iter().all(|m| m != "agent.start") && methods[0] == "agent.get",
            "the end came first: {methods:?}"
        );
    }

    /// Herdr keeps the ended agent's name for a moment, so the first starts
    /// are refused as `agent_name_taken`; the reopen waits that out instead
    /// of leaving the pane on its shell.
    #[test]
    fn a_name_herdr_has_not_released_yet_is_waited_out_and_the_session_starts() {
        let mut child =
            process::OwnedChild::spawn(Command::new("/bin/sleep").arg("60")).expect("sleep starts");
        let pid = child.id();
        let exited = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&exited);
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let attempts = Arc::clone(&starts);
        let herdr =
            FakeHerdr::start_with_errors("pane-reopen-name", move |method, _| match method {
                "agent.get" => Ok(agent_info("claude", "idle")),
                "pane.process_info" => {
                    let shell = std::process::id();
                    let gone = !process::is_alive(pid) || seen.load(Ordering::SeqCst);
                    Ok(process_info(shell, if gone { shell } else { pid }))
                }
                "agent.start" if attempts.fetch_add(1, Ordering::SeqCst) < 2 => Err((
                    "agent_name_taken".into(),
                    "agent name reviewer is already used".into(),
                )),
                "agent.start" => Ok(json!({"type": "agent_started", "argv": [], "agent": {
                    "pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1",
                    "terminal_id": "term_1", "agent_status": "idle", "focused": false,
                    "revision": 1
                }})),
                other => panic!("unexpected {other}"),
            });
        let reaper = thread::spawn(move || {
            let _ = child.wait();
            exited.store(true, Ordering::SeqCst);
        });
        reopen(
            &herdr.connector(),
            &hide_node::Local::of_process(),
            &request("claude", CodexDaemon::Unknown),
        )
        .expect("reopened");
        reaper.join().expect("reaped");
        assert_eq!(
            starts.load(Ordering::SeqCst),
            3,
            "two refusals, then the start"
        );
    }
}
