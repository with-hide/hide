//! Each agent's commands read ahead of the operator (mission-106
//! command-lists): a thread of Desk's worker opens each agent in place
//! without a prompt, takes the list it sends and ends it
//! ([`hide_desk::read_ahead`]), so `/` lists it at once in Desk's composer
//! and in a session's input, for an agent Desk never talked to too.
//!
//! What it owns and its bounds (engineering principles 14 and 15): one
//! thread while Desk is on, which runs one agent's process at a time, never
//! under the runtime lock, and ends it on every path (its list, a refusal,
//! [`hide_desk::read_ahead::READ_DEADLINE`], Desk switched off, the worker's
//! end); dropping the [`Reader`] stops the thread within a fifth of a second.
//! An agent is read when the worker hands it in place (Desk turned on or
//! restarted, hided starting, its adapter installed) and again on the
//! [`ROUND`], each time only when its list depends on something that moved
//! since its last read ([`hide_desk::read_ahead::mark`]: the agent's
//! program and the skills folder its definition names, file facts only), or
//! that read is a day old; an agent that brought no list is tried again at
//! most once an hour unless its mark moves.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use hide_desk::AdapterId;
use hide_desk::read_ahead::{self, Unread};

use super::worker::{Order, launch_for};
use crate::desk::{DeskPlaces, ReadMark, ReadMarks};

/// How often the agents in place are looked at again.
const ROUND: Duration = Duration::from_secs(10 * 60);
/// A list read longer ago than this is read again whatever its mark says:
/// what the mark does not see (a plugin, a project's command) still reaches
/// the menu within a day.
const STALE_MS: u64 = 24 * 60 * 60 * 1000;
/// An agent whose read brought no list is not read again sooner, unless what
/// it depends on moved.
const RETRY: Duration = Duration::from_secs(60 * 60);

/// What the worker hands the thread.
struct Wanted {
    /// The core's last reads, which replace the thread's; none keeps them.
    marks: Option<ReadMarks>,
    agents: Vec<AdapterId>,
}

/// The thread that reads the agents' commands; dropping it stops the thread
/// and the process it runs.
pub(super) struct Reader {
    wanted: Sender<Wanted>,
    stop: Arc<AtomicBool>,
}

impl Reader {
    pub(super) fn start(places: DeskPlaces, orders: Sender<Order>) -> std::io::Result<Self> {
        let (wanted, received) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = Thread {
            places,
            orders,
            stop: Arc::clone(&stop),
            marks: ReadMarks::new(),
            tried: HashMap::new(),
            in_place: BTreeSet::new(),
            queue: VecDeque::new(),
        };
        std::thread::Builder::new()
            .name("hide-core-desk-lists".into())
            .spawn(move || thread.run(&received))?;
        Ok(Self { wanted, stop })
    }

    /// Reads `agents` when their lists are due, in this order, after any
    /// already waiting.
    pub(super) fn read(&self, marks: Option<ReadMarks>, agents: Vec<AdapterId>) {
        let _ = self.wanted.send(Wanted { marks, agents });
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct Thread {
    places: DeskPlaces,
    orders: Sender<Order>,
    stop: Arc<AtomicBool>,
    marks: ReadMarks,
    /// Each agent whose last read brought no list: what it depended on then,
    /// and when.
    tried: HashMap<AdapterId, (String, Instant)>,
    /// The agents the worker said are in place, looked at each round.
    in_place: BTreeSet<AdapterId>,
    queue: VecDeque<AdapterId>,
}

impl Thread {
    fn run(mut self, received: &Receiver<Wanted>) {
        let mut round = Instant::now() + ROUND;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            // An agent waiting is read before the thread sleeps again.
            let wait = if self.queue.is_empty() {
                round.saturating_duration_since(Instant::now())
            } else {
                Duration::ZERO
            };
            match received.recv_timeout(wait.min(Duration::from_millis(200))) {
                Ok(wanted) => self.take(wanted),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= round {
                round = Instant::now() + ROUND;
                let in_place: Vec<AdapterId> = self.in_place.iter().copied().collect();
                self.queue_up(in_place);
            }
            if let Some(adapter) = self.queue.pop_front() {
                self.read(adapter);
            }
        }
    }

    fn take(&mut self, wanted: Wanted) {
        if let Some(marks) = wanted.marks {
            self.marks = marks;
        }
        self.in_place.extend(wanted.agents.iter().copied());
        self.queue_up(wanted.agents);
    }

    fn queue_up(&mut self, agents: Vec<AdapterId>) {
        for adapter in agents {
            if !self.queue.contains(&adapter) {
                self.queue.push_back(adapter);
            }
        }
    }

    /// Reads `adapter`'s list when it is due, and tells the worker.
    fn read(&mut self, adapter: AdapterId) {
        let (launch, cli) = match launch_for(adapter, &self.places, None) {
            Ok(found) => found,
            // Gone since the worker looked: its next look says so.
            Err(_) => {
                self.in_place.remove(&adapter);
                return;
            }
        };
        let account = hide_platform::programs::account_environment();
        let variable = |name: &str| {
            account.as_deref().and_then(|account| {
                account
                    .iter()
                    .find(|(own, _)| hide_desk::host::same_name(own, std::ffi::OsStr::new(name)))
                    .map(|(_, value)| value.clone())
            })
        };
        let home = hide_platform::host::home_dir().ok();
        let facts = read_ahead::mark(adapter, cli.as_deref(), home.as_deref(), variable);
        let now_ms = crate::observed::now_unix_ms();
        let mark = ReadMark::of(&facts, now_ms);
        if !due(self.marks.get(&adapter), &mark) {
            return;
        }
        if self
            .tried
            .get(&adapter)
            .is_some_and(|(tried, at)| *tried == mark.mark && at.elapsed() < RETRY)
        {
            return;
        }
        let cwd = hide_desk::adapter::work_dir(&self.places.desk);
        let result = read_ahead::read(adapter, &launch, &cwd, &self.stop);
        match &result {
            Ok(_) => {
                self.tried.remove(&adapter);
                self.marks.insert(adapter, mark.clone());
            }
            Err(Unread::Stopped) => return,
            Err(_) => {
                self.tried
                    .insert(adapter, (mark.mark.clone(), Instant::now()));
            }
        }
        let _ = self.orders.send(Order::ListRead {
            adapter,
            result: result.map(|commands| (commands, mark)),
        });
    }
}

/// Whether a list last read as `last` is read again, now that what it
/// depends on reads as `now`: never read, moved, or a day old.
fn due(last: Option<&ReadMark>, now: &ReadMark) -> bool {
    last.is_none_or(|last| {
        last.mark != now.mark || now.at_ms.saturating_sub(last.at_ms) >= STALE_MS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_is_read_again_only_when_what_it_depends_on_moved_or_it_is_a_day_old() {
        let read = ReadMark::of("cli /opt/omp 10 1\n", 1_000);
        assert!(due(None, &read));
        assert!(!due(
            Some(&read),
            &ReadMark::of("cli /opt/omp 10 1\n", 1_000 + 60_000)
        ));
        assert!(due(
            Some(&read),
            &ReadMark::of("cli /opt/omp 11 2\n", 2_000)
        ));
        assert!(due(
            Some(&read),
            &ReadMark::of("cli /opt/omp 10 1\n", 1_000 + STALE_MS)
        ));
    }
}
