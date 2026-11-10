//! A daemon whose pane holders died without saying why restarts itself.
//!
//! A holder killed or crashed while the daemon runs leaves its record
//! `running` and its pane unreachable. The start of a daemon already finds
//! such a holder, ends its record `holder_lost` and wakes its agent
//! (`Holders::recover`), so the daemon asks for that start instead of
//! settling the pane a second way: a resident `hide restart --daemon <pid>`
//! ends this daemon and starts the next. The holders, and the programs in
//! them, keep running through it. A window follows the new daemon as it
//! follows any other (`desktop/src/main/host.ts`).
//!
//! The helper is started as a resident child (`hide_platform::process::start_resident`),
//! so it is no descendant of this daemon: the forced end `hide restart` makes
//! of a daemon that does not stop in time (`kill_tree`) never reaches it. It
//! names the daemon that asked, and does nothing when that daemon was stopped
//! or replaced meanwhile (`cli::restart`).
//!
//! A request is admitted at most once in [`EVERY`], across restarts: the check
//! and the record of it happen under one lock of the state folder, so the
//! many panes a single loss takes at once ask once. A pane lost while a
//! request is suppressed stays unreachable until the next restart or start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use hide_platform::fs::lock::{Mode, lock_file_blocking};
use hide_platform::fs::private;

/// The least time between two requests.
pub const EVERY: Duration = Duration::from_secs(600);

/// What `Holders::on_holder_lost` calls: asks for a restart of this daemon
/// of `state_dir`, or says why it did not.
pub fn on_holder_lost(state_dir: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
    Arc::new(move |pane| {
        let outcome = request(&state_dir, SystemTime::now(), &|| {
            launch(&state_dir, std::process::id())
        });
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
    /// A request was admitted within [`EVERY`].
    Waiting,
}

/// Admits one request: under the folder's lock, a request within [`EVERY`] of
/// the last waits, and otherwise `start` runs and, once it has, the request
/// is recorded. A start that fails records nothing, so the next loss asks
/// again.
fn request(
    state_dir: &Path,
    now: SystemTime,
    start: &dyn Fn() -> Result<(), String>,
) -> Result<Asked, String> {
    let lock = private::open_own_file(&hide_kit::layout::holder_restart_lock(state_dir), true)
        .map_err(|error| format!("the restart lock: {error}"))?;
    let _held = lock_file_blocking(lock, Mode::Exclusive)
        .map_err(|error| format!("the restart lock: {error}"))?;
    let marker = hide_kit::layout::holder_restart(state_dir);
    if let Ok(modified) = std::fs::metadata(&marker).and_then(|meta| meta.modified())
        // A mark in the future (a clock set back, or a request that read the
        // time before another recorded its own) is recent, not old.
        && now.duration_since(modified).map_or(true, |ago| ago < EVERY)
    {
        return Ok(Asked::Waiting);
    }
    start()?;
    std::fs::write(&marker, b"").map_err(|error| format!("{}: {error}", marker.display()))?;
    Ok(Asked::Requested)
}

/// Starts `hide restart --daemon <daemon>` beside this program.
fn launch(state_dir: &Path, daemon: u32) -> Result<(), String> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    let hide = executable
        .parent()
        .map(|folder| folder.join(format!("hide{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("no hide beside {}", executable.display()))?;
    start_resident(
        &hide,
        &["restart", "--daemon", &daemon.to_string()],
        state_dir,
    )
}

fn start_resident(program: &Path, args: &[&str], state_dir: &Path) -> Result<(), String> {
    let mut command = hide_platform::process::ResidentCommand::new(program);
    command.args(args).env("HIDE_STATE_DIR", state_dir);
    hide_platform::process::start_resident(&command, &[])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn many_requests_at_once_start_one_restart() {
        let folder = tempfile::tempdir().unwrap();
        let started = AtomicUsize::new(0);
        let answers: Vec<_> = std::thread::scope(|scope| {
            (0..24)
                .map(|_| {
                    scope.spawn(|| {
                        request(folder.path(), SystemTime::now(), &|| {
                            started.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(20));
                            Ok(())
                        })
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap().unwrap())
                .collect()
        });
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert_eq!(
            answers.iter().filter(|a| **a == Asked::Requested).count(),
            1
        );
    }

    #[test]
    fn a_request_waits_within_the_limit_asks_again_after_it_and_a_failed_start_records_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        assert!(request(folder.path(), now, &|| Err("no hide".to_owned())).is_err());
        assert!(!hide_kit::layout::holder_restart(folder.path()).exists());

        assert_eq!(
            request(folder.path(), now, &|| Ok(())),
            Ok(Asked::Requested)
        );
        let within = now + Duration::from_secs(60);
        assert_eq!(
            request(folder.path(), within, &|| panic!("asked within the limit")),
            Ok(Asked::Waiting)
        );
        let after = SystemTime::now() + EVERY + Duration::from_secs(1);
        assert_eq!(
            request(folder.path(), after, &|| Ok(())),
            Ok(Asked::Requested)
        );
    }

    /// The helper outlives a forced end of its starter's tree: it is no child
    /// of the starter, so a kill of the starter's descendants never finds it.
    #[cfg(unix)]
    #[test]
    fn the_helper_is_no_descendant_of_its_starter() {
        let folder = tempfile::tempdir().unwrap();
        let out = folder.path().join("parent");
        let script = format!("ps -o ppid= -p $$ > {}", out.display());
        start_resident(Path::new("/bin/sh"), &["-c", &script], folder.path()).unwrap();
        let parent = loop {
            if let Some(text) = std::fs::read_to_string(&out)
                .ok()
                .filter(|text| text.ends_with('\n'))
            {
                break text.trim().parse::<u32>().unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_ne!(parent, std::process::id());
    }
}
