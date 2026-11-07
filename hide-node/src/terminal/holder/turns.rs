//! One change of a pane's holder at a time (with-hide/hide-private#646): a
//! renewal, which ends the pane's holder and starts the next one, and a
//! close, which ends whichever holder runs the pane then, each take the
//! pane's turn and keep it until their last effect is done, so a close that
//! comes during a renewal ends the holder the renewal left, and a renewal
//! that comes after a close finds the pane closed. Turns are given in the
//! order they were asked for, and a caller that stops waiting gives up its
//! place without holding up the ones behind it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The most callers waiting for one pane's turn, beside the one that has it.
pub(super) const TURN_WAITERS: u64 = 8;

/// The line for one pane's turn.
#[derive(Default)]
struct Line {
    /// The ticket the next caller takes.
    next: u64,
    /// The ticket whose turn it is.
    serving: u64,
    /// Tickets whose callers stopped waiting before their turn came.
    gone: BTreeSet<u64>,
}

impl Line {
    /// Moves the turn on past the tickets nobody waits on.
    fn advance(&mut self) {
        self.serving += 1;
        while self.gone.remove(&self.serving) {
            self.serving += 1;
        }
    }

    fn empty(&self) -> bool {
        self.serving == self.next
    }
}

/// Why a turn was not given.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TurnRefused {
    /// [`TURN_WAITERS`] callers already wait for the pane.
    Full,
    /// The wait's bound passed before the turn came.
    Expired,
}

/// Every pane's line; an entry lives while anyone has or waits for the
/// pane's turn, so there are never more than the panes being changed.
#[derive(Default)]
pub(super) struct Turns {
    lines: Mutex<BTreeMap<String, Line>>,
    moved: Condvar,
}

/// A pane's turn, given up when dropped, on every way out.
pub(super) struct Turn<'a> {
    turns: &'a Turns,
    pane: String,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut lines = self.turns.lock();
        if let Some(line) = lines.get_mut(&self.pane) {
            line.advance();
            if line.empty() {
                lines.remove(&self.pane);
            }
        }
        drop(lines);
        self.turns.moved.notify_all();
    }
}

impl Turns {
    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Line>> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits up to `within` for `pane`'s turn, behind every caller that asked
    /// before.
    pub(super) fn take(&self, pane: &str, within: Duration) -> Result<Turn<'_>, TurnRefused> {
        let deadline = Instant::now() + within;
        let mut lines = self.lock();
        let line = lines.entry(pane.to_owned()).or_default();
        if line.next - line.serving > TURN_WAITERS {
            return Err(TurnRefused::Full);
        }
        let ticket = line.next;
        line.next += 1;
        loop {
            let line = lines.get_mut(pane).expect("a waiter keeps its line");
            if line.serving == ticket {
                return Ok(Turn {
                    turns: self,
                    pane: pane.to_owned(),
                });
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                line.gone.insert(ticket);
                return Err(TurnRefused::Expired);
            };
            lines = self
                .moved
                .wait_timeout(lines, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Whether anyone has `pane`'s turn now.
    pub(super) fn taken(&self, pane: &str) -> bool {
        self.lock().contains_key(pane)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::thread;

    use super::*;

    const LONG: Duration = Duration::from_secs(5);

    #[test]
    fn turns_are_given_in_the_order_they_were_asked_and_the_line_goes_with_the_last() {
        let turns = Turns::default();
        let first = turns.take("p", LONG).unwrap();
        let (told, order) = channel();
        thread::scope(|scope| {
            for who in ["second", "third"] {
                let told = told.clone();
                let turns = &turns;
                scope.spawn(move || {
                    let _turn = turns.take("p", LONG).unwrap();
                    told.send(who).unwrap();
                });
                // The second asks before the third.
                while turns.lock()["p"].next < if who == "second" { 2 } else { 3 } {
                    thread::yield_now();
                }
            }
            assert!(
                turns.take("other", LONG).is_ok(),
                "another pane waits on none"
            );
            drop(first);
        });
        assert_eq!(order.try_iter().collect::<Vec<_>>(), ["second", "third"]);
        assert!(!turns.taken("p"));
    }

    #[test]
    fn a_caller_that_stops_waiting_holds_up_nobody_behind_it() {
        let turns = Turns::default();
        let first = turns.take("p", LONG).unwrap();
        assert_eq!(
            turns.take("p", Duration::from_millis(10)).err(),
            Some(TurnRefused::Expired)
        );
        thread::scope(|scope| {
            let waiting = scope.spawn(|| turns.take("p", LONG).map(drop));
            while turns.lock()["p"].next < 3 {
                thread::yield_now();
            }
            drop(first);
            assert_eq!(waiting.join().unwrap(), Ok(()));
        });
        assert!(!turns.taken("p"));
    }

    #[test]
    fn a_full_line_refuses_the_next_caller() {
        let turns = Turns::default();
        let _first = turns.take("p", LONG).unwrap();
        for _ in 0..TURN_WAITERS {
            assert_eq!(
                turns.take("p", Duration::ZERO).err(),
                Some(TurnRefused::Expired)
            );
        }
        // Expired callers left their tickets behind, gone; the line still
        // counts them until the turn passes them.
        assert_eq!(
            turns.take("p", Duration::ZERO).err(),
            Some(TurnRefused::Full)
        );
    }
}
