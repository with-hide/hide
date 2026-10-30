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
    let events = context
        .engine
        .subscribe(context.request_timeout())
        .map_err(|error| match error {
            EngineError::NotRunning(_) => connect_failure_from_engine(error, has_projection),
            error => protocol_behind_refusal(context)
                .unwrap_or_else(|| connect_failure_from_engine(error, has_projection)),
        })?;
    *generation = generation.saturating_add(1);
    let subscription = spawn_subscription_reader(events, *generation, sender.clone())
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
    let pong = context
        .engine
        .ping(Wait::Each(context.request_timeout()))
        .ok()?;
    crate::engine::herdr::protocol_refusal(pong)
}

fn spawn_subscription_reader(
    events: crate::engine::EventStream,
    generation: u64,
    sender: Sender<CoordinatorMessage>,
) -> Result<ActiveSubscription, String> {
    let crate::engine::EventStream {
        mut reader,
        stopper,
    } = events;
    let worker = thread::Builder::new()
        .name("herdr-core-event-reader".to_owned())
        .spawn(move || {
            loop {
                match reader.next_event() {
                    Ok(None) => {
                        let _ = sender.send(CoordinatorMessage::SubscriptionEnded {
                            generation,
                            message: "socket reached EOF".to_owned(),
                        });
                        return;
                    }
                    Ok(Some(line)) => {
                        if sender
                            .send(CoordinatorMessage::SubscriptionLine {
                                generation,
                                line: Box::new(line),
                                received_at: Instant::now(),
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(message) => {
                        let _ = sender.send(CoordinatorMessage::SubscriptionEnded {
                            generation,
                            message,
                        });
                        return;
                    }
                }
            }
        })
        .map_err(|error| format!("subscription reader could not be started: {error}"))?;
    Ok(ActiveSubscription {
        generation,
        stopper,
        worker: Some(worker),
    })
}

pub(crate) fn stop_subscription(subscription: &mut Option<ActiveSubscription>) {
    if let Some(active) = subscription.take() {
        active.stop();
    }
}

fn fetch_replica(context: &SessionSyncContext) -> Result<SessionReplica, SessionFetchError> {
    let state = context
        .engine
        .session(Wait::Each(context.request_timeout()))
        .map_err(session_error_from_engine)?;
    SessionReplica::from_decoded(state)
}

pub(crate) fn fetch_agents(
    context: &SessionSyncContext,
) -> Result<Vec<ProjectedAgent>, SessionFetchError> {
    context
        .engine
        .agents(Wait::Each(context.request_timeout()))
        .map_err(|error| match error {
            // A server refusal is an answered query, not a delayed transport.
            EngineError::Refused { code, message } => {
                SessionFetchError::Stale(format!("agent.list refused ({code}): {message}"))
            }
            error => session_error_from_engine(error),
        })
}

/// Reads which tab a workspace now holds as active. It is the one read the
/// replica needs after a close removed a workspace's active tab, because
/// Herdr names the replacement in an event only for the workspace that holds
/// its keyboard focus (`SessionReplica::settle_active_tab`).
pub(crate) fn fetch_workspace_active_tab(
    context: &SessionSyncContext,
    workspace_id: &str,
) -> Result<String, SessionFetchError> {
    context
        .engine
        .workspace_active_tab(Wait::Each(context.request_timeout()), workspace_id)
        .map_err(session_error_from_engine)
}

/// Reads the workspace Herdr has in focus now, the answer a replica waits
/// for once the focused workspace closed and Herdr moved its focus without an
/// event (`SessionReplica::settle_focus`).
pub(crate) fn fetch_focused_workspace(
    context: &SessionSyncContext,
) -> Result<Option<String>, SessionFetchError> {
    context
        .engine
        .focused_workspace(Wait::Each(context.request_timeout()))
        .map_err(session_error_from_engine)
}

/// Reads the cwd Herdr holds for a pane now, the confirmation a pane
/// announced by `pane_created` waits for (`SessionReplica::confirm_pane_cwd`).
/// A pane closed before the read answers no cwd; its `pane_closed` event
/// follows on the stream.
pub(crate) fn fetch_pane_cwd(
    context: &SessionSyncContext,
    pane_id: &str,
) -> Result<Option<String>, SessionFetchError> {
    match context
        .engine
        .pane_session(Wait::Each(context.request_timeout()), pane_id)
    {
        Ok((cwd, _)) => Ok(cwd),
        Err(error) if error.code() == Some("pane_not_found") => Ok(None),
        Err(error) => Err(session_error_from_engine(error)),
    }
}

/// Reads a pane's agent with `agent.get`, which is the read on which Herdr
/// settles a start it still holds open (`launch_settle.rs`); what it settles
/// to reaches the core with the next agent list. A pane closed, or whose
/// agent left, before the read answers nothing to settle.
pub(crate) fn fetch_agent_launch(
    context: &SessionSyncContext,
    pane_id: &str,
) -> Result<(), SessionFetchError> {
    match context
        .engine
        .agent_state(Wait::Each(context.request_timeout()), pane_id)
    {
        Ok(_) => Ok(()),
        Err(error) if error.code() == Some("agent_not_found") => Ok(()),
        Err(error) => Err(session_error_from_engine(error)),
    }
}

/// A read's failure as the session sync reports it: no server, a server it
/// could not reach or that refused, or an answer it could not read.
fn session_error_from_engine(error: EngineError) -> SessionFetchError {
    match error {
        EngineError::NotRunning(message) => SessionFetchError::SocketMissing(message),
        EngineError::Transport(message)
        | EngineError::Unanswered(message)
        | EngineError::Refused { message, .. } => SessionFetchError::Unreachable(message),
        EngineError::Protocol {
            message,
            expected_protocol,
            received_protocol,
            received_version,
        } => SessionFetchError::Protocol {
            message,
            expected_protocol,
            received_protocol,
            received_version,
        },
        EngineError::Malformed(message)
        | EngineError::Request(message)
        | EngineError::Answer(message) => SessionFetchError::Malformed(message),
    }
}

/// A subscription that failed to open. With a projection already on screen
/// the failure is what makes it stale; without one the server is unreachable.
pub(crate) fn connect_failure_from_engine(
    error: EngineError,
    has_projection: bool,
) -> SessionFetchError {
    match error {
        EngineError::NotRunning(message) => SessionFetchError::SocketMissing(message),
        EngineError::Transport(message)
        | EngineError::Unanswered(message)
        | EngineError::Refused { message, .. }
            if has_projection =>
        {
            SessionFetchError::Stale(message)
        }
        EngineError::Transport(message)
        | EngineError::Unanswered(message)
        | EngineError::Refused { message, .. } => SessionFetchError::Unreachable(message),
        other => session_error_from_engine(other),
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
