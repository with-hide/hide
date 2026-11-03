//! Holder starts shared by restores and newly drawn panes: eight run at
//! once, eight more may wait, and a pane's readers wait for its actual
//! start outcome without holding the topology lock.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Instant;

use super::{EngineError, refused};

pub(super) const STARTS_AT_ONCE: usize = 8;
const WAITING_STARTS: usize = 8;

#[derive(Debug, Default)]
pub(super) struct Starting {
    result: Mutex<Option<Result<(), EngineError>>>,
    settled: Condvar,
}

impl Starting {
    pub(super) fn finish(&self, result: Result<(), EngineError>) {
        *self.result.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        self.settled.notify_all();
    }

    pub(super) fn wait(&self) -> Result<(), EngineError> {
        let result = self.result.lock().unwrap_or_else(PoisonError::into_inner);
        let (result, _) = self
            .settled
            .wait_timeout_while(result, hide_kit::panes::START_LIMIT, |result| {
                result.is_none()
            })
            .unwrap_or_else(PoisonError::into_inner);
        result.clone().unwrap_or_else(|| {
            Err(refused(
                "holder_start_timeout",
                "the pane's holder did not finish starting before the start deadline".to_owned(),
            ))
        })
    }
}

#[derive(Default)]
struct Counts {
    running: usize,
    waiting: usize,
}

#[derive(Default)]
pub(super) struct Starts {
    counts: Mutex<Counts>,
    available: Condvar,
}

/// Reserved before a background thread is made, so the thread count is
/// bounded as well. Dropping any refused, timed-out or completed start
/// returns its place.
pub(super) struct Turn {
    starts: Arc<Starts>,
    running: bool,
    deadline: Instant,
}

impl Starts {
    #[cfg(test)]
    pub(super) fn waiting(&self) -> usize {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .waiting
    }

    pub(super) fn reserve(self: &Arc<Self>) -> Result<Turn, EngineError> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let running = counts.running < STARTS_AT_ONCE;
        if running {
            counts.running += 1;
        } else if counts.waiting < WAITING_STARTS {
            counts.waiting += 1;
        } else {
            return Err(refused(
                "holder_start_queue_full",
                "eight holders are starting and eight more are waiting".to_owned(),
            ));
        }
        Ok(Turn {
            starts: Arc::clone(self),
            running,
            deadline: Instant::now() + hide_kit::panes::START_LIMIT,
        })
    }
}

impl Turn {
    pub(super) fn enter(&mut self) -> Result<(), EngineError> {
        if self.running {
            return Ok(());
        }
        let mut counts = self
            .starts
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while counts.running == STARTS_AT_ONCE {
            let left = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    refused(
                        "holder_start_timeout",
                        "the holder start queue did not advance before the start deadline"
                            .to_owned(),
                    )
                })?;
            counts = self
                .starts
                .available
                .wait_timeout(counts, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        counts.waiting -= 1;
        counts.running += 1;
        self.running = true;
        Ok(())
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let mut counts = self
            .starts
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.running {
            counts.running -= 1;
        } else {
            counts.waiting -= 1;
        }
        self.starts.available.notify_one();
    }
}
