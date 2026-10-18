//! One bounded, nonblocking queue keeps diagnostic I/O out of the runtime lock.
//! The stderr line and the on-disk line are identical. Files live next to the
//! app's state in Logs, so a suffixed development instance owns separate logs.
//! A process drains the queue before it exits ([`drain`]), within a bound, so
//! the records it wrote last are on disk; a panic out of `main` and a SIGKILL
//! are not covered, and what was queued then is lost.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const FILE_LIMIT: u64 = 1024 * 1024;
const LINE_LIMIT: usize = 64 * 1024;
const QUEUE: usize = 512;
/// How long [`drain`] waits for the writers before the process exits anyway,
/// so a stalled disk cannot hold the exit.
const DRAIN_WITHIN: Duration = Duration::from_secs(1);
static SINK: OnceLock<Mutex<Option<DiagnosticSink>>> = OnceLock::new();
/// The writers this process started that may still be writing: the current
/// sink's and those an `install` replaced, which end once they have written
/// what they hold.
static WRITERS: OnceLock<Mutex<Vec<WriterEnd>>> = OnceLock::new();

#[derive(Clone)]
struct DiagnosticSink {
    sender: SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

/// Disconnects when its writer thread has written its queue and returned,
/// or unwound.
struct WriterEnd(mpsc::Receiver<()>);

impl WriterEnd {
    fn running(&self) -> bool {
        matches!(self.0.try_recv(), Err(mpsc::TryRecvError::Empty))
    }

    /// Whether the writer ended by `deadline`.
    fn ended_by(&self, deadline: Instant) -> bool {
        matches!(
            self.0
                .recv_timeout(deadline.saturating_duration_since(Instant::now())),
            Err(mpsc::RecvTimeoutError::Disconnected)
        )
    }
}

/// Emits one JSON record. The argument is a `serde_json::Value`, so a line
/// that is not JSON cannot be written; the sink never has to parse its input.
#[macro_export]
macro_rules! diagnostic {
    ($value:expr) => {
        $crate::diagnostics::emit($value)
    };
}

/// Sends every record to the Logs file beside `state_path`: the core's, or
/// a node-role daemon's, which has no core to install it.
#[cfg(not(test))]
pub fn install(state_path: &Path) -> io::Result<()> {
    let directory = state_path
        .parent()
        .ok_or_else(|| io::Error::other("state path has no parent"))?
        .join("Logs");
    let (sink, end) = start_writer(directory)?;
    {
        let mut writers = lock(writer_ends());
        writers.retain(WriterEnd::running);
        writers.push(end);
    }
    // A process normally owns one core. Replacing the active sink also makes
    // destroy-and-create cycles bind diagnostics to the new core's state path
    // instead of keeping the first path for the rest of the process lifetime.
    // The sink replaced here is dropped, so its writer ends once it has
    // written what it holds.
    *lock(sink_slot()) = Some(sink);
    Ok(())
}

/// Ends every writer this process started, once each has written what it
/// holds, within [`DRAIN_WITHIN`]; answers whether all ended. Called once,
/// just before the process exits: a record after it goes to stderr at once,
/// as before [`install`]. A writer still running at the bound is named by
/// one `diagnostics.drain_timed_out` line on stderr, and the exit goes on.
pub fn drain() -> bool {
    let sink = lock(sink_slot()).take();
    let writers = std::mem::take(&mut *lock(writer_ends()));
    let running = end_writers(sink, &writers, DRAIN_WITHIN);
    if running > 0 {
        write_stderr_line(
            &serde_json::json!({
                "kind": "diagnostics.drain_timed_out",
                "writers": running,
                "within_ms": DRAIN_WITHIN.as_millis() as u64,
            })
            .to_string(),
        );
    }
    running == 0
}

/// Drops `sink` and waits for `writers` until one deadline `within` from
/// now; answers how many were still running at it.
fn end_writers(sink: Option<DiagnosticSink>, writers: &[WriterEnd], within: Duration) -> usize {
    drop(sink);
    let deadline = Instant::now() + within;
    writers
        .iter()
        .filter(|writer| !writer.ended_by(deadline))
        .count()
}

/// A writer thread appending each queued record to `directory`'s log, and
/// the end it reports once its queue's last sender is dropped and it has
/// written what the queue held.
fn start_writer(directory: PathBuf) -> io::Result<(DiagnosticSink, WriterEnd)> {
    let writer = RotatingLog::open(directory, FILE_LIMIT)?;
    let (sender, receiver) = mpsc::sync_channel::<String>(QUEUE);
    let (ended, end) = mpsc::channel::<()>();
    let dropped = Arc::new(AtomicU64::new(0));
    let writer_dropped = Arc::clone(&dropped);
    std::thread::Builder::new()
        .name("diagnostics".into())
        .spawn(move || {
            // Dropped when this thread returns or unwinds, which is the end
            // `drain` waits for.
            let _ended = ended;
            let mut writer = writer;
            let mut write_failed = false;
            for message in receiver {
                let dropped = writer_dropped.swap(0, Ordering::Relaxed);
                let overflow = (dropped > 0).then(|| {
                    serde_json::json!({
                        "kind": "diagnostics.queue_overflow", "dropped": dropped
                    })
                    .to_string()
                });
                for raw in overflow.iter().chain(std::iter::once(&message)) {
                    let line = if raw.len() > LINE_LIMIT {
                        serde_json::json!({"kind":"diagnostics.line_too_large", "bytes":raw.len()})
                            .to_string()
                    } else {
                        raw.clone()
                    };
                    write_stderr_line(&line);
                    match writer.append(&line) {
                        Ok(()) => write_failed = false,
                        Err(error) if !write_failed => {
                            write_stderr_line(
                                &serde_json::json!({
                                    "kind": "diagnostics.write_failed", "message": error.to_string()
                                })
                                .to_string(),
                            );
                            write_failed = true;
                        }
                        Err(_) => {}
                    }
                }
            }
        })?;
    Ok((DiagnosticSink { sender, dropped }, WriterEnd(end)))
}

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Option<Vec<serde_json::Value>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `body` and returns, beside its result, every record it emitted on
/// this thread, so a test can assert a reported shortfall; the records still
/// reach stderr.
#[cfg(test)]
pub(crate) fn capture<T>(body: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    CAPTURED.with(|captured| *captured.borrow_mut() = Some(Vec::new()));
    let value = body();
    let records = CAPTURED
        .with(|captured| captured.borrow_mut().take())
        .unwrap_or_default();
    (value, records)
}

/// Public so the daemon that owns this core can record its own events in the
/// same Logs file; a detached daemon's stderr goes nowhere.
pub fn emit(record: serde_json::Value) {
    #[cfg(test)]
    CAPTURED.with(|captured| {
        if let Some(records) = captured.borrow_mut().as_mut() {
            records.push(record.clone());
        }
    });
    let message = record.to_string();
    let sink = lock(sink_slot()).clone();
    if let Some(sink) = sink {
        if sink.sender.try_send(message).is_err() {
            sink.dropped.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        write_stderr_line(&message);
    }
}

/// Writes one record to the Logs file beside `state_path` at once, before
/// any core exists: a daemon that refuses to start leaves its reason where
/// its operator reads, since a detached daemon's stderr goes nowhere, and a
/// record written this way is on disk even when [`drain`] ran out of time.
pub fn record_now(state_path: &Path, record: serde_json::Value) -> io::Result<()> {
    let line = record.to_string();
    write_stderr_line(&line);
    let directory = state_path
        .parent()
        .ok_or_else(|| io::Error::other("state path has no parent"))?
        .join("Logs");
    RotatingLog::open(directory, FILE_LIMIT)?.append(&line)
}

/// The newest record in the Logs file beside `state_path` that `wanted`
/// accepts, so a process that started another can read why it stopped.
pub fn newest_record(
    state_path: &Path,
    wanted: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    let text = fs::read_to_string(state_path.parent()?.join("Logs").join("core.jsonl")).ok()?;
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .find(|record| wanted(record))
}

fn sink_slot() -> &'static Mutex<Option<DiagnosticSink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

fn writer_ends() -> &'static Mutex<Vec<WriterEnd>> {
    WRITERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_stderr_line(message: &str) {
    // Attach children inherit stderr, so build each complete JSONL record before
    // taking the process-local lock and writing it to the shared descriptor.
    let mut line = String::with_capacity(message.len() + 1);
    line.push_str(message);
    line.push('\n');
    let _ = io::stderr().lock().write_all(line.as_bytes());
}

struct RotatingLog {
    directory: PathBuf,
    file: File,
    size: u64,
    limit: u64,
}

impl RotatingLog {
    fn open(directory: PathBuf, limit: u64) -> io::Result<Self> {
        hide_platform::fs::private::create_dir_all(&directory)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("core.jsonl"))?;
        let size = file.metadata()?.len();
        Ok(Self {
            directory,
            file,
            size,
            limit,
        })
    }

    fn append(&mut self, line: &str) -> io::Result<()> {
        if self.size + line.len() as u64 + 1 > self.limit {
            fs::rename(
                self.directory.join("core.jsonl"),
                self.directory.join("core.previous.jsonl"),
            )?;
            self.file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(self.directory.join("core.jsonl"))?;
            self.size = 0;
        }
        writeln!(self.file, "{line}")?;
        self.size += line.len() as u64 + 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_rotate_without_splitting_json_records() {
        let folder = tempfile::tempdir().unwrap();
        let directory = folder.path().to_path_buf();
        let mut log = RotatingLog::open(directory.clone(), 256).unwrap();
        for n in 0..100 {
            log.append(&serde_json::json!({"kind":"test", "n":n}).to_string())
                .unwrap();
        }
        for name in ["core.jsonl", "core.previous.jsonl"] {
            let bytes = fs::read_to_string(directory.join(name)).unwrap();
            assert!(bytes.len() <= 256);
            for line in bytes.lines() {
                assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
            }
        }
        assert!(
            fs::read_to_string(directory.join("core.jsonl"))
                .unwrap()
                .contains("99")
        );
    }

    #[test]
    fn a_drain_returns_once_every_writer_wrote_all_it_held() {
        let folders = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        // The first sink is replaced, as a second `install` replaces it; the
        // second is the current one. Each holds a full queue.
        let (replaced, replaced_end) = start_writer(folders[0].path().to_path_buf()).unwrap();
        let (current, current_end) = start_writer(folders[1].path().to_path_buf()).unwrap();
        for sink in [&replaced, &current] {
            for n in 0..QUEUE {
                sink.sender
                    .try_send(serde_json::json!({"kind": "test", "n": n}).to_string())
                    .unwrap();
            }
        }
        assert!(replaced_end.running() && current_end.running());
        drop(replaced);
        let writers = [replaced_end, current_end];
        // Only a hang guard: the drain answers once both writers ended.
        assert_eq!(
            end_writers(Some(current), &writers, Duration::from_secs(10)),
            0
        );
        assert!(writers.iter().all(|writer| !writer.running()));
        for folder in &folders {
            let written: Vec<u64> = fs::read_to_string(folder.path().join("core.jsonl"))
                .unwrap()
                .lines()
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).unwrap()["n"]
                        .as_u64()
                        .unwrap()
                })
                .collect();
            assert_eq!(written, (0..QUEUE as u64).collect::<Vec<_>>());
        }
    }
}
