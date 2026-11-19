//! A new project made on its node: the folder, its repository and the path
//! the core registers it by.

use serde::{Deserialize, Serialize};

/// An automation's own folder as its node made it (`Call::AutomationFolder`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AutomationFolder {
    /// The folder with its links resolved, spelled with `/`: the path its
    /// registration names.
    pub path: String,
}

/// What a project creation did on the node.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectCreated {
    /// The project's folder with its links resolved, spelled with `/`: the
    /// path its registration names.
    pub path: String,
    /// Why `git init` failed, when it was asked for and did; the folder is
    /// registered anyway.
    pub git_error: Option<String>,
}
