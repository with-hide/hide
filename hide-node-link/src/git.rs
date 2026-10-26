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
    /// The newest `count` commits (at most [`LOG_COUNT_MAX`]), of the
    /// repository or of one repository-relative path, one a line: short
    /// commit, date and subject. A Hide AI suggestion investigation reads
    /// it once a day.
    Log {
        path: Option<String>,
        count: u32,
    },
    /// The lines of tracked text files holding `text`, as fixed text with
    /// case ignored, at most [`GREP_PER_FILE`] a file, under one
    /// repository-relative path when named: `path NUL line NUL text` a
    /// line, at most [`GREP_OUTPUT_BYTES`]. No match answers nothing.
    Grep {
        text: String,
        path: Option<String>,
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
                let path = repository_path(path).map_err(|_| "invalid_document_path")?;
                owned(&[
                    "--literal-pathspecs",
                    "log",
                    "-1",
                    "--format=%ct",
                    "--",
                    path,
                ])
            }
            Self::Log { path, count } => {
                if !(1..=LOG_COUNT_MAX).contains(count) {
                    return Err("invalid_log_count".into());
                }
                let count = count.to_string();
                let mut args = vec![
                    "--literal-pathspecs",
                    "log",
                    "--no-color",
                    "-n",
                    &count,
                    "--date=short",
                    "--format=%h %ad %s",
                    "--",
                ];
                if let Some(path) = path {
                    args.push(repository_path(path)?);
                }
                owned(&args)
            }
            Self::Grep { text, path } => {
                if text.is_empty()
                    || text.chars().count() > GREP_TEXT_CHARS
                    || text.chars().any(char::is_control)
                {
                    return Err("invalid_grep_text".into());
                }
                let per_file = GREP_PER_FILE.to_string();
                // `-e` keeps a text that begins with `-` a pattern.
                let mut args = vec![
                    "--literal-pathspecs",
                    "grep",
                    "--no-color",
                    "-n",
                    "-z",
                    "-I",
                    "-F",
                    "-i",
                    "--max-count",
                    &per_file,
                    "-e",
                    text,
                    "--",
                ];
                if let Some(path) = path {
                    args.push(repository_path(path)?);
                }
                owned(&args)
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

/// The most commits [`GitCommand::Log`] answers.
pub const LOG_COUNT_MAX: u32 = 50;
/// The longest text [`GitCommand::Grep`] looks for, in characters.
pub const GREP_TEXT_CHARS: usize = 200;
/// The most lines [`GitCommand::Grep`] answers from one file.
pub const GREP_PER_FILE: u32 = 5;
/// The most bytes of output [`GitCommand::Grep`] answers; the rest is cut.
pub const GREP_OUTPUT_BYTES: u64 = 64 * 1024;

/// A path below the repository's root that a command names after `--`:
/// relative, no `..`, no backslash and no NUL.
pub fn repository_path(path: &str) -> Result<&str, String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part == "..")
        || path.contains('\0')
    {
        return Err("invalid_repository_path".into());
    }
    Ok(path)
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

    /// A history or a search names its text and path, never an argument:
    /// a text that begins with `-` stays the pattern, a path stays below the
    /// root, and the counts are bounded.
    #[test]
    fn a_history_or_search_keeps_its_text_a_pattern_and_its_path_below_the_root() {
        let grep = GitCommand::Grep {
            text: "--output=/tmp/x".into(),
            path: Some("scripts".into()),
        }
        .args()
        .unwrap();
        let at = grep.iter().position(|arg| arg == "-e").unwrap();
        assert_eq!(grep[at + 1], "--output=/tmp/x");
        assert_eq!(grep[grep.len() - 2..], ["--", "scripts"]);
        for path in ["../outside", "/etc", "a\\b", ""] {
            let grep = GitCommand::Grep {
                text: "x".into(),
                path: Some(path.into()),
            };
            let log = GitCommand::Log {
                path: Some(path.into()),
                count: 5,
            };
            assert!(grep.args().is_err() && log.args().is_err(), "{path:?}");
        }
        for count in [0, LOG_COUNT_MAX + 1] {
            assert!(GitCommand::Log { path: None, count }.args().is_err());
        }
        assert!(
            GitCommand::Grep {
                text: "a\nb".into(),
                path: None
            }
            .args()
            .is_err()
        );
        assert_eq!(
            GitCommand::Log {
                path: None,
                count: 3
            }
            .args()
            .unwrap()
            .last()
            .map(String::as_str),
            Some("--")
        );
    }
}
