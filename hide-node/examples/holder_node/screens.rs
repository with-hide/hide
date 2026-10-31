//! What the node hands its screen and its core, kept for the requests.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hide_node::terminal::{OutputSink, ReportSink};
use hide_node_link::terminal::TerminalReport;
use serde_json::{Value, json};

/// How many full frames of a pane are kept for `wait_full`.
const KEPT_FULLS: usize = 8;

/// One pane's frames.
#[derive(Default)]
struct Pane {
    count: usize,
    fulls: usize,
    bytes: u64,
    /// The latest full frames, by index.
    kept: VecDeque<(usize, Vec<u8>)>,
    /// When the first frame and the latest one came.
    first_at: Option<Instant>,
    last_at: Option<Instant>,
}

/// A text the measurement waits for in one pane's frames.
struct Watch {
    pane: String,
    token: Vec<u8>,
    seen: Option<Instant>,
}

#[derive(Default)]
pub struct Screens {
    panes: Mutex<HashMap<String, Pane>>,
    watch: Mutex<Option<Watch>>,
    changed: Condvar,
    /// The slow consumer: how long each frame of a pane whose name starts
    /// with `slow` holds the delivery, in microseconds.
    slow_micros: AtomicU64,
}

impl OutputSink for Screens {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        let now = Instant::now();
        {
            let mut watch = lock(&self.watch);
            if let Some(watch) = watch.as_mut()
                && watch.seen.is_none()
                && watch.pane == pane
                && contains(&text_of(bytes), &watch.token)
            {
                watch.seen = Some(now);
            }
        }
        {
            let mut panes = lock(&self.panes);
            let entry = panes.entry(pane.to_owned()).or_default();
            let index = entry.count;
            entry.count += 1;
            entry.bytes += bytes.len() as u64;
            entry.first_at.get_or_insert(now);
            entry.last_at = Some(now);
            if full {
                entry.fulls += 1;
                entry.kept.push_back((index, bytes.to_vec()));
                if entry.kept.len() > KEPT_FULLS {
                    entry.kept.pop_front();
                }
            }
        }
        self.changed.notify_all();
        let slow = self.slow_micros.load(Ordering::Relaxed);
        if slow > 0 && pane.starts_with("slow") {
            hold(Duration::from_micros(slow));
        }
    }

    fn forget(&self, _pane: &str) {}
}

/// Holds the calling thread for `length`: the slow consumer's delivery.
fn hold(length: Duration) {
    let until = Instant::now() + length;
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        if left.is_zero() {
            break;
        }
        std::thread::park_timeout(left);
    }
}

impl Screens {
    pub fn set_slow(&self, micros: u64) {
        self.slow_micros.store(micros, Ordering::Relaxed);
    }

    pub fn count(&self, pane: &str) -> usize {
        lock(&self.panes).get(pane).map_or(0, |entry| entry.count)
    }

    /// Frames and bytes of every pane, and each pane's own.
    pub fn totals(&self) -> (usize, u64, Vec<(String, usize, u64)>) {
        let panes = lock(&self.panes);
        let each: Vec<(String, usize, u64)> = panes
            .iter()
            .map(|(name, entry)| (name.clone(), entry.count, entry.bytes))
            .collect();
        (
            each.iter().map(|(_, count, _)| count).sum(),
            each.iter().map(|(_, _, bytes)| bytes).sum(),
            each,
        )
    }

    /// The panes that drew at least one full frame.
    pub fn drawn(&self) -> usize {
        lock(&self.panes)
            .values()
            .filter(|entry| entry.fulls > 0)
            .count()
    }

    /// When the last pane drew its first frame.
    pub fn last_first(&self) -> Option<Instant> {
        lock(&self.panes)
            .values()
            .filter_map(|entry| entry.first_at)
            .max()
    }

    pub fn summary(&self, pane: &str) -> Value {
        let panes = lock(&self.panes);
        match panes.get(pane) {
            Some(entry) => {
                json!({"count": entry.count, "fulls": entry.fulls, "bytes": entry.bytes})
            }
            None => json!({"count": 0, "fulls": 0, "bytes": 0}),
        }
    }

    /// Waits for a full frame of `pane` whose index is at least `since`,
    /// and answers the first such one kept.
    pub fn wait_full(&self, pane: &str, since: usize, limit: Duration) -> Option<(usize, Vec<u8>)> {
        let deadline = Instant::now() + limit;
        let mut panes = lock(&self.panes);
        loop {
            if let Some(found) = panes
                .get(pane)
                .and_then(|entry| entry.kept.iter().find(|(index, _)| *index >= since))
            {
                return Some(found.clone());
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            panes = self
                .changed
                .wait_timeout(panes, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits until every pane in `names` drew a full frame.
    pub fn wait_drawn(&self, names: &[String], limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut panes = lock(&self.panes);
        loop {
            if names
                .iter()
                .all(|name| panes.get(name).is_some_and(|entry| entry.fulls > 0))
            {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            panes = self
                .changed
                .wait_timeout(panes, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Arms the watch for `token` in `pane`'s frames from now on.
    pub fn arm(&self, pane: &str, token: &[u8]) {
        *lock(&self.watch) = Some(Watch {
            pane: pane.to_owned(),
            token: token.to_vec(),
            seen: None,
        });
    }

    /// Waits for the armed token, and answers when it was drawn.
    pub fn wait_seen(&self, limit: Duration) -> Option<Instant> {
        let deadline = Instant::now() + limit;
        let mut panes = lock(&self.panes);
        loop {
            if let Some(seen) = lock(&self.watch).as_ref().and_then(|watch| watch.seen) {
                return Some(seen);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            panes = self
                .changed
                .wait_timeout(panes, left.min(Duration::from_millis(50)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// What the node told its core: each pane's states, in order.
#[derive(Default)]
pub struct Reports {
    states: Mutex<HashMap<String, Vec<String>>>,
}

impl ReportSink for Reports {
    fn report(&self, report: TerminalReport) {
        if let TerminalReport::State { pane, state } = report {
            let mut states = lock(&self.states);
            let entry = states.entry(pane).or_default();
            let line = match state.exit_category {
                Some(category) => format!("{}:{category}", state.state),
                None => state.state,
            };
            if entry.len() < 256 {
                entry.push(line);
            }
        }
    }
}

impl Reports {
    pub fn states(&self, pane: &str) -> Vec<String> {
        lock(&self.states).get(pane).cloned().unwrap_or_default()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// The text a frame draws: its bytes less their escape sequences.
pub fn text_of(bytes: &[u8]) -> Vec<u8> {
    let mut text = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != 0x1b {
            if bytes[at] >= 0x20 {
                text.push(bytes[at]);
            }
            at += 1;
            continue;
        }
        at += 1;
        match bytes.get(at) {
            Some(b'[') => {
                at += 1;
                while at < bytes.len() && !(0x40..=0x7e).contains(&bytes[at]) {
                    at += 1;
                }
                at += 1;
            }
            Some(b']' | b'P' | b'_') => {
                while at < bytes.len() && bytes[at] != 0x07 && bytes[at] != 0x1b {
                    at += 1;
                }
                at += if bytes.get(at) == Some(&0x1b) { 2 } else { 1 };
            }
            _ => at += 1,
        }
    }
    text
}
