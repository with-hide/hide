//! What a change request write names, whichever code host takes it: the
//! core's events and every host's node commands share these values.

use serde::{Deserialize, Serialize};

/// How a change request is merged. A repository with a merge queue takes
/// none: the host adds the change request to the queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrMergeMethod {
    Merge,
    Squash,
    Rebase,
}
