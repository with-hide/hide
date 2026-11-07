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

use super::executor::{RecordedPane, close_pane, context, mutate, recorded_pane, registrations};
use super::{AgentRecord, Mutation, agent_view};
use crate::delivery::answer::{AgentEnd, ConversationEnd, PaneEnd};
use crate::delivery::{
    Actor,
    worker::{Authority, Client},
};
use crate::runtime::session_archive::EndPlan;
use serde_json::{Value, json};

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
/// and keeps the conversation that ran there. Each engine call waits at
/// most `PANE_WAIT`, which the CLI's wait for an end covers.
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
    let gone = |reason: Option<&str>| {
        logged("gone", reason);
        Ok((PaneEnd::Gone, ConversationEnd::None))
    };
    let terminal = match recorded_pane(engine.as_ref(), record.terminal_id.as_deref(), pane) {
        Ok(RecordedPane::Gone) => return gone(None),
        Ok(RecordedPane::Replaced) => return gone(Some("pane_replaced")),
        Ok(terminal) => terminal,
        Err(error) => return Err(close_failed(record, &error.to_string())),
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
        EndPlan::Shell if terminal == RecordedPane::Unrecorded => {
            return gone(Some("pane_identity_unrecorded"));
        }
        EndPlan::Shell => Err("no_agent"),
        EndPlan::Own(keep) => keep.map_err(|why| why.code()),
    };
    close_pane(engine.as_ref(), pane).map_err(|detail| close_failed(record, &detail))?;
    let (conversation, reason) = match keep {
        Err(reason) => (ConversationEnd::None, Some(reason)),
        Ok(ended) => match runtime
            .lock()
            .map_err(|_| "delivery_unavailable")?
            .archive_ended(*ended)
        {
            Ok(()) => (ConversationEnd::Archived, None),
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "coordination", "kind": "agent.end_not_archived",
                    "agent_id": record.id, "pane_id": pane, "reason": reason,
                }));
                (ConversationEnd::ArchiveFull, Some("archive_full"))
            }
        },
    };
    logged("closed", reason);
    Ok((PaneEnd::Closed, conversation))
}

/// The refusal for a close the engine did not take: the registration has
/// ended, and the same command closes the pane once the engine answers.
fn close_failed(record: &AgentRecord, detail: &str) -> String {
    crate::diagnostic!(json!({
        "component": "coordination", "kind": "agent.end_pane_failed",
        "agent_id": record.id, "machine": record.actor.device_id, "pane_id": record.pane,
        "detail": detail,
    }));
    "pane_close_failed".to_owned()
}
