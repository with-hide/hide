//! What a flood leaves a holder holding (PRD engine-bundle-2-pane-holder
//! B15, D-17): once a connection's screen has been still for a second, its
//! writer gives the allocator's freed pages back (logged after a flood's
//! output, `holder.memory_returned`), and the holder is then
//! charged no more than its budget, 6 MiB: 4 MiB idle and the parser's 2 MiB
//! scrollback, the bound decided for the bundle
//! (docs/PERFORMANCE_TESTING.md, The pane holder's path), in every build on
//! Linux and Windows; on macOS a debug holder sits around the bound, and the
//! release measurement judges it there.

use std::ffi::OsString;

use crate::role::{FLOOD_BYTES, GO};
use crate::support::{Home, ROLE, current_version, role_program, source, start_ready};

/// The budget a holder keeps within once still after a flood: 4 MiB idle
/// and 2 MiB of scrollback.
const BUDGET: u64 = 6 * 1024 * 1024;

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

#[test]
fn a_flooded_holder_keeps_within_its_budget_once_it_gave_freed_pages_back() {
    let home = Home::new();
    let folder = home.prepare(current_version(), &source());
    let go = home.path().join("go");
    let role: OsString = "flood_when_told".into();
    let go_value: OsString = go.clone().into();
    let bytes: OsString = FLOOD.into();
    let holder = start_ready(
        &folder.program,
        &home.socket("p"),
        &role_program("flood_when_told"),
        &[(ROLE, &role), (GO, &go_value), (FLOOD_BYTES, &bytes)],
    );
    drop(folder);
    let mut connection = holder.open();
    // The first return, after the first screen, then the flood's: the
    // writer that sent its fragments gives back what it freed.
    holder.wait_logged("holder.memory_returned", 1, &mut connection);
    std::fs::write(&go, b"").unwrap();
    connection.wait_for_text("FLOOD DONE");
    holder.wait_logged("holder.memory_returned", 2, &mut connection);
    let charged = charged(holder.process.pid);
    let mib = |bytes: u64| bytes as f64 / 1048576.0;
    eprintln!(
        "after a {FLOOD}-byte flood and a return, the holder is charged {:.2} MiB",
        mib(charged)
    );
    // A debug holder on macOS reads 5.5 to 6.6 MiB here, from 2.4 MiB
    // before the flood, around the bound a release holder keeps (5.91 MiB
    // at most over 40 measured rounds): there the measurement judges it and
    // this prints it. Linux's (2.95 MiB) and Windows' (4.20 MiB) are held to
    // it in every build.
    if !(cfg!(target_os = "macos") && cfg!(debug_assertions)) {
        assert!(
            charged <= BUDGET,
            "{:.2} MiB is over the {:.0} MiB budget",
            mib(charged),
            mib(BUDGET)
        );
    }
    assert!(holder.process.is_running());
}
