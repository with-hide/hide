//! What the kernel says about a process asking for a Workspace credential
//! (PRD core-host-node D-21): which pane's shell it descends from, where it
//! works, whether a process is still the one that started, and the local
//! socket a pane's process asks on, whose system reports the caller's pid.
//!
//! These are facts of the machine the caller runs on, so they are the node's.
//! hided's node role reads them and the credential is issued from them.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use hide_platform::fs::private;

pub use hide_host::pane_peer::{
    PaneIdentity, current_address, descends_from, inspect, inspect_address, inspect_until,
    process_start,
};

/// The listener a pane's process asks for a capability on, or a node asks
/// its core on: a Unix socket or a named pipe, whose system reports the
/// caller's pid. It owns the private folder [`bind_recorded`] made for it:
/// the socket and its lock go as the listener drops, and the folder after
/// them, so a daemon that stopped leaves no folder behind.
pub struct BootstrapListener {
    listener: Option<hide_platform::ipc::LocalListener>,
    directory: PathBuf,
}

impl std::ops::Deref for BootstrapListener {
    type Target = hide_platform::ipc::LocalListener;

    fn deref(&self) -> &Self::Target {
        self.listener
            .as_ref()
            .expect("the listener is held until drop")
    }
}

impl Drop for BootstrapListener {
    fn drop(&mut self) {
        drop(self.listener.take());
        let _ = fs::remove_dir(&self.directory);
    }
}

/// The longest bootstrap socket path the record may hold: a short `/tmp`
/// path on Unix, the account's temporary folder on Windows.
const BOOTSTRAP_RECORD_CAP: u64 = 1024;

/// The folder `peer` works in, in the wire spelling, read from the kernel
/// and resolved to its real path; never from what the caller says.
pub fn caller_directory(peer: i32) -> Result<String, &'static str> {
    let cwd = hide_host::pane_peer::process_cwd(peer).ok_or("caller_unavailable")?;
    let canonical =
        hide_platform::fs::identity::canonical(&cwd).map_err(|_| "caller_unavailable")?;
    hide_platform::path::to_wire(&canonical).map_err(|_| "caller_unavailable")
}

/// Where the bootstrap's private folder goes. A Unix socket path is limited
/// to about a hundred bytes, so a short folder under `/tmp` there, whose
/// random name another user cannot reserve; a pipe name has no such limit,
/// so the account's own temporary folder on Windows.
fn bootstrap_parent() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir()
    } else {
        PathBuf::from("/tmp")
    }
}

/// Binds the bootstrap socket in a new private folder and records its path
/// in `state_dir`. `token` names the folder and the record's staging file;
/// each call must answer a new random value.
pub fn bind(
    state_dir: &Path,
    token: impl Fn() -> String,
) -> Result<(BootstrapListener, PathBuf), String> {
    bind_recorded(
        &bootstrap_socket_record(state_dir),
        "hide-pane",
        "b.sock",
        token,
    )
}

/// Binds a local socket named `name` in a new private folder whose name
/// starts with `prefix`, and records its path at `record`, so a process of
/// this account finds it there: the pane bootstrap, and the attach socket a
/// node's link reaches its core on (PRD core-host-node-remote-core D-07).
/// The folder is owner-only, so only this account's processes connect.
pub fn bind_recorded(
    record: &Path,
    prefix: &str,
    name: &str,
    token: impl Fn() -> String,
) -> Result<(BootstrapListener, PathBuf), String> {
    let parent = bootstrap_parent();
    let directory = (0..8)
        .find_map(|_| {
            let candidate = parent.join(format!("{prefix}-{}", &token()[..24]));
            match private::create_dir(&candidate) {
                Ok(()) => Some(Ok(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error.to_string())),
            }
        })
        .unwrap_or_else(|| Err(format!("{prefix} directory collision limit")))?;
    let path = directory.join(name);
    let result = (|| {
        let listener = BootstrapListener {
            listener: Some(
                hide_platform::ipc::LocalListener::bind(&path)
                    .map_err(|error| error.to_string())?,
            ),
            directory: directory.clone(),
        };
        private::restrict_to_owner(&path).map_err(|error| error.to_string())?;
        let staging = record.with_extension(format!("{}.tmp", &token()[..16]));
        let mut file = private::create_new_file(&staging).map_err(|error| error.to_string())?;
        let published = (|| {
            let text = path.to_str().ok_or("socket path is not text")?;
            file.write_all(text.as_bytes())
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            fs::rename(&staging, record).map_err(|error| error.to_string())
        })();
        if published.is_err() {
            let _ = fs::remove_file(staging);
        }
        published?;
        Ok((listener, path.clone()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
    }
    result
}

pub fn bootstrap_socket_record(state_dir: &Path) -> PathBuf {
    state_dir.join("pane-capabilities/bootstrap-socket")
}

/// The bootstrap socket the running daemon published, trusted only when the
/// record and the folder the socket sits in are this account's own and
/// private.
pub fn bootstrap_socket_path(state_dir: &Path) -> Result<PathBuf, String> {
    recorded_socket_path(&bootstrap_socket_record(state_dir))
        .map_err(|refusal| refusal.code().to_owned())
}

/// Why a socket record is not followed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordRefusal {
    /// There is no record this account can open: nothing published one.
    Missing,
    /// The record or the socket's folder is not this account's own and
    /// private, or the record is not a path.
    Untrusted,
    /// The folder the record names is gone: the process that wrote it ended
    /// without removing it.
    Stale,
}

impl RecordRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Missing => "hide_unavailable",
            Self::Untrusted | Self::Stale => "invalid_bootstrap_socket_record",
        }
    }
}

impl std::fmt::Display for RecordRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The socket a record written by [`bind_recorded`] names, trusted only when
/// the record and the socket's folder are this account's own and private.
pub fn recorded_socket_path(record: &Path) -> Result<PathBuf, RecordRefusal> {
    let file = private::open_own_file(record, false).map_err(|_| RecordRefusal::Missing)?;
    let metadata = file.metadata().map_err(|_| RecordRefusal::Missing)?;
    if !private::is_private(record).unwrap_or(false) || metadata.len() > BOOTSTRAP_RECORD_CAP {
        return Err(RecordRefusal::Untrusted);
    }
    let mut bytes = Vec::new();
    file.take(BOOTSTRAP_RECORD_CAP)
        .read_to_end(&mut bytes)
        .map_err(|_| RecordRefusal::Untrusted)?;
    let path = PathBuf::from(String::from_utf8(bytes).map_err(|_| RecordRefusal::Untrusted)?);
    let directory = path.parent().ok_or(RecordRefusal::Untrusted)?;
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RecordRefusal::Stale
        } else {
            RecordRefusal::Untrusted
        }
    })?;
    if !path.is_absolute()
        || !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || !private::owned_by_current_user(directory).unwrap_or(false)
        || !private::is_private(directory).unwrap_or(false)
    {
        return Err(RecordRefusal::Untrusted);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Windows cannot read a process's working directory (`cwd_of`).
    #[cfg(unix)]
    #[test]
    fn caller_directory_reads_the_callers_working_directory() {
        let expected = hide_platform::path::to_wire(
            &hide_platform::fs::identity::canonical(&std::env::current_dir().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(caller_directory(std::process::id() as i32), Ok(expected));
    }

    #[test]
    fn a_caller_that_is_gone_is_unavailable() {
        assert_eq!(caller_directory(i32::MAX), Err("caller_unavailable"));
    }

    /// A recorded listener takes its private folder with it: the socket,
    /// its lock and the folder are gone once it drops (L4).
    #[test]
    fn a_recorded_listener_leaves_no_folder_behind() {
        let state = tempfile::tempdir().unwrap();
        // The folder takes the token's first 24 characters, and the record's
        // staging file its first 16, so both vary within them: a folder a
        // failed run left never collides with the next run's.
        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let tokens = std::sync::atomic::AtomicU64::new(0);
        let token = || {
            let next = tokens.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("{next:04x}{started:016x}{:08x}", std::process::id())
        };
        let (listener, socket) =
            bind_recorded(&state.path().join("record"), "hide-test", "t.sock", token).unwrap();
        let folder = socket.parent().unwrap().to_path_buf();
        assert!(folder.is_dir());
        drop(listener);
        assert!(!folder.exists(), "{} was left", folder.display());
    }
}
