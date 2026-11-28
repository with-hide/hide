//! Native Claude/Codex question and plan records. Text is never inferred
//! from a label, title, or an ordinary assistant sentence.

use serde_json::Value;

use super::content::Asked;
use super::{
    NATIVE_ID_LIMIT_BYTES, QUESTION_CALL_LIMIT, ToolTurnMark, TurnMark, TurnMode, UserTurnContent,
};
use crate::SkipReason;

/// A single native record is already bounded by the session line limit.
/// Bound the retained collection as well, including unrelated tool results.
pub(crate) const TOOL_MARK_LIMIT: usize = 64;

type Mark = Result<Option<TurnMark>, SkipReason>;
pub(crate) type Parser = fn(&Value) -> Mark;

pub(crate) fn id(value: Option<&Value>) -> Result<Option<String>, SkipReason> {
    let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if value.len() > NATIVE_ID_LIMIT_BYTES {
        return Err(SkipReason::UserTurnCapacity);
    }
    Ok(Some(value.to_owned()))
}

pub(crate) fn question_content(input: &Value) -> Option<UserTurnContent> {
    let questions = record_questions(input)?;
    Some(UserTurnContent::questions(questions.into_iter().map(
        |question| Asked {
            text: question.text,
            choices: question.choices,
            several: question.several,
        },
    )))
}

/// One question an agent's own question call asks, as its record writes
/// it, before any cap.
pub(crate) struct RecordQuestion<'a> {
    /// The id the agent gave the question (Codex's `id`), by which its
    /// answer may come back.
    pub id: Option<&'a str>,
    pub text: &'a str,
    /// Each choice's label and the note under it.
    pub choices: Vec<(&'a str, Option<&'a str>)>,
    pub several: bool,
}

/// The questions a question call's input asks, in each shape an agent
/// writes: `questions`, each with its text as `question` and its choices as
/// `options` of `{label, description}` (Claude Code, Codex, OpenCode, omp)
/// or of plain strings (Antigravity's `ask_question`). `None` when the input
/// asks none, or any question lacks its text or holds a choice of neither
/// shape: such an input is not a question call.
pub(crate) fn record_questions(input: &Value) -> Option<Vec<RecordQuestion<'_>>> {
    let questions = input.get("questions")?.as_array()?;
    if questions.is_empty() {
        return None;
    }
    questions
        .iter()
        .map(|question| {
            let text = question.get("question")?.as_str()?;
            let choices = match question.get("options") {
                None => Vec::new(),
                Some(options) => options
                    .as_array()?
                    .iter()
                    .map(|choice| match choice {
                        Value::String(label) => Some((label.as_str(), None)),
                        choice => Some((
                            choice.get("label")?.as_str()?,
                            choice.get("description").and_then(Value::as_str),
                        )),
                    })
                    .collect::<Option<Vec<_>>>()?,
            };
            Some(RecordQuestion {
                id: question.get("id").and_then(Value::as_str),
                text,
                choices,
                // Claude Code's `multiSelect`, OpenCode's `multiple`,
                // Antigravity's `is_multi_select`.
                several: ["multiSelect", "multiple", "is_multi_select"]
                    .iter()
                    .any(|key| question.get(key).and_then(Value::as_bool) == Some(true)),
            })
        })
        .collect()
}

/// What the operator answered each question of a Codex
/// `request_user_input_async` call, from one reply message's text: the call,
/// the question's place in it, and the answer. `None` when the text is not
/// such a reply or its body cannot be read.
pub(crate) fn async_reply_answers(text: &str) -> Option<Vec<(String, usize, String)>> {
    let body = text.trim_start().strip_prefix(ASYNC_REPLY_OPEN)?;
    let body = body.rsplit_once(ASYNC_REPLY_CLOSE).map_or(body, |(body, _)| body);
    let Value::Array(replies) = serde_json::from_str::<Value>(body.trim()).ok()? else {
        return None;
    };
    Some(
        replies
            .iter()
            .filter_map(|reply| {
                let item = match reply.get("questionItemId")? {
                    Value::String(item) => serde_json::from_str::<Value>(item).ok()?,
                    item => item.clone(),
                };
                let item = item.as_array()?;
                if item.first()?.as_str()? != ASYNC_QUESTION_TOOL {
                    return None;
                }
                let call = item.get(1)?.as_str()?.to_owned();
                let question = usize::try_from(item.get(2)?.as_u64()?).ok()?;
                Some((call, question, answer_text(reply.get("answer")?)?))
            })
            .collect(),
    )
}

/// An answer as one line: a list of picked choices joined by commas.
pub(crate) fn answer_text(answer: &Value) -> Option<String> {
    match answer {
        Value::String(text) => Some(text.clone()),
        Value::Array(picked) => Some(
            picked
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    }
}

/// Codex 0.162.1 asks with this tool instead of `request_user_input`: it
/// returns `{"accepted":true}` at once and the turn goes on, so the
/// operator's answer is the later reply message below, not the tool result.
const ASYNC_QUESTION_TOOL: &str = "request_user_input_async";
/// What opens the user message Codex writes for the operator's answer; Hide
/// reads it as scaffolding, never as the person's turn.
pub(crate) const ASYNC_REPLY_OPEN: &str = "<send_user_message_question_reply>";
const ASYNC_REPLY_CLOSE: &str = "</send_user_message_question_reply>";

/// An async call's questions carry a `title` and optional plain-string
/// `options`, not the `question` and `{label}` options of the older tool.
fn async_question_content(questions: &[Value]) -> Option<UserTurnContent> {
    let mut asked = Vec::with_capacity(questions.len());
    for question in questions {
        let options = match question.get("options") {
            None => &[][..],
            Some(options) => options.as_array()?,
        };
        if options.iter().any(|option| option.as_str().is_none()) {
            return None;
        }
        asked.push(Asked {
            text: question.get("title")?.as_str()?,
            choices: options
                .iter()
                .filter_map(Value::as_str)
                .map(|label| (label, None)),
            several: false,
        });
    }
    Some(UserTurnContent::questions(asked))
}

fn async_question(payload: &Value) -> Mark {
    let call = id(payload.get("call_id"))?.ok_or(SkipReason::UserTurnInvalid)?;
    let arguments = payload
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
    let Some(questions) = arguments
        .as_ref()
        .and_then(|arguments| arguments.get("questions"))
        .and_then(Value::as_array)
        .filter(|questions| !questions.is_empty())
    else {
        // A shape this reader does not know: what the turn waits for is not.
        return Ok(Some(TurnMark::Unreadable));
    };
    if questions.len() > QUESTION_CALL_LIMIT {
        return Err(SkipReason::UserTurnCapacity);
    }
    Ok(Some(TurnMark::Tools(vec![ToolTurnMark::AskedAsync {
        call,
        questions: questions.len() as u32,
        content: async_question_content(questions),
    }])))
}

/// The operator's replies in one `<send_user_message_question_reply>`
/// message, each matched to its question by `questionItemId` (a JSON array
/// `[tool, call, question number]`, written as a string), never by its text.
/// `None` when the message is not a reply.
fn async_replies(payload: &Value) -> Option<Mark> {
    let text = payload
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))?
        .trim_start()
        .strip_prefix(ASYNC_REPLY_OPEN)?;
    Some(replied_marks(
        text.rsplit_once(ASYNC_REPLY_CLOSE)
            .map_or(text, |(body, _)| body),
    ))
}

fn replied_marks(body: &str) -> Mark {
    let Ok(Value::Array(replies)) = serde_json::from_str::<Value>(body.trim()) else {
        return Ok(Some(TurnMark::Unreadable));
    };
    let mut marks = Vec::new();
    for reply in &replies {
        let item = match reply.get("questionItemId") {
            Some(Value::String(item)) => serde_json::from_str::<Value>(item).ok(),
            Some(item) => Some(item.clone()),
            None => None,
        };
        let Some(Value::Array(item)) = item else {
            return Ok(Some(TurnMark::Unreadable));
        };
        if item.first().and_then(Value::as_str) != Some(ASYNC_QUESTION_TOOL) {
            continue;
        }
        let question = item
            .get(2)
            .and_then(Value::as_u64)
            .and_then(|question| u32::try_from(question).ok());
        let (Some(call), Some(question)) = (id(item.get(1))?, question) else {
            return Ok(Some(TurnMark::Unreadable));
        };
        if marks.len() == TOOL_MARK_LIMIT {
            return Err(SkipReason::UserTurnCapacity);
        }
        marks.push(ToolTurnMark::Replied { call, question });
    }
    Ok((!marks.is_empty()).then_some(TurnMark::Tools(marks)))
}

/// Claude Code writes a `system` record of subtype `turn_duration`, with
/// `isSidechain` false, as a turn ends, after its Stop hooks ran. In the
/// 2.1.319 sessions of one Mac (2026-11-08: 130 files, 1376 turns) each
/// turn had at most one, after the last Stop hook summary when the agent
/// went on after the first, and none was written after a later prompt;
/// that is what was seen there, not a promise Claude Code makes. Only that
/// form is read. It names no turn and no reason: it also follows some
/// interruptions of a tool call, which stay interruptions (`TurnTracker`),
/// and a turn a compaction, a queued prompt or a closed session cut short
/// has none, so a turn without it is not guessed complete.
const CLAUDE_TURN_END: &str = "turn_duration";

pub(crate) fn claude(item: &Value) -> Mark {
    let role = item.get("type").and_then(Value::as_str);
    if role == Some("system") {
        let end = item.get("subtype").and_then(Value::as_str) == Some(CLAUDE_TURN_END)
            && item.get("isSidechain").and_then(Value::as_bool) == Some(false);
        return Ok(end.then_some(TurnMark::Completed { turn: None }));
    }
    let Some(blocks) = item.pointer("/message/content").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut marks = Vec::new();
    for block in blocks {
        let kind = block.get("type").and_then(Value::as_str);
        let mark = match (role, kind) {
            (Some("assistant"), Some("tool_use"))
                if block.get("name").and_then(Value::as_str)
                    == Some(crate::calls::CLAUDE_QUESTION_TOOL) =>
            {
                let call = id(block.get("id"))?.ok_or(SkipReason::UserTurnInvalid)?;
                Some(ToolTurnMark::Asked {
                    call,
                    content: block.get("input").and_then(question_content),
                })
            }
            // Every other call, so a wait on it (a permission dialog, which
            // names no call id) can be told answered by the session read.
            (Some("assistant"), Some("tool_use")) => {
                let key = block.get("name").and_then(Value::as_str).and_then(|name| {
                    crate::calls::key(name, block.get("input").unwrap_or(&Value::Null))
                });
                match (id(block.get("id"))?, key) {
                    (Some(call), Some(key)) => Some(ToolTurnMark::Called { call, key }),
                    _ => None,
                }
            }
            (Some("user"), Some("tool_result")) => {
                id(block.get("tool_use_id"))?.map(|call| ToolTurnMark::Answered { call })
            }
            _ => None,
        };
        if let Some(mark) = mark {
            if marks.len() == TOOL_MARK_LIMIT {
                return Err(SkipReason::UserTurnCapacity);
            }
            marks.push(mark);
        }
    }
    if marks.is_empty() && role == Some("assistant") {
        return Ok(Some(TurnMark::Reply));
    }
    Ok((!marks.is_empty()).then_some(TurnMark::Tools(marks)))
}

/// omp records native ask calls inside assistant message blocks and their
/// toolResult correlation separately. Audit/custom records are not turns.
pub(crate) fn omp(item: &Value) -> Mark {
    if item["type"] != "message" {
        return Ok(None);
    }
    let message = &item["message"];
    if message["role"] == "toolResult" {
        return Ok(id(message.get("toolCallId"))?
            .map(|call| TurnMark::Tools(vec![ToolTurnMark::Answered { call }])));
    }
    if message["role"] != "assistant" {
        return Ok(None);
    }
    if message["stopReason"] == "aborted" {
        return Ok(Some(TurnMark::Interrupted));
    }
    let mut marks = Vec::new();
    for block in message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if block["type"] != "toolCall" || block["name"] != "ask" {
            continue;
        }
        if marks.len() == TOOL_MARK_LIMIT {
            return Err(SkipReason::UserTurnCapacity);
        }
        let call = id(block.get("id"))?.ok_or(SkipReason::UserTurnInvalid)?;
        marks.push(ToolTurnMark::Asked {
            call,
            content: block.get("arguments").and_then(question_content),
        });
    }
    Ok((!marks.is_empty()).then_some(TurnMark::Tools(marks)))
}

/// Codex 0.160.1 emits task lifecycle records and native function-call
/// records. A completed plan-mode turn holds approval; an unanswered native
/// request_user_input holds a question while the turn is still running, and
/// so does a `request_user_input_async` (0.162.1) until its reply arrives.
pub(crate) fn codex(item: &Value) -> Mark {
    let Some(payload) = item.get("payload") else {
        return Ok(None);
    };
    let kind = payload.get("type").and_then(Value::as_str);
    let turn = || id(payload.get("turn_id"));
    let mark = match item.get("type").and_then(Value::as_str) {
        Some("event_msg") => match kind {
            Some("task_started") => Some(TurnMark::Started {
                turn: turn()?,
                mode: match payload
                    .get("collaboration_mode_kind")
                    .and_then(Value::as_str)
                {
                    Some("plan") => TurnMode::Plan,
                    Some("default") => TurnMode::Other,
                    _ => TurnMode::Unknown,
                },
            }),
            Some("item_completed")
                if payload.pointer("/item/type").and_then(Value::as_str) == Some("Plan") =>
            {
                Some(
                    match payload.pointer("/item/text").and_then(Value::as_str) {
                        Some(text) => TurnMark::PlanContent {
                            turn: turn()?,
                            content: UserTurnContent::new(text, []),
                        },
                        None => TurnMark::Plan { turn: turn()? },
                    },
                )
            }
            Some("task_complete") => Some(TurnMark::Completed { turn: turn()? }),
            Some("turn_aborted") => Some(TurnMark::Aborted { turn: turn()? }),
            _ => None,
        },
        Some("response_item") => match kind {
            Some("function_call")
                if payload.get("name").and_then(Value::as_str) == Some("request_user_input") =>
            {
                let call = id(payload.get("call_id"))?.ok_or(SkipReason::UserTurnInvalid)?;
                let arguments = payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
                Some(TurnMark::Tools(vec![ToolTurnMark::Asked {
                    call,
                    content: arguments.as_ref().and_then(question_content),
                }]))
            }
            Some("function_call")
                if payload.get("name").and_then(Value::as_str) == Some(ASYNC_QUESTION_TOOL) =>
            {
                async_question(payload)?
            }
            Some("function_call_output") => id(payload.get("call_id"))?
                .map(|call| TurnMark::Tools(vec![ToolTurnMark::Answered { call }])),
            Some("message") if payload.get("role").and_then(Value::as_str) == Some("user") => {
                async_replies(payload).transpose()?.flatten()
            }
            Some("message") if payload.get("role").and_then(Value::as_str) == Some("assistant") => {
                // This explicit native wrapper is also written when the
                // structured Plan item is absent from the read's prefix.
                payload
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .find_map(|text| {
                        let body = text
                            .trim()
                            .strip_prefix("<proposed_plan>")?
                            .strip_suffix("</proposed_plan>")?
                            .trim();
                        Some(TurnMark::PlanContent {
                            turn: None,
                            content: UserTurnContent::new(body, []),
                        })
                    })
            }
            _ => None,
        },
        _ => None,
    };
    Ok(mark)
}

/// OpenCode 1.18.30 keeps a `question` tool call as one part whose state
/// moves from `pending`/`running` while it asks to `completed` (answered) or
/// `error` (dismissed or aborted). Its input has Claude's question shape.
pub(crate) fn opencode_part(part: &Value) -> Result<Option<ToolTurnMark>, SkipReason> {
    if part.get("type").and_then(Value::as_str) != Some("tool")
        || part.get("tool").and_then(Value::as_str) != Some("question")
    {
        return Ok(None);
    }
    let call = id(part.get("callID"))?.ok_or(SkipReason::UserTurnInvalid)?;
    match part.pointer("/state/status").and_then(Value::as_str) {
        // OpenCode is still streaming the call's input: nothing has been put
        // to the operator yet, and the running state that follows asks it.
        Some("pending") => Ok(None),
        Some("running") => Ok(Some(ToolTurnMark::Asked {
            call,
            content: part.pointer("/state/input").and_then(question_content),
        })),
        Some("completed" | "error") => Ok(Some(ToolTurnMark::Answered { call })),
        _ => Err(SkipReason::UserTurnInvalid),
    }
}

/// OpenCode 2.x keeps a `question` tool call as one content item of its
/// assistant message, named by `id` and `name`: `streaming` while its input is
/// written, `running` while it asks, then `completed` or `error`.
pub(crate) fn opencode_2_part(part: &Value) -> Result<Option<ToolTurnMark>, SkipReason> {
    if part.get("type").and_then(Value::as_str) != Some("tool")
        || part.get("name").and_then(Value::as_str) != Some("question")
    {
        return Ok(None);
    }
    let call = id(part.get("id"))?.ok_or(SkipReason::UserTurnInvalid)?;
    match part.pointer("/state/status").and_then(Value::as_str) {
        Some("streaming") => Ok(None),
        Some("running") => Ok(Some(ToolTurnMark::Asked {
            call,
            content: part.pointer("/state/input").and_then(question_content),
        })),
        Some("completed" | "error") => Ok(Some(ToolTurnMark::Answered { call })),
        _ => Err(SkipReason::UserTurnInvalid),
    }
}

/// The question marks of one OpenCode 1.x message's parts, bounded like a
/// native record's tool marks.
pub(crate) fn opencode_marks(parts: &[Value]) -> Result<Vec<ToolTurnMark>, SkipReason> {
    marks_of(parts, opencode_part)
}

/// The question marks of one OpenCode 2.x message's content items.
pub(crate) fn opencode_2_marks(parts: &[Value]) -> Result<Vec<ToolTurnMark>, SkipReason> {
    marks_of(parts, opencode_2_part)
}

fn marks_of(
    parts: &[Value],
    mark: fn(&Value) -> Result<Option<ToolTurnMark>, SkipReason>,
) -> Result<Vec<ToolTurnMark>, SkipReason> {
    let mut marks = Vec::new();
    for part in parts {
        if let Some(mark) = mark(part)? {
            if marks.len() == TOOL_MARK_LIMIT {
                return Err(SkipReason::UserTurnCapacity);
            }
            marks.push(mark);
        }
    }
    Ok(marks)
}
