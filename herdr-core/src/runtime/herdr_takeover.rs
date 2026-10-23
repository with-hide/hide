//! What the screen offers for the operator's own older Herdr server on this
//! machine's socket, and the move itself (PRD herdr-takeover).
//!
//! `hided` installs the [`HerdrTakeover`] that does the work (one that finds
//! nothing when the daemon does not run from a desktop package with its own
//! Herdr); the core asks
//! it on a worker of its own, one call at a time, whenever this machine's
//! Herdr connection changes state, and when the operator asks for the move.
//! Nothing moves without that press (D-14): the look only decides what is
//! offered, `status.herdr_takeover`, and whether the first-run wizard opens
//! on its Herdr step.

use std::sync::{Arc, Mutex, Weak};
use std::thread;

use super::Runtime;
use crate::handle::ChangeNotifier;
use crate::herdr_takeover::{HerdrTakeover, Look, Moved, OfferKind, Versions};
use crate::model::{FirstRunStep, HerdrTakeoverSnapshot, HerdrVersionsSnapshot, PathHerdrSnapshot};

/// The installed takeover and the one call it is making.
#[derive(Default)]
pub(super) struct Takeover {
    worker: Option<Worker>,
    looking: bool,
    /// The connection changed again while a look or the move ran.
    look_again: bool,
    /// A look has answered since the takeover was installed.
    answered: bool,
    moving: bool,
    /// Bumped by each move press, so a look that started before it, and so
    /// read the server the move replaces, is dropped when it answers.
    epoch: u64,
    /// The connection state the last look was asked for.
    seen_state: Option<String>,
    /// The wizard was closed with the move unanswered (B9): it does not open
    /// again on its own until the server moves or is gone, or Hide starts
    /// again.
    closed_unanswered: bool,
    /// The server the last look offered something for.
    offered_server: Option<Server>,
    /// The server the desktop host's restart could not stop: its restart
    /// step stays on that failure, with Not now, while a server of that pid
    /// (where the system names one) and version answers, until a look finds
    /// none (D-29).
    stop_failed: Option<Server>,
}

/// A server as a look reads it: its process, where the system says, and its
/// version.
type Server = (Option<u32>, String);

#[derive(Clone)]
struct Worker {
    takeover: Arc<dyn HerdrTakeover>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

/// What the operator pressed (`herdr_takeover {action}`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TakeoverAction {
    /// Move the server now; a second press while it moves is the same one.
    Move,
    /// Leave the step without moving: offered on a server Hide reads as it
    /// is, and on any server once its move or restart failed (D-16, D-29).
    Later,
    /// The moved server's step was read.
    Continue,
    /// Look at the server again: the desktop host restarted it, which the
    /// connection state may not show when the stop and start are quick.
    Look,
    /// The desktop host's restart did not stop the server, so nothing
    /// changed: the restart fails as a move does, and Not now appears (D-29).
    StopFailed,
}

impl Runtime {
    pub(crate) fn install_herdr_takeover(
        &mut self,
        takeover: Arc<dyn HerdrTakeover>,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) {
        self.herdr_takeover.worker = Some(Worker {
            takeover,
            runtime,
            notifier,
        });
        self.herdr_takeover.seen_state = Some(self.snapshot.status.herdr.state.clone());
        self.look_at_herdr_server();
    }

    /// Looks at the server again when this machine's Herdr connection moved
    /// to another state: a server that started, stopped or was replaced.
    pub(super) fn herdr_connection_changed(&mut self) {
        let state = &self.snapshot.status.herdr.state;
        if self.herdr_takeover.worker.is_none()
            || self.herdr_takeover.seen_state.as_ref() == Some(state)
        {
            return;
        }
        self.herdr_takeover.seen_state = Some(state.clone());
        self.look_at_herdr_server();
    }

    fn look_at_herdr_server(&mut self) {
        let takeover = &mut self.herdr_takeover;
        let Some(worker) = takeover.worker.clone() else {
            return;
        };
        // Never beside the move, which is changing the server it would read.
        if takeover.looking || takeover.moving {
            takeover.look_again = true;
            return;
        }
        takeover.looking = true;
        let epoch = takeover.epoch;
        let spawned = thread::Builder::new()
            .name("herdr-core-herdr-takeover-look".into())
            .spawn(move || {
                let (look, versions) = bounded_look(&worker.takeover, LOOK_WITHIN);
                answer(&worker, |runtime| {
                    let versions_changed =
                        versions.is_some_and(|versions| runtime.ingest_herdr_versions(versions));
                    runtime.ingest_herdr_look_from(epoch, look) | versions_changed
                });
            });
        if let Err(error) = spawned {
            self.ingest_herdr_look(Look::Unreadable {
                stage: "worker".into(),
                detail: error.to_string(),
            });
        }
    }

    pub(super) fn ingest_herdr_look(&mut self, look: Look) -> bool {
        self.ingest_herdr_look_from(self.herdr_takeover.epoch, look)
    }

    /// A look's answer, dropped when a move was pressed after it started:
    /// it read the server the move replaces, and would unlock the button or
    /// take the step away while the move runs.
    pub(super) fn ingest_herdr_look_from(&mut self, epoch: u64, look: Look) -> bool {
        let takeover = &mut self.herdr_takeover;
        takeover.looking = false;
        if epoch != takeover.epoch {
            if !takeover.moving && std::mem::take(&mut takeover.look_again) {
                self.look_at_herdr_server();
            }
            return false;
        }
        takeover.answered = true;
        let current = self.snapshot.status.herdr_takeover.clone();
        let next = match &look {
            // A server Hide cannot use and could not read says so, with a
            // look again, rather than leaving the wizard as if there were
            // nothing to move (B8); one Hide reads keeps the log line alone.
            Look::Unreadable { .. }
                if !self.herdr_usable()
                    && current.as_ref().is_none_or(|shown| shown.stage != "moved") =>
            {
                Some(HerdrTakeoverSnapshot {
                    kind: OfferKind::Look.as_str().to_owned(),
                    stage: "failed".to_owned(),
                    server_version: String::new(),
                    version: self
                        .snapshot
                        .status
                        .herdr_versions
                        .as_ref()
                        .map(|versions| versions.hide.clone())
                        .unwrap_or_default(),
                    same_protocol: false,
                    failure: Some(
                        crate::herdr_takeover::FailureCause::Unreadable
                            .as_str()
                            .to_owned(),
                    ),
                    path_herdr_behind: None,
                })
            }
            Look::Nothing | Look::Unreadable { .. } => {
                // Only a server that is gone takes a restart's stop failure
                // with it; one that did not answer in time may still be there.
                if matches!(look, Look::Nothing) {
                    self.herdr_takeover.offered_server = None;
                    self.herdr_takeover.stop_failed = None;
                }
                // A move's step the operator has not read yet stays.
                current.filter(|shown| shown.stage == "moved")
            }
            Look::Offer(offer) => {
                let server = (offer.server_pid, offer.server_version.clone());
                let takeover = &mut self.herdr_takeover;
                if takeover.stop_failed.as_ref() != Some(&server) {
                    takeover.stop_failed = None;
                }
                let stop_failed =
                    takeover.stop_failed.is_some() && offer.kind == OfferKind::Restart;
                takeover.offered_server = Some(server);
                let failure = offer
                    .failed_before
                    .map(|cause| cause.as_str())
                    .or(stop_failed.then_some(STOP_FAILED));
                Some(HerdrTakeoverSnapshot {
                    kind: offer.kind.as_str().to_owned(),
                    stage: if failure.is_some() {
                        "failed"
                    } else {
                        "offered"
                    }
                    .to_owned(),
                    server_version: offer.server_version.clone(),
                    version: offer.version.clone(),
                    same_protocol: offer.server_protocol
                        == hide_herdr_client::HERDR_PROTOCOL_REVISION,
                    failure: failure.map(str::to_owned),
                    path_herdr_behind: None,
                })
            }
        };
        if next != self.snapshot.status.herdr_takeover || matches!(look, Look::Unreadable { .. }) {
            crate::diagnostic!(look_record(&look));
        }
        let mut changed = self.show_herdr_takeover(next);
        if matches!(look, Look::Nothing) {
            // The server was restarted or stopped away.
            changed |= self.ask_the_unanswered_agent_question();
        }
        if std::mem::take(&mut self.herdr_takeover.look_again) {
            self.look_at_herdr_server();
        }
        changed |= self.settle_first_run_herdr();
        changed
    }

    /// What a look read about this machine's Herdr programs, for Settings >
    /// About; a daemon without a Herdr of its package's own names none.
    pub(super) fn ingest_herdr_versions(&mut self, versions: Versions) -> bool {
        let next = versions.hide.map(|hide| HerdrVersionsSnapshot {
            hide,
            path_herdr: versions.path_herdr.map(|path| PathHerdrSnapshot {
                behind: path.behind(),
                version: path.version,
                update_command: path.update,
            }),
        });
        if self.snapshot.status.herdr_versions == next {
            return false;
        }
        self.snapshot.status.herdr_versions = next;
        true
    }

    /// Hide reads the server on this machine's socket: its snapshot answers
    /// on this build's protocol, or the last one that did is on screen while
    /// the stream reconnects.
    fn herdr_usable(&self) -> bool {
        matches!(
            self.snapshot.status.herdr.state.as_str(),
            "connected" | "stale"
        )
    }

    pub(super) fn herdr_takeover_action(&mut self, action: TakeoverAction) -> bool {
        if action == TakeoverAction::Look {
            if self.herdr_takeover.worker.is_none() {
                return refused(action, "this app cannot look at Herdr");
            }
            self.look_at_herdr_server();
            return false;
        }
        let Some(shown) = self.snapshot.status.herdr_takeover.clone() else {
            return refused(action, "nothing is offered");
        };
        match action {
            TakeoverAction::Move => {
                if self.herdr_takeover.moving {
                    return false;
                }
                if shown.kind != OfferKind::Move.as_str()
                    || !["offered", "failed"].contains(&shown.stage.as_str())
                {
                    return refused(action, "no move is offered");
                }
                let Some(worker) = self.herdr_takeover.worker.clone() else {
                    return refused(action, "this app cannot move Herdr");
                };
                self.herdr_takeover.moving = true;
                self.herdr_takeover.epoch += 1;
                let spawned = thread::Builder::new()
                    .name("herdr-core-herdr-takeover-move".into())
                    .spawn(move || {
                        // A move that panics still ends, so the step is not
                        // held on "Moving…" and looks resume.
                        let moved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            worker.takeover.hand_over()
                        }))
                        .unwrap_or_else(|_| Moved::Failed {
                            cause: crate::herdr_takeover::FailureCause::Unreadable,
                            stage: "panic".into(),
                            detail: "the move stopped unexpectedly".into(),
                        });
                        answer(&worker, |runtime| runtime.ingest_herdr_move(moved));
                    });
                if let Err(error) = spawned {
                    self.herdr_takeover.moving = false;
                    return self.ingest_herdr_move(Moved::Failed {
                        cause: crate::herdr_takeover::FailureCause::Unreadable,
                        stage: "worker".into(),
                        detail: error.to_string(),
                    });
                }
                self.show_herdr_takeover(Some(HerdrTakeoverSnapshot {
                    stage: "moving".into(),
                    failure: None,
                    ..shown
                }))
            }
            TakeoverAction::Later => {
                let movable = [
                    OfferKind::Move.as_str(),
                    OfferKind::Restart.as_str(),
                    OfferKind::Look.as_str(),
                ]
                .contains(&shown.kind.as_str());
                if !movable
                    || self.herdr_takeover.moving
                    || shown.stage == "moved"
                    || !(shown.same_protocol || shown.stage == "failed")
                {
                    return refused(action, "this server cannot be left for later");
                }
                if self.first_run_step() != Some(FirstRunStep::Herdr) {
                    return false;
                }
                if shown.same_protocol {
                    return self.move_first_run(FirstRunStep::Herdr, FirstRunStep::Agents);
                }
                // A server Hide cannot read whose move failed: the wizard
                // closes with the move unanswered, and Overview and Help >
                // Set up Hide offer it again (B9).
                self.snapshot.ui_state.first_run = None;
                self.herdr_takeover.closed_unanswered = true;
                self.persist_ui_state();
                crate::diagnostic!(serde_json::json!({
                    "component": "first_run",
                    "kind": "first_run.closed_herdr_unmoved",
                }));
                true
            }
            TakeoverAction::Look => false,
            TakeoverAction::StopFailed => {
                if shown.kind != OfferKind::Restart.as_str() {
                    return refused(action, "no restart is offered");
                }
                self.herdr_takeover.stop_failed = self.herdr_takeover.offered_server.clone();
                crate::diagnostic!(serde_json::json!({
                    "component": "herdr",
                    "kind": "herdr.takeover_stop_failed",
                    "server_version": shown.server_version,
                }));
                self.show_herdr_takeover(Some(HerdrTakeoverSnapshot {
                    stage: "failed".into(),
                    failure: Some(STOP_FAILED.into()),
                    ..shown
                }))
            }
            TakeoverAction::Continue => {
                if shown.stage != "moved" {
                    return refused(action, "nothing was moved");
                }
                let changed = self.show_herdr_takeover(None);
                changed | self.settle_first_run_herdr()
            }
        }
    }

    pub(super) fn ingest_herdr_move(&mut self, moved: Moved) -> bool {
        self.herdr_takeover.moving = false;
        crate::diagnostic!(move_record(&moved));
        let Some(shown) = self.snapshot.status.herdr_takeover.clone() else {
            return false;
        };
        let mut changed = match moved {
            Moved::Done {
                path_herdr_behind, ..
            } => {
                let next = path_herdr_behind.map(|behind| HerdrTakeoverSnapshot {
                    stage: "moved".into(),
                    failure: None,
                    path_herdr_behind: Some(behind),
                    ..shown
                });
                self.show_herdr_takeover(next) | self.ask_the_unanswered_agent_question()
            }
            Moved::Failed { cause, .. } => self.show_herdr_takeover(Some(HerdrTakeoverSnapshot {
                stage: "failed".into(),
                failure: Some(cause.as_str().to_owned()),
                ..shown
            })),
        };
        // The server the socket answers for now decides what is offered.
        self.herdr_takeover.look_again = false;
        self.look_at_herdr_server();
        changed |= self.settle_first_run_herdr();
        changed
    }

    /// The wizard closed with the move or restart unanswered opens again on
    /// the agent question it left, once that server has moved or is gone.
    fn ask_the_unanswered_agent_question(&mut self) -> bool {
        if std::mem::take(&mut self.herdr_takeover.closed_unanswered)
            && self.snapshot.ui_state.agent_onboarding
                == Some(crate::model::AgentOnboarding::Pending)
            && self.open_first_run()
        {
            self.persist_ui_state();
            return true;
        }
        false
    }

    /// Whether the first-run wizard has a Herdr step to show or wait for:
    /// a look has not answered yet, or it offered a move or a restart that
    /// has not happened.
    pub(super) fn first_run_herdr_pending(&self) -> bool {
        let takeover = &self.herdr_takeover;
        if takeover.worker.is_none() {
            return false;
        }
        if !takeover.answered || takeover.moving {
            return true;
        }
        self.snapshot
            .status
            .herdr_takeover
            .as_ref()
            .is_some_and(|shown| {
                shown.stage == "moved"
                    || [
                        OfferKind::Move.as_str(),
                        OfferKind::Restart.as_str(),
                        OfferKind::Look.as_str(),
                    ]
                    .contains(&shown.kind.as_str())
            })
    }

    #[cfg(test)]
    pub(super) fn herdr_takeover_looking_for_test(&self) -> bool {
        self.herdr_takeover.looking
    }

    /// The wizard closed with the move unanswered stays closed (B9).
    pub(super) fn first_run_closed_for_herdr(&self) -> bool {
        self.herdr_takeover.closed_unanswered
    }

    /// The Herdr step leaves for the agent step once there is nothing to
    /// move, as the devices step does once there is no device (D-17).
    pub(super) fn settle_first_run_herdr(&mut self) -> bool {
        if self.first_run_step() == Some(FirstRunStep::Herdr) && !self.first_run_herdr_pending() {
            return self.move_first_run(FirstRunStep::Herdr, FirstRunStep::Agents);
        }
        false
    }

    fn show_herdr_takeover(&mut self, next: Option<HerdrTakeoverSnapshot>) -> bool {
        if self.snapshot.status.herdr_takeover == next {
            return false;
        }
        self.snapshot.status.herdr_takeover = next;
        true
    }
}

/// The failure of a restart whose stop did not end the server.
const STOP_FAILED: &str = "stop_failed";

/// How long a look may take before it counts as one that could not read the
/// server: the wizard waits on its Herdr step for it, and a server that does
/// not answer must not hold it there with nothing to press (engineering rule
/// 10). Each of the look's own reads is bounded at 5 s; two in a row are the
/// slow case.
const LOOK_WITHIN: std::time::Duration = std::time::Duration::from_secs(8);

/// The takeover's look and the versions read beside it, or `Unreadable`
/// with no versions once `within` has passed; a look that answers later is
/// dropped with its thread.
pub(super) fn bounded_look(
    takeover: &Arc<dyn HerdrTakeover>,
    within: std::time::Duration,
) -> (Look, Option<Versions>) {
    let (sender, receiver) = std::sync::mpsc::channel();
    let takeover = Arc::clone(takeover);
    let spawned = thread::Builder::new()
        .name("herdr-core-herdr-takeover-read".into())
        .spawn(move || {
            let look = takeover.look();
            let _ = sender.send((look, Some(takeover.versions())));
        });
    if let Err(error) = spawned {
        let look = Look::Unreadable {
            stage: "worker".into(),
            detail: error.to_string(),
        };
        return (look, None);
    }
    receiver.recv_timeout(within).unwrap_or_else(|_| {
        let look = Look::Unreadable {
            stage: "timeout".into(),
            detail: format!("the look at the server took longer than {within:?}"),
        };
        (look, None)
    })
}

/// Hands a worker's result to the runtime and announces it.
fn answer(worker: &Worker, ingest: impl FnOnce(&mut Runtime) -> bool) {
    let Some(runtime) = worker.runtime.upgrade() else {
        return;
    };
    let changed = match runtime.lock() {
        Ok(mut guard) => ingest(&mut guard),
        Err(poisoned) => ingest(&mut poisoned.into_inner()),
    };
    drop(runtime);
    if changed {
        worker.notifier.notify();
    }
}

fn refused(action: TakeoverAction, reason: &str) -> bool {
    crate::diagnostic!(serde_json::json!({
        "component": "herdr",
        "kind": "herdr.takeover_refused",
        "action": format!("{action:?}"),
        "reason": reason,
    }));
    false
}

fn look_record(look: &Look) -> serde_json::Value {
    let mut record = match look {
        Look::Nothing => serde_json::json!({"outcome": "nothing_to_move"}),
        Look::Unreadable { stage, detail } => {
            serde_json::json!({"outcome": "unreadable", "stage": stage, "message": detail})
        }
        Look::Offer(offer) => serde_json::json!({
            "outcome": "offered",
            "offer": offer.kind.as_str(),
            "server_pid": offer.server_pid,
            "server_version": offer.server_version,
            "server_protocol": offer.server_protocol,
            "version": offer.version,
            "failed_before": offer.failed_before.map(|cause| cause.as_str()),
        }),
    };
    record["component"] = "herdr".into();
    record["kind"] = "herdr.handoff".into();
    record
}

fn move_record(moved: &Moved) -> serde_json::Value {
    let mut record = match moved {
        Moved::Done {
            from_pid,
            to_pid,
            elapsed_ms,
            path_herdr_behind,
        } => serde_json::json!({
            "outcome": "handed_off",
            "from_pid": from_pid,
            "to_pid": to_pid,
            "elapsed_ms": elapsed_ms,
            "path_herdr_behind": path_herdr_behind,
        }),
        Moved::Failed {
            cause,
            stage,
            detail,
        } => serde_json::json!({
            "outcome": "failed",
            "cause": cause.as_str(),
            "stage": stage,
            "message": detail,
        }),
    };
    record["component"] = "herdr".into();
    record["kind"] = "herdr.handoff".into();
    record["origin"] = "operator".into();
    record
}
