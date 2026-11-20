//! Pair view (PRD lane-desk-pair-view D-08, D-09, D-16, D-22, D-45..D-47):
//! one mode for the window, kept in `workspace-views.json` with its divider's
//! ratio (`PairMode`). While it is on, the pane the operator is on is drawn
//! beside its tab's desk, with the View its agent opened in front: the View
//! named, else its unseen View, else its last View, else its session record
//! (B17, B18). Going to another pane by any way (a pane chord, the sidebar, a
//! tab switch) takes the mode with it (`follow_pair_focus`,
//! `follow_pair_look`); a pane with nothing to show draws its tab's own layout
//! and the mode stays on (B19). The engine's layout, zoom and focus are not
//! touched, and the shell draws the pane's terminal over them (D-26). It ends
//! only on ⌘., the pressed ◧ or the strip's way back (`PairEnd::Left`), when
//! the drawn pane closes (`end_pairs_of_gone_panes`) or when the drawn desk's
//! last View closes or moves away (D-46); ending folds that desk (D-44).

use serde::Deserialize;

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::links::ledger::SessionIdentity;
use crate::model::DetailItem;
use crate::workspace_views::Place;

/// The pane pair view draws on the left, with its Workspace and tab.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PairLeft {
    pub(super) key: WorkspaceKey,
    pub(super) tab: String,
    pub(super) pane: String,
}

impl PairLeft {
    fn is_in(&self, key: &WorkspaceKey, tab: &str) -> bool {
        self.key == *key && self.tab == tab
    }
}

/// Why a pair view ended, which the diagnostic names and which decides
/// whether the keyboard goes back to the pane that held it before (B26).
#[derive(Clone, Copy, Debug)]
pub(super) enum PairEnd {
    /// The operator left: the strip's way back, ⌘., the pressed ◧.
    Left,
    /// The drawn desk's last View closed or moved away.
    Emptied,
    /// The drawn pane closed or left its tab (B28).
    PaneGone,
}

impl PairEnd {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Emptied => "emptied",
            Self::PaneGone => "pane_gone",
        }
    }

    /// The operator leaving, or closing the desk's last View, returns the
    /// keyboard; a pane that went leaves it.
    fn returns_keyboard(self) -> bool {
        matches!(self, Self::Left | Self::Emptied)
    }
}

/// `pair_view`: the pane to draw on the left, which turns the mode on, or
/// none to turn it off.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairViewPayload {
    #[serde(default)]
    pub(super) pane_id: Option<String>,
    /// The desk tab whose pin was pressed, which comes in front (B17).
    #[serde(default)]
    pub(super) display_id: Option<String>,
}

impl Runtime {
    /// Whether the window is in pair view mode.
    pub(super) fn pair_mode_on(&self) -> bool {
        self.workspace_views
            .as_ref()
            .is_some_and(|store| store.views.pair.on)
    }

    pub(super) fn set_pair_view(&mut self, payload: PairViewPayload) -> bool {
        let Some(pane) = payload.pane_id else {
            return self.end_pair_view(PairEnd::Left);
        };
        let Some(key) = self.front_workspace_key() else {
            crate::diagnostic!(serde_json::json!({
                "component": "pair_view", "kind": "pair_view.no_workspace",
            }));
            return false;
        };
        // A terminal outside every Agent tab, such as an automation run's
        // chat, has no desk to pair with (inventory X02); a pane of another
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
        let content = match payload.display_id {
            Some(id) if self.view_display(&key, &id).is_some() => id,
            Some(id) => {
                let error = crate::view_layout::LayoutError::UnknownDisplay(id);
                self.set_error(error.kind(), error.message(), false);
                return true;
            }
            None => match self.pair_content(&key, &pane, &self.live_sessions()) {
                Some(content) => content,
                None => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "pair_view", "kind": "pair_view.nothing_to_show",
                        "device": key.0, "pane": pane,
                    }));
                    return false;
                }
            },
        };
        let on = self.pair_mode_on();
        let left = PairLeft {
            key: key.clone(),
            tab: tab.clone(),
            pane: pane.clone(),
        };
        // The same pane again, with its View already in front of its desk,
        // changes nothing and publishes nothing.
        let store = self.workspace_views.as_ref();
        let unchanged = on
            && store.is_some_and(|store| store.pair_left.as_ref() == Some(&left))
            && self
                .view_layout_at(&key, &Place::Tray(tab.clone()))
                .is_some_and(|layout| layout.active_area().active.as_ref() == Some(&content));
        if unchanged {
            return false;
        }
        // A pair view already on keeps the pane it returns the keyboard to;
        // a new one takes the pane that holds it now.
        let before = if on {
            None
        } else {
            self.tab_keyboard_pane(&key, &tab)
        };
        if let Some(store) = self.workspace_views.as_mut() {
            if !on {
                store.views.pair.on = true;
                store.pair_before = before.map(|pane| (key.clone(), pane));
            }
            store.generation += 1;
        }
        if !on {
            self.persist_workspace_views();
        }
        self.show_pair(left, Some(content));
        self.focus_tab_pane(&key, &pane);
        true
    }

    /// Draws `left` beside its tab's desk, with `content` in front of the
    /// desk; none leaves the desk as it is (a pane with nothing to show).
    fn show_pair(&mut self, left: PairLeft, content: Option<String>) {
        let key = left.key.clone();
        let place = Place::Tray(left.tab.clone());
        if let Some(store) = self.workspace_views.as_mut() {
            // The desk the mode leaves for another tab folds as the one it
            // ends on does, so leaving opens no lane anywhere pair view went
            // (D-44): a pin pressed on an open lane, or an agent's View that
            // opened the lane meanwhile, leaves nothing over the agents there.
            if let Some(old) = store.pair_left.take()
                && !old.is_in(&left.key, &left.tab)
            {
                fold_pair_desk(&mut store.views, &old);
            }
            store.pair_left = Some(left);
            store.generation += 1;
        }
        let Some(id) = content else {
            return;
        };
        if self.view_place(&key, &id).as_ref() != Some(&place)
            && !self.bring_view_to(&key, &id, &place)
        {
            let error = crate::view_layout::LayoutError::DisplayLimit;
            self.set_error(error.kind(), error.message(), false);
            return;
        }
        let _ = self.change_view_layout(&key, &place, |layout, stamp| {
            layout.focus(&id, stamp).map(|changed| ((), changed))
        });
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

    /// Turns the mode off, everywhere (D-45); whether it was on. The desk it
    /// drew folds, and the lane closes when nothing else is in it, so the
    /// screen it leaves shows no View on the right (D-44).
    pub(super) fn end_pair_view(&mut self, cause: PairEnd) -> bool {
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        if !store.views.pair.on {
            return false;
        }
        store.views.pair.on = false;
        let left = store.pair_left.take();
        let before = store.pair_before.take();
        store.generation += 1;
        if let Some(left) = &left {
            fold_pair_desk(&mut store.views, left);
        }
        crate::diagnostic!(serde_json::json!({
            "component": "pair_view", "kind": "pair_view.ended",
            "device": left.as_ref().map(|left| left.key.0.clone()),
            "tab": left.as_ref().map(|left| left.tab.clone()),
            "pane": left.as_ref().map(|left| left.pane.clone()),
            "cause": cause.as_str(),
        }));
        self.persist_workspace_views();
        if cause.returns_keyboard()
            && let Some(left) = left
            && let Some((key, before)) = before.filter(|(key, before)| {
                *key == left.key
                    && *before != left.pane
                    && self.pane_tab(key, before).as_deref() == Some(left.tab.as_str())
            })
        {
            self.focus_tab_pane(&key, &before);
        }
        true
    }

    /// Ends pair view when the screen drew it on `tab` of `key`, whose desk
    /// just lost its last View (D-46). It asks the pair last published, not
    /// `drawn_pair`, which reads the desk already emptied: a pane whose only
    /// thing to show was that View, such as an agent's that ended and left
    /// its shell, has nothing to show once it closed, yet the screen drew it.
    /// On a desk pair view did not draw, such as beside a shell, the mode
    /// stays on (D-45).
    pub(super) fn end_pair_view_of_emptied(&mut self, key: &WorkspaceKey, tab: &str) -> bool {
        if self
            .published_pair()
            .is_none_or(|(drawn, drawn_tab, _)| drawn != *key || drawn_tab != tab)
        {
            return false;
        }
        self.end_pair_view(PairEnd::Emptied)
    }

    /// The pair view the last published snapshot drew: its Workspace, tab
    /// and pane. What the screen shows now, read by the ends that follow a
    /// change already applied (an emptied desk, a pane gone).
    pub(super) fn published_pair(&self) -> Option<(WorkspaceKey, String, String)> {
        if !self.pair_mode_on() {
            return None;
        }
        let view = self.snapshot.workspace_view.as_ref()?;
        let pair = view.tray.pair.as_ref()?;
        Some((
            (view.device_id.clone(), view.path.clone()),
            pair.tab_id.clone(),
            pair.pane_id.clone(),
        ))
    }

    /// The pane pair view draws beside `tab`'s desk now (D-45, B19): the
    /// mode is on, the pane the operator is on there has something to show,
    /// and the desk has a View. The one rule the snapshot, the desk's
    /// visibility (`tray_in_view`, `drawn_tray_layout`) and a desk tab
    /// brought in front (`follow_pair_front`) read, so the core never treats
    /// as drawn a pair view the screen does not draw.
    pub(super) fn drawn_pair(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let pane = self.pair_showing(key, tab)?;
        self.view_layout_at(key, &Place::Tray(tab.to_owned()))
            .is_some_and(|layout| layout.display_count() > 0)
            .then_some(pane)
    }

    /// `drawn_pair` before its desk's count: the pane on the left while it
    /// has something to show. Free while the mode is off; while it is on,
    /// each read lists the live agents' sessions once and walks the
    /// Workspace's Views once (`pair_content`), work under the lock bounded
    /// by the agents and the Views a Workspace holds, as every snapshot's
    /// own reads of them are.
    fn pair_showing(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let pane = self.pair_left_of(key, tab)?;
        self.pair_content(key, &pane, &self.live_sessions())
            .is_some()
            .then_some(pane)
    }

    /// The pane on the left of `tab` while the mode is on: the pane the
    /// operator last went to there while it is still in the tab, else, as
    /// after a restart or once that pane closed, the tab's keyboard pane.
    /// Whether it has something to show is `drawn_pair`'s to ask.
    pub(super) fn pair_left_of(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let store = self.workspace_views.as_ref()?;
        if !store.views.pair.on {
            return None;
        }
        match store.pair_left.as_ref() {
            Some(left)
                if left.is_in(key, tab)
                    && self.pane_tab(key, &left.pane).as_deref() == Some(tab) =>
            {
                Some(left.pane.clone())
            }
            _ => self.tab_keyboard_pane(key, tab),
        }
    }

    /// The pane of `tab` that holds the keyboard now, on this machine or
    /// its device: the pane the operator chose, else the one the engine
    /// focuses.
    fn tab_keyboard_pane(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let in_tab = |pane: &String| self.pane_tab(key, pane).as_deref() == Some(tab);
        if key.0 == self.node.as_str() {
            return self
                .look
                .focus()
                .map(str::to_owned)
                .filter(in_tab)
                .or_else(|| self.snapshot.focused.pane_id.clone().filter(in_tab));
        }
        self.snapshot
            .status
            .remote
            .iter()
            .find(|remote| remote.target_id == key.0)
            .and_then(|remote| remote.session.as_ref())
            .and_then(|session| session.focused_pane_id.clone())
            .filter(in_tab)
    }

    /// In pair view, a desk tab the operator brought in front that another
    /// agent of the drawn tab opened takes that agent's pane to the left; a
    /// tab nobody's agent opened leaves the left pane (B24, D-21).
    pub(super) fn follow_pair_front(&mut self, key: &WorkspaceKey, display_id: &str) {
        let Some(Place::Tray(tab)) = self.view_place(key, display_id) else {
            return;
        };
        let Some(left) = self.drawn_pair(key, &tab) else {
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
        self.show_pair(
            PairLeft {
                key: key.clone(),
                tab,
                pane: pane.clone(),
            },
            None,
        );
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

    /// What pair view shows beside `pane` (B17, B19, D-49): of the Views
    /// its agent opened and the record cards of its session the operator
    /// put on a desk, the newest the operator has not looked at, else the
    /// newest; none for a pane with nothing to show, such as a shell or an
    /// agent that opened nothing, which draws its tab's own layout. No card
    /// is opened for it. `sessions` is the machine's live sessions by pane.
    fn pair_content(
        &self,
        key: &WorkspaceKey,
        pane: &str,
        sessions: &[(String, SessionIdentity)],
    ) -> Option<String> {
        let scope = self.session_scope_of_pane(pane);
        let own: Vec<&crate::view_layout::Display> = self
            .view_displays(key)
            .filter(|display| match &display.card {
                Some(card @ DetailItem::Record { .. }) => {
                    self.detail_owner_pane_among(card, sessions).as_deref() == Some(pane)
                }
                Some(_) => false,
                None => scope
                    .as_ref()
                    .is_some_and(|scope| display.openers.iter().any(|o| o.of_session(scope))),
            })
            .collect();
        own.iter()
            .filter(|display| display.unseen.is_some())
            .max_by_key(|display| display.opened_stamp)
            .or_else(|| own.iter().max_by_key(|display| display.opened_stamp))
            .map(|display| display.id.clone())
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

    /// The operator chose `pane`, from the sidebar, a pane chord or a
    /// click, on this machine or a device: while the mode is on it is drawn
    /// on the left with its own View on the right, so the two sides keep one
    /// owner (B25, D-09, D-45). A pane with nothing to show draws its tab's
    /// own layout, and the mode stays on.
    pub(super) fn follow_pair_focus(&mut self, pane: &str) {
        if !self.pair_mode_on() {
            return;
        }
        let Some(key) = self
            .front_workspace_key()
            .filter(|key| self.pane_tab(key, pane).is_some())
            .or_else(|| self.pane_workspace_of(pane))
        else {
            return;
        };
        let Some(tab) = self.pane_tab(&key, pane) else {
            return;
        };
        self.follow_pair_to(key, tab, pane.to_owned());
    }

    /// After a selection moved the operator's look (`change_look`): while
    /// the mode is on and the tab in front is not the one pair view last
    /// drew, as after a tab switch, ⌃⇥ or a Workspace chosen in the sidebar,
    /// its keyboard pane goes to the left (D-45). A pane chosen inside the
    /// drawn tab is `follow_pair_focus`'s, and the engine's own focus moves
    /// nothing.
    pub(super) fn follow_pair_look(&mut self) {
        if !self.pair_mode_on() {
            return;
        }
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let tab = self.front_tab(&key);
        if tab == crate::workspace_views::PENDING_TRAY
            || self.workspace_views.as_ref().is_some_and(|store| {
                store
                    .pair_left
                    .as_ref()
                    .is_some_and(|left| left.is_in(&key, &tab))
            })
        {
            return;
        }
        let Some(pane) = self.tab_keyboard_pane(&key, &tab) else {
            return;
        };
        self.follow_pair_to(key, tab, pane);
    }

    /// `pane` of `tab` goes to the left, its View in front of the desk.
    fn follow_pair_to(&mut self, key: WorkspaceKey, tab: String, pane: String) {
        let left = PairLeft { key, tab, pane };
        if self
            .workspace_views
            .as_ref()
            .is_some_and(|store| store.pair_left.as_ref() == Some(&left))
        {
            return;
        }
        let content = self.pair_content(&left.key, &left.pane, &self.live_sessions());
        self.show_pair(left, content);
    }
}

/// Folds the desk pair view drew on `left`'s tab and closes its lane when
/// no column is left in it, so the screen there shows no View on the right
/// (D-44).
fn fold_pair_desk(views: &mut crate::workspace_views::WorkspaceViews, left: &PairLeft) {
    let Some(view) = views.get_mut(&left.key.0, &left.key.1) else {
        return;
    };
    if let Some(tray) = view.tray_mut(&left.tab) {
        tray.shown = false;
    }
    if !view.has_column(Some(&left.tab)) {
        view.open = false;
    }
}
