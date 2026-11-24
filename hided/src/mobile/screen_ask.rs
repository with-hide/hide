//! An agent's ask answered from a window (PRD desk-2-ui B14): the session
//! panel beside Desk shows the question card the phone's detail shows and
//! answers it the way a phone does. `pane_ask` reads the ask as the phone's
//! `ask` frame says it (`ask::of`), and `pane_choice` types the keys a
//! phone's choice types (`ask::keys`, then `operator_input::write`, which
//! checks the screen before each key); the refusals are the phone's.
//!
//! The phone's follower keeps a projection only while a phone is paired, so
//! both read the core at the moment they are asked, one read per press, and
//! the choice reads it again at the write, so an agent that left the ask in
//! between is refused `stale` and nothing is typed. A second press of the
//! same choice finds the screen no longer showing the ask and types nothing.

use std::sync::Arc;

use hide_core::operator_input::{self, Screen, Typed};
use serde_json::{Value, json};

use super::Mobile;
use super::ask;
use super::calls::Calls;
use super::conversation;
use super::pane;
use super::phone::{follow_calls, key_of, request_id_of};
use super::projection::{self, AgentKey, Projection};

/// The `pane_ask` frame: the ask `{device_id, pane_id}`'s agent waits on, as
/// the phone's `ask` frame body, or `null` with why.
pub fn ask_frame(mobile: &Mobile, payload: &Value) -> Value {
    let key = key_of(payload);
    let read = key.as_ref().ok_or("invalid_request").and_then(|key| {
        let projection = projection_now(mobile)?;
        shown(mobile, &projection, key)
    });
    let (ask, reason) = match read {
        Ok(Some(shown)) => (shown.frame, None),
        Ok(None) => (Value::Null, Some("not_asking")),
        Err(reason) => (Value::Null, Some(reason)),
    };
    json!({"type": "pane_ask", "payload": {
        "device_id": key.as_ref().map(|key| key.device_id.as_str()),
        "pane_id": key.as_ref().map(|key| key.pane_id.as_str()),
        "ask": ask,
        "reason": reason,
    }})
}

/// The `pane_choice_result` frame for a `pane_choice` {request_id, device_id,
/// pane_id, turn, kind, choice}: `ok`, or the phone's reason nothing was
/// typed (`stale`, `unknown_choice`, `invalid_request`, `gone`,
/// `device_unreachable`, `unavailable`), or `uncertain` when keys may have
/// landed. Blocking: it reads the core and the session, and writes.
pub fn choose(mobile: &Arc<Mobile>, payload: &Value) -> Value {
    let request_id = request_id_of(payload).map(str::to_owned);
    let result = request_id
        .as_ref()
        .ok_or("invalid_request")
        .and_then(|_| write_choice(mobile, payload));
    let reason = result.err();
    hide_core::diagnostic!(json!({
        "component": "screen_ask", "kind": if reason.is_none() { "choice.sent" } else { "choice.refused" },
        "device_id": payload.get("device_id"), "pane_id": payload.get("pane_id"), "reason": reason,
    }));
    json!({"type": "pane_choice_result", "payload": {
        "request_id": request_id,
        "ok": reason.is_none(),
        "reason": reason,
    }})
}

fn write_choice(mobile: &Arc<Mobile>, payload: &Value) -> Result<(), &'static str> {
    let projection = projection_now(mobile)?;
    let key = key_of(payload).ok_or("invalid_request")?;
    let shown = shown(mobile, &projection, &key)?;
    let (key, keys, screen) = chosen(&projection, shown.as_ref(), payload)?;
    let typed: Vec<&str> = keys.iter().map(String::as_str).collect();
    // The write's last look, as a phone's: the agent still asks what was
    // shown, then the node's permit and the gate.
    let ready = || {
        let now = projection_now(mobile).map_err(|_| "stale")?;
        if chosen(&now, shown.as_ref(), payload).map_err(|_| "stale")?
            != (key.clone(), keys.clone(), screen.clone())
        {
            return Err("stale");
        }
        let engine = mobile.engine(&key.device_id).map_err(|error| match error {
            pane::PaneError::DeviceUnreachable => "device_unreachable",
            _ => "unavailable",
        })?;
        mobile.note_input(&key.pane_id);
        Ok(engine)
    };
    operator_input::write(
        mobile.terminals(),
        mobile.input_gate(),
        &key.pane_id,
        key.engine_pane_id(),
        Typed::Choice {
            keys: &typed,
            screen: &screen,
        },
        &ready,
    )
    .map_err(|unwritten| {
        if unwritten.certain() {
            pane::unwritten_reason(unwritten)
        } else {
            "uncertain"
        }
    })
}

/// The keys a choice types and what the screen must show while they are
/// typed, judged as a phone's choice is (`ask::keys`) against the ask the
/// window was shown: a `turn` the agent has left or another `kind` is
/// `stale`, a choice its keys do not reach `unknown_choice`.
fn chosen(
    projection: &Projection,
    shown: Option<&ask::Shown>,
    payload: &Value,
) -> Result<(AgentKey, Vec<String>, Screen), &'static str> {
    let key = key_of(payload).ok_or("invalid_request")?;
    let field = |name: &str| payload.get(name).and_then(Value::as_str);
    let (Some(turn), Some(kind), Some(choice)) = (field("turn"), field("kind"), field("choice"))
    else {
        return Err("invalid_request");
    };
    let agent = projection.find(&key).ok_or("gone")?;
    let (keys, screen) = ask::keys(agent, shown, turn, kind, choice)?;
    Ok((key, keys, screen))
}

/// The ask `key`'s agent waits on now, as `ask::of` says it with the
/// session's open calls, read on this machine; a device's agent is judged
/// without them, as the phone judges it.
fn shown(
    mobile: &Mobile,
    projection: &Projection,
    key: &AgentKey,
) -> Result<Option<ask::Shown>, &'static str> {
    let agent = projection.find(key).ok_or("gone")?;
    let calls = (key.device_id == mobile.node().as_str())
        .then(|| open_calls(mobile, key))
        .flatten();
    let pending = calls.as_ref().map(Calls::pending);
    Ok(ask::of(agent, pending.as_ref()))
}

fn open_calls(mobile: &Mobile, key: &AgentKey) -> Option<Calls> {
    let engine = mobile.engine(mobile.node().as_str()).ok()?;
    let source = conversation::source(&engine, key.engine_pane_id()).ok()??;
    let account = mobile.core().pane_account(key.engine_pane_id())?;
    follow_calls(mobile, &source, &account, None, true)
}

/// The agent list as the core holds it now.
fn projection_now(mobile: &Mobile) -> Result<Projection, &'static str> {
    let payload = mobile.core().snapshot(0).map_err(|_| "unavailable")?;
    let mut rest = Value::Null;
    projection::update_inputs(&mut rest, &payload.current_rest, payload.have_revision)
        .map_err(|_| "unavailable")?;
    Ok(projection::project(&rest, mobile.node().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile::calls::{OpenCall, Pending};
    use hide_core::operator_input::Ask;

    fn asking(changed_at: u64) -> Projection {
        let agent = json!({
            "pane_id": "w1:p1", "group": "needs_you", "mark": "turn", "identity_label": "Review parser",
            "agent_kind": "claude", "status_code": "approval", "demand": "approval", "changed_at_unix_ms": changed_at,
        });
        projection::project(&json!({"navigator": {"agents": [agent]}}), "test-node")
    }

    fn choice(choice: &str, turn: &str, kind: &str) -> Value {
        json!({"request_id": "r1", "device_id": "test-node", "pane_id": "w1:p1", "choice": choice, "turn": turn, "kind": kind})
    }

    /// A window's choice on a permission Claude Code waits on types the keys
    /// Claude Code's own screen takes for it, the ones a phone's choice types,
    /// and is refused for the phone's reasons.
    #[test]
    fn a_windows_choice_types_the_phones_keys_and_is_refused_as_a_phones() {
        let waiting = asking(5);
        let key = key_of(&choice("allow", "5", "permission")).unwrap();
        let agent = waiting.find(&key).unwrap();
        let open = OpenCall {
            id: "toolu_1".to_owned(),
            command: Some("cargo test".to_owned()),
            plan: false,
            since_ms: Some(4),
        };
        let shown = ask::of(agent, Some(&Pending::One(&open))).unwrap();

        let (_, keys, screen) =
            chosen(&waiting, Some(&shown), &choice("allow", "5", "permission")).unwrap();
        assert_eq!(
            Some(keys),
            operator_input::choice_keys("claude", Ask::Permission, "allow"),
            "the keys Claude Code's screen takes for `allow`"
        );
        assert_eq!(
            screen,
            operator_input::choice_screen("claude", Ask::Permission, "allow", &[])
                .showing("cargo test"),
            "typed only while the screen shows the command the card showed"
        );

        for (payload, shown_then, expected) in [
            (choice("allow", "4", "permission"), Some(&shown), "stale"),
            (
                choice("approve", "5", "plan_approval"),
                Some(&shown),
                "stale",
            ),
            (choice("allow", "5", "permission"), None, "stale"),
            (
                choice("approve", "5", "permission"),
                Some(&shown),
                "unknown_choice",
            ),
            (
                json!({"device_id": "test-node", "pane_id": "w1:p1", "choice": "allow", "kind": "permission"}),
                Some(&shown),
                "invalid_request",
            ),
        ] {
            assert_eq!(
                chosen(&waiting, shown_then, &payload).err(),
                Some(expected),
                "{payload}"
            );
        }
        let gone = projection::project(&json!({"navigator": {"agents": []}}), "test-node");
        assert_eq!(
            chosen(&gone, Some(&shown), &choice("allow", "5", "permission")).err(),
            Some("gone")
        );
    }
}
