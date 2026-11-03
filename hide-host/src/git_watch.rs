//! Watches repositories' Git facts for the core's worktree reader: one call
//! holds one OS watcher over the repositories' common directories, reports
//! which of them changed, and ends the watcher when its caller stops it or
//! the node closes (`hide_node_link::worktrees::GitWatchReport`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use hide_node_link::worktrees::{GIT_WATCH_LIMIT, GitWatchReport, git_fact_path};
use hide_platform::watch::{Change, Watcher};

use crate::error::{ErrorCode, HostError, HostResult};
use crate::reporting::HEARTBEAT;

/// Watches `common_dirs` until `report` answers false or `stop` is set.
pub fn watch(
    common_dirs: &[PathBuf],
    stop: &AtomicBool,
    report: &mut dyn FnMut(GitWatchReport) -> bool,
) -> HostResult<()> {
    if common_dirs.len() > GIT_WATCH_LIMIT {
        return Err(HostError::new(
            ErrorCode::InvalidRequest,
            format!("At most {GIT_WATCH_LIMIT} repositories are watched at once"),
        ));
    }
    let (mut watcher, changes) = Watcher::keeping(git_fact_path).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("the Git watch could not start: {error}"),
        )
    })?;
    let mut unwatched = Vec::new();
    for dir in common_dirs {
        if let Err(error) = watcher.watch(dir) {
            unwatched.push((crate::root::wire(dir), error.to_string()));
        }
    }
    if !report(GitWatchReport::Watching { unwatched }) {
        return Ok(());
    }
    while !stop.load(Ordering::Relaxed) {
        let mut changed = BTreeSet::new();
        let mut overflow = None;
        let first = changes.recv_timeout(HEARTBEAT);
        for change in first
            .into_iter()
            .chain(std::iter::from_fn(|| changes.try_recv()))
        {
            match change {
                Change::Path { path, .. } => {
                    for dir in common_dirs {
                        if path.strip_prefix(dir).is_ok_and(git_fact_path) {
                            changed.insert(crate::root::wire(dir));
                        }
                    }
                }
                Change::Overflow { reason, .. } => overflow = Some(reason),
            }
        }
        let next = match overflow {
            Some(reason) => GitWatchReport::Overflow { reason },
            None if changed.is_empty() => GitWatchReport::Quiet,
            None => GitWatchReport::Changed {
                common_dirs: changed.into_iter().collect(),
            },
        };
        if !report(next) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A commit's ref write reaches the caller as its repository's change,
    /// and a false answer ends the watch.
    #[test]
    fn a_ref_write_is_reported_as_its_repository_changing() {
        let repository = tempfile::tempdir().unwrap();
        let common = repository.path().canonicalize().unwrap();
        std::fs::create_dir_all(common.join("refs/heads")).unwrap();
        let stop = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut wrote = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        watch(std::slice::from_ref(&common), &stop, &mut |report| {
            if !wrote {
                std::fs::write(common.join("refs/heads/main"), "0000\n").unwrap();
                wrote = true;
            }
            let done = matches!(report, GitWatchReport::Changed { .. })
                || std::time::Instant::now() > deadline;
            seen.push(report);
            !done
        })
        .unwrap();
        assert_eq!(
            seen.first(),
            Some(&GitWatchReport::Watching {
                unwatched: Vec::new()
            })
        );
        assert_eq!(
            seen.last(),
            Some(&GitWatchReport::Changed {
                common_dirs: vec![crate::root::wire(&common)]
            })
        );
    }
}
