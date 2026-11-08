//! The wait of a profile's start for `hide agent exec` (PRD agent-profiles
//! B22, D-26).
//!
//! A profile's start line runs `hide agent exec`, which reports through the
//! pane's hook connection (`HookCommand::AgentExec`) whether it handed the
//! pane to the agent's program or why that program could not start. Typing
//! the line proves neither, so the start waits for that report: a program
//! that cannot start fails the start that typed it, with the sentence that
//! names the profile and the program, instead of leaving a shell behind a
//! start that reads as made. The executor checks the program before it
//! reports the hand-over, so on Unix, where the hand-over is the program
//! replacing the executor, a later failure of that replacement is rare, and
//! it still ends the pane's provisional agent when it is reported.

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// What a start that waits has heard: nothing yet, or the executor's report
/// (`None` handed over, `Some` why the program could not start).
type Heard = Option<Option<String>>;

/// The panes whose profile's start waits for its executor, at most one per
/// pane, since a pane starts one agent at a time.
#[derive(Default)]
pub(super) struct Execs {
    waiting: Mutex<BTreeMap<String, Heard>>,
    heard: Condvar,
}

impl Execs {
    /// Waits for the executor of the start about to be typed in `pane`,
    /// until the returned wait is dropped. A report from before it, or of
    /// an earlier start, is not this start's.
    pub(super) fn expect(&self, pane: &str) -> Expected<'_> {
        self.lock().insert(pane.to_owned(), None);
        Expected {
            execs: self,
            pane: pane.to_owned(),
        }
    }

    /// Hands the executor's report to the start that waits in `pane`;
    /// whether one waited.
    pub(super) fn report(&self, pane: &str, failure: Option<String>) -> bool {
        let mut waiting = self.lock();
        let Some(heard) = waiting.get_mut(pane).filter(|heard| heard.is_none()) else {
            return false;
        };
        *heard = Some(failure);
        self.heard.notify_all();
        true
    }

    /// Whether a start waits in `pane` for its executor.
    #[cfg(test)]
    pub(super) fn waits(&self, pane: &str) -> bool {
        self.lock().get(pane).is_some_and(Option::is_none)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Heard>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One start's wait for its executor.
pub(super) struct Expected<'a> {
    execs: &'a Execs,
    pane: String,
}

impl Expected<'_> {
    /// The executor's report, within `within`: `None` when none came.
    pub(super) fn wait(&self, within: Duration) -> Heard {
        let deadline = Instant::now() + within;
        let mut waiting = self.execs.lock();
        loop {
            match waiting.get(&self.pane) {
                Some(Some(heard)) => return Some(heard.clone()),
                Some(None) => {}
                None => return None,
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            waiting = self
                .execs
                .heard
                .wait_timeout(waiting, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Expected<'_> {
    fn drop(&mut self) {
        self.execs.lock().remove(&self.pane);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_hears_only_the_report_that_came_while_it_waited() {
        let execs = Execs::default();
        assert!(!execs.report("p1", None), "no start waits");
        let expected = execs.expect("p1");
        assert_eq!(expected.wait(Duration::ZERO), None);
        assert!(execs.report("p1", Some("work could not start gw".into())));
        assert!(!execs.report("p1", None), "the first report stands");
        assert_eq!(
            expected.wait(Duration::from_secs(1)),
            Some(Some("work could not start gw".into()))
        );
        drop(expected);
        assert!(!execs.waits("p1"));
        assert!(!execs.report("p1", None), "the wait ended with its start");
    }

    #[test]
    fn a_report_from_another_thread_ends_the_wait() {
        let execs = Execs::default();
        let expected = execs.expect("p1");
        std::thread::scope(|scope| {
            scope.spawn(|| assert!(execs.report("p1", None)));
            assert_eq!(expected.wait(Duration::from_secs(30)), Some(None));
        });
    }
}
