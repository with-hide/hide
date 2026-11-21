//! Pair view (PRD lane-desk-pair-view D-08, D-09, D-16, D-22, D-45..D-47,
//! D-53): one mode for the window, kept in `workspace-views.json` with its
//! divider's ratio (`PairMode`). While it is on, the pane the operator is on
//! is drawn on the left: beside its tab's desk, with the View its agent
//! opened in front (the View named, else its unseen View, else its last
//! View, B17, B18), or alone at the body's full width while its agent has
//! no View to show (a shell, an agent that opened none), and beside the
//! desk once its agent opens one (D-53). Going to another pane by any way (a
//! pane chord, the sidebar, a tab switch) takes the mode with it
//! (`follow_pair_focus`, `follow_pair_look`). The engine's layout, zoom and
//! focus are not touched, and the shell draws the pane's terminal over them
//! (D-26). It ends only on ⌘., the pressed ◧ or the strip's way back
//! (`PairEnd::Left`), when the drawn pane closes (`end_pairs_of_gone_panes`)
//! or when the drawn desk's last View closes or moves away (D-46). Every
//! desk the mode leaves, by a move or its end, gets back the lane as it was
//! before the mode drew it: open or closed, the desk shown or folded, the
//! same View in front (D-51).

use serde::Deserialize;

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::links::ledger::SessionIdentity;
use crate::model::DetailItem;
use crate::workspace_views::Place;

/// What the pair rules read once for a publish or an event
/// (`Runtime::pair_reads`).
pub(super) struct PairReads {
    /// The machine's live agents' sessions, by pane.
    sessions: Vec<(String, SessionIdentity)>,
    /// The pane each View a terminal link opened belongs to now, by the
    /// View's id.
    links: std::collections::BTreeMap<String, String>,
}

/// The pane pair view draws on the left, with its Workspace and tab.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PairLeft {
    pub(super) key: WorkspaceKey,
    pub(super) tab: String,
    pub(super) pane: String,
}

/// A desk pair view drew, with its lane as it was before the mode first
/// drew it: whether the lane was open, whether it showed this desk, and
/// the View in front of the desk (D-51).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PairDrawn {
    key: WorkspaceKey,
    tab: String,
    open: bool,
    shown: bool,
    front: Option<String>,
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

/// `pair_view_desk`: whether the desk stands beside the pane focus mode
/// draws (D-57).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PairDeskPayload {
    pub(super) shown: bool,
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
        // A pane with no View to show is drawn alone (D-53).
        let content = match payload.display_id {
            Some(id) if self.view_display(&key, &id).is_some() => Some(id),
            Some(id) => {
                let error = crate::view_layout::LayoutError::UnknownDisplay(id);
                self.set_error(error.kind(), error.message(), false);
                return true;
            }
            None => self.pair_content(&key, &pane, &self.pair_reads(&key)),
        };
        let on = self.pair_mode_on();
        let left = PairLeft {
            key: key.clone(),
            tab: tab.clone(),
            pane: pane.clone(),
        };
        // The same pane again, with its View already in front of its desk or
        // with none to show, changes nothing and publishes nothing.
        let store = self.workspace_views.as_ref();
        let unchanged = on
            && store.is_some_and(|store| store.pair_left.as_ref() == Some(&left))
            && content.as_ref().is_none_or(|content| {
                self.view_layout_at(&key, &Place::Tray(tab.clone()))
                    .is_some_and(|layout| layout.active_area().active.as_ref() == Some(content))
            });
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
        self.show_pair(left, content);
        self.focus_tab_pane(&key, &pane);
        true
    }

    /// Draws `left` with its tab's desk, `content` in front of the desk;
    /// none leaves the desk as it is, as a pane drawn alone or a desk tab
    /// the operator brought in front does. The desk is recorded as it is
    /// now even for a pane drawn alone, so a View its agent opens later
    /// leaves with the mode (D-51, D-53).
    fn show_pair(&mut self, left: PairLeft, content: Option<String>) {
        let key = left.key.clone();
        let place = Place::Tray(left.tab.clone());
        self.restore_pair_desks(Some((&left.key, &left.tab)));
        self.note_pair_desk(&left.key, &left.tab);
        if let Some(store) = self.workspace_views.as_mut() {
            if store
                .pair_desk_choice
                .as_ref()
                .is_some_and(|(chosen, _)| *chosen != left)
            {
                store.pair_desk_choice = None;
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

    /// Turns the mode off, everywhere (D-45); whether it was on. The screen
    /// goes back to how it was before the mode drew it: the lane open or
    /// closed, its desk shown or folded, the same View in front (D-51).
    pub(super) fn end_pair_view(&mut self, cause: PairEnd) -> bool {
        if !self.pair_mode_on() {
            return false;
        }
        self.restore_pair_desks(None);
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        store.views.pair.on = false;
        store.pair_desk_choice = None;
        let left = store.pair_left.take();
        let before = store.pair_before.take();
        store.generation += 1;
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

    /// Records `tab`'s desk of `key` as pair view draws it, with how it is
    /// now, unless it is recorded already: the state it gets back (D-51).
    pub(super) fn note_pair_desk(&mut self, key: &WorkspaceKey, tab: &str) {
        let recorded = self.workspace_views.as_ref().is_none_or(|store| {
            store
                .pair_drawn
                .iter()
                .any(|desk| desk.key == *key && desk.tab == tab)
        });
        if recorded {
            return;
        }
        let front = self
            .view_layout_at(key, &Place::Tray(tab.to_owned()))
            .and_then(|layout| layout.active_area().active.clone());
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let view = store.views.get(&key.0, &key.1);
        let open = view.is_some_and(|view| view.open);
        let shown = view
            .and_then(|view| view.tray(tab))
            .is_some_and(|tray| tray.shown);
        store.pair_drawn.push(PairDrawn {
            key: key.clone(),
            tab: tab.to_owned(),
            open,
            shown,
            front,
        });
    }

    /// Gives every desk pair view drew (`pair_drawn`) but the one the mode
    /// stays on (`staying`) back the state it had before the mode drew it:
    /// shown or folded, with the same View in front if it is still there,
    /// so a desk the mode leaves, for another tab or Workspace or out of the
    /// mode, is as the operator left it (D-51, D-53).
    fn restore_pair_desks(&mut self, staying: Option<(&WorkspaceKey, &str)>) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let (kept, left): (Vec<_>, Vec<_>) = std::mem::take(&mut store.pair_drawn)
            .into_iter()
            .partition(|desk| {
                staying.is_some_and(|(key, tab)| desk.key == *key && desk.tab == tab)
            });
        store.pair_drawn = kept;
        for desk in left {
            let place = Place::Tray(desk.tab.clone());
            if let Some(store) = self.workspace_views.as_mut()
                && let Some(view) = store.views.get_mut(&desk.key.0, &desk.key.1)
            {
                view.open = desk.open;
                if let Some(tray) = view.tray_mut(&desk.tab) {
                    // A desk the mode emptied has nothing to show again.
                    tray.shown = desk.shown && tray.layout.display_count() > 0;
                }
            }
            let Some(front) = desk.front else {
                continue;
            };
            let there = self
                .view_layout_at(&desk.key, &place)
                .is_some_and(|layout| layout.displays().any(|display| display.id == front));
            if there {
                let _ = self.change_view_layout(&desk.key, &place, |layout, stamp| {
                    layout.focus(&front, stamp).map(|changed| ((), changed))
                });
            }
        }
    }

    /// Ends pair view when a View leaving a desk (closed, or moved away)
    /// leaves the desk the screen drew beside the pane with nothing to draw
    /// (D-46, read by the pane): the pane on the left has nothing left to
    /// show, or its desk is empty, even with the operator's own tabs still
    /// on it. It compares the pair last published with `pair_desk` now, so
    /// a pane whose only thing to show was the View that left still ends
    /// the mode it drew; a pane drawn alone had no View to close, so its
    /// mode stays on, and a desk the operator's open stood beside it goes
    /// once emptied and leaves it drawn alone again (D-53). The mode moving
    /// itself is not a View leaving, so a pair whose left pane is no longer
    /// the drawn tab's is left alone.
    pub(super) fn end_pair_view_if_undrawn(&mut self) -> bool {
        let Some((key, tab, pane)) = self.published_pair() else {
            return false;
        };
        let alone = self
            .snapshot
            .workspace_view
            .as_ref()
            .and_then(|view| view.tray.pair.as_ref())
            .is_some_and(|pair| pair.alone);
        let moved = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.pair_left.as_ref())
            .is_some_and(|left| !left.is_in(&key, &tab));
        if alone
            || moved
            || self.pair_desk_choice(&key, &tab, &pane) == Some(true)
            || self.pair_desk(&key, &tab).is_some()
        {
            return false;
        }
        self.end_pair_view(PairEnd::Emptied)
    }

    /// The pair view the last published snapshot drew: its Workspace, tab
    /// and pane, alone or beside the desk. What the screen shows now, read
    /// by the ends that follow a change already applied (an emptied desk, a
    /// pane gone).
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

    /// The pane pair view draws beside `tab`'s desk now (D-45, D-53): the
    /// mode is on, the pane the operator is on there has something to show,
    /// and the desk has a View; the pane is drawn alone otherwise. The one
    /// rule the snapshot, the desk's visibility (`drawn_tray_layout`) and a
    /// desk tab brought in front (`follow_pair_front`) read, so the core
    /// never treats as drawn a desk the screen does not draw. Free while the
    /// mode is off; while it is on, each read takes the pair's reads once
    /// (`pair_reads`) and walks the Workspace's Views once (`pair_content`),
    /// work under the lock bounded by the agents and the Views a Workspace
    /// holds, as every snapshot's own reads of them are.
    pub(super) fn pair_desk(&self, key: &WorkspaceKey, tab: &str) -> Option<String> {
        let pane = self.pair_left_of(key, tab)?;
        self.pair_desk_beside(key, tab, &pane, &self.pair_reads(key))
            .then_some(pane)
    }

    /// Whether `tab`'s desk stands beside `pane`, the pane focus mode draws
    /// there (`pair_desk`): as the operator chose for it (their open stood
    /// it there, ⌘E showed or folded it), else while the pane has a View to
    /// show, and only while the desk has a View.
    pub(super) fn pair_desk_beside(
        &self,
        key: &WorkspaceKey,
        tab: &str,
        pane: &str,
        reads: &PairReads,
    ) -> bool {
        let chosen = match self.pair_desk_choice(key, tab, pane) {
            Some(shown) => shown,
            None => self.pair_content(key, pane, reads).is_some(),
        };
        chosen
            && self
                .view_layout_at(key, &Place::Tray(tab.to_owned()))
                .is_some_and(|layout| layout.display_count() > 0)
    }

    /// What the pair rules read of the machine and of `key`'s Views, once
    /// for a publish or an event: the live agents' sessions by pane, and the
    /// pane each View a terminal link opened belongs to now, each link's
    /// terminal looked up once (D-54).
    pub(super) fn pair_reads(&self, key: &WorkspaceKey) -> PairReads {
        PairReads {
            sessions: self.live_sessions(),
            links: self
                .view_displays(key)
                .filter_map(|display| {
                    self.link_pane_of(display)
                        .map(|pane| (display.id.clone(), pane))
                })
                .collect(),
        }
    }

    /// Whether `tab`'s desk of `key` is on screen while focus mode draws a
    /// pane of that tab: only beside it (`pair_desk`). `None` while the mode
    /// draws no pane there, where the lane's own state decides. The one rule
    /// the desk's drawing (`drawn_tray_layout`), a View landing in view
    /// (`tray_in_view`) and the lane a View calls (`open_tray_for`) read, so
    /// an open lane under a pane drawn alone is never taken for a desk on
    /// screen.
    pub(super) fn pair_desk_drawn(&self, key: &WorkspaceKey, tab: &str) -> Option<bool> {
        self.pair_left_of(key, tab)
            .map(|_| self.pair_desk(key, tab).is_some())
    }

    /// What the operator chose for the desk beside `pane`, drawn by focus
    /// mode on `tab`: shown, folded, or none (`pair_desk_choice`).
    fn pair_desk_choice(&self, key: &WorkspaceKey, tab: &str, pane: &str) -> Option<bool> {
        self.workspace_views
            .as_ref()
            .and_then(|store| store.pair_desk_choice.as_ref())
            .filter(|(chosen, _)| chosen.is_in(key, tab) && chosen.pane == pane)
            .map(|(_, shown)| *shown)
    }

    fn choose_pair_desk(&mut self, key: &WorkspaceKey, tab: &str, pane: String, shown: bool) {
        if let Some(store) = self.workspace_views.as_mut() {
            store.pair_desk_choice = Some((
                PairLeft {
                    key: key.clone(),
                    tab: tab.to_owned(),
                    pane,
                },
                shown,
            ));
            store.generation += 1;
        }
    }

    /// A View came onto `tab`'s desk, and the lane is called to it, while
    /// focus mode draws a pane of that tab with no desk beside it: an
    /// Explorer, ⌘P or ⌘K open, the pane's own `hide file open`, or its
    /// agent's open onto a desk ⌘E folded. The desk stands beside the pane
    /// with that View in front, and the pane stays, as for any tab the
    /// operator opened (D-21); ⌘E folds it again (D-57).
    pub(super) fn stand_pair_desk(&mut self, key: &WorkspaceKey, tab: &str) {
        let Some(pane) = self.pair_left_of(key, tab) else {
            return;
        };
        if self.pair_desk_beside(key, tab, &pane, &self.pair_reads(key)) {
            return;
        }
        self.choose_pair_desk(key, tab, pane, true);
    }

    /// `pair_view_desk`: ⌘E in focus mode shows or folds the desk beside the
    /// pane it draws (D-57); folded, the pane is drawn alone at the body's
    /// full width, as a zoom was. A desk with no View has nothing to show,
    /// which the log records. Whether anything changed.
    pub(super) fn set_pair_desk(&mut self, payload: PairDeskPayload) -> bool {
        let Some(key) = self.front_workspace_key() else {
            return false;
        };
        let tab = self.front_tab(&key);
        let Some(pane) = self.pair_left_of(&key, &tab) else {
            crate::diagnostic!(serde_json::json!({
                "component": "pair_view", "kind": "pair_view.desk_without_mode",
            }));
            return false;
        };
        let reads = self.pair_reads(&key);
        if self.pair_desk_beside(&key, &tab, &pane, &reads) == payload.shown {
            return false;
        }
        let empty = self
            .view_layout_at(&key, &Place::Tray(tab.clone()))
            .is_none_or(|layout| layout.display_count() == 0);
        if payload.shown && empty {
            crate::diagnostic!(serde_json::json!({
                "component": "pair_view", "kind": "pair_view.desk_empty",
                "device": key.0, "tab": tab, "pane": pane,
            }));
            return false;
        }
        self.choose_pair_desk(&key, &tab, pane, payload.shown);
        true
    }

    /// The pane pair view draws on the left of `tab` while the mode is on,
    /// alone or beside the desk (`pair_desk`): the pane the operator last
    /// went to there while it is still in the tab, else, as after a restart
    /// or once that pane closed, the tab's keyboard pane.
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
        let Some(left) = self.pair_desk(key, &tab) else {
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

    /// The pane a desk tab belongs to, the one rule the tab's path line,
    /// pair view's left pane and its folded tabs, and the pane's `N views`
    /// list read (B24, D-41): a card's owner (`detail_owner_pane`), else the
    /// pane of whoever opened it last, its agent or the pane whose link
    /// opened it (D-54); none for what the operator opened. `opener` is the
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
        let agent = match opener {
            Some(opener) => opener.pane_id.clone(),
            None => display
                .opener
                .as_ref()
                .and_then(|opener| self.view_opener_snapshot(opener).pane_id),
        };
        agent.or_else(|| self.link_pane_of(display))
    }

    /// The pane whose link opened `display` last (D-54).
    fn link_pane_of(&self, display: &crate::view_layout::Display) -> Option<String> {
        display
            .link_terminal
            .as_ref()
            .and_then(|terminal| self.link_terminal_pane(terminal))
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

    /// What pair view shows beside `pane` (B17, B19, D-49, D-54): of the
    /// Views its agent opened, those the operator opened from its terminal's
    /// links, and the record cards of its session the operator put on a
    /// desk, the newest the operator has not looked at, else the newest; none for a pane with nothing to show, such as a shell or an
    /// agent that opened nothing, which is drawn alone (D-53). No card is
    /// opened for it.
    fn pair_content(&self, key: &WorkspaceKey, pane: &str, reads: &PairReads) -> Option<String> {
        let scope = self.session_scope_of_pane(pane);
        let own: Vec<&crate::view_layout::Display> =
            self.view_displays(key)
                .filter(|display| match &display.card {
                    Some(card @ DetailItem::Record { .. }) => {
                        self.detail_owner_pane_among(card, &reads.sessions)
                            .as_deref()
                            == Some(pane)
                    }
                    Some(_) => false,
                    None => {
                        scope.as_ref().is_some_and(|scope| {
                            display.openers.iter().any(|o| o.of_session(scope))
                        }) || reads.links.get(&display.id).map(String::as_str) == Some(pane)
                    }
                })
                .collect();
        own.iter()
            .filter(|display| display.unseen.is_some())
            .max_by_key(|display| display.opened_stamp)
            .or_else(|| own.iter().max_by_key(|display| display.opened_stamp))
            .map(|display| display.id.clone())
    }

    /// The panes of `key`'s Agent tabs with a View to show beside the desk,
    /// by `pair_content`: the pane head's ◧ cell shows on
    /// these only, to keep the other heads quiet (D-53). It runs on each
    /// publish of the Workspace in front: the checkout's panes times its
    /// Views (at most `MAX_TRAYS` desks of `MAX_VIEW_DISPLAYS` and the
    /// shelf's), with the pair's reads taken once by the caller.
    pub(super) fn panes_with_views(&self, key: &WorkspaceKey, reads: &PairReads) -> Vec<String> {
        self.checkout_tabs(key)
            .flat_map(|tab| &tab.panes)
            .filter(|row| self.pair_content(key, &row.id, reads).is_some())
            .map(|row| row.id.clone())
            .collect()
    }

    /// The operator chose `pane`, from the sidebar, a pane chord or a
    /// click, on this machine or a device: while the mode is on it is drawn
    /// on the left with its own View on the right, so the two sides keep one
    /// owner (B25, D-09, D-45). A pane with nothing to show is drawn alone
    /// (D-53).
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
        let content = self.pair_content(&left.key, &left.pane, &self.pair_reads(&left.key));
        self.show_pair(left, content);
    }
}
