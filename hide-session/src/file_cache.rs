//! Facts a Claude or Codex session file yields from its own bytes, kept
//! while the file's stamp holds.
//!
//! The catalog lists every session on each call, and each project's read
//! and each memory pass make that call, so an unchanged transcript would be
//! opened and parsed again every time. A file whose stamp moved, or whose
//! stamp cannot be read, is read in full as before. Entries whose file left
//! the listing of their home are dropped, so a home holds no more files than
//! that listing's own `SESSION_DISCOVERY_LIMIT` cap admits.

use crate::Agent;
use crate::catalog::ProjectSession;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

type Kept<T> = Mutex<HashMap<PathBuf, (String, T)>>;

static HEADERS: OnceLock<Kept<Option<PathBuf>>> = OnceLock::new();
static SESSIONS: OnceLock<Kept<Option<ProjectSession>>> = OnceLock::new();

/// Whether a session's facts come from its own file alone, so its stamp
/// says whether they are current. Grok, Pi, omp, Antigravity and Cursor
/// also read other files or prove the file's owner, and are read each time.
pub(crate) fn keeps(agent: Agent) -> bool {
    matches!(agent, Agent::Claude | Agent::Codex)
}

/// The working directory a file's header names, kept while `path` holds
/// `stamp`. Without a stamp the header is read and kept nowhere.
pub(crate) fn header_cwd(
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    fetch(&HEADERS, path, stamp, read)
}

/// The listed facts of a session file, kept while `path` holds `stamp`.
pub(crate) fn session(
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce() -> Option<ProjectSession>,
) -> Option<ProjectSession> {
    fetch(&SESSIONS, path, stamp, read)
}

/// Drops the kept files under `home` that the listing of `home` no longer
/// names. Other homes' entries stay, so two listings of different homes do
/// not evict each other's facts.
pub(crate) fn retain(home: &Path, listed: &HashSet<PathBuf>) {
    let gone = |path: &PathBuf| path.starts_with(home) && !listed.contains(path);
    if let Some(kept) = HEADERS.get() {
        lock(kept).retain(|path, _| !gone(path));
    }
    if let Some(kept) = SESSIONS.get() {
        lock(kept).retain(|path, _| !gone(path));
    }
}

fn fetch<T: Clone>(
    kept: &OnceLock<Kept<T>>,
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce() -> T,
) -> T {
    let Some(stamp) = stamp else {
        return read();
    };
    let kept = kept.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((held, value)) = lock(kept).get(path)
        && *held == stamp
    {
        return value.clone();
    }
    let value = read();
    lock(kept).insert(path.to_path_buf(), (stamp, value.clone()));
    value
}

/// The map's values are plain data written whole, so a poisoned lock holds
/// nothing a reader could misread; it is taken as it stands.
fn lock<T>(kept: &Kept<T>) -> MutexGuard<'_, HashMap<PathBuf, (String, T)>> {
    kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
