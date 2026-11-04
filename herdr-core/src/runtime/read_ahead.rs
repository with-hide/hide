//! Panes a read of Herdr's own session showed before Herdr's event stream did
//! (with-hide/hide-private#464).
//!
//! A worker that changed Herdr can read the session itself and hand that read
//! to the core (a project's creation does), and the read can be ahead of the
//! event stream: the stream has no ordering cursor against it, so the next
//! session the stream publishes can still describe Herdr from before the read.
//! A workspace held back while its first layout settles, or not announced yet,
//! is simply absent there. That absence is not a close. A pane the read laid
//! out and the stream has not keeps its terminal, the size its view reported
//! and its held keys until the stream first lays it out, the bound passes, or
//! the connection to Herdr is lost; only then does its absence retire it.
//!
//! A session the sync published carries its tab moves
//! ([`SessionSnapshotPayload::tab_moves`]) and a read of its own has none,
//! which is how the two are told apart.

use crate::sidebar::SessionSnapshotPayload;

/// How many panes can wait for the stream at once; the oldest is dropped past
/// it and logged (engineering 15).
const READ_AHEAD_LIMIT: usize = 64;
/// How long a pane the stream never lays out is kept; the same bound as every
/// operation stage.
const READ_AHEAD_TIMEOUT_MS: u64 = super::CLOSE_STAGE_TIMEOUT_MS;

#[derive(Debug, Default)]
pub(super) struct ReadAhead {
    /// Each pane with when the read showed it, oldest first.
    pub(super) panes: Vec<(String, u64)>,
}

impl ReadAhead {
    /// Notes what `payload` lays out. A read of its own records each pane it
    /// lays out that the core does not lay out yet (`laid_out`, read before
    /// the payload is applied); a session the stream published ends the
    /// record of every pane it lays out. A record past its bound ends either
    /// way.
    pub(super) fn observe(
        &mut self,
        payload: &SessionSnapshotPayload,
        laid_out: impl Fn(&str) -> bool,
        now_unix_ms: u64,
    ) {
        self.panes.retain(|(pane_id, read_at)| {
            let live = now_unix_ms.saturating_sub(*read_at) < READ_AHEAD_TIMEOUT_MS;
            if !live {
                crate::diagnostic!(serde_json::json!({
                    "component": "terminal",
                    "kind": "terminal.read_ahead_expired",
                    "pane_id": pane_id,
                }));
            }
            live
        });
        let shown = payload
            .layouts
            .iter()
            .flat_map(|layout| &layout.panes)
            .map(|pane| pane.pane_id.as_str());
        if payload.tab_moves.is_some() {
            let shown = shown.collect::<std::collections::HashSet<_>>();
            self.panes
                .retain(|(pane_id, _)| !shown.contains(pane_id.as_str()));
            return;
        }
        let mut dropped = Vec::new();
        for pane_id in shown {
            if laid_out(pane_id) || self.holds(pane_id) {
                continue;
            }
            if self.panes.len() >= READ_AHEAD_LIMIT {
                dropped.push(self.panes.remove(0).0);
            }
            self.panes.push((pane_id.to_owned(), now_unix_ms));
        }
        if !dropped.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "terminal",
                "kind": "terminal.read_ahead_dropped",
                "pane_ids": dropped,
                "limit": READ_AHEAD_LIMIT,
            }));
        }
    }

    pub(super) fn holds(&self, pane_id: &str) -> bool {
        self.panes.iter().any(|(held, _)| held == pane_id)
    }

    /// Herdr's pane ids are its own server's, and a later connection may hand
    /// one to a pane no read showed.
    pub(super) fn clear(&mut self) {
        self.panes.clear();
    }
}
