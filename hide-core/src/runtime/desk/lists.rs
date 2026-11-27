//! Each agent's commands read ahead of the operator (mission-106
//! command-lists): a thread of Desk's worker opens each agent in place and
//! switched on without a prompt, takes the list it sends and ends it
//! ([`hide_desk::read_ahead`]), so `/` lists it at once in Desk's composer
//! and in a session's input, for an agent Desk never talked to too.
//!
//! What it owns and its bounds (engineering principles 14 and 15): one
//! thread while Desk is on, which runs one agent's process at a time, never
//! under the runtime lock, and ends it on every path (its list, a refusal,
//! [`hide_desk::read_ahead::Bounds`], Desk switched off, the worker's end);
//! dropping the [`Reader`] stops the thread and waits for it, so the next
//! reader never runs beside it. It keeps no timer: it sleeps until the
//! worker hands it something.
//!
//! It looks at an agent at three moments only: a look at every agent (Desk
//! turned on, hided starting, `다시 시작`, and a change of the catalog: an
//! agent switched on or off in Settings › Agents, the operator signing in to
//! or out of one as Settings reads it), an agent's adapter installed, and
//! the operator opening the `/` menu on that agent. Each time it digests
//! what the agent's list depends on ([`hide_desk::read_ahead::mark`]: the
//! agent's program and the skills folder its definition names, file facts
//! only, and whether the operator is signed out of it) and starts the agent
//! only when that moved since the list it has, and not when a read of that
//! same digest already failed: a failure is tried again once the program,
//! the skills or the sign-in moved, or when the operator asks: opening `/`
//! on the agent or `다시 시작` tries it once more, so a transient failure
//! or a sign-in Settings does not read is never stuck, and the retries are
//! as many as the operator's own presses. The agent whose Desk session is
//! open is not read, since that session sends its list. Each read can leave
//! one empty session in the agents that cannot delete it (omp, Grok, agy),
//! so a read happens only when what the list depends on moved.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use hide_agent_adapter::{AccountPath, NodeEnv, Place, Profile};
use hide_desk::AdapterId;
use hide_desk::read_ahead::{self, Bounds, Unread};

use super::worker::{Order, launch_for};
use crate::desk::{DeskPlaces, ReadMark, ReadMarks};

/// What the worker hands the thread.
pub(super) enum Wanted {
    /// A look at every agent: the core's last reads, the agents switched on
    /// in Settings › Agents, those of them in place now, and those the
    /// operator is signed out of, as Settings reads it.
    Look {
        marks: ReadMarks,
        on: Vec<AdapterId>,
        ready: Vec<AdapterId>,
        signed_out: Vec<AdapterId>,
        /// The operator asked for it (`다시 시작`): failures are forgotten.
        retry: bool,
    },
    /// An agent whose adapter was just installed.
    Ready(AdapterId),
    /// The operator opened the `/` menu on an agent: a failure of its read
    /// is forgotten, so this look reads it once more.
    Check(AdapterId),
}

/// Which agent the thread looks at next and whether it reads it: the
/// thread's state, with no clock and no process, so a test drives it.
#[derive(Debug, Default)]
struct Schedule {
    marks: ReadMarks,
    /// The agents switched on at the last look.
    on: BTreeSet<AdapterId>,
    /// The agents the operator is signed out of at the last look.
    signed_out: BTreeSet<AdapterId>,
    /// The agents switched on and in place.
    in_place: BTreeSet<AdapterId>,
    queue: VecDeque<AdapterId>,
    /// Each agent whose last read brought no list, by what it depended on
    /// then: the same digest is not read again.
    failed: HashMap<AdapterId, String>,
}

impl Schedule {
    fn take(&mut self, wanted: Wanted) {
        match wanted {
            Wanted::Look {
                marks,
                on,
                ready,
                signed_out,
                retry,
            } => {
                if retry {
                    self.failed.clear();
                }
                self.marks = marks;
                self.on = on.into_iter().collect();
                self.signed_out = signed_out.into_iter().collect();
                self.in_place = ready
                    .into_iter()
                    .filter(|agent| self.on.contains(agent))
                    .collect();
                self.queue.retain(|agent| self.in_place.contains(agent));
                let agents: Vec<AdapterId> = self.in_place.iter().copied().collect();
                self.queue_up(agents);
            }
            Wanted::Ready(agent) => {
                if self.on.contains(&agent) {
                    self.in_place.insert(agent);
                    self.queue_up(vec![agent]);
                }
            }
            Wanted::Check(agent) => {
                if self.in_place.contains(&agent) {
                    self.failed.remove(&agent);
                    self.queue_up(vec![agent]);
                }
            }
        }
    }

    fn queue_up(&mut self, agents: Vec<AdapterId>) {
        for agent in agents {
            if !self.queue.contains(&agent) {
                self.queue.push_back(agent);
            }
        }
    }

    fn next(&mut self) -> Option<AdapterId> {
        self.queue.pop_front()
    }

    /// What `agent`'s list depends on beyond its files: a sign-out and the
    /// sign-in after it each read it again.
    fn facts(&self, agent: AdapterId, files: String) -> String {
        if self.signed_out.contains(&agent) {
            files + "signed out\n"
        } else {
            files
        }
    }

    /// Whether `agent`, whose list depends on what `mark` digests now, is
    /// read: never read or moved since its list, and no read of this same
    /// digest failed.
    fn due(&self, agent: AdapterId, mark: &ReadMark) -> bool {
        self.marks
            .get(&agent)
            .is_none_or(|last| last.mark != mark.mark)
            && self.failed.get(&agent) != Some(&mark.mark)
    }

    /// A read of `agent` ended, with a list or without.
    fn read(&mut self, agent: AdapterId, mark: &ReadMark, listed: bool) {
        if listed {
            self.failed.remove(&agent);
            self.marks.insert(agent, mark.clone());
        } else {
            self.failed.insert(agent, mark.mark.clone());
        }
    }

    /// `agent` is not in place any more: a later look finds it again.
    fn gone(&mut self, agent: AdapterId) {
        self.in_place.remove(&agent);
    }
}

/// The thread that reads the agents' commands; dropping it stops the thread
/// and the process it runs, and waits for both.
pub(super) struct Reader {
    wanted: Option<Sender<Wanted>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    /// Starts the thread; `open` is the agent whose Desk session is open,
    /// which the worker keeps.
    pub(super) fn start(
        places: DeskPlaces,
        orders: Sender<Order>,
        open: Arc<Mutex<Option<AdapterId>>>,
    ) -> std::io::Result<Self> {
        let (wanted, received) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = Thread {
            places,
            orders,
            open,
            stop: Arc::clone(&stop),
            schedule: Schedule::default(),
        };
        let thread = std::thread::Builder::new()
            .name("hide-core-desk-lists".into())
            .spawn(move || thread.run(&received))?;
        Ok(Self {
            wanted: Some(wanted),
            stop,
            thread: Some(thread),
        })
    }

    pub(super) fn read(&self, wanted: Wanted) {
        if let Some(sender) = &self.wanted {
            let _ = sender.send(wanted);
        }
    }
}

impl Drop for Reader {
    /// Stops a read under way at its next poll, and wakes a thread that
    /// sleeps by closing its channel.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wanted = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Thread {
    places: DeskPlaces,
    orders: Sender<Order>,
    open: Arc<Mutex<Option<AdapterId>>>,
    stop: Arc<AtomicBool>,
    schedule: Schedule,
}

impl Thread {
    fn run(mut self, received: &Receiver<Wanted>) {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            // Everything waiting is taken before the next agent is read, so
            // a look that switched an agent off comes first.
            while let Ok(wanted) = received.try_recv() {
                self.schedule.take(wanted);
            }
            match self.schedule.next() {
                Some(agent) => self.read(agent),
                None => match received.recv() {
                    Ok(wanted) => self.schedule.take(wanted),
                    Err(_) => return,
                },
            }
        }
    }

    /// Reads `agent`'s list when it is due, and tells the worker.
    fn read(&mut self, agent: AdapterId) {
        let open = *self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if open == Some(agent) {
            return;
        }
        let (launch, cli) = match launch_for(agent, &self.places, None) {
            Ok(found) => found,
            Err(why) => {
                // Gone since the worker looked (its next look says so), or
                // a piece of its start missing: the menu says it failed.
                self.schedule.gone(agent);
                self.tell(agent, Err(Unread::Failed(why)));
                return;
            }
        };
        let skills = skills_folder(agent);
        let facts = self.schedule.facts(
            agent,
            read_ahead::mark(agent, cli.as_deref(), skills.as_deref()),
        );
        let mark = ReadMark::of(&facts, crate::observed::now_unix_ms());
        if !self.schedule.due(agent, &mark) {
            return;
        }
        let cwd = hide_desk::adapter::work_dir(&self.places.desk);
        let result = read_ahead::read(agent, &launch, &cwd, &self.stop, Bounds::DEFAULT);
        if matches!(result, Err(Unread::Stopped)) {
            return;
        }
        self.schedule.read(agent, &mark, result.is_ok());
        self.tell(agent, result.map(|commands| (commands, mark)));
    }

    fn tell(
        &self,
        agent: AdapterId,
        result: Result<(hide_desk::commands::Commands, ReadMark), Unread>,
    ) {
        let _ = self.orders.send(Order::ListRead {
            adapter: agent,
            result,
        });
    }
}

/// The skills folder `agent`'s definition names for the account's default
/// profile (`[account.paths] skills`), with the account's own variables as
/// a new terminal has them; the shared folder for `"shared"`.
fn skills_folder(agent: AdapterId) -> Option<PathBuf> {
    let home = hide_platform::host::home_dir().ok()?;
    let catalog = agent.catalog();
    let account = hide_platform::programs::account_environment();
    let variable = |name: &str| {
        account.as_deref().and_then(|account| {
            account
                .iter()
                .find(|(own, _)| hide_desk::host::same_name(own, std::ffi::OsStr::new(name)))
                .map(|(_, value)| value.clone())
        })
    };
    let resolved = catalog
        .resolve(
            &Profile::builtin_default(catalog.id),
            &NodeEnv::new(Some(home.clone()))
                .with_account_variables(catalog.account_rule(), variable),
        )
        .ok()?;
    match resolved.place(AccountPath::Skills) {
        Place::At(path) => Some(path.clone()),
        Place::Shared => Some(hide_kit::agents::SkillDir::Shared.path(&home)),
        Place::Absent => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PI: AdapterId = AdapterId::PI;
    const OMP: AdapterId = AdapterId::OMP;

    fn look(on: &[AdapterId], signed_out: &[AdapterId], marks: ReadMarks) -> Wanted {
        Wanted::Look {
            marks,
            on: on.to_vec(),
            ready: vec![PI, OMP],
            signed_out: signed_out.to_vec(),
            retry: false,
        }
    }

    fn drain(schedule: &mut Schedule) -> Vec<AdapterId> {
        std::iter::from_fn(|| schedule.next()).collect()
    }

    /// An agent switched off is never looked at: the look that says so
    /// drops it from the queue, and neither an install of its adapter nor
    /// a `/` on it brings it back; switched on again, the next look does.
    #[test]
    fn an_agent_switched_off_is_never_read() {
        let mut schedule = Schedule::default();
        schedule.take(look(&[PI, OMP], &[], ReadMarks::new()));
        schedule.take(look(&[OMP], &[], ReadMarks::new()));
        assert_eq!(drain(&mut schedule), [OMP]);
        schedule.take(Wanted::Ready(PI));
        schedule.take(Wanted::Check(PI));
        assert_eq!(drain(&mut schedule), Vec::<AdapterId>::new());
        schedule.take(Wanted::Check(OMP));
        assert_eq!(drain(&mut schedule), [OMP]);

        schedule.take(look(&[PI, OMP], &[], ReadMarks::new()));
        assert_eq!(drain(&mut schedule), [PI, OMP]);
    }

    /// A `/` looks at its agent; the agent is read only when what its list
    /// depends on moved since the list it has.
    #[test]
    fn a_slash_reads_its_agent_again_only_when_its_mark_moved() {
        let mut schedule = Schedule::default();
        schedule.take(look(&[OMP], &[], ReadMarks::new()));
        let v1 = ReadMark::of("cli /opt/omp 10 1\n", 1_000);
        assert!(schedule.due(OMP, &v1));
        schedule.read(OMP, &v1, true);
        assert_eq!(drain(&mut schedule), [OMP]);

        schedule.take(Wanted::Check(OMP));
        assert_eq!(drain(&mut schedule), [OMP]);
        // Time alone moves nothing: a week later the same files read as the
        // same list.
        let later = ReadMark::of("cli /opt/omp 10 1\n", 1_000 + 7 * 24 * 3_600_000);
        assert!(!schedule.due(OMP, &later));
        let v2 = ReadMark::of("cli /opt/omp 11 2\n", 2_000);
        assert!(schedule.due(OMP, &v2));
    }

    /// A read that brought no list is not tried again on the same digest by
    /// a look Hide takes itself; a new program, skill or sign-in is a new
    /// digest, and the sign-in moves it through the look that reports it.
    #[test]
    fn a_failed_read_is_tried_again_only_when_its_files_or_sign_in_moved() {
        let mut schedule = Schedule::default();
        let files = || "cli /opt/omp 10 1\n".to_owned();
        schedule.take(look(&[OMP], &[], ReadMarks::new()));
        let refused = ReadMark::of(&schedule.facts(OMP, files()), 1_000);
        schedule.read(OMP, &refused, false);
        for _ in 0..3 {
            schedule.take(look(&[OMP], &[], ReadMarks::new()));
            assert!(!schedule.due(OMP, &refused));
        }
        // Settings reads the operator signed out, then in: each moves the
        // digest, and so reads again.
        schedule.take(look(&[OMP], &[OMP], ReadMarks::new()));
        let signed_out = ReadMark::of(&schedule.facts(OMP, files()), 1_000);
        assert!(schedule.due(OMP, &signed_out));
        schedule.read(OMP, &signed_out, false);
        assert!(!schedule.due(OMP, &signed_out));
        schedule.take(look(&[OMP], &[], ReadMarks::new()));
        let signed_in = ReadMark::of(&schedule.facts(OMP, files()), 1_000);
        assert_eq!(signed_in.mark, refused.mark);
        assert!(schedule.due(OMP, &signed_in));
        // A new program is a new digest too.
        let updated = ReadMark::of(&schedule.facts(OMP, "cli /opt/omp 11 2\n".into()), 1_000);
        assert!(schedule.due(OMP, &updated));
    }

    /// The operator's own press tries a failed read once more on the same
    /// digest: opening `/` on that agent, or `다시 시작` for every agent; a
    /// failure after it waits for the next press.
    #[test]
    fn a_slash_or_a_restart_tries_a_failed_read_once_more() {
        let mut schedule = Schedule::default();
        schedule.take(look(&[PI, OMP], &[], ReadMarks::new()));
        drain(&mut schedule);
        let pi = ReadMark::of("cli /opt/pi 1 1\n", 1_000);
        let omp = ReadMark::of("cli /opt/omp 1 1\n", 1_000);
        schedule.read(PI, &pi, false);
        schedule.read(OMP, &omp, false);

        schedule.take(Wanted::Check(PI));
        assert_eq!(drain(&mut schedule), [PI]);
        assert!(schedule.due(PI, &pi));
        assert!(!schedule.due(OMP, &omp));
        schedule.read(PI, &pi, false);
        assert!(!schedule.due(PI, &pi));

        schedule.take(Wanted::Look {
            marks: ReadMarks::new(),
            on: vec![PI, OMP],
            ready: vec![PI, OMP],
            signed_out: Vec::new(),
            retry: true,
        });
        assert!(schedule.due(PI, &pi) && schedule.due(OMP, &omp));
    }
}
