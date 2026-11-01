//! An idle holder does not wake (PRD engine-bundle-2-pane-holder B21): with
//! its program waiting for input, attached or not, the holder runs nothing,
//! by what the kernel counts for it, once the one return of freed pages a
//! second after its last fragment is behind it (`holder.memory_returned` in
//! its log). The local measurement watches 60 s (docs/PERFORMANCE_TESTING.md,
//! The pane holder's path); this is the short form every system's CI runs.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::support::{Home, current_version, print_ok, shell, source, start_ready};
use hide_holder_protocol::ClientMessage;

/// How long the holder is watched in each state; a periodic timer of up to
/// this length shows as a count above zero.
const WATCHED: Duration = Duration::from_secs(10);

/// What the kernel counts each time the process `pid` runs, by thread where
/// the kernel counts by thread: on Linux each thread's context switches, by
/// thread id; on macOS the process's wakeups from idle and from interrupts,
/// and on Windows the processor cycles its threads used, each under 0.
///
/// A thread that ends is work the holder did, so it is part of the state
/// read here, never an error: on Linux the count of a thread that ended
/// leaves the sum with it, and a thread can end between listing the
/// process's threads and reading one (with-hide/hide-private#409), which
/// reads as the thread gone. In an idle holder the threads that end are
/// `connection-opening`, once the hello it read is handed to the holder, and
/// `connection-reader` and `connection-writer`, once the connection is
/// closed (a release); the PTY's reader and writer, the program's waiter and
/// the listener live as long as the pane.
fn runs(pid: u32) -> BTreeMap<u64, u64> {
    #[cfg(target_os = "linux")]
    {
        let mut threads = BTreeMap::new();
        for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
            let task = match task {
                Ok(task) => task,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => panic!("the holder's threads could not be listed: {error}"),
            };
            let status = match std::fs::read_to_string(task.path().join("status")) {
                Ok(status) => status,
                // The thread ended since the listing.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => panic!("{} could not be read: {error}", task.path().display()),
            };
            let tid: u64 = task.file_name().to_string_lossy().parse().unwrap();
            let mut total = 0;
            for line in status.lines() {
                if let Some(count) = line
                    .strip_prefix("voluntary_ctxt_switches:")
                    .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
                {
                    total += count.trim().parse::<u64>().unwrap();
                }
            }
            threads.insert(tid, total);
        }
        threads
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: an all-zero rusage_info_v2 is a valid value, and the
        // buffer is that structure, the flavor asked for.
        let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
        let asked = unsafe {
            libc::proc_pid_rusage(
                pid as libc::c_int,
                libc::RUSAGE_INFO_V2,
                (&mut info as *mut libc::rusage_info_v2).cast(),
            )
        };
        assert_eq!(asked, 0, "{}", std::io::Error::last_os_error());
        BTreeMap::from([(0, info.ri_pkg_idle_wkups + info.ri_interrupt_wkups)])
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows_sys::Win32::System::WindowsProgramming::QueryProcessCycleTime;
        // SAFETY: plain calls; the handle is closed before returning.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!process.is_null(), "{}", std::io::Error::last_os_error());
            let mut cycles = 0u64;
            let asked = QueryProcessCycleTime(process, &mut cycles);
            let error = std::io::Error::last_os_error();
            CloseHandle(process);
            assert_ne!(asked, 0, "{error}");
            BTreeMap::from([(0, cycles)])
        }
    }
}

/// What the holder ran between `before` and `after`: what each thread that
/// lived through both ran, and one for each thread that ended or started.
fn ran(before: &BTreeMap<u64, u64>, after: &BTreeMap<u64, u64>) -> u64 {
    let mut total = 0;
    for (thread, count) in before {
        total += match after.get(thread) {
            Some(later) => later - count,
            None => 1,
        };
    }
    total
        + after
            .keys()
            .filter(|thread| !before.contains_key(thread))
            .count() as u64
}

/// How long the holder must have run nothing for the work an event started
/// to be over.
const QUIET: Duration = Duration::from_millis(200);

/// Lets `length` pass.
fn settle(length: Duration) {
    let until = Instant::now() + length;
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        if left.is_zero() {
            break;
        }
        std::thread::park_timeout(left);
    }
}

/// Waits until the holder has run nothing for `QUIET`: the end of the work
/// an event started, as the kernel's count shows it. A holder that never
/// goes quiet fails here, as one that wakes on a timer fails the watch.
fn quiet(pid: u32) {
    let started = Instant::now();
    let mut last = runs(pid);
    loop {
        settle(QUIET);
        let now = runs(pid);
        if now == last {
            return;
        }
        last = now;
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the holder never went quiet"
        );
    }
}

/// What the holder ran while `WATCHED` passed.
fn watched(pid: u32) -> u64 {
    let before = runs(pid);
    settle(WATCHED);
    ran(&before, &runs(pid))
}

#[test]
fn an_idle_holder_does_not_wake_attached_or_not() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let holder = start_ready(&folder.program, &home.socket("p"), &shell(), &[]);
    drop(folder);
    let mut connection = holder.open();
    connection
        .send(ClientMessage::Input(print_ok().to_vec()))
        .unwrap();
    connection.wait_for_text("hide-ok");
    // What the line drew has gone out, and a second later the writer gave
    // back what drawing it freed, once (`RETURN_FREED_AFTER`); the holder
    // waits on its program and its connection now, with no bound.
    holder.wait_logged("holder.memory_returned", 1, &mut connection);
    quiet(holder.process.pid);
    let attached = watched(holder.process.pid);
    // Let go of: the holder closes the connection and its writer ends.
    connection.send(ClientMessage::Release).unwrap();
    connection.wait_closed();
    drop(connection);
    quiet(holder.process.pid);
    let alone = watched(holder.process.pid);
    eprintln!("the idle holder ran {attached} attached and {alone} alone over {WATCHED:?} each");
    assert_eq!(attached, 0, "the holder woke while attached and idle");
    assert_eq!(alone, 0, "the holder woke while idle with nothing attached");
    assert!(holder.process.is_running());
}
