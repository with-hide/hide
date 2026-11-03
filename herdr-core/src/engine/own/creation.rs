//! One admitted creation through durable reserve, start and cancellation.
//! The same bounded intent retains its request, placement and capacity while
//! an answer is unknown. Resolution is caller driven, never a retry timer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use hide_node_link::holders::{HolderRequest, HolderReservation, HolderStart, PaneHolders};

use super::{EngineError, OwnEngine, Shared, holder_error, starts};
use crate::engine::Placement;

pub(super) type Ticket = Arc<Intent>;

pub(super) struct Intent {
    pub start: HolderStart,
    request: HolderRequest,
    submission_closed: Mutex<bool>,
    retiring: AtomicBool,
    reserved: OnceLock<HolderReservation>,
    _turn: starts::Turn,
    placements: Vec<Arc<dyn Placement>>,
    parent: Option<String>,
    pub drawn: AtomicBool,
    pub accepted: AtomicBool,
    unresolved: AtomicBool,
    resolving: AtomicBool,
    canceled: AtomicBool,
    withdrawn: AtomicBool,
}

impl Intent {
    fn submit(&self) -> Result<(), EngineError> {
        if *self
            .submission_closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            Err(EngineError::Unanswered(
                "the pane request has closed submission".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    fn withdraw(&self) {
        if !self.withdrawn.swap(true, Ordering::AcqRel) {
            for placement in &self.placements {
                placement.withdraw(&self.start.pane);
            }
        }
    }

    pub fn check_open(&self) -> Result<(), EngineError> {
        if self.canceled.load(Ordering::Acquire) {
            Err(super::refused(
                "pane_creation_cancelled",
                format!(
                    "pane {} was cancelled while being reserved",
                    self.start.pane
                ),
            ))
        } else {
            Ok(())
        }
    }
}

fn finish(shared: &Weak<Shared>, pane: &str) {
    if let Some(shared) = shared.upgrade() {
        let removed = shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(pane);
        drop(removed);
    }
}

pub(super) fn retire(
    holders: &dyn PaneHolders,
    ticket: &Intent,
    shared: &Weak<Shared>,
) -> Result<(), EngineError> {
    // Requests already admitted locally may still reach the node, but its
    // matching retire transition refuses them before storage work. Actual
    // node guards keep the slot until their final mutation and cleanup.
    *ticket
        .submission_closed
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = true;
    ticket.retiring.store(true, Ordering::Release);
    match holders.retire(ticket.request) {
        Ok(()) => {
            finish(shared, &ticket.start.pane);
            Ok(())
        }
        Err(error) => {
            ticket.unresolved.store(true, Ordering::Release);
            Err(EngineError::Unanswered(format!(
                "the pane request retirement is unresolved: {error:?}"
            )))
        }
    }
}

fn cancel(holders: &dyn PaneHolders, ticket: &Intent) -> Result<(), EngineError> {
    ticket.submit()?;
    holders
        .cancel(
            &ticket.start.intent().map_err(holder_error)?,
            ticket.request,
        )
        .map_err(holder_error)?;
    ticket.canceled.store(true, Ordering::Release);
    ticket.withdraw();
    Ok(())
}

pub(super) fn run(
    ticket: Ticket,
    holders: &dyn PaneHolders,
    shared: &Weak<Shared>,
) -> Result<(), EngineError> {
    let Some(reserved) = ticket.reserved.get().copied() else {
        ticket.unresolved.store(true, Ordering::Release);
        return Err(EngineError::Unanswered(
            "the reservation has not answered".to_owned(),
        ));
    };
    let result = ticket
        .check_open()
        .and_then(|()| ticket.submit())
        .and_then(|()| {
            holders
                .start_reserved(&ticket.start, reserved, ticket.request)
                .map_err(holder_error)
        });
    if result.as_ref().is_err_and(|error| error.reply_unknown()) {
        ticket.unresolved.store(true, Ordering::Release);
        return result;
    }
    if result.is_err()
        && !ticket.accepted.load(Ordering::Acquire)
        && let Err(error) = cancel(holders, &ticket)
    {
        ticket.unresolved.store(true, Ordering::Release);
        return Err(EngineError::Unanswered(format!(
            "the failed start's cancellation is unresolved: {error}"
        )));
    }
    retire(holders, &ticket, shared)?;
    result
}

impl OwnEngine<'_> {
    pub(super) fn reserve_start(
        &self,
        start: HolderStart,
        previous: Option<u64>,
        place: bool,
        parent: Option<&str>,
    ) -> Result<Ticket, EngineError> {
        let mut turn = self.shared.starts.reserve()?;
        turn.enter()?;
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if pending.contains_key(&start.pane) {
            return Err(EngineError::Unanswered(format!(
                "pane {} still has its earlier request",
                start.pane
            )));
        }
        // request() only issues a local number; it never sends or blocks.
        let request = self.shared.holders.request().map_err(holder_error)?;
        let ticket = Arc::new(Intent {
            start,
            request,
            submission_closed: Mutex::new(false),
            retiring: AtomicBool::new(false),
            reserved: OnceLock::new(),
            _turn: turn,
            placements: if place {
                self.gates
                    .iter()
                    .filter_map(|gate| gate.placement.clone())
                    .collect()
            } else {
                Vec::new()
            },
            parent: parent.map(str::to_owned),
            drawn: AtomicBool::new(false),
            accepted: AtomicBool::new(false),
            unresolved: AtomicBool::new(false),
            resolving: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
            withdrawn: AtomicBool::new(false),
        });
        pending.insert(ticket.start.pane.clone(), Arc::clone(&ticket));
        drop(pending);
        if place && let Err(error) = self.place(&ticket.start.pane) {
            // place already withdrew precisely the preceding placements.
            ticket.withdrawn.store(true, Ordering::Release);
            retire(
                &*self.shared.holders,
                &ticket,
                &Arc::downgrade(&self.shared),
            )?;
            return Err(error);
        }
        ticket.submit()?;
        match self
            .shared
            .holders
            .reserve(&ticket.start, previous, ticket.request)
            .map_err(holder_error)
        {
            Ok(reserved) => {
                let _ = ticket.reserved.set(reserved);
                Ok(ticket)
            }
            Err(error) => {
                if error.reply_unknown() {
                    ticket.unresolved.store(true, Ordering::Release);
                } else {
                    ticket.withdraw();
                    retire(
                        &*self.shared.holders,
                        &ticket,
                        &Arc::downgrade(&self.shared),
                    )?;
                }
                Err(error)
            }
        }
    }

    pub(super) fn cancel_start(&self, ticket: Ticket) -> Result<(), EngineError> {
        match cancel(&*self.shared.holders, &ticket) {
            Ok(()) => retire(
                &*self.shared.holders,
                &ticket,
                &Arc::downgrade(&self.shared),
            ),
            Err(error) => {
                ticket.unresolved.store(true, Ordering::Release);
                Err(EngineError::Unanswered(format!(
                    "the pane cancellation is unresolved: {error}"
                )))
            }
        }
    }

    /// Entries stay in the registry through a cancellation, so concurrent
    /// callers cannot mistake an in-flight resolution for absence.
    pub(super) fn resolve_unpublished(&self) -> Result<(), EngineError> {
        let pending: Vec<_> = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|entry| {
                entry.unresolved.load(Ordering::Acquire)
                    && (entry.retiring.load(Ordering::Acquire)
                        || !entry.drawn.load(Ordering::Acquire))
            })
            .cloned()
            .collect();
        for ticket in pending {
            if ticket.resolving.swap(true, Ordering::AcqRel) {
                return Err(EngineError::Unanswered(format!(
                    "pane {} is still resolving its earlier request",
                    ticket.start.pane
                )));
            }
            let result = if ticket.retiring.load(Ordering::Acquire) {
                retire(
                    &*self.shared.holders,
                    &ticket,
                    &Arc::downgrade(&self.shared),
                )
            } else {
                self.cancel_start(Arc::clone(&ticket))
            };
            ticket.resolving.store(false, Ordering::Release);
            result?;
        }
        Ok(())
    }

    /// Closing a parent includes creations not yet published in its layout.
    /// Cancel-before-reserve is durable even while that node request runs.
    pub(super) fn cancel_unpublished_for(&self, panes: &[String]) -> Result<(), EngineError> {
        let closing: Vec<String> = {
            let topology = self.topology();
            panes
                .iter()
                .filter_map(|pane| {
                    topology
                        .pane_place(pane)
                        .ok()
                        .map(|(workspace, _)| workspace)
                })
                .filter(|workspace| {
                    topology
                        .workspace_panes(workspace)
                        .is_ok_and(|held| held.iter().all(|pane| panes.contains(pane)))
                })
                .collect()
        };
        let pending: Vec<_> = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|entry| {
                !entry.drawn.load(Ordering::Acquire)
                    && (entry
                        .parent
                        .as_ref()
                        .is_some_and(|parent| panes.contains(parent))
                        || entry
                            .start
                            .pane
                            .split_once(':')
                            .is_some_and(|(workspace, _)| closing.iter().any(|id| id == workspace)))
            })
            .cloned()
            .collect();
        for ticket in pending {
            self.cancel_start(ticket)?;
        }
        Ok(())
    }

    pub(super) fn release_closed(&self, panes: &[String]) -> Result<(), EngineError> {
        let released: Vec<_> = {
            let pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            panes
                .iter()
                .filter_map(|pane| pending.get(pane).cloned())
                .collect()
        };
        for ticket in released {
            ticket.canceled.store(true, Ordering::Release);
            if !ticket.accepted.load(Ordering::Acquire) {
                ticket.withdraw();
            }
            retire(
                &*self.shared.holders,
                &ticket,
                &Arc::downgrade(&self.shared),
            )?;
        }
        Ok(())
    }
}
