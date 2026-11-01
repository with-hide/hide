//! The session of an engine whose changes are pulled (PRD
//! engine-bundle-3-core-owns D-3): the core's own engine wakes the
//! coordinator once its topology changed, and the coordinator reads the
//! whole session again ([`crate::engine::Engine::session`]) and publishes
//! it as it is. [`PulledSession`] keeps only the last session read; nothing
//! is replayed, so none of the Herdr event replica (`replica.rs`) or its
//! subscription runs for such an engine, and deleting them with Herdr
//! leaves this path whole.

use super::coordinator::PublishSource;
use super::*;

/// The last session a pulled engine answered.
pub(super) struct PulledSession {
    state: ProjectionState,
}

impl PulledSession {
    /// Reads the engine's whole session now.
    pub(super) fn read(context: &SessionSyncContext) -> Result<Self, SessionFetchError> {
        let state = context
            .engine
            .session(Wait::Each(context.request_timeout()))
            .map_err(fetch_error)?;
        Ok(Self { state })
    }
}

impl PublishSource for PulledSession {
    fn panes_awaiting_cwd(&self) -> Vec<String> {
        // The engine started each pane in the folder it holds for it.
        Vec::new()
    }

    fn project(&self) -> SessionSnapshotPayload {
        self.state.project()
    }

    fn published_state(&self) -> &ProjectionState {
        &self.state
    }

    fn agents_confirmed(&self) -> bool {
        // The agents are part of the session read, never a list asked apart.
        true
    }
}

/// A failed session read in the words the screen's engine state reads.
fn fetch_error(error: EngineError) -> SessionFetchError {
    match error {
        EngineError::NotRunning(message)
        | EngineError::Transport(message)
        | EngineError::Unanswered(message)
        | EngineError::Refused { message, .. } => SessionFetchError::Unreachable(message),
        EngineError::Protocol { message, .. }
        | EngineError::Malformed(message)
        | EngineError::Request(message)
        | EngineError::Answer(message) => SessionFetchError::Malformed(message),
    }
}
