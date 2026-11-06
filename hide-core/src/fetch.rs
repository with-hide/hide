//! The background fetch's one thread (PRD git-freshness D-01, D-02).
//!
//! Once a second it applies the fetches that ended and starts the ones the
//! runtime says are due (`Runtime::due_fetches`): at most one per node, each
//! on a thread of its own that asks the node holding the repository
//! (`Call::FetchRemote`), so the operator's credentials on that machine are
//! the ones used and nothing runs under the lock, on the session-sync thread
//! or on this core's machine for a device's repository. A fetch's thread
//! ends when its node answers or the call's own bound
//! (`worktrees::FETCH_TIMEOUT`) passes; the node stops Git at
//! `worktrees::FETCH_TIME`.
//!
//! The same tick drives each connected device's Git watch
//! (`worktrees::DeviceGitWatch`, D-08): a fetch there moves that device's
//! refs, and so does a branch an agent switches or a worktree it adds, and
//! the watch hands the repositories that changed to the device catalog to
//! read again, and a device whose last read failed is read again after a
//! pause (`Runtime::retry_device_reads`). With no window attached the
//! watches rest, as the fetch does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use hide_node_link::protocol::Call;
use hide_node_link::worktrees::{FETCH_TIMEOUT, FetchOutcome};

use crate::handle::ChangeNotifier;
use crate::runtime::{FetchJob, Runtime};
use crate::worktrees::{DeviceGitWatch, DeviceWatchSet};

const PUMP_TICK: Duration = Duration::from_secs(1);

type Answer = Result<FetchOutcome, String>;

pub(crate) struct FetchPump {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl FetchPump {
    pub fn spawn(runtime: Weak<Mutex<Runtime>>, notifier: ChangeNotifier) -> std::io::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("fetch-pump".into())
            .spawn(move || {
                // The fetch out on each node, by device.
                let mut running: BTreeMap<String, (FetchJob, Receiver<Answer>)> = BTreeMap::new();
                let mut watches: BTreeMap<String, DeviceGitWatch> = BTreeMap::new();
                while matches!(
                    receiver.recv_timeout(PUMP_TICK),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    let Some(runtime) = runtime.upgrade() else {
                        break;
                    };
                    let mut ended = Vec::new();
                    running.retain(|_, (job, answer)| match answer.try_recv() {
                        Ok(answer) => {
                            ended.push((job.clone(), answer));
                            false
                        }
                        Err(TryRecvError::Empty) => true,
                        Err(TryRecvError::Disconnected) => {
                            ended.push((
                                job.clone(),
                                Err("The fetch thread ended without an answer".into()),
                            ));
                            false
                        }
                    });
                    let Ok((jobs, mut changed, watch_sets)) = runtime.lock().map(|mut runtime| {
                        let mut changed = false;
                        for (job, answer) in &ended {
                            changed |= runtime.ingest_fetch(
                                job,
                                answer.clone(),
                                crate::observed::now_unix_ms(),
                            );
                        }
                        if changed {
                            let before = runtime.published_checkout_header();
                            changed = runtime.refresh_checkout_header() != before;
                        }
                        changed |= runtime.retry_device_reads(Instant::now());
                        (
                            runtime.due_fetches(Instant::now()),
                            changed,
                            runtime.device_git_watch_sets(),
                        )
                    }) else {
                        break;
                    };
                    // Off the lock: a watch replaced or ended here waits a
                    // bounded time for its thread.
                    let moved = watch_devices(&mut watches, watch_sets);
                    if !moved.is_empty() {
                        let Ok(read) = runtime.lock().map(|mut runtime| {
                            moved.into_iter().fold(false, |read, (device, roots)| {
                                read | runtime.device_git_changed(&device, roots)
                            })
                        }) else {
                            break;
                        };
                        changed |= read;
                    }
                    drop(runtime);
                    if changed {
                        notifier.notify();
                    }
                    for (job, link) in jobs {
                        let (sender, answer) = mpsc::channel();
                        let path = job.key.1.clone();
                        let spawned =
                            thread::Builder::new()
                                .name("hide-fetch".into())
                                .spawn(move || {
                                    let outcome = crate::node_access::call_as::<FetchOutcome>(
                                        link.as_ref(),
                                        Call::FetchRemote { path },
                                        FETCH_TIMEOUT,
                                    )
                                    .map_err(|error| error.to_string());
                                    let _ = sender.send(outcome);
                                });
                        match spawned {
                            Ok(_) => {
                                running.insert(job.key.0.clone(), (job, answer));
                            }
                            // Dropping the sender reads as an ended fetch on
                            // the next pass, which records the failure.
                            Err(error) => {
                                crate::diagnostic!(serde_json::json!({
                                    "component": "fetch",
                                    "kind": "fetch.thread_failed",
                                    "message": error.to_string(),
                                }));
                                running.insert(job.key.0.clone(), (job, answer));
                            }
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}

/// Brings the device Git watches to `sets` and answers each device's
/// repositories that changed. A device no longer listed, or listed over a
/// new link, loses its watch; `None` rests every watch.
fn watch_devices(
    watches: &mut BTreeMap<String, DeviceGitWatch>,
    sets: Option<Vec<DeviceWatchSet>>,
) -> Vec<(String, Vec<PathBuf>)> {
    let Some(sets) = sets else {
        watches.values_mut().for_each(DeviceGitWatch::rest);
        return Vec::new();
    };
    watches.retain(|device, watch| {
        sets.iter()
            .any(|(listed, link, _)| listed == device && watch.watches_over(link))
    });
    let mut moved = Vec::new();
    for (device, link, projects) in sets {
        let roots = watches
            .entry(device.clone())
            .or_insert_with(|| DeviceGitWatch::new(link))
            .changed(projects);
        if !roots.is_empty() {
            moved.push((device, roots));
        }
    }
    moved
}

impl Drop for FetchPump {
    /// The pump ends at once; a fetch still out ends on its own bound, and
    /// its answer goes nowhere.
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
