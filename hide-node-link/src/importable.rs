//! The projects worth adding on a node, for Add a project's importable
//! list (PRD device-add-project D-09 to D-14): the folders the node's
//! agents worked in and the projects Paseo and Herdr keep there, each
//! merged into the project Hide would register, newest first.

use serde::{Deserialize, Serialize};

/// The most projects one answer carries (D-11).
pub const IMPORTABLE_CAP: usize = 20;

/// How far back activity counts: thirty days, Paseo's own window for its
/// project suggestions (D-11).
pub const RECENT_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// The most distinct folders one answer judges as projects, newest first;
/// the rest are not read.
pub const JUDGED_FOLDER_CAP: usize = 400;

/// The most registered projects a request may name to leave out; a longer
/// list is refused rather than cut.
pub const EXCLUDE_CAP: usize = 4096;

/// The largest Paseo project registry read: a registry of a thousand
/// projects stays far below it.
pub const PASEO_READ_CAP: u64 = 4 * 1024 * 1024;

/// An app whose own project list a node reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportApp {
    Paseo,
    Herdr,
}

/// Where a node found a project: an agent's sessions (by the session
/// format's name, `claude`, `codex`, `opencode` ...), or an app's list. It
/// is one string on the wire.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImportSource {
    Agent(hide_session::Agent),
    App(ImportApp),
}

/// A project worth adding: the root a registration of it adds, as `~`
/// spells it, its name, every source that named it, and its newest
/// activity among them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportableProject {
    /// The root in the wire spelling a registration's path uses.
    pub path: String,
    /// `~/...`, names between `/`.
    pub label: String,
    pub name: String,
    pub sources: Vec<ImportSource>,
    pub active_at_unix_ms: u64,
}

/// A source the node could not read, and why; it goes to the log, never to
/// the screen (D-12, B27).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DroppedSource {
    pub source: String,
    pub reason: String,
}

/// A node's answer to `Call::ImportableProjects`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportableProjects {
    /// At most [`IMPORTABLE_CAP`], newest activity first.
    pub projects: Vec<ImportableProject>,
    pub dropped: Vec<DroppedSource>,
}
