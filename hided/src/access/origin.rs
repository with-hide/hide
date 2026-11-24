//! Kernel proof of a caller under a holder, including another machine's
//! holders on this account. No guest build's internal state is read.

use std::path::{Path, PathBuf};

use hide_platform::fs::identity::canonical;
use hide_platform::process;

pub(super) const UNAVAILABLE: &str = "caller_origin_unavailable";

/// Bounds the work of proving one live ancestry, as the platform's lineage
/// does. At the bound there is no complete proof, so the ask is refused;
/// no process or record is dropped. This runs only at bootstrap, off-lock.
const MAX_ANCESTORS: usize = 32;

fn directory(path: &Path) -> Result<(), &'static str> {
    if std::fs::metadata(path).map_err(|_| UNAVAILABLE)?.is_dir() {
        Ok(())
    } else {
        Err(UNAVAILABLE)
    }
}

fn root(path: &Path) -> Result<Option<PathBuf>, &'static str> {
    match canonical(path) {
        Ok(path) => {
            directory(&path)?;
            Ok(Some(path))
        }
        // No installation can run from a root that has never existed. A
        // running image whose root was removed fails its own canonical read.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Windows also reports PATH_NOT_FOUND when a prefix is a file.
            // Prove the nearest existing prefix is a directory; a broken
            // link, non-directory or unreadable prefix is not an empty root.
            for ancestor in path.ancestors() {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(_) => {
                        directory(ancestor)?;
                        return Ok(None);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(UNAVAILABLE),
                }
            }
            Err(UNAVAILABLE)
        }
        Err(_) => Err(UNAVAILABLE),
    }
}

/// [`UNAVAILABLE`] for a walk that could not read `step` of `pid`, with the
/// step and the system's error in the log, which is all that says why an
/// operator's command was refused.
fn unread(step: &'static str, pid: u32, error: &std::io::Error) -> &'static str {
    hide_core::diagnostic!(serde_json::json!({
        "component": "access",
        "kind": "access.caller_origin_unavailable",
        "step": step,
        "pid": pid,
        "error": error.to_string(),
        "os_error": error.raw_os_error(),
    }));
    UNAVAILABLE
}

/// The image of `pid`, a process of this account, from what the kernel
/// answered for it (`read`): `None` when the kernel refuses it to this
/// account, and any other failure refuses the walk.
///
/// Linux names a process's image only to an account that may trace it, and
/// a process that changed accounts on its way to this one may not be traced
/// by it: the process a root sshd keeps for each login, which every command
/// that arrives over SSH descends from. Such a process is none of this
/// home's holders, which this account starts from its own files and so
/// always reads, and the walk goes on to its parent, whose account and
/// birth it still has to read. A pane's program that takes that right from
/// itself is walked past the same way, to the holder above it. The rule is
/// the same on every system and is reached on Linux alone: macOS names a
/// process's image to any account, and on Windows the right that names the
/// image is the one the account and the birth need, which refuse first.
fn image_read(pid: u32, read: std::io::Result<PathBuf>) -> Result<Option<PathBuf>, &'static str> {
    match read {
        Ok(image) => canonical(&image)
            .map(Some)
            .map_err(|error| unread("image", pid, &error)),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
        Err(error) => Err(unread("image", pid, &error)),
    }
}

/// The walk's answer for a caller it could not read at all: on Windows,
/// [`Origin::Hidden`] when the system confirms the caller keeps this
/// account out of its access list too; anything else is unavailable.
fn hidden(pid: u32, error: std::io::Error) -> Result<Origin, &'static str> {
    #[cfg(windows)]
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return match process::owned_elsewhere(pid) {
            Ok(true) => Ok(Origin::Hidden),
            Ok(false) => Err(unread("caller_owned", pid, &error)),
            Err(other) => Err(unread("caller_owner", pid, &other)),
        };
    }
    Err(unread("start", pid, &error))
}

/// Every Hide holder of this home counts as a pane. Guest holders use the
/// host-owned guest placement root, including development builds and work
/// retained after removal. Helper holders inherited at a split use the
/// home's ordinary holder root. These are placement boundaries, not a
/// parser for any file or folder inside a guest's node.
/// Where the walk from a caller ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Origin {
    /// Under one of this home's holders: a pane.
    Holder,
    /// At another account's process, the account boundary: outside.
    Outside,
    /// Windows: the caller itself is a process the system will not show
    /// this account and whose access list keeps this account out, as an
    /// administrator's SSH login is to the desktop session's daemon
    /// (`process::owned_elsewhere`). Outside, as far as the walk can say;
    /// nothing of its ancestry was read, so a check that needs it cannot
    /// clear it.
    /// Only Windows hides a caller from its own account.
    #[cfg_attr(not(windows), allow(dead_code))]
    Hidden,
}

pub(super) fn in_holder(peer: i32, home: &Path, state: &Path) -> Result<Origin, &'static str> {
    let mut pid = u32::try_from(peer)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or(UNAVAILABLE)?;
    let caller = pid;
    let roots = [
        root(&hide_kit::layout::holders_root(home))?,
        root(&hide_kit::layout::guests(state))?,
    ];
    let mut started = match process::start_time(pid) {
        Ok(started) => started,
        Err(error) => return hidden(pid, error),
    };
    let moved = |pid: u32| {
        hide_core::diagnostic!(serde_json::json!({
            "component": "access",
            "kind": "access.caller_origin_unavailable",
            "step": "moved",
            "pid": pid,
        }));
        UNAVAILABLE
    };
    for _ in 0..MAX_ANCESTORS {
        let same_account = match process::same_account(pid) {
            Ok(same) => same,
            Err(error) if pid == caller => return hidden(pid, error),
            Err(error) => return Err(unread("account", pid, &error)),
        };
        if process::start_time(pid).map_err(|error| unread("start", pid, &error))? != started {
            return Err(moved(pid));
        }
        // A root sshd above the account's shell is outside this account's
        // holders. Setuid intermediates retaining our real/saved identity
        // do not form this boundary. Unknown identity never permits an ask.
        if !same_account {
            return Ok(Origin::Outside);
        }
        let image = image_read(pid, process::image_path(pid))?;
        if process::start_time(pid).map_err(|error| unread("start", pid, &error))? != started {
            return Err(moved(pid));
        }
        if image.is_some_and(|image| roots.iter().flatten().any(|root| image.starts_with(root))) {
            return Ok(Origin::Holder);
        }
        let parent = process::parent_of(pid);
        if process::start_time(pid).map_err(|error| unread("start", pid, &error))? != started {
            return Err(moved(pid));
        }
        let parent = match parent {
            Ok(parent) => parent,
            // Windows retains a dead parent's old pid. No live ancestry
            // remains beyond it, as for the accepted detached-process case.
            #[cfg(windows)]
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Origin::Outside);
            }
            // Windows will not describe another account's process to this
            // one: the SSH service's own process, which runs as SYSTEM and
            // started this account's login. That is the account boundary a
            // root sshd is on Unix, once the system confirms the parent is
            // not this account's; a parent this account owns, or one that
            // cannot be read at all, stays refused.
            #[cfg(windows)]
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return match process::started_by_other_account(pid) {
                    Ok(true) => Ok(Origin::Outside),
                    Ok(false) => Err(unread("parent_owned", pid, &error)),
                    Err(error) => Err(unread("parent_owner", pid, &error)),
                };
            }
            Err(error) => return Err(unread("parent", pid, &error)),
        };
        if parent <= 1 {
            return Ok(Origin::Outside);
        }
        let parent_started =
            process::start_time(parent).map_err(|error| unread("start", parent, &error))?;
        if parent == pid || parent_started > started {
            return Err(moved(parent));
        }
        pid = parent;
        started = parent_started;
    }
    hide_core::diagnostic!(serde_json::json!({
        "component": "access",
        "kind": "access.caller_origin_unavailable",
        "step": "too_deep",
        "pid": pid,
    }));
    Err(UNAVAILABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_process_is_not_an_outside_terminal() {
        let root = tempfile::tempdir().unwrap();
        for pid in [-1, 0, i32::MAX] {
            assert_eq!(in_holder(pid, root.path(), root.path()), Err(UNAVAILABLE));
        }
    }

    #[test]
    fn unreadable_ownership_is_not_an_empty_guest_list() {
        let root = tempfile::tempdir().unwrap();
        // ENOTDIR, as opposed to an installation never having been made.
        let home = root.path().join("not-a-home");
        std::fs::write(&home, "broken ownership").unwrap();
        assert_eq!(
            in_holder(std::process::id() as i32, &home, root.path()),
            Err(UNAVAILABLE)
        );
        assert_eq!(super::root(&home), Err(UNAVAILABLE));
        assert_eq!(super::root(&root.path().join("never-installed")), Ok(None));
        #[cfg(unix)]
        {
            let broken = root.path().join("broken-link");
            std::os::unix::fs::symlink(root.path().join("gone"), &broken).unwrap();
            assert_eq!(super::root(&broken.join("holders")), Err(UNAVAILABLE));
        }
    }

    // Only an image the kernel refuses this account is walked past; one it
    // cannot name for any other reason (a removed file, an ended process)
    // refuses the walk, as an ancestor's unread parent or birth does.
    #[test]
    fn only_an_image_the_kernel_refuses_this_account_is_walked_past() {
        use std::io::{Error, ErrorKind};
        let own = std::process::id();
        assert_eq!(
            image_read(own, Err(Error::from(ErrorKind::PermissionDenied))),
            Ok(None)
        );
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::Other,
            ErrorKind::Unsupported,
        ] {
            assert_eq!(image_read(own, Err(Error::from(kind))), Err(UNAVAILABLE));
        }
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            image_read(own, Ok(root.path().join("removed"))),
            Err(UNAVAILABLE)
        );
        let image = std::env::current_exe().unwrap();
        assert_eq!(
            image_read(own, Ok(image.clone())),
            Ok(Some(canonical(&image).unwrap()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_proved_foreign_ancestor_needs_no_access_to_its_image() {
        // PID 1 is a real other-account process for an unprivileged test.
        // It exercises the boundary used at a normal SSH login's root
        // sshd, without starting a privileged server on the workstation.
        // SAFETY: getuid takes no arguments and cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        assert_eq!(in_holder(1, root.path(), root.path()), Ok(Origin::Outside));
    }
    /// The walk on Windows, over real processes whose access lists the test
    /// sets: where it ends and what it refuses decides what an operator's
    /// SSH login, and a pane, may ask.
    #[cfg(windows)]
    mod windows {
        use super::*;
        use std::process::{Child, Command, Stdio};

        /// A `cmd` that runs `program` (a long `ping`), and the pid of
        /// `program` under it.
        struct Tree {
            shell: Child,
            caller: u32,
            /// Opened before any access list changes, so the test can end
            /// the caller whatever it is set to.
            ending: windows_sys::Win32::Foundation::HANDLE,
        }

        impl Tree {
            fn new(program: &Path) -> Self {
                let shell = Command::new("cmd.exe")
                    .arg("/c")
                    .arg(program)
                    .args(["-n", "120", "127.0.0.1"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                let name = program.file_name().unwrap().to_ascii_lowercase();
                let started = std::time::Instant::now();
                let caller = loop {
                    let found = process::descendants(shell.id())
                        .unwrap()
                        .into_iter()
                        .find(|pid| {
                            process::image_path(*pid).is_ok_and(|image| {
                                image.file_name().map(|file| file.to_ascii_lowercase())
                                    == Some(name.clone())
                            })
                        });
                    if let Some(pid) = found {
                        break pid;
                    }
                    assert!(
                        started.elapsed() < std::time::Duration::from_secs(30),
                        "the shell started its program"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(50)); // wait-ok: a child process the test started appears in the table
                };
                // SAFETY: a plain open of a process this test started.
                let ending = unsafe {
                    windows_sys::Win32::System::Threading::OpenProcess(
                        windows_sys::Win32::System::Threading::PROCESS_TERMINATE,
                        0,
                        caller,
                    )
                };
                assert!(!ending.is_null(), "{}", std::io::Error::last_os_error());
                Self {
                    shell,
                    caller,
                    ending,
                }
            }

            fn shell(&self) -> u32 {
                self.shell.id()
            }
        }

        impl Drop for Tree {
            fn drop(&mut self) {
                // SAFETY: the handle was opened in `new` and is closed here
                // once; the child's own handle carries every right.
                unsafe {
                    windows_sys::Win32::System::Threading::TerminateProcess(self.ending, 1);
                    windows_sys::Win32::Foundation::CloseHandle(self.ending);
                }
                let _ = self.shell.kill();
                let _ = self.shell.wait();
            }
        }

        fn ping() -> PathBuf {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32")
                .join("PING.EXE")
        }

        /// Sets `pid`'s access list to `sddl`: a test's own child, which it
        /// owns and may change.
        fn set_access(pid: u32, sddl: &str) {
            use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
            use windows_sys::Win32::Security::Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                SE_KERNEL_OBJECT, SetSecurityInfo,
            };
            use windows_sys::Win32::Security::{
                DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
                PROTECTED_DACL_SECURITY_INFORMATION,
            };
            use windows_sys::Win32::Storage::FileSystem::WRITE_DAC;
            use windows_sys::Win32::System::Threading::OpenProcess;

            let text: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
            let mut descriptor = std::ptr::null_mut();
            // SAFETY: `text` is NUL-terminated; `descriptor` receives a
            // LocalAlloc buffer freed below.
            let made = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(made, 0, "{sddl}");
            let (mut present, mut acl, mut defaulted) = (0, std::ptr::null_mut(), 0);
            // SAFETY: `descriptor` was made above; the out pointers are live.
            let read = unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
            };
            assert_ne!(read, 0);
            // SAFETY: a plain open of a process this test started.
            let handle = unsafe { OpenProcess(WRITE_DAC, 0, pid) };
            assert!(!handle.is_null(), "{}", std::io::Error::last_os_error());
            // SAFETY: the handle is open with WRITE_DAC and `acl` lives in
            // `descriptor` until it is freed below.
            let status = unsafe {
                SetSecurityInfo(
                    handle,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    acl,
                    std::ptr::null(),
                )
            };
            // SAFETY: both were allocated above and are not used again.
            unsafe {
                CloseHandle(handle);
                LocalFree(descriptor);
            }
            assert_eq!(status, 0, "{sddl}");
        }

        /// Turns off this process's debug privilege, which an administrator's
        /// token holds and which opens every process whatever its access
        /// list says, until dropped.
        struct NoDebug(bool);

        impl NoDebug {
            fn new() -> Self {
                Self(debug_privilege(false))
            }
        }

        impl Drop for NoDebug {
            fn drop(&mut self) {
                if self.0 {
                    debug_privilege(true);
                }
            }
        }

        /// Sets the debug privilege on or off; whether the token holds it.
        fn debug_privilege(on: bool) -> bool {
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LUID};
            use windows_sys::Win32::Security::{
                AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
                SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
            };
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

            let name: Vec<u16> = "SeDebugPrivilege".encode_utf16().chain([0]).collect();
            let mut luid = LUID {
                LowPart: 0,
                HighPart: 0,
            };
            // SAFETY: `name` is NUL-terminated and `luid` is writable.
            assert_ne!(
                unsafe { LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) },
                0
            );
            let mut token = std::ptr::null_mut();
            // SAFETY: the current-process pseudo handle is always valid.
            assert_ne!(
                unsafe {
                    OpenProcessToken(
                        GetCurrentProcess(),
                        TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                        &mut token,
                    )
                },
                0
            );
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: if on { SE_PRIVILEGE_ENABLED } else { 0 },
                }],
            };
            // SAFETY: the token is open for adjusting; one entry is passed.
            let adjusted = unsafe {
                AdjustTokenPrivileges(
                    token,
                    0,
                    &privileges,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            // SAFETY: read right after the call it describes.
            let held = adjusted != 0 && unsafe { GetLastError() } == 0;
            // SAFETY: opened above and not used again.
            unsafe { CloseHandle(token) };
            held
        }

        /// Owner and everyone denied: the access list keeps even the owner
        /// out, as another account's process is to this one.
        const FOREIGN: &str = "D:P(D;;0x1fffff;;;OW)";
        /// No entry: closed, while the owner keeps its right to the list.
        const CLOSED: &str = "D:P";

        #[test]
        fn the_walk_ends_outside_only_at_a_process_that_keeps_this_account_out() {
            let _no_debug = NoDebug::new();
            let state = tempfile::tempdir().unwrap();
            let home = tempfile::tempdir().unwrap();
            let walk = |pid: u32| in_holder(pid as i32, home.path(), state.path());

            // Under a parent that keeps this account out: the boundary an SSH
            // login's service process is.
            let foreign_parent = Tree::new(&ping());
            set_access(foreign_parent.shell(), FOREIGN);
            assert_eq!(walk(foreign_parent.caller), Ok(Origin::Outside));

            // Under a parent this account owns that only hides itself.
            let closed_parent = Tree::new(&ping());
            set_access(closed_parent.shell(), CLOSED);
            assert_eq!(walk(closed_parent.caller), Err(UNAVAILABLE));

            // A caller that keeps this account out: an administrator's SSH
            // login seen from the desktop session. Outside as far as the
            // walk reads, and still found under the pane program above it.
            let hidden = Tree::new(&ping());
            set_access(hidden.caller, FOREIGN);
            assert_eq!(walk(hidden.caller), Ok(Origin::Hidden));
            let program = hide_holder_protocol::record::Identity {
                pid: hidden.shell(),
                started: process::start_time(hidden.shell()).unwrap(),
            };
            assert_eq!(
                crate::access::runs_under(hidden.caller, &program),
                Ok(true),
                "a hidden caller is found under the pane program that started it"
            );

            // A caller this account owns that only hides itself.
            let closed = Tree::new(&ping());
            set_access(closed.caller, CLOSED);
            assert_eq!(walk(closed.caller), Err(UNAVAILABLE));

            // A program run from a holder build is a pane whatever is above.
            let holders = hide_kit::layout::holders_root(home.path()).join("v1");
            std::fs::create_dir_all(&holders).unwrap();
            let copy = holders.join("PING.EXE");
            std::fs::copy(ping(), &copy).unwrap();
            let in_pane = Tree::new(&copy);
            set_access(in_pane.shell(), FOREIGN);
            assert_eq!(walk(in_pane.caller), Ok(Origin::Holder));
        }
    }
}
