//! What ssh asks while a connection to another machine signs in, on the
//! screen whose press started it (PRD system-ssh-device-link B33): the node
//! window's strip for the link to its core, the move dialog for a core
//! move's checks. The rules are the device row's (D-05 to D-08): the
//! question is ssh's own, it waits at most five minutes, and the operator's
//! answer travels from the screen's event to the waiting question over a
//! channel of that one question, never through a frame, a file or a log
//! (B10). Which attempt may ask is the caller's: an [`Upstream`] whose dial
//! nobody pressed for never reaches this.
//!
//! [`Upstream`]: hide_node::ssh::upstream::Upstream

use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use hide_node_link::device::{Answer, Refusal, ShownSignIn, SignIn, SignInReply, SignInWait};
use serde_json::json;

/// How long a question waits for the operator (D-05).
const ANSWER_WITHIN: Duration = Duration::from_secs(5 * 60);

/// A screen's answer to the question it shows: the question's number and
/// the reply (`text`, `accept` or `cancel`), as a held node window's
/// `core_link` event and the move dialog's `core_move` event carry them,
/// each with `action: sign_in`.
#[derive(serde::Deserialize)]
pub struct SignInAnswer {
    pub question: u64,
    #[serde(flatten)]
    pub reply: SignInReply,
}

/// Hears what the screen shows each time it changes: the wait, or none.
pub type Publish = Box<dyn Fn(Option<ShownSignIn>) + Send + Sync>;

/// One connection's sign-in on its screen: at most one wait at a time.
pub struct ScreenSignIn {
    /// Names the connection in the log.
    target: &'static str,
    state: Mutex<Waiting>,
    publish: Publish,
}

#[derive(Default)]
struct Waiting {
    /// The last question's number, so an answer typed for a question ssh no
    /// longer asks never answers the next one (B7).
    last: u64,
    shown: Option<ShownSignIn>,
    /// Where the answer to the shown question goes; `None` for a wait that
    /// ends by itself (a browser check, a security key's touch).
    answer: Option<mpsc::SyncSender<Result<Answer, Refusal>>>,
    /// The screen went away or cancelled what it showed: the attempt ends
    /// (B8), and nothing more of it is shown or asked until it has.
    withdrawn: bool,
}

impl ScreenSignIn {
    pub fn new(target: &'static str, publish: Publish) -> Self {
        Self {
            target,
            state: Mutex::default(),
            publish,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Replaces what is shown, telling the screen after the lock is let go,
    /// and answers the number the new wait was given.
    fn set(&self, mut state: MutexGuard<'_, Waiting>, wait: Option<SignInWait>) -> u64 {
        let shown = wait.map(|wait| {
            state.last += 1;
            ShownSignIn::new(state.last, wait)
        });
        let number = state.last;
        if state.shown != shown {
            state.shown.clone_from(&shown);
            drop(state);
            (self.publish)(shown);
        }
        number
    }

    /// What the screen shows now.
    pub fn shown(&self) -> Option<ShownSignIn> {
        self.lock().shown.clone()
    }

    /// The screen that started the attempt went away, or cancelled a wait:
    /// whatever is shown goes, a question waiting is refused as cancelled,
    /// and the attempt ends at its next look (`SignIn::withdrawn`), a
    /// browser check or a key touch included. Nothing shown withdraws
    /// nothing, so a screen that leaves between attempts ends none.
    pub fn withdraw(&self) {
        let mut state = self.lock();
        if state.shown.is_none() {
            return;
        }
        state.withdrawn = true;
        if let Some(answer) = state.answer.take() {
            let _ = answer.send(Err(Refusal::Cancelled));
        }
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": "sign_in.withdrawn",
            "target": self.target,
            "question": state.last,
        }));
        self.set(state, None);
    }

    /// The operator's reply to `question`. A reply for a question no longer
    /// asked reaches nothing: the screen already shows what is asked now.
    /// A Cancel of a wait that takes no answer withdraws it. Answers whether
    /// it reached the question.
    pub fn answer(&self, question: u64, reply: SignInReply) -> bool {
        let mut state = self.lock();
        let shown = state
            .shown
            .as_ref()
            .is_some_and(|shown| shown.question == question);
        if shown && state.answer.is_none() && matches!(reply, SignInReply::Cancel) {
            drop(state);
            self.withdraw();
            return true;
        }
        let pending = shown && state.answer.is_some();
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": if pending { "sign_in.answered" } else { "sign_in.answer_stale" },
            "target": self.target,
            "question": question,
            "reply": reply.code(),
        }));
        if !pending {
            return false;
        }
        if let Some(answer) = state.answer.take() {
            let _ = answer.send(reply.into_answer());
        }
        self.set(state, None);
        true
    }
}

impl SignIn for ScreenSignIn {
    fn show(&self, wait: Option<SignInWait>) {
        let state = self.lock();
        // A question waiting for its answer stays: it ends with its answer.
        if state.answer.is_some() || state.withdrawn {
            return;
        }
        self.set(state, wait);
    }

    fn ask(&self, wait: SignInWait) -> Result<Answer, Refusal> {
        let (sender, answers) = mpsc::sync_channel(1);
        let kind = wait.kind();
        let mut state = self.lock();
        if state.withdrawn {
            return Err(Refusal::Cancelled);
        }
        state.answer = Some(sender);
        let question = self.set(state, Some(wait));
        herdr_core::diagnostic!(json!({
            "component": "screen_sign_in",
            "kind": "sign_in.asked",
            "target": self.target,
            "question": question,
            "asks": kind,
        }));
        match answers.recv_timeout(ANSWER_WITHIN) {
            Ok(answer) => answer,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut state = self.lock();
                if state
                    .shown
                    .as_ref()
                    .is_some_and(|shown| shown.question == question)
                {
                    state.answer = None;
                    self.set(state, None);
                }
                Err(Refusal::TimedOut)
            }
            // The attempt ended and took the question with it.
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Refusal::Ended),
        }
    }

    fn ended(&self) {
        let mut state = self.lock();
        state.answer = None;
        state.withdrawn = false;
        self.set(state, None);
    }

    fn withdrawn(&self) -> bool {
        self.lock().withdrawn
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hide_node_link::device::SignInReply;

    use super::*;

    fn screen() -> (Arc<ScreenSignIn>, Arc<Mutex<Vec<Option<ShownSignIn>>>>) {
        let told = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&told);
        let sign_in = ScreenSignIn::new(
            "test",
            Box::new(move |shown| heard.lock().unwrap().push(shown)),
        );
        (Arc::new(sign_in), told)
    }

    fn reply(value: serde_json::Value) -> SignInReply {
        serde_json::from_value(value).unwrap()
    }

    fn secret() -> SignInWait {
        SignInWait::Secret {
            prompt: "al@core's password: ".to_owned(),
            key_file: None,
        }
    }

    /// Waits until the screen shows a question, and answers its number.
    fn asked(sign_in: &ScreenSignIn) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(shown) = sign_in.shown() {
                return shown.question;
            }
            assert!(std::time::Instant::now() < deadline, "nothing was asked");
            std::thread::yield_now();
        }
    }

    /// B6, B7, B10: the answer to the question shown reaches the waiting
    /// ask and clears the screen; an answer naming an older question
    /// reaches nothing, and nothing typed reaches what the screen is told.
    #[test]
    fn the_answer_to_the_shown_question_reaches_its_ask_and_nothing_else() {
        let (sign_in, told) = screen();
        let asking = {
            let sign_in = Arc::clone(&sign_in);
            std::thread::spawn(move || sign_in.ask(secret()))
        };
        let question = asked(&sign_in);
        assert!(!sign_in.answer(
            question + 1,
            reply(json!({"reply": "text", "text": "stale"}))
        ));
        assert!(sign_in.answer(question, reply(json!({"reply": "text", "text": "hunter2"}))));
        match asking.join().unwrap() {
            Ok(Answer::Text(secret)) => assert_eq!(secret.expose(), "hunter2"),
            other => panic!("{other:?}"),
        }
        assert_eq!(sign_in.shown(), None);
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(!format!("{told:?}").contains("hunter2"));
    }

    /// B8: Cancel is the ask's refusal, and the screen shows nothing more.
    #[test]
    fn a_cancel_refuses_the_question() {
        let (sign_in, _) = screen();
        let asking = {
            let sign_in = Arc::clone(&sign_in);
            std::thread::spawn(move || sign_in.ask(secret()))
        };
        let question = asked(&sign_in);
        assert!(sign_in.answer(question, reply(json!({"reply": "cancel"}))));
        assert!(matches!(asking.join().unwrap(), Err(Refusal::Cancelled)));
        assert_eq!(sign_in.shown(), None);
    }

    /// The attempt's end takes a question still waiting with it, and what
    /// it showed.
    #[test]
    fn the_sign_in_s_end_ends_a_waiting_question() {
        let (sign_in, _) = screen();
        let asking = {
            let sign_in = Arc::clone(&sign_in);
            std::thread::spawn(move || sign_in.ask(secret()))
        };
        asked(&sign_in);
        sign_in.ended();
        assert!(matches!(asking.join().unwrap(), Err(Refusal::Ended)));
        assert_eq!(sign_in.shown(), None);
    }

    /// B8: a screen that goes away, or cancels a wait that takes no answer,
    /// withdraws it: the attempt reads it withdrawn and nothing more is shown
    /// or asked until the attempt has ended; a withdraw with nothing shown
    /// ends nothing.
    #[test]
    fn a_withdrawn_browser_check_ends_the_attempt_and_asks_nothing_more() {
        let (sign_in, _) = screen();
        sign_in.withdraw();
        assert!(!sign_in.withdrawn(), "nothing was shown to withdraw");
        let browser = SignInWait::Browser {
            url: "https://login.example.com/a".to_owned(),
        };
        sign_in.show(Some(browser.clone()));
        let question = sign_in.shown().unwrap().question;
        assert!(sign_in.answer(question, reply(json!({"reply": "cancel"}))));
        assert!(sign_in.withdrawn());
        assert_eq!(sign_in.shown(), None);
        sign_in.show(Some(browser.clone()));
        assert_eq!(sign_in.shown(), None);
        assert!(matches!(sign_in.ask(secret()), Err(Refusal::Cancelled)));
        sign_in.ended();
        assert!(!sign_in.withdrawn());
        sign_in.show(Some(browser));
        assert!(sign_in.shown().is_some());
        sign_in.withdraw();
        assert!(sign_in.withdrawn());
    }

    /// A wait that ends by itself (a touch) is shown and cleared, and does
    /// not replace a question still waiting for its answer.
    #[test]
    fn a_touch_is_shown_but_never_over_a_waiting_question() {
        let (sign_in, _) = screen();
        let touch = || SignInWait::Touch {
            prompt: "Confirm user presence".to_owned(),
        };
        sign_in.show(Some(touch()));
        assert_eq!(sign_in.shown().map(|shown| shown.wait), Some(touch()));
        sign_in.show(None);
        assert_eq!(sign_in.shown(), None);
        let asking = {
            let sign_in = Arc::clone(&sign_in);
            std::thread::spawn(move || sign_in.ask(secret()))
        };
        let question = asked(&sign_in);
        sign_in.show(Some(touch()));
        assert_eq!(sign_in.shown().map(|shown| shown.question), Some(question));
        sign_in.ended();
        assert!(asking.join().unwrap().is_err());
    }
}
