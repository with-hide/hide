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
    #[serde(default)]
    pub effect: Option<crate::start_claim::Effect>,
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
    explicit_target: bool,
}

pub(super) struct TaskStartFocus {
    task: u64,
    /// Global starts may bring their checkout forward only while the
    /// original screen's selection has not been superseded.
    global_focus: Option<StartFocus>,
    pending_focus: Option<StartPaneFocus>,
}

struct StartPaneFocus {
    workspace: workspace_view::WorkspaceKey,
    tab: Option<String>,
    pane: String,
    generation: u64,
    deadline: std::time::Instant,
}

#[derive(PartialEq)]
struct StartFocus {
    workspace: Option<workspace_view::WorkspaceKey>,
    serial: u64,
    group: Option<String>,
    tab: Option<String>,
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

/// A physical staging tab consumes the already admitted draft/start, not a
/// second visible tab. Hide only its own exact claim, never unrelated panes
/// that somebody subsequently placed beside the staged terminal.
pub(super) fn staged_tab(
    view: &workspace_views::WorkspaceView,
    local: bool,
    device: &str,
    owner: Option<&str>,
    tab: &TabSnapshot,
) -> bool {
    let scoped = |id: &str| {
        if local {
            id.to_owned()
        } else {
            crate::session_sync::remote_tab_id(device, id)
        }
    };
    view.agent_starts.values().any(|start| {
        let Some(effect) = start.effect.as_ref().filter(|_| start.is_pending()) else {
            return false;
        };
        if owner != Some(effect.workspace_id.as_str())
            || effect
                .target
                .as_ref()
                .is_some_and(|target| tab.id.as_deref() == Some(scoped(&target.tab_id).as_str()))
        {
            return false;
        }
        let claim = crate::start_claim::label(&start.marker);
        let seed = effect.seed_tab_id.as_ref().is_some_and(|id| {
            tab.id.as_deref() == Some(scoped(id).as_str())
                && effect.binding.is_none()
                && tab.panes.len() == 1
                && !tab.naming.by_operator
                && tab.agent.is_none()
        });
        seed || (!tab.panes.is_empty()
            && tab
                .panes
                .iter()
                .all(|pane| pane.start_claim.as_deref() == Some(claim.as_str())))
    })
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
            .ok_or("agent_start.confirmation_unavailable")?;
        Ok(start
            .agent_kind
            .clone()
            .unwrap_or_else(|| "terminal".into()))
    }

    pub(super) fn prepare_group_start(
        &mut self,
        task: u64,
        workspace: (&str, &str),
        requested: Option<StartTarget>,
        request_id: Option<&str>,
        agent_kind: Option<&str>,
        mut confirm_only: bool,
    ) -> Result<PreparedStart, String> {
        let (device, path) = workspace;
        let workspace = (device.to_owned(), path.to_owned());
        let local = device == self.node.as_str();
        let delegated_tabs: HashSet<String> = self
            .catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == path)
            .flat_map(|checkout| &checkout.tabs)
            .filter(|tab| tab.delegated)
            .filter_map(|tab| tab.id.clone())
            .collect();
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
            .ok_or("task_operation.settings_not_ready")?;
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
            return Err("agent_start.confirmation_unavailable".into());
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
                if !layout.tree.area(&existing.area_id).is_some_and(|area| {
                    area.displays.iter().any(|item| item.id == tab)
                        || (existing.add_pane && delegated_tabs.contains(tab))
                }) {
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
                if !area.displays.iter().any(|item| item.id == tab)
                    && !(target.add_pane
                        && delegated_tabs.contains(tab)
                        && layout
                            .canvases
                            .get(&target.area_id)
                            .is_some_and(|shown| shown == tab))
                {
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
                    let prior_canvas = view
                        .selected_agent_layout()
                        .canvases
                        .get(&target.area_id)
                        .cloned();
                    let draft = view
                        .create_agent_draft(&target.area_id, unix_milliseconds())
                        .map_err(|_| "The tab could not be reserved. Retry.".to_owned());
                    if !explicit || stays_behind {
                        let layout = view.selected_agent_layout_mut();
                        layout.tree.active_area = prior_area;
                        if let Some(area) = layout.tree.area_mut(&target.area_id) {
                            area.active = prior_tab;
                        }
                        if let Some(canvas) = prior_canvas {
                            layout.canvases.insert(target.area_id.clone(), canvas);
                        } else {
                            layout.canvases.remove(&target.area_id);
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
                    effect: None,
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
        if self
            .task_start_focus
            .as_ref()
            .is_none_or(|focus| focus.task != task)
        {
            self.capture_task_start_focus(task, "agent_start");
        }
        if explicit || stays_behind {
            self.supersede_start_navigation();
        }
        self.group_start = Some(ActiveStart {
            task,
            workspace,
            key,
            explicit_target: explicit,
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
                effect: intent.effect,
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

    pub(crate) fn save_group_start_effect(
        &mut self,
        task: u64,
        effect: crate::start_claim::Effect,
    ) -> Result<Receiver<Result<(), String>>, String> {
        let active = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
            .ok_or("The start is no longer current. Check its original tab.")?;
        let store = self
            .workspace_views
            .as_mut()
            .ok_or("The start cannot be saved. Retry its status check.")?;
        let start = store
            .views
            .get_mut(&active.workspace.0, &active.workspace.1)
            .and_then(|view| view.agent_starts.get_mut(&active.key))
            .ok_or("The start has no saved intent. Check its original tab.")?;
        if let Some(previous) = &start.effect
            && (previous.workspace_id != effect.workspace_id
                || previous.seed_tab_id != effect.seed_tab_id
                || previous.target != effect.target
                || previous.expected_command != effect.expected_command
                || previous.shell_env != effect.shell_env
                || previous.shell_readiness != effect.shell_readiness
                || previous
                    .binding
                    .as_ref()
                    .is_some_and(|bound| effect.binding.as_ref() != Some(bound)))
        {
            return Err(
                "The original start's identity changed. Inspect its tab before continuing.".into(),
            );
        }
        start.effect = Some(effect);
        let intent = start.clone();
        let (sender, receiver) = sync_channel(1);
        self.group_start_saves.push(SaveWaiter {
            workspace: active.workspace.clone(),
            key: active.key.clone(),
            intent,
            sender,
        });
        store.generation = store.generation.wrapping_add(1);
        self.persist_workspace_views();
        Ok(receiver)
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

    /// The immutable creation proof a dependent agent launch must recheck.
    /// Ordinary task starts have no group receipt and retain their own policy.
    pub(crate) fn group_start_binding(
        &self,
        task: u64,
    ) -> Result<Option<crate::start_claim::BoundPane>, String> {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return Ok(None);
        };
        self.workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&active.workspace.0, &active.workspace.1))
            .and_then(|view| view.agent_starts.get(&active.key))
            .and_then(|start| start.effect.as_ref())
            .and_then(|effect| effect.binding.clone())
            .map(Some)
            .ok_or_else(|| "agent_start.terminal_unconfirmed".into())
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
        // A delegated tab has no ordinary strip membership. Once its exact
        // visible canvas admitted an add-pane request, that recorded tab may
        // finish in the background even if another canvas is selected.
        let live_delegated = start.add_pane
            && start.live_tab_id.as_deref().is_some_and(|id| {
                self.catalog_workspaces()
                    .filter(|workspace| workspace.device_id == active.workspace.0)
                    .flat_map(|workspace| &workspace.checkouts)
                    .filter(|checkout| checkout.path == active.workspace.1)
                    .flat_map(|checkout| &checkout.tabs)
                    .any(|tab| tab.delegated && tab.id.as_deref() == Some(id))
            });
        if start.state == StartState::Ready {
            return start
                .live_tab_id
                .as_deref()
                .is_some_and(|tab| view.agent_tab_group(tab).is_some())
                || live_delegated;
        }
        let tab = start
            .draft_tab_id
            .as_deref()
            .or(start.live_tab_id.as_deref());
        view.group_layout(start.group_id.as_deref())
            .and_then(|layout| layout.tree.area(&start.area_id))
            .is_some_and(|area| {
                tab.is_some_and(|tab| area.displays.iter().any(|item| item.id == tab))
                    || live_delegated
            })
    }

    fn start_focus(&self) -> StartFocus {
        let workspace = self.front_workspace_key();
        let view = workspace
            .as_ref()
            .and_then(|key| self.workspace_views.as_ref()?.views.get(&key.0, &key.1));
        StartFocus {
            workspace: workspace.clone(),
            serial: self.last_view_intent_serial,
            group: view.and_then(|view| view.agent_groups.selected.clone()),
            tab: view.and_then(|view| view.selected_agent_layout().active().map(str::to_owned)),
        }
    }

    /// A user visit remains newer even after returning to the original empty
    /// group or draft, which needs no Herdr focus turn of its own.
    pub(super) fn supersede_start_navigation(&mut self) {
        if let Some(active) = self.task_start_focus.as_mut() {
            active.global_focus = None;
            active.pending_focus = None;
        }
    }

    /// An agent start and a new worktree both bring their pane forward, so
    /// both hold this permission and its catalog follow.
    pub(super) fn capture_task_start_focus(&mut self, task: u64, kind: &str) {
        self.task_start_focus =
            matches!(kind, "agent_start" | "worktree_create").then(|| TaskStartFocus {
                task,
                global_focus: Some(self.start_focus()),
                pending_focus: None,
            });
    }

    /// One pending follow belongs to the current start, never a shell-side
    /// open intent. It expires with the start panel's 90-second answer bound.
    pub(super) fn queue_start_focus(&mut self, task: u64, tab: Option<String>, pane: String) {
        let operation = self
            .snapshot
            .task_operation
            .as_ref()
            .expect("the task result is ready");
        let device = operation
            .device_id
            .clone()
            .unwrap_or_else(|| self.node.as_str().to_owned());
        let path = operation
            .path
            .clone()
            .expect("a successful start names its checkout");
        let Some(active) = self
            .task_start_focus
            .as_mut()
            .filter(|start| start.task == task)
        else {
            self.push_diagnostic(
                "agent_start.focus_unavailable",
                format!("Task {task} has no navigation permission"),
            );
            return;
        };
        active.pending_focus = Some(StartPaneFocus {
            workspace: (device.clone(), path),
            tab,
            pane,
            generation: if device == self.node.as_str() {
                self.live_generation
            } else {
                self.remote_connection_generations
                    .get(&device)
                    .copied()
                    .unwrap_or(0)
            },
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(90),
        });
    }

    /// Automatic archive descendants share the start slot, so they wait for
    /// this bounded follow rather than replacing their root's permission.
    pub(super) fn pending_start_focus(&mut self) -> bool {
        let Some(active) = self.task_start_focus.as_ref() else {
            return false;
        };
        let Some(pending) = active.pending_focus.as_ref() else {
            return false;
        };
        let device = &pending.workspace.0;
        let current = self
            .snapshot
            .task_operation
            .as_ref()
            .is_some_and(|op| op.id == active.task && op.phase == "ready" && !op.stays_behind)
            && self.group_start_should_focus(active.task)
            && pending.generation
                == if device == self.node.as_str() {
                    self.live_generation
                } else {
                    self.remote_connection_generations
                        .get(device.as_str())
                        .copied()
                        .unwrap_or(0)
                }
            && std::time::Instant::now() < pending.deadline;
        if !current {
            let device = device.clone();
            self.task_start_focus.as_mut().unwrap().pending_focus = None;
            self.push_diagnostic(
                "agent_start.focus_cancelled",
                format!("Start focus on {device} was superseded or expired"),
            );
            return false;
        }
        true
    }

    /// A start answer names the final tab, while an older catalog may still
    /// place its pane in the temporary staging tab used by an add-pane start.
    pub(super) fn start_pane_is_listed(
        &self,
        device: &str,
        path: &str,
        tab_id: Option<&str>,
        pane_id: &str,
    ) -> bool {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == path)
            .flat_map(|checkout| &checkout.tabs)
            .filter(|tab| tab_id.is_none_or(|id| tab.id.as_deref() == Some(id)))
            .flat_map(|tab| &tab.panes)
            .any(|pane| pane.id == pane_id)
    }

    pub(super) fn follow_start(&mut self, device: Option<&str>) -> bool {
        if !self.pending_start_focus() {
            return false;
        }
        let device = device.unwrap_or(self.node.as_str());
        let active = self.task_start_focus.as_ref().unwrap();
        let Some(pending) = active
            .pending_focus
            .as_ref()
            .filter(|pending| pending.workspace.0 == device)
        else {
            return false;
        };
        // The answer's final tab must own the exact pane on this connection
        // before the ordinary focus boundary accepts it.
        if !self.start_pane_is_listed(
            device,
            &pending.workspace.1,
            pending.tab.as_deref(),
            &pending.pane,
        ) {
            return false;
        }
        let task = active.task;
        let pending = self
            .task_start_focus
            .as_mut()
            .unwrap()
            .pending_focus
            .take()
            .unwrap();
        let device = pending.workspace.0;
        if device == self.node.as_str() {
            self.focus_pane(pending.pane, PaneFocusOrigin::Restore, None);
            if self.snapshot.status.last_error.is_none() && !self.device_in_front(&device) {
                self.bring_device_forward(device);
            }
            return true;
        }
        let request_id = format!("agent-start-focus:{task}:{}", pending.generation);
        self.request_remote_control(RemoteControlPayload {
            target_id: device,
            request_id,
            report_pane_focus_outcome: false,
            focus_device: true,
            request: RemoteControlRequest::FocusPane {
                pane_id: pending.pane,
            },
        })
    }

    /// A targeted start keeps its selected slot. A global start may open its
    /// checkout if no intervening selection superseded the original press.
    pub(super) fn group_start_should_focus(&self, task: u64) -> bool {
        let origin = self
            .task_start_focus
            .as_ref()
            .filter(|focus| focus.task == task);
        let global_current = origin.is_none_or(|focus| {
            focus.global_focus.as_ref().is_some_and(|origin| {
                // Catalog restoration may replace the selected tab of an
                // empty checkout. Only an accepted visit supersedes this
                // permission, through the serial or explicit invalidation.
                origin.serial == self.last_view_intent_serial
            })
        });
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return global_current;
        };
        if !active.explicit_target {
            return global_current && self.group_start_target_is_current(task);
        }
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
        (self.front_workspace_key().as_ref() == Some(&active.workspace)
            || origin
                .and_then(|focus| focus.global_focus.as_ref())
                .is_some_and(|origin| *origin == self.start_focus()))
            && view.agent_groups.selected == start.group_id
            && view
                .group_layout(start.group_id.as_deref())
                .is_some_and(|layout| {
                    layout.active()
                        == if start.state == StartState::Ready {
                            start.live_tab_id.as_deref()
                        } else {
                            start
                                .draft_tab_id
                                .as_deref()
                                .or(start.live_tab_id.as_deref())
                        }
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
            && start.state != StartState::Ready
        {
            start.state = if uncertain {
                StartState::Pending
            } else {
                StartState::Failed
            };
        }
        self.group_start_saves
            .retain(|waiter| waiter.workspace != active.workspace || waiter.key != active.key);
        if let Some(store) = self.workspace_views.as_mut() {
            store.generation = store.generation.wrapping_add(1);
        }
        self.persist_workspace_views();
    }

    pub(super) fn complete_group_start(
        &mut self,
        task: u64,
        tab: Option<&str>,
        pane: &str,
        should_focus: bool,
    ) {
        let Some(active) = self
            .group_start
            .as_ref()
            .filter(|active| active.task == task)
        else {
            return;
        };
        let local = active.workspace.0 == self.node.as_str();
        let follows_local_global = local && !active.explicit_target && should_focus;
        let workspace = active.workspace.clone();
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
        let mut follow_local_global = follows_local_global && promoted;
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
                if let Some(layout) = view.group_layout_mut(start.group_id.as_deref())
                    && let Some(area) = layout.tree.area_mut(&start.area_id)
                    && let Some(item) = area.displays.iter_mut().find(|item| item.id == draft)
                {
                    item.id = tab.to_owned();
                    if area.active.as_deref() == Some(draft) {
                        area.active = Some(tab.to_owned());
                    }
                }
                view.agent_groups.drafts.remove(draft);
            }
        }
        start.live_tab_id = Some(tab);
        start.pane_id = Some(pane);
        start.state = StartState::Ready;
        start.seen_in_topology |= seen;
        // Only a newly filled slot needs selection before topology arrives.
        // A Ready receipt may retain a group that was legally moved or
        // dissolved; its ordinary pane focus follows the current location.
        let selection_error = if follow_local_global {
            let selection = view
                .group_layout_mut(start.group_id.as_deref())
                .ok_or_else(|| "The admitted start group is no longer present".to_owned())
                .and_then(|layout| {
                    let stamp = layout.tree.next_stamp(unix_milliseconds());
                    layout
                        .select(start.live_tab_id.as_deref().unwrap(), false, stamp)
                        .map_err(|error| format!("{error:?}"))
                });
            match selection {
                Ok(_) => {
                    view.agent_groups.selected = start.group_id.clone();
                    None
                }
                Err(error) => {
                    follow_local_global = false;
                    Some(error)
                }
            }
        } else {
            None
        };
        if start.agent_phase.is_none() {
            start.agent_phase = start.agent_kind.as_ref().map(|_| "starting".into());
        }
        view.agent_starts.insert(active.key.clone(), start);
        if promoted {
            view.invalidate_agent_group_undo();
        }
        store.generation = store.generation.wrapping_add(1);
        self.persist_workspace_views();
        if let Some(error) = selection_error {
            self.push_diagnostic("agent_start.slot_selection_failed", error);
        }
        if follow_local_global {
            // The result owns this selection even before its first owner
            // appears in topology. Clear the old visible pane in this event;
            // result ingestion follows with the ordinary ordered pane focus.
            if let Some((workspace_id, checkout_id)) = self.agent_workspace_ids(&workspace) {
                self.focus_checkout(&workspace_id, &checkout_id);
                self.clear_agent_workspace_pane(&checkout_id);
            }
        }
        self.request_republish();
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
