//! Memory: this process's own heap, giving the system back what it freed,
//! and the machine's memory pressure as its operating system judges it.
//!
//! An allocator keeps the pages of freed blocks for the next allocation, so
//! a burst of work leaves a resident process holding memory it no longer
//! uses: on macOS a pane holder held about 5 MB more after a 300 MB output
//! flood than before it, 87% of its malloc zone free (PRD
//! engine-bundle-2-pane-holder D-17, B15). [`return_freed`] asks the
//! allocator to hand those pages back, once, when the caller's work has gone
//! quiet; it costs a walk of the heap, so it is never for a per-input path.

use std::io;

/// How hard the machine is pressed for memory, as its operating system
/// judges it ([`pressure`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}

/// The machine's memory pressure now, read once with no child process and
/// no wait. Each system answers in its own terms, and none is Hide's guess
/// from free memory:
///
/// - macOS: the kernel's level, the one Activity Monitor draws
///   (`kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4
///   critical).
/// - Linux: pressure stall information, `/proc/pressure/memory`'s ten-second
///   averages: critical when every running task stalled on memory for 10%
///   of them (`full avg10`), warning when some task did (`some avg10`).
///   A machine that is not short reads 0.00 on both; 10% is where waiting
///   on memory starts to be felt, and a ten-second average fits a reader
///   that asks once a minute and holds no trigger open.
/// - Windows: the system's own low and high memory resource notifications:
///   low is critical, high is normal, and between the two is warning. The
///   system sets low at about 32 MB available per 4 GB (64 MB at most) and
///   high at three times that, so Windows says warning much later than
///   macOS does.
///
/// A system without the signal (a Linux kernel built or booted without
/// pressure stall information, another system) answers an error, never
/// `Normal`.
pub fn pressure() -> io::Result<Pressure> {
    pressure::read()
}

/// Asks the allocator to return the pages its free blocks hold to the
/// system: `malloc_zone_pressure_relief` on macOS, glibc's `malloc_trim` on
/// Linux, `HeapCompact` on the process heap on Windows. What it returned is
/// not counted. A C library without such a call (musl) answers
/// `ErrorKind::Unsupported`.
pub fn return_freed() -> io::Result<()> {
    imp::return_freed()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::io;

    unsafe extern "C" {
        /// libmalloc: frees the pages of free blocks in `zone`, or every
        /// zone when it is null, until `goal` bytes (0: all it can), and
        /// answers how many bytes it freed.
        fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
    }

    pub(super) fn return_freed() -> io::Result<()> {
        // SAFETY: a null zone names every zone; the call takes no lock the
        // caller holds and touches no allocation of the caller's.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        Ok(())
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    use std::io;

    pub(super) fn return_freed() -> io::Result<()> {
        // SAFETY: plain call; 0 keeps no padding at the top of the heap. It
        // answers 1 when it returned memory and 0 when there was none.
        unsafe { libc::malloc_trim(0) };
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use windows_sys::Win32::System::Memory::{GetProcessHeap, HeapCompact};

    pub(super) fn return_freed() -> io::Result<()> {
        // SAFETY: plain calls on this process's own heap, which Rust's
        // allocator uses; `HeapCompact` coalesces its free blocks and
        // decommits the large ones. It answers the largest free block, and
        // 0 with an error only when it failed.
        unsafe {
            let heap = GetProcessHeap();
            if heap.is_null() {
                return Err(io::Error::last_os_error());
            }
            windows_sys::Win32::Foundation::SetLastError(0);
            if HeapCompact(heap, 0) == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "macos",
    all(target_os = "linux", target_env = "gnu"),
    windows
)))]
mod imp {
    use std::io;

    pub(super) fn return_freed() -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this C library has no call that returns freed heap pages",
        ))
    }
}

#[cfg(target_os = "macos")]
mod pressure {
    use std::ffi::c_void;
    use std::io;

    use super::Pressure;

    /// `DISPATCH_MEMORYPRESSURE_NORMAL`, `_WARN` and `_CRITICAL`, which the
    /// sysctl answers.
    pub(super) fn from_level(level: i32) -> io::Result<Pressure> {
        match level {
            1 => Ok(Pressure::Normal),
            2 => Ok(Pressure::Warning),
            4 => Ok(Pressure::Critical),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the kernel's memory pressure level {other} is none of 1, 2 or 4"),
            )),
        }
    }

    pub(super) fn read() -> io::Result<Pressure> {
        let mut level: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: the name is a NUL-terminated literal, `level` and `size`
        // outlive the call and `size` says how many bytes `level` holds; no
        // new value is written.
        let status = unsafe {
            libc::sysctlbyname(
                c"kern.memorystatus_vm_pressure_level".as_ptr(),
                (&raw mut level).cast::<c_void>(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return Err(io::Error::last_os_error());
        }
        if size != std::mem::size_of::<libc::c_int>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the kernel answered {size} bytes for its memory pressure level"),
            ));
        }
        from_level(level)
    }
}

#[cfg(any(target_os = "linux", test))]
mod psi {
    use std::io;

    use super::Pressure;

    /// The share of the last ten seconds, in percent, past which a stall
    /// counts (see [`super::pressure`]).
    const STALLED_PERCENT: f64 = 10.0;

    /// The level `/proc/pressure/memory` says: its `some` and `full` lines'
    /// `avg10`. A file without both is an error, not a calm machine.
    pub(super) fn level(text: &str) -> io::Result<Pressure> {
        let average = |kind: &str| -> io::Result<f64> {
            let line = text
                .lines()
                .find(|line| line.split_whitespace().next() == Some(kind))
                .ok_or_else(|| invalid(format!("no `{kind}` line")))?;
            let value = line
                .split_whitespace()
                .find_map(|field| field.strip_prefix("avg10="))
                .ok_or_else(|| invalid(format!("no avg10 on the `{kind}` line")))?;
            value
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or_else(|| invalid(format!("`{kind}` avg10 is {value:?}")))
        };
        let (some, full) = (average("some")?, average("full")?);
        Ok(if full >= STALLED_PERCENT {
            Pressure::Critical
        } else if some >= STALLED_PERCENT {
            Pressure::Warning
        } else {
            Pressure::Normal
        })
    }

    fn invalid(what: String) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("/proc/pressure/memory has {what}"),
        )
    }
}

#[cfg(target_os = "linux")]
mod pressure {
    use std::io;

    use super::Pressure;

    pub(super) fn read() -> io::Result<Pressure> {
        // A kernel built without pressure stall information has no file; one
        // booted with `psi=0` answers the read with EOPNOTSUPP. Either is the
        // caller's error.
        let text = std::fs::read_to_string("/proc/pressure/memory").map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("this kernel reports no memory pressure (/proc/pressure/memory: {error})"),
            )
        })?;
        super::psi::level(&text)
    }
}

#[cfg(windows)]
mod pressure {
    use std::io;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Memory::{
        CreateMemoryResourceNotification, HighMemoryResourceNotification,
        LowMemoryResourceNotification, MEMORY_RESOURCE_NOTIFICATION_TYPE,
        QueryMemoryResourceNotification,
    };

    use super::Pressure;

    /// One notification object, closed when the read ends.
    struct Notification(HANDLE);

    impl Drop for Notification {
        fn drop(&mut self) {
            // SAFETY: the handle came from `CreateMemoryResourceNotification`
            // and is closed once, here.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Whether the system's notification of this kind is signaled now.
    fn signaled(kind: MEMORY_RESOURCE_NOTIFICATION_TYPE) -> io::Result<bool> {
        // SAFETY: plain call; it answers a handle or null with the error set.
        let handle = unsafe { CreateMemoryResourceNotification(kind) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let notification = Notification(handle);
        let mut state = 0;
        // SAFETY: the handle is open for the call and `state` outlives it.
        if unsafe { QueryMemoryResourceNotification(notification.0, &mut state) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(state != 0)
    }

    pub(super) fn read() -> io::Result<Pressure> {
        if signaled(LowMemoryResourceNotification)? {
            return Ok(Pressure::Critical);
        }
        Ok(if signaled(HighMemoryResourceNotification)? {
            Pressure::Normal
        } else {
            Pressure::Warning
        })
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod pressure {
    use std::io;

    use super::Pressure;

    pub(super) fn read() -> io::Result<Pressure> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this system reports no memory pressure Hide reads",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Pressure;

    const CALM: &str = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

    #[test]
    fn linux_pressure_rises_with_the_share_of_time_tasks_stalled_on_memory() {
        let read = |some: &str, full: &str| {
            super::psi::level(&format!(
                "some avg10={some} avg60=1.00 avg300=0.50 total=12345\nfull avg10={full} avg60=0.20 avg300=0.10 total=678\n"
            ))
            .unwrap()
        };
        assert_eq!(super::psi::level(CALM).unwrap(), Pressure::Normal);
        assert_eq!(read("9.99", "0.00"), Pressure::Normal);
        assert_eq!(read("10.00", "0.00"), Pressure::Warning);
        assert_eq!(read("55.20", "9.99"), Pressure::Warning);
        assert_eq!(read("55.20", "10.00"), Pressure::Critical);
    }

    #[test]
    fn a_linux_pressure_file_without_both_averages_is_an_error_not_normal() {
        for text in [
            "",
            "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n",
            "some avg60=0.00\nfull avg10=0.00\n",
            "some avg10=nan\nfull avg10=0.00\n",
            "some avg10=-1\nfull avg10=0.00\n",
        ] {
            let error = super::psi::level(text).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{text:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_macos_level_the_kernel_does_not_define_is_an_error_not_normal() {
        assert_eq!(super::pressure::from_level(1).unwrap(), Pressure::Normal);
        assert_eq!(super::pressure::from_level(2).unwrap(), Pressure::Warning);
        assert_eq!(super::pressure::from_level(4).unwrap(), Pressure::Critical);
        for level in [0, 3, 8, -1] {
            assert!(super::pressure::from_level(level).is_err(), "{level}");
        }
    }
}
