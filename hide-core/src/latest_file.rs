//! A file that only ever holds the latest value its owner handed over.
//!
//! The runtime holds the authoritative copy; the file lets a daemon that has
//! just started draw it before it has read the world again. The write runs on
//! a thread of its own, never under `Mutex<Runtime>`: the owner hands over an
//! owned value and the thread serializes and replaces the file. One thread
//! and one pending slot per file, so a burst of values writes the newest and
//! skips the rest, and the thread is joined when the file is dropped.
//!
//! Each save has a ticket, and an owner whose next step must not happen
//! before a value is on disk waits for its ticket off the lock
//! ([`Written::wait`]): the write that answers it may carry a newer value.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

pub(crate) struct LatestFile<T: Send + 'static> {
    path: PathBuf,
    thread: &'static str,
    /// Diagnostics are `{component}` records of kind `{subject}.save_failed`
    /// and `{subject}.save_join_failed`.
    component: &'static str,
    subject: &'static str,
    encode: fn(&T) -> Result<Vec<u8>, String>,
    slot: Arc<Shared<T>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

struct Shared<T> {
    slot: Mutex<Slot<T>>,
    /// Rung each time a write ends, for [`Written::wait`].
    ended: Condvar,
}

struct Slot<T> {
    latest: Option<T>,
    active: bool,
    /// The ticket of the newest value handed over, which `latest` holds
    /// while it waits.
    handed: u64,
    /// The newest ticket whose value, or a newer one, reached the disk.
    written: u64,
    /// The newest ticket whose write failed, so its waiters are told.
    failed: u64,
}

/// A wait for one save to reach the disk, held off the owner's lock.
pub(crate) struct Written {
    shared: Arc<dyn Ticketed + Send + Sync>,
    ticket: u64,
}

trait Ticketed {
    fn wait(&self, ticket: u64, limit: Duration) -> Result<(), &'static str>;
}

impl<T: Send> Ticketed for Shared<T> {
    fn wait(&self, ticket: u64, limit: Duration) -> Result<(), &'static str> {
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        let (slot, waited) = self
            .ended
            .wait_timeout_while(slot, limit, |slot| {
                slot.written < ticket && slot.failed < ticket
            })
            .unwrap_or_else(PoisonError::into_inner);
        if slot.written >= ticket {
            Ok(())
        } else if waited.timed_out() {
            Err("save_timeout")
        } else {
            Err("save_failed")
        }
    }
}

impl Written {
    /// Blocks until the value this ticket stands for, or a newer one, is on
    /// disk; refused when its write failed or took past `limit`. Never under
    /// the owner's lock.
    pub(crate) fn wait(&self, limit: Duration) -> Result<(), &'static str> {
        self.shared.wait(self.ticket, limit)
    }
}

impl<T: Send + 'static> LatestFile<T> {
    pub(crate) fn new(
        path: PathBuf,
        thread: &'static str,
        component: &'static str,
        subject: &'static str,
        encode: fn(&T) -> Result<Vec<u8>, String>,
    ) -> Self {
        Self {
            path,
            thread,
            component,
            subject,
            encode,
            slot: Arc::new(Shared {
                slot: Mutex::new(Slot {
                    latest: None,
                    active: false,
                    handed: 0,
                    written: 0,
                    failed: 0,
                }),
                ended: Condvar::new(),
            }),
            worker: Mutex::new(None),
        }
    }

    /// Queues `value` to replace the file. A value still waiting when a newer
    /// one arrives is dropped.
    pub(crate) fn save(&self, value: T) {
        self.hand(value);
    }

    /// [`Self::save`], with the wait for that value to reach the disk.
    pub(crate) fn save_written(&self, value: T) -> Written {
        let ticket = self.hand(value);
        Written {
            shared: Arc::clone(&self.slot) as Arc<dyn Ticketed + Send + Sync>,
            ticket,
        }
    }

    fn hand(&self, value: T) -> u64 {
        let mut slot = self
            .slot
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.handed += 1;
        let ticket = slot.handed;
        slot.latest = Some(value);
        if slot.active {
            return ticket;
        }
        let mut worker = self.worker.lock().unwrap_or_else(PoisonError::into_inner);
        // The previous worker has cleared `active` and is returning.
        if let Some(finished) = worker.take() {
            let _ = finished.join();
        }
        let shared = Arc::clone(&self.slot);
        let path = self.path.clone();
        let (component, subject, encode) = (self.component, self.subject, self.encode);
        match std::thread::Builder::new()
            .name(self.thread.to_owned())
            .spawn(move || write_latest(&shared, &path, component, subject, encode))
        {
            Ok(handle) => {
                slot.active = true;
                *worker = Some(handle);
            }
            Err(error) => {
                // Nothing will write it: its waiters are told now.
                slot.failed = ticket;
                self.slot.ended.notify_all();
                crate::diagnostic!(serde_json::json!({
                    "component": self.component,
                    "kind": format!("{}.save_failed", self.subject),
                    "message": format!("worker could not start: {error}"),
                }));
            }
        }
        ticket
    }
}

impl<T: Send + 'static> Drop for LatestFile<T> {
    /// A save queued just before quitting reaches the disk first.
    fn drop(&mut self) {
        let worker = self
            .worker
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            crate::diagnostic!(serde_json::json!({
                "component": self.component,
                "kind": format!("{}.save_join_failed", self.subject),
            }));
        }
    }
}

fn write_latest<T>(
    shared: &Shared<T>,
    path: &Path,
    component: &str,
    subject: &str,
    encode: fn(&T) -> Result<Vec<u8>, String>,
) {
    loop {
        let (value, ticket) = {
            let mut slot = shared.slot.lock().unwrap_or_else(PoisonError::into_inner);
            match slot.latest.take() {
                Some(value) => (value, slot.handed),
                None => {
                    slot.active = false;
                    return;
                }
            }
        };
        let written = write(path, &value, encode);
        {
            let mut slot = shared.slot.lock().unwrap_or_else(PoisonError::into_inner);
            match &written {
                Ok(()) => slot.written = slot.written.max(ticket),
                Err(_) => slot.failed = slot.failed.max(ticket),
            }
        }
        shared.ended.notify_all();
        if let Err(reason) = written {
            crate::diagnostic!(serde_json::json!({
                "component": component,
                "kind": format!("{subject}.save_failed"),
                "path": path.to_string_lossy(),
                "message": reason,
            }));
        }
    }
}

fn write<T>(
    path: &Path,
    value: &T,
    encode: fn(&T) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let bytes = encode(value)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        hide_platform::fs::private::create_dir_all(parent)
            .map_err(|error| format!("folder could not be prepared: {error}"))?;
    }
    hide_platform::fs::atomic::write_file(path, &bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| format!("could not be replaced: {error}"))
}

/// Writes `bytes` to `path` unless a file is there already, which is the
/// earliest copy and is kept: a store's copy of the file a schema conversion
/// read, which a build from before can be put back on. Whether it wrote.
pub(crate) fn keep_first_copy(path: &Path, bytes: &[u8]) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }
    hide_platform::fs::atomic::write_file(path, bytes, hide_platform::fs::Access::Private)
        .map(|_| true)
        .map_err(|error| format!("could not be written: {error}"))
}

/// Moves a store's file its loader could not read beside it, named with
/// `now_unix_ms`, so the next save starts a new file and the unreadable one
/// is kept for a person rather than written over. Where it went.
pub(crate) fn set_aside(path: &Path, now_unix_ms: u64) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("has no file name")?;
    let aside = path.with_file_name(format!("{name}.unreadable-{now_unix_ms}"));
    std::fs::rename(path, &aside).map_err(|error| format!("could not be set aside: {error}"))?;
    Ok(aside)
}

/// The file's bytes, or `None` when there is no file; a file past `cap` is
/// refused rather than read.
pub(crate) fn read_capped(path: &Path, cap: u64) -> Result<Option<Vec<u8>>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not be opened: {error}")),
    };
    let length = file
        .metadata()
        .map_err(|error| format!("could not be measured: {error}"))?
        .len();
    if length > cap {
        return Err(format!("is {length} bytes, past the {cap} byte cap"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    std::io::Read::read_to_end(&mut std::io::Read::take(file, cap), &mut bytes)
        .map_err(|error| format!("could not be read: {error}"))?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: &String) -> Result<Vec<u8>, String> {
        Ok(value.as_bytes().to_vec())
    }

    #[test]
    fn a_saved_value_is_on_disk_once_its_wait_returns() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("value.txt");
        let file = LatestFile::new(path.clone(), "test-save", "test", "test", encode);
        file.save("first".to_owned());
        let written = file.save_written("second".to_owned());
        assert_eq!(written.wait(Duration::from_secs(10)), Ok(()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn a_wait_whose_write_failed_is_refused_at_once() {
        let root = tempfile::tempdir().unwrap();
        // A file where the folder would be: nothing can be written under it.
        let blocked = root.path().join("blocked");
        std::fs::write(&blocked, b"").unwrap();
        let file = LatestFile::new(
            blocked.join("value.txt"),
            "test-save",
            "test",
            "test",
            encode,
        );
        let written = file.save_written("lost".to_owned());
        assert_eq!(written.wait(Duration::from_secs(10)), Err("save_failed"));
    }
}
