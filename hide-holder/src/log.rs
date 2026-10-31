//! The holder's diagnostic log: one JSON line per event, with the pane it
//! holds as the subject, in a file its starter names. The holder has no
//! standard streams, so a holder started without a file logs nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

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
}

impl Log {
    pub fn open(path: Option<&Path>, pane: &str) -> Self {
        let Some(path) = path else {
            return Self(None);
        };
        let opened = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path);
        let Ok(file) = opened else {
            return Self(None);
        };
        let written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Self(Some(Arc::new(Mutex::new(Sink {
            path: path.to_owned(),
            file,
            written,
            pane: pane.to_owned(),
        }))))
    }

    /// Writes `event` with `fields`; a write that fails is dropped, since
    /// there is nowhere else to say it.
    pub fn event(&self, event: &str, fields: &[(&str, serde_json::Value)]) {
        let Some(sink) = &self.0 else { return };
        let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
        let mut line = serde_json::Map::new();
        line.insert("event".to_owned(), event.into());
        line.insert("pane".to_owned(), sink.pane.clone().into());
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
        if sink.written + text.len() as u64 > MAX_LOG_BYTES {
            let aside = sink.path.with_extension("log.1");
            let reopened = std::fs::rename(&sink.path, &aside).and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&sink.path)
            });
            if let Ok(file) = reopened {
                sink.file = file;
                sink.written = 0;
            }
        }
        if sink.file.write_all(text.as_bytes()).is_ok() {
            sink.written += text.len() as u64;
        }
    }
}
