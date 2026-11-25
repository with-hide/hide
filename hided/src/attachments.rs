//! Staging for terminal attachments (PRD B14, B15, D-02, D-08).
//!
//! The browser can read a dropped file's bytes but cannot name its path, so
//! hided is the one that stages them: the client uploads one file per stage in
//! binary frames, and a commit turns the staged files into the one
//! `terminal_attachment` event the core takes for a batch of paths. The
//! staged file's path is what the core pastes, so it lands in the app state
//! dir, where it outlives the paste; only the newest `STAGED_KEEP` files are
//! kept, so the directory cannot grow with use (engineering 15). The caps are
//! the core's own numbers (20 MiB per file, 40 MiB and 8 files per batch);
//! hided refuses past them here rather than making the core read a file it will
//! reject. A clipboard image stages at the exact path the core reads it from.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hide_node_link::attachments::COMMIT_GRACE;

/// Bytes one attachment may hold, the same cap the core enforces.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Bytes one batch may hold, the same cap the core enforces.
pub const MAX_BATCH_BYTES: u64 = 40 * 1024 * 1024;
/// Files one batch may carry, the same cap the core enforces.
pub const MAX_FILES: usize = 8;
/// Staged files kept on disk. A pasted token is the staged path, so the bound
/// is generous; older files are removed only as newer ones arrive.
const STAGED_KEEP: usize = 512;

/// Bytes the staged files may hold together. Past it the oldest evictable
/// file goes, and nothing committed within the grace window is evicted: the
/// core may still be reading the file a just-pasted token names.
const MAX_STAGED_BYTES: u64 = 256 * 1024 * 1024;
/// Desk's own folder keeps this many of the newest files its messages sent,
/// and no more bytes than this; past either the oldest go first, never one
/// Desk still has to send (a waiting message's, the turn under way's) or one
/// given within `COMMIT_GRACE`, and a message those alone would push past
/// either is refused (`desk_full`). A message sent again whose file went is
/// told so under it (`hide-core/src/runtime/desk/worker.rs`).
const DESK_KEEP: usize = 64;
const DESK_KEEP_BYTES: u64 = 256 * 1024 * 1024;

/// Stages one connection may have open at once; past it the oldest open stage
/// is dropped, so an abandoned upload cannot hold file descriptors forever.
const MAX_OPEN_UPLOADS: usize = 64;

/// Whether an id has the 36-character UUID shape the core requires for an
/// attachment request (`hide-core/src/terminal_attachments.rs`).
pub fn valid_request_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// A random version-4 UUID, the shape [`valid_request_id`] takes; `None`
/// when the system gave no randomness.
pub fn random_id() -> Option<String> {
    let mut id = [0u8; 16];
    getrandom::getrandom(&mut id).ok()?;
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    let hex: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// One client-supplied name, flattened to a single relative component: a
/// separator, a control character, or a parent segment would let the client
/// name a path hided did not choose. Returns `None` for a shape no stage may
/// carry.
fn safe_component(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 96 {
        return None;
    }
    if value == "."
        || value == ".."
        || value
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return None;
    }
    Some(value.to_owned())
}

/// One staged file a commit hands on with its name (`commit_named`).
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct CommittedFile {
    pub name: String,
    pub bytes: u64,
    pub path: String,
}

struct Upload {
    path: PathBuf,
    file: Option<File>,
    written: u64,
    /// What the client said it would send, checked against the bytes at eof.
    declared: u64,
    /// Whether this stage writes the clipboard image the core reads by id.
    clipboard: bool,
}

/// One staged file on disk, with what eviction needs to judge it.
struct Staged {
    request_id: String,
    /// The connection that opened the stage; its stages are released when it
    /// goes, so a dead upload cannot hold a descriptor or a file.
    connection: u64,
    path: PathBuf,
    bytes: u64,
    /// When the core was told about it; until the grace window passes, the
    /// core may still be reading it, so eviction leaves it alone.
    committed_at: Option<Instant>,
}

#[derive(Default)]
struct State {
    uploads: HashMap<String, Upload>,
    /// The open stages in arrival order, oldest first, so the cap can drop one.
    open: VecDeque<String>,
    staged: VecDeque<Staged>,
    staged_bytes: u64,
}

/// Removes one stage everywhere the state names it: the file, the staged
/// entry (with its byte count), the open upload and its queue slot.
fn drop_stage(state: &mut State, request_id: &str) {
    let index = state
        .staged
        .iter()
        .position(|entry| entry.request_id == request_id);
    if let Some(entry) = index.and_then(|index| state.staged.remove(index)) {
        state.staged_bytes = state.staged_bytes.saturating_sub(entry.bytes);
        let _ = std::fs::remove_file(&entry.path);
    }
    state.uploads.remove(request_id);
    state.open.retain(|id| id != request_id);
}

/// Makes room for one more staged file of `size` bytes: the oldest entry that
/// is not a freshly committed file goes first, and a fully fresh set is
/// refused rather than unlinked (the core may still be reading it).
///
/// `keep` is the stage the caller is writing: it is never the victim, so a
/// byte-budget refusal leaves the stage it refused intact. `grows` is true
/// only for a caller that is about to add an entry, so the count cap is
/// checked exactly where the count can grow.
fn make_room(
    state: &mut State,
    size: u64,
    keep: Option<&str>,
    grows: bool,
) -> Result<(), &'static str> {
    while (grows && state.staged.len() >= STAGED_KEEP)
        || state.staged_bytes.saturating_add(size) > MAX_STAGED_BYTES
    {
        let Some(evictable) = state.staged.iter().find_map(|entry| {
            (Some(entry.request_id.as_str()) != keep
                && entry
                    .committed_at
                    .is_none_or(|at| at.elapsed() >= COMMIT_GRACE))
            .then(|| entry.request_id.clone())
        }) else {
            return Err("staging_full");
        };
        drop_stage(state, &evictable);
    }
    Ok(())
}

/// The files in `dir`, oldest first, with their bytes.
fn files_in(dir: &Path) -> Vec<(std::time::SystemTime, u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    files.sort_by_key(|(modified, _, _)| *modified);
    files
}

/// Leaves at most `count` files and `bytes` in `dir`, removing the oldest
/// first; `kept` (files in use and the ones just put there) are never removed.
fn keep_newest(dir: &Path, kept: &HashSet<PathBuf>, count: usize, bytes: u64) {
    let files = files_in(dir);
    let mut left = files.len();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    for (_, size, path) in files {
        if left <= count && total <= bytes {
            break;
        }
        if kept.contains(&path) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            left -= 1;
            total = total.saturating_sub(size);
        }
    }
}

pub struct Attachments {
    dir: PathBuf,
    /// Desk's files, kept across runs (`hide_kit::layout::desk_attachments`).
    desk_dir: PathBuf,
    clipboard_root: PathBuf,
    /// The account's state folder, where a View comment's screenshot is
    /// staged (`hide_host::feedback`).
    state_dir: PathBuf,
    state: Mutex<State>,
    /// The files given to Desk's messages within `COMMIT_GRACE`, which the
    /// core may not have queued yet; locked for a whole commit to Desk's
    /// folder, so two windows' commits take it one at a time.
    desk_recent: Mutex<Vec<(PathBuf, Instant)>>,
}

impl Attachments {
    pub fn new(state_dir: &Path) -> Self {
        let dir = hide_kit::layout::screen_attachments(state_dir);
        // The core reads a clipboard image from its own fixed path, so hided
        // stages it exactly there (`hide-core/src/terminal_attachments.rs`).
        let clipboard_root = state_dir.join(hide_node_link::attachments::CLIPBOARD_FOLDER);
        let desk_dir = hide_kit::layout::desk_attachments(state_dir);
        for folder in [&dir, &clipboard_root, &desk_dir] {
            // The staged bytes are the operator's own files and screenshots;
            // the directories stay private like the state file itself.
            if let Err(error) = hide_platform::fs::private::create_dir_all(folder) {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "hided",
                        "kind": "attachment.stage_dir_failed",
                        "message": error.to_string(),
                    })
                );
            }
        }
        // A previous run's staged files must not survive as garbage: the
        // attachments directory is hided's own, so it starts empty. The
        // clipboard directory may hold files another writer left, so only
        // its own retention window (24 h) is applied there.
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        if let Ok(entries) = std::fs::read_dir(&clipboard_root) {
            for entry in entries.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(24 * 60 * 60));
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Self {
            dir,
            desk_dir,
            clipboard_root,
            state_dir: state_dir.to_owned(),
            state: Mutex::new(State::default()),
            desk_recent: Mutex::new(Vec::new()),
        }
    }

    /// Stages one View comment's screenshot under a name Hide makes and
    /// answers that name; only the name reaches the core (PRD
    /// view-opener-feedback D-23). Blocks on the disk.
    pub fn stage_view_shot(&self, png: &[u8]) -> Result<String, String> {
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("The screenshot is not a PNG image.".to_owned());
        }
        let id = random_id().ok_or_else(|| "No name for the screenshot.".to_owned())?;
        let name = format!("shot-{id}.png");
        hide_host::feedback::stage_shot(&self.state_dir, &name, png)?;
        Ok(name)
    }

    /// Opens one stage: a file of `size` bytes the client will upload. The
    /// name is flattened to one component so it can never name another path.
    /// A clipboard image takes the path the core reads it from.
    pub fn begin(
        &self,
        connection: u64,
        request_id: &str,
        name: &str,
        size: u64,
        clipboard: bool,
    ) -> Result<(), &'static str> {
        if size > MAX_FILE_BYTES {
            return Err("too_large");
        }
        // Both components are client input; the id is flattened the same way
        // the name is, so the joined path is always one file directly in the
        // staging directory and can never name another path.
        let Some(request_id) = safe_component(request_id) else {
            return Err("invalid_request_id");
        };
        let Some(name) = safe_component(name) else {
            return Err("invalid_request_id");
        };
        // The core derives a clipboard file from a 36-character UUID, so a
        // clipboard stage must carry that shape or the core would never read
        // what hided wrote.
        if clipboard && !valid_request_id(&request_id) {
            return Err("invalid_request_id");
        }
        let root = if clipboard {
            &self.clipboard_root
        } else {
            &self.dir
        };
        let file_name = if clipboard {
            hide_node_link::attachments::clipboard_file_name(&request_id)
        } else {
            format!("{request_id}-{name}")
        };
        let path = root.join(&file_name);
        if path.parent() != Some(root.as_path()) || file_name.len() > 255 {
            return Err("invalid_request_id");
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // One id and one path per stage, checked and inserted under the same
        // lock: two connections could otherwise both pass a check made in a
        // separate critical section, and a joined name can collide across
        // different ids (`a-b` + `c` and `a` + `b-c`).
        if state
            .staged
            .iter()
            .any(|entry| entry.request_id == request_id || entry.path == path)
        {
            return Err("invalid_request_id");
        }
        // The count cap only; the byte budget is charged as bytes arrive.
        make_room(&mut state, 0, None, true)?;
        // `create_new` means an existing file (a case-insensitive collision,
        // or a path staged by another run) is never truncated.
        let file =
            hide_platform::fs::private::create_new_file(&path).map_err(|_| "stage_failed")?;
        // An abandoned stage would otherwise hold its file descriptor for the
        // daemon's life; the oldest open one goes instead.
        while state.open.len() >= MAX_OPEN_UPLOADS {
            let Some(oldest) = state.open.front().cloned() else {
                break;
            };
            drop_stage(&mut state, &oldest);
        }
        state.open.push_back(request_id.clone());
        state.staged.push_back(Staged {
            request_id: request_id.clone(),
            connection,
            path: path.clone(),
            bytes: 0,
            committed_at: None,
        });
        state.uploads.insert(
            request_id,
            Upload {
                path,
                file: Some(file),
                written: 0,
                declared: size,
                clipboard,
            },
        );
        Ok(())
    }

    /// Appends one chunk; `eof` closes the stage. Returns whether it closed.
    ///
    /// A stage that closes with fewer or more bytes than it declared is
    /// dropped: the declaration is what the client promised, and the arriving
    /// bytes are what the budget charges.
    pub fn write(&self, request_id: &str, bytes: &[u8], eof: bool) -> Result<bool, &'static str> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.uploads.get(request_id) {
            Some(upload) if upload.file.is_some() => {
                if upload.written + bytes.len() as u64 > MAX_FILE_BYTES {
                    return Err("too_large");
                }
            }
            // A frame after eof, or an id that never began, is not this stage.
            _ => return Err("unknown_stage"),
        }
        // Every arriving byte is charged before it lands, so the staged byte
        // budget bounds the disk at all times, not only at eof.
        let arriving = bytes.len() as u64;
        make_room(&mut state, arriving, Some(request_id), false)?;
        // A short write leaves bytes on disk that `written` never counted, so
        // the stage is dropped rather than resumed: its declared size can no
        // longer describe the file.
        let written_now = {
            let Some(upload) = state.uploads.get_mut(request_id) else {
                return Err("unknown_stage");
            };
            let Some(file) = upload.file.as_mut() else {
                return Err("unknown_stage");
            };
            match file.write_all(bytes) {
                Ok(()) => {
                    upload.written += arriving;
                    Some(upload.written)
                }
                Err(_) => None,
            }
        };
        let Some(written) = written_now else {
            drop_stage(&mut state, request_id);
            return Err("stage_failed");
        };
        let declared = state
            .uploads
            .get(request_id)
            .map(|upload| upload.declared)
            .unwrap_or_default();
        if let Some(entry) = state
            .staged
            .iter_mut()
            .find(|entry| entry.request_id == request_id)
        {
            entry.bytes = entry.bytes.saturating_add(arriving);
        }
        state.staged_bytes = state.staged_bytes.saturating_add(arriving);
        if eof {
            if written != declared {
                drop_stage(&mut state, request_id);
                return Err("size_mismatch");
            }
            if let Some(mut file) = state
                .uploads
                .get_mut(request_id)
                .and_then(|upload| upload.file.take())
            {
                let _ = file.flush();
            }
            // A completed stage holds no descriptor, so it does not consume an
            // open-upload slot while it waits for its commit.
            state.open.retain(|id| id != request_id);
            return Ok(true);
        }
        Ok(false)
    }

    /// The staged paths for a batch, in the order the client names them, with
    /// the batch caps applied. A clipboard batch names no paths: the core reads
    /// the image it staged at its own path. A stage that was never opened is an
    /// error, so a commit cannot name its way to a path hided did not write.
    pub fn commit(&self, stages: &[String], clipboard: bool) -> Result<Vec<String>, &'static str> {
        if clipboard {
            if stages.len() > 1 {
                return Err("too_many_files");
            }
            if let Some(stage) = stages.first() {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match state.uploads.get(stage) {
                    // The core reads the image at the path this stage's id
                    // derives, so only a clipboard stage may satisfy it.
                    Some(upload) if upload.file.is_none() && upload.clipboard => {}
                    _ => return Err("unknown_stage"),
                }
                // Committed: the file stays for the core to read, the open
                // stage entry does not.
                if let Some(entry) = state
                    .staged
                    .iter_mut()
                    .find(|entry| entry.request_id == *stage)
                {
                    entry.committed_at = Some(Instant::now());
                }
                state.uploads.remove(stage);
                state.open.retain(|id| id != stage);
            }
            return Ok(Vec::new());
        }
        self.commit_paths(stages, MAX_FILES, None)
    }

    /// The staged files of a message to Desk's agent (PRD desk-2-ui B41), in
    /// the order the client names them, with a batch's caps, moved into
    /// Desk's own folder: the stored message names them after a restart and
    /// past the staging folder's eviction. Each answers its name as the
    /// operator's file was called, its bytes and its path there. `in_use`
    /// are the files Desk still has to send (`Core::desk_files_in_use`):
    /// the folder's cap never removes them, nor a file given within
    /// `COMMIT_GRACE`, and when those alone leave no room for this message
    /// it is refused `desk_full` and its staged files let go. Blocks on the
    /// disk.
    pub fn commit_named(
        &self,
        stages: &[String],
        in_use: &[String],
    ) -> Result<Vec<CommittedFile>, &'static str> {
        let mut recent = self
            .desk_recent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let files = self.commit_files(stages, MAX_FILES, None)?;
        // The staging folder lets go of them: they are Desk's now.
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (moved, kept): (VecDeque<_>, VecDeque<_>) = std::mem::take(&mut state.staged)
                .into_iter()
                .partition(|entry| stages.contains(&entry.request_id));
            state.staged = kept;
            let freed: u64 = moved.iter().map(|entry| entry.bytes).sum();
            state.staged_bytes = state.staged_bytes.saturating_sub(freed);
        }
        recent.retain(|(_, at)| at.elapsed() < COMMIT_GRACE);
        let mut keep: HashSet<PathBuf> = in_use
            .iter()
            .map(PathBuf::from)
            .chain(recent.iter().map(|(path, _)| path.clone()))
            .collect();
        let (held, held_bytes) = files_in(&self.desk_dir)
            .into_iter()
            .filter(|(_, _, path)| keep.contains(path))
            .fold((0usize, 0u64), |(count, sum), (_, size, _)| {
                (count + 1, sum + size)
            });
        let incoming: u64 = files.iter().map(|(_, bytes)| bytes).sum();
        if held + files.len() > DESK_KEEP || held_bytes + incoming > DESK_KEEP_BYTES {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "hided",
                    "kind": "attachment.desk_full",
                    "held_files": held,
                    "held_bytes": held_bytes,
                    "files": files.len(),
                    "bytes": incoming,
                })
            );
            for (path, _) in &files {
                let _ = std::fs::remove_file(path);
            }
            return Err("desk_full");
        }
        let mut committed: Vec<CommittedFile> = Vec::with_capacity(files.len());
        for (stage, (path, bytes)) in stages.iter().zip(&files) {
            let staged = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            // A prompt quotes each path in `"`, so the kept name has none
            // (`hide-core/src/runtime/desk.rs`); the bubble keeps the file's name.
            let kept = self.desk_dir.join(staged.replace('"', "'"));
            if let Err(error) = std::fs::rename(path, &kept) {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "component": "hided",
                        "kind": "attachment.desk_keep_failed",
                        "request_id": stage,
                        "message": error.to_string(),
                    })
                );
                for (path, _) in &files {
                    let _ = std::fs::remove_file(path);
                }
                for file in &committed {
                    let _ = std::fs::remove_file(&file.path);
                }
                return Err("stage_failed");
            }
            // `begin` names a stage's file `<stage>-<name>`.
            let name = staged
                .strip_prefix(stage.as_str())
                .and_then(|rest| rest.strip_prefix('-'))
                .unwrap_or(&staged)
                .to_owned();
            committed.push(CommittedFile {
                name,
                bytes: *bytes,
                path: kept.display().to_string(),
            });
        }
        let now = Instant::now();
        for file in &committed {
            let path = PathBuf::from(&file.path);
            recent.push((path.clone(), now));
            keep.insert(path);
        }
        keep_newest(&self.desk_dir, &keep, DESK_KEEP, DESK_KEEP_BYTES);
        Ok(committed)
    }

    /// The staged paths of a phone's reply, in the order it names them, with
    /// a batch's caps: a stage this connection committed for a reply that did
    /// not land answers the same path again while its file is kept, so the
    /// reply's retry types the same files and their bytes never cross twice.
    pub fn commit_reply(
        &self,
        connection: u64,
        stages: &[String],
    ) -> Result<Vec<String>, &'static str> {
        self.commit_paths(stages, MAX_FILES, Some(connection))
    }

    /// The staged paths of a drop from the file manager, in the order the
    /// client names them: the same checks as a batch of attachments, with the
    /// count cap of one drop (`hide_node_link::mutate::IMPORT_MAX_ITEMS`).
    pub fn commit_dropped(&self, stages: &[String]) -> Result<Vec<String>, &'static str> {
        self.commit_paths(stages, hide_node_link::mutate::IMPORT_MAX_ITEMS, None)
    }

    fn commit_paths(
        &self,
        stages: &[String],
        max_files: usize,
        again: Option<u64>,
    ) -> Result<Vec<String>, &'static str> {
        Ok(self
            .commit_files(stages, max_files, again)?
            .into_iter()
            .map(|(path, _)| path.display().to_string())
            .collect())
    }

    /// `again` is the connection whose committed stages may be named again
    /// (`commit_reply`); every other commit takes open stages only.
    fn commit_files(
        &self,
        stages: &[String],
        max_files: usize,
        again: Option<u64>,
    ) -> Result<Vec<(PathBuf, u64)>, &'static str> {
        if stages.len() > max_files || stages.is_empty() {
            return Err("too_many_files");
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut total = 0u64;
        let mut paths = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for stage in stages {
            if !seen.insert(stage) {
                return Err("unknown_stage");
            }
            let (path, bytes) = match state.uploads.get(stage) {
                Some(upload) if upload.file.is_some() => return Err("stage_incomplete"),
                Some(upload) if upload.clipboard => return Err("unknown_stage"),
                Some(upload) => (upload.path.clone(), upload.written),
                None => {
                    let committed = again.and_then(|connection| {
                        state.staged.iter().find(|entry| {
                            entry.request_id == *stage
                                && entry.connection == connection
                                && entry.committed_at.is_some()
                        })
                    });
                    match committed {
                        Some(entry) => (entry.path.clone(), entry.bytes),
                        None => return Err("unknown_stage"),
                    }
                }
            };
            total += bytes;
            if total > MAX_BATCH_BYTES {
                return Err("batch_too_large");
            }
            paths.push((path, bytes));
        }
        // Committed: the files stay for the core to read, the open stage
        // entries do not, and the files are safe from eviction for a window.
        let committed_at = Instant::now();
        for stage in stages {
            if let Some(entry) = state
                .staged
                .iter_mut()
                .find(|entry| entry.request_id == *stage)
            {
                entry.committed_at = Some(committed_at);
            }
            state.uploads.remove(stage);
        }
        state.open.retain(|id| !stages.contains(id));
        Ok(paths)
    }

    /// Parses one binary upload frame (a 4-byte big-endian header length, the
    /// header JSON, then bytes) and writes it. A refusal names the request and
    /// the reason so the caller can answer it.
    pub fn receive(&self, connection: u64, data: &[u8]) -> Option<(String, &'static str)> {
        if data.len() < 4 {
            return None;
        }
        let header_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if 4 + header_len > data.len() {
            return None;
        }
        let header: serde_json::Value = serde_json::from_slice(&data[4..4 + header_len]).ok()?;
        let request_id = header.get("request_id")?.as_str()?.to_owned();
        let eof = header
            .get("eof")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let _ = connection;
        match self.write(&request_id, &data[4 + header_len..], eof) {
            Ok(_) => None,
            Err(reason) => Some((request_id, reason)),
        }
    }

    /// Releases the uncommitted stages a connection opened. A committed file
    /// stays: the core may still be reading the path it was handed.
    pub fn release(&self, connection: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let owned: Vec<String> = state
            .staged
            .iter()
            .filter(|entry| entry.connection == connection && entry.committed_at.is_none())
            .map(|entry| entry.request_id.clone())
            .collect();
        for request_id in owned {
            drop_stage(&mut state, &request_id);
        }
    }

    /// The file a stage wrote, once all its bytes have arrived.
    pub fn staged_path(&self, request_id: &str) -> Result<PathBuf, &'static str> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.uploads.get(request_id) {
            Some(upload) if upload.file.is_none() => Ok(upload.path.clone()),
            Some(_) => Err("stage_incomplete"),
            None => Err("unknown_stage"),
        }
    }

    /// Removes a stage and its file whatever its state: a node whose screen
    /// staged a file for another machine's pane sends the bytes on and keeps
    /// no copy (`node_uploads`).
    pub fn remove(&self, request_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        drop_stage(&mut state, request_id);
    }

    /// Drops a stage that was refused or abandoned, so the file goes with it.
    /// A file the core was already told about is left to eviction's grace: a
    /// cancel must not unlink a path the terminal was just handed.
    pub fn discard(&self, request_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let committed = state
            .staged
            .iter()
            .any(|entry| entry.request_id == request_id && entry.committed_at.is_some());
        state.uploads.remove(request_id);
        state.open.retain(|id| id != request_id);
        if !committed {
            drop_stage(&mut state, request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_screenshot_is_staged_under_a_name_hide_makes_and_only_a_png_is() {
        let (dir, service) = attachments();
        let png = b"\x89PNG\r\n\x1a\nrest of the image";
        let name = service.stage_view_shot(png).unwrap();
        assert!(hide_node_link::feedback::valid_shot_name(&name), "{name}");
        let staged = hide_kit::layout::view_feedback_shots(dir.path()).join(&name);
        assert_eq!(std::fs::read(staged).unwrap(), png);
        assert!(service.stage_view_shot(b"<svg/>").is_err());
        assert_ne!(
            service.stage_view_shot(png).unwrap(),
            name,
            "each shot is its own file"
        );
    }

    fn attachments() -> (tempfile::TempDir, Attachments) {
        let dir = tempfile::tempdir().unwrap();
        let service = Attachments::new(dir.path());
        (dir, service)
    }

    #[test]
    fn a_staged_file_is_written_and_committed_as_a_path() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "shot.png", 5, false).unwrap();
        assert_eq!(service.write("r1", &[1, 2, 3], false), Ok(false));
        assert_eq!(service.write("r1", &[4, 5], true), Ok(true));
        let paths = service.commit(&["r1".to_owned()], false).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(std::fs::read(&paths[0]).unwrap() == vec![1, 2, 3, 4, 5]);
        assert!(paths[0].contains("r1-shot.png"));
    }

    /// A Desk message's files (PRD desk-2-ui B41) land in the folder the
    /// layout names for a window's attachments, with the operator's own
    /// file names and sizes beside their staged paths.
    #[test]
    fn a_desk_message_s_files_are_moved_to_desk_s_own_folder_with_their_names() {
        let (dir, service) = attachments();
        let id = "0b7a3c52-9a1e-4f7d-8c2b-1a2b3c4d5e6f";
        for (index, (name, bytes)) in [("화면 a.png", &[1u8, 2, 3][..]), ("log\"1.txt", &[9u8][..])]
            .into_iter()
            .enumerate()
        {
            let stage = format!("{id}-{index}");
            service
                .begin(0, &stage, name, bytes.len() as u64, false)
                .unwrap();
            assert_eq!(service.write(&stage, bytes, true), Ok(true));
        }
        let files = service
            .commit_named(&[format!("{id}-0"), format!("{id}-1")], &[])
            .unwrap();
        assert_eq!(
            files
                .iter()
                .map(|file| (file.name.as_str(), file.bytes))
                .collect::<Vec<_>>(),
            [("화면 a.png", 3), ("log\"1.txt", 1)]
        );
        // The bubble keeps the file's name; the path a prompt quotes has no `"`.
        assert!(files[1].path.ends_with("-1-log'1.txt"), "{}", files[1].path);
        let folder = hide_kit::layout::desk_attachments(dir.path());
        for file in &files {
            assert_eq!(
                std::path::Path::new(&file.path).parent(),
                Some(folder.as_path())
            );
        }
        assert_eq!(std::fs::read(&files[0].path).unwrap(), [1, 2, 3]);
        assert_eq!(
            std::fs::read_dir(hide_kit::layout::screen_attachments(dir.path()))
                .unwrap()
                .count(),
            0,
            "the staging folder lets go of them"
        );
        // A commit takes open stages only: the same files cannot be named twice.
        assert_eq!(
            service.commit_named(&[format!("{id}-0")], &[]),
            Err("unknown_stage")
        );
        // A new run empties the staging folder, not Desk's.
        drop(service);
        let _ = Attachments::new(dir.path());
        assert_eq!(std::fs::read(&files[1].path).unwrap(), [9]);
    }

    /// letter-47595: while Desk answers, nine messages of eight files wait
    /// (72 files, past the folder's 64). The folder never removes a file a
    /// waiting message names: a message only those files would push past the
    /// cap is refused `desk_full` and its staged files let go, and once
    /// messages went, the next one removes the oldest file no message waits on.
    #[test]
    fn desk_s_folder_never_removes_a_file_desk_still_has_to_send() {
        let (dir, service) = attachments();
        let base = std::time::SystemTime::now() - Duration::from_secs(3600);
        let send = |message: u64, in_use: &[String]| {
            let stages: Vec<String> = (0..MAX_FILES)
                .map(|index| {
                    let stage = format!("m{message}-{index}");
                    service
                        .begin(0, &stage, &format!("f{index}.txt"), 1, false)
                        .unwrap();
                    assert_eq!(service.write(&stage, &[7], true), Ok(true));
                    stage
                })
                .collect();
            let files = service.commit_named(&stages, in_use)?;
            // Each message a minute after the one before, so "oldest" is
            // the order they were sent in; and past the commit's grace.
            for file in &files {
                File::options()
                    .write(true)
                    .open(&file.path)
                    .unwrap()
                    .set_modified(base + Duration::from_secs(message * 60))
                    .unwrap();
            }
            service.desk_recent.lock().unwrap().clear();
            Ok::<Vec<String>, &str>(files.into_iter().map(|file| file.path).collect())
        };
        let mut messages: Vec<Vec<String>> = Vec::new();
        for message in 0..8 {
            let in_use = messages.concat();
            messages.push(send(message, &in_use).unwrap());
        }
        assert_eq!(send(8, &messages.concat()), Err("desk_full"));
        for path in messages.concat() {
            assert!(Path::new(&path).exists(), "{path} was removed");
        }
        assert_eq!(
            std::fs::read_dir(hide_kit::layout::screen_attachments(dir.path()))
                .unwrap()
                .count(),
            0,
            "the refused message's staged files are let go"
        );

        // The first two messages went: the ninth fits, and the folder lets
        // go of the first message's files, the oldest no message waits on.
        let in_use = messages[2..].concat();
        let ninth = send(9, &in_use).unwrap();
        for path in &messages[0] {
            assert!(!Path::new(path).exists(), "{path} was kept");
        }
        for path in in_use.iter().chain(&ninth).chain(&messages[1]) {
            assert!(Path::new(path).exists(), "{path} was removed");
        }
    }

    #[test]
    fn desk_s_folder_keeps_its_newest_files_and_never_the_ones_just_sent() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("kept");
        std::fs::create_dir(&folder).unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(600);
        for index in 0..4 {
            let path = folder.join(format!("f{index}"));
            std::fs::write(&path, [0u8; 10]).unwrap();
            let at = old + Duration::from_secs(index * 60);
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        // The oldest file is the one just sent; past three files the next oldest goes.
        let fresh = folder.join("f0");
        keep_newest(&folder, &HashSet::from([fresh]), 3, u64::MAX);
        let mut left: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["f0", "f2", "f3"]);
        // Past the bytes the oldest go too.
        keep_newest(&folder, &HashSet::new(), 3, 10);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
        assert!(folder.join("f3").exists());
    }

    #[test]
    fn the_caps_and_shape_are_enforced() {
        let (_dir, service) = attachments();
        assert_eq!(
            service.begin(0, "r1", "big.bin", MAX_FILE_BYTES + 1, false),
            Err("too_large")
        );
        assert_eq!(service.write("missing", &[1], true), Err("unknown_stage"));
        service.begin(0, "r1", "a", 1, false).unwrap();
        assert_eq!(service.commit(&[], false), Err("too_many_files"));
        assert_eq!(
            service.commit(&["r1".to_owned()], false),
            Err("stage_incomplete")
        );
        service.write("r1", &[1], true).unwrap();
        assert_eq!(
            service.commit(&["nope".to_owned()], false),
            Err("unknown_stage")
        );
        let too_many: Vec<String> = (0..(MAX_FILES + 1)).map(|i| format!("r{i}")).collect();
        assert_eq!(service.commit(&too_many, false), Err("too_many_files"));
    }

    #[test]
    fn a_stage_id_that_names_a_path_is_refused() {
        let (_dir, service) = attachments();
        for id in ["../escape", "/tmp/absolute", "a/b", "", ".", ".."] {
            assert_eq!(
                service.begin(0, id, "shot.png", 1, false),
                Err("invalid_request_id"),
                "{id:?}"
            );
        }
        // The accepted id lands directly in the staging directory.
        service.begin(0, "ok-id", "shot.png", 1, false).unwrap();
        assert_eq!(service.write("ok-id", &[1], true), Ok(true));
        let path = service.commit(&["ok-id".to_owned()], false).unwrap();
        assert!(path[0].contains("/attachments/ok-id-shot.png"), "{path:?}");
    }

    #[test]
    fn abandoned_stages_are_bounded_and_their_files_go_with_them() {
        let (_dir, service) = attachments();
        for index in 0..(MAX_OPEN_UPLOADS + 2) {
            service
                .begin(0, &format!("s{index}"), "x", 1, false)
                .unwrap();
        }
        assert_eq!(service.write("s0", &[1], true), Err("unknown_stage"));
        assert_eq!(service.write("s1", &[1], true), Err("unknown_stage"));
        assert_eq!(
            service.write(&format!("s{}", MAX_OPEN_UPLOADS + 1), &[1], true),
            Ok(true)
        );
    }

    #[test]
    fn a_committed_stage_is_released_but_its_file_stays() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "shot.png", 1, false).unwrap();
        service.write("r1", &[1], true).unwrap();
        let paths = service.commit(&["r1".to_owned()], false).unwrap();
        assert!(std::path::Path::new(&paths[0]).is_file());
        assert_eq!(service.write("r1", &[1], true), Err("unknown_stage"));
    }

    #[test]
    fn staged_bytes_are_bounded_and_a_fresh_commit_is_spared() {
        let mut state = State::default();
        state.staged.push_back(Staged {
            request_id: "keep".to_owned(),
            connection: 0,
            path: PathBuf::from("/nonexistent/keep"),
            bytes: 1,
            committed_at: Some(Instant::now()),
        });
        state.staged.push_back(Staged {
            request_id: "old".to_owned(),
            connection: 0,
            path: PathBuf::from("/nonexistent/old"),
            bytes: MAX_STAGED_BYTES,
            committed_at: None,
        });
        state.staged_bytes = MAX_STAGED_BYTES + 1;
        make_room(&mut state, 1, None, true).unwrap();
        assert!(
            state.staged.iter().any(|entry| entry.request_id == "keep"),
            "a fresh commit stays"
        );
        assert!(!state.staged.iter().any(|entry| entry.request_id == "old"));
    }

    #[test]
    fn a_set_of_fresh_commits_refuses_rather_than_unlinks() {
        let mut state = State::default();
        for index in 0..2 {
            state.staged.push_back(Staged {
                request_id: format!("c{index}"),
                connection: 0,
                path: PathBuf::from(format!("/nonexistent/c{index}")),
                bytes: MAX_STAGED_BYTES,
                committed_at: Some(Instant::now()),
            });
        }
        state.staged_bytes = MAX_STAGED_BYTES * 2;
        assert_eq!(make_room(&mut state, 1, None, true), Err("staging_full"));
        assert_eq!(state.staged.len(), 2, "nothing may be unlinked");
    }

    #[test]
    fn a_stage_whose_bytes_do_not_match_its_declaration_is_dropped() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "x", 5, false).unwrap();
        assert_eq!(service.write("r1", &[1, 2], true), Err("size_mismatch"));
        assert_eq!(service.write("r1", &[1], true), Err("unknown_stage"));
        assert_eq!(
            service.commit(&["r1".to_owned()], false),
            Err("unknown_stage")
        );
        // The declared size is what the honest client sends.
        service.begin(0, "r2", "x", 2, false).unwrap();
        assert_eq!(service.write("r2", &[1, 2], true), Ok(true));
        assert!(service.commit(&["r2".to_owned()], false).is_ok());
    }

    #[test]
    fn one_id_is_one_stage_and_an_over_long_name_is_refused() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "a", 1, false).unwrap();
        assert_eq!(
            service.begin(0, "r1", "b", 1, false),
            Err("invalid_request_id")
        );
        assert_eq!(
            service.begin(0, "r2", &"a".repeat(97), 1, false),
            Err("invalid_request_id")
        );
    }

    #[test]
    fn staged_bytes_and_dirs_are_private() {
        let (dir, service) = attachments();
        service.begin(0, "r1", "shot.png", 1, false).unwrap();
        service.write("r1", &[1], true).unwrap();
        let staged = service.commit(&["r1".to_owned()], false).unwrap();
        use hide_platform::fs::private::is_private;
        assert!(
            is_private(Path::new(&staged[0])).unwrap(),
            "a staged file is the operator's own"
        );
        assert!(is_private(&dir.path().join("attachments")).unwrap());
    }

    #[test]
    fn the_count_cap_does_not_refuse_bytes_it_does_not_spend() {
        let (_dir, service) = attachments();
        {
            let mut state = service
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for index in 0..(STAGED_KEEP - 1) {
                state.staged.push_back(Staged {
                    request_id: format!("old{index}"),
                    connection: 0,
                    path: PathBuf::from(format!("/nonexistent/old{index}")),
                    bytes: 1,
                    committed_at: Some(Instant::now()),
                });
                state.staged_bytes += 1;
            }
        }
        // The 512th entry is admitted, and its bytes must not be refused by a
        // count cap the write is not spending.
        service.begin(0, "mine", "x", 4, false).unwrap();
        assert_eq!(service.write("mine", &[1, 2], false), Ok(false));
        assert_eq!(service.write("mine", &[3, 4], true), Ok(true));
        assert!(service.commit(&["mine".to_owned()], false).is_ok());
    }

    #[test]
    fn a_completed_stage_leaves_the_open_cap_and_a_dead_connection_is_released() {
        let (_dir, service) = attachments();
        service.begin(7, "done", "x", 1, false).unwrap();
        service.write("done", &[1], true).unwrap();
        for index in 0..(MAX_OPEN_UPLOADS - 1) {
            service
                .begin(7, &format!("open{index}"), "x", 1, false)
                .unwrap();
        }
        // The completed stage holds no slot, so a new stage evicts an open one.
        service.begin(7, "late", "x", 1, false).unwrap();
        let completed = service.commit(&["done".to_owned()], false);
        assert!(completed.is_ok(), "a completed stage survives the open cap");
        // A connection's uncommitted stages go with it; a committed file stays.
        service.release(7);
        assert_eq!(service.write("late", &[1], false), Err("unknown_stage"));
        assert!(Path::new(&completed.unwrap()[0]).is_file());
    }

    #[test]
    fn a_batch_names_each_stage_once_and_a_clipboard_id_is_a_uuid() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "x", 1, false).unwrap();
        service.write("r1", &[1], true).unwrap();
        assert_eq!(
            service.commit(&["r1".to_owned(), "r1".to_owned()], false),
            Err("unknown_stage")
        );
        assert_eq!(
            service.begin(0, "not-a-uuid", "x", 1, true),
            Err("invalid_request_id")
        );
        assert!(
            service
                .begin(0, "01234567-0123-0123-0123-0123456789ab", "x", 1, true)
                .is_ok()
        );
    }

    #[test]
    fn a_path_staged_under_another_id_is_refused() {
        let (_dir, service) = attachments();
        service.begin(0, "a-b", "c", 1, false).unwrap();
        // The joined name of a different, valid id would be the same file.
        assert_eq!(
            service.begin(0, "a", "b-c", 1, false),
            Err("invalid_request_id")
        );
    }

    #[test]
    fn arriving_bytes_are_charged_and_a_frame_after_eof_is_not_this_stage() {
        let (_dir, service) = attachments();
        service
            .begin(0, "r1", "x", 20 * 1024 * 1024, false)
            .unwrap();
        service.write("r1", &[0u8; 1024], false).unwrap();
        let charged = service
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .staged_bytes;
        assert_eq!(charged, 1024, "the budget sees the bytes on disk");
        assert_eq!(service.write("r1", &[1], true), Err("size_mismatch"));
        assert_eq!(service.write("r1", &[2], false), Err("unknown_stage"));
        assert_eq!(
            service
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .staged_bytes,
            0,
            "a dropped stage gives its bytes back"
        );
    }

    #[test]
    fn a_new_run_starts_with_an_empty_attachments_directory() {
        let dir = tempfile::tempdir().unwrap();
        let first = Attachments::new(dir.path());
        first.begin(0, "r1", "x", 1, false).unwrap();
        first.write("r1", &[1], true).unwrap();
        let staged = first.commit(&["r1".to_owned()], false).unwrap();
        assert!(Path::new(&staged[0]).is_file());
        let _second = Attachments::new(dir.path());
        assert!(
            !Path::new(&staged[0]).is_file(),
            "a previous run's staging must not survive as garbage"
        );
    }

    #[test]
    fn a_refused_chunk_leaves_the_writing_stage_intact() {
        let (_dir, service) = attachments();
        {
            let mut state = service
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.staged.push_back(Staged {
                request_id: "fresh".to_owned(),
                connection: 0,
                path: PathBuf::from("/nonexistent/fresh"),
                bytes: MAX_STAGED_BYTES,
                committed_at: Some(Instant::now()),
            });
            state.staged_bytes = MAX_STAGED_BYTES;
        }
        service.begin(0, "mine", "x", 4, false).unwrap();
        // The budget is full of a fresh commit, so the arriving bytes are
        // refused rather than evicting the very stage being written.
        assert_eq!(service.write("mine", &[1, 2], false), Err("staging_full"));
        assert_eq!(service.write("mine", &[1, 2], true), Err("staging_full"));
        assert_eq!(
            service.commit(&["mine".to_owned()], false),
            Err("stage_incomplete")
        );
    }

    #[test]
    fn a_commit_binds_a_stage_to_the_mode_it_was_opened_with() {
        let (_dir, service) = attachments();
        // A file stage cannot satisfy a clipboard commit: the core would read
        // a clipboard path this stage never wrote.
        service.begin(0, "file", "x", 1, false).unwrap();
        service.write("file", &[1], true).unwrap();
        assert_eq!(
            service.commit(&["file".to_owned()], true),
            Err("unknown_stage")
        );
        // A clipboard stage cannot satisfy a file batch either.
        let clip = "01234567-0123-0123-0123-0123456789ab";
        service.begin(0, clip, "shot.png", 1, true).unwrap();
        service.write(clip, &[1], true).unwrap();
        assert_eq!(
            service.commit(&[clip.to_owned()], false),
            Err("unknown_stage")
        );
        // Each mode commits to its own root.
        assert!(service.commit(&["file".to_owned()], false).is_ok());
        assert!(service.commit(&[clip.to_owned()], true).is_ok());
    }

    #[test]
    fn a_cancel_does_not_unlink_a_committed_file() {
        let (_dir, service) = attachments();
        service.begin(0, "r1", "x", 1, false).unwrap();
        service.write("r1", &[1], true).unwrap();
        let paths = service.commit(&["r1".to_owned()], false).unwrap();
        service.discard("r1");
        assert!(
            Path::new(&paths[0]).is_file(),
            "the core may still be reading what it was told about"
        );
    }

    #[test]
    fn a_batch_past_the_total_cap_is_refused() {
        let (_dir, service) = attachments();
        for i in 0..3 {
            let id = format!("r{i}");
            service
                .begin(0, &id, "chunk.bin", MAX_FILE_BYTES, false)
                .unwrap();
            // Write the cap's worth without allocating it all at once.
            let chunk = vec![0u8; 1024 * 1024];
            let mut written = 0u64;
            while written < MAX_FILE_BYTES {
                let remaining = MAX_FILE_BYTES - written;
                let take = remaining.min(chunk.len() as u64) as usize;
                written += take as u64;
                service
                    .write(&id, &chunk[..take], written >= MAX_FILE_BYTES)
                    .unwrap();
            }
        }
        assert_eq!(
            service.commit(&["r0".to_owned(), "r1".to_owned(), "r2".to_owned()], false),
            Err("batch_too_large")
        );
    }
}
