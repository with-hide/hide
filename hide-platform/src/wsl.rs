//! The Windows Subsystem for Linux distributions of this account, as a
//! Windows app reaches them (PRD wsl-device D-01, D-07, D-15).
//!
//! What a distribution is comes from WSL's own per-account registry
//! (`HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`): its name, the id
//! WSL gave it when it was installed, whether it runs as WSL 2 and whether
//! its first-run setup has run. Reading it starts no
//! process, so it can neither hang nor start a distribution that is off.
//! Which distributions run is `wsl.exe --list --running --quiet`, the one
//! process this module starts, bounded by its caller's deadline: a slow or
//! failing `wsl.exe` is a failure, never an empty answer. `wsl.exe` writes
//! UTF-16 without a byte order mark; [`decode`] reads that and UTF-8 alike.
//!
//! Only Windows has these; every other system answers `Unsupported`, except
//! [`running_from`], which asks the program it is given, so a stand-in for
//! `wsl.exe` answers it anywhere.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// The most output one `wsl.exe` listing may write; past it the listing is
/// a failure (engineering rule 15).
pub const MAX_LISTING_BYTES: usize = 64 * 1024;

/// The most distributions one account's registry is read for; an account
/// with more is a failure, never a shorter list (engineering rule 15).
pub const MAX_DISTRIBUTIONS: usize = 64;

/// One installed distribution, as WSL records it for this account.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Distribution {
    /// The name `wsl.exe -d` takes.
    pub name: String,
    /// The id WSL gave the distribution when it was installed, lowercased
    /// and without braces: a distribution installed again under the same
    /// name has another.
    pub id: String,
    /// 1 or 2.
    pub wsl_version: u32,
    /// WSL finished installing it: no install, conversion or removal is
    /// under way.
    pub installed: bool,
    /// Its first-run setup, which creates the account's Linux user, has not
    /// run yet.
    pub first_run_pending: bool,
    /// It is the distribution `wsl.exe` runs when none is named.
    pub default: bool,
}

/// Every distribution this account has installed, in registry order; none
/// when WSL never installed one. `Unsupported` off Windows.
pub fn distributions() -> io::Result<Vec<Distribution>> {
    sys::distributions()
}

/// The `wsl.exe` of this Windows installation, in its system folder; a
/// `wsl.exe` that comes first on `PATH` is never asked. `NotFound` on a
/// Windows without it, `Unsupported` off Windows.
pub fn program() -> io::Result<PathBuf> {
    sys::program()
}

/// The distributions that run now, as `program --list --running --quiet`
/// answers within `deadline`; one that is installed but not running is not
/// named. `TimedOut` when the program did not answer in time, and an error
/// when it failed, each of which says nothing about which run (D-15).
pub fn running_from(program: &Path, deadline: Duration) -> io::Result<BTreeSet<String>> {
    let mut command = Command::new(program);
    command
        .args(["--list", "--running", "--quiet"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = crate::process::OwnedChild::spawn(&mut command)?;
    let output = child
        .capture_until(Instant::now() + deadline, MAX_LISTING_BYTES)
        .map_err(|error| match error.kind {
            crate::process::CaptureFailureKind::Deadline => io::Error::new(
                io::ErrorKind::TimedOut,
                "wsl.exe did not list the running distributions in time",
            ),
            _ => io::Error::other(format!(
                "wsl.exe could not list the running distributions: {error}"
            )),
        })?;
    // With `--quiet`, a list with no running distribution exits 0 and
    // writes nothing (WSL's `WslClient.cpp`; only the form without
    // `--quiet` prints a sentence and exits -1), so a nonzero exit is
    // always a failure, whatever it wrote.
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "wsl.exe could not list the running distributions ({})",
            output.status
        )));
    }
    Ok(running_names(&decode(&output.stdout)))
}

/// The names in a `--list --running --quiet` answer: one per line, and any
/// line that is no distribution name (a sentence saying none runs) left out.
pub fn running_names(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .map(str::trim)
        .filter(|line| valid_distribution_name(line))
        .map(str::to_owned)
        .collect()
}

/// `wsl.exe` output as text: UTF-16 little-endian, with or without its byte
/// order mark, which `wsl.exe` writes unless `WSL_UTF8` is set, or UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let utf16 = bytes.starts_with(&[0xFF, 0xFE])
        || (bytes.len() >= 2
            && bytes.len().is_multiple_of(2)
            && bytes.iter().skip(1).step_by(2).any(|byte| *byte == 0));
    if !utf16 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Whether `name` is a distribution name Hide passes to `wsl.exe -d`: one
/// word of at most 64 characters with no path separator, control character
/// or leading `-`. WSL's own names are letters, digits, `.`, `_` and `-`;
/// any other letter is kept, so a name in another script is read as it is.
pub fn valid_distribution_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '"' | '\''))
}

/// Whether `user` is a Linux account name Hide passes to `wsl.exe -u`: the
/// portable account name POSIX defines, at most 32 characters.
pub fn valid_user_name(user: &str) -> bool {
    let mut chars = user.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && user.len() <= 32
        && user
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
}

/// The id WSL keeps a distribution under, as its registry key names it
/// (`{25622596-DB24-...}`), lowercased and without braces; `None` for a key
/// that is no id.
pub fn id_of_key(key: &str) -> Option<String> {
    let id = key.strip_prefix('{')?.strip_suffix('}')?;
    let shaped = id.len() == 36
        && id.char_indices().all(|(index, c)| match index {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
    shaped.then(|| id.to_ascii_lowercase())
}

/// WSL's flag for a distribution that runs in the WSL 2 virtual machine.
pub const FLAG_VM_MODE: u32 = 0x8;

/// WSL's state of a distribution that is installed and not being changed.
pub const STATE_INSTALLED: u32 = 1;

/// One distribution from the values WSL keeps under its key, or `None` for
/// a key that is not one (no id, no usable name).
pub fn distribution_of(
    key: &str,
    name: Option<String>,
    flags: Option<u32>,
    state: Option<u32>,
    run_oobe: Option<u32>,
    default_key: Option<&str>,
) -> Option<Distribution> {
    let id = id_of_key(key)?;
    let name = name.filter(|name| valid_distribution_name(name))?;
    Some(Distribution {
        name,
        wsl_version: if flags.unwrap_or(0) & FLAG_VM_MODE != 0 {
            2
        } else {
            1
        },
        installed: state == Some(STATE_INSTALLED),
        first_run_pending: run_oobe.unwrap_or(0) != 0,
        default: default_key.and_then(id_of_key).as_deref() == Some(id.as_str()),
        id,
    })
}

#[cfg(not(windows))]
mod sys {
    use std::io;
    use std::path::PathBuf;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "WSL distributions exist on Windows only",
        )
    }

    pub(super) fn distributions() -> io::Result<Vec<super::Distribution>> {
        Err(unsupported())
    }

    pub(super) fn program() -> io::Result<PathBuf> {
        Err(unsupported())
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::OsString;
    use std::io;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey,
        RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    const LXSS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Lxss";

    /// An open registry key, closed when it goes.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle came from RegOpenKeyExW and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn wide(text: &str) -> io::Result<widestring::U16CString> {
        widestring::U16CString::from_str(text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }

    fn text(key: HKEY, value: &str) -> io::Result<Option<String>> {
        let value = wide(value)?;
        let mut buffer = vec![0u16; 512];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the value name is NUL-terminated and `buffer` holds `size`
        // writable bytes.
        let status = unsafe {
            RegGetValueW(
                key,
                null(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let units = (size as usize / 2).saturating_sub(1);
        Ok(Some(
            OsString::from_wide(&buffer[..units])
                .to_string_lossy()
                .into_owned(),
        ))
    }

    fn number(key: HKEY, value: &str) -> io::Result<Option<u32>> {
        let value = wide(value)?;
        let mut number = 0u32;
        let mut size = 4u32;
        // SAFETY: the value name is NUL-terminated and `number` holds the
        // four bytes a DWORD takes.
        let status = unsafe {
            RegGetValueW(
                key,
                null(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                null_mut(),
                (&mut number as *mut u32).cast(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(Some(number))
    }

    fn open(parent: HKEY, name: &str) -> io::Result<Option<Key>> {
        let name = wide(name)?;
        let mut key: HKEY = null_mut();
        // SAFETY: the name is NUL-terminated; `key` receives the handle.
        let status = unsafe { RegOpenKeyExW(parent, name.as_ptr(), 0, KEY_READ, &mut key) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(Some(Key(key)))
    }

    pub(super) fn distributions() -> io::Result<Vec<super::Distribution>> {
        // No key: WSL never installed a distribution for this account.
        let Some(root) = open(HKEY_CURRENT_USER, LXSS)? else {
            return Ok(Vec::new());
        };
        let default_key = text(root.0, "DefaultDistribution")?;
        let mut found = Vec::new();
        for index in 0u32.. {
            let mut name = vec![0u16; 256];
            let mut length = name.len() as u32;
            // SAFETY: `name` holds `length` writable units; the class and
            // time are not asked for.
            let status = unsafe {
                RegEnumKeyExW(
                    root.0,
                    index,
                    name.as_mut_ptr(),
                    &mut length,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            if index as usize >= super::MAX_DISTRIBUTIONS {
                return Err(io::Error::other(format!(
                    "this account has more than {} WSL distributions, more than Hide reads",
                    super::MAX_DISTRIBUTIONS
                )));
            }
            let key_name = OsString::from_wide(&name[..length as usize])
                .to_string_lossy()
                .into_owned();
            let Some(key) = open(root.0, &key_name)? else {
                continue;
            };
            if let Some(distribution) = super::distribution_of(
                &key_name,
                text(key.0, "DistributionName")?,
                number(key.0, "Flags")?,
                number(key.0, "State")?,
                number(key.0, "RunOOBE")?,
                default_key.as_deref(),
            ) {
                found.push(distribution);
            }
        }
        Ok(found)
    }

    pub(super) fn program() -> io::Result<PathBuf> {
        let mut buffer = vec![0u16; 260];
        // SAFETY: `buffer` holds its length in writable units.
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err(io::Error::last_os_error());
        }
        let program =
            PathBuf::from(OsString::from_wide(&buffer[..length as usize])).join("wsl.exe");
        if !program.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "this Windows has no wsl.exe",
            ));
        }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// The bytes `wsl.exe --list --verbose` wrote on GitHub's Windows runner
    /// (2026-10-28, WSL 3.0.3): UTF-16 little-endian, no byte order mark,
    /// CRLF line ends.
    #[test]
    fn wsl_output_reads_as_text_whichever_encoding_it_came_in() {
        let listing =
            "  NAME            STATE           VERSION\r\n* Ubuntu-24.04    Running         2\r\n";
        assert_eq!(decode(&utf16(listing)), listing);
        let mut marked = vec![0xFF, 0xFE];
        marked.extend(utf16("우분투\r\n"));
        assert_eq!(decode(&marked), "우분투\r\n");
        assert_eq!(decode("Ubuntu\n".as_bytes()), "Ubuntu\n");
        assert_eq!(decode("데비안\n".as_bytes()), "데비안\n");
    }

    #[test]
    fn a_running_listing_names_only_distributions() {
        let names = running_names(&decode(&utf16(
            "Ubuntu-24.04\r\n우분투\r\n\r\nThere are no running distributions.\r\n",
        )));
        assert_eq!(
            names,
            BTreeSet::from(["Ubuntu-24.04".to_owned(), "우분투".to_owned()])
        );
        assert!(running_names("").is_empty());
    }

    /// D-15: a `wsl.exe` that fails is never read as nothing running.
    #[cfg(unix)]
    #[test]
    fn a_failing_wsl_exe_says_nothing_about_which_distributions_run() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("wsl.exe");
        let answer = |script: &str| {
            std::fs::write(&program, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            running_from(&program, Duration::from_secs(10))
        };
        assert!(answer("exit 1").is_err(), "silent and failing");
        assert!(
            answer("echo 'Access is denied.' >&2; exit 1").is_err(),
            "a refusal on standard error"
        );
        assert!(
            answer("echo 'There are no running distributions.'; exit 1").is_err(),
            "whatever a failing exit says"
        );
        assert_eq!(answer("exit 0").unwrap(), BTreeSet::new(), "none runs");
        assert_eq!(
            answer("echo Ubuntu; exit 0").unwrap(),
            BTreeSet::from(["Ubuntu".to_owned()])
        );
    }

    #[test]
    fn a_distribution_reads_its_registry_values() {
        // The values the runner's Ubuntu-24.04 had right after setup-wsl
        // installed it without its first run (2026-10-28).
        let distribution = distribution_of(
            "{25622596-DB24-4761-8039-E647093F5F7C}",
            Some("Ubuntu-24.04".to_owned()),
            Some(15),
            Some(1),
            Some(1),
            Some("{25622596-db24-4761-8039-e647093f5f7c}"),
        )
        .unwrap();
        assert_eq!(
            distribution,
            Distribution {
                name: "Ubuntu-24.04".to_owned(),
                id: "25622596-db24-4761-8039-e647093f5f7c".to_owned(),
                wsl_version: 2,
                installed: true,
                first_run_pending: true,
                default: true,
            }
        );
        let wsl1 = distribution_of(
            "{00000000-0000-0000-0000-000000000001}",
            Some("Legacy".to_owned()),
            Some(7),
            Some(3),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            (
                wsl1.wsl_version,
                wsl1.installed,
                wsl1.first_run_pending,
                wsl1.default
            ),
            (1, false, false, false)
        );
        assert!(
            distribution_of("not-an-id", Some("x".to_owned()), None, None, None, None).is_none()
        );
        assert!(
            distribution_of(
                "{00000000-0000-0000-0000-000000000002}",
                Some("two words".to_owned()),
                None,
                None,
                None,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn names_hide_passes_to_wsl_exe_are_one_word() {
        for name in [
            "Ubuntu",
            "Ubuntu-24.04",
            "kali-linux",
            "openSUSE-Leap-15.2",
            "우분투",
        ] {
            assert!(valid_distribution_name(name), "{name}");
        }
        for name in [
            "",
            "-d",
            "two words",
            "a/b",
            "a\\b",
            "x\ny",
            "q\"",
            &"x".repeat(65),
        ] {
            assert!(!valid_distribution_name(name), "{name:?}");
        }
        for user in ["hoyeon", "_svc", "a.b-c_1"] {
            assert!(valid_user_name(user), "{user}");
        }
        for user in ["", "Root", "1st", "a b", "-u", &"a".repeat(33)] {
            assert!(!valid_user_name(user), "{user:?}");
        }
    }

    #[test]
    fn the_distribution_list_is_windows_only() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(
            distributions().unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(program().unwrap_err().kind(), io::ErrorKind::Unsupported);
    }
}
