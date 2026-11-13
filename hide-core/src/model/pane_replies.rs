//! What became of the replies the operator sent agents from a screen that
//! does not show their session (`pane_reply`): the Overview's 나를 기다리는
//! 것 and its session detail. One entry per pane, the newest reply's, each
//! named by its request id, so the field that sent it reads its own answer
//! and an older one is never taken for it.

use std::collections::BTreeMap;

use serde::Serialize;

use super::ThreadInputSnapshot;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct PaneRepliesSnapshot {
    /// By core pane id; a pane Hide no longer observes is dropped at the
    /// next reply, so the section holds at most one entry per live agent.
    pub replies: BTreeMap<String, ThreadInputSnapshot>,
}
