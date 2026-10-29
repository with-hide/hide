//! What ssh asks while a connection to another machine signs in, on the
//! screen whose press started it (PRD system-ssh-device-link B33): the node
//! window's strip for the link to its core, the move dialog for a core
//! move's checks. The rules are the device row's (D-05 to D-08): the
//! question is ssh's own, it waits at most five minutes, and the operator's
//! answer travels from the screen's event to the waiting question over a
//! channel of that one question, never through a frame, a file or a log
//! (B10).
//!
//! Each press is one [`Lease`], taken when the press is: it names the
//! screen connection that pressed, and only that screen is shown what ssh
//! asks, answers it, and by leaving or cancelling ends it, before the first
//! question, between two, or on one. Everything a lease does (show, ask,
//! answer, cancel, its deadline) goes through the one lock of its
//! [`ScreenSignIn`], so a reply checked against one question cannot act on
//! the next. A lease that is no longer the current one reads as withdrawn,
//! so the dial holding it ends and reaps its `ssh` at its next look. A dial
//! nobody pressed for holds no lease and never reaches this.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hide_node_link::device::{Answer, Refusal, ShownSignIn, SignIn, SignInReply, SignInWait};
use serde_json::json;

/// How long a question, a browser check or a key touch waits for the
/// operator (D-05, D-10).
const ANSWER_WITHIN: Duration = Duration::from_secs(5 * 60);

/// A screen's answer to the question it shows: the question's number and
/// the reply (`text`, `accept` or `cancel`), as a held node window's
/// `core_link` event and the move dialog's `core_move` event carry them,
/// each with `action: sign_in`. A `cancel` with no question ends the
/// screen's attempt whatever it shows, before its first question too.
#[derive(serde::Deserialize)]
pub struct SignInAnswer {
    #[serde(default)]
    pub question: Option<u64>,
    #[serde(flatten)]
    pub reply: SignInReply,
}

/// What a screen is shown, and the screen connection it is shown to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shown {
    pub owner: u64,
    pub sign_in: ShownSignIn,
}

/// Hears what is shown each time it changes: the wait, or none.
pub type Publish = Box<dyn Fn(Option<Shown>) + Send + Sync>;

/// A reply from a screen other than the one whose press the attempt is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotOwner;

/// One connection's sign-in: the press it runs for now, at most one.
pub struct ScreenSignIn {
    /// Names the connection in the log.
    target: &'static str,
    /// How long a wait waits for the operator.
    within: Duration,
    slot: Mutex<Slot>,
    publish: Publish,
}

#[derive(Default)]
struct Slot {
    /// The last question's number, across presses, so an answer typed for
    /// a question ssh no longer asks never answers the next one (B7).
    question: u64,
    /// The last lease's number.
    generation: u64,
    current: Option<Held>,
}

/// The current press.
struct Held {
    generation: u64,
    owner: u64,
    shown: Option<ShownSignIn>,
    /// Where the answer to the shown question goes; `None` for a wait that
    /// ends by itself (a browser check, a security key's touch).
    answer: Option<mpsc::SyncSender<Result<Answer, Refusal>>>,
    /// When a wait that takes no answer runs out (D-05, D-10).
    deadline: Option<Instant>,
}

/// One press's sign-in, the [`SignIn`] the dial it started asks through.
/// Dropping the last hold of it ends it.
pub struct Lease {
    screen: Arc<ScreenSignIn>,
    generation: u64,
}

impl ScreenSignIn {
    pub fn new(target: &'static str, publish: Publish) -> Self {
        Self::within(target, ANSWER_WITHIN, publish)
    }

    fn within(target: &'static str, within: Duration, publish: Publish) -> Self {
        Self {
            target,
            within,
            slot: Mutex::default(),
            publish,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Slot> {
        self.slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes the press of the screen connection `owner`: the attempt it
    /// starts asks through the lease answered, and a press before it ends.
    pub fn lease(self: &Arc<Self>, owner: u64) -> Arc<Lease> {
        let mut slot = self.lock();
        self.end(&mut slot, Refusal::Cancelled, "replaced");
        slot.generation += 1;
        slot.current = Some(Held {
            generation: slot.generation,
            owner,
            shown: None,
            answer: None,
            deadline: None,
        });
        Arc::new(Lease {
            screen: Arc::clone(self),
            generation: slot.generation,
        })
    }

    /// Ends the current press: a question waiting is refused with
    /// `refusal`, what it shows goes, and the dial holding it reads it
    /// withdrawn. Under the slot's lock, so nothing of a later press is
    /// touched.
    fn end(&self, slot: &mut Slot, refusal: Refusal, why: &'static str) {
        let Some(held) = slot.current.take() else {
            return;
        };
        if let Some(answer) = held.answer {
            let _ = answer.send(Err(refusal));
        }
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": "sign_in.ended",
            "target": self.target,
            "generation": held.generation,
            "why": why,
        }));
        if held.shown.is_some() {
            (self.publish)(None);
        }
    }

    /// The press of `generation`, while it is the current one.
    fn held(slot: &mut Slot, generation: u64) -> Option<&mut Held> {
        slot.current
            .as_mut()
            .filter(|held| held.generation == generation)
    }

    /// Shows `wait` for the current press, or clears it, telling the screen.
    fn set(&self, slot: &mut Slot, wait: Option<SignInWait>) -> u64 {
        if wait.is_some() {
            slot.question += 1;
        }
        let question = slot.question;
        let Some(held) = slot.current.as_mut() else {
            return question;
        };
        let shown = wait.map(|wait| ShownSignIn::new(question, wait));
        if held.shown != shown {
            held.shown.clone_from(&shown);
            (self.publish)(shown.map(|sign_in| Shown {
                owner: held.owner,
                sign_in,
            }));
        }
        question
    }

    /// What is shown now, and to whom.
    pub fn shown(&self) -> Option<Shown> {
        let slot = self.lock();
        let held = slot.current.as_ref()?;
        Some(Shown {
            owner: held.owner,
            sign_in: held.shown.clone()?,
        })
    }

    /// The screen connection `owner` went away: the press it made ends,
    /// whether or not anything shows, and a press of another screen stays
    /// (B8).
    pub fn release(&self, owner: u64) {
        let mut slot = self.lock();
        if slot.current.as_ref().is_some_and(|held| held.owner == owner) {
            self.end(&mut slot, Refusal::Cancelled, "screen_left");
        }
    }

    /// The reply of the screen connection `owner` to `question`, checked and
    /// acted on in one step. A reply for a question no longer asked reaches
    /// nothing: the screen already shows what is asked now. A Cancel of what
    /// is shown, or a Cancel naming no question, ends the press (B8). Answers whether it reached the attempt; `NotOwner` when
    /// another screen's press runs.
    pub fn answer(
        &self,
        owner: u64,
        question: Option<u64>,
        reply: SignInReply,
    ) -> Result<bool, NotOwner> {
        let mut slot = self.lock();
        let Some(held) = slot.current.as_mut() else {
            return Ok(false);
        };
        if held.owner != owner {
            return Err(NotOwner);
        }
        let cancel = matches!(reply, SignInReply::Cancel);
        let shown = held
            .shown
            .as_ref()
            .is_some_and(|shown| Some(shown.question) == question);
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": if shown || (cancel && question.is_none()) { "sign_in.answered" } else { "sign_in.answer_stale" },
            "target": self.target,
            "question": question,
            "reply": reply.code(),
        }));
        if cancel && (question.is_none() || shown) {
            self.end(&mut slot, Refusal::Cancelled, "cancelled");
            return Ok(true);
        }
        if !shown {
            return Ok(false);
        }
        let Some(answer) = held.answer.take() else {
            return Ok(false);
        };
        let _ = answer.send(reply.into_answer());
        self.set(&mut slot, None);
        Ok(true)
    }
}

impl SignIn for Lease {
    fn show(&self, wait: Option<SignInWait>) {
        let screen = &self.screen;
        let mut slot = screen.lock();
        let Some(held) = ScreenSignIn::held(&mut slot, self.generation) else {
            return;
        };
        // A question waiting for its answer stays: it ends with its answer.
        if held.answer.is_some() {
            return;
        }
        held.deadline = wait.as_ref().map(|_| Instant::now() + screen.within);
        screen.set(&mut slot, wait);
    }

    fn ask(&self, wait: SignInWait) -> Result<Answer, Refusal> {
        let screen = &self.screen;
        let (sender, answers) = mpsc::sync_channel(1);
        let kind = wait.kind();
        let question = {
            let mut slot = screen.lock();
            let Some(held) = ScreenSignIn::held(&mut slot, self.generation) else {
                return Err(Refusal::Cancelled);
            };
            held.answer = Some(sender);
            held.deadline = None;
            screen.set(&mut slot, Some(wait))
        };
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": "sign_in.asked",
            "target": screen.target,
            "question": question,
            "asks": kind,
        }));
        match answers.recv_timeout(screen.within) {
            Ok(answer) => answer,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut slot = screen.lock();
                let unanswered = ScreenSignIn::held(&mut slot, self.generation).is_some_and(|held| {
                    held.shown
                        .as_ref()
                        .is_some_and(|shown| shown.question == question)
                });
                if unanswered {
                    screen.end(&mut slot, Refusal::TimedOut, "timed_out");
                }
                Err(Refusal::TimedOut)
            }
            // The press ended and took the question with it.
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Refusal::Ended),
        }
    }

    fn ended(&self) {
        let mut slot = self.screen.lock();
        if ScreenSignIn::held(&mut slot, self.generation).is_some() {
            self.screen.end(&mut slot, Refusal::Ended, "attempt_ended");
        }
    }

    fn withdrawn(&self) -> bool {
        let mut slot = self.screen.lock();
        let Some(held) = ScreenSignIn::held(&mut slot, self.generation) else {
            return true;
        };
        // A browser check or a key touch nobody finished in time.
        if held.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.screen.end(&mut slot, Refusal::TimedOut, "timed_out");
            return true;
        }
        false
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.ended();
    }
}

#[cfg(test)]
mod tests {
    use hide_node_link::device::SignInReply;

    use super::*;

    const A: u64 = 1;
    const B: u64 = 2;

    type Told = Arc<Mutex<Vec<Option<Shown>>>>;

    fn screen_within(within: Duration) -> (Arc<ScreenSignIn>, Told) {
        let told = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&told);
        let sign_in = ScreenSignIn::within(
            "test",
            within,
            Box::new(move |shown| heard.lock().unwrap().push(shown)),
        );
        (Arc::new(sign_in), told)
    }

    fn screen() -> (Arc<ScreenSignIn>, Told) {
        screen_within(ANSWER_WITHIN)
    }

    fn reply(value: serde_json::Value) -> SignInReply {
        serde_json::from_value(value).unwrap()
    }

    fn cancel() -> SignInReply {
        reply(json!({"reply": "cancel"}))
    }

    fn secret() -> SignInWait {
        SignInWait::Secret {
            prompt: "al@core's password: ".to_owned(),
            key_file: None,
        }
    }

    fn browser() -> SignInWait {
        SignInWait::Browser {
            url: "https://login.example.com/a".to_owned(),
        }
    }

    fn touch() -> SignInWait {
        SignInWait::Touch {
            prompt: "Confirm user presence".to_owned(),
        }
    }

    /// Asks `wait` on `lease` from a thread of its own, as ssh's desk does.
    fn asking(
        lease: &Arc<Lease>,
        wait: SignInWait,
    ) -> std::thread::JoinHandle<Result<Answer, Refusal>> {
        let lease = Arc::clone(lease);
        std::thread::spawn(move || lease.ask(wait))
    }

    /// Waits until a question numbered past `after` shows, and answers it.
    fn asked_after(sign_in: &ScreenSignIn, after: u64) -> Shown {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(shown) = sign_in.shown().filter(|shown| shown.sign_in.question > after) {
                return shown;
            }
            assert!(Instant::now() < deadline, "nothing was asked");
            std::thread::yield_now();
        }
    }

    fn asked(sign_in: &ScreenSignIn) -> u64 {
        asked_after(sign_in, 0).sign_in.question
    }

    /// B6, B7, B10: the answer to the question shown reaches the waiting
    /// ask and clears the screen; an answer naming an older question
    /// reaches nothing, and nothing typed reaches what the screen is told.
    #[test]
    fn the_answer_to_the_shown_question_reaches_its_ask_and_nothing_else() {
        let (sign_in, told) = screen();
        let lease = sign_in.lease(A);
        let waiting = asking(&lease, secret());
        let question = asked(&sign_in);
        assert_eq!(sign_in.shown().unwrap().owner, A);
        let stale = reply(json!({"reply": "text", "text": "stale"}));
        assert_eq!(sign_in.answer(A, Some(question + 1), stale), Ok(false));
        let typed = reply(json!({"reply": "text", "text": "hunter2"}));
        assert_eq!(sign_in.answer(A, Some(question), typed), Ok(true));
        match waiting.join().unwrap() {
            Ok(Answer::Text(secret)) => assert_eq!(secret.expose(), "hunter2"),
            other => panic!("{other:?}"),
        }
        assert_eq!(sign_in.shown(), None);
        assert!(!lease.withdrawn(), "an answer ends no attempt");
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(!format!("{told:?}").contains("hunter2"));
    }

    /// B8: Cancel is the ask's refusal, and the screen shows nothing more.
    #[test]
    fn a_cancel_refuses_the_question() {
        let (sign_in, _) = screen();
        let lease = sign_in.lease(A);
        let waiting = asking(&lease, secret());
        let question = asked(&sign_in);
        assert_eq!(sign_in.answer(A, Some(question), cancel()), Ok(true));
        assert!(matches!(waiting.join().unwrap(), Err(Refusal::Cancelled)));
        assert_eq!(sign_in.shown(), None);
        assert!(lease.withdrawn());
    }

    /// The attempt's end takes a question still waiting with it, and what
    /// it showed.
    #[test]
    fn the_sign_in_s_end_ends_a_waiting_question() {
        let (sign_in, told) = screen();
        let lease = sign_in.lease(A);
        let waiting = asking(&lease, secret());
        asked(&sign_in);
        lease.ended();
        assert!(matches!(waiting.join().unwrap(), Err(Refusal::Ended)));
        assert_eq!(sign_in.shown(), None);
        assert_eq!(told.lock().unwrap().last(), Some(&None));
    }

    /// B8, B33: only the screen that pressed answers, and it leaving ends
    /// its press before any question shows: the dial reads it withdrawn,
    /// asks nothing and shows nothing. Another screen leaving ends nothing.
    #[test]
    fn the_screen_that_pressed_owns_the_attempt_and_its_leaving_ends_it_before_any_question() {
        let (sign_in, told) = screen();
        let lease = sign_in.lease(A);
        assert_eq!(sign_in.answer(B, None, cancel()), Err(NotOwner));
        sign_in.release(B);
        assert!(!lease.withdrawn());
        sign_in.release(A);
        assert!(lease.withdrawn());
        assert!(matches!(lease.ask(secret()), Err(Refusal::Cancelled)));
        lease.show(Some(browser()));
        assert_eq!(sign_in.shown(), None);
        assert!(told.lock().unwrap().is_empty(), "{:?}", told.lock().unwrap());
    }

    /// B8: the screen that pressed leaving between two questions ends the
    /// attempt: the second is never asked.
    #[test]
    fn a_screen_leaving_between_two_questions_ends_the_attempt() {
        let (sign_in, _) = screen();
        let lease = sign_in.lease(A);
        let waiting = asking(&lease, secret());
        let first = asked(&sign_in);
        let typed = reply(json!({"reply": "text", "text": "hunter2"}));
        assert_eq!(sign_in.answer(A, Some(first), typed), Ok(true));
        assert!(waiting.join().unwrap().is_ok());
        sign_in.release(A);
        assert!(lease.withdrawn());
        assert!(matches!(lease.ask(secret()), Err(Refusal::Cancelled)));
        assert_eq!(sign_in.shown(), None);
    }

    /// The window's Cancel with no question, as closing the move dialog
    /// sends it, ends the press whatever it shows.
    #[test]
    fn a_cancel_naming_no_question_ends_the_press() {
        let (sign_in, _) = screen();
        let lease = sign_in.lease(A);
        assert_eq!(
            sign_in.answer(A, None, reply(json!({"reply": "accept"}))),
            Ok(false)
        );
        assert!(!lease.withdrawn());
        assert_eq!(sign_in.answer(A, None, cancel()), Ok(true));
        assert!(lease.withdrawn());
    }

    /// D-05, D-10: a browser check or a key touch nobody finishes ends the
    /// attempt at its bound, and the screen shows it no more.
    #[test]
    fn a_touch_or_a_browser_check_nobody_finishes_ends_the_attempt_at_its_bound() {
        for wait in [touch(), browser()] {
            let (sign_in, told) = screen_within(Duration::from_millis(200));
            let lease = sign_in.lease(A);
            lease.show(Some(wait.clone()));
            assert_eq!(sign_in.shown().map(|shown| shown.sign_in.wait), Some(wait));
            assert!(!lease.withdrawn());
            std::thread::sleep(Duration::from_millis(250));
            assert!(lease.withdrawn());
            assert_eq!(sign_in.shown(), None);
            assert_eq!(told.lock().unwrap().last(), Some(&None));
        }
    }

    /// B7: a Cancel the screen sent for a browser check that ended, which
    /// arrives after ssh asked its next question, leaves that question.
    #[test]
    fn a_cancel_of_an_earlier_wait_leaves_the_question_asked_since() {
        let (sign_in, _) = screen();
        let lease = sign_in.lease(A);
        lease.show(Some(browser()));
        let first = sign_in.shown().unwrap().sign_in.question;
        lease.show(None);
        let waiting = asking(&lease, secret());
        let second = asked_after(&sign_in, first).sign_in.question;
        assert_eq!(sign_in.answer(A, Some(first), cancel()), Ok(false));
        assert!(!lease.withdrawn());
        assert_eq!(
            sign_in.shown().map(|shown| shown.sign_in.question),
            Some(second)
        );
        let typed = reply(json!({"reply": "text", "text": "hunter2"}));
        assert_eq!(sign_in.answer(A, Some(second), typed), Ok(true));
        assert!(waiting.join().unwrap().is_ok());
    }

    /// A wait that ends by itself (a touch) is shown and cleared, and does
    /// not replace a question still waiting for its answer.
    #[test]
    fn a_touch_is_shown_but_never_over_a_waiting_question() {
        let (sign_in, _) = screen();
        let lease = sign_in.lease(A);
        lease.show(Some(touch()));
        assert_eq!(sign_in.shown().map(|shown| shown.sign_in.wait), Some(touch()));
        lease.show(None);
        assert_eq!(sign_in.shown(), None);
        let waiting = asking(&lease, secret());
        let question = asked(&sign_in);
        lease.show(Some(touch()));
        assert_eq!(
            sign_in.shown().map(|shown| shown.sign_in.question),
            Some(question)
        );
        lease.ended();
        assert!(waiting.join().unwrap().is_err());
    }

    /// A new press ends the one before it, and the old lease's end or drop
    /// touches nothing of the new one.
    #[test]
    fn a_new_press_ends_the_last_and_the_last_ends_nothing_of_it() {
        let (sign_in, _) = screen();
        let first = sign_in.lease(A);
        let second = sign_in.lease(B);
        assert!(first.withdrawn());
        first.ended();
        drop(first);
        sign_in.release(A);
        assert!(!second.withdrawn());
        second.show(Some(browser()));
        assert_eq!(sign_in.shown().unwrap().owner, B);
        drop(second);
        assert_eq!(sign_in.shown(), None);
    }
}
