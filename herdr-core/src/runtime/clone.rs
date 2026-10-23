//! Add a project's Clone from URL: one repository clone at a time, run by a
//! worker off the lock, reported in the `repository_clone` slot, and handed to
//! the ordinary local registration once the folder is in place.
//!
//! The worker is the clone's only owner. It asks the core's own node for the
//! clone (`hide_host::clone` on the node), which stops Git's whole process
//! group and removes the staging folder on every exit path; the node reports
//! at least once a second, the worker answers each report with the runtime's
//! cancel flag, and a runtime that has gone away answers as a cancel, so the
//! clone ends with its owner.
//! The URL is kept here, never in the snapshot or a log line, because it can
//! carry credentials; the host is what the shell shows and the log records.

use super::*;
use hide_node_link::clone::{CloneAnswer, CloneFailure, CloneProgress, CloneReport, CloneSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The longest one clone may run; Git silent for two minutes is already
/// ended by the node as stalled, so this bounds only a clone that keeps
/// making progress.
const CLONE_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// The clone the worker is running, with what only the core may hold.
pub(super) struct CloneJob {
    id: u64,
    /// As the shell sent it, credentials included; compared, never shown.
    url: String,
    cancel: Arc<AtomicBool>,
}

impl Runtime {
    /// Starts a clone of `url` into `parent/name`, or joins the one already
    /// running for the same URL and folder. `parent` is the canonical folder
    /// hided's `$HOME` line checked, and `name` the one it found free.
    pub(super) fn clone_repository(&mut self, url: &str, parent: &str, name: &str) -> bool {
        let source = match CloneSource::parse(url) {
            Ok(source) => source,
            Err(message) => {
                self.set_error("repository.clone_invalid", message, false);
                return true;
            }
        };
        if source.name() != name {
            self.set_error(
                "repository.clone_invalid",
                format!("{name} is not the folder this URL clones into"),
                false,
            );
            return true;
        }
        let path = Path::new(parent).join(name).display().to_string();
        let running = self
            .snapshot
            .repository_clone
            .as_ref()
            .filter(|clone| matches!(clone.phase.as_str(), "cloning" | "cancelling"));
        if let Some(running) = running {
            // The same clone sent again joins the one running: one clone,
            // one registration.
            if running.path == path
                && self
                    .repository_clone_job
                    .as_ref()
                    .is_some_and(|job| job.url == url.trim())
            {
                crate::diagnostic!(serde_json::json!({
                    "component": "repository_clone",
                    "kind": "duplicate",
                    "id": running.id,
                    "host": source.host(),
                }));
                return false;
            }
            self.set_error(
                "repository.clone_busy",
                "Another clone is still running; wait for it or cancel it first",
                false,
            );
            return true;
        }
        let Some(context) = self.live.as_ref().cloned() else {
            self.set_error(
                "workspace.control_unavailable",
                "Cloning requires a live Herdr connection so the new project can be opened",
                true,
            );
            return true;
        };
        self.next_repository_clone_id = self.next_repository_clone_id.wrapping_add(1).max(1);
        let id = self.next_repository_clone_id;
        let cancel = Arc::new(AtomicBool::new(false));
        self.snapshot.repository_clone = Some(crate::model::RepositoryCloneSnapshot {
            id,
            host: source.host().to_owned(),
            path: path.clone(),
            phase: "cloning".to_owned(),
            stage: None,
            percent: None,
            failure: None,
            git_line: None,
        });
        self.repository_clone_job = Some(CloneJob {
            id,
            url: url.trim().to_owned(),
            cancel: Arc::clone(&cancel),
        });
        crate::diagnostic!(serde_json::json!({
            "component": "repository_clone",
            "kind": "started",
            "id": id,
            "host": source.host(),
        }));
        if let Err(message) = spawn_clone(context, id, source, PathBuf::from(parent), cancel) {
            self.settle_repository_clone(id, Err(CloneFailure::Io(message)));
        }
        true
    }

    /// Asks the running clone to stop; the worker settles it as `cancelled`
    /// once Git has ended and the staging folder is gone.
    pub(super) fn cancel_repository_clone(&mut self, id: u64) -> bool {
        let Some(clone) = self
            .snapshot
            .repository_clone
            .as_mut()
            .filter(|clone| clone.id == id && clone.phase == "cloning")
        else {
            return false;
        };
        clone.phase = "cancelling".to_owned();
        if let Some(job) = self
            .repository_clone_job
            .as_ref()
            .filter(|job| job.id == id)
        {
            job.cancel.store(true, Ordering::SeqCst);
        }
        true
    }

    /// Git named a new stage or percent. Only the running clone moves.
    pub(crate) fn note_repository_clone_progress(
        &mut self,
        id: u64,
        progress: CloneProgress,
    ) -> bool {
        let Some(clone) = self
            .snapshot
            .repository_clone
            .as_mut()
            .filter(|clone| clone.id == id && clone.phase == "cloning")
        else {
            return false;
        };
        clone.stage = Some(progress.stage);
        clone.percent = progress.percent;
        true
    }

    /// Settles the clone with how it ended; a finished one is registered
    /// through the same path Browse folder uses.
    pub(crate) fn settle_repository_clone(
        &mut self,
        id: u64,
        result: Result<PathBuf, CloneFailure>,
    ) -> bool {
        let Some(clone) = self
            .snapshot
            .repository_clone
            .as_mut()
            .filter(|clone| clone.id == id)
        else {
            return false;
        };
        self.repository_clone_job = None;
        let phase = match &result {
            Ok(_) => "finished",
            Err(CloneFailure::Cancelled) => "cancelled",
            Err(_) => "failed",
        };
        clone.phase = phase.to_owned();
        clone.failure = result
            .as_ref()
            .err()
            .filter(|failure| !matches!(failure, CloneFailure::Cancelled))
            .map(|failure| failure.code().to_owned());
        clone.git_line = match &result {
            Err(CloneFailure::Git(line)) => Some(line.clone()),
            _ => None,
        };
        clone.stage = None;
        clone.percent = None;
        crate::diagnostic!(serde_json::json!({
            "component": "repository_clone",
            "kind": phase,
            "id": id,
            "host": clone.host,
            "reason": result.as_ref().err().map(CloneFailure::code),
            "message": result.as_ref().err().map(CloneFailure::message),
        }));
        let Ok(path) = result else {
            return true;
        };
        let path = path.display().to_string();
        let label = path.rsplit('/').next().unwrap_or_default().to_owned();
        // The same event Browse folder sends, so a cloned folder is
        // registered, refused or deduplicated exactly as a picked one.
        self.apply(Event::CreateWorkspace(CreateWorkspacePayload {
            device_id: None,
            path,
            label,
            initialize_git: false,
            // The clone made the folder; the registration only adds it.
            new_folder: false,
        }));
        true
    }
}

/// Runs the clone on its own thread. Progress and the result each take the
/// lock only to write the slot; Git runs with the lock free.
fn spawn_clone(
    context: live::LiveContext,
    id: u64,
    source: CloneSource,
    parent: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("herdr-core-repository-clone".to_owned())
        .spawn(move || {
            let runtime = context.runtime.clone();
            let notifier = context.notifier.clone();
            let publish = |apply: &mut dyn FnMut(&mut Runtime) -> bool| {
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let changed = runtime
                    .lock()
                    .map(|mut guard| apply(&mut guard))
                    .unwrap_or(false);
                if changed {
                    notifier.notify();
                }
            };
            // A panic still settles the slot, which is the only one: a clone
            // left `cloning` would refuse every later clone. Git's group and
            // the staging folder are released by their guards on unwinding.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let call = hide_node_link::protocol::Call::RepositoryClone {
                    source,
                    parent: parent.to_string_lossy().into_owned(),
                };
                let answer = crate::node_access::call_as_with_progress::<CloneAnswer, CloneReport>(
                    context.node.as_ref(),
                    call,
                    CLONE_TIMEOUT,
                    |report| {
                        if let CloneReport::Progress { progress } = report {
                            publish(&mut |guard| {
                                guard.note_repository_clone_progress(id, progress.clone())
                            });
                        }
                        // The runtime going away ends the clone with it.
                        !cancel.load(Ordering::SeqCst) && runtime.strong_count() > 0
                    },
                );
                match answer {
                    Ok(CloneAnswer::Cloned { path }) => Ok(path),
                    Ok(CloneAnswer::Failed { failure }) => Err(failure),
                    Err(error) => Err(CloneFailure::Io(format!(
                        "The clone could not be asked for: {error}"
                    ))),
                }
            }))
            .unwrap_or_else(|_| {
                Err(CloneFailure::Io(
                    "The clone stopped unexpectedly; nothing was kept.".to_owned(),
                ))
            });
            let mut result = Some(result);
            publish(&mut |guard| {
                result
                    .take()
                    .is_some_and(|result| guard.settle_repository_clone(id, result))
            });
        })
        .map(|_| ())
        .map_err(|error| format!("The clone could not be started: {error}"))
}
