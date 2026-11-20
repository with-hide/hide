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
use crate::links::ledger::SessionIdentity;
use crate::model::DetailItem;
use crate::workspace_views::{PairView, Place};

/// Why a pair view ended, which the diagnostic names and which decides
/// whether the keyboard goes back to the pane that held it before (B26).
#[derive(Clone, Copy, Debug)]
pub(super) enum PairEnd {
    /// The operator left: the strip's way back, ⌘., the pressed ◧.
    Left,
    /// The desk's last tab closed or moved away.
    Emptied,
    /// The left pane closed or left the tab (B28).
    PaneGone,
    /// The operator chose a pane of the tab with nothing to show (B25).
    ChoseUnpaired,
}

impl PairEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Emptied => "emptied",
            Self::PaneGone => "pane_gone",
            Self::ChoseUnpaired => "chose_unpaired_pane",
        }
    }

    /// The operator leaving, or closing the desk's last tab, returns the
    /// keyboard; a pane that went, or another chosen, leaves it.
    fn returns_keyboard(self) -> bool {
        matches!(self, Self::Left | Self::Emptied)
    }
}

/// What pair view puts on the desk beside a pane (B17, B19): the View its
/// agent opened, else its session's record. The one rule `pairable`, the
/// entries and the operator's choice of another pane read.
enum PairContent {
    View(String),
    Record(SessionIdentity),
}

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
        self.set_pair_view_with(payload, true)
    }

    /// `set_pair_view`, giving the left pane the keyboard when `focus`: the
    /// pane on screen is the one the keys, ⇧⌘W and the text scale reach.
    /// The operator's own focus of that pane (`follow_pair_focus`) passes
    /// false, since the keyboard is already there.
    fn set_pair_view_with(&mut self, payload: PairViewPayload, focus: bool) -> bool {
        let Some(key) = self.front_workspace_key() else {
            crate::diagnostic!(serde_json::json!({
                "component": "pair_view", "kind": "pair_view.no_workspace",
            }));
            return false;
        };
        let Some(pane) = payload.pane_id else {
            let tab = self.front_tab(&key);
            return self.end_pair_view(&key, &tab, PairEnd::Left);
        };
        // A terminal outside every Agent tab, such as an automation run's
        // chat, has no tray to pair with (inventory X02); a pane of another
        // Workspace pairs only once that Workspace is in front.
        let Some(tab) = self.pane_tab(&key, &pane) else {
            if self.pane_workspace_of(&pane).is_some() {
                self.set_error(
                    "pair_view.other_workspace",
                    "Pair view shows a pane beside the desk of its own Workspace; bring that Workspace forward first",
                    false,
                );
            } else {
                self.set_error(
                    "pair_view.no_agent_tab",
                    "Pair view shows a pane of an Agent tab beside its tray; this pane is in none",
                    false,
                );
            }
            return true;
        };
        let place = Place::Tray(tab.clone());
        let mut record = None;
        let shown = match payload.display_id {
            Some(id) if self.view_display(&key, &id).is_some() => Some(id),
            Some(id) => {
                let error = crate::view_layout::LayoutError::UnknownDisplay(id);
                self.set_error(error.kind(), error.message(), false);
                return true;
            }
            None => match self.pair_content(&key, &pane, &self.live_sessions()) {
                Some(PairContent::View(id)) => Some(id),
                Some(PairContent::Record(session)) => {
                    record = Some(session);
                    None
                }
                None => None,
            },
        };
        // The same pane again, with its View already in front of the drawn
        // desk, changes nothing and publishes nothing.
        let view = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1));
        let unchanged = view.is_some_and(|view| {
            view.draws_tray(&tab)
                && view.tray(&tab).is_some_and(|tray| {
                    tray.pair.as_ref().is_some_and(|pair| pair.pane == pane)
                        && shown.is_some()
                        && tray.layout.active_area().active == shown
                })
        });
        if unchanged {
            return false;
        }
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
                let Some(session) = record else {
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
        // A pair view already on keeps the pane it returns the keyboard to;
        // a new one takes the pane that holds it now.
        let before = match self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .and_then(|view| view.tray(&tab))
            .and_then(|tray| tray.pair.as_ref())
        {
            Some(pair) => pair.before.clone(),
            None => self.tab_keyboard_pane(&key, &tab),
        };
        if let Some(store) = self.workspace_views.as_mut() {
            store.views.entry(&key.0, &key.1).tray_entry(&tab).pair = Some(PairView {
                pane: pane.clone(),
                before,
            });
            store.generation += 1;
        }
        self.persist_workspace_views();
        if focus {
            self.focus_tab_pane(&key, &pane);
        }
        true
    }

    /// Gives the keyboard to `pane`, on this machine or on its device,
    /// bringing its tab in front: the left of a pair view entered or chosen,
    /// or the pane it returns the keyboard to.
    fn focus_tab_pane(&mut self, key: &WorkspaceKey, pane: &str) {
        if key.0 == self.node.as_str() {
            if self.local_tab_pane(pane) {
                self.focus_pane(pane.to_owned(), PaneFocusOrigin::Operator, None);
            }
            return;
        }
        self.request_remote_control(RemoteControlPayload {
            target_id: key.0.clone(),
            request_id: format!("pair-view:{}:{pane}", unix_milliseconds()),
            report_pane_focus_outcome: false,
            focus_device: false,
            request: RemoteControlRequest::FocusPane {
                pane_id: pane.to_owned(),
            },
        });
    }

    /// Ends `tab`'s pair view; whether it was in one.
    pub(super) fn end_pair_view(&mut self, key: &WorkspaceKey, tab: &str, cause: PairEnd) -> bool {
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
            "device": key.0, "tab": tab, "pane": pair.pane, "cause": cause.as_str(),
        }));
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation += 1;
        }
        self.persist_workspace_views();
        if cause.returns_keyboard()
            && let Some(before) = pair.before.filter(|before| {
                *before != pair.pane && self.pane_tab(key, before).as_deref() == Some(tab)
            })
        {
            self.focus_tab_pane(key, &before);
        }
        true
    }

    /// The pane of `tab` that holds the keyboard now, on this machine or
    /// its device.
    fn tab_keyboard_pane(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let focused = if key.0 == self.node.as_str() {
            self.snapshot.focused.pane_id.clone()
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == key.0)
                .and_then(|remote| remote.session.as_ref())
                .and_then(|session| session.focused_pane_id.clone())
        };
        focused.filter(|pane| self.pane_tab(key, pane).as_deref() == Some(tab))
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
        let sessions = self.live_sessions();
        let Some(pane) = self
            .view_display(key, display_id)
            .and_then(|display| self.owner_pane_of(display, None, &sessions))
            .filter(|pane| {
                *pane != left && self.pane_tab(key, pane).as_deref() == Some(tab.as_str())
            })
        else {
            return;
        };
        if let Some(tray) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
            .and_then(|view| view.tray_mut(&tab))
        {
            let before = tray.pair.take().and_then(|pair| pair.before);
            tray.pair = Some(PairView {
                pane: pane.clone(),
                before,
            });
        }
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation += 1;
        }
        self.persist_workspace_views();
        // The pane on screen is the tab's keyboard pane, so ⇧⌘W, a split and
        // the sidebar reach it; the shell leaves the keys on the desk tab the
        // operator chose (`PaneView`).
        self.focus_tab_pane(key, &pane);
    }

    /// The pane of the agent a desk tab belongs to, the one rule the tab's
    /// path line, pair view's left pane and its folded tabs read (B24,
    /// D-41): a card's owner (`detail_owner_pane`), else the pane its
    /// opener is in now; none for what the operator opened. `opener` is the
    /// opener's snapshot when the caller already holds it, and `sessions`
    /// the machine's live sessions by pane, read once by the caller.
    pub(super) fn owner_pane_of(
        &self,
        display: &crate::view_layout::Display,
        opener: Option<&crate::model::ViewOpenerSnapshot>,
        sessions: &[(String, SessionIdentity)],
    ) -> Option<String> {
        if let Some(card) = display.card.as_ref() {
            return self.detail_owner_pane_among(card, sessions);
        }
        match opener {
            Some(opener) => opener.pane_id.clone(),
            None => display
                .opener
                .as_ref()
                .and_then(|opener| self.view_opener_snapshot(opener).pane_id),
        }
    }

    /// The tabs of the drawn desk `root` that another agent of `pair`'s tab
    /// owns (D-41), read from the owner each display already carries.
    pub(super) fn pair_folded(
        &self,
        key: &WorkspaceKey,
        pair: &crate::model::PairViewSnapshot,
        root: &crate::model::ViewNodeSnapshot,
    ) -> Vec<String> {
        fn walk<'a>(
            node: &'a crate::model::ViewNodeSnapshot,
            out: &mut Vec<&'a crate::model::ViewDisplaySnapshot>,
        ) {
            match node {
                crate::model::ViewNodeSnapshot::Area(area) => out.extend(&area.displays),
                crate::model::ViewNodeSnapshot::Split(split) => {
                    walk(&split.first, out);
                    walk(&split.second, out);
                }
            }
        }
        let mut displays = Vec::new();
        walk(root, &mut displays);
        displays
            .into_iter()
            .filter(|display| {
                display.owner_pane.as_ref().is_some_and(|owner| {
                    *owner != pair.pane_id
                        && self.pane_tab(key, owner).as_deref() == Some(pair.tab_id.as_str())
                })
            })
            .map(|display| display.id.clone())
            .collect()
    }

    /// What pair view shows beside `pane` (B17, B19): the newest View its
    /// agent opened that the operator has not looked at, else the one it
    /// opened last, else its session's record; none for a pane with
    /// nothing to show. `sessions` is the machine's live sessions by pane.
    fn pair_content(
        &self,
        key: &WorkspaceKey,
        pane: &str,
        sessions: &[(String, SessionIdentity)],
    ) -> Option<PairContent> {
        if let Some(id) = self.pair_view_of(key, pane) {
            return Some(PairContent::View(id));
        }
        sessions
            .iter()
            .find(|(live, _)| live == pane)
            .map(|(_, session)| PairContent::Record(session.clone()))
    }

    /// The panes of `key`'s Agent tabs that have something to show beside
    /// the desk (B19), by `pair_content`, so ◧, ⌘. and the operator's
    /// choice agree. It runs on each publish of the Workspace in front:
    /// the checkout's panes times its Views (at most `MAX_TRAYS` desks of
    /// `MAX_VIEW_DISPLAYS` and the shelf's), with the live sessions read once.
    pub(super) fn pairable_panes(&self, key: &WorkspaceKey) -> Vec<String> {
        let sessions = self.live_sessions();
        self.checkout_tabs(key)
            .flat_map(|tab| &tab.panes)
            .filter(|row| self.pair_content(key, &row.id, &sessions).is_some())
            .map(|row| row.id.clone())
            .collect()
    }

    /// The operator choosing another pane of a tab in pair view, from the
    /// sidebar or a pane chord, takes it to the left with its own View on
    /// the right, so the two sides keep one owner (B25, D-09). A pane with
    /// nothing to show ends the pair view, so the pane chosen is on screen
    /// in the tab's own layout.
    pub(super) fn follow_pair_focus(&mut self, pane: &str) {
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let Some(tab) = self.pane_tab(&key, pane) else {
            return;
        };
        let left = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .and_then(|view| view.tray(&tab))
            .and_then(|tray| tray.pair.as_ref())
            .map(|pair| pair.pane.clone());
        if left.is_none_or(|left| left == pane) {
            return;
        }
        if self
            .pair_content(&key, pane, &self.live_sessions())
            .is_some()
        {
            self.set_pair_view_with(
                PairViewPayload {
                    pane_id: Some(pane.to_owned()),
                    display_id: None,
                },
                false,
            );
        } else {
            self.end_pair_view(&key, &tab, PairEnd::ChoseUnpaired);
        }
    }
}
