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
    /// The engine took the words and the Enter for the pane.
    Sent,
    /// The agent was at work, so the words wait in the pane's line
    /// (`queue`), typed at a later rest (PRD desk-queue B1).
    Queued,
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
    /// The record file the page was read from (its incarnation), and where
    /// the page begins in it, a byte offset: together the page's name, which
    /// reading a page before it, the live page filling and joining the kept
    /// ones, or the oldest kept page being let go never changes, and which a
    /// record replaced at the same path never shares, so a screen keys a line
    /// by them and the line's place in the page.
    pub record: String,
    pub start: u64,
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
    /// The operator's words that wait for the pane's agent to rest (PRD
    /// desk-queue); none while it holds none.
    pub queue: Option<InputQueueSnapshot>,
    /// The model and effort the session runs with as Hide knows them: what
    /// its start named, or the last a session input changed; each absent
    /// for the CLI's own (docs/ARCHITECTURE.md, A session's model and
    /// effort).
    pub choice: hide_agent_adapter::LaunchChoice,
}

/// Whether a pane's waiting words go at its agent's rests
/// (`input_queue_state`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputQueueState {
    /// `대기 중`: one goes each time the agent rests.
    Waiting,
    /// `멈춤`: none goes until the operator sends one or resumes.
    Paused,
}

/// A pane's waiting words, as every screen draws them above its input
/// (PRD desk-queue D-05).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputQueueSnapshot {
    pub state: InputQueueState,
    /// Oldest first, the order they go in.
    pub lines: Vec<InputQueueLine>,
    /// How many the line holds (`input_queue::LINES_MAX`).
    pub max: usize,
    /// The line holds `max`: a send now is refused.
    pub full: bool,
    /// The line typed last opened no turn the doorbell could see, so the
    /// next waits for `▶ 이어 보내기` or the agent's next move.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stalled: bool,
    /// What `모두 지우기` took, while `되돌리기` can bring it back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleared: Option<InputQueueCleared>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputQueueLine {
    pub id: u64,
    pub text: String,
    /// The names of the files it attaches.
    pub files: Vec<String>,
    /// On its way to the pane: it has no ✕.
    pub sending: bool,
    /// Why it did not go (`input_queue_failure`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<crate::input_queue::Failure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InputQueueCleared {
    pub count: usize,
    /// When `되돌리기` stops being offered.
    pub until_unix_ms: u64,
    /// Why the last `되돌리기` could not bring them back now (`queue_full`,
    /// `queue_files_full`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<&'static str>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SessionThreadHistorySnapshot {
    pub generation: u64,
    /// The whole pages before the tail, oldest first.
    pub pages: Vec<Arc<ThreadPageSnapshot>>,
}
