//! What a flood leaves a holder holding (PRD engine-bundle-2-pane-holder
//! B15, D-17): once its screen has been still for a second, a holder gives
//! the allocator's freed pages back. Run beside a holder that keeps what it
//! freed (`HIDE_HOLDER_TEST_KEEP_FREED`, debug builds only), it prints both
//! on every system and holds the return to having saved something on macOS
//! and Linux. The bound itself, 6 MiB after a flood, is a release holder's
//! and is judged by the measurement (docs/PERFORMANCE_TESTING.md, The pane
//! holder's path): a debug holder's own allocations move its figure by
//! megabytes from run to run.

use std::ffi::OsString;
use std::time::Duration;

use crate::role::{FLOOD_BYTES, GO};
use crate::support::{Connection, Home, ROLE, current_version, role_program, source, start_ready};

/// The flood: well past the 2 MiB scrollback, so the parser's pages are
/// full; smaller on Windows, whose ConPTY carries about 0.8 MB a second on a
/// runner, inside the 30 s a wait for text takes.
const FLOOD: &str = if cfg!(windows) { "8000000" } else { "30000000" };

/// The memory the system charges `pid` with: its footprint on macOS, its
/// anonymous resident pages on Linux, its private bytes on Windows.
fn charged(pid: u32) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        let kib: u64 = status
            .lines()
            .find_map(|line| line.strip_prefix("RssAnon:"))
            .unwrap()
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .unwrap();
        kib * 1024
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
        info.ri_phys_footprint
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: plain calls; the counters are valid for the size passed and
        // the handle is closed before returning.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!process.is_null(), "{}", std::io::Error::last_os_error());
            let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            let asked = GetProcessMemoryInfo(
                process,
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            );
            let error = std::io::Error::last_os_error();
            CloseHandle(process);
            assert_ne!(asked, 0, "{error}");
            counters.PrivateUsage as u64
        }
    }
}

/// Lets the holder's screen stay still past its one-second wait, with room
/// for a loaded runner, taking what it sends meanwhile: a writer blocked on a
/// full connection never reaches that wait.
fn still(connection: &mut Connection) {
    let until = std::time::Instant::now() + Duration::from_millis(2500);
    while std::time::Instant::now() < until {
        connection.drain();
        std::thread::park_timeout(Duration::from_millis(20));
    }
}

/// Floods one holder and answers what it was charged before and after, once
/// its screen was still.
fn flooded(home: &Home, name: &str, keep_freed: bool) -> (u64, u64) {
    let folder = home.prepare(current_version(), &source());
    let go = home.path().join(format!("go-{name}"));
    let role: OsString = "flood_when_told".into();
    let go_value: OsString = go.clone().into();
    let bytes: OsString = FLOOD.into();
    let keep: OsString = "1".into();
    let mut variables = vec![(ROLE, &role), (GO, &go_value), (FLOOD_BYTES, &bytes)];
    if keep_freed {
        variables.push(("HIDE_HOLDER_TEST_KEEP_FREED", &keep));
    }
    let holder = start_ready(
        &folder.program,
        &home.socket(name),
        &role_program("flood_when_told"),
        &variables,
    );
    drop(folder);
    let mut connection = holder.open();
    still(&mut connection);
    let before = charged(holder.process.pid);
    std::fs::write(&go, b"").unwrap();
    connection.wait_for_text("FLOOD DONE");
    still(&mut connection);
    let after = charged(holder.process.pid);
    assert!(holder.process.is_running());
    (before, after)
}

#[test]
fn a_holder_gives_back_what_a_flood_freed_once_its_screen_is_still() {
    let home = Home::new();
    let (kept_before, kept_after) = flooded(&home, "kept", true);
    let (before, after) = flooded(&home, "given", false);
    let mib = |bytes: u64| bytes as f64 / 1048576.0;
    eprintln!(
        "after a {FLOOD}-byte flood: kept {:.2} -> {:.2} MiB, given back {:.2} -> {:.2} MiB",
        mib(kept_before),
        mib(kept_after),
        mib(before),
        mib(after)
    );
    // Windows: whether `HeapCompact` saves anything is what this prints.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    assert!(
        after < kept_after,
        "giving freed pages back saved nothing: {:.2} MiB against {:.2}",
        mib(after),
        mib(kept_after)
    );
}
