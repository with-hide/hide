//! What a pane of a guest's node hears from its own `hide` while the
//! guest's core is not connected (PRD device-connect-iroh B12, B13).
//!
//! A pane's `hide` reaches its core through the pane service of the node's
//! link, which lives only as long as a link does. With no link nothing
//! answers there, so the same `hide` asks here instead, on an endpoint in
//! the node's folder that is bound for the node's whole life: a letter it
//! sends is kept in the node's box, with the terminal the node proved its
//! sender runs in, and answered `queued`, and anything that needs the
//! core's judgement is answered `owner_offline` at once.
//!
//! Both ends are this guest's own build, one line each way; the host's
//! build never reads the endpoint.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hide_host::pane_holders::PaneSlot;
use hide_platform::ipc::{LocalListener, LocalStream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::event_box::{EventBox, Refused};

/// The endpoint's name in the node's folder.
pub const ENDPOINT: &str = "away.sock";

/// The reason a command that needs the guest's core gets.
pub const OWNER_OFFLINE: &str = "owner_offline";
/// The reason a letter gets when the box has no room for it (B13).
pub const OUTBOX_FULL: &str = "outbox_full";
/// The reason a letter gets when the node could not open or write its box.
pub const OUTBOX_UNAVAILABLE: &str = "outbox_unavailable";

/// Every reason this service gives a pane's `hide`.
pub const REFUSALS: &[&str] = &[OWNER_OFFLINE, OUTBOX_FULL, OUTBOX_UNAVAILABLE];

/// How long one caller has for its line, and the node for its answer.
const EXCHANGE_WITHIN: Duration = Duration::from_secs(5);
/// How long a `hide` waits for the endpoint to take its connection: it is
/// on this machine and bound, or it is not there.
const CONNECT_WITHIN: Duration = Duration::from_millis(500);
/// The longest line a caller writes: a letter's body and its few names.
const MAX_ASK: u64 = (hide_core::delivery::BODY_LIMIT + 4096) as u64;

/// What a pane's `hide` asks.
#[derive(Serialize, Deserialize)]
struct Ask {
    /// The pane the caller says it runs in.
    #[serde(default)]
    pane_id: String,
    /// The delivery command it would have run, when it is one.
    #[serde(default)]
    command: Option<Value>,
}

pub fn endpoint(node_dir: &Path) -> PathBuf {
    node_dir.join(ENDPOINT)
}

/// Answers every caller of `listener` until the node ends. One caller at a
/// time: each exchange is two lines within [`EXCHANGE_WITHIN`].
pub fn serve(listener: &LocalListener, panes: &PaneSlot, events: Option<&Arc<EventBox>>) {
    while let Ok(mut stream) = listener.accept() {
        stream.set_deadline(Instant::now() + EXCHANGE_WITHIN);
        let answer = answer(&stream, panes, events);
        // A caller that is gone hears nothing; what was kept stays kept.
        let _ = writeln!(stream, "{answer}");
    }
}

fn refused(reason: &str) -> Value {
    json!({"ok": false, "reason": reason})
}

fn answer(stream: &LocalStream, panes: &PaneSlot, events: Option<&Arc<EventBox>>) -> Value {
    let mut line = String::new();
    if BufReader::new(stream.duplicate())
        .take(MAX_ASK)
        .read_line(&mut line)
        .is_err()
    {
        return refused("invalid_request");
    }
    let Ok(ask) = serde_json::from_str::<Ask>(&line) else {
        return refused("invalid_request");
    };
    // Only a letter waits for the core; every other command needs its
    // judgement now.
    let Some((command, intent)) = ask.command.as_ref().and_then(|command| {
        let intent = command.get("intent")?.as_str()?;
        (command.get("op")?.as_str()? == "send").then_some((command, intent))
    }) else {
        return refused(OWNER_OFFLINE);
    };
    // A letter is kept only for the pane this machine's kernel says the
    // caller runs in, and it names that pane's terminal: one life of one
    // pane, by which the guest's core knows the agent it registered there
    // whether or not the pane still runs when the letter arrives.
    let identity = match stream
        .peer_pid()
        .ok()
        .and_then(|pid| i32::try_from(pid).ok())
        .ok_or("caller_unavailable")
        .and_then(|peer| hide_host::panes::prove_caller(panes, &ask.pane_id, peer))
    {
        Ok(identity) => identity,
        Err(reason) => return refused(reason),
    };
    let Some(events) = events else {
        return refused(OUTBOX_UNAVAILABLE);
    };
    let kept = events.push(
        "letter",
        // A letter is one per sender and intent.
        &format!("{}/{intent}", ask.pane_id),
        super::now_unix_ms(),
        json!({"pane": ask.pane_id, "terminal": identity.terminal_id, "command": command}),
    );
    match kept {
        Ok(()) => json!({"ok": true, "result": {"queued": true, "intent": intent}}),
        Err(Refused::Full) => refused(OUTBOX_FULL),
        Err(Refused::TooLarge) => refused("invalid_request"),
        Err(Refused::Failed(error)) => {
            super::record(
                "guest_node.box_write_failed",
                json!({"pane": ask.pane_id, "error": error.to_string()}),
            );
            refused(OUTBOX_UNAVAILABLE)
        }
    }
}

/// Asks the node whose folder is `node_dir` on behalf of the pane
/// `pane_id`: `None` when no guest's node listens there, which is every
/// `hide` that is not in a guest's pane; otherwise the node's one line.
pub fn ask(node_dir: &Path, pane_id: Option<&str>, command: Option<&Value>) -> Option<Value> {
    let path = endpoint(node_dir);
    // Most callers are in no guest's pane: they pay one look at a name.
    if !path.exists() {
        return None;
    }
    let mut stream = LocalStream::connect_until(&path, Instant::now() + CONNECT_WITHIN).ok()?;
    stream.set_deadline(Instant::now() + EXCHANGE_WITHIN);
    let ask = serde_json::to_string(&Ask {
        pane_id: pane_id.unwrap_or_default().to_owned(),
        command: command.cloned(),
    })
    .ok()?;
    writeln!(stream, "{ask}").ok()?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(4096)
        .read_line(&mut line)
        .ok()?;
    serde_json::from_str(&line).ok()
}

/// What a caller does about a reason this service gave.
pub fn next_action(reason: &str) -> Option<&'static str> {
    match reason {
        OWNER_OFFLINE => Some(
            "The Hide this work belongs to is not connected and reconnects by itself; run the command again then. A letter (hide request send) is kept and delivered once it is back",
        ),
        OUTBOX_FULL => Some(
            "Too many letters wait for the Hide this work belongs to; send again once it has reconnected",
        ),
        OUTBOX_UNAVAILABLE => Some(
            "This machine could not keep the letter; send again once the Hide this work belongs to has reconnected",
        ),
        _ => None,
    }
}
