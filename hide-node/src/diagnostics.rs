//! Where a node's diagnostic records go. The process that owns the node
//! installs its log once (`hided` passes the core's), so a record the SSH
//! transport writes lands in the same Logs file as the core's; before that,
//! and in a process that installs none, a record goes to stderr.

use std::sync::OnceLock;

static SINK: OnceLock<fn(serde_json::Value)> = OnceLock::new();

/// Raises this process's open-file limit as far as the system lets it
/// (`hide_platform::process::raise_open_file_limit`) and answers the record
/// of what it got, for the caller to send to the log its role writes; `None`
/// on Windows, which has no such limit. A core or a node holds a connection,
/// a watch and files for each pane, and at the 256 a macOS app starts with
/// the core ran out on 2026-11-20.
pub fn raise_open_files() -> Option<serde_json::Value> {
    Some(match hide_platform::process::raise_open_file_limit() {
        Ok(Some(limit)) => serde_json::json!({
            "component": "daemon",
            "kind": "open_files.limit",
            "before": limit.before,
            "after": limit.after,
        }),
        Ok(None) => return None,
        Err(error) => serde_json::json!({
            "component": "daemon",
            "kind": "open_files.raise_failed",
            "message": error.to_string(),
        }),
    })
}

/// Emits one JSON record.
#[macro_export]
macro_rules! diagnostic {
    ($value:expr) => {
        $crate::diagnostics::emit($value)
    };
}

/// Sends every later record to `sink`. The first install holds for the life
/// of the process; a second one is refused and answers false.
pub fn install(sink: fn(serde_json::Value)) -> bool {
    SINK.set(sink).is_ok()
}

#[cfg(test)]
thread_local! {
    static CAPTURED: std::cell::RefCell<Option<Vec<serde_json::Value>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `body` and returns, beside its result, every record it emitted on
/// this thread; the records still reach the sink.
#[cfg(test)]
pub(crate) fn capture<T>(body: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    CAPTURED.with(|captured| *captured.borrow_mut() = Some(Vec::new()));
    let value = body();
    let records = CAPTURED
        .with(|captured| captured.borrow_mut().take())
        .unwrap_or_default();
    (value, records)
}

pub fn emit(record: serde_json::Value) {
    #[cfg(test)]
    CAPTURED.with(|captured| {
        if let Some(records) = captured.borrow_mut().as_mut() {
            records.push(record.clone());
        }
    });
    match SINK.get() {
        Some(sink) => sink(record),
        None => eprintln!("{record}"),
    }
}
