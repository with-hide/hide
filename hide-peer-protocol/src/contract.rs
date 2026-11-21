//! `contracts/peer-link.json`, derived from this crate's types.
//!
//! [`shape`] is what the file must say. The test below fails until the
//! committed file equals it, so a change to anything another build reads
//! shows up as a change to that file in the same pull request
//! (`contracts/README.md`, Peer link, says how to write it again).
//! [`hash`] is what a build that is not a release puts in its hello (D-40).

use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::build::{self, BuildFile, FileMessage};
use crate::core::{Agent, Ask, CoreMessage, Item, Party};
use crate::event::{self, Acked, Event};
use crate::hello::{self, GrantState, Hello, HelloFrame, LinkRange};
use crate::launch::{
    self, BoxFull, EndMode, FromNode, HandedTo, Installation, Position, Ready, ToNode,
};
use crate::manage::{Manage, NodeStop, Release};
use crate::pairing::{ConfirmState, PairRefusal, Pairing, Revoke};
use crate::panes::{self, Pane, Panes};
use crate::stream::{self, Status};
use crate::{ALPN, CloseCode, StreamKind, frame, key, tap};

const FILE: &str = include_str!("../../contracts/peer-link.json");

/// The SHA-256 of the contract file this build was made from, as lower hex.
/// Carriage returns are left out, so a checkout that rewrote line endings
/// hashes the same.
pub fn hash() -> String {
    let mut hasher = Sha256::new();
    for line in FILE.split('\r') {
        hasher.update(line.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// The committed contract file, parsed.
pub fn committed() -> Value {
    serde_json::from_str(FILE).expect("contracts/peer-link.json is JSON")
}

fn name<T: Serialize>(value: &T) -> Value {
    match serde_json::to_value(value).expect("a wire name serializes") {
        Value::String(text) => Value::String(text),
        other => panic!("a wire name is a string, got {other}"),
    }
}

/// Lists an enum's wire names. The match fails to compile when a variant is
/// added and not listed here.
macro_rules! names {
    ($ty:ty: $($variant:ident),+ $(,)?) => {{
        fn exhaustive(value: $ty) {
            match value {
                $(<$ty>::$variant => {}),+
            }
        }
        let _ = exhaustive;
        Value::Array(vec![$(name(&<$ty>::$variant)),+])
    }};
}

/// The sorted field names of a message, without its `t`.
fn fields<T: Serialize>(sample: &T) -> Value {
    let Value::Object(map) = serde_json::to_value(sample).expect("a message serializes") else {
        panic!("a message is a JSON object");
    };
    let mut names: Vec<&String> = map.keys().filter(|key| key.as_str() != "t").collect();
    names.sort();
    json!(names)
}

/// One entry per message type: the fields it carries and the device
/// connection number it came with.
fn messages<T: Serialize>(types: &[&str], samples: &[T]) -> Value {
    let mut out = serde_json::Map::new();
    for sample in samples {
        let value = serde_json::to_value(sample).expect("a message serializes");
        let tag = value["t"]
            .as_str()
            .expect("a message names its type")
            .to_owned();
        out.insert(tag, json!({ "since": 1, "fields": fields(sample) }));
    }
    // Sorted on both sides: whether a JSON object keeps insertion order
    // depends on a feature another crate in the build may turn on.
    let mut listed: Vec<&str> = out.keys().map(String::as_str).collect();
    listed.sort();
    let mut known: Vec<&str> = types.to_vec();
    known.sort();
    assert_eq!(
        listed, known,
        "every type a reader accepts has a sample here"
    );
    Value::Object(out)
}

fn hello_frames() -> Vec<HelloFrame> {
    fn exhaustive(frame: &HelloFrame) {
        match frame {
            HelloFrame::Grant { .. } | HelloFrame::Update | HelloFrame::UpdateAnswer { .. } => {}
        }
    }
    let _ = exhaustive;
    vec![
        HelloFrame::Grant {
            state: GrantState::Active,
            generation: 1,
        },
        HelloFrame::Update,
        HelloFrame::UpdateAnswer {
            started: false,
            reason: Some(String::new()),
        },
    ]
}

fn pairing_messages() -> Vec<Pairing> {
    fn exhaustive(message: &Pairing) {
        match message {
            Pairing::Pair { .. }
            | Pairing::Asking { .. }
            | Pairing::Reveal { .. }
            | Pairing::Request { .. }
            | Pairing::Granted { .. }
            | Pairing::Refused { .. }
            | Pairing::Confirm { .. }
            | Pairing::Confirmed { .. } => {}
        }
    }
    let _ = exhaustive;
    let text = String::new;
    vec![
        Pairing::Pair {
            secret: text(),
            device: text(),
            name: text(),
            commit: text(),
        },
        Pairing::Asking {
            device: text(),
            name: text(),
            words: text(),
            nonce: text(),
        },
        Pairing::Reveal { nonce: text() },
        Pairing::Request {
            device: text(),
            name: text(),
        },
        Pairing::Granted {
            grant: text(),
            device: text(),
            name: text(),
        },
        Pairing::Refused {
            reason: PairRefusal::NoOffer,
        },
        Pairing::Confirm { grant: text() },
        Pairing::Confirmed {
            grant: text(),
            state: ConfirmState::Active,
        },
    ]
}

fn revoke_messages() -> Vec<Revoke> {
    fn exhaustive(message: &Revoke) {
        match message {
            Revoke::Revoke { .. } | Revoke::Revoked { .. } => {}
        }
    }
    let _ = exhaustive;
    vec![
        Revoke::Revoke {
            grant: String::new(),
        },
        Revoke::Revoked {
            grant: String::new(),
        },
    ]
}

fn sample_event() -> Event {
    Event {
        v: event::BOX_VERSION,
        outbox: String::new(),
        seq: 1,
        kind: event::KINDS[0].to_owned(),
        key: String::new(),
        at_unix_ms: 0,
        body: json!({}),
    }
}

fn manage_messages() -> Vec<Manage> {
    fn exhaustive(message: &Manage) {
        match message {
            Manage::Resume { .. }
            | Manage::Event(_)
            | Manage::EventAck { .. }
            | Manage::NodeReady { .. }
            | Manage::NodeStopped { .. }
            | Manage::NodeStart { .. }
            | Manage::NodePreparing { .. }
            | Manage::NodeRetry
            | Manage::NodeEnd { .. } => {}
        }
    }
    let _ = exhaustive;
    vec![
        Manage::Resume {
            outbox: String::new(),
            seq: 0,
        },
        Manage::Event(sample_event()),
        Manage::EventAck {
            outbox: String::new(),
            seq: 0,
        },
        Manage::NodeReady {
            build: String::new(),
        },
        Manage::NodeStopped {
            reason: NodeStop::Ended,
            retry: false,
            detail: Some(String::new()),
        },
        Manage::NodeStart {
            build: String::new(),
            build_id: Some(String::new()),
            release: Some(sample_release()),
        },
        Manage::NodePreparing {
            os: String::new(),
            arch: String::new(),
        },
        Manage::NodeRetry,
        Manage::NodeEnd { mode: EndMode::Now },
    ]
}

fn core_messages() -> Vec<CoreMessage> {
    fn exhaustive(message: &CoreMessage) {
        match message {
            CoreMessage::Letter { .. }
            | CoreMessage::LetterTaken { .. }
            | CoreMessage::LetterRefused { .. }
            | CoreMessage::Call { .. }
            | CoreMessage::CallAnswer { .. } => {}
        }
    }
    let _ = exhaustive;
    let text = String::new;
    vec![
        CoreMessage::Letter {
            id: text(),
            from: sample_agent(),
            to: text(),
            reply_to: Some(text()),
            kind: text(),
            intent: text(),
            body: text(),
            now: false,
        },
        CoreMessage::LetterTaken {
            id: text(),
            recipient: sample_agent(),
        },
        CoreMessage::LetterRefused {
            id: text(),
            reason: text(),
        },
        CoreMessage::Call {
            call: 0,
            from: sample_party(),
            ask: Ask::Delivery,
            command: json!({}),
        },
        CoreMessage::CallAnswer {
            call: 0,
            answer: Some(json!({})),
            refused: Some(text()),
        },
    ]
}

fn sample_party() -> Party {
    Party {
        item: Item::Agent,
        id: String::new(),
        name: String::new(),
    }
}

fn sample_agent() -> Agent {
    Agent {
        id: String::new(),
        name: String::new(),
    }
}

fn sample_build_file() -> BuildFile {
    BuildFile {
        path: String::new(),
        size: 0,
        sha256: String::new(),
        executable: false,
    }
}

fn file_messages() -> Vec<FileMessage> {
    fn exhaustive(message: &FileMessage) {
        match message {
            FileMessage::Build { .. }
            | FileMessage::BuildPlaced { .. }
            | FileMessage::Refused { .. } => {}
        }
    }
    let _ = exhaustive;
    vec![
        FileMessage::Build {
            id: String::new(),
            files: vec![sample_build_file()],
        },
        FileMessage::BuildPlaced { id: String::new() },
        FileMessage::Refused {
            reason: String::new(),
        },
    ]
}

fn sample_release() -> Release {
    Release {
        commit: String::new(),
        order: 0,
    }
}

/// The one key of each line a side writes the other after the ready line.
fn line_keys<T: Serialize>(samples: &[T]) -> Value {
    let mut keys: Vec<String> = samples
        .iter()
        .map(
            |sample| match serde_json::to_value(sample).expect("a line serializes") {
                Value::Object(map) if map.len() == 1 => map.keys().next().expect("one key").clone(),
                Value::String(name) => name,
                other => panic!("a line is one key or one word, got {other}"),
            },
        )
        .collect();
    keys.sort();
    json!(keys)
}

fn from_node_lines() -> Vec<FromNode> {
    fn exhaustive(line: &FromNode) {
        match line {
            FromNode::Idle(_) | FromNode::Event(_) | FromNode::BoxFull(_) => {}
        }
    }
    let _ = exhaustive;
    vec![
        FromNode::Idle(true),
        FromNode::Event(sample_event()),
        FromNode::BoxFull(BoxFull {
            pane: Some(String::new()),
        }),
    ]
}

fn to_node_lines() -> Vec<ToNode> {
    fn exhaustive(line: &ToNode) {
        match line {
            ToNode::End(_) | ToNode::Resume(_) | ToNode::Ack(_) | ToNode::Away => {}
        }
    }
    let _ = exhaustive;
    let position = || Position {
        outbox: String::new(),
        seq: 0,
    };
    vec![
        ToNode::End(EndMode::Now),
        ToNode::Resume(position()),
        ToNode::Ack(position()),
        ToNode::Away,
    ]
}

fn hex_byte(byte: u8) -> String {
    format!("0x{byte:02x}")
}

/// What `contracts/peer-link.json` must say.
pub fn shape() -> Value {
    let sample_hello = Hello {
        framing: hello::FRAMING,
        link: LinkRange {
            oldest: hello::OLDEST_LINK,
            newest: hello::NEWEST_LINK,
        },
        capabilities: Vec::new(),
        build: String::new(),
        device: String::new(),
        contract: Some(String::new()),
    };
    let ready = Ready {
        guest_node: launch::GUEST_NODE,
        build: String::new(),
        build_id: String::new(),
        pid: 0,
        link: launch::LINK_ENDPOINT.to_owned(),
        tap: launch::TAP_ENDPOINT.to_owned(),
        tap_versions: [tap::OLDEST_TAP, tap::NEWEST_TAP],
        capabilities: Vec::new(),
    };
    let pane = Pane {
        pane: String::new(),
        rows: 0,
        cols: 0,
        since_unix_ms: 0,
    };
    let streams: Vec<Value> = StreamKind::ALL
        .into_iter()
        .map(|kind| {
            json!({
                "name": kind.name(),
                "tag": kind.tag(),
                "priority": kind.priority(),
                "needs_grant": kind.needs_grant(),
                "since": 1,
            })
        })
        .collect();
    assert!(
        StreamKind::ALL
            .into_iter()
            .all(|kind| name(&kind) == kind.name()),
        "a stream kind has one name"
    );
    json!({
        "alpn": String::from_utf8_lossy(ALPN),
        "key": { "bytes": 32, "spelling": "lower hex", "short_chars": key::SHORT_KEY_CHARS },
        "hello": {
            "magic": String::from_utf8_lossy(&hello::MAGIC),
            "max_bytes": hello::MAX_HELLO,
            "framing": hello::FRAMING,
            "link": { "oldest": hello::OLDEST_LINK, "newest": hello::NEWEST_LINK },
            "capabilities": hello::CAPABILITIES,
            "fields": fields(&sample_hello),
            "link_fields": fields(&sample_hello.link),
            "frames": messages(&HelloFrame::TYPES, &hello_frames()),
            "grant_states": names!(GrantState: None, Pending, Active, Paused, Revoked),
        },
        "streams": {
            "header_bytes": stream::HEADER_BYTES,
            "answer_bytes": stream::ANSWER_BYTES,
            "statuses": Status::ALL
                .into_iter()
                .map(|status| json!({ "name": name(&status), "tag": status.tag() }))
                .collect::<Vec<_>>(),
            "message": { "length_bytes": frame::LENGTH_BYTES, "max_bytes": frame::MAX_MESSAGE },
            "kinds": streams,
        },
        "close_codes": CloseCode::ALL
            .into_iter()
            .map(|code| json!({ "name": code.name(), "number": code.number() }))
            .collect::<Vec<_>>(),
        "pairing": {
            "messages": messages(&Pairing::TYPES, &pairing_messages()),
            "refusals": names!(PairRefusal: NoOffer, Declined, Full, Unanswered, Other),
            "confirm_states": names!(ConfirmState: Active, Paused, Unknown),
        },
        "revoke": { "messages": messages(&Revoke::TYPES, &revoke_messages()) },
        "management": {
            "messages": messages(&Manage::TYPES, &manage_messages()),
            "node_stops": names!(
                NodeStop: Ended, BuildMissing, UpdateNeeded, FolderRefused, Crashed, HostFull, Other
            ),
            "release_fields": fields(&sample_release()),
        },
        "core": {
            "messages": messages(&CoreMessage::TYPES, &core_messages()),
            "party_fields": fields(&sample_party()),
            "agent_fields": fields(&sample_agent()),
            "items": names!(Item: Agent, Pane),
            "asks": names!(Ask: Delivery, Pane),
        },
        "file": {
            "messages": messages(&FileMessage::TYPES, &file_messages()),
            "build_file_fields": fields(&sample_build_file()),
            "build": {
                "id_chars": build::ID_CHARS,
                "max_files": build::MAX_FILES,
                "max_bytes": build::MAX_BYTES,
                "max_path_bytes": build::MAX_PATH,
            },
        },
        "event": {
            "version": event::BOX_VERSION,
            "fields": fields(&sample_event()),
            "kinds": event::KINDS,
            "box": {
                "file": event::BOX_FILE,
                "acked_file": event::ACKED_FILE,
                "acked_fields": fields(&Acked { v: event::BOX_VERSION, outbox: String::new(), seq: 0 }),
                "max_events": event::BOX_MAX_EVENTS,
                "max_bytes": event::BOX_MAX_BYTES,
            },
        },
        "guest": {
            "folder": {
                "guests": launch::GUESTS_DIR,
                "key_file": launch::KEY_FILE,
                "host_side": launch::HOST_SIDE,
                "node_side": launch::NODE_SIDE,
                "install_side": launch::INSTALL_SIDE,
                "builds_kept": launch::BUILDS_KEPT,
                "panes": panes::FILE,
            },
            "install": {
                "name_prefix": launch::INSTALL_PREFIX,
                "hooks_name_prefix": launch::HOOKS_INSTALL_PREFIX,
                "build_chars": 16,
            },
            "sockets": {
                "link": launch::LINK_ENDPOINT,
                "tap": launch::TAP_ENDPOINT,
                "node_tail_bytes": launch::NODE_SOCKET_TAIL,
                "longest_tail_bytes": launch::LONGEST_SOCKET_TAIL,
            },
            "launch": {
                "arguments": launch::ARGUMENTS,
                "ready": {
                    "version": launch::GUEST_NODE,
                    "fields": fields(&ready),
                    "within_secs": launch::READY_WITHIN_SECS,
                },
                "capabilities": launch::NODE_CAPABILITIES,
                "max_line_bytes": launch::MAX_LINE,
                "from_node": line_keys(&from_node_lines()),
                "to_node": line_keys(&to_node_lines()),
                "position_fields": fields(&Position { outbox: String::new(), seq: 0 }),
                "box_full_fields": fields(&BoxFull { pane: Some(String::new()) }),
                "end_modes": names!(EndMode: Now, AfterLastPane),
                "exit_codes": {
                    "ended": launch::EXIT_ENDED,
                    "unknown_word": launch::EXIT_UNKNOWN_WORD,
                    "host_unreachable": launch::EXIT_HOST_UNREACHABLE,
                    "folder_refused": launch::EXIT_FOLDER_REFUSED,
                    "folder_held": launch::EXIT_FOLDER_HELD,
                    "restart": launch::EXIT_RESTART,
                },
                "limits": {
                    "guest_nodes": launch::MAX_GUEST_NODES,
                    "idle_end_secs": launch::IDLE_END_SECS,
                    "end_now_waits_secs": launch::END_NOW_WAITS_SECS,
                    "folder_held_retry_secs": launch::FOLDER_HELD_RETRY_SECS,
                    "folder_held_retries": launch::FOLDER_HELD_RETRIES,
                    "crashes": launch::CRASHES,
                    "crash_window_secs": launch::CRASH_WINDOW_SECS,
                    "holder_lost_restart_secs": launch::HOLDER_LOST_RESTART_SECS,
                },
            },
            "handed_to": {
                "fields": fields(&HandedTo { handed_to: Installation { install: String::new() } }),
                "installation_fields": fields(&Installation { install: String::new() }),
            },
        },
        "tap": {
            "magic": String::from_utf8_lossy(&tap::MAGIC),
            "versions": [tap::OLDEST_TAP, tap::NEWEST_TAP],
            "cap_bytes": tap::CAP_BYTES,
            "max_frame_bytes": tap::MAX_FRAME,
            "to_node": {
                "hello": hex_byte(tap::kind::HELLO),
                "input": hex_byte(tap::kind::INPUT),
                "resize": hex_byte(tap::kind::RESIZE),
                "terminate": hex_byte(tap::kind::TERMINATE),
                "redraw": hex_byte(tap::kind::REDRAW),
            },
            "to_host": {
                "welcome": hex_byte(tap::kind::WELCOME),
                "output": hex_byte(tap::kind::OUTPUT),
                "pane": hex_byte(tap::kind::PANE),
                "gone": hex_byte(tap::kind::GONE),
            },
        },
        "panes": {
            "version": panes::VERSION,
            "fields": fields(&Panes { version: panes::VERSION, panes: Vec::new() }),
            "pane_fields": fields(&pane),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_contract_is_what_the_code_speaks() {
        assert_eq!(
            committed(),
            shape(),
            "contracts/peer-link.json differs from the code; write it again as contracts/README.md, Peer link, says"
        );
    }

    #[test]
    fn the_hash_ignores_rewritten_line_endings() {
        let plain = hash();
        assert_eq!(plain.len(), 64);
        let mut hasher = Sha256::new();
        hasher.update(FILE.replace('\r', "").as_bytes());
        assert_eq!(plain, hex::encode(hasher.finalize()));
    }

    #[test]
    fn a_guest_endpoint_fits_a_macos_socket_path_for_a_16_byte_account() {
        // The state folder of a macOS account whose name is 16 bytes.
        let state = std::path::Path::new("/")
            .join("Users")
            .join("a".repeat(16))
            .join(".hide")
            .join("state");
        let state = state.as_path();
        let longest = launch::longest_endpoint(state);
        assert_eq!(
            longest.as_os_str().len(),
            state.as_os_str().len() + launch::LONGEST_SOCKET_TAIL
        );
        // macOS refuses a Unix socket path of 104 bytes or more.
        assert!(
            longest.as_os_str().len() < 104,
            "{}",
            longest.as_os_str().len()
        );
    }
}
