//! `hide agent end <id> [--keep-pane]`: ends a registration and, unless the
//! caller keeps the pane, closes the pane its agent ran in.
//!
//! Closing the pane ends its holder, which ends the shell, the agent and
//! everything under them (`hide_platform::process::Session`), so an ended
//! agent leaves no process and no holder behind (with-hide/hide#2181). The
//! registration ends first, as it always did, so a close that fails leaves
//! the same state the end alone left before, and the same command closes it
//! again. The pane is closed only while it is still the registration's: it
//! runs the terminal the registration recorded and shows no other agent's
//! conversation. The agent's conversation is kept in the session archive,
//! where 이어가기 resumes it (`Runtime::archive_ended`).
//!
//! The close is the engine's own, on the agent's machine, as a spawn closes
//! the panes of its unstarted attempts: it enters no Reopen Closed, and a
//! tab it empties leaves the screen as one the engine closed.

use super::executor::{context, mutate, registrations};
use super::{AgentRecord, Mutation, agent_view};
use crate::delivery::answer::{AgentEnd, ConversationEnd, PaneEnd};
use crate::delivery::{
    Actor,
    worker::{Authority, Client},
};
use crate::engine::{EngineError, Wait};
use crate::runtime::session_archive::EndPlan;
use serde_json::{Value, json};
use std::time::Duration;

/// Each engine call of the close; the CLI waits 10 s for the whole command.
const ENGINE_WAIT: Duration = Duration::from_secs(3);

/// Ends registration `id` for `actor`, and closes its pane unless
/// `keep_pane`.
pub(super) fn end(
    client: &Client,
    authority: &Authority,
    actor: &Actor,
    id: String,
    asserted: Option<String>,
    keep_pane: bool,
) -> Result<Value, String> {
    mutate(
        client,
        authority,
        actor,
        Mutation::End {
            id: id.clone(),
            actor: asserted,
        },
    )?;
    let (ledger, sleeping) = registrations(client)?;
    let record = ledger
        .agents
        .iter()
        .find(|record| record.id == id)
        .ok_or("agent_unavailable")?;
    let (pane, conversation) = if keep_pane {
        (PaneEnd::Kept, ConversationEnd::None)
    } else {
        close(client, record)?
    };
    Ok(json!(AgentEnd {
        agent: agent_view(record, &ledger, &sleeping),
        pane,
        conversation,
    }))
}

/// Closes the pane `record` ran in while it is still the registration's,
/// and keeps the conversation that ran there.
fn close(client: &Client, record: &AgentRecord) -> Result<(PaneEnd, ConversationEnd), String> {
    let machine = record.actor.device_id.as_str();
    let on_device = crate::address::pane_device(&record.actor.pane_id).is_some();
    let engine = context(client, machine)
        .map_err(|reason| {
            if on_device {
                "machine_unavailable".to_owned()
            } else {
                reason
            }
        })?
        .engine;
    let pane = record.pane.as_str();
    let logged = |outcome: &str, reason: Option<&str>| {
        crate::diagnostic!(json!({
            "component": "coordination", "kind": "agent.end_pane", "agent_id": record.id,
            "machine": machine, "pane_id": pane, "outcome": outcome, "reason": reason,
        }));
    };
    let terminal_recorded = match engine.pane_terminal(Wait::Each(ENGINE_WAIT), pane) {
        Err(error) if error.code() == Some("pane_not_found") => {
            logged("gone", None);
            return Ok((PaneEnd::Gone, ConversationEnd::None));
        }
        Err(error) => return Err(close_failed(record, &error)),
        // The pane id now names another terminal: the agent's pane is gone.
        Ok(terminal)
            if record
                .terminal_id
                .as_ref()
                .is_some_and(|at| *at != terminal) =>
        {
            logged("gone", Some("pane_replaced"));
            return Ok((PaneEnd::Gone, ConversationEnd::None));
        }
        Ok(_) => record.terminal_id.is_some(),
    };
    let runtime = client.runtime.upgrade().ok_or("delivery_unavailable")?;
    let plan = runtime
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .end_pane_plan(&record.actor.pane_id, record.actor.session.as_deref());
    let keep = match plan {
        EndPlan::OtherAgent => {
            logged("left", Some("other_conversation"));
            return Ok((PaneEnd::OtherAgent, ConversationEnd::None));
        }
        // A registration from before terminals were recorded owns its pane
        // only while the pane shows its conversation.
        EndPlan::Close { keep: None } if !terminal_recorded => {
            logged("gone", Some("pane_identity_unrecorded"));
            return Ok((PaneEnd::Gone, ConversationEnd::None));
        }
        EndPlan::Close { keep } => keep,
    };
    engine
        .close_pane(Wait::Each(ENGINE_WAIT), pane)
        .map_err(|error| close_failed(record, &error))?;
    let conversation = match keep {
        None => ConversationEnd::None,
        Some(ended) => match runtime
            .lock()
            .map_err(|_| "delivery_unavailable")?
            .archive_ended(ended)
        {
            Ok(()) => ConversationEnd::Archived,
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "coordination", "kind": "agent.end_not_archived",
                    "agent_id": record.id, "pane_id": pane, "reason": reason,
                }));
                ConversationEnd::ArchiveFull
            }
        },
    };
    logged("closed", None);
    Ok((PaneEnd::Closed, conversation))
}

/// The refusal for a close the engine did not take: the registration has
/// ended, and the same command closes the pane once the engine answers.
fn close_failed(record: &AgentRecord, error: &EngineError) -> String {
    crate::diagnostic!(json!({
        "component": "coordination", "kind": "agent.end_pane_failed",
        "agent_id": record.id, "machine": record.actor.device_id, "pane_id": record.pane,
        "detail": error.to_string(),
    }));
    "pane_close_failed".to_owned()
}
