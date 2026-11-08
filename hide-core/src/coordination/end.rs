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
//! tab it empties leaves the screen as one the engine closed. The
//! conversation is archived before the close and taken back when the pane
//! is still there after a close that failed.

use super::executor::{RecordedPane, close_pane, context, mutate, recorded_pane, registrations};
use super::{AgentRecord, Mutation, agent_view};
use crate::delivery::answer::{AgentEnd, ConversationEnd, PaneEnd};
use crate::delivery::{
    Actor,
    worker::{Authority, Client},
};
use crate::runtime::session_archive::EndPlan;
use crate::session_archive::AdmitRefusal;
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
/// and keeps the conversation that ran there. The engine reads the pane
/// from its topology, and its close ends the pane on its node within
/// `END_ANSWER_WAIT`, which the CLI's wait for an end covers.
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
    let recorded = record.terminal_id.as_deref();
    let logged = |outcome: &str, reason: Option<&str>| {
        crate::diagnostic!(json!({
            "component": "coordination", "kind": "agent.end_pane", "agent_id": record.id,
            "machine": machine, "pane_id": pane, "outcome": outcome, "reason": reason,
        }));
    };
    let terminal = match recorded_pane(engine.as_ref(), recorded, pane) {
        Ok(RecordedPane::Gone) => {
            logged("gone", None);
            return Ok((PaneEnd::Gone, ConversationEnd::None));
        }
        Ok(RecordedPane::Replaced) => {
            logged("gone", Some("pane_replaced"));
            return Ok((PaneEnd::Gone, ConversationEnd::None));
        }
        Ok(terminal) => terminal,
        Err(error) => return Err(close_failed(record, &error.to_string())),
    };
    let runtime = client.runtime.upgrade().ok_or("delivery_unavailable")?;
    let lock = || runtime.lock().map_err(|_| "delivery_unavailable");
    let plan = lock()?.end_pane_plan(&record.actor.pane_id, record.actor.session.as_deref());
    let keep = match plan {
        EndPlan::OtherAgent => {
            logged("left", Some("other_conversation"));
            return Ok((PaneEnd::OtherAgent, ConversationEnd::None));
        }
        // A registration from before terminals were recorded owns its pane
        // only while the pane shows its conversation.
        EndPlan::Shell if terminal == RecordedPane::Unrecorded => {
            logged("left", Some("pane_identity_unrecorded"));
            return Ok((PaneEnd::Left, ConversationEnd::None));
        }
        EndPlan::Shell => Err("no_agent"),
        EndPlan::Own(keep) => keep.map_err(|why| why.code()),
    };
    // Kept before the close (`Runtime::archive_ended`).
    let kept = match keep {
        Err(reason) => Err((ConversationEnd::None, reason)),
        Ok(ended) => lock()?.archive_ended(*ended).map_err(|refusal| {
            let (conversation, reason, fault) = match refusal {
                AdmitRefusal::Full => (ConversationEnd::ArchiveFull, "archive_full", None),
                AdmitRefusal::Fault(fault) => {
                    (ConversationEnd::None, "archive_refused", Some(fault))
                }
            };
            crate::diagnostic!(json!({
                "component": "coordination", "kind": "agent.end_not_archived",
                "agent_id": record.id, "pane_id": pane, "reason": reason, "fault": fault,
            }));
            (conversation, reason)
        }),
    };
    if let Err(detail) = close_pane(engine.as_ref(), pane) {
        match recorded_pane(engine.as_ref(), recorded, pane) {
            // The close failed after the pane left (its release): it closed.
            Ok(RecordedPane::Gone | RecordedPane::Replaced) => crate::diagnostic!(json!({
                "component": "coordination", "kind": "agent.end_pane_failed",
                "agent_id": record.id, "machine": machine, "pane_id": pane,
                "detail": detail, "pane_left": true,
            })),
            // The pane runs on with its conversation, which is not archived.
            Ok(RecordedPane::Same | RecordedPane::Unrecorded) => {
                if let Ok(Some(id)) = &kept {
                    lock()?.withdraw_ended(id);
                }
                return Err(close_failed(record, &detail));
            }
            // Unknown: a kept record waits for the pane to leave.
            Err(_) => return Err(close_failed(record, &detail)),
        }
    }
    let (conversation, reason) = match kept {
        Ok(_) => (ConversationEnd::Archived, None),
        Err((conversation, reason)) => (conversation, Some(reason)),
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
