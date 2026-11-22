//! `hide <path>` in the daemon (`hide_core::project_open`): one request
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
use hide_core::project_open::{ProjectOpenReceipt, Unheld, next_action, valid_path};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;

use crate::boundary::{Boundary, Refusal, Unregistrable};
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
    roots: &crate::RootFollower,
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
    let boundary = roots.boundary();
    let (path, unheld) = if boundary.names_this_node(Some(device)) {
        admit(boundary, raw, || crate::server::roots_current(roots))
            .map_err(|refusal| refused(refusal.code()))?
    } else {
        (raw.to_owned(), Unheld::Register)
    };
    match core
        .project_open(device, &path, request_id, unheld)
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

/// Registers this machine's folder `raw` as a project and brings nothing
/// forward: what a machine switching a device to this one's own Hide asks
/// for each project it had registered there (PRD device-connect-iroh D-48).
/// The folder is judged on hided's `$HOME` line as a picked one is, so one
/// the line would ask the operator about is refused as `hide <path>`
/// refuses it; a folder inside a project that is registered already adds
/// nothing. Blocks for up to `PROJECT_OPEN_WAIT`; runs off the async
/// runtime.
pub(crate) fn run_add(
    core: &CoreHandle,
    roots: &crate::RootFollower,
    raw: &str,
    request_id: &str,
) -> Result<Value, Refused> {
    let refused = |code: &str| (code.to_owned(), next_action(code));
    if !valid_path(raw) {
        return Err(refused("invalid_path"));
    }
    let (path, unheld) = admit(roots.boundary(), raw, || {
        crate::server::roots_current(roots)
    })
    .map_err(|refusal| refused(refusal.code()))?;
    if unheld == Unheld::Refuse {
        return Ok(json!({"path": path, "added": false}));
    }
    let label = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&path)
        .to_owned();
    match core
        .project_add(&path, &label, request_id)
        .map_err(|code| refused(&code))?
    {
        ProjectOpenReceipt::Opened {
            label, path, added, ..
        } => Ok(json!({"label": label, "path": path, "added": added})),
        ProjectOpenReceipt::Refused { reason } => Err(refused(&reason)),
        ProjectOpenReceipt::Pending => Err(refused("project_open_unconfirmed")),
    }
}

/// The spelling a folder of this machine reaches the core in, and whether
/// the core may register it. The `$HOME` line Add a project's Browse folder
/// meets comes first, so a folder under home registers as a picked one does.
/// A folder it would ask the operator to confirm, one outside home, is
/// admitted only when it is written inside a checkout the operator already
/// registered, by the walk that refuses a link out of that checkout, and the
/// core then only brings that checkout forward: the registration is the
/// operator's confirmation, and nothing new is added without the question.
/// The roots follow the core only while a window is open, so a path no root
/// holds brings them current (`catch_up`) and is checked once more before it
/// is refused, as a client event is (`server::admit_event`).
fn admit(
    boundary: &Boundary,
    raw: &str,
    catch_up: impl FnOnce() -> bool,
) -> Result<(String, Unheld), Refusal> {
    let unconfirmed = match boundary.resolve_registration(raw, None) {
        Ok(real) => return Ok((hide_platform::path::to_wire_lossy(&real), Unheld::Register)),
        Err(Unregistrable::Refused(Refusal::NeedsConfirmation))
        | Err(Unregistrable::ConfirmProject(_))
        | Err(Unregistrable::ConfirmHome) => Refusal::NeedsConfirmation,
        Err(Unregistrable::Refused(refusal)) => return Err(refusal),
    };
    let held = match boundary
        .resolve_target(raw)
        .or_else(|refusal| match refusal {
            Refusal::OutsideCheckout if catch_up() => boundary.resolve_target(raw),
            refusal => Err(refusal),
        }) {
        Ok(held) => held,
        Err(Refusal::OutsideCheckout) => return Err(unconfirmed),
        Err(refusal) => return Err(refusal),
    };
    if !held.is_dir() {
        return Err(Refusal::NotADirectory);
    }
    Ok((hide_platform::path::to_wire_lossy(&held), Unheld::Refuse))
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
/// path}`, `{type: "project_add", request_id, path}` for a project that is
/// only registered ([`run_add`]), or `{type: "herdr_import", request_id, yes}` for `hide import
/// herdr` (`herdr_transition::cli_answer`), answered with a
/// `workspace_result` as every scoped request is, then closed. It is never a
/// screen: no snapshot, no terminal.
pub(crate) async fn operator_request(mut socket: WebSocket, state: AppState, connection: u64) {
    let _local = match crate::server::LocalSeat::take(&state) {
        Ok(local) => local,
        Err(count) => {
            crate::server::refuse(
                &mut socket,
                crate::server::CloseReason::ClientLimit,
                Some(count),
            )
            .await;
            return;
        }
    };
    state.clients.fetch_add(1, Ordering::SeqCst);
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
    let adding = request
        .as_ref()
        .is_some_and(|value| value["type"] == "project_add");
    let path = request
        .as_ref()
        .filter(|value| value["type"] == "project_open" || adding)
        .and_then(|value| value["path"].as_str())
        .map(str::to_owned);
    let herdr_import = request
        .as_ref()
        .filter(|value| value["type"] == "herdr_import")
        .map(|value| value["yes"] == true);
    let response = match (request_id, path, herdr_import) {
        (Some(request_id), None, Some(yes)) => {
            let core = std::sync::Arc::clone(&state.core);
            let outcome = tokio::task::spawn_blocking(move || {
                crate::herdr_transition::cli_answer(&core, yes)
            })
            .await;
            let outcome = match outcome {
                Ok(Ok(result)) => Ok(result),
                Ok(Err(_)) | Err(_) => Err((
                    "herdr_import_unavailable".to_owned(),
                    "Run the command again once Hide has started",
                )),
            };
            answer(&request_id, outcome)
        }
        (Some(request_id), Some(path), None) => {
            let (core, roots, calls) = (
                std::sync::Arc::clone(&state.core),
                std::sync::Arc::clone(&state.roots),
                state.window_calls.clone(),
            );
            let windows = state.renderers.load(Ordering::SeqCst);
            let id = request_id.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                if adding {
                    return run_add(&core, &roots, &path, &id);
                }
                let device = roots.boundary().node().as_str().to_owned();
                run(&core, &roots, &calls, windows, &device, &path, &id)
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
