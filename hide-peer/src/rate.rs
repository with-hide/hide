//! How often one machine may ask for the same thing (security spec 2.4,
//! 3.1): six confirms, six removals, six requests to be let in, each in a
//! minute.
//!
//! The streams that carry them open with no grant, so any machine that can
//! reach this one can send them; the count is per proved key.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use hide_peer_protocol::PeerKey;
use tokio::time::Instant;

use crate::limits;

#[derive(Default)]
pub struct Rate {
    asked: Mutex<HashMap<PeerKey, VecDeque<Instant>>>,
}

impl Rate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts one ask from `key`. `false` when it is one too many: the
    /// caller answers nothing and does nothing.
    pub fn admit(&self, key: &PeerKey) -> bool {
        let now = Instant::now();
        let fresh = |at: &Instant| now.duration_since(*at) < limits::RATE_WINDOW;
        let mut asked = self
            .asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !asked.contains_key(key) && asked.len() >= limits::MAX_RATED {
            // Machines that asked nothing for a window are no longer
            // counted. When every place is still held, a machine not yet
            // counted is refused rather than counted from nothing.
            asked.retain(|_, times| times.back().is_some_and(fresh));
            if asked.len() >= limits::MAX_RATED {
                return false;
            }
        }
        let times = asked.entry(*key).or_default();
        while times.front().is_some_and(|at| !fresh(at)) {
            times.pop_front();
        }
        if times.len() >= limits::RATE_PER_WINDOW {
            return false;
        }
        times.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn key(index: usize) -> PeerKey {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
        PeerKey::from_bytes(bytes)
    }

    #[tokio::test(start_paused = true)]
    async fn the_seventh_ask_in_a_minute_is_refused_and_another_machine_is_not() {
        let rate = Rate::new();
        for _ in 0..limits::RATE_PER_WINDOW {
            assert!(rate.admit(&key(1)));
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert!(!rate.admit(&key(1)));
        assert!(rate.admit(&key(2)), "counted per machine");

        // A minute after the first ask, one place is free again; a refused
        // ask took none.
        tokio::time::advance(limits::RATE_WINDOW - Duration::from_secs(6)).await;
        assert!(rate.admit(&key(1)));
        assert!(!rate.admit(&key(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn machines_that_went_quiet_make_room_and_a_full_table_refuses() {
        let rate = Rate::new();
        for index in 0..limits::MAX_RATED {
            assert!(rate.admit(&key(index)));
        }
        assert!(!rate.admit(&key(limits::MAX_RATED)), "every place is held");
        assert!(rate.admit(&key(0)), "a machine already counted still asks");

        tokio::time::advance(limits::RATE_WINDOW).await;
        assert!(rate.admit(&key(limits::MAX_RATED)));
    }
}
