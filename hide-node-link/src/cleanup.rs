//! What a node answers for a reviewed cleanup of a repository on its
//! machine: the real path of a folder, the judgement of a folder chosen for
//! emptying made again from its files right before the move, and a clean
//! worktree removal with the trash entries it left. The core decides which
//! worktree and folder may go; the node reads and moves them.

use serde::{Deserialize, Serialize};

use crate::disk::{FolderRefusal, Layer};
use crate::worktrees::RemovalOutcome;

/// A path as the node's file system resolves it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PathState {
    /// The path with every link and alias resolved.
    Real {
        path: String,
    },
    Missing,
    /// It may exist, but the node could not read it.
    Unreadable {
        reason: String,
    },
}

/// A repository's folders as its `.git` names them, read from the files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RepositoryDirs {
    /// The checkout that holds the path.
    pub root: String,
    /// This checkout's own Git directory.
    pub git_dir: String,
    /// The Git directory every worktree of the repository shares.
    pub common_dir: String,
}

/// What is left of a cleanup run's look through folders for a nested
/// repository. The core carries it from one checkout's judgement to the
/// next, so a run over many big folders is bounded as a whole.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WalkAllowance {
    pub entries: u64,
    pub millis: u64,
}

/// One measured folder chosen for emptying.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderToJudge {
    pub layer: Layer,
    pub path: String,
}

/// A chosen folder judged again from its files.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum FolderVerdict {
    /// Still ignored, still its layer, reached through no link, holding no
    /// repository.
    Fits,
    Refused {
        refusal: FolderRefusal,
    },
    NestedRepository,
    /// The look for a nested repository could not finish.
    Unverified {
        reason: String,
    },
}

/// The verdicts in the order the folders were named, and what is left of
/// the run's allowance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FolderJudgments {
    pub verdicts: Vec<FolderVerdict>,
    pub walk_left: WalkAllowance,
}

/// A clean worktree's removal, without force, and the entries it put in the
/// repository's trash, which the run deletes and waits for.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanRemoval {
    pub outcome: RemovalOutcome,
    pub trashed: Vec<String>,
}
