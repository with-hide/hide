//! The thread a screen shows of one agent pane's session (PRD
//! session-docs-and-assistant B25-B29): its record read as ACP session
//! updates, the `hide` calls in it named as the Hide parts they made, and
//! what became of the words its input box sent to the pane.
//!
//! Two sections carry it, so a session that works re-sends only what it
//! wrote: `session_thread` holds the thread's state and the page the
//! session is still writing, `session_thread_history` the whole pages
//! before it, which change only when one is read or the live page fills.

use std::collections::BTreeMap;
use std::sync::Arc;

use hide_session::thread::HideCall;
use serde::Serialize;

/// Where the thread's read stands (`contracts/snapshot-wire-enums.json`:
/// `session_thread_state`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    /// No thread is open.
    #[default]
    Closed,
    /// The first page is being read.
    Loading,
    Ready,
    /// The record could not be read; `failure` says why.
    Failed,
}

/// Why a thread's record could not be read, the one line the screen words
/// beside `터미널로` (B27; `session_thread_failure`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadFailure {
    /// The session has no record Hide can read: none was reported, its file
    /// is gone, or its agent has no thread reader.
    NoRecord,
    /// The record is there and its reader refused it.
    Refused,
}

/// Whether the pane still runs the session the thread was opened for
/// (`session_thread_pane`). Input goes only to the same session.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadPane {
    #[default]
    Same,
    /// The pane now runs another session, terminal or agent.
    Changed,
    /// The pane is no longer there.
    Gone,
}

/// Where the read of the page before the oldest one stands
/// (`session_thread_older`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadOlder {
    #[default]
    Idle,
    Loading,
    /// The thread already holds as many pages as it keeps.
    Capacity,
    Failed,
}

/// What became of the words the thread's input box sent
/// (`session_thread_input_state`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadInputState {
    Sending,
    /// Herdr took the words and the Enter for the pane.
    Sent,
    /// Nothing was typed into the pane; `reason` says why.
    Refused,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThreadInputSnapshot {
    pub request_id: String,
    pub state: ThreadInputState,
    /// A stable code the screen words, for a refusal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One page of the thread: ACP session updates as the reader wrote them
/// (`sessionUpdate` names the variant, fields in camelCase), each `hide`
/// call on it by the tool call id it replaces, and where the sessions the
/// page names run.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ThreadPageSnapshot {
    pub updates: Vec<serde_json::Value>,
    pub parts: BTreeMap<String, HideCall>,
    /// Each session a letter or a spawn on the page names (its registration
    /// id or name, as written) that a live registration answers to, by that
    /// name: a chip draws the session's row (B26). A name no live
    /// registration answers to is left out, and the chip draws the name.
    pub sessions: BTreeMap<String, ThreadSession>,
}

/// Where a session a thread names runs: its pane as the agent rows key it,
/// on its machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThreadSession {
    pub pane_id: String,
    pub machine: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SessionThreadSnapshot {
    /// Advances with every open, so a screen tells a new thread from the
    /// one it drew and an older page or input answers only this one.
    pub generation: u64,
    pub state: ThreadState,
    /// The pane the thread was opened for, as the core names it (a device's
    /// pane as `remote:<device>:pane:<id>`); `None` while closed.
    pub pane_id: Option<String>,
    /// The machine whose node reads the record.
    pub machine: Option<String>,
    pub failure: Option<ThreadFailure>,
    pub pane: ThreadPane,
    /// Whether that machine's node can be reached now; a thread whose node
    /// is not keeps what it read, dimmed (B23).
    pub connected: bool,
    /// Whether the session holds records before the oldest page read.
    pub has_older: bool,
    pub older: ThreadOlder,
    /// The page the session is still writing, read again as it writes.
    pub tail: Option<Arc<ThreadPageSnapshot>>,
    pub input: Option<ThreadInputSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SessionThreadHistorySnapshot {
    pub generation: u64,
    /// The whole pages before the tail, oldest first.
    pub pages: Vec<Arc<ThreadPageSnapshot>>,
}
