//! Settings › Bots' call on the bots the operator makes, whose answer
//! carries a one-time code: make a bot, give it a new code, or show its live
//! code again for the sentence the window copies. The same transport as the
//! automations' window call (`automation_screen`), answered to the window
//! that asked and to no other; only this machine's own window may send it
//! (`event_class`, `OwnWindow`), so another machine's view of this one and
//! the phone never see a code. Taking a made bot back is the window event
//! `access_invite_cancel`, whose result the snapshot shows.

use std::sync::Arc;

use hide_core::access::{Call, Mode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::core::CoreHandle;

/// The call as the window writes it, in the frame `{"schema_version":2,
/// "kind":"access_call","payload":{"request_id":…,"call":{"call":"invite",
/// "name":…,"mode":…}}}`; hided adds the id and the code.
#[derive(Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
enum WindowCall {
    Invite { name: String, mode: Mode },
    Renew { id: String },
    Code { id: String },
}

/// Asks the core on a blocking task, since the call waits on the core's
/// owner thread, and hands the answer frame to the window's socket loop; a
/// window gone by then drops it.
pub fn spawn(core: &Arc<CoreHandle>, event: Value, frames: tokio::sync::mpsc::Sender<String>) {
    let core = Arc::clone(core);
    tokio::spawn(async move {
        let request_id = request_id(&event);
        let frame = tokio::task::spawn_blocking(move || {
            answer(&event, fresh, |call| core.access(call))
        })
        .await
        .unwrap_or_else(|error| {
            hide_core::diagnostic!(json!({
                "component": "access", "kind": "access.window_call_failed",
                "request_id": request_id, "reason": error.to_string(),
            }));
            result(&request_id, Err("access_unavailable".to_owned()))
        });
        let _ = frames.send(frame.to_string()).await;
    });
}

/// A new id and code, from the system's random source.
fn fresh() -> Option<(String, String)> {
    Some((crate::access::fresh_id()?, crate::access::fresh_code()?))
}

fn request_id(event: &Value) -> String {
    event
        .pointer("/payload/request_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The `access_result` frame for `event`, the call answered by `call`. A
/// frame with no request id or no call this reads is refused before the
/// core is asked.
fn answer(
    event: &Value,
    fresh: impl FnOnce() -> Option<(String, String)>,
    call: impl FnOnce(Call) -> Result<Value, String>,
) -> Value {
    let request_id = request_id(event);
    if request_id.is_empty() {
        return result(&request_id, Err("access_call_invalid".to_owned()));
    }
    let Some(asked) = event
        .pointer("/payload/call")
        .cloned()
        .and_then(|value| serde_json::from_value::<WindowCall>(value).ok())
    else {
        return result(&request_id, Err("access_call_invalid".to_owned()));
    };
    let core_call = match asked {
        WindowCall::Code { id } => Some(Call::Code { id }),
        WindowCall::Invite { name, mode } => {
            fresh().map(|(id, code)| Call::Invite { id, code, name, mode })
        }
        WindowCall::Renew { id } => fresh().map(|(_, code)| Call::Renew { id, code }),
    };
    let answer = core_call
        .ok_or_else(|| "access_unavailable".to_owned())
        .and_then(call);
    result(&request_id, answer)
}

fn result(request_id: &str, answer: Result<Value, String>) -> Value {
    let payload = match answer {
        Ok(value) => json!({"request_id": request_id, "ok": true, "value": value}),
        Err(code) => json!({"request_id": request_id, "ok": false, "code": code}),
    };
    json!({"type": "access_result", "payload": payload})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(payload: Value) -> Value {
        json!({"schema_version": 2, "kind": "access_call", "payload": payload})
    }

    fn fixed() -> Option<(String, String)> {
        Some(("00000000000000c1".into(), "K7Q4MZ2PX3RD".into()))
    }

    #[test]
    fn a_make_reaches_the_core_with_the_id_and_code_hided_drew() {
        let frame = answer(
            &event(json!({"request_id": "r1", "call": {"call": "invite", "name": "hermes", "mode": {"kind": "own"}}})),
            fixed,
            |call| {
                assert_eq!(
                    call,
                    Call::Invite {
                        id: "00000000000000c1".into(),
                        code: "K7Q4MZ2PX3RD".into(),
                        name: "hermes".into(),
                        mode: Mode::Own,
                    }
                );
                Ok(json!({"code": "K7Q4-MZ2P-X3RD"}))
            },
        );
        assert_eq!(frame["type"], "access_result");
        assert_eq!(
            frame["payload"],
            json!({"request_id": "r1", "ok": true, "value": {"code": "K7Q4-MZ2P-X3RD"}})
        );
    }

    #[test]
    fn the_core_s_refusal_reaches_the_window_as_its_code() {
        let frame = answer(
            &event(json!({"request_id": "r2", "call": {"call": "code", "id": "00000000000000c1"}})),
            || panic!("a code asks for no new code"),
            |_| Err("access_invite_expired".to_owned()),
        );
        assert_eq!(
            frame["payload"],
            json!({"request_id": "r2", "ok": false, "code": "access_invite_expired"})
        );
    }

    #[test]
    fn a_call_this_cannot_read_is_refused_before_the_core_is_asked() {
        for payload in [
            json!({"request_id": "r3", "call": {"call": "approve", "id": "a"}}),
            json!({"request_id": "r3", "call": {"call": "invite", "name": "x", "mode": {"kind": "own"}, "code": "K7Q4MZ2PX3RD"}}),
            json!({"request_id": "r3"}),
            json!({"call": {"call": "code", "id": "a"}}),
        ] {
            let frame = answer(&event(payload.clone()), fixed, |_| {
                panic!("asked the core for {payload}")
            });
            assert_eq!(frame["payload"]["ok"], false);
            assert_eq!(frame["payload"]["code"], "access_call_invalid");
        }
    }
}
