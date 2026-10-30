//! Owns Herdr subscription setup, reader lifetime, and snapshot fetches.

use super::*;

/// What a successful connect hands the coordinator: the replica, the live
/// subscription, and when the snapshot the replica was built from arrived.
/// Lines read before that instant, plus a grace for scheduling, describe
/// changes the snapshot already holds (`ApplyMode::Reconcile`).
pub(crate) struct Connected {
    pub(crate) replica: SessionReplica,
    pub(crate) subscription: ActiveSubscription,
    pub(crate) snapshot_at: Instant,
}

/// Subscribes first and reads the snapshot second. Herdr's stream cannot be
/// resumed from a position, so the only way not to miss an event between the
/// two is to be listening before the snapshot is taken; the replica then
/// starts from the snapshot and applies what the stream delivers after it.
pub(crate) fn connect(
    context: &SessionSyncContext,
    sender: &Sender<CoordinatorMessage>,
    generation: &mut u64,
    has_projection: bool,
) -> Result<Connected, SessionFetchError> {
    let params = crate::wire::subscription_params(TOPOLOGY_SUBSCRIPTIONS)
        .map_err(SessionFetchError::Malformed)?;
    let subscription = hide_herdr_client::subscribe_with_connector(
        context.api_connector.as_ref(),
        params,
        context.request_timeout(),
    )
    .map_err(|error| match error {
        ApiError::NotRunning(_) => connect_failure_from_api(error, has_projection),
        error => protocol_behind_refusal(context)
            .unwrap_or_else(|| connect_failure_from_api(error, has_projection)),
    })?;
    *generation = generation.saturating_add(1);
    let subscription = spawn_subscription_reader(subscription, *generation, sender.clone())
        .map_err(SessionFetchError::Unreachable)?;
    let replica = match fetch_replica(context) {
        Ok(replica) => replica,
        Err(error) => {
            subscription.stop();
            return Err(error);
        }
    };
    Ok(Connected {
        replica,
        subscription,
        snapshot_at: Instant::now(),
    })
}

/// Why a server refused the subscription, when it is the protocol: a server
/// older than 0.9 refuses the topics this build subscribes to before any
/// snapshot could name its protocol, and every release answers `ping` with
/// it. A ping that fails, or names this build's protocol, leaves the
/// subscription's own failure.
fn protocol_behind_refusal(context: &SessionSyncContext) -> Option<SessionFetchError> {
    let pong = hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "ping",
        crate::wire::ping_params().ok()?,
        context.request_timeout(),
    )
    .ok()
    .and_then(|answer| crate::wire::server_pong(answer).ok())?;
    let protocol = u64::from(pong.protocol);
    (protocol != hide_herdr_client::HERDR_PROTOCOL_REVISION)
        .then(|| crate::wire::protocol_mismatch(protocol, Some(pong.version)))
}

fn spawn_subscription_reader(
    subscription: hide_herdr_client::Subscription,
    generation: u64,
    sender: Sender<CoordinatorMessage>,
) -> Result<ActiveSubscription, String> {
    let (mut reader, shutdown) = subscription.into_parts();
    let worker = thread::Builder::new()
        .name("herdr-core-event-reader".to_owned())
        .spawn(move || {
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = sender.send(CoordinatorMessage::SubscriptionEnded {
                            generation,
                            message: "socket reached EOF".to_owned(),
                        });
                        return;
                    }
                    Ok(_) => {
                        if sender
                            .send(CoordinatorMessage::SubscriptionLine {
                                generation,
                                line,
                                received_at: Instant::now(),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(CoordinatorMessage::SubscriptionEnded {
                            generation,
                            message: format!("socket read failed: {error}"),
                        });
                        return;
                    }
                }
            }
        })
        .map_err(|error| format!("subscription reader could not be started: {error}"))?;
    Ok(ActiveSubscription {
        generation,
        shutdown,
        worker: Some(worker),
    })
}

pub(crate) fn stop_subscription(subscription: &mut Option<ActiveSubscription>) {
    if let Some(active) = subscription.take() {
        active.stop();
    }
}

fn fetch_replica(context: &SessionSyncContext) -> Result<SessionReplica, SessionFetchError> {
    let result = hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "session.snapshot",
        json!({}),
        context.request_timeout(),
    )
    .map_err(session_error_from_api)?;
    SessionReplica::from_decoded(wire::snapshot_response(result)?)
}

pub(crate) fn fetch_agents(
    context: &SessionSyncContext,
) -> Result<Vec<ProjectedAgent>, SessionFetchError> {
    let result = hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "agent.list",
        json!({}),
        context.request_timeout(),
    )
    .map_err(|error| match error {
        // A server refusal is an answered query, not a delayed transport.
        ApiError::Remote { code, message } => {
            SessionFetchError::Stale(format!("agent.list refused ({code}): {message}"))
        }
        error => session_error_from_api(error),
    })?;
    wire::agents_response(result)
}

/// Reads which tab a workspace now holds as active. It is the one read the
/// replica needs after a close removed a workspace's active tab, because
/// Herdr names the replacement in an event only for the workspace that holds
/// its keyboard focus (`SessionReplica::settle_active_tab`).
pub(crate) fn fetch_workspace_active_tab(
    context: &SessionSyncContext,
    workspace_id: &str,
) -> Result<String, SessionFetchError> {
    let result = hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "workspace.get",
        wire::workspace_target_params(workspace_id).map_err(SessionFetchError::Malformed)?,
        context.request_timeout(),
    )
    .map_err(session_error_from_api)?;
    wire::workspace_active_tab(result).map_err(SessionFetchError::Malformed)
}

/// Reads the workspace Herdr has in focus now, the answer a replica waits
/// for once the focused workspace closed and Herdr moved its focus without an
/// event (`SessionReplica::settle_focus`).
pub(crate) fn fetch_focused_workspace(
    context: &SessionSyncContext,
) -> Result<Option<String>, SessionFetchError> {
    let result = hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "workspace.list",
        wire::workspace_list_params().map_err(SessionFetchError::Malformed)?,
        context.request_timeout(),
    )
    .map_err(session_error_from_api)?;
    wire::listed_focused_workspace(result).map_err(SessionFetchError::Malformed)
}

/// Reads the cwd Herdr holds for a pane now, the confirmation a pane
/// announced by `pane_created` waits for (`SessionReplica::confirm_pane_cwd`).
/// A pane closed before the read answers no cwd; its `pane_closed` event
/// follows on the stream.
pub(crate) fn fetch_pane_cwd(
    context: &SessionSyncContext,
    pane_id: &str,
) -> Result<Option<String>, SessionFetchError> {
    let result = match hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "pane.get",
        wire::pane_target_params(pane_id).map_err(SessionFetchError::Malformed)?,
        context.request_timeout(),
    ) {
        Ok(result) => result,
        Err(error) if error.code() == Some("pane_not_found") => return Ok(None),
        Err(error) => return Err(session_error_from_api(error)),
    };
    let (cwd, _) = wire::pane_session(result).map_err(SessionFetchError::Malformed)?;
    Ok(cwd)
}

/// Reads a pane's agent with `agent.get`, which is the read on which Herdr
/// settles a start it still holds open (`launch_settle.rs`); what it settles
/// to reaches the core with the next agent list. A pane closed, or whose
/// agent left, before the read answers nothing to settle.
pub(crate) fn fetch_agent_launch(
    context: &SessionSyncContext,
    pane_id: &str,
) -> Result<(), SessionFetchError> {
    let result = match hide_herdr_client::request_with_connector(
        context.api_connector.as_ref(),
        "agent.get",
        wire::agent_target_params(pane_id).map_err(SessionFetchError::Malformed)?,
        context.request_timeout(),
    ) {
        Ok(result) => result,
        Err(error) if error.code() == Some("agent_not_found") => return Ok(()),
        Err(error) => return Err(session_error_from_api(error)),
    };
    wire::agent_state(result)
        .map(drop)
        .map_err(SessionFetchError::Malformed)
}

fn session_error_from_api(error: ApiError) -> SessionFetchError {
    match error {
        ApiError::NotRunning(message) => SessionFetchError::SocketMissing(message),
        ApiError::Transport(message) | ApiError::Remote { message, .. } => {
            SessionFetchError::Unreachable(message)
        }
        ApiError::Malformed(message) => SessionFetchError::Malformed(message),
    }
}

/// A subscription that failed to open. With a projection already on screen
/// the failure is what makes it stale; without one the server is unreachable.
pub(crate) fn connect_failure_from_api(error: ApiError, has_projection: bool) -> SessionFetchError {
    match error {
        ApiError::NotRunning(message) => SessionFetchError::SocketMissing(message),
        ApiError::Malformed(message) => SessionFetchError::Malformed(message),
        ApiError::Transport(message) | ApiError::Remote { message, .. } if has_projection => {
            SessionFetchError::Stale(message)
        }
        ApiError::Transport(message) | ApiError::Remote { message, .. } => {
            SessionFetchError::Unreachable(message)
        }
    }
}

pub(crate) fn log_sync_failure(
    context: &SessionSyncContext,
    kind: &str,
    replica: Option<&SessionReplica>,
    error: &SessionFetchError,
) {
    crate::diagnostic!(json!({
        "component": "session_sync",
        "kind": kind,
        "target": context.log_target(),
        "state": error.state(),
        "applied_events": replica.map(|current| current.applied_events),
        "message": error.message(),
    }));
}
