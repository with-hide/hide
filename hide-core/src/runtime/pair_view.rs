//! Pair view (PRD lane-desk-pair-view D-08, D-09, D-15, D-16, D-22): an
//! Agent tab drawn as one of its panes beside its tray. It is the tab's
//! state, kept with the tray in `workspace-views.json` (B27); the engine's
//! layout, zoom and focus are not touched, and the shell draws the pane's
//! terminal over them (D-26). Entering puts on the tray, in front, what the
//! pane's agent opened: the View named, else its unseen View, else its last
//! View, else its session record (B17, B18); a pane with none of these has
//! nothing to pair with and nothing changes (B19). It ends on the event, when
//! the left pane leaves the tab (B28, `end_pairs_of_gone_panes`) or when the
//! tray's last tab closes (B26, `close_emptied_lane`); the tray then stays in
//! the lane, which entering opened.

use serde::Deserialize;

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::model::DetailItem;
use crate::workspace_views::{PairView, Place};

/// `pair_view`: the pane drawn on the left of its tab's tray, or none to
/// end the pair view of the tab in front.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairViewPayload {
    #[serde(default)]
    pub(super) pane_id: Option<String>,
    /// The tray tab whose pin was pressed, which comes in front (B17).
    #[serde(default)]
    pub(super) display_id: Option<String>,
}

impl Runtime {
    pub(super) fn set_pair_view(&mut self, payload: PairViewPayload) -> bool {
        let Some(key) = self.front_workspace_key() else {
            crate::diagnostic!(serde_json::json!({
                "component": "pair_view", "kind": "pair_view.no_workspace",
            }));
            return false;
        };
        let Some(pane) = payload.pane_id else {
            let tab = self.front_tab(&key);
            return self.end_pair_view(&key, &tab, "left");
        };
        // A terminal outside every Agent tab, such as an automation run's
        // chat, has no tray to pair with (inventory X02).
        let Some(tab) = self.pane_tab(&key, &pane) else {
            self.set_error(
                "pair_view.no_agent_tab",
                "Pair view shows a pane of an Agent tab beside its tray; this pane is in none",
                false,
            );
            return true;
        };
        let place = Place::Tray(tab.clone());
        let shown = match payload.display_id {
            Some(id) if self.view_display(&key, &id).is_some() => Some(id),
            Some(id) => {
                let error = crate::view_layout::LayoutError::UnknownDisplay(id);
                self.set_error(error.kind(), error.message(), false);
                return true;
            }
            None => self.pair_view_of(&key, &pane),
        };
        match shown {
            Some(id) => {
                if self.view_place(&key, &id).as_ref() != Some(&place)
                    && !self.bring_view_to(&key, &id, &place)
                {
                    let error = crate::view_layout::LayoutError::DisplayLimit;
                    self.set_error(error.kind(), error.message(), false);
                    return true;
                }
                let _ = self.change_view_layout(&key, &place, |layout, stamp| {
                    layout.focus(&id, stamp).map(|changed| ((), changed))
                });
            }
            None => {
                let Some(session) = self
                    .live_sessions()
                    .into_iter()
                    .find(|(live, _)| *live == pane)
                    .map(|(_, session)| session)
                else {
                    crate::diagnostic!(serde_json::json!({
                        "component": "pair_view", "kind": "pair_view.nothing_to_show",
                        "device": key.0, "pane": pane,
                    }));
                    return false;
                };
                self.open_card_on(&key, &place, DetailItem::Record { session });
            }
        }
        self.open_tray_for(&key, &place);
        if let Some(store) = self.workspace_views.as_mut() {
            store.views.entry(&key.0, &key.1).tray_entry(&tab).pair =
                Some(PairView { pane: pane.clone() });
            store.generation += 1;
        }
        self.persist_workspace_views();
        if self.front_tab(&key) != tab
            && self.local_tab_pane(&pane)
        {
            self.focus_pane(pane, PaneFocusOrigin::Operator, None);
        }
        true
    }

    /// Ends `tab`'s pair view; whether it was in one.
    pub(super) fn end_pair_view(&mut self, key: &WorkspaceKey, tab: &str, cause: &str) -> bool {
        let Some(pair) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
            .and_then(|view| view.tray_mut(tab))
            .and_then(|tray| tray.pair.take())
        else {
            return false;
        };
        crate::diagnostic!(serde_json::json!({
            "component": "pair_view", "kind": "pair_view.ended",
            "device": key.0, "tab": tab, "pane": pair.pane, "cause": cause,
        }));
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation += 1;
        }
        self.persist_workspace_views();
        true
    }

    /// The View of `pane`'s agent that pairs with it: the newest it opened
    /// that the operator has not looked at, else the one it opened last.
    fn pair_view_of(&self, key: &WorkspaceKey, pane: &str) -> Option<String> {
        let scope = self.session_scope_of_pane(pane)?;
        let own: Vec<&crate::view_layout::Display> = self
            .view_displays(key)
            .filter(|display| display.openers.iter().any(|o| o.of_session(&scope)))
            .collect();
        own.iter()
            .filter(|display| display.unseen.is_some())
            .max_by_key(|display| display.opened_stamp)
            .or_else(|| own.iter().max_by_key(|display| display.opened_stamp))
            .map(|display| display.id.clone())
    }

    /// In pair view, a tray tab the operator brought in front that another
    /// agent of the tab opened takes that agent's pane to the left; a tab
    /// nobody's agent opened leaves the left pane (B24, D-21).
    pub(super) fn follow_pair_front(&mut self, key: &WorkspaceKey, display_id: &str) {
        let Some(Place::Tray(tab)) = self.view_place(key, display_id) else {
            return;
        };
        let paired = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .and_then(|view| view.tray(&tab))
            .and_then(|tray| tray.pair.as_ref())
            .map(|pair| pair.pane.clone());
        let Some(left) = paired else {
            return;
        };
        let Some(pane) = self
            .view_display(key, display_id)
            .and_then(|display| display.opener.as_ref())
            .and_then(|opener| self.view_opener_snapshot(opener).pane_id)
            .filter(|pane| *pane != left && self.pane_tab(key, pane).as_deref() == Some(tab.as_str()))
        else {
            return;
        };
        if let Some(tray) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
            .and_then(|view| view.tray_mut(&tab))
        {
            tray.pair = Some(PairView { pane });
        }
        self.persist_workspace_views();
    }
}
