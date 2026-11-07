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
    answered(
        target,
        TYPED,
        engine.prompt_agent(Wait::Small(PROMPT_TIMEOUT), &target.raw_pane_id, &text),
    )
}

/// Presses Enter in the pane for a line Hide typed there that its composer
/// still shows, unsubmitted, and nothing else (`composer::Shows::Typed`).
pub(crate) fn press_enter(engine: &dyn Engine, target: &InputTarget) -> Delivered {
    answered(
        target,
        ENTER,
        engine.send_keys(Wait::Small(PROMPT_TIMEOUT), &target.raw_pane_id, &["Enter"]),
    )
}

/// One input's answer as what it delivered, logged by its rung.
fn answered(target: &InputTarget, rung: &str, answer: Result<(), EngineError>) -> Delivered {
    let Err(error) = answer else {
        logged(target, rung);
        return Delivered::Sent(SendRoute::Typed);
    };
    match not_typed(&error) {
        Some(reason) => {
            unavailable_reason(target, rung, reason, &error);
            Delivered::NotYet(reason)
        }
        None => {
            let reason = if matches!(error, EngineError::Answer(_)) {
                "herdr_format"
            } else {
                "herdr_call_failed"
            };
            lost(target, rung, reason, &error);
            Delivered::Lost(reason)
        }
    }
}

/// Why a submission that failed typed nothing, or `None` when it may have.
/// The engine's contract decides it, never a list of its codes: a request it
/// could not take, an engine that is not running or could not be reached,
/// and every refusal, its own (a blocked or starting agent, an empty prompt)
/// or its node's (a pane with no terminal owner, input over its cap, a
/// queued input cancelled or expired, a node busy or not connected), are
/// answered before anything is written (`hide-node`'s `command.rs`: once a
/// write began, its failure is `Unreachable`). An answer that never came,
/// or one that could not be read, may follow a submission that arrived.
fn not_typed(error: &EngineError) -> Option<&'static str> {
    match error {
        EngineError::Request(_) => Some("herdr_parameters"),
        EngineError::Refused { .. } => Some("engine_refused"),
        EngineError::NotRunning(_) | EngineError::Transport(_) => Some("engine_unreachable"),
        EngineError::Unanswered(_) | EngineError::Malformed(_) | EngineError::Answer(_) => None,
    }
}

/// The typed rung's name in a log line (`InputRung::name`).
const TYPED: &str = "typed";
/// The Enter a line still in the composer gets, in a log line.
const ENTER: &str = "enter";

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
        let target = InputTarget {
            kind: "claude".into(),
            pane_id: "pane".into(),
            raw_pane_id: "w1:p2".into(),
        };
        // The engine's own refusals and its node's, on this machine or a
        // device: a pane the node no longer lists answered
        // `pane_input_unavailable` before writing (with-hide/hide-private#677).
        for code in [
            "agent_blocked",
            "pane_input_unavailable",
            "pane_input_limit",
            "pane_input_cancelled",
            "node_busy",
        ] {
            let fake = FakeEngine::start(move |method, _| {
                assert_eq!(method, "prompt_agent");
                refused(code, "refused before typing")
            });
            assert_eq!(
                deliver(fake.engine().as_ref(), &target, "letter"),
                Delivered::NotYet("engine_refused"),
                "{code}"
            );
        }
        let fake = FakeEngine::start(|_, _| refused("agent_blocked", "agent is blocked"));
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
                    readiness: if interactive {
                        Readiness::Ready
                    } else {
                        Readiness::NotReady
                    },
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
