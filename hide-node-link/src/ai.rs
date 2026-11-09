//! The background AI as the core asks a node for it. The core keeps the
//! router (provider choice, retries, budgets, cooldowns); each of its
//! backends is a `BackendSpec` the node answers for with a real backend of
//! its own, so the provider processes run on the node's machine with its
//! logins. A backend's log events come back with each answer.

use serde::{Deserialize, Serialize};

pub use hide_ai::{
    AiError, AiLogEvent, AiRequest, AiResponse, Availability, ModelCatalog, ProcessMeasurement,
    ProviderId,
};

/// One backend of one core router. `instance` is the core's own number for
/// it: the node keeps one real backend per instance, so two routers never
/// share a resident provider process, and drops it when the core releases
/// the instance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackendSpec {
    pub instance: u64,
    pub provider: ProviderId,
    pub model: String,
    /// The node's profile of the provider's agent the backend runs as (PRD
    /// agent-profiles-phase2 D-09): the node resolves it from its own
    /// profile file as a start of it would run, its account's and its own
    /// variables and its wrapper, so the core never names an account's
    /// folder. `None` is the agent's built-in default profile, and is left
    /// off the wire as an older core wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<hide_agent_adapter::ProfileId>,
}

/// An answer and the log events the backend wrote since the last answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Logged<T> {
    pub value: T,
    pub log: Vec<AiLogEvent>,
}
