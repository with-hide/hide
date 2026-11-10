//! A daemon whose pane holders died without saying why restarts itself.
//!
//! A holder killed or crashed while the daemon runs leaves its record
//! `running` and its pane unreachable. The start of a daemon already finds
//! such a holder, ends its record `holder_lost` and wakes its agent
//! (`Holders::recover`), so the daemon asks for that start instead of
//! settling the pane a second way: a resident `hide restart --daemon <pid>
//! --started <start>` ends this daemon and starts the next. The holders, and the programs in
//! them, keep running through it. A window follows the new daemon as it
//! follows any other (`desktop/src/main/host.ts`).
//!
//! The helper is started as a resident child (`hide_platform::process::start_resident`),
//! so it is no descendant of this daemon: the forced end `hide restart` makes
//! of a daemon that does not stop in time (`kill_tree`) never reaches it. It
//! names the daemon that asked, by pid and start time, and does nothing when that daemon was stopped
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
        let outcome = request(&state_dir, SystemTime::now(), &|| launch(&state_dir));
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
/// the last waits. Otherwise the admission is recorded first, so a helper that
/// escapes is never without its receipt, and then `start` runs; a record that
/// cannot be written starts nothing, and a start that fails withdraws the
/// record so the next loss asks again.
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
    std::fs::write(&marker, b"").map_err(|error| format!("{}: {error}", marker.display()))?;
    if let Err(error) = start() {
        let _ = std::fs::remove_file(&marker);
        return Err(error);
    }
    Ok(Asked::Requested)
}

/// Starts `hide restart` for this daemon, named by its pid and start time,
/// beside this program.
fn launch(state_dir: &Path) -> Result<(), String> {
    let daemon = std::process::id();
    let started = hide_platform::process::start_time(daemon)
        .map_err(|error| format!("this daemon's start time: {error}"))?;
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| error.to_string())?;
    let hide = executable
        .parent()
        .map(|folder| folder.join(format!("hide{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("no hide beside {}", executable.display()))?;
    let (daemon, started) = (daemon.to_string(), started.to_string());
    start_resident(
        &hide,
        &["restart", "--daemon", &daemon, "--started", &started],
        state_dir,
    )
}

/// Starts `program` as a resident child with the state folder in its environment.
fn start_resident(program: &Path, args: &[&str], state_dir: &Path) -> Result<(), String> {
    let mut command = hide_platform::process::ResidentCommand::new(program);
    command.args(args).env("HIDE_STATE_DIR", state_dir);
    hide_platform::process::start_resident(&command, &[])
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn many_requests_at_once_start_one_restart() {
        let folder = tempfile::tempdir().unwrap();
        let started = AtomicUsize::new(0);
        let together = Barrier::new(24);
        let answers: Vec<_> = std::thread::scope(|scope| {
            (0..24)
                .map(|_| {
                    scope.spawn(|| {
                        together.wait();
                        request(folder.path(), SystemTime::now(), &|| {
                            started.fetch_add(1, Ordering::SeqCst);
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

    /// An admission that cannot be recorded starts nothing.
    #[test]
    fn a_request_that_cannot_be_recorded_starts_no_helper() {
        let folder = tempfile::tempdir().unwrap();
        // A folder where the mark goes: its modified time is old enough, and
        // writing a file over it fails.
        std::fs::create_dir(hide_kit::layout::holder_restart(folder.path())).unwrap();
        let later = SystemTime::now() + EVERY + Duration::from_secs(1);
        let started = AtomicUsize::new(0);
        let answer = request(folder.path(), later, &|| {
            started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(answer.is_err(), "{answer:?}");
        assert_eq!(started.load(Ordering::SeqCst), 0);
    }

    /// The helper outlives a forced end of its starter's tree: it is no child
    /// of the starter, so a kill of the starter's descendants never finds it.
    /// It reports its parent through a handle it is lent, and the read ends
    /// when the helper does.
    #[cfg(unix)]
    #[test]
    fn the_helper_is_no_descendant_of_its_starter() {
        use std::io::Read;
        use std::os::fd::AsFd;
        let (mut reader, writer) = std::io::pipe().unwrap();
        let mut command = hide_platform::process::ResidentCommand::new("/bin/sh");
        command.args(["-c", "ps -o ppid= -p $$ >&3"]);
        hide_platform::process::start_resident(&command, &[writer.as_fd()]).unwrap();
        drop(writer);
        let mut said = String::new();
        reader.read_to_string(&mut said).unwrap();
        let parent: u32 = said
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("the helper said {said:?}"));
        assert_ne!(parent, std::process::id());
    }
}
