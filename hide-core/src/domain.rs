use serde::{Deserialize, Serialize};

/// What a workspace's folder is in its repository: the facts a published
/// workspace carries so the sidebar groups it under its repository.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeProjection {
    pub repo_key: String,
    pub repo_name: String,
    pub repo_root: String,
    pub checkout_path: String,
    pub is_linked_worktree: bool,
}
