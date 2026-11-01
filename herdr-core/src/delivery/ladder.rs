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
    pub(crate) fn deliver(self, engine: &dyn Engine, target: &InputTarget, text: &str) -> Delivered {
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
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung_unavailable",
        "pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason}));
}

fn lost(target: &InputTarget, rung: &str, reason: &str) {
    crate::diagnostic!(json!({"component":"delivery","kind":"input.rung_lost",
        "pane_id":target.raw_pane_id,"agent":target.kind,"rung":rung,"reason":reason}));
}
