//! One admitted creation through durable reserve, start and cancellation.
//! The same bounded intent retains its request, placement and capacity while
//! an answer is unknown. Resolution is caller driven, never a retry timer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use hide_node_link::holders::{HolderRequest, HolderReservation, HolderStart, PaneHolders};

use super::{EngineError, OwnEngine, Shared, holder_error, starts};
use crate::engine::Placement;

pub(super) type Ticket = Arc<Intent>;

pub(super) struct Intent {
    pub start: HolderStart,
    request: HolderRequest,
    state: Mutex<State>,
    placement_done: Condvar,
    retiring: AtomicBool,
    reserved: OnceLock<HolderReservation>,
    _turn: starts::Turn,
    placements: Vec<Arc<dyn Placement>>,
    parent: Option<String>,
    unresolved: AtomicBool,
    resolving: AtomicBool,
}

/// This intent alone owns placement callbacks. No mutex is held while they
/// run, and cancellation waits for their actual completion before withdrawal.
/// Publication takes this lock before the topology lock; closers release
/// topology before claiming cancellation here.
#[derive(Default)]
struct State {
    submission_closed: bool,
    drawn: bool,
    accepted: bool,
    canceled: bool,
    cancellation_confirmed: bool,
    placement: PlacementState,
}

#[derive(Default)]
enum PlacementState {
    #[default]
    Pending,
    Running,
    Ready(usize),
    Withdrawing,
    Withdrawn,
}

impl Intent {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn submit(&self) -> Result<(), EngineError> {
        if self.state().submission_closed {
            Err(EngineError::Unanswered(
                "the pane request has closed submission".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    fn cancelled(&self) -> EngineError {
        super::refused(
            "pane_creation_cancelled",
            format!(
                "pane {} was cancelled while being reserved",
                self.start.pane
            ),
        )
    }

    fn place(&self) -> Result<(), EngineError> {
        {
            let mut state = self.state();
            if state.canceled || state.submission_closed {
                return Err(self.cancelled());
            }
            state.placement = PlacementState::Running;
        }
        let mut placed = 0;
        let result = self.placements.iter().try_for_each(|placement| {
            self.check_open()?;
            placement.place(&self.start.pane)?;
            placed += 1;
            Ok(())
        });
        self.state().placement = PlacementState::Ready(placed);
        self.placement_done.notify_all();
        result.and_then(|()| self.check_open())
    }

    fn withdraw(&self) -> Result<(), EngineError> {
        let placed = {
            let state = self.state();
            let (mut state, _) = self
                .placement_done
                .wait_timeout_while(state, hide_kit::panes::START_LIMIT, |state| {
                    matches!(
                        state.placement,
                        PlacementState::Running | PlacementState::Withdrawing
                    )
                })
                .unwrap_or_else(PoisonError::into_inner);
            match state.placement {
                PlacementState::Ready(placed) => {
                    state.placement = PlacementState::Withdrawing;
                    placed
                }
                PlacementState::Pending | PlacementState::Withdrawn => {
                    state.placement = PlacementState::Withdrawn;
                    return Ok(());
                }
                PlacementState::Running | PlacementState::Withdrawing => {
                    self.unresolved.store(true, Ordering::Release);
                    return Err(EngineError::Unanswered(
                        "the pane placement has not finished its cleanup".to_owned(),
                    ));
                }
            }
        };
        for placement in &self.placements[..placed] {
            placement.withdraw(&self.start.pane);
        }
        self.state().placement = PlacementState::Withdrawn;
        self.placement_done.notify_all();
        Ok(())
    }

    /// Selection and the cancellation claim are one transition, not a read
    /// of drawn followed later by an unconditional cancellation.
    fn claim_cancel(&self, only_unpublished: bool) -> bool {
        let mut state = self.state();
        if only_unpublished && state.drawn {
            return false;
        }
        state.canceled = true;
        true
    }

    pub fn check_open(&self) -> Result<(), EngineError> {
        if self.state().canceled {
            Err(self.cancelled())
        } else {
            Ok(())
        }
    }

    pub fn accepted(&self) -> bool {
        self.state().accepted
    }

    pub fn publish<R>(
        &self,
        accepted: bool,
        add: impl FnOnce() -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let mut state = self.state();
        if state.canceled || state.submission_closed {
            return Err(self.cancelled());
        }
        let result = add()?;
        state.drawn = true;
        state.accepted = accepted;
        Ok(result)
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
    ticket.state().submission_closed = true;
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
    ticket.claim_cancel(false);
    if ticket.state().cancellation_confirmed {
        return ticket.withdraw();
    }
    ticket.submit()?;
    let result = holders.cancel(
        &ticket.start.intent().map_err(holder_error)?,
        ticket.request,
    );
    let mut state = ticket.state();
    // Another cancellation may have confirmed and retired while this call
    // was at the node. Its confirmed decision remains authoritative.
    if !state.cancellation_confirmed {
        result.map_err(holder_error)?;
        state.cancellation_confirmed = true;
    }
    drop(state);
    ticket.withdraw()
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
        && !ticket.accepted()
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
        let ticket = self.admit_start(start, place, parent)?;
        self.reserve_admitted(ticket, previous)
    }

    pub(super) fn admit_start(
        &self,
        start: HolderStart,
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
            state: Mutex::default(),
            placement_done: Condvar::new(),
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
            unresolved: AtomicBool::new(false),
            resolving: AtomicBool::new(false),
        });
        pending.insert(ticket.start.pane.clone(), Arc::clone(&ticket));
        drop(pending);
        Ok(ticket)
    }

    pub(super) fn reserve_admitted(
        &self,
        ticket: Ticket,
        previous: Option<u64>,
    ) -> Result<Ticket, EngineError> {
        self.reserve_admitted_by(ticket, |holders, start, request| {
            holders.reserve(start, previous, request)
        })
    }

    /// [`Self::reserve_admitted`] with the node call that makes the
    /// reservation named: a plain reserve, or a holder's replacement that
    /// ends the earlier holder first ([`PaneHolders::replace`]).
    pub(super) fn reserve_admitted_by(
        &self,
        ticket: Ticket,
        reserve: impl FnOnce(
            &dyn PaneHolders,
            &HolderStart,
            HolderRequest,
        ) -> Result<HolderReservation, hide_node_link::holders::HolderError>,
    ) -> Result<Ticket, EngineError> {
        if let Err(error) = ticket.place() {
            ticket.withdraw()?;
            retire(
                &*self.shared.holders,
                &ticket,
                &Arc::downgrade(&self.shared),
            )?;
            return Err(error);
        }
        ticket.submit()?;
        match reserve(&*self.shared.holders, &ticket.start, ticket.request).map_err(holder_error) {
            Ok(reserved) => {
                let _ = ticket.reserved.set(reserved);
                Ok(ticket)
            }
            Err(error) => {
                if error.reply_unknown() {
                    ticket.unresolved.store(true, Ordering::Release);
                } else {
                    ticket.withdraw()?;
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
            .cloned()
            .collect();
        for ticket in pending {
            if !ticket.unresolved.load(Ordering::Acquire)
                || (!ticket.retiring.load(Ordering::Acquire) && ticket.state().drawn)
            {
                continue;
            }
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
                entry
                    .parent
                    .as_ref()
                    .is_some_and(|parent| panes.contains(parent))
                    || entry
                        .start
                        .pane
                        .split_once(':')
                        .is_some_and(|(workspace, _)| closing.iter().any(|id| id == workspace))
            })
            .cloned()
            .collect();
        for ticket in pending {
            if ticket.claim_cancel(true) {
                self.cancel_start(ticket)?;
            }
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
            let accepted = {
                let mut state = ticket.state();
                state.canceled = true;
                state.cancellation_confirmed = true;
                state.accepted
            };
            if !accepted {
                ticket.withdraw()?;
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
