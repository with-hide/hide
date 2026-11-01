//! The own engine's saved session (PRD engine-bundle-3-core-owns D-6): one
//! file, `hide_kit::layout::engine_session` (`<state>/engine/session.json`),
//! holding the workspaces, tabs, pane trees and panes (folder, name, Hide's
//! tokens, the agent and its session) and the id counters. It is the first
//! form of a user's session Hide saves itself and outlives bundle 4, so it
//! carries a format number from its first write: this build reads format
//! [`FORMAT`] and refuses any other, and a field added later comes with the
//! next number. What a pane was started with (its command and environment)
//! is not saved, since an environment can carry what is not the session's.
//!
//! One saving thread writes it ([`Saver`]): a change marks the session
//! changed and wakes it, it waits [`SETTLE`] for the changes of the same
//! operation to land, reads the session once under the topology lock and
//! replaces the file whole, durably, outside it; so an operation writes it
//! at most once, and the last change is written as the engine ends. A file
//! this build cannot read is never deleted or overwritten in place: it is
//! moved to `hide_kit::layout::engine_backups`, the newest [`BACKUPS_KEPT`]
//! kept, with the reason in the diagnostic log.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The format this build writes and reads.
pub(super) const FORMAT: u32 = 1;

/// How long the saving thread lets one operation's changes land before it
/// writes them once.
pub(super) const SETTLE: Duration = Duration::from_millis(200);

/// How many unreadable saved sessions are kept.
pub(super) const BACKUPS_KEPT: usize = 3;

/// The largest saved session read: about 24 times what 128 panes with long
/// folders, names and tokens take.
const READ_CAP: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub(super) format: u32,
    pub(super) workspace_numbers: usize,
    pub(super) focused_workspace: Option<String>,
    pub(super) workspaces: Vec<SavedWorkspace>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedWorkspace {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) tokens: BTreeMap<String, Value>,
    pub(super) worktree: Option<SavedWorktree>,
    pub(super) active_tab: String,
    pub(super) tab_numbers: usize,
    pub(super) pane_numbers: usize,
    pub(super) tabs: Vec<SavedTab>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedWorktree {
    pub(super) repo_key: String,
    pub(super) repo_name: String,
    pub(super) repo_root: String,
    pub(super) checkout_path: String,
    pub(super) linked: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedTab {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) focused: String,
    pub(super) zoomed: bool,
    pub(super) root: SavedNode,
    pub(super) panes: Vec<SavedPane>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum SavedNode {
    Pane {
        id: String,
    },
    Split {
        direction: SavedDirection,
        ratio: f32,
        first: Box<SavedNode>,
        second: Box<SavedNode>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SavedDirection {
    Right,
    Down,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedPane {
    pub(super) id: String,
    pub(super) cwd: String,
    pub(super) label: Option<String>,
    pub(super) tokens: BTreeMap<String, Value>,
    pub(super) agent: Option<SavedAgent>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SavedAgent {
    /// The agent definition's id.
    pub(super) kind: String,
    /// The name the engine started it under, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    /// Its session, an id or a session file's path, once known; an agent
    /// with none resumes its folder's last conversation when its definition
    /// says how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<String>,
}

/// What reading the saved session found.
#[derive(Debug)]
pub(super) enum Read {
    /// No session was saved yet.
    Missing,
    Saved(Saved),
    /// The file could not be read as a session of this build. It has been
    /// moved to the backups and why logged.
    Unreadable,
}

/// Reads the saved session at `path`, moving a file it cannot read to
/// `backups` (the newest [`BACKUPS_KEPT`] kept) and logging why.
pub(super) fn read(path: &Path, backups: &Path) -> Read {
    let why = match read_text(path) {
        Ok(None) => return Read::Missing,
        Ok(Some(text)) => match parse(&text) {
            Ok(saved) => return Read::Saved(saved),
            Err(why) => why,
        },
        Err(why) => why,
    };
    set_aside_unusable(path, backups, &why);
    Read::Unreadable
}

/// Moves the saved session at `path`, which cannot be used for `why`, to
/// `backups` (the newest [`BACKUPS_KEPT`] kept) and logs why: a session
/// that reads but does not hold together is set aside as one that does not
/// read is.
pub(super) fn set_aside_unusable(path: &Path, backups: &Path, why: &str) {
    let kept = set_aside(path, backups);
    crate::diagnostic!(serde_json::json!({
        "component": "own_engine",
        "kind": "engine.session_unreadable",
        "path": path.display().to_string(),
        "reason": why,
        "kept_as": kept.as_ref().ok().map(|kept| kept.display().to_string()),
        "keep_failed": kept.as_ref().err(),
    }));
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("the file could not be opened: {error}")),
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(file, READ_CAP + 1), &mut text)
        .map_err(|error| format!("the file could not be read: {error}"))?;
    if text.len() as u64 > READ_CAP {
        return Err(format!("the file is larger than {READ_CAP} bytes"));
    }
    Ok(Some(text))
}

/// The session `text` holds, refused when it names a format this build does
/// not read or does not have the shape of one.
pub(super) fn parse(text: &str) -> Result<Saved, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("the file is not JSON: {error}"))?;
    match value.get("format").and_then(Value::as_u64) {
        Some(format) if format == u64::from(FORMAT) => {}
        Some(format) => {
            return Err(format!(
                "the file is format {format}, this build reads format {FORMAT}"
            ));
        }
        None => return Err("the file names no format".to_owned()),
    }
    serde_json::from_value(value).map_err(|error| format!("the file is not a session: {error}"))
}

/// Moves `path` into `backups` under a name that sorts after every backup
/// already there, then removes all but the newest [`BACKUPS_KEPT`]: where it
/// went.
fn set_aside(path: &Path, backups: &Path) -> Result<PathBuf, String> {
    hide_platform::fs::private::create_dir_all(backups)
        .map_err(|error| format!("{} could not be made: {error}", backups.display()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let newest = backup_names(backups)
        .last()
        .and_then(|name| backup_order(name));
    let kept = backups.join(backup_name(next_backup_order(now, newest)));
    std::fs::rename(path, &kept)
        .map_err(|error| format!("{} could not be moved: {error}", path.display()))?;
    let found = backup_names(backups);
    let excess = found.len().saturating_sub(BACKUPS_KEPT);
    for old in found.into_iter().take(excess) {
        let _ = std::fs::remove_file(backups.join(old));
    }
    Ok(kept)
}

/// The most turns one millisecond's name carries before the next backup
/// moves on to the following millisecond, so names keep their fixed width.
const BACKUP_TURNS: u32 = 1000;

/// The backup names in `backups`, oldest first.
fn backup_names(backups: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(backups)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name.starts_with("session-") && name.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// The millisecond and turn a backup name carries.
fn backup_order(name: &str) -> Option<(u128, u32)> {
    let (stamp, turn) = name
        .strip_prefix("session-")?
        .strip_suffix(".json")?
        .split_once('-')?;
    Some((stamp.parse().ok()?, turn.parse().ok()?))
}

/// The order of the next backup: now, unless that would not sort after the
/// newest one already kept (several in one millisecond, or a clock set back),
/// and then one turn after it.
fn next_backup_order(now: u128, newest: Option<(u128, u32)>) -> (u128, u32) {
    match newest {
        Some((stamp, _)) if stamp < now => (now, 0),
        Some((stamp, turn)) if turn + 1 < BACKUP_TURNS => (stamp, turn + 1),
        Some((stamp, _)) => (stamp + 1, 0),
        None => (now, 0),
    }
}

fn backup_name((stamp, turn): (u128, u32)) -> String {
    format!("session-{stamp:015}-{turn:03}.json")
}

/// Replaces the saved session at `path` with `saved`, durably.
pub(super) fn write(path: &Path, saved: &Saved) -> Result<(), String> {
    let bytes = serde_json::to_vec(saved)
        .map_err(|error| format!("the session could not be written out: {error}"))?;
    if let Some(dir) = path.parent() {
        hide_platform::fs::private::create_dir_all(dir)
            .map_err(|error| format!("{} could not be made: {error}", dir.display()))?;
    }
    hide_platform::fs::atomic::write_file_durable(
        path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{} could not be replaced: {error}", path.display()))
}

#[derive(Default)]
struct Pending {
    changed: bool,
    ending: bool,
}

/// The one thread that saves the session: [`Saver::changed`] wakes it, and
/// dropping the saver writes the last change and ends it.
pub(super) struct Saver {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Saver {
    /// Starts the thread, which writes what `session` reads to `path`;
    /// `session` answers `None` once there is no session left to read.
    pub(super) fn start(
        path: PathBuf,
        session: impl Fn() -> Option<Saved> + Send + 'static,
    ) -> Result<Self, String> {
        let pending: Arc<(Mutex<Pending>, Condvar)> = Arc::default();
        let waiting = Arc::clone(&pending);
        let thread = std::thread::Builder::new()
            .name("own-engine-save".to_owned())
            .spawn(move || save_loop(&path, &waiting, &session))
            .map_err(|error| format!("the session's saving thread did not start: {error}"))?;
        Ok(Self {
            pending,
            thread: Some(thread),
        })
    }

    /// Marks the session changed; the thread writes it once the operation's
    /// changes have landed.
    pub(super) fn changed(&self) {
        let (pending, wake) = &*self.pending;
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .changed = true;
        wake.notify_one();
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        let (pending, wake) = &*self.pending;
        pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ending = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn save_loop(
    path: &Path,
    pending: &(Mutex<Pending>, Condvar),
    session: &dyn Fn() -> Option<Saved>,
) {
    let (state, wake) = pending;
    loop {
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        while !held.changed && !held.ending {
            held = wake.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
        if !held.ending {
            // The operation's other changes land meanwhile; an ending
            // writes at once.
            held = wake
                .wait_timeout_while(held, SETTLE, |held| !held.ending)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let (changed, ending) = (held.changed, held.ending);
        held.changed = false;
        drop(held);
        if changed
            && let Some(saved) = session()
            && let Err(why) = write(path, &saved)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "engine.session_save_failed",
                "path": path.display().to_string(),
                "message": why,
            }));
        }
        if ending {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Saved {
        Saved {
            format: FORMAT,
            workspace_numbers: 1,
            focused_workspace: Some("w1".to_owned()),
            workspaces: vec![SavedWorkspace {
                id: "w1".to_owned(),
                label: "w".to_owned(),
                tokens: BTreeMap::new(),
                worktree: None,
                active_tab: "w1:t1".to_owned(),
                tab_numbers: 1,
                pane_numbers: 1,
                tabs: vec![SavedTab {
                    id: "w1:t1".to_owned(),
                    label: String::new(),
                    focused: "w1:p1".to_owned(),
                    zoomed: false,
                    root: SavedNode::Pane {
                        id: "w1:p1".to_owned(),
                    },
                    panes: vec![SavedPane {
                        id: "w1:p1".to_owned(),
                        cwd: "/w".to_owned(),
                        label: None,
                        tokens: BTreeMap::new(),
                        agent: None,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn a_session_written_reads_back_and_a_newer_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine/session.json");
        let backups = dir.path().join("engine/backups");
        write(&path, &session()).unwrap();
        assert!(matches!(read(&path, &backups), Read::Saved(saved) if saved.workspaces.len() == 1));

        let mut newer: Value = serde_json::to_value(session()).unwrap();
        newer["format"] = 2.into();
        std::fs::write(&path, newer.to_string()).unwrap();
        let why = parse(&newer.to_string()).expect_err("a newer format is refused");
        assert!(why.contains("format 2"), "{why}");
        assert!(matches!(read(&path, &backups), Read::Unreadable));
        assert!(!path.exists(), "the refused file is moved, not left");
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
    }

    #[test]
    fn an_unreadable_session_is_kept_in_the_backups_the_newest_three() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let backups = dir.path().join("backups");
        for turn in 0..5 {
            std::fs::write(&path, format!("not a session {turn}")).unwrap();
            assert!(matches!(read(&path, &backups), Read::Unreadable));
        }
        let mut kept: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        kept.sort();
        assert_eq!(
            kept,
            ["not a session 2", "not a session 3", "not a session 4"]
        );
        assert!(matches!(read(&path, &backups), Read::Missing));
    }

    #[test]
    fn a_backup_always_sorts_after_the_ones_already_kept() {
        assert_eq!(next_backup_order(7, None), (7, 0));
        assert_eq!(next_backup_order(7, Some((5, 4))), (7, 0));
        // Several in one millisecond, and a clock set back.
        assert_eq!(next_backup_order(7, Some((7, 0))), (7, 1));
        assert_eq!(next_backup_order(3, Some((7, 2))), (7, 3));
        assert_eq!(next_backup_order(7, Some((7, 999))), (8, 0));
        let names: Vec<String> = [(7, 2), (7, 3), (7, 999), (8, 0)]
            .into_iter()
            .map(backup_name)
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, names);
        assert_eq!(backup_order(&names[2]), Some((7, 999)));
    }

    #[test]
    fn a_field_this_format_does_not_name_is_refused() {
        let mut value: Value = serde_json::to_value(session()).unwrap();
        value["workspaces"][0]["tabs"][0]["panes"][0]["shell"] = "zsh".into();
        assert!(parse(&value.to_string()).is_err());
    }

    #[test]
    fn a_burst_of_changes_is_written_once_and_at_once_when_the_engine_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let saver = Saver::start(path.clone(), move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(session())
        })
        .unwrap();
        for _ in 0..20 {
            saver.changed();
        }
        // Ending does not wait out the settle: the burst is written once,
        // now.
        drop(saver);
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(parse(&std::fs::read_to_string(&path).unwrap()).is_ok());
    }
}
