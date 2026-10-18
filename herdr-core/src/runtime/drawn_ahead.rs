//! What the core draws ahead of Herdr (PRD instant-pane-topology D-05, D-07,
//! D-08): a created tab from the moment Herdr names it, and each tab's pane
//! geometry line (`operations.rs`) as it will stand once Herdr applies it.
//!
//! Both are laid over the session as it is ingested, after the session's
//! confirmations have read Herdr's own layout, so every reader of the session
//! sees one layout and nothing drawn here can confirm what it predicts. The
//! core asks the coordinator for a republish whenever what is drawn changes
//! (`request_republish`), and Herdr's own layout replaces each drawing when
//! it arrives.

use super::*;

/// How many created tabs can wait for their layout at once; the oldest is
/// dropped past it and logged (engineering 15).
const PROVISIONAL_TAB_LIMIT: usize = 8;
/// How long a created tab is drawn without Herdr's layout before it is taken
/// back; the same bound as every operation stage.
const PROVISIONAL_TAB_TIMEOUT_MS: u64 = CLOSE_STAGE_TIMEOUT_MS;

/// A tab Herdr acknowledged creating, pending until its ordered layout
/// arrives. When its workspace is already known, draw it as one pane (D-05).
#[derive(Clone, Debug)]
pub(super) struct ProvisionalTab {
    // Opening an owner may create a workspace too. Its identity is learned
    // from the session, never guessed from the returned tab or pane id.
    pub(super) workspace_id: Option<String>,
    pub(super) tab_id: String,
    pub(super) pane_id: String,
    pub(super) cwd: String,
    pub(super) label: String,
    /// The stage record of the creation, when it has one.
    pub(super) timing_id: Option<String>,
    pub(super) recorded_at_unix_ms: u64,
}

/// A fork's pane Herdr has named and not yet laid out. It is the checkout's
/// it was made in for at most `PROVISIONAL_TAB_TIMEOUT_MS`, as a created tab
/// is drawn: a split is not drawn, so until Herdr lays it out the projection
/// reads it as a creation pending rather than a missing pane.
#[derive(Clone, Debug)]
pub(super) struct MadePane {
    pub(super) pane_id: String,
    pub(super) checkout: String,
    pub(super) recorded_at_unix_ms: u64,
}

impl Runtime {
    /// Lays everything drawn ahead of Herdr over a session about to be
    /// ingested.
    pub(super) fn overlay_drawn_ahead(&mut self, payload: &mut SessionSnapshotPayload) {
        let laid_out = payload
            .layouts
            .iter()
            .flat_map(|layout| &layout.panes)
            .map(|pane| pane.pane_id.as_str())
            .collect::<HashSet<_>>();
        self.settle_made_panes(|pane_id| laid_out.contains(pane_id), unix_milliseconds());
        self.overlay_created_tabs(payload);
        self.overlay_geometry(payload);
    }

    /// Ends each fork pane's hold once Herdr's layout lists it, or at its
    /// bound, logged, after which a pane still missing is one the projection
    /// waits on and says so. Says whether one lapsed.
    pub(super) fn settle_made_panes(
        &mut self,
        laid_out: impl Fn(&str) -> bool,
        now_unix_ms: u64,
    ) -> bool {
        let mut lapsed = false;
        for fork in self.forks_in_flight.values_mut() {
            let Some(made) = fork.made.as_ref() else {
                continue;
            };
            if laid_out(&made.pane_id) {
                fork.made = None;
            } else if now_unix_ms.saturating_sub(made.recorded_at_unix_ms)
                >= PROVISIONAL_TAB_TIMEOUT_MS
            {
                crate::diagnostic!(serde_json::json!({
                    "component": "pane_fork",
                    "kind": "pane.fork.layout_late",
                    "pane_id": made.pane_id,
                }));
                fork.made = None;
                lapsed = true;
            }
        }
        lapsed
    }

    /// Holds the acknowledged creation until Herdr lays it out. A known
    /// workspace can draw it already; opening an owner waits for that identity.
    pub(super) fn draw_created_tab(&mut self, tab: ProvisionalTab) {
        if self
            .snapshot
            .pane_layouts
            .iter()
            .any(|layout| layout.tab_id == tab.tab_id)
            || self
                .provisional_tabs
                .iter()
                .any(|drawn| drawn.tab_id == tab.tab_id)
        {
            return;
        }
        if self.provisional_tabs.len() >= PROVISIONAL_TAB_LIMIT {
            let dropped = self.provisional_tabs.remove(0);
            crate::diagnostic!(serde_json::json!({
                "component": "tab_control",
                "kind": "tab.create.provisional_dropped",
                "tab_id": dropped.tab_id,
                "limit": PROVISIONAL_TAB_LIMIT,
            }));
        }
        self.provisional_tabs.push(tab);
        self.request_republish();
    }

    pub(super) fn created_pane_awaits_layout(&self, pane_id: &str, checkout: &str) -> bool {
        self.created_pane_checkout(pane_id) == Some(checkout)
    }

    /// The checkout a pane Hide made was made in, which holds it before
    /// Herdr's layout lists it: a created tab's, or a starting fork's, which
    /// is its source's whether the fork has a tab of its own or a split.
    pub(super) fn created_pane_checkout(&self, pane_id: &str) -> Option<&str> {
        self.provisional_tabs
            .iter()
            .find(|tab| tab.pane_id == pane_id)
            .map(|tab| tab.cwd.as_str())
            .or_else(|| {
                self.forks_in_flight
                    .values()
                    .filter_map(|fork| fork.made.as_ref())
                    .find(|made| made.pane_id == pane_id)
                    .map(|made| made.checkout.as_str())
            })
    }

    /// Takes back a created tab whose pane its worker closed before Herdr
    /// laid it out.
    pub(super) fn take_back_created_tab(&mut self, tab_id: &str) {
        let Some(index) = self
            .provisional_tabs
            .iter()
            .position(|tab| tab.tab_id == tab_id)
        else {
            return;
        };
        let tab = self.provisional_tabs.remove(index);
        crate::diagnostic!(serde_json::json!({
            "component": "tab_control",
            "kind": "tab.create.taken_back",
            "tab_id": tab.tab_id,
            "pane_id": tab.pane_id,
        }));
        self.request_republish();
    }

    /// The Herdr workspace that holds the checkout at `path`, as the session
    /// last reported it.
    pub(super) fn owner_workspace_of_path(&self, path: &str) -> Option<String> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == path)
            .and_then(|checkout| checkout.owner_workspace_id.clone())
    }

    /// Takes back a created tab Herdr's layout has not shown within its
    /// bound, and says whether one went.
    pub(super) fn expire_provisional_tabs(&mut self, now_unix_ms: u64) -> bool {
        let before = self.provisional_tabs.len();
        self.provisional_tabs.retain(|tab| {
            let live =
                now_unix_ms.saturating_sub(tab.recorded_at_unix_ms) < PROVISIONAL_TAB_TIMEOUT_MS;
            if !live {
                crate::diagnostic!(serde_json::json!({
                    "component": "tab_control",
                    "kind": "tab.create.layout_late",
                    "tab_id": tab.tab_id,
                    "pane_id": tab.pane_id,
                }));
            }
            live
        });
        let expired = self.provisional_tabs.len() != before;
        if expired {
            self.request_republish();
        }
        expired
    }

    /// A created tab with no layout in the session yet gets its tab row, a
    /// one-pane layout the size of its workspace's other tabs, and a pane row
    /// with the folder it was created in. A tab the session already lays out
    /// is Herdr's from now on.
    fn overlay_created_tabs(&mut self, payload: &mut SessionSnapshotPayload) {
        if self.provisional_tabs.is_empty() {
            return;
        }
        self.provisional_tabs.retain(|tab| {
            !payload
                .layouts
                .iter()
                .any(|layout| layout.tab_id == tab.tab_id)
        });
        let now = unix_milliseconds();
        self.expire_provisional_tabs(now);
        for tab in self.provisional_tabs.clone() {
            let Some(workspace_id) = tab.workspace_id else {
                continue;
            };
            if !payload.tabs.iter().any(|row| row.tab_id == tab.tab_id) {
                let number = payload
                    .tabs
                    .iter()
                    .filter(|row| row.workspace_id == workspace_id)
                    .map(|row| row.number)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                payload.tabs.push(crate::sidebar::SessionTabPayload {
                    number,
                    tab_id: tab.tab_id.clone(),
                    workspace_id: workspace_id.clone(),
                    label: tab.label.clone(),
                });
            }
            let area = payload
                .layouts
                .iter()
                .find(|layout| layout.workspace_id == workspace_id)
                .map(|layout| layout.area)
                .unwrap_or(crate::sidebar::SessionLayoutRect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 24,
                });
            payload.layouts.push(crate::sidebar::SessionLayoutPayload {
                workspace_id,
                tab_id: tab.tab_id.clone(),
                zoomed: false,
                area,
                focused_pane_id: tab.pane_id.clone(),
                panes: vec![crate::sidebar::SessionLayoutPanePayload {
                    pane_id: tab.pane_id.clone(),
                    rect: area,
                }],
                splits: Vec::new(),
            });
            if !payload.panes.iter().any(|pane| pane.pane_id == tab.pane_id) {
                payload.panes.push(crate::sidebar::SessionPanePayload {
                    terminal_id: None,
                    failed_terminal_id: None,
                    foreground_process: None,
                    pane_id: tab.pane_id.clone(),
                    tokens: Default::default(),
                    cwd: Some(tab.cwd.clone()),
                    label: None,
                    terminal_title: None,
                });
            }
            if let Some(id) = tab.timing_id.as_deref() {
                self.op_timings.predicted(id);
            }
        }
    }

    /// Lays every tab's line over a session about to be ingested (D-07): a
    /// tab with predictions gets its predicted layout, and a split's new
    /// pane gets a pane row with its target's cwd and nothing Herdr has not
    /// said. A terminal's grid follows the drawn rectangle at once (PRD
    /// grid-ahead-of-herdr D-03): the PTY size is the controller's to set,
    /// never Herdr's layout's, so nothing here holds a resize.
    ///
    /// The caller runs it after the session's confirmations have read Herdr's
    /// own layout, so nothing Hide drew can confirm an operation.
    fn overlay_geometry(&mut self, payload: &mut SessionSnapshotPayload) {
        let mut scopes = self
            .pane_operations
            .values()
            .filter(|operation| operation.prediction.is_some())
            .map(|operation| operation.scope_id.clone())
            .collect::<HashSet<_>>();
        scopes.extend(
            self.close_operations
                .values()
                .filter(|operation| {
                    matches!(
                        operation.phase.as_str(),
                        "preparing" | "transmitting" | "awaiting_topology"
                    ) && operation.leaving_tab().is_none()
                })
                .map(|operation| operation.scope_id.clone()),
        );
        let mut drawn_closes = HashMap::new();
        let mut confirmed = HashMap::new();
        for scope_id in scopes {
            // A closing pane leaves the canvas only (`draw_closes`): its row,
            // its session and its read state stay until Herdr confirms, so
            // the close can still record it and a refusal finds it intact.
            let (closes, predictions): (Vec<_>, Vec<_>) = self
                .geometry_predictions(&scope_id)
                .into_iter()
                .partition(|prediction| {
                    matches!(prediction, super::pane_prediction::Prediction::Close { .. })
                });
            let Some(index) = payload
                .layouts
                .iter()
                .position(|layout| layout.tab_id == scope_id)
            else {
                continue;
            };
            // What Herdr confirmed, which the next request is predicted on.
            if let Ok(layout) = crate::live::project_layout(&payload.layouts[index]) {
                confirmed.insert(scope_id.clone(), layout);
            }
            let overlaid = super::pane_prediction::overlay_session_layout(
                &payload.layouts[index],
                &predictions,
            );
            if overlaid.is_none() && closes.is_empty() {
                continue;
            }
            if !closes.is_empty() {
                drawn_closes.insert(scope_id.clone(), closes);
            }
            self.mark_geometry_drawn(&scope_id);
            let Some(overlaid) = overlaid else {
                continue;
            };
            for prediction in &predictions {
                let super::pane_prediction::Prediction::Split {
                    target, created, ..
                } = prediction
                else {
                    continue;
                };
                if payload.panes.iter().any(|pane| pane.pane_id == *created) {
                    continue;
                }
                let cwd = payload
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == *target)
                    .and_then(|pane| pane.cwd.clone());
                payload.panes.push(crate::sidebar::SessionPanePayload {
                    terminal_id: None,
                    failed_terminal_id: None,
                    foreground_process: None,
                    pane_id: created.clone(),
                    tokens: Default::default(),
                    cwd,
                    label: None,
                    terminal_title: None,
                });
            }
            payload.layouts[index] = overlaid;
        }
        self.drawn_closes = drawn_closes;
        self.confirmed_layouts = confirmed;
    }

    /// The stage records of the tab's predictions are drawn with the next
    /// snapshot the browser reads.
    fn mark_geometry_drawn(&mut self, scope_id: &str) {
        let drawn = self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == scope_id && operation.prediction.is_some())
            .map(|operation| operation.id.clone())
            .chain(
                self.close_operations
                    .values()
                    .filter(|operation| operation.scope_id == scope_id)
                    .map(|operation| super::op_timing::pane_close_op_id(&operation.request.key)),
            )
            .collect::<Vec<_>>();
        for id in drawn {
            self.op_timings.predicted(&id);
        }
    }

    /// `layout` less the panes a close is taking away (D-07): the canvas
    /// shows the sibling in the closed pane's place at once.
    pub(super) fn draw_closes(&self, layout: PaneLayoutSnapshot) -> PaneLayoutSnapshot {
        match self.drawn_closes.get(&layout.tab_id) {
            Some(closes) => super::pane_prediction::predict(&layout, closes),
            None => layout,
        }
    }
}
