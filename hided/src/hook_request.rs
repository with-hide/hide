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
            HookCommand::Observation { observation } => {
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
        HookCommand::Observation { observation } => observation_answer(core, cap, observation),
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
        ),
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
        ("memory_unavailable".to_owned(), "Continue without Project Memory")
    })?;
    serde_json::to_value(herdr_core::memory_hook::answer(&scope, ask, deadline)).map_err(|_| {
        (
            "result_encoding_failed".to_owned(),
            "Continue without Project Memory",
        )
    })
}
