//! A terminal session: the process a pane runs and everything it starts,
//! which the program holding the pane ends as one when the pane closes or
//! its shell exits.
//!
//! Unix: the pane's program leads a session of its own (the PTY's `setsid`),
//! so its tree is every process of that session and every process under one
//! of them. Ending it sends the hangup, then the terminate, then the kill
//! signal, each to every member found by a fresh look, and it has ended once
//! every member ever found is gone and two looks in a row find none. A
//! process that left the session and was handed to init before a look (a
//! daemon that double-forked) is no member: that is the boundary terminals
//! keep.
//!
//! Windows: a kill-on-close job, made before the pane's program starts and
//! held by this process alone, which the program joins before it runs
//! (`PsuedoCon::spawn_command_in_job` in `vendor/portable-pty`), so nothing
//! it starts is outside it and the tree ends with the job however this
//! process ends. Ending it ends the job and waits until every process it
//! ever ran has ended, as an owned child's end does.

use std::io;
use std::time::Duration;

/// How ending a session went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEnd {
    /// Nothing of the session runs.
    Ended,
    /// The bound passed with these processes still running.
    Unconfirmed { left: Vec<u32> },
}

/// The tree of a pane's program. See the module documentation.
pub struct Session {
    inner: sys::Session,
}

impl Session {
    /// A session with no program yet. On Windows this makes the job the
    /// program joins; its handle is never inherited.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            inner: sys::Session::new()?,
        })
    }

    /// The job the pane's program joins before it runs.
    #[cfg(windows)]
    pub fn job(&self) -> std::os::windows::io::RawHandle {
        self.inner.job()
    }

    /// Records `leader`, the pane's program, which leads the session. Call
    /// it before the leader is waited for: until then its pid cannot name
    /// another process, so a leader that already exited still names its
    /// session. On Unix its start time is read now, when it still runs, so a
    /// later process that took its pid is never taken for it.
    pub fn adopt(&mut self, leader: u32) -> io::Result<()> {
        self.inner.adopt(leader)
    }

    /// The processes of the session that run now, the leader first when it
    /// runs. Empty before [`Session::adopt`].
    pub fn members(&self) -> io::Result<Vec<u32>> {
        self.inner.members()
    }

    /// Ends every process of the session and waits, up to `limit`, until
    /// none is left. An error means the session could not be read or ended
    /// at all; a bound that passed is [`SessionEnd::Unconfirmed`].
    pub fn end(&mut self, limit: Duration) -> io::Result<SessionEnd> {
        self.inner.end(limit)
    }
}

#[cfg(unix)]
mod sys {
    use std::collections::BTreeMap;
    use std::io;
    use std::time::{Duration, Instant};

    use super::SessionEnd;

    /// How long each polite signal is given before the next.
    const GRACE: Duration = Duration::from_millis(250);

    /// How often a look is taken while waiting.
    const LOOK_EVERY: Duration = Duration::from_millis(10);

    pub(super) struct Session {
        /// The session's id, which is its leader's pid.
        leader: Option<u32>,
        /// Every member ever found, by pid, with its start time.
        found: BTreeMap<u32, u64>,
    }

    impl Session {
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self {
                leader: None,
                found: BTreeMap::new(),
            })
        }

        pub(super) fn adopt(&mut self, leader: u32) -> io::Result<()> {
            if leader <= 1 || leader > i32::MAX as u32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{leader} leads no session that may be ended"),
                ));
            }
            self.leader = Some(leader);
            // A leader that already exited has no start time and nothing of
            // it to end; its session's members are found by its id.
            if let Ok(started) = crate::process::start_time(leader)
                && crate::process::is_alive(leader)
            {
                self.found.insert(leader, started);
            }
            Ok(())
        }

        pub(super) fn members(&self) -> io::Result<Vec<u32>> {
            Ok(self.look()?.into_iter().map(|(pid, _)| pid).collect())
        }

        /// The members running now, each with its start time: the leader
        /// while it is the process it was, every process whose session is
        /// the leader's pid, and every process under one of them. A pid
        /// stays reserved as a session id while any process of that session
        /// lives, so no later process can take the leader's pid while a
        /// member remains.
        fn look(&self) -> io::Result<Vec<(u32, u64)>> {
            let Some(sid) = self.leader else {
                return Ok(Vec::new());
            };
            let mut members: Vec<u32> = Vec::new();
            for pid in crate::process::sys::all_pids()? {
                if pid <= 1 || pid == std::process::id() {
                    continue;
                }
                // SAFETY: `getsid` takes a pid and has no memory effects;
                // Linux and macOS answer for a process of another session.
                let session = unsafe { libc::getsid(pid as libc::pid_t) };
                if session == sid as libc::pid_t {
                    members.push(pid);
                }
            }
            let roots = members.clone();
            for root in roots {
                for below in crate::process::descendants(root).unwrap_or_default() {
                    if !members.contains(&below) {
                        members.push(below);
                    }
                }
            }
            let mut running = Vec::new();
            for pid in members {
                // A process that ended since the table was read has no start
                // time, and is not running.
                if let Ok(started) = crate::process::start_time(pid)
                    && crate::process::is_alive(pid)
                {
                    running.push((pid, started));
                }
            }
            running.sort_by_key(|(pid, _)| (Some(*pid) != self.leader, *pid));
            Ok(running)
        }

        /// Whether a member found before still runs as the process it was.
        fn still_runs(pid: u32, started: u64) -> bool {
            crate::process::start_time(pid).is_ok_and(|now| now == started)
                && crate::process::is_alive(pid)
        }

        fn send(&mut self, signal: libc::c_int) -> io::Result<()> {
            for (pid, started) in self.look()? {
                self.found.entry(pid).or_insert(started);
            }
            for (pid, started) in &self.found {
                // Read again right before the signal, so a pid another
                // process took since it was found is never signalled.
                if Self::still_runs(*pid, *started) {
                    // SAFETY: `kill` takes plain integers and has no memory
                    // effects; a process gone since is `ESRCH`, which is fine.
                    unsafe { libc::kill(*pid as libc::pid_t, signal) };
                }
            }
            Ok(())
        }

        /// Whether nothing of the session runs: every member found has gone
        /// and a fresh look finds none. A new member is recorded.
        fn quiet(&mut self) -> io::Result<bool> {
            let now = self.look()?;
            let fresh = now.is_empty();
            for (pid, started) in now {
                self.found.entry(pid).or_insert(started);
            }
            Ok(fresh
                && self
                    .found
                    .iter()
                    .all(|(pid, started)| !Self::still_runs(*pid, *started)))
        }

        fn left(&self) -> Vec<u32> {
            self.found
                .iter()
                .filter(|(pid, started)| Self::still_runs(**pid, **started))
                .map(|(pid, _)| *pid)
                .collect()
        }

        pub(super) fn end(&mut self, limit: Duration) -> io::Result<SessionEnd> {
            if self.leader.is_none() {
                return Ok(SessionEnd::Ended);
            }
            let deadline = Instant::now() + limit;
            for signal in [libc::SIGHUP, libc::SIGTERM] {
                self.send(signal)?;
                let until = (Instant::now() + GRACE).min(deadline);
                if self.wait_quiet(until)? {
                    return Ok(SessionEnd::Ended);
                }
            }
            // Killed processes are gone at once, but a member may have
            // started a process just before it ended, so the kill is sent to
            // every fresh look until two in a row find nothing.
            loop {
                self.send(libc::SIGKILL)?;
                if self.quiet()? {
                    crate::process::pause(LOOK_EVERY);
                    if self.quiet()? {
                        return Ok(SessionEnd::Ended);
                    }
                }
                if Instant::now() >= deadline {
                    return Ok(SessionEnd::Unconfirmed { left: self.left() });
                }
                crate::process::pause(LOOK_EVERY);
            }
        }

        /// Waits until two looks in a row are quiet, or `until` passes.
        fn wait_quiet(&mut self, until: Instant) -> io::Result<bool> {
            loop {
                if self.quiet()? {
                    crate::process::pause(LOOK_EVERY);
                    if self.quiet()? {
                        return Ok(true);
                    }
                }
                if Instant::now() >= until {
                    return Ok(false);
                }
                crate::process::pause(LOOK_EVERY);
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::io;
    use std::time::Duration;

    use super::SessionEnd;

    pub(super) struct Session {
        job: crate::process::sys::SessionJob,
    }

    impl Session {
        pub(super) fn new() -> io::Result<Self> {
            Ok(Self {
                job: crate::process::sys::SessionJob::new()?,
            })
        }

        pub(super) fn job(&self) -> std::os::windows::io::RawHandle {
            self.job.handle()
        }

        pub(super) fn adopt(&mut self, _leader: u32) -> io::Result<()> {
            // The job already holds the leader: it joined before it ran.
            Ok(())
        }

        pub(super) fn members(&self) -> io::Result<Vec<u32>> {
            self.job.pids()
        }

        pub(super) fn end(&mut self, limit: Duration) -> io::Result<SessionEnd> {
            self.job.end(limit)
        }
    }
}
