//! How a host starts and ends the guest node process, and where that node
//! keeps its state (PRD D-61, D-77).
//!
//! A guest's work runs on the host in a process of the guest's own build,
//! `hided node serve --guest <dir>`. The two builds differ, so everything
//! the host build knows about that process is here and nowhere else.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The words after the program's path; the node's state folder follows.
pub const ARGUMENTS: [&str; 3] = ["node", "serve", "--guest"];

/// The version of the ready line.
pub const GUEST_NODE: u32 = 1;

/// The folder under a host's state folder that holds every guest's folder,
/// each named by the first [`crate::key::SHORT_KEY_CHARS`] hex characters of
/// the guest's key. The names are this short because every endpoint below
/// them has to fit a Unix socket path (see [`LONGEST_SOCKET_TAIL`]).
pub const GUESTS_DIR: &str = "g";
/// The file in a guest's folder that holds its whole key.
pub const KEY_FILE: &str = "key";
/// What the host build alone writes: `panes.json`.
pub const HOST_SIDE: &str = "h";
/// The guest build's state: the node's `--guest` folder. The host build
/// never reads inside it.
pub const NODE_SIDE: &str = "n";

/// The endpoints a node binds in its folder and names in its ready line.
pub const LINK_ENDPOINT: &str = "link.sock";
pub const TAP_ENDPOINT: &str = "tap.sock";

/// The most bytes any endpoint of a guest-mode node takes below its
/// `--guest` folder, separator included. A node whose endpoint would be
/// longer exits [`EXIT_FOLDER_REFUSED`].
pub const NODE_SOCKET_TAIL: usize = 47;

/// The most bytes any guest endpoint takes below the host's state folder:
/// `/g/<16 hex>/n` and [`NODE_SOCKET_TAIL`]. With macOS's 103 byte socket
/// path it leaves room for an account name of 16 bytes, the state folder
/// being `.hide/state` in that account's home. The host checks it before it starts a
/// node and refuses with the reason instead (D-77 (3)).
pub const LONGEST_SOCKET_TAIL: usize =
    1 + GUESTS_DIR.len() + 1 + crate::key::SHORT_KEY_CHARS + 1 + NODE_SIDE.len() + NODE_SOCKET_TAIL;

/// A path as long as the longest endpoint a guest's node may bind under
/// `state_dir`. A host hands it to its platform's socket path check before
/// it starts the node, so the one owner of that limit decides.
pub fn longest_endpoint(state_dir: &Path) -> PathBuf {
    state_dir.join("x".repeat(LONGEST_SOCKET_TAIL - 1))
}

/// The one line a node writes on its standard output once both endpoints
/// are bound. The host waits [`READY_WITHIN_SECS`] for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    pub guest_node: u32,
    pub build: String,
    pub build_id: String,
    pub pid: u32,
    /// The endpoints' names inside the node's folder.
    pub link: String,
    pub tap: String,
    /// The oldest and newest tap version the node speaks.
    pub tap_versions: [u16; 2],
    #[serde(default)]
    pub capabilities: Vec<String>,
}

pub const READY_WITHIN_SECS: u64 = 10;

/// What a node writes on its standard output after the ready line, when it
/// changes: no live pane and no link session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Idle {
    pub idle: bool,
}

/// A line the host writes on the node's standard input. The end of that
/// input means the host is gone: the node exits and leaves its holders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct End {
    pub end: EndMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndMode {
    /// End every pane, then exit.
    Now,
    /// Take no new link session and exit when the last pane ends.
    AfterLastPane,
}

/// Ended as asked, or the host's input ended.
pub const EXIT_ENDED: i32 = 0;
/// A word after `serve` the node does not know: the build is too old or too
/// new for this host (`update_needed`), not a crash.
pub const EXIT_UNKNOWN_WORD: i32 = 64;
/// The folder or an endpoint could not be made.
pub const EXIT_FOLDER_REFUSED: i32 = 73;
/// Another node still holds the folder; the host tries again shortly.
pub const EXIT_FOLDER_HELD: i32 = 75;

/// What a host answers a folder-proved call that belongs to a guest: the
/// installation whose own `hide` takes it (D-55). The host names the
/// installation, never a path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandedTo {
    pub handed_to: Installation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    pub install: String,
}
