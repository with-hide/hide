//! What the window sees of a move and how it asks for one (PRD
//! core-host-node-move B2 to B5): the process's supervisor owns the move,
//! which outlives the core role that took the request, so its state is the
//! supervisor's `core_move` frame rather than the core's snapshot. Every
//! role this process mounts (the core, the move screen, the node) sends the
//! frame to its windows.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use hide_node_link::device::{ShownSignIn, SignIn};

use super::journal::{Journal, MoveFailure, MoveStep};
use crate::screen_sign_in::{ScreenSignIn, SignInAnswer};

/// What a window's `core_move` event asks: a request the supervisor takes,
/// or the abandon of the move its wait offers, which the wait itself hears.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "action")]
enum WindowAsk {
    Check { device: String },
    Start { device: String },
    CheckBack,
    Back,
    Abandon,
}

/// What a window asks of the move's supervisor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MoveRequest {
    /// Run the checks for moving the core to `device`; nothing changes.
    Check { device: String },
    /// Move the core to `device`, its checks run once more first.
    Start { device: String },
    /// From a node's window: run the checks for moving the core back to
    /// this machine; nothing changes.
    CheckBack,
    /// From a node's window: move the core back to this machine.
    Back,
}

/// The core's own machine asking its running core to stop for a move back
/// (`attach`, from `hided core-move release`): the core answers once it
/// has checked and recorded the stop, then stops. The asker waits a bounded
/// time, so the core stops only for a release whose `claim` it took before
/// the asker gave the wait up; one the asker gave up asks nothing any more.
#[derive(Debug)]
pub struct Release {
    pub intent: String,
    /// The node the core goes back to.
    pub target: String,
    pub claim: Arc<ReleaseClaim>,
    pub reply: std::sync::mpsc::Sender<Result<(), String>>,
}

/// Who decided a release: the core, which takes it once the stop is
/// recorded, or the asker, which gives it up when its wait ends. Whichever
/// comes first decides, so a release that waited in line past its asker
/// never stops a core no move takes over.
#[derive(Debug, Default)]
pub struct ReleaseClaim(AtomicU8);

const RELEASE_WAITING: u8 = 0;
const RELEASE_TAKEN: u8 = 1;
const RELEASE_GIVEN_UP: u8 = 2;

impl ReleaseClaim {
    /// The core's, once it recorded the stop: true while the asker still
    /// waits, which then waits for the answer however late its wait ends.
    pub fn take(&self) -> bool {
        self.0
            .compare_exchange(
                RELEASE_WAITING,
                RELEASE_TAKEN,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// The asker's, as its wait ends: true when the core had not taken it.
    pub fn give_up(&self) -> bool {
        self.0
            .compare_exchange(
                RELEASE_WAITING,
                RELEASE_GIVEN_UP,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn given_up(&self) -> bool {
        self.0.load(Ordering::Acquire) == RELEASE_GIVEN_UP
    }
}

/// What the supervisor is asked.
#[derive(Debug)]
pub enum Asked {
    Window(MoveRequest),
    Release(Release),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveState {
    /// No move asked for since this process started.
    #[default]
    Idle,
    Checking,
    /// Every check passed; the move can start.
    Ready,
    ChecksFailed,
    /// The core is stopping.
    Stopping,
    /// The copy is being made and sent.
    Copying,
    /// The other machine's core is starting.
    Starting,
    /// This machine's node is linking to the new core.
    Linking,
    /// The move committed; this window is a node of the new core.
    Done,
    /// A step failed and the move is being undone.
    RollingBack,
    /// The move was undone; this machine's core runs as before.
    RolledBack,
    /// The other machine cannot be reached at a step where the move can
    /// neither go on nor be undone without it: this machine waits for it.
    Waiting,
    /// The operator gave the waiting move up: this machine's core runs on
    /// its folder as the move found it, and the other machine's core of the
    /// move is stopped once this machine reaches it.
    Abandoned,
}

impl MoveState {
    /// Whether a move or its checks run, so another request is refused.
    pub fn busy(self) -> bool {
        matches!(
            self,
            Self::Checking
                | Self::Stopping
                | Self::Copying
                | Self::Starting
                | Self::Linking
                | Self::RollingBack
                | Self::Waiting
        )
    }
}

/// One check that did not pass, with what to do.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FailedCheck {
    pub check: CheckId,
    /// What the check found, for the line under it.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    /// The device is a connected device this core dials.
    Connection,
    /// No other node is linked to this core.
    OtherNode,
    /// The device's state folder holds no brain state and no other move.
    TargetState,
    /// The copy can reach the device's state folder: nothing on the way is
    /// a file or a link that leads nowhere, and the account owns it.
    TargetFolder,
    /// Both machines have a Herdr server.
    Herdr,
    /// The device answers as the machine this core knows.
    Identity,
    /// For a move back: this machine's state folder holds no brain state
    /// of its own.
    OwnState,
    /// For a move back: this machine's node is linked to the core.
    Link,
    /// The machine taking the core is signed in to `gh`.
    Gh,
    /// Someone is logged in to the desktop of the machine taking the core,
    /// the session its login item runs in.
    GuiSession,
    /// The machine taking the core does not sleep by itself on power.
    Sleep,
    /// The `hided` the move runs there is this build.
    Build,
    /// Every agent Hide AI asks answers on the machine taking the core, with
    /// that machine's logins.
    Ai,
    /// Hide AI is on and has an agent chosen on the machine whose core
    /// moves: its own setting, which moves with the core.
    AiSetup,
    /// No agent of this machine is asleep with its pane closed: a node
    /// cannot wake it.
    Dormant,
}

/// The checks of a move toward a device, each once.
pub const FORWARD_CHECKS: [CheckId; 13] = [
    CheckId::Connection,
    CheckId::OtherNode,
    CheckId::Herdr,
    CheckId::AiSetup,
    CheckId::Ai,
    CheckId::Identity,
    CheckId::Build,
    CheckId::Gh,
    CheckId::GuiSession,
    CheckId::Sleep,
    CheckId::Dormant,
    CheckId::TargetState,
    CheckId::TargetFolder,
];

/// The checks of a move back to this machine, each once.
pub const BACK_CHECKS: [CheckId; 8] = [
    CheckId::Link,
    CheckId::Herdr,
    CheckId::OwnState,
    CheckId::Connection,
    CheckId::Identity,
    CheckId::TargetState,
    CheckId::AiSetup,
    CheckId::Ai,
];

/// How many of `checks` ran when `failed` came back, or none when some
/// did not: a connection that failed stops the checks behind it, and with
/// nobody logged in to the desktop of the machine taking the core its
/// logins may not have been checked.
pub fn checks_run(checks: &[CheckId], failed: &[FailedCheck]) -> Option<u32> {
    let stopped = failed
        .iter()
        .any(|check| matches!(check.check, CheckId::Connection | CheckId::GuiSession));
    (!stopped).then_some(checks.len() as u32)
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MoveView {
    pub state: MoveState,
    /// Toward the machine named by `device`, or back to this one.
    pub direction: Option<super::journal::Direction>,
    pub device: Option<String>,
    /// The SSH alias the move reaches its other machine by, which a window
    /// the move holds before it drew the core names that machine by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub intent: Option<String>,
    pub sent: u64,
    pub total: u64,
    pub failed: Vec<FailedCheck>,
    /// How many checks ran, so the window counts the ones that passed;
    /// none when some did not run ([`checks_run`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<u32>,
    pub step: Option<MoveStep>,
    pub cause: Option<MoveFailure>,
    /// The node the window becomes after the move, so the page names it in
    /// its address.
    pub node: Option<String>,
    /// Whether the move's wait takes an `abandon` request now.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abandon: bool,
    /// What `ssh` asks while the checks the operator pressed for sign in to
    /// the other machine (PRD system-ssh-device-link B33); the dialog
    /// answers it with a `core_move` event {action: `sign_in`}.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<ShownSignIn>,
    /// The connection check stopped on ssh: the device row's grouping of its
    /// failure (`sign_in_needed`, `sign_in_refused`, `host_key_changed`,
    /// `cannot_connect`, ...), so the dialog offers the row's action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_problem: Option<String>,
}

impl MoveView {
    /// `journal`'s move in `state`: its direction, its other machine and
    /// the move itself; the step, the cause and the counts are the caller's.
    pub fn of(journal: &Journal, state: MoveState) -> Self {
        MoveView {
            state,
            direction: Some(journal.phase.direction()),
            device: Some(journal.peer.device.clone()),
            alias: Some(journal.peer.alias.clone()),
            intent: Some(journal.intent.clone()),
            ..MoveView::default()
        }
    }
}

/// The supervisor's side of the move: one request waiting at most, and the
/// view every role sends.
pub struct MoveControl {
    view: Arc<watch::Sender<MoveView>>,
    /// What the checks' `ssh` asks, which the view carries as `sign_in`.
    sign_in: Arc<ScreenSignIn>,
    requests: mpsc::Sender<Asked>,
    receiver: Mutex<Option<mpsc::Receiver<Asked>>>,
    /// The wait that offers an abandon now, whose move a resumed move's
    /// view does not name; changed only with the view's `abandon`, inside
    /// the view's own lock.
    offered: Mutex<Option<Offer>>,
    /// The offer the operator took; level-triggered, so a wait between two
    /// looks still hears it. Only the wait of that offer hears it: one
    /// published after its offer ended is left for no later wait to take.
    abandoned: watch::Sender<Option<Offer>>,
    /// The number the next offer is given.
    offers: AtomicU64,
}

/// One wait's offer of an abandon: its move and its own number, since the
/// same move can wait again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Offer {
    intent: String,
    number: u64,
}

/// Why a request was not taken.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestRefusal {
    /// A move or its checks are running.
    Busy,
    /// No supervisor takes moves in this process (a daemon started on its
    /// own seat).
    Unavailable,
    /// An abandon asked while no move waits on its other machine.
    NotWaiting,
}

impl RequestRefusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::Busy => "move_busy",
            Self::Unavailable => "move_unavailable",
            Self::NotWaiting => "move_not_waiting",
        }
    }
}

impl Default for MoveControl {
    fn default() -> Self {
        let (requests, receiver) = mpsc::channel(1);
        let view = Arc::new(watch::Sender::new(MoveView::default()));
        let shown = Arc::clone(&view);
        let sign_in = Arc::new(ScreenSignIn::new(
            "core_move",
            Box::new(move |wait| {
                shown.send_if_modified(|view| {
                    let changed = view.sign_in != wait;
                    view.sign_in = wait;
                    changed
                });
            }),
        ));
        Self {
            view,
            sign_in,
            requests,
            receiver: Mutex::new(Some(receiver)),
            offered: Mutex::new(None),
            abandoned: watch::Sender::new(None),
            offers: AtomicU64::new(0),
        }
    }
}

impl MoveControl {
    /// Takes `request` when no move or check runs. Taking it is the change
    /// to Checking, made in one step with the look at the view, so of two
    /// requests at once one is taken and the other refused as busy.
    pub fn request(&self, request: MoveRequest) -> Result<(), RequestRefusal> {
        if !self.has_supervisor() {
            return Err(RequestRefusal::Unavailable);
        }
        let (direction, device) = match &request {
            MoveRequest::Check { device } | MoveRequest::Start { device } => {
                (None, Some(device.clone()))
            }
            MoveRequest::CheckBack | MoveRequest::Back => {
                (Some(super::journal::Direction::Back), None)
            }
        };
        let checking = MoveView {
            state: MoveState::Checking,
            direction,
            device,
            ..MoveView::default()
        };
        let mut before = None;
        let taken = self.view.send_if_modified(|view| {
            if view.state.busy() {
                return false;
            }
            before = Some(std::mem::replace(view, checking));
            true
        });
        if !taken {
            return Err(RequestRefusal::Busy);
        }
        self.ask(Asked::Window(request)).inspect_err(|_| {
            if let Some(before) = before {
                self.view.send_replace(before);
            }
        })
    }

    /// Gives up the move whose wait offers it (amendment 4), as one change
    /// with the look at the view: the move goes on to end itself here, and
    /// a second abandon is busy.
    pub fn abandon(&self) -> Result<(), RequestRefusal> {
        if !self.has_supervisor() {
            return Err(RequestRefusal::Unavailable);
        }
        let mut taken = None;
        let mut refusal = RequestRefusal::NotWaiting;
        self.view.send_if_modified(|view| {
            let mut offered = self
                .offered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if view.state == MoveState::Waiting
                && view.abandon
                && let Some(offer) = offered.take()
            {
                // A move back given up goes on to commit here; a forward
                // one restores this machine's core.
                view.state = if view.direction == Some(super::journal::Direction::Back) {
                    MoveState::Starting
                } else {
                    MoveState::RollingBack
                };
                view.abandon = false;
                taken = Some(offer);
                return true;
            }
            if view.state.busy() {
                refusal = RequestRefusal::Busy;
            }
            false
        });
        let Some(offer) = taken else {
            return Err(refusal);
        };
        self.abandoned.send_replace(Some(offer));
        Ok(())
    }

    /// Offers an abandon of the move `intent` from its wait, which waits for
    /// it with the offer this returns.
    pub fn offer_abandon(&self, intent: &str) -> Offer {
        let offer = Offer {
            intent: intent.to_owned(),
            number: self.offers.fetch_add(1, Ordering::Relaxed),
        };
        self.set_offer(Some(offer.clone()));
        offer
    }

    /// Ends the wait's offer, dropping an abandon the wait did not hear
    /// because the peer answered first: the answer says what is so.
    pub fn end_abandon_offer(&self) {
        self.abandoned.send_replace(None);
        self.set_offer(None);
    }

    fn set_offer(&self, offer: Option<Offer>) {
        self.view.send_if_modified(|view| {
            let offering = offer.is_some();
            *self
                .offered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = offer;
            let changed = view.abandon != offering;
            view.abandon = offering;
            changed
        });
    }

    /// Returns once the operator took `offer`.
    pub async fn abandoned(&self, offer: &Offer) {
        let mut abandoned = self.abandoned.subscribe();
        // The sender is this value's own, so the wait cannot lose it.
        let _ = abandoned
            .wait_for(|abandoned| abandoned.as_ref() == Some(offer))
            .await;
    }

    /// Whether this process's supervisor takes requests (it took the
    /// receiver at its start).
    fn has_supervisor(&self) -> bool {
        self.receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    }

    /// Takes a window's `core_move` event; the refusal is its reason code.
    pub fn request_event(&self, event: &serde_json::Value) -> Result<(), &'static str> {
        let payload = event.get("payload").cloned();
        if payload
            .as_ref()
            .and_then(|payload| payload.get("action"))
            .and_then(serde_json::Value::as_str)
            == Some("sign_in")
        {
            // The answer goes to the waiting question and nowhere else; one
            // for a question no longer asked reaches nothing (B7, B10).
            let answer = payload
                .and_then(|payload| serde_json::from_value::<SignInAnswer>(payload).ok())
                .ok_or("move_malformed")?;
            self.sign_in.answer(answer.question, answer.reply);
            return Ok(());
        }
        let ask: WindowAsk = serde_json::from_value(
            event
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|_| "move_malformed")?;
        let request = match ask {
            WindowAsk::Check { device } => MoveRequest::Check { device },
            WindowAsk::Start { device } => MoveRequest::Start { device },
            WindowAsk::CheckBack => MoveRequest::CheckBack,
            WindowAsk::Back => MoveRequest::Back,
            WindowAsk::Abandon => return self.abandon().map_err(RequestRefusal::code),
        };
        self.request(request).map_err(RequestRefusal::code)
    }

    /// Asks the supervisor to stop its core for a move back and waits for
    /// its answer, which comes before the core stops.
    pub fn release(
        &self,
        intent: &str,
        target: &str,
        within: std::time::Duration,
    ) -> Result<(), String> {
        let (reply, answer) = std::sync::mpsc::channel();
        let claim = Arc::new(ReleaseClaim::default());
        self.ask(Asked::Release(Release {
            intent: intent.to_owned(),
            target: target.to_owned(),
            claim: Arc::clone(&claim),
            reply,
        }))
        .map_err(|refusal| refusal.code().to_owned())?;
        let unanswered = || "the core did not answer the release".to_owned();
        match answer.recv_timeout(within) {
            Ok(answered) => answered,
            // The core took the release as the wait ended: it answers next,
            // before it stops.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !claim.give_up() => {
                answer.recv().map_err(|_| unanswered())?
            }
            Err(_) => Err(unanswered()),
        }
    }

    fn ask(&self, asked: Asked) -> Result<(), RequestRefusal> {
        if !self.has_supervisor() {
            return Err(RequestRefusal::Unavailable);
        }
        self.requests.try_send(asked).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => RequestRefusal::Busy,
            mpsc::error::TrySendError::Closed(_) => RequestRefusal::Unavailable,
        })
    }

    /// The requests' receiver, once: the supervisor takes it at start.
    pub fn take_requests(&self) -> Option<mpsc::Receiver<Asked>> {
        self.receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub fn subscribe(&self) -> watch::Receiver<MoveView> {
        self.view.subscribe()
    }

    pub fn view(&self) -> MoveView {
        self.view.borrow().clone()
    }

    /// Replaces the view; what `ssh` asks now stays, since only its
    /// question's answer or end takes it away.
    pub fn set(&self, mut view: MoveView) {
        self.view.send_if_modified(|current| {
            view.sign_in = current.sign_in.take();
            let changed = *current != view;
            *current = view;
            changed
        });
    }

    /// Where the checks' `ssh` asks the operator: the dialog.
    pub fn sign_in(&self) -> Arc<dyn SignIn> {
        Arc::clone(&self.sign_in) as _
    }

    pub fn update(&self, change: impl FnOnce(&mut MoveView)) {
        self.view.send_if_modified(|current| {
            let before = current.clone();
            change(current);
            *current != before
        });
    }
}

/// The `core_move` frame of `view`.
pub fn frame(view: &MoveView) -> String {
    serde_json::json!({"type": "core_move", "payload": view}).to_string()
}

/// The frame a window hears when its `core_move` event is not taken.
pub fn refusal_frame(reason: &str) -> String {
    serde_json::json!({"type": "error", "payload": {"kind": "core_move", "reason": reason}, "message": reason})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B20: once a request is taken, a second one is refused as busy even
    /// before the supervisor has begun on the first, and the checks' end
    /// takes requests again.
    #[test]
    fn a_second_request_is_busy_until_the_first_has_ended() {
        let control = MoveControl::default();
        let start = || MoveRequest::Start {
            device: "mini".to_owned(),
        };
        assert_eq!(control.request(start()), Err(RequestRefusal::Unavailable));
        assert_eq!(control.view().state, MoveState::Idle);
        let mut requests = control.take_requests().unwrap();
        assert_eq!(control.request(start()), Ok(()));
        // The supervisor took it and has not yet said anything of it.
        assert!(matches!(
            requests.try_recv(),
            Ok(Asked::Window(MoveRequest::Start { .. }))
        ));
        assert_eq!(control.request(start()), Err(RequestRefusal::Busy));
        assert_eq!(
            control.request(MoveRequest::CheckBack),
            Err(RequestRefusal::Busy)
        );
        assert!(requests.try_recv().is_err(), "a second request was queued");
        let view = control.view();
        assert_eq!(view.state, MoveState::Checking);
        assert_eq!(view.device.as_deref(), Some("mini"));
        control.set(MoveView {
            state: MoveState::ChecksFailed,
            ..MoveView::default()
        });
        assert_eq!(control.request(MoveRequest::CheckBack), Ok(()));
    }

    /// Amendment 4: an abandon is taken only while a move's wait offers it;
    /// anywhere else it is refused, as busy while a move runs, and once
    /// taken a second one is busy. The wait that offered it hears it.
    #[tokio::test]
    async fn an_abandon_is_taken_only_from_the_wait_that_offers_it() {
        let control = MoveControl::default();
        let _requests = control.take_requests().unwrap();
        let abandon =
            || control.request_event(&serde_json::json!({"payload": {"action": "abandon"}}));
        assert_eq!(abandon(), Err("move_not_waiting"));
        // A move resumed after a restart, whose view names no intent.
        control.set(MoveView {
            state: MoveState::Copying,
            ..MoveView::default()
        });
        assert_eq!(abandon(), Err("move_busy"));
        // A wait that does not offer it, as the move back's.
        control.update(|view| view.state = MoveState::Waiting);
        assert_eq!(abandon(), Err("move_busy"));
        let offer = control.offer_abandon("move-1");
        assert_eq!(abandon(), Ok(()));
        let view = control.view();
        assert_eq!(view.state, MoveState::RollingBack);
        assert!(!view.abandon);
        assert_eq!(abandon(), Err("move_busy"));
        tokio::time::timeout(std::time::Duration::from_secs(1), control.abandoned(&offer))
            .await
            .expect("the wait did not hear the abandon");
        // A move back given up goes on to commit here, not back.
        control.set(MoveView {
            state: MoveState::Waiting,
            direction: Some(super::super::journal::Direction::Back),
            ..MoveView::default()
        });
        control.offer_abandon("move-2");
        assert_eq!(abandon(), Ok(()));
        assert_eq!(control.view().state, MoveState::Starting);
    }

    /// An abandon taken from one wait and published after that wait's offer
    /// ended, as an end that runs between the take and the publish leaves
    /// it, ends no later wait of the same move; that wait hears only its own.
    #[tokio::test]
    async fn an_abandon_of_an_ended_offer_ends_no_later_wait_of_the_same_move() {
        let control = MoveControl::default();
        let _requests = control.take_requests().unwrap();
        let waiting = || {
            control.set(MoveView {
                state: MoveState::Waiting,
                ..MoveView::default()
            })
        };
        waiting();
        let first = control.offer_abandon("move-1");
        assert_eq!(control.abandon(), Ok(()));
        control.end_abandon_offer();
        // The publish that came after the end.
        control.abandoned.send_replace(Some(first));
        waiting();
        let second = control.offer_abandon("move-1");
        // A zero timeout polls the wait once: a stale abandon would end it.
        assert!(
            tokio::time::timeout(std::time::Duration::ZERO, control.abandoned(&second))
                .await
                .is_err(),
            "the later wait heard the earlier offer's abandon"
        );
        assert_eq!(control.abandon(), Ok(()));
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            control.abandoned(&second),
        )
        .await
        .expect("the later wait heard its own abandon");
    }
}
