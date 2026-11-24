//! An agent's ask answered from a window (PRD desk-2-ui B14): the session
//! panel beside Desk shows the question card the phone's detail shows and
//! answers it the way a phone does. `pane_ask` reads the ask as the phone's
//! `ask` frame says it (`ask::of`), and `pane_choice` types the keys a
//! phone's choice types (`ask::keys`, then `operator_input::write`, which
//! checks the screen before each key); the refusals are the phone's.
//!
//! A phone's choice is checked against the ask the daemon last pushed it; a
//! window is pushed nothing, so its choice carries back the `ask` body its
//! card drew, and is `stale` unless the agent asks exactly that at the press
//! and again at the write: the same call, command, words and choices, not
//! only the same turn, which two asks can share.
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
/// pane_id, choice, ask}: `ok`, or the phone's reason nothing was
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
        let asks = self::shown(mobile, &now, &key).map_err(|_| "stale")?;
        still_chosen(
            &now,
            asks.as_ref(),
            payload,
            &(key.clone(), keys.clone(), screen.clone()),
        )?;
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

/// The largest `ask` body a choice may carry back, well past what
/// `ask::of` draws (its command and words are cut).
const ASK_BYTES_MAX: usize = 64 * 1024;

/// The keys a choice types and what the screen must show while they are
/// typed, judged as a phone's choice is (`ask::keys`) against the ask the
/// agent waits on now (`shown`), which must be the one the window's card
/// drew (`ask`, whose `turn` and `kind` the choice names): another ask is
/// `stale`, a choice its keys do not reach `unknown_choice`.
fn chosen(
    projection: &Projection,
    shown: Option<&ask::Shown>,
    payload: &Value,
) -> Result<(AgentKey, Vec<String>, Screen), &'static str> {
    let key = key_of(payload).ok_or("invalid_request")?;
    let drawn = payload
        .get("ask")
        .filter(|ask| ask.is_object() && ask.to_string().len() <= ASK_BYTES_MAX)
        .ok_or("invalid_request")?;
    fn text<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
        value.get(name).and_then(Value::as_str)
    }
    let (Some(turn), Some(kind), Some(choice)) = (
        text(drawn, "turn"),
        text(drawn, "kind"),
        text(payload, "choice"),
    ) else {
        return Err("invalid_request");
    };
    let agent = projection.find(&key).ok_or("gone")?;
    let shown = shown.filter(|shown| &shown.frame == drawn);
    let (keys, screen) = ask::keys(agent, shown, turn, kind, choice)?;
    Ok((key, keys, screen))
}

/// The write's last look: the agent still asks what the card drew, and the
/// choice still types the same keys against the same screen; anything else
/// is `stale` and nothing is typed.
fn still_chosen(
    projection: &Projection,
    shown: Option<&ask::Shown>,
    payload: &Value,
    first: &(AgentKey, Vec<String>, Screen),
) -> Result<(), &'static str> {
    match chosen(projection, shown, payload) {
        Ok(now) if &now == first => Ok(()),
        _ => Err("stale"),
    }
}

/// The ask `key`'s agent waits on now, as `ask::of` says it with the
/// session's open calls, read on this machine; a device's agent is judged
/// without them, as the phone judges it.
fn shown(
    mobile: &Mobile,
    projection: &Projection,
    key: &AgentKey,
) -> Result<Option<ask::Shown>, &'static str> {
    projection.find(key).ok_or("gone")?;
    let calls = (key.device_id == mobile.node().as_str())
        .then(|| open_calls(mobile, key))
        .flatten();
    asked(projection, key, calls.as_ref())
}

/// The ask `key`'s agent waits on, given the session's open calls.
fn asked(
    projection: &Projection,
    key: &AgentKey,
    calls: Option<&Calls>,
) -> Result<Option<ask::Shown>, &'static str> {
    let agent = projection.find(key).ok_or("gone")?;
    let pending = calls.map(Calls::pending);
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

    fn agents(group: &str, changed_at: u64) -> Projection {
        let agent = json!({
            "pane_id": "w1:p1", "group": group, "mark": "turn", "identity_label": "Review parser",
            "agent_kind": "claude", "status_code": "approval", "demand": "approval", "changed_at_unix_ms": changed_at,
        });
        projection::project(&json!({"navigator": {"agents": [agent]}}), "test-node")
    }

    fn open(id: &str, command: &str) -> OpenCall {
        OpenCall {
            id: id.to_owned(),
            command: Some(command.to_owned()),
            plan: false,
            since_ms: Some(4),
        }
    }

    /// The permission `command` an agent asking at turn 5 shows.
    fn shown_on(projection: &Projection, call: &OpenCall) -> ask::Shown {
        let key = key_of(&json!({"device_id": "test-node", "pane_id": "w1:p1"})).unwrap();
        ask::of(projection.find(&key).unwrap(), Some(&Pending::One(call))).unwrap()
    }

    /// A press of `choice` on a card that drew `drawn`, its `ask` sent
    /// through a JSON string and back, as a window's is.
    fn choice(choice: &str, drawn: &Value) -> Value {
        let ask: Value = serde_json::from_str(&drawn.to_string()).unwrap();
        json!({"request_id": "r1", "device_id": "test-node", "pane_id": "w1:p1",
            "choice": choice, "ask": ask})
    }

    /// `drawn` with `field` changed, as a card of another ask would carry.
    fn with(drawn: &ask::Shown, field: &str, value: Value) -> Value {
        let mut frame = drawn.frame.clone();
        frame[field] = value;
        frame
    }

    /// A window's choice on a permission Claude Code waits on types the keys
    /// Claude Code's own screen takes for it, the ones a phone's choice types,
    /// and is refused for the phone's reasons.
    #[test]
    fn a_windows_choice_types_the_phones_keys_and_is_refused_as_a_phones() {
        let waiting = agents("needs_you", 5);
        let shown = shown_on(&waiting, &open("toolu_1", "cargo test"));
        let allow = choice("allow", &shown.frame);

        let first = chosen(&waiting, Some(&shown), &allow).unwrap();
        assert_eq!(
            Some(first.1.clone()),
            operator_input::choice_keys("claude", Ask::Permission, "allow"),
            "the keys Claude Code's screen takes for `allow`"
        );
        assert_eq!(
            first.2,
            operator_input::choice_screen("claude", Ask::Permission, "allow", &[])
                .showing("cargo test"),
            "typed only while the screen shows the command the card showed"
        );
        assert_eq!(still_chosen(&waiting, Some(&shown), &allow, &first), Ok(()));

        let mut without_ask = allow.clone();
        without_ask.as_object_mut().unwrap().remove("ask");
        let mut without_choice = allow.clone();
        without_choice.as_object_mut().unwrap().remove("choice");
        let huge = json!("x".repeat(ASK_BYTES_MAX));
        for (payload, shown_then, expected) in [
            (
                choice("allow", &with(&shown, "turn", json!("4"))),
                Some(&shown),
                "stale",
            ),
            (
                choice("approve", &with(&shown, "kind", json!("plan_approval"))),
                Some(&shown),
                "stale",
            ),
            (allow.clone(), None, "stale"),
            (
                choice("approve", &shown.frame),
                Some(&shown),
                "unknown_choice",
            ),
            (without_choice, Some(&shown), "invalid_request"),
            (without_ask, Some(&shown), "invalid_request"),
            (
                choice("allow", &with(&shown, "text", huge)),
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
        assert_eq!(chosen(&gone, Some(&shown), &allow).err(), Some("gone"));
    }

    /// The agent answered command A in its terminal and asks for B within
    /// the same turn: a press on the card that still shows A is `stale`, at
    /// the press and at the write's last look (the agent moved to B between
    /// the two reads), and types nothing.
    #[test]
    fn a_choice_on_a_card_the_agent_no_longer_asks_is_stale_even_in_the_same_turn() {
        let waiting = agents("needs_you", 5);
        let card = shown_on(&waiting, &open("toolu_1", "cargo test"));
        let now = shown_on(&waiting, &open("toolu_2", "rm -rf target"));
        assert_eq!(
            (now.turn.as_str(), now.kind),
            (card.turn.as_str(), card.kind),
            "one turn, one kind"
        );
        let allow = choice("allow", &card.frame);

        assert_eq!(chosen(&waiting, Some(&now), &allow).err(), Some("stale"));
        let first = chosen(&waiting, Some(&card), &allow).unwrap();
        assert_eq!(
            still_chosen(&waiting, Some(&now), &allow, &first),
            Err("stale")
        );
        assert_eq!(
            still_chosen(&waiting, None, &allow, &first),
            Err("stale"),
            "left the ask"
        );
    }

    /// A question whose choices changed within one turn: "2" on the old
    /// card would land on the new row 2, so it is `stale`.
    #[test]
    fn a_choice_on_a_question_whose_choices_changed_in_the_same_turn_is_stale() {
        let question = |choices: Value| {
            let agent = json!({"pane_id": "w1:p1", "group": "needs_you", "mark": "turn",
                "identity_label": "Review", "changed_at_unix_ms": 5, "agent_kind": "claude",
                "status_code": "question", "demand": "question",
                "user_turn": {"kind": "question", "content": {"text": "어느 쪽?", "choices": choices,
                    "notes": ["", ""], "pick_one": true, "truncated": false}}});
            projection::project(&json!({"navigator": {"agents": [agent]}}), "test-node")
        };
        let key = key_of(&json!({"device_id": "test-node", "pane_id": "w1:p1"})).unwrap();
        let before = question(json!(["전부", "나눠서"]));
        let after = question(json!(["전부", "버리기"]));
        let card = ask::of(before.find(&key).unwrap(), None).unwrap();
        let now = ask::of(after.find(&key).unwrap(), None).unwrap();
        assert_eq!(
            (now.turn.as_str(), now.kind),
            (card.turn.as_str(), card.kind)
        );

        let second = choice("2", &card.frame);
        assert!(chosen(&before, Some(&card), &second).is_ok());
        assert_eq!(chosen(&after, Some(&now), &second).err(), Some("stale"));
    }

    /// `pane_ask` on an agent that asks nothing answers `not_asking`'s read.
    #[test]
    fn an_agent_that_asks_nothing_reads_as_no_ask() {
        let key = key_of(&json!({"device_id": "test-node", "pane_id": "w1:p1"})).unwrap();
        let working = projection::project(
            &json!({"navigator": {"agents": [{"pane_id": "w1:p1", "group": "working", "agent_kind": "claude", "status_code": "working", "changed_at_unix_ms": 5}]}}),
            "test-node",
        );
        assert_eq!(asked(&working, &key, None), Ok(None));
        let gone = projection::project(&json!({"navigator": {"agents": []}}), "test-node");
        assert_eq!(asked(&gone, &key, None).err(), Some("gone"));
    }

    /// Through a daemon's own Mobile: a pane no agent runs in is `gone` and a
    /// request with no pane or no request id is `invalid_request`, each
    /// refused before the write. A Mobile starts its loops on a runtime, and
    /// these calls block as on the daemon's blocking pool.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_without_an_agent_answers_gone_and_a_choice_there_writes_nothing() {
        tokio::task::block_in_place(pane_without_an_agent);
    }

    fn pane_without_an_agent() {
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(hide_core::input_gate::PaneInputGate::default());
        let mobile = crate::mobile::test_mobile(directory.path(), &gate);
        let pane = json!({"device_id": mobile.node().as_str(), "pane_id": "w1:p1"});

        let asked = ask_frame(&mobile, &pane);
        assert_eq!(asked["type"], "pane_ask");
        assert_eq!(asked["payload"]["ask"], Value::Null);
        assert_eq!(asked["payload"]["reason"], "gone");
        assert_eq!(
            ask_frame(&mobile, &json!({}))["payload"]["reason"],
            "invalid_request"
        );

        let mut press = pane.clone();
        press.as_object_mut().unwrap().extend(
            json!({"request_id": "r1", "choice": "allow", "ask": {"kind": "permission", "turn": "5"}})
                .as_object()
                .unwrap()
                .clone(),
        );
        let answered = choose(&mobile, &press);
        assert_eq!(answered["type"], "pane_choice_result");
        assert_eq!(
            answered["payload"],
            json!({"request_id": "r1", "ok": false, "reason": "gone"})
        );
        let mut unnamed = press.clone();
        unnamed.as_object_mut().unwrap().remove("request_id");
        assert_eq!(
            choose(&mobile, &unnamed)["payload"]["reason"],
            "invalid_request"
        );
    }
}
