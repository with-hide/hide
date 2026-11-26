//! Which connected clients are looking at a screen whose data costs the core a
//! probe.
//!
//! The core keeps one runtime-wide flag for the Settings agents tab
//! (`ai_settings.observing`): while it is set, the provider probe and the hook
//! diagnosis run on the coordinator. One browser tab closing that screen must
//! not stop another tab's, and a tab that disconnects without saying so must
//! not leave the probe running, so the daemon owns the flag and each
//! connection only owns its own demand.

use hide_core::links::ledger::BacklogOriginIdentity;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
pub struct ObservationDemand {
    observers: Mutex<HashSet<u64>>,
}

impl ObservationDemand {
    /// Records one connection's demand. When that changes the aggregate,
    /// `announce` receives the flag the core should now hold, called while the
    /// demand is still locked, so two transitions reach the core in the order
    /// they happened. Returns the flag it announced, if any.
    pub fn set(
        &self,
        connection: u64,
        observing: bool,
        announce: impl FnOnce(bool),
    ) -> Option<bool> {
        let mut observers = self.observers.lock().expect("observation demand");
        let before = !observers.is_empty();
        if observing {
            observers.insert(connection);
        } else {
            observers.remove(&connection);
        }
        let after = !observers.is_empty();
        let changed = (before != after).then_some(after);
        if let Some(flag) = changed {
            announce(flag);
        }
        changed
    }

    /// Drops whatever demand a closed connection held.
    pub fn release(&self, connection: u64, announce: impl FnOnce(bool)) -> Option<bool> {
        self.set(connection, false, announce)
    }

    #[cfg(test)]
    fn observers(&self) -> usize {
        self.observers.lock().expect("observation demand").len()
    }
}

/// Items a single window may hold the origin of at once: its panel and its
/// Start dialog, with room for a few left open.
pub const ORIGIN_HOLDS_PER_CONNECTION: usize = 8;

/// Which connections show a backlog item's origin (PRD overview-device-tasks
/// B31, B33). The core keeps an item's origin, with its turn's words, while
/// any window shows it: one window closing the item's panel must not empty
/// another's, and a window that disconnects without saying so must not keep
/// the words in the snapshot.
#[derive(Default)]
pub struct OriginHolds {
    holders: Mutex<HashMap<BacklogOriginIdentity, HashSet<u64>>>,
}

impl OriginHolds {
    /// Records that `connection` shows this owner's origin; refused for an
    /// invalid identity and past `ORIGIN_HOLDS_PER_CONNECTION` items.
    pub fn hold(&self, connection: u64, key: &BacklogOriginIdentity) -> Result<(), String> {
        key.validate()?;
        let mut holders = self.holders.lock().expect("origin holds");
        let held = holders
            .values()
            .filter(|connections| connections.contains(&connection))
            .count();
        let already = holders
            .get(key)
            .is_some_and(|connections| connections.contains(&connection));
        if !already && held >= ORIGIN_HOLDS_PER_CONNECTION {
            return Err(format!(
                "backlog_origin: one window holds at most {ORIGIN_HOLDS_PER_CONNECTION} items' origins"
            ));
        }
        holders
            .entry(key.to_owned())
            .or_default()
            .insert(connection);
        Ok(())
    }

    /// `connection` no longer shows `key`'s origin. When no connection does,
    /// `close` tells the core to drop it, called while the holds are still
    /// locked: another window's hold and read of the same item then reach the
    /// core after this close, never before it. Returns whether it closed.
    pub fn let_go(
        &self,
        connection: u64,
        key: &BacklogOriginIdentity,
        close: impl FnOnce(&BacklogOriginIdentity),
    ) -> bool {
        let mut holders = self.holders.lock().expect("origin holds");
        let Some(connections) = holders.get_mut(key) else {
            return false;
        };
        if !connections.remove(&connection) || !connections.is_empty() {
            return false;
        }
        holders.remove(key);
        close(key);
        true
    }

    /// Drops what a closed connection held, calling `close` under the lock,
    /// in key order, for each key no connection shows any more.
    pub fn release(&self, connection: u64, mut close: impl FnMut(&BacklogOriginIdentity)) {
        let mut holders = self.holders.lock().expect("origin holds");
        let mut emptied = Vec::new();
        holders.retain(|key, connections| {
            if connections.remove(&connection) && connections.is_empty() {
                emptied.push(key.clone());
                return false;
            }
            !connections.is_empty()
        });
        emptied.sort();
        for key in &emptied {
            close(key);
        }
    }
}

/// Which windows show the core's one session thread, and how many panels of
/// each (`session_thread_open`, `session_thread_close`). One window closing
/// its panel must not empty another's, and a window that reloads, crashes or
/// loses its socket says nothing, so the daemon counts each connection's holds
/// and tells the core to close only when the last one is let go or leaves.
/// The core holds one thread: another pane's open replaces it, and the holds
/// on the replaced one with it, so they can never close the new thread. A
/// tool draws no thread and holds none; its close closes the thread.
#[derive(Default)]
pub struct ThreadHolds {
    held: Mutex<HeldThread>,
}

#[derive(Default)]
struct HeldThread {
    pane: Option<String>,
    windows: HashMap<u64, usize>,
}

impl ThreadHolds {
    fn lock(&self) -> std::sync::MutexGuard<'_, HeldThread> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A window (`Some(connection)`) or a tool (`None`) opens `pane`'s
    /// thread. `dispatch` hands the open to the core while the holds are
    /// still locked, so the holds kept and the order the core hears agree; a
    /// refused dispatch changes nothing.
    pub fn open(
        &self,
        window: Option<u64>,
        pane: &str,
        dispatch: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let mut held = self.lock();
        dispatch()?;
        if held.pane.as_deref() != Some(pane) {
            held.pane = Some(pane.to_owned());
            held.windows.clear();
        }
        if let Some(connection) = window {
            *held.windows.entry(connection).or_default() += 1;
        }
        Ok(())
    }

    /// A window lets go of one hold (`Some(connection)`), or a tool closes
    /// the thread (`None`). The core hears the close, through `dispatch`
    /// under the lock, only when no window holds the thread any more.
    /// Returns whether it was told.
    pub fn close(
        &self,
        window: Option<u64>,
        dispatch: impl FnOnce() -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut held = self.lock();
        if let Some(connection) = window
            && let Some(holds) = held.windows.get_mut(&connection)
        {
            *holds -= 1;
            if *holds == 0 {
                held.windows.remove(&connection);
            }
        }
        if !held.windows.is_empty() {
            return Ok(false);
        }
        dispatch()?;
        *held = HeldThread::default();
        Ok(true)
    }

    /// Drops the holds of a closed connection; when they were the last, the
    /// thread is closed through `close`, called under the lock. Returns
    /// whether it was.
    pub fn release(&self, connection: u64, close: impl FnOnce()) -> bool {
        let mut held = self.lock();
        if held.windows.remove(&connection).is_none() || !held.windows.is_empty() {
            return false;
        }
        *held = HeldThread::default();
        close();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(key: &str) -> BacklogOriginIdentity {
        BacklogOriginIdentity {
            workspace_id: "project-a".into(),
            device_id: "device-a".into(),
            task_key: key.into(),
        }
    }

    #[test]
    fn a_session_thread_closes_only_when_the_last_window_showing_it_lets_go_or_leaves() {
        let ok = || Ok(());
        let holds = ThreadHolds::default();
        let mut closed = 0;

        // Two windows show the thread: one leaving keeps it for the other.
        holds.open(Some(1), "w1:p1", ok).unwrap();
        holds.open(Some(2), "w1:p1", ok).unwrap();
        assert!(!holds.release(1, || closed += 1), "another window shows it");
        assert!(holds.release(2, || closed += 1), "the last window left");
        assert!(!holds.release(2, || closed += 1), "closed once");
        assert_eq!(closed, 1);

        // Two panels of one window: closing one keeps the other's.
        holds.open(Some(3), "w1:p1", ok).unwrap();
        holds.open(Some(3), "w1:p1", ok).unwrap();
        assert_eq!(holds.close(Some(3), ok), Ok(false));
        assert_eq!(holds.close(Some(3), ok), Ok(true));

        // One window's panel closing does not empty another window's.
        holds.open(Some(4), "w1:p1", ok).unwrap();
        holds.open(Some(5), "w1:p1", ok).unwrap();
        assert_eq!(holds.close(Some(4), ok), Ok(false));
        assert!(holds.release(5, || closed += 1));
        assert_eq!(closed, 2);

        // Another pane's open replaces the thread: the replaced pane's
        // window leaving does not close the new one.
        holds.open(Some(6), "w1:p1", ok).unwrap();
        holds.open(Some(7), "w1:p2", ok).unwrap();
        assert!(!holds.release(6, || closed += 1));
        assert_eq!(holds.close(Some(6), ok), Ok(false), "it holds nothing now");
        assert!(holds.release(7, || closed += 1));
        assert_eq!(closed, 3);

        // A tool holds nothing, and its close reaches the core.
        holds.open(None, "w1:p1", ok).unwrap();
        assert!(!holds.release(8, || closed += 1));
        assert_eq!(holds.close(None, ok), Ok(true));

        // A refused dispatch keeps the holds it had.
        holds.open(Some(9), "w1:p1", ok).unwrap();
        assert!(
            holds
                .open(Some(10), "w1:p1", || Err("full".into()))
                .is_err()
        );
        assert!(!holds.release(10, || closed += 1));
        assert!(holds.release(9, || closed += 1));
        assert_eq!(closed, 4);
    }

    #[test]
    fn an_origin_is_let_go_only_when_the_last_window_showing_it_closes_or_leaves() {
        let holds = OriginHolds::default();
        holds.hold(1, &owner("github:acme/app#41")).unwrap();
        holds.hold(2, &owner("github:acme/app#41")).unwrap();
        holds.hold(2, &owner("github:acme/app#42")).unwrap();
        let closed = std::cell::RefCell::new(Vec::<BacklogOriginIdentity>::new());
        let close = |key: &BacklogOriginIdentity| closed.borrow_mut().push(key.clone());
        assert!(
            !holds.let_go(1, &owner("github:acme/app#41"), close),
            "window 2 still shows it"
        );
        assert!(
            !holds.let_go(1, &owner("github:acme/app#41"), close),
            "a repeated close is one"
        );
        assert!(closed.borrow().is_empty());
        holds.release(2, close);
        assert_eq!(
            *closed.borrow(),
            [owner("github:acme/app#41"), owner("github:acme/app#42")],
            "a window that disconnects lets go of everything it showed"
        );
        holds.release(2, close);
        assert_eq!(closed.borrow().len(), 2);
    }

    #[test]
    fn the_last_close_reaches_the_core_before_another_window_can_hold_the_item_again() {
        // Window 1 lets go while window 2 is about to open the same item; the
        // close is said before the holds are unlocked, so window 2's read can
        // only follow it and is never emptied by it.
        let holds = std::sync::Arc::new(OriginHolds::default());
        holds.hold(1, &owner("github:acme/app#41")).unwrap();
        let (entered, wait) = std::sync::mpsc::channel();
        let (finish, finished) = std::sync::mpsc::channel::<()>();
        let closing = {
            let holds = holds.clone();
            std::thread::spawn(move || {
                holds.let_go(1, &owner("github:acme/app#41"), |_| {
                    entered.send(()).unwrap();
                    finished.recv().unwrap();
                })
            })
        };
        wait.recv().unwrap();
        let (held, holding) = std::sync::mpsc::channel();
        let opener = {
            let holds = holds.clone();
            std::thread::spawn(move || {
                holds.hold(2, &owner("github:acme/app#41")).unwrap();
                held.send(()).unwrap();
            })
        };
        assert!(
            holding
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "window 2 holds the item only after window 1's close was said"
        );
        finish.send(()).unwrap();
        assert!(closing.join().unwrap());
        holding.recv().unwrap();
        opener.join().unwrap();
    }

    #[test]
    fn an_empty_or_overlong_item_key_is_refused() {
        let holds = OriginHolds::default();
        assert!(holds.hold(1, &owner("")).is_err());
        assert!(holds.hold(1, &owner(&"x".repeat(257))).is_err());
        holds.hold(1, &owner(&"x".repeat(256))).unwrap();
        let mut invalid = owner("local:/repo#1");
        invalid.workspace_id.clear();
        assert!(holds.hold(1, &invalid).is_err());
        invalid.workspace_id = "project-a".into();
        invalid.device_id.clear();
        assert!(holds.hold(1, &invalid).is_err());
    }

    #[test]
    fn one_window_holds_a_bounded_number_of_origins() {
        let holds = OriginHolds::default();
        for number in 0..ORIGIN_HOLDS_PER_CONNECTION {
            holds
                .hold(1, &owner(&format!("local:/notes#{number}")))
                .unwrap();
        }
        holds.hold(1, &owner("local:/notes#0")).unwrap();
        assert!(holds.hold(1, &owner("local:/notes#99")).is_err());
        let elsewhere = BacklogOriginIdentity {
            workspace_id: "project-b".into(),
            device_id: "device-b".into(),
            ..owner("local:/notes#0")
        };
        assert!(
            holds.hold(1, &elsewhere).is_err(),
            "a same-key owner also consumes one of the eight holds"
        );
        holds.hold(2, &owner("local:/notes#99")).unwrap();
    }

    #[test]
    fn closing_and_disconnecting_one_same_key_owner_keeps_the_other_owners_hold() {
        let holds = OriginHolds::default();
        let a = owner("local:/repo#1");
        let b = BacklogOriginIdentity {
            workspace_id: "project-b".into(),
            device_id: "device-b".into(),
            ..a.clone()
        };
        holds.hold(1, &a).unwrap();
        holds.hold(2, &b).unwrap();
        holds.hold(3, &b).unwrap();
        let closed = std::cell::RefCell::new(Vec::new());
        let close = |key: &BacklogOriginIdentity| closed.borrow_mut().push(key.clone());
        assert!(holds.let_go(1, &a, close));
        assert_eq!(*closed.borrow(), [a]);
        holds.release(2, close);
        assert_eq!(closed.borrow().len(), 1);
        holds.release(3, close);
        assert_eq!(closed.borrow().as_slice().last(), Some(&b));
    }

    #[test]
    fn the_flag_follows_the_first_observer_in_and_the_last_one_out() {
        let demand = ObservationDemand::default();
        let announced = std::cell::RefCell::new(Vec::new());
        let note = |flag| announced.borrow_mut().push(flag);
        assert_eq!(demand.set(1, true, note), Some(true));
        assert_eq!(demand.set(2, true, note), None);
        assert_eq!(
            demand.set(1, true, note),
            None,
            "a repeated open is one demand"
        );
        assert_eq!(
            demand.set(1, false, note),
            None,
            "another tab is still looking"
        );
        assert_eq!(demand.release(2, note), Some(false));
        assert_eq!(demand.observers(), 0);
        assert_eq!(
            *announced.borrow(),
            [true, false],
            "only transitions are announced"
        );
    }

    #[test]
    fn a_connection_that_never_observed_releases_nothing() {
        let demand = ObservationDemand::default();
        assert_eq!(demand.release(7, |_| panic!("nothing to announce")), None);
        assert_eq!(
            demand.set(3, false, |_| panic!("nothing to announce")),
            None
        );
    }
}
