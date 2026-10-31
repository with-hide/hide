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
//! - Windows: out of the starter's own job object
//!   (`CREATE_BREAKAWAY_FROM_JOB`), which is refused when that job does not
//!   allow it or the child is found in it still, rather than started into
//!   the job the starter's owner ends it with; with no console; and with
//!   only the handles it is lent, named on a handle list, so no other handle
//!   of the starter's reaches it whatever other threads start meanwhile. A
//!   job above the starter's that keeps the child is allowed, since the
//!   starter's owner does not end it, and is said in [`Resident::outer_job`]
//!   for the caller to record; the child reads that job's limits with
//!   [`own_job`].
//!
//! The lent handles reach the child under names it reads back with
//! [`take_lent`]: descriptors 3, 4, ... on Unix, and on Windows the values of
//! inheritable copies made for the start, both listed in [`LENT_KEY`].

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

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

/// What a resident child runs: its program, its arguments, the changes to
/// this process's environment it starts with, and its working folder. It
/// has nothing else a [`std::process::Command`] carries (standard streams, a
/// cleared environment, hooks), since a resident start decides those itself
/// and the same way on every system.
#[derive(Clone, Debug)]
pub struct ResidentCommand {
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, Option<OsString>)>,
    current_dir: Option<PathBuf>,
}

impl ResidentCommand {
    /// Runs `program`, named by an absolute path.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            envs: Vec::new(),
            current_dir: None,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Sets `key` to `value` in the child's environment.
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.envs
            .push((key.as_ref().to_owned(), Some(value.as_ref().to_owned())));
        self
    }

    /// Leaves `key` out of the child's environment.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.envs.push((key.as_ref().to_owned(), None));
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.current_dir = Some(dir.as_ref().to_owned());
        self
    }
}

/// A resident child that runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resident {
    pub pid: u32,
    /// Windows: the child left the starter's own job, but a job above it
    /// holds the child still and ends it when that job is closed. Always
    /// false on Unix.
    pub outer_job: bool,
}

/// The limits of a Windows job object, as far as they decide whether a
/// process in it outlives it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobLimits {
    /// The job's limit flags, as Windows answers them.
    pub flags: u32,
}

impl JobLimits {
    const BREAKAWAY_OK: u32 = 0x0000_0800;
    const SILENT_BREAKAWAY_OK: u32 = 0x0000_1000;
    const KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    /// Every process in the job ends when its last handle closes.
    pub fn kill_on_close(self) -> bool {
        self.flags & Self::KILL_ON_JOB_CLOSE != 0
    }

    /// A process in the job may start a child outside it.
    pub fn breakaway_ok(self) -> bool {
        self.flags & Self::BREAKAWAY_OK != 0
    }

    /// Every child a process in the job starts is outside it.
    pub fn silent_breakaway_ok(self) -> bool {
        self.flags & Self::SILENT_BREAKAWAY_OK != 0
    }
}

/// One word with no spaces, so it fits a `key=value` field.
impl std::fmt::Display for JobLimits {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "kill_on_close={},breakaway_ok={},silent_breakaway_ok={},flags={:#010x}",
            self.kill_on_close(),
            self.breakaway_ok(),
            self.silent_breakaway_ok(),
            self.flags
        )
    }
}

/// The limits of the job this process runs in, the innermost one when jobs
/// nest; `None` outside every job, and always on Unix, which has none.
pub fn own_job() -> io::Result<Option<JobLimits>> {
    sys::own_job()
}

/// Starts `command` as a resident child lent `lent`, and answers it once the
/// child runs its program. Its standard streams are nothing (`/dev/null` on
/// Unix, no handle on Windows), its environment is this process's with the
/// command's changes, and its program must be an absolute path. An error
/// means no child runs.
///
/// On Windows a start the starter's own job keeps is refused with
/// `PermissionDenied` and that job's limits: the child would end with it.
pub fn start_resident(command: &ResidentCommand, lent: &[Lent<'_>]) -> io::Result<Resident> {
    if lent.len() > MAX_LENT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a resident child is lent at most {MAX_LENT} handles"),
        ));
    }
    if !Path::new(&command.program).is_absolute() {
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

    use super::{LENT_KEY, Lent, MAX_LENT, Resident, ResidentCommand};

    /// The first descriptor a lent handle is given in the child.
    const FIRST_LENT: RawFd = 3;

    /// The highest signal number reset to its default; a number the system
    /// does not have is refused by `sigaction` and passed over.
    const LAST_SIGNAL: libc::c_int = 64;

    /// How often, and how far apart, a start is tried while its program is
    /// busy (`ETXTBSY`): a program this process has just written (a holder
    /// copied into its version folder) stays busy while a child another
    /// thread forked meanwhile still holds the writer's descriptor, which
    /// close-on-exec closes only once that child runs its own program.
    const BUSY_TRIES: u32 = 20;
    const BUSY_PAUSE: std::time::Duration = std::time::Duration::from_millis(10);

    pub(super) fn own_job() -> io::Result<Option<super::JobLimits>> {
        Ok(None)
    }

    pub(super) fn start(resident: &ResidentCommand, lent: &[Lent<'_>]) -> io::Result<Resident> {
        let mut tries = 1;
        loop {
            match start_once(resident, lent) {
                // A failed start leaves nothing running, so it is tried
                // again whole.
                Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && tries < BUSY_TRIES => {
                    tries += 1;
                    super::super::pause(BUSY_PAUSE);
                }
                started => return started,
            }
        }
    }

    fn start_once(resident: &ResidentCommand, lent: &[Lent<'_>]) -> io::Result<Resident> {
        let count = lent.len();
        let mut sources = [-1 as RawFd; MAX_LENT];
        for (slot, handle) in sources.iter_mut().zip(lent) {
            *slot = handle.as_raw_fd();
        }
        let mut command = Command::new(&resident.program);
        command.args(&resident.args);
        for (key, value) in &resident.envs {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        if let Some(dir) = &resident.current_dir {
            command.current_dir(dir);
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
        Ok(Resident {
            pid: u32::from_ne_bytes(bytes),
            outer_job: false,
        })
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

    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_ACCESS_DENIED, ERROR_MORE_DATA,
        HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
    };
    use windows_sys::Win32::System::JobObjects::{
        IsProcessInJob, JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    };

    use super::{JobLimits, LENT_KEY, Lent, Resident, ResidentCommand};

    /// The most processes of the starter's own job read to find the child
    /// among them; a job with more is refused rather than read in part.
    const MAX_JOB_PROCESSES: usize = 65_536;

    /// A handle that closes when dropped.
    struct Closing(HANDLE);

    impl Drop for Closing {
        fn drop(&mut self) {
            // SAFETY: the handle is open and owned by this value.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub(super) fn start(command: &ResidentCommand, lent: &[Lent<'_>]) -> io::Result<Resident> {
        // The starter's own job, read before the start so a refusal can say
        // what it is.
        let own = own_job()?;
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
        let mut command = command.clone();
        command.env(
            LENT_KEY,
            super::names(copies.iter().map(|copy| copy.0 as usize as u64)),
        );
        let mut line = command_line(&command)?;
        let environment = environment_block(&command)?;
        let directory = command
            .current_dir
            .as_ref()
            .map(|dir| wide(dir.as_os_str()));
        let program = wide(&command.program);

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
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32)
                && let Some(own) = own
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "this process's job does not let a process leave it ({own}), \
                         so a resident child would end with that job"
                    ),
                ));
            }
            return Err(error);
        }
        let process = Closing(info.hProcess);
        let thread = Closing(info.hThread);
        // Where the child is decides before it runs: in the starter's own
        // job it ends with the starter's owner, so it is refused; in a job
        // above that one only, it runs and the caller is told.
        let placed = placement(process.0, info.dwProcessId, own);
        let outer_job = match placed {
            Ok(outer_job) => outer_job,
            Err(refusal) => {
                // SAFETY: the process is suspended and its handle open.
                unsafe { TerminateProcess(process.0, 1) };
                return Err(refusal);
            }
        };
        // SAFETY: the thread handle is open; `u32::MAX` is the failure.
        if unsafe { ResumeThread(thread.0) } == u32::MAX {
            let error = io::Error::last_os_error();
            // SAFETY: as above.
            unsafe { TerminateProcess(process.0, 1) };
            return Err(error);
        }
        drop(attributes);
        drop(copies);
        Ok(Resident {
            pid: info.dwProcessId,
            outer_job,
        })
    }

    /// Whether the started child is in a job above the starter's own one;
    /// an error when it is in the starter's own job, or that cannot be read.
    fn placement(process: HANDLE, pid: u32, own: Option<JobLimits>) -> io::Result<bool> {
        let mut in_job = 0;
        // SAFETY: the process handle is open; `in_job` is valid for the write.
        if unsafe { IsProcessInJob(process, std::ptr::null_mut(), &mut in_job) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if in_job == 0 {
            return Ok(false);
        }
        if let Some(own) = own
            && own_job_processes()?.contains(&(pid as usize))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the resident child stayed in this process's job ({own}), \
                     which would end it"
                ),
            ));
        }
        Ok(true)
    }

    pub(super) fn own_job() -> io::Result<Option<JobLimits>> {
        let mut in_job = 0;
        // SAFETY: this process's pseudo handle; `in_job` is valid for the write.
        if unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if in_job == 0 {
            return Ok(None);
        }
        // SAFETY: an all-zero limit structure is a valid value.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a null job is this process's innermost job; the structure
        // is valid for its size.
        let asked = unsafe {
            QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if asked == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(JobLimits {
            flags: limits.BasicLimitInformation.LimitFlags,
        }))
    }

    /// The pids in this process's innermost job and the jobs below it.
    fn own_job_processes() -> io::Result<Vec<usize>> {
        let head = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
            / std::mem::size_of::<usize>();
        let mut capacity = 64;
        loop {
            let mut buffer = vec![0usize; head + capacity];
            let bytes = std::mem::size_of_val(buffer.as_slice()) as u32;
            // SAFETY: a null job is this process's innermost job; the buffer
            // is aligned for the list and valid for `bytes`.
            let asked = unsafe {
                QueryInformationJobObject(
                    std::ptr::null_mut(),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr().cast(),
                    bytes,
                    std::ptr::null_mut(),
                )
            };
            let short = match (asked == 0).then(io::Error::last_os_error) {
                None => false,
                Some(error) if error.raw_os_error() == Some(ERROR_MORE_DATA as i32) => true,
                Some(error) => return Err(error),
            };
            // SAFETY: the buffer starts with the list's header.
            let list = unsafe { &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let assigned = list.NumberOfAssignedProcesses as usize;
            let listed = list.NumberOfProcessIdsInList as usize;
            if !short && listed >= assigned {
                return Ok(buffer[head..head + listed].to_vec());
            }
            if assigned > MAX_JOB_PROCESSES {
                return Err(io::Error::other(format!(
                    "this process's job holds {assigned} processes, more than \
                     {MAX_JOB_PROCESSES} are read"
                )));
            }
            capacity = assigned + 16;
        }
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
    fn command_line(command: &ResidentCommand) -> io::Result<Vec<u16>> {
        let mut line: Vec<u16> = Vec::new();
        for (index, argument) in std::iter::once(&command.program)
            .chain(&command.args)
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
    fn environment_block(command: &ResidentCommand) -> io::Result<Vec<u16>> {
        let mut variables: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        for (name, value) in &command.envs {
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
