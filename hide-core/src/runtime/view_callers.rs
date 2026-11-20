//! Caller placement on the checkout-owned View list.

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::workspace_control::Caller;
use crate::workspace_views::Place;

/// The selection an open the operator does not watch leaves as it found it
/// (PRD lane-desk-pair-view B1): on each tree of the checkout, the area in
/// use and the View in front of each area, and the editor's active tab.
pub(super) struct Kept {
    key: WorkspaceKey,
    editor_active: Option<String>,
    trees: Vec<(Place, String, Vec<String>)>,
}

impl Runtime {
    fn agent_view_of(&self, key: &WorkspaceKey) -> (Vec<&str>, Option<&str>) {
        if let Some(layout) = self.agent_layout_of(key) {
            let shown: Vec<&str> = layout
                .tree
                .areas()
                .into_iter()
                .filter_map(|area| layout.shown_in(&area.id))
                .collect();
            return (shown, layout.active());
        }
        let active = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == key.1)
            .and_then(|checkout| checkout.active_tab_id.as_deref());
        (active.into_iter().collect(), active)
    }

    /// The Workspace's active Agent tab: the one whose panel it is.
    pub(super) fn workspace_active_tab(&self, key: &WorkspaceKey) -> Option<String> {
        self.agent_view_of(key).1.map(str::to_owned)
    }

    /// The engine tab of a pane caller in the Workspace `key`; a checkout
    /// caller has none.
    pub(super) fn control_caller_tab(&self, key: &WorkspaceKey, caller_id: &str) -> Option<String> {
        let Caller::Pane(pane_id) = Caller::parse(caller_id) else {
            return None;
        };
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == key.1)?
            .tabs
            .iter()
            .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
            .and_then(|tab| tab.id.clone())
    }

    /// Where a Workspace command's open or select lands (PRD
    /// lane-desk-pair-view B1, B2, D-05, D-36): the tray of the tab of the
    /// pane the operator watches, the selected pane of the Workspace in
    /// front, when the caller is that pane; for any other agent's pane, on
    /// no tray, where the View waits in its pane's list with the To look at
    /// mark. A Workspace not on screen has no watched pane (conductor
    /// decision 5). A caller with no agent to list the View (a plain
    /// shell's pane, the window's browser gateway, Desk, a checkout or an
    /// external tool's own connection) opens as the operator does, on the
    /// tray in front, where nothing else would show it.
    pub(super) fn control_place(&self, key: &WorkspaceKey, caller_id: &str) -> Place {
        match Caller::parse(caller_id) {
            Caller::Pane(_) if self.watched_caller(key, caller_id) => self
                .control_caller_tab(key, caller_id)
                .map_or(Place::Shelf, Place::Tray),
            Caller::Pane(pane) if self.pane_opener(pane).is_ok() => Place::Shelf,
            Caller::Pane(_) => self.front_place(key),
            Caller::Checkout { .. } | Caller::Access { .. } | Caller::Desk => self.front_place(key),
        }
    }

    /// Whether an open from `caller_id` comes in front, lane permitting:
    /// the watched pane's agent's, the gateway's and Desk's do; a checkout's
    /// or a tool connection's lands behind what the operator looks at, and
    /// any other agent's waits on no tray (B1).
    pub(super) fn watched_caller(&self, key: &WorkspaceKey, caller_id: &str) -> bool {
        match Caller::parse(caller_id) {
            Caller::Checkout { gateway: true, .. } | Caller::Desk => true,
            Caller::Pane(pane) => {
                self.front_workspace_key().as_ref() == Some(key)
                    && self.operator_pane(&key.0) == Some(pane)
            }
            Caller::Checkout { .. } | Caller::Access { .. } => false,
        }
    }

    /// The pane the operator is in on `device`, the device in front: this
    /// machine's selected pane, or the pane a device's engine has focused,
    /// which the shell draws focused and follows (`remoteView` in
    /// `web/src/remote.ts`).
    pub(super) fn operator_pane(&self, device: &str) -> Option<&str> {
        if device == self.node.as_str() {
            return self.snapshot.terminal.pane_id.as_deref();
        }
        self.snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_ref())
            .and_then(|session| session.focused_pane_id.as_deref())
    }

    /// What an open the operator does not watch leaves as it was (B1): the
    /// fronts of every tree of the checkout, so a View it takes from a tray
    /// or adds to one never comes in front.
    pub(super) fn keep_front(&self, key: &WorkspaceKey) -> Kept {
        let trees = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .map(|view| {
                view.layouts()
                    .map(|(place, layout)| {
                        (
                            place,
                            layout.active_area.clone(),
                            layout
                                .areas()
                                .into_iter()
                                .filter_map(|area| area.active.clone())
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Kept {
            key: key.clone(),
            editor_active: self.snapshot.editor.active_tab_id.clone(),
            trees,
        }
    }

    /// Puts the selection back after an open the operator does not watch:
    /// the View stays where it landed, in no area's front and never in the
    /// keyboard's.
    pub(super) fn restore_front(&mut self, kept: Kept) {
        self.snapshot.editor.active_tab_id = kept.editor_active;
        for (place, active_area, fronts) in kept.trees {
            if self.view_layout_at(&kept.key, &place).is_none() {
                continue;
            }
            let outcome = self.change_view_layout(&kept.key, &place, |layout, stamp| {
                let mut changed = false;
                for display in &fronts {
                    if layout.display(display).is_some() {
                        changed |= layout.show(display, stamp).unwrap_or(false);
                    }
                }
                if layout.area(&active_area).is_some() && layout.active_area != active_area {
                    layout.active_area = active_area.clone();
                    changed = true;
                }
                Ok(((), changed))
            });
            if let Err(error) = outcome {
                self.push_diagnostic("view_layout.front_not_restored", error.message());
            }
        }
    }

    /// The engine listed the Agent tabs `tabs` of `key`: a tab that left
    /// takes its pending reads and its tray with it, its agents' Views
    /// staying in their panes' lists and the operator's going to the desk in
    /// front. Whether anything changed.
    pub(super) fn prune_view_callers(&mut self, key: &WorkspaceKey, tabs: &[&str]) -> bool {
        let reads = self.prune_gui_reads(key, tabs);
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
        else {
            return reads;
        };
        let gone: Vec<String> = view
            .trays
            .iter()
            .filter(|tray| {
                tray.tab != crate::workspace_views::PENDING_TRAY
                    && !tabs.contains(&tray.tab.as_str())
            })
            .map(|tray| tray.tab.clone())
            .collect();
        if gone.is_empty() {
            return reads;
        }
        // The operator's Views of a tab that left go to the desk in front,
        // or wait on the pending desk while no tab of the checkout is left.
        let heir = match self.front_place(key) {
            Place::Tray(tab) if tabs.contains(&tab.as_str()) => tab,
            _ => crate::workspace_views::PENDING_TRAY.to_owned(),
        };
        let store = self.workspace_views.as_mut().expect("read above");
        let view = store.views.entry(&key.0, &key.1);
        for tab in &gone {
            let retired = view.retire_tray(tab, &heir);
            crate::diagnostic!(serde_json::json!({
                "component": "view_layout", "kind": "tray.tab_gone",
                "device": key.0, "tab": tab, "heir": heir,
                "shelved": retired.shelved, "inherited": retired.inherited,
                "dropped": retired.dropped,
            }));
        }
        store.generation += 1;
        self.persist_workspace_views();
        true
    }

    /// Ends pair view when the pane it drew, as last published, is one the
    /// catalog no longer lists in its tab (PRD lane-desk-pair-view B28, D-22,
    /// D-46), without a notice. Runs once a new catalog is in place, so it
    /// reads the panes as they are now; a tab the catalog does not list is
    /// left to the tab prune, which an unanswered catalog does not run.
    /// Whether it ended.
    pub(super) fn end_pairs_of_gone_panes(&mut self) -> bool {
        let Some((key, tab, pane)) = self.published_pair() else {
            return false;
        };
        let listed: Option<Vec<&str>> = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == key.1)
            .flat_map(|checkout| &checkout.tabs)
            .find(|held| held.id.as_deref() == Some(tab.as_str()))
            .map(|held| held.panes.iter().map(|pane| pane.id.as_str()).collect());
        if listed.is_some_and(|panes| !panes.contains(&pane.as_str())) {
            return self.end_pair_view(super::pair_view::PairEnd::PaneGone);
        }
        false
    }
}
