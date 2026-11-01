//! The private `CODEX_HOME` each app-server runs under, and the sweep that
//! releases the ones an owner could not.
//!
//! `Drop` removes a home on every exit path the owner survives. A `kill -9`
//! is the one it does not, so the next owner to start removes the homes whose
//! owner pid is gone. The sweep only touches directories this module named.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hide_platform::process::is_alive as pid_alive;

use crate::log::AiLogEvent;
use crate::{AiError, AiLogSink, ProviderId};

const PREFIX: &str = "hide-ai-codex-home-";

/// At most this many homes are removed per sweep, so one start never spends
/// long on a backlog (344 dead owners' homes on one machine on 2026-09-28);
/// the next start continues where this one stopped.
const SWEEP_LIMIT: usize = 32;

/// A private `CODEX_HOME` for one app-server: an owner-only directory holding
/// nothing but a symlink to the user's `auth.json`. It carries no
/// `config.toml`, so the app-server starts none of the MCP servers the user's
/// real config declares, and it is removed when the session ends. The
/// credential file itself is only referenced, never read.
pub(crate) struct CodexHome {
    pub(crate) path: PathBuf,
}

impl CodexHome {
    pub(crate) fn create() -> Result<Self, AiError> {
        let dir = std::env::temp_dir().join(format!(
            "{PREFIX}{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        create_private_dir(&dir).map_err(|kind| home_unavailable("mkdir", kind))?;
        let home = Self { path: dir };
        link_auth_json(&home.path).map_err(|kind| home_unavailable("symlink", kind))?;
        Ok(home)
    }
}

impl Drop for CodexHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Sweeps the temporary directory for homes of dead owners on a thread of its
/// own, so the caller (a backend being built, possibly under someone else's
/// lock) never waits on the filesystem. The thread is detached: a process that
/// exits mid-sweep leaves the rest for the next start, which is safe because
/// a removal that already happened is simply not found again.
pub(crate) fn spawn_sweep(sink: Arc<dyn AiLogSink>) {
    let thread_sink = Arc::clone(&sink);
    let spawned = std::thread::Builder::new()
        .name("hide-ai-codex-home-sweep".to_owned())
        .spawn(move || {
            let report = sweep(&std::env::temp_dir(), SWEEP_LIMIT);
            log_sweep(report, thread_sink.as_ref());
        });
    if let Err(error) = spawned {
        let mut event = AiLogEvent::new("ai.codex_home.sweep_failed");
        event.provider = Some(ProviderId::CODEX);
        event.detail = Some(format!("stage=thread;kind={}", error.kind()));
        sink.log(event);
    }
}

#[derive(Debug, Default, PartialEq)]
struct SweepReport {
    removed: usize,
    bytes: u64,
    /// Dead owners' homes that could not be removed.
    failed: usize,
    /// Directory listing errors. The listing stops at the first one, so a
    /// nonzero count means the rest of the temporary directory went unswept.
    unreadable: usize,
    /// The kind of the first removal or read error, so a `failed` count that
    /// repeats every start says why.
    first_error: Option<std::io::ErrorKind>,
    /// More dead owners' homes were left than the limit allowed removing.
    capped: bool,
}

impl SweepReport {
    fn record_error(&mut self, kind: std::io::ErrorKind) {
        self.first_error.get_or_insert(kind);
    }
}

fn log_sweep(report: Result<SweepReport, std::io::ErrorKind>, sink: &dyn AiLogSink) {
    let mut event = match report {
        Ok(SweepReport {
            removed: 0,
            failed: 0,
            unreadable: 0,
            ..
        }) => return,
        Ok(report) => {
            let mut detail = format!(
                "removed={};bytes={};failed={};unreadable={};capped={}",
                report.removed, report.bytes, report.failed, report.unreadable, report.capped
            );
            if let Some(kind) = report.first_error {
                detail.push_str(&format!(";first_error={kind}"));
            }
            let mut event = AiLogEvent::new("ai.codex_home.swept");
            event.detail = Some(detail);
            event
        }
        Err(kind) => {
            let mut event = AiLogEvent::new("ai.codex_home.sweep_failed");
            event.detail = Some(format!("stage=read_dir;kind={kind}"));
            event
        }
    };
    event.provider = Some(ProviderId::CODEX);
    sink.log(event);
}

/// Removes up to `limit` homes under `root` whose owner pid is no longer
/// alive. A live pid keeps its homes even when the pid was reused by an
/// unrelated process: keeping a dead owner's directory costs disk, deleting a
/// live owner's breaks its app-server. A name that does not parse, anything
/// that is not a plain directory, and a directory another user owns are left
/// alone.
fn sweep(root: &Path, limit: usize) -> Result<SweepReport, std::io::ErrorKind> {
    let mut report = SweepReport::default();
    let entries = std::fs::read_dir(root).map_err(|error| error.kind())?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.unreadable += 1;
                report.record_error(error.kind());
                continue;
            }
        };
        let Some(pid) = entry.file_name().to_str().and_then(owner_pid) else {
            continue;
        };
        let path = entry.path();
        if !is_own_directory(&path) || pid_alive(pid) {
            continue;
        }
        if report.removed + report.failed >= limit {
            report.capped = true;
            break;
        }
        let bytes = tree_bytes(&path);
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                report.removed += 1;
                report.bytes += bytes;
            }
            // Another owner's sweep got there first.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                report.failed += 1;
                report.record_error(error.kind());
            }
        }
    }
    Ok(report)
}

/// The owner pid encoded in `hide-ai-codex-home-<pid>-<nanos>`, or `None`
/// for any name this module did not produce.
fn owner_pid(name: &str) -> Option<u32> {
    let (pid, nanos) = name.strip_prefix(PREFIX)?.split_once('-')?;
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(pid) || !all_digits(nanos) {
        return None;
    }
    pid.parse().ok().filter(|pid| *pid != 0)
}

fn is_own_directory(path: &Path) -> bool {
    // `symlink_metadata`, so a symlink carrying the prefix is never followed.
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && hide_platform::fs::private::owned_by_current_user(path).unwrap_or(false)
}

/// The bytes a home holds, for the log line. Symlinks are counted as
/// themselves and never followed, so the user's `auth.json` is not measured.
fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| tree_bytes(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

fn home_unavailable(stage: &str, kind: std::io::ErrorKind) -> AiError {
    AiError::ProviderUnavailable(format!("codex_home_unavailable:{stage}:{kind}"))
}

/// The directory the user's real `auth.json` lives in: Codex's own folder as
/// the operator runs it (`CODEX_HOME`, or `~/.codex`), from the one resolver
/// of agent folders. A relative `CODEX_HOME` leaves the home unavailable
/// rather than linking a path Codex would read against another folder.
fn source_codex_dir() -> Result<PathBuf, std::io::ErrorKind> {
    let codex = hide_agent_adapter::AgentId::Codex.adapter();
    let account_home = hide_platform::host::home_dir().ok();
    let value = codex.home_variable.and_then(hide_platform::host::variable);
    codex
        .home(
            account_home.as_deref(),
            value.as_deref(),
            hide_agent_adapter::HomeScope::Read,
        )
        .map_err(|unresolved| match unresolved {
            hide_agent_adapter::HomeUnresolved::NoAccountHome => std::io::ErrorKind::NotFound,
            hide_agent_adapter::HomeUnresolved::InvalidOverride { .. } => {
                std::io::ErrorKind::InvalidInput
            }
        })
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn create_private_dir(dir: &Path) -> Result<(), std::io::ErrorKind> {
    hide_platform::fs::private::create_dir(dir).map_err(|error| error.kind())
}

/// A link to a file needs a privilege on a Windows account without
/// Developer Mode; the home is then unavailable with that kind.
fn link_auth_json(home: &Path) -> Result<(), std::io::ErrorKind> {
    let source = source_codex_dir()?.join("auth.json");
    hide_platform::fs::link::create_link(&source, &home.join("auth.json"))
        .map_err(|error| error.kind())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A fixture root of its own, so the sweep under test never sees the
    /// real temporary directory.
    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hide-ai-sweep-{name}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        std::fs::create_dir_all(&root).expect("a fixture root is creatable");
        root
    }

    /// A pid no process has: a child that has exited and been reaped.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("`true` starts");
        let pid = child.id();
        child.wait().expect("`true` is reaped");
        pid
    }

    /// A home as an app-server leaves it: state files beside an `auth.json`
    /// symlink pointing at a credential outside the home.
    fn home(root: &Path, name: &str, credential: &Path) -> PathBuf {
        let home = root.join(name);
        std::fs::create_dir_all(home.join("shell_snapshots")).expect("home is creatable");
        std::fs::write(home.join("models_cache.json"), vec![b'x'; 1000]).expect("state writes");
        std::fs::write(home.join("shell_snapshots/one"), vec![b'x'; 24]).expect("state writes");
        std::os::unix::fs::symlink(credential, home.join("auth.json")).expect("symlink");
        home
    }

    #[test]
    fn a_dead_owners_home_is_removed_and_its_credential_survives() {
        let root = root("dead");
        let credential = root.join("auth.json");
        std::fs::write(&credential, b"{}").expect("credential writes");
        let home = home(&root, &format!("{PREFIX}{}-1", dead_pid()), &credential);

        let report = sweep(&root, SWEEP_LIMIT).expect("the root is readable");

        assert!(!home.exists());
        assert!(credential.exists(), "the symlink target is never followed");
        assert_eq!(report.removed, 1);
        assert!(report.bytes >= 1024, "state files are counted: {report:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_live_owners_home_and_foreign_names_are_kept() {
        let root = root("kept");
        let credential = root.join("auth.json");
        std::fs::write(&credential, b"{}").expect("credential writes");
        let dead = dead_pid();
        let kept = [
            format!("{PREFIX}{}-1", std::process::id()),
            format!("{PREFIX}{dead}"),
            format!("{PREFIX}{dead}-1-extra"),
            format!("{PREFIX}x{dead}-1"),
            format!("{PREFIX}0-1"),
            format!("hide-ai-codex-{dead}-1"),
        ];
        for name in &kept {
            home(&root, name, &credential);
        }
        let file = root.join(format!("{PREFIX}{dead}-2"));
        std::fs::write(&file, b"not a directory").expect("file writes");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir(&elsewhere).expect("target is creatable");
        let link = root.join(format!("{PREFIX}{dead}-3"));
        std::os::unix::fs::symlink(&elsewhere, &link).expect("symlink");

        let report = sweep(&root, SWEEP_LIMIT).expect("the root is readable");

        for name in &kept {
            assert!(root.join(name).exists(), "{name} is kept");
        }
        assert!(file.exists());
        assert!(link.symlink_metadata().is_ok() && elsewhere.exists());
        assert_eq!(report, SweepReport::default());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_home_whose_pid_belongs_to_another_user_is_kept() {
        let root = root("eperm");
        let credential = root.join("auth.json");
        // SAFETY: `getuid` has no preconditions and cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return; // root may signal pid 1, so `kill` would not answer EPERM.
        }
        // pid 1 is root's, so `kill` answers EPERM for anyone else: a process
        // exists, so its homes are not ours to judge.
        let home = home(&root, &format!("{PREFIX}1-1"), &credential);

        let report = sweep(&root, SWEEP_LIMIT).expect("the root is readable");

        assert!(pid_alive(1));
        assert!(home.exists());
        assert_eq!(report, SweepReport::default());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_home_that_cannot_be_removed_is_logged_with_its_error_kind() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: `getuid` has no preconditions and cannot fail.
        if unsafe { libc::getuid() } == 0 {
            return; // root removes a read-only directory's entries anyway.
        }
        let root = root("failed");
        let credential = root.join("auth.json");
        let home = home(&root, &format!("{PREFIX}{}-1", dead_pid()), &credential);
        let locked = home.join("shell_snapshots");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).expect("chmod");

        let report = sweep(&root, SWEEP_LIMIT);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).ok();
        let recorder = Recorder::default();
        log_sweep(report, &recorder);

        let events = recorder.0.lock().expect("recorder lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "ai.codex_home.swept");
        let detail = events[0].detail.as_deref().unwrap_or_default();
        assert!(
            detail.contains("removed=0;") && detail.contains("failed=1;"),
            "{detail}"
        );
        assert!(
            detail.ends_with(";first_error=permission denied"),
            "{detail}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<AiLogEvent>>);

    impl AiLogSink for Recorder {
        fn log(&self, event: AiLogEvent) {
            self.0.lock().expect("recorder lock").push(event);
        }
    }

    #[test]
    fn one_sweep_removes_at_most_the_limit() {
        let root = root("limit");
        let credential = root.join("auth.json");
        let dead = dead_pid();
        for nanos in 0..3 {
            home(&root, &format!("{PREFIX}{dead}-{nanos}"), &credential);
        }

        let first = sweep(&root, 2).expect("the root is readable");
        let second = sweep(&root, 2).expect("the root is readable");

        assert_eq!((first.removed, first.capped), (2, true));
        assert_eq!((second.removed, second.capped), (1, false));
        std::fs::remove_dir_all(&root).ok();
    }
}
