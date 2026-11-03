// `path_slug` is derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr,
// commit 2ecf19a), src/worktree.rs (`branch_to_path_slug`).
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: an empty slug falls back to `worktree` rather than Herdr's
// branch prefix; the rest of this file is Hide's own.

//! The worktrees of the core's own engine (PRD engine-bundle-3-core-owns
//! D-28): a new one is made with Git in Hide's own folder,
//! `hide_kit::layout::engine_worktree` (`~/.hide/worktrees/<repo>/<name>`),
//! and opened in a workspace bound to it; a checkout made anywhere else, a
//! Herdr worktree under `~/.herdr/worktrees` included, is opened where it is
//! and never moved, since the agents working in it, their resume records and
//! the folder trust entries name that path. The core touches no machine
//! (PRD core-host-node D-21): the node of the machine the panes run on reads
//! each repository and runs its Git.

use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_node_link::cleanup::{CleanRemoval, RepositoryDirs};
use hide_node_link::git::GitCommand;
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::Registered;

use super::topology::{Bound, Pane};
use super::{Answer, Env, OwnEngine};
use crate::engine::{CreatedWorktree, EngineError, OpenedWorktree};
use crate::node_access::{LinkError, call_as, link_failure_code};

/// How long a read of a repository waits for the node.
const READ_WAIT: Duration = Duration::from_secs(30);
/// How long a new worktree or a removal waits: the node's own bound for a
/// change of a checkout (`git_run::CHANGE_DEADLINE`, ten minutes), and a
/// little more.
const CHANGE_WAIT: Duration = Duration::from_secs(10 * 60 + 30);

fn refused(code: &str, message: String) -> EngineError {
    EngineError::Refused {
        code: code.to_owned(),
        message,
    }
}

fn native(path: &str) -> Answer<PathBuf> {
    hide_platform::path::from_wire(path)
        .map_err(|error| EngineError::Request(format!("{path} is not a folder path: {error}")))
}

fn wire(path: &Path) -> Answer<String> {
    hide_platform::path::to_wire(path).map_err(|error| {
        EngineError::Request(format!("{} has no wire spelling: {error}", path.display()))
    })
}

/// A path as the node spelled it, in the engine's wire spelling.
fn rewired(path: &str) -> Answer<String> {
    wire(Path::new(path))
}

/// A branch as a folder name: its letters and digits in lower case, every
/// other run of characters one `-`.
pub(super) fn path_slug(branch: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "worktree".to_owned()
    } else {
        trimmed
    }
}

/// The binding of the checkout `dirs` names: its repository, read from the
/// shared Git folder, which is the main checkout's `.git`, and whether it is
/// a linked worktree, whose own Git folder is not the shared one.
fn bound(path: &str, dirs: RepositoryDirs) -> Answer<Bound> {
    let common = PathBuf::from(&dirs.common_dir);
    let root = match common.file_name() {
        Some(name) if name == ".git" => {
            common.parent().map(Path::to_path_buf).ok_or_else(|| {
                refused(
                    "not_a_checkout",
                    format!("{} has no folder", common.display()),
                )
            })?
        }
        _ => {
            return Err(refused(
                "not_a_checkout",
                format!("{path} belongs to a bare repository"),
            ));
        }
    };
    Ok(Bound {
        repo_key: wire(&common)?,
        repo_name: root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        linked: dirs.git_dir != dirs.common_dir,
        repo_root: wire(&root)?,
        checkout_path: rewired(&dirs.root)?,
    })
}

impl OwnEngine<'_> {
    /// `call` answered by the node of this engine's machine, its failure
    /// named by `what`.
    fn ask<T: serde::de::DeserializeOwned>(
        &self,
        what: &str,
        call: Call,
        wait: Duration,
    ) -> Answer<T> {
        let node = self.node()?;
        call_as(node.as_ref(), call, wait)
            .map_err(|error| refused(&link_failure_code(&error), format!("{what}: {error}")))
    }

    /// The binding of the checkout that holds `path`.
    fn binding(&self, path: &str) -> Answer<Bound> {
        let dirs: Option<RepositoryDirs> = self.ask(
            &format!("reading the repository of {path}"),
            Call::Repository {
                path: path.to_owned(),
            },
            READ_WAIT,
        )?;
        let dirs = dirs
            .ok_or_else(|| refused("not_a_checkout", format!("{path} is in no Git repository")))?;
        bound(path, dirs)
    }

    /// A new worktree of the repository at `cwd` on `branch` (made from
    /// `base` when the branch is new), at `path` or in Hide's worktree
    /// folder, opened in a new workspace bound to it.
    #[allow(clippy::too_many_arguments)] // one worktree's branch, start, name and place
    pub(super) fn make_worktree(
        &self,
        cwd: &str,
        branch: &str,
        base: Option<&str>,
        label: Option<&str>,
        path: Option<&str>,
        focus: bool,
    ) -> Answer<CreatedWorktree> {
        let repository = self.binding(cwd)?;
        let place = match path {
            Some(path) => path.to_owned(),
            None => {
                let home = self.home()?;
                hide_kit::layout::engine_worktree(&home, &repository.repo_name, &path_slug(branch))
                    .map_err(|error| {
                        EngineError::Request(format!(
                            "no worktree folder for {branch} under {home}: {error}"
                        ))
                    })?
            }
        };
        // As the core reads a branch elsewhere: a refusal is a branch that is
        // not there, which `worktree add -b` then makes or names Git's reason.
        let exists = match self.node()?.call(
            Call::Git {
                root: repository.repo_root.clone(),
                command: GitCommand::HasLocalBranch {
                    branch: branch.to_owned(),
                },
            },
            READ_WAIT,
        ) {
            Ok(_) => true,
            Err(LinkError::Refused(_)) => false,
            Err(error) => {
                return Err(refused(
                    &link_failure_code(&error),
                    format!("reading whether {branch} is a branch: {error}"),
                ));
            }
        };
        let _: String = self.ask(
            &format!("adding the worktree {place}"),
            Call::Git {
                root: repository.repo_root.clone(),
                command: GitCommand::WorktreeAdd {
                    path: place.clone(),
                    branch: branch.to_owned(),
                    new_branch: !exists,
                    base: base.filter(|_| !exists).map(str::to_owned),
                },
            },
            CHANGE_WAIT,
        )?;
        let bound = self.binding(&place)?;
        let label = label.map_or_else(
            || {
                native(&bound.checkout_path)
                    .ok()
                    .and_then(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default()
            },
            str::to_owned,
        );
        let checkout = bound.checkout_path.clone();
        let (workspace, tab, pane) = self.open_bound(&checkout, &label, focus, bound)?;
        Ok(CreatedWorktree {
            tab_label: self.topology().tab_label(&tab)?,
            workspace_id: workspace,
            tab_id: tab,
            pane_id: pane,
            path: checkout,
            branch: Some(branch.to_owned()),
        })
    }

    /// Where the repository at `cwd` has `branch` checked out, if anywhere.
    pub(super) fn listed_worktree(&self, cwd: &str, branch: &str) -> Answer<Option<String>> {
        let listed: Vec<Registered> = self.ask(
            &format!("listing the worktrees of {cwd}"),
            Call::WorktreesRegistered {
                root: cwd.to_owned(),
            },
            READ_WAIT,
        )?;
        listed
            .into_iter()
            .find(|worktree| worktree.branch.as_deref() == Some(branch))
            .map(|worktree| rewired(&worktree.path))
            .transpose()
    }

    /// The workspace bound to the checkout at `path`, opened in a new one
    /// bound to it when none is.
    pub(super) fn open_checkout(
        &self,
        path: &str,
        _repository_root: &str,
        focus: bool,
    ) -> Answer<OpenedWorktree> {
        let bound = self.binding(path)?;
        if let Some(open) = self.topology().bound_to(&bound.checkout_path) {
            if focus {
                self.topology().focus_tab(&open.tab)?;
                self.changed();
            }
            return Ok(OpenedWorktree {
                tab_label: open.tab_label,
                workspace_id: open.workspace,
                label: open.label,
                tab_id: open.tab,
                pane_id: open.pane,
                already_open: true,
            });
        }
        let checkout = bound.checkout_path.clone();
        let label = native(&checkout)?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (workspace, tab, pane) = self.open_bound(&checkout, &label, focus, bound)?;
        Ok(OpenedWorktree {
            tab_label: self.topology().tab_label(&tab)?,
            workspace_id: workspace,
            label,
            tab_id: tab,
            pane_id: pane,
            already_open: false,
        })
    }

    /// Ends the workspace's panes, closes it, and removes the worktree it
    /// was bound to without force, which Git refuses while it holds changes.
    pub(super) fn drop_worktree(&self, workspace: &str) -> Answer<()> {
        let bound = self.topology().bound(workspace)?.ok_or_else(|| {
            refused(
                "not_a_worktree",
                format!("workspace {workspace} is bound to no worktree"),
            )
        })?;
        if !bound.linked {
            return Err(refused(
                "main_worktree",
                format!("{} is its repository's main worktree", bound.checkout_path),
            ));
        }
        let panes = self.topology().workspace_panes(workspace)?;
        match self.end_and_remove(&panes, |topology| topology.remove_workspace(workspace))? {
            // Its last pane closed it while the holders ended.
            Ok(()) | Err(EngineError::Refused { .. }) => {}
            Err(error) => return Err(error),
        }
        self.changed();
        let removal: CleanRemoval = self.ask(
            &format!("removing the worktree {}", bound.checkout_path),
            Call::WorktreeRemoveClean {
                root: bound.repo_root.clone(),
                checkout: bound.checkout_path.clone(),
                common: bound.repo_key.clone(),
            },
            CHANGE_WAIT,
        )?;
        if !removal.outcome.removed {
            return Err(refused("worktree_kept", removal.outcome.message));
        }
        // The folder left its checkout at once; the node deletes it from the
        // repository's trash, as a cleanup's removal does.
        if !removal.trashed.is_empty() {
            let _: usize = self.ask(
                &format!("deleting the removed worktree {}", bound.checkout_path),
                Call::DrainTrash {
                    common: bound.repo_key,
                    ours: removal.trashed,
                    wait_ms: 0,
                },
                CHANGE_WAIT,
            )?;
        }
        Ok(())
    }

    /// A new workspace in `checkout` named `label`, bound to `bound`.
    fn open_bound(
        &self,
        checkout: &str,
        label: &str,
        focus: bool,
        bound: Bound,
    ) -> Answer<(String, String, String)> {
        self.restored()?;
        self.resolve_unpublished()?;
        let reserved = self.topology().reserve_workspace();
        self.start_placed(&reserved.pane, None, checkout, Env::new())?;
        {
            let mut topology = self.topology();
            topology.add_workspace(
                &reserved,
                label,
                Pane::started(checkout, None, None, &Env::new()),
                focus,
            );
            topology.bind(&reserved.workspace, bound)?;
        }
        self.changed();
        Ok((reserved.workspace, reserved.tab, reserved.pane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_becomes_a_folder_name_as_herdr_spells_it() {
        assert_eq!(
            path_slug("feat/Engine Own__layout"),
            "feat-engine-own-layout"
        );
        assert_eq!(path_slug("--"), "worktree");
    }
}
