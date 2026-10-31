//! A file that only ever holds the latest value its owner handed over.
//!
//! The runtime holds the authoritative copy; the file lets a daemon that has
//! just started draw it before it has read the world again. The write runs on
//! a thread of its own, never under `Mutex<Runtime>`: the owner hands over an
//! owned value and the thread serializes and replaces the file. One thread
//! and one pending slot per file, so a burst of values writes the newest and
//! skips the rest, and the thread is joined when the file is dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

pub(crate) struct LatestFile<T: Send + 'static> {
    path: PathBuf,
    thread: &'static str,
    /// Diagnostics are `{component}` records of kind `{subject}.save_failed`
    /// and `{subject}.save_join_failed`.
    component: &'static str,
    subject: &'static str,
    encode: fn(&T) -> Result<Vec<u8>, String>,
    slot: Arc<Mutex<Slot<T>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// The number of the newest value known on disk (`save` numbers them
    /// from 1); a failed write leaves it where it was.
    durable: Arc<AtomicU64>,
}

struct Slot<T> {
    latest: Option<(u64, T)>,
    /// The number the last `save` gave.
    numbered: u64,
    active: bool,
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
            slot: Arc::new(Mutex::new(Slot {
                latest: None,
                numbered: 0,
                active: false,
            })),
            worker: Mutex::new(None),
            durable: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Queues `value` to replace the file and returns its number. A value
    /// still waiting when a newer one arrives is dropped: the newer one
    /// holds it, so its number is reached with the newer one's.
    pub(crate) fn save(&self, value: T) -> u64 {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.numbered += 1;
        let number = slot.numbered;
        slot.latest = Some((number, value));
        if slot.active {
            return number;
        }
        let mut worker = self.worker.lock().unwrap_or_else(PoisonError::into_inner);
        // The previous worker has cleared `active` and is returning.
        if let Some(finished) = worker.take() {
            let _ = finished.join();
        }
        let shared = Arc::clone(&self.slot);
        let durable = Arc::clone(&self.durable);
        let path = self.path.clone();
        let (component, subject, encode) = (self.component, self.subject, self.encode);
        match std::thread::Builder::new()
            .name(self.thread.to_owned())
            .spawn(move || write_latest(&shared, &durable, &path, component, subject, encode))
        {
            Ok(handle) => {
                slot.active = true;
                *worker = Some(handle);
            }
            Err(error) => crate::diagnostic!(serde_json::json!({
                "component": self.component,
                "kind": format!("{}.save_failed", self.subject),
                "message": format!("worker could not start: {error}"),
            })),
        }
        number
    }

    /// The number of the newest value known on disk: every value `save`
    /// numbered up to it is written, in it or in a newer one.
    pub(crate) fn durable(&self) -> u64 {
        self.durable.load(Ordering::Acquire)
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
    slot: &Mutex<Slot<T>>,
    durable: &AtomicU64,
    path: &Path,
    component: &str,
    subject: &str,
    encode: fn(&T) -> Result<Vec<u8>, String>,
) {
    loop {
        let (number, value) = {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            match slot.latest.take() {
                Some(value) => value,
                None => {
                    slot.active = false;
                    return;
                }
            }
        };
        match write(path, &value, encode) {
            Ok(()) => durable.store(number, Ordering::Release),
            Err(reason) => crate::diagnostic!(serde_json::json!({
                "component": component,
                "kind": format!("{subject}.save_failed"),
                "path": path.to_string_lossy(),
                "message": reason,
            })),
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

    fn encode(value: &u32) -> Result<Vec<u8>, String> {
        Ok(value.to_string().into_bytes())
    }

    /// A value's number is known on disk only once a write holding it
    /// succeeded, so a caller waiting on it never acts on a value a stop or
    /// a failed write lost.
    #[test]
    fn a_saves_number_is_on_disk_only_after_a_write_that_held_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = LatestFile::new(
            dir.path().join("value"),
            "test-save",
            "test",
            "value",
            encode,
        );
        let durable = Arc::clone(&file.durable);
        assert_eq!(file.save(1), 1);
        let last = file.save(2);
        assert_eq!(last, 2);
        drop(file);
        assert_eq!(durable.load(Ordering::Acquire), last);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("value")).unwrap(),
            "2"
        );

        // A folder where the file's own folder should be fails every write.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let file = LatestFile::new(blocked.join("value"), "test-save", "test", "value", encode);
        let durable = Arc::clone(&file.durable);
        assert_eq!(file.save(3), 1);
        drop(file);
        assert_eq!(durable.load(Ordering::Acquire), 0);
    }
}
