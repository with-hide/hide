//! One connection's bounded, numbered creation requests. A completed number
//! is represented by absence below `high`, not by a growing tombstone list.
//! Admission and retirement share this mutex; I/O never holds it.

use std::collections::BTreeMap;
use std::sync::PoisonError;

use super::{HOLDER_CAP, Owner};

#[derive(Default, Debug)]
pub(super) struct Requests {
    high: u64,
    pending: BTreeMap<u64, Pending>,
}

#[derive(Default, Debug)]
struct Pending {
    intent: Option<(String, String)>,
    workers: usize,
    retiring: bool,
}

/// Keeps an admitted operation alive until its last mutation and cleanup,
/// including after its answer is lost. Dropping it never performs I/O.
pub struct Request<'a> {
    owner: &'a Owner,
    number: u64,
}

impl Requests {
    fn reach(&mut self, number: u64) -> Result<(), String> {
        if number == 0 {
            return Err("pane request zero is not issued".to_owned());
        }
        if number <= self.high {
            return Ok(());
        }
        let added = number - self.high;
        if added > (HOLDER_CAP - self.pending.len()) as u64 {
            return Err("too many unresolved pane requests".to_owned());
        }
        // The subtraction above proves this range has at most HOLDER_CAP
        // entries, even for a hostile u64::MAX request number.
        for next in self.high + 1..=number {
            self.pending.insert(next, Pending::default());
        }
        self.high = number;
        Ok(())
    }

    /// The next number, pending with no intent until its first admission.
    fn next(&mut self) -> Result<u64, String> {
        let number = self
            .high
            .checked_add(1)
            .ok_or_else(|| "pane request numbers are exhausted".to_owned())?;
        self.reach(number)?;
        Ok(number)
    }
}

impl Owner {
    /// A new request number of this incarnation, which its first admission
    /// binds to an intent. It is the one numbering of the connection, so a
    /// number is never issued twice; one never sent must still be retired.
    pub fn number(&self) -> Result<u64, String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next()
    }

    /// Synchronous library callers own one whole request, without a wire
    /// sender. They use the same admission and retirement state machine.
    pub fn issue(&self, pane: &str, digest: &str) -> Result<Request<'_>, String> {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let number = requests.next()?;
        let pending = requests
            .pending
            .get_mut(&number)
            .expect("the new number was admitted");
        pending.intent = Some((pane.to_owned(), digest.to_owned()));
        pending.workers = 1;
        Ok(Request {
            owner: self,
            number,
        })
    }

    /// Bind a number to one original intent before any reserve/start/cancel
    /// work. The owner number must be the one this connection recovered.
    pub fn admit(
        &self,
        owner: u64,
        number: u64,
        pane: &str,
        digest: &str,
    ) -> Result<Request<'_>, String> {
        self.request_owner(owner)?;
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.reach(number)?;
        let pending = requests
            .pending
            .get_mut(&number)
            .ok_or_else(|| "the pane request has already finished".to_owned())?;
        if pending.retiring {
            return Err("the pane request is retiring".to_owned());
        }
        match &pending.intent {
            Some((given_pane, given_digest)) if given_pane != pane || given_digest != digest => {
                return Err("the pane request names a different intent".to_owned());
            }
            None => pending.intent = Some((pane.to_owned(), digest.to_owned())),
            _ => {}
        }
        pending.workers = pending
            .workers
            .checked_add(1)
            .ok_or_else(|| "too many workers for one pane request".to_owned())?;
        Ok(Request {
            owner: self,
            number,
        })
    }

    /// Close admission first. A false answer means an actual worker still
    /// owns cleanup, not that the caller may drop its unresolved intent.
    /// Repeating retirement of an already completed number converges.
    pub fn retire(&self, owner: u64, number: u64) -> Result<bool, String> {
        self.request_owner(owner)?;
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.reach(number)?;
        let Some(pending) = requests.pending.get_mut(&number) else {
            return Ok(true);
        };
        pending.retiring = true;
        if pending.workers != 0 {
            return Ok(false);
        }
        requests.pending.remove(&number);
        Ok(true)
    }

    pub(super) fn request_pending(&self, number: u64) -> bool {
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        // A pre-numbered record remains pinned until owner turnover. A new
        // numbered record can only have been written after admission.
        number == 0 || requests.pending.contains_key(&number)
    }

    fn request_owner(&self, owner: u64) -> Result<(), String> {
        if owner == self.incarnation() {
            Ok(())
        } else {
            Err("the pane request belongs to an earlier connection".to_owned())
        }
    }
}

impl Request<'_> {
    pub fn number(&self) -> u64 {
        self.number
    }
    pub fn owner(&self) -> &Owner {
        self.owner
    }
}

impl Drop for Request<'_> {
    fn drop(&mut self) {
        let mut requests = self
            .owner
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let pending = requests
            .pending
            .get_mut(&self.number)
            .expect("an admitted request stays until its last worker leaves");
        pending.workers -= 1;
        if pending.retiring && pending.workers == 0 {
            requests.pending.remove(&self.number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::Panes;
    use std::time::Duration;

    #[test]
    fn completion_reclaims_capacity_and_old_requests_cannot_reenter() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        for n in 1..=HOLDER_CAP as u64 * 3 {
            let worker = owner.admit(owner.incarnation(), n, "p", "digest").unwrap();
            assert!(!owner.retire(owner.incarnation(), n).unwrap());
            assert!(owner.admit(owner.incarnation(), n, "p", "digest").is_err());
            drop(worker);
            assert!(owner.retire(owner.incarnation(), n).unwrap());
            assert!(owner.admit(owner.incarnation(), n, "p", "digest").is_err());
        }
        assert!(owner.requests.lock().unwrap().pending.is_empty());
        assert!(owner.admit(owner.incarnation(), 1, "p", "digest").is_err());
    }

    #[test]
    fn no_send_gaps_retire_and_a_refused_jump_does_not_advance_the_window() {
        let home = tempfile::tempdir().unwrap();
        let panes = Panes::open(home.path(), &home.path().join("state"), |_| {}).unwrap();
        let owner = panes.acquire(Duration::ZERO).unwrap();
        let number = owner.incarnation();
        let second = owner.admit(number, 2, "p2", "digest").unwrap();
        assert!(owner.retire(number, 1).unwrap()); // issued, definitely not sent
        assert!(owner.admit(number, u64::MAX, "p", "digest").is_err());
        assert_eq!(owner.requests.lock().unwrap().high, 2);
        drop(second);
        for n in 3..=HOLDER_CAP as u64 + 1 {
            drop(owner.admit(number, n, "p", "digest").unwrap());
        }
        assert!(
            owner
                .admit(number, HOLDER_CAP as u64 + 2, "p", "digest")
                .is_err()
        );
        assert_eq!(owner.requests.lock().unwrap().pending.len(), HOLDER_CAP);
        assert!(owner.retire(number, 2).unwrap());
        assert!(
            owner
                .admit(number, HOLDER_CAP as u64 + 2, "p", "digest")
                .is_ok()
        );
        assert!(owner.admit(number + 1, 3, "p", "digest").is_err());
    }
}
