//! Bounded read-only hover content. One read runs and only the newest hover
//! waits; no terminal controller, attachment, focus or grid is touched.
use super::*;
use crate::wire;
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Request {
    pub request_id: String,
    workspace: Workspace,
    #[serde(default)]
    group_id: Option<String>,
    tab_id: String,
}
#[derive(Clone, Debug, Deserialize)]
struct Workspace {
    device_id: String,
    checkout_path: String,
}
#[derive(Debug, Deserialize)]
pub(super) struct Cancel {
    request_id: String,
}
#[derive(Default)]
pub(super) struct Preview {
    running: bool,
    pending: Option<Request>,
}

impl Runtime {
    pub(super) fn request_pane_move_preview(&mut self, request: Request) -> bool {
        if request.request_id.is_empty() || request.request_id.len() > 128 {
            return false;
        }
        let key = (
            request.workspace.device_id.clone(),
            request.workspace.checkout_path.clone(),
        );
        if self.front_workspace_key().as_ref() != Some(&key) {
            return false;
        }
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
        else {
            return false;
        };
        if view.agent_groups.selected != request.group_id
            || view
                .selected_agent_layout()
                .tree
                .area_of(&request.tab_id)
                .is_none()
        {
            return false;
        }
        self.snapshot.status.pane_move_preview = Some(crate::model::PaneMovePreviewSnapshot {
            request_id: request.request_id.clone(),
            tab_id: request.tab_id.clone(),
            phase: "preparing".into(),
            panes: Vec::new(),
            message: None,
        });
        self.pane_move_preview.pending = Some(request);
        self.pump_pane_move_preview();
        true
    }
    pub(super) fn cancel_pane_move_preview(&mut self, cancel: Cancel) -> bool {
        if self
            .snapshot
            .status
            .pane_move_preview
            .as_ref()
            .is_some_and(|preview| preview.request_id == cancel.request_id)
        {
            self.snapshot.status.pane_move_preview = None;
            self.pane_move_preview.pending = None;
            return true;
        }
        false
    }
    fn fail_pane_move_preview(&mut self, message: &str) {
        if let Some(preview) = self.snapshot.status.pane_move_preview.as_mut() {
            preview.phase = "failed".into();
            preview.message = Some(message.into());
        }
    }
    fn pane_move_preview_current(&self, request: &Request) -> bool {
        let key = (
            request.workspace.device_id.clone(),
            request.workspace.checkout_path.clone(),
        );
        self.snapshot
            .status
            .pane_move_preview
            .as_ref()
            .is_some_and(|preview| preview.request_id == request.request_id)
            && self.front_workspace_key().as_ref() == Some(&key)
            && self
                .workspace_views
                .as_ref()
                .and_then(|store| store.views.get(&key.0, &key.1))
                .is_some_and(|view| view.agent_groups.selected == request.group_id)
    }
    fn pump_pane_move_preview(&mut self) {
        if self.pane_move_preview.running {
            return;
        }
        let Some(request) = self.pane_move_preview.pending.take() else {
            return;
        };
        let Some(context) = self.worker_context.clone() else {
            self.fail_pane_move_preview("Pane previews are unavailable in this shell");
            return;
        };
        let device = &request.workspace.device_id;
        let connector = if device == self.node.as_str() {
            self.live
                .as_ref()
                .map(|live| Arc::clone(&live.api_connector))
        } else {
            self.remote_controls
                .get(device)
                .map(|remote| remote.api_connector())
        };
        let Some(connector) = connector else {
            self.fail_pane_move_preview("The terminal connection is unavailable");
            return;
        };
        let panes = self
            .catalog_workspaces()
            .filter(|workspace| &workspace.device_id == device)
            .flat_map(|workspace| &workspace.checkouts)
            .filter(|checkout| checkout.path == request.workspace.checkout_path)
            .flat_map(|checkout| &checkout.tabs)
            .find(|tab| tab.id.as_deref() == Some(&request.tab_id))
            .map(|tab| {
                tab.panes
                    .iter()
                    .map(|pane| pane.id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if panes.is_empty() || panes.len() > 64 {
            self.fail_pane_move_preview("This tab has no bounded live preview");
            return;
        }
        let local = device == self.node.as_str();
        let device = device.clone();
        self.pane_move_preview.running = true;
        let spawned = thread::Builder::new()
            .name("hide-pane-move-preview".into())
            .spawn(move || {
                let mut rows = Vec::new();
                let mut message = None;
                let deadline = Instant::now() + Duration::from_secs(5);
                for pane_id in panes {
                    // Another hover cancels between bounded reads, so only one
                    // already-admitted socket request can outlive cancellation.
                    let current = context.runtime.upgrade().is_some_and(|runtime| {
                        runtime
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .pane_move_preview_current(&request)
                    });
                    if !current {
                        break;
                    }
                    let raw = if local {
                        pane_id.as_str()
                    } else {
                        pane_id
                            .strip_prefix(&format!("remote:{device}:pane:"))
                            .unwrap_or("")
                    };
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        message = Some("The tab preview took too long".into());
                        break;
                    }
                    let read = wire::pane_read_params(raw, "visible", 80)
                        .and_then(|params| {
                            hide_herdr_client::request_small_response(
                                connector.as_ref(),
                                "pane.read",
                                params,
                                remaining.min(Duration::from_secs(2)),
                            )
                            .map_err(|error| error.to_string())
                        })
                        .and_then(wire::pane_text);
                    match read {
                        Ok(read) => rows.push(crate::model::PaneMovePreviewPaneSnapshot {
                            pane_id,
                            text: read.text.chars().take(4096).collect(),
                        }),
                        Err(error) => {
                            message = Some(error);
                            break;
                        }
                    }
                }
                if let Some(runtime) = context.runtime.upgrade() {
                    let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
                    guard.pane_move_preview.running = false;
                    if guard.pane_move_preview_current(&request)
                        && let Some(preview) = guard.snapshot.status.pane_move_preview.as_mut()
                    {
                        preview.panes = rows;
                        preview.phase = if message.is_some() { "failed" } else { "ready" }.into();
                        preview.message = message;
                    }
                    guard.pump_pane_move_preview();
                    drop(guard);
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            self.pane_move_preview.running = false;
            if let Some(preview) = self.snapshot.status.pane_move_preview.as_mut() {
                preview.phase = "failed".into();
                preview.message = Some(error.to_string());
            }
        }
    }
}
