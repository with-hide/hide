//! An idle holder does not wake (PRD engine-bundle-2-pane-holder B21): with
//! its program waiting for input, attached or not, the holder runs nothing,
//! by what the kernel counts for it. The local measurement watches 60 s
//! (docs/PERFORMANCE_TESTING.md, The pane holder's path); this is the short
//! form every system's CI runs.

use std::time::{Duration, Instant};

use crate::support::{Home, current_version, print_ok, shell, source, start_ready};
use hide_holder_protocol::ClientMessage;

/// How long the holder is watched in each state; a periodic timer of up to
/// this length shows as a count above zero.
const WATCHED: Duration = Duration::from_secs(10);

/// What the kernel counts each time a process of `pid` runs: on Linux its
/// threads' context switches, on macOS its wakeups from idle and from
/// interrupts, on Windows the processor cycles its threads used.
fn runs(pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let mut total = 0;
        for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
            let status = std::fs::read_to_string(task.unwrap().path().join("status")).unwrap();
            for line in status.lines() {
                if let Some(count) = line
                    .strip_prefix("voluntary_ctxt_switches:")
                    .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
                {
                    total += count.trim().parse::<u64>().unwrap();
                }
            }
        }
        total
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
        info.ri_pkg_idle_wkups + info.ri_interrupt_wkups
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
            cycles
        }
    }
}

/// What the holder ran while `WATCHED` passed.
fn watched(pid: u32) -> u64 {
    let before = runs(pid);
    let until = Instant::now() + WATCHED;
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        if left.is_zero() {
            break;
        }
        std::thread::park_timeout(left);
    }
    runs(pid) - before
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
    // What the line drew has gone out; the holder waits on its program and
    // its connection now.
    std::thread::park_timeout(Duration::from_millis(500));
    let attached = watched(holder.process.pid);
    connection.send(ClientMessage::Release).unwrap();
    drop(connection);
    std::thread::park_timeout(Duration::from_millis(500));
    let alone = watched(holder.process.pid);
    eprintln!("the idle holder ran {attached} attached and {alone} alone over {WATCHED:?} each");
    assert_eq!(attached, 0, "the holder woke while attached and idle");
    assert_eq!(alone, 0, "the holder woke while idle with nothing attached");
    assert!(holder.process.is_running());
}
