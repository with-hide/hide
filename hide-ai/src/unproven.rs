//! The agents Hide AI can name but cannot yet use, and why.
//!
//! A background call may read a transcript and answer; it must never change a
//! file or run a command (D-15, B35). A provider that runs tools in print
//! mode with no switch that turns them off, and OpenCode's default agent has
//! every tool; neither documents a flag or an environment variable that makes
//! the call read-only. Until one is proven against the real CLI the backend is
//! `Unsupported` and the shell shows the agent dimmed under "Hide AI can't use
//! these yet" with this reason (B38). Flipping an agent over is writing its
//! real backend, not removing this one's refusal.

use crate::{
    AiBackend, AiError, AiRequest, AiResponse, Availability, CancelToken, ModelCatalog, ProviderId,
};

/// The reason both agents report. One code, so the shell has one sentence for
/// it; the per-agent cause is in `AI_PROVIDERS.md`.
pub const CANNOT_GUARANTEE_READ_ONLY: &str = "cannot_guarantee_read_only";

pub struct UnprovenReadOnlyBackend {
    provider: ProviderId,
}

impl UnprovenReadOnlyBackend {
    pub fn new(provider: ProviderId) -> Self {
        Self { provider }
    }
}

impl AiBackend for UnprovenReadOnlyBackend {
    fn id(&self) -> ProviderId {
        self.provider
    }

    /// Always unsupported. Whether the agent's program is on this machine is
    /// the install kit's answer, which the core puts on the row (`ai::project`),
    /// so there is no second program table or search here.
    fn availability(&self) -> Availability {
        Availability::Unsupported {
            reason: CANNOT_GUARANTEE_READ_ONLY.to_owned(),
        }
    }

    fn models(&self) -> ModelCatalog {
        ModelCatalog::Unknown {
            reason: CANNOT_GUARANTEE_READ_ONLY.to_owned(),
        }
    }

    fn execute(&self, _: &AiRequest, _: &CancelToken) -> Result<AiResponse, AiError> {
        Err(AiError::Unsupported(CANNOT_GUARANTEE_READ_ONLY.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RequestId;
    use std::time::Duration;

    #[test]
    fn a_request_to_an_unproven_agent_is_refused_before_anything_runs() {
        let backend = UnprovenReadOnlyBackend::new(ProviderId::OPENCODE);
        let request = AiRequest {
            feature_id: "test".into(),
            request_id: RequestId("r".to_owned()),
            subject_id: "s".to_owned(),
            system: String::new(),
            input: "transcript".to_owned(),
            output_schema: serde_json::json!({}),
            deadline: Duration::from_secs(1),
            schema_version: "v1".into(),
            pick: None,
            output_language: None,
            answer: crate::AnswerMode::default(),
            provider_efforts: Vec::new(),
        };
        assert_eq!(
            backend.execute(&request, &CancelToken::new()).unwrap_err(),
            AiError::Unsupported("cannot_guarantee_read_only".to_owned())
        );
        assert_eq!(backend.id(), ProviderId::OPENCODE);
    }
}
