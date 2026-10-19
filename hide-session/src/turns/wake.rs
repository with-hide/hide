//! Background work a Claude Code session started and that will wake the agent
//! again when it ends (PRD agent-blocked-state B18, D-26).
//!
//! Only a record the agent's own process wrote proves a device: the start
//! text its tool returned, the `task-notification` that ends it, a `TaskStop`
//! call whose result says it stopped the task, and the `SessionStart` hook
//! record that marks a new process (a device never outlives the process that
//! started it). Anything the records do not show is not a device.
//!
//! A record too large to keep is read by structure, never by its words: which
//! record kind it is, which tool a call names and which call a result answers
//! (`conversation_cursor::LargeRecord`). The calls to tools that can start
//! work are marked here, so the result of one that cannot be read is known to
//! be one that may have started a device.

use hide_agent_adapter::work::WorkKind;
use serde_json::Value;

use super::{NATIVE_ID_LIMIT_BYTES, StopOutcome, TurnMark, WakeMark, native::TOOL_MARK_LIMIT};
use crate::SkipReason;
use crate::claude_attachment::Attachment;

const BACKGROUND_STARTED: &str = "Command running in background with ID: ";
const TIMED_OUT: &str = "Command did not complete within its ";
const MOVED_TO_BACKGROUND: &str = "was moved to the background (ID: ";
const MONITOR_STARTED: &str = "Monitor started (task ";
const AGENT_LAUNCHED: &str = "Async agent launched successfully";
const NOTIFICATION: &str = "<task-notification>";
const TASK_STOP: &str = "TaskStop";

/// Whether a call to the tool `name` may return the start of background work:
/// a shell command that runs on or moves to the background, a monitor, a
/// launched agent (`Task` is the name an older Claude Code gave `Agent`). The
/// result of any other tool is not read for a start by a record too large to
/// keep; a record that is kept is read for a start whatever tool wrote it.
pub(crate) fn starts_work(name: &str) -> bool {
    matches!(name, "Bash" | "Monitor" | "Agent" | "Task")
}

/// Whether a call to the tool `name` asks to stop a task.
pub(crate) fn stops_work(name: &str) -> bool {
    name == TASK_STOP
}

pub(crate) type Parser = fn(&Value) -> Result<Option<TurnMark>, SkipReason>;

/// The kinds of session record [`claude`] takes marks from; a record of any
/// other kind carries none. A record too large to keep is told apart by this
/// kind alone, so a kind added here is one the compiler has the large-record
/// reader decide as well (`conversation_cursor::LargeRecord::wake_mark`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Record {
    /// A hook that began a process, or a queued command that ends a task.
    Attachment,
    /// A queued notification that ends a task.
    QueueOperation,
    /// A tool's result, whose text may start a task.
    User,
    /// A call to a tool that may start a task, or a `TaskStop`.
    Assistant,
}

impl Record {
    pub(crate) fn of(kind: &str) -> Option<Self> {
        match kind {
            "attachment" => Some(Self::Attachment),
            "queue-operation" => Some(Self::QueueOperation),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

/// The wake marks one Claude Code record carries; empty for most records.
pub(crate) fn claude(item: &Value) -> Result<Option<TurnMark>, SkipReason> {
    let mut marks = Vec::new();
    match item
        .get("type")
        .and_then(Value::as_str)
        .and_then(Record::of)
    {
        Some(Record::Attachment) => attachment(item, &mut marks),
        Some(Record::QueueOperation)
            if item.get("operation").and_then(Value::as_str) == Some("enqueue") =>
        {
            if let Some(text) = item.get("content").and_then(Value::as_str) {
                notifications(text, &mut marks);
            }
        }
        Some(Record::User) => user(item, &mut marks),
        Some(Record::Assistant) => assistant(item, &mut marks),
        Some(Record::QueueOperation) | None => {}
    }
    Ok((!marks.is_empty()).then_some(TurnMark::Wake(marks)))
}

/// Keeps a record's marks bounded. A record with more than the limit, or a
/// task or call id too long to keep, is one this reader cannot follow, so what
/// it says about devices is lost (`WakeMark::Lost`); it never fails the read
/// the turn and label depend on.
fn push(marks: &mut Vec<WakeMark>, mark: Option<WakeMark>) {
    match mark {
        Some(mark) if marks.len() < TOOL_MARK_LIMIT => marks.push(mark),
        _ if marks.last() == Some(&WakeMark::Lost) => {}
        _ => marks.push(WakeMark::Lost),
    }
}

fn attachment(item: &Value, marks: &mut Vec<WakeMark>) {
    match Attachment::of(item) {
        Some(Attachment::HookSuccess {
            hook_name: Some("SessionStart:startup" | "SessionStart:resume"),
        }) => push(marks, Some(WakeMark::Boot)),
        Some(Attachment::QueuedCommand { prompt: Some(text) }) => notifications(text, marks),
        _ => {}
    }
}

/// Every `<task-notification>` in `text` that names a finished task. A
/// `running` status, or a notification with none, is an event of a task that
/// goes on.
fn notifications(text: &str, marks: &mut Vec<WakeMark>) {
    for part in text.split(NOTIFICATION).skip(1) {
        let part = part.split("</task-notification>").next().unwrap_or(part);
        let (Some(id), Some(status)) = (tag(part, "task-id"), tag(part, "status")) else {
            continue;
        };
        if matches!(status.trim(), "completed" | "failed" | "killed" | "stopped") {
            push(marks, id_of(id.trim()).map(|id| WakeMark::Ended { id }));
        }
    }
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let rest = &text[text.find(&open)? + open.len()..];
    Some(&rest[..rest.find(&format!("</{name}>"))?])
}

fn id_of(id: &str) -> Option<String> {
    (!id.is_empty() && id.len() <= NATIVE_ID_LIMIT_BYTES).then(|| id.to_owned())
}

/// The task id that begins at the start of `text`.
fn token(text: &str) -> Option<&str> {
    let end = text
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(text.len());
    (end > 0).then(|| &text[..end])
}

fn after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    Some(&text[text.find(marker)? + marker.len()..])
}

/// What follows `marker` when the tool's own message opens with it. A start
/// marker quoted in a file or a search result the agent read sits mid-text and
/// proves nothing.
fn opening<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    text.trim_start().strip_prefix(marker)
}

fn user(item: &Value, marks: &mut Vec<WakeMark>) {
    let Some(blocks) = item.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    let at = || crate::timestamp_ms(item.get("timestamp")).ok();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
        // Keep the call until starts have borrowed its native description.
        let call = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .and_then(id_of);
        if failed {
            if let Some(call) = call {
                push(
                    marks,
                    Some(WakeMark::Answered {
                        call,
                        outcome: StopOutcome::Failed,
                    }),
                );
            }
            continue;
        }
        let texts: Vec<&str> = match block.get("content") {
            Some(Value::String(text)) => vec![text],
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect(),
            _ => Vec::new(),
        };
        for text in texts {
            let timed_out =
                opening(text, TIMED_OUT).and_then(|rest| after(rest, MOVED_TO_BACKGROUND));
            if let Some(id) = opening(text, BACKGROUND_STARTED)
                .or(timed_out)
                .and_then(token)
            {
                push(
                    marks,
                    id_of(id).map(|id| WakeMark::Started {
                        id,
                        kind: Some(WorkKind::Background),
                        call: call.clone(),
                        expires_at_unix_ms: None,
                    }),
                );
            } else if let Some(rest) = opening(text, MONITOR_STARTED)
                && let Some(id) = token(rest)
            {
                // A monitor that announces an expiry but has no time to count
                // it from cannot be proven alive.
                let expires = match after(rest, "expires in ").and_then(duration_ms) {
                    Some(ms) => at().map(|at| Some(at.saturating_add(ms))),
                    None => Some(None),
                };
                let Some(expires) = expires else {
                    push(marks, None);
                    continue;
                };
                push(
                    marks,
                    id_of(id).map(|id| WakeMark::Started {
                        id,
                        kind: Some(WorkKind::Background),
                        call: call.clone(),
                        expires_at_unix_ms: expires,
                    }),
                );
            } else if text.trim_start().starts_with(AGENT_LAUNCHED)
                && let Some(id) = after(text, "agentId: ").and_then(token)
            {
                push(
                    marks,
                    id_of(id).map(|id| WakeMark::Started {
                        id,
                        kind: Some(WorkKind::Subagent),
                        call: call.clone(),
                        expires_at_unix_ms: None,
                    }),
                );
            }
        }
        if let Some(call) = call {
            push(
                marks,
                Some(WakeMark::Answered {
                    call,
                    outcome: StopOutcome::Succeeded,
                }),
            );
        }
    }
}

/// `30m`, `90s`, `2h` at the start of `text`.
fn duration_ms(text: &str) -> Option<u64> {
    let digits = text.find(|c: char| !c.is_ascii_digit())?;
    let value: u64 = text[..digits].parse().ok()?;
    let unit = match text[digits..].chars().next()? {
        's' => 1_000,
        'm' => 60_000,
        'h' => 3_600_000,
        _ => return None,
    };
    value.checked_mul(unit)
}

/// A call to a tool that can start work, and a `TaskStop` call. A `TaskStop`
/// stops nothing by itself: the result of the call says whether the task
/// stopped (a refusal, or no result yet, leaves it running). A call whose id
/// cannot be kept cannot be paired with its result, so what it says is lost.
fn assistant(item: &Value, marks: &mut Vec<WakeMark>) {
    let Some(blocks) = item.pointer("/message/content").and_then(Value::as_array) else {
        return;
    };
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let Some(name) = block.get("name").and_then(Value::as_str) else {
            continue;
        };
        let call = || block.get("id").and_then(Value::as_str).and_then(id_of);
        if starts_work(name) {
            let name = block
                .pointer("/input/description")
                .and_then(Value::as_str)
                .filter(|text| !text.chars().any(char::is_control))
                .and_then(id_of);
            push(marks, call().map(|call| WakeMark::Call { call, name }));
        } else if stops_work(name)
            && let Some(task) = block.pointer("/input/task_id").and_then(Value::as_str)
        {
            push(
                marks,
                call()
                    .zip(id_of(task))
                    .map(|(call, id)| WakeMark::Stop { call, id }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every mark the record carries.
    fn all_marks(item: &Value) -> Vec<WakeMark> {
        match claude(item).unwrap() {
            Some(TurnMark::Wake(marks)) => marks,
            None => Vec::new(),
            other => panic!("{other:?}"),
        }
    }

    /// The marks about devices, without the answer every tool result gives.
    fn marks(item: Value) -> Vec<WakeMark> {
        all_marks(&item)
            .into_iter()
            .filter(|mark| !matches!(mark, WakeMark::Answered { .. }))
            .collect()
    }

    fn result(text: &str) -> Value {
        json!({"type": "user", "timestamp": "2026-10-07T13:15:43.912Z",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t", "content": text}]}})
    }

    fn started(id: &str) -> WakeMark {
        WakeMark::Started {
            id: id.into(),
            kind: Some(WorkKind::Background),
            call: Some("t".into()),
            expires_at_unix_ms: None,
        }
    }

    #[test]
    fn each_way_claude_starts_background_work_names_its_task() {
        assert_eq!(
            marks(result(
                "Command running in background with ID: bz0nfp0pu. Output is being written to: /x"
            )),
            [started("bz0nfp0pu")]
        );
        assert_eq!(
            marks(result(
                "Command did not complete within its 600s timeout and was moved to the background (ID: bjbx2pcf9). Output"
            )),
            [started("bjbx2pcf9")]
        );
        let launched = json!({"type": "user", "message": {"content": [{"type": "tool_result",
            "content": [{"type": "text", "text": "Async agent launched successfully. (internal)\nagentId: a7359e5355eca4579 (internal ID)\nThe agent is working"}]}]}});
        assert_eq!(
            marks(launched),
            [WakeMark::Started {
                id: "a7359e5355eca4579".into(),
                kind: Some(WorkKind::Subagent),
                call: None,
                expires_at_unix_ms: None,
            }]
        );
    }

    #[test]
    fn internal_work_keeps_native_descriptions_and_execution_ends_only() {
        use crate::turns::TurnTracker;
        use hide_agent_adapter::work::WorkState;
        let mut tracker = TurnTracker::default();
        tracker.fold(0, &TurnMark::Wake(vec![WakeMark::Boot]));
        let call = json!({"type":"assistant","message":{"content":[{
            "type":"tool_use","name":"Agent","id":"t",
            "input":{"description":"의존성 검토","prompt":"Private task body"}
        }]}});
        let start = result("Async agent launched successfully.\nagentId: child-1 (internal ID)");
        for (offset, item) in [(1, &call), (2, &start), (2, &start)] {
            tracker.fold(offset, &TurnMark::Wake(all_marks(item)));
        }
        let live = tracker.work_records(0);
        assert_eq!(live.len(), 1, "a repeated read does not duplicate work");
        assert_eq!(live[0].kind, Some(WorkKind::Subagent));
        assert_eq!(live[0].name.as_deref(), Some("의존성 검토"));
        assert_eq!(live[0].state, WorkState::Running);
        let ended = json!({"type":"queue-operation","operation":"enqueue",
            "content":"<task-notification><task-id>child-1</task-id><status>failed</status></task-notification>"});
        tracker.fold(3, &TurnMark::Wake(all_marks(&ended)));
        let restored: TurnTracker =
            serde_json::from_str(&serde_json::to_string(&tracker).unwrap()).unwrap();
        let records = restored.work_records(0);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].state,
            WorkState::Ended,
            "failed execution is no assignment-success verdict"
        );
        assert_eq!(records[0].name.as_deref(), Some("의존성 검토"));

        let mut unnamed = call;
        unnamed["message"]["content"][0]["input"]["description"] =
            json!("x".repeat(NATIVE_ID_LIMIT_BYTES + 1));
        assert_eq!(
            all_marks(&unnamed),
            [WakeMark::Call {
                call: "t".into(),
                name: None
            }]
        );
        unnamed["message"]["content"][0]["input"]
            .as_object_mut()
            .unwrap()
            .remove("description");
        assert_eq!(
            all_marks(&unnamed),
            [WakeMark::Call {
                call: "t".into(),
                name: None
            }],
            "a prompt is never a display name"
        );
    }

    #[test]
    fn a_monitor_expires_after_the_time_it_announces() {
        let at = crate::parse_rfc3339_unix_ms("2026-10-07T13:15:43.912Z").unwrap();
        assert_eq!(
            marks(result(
                "Monitor started (task bcwkc73y4, expires in 30m unless the source ends first; you get one notice"
            )),
            [WakeMark::Started {
                id: "bcwkc73y4".into(),
                kind: Some(WorkKind::Background),
                call: Some("t".into()),
                expires_at_unix_ms: Some(at + 30 * 60_000)
            }]
        );
    }

    #[test]
    fn a_failed_start_and_ordinary_results_name_no_task() {
        let mut failed = result("Command running in background with ID: x");
        failed["message"]["content"][0]["is_error"] = json!(true);
        assert!(marks(failed).is_empty());
        assert!(marks(result("Compiling hide v0.1.0")).is_empty());
    }

    #[test]
    fn a_task_ends_by_notification_in_either_record() {
        let notification = |status: &str| {
            format!(
                "<task-notification>\n<task-id>bz0nfp0pu</task-id>\n<status>{status}</status>\n</task-notification>"
            )
        };
        let ended = WakeMark::Ended {
            id: "bz0nfp0pu".into(),
        };
        for status in ["completed", "failed", "killed", "stopped"] {
            assert_eq!(
                marks(
                    json!({"type": "queue-operation", "operation": "enqueue", "content": notification(status)})
                ),
                std::slice::from_ref(&ended)
            );
            assert_eq!(
                marks(
                    json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": notification(status)}})
                ),
                std::slice::from_ref(&ended)
            );
        }
        assert!(marks(json!({"type": "queue-operation", "operation": "enqueue", "content": notification("running")})).is_empty());
        assert!(marks(json!({"type": "queue-operation", "operation": "dequeue", "content": notification("completed")})).is_empty());
    }

    fn stop(call: Value, task: Value) -> Value {
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": call,
            "name": "TaskStop", "input": {"task_id": task}}]}})
    }

    #[test]
    fn a_task_stop_names_its_call_and_ends_nothing_until_answered() {
        assert_eq!(
            marks(stop(json!("toolu_1"), json!("bz0nfp0pu"))),
            [WakeMark::Stop {
                call: "toolu_1".into(),
                id: "bz0nfp0pu".into()
            }]
        );
        // A call that names no task stops nothing, and one whose ids cannot be
        // kept cannot be paired with its result, so what it says is lost.
        assert!(
            marks(
                json!({"type": "assistant", "message": {"content": [{"type": "tool_use",
            "id": "toolu_1", "name": "TaskStop", "input": {}}]}})
            )
            .is_empty()
        );
        assert_eq!(
            marks(stop(json!(null), json!("bz0nfp0pu"))),
            [WakeMark::Lost]
        );
        assert_eq!(
            marks(stop(
                json!("toolu_1"),
                json!("x".repeat(NATIVE_ID_LIMIT_BYTES + 1))
            )),
            [WakeMark::Lost]
        );
    }

    fn call(name: &str, id: Value) -> Value {
        json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": "running it"},
            {"type": "tool_use", "id": id, "name": name, "input": {}}]}})
    }

    #[test]
    fn a_call_to_a_tool_that_can_start_work_names_its_call() {
        for name in ["Bash", "Monitor", "Agent", "Task"] {
            assert_eq!(
                marks(call(name, json!("toolu_1"))),
                [WakeMark::Call {
                    call: "toolu_1".into(),
                    name: None,
                }],
                "{name}"
            );
        }
        // Any other tool's result is not read for a start, a stop that names
        // no task stops nothing, and a call whose id cannot be kept cannot be
        // paired with its result.
        assert!(marks(call("Read", json!("toolu_1"))).is_empty());
        assert!(marks(call("mcp__x__screenshot", json!("toolu_1"))).is_empty());
        assert_eq!(marks(call("Bash", json!(null))), [WakeMark::Lost]);
        assert_eq!(marks(call("Bash", json!(""))), [WakeMark::Lost]);
        assert_eq!(
            marks(call("Bash", json!("x".repeat(NATIVE_ID_LIMIT_BYTES + 1)))),
            [WakeMark::Lost]
        );
    }

    #[test]
    fn a_tool_result_answers_its_call_as_stopped_or_refused() {
        let answer = |is_error: bool| {
            let mut block =
                json!({"type": "tool_result", "tool_use_id": "toolu_1", "content": "x"});
            if is_error {
                block["is_error"] = json!(true);
            }
            json!({"type": "user", "message": {"content": [block]}})
        };
        assert_eq!(
            all_marks(&answer(false)),
            [WakeMark::Answered {
                call: "toolu_1".into(),
                outcome: StopOutcome::Succeeded
            }]
        );
        assert_eq!(
            all_marks(&answer(true)),
            [WakeMark::Answered {
                call: "toolu_1".into(),
                outcome: StopOutcome::Failed
            }]
        );
    }

    #[test]
    fn only_a_session_start_hook_of_a_new_process_is_a_boundary() {
        let hook = |name: &str| json!({"type": "attachment", "attachment": {"type": "hook_success", "hookName": name}});
        assert_eq!(marks(hook("SessionStart:startup")), [WakeMark::Boot]);
        assert_eq!(marks(hook("SessionStart:resume")), [WakeMark::Boot]);
        assert!(marks(hook("SessionStart:clear")).is_empty());
        assert!(marks(hook("SessionStart:compact")).is_empty());
        assert!(marks(hook("PreToolUse:Bash")).is_empty());
    }
}
