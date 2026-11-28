//! A test fixture's daemons end with the test that owns the fixture
//! (with-hide/hide#3119). A core update under a fixture HOME starts its core
//! as a detached process (`core_install::starter`), which no process of the
//! test holds, so a test that ends without unwinding (a runner's timeout, a
//! cancelled job) left it running. The test holds its fixture HOME's owner
//! file ([`crate::env::FIXTURE_OWNER_FILE`]) locked for its whole life, and
//! the system lets that lock go with the test's process however it ends, on
//! every system; every `hided` under that HOME waits for the lock and ends
//! its own tree once it comes free. A HOME with no owner file is left to its
//! test's own cleanup, as before.

use std::fs::File;
use std::io;
use std::path::Path;

use hide_platform::fs::lock::{Mode, lock_file_blocking};
use serde_json::json;

/// Starts the wait on the owner file when this process runs under a fixture
/// HOME that has one; an owner file that cannot be opened is an error, since
/// the process would otherwise outlive its test.
pub fn watch() -> Result<(), String> {
    let Some(path) = crate::env::fixture_owner_file() else {
        return Ok(());
    };
    let file = File::open(&path)
        .map_err(|error| format!("the fixture's owner file {}: {error}", path.display()))?;
    std::thread::Builder::new()
        .name("fixture-owner".into())
        .spawn(move || end_with_owner(&path, file))
        .map(drop)
        .map_err(|error| format!("the fixture owner's watch did not start: {error}"))
}

/// Waits until the owner's lock comes free, then ends this process and what
/// it started, as an owner watch does (`hide_platform::process::OwnerWatch`).
fn end_with_owner(path: &Path, file: File) {
    let held = loop {
        match file
            .try_clone()
            .and_then(|file| lock_file_blocking(file, Mode::Shared))
        {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            held => break held,
        }
    };
    // A lock that cannot be taken proves no owner either: the process ends
    // rather than outlive a test it can no longer see.
    hide_core::diagnostic!(json!({
        "component": "fixture_owner",
        "kind": "fixture_owner.gone",
        "file": path.display().to_string(),
        "error": held.err().map(|error| error.to_string()),
    }));
    hide_core::diagnostics::drain();
    let ended = hide_platform::process::kill_tree(std::process::id());
    // Reached only when the kill was refused.
    hide_core::diagnostic!(json!({
        "component": "fixture_owner",
        "kind": "fixture_owner.kill_failed",
        "error": ended.err().map(|error| error.to_string()),
    }));
    hide_core::diagnostics::drain();
    std::process::exit(125);
}
