//! How many files this process may hold open at once.
//!
//! A Unix process starts with the soft `RLIMIT_NOFILE` of whoever started it:
//! 256 for an app launchd starts on macOS, 1024 on most Linux systems, both
//! kept low for old C programs that `select()` on descriptors below 1024. A
//! daemon that holds a connection, a watch and a file for each pane runs out
//! at that number while its hard limit is far higher. [`raise_open_file_limit`]
//! lifts the soft limit to the hard one, as Go's runtime does for every
//! program since Go 1.19 (`syscall/rlimit.go`), and caps it on macOS at
//! `kern.maxfilesperproc`, which `setrlimit` refuses to pass there.
//!
//! As Go gives a program it starts the original limit back, a resident child
//! ([`super::start_resident`]) starts with the limit this process started
//! with: a resident is a pane's holder, and the programs a pane runs are the
//! operator's, which see the limit they saw before the raise.
//!
//! Windows has no per-process limit on the handles a program opens (the C
//! runtime's 512 counts its own `FILE` streams, which Rust does not use), so
//! there the raise changes nothing and answers `None`.

use std::io;

/// The soft open-file limit before and after [`raise_open_file_limit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenFileLimit {
    pub before: u64,
    pub after: u64,
}

/// Raises this process's soft open-file limit as far as the system lets it:
/// the hard limit, and on macOS no further than `kern.maxfilesperproc`.
/// Answers the limit before and after, or `None` on Windows, which has no
/// such limit. Calling it again changes nothing and answers the same `after`.
pub fn raise_open_file_limit() -> io::Result<Option<OpenFileLimit>> {
    #[cfg(unix)]
    return sys::raise().map(Some);
    #[cfg(windows)]
    Ok(None)
}

/// Starts `command`'s program with the open-file limit this process started
/// with, as a resident child starts: for a child that is no resident but
/// starts the operator's programs itself, such as a guest's node, which
/// raises its own limit and gives its holders back the one it started with.
/// Nothing changes when this process raised nothing, or on Windows.
pub fn keep_starting_open_file_limit(command: &mut std::process::Command) {
    #[cfg(unix)]
    if let Some(limit) = starting_limit() {
        use std::os::unix::process::CommandExt;
        // SAFETY: after the fork the closure makes one system call,
        // `setrlimit`, on a structure it owns; it allocates nothing and takes
        // no lock.
        unsafe {
            command.pre_exec(move || {
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    let _ = command;
}

/// The limit this process started with, when [`raise_open_file_limit`] has
/// changed it, for a resident child to start with.
#[cfg(unix)]
pub(super) fn starting_limit() -> Option<libc::rlimit> {
    sys::STARTING.get().copied()
}

#[cfg(unix)]
mod sys {
    use std::io;
    use std::sync::OnceLock;

    use super::OpenFileLimit;

    /// The limit before the first raise that changed it; set once, before
    /// the change, so a raise asked twice keeps the first one's original.
    pub(super) static STARTING: OnceLock<libc::rlimit> = OnceLock::new();

    pub(super) fn raise() -> io::Result<OpenFileLimit> {
        let mut limit = current()?;
        let before = limit.rlim_cur;
        let ceiling = ceiling(limit.rlim_max)?;
        if before < ceiling {
            STARTING.get_or_init(|| limit);
            limit.rlim_cur = ceiling;
            // SAFETY: `setrlimit` reads the structure we pass and nothing else.
            if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(OpenFileLimit {
            before,
            after: current()?.rlim_cur,
        })
    }

    fn current() -> io::Result<libc::rlimit> {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `getrlimit` writes into the structure we own.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(limit)
    }

    /// The highest soft limit the system takes. A macOS process's hard limit
    /// is usually `RLIM_INFINITY`, which `setrlimit` refuses as a soft limit;
    /// the kernel's own per-process maximum is what it takes.
    #[cfg(target_os = "macos")]
    fn ceiling(hard: libc::rlim_t) -> io::Result<libc::rlim_t> {
        let mut most: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: the name is a NUL-terminated literal, and the kernel writes
        // at most `size` bytes into `most`.
        let answered = unsafe {
            libc::sysctlbyname(
                c"kern.maxfilesperproc".as_ptr(),
                (&raw mut most).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if answered == -1 {
            return Err(io::Error::last_os_error());
        }
        let most = libc::rlim_t::try_from(most).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("kern.maxfilesperproc is {most}"),
            )
        })?;
        Ok(hard.min(most))
    }

    /// Linux bounds the hard limit itself by `fs.nr_open`, so the hard limit
    /// is what `setrlimit` takes.
    #[cfg(not(target_os = "macos"))]
    fn ceiling(hard: libc::rlim_t) -> io::Result<libc::rlim_t> {
        Ok(hard)
    }
}
