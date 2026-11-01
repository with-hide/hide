//! The routes a linked node's relay reaches on the core (PRD
//! core-host-node-remote-core): its screens and terminals over its own SSH
//! connection, its Browser View sources and windows, and the files its
//! screens staged on its own machine, each admitted on the link's live
//! relay grant.

use super::*;

/// What a linked node asks for one of its screens' Browser Views.
#[derive(Deserialize)]
pub(crate) struct BrowserSourceQuery {
    pub(crate) device_id: String,
    pub(crate) checkout_path: String,
    pub(crate) id: String,
    pub(crate) load: u64,
}

/// A linked node's screen resolving a Browser View's page (PRD
/// core-host-node-remote-core D-17): the core answers the View's current
/// source, and the node routes it on its own machine (`node_pages`). Taken
/// only on a live relay grant, never through `tailscale serve`.
pub(super) async fn relay_browser_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(query): axum::Json<BrowserSourceQuery>,
) -> Response {
    let Some(admitted) = relay_admitted(&headers, &state).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    // One of the answers the node's screens may wait on (B17, D-20).
    let Some(_slot) = admitted.requests.take(&admitted.node) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"reason":"relay_busy"})),
        )
            .into_response();
    };
    if query.device_id.is_empty()
        || query.device_id.len() > 256
        || !hide_platform::path::is_wire_absolute(&query.checkout_path)
        || query.checkout_path.len() > 8192
        || query.id.is_empty()
        || query.id.len() > 256
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let core = Arc::clone(&state.core);
    let source = tokio::task::spawn_blocking(move || {
        core.browser_route_source(
            &query.device_id,
            &query.checkout_path,
            &query.id,
            query.load,
        )
    })
    .await;
    match source {
        Ok(Ok(source)) => axum::Json(json!({ "source": source })).into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"reason":"core_unavailable"})),
        )
            .into_response(),
    }
}

/// The windows a linked node's daemon has now (PRD core-host-node-remote-core
/// B4, B13): only their owner pids, taken on the node's live relay grant
/// and kept for as long as its link lasts.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NodeWindows {
    owners: Vec<i32>,
}

pub(super) async fn relay_browser_control(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<NodeWindows>,
) -> Response {
    let Some(admitted) = relay_admitted(&headers, &state).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match state
        .browser_control
        .announce(&admitted.node, admitted.link, request.owners)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => browser_control_failure(failure),
    }
}

/// A page action of a linked node's window, which its daemon passes on:
/// the core runs it as that window, as it runs its own window's.
pub(super) async fn relay_browser_control_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<crate::browser_control::BrowserAction>,
) -> Response {
    let Some(admitted) = relay_admitted(&headers, &state).await else {
        return StatusCode::FORBIDDEN.into_response();
    };
    // One of the answers the node's screens may wait on (B17, D-20).
    let Some(slot) = admitted.requests.take(&admitted.node) else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"reason":"relay_busy"})),
        )
            .into_response();
    };
    let node = admitted.node;
    run_browser_action(state, request, move |control, request| {
        let _slot = slot;
        control.node_caller(&node, request.owner_pid, &request.checkout_path)
    })
    .await
}

/// The node and link a relay grant on `headers` belongs to, while the link
/// lives, waiting briefly for a grant just handed out to be bound; never
/// through `tailscale serve`.
async fn relay_admitted(headers: &HeaderMap, state: &AppState) -> Option<crate::attach::Admitted> {
    if via_tailnet(headers) {
        return None;
    }
    let grant = headers.get(RELAY_GRANT_HEADER)?.to_str().ok()?;
    state.relay_grants.admit(grant).await
}

/// A linked node's screen or terminals, through its own SSH connection
/// (`relay`). Only the grant the core handed the node's link admits it,
/// never through `tailscale serve`; a grant that is not live is refused
/// before the upgrade.
pub(super) async fn relay_upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<RelayQuery>,
    State(state): State<AppState>,
) -> Response {
    if via_tailnet(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(grant) = headers
        .get(RELAY_GRANT_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(crate::attach::Admitted {
        node,
        link,
        requests,
    }) = state.relay_grants.admit(grant).await
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match query.mode.as_str() {
        "terminals" => {
            let outputs = Arc::clone(&state.core.outputs);
            let terminals = Arc::clone(&state.core.terminals) as Arc<dyn TerminalNode>;
            ws.on_upgrade(move |socket| {
                crate::relay::serve_terminals(socket, outputs, terminals, node, link)
            })
        }
        "screen" => ws.on_upgrade(move |socket| {
            relay_screen(
                socket,
                state,
                RelayScreen {
                    node,
                    link,
                    requests,
                },
            )
        }),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

/// The header a node's relay carries its grant in.
pub const RELAY_GRANT_HEADER: &str = "x-hide-relay-grant";

#[derive(Deserialize)]
pub(super) struct RelayQuery {
    mode: String,
}

/// A linked node's screen, as its relay was admitted.
pub(super) struct RelayScreen {
    pub(super) node: String,
    pub(super) link: hide_node::ssh::RemoteHost,
    pub(super) requests: Arc<crate::relay::RelayRequests>,
}

impl RelayScreen {
    /// A slot for one answer that runs beside this screen; `None` at the
    /// node's cap.
    pub(super) fn slot(&self) -> Option<crate::relay::RelaySlot> {
        self.requests.take(&self.node)
    }
}

/// What a linked node's screen is told when its node's screens already wait
/// on [`crate::relay::MAX_RELAY_REQUESTS`] answers.
pub(super) fn relay_busy() -> Message {
    Message::Text(
        json!({
            "type": "error",
            "payload": {"reason": "relay_busy"},
            "message": "The core is answering as many requests from this machine as it takes; try again once they are answered",
        })
        .to_string()
        .into(),
    )
}

async fn relay_screen(mut socket: WebSocket, state: AppState, relay: RelayScreen) {
    let handshake = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Handshake>(&text).ok(),
        _ => None,
    };
    let Some(handshake) = handshake else {
        refuse(&mut socket, CloseReason::InvalidToken, None).await;
        return;
    };
    if handshake.schema_version != SCHEMA_VERSION {
        refuse(&mut socket, CloseReason::SchemaMismatch, None).await;
        return;
    }
    let Some(_place) = relay.requests.take_screen(&relay.node) else {
        refuse(&mut socket, CloseReason::ClientLimit, None).await;
        return;
    };
    let connection = state.connections.fetch_add(1, Ordering::SeqCst);
    screen_loop(socket, state, connection, handshake, Some(relay)).await;
}

/// What a screen relay takes before the core: its screen's notice that it
/// sent a pane input this daemon does not see (a node's own pane, typed on
/// its machine), which is never passed to the core, and the files that
/// screen staged on its own machine.
pub(super) const RELAY_TAKES: &[Kind] = &[Kind::TerminalInput, Kind::TerminalAttachment];

/// A file a linked node's screen staged on its own machine for one of that
/// machine's panes (`node_uploads`): the core pastes the node's paths into
/// the pane and the bytes never come here. Answers the error frame the
/// screen gets when it is refused. Only the relay of the node that staged
/// the files may name them, and only for its own panes.
pub(super) fn relay_attachment(
    state: &AppState,
    connection: u64,
    node: &str,
    event: &Value,
    text: &str,
) -> Option<Value> {
    let pane = payload_str(event, "pane_id");
    let own = event.pointer("/payload/staged_on").and_then(Value::as_str) == Some(node)
        && hide_node_link::address::is_pane_of(&pane, node);
    if !own {
        return rejected("terminal_attachment");
    }
    screen_input(state, connection, &pane);
    state
        .core
        .dispatch(text.as_bytes().to_vec())
        .err()
        .map(|error| {
            log_snapshot_failure("attachment", &error);
            attachment_refused(&payload_str(event, "request_id"), "forward_failed")
        })
}
