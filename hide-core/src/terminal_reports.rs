//! What every node says about its terminals reaches the runtime here: a
//! node's service sends a report from its own threads, sometimes while the
//! runtime lock is held by the control that caused it, so sending never
//! waits for that lock; one thread takes what has arrived into the runtime
//! in a batch and announces once (PRD core-host-node-terminal D-05).

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread;

use hide_node_link::terminal::{ReportSink, TerminalReport};

use crate::handle::ChangeNotifier;
use crate::runtime::Runtime;

/// Reports waiting for the runtime. Every report is a pane's state change, a
/// coalesced input fact or an armed frame, so this many waiting means the
/// runtime lock has been held for a long time. Past it a report takes the
/// place of the waiting one about the same subject
/// ([`TerminalReport::same_subject`]), so what the core keeps state from is
/// never lost and the queue grows only by reports about a subject none
/// waiting names.
const WAITING_LIMIT: usize = 16_384;

/// Subjects are panes, pastes and creations, each capped at its node, so
/// this many waiting past the limit is a node naming subjects without
/// bound: the report is dropped and logged (rule 15).
const SUBJECT_LIMIT: usize = 4 * WAITING_LIMIT;

#[derive(Default)]
struct Waiting {
    reports: VecDeque<TerminalReport>,
    /// The pump is stopping or gone: it takes what is waiting, no more.
    closed: bool,
}

#[derive(Default)]
struct Queue {
    waiting: Mutex<Waiting>,
    arrived: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// No report is taken from now on; the pump, if one runs, still applies
    /// what was already waiting and then ends.
    fn stop(&self) {
        self.lock().closed = true;
        self.arrived.notify_all();
    }

    /// Nothing reads reports any more.
    fn close(&self) {
        let mut waiting = self.lock();
        waiting.closed = true;
        waiting.reports.clear();
    }
}

/// The sending half, given to the node services and the router.
pub struct ReportChannel {
    queue: Arc<Queue>,
}

/// The receiving half, given to [`crate::Core::create`].
pub struct TerminalReports {
    queue: Option<Arc<Queue>>,
}

impl Drop for TerminalReports {
    fn drop(&mut self) {
        if let Some(queue) = self.queue.take() {
            queue.close();
        }
    }
}

/// A connected pair: the routes report into the first, the core reads the
/// second.
pub fn terminal_reports() -> (ReportChannel, TerminalReports) {
    let queue = Arc::new(Queue::default());
    (
        ReportChannel {
            queue: Arc::clone(&queue),
        },
        TerminalReports { queue: Some(queue) },
    )
}

impl ReportSink for ReportChannel {
    fn report(&self, report: TerminalReport) {
        let mut waiting = self.queue.lock();
        if waiting.closed {
            return;
        }
        if waiting.reports.len() >= WAITING_LIMIT {
            let earlier = waiting
                .reports
                .iter()
                .rposition(|waiting| waiting.same_subject(&report));
            if let Some(at) = earlier {
                let earlier = waiting.reports.remove(at).expect("found above");
                match earlier.superseded_by(report) {
                    (report, true) => waiting.reports.insert(at, report),
                    (report, false) => waiting.reports.push_back(report),
                }
                return;
            }
            if waiting.reports.len() >= SUBJECT_LIMIT {
                drop(waiting);
                crate::diagnostic!(serde_json::json!({
                    "kind": "terminal.report_dropped",
                    "pane_id": report_pane(&report),
                    "waiting": SUBJECT_LIMIT,
                }));
                return;
            }
        }
        waiting.reports.push_back(report);
        drop(waiting);
        self.queue.arrived.notify_one();
    }
}

fn report_pane(report: &TerminalReport) -> Option<String> {
    let mut report = report.clone();
    report.pane_mut().map(|pane| pane.clone())
}

/// The thread that applies reports, ended and joined with the core.
pub(crate) struct ReportPump {
    queue: Arc<Queue>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ReportPump {
    pub(crate) fn spawn(
        mut reports: TerminalReports,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) -> std::io::Result<Self> {
        let queue = reports.queue.take().expect("a report queue is pumped once");
        let pumped = Arc::clone(&queue);
        let worker = thread::Builder::new()
            .name("terminal-reports".into())
            .spawn(move || {
                loop {
                    let (batch, stopping) = {
                        let mut waiting = pumped.lock();
                        while waiting.reports.is_empty() && !waiting.closed {
                            waiting = pumped
                                .arrived
                                .wait(waiting)
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                        }
                        (
                            Vec::from(std::mem::take(&mut waiting.reports)),
                            waiting.closed,
                        )
                    };
                    let Some(runtime) = runtime.upgrade() else {
                        break;
                    };
                    if !batch.is_empty()
                        && runtime
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .ingest_terminal_reports(batch)
                    {
                        notifier.notify();
                    }
                    if stopping {
                        break;
                    }
                }
                pumped.close();
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                queue.close();
                return Err(error);
            }
        };
        Ok(Self {
            queue,
            worker: Some(worker),
        })
    }
}

impl Drop for ReportPump {
    fn drop(&mut self) {
        self.queue.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pane: &str, at: u64) -> TerminalReport {
        TerminalReport::Input {
            pane: pane.to_owned(),
            at_unix_ms: at,
            submitted: false,
            focus: true,
        }
    }

    /// Past the limit a paste's result and a creation's discard are never
    /// dropped, and the queue grows only by the subjects nothing waiting
    /// names.
    #[test]
    fn past_the_limit_results_are_kept_and_the_queue_grows_only_by_new_subjects() {
        let (channel, reports) = terminal_reports();
        for at in 0..WAITING_LIMIT as u64 {
            channel.report(key("w1:p1", at));
        }
        channel.report(TerminalReport::AttachmentDelivered {
            intent: "i1".to_owned(),
            written: true,
        });
        channel.report(TerminalReport::RequestDiscarded {
            request: "req-1".to_owned(),
            reason: "cap".to_owned(),
        });
        for at in 0..10_000 {
            channel.report(key("w1:p2", at));
        }
        let waiting = reports.queue.as_ref().unwrap().lock();
        assert_eq!(waiting.reports.len(), WAITING_LIMIT + 3);
        assert!(waiting.reports.iter().any(|report| matches!(
            report,
            TerminalReport::AttachmentDelivered { written: true, .. }
        )));
        assert!(
            waiting
                .reports
                .iter()
                .any(|report| matches!(report, TerminalReport::RequestDiscarded { .. }))
        );
        assert_eq!(waiting.reports.back(), Some(&key("w1:p2", 9_999)));
    }
}
