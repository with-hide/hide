//! What became of the replies the operator sent agents from a screen that
//! does not show their session (`pane_reply`): the Overview's 나를 기다리는
//! 것 and its session detail. One receipt per request, named by its request
//! id, so each field that sent one reads its own answer whatever else was
//! sent since, from this window or another.

use serde::Serialize;

use super::ThreadInputState;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PaneRepliesSnapshot {
    /// Oldest first, at most `runtime::pane_reply::RECEIPTS`.
    pub receipts: Vec<PaneReplyReceipt>,
}

/// One reply's answer: `sending`, then `sent` or `refused` with a stable
/// code the screen words.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PaneReplyReceipt {
    pub request_id: String,
    pub pane_id: String,
    pub state: ThreadInputState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
