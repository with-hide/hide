//! Bounded hook reads, shared by local and node proof exchanges.
//! Authentication belongs to the proof owners; execution remains the same
//! core operations the scoped Workspace route uses.

use std::path::Path;
use std::time::{Duration, Instant};

use hide_agent_hooks::outbox;

use hide_node_link::panes::{
    HOOK_REQUEST_LIMIT, HOOK_RESPONSE_LIMIT, HookCommand, HookRequest, ProofAnswer, QueuedReport,
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
            HookCommand::WorkspaceInfo {} => true,
            HookCommand::Memory {
                event,
                runtime,
                session,
                prompt,
            } => memory_ask(event, runtime, session, prompt).is_some(),
            HookCommand::Status { report } => valid_status(report),
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
        HookCommand::Status { report } => status(core, cap, report),
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
/// Applies the status reports a machine's hooks queued while its core did
/// not answer them (`hide_agent_hooks::outbox`), in their order: each one
/// the hook made after the newest report its pane applied changes what the
/// core keeps, as the hook's own report would have (`Core::hook_replay`),
/// and one naming a pane of another device than `device` is refused there.
/// The counts, with what the outbox dropped at its limit, go to the
/// diagnostic log; nothing reaches the screen (design principle 13). A core
/// that stopped answering ends the replay with `hook_status_unavailable`, and
/// the batch stays with its outbox.
pub(crate) fn replay(
    core: &CoreHandle,
    device: &str,
    reports: &[QueuedReport],
    dropped: u64,
) -> Result<ProofAnswer, &'static str> {
    let (mut applied, mut ignored, mut refusal) = (0, 0, None);
    for queued in reports {
        let pane = &queued.pane_id;
        let valid = valid_status(&queued.report)
            && !pane.is_empty()
            && pane.len() <= 256
            && !pane.chars().any(char::is_control);
        match valid.then(|| core.hook_replay(device, pane, queued.report.clone())) {
            Some(Ok(true)) => applied += 1,
            Some(Ok(false)) => ignored += 1,
            Some(Err(UNAVAILABLE)) => return Err(UNAVAILABLE),
            Some(Err(reason)) => {
                ignored += 1;
                refusal = Some(reason);
            }
            None => {
                ignored += 1;
                refusal = Some("invalid_request");
            }
        }
    }
    hide_core::diagnostic!(json!({
        "component": "agent_hooks",
        "kind": "outbox.replayed",
        "device": device,
        "applied": applied,
        "ignored": ignored,
        "dropped": dropped,
        "refusal": refusal,
    }));
    Ok(ProofAnswer::Replayed { applied, ignored })
}

/// What `CoreHandle` answers once the core no longer takes reports.
const UNAVAILABLE: &str = "hook_status_unavailable";

/// Applies what this machine's hooks queued while this daemon did not
/// answer them (a restart, an update), oldest first, the way a node's
/// replay is applied; the bootstrap's sweep calls it, one at a time
/// (`pane_auth::serve`), and [`outbox::drain`] keeps any other drain of the
/// same outbox out while it runs.
pub(crate) fn replay_own(state_dir: &Path, core: &CoreHandle) {
    if !outbox::waiting(state_dir) {
        return;
    }
    let device = core.node().as_str().to_owned();
    let drained = outbox::drain(state_dir, crate::pane_auth::own_install(), |batch| {
        if batch.damaged {
            hide_core::diagnostic!(json!({
                "component": "agent_hooks", "kind": "outbox.damaged", "device": device,
            }));
        }
        replay(core, &device, &batch.reports, batch.dropped).is_ok()
    });
    if let Err(error) = drained {
        hide_core::diagnostic!(json!({
            "component": "agent_hooks", "kind": "outbox.unavailable", "device": device,
            "error": error.kind().to_string(),
        }));
    }
}

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
