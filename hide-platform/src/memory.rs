//! This process's own heap: giving the system back what it freed.
//!
//! An allocator keeps the pages of freed blocks for the next allocation, so
//! a burst of work leaves a resident process holding memory it no longer
//! uses: on macOS a pane holder held about 5 MB more after a 300 MB output
//! flood than before it, 87% of its malloc zone free (PRD
//! engine-bundle-2-pane-holder D-17, B15). [`return_freed`] asks the
//! allocator to hand those pages back, once, when the caller's work has gone
//! quiet; it costs a walk of the heap, so it is never for a per-input path.

use std::io;

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
