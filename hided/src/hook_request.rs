//! Bounded hook reads, shared by local and node proof exchanges.
//! Authentication belongs to the proof owners; execution remains the same
//! core operations the scoped Workspace route uses.

use std::time::{Duration, Instant};

use hide_node_link::panes::{
    AgentExecEvent, ExecProcess, HOOK_REQUEST_LIMIT, HOOK_RESPONSE_LIMIT, HookCommand, HookRequest,
};
use serde_json::{Value, json};

use crate::core::CoreHandle;
use crate::pane_auth::{Capability, Registry};

pub(crate) type Failure = (String, &'static str);

/// Withdraw newly issued authority on failure or unwind. Letter and Memory
/// tokens always end here; guidance transfers its persistent reference only
/// after a successful answer. A reused reference still belongs to its caller.
pub(crate) struct Token<'a>(pub &'a Registry, pub String, pub bool);

impl Token<'_> {
    /// Transfer a persistent reference only after its successful answer.
    pub(crate) fn keep(mut self) {
        self.2 = false;
    }
}

impl Drop for Token<'_> {
    fn drop(&mut self) {
        if self.2 {
            self.0.revoke(&self.1);
        }
    }
}

pub(crate) fn valid(request: &HookRequest) -> bool {
    serde_json::to_vec(request).is_ok_and(|encoded| encoded.len() < HOOK_REQUEST_LIMIT)
        && !request.request_id.is_empty()
        && request.request_id.len() <= 64
        && request
            .request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && request
            .token
            .as_ref()
            .is_none_or(|token| token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        && match &request.command {
            HookCommand::WorkspaceInfo {} | HookCommand::Registration {} => true,
            HookCommand::Memory {
                event,
                runtime,
                session,
                prompt,
                prompt_digest,
            } => memory_ask(event, runtime, session, prompt, prompt_digest.as_deref()).is_some(),
            HookCommand::Status { report } => valid_status(report),
            HookCommand::AgentExec { .. } => request.command.agent_exec_valid(),
            HookCommand::LetterPull {
                prompt_digest,
                session,
                point,
                ..
            } => {
                prompt_digest.as_ref().is_none_or(|digest| {
                    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
                }) && session
                    .as_ref()
                    .is_none_or(|session| hide_core::delivery::valid_session(session))
                    // A tool event's ask names its session.
                    && (*point == hide_node_link::panes::PullPoint::Prompt || session.is_some())
            }
            HookCommand::LetterConfirm { ids } => {
                !ids.is_empty() && hide_agent_hooks::delivery::valid_letter_ids(ids)
            }
        }
}

/// `executor` is the process that sent an `AgentExec`, as the kernel of
/// the pane's machine says it is that attempt's executor
/// (`hide_agent_hooks::hook_request::executor_process`); none for every
/// other command. `lineage` is the processes from the one that sent a
/// `Status` up to the pane's shell, as that kernel names them
/// (`hide_host::pane_peer::lineage`); none for every other command and
/// when the system could not show them.
pub(crate) fn answer(
    core: &CoreHandle,
    cap: &Capability,
    request: &HookRequest,
    executor: Option<ExecProcess>,
    lineage: Option<Vec<ExecProcess>>,
    screens: &crate::server::Screens,
) -> Value {
    let result = match &request.command {
        HookCommand::WorkspaceInfo {} => workspace_info(core, cap, screens),
        // The caller's proof admitted it to a registered checkout.
        HookCommand::Registration {} => Ok(json!({})),
        HookCommand::LetterPull {
            prompt_digest,
            session,
            point,
            bundle,
        } => delivery(
            core,
            cap,
            hide_core::delivery::Command::Pull {
                prompt_digest: prompt_digest.clone(),
                session: session.clone(),
                point: *point,
                bundle: *bundle,
            },
            None,
        )
        .map(|mut result| {
            // The prompt's pull also carries the pane's plan summary, which
            // the hook keeps for its hint and its refusals (PRD hide-plan
            // D-30): no request of its own, and none at a tool event.
            if *point == hide_node_link::panes::PullPoint::Prompt
                && let Some(context) = core.plan_context(&cap.context.device_id, &cap.pane_id)
                && let Value::Object(fields) = &mut result
            {
                fields.insert("plan_context".to_owned(), json!(context));
            }
            result
        }),
        HookCommand::LetterConfirm { ids } => delivery(
            core,
            cap,
            hide_core::delivery::Command::Confirm { ids: ids.clone() },
            None,
        ),
        HookCommand::Status { report } => status(core, cap, report, lineage),
        HookCommand::AgentExec { attempt, event } => {
            agent_exec(core, cap, attempt, event.clone(), executor)
        }
        HookCommand::Memory {
            event,
            runtime,
            session,
            prompt,
            prompt_digest,
        } => match memory_ask(event, runtime, session, prompt, prompt_digest.as_deref()) {
            Some(ask) => memory(core, cap, &ask),
            None => Err((
                "invalid_request".to_owned(),
                "Check hide command arguments and retry",
            )),
        },
    };
    let answer = match result {
        Ok(result) => {
            json!({"type":"workspace_result","request_id":request.request_id,"ok":true,"result":result})
        }
        Err((reason, next_action)) => {
            json!({"type":"workspace_result","request_id":request.request_id,"ok":false,"reason":reason,"next_action":next_action})
        }
    };
    if answer.to_string().len() + 1 > HOOK_RESPONSE_LIMIT {
        json!({"type":"workspace_result","request_id":request.request_id,"ok":false,"reason":"response_too_large","next_action":"Continue without hook context"})
    } else {
        answer
    }
}

/// A report names a known agent and, when it names one, a session id the
/// delivery ledger would take; a refusal it carries is one line the helper
/// cut (`report::REFUSAL_LINE_LIMIT`).
fn valid_status(report: &hide_node_link::panes::HookStatusReport) -> bool {
    hide_agent_adapter::adapter(&report.runtime).is_some_and(|adapter| adapter.id == report.runtime)
        && report
            .session
            .as_ref()
            .is_none_or(|session| hide_core::delivery::valid_session(session))
        && report.refused.as_ref().is_none_or(|refused| {
            refused.line.len() <= hide_agent_hooks::report::REFUSAL_LINE_LIMIT
                && !refused.line.contains('\n')
        })
}

/// Records the hook's report on the pane the proof named. Only a caller
/// proved to run in its pane reports: a checkout-bound caller (a hook in
/// Codex's shared server) names no pane the report could be about. A refusal
/// is written to the diagnostic log with the pane and the agent; nothing
/// reaches the screen (PRD engine-bundle-1 B14, design principle 13).
fn status(
    core: &CoreHandle,
    cap: &Capability,
    report: &hide_node_link::panes::HookStatusReport,
    lineage: Option<Vec<ExecProcess>>,
) -> Result<Value, Failure> {
    cap.pane_terminal()
        .ok_or("agent_pane_required")
        .and_then(|_| {
            core.hook_status(
                &cap.context.device_id,
                &cap.pane_id,
                report.clone(),
                lineage,
            )
        })
        .map(|()| json!({"recorded": true}))
        .map_err(|reason| {
            hide_core::diagnostic!(json!({
                "component": "agent_hooks",
                "kind": "status.refused",
                "pane": cap.pane_id,
                "device": cap.context.device_id,
                "agent": report.runtime,
                "reason": reason,
            }));
            (reason.to_owned(), "Continue without the status report")
        })
}

/// What `hide agent exec` in the caller's pane asked or reported of its
/// profile's start, which ends the engine's wait for it.
/// An `AgentExec` report, taken only with the kernel's word for its
/// sender: a `Ready` refused here leaves the program unstarted. Its ask for
/// the profile (`Profile`) is answered with how the profile runs now
/// (`ExecProfile`), its machine asked on this thread.
fn agent_exec(
    core: &CoreHandle,
    cap: &Capability,
    attempt: &str,
    event: AgentExecEvent,
    executor: Option<ExecProcess>,
) -> Result<Value, Failure> {
    let proved = cap
        .pane_terminal()
        .ok_or("agent_pane_required")
        .and_then(|_| executor.ok_or("caller_unavailable"));
    if event == AgentExecEvent::Profile {
        return proved
            .and_then(|_| core.exec_start(&cap.context.device_id, &cap.pane_id, attempt))
            .and_then(|answer| serde_json::to_value(answer).map_err(|_| "result_encoding_failed"))
            .map_err(|reason| (reason.to_owned(), "Start the agent again"));
    }
    proved
        .and_then(|executor| {
            core.agent_exec(
                &cap.context.device_id,
                &cap.pane_id,
                hide_core::engine::AgentExecReport {
                    attempt: attempt.to_owned(),
                    event,
                    executor,
                },
            )
        })
        .map(|()| json!({"recorded": true}))
        .map_err(|reason| (reason.to_owned(), "Start the agent again"))
}

/// The answer an agent hook's `hide workspace info --hook` gets, its page commands told
/// as the command loop tells them (`Seen::scope`).
fn workspace_info(
    core: &CoreHandle,
    cap: &Capability,
    screens: &crate::server::Screens,
) -> Result<Value, Failure> {
    let mut result = core
        .workspace_query(
            &cap.context.device_id,
            &cap.pane_id,
            hide_core::workspace_control::Query::Info,
        )
        .map_err(|refusal| (refusal.reason.to_owned(), refusal.next_action))?;
    if result.context != cap.context {
        let (reason, next_action) = cap.changed_refusal();
        return Err((reason.to_owned(), next_action));
    }
    screens.now().scope(&mut result);
    serde_json::to_value(result).map_err(|_| {
        (
            "result_encoding_failed".to_owned(),
            "Reconnect Hide and retry",
        )
    })
}

fn memory_ask(
    event: &str,
    runtime: &str,
    session: &str,
    prompt: &str,
    prompt_digest: Option<&str>,
) -> Option<hide_core::memory_hook::Ask> {
    hide_core::memory_hook::Ask::from_wire(&json!({
        "event": event, "runtime": runtime, "session": session, "prompt": prompt,
        "prompt_digest": prompt_digest,
    }))
}

pub(crate) fn delivery(
    core: &CoreHandle,
    cap: &Capability,
    command: hide_core::delivery::Command,
    hint: Option<String>,
) -> Result<Value, Failure> {
    core.prepare_delivery(
        &cap.context.device_id,
        &cap.pane_id,
        &cap.context,
        cap.canonical_hint(hint),
        command,
    )
    .and_then(|prepared| prepared.run(Duration::from_secs(5)))
    .map_err(|code| {
        let next = crate::server::delivery_next_action(&code);
        (code, next)
    })
}

pub(crate) fn memory(
    core: &CoreHandle,
    cap: &Capability,
    ask: &hide_core::memory_hook::Ask,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + hide_core::memory_hook::WITHIN;
    let mine = |session: &str| (cap.pane_id.clone(), session.to_owned());
    let (scope, give, brief) = core.hook_scope(
        &cap.context,
        &cap.pane_id,
        ask.prompt_session().map(mine),
        ask.start_session().map(mine),
        deadline,
    );
    let scope = scope.map_err(|reason| {
        hide_core::diagnostic!(json!({"component":"memory","kind":"hook.unavailable","pane":cap.pane_id,"device":cap.context.device_id,"reason":reason}));
    });
    // The session's open questions are read beside Memory and left out when
    // they miss the answer's deadline: the next prompt gives them, since a
    // question counts as given only once its receipt comes back from the
    // session's record.
    let asks = beside(
        &cap.pane_id,
        "asks.give",
        deadline,
        give.map(|give| move || give.context()),
    );
    // The Project's other sessions, read the same way; a start that misses
    // them starts without them.
    let sessions = beside(
        &cap.pane_id,
        "brief.give",
        deadline,
        brief.map(|brief| move || hide_core::links::session_brief::for_session_start(&brief)),
    );
    memory_answer(core.state_dir(), scope, ask, deadline, asks, sessions)
}

/// `read` on a thread of its own that ends with it, while Memory is read on
/// this one; the closure it returns waits for its answer until `deadline`,
/// and gives none past it, with the late read logged under `name`.
fn beside(
    pane: &str,
    name: &'static str,
    deadline: Instant,
    read: Option<impl FnOnce() -> Option<String> + Send + 'static>,
) -> impl FnOnce() -> Option<String> {
    let receive = read.and_then(|read| {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("hook-{name}"))
            .spawn(move || {
                let _ = send.send(read());
            })
            .map(|_| receive)
            .map_err(|error| {
                hide_core::diagnostic!(json!({"component":"session","kind":format!("{name}_unstarted"),"pane":pane,"error":error.to_string()}));
            })
            .ok()
    });
    let pane = pane.to_owned();
    move || {
        receive.and_then(|receive| {
            match receive.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(answer) => answer,
                Err(_) => {
                    hide_core::diagnostic!(
                        json!({"component":"session","kind":format!("{name}_late"),"pane":pane})
                    );
                    None
                }
            }
        })
    }
}

/// A hook's answer: Memory's context when its scope was found, on a session
/// start the Hide global instruction whatever Memory answers (off,
/// unresolved or unreachable; PRD memory-self-improve D-31) and the
/// Project's other sessions `sessions` reads, and at a prompt the session's
/// open questions `asks` reads, both whatever Memory answers too. The global
/// read is one small bounded file, outside Memory's deadline.
fn memory_answer(
    state_dir: &std::path::Path,
    scope: Result<hide_core::memory_hook::Scope, ()>,
    ask: &hide_core::memory_hook::Ask,
    deadline: Instant,
    asks: impl FnOnce() -> Option<String>,
    sessions: impl FnOnce() -> Option<String>,
) -> Result<Value, Failure> {
    let global = ask
        .starts_session()
        .then(|| hide_core::global_instruction::for_session(state_dir))
        .flatten();
    let memory = scope
        .ok()
        .map(|scope| hide_core::memory_hook::answer(&scope, ask, deadline));
    let answer = match (memory, global, asks(), sessions()) {
        (Some(memory), global, asks, sessions) => hide_core::memory_hook::HookContext {
            global,
            asks,
            sessions,
            ..memory
        },
        (None, None, None, None) => {
            return Err((
                "memory_unavailable".to_owned(),
                "Continue without Project Memory",
            ));
        }
        (None, global, asks, sessions) => {
            hide_core::memory_hook::without_memory(global, asks, sessions)
        }
    };
    serde_json::to_value(answer).map_err(|_| {
        (
            "result_encoding_failed".to_owned(),
            "Continue without Project Memory",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(event: &str) -> hide_core::memory_hook::Ask {
        memory_ask(event, "claude", "s-1", "", None).unwrap()
    }

    /// A hook's confirmation names at most one bundle of letters, each an id
    /// `hide inbox --confirm` would take; an empty list, an empty id, one
    /// that reads as a flag or one with a line break is refused before the
    /// core sees it. Whose letters they are is the core's to decide
    /// (`hide_core::delivery`), as for the command.
    #[test]
    fn a_letter_confirmation_names_letters_the_inbox_would_take() {
        let request = |ids: Vec<String>| HookRequest {
            request_id: "r-1".into(),
            command: HookCommand::LetterConfirm { ids },
            token: None,
        };
        let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert!(valid(&request(ids(&["letter-1", "letter-2"]))));
        for refused in [
            ids(&[]),
            ids(&[""]),
            ids(&["--all"]),
            ids(&["letter\n1"]),
            (0..=hide_agent_hooks::delivery::BUNDLE_LETTERS)
                .map(|n| format!("letter-{n}"))
                .collect(),
        ] {
            assert!(!valid(&request(refused.clone())), "{refused:?}");
        }
    }

    /// B1, D-31: a session start with no Memory still gets the global
    /// instruction; a prompt, or an empty file, keeps the old refusal.
    #[test]
    fn a_session_start_gets_the_global_instruction_without_memory() {
        let state = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + hide_core::memory_hook::WITHIN;
        assert_eq!(
            memory_answer(
                state.path(),
                Err(()),
                &ask("SessionStart"),
                deadline,
                || None,
                || None
            )
            .unwrap_err()
            .0,
            "memory_unavailable"
        );
        std::fs::write(
            hide_kit::layout::global_instruction(state.path()),
            "Answer in Korean.\n",
        )
        .unwrap();
        let answer = memory_answer(
            state.path(),
            Err(()),
            &ask("SessionStart"),
            deadline,
            || None,
            || None,
        )
        .unwrap();
        assert_eq!(answer["context"], Value::Null);
        assert_eq!(answer["outcome"], "unavailable");
        assert!(
            answer["global"]
                .as_str()
                .unwrap()
                .contains("trust=\"operator-approved\">\nThe operator approved")
        );
        assert!(
            answer["global"]
                .as_str()
                .unwrap()
                .contains("Answer in Korean.")
        );
        assert_eq!(
            memory_answer(
                state.path(),
                Err(()),
                &ask("UserPromptSubmit"),
                deadline,
                || None,
                || None
            )
            .unwrap_err()
            .0,
            "memory_unavailable"
        );
    }

    /// A prompt's open questions reach the agent whether or not Memory
    /// answers, in their own slot.
    #[test]
    fn a_prompt_gets_its_open_questions_without_memory() {
        let state = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + hide_core::memory_hook::WITHIN;
        let answer = memory_answer(
            state.path(),
            Err(()),
            &ask("UserPromptSubmit"),
            deadline,
            || Some("<hide-open-questions>".to_owned()),
            || None,
        )
        .unwrap();
        assert_eq!(answer["context"], Value::Null);
        assert_eq!(answer["asks"], "<hide-open-questions>");
    }

    /// A session start's other sessions reach the agent whether or not
    /// Memory answers, in their own slot beside the global instruction.
    #[test]
    fn a_session_start_gets_the_projects_other_sessions_without_memory() {
        let state = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + hide_core::memory_hook::WITHIN;
        let answer = memory_answer(
            state.path(),
            Err(()),
            &ask("SessionStart"),
            deadline,
            || None,
            || Some("<hide-project-sessions>".to_owned()),
        )
        .unwrap();
        assert_eq!(answer["context"], Value::Null);
        assert_eq!(answer["global"], Value::Null);
        assert_eq!(answer["sessions"], "<hide-project-sessions>");
    }
}
