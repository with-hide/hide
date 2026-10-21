//! Existing-terminal relocation. Reservations cover both tabs, a saved intent
//! precedes every external effect, and recovery only reads authoritative state.
use super::*;
use crate::sidebar::SessionSnapshotPayload;
use crate::split_tree::Edge;
use crate::{wire, workspace_views};
use hide_herdr_client::{ApiConnector, request_with_connector};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

const ACTIVE_LIMIT: usize = 16;
const RECEIPT_LIMIT: usize = 128;
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct Workspace {
    device_id: String,
    checkout_path: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct Source {
    pane_id: String,
    tab_id: String,
    area_id: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Destination {
    Pane {
        tab_id: String,
        pane_id: String,
        area_id: String,
        edge: Edge,
    },
    Tab {
        tab_id: String,
        pane_id: String,
        area_id: String,
    },
    NewTab {
        area_id: String,
    },
}
impl Destination {
    fn area(&self) -> &str {
        match self {
            Self::Pane { area_id, .. } | Self::Tab { area_id, .. } | Self::NewTab { area_id } => {
                area_id
            }
        }
    }
    fn target(&self) -> Option<(&str, &str, Edge)> {
        match self {
            Self::Pane {
                tab_id,
                pane_id,
                edge,
                ..
            } => Some((tab_id, pane_id, *edge)),
            Self::Tab {
                tab_id, pane_id, ..
            } => Some((tab_id, pane_id, Edge::Right)),
            Self::NewTab { .. } => None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct Request {
    request_id: String,
    workspace: Workspace,
    #[serde(default)]
    group_id: Option<String>,
    source: Source,
    destination: Destination,
}
#[derive(Debug, Deserialize)]
pub(super) struct Check {
    request_id: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Intent {
    request: Request,
    terminal_id: String,
    workspace_id: String,
    phase: String,
    started_at: u64,
    message: Option<String>,
    #[serde(default)]
    target_terminal: Option<String>,
    #[serde(default)]
    new_tab_slot: bool,
    /// The last acknowledged public identity, never a replacement terminal.
    result_pane: Option<String>,
    result_tab: Option<String>,
}
pub(super) struct Operation {
    intent: Intent,
    /// Only live transactions hide intermediate topology. On restart there is
    /// deliberately no overlay: any surviving temporary tab is reachable.
    baseline: Option<SessionSnapshotPayload>,
    worker: bool,
    remote_baseline: Option<RemoteSessionSnapshot>,
    publication_barrier: Option<std::time::Instant>,
}
pub(super) struct SaveWaiter {
    key: (String, String),
    id: String,
    intent: Intent,
    sender: SyncSender<Result<(), String>>,
}

impl Runtime {
    /// A bootstrap begun after a transaction's final read is authoritative
    /// even if another client changed or closed the pane in the meantime.
    pub(crate) fn release_pane_move_publication_barriers(
        &mut self,
        device: Option<&str>,
        snapshot_started_at: std::time::Instant,
    ) -> bool {
        let device = device.unwrap_or(self.node.as_str());
        let mut changed = false;
        for op in self.pane_moves.values_mut().filter(|op| {
            op.intent.request.workspace.device_id == device
                && op
                    .publication_barrier
                    .is_some_and(|created| created <= snapshot_started_at)
        }) {
            changed = true;
            op.publication_barrier = None;
            op.baseline = None;
            op.remote_baseline = None;
        }
        if changed {
            self.sync_async_operations();
        }
        changed
    }
    pub(super) fn pending_pane_move_tabs(&self, key: &(String, String)) -> usize {
        self.workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .map_or(0, |view| {
                view.pane_moves
                    .values()
                    .filter(|intent| {
                        intent.new_tab_slot
                            && !matches!(intent.phase.as_str(), "completed" | "failed")
                    })
                    .count()
            })
    }
    pub(super) fn pane_move_holds_tab(&self, tab: &str) -> bool {
        self.pane_moves.values().any(|op| {
            (!matches!(op.intent.phase.as_str(), "failed" | "completed")
                || op.publication_barrier.is_some())
                && (op.intent.request.source.tab_id == tab
                    || op.intent.result_tab.as_ref().is_some_and(|id| {
                        self.pane_move_public(&op.intent.request.workspace.device_id, "tab", id)
                            == tab
                    })
                    || op
                        .intent
                        .request
                        .destination
                        .target()
                        .is_some_and(|(id, _, _)| id == tab))
        }) || self.workspace_views.as_ref().is_some_and(|store| {
            store.views.workspaces.iter().any(|view| {
                view.pane_moves.values().any(|intent| {
                    !matches!(intent.phase.as_str(), "failed" | "completed")
                        && (intent.request.source.tab_id == tab
                            || intent
                                .request
                                .destination
                                .target()
                                .is_some_and(|(id, _, _)| id == tab))
                })
            })
        })
    }
    pub(super) fn pane_move_holds_workspace(&self, key: &(String, String)) -> bool {
        self.pane_move_operations().iter().any(|operation| {
            !matches!(operation.phase.as_str(), "completed" | "failed")
                && self
                    .pane_moves
                    .get(&operation.id)
                    .map(|op| &op.intent)
                    .or_else(|| {
                        self.workspace_views
                            .as_ref()
                            .and_then(|store| store.views.get(&key.0, &key.1))
                            .and_then(|view| view.pane_moves.get(&operation.id))
                    })
                    .is_some_and(|intent| {
                        intent.request.workspace.device_id == key.0
                            && intent.request.workspace.checkout_path == key.1
                    })
        })
    }
    pub(super) fn pane_move_holds_pane(&self, pane: &str) -> bool {
        self.pane_operation_scope(pane)
            .is_some_and(|tab| self.pane_move_holds_tab(&tab))
    }
    fn pane_move_connector(&self, device: &str) -> Result<Arc<dyn ApiConnector>, String> {
        if device == self.node.as_str() {
            self.live
                .as_ref()
                .map(|live| Arc::clone(&live.api_connector))
        } else {
            self.remote_controls
                .get(device)
                .map(|remote| remote.api_connector())
        }
        .ok_or_else(|| "The terminal connection is unavailable".into())
    }
    fn pane_move_raw<'a>(
        &self,
        device: &str,
        kind: &str,
        value: &'a str,
    ) -> Result<&'a str, String> {
        if device == self.node.as_str() {
            return Ok(value);
        }
        value
            .strip_prefix(&format!("remote:{device}:{kind}:"))
            .ok_or_else(|| "The pane or tab belongs to another device".into())
    }
    fn pane_move_public(&self, device: &str, kind: &str, value: &str) -> String {
        if device == self.node.as_str() {
            value.into()
        } else {
            format!("remote:{device}:{kind}:{value}")
        }
    }
    fn validate_pane_move(&self, request: &Request) -> Result<(String, String, bool), String> {
        if request.request_id.is_empty() || request.request_id.len() > 128 {
            return Err("Invalid move request identity".into());
        }
        let key = (
            request.workspace.device_id.clone(),
            request.workspace.checkout_path.clone(),
        );
        if self.front_workspace_key().as_ref() != Some(&key) {
            return Err("The Workspace changed. Choose the pane again.".into());
        }
        let view = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .ok_or("Agent areas are unavailable")?;
        if view.agent_groups.selected != request.group_id {
            return Err("The selected group changed".into());
        }
        let layout = view
            .group_layout(request.group_id.as_deref())
            .ok_or("The group is unavailable")?;
        let source_area = layout
            .tree
            .area(&request.source.area_id)
            .ok_or("The source area is unavailable")?;
        if layout.shown_in(&source_area.id) != Some(request.source.tab_id.as_str()) {
            return Err("The source tab is no longer shown".into());
        }
        if layout.tree.area(request.destination.area()).is_none() {
            return Err("The destination area is unavailable".into());
        }
        if source_has_fork(self, &request.source.tab_id)
            || self.geometry_tab_busy(&request.source.tab_id)
        {
            return Err("This tab already has a pending change".into());
        }
        let checkout = self
            .catalog_workspaces()
            .filter(|w| w.device_id == key.0)
            .flat_map(|w| &w.checkouts)
            .find(|c| c.path == key.1)
            .ok_or("The checkout is unavailable")?;
        let source = checkout
            .tabs
            .iter()
            .find(|tab| tab.id.as_deref() == Some(&request.source.tab_id))
            .ok_or("The source tab closed")?;
        let pane = source
            .panes
            .iter()
            .find(|pane| pane.id == request.source.pane_id)
            .ok_or("The source pane closed")?;
        if pane.failed_terminal_id.is_some()
            || pane.sleep.is_some()
            || self.close_operation_holds_pane(&pane.id)
        {
            return Err("The source pane is unavailable or closing".into());
        }
        if view.agent_starts.values().any(|start| {
            start.is_pending()
                && (start.live_tab_id.as_ref() == Some(&request.source.tab_id)
                    || start.draft_tab_id.as_ref() == Some(&request.source.tab_id))
        }) {
            return Err("This tab is still starting".into());
        }
        let terminal = pane
            .terminal_id
            .clone()
            .ok_or("The source terminal identity is unavailable")?;
        if let Some((tab_id, pane_id, _)) = request.destination.target() {
            if pane_id == request.source.pane_id {
                return Err("A pane cannot be dropped on itself".into());
            }
            if tab_id != request.source.tab_id
                && (source_has_fork(self, tab_id) || self.geometry_tab_busy(tab_id))
            {
                return Err("The destination tab has a pending change".into());
            }
            if layout.tree.area_of(tab_id).map(|area| area.id.as_str())
                != Some(request.destination.area())
                && layout.shown_in(request.destination.area()) != Some(tab_id)
            {
                return Err("The target tab moved or changed groups".into());
            }
            let target = checkout
                .tabs
                .iter()
                .find(|tab| tab.id.as_deref() == Some(tab_id))
                .ok_or("The target tab closed")?;
            let target_pane = target
                .panes
                .iter()
                .find(|pane| pane.id == pane_id)
                .ok_or("The target pane closed")?;
            if target_pane.terminal_id.is_none()
                || target_pane.failed_terminal_id.is_some()
                || target_pane.sleep.is_some()
                || self.close_operation_holds_pane(pane_id)
            {
                return Err("The target pane is unavailable or closing".into());
            }
            if view.agent_starts.values().any(|start| {
                start.is_pending()
                    && (start.live_tab_id.as_deref() == Some(tab_id)
                        || start.draft_tab_id.as_deref() == Some(tab_id))
            }) {
                return Err("The target tab is still starting".into());
            }
        } else if source.panes.len() > 1
            && view.agent_tab_count() + self.pending_agent_admissions(&key) >= 64
        {
            return Err("At most 64 Agent tabs can be arranged. Close a tab first.".into());
        }
        let native_workspace = if key.0 == self.node.as_str() {
            self.snapshot
                .pane_layouts
                .iter()
                .find(|layout| {
                    layout.tab_id == request.source.tab_id
                        && layout.pane_ids().contains(&request.source.pane_id.as_str())
                })
                .map(|layout| layout.workspace_id.as_str())
        } else {
            self.device_raw_sessions.get(&key.0).and_then(|session| {
                session
                    .pane_layouts
                    .iter()
                    .find(|layout| {
                        layout.tab_id == request.source.tab_id
                            && layout
                                .frames
                                .iter()
                                .any(|frame| frame.pane_id == request.source.pane_id)
                    })
                    .map(|layout| layout.workspace_id.as_str())
            })
        }
        .ok_or("The source native layout is unavailable")?;
        Ok((
            terminal,
            self.pane_move_raw(&key.0, "workspace", native_workspace)?
                .to_owned(),
            source.panes.len() == 1,
        ))
    }
    pub(super) fn apply_pane_move(&mut self, request: Request) -> bool {
        if self.pane_moves.contains_key(&request.request_id)
            || self.workspace_views.as_ref().is_some_and(|store| {
                store
                    .views
                    .workspaces
                    .iter()
                    .any(|view| view.pane_moves.contains_key(&request.request_id))
            })
        {
            return false;
        }
        let checked = self.validate_pane_move(&request);
        let (terminal_id, workspace_id, sole) = match checked {
            Ok(value) => value,
            Err(message) => {
                self.reject_pane_move(request, message);
                return true;
            }
        };
        if sole && matches!(request.destination, Destination::NewTab { .. }) {
            let key = (
                request.workspace.device_id.clone(),
                request.workspace.checkout_path.clone(),
            );
            let view = self
                .workspace_views
                .as_mut()
                .expect("validated")
                .views
                .entry(&key.0, &key.1);
            let layout = view.selected_agent_layout_mut();
            if request.source.area_id != request.destination.area() {
                let end = layout
                    .tree
                    .area(request.destination.area())
                    .expect("validated")
                    .displays
                    .len();
                if let Err(error) = layout.tree.move_display(
                    &request.source.tab_id,
                    request.destination.area(),
                    end,
                    unix_milliseconds(),
                ) {
                    self.reject_pane_move(request, format!("The tab could not move: {error:?}"));
                    return true;
                }
            }
            self.persist_workspace_views();
            if request.source.area_id != request.destination.area() {
                self.show_agent_workspace_tab(&key, Some(request.source.tab_id.clone()));
                self.focus_pane(
                    request.source.pane_id.clone(),
                    PaneFocusOrigin::Operator,
                    None,
                );
            }
            self.record_pane_move(Intent {
                request,
                terminal_id,
                workspace_id,
                phase: "completed".into(),
                started_at: unix_milliseconds(),
                message: None,
                target_terminal: None,
                new_tab_slot: false,
                result_pane: None,
                result_tab: None,
            });
            self.persist_workspace_views();
            return true;
        }
        if self
            .pane_moves
            .values()
            .filter(|op| {
                op.publication_barrier.is_some()
                    || !matches!(op.intent.phase.as_str(), "failed" | "completed")
            })
            .count()
            >= ACTIVE_LIMIT
        {
            self.reject_pane_move(request, "Too many pane moves are pending".into());
            return true;
        }
        let key = (
            request.workspace.device_id.clone(),
            request.workspace.checkout_path.clone(),
        );
        if self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .is_some_and(|view| {
                view.pane_moves.len() >= RECEIPT_LIMIT
                    && !view.pane_moves.values().any(|previous| {
                        previous.terminal_id == terminal_id
                            && matches!(previous.phase.as_str(), "failed" | "completed")
                    })
            })
        {
            self.reject_pane_move(
                request,
                "Too many pane move receipts are retained. Check pending moves first.".into(),
            );
            return true;
        }
        let connector = match self.pane_move_connector(&key.0) {
            Ok(value) => value,
            Err(message) => {
                self.reject_pane_move(request, message);
                return true;
            }
        };
        let Some(context) = self.worker_context.clone() else {
            self.reject_pane_move(request, "Pane moves are unavailable in this shell".into());
            return true;
        };
        let target_terminal = request.destination.target().and_then(|(_, id, _)| {
            self.catalog_workspaces()
                .filter(|workspace| workspace.device_id == key.0)
                .flat_map(|w| &w.checkouts)
                .flat_map(|c| &c.tabs)
                .flat_map(|t| &t.panes)
                .find(|p| p.id == id)
                .and_then(|p| p.terminal_id.clone())
        });
        let new_tab_slot = matches!(request.destination, Destination::NewTab { .. });
        let intent = Intent {
            request,
            terminal_id,
            workspace_id,
            phase: "preparing".into(),
            started_at: unix_milliseconds(),
            message: None,
            target_terminal: None,
            new_tab_slot,
            result_pane: None,
            result_tab: None,
        };
        let mut intent = intent;
        intent.target_terminal = target_terminal;
        let (sender, saved) = sync_channel(1);
        self.pane_move_saves.push(SaveWaiter {
            key: key.clone(),
            id: intent.request.request_id.clone(),
            intent: intent.clone(),
            sender,
        });
        self.record_pane_move(intent.clone());
        self.persist_workspace_views();
        let id = intent.request.request_id.clone();
        if let Err(error) = thread::Builder::new()
            .name("hide-pane-move".into())
            .spawn(move || run(context, connector, intent, Some(saved), false))
        {
            self.finish_pane_move(
                &id,
                "failed",
                Some(format!("Pane move worker could not start: {error}")),
                None,
            );
        }
        true
    }
    fn reject_pane_move(&mut self, request: Request, message: String) {
        self.record_pane_move(Intent {
            request,
            terminal_id: String::new(),
            workspace_id: String::new(),
            phase: "failed".into(),
            started_at: unix_milliseconds(),
            message: Some(message),
            target_terminal: None,
            new_tab_slot: false,
            result_pane: None,
            result_tab: None,
        });
    }
    fn record_pane_move(&mut self, intent: Intent) {
        let request = &intent.request;
        // Terminal identity deduplicates successive explicit placements of the
        // same running pane, keeping one durable manual-placement receipt.
        if !intent.terminal_id.is_empty()
            && let Some(store) = self.workspace_views.as_mut()
        {
            let view = store.views.entry(
                &request.workspace.device_id,
                &request.workspace.checkout_path,
            );
            view.pane_moves.retain(|id, previous| {
                id == &request.request_id
                    || previous.terminal_id != intent.terminal_id
                    || !matches!(previous.phase.as_str(), "failed" | "completed")
            });
            view.pane_moves
                .insert(request.request_id.clone(), intent.clone());
        }
        self.pane_moves.retain(|_, op| {
            op.publication_barrier.is_some()
                || !matches!(op.intent.phase.as_str(), "failed" | "completed")
                || op.intent.started_at.saturating_add(60_000) > unix_milliseconds()
        });
        let remote_baseline = self
            .device_raw_sessions
            .get(&request.workspace.device_id)
            .cloned();
        while self.pane_moves.len() >= RECEIPT_LIMIT {
            let oldest = self
                .pane_moves
                .iter()
                .filter(|(_, op)| {
                    op.publication_barrier.is_none()
                        && matches!(op.intent.phase.as_str(), "failed" | "completed")
                })
                .min_by_key(|(_, op)| op.intent.started_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.pane_moves.remove(&id);
            } else {
                break;
            }
        }
        let worker = !matches!(intent.phase.as_str(), "failed" | "completed");
        self.pane_moves.insert(
            request.request_id.clone(),
            Operation {
                intent,
                baseline: None,
                remote_baseline,
                worker,
                publication_barrier: None,
            },
        );
        self.sync_async_operations();
    }
    pub(super) fn acknowledge_pane_move_save(
        &mut self,
        saved: &workspace_views::WorkspaceViews,
        result: &Result<(), String>,
    ) {
        self.pane_move_saves.retain(|waiter| {
            let matches = saved
                .get(&waiter.key.0, &waiter.key.1)
                .and_then(|view| view.pane_moves.get(&waiter.id))
                == Some(&waiter.intent);
            if matches {
                let _ = waiter.sender.try_send(result.clone());
            }
            !matches
        });
    }
    pub(super) fn check_pane_move(&mut self, check: Check) -> bool {
        let intent = self
            .pane_moves
            .get(&check.request_id)
            .map(|op| op.intent.clone())
            .or_else(|| {
                self.workspace_views.as_ref().and_then(|store| {
                    store
                        .views
                        .workspaces
                        .iter()
                        .find_map(|view| view.pane_moves.get(&check.request_id).cloned())
                })
            });
        let Some(mut intent) = intent else {
            return false;
        };
        if matches!(intent.phase.as_str(), "failed" | "completed")
            || self
                .pane_moves
                .get(&check.request_id)
                .is_some_and(|op| op.worker || op.publication_barrier.is_some())
        {
            return false;
        }
        self.pane_moves
            .entry(check.request_id.clone())
            .or_insert_with(|| Operation {
                intent: intent.clone(),
                baseline: None,
                remote_baseline: None,
                worker: false,
                publication_barrier: None,
            });
        if self
            .pane_moves
            .values()
            .filter(|operation| operation.worker || operation.publication_barrier.is_some())
            .count()
            >= ACTIVE_LIMIT
        {
            self.finish_pane_move(
                &check.request_id,
                "unknown",
                Some("Too many pane moves are being checked".into()),
                None,
            );
            return true;
        }
        let connector = match self.pane_move_connector(&intent.request.workspace.device_id) {
            Ok(value) => value,
            Err(message) => {
                self.finish_pane_move(&check.request_id, "unknown", Some(message), None);
                return true;
            }
        };
        let Some(context) = self.worker_context.clone() else {
            return false;
        };
        intent.phase = "unknown".into();
        self.pane_moves.insert(
            check.request_id.clone(),
            Operation {
                intent: intent.clone(),
                baseline: None,
                remote_baseline: None,
                worker: true,
                publication_barrier: None,
            },
        );
        if let Err(error) = thread::Builder::new()
            .name("hide-pane-move-check".into())
            .spawn(move || run(context, connector, intent, None, true))
        {
            self.finish_pane_move(&check.request_id, "unknown", Some(error.to_string()), None);
        }
        true
    }
    pub(super) fn pane_move_operations(&self) -> Vec<crate::model::AsyncOperationSnapshot> {
        let mut intents = BTreeMap::new();
        if let Some(store) = &self.workspace_views {
            for view in &store.views.workspaces {
                for (id, intent) in &view.pane_moves {
                    intents.insert(id.clone(), intent.clone());
                }
            }
        }
        for (id, op) in &self.pane_moves {
            intents.insert(id.clone(), op.intent.clone());
        }
        intents
            .values()
            .filter(|intent| {
                !matches!(intent.phase.as_str(), "completed")
                    || self
                        .pane_moves
                        .get(&intent.request.request_id)
                        .is_some_and(|op| op.publication_barrier.is_some())
                    || intent.started_at.saturating_add(60_000) > unix_milliseconds()
            })
            .map(|intent| crate::model::AsyncOperationSnapshot {
                related_scope_ids: {
                    let mut tabs = intent
                        .request
                        .destination
                        .target()
                        .map(|(id, _, _)| vec![intent.request.source.tab_id.clone(), id.into()])
                        .unwrap_or_else(|| vec![intent.request.source.tab_id.clone()]);
                    if let Some(result) = &intent.result_tab {
                        let public = self.pane_move_public(
                            &intent.request.workspace.device_id,
                            "tab",
                            result,
                        );
                        if !tabs.contains(&public) {
                            tabs.push(public);
                        }
                    }
                    tabs
                },
                id: intent.request.request_id.clone(),
                kind: "pane.move".into(),
                target_id: intent.request.source.pane_id.clone(),
                scope_id: intent.request.source.tab_id.clone(),
                phase: if self
                    .pane_moves
                    .get(&intent.request.request_id)
                    .is_some_and(|op| op.publication_barrier.is_some())
                {
                    "awaiting_topology".into()
                } else if self.pane_moves.contains_key(&intent.request.request_id) {
                    intent.phase.clone()
                } else if matches!(
                    intent.phase.as_str(),
                    "preparing" | "transmitting" | "awaiting_topology"
                ) {
                    "unknown".into()
                } else {
                    intent.phase.clone()
                },
                stage: "relocate".into(),
                started_at_unix_ms: intent.started_at,
                deadline_at_unix_ms: None,
                message: intent.message.clone(),
                retryable: intent.phase == "failed"
                    && !self
                        .pane_moves
                        .get(&intent.request.request_id)
                        .is_some_and(|op| op.publication_barrier.is_some()),
            })
            .collect()
    }
    fn finish_pane_move(
        &mut self,
        id: &str,
        phase: &str,
        message: Option<String>,
        result: Option<(String, String)>,
    ) {
        self.pane_move_saves.retain(|waiter| waiter.id != id);
        if let Some(op) = self.pane_moves.get_mut(id) {
            op.intent.phase = phase.into();
            op.intent.message = message;
            op.baseline = None;
            op.remote_baseline = None;
            op.worker = false;
            if let Some((pane, tab)) = result {
                op.intent.result_pane = Some(pane);
                op.intent.result_tab = Some(tab);
            }
            let intent = op.intent.clone();
            if let Some(store) = self.workspace_views.as_mut() {
                let view = store.views.entry(
                    &intent.request.workspace.device_id,
                    &intent.request.workspace.checkout_path,
                );
                view.pane_moves.insert(id.into(), intent.clone());
            }
            if matches!(phase, "completed" | "unknown") && intent.result_pane.is_some() {
                self.place_moved_pane(&intent);
            }
            self.persist_workspace_views();
        }
        self.sync_async_operations();
    }
    fn place_moved_pane(&mut self, intent: &Intent) {
        let (Some(raw_pane), Some(raw_tab)) = (&intent.result_pane, &intent.result_tab) else {
            return;
        };
        let key = (
            intent.request.workspace.device_id.clone(),
            intent.request.workspace.checkout_path.clone(),
        );
        let pane = self.pane_move_public(&key.0, "pane", raw_pane);
        let tab = self.pane_move_public(&key.0, "tab", raw_tab);
        if let Some(store) = self.workspace_views.as_mut() {
            let view = store.views.entry(&key.0, &key.1);
            // Intermediate topology never chose a group or area. Apply the
            // final destination once, after the actual tab identity is known.
            let _ = view.move_agent_to_group(
                &tab,
                intent.request.group_id.as_deref(),
                unix_milliseconds(),
            );
            if let Some(layout) = view.group_layout_mut(intent.request.group_id.as_deref()) {
                if layout.tree.display(&tab).is_none() {
                    let _ = layout.tree.insert(
                        intent.request.destination.area(),
                        crate::agent_layout::Tab::new(tab.clone()),
                        unix_milliseconds(),
                    );
                } else if matches!(intent.request.destination, Destination::NewTab { .. }) {
                    let end = layout
                        .tree
                        .area(intent.request.destination.area())
                        .map_or(0, |area| area.displays.len());
                    let _ = layout.tree.move_display(
                        &tab,
                        intent.request.destination.area(),
                        end,
                        unix_milliseconds(),
                    );
                }
                layout.canvases.remove(intent.request.destination.area());
            }
        }
        self.show_agent_workspace_tab(&key, Some(tab));
        self.focus_pane(pane, PaneFocusOrigin::Operator, None);
    }
    /// Preserve the affected native tabs while a live transaction uses a
    /// temporary tab. Unrelated tabs and agent activity keep publishing.
    pub(super) fn overlay_pane_move_catalog(&self, catalog: &mut session_sync::PrecomputedCatalog) {
        for op in self.pane_moves.values().filter(|op| {
            (op.worker || op.publication_barrier.is_some())
                && op.baseline.is_some()
                && op.intent.request.workspace.device_id == self.node.as_str()
        }) {
            let affected = |tab: &str| {
                tab == op.intent.request.source.tab_id
                    || op
                        .intent
                        .request
                        .destination
                        .target()
                        .is_some_and(|(id, _, _)| id == tab)
                    || (op.publication_barrier.is_some()
                        && op.intent.result_tab.as_deref() == Some(tab))
            };
            for checkout in catalog
                .workspaces
                .iter_mut()
                .flat_map(|workspace| &mut workspace.checkouts)
            {
                if checkout.path != op.intent.request.workspace.checkout_path {
                    continue;
                }
                checkout.tabs.retain(|tab| {
                    !tab.id.as_deref().is_some_and(affected)
                        && !tab
                            .panes
                            .iter()
                            .any(|pane| pane.terminal_id.as_deref() == Some(&op.intent.terminal_id))
                });
                checkout.tabs.extend(
                    self.snapshot
                        .navigator
                        .workspaces
                        .iter()
                        .flat_map(|workspace| &workspace.checkouts)
                        .filter(|original| original.path == checkout.path)
                        .flat_map(|original| &original.tabs)
                        .filter(|tab| tab.id.as_deref().is_some_and(affected))
                        .cloned(),
                );
            }
        }
    }
    pub(super) fn overlay_pane_moves(&self, payload: &mut SessionSnapshotPayload) {
        for op in self
            .pane_moves
            .values()
            .filter(|op| op.worker || op.publication_barrier.is_some())
        {
            let Some(baseline) = &op.baseline else {
                continue;
            };
            if op.intent.request.workspace.device_id != self.node.as_str() {
                continue;
            }
            let source = &op.intent.request.source.tab_id;
            let target = op
                .intent
                .request
                .destination
                .target()
                .map(|(tab, _, _)| tab);
            let affected = |tab: &str| {
                tab == source
                    || target == Some(tab)
                    || (op.publication_barrier.is_some()
                        && op.intent.result_tab.as_deref() == Some(tab))
            };
            let moved = payload
                .panes
                .iter()
                .find(|pane| pane.terminal_id.as_deref() == Some(&op.intent.terminal_id))
                .map(|pane| pane.pane_id.clone());
            let temporary = payload
                .layouts
                .iter()
                .filter(|layout| {
                    !affected(&layout.tab_id)
                        && moved
                            .as_ref()
                            .is_some_and(|pane| layout.panes.iter().any(|row| &row.pane_id == pane))
                })
                .map(|layout| layout.tab_id.clone())
                .collect::<HashSet<_>>();
            let replace = |tab: &str| affected(tab) || temporary.contains(tab);
            payload.tabs.retain(|tab| !replace(&tab.tab_id));
            payload.tabs.extend(
                baseline
                    .tabs
                    .iter()
                    .filter(|tab| affected(&tab.tab_id))
                    .cloned(),
            );
            let ids = payload
                .layouts
                .iter()
                .filter(|layout| replace(&layout.tab_id))
                .flat_map(|layout| layout.panes.iter().map(|pane| pane.pane_id.clone()))
                .collect::<HashSet<_>>();
            let original = baseline
                .layouts
                .iter()
                .filter(|layout| affected(&layout.tab_id))
                .flat_map(|layout| layout.panes.iter().map(|pane| pane.pane_id.clone()))
                .collect::<HashSet<_>>();
            payload.layouts.retain(|layout| !replace(&layout.tab_id));
            payload.layouts.extend(
                baseline
                    .layouts
                    .iter()
                    .filter(|layout| affected(&layout.tab_id))
                    .cloned(),
            );
            payload.agents.retain(|agent| {
                agent
                    .pane_id
                    .as_ref()
                    .is_none_or(|id| !ids.contains(id) && !original.contains(id))
            });
            payload.agents.extend(
                baseline
                    .agents
                    .iter()
                    .filter(|agent| {
                        agent
                            .pane_id
                            .as_ref()
                            .is_some_and(|id| original.contains(id))
                    })
                    .cloned(),
            );
            payload
                .panes
                .retain(|pane| !ids.contains(&pane.pane_id) && !original.contains(&pane.pane_id));
            payload.panes.extend(
                baseline
                    .panes
                    .iter()
                    .filter(|pane| original.contains(&pane.pane_id))
                    .cloned(),
            );
            for old in &baseline.workspaces {
                if baseline
                    .tabs
                    .iter()
                    .any(|tab| affected(&tab.tab_id) && tab.workspace_id == old.workspace_id)
                {
                    if let Some(current) = payload
                        .workspaces
                        .iter_mut()
                        .find(|workspace| workspace.workspace_id == old.workspace_id)
                    {
                        current.active_tab_id = old.active_tab_id.clone();
                    } else {
                        payload.workspaces.push(old.clone());
                    }
                }
            }
            if payload
                .focused_pane_id
                .as_ref()
                .is_some_and(|id| ids.contains(id))
            {
                payload.focused_pane_id = baseline.focused_pane_id.clone();
                payload.focused_workspace_id = baseline.focused_workspace_id.clone();
                payload.tab_focus = None;
            }
        }
    }
    pub(super) fn overlay_remote_pane_moves(
        &self,
        device: &str,
        payload: &mut RemoteSessionSnapshot,
    ) {
        for op in self.pane_moves.values().filter(|op| {
            (op.worker || op.publication_barrier.is_some())
                && op.intent.request.workspace.device_id == device
        }) {
            let Some(baseline) = &op.remote_baseline else {
                continue;
            };
            let source = &op.intent.request.source.tab_id;
            let target = op
                .intent
                .request
                .destination
                .target()
                .map(|(tab, _, _)| tab);
            let result = op
                .intent
                .result_tab
                .as_ref()
                .map(|id| self.pane_move_public(device, "tab", id));
            let affected = |id: &str| {
                id == source
                    || target == Some(id)
                    || (op.publication_barrier.is_some() && result.as_deref() == Some(id))
            };
            let mut temporary = HashSet::new();
            for tab in payload
                .workspaces
                .iter()
                .flat_map(|w| &w.checkouts)
                .flat_map(|c| &c.tabs)
            {
                if tab.id.as_deref().is_some_and(|id| !affected(id))
                    && tab
                        .panes
                        .iter()
                        .any(|pane| pane.terminal_id.as_deref() == Some(&op.intent.terminal_id))
                    && let Some(id) = &tab.id
                {
                    temporary.insert(id.clone());
                }
            }
            let replaced = payload
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .filter(|tab| {
                    tab.id
                        .as_deref()
                        .is_some_and(|id| affected(id) || temporary.contains(id))
                })
                .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
                .collect::<HashSet<_>>();
            for workspace in &mut payload.workspaces {
                for checkout in &mut workspace.checkouts {
                    checkout.tabs.retain(|tab| {
                        tab.id
                            .as_deref()
                            .is_none_or(|id| !affected(id) && !temporary.contains(id))
                    });
                }
            }
            for old in &baseline.workspaces {
                for checkout in &old.checkouts {
                    let restored = checkout
                        .tabs
                        .iter()
                        .filter(|tab| tab.id.as_deref().is_some_and(affected))
                        .cloned()
                        .collect::<Vec<_>>();
                    if restored.is_empty() {
                        continue;
                    }
                    if let Some(current) = payload
                        .workspaces
                        .iter_mut()
                        .find(|w| w.id == old.id)
                        .and_then(|w| w.checkouts.iter_mut().find(|c| c.id == checkout.id))
                    {
                        current.tabs.extend(restored);
                        current.active_tab_id = checkout.active_tab_id.clone();
                    } else {
                        let mut old = old.clone();
                        old.checkouts.retain(|c| c.id == checkout.id);
                        old.checkouts[0].tabs = restored;
                        payload.workspaces.push(old);
                    }
                }
            }
            let original = baseline
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .filter(|tab| tab.id.as_deref().is_some_and(affected))
                .flat_map(|tab| tab.panes.iter().map(|pane| pane.id.clone()))
                .collect::<HashSet<_>>();
            payload.agents.retain(|agent| {
                !original.contains(&agent.pane_id) && !replaced.contains(&agent.pane_id)
            });
            payload.agents.extend(
                baseline
                    .agents
                    .iter()
                    .filter(|agent| original.contains(&agent.pane_id))
                    .cloned(),
            );
            payload
                .pane_layouts
                .retain(|layout| !affected(&layout.tab_id) && !temporary.contains(&layout.tab_id));
            payload.pane_layouts.extend(
                baseline
                    .pane_layouts
                    .iter()
                    .filter(|layout| affected(&layout.tab_id))
                    .cloned(),
            );
        }
    }
    pub(super) fn manually_placed_pane(&self, pane_id: &str) -> bool {
        let terminal = self
            .catalog_workspaces()
            .flat_map(|w| &w.checkouts)
            .flat_map(|c| &c.tabs)
            .flat_map(|tab| &tab.panes)
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.terminal_id.as_deref());
        terminal.is_some_and(|terminal| {
            self.workspace_views.as_ref().is_some_and(|store| {
                store.views.workspaces.iter().any(|view| {
                    view.pane_moves.values().any(|intent| {
                        intent.terminal_id == terminal
                            && (intent.phase != "failed" || intent.result_pane.is_some())
                    })
                })
            })
        })
    }
}

fn source_has_fork(runtime: &Runtime, tab_id: &str) -> bool {
    runtime
        .forks_in_flight
        .keys()
        .any(|pane| runtime.pane_operation_scope(pane).as_deref() == Some(tab_id))
}

fn locate<'a>(
    payload: &'a SessionSnapshotPayload,
    terminal: &str,
) -> Result<(&'a str, &'a str, &'a str), String> {
    let panes = payload
        .panes
        .iter()
        .filter(|pane| pane.terminal_id.as_deref() == Some(terminal))
        .collect::<Vec<_>>();
    if panes.len() != 1 {
        return Err(
            "The original terminal cannot be uniquely located; no move was repeated".into(),
        );
    }
    let pane = &panes[0].pane_id;
    let layout = payload
        .layouts
        .iter()
        .find(|layout| layout.panes.iter().any(|row| &row.pane_id == pane))
        .ok_or("The original terminal has no confirmed layout")?;
    Ok((pane, &layout.tab_id, &layout.workspace_id))
}
fn ask(
    connector: &dyn ApiConnector,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    request_with_connector(connector, method, params, CALL_TIMEOUT)
        .map_err(|error| error.to_string())
}
fn execute(connector: &dyn ApiConnector, intent: &Intent, local: bool) -> Result<(), String> {
    let request = &intent.request;
    let device = &request.workspace.device_id;
    let raw = |kind: &str, id: &str| -> Result<String, String> {
        if local {
            Ok(id.into())
        } else {
            id.strip_prefix(&format!("remote:{device}:{kind}:"))
                .map(str::to_owned)
                .ok_or_else(|| "The target device changed".into())
        }
    };
    let source = raw("pane", &request.source.pane_id)?;
    let source_tab = raw("tab", &request.source.tab_id)?;
    let mut current = source.clone();
    // Explicit off is harmless for a nonzoomed tab and never runs during drag.
    ask(connector, "pane.zoom", wire::pane_unzoom_params(&source)?)?;
    if let Some((tab, pane, edge)) = request.destination.target() {
        let tab = raw("tab", tab)?;
        let pane = raw("pane", pane)?;
        if tab != source_tab {
            ask(connector, "pane.zoom", wire::pane_unzoom_params(&pane)?)?;
        }
        if tab == source_tab {
            let result = wire::joined_pane(ask(
                connector,
                "pane.move",
                wire::pane_move_to_new_tab_params(&current, &intent.workspace_id, "")?,
            )?)?;
            if !result.changed {
                return Err(format!("Herdr refused the pane move: {:?}", result.reason));
            }
            if result.terminal_id != intent.terminal_id {
                return Err("Herdr returned a different terminal identity".into());
            }
            current = result.pane_id;
        }
        let result = wire::joined_pane(ask(
            connector,
            "pane.move",
            wire::pane_insert_params(&current, &tab, &pane, matches!(edge, Edge::Up | Edge::Down))?,
        )?)?;
        if !result.changed {
            return Err(format!("Herdr refused the pane move: {:?}", result.reason));
        }
        if result.terminal_id != intent.terminal_id {
            return Err("Herdr returned a different terminal identity".into());
        }
        if matches!(edge, Edge::Left | Edge::Up) {
            ask(
                connector,
                "pane.swap",
                wire::pane_swap_params(&result.pane_id, &pane)?,
            )?;
        }
    } else {
        let result = wire::joined_pane(ask(
            connector,
            "pane.move",
            wire::pane_move_to_new_tab_params(&current, &intent.workspace_id, "")?,
        )?)?;
        if !result.changed {
            return Err(format!("Herdr refused the pane move: {:?}", result.reason));
        }
        if result.terminal_id != intent.terminal_id {
            return Err("Herdr returned a different terminal identity".into());
        }
    }
    Ok(())
}
fn run(
    context: RuntimeWorkerContext,
    connector: Arc<dyn ApiConnector>,
    intent: Intent,
    saved: Option<Receiver<Result<(), String>>>,
    check_only: bool,
) {
    let id = &intent.request.request_id;
    if let Some(saved) = saved {
        match saved.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => {}
            result => {
                if let Some(runtime) = context.runtime.upgrade() {
                    runtime
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .finish_pane_move(
                            id,
                            "failed",
                            Some(format!("The move intent could not be saved: {result:?}")),
                            None,
                        );
                    context.notifier.notify();
                }
                return;
            }
        }
    }
    let baseline = live::fetch_session_with_connector(connector.as_ref());
    let mut effect = Ok(());
    let mut executed = false;
    if !check_only {
        effect = baseline
            .as_ref()
            .map_err(|error| error.message().to_owned())
            .and_then(|payload| {
                let (pane, tab, workspace) = locate(payload, &intent.terminal_id)?;
                let Some(runtime) = context.runtime.upgrade() else {
                    return Err("The move owner ended".into());
                };
                let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
                if guard.pane_move_raw(
                    &intent.request.workspace.device_id,
                    "pane",
                    &intent.request.source.pane_id,
                )? != pane
                    || guard.pane_move_raw(
                        &intent.request.workspace.device_id,
                        "tab",
                        &intent.request.source.tab_id,
                    )? != tab
                    || workspace != intent.workspace_id
                {
                    return Err("The source topology changed before the move".into());
                }
                if let Some((target_tab, target_pane, edge)) = intent.request.destination.target() {
                    let target_tab = guard.pane_move_raw(
                        &intent.request.workspace.device_id,
                        "tab",
                        target_tab,
                    )?;
                    let target_pane = guard.pane_move_raw(
                        &intent.request.workspace.device_id,
                        "pane",
                        target_pane,
                    )?;
                    let target_layout = payload
                        .layouts
                        .iter()
                        .find(|layout| layout.tab_id == target_tab);
                    // A same-tab detach collapses the source's sibling first.
                    // Reuse the existing close geometry predictor so a narrow
                    // target that expands after removal is not falsely refused.
                    let after_detach = target_layout
                        .filter(|layout| layout.tab_id == tab)
                        .and_then(|layout| {
                            super::pane_prediction::overlay_session_layout(
                                layout,
                                &[super::pane_prediction::Prediction::Close { pane: pane.into() }],
                            )
                        });
                    let target_rectangle = after_detach
                        .as_ref()
                        .or(target_layout)
                        .and_then(|layout| {
                            layout.panes.iter().find(|pane| pane.pane_id == target_pane)
                        })
                        .map(|pane| pane.rect);
                    if target_rectangle.is_some_and(|rect| match edge {
                        Edge::Left | Edge::Right => rect.width < 2 || rect.height < 1,
                        Edge::Up | Edge::Down => rect.height < 2 || rect.width < 1,
                    }) {
                        return Err(
                            "The target pane is too small to insert another visible pane".into(),
                        );
                    }
                    if !payload.layouts.iter().any(|layout| {
                        layout.tab_id == target_tab
                            && layout.panes.iter().any(|pane| pane.pane_id == target_pane)
                    }) || !payload.panes.iter().any(|pane| {
                        pane.pane_id == target_pane && pane.terminal_id == intent.target_terminal
                    }) {
                        return Err("The target topology changed before the move".into());
                    }
                }
                let local = intent.request.workspace.device_id == guard.node.as_str();
                if let Some(op) = guard.pane_moves.get_mut(id) {
                    op.baseline = Some(payload.clone());
                    op.intent.phase = "transmitting".into();
                }
                guard.sync_async_operations();
                drop(guard);
                context.notifier.notify();
                executed = true;
                execute(connector.as_ref(), &intent, local)
            });
    }
    // Even a malformed/lost answer is followed by an authoritative read.
    // Recovery never executes the mutation above a second time.
    let read_final = || {
        ask(connector.as_ref(), "session.snapshot", wire::empty_params()).and_then(|value| {
            session_sync::project_remote_read(value, &intent.request.workspace.device_id)
                .map_err(|error| error.message().to_owned())
        })
    };
    let mut final_payload = read_final();
    let mut restore_failed = false;
    if executed
        && effect.is_err()
        && baseline.as_ref().is_ok_and(|before| {
            final_payload
                .as_ref()
                .is_ok_and(|(after, _)| same_placement(before, after, &intent))
        })
    {
        let before = baseline.as_ref().expect("checked");
        if let Err(error) = restore_refused_presentation(connector.as_ref(), before, &intent) {
            restore_failed = true;
            effect = Err(format!(
                "The move was refused; its previous zoom or focus could not be restored: {error}"
            ));
        }
        final_payload = read_final();
        if final_payload
            .as_ref()
            .is_ok_and(|(after, _)| !same_zoom(before, after, &intent))
        {
            restore_failed = true;
        }
    }
    if let Some(runtime) = context.runtime.upgrade() {
        let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(op) = guard.pane_moves.get_mut(id) {
            op.baseline = None;
            op.remote_baseline = None;
            op.worker = false;
        }
        match final_payload {
            Ok((payload, remote)) => {
                let publication =
                    (executed || check_only).then(|| (payload.clone(), remote.clone()));
                let found = locate(&payload, &intent.terminal_id)
                    .map(|(pane, tab, _)| (pane.to_owned(), tab.to_owned()));
                let local = intent.request.workspace.device_id == guard.node.as_str();
                let confirmed = confirms(&payload, &intent, local);
                let payload_for_comparison = payload.clone();
                if local {
                    guard.ingest_session(Ok(payload));
                } else {
                    guard.ingest_remote_session(&intent.request.workspace.device_id, Ok(remote));
                }
                match found {
                    Ok((pane, tab)) => {
                        let unchanged = baseline.as_ref().is_ok_and(|before| {
                            same_placement(before, &payload_for_comparison, &intent)
                        });
                        let complete = confirmed;
                        let phase = if complete {
                            "completed"
                        } else if check_only || (effect.is_err() && unchanged && !restore_failed) {
                            "failed"
                        } else {
                            "unknown"
                        };
                        let message = if complete {
                            None
                        } else {
                            effect.err().or_else(||Some(if check_only {"The pane is live at its confirmed location. The requested placement was not completed; you can move it again."}else{"The pane is live at its confirmed location. Check this move before trying again."}.into()))
                        };
                        guard.finish_pane_move(id, phase, message, Some((pane, tab)));
                    }
                    Err(message) => {
                        let gone = check_only
                            && !payload_for_comparison.panes.iter().any(|pane| {
                                pane.terminal_id.as_deref() == Some(&intent.terminal_id)
                            });
                        guard.finish_pane_move(id, if gone{"failed"}else{"unknown"}, Some(if gone{"The original terminal is no longer live; no replacement was started".into()}else{message}), None)
                    }
                }
                if let Some((payload, remote)) = publication {
                    if let Some(op) = guard.pane_moves.get_mut(id) {
                        op.baseline = Some(payload);
                        op.remote_baseline = Some(remote);
                        op.publication_barrier = Some(std::time::Instant::now());
                    }
                    let refresh_started = if local {
                        if let Some(waker) = &guard.republish_waker {
                            waker.refresh_topology()
                        } else {
                            false
                        }
                    } else {
                        guard.refresh_remote_topology(&intent.request.workspace.device_id)
                    };
                    if !refresh_started && let Some(op) = guard.pane_moves.get_mut(id) {
                        op.publication_barrier = None;
                        op.baseline = None;
                        op.remote_baseline = None;
                    }
                    guard.sync_async_operations();
                }
            }
            Err(error) => guard.finish_pane_move(
                id,
                "unknown",
                Some(format!("The move result could not be checked: {}", error)),
                None,
            ),
        }
        if let Some(waker) = &guard.republish_waker {
            waker.wake();
        }
        drop(guard);
        context.notifier.notify();
    }
}

fn affected_before<'a>(
    before: &'a SessionSnapshotPayload,
    intent: &Intent,
) -> impl Iterator<Item = &'a crate::sidebar::SessionLayoutPayload> {
    let terminals = [
        Some(intent.terminal_id.as_str()),
        intent.target_terminal.as_deref(),
    ];
    let panes = before
        .panes
        .iter()
        .filter(|pane| terminals.contains(&pane.terminal_id.as_deref()))
        .map(|pane| pane.pane_id.as_str())
        .collect::<HashSet<_>>();
    before.layouts.iter().filter(move |layout| {
        layout
            .panes
            .iter()
            .any(|pane| panes.contains(pane.pane_id.as_str()))
    })
}
fn restore_refused_presentation(
    connector: &dyn ApiConnector,
    before: &SessionSnapshotPayload,
    intent: &Intent,
) -> Result<(), String> {
    for layout in affected_before(before, intent).filter(|layout| layout.zoomed) {
        ask(
            connector,
            "pane.zoom",
            wire::pane_restore_zoom_params(&layout.focused_pane_id)?,
        )?;
    }
    if let Some(pane) = before.focused_pane_id.as_deref() {
        ask(connector, "pane.focus", wire::pane_target_params(pane)?)?;
    }
    Ok(())
}
fn same_zoom(
    before: &SessionSnapshotPayload,
    after: &SessionSnapshotPayload,
    intent: &Intent,
) -> bool {
    affected_before(before, intent).all(|layout| {
        after
            .layouts
            .iter()
            .find(|new| new.tab_id == layout.tab_id)
            .is_some_and(|new| new.zoomed == layout.zoomed)
    })
}

fn confirms(payload: &SessionSnapshotPayload, intent: &Intent, local: bool) -> bool {
    let Ok((pane, tab, _)) = locate(payload, &intent.terminal_id) else {
        return false;
    };
    let raw = |kind: &str, id: &str| {
        if local {
            Some(id.to_owned())
        } else {
            id.strip_prefix(&format!(
                "remote:{}:{kind}:",
                intent.request.workspace.device_id
            ))
            .map(str::to_owned)
        }
    };
    let Some(layout) = payload.layouts.iter().find(|layout| layout.tab_id == tab) else {
        return false;
    };
    if let Some((target_tab, target, edge)) = intent.request.destination.target() {
        if raw("tab", target_tab).as_deref() != Some(tab) {
            return false;
        }
        let target = raw("pane", target);
        if !payload.panes.iter().any(|pane| {
            Some(pane.pane_id.as_str()) == target.as_deref()
                && pane.terminal_id == intent.target_terminal
        }) {
            return false;
        }
        let Some(first) = layout
            .panes
            .iter()
            .find(|row| row.pane_id == pane)
            .map(|row| row.rect)
        else {
            return false;
        };
        let Some(second) = layout
            .panes
            .iter()
            .find(|row| Some(row.pane_id.as_str()) == target.as_deref())
            .map(|row| row.rect)
        else {
            return false;
        };
        match edge {
            Edge::Left => {
                first.x.saturating_add(first.width) == second.x
                    && first.y == second.y
                    && first.height == second.height
            }
            Edge::Right => {
                second.x.saturating_add(second.width) == first.x
                    && first.y == second.y
                    && first.height == second.height
            }
            Edge::Up => {
                first.y.saturating_add(first.height) == second.y
                    && first.x == second.x
                    && first.width == second.width
            }
            Edge::Down => {
                second.y.saturating_add(second.height) == first.y
                    && first.x == second.x
                    && first.width == second.width
            }
        }
    } else {
        layout.panes.len() == 1 && raw("tab", &intent.request.source.tab_id).as_deref() != Some(tab)
    }
}

/// A refused operation is retryable only when the authoritative affected
/// geometry still matches the state before any native side effect.
fn same_placement(
    before: &SessionSnapshotPayload,
    after: &SessionSnapshotPayload,
    intent: &Intent,
) -> bool {
    let Ok((before_pane, before_tab, _)) = locate(before, &intent.terminal_id) else {
        return false;
    };
    let Ok((after_pane, after_tab, _)) = locate(after, &intent.terminal_id) else {
        return false;
    };
    if before_pane != after_pane || before_tab != after_tab {
        return false;
    }
    let tabs = before.layouts.iter().filter(|layout| {
        layout.tab_id == before_tab
            || intent.target_terminal.as_ref().is_some_and(|terminal| {
                before.panes.iter().any(|pane| {
                    pane.terminal_id.as_ref() == Some(terminal)
                        && layout.panes.iter().any(|row| row.pane_id == pane.pane_id)
                })
            })
    });
    tabs.into_iter().all(|old| {
        after
            .layouts
            .iter()
            .find(|layout| layout.tab_id == old.tab_id)
            .is_some_and(|new| old.panes == new.panes && old.splits == new.splits)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use serde_json::json;

    fn intent(edge: Edge, target_tab: &str) -> Intent {
        Intent {
            request: Request {
                request_id: "move-fixture".into(),
                workspace: Workspace {
                    device_id: crate::node::TEST_NODE.into(),
                    checkout_path: "/fixture".into(),
                },
                group_id: None,
                source: Source {
                    pane_id: "w1:p1".into(),
                    tab_id: "w1:t1".into(),
                    area_id: "root".into(),
                },
                destination: Destination::Pane {
                    tab_id: target_tab.into(),
                    pane_id: "w1:p2".into(),
                    area_id: "root".into(),
                    edge,
                },
            },
            terminal_id: "term-original".into(),
            workspace_id: "w1".into(),
            phase: "preparing".into(),
            started_at: 1,
            message: None,
            target_terminal: Some("term-target".into()),
            new_tab_slot: false,
            result_pane: None,
            result_tab: None,
        }
    }

    fn payload(first: serde_json::Value, second: serde_json::Value) -> SessionSnapshotPayload {
        serde_json::from_value(json!({
            "focused_pane_id":"w1:p2", "focused_workspace_id":"w1",
            "agents":[],"workspaces":[{"workspace_id":"w1","active_tab_id":"w1:t1"}],
            "tabs":[{"workspace_id":"w1","tab_id":"w1:t1"}],
            "panes":[{"pane_id":"w1:p1","terminal_id":"term-original"},{"pane_id":"w1:p2","terminal_id":"term-target"}],
            "layouts":[{"workspace_id":"w1","tab_id":"w1:t1","zoomed":false,"area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":"w1:p2", "panes":[{"pane_id":"w1:p1","rect":first},{"pane_id":"w1:p2","rect":second}],"splits":[]}]
        })).unwrap()
    }
    fn rect(x: u16, y: u16, width: u16, height: u16) -> serde_json::Value {
        json!({"x":x,"y":y,"width":width,"height":height})
    }

    fn moved(pane: &str, tab: &str, terminal: &str) -> serde_json::Value {
        json!({"type":"pane_move","move_result":{
            "changed":true,"previous_pane_id":"w1:p1","previous_workspace_id":"w1","previous_tab_id":"w1:t1",
            "pane":{"pane_id":pane,"terminal_id":terminal,"workspace_id":"w1","tab_id":tab,"focused":false,"agent_status":"unknown","revision":1},
            "target_layout":{"workspace_id":"w1","tab_id":tab,"zoomed":false,"area":rect(0,0,80,24),"focused_pane_id":pane,"panes":[{"pane_id":pane,"focused":true,"rect":rect(0,0,80,24)}],"splits":[]},"focused_pane_id":pane
        }})
    }

    #[test]
    fn authoritative_geometry_confirms_each_edge_and_rejects_other_placements() {
        for (edge, first, second) in [
            (Edge::Left, rect(0, 0, 40, 24), rect(40, 0, 40, 24)),
            (Edge::Right, rect(40, 0, 40, 24), rect(0, 0, 40, 24)),
            (Edge::Up, rect(0, 0, 80, 12), rect(0, 12, 80, 12)),
            (Edge::Down, rect(0, 12, 80, 12), rect(0, 0, 80, 12)),
        ] {
            let actual = payload(first, second);
            assert!(confirms(&actual, &intent(edge, "w1:t1"), true));
            assert!(!confirms(&actual, &intent(edge, "w1:t2"), true));
            let mut duplicate = actual.clone();
            duplicate.panes.push(actual.panes[0].clone());
            assert!(!confirms(&duplicate, &intent(edge, "w1:t1"), true));
        }
    }

    #[test]
    fn partial_same_tab_reorder_is_unknown_rather_than_a_retryable_refusal() {
        let before = payload(rect(0, 0, 40, 24), rect(40, 0, 40, 24));
        let after = payload(rect(40, 0, 40, 24), rect(0, 0, 40, 24));
        assert!(same_placement(&before, &before, &intent(Edge::Up, "w1:t1")));
        assert!(!same_placement(&before, &after, &intent(Edge::Up, "w1:t1")));
    }

    #[test]
    fn pending_new_tab_move_reserves_the_shared_last_agent_slot_until_settled() {
        let mut runtime = crate::runtime::tests::pane_move::arranged();
        let ids = (1..=63)
            .map(|number| format!("w-order:t{number}"))
            .collect::<Vec<_>>();
        let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
        let mut before = crate::runtime::tests::pane_move::topology(&refs);
        let sibling: crate::sidebar::SessionPanePayload = serde_json::from_value(json!({
            "pane_id":"w-order:extra", "terminal_id":"extra", "cwd":"/fixture/pane-move"
        }))
        .unwrap();
        before.panes.push(sibling);
        let extra = before.layouts[0].panes[0].clone();
        before.layouts[0]
            .panes
            .push(crate::sidebar::SessionLayoutPanePayload {
                pane_id: "w-order:extra".into(),
                ..extra
            });
        runtime.ingest_session(Ok(before));
        let key = (
            crate::node::TEST_NODE.to_owned(),
            "/fixture/pane-move".to_owned(),
        );
        let area = runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(&key.0, &key.1)
            .unwrap()
            .agent_layout
            .tree
            .area_of("w-order:t1")
            .unwrap()
            .id
            .clone();
        let mut first = intent(Edge::Right, "w1:t1");
        first.request.workspace.checkout_path = key.1.clone();
        first.request.source = Source {
            pane_id: "w-order:t2:p".into(),
            tab_id: "w-order:t2".into(),
            area_id: area.clone(),
        };
        first.request.destination = Destination::NewTab {
            area_id: area.clone(),
        };
        first.new_tab_slot = true;
        first.terminal_id = "terminal:w-order:t2:p".into();
        runtime.record_pane_move(first.clone());
        assert_eq!(runtime.pending_agent_admissions(&key), 1);
        assert!(!runtime.admit_agent_tab_for_workspace(&key));
        let mut second = first.request.clone();
        second.request_id = "second".into();
        second.source.pane_id = "w-order:t1:p".into();
        second.source.tab_id = "w-order:t1".into();
        assert!(
            runtime
                .validate_pane_move(&second)
                .unwrap_err()
                .contains("64 Agent tabs")
        );
        runtime.finish_pane_move(&first.request.request_id, "unknown", None, None);
        assert_eq!(runtime.pending_agent_admissions(&key), 1);
        runtime.finish_pane_move(&first.request.request_id, "failed", None, None);
        assert_eq!(runtime.pending_agent_admissions(&key), 0);
        assert!(runtime.admit_agent_tab_for_workspace(&key));
    }

    #[test]
    fn final_publication_barrier_preserves_named_group_area_and_accepts_a_later_close() {
        let mut runtime = crate::runtime::tests::pane_move::arranged();
        let key = (
            crate::node::TEST_NODE.to_owned(),
            "/fixture/pane-move".to_owned(),
        );
        let mut before = crate::runtime::tests::pane_move::topology(&["w-order:t1", "w-order:t2"]);
        let mut remaining = before.panes[0].clone();
        remaining.pane_id = "w-order:remaining".into();
        remaining.terminal_id = Some("remaining-terminal".into());
        before.panes.push(remaining.clone());
        let mut sibling = before.layouts[0].panes[0].clone();
        sibling.pane_id = remaining.pane_id.clone();
        before.layouts[0].panes.push(sibling);
        runtime.ingest_session(Ok(before.clone()));
        let stale_catalog = session_sync::PrecomputedCatalog::here(
            runtime.snapshot.ui_state.workspace_registrations.clone(),
            &before,
            &runtime.worktree_catalog(),
        );
        let (group, area) = {
            let view = runtime
                .workspace_views
                .as_mut()
                .unwrap()
                .views
                .entry(&key.0, &key.1);
            let group = view.create_agent_group("Review").unwrap();
            view.move_agent_to_group("w-order:t1", Some(&group), 1)
                .unwrap();
            view.move_agent_to_group("w-order:t2", Some(&group), 2)
                .unwrap();
            let area = view
                .group_layout_mut(Some(&group))
                .unwrap()
                .tree
                .split("w-order:t2", "a1", Edge::Right, 3)
                .unwrap();
            (group, area)
        };
        let mut final_payload =
            crate::runtime::tests::pane_move::topology(&["w-order:t1", "w-order:t2", "w-order:t3"]);
        final_payload.panes[0] = remaining;
        final_payload.layouts[0].panes[0].pane_id = "w-order:remaining".into();
        final_payload.layouts[0].focused_pane_id = "w-order:remaining".into();
        final_payload.panes[2].pane_id = "w-order:t1:p".into();
        final_payload.panes[2].terminal_id = Some("terminal:w-order:t1:p".into());
        final_payload.layouts[2].panes[0].pane_id = "w-order:t1:p".into();
        final_payload.layouts[2].focused_pane_id = "w-order:t1:p".into();
        let mut closed = final_payload.clone();
        closed.tabs.retain(|tab| tab.tab_id != "w-order:t3");
        closed
            .layouts
            .retain(|layout| layout.tab_id != "w-order:t3");
        closed.panes.retain(|pane| pane.pane_id != "w-order:t1:p");
        let mut moved = intent(Edge::Right, "w1:t1");
        moved.request.workspace.checkout_path = key.1.clone();
        moved.request.group_id = Some(group.clone());
        moved.request.source = Source {
            pane_id: "w-order:t1:p".into(),
            tab_id: "w-order:t1".into(),
            area_id: "a1".into(),
        };
        moved.request.destination = Destination::NewTab {
            area_id: area.clone(),
        };
        moved.terminal_id = "terminal:w-order:t1:p".into();
        runtime.record_pane_move(moved.clone());
        runtime
            .pane_moves
            .get_mut(&moved.request.request_id)
            .unwrap()
            .worker = false;
        runtime.ingest_session(Ok(final_payload.clone()));
        runtime.finish_pane_move(
            &moved.request.request_id,
            "completed",
            None,
            Some(("w-order:t1:p".into(), "w-order:t3".into())),
        );
        let created = std::time::Instant::now();
        let op = runtime
            .pane_moves
            .get_mut(&moved.request.request_id)
            .unwrap();
        op.baseline = Some(final_payload);
        op.publication_barrier = Some(created);
        runtime.ingest_session_with_catalog(Ok(before.clone()), Some(stale_catalog));
        let view = runtime
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(&key.0, &key.1)
            .unwrap();
        assert_eq!(
            view.agent_tab_group("w-order:t3"),
            Some(Some(group.as_str()))
        );
        assert_eq!(
            view.group_layout(Some(&group))
                .unwrap()
                .tree
                .area_of("w-order:t3")
                .unwrap()
                .id,
            area
        );
        assert!(runtime.pane_move_holds_tab("w-order:t3"));
        let locations = runtime
            .catalog_workspaces()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .filter(|tab| {
                tab.panes
                    .iter()
                    .any(|pane| pane.terminal_id.as_deref() == Some(&moved.terminal_id))
            })
            .filter_map(|tab| tab.id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(locations, ["w-order:t3"]);
        runtime.release_pane_move_publication_barriers(None, created - Duration::from_millis(1));
        assert!(
            runtime.pane_moves[&moved.request.request_id]
                .publication_barrier
                .is_some()
        );
        runtime.release_pane_move_publication_barriers(None, std::time::Instant::now());
        runtime.ingest_session(Ok(closed));
        assert!(!runtime.pane_move_holds_tab("w-order:t3"));
        assert!(
            runtime
                .workspace_views
                .as_ref()
                .unwrap()
                .views
                .get(&key.0, &key.1)
                .unwrap()
                .agent_tab_group("w-order:t3")
                .is_none()
        );
    }

    #[test]
    fn same_tab_insertion_detaches_once_and_swaps_only_for_left_or_up() {
        for edge in [Edge::Left, Edge::Right, Edge::Up, Edge::Down] {
            let mut moves = 0;
            let server = FakeHerdr::start("pane-move-same-tab", move |method, _| match method {
                "pane.zoom" | "pane.swap" => json!({"type":"ok"}),
                "pane.move" => {
                    moves += 1;
                    moved(
                        "w1:p1",
                        if moves == 1 { "w1:temporary" } else { "w1:t1" },
                        "term-original",
                    )
                }
                other => panic!("unexpected request {other}"),
            });
            execute(&server.connector(), &intent(edge, "w1:t1"), true).unwrap();
            let calls = server.calls();
            let moving = calls
                .iter()
                .filter(|(method, _)| method == "pane.move")
                .collect::<Vec<_>>();
            assert_eq!(moving.len(), 2);
            assert_eq!(moving[0].1["destination"]["type"], "new_tab");
            assert_eq!(moving[1].1["destination"]["type"], "tab");
            assert_eq!(
                moving[1].1["destination"]["split"],
                if matches!(edge, Edge::Up | Edge::Down) {
                    "down"
                } else {
                    "right"
                }
            );
            assert!(moving.iter().all(|(_, params)| params["focus"] == false));
            assert_eq!(
                calls
                    .iter()
                    .filter(|(method, _)| method == "pane.swap")
                    .count(),
                usize::from(matches!(edge, Edge::Left | Edge::Up))
            );
            assert!(calls.iter().all(|(method, _)| !matches!(
                method.as_str(),
                "pane.split" | "layout.apply" | "tab.create"
            )));
        }
    }

    #[test]
    fn a_lost_or_refused_move_answer_is_never_replayed() {
        let server = FakeHerdr::start_with_errors("pane-move-refusal", |method, _| match method {
            "pane.zoom" => Ok(json!({"type":"ok"})),
            "pane.move" => Err(("target_pane_not_found".into(), "target ended".into())),
            other => panic!("unexpected {other}"),
        });
        assert!(execute(&server.connector(), &intent(Edge::Left, "w1:t2"), true).is_err());
        assert_eq!(server.methods(), ["pane.zoom", "pane.zoom", "pane.move"]);
    }

    #[test]
    fn changed_terminal_on_detach_stops_before_the_join() {
        let server = FakeHerdr::start("pane-move-identity", |method, _| match method {
            "pane.zoom" => json!({"type":"ok"}),
            "pane.move" => moved("w1:p1", "w1:temporary", "another-terminal"),
            other => panic!("unexpected {other}"),
        });
        assert!(
            execute(&server.connector(), &intent(Edge::Right, "w1:t1"), true)
                .unwrap_err()
                .contains("different terminal")
        );
        assert_eq!(server.methods(), ["pane.zoom", "pane.move"]);
    }

    #[test]
    fn live_overlay_keeps_original_tabs_and_restart_exposes_actual_temporary_tab() {
        let baseline = payload(rect(0, 0, 40, 24), rect(40, 0, 40, 24));
        let mut intermediate = baseline.clone();
        intermediate.layouts[0]
            .panes
            .retain(|pane| pane.pane_id != "w1:p1");
        let mut temporary = baseline.layouts[0].clone();
        temporary.tab_id = "w1:temporary".into();
        temporary.panes.retain(|pane| pane.pane_id == "w1:p1");
        intermediate.layouts.push(temporary);
        intermediate.tabs.push(crate::sidebar::SessionTabPayload {
            number: 2,
            workspace_id: "w1".into(),
            tab_id: "w1:temporary".into(),
            label: "".into(),
        });
        let mut runtime = crate::runtime::tests::runtime();
        let intent = intent(Edge::Right, "w1:t1");
        runtime.record_pane_move(intent.clone());
        runtime
            .pane_moves
            .get_mut(&intent.request.request_id)
            .unwrap()
            .baseline = Some(baseline.clone());
        let mut projected = intermediate.clone();
        runtime.overlay_pane_moves(&mut projected);
        assert_eq!(projected.tabs.len(), 1);
        assert_eq!(projected.layouts, baseline.layouts);
        assert!(runtime.pane_move_holds_tab("w1:t1"));
        runtime.pane_moves.clear();
        runtime.overlay_pane_moves(&mut intermediate);
        assert!(
            intermediate
                .tabs
                .iter()
                .any(|tab| tab.tab_id == "w1:temporary")
        );
    }
    #[test]
    fn failed_durable_save_sends_no_native_request() {
        let server = FakeHerdr::start("pane-move-save-refusal", |method, _| {
            panic!("save failure sent {method}")
        });
        let runtime = crate::runtime::tests::SharedRuntime::new(crate::runtime::tests::runtime());
        let intent = intent(Edge::Right, "w1:t2");
        runtime.lock().unwrap().record_pane_move(intent.clone());
        let (send, saved) = sync_channel(1);
        send.send(Err("disk refused".into())).unwrap();
        run(
            RuntimeWorkerContext {
                runtime: runtime.weak(),
                notifier: ChangeNotifier::noop(),
            },
            Arc::new(server.connector()),
            intent.clone(),
            Some(saved),
            false,
        );
        assert!(server.calls().is_empty());
        let guard = runtime.lock().unwrap();
        assert_eq!(
            guard
                .pane_move_operations()
                .iter()
                .find(|op| op.id == intent.request.request_id)
                .unwrap()
                .phase,
            "failed"
        );
        assert!(!guard.pane_move_holds_tab("w1:t1"));
    }

    #[test]
    fn both_tabs_remain_reserved_when_result_is_unknown() {
        let mut runtime = crate::runtime::tests::runtime();
        let mut intent = intent(Edge::Right, "w1:t2");
        intent.phase = "unknown".into();
        runtime.record_pane_move(intent.clone());
        assert!(runtime.pane_move_holds_tab("w1:t1"));
        assert!(runtime.pane_move_holds_tab("w1:t2"));
        assert!(!runtime.pane_move_holds_tab("w1:t3"));
        runtime.finish_pane_move(&intent.request.request_id, "failed", None, None);
        assert!(!runtime.pane_move_holds_tab("w1:t1"));
        assert!(!runtime.pane_move_holds_tab("w1:t2"));
    }
    #[test]
    fn refusal_restores_only_original_zoomed_tab_and_original_keyboard_target() {
        let server = FakeHerdr::start("pane-move-restore-zoom", |_, _| json!({"type":"ok"}));
        let mut before = payload(rect(0, 0, 40, 24), rect(40, 0, 40, 24));
        before.layouts[0].zoomed = true;
        restore_refused_presentation(&server.connector(), &before, &intent(Edge::Right, "w1:t1"))
            .unwrap();
        assert_eq!(
            server.calls(),
            vec![
                ("pane.zoom".into(), json!({"pane_id":"w1:p2","mode":"on"})),
                ("pane.focus".into(), json!({"pane_id":"w1:p2"}))
            ]
        );
    }
    fn native_snapshot() -> serde_json::Value {
        let layout =
            moved("w1:p1", "w1:t1", "term-original")["move_result"]["target_layout"].clone();
        json!({"type":"session_snapshot","snapshot":{
            "protocol":crate::herdr_contract::HERDR_PROTOCOL_REVISION,"version":"fixture",
            "focused_workspace_id":"w1","focused_tab_id":"w1:t1","focused_pane_id":"w1:p1",
            "workspaces":[{"workspace_id":"w1","number":1,"label":"fixture","focused":true,"pane_count":1,"tab_count":1,"active_tab_id":"w1:t1","agent_status":"unknown"}],
            "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","number":1,"label":"fixture","focused":true,"pane_count":1,"agent_status":"unknown"}],
            "panes":[{"pane_id":"w1:p1","terminal_id":"term-original","workspace_id":"w1","tab_id":"w1:t1","focused":true,"agent_status":"unknown","revision":1}],
            "layouts":[layout],"agents":[]
        }})
    }
    #[test]
    fn explicit_check_settles_confirmed_unintended_location_without_repeating_effects() {
        let server = FakeHerdr::start("pane-move-check-only", |method, _| match method {
            "session.snapshot" => native_snapshot(),
            other => panic!("status check mutated {other}"),
        });
        let runtime = crate::runtime::tests::SharedRuntime::new(crate::runtime::tests::runtime());
        let mut intent = intent(Edge::Right, "w1:t2");
        intent.phase = "unknown".into();
        runtime.lock().unwrap().record_pane_move(intent.clone());
        run(
            RuntimeWorkerContext {
                runtime: runtime.weak(),
                notifier: ChangeNotifier::noop(),
            },
            Arc::new(server.connector()),
            intent.clone(),
            None,
            true,
        );
        let guard = runtime.lock().unwrap();
        let operations = guard.pane_move_operations();
        let operation = operations
            .iter()
            .find(|op| op.id == intent.request.request_id)
            .unwrap();
        assert_eq!(operation.phase, "failed");
        assert!(operation.retryable);
        assert!(!guard.pane_move_holds_tab("w1:t1"));
        assert!(!guard.pane_move_holds_tab("w1:t2"));
        assert!(
            server
                .methods()
                .iter()
                .all(|method| method == "session.snapshot")
        );
    }
}
