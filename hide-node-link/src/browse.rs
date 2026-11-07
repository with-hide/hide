//! Add a project's folders on a node: one folder under the node's home as
//! the in-window folder list shows it, and where a clone or a new project
//! would land under that home (PRD device-add-project D-03, D-05, D-07, D-08).

use serde::{Deserialize, Serialize};

/// Children a folder list carries at most, the Explorer's cap; a folder with
/// more is answered as truncated, never grown (PRD device-add-project B30).
pub const HOME_LIST_CAP: usize = crate::list::LIST_CAP;

/// The most children of one listed folder read to count the repositories in
/// it; a folder with more carries no count (PRD device-add-project D-05).
pub const REPOSITORY_COUNT_FOLDER_CAP: usize = 200;

/// The most children one answer reads for every count together, so a folder
/// of large folders (`node_modules` beside `node_modules`) costs a bound and
/// the folders past it carry no count.
pub const REPOSITORY_COUNT_ANSWER_CAP: usize = 4000;

/// One folder under the node's home: its real path, the same path as `~`
/// spells it, and its children.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeListing {
    /// The folder with its links resolved, spelled with `/` (the wire
    /// spelling a registration's path uses).
    pub path: String,
    /// `~` or `~/...`, names between `/`.
    pub label: String,
    /// The node's home, spelled like `path`.
    pub home: String,
    pub entries: Vec<HomeEntry>,
    /// More than [`HOME_LIST_CAP`] children were shown; the rest were not read.
    pub truncated: bool,
}

/// A child of a listed folder. Hidden names are never children: the list
/// leaves them out (PRD device-add-project D-03).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeEntry {
    pub name: String,
    pub is_directory: bool,
    /// A Git repository's root.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repository: bool,
    /// The branch a repository has checked out; absent when HEAD is detached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// For a folder that is no repository, how many of its own children are
    /// repositories, when there are any and they could be counted within the
    /// caps; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repositories: Option<u32>,
}

/// What a new folder is for: a clone writes only a folder it makes, while a
/// new project continues into the leftover of a create that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewFolderKind {
    Clone,
    Project,
}

/// Where a new folder would land: `parent` is home or a folder under it,
/// with its links resolved, and `path` its child named as asked.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NewFolderTarget {
    pub parent: String,
    /// `parent` as `~` spells it.
    pub parent_label: String,
    /// The new folder's path; absent when a new project's name is still
    /// empty and only the parent was asked about.
    pub path: Option<String>,
    /// A new project's folder already stands there holding nothing but
    /// `.git`, the leftover of a create that failed; the create continues
    /// into it.
    #[serde(default)]
    pub leftover: bool,
}
