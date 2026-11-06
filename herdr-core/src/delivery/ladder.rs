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
    /// The engine refused the read's parameters.
    Request(String),
    /// The pane's agent could not be read.
    Unread(String),
    /// The agent did not take input within the wait; nothing was typed.
    Late(String),
}

/// Waits until the engine reports the agent `name` in `pane` ready for input,
/// the moment an input that did not ride the start is typed after the
/// agent's screen is drawn; `wait` bounds it.
#[allow(clippy::disallowed_methods)] // a production wait on the engine's readiness, bounded by the deadline
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

/// Submits input once through the pane's engine, on its owning machine,
/// with each tab expanded to four spaces first, the same rule the launch
/// argument and prompt identity use (`hide_agent_hooks::delivery::input_text`).
pub(crate) fn deliver(engine: &dyn Engine, target: &InputTarget, text: &str) -> Delivered {
    let text = hide_agent_hooks::delivery::input_text(text);
    match engine.prompt_agent(Wait::Small(PROMPT_TIMEOUT), &target.raw_pane_id, &text) {
        Ok(()) => {
            logged(target, TYPED);
            Delivered::Sent(SendRoute::Typed)
        }
        Err(error) => match not_typed(&error) {
            Some(reason) => {
                unavailable_reason(target, TYPED, reason, &error);
                Delivered::NotYet(reason)
            }
            None => {
                let reason = if matches!(error, EngineError::Answer(_)) {
                    "herdr_format"
                } else {
                    "herdr_call_failed"
                };
                lost(target, TYPED, reason, &error);
                Delivered::Lost(reason)
            }
        },
    }
}

/// Why a submission that failed typed nothing, or `None` when it may have:
/// a request the engine could not take and a refusal the engine makes before
/// it writes leave the agent's input untouched, so the doorbell may ring
/// again. Any other refusal, such as the engine's `timeout` while it waits after
/// the submission, a failed connection, and an answer that never came or
/// could not be read may follow a submission that arrived.
fn not_typed(error: &EngineError) -> Option<&'static str> {
    match error {
        EngineError::Request(_) => Some("herdr_parameters"),
        EngineError::Refused { code, .. } if REFUSED_BEFORE_TYPING.contains(&code.as_str()) => {
            Some("engine_refused")
        }
        // A connection that failed after the request was written can read
        // as a transport failure, so only the engine's own words prove that
        // nothing was typed.
        _ => None,
    }
}

/// The refusals an engine answers `agent.prompt` with before it writes
/// anything: the engine's `agent_blocked` ("returns `agent_blocked` without
/// sending input", its socket API), and the core's own engine's three
/// (`engine::own`'s `prompt_agent`).
const REFUSED_BEFORE_TYPING: &[&str] = &["agent_blocked", "agent_not_ready", "empty_agent_prompt"];

/// The typed rung's name in a log line (`InputRung::name`).
const TYPED: &str = "typed";

fn logged(target: &InputTarget, rung: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung}));
}

/// A prompt that could not be submitted: diagnostic detail stays in the log
/// (design principle 13), while the caller handles NotYet or Lost.
fn unavailable_reason(target: &InputTarget, rung: &str, reason: &str, error: &EngineError) {
    crate::diagnostic!(
        json!({"component":"delivery","kind":"input.rung_unavailable",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason,
        "detail":error.to_string()})
    );
}

/// A prompt whose answer was lost; the engine's own words go to the log
/// only, as the detail of a failure the caller handles (design principle 13).
fn lost(target: &InputTarget, rung: &str, reason: &str, error: &EngineError) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung_lost",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason,
        "detail":error.to_string()}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::answers::{DeliveryAgent, Readiness};
    use crate::engine::fake::{FakeEngine, refused, reply, unanswered};

    #[test]
    fn a_working_codex_receives_one_prompt_in_its_current_turn() {
        let fake = FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            reply(())
        });
        let target = InputTarget {
            kind: "codex".into(),
            pane_id: "pane".into(),
            raw_pane_id: "w1:p2".into(),
        };
        assert_eq!(
            deliver(fake.engine().as_ref(), &target, "letter"),
            Delivered::Sent(SendRoute::Typed)
        );
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["text"], "letter");
    }

    #[test]
    fn a_tab_in_the_input_reaches_the_agent_as_four_spaces() {
        let fake = FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            reply(())
        });
        let target = InputTarget {
            kind: "omp".into(),
            pane_id: "pane".into(),
            raw_pane_id: "w1:p2".into(),
        };
        assert_eq!(
            deliver(
                fake.engine().as_ref(),
                &target,
                "names no file.\n\tIt needs no change.\nReply TOKEN"
            ),
            Delivered::Sent(SendRoute::Typed)
        );
        assert_eq!(
            fake.calls()[0].1["text"],
            "names no file.\n    It needs no change.\nReply TOKEN"
        );
    }

    /// A refusal the engine makes before writing typed nothing, so a bell may
    /// ring again and a brief is not submitted; a submission whose answer
    /// never came, or whose wait after it timed out, may have arrived
    /// and is never sent again blindly.
    #[test]
    fn a_refused_submission_is_not_typed_and_an_unanswered_one_may_be() {
        let fake = FakeEngine::start(|method, _| {
            assert_eq!(method, "prompt_agent");
            refused("agent_blocked", "agent is blocked")
        });
        let target = InputTarget {
            kind: "claude".into(),
            pane_id: "pane".into(),
            raw_pane_id: "w1:p2".into(),
        };
        assert_eq!(
            deliver(fake.engine().as_ref(), &target, "letter"),
            Delivered::NotYet("engine_refused")
        );
        assert_eq!(
            crate::coordination::executor::send_brief(
                fake.engine().as_ref(),
                &target,
                "spawn-1",
                "brief"
            ),
            Err("brief_not_submitted".to_owned())
        );

        let uncertain = FakeEngine::start(|_, _| unanswered("the answer never came"));
        assert_eq!(
            deliver(uncertain.engine().as_ref(), &target, "letter"),
            Delivered::Lost("herdr_call_failed"),
        );
    }

    /// An agent that never takes input within the wait is `Late`, with
    /// nothing typed, and one ready under another name is not ready: the
    /// wait reads the agent the start named.
    #[test]
    fn an_agent_that_never_takes_input_is_late_and_nothing_is_typed() {
        for (name, interactive) in [("hide-codex-w1-p2", false), ("someone-else", true)] {
            let fake = FakeEngine::start(move |method, _| match method {
                "delivery_agent" => reply(DeliveryAgent {
                    pane_id: "w1:p2".into(),
                    name: name.into(),
                    kind: Some("codex".into()),
                    session: None,
                    status: "idle".into(),
                    state_change_seq: 1,
                    readiness: Readiness::of(Some(interactive), None),
                }),
                other => panic!("unexpected {other}"),
            });
            let answer = ready(
                fake.engine().as_ref(),
                "w1:p2",
                "hide-codex-w1-p2",
                Duration::from_millis(300),
            );
            assert!(
                matches!(answer, Err(NotReady::Late(_))),
                "{name}: {answer:?}"
            );
            assert!(
                fake.methods()
                    .iter()
                    .all(|method| *method == "delivery_agent")
            );
        }
    }
}
