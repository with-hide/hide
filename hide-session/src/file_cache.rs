//! Facts a Claude or Codex session file yields from its own bytes, kept
//! while the file's stamp holds.
//!
//! The catalog lists every session on each call, and each project's read
//! and each memory pass make that call, so an unchanged transcript would be
//! opened and parsed again every time. A file whose stamp moved is read on
//! from where its last read stopped (`catalog::Transcript`); a file whose
//! stamp cannot be read is read whole and nothing is kept. Entries whose file
//! left the listing of their home are dropped, so a home holds no more files
//! than that listing's own `SESSION_DISCOVERY_LIMIT` cap admits.

use crate::Agent;
use crate::catalog::{ProjectSession, SessionAvailability, Transcript};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

type Kept<T> = Mutex<HashMap<PathBuf, (String, T)>>;

static HEADERS: OnceLock<Kept<Option<PathBuf>>> = OnceLock::new();
static SESSIONS: OnceLock<Kept<Listed>> = OnceLock::new();

/// A transcript's listed facts and where its read stopped; `None` for a file
/// the listing answers without reading it (one past the size cap).
pub(crate) type Listed = (ProjectSession, Option<Transcript>);

/// Whether a session's facts come from its own file alone, so its stamp
/// says whether they are current. Grok, Pi, omp, Antigravity and Cursor
/// also read other files or prove the file's owner, and are read each time.
pub(crate) fn keeps(agent: Agent) -> bool {
    matches!(agent, Agent::Claude | Agent::Codex)
}

/// The working directory a file's header names, kept while `path` holds
/// `stamp`. Only a header that named a directory is kept: `None` cannot tell
/// a file without one from a file that could not be opened, and the latter
/// must be read again once it can be.
pub(crate) fn header_cwd(
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    fetch(&HEADERS, path, stamp, |_| read(), |cwd| cwd.is_some())
}

/// The listed facts of a session file, kept while `path` holds `stamp`.
/// When the stamp moved, `read` is handed what the file's last read kept, so
/// it reads on from there. Only facts derived from bytes that were read are
/// kept: an `Unavailable` answer from a failed open, read or metadata call is
/// read again, from the start, on the next listing.
pub(crate) fn session(
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce(Option<Listed>) -> Listed,
) -> ProjectSession {
    fetch(&SESSIONS, path, stamp, read, |(session, _)| {
        settled(session)
    })
    .0
}

/// Whether a session's answer follows from its bytes alone: readable, or a
/// file whose size or content the reader refused by rule.
fn settled(session: &ProjectSession) -> bool {
    match &session.availability {
        SessionAvailability::Available => true,
        SessionAvailability::Unavailable { reason } => {
            reason == "session_too_large" || reason == "session_malformed"
        }
    }
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

/// The value kept for `path` while it holds `stamp`; otherwise `read`'s
/// answer, which is handed the value kept for an earlier stamp.
fn fetch<T: Clone>(
    kept: &OnceLock<Kept<T>>,
    path: &Path,
    stamp: Option<String>,
    read: impl FnOnce(Option<T>) -> T,
    keep: impl FnOnce(&T) -> bool,
) -> T {
    let Some(stamp) = stamp else {
        return read(None);
    };
    let kept = kept.get_or_init(|| Mutex::new(HashMap::new()));
    let earlier = match lock(kept).get(path) {
        Some((held, value)) if *held == stamp => return value.clone(),
        Some((_, value)) => Some(value.clone()),
        None => None,
    };
    let value = read(earlier);
    if keep(&value) {
        lock(kept).insert(path.to_path_buf(), (stamp, value.clone()));
    } else {
        lock(kept).remove(path);
    }
    value
}

/// The kept header and session counts under `home`, for the tests that
/// check the listing's pruning.
#[cfg(test)]
pub(crate) fn kept(home: &Path) -> (usize, usize) {
    (count_under(&HEADERS, home), count_under(&SESSIONS, home))
}

#[cfg(test)]
fn count_under<T>(kept: &OnceLock<Kept<T>>, home: &Path) -> usize {
    kept.get().map_or(0, |kept| {
        lock(kept)
            .keys()
            .filter(|path| path.starts_with(home))
            .count()
    })
}

/// The map's values are plain data written whole, so a poisoned lock holds
/// nothing a reader could misread; it is taken as it stands.
fn lock<T>(kept: &Kept<T>) -> MutexGuard<'_, HashMap<PathBuf, (String, T)>> {
    kept.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
