//! The socket reads the coordinator needs besides the event stream: the agent
//! list, a new pane's cwd, a workspace's replacement active tab, and the
//! agent of a start Herdr still holds open while the agent is at rest
//! (`launch_settle.rs`).
//!
//! Each opens a fresh connection to Herdr, so they run on worker threads,
//! one per kind, and the thread that applies Herdr's events never waits on
//! one (PRD instant-pane-topology D-13). An answer comes back as a
//! coordinator message carrying the subscription generation it was asked
//! under, so an answer that outlives its subscription is dropped. One read
//! of each kind is in flight at a time, which bounds the work queued behind
//! a slow Herdr; the workers end when the coordinator drops its reader.

use super::*;

pub(crate) enum SyncRead {
    Agents,
    PaneCwds(Vec<String>),
    ActiveTabs(Vec<String>),
    /// `agent.get` of each pane, which is what lets Herdr settle its start
    /// (herdrdev/herdr#3853, `launch_settle.rs`).
    Launches(Vec<String>),
}

pub(crate) enum SyncReadAnswer {
    Agents {
        started_at: Instant,
        result: Result<Vec<ProjectedAgent>, SessionFetchError>,
    },
    PaneCwds(Result<Vec<(String, Option<String>)>, SessionFetchError>),
    ActiveTabs(Result<Vec<(String, String)>, SessionFetchError>),
    /// Each pane read, with the reason a read failed.
    Launches(Vec<(String, Result<(), SessionFetchError>)>),
}

impl SyncRead {
    fn kind(&self) -> usize {
        match self {
            Self::Agents => 0,
            Self::PaneCwds(_) => 1,
            Self::ActiveTabs(_) => 2,
            Self::Launches(_) => 3,
        }
    }
}

impl SyncReadAnswer {
    fn kind(&self) -> usize {
        match self {
            Self::Agents { .. } => 0,
            Self::PaneCwds(_) => 1,
            Self::ActiveTabs(_) => 2,
            Self::Launches(_) => 3,
        }
    }
}

/// One worker per kind, so a slow `agent.list` never holds a new pane's
/// `pane.get` behind it.
pub(crate) struct SyncReader {
    context: SessionSyncContext,
    coordinator: Sender<CoordinatorMessage>,
    workers: [Sender<(u64, SyncRead)>; 4],
    /// The generation each kind's read in flight was asked under.
    inflight: [Option<u64>; 4],
}

const KIND_NAMES: [&str; 4] = ["agents", "pane-cwds", "active-tabs", "launches"];

impl SyncReader {
    pub(crate) fn start(
        context: &SessionSyncContext,
        coordinator: Sender<CoordinatorMessage>,
    ) -> Result<Self, String> {
        let workers = [
            spawn_worker(context, &coordinator, 0)?,
            spawn_worker(context, &coordinator, 1)?,
            spawn_worker(context, &coordinator, 2)?,
            spawn_worker(context, &coordinator, 3)?,
        ];
        Ok(Self {
            context: context.clone(),
            coordinator,
            workers,
            inflight: [None; 4],
        })
    }

    /// Asks for `read` under subscription `generation`; false while a read of
    /// the same kind is still out, whose answer covers the same need. A
    /// worker that ended is started again, so a read that gates publishing
    /// is never lost to a dead thread; a read that panics answers as a
    /// failure instead (`spawn_worker`), so this is the path of a worker the
    /// system ended.
    pub(crate) fn ask(&mut self, generation: u64, read: SyncRead) -> bool {
        let kind = read.kind();
        if self.inflight[kind].is_some() {
            return false;
        }
        let read = match self.workers[kind].send((generation, read)) {
            Ok(()) => {
                self.inflight[kind] = Some(generation);
                return true;
            }
            Err(error) => error.0.1,
        };
        crate::diagnostic!(json!({
            "component": "session_sync",
            "kind": "sync_read.worker_restarted",
            "read": KIND_NAMES[kind],
        }));
        match spawn_worker(&self.context, &self.coordinator, kind) {
            Ok(worker) => {
                self.workers[kind] = worker;
                if self.workers[kind].send((generation, read)).is_ok() {
                    self.inflight[kind] = Some(generation);
                    return true;
                }
                false
            }
            Err(message) => {
                crate::diagnostic!(json!({
                    "component": "session_sync",
                    "kind": "sync_read.worker_unavailable",
                    "read": KIND_NAMES[kind],
                    "message": message,
                }));
                false
            }
        }
    }

    /// Notes that `answer`, asked under `generation`, arrived, so the next
    /// read of its kind can go. An answer for an older subscription frees
    /// nothing: the current one may have its own read out.
    pub(crate) fn answered(&mut self, generation: u64, answer: &SyncReadAnswer) {
        let kind = answer.kind();
        if self.inflight[kind] == Some(generation) {
            self.inflight[kind] = None;
        }
    }

    /// A new subscription starts on fresh workers: the old ones finish the
    /// read they hold, for a subscription that has ended, and exit.
    pub(crate) fn reset(&mut self) -> Result<(), String> {
        *self = Self::start(&self.context, self.coordinator.clone())?;
        Ok(())
    }
}

fn spawn_worker(
    context: &SessionSyncContext,
    coordinator: &Sender<CoordinatorMessage>,
    kind: usize,
) -> Result<Sender<(u64, SyncRead)>, String> {
    let (requests, receiver) = channel::<(u64, SyncRead)>();
    let context = context.clone();
    let coordinator = coordinator.clone();
    thread::Builder::new()
        .name(format!("herdr-core-sync-{}", KIND_NAMES[kind]))
        .spawn(move || {
            while let Ok((generation, read)) = receiver.recv() {
                // A read that panics still answers, as a failure of its kind,
                // so its kind is not left in flight with no worker behind it.
                let started_at = Instant::now();
                let answer =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&context, read)))
                        .unwrap_or_else(|_| {
                            crate::diagnostic!(json!({
                                "component": "session_sync",
                                "kind": "sync_read.panicked",
                                "target": context.log_target(),
                                "read": KIND_NAMES[kind],
                                "generation": generation,
                            }));
                            failed_answer(kind, started_at)
                        });
                if coordinator
                    .send(CoordinatorMessage::SyncRead { generation, answer })
                    .is_err()
                {
                    return;
                }
            }
        })
        .map_err(|error| format!("sync read worker could not be started: {error}"))?;
    Ok(requests)
}

fn failed_answer(kind: usize, started_at: Instant) -> SyncReadAnswer {
    let failure = || SessionFetchError::Stale("the sync read panicked".to_string());
    match kind {
        0 => SyncReadAnswer::Agents {
            started_at,
            result: Err(failure()),
        },
        1 => SyncReadAnswer::PaneCwds(Err(failure())),
        2 => SyncReadAnswer::ActiveTabs(Err(failure())),
        // Which panes the panicked read had reached is not known; none is
        // read again before the next agent list asks for it.
        _ => SyncReadAnswer::Launches(Vec::new()),
    }
}

fn run(context: &SessionSyncContext, read: SyncRead) -> SyncReadAnswer {
    match read {
        SyncRead::Agents => {
            let started_at = Instant::now();
            SyncReadAnswer::Agents {
                started_at,
                result: fetch_agents(context),
            }
        }
        SyncRead::PaneCwds(pane_ids) => SyncReadAnswer::PaneCwds(
            pane_ids
                .into_iter()
                .map(|pane_id| fetch_pane_cwd(context, &pane_id).map(|cwd| (pane_id, cwd)))
                .collect(),
        ),
        SyncRead::ActiveTabs(workspace_ids) => SyncReadAnswer::ActiveTabs(
            workspace_ids
                .into_iter()
                .map(|workspace_id| {
                    fetch_workspace_active_tab(context, &workspace_id)
                        .map(|tab_id| (workspace_id, tab_id))
                })
                .collect(),
        ),
        SyncRead::Launches(pane_ids) => SyncReadAnswer::Launches(
            pane_ids
                .into_iter()
                .map(|pane_id| {
                    let read = fetch_agent_launch(context, &pane_id);
                    (pane_id, read)
                })
                .collect(),
        ),
    }
}
