//! One shape for a value read from Git or GitHub (PRD git-freshness D-09):
//! the last value read successfully stays beside it, with when it was read
//! and whether it is current, older than its reader's interval, or behind a
//! read that failed. The checkout header's Git read, each of its CI results
//! and the remote the background fetch checked carry it; the pull request
//! list carries the same facts in `GithubStatusSnapshot`
//! (`last_success_at_unix_ms`, `stale`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Freshness {
    /// Read within its reader's interval.
    Fresh,
    /// Not read again within its interval; the value is the last one read.
    Stale,
    /// The last read failed; the value is the last one read, if any.
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Observation {
    pub freshness: Freshness,
    /// When the value was last read successfully; absent when it never was.
    pub read_at_unix_ms: Option<u64>,
}

impl Observation {
    pub(crate) fn fresh(read_at_unix_ms: u64) -> Self {
        Self {
            freshness: Freshness::Fresh,
            read_at_unix_ms: Some(read_at_unix_ms),
        }
    }

    /// The read failed: what was read before stays, as of when it was read.
    pub(crate) fn failed(before: Option<Self>) -> Self {
        Self {
            freshness: Freshness::Failed,
            read_at_unix_ms: before.and_then(|before| before.read_at_unix_ms),
        }
    }

    /// The value aged past its interval; a failure stays a failure.
    pub(crate) fn aged(self) -> Self {
        match self.freshness {
            Freshness::Fresh => Self {
                freshness: Freshness::Stale,
                ..self
            },
            Freshness::Stale | Freshness::Failed => self,
        }
    }
}

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}
