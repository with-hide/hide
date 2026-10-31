//! The holder's diagnostic log: one JSON line per event, with the pane it
//! holds as the subject, in a file its starter names. The holder has no
//! standard streams, so a holder started without a file logs nothing, and a
//! file it cannot open fails its start.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

/// How large the file grows before it is set aside as `<file>.1`, replacing
/// the one before, so a holder that lives for weeks keeps two files at most.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct Log(Option<Arc<Mutex<Sink>>>);

struct Sink {
    path: PathBuf,
    file: std::fs::File,
    written: u64,
    pane: String,
    /// Lines that could not be written since the last one that was; the
    /// next line written says how many.
    dropped: u64,
}

impl Log {
    /// The log at `path`, or none when no path is given. A file that cannot
    /// be opened is an error, which fails the holder's start.
    pub fn open(path: Option<&Path>, pane: &str) -> Result<Self, String> {
        let Some(path) = path else {
            return Ok(Self(None));
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("the log {} could not be opened: {error}", path.display()))?;
        let written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self(Some(Arc::new(Mutex::new(Sink {
            path: path.to_owned(),
            file,
            written,
            pane: pane.to_owned(),
            dropped: 0,
        })))))
    }

    /// Writes `event` with `fields`. A line that cannot be written is
    /// counted, and the next line written is preceded by
    /// `holder.log_lines_dropped` with the count.
    pub fn event(&self, event: &str, fields: &[(&str, serde_json::Value)]) {
        let Some(sink) = &self.0 else { return };
        let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
        sink.write(event, fields);
    }

    /// As [`Log::event`], for a panic hook: it never waits for the log, since
    /// the thread that panicked may hold it, and then writes nothing.
    pub fn event_now(&self, event: &str, fields: &[(&str, serde_json::Value)]) {
        let Some(sink) = &self.0 else { return };
        let mut sink = match sink.try_lock() {
            Ok(sink) => sink,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        sink.write(event, fields);
    }
}

impl Sink {
    fn line(&self, event: &str, fields: &[(&str, serde_json::Value)]) -> String {
        let mut line = serde_json::Map::new();
        line.insert("event".to_owned(), event.into());
        line.insert("pane".to_owned(), self.pane.clone().into());
        line.insert("holder".to_owned(), std::process::id().into());
        line.insert(
            "at_unix_ms".to_owned(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis() as u64)
                .unwrap_or(0)
                .into(),
        );
        for (key, value) in fields {
            line.insert((*key).to_owned(), value.clone());
        }
        let mut text = serde_json::Value::Object(line).to_string();
        text.push('\n');
        text
    }

    fn write(&mut self, event: &str, fields: &[(&str, serde_json::Value)]) {
        let mut text = String::new();
        if self.dropped > 0 {
            text = self.line(
                "holder.log_lines_dropped",
                &[("count", self.dropped.into())],
            );
        }
        text.push_str(&self.line(event, fields));
        if self.written + text.len() as u64 > MAX_LOG_BYTES {
            let aside = self.path.with_extension("log.1");
            let reopened = std::fs::rename(&self.path, &aside).and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
            });
            match reopened {
                Ok(file) => {
                    self.file = file;
                    self.written = 0;
                }
                // Past its bound with nowhere to set it aside: the line is
                // left out rather than the file grown.
                Err(_) => {
                    self.dropped += 1;
                    return;
                }
            }
        }
        if self.file.write_all(text.as_bytes()).is_ok() {
            self.written += text.len() as u64;
            self.dropped = 0;
        } else {
            self.dropped += 1;
        }
    }
}
