//! The one-time retirement of Hide's old Home folder (`~/hide`).

use serde::{Deserialize, Serialize};

/// What [`crate::protocol::Call::HomeRetire`] did to one machine's `~/hide`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeRetired {
    /// Absolute path of `<user home>/hide`, in wire spelling (`/`).
    pub home: String,
    /// Whether the folder is still there after the call: it holds something
    /// Hide did not make, a session or pane was recorded in it, it has no
    /// marker of Hide's, or it is not a folder.
    pub kept: bool,
    /// Hide's marker was found, so its entries were Hide's to remove.
    pub marked: bool,
    /// The names of the entries removed.
    pub removed: Vec<String>,
    /// Entries the marker listed that now hold something else, left as the
    /// operator's.
    pub changed: Vec<String>,
    /// Why the cleanup of a folder Hide made (its marker was found) could not
    /// finish, when it could not: the folder is still there, and a later call
    /// retries. Failures before the marker was found are errors of the call.
    #[serde(default)]
    pub failure: Option<String>,
}
