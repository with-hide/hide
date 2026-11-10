//! A daemon whose pane holders died without saying why restarts itself.
//!
//! A holder killed or crashed while the daemon runs leaves its record
//! `running` and its pane unreachable. The start of a daemon already finds
//! such a holder, ends its record `holder_lost` and wakes its agent
//! (`Holders::recover`), so the daemon asks for that start instead of
//! settling the pane a second way: a detached `hide restart` ends this
//! daemon and starts the next. The holders, and the programs in them, keep
//! running through it. A window follows the new daemon as it follows any
//! other (`desktop/src/main/host.ts`).
//!
//! The request is made at most once in [`EVERY`], across restarts, by the
//! modified time of one file in the state folder, so a record a start cannot
//! settle never becomes a restart loop; the unreachable pane stays as it was.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// The least time between two requests.
pub const EVERY: Duration = Duration::from_secs(600);

/// What `Holders::on_holder_lost` calls: asks for a restart of the daemon of
/// `state_dir`, or says why it did not.
pub fn on_holder_lost(state_dir: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
    Arc::new(move |pane| {
        let outcome = request(&state_dir, SystemTime::now());
        hide_core::diagnostic!(serde_json::json!({
            "component": "hided",
            "kind": "daemon.restart_for_lost_holder",
            "pane_id": pane,
            "outcome": match &outcome {
                Ok(Asked::Requested) => "requested".to_owned(),
                Ok(Asked::Waiting) => "already_asked".to_owned(),
                Err(error) => format!("failed: {error}"),
            },
        }));
    })
}

#[derive(Debug, Eq, PartialEq)]
enum Asked {
    Requested,
    /// A request was made within [`EVERY`].
    Waiting,
}

fn request(state_dir: &Path, now: SystemTime) -> Result<Asked, String> {
    let marker = hide_kit::layout::holder_restart(state_dir);
    if let Ok(modified) = std::fs::metadata(&marker).and_then(|meta| meta.modified())
        && now.duration_since(modified).is_ok_and(|ago| ago < EVERY)
    {
        return Ok(Asked::Waiting);
    }
    // Marked before it starts, so two panes found at once ask once: the
    // second finds the file the first wrote.
    std::fs::write(&marker, b"").map_err(|error| format!("{}: {error}", marker.display()))?;
    start(state_dir)?;
    Ok(Asked::Requested)
}

/// Starts `hide restart` beside this daemon, in a group of its own so the
/// daemon's end does not take it along. Where a process cannot be detached
/// safely from a daemon with threads (Windows, `hide_platform::process::detach`)
/// it is refused, and the pane is settled by the next start as before.
#[cfg(unix)]
fn start(state_dir: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    let hide = executable
        .parent()
        .map(|folder| folder.join(format!("hide{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("no hide beside {}", executable.display()))?;
    let mut command = std::process::Command::new(hide);
    command.arg("restart").env("HIDE_STATE_DIR", state_dir);
    crate::spawn::spawn_owned(&mut command)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn start(_: &Path) -> Result<(), String> {
    Err("a daemon does not restart itself on this system".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_request_within_the_limit_waits_and_one_after_it_asks_again() {
        let folder = tempfile::tempdir().unwrap();
        let marker = hide_kit::layout::holder_restart(folder.path());
        std::fs::write(&marker, b"").unwrap();
        let modified = std::fs::metadata(&marker).unwrap().modified().unwrap();
        assert_eq!(
            request(folder.path(), modified + Duration::from_secs(60)),
            Ok(Asked::Waiting)
        );
        // Past the limit it asks, which here finds no `hide` beside the test
        // program and says so; the marker is written first either way.
        assert!(request(folder.path(), modified + EVERY + Duration::from_secs(1)).is_err());
    }
}
