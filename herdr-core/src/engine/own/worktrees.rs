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
//! the folder trust entries name that path. Git runs on this machine
//! (`hide_host::worktrees::git`), the machine this engine runs.

use std::path::{Path, PathBuf};

use super::topology::{Bound, Pane};
use super::{Answer, Env, OwnEngine};
use crate::engine::{CreatedWorktree, EngineError, OpenedWorktree};

fn refused(code: &str, message: String) -> EngineError {
    EngineError::Refused {
        code: code.to_owned(),
        message,
    }
}

fn git(cwd: &Path, arguments: &[&str]) -> Answer<String> {
    hide_host::worktrees::git(cwd, arguments).map_err(|why| {
        refused(
            "git_failed",
            format!("git {} in {}: {why}", arguments.join(" "), cwd.display()),
        )
    })
}

/// A path Git prints on one line, without the line's end.
fn git_path(cwd: &Path, arguments: &[&str]) -> Answer<PathBuf> {
    let printed = git(cwd, arguments)?;
    let path = printed.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        return Err(refused(
            "git_failed",
            format!(
                "git {} in {} printed no path",
                arguments.join(" "),
                cwd.display()
            ),
        ));
    }
    Ok(PathBuf::from(path))
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

/// The repository a checkout at `path` belongs to, and the checkout's own
/// top folder, in the binding's terms.
fn binding(path: &Path) -> Answer<Bound> {
    let common = git_path(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let top = git_path(path, &["rev-parse", "--show-toplevel"])?;
    // A repository's shared folder is its main checkout's `.git`.
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
                format!("{} belongs to a bare repository", path.display()),
            ));
        }
    };
    let same = |left: &Path, right: &Path| {
        std::fs::canonicalize(left).ok() == std::fs::canonicalize(right).ok()
    };
    Ok(Bound {
        repo_key: wire(&common)?,
        repo_name: root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        linked: !same(&top, &root),
        repo_root: wire(&root)?,
        checkout_path: wire(&top)?,
    })
}

impl OwnEngine<'_> {
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
        let cwd = native(cwd)?;
        let repository = binding(&cwd)?;
        let root = native(&repository.repo_root)?;
        let place = match path {
            Some(path) => native(path)?,
            None => {
                let home = self.home()?;
                hide_kit::layout::engine_worktree(&home, &repository.repo_name, &path_slug(branch))
            }
        };
        if let Some(parent) = place.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                refused(
                    "worktree_folder_failed",
                    format!("{} could not be made: {error}", parent.display()),
                )
            })?;
        }
        let place_arg = place.to_string_lossy().into_owned();
        let exists = !git(&root, &["branch", "--list", branch])?.trim().is_empty();
        if exists {
            git(&root, &["worktree", "add", &place_arg, branch])?;
        } else {
            let mut arguments = vec!["worktree", "add", "-b", branch, &place_arg];
            if let Some(base) = base {
                arguments.push(base);
            }
            git(&root, &arguments)?;
        }
        let bound = binding(&place)?;
        let label = label.map_or_else(
            || {
                place
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
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
        let cwd = native(cwd)?;
        let listed = git(&cwd, &["worktree", "list", "--porcelain"])?;
        hide_host::worktrees::parse_worktree_list(&listed)
            .into_iter()
            .find(|worktree| worktree.branch.as_deref() == Some(branch))
            .map(|worktree| wire(&worktree.path))
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
        let bound = binding(&native(path)?)?;
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
    /// was bound to, which Git refuses while it holds changes.
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
        self.end_panes(&panes)?;
        match self.topology().remove_workspace(workspace) {
            // Its last pane closed it while the holders ended.
            Ok(()) | Err(EngineError::Refused { .. }) => {}
            Err(error) => return Err(error),
        }
        self.changed();
        let checkout = bound.checkout_path.clone();
        git(
            &native(&bound.repo_root)?,
            &["worktree", "remove", &native(&checkout)?.to_string_lossy()],
        )
        .map(|_| ())
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
        let reserved = self.topology().reserve_workspace();
        self.start_pane(&reserved.pane, None, checkout, Env::new(), None)?;
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
