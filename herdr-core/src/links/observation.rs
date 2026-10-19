//! Passive Herdr input facts, admitted after caller and endpoint proof.
//! These are never delivery letters, native intake receipts or lineage.
use crate::model::{GraphDirectInput, GraphParty};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ITEM_BYTE_LIMIT: usize = 8 * 1024;
pub const EVENT_LIMIT: usize = 2_000;
pub const TOTAL_BYTE_LIMIT: usize = 16 * 1024 * 1024;
pub const RETENTION_MS: u64 = 30 * super::DAY_MS;
/// A worker never keeps an admitted write waiting indefinitely.
pub const PENDING_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
pub const SHUTDOWN_DRAIN: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectOutcome {
    Submitted,
    Unknown,
    Failed,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectReason {
    RecipientUnproven,
    ResultMissing,
    AmbiguousCommand,
    StaleSession,
    TransportFailed,
}

/// The admission owner supplies exact device/pane/native-session digests.
/// `recipient` requires independent endpoint proof; tool stdout alone is not proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectFact {
    /// Identifies the source Herdr server and named session, not its display name.
    pub source_server: String,
    pub call_key: String,
    pub sender: GraphParty,
    pub recipient: Option<GraphParty>,
    pub at_unix_ms: u64,
    pub outcome: DirectOutcome,
    pub reason: Option<DirectReason>,
}

impl DirectFact {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        let identifier = |value: &str| {
            !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
        };
        let party = |party: &GraphParty| {
            identifier(&party.device_id)
                && identifier(&party.pane_id)
                && party.session.len() == 64
                && party.session.bytes().all(|value| value.is_ascii_hexdigit())
        };
        if !identifier(&self.source_server)
            || !identifier(&self.call_key)
            || !party(&self.sender)
            || self.recipient.as_ref().is_some_and(|p| !party(p))
        {
            return Err("direct_observation_identity_invalid".into());
        }
        if self.at_unix_ms > now.saturating_add(60_000)
            || now.saturating_sub(self.at_unix_ms) >= RETENTION_MS
        {
            return Err("direct_observation_stale".into());
        }
        if self.encoded()?.len() > ITEM_BYTE_LIMIT {
            return Err("direct_observation_item_capacity".into());
        }
        Ok(())
    }

    pub(crate) fn encoded(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|_| "direct_observation_value_invalid".into())
    }

    /// Stable across duplicate terminal events and restarts, without exposing call keys.
    pub(crate) fn id(&self) -> String {
        let key = serde_json::json!([
            self.source_server,
            self.sender.device_id,
            self.sender.pane_id,
            self.sender.session,
            self.call_key
        ]);
        format!("{:x}", Sha256::digest(key.to_string().as_bytes()))
    }

    pub(crate) fn graph_input(&self) -> Option<GraphDirectInput> {
        if self.outcome != DirectOutcome::Submitted {
            return None;
        }
        Some(GraphDirectInput {
            id: self.id(),
            sender: self.sender.clone(),
            recipient: self.recipient.clone()?,
            at_unix_ms: self.at_unix_ms,
        })
    }
}

pub(super) const SCHEMA: &str = "
CREATE TABLE direct_observations(id TEXT PRIMARY KEY, at INTEGER NOT NULL,
    bytes INTEGER NOT NULL, fact TEXT NOT NULL);
CREATE INDEX direct_observations_at ON direct_observations(at);
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::links::{store::LinkStore, worker::LinkClient};

    fn fact(at: u64) -> DirectFact {
        DirectFact {
            source_server: "device:local/server:one/session:named".into(),
            call_key: "native-call-1".into(),
            sender: GraphParty {
                device_id: "local".into(),
                pane_id: "pane-1".into(),
                session: "a".repeat(64),
            },
            recipient: Some(GraphParty {
                device_id: "local".into(),
                pane_id: "pane-2".into(),
                session: "b".repeat(64),
            }),
            at_unix_ms: at,
            outcome: DirectOutcome::Submitted,
            reason: None,
        }
    }

    #[test]
    fn incomplete_results_never_become_model_or_delivery_receipts() {
        let mut input = fact(1_000_000);
        for outcome in [
            DirectOutcome::Unknown,
            DirectOutcome::Failed,
            DirectOutcome::TimedOut,
            DirectOutcome::Interrupted,
        ] {
            input.outcome = outcome;
            assert!(input.graph_input().is_none());
        }
        input.outcome = DirectOutcome::Submitted;
        input.recipient = None;
        assert!(input.graph_input().is_none());
        let json = serde_json::to_value(fact(1_000_000).graph_input().unwrap()).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 4);
        assert!(
            ["at_unix_ms", "id", "recipient", "sender"]
                .iter()
                .all(|key| json.get(key).is_some())
        );
        assert!(!json.to_string().contains("native-call"));
        assert!(!json.to_string().contains("source_server"));
    }

    #[test]
    fn duplicate_terminal_events_converge_across_restart_without_losing_uncertainty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("links.sqlite3");
        let (mut store, _) = LinkStore::open(&path).unwrap();
        let mut uncertain = fact(1_000_000);
        uncertain.recipient = None;
        uncertain.outcome = DirectOutcome::Unknown;
        uncertain.reason = Some(DirectReason::ResultMissing);
        assert!(
            store
                .apply_direct_observation(&uncertain, 1_000_000)
                .unwrap()
        );
        assert!(
            !store
                .apply_direct_observation(&uncertain, 1_000_000)
                .unwrap()
        );
        assert!(
            store.direct_observations(1_000_000).unwrap()[0]
                .graph_input()
                .is_none()
        );
        drop(store);
        let (mut store, _) = LinkStore::open(&path).unwrap();
        let proven = fact(1_000_001);
        assert!(store.apply_direct_observation(&proven, 1_000_001).unwrap());
        let recorded = store.direct_observations(1_000_001).unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].at_unix_ms, 1_000_000);
        assert!(recorded[0].graph_input().is_some());
        assert!(
            !store
                .apply_direct_observation(&uncertain, 1_000_001)
                .unwrap()
        );
        let mut conflict = proven;
        conflict.recipient.as_mut().unwrap().session = "c".repeat(64);
        assert_eq!(
            store.apply_direct_observation(&conflict, 1_000_001),
            Err("direct_observation_identity_conflict".into())
        );
        assert_eq!(store.direct_observations(1_000_001).unwrap(), recorded);
    }

    #[test]
    fn stale_items_and_shared_queue_overflow_are_refused() {
        let now = super::super::now_ms();
        let client = LinkClient::full_mailbox_for_test();
        assert_eq!(
            client.observe(fact(now)),
            Err("direct_observation_queue_capacity".into())
        );
        let client = LinkClient::open_mailbox_for_test();
        for index in 0..crate::links::ledger::LEDGER_QUEUE_LIMIT {
            let mut input = fact(now);
            input.call_key = format!("call-{index}");
            client.observe(input).unwrap();
        }
        assert_eq!(
            client.observe(fact(now)),
            Err("direct_observation_queue_capacity".into())
        );
        assert_eq!(
            fact(now.saturating_sub(RETENTION_MS)).validate(now),
            Err("direct_observation_stale".into())
        );
        assert!(
            fact(now.saturating_sub(RETENTION_MS - 1))
                .validate(now)
                .is_ok()
        );
        let mut oversized = fact(now);
        oversized.source_server = "\"".repeat(1024);
        oversized.call_key = "\"".repeat(1024);
        oversized.sender.device_id = "\"".repeat(1024);
        oversized.sender.pane_id = "\"".repeat(1024);
        assert_eq!(
            oversized.validate(now),
            Err("direct_observation_item_capacity".into())
        );
    }

    #[test]
    fn separate_incomplete_callbacks_never_combine_into_a_confirmed_submission() {
        let directory = tempfile::tempdir().unwrap();
        let (mut store, _) = LinkStore::open(&directory.path().join("links.sqlite3")).unwrap();
        let mut target_only = fact(1_000_000);
        target_only.outcome = DirectOutcome::Unknown;
        target_only.reason = Some(DirectReason::ResultMissing);
        store
            .apply_direct_observation(&target_only, 1_000_000)
            .unwrap();
        let mut submitted_only = fact(1_000_001);
        submitted_only.recipient = None;
        submitted_only.reason = Some(DirectReason::RecipientUnproven);
        store
            .apply_direct_observation(&submitted_only, 1_000_001)
            .unwrap();
        assert!(
            store.direct_observations(1_000_001).unwrap()[0]
                .graph_input()
                .is_none()
        );
        store
            .apply_direct_observation(&target_only, 1_000_001)
            .unwrap();
        assert!(
            store.direct_observations(1_000_001).unwrap()[0]
                .graph_input()
                .is_none()
        );
    }
}
