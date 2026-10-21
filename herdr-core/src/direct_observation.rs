//! Proof of passive Herdr input-submission observations. The kernel-bound
//! caller supplies metadata; the existing server and lifecycle facts fence
//! its endpoint claims. No model receipt, letter or responsibility is made.

use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use hide_agent_hooks::observation::{
    DirectObservation, ObservationCorrelation, ObservationReason, ObservationStatus,
    ServerReportedTarget,
};
use sha2::{Digest, Sha256};

use crate::delivery::Actor;
use crate::links::observation::{DirectFact, DirectOutcome, DirectReason};
use crate::model::GraphParty;
use crate::node_access::NodeLink;
use crate::runtime::Runtime;

pub const ADMISSION_TTL_MS: u64 = 60_000;

pub fn valid(observation: &DirectObservation) -> bool {
    let key = |value: &str| {
        !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
    };
    hide_agent_adapter::ADAPTERS
        .iter()
        .any(|adapter| adapter.id == observation.runtime_id)
        && key(&observation.session_id)
        && key(&observation.call_id)
        && observation.target.as_ref().is_none_or(|target| {
            key(&target.pane_id)
                && target.terminal_id.as_deref().is_none_or(key)
                && target.native_session.as_ref().is_none_or(|native| {
                    key(&native.source)
                        && key(&native.agent)
                        && key(&native.value)
                        && matches!(native.kind.as_str(), "id" | "path")
                })
        })
        && serde_json::to_vec(observation)
            .is_ok_and(|bytes| bytes.len() <= hide_agent_hooks::observation::ITEM_LIMIT)
}

pub(crate) struct Endpoint {
    pub actor: Actor,
    pub raw_pane: String,
    pub terminal: String,
    pub observed_since: u64,
    pub reference: Option<hide_session::session_activity::SessionActivityRequest>,
}

/// Owned facts taken under a short Runtime lock, executed on the admitted
/// bootstrap worker. Its absolute deadline is not renewed by an RPC.
pub struct PreparedObservation {
    pub(crate) observation: DirectObservation,
    pub(crate) sender: Endpoint,
    pub(crate) recipient: Option<Endpoint>,
    /// The call started an agent: its proof records an origin, not an input.
    pub(crate) start: bool,
    /// The session Hide already held in a started pane before the call; that
    /// agent was there before, so it is not the one the call started.
    pub(crate) seen_before: Option<String>,
    pub(crate) connector: Arc<dyn hide_herdr_client::ApiConnector>,
    pub(crate) node: Arc<dyn NodeLink>,
    pub(crate) source_server: String,
    pub(crate) runtime: Weak<Mutex<Runtime>>,
    pub(crate) deadline: Instant,
    pub(crate) context: crate::workspace_control::Context,
}

impl PreparedObservation {
    pub fn run(self) -> Result<(), String> {
        self.prove_sender()?;
        if self.start {
            return self.record_start();
        }
        let recipient = self.recipient.as_ref().filter(|endpoint| {
            self.observation.target.as_ref().is_some_and(|claim| {
                match self.prove_recipient(endpoint, claim) {
                    Ok(()) => true,
                    Err(reason) => {
                        crate::diagnostic!(serde_json::json!({"component":"observation",
                            "kind":"observation.recipient_unproven","reason":reason,
                            "device":self.sender.actor.device_id,"pane":self.sender.actor.pane_id}));
                        false
                    }
                }
            })
        });
        if self.observation.runtime_id == "antigravity"
            && self.observation.correlation_kind == ObservationCorrelation::ConversationStep
        {
            // The native transcript's GENERIC result names no tool/call id.
            // The existing conversation reader projects only sightings from
            // it; a nearby step cannot prove which tool produced the result.
            crate::diagnostic!(serde_json::json!({"component":"observation",
                "kind":"observation.result_unavailable",
                "reason":"antigravity_result_correlation_unavailable",
                "device":self.sender.actor.device_id,"pane":self.sender.actor.pane_id}));
        }
        let (outcome, reason) = match self.observation.status {
            ObservationStatus::SubmittedObserved if recipient.is_some() => {
                (DirectOutcome::Submitted, None)
            }
            _ if self.observation.reason == ObservationReason::Interrupted => (
                DirectOutcome::Interrupted,
                Some(DirectReason::TransportFailed),
            ),
            ObservationStatus::RejectedObserved => {
                (DirectOutcome::Failed, Some(DirectReason::TransportFailed))
            }
            _ => (
                DirectOutcome::Unknown,
                Some(match self.observation.reason {
                    ObservationReason::Interrupted => DirectReason::TransportFailed,
                    ObservationReason::CommandAmbiguous => DirectReason::AmbiguousCommand,
                    ObservationReason::ResultMissing | ObservationReason::ResultTruncated => {
                        DirectReason::ResultMissing
                    }
                    _ => DirectReason::RecipientUnproven,
                }),
            ),
        };
        let fact = DirectFact {
            source_server: self.source_server.clone(),
            // Native correlation keys are never persisted or exposed verbatim.
            call_key: hash(
                &serde_json::json!([
                    self.observation.runtime_id,
                    self.observation.session_id,
                    self.observation.call_id,
                    self.observation.correlation_kind,
                ])
                .to_string(),
            ),
            sender: party(&self.sender.actor)?,
            recipient: recipient
                .map(|endpoint| party(&endpoint.actor))
                .transpose()?,
            at_unix_ms: self.observation.observed_at_unix_ms,
            outcome,
            reason,
        };
        self.remaining()?;
        let runtime = self.runtime.upgrade().ok_or("observation_unavailable")?;
        let guard = runtime.try_lock().map_err(|_| "observation_busy")?;
        guard.commit_direct_observation(&self, fact)
    }

    fn remaining(&self) -> Result<std::time::Duration, String> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| "observation_expired".into())
    }

    fn fresh(
        &self,
        endpoint: &Endpoint,
    ) -> Result<
        (
            crate::wire::DeliveryAgent,
            Option<hide_agent_hooks::observation::ObservationNativeSession>,
        ),
        String,
    > {
        self.remaining()?;
        let value = hide_herdr_client::request_small_response_strict_until(
            self.connector.as_ref(),
            "agent.get",
            crate::wire::agent_target_params(&endpoint.raw_pane)?,
            self.deadline,
        )
        .map_err(|_| "observation_native_unavailable")?;
        let (fresh, native) = crate::wire::direct_observation_agent(value)?;
        if fresh.pane_id != endpoint.raw_pane
            || fresh.terminal_id != endpoint.terminal
            || fresh.session.is_none()
            || fresh.session != endpoint.actor.session
            || fresh
                .kind
                .as_deref()
                .map(hide_agent_adapter::canonical_kind)
                != Some(hide_agent_adapter::canonical_kind(&endpoint.actor.kind))
        {
            return Err("observation_native_changed".into());
        }
        Ok((fresh, native))
    }

    pub(crate) fn prove_sender(&self) -> Result<(), String> {
        let (_, native) = self.fresh(&self.sender)?;
        let native = native.ok_or("observation_native_unavailable")?;
        if hide_agent_adapter::canonical_kind(&native.agent)
            != hide_agent_adapter::canonical_kind(&self.sender.actor.kind)
        {
            return Err("observation_sender_session_changed".into());
        }
        // Native identity is a server fact, independent of whether this
        // runtime stores a transcript file (OpenCode uses a database).
        if native.kind == "id" {
            return if native.value == self.observation.session_id {
                Ok(())
            } else {
                Err("observation_sender_session_changed".into())
            };
        }
        let reference = self
            .sender
            .reference
            .as_ref()
            .ok_or("observation_native_unavailable")?;
        if native.kind != "path"
            || reference.reference_kind != native.kind
            || reference.reference_value != native.value
        {
            return Err("observation_sender_session_changed".into());
        }
        {
            // A file/path is not a native session id. The existing bounded
            // native reader must independently prove the event's exact id.
            let mut request = reference.clone();
            request.expected_id = Some(self.observation.session_id.clone());
            let _: hide_session::session_activity::SessionActivity = crate::node_access::call_as(
                self.node.as_ref(),
                hide_node_link::protocol::Call::SessionActivity { request },
                self.remaining()?,
            )
            .map_err(|_| "observation_sender_session_unproven")?;
        }
        Ok(())
    }

    /// The server still answers the recipient as projected, and, when Herdr
    /// reported it, as it reported it then.
    pub(crate) fn prove_recipient(
        &self,
        endpoint: &Endpoint,
        claim: &ServerReportedTarget,
    ) -> Result<(), String> {
        let (_, fresh) = self.fresh(endpoint)?;
        if claim.native_session.as_ref().is_some_and(|native| {
            fresh.as_ref() != Some(native) || claim.pane_id != endpoint.raw_pane
        }) || claim
            .terminal_id
            .as_deref()
            .is_some_and(|terminal| terminal != endpoint.terminal)
        {
            return Err("observation_recipient_changed".into());
        }
        Ok(())
    }

    /// Records the agent the call started, with the sender as its origin
    /// (PRD direct-prompt-observation-widen D-23). Herdr's current list
    /// names it: by the pane, terminal and session Herdr reported, else by
    /// the pane the command wrote, holding an agent Hide had not seen there
    /// by the observation time. No session yet records nothing.
    fn record_start(&self) -> Result<(), String> {
        let claim = self
            .observation
            .target
            .as_ref()
            .ok_or("observation_recipient_unproven")?;
        let digest = |native: &hide_agent_hooks::observation::ObservationNativeSession| {
            crate::wire::session_digest(&native.value)
        };
        let started = |agent: &crate::session_sync::ProjectedAgent| {
            let named = match &claim.terminal_id {
                Some(terminal) => agent.pane_id == claim.pane_id && &agent.terminal_id == terminal,
                None => agent.pane_id == claim.pane_id,
            };
            named
                && agent.agent_session.is_some()
                && agent.lineage_session.is_some()
                && claim
                    .native_session
                    .as_ref()
                    .is_none_or(|native| agent.lineage_session == digest(native))
                && (self.seen_before.is_none() || agent.lineage_session != self.seen_before)
        };
        let result = crate::coordination::link_started(
            &self.runtime,
            &self.sender.actor,
            &self.sender.raw_pane,
            started,
            self.remaining()?,
        );
        match result {
            Ok(agent) => {
                crate::diagnostic!(serde_json::json!({"component":"observation",
                    "kind":"observation.start_recorded","agent_id":agent,
                    "device":self.sender.actor.device_id,"pane":self.sender.actor.pane_id}));
                Ok(())
            }
            Err(reason) => {
                crate::diagnostic!(serde_json::json!({"component":"observation",
                    "kind":"observation.start_unrecorded","reason":reason,
                    "device":self.sender.actor.device_id,"pane":self.sender.actor.pane_id}));
                Err(reason)
            }
        }
    }
}

pub(crate) fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn party(actor: &Actor) -> Result<GraphParty, String> {
    Ok(GraphParty {
        device_id: actor.device_id.clone(),
        pane_id: actor.pane_id.clone(),
        session: actor
            .session
            .clone()
            .ok_or("observation_native_unavailable")?,
    })
}
