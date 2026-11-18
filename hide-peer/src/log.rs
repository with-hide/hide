//! Where this crate's diagnostic records go.
//!
//! `hide-peer` does not link `hide-core`, so the daemon hands in its
//! diagnostic log once at start ([`set_sink`]). Until it does, a record is
//! written to standard error rather than lost.

use std::sync::OnceLock;

use serde_json::Value;

static SINK: OnceLock<fn(Value)> = OnceLock::new();

/// Sends every later record to `sink`. The first call wins; a process has
/// one diagnostic log.
pub fn set_sink(sink: fn(Value)) {
    let _ = SINK.set(sink);
}

pub fn emit(record: Value) {
    match SINK.get() {
        Some(sink) => sink(record),
        None => eprintln!("{record}"),
    }
}

/// Records one structured event:
/// `diagnostic!(json!({"component": ..., "event": ...}))`.
#[macro_export]
macro_rules! diagnostic {
    ($record:expr) => {
        $crate::log::emit($record)
    };
}
