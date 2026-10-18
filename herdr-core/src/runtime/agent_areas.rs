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
    #[serde(default)]
    group_id: Option<String>,
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
        // Drawn-ahead tabs may already be in this catalog. They occupy their
        // reserved slot but cannot confirm their own first real appearance.
        let unconfirmed = self
            .provisional_tabs
            .iter()
            .map(|tab| tab.tab_id.as_str())
            .collect();
        let Some(store) = self.workspace_views.as_mut() else {
            return Ok(false);
        };
        let admitted = store
            .agent_placements
            .iter()
            .filter(|(_, (scope, _, _, _))| scope == key)
            .map(|(id, _)| id.clone())
            .collect();
        let changed = store.views.entry(&key.0, &key.1).reconcile_agent_groups(
            topology,
            reserved,
            &admitted,
            &unconfirmed,
        )?;
        // A tab that left Herdr takes its View bookmark with it (D-11).
        let listed: Vec<&str> = topology.iter().map(|(id, _)| id.as_str()).collect();
        // Not `||`: the prune must run whether or not the layout changed.
        Ok(self.prune_view_bookmarks(key, &listed) | changed)
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
                            .is_some_and(|id| self.agent_tab_waiting_in(&key, id))
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
            self.workspace_views
                .as_ref()?
                .views
                .get(&key.0, &key.1)?
                .selected_agent_layout(),
        )
    }

    /// Count external effects that can still add a tab, together with placed
    /// items. The existing worker/close/reopen owners release these claims.
    pub(super) fn admit_agent_tab(&mut self, path: &str) -> bool {
        let key = (self.node.as_str().to_owned(), path.to_owned());
        self.admit_agent_tab_for_workspace(&key)
    }

    pub(super) fn admit_agent_tab_for_workspace(&mut self, key: &WorkspaceKey) -> bool {
        let Some(store) = self.workspace_views.as_mut() else {
            return true;
        };
        store.views.entry(&key.0, &key.1);
        let store = self.workspace_views.as_ref().expect("layout exists");
        let view = store.views.get(&key.0, &key.1).expect("entry exists");
        let placed = view.agent_tab_count();
        let arriving = store
            .agent_placements
            .iter()
            .filter(|(id, (scope, _, _, _))| scope == key && view.agent_tab_group(id).is_none())
            .count();
        let pending = store.agent_admissions.get(key).map_or(0, HashSet::len);
        if placed + arriving + pending < Tab::LIMITS.items {
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
        self.workspace_views
            .as_ref()
            .and_then(|s| s.views.get(self.node.as_str(), path))
            .is_none_or(|view| {
                view.agent_tab_group(id).is_some() || view.agent_tab_count() < Tab::LIMITS.items
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
                .workspace_views
                .as_ref()
                .and_then(|s| s.views.get(&key.0, &key.1))
                .is_some_and(|view| view.agent_tab_group(tab_id).is_none())
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
        if self.workspace_views.is_none() {
            self.set_error(
                "agent_layout.unsupported",
                "Agent layouts are not available in this shell",
                false,
            );
            return true;
        }
        if matches!(payload.action, AgentLayoutAction::Split { .. })
            && !self.supports_multiple_agent_areas(&key.0)
        {
            self.set_error(
                "agent_layout.unsupported",
                "SSH devices support one Agent area per group",
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
        let view = self
            .workspace_views
            .as_mut()
            .expect("checked")
            .views
            .entry(&key.0, &key.1);
        if view.agent_groups.selected != payload.group_id {
            self.set_error(
                "agent_layout.stale_group",
                "The selected group changed. Try again in its current area.",
                false,
            );
            return true;
        }
        if let AgentLayoutAction::Move { tab_id, .. } | AgentLayoutAction::Split { tab_id, .. } =
            &payload.action
            && view.agent_starts.values().any(|start| {
                start.is_pending()
                    && (start.draft_tab_id.as_ref() == Some(tab_id)
                        || start.live_tab_id.as_ref() == Some(tab_id))
            })
        {
            self.set_error(
                "agent_layout.start_pending",
                "This tab is still starting. Check its result before moving it.",
                false,
            );
            return true;
        }
        let layout = view.selected_agent_layout_mut();
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
        let tab = tab.filter(|id| !view.agent_groups.drafts.contains(id));
        if let Some(request) = request {
            self.remember_split_request(&key, request);
        }
        if changed {
            self.persist_workspace_views();
        }
        if !resize {
            let prior = self.focused_visible_tab_id();
            self.show_agent_workspace_tab(&key, tab);
            if key.0 == self.node.as_str() {
                self.agent_sleep_visit(prior.as_deref());
            }
        }
        changed
    }

    /// Every connected device has groups; outbound SSH retains one area.
    pub(super) fn arranges_agent_areas(&self, device: &str) -> bool {
        device == self.node.as_str()
            || self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == device)
    }

    fn supports_multiple_agent_areas(&self, device: &str) -> bool {
        device == self.node.as_str() || self.link_origin(device) == Some(&LinkOrigin::Inbound)
    }

    /// Applies the selected group's active canvas without creating a pane.
    pub(super) fn show_agent_workspace_tab(&mut self, key: &WorkspaceKey, live: Option<String>) {
        if !self.choose_agent_workspace(key) {
            return;
        }
        if key.0 != self.node.as_str() {
            if let Some(tab) = live {
                self.focus_node_tab(&key.0, tab, unix_milliseconds());
            }
            self.reconcile_remote_terminal_selection();
            return;
        }
        let Some((workspace_id, checkout_id)) = self.agent_workspace_ids(key) else {
            return;
        };
        if let Some(tab_id) = live {
            self.apply(Event::FocusTab(FocusTabPayload {
                workspace_id,
                checkout_id,
                tab_id,
                focus_device: false,
            }));
        } else {
            self.visible_tab_ids.remove(&checkout_id);
            if let Some(checkout) = self
                .snapshot
                .navigator
                .workspaces
                .iter_mut()
                .flat_map(|workspace| &mut workspace.checkouts)
                .find(|checkout| checkout.id == checkout_id)
            {
                checkout.active_tab_id = None;
            }
            self.select_terminal_pane(None);
            self.operator_focused_pane_id = None;
        }
    }

    /// An explicit real-agent navigation reveals its owning group. Session
    /// topology never calls this, so it cannot overwrite a draft selection.
    pub(super) fn reveal_remote_agent_tab(
        &mut self,
        key: &WorkspaceKey,
        tab_id: &str,
        delegated: bool,
    ) {
        let group = if delegated {
            let Some(group) = self.delegated_group_owner(key, tab_id) else {
                return;
            };
            group
        } else {
            self.workspace_views
                .as_ref()
                .and_then(|store| store.views.get(&key.0, &key.1))
                .and_then(|view| view.agent_tab_group(tab_id).flatten().map(str::to_owned))
        };
        if let Some(store) = self.workspace_views.as_mut() {
            let view = store.views.entry(&key.0, &key.1);
            let previous_group = view.agent_groups.selected.clone();
            view.agent_groups.selected = group;
            let layout = view.selected_agent_layout_mut();
            let stamp = layout.tree.next_stamp(unix_milliseconds());
            if let Err(error) = layout.select(tab_id, delegated, stamp) {
                view.agent_groups.selected = previous_group;
                self.push_diagnostic(
                    "agent_layout.selection_unavailable",
                    format!("Could not select remote tab {tab_id}: {error:?}"),
                );
                return;
            }
            store.remote_front.insert(key.0.clone(), key.1.clone());
            self.mark_workspace_chosen(key);
            self.persist_workspace_views();
            self.reconcile_remote_terminal_selection();
        }
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
    pub(super) fn node_shown_agent_tabs(&self, device: &str) -> Option<Vec<String>> {
        if device == self.node.as_str() || !self.arranges_agent_areas(device) {
            return None;
        }
        self.front_workspace_key()
            .filter(|key| key.0 == device)
            .and_then(|key| {
                self.agent_layout_of(&key)
                    .map(|layout| self.shown_agent_layout(&key, layout).shown())
            })
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
        if key.0 != self.node.as_str()
            && self
                .workspace_views
                .as_ref()
                .is_some_and(|store| store.remote_front.contains_key(&key.0))
        {
            return;
        }
        let Some((tab_id, delegated)) = self.front_visible_tab(&key) else {
            return;
        };
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|s| s.views.get(&key.0, &key.1))
        else {
            return;
        };
        let group = if delegated {
            let Some(group) = self.delegated_group_owner(&key, &tab_id) else {
                return;
            };
            group
        } else {
            view.agent_tab_group(&tab_id).flatten().map(str::to_owned)
        };
        if view.selected_agent_layout().active() == Some(&tab_id)
            && view.agent_groups.selected == group
        {
            return;
        }
        let view = self
            .workspace_views
            .as_mut()
            .expect("layout exists")
            .views
            .entry(&key.0, &key.1);
        view.agent_groups.selected = group;
        let layout = view.selected_agent_layout_mut();
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

    // An unresolved lineage is different from a root placed in Ungrouped.
    // A partial catalog cannot invalidate a durable delegated canvas.
    fn delegated_group_owner(&self, key: &WorkspaceKey, id: &str) -> Option<Option<String>> {
        let checkout = self
            .catalog_workspaces()
            .filter(|w| w.device_id == key.0)
            .flat_map(|w| &w.checkouts)
            .find(|c| c.path == key.1)?;
        let tab = checkout.tabs.iter().find(|t| t.id.as_deref() == Some(id))?;
        let agents = if key.0 == self.node.as_str() {
            &self.snapshot.navigator.agents
        } else {
            &self
                .snapshot
                .status
                .remote
                .iter()
                .find(|r| r.target_id == key.0)?
                .session
                .as_ref()?
                .agents
        };
        let root = agents
            .iter()
            .filter(|a| tab.panes.iter().any(|p| p.id == a.pane_id))
            .find_map(|a| a.lineage_path_pane_ids.first())?;
        let root_tab = checkout
            .tabs
            .iter()
            .find(|t| t.panes.iter().any(|p| &p.id == root))?
            .id
            .as_deref()?;
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)?
            .agent_tab_group(root_tab)
            .map(|group| group.map(str::to_owned))
    }

    /// Root membership changes invalidate only canvases belonging to that
    /// root. The delegated tab stays alive and explicit navigation reveals it
    /// in its root's new group; the move itself keeps showing the moved root.
    pub(super) fn prune_agent_group_canvases(&mut self, key: &WorkspaceKey) -> bool {
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
        else {
            return false;
        };
        let misplaced: Vec<_> = std::iter::once((None, &view.agent_layout))
            .chain(
                view.agent_groups
                    .items
                    .iter()
                    .map(|group| (Some(group.id.as_str()), &group.layout)),
            )
            .flat_map(|(group, layout)| {
                layout
                    .canvases
                    .iter()
                    .map(move |(area, tab)| (group, area, tab))
            })
            .filter(|(group, _, tab)| {
                self.delegated_group_owner(key, tab)
                    .is_some_and(|owner| owner.as_deref() != *group)
            })
            .map(|(group, area, _)| (group.map(str::to_owned), area.clone()))
            .collect();
        let changed = !misplaced.is_empty();
        let view = self
            .workspace_views
            .as_mut()
            .expect("view exists")
            .views
            .get_mut(&key.0, &key.1)
            .expect("view exists");
        for (group, area) in misplaced {
            if let Some(layout) = view.group_layout_mut(group.as_deref()) {
                layout.canvases.remove(&area);
            }
        }
        changed
    }

    pub(super) fn shown_agent_tabs(&self) -> Vec<String> {
        if let Some(key) = self
            .front_workspace_key()
            .filter(|key| key.0 == self.node.as_str())
            && let Some(layout) = self.agent_layout_of(&key)
        {
            let shown = self.shown_agent_layout(&key, layout).shown();
            let view = self
                .workspace_views
                .as_ref()
                .and_then(|s| s.views.get(&key.0, &key.1));
            return shown
                .into_iter()
                .filter(|id| view.is_none_or(|v| !v.agent_groups.drafts.contains(id)))
                .collect();
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
        let view = self
            .workspace_views
            .as_ref()
            .and_then(|s| s.views.get(&key.0, &key.1));
        let groups = view
            .map(|view| {
                std::iter::once(crate::model::AgentGroupSnapshot {
                    id: None,
                    name: None,
                    tabs: view
                        .agent_layout
                        .tree
                        .displays()
                        .map(|t| t.id.clone())
                        .collect(),
                })
                .chain(
                    view.agent_groups
                        .items
                        .iter()
                        .map(|g| crate::model::AgentGroupSnapshot {
                            id: Some(g.id.clone()),
                            name: Some(g.name.clone()),
                            tabs: g.layout.tree.displays().map(|t| t.id.clone()).collect(),
                        }),
                )
                .collect()
            })
            .unwrap_or_default();
        crate::model::AgentLayoutSnapshot {
            group_id: view.and_then(|v| v.agent_groups.selected.clone()),
            groups,
            drafts: view
                .map(|v| v.agent_groups.drafts.iter().cloned().collect())
                .unwrap_or_default(),
            start_receipts: view.map(super::group_starts::receipts).unwrap_or_default(),
            can_undo_group: view.is_some_and(|v| v.can_undo_agent_group()),
            waiting: layout.waiting,
            root: layout.tree.root.clone(),
            active_area: layout.tree.active_area.clone(),
            canvases: layout.canvases.clone(),
            limits: crate::model::ViewLimitsSnapshot {
                areas: if self.supports_multiple_agent_areas(&key.0) {
                    Tab::LIMITS.areas
                } else {
                    1
                },
                depth: if self.supports_multiple_agent_areas(&key.0) {
                    Tab::LIMITS.depth
                } else {
                    1
                },
                displays: Tab::LIMITS.items,
            },
            display_count: layout.tree.display_count(),
        }
    }
}
