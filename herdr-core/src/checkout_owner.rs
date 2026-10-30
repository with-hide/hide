//! Which Herdr workspace owns a checkout (PRD checkout-workspace-binding).
//!
//! A checkout has one owner Herdr workspace, read from Herdr rather than
//! inferred from where its tabs sit (D-06). For a Git checkout it is the
//! workspace Herdr binds to that checkout path (`worktree.checkout_path`,
//! set only by `worktree.open` and `worktree.create`); a plain folder cannot
//! be bound, so its owner is the live workspace carrying Hide's own mark, a
//! workspace token whose value is a digest of the device and the folder
//! (D-11). Every tab Hide creates in a checkout goes to its owner, and a
//! checkout without one gets one first (D-07); a tab in any other workspace
//! still shows under the checkout its cwd names and is never moved (D-08).
//!
//! The owner is decided from the session Hide already receives, so a
//! snapshot costs no Herdr call; only creating a tab in a checkout with no
//! owner opens one (B16).

pub(crate) mod open;

use sha2::{Digest, Sha256};

/// The workspace token that marks a plain-folder workspace Hide opened as the
/// folder's owner. Herdr drops workspace tokens on a server restart, so the
/// next tab after one makes and marks a new owner.
pub(crate) const OWNER_TOKEN: &str = "hide_owner";

/// The mark a plain folder's owner carries: a stable digest of the device and
/// the folder, so the same path on two devices never shares an owner.
pub(crate) fn owner_mark(device_id: &str, path: &str) -> String {
    let digest = Sha256::digest(format!("{device_id}\0{}", comparable(path)).as_bytes());
    digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A path as the owner rule compares it: Herdr and the catalog spell the same
/// folder with or without a trailing slash.
fn comparable(path: &str) -> &str {
    let trimmed = path.trim();
    match trimmed.trim_end_matches('/') {
        "" => trimmed,
        path => path,
    }
}

/// One Herdr workspace as the owner rule reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkspaceFacts<'a> {
    pub(crate) workspace_id: &'a str,
    /// The checkout Herdr binds this workspace to, if it is a worktree
    /// workspace.
    pub(crate) bound_path: Option<&'a str>,
    /// The value of this workspace's `hide_owner` token, if it has one.
    pub(crate) mark: Option<&'a str>,
}

impl<'a> From<&'a crate::sidebar::SessionWorkspacePayload> for WorkspaceFacts<'a> {
    fn from(workspace: &'a crate::sidebar::SessionWorkspacePayload) -> Self {
        Self {
            workspace_id: &workspace.workspace_id,
            bound_path: workspace
                .worktree
                .as_ref()
                .map(|worktree| worktree.checkout_path.as_str()),
            mark: workspace
                .tokens
                .get(OWNER_TOKEN)
                .and_then(serde_json::Value::as_str),
        }
    }
}

/// The workspace that owns the checkout at `path` on `device_id`, among the
/// host's workspaces, or `None` when none is open.
pub(crate) fn owner_of<'a>(
    device_id: &str,
    path: &str,
    is_git: bool,
    workspaces: impl IntoIterator<Item = WorkspaceFacts<'a>>,
) -> Option<&'a str> {
    if path.trim().is_empty() {
        return None;
    }
    OwnerOpen::for_checkout(device_id, path, path, is_git, "").find_in(workspaces)
}

/// The owner of the checkout at `path` on the core's own node. A plain
/// folder's owner opened before node ids existed carries the mark of the
/// legacy `local` device until Herdr drops workspace tokens on a restart; it
/// is still this folder's owner, so no second one is opened (PRD
/// core-host-node D-23, the transition path engineering principle 1 keeps).
pub(crate) fn node_owner_of<'a>(
    node: &str,
    path: &str,
    is_git: bool,
    workspaces: impl IntoIterator<Item = WorkspaceFacts<'a>>,
) -> Option<&'a str> {
    if path.trim().is_empty() {
        return None;
    }
    OwnerOpen::for_checkout(node, path, path, is_git, "")
        .on_node()
        .find_in(workspaces)
}

/// How a checkout with no open owner gets one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerOpen {
    /// `worktree.open` on the checkout, from its repository's main worktree
    /// (Herdr refuses a linked worktree opened from anywhere else): Herdr
    /// answers with the workspace it binds to that path, binding an unbound
    /// one already there or opening one, so a repeated or racing request
    /// converges on one owner (`already_open`). `label` names only a
    /// workspace Herdr newly opened; one already there keeps its name.
    Worktree {
        path: String,
        repository_root: String,
        label: String,
    },
    /// A plain folder: reuse the live workspace carrying `mark`, else
    /// `workspace.create` at the folder and mark it.
    Folder {
        path: String,
        label: String,
        mark: String,
        /// The mark this folder's owner on the core's own node carried before
        /// node ids existed (`on_node`); a workspace carrying it is reused,
        /// and a new owner gets `mark`.
        legacy_mark: Option<String>,
    },
}

impl OwnerOpen {
    /// The owner a checkout with no open owner gets.
    pub(crate) fn for_checkout(
        device_id: &str,
        path: &str,
        repository_root: &str,
        is_git: bool,
        label: &str,
    ) -> Self {
        if is_git {
            Self::Worktree {
                path: path.to_owned(),
                repository_root: repository_root.to_owned(),
                label: label.to_owned(),
            }
        } else {
            Self::Folder {
                path: path.to_owned(),
                label: label.to_owned(),
                mark: owner_mark(device_id, path),
                legacy_mark: None,
            }
        }
    }

    /// This owner is on the core's own node, so a folder owner marked for
    /// the legacy `local` device is accepted too (`node_owner_of`).
    pub(crate) fn on_node(self) -> Self {
        match self {
            Self::Folder {
                path, label, mark, ..
            } => Self::Folder {
                legacy_mark: Some(owner_mark(crate::node::LEGACY_LOCAL_DEVICE_ID, &path)),
                path,
                label,
                mark,
            },
            worktree => worktree,
        }
    }

    /// Whether a workspace's `hide_owner` value marks it as this folder's
    /// owner.
    pub(crate) fn marks(&self, value: &str) -> bool {
        match self {
            Self::Folder {
                mark, legacy_mark, ..
            } => mark == value || legacy_mark.as_deref() == Some(value),
            Self::Worktree { .. } => false,
        }
    }

    /// The open workspace that is this checkout's owner, among the host's
    /// workspaces: the one Herdr binds to the path, or the one carrying the
    /// folder's mark.
    pub(crate) fn find_in<'a>(
        &self,
        workspaces: impl IntoIterator<Item = WorkspaceFacts<'a>>,
    ) -> Option<&'a str> {
        workspaces
            .into_iter()
            .find(|workspace| match self {
                Self::Worktree { path, .. } => workspace
                    .bound_path
                    .is_some_and(|bound| comparable(bound) == comparable(path)),
                Self::Folder { .. } => workspace.mark.is_some_and(|mark| self.marks(mark)),
            })
            .map(|workspace| workspace.workspace_id)
    }

    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Worktree { path, .. } | Self::Folder { path, .. } => path,
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Worktree { .. } => "worktree.open",
            Self::Folder { .. } => "workspace.create",
        }
    }
}

/// Where a new tab in a checkout goes: its open owner, or an owner opened
/// first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TabHost {
    Workspace(String),
    Open(OwnerOpen),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(id: &'a str, bound: Option<&'a str>, mark: Option<&'a str>) -> WorkspaceFacts<'a> {
        WorkspaceFacts {
            workspace_id: id,
            bound_path: bound,
            mark,
        }
    }

    #[test]
    fn a_git_checkout_is_owned_by_the_workspace_herdr_binds_to_its_path_not_by_where_its_tabs_sit()
    {
        // w8P holds the checkout's tabs but Herdr binds the path to w9J.
        let workspaces = [
            facts("w8P", None, None),
            facts("w9J", Some("/repo/"), None),
            facts("wX", Some("/repo/.worktrees/other"), None),
        ];
        assert_eq!(owner_of("local", "/repo", true, workspaces), Some("w9J"));
        assert_eq!(
            owner_of("local", "/repo/.worktrees/missing", true, workspaces),
            None
        );
    }

    #[test]
    fn a_plain_folder_is_owned_only_by_the_workspace_carrying_its_own_mark() {
        let mark = owner_mark("local", "/notes");
        let other_device = owner_mark("mini", "/notes");
        assert_ne!(mark, other_device);
        let workspaces = [
            facts("w1", Some("/notes"), None),
            facts("w2", None, Some(other_device.as_str())),
            facts("w3", None, Some(mark.as_str())),
        ];
        assert_eq!(owner_of("local", "/notes/", false, workspaces), Some("w3"));
        // A workspace Herdr binds, or one marked for another device, is not it.
        assert_eq!(
            owner_of("local", "/notes", false, [workspaces[0], workspaces[1]]),
            None
        );
    }
}
