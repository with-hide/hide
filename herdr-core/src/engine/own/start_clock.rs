//! The thread that tells the core when an agent's pending start ends on its
//! own. A start stays pending (`launch_pending`) until its settle has passed
//! and its screen rests, or until its deadline; when the screen already rests,
//! nothing the holder reports changes at that moment, so without this clock
//! the core would keep reading the start pending, and its row starting,
//! until something else changed. Each read of the session arms the clock
//! with the next moment a pending start can end ([`StartClock::arm`]); at
//! that moment it wakes whoever watches the session, whose read arms it
//! again.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::engine::Wake;

#[derive(Default)]
struct Due {
    at: Option<Instant>,
    ended: bool,
}

type Shared = Arc<(Mutex<Due>, Condvar)>;

pub(super) struct StartClock {
    due: Shared,
    thread: Option<JoinHandle<()>>,
}

impl StartClock {
    /// Starts the clock, which wakes through `wake` (the engine's watcher,
    /// once one is set).
    pub(super) fn start(wake: Arc<Mutex<Option<Wake>>>) -> Result<Self, String> {
        let due: Shared = Arc::default();
        let waiting = Arc::clone(&due);
        let thread = std::thread::Builder::new()
            .name("hide-own-engine-start-clock".into())
            .spawn(move || tick_loop(&waiting, &wake))
            .map_err(|error| format!("the agent start clock could not start: {error}"))?;
        Ok(Self {
            due,
            thread: Some(thread),
        })
    }

    /// Wakes the watcher at `at`, in place of any moment armed before;
    /// `None` when no start is pending.
    pub(super) fn arm(&self, at: Option<Instant>) {
        let (due, changed) = &*self.due;
        let mut due = due.lock().unwrap_or_else(PoisonError::into_inner);
        if due.ended || due.at == at {
            return;
        }
        due.at = at;
        drop(due);
        changed.notify_one();
    }

    #[cfg(test)]
    pub(super) fn armed(&self) -> Option<Instant> {
        self.due.0.lock().unwrap_or_else(PoisonError::into_inner).at
    }
}

impl Drop for StartClock {
    fn drop(&mut self) {
        let (due, changed) = &*self.due;
        due.lock().unwrap_or_else(PoisonError::into_inner).ended = true;
        changed.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn tick_loop(due: &Shared, wake: &Mutex<Option<Wake>>) {
    let (due, changed) = &**due;
    let mut held = due.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        if held.ended {
            return;
        }
        let Some(at) = held.at else {
            held = changed.wait(held).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if now < at {
            held = changed
                .wait_timeout(held, at - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        held.at = None;
        drop(held);
        let watcher = wake.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(watcher) = watcher {
            watcher();
        }
        held = due.lock().unwrap_or_else(PoisonError::into_inner);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_watcher_is_woken_once_the_armed_moment_passes_and_not_before() {
        let (woke, woken) = channel();
        let woke = Mutex::new(woke);
        let wake: Wake = Arc::new(move || {
            let _ = woke.lock().unwrap().send(Instant::now());
        });
        let clock = StartClock::start(Arc::new(Mutex::new(Some(wake)))).unwrap();
        let at = Instant::now() + Duration::from_millis(200);
        clock.arm(Some(at));
        let when = woken.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(when >= at, "woken before the armed moment");
        assert_eq!(clock.armed(), None, "a moment wakes once");
        assert!(woken.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn a_moment_armed_again_replaces_the_one_before_and_none_disarms() {
        let (woke, woken) = channel();
        let woke = Mutex::new(woke);
        let wake: Wake = Arc::new(move || {
            let _ = woke.lock().unwrap().send(());
        });
        let clock = StartClock::start(Arc::new(Mutex::new(Some(wake)))).unwrap();
        clock.arm(Some(Instant::now() + Duration::from_millis(150)));
        clock.arm(None);
        assert!(woken.recv_timeout(Duration::from_millis(400)).is_err());
        clock.arm(Some(Instant::now() + Duration::from_secs(60)));
        clock.arm(Some(Instant::now() + Duration::from_millis(50)));
        woken.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
