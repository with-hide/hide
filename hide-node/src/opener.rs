//! Opening a file with this machine's own handler (open-in-macOS, PRD
//! core-host-node D-21): one owned, bounded path for launching the host's
//! file association handler, and the private supervisor that ends an
//! explicit CLI helper with the daemon that started it.
//!
//! It runs on the node whose machine holds the file; hided's node role
//! calls it, and its `--open-helper` mode is [`run_opener_helper`].

use std::collections::VecDeque;
use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(unix)]
use hide_platform::process;
use hide_platform::process::OwnedChild;
use tokio::sync::{Semaphore, watch};

const MAX_IN_FLIGHT_OPENERS: usize = 4;
const MAX_OPENS_PER_MINUTE: usize = 12;
const OPENER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct OpenHandler {
    configured: Option<PathBuf>,
    /// The binary the Unix supervisor of an explicit helper runs as.
    supervisor_exe: PathBuf,
    slots: Arc<Semaphore>,
    recent: Arc<Mutex<VecDeque<Instant>>>,
    /// True once the role that owns the handler ended, and never false
    /// again; a closed channel is an ended role too.
    shutdown: watch::Receiver<bool>,
}

impl OpenHandler {
    pub fn new(
        configured: Option<PathBuf>,
        shutdown: watch::Receiver<bool>,
        supervisor_exe: PathBuf,
    ) -> Self {
        Self {
            configured,
            supervisor_exe,
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT_OPENERS)),
            recent: Arc::new(Mutex::new(VecDeque::new())),
            shutdown,
        }
    }

    pub fn in_flight(&self) -> usize {
        MAX_IN_FLIGHT_OPENERS - self.slots.available_permits()
    }

    /// A successful call means the handler accepted the file, not that the
    /// eventual application opened it. An explicit CLI helper is ended within
    /// ten seconds or on daemon stop; the OS default is handed off at spawn.
    pub fn launch(&self, path: &Path) -> Result<(), &'static str> {
        let path = &program_spelling(path);
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "over_budget")?;
        {
            let now = Instant::now();
            let mut recent = self.recent.lock().map_err(|_| "over_budget")?;
            while recent
                .front()
                .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
            {
                recent.pop_front();
            }
            if recent.len() >= MAX_OPENS_PER_MINUTE {
                return Err("over_budget");
            }
            recent.push_back(now);
        }

        #[cfg(windows)]
        if self.configured.is_none() {
            // ShellExecuteW passes a pathname directly to the association API.
            // `cmd /C start` would interpret metacharacters in that pathname.
            let result = shell_open(path);
            drop(permit);
            return result;
        }

        #[cfg(unix)]
        if self.configured.is_none() {
            // Starting the OS association utility is the handoff. It may
            // become or wait for the chosen application, so it is not an
            // owned CLI helper and must not enter the ten-second kill path.
            let result =
                handoff_default_opener(platform_opener(), path).map_err(|_| "spawn_failed");
            drop(permit);
            return result;
        }

        let mut child = {
            let program = self.configured.as_deref().expect("configured opener");
            spawn_opener(&self.supervisor_exe, program.as_os_str(), path)
                .map_err(|_| "spawn_failed")?
        };
        let mut shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(OPENER_TIMEOUT);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    _ = shutdown.wait_for(|ended| *ended) => break,
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        match child.try_wait() {
                            Ok(true) => break,
                            Ok(false) => {},
                            Err(_) => break,
                        }
                    },
                }
            }
            child.stop();
            drop(permit);
        });
        Ok(())
    }
}

/// The path as a program on this system is handed one: on Windows without
/// the `\\?\` prefix a canonical path carries, which `cmd.exe` and many
/// programs cannot read, and with `\` between names; elsewhere the path as
/// it is. It goes through the path model's wire spelling and back, and a
/// path the wire cannot spell (not UTF-8) is handed over unchanged.
fn program_spelling(path: &Path) -> PathBuf {
    hide_platform::path::to_wire(path)
        .and_then(|wire| hide_platform::path::from_wire(&wire))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(target_os = "macos")]
fn platform_opener() -> &'static std::ffi::OsStr {
    std::ffi::OsStr::new("open")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_opener() -> &'static std::ffi::OsStr {
    std::ffi::OsStr::new("xdg-open")
}

/// Hands `target`, a file or an address such as the shell's own URL, to
/// this system's handler for it and waits only for the handoff: macOS's
/// `open`, the desktop's `xdg-open`, Windows's `ShellExecuteW`, which takes
/// the text as it is, where `cmd /C start` would read the `&` an address
/// carries as a second command.
pub fn hand_to_system(target: &OsStr) -> io::Result<()> {
    #[cfg(windows)]
    {
        shell_execute(target).map_err(|_| io::Error::other("ShellExecuteW refused the address"))
    }
    #[cfg(unix)]
    {
        let status = Command::new(platform_opener())
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "{} exited with {status}",
                platform_opener().to_string_lossy()
            )))
        }
    }
}

#[cfg(windows)]
fn shell_open(path: &Path) -> Result<(), &'static str> {
    shell_execute(path.as_os_str())
}

#[cfg(windows)]
fn shell_execute(target: &OsStr) -> Result<(), &'static str> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }
    let verb = "open\0".encode_utf16().collect::<Vec<_>>();
    let file = target
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // A ShellExecuteW return above 32 means the association accepted it.
    let result = unsafe {
        ShellExecuteW(
            0,
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        Err("spawn_failed")
    } else {
        Ok(())
    }
}

/// A short-lived CLI opener this daemon answers for: stopping it, dropping it
/// and this daemon's death without Drop each end the helper and everything it
/// started. On Windows the job object [`OwnedChild`] puts the helper in does
/// that by itself, because the system ends the job when its last handle
/// closes. No Unix system ends a whole process group when its owner dies, so
/// there a private supervisor holds the helper's group and watches an owner
/// channel whose EOF reaches it however hided died.
pub struct OwnedOpener {
    #[cfg(unix)]
    supervisor: OwnedChild,
    #[cfg(unix)]
    owner: Option<UnixStream>,
    #[cfg(windows)]
    helper: OwnedChild,
}

#[cfg(windows)]
impl OwnedOpener {
    pub fn try_wait(&mut self) -> io::Result<bool> {
        Ok(self.helper.try_wait()?.is_some())
    }

    /// Ends what the helper left in its job too, as the Unix supervisor ends
    /// the helper's group when the helper returns. Safe to call more than once.
    /// A failed kill does not wait on a helper that may still run; dropping
    /// the job's last handle still ends it.
    pub fn stop(&mut self) {
        if self.helper.kill_tree().is_ok() {
            let _ = self.helper.wait();
        }
    }
}

#[cfg(unix)]
impl OwnedOpener {
    pub fn supervisor_pid(&self) -> u32 {
        self.supervisor.id()
    }

    pub fn try_wait(&mut self) -> io::Result<bool> {
        let Some(owner) = self.owner.as_mut() else {
            return Ok(true);
        };
        let mut byte = [0];
        match owner.read(&mut byte) {
            Ok(0) => Ok(true),
            Ok(_) => Err(io::Error::other("unexpected opener supervisor message")),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(error) => Err(error),
        }
    }

    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    pub fn stop(&mut self) {
        // End the tree ourselves even if the supervisor crashed before its
        // watcher ran. Ending it more than once is safe, so explicit stop
        // followed by Drop is too.
        self.owner.take();
        let _ = self.supervisor.kill_tree();
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            match self.supervisor.try_wait() {
                Ok(Some(_)) => return,
                Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        let _ = self.supervisor.kill_tree();
        let _ = self.supervisor.wait();
    }
}

impl Drop for OwnedOpener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Hands a default association utility to the OS. Tokio's process driver
/// attempts to reap a short-lived utility after its handle is dropped; one
/// that becomes the registered application may live for that app's lifetime.
#[cfg(unix)]
pub fn handoff_default_opener(program: &OsStr, path: &Path) -> io::Result<()> {
    let mut command = tokio::process::Command::new(program);
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _app = command.spawn()?;
    Ok(())
}

/// Starts an explicit CLI override as an [`OwnedOpener`]. On Unix that is the
/// same hided binary (`supervisor_exe`) in its internal supervisor mode, which
/// watches this owner's socket; Windows starts the helper itself and never
/// runs `supervisor_exe`.
#[cfg(windows)]
pub fn spawn_opener(
    _supervisor_exe: &Path,
    program: &OsStr,
    path: &Path,
) -> io::Result<OwnedOpener> {
    let mut command = Command::new(program);
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(OwnedOpener {
        helper: OwnedChild::spawn(&mut command)?,
    })
}

#[cfg(unix)]
pub fn spawn_opener(
    supervisor_exe: &Path,
    program: &OsStr,
    path: &Path,
) -> io::Result<OwnedOpener> {
    let (owner, supervisor_socket) = UnixStream::pair()?;
    let inherited_fd = supervisor_socket.as_raw_fd();
    let mut command = Command::new(supervisor_exe);
    command
        .arg("--open-helper")
        .arg(inherited_fd.to_string())
        .arg(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // UnixStream::pair uses close-on-exec. Only the supervisor receives its
    // end; the parent end stays close-on-exec and the launched app receives no
    // liveness descriptor.
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(inherited_fd, libc::F_SETFD, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The supervisor leads its own tree, which is the CLI helper's too.
    let supervisor = OwnedChild::spawn(&mut command)?;
    drop(supervisor_socket);
    let mut opener = OwnedOpener {
        supervisor,
        owner: Some(owner),
    };
    let owner = opener.owner.as_mut().expect("owner channel");
    owner.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut accepted = [0];
    if owner.read_exact(&mut accepted).is_err() || accepted != [1] {
        opener.stop();
        return Err(io::Error::other(
            "opener supervisor could not start the CLI helper",
        ));
    }
    owner.set_read_timeout(None)?;
    // EOF now observes supervisor completion without reaping its PID. That
    // keeps its process-group ID reserved until stop signals the group.
    owner.set_nonblocking(true)?;
    Ok(opener)
}

/// Invoked only by hided's private `--open-helper` mode. An owner death closes
/// the socket even after SIGKILL; the watcher then kills the still-running CLI
/// process group. Default-app handoff does not enter this helper.
#[cfg(unix)]
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub fn run_opener_helper(args: &[OsString]) -> Result<(), String> {
    if args.len() != 4 || args[0] != "--open-helper" {
        return Err("invalid opener helper arguments".into());
    }
    let fd: i32 = args[1]
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|fd| *fd >= 3)
        .ok_or("invalid opener helper descriptor")?;
    let mut owner = unsafe { UnixStream::from_raw_fd(fd) };
    // The actual CLI must never inherit the liveness channel.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error().to_string());
    }
    let mut command = Command::new(&args[2]);
    command
        .arg(&args[3])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = owner.write_all(&[0]);
            return Err(error.to_string());
        }
    };
    #[cfg(debug_assertions)]
    if let Some(milliseconds) = std::env::var("HIDE_OPEN_HELPER_TEST_PAUSE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= 3_000)
    {
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
    let group = std::process::id();
    let mut watcher_owner = match owner.try_clone() {
        Ok(socket) => socket,
        Err(error) => {
            let _ = process::kill_tree(group);
            let _ = child.wait();
            return Err(error.to_string());
        }
    };
    let watcher = std::thread::Builder::new()
        .name("hide-open-owner".into())
        .spawn(move || {
            let mut byte = [0];
            // Any read or EOF means the daemon stopped. The CLI cannot inherit
            // this close-on-exec descriptor.
            let _ = watcher_owner.read(&mut byte);
            let _ = process::kill_tree(group);
        });
    if let Err(error) = watcher {
        let _ = process::kill_tree(group);
        let _ = child.wait();
        return Err(error.to_string());
    }
    // Acceptance follows watcher creation. The parent's process-group fallback
    // covers the scheduling gap before the new thread gets CPU time.
    if owner.write_all(&[1]).is_err() {
        let _ = process::kill_tree(group);
        let _ = child.wait();
        return Err("opener owner disappeared before acceptance".into());
    }
    let result = child.wait().map_err(|error| error.to_string());
    // An explicit override is a CLI helper, not a registered default app.
    // It may have forked and returned, so close its process group on exit too.
    let _ = process::kill_tree(group);
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_handler_is_shell_free() {
        #[cfg(target_os = "macos")]
        assert_eq!(platform_opener(), "open");
        #[cfg(all(unix, not(target_os = "macos")))]
        assert_eq!(platform_opener(), "xdg-open");
    }

    #[test]
    fn a_burst_is_bounded() {
        let (_role, ended) = watch::channel(false);
        let handler = OpenHandler::new(None, ended, PathBuf::new());
        let mut recent = handler.recent.lock().unwrap();
        for _ in 0..MAX_OPENS_PER_MINUTE {
            recent.push_back(Instant::now());
        }
        drop(recent);
        assert_eq!(handler.launch(Path::new("/ignored")), Err("over_budget"));
    }
}
