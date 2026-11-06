//! The terminal panes the keyboard has been in, newest first: the order the
//! Agent area's Ctrl+Tab walks (issue 301).
//!
//! The shell decides what a visit is, because only the page knows where the
//! keyboard is (moving between a checkout's terminal and its View area changes
//! no core state), and reports each one with `pane_visit`. The core keeps the
//! order in `ui_state.recent_pane_ids` and saves it with the rest of the UI
//! state, so it outlives a reload, the app and the daemon.
//!
//! A record leaves only past `RECENT_PANE_LIMIT`, never because its pane is
//! not listed: after a restart the panes and the devices' sessions arrive
//! after the first frame, so pruning against them would empty the list on
//! every start. The shell leaves out a pane no listed tab holds.

use super::*;
use crate::model::RECENT_PANE_LIMIT;

impl Runtime {
    /// Moves `pane_id` to the head of the recent pane order. A pane no listed
    /// tab holds is refused into the diagnostic log: the shell only reports a
    /// pane it draws, so one the core does not list is a stale report the
    /// operator cannot act on.
    pub(super) fn record_pane_visit(&mut self, pane_id: String) -> bool {
        if !self.pane_listed(&pane_id) {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_panes",
                "kind": "visit.unlisted_pane",
                "pane_id": pane_id,
            }));
            return false;
        }
        let recent = &mut self.snapshot.ui_state.recent_pane_ids;
        if recent.first() == Some(&pane_id) {
            return false;
        }
        recent.retain(|held| *held != pane_id);
        recent.insert(0, pane_id);
        recent.truncate(RECENT_PANE_LIMIT);
        self.persist_ui_state();
        true
    }

    /// Whether a tab the shell can draw holds `pane_id`: this machine's
    /// checkouts, or a connected device's, whose pane ids are already scoped
    /// to the device.
    pub(super) fn pane_listed(&self, pane_id: &str) -> bool {
        let holds = |workspaces: &[WorkspaceSnapshot]| {
            workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .flat_map(|tab| &tab.panes)
                .any(|pane| pane.id == pane_id)
        };
        holds(&self.snapshot.navigator.workspaces)
            || self
                .snapshot
                .status
                .remote
                .iter()
                .filter_map(|remote| remote.session.as_ref())
                .any(|session| holds(&session.workspaces))
    }
}
