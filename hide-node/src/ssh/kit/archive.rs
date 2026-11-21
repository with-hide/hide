//! A kit's archive: a gzip-compressed USTAR file of exactly the kit's files as
//! regular entries, under their `/`-separated paths, unpacked into a staging
//! folder. A link, a folder entry, a name the manifest does not list, a
//! duplicate or a file of another size is refused before it is written.

use super::contract::Header;
use super::manifest::{Bundle, MAX_KIT};
use anyhow::{Context, Result, ensure};
use hide_platform::fs::private;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// What tar adds around the kit's bytes: a 512-byte header and up to 511
/// bytes of padding per entry, the end-of-archive blocks and the 10 KiB
/// record a `tar` rounds up to, with room to spare.
const TAR_OVERHEAD: u64 = 64 * 1024;

/// Where the file `name` (`/`-separated) goes under `folder`.
pub(super) fn place(folder: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(folder.to_path_buf(), |path, part| path.join(part))
}

/// Unpacks `archive` into `stage`, which holds nothing else of the kit yet.
pub(super) fn unpack(archive: &Path, stage: &Path, bundle: &Bundle) -> Result<()> {
    let gzip = flate2::read::GzDecoder::new(File::open(archive)?);
    let mut tar = tar::Archive::new(gzip.take(MAX_KIT + TAR_OVERHEAD));
    let mut seen = BTreeSet::new();
    // Raw entries: no PAX or long-name record is read as a name, since this
    // contract's names all fit a USTAR header.
    for entry in tar.entries()?.raw(true) {
        let mut entry = entry?;
        let name = std::str::from_utf8(&entry.path_bytes())?.to_owned();
        let expected = bundle
            .files
            .get(&name)
            .with_context(|| format!("Unexpected path {name:?} in device archive"))?;
        ensure!(
            seen.insert(name.clone())
                && entry.header().entry_type().is_file()
                && entry.size() == expected.size,
            "Device archive contains a link, folder, duplicate, invalid type or size at {name:?}"
        );
        let target = place(stage, &name);
        if let Some(parent) = target.parent() {
            private::create_dir_all(parent)?;
        }
        let mut file = private::create_new_file(&target)?;
        ensure!(
            std::io::copy(&mut entry, &mut file)? == expected.size,
            "Truncated device file {name:?}"
        );
    }
    ensure!(
        seen.len() == bundle.files.len(),
        "Incomplete device archive"
    );
    // Consume gzip through its checksum; reaching the cap is a failure.
    let mut reader = tar.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink())?;
    ensure!(
        reader.limit() > 0,
        "Expanded device archive is over its size limit"
    );
    Ok(())
}

/// Whether `bytes` start as a program for `header` says, read without
/// running it: a packager or a core of one system checks another's.
pub(super) fn check_header(bytes: &[u8], header: Header) -> Result<()> {
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let valid = match header {
        // 64-bit, little-endian; an executable or a position-independent one.
        Header::Elf { machine } => {
            bytes.starts_with(&[0x7f, b'E', b'L', b'F', 2, 1])
                && matches!(u16_at(16), Some(2 | 3))
                && u16_at(18) == Some(machine)
        }
        // MH_MAGIC_64 and MH_EXECUTE; a universal file is refused.
        Header::MachO { cputype } => {
            u32_at(0) == Some(0xfeed_facf) && u32_at(4) == Some(cputype) && u32_at(12) == Some(2)
        }
        // The MS-DOS stub names where the PE signature and its machine are.
        Header::Pe { machine } => {
            bytes.starts_with(b"MZ")
                && u32_at(0x3c)
                    .map(|offset| offset as usize)
                    .is_some_and(|offset| {
                        bytes.get(offset..offset + 4) == Some(b"PE\0\0".as_slice())
                            && u16_at(offset + 4) == Some(machine)
                    })
        }
    };
    ensure!(valid, "Device executable is for another platform");
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A program's first 64 bytes for `header`, as its linker writes them.
    pub(in crate::ssh) fn program(header: Header) -> Vec<u8> {
        let mut bytes = vec![0; 128];
        match header {
            Header::Elf { machine } => {
                bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
                bytes[16] = 3;
                bytes[18..20].copy_from_slice(&machine.to_le_bytes());
            }
            Header::MachO { cputype } => {
                bytes[..4].copy_from_slice(&0xfeed_facf_u32.to_le_bytes());
                bytes[4..8].copy_from_slice(&cputype.to_le_bytes());
                bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
            }
            Header::Pe { machine } => {
                bytes[..2].copy_from_slice(b"MZ");
                bytes[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
                bytes[0x40..0x44].copy_from_slice(b"PE\0\0");
                bytes[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
            }
        }
        bytes
    }

    /// Each kind of header passes for its own processor and fails for any
    /// other kind or processor, and a truncated file passes for none.
    #[test]
    fn a_header_passes_only_for_its_own_kind_and_processor() {
        let kinds = [
            Header::Elf { machine: 62 },
            Header::Elf { machine: 183 },
            Header::MachO {
                cputype: 0x0100_000c,
            },
            Header::Pe { machine: 0x8664 },
        ];
        for header in kinds {
            for other in kinds {
                assert_eq!(
                    check_header(&program(header), other).is_ok(),
                    header == other,
                    "{header:?} checked as {other:?}"
                );
            }
            assert!(check_header(&program(header)[..8], header).is_err());
        }
        // A PE whose signature offset points past the file.
        let mut pe = program(Header::Pe { machine: 0x8664 });
        pe[0x3c..0x40].copy_from_slice(&0xffff_fff0_u32.to_le_bytes());
        assert!(check_header(&pe, Header::Pe { machine: 0x8664 }).is_err());
    }
}
