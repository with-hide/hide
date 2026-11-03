//! Running-agent input goes through the engine's prompt submission once.
//! A launch prompt carries only a fresh start's brief. A queued message that
//! waits for the turn to end cannot deliver a letter during that turn.
//! The agent's own record proves intake; a lost answer is never retried blindly.

use crate::delivery::typed::SendRoute;
use crate::engine::{Engine, EngineError, Wait};
use serde_json::json;
use std::time::{Duration, Instant};

const PROMPT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Delivered {
    Sent(SendRoute),
    NotYet(&'static str),
    Lost(&'static str),
}

/// The destination copied under Runtime's lock; the send runs off it.
#[derive(Clone)]
pub(crate) struct InputTarget {
    pub kind: String,
    pub pane_id: String,
    pub raw_pane_id: String,
}

/// Why an agent was not ready for input within its wait ([`ready`]).
#[derive(Debug)]
pub(crate) enum NotReady {
    /// Herdr refused the read's parameters.
    Request(String),
    /// The pane's agent could not be read.
    Unread(String),
    /// The agent did not take input within the wait; nothing was typed.
    Late(String),
}

/// Waits until Herdr reports the agent `name` in `pane` ready for input,
/// the moment an input that did not ride the start is typed after the
/// agent's screen is drawn; `wait` bounds it.
#[allow(clippy::disallowed_methods)] // a production wait on Herdr's readiness, bounded by the deadline
pub(crate) fn ready(
    engine: &dyn Engine,
    pane: &str,
    name: &str,
    wait: Duration,
) -> Result<(), NotReady> {
    let deadline = Instant::now() + wait;
    loop {
        let agent = match engine.delivery_agent(Wait::Each(Duration::from_secs(5)), pane) {
            Ok(agent) => agent,
            Err(EngineError::Request(message)) => return Err(NotReady::Request(message)),
            Err(error) => return Err(NotReady::Unread(error.to_string())),
        };
        if agent.pane_id == pane && agent.name == name && agent.readiness.accepts_input() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(NotReady::Late(format!(
                "readiness {:?} after {wait:?}",
                agent.readiness
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Submits input once through the pane's engine, on its owning machine.
pub(crate) fn deliver(engine: &dyn Engine, target: &InputTarget, text: &str) -> Delivered {
    match engine.prompt_agent(Wait::Small(PROMPT_TIMEOUT), &target.raw_pane_id, text) {
        Ok(()) => {
            logged(target, TYPED);
            Delivered::Sent(SendRoute::Typed)
        }
        // Herdr refused the parameters: nothing was typed.
        Err(EngineError::Request(_)) => {
            unavailable_reason(target, TYPED, "herdr_parameters");
            Delivered::NotYet("herdr_parameters")
        }
        Err(EngineError::Answer(_)) => {
            lost(target, TYPED, "herdr_format");
            Delivered::Lost("herdr_format")
        }
        Err(_) => {
            lost(target, TYPED, "herdr_call_failed");
            Delivered::Lost("herdr_call_failed")
        }
    }
}

/// The typed rung's name in a log line (`InputRung::name`).
const TYPED: &str = "typed";

fn logged(target: &InputTarget, rung: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung}));
}

/// A prompt that could not be submitted: diagnostic detail stays in the log
/// (design principle 13), while the caller handles NotYet or Lost.
fn unavailable_reason(target: &InputTarget, rung: &str, reason: &str) {
    crate::diagnostic!(
        json!({"component":"delivery","kind":"input.rung_unavailable",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason})
    );
}

fn lost(target: &InputTarget, rung: &str, reason: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung_lost",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use serde_json::json;

    #[test]
    fn a_working_codex_receives_one_prompt_in_its_current_turn() {
        let herdr = FakeHerdr::start("live-letter", |method, _| {
            assert_eq!(method, "agent.prompt");
            json!({"type":"agent_prompted","agent":{
                "pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1",
                "terminal_id":"term_1","agent":"codex","agent_status":"working",
                "state_change_seq":2,"focused":false,"interactive_ready":true,"revision":0}})
        });
        let target = InputTarget {
            kind: "codex".into(),
            pane_id: "pane".into(),
            raw_pane_id: "w1:p2".into(),
        };
        assert_eq!(
            deliver(herdr.engine().as_ref(), &target, "letter"),
            Delivered::Sent(SendRoute::Typed)
        );
        let calls = herdr.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["text"], "letter");
    }

    /// An agent that never takes input within the wait is `Late`, with
    /// nothing typed, and one ready under another name is not ready: the
    /// wait reads the agent the start named.
    #[test]
    fn an_agent_that_never_takes_input_is_late_and_nothing_is_typed() {
        for (name, interactive) in [("hide-codex-w1-p2", false), ("someone-else", true)] {
            let herdr = FakeHerdr::start("ladder-never-ready", move |method, _| match method {
                "agent.get" => json!({"type": "agent_info", "agent": {
                    "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                    "terminal_id": "term_1", "agent": "codex", "agent_status": "idle",
                    "state_change_seq": 1, "focused": false, "interactive_ready": interactive,
                    "name": name, "revision": 0}}),
                other => panic!("unexpected {other}"),
            });
            let answer = ready(
                herdr.engine().as_ref(),
                "w1:p2",
                "hide-codex-w1-p2",
                Duration::from_millis(300),
            );
            assert!(
                matches!(answer, Err(NotReady::Late(_))),
                "{name}: {answer:?}"
            );
            assert!(
                herdr
                    .calls()
                    .iter()
                    .all(|(method, _)| method == "agent.get")
            );
        }
    }
}
