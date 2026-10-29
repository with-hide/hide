//! Bounded hook reads, shared by local and node proof exchanges.
//! Authentication belongs to the proof owners; execution remains the same
//! core operations the scoped Workspace route uses.

use std::time::{Duration, Instant};

use hide_node_link::panes::{HOOK_REQUEST_LIMIT, HOOK_RESPONSE_LIMIT, HookCommand, HookRequest};
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
            HookCommand::WorkspaceInfo {} => true,
            HookCommand::Observation { observation }
            | HookCommand::StartObservation { observation } => {
                herdr_core::direct_observation::valid(observation)
            }
            HookCommand::Memory {
                event,
                runtime,
                session,
                prompt,
            } => memory_ask(event, runtime, session, prompt).is_some(),
            HookCommand::LetterPull {
                prompt_digest,
                session,
                point,
            } => {
                prompt_digest.as_ref().is_none_or(|digest| {
                    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
                }) && session
                    .as_ref()
                    .is_none_or(|session| herdr_core::delivery::valid_session(session))
                    // A tool event's ask names its session.
                    && (*point == hide_node_link::panes::PullPoint::Prompt || session.is_some())
            }
        }
}

pub(crate) fn answer(
    core: &CoreHandle,
    cap: &Capability,
    request: &HookRequest,
    desktop: bool,
) -> Value {
    let result = match &request.command {
        HookCommand::WorkspaceInfo {} => workspace_info(core, cap, desktop),
        HookCommand::Observation { observation } => {
            observation_answer(core, cap, observation, false)
        }
        HookCommand::StartObservation { observation } => {
            observation_answer(core, cap, observation, true)
        }
        HookCommand::LetterPull {
            prompt_digest,
            session,
            point,
        } => delivery(
            core,
            cap,
            herdr_core::delivery::Command::Pull {
                prompt_digest: prompt_digest.clone(),
                session: session.clone(),
                point: *point,
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

fn observation_answer(
    core: &CoreHandle,
    cap: &Capability,
    observation: &hide_node_link::panes::DirectObservation,
    start: bool,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + hide_node_link::panes::OBSERVATION_SEND_BUDGET;
    let result = cap
        .pane_terminal()
        .ok_or("observation_agent_pane_required".to_owned())
        .and_then(|terminal| {
            core.prepare_direct_observation(
                &cap.context.device_id,
                &cap.pane_id,
                &cap.context,
                terminal.to_owned(),
                observation.clone(),
                start,
                deadline,
            )
        })
        .and_then(|prepared| prepared.run());
    result.map(|()| json!({"queued":true})).map_err(|reason| {
        herdr_core::diagnostic!(
            json!({"component":"observation","kind":"observation.refused",
            "pane":cap.pane_id,"device":cap.context.device_id,"reason":reason})
        );
        (reason, "Continue without the passive observation")
    })
}

fn workspace_info(core: &CoreHandle, cap: &Capability, desktop: bool) -> Result<Value, Failure> {
    let mut result = core
        .workspace_query(
            &cap.context.device_id,
            &cap.pane_id,
            herdr_core::workspace_control::Query::Info,
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
) -> Option<herdr_core::memory_hook::Ask> {
    herdr_core::memory_hook::Ask::from_wire(
        &json!({"event":event,"runtime":runtime,"session":session,"prompt":prompt}),
    )
}

pub(crate) fn delivery(
    core: &CoreHandle,
    cap: &Capability,
    command: herdr_core::delivery::Command,
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
    ask: &herdr_core::memory_hook::Ask,
) -> Result<Value, Failure> {
    let deadline = Instant::now() + herdr_core::memory_hook::WITHIN;
    let scope = core.memory_scope(&cap.context, deadline).map_err(|reason| {
        herdr_core::diagnostic!(json!({"component":"memory","kind":"hook.unavailable","pane":cap.pane_id,"device":cap.context.device_id,"reason":reason}));
    });
    memory_answer(core.state_dir(), scope, ask, deadline)
}

/// A hook's answer: Memory's context when its scope was found, and on a
/// session start the Hide global instruction whatever Memory answers (off,
/// unresolved or unreachable; PRD memory-self-improve D-31). The global read
/// is one small bounded file, outside Memory's deadline.
fn memory_answer(
    state_dir: &std::path::Path,
    scope: Result<herdr_core::memory_hook::Scope, ()>,
    ask: &herdr_core::memory_hook::Ask,
    deadline: Instant,
) -> Result<Value, Failure> {
    let global = ask
        .starts_session()
        .then(|| herdr_core::global_instruction::for_session(state_dir))
        .flatten();
    let answer = match (scope, global) {
        (Ok(scope), global) => herdr_core::memory_hook::HookContext {
            global,
            ..herdr_core::memory_hook::answer(&scope, ask, deadline)
        },
        (Err(()), Some(global)) => herdr_core::memory_hook::global_only(global),
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

    fn ask(event: &str) -> herdr_core::memory_hook::Ask {
        memory_ask(event, "claude", "s-1", "").unwrap()
    }

    /// B1, D-31: a session start with no Memory still gets the global
    /// instruction; a prompt, or an empty file, keeps the old refusal.
    #[test]
    fn a_session_start_gets_the_global_instruction_without_memory() {
        let state = tempfile::tempdir().unwrap();
        let deadline = Instant::now() + herdr_core::memory_hook::WITHIN;
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
