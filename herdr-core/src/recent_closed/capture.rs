//! Capturing what a close ends before it ends it, and running the close:
//! the panes and agents a closed item keeps, the shell that replaces a closed
//! tab's last pane, and the check that nothing still running is closed.

use std::thread;
use std::time::{Duration, Instant};

use crate::engine::{Engine, EngineError, Wait};

use crate::live::{CONTROL_WAIT, LiveContext, failed, fetch_session};
use crate::recent_closed::reopen::{
    ReopenIntentStage, export_reopen_layout, pane_ids_with_reopen_marker, reopen_intent_env,
    reopen_intent_marker,
};
use crate::recent_closed::{ClosedContext, ClosedItem, ClosedPane, PanePlacement};
use crate::sidebar::SessionSnapshotPayload;

#[derive(Clone, Debug)]
pub enum CloseCaptureTarget {
    Pane { pane_id: String },
    Tab { tab_id: String },
}

#[derive(Clone, Debug)]
pub struct CloseCaptureRequest {
    pub failed_terminals: Vec<(String, String)>,
    /// A sleep close must never acquire the independent ordinary undo route.
    pub retain_for_reopen: bool,
    pub key: String,
    /// A response from an older live connection must never settle a newer
    /// close intent after reconnect.
    pub connection_generation: u64,
    pub context: ClosedContext,
    pub panes: Vec<ClosedPane>,
    pub target: CloseCaptureTarget,
}

#[derive(Debug)]
pub struct CloseCaptureOutcome {
    pub item: Option<ClosedItem>,
}

/// The work included in the operator's existing close confirmation.
#[derive(Clone, Debug)]
pub struct ClosePeer {
    pub pane_id: String,
    pub terminal_id: String,
    pub session: Option<crate::sidebar::SessionAgentSessionPayload>,
}

/// Fresh terminal-close authorization and the admission's shared deadline.
#[derive(Clone, Debug)]
pub struct CloseSafety {
    /// Only the working terminals included in the admitted confirmation.
    pub approved_peers: Vec<ClosePeer>,
    pub pane_ids: Vec<String>,
    pub deadline: Instant,
}

#[derive(Clone, Debug)]
pub struct CloseEffectRequest {
    pub safety: Option<CloseSafety>,
    pub failed_terminals: Vec<(String, String)>,
    pub retain_for_reopen: bool,
    pub allow_replacement_create: bool,
    pub replacement: Option<ClosedContext>,
    pub key: String,
    pub connection_generation: u64,
    pub target: CloseCaptureTarget,
}

pub fn spawn_close_capture(
    context: LiveContext,
    request: CloseCaptureRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-close-capture".to_owned())
        .spawn(move || {
            let result = capture_close_item(context.engine.as_ref(), &request);
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let (changed, effects) = match runtime.lock() {
                Ok(mut guard) => guard.ingest_close_capture_result(&request, result),
                Err(_) => return,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
            run_close_effects(&context, context.engine.as_ref(), effects);
        })
        .map(|_| ())
        .map_err(|error| format!("close capture worker could not be started: {error}"))
}

/// Continues the same close operation after a durable sleeping-session
/// receipt. It neither captures another item nor sends an extra close.
pub(crate) fn spawn_saved_close_effect(
    context: LiveContext,
    effect: CloseEffectRequest,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-saved-close".into())
        .spawn(move || {
            let gate = crate::agent_sleep_herdr::dormant_close_gate(&context, &effect);
            let engine = context.engine.gated(gate);
            run_close_effects(&context, engine.as_ref(), vec![effect]);
        })
        .map(|_| ())
        .map_err(|_| "The saved close worker could not start".into())
}

fn run_close_effects(context: &LiveContext, engine: &dyn Engine, effects: Vec<CloseEffectRequest>) {
    for effect in effects {
        let prepared = if let Some(replacement) = &effect.replacement {
            prepare_close_replacement(
                engine,
                &effect.key,
                replacement,
                &effect.target,
                effect.allow_replacement_create,
            )
            .map_err(|failure| {
                if let Some(shell) = &failure.shell_in_tab
                    && let Some(runtime) = context.runtime.upgrade()
                    && let Ok(mut guard) = runtime.lock()
                {
                    guard.note_shell_in_tab(&effect.key, shell);
                }
                failure.message
            })
            .and_then(|(shell, payload)| {
                let runtime = context.runtime.upgrade().ok_or("runtime stopped")?;
                let mut guard = runtime.lock().map_err(|_| "runtime unavailable")?;
                guard.ingest_close_replacement(&effect, &shell, payload)?;
                context.notifier.notify();
                Ok(())
            })
            .map_err(|message| EngineError::Refused {
                code: "replacement_failed".into(),
                message,
            })
        } else {
            Ok(())
        };
        let result = prepared.and_then(|()| run_close_effect(engine, &effect));
        let Some(runtime) = context.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => guard.ingest_close_effect_result(&effect, result),
            Err(_) => return,
        };
        drop(runtime);
        if changed {
            context.notifier.notify();
        }
    }
}

fn capture_close_item(
    engine: &dyn Engine,
    request: &CloseCaptureRequest,
) -> Result<CloseCaptureOutcome, String> {
    // A terminal Herdr could not recreate has no live layout to export.
    // Keep the close intent, but do not promise an undo that cannot restore it.
    let targets = match &request.target {
        CloseCaptureTarget::Pane { pane_id } => vec![pane_id.as_str()],
        CloseCaptureTarget::Tab { .. } => request
            .panes
            .iter()
            .map(|pane| pane.pane_id.as_str())
            .collect(),
    };
    if targets
        .iter()
        .any(|id| request.failed_terminals.iter().any(|(pane, _)| pane == id))
    {
        return Ok(CloseCaptureOutcome { item: None });
    }
    let layout = engine
        .export_layout(CONTROL_WAIT, &request.context.tab_id)
        .map_err(failed("layout.export"))?;
    let item = match &request.target {
        CloseCaptureTarget::Pane { pane_id } => {
            let pane = request
                .panes
                .iter()
                .find(|pane| pane.pane_id == *pane_id)
                .cloned()
                .ok_or_else(|| format!("pane {pane_id} disappeared before close capture"))?;
            let placement = layout.root.placement_for(pane_id).unwrap_or(PanePlacement {
                neighbor_pane_id: None,
                parent_path: Vec::new(),
                direction: crate::recent_closed::ClosedSplitDirection::Right,
                ratio: 0.5,
                target_was_first: false,
            });
            Some(ClosedItem::Pane {
                key: request.key.clone(),
                context: request.context.clone(),
                pane,
                placement,
            })
        }
        CloseCaptureTarget::Tab { .. } => Some(ClosedItem::Tab {
            key: request.key.clone(),
            context: request.context.clone(),
            layout,
            panes: request.panes.clone(),
        }),
    };
    Ok(CloseCaptureOutcome { item })
}

/// Why a close's shell could not be handed over. A shell already standing in
/// the closing tab is named: it is this close's own, so the runtime keeps it
/// among the tab's panes and a retry closes beside it.
#[derive(Debug)]
pub(crate) struct ReplacementFailure {
    pub(crate) message: String,
    pub(crate) shell_in_tab: Option<String>,
}

impl From<String> for ReplacementFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            shell_in_tab: None,
        }
    }
}

impl From<&str> for ReplacementFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

/// Makes, or on a retry finds by its marker, the shell a close needs first
/// in the closing tab, beside the pane `target` closes, and answers it with a
/// fresh session that holds it.
fn prepare_close_replacement(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    target: &CloseCaptureTarget,
    allow_create: bool,
) -> Result<(String, SessionSnapshotPayload), ReplacementFailure> {
    let CloseCaptureTarget::Pane { pane_id } = target else {
        return Err("a shell inside the tab needs the pane it stands beside".into());
    };
    prepare_shell_in_tab(engine, key, context, pane_id, allow_create)
}

/// The shell a named tab keeps when its last agent pane closes: split from
/// that pane at the checkout, unfocused, and stamped with the close's marker,
/// which is the only proof a retry adopts it by.
fn prepare_shell_in_tab(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    closing_pane: &str,
    allow_create: bool,
) -> Result<(String, SessionSnapshotPayload), ReplacementFailure> {
    let pane_id = match marked_shells_in_tab(engine, key, &context.tab_id)?.as_slice() {
        [] if !allow_create => {
            return Err("The replacement shell disappeared; retry to make another".into());
        }
        [] => split_shell_in_tab(engine, key, context, closing_pane)?,
        [id] => id.clone(),
        _ => return Err("multiple shells carry the same close intent".into()),
    };
    let held = |message: String| ReplacementFailure {
        message,
        shell_in_tab: Some(pane_id.clone()),
    };
    let mut snapshot = fetch_session(engine).map_err(|error| held(error.message().to_owned()))?;
    if !snapshot.layouts.iter().any(|layout| {
        layout.tab_id == context.tab_id && layout.panes.iter().any(|pane| pane.pane_id == pane_id)
    }) {
        return Err(held("replacement shell is not confirmed in its tab".into()));
    }
    // Its folder is the one it was made at.
    if let Some(pane) = snapshot
        .panes
        .iter_mut()
        .find(|pane| pane.pane_id == pane_id)
    {
        pane.cwd = Some(context.checkout_path.clone());
    }
    Ok((pane_id, snapshot))
}

fn marked_shells_in_tab(
    engine: &dyn Engine,
    key: &str,
    tab_id: &str,
) -> Result<Vec<String>, String> {
    let layout = export_reopen_layout(engine, tab_id)?;
    let marker = reopen_intent_marker(key, ReopenIntentStage::CloseReplacement);
    let mut owned = Vec::new();
    pane_ids_with_reopen_marker(&layout.root, &marker, &mut owned);
    Ok(owned)
}

/// A lost or malformed answer can follow a split Herdr made, so a failed
/// split reads the tab once more for the marked pane a retry would adopt.
fn split_shell_in_tab(
    engine: &dyn Engine,
    key: &str,
    context: &ClosedContext,
    closing_pane: &str,
) -> Result<String, ReplacementFailure> {
    engine
        .split_replacement(
            CONTROL_WAIT,
            closing_pane,
            &context.checkout_path,
            reopen_intent_env(key, ReopenIntentStage::CloseReplacement),
        )
        .map_err(|error| match error {
            // Nothing was sent, so no split can be standing in the tab.
            EngineError::Request(message) => ReplacementFailure::from(message),
            error => ReplacementFailure {
                message: error.within("pane.split failed"),
                shell_in_tab: marked_shells_in_tab(engine, key, &context.tab_id)
                    .ok()
                    .and_then(|owned| match owned.as_slice() {
                        [id] => Some(id.clone()),
                        _ => None,
                    }),
            },
        })
}

pub(crate) fn run_close_effect(
    engine: &dyn Engine,
    request: &CloseEffectRequest,
) -> Result<(), EngineError> {
    run_close_effect_with_clock(engine, request, &Instant::now)
}

fn run_close_effect_with_clock(
    engine: &dyn Engine,
    request: &CloseEffectRequest,
    now: &dyn Fn() -> Instant,
) -> Result<(), EngineError> {
    check_failed_terminals(
        engine,
        &request.failed_terminals,
        &request.target,
        request.safety.as_ref(),
        now,
    )?;
    let wait = Wait::Each(close_remaining(request.safety.as_ref(), now())?);
    match &request.target {
        CloseCaptureTarget::Pane { pane_id } => engine.close_pane(wait, pane_id),
        CloseCaptureTarget::Tab { tab_id } => engine.close_tab(wait, tab_id),
    }
}

/// One fresh read per close, on its existing worker, with no runtime lock held.
/// Refuse the old authorization if any failed terminal was replaced or recovered.
fn check_failed_terminals(
    engine: &dyn Engine,
    expected: &[(String, String)],
    target: &CloseCaptureTarget,
    safety: Option<&CloseSafety>,
    now: &dyn Fn() -> Instant,
) -> Result<(), EngineError> {
    if expected.is_empty() {
        return Ok(());
    }
    let snapshot = engine.session(Wait::Each(close_remaining(safety, now())?))?;
    let scope_valid = match target {
        CloseCaptureTarget::Pane { pane_id } => expected.len() == 1 && expected[0].0 == *pane_id,
        CloseCaptureTarget::Tab { tab_id } => safety.is_some_and(|safety| {
            let mut current = snapshot
                .panes
                .iter()
                .filter(|pane| &pane.tab_id == tab_id)
                .map(|pane| pane.pane_id.clone())
                .collect::<Vec<_>>();
            current.sort();
            let mut approved = safety.pane_ids.clone();
            approved.sort();
            current == approved
                && expected.iter().all(|(id, _)| current.contains(id))
                && snapshot
                    .panes
                    .iter()
                    .filter(|pane| &pane.tab_id == tab_id)
                    .filter(|pane| !expected.iter().any(|(id, _)| id == &pane.pane_id))
                    .all(|pane| {
                        let approved = safety.approved_peers.iter().any(|peer| {
                            peer.pane_id == pane.pane_id
                                && peer.terminal_id == pane.terminal_id
                                && peer.session.as_ref()
                                    == snapshot
                                        .agents
                                        .iter()
                                        .find(|agent| agent.pane_id == pane.pane_id)
                                        .and_then(|agent| agent.agent_session.as_ref())
                        });
                        let allowed = |status: &str| {
                            matches!(status, "idle" | "done")
                                || (approved && matches!(status, "working" | "blocked"))
                        };
                        allowed(&pane.agent_status)
                            && snapshot
                                .agents
                                .iter()
                                .filter(|agent| agent.pane_id == pane.pane_id)
                                .all(|agent| agent.agent_status.as_deref().is_some_and(allowed))
                    })
        }),
    };
    if scope_valid
        && expected.iter().all(|(id, terminal)| {
            snapshot.panes.iter().any(|pane| {
                &pane.pane_id == id && &pane.terminal_id == terminal && pane.restore_error.is_some()
            })
        })
    {
        Ok(())
    } else {
        Err(EngineError::Refused {
            code: "close_target_changed".into(),
            message:
                "The terminal changed before closing. Check its current state and close it again."
                    .into(),
        })
    }
}

fn close_remaining(safety: Option<&CloseSafety>, now: Instant) -> Result<Duration, EngineError> {
    match safety {
        None => Ok(Duration::from_secs(5)),
        Some(safety) => safety.deadline.checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| EngineError::Refused {
                code: "close_deadline_expired".into(),
                message: "The close expired before it was sent. Check its current state and close it again.".into(),
            }),
    }
}

/// A read-only fresh session check for a close whose transport result was
/// ambiguous. It does not repeat the mutation and carries the connection
/// generation so a late answer from an older socket cannot change current
/// state.
#[derive(Clone, Debug)]
pub struct CloseStatusCheckRequest {
    pub key: String,
    pub target: CloseCaptureTarget,
}

/// Runs one read for all close operations waiting on the same authoritative
/// session. The result is fanned out while the runtime lock is held, so a
/// single fresh snapshot cannot settle one close and leave its siblings stale.
pub fn spawn_close_status_checks(
    context: LiveContext,
    connection_generation: u64,
    requests: Vec<CloseStatusCheckRequest>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-close-status-check".to_owned())
        .spawn(move || {
            let result = fetch_session(context.engine.as_ref());
            let Some(runtime) = context.runtime.upgrade() else {
                return;
            };
            let changed = match runtime.lock() {
                Ok(mut guard) => {
                    guard.ingest_close_status_results(connection_generation, &requests, result)
                }
                Err(_) => false,
            };
            drop(runtime);
            if changed {
                context.notifier.notify();
            }
        })
        .map(|_| ())
        .map_err(|error| format!("close status worker could not be started: {error}"))
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
