//! The Home folder after a sync.

use serde::{Deserialize, Serialize};

/// The most project links one Home holds.
pub const MAX_LINKS: usize = 256;

/// The Home folder after a sync.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeSynced {
    /// Absolute, canonical path of `<user home>/hide`, in wire spelling (`/`).
    pub home: String,
    /// This call made the Home folder.
    pub created: bool,
    /// Every link Hide manages after the sync, sorted by name.
    pub links: Vec<HomeLink>,
    /// Managed links this sync removed.
    pub dropped: Vec<HomeDrop>,
    /// Requested projects that got no link.
    pub skipped: Vec<HomeSkip>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeLink {
    pub name: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeDrop {
    pub name: String,
    pub target: String,
    /// `unregistered` (the project left the list) or `dangling` (its folder
    /// is gone).
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HomeSkip {
    pub target: String,
    /// `missing` (not a folder now), `name_taken` (something of the
    /// operator's holds the link's name), `link_failed` (the disk refused the
    /// link), `not_absolute`, or `no_name` (the path has no final component).
    pub reason: String,
}
