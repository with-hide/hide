//! The role a process mounts as its core or node stops for a move, until
//! the next role starts (PRD core-host-node-move Q12): `/health` answers
//! with `role: "moving"`, so the desktop host never reads the daemon as
//! lost, and a window's socket hears the move's `core_move` frames. It is
//! mounted before the role it replaces stops, so a window whose socket that
//! role closes (1012 `role_ended`) reconnects here. A window's `core_move`
//! events reach the move (the abandon its wait offers among them); every
//! other event waits for the role after it. Ending this role closes every
//! socket with 1012, and the page reconnects to the role mounted next.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::control::{MoveControl, frame};

#[derive(Clone)]
struct ScreenState {
    token: Arc<String>,
    allowed_origins: Arc<HashSet<String>>,
    moves: Arc<MoveControl>,
    instance: Arc<dyn Fn() -> u64 + Send + Sync>,
    ui_dir: Option<PathBuf>,
    ended: watch::Receiver<bool>,
}

/// The mounted move screen; dropping it closes its windows' sockets.
pub struct MoveScreen {
    ended: watch::Sender<bool>,
}

impl Drop for MoveScreen {
    fn drop(&mut self) {
        self.ended.send_replace(true);
    }
}

impl MoveScreen {
    pub fn mount(seat: &crate::seat::SeatParts, vite_origin: Option<&str>) -> Self {
        let (ended, ended_rx) = watch::channel(false);
        let parts = seat.clone();
        let state = ScreenState {
            token: Arc::new(seat.token.clone()),
            allowed_origins: Arc::new(crate::server::allowed_origins(seat.port, vite_origin)),
            moves: Arc::clone(&seat.moves),
            instance: Arc::new(move || parts.instance()),
            ui_dir: if crate::server::has_embedded_ui() {
                None
            } else {
                crate::find_ui_dir()
            },
            ended: ended_rx,
        };
        seat.mount(
            Router::new()
                .route("/health", get(health))
                .route("/ws", get(ws_upgrade))
                .route("/", get(asset))
                .route("/assets/{*path}", get(asset))
                .fallback(|| async { axum::http::StatusCode::NOT_FOUND })
                .with_state(state),
        );
        Self { ended }
    }
}

async fn health(State(state): State<ScreenState>) -> impl IntoResponse {
    axum::Json(json!({
        "pid": std::process::id(),
        "role": "moving",
        "version": crate::VERSION,
        "instance": (state.instance)(),
    }))
}

async fn asset(uri: Uri, State(state): State<ScreenState>) -> Response {
    crate::server::ui_asset(state.ui_dir.as_deref(), uri.path()).await
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    State(state): State<ScreenState>,
) -> Response {
    let origin = headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    ws.on_upgrade(move |socket| serve(socket, state, origin))
}

async fn serve(mut socket: WebSocket, mut state: ScreenState, origin: Option<String>) {
    let close = |code: u16, reason: &'static str| {
        Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))
    };
    if crate::server::check_origin(origin.as_deref(), &state.allowed_origins).is_err() {
        let _ = socket.send(close(4003, "origin_not_allowed")).await;
        return;
    }
    let first = tokio::time::timeout(crate::server::FIRST_FRAME_TIMEOUT, socket.recv()).await;
    let Ok(Some(Ok(Message::Text(text)))) = first else {
        let _ = socket.send(close(4001, "invalid_token")).await;
        return;
    };
    let offered = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .get("token")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if !crate::server::token_matches(&offered, &state.token) {
        let _ = socket.send(close(4001, "invalid_token")).await;
        return;
    }
    let mover = super::control::MoveScreen::new(&state.moves);
    let mut frames = state.moves.subscribe();
    let first = frame(&frames.borrow_and_update());
    if socket.send(Message::Text(first.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            changed = frames.changed() => {
                if changed.is_err() {
                    break;
                }
                let next = frame(&frames.borrow_and_update());
                if socket.send(Message::Text(next.into())).await.is_err() {
                    return;
                }
            }
            () = async { let _ = state.ended.wait_for(|ended| *ended).await; } => break,
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let taken = crate::screen_event::read(&text, &[crate::screen_event::Kind::CoreMove])
                        .map(|routed| mover.request_event(&routed.event));
                    let refused = match taken {
                        Some(Ok(())) => continue,
                        Some(Err(reason)) => super::control::refusal_frame(reason),
                        // Every other event waits for the role after the move.
                        None => json!({"type": "error", "payload": {"kind": "core_move", "reason": "moving"}, "message": "moving"}).to_string(),
                    };
                    if socket.send(Message::Text(refused.into())).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = socket.send(close(1012, "core_moved")).await;
}
