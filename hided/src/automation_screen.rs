//! A window's call on the automations that waits for the core's answer (PRD
//! automations B6-B8): the form's preview of a schedule, its make and its
//! edit, whose refusal the form shows under the field and whose made id it
//! opens. The same `hide_core::automations::Call` the CLI's `hide
//! automation` sends, answered to this window alone; the window events
//! `automation_*` stay the way for a press whose answer the snapshot says.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::core::CoreHandle;

/// The frame a window sends: `{"schema_version":2,"kind":"automation_call",
/// "payload":{"request_id":…,"call":{"call":"preview",…}}}`.
pub const KIND: &str = "automation_call";

/// Asks the core on a blocking task, since the call waits on the core's
/// owner thread, and hands the answer frame to the window's socket loop; a
/// window gone by then drops it.
pub fn spawn(
    core: &Arc<CoreHandle>,
    event: Value,
    screen: Option<hide_core::LookScreen>,
    frames: tokio::sync::mpsc::Sender<String>,
) {
    let core = Arc::clone(core);
    tokio::spawn(async move {
        let request_id = request_id(&event);
        let frame = tokio::task::spawn_blocking(move || {
            answer(&event, |call| core.automation_from(call, screen))
        })
        .await
        .unwrap_or_else(|error| {
            hide_core::diagnostic!(json!({
                "component": "automations", "kind": "automation.window_call_failed",
                "request_id": request_id, "reason": error.to_string(),
            }));
            result(&request_id, Err("automations_unavailable".to_owned()))
        });
        let _ = frames.send(frame.to_string()).await;
    });
}

fn request_id(event: &Value) -> String {
    event
        .pointer("/payload/request_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The `automation_result` frame for `event`, the call answered by `call`.
/// A frame with no request id or no call the core reads is refused before
/// the core is asked.
fn answer(
    event: &Value,
    call: impl FnOnce(hide_core::automations::Call) -> Result<Value, String>,
) -> Value {
    let request_id = request_id(event);
    if request_id.is_empty() {
        return result(&request_id, Err("automation_call_invalid".to_owned()));
    }
    let parsed = event
        .pointer("/payload/call")
        .cloned()
        .ok_or_else(|| "automation_call_invalid".to_owned())
        .and_then(|value| {
            serde_json::from_value::<hide_core::automations::Call>(value)
                .map_err(|_| "automation_call_invalid".to_owned())
        });
    result(&request_id, parsed.and_then(call))
}

fn result(request_id: &str, answer: Result<Value, String>) -> Value {
    let payload = match answer {
        Ok(value) => json!({"request_id": request_id, "ok": true, "value": value}),
        Err(code) => json!({"request_id": request_id, "ok": false, "code": code}),
    };
    json!({"type": "automation_result", "payload": payload})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(payload: Value) -> Value {
        json!({"schema_version": 2, "kind": KIND, "payload": payload})
    }

    #[test]
    fn a_call_the_core_answers_reaches_the_window_under_its_request_id() {
        let frame = answer(
            &event(
                json!({"request_id": "r1", "call": {"call": "preview", "schedule": {"kind": "daily", "time": "09:00"}}}),
            ),
            |call| {
                assert!(matches!(call, hide_core::automations::Call::Preview { .. }));
                Ok(json!({"next_run_at_unix_ms": 1}))
            },
        );
        assert_eq!(frame["type"], "automation_result");
        assert_eq!(
            frame["payload"],
            json!({"request_id": "r1", "ok": true, "value": {"next_run_at_unix_ms": 1}})
        );
    }

    #[test]
    fn the_core_s_refusal_reaches_the_window_as_its_code() {
        let frame = answer(
            &event(
                json!({"request_id": "r2", "call": {"call": "preview", "schedule": {"kind": "cron", "expr": "*/4 * * * *"}, "name": "점검"}}),
            ),
            |_| Err("schedule_too_often".to_owned()),
        );
        assert_eq!(
            frame["payload"],
            json!({"request_id": "r2", "ok": false, "code": "schedule_too_often"})
        );
    }

    #[test]
    fn a_call_the_core_cannot_read_is_refused_before_the_core_is_asked() {
        for payload in [
            json!({"request_id": "r3", "call": {"call": "launch"}}),
            json!({"request_id": "r3"}),
            json!({"call": {"call": "list"}}),
        ] {
            let frame = answer(&event(payload.clone()), |_| {
                panic!("asked the core for {payload}")
            });
            assert_eq!(frame["payload"]["ok"], false);
            assert_eq!(frame["payload"]["code"], "automation_call_invalid");
        }
    }
}
