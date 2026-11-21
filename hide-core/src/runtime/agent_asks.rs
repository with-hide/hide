//! What an agent's own open questions take from the runtime
//! (docs/status-model.md, A question still waiting): the pane's own session,
//! named under the lock as `hide task` names it; the record's read, the
//! fold and the receipts' write run off it (`links::agent_asks`).

use std::collections::BTreeMap;

use serde_json::json;

use super::Runtime;
use crate::labels::worker::SightedMemoryReceipt;
use crate::links::agent_asks::{FoldScope, GiveScope, receipt_ids};
use crate::links::ledger::{LedgerUpdate, SessionIdentity};
use crate::workspace_control::{Caller, Context, Query};

impl Runtime {
    /// The session of the agent in the pane `caller` names on `device`, as
    /// its plan is keyed (`caller_session`).
    fn own_ask_session(&self, device: &str, caller: &str) -> Result<SessionIdentity, &'static str> {
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("agent_pane_required");
        };
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
        let actor = &self
            .delivery_observations
            .get(&pane)
            .ok_or("agent_pane_required")?
            .actor;
        self.caller_session(device, &pane, actor)
            .ok_or("ask_session_unknown")
    }

    /// What a prompt hook in `caller`'s pane reads its session's open
    /// questions from, when `session_id`, the session its hook names, is the
    /// pane's own; none while Hide AI's summaries are off, since nothing
    /// keeps the list then.
    pub(crate) fn asks_give_scope(
        &self,
        device: &str,
        caller: &str,
        session_id: &str,
    ) -> Option<GiveScope> {
        if !super::backlog::summaries_on(self.ai_settings.as_ref()) {
            return None;
        }
        // A prompt of an agent pane whose session Hide cannot name, or names
        // as another, gives nothing; the log says which.
        let skipped = |reason: &str, own: Option<&str>| {
            crate::diagnostic!(json!({
                "component": "session",
                "kind": "asks.give_skipped",
                "device": device,
                "pane_id": caller,
                "session_id": session_id,
                "own_session_id": own,
                "reason": reason,
            }));
        };
        let session = match self.own_ask_session(device, caller) {
            Ok(session) => session,
            Err(reason) => {
                skipped(reason, None);
                return None;
            }
        };
        if session.session_id != session_id {
            skipped("session_not_own", Some(&session.session_id));
            return None;
        }
        let directory = self
            .state_path
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())?;
        Some(GiveScope {
            store: hide_kit::layout::links_store(directory),
            session,
        })
    }

    /// What `hide session fold-ask` from `caller`'s pane writes through: its own
    /// session and the link worker, for a caller still in the checkout it
    /// was admitted to.
    pub(crate) fn fold_ask_scope(
        &self,
        device: &str,
        caller: &str,
        expected: &Context,
    ) -> Result<FoldScope, String> {
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let session = self.own_ask_session(device, caller)?;
        let client = self.links_work.client.clone().ok_or("ask_unavailable")?;
        Ok(FoldScope { client, session })
    }

    /// Takes the open questions' receipt lines out of `receipts`, leaving
    /// Memory's, and records the questions each session's agent was given
    /// through the link worker, whether or not `device` has Memory.
    pub(super) fn take_asks_receipts(
        &self,
        device: &str,
        receipts: &mut Vec<SightedMemoryReceipt>,
    ) {
        let mut given: BTreeMap<SessionIdentity, Vec<String>> = BTreeMap::new();
        receipts.retain(|receipt| {
            if !receipt
                .text
                .starts_with(hide_session::label_transcript::ASKS_RECEIPT_MARKER)
            {
                return true;
            }
            if let (Some(ids), Some(agent)) = (
                receipt_ids(&receipt.text),
                hide_session::Agent::from_kind(receipt.provider),
            ) {
                given
                    .entry(SessionIdentity {
                        device_id: device.to_owned(),
                        agent: agent.as_str().to_owned(),
                        session_id: receipt.session_id.clone(),
                    })
                    .or_default()
                    .extend(ids);
            }
            false
        });
        if given.is_empty() {
            return;
        }
        let Some(client) = self.links_work.client.as_ref() else {
            return;
        };
        let at_unix_ms = super::memory::now_ms();
        for (session, ids) in given {
            if let Err(code) = client.ledger(LedgerUpdate::AsksGiven {
                session: session.clone(),
                ids,
                at_unix_ms,
            }) {
                crate::diagnostic!(json!({
                    "component": "session",
                    "kind": "asks.receipt_unrecorded",
                    "session_id": session.session_id,
                    "code": code,
                }));
            }
        }
    }
}
