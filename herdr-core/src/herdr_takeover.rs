//! Moving the operator's own older Herdr server on this machine's socket onto
//! the Herdr this app ships (PRD herdr-takeover).
//!
//! The core decides what the screen offers and when; the work itself (who
//! runs the server, its ping, the handoff and its confirmation, the record
//! of handoffs that failed) is `hided`'s, which installs a working
//! [`HerdrTakeover`] only in a daemon that runs from a desktop package with
//! that package's own Herdr and one that finds nothing (`NoTakeover`) in any
//! other (`hided/src/herdr_handoff.rs`). Every call blocks on a socket or a
//! child process, so the core makes it on a worker of its own, never under
//! the runtime lock.

/// This machine's Herdr server against the Herdr this app ships.
pub trait HerdrTakeover: Send + Sync {
    /// What the server on the socket is and what Hide can offer for it.
    fn look(&self) -> Look;
    /// Has the server hand its panes to this app's Herdr and confirms it
    /// runs it afterwards (`server.live_handoff`, Unix only).
    fn hand_over(&self) -> Moved;
}

/// One look at the server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Look {
    /// No server, this app's own Herdr, the pinned release, a newer release
    /// on the same protocol, or another Hide app's server that `hide
    /// connect` moves without asking (PRD herdr-takeover B14, B15).
    Nothing,
    Offer(Offer),
    /// The server or this app's Herdr could not be read; the log keeps why,
    /// and the screen offers nothing it could not act on.
    Unreadable {
        stage: String,
        detail: String,
    },
}

/// What the operator is offered for the server on the socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Offer {
    pub kind: OfferKind,
    /// The server's process, for the log; `None` where the system does not
    /// say which process answers a socket.
    pub server_pid: Option<u32>,
    pub server_version: String,
    pub server_protocol: u64,
    /// The release this app ships.
    pub version: String,
    /// An earlier handoff of this same server process to this same pin
    /// failed, so the step opens on its failure (D-23).
    pub failed_before: Option<FailureCause>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfferKind {
    /// An older stable release that can hand its panes over.
    Move,
    /// An older stable release that cannot (Herdr on Windows): it is stopped
    /// and the desktop host starts this app's Herdr, after the operator
    /// confirmed what ends (D-13).
    Restart,
    /// A newer protocol than this build speaks: only a newer Hide reads it.
    UpdateHide,
}

impl OfferKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Restart => "restart",
            Self::UpdateHide => "update_hide",
        }
    }
}

/// Why a handoff did not move the server, as the screen names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCause {
    /// Herdr refused the handoff or did not finish it in time; the old
    /// server stays as it was.
    Refused,
    /// The handoff answered, but the server on the socket does not run this
    /// app's Herdr afterwards.
    NotConfirmed,
    /// The server, this app's Herdr or the record could not be read.
    Unreadable,
}

impl FailureCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::NotConfirmed => "not_confirmed",
            Self::Unreadable => "unreadable",
        }
    }
}

/// What one handoff came to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Moved {
    /// The panes belong to a server running this app's Herdr.
    Done {
        from_pid: u32,
        to_pid: u32,
        elapsed_ms: u64,
        /// The `herdr` the operator's own terminal finds, when its protocol
        /// is older than the server's now: its commands, and those of an
        /// agent in a pane that started under the old server, are refused
        /// until it is updated (B5, D-12).
        path_herdr_behind: Option<String>,
    },
    Failed {
        cause: FailureCause,
        stage: String,
        detail: String,
    },
}
