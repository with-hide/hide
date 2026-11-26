//! The places the operator has been, newest first: the order Ctrl+Tab walks
//! outside a View area (issue 301): the terminal panes the keyboard has been
//! in, the missions and automations whose detail was in front, and Desk.
//!
//! The shell decides what a visit is, because only the page knows where the
//! keyboard is (moving between a checkout's terminal and its View area
//! changes no core state) and which screen it shows, and reports each one
//! with `place_visit`. The core keeps the order in `ui_state.recent_places`
//! and saves it with the rest of the UI state, so it outlives a reload, the
//! app and the daemon.
//!
//! A record leaves only past `RECENT_PLACE_LIMIT`, never because its place is
//! not listed: after a restart the panes and the devices' sessions arrive
//! after the first frame, so pruning against them would empty the list on
//! every start. The shell leaves out a place it cannot draw.

use super::*;
use crate::model::{RECENT_PLACE_LIMIT, RecentPlace};
use serde::Deserialize;

/// `place_visit`: the place the operator is on now.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlaceVisitPayload {
    pub(super) place: RecentPlace,
}

impl Runtime {
    /// Moves `place` to the head of the recent place order. A place the core
    /// does not hold (a pane no listed tab holds, a mission or automation its
    /// book does not keep) is refused into the diagnostic log: the shell
    /// only reports a place it draws, so one the core does not hold is a
    /// stale report the operator cannot act on.
    pub(super) fn record_place_visit(&mut self, place: RecentPlace) -> bool {
        if !self.place_held(&place) {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_places",
                "kind": "visit.unheld_place",
                "place": place,
            }));
            return false;
        }
        let recent = &mut self.snapshot.ui_state.recent_places;
        if recent.first() == Some(&place) {
            return false;
        }
        recent.retain(|held| *held != place);
        recent.insert(0, place);
        recent.truncate(RECENT_PLACE_LIMIT);
        self.persist_ui_state();
        true
    }

    /// Whether the core holds what `place` names now.
    fn place_held(&self, place: &RecentPlace) -> bool {
        match place {
            RecentPlace::Pane { pane_id } => self.pane_listed(pane_id),
            RecentPlace::Mission { mission_id } => self.missions.book.get(mission_id).is_some(),
            RecentPlace::Automation { automation_id } => self.automation_held(automation_id),
            RecentPlace::Desk => true,
        }
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
