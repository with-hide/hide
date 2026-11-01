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
//! - [`Delivered::Received`]: the agent's own API said it took the input.
//! - [`Delivered::NotYet`]: nothing was sent, because the agent cannot take
//!   it now (its session has not reported the way in yet, a refused call);
//!   Hide tries again.
//! - [`Delivered::Unconfirmed`]: sent without the agent's own answer (typed,
//!   or through a way in that answers nothing); the agent's own record
//!   decides, as for typing (`delivery::typed`).
//!
//! An agent's own way in runs on the node of the machine whose pane runs
//! the agent (`Call::Deliver`), so a connected device takes it as this
//! machine does. Everything here runs off the runtime lock: the target is
//! owned data the runtime assembled under its lock ([`InputTarget`]).

use std::sync::Arc;
use std::time::Duration;

use hide_agent_adapter::{AgentApi, BeforeSession, InputRung};
use hide_node_link::input::{Delivery, InputChannel};
use hide_node_link::panes::MessagingChannel;
use hide_node_link::protocol::Call;
use serde_json::json;

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
    /// The agent's own API said it took the input.
    Received,
    /// Nothing was sent; `cause` says why, and the input is tried again.
    NotYet(&'static str),
    /// Sent with no answer of the agent's own; its record decides. `lost`
    /// names a send whose own answer was lost, so the input may or may not
    /// have arrived and is never sent again on a timer.
    Unconfirmed { lost: Option<&'static str> },
}

/// The cause of a [`Delivered::NotYet`] while the agent has not reported
/// the session its way in needs.
pub(crate) const SESSION_UNREPORTED: &str = "session_unreported";

/// What the pane's session told Hide through its own hook, for the rungs
/// that need an address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionAddress {
    /// The agent's hook has not reported the pane's session yet, and nothing
    /// says it never will.
    Pending,
    /// What the session reported: its id and its messaging channel. Both
    /// absent for a session whose hooks are not connected.
    Known {
        session: Option<String>,
        messaging: Option<MessagingChannel>,
    },
}

/// One input's destination, owned, as the runtime knew it under its lock.
#[derive(Clone)]
pub(crate) struct InputTarget {
    /// The agent kind Herdr reports.
    pub kind: String,
    /// The pane as its own Herdr names it.
    pub raw_pane_id: String,
    pub address: SessionAddress,
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

/// The rungs `target` can take now, in order, with nothing sent. A rung
/// that waits for the session answers its cause.
pub(crate) struct Prepared {
    steps: Vec<Step>,
}

/// Decides the route for `target` without sending anything, so a caller
/// that reserves an attempt does so only when something will be sent.
pub(crate) fn prepare(target: &InputTarget) -> Result<Prepared, &'static str> {
    let ladder = hide_agent_adapter::adapter(&target.kind)
        .map_or(hide_agent_adapter::TYPED_ONLY, |row| row.input);
    let mut steps = Vec::new();
    for rung in ladder {
        match rung {
            InputRung::AgentApi(api) if api.first_input_only() => {}
            InputRung::AgentApi(api) => match channel(*api, &target.address) {
                Address::Channel(channel) => steps.push(Step::Channel(channel)),
                Address::Next => {}
                Address::Wait => return Err(SESSION_UNREPORTED),
            },
            InputRung::Typed => steps.push(Step::Typed),
        }
    }
    Ok(Prepared { steps })
}

enum Address {
    Channel(InputChannel),
    Next,
    Wait,
}

fn channel(api: AgentApi, address: &SessionAddress) -> Address {
    let reported = match address {
        SessionAddress::Pending => {
            return match api.before_session() {
                BeforeSession::Wait => Address::Wait,
                BeforeSession::NextRung => Address::Next,
            };
        }
        SessionAddress::Known { session, messaging } => (session, messaging),
    };
    match (api, reported) {
        (AgentApi::CodexQueue, (Some(thread), _)) => Address::Channel(InputChannel::CodexQueue {
            thread: thread.clone(),
        }),
        (AgentApi::ClaudeMessaging, (_, Some(messaging))) => {
            Address::Channel(InputChannel::ClaudeMessaging {
                socket: messaging.socket.clone(),
                token: messaging.token.clone(),
            })
        }
        _ => Address::Next,
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
                        Ok(Delivery::Received) => {
                            logged(target, rung, "received");
                            return Delivered::Received;
                        }
                        Ok(Delivery::Sent) => {
                            logged(target, rung, "sent");
                            return Delivered::Unconfirmed { lost: None };
                        }
                        Ok(Delivery::Unavailable { reason }) => {
                            unavailable_reason(target, rung, &reason);
                        }
                        Ok(Delivery::Unknown { reason }) => {
                            lost(target, rung, &reason);
                            return Delivered::Unconfirmed {
                                lost: Some("channel_answer_lost"),
                            };
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
                            return Delivered::Unconfirmed {
                                lost: Some("link_answer_lost"),
                            };
                        }
                    }
                }
                Step::Typed => {
                    return match engine.prompt_agent(
                        Wait::Small(PROMPT_TIMEOUT),
                        &target.raw_pane_id,
                        text,
                    ) {
                        Ok(()) => Delivered::Unconfirmed { lost: None },
                        // Herdr refused the parameters: nothing was typed.
                        Err(EngineError::Request(_)) => Delivered::NotYet("herdr_parameters"),
                        Err(EngineError::Answer(_)) => Delivered::Unconfirmed {
                            lost: Some("herdr_format"),
                        },
                        Err(_) => Delivered::Unconfirmed {
                            lost: Some("herdr_call_failed"),
                        },
                    };
                }
            }
        }
        // Every ladder ends typed, so only an undeclared kind gets here.
        Delivered::NotYet("no_rung")
    }
}

fn logged(target: &InputTarget, rung: &str, outcome: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung",
        "pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"outcome":outcome}));
}

fn unavailable(target: &InputTarget, channel: &InputChannel, reason: &str) {
    unavailable_reason(target, channel.name(), reason);
}

/// A way in that could not be used: logged, never shown (design principle
/// 13), and the next rung takes the input.
fn unavailable_reason(target: &InputTarget, rung: &str, reason: &str) {
    crate::diagnostic!(
        json!({"component":"delivery","kind":"input.rung_unavailable",
        "pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason})
    );
}

fn lost(target: &InputTarget, rung: &str, reason: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung_lost",
        "pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason}));
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

    fn target(kind: &str, address: SessionAddress, node: Option<Arc<Node>>) -> InputTarget {
        InputTarget {
            kind: kind.into(),
            raw_pane_id: "w1:p2".into(),
            address,
            node: node.map(|node| node as Arc<dyn NodeLink>),
        }
    }

    fn reported(session: Option<&str>, messaging: Option<(&str, &str)>) -> SessionAddress {
        SessionAddress::Known {
            session: session.map(str::to_owned),
            messaging: messaging.map(|(socket, token)| MessagingChannel {
                socket: socket.into(),
                token: token.into(),
            }),
        }
    }

    fn deliver(herdr: &FakeHerdr, target: &InputTarget, text: &str) -> Delivered {
        match prepare(target) {
            Ok(prepared) => prepared.deliver(herdr.engine().as_ref(), target, text),
            Err(cause) => Delivered::NotYet(cause),
        }
    }

    #[test]
    fn codex_input_goes_to_its_session_queue_and_is_received_without_typing() {
        let herdr = herdr("ladder-codex-queue");
        let node = Node::answering(Ok(Delivery::Received));
        let target = target(
            "codex",
            reported(Some("thread-1"), None),
            Some(node.clone()),
        );
        assert_eq!(
            deliver(&herdr, &target, "line one\nline two"),
            Delivered::Received
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
        let node = Node::answering(Ok(Delivery::Received));
        let target = target("codex", SessionAddress::Pending, Some(node.clone()));
        assert_eq!(
            deliver(&herdr, &target, "brief"),
            Delivered::Unconfirmed { lost: None }
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
            let target = target(
                "codex",
                reported(Some("thread-1"), None),
                Some(node.clone()),
            );
            assert_eq!(
                deliver(&herdr, &target, "letter"),
                Delivered::Unconfirmed { lost: None }
            );
            assert_eq!(node.asked().len(), 1);
            assert_eq!(typed(&herdr), [json!("letter")]);
        }
        // A machine whose node cannot be reached is typed straight away.
        let herdr = herdr("ladder-no-node");
        let target = target("codex", reported(Some("thread-1"), None), None);
        assert_eq!(
            deliver(&herdr, &target, "letter"),
            Delivered::Unconfirmed { lost: None }
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
            let target = target("codex", reported(Some("thread-1"), None), Some(node));
            let Delivered::Unconfirmed { lost: Some(_) } = deliver(&herdr, &target, "letter")
            else {
                panic!("a lost answer is unconfirmed and says so");
            };
            assert!(typed(&herdr).is_empty(), "the input may have arrived");
        }
    }

    #[test]
    fn claude_input_waits_until_herdr_names_its_session_then_goes_to_its_socket() {
        let herdr = herdr("ladder-claude");
        let node = Node::answering(Ok(Delivery::Sent));
        let waiting = target("claude", SessionAddress::Pending, Some(node.clone()));
        assert_eq!(
            deliver(&herdr, &waiting, "brief"),
            Delivered::NotYet(SESSION_UNREPORTED)
        );
        assert!(
            node.asked().is_empty() && typed(&herdr).is_empty(),
            "nothing is sent"
        );
        let reported = target(
            "claude",
            reported(Some("session-1"), Some(("/tmp/cc-socks/1.sock", "token-1"))),
            Some(node.clone()),
        );
        // The socket answers nothing: the prompt hook's record decides.
        assert_eq!(
            deliver(&herdr, &reported, "letter"),
            Delivered::Unconfirmed { lost: None }
        );
        assert_eq!(
            node.asked(),
            [(
                InputChannel::ClaudeMessaging {
                    socket: "/tmp/cc-socks/1.sock".into(),
                    token: "token-1".into()
                },
                "letter".into()
            )]
        );
        assert!(typed(&herdr).is_empty());
    }

    #[test]
    fn a_session_that_reported_no_channel_is_typed() {
        let herdr = herdr("ladder-claude-none");
        let node = Node::answering(Ok(Delivery::Sent));
        let target = target("claude", reported(None, None), Some(node.clone()));
        assert_eq!(
            deliver(&herdr, &target, "letter"),
            Delivered::Unconfirmed { lost: None }
        );
        assert!(node.asked().is_empty());
        assert_eq!(typed(&herdr), [json!("letter")]);
    }

    #[test]
    fn a_typed_only_agent_and_a_launch_prompt_agent_after_its_start_are_typed() {
        // Grok declares typing alone; agy's launch prompt takes only a fresh
        // start's first input.
        for kind in ["grok", "agy"] {
            let herdr = herdr("ladder-typed");
            let node = Node::answering(Ok(Delivery::Received));
            let target = target(kind, reported(Some("s"), None), Some(node.clone()));
            assert_eq!(
                deliver(&herdr, &target, "letter"),
                Delivered::Unconfirmed { lost: None }
            );
            assert!(node.asked().is_empty(), "{kind}");
            assert_eq!(typed(&herdr), [json!("letter")], "{kind}");
        }
    }

    #[test]
    fn the_token_never_reaches_a_debug_line() {
        let channel = InputChannel::ClaudeMessaging {
            socket: "/tmp/cc-socks/1.sock".into(),
            token: "secret-token".into(),
        };
        assert!(!format!("{channel:?}").contains("secret-token"));
        let fact = MessagingChannel {
            socket: "/tmp/cc-socks/1.sock".into(),
            token: "secret-token".into(),
        };
        assert!(!format!("{fact:?}").contains("secret-token"));
    }
}
