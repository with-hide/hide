//! The one request budget of the signed-in gitlab.com account.
//!
//! gitlab.com allows the account 100 API requests a minute, and the
//! operator's own `glab` in a terminal counts against the same 100. Every
//! `glab` line Hide runs that reaches gitlab.com (the sign-in check and every
//! `glab api` call; `glab --version` asks nothing) takes one request from
//! this budget first, for the whole core process: every reader, every
//! device's projects, the list, the card and the exact-commit reads.
//!
//! What it counts: requests taken in the last minute, at most
//! [`HIDE_PER_MINUTE`] (60), which leaves 40 for the operator's own `glab`.
//! Background reads (the list and the sign-in check of a pass) take at most
//! [`BACKGROUND_PER_MINUTE`] (45) of them, so the 15 left are reserved for
//! reads the operator asked for by opening something (a card, a checkout's
//! commits).
//!
//! When it is full: a background read is not made this pass (the project
//! keeps its previous answer, with no failure clock); a read the operator
//! opened is refused with its reason only when its reserved share is spent
//! too. A merge the operator asked for takes its read, its write and its
//! reads after from the opened share together, or is refused before
//! anything leaves. The reads a mission's waiting node and a worktree's
//! cleanup make on the core's own clock (a merge request by its number, a
//! branch's merged merge requests, a merge base) are background reads. The record holds at most [`HIDE_PER_MINUTE`] entries, the oldest
//! dropped as they leave the minute.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The requests a minute Hide takes of the account's 100.
pub(crate) const HIDE_PER_MINUTE: usize = 60;

/// The requests a minute background reads take of [`HIDE_PER_MINUTE`].
pub(crate) const BACKGROUND_PER_MINUTE: usize = 45;

const WINDOW: Duration = Duration::from_secs(60);

/// Who asks for a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Share {
    /// A read the core makes on its own clock.
    Background,
    /// A read the operator asked for by opening something.
    Opened,
}

/// The requests taken in the last minute, oldest first.
#[derive(Debug, Default)]
pub(crate) struct Budget {
    taken: VecDeque<(Instant, Share)>,
}

impl Budget {
    pub(crate) const fn new() -> Self {
        Self {
            taken: VecDeque::new(),
        }
    }

    /// Takes one request for `share` at `now`, or says the share is spent.
    pub(crate) fn take(&mut self, now: Instant, share: Share) -> bool {
        self.take_many(now, share, 1)
    }

    /// Takes `count` requests for `share` at `now` together, or none and
    /// says the share cannot hold them: a merge takes its read, its write
    /// and its reads after at once, so a write never leaves without the
    /// read that decides it.
    pub(crate) fn take_many(&mut self, now: Instant, share: Share, count: usize) -> bool {
        while self
            .taken
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= WINDOW)
        {
            self.taken.pop_front();
        }
        let background = self
            .taken
            .iter()
            .filter(|(_, taken)| *taken == Share::Background)
            .count();
        let total = self.taken.len() + count;
        let room = match share {
            Share::Background => {
                background + count <= BACKGROUND_PER_MINUTE && total <= HIDE_PER_MINUTE
            }
            Share::Opened => total <= HIDE_PER_MINUTE,
        };
        if room {
            self.taken.extend(std::iter::repeat_n((now, share), count));
        }
        room
    }
}

/// The budget every `glab` request of this process takes from.
pub(crate) static ACCOUNT: Mutex<Budget> = Mutex::new(Budget::new());

/// Takes one request for `share` from the process's budget now.
pub(crate) fn take(share: Share) -> bool {
    ACCOUNT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take(Instant::now(), share)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_reads_leave_the_opened_share_and_the_minute_resets_it() {
        let start = Instant::now();
        let mut budget = Budget::new();
        let taken = (0..100)
            .filter(|_| budget.take(start, Share::Background))
            .count();
        assert_eq!(taken, BACKGROUND_PER_MINUTE);
        let opened = (0..100)
            .filter(|_| budget.take(start, Share::Opened))
            .count();
        assert_eq!(opened, HIDE_PER_MINUTE - BACKGROUND_PER_MINUTE);
        assert!(!budget.take(start + Duration::from_secs(59), Share::Opened));
        // A minute later every request has left the window.
        let later = start + WINDOW;
        assert!(budget.take(later, Share::Background));
        assert_eq!(budget.taken.len(), 1);
    }

    /// A merge's requests are taken together or not at all: with two left,
    /// a merge that needs four takes none and leaves the two.
    #[test]
    fn requests_taken_together_are_all_or_none() {
        let start = Instant::now();
        let mut budget = Budget::new();
        assert!(budget.take_many(start, Share::Opened, HIDE_PER_MINUTE - 2));
        assert!(!budget.take_many(start, Share::Opened, 4));
        assert_eq!(budget.taken.len(), HIDE_PER_MINUTE - 2);
        assert!(budget.take_many(start, Share::Opened, 2));
        assert!(!budget.take(start, Share::Opened));
    }

    #[test]
    fn opened_reads_may_take_the_whole_budget_and_then_starve_background() {
        let start = Instant::now();
        let mut budget = Budget::new();
        let opened = (0..100)
            .filter(|_| budget.take(start, Share::Opened))
            .count();
        assert_eq!(opened, HIDE_PER_MINUTE);
        assert!(!budget.take(start, Share::Background));
    }

    /// Several readers at once (each device's coordinator runs its own) take
    /// from the one budget and never pass it together.
    #[test]
    fn readers_at_once_share_one_budget() {
        let budget = Mutex::new(Budget::new());
        let start = Instant::now();
        let taken: usize = std::thread::scope(|scope| {
            let readers: Vec<_> = (0..4)
                .map(|index| {
                    let budget = &budget;
                    scope.spawn(move || {
                        let share = if index == 0 {
                            Share::Opened
                        } else {
                            Share::Background
                        };
                        (0..40)
                            .filter(|_| budget.lock().unwrap().take(start, share))
                            .count()
                    })
                })
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().unwrap())
                .sum()
        });
        assert_eq!(taken, HIDE_PER_MINUTE);
        let background = budget
            .lock()
            .unwrap()
            .taken
            .iter()
            .filter(|(_, share)| *share == Share::Background)
            .count();
        assert!(background <= BACKGROUND_PER_MINUTE);
    }
}
