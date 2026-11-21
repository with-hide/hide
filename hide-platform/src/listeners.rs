//! Native TCP listener observations, distinct from process identity proof.
//!
//! Windows exposes a socket's owning pid, but no kernel cwd query. Its cwd
//! observation comes from that owner's process parameters. It can attribute a
//! dev server or refuse cleanup; it must never authenticate a caller. The
//! trusted [`crate::process::cwd_of`] contract is unchanged.

use std::io;
use std::net::SocketAddr;
use std::path::Path;

/// A filesystem-resolved cwd observed in process memory, not a kernel
/// attestation or capability. Its spelling supports native checkout ancestry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedWorkingDirectory(String);

impl ObservedWorkingDirectory {
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(windows)]
    fn resolve(self) -> io::Result<Self> {
        if !self.as_path().is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "observed cwd is not an absolute native path",
            ));
        }
        let path = crate::fs::identity::canonical(self.as_path())?;
        // Cleanup compares against canonical checkout paths by ancestry.
        // canonical() retains verbatim spellings where shortening changes
        // meaning or crosses the classic length limit. A short ancestor and
        // such a child would not compare by prefix: refuse that sample.
        if matches!(
            path.components().next(),
            Some(std::path::Component::Prefix(prefix))
                if !matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::UNC(_, _))
        ) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "observed cwd cannot support canonical checkout ancestry",
            ));
        }
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "resolved cwd is not UTF-8")
        })?;
        if path.len() > MAX_SAMPLE_CWD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resolved cwd exceeds the sample text limit",
            ));
        }
        Ok(Self(path.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListeningSocket {
    pub pid: u32,
    pub address: SocketAddr,
    pub cwd: ObservedWorkingDirectory,
}

/// A native observation of accessible owners, or an explicit refusal.
///
/// A cwd observation denied permission excludes that owner from the sample;
/// protected owners are outside the current user's cleanup targets. Other cwd
/// errors, unknown layouts, changed owners and resource limits fail the entire
/// sample. Owners no longer listening at the final bounded table read are also
/// excluded. No subprocess, privilege elevation, process mutation or
/// executable-directory guess.
/// The synchronous read checks a ten-second work budget between bounded
/// native calls; it does not preempt a kernel call. It leaves no pending work
/// to cancel, and the caller observes its own cancellation after it returns.
/// On Unix the core retains its established `lsof` reader; this API explicitly
/// returns `Unsupported` there.
pub fn read() -> io::Result<Vec<ListeningSocket>> {
    #[cfg(windows)]
    {
        windows::read()
    }
    #[cfg(not(windows))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native TCP owner observation is implemented on Windows",
        ))
    }
}

/// The folder `pid` works in, observed from its process parameters as a
/// listener owner's is: Windows has no kernel query for another process's
/// folder. Like a listener's, it can refuse a cleanup and never
/// authenticates a caller ([`crate::process::observed_cwd_of`]).
#[cfg(windows)]
pub(crate) fn observe_cwd(pid: u32) -> io::Result<ObservedWorkingDirectory> {
    windows::observe(pid)
}

#[cfg(windows)]
mod windows;

#[cfg(any(windows, test))]
type Owners = std::collections::BTreeMap<u32, std::collections::BTreeSet<SocketAddr>>;

#[cfg(any(windows, test))]
const MAX_SAMPLE_CWD_BYTES: usize = 1024 * 1024;

#[cfg(any(windows, test))]
fn complete_sample(
    before: &Owners,
    observations: &std::collections::BTreeMap<u32, io::Result<(u64, ObservedWorkingDirectory)>>,
    after: Owners,
    mut identity: impl FnMut(u32) -> io::Result<u64>,
) -> io::Result<Vec<ListeningSocket>> {
    if after
        .iter()
        .any(|(pid, endpoints)| before.get(pid).is_none_or(|old| !endpoints.is_subset(old)))
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "TCP listener owners changed during observation",
        ));
    }
    let mut answer = Vec::new();
    let mut cwd_bytes = 0usize;
    for (pid, endpoints) in after {
        let observation = observations.get(&pid).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "listener owner has no observation",
            )
        })?;
        if matches!(observation, Err(error) if error.kind() == io::ErrorKind::PermissionDenied) {
            continue;
        }
        let (birth, cwd) = observation.as_ref().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("TCP listener owner {pid} cwd unavailable: {error}"),
            )
        })?;
        if identity(pid).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("TCP listener owner {pid} identity unavailable: {error}"),
            )
        })? != *birth
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("TCP listener owner {pid} identity changed"),
            ));
        }
        cwd_bytes = cwd
            .as_str()
            .len()
            .checked_mul(endpoints.len())
            .and_then(|bytes| cwd_bytes.checked_add(bytes))
            .filter(|&bytes| bytes <= MAX_SAMPLE_CWD_BYTES)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TCP listener cwd sample limit exceeded",
                )
            })?;
        for address in endpoints {
            answer.push(ListeningSocket {
                pid,
                address,
                cwd: cwd.clone(),
            });
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn permission_denied_cwd_owners_are_excluded_and_readable_owners_are_reported() {
        let before = Owners::from([
            (4, ["[::1]:8080".parse().unwrap()].into()),
            (41, ["127.0.0.1:5173".parse().unwrap()].into()),
        ]);
        let observations = BTreeMap::from([
            (
                4,
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "protected owner",
                )),
            ),
            (
                41,
                Ok((700, ObservedWorkingDirectory("C:\\checkout".into()))),
            ),
        ]);
        let sample = complete_sample(&before, &observations, before.clone(), |pid| {
            if pid == 41 {
                Ok(700)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "protected owner identity",
                ))
            }
        })
        .unwrap();
        assert_eq!(
            sample,
            vec![ListeningSocket {
                pid: 41,
                address: "127.0.0.1:5173".parse().unwrap(),
                cwd: ObservedWorkingDirectory("C:\\checkout".into()),
            }]
        );
    }

    #[test]
    fn other_cwd_errors_refuse_the_whole_sample_and_only_proven_absence_excludes_them() {
        let before = Owners::from([
            (41, ["127.0.0.1:5173".parse().unwrap()].into()),
            (42, ["[::1]:8080".parse().unwrap()].into()),
        ]);
        let observations = BTreeMap::from([
            (
                41,
                Ok((700, ObservedWorkingDirectory("C:\\checkout".into()))),
            ),
            (
                42,
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "unknown owner layout",
                )),
            ),
        ]);
        let error =
            complete_sample(&before, &observations, before.clone(), |_| Ok(700)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("42"));
        let mut after = before.clone();
        after.remove(&42);
        let sample = complete_sample(&before, &observations, after, |_| Ok(700)).unwrap();
        assert_eq!(sample.len(), 1);
        assert_eq!(
            sample[0].address,
            "127.0.0.1:5173".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn reused_owner_or_new_listener_makes_the_sample_inconsistent() {
        let before = Owners::from([(41, ["127.0.0.1:5173".parse().unwrap()].into())]);
        let observations = BTreeMap::from([(
            41,
            Ok((700, ObservedWorkingDirectory("C:\\checkout".into()))),
        )]);
        assert_eq!(
            complete_sample(&before, &observations, before.clone(), |_| Ok(701))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        let after = Owners::from([(41, ["127.0.0.1:8080".parse().unwrap()].into())]);
        assert_eq!(
            complete_sample(&before, &observations, after, |_| Ok(700))
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn listener_fanout_cannot_expand_cwd_text_beyond_the_sample_bound() {
        let endpoints = (5000..5032)
            .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
            .collect();
        let before = Owners::from([(41, endpoints)]);
        let observations = BTreeMap::from([(
            41,
            Ok((
                700,
                ObservedWorkingDirectory(format!("C:\\{}", "a".repeat(36000))),
            )),
        )]);
        assert_eq!(
            complete_sample(&before, &observations, before.clone(), |_| Ok(700))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

// Decode bounded records as bytes, never as local pointers or Rust values
// containing validity-sensitive fields. This also lets malformed foreign
// memory be tested on every platform.
#[cfg(any(windows, test))]
mod memory {
    use super::*;
    use std::mem::{offset_of, size_of};

    // The prefix layouts used by sysinfo's Windows reader and ntapi 0.4.1:
    // https://github.com/MSxDOS/ntapi/blob/3f8ac79fdfea53d8fbe1282237173cee8c4f198c/src/ntrtl.rs
    // https://github.com/MSxDOS/ntapi/blob/3f8ac79fdfea53d8fbe1282237173cee8c4f198c/src/ntpebteb.rs
    // Windows documents these structures as changeable. Architecture, record
    // lengths, normalization, pointers and stable rereads are prerequisites;
    // a layout we cannot validate is Unsupported, never a guessed cwd.
    #[repr(C)]
    struct UnicodeString<P> {
        length: u16,
        maximum_length: u16,
        buffer: P,
    }

    #[repr(C)]
    struct Parameters<P> {
        maximum_length: u32,
        length: u32,
        flags: u32,
        debug_flags: u32,
        console: P,
        console_flags: u32,
        stdin: P,
        stdout: P,
        stderr: P,
        cwd: UnicodeString<P>,
        cwd_handle: P,
    }

    #[repr(C)]
    struct Peb<P> {
        flags: [u8; 4],
        mutant: P,
        image: P,
        loader: P,
        parameters: P,
    }

    #[derive(Clone, Copy)]
    pub(super) enum Layout {
        Bits32,
        Bits64,
    }

    impl Layout {
        pub(super) fn pointer(self, bytes: &[u8], offset: usize) -> usize {
            match self {
                Self::Bits32 => {
                    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize
                }
                Self::Bits64 => {
                    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize
                }
            }
        }

        fn peb_size(self) -> usize {
            match self {
                Self::Bits32 => size_of::<Peb<u32>>(),
                Self::Bits64 => size_of::<Peb<u64>>(),
            }
        }

        fn parameters_offset(self) -> usize {
            match self {
                Self::Bits32 => offset_of!(Peb<u32>, parameters),
                Self::Bits64 => offset_of!(Peb<u64>, parameters),
            }
        }

        fn check_record(self, address: usize) -> io::Result<()> {
            let alignment = match self {
                Self::Bits32 => 4,
                Self::Bits64 => 8,
            };
            if !address.is_multiple_of(alignment) {
                return Err(unsupported("unaligned remote process record"));
            }
            Ok(())
        }

        fn parameters_size(self) -> usize {
            match self {
                Self::Bits32 => size_of::<Parameters<u32>>(),
                Self::Bits64 => size_of::<Parameters<u64>>(),
            }
        }

        fn cwd_offset(self) -> usize {
            match self {
                Self::Bits32 => offset_of!(Parameters<u32>, cwd),
                Self::Bits64 => offset_of!(Parameters<u64>, cwd),
            }
        }

        fn buffer_offset(self) -> usize {
            match self {
                Self::Bits32 => offset_of!(UnicodeString<u32>, buffer),
                Self::Bits64 => offset_of!(UnicodeString<u64>, buffer),
            }
        }

        pub(super) fn check_range(self, address: usize, length: usize) -> io::Result<()> {
            let end = address
                .checked_add(length)
                .ok_or_else(|| unsupported("remote pointer overflow"))?;
            if address < 0x10000
                || length == 0
                || matches!(self, Self::Bits32) && end > u32::MAX as usize
            {
                return Err(unsupported("invalid remote memory range"));
            }
            Ok(())
        }
    }

    fn unsupported(reason: &str) -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, reason)
    }

    #[derive(Eq, PartialEq)]
    struct Cwd {
        address: usize,
        length: usize,
        maximum_length: usize,
    }

    fn cwd(layout: Layout, record: &[u8]) -> io::Result<Cwd> {
        let maximum = u32::from_le_bytes(record[0..4].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
        let flags = u32::from_le_bytes(record[8..12].try_into().unwrap());
        if length < layout.parameters_size() || maximum < length || flags & 1 == 0 {
            return Err(unsupported(
                "unrecognized or unnormalized process parameters",
            ));
        }
        let offset = layout.cwd_offset();
        let cwd = Cwd {
            address: layout.pointer(record, offset + layout.buffer_offset()),
            length: u16::from_le_bytes(record[offset..offset + 2].try_into().unwrap()) as usize,
            maximum_length: u16::from_le_bytes(record[offset + 2..offset + 4].try_into().unwrap())
                as usize,
        };
        // UNICODE_STRING's lengths are bytes, with at most 65534 readable
        // bytes. Neither a remote length nor an address controls an unbounded
        // allocation. No environment, image name or command line is read.
        if cwd.length == 0
            || !cwd.length.is_multiple_of(2)
            || !cwd.maximum_length.is_multiple_of(2)
            || cwd.length > cwd.maximum_length
            || !cwd.address.is_multiple_of(2)
        {
            return Err(unsupported("invalid cwd string descriptor"));
        }
        layout.check_range(cwd.address, cwd.length)?;
        Ok(cwd)
    }

    pub(super) fn observe(
        layout: Layout,
        peb_address: usize,
        mut read: impl FnMut(usize, usize) -> io::Result<Vec<u8>>,
    ) -> io::Result<ObservedWorkingDirectory> {
        let mut record = |address, length| {
            layout.check_range(address, length)?;
            let bytes = read(address, length)?;
            if bytes.len() != length {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short process memory read",
                ));
            }
            Ok(bytes)
        };
        layout.check_record(peb_address)?;
        let peb = record(peb_address, layout.peb_size())?;
        let parameters = layout.pointer(&peb, layout.parameters_offset());
        layout.check_record(parameters)?;
        let descriptor = cwd(layout, &record(parameters, layout.parameters_size())?)?;
        let text = record(descriptor.address, descriptor.length)?;
        let after = cwd(layout, &record(parameters, layout.parameters_size())?)?;
        let after_peb = record(peb_address, layout.peb_size())?;
        if after != descriptor
            || layout.pointer(&after_peb, layout.parameters_offset()) != parameters
            || record(after.address, after.length)? != text
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "cwd changed during observation",
            ));
        }
        let units: Vec<_> = text
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .collect();
        if units.contains(&0) {
            return Err(unsupported("cwd contains an embedded nul"));
        }
        let text =
            String::from_utf16(&units).map_err(|_| unsupported("cwd is not valid UTF-16"))?;
        Ok(ObservedWorkingDirectory(text))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn source(layout: Layout) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
            let text: Vec<u8> = "C:\\fixture\\한글"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            let (mut peb, mut parameters, cwd_offset, pointer_offset, parameter_offset) =
                match layout {
                    Layout::Bits32 => (vec![0; 20], vec![0; 48], 36, 4, 16),
                    Layout::Bits64 => (vec![0; 40], vec![0; 80], 56, 8, 32),
                };
            let pointer_bytes = match layout {
                Layout::Bits32 => 4,
                Layout::Bits64 => 8,
            };
            peb[parameter_offset..parameter_offset + pointer_bytes]
                .copy_from_slice(&0x20000u64.to_le_bytes()[..pointer_bytes]);
            let size = parameters.len() as u32;
            parameters[0..4].copy_from_slice(&size.to_le_bytes());
            parameters[4..8].copy_from_slice(&size.to_le_bytes());
            parameters[8] = 1;
            parameters[cwd_offset..cwd_offset + 2]
                .copy_from_slice(&(text.len() as u16).to_le_bytes());
            parameters[cwd_offset + 2..cwd_offset + 4]
                .copy_from_slice(&(text.len() as u16).to_le_bytes());
            parameters[cwd_offset + pointer_offset..cwd_offset + pointer_offset + pointer_bytes]
                .copy_from_slice(&0x30000u64.to_le_bytes()[..pointer_bytes]);
            (peb, parameters, text)
        }

        fn sample(
            layout: Layout,
            source: (Vec<u8>, Vec<u8>, Vec<u8>),
        ) -> io::Result<ObservedWorkingDirectory> {
            observe(layout, 0x10000, |address, length| {
                let data = match address {
                    0x10000 => &source.0,
                    0x20000 => &source.1,
                    0x30000 => &source.2,
                    _ => panic!("unexpected remote address"),
                };
                assert_eq!(data.len(), length);
                Ok(data.clone())
            })
        }

        #[test]
        fn unicode_cwd_is_observed_from_native_and_wow64_records() {
            for layout in [Layout::Bits32, Layout::Bits64] {
                assert_eq!(
                    sample(layout, source(layout)).unwrap().as_str(),
                    "C:\\fixture\\한글"
                );
            }
        }

        #[test]
        fn invalid_and_changing_memory_is_unavailable_never_an_empty_cwd() {
            for layout in [Layout::Bits32, Layout::Bits64] {
                let mut unnormalized = source(layout);
                unnormalized.1[8] = 0;
                assert_eq!(
                    sample(layout, unnormalized).unwrap_err().kind(),
                    io::ErrorKind::Unsupported
                );
                let mut malformed = source(layout);
                malformed.2[..2].copy_from_slice(&0xd800u16.to_le_bytes());
                assert_eq!(
                    sample(layout, malformed).unwrap_err().kind(),
                    io::ErrorKind::Unsupported
                );
                let mut odd_length = source(layout);
                let offset = match layout {
                    Layout::Bits32 => 36,
                    Layout::Bits64 => 56,
                };
                odd_length.1[offset] |= 1;
                assert_eq!(
                    sample(layout, odd_length).unwrap_err().kind(),
                    io::ErrorKind::Unsupported
                );
                let mut nul = source(layout);
                nul.2[..2].copy_from_slice(&0u16.to_le_bytes());
                assert_eq!(
                    sample(layout, nul).unwrap_err().kind(),
                    io::ErrorKind::Unsupported
                );
                let data = source(layout);
                let mut text_reads = 0;
                let changed = observe(layout, 0x10000, |address, _| {
                    Ok(match address {
                        0x10000 => data.0.clone(),
                        0x20000 => data.1.clone(),
                        0x30000 => {
                            text_reads += 1;
                            let mut text = data.2.clone();
                            if text_reads == 2 {
                                text[0] = b'D';
                            }
                            text
                        }
                        _ => unreachable!(),
                    })
                });
                assert_eq!(changed.unwrap_err().kind(), io::ErrorKind::WouldBlock);
            }
        }

        #[test]
        fn remote_range_overflow_and_short_reads_are_refused_before_dereference() {
            assert!(Layout::Bits64.check_range(usize::MAX - 1, 4).is_err());
            assert!(
                Layout::Bits32
                    .check_range(u32::MAX as usize - 1, 4)
                    .is_err()
            );
            let error = observe(Layout::Bits64, 0x10000, |_, _| Ok(vec![0])).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
    }
}
