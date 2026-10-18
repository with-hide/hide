//! Core-owned records for every Herdr-owned mutation still in flight.
//!
//! A split, zoom, resize, tab move, remote mutation or close is one record
//! from the moment the operator asks until fresh topology confirms it, with
//! a host connection generation, target and conflict-scope ids, phase, stage,
//! start time and absolute deadline. Transport success is never topology
//! truth, and an expired or ambiguous mutation becomes an explicit unknown
//! that is never resent; `docs/ARCHITECTURE.md` owns the reasons. The close
//! lifecycle itself lives beside reopen in `editor.rs`.

use super::*;

/// One user close from intent through authoritative topology confirmation.
/// The captured item is held here as a reservation until the request reaches
/// the front of `close_capture_order`; it never competes with the confirmed
/// twenty-entry undo stack while its result is unknown.
#[derive(Clone, Debug)]
pub(super) struct PendingClose {
    pub(super) replacement_effect_started: bool,
    /// The shell this close made before closing: its tab, or its pane when
    /// the shell went into the closing tab (`replacement_in_tab`).
    pub(super) replacement_id: Option<String>,
    pub(super) allow_replacement_create: bool,
    pub(super) request: live::CloseCaptureRequest,
    pub(super) target_id: String,
    pub(super) scope_id: String,
    /// The full pane set observed when the user approved this close. A later
    /// pane addition or removal changes the target range, so the old approval
    /// cannot be applied to it.
    pub(super) scope_pane_ids: Vec<String>,
    pub(super) pane_ids: Vec<String>,
    /// Agent panes that required the approval when this close was requested.
    /// A newly protected pane invalidates the approval even when the tab's
    /// pane set itself is unchanged.
    pub(super) protected_agent_pane_ids: Vec<String>,
    pub(super) approved_peer_terminals: Vec<live::ClosePeer>,
    pub(super) label: String,
    pub(super) item: Option<ClosedItem>,
    pub(super) phase: String,
    pub(super) stage: String,
    pub(super) started_at_unix_ms: u64,
    pub(super) deadline_at_unix_ms: Option<u64>,
    pub(super) connection_generation: u64,
    pub(super) message: Option<String>,
    pub(super) retryable: bool,
    pub(super) selection_restore: Option<CloseSelectionRestore>,
}

impl PendingClose {
    /// Still on its way to a confirmed close: not refused, failed or unknown.
    pub(super) fn settling(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "preparing" | "transmitting" | "awaiting_topology" | "completed"
        )
    }

    /// The tab this close takes out of the Agent areas while it runs: the
    /// whole tab, from approval until a refusal, failure or unknown result
    /// puts it back. A close that must first open a replacement shell leaves
    /// its tab in place, because the area would otherwise stand empty until
    /// that shell exists.
    pub(super) fn leaving_tab(&self) -> Option<&str> {
        let whole_tab = match &self.request.target {
            live::CloseCaptureTarget::Tab { .. } => true,
            live::CloseCaptureTarget::Pane { pane_id } => {
                matches!(self.scope_pane_ids.as_slice(), [only] if only == pane_id)
            }
        };
        (whole_tab && self.settling() && !self.request.context.replacement_shell)
            .then_some(self.scope_id.as_str())
    }
}

#[derive(Clone, Debug)]
pub(super) struct PendingPaneOperation {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) target_id: String,
    pub(super) scope_id: String,
    pub(super) connection_generation: u64,
    pub(super) phase: String,
    pub(super) stage: String,
    pub(super) started_at_unix_ms: u64,
    pub(super) deadline_at_unix_ms: Option<u64>,
    pub(super) message: Option<String>,
    pub(super) retryable: bool,
    pub(super) created_pane_id: Option<String>,
    pub(super) baseline_signature: PaneTopologySignature,
    /// What this record asks Herdr for. It waits in its tab's line in phase
    /// `queued` and is sent when every operation ahead of it has answered
    /// (PRD instant-pane-topology D-10).
    pub(super) request: GeometryRequest,
    /// Its place in its tab's line.
    pub(super) sequence: u64,
    /// What the canvas draws for it ahead of Herdr (D-07): set at the request
    /// for a close, zoom or resize and at the answer for a split, and cleared
    /// when it settles, fails, or its result becomes unknown.
    pub(super) prediction: Option<super::pane_prediction::Prediction>,
}

/// How many operations may wait in one tab's line behind the one Herdr is
/// answering; the next is refused and logged (D-10, engineering 15).
pub(super) const GEOMETRY_QUEUE_LIMIT: usize = 8;

#[derive(Clone, Debug)]
pub(super) enum GeometryRequest {
    /// A split, zoom or resize, sent as this action.
    Pane(PaneControlAction),
    /// The operator's close of one pane, started through the ordinary close
    /// (`close_local_pane`) when its turn comes; from then on the close keeps
    /// its own record and this one is gone.
    Close { pane_id: String, confirmed: bool },
}

#[derive(Clone, Debug)]
pub(super) struct PendingRemoteOperation {
    pub(super) id: String,
    pub(super) target_id: String,
    pub(super) scope_id: String,
    pub(super) connection_generation: u64,
    pub(super) kind: String,
    pub(super) phase: String,
    pub(super) stage: String,
    pub(super) started_at_unix_ms: u64,
    pub(super) deadline_at_unix_ms: Option<u64>,
    pub(super) message: Option<String>,
    pub(super) retryable: bool,
    pub(super) created_pane_id: Option<String>,
    pub(super) baseline_zoomed: Option<bool>,
}

pub(super) type PaneRect = (String, u16, u16, u16, u16);
pub(super) type SplitRect = (u8, u32, u16, u16, u16, u16);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct PaneTopologySignature {
    pub(super) zoomed: bool,
    pub(super) focused_pane_id: String,
    pub(super) pane_ids: Vec<String>,
    pub(super) splits: Vec<(u8, u32)>,
    /// Raw geometry is present only on a signature taken from Herdr's fresh
    /// session payload. Model projections intentionally do not invent it.
    pub(super) pane_rects: Option<Vec<PaneRect>>,
    pub(super) split_rects: Option<Vec<SplitRect>>,
}

pub(super) struct TabMoveResultContext<'a> {
    pub(super) checkout_id: &'a str,
    pub(super) tab_id: &'a str,
    pub(super) expected_order: &'a [String],
    pub(super) generation: u64,
    pub(super) connection_generation: u64,
    pub(super) elapsed_ms: u128,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct CloseSelectionState {
    pub(super) focused_checkout_id: Option<String>,
    pub(super) terminal_pane_id: Option<String>,
    pub(super) focused_pane_id: Option<String>,
    pub(super) selected_pane_id: Option<String>,
    pub(super) active_tab_id: Option<String>,
    pub(super) visible_tab_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CloseSelectionRestore {
    pub(super) checkout_id: Option<String>,
    pub(super) before: CloseSelectionState,
    pub(super) after: CloseSelectionState,
}

pub(super) fn close_target_id(target: &live::CloseCaptureTarget) -> &str {
    match target {
        live::CloseCaptureTarget::Pane { pane_id } => pane_id,
        live::CloseCaptureTarget::Tab { tab_id } => tab_id,
    }
}

pub(super) fn remote_pane_id(target_id: &str, pane_id: &str) -> String {
    format!("remote:{target_id}:pane:{pane_id}")
}

/// Copies each pane's status word and close-confirmation answer from the agent
/// rows that carry the read axis.
///

#[derive(Clone, Debug)]
pub(super) struct RemoteMutationDescriptor {
    pub(super) kind: String,
    pub(super) target_id: String,
    pub(super) scope_id: String,
    pub(super) baseline_zoomed: Option<bool>,
}

/// A device's [`Runtime::pane_alone_unzoomed`]: whether `pane_id` is the only
/// pane of an unzoomed tab in that device's session.
pub(super) fn remote_pane_alone_unzoomed(session: &RemoteSessionSnapshot, pane_id: &str) -> bool {
    let Some(tab) = session
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
    else {
        return false;
    };
    tab.panes.len() == 1
        && !session
            .pane_layouts
            .iter()
            .any(|layout| tab.id.as_deref() == Some(layout.tab_id.as_str()) && layout.zoomed)
}

pub(super) fn remote_mutation_descriptor(
    session: &RemoteSessionSnapshot,
    request: &RemoteControlRequest,
) -> Option<RemoteMutationDescriptor> {
    let kind = request.mutation_kind()?.to_owned();
    match request {
        RemoteControlRequest::SplitPane { pane_id, .. }
        | RemoteControlRequest::TogglePaneZoom { pane_id, .. }
        | RemoteControlRequest::ClosePane { pane_id, .. } => {
            let tab = session
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .flat_map(|checkout| checkout.tabs.iter())
                .find(|tab| tab.panes.iter().any(|pane| pane.id == *pane_id))?;
            let scope_id = tab.id.clone()?;
            let baseline_zoomed = (kind == "pane.zoom").then(|| {
                session
                    .pane_layouts
                    .iter()
                    .find(|layout| layout.tab_id == scope_id)
                    .is_some_and(|layout| layout.zoomed)
            });
            Some(RemoteMutationDescriptor {
                kind,
                target_id: pane_id.clone(),
                scope_id,
                baseline_zoomed,
            })
        }
        RemoteControlRequest::CloseTab { tab_id, .. } => {
            let _tab = session
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .flat_map(|checkout| checkout.tabs.iter())
                .find(|tab| tab.id.as_deref() == Some(tab_id.as_str()))?;
            Some(RemoteMutationDescriptor {
                kind,
                target_id: tab_id.clone(),
                scope_id: tab_id.clone(),
                baseline_zoomed: None,
            })
        }
        RemoteControlRequest::FocusPane { .. }
        | RemoteControlRequest::FocusWorkspace { .. }
        | RemoteControlRequest::FocusTab { .. }
        | RemoteControlRequest::CreateTab { .. } => None,
    }
}

/// Which report settled a remote operation, so the diagnostic tells the two
/// orderings apart.
#[derive(Clone, Copy)]
enum RemoteSettle {
    /// The device's own report, read when it arrived.
    FreshSession,
    /// A report that arrived earlier, read when the operation began to wait.
    HeldSession,
}

impl RemoteSettle {
    fn source(self) -> &'static str {
        match self {
            Self::FreshSession => "fresh topology",
            Self::HeldSession => "the session received before it waited",
        }
    }
}

/// Whether `session`, a device's session as its Herdr last reported it, shows
/// the effect of `operation`.
fn remote_operation_confirmed_by(
    operation: &PendingRemoteOperation,
    session: &RemoteSessionSnapshot,
) -> bool {
    let tabs = || {
        session
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
    };
    match operation.kind.as_str() {
        "pane.close" => !tabs()
            .flat_map(|tab| tab.panes.iter())
            .any(|pane| pane.id == operation.target_id),
        "tab.close" => !tabs().any(|tab| tab.id.as_deref() == Some(operation.target_id.as_str())),
        "pane.split" => operation.created_pane_id.as_ref().is_some_and(|created| {
            tabs().any(|tab| {
                tab.id.as_deref() == Some(operation.scope_id.as_str())
                    && tab.panes.iter().any(|pane| pane.id == *created)
            })
        }),
        "pane.zoom" => operation.baseline_zoomed.is_some_and(|baseline| {
            session
                .pane_layouts
                .iter()
                .find(|layout| layout.tab_id == operation.scope_id)
                .is_some_and(|layout| layout.zoomed != baseline)
        }),
        _ => false,
    }
}

impl Runtime {
    pub(crate) fn request_status_refresh(&mut self) -> bool {
        self.status_refresh_requested = true;
        self.push_diagnostic(
            "session.status_refresh_requested",
            "A read-only session refresh was requested by the shell",
        );
        true
    }

    pub(crate) fn take_status_refresh_request(&mut self) -> bool {
        let requested = self.status_refresh_requested;
        self.status_refresh_requested = false;
        requested
    }

    /// Advances every bounded asynchronous operation from a coordinator timer.
    /// Snapshot publication is an output of this work, never its clock: an
    /// idle Herdr session must still turn an expired acknowledgement into an
    /// explicit unknown result at its deadline.
    pub(crate) fn tick_async_operations(&mut self, now_unix_ms: u64) -> bool {
        self.with_close_geometry(|runtime| runtime.tick_operations(now_unix_ms))
    }

    fn tick_operations(&mut self, now_unix_ms: u64) -> bool {
        self.op_timings.expire(std::time::Instant::now());
        let mut changed = false;
        changed |= self.expire_pending_view_focus(now_unix_ms);
        changed |= self.expire_tab_moves(now_unix_ms);
        changed |= self.expire_close_operations(now_unix_ms);
        changed |= self.expire_pane_operations(now_unix_ms);
        changed |= self.expire_provisional_tabs(now_unix_ms);
        changed |= self.expire_remote_operations(now_unix_ms);
        changed |= self.expire_pending_choice(now_unix_ms);
        changed |= self.advance_tree_closes();
        changed |= self.expire_attachment_wait(now_unix_ms);
        changed |= self.reconcile_attachment_target();
        changed |= self.tick_attachment();
        changed |= self.tick_project_memory(now_unix_ms);
        changed |= self.tick_view_feedback(now_unix_ms);
        self.tick_memory_promotions(now_unix_ms);
        changed |= self.tick_device_hosts(now_unix_ms);
        changed |= self.tick_session_window(now_unix_ms);
        changed |= self.tick_work_clock(now_unix_ms);
        // Agents fall asleep and closes end on paths of their own; their
        // nodes hear of it here at the latest.
        changed |= self.sync_terminal_intents();
        // A close ahead in a tab's line ends on paths of its own; its turn
        // passes on here at the latest.
        changed |= self.pump_geometry_queues();
        if changed {
            self.sync_recent_closed_snapshot();
            self.sync_async_operations();
        }
        changed
    }

    pub(super) fn insert_remote_operation(
        &mut self,
        key: &(String, String),
        descriptor: RemoteMutationDescriptor,
        started_at_unix_ms: u64,
        connection_generation: u64,
    ) {
        if self.remote_operations.len() >= 128 {
            let oldest = self
                .remote_operations
                .iter()
                .min_by_key(|(_, operation)| operation.started_at_unix_ms)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.remote_operations.remove(&oldest);
            }
        }
        self.remote_operations.insert(
            key.clone(),
            PendingRemoteOperation {
                id: format!("remote.control:{}:{}", key.0, key.1),
                target_id: descriptor.target_id,
                scope_id: descriptor.scope_id,
                connection_generation,
                kind: descriptor.kind,
                phase: "transmitting".to_owned(),
                stage: "request".to_owned(),
                started_at_unix_ms,
                deadline_at_unix_ms: Some(
                    started_at_unix_ms.saturating_add(CLOSE_STAGE_TIMEOUT_MS),
                ),
                message: None,
                retryable: false,
                created_pane_id: None,
                baseline_zoomed: descriptor.baseline_zoomed,
            },
        );
        self.sync_async_operations();
    }

    pub(super) fn fail_remote_operation(
        &mut self,
        target_id: &str,
        request_id: &str,
        message: String,
    ) {
        let key = (target_id.to_owned(), request_id.to_owned());
        let Some(operation) = self.remote_operations.get(&key).cloned() else {
            return;
        };
        if let Some(current) = self.remote_operations.get_mut(&key) {
            current.phase = "failed".to_owned();
            current.stage = "request".to_owned();
            current.deadline_at_unix_ms = None;
            current.message = Some(message.clone());
            current.retryable = true;
        }
        self.push_diagnostic(
            "remote.control.failed",
            format!(
                "{} {} (request {}): {message}",
                operation.kind, operation.target_id, request_id
            ),
        );
        self.set_error(
            "remote.control.failed",
            format!("{} for {target_id} failed: {message}", operation.kind),
            true,
        );
        self.sync_async_operations();
    }

    pub(super) fn acknowledge_remote_operation(
        &mut self,
        target_id: &str,
        request_id: &str,
        created_pane_id: Option<String>,
    ) -> bool {
        let key = (target_id.to_owned(), request_id.to_owned());
        let Some(operation) = self.remote_operations.get(&key).cloned() else {
            return false;
        };
        if operation.kind == "pane.split" && created_pane_id.is_none() {
            self.fail_remote_operation(
                target_id,
                request_id,
                "Remote Herdr acknowledged split without a created pane id".to_owned(),
            );
            return true;
        }
        if let Some(current) = self.remote_operations.get_mut(&key) {
            current.phase = "awaiting_topology".to_owned();
            current.stage = "topology".to_owned();
            current.message = Some(
                "Request accepted; waiting for the remote Herdr topology to confirm it".to_owned(),
            );
            current.retryable = false;
            current.created_pane_id = created_pane_id
                .as_deref()
                .map(|pane_id| remote_pane_id(target_id, pane_id));
            current.deadline_at_unix_ms =
                Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
        }
        if !self.settle_remote_operation_from_known_session(&key) {
            self.sync_async_operations();
        }
        true
    }

    /// Settles the operation `key` from the session its device last reported.
    /// A session that shows an operation's effect can arrive while the request
    /// is still unanswered, when only that session's own pass could have
    /// confirmed it; the device sends nothing more once its layout stops
    /// moving, so the answer, and every other step that leaves the operation
    /// waiting, has to look at what was already received.
    ///
    /// The held session counts only when it belongs to the connection the
    /// operation was sent on: a device's session is kept across a disconnected
    /// interval, and one from before a reconnect says nothing about a request
    /// made after it, so that case waits for the next pass.
    fn settle_remote_operation_from_known_session(&mut self, key: &(String, String)) -> bool {
        let Some(operation) = self.remote_operations.get(key) else {
            return false;
        };
        if !matches!(operation.phase.as_str(), "awaiting_topology" | "unknown")
            || operation.connection_generation
                != self
                    .remote_connection_generations
                    .get(&key.0)
                    .copied()
                    .unwrap_or(0)
        {
            return false;
        }
        let Some(session) = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == key.0 && status.state == "connected")
            .and_then(|status| status.session.as_ref())
        else {
            return false;
        };
        if !remote_operation_confirmed_by(operation, session) {
            return false;
        }
        self.settle_remote_operations(vec![key.clone()], RemoteSettle::HeldSession)
    }

    /// Settles the operations of `target_id` that `session`, that device's
    /// fresh report, shows the effect of. Another device's operation is not
    /// judged by it: its pane being absent here proves nothing there.
    pub(super) fn observe_remote_operations(
        &mut self,
        target_id: &str,
        session: &RemoteSessionSnapshot,
    ) -> bool {
        let completed = self
            .remote_operations
            .iter()
            .filter(|((target, _), operation)| {
                target == target_id
                    && matches!(operation.phase.as_str(), "awaiting_topology" | "unknown")
            })
            .filter(|(_, operation)| remote_operation_confirmed_by(operation, session))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        self.settle_remote_operations(completed, RemoteSettle::FreshSession)
    }

    fn settle_remote_operations(
        &mut self,
        completed: Vec<(String, String)>,
        via: RemoteSettle,
    ) -> bool {
        let mut changed = false;
        for key in completed {
            if let Some(operation) = self.remote_operations.remove(&key) {
                changed = true;
                self.push_diagnostic(
                    "remote.control.topology_confirmed",
                    format!(
                        "Confirmed {} for {} on {} from {}",
                        operation.kind,
                        operation.target_id,
                        key.0,
                        via.source()
                    ),
                );
            }
        }
        if changed {
            self.sync_async_operations();
        }
        changed
    }

    pub(super) fn expire_remote_operations(&mut self, now_unix_ms: u64) -> bool {
        let mut expired = Vec::new();
        for (key, operation) in &mut self.remote_operations {
            let Some(deadline) = operation.deadline_at_unix_ms else {
                continue;
            };
            if deadline > now_unix_ms {
                continue;
            }
            if matches!(
                operation.phase.as_str(),
                "transmitting" | "awaiting_topology"
            ) {
                operation.phase = "unknown".to_owned();
                operation.stage = "status_check".to_owned();
                operation.message = Some(
                    "Remote result is unknown; no mutation was resent and a fresh topology is required"
                        .to_owned(),
                );
                operation.retryable = false;
                operation.deadline_at_unix_ms = None;
                expired.push(key.clone());
            }
        }
        let changed = !expired.is_empty();
        // An unknown operation waits for the device's next session, which an
        // idle device may never send; the one already held may show the effect.
        let mut settled = false;
        for key in &expired {
            settled |= self.settle_remote_operation_from_known_session(key);
        }
        if changed && !settled {
            self.sync_async_operations();
        }
        changed
    }

    pub(super) fn mark_remote_operation_unknown(
        &mut self,
        key: &(String, String),
        message: String,
    ) -> bool {
        let operation_id = {
            let Some(operation) = self.remote_operations.get_mut(key) else {
                return false;
            };
            if matches!(operation.phase.as_str(), "completed" | "failed" | "refused") {
                return false;
            }
            operation.phase = "unknown".to_owned();
            operation.stage = "status_check".to_owned();
            operation.message = Some(message);
            operation.retryable = false;
            operation.deadline_at_unix_ms = None;
            operation.id.clone()
        };
        self.push_diagnostic(
            "remote.control.unknown",
            format!("Remote operation {operation_id} requires fresh topology"),
        );
        if !self.settle_remote_operation_from_known_session(key) {
            self.sync_async_operations();
        }
        true
    }

    pub(super) fn mark_tab_move_unknown(
        &mut self,
        checkout_id: &str,
        generation: u64,
        connection_generation: u64,
        reason: String,
    ) -> bool {
        let Some(pending) = self.pending_tab_move.get_mut(checkout_id) else {
            return false;
        };
        if pending.generation != generation
            || pending.connection_generation != connection_generation
            || matches!(pending.phase.as_str(), "completed" | "failed" | "refused")
        {
            return false;
        }
        pending.phase = "unknown".to_owned();
        pending.stage = "status_check".to_owned();
        pending.message = Some(format!(
            "Tab move result is unknown; no mutation was resent: {reason}"
        ));
        pending.retryable = false;
        pending.deadline_at_unix_ms = None;
        self.push_diagnostic(
            "tab.move.unknown",
            format!("Tab move for {checkout_id} requires fresh topology; mutation was not resent"),
        );
        self.set_error(
            "tab.move_unknown",
            format!("Tab move result is unknown; no move was resent: {reason}"),
            true,
        );
        self.sync_async_operations();
        true
    }

    /// The confirmable shape of a model layout: pane ids, split geometry, zoom
    /// and focus. It carries no PTY rectangles, so a resize cannot be
    /// confirmed against it; only a fresh session layout can do that.
    pub(super) fn model_layout_signature(layout: &PaneLayoutSnapshot) -> PaneTopologySignature {
        fn collect(
            node: &PaneLayoutNodeSnapshot,
            pane_ids: &mut Vec<String>,
            splits: &mut Vec<(u8, u32)>,
        ) {
            match node {
                PaneLayoutNodeSnapshot::Pane { pane_id } => pane_ids.push(pane_id.clone()),
                PaneLayoutNodeSnapshot::Split {
                    direction,
                    ratio,
                    first,
                    second,
                } => {
                    splits.push((
                        match direction {
                            crate::model::PaneLayoutDirection::Right => 0,
                            crate::model::PaneLayoutDirection::Down => 1,
                        },
                        ratio.to_bits(),
                    ));
                    collect(first, pane_ids, splits);
                    collect(second, pane_ids, splits);
                }
            }
        }

        let mut signature = PaneTopologySignature {
            zoomed: layout.zoomed,
            focused_pane_id: layout.focused_pane_id.clone(),
            ..PaneTopologySignature::default()
        };
        collect(&layout.root, &mut signature.pane_ids, &mut signature.splits);
        signature.pane_ids.sort();
        signature.splits.sort_unstable();
        signature
    }

    pub(super) fn session_layout_signature(
        layout: &crate::sidebar::SessionLayoutPayload,
    ) -> PaneTopologySignature {
        let mut signature = PaneTopologySignature {
            zoomed: layout.zoomed,
            focused_pane_id: layout.focused_pane_id.clone(),
            pane_ids: layout
                .panes
                .iter()
                .map(|pane| pane.pane_id.clone())
                .collect(),
            splits: layout
                .splits
                .iter()
                .map(|split| {
                    (
                        match split.direction {
                            crate::model::PaneLayoutDirection::Right => 0,
                            crate::model::PaneLayoutDirection::Down => 1,
                        },
                        split.ratio.to_bits(),
                    )
                })
                .collect(),
            pane_rects: Some(
                layout
                    .panes
                    .iter()
                    .map(|pane| {
                        (
                            pane.pane_id.clone(),
                            pane.rect.x,
                            pane.rect.y,
                            pane.rect.width,
                            pane.rect.height,
                        )
                    })
                    .collect(),
            ),
            split_rects: Some(
                layout
                    .splits
                    .iter()
                    .map(|split| {
                        (
                            match split.direction {
                                crate::model::PaneLayoutDirection::Right => 0,
                                crate::model::PaneLayoutDirection::Down => 1,
                            },
                            split.ratio.to_bits(),
                            split.rect.x,
                            split.rect.y,
                            split.rect.width,
                            split.rect.height,
                        )
                    })
                    .collect(),
            ),
        };
        signature.pane_ids.sort();
        signature.splits.sort_unstable();
        signature.pane_rects.as_mut().expect("set above").sort();
        signature
            .split_rects
            .as_mut()
            .expect("set above")
            .sort_unstable();
        signature
    }

    pub(super) fn pane_operation_scope(&self, pane_id: &str) -> Option<String> {
        self.snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&pane_id))
            .map(|layout| layout.tab_id.clone())
    }

    /// Whether `pane_id` is the only pane of an unzoomed tab, where zoom has
    /// nothing to hide. Herdr answers such a zoom unchanged (`single_pane`),
    /// so no topology would ever confirm it, and the operation left waiting
    /// would turn away every later split, zoom, resize and close in the tab.
    pub(super) fn pane_alone_unzoomed(&self, pane_id: &str) -> bool {
        self.snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&pane_id))
            .is_some_and(|layout| !layout.zoomed && layout.pane_ids().len() == 1)
    }

    pub(super) fn pane_operation_baseline(&self, pane_id: &str) -> Option<PaneTopologySignature> {
        self.snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.pane_ids().contains(&pane_id))
            .map(|layout| {
                self.confirmed_pane_layout_signatures
                    .get(&layout.tab_id)
                    .cloned()
                    .unwrap_or_else(|| Self::model_layout_signature(layout))
            })
    }

    /// Admits a split, zoom or resize into its tab's line and sends it when
    /// nothing ahead of it is still waiting for Herdr (D-10). A zoom or resize
    /// is drawn at once on top of what the operations ahead of it will leave
    /// (D-07); a split is drawn when Herdr names the pane it made.
    pub(super) fn begin_pane_operation(
        &mut self,
        action: PaneControlAction,
        input_request: Option<&str>,
    ) -> bool {
        let kind = action.kind();
        let target_id = action.pane_id().to_owned();
        let Some(scope_id) = self.pane_operation_scope(&target_id) else {
            self.set_error(
                "pane.operation_scope_unavailable",
                format!("Herdr has no confirmed layout for pane {target_id}"),
                true,
            );
            return true;
        };
        let Some(baseline_signature) = self.pane_operation_baseline(&target_id) else {
            self.set_error(
                "pane.operation_scope_unavailable",
                format!("Herdr has no confirmed layout for pane {target_id}"),
                true,
            );
            return true;
        };
        self.pane_operations.retain(|_, operation| {
            !(operation.scope_id == scope_id
                && matches!(operation.phase.as_str(), "failed" | "refused"))
        });
        // A resize behind an unsent resize of the same pane and axis moves
        // that one further instead, so a drag cannot fill the line.
        if self.fold_queued_resize(&scope_id, &action) {
            self.request_republish();
            self.sync_recent_closed_snapshot();
            return true;
        }
        if !self.admit_to_geometry_queue(&scope_id, kind, &target_id) {
            return true;
        }
        let prediction = self.request_prediction(&scope_id, &action);
        let id = self.insert_queued_geometry(
            kind,
            target_id.clone(),
            scope_id.clone(),
            baseline_signature,
            GeometryRequest::Pane(action),
            prediction,
        );
        self.begin_op_timing(&id, kind, Some(&scope_id), &[&target_id]);
        // A zoom or resize is drawn at the request, and the pane's grid
        // follows the drawing, so its record waits for the frame after the
        // resize that drawing sends (PRD grid-ahead-of-herdr D-06); a
        // split's record learns its pane from Herdr's answer.
        if matches!(kind, "pane.zoom" | "pane.resize") {
            self.await_op_frame(&id, &target_id, true);
        }
        if let Some(request_id) = input_request {
            self.input_requests.open(
                request_id,
                super::terminal_input::InputOrigin::Split(id.clone()),
                std::time::Instant::now(),
            );
            self.sync_input_requests();
        }
        if self.pane_operations[&id].prediction.is_some() {
            self.request_republish();
        }
        self.sync_recent_closed_snapshot();
        self.pump_geometry_queue(&scope_id);
        true
    }

    /// Draws a split Herdr has already answered outside the tab's line, as
    /// the fork worker's split right of its source is, through the record a
    /// split Hide asked for keeps from its answer: drawn until Herdr's layout
    /// lists `created`, unknown at the same bound, and holding the tab's line
    /// meanwhile, since what waits there is predicted on the layout with it.
    pub(super) fn draw_answered_split(&mut self, target: &str, created: &str) {
        let (Some(scope_id), Some(baseline_signature)) = (
            self.pane_operation_scope(target),
            self.pane_operation_baseline(target),
        ) else {
            return;
        };
        let id = self.insert_queued_geometry(
            "pane.split",
            target.to_owned(),
            scope_id,
            baseline_signature,
            GeometryRequest::Pane(PaneControlAction::Split {
                pane_id: target.to_owned(),
                direction: PaneSplitDirection::Right,
                cwd: None,
            }),
            Some(super::pane_prediction::Prediction::Split {
                target: target.to_owned(),
                direction: crate::model::PaneLayoutDirection::Right,
                created: created.to_owned(),
            }),
        );
        if let Some(operation) = self.pane_operations.get_mut(&id) {
            operation.phase = "awaiting_topology".to_owned();
            operation.stage = "topology".to_owned();
            operation.created_pane_id = Some(created.to_owned());
            operation.deadline_at_unix_ms =
                Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
        }
        self.request_republish();
        self.settle_pane_operation_from_known_topology(&id);
        self.sync_recent_closed_snapshot();
    }

    /// Takes back the drawn split of `created`, a pane its worker closed
    /// before Herdr laid it out, and lets the tab's line move on.
    pub(super) fn take_back_answered_split(&mut self, created: &str) {
        let Some(id) = self
            .pane_operations
            .values()
            .find(|operation| {
                operation.phase == "awaiting_topology"
                    && operation.created_pane_id.as_deref() == Some(created)
            })
            .map(|operation| operation.id.clone())
        else {
            return;
        };
        let operation = self
            .pane_operations
            .remove(&id)
            .expect("the operation was just found");
        crate::diagnostic!(serde_json::json!({
            "component": "pane_operation",
            "kind": "pane.split.taken_back",
            "tab_id": operation.scope_id,
            "pane_id": created,
        }));
        self.request_republish();
        self.pump_geometry_queue(&operation.scope_id);
        self.sync_recent_closed_snapshot();
    }

    /// Admits the operator's close of one pane among several into its tab's
    /// line when the tab is busy; returns false when the tab is idle and the
    /// close should start now. A close that removes the whole tab, a pane on
    /// a busy tab past the line's limit, and every tree close start as they
    /// always did.
    ///
    /// Whether the close removes the whole tab is Herdr's layout's answer,
    /// not the drawing's: a pane drawn alone because a close ahead took its
    /// sibling still has that sibling in Herdr's tab, so its close waits its
    /// turn instead of meeting the close in progress. It is drawn gone only
    /// while the line leaves another pane; the tab's last pane leaves with
    /// its tab when the close runs.
    pub(super) fn queue_pane_close(&mut self, pane_id: &str, confirmed: bool) -> bool {
        let Some(scope_id) = self.pane_operation_scope(pane_id) else {
            return false;
        };
        let several = self
            .confirmed_layouts
            .get(&scope_id)
            .or_else(|| {
                self.snapshot
                    .pane_layouts
                    .iter()
                    .find(|layout| layout.tab_id == scope_id)
            })
            .is_some_and(|layout| layout.pane_ids().len() > 1);
        if !several || !self.geometry_tab_busy(&scope_id) {
            return false;
        }
        let keeps_a_pane = self
            .predicted_tab_layout(&scope_id)
            .is_some_and(|layout| layout.pane_ids().into_iter().any(|id| id != pane_id));
        let Some(baseline_signature) = self.pane_operation_baseline(pane_id) else {
            return false;
        };
        if !self.admit_to_geometry_queue(&scope_id, "pane.close", pane_id) {
            return true;
        }
        let prediction = keeps_a_pane.then(|| super::pane_prediction::Prediction::Close {
            pane: pane_id.to_owned(),
        });
        self.insert_queued_geometry(
            "pane.close",
            pane_id.to_owned(),
            scope_id,
            baseline_signature,
            GeometryRequest::Close {
                pane_id: pane_id.to_owned(),
                confirmed,
            },
            prediction,
        );
        self.push_diagnostic(
            "pane.close.queued",
            format!("Closing pane {pane_id} after the operations ahead of it"),
        );
        self.request_republish();
        self.sync_recent_closed_snapshot();
        true
    }

    fn insert_queued_geometry(
        &mut self,
        kind: &str,
        target_id: String,
        scope_id: String,
        baseline_signature: PaneTopologySignature,
        request: GeometryRequest,
        prediction: Option<super::pane_prediction::Prediction>,
    ) -> String {
        self.next_async_operation_id = self.next_async_operation_id.saturating_add(1).max(1);
        let sequence = self.next_async_operation_id;
        let now = unix_milliseconds();
        let id = format!("pane-op-{now}-{sequence}");
        self.pane_operations.insert(
            id.clone(),
            PendingPaneOperation {
                id: id.clone(),
                kind: kind.to_owned(),
                target_id,
                scope_id,
                connection_generation: self.live_generation,
                phase: "queued".to_owned(),
                stage: "queue".to_owned(),
                started_at_unix_ms: now,
                deadline_at_unix_ms: None,
                message: None,
                retryable: false,
                created_pane_id: None,
                baseline_signature,
                request,
                sequence,
                prediction,
            },
        );
        id
    }

    /// Whether the tab has room in its line, refusing and logging the
    /// request when it does not.
    fn admit_to_geometry_queue(&mut self, scope_id: &str, kind: &str, target_id: &str) -> bool {
        let waiting = self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == scope_id && operation.phase == "queued")
            .count();
        if waiting < GEOMETRY_QUEUE_LIMIT {
            return true;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "pane_operation",
            "kind": "pane.operation.queue_full",
            "operation": kind,
            "tab_id": scope_id,
            "pane_id": target_id,
            "limit": GEOMETRY_QUEUE_LIMIT,
        }));
        self.push_diagnostic(
            "pane.operation.queue_full",
            format!(
                "{kind} for {target_id} was refused: {GEOMETRY_QUEUE_LIMIT} operations already wait in tab {scope_id}"
            ),
        );
        false
    }

    /// Whether an operation of the tab is still waiting for Herdr, so the
    /// next one has to wait its turn. A result that became unknown does not
    /// hold the line (D-09, D-12).
    fn geometry_in_flight(&self, scope_id: &str) -> bool {
        self.pane_operations.values().any(|operation| {
            operation.scope_id == scope_id
                && matches!(
                    operation.phase.as_str(),
                    "transmitting" | "awaiting_topology"
                )
        }) || self.close_operations.values().any(|operation| {
            operation.scope_id == scope_id
                && matches!(
                    operation.phase.as_str(),
                    "preparing" | "transmitting" | "awaiting_topology"
                )
        })
    }

    pub(super) fn geometry_tab_busy(&self, scope_id: &str) -> bool {
        self.geometry_in_flight(scope_id)
            || self
                .pane_operations
                .values()
                .any(|operation| operation.scope_id == scope_id && operation.phase == "queued")
    }

    /// The tab's layout as it will stand once every operation already in its
    /// line lands: what the next request is predicted on.
    /// The tab as the line will leave it: Herdr's confirmed layout with every
    /// prediction folded in. The stored layout already shows the drawn ones,
    /// so the fold starts from what Herdr confirmed when there is a drawing.
    fn predicted_tab_layout(&self, scope_id: &str) -> Option<PaneLayoutSnapshot> {
        let layout = self.confirmed_layouts.get(scope_id).or_else(|| {
            self.snapshot
                .pane_layouts
                .iter()
                .find(|layout| layout.tab_id == scope_id)
        })?;
        Some(super::pane_prediction::predict(
            layout,
            &self.geometry_predictions(scope_id),
        ))
    }

    fn request_prediction(
        &self,
        scope_id: &str,
        action: &PaneControlAction,
    ) -> Option<super::pane_prediction::Prediction> {
        let layout = self.predicted_tab_layout(scope_id)?;
        match action {
            PaneControlAction::ToggleZoom { pane_id } => {
                Some(super::pane_prediction::Prediction::Zoom {
                    pane: pane_id.clone(),
                    zoomed: !layout.zoomed,
                })
            }
            PaneControlAction::Resize {
                pane_id,
                direction,
                amount,
            } => super::pane_prediction::resize_prediction(&layout, pane_id, *direction, *amount),
            _ => None,
        }
    }

    /// Folds `action` into the last unsent resize of the same pane and axis
    /// in the tab's line, when that is the line's last entry.
    fn fold_queued_resize(&mut self, scope_id: &str, action: &PaneControlAction) -> bool {
        let PaneControlAction::Resize {
            pane_id,
            direction,
            amount,
        } = action
        else {
            return false;
        };
        let signed = |direction: PaneResizeDirection, amount: f32| match direction {
            PaneResizeDirection::Left | PaneResizeDirection::Up => -amount,
            PaneResizeDirection::Right | PaneResizeDirection::Down => amount,
        };
        let horizontal = |direction: PaneResizeDirection| {
            matches!(
                direction,
                PaneResizeDirection::Left | PaneResizeDirection::Right
            )
        };
        let Some(last_id) = self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == scope_id && operation.phase == "queued")
            .max_by_key(|operation| operation.sequence)
            .map(|operation| operation.id.clone())
        else {
            return false;
        };
        let GeometryRequest::Pane(PaneControlAction::Resize {
            pane_id: queued_pane,
            direction: queued_direction,
            amount: queued_amount,
        }) = &self.pane_operations[&last_id].request
        else {
            return false;
        };
        if queued_pane != pane_id || horizontal(*queued_direction) != horizontal(*direction) {
            return false;
        }
        let delta = (signed(*queued_direction, *queued_amount) + signed(*direction, *amount))
            .clamp(-0.5, 0.5);
        let folded = if delta.abs() < 0.001 {
            None
        } else {
            let direction = match (horizontal(*direction), delta > 0.0) {
                (true, true) => PaneResizeDirection::Right,
                (true, false) => PaneResizeDirection::Left,
                (false, true) => PaneResizeDirection::Down,
                (false, false) => PaneResizeDirection::Up,
            };
            Some(PaneControlAction::Resize {
                pane_id: pane_id.clone(),
                direction,
                amount: delta.abs(),
            })
        };
        let Some(folded) = folded else {
            // The two cancel out: nothing is left to send.
            self.pane_operations.remove(&last_id);
            self.op_timings.finish(&last_id, "cancelled");
            return true;
        };
        // Predicted on the line without the entry it replaces.
        let mut predictions = self.geometry_predictions(scope_id);
        predictions.retain(|prediction| {
            self.pane_operations[&last_id].prediction.as_ref() != Some(prediction)
        });
        // From Herdr's confirmed layout, as `predicted_tab_layout`: the
        // stored one already shows the entry being replaced.
        let prediction = self
            .confirmed_layouts
            .get(scope_id)
            .or_else(|| {
                self.snapshot
                    .pane_layouts
                    .iter()
                    .find(|layout| layout.tab_id == scope_id)
            })
            .map(|layout| super::pane_prediction::predict(layout, &predictions))
            .and_then(|layout| match &folded {
                PaneControlAction::Resize {
                    pane_id,
                    direction,
                    amount,
                } => {
                    super::pane_prediction::resize_prediction(&layout, pane_id, *direction, *amount)
                }
                _ => None,
            });
        let operation = self.pane_operations.get_mut(&last_id).expect("found above");
        operation.request = GeometryRequest::Pane(folded);
        operation.prediction = prediction;
        true
    }

    /// The predictions of the tab's line in order: its pane operations and
    /// the closes of one pane among several that are still settling.
    pub(super) fn geometry_predictions(
        &self,
        scope_id: &str,
    ) -> Vec<super::pane_prediction::Prediction> {
        let mut ordered = self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == scope_id)
            .filter_map(|operation| {
                operation.prediction.clone().map(|prediction| {
                    (operation.started_at_unix_ms, operation.sequence, prediction)
                })
            })
            .collect::<Vec<_>>();
        ordered.extend(
            self.close_operations
                .values()
                .filter(|operation| operation.scope_id == scope_id)
                .filter(|operation| {
                    matches!(
                        operation.phase.as_str(),
                        "preparing" | "transmitting" | "awaiting_topology"
                    ) && operation.leaving_tab().is_none()
                })
                .filter_map(|operation| match &operation.request.target {
                    live::CloseCaptureTarget::Pane { pane_id } => {
                        let (started, sequence) = self
                            .close_line_places
                            .get(&operation.request.key)
                            .copied()
                            .unwrap_or((operation.started_at_unix_ms, 0));
                        Some((
                            started,
                            sequence,
                            super::pane_prediction::Prediction::Close {
                                pane: pane_id.clone(),
                            },
                        ))
                    }
                    live::CloseCaptureTarget::Tab { .. } => None,
                }),
        );
        ordered.sort_by_key(|(started, sequence, _)| (*started, *sequence));
        ordered
            .into_iter()
            .map(|(_, _, prediction)| prediction)
            .filter(|prediction| match prediction {
                super::pane_prediction::Prediction::Close { pane } => {
                    self.failed_terminal_id(pane).is_none()
                }
                _ => true,
            })
            .collect()
    }

    /// The closes of one pane among several drawn gone ahead of Herdr.
    fn drawn_close_keys(&self) -> Vec<&str> {
        let mut keys = self
            .close_operations
            .iter()
            .filter(|(_, operation)| {
                matches!(
                    operation.phase.as_str(),
                    "preparing" | "transmitting" | "awaiting_topology"
                ) && operation.leaving_tab().is_none()
            })
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    /// Runs `apply`, a close's answer arriving off the session stream, and
    /// when it changed which closes are drawn ahead of Herdr reads the session
    /// again (a refused or unknown close is drawn back) and lets the tab
    /// lines behind those closes move on.
    pub(super) fn with_close_geometry<T>(&mut self, apply: impl FnOnce(&mut Self) -> T) -> T {
        let before = self
            .drawn_close_keys()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let answer = apply(self);
        if self.drawn_close_keys() != before {
            self.request_republish();
            self.pump_geometry_queues();
        }
        answer
    }

    /// Sends the next operation of the tab's line once nothing ahead of it
    /// is still waiting for Herdr.
    pub(super) fn pump_geometry_queue(&mut self, scope_id: &str) -> bool {
        // An operation the pump starts can end at once and pump again; the
        // outer pump owns the line, so the inner one leaves it alone.
        if self.pumping_geometry {
            return false;
        }
        self.pumping_geometry = true;
        let moved = self.pump_geometry_line(scope_id);
        self.pumping_geometry = false;
        moved
    }

    fn pump_geometry_line(&mut self, scope_id: &str) -> bool {
        let mut moved = false;
        while !self.geometry_in_flight(scope_id) {
            let Some(id) = self
                .pane_operations
                .values()
                .filter(|operation| operation.scope_id == scope_id && operation.phase == "queued")
                .min_by_key(|operation| operation.sequence)
                .map(|operation| operation.id.clone())
            else {
                break;
            };
            let request = self.pane_operations[&id].request.clone();
            // A close starts once the close ahead of it has settled and left
            // the tab, which a settling one about to leave would refuse, and
            // from a navigator that shows the session as it now stands. A
            // close ahead that failed or is unknown stays until the operator
            // acts; starting meets its refusal, which drops the line.
            if matches!(request, GeometryRequest::Close { .. })
                && (self.ingesting_session
                    || self
                        .close_operations
                        .values()
                        .any(|operation| operation.scope_id == scope_id && operation.settling()))
            {
                break;
            }
            moved = true;
            match request {
                GeometryRequest::Pane(action) => {
                    self.send_pane_operation(&id, action);
                    break;
                }
                GeometryRequest::Close { pane_id, confirmed } => {
                    let place = self
                        .pane_operations
                        .remove(&id)
                        .map(|queued| (queued.started_at_unix_ms, queued.sequence));
                    let before = self
                        .close_operations
                        .keys()
                        .cloned()
                        .collect::<HashSet<_>>();
                    self.close_local_pane(pane_id.clone(), confirmed, false);
                    let started = self
                        .close_operations
                        .iter()
                        .find(|(key, operation)| {
                            !before.contains(*key) && operation.target_id == pane_id
                        })
                        .map(|(key, _)| key.clone());
                    // The close keeps its place in the line for the drawing:
                    // what was asked after it is predicted on the tab without
                    // the pane.
                    if let (Some(key), Some(place)) = (started.as_ref(), place) {
                        let live_keys = self
                            .close_operations
                            .keys()
                            .cloned()
                            .collect::<HashSet<_>>();
                        self.close_line_places
                            .retain(|key, _| live_keys.contains(key));
                        self.close_line_places.insert(key.clone(), place);
                    }
                    if started.is_none() {
                        // A close that did not start is refused like any other
                        // step of the line: what waited behind it goes too.
                        self.drop_geometry_queue(scope_id, "close_refused");
                        self.request_republish();
                        break;
                    }
                }
            }
        }
        moved
    }

    /// Every tab whose line has an operation waiting.
    pub(super) fn pump_geometry_queues(&mut self) -> bool {
        let scopes = self
            .pane_operations
            .values()
            .filter(|operation| operation.phase == "queued")
            .map(|operation| operation.scope_id.clone())
            .collect::<HashSet<_>>();
        let mut moved = false;
        for scope_id in scopes {
            moved |= self.pump_geometry_queue(&scope_id);
        }
        moved
    }

    fn send_pane_operation(&mut self, id: &str, action: PaneControlAction) {
        let Some(context) = self.live.as_ref().cloned() else {
            self.fail_pane_operation(id, "Herdr disconnected before the request was sent".into());
            return;
        };
        let now = unix_milliseconds();
        let baseline = self
            .pane_operation_baseline(action.pane_id())
            .unwrap_or_else(|| self.pane_operations[id].baseline_signature.clone());
        let operation = self.pane_operations.get_mut(id).expect("queued above");
        operation.phase = "transmitting".to_owned();
        operation.stage = "request".to_owned();
        operation.connection_generation = self.live_generation;
        operation.deadline_at_unix_ms = Some(now.saturating_add(CLOSE_STAGE_TIMEOUT_MS));
        // The tab as Herdr has it now, after what was ahead in the line.
        operation.baseline_signature = baseline;
        if let Err(message) =
            live::spawn_pane_control_with_generation(context, action, Some(self.live_generation))
        {
            self.fail_pane_operation(id, message);
        }
    }

    /// Drops every operation still waiting in the tab's line after the one
    /// ahead of it was refused, failed or became unknown (D-09, D-10): the
    /// layout returns to what Herdr confirmed and keys typed for a dropped
    /// split reach no pane.
    pub(super) fn drop_geometry_queue(&mut self, scope_id: &str, reason: &str) {
        let dropped = self
            .pane_operations
            .values()
            .filter(|operation| operation.scope_id == scope_id && operation.phase == "queued")
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>();
        for id in dropped {
            let Some(operation) = self.pane_operations.remove(&id) else {
                continue;
            };
            self.op_timings.finish(&id, "dropped");
            self.discard_input_request(
                &super::terminal_input::InputOrigin::Split(id.clone()),
                reason,
            );
            crate::diagnostic!(serde_json::json!({
                "component": "pane_operation",
                "kind": "pane.operation.dropped",
                "operation_id": id,
                "operation": operation.kind,
                "tab_id": scope_id,
                "pane_id": operation.target_id,
                "reason": reason,
            }));
        }
    }

    pub(super) fn fail_pane_operation(&mut self, id: &str, message: String) {
        let Some(operation) = self.pane_operations.get(id).cloned() else {
            return;
        };
        self.op_timings.finish(id, "failed");
        self.discard_input_request(
            &super::terminal_input::InputOrigin::Split(id.to_owned()),
            "failed",
        );
        if let Some(current) = self.pane_operations.get_mut(id) {
            current.phase = "failed".to_owned();
            current.stage = "request".to_owned();
            current.deadline_at_unix_ms = None;
            current.message = Some(message.clone());
            current.retryable = true;
            current.created_pane_id = None;
            current.prediction = None;
        }
        self.drop_geometry_queue(&operation.scope_id, "refused");
        self.request_republish();
        self.set_error(
            "pane.operation_failed",
            format!(
                "{} for {} failed: {message}",
                operation.kind, operation.target_id
            ),
            true,
        );
        self.push_diagnostic(
            "pane.operation_failed",
            format!("{} {}: {message}", operation.kind, operation.target_id),
        );
        self.sync_recent_closed_snapshot();
    }

    pub(super) fn mark_pane_operation_unknown(&mut self, id: &str, message: String) -> bool {
        let operation_id = {
            let Some(operation) = self.pane_operations.get_mut(id) else {
                return false;
            };
            if matches!(operation.phase.as_str(), "completed" | "failed" | "refused") {
                return false;
            }
            operation.phase = "unknown".to_owned();
            operation.stage = "status_check".to_owned();
            operation.message = Some(format!(
                "{}; no mutation was resent and a fresh topology is required",
                message
            ));
            operation.retryable = false;
            operation.deadline_at_unix_ms = None;
            operation.prediction = None;
            (operation.id.clone(), operation.scope_id.clone())
        };
        let (operation_id, scope_id) = operation_id;
        // An unknown result draws what Herdr confirmed and drops what waited
        // behind it, the same as a refusal; it does not hold the line.
        self.drop_geometry_queue(&scope_id, "unknown");
        self.request_republish();
        self.op_timings.finish(&operation_id, "unknown");
        self.discard_input_request(
            &super::terminal_input::InputOrigin::Split(operation_id.clone()),
            "unknown",
        );
        self.push_diagnostic(
            "pane.operation.unknown",
            format!("Pane operation {operation_id} requires fresh topology"),
        );
        self.sync_async_operations();
        true
    }

    pub(super) fn ingest_pane_mutation_result(
        &mut self,
        id: &str,
        result: Result<PaneControlOutcome, live::ControlFailure>,
        elapsed_ms: u128,
    ) -> bool {
        let Some(operation) = self.pane_operations.get(id).cloned() else {
            return false;
        };
        if operation.phase != "transmitting" {
            return false;
        }
        if operation
            .deadline_at_unix_ms
            .is_some_and(|deadline| deadline <= unix_milliseconds())
        {
            return self.mark_pane_operation_unknown(
                id,
                "Pane mutation result arrived after its deadline".to_owned(),
            );
        }
        match result {
            Ok(PaneControlOutcome::Acknowledged { created_pane_id }) => {
                if operation.kind == "pane.split" && created_pane_id.is_none() {
                    self.fail_pane_operation(
                        id,
                        "Herdr acknowledged split without a created pane id".to_owned(),
                    );
                    return true;
                }
                let now = std::time::Instant::now();
                self.op_timings
                    .stamp(id, super::op_timing::Stage::HerdrAck, now);
                if let Some(created) = created_pane_id.as_deref() {
                    self.op_timings.learn(id, None, Some(created));
                    self.await_op_frame(id, created, false);
                    // The split is drawn now, with the pane Herdr named
                    // (D-03, D-07); Herdr's layout replaces it on arrival.
                    if let GeometryRequest::Pane(PaneControlAction::Split {
                        pane_id,
                        direction,
                        ..
                    }) = &operation.request
                        && let Some(current) = self.pane_operations.get_mut(id)
                    {
                        current.prediction = Some(super::pane_prediction::Prediction::Split {
                            target: pane_id.clone(),
                            direction: match direction {
                                PaneSplitDirection::Right => {
                                    crate::model::PaneLayoutDirection::Right
                                }
                                PaneSplitDirection::Down => crate::model::PaneLayoutDirection::Down,
                            },
                            created: created.to_owned(),
                        });
                        self.request_republish();
                    }
                    // Keys typed since the split was asked for belong to
                    // the pane Herdr just named, and to no other, once its
                    // shell reads them.
                    self.resolve_input_request_at_shell(
                        &super::terminal_input::InputOrigin::Split(id.to_owned()),
                        created,
                    );
                }
                if let Some(current) = self.pane_operations.get_mut(id) {
                    current.phase = "awaiting_topology".to_owned();
                    current.stage = "topology".to_owned();
                    current.message = Some(
                        "Request accepted; waiting for Herdr to confirm the new topology"
                            .to_owned(),
                    );
                    current.retryable = false;
                    current.created_pane_id = created_pane_id;
                    current.deadline_at_unix_ms =
                        Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                }
                self.push_diagnostic(
                    "pane.operation.accepted",
                    format!(
                        "{} for {} acknowledged in {elapsed_ms} ms; awaiting topology",
                        operation.kind, operation.target_id
                    ),
                );
                if self.settle_pane_operation_from_known_topology(id) {
                    self.request_republish();
                }
            }
            // Herdr moved nothing, so no layout event follows: the operation
            // ends at this answer and the line moves on (D-12).
            Ok(PaneControlOutcome::Unchanged) => {
                self.pane_operations.remove(id);
                self.op_timings.finish(id, "unchanged");
                // A split that made no pane leaves its keys nowhere to go.
                self.discard_input_request(
                    &super::terminal_input::InputOrigin::Split(id.to_owned()),
                    "unchanged",
                );
                self.push_diagnostic(
                    "pane.operation.unchanged",
                    format!(
                        "{} for {} changed nothing in {elapsed_ms} ms",
                        operation.kind, operation.target_id
                    ),
                );
                if operation.prediction.is_some() {
                    self.request_republish();
                }
            }
            Ok(PaneControlOutcome::Projected { .. }) => self.fail_pane_operation(
                id,
                "Herdr returned a projection outcome for a mutation request".to_owned(),
            ),
            Err(error) if error.is_ambiguous() => {
                self.mark_pane_operation_unknown(id, error.message().to_owned());
            }
            Err(error) => self.fail_pane_operation(id, error.message().to_owned()),
        }
        self.pump_geometry_queue(&operation.scope_id);
        self.sync_recent_closed_snapshot();
        true
    }

    pub(super) fn observe_pane_operations(&mut self, payload: &SessionSnapshotPayload) -> bool {
        let completed = self
            .pane_operations
            .iter()
            .filter(|(_, operation)| {
                matches!(operation.phase.as_str(), "awaiting_topology" | "unknown")
            })
            .filter_map(|(id, operation)| {
                let layout = payload
                    .layouts
                    .iter()
                    .find(|layout| layout.tab_id == operation.scope_id)?;
                let current = Self::session_layout_signature(layout);
                Self::pane_operation_confirmed_by(operation, &current)
                    .then(|| (id.clone(), current))
            })
            .collect::<Vec<_>>();
        self.settle_pane_operations(completed)
    }

    /// Whether `current`, a tab's layout as Herdr last reported it, shows the
    /// effect of `operation`.
    fn pane_operation_confirmed_by(
        operation: &PendingPaneOperation,
        current: &PaneTopologySignature,
    ) -> bool {
        if !current.pane_ids.contains(&operation.target_id) {
            return false;
        }
        match operation.kind.as_str() {
            // A split is confirmed by the exact pane Herdr said it created. A
            // changed focus or ratio alone is not proof that this split landed.
            "pane.split" => operation
                .created_pane_id
                .as_ref()
                .is_some_and(|created| current.pane_ids.contains(created)),
            // Zoom has no geometry contract of its own. The zoom bit is the
            // authoritative confirmation, while a focus change is a separate
            // event and must not settle it.
            "pane.zoom" => current.zoomed != operation.baseline_signature.zoomed,
            // Resize must be proven by fresh raw rectangles. A no-op
            // acknowledgement or a focus-only event cannot settle a resize,
            // and a model projection has no rectangles to invent.
            "pane.resize" => {
                operation.baseline_signature.pane_rects.is_some()
                    && operation.baseline_signature.pane_rects != current.pane_rects
                    || operation.baseline_signature.split_rects.is_some()
                        && operation.baseline_signature.split_rects != current.split_rects
            }
            _ => *current != operation.baseline_signature,
        }
    }

    /// Settles the operation `id` from the layout Herdr last reported for its
    /// tab. A session update that shows an operation's effect can arrive while
    /// the request is still unanswered, when only the update's own pass could
    /// have confirmed it; Herdr sends nothing more once the layout stops
    /// moving, so the answer has to look at what was already received.
    fn settle_pane_operation_from_known_topology(&mut self, id: &str) -> bool {
        let Some(operation) = self.pane_operations.get(id) else {
            return false;
        };
        let Some(current) = self
            .confirmed_pane_layout_signatures
            .get(&operation.scope_id)
        else {
            return false;
        };
        if !Self::pane_operation_confirmed_by(operation, current) {
            return false;
        }
        let completed = vec![(id.to_owned(), current.clone())];
        self.settle_pane_operations(completed)
    }

    fn settle_pane_operations(&mut self, completed: Vec<(String, PaneTopologySignature)>) -> bool {
        let mut changed = false;
        let mut scopes = HashSet::new();
        for (id, current_signature) in completed {
            let Some(operation) = self.pane_operations.remove(&id) else {
                continue;
            };
            self.report_prediction_mismatch(&operation, &current_signature);
            self.op_timings.applied(&id, std::time::Instant::now());
            self.push_diagnostic(
                "pane.operation.topology_confirmed",
                format!(
                    "Confirmed {} for {} from fresh topology",
                    operation.kind, operation.target_id
                ),
            );
            // The next operation in the line is sent below and takes its
            // baseline from here, so it must be the layout that confirmed
            // this one, not the one from before it (D-10).
            self.confirmed_pane_layout_signatures
                .insert(operation.scope_id.clone(), current_signature);
            scopes.insert(operation.scope_id);
            changed = true;
        }
        for scope_id in scopes {
            self.pump_geometry_queue(&scope_id);
        }
        if changed {
            self.sync_recent_closed_snapshot();
        }
        changed
    }

    /// When Herdr's confirmed layout differs from what was drawn for the
    /// operation it confirms (a ratio Herdr rounds otherwise, a focus it
    /// places elsewhere), the confirmed layout is drawn and the difference
    /// goes to the log only (B18). Read only when nothing else is drawn for
    /// the tab, so the comparison is of this operation alone.
    fn report_prediction_mismatch(
        &self,
        operation: &PendingPaneOperation,
        confirmed: &PaneTopologySignature,
    ) {
        if operation.prediction.is_none()
            || self.drawn_closes.contains_key(&operation.scope_id)
            || self
                .pane_operations
                .values()
                .any(|other| other.scope_id == operation.scope_id && other.prediction.is_some())
        {
            return;
        }
        let Some(drawn) = self
            .snapshot
            .pane_layouts
            .iter()
            .find(|layout| layout.tab_id == operation.scope_id)
            .map(Self::model_layout_signature)
        else {
            return;
        };
        if drawn.pane_ids == confirmed.pane_ids
            && drawn.splits == confirmed.splits
            && drawn.zoomed == confirmed.zoomed
            && drawn.focused_pane_id == confirmed.focused_pane_id
        {
            return;
        }
        let ratios = |signature: &PaneTopologySignature| {
            signature
                .splits
                .iter()
                .map(|(_, bits)| f32::from_bits(*bits))
                .collect::<Vec<_>>()
        };
        crate::diagnostic!(serde_json::json!({
            "component": "pane_operation",
            "kind": "pane.prediction_mismatch",
            "operation_id": operation.id,
            "operation": operation.kind,
            "tab_id": operation.scope_id,
            "drawn_pane_ids": drawn.pane_ids,
            "confirmed_pane_ids": confirmed.pane_ids,
            "drawn_ratios": ratios(&drawn),
            "confirmed_ratios": ratios(confirmed),
            "drawn_zoomed": drawn.zoomed,
            "confirmed_zoomed": confirmed.zoomed,
            "drawn_focus": drawn.focused_pane_id,
            "confirmed_focus": confirmed.focused_pane_id,
        }));
    }

    pub(super) fn sync_async_operations(&mut self) {
        let mut operations = self
            .close_capture_order
            .iter()
            .filter_map(|key| {
                self.close_operations
                    .get(key)
                    .map(|operation| (key, operation))
            })
            .map(|(_key, operation)| crate::model::AsyncOperationSnapshot {
                id: operation.request.key.clone(),
                kind: Self::close_operation_kind(&operation.request.target).to_owned(),
                target_id: operation.target_id.clone(),
                scope_id: operation.scope_id.clone(),
                phase: operation.phase.clone(),
                stage: operation.stage.clone(),
                started_at_unix_ms: operation.started_at_unix_ms,
                deadline_at_unix_ms: operation.deadline_at_unix_ms,
                message: operation.message.clone(),
                retryable: operation.retryable,
            })
            .collect::<Vec<_>>();
        operations.extend(self.pane_operations.values().map(|operation| {
            crate::model::AsyncOperationSnapshot {
                id: operation.id.clone(),
                kind: operation.kind.clone(),
                target_id: operation.target_id.clone(),
                scope_id: operation.scope_id.clone(),
                phase: operation.phase.clone(),
                stage: operation.stage.clone(),
                started_at_unix_ms: operation.started_at_unix_ms,
                deadline_at_unix_ms: operation.deadline_at_unix_ms,
                message: operation.message.clone(),
                retryable: operation.retryable,
            }
        }));
        operations.extend(
            self.pending_tab_move
                .iter()
                .map(
                    |(checkout_id, operation)| crate::model::AsyncOperationSnapshot {
                        id: format!("tab.move:{checkout_id}:{}", operation.generation),
                        kind: "tab.move".to_owned(),
                        target_id: operation.target_id.clone(),
                        scope_id: checkout_id.clone(),
                        phase: operation.phase.clone(),
                        stage: operation.stage.clone(),
                        started_at_unix_ms: operation.started_at_unix_ms,
                        deadline_at_unix_ms: operation.deadline_at_unix_ms,
                        message: operation.message.clone(),
                        retryable: operation.retryable,
                    },
                ),
        );
        operations.extend(self.tree_close_operations());
        operations.extend(self.remote_operations.values().map(|operation| {
            crate::model::AsyncOperationSnapshot {
                id: operation.id.clone(),
                kind: operation.kind.clone(),
                target_id: operation.target_id.clone(),
                scope_id: operation.scope_id.clone(),
                phase: operation.phase.clone(),
                stage: operation.stage.clone(),
                started_at_unix_ms: operation.started_at_unix_ms,
                deadline_at_unix_ms: operation.deadline_at_unix_ms,
                message: operation.message.clone(),
                retryable: operation.retryable,
            }
        }));
        if let Some(attachment) = &self.attachment {
            operations.push(attachment.operation.clone());
        }
        if let Some(rejection) = &self.attachment_rejection {
            operations.push(rejection.clone());
        }
        operations.sort_by(|left, right| {
            left.started_at_unix_ms
                .cmp(&right.started_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.snapshot.status.async_operations = operations;
    }

    pub(super) fn expire_pane_operations(&mut self, now_unix_ms: u64) -> bool {
        let mut changed = false;
        let mut expired_scopes = Vec::new();
        for id in self.pane_operations.keys().cloned().collect::<Vec<_>>() {
            let Some(operation) = self.pane_operations.get(&id).cloned() else {
                continue;
            };
            let Some(deadline) = operation.deadline_at_unix_ms else {
                continue;
            };
            if deadline > now_unix_ms {
                continue;
            }
            let Some(current) = self.pane_operations.get_mut(&id) else {
                continue;
            };
            match current.phase.as_str() {
                "transmitting" | "awaiting_topology" => {
                    current.phase = "unknown".to_owned();
                    current.stage = "status_check".to_owned();
                    current.message = Some(
                        "Pane operation result is unknown; no mutation was resent. Waiting for a fresh topology check"
                            .to_owned(),
                    );
                    current.retryable = false;
                    current.prediction = None;
                    current.deadline_at_unix_ms =
                        Some(now_unix_ms.saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                    self.push_diagnostic(
                        "pane.operation.unconfirmed",
                        format!(
                            "{} for {} exceeded its deadline; mutation was not resent",
                            operation.kind, operation.target_id
                        ),
                    );
                    self.op_timings.finish(&id, "unknown");
                    self.discard_input_request(
                        &super::terminal_input::InputOrigin::Split(id.clone()),
                        "unknown",
                    );
                    expired_scopes.push(operation.scope_id.clone());
                    changed = true;
                }
                "unknown" => {
                    current.message = Some(
                        "Result check needed; no mutation was resent and the confirmed topology is unchanged"
                            .to_owned(),
                    );
                    current.deadline_at_unix_ms = None;
                    current.retryable = false;
                    self.push_diagnostic(
                        "pane.operation.status_needed",
                        format!(
                            "{} for {} remains unconfirmed after the read-only check",
                            operation.kind, operation.target_id
                        ),
                    );
                    changed = true;
                }
                _ => {}
            }
        }
        // A deadline is an unknown result: it draws what Herdr confirmed and
        // drops what waited behind it (D-09, D-10).
        for scope_id in &expired_scopes {
            self.drop_geometry_queue(scope_id, "unknown");
        }
        if !expired_scopes.is_empty() {
            self.request_republish();
        }
        if changed {
            self.sync_recent_closed_snapshot();
        }
        changed
    }

    pub(super) fn expire_tab_moves(&mut self, now_unix_ms: u64) -> bool {
        let mut changed = false;
        let checkout_ids = self.pending_tab_move.keys().cloned().collect::<Vec<_>>();
        for checkout_id in checkout_ids {
            let Some(pending) = self.pending_tab_move.get(&checkout_id).cloned() else {
                continue;
            };
            let Some(deadline) = pending.deadline_at_unix_ms else {
                continue;
            };
            if deadline > now_unix_ms {
                continue;
            }
            let Some(current) = self.pending_tab_move.get_mut(&checkout_id) else {
                continue;
            };
            match current.phase.as_str() {
                "transmitting" | "awaiting_topology" => {
                    current.phase = "unknown".to_owned();
                    current.stage = "status_check".to_owned();
                    current.message = Some(
                        "Tab move result is unknown; no mutation was resent. Waiting for a fresh topology check"
                            .to_owned(),
                    );
                    current.retryable = false;
                    current.deadline_at_unix_ms =
                        Some(now_unix_ms.saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                    self.push_diagnostic(
                        "tab.move.unconfirmed",
                        format!(
                            "Tab move for {} exceeded its deadline; mutation was not resent",
                            pending.target_id
                        ),
                    );
                    changed = true;
                }
                "unknown" => {
                    current.message = Some(
                        "Result check needed; no mutation was resent and the confirmed order is unchanged"
                            .to_owned(),
                    );
                    current.deadline_at_unix_ms = None;
                    current.retryable = false;
                    self.push_diagnostic(
                        "tab.move.status_needed",
                        format!(
                            "Tab move for {} remains unconfirmed after the topology check",
                            pending.target_id
                        ),
                    );
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.sync_async_operations();
        }
        changed
    }
}
