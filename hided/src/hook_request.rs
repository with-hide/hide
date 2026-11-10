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
            } => memory_ask(event, runtime, session, prompt).is_some(),
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
/// other command.
pub(crate) fn answer(
    core: &CoreHandle,
    cap: &Capability,
    request: &HookRequest,
    executor: Option<ExecProcess>,
    desktop: bool,
) -> Value {
    let started = Instant::now();
    let mark = |stage: &str| hide_core::diagnostic!(json!({"component":"hook_probe","kind":"answer.stage","request_id":request.request_id,"stage":stage,"elapsed_ms":started.elapsed().as_millis()}));
    let result = match &request.command {
        HookCommand::WorkspaceInfo {} => workspace_info(core, cap, desktop),
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
            mark("delivery_end");
            // The prompt's pull also carries the pane's plan summary, which
            // the hook keeps for its hint and its refusals (PRD hide-plan
            // D-30): no request of its own, and none at a tool event.
            if *point == hide_node_link::panes::PullPoint::Prompt
                && let Some(context) = core.plan_context(&cap.context.device_id, &cap.pane_id)
                && let Value::Object(fields) = &mut result
            {
                fields.insert("plan_context".to_owned(), json!(context));
            }
            mark("plan_end");
            result
        }),
        HookCommand::LetterConfirm { ids } => delivery(
            core,
            cap,
            hide_core::delivery::Command::Confirm { ids: ids.clone() },
            None,
        ),
        HookCommand::Status { report } => status(core, cap, report),
        HookCommand::AgentExec { attempt, event } => {
            agent_exec(core, cap, attempt, event.clone(), executor)
        }
        HookCommand::Memory {
            event,
            runtime,
            session,
            prompt,
        } => match memory_ask(event, runtime, session, prompt) {
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
/// delivery ledger would take.
fn valid_status(report: &hide_node_link::panes::HookStatusReport) -> bool {
    hide_agent_adapter::adapter(&report.runtime).is_some_and(|adapter| adapter.id == report.runtime)
        && report
            .session
            .as_ref()
            .is_none_or(|session| hide_core::delivery::valid_session(session))
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
) -> Result<Value, Failure> {
    cap.pane_terminal()
        .ok_or("agent_pane_required")
        .and_then(|_| core.hook_status(&cap.context.device_id, &cap.pane_id, report.clone()))
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

fn workspace_info(core: &CoreHandle, cap: &Capability, desktop: bool) -> Result<Value, Failure> {
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
    if !desktop {
        result
            .capabilities
            .retain(|capability| !capability.starts_with("browser."));
    }
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
) -> Option<hide_core::memory_hook::Ask> {
    hide_core::memory_hook::Ask::from_wire(
        &json!({"event":event,"runtime":runtime,"session":session,"prompt":prompt}),
    )
}

pub(crate) fn delivery(
    core: &CoreHandle,
    cap: &Capability,
    command: hide_core::delivery::Command,
    hint: Option<String>,
) -> Result<Value, Failure> {
    let started = Instant::now();
    let prepared = core.prepare_delivery(
        &cap.context.device_id,
        &cap.pane_id,
        &cap.context,
        cap.canonical_hint(hint),
        command,
    );
    hide_core::diagnostic!(json!({"component":"hook_probe","kind":"delivery.prepared","pane":cap.pane_id,"elapsed_ms":started.elapsed().as_millis()}));
    prepared.and_then(|prepared| {
        let result = prepared.run(Duration::from_secs(5));
        hide_core::diagnostic!(json!({"component":"hook_probe","kind":"delivery.ran","pane":cap.pane_id,"elapsed_ms":started.elapsed().as_millis()}));
        result
    })
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
    let scope = core.memory_scope(&cap.context, deadline).map_err(|reason| {
        hide_core::diagnostic!(json!({"component":"memory","kind":"hook.unavailable","pane":cap.pane_id,"device":cap.context.device_id,"reason":reason}));
    });
    memory_answer(core.state_dir(), scope, ask, deadline)
}

/// A hook's answer: Memory's context when its scope was found, and on a
/// session start the Hide global instruction whatever Memory answers (off,
/// unresolved or unreachable; PRD memory-self-improve D-31). The global read
/// is one small bounded file, outside Memory's deadline.
fn memory_answer(
    state_dir: &std::path::Path,
    scope: Result<hide_core::memory_hook::Scope, ()>,
    ask: &hide_core::memory_hook::Ask,
    deadline: Instant,
) -> Result<Value, Failure> {
    let global = ask
        .starts_session()
        .then(|| hide_core::global_instruction::for_session(state_dir))
        .flatten();
    let answer = match (scope, global) {
        (Ok(scope), global) => hide_core::memory_hook::HookContext {
            global,
            ..hide_core::memory_hook::answer(&scope, ask, deadline)
        },
        (Err(()), Some(global)) => hide_core::memory_hook::global_only(global),
        (Err(()), None) => {
            return Err((
                "memory_unavailable".to_owned(),
                "Continue without Project Memory",
            ));
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
        memory_ask(event, "claude", "s-1", "").unwrap()
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
            memory_answer(state.path(), Err(()), &ask("SessionStart"), deadline)
                .unwrap_err()
                .0,
            "memory_unavailable"
        );
        std::fs::write(
            hide_kit::layout::global_instruction(state.path()),
            "Answer in Korean.\n",
        )
        .unwrap();
        let answer = memory_answer(state.path(), Err(()), &ask("SessionStart"), deadline).unwrap();
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
            memory_answer(state.path(), Err(()), &ask("UserPromptSubmit"), deadline)
                .unwrap_err()
                .0,
            "memory_unavailable"
        );
    }
}
