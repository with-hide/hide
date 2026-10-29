//! `hide <path>` in the daemon (`herdr_core::project_open`): one request
//! names a folder on the caller's machine; it comes forward as a project,
//! registered first through the same `create_workspace` the Add a project
//! dialog sends, and every window of this machine is asked to come forward
//! and show it.
//!
//! Two callers reach it. A command on the core's own machine holds the
//! daemon's token from the state file the account owns, which is the
//! authority bare `hide` opens the shell with, and asks over one `/ws`
//! connection whose handshake names `client_kind: "cli"`
//! ([`operator_request`]). A command in a pane, a device's included, asks
//! with its pane credential as every scoped request does, and its machine
//! is the credential's (`server::scoped_client_loop`). A plain shell of a
//! device's own account asks its node, which sends the request up its link
//! with no credential (`node_panes`, `NodeEvent::ProjectOpen`), and its
//! machine is that link's node. A folder on this
//! machine is judged on hided's `$HOME` line first, exactly as a picked
//! folder is; a device's folder is judged by that device's helper inside the
//! core's registration.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use herdr_core::project_open::{ProjectOpenReceipt, next_action, valid_path};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

use crate::boundary::{Boundary, Unregistrable};
use crate::core::CoreHandle;
use crate::server::AppState;

/// The refusal of a request, with what its caller does next.
pub(crate) type Refused = (String, &'static str);

/// Brings the project holding `raw` on `device` in front, registering it
/// first when no registered checkout holds it, and calls this machine's
/// windows forward. Blocks for up to `PROJECT_OPEN_WAIT`; runs off the
/// async runtime.
pub(crate) fn run(
    core: &CoreHandle,
    boundary: &Boundary,
    window_calls: &tokio::sync::broadcast::Sender<Value>,
    windows: usize,
    device: &str,
    raw: &str,
    request_id: &str,
) -> Result<Value, Refused> {
    let refused = |code: &str| (code.to_owned(), next_action(code));
    if !valid_path(raw) {
        return Err(refused("invalid_path"));
    }
    let path = if boundary.names_this_node(Some(device)) {
        match boundary.resolve_registration(raw, None) {
            Ok(real) => hide_platform::path::to_wire_lossy(&real),
            Err(Unregistrable::Refused(refusal)) => return Err(refused(refusal.code())),
            Err(Unregistrable::ConfirmProject(_)) => return Err(refused("needs_confirmation")),
        }
    } else {
        raw.to_owned()
    };
    match core
        .project_open(device, &path, request_id)
        .map_err(|code| refused(&code))?
    {
        ProjectOpenReceipt::Opened {
            label,
            path,
            checkout_id,
            added,
        } => {
            // No window receives it when none is open; the caller opens one.
            let _ = window_calls
                .send(json!({"device_id": device, "path": path, "checkout_id": checkout_id}));
            Ok(json!({"label": label, "path": path, "added": added, "window": windows > 0}))
        }
        ProjectOpenReceipt::Refused { reason } => Err(refused(&reason)),
        ProjectOpenReceipt::Pending => Err(refused("project_open_unconfirmed")),
    }
}

/// The `workspace_result` line `hide <path>` reads for `outcome`.
pub(crate) fn answer(request_id: &str, outcome: Result<Value, Refused>) -> Value {
    match outcome {
        Ok(result) => {
            json!({"type":"workspace_result","request_id":request_id,"ok":true,"result":result})
        }
        Err((reason, next_action)) => {
            json!({"type":"workspace_result","request_id":request_id,"ok":false,"reason":reason,"next_action":next_action})
        }
    }
}

/// The one request a `/ws` connection that proved the daemon's own token
/// with `client_kind: "cli"` makes: `{type: "project_open", request_id,
/// path}`, answered with a `workspace_result` as every scoped request is,
/// then closed. It is never a screen: no snapshot, no terminal, no event.
pub(crate) async fn operator_request(mut socket: WebSocket, state: AppState, connection: u64) {
    let previous = state.clients.fetch_add(1, Ordering::SeqCst);
    if previous >= crate::state_file::MAX_CLIENTS {
        state.clients.fetch_sub(1, Ordering::SeqCst);
        crate::server::refuse(
            &mut socket,
            crate::server::CloseReason::ClientLimit,
            Some(previous + 1),
        )
        .await;
        return;
    }
    let incoming = tokio::time::timeout(Duration::from_secs(10), socket.recv()).await;
    let request = match incoming {
        Ok(Some(Ok(Message::Text(text)))) if text.len() <= 16 * 1024 => {
            serde_json::from_str::<Value>(&text).ok()
        }
        _ => None,
    };
    let request_id = request
        .as_ref()
        .and_then(|value| value["request_id"].as_str())
        .filter(|id| crate::workspace_cli::valid_request_id(id))
        .map(str::to_owned);
    let path = request
        .as_ref()
        .filter(|value| value["type"] == "project_open")
        .and_then(|value| value["path"].as_str())
        .map(str::to_owned);
    let response = match (request_id, path) {
        (Some(request_id), Some(path)) => {
            let (core, boundary, calls) = (
                std::sync::Arc::clone(&state.core),
                std::sync::Arc::clone(&state.boundary),
                state.window_calls.clone(),
            );
            let windows = state.renderers.load(Ordering::SeqCst);
            let id = request_id.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let device = boundary.node().as_str().to_owned();
                run(&core, &boundary, &calls, windows, &device, &path, &id)
            })
            .await;
            match outcome {
                Ok(outcome) => answer(&request_id, outcome),
                Err(_) => answer(
                    &request_id,
                    Err(("project_open_unavailable".to_owned(), next_action(""))),
                ),
            }
        }
        _ => {
            json!({"type":"workspace_result","ok":false,"reason":"invalid_request","next_action":"Check hide command arguments and retry"})
        }
    };
    let _ = socket
        .send(Message::Text(response.to_string().into()))
        .await;
    let _ = socket.send(Message::Close(None)).await;
    crate::server::client_gone(&state, connection, false, false);
}
