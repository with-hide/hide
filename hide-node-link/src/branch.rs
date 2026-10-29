//! What a node answers about a branch against its base and about merging it
//! (`Call::BranchFacts`, `Call::BranchMerge`; PRD hide-ai-branch-first-apply).
//!
//! Both run on the machine that owns the checkout. A merge is local: it
//! never pushes or fetches, and every answer but `Merged` means nothing in
//! the repository changed.

use serde::{Deserialize, Serialize};

/// The most files one [`BranchFacts`] lists; `files_total` and the line
/// counts still cover all of them.
pub const FILES_MAX: usize = 200;

/// The most paths a [`MergeOutcome::Dirty`] names.
pub const DIRTY_FILES_MAX: usize = 20;

/// The longest merge commit message a request may carry.
pub const MESSAGE_MAX_BYTES: usize = 16 * 1024;

/// One file the branch changes against the merge base.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BranchFile {
    /// Repository-relative, `/` between names on every system.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// Git counts no lines for it; `added` and `removed` are 0.
    pub binary: bool,
}

/// A branch measured against the base branch, as the repository answers now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BranchFacts {
    /// `refs/heads/<branch>` is there.
    pub exists: bool,
    /// The full object name `refs/heads/<branch>` points at; `None` when the
    /// branch is gone.
    pub tip: Option<String>,
    /// Commits on the branch that the base does not have; 0 when the branch
    /// or the base is gone.
    pub ahead: u32,
    /// The branch's tip, or the request's `tip` when the branch is gone, is
    /// an ancestor of the base. `None` when the base is gone or the tip is
    /// not an object of this repository.
    pub merged: Option<bool>,
    /// The committed diff from the merge base to the tip, at most
    /// [`FILES_MAX`] files in path order; empty when the branch is gone.
    pub files: Vec<BranchFile>,
    /// How many files that diff changes, listed or not.
    pub files_total: u32,
    /// Lines added and removed over all `files_total` files.
    pub added: u32,
    pub removed: u32,
}

/// What a merge request did. Only `Merged` changes the repository.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// The base now holds the branch. `commit` is the base's new head, the
    /// branch's tip itself when `fast_forward`.
    Merged { commit: String, fast_forward: bool },
    /// The worktree that holds the base has uncommitted changes in files the
    /// branch changes (repository-relative, at most [`DIRTY_FILES_MAX`]), or
    /// is in the middle of a merge, rebase, cherry-pick or revert, in which
    /// case `files` is empty.
    Dirty { files: Vec<String> },
    /// The branch and the base change the same lines.
    Conflict,
    /// `refs/heads/<branch>` is not there.
    BranchMissing,
    /// The base moved while the merge was prepared; nothing was written.
    BaseMoved,
    /// The branch's tip is already in the base, so there was nothing to do.
    /// `commit` is the base's head now, which a retry after a merge finds.
    AlreadyMerged { commit: String },
}

/// What a discard request did. Only `Deleted` changes the repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DiscardOutcome {
    /// `refs/heads/<branch>` is gone; no worktree and nothing remote was touched.
    Deleted,
    /// There was no such branch, which is where a retry after a discard ends.
    Missing,
    /// A worktree has the branch checked out; nothing was done.
    CheckedOut,
    /// The request named a `tip` and the branch no longer points at it, so
    /// someone has committed since; nothing was done.
    Moved,
}

/// A branch name a request may carry (`git::branch_name`).
pub fn check_branch(value: &str) -> Result<&str, String> {
    crate::git::branch_name(value)
}

/// A full object name: 40 hex digits, or 64 for a SHA-256 repository.
pub fn check_tip(value: &str) -> Result<&str, String> {
    let valid = matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    valid
        .then_some(value)
        .ok_or_else(|| format!("not an object name: {value:?}"))
}

/// A `BranchFacts` request's values, or why they cannot make a git command.
pub fn check_facts<'a>(branch: &'a str, tip: Option<&'a str>, base: &'a str) -> Result<(), String> {
    check_branch(branch)?;
    check_branch(base)?;
    tip.map(check_tip).transpose()?;
    Ok(())
}

/// A `BranchDiscard` request's values, or why they cannot make a git command.
pub fn check_discard(branch: &str, tip: Option<&str>) -> Result<(), String> {
    check_branch(branch)?;
    tip.map(check_tip).transpose()?;
    Ok(())
}

/// A `BranchDiff` request's values, or why they cannot make a git command:
/// `path` is repository-relative (`git::repository_path`).
pub fn check_diff(branch: &str, base: &str, path: &str) -> Result<(), String> {
    check_branch(branch)?;
    check_branch(base)?;
    crate::git::repository_path(path).map(|_| ())
}

/// A `BranchMerge` request's values, or why they cannot make a git command.
pub fn check_merge(branch: &str, base: &str, message: &str) -> Result<(), String> {
    check_branch(branch)?;
    check_branch(base)?;
    if branch == base {
        return Err("a branch is not merged into itself".to_owned());
    }
    if message.trim().is_empty() || message.len() > MESSAGE_MAX_BYTES || message.contains('\0') {
        return Err("not a merge message".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outcome_is_one_tagged_object_and_reads_back() {
        let merged = MergeOutcome::Merged {
            commit: "a".repeat(40),
            fast_forward: true,
        };
        let value = serde_json::to_value(&merged).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"outcome": "merged", "commit": "a".repeat(40), "fast_forward": true})
        );
        for outcome in [
            merged,
            MergeOutcome::Dirty {
                files: vec!["src/a.rs".into()],
            },
            MergeOutcome::Conflict,
            MergeOutcome::BranchMissing,
            MergeOutcome::BaseMoved,
            MergeOutcome::AlreadyMerged {
                commit: "b".repeat(40),
            },
        ] {
            let text = serde_json::to_string(&outcome).unwrap();
            assert_eq!(
                serde_json::from_str::<MergeOutcome>(&text).unwrap(),
                outcome
            );
        }
        assert_eq!(
            serde_json::to_value(MergeOutcome::BaseMoved).unwrap(),
            serde_json::json!({"outcome": "base_moved"})
        );
    }

    #[test]
    fn a_discard_outcome_is_one_tagged_object() {
        for (outcome, name) in [
            (DiscardOutcome::Deleted, "deleted"),
            (DiscardOutcome::Missing, "missing"),
            (DiscardOutcome::CheckedOut, "checked_out"),
            (DiscardOutcome::Moved, "moved"),
        ] {
            let value = serde_json::to_value(outcome).unwrap();
            assert_eq!(value, serde_json::json!({ "outcome": name }));
            assert_eq!(
                serde_json::from_value::<DiscardOutcome>(value).unwrap(),
                outcome
            );
        }
        assert!(check_diff("feat/x", "main", "src/a.rs").is_ok());
        for path in ["", "/etc/passwd", "../x", "a/../../x", "a\\b", "a\0b"] {
            assert!(check_diff("feat/x", "main", path).is_err(), "{path:?}");
        }
        assert!(check_diff("--x", "main", "a").is_err());
        assert!(check_discard("feat/x", Some("main")).is_err());
        assert!(check_discard("-D", None).is_err());
        assert!(check_discard("feat/x", None).is_ok());
    }

    #[test]
    fn facts_read_back_with_every_field() {
        let facts = BranchFacts {
            exists: false,
            tip: None,
            ahead: 0,
            merged: Some(true),
            files: vec![BranchFile {
                path: "a.bin".into(),
                added: 0,
                removed: 0,
                binary: true,
            }],
            files_total: 1,
            added: 0,
            removed: 0,
        };
        let text = serde_json::to_string(&facts).unwrap();
        assert_eq!(serde_json::from_str::<BranchFacts>(&text).unwrap(), facts);
    }

    /// A branch or an object name is never an option, a range or a refspec,
    /// and a merge message is bounded text.
    #[test]
    fn a_value_git_could_read_as_something_else_is_refused() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert!(check_facts("feat/x", Some(sha), "main").is_ok());
        assert!(check_facts("feat/x", None, "main").is_ok());
        for branch in ["--orphan=x", "a..b", "a:b", "HEAD~1", "", "main^"] {
            assert!(check_facts(branch, None, "main").is_err(), "{branch}");
            assert!(check_facts("feat/x", None, branch).is_err(), "{branch}");
        }
        for tip in [
            "",
            "main",
            "0123",
            "ABCDEF0123456789abcdef0123456789abcdef01",
            "-123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(check_facts("feat/x", Some(tip), "main").is_err(), "{tip}");
        }
        assert!(check_tip(&"f".repeat(64)).is_ok());
        assert!(check_merge("feat/x", "main", "Merge feat/x").is_ok());
        assert!(check_merge("main", "main", "Merge").is_err());
        assert!(check_merge("feat/x", "main", "  ").is_err());
        assert!(check_merge("feat/x", "main", &"x".repeat(MESSAGE_MAX_BYTES + 1)).is_err());
    }
}
