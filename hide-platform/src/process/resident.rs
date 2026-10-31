//! A resident child: a process started to outlive the one that starts it,
//! such as a pane's holder, which keeps a terminal running while the node
//! that started it ends and starts again.
//!
//! [`super::detach`] is for a short-lived program that starts a daemon and
//! ends; it leaves the child in the starter's session (Unix) and job
//! (Windows), and on Windows it changes the starter's own handle table. A
//! resident child is started apart from its starter instead:
//!
//! - Unix: in a session of its own, so no terminal the starter controls can
//!   reach it, started through an intermediate process that exits at once, so
//!   the child is no child of the starter's and can never take a controlling
//!   terminal; every signal at its default and none blocked; standard
//!   streams on `/dev/null`; and no descriptor of the starter's but the ones
//!   it is lent.
//! - Windows: out of the starter's job object (`CREATE_BREAKAWAY_FROM_JOB`),
//!   which is refused when a job above the starter does not allow it, rather
//!   than started into a job that would end it; with no console; and with
//!   only the handles it is lent, named on a handle list, so no other handle
//!   of the starter's reaches it whatever other threads start meanwhile.
//!
//! The lent handles reach the child under names it reads back with
//! [`take_lent`]: descriptors 3, 4, ... on Unix, and on Windows the values of
//! inheritable copies made for the start, both listed in [`LENT_KEY`].

use std::io;
use std::process::Command;

/// The environment variable that names the lent handles in the child, as
/// decimal numbers separated by commas. [`take_lent`] reads it.
pub const LENT_KEY: &str = "HIDE_RESIDENT_LENT";

/// The most handles one start lends.
pub const MAX_LENT: usize = 4;

/// A handle a resident child is lent.
#[cfg(unix)]
pub type Lent<'a> = std::os::fd::BorrowedFd<'a>;
#[cfg(windows)]
pub type Lent<'a> = std::os::windows::io::BorrowedHandle<'a>;

/// A handle a resident child was lent, owned by the child.
#[cfg(unix)]
pub type Taken = std::os::fd::OwnedFd;
#[cfg(windows)]
pub type Taken = std::os::windows::io::OwnedHandle;

/// Starts `command` as a resident child lent `lent`, and answers its pid
/// once the child runs its program. The command's standard streams are
/// replaced by nothing (`/dev/null` on Unix, no handle on Windows), and its
/// program must be an absolute path. An error means no child runs.
///
/// On Windows a starter inside a job that does not allow a process to leave
/// it is refused with `PermissionDenied`: the child would end with that job.
pub fn start_resident(command: &mut Command, lent: &[Lent<'_>]) -> io::Result<u32> {
    if lent.len() > MAX_LENT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a resident child is lent at most {MAX_LENT} handles"),
        ));
    }
    if !std::path::Path::new(command.get_program()).is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a resident child's program is named by an absolute path",
        ));
    }
    sys::start(command, lent)
}

/// The handles this process was lent as a resident child, in the order they
/// were lent, owned from now on. A process started otherwise has none. Call
/// it once: a second call would own the same handles twice, so it answers
/// `AlreadyExists`.
pub fn take_lent() -> io::Result<Vec<Taken>> {
    static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TAKEN.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the lent handles were taken already",
        ));
    }
    let Some(names) = std::env::var_os(LENT_KEY) else {
        return Ok(Vec::new());
    };
    let names = names.to_str().ok_or_else(|| invalid_names(&names))?;
    let mut taken = Vec::new();
    for name in names.split(',').filter(|name| !name.is_empty()) {
        let value: u64 = name.parse().map_err(|_| invalid_names(names.as_ref()))?;
        taken.push(sys::own(value)?);
        if taken.len() > MAX_LENT {
            return Err(invalid_names(names.as_ref()));
        }
    }
    Ok(taken)
}

fn invalid_names(names: &std::ffi::OsStr) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{LENT_KEY} names no handles: {names:?}"),
    )
}

fn names(values: impl IntoIterator<Item = u64>) -> String {
    values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(unix)]
mod sys {
    use std::io::{self, Read};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    use super::{LENT_KEY, Lent, MAX_LENT};

    /// The first descriptor a lent handle is given in the child.
    const FIRST_LENT: RawFd = 3;

    /// The highest signal number reset to its default; a number the system
    /// does not have is refused by `sigaction` and passed over.
    const LAST_SIGNAL: libc::c_int = 64;

    pub(super) fn start(command: &mut Command, lent: &[Lent<'_>]) -> io::Result<u32> {
        let count = lent.len();
        let mut sources = [-1 as RawFd; MAX_LENT];
        for (slot, handle) in sources.iter_mut().zip(lent) {
            *slot = handle.as_raw_fd();
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env(
                LENT_KEY,
                super::names((0..count).map(|index| (FIRST_LENT as u64) + index as u64)),
            );
        // The intermediate process writes the resident's pid here and exits.
        let (mut report_read, report_write) = std::io::pipe()?;
        let report = report_write.as_raw_fd();
        // `sysconf` is not on POSIX's list of calls safe between fork and
        // exec, so the bound is read here and the child only uses the number.
        let limit = super::super::sys::fallback_limit();
        // SAFETY: after each fork the closure calls only functions POSIX lists
        // as async-signal-safe (`setsid`, `fork`, `write`, `_exit`,
        // `sigaction`, `sigprocmask`, `fcntl`, `dup2`, `close`) and the
        // bare system-call entry; it allocates nothing and takes no lock.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                // The session's leader leaves: its child can never take a
                // controlling terminal, and is a child of init (or of a
                // subreaper), never of the starter.
                match libc::fork() {
                    -1 => return Err(io::Error::last_os_error()),
                    0 => {}
                    resident => {
                        let bytes = (resident as u32).to_ne_bytes();
                        let written = libc::write(report, bytes.as_ptr().cast(), bytes.len());
                        libc::_exit(if written == bytes.len() as isize {
                            0
                        } else {
                            1
                        });
                    }
                }
                let mut default: libc::sigaction = std::mem::zeroed();
                default.sa_sigaction = libc::SIG_DFL;
                for signal in 1..=LAST_SIGNAL {
                    if signal != libc::SIGKILL && signal != libc::SIGSTOP {
                        libc::sigaction(signal, &default, std::ptr::null_mut());
                    }
                }
                let mut none: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut none);
                libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
                super::super::sys::mark_inherited_descriptors(limit)?;
                // Each lent descriptor is first copied above the names it
                // will take, so placing one never overwrites another.
                let mut copies = [-1 as RawFd; MAX_LENT];
                for (copy, source) in copies.iter_mut().zip(&sources[..count]) {
                    *copy = libc::fcntl(
                        *source,
                        libc::F_DUPFD_CLOEXEC,
                        FIRST_LENT + MAX_LENT as RawFd,
                    );
                    if *copy == -1 {
                        return Err(io::Error::last_os_error());
                    }
                }
                for (name, copy) in (FIRST_LENT..).zip(&copies[..count]) {
                    // `dup2` leaves the new name without close-on-exec.
                    if libc::dup2(*copy, name) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    libc::close(*copy);
                }
                Ok(())
            })
        };
        // Spawn returns once the resident runs its program: the standard
        // library's exec report reaches it only when both the intermediate
        // and the resident have let go of it.
        let mut intermediate = command.spawn()?;
        drop(report_write);
        let status = intermediate.wait()?;
        let mut bytes = [0u8; 4];
        report_read.read_exact(&mut bytes).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("the resident's starter ended ({status}) without naming it: {error}"),
            )
        })?;
        Ok(u32::from_ne_bytes(bytes))
    }

    pub(super) fn own(value: u64) -> io::Result<OwnedFd> {
        let descriptor = RawFd::try_from(value)
            .ok()
            .filter(|descriptor| *descriptor >= FIRST_LENT)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a lent descriptor"))?;
        // SAFETY: `fcntl` on a number this process may not hold fails with
        // EBADF and does nothing else.
        if unsafe { libc::fcntl(descriptor, libc::F_GETFD) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the descriptor is open and was lent to this process alone;
        // `take_lent` hands it out once.
        let owned = unsafe { OwnedFd::from_raw_fd(descriptor) };
        // Nothing this process starts inherits it.
        // SAFETY: the flag is the only thing set on an open descriptor.
        unsafe { libc::fcntl(descriptor, libc::F_SETFD, libc::FD_CLOEXEC) };
        Ok(owned)
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Command;

    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ACCESS_DENIED, HANDLE,
        HANDLE_FLAG_INHERIT, SetHandleInformation,
    };
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    };

    use super::{LENT_KEY, Lent};

    /// A handle that closes when dropped.
    struct Closing(HANDLE);

    impl Drop for Closing {
        fn drop(&mut self) {
            // SAFETY: the handle is open and owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn start(command: &mut Command, lent: &[Lent<'_>]) -> io::Result<u32> {
        // Inheritable copies, made for this start and closed after it, so
        // the starter's own handles never change.
        let mut copies = Vec::with_capacity(lent.len());
        for handle in lent {
            let mut copy: HANDLE = std::ptr::null_mut();
            // SAFETY: both process handles are this process's pseudo handle;
            // `copy` is valid for the write.
            let copied = unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    handle.as_raw_handle(),
                    GetCurrentProcess(),
                    &mut copy,
                    0,
                    1,
                    DUPLICATE_SAME_ACCESS,
                )
            };
            if copied == 0 {
                return Err(io::Error::last_os_error());
            }
            copies.push(Closing(copy));
        }
        command.env(
            LENT_KEY,
            super::names(copies.iter().map(|copy| copy.0 as usize as u64)),
        );
        let mut line = command_line(command)?;
        let environment = environment_block(command)?;
        let directory = command.get_current_dir().map(|dir| wide(dir.as_os_str()));
        let program = wide(command.get_program());

        let list: Vec<HANDLE> = copies.iter().map(|copy| copy.0).collect();
        let mut attributes = Attributes::new(&list)?;
        // SAFETY: an all-zero startup structure is a valid value.
        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        // No standard handles at all: null handles, which a program reads as
        // no stream.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.lpAttributeList = attributes.as_ptr();
        // SAFETY: an all-zero process information is a valid value.
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let flags = CREATE_BREAKAWAY_FROM_JOB
            | DETACHED_PROCESS
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_SUSPENDED
            | CREATE_UNICODE_ENVIRONMENT
            | EXTENDED_STARTUPINFO_PRESENT;
        // SAFETY: every string is terminated and lives through the call; the
        // command line is a mutable buffer the call may change; the startup
        // structure and its attribute list are valid.
        let created = unsafe {
            CreateProcessW(
                program.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                i32::from(!list.is_empty()),
                flags,
                environment.as_ptr().cast(),
                directory
                    .as_ref()
                    .map_or(std::ptr::null(), |directory| directory.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        };
        if created == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) && in_a_job() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "this process runs in a job that does not let a process leave it, \
                     so a resident child would end with that job",
                ));
            }
            return Err(error);
        }
        let process = Closing(info.hProcess);
        let thread = Closing(info.hThread);
        // A job that lets a process leave it silently, or a job the system
        // put the child in, would still end it: refuse before it runs.
        let mut in_job = 0;
        // SAFETY: the process handle is open; `in_job` is valid for the write.
        let asked = unsafe { IsProcessInJob(process.0, std::ptr::null_mut(), &mut in_job) };
        let refusal = if asked == 0 {
            Some(io::Error::last_os_error())
        } else if in_job != 0 {
            Some(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the resident child was started inside a job, which would end it",
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            // SAFETY: the process is suspended and its handle open.
            unsafe { TerminateProcess(process.0, 1) };
            return Err(refusal);
        }
        // SAFETY: the thread handle is open; `u32::MAX` is the failure.
        if unsafe { ResumeThread(thread.0) } == u32::MAX {
            let error = io::Error::last_os_error();
            // SAFETY: as above.
            unsafe { TerminateProcess(process.0, 1) };
            return Err(error);
        }
        drop(attributes);
        drop(copies);
        Ok(info.dwProcessId)
    }

    pub(super) fn own(value: u64) -> io::Result<OwnedHandle> {
        let raw = usize::try_from(value)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not a lent handle"))?
            as HANDLE;
        let mut flags = 0u32;
        // SAFETY: a value that names no handle fails and changes nothing.
        if unsafe { windows_sys::Win32::Foundation::GetHandleInformation(raw, &mut flags) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Nothing this process starts inherits it.
        // SAFETY: the handle is open; the flag is the only thing cleared.
        unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, 0) };
        // SAFETY: the handle was lent to this process alone; `take_lent`
        // hands it out once.
        Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
    }

    fn in_a_job() -> bool {
        let mut in_job = 0;
        // SAFETY: this process's pseudo handle; `in_job` is valid for the write.
        let asked =
            unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) };
        asked != 0 && in_job != 0
    }

    /// A process attribute list naming the handles a child inherits.
    struct Attributes {
        buffer: Vec<u8>,
        _list: Box<[HANDLE]>,
    }

    impl Attributes {
        fn new(handles: &[HANDLE]) -> io::Result<Self> {
            let mut size = 0usize;
            // SAFETY: the first call asks for the size with a null list.
            unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
            let mut buffer = vec![0u8; size];
            let list: LPPROC_THREAD_ATTRIBUTE_LIST = buffer.as_mut_ptr().cast();
            // SAFETY: `buffer` holds `size` bytes, as the first call asked.
            if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let handles: Box<[HANDLE]> = handles.into();
            let mut attributes = Self {
                buffer,
                _list: handles,
            };
            // An empty list is refused by the system; no handle is
            // inherited anyway when the start inherits none.
            if !attributes._list.is_empty() {
                // SAFETY: the list is initialized; the handle array lives as
                // long as the list does.
                let updated = unsafe {
                    UpdateProcThreadAttribute(
                        attributes.as_ptr(),
                        0,
                        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                        attributes._list.as_ptr().cast(),
                        std::mem::size_of_val(&*attributes._list),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                };
                if updated == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(attributes)
        }

        fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.buffer.as_mut_ptr().cast()
        }
    }

    impl Drop for Attributes {
        fn drop(&mut self) {
            // SAFETY: the list was initialized in `new`.
            unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    /// The program and its arguments as one command line, quoted the way the
    /// C runtime splits it back.
    fn command_line(command: &Command) -> io::Result<Vec<u16>> {
        let mut line: Vec<u16> = Vec::new();
        for (index, argument) in std::iter::once(command.get_program())
            .chain(command.get_args())
            .enumerate()
        {
            if index > 0 {
                line.push(u16::from(b' '));
            }
            let units: Vec<u16> = argument.encode_wide().collect();
            if units.contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an argument holds a nul character",
                ));
            }
            quote(&units, &mut line);
        }
        line.push(0);
        Ok(line)
    }

    fn quote(argument: &[u16], line: &mut Vec<u16>) {
        let plain = !argument.is_empty()
            && !argument
                .iter()
                .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x0a | 0x0b | 0x22));
        if plain {
            line.extend_from_slice(argument);
            return;
        }
        line.push(u16::from(b'"'));
        let mut backslashes = 0usize;
        for unit in argument {
            if *unit == u16::from(b'\\') {
                backslashes += 1;
                continue;
            }
            if *unit == u16::from(b'"') {
                // Each backslash before a quote doubles, and the quote is escaped.
                line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            } else {
                line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            }
            backslashes = 0;
            line.push(*unit);
        }
        // Backslashes before the closing quote double.
        line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
        line.push(u16::from(b'"'));
    }

    /// This process's environment with the command's changes, sorted by
    /// name without regard to case as the system keeps it.
    fn environment_block(command: &Command) -> io::Result<Vec<u16>> {
        let mut variables: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        for (name, value) in command.get_envs() {
            let upper = name.to_string_lossy().to_uppercase();
            variables.retain(|(kept, _)| kept.to_string_lossy().to_uppercase() != upper);
            if let Some(value) = value {
                variables.push((name.to_owned(), value.to_owned()));
            }
        }
        variables.sort_by_key(|(name, _)| name.to_string_lossy().to_uppercase());
        let mut block = Vec::new();
        for (name, value) in variables {
            let name: Vec<u16> = name.encode_wide().collect();
            let value: Vec<u16> = value.encode_wide().collect();
            if name.is_empty()
                || name.contains(&0)
                || value.contains(&0)
                || name[1..].contains(&u16::from(b'='))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an environment variable cannot be passed to a child",
                ));
            }
            block.extend(name);
            block.push(u16::from(b'='));
            block.extend(value);
            block.push(0);
        }
        if block.is_empty() {
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }
}
