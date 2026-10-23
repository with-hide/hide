//! The git shapes a request carries.

use serde::{Deserialize, Serialize};

/// The further diffs one read answers at most: one per View area the shell
/// can show side by side (PRD S7 A5), so a read stays bounded whoever asks.
pub const MAX_DIFFS: usize = 6;

/// A file whose diff a View display shows, relative to the scope like every
/// path in the answer, and the group it is taken in.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffTarget {
    pub path: String,
    pub committed: bool,
}

/// What to read: the folder below the root the answer is limited to, the
/// file whose diff to fetch and which group it is in, and the branch the
/// committed group is measured against. `base: None` measures against the
/// repository's default branch when it has one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangesQuery {
    pub scope: String,
    pub selected: Option<String>,
    pub committed: bool,
    pub base: Option<String>,
    /// Further diffs to take in the same read, one per View display of a
    /// diff; past `MAX_DIFFS` they are not answered.
    #[serde(default)]
    pub diffs: Vec<DiffTarget>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Changes {
    /// The working tree against `HEAD`: what `git status` reports.
    pub entries: Vec<ChangedFile>,
    /// What this branch's commits changed since `base`. Absent when there is
    /// no base to compare with, which is not the same as an empty group.
    pub committed: Option<Vec<ChangedFile>>,
    /// The base the committed group was measured against, as this checkout
    /// resolved it.
    pub base: Option<String>,
    pub diff: Option<Diff>,
    /// One diff per answered target of `ChangesQuery::diffs`, in order. A
    /// target no longer in its group is answered with empty text and a
    /// notice saying so, never left out.
    #[serde(default)]
    pub diffs: Vec<GroupDiff>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDiff {
    pub committed: bool,
    #[serde(flatten)]
    pub diff: Diff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub previous: Option<String>,
    pub status: FileStatus,
    pub added: Option<u32>,
    pub removed: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub path: String,
    pub text: String,
    /// Set when the diff was cut short or could not be taken, naming why;
    /// the core's log keeps it.
    pub notice: Option<String>,
    /// The same, as a code the page words; absent from a helper of an older
    /// build, which the core reads as `unreadable`.
    #[serde(default)]
    pub kind: Option<DiffNotice>,
}

/// Why a diff is short or absent, as a code the page words
/// (docs/LOCALIZATION.md, Translation boundary).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DiffNotice {
    /// Larger than `limit_kb` and shown cut short.
    Truncated { limit_kb: u32 },
    /// The checkout has no base branch to compare with.
    NoBase,
    /// The file has no changes on this branch.
    NoBranchChanges,
    /// The file has no uncommitted changes.
    NoUncommittedChanges,
    /// Git could not produce the diff; why is in the log.
    Unreadable,
    /// The file is outside `root`, the folder History reads for the
    /// Project in front.
    Outside { root: String },
}

/// The six working-tree states the view presents. Git's porcelain codes
/// carry more distinctions; [`FileStatus::from_porcelain`] is the single
/// place they collapse.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Renamed,
    Conflict,
}

impl FileStatus {
    /// Maps one porcelain v1 `XY` pair onto the presented status. Index and
    /// worktree columns are read together: a file staged as added and then
    /// edited is still an addition to the reader, and a delete on either side
    /// is a delete.
    pub fn from_porcelain(code: &str) -> Self {
        let mut characters = code.chars();
        let index = characters.next().unwrap_or(' ');
        let worktree = characters.next().unwrap_or(' ');
        if matches!(code, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU") {
            return Self::Conflict;
        }
        if index == '?' && worktree == '?' {
            return Self::Untracked;
        }
        if index == 'R' || worktree == 'R' {
            return Self::Renamed;
        }
        if index == 'D' || worktree == 'D' {
            return Self::Deleted;
        }
        if index == 'A' || worktree == 'A' {
            return Self::Added;
        }
        Self::Modified
    }
}

/// A branch setting Hide keeps in a repository's own configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchConfigKey {
    /// `branch.<name>.description`: the checkout's purpose.
    Description,
    /// `branch.<name>.issue`: the issue the checkout works on.
    Issue,
}

impl BranchConfigKey {
    fn name(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Issue => "issue",
        }
    }
}

/// One git command the core has a node run in a repository on that node's
/// machine. Each is a fixed command line ([`GitCommand::args`]); a request
/// names the branch and value, never the arguments.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum GitCommand {
    /// Every uncommitted change, untracked files included.
    Status,
    /// The absolute path of the repository's shared `.git` folder.
    CommonDir,
    /// The commit a ref names; fails when it names none.
    RefCommit {
        name: String,
    },
    /// How many commits `head` has that `base` does not.
    CountCommits {
        base: String,
        head: String,
    },
    /// Every path in the index, each ended by a NUL.
    ListFiles,
    /// Last commit touching this repository-relative file, Unix seconds.
    DocumentTime {
        path: String,
    },
    /// The branch the worktree has checked out.
    CurrentBranch,
    Checkout {
        branch: String,
    },
    /// Answers only when `refs/heads/<branch>` exists.
    HasLocalBranch {
        branch: String,
    },
    /// Fetches `branch` from `origin` into its remote-tracking ref.
    FetchBranch {
        branch: String,
    },
    SetBranchConfig {
        branch: String,
        key: BranchConfigKey,
        value: String,
    },
    /// Removing a setting that is not there succeeds.
    UnsetBranchConfig {
        branch: String,
        key: BranchConfigKey,
    },
}

impl GitCommand {
    /// The arguments after `git --no-optional-locks -C <root>`, or why the
    /// values cannot make them: a branch or a revision is never an option,
    /// a range or a refspec.
    pub fn args(&self) -> Result<Vec<String>, String> {
        let owned = |args: &[&str]| Ok(args.iter().map(|arg| (*arg).to_owned()).collect());
        match self {
            Self::Status => owned(&["status", "--porcelain=v1", "--untracked-files=all"]),
            Self::CommonDir => owned(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
            Self::RefCommit { name } => owned(&[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", revision_name(name)?),
            ]),
            Self::CountCommits { base, head } => owned(&[
                "rev-list",
                "--count",
                &format!("{}..{}", revision_name(base)?, revision_name(head)?),
                "--",
            ]),
            Self::ListFiles => owned(&["ls-files", "-z"]),
            Self::DocumentTime { path } => {
                if path.is_empty()
                    || path.starts_with('/')
                    || path.contains('\\')
                    || path.split('/').any(|part| part == "..")
                    || path.contains('\0')
                {
                    return Err("invalid_document_path".into());
                }
                owned(&[
                    "--literal-pathspecs",
                    "log",
                    "-1",
                    "--format=%ct",
                    "--",
                    path,
                ])
            }
            Self::CurrentBranch => owned(&["symbolic-ref", "--quiet", "--short", "HEAD"]),
            // The closing `--` makes git read the name as a branch only; a
            // tracked file of that name is never restored over its edits.
            Self::Checkout { branch } => owned(&[
                "checkout",
                "--no-overwrite-ignore",
                branch_name(branch)?,
                "--",
            ]),
            Self::HasLocalBranch { branch } => owned(&[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", branch_name(branch)?),
            ]),
            Self::FetchBranch { branch } => {
                let branch = branch_name(branch)?;
                owned(&[
                    "fetch",
                    "--no-tags",
                    "origin",
                    &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}"),
                ])
            }
            Self::SetBranchConfig { branch, key, value } => owned(&[
                "config",
                "--",
                &format!("branch.{}.{}", branch_name(branch)?, key.name()),
                value,
            ]),
            Self::UnsetBranchConfig { branch, key } => owned(&[
                "config",
                "--unset-all",
                "--",
                &format!("branch.{}.{}", branch_name(branch)?, key.name()),
            ]),
        }
    }
}

/// A revision git reads as one: never an option, never a refspec or range.
pub fn revision_name(value: &str) -> Result<&str, String> {
    let valid = !value.is_empty()
        && value.len() <= 255
        && !value.starts_with('-')
        && !value.contains("..")
        && value.chars().all(|c| {
            !c.is_control() && !c.is_whitespace() && !matches!(c, ':' | '\\' | '*' | '?' | '[')
        });
    valid
        .then_some(value)
        .ok_or_else(|| format!("not a revision: {value:?}"))
}

/// A branch name as a revision, which also cannot leave `refs/heads/` or
/// name a revision relative to one (`~`, `^`, `@{`).
pub fn branch_name(value: &str) -> Result<&str, String> {
    let value = revision_name(value)?;
    (!value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(['~', '^'])
        && !value.contains("@{"))
    .then_some(value)
    .ok_or_else(|| format!("not a branch: {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request names a branch or a revision, never an argument: a value
    /// git would read as an option, a range or a refspec is refused before
    /// git sees it, and a setting's value stays text whatever it starts with.
    #[test]
    fn a_value_that_would_read_as_an_option_a_range_or_a_refspec_is_refused() {
        for branch in [
            "--orphan=x",
            "-b",
            "a..b",
            "a:b",
            "main ",
            "/main",
            "",
            "a*",
            "a?",
            "a[b]",
            "HEAD~3",
            "main^",
            "main@{1}",
        ] {
            assert!(
                GitCommand::Checkout {
                    branch: branch.into()
                }
                .args()
                .is_err(),
                "{branch:?}"
            );
            assert!(
                GitCommand::FetchBranch {
                    branch: branch.into()
                }
                .args()
                .is_err(),
                "{branch:?}"
            );
        }
        assert!(
            GitCommand::CountCommits {
                base: "--all".into(),
                head: "HEAD".into()
            }
            .args()
            .is_err()
        );
        assert_eq!(
            GitCommand::SetBranchConfig {
                branch: "feature/x".into(),
                key: BranchConfigKey::Description,
                value: "--global".into()
            }
            .args()
            .unwrap(),
            ["config", "--", "branch.feature/x.description", "--global"]
        );
        assert_eq!(
            GitCommand::Checkout {
                branch: "feature/x".into()
            }
            .args()
            .unwrap(),
            ["checkout", "--no-overwrite-ignore", "feature/x", "--"]
        );
        assert_eq!(
            GitCommand::FetchBranch {
                branch: "main".into()
            }
            .args()
            .unwrap(),
            [
                "fetch",
                "--no-tags",
                "origin",
                "+refs/heads/main:refs/remotes/origin/main"
            ]
        );
    }
}
