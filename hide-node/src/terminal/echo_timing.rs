//! How long a pane takes from input the node writes to its program to the
//! next frame the node reads of it, taken at the same point on either engine
//! (docs/PERFORMANCE_TESTING.md, Screen key echo, lost keys and the terminal
//! path's cost). The session's writer stamps the first input since the last
//! frame; its reader closes the stamp at the next frame and folds the time
//! into a fixed table, which it writes as one `terminal.echo_timing` record a
//! second while input comes, and once more when the session ends.
//!
//! Per input: one clock read and one atomic exchange. Per frame: one atomic
//! exchange, and when an input waited, one clock read and an uncontended
//! lock. No timer, thread, queue or I/O beyond the record itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

/// Half-millisecond buckets up to 50 ms; the last counts everything longer.
const BUCKET_US: u64 = 500;
const BUCKETS: usize = 101;
/// How long a pane's table gathers before it is written while input comes.
const SUMMARY_EVERY: Duration = Duration::from_secs(1);

/// One session's input-to-frame clock.
pub(super) struct EchoClock {
    pane: String,
    base: Instant,
    /// Microseconds after `base`, plus one, of the first input written since
    /// the last frame; zero when none waits.
    waiting: AtomicU64,
    table: Mutex<Table>,
}

struct Table {
    /// Microseconds after the clock's base when this table's first time came.
    opened_us: Option<u64>,
    counts: [u32; BUCKETS],
    count: u32,
    total_us: u64,
    max_us: u64,
}

impl EchoClock {
    pub(super) fn new(pane: &str) -> Self {
        Self {
            pane: pane.to_owned(),
            base: Instant::now(),
            waiting: AtomicU64::new(0),
            table: Mutex::new(Table::empty()),
        }
    }

    fn now_us(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_micros()).unwrap_or(u64::MAX - 1)
    }

    /// Input is being written to the program; only the first since the last
    /// frame is timed.
    pub(super) fn input(&self) {
        let at = self.now_us() + 1;
        let _ = self
            .waiting
            .compare_exchange(0, at, Ordering::AcqRel, Ordering::Acquire);
    }

    /// A frame of the pane was read.
    pub(super) fn frame(&self) {
        let waiting = self.waiting.swap(0, Ordering::AcqRel);
        if waiting == 0 {
            return;
        }
        let now = self.now_us();
        let mut table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        table.add(now.saturating_sub(waiting - 1));
        let opened = *table.opened_us.get_or_insert(now);
        if Duration::from_micros(now.saturating_sub(opened)) >= SUMMARY_EVERY {
            table.write(&self.pane);
        }
    }
}

impl Drop for EchoClock {
    fn drop(&mut self) {
        self.table
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .write(&self.pane);
    }
}

impl Table {
    fn empty() -> Self {
        Self {
            opened_us: None,
            counts: [0; BUCKETS],
            count: 0,
            total_us: 0,
            max_us: 0,
        }
    }

    fn add(&mut self, took_us: u64) {
        let bucket = usize::try_from(took_us / BUCKET_US)
            .unwrap_or(BUCKETS - 1)
            .min(BUCKETS - 1);
        self.counts[bucket] += 1;
        self.count += 1;
        self.total_us += took_us;
        self.max_us = self.max_us.max(took_us);
    }

    /// The upper edge, in milliseconds, of the bucket holding the nearest-rank
    /// `share` of the times.
    fn rank_ms(&self, share: f64) -> f64 {
        let rank = ((f64::from(self.count) * share).ceil() as u32).max(1);
        let mut seen = 0;
        for (bucket, count) in self.counts.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return ((bucket as u64 + 1) * BUCKET_US) as f64 / 1000.0;
            }
        }
        self.max_us as f64 / 1000.0
    }

    /// Writes the table as one record and empties it; an empty one writes
    /// nothing.
    fn write(&mut self, pane: &str) {
        if self.count == 0 {
            return;
        }
        crate::diagnostic!(json!({
            "component": "terminal_session",
            "kind": "terminal.echo_timing",
            "pane_id": pane,
            "count": self.count,
            "p50_ms": self.rank_ms(0.5),
            "p95_ms": self.rank_ms(0.95),
            "mean_ms": (self.total_us as f64 / f64::from(self.count) / 100.0).round() / 10.0,
            "max_ms": (self.max_us as f64 / 100.0).round() / 10.0,
        }));
        *self = Self::empty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_input_since_a_frame_is_timed_and_a_frame_without_input_counts_nothing() {
        let clock = EchoClock::new("w1:p1");
        clock.frame();
        assert_eq!(clock.table.lock().unwrap().count, 0);
        // The first input, as if written at the clock's base.
        clock.waiting.store(1, Ordering::Release);
        clock.input();
        assert_eq!(
            clock.waiting.load(Ordering::Acquire),
            1,
            "a later input keeps the first one's stamp"
        );
        clock.frame();
        clock.frame();
        let table = clock.table.lock().unwrap();
        assert_eq!(table.count, 1, "the second frame had no input before it");
        assert!(table.total_us <= clock.now_us());
    }

    #[test]
    fn ranks_read_the_bucket_edges() {
        let mut table = Table::empty();
        for took in [1_200, 16_100, 16_300, 20_400, 70_000] {
            table.add(took);
        }
        assert_eq!(table.rank_ms(0.5), 16.5);
        assert_eq!(table.rank_ms(0.95), 50.5);
        assert_eq!(table.max_us, 70_000);
    }
}
