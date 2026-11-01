//! The input ladder: the one way Hide hands a running agent a brief or a
//! letter (`docs/delivery.md`, The input ladder).
//!
//! Each agent declares its rungs once, in `hide-agent-adapter`
//! (`AgentAdapter::input`): its own way in first, typing last. Every brief
//! and letter goes through [`prepare`], which picks the route with nothing
//! sent (so a caller reserves an attempt only when something will be sent),
//! then [`Prepared::deliver`], which walks it and answers one of three
//! things, all the rest of Hide reads, never the agent's kind:
//!
//! - [`Delivered::Sent`]: the input was sent whole, typed or through the
//!   agent's own way in. No way in says the agent took it (`codex queue`
//!   answers only that it queued the message), so the agent's own record
//!   decides, as for typing (`delivery::typed`).
//! - [`Delivered::NotYet`]: nothing was sent, because the agent cannot take
//!   it now (Herdr refused the typing); Hide tries again.
//! - [`Delivered::Lost`]: the send began and its answer was lost, so the
//!   input may or may not have arrived; it is never sent again blindly, and
//!   the agent's record still decides.
//!
//! An agent's own way in runs on the node of the machine whose pane runs
//! the agent (`Call::Deliver`), so a connected device takes it as this
//! machine does. Everything here runs off the runtime lock: the target is
//! owned data the runtime assembled under its lock ([`InputTarget`]).

use std::sync::Arc;
use std::time::Duration;

use hide_agent_adapter::{AgentApi, InputRung};
use hide_node_link::input::{Delivery, InputChannel};
use hide_node_link::protocol::Call;
use serde_json::json;

use crate::delivery::typed::SendRoute;
use crate::engine::{Engine, EngineError, Wait};
use crate::node_access::{LinkError, NodeLink};

/// How long a node's `Call::Deliver` may take: its own bound (ten seconds
/// for `codex queue`) and the link's carry.
const DELIVER_TIMEOUT: Duration = Duration::from_secs(15);

/// How long Herdr's `agent.prompt` may take for one typing.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(10);

/// What [`deliver`] answers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Delivered {
    /// Sent whole, typed or through the agent's own way in (the route
    /// says which); the agent's own record decides whether it took it.
    Sent(SendRoute),
    /// Nothing was sent; the cause says why, and the input is tried again.
    NotYet(&'static str),
    /// The send began and its answer was lost; the cause names where. The
    /// input may have arrived, so it is never sent again on a timer.
    Lost(&'static str),
}

/// One input's destination, owned, as the runtime knew it under its lock.
#[derive(Clone)]
pub(crate) struct InputTarget {
    /// The agent kind Herdr reports.
    pub kind: String,
    /// The pane as Hide names it, in its log lines.
    pub pane_id: String,
    /// The pane as its own Herdr names it.
    pub raw_pane_id: String,
    /// The id of the session Herdr names for the pane, as Hide's own hook
    /// reported it there; `None` until both name the same session, and then
    /// only typing is left (`HookFacts::reported_session`).
    pub session: Option<String>,
    /// The node of the machine whose pane runs the agent; `None` when its
    /// link is down, and then only typing is left.
    pub node: Option<Arc<dyn NodeLink>>,
}

/// The way each rung would be taken, decided before anything is sent.
#[derive(Clone, Debug)]
enum Step {
    Channel(InputChannel),
    Typed,
}

/// The rungs `target` can take now, in order, with nothing sent.
pub(crate) struct Prepared {
    steps: Vec<Step>,
}

/// Decides the route for `target` without sending anything, so a caller
/// that reserves an attempt does so only when something will be sent.
pub(crate) fn prepare(target: &InputTarget) -> Prepared {
    let ladder = hide_agent_adapter::adapter(&target.kind)
        .map_or(hide_agent_adapter::TYPED_ONLY, |row| row.input);
    let steps = ladder
        .iter()
        .filter_map(|rung| match rung {
            InputRung::AgentApi(api) => channel(*api, target.session.as_deref()).map(Step::Channel),
            InputRung::Typed => Some(Step::Typed),
        })
        .collect();
    Prepared { steps }
}

/// The route for input that is the operator's own words, such as a
/// mission's answer to its Conductor: typed, as the operator types, never
/// framed as another session's message.
pub(crate) fn typed_only() -> Prepared {
    Prepared {
        steps: vec![Step::Typed],
    }
}

/// The way in `api` gives for `session`; `None` hands the input to the next
/// rung. A launch prompt takes only a fresh start's first input, and Codex
/// has no thread before its first turn, which the input itself starts.
fn channel(api: AgentApi, session: Option<&str>) -> Option<InputChannel> {
    match api {
        AgentApi::LaunchPrompt => None,
        AgentApi::CodexQueue => session.map(|thread| InputChannel::CodexQueue {
            thread: thread.to_owned(),
        }),
    }
}

impl Prepared {
    /// Sends `text` down the prepared rungs: the first that takes it ends
    /// the walk, and one that cannot be used here hands it to the next.
    pub(crate) fn deliver(
        self,
        engine: &dyn Engine,
        target: &InputTarget,
        text: &str,
    ) -> Delivered {
        for step in self.steps {
            match step {
                Step::Channel(channel) => {
                    let Some(node) = target.node.as_deref() else {
                        unavailable(target, &channel, "node_unreachable");
                        continue;
                    };
                    let rung = channel.name();
                    let answer = crate::node_access::call_as::<Delivery>(
                        node,
                        Call::Deliver {
                            channel,
                            text: text.to_owned(),
                        },
                        DELIVER_TIMEOUT,
                    );
                    match answer {
                        Ok(Delivery::Sent) => {
                            logged(target, rung);
                            return Delivered::Sent(SendRoute::Channel);
                        }
                        Ok(Delivery::Unavailable { reason }) => {
                            unavailable_reason(target, rung, &reason);
                        }
                        Ok(Delivery::Unknown { reason }) => {
                            lost(target, rung, &reason);
                            return Delivered::Lost("channel_answer_lost");
                        }
                        // Nothing reached the node: the next rung takes it.
                        Err(
                            error @ (LinkError::NotConnected(_)
                            | LinkError::Busy
                            | LinkError::Refused(_)),
                        ) => {
                            unavailable_reason(
                                target,
                                rung,
                                &crate::node_access::link_failure_code(&error),
                            );
                        }
                        Err(LinkError::Unknown(reason)) => {
                            lost(target, rung, &reason);
                            return Delivered::Lost("link_answer_lost");
                        }
                    }
                }
                Step::Typed => {
                    return match engine.prompt_agent(
                        Wait::Small(PROMPT_TIMEOUT),
                        &target.raw_pane_id,
                        text,
                    ) {
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
                    };
                }
            }
        }
        // Every ladder ends typed, so only an undeclared kind gets here.
        Delivered::NotYet("no_rung")
    }
}

/// The typed rung's name in a log line (`InputRung::name`).
const TYPED: &str = "typed";

fn logged(target: &InputTarget, rung: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung",
        "pane_id":target.pane_id,"raw_pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung}));
}

fn unavailable(target: &InputTarget, channel: &InputChannel, reason: &str) {
    unavailable_reason(target, channel.name(), reason);
}

/// A way in that could not be used: logged, never shown (design principle
/// 13), and the next rung takes the input.
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
    //! The ladder's contract per rung kind: what a caller observes (the
    //! answer, what reached the node, what was typed), with Herdr behind the
    //! one fake that checks its answers against the pinned schema and a node
    //! that answers as a node does.

    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use crate::node_access::LinkAnswer;

    /// A node that answers every `Call::Deliver` with `answer` and keeps
    /// what it was asked.
    struct Node {
        answer: Result<Delivery, &'static str>,
        asked: Mutex<Vec<(InputChannel, String)>>,
    }

    impl Node {
        fn answering(answer: Result<Delivery, &'static str>) -> Arc<Self> {
            Arc::new(Self {
                answer,
                asked: Mutex::default(),
            })
        }

        fn asked(&self) -> Vec<(InputChannel, String)> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl NodeLink for Node {
        fn call(&self, call: Call, _: Duration) -> Result<LinkAnswer, LinkError> {
            let Call::Deliver { channel, text } = call else {
                panic!("the ladder asks its node only to deliver");
            };
            self.asked.lock().unwrap().push((channel, text));
            match &self.answer {
                Ok(answer) => Ok(LinkAnswer::Parsed(serde_json::to_value(answer).unwrap())),
                Err("lost") => Err(LinkError::Unknown("the link dropped".into())),
                Err(_) => Err(LinkError::NotConnected("no link".into())),
            }
        }
    }

    /// A Herdr that takes `agent.prompt` and keeps what it was asked.
    fn herdr(name: &str) -> FakeHerdr {
        FakeHerdr::start(name, |method, _| match method {
            "agent.prompt" => json!({"type": "agent_prompted", "agent": {
                "pane_id": "w1:p2", "tab_id": "w1:t1", "workspace_id": "w1",
                "terminal_id": "term_1", "agent": "codex", "agent_status": "working",
                "state_change_seq": 2, "focused": false, "interactive_ready": true,
                "revision": 0}}),
            other => panic!("unexpected {other}"),
        })
    }

    fn typed(herdr: &FakeHerdr) -> Vec<Value> {
        herdr
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "agent.prompt")
            .map(|(_, params)| params["text"].clone())
            .collect()
    }

    fn target(kind: &str, session: Option<&str>, node: Option<Arc<Node>>) -> InputTarget {
        InputTarget {
            pane_id: "pane".into(),
            kind: kind.into(),
            raw_pane_id: "w1:p2".into(),
            session: session.map(str::to_owned),
            node: node.map(|node| node as Arc<dyn NodeLink>),
        }
    }

    fn deliver(herdr: &FakeHerdr, target: &InputTarget, text: &str) -> Delivered {
        prepare(target).deliver(herdr.engine().as_ref(), target, text)
    }

    #[test]
    fn codex_input_goes_to_its_session_queue_without_typing() {
        let herdr = herdr("ladder-codex-queue");
        let node = Node::answering(Ok(Delivery::Sent));
        let target = target("codex", Some("thread-1"), Some(node.clone()));
        assert_eq!(
            deliver(&herdr, &target, "line one\nline two"),
            Delivered::Sent(SendRoute::Channel),
            "queued is not taken: the prompt hook's record decides"
        );
        assert_eq!(
            node.asked(),
            [(
                InputChannel::CodexQueue {
                    thread: "thread-1".into()
                },
                "line one\nline two".into()
            )]
        );
        assert!(typed(&herdr).is_empty(), "nothing is typed");
    }

    #[test]
    fn codex_with_no_reported_session_is_typed_since_its_first_turn_makes_the_session() {
        let herdr = herdr("ladder-codex-fresh");
        let node = Node::answering(Ok(Delivery::Sent));
        let target = target("codex", None, Some(node.clone()));
        assert_eq!(
            deliver(&herdr, &target, "brief"),
            Delivered::Sent(SendRoute::Typed)
        );
        assert!(node.asked().is_empty());
        assert_eq!(typed(&herdr), [json!("brief")]);
    }

    #[test]
    fn a_way_in_that_cannot_be_used_hands_the_input_to_typing() {
        for answer in [
            Ok(Delivery::Unavailable {
                reason: "codex_not_found".into(),
            }),
            Err("not connected"),
        ] {
            let herdr = herdr("ladder-unavailable");
            let node = Node::answering(answer);
            let target = target("codex", Some("thread-1"), Some(node.clone()));
            assert_eq!(
                deliver(&herdr, &target, "letter"),
                Delivered::Sent(SendRoute::Typed)
            );
            assert_eq!(node.asked().len(), 1);
            assert_eq!(typed(&herdr), [json!("letter")]);
        }
        // A machine whose node cannot be reached is typed straight away.
        let herdr = herdr("ladder-no-node");
        let target = target("codex", Some("thread-1"), None);
        assert_eq!(
            deliver(&herdr, &target, "letter"),
            Delivered::Sent(SendRoute::Typed)
        );
        assert_eq!(typed(&herdr), [json!("letter")]);
    }

    #[test]
    fn a_send_whose_answer_was_lost_is_never_typed_after_it() {
        for answer in [
            Ok(Delivery::Unknown {
                reason: "codex queue did not answer in time".into(),
            }),
            Err("lost"),
        ] {
            let herdr = herdr("ladder-lost");
            let node = Node::answering(answer);
            let target = target("codex", Some("thread-1"), Some(node));
            let Delivered::Lost(_) = deliver(&herdr, &target, "letter") else {
                panic!("a lost answer says so");
            };
            assert!(typed(&herdr).is_empty(), "the input may have arrived");
        }
    }

    #[test]
    fn a_typed_only_agent_and_a_launch_prompt_agent_after_its_start_are_typed() {
        // Grok declares typing alone; the launch prompt of agy and Claude
        // Code takes only a fresh start's first input.
        for kind in ["grok", "agy", "claude"] {
            let herdr = herdr("ladder-typed");
            let node = Node::answering(Ok(Delivery::Sent));
            let target = target(kind, Some("s"), Some(node.clone()));
            assert_eq!(
                deliver(&herdr, &target, "letter"),
                Delivered::Sent(SendRoute::Typed)
            );
            assert!(node.asked().is_empty(), "{kind}");
            assert_eq!(typed(&herdr), [json!("letter")], "{kind}");
        }
    }
}
