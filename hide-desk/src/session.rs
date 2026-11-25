//! The ACP session on a Desk process: open or resume it in its ask mode,
//! prompt, cancel, choose a model or effort, and read what the agent sends
//! back (ACP v1).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::adapter::{AdapterId, AskMode, FailedTurnMark};
use crate::rpc::{CallError, Connection, Incoming, Sink};

/// How long a control request may take: a first `session/new` starts the
/// agent's CLI, and a `session/load` replays the conversation.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(90);
/// ACP's error code for a request that needs the agent signed in.
pub const AUTH_REQUIRED: i64 = -32000;

/// One option the session lets the operator choose (`configOptions`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Choice {
    pub config_id: String,
    pub current: Option<String>,
    pub values: Vec<(String, String)>,
}

/// The model and effort choices the session offers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Choices {
    pub model: Option<Choice>,
    pub effort: Option<Choice>,
}

/// An open session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Opened {
    pub session_id: String,
    /// The conversation asked for was loaded; `false` is a new session.
    pub resumed: bool,
    /// A resume was asked for and the agent could not load it.
    pub resume_failed: bool,
    pub choices: Choices,
    /// The text the adapter said it writes on its own after the session
    /// opens (Pi's startup lines, `_meta.piAcp.startupInfo`): not a reply.
    pub prelude: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenError {
    /// The agent needs the operator signed in (B6).
    Login,
    /// The ask mode could not be confirmed: the session is not used (D-53).
    AskModeUnconfirmed(String),
    Failed(String),
}

impl OpenError {
    fn from_call(step: &str, error: CallError) -> Self {
        match error {
            CallError::Rejected(rejected) if rejected.code == AUTH_REQUIRED => Self::Login,
            other => Self::Failed(format!("{step}: {other}")),
        }
    }
}

/// Opens a session in `cwd`: loads `resume` when the agent can, else starts
/// a new one, then puts it in the adapter's ask mode. Pi's mode is proven by
/// its extension's marker, which `adapter::pi_extension_loaded` checks once the session exists; an agent that starts in it needs nothing more.
pub fn open(
    connection: &Connection,
    adapter: AdapterId,
    cwd: &Path,
    resume: Option<&str>,
    pi_ready: &dyn Fn() -> Result<(), String>,
) -> Result<Opened, OpenError> {
    let initialized = connection
        .call(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "hide-desk", "version": env!("CARGO_PKG_VERSION")},
            }),
            CONTROL_TIMEOUT,
        )
        .map_err(|error| OpenError::from_call("initialize", error))?;
    let can_load = initialized
        .pointer("/agentCapabilities/loadSession")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut params = json!({"cwd": cwd, "mcpServers": []});
    if let Some(meta) = adapter.session_meta() {
        params["_meta"] = meta;
    }
    let mut resume_failed = resume.is_some() && !can_load;
    let mut opened = None;
    if let Some(session_id) = resume.filter(|_| can_load) {
        let mut load = params.clone();
        load["sessionId"] = json!(session_id);
        match connection.call("session/load", load, CONTROL_TIMEOUT) {
            Ok(answer) => opened = Some((session_id.to_owned(), answer, true)),
            Err(CallError::Rejected(rejected)) if rejected.code == AUTH_REQUIRED => {
                return Err(OpenError::Login);
            }
            Err(CallError::Rejected(_)) => resume_failed = true,
            Err(error) => return Err(OpenError::from_call("session/load", error)),
        }
    }
    let (session_id, answer, resumed) = match opened {
        Some(opened) => opened,
        None => {
            let answer = connection
                .call("session/new", params, CONTROL_TIMEOUT)
                .map_err(|error| OpenError::from_call("session/new", error))?;
            let session_id = answer
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| OpenError::Failed("session/new: no sessionId".into()))?
                .to_owned();
            (session_id, answer, false)
        }
    };
    match adapter.ask_mode() {
        Some(AskMode::SetMode(mode)) => {
            connection
                .call(
                    "session/set_mode",
                    json!({"sessionId": session_id, "modeId": mode}),
                    CONTROL_TIMEOUT,
                )
                .map_err(|error| {
                    OpenError::AskModeUnconfirmed(format!("set_mode {mode}: {error}"))
                })?;
        }
        Some(AskMode::PiExtension) => pi_ready().map_err(OpenError::AskModeUnconfirmed)?,
        // Held by how the process started (`adapter::launch`).
        Some(AskMode::AtStart) => {}
        None => {
            return Err(OpenError::AskModeUnconfirmed(format!(
                "{} has no ask mode Desk can hold",
                adapter.label()
            )));
        }
    }
    Ok(Opened {
        session_id,
        resumed,
        resume_failed,
        choices: choices(&answer),
        prelude: answer
            .pointer("/_meta/piAcp/startupInfo")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
    })
}

/// The model and effort choices in a session answer or a
/// `config_option_update`.
pub fn choices(answer: &Value) -> Choices {
    let options = answer
        .get("configOptions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let pick = |category: &str| {
        options
            .iter()
            .find(|option| option.get("category").and_then(Value::as_str) == Some(category))
            .and_then(|option| {
                Some(Choice {
                    config_id: option.get("id")?.as_str()?.to_owned(),
                    current: option
                        .get("currentValue")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    values: option
                        .get("options")?
                        .as_array()?
                        .iter()
                        .filter_map(|value| {
                            let id = value.get("value")?.as_str()?.to_owned();
                            let name = value
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_owned();
                            Some((id, name))
                        })
                        .take(MAX_CHOICES)
                        .collect(),
                })
            })
    };
    Choices {
        model: pick("model"),
        effort: pick("thought_level"),
    }
}

/// What the agent sends, numbered in the order it arrives. A turn owns
/// only what arrived after its prompt went out: a `session/load` replays
/// the whole conversation as updates that all land before the load
/// answers, so an owner that reads its messages after it prompted would
/// otherwise join every earlier answer into the new reply. The rule is
/// ACP's, so it holds for every agent that loads a session.
#[derive(Clone, Debug, Default)]
pub struct Arrivals(Arc<AtomicU64>);

impl Arrivals {
    /// A sink that numbers each message before `owner` takes it.
    pub fn sink(&self, owner: impl Fn(u64, Incoming) + Send + Sync + 'static) -> Sink {
        let count = Arc::clone(&self.0);
        Arc::new(move |incoming| owner(count.fetch_add(1, Ordering::SeqCst), incoming))
    }

    /// The number the next message will carry: read just before a prompt
    /// goes out, it is the first its turn owns.
    pub fn next(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// What an agent says of its own sign-in, where it says it over ACP:
/// codex-acp announces its account with `_auth/status_update`, whose
/// `authStatus.kind` is `none` once it is signed out, as after it found a
/// login it could no longer refresh and logged itself out. It then answers
/// that open with an internal error, not ACP's `auth_required`, so this is
/// how Desk learns the refusal was for the sign-in. `None` for any other
/// message.
pub fn sign_in_status(method: &str, params: &Value) -> Option<bool> {
    if method != "_auth/status_update" {
        return None;
    }
    let kind = params.pointer("/authStatus/kind")?.as_str()?;
    Some(kind != "none")
}

/// Choices kept per option; a longer list is cut.
pub const MAX_CHOICES: usize = 64;

/// Sends one prompt; its answer reaches the sink tagged `tag`.
pub fn prompt(
    connection: &Connection,
    session_id: &str,
    text: &str,
    tag: u64,
) -> Result<(), CallError> {
    connection.send(
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
        tag,
    )
}

/// Ends the turn under way; its prompt answers `cancelled`.
pub fn cancel(connection: &Connection, session_id: &str) -> Result<(), CallError> {
    connection.notify("session/cancel", json!({"sessionId": session_id}))
}

pub fn set_choice(
    connection: &Connection,
    session_id: &str,
    config_id: &str,
    value: &str,
) -> Result<Choices, CallError> {
    connection
        .call(
            "session/set_config_option",
            json!({"sessionId": session_id, "configId": config_id, "value": value}),
            CONTROL_TIMEOUT,
        )
        .map(|answer| choices(&answer))
}

/// What one `session/update` says, as far as Desk uses it.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// Text of the agent's reply, with the ACP message it belongs to when
    /// the adapter names one.
    Text {
        text: String,
        message: Option<String>,
    },
    /// A thought or a plan: shown between the reply's words, so the text
    /// after it is a block of its own.
    Break,
    /// A tool call announced or changed; kept by id for the permission
    /// request that may follow.
    Tool {
        id: String,
        call: Value,
    },
    /// The session's mode moved.
    Mode(String),
    /// The model or effort lists or values moved.
    Choices(Choices),
    /// The session's own facts moved (`session_info_update`): its title, or
    /// what an adapter adds in `_meta`, such as a failed turn's mark.
    Info(Value),
    Other,
}

pub fn read_update(params: &Value) -> Update {
    let Some(update) = params.get("update") else {
        return Update::Other;
    };
    match update.get("sessionUpdate").and_then(Value::as_str) {
        Some("agent_message_chunk") => update
            .pointer("/content/text")
            .and_then(Value::as_str)
            .filter(|_| update.pointer("/content/type").and_then(Value::as_str) == Some("text"))
            .map_or(Update::Other, |text| Update::Text {
                text: text.to_owned(),
                message: update
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
        Some("agent_thought_chunk" | "plan") => Update::Break,
        Some("tool_call" | "tool_call_update") => update
            .get("toolCallId")
            .and_then(Value::as_str)
            .map_or(Update::Other, |id| Update::Tool {
                id: id.to_owned(),
                call: update.clone(),
            }),
        Some("current_mode_update") => update
            .get("currentModeId")
            .and_then(Value::as_str)
            .map_or(Update::Other, |mode| Update::Mode(mode.to_owned())),
        Some("config_option_update") => Update::Choices(choices(update)),
        Some("session_info_update") => Update::Info(update.clone()),
        _ => Update::Other,
    }
}

/// One turn's reply text as the agent's own screen shows it: the streamed
/// chunks of one text block joined as they came, and each later block a
/// paragraph of its own. A block ends where the agent shows something else
/// (a tool call, a thought, a plan) or starts another ACP message.
#[derive(Debug, Default)]
pub struct ReplyBlocks {
    /// The message of the block still open, while one is.
    open: Option<Option<String>>,
    /// Text has been written in this turn.
    written: bool,
}

impl ReplyBlocks {
    /// The text `update` adds to the reply, if any; a block after an
    /// earlier one starts with a paragraph break.
    pub fn piece(&mut self, update: &Update) -> Option<String> {
        match update {
            Update::Text { text, message } => {
                if text.is_empty() {
                    return None;
                }
                let joins = self.open.as_ref() == Some(message);
                let piece = if self.written && !joins {
                    format!("\n\n{text}")
                } else {
                    text.clone()
                };
                self.open = Some(message.clone());
                self.written = true;
                Some(piece)
            }
            Update::Tool { .. } | Update::Break => {
                self.open = None;
                None
            }
            Update::Mode(_) | Update::Choices(_) | Update::Info(_) | Update::Other => None,
        }
    }
}

/// Tool calls a turn keeps for the steps its reply is folded into; past
/// this the rest are not counted.
pub const MAX_TURN_CALLS: usize = 256;

/// The words a failed turn keeps after its mark; the failure line shows
/// only their first line, so what comes past this is cut, and
/// [`FAILURE_CUT`] says so.
pub const MAX_FAILURE_BYTES: usize = 4 * 1024;

/// Ends a failed turn's words that were cut at [`MAX_FAILURE_BYTES`].
pub const FAILURE_CUT: char = '…';

/// Adds `text` to a failed turn's words below [`MAX_FAILURE_BYTES`]: what
/// reaches the cap is cut on a character and ends with [`FAILURE_CUT`],
/// which leaves the words at the cap or past it, so nothing more is kept.
fn keep_failure(words: &mut String, text: &str) {
    if words.len() >= MAX_FAILURE_BYTES {
        return;
    }
    let room = MAX_FAILURE_BYTES - words.len();
    if text.len() < room {
        words.push_str(text);
        return;
    }
    let mut end = room;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    words.push_str(&text[..end]);
    words.push(FAILURE_CUT);
}

/// How a turn ended, as its updates told it.
#[derive(Debug, Default, PartialEq)]
pub struct Answered {
    /// The tool calls it made.
    pub calls: Vec<String>,
    /// The turn met its adapter's failed-turn mark: the text that came after
    /// it, which is the agent's error and not a reply.
    pub failed: Option<String>,
}

/// The turn an owner follows on a session: which one runs, what its reply
/// gains from each update, and the tool calls it made. A turn owns only the
/// arrivals numbered from its prompt on ([`Arrivals`]): what came before,
/// such as the conversation a `session/load` replayed, is no turn's.
#[derive(Debug, Default)]
pub struct TurnReader {
    tag: Option<u64>,
    from: u64,
    calls: Vec<String>,
    /// The startup text the adapter said it writes on its own (Pi's), until
    /// it arrives: it may land inside the first turn and is not its reply.
    prelude: Option<String>,
    reply: ReplyBlocks,
    /// The adapter's mark of a failed turn, when it has one.
    mark: Option<FailedTurnMark>,
    /// The running turn met the mark: the text written since.
    failed: Option<String>,
}

impl TurnReader {
    pub fn new(prelude: Option<String>, mark: Option<FailedTurnMark>) -> Self {
        Self {
            prelude,
            mark,
            ..Self::default()
        }
    }

    /// The running turn's tag.
    pub fn running(&self) -> Option<u64> {
        self.tag
    }

    /// A prompt for `tag` went out; `from`, read from [`Arrivals::next`]
    /// just before it, is the first arrival it owns.
    pub fn start(&mut self, tag: u64, from: u64) {
        self.tag = Some(tag);
        self.from = from;
        self.calls.clear();
        self.reply = ReplyBlocks::default();
        self.failed = None;
    }

    /// What `update`, numbered `number`, adds to the running turn: its tag
    /// and the reply text it gains. A tool call it made is kept for
    /// [`TurnReader::end`], and so is the adapter's failed-turn mark, after
    /// which the turn's text is the failure's words and not its reply.
    pub fn read(&mut self, number: u64, update: &Update) -> Option<(u64, String)> {
        if let Update::Text { text, .. } = update
            && self.prelude.as_deref() == Some(text.as_str())
        {
            self.prelude = None;
            return None;
        }
        if number < self.from {
            return None;
        }
        if let Update::Info(info) = update {
            if self.tag.is_some() && self.mark.is_some_and(|mark| mark.marks(info)) {
                self.failed.get_or_insert_with(String::new);
            }
            return None;
        }
        if let (Some(words), Update::Text { text, .. }) = (&mut self.failed, update) {
            keep_failure(words, text);
            return None;
        }
        let piece = self.reply.piece(update);
        if let Update::Tool { id, .. } = update {
            if self.tag.is_some() && self.calls.len() < MAX_TURN_CALLS && !self.calls.contains(id) {
                self.calls.push(id.clone());
            }
            return None;
        }
        Some((self.tag?, piece?))
    }

    /// The turn `tag` answered, when it was the running one; it runs no
    /// more.
    pub fn end(&mut self, tag: u64) -> Option<Answered> {
        if self.tag != Some(tag) {
            return None;
        }
        self.tag = None;
        Some(Answered {
            calls: std::mem::take(&mut self.calls),
            failed: self.failed.take(),
        })
    }
}

/// Merges a tool call update into what was announced for its id, so a
/// permission request reads the latest of each field.
pub fn merge_tool(announced: &mut Value, update: &Value) {
    if let (Value::Object(announced), Value::Object(update)) = (announced, update) {
        for (key, value) in update {
            if !value.is_null() {
                announced.insert(key.clone(), value.clone());
            }
        }
    }
}

/// The stop reason of a prompt's answer.
pub fn stop_reason(answer: &Value) -> Option<&str> {
    answer.get("stopReason").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_and_effort_choices_are_read_by_category() {
        // The shape claude-agent-acp 0.97 answered session/new with.
        let answer = json!({"configOptions": [
            {"id": "mode", "category": "mode", "type": "select", "currentValue": "auto",
             "options": [{"value": "default", "name": "Manual"}]},
            {"id": "model", "category": "model", "type": "select", "currentValue": "opus",
             "options": [{"value": "opus", "name": "Opus 5.5"}, {"value": "sonnet", "name": "Sonnet 5.5"}]},
            {"id": "effort", "category": "thought_level", "type": "select", "currentValue": "high",
             "options": [{"value": "low", "name": "Low"}, {"value": "high", "name": "High"}]}
        ]});
        let read = choices(&answer);
        let model = read.model.unwrap();
        assert_eq!(model.config_id, "model");
        assert_eq!(model.current.as_deref(), Some("opus"));
        assert_eq!(
            model.values[1],
            ("sonnet".to_owned(), "Sonnet 5.5".to_owned())
        );
        assert_eq!(read.effort.unwrap().config_id, "effort");
    }

    fn numbered(updates: &[(u64, Value)]) -> (TurnReader, Vec<String>) {
        let mut turn = TurnReader::new(Some("pi v1 started".into()), None);
        turn.start(7, 3);
        let text = updates
            .iter()
            .filter_map(|(number, update)| {
                turn.read(*number, &read_update(&json!({"update": update})))
            })
            .map(|(tag, text)| {
                assert_eq!(tag, 7);
                text
            })
            .collect();
        (turn, text)
    }

    /// letter-47179: what arrived before the turn's prompt went out, the
    /// conversation a `session/load` replayed, is no part of its reply or
    /// steps, however late its owner reads it.
    #[test]
    fn a_turn_owns_only_what_arrived_after_its_prompt() {
        let tool = |id: &str| json!({"sessionUpdate": "tool_call", "toolCallId": id});
        let (mut turn, text) = numbered(&[
            (0, said("응답: 알파첫째")),
            (1, tool("old")),
            (2, said("응답: 브라보둘째")),
            (3, said("pi v1 started")),
            (4, said("응답: 델타넷째")),
            (5, tool("new")),
        ]);
        assert_eq!(text, ["응답: 델타넷째"]);
        assert_eq!(
            turn.end(7),
            Some(Answered {
                calls: vec!["new".to_owned()],
                failed: None,
            })
        );
        assert_eq!(turn.running(), None);
        assert_eq!(turn.end(7), None);
    }

    /// codex-acp 2.5.0 with a model that failed (letter-47671, frames
    /// taken on the mini): Codex's thread status turns `systemError`, the
    /// model's error follows as reply text with no message id, and the prompt
    /// is answered `end_turn`. With Codex's mark the turn reads failed and its
    /// error is no reply; without a mark the same frames are a reply, and a
    /// turn that ends `idle` reads as it did.
    #[test]
    fn a_turn_its_adapter_marks_failed_keeps_its_error_out_of_the_reply() {
        let status = |kind: &str| {
            json!({"sessionUpdate": "session_info_update",
                   "_meta": {"codex": {"threadStatus": {"type": kind}}}})
        };
        let active = json!({"sessionUpdate": "session_info_update",
            "_meta": {"codex": {"threadStatus": {"type": "active", "activeFlags": []}}}});
        let retry = json!({"sessionUpdate": "session_info_update", "_meta": {"codex": {"error": {
            "message": "Reconnecting... 1/5",
            "codexErrorInfo": {"responseStreamDisconnected": {"httpStatusCode": null}},
            "additionalDetails": "We’re currently experiencing high demand, which may cause temporary errors.",
            "misalignment": null, "turnId": "01a211bd-a9af-7833-beea-9779755ca9f0", "willRetry": true}}}});
        let title =
            json!({"sessionUpdate": "session_info_update", "title": "fail please @@fail429"});
        let read = |mark, updates: &[Value]| {
            let mut turn = TurnReader::new(None, mark);
            turn.start(7, 0);
            let text: String = updates
                .iter()
                .filter_map(|update| turn.read(1, &read_update(&json!({"update": update}))))
                .map(|(_, text)| text)
                .collect();
            (text, turn.end(7).unwrap().failed)
        };
        let codex = AdapterId::CODEX.failed_turn_mark();
        assert!(codex.is_some());
        let high_demand =
            "We’re currently experiencing high demand, which may cause temporary errors.\n\n";
        let too_many = "exceeded retry limit, last status: 429 Too Many Requests\n\n";
        for (updates, words) in [
            (
                vec![
                    active.clone(),
                    status("systemError"),
                    said("\n\n"),
                    title.clone(),
                ],
                "\n\n",
            ),
            (
                vec![
                    active.clone(),
                    retry.clone(),
                    retry,
                    status("systemError"),
                    said(high_demand),
                    title.clone(),
                ],
                high_demand,
            ),
            (
                vec![
                    active.clone(),
                    status("systemError"),
                    said(too_many),
                    title.clone(),
                ],
                too_many,
            ),
        ] {
            assert_eq!(
                read(codex, &updates),
                (String::new(), Some(words.to_owned()))
            );
            assert_eq!(read(None, &updates), (words.to_owned(), None));
        }
        let answered = json!({"sessionUpdate": "agent_message_chunk",
            "messageId": "msg_f218fd2cdca842c78bedf5e93f462fcd",
            "content": {"type": "text", "text": "응답: fail please say hi"}});
        assert_eq!(
            read(codex, &[active, answered, status("idle"), title]),
            ("응답: fail please say hi".to_owned(), None)
        );
        assert_eq!(AdapterId::CLAUDE.failed_turn_mark(), None);
    }

    /// A failed turn's words past the cap are cut on a character and say
    /// so, and nothing after the cut is kept: a first piece larger than the
    /// cap still gives the failure line its first words.
    #[test]
    fn a_failed_turn_s_words_are_cut_at_the_cap_and_say_so() {
        let mut turn = TurnReader::new(None, AdapterId::CODEX.failed_turn_mark());
        turn.start(7, 0);
        let long = "한도를 넘었어요 ".repeat(400);
        assert!(long.len() > MAX_FAILURE_BYTES);
        for update in [
            json!({"sessionUpdate": "session_info_update",
                   "_meta": {"codex": {"threadStatus": {"type": "systemError"}}}}),
            said(&long),
            said("after the cut"),
        ] {
            assert_eq!(turn.read(1, &read_update(&json!({"update": update}))), None);
        }
        let words = turn.end(7).unwrap().failed.unwrap();
        assert!(words.ends_with(FAILURE_CUT), "not marked as cut");
        assert!(long.starts_with(words.trim_end_matches(FAILURE_CUT)));
        assert!((MAX_FAILURE_BYTES..=MAX_FAILURE_BYTES + 3).contains(&words.len()));
    }

    #[test]
    fn an_agent_that_says_it_is_signed_out_reads_as_signed_out() {
        // codex-acp 2.5.0 after it logged a login it could not refresh out.
        let none = json!({"authStatus": {"kind": "none", "label": "Not logged in"}});
        assert_eq!(sign_in_status("_auth/status_update", &none), Some(false));
        let chatgpt = json!({"authStatus": {"kind": "chatgpt", "label": "ChatGPT Pro"}});
        assert_eq!(sign_in_status("_auth/status_update", &chatgpt), Some(true));
        assert_eq!(sign_in_status("session/update", &none), None);
        assert_eq!(sign_in_status("_auth/status_update", &json!({})), None);
    }

    #[test]
    fn reply_text_and_tool_calls_are_told_apart() {
        assert_eq!(
            read_update(&json!({"update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "안녕"}}})),
            Update::Text {
                text: "안녕".into(),
                message: None
            }
        );
        assert!(matches!(
            read_update(&json!({"update": {"sessionUpdate": "tool_call", "toolCallId": "t1"}})),
            Update::Tool { .. }
        ));
        assert_eq!(
            read_update(&json!({"update": {"sessionUpdate": "agent_thought_chunk",
                "content": {"type": "text", "text": "hmm"}}})),
            Update::Break
        );
    }

    fn reply(updates: &[Value]) -> String {
        let mut blocks = ReplyBlocks::default();
        updates
            .iter()
            .filter_map(|update| blocks.piece(&read_update(&json!({"update": update}))))
            .collect()
    }

    fn said(text: &str) -> Value {
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
    }

    #[test]
    fn a_reply_keeps_one_block_s_chunks_together_and_parts_blocks_a_call_or_a_thought_divides() {
        let text = reply(&[
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}}),
            said("명령을 "),
            said("실행합니다."),
            json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Bash", "status": "pending"}),
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}),
            said("desk-probe-2.txt를 만들었어요."),
            json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "check"}}),
            said("끝."),
        ]);
        assert_eq!(
            text,
            "명령을 실행합니다.\n\ndesk-probe-2.txt를 만들었어요.\n\n끝."
        );
    }

    #[test]
    fn a_reply_parts_two_messages_and_joins_chunks_of_one() {
        let chunk = |id: &str, text: &str| {
            json!({"sessionUpdate": "agent_message_chunk", "messageId": id,
                "content": {"type": "text", "text": text}})
        };
        let text = reply(&[
            chunk("m1", "하나"),
            chunk("m1", " 둘"),
            json!({"sessionUpdate": "current_mode_update", "currentModeId": "ask"}),
            chunk("m1", " 셋"),
            chunk("m2", "넷"),
        ]);
        assert_eq!(text, "하나 둘 셋\n\n넷");
    }
}
