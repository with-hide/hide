//! One connection's authority to mutate a state folder's pane records.
//! An admitted worker borrows this authority through its last write; the
//! connection and its workers share the guard, never independently unlock it.

use std::time::Duration;

use hide_platform::fs::lock::Lock;

use super::Panes;
use crate::layout;

/// Held until the connection and all of its admitted node work have ended.
/// Holder processes use their record incarnation and do not own this lock.
#[derive(Debug)]
pub struct Owner {
    identity: String,
    incarnation: u64,
    _lock: Lock,
    pub(super) requests: std::sync::Mutex<super::requests::Requests>,
}

impl Owner {
    pub fn incarnation(&self) -> u64 {
        self.incarnation
    }
}

impl Panes {
    /// Acquire only off runtime/admission locks. A timeout proves nothing
    /// about earlier work and never permits reconciliation or ID allocation.
    pub fn acquire(&self, within: Duration) -> Result<Owner, String> {
        let lock =
            hide_holder_protocol::record::lock(&layout::pane_owner_lock(&self.state_dir), within)
                .map_err(|error| format!("pane ownership could not be acquired: {error}"))?
                .ok_or_else(|| "the previous pane owner still has work in flight".to_owned())?;
        // Even crash repair writes records. It follows ownership acquisition,
        // before a new connection reads the recovered set or accepts starts.
        self.mend_after_a_crash()?;
        let _records = self.lock()?;
        Ok(Owner {
            identity: self.owner.clone(),
            incarnation: self.next_incarnation()?,
            _lock: lock,
            requests: std::sync::Mutex::default(),
        })
    }

    pub(super) fn check_owner(&self, owner: &Owner) -> Result<(), String> {
        if owner.identity == self.owner {
            Ok(())
        } else {
            Err("the pane ownership belongs to another state folder".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, mpsc};

    use super::*;

    #[test]
    fn a_draining_worker_keeps_the_next_connection_out() {
        let home = tempfile::tempdir().unwrap();
        let state = home.path().join("state");
        let previous = Arc::new(Panes::open(home.path(), &state, |_| {}).unwrap());
        let owner = Arc::new(previous.acquire(Duration::ZERO).unwrap());
        let original = owner.incarnation();
        let next = Panes::open(home.path(), &state, |_| {}).unwrap();
        let (entered, waiting) = mpsc::sync_channel(0);
        let (finish, released) = mpsc::sync_channel(0);
        let worker_owner = Arc::clone(&owner);
        let worker = std::thread::spawn(move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
            // The request still owns its records after its caller went away.
            previous.reconcile(&worker_owner, Duration::ZERO)
        });
        waiting.recv().unwrap();
        drop(owner);
        assert!(next.acquire(Duration::ZERO).is_err());
        finish.send(()).unwrap();
        assert!(worker.join().unwrap().failed.is_none());
        let owner = next.acquire(Duration::ZERO).unwrap();
        assert_ne!(owner.incarnation(), original);
    }

    #[test]
    fn another_state_folders_owner_cannot_authorize_reconciliation() {
        let home = tempfile::tempdir().unwrap();
        let first = Panes::open(home.path(), &home.path().join("first"), |_| {}).unwrap();
        let second = Panes::open(home.path(), &home.path().join("second"), |_| {}).unwrap();
        let owner = first.acquire(Duration::ZERO).unwrap();
        assert!(second.reconcile(&owner, Duration::ZERO).failed.is_some());
    }
}
