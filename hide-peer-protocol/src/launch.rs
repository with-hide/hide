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
/// The guest's install root: the builds of that guest the host keeps, each
/// in a folder named by the first 16 hex characters of its digest, and the
/// `current` that leads to the one its hooks run. The host build writes the
/// build folders and removes them, and touches nothing else there; the
/// guest build runs from one, points `current` at itself and keeps its
/// holders' version folders below the root as it keeps the account's below
/// a home, apart from the account's own (D-77 (2)).
pub const INSTALL_SIDE: &str = "i";
/// How many builds of one guest a host keeps: the one that runs and the one
/// before it.
pub const BUILDS_KEPT: usize = 2;

/// The name a host gives a guest's installation where it names one without
/// a path (D-55): `guest:` and the guest's short key.
pub const INSTALL_PREFIX: &str = "guest:";

pub fn install_name(guest: &str) -> String {
    format!("{INSTALL_PREFIX}{guest}")
}

/// The short key an installation name carries, when it is a guest's and
/// spelled the one way a folder name is.
pub fn guest_of_install(name: &str) -> Option<&str> {
    let guest = name.strip_prefix(INSTALL_PREFIX)?;
    short_key_spelled(guest).then_some(guest)
}

fn short_key_spelled(name: &str) -> bool {
    name.len() == crate::key::SHORT_KEY_CHARS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// How the name of a guest's installation starts in the account's agent
/// settings (D-77 (4), (5)): the guest's build writes its hook entries under
/// that name and the host's build takes them out by Hide's marker and the
/// same name, so the two builds have to spell it alike. The rest of the
/// name is the guest's install root.
pub const HOOKS_INSTALL_PREFIX: &str = "guest-root:";

/// Whether `root` is spelled as a guest's install root, `<…>/g/<16
/// hex>/i`: the one shape whose installation is a guest's.
pub fn is_install_root(root: &Path) -> bool {
    let named = |folder: &Path, name: &str| folder.file_name().is_some_and(|found| found == name);
    let Some(guest) = root.parent() else {
        return false;
    };
    named(root, INSTALL_SIDE)
        && guest
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(short_key_spelled)
        && guest
            .parent()
            .is_some_and(|guests| named(guests, GUESTS_DIR))
}

/// Whether a node whose `--guest` folder is `node_dir` and whose program is
/// `program` runs from that guest's install root: `<guest folder>/i/<16
/// hex>/<program>`, the guest folder being the one that holds `node_dir`.
/// Only then does a guest-mode node install its hook entries (D-77 (4)); a
/// copy anywhere else installs nothing into the account.
pub fn runs_from_install_root(node_dir: &Path, program: &Path) -> Option<PathBuf> {
    let root = node_dir.parent()?.join(INSTALL_SIDE);
    let build = program.parent()?;
    let named = build.file_name()?.to_str().is_some_and(|name| {
        name.len() == 16
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    (named && build.parent()? == root).then_some(root)
}

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

/// A capability a node names in its ready line: it keeps the event box and
/// speaks [`ToNode::Resume`], [`ToNode::Ack`] and [`FromNode::Event`].
pub const CAP_EVENTS: &str = "events";
/// The capabilities this build's node names.
pub const NODE_CAPABILITIES: [&str; 1] = [CAP_EVENTS];

/// The longest line either side writes the other after the ready line: an
/// event is at most a message of the management stream.
pub const MAX_LINE: usize = crate::frame::MAX_MESSAGE + 1024;

/// How long a node with no live pane and no link session is left running
/// before the host ends it (D-77 (6)).
pub const IDLE_END_SECS: u64 = 60;
/// How long a node told to end now waits for its panes before it exits.
pub const END_NOW_WAITS_SECS: u64 = 5;
/// How often the host tries again to start a node whose folder another node
/// still holds, and how many times, before it counts a failed start.
pub const FOLDER_HELD_RETRY_SECS: u64 = 1;
pub const FOLDER_HELD_RETRIES: u32 = 10;
/// How many guests one host has at work at once (D-77 (7)). A guest is at
/// work while its node runs or a pane of it is left on the host, and that
/// long its hook entries are on the host's account, where every agent
/// event, the host's own among them, starts each guest's helper once. The
/// number keeps what the guests add to one event inside a second on the
/// slowest system Hide supports, for an agent that runs its hooks one after
/// another: a Windows runner printed what one start takes there, about a
/// quarter of a second with the PowerShell an entry runs in
/// (`hide-agent-hooks/tests/it/windows_hook_command.rs`), and
/// `docs/ARCHITECTURE.md`, What a guest may take, has the figures. A guest
/// past it is told [`crate::manage::NodeStop::HostFull`].
pub const MAX_GUEST_NODES: usize = 4;
/// A node that crashed more than [`CRASHES`] times within
/// [`CRASH_WINDOW_SECS`] is not started again by itself (B70).
pub const CRASHES: usize = 5;
pub const CRASH_WINDOW_SECS: u64 = 600;
/// A node whose holder was lost restarts its own service at most once in
/// this long; the host never restarts for it (D-77 (6)).
pub const HOLDER_LOST_RESTART_SECS: u64 = 600;

/// How far a guest's core has durably recorded a box.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub outbox: String,
    pub seq: u64,
}

/// A pane whose report could not be kept because the box is full (B13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxFull {
    /// The pane the report was about, when it was about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// A line a node writes on its standard output after the ready line, one
/// JSON object with one key. A reader skips a line it cannot read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FromNode {
    /// Whether the node has no live pane and no link session, written when
    /// it changes.
    Idle(bool),
    /// One event of the box, after a [`ToNode::Resume`] and in order
    /// ([`CAP_EVENTS`]).
    Event(crate::event::Event),
    /// A report the box had no room for.
    BoxFull(BoxFull),
}

/// A line the host writes on the node's standard input. The end of that
/// input means the host is gone: the node exits and leaves its holders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToNode {
    End(EndMode),
    /// The guest's core is connected and has recorded its box this far: the
    /// node sends every event after it, and each new one as it is made,
    /// until the next resume or [`ToNode::Away`].
    Resume(Position),
    /// The guest's core recorded the box through this event.
    Ack(Position),
    /// The guest's core is gone: the node keeps its events until the next
    /// resume.
    Away,
}

impl FromNode {
    /// `None` for a line this build does not know.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
}

impl ToNode {
    /// `None` for a line this build does not know.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
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
/// A write to the host failed or a thread of the node panicked: the node
/// ends rather than run on unheard. The host counts it as a crash.
pub const EXIT_HOST_UNREACHABLE: i32 = 70;
/// The node ended itself to be started again: a holder of one of its panes
/// died without saying why, and a node's start is what settles such a pane.
/// It asks at most once in [`HOLDER_LOST_RESTART_SECS`]; the host starts it
/// again at once and counts the end with the crashes, so a node that asks
/// without end is left stopped like one that crashes (D-77 (6)).
pub const EXIT_RESTART: i32 = 71;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_one_key_and_an_unknown_one_is_skipped() {
        assert_eq!(
            FromNode::parse(r#"{"idle":true}"#),
            Some(FromNode::Idle(true))
        );
        assert_eq!(
            ToNode::parse(r#"{"end":"after_last_pane"}"#),
            Some(ToNode::End(EndMode::AfterLastPane))
        );
        assert_eq!(
            ToNode::parse(r#"{"resume":{"outbox":"a","seq":7}}"#),
            Some(ToNode::Resume(Position {
                outbox: "a".into(),
                seq: 7
            }))
        );
        assert_eq!(ToNode::parse(r#""away""#), Some(ToNode::Away));
        assert_eq!(
            serde_json::to_string(&FromNode::Idle(false)).unwrap(),
            r#"{"idle":false}"#
        );
        assert_eq!(
            serde_json::to_string(&ToNode::End(EndMode::Now)).unwrap(),
            r#"{"end":"now"}"#
        );
        assert_eq!(ToNode::parse(r#"{"later":1}"#), None);
        assert_eq!(FromNode::parse("not json"), None);
    }

    #[test]
    fn only_a_build_folder_of_the_guests_own_install_root_installs() {
        let node = Path::new("/s/g/0123456789abcdef/n");
        let root = Path::new("/s/g/0123456789abcdef/i");
        assert_eq!(
            runs_from_install_root(node, &root.join("00112233445566ff/hided")),
            Some(root.to_path_buf())
        );
        for elsewhere in [
            "/s/g/0123456789abcdef/i/current/hided",
            "/s/g/0123456789abcdef/i/00112233445566FF/hided",
            "/s/g/fedcba9876543210/i/00112233445566ff/hided",
            "/home/example/.hide/remote-helper/00112233445566ff/hided",
            "/s/g/0123456789abcdef/i/hided",
        ] {
            assert_eq!(
                runs_from_install_root(node, Path::new(elsewhere)),
                None,
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn an_installation_name_carries_a_short_key_and_nothing_else() {
        assert_eq!(install_name("0123456789abcdef"), "guest:0123456789abcdef");
        assert_eq!(
            guest_of_install("guest:0123456789abcdef"),
            Some("0123456789abcdef")
        );
        for other in [
            "app",
            "root:/x",
            "guest:../../etc",
            "guest:0123456789ABCDEF",
            "guest:01",
        ] {
            assert_eq!(guest_of_install(other), None, "{other}");
        }
    }
}
