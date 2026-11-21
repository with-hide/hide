//! One label generator per engine (PRD labels-in-hided D-10). Every core
//! that labels an engine's agents takes the engine's lock through its own
//! node, so two daemons that open the same engine folder meet at one file
//! beside it: `<engine folder>.hide-label-generator.lock`.
//!
//! A node holds the locks its core's workers took for as long as its core's
//! link lives, each for the worker that took it, so a worker that ends and
//! the one that replaces it never share a lock. It unlocks them explicitly
//! when the link ends, even if a concurrently starting child still holds an
//! inherited descriptor. After a crash, the kernel releases a lock when its
//! last descriptor closes.
//! Windows locks the file's bytes against reading too, so a core standing
//! by there cannot name the holder.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hide_node_link::protocol::LabelLock;
use serde_json::json;

/// The most engines one link's core labels through this node.
const MAX_LOCKS: usize = 64;

/// The lock of the engine whose folder is `socket`.
pub fn lock_path(socket: &Path) -> PathBuf {
    let socket = std::fs::canonicalize(socket).unwrap_or_else(|_| socket.to_path_buf());
    let name = socket
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "herdr.sock".to_owned());
    socket.with_file_name(format!("{name}.hide-label-generator.lock"))
}

/// A lock this node holds, and the core's worker it holds it for.
#[derive(Debug)]
struct Held {
    file: File,
    generator: u64,
}

/// The label locks one core holds through this node, keyed by the socket
/// as the core named it: the lock file's own path depends on whether the
/// socket exists when it is spelled, so it is not a stable key.
#[derive(Debug, Default)]
pub struct LabelLocks {
    state: Mutex<LockState>,
}

#[derive(Debug, Default)]
struct LockState {
    closed: bool,
    held: HashMap<PathBuf, Held>,
}

impl LabelLocks {
    /// Takes the lock of the engine at `socket` for the core's worker
    /// `generator`, or answers which process holds it. Asked again by the
    /// same worker while held, it is held; by another worker of the same
    /// core, this process holds it.
    pub fn take(&self, socket: &Path, generator: u64) -> io::Result<LabelLock> {
        let ours = |lock: &Held| LabelLock {
            held: lock.generator == generator,
            holder: Some(u64::from(std::process::id())),
        };
        {
            let state = self.lock();
            if state.closed {
                return Err(closed());
            }
            let held = &state.held;
            if let Some(lock) = held.get(socket) {
                return Ok(ours(lock));
            }
            if held.len() >= MAX_LOCKS {
                return Err(io::Error::other(format!(
                    "this node already holds {MAX_LOCKS} label locks"
                )));
            }
        }
        // The file is opened and locked outside the table's lock, so a slow
        // filesystem holds only this ask.
        let path = lock_path(socket);
        let Some(file) = try_lock(&path)? else {
            return Ok(LabelLock {
                held: false,
                holder: holder(&path),
            });
        };
        record_holder(&file);
        let mut state = self.lock();
        // Close may have released the table while this ask opened its file.
        // Never retain a late acquisition beyond the link's lifetime.
        if state.closed || state.held.len() >= MAX_LOCKS {
            let error = if state.closed {
                closed()
            } else {
                io::Error::other(format!("this node already holds {MAX_LOCKS} label locks"))
            };
            drop(state);
            unlock(file);
            return Err(error);
        }
        let held = &mut state.held;
        // Another worker of this core took it meanwhile: this lock goes.
        if let Some(lock) = held.get(socket) {
            let answer = ours(lock);
            drop(state);
            unlock(file);
            return Ok(answer);
        }
        held.insert(socket.to_path_buf(), Held { file, generator });
        Ok(LabelLock {
            held: true,
            holder: Some(u64::from(std::process::id())),
        })
    }

    /// Gives the lock of the server at `socket` back, if this node holds it
    /// for `generator`.
    pub fn release(&self, socket: &Path, generator: u64) {
        let mut state = self.lock();
        let held = &mut state.held;
        let lock = held
            .get(socket)
            .is_some_and(|lock| lock.generator == generator)
            .then(|| held.remove(socket))
            .flatten();
        drop(state);
        if let Some(lock) = lock {
            unlock(lock.file);
        }
    }

    /// Ends the link's ownership even while other work retains this table.
    /// Taking the same mutex before and after file acquisition fences late asks.
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        for (_, lock) in state.held.drain() {
            unlock(lock.file);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LockState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for LabelLocks {
    fn drop(&mut self) {
        self.close();
    }
}

fn closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "this node's label-lock owner has closed",
    )
}

/// On Unix, close alone keeps a flock alive while a concurrent fork holds
/// the same open file description, so the lock is released before the
/// descriptor closes.
fn unlock(file: File) {
    if let Err(error) = file.unlock() {
        eprintln!(
            "{}",
            json!({
                "component": "labels",
                "kind": "generator.release_failed",
                "message": error.to_string(),
            })
        );
    }
}

/// The holder writes who it is into the lock, so a core standing by can say
/// which process generates for its server: a stray daemon on the same engine
/// is otherwise invisible.
fn record_holder(file: &File) {
    use std::io::{Seek, Write};
    let mut file = file;
    let holder = json!({"pid": std::process::id()}).to_string();
    let _ = file
        .set_len(0)
        .and_then(|()| file.seek(std::io::SeekFrom::Start(0)))
        .and_then(|_| file.write_all(holder.as_bytes()));
}

/// The pid the holder wrote, and nothing else of the file; `None` when it
/// cannot be read.
fn holder(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut bytes = Vec::new();
    open_private(path, false)
        .and_then(|file| file.take(64).read_to_end(&mut bytes))
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("pid")?
        .as_u64()
}

/// Opens the lock file itself, never what a link at its path names, and
/// only when it is a regular file of this user: the lock can sit in a
/// directory other users write (a socket under `/tmp`), where a planted
/// link would otherwise have its target truncated.
fn open_private(path: &Path, create: bool) -> io::Result<File> {
    hide_platform::fs::private::open_own_file(path, create)
}

fn try_lock(path: &Path) -> io::Result<Option<File>> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let file = open_private(path, true)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A worker asking again after its server's socket appeared, through a
    /// folder reached by a link, still holds its lock: the table is keyed
    /// by the socket as named, not by a path spelled differently once the
    /// socket exists (L1).
    #[cfg(unix)]
    #[test]
    fn a_lock_asked_again_after_its_socket_appears_is_still_held() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let linked = dir.path().join("linked");
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let socket = linked.join("herdr.sock");
        let locks = LabelLocks::default();
        assert!(locks.take(&socket, 7).unwrap().held);
        std::fs::write(real.join("herdr.sock"), b"").unwrap();
        assert!(locks.take(&socket, 7).unwrap().held);
        locks.release(&socket, 7);
        assert!(LabelLocks::default().take(&socket, 8).unwrap().held);
    }

    #[test]
    fn a_link_planted_at_the_lock_path_is_neither_followed_nor_held() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let target = dir.path().join("operator-file.json");
        std::fs::write(&target, r#"{"secret":"kept"}"#).unwrap();
        match hide_platform::fs::link::create_link(&target, &lock_path(&socket)) {
            // A Windows account without the privilege cannot plant one either.
            Err(error) if hide_platform::fs::link::needs_privilege(&error) => return,
            planted => planted.unwrap(),
        }
        assert!(LabelLocks::default().take(&socket, 1).is_err());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            r#"{"secret":"kept"}"#
        );
        assert_eq!(holder(&lock_path(&socket)), None);
    }

    #[test]
    fn a_second_core_for_the_same_server_stands_by_until_the_first_ends() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let first = LabelLocks::default();
        let second = LabelLocks::default();
        assert!(first.take(&socket, 1).unwrap().held);
        assert!(
            first.take(&socket, 1).unwrap().held,
            "asked again, still held"
        );
        let standing = second.take(&socket, 1).unwrap();
        assert!(!standing.held);
        assert_eq!(
            standing.holder,
            Some(u64::from(std::process::id())),
            "the standby can name the process that generates"
        );
        assert!(second.take(&dir.path().join("other.sock"), 1).unwrap().held);
        drop(first);
        assert!(second.take(&socket, 1).unwrap().held);
        second.release(&socket, 1);
        assert!(LabelLocks::default().take(&socket, 1).unwrap().held);
    }

    #[test]
    fn a_worker_that_replaces_another_neither_shares_nor_frees_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let locks = LabelLocks::default();
        assert!(locks.take(&socket, 1).unwrap().held);
        assert!(
            !locks.take(&socket, 2).unwrap().held,
            "the old worker holds it"
        );
        locks.release(&socket, 2);
        assert!(
            !LabelLocks::default().take(&socket, 1).unwrap().held,
            "a release by a worker that holds nothing frees nothing"
        );
        locks.release(&socket, 1);
        assert!(locks.take(&socket, 2).unwrap().held);
        locks.release(&socket, 1);
        assert!(
            !LabelLocks::default().take(&socket, 1).unwrap().held,
            "the old worker's late release leaves its successor's lock"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_ends_releases_its_locks_even_with_an_inherited_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("herdr.sock");
        let first = LabelLocks::default();
        assert!(first.take(&socket, 1).unwrap().held);
        // A fork retains this same open file description until exec closes
        // it. Keep a duplicate to hold that window open deterministically.
        let inherited = first
            .lock()
            .held
            .values()
            .next()
            .unwrap()
            .file
            .try_clone()
            .unwrap();
        let second = LabelLocks::default();
        assert!(!second.take(&socket, 1).unwrap().held);
        first.close();
        assert_eq!(
            first.take(&socket, 1).unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
        assert!(
            second.take(&socket, 1).unwrap().held,
            "the role ends with its owner, even before a forked child execs"
        );
        // Closing the previous owner's inherited descriptor must not give
        // away the role that the second core now holds.
        drop(inherited);
        first.close();
        drop(first);
        assert!(!LabelLocks::default().take(&socket, 1).unwrap().held);
    }
}
