//! Files the operator picked to attach to a terminal, as the node that holds
//! them reads them: the limits both sides keep, and each file's bytes.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const MAX_FILES: usize = 8;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: u64 = 40 * 1024 * 1024;
/// Files and bytes staged on a device at once, and how long a staged file
/// may wait there before a later staging removes it.
pub const MAX_STAGED_FILES: usize = 128;
pub const MAX_STAGED_BYTES: u64 = 256 * 1024 * 1024;
pub const STAGING_TTL_SECONDS: u64 = 24 * 60 * 60;

/// One file to stage on a device, read already.
#[derive(Clone, Debug)]
pub struct AttachmentFile {
    pub path: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

/// An attachment request's id: a UUID in its hyphenated form.
pub fn valid_request_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// The folder beside the core's state file where a pasted clipboard image
/// waits, and the one file name a request's image takes there.
pub const CLIPBOARD_FOLDER: &str = "TerminalClipboard";

pub fn clipboard_file_name(request_id: &str) -> String {
    format!("hide-{request_id}.png")
}

/// Whether `path` names a clipboard image: a request's file directly inside
/// a clipboard folder, the only file a clipboard removal may delete.
pub fn is_clipboard_path(path: &std::path::Path) -> bool {
    fn name(component: Option<&std::ffi::OsStr>) -> Option<&str> {
        component.and_then(std::ffi::OsStr::to_str)
    }
    path.is_absolute()
        && clipboard_names(
            name(path.parent().and_then(std::path::Path::file_name)),
            name(path.file_name()),
        )
}

/// Whether a wire spelling (`hide_platform::path`) names a clipboard image,
/// by the same names as `is_clipboard_path`, so any machine may ask it of
/// any machine's path.
pub fn is_clipboard_wire_path(path: &str) -> bool {
    use hide_platform::path::{is_wire_absolute, wire_name, wire_parent};
    is_wire_absolute(path)
        && clipboard_names(wire_parent(path).map(wire_name), Some(wire_name(path)))
}

fn clipboard_names(folder: Option<&str>, name: Option<&str>) -> bool {
    folder == Some(CLIPBOARD_FOLDER)
        && name
            .and_then(|name| name.strip_prefix("hide-"))
            .and_then(|rest| rest.strip_suffix(".png"))
            .is_some_and(valid_request_id)
}

pub fn check_cancelled(cancelled: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        Err("File transfer was cancelled.".to_owned())
    } else {
        Ok(())
    }
}

/// How long a committed file is safe from eviction. The pasted token is the
/// staged file's path, so the core waits no longer than this for a terminal
/// to paste it into.
pub const COMMIT_GRACE: Duration = Duration::from_secs(60);

/// One picked file, read whole and unchanged while it was read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadFile {
    pub path: String,
    pub name: String,
    /// The bytes, base64 encoded for the JSON line.
    pub data: String,
}

impl ReadFile {
    pub fn new(path: String, name: String, bytes: &[u8]) -> Self {
        Self {
            path,
            name,
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|error| format!("A selected file's bytes arrived damaged: {error}"))
    }
}
