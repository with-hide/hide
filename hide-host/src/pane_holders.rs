//! A node's side of its pane holders over its link (PRD
//! engine-bundle-3-core-owns D-22): each [`PaneCall`] the core's engine
//! sends ([`Call::Pane`]) answered from the node's [`PaneHolders`], and the
//! pane watch ([`Call::PaneWatch`]) that tells the core each pane's facts
//! and each pane that ended.
//!
//! The core's own node and a device's node both answer through here, so the
//! engine of every machine reaches its panes the same way. The holders
//! themselves live in `hide-node`, which depends on this crate; the node
//! hands them in ([`PaneSlot`]).
//!
//! [`Call::Pane`]: hide_node_link::protocol::Call::Pane
//! [`Call::PaneWatch`]: hide_node_link::protocol::Call::PaneWatch

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use hide_node_link::holders::{
    HolderError, PANE_WATCH_CAP, PANE_WATCH_QUIET, PaneAnswer, PaneCall, PaneFacts, PaneHolders,
    PaneText, PaneWatchReport,
};
use serde_json::Value;

use crate::{ErrorCode, HostError, HostResult};

/// The node's pane holders, once it runs them: the core's own node is given
/// them when it is made, a device's node when its core asks it to start
/// them (`Call::HoldersStart`). Shared by every copy of the node's
/// environment.
#[derive(Clone, Default)]
pub struct PaneSlot {
    holders: Arc<OnceLock<Arc<dyn PaneHolders>>>,
    owner: Arc<Mutex<Ownership>>,
}

#[derive(Default)]
enum Ownership {
    #[default]
    Unowned,
    Acquiring,
    Ready(Arc<dyn PaneHolders>),
    Closed,
}

impl fmt::Debug for PaneSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.holders.get().is_some() {
            "PaneSlot(running)"
        } else {
            "PaneSlot(none)"
        })
    }
}

impl PaneSlot {
    /// A slot holding `holders` already.
    pub fn holding(holders: Arc<dyn PaneHolders>) -> Self {
        let slot = Self::default();
        slot.fill(holders);
        slot
    }

    /// Puts `holders` in the slot; a slot that holds some keeps them.
    pub fn fill(&self, holders: Arc<dyn PaneHolders>) {
        let _ = self.holders.set(holders);
    }

    pub fn holders(&self) -> Option<&Arc<dyn PaneHolders>> {
        self.holders.get()
    }

    /// Stop admission, but do not release the ownership retained by an
    /// admitted worker. The next connection cannot acquire the state lock
    /// until those workers actually return, even if their answers were lost.
    pub fn close(&self) {
        let previous = {
            let mut owner = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *owner, Ownership::Closed)
        };
        drop(previous);
    }

    fn owned(&self, acquire: Option<Duration>) -> Result<Arc<dyn PaneHolders>, HolderError> {
        {
            let mut owner = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
            match &*owner {
                Ownership::Ready(holders) => return Ok(Arc::clone(holders)),
                Ownership::Unowned if acquire.is_some() => *owner = Ownership::Acquiring,
                _ => return Err(owner_unavailable()),
            }
        }
        let result = self
            .holders()
            .ok_or_else(owner_unavailable)
            .and_then(|holders| Arc::clone(holders).own(acquire.unwrap_or_default()));
        let mut owner = self.owner.lock().unwrap_or_else(PoisonError::into_inner);
        // Close can happen while acquisition or recovery blocks. It never
        // reactivates this old slot, and drops a newly acquired guard here.
        if matches!(*owner, Ownership::Closed) {
            return Err(owner_unavailable());
        }
        *owner = match &result {
            Ok(holders) => Ownership::Ready(Arc::clone(holders)),
            Err(_) => Ownership::Unowned,
        };
        result
    }

    fn running(&self) -> HostResult<&Arc<dyn PaneHolders>> {
        self.holders().ok_or_else(|| {
            HostError::new(
                ErrorCode::Unsupported,
                "This node runs no pane holders; its core starts them first",
            )
        })
    }
}

/// Answers `call` from the slot's holders. A request the holders refuse is
/// answered, with its refusal, as the holders' answer, so the core reads its
/// kind and code as the holders gave them; only a node that runs no holders
/// refuses the request itself.
pub fn answer(slot: &PaneSlot, call: PaneCall) -> HostResult<Value> {
    slot.running()?;
    let acquire = match &call {
        PaneCall::Recovered { timeout_ms } => Some(Duration::from_millis(*timeout_ms)),
        _ => None,
    };
    // This Arc, not the answer channel, owns every mutation until ask has
    // really finished. Timeout/close cannot release it from another thread.
    let answer: PaneAnswer = slot
        .owned(acquire)
        .and_then(|holders| ask(holders.as_ref(), call));
    serde_json::to_value(answer).map_err(|error| HostError::new(ErrorCode::Io, error.to_string()))
}

fn owner_unavailable() -> HolderError {
    HolderError::Refused {
        code: "pane_owner_unavailable".to_owned(),
        message: "the pane connection is closed or has not finished recovery".to_owned(),
    }
}

fn ask(holders: &dyn PaneHolders, call: PaneCall) -> PaneAnswer {
    fn value(answer: impl serde::Serialize) -> PaneAnswer {
        serde_json::to_value(answer).map_err(|error| HolderError::Unreachable {
            message: format!("the node could not write its answer: {error}"),
        })
    }
    let text = |(text, truncated)| value(PaneText { text, truncated });
    match call {
        PaneCall::Reserve {
            start,
            previous,
            request,
        } => holders.reserve(&start, previous, request).and_then(value),
        PaneCall::StartReserved {
            start,
            reserved,
            request,
        } => holders
            .start_reserved(&start, reserved, request)
            .and_then(value),
        PaneCall::Cancel { intent, request } => holders.cancel(&intent, request).and_then(value),
        PaneCall::Retire { request } => holders.retire(request).and_then(value),
        PaneCall::ShellLaunch => value(holders.shell_launch()),
        PaneCall::Terminate { pane } => holders.terminate(&pane).and_then(value),
        PaneCall::Read { pane, lines } => holders.read(&pane, lines).and_then(text),
        PaneCall::ReadStyled { pane, read } => holders.read_styled(&pane, read).and_then(text),
        PaneCall::ReadVisible { pane } => holders.read_visible(&pane).and_then(|text| {
            value(PaneText {
                text,
                truncated: false,
            })
        }),
        PaneCall::Scroll { pane, to } => holders.scroll(&pane, to).and_then(value),
        PaneCall::Processes { pane } => holders.processes(&pane).and_then(value),
        PaneCall::Proof { pane } => holders.proof(&pane).and_then(value),
        PaneCall::Input { pane, input } => holders.input(&pane, input).and_then(value),
        PaneCall::Letter {
            pane,
            text,
            submit,
            gap_ms,
        } => holders
            .letter(&pane, text, submit, Duration::from_millis(gap_ms))
            .and_then(value),
        PaneCall::Recovered { timeout_ms } => holders
            .recovered(Duration::from_millis(timeout_ms))
            .and_then(value),
        PaneCall::SetRules { pane, rules } => holders.set_rules(&pane, rules).and_then(value),
    }
}

/// What a watch has to say and has not said yet.
#[derive(Default)]
struct Waiting {
    reports: VecDeque<PaneWatchReport>,
    /// Facts dropped past [`PANE_WATCH_CAP`] since the last record of it.
    dropped: u64,
}

/// Where the holders' sinks put what the watch says, until the watch ends.
#[derive(Default)]
struct Queue {
    waiting: Mutex<Waiting>,
    arrived: Condvar,
    ended: AtomicBool,
}

impl Queue {
    /// A pane's facts take the place of any facts of it still waiting, so a
    /// watch that falls behind says each pane's latest once.
    fn facts(&self, pane: &str, facts: &PaneFacts) {
        if self.ended.load(Ordering::Acquire) {
            return;
        }
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let report = PaneWatchReport::Facts {
            pane: pane.to_owned(),
            facts: facts.clone(),
        };
        let same = waiting.reports.iter_mut().find(
            |waiting| matches!(waiting, PaneWatchReport::Facts { pane: theirs, .. } if theirs == pane),
        );
        match same {
            Some(same) => *same = report,
            None => {
                push(&mut waiting, report);
            }
        }
        drop(waiting);
        self.arrived.notify_one();
    }

    fn ended(&self, pane: &str) {
        if self.ended.load(Ordering::Acquire) {
            return;
        }
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let told = waiting.reports.iter().any(
            |waiting| matches!(waiting, PaneWatchReport::Ended { pane: theirs } if theirs == pane),
        );
        if !told {
            push(
                &mut waiting,
                PaneWatchReport::Ended {
                    pane: pane.to_owned(),
                },
            );
        }
        drop(waiting);
        self.arrived.notify_one();
    }

    /// The next report, or [`PaneWatchReport::Quiet`] once
    /// [`PANE_WATCH_QUIET`] passed with none.
    fn next(&self) -> (PaneWatchReport, u64) {
        let waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut waiting, _) = self
            .arrived
            .wait_timeout_while(waiting, PANE_WATCH_QUIET, |waiting| {
                waiting.reports.is_empty()
            })
            .unwrap_or_else(PoisonError::into_inner);
        let dropped = std::mem::take(&mut waiting.dropped);
        (
            waiting
                .reports
                .pop_front()
                .unwrap_or(PaneWatchReport::Quiet),
            dropped,
        )
    }
}

/// Keeps `report` waiting; past the cap the oldest facts make room, and when
/// only ends wait, new facts are dropped, both counted. An end is never
/// dropped, since the engine learns of it nowhere else while the link
/// lasts: ends wait one per pane, so they are bounded by the panes the node
/// ran while the watcher did not read, which needs that same engine to
/// start them.
fn push(waiting: &mut Waiting, report: PaneWatchReport) {
    if waiting.reports.len() >= PANE_WATCH_CAP {
        let oldest_facts = waiting
            .reports
            .iter()
            .position(|waiting| matches!(waiting, PaneWatchReport::Facts { .. }));
        match (oldest_facts, &report) {
            (Some(oldest), _) => {
                waiting.reports.remove(oldest);
                waiting.dropped += 1;
            }
            (None, PaneWatchReport::Facts { .. }) => {
                waiting.dropped += 1;
                return;
            }
            (None, _) => {}
        }
    }
    waiting.reports.push_back(report);
}

/// Follows the slot's panes until `progress` answers a report with false or
/// `stop` is raised: each pane's facts and each pane that ended as the
/// holders tell them, and a quiet report every [`PANE_WATCH_QUIET`]. A later
/// watch takes the holders' sinks from this one, which then hears nothing
/// more and ends at its next quiet report.
pub fn watch(
    slot: &PaneSlot,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(Value) -> bool,
) -> HostResult<Value> {
    let holders = slot.running()?;
    let queue = Arc::new(Queue::default());
    let facts = Arc::downgrade(&queue);
    holders.watch_facts(Arc::new(move |pane: &str, told: &PaneFacts| {
        if let Some(queue) = facts.upgrade() {
            queue.facts(pane, told);
        }
    }));
    let ends = Arc::downgrade(&queue);
    holders.watch_ends(Arc::new(move |pane: &str| {
        if let Some(queue) = ends.upgrade() {
            queue.ended(pane);
        }
    }));
    let result = loop {
        if stop.load(Ordering::Acquire) {
            break Ok(Value::Null);
        }
        let (report, dropped) = queue.next();
        if dropped > 0 {
            // The node's error stream is its diagnostic log.
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "pane_watch",
                    "kind": "pane_watch.facts_dropped",
                    "dropped": dropped,
                    "cap": PANE_WATCH_CAP,
                })
            );
        }
        let report = match serde_json::to_value(report) {
            Ok(report) => report,
            Err(error) => break Err(HostError::new(ErrorCode::Io, error.to_string())),
        };
        if !progress(report) {
            break Ok(Value::Null);
        }
    };
    queue.ended.store(true, Ordering::Release);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::holders::{
        HolderProcesses, HolderProof, HolderRecovery, HolderScroll, HolderStart, PaneEnded,
        PaneFactsSink, PaneInput, ScreenRules, StyledRead,
    };

    /// Holders that refuse every start under a code of their own and
    /// remember the sinks they were given.
    #[derive(Default)]
    struct Holders {
        facts: Mutex<Option<PaneFactsSink>>,
        /// Told when a watch hands its facts sink.
        given: std::sync::Condvar,
        ends: Mutex<Option<PaneEnded>>,
    }

    impl PaneHolders for Holders {
        fn own(self: Arc<Self>, _: Duration) -> Result<Arc<dyn PaneHolders>, HolderError> {
            Ok(self)
        }
        fn reserve(
            &self,
            _: &HolderStart,
            _: Option<u64>,
            _: hide_node_link::holders::HolderRequest,
        ) -> Result<hide_node_link::holders::HolderReservation, HolderError> {
            Err(HolderError::Refused {
                code: "holder_cap".to_owned(),
                message: "128 holders run".to_owned(),
            })
        }
        fn terminate(&self, pane: &str) -> Result<(), HolderError> {
            Err(HolderError::NoPane {
                message: format!("no {pane}"),
            })
        }
        fn read(&self, _: &str, lines: u16) -> Result<(String, bool), HolderError> {
            Ok((format!("{lines} rows"), true))
        }
        fn read_styled(&self, _: &str, _: StyledRead) -> Result<(String, bool), HolderError> {
            unreachable!()
        }
        fn read_visible(&self, _: &str) -> Result<String, HolderError> {
            unreachable!()
        }
        fn scroll(&self, _: &str, _: Option<u64>) -> Result<HolderScroll, HolderError> {
            unreachable!()
        }
        fn processes(&self, _: &str) -> Result<HolderProcesses, HolderError> {
            unreachable!()
        }
        fn proof(&self, _: &str) -> Result<HolderProof, HolderError> {
            Ok(HolderProof {
                incarnation: 3,
                program: 41,
                started: 7,
            })
        }
        fn input(&self, _: &str, _: PaneInput) -> Result<(), HolderError> {
            Ok(())
        }
        fn letter(
            &self,
            _: &str,
            _: Vec<PaneInput>,
            _: PaneInput,
            _: Duration,
        ) -> Result<(), HolderError> {
            Ok(())
        }
        fn recovered(&self, _: Duration) -> Result<HolderRecovery, HolderError> {
            unreachable!()
        }
        fn watch_ends(&self, ended: PaneEnded) {
            *self.ends.lock().unwrap() = Some(ended);
        }
        fn watch_facts(&self, sink: PaneFactsSink) {
            *self.facts.lock().unwrap() = Some(sink);
            self.given.notify_all();
        }
        fn set_rules(&self, _: &str, _: Option<ScreenRules>) -> Result<(), HolderError> {
            Ok(())
        }
    }

    fn answered(slot: &PaneSlot, call: PaneCall) -> PaneAnswer {
        serde_json::from_value(answer(slot, call).unwrap()).unwrap()
    }

    /// The core reads a refusal with the kind and code the holders gave it,
    /// and an answer in the shape the request names.
    #[test]
    fn a_pane_request_is_answered_with_the_holders_own_answer_or_refusal() {
        let slot = PaneSlot::holding(Arc::new(Holders::default()));
        slot.owned(Some(Duration::ZERO)).unwrap();
        let start = HolderStart {
            pane: "w1:p1".to_owned(),
            program: None,
            cwd: "/".to_owned(),
            env: Vec::new(),
            rows: 24,
            cols: 80,
            notice: None,
        };
        assert_eq!(
            answered(
                &slot,
                PaneCall::Reserve {
                    start,
                    previous: None,
                    request: hide_node_link::holders::HolderRequest {
                        owner: 1,
                        number: 1
                    }
                }
            ),
            Err(HolderError::Refused {
                code: "holder_cap".to_owned(),
                message: "128 holders run".to_owned(),
            })
        );
        assert!(matches!(
            answered(
                &slot,
                PaneCall::Terminate {
                    pane: "w1:p2".to_owned()
                }
            ),
            Err(HolderError::NoPane { .. })
        ));
        let read = answered(
            &slot,
            PaneCall::Read {
                pane: "w1:p1".to_owned(),
                lines: 5,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<PaneText>(read).unwrap(),
            PaneText {
                text: "5 rows".to_owned(),
                truncated: true,
            }
        );
        let proof = answered(
            &slot,
            PaneCall::Proof {
                pane: "w1:p1".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_value::<HolderProof>(proof)
                .unwrap()
                .program,
            41
        );
    }

    #[test]
    fn a_closed_connection_refuses_a_late_start_and_cannot_reacquire_ownership() {
        let slot = PaneSlot::holding(Arc::new(Holders::default()));
        slot.owned(Some(Duration::ZERO)).unwrap();
        slot.close();
        for call in [
            PaneCall::Terminate {
                pane: "w1:p1".to_owned(),
            },
            PaneCall::Recovered { timeout_ms: 1 },
        ] {
            assert!(matches!(
                answered(&slot, call),
                Err(HolderError::Refused { code, .. }) if code == "pane_owner_unavailable"
            ));
        }
    }

    /// A node that runs no holders refuses the request itself, so the core
    /// never reads it as a pane that is not there.
    #[test]
    fn a_node_without_holders_refuses_pane_requests() {
        let refused = answer(&PaneSlot::default(), PaneCall::ShellLaunch).unwrap_err();
        assert_eq!(refused.code, ErrorCode::Unsupported);
        let refused =
            watch(&PaneSlot::default(), &AtomicBool::new(false), &mut |_| true).unwrap_err();
        assert_eq!(refused.code, ErrorCode::Unsupported);
    }

    fn titled(title: &str) -> PaneFacts {
        PaneFacts {
            title: Some(title.to_owned()),
            ..PaneFacts::default()
        }
    }

    /// A watch that fell behind says each pane's latest facts once, in the
    /// place of its first, and every end after the facts told before it.
    #[test]
    fn a_watch_behind_says_each_pane_s_latest_facts_once_and_every_end() {
        let queue = Queue::default();
        queue.facts("w1:p1", &titled("one"));
        queue.facts("w1:p2", &titled("other"));
        queue.facts("w1:p1", &titled("two"));
        queue.ended("w1:p2");
        let said: Vec<_> = (0..4).map(|_| queue.next().0).collect();
        assert_eq!(
            said,
            [
                PaneWatchReport::Facts {
                    pane: "w1:p1".to_owned(),
                    facts: titled("two"),
                },
                PaneWatchReport::Facts {
                    pane: "w1:p2".to_owned(),
                    facts: titled("other"),
                },
                PaneWatchReport::Ended {
                    pane: "w1:p2".to_owned(),
                },
                PaneWatchReport::Quiet,
            ]
        );
    }

    /// Past the cap the oldest facts make room and are counted; no end is
    /// dropped while facts wait.
    #[test]
    fn a_full_watch_drops_the_oldest_facts_before_any_end() {
        let queue = Queue::default();
        queue.ended("w1:p0");
        for pane in 0..PANE_WATCH_CAP {
            queue.facts(&format!("w2:p{pane}"), &titled("t"));
        }
        let (first, dropped) = queue.next();
        assert_eq!(
            first,
            PaneWatchReport::Ended {
                pane: "w1:p0".to_owned()
            }
        );
        assert_eq!(dropped, 1);
    }

    /// A queue full of ends keeps every end and drops new facts.
    #[test]
    fn a_watch_full_of_ends_keeps_them_and_drops_new_facts() {
        let queue = Queue::default();
        for pane in 0..PANE_WATCH_CAP {
            queue.ended(&format!("w1:p{pane}"));
        }
        queue.ended("w1:p0");
        queue.facts("w2:p1", &titled("t"));
        queue.ended("w3:p1");
        let mut ends = 0;
        let mut dropped = 0;
        loop {
            let (report, gone) = queue.next();
            dropped += gone;
            match report {
                PaneWatchReport::Ended { .. } => ends += 1,
                PaneWatchReport::Quiet => break,
                PaneWatchReport::Facts { .. } => panic!("facts past a full queue of ends"),
            }
        }
        assert_eq!(ends, PANE_WATCH_CAP + 1);
        assert_eq!(dropped, 1);
    }

    /// The watch hears the holders' sinks and ends when its caller stops
    /// listening.
    #[test]
    fn a_watch_tells_what_the_holders_tell_until_its_caller_stops() {
        let holders = Arc::new(Holders::default());
        let slot = PaneSlot::holding(holders.clone());
        let (heard, reports) = std::sync::mpsc::channel();
        let watching = std::thread::spawn(move || {
            watch(&slot, &AtomicBool::new(false), &mut |report| {
                let report: PaneWatchReport = serde_json::from_value(report).unwrap();
                let go_on = report == PaneWatchReport::Quiet;
                heard.send(report).unwrap();
                go_on
            })
        });
        let (given, _) = holders
            .given
            .wait_timeout_while(
                holders.facts.lock().unwrap(),
                Duration::from_secs(5),
                |facts| facts.is_none(),
            )
            .unwrap();
        let facts = given.clone().expect("the watch hands its facts sink");
        drop(given);
        facts("w1:p1", &titled("one"));
        let told = loop {
            match reports.recv_timeout(Duration::from_secs(5)).unwrap() {
                PaneWatchReport::Quiet => continue,
                told => break told,
            }
        };
        assert_eq!(
            told,
            PaneWatchReport::Facts {
                pane: "w1:p1".to_owned(),
                facts: titled("one"),
            }
        );
        assert_eq!(watching.join().unwrap().unwrap(), Value::Null);
    }
}
