//! The node's side of files the operator attaches to a terminal: the read
//! of the files picked on the core's machine (bounded, regular files only,
//! each read whole and checked unchanged), and their staging on the machine
//! of a device's pane, which every device's link carries (PRD wsl-device
//! B10).

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use hide_node_link::attachments::{
    MAX_FILE_BYTES, MAX_FILES, MAX_PATH_BYTES, MAX_REQUEST_BYTES, MAX_STAGED_BYTES,
    MAX_STAGED_FILES, ReadFile, STAGING_TTL_SECONDS, valid_request_id,
};
use hide_node_link::feedback::File;
use hide_platform::fs::identity::stamp_of;
use hide_platform::fs::private;

/// Removes a paste's clipboard image. Only a clipboard image's own path is
/// accepted, so the call cannot name its way to another file.
pub fn remove_clipboard(path: &Path) -> Result<(), String> {
    if !hide_node_link::attachments::is_clipboard_path(path) {
        return Err("The path is not a pasted clipboard image.".to_owned());
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("The clipboard image could not be removed: {error}")),
    }
}

/// Reads each of `paths` in order; `go_on` is asked before each file, and a
/// false answer ends the read as cancelled.
pub fn read_sources(
    paths: &[String],
    go_on: &mut dyn FnMut(usize) -> bool,
) -> Result<Vec<ReadFile>, String> {
    read_files(paths, MAX_FILES, go_on)
}

/// Reads the staged files of a drop from the file manager, which may hold
/// more files than a terminal attachment but not more bytes
/// (`hide_node_link::mutate::IMPORT_MAX_ITEMS`).
pub fn read_dropped(
    paths: &[String],
    go_on: &mut dyn FnMut(usize) -> bool,
) -> Result<Vec<ReadFile>, String> {
    read_files(paths, hide_node_link::mutate::IMPORT_MAX_ITEMS, go_on)
}

fn read_files(
    paths: &[String],
    max_files: usize,
    go_on: &mut dyn FnMut(usize) -> bool,
) -> Result<Vec<ReadFile>, String> {
    if paths.is_empty() || paths.len() > max_files {
        return Err(format!("Choose between 1 and {max_files} regular files."));
    }
    let mut files = Vec::with_capacity(paths.len());
    let mut total = 0u64;
    for (index, path) in paths.iter().enumerate() {
        if !go_on(index) {
            return Err("File transfer was cancelled.".to_owned());
        }
        let path = Path::new(path);
        if path.as_os_str().len() > MAX_PATH_BYTES
            || !path.is_absolute()
            || path
                .as_os_str()
                .as_encoded_bytes()
                .iter()
                .any(|byte| byte.is_ascii_control())
        {
            return Err(
                "File paths must be absolute and contain no control characters.".to_owned(),
            );
        }
        let mut file = hide_platform::fs::open_regular(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "Folders, symbolic links and special files cannot be attached. Choose regular files."
                    .to_owned()
            } else {
                "A selected file is unavailable or is a symbolic link. Choose a regular file."
                    .to_owned()
            }
        })?;
        let before = file
            .metadata()
            .map_err(|_| "Could not inspect a selected file.".to_owned())?;
        let stamp = stamp_of(&file).map_err(|_| "Could not inspect a selected file.".to_owned())?;
        if before.len() > MAX_FILE_BYTES {
            return Err("A file exceeds the 20 MiB attachment limit.".to_owned());
        }
        total = total
            .checked_add(before.len())
            .ok_or("Attachment size overflow.")?;
        if total > MAX_REQUEST_BYTES {
            return Err("The selected files exceed the 40 MiB total attachment limit.".to_owned());
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                "Could not read a selected file. Check its permissions and retry.".to_owned()
            })?;
        let after = stamp_of(&file).map_err(|_| "Could not recheck a selected file.".to_owned())?;
        if bytes.len() as u64 != before.len() || stamp != after {
            return Err(
                "A selected file changed while it was being read. Choose it again.".to_owned(),
            );
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("A file name is not valid UTF-8.")?;
        files.push(ReadFile::new(
            path.to_string_lossy().into_owned(),
            name.to_owned(),
            &bytes,
        ));
    }
    Ok(files)
}

/// Requests one link may have staged in part at once; the core stages one
/// at a time, so more is a core that lost count, refused.
const MAX_OPEN_REQUESTS: usize = 8;

/// The terminal attachments one link staged on this machine: each request's
/// files, written one call at a time into the account's own folder
/// (`hide_kit::layout::terminal_attachments`), with generated names only,
/// owner-only, kept 24 hours past their last use and at most 128 files and
/// 256 MiB. A request whose files did not all arrive when the link ends, or
/// that its core cancels, is removed, so a link lost mid-transfer leaves
/// nothing behind; a file being written then is removed when its write
/// finishes, whichever comes first.
pub struct Staging {
    folder: Option<PathBuf>,
    legacy: Option<PathBuf>,
    legacy_swept: AtomicBool,
    open: Mutex<Open>,
}

/// The requests this link staged in part, and whether it ended.
#[derive(Default)]
struct Open {
    /// The link ended: no file is begun any more.
    ended: bool,
    requests: HashMap<String, Partial>,
}

/// A request staged in part.
struct Partial {
    count: usize,
    arrived: BTreeSet<usize>,
    /// Its files being written now.
    writing: usize,
    /// Cancelled, its link ended, or a write of it unwound, while a file
    /// was being written: the last write to leave removes its files.
    dropped: bool,
}

/// A write [`Staging::begin`] entered. It leaves exactly once: by
/// [`Writing::finish`], or as it drops when its worker unwinds first, which
/// drops its request so no file of it outlives the failure.
struct Writing<'a> {
    staging: &'a Staging,
    request: String,
    left: bool,
}

impl Writing<'_> {
    fn finish(mut self, index: usize, written: Result<PathBuf, String>) -> Result<String, String> {
        self.left = true;
        self.staging.leave(&self.request, Some((index, written)))
    }
}

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        if !self.left {
            // An unwound write has no caller to answer; `leave` logs what fails.
            let _ = self.staging.leave(&self.request, None);
        }
    }
}

impl Staging {
    /// Staging into the state folder `state`; `home` holds the folder an
    /// earlier build staged into over SSH.
    pub fn new(state: Option<&Path>, home: Option<&Path>) -> Self {
        Self {
            folder: state.map(hide_kit::layout::terminal_attachments),
            legacy: home.map(hide_kit::layout::legacy_terminal_attachments),
            legacy_swept: AtomicBool::new(false),
            open: Mutex::new(Open::default()),
        }
    }

    /// Stages the `index`th of `count` files of `request` and answers its
    /// path in the wire spelling. A file already at its place with the same
    /// bytes is kept, so a retried request writes nothing twice.
    pub fn stage(
        &self,
        request: &str,
        index: usize,
        count: usize,
        file: &File,
    ) -> Result<String, String> {
        if !valid_request_id(request) || count == 0 || count > MAX_FILES || index >= count {
            return Err("Invalid attachment request.".to_owned());
        }
        if file.len() > MAX_FILE_BYTES {
            return Err("A file exceeds the 20 MiB attachment limit.".to_owned());
        }
        let bytes = file.bytes()?;
        let writing = self.begin(request, count)?;
        let written = self.write(request, index, &file.name, &bytes);
        writing.finish(index, written)
    }

    /// Enters a write of `request`, which its link ending or its cancel
    /// cannot overtake: the request stays until the write leaves.
    fn begin(&self, request: &str, count: usize) -> Result<Writing<'_>, String> {
        let mut open = lock(&self.open);
        if open.ended {
            return Err("The link to this machine ended. Paste again.".to_owned());
        }
        if !open.requests.contains_key(request) && open.requests.len() >= MAX_OPEN_REQUESTS {
            return Err("Too many attachments are being staged at once. Retry.".to_owned());
        }
        let partial = open
            .requests
            .entry(request.to_owned())
            .or_insert_with(|| Partial {
                count,
                arrived: BTreeSet::new(),
                writing: 0,
                dropped: false,
            });
        if partial.dropped {
            return Err("This attachment was cancelled. Paste again.".to_owned());
        }
        partial.writing += 1;
        Ok(Writing {
            staging: self,
            request: request.to_owned(),
            left: false,
        })
    }

    /// Writes the file at its place, or finds the same bytes already there.
    fn write(
        &self,
        request: &str,
        index: usize,
        original: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, String> {
        let folder = self
            .folder
            .as_deref()
            .ok_or("This machine has no state folder for attachments.")?;
        self.sweep_legacy();
        private::own_dir(folder).map_err(|error| {
            format!("The attachment folder must be this account's private folder: {error}")
        })?;
        let kept = inspect_staging(folder)?;
        let name = attachment_name(request, index, original);
        let path = folder.join(&name);
        if kept.contains_key(&name) {
            if read_owned(&path)? != bytes {
                return Err(
                    "An earlier attachment at this request's place differs. Cancel and paste again."
                        .to_owned(),
                );
            }
        } else {
            if kept.len() >= MAX_STAGED_FILES
                || kept
                    .values()
                    .sum::<u64>()
                    .saturating_add(bytes.len() as u64)
                    > MAX_STAGED_BYTES
            {
                return Err("This machine's attachment storage is full (128 files or 256 MiB). Files expire after 24 hours; retry later.".to_owned());
            }
            write_new(&path, bytes)?;
        }
        Ok(path)
    }

    /// Leaves a write [`Staging::begin`] entered, with what it wrote, or
    /// with nothing when its worker unwound, which drops its request. A
    /// request dropped while it ran is refused, and its last write removes
    /// its files; a request whose files all arrived, or that holds none, is
    /// no longer open.
    fn leave(
        &self,
        request: &str,
        outcome: Option<(usize, Result<PathBuf, String>)>,
    ) -> Result<String, String> {
        let mut open = lock(&self.open);
        let Some(partial) = open.requests.get_mut(request) else {
            eprintln!(
                "{}",
                serde_json::json!({"component":"node","kind":"terminal.attachment.request_lost","request_id":request})
            );
            return Err("The attachment's request was lost while it was written.".to_owned());
        };
        partial.writing -= 1;
        let outcome = if partial.dropped { None } else { outcome };
        let Some((index, written)) = outcome else {
            partial.dropped = true;
            let last = partial.writing == 0;
            if last {
                open.requests.remove(request);
            }
            drop(open);
            if last {
                self.cleanup(request);
            }
            return Err("This attachment was cancelled or its link ended. Paste again.".to_owned());
        };
        if written.is_ok() {
            partial.arrived.insert(index);
        }
        if partial.writing == 0
            && (partial.arrived.len() >= partial.count || partial.arrived.is_empty())
        {
            open.requests.remove(request);
        }
        drop(open);
        hide_platform::path::to_wire(&written?).map_err(|error| error.to_string())
    }

    /// The core cancelled `request`: removes every file it staged, and a
    /// file being written once its write finishes. One already gone is not
    /// an error, and only a file with a name this staging generates is
    /// touched.
    pub fn remove(&self, request: &str) -> Result<(), String> {
        if !valid_request_id(request) {
            return Err("Invalid attachment request.".to_owned());
        }
        {
            let mut open = lock(&self.open);
            match open.requests.get_mut(request) {
                Some(partial) if partial.writing > 0 => partial.dropped = true,
                _ => {
                    open.requests.remove(request);
                }
            }
        }
        self.remove_files(request)
    }

    fn remove_files(&self, request: &str) -> Result<(), String> {
        let Some(folder) = self.folder.as_deref() else {
            return Ok(());
        };
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("The attachment folder could not be read: {error}")),
        };
        let prefix = format!("hide-{request}-");
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(&prefix)
                && owned_name(name)
                && entry.file_type().is_ok_and(|kind| kind.is_file())
            {
                remove_file(&entry.path())?;
            }
        }
        Ok(())
    }

    /// The link ended: no file is begun any more, and each request staged
    /// in part goes with it, a file being written once its write finishes.
    pub fn end(&self) {
        let gone: Vec<String> = {
            let mut open = lock(&self.open);
            open.ended = true;
            let mut gone = Vec::new();
            open.requests.retain(|request, partial| {
                if partial.writing > 0 {
                    partial.dropped = true;
                    return true;
                }
                gone.push(request.clone());
                false
            });
            gone
        };
        for request in gone {
            self.cleanup(&request);
        }
    }

    /// Removes a dropped request's files, with no caller left to tell.
    fn cleanup(&self, request: &str) {
        if let Err(reason) = self.remove_files(request) {
            eprintln!(
                "{}",
                serde_json::json!({"component":"node","kind":"terminal.attachment.cleanup_failed","request_id":request,"reason":reason})
            );
        }
    }

    /// Takes the files an earlier build left in its SSH staging folder once
    /// their 24 hours are over, and the folder once it is empty; once per
    /// link, and never a file another program put there.
    fn sweep_legacy(&self) {
        let Some(legacy) = self.legacy.as_deref() else {
            return;
        };
        if self.legacy_swept.swap(true, Ordering::Relaxed) {
            return;
        }
        let swept = match std::fs::symlink_metadata(legacy) {
            Ok(meta) if meta.is_dir() => inspect_staging(legacy).map(|kept| {
                if kept.is_empty() {
                    let _ = std::fs::remove_dir(legacy);
                }
            }),
            _ => Ok(()),
        };
        if let Err(reason) = swept {
            eprintln!(
                "{}",
                serde_json::json!({"component":"node","kind":"terminal.attachment.legacy_kept","reason":reason})
            );
        }
    }
}

/// A staged file's name: the request, the file's place in it and the
/// original's short extension, so an agent can tell an image by its name.
fn attachment_name(request: &str, index: usize, original: &str) -> String {
    let extension = Path::new(original)
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| {
            value.len() <= 12 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    format!(
        "hide-{request}-{index}{}",
        extension
            .map(|value| format!(".{value}"))
            .unwrap_or_default()
    )
}

/// Whether `name` is one [`attachment_name`] makes.
fn owned_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("hide-") else {
        return false;
    };
    rest.len() > 37
        && rest.is_char_boundary(36)
        && valid_request_id(&rest[..36])
        && rest.as_bytes()[36] == b'-'
        && rest[37..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
}

/// The staged files of `folder` still within their 24 hours, by name and
/// size, after removing those past them. Anything but this account's
/// private regular files with generated names stops the staging, so it is
/// never mistaken for room or removed.
fn inspect_staging(folder: &Path) -> Result<std::collections::BTreeMap<String, u64>, String> {
    let entries = std::fs::read_dir(folder)
        .map_err(|error| format!("The attachment folder could not be read: {error}"))?;
    let now = SystemTime::now();
    let mut kept = std::collections::BTreeMap::new();
    for (visited, entry) in entries.enumerate() {
        if visited >= MAX_STAGED_FILES * 2 {
            return Err(
                "The attachment folder holds too many entries. Remove old attachments and retry."
                    .to_owned(),
            );
        }
        let entry =
            entry.map_err(|error| format!("The attachment folder could not be read: {error}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("An attachment could not be inspected: {error}"))?;
        let ours = name.to_str().is_some_and(owned_name)
            && meta.is_file()
            && private::owned_by_current_user(&path).unwrap_or(false)
            && private::is_private(&path).unwrap_or(false);
        if !ours {
            return Err("The attachment folder holds an entry Hide did not stage. Inspect it before retrying.".to_owned());
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or(Duration::ZERO);
        if age.as_secs() >= STAGING_TTL_SECONDS {
            remove_file(&path)?;
        } else {
            kept.insert(name.to_string_lossy().into_owned(), meta.len());
        }
    }
    Ok(kept)
}

fn read_owned(path: &Path) -> Result<Vec<u8>, String> {
    let file = hide_platform::fs::open_regular(path)
        .map_err(|_| "An earlier attachment is unavailable. Cancel and paste again.".to_owned())?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("An earlier attachment could not be read: {error}"))?;
    Ok(bytes)
}

/// Writes a new private file, and takes a partly written one away.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = private::create_new_file(path)
        .map_err(|error| format!("The attachment could not be staged: {error}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("The attachment could not be staged: {error}"));
    }
    Ok(())
}

fn remove_file(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("An attachment could not be removed: {error}")),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const REQUEST: &str = "8a6e0e4c-3f0b-4d2c-9a51-0c6f2b9d7e10";

    fn staging() -> (tempfile::TempDir, Staging) {
        let folder = tempfile::tempdir().unwrap();
        let state = folder.path().join("state");
        fs::create_dir(&state).unwrap();
        let staging = Staging::new(Some(&state), Some(folder.path()));
        (folder, staging)
    }

    /// B10: each file lands as its own private file under a generated name;
    /// a retry of the same bytes is kept and other bytes at its place are
    /// refused, and the request's removal takes exactly its files.
    #[test]
    fn a_request_stages_its_files_privately_and_its_removal_takes_only_them() {
        let (_folder, staging) = staging();
        let first = File::new("한글 image.png", b"explicit\0\xff bytes");
        let path = staging.stage(REQUEST, 0, 2, &first).unwrap();
        let staged = PathBuf::from(&path);
        assert!(path.ends_with(&format!("terminal-attachments/hide-{REQUEST}-0.png")));
        assert_eq!(fs::read(&staged).unwrap(), b"explicit\0\xff bytes");
        assert!(private::is_private(&staged).unwrap());
        assert_eq!(staging.stage(REQUEST, 0, 2, &first).unwrap(), path);
        assert!(
            staging
                .stage(REQUEST, 0, 2, &File::new("한글 image.png", b"other"))
                .unwrap_err()
                .contains("differs")
        );
        let other = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let kept = staging
            .stage(other, 0, 1, &File::new("a.txt", b"keep"))
            .unwrap();
        staging.remove(REQUEST).unwrap();
        assert!(!staged.exists());
        assert!(Path::new(&kept).exists());
        staging.remove(REQUEST).unwrap();
    }

    /// A link that ends before a request's last file arrived takes the
    /// files it staged; a whole request stays for its pane to read.
    #[test]
    fn the_link_ending_mid_request_removes_what_it_staged() {
        let (_folder, staging) = staging();
        let partial = staging
            .stage(REQUEST, 0, 2, &File::new("a.txt", b"a"))
            .unwrap();
        let whole_id = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let whole = staging
            .stage(whole_id, 0, 1, &File::new("b.txt", b"b"))
            .unwrap();
        staging.end();
        assert!(!Path::new(&partial).exists());
        assert!(Path::new(&whole).exists());
    }

    /// The link ends between a write's start and its file appearing: the
    /// end cannot remove what is not there yet, so it leaves the request to
    /// that write, which removes its file as it finishes. Each step is the
    /// test's to order, as the node's workers would interleave them.
    #[test]
    fn a_write_the_link_s_end_overtook_removes_its_own_file() {
        let (_folder, staging) = staging();
        let writing = staging.begin(REQUEST, 2).unwrap();
        staging.end();
        // The end saw the write under way and left the request to it.
        assert!(lock(&staging.open).requests[REQUEST].dropped);
        let written = staging.write(REQUEST, 0, "a.txt", b"a");
        let path = written.clone().unwrap();
        assert!(path.exists());
        assert!(
            writing
                .finish(0, written)
                .unwrap_err()
                .contains("link ended")
        );
        assert!(
            !path.exists(),
            "the overtaken write left {}",
            path.display()
        );
        assert!(lock(&staging.open).requests.is_empty());
        let other = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert!(
            staging
                .stage(other, 0, 1, &File::new("b.txt", b"b"))
                .unwrap_err()
                .contains("ended")
        );
    }

    /// The core cancels a request between a write's start and its file
    /// appearing: the write removes its file as it finishes, and the next
    /// request stages as before.
    #[test]
    fn a_write_its_cancel_overtook_removes_its_own_file() {
        let (_folder, staging) = staging();
        let writing = staging.begin(REQUEST, 1).unwrap();
        staging.remove(REQUEST).unwrap();
        let written = staging.write(REQUEST, 0, "a.txt", b"a");
        let path = written.clone().unwrap();
        assert!(
            writing
                .finish(0, written)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(
            !path.exists(),
            "the cancelled write left {}",
            path.display()
        );
        let other = "0f8fad5b-d9cb-469f-a165-70867728950e";
        let kept = staging
            .stage(other, 0, 1, &File::new("b.txt", b"b"))
            .unwrap();
        assert!(Path::new(&kept).exists());
    }

    /// A worker that unwinds after its file exists still leaves its write:
    /// the request is dropped, and its files and its place go with it.
    #[test]
    fn a_write_whose_worker_unwinds_leaves_no_file_and_no_open_request() {
        let (_folder, staging) = staging();
        let earlier = staging
            .stage(REQUEST, 0, 3, &File::new("a.txt", b"a"))
            .unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _writing = staging.begin(REQUEST, 3).unwrap();
            let path = staging.write(REQUEST, 1, "b.txt", b"b").unwrap();
            assert!(path.exists());
            std::panic::panic_any(path);
        }));
        let path = unwound.unwrap_err().downcast::<PathBuf>().unwrap();
        assert!(!path.exists(), "the unwound write left {}", path.display());
        assert!(!Path::new(&earlier).exists());
        assert!(lock(&staging.open).requests.is_empty());
    }

    /// A file of exactly the cap is staged; one byte more is refused.
    #[test]
    fn a_file_is_judged_by_its_bytes_against_the_cap() {
        let (_folder, staging) = staging();
        let cap = MAX_FILE_BYTES as usize;
        for (index, size) in [cap - 1, cap].into_iter().enumerate() {
            let path = staging
                .stage(REQUEST, index, 2, &File::new("a.bin", &vec![1; size]))
                .unwrap();
            assert_eq!(fs::metadata(path).unwrap().len(), size as u64);
        }
        let other = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert!(
            staging
                .stage(other, 0, 1, &File::new("a.bin", &vec![1; cap + 1]))
                .unwrap_err()
                .contains("20 MiB")
        );
    }

    /// The folder's caps and its contents are judged before a byte is
    /// written: an entry Hide did not stage stops the staging.
    #[test]
    fn staging_refuses_a_folder_holding_what_hide_did_not_stage_and_past_its_cap() {
        let (folder, staging) = staging();
        let path = staging
            .stage(REQUEST, 0, 1, &File::new("a.txt", b"a"))
            .unwrap();
        let root = PathBuf::from(&path).parent().unwrap().to_path_buf();
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        let other = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert!(
            staging
                .stage(other, 0, 1, &File::new("b.txt", b"b"))
                .unwrap_err()
                .contains("did not stage")
        );
        fs::remove_file(root.join("notes.txt")).unwrap();
        assert!(staging.stage(REQUEST, 1, 1, &File::new("a", b"a")).is_err());
        assert!(
            staging
                .stage("not-a-request", 0, 1, &File::new("a", b"a"))
                .is_err()
        );
        drop(folder);
    }

    #[test]
    fn selected_files_are_read_and_special_files_refused() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().to_path_buf();
        let file = root.join("한글.png");
        fs::write(&file, b"explicit bytes").unwrap();
        let link = root.join("link.png");
        // A Windows account without the privilege cannot make the link.
        let linked = match hide_platform::fs::link::create_link(&file, &link) {
            Err(error) if hide_platform::fs::link::needs_privilege(&error) => false,
            made => made.map(|()| true).unwrap(),
        };
        let result = read_sources(&[file.to_string_lossy().into_owned()], &mut |_| true).unwrap();
        assert_eq!(result[0].bytes().unwrap(), b"explicit bytes");
        assert!(
            !linked || read_sources(&[link.to_string_lossy().into_owned()], &mut |_| true).is_err()
        );
        assert!(read_sources(&[root.to_string_lossy().into_owned()], &mut |_| true).is_err());
        let oversized = root.join("large");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert!(
            read_sources(&[oversized.to_string_lossy().into_owned()], &mut |_| true)
                .unwrap_err()
                .contains("20 MiB")
        );
        assert!(read_sources(&[file.to_string_lossy().into_owned()], &mut |_| false).is_err());
    }

    /// Only a paste's own clipboard image is removed; any other path the
    /// call names is refused and left in place.
    #[test]
    fn a_clipboard_removal_deletes_only_a_clipboard_image() {
        let folder = tempfile::tempdir().unwrap();
        let clipboard = folder
            .path()
            .join(hide_node_link::attachments::CLIPBOARD_FOLDER);
        fs::create_dir(&clipboard).unwrap();
        let id = "123e4567-e89b-12d3-a456-426614174000";
        let image = clipboard.join(hide_node_link::attachments::clipboard_file_name(id));
        fs::write(&image, b"png").unwrap();
        let other = clipboard.join("notes.txt");
        fs::write(&other, b"keep").unwrap();
        let outside = folder
            .path()
            .join(hide_node_link::attachments::clipboard_file_name(id));
        fs::write(&outside, b"keep").unwrap();

        assert!(remove_clipboard(&other).is_err());
        assert!(remove_clipboard(&outside).is_err());
        assert!(other.exists() && outside.exists());
        remove_clipboard(&image).unwrap();
        assert!(!image.exists());
        remove_clipboard(&image).unwrap();
    }
}
