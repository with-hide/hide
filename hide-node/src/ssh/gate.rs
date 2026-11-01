//! A bounded number of attempts that run at once in this process: the ssh
//! dials every device shares and the `wsl.exe` calls every distribution
//! shares are each one [`Gate`].

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::lock_recover;

pub(crate) struct Gate {
    taken: Mutex<usize>,
    freed: Condvar,
    cap: usize,
}

/// One admitted attempt; its end frees its place.
pub(crate) struct Slot(&'static Gate);

impl Gate {
    pub(crate) const fn new(cap: usize) -> Self {
        Self {
            taken: Mutex::new(0),
            freed: Condvar::new(),
            cap,
        }
    }

    /// Waits for a place until `deadline`; none past it, or once
    /// `cancelled` answers true, which is asked at least once a second while
    /// the attempt waits.
    pub(crate) fn admit(
        &'static self,
        deadline: Instant,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Option<Slot> {
        let is_cancelled = || cancelled.is_some_and(|cancelled| cancelled());
        let mut taken = lock_recover(&self.taken);
        while *taken >= self.cap {
            if is_cancelled() {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let wait = match cancelled {
                Some(_) => left.min(Duration::from_secs(1)),
                None => left,
            };
            taken = self
                .freed
                .wait_timeout(taken, wait)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
        if is_cancelled() {
            return None;
        }
        *taken += 1;
        Some(Slot(self))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        *lock_recover(&self.0.taken) -= 1;
        self.0.freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn gate(cap: usize) -> &'static Gate {
        Box::leak(Box::new(Gate::new(cap)))
    }

    #[test]
    fn an_attempt_past_the_cap_waits_until_its_deadline_and_one_that_ends_frees_a_place() {
        let gate = gate(4);
        let mut held: Vec<_> = (0..4)
            .map(|_| gate.admit(Instant::now(), None).expect("a free place"))
            .collect();
        let asked = Instant::now();
        assert!(
            gate.admit(asked + Duration::from_millis(50), None)
                .is_none(),
            "an attempt past the cap was admitted"
        );
        assert!(asked.elapsed() >= Duration::from_millis(50));
        held.pop();
        assert!(gate.admit(Instant::now(), None).is_some());
    }

    #[test]
    fn a_cancelled_attempt_leaves_the_queue_before_its_deadline_and_takes_no_place() {
        let gate = gate(1);
        let held = gate.admit(Instant::now(), None).expect("a free place");
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting = {
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                let cancelled = || cancel.load(Ordering::SeqCst);
                gate.admit(Instant::now() + Duration::from_secs(600), Some(&cancelled))
                    .is_some()
            })
        };
        cancel.store(true, Ordering::SeqCst);
        assert!(!waiting.join().unwrap(), "a cancelled attempt was admitted");
        drop(held);
        // The cancelled attempt took nothing: the one place is free again.
        let first = gate.admit(Instant::now(), None).expect("the freed place");
        assert!(gate.admit(Instant::now(), None).is_none());
        drop(first);
    }
}
