//! A device's panes asking the core for a Workspace credential and running
//! `hide` commands with it, over the node's link (PRD core-host-node B18,
//! B19). The node proves each caller with its own kernel and sends the proof
//! up; the core issues the credential and answers down. A command's bytes
//! travel the same link as a stream, so a device pane needs no other
//! connection to the core.

use serde::{Deserialize, Serialize};

/// Recovery advice shared by local and device pane refusals.
pub const CHECKOUT_NEXT_ACTION: &str = "Run the command from a shell inside a registered project checkout, or reconnect Hide, and retry";
pub const PANE_NEXT_ACTION: &str = "Reconnect the pane and retry";

pub fn refusal_next_action(reason: &str) -> &'static str {
    match reason {
        "checkout_not_registered" | "caller_unavailable" => CHECKOUT_NEXT_ACTION,
        _ => PANE_NEXT_ACTION,
    }
}

/// Streams one link carries at once.
pub const MAX_STREAMS: usize = 8;
/// The most bytes of a stream one message carries.
pub const MAX_CHUNK: usize = 48 * 1024;
/// Chunks the core holds for one stream before the stream is ended.
pub const MAX_PENDING_CHUNKS: usize = 64;
/// Herdr streams one link carries at once (PRD core-host-node-remote-core
/// D-20): the session subscription, controls, doorbell, find and phone
/// reply of a node that dialed its core each hold one while they run.
pub const MAX_HERDR_STREAMS: usize = 32;
/// Browser relay streams one link carries at once (B15): as many as the
/// core relays `hide browser` commands at once.
pub const MAX_BROWSER_STREAMS: usize = 4;
/// How often a node that dialed its core writes [`NodeEvent::Ping`].
pub const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the attach role waits for a byte from its node before it ends
/// the link as gone: three heartbeats.
pub const HEARTBEAT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Which shell a pane runs, as its Herdr terminal and the shell's pid and
/// start: the identity a bootstrap checks a caller against.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PaneIdentity {
    pub terminal_id: String,
    pub shell_pid: i32,
    pub shell_started: u64,
}

/// Where a runtime hook can hand letters to its model.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PullPoint {
    #[default]
    Prompt,
    Tool,
    TurnEnd,
}

/// The only commands admitted by a hook's proof-and-answer exchange.
/// Keeping this separate from Workspace commands prevents this transport
/// from acquiring dispatch, browser, or letter-confirmation authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookCommand {
    WorkspaceInfo {},
    LetterPull {
        prompt_digest: Option<String>,
        session: Option<String>,
        #[serde(default)]
        point: PullPoint,
    },
    Memory {
        event: String,
        runtime: String,
        session: String,
        #[serde(default)]
        prompt: String,
    },
}

impl HookCommand {
    /// Only session guidance keeps a reference for the commands it advertises.
    pub fn one_shot(&self) -> bool {
        !matches!(self, Self::WorkspaceInfo {})
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookRequest {
    pub request_id: String,
    pub command: HookCommand,
    /// A named reference constrains the fresh proof; it never substitutes
    /// for the node's kernel check of the caller.
    pub token: Option<String>,
}

pub const HOOK_REQUEST_LIMIT: usize = 16 * 1024;
pub const HOOK_RESPONSE_LIMIT: usize = 64 * 1024;

/// What a node tells the core without being asked: one JSON line with an
/// `event` field and no `id`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum NodeEvent {
    /// A process in `pane_id` asked for a credential, and the node's kernel
    /// says it descends from that pane's shell. `request` pairs the core's
    /// [`ProofAnswer`].
    PaneProof {
        request: u64,
        pane_id: String,
        #[serde(flatten)]
        identity: PaneIdentity,
        nonce: String,
        one_shot: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<HookRequest>,
    },
    /// A process that is in no pane the node can prove asked for a
    /// credential, and the node's kernel says its working directory is
    /// `path` (the wire spelling of its canonical form). Only a node that
    /// dialed its core sends it (PRD core-host-node-remote-core D-10), and
    /// the core binds the caller to the node's own registered checkout that
    /// holds `path`. `request` pairs the core's [`ProofAnswer`].
    CheckoutProof {
        request: u64,
        path: String,
        nonce: String,
        one_shot: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<HookRequest>,
    },
    /// A credential the node handed out is no longer held: its reference
    /// expired, was removed, or its holder ended.
    Revoke { token: String },
    /// A `hide` command opened a stream. `caller` is what the node proved
    /// about the process that opened it, as it opened ([`StreamCaller`]);
    /// absent when the node proved nothing.
    StreamOpen {
        stream: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caller: Option<StreamCaller>,
    },
    /// Base64 bytes the command sent.
    StreamData { stream: u64, data: String },
    /// The command's side of the stream ended.
    StreamClosed { stream: u64 },
    /// The node is alive: a node that dialed its core says so every
    /// [`HEARTBEAT`], so the attach role on the core's machine ends a link
    /// whose node fell silent (PRD core-host-node-remote-core B20).
    Ping,
    /// Base64 bytes the node read from a stream's end (its Herdr or its
    /// browser relay) on a stream the core opened with `link_open`.
    LinkData { stream: u64, data: String },
    /// A stream's end closed a stream the core opened, or the node did at
    /// one of its caps; `reason` says which, never what the stream carried.
    LinkClosed { stream: u64, reason: String },
    /// The node turned a caller away on its own, before the core was asked
    /// or at one of its caps, with the reason the caller read; the core
    /// records it with the node, since a device's stderr reaches no log.
    Refused {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane_id: Option<String>,
        reason: String,
    },
}

/// Who opened a stream, as the node proved it when the stream opened: the
/// process using a credential the node issued for `pane_id`,
/// descended from that pane's shell, whose identity the node read from its
/// Herdr then. The core checks the stream's command against it rather than
/// asking [`crate::protocol::Call::PaneInspect`], and only for a credential
/// the same link vouched for the same pane.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct StreamCaller {
    pub pane_id: String,
    pub identity: PaneIdentity,
}

/// The core's answer to a [`NodeEvent::PaneProof`]: the credential, or why
/// there is none in the reason codes a caller already acts on.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ProofAnswer {
    WorkspaceInfo {
        token: String,
        issued_new: bool,
        answer: serde_json::Value,
    },
    Hook {
        answer: serde_json::Value,
    },
    Issued {
        token: String,
        /// The token was made for this proof; one already held by the pane
        /// is not, and a failed delivery must not revoke it.
        issued_new: bool,
    },
    Refused {
        reason: String,
        next_action: String,
    },
}

/// What `Call::PanesStart` answers: where the node's bootstrap socket is.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PanesStarted {
    pub socket: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A stream the node proved nothing for opens without `caller`, and the
    /// core reads that line as no proof; a proved stream names the pane and
    /// the identity the node read.
    #[test]
    fn a_stream_open_reads_with_or_without_its_caller() {
        let older: NodeEvent =
            serde_json::from_value(json!({"event": "stream_open", "stream": 3})).unwrap();
        assert_eq!(
            older,
            NodeEvent::StreamOpen {
                stream: 3,
                caller: None
            }
        );
        let caller = StreamCaller {
            pane_id: "w1:p2".to_owned(),
            identity: PaneIdentity {
                terminal_id: "t-1".to_owned(),
                shell_pid: 42,
                shell_started: 7,
            },
        };
        let line = serde_json::to_value(NodeEvent::StreamOpen {
            stream: 3,
            caller: Some(caller),
        })
        .unwrap();
        assert_eq!(
            line,
            json!({"event": "stream_open", "stream": 3, "caller": {
                "pane_id": "w1:p2",
                "identity": {"terminal_id": "t-1", "shell_pid": 42, "shell_started": 7},
            }})
        );
        assert_eq!(
            serde_json::to_value(NodeEvent::StreamOpen {
                stream: 3,
                caller: None
            })
            .unwrap(),
            json!({"event": "stream_open", "stream": 3})
        );
    }
}
