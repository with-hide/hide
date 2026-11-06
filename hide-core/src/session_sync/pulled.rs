//! The session of an engine whose changes are pulled (PRD
//! engine-bundle-3-core-owns D-3): the core's own engine wakes the
//! coordinator once its topology changed, and the coordinator reads the
//! whole session again ([`crate::engine::Engine::session`]) and publishes
//! it as it is. [`PulledSession`] keeps only the last session read; nothing
//! is replayed, so no event replica or subscription runs for such an engine.

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

impl PulledSession {
    /// The session the runtime may see, as the sidebar projects it.
    pub(super) fn project(&self) -> SessionSnapshotPayload {
        self.state.project()
    }

    /// The state behind [`Self::project`].
    pub(super) fn published_state(&self) -> &ProjectionState {
        &self.state
    }
}

/// A failed session read in the words the screen's engine state reads.
fn fetch_error(error: EngineError) -> SessionFetchError {
    match error {
        EngineError::NotRunning(message)
        | EngineError::Transport(message)
        | EngineError::Unanswered(message)
        | EngineError::Refused { message, .. } => SessionFetchError::Unreachable(message),
        EngineError::Malformed(message)
        | EngineError::Request(message)
        | EngineError::Answer(message) => SessionFetchError::Malformed(message),
    }
}
