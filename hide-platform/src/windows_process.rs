// Partly derived from Herdr v0.9.5 (https://github.com/herdrdev/herdr,
// commit 2ecf19a), src/platform/windows.rs: read_process_command_line.
// Copyright 2025 herdr contributors. Licensed under the Apache License,
// Version 2.0; see NOTICE at the repository root.
// Changed by Hide: bounded allocation, checked descriptor pointers, typed
// errors, and a native query resolver shared with listener observation.

//! Native process queries shared by process arguments and listener owners.
//! The command-line query follows Herdr v0.9.5's Windows implementation,
//! with bounded allocation and validation of the returned string pointer.

use std::ffi::c_void;
use std::io;
use std::mem::size_of;

use windows_sys::Win32::Foundation::{HANDLE, LocalFree, RtlNtStatusToDosError};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

pub(crate) type NtQuery =
    unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

pub(crate) fn nt_query() -> io::Result<NtQuery> {
    let name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
    // The system has loaded ntdll before any Rust code runs.
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    let function = if module.is_null() {
        None
    } else {
        unsafe { GetProcAddress(module, c"NtQueryInformationProcess".as_ptr().cast()) }
    };
    let Some(function) = function else {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "NtQueryInformationProcess is unavailable",
        ));
    };
    // The documented system ABI, dynamically resolved as Microsoft requires
    // for this changeable native query. Callers check every return length.
    Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, NtQuery>(function) })
}

const COMMAND_LINE_INFORMATION: u32 = 60;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xc0000004_u32 as i32;
const STATUS_BUFFER_OVERFLOW: i32 = 0x80000005_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xc0000023_u32 as i32;
// Windows command lines contain at most 32767 UTF-16 units. Allow space
// for the native descriptor too, without trusting a process-sized request.
const MAX_COMMAND_BYTES: usize = 64 * 1024 + size_of::<UnicodeString>();

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

fn needs_buffer(status: i32) -> bool {
    matches!(
        status,
        STATUS_INFO_LENGTH_MISMATCH | STATUS_BUFFER_OVERFLOW | STATUS_BUFFER_TOO_SMALL
    )
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Read through a held process handle, so pid reuse cannot change the
/// subject halfway through the query. Only query access is required;
/// reading the target's address space is unnecessary.
pub(crate) fn argv(process: HANDLE) -> io::Result<Vec<String>> {
    let query = nt_query()?;
    let mut required = 0;
    // A null buffer asks for the required allocation only.
    let status = unsafe {
        query(
            process,
            COMMAND_LINE_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut required,
        )
    };
    if !needs_buffer(status) {
        return Err(if status < 0 {
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
        } else {
            invalid("process command-line query returned no buffer size")
        });
    }
    // A process can rewrite its command line between the size and data
    // queries. Retry that race twice, never an unbounded allocation loop.
    for _ in 0..3 {
        let size = required as usize;
        if !(size_of::<UnicodeString>()..=MAX_COMMAND_BYTES).contains(&size) {
            return Err(invalid("process command line exceeds its byte limit"));
        }
        let mut bytes = vec![0u8; size];
        let status = unsafe {
            query(
                process,
                COMMAND_LINE_INFORMATION,
                bytes.as_mut_ptr().cast(),
                size as u32,
                &mut required,
            )
        };
        if needs_buffer(status) {
            continue;
        }
        if status < 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ));
        }
        if required as usize > size || (required as usize) < size_of::<UnicodeString>() {
            return Err(invalid("invalid process command-line return length"));
        }
        bytes.truncate(required as usize);
        let line = command_line(&bytes)?;
        return split_command_line(&line);
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "process command line changed during every read",
    ))
}

fn command_line(bytes: &[u8]) -> io::Result<Vec<u16>> {
    // The caller checked the descriptor's size. Vec<u8> does not promise
    // its alignment, so read only the header unaligned and bounds-check its
    // pointer against this allocation before accessing the string bytes.
    let descriptor = unsafe { bytes.as_ptr().cast::<UnicodeString>().read_unaligned() };
    let length = usize::from(descriptor.length);
    let offset = (descriptor.buffer as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .ok_or_else(|| invalid("process command-line pointer precedes its buffer"))?;
    if length == 0
        || !length.is_multiple_of(2)
        || descriptor.length > descriptor.maximum_length
        || offset < size_of::<UnicodeString>()
        || offset.checked_add(length).is_none_or(|end| end > bytes.len())
    {
        return Err(invalid("invalid process command-line descriptor"));
    }
    let mut line: Vec<u16> = bytes[offset..offset + length]
        .chunks_exact(2)
        .map(|unit| u16::from_ne_bytes([unit[0], unit[1]]))
        .collect();
    if line.contains(&0) {
        return Err(invalid("process command line contains an embedded NUL"));
    }
    line.push(0);
    Ok(line)
}

fn split_command_line(line: &[u16]) -> io::Result<Vec<String>> {
    let mut count = 0;
    // `line` is NUL-terminated. Windows owns the quote/backslash grammar.
    let arguments = unsafe { CommandLineToArgvW(line.as_ptr(), &mut count) };
    if arguments.is_null() {
        return Err(io::Error::last_os_error());
    }
    struct Arguments(*mut *mut u16);
    impl Drop for Arguments {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0.cast()) };
        }
    }
    let owned = Arguments(arguments);
    if count <= 0 || count as usize > line.len() {
        return Err(invalid("invalid process argument count"));
    }
    let mut result = Vec::with_capacity(count as usize);
    for argument in unsafe { std::slice::from_raw_parts(owned.0, count as usize) } {
        // These pointers and their NUL terminators come from the system
        // parser, not the process. Each string fits within the input line.
        let mut length = 0;
        while length < line.len() && unsafe { *argument.add(length) } != 0 {
            length += 1;
        }
        if length == line.len() {
            return Err(invalid("unterminated process argument"));
        }
        result.push(
            String::from_utf16(unsafe { std::slice::from_raw_parts(*argument, length) })
                .map_err(|_| invalid("process argument is not valid UTF-16"))?,
        );
    }
    Ok(result)
}

// npm launchers can run a generic cli.js. Its nearest node_modules path
// identifies the package without reading a package file or knowing any agent.
pub(crate) fn package_name(script: &str) -> Option<String> {
    let parts: Vec<_> = script
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    // The nearest node_modules owns this script, including nested npm and
    // pnpm layouts. Dot segments would make that ownership ambiguous.
    if parts.iter().any(|part| matches!(*part, "." | "..")) {
        return None;
    }
    let at = parts
        .iter()
        .rposition(|part| part.eq_ignore_ascii_case("node_modules"))?;
    let package = parts.get(at + 1)?;
    let (package, first_script) = if package.starts_with('@') {
        (format!("{package}/{}", parts.get(at + 2)?), at + 3)
    } else {
        ((*package).to_owned(), at + 2)
    };
    if package.len() > 214 || first_script >= parts.len() {
        return None;
    }
    Some(package)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_packages_with_the_same_script_keep_distinct_identities() {
        assert_eq!(
            package_name(r"C:\npm\node_modules\@earendil-works\pi-coding-agent\dist\cli.js"),
            Some("@earendil-works/pi-coding-agent".into())
        );
        assert_eq!(
            package_name(r"C:\npm\node_modules\@oh-my-pi\pi-coding-agent\dist\cli.js"),
            Some("@oh-my-pi/pi-coding-agent".into())
        );
        assert_eq!(package_name(r"C:\work\cli.js"), None);
        assert_eq!(
            package_name(r"C:\node_modules\@oh-my-pi\pi-coding-agent\..\other\cli.js"),
            None
        );
    }

    #[test]
    fn a_nested_dependency_is_not_its_outer_package() {
        assert_eq!(
            package_name(r"C:\npm\node_modules\@oh-my-pi\pi-coding-agent\node_modules\helper\cli.js"),
            Some("helper".into())
        );
    }
}
