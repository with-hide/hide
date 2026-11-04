use super::memory::{self, Layout};
use super::*;
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::mem::{offset_of, size_of};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, FILETIME, NO_ERROR, RtlNtStatusToDosError, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::SystemInformation::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, IsWow64Process2, OpenProcess, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_VM_READ, WaitForSingleObject,
};

// Limits apply to the entire sample, not once per row. At most two complete
// IPv4/IPv6 table reads, each with three bounded allocation attempts, and six
// memory reads per owner (two <=65534-byte strings, four <=80-byte prefixes).
// Checking elapsed work between native calls prevents unbounded owner/retry
// work; no thread or process is left behind to enforce it.
const MAX_TABLE_BYTES: usize = 1024 * 1024;
const MAX_ROWS: usize = 4096;
const MAX_OWNERS: usize = 512;
const TABLE_ATTEMPTS: usize = 3;
const READ_BUDGET: Duration = Duration::from_secs(10);

use crate::windows_process::{NtQuery, nt_query};

fn check_budget(start: Instant) -> io::Result<()> {
    if start.elapsed() >= READ_BUDGET {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "TCP listener observation exceeded its work budget",
        ))
    } else {
        Ok(())
    }
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

pub(super) fn read() -> io::Result<Vec<ListeningSocket>> {
    let start = Instant::now();
    let before = owners(start)?;
    if before.is_empty() {
        return Ok(Vec::new());
    }
    let query = nt_query()?;
    let mut observations = BTreeMap::new();
    let mut cwd_bytes = 0usize;
    for &pid in before.keys() {
        check_budget(start)?;
        let observation = observe_owner(pid, query, start);
        if let Ok((_, cwd)) = &observation {
            cwd_bytes += cwd.as_str().len();
            if cwd_bytes > super::MAX_SAMPLE_CWD_BYTES {
                return Err(invalid("TCP listener cwd observation limit exceeded"));
            }
        }
        observations.insert(pid, observation);
    }
    // A failed cwd read is not evidence that a listener is absent. This one
    // bounded recheck can prove absence; otherwise its original failure wins.
    let after = owners(start)?;
    let answer = super::complete_sample(&before, &observations, after, |pid| {
        check_budget(start)?;
        // Open the pid again after the table recheck. Keeping and measuring
        // the first handle alone would miss a pid reused by a new process.
        let owner = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE)?;
        identity(&owner)
    })?;
    check_budget(start)?;
    Ok(answer)
}

/// The folder `pid` works in, read from its process parameters as a
/// listener owner's is ([`super::observe_cwd`]).
pub(super) fn observe(pid: u32) -> io::Result<ObservedWorkingDirectory> {
    observe_owner(pid, nt_query()?, Instant::now()).map(|(_, cwd)| cwd)
}

fn owners(start: Instant) -> io::Result<Owners> {
    let mut owners = Owners::new();
    let ipv4 = table(
        AF_INET as u32,
        size_of::<MIB_TCPROW_OWNER_PID>(),
        offset_of!(MIB_TCPTABLE_OWNER_PID, table),
        start,
    )?;
    let ipv6 = table(
        AF_INET6 as u32,
        size_of::<MIB_TCP6ROW_OWNER_PID>(),
        offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
        start,
    )?;
    if ipv4.len() / size_of::<MIB_TCPROW_OWNER_PID>()
        + ipv6.len() / size_of::<MIB_TCP6ROW_OWNER_PID>()
        > MAX_ROWS
    {
        return Err(invalid("TCP listener row limit exceeded"));
    }
    for bytes in ipv4.as_chunks::<{ size_of::<MIB_TCPROW_OWNER_PID>() }>().0 {
        // SDK record containing only u32 fields: every bit pattern is valid.
        let row = unsafe {
            bytes
                .as_ptr()
                .cast::<MIB_TCPROW_OWNER_PID>()
                .read_unaligned()
        };
        if row.dwState != 2 || row.dwOwningPid == 0 {
            return Err(invalid("invalid IPv4 listener owner row"));
        }
        let address = std::net::Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
        let port = port(row.dwLocalPort)?;
        owners
            .entry(row.dwOwningPid)
            .or_default()
            .insert(SocketAddr::from((address, port)));
    }
    for bytes in ipv6.as_chunks::<{ size_of::<MIB_TCP6ROW_OWNER_PID>() }>().0 {
        // SDK record containing only u32 fields and byte arrays.
        let row = unsafe {
            bytes
                .as_ptr()
                .cast::<MIB_TCP6ROW_OWNER_PID>()
                .read_unaligned()
        };
        if row.dwState != 2 || row.dwOwningPid == 0 {
            return Err(invalid("invalid IPv6 listener owner row"));
        }
        let address = std::net::SocketAddrV6::new(
            std::net::Ipv6Addr::from(row.ucLocalAddr),
            port(row.dwLocalPort)?,
            0,
            u32::from_be(row.dwLocalScopeId),
        );
        owners
            .entry(row.dwOwningPid)
            .or_default()
            .insert(address.into());
    }
    if owners.len() > MAX_OWNERS {
        return Err(invalid("TCP listener owner limit exceeded"));
    }
    Ok(owners)
}

fn port(value: u32) -> io::Result<u16> {
    if value > u16::MAX as u32 || value == 0 {
        return Err(invalid("invalid TCP listener port"));
    }
    Ok(u16::from_be(value as u16))
}

fn table(family: u32, row_size: usize, row_offset: usize, start: Instant) -> io::Result<Vec<u8>> {
    check_budget(start)?;
    let mut bytes = 0u32;
    // Null/zero is the documented size query. No owner information is inferred
    // from an error or a zero-sized answer.
    let status = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut bytes,
            0,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != ERROR_INSUFFICIENT_BUFFER && status != NO_ERROR {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    for _ in 0..TABLE_ATTEMPTS {
        check_budget(start)?;
        let capacity = bytes as usize;
        if capacity < row_offset || capacity > MAX_TABLE_BYTES {
            return Err(invalid("invalid or oversized TCP owner table"));
        }
        // DWORD alignment is sufficient for both documented table records.
        let mut buffer = vec![0u32; capacity.div_ceil(size_of::<u32>())];
        let status = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut bytes,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if status != NO_ERROR {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let count = buffer[0] as usize;
        let end = count
            .checked_mul(row_size)
            .and_then(|size| row_offset.checked_add(size))
            .ok_or_else(|| invalid("TCP owner table length overflow"))?;
        if count > MAX_ROWS || bytes as usize > capacity || end > bytes as usize {
            return Err(invalid("invalid or oversized TCP owner table rows"));
        }
        // The returned count/length is validated against the actual allocation.
        let rows = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>().add(row_offset),
                end - row_offset,
            )
        };
        return Ok(rows.to_vec());
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "TCP owner table kept changing size",
    ))
}

fn open(pid: u32, access: u32) -> io::Result<OwnedHandle> {
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // This new, non-inheritable handle belongs to this call and closes on
    // every exit path. No process is started or altered by observation.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn identity(owner: &OwnedHandle) -> io::Result<u64> {
    let handle = owner.as_raw_handle();
    match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_TIMEOUT => {}
        WAIT_OBJECT_0 => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "listener owner exited",
            ));
        }
        _ => return Err(io::Error::last_os_error()),
    }
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
}

#[repr(C)]
#[derive(Default)]
struct BasicInformation {
    exit_status: i32,
    peb: usize,
    affinity: usize,
    priority: i32,
    pid: usize,
    parent: usize,
}

fn query_information<T>(
    query: NtQuery,
    owner: &OwnedHandle,
    class: u32,
    answer: &mut T,
) -> io::Result<()> {
    let mut length = 0;
    // Only fixed-size integer records owned by this module reach this call.
    let status = unsafe {
        query(
            owner.as_raw_handle(),
            class,
            (answer as *mut T).cast(),
            size_of::<T>() as u32,
            &mut length,
        )
    };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    if length as usize != size_of::<T>() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unrecognized process query record length",
        ));
    }
    Ok(())
}

fn observe_owner(
    pid: u32,
    query: NtQuery,
    start: Instant,
) -> io::Result<(u64, ObservedWorkingDirectory)> {
    check_budget(start)?;
    if size_of::<usize>() != 8 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "listener cwd observation requires a 64-bit observer",
        ));
    }
    // NT basic information needs QUERY_INFORMATION; cwd memory needs VM_READ;
    // SYNCHRONIZE proves positive exit. No ALL_ACCESS or debug privilege.
    let owner = open(
        pid,
        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE,
    )?;
    let birth = identity(&owner)?;
    let mut machine = 0;
    let mut native = 0;
    if unsafe { IsWow64Process2(owner.as_raw_handle(), &mut machine, &mut native) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if !matches!(native, IMAGE_FILE_MACHINE_AMD64 | IMAGE_FILE_MACHINE_ARM64) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unrecognized native process architecture",
        ));
    }
    let mut basic = BasicInformation::default();
    query_information(query, &owner, 0, &mut basic)?;
    if basic.pid != pid as usize {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "process query identity changed",
        ));
    }
    let (layout, peb) = match machine {
        0 | IMAGE_FILE_MACHINE_AMD64 | IMAGE_FILE_MACHINE_ARM64 => (Layout::Bits64, basic.peb),
        IMAGE_FILE_MACHINE_I386 => {
            let mut wow64_peb = 0usize;
            query_information(query, &owner, 26, &mut wow64_peb)?;
            (Layout::Bits32, wow64_peb)
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "unrecognized process architecture",
            ));
        }
    };
    let cwd = memory::observe(layout, peb, |address, length| {
        check_budget(start)?;
        let mut buffer = vec![0u8; length];
        let mut read = 0;
        if unsafe {
            ReadProcessMemory(
                owner.as_raw_handle(),
                address as *const c_void,
                buffer.as_mut_ptr().cast(),
                length,
                &mut read,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if read != length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short process memory read",
            ));
        }
        Ok(buffer)
    })?;
    check_budget(start)?;
    let cwd = cwd.resolve()?;
    if identity(&owner)? != birth {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "listener owner identity changed during cwd read",
        ));
    }
    check_budget(start)?;
    Ok((birth, cwd))
}
