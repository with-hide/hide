//! Agent adapter: workspace-scoped intents and projection of Hide-owned tab groups.
use super::view_areas::ViewWorkspace;
use super::workspace_view::WorkspaceKey;
use super::*;
use crate::agent_layout::{Layout, Tab};
use crate::split_tree::{AreaItem, Edge, LayoutError};
use serde::Deserialize;
use std::borrow::Cow;

#[derive(Debug, Deserialize)]
pub(super) struct AgentLayoutPayload {
    workspace: ViewWorkspace,
    #[serde(flatten)]
    action: AgentLayoutAction,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum AgentLayoutAction {
    Focus {
        tab_id: String,
    },
    FocusArea {
        area_id: String,
    },
    Move {
        tab_id: String,
        area_id: String,
        index: usize,
    },
    Split {
        tab_id: String,
        area_id: String,
        edge: Edge,
        request_id: String,
    },
    Resize {
        split_id: String,
        ratio: f32,
    },
}

impl Runtime {
    /// Both authoritative topology and resolved ownership enter the same
    /// admission policy; lineage changes do not create a second placement rule.
    pub(super) fn reconcile_agent_topology(
        &mut self,
        key: &WorkspaceKey,
        topology: &[(String, bool)],
    ) -> Result<bool, LayoutError> {
        // Only this machine's tabs are created with a claim on their place.
        let reserved = if key.0 == self.node.as_str() {
            self.pending_agent_admissions(&key.1)
        } else {
            0
        };
        let Some(store) = self.workspace_views.as_mut() else {
            return Ok(false);
        };
        let admitted = store
            .agent_placements
            .iter()
            .filter(|(_, (scope, _, _))| scope == key)
            .map(|(id, _)| id.clone())
            .collect();
        let changed = store
            .views
            .entry(&key.0, &key.1)
            .agent_layout
            .reconcile_admissions(topology, reserved, &admitted)?;
        // A tab that left Herdr takes its View bookmark with it (D-11).
        let listed: Vec<&str> = topology.iter().map(|(id, _)| id.as_str()).collect();
        // Not `||`: the prune must run whether or not the layout changed.
        Ok(self.prune_view_callers(key, &listed) | changed)
    }

    pub(super) fn sync_agent_lineage_layouts(&mut self) {
        let topologies = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.device_id == self.node.as_str())
            .flat_map(|workspace| {
                workspace.checkouts.iter().map(|checkout| {
                    (
                        (workspace.device_id.clone(), checkout.path.clone()),
                        checkout
                            .tabs
                            .iter()
                            .filter_map(|tab| Some((tab.id.clone()?, tab.delegated)))
                            .collect::<Vec<_>>(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (key, topology) in topologies {
            match self.reconcile_agent_topology(&key, &topology) {
                Ok(updated) => changed |= updated,
                Err(error) => self.push_diagnostic(
                    "agent_layout.reconcile_refused",
                    format!("Lineage placement for {}: {error:?}", key.1),
                ),
            }
        }
        if changed {
            self.persist_workspace_views();
        }
        // Identity loss can turn a selected delegated canvas into overflow.
        // Repair every checkout's visible identity and the keyboard together,
        // without waiting for the next local session snapshot.
        let mut replacements = Vec::new();
        for workspace in &self.snapshot.navigator.workspaces {
            if workspace.device_id != self.node.as_str() {
                continue;
            }
            for checkout in &workspace.checkouts {
                let key = (workspace.device_id.clone(), checkout.path.clone());
                let Some(layout) = self.agent_layout_of(&key) else {
                    continue;
                };
                let waiting = |tab: &TabSnapshot| {
                    !tab.delegated
                        && tab
                            .id
                            .as_deref()
                            .is_some_and(|id| layout.tree.display(id).is_none())
                };
                let selected_waiting = checkout.tabs.iter().any(|tab| {
                    waiting(tab)
                        && tab
                            .panes
                            .iter()
                            .any(|pane| self.snapshot.terminal.pane_id.as_ref() == Some(&pane.id))
                });
                let visible_waiting = checkout
                    .tabs
                    .iter()
                    .any(|tab| waiting(tab) && tab.id == checkout.active_tab_id);
                if !selected_waiting && !visible_waiting {
                    continue;
                }
                let tab = layout.active().and_then(|id| {
                    checkout
                        .tabs
                        .iter()
                        .find(|tab| tab.id.as_deref() == Some(id))
                });
                let tab_id = tab.and_then(|tab| tab.id.clone());
                let pane = tab.and_then(|tab| {
                    self.tab_focus_pane_id(
                        tab.id.as_deref().expect("matched a stable tab"),
                        tab.panes.first().map(|p| p.id.clone()),
                    )
                });
                replacements.push((checkout.id.clone(), tab_id, selected_waiting, pane));
            }
        }
        for (checkout_id, tab_id, selected_waiting, pane) in replacements {
            if let Some(id) = &tab_id {
                self.visible_tab_ids.insert(checkout_id.clone(), id.clone());
            } else {
                self.visible_tab_ids.remove(&checkout_id);
            }
            if let Some(checkout) = self
                .snapshot
                .navigator
                .workspaces
                .iter_mut()
                .flat_map(|workspace| &mut workspace.checkouts)
                .find(|checkout| checkout.id == checkout_id)
            {
                checkout.active_tab_id = tab_id;
            }
            if selected_waiting {
                self.select_terminal_pane(pane);
                self.operator_focused_pane_id = None;
            }
        }
    }
    /// Common stale-frame boundary for both independently owned columns.
    pub(super) fn area_workspace_is_current(&mut self, key: &WorkspaceKey, column: &str) -> bool {
        if self.front_workspace_key().as_ref() == Some(key) {
            return true;
        }
        self.push_diagnostic(
            format!("{column}_layout.stale_workspace"),
            format!(
                "Ignored area intent for {} on {}; the front Workspace is {:?}",
                key.1,
                key.0,
                self.front_workspace_key()
            ),
        );
        false
    }

    pub(super) fn agent_layout_of(&self, key: &WorkspaceKey) -> Option<&Layout> {
        Some(
            &self
                .workspace_views
                .as_ref()?
                .views
                .get(&key.0, &key.1)?
                .agent_layout,
        )
    }

    /// Count external effects that can still add a tab, together with placed
    /// items. The existing worker/close/reopen owners release these claims.
    pub(super) fn admit_agent_tab(&mut self, path: &str) -> bool {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        let Some(store) = self.workspace_views.as_mut() else {
            return true;
        };
        store.views.entry(&key.0, &key.1);
        let layout = self.agent_layout_of(&key).expect("entry exists");
        let placed = layout.tree.display_count();
        let store = self.workspace_views.as_ref().expect("layout exists");
        let arriving = store
            .agent_placements
            .iter()
            .filter(|(id, (scope, _, _))| scope == &key && layout.tree.display(id).is_none())
            .count();
        if placed + arriving + self.pending_agent_admissions(path) < Tab::LIMITS.items {
            return true;
        }
        self.set_error(
            "agent_layout.display_limit",
            "64 Agent tabs are placed or opening. Close a tab to make room.",
            false,
        );
        false
    }

    pub(super) fn pending_agent_admissions(&self, path: &str) -> usize {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        self.workspace_views
            .as_ref()
            .and_then(|store| store.agent_admissions.get(&key))
            .map_or(0, HashSet::len)
    }

    pub(super) fn has_agent_effect(&self, path: &str, claim: &str) -> bool {
        self.workspace_views
            .as_ref()
            .and_then(|store| {
                store
                    .agent_admissions
                    .get(&(self.node.as_str().to_owned(), path.to_owned()))
            })
            .is_some_and(|claims| claims.contains(claim))
    }

    pub(super) fn reserve_agent_effect(&mut self, path: &str, claim: &str) -> bool {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        if self
            .workspace_views
            .as_ref()
            .and_then(|store| store.agent_admissions.get(&key))
            .is_some_and(|claims| claims.contains(claim))
        {
            return true;
        }
        if !self.admit_agent_tab(path) {
            return false;
        }
        if let Some(store) = self.workspace_views.as_mut() {
            store
                .agent_admissions
                .entry(key)
                .or_default()
                .insert(claim.to_owned());
        }
        true
    }

    pub(super) fn agent_can_show_created(&self, path: &str, id: &str) -> bool {
        self.agent_layout_of(&(self.node.as_str().to_owned(), path.to_owned()))
            .is_none_or(|layout| {
                layout.tree.display(id).is_some() || layout.tree.display_count() < Tab::LIMITS.items
            })
    }

    pub(super) fn agent_tab_waiting(&self, tab_id: &str) -> bool {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|w| w.device_id == self.node.as_str())
            .flat_map(|w| &w.checkouts)
            .any(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id) && !tab.delegated)
                    && self.agent_tab_waiting_in(
                        &(self.node.as_str().to_owned(), checkout.path.clone()),
                        tab_id,
                    )
            })
    }

    pub(super) fn agent_tab_waiting_in(&self, key: &WorkspaceKey, tab_id: &str) -> bool {
        // Delegated tabs live in canvases, not the ordinary admission tree.
        // Their absence from that tree is not an overflow refusal.
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == key.1)
            .any(|checkout| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.id.as_deref() == Some(tab_id) && !tab.delegated)
            })
            && self
                .agent_layout_of(key)
                .is_some_and(|layout| layout.tree.display(tab_id).is_none())
    }

    pub(super) fn finish_agent_admission(&mut self, path: &str, id: u64) {
        self.finish_agent_effect(path, &format!("create:{id}"));
    }

    pub(super) fn finish_agent_effect(&mut self, path: &str, claim: &str) {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        if let Some(store) = self.workspace_views.as_mut()
            && let Some(pending) = store.agent_admissions.get_mut(&key)
        {
            pending.remove(claim);
            if pending.is_empty() {
                store.agent_admissions.remove(&key);
            }
        }
    }

    pub(super) fn apply_agent_layout(&mut self, payload: AgentLayoutPayload) -> bool {
        let key = (payload.workspace.device_id, payload.workspace.path);
        if !self.area_workspace_is_current(&key, "agent") {
            return true;
        }
        if !self.arranges_agent_areas(&key.0) || self.workspace_views.is_none() {
            self.set_error(
                "agent_layout.unsupported",
                "Agent groups are not available on SSH devices",
                false,
            );
            return true;
        }
        let request = match &payload.action {
            AgentLayoutAction::Split { request_id, .. } => Some(format!("agent:{request_id}")),
            _ => None,
        };
        if request
            .as_ref()
            .is_some_and(|id| self.split_request_seen(&key, id))
        {
            return false;
        }
        let layout = &mut self
            .workspace_views
            .as_mut()
            .expect("checked")
            .views
            .entry(&key.0, &key.1)
            .agent_layout;
        let before = layout.clone();
        let stamp = layout.tree.next_stamp(unix_milliseconds());
        let resize = matches!(payload.action, AgentLayoutAction::Resize { .. });
        let replaces_canvas = matches!(
            payload.action,
            AgentLayoutAction::Focus { .. }
                | AgentLayoutAction::Move { .. }
                | AgentLayoutAction::Split { .. }
        );
        let outcome = match payload.action {
            AgentLayoutAction::Focus { tab_id } => layout.select(&tab_id, false, stamp),
            AgentLayoutAction::FocusArea { area_id } => layout.tree.focus_area(&area_id, stamp),
            AgentLayoutAction::Move {
                tab_id,
                area_id,
                index,
            } => layout.tree.move_display(&tab_id, &area_id, index, stamp),
            AgentLayoutAction::Split {
                tab_id,
                area_id,
                edge,
                ..
            } => layout
                .tree
                .split(&tab_id, &area_id, edge, stamp)
                .map(|_| true),
            AgentLayoutAction::Resize { split_id, ratio } => layout.tree.resize(&split_id, ratio),
        };
        if let Err(error) = outcome {
            self.push_diagnostic(
                "agent_layout.refused",
                format!("Agent layout intent refused: {error:?}"),
            );
            let message = match error {
                LayoutError::AreaLimit => "At most six Agent areas can be open",
                LayoutError::DepthLimit => "Agent areas can be split at most three levels deep",
                LayoutError::DisplayLimit => "At most 64 Agent tabs can be arranged",
                LayoutError::NothingToSplit => "The area's only tab cannot split its own area",
                LayoutError::InvalidRatio => "The divider position is invalid",
                _ => "The Agent tab or area is no longer available",
            };
            self.set_error("agent_layout.refused", message, false);
            return true;
        }
        // Moving a normal tab into an area replaces its delegated canvas.
        if replaces_canvas {
            layout.canvases.remove(&layout.tree.active_area);
        }
        layout
            .canvases
            .retain(|area, _| layout.tree.area(area).is_some());
        let changed = before != *layout;
        let tab = layout.active().map(str::to_owned);
        if let Some(request) = request {
            self.remember_split_request(&key, request);
        }
        if changed {
            self.persist_workspace_views();
        }
        if key.0 != self.node.as_str() {
            // A node's Herdr shows the tab its keyboard is on, so the active
            // area's tab is asked of it; the areas beside it attach now.
            if !resize && let Some(tab_id) = tab {
                self.focus_node_tab(&key.0, tab_id, stamp);
            }
            self.reconcile_remote_terminal_selection();
            return changed;
        }
        if !resize
            && let Some(tab_id) = tab
            && let Some((workspace_id, checkout_id)) = self
                .front_checkout()
                .map(|(w, c)| (w.to_owned(), c.to_owned()))
        {
            let prior = self.focused_visible_tab_id();
            self.apply(Event::FocusTab(FocusTabPayload {
                workspace_id,
                checkout_id,
                tab_id,
                focus_device: false,
            }));
            self.agent_sleep_visit(prior.as_deref());
        }
        changed
    }

    /// Whether `device`'s Workspaces arrange their tabs in Agent areas: this
    /// machine's, and a node that dialed in, whose Workspaces the core
    /// arranged as its own before the core moved off that machine (PRD
    /// core-host-node-move D-29). A device this core dials shows the one tab
    /// its Herdr has in front.
    pub(super) fn arranges_agent_areas(&self, device: &str) -> bool {
        device == self.node.as_str() || self.link_origin(device) == Some(&LinkOrigin::Inbound)
    }

    /// Asks a node's Herdr to bring `tab_id` forward, unless it already has.
    fn focus_node_tab(&mut self, device: &str, tab_id: String, stamp: u64) {
        let in_front = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_ref())
            .and_then(|session| session.focused_tab_id.as_deref())
            == Some(tab_id.as_str());
        if in_front {
            return;
        }
        self.request_remote_control(RemoteControlPayload {
            target_id: device.to_owned(),
            request_id: format!("agent-area:{stamp}:{tab_id}"),
            report_pane_focus_outcome: false,
            focus_device: false,
            request: RemoteControlRequest::FocusTab { tab_id },
        });
    }

    /// The tab the front Workspace shows and whether it is delegated: Hide's
    /// choice on this machine, the node Herdr's own on a node.
    fn front_visible_tab(&self, key: &WorkspaceKey) -> Option<(String, bool)> {
        if key.0 == self.node.as_str() {
            let tab_id = self.focused_visible_tab_id()?;
            let delegated = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|w| &w.checkouts)
                .filter(|c| c.path == key.1)
                .flat_map(|c| &c.tabs)
                .find(|tab| tab.id.as_deref() == Some(&tab_id))?
                .delegated;
            return Some((tab_id, delegated));
        }
        let session = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == key.0)?
            .session
            .as_ref()?;
        let checkout = session
            .workspaces
            .iter()
            .flat_map(|w| &w.checkouts)
            .find(|c| c.path == key.1)?;
        let tab_id = session
            .focused_tab_id
            .as_ref()
            .or_else(|| session.active_tab_ids.get(&checkout.id))?;
        let delegated = checkout
            .tabs
            .iter()
            .find(|tab| tab.id.as_ref() == Some(tab_id))?
            .delegated;
        Some((tab_id.clone(), delegated))
    }

    /// The tabs a node's front Workspace shows in its Agent areas, whose
    /// panes attach as the tab its Herdr has in front does; none while
    /// another machine is in front.
    pub(super) fn node_shown_agent_tabs(&self, device: &str) -> Vec<String> {
        if device == self.node.as_str() || !self.arranges_agent_areas(device) {
            return Vec::new();
        }
        self.front_workspace_key()
            .filter(|key| key.0 == device)
            .and_then(|key| {
                self.agent_layout_of(&key)
                    .map(|layout| self.shown_agent_layout(&key, layout).shown())
            })
            .unwrap_or_default()
    }

    /// Takes a node's tabs as they stand into its Workspaces' Agent areas,
    /// as a local session does for this machine's (`reconcile_agent_topology`).
    /// An empty checkout clears its saved tree only when the catalog is
    /// fully answered, so an unresolved startup placeholder cannot erase it.
    pub(super) fn reconcile_node_agent_topology(&mut self, device: &str) {
        if device == self.node.as_str()
            || !self.arranges_agent_areas(device)
            || self.workspace_views.is_none()
        {
            return;
        }
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device)
            .and_then(|status| status.session.as_ref())
        else {
            return;
        };
        let empty_authoritative = self.device_catalog_ready(device);
        let topologies = session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| !checkout.tabs.is_empty() || empty_authoritative)
            .map(|checkout| {
                (
                    (device.to_owned(), checkout.path.clone()),
                    checkout
                        .tabs
                        .iter()
                        .filter_map(|tab| Some((tab.id.clone()?, tab.delegated)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let mut changed = false;
        for (key, topology) in topologies {
            match self.reconcile_agent_topology(&key, &topology) {
                Ok(updated) => changed |= updated,
                Err(error) => self.push_diagnostic(
                    "agent_layout.reconcile_refused",
                    format!("Node {device} placement for {}: {error:?}", key.1),
                ),
            }
        }
        if changed {
            self.persist_workspace_views();
        }
    }

    /// A committed pane/tab selection finds its owning area; merely drawing
    /// another live canvas never enters this path or marks its agents read.
    pub(super) fn sync_agent_selection(&mut self) {
        let Some(key) = self
            .front_workspace_key()
            .filter(|key| self.arranges_agent_areas(&key.0))
        else {
            return;
        };
        let Some((tab_id, delegated)) = self.front_visible_tab(&key) else {
            return;
        };
        let Some(layout) = self.agent_layout_of(&key) else {
            return;
        };
        if layout.active() == Some(&tab_id) {
            return;
        }
        let layout = &mut self
            .workspace_views
            .as_mut()
            .expect("layout exists")
            .views
            .entry(&key.0, &key.1)
            .agent_layout;
        if let Err(error) = layout.select(
            &tab_id,
            delegated,
            layout.tree.next_stamp(unix_milliseconds()),
        ) {
            self.push_diagnostic(
                "agent_layout.selection_unavailable",
                format!("Could not place selected tab {tab_id}: {error:?}"),
            );
        } else {
            self.persist_workspace_views();
            if key.0 != self.node.as_str() {
                self.reconcile_remote_terminal_selection();
            }
        }
    }

    pub(super) fn shown_agent_tabs(&self) -> Vec<String> {
        if let Some(key) = self
            .front_workspace_key()
            .filter(|key| key.0 == self.node.as_str())
            && let Some(layout) = self.agent_layout_of(&key)
        {
            let shown = self.shown_agent_layout(&key, layout).shown();
            if !shown.is_empty() {
                return shown;
            }
        }
        self.focused_visible_tab_id().into_iter().collect()
    }

    /// What a Workspace's Agent areas show: the stored layout less every tab
    /// a local close is taking away (`PendingClose::leaving_tab`), so its chip
    /// leaves when the close is approved and each area shows what it will show
    /// once Herdr confirms. The stored layout keeps the tab, so a close that
    /// does not happen puts it back where it stood. Nothing is cloned while
    /// no close is running.
    pub(super) fn shown_agent_layout<'a>(
        &self,
        key: &WorkspaceKey,
        layout: &'a Layout,
    ) -> Cow<'a, Layout> {
        if key.0 != self.node.as_str() {
            return Cow::Borrowed(layout);
        }
        let leaving = self
            .close_operations
            .values()
            .filter(|operation| operation.request.context.checkout_path == key.1)
            .filter_map(|operation| operation.leaving_tab())
            .filter(|id| {
                layout.tree.display(id).is_some() || layout.canvases.values().any(|tab| tab == id)
            })
            .collect::<Vec<_>>();
        if leaving.is_empty() {
            return Cow::Borrowed(layout);
        }
        let mut shown = layout.clone();
        for id in leaving {
            shown.tree.remove(id);
            shown.canvases.retain(|_, tab| tab != id);
        }
        shown
            .canvases
            .retain(|area, _| shown.tree.area(area).is_some());
        Cow::Owned(shown)
    }

    pub(super) fn agent_layout_snapshot(
        &self,
        key: &WorkspaceKey,
        layout: &Layout,
    ) -> crate::model::AgentLayoutSnapshot {
        let layout = self.shown_agent_layout(key, layout);
        crate::model::AgentLayoutSnapshot {
            waiting: layout.waiting,
            root: layout.tree.root.clone(),
            active_area: layout.tree.active_area.clone(),
            canvases: layout.canvases.clone(),
            limits: crate::model::ViewLimitsSnapshot {
                areas: Tab::LIMITS.areas,
                depth: Tab::LIMITS.depth,
                displays: Tab::LIMITS.items,
            },
            display_count: layout.tree.display_count(),
        }
    }
}
