//! Group-qualified starts share the task executor. The saved intent is the
//! write-ahead claim for its external effect, never a saved prompt.
use super::*;
use crate::workspace_views;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

const START_RECEIPT_LIMIT: usize = 64;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StartTarget {
    pub group_id: Option<String>,
    pub area_id: String,
    pub tab_id: Option<String>,
    #[serde(default)]
    pub add_pane: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartState {
    Pending,
    Failed,
    Ready,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StartIntent {
    pub group_id: Option<String>,
    pub area_id: String,
    pub draft_tab_id: Option<String>,
    pub live_tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub add_pane: bool,
    pub marker: String,
    pub state: StartState,
    pub agent_kind: Option<String>,
    pub agent_phase: Option<String>,
    pub recorded_at: u64,
    #[serde(default)]
    pub seen_in_topology: bool,
}

impl StartIntent {
    pub(crate) fn is_pending(&self) -> bool {
        self.state == StartState::Pending
    }
}

pub(super) struct ActiveStart {
    pub task: u64,
    pub workspace: workspace_view::WorkspaceKey,
    pub key: String,
}

pub(super) struct SaveWaiter {
    workspace: workspace_view::WorkspaceKey,
    key: String,
    intent: StartIntent,
    sender: SyncSender<Result<(), String>>,
}

pub(super) struct PreparedStart {
    pub request: live::CheckoutStart,
    pub saved: Receiver<Result<(), String>>,
}

pub(crate) fn receipts(
    view: &workspace_views::WorkspaceView,
) -> Vec<crate::model::AgentStartReceiptSnapshot> {
    view.agent_starts
        .iter()
        .map(
            |(request_id, start)| crate::model::AgentStartReceiptSnapshot {
                request_id: request_id.clone(),
                group_id: start.group_id.clone(),
                area_id: start.area_id.clone(),
                draft_tab_id: start.draft_tab_id.clone(),
                live_tab_id: start.live_tab_id.clone(),
                pane_id: start.pane_id.clone(),
                add_pane: start.add_pane,
                state: match start.state {
                    StartState::Pending => "pending",
                    StartState::Failed => "failed",
                    StartState::Ready => "ready",
                }
                .into(),
                agent_phase: start.agent_phase.clone(),
                agent_kind: start.agent_kind.clone(),
            },
        )
        .collect()
}

impl Runtime {
    /// Duplicate deliveries join the running task or return its settled
    /// receipt. A failed agent is retried through task_agent_retry instead.
    pub(super) fn repeat_checkout_start(&mut self, request: Option<&str>) -> bool {
        request.is_some_and(|request| {
            self.snapshot.task_operation.as_ref().is_some_and(|op| {
                op.request_id.as_deref() == Some(request)
                    && op.phase != "failed"
                    && op.agent_phase.as_deref() != Some("failed")
            })
        })
    }

    pub(super) fn group_start_confirmation_kind(
        &self,
        device: &str,
        path: Option<&str>,
        request: Option<&str>,
    ) -> Result<String, String> {
        let start = path
            .zip(request)
            .and_then(|(path, request)| {
                self.workspace_views
                    .as_ref()?
                    .views
                    .get(device, path)?
                    .agent_starts
                    .get(request)
            })
            .ok_or(
                "The original start receipt is unavailable. Return to its tab before retrying.",
            )?;
        Ok(start
            .agent_kind
            .clone()
            .unwrap_or_else(|| "terminal".into()))
    }

    pub(super) fn prepare_group_start(
        &mut self,
        task: u64,
        device: &str,
        path: &str,
        requested: Option<StartTarget>,
        request_id: Option<&str>,
        agent_kind: Option<&str>,
        mut confirm_only: bool,
    ) -> Result<PreparedStart, String> {
        let workspace = (device.to_owned(), path.to_owned());
        let local = device == self.node.as_str();
        let stays_behind = self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|operation| operation.id == task && operation.stays_behind);
        let active_key = self
            .group_start
            .as_ref()
            .filter(|active| active.workspace == workspace)
            .map(|active| active.key.clone());
        let store = self
            .workspace_views
            .as_mut()
            .ok_or("Workspace settings are not ready. Retry.")?;
        let reserved = store
            .agent_admissions
            .get(&workspace)
            .map_or(0, HashSet::len);
        let view = store.views.entry(device, path);
        let explicit = requested.is_some();
        let target = requested.unwrap_or_else(|| StartTarget {
            group_id: view.agent_groups.selected.clone(),
            area_id: view.selected_agent_layout().tree.active_area.clone(),
            tab_id: None,
            add_pane: false,
        });
        let requested_key = request_id
            .filter(|key| !key.is_empty() && key.len() <= 256 && !key.chars().any(char::is_control))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "start-{}-{task}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )
            });
        // A restored draft's prompt is intentionally not saved. A new renderer
        // request still recovers that draft's original claim before any create.
        let existing = view
            .agent_starts
            .get_key_value(&requested_key)
            .or_else(|| {
                explicit
                    .then(|| {
                        view.agent_starts.iter().find(|(_, start)| {
                            start.state == StartState::Pending
                                && start.group_id == target.group_id
                                && start.area_id == target.area_id
                                && if target.add_pane {
                                    start.add_pane && start.live_tab_id == target.tab_id
                                } else {
                                    !start.add_pane && start.draft_tab_id == target.tab_id
                                }
                        })
                    })
                    .flatten()
            })
            .map(|(key, start)| (key.clone(), start.clone()));
        if confirm_only && existing.is_none() {
            return Err(
                "The original start receipt is unavailable. No process was started.".into(),
            );
        }
        let (key, mut intent, recovery_only) = if let Some((key, existing)) = existing {
            if existing.state == StartState::Pending && active_key.as_deref() != Some(key.as_str())
            {
                // On startup an unanswered claim is only inspected. The
                // operator supplies a new prompt after the original pane is
                // recovered, through the same pane's explicit agent retry.
                confirm_only = true;
            }
            if explicit
                && (existing.group_id != target.group_id
                    || existing.area_id != target.area_id
                    || existing.add_pane != target.add_pane
                    || (target.tab_id.is_some()
                        && target.tab_id != existing.draft_tab_id
                        && target.tab_id != existing.live_tab_id))
            {
                return Err(
                    "This start belongs to another tab. Return to its original tab.".into(),
                );
            }
            if existing.agent_kind.as_deref() != agent_kind {
                return Err("The original start must be confirmed before changing its agent. Retry its original choice.".into());
            }
            if existing.state != StartState::Ready {
                let layout = view.group_layout(existing.group_id.as_deref())
                    .ok_or("The original group is no longer available. Return to its checkout before retrying.")?;
                let tab = existing
                    .draft_tab_id
                    .as_deref()
                    .or(existing.live_tab_id.as_deref())
                    .ok_or("The original start has no target. Refresh before retrying.")?;
                if !layout
                    .tree
                    .area(&existing.area_id)
                    .is_some_and(|area| area.displays.iter().any(|item| item.id == tab))
                {
                    return Err(
                        "The original tab moved. Return to its location before retrying.".into(),
                    );
                }
            }
            if confirm_only && existing.state == StartState::Failed {
                return Err("The original start did not create a process. Retry the start.".into());
            }
            let recovery = confirm_only || existing.state != StartState::Failed;
            (key, existing, recovery)
        } else {
            let layout = view
                .group_layout(target.group_id.as_deref())
                .ok_or("The group was removed. Select a group and retry.")?;
            let area = layout
                .tree
                .area(&target.area_id)
                .ok_or("The area was removed. Select an area and retry.")?;
            if let Some(tab) = target.tab_id.as_deref() {
                if !area.displays.iter().any(|item| item.id == tab) {
                    return Err("The tab moved. Return to the tab and retry.".into());
                }
                if target.add_pane == view.agent_groups.drafts.contains(tab) {
                    return Err("The tab changed. Refresh its start and retry.".into());
                }
            } else if target.add_pane || (explicit && !area.displays.is_empty()) {
                return Err("Select a tab before starting here.".into());
            }
            if view.agent_starts.len() >= START_RECEIPT_LIMIT {
                let oldest = view
                    .agent_starts
                    .iter()
                    .filter(|(_, start)| !start.is_pending())
                    .min_by_key(|(_, start)| start.recorded_at)
                    .map(|(key, _)| key.clone());
                let Some(oldest) = oldest else {
                    return Err("64 starts are awaiting confirmation. Confirm an existing start before opening another.".into());
                };
                view.agent_starts.remove(&oldest);
            }
            if target.tab_id.is_none() && view.agent_tab_count() + reserved >= 64 {
                return Err(
                    "64 Agent tabs are placed or opening. Close a tab to make room.".into(),
                );
            }
            let draft = if !target.add_pane {
                if let Some(tab) = target.tab_id.clone() {
                    Some(tab)
                } else {
                    // A global start is independent and must not change the
                    // selected group before its result. Mint into its target.
                    let selected = view.agent_groups.selected.clone();
                    view.agent_groups.selected = target.group_id.clone();
                    let prior_area = view.selected_agent_layout().tree.active_area.clone();
                    let prior_tab = view
                        .selected_agent_layout()
                        .tree
                        .area(&target.area_id)
                        .and_then(|area| area.active.clone());
                    let draft = view
                        .create_agent_draft(&target.area_id, unix_milliseconds())
                        .map_err(|_| "The tab could not be reserved. Retry.".to_owned());
                    if stays_behind {
                        let layout = view.selected_agent_layout_mut();
                        layout.tree.active_area = prior_area;
                        if let Some(tab) = prior_tab
                            && let Some(area) = layout.tree.area_mut(&target.area_id)
                        {
                            area.active = Some(tab);
                        }
                    }
                    view.agent_groups.selected = selected;
                    Some(draft?)
                }
            } else {
                None
            };
            let marker = format!(
                "{:x}",
                Sha256::digest(format!("{}\0{}\0{}", device, path, requested_key).as_bytes())
            );
            (
                requested_key,
                StartIntent {
                    group_id: target.group_id,
                    area_id: target.area_id,
                    draft_tab_id: draft,
                    live_tab_id: target.add_pane.then_some(target.tab_id).flatten(),
                    pane_id: None,
                    add_pane: target.add_pane,
                    marker,
                    state: StartState::Pending,
                    agent_kind: agent_kind.map(str::to_owned),
                    agent_phase: None,
                    recorded_at: unix_milliseconds(),
                    seen_in_topology: false,
                },
                false,
            )
        };
        let already_ready = intent.state == StartState::Ready;
        if !already_ready {
            intent.state = StartState::Pending;
        }
        let tab_id = intent.live_tab_id.clone();
        let source_tab = |id: Option<String>| -> Result<Option<String>, String> {
            match id {
                Some(id) if !local => remote_tab_source_id(device, &id)
                    .map(|id| Some(id.to_owned()))
                    .ok_or_else(|| "The tab belongs to another device. Refresh and retry.".into()),
                id => Ok(id),
            }
        };
        let target_tab_id = source_tab(intent.add_pane.then_some(tab_id).flatten())?;
        let settled_tab_id = source_tab(
            already_ready
                .then_some(intent.live_tab_id.clone())
                .flatten(),
        )?;
        view.agent_starts.insert(key.clone(), intent.clone());
        store.generation = store.generation.wrapping_add(1);
        let (sender, saved) = sync_channel(1);
        self.group_start_saves.push(SaveWaiter {
            workspace: workspace.clone(),
            key: key.clone(),
            intent: intent.clone(),
            sender,
        });
        self.group_start = Some(ActiveStart {
            task,
            workspace,
            key,
        });
        self.persist_workspace_views();
        Ok(PreparedStart {
            request: live::CheckoutStart {
                marker: intent.marker,
                recovery_only,
                tab_id: target_tab_id,
                settled_tab_id,
                agent_already_attempted: already_ready,
                agent_phase: intent.agent_phase,
                confirm_only,
            },
            saved,
        })
    }

    pub(super) fn acknowledge_group_start_save(
        &mut self,
        saved: &workspace_views::WorkspaceViews,
        result: &Result<(), String>,
    ) {
        self.group_start_saves.retain(|waiter| {
            let matches = saved
                .get(&waiter.workspace.0, &waiter.workspace.1)
                .and_then(|view| view.agent_starts.get(&waiter.key))
                .is_some_and(|saved| {
                    // Topology may confirm the tab while this coalesced save
                    // is queued. That observation does not change the effect
                    // being authorized; every claim/launch field still must
                    // match the exact write-ahead record.
                    let mut saved = saved.clone();
                    saved.seen_in_topology = waiter.intent.seen_in_topology;
                    saved == waiter.intent
                });
            if matches {
                let _ = waiter.sender.try_send(result.clone());
            }
            !matches
        });
    }

    pub(crate) fn group_agent_start_save(
        &mut self,
        task: u64,
    ) -> Option<Receiver<Result<(), String>>> {
        if !self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|op| op.id == task && op.agent_phase.as_deref() == Some("starting"))
        {
            return None;
        }
        let active = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)?;
        let start = self
            .workspace_views
            .as_mut()?
            .views
            .get_mut(&active.workspace.0, &active.workspace.1)?
            .agent_starts
            .get_mut(&active.key)?;
        start.agent_phase = Some("starting".into());
        let intent = start.clone();
        let (sender, receiver) = sync_channel(1);
        self.group_start_saves.push(SaveWaiter {
            workspace: active.workspace.clone(),
            key: active.key.clone(),
            intent,
            sender,
        });
        self.persist_workspace_views();
        Some(receiver)
    }

    pub(super) fn group_start_adds_pane(&self, task: u64) -> bool {
        self.group_start
            .as_ref()
            .filter(|active| active.task == task)
            .and_then(|active| {
                self.workspace_views
                    .as_ref()?
                    .views
                    .get(&active.workspace.0, &active.workspace.1)?
                    .agent_starts
                    .get(&active.key)
            })
            .is_some_and(|start| start.add_pane)
    }

    pub(crate) fn group_start_target_is_current(&self, task: u64) -> bool {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return true;
        };
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&active.workspace.0, &active.workspace.1))
        else {
            return false;
        };
        let Some(start) = view.agent_starts.get(&active.key) else {
            return false;
        };
        if start.state == StartState::Ready {
            return start
                .live_tab_id
                .as_deref()
                .is_some_and(|tab| view.agent_tab_group(tab).is_some());
        }
        let tab = start
            .draft_tab_id
            .as_deref()
            .or(start.live_tab_id.as_deref());
        view.group_layout(start.group_id.as_deref())
            .and_then(|layout| layout.tree.area(&start.area_id))
            .is_some_and(|area| {
                tab.is_some_and(|tab| area.displays.iter().any(|item| item.id == tab))
            })
    }

    /// Whether the user is still looking at the target. A late answer may
    /// fill it, but never selects a group the user has left.
    pub(super) fn group_start_should_focus(&self, task: u64) -> bool {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return true;
        };
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&active.workspace.0, &active.workspace.1))
        else {
            return false;
        };
        let Some(start) = view.agent_starts.get(&active.key) else {
            return false;
        };
        self.front_workspace_key().as_ref() == Some(&active.workspace)
            && view.agent_groups.selected == start.group_id
            && view
                .group_layout(start.group_id.as_deref())
                .is_some_and(|layout| {
                    layout.active()
                        == start
                            .draft_tab_id
                            .as_deref()
                            .or(start.live_tab_id.as_deref())
                })
    }

    pub(crate) fn group_start_failed(&mut self, task: u64, uncertain: bool) {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return;
        };
        if let Some(start) = self
            .workspace_views
            .as_mut()
            .and_then(|store| {
                store
                    .views
                    .get_mut(&active.workspace.0, &active.workspace.1)
            })
            .and_then(|view| view.agent_starts.get_mut(&active.key))
        {
            if start.state != StartState::Ready {
                start.state = if uncertain {
                    StartState::Pending
                } else {
                    StartState::Failed
                };
            }
        }
        self.group_start_saves
            .retain(|waiter| waiter.workspace != active.workspace || waiter.key != active.key);
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation = store.generation.wrapping_add(1);
        }
        self.persist_workspace_views();
    }

    pub(super) fn complete_group_start(&mut self, task: u64, tab: Option<&str>, pane: &str) {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return;
        };
        let local = active.workspace.0 == self.node.as_str();
        let Some(tab) = tab else {
            return;
        };
        let tab = if local {
            tab.to_owned()
        } else {
            crate::session_sync::remote_tab_id(&active.workspace.0, tab)
        };
        let pane = if local {
            pane.to_owned()
        } else {
            operations::remote_pane_id(&active.workspace.0, pane)
        };
        let seen = !self
            .provisional_tabs
            .iter()
            .any(|drawn| drawn.tab_id == tab)
            && self
                .catalog_workspaces()
                .filter(|workspace| workspace.device_id == active.workspace.0)
                .flat_map(|workspace| &workspace.checkouts)
                .filter(|checkout| checkout.path == active.workspace.1)
                .flat_map(|checkout| &checkout.tabs)
                .any(|item| item.id.as_deref() == Some(tab.as_str()));
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let Some(view) = store
            .views
            .get_mut(&active.workspace.0, &active.workspace.1)
        else {
            return;
        };
        let Some(mut start) = view.agent_starts.get(&active.key).cloned() else {
            return;
        };
        let promoted = !start.add_pane && start.state != StartState::Ready;
        if promoted {
            // Topology may arrive before its acknowledgement; remove that
            // provisional ungrouped entry before replacing the draft slot.
            if let Some(owner) = view.agent_tab_group(&tab).map(|g| g.map(str::to_owned)) {
                view.group_layout_mut(owner.as_deref())
                    .unwrap()
                    .tree
                    .remove(&tab);
            }
            if let Some(draft) = start.draft_tab_id.as_deref() {
                if let Some(layout) = view.group_layout_mut(start.group_id.as_deref()) {
                    if let Some(area) = layout.tree.area_mut(&start.area_id) {
                        if let Some(item) = area.displays.iter_mut().find(|item| item.id == draft) {
                            item.id = tab.to_owned();
                            if area.active.as_deref() == Some(draft) {
                                area.active = Some(tab.to_owned());
                            }
                        }
                    }
                }
                view.agent_groups.drafts.remove(draft);
            }
        }
        start.live_tab_id = Some(tab);
        start.pane_id = Some(pane);
        start.state = StartState::Ready;
        start.seen_in_topology |= seen;
        if start.agent_phase.is_none() {
            start.agent_phase = start.agent_kind.as_ref().map(|_| "starting".into());
        }
        view.agent_starts.insert(active.key.clone(), start);
        if promoted {
            view.invalidate_agent_group_undo();
        }
        store.generation = store.generation.wrapping_add(1);
        self.persist_workspace_views();
    }

    pub(super) fn save_group_agent_result(&mut self, task: u64, phase: &str) {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return;
        };
        if let Some(start) = self
            .workspace_views
            .as_mut()
            .and_then(|store| {
                store
                    .views
                    .get_mut(&active.workspace.0, &active.workspace.1)
            })
            .and_then(|view| view.agent_starts.get_mut(&active.key))
        {
            start.agent_phase = Some(phase.to_owned());
        }
        self.group_start_saves
            .retain(|waiter| waiter.workspace != active.workspace || waiter.key != active.key);
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation = store.generation.wrapping_add(1);
        }
        self.persist_workspace_views();
    }
}
