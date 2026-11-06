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

use std::collections::{HashMap, VecDeque};
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

pub struct Attachments {
    dir: PathBuf,
    clipboard_root: PathBuf,
    /// The account's state folder, where a View comment's screenshot is
    /// staged (`hide_host::feedback`).
    state_dir: PathBuf,
    state: Mutex<State>,
}

impl Attachments {
    pub fn new(state_dir: &Path) -> Self {
        let dir = state_dir.join("attachments");
        // The core reads a clipboard image from its own fixed path, so hided
        // stages it exactly there (`hide-core/src/terminal_attachments.rs`).
        let clipboard_root = state_dir.join(hide_node_link::attachments::CLIPBOARD_FOLDER);
        for folder in [&dir, &clipboard_root] {
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
            clipboard_root,
            state_dir: state_dir.to_owned(),
            state: Mutex::new(State::default()),
        }
    }

    /// Stages one View comment's screenshot under a name Hide makes and
    /// answers that name; only the name reaches the core (PRD
    /// view-opener-feedback D-23). Blocks on the disk.
    pub fn stage_view_shot(&self, png: &[u8]) -> Result<String, String> {
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("The screenshot is not a PNG image.".to_owned());
        }
        let mut id = [0u8; 16];
        getrandom::getrandom(&mut id).map_err(|_| "No name for the screenshot.".to_owned())?;
        id[6] = (id[6] & 0x0f) | 0x40;
        id[8] = (id[8] & 0x3f) | 0x80;
        let hex: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!(
            "shot-{}-{}-{}-{}-{}.png",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );
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
        self.commit_files(stages, MAX_FILES)
    }

    /// The staged paths of a drop from the file manager, in the order the
    /// client names them: the same checks as a batch of attachments, with the
    /// count cap of one drop (`hide_node_link::mutate::IMPORT_MAX_ITEMS`).
    pub fn commit_dropped(&self, stages: &[String]) -> Result<Vec<String>, &'static str> {
        self.commit_files(stages, hide_node_link::mutate::IMPORT_MAX_ITEMS)
    }

    fn commit_files(
        &self,
        stages: &[String],
        max_files: usize,
    ) -> Result<Vec<String>, &'static str> {
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
            let Some(upload) = state.uploads.get(stage) else {
                return Err("unknown_stage");
            };
            if !seen.insert(stage) {
                return Err("unknown_stage");
            }
            if upload.file.is_some() {
                return Err("stage_incomplete");
            }
            if upload.clipboard {
                return Err("unknown_stage");
            }
            total += upload.written;
            if total > MAX_BATCH_BYTES {
                return Err("batch_too_large");
            }
            paths.push(upload.path.display().to_string());
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
