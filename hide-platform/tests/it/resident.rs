//! The contract of `hide_platform::process::start_resident` and
//! `hide_platform::process::Session`, stated as what a caller observes: a
//! resident child that keeps nothing of its starter and outlives it, and a
//! session whose end leaves nothing of it running. The same file runs on
//! macOS, Linux and Windows.
//!
//! The test binary runs itself with `HIDE_PLATFORM_RESIDENT_ROLE` set to play
//! a starter, a resident or a session's member; `resident::role` reads it and
//! does nothing without it. Every process a test starts is ended by the
//! guard that holds its pid, and a role that is left alone ends by itself
//! within a minute.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use hide_platform::process::{
    ResidentCommand, Session, SessionEnd, is_alive, kill_tree, start_resident, start_time,
    take_lent,
};

const ROLE: &str = "HIDE_PLATFORM_RESIDENT_ROLE";
/// What a starter does to itself before it starts a resident.
const SETUP: &str = "HIDE_PLATFORM_RESIDENT_SETUP";
/// Where a session member writes the pids it started.
const PIDS_FILE: &str = "HIDE_PLATFORM_RESIDENT_PIDS";

/// A hang guard for a start or an answer, never a measure of speed.
const HANG_LIMIT: Duration = Duration::from_secs(30);
/// How long a role that nobody ends lives.
const ROLE_LIFE: Duration = Duration::from_secs(60);
/// How long a session's end may take; `Session::end`'s own bound in a holder.
const END_LIMIT: Duration = Duration::from_secs(5);

/// A process named while it runs, by its pid and start time, so asking
/// later whether it still runs, or ending it, never reaches a process that
/// took its pid after it ended (docs/TESTING.md, rule 7).
#[derive(Clone, Copy, Debug)]
struct Pinned {
    pid: u32,
    started: u64,
}

impl Pinned {
    /// `pid` as it runs now; `None` when it has already ended.
    fn of(pid: u32) -> Option<Self> {
        let started = start_time(pid).ok()?;
        is_alive(pid).then_some(Self { pid, started })
    }

    fn is_running(&self) -> bool {
        is_alive(self.pid) && start_time(self.pid).ok() == Some(self.started)
    }
}

/// Ends the process it names when dropped, however the test ends, while it
/// is still that process.
struct Started(Option<Pinned>);

impl Started {
    fn new(pid: u32) -> Self {
        Self(Pinned::of(pid))
    }

    fn is_running(&self) -> bool {
        self.0.is_some_and(|pinned| pinned.is_running())
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(pinned) = self.0
            && pinned.is_running()
        {
            let _ = kill_tree(pinned.pid);
        }
    }
}

/// A child of the test, ended and reaped when dropped. One the test already
/// waited for names nothing any more, so it is left alone.
struct Owned(Child);

impl Drop for Owned {
    fn drop(&mut self) {
        // Until it is reaped its pid is its own, even once it has exited.
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = kill_tree(self.0.id());
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Each of `pids` that still runs, named now.
fn pinned(pids: &[u32]) -> Vec<Pinned> {
    pids.iter().filter_map(|pid| Pinned::of(*pid)).collect()
}

fn role_command(role: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "resident::role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, role)
        .env_remove(SETUP)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// The `resident` role, as a resident start runs it.
fn resident_command() -> ResidentCommand {
    resident_command_of(&std::env::current_exe().unwrap())
}

/// The `resident` role, run from `program`, a copy of this test binary.
fn resident_command_of(program: &Path) -> ResidentCommand {
    let mut command = ResidentCommand::new(program);
    command
        .args([
            "--exact",
            "resident::role",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ROLE, "resident")
        .env_remove(SETUP);
    command
}

#[allow(clippy::disallowed_methods)] // keeps a role alive: the sleep is its life
fn live_out_role() {
    thread::sleep(ROLE_LIFE);
}

/// The text after `marker` on the first line of `stdout` that holds it.
fn line_after(stdout: impl Read + Send + 'static, marker: &'static str) -> String {
    let (sender, heard) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((_, rest)) = line.split_once(marker) {
                let _ = sender.send(rest.trim().to_owned());
                return;
            }
        }
    });
    heard
        .recv_timeout(HANG_LIMIT)
        .unwrap_or_else(|_| panic!("no line with {marker:?} came"))
}

/// One `key=value` field of a report.
fn field<'a>(report: &'a str, key: &str) -> &'a str {
    report
        .split_whitespace()
        .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("the report {report:?} has no {key}"))
}

#[allow(clippy::disallowed_methods)] // a polling helper bounded by a deadline
fn wait_until(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < HANG_LIMIT, "{what} never happened");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Starts the `resident` role as a resident child of this process and reads
/// its report.
fn start_reporting_resident() -> (Started, String) {
    start_reporting(&resident_command())
}

fn start_reporting(command: &ResidentCommand) -> (Started, String) {
    let (reader, writer) = std::io::pipe().unwrap();
    #[cfg(unix)]
    let lent = [std::os::fd::AsFd::as_fd(&writer)];
    #[cfg(windows)]
    let lent = [std::os::windows::io::AsHandle::as_handle(&writer)];
    let pid = start_resident(command, &lent).unwrap().pid;
    let started = Started::new(pid);
    drop(writer);
    let report = line_after(reader, "RESIDENT ");
    (started, report)
}

#[test]
fn role() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    match role.as_str() {
        "resident" => {
            let mut lent = take_lent().unwrap();
            assert_eq!(lent.len(), 1, "a resident is lent its report pipe");
            let report = describe_self();
            let mut pipe = std::fs::File::from(lent.remove(0));
            writeln!(pipe, "RESIDENT {report}").unwrap();
            drop(pipe);
            live_out_role();
        }
        "starter" => starter(),
        "quick" => println!("QUICK"),
        "sleep" => {
            println!("READY {}", std::process::id());
            live_out_role();
        }
        #[cfg(unix)]
        "session_leader" => session_leader(),
        #[cfg(unix)]
        "stubborn" => stubborn(),
        #[cfg(windows)]
        "job_leader" => job_leader(),
        other => panic!("unknown role {other}"),
    }
}

/// Starts a resident after doing to itself what `SETUP` names, prints the
/// resident's report or the refusal, and then lives out its role while the
/// test ends it.
fn starter() {
    let setup = std::env::var(SETUP).unwrap_or_default();
    #[cfg(unix)]
    if setup.contains("own_session") {
        // SAFETY: setsid takes nothing and has no memory effects.
        assert!(unsafe { libc::setsid() } >= 0);
    }
    #[cfg(unix)]
    let _extra = if setup.contains("unix_extras") {
        // A starter's supervisor lends it more than its streams, and a
        // starter ignores or blocks signals of its own.
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file);
        // SAFETY: plain calls on a descriptor this process holds and on its
        // own signal state.
        unsafe {
            // A copy above the names lent handles take, without
            // close-on-exec, as a supervisor's loan arrives.
            assert!(libc::fcntl(fd, libc::F_DUPFD, 20) >= 20);
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            let mut blocked: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, std::ptr::null_mut());
        }
        Some(file)
    } else {
        None
    };
    #[cfg(windows)]
    let _job = windows_job::enter(&setup);
    let (reader, writer) = std::io::pipe().unwrap();
    let command = resident_command();
    #[cfg(unix)]
    let lent = [std::os::fd::AsFd::as_fd(&writer)];
    #[cfg(windows)]
    let lent = [std::os::windows::io::AsHandle::as_handle(&writer)];
    match start_resident(&command, &lent) {
        Ok(resident) => {
            drop(writer);
            let report = line_after(reader, "RESIDENT ");
            println!(
                "STARTED pid={} outer_job={} starter={} {report}",
                resident.pid,
                resident.outer_job,
                std::process::id()
            );
        }
        Err(error) => println!("REFUSED kind={:?} {error}", error.kind()),
    }
    if setup.contains("exit") {
        return;
    }
    live_out_role();
}

/// What a resident observes of itself, as `key=value` fields.
#[cfg(unix)]
fn describe_self() -> String {
    use std::os::fd::AsRawFd;
    // SAFETY: plain queries of this process's own state.
    let (pid, sid, parent) = unsafe { (libc::getpid(), libc::getsid(0), libc::getppid()) };
    let tty = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    {
        Ok(_) => "yes",
        Err(_) => "none",
    };
    let null = std::fs::metadata("/dev/null").unwrap();
    let mut streams_null = true;
    for fd in 0..3 {
        // SAFETY: `fstat` writes into a structure we own.
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::fstat(fd, &mut stat) } == 0;
        streams_null &= ok
            && std::os::unix::fs::MetadataExt::rdev(&null) == stat.st_rdev as u64
            && std::os::unix::fs::MetadataExt::ino(&null) == stat.st_ino as u64;
    }
    // Every descriptor above the lent one, but the folder this look opens.
    let listing = std::fs::File::open(if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    })
    .unwrap();
    let listing_fd = listing.as_raw_fd();
    drop(listing);
    let mut foreign = Vec::new();
    for fd in 4..1024 {
        if fd == listing_fd {
            continue;
        }
        // SAFETY: `fcntl` on a number this process may not hold fails.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0 {
            foreign.push(fd.to_string());
        }
    }
    let disposition = |signal| {
        // SAFETY: a query of this process's own signal state.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        unsafe { libc::sigaction(signal, std::ptr::null(), &mut current) };
        if current.sa_sigaction == libc::SIG_DFL {
            "default"
        } else {
            "changed"
        }
    };
    // SAFETY: a query of this thread's signal mask.
    let term_blocked = unsafe {
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut mask);
        libc::sigismember(&mask, libc::SIGTERM) == 1
    };
    format!(
        "pid={pid} sid={sid} parent={parent} tty={tty} streams_null={streams_null} \
         foreign=[{}] hup={} int={} term_blocked={term_blocked}",
        foreign.join(","),
        disposition(libc::SIGHUP),
        disposition(libc::SIGINT),
    )
}

#[cfg(windows)]
fn describe_self() -> String {
    use windows_sys::Win32::System::Console::{
        GetConsoleWindow, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut in_job = 0;
    // SAFETY: plain queries of this process's own state.
    let (asked, console, streams_null) = unsafe {
        let asked = IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job);
        let streams = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .all(|which| GetStdHandle(which).is_null());
        (asked, !GetConsoleWindow().is_null(), streams)
    };
    assert_ne!(asked, 0);
    let job = hide_platform::process::own_job()
        .unwrap()
        .map_or_else(|| "none".to_owned(), |limits| limits.to_string());
    format!(
        "pid={} in_job={} job={job} console={console} streams_null={streams_null}",
        std::process::id(),
        in_job != 0
    )
}

#[cfg(windows)]
mod windows_job {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    /// Puts this process in a new kill-on-close job, which lets a process
    /// leave it when `setup` says `breakaway`; held until this process
    /// exits, which closes the job and ends what is in it.
    pub(super) fn enter(setup: &str) -> Option<HANDLE> {
        if !setup.contains("job") {
            return None;
        }
        // SAFETY: plain calls; the limit structure is valid for its size.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            assert!(!job.is_null());
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | if setup.contains("breakaway") {
                    JOB_OBJECT_LIMIT_BREAKAWAY_OK
                } else {
                    0
                };
            assert_ne!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ),
                0
            );
            assert_ne!(AssignProcessToJobObject(job, GetCurrentProcess()), 0);
            Some(job)
        }
    }
}

/// Runs a starter with `setup` and answers it with the line it printed.
fn run_starter(setup: &str) -> (Owned, String) {
    let mut command = role_command("starter");
    command.env(SETUP, setup);
    let mut child = Owned(command.spawn().unwrap());
    let stdout = child.0.stdout.take().unwrap();
    let (sender, heard) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            for marker in ["STARTED ", "REFUSED "] {
                if let Some((_, rest)) = line.split_once(marker) {
                    let _ = sender.send(format!("{marker}{rest}"));
                    return;
                }
            }
        }
    });
    let line = heard
        .recv_timeout(HANG_LIMIT)
        .expect("the starter answered nothing");
    (child, line)
}

#[test]
fn a_resident_child_keeps_nothing_of_its_starter() {
    let setup = if cfg!(unix) {
        "own_session unix_extras"
    } else {
        ""
    };
    let (_starter, line) = run_starter(setup);
    assert!(line.starts_with("STARTED "), "{line}");
    let pid: u32 = field(&line, "pid").parse().unwrap();
    let resident = Started::new(pid);
    assert_eq!(field(&line, "streams_null"), "true", "{line}");
    #[cfg(unix)]
    {
        let starter: u32 = field(&line, "starter").parse().unwrap();
        let sid: u32 = field(&line, "sid").parse().unwrap();
        // A session of its own that it does not lead, so it can never take
        // a controlling terminal, and no child of the starter's.
        assert_ne!(sid, starter, "{line}");
        assert_ne!(sid, pid, "{line}");
        assert_ne!(field(&line, "parent"), starter.to_string(), "{line}");
        assert_eq!(field(&line, "tty"), "none", "{line}");
        assert_eq!(field(&line, "foreign"), "[]", "{line}");
        assert_eq!(field(&line, "hup"), "default", "{line}");
        assert_eq!(field(&line, "int"), "default", "{line}");
        assert_eq!(field(&line, "term_blocked"), "false", "{line}");
    }
    #[cfg(windows)]
    {
        // A job above the starter's (the runner's, or the test harness's)
        // may keep it; the start says so, and nothing else does.
        assert_eq!(field(&line, "in_job"), field(&line, "outer_job"), "{line}");
        assert_eq!(field(&line, "console"), "false", "{line}");
        // The jobs this run had, for the record: CI shows this line.
        let test_job = hide_platform::process::own_job()
            .unwrap()
            .map_or_else(|| "none".to_owned(), |limits| limits.to_string());
        println!("JOBS test_job={test_job} {line}");
    }
    assert!(resident.is_running());
}

/// The starter's group and session (Unix), or its kill-on-close job
/// (Windows), end; the resident does not.
#[test]
fn a_resident_child_outlives_what_ends_its_starter() {
    let setup = if cfg!(unix) {
        "own_session"
    } else {
        "job breakaway exit"
    };
    let (mut starter, line) = run_starter(setup);
    assert!(line.starts_with("STARTED "), "{line}");
    let pid: u32 = field(&line, "pid").parse().unwrap();
    let resident = Started::new(pid);
    #[cfg(unix)]
    {
        // What a closed terminal and a killed group send the starter's
        // session: the hangup, then the kill, to its whole group.
        let group = -(starter.0.id() as libc::pid_t);
        // SAFETY: plain calls naming the starter's own group.
        unsafe {
            libc::kill(group, libc::SIGHUP);
            libc::kill(group, libc::SIGKILL);
        }
    }
    // On Windows the starter exits, which closes its job; a job above it
    // may keep the resident, and the start said so.
    starter.0.wait().unwrap();
    #[cfg(windows)]
    assert_eq!(field(&line, "in_job"), field(&line, "outer_job"), "{line}");
    // Nothing more ends it: it still runs once its starter is gone.
    assert!(resident.is_running(), "the resident ended with its starter");
}

#[cfg(windows)]
#[test]
fn a_start_inside_a_job_that_keeps_its_processes_is_refused() {
    let (_starter, line) = run_starter("job");
    assert!(line.starts_with("REFUSED "), "{line}");
    assert_eq!(field(&line, "kind"), "PermissionDenied", "{line}");
    // The refusal names the job that kept it.
    assert!(
        line.contains("kill_on_close=true,breakaway_ok=false"),
        "{line}"
    );
}

/// A child another thread starts while a resident starts keeps its own
/// pipe: the resident holds no copy of it, so the reader sees its end once
/// that child exits.
#[test]
fn children_started_beside_a_resident_keep_their_own_pipes() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let beside = {
        let stop = stop.clone();
        thread::spawn(move || {
            let mut rounds = 0;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) || rounds < 10 {
                let mut child = Owned(role_command("quick").spawn().unwrap());
                let mut stdout = child.0.stdout.take().unwrap();
                let (sender, ended) = mpsc::channel();
                thread::spawn(move || {
                    let mut all = String::new();
                    let _ = stdout.read_to_string(&mut all);
                    let _ = sender.send(all);
                });
                let all = ended
                    .recv_timeout(HANG_LIMIT)
                    .expect("a child's pipe stayed open past its end");
                assert!(all.contains("QUICK"), "{all}");
                child.0.wait().unwrap();
                rounds += 1;
            }
        })
    };
    let mut residents = Vec::new();
    for _ in 0..4 {
        let (started, report) = start_reporting_resident();
        #[cfg(unix)]
        assert_eq!(field(&report, "foreign"), "[]", "{report}");
        let _ = report;
        residents.push(started);
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    beside.join().unwrap();
}

/// The pids a session member wrote to `path`, once it wrote `count`.
fn pids_in(path: &Path, count: usize) -> Vec<u32> {
    let read = || -> Vec<u32> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|pid| pid.parse().ok())
            .collect()
    };
    wait_until("the member's report", || read().len() >= count);
    read()
}

#[cfg(unix)]
fn session_leader() {
    // SAFETY: setsid takes nothing and has no memory effects.
    assert!(unsafe { libc::setsid() } >= 0);
    let mut stubborn = role_command("stubborn")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    println!("READY {}", stubborn.id());
    if std::env::var(SETUP).is_ok_and(|setup| setup == "exit") {
        // The shell exits on its own and leaves its child behind.
        std::mem::forget(stubborn);
        return;
    }
    let _ = stubborn.wait();
}

/// Ignores the hangup and the terminate signal, and starts another stubborn
/// process for each one it is sent: a member that starts a grandchild as
/// its session is ended.
#[cfg(unix)]
fn stubborn() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    extern "C" fn count(_: libc::c_int) {
        ASKED.fetch_add(1, Ordering::SeqCst);
    }
    // SAFETY: the handler only adds to an atomic.
    unsafe {
        libc::signal(libc::SIGHUP, count as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, count as *const () as libc::sighandler_t);
    }
    let path = std::env::var_os(PIDS_FILE).unwrap();
    let record = |pid: u32| {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{pid}").unwrap();
    };
    record(std::process::id());
    let mut started = 0;
    let deadline = Instant::now() + ROLE_LIFE;
    while Instant::now() < deadline {
        if ASKED.load(Ordering::SeqCst) > started && started < 4 {
            started += 1;
            let child = role_command("stubborn")
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            record(child.id());
            std::mem::forget(child);
        }
        #[allow(clippy::disallowed_methods)] // the role's own pace
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
fn job_leader() {
    // Waits until the test has put it in the session's job.
    let mut go = [0u8; 1];
    std::io::stdin().read_exact(&mut go).unwrap();
    let path = std::env::var_os(PIDS_FILE).unwrap();
    let mut pids = vec![std::process::id()];
    for _ in 0..2 {
        let child = role_command("sleep").stdout(Stdio::null()).spawn().unwrap();
        pids.push(child.id());
        std::mem::forget(child);
    }
    let listed: Vec<String> = pids.iter().map(u32::to_string).collect();
    std::fs::write(&path, listed.join("\n")).unwrap();
    live_out_role();
}

/// Starts a session's leader the way a terminal does, adopts it, and
/// answers the leader and the pids its members recorded.
fn start_session(session: &mut Session, setup: &str, pids: &Path) -> (Owned, Vec<u32>) {
    #[cfg(unix)]
    {
        let mut command = role_command("session_leader");
        command.env(SETUP, setup).env(PIDS_FILE, pids);
        let mut leader = Owned(command.spawn().unwrap());
        // The leader calls setsid first thing; adopt it once it has.
        let stubborn = line_after(leader.0.stdout.take().unwrap(), "READY ");
        let _ = stubborn;
        session.adopt(leader.0.id()).unwrap();
        let members = pids_in(pids, 1);
        (leader, members)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let _ = setup;
        let mut command = role_command("job_leader");
        command.env(PIDS_FILE, pids).stdin(Stdio::piped());
        let mut leader = Owned(command.spawn().unwrap());
        // SAFETY: both handles are open.
        let assigned = unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                session.job().cast(),
                leader.0.as_raw_handle().cast(),
            )
        };
        assert_ne!(assigned, 0, "{}", std::io::Error::last_os_error());
        session.adopt(leader.0.id()).unwrap();
        leader.0.stdin.take().unwrap().write_all(b"g").unwrap();
        let members = pids_in(pids, 3);
        (leader, members)
    }
}

#[test]
fn ending_a_session_ends_every_member_and_what_they_start_meanwhile() {
    let folder = tempfile::tempdir().unwrap();
    let pids = folder.path().join("pids");
    let outsider = Owned(role_command("sleep").spawn().unwrap());
    let mut session = Session::new().unwrap();
    let (mut leader, members) = start_session(&mut session, "", &pids);
    let named = pinned(&members);
    assert_eq!(named.len(), members.len(), "a member ended before the end");
    assert!(session.members().unwrap().contains(&members[0]));

    assert_eq!(session.end(END_LIMIT).unwrap(), SessionEnd::Ended);
    let _ = leader.0.wait();
    for member in &named {
        assert!(
            !member.is_running(),
            "member {} outlived its session's end",
            member.pid
        );
    }
    // Those started as the end was sent were recorded after the members
    // were named; one that runs now is one the end missed.
    for pid in pids_in(&pids, members.len()) {
        assert!(
            Pinned::of(pid).is_none(),
            "member {pid} outlived its session's end"
        );
    }
    assert!(session.members().unwrap().is_empty());
    #[cfg(unix)]
    assert!(
        pids_in(&pids, 1).len() > 1,
        "a member started nothing as it was asked to end"
    );
    assert!(
        is_alive(outsider.0.id()),
        "the end reached a process outside the session"
    );
}

#[cfg(unix)]
#[test]
fn a_session_whose_leader_exited_still_ends_what_it_left() {
    let folder = tempfile::tempdir().unwrap();
    let pids = folder.path().join("pids");
    let mut session = Session::new().unwrap();
    let (mut leader, members) = start_session(&mut session, "exit", &pids);
    let named = pinned(&members);
    leader.0.wait().unwrap();
    assert!(
        named.iter().any(Pinned::is_running),
        "the leader's child ended with it"
    );
    assert_eq!(session.end(END_LIMIT).unwrap(), SessionEnd::Ended);
    for member in &named {
        assert!(
            !member.is_running(),
            "member {} outlived its session's end",
            member.pid
        );
    }
    for pid in pids_in(&pids, 1) {
        assert!(
            Pinned::of(pid).is_none(),
            "member {pid} outlived its session's end"
        );
    }
}

/// On Linux a program file stays busy while any process holds it open for
/// writing, which a child another thread forked does from its fork until it
/// runs its own program; a start waits that out rather than failing with
/// `ETXTBSY`, as when a node copies a holder into its version folder while it
/// starts another. Here a process keeps the writer's descriptor past its own
/// start, for less than the start's retries last. macOS runs a program file
/// open for writing, so there is nothing to wait out there.
#[cfg(target_os = "linux")]
#[test]
fn a_start_waits_out_a_program_another_process_holds_open_for_writing() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    const HOLD: Duration = Duration::from_millis(100);
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("resident");
    let mut file = std::fs::File::create(&program).unwrap();
    file.write_all(&std::fs::read(std::env::current_exe().unwrap()).unwrap())
        .unwrap();
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
        .unwrap();
    let written = file.as_raw_fd();
    let mut holder = Command::new("/bin/sh");
    holder.args(["-c", &format!("exec sleep {}", HOLD.as_secs_f64())]);
    // SAFETY: `fcntl` is async-signal-safe; it clears close-on-exec on the
    // child's copy only, so the sleep holds the file open for writing.
    unsafe {
        holder.pre_exec(move || {
            if libc::fcntl(written, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let held_from = Instant::now();
    let mut holder = Owned(holder.spawn().unwrap());
    drop(file);
    let (resident, report) = start_reporting(&resident_command_of(&program));
    // The start ran only once nothing held the file for writing.
    assert!(held_from.elapsed() >= HOLD, "{report}");
    assert!(holder.0.try_wait().unwrap().is_some(), "{report}");
    assert!(resident.is_running(), "{report}");
}
