//! What agy's (the Antigravity CLI's) hook events say, and what Hide answers
//! them (PRD antigravity-first-class).
//!
//! agy runs one `hooks.json` block per tool beside the others, and Hide's is
//! named `hide` (`crate::guidance`, [`crate::guidance::GuidanceAgent::Antigravity`]).
//! Its events carry camelCase JSON on stdin, every one naming the
//! conversation it belongs to (agy 1.3.6, measured 2026-10-16):
//!
//! | Event | Hide's answer |
//! | --- | --- |
//! | `SessionStart` (the first prompt of a conversation, never a resume) | the guidance and the Memory index, as an injected system message |
//! | `PreInvocation` (every model call; `invocationNum` 0 is a prompt's first) | the prompt's Memory cards and the pane's letters, the same way; a subagent's counts it working |
//! | `PreToolUse` | `{"decision":"deny","reason"}` for a refused call, `{"decision":"ask"}` on every other path |
//! | `Stop` (one per turn of each conversation) | nothing; a subagent's own stop counts it done |
//!
//! agy refuses a tool call whose `PreToolUse` hook fails (a nonzero exit,
//! output that is not its JSON, a timeout, a missing program), unlike every
//! other agent Hide supports; empty output with exit 0 lets the call run
//! (agy 1.4.0, measured 2026-10-17). So the tool answer is always one of the
//! two lines above, and the installed command prints the ask line itself when
//! the helper is gone.
//!
//! In agy 1.4.0 only `deny` changes what agy does: `allow`, `ask` and an empty
//! answer all leave agy's own permission question exactly as it is with no
//! hook, asking before a command or a write and not before a read (measured
//! 2026-10-17). Hide answers `ask` because agy's hook contract documents
//! `allow` as running the call without its question: Hide never makes agy ask
//! less than agy's own rules would, and so cannot spare the operator agy's
//! question before Hide's own commands either.
//!
//! A subagent is a conversation of its own in the same process and pane,
//! with its own `SessionStart`, `PreInvocation` and `Stop` and no parent
//! field. Its transcript opens with the message its parent sent it
//! (`[Message] ... sender=<parent conversation id> ...`), after any steps a
//! hook injected at its start, which is how its events are told from the
//! pane's own conversation ([`root_of`]). agy 1.4.0 wrote that message as the
//! subagent's `SessionStart` ran; agy 1.5.9 writes it only after the
//! subagent's first model call, and names every event of a conversation its
//! operator never prompted with an empty `lastUserInput`, which is how a
//! subagent is known before its parent is ([`Event::prompted`], measured
//! 2026-11-15).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

/// agy's answer that leaves a tool call to agy's own permission rules.
pub const ASK: &str = r#"{"decision":"ask"}"#;

/// agy's refusal of a tool call; the model reads the reason as the step's
/// error.
pub fn deny(reason: &str) -> String {
    json!({ "decision": "deny", "reason": reason }).to_string()
}

/// agy cuts one injected message at about 48 KiB (49,312 characters of a
/// 161 KB one arrived); Hide's text stays far below it, and a longer one is
/// cut here at a character boundary rather than by agy in the middle.
const INJECT_LIMIT: usize = 40 * 1024;

/// One system message added to the conversation, which agy records in the
/// transcript (`SYSTEM_MESSAGE`) and never draws on screen.
pub fn inject(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut end = text.len().min(INJECT_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let text = &text[..end];
    Some(json!({ "injectSteps": [{ "systemMessage": { "systemMessage": text } }] }).to_string())
}

/// The facts of one agy hook event Hide reads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Event {
    /// The conversation id, a lowercase UUID; one that is not is no id.
    pub conversation: Option<String>,
    /// `invocationNum` of an invocation event: 0 for a prompt's first model
    /// call.
    pub invocation: Option<u64>,
    /// Whether the operator prompted the conversation: `lastUserInput` is
    /// the operator's latest prompt in each event of the pane's own
    /// conversation and empty in each of a subagent's, which only its
    /// parent's message started. `None` from an agy that sends no such field.
    pub prompted: Option<bool>,
    /// That prompt's text, with agy's `<USER_REQUEST>` wrapper taken off
    /// ([`request_text`]); `None` when the event carries none.
    pub prompt: Option<String>,
}

impl Event {
    /// Reads the payload whole; a cut or unreadable one names nothing.
    pub fn read(payload: &[u8], truncated: bool) -> Self {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            conversation_id: Option<String>,
            invocation_num: Option<u64>,
            last_user_input: Option<String>,
        }
        let Some(input) = (!truncated)
            .then(|| serde_json::from_slice::<Input>(payload).ok())
            .flatten()
        else {
            return Self::default();
        };
        Self {
            conversation: input.conversation_id.filter(|id| valid_conversation(id)),
            invocation: input.invocation_num,
            prompted: input
                .last_user_input
                .as_ref()
                .map(|input| !input.is_empty()),
            prompt: input
                .last_user_input
                .map(|input| request_text(&input))
                .filter(|prompt| !prompt.is_empty()),
        }
    }
}

/// agy's conversation ids are lowercase UUIDs (`984dbf44-34b2-444c-91fd-34a2581196c4`).
pub fn valid_conversation(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(at, character)| match at {
            8 | 13 | 18 | 23 => character == '-',
            _ => character.is_ascii_digit() || ('a'..='f').contains(&character),
        })
}

/// The transcript agy writes for `conversation`. The path is built from the
/// id rather than read from the payload's `transcriptPath`, so a hook never
/// opens a file outside agy's own folder.
pub fn transcript(home: &Path, conversation: &str) -> PathBuf {
    hide_session::antigravity::transcript(home, conversation)
}

/// How much of a transcript's opening is read: the steps hooks injected at
/// the start, each at most about 48 KiB ([`INJECT_LIMIT`] for Hide's), and
/// then the parent's message.
const OPENING_LIMIT: u64 = 256 * 1024;

/// The most opening steps read for the parent's message.
const OPENING_STEPS: usize = 8;

/// The most subagent levels followed up to the pane's own conversation; agy
/// nests far fewer, and a longer chain is cut rather than walked.
const NESTING_LIMIT: usize = 8;

/// The pane's own conversation above a subagent `conversation`: its parent,
/// or for a subagent a subagent started, the first conversation up the chain
/// that no other started. `None` for a conversation of its own, and for a
/// subagent whose parent's message is not written yet. Each parent is read
/// from its transcript's opening. In agy 1.4.0 a subagent's `SessionStart`
/// runs as agy writes that message (both stamped the same second), so a start
/// waits until `deadline` for it to appear; the parents above it are already
/// written.
pub fn root_of(home: &Path, conversation: &str, deadline: Instant) -> Option<String> {
    let mut root = parent_of(home, conversation, deadline)?;
    let mut seen = vec![conversation.to_owned(), root.clone()];
    while seen.len() <= NESTING_LIMIT {
        match parent_of(home, &root, Instant::now()) {
            Some(parent) if !seen.contains(&parent) => {
                seen.push(parent.clone());
                root = parent;
            }
            _ => break,
        }
    }
    Some(root)
}

/// The conversation that started subagent `conversation`, read from its
/// transcript's first step, waiting until `deadline` for that line.
// A production wait, not test code: agy gives the hook no event for the
// line it is about to write, and the deadline bounds the wait.
#[allow(clippy::disallowed_methods)]
fn parent_of(home: &Path, conversation: &str, deadline: Instant) -> Option<String> {
    let path = transcript(home, conversation);
    loop {
        match opening(&path) {
            Opening::Message(message) => {
                return sender(&message).filter(|parent| parent != conversation);
            }
            Opening::Own => return None,
            Opening::Unwritten if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Opening::Unwritten => return None,
        }
    }
}

/// What the parent asked subagent `conversation` to do: the `content=` of the
/// `[Message]` that opens its transcript; none for a conversation of its own
/// or one whose message is not written yet.
pub fn subagent_task(home: &Path, conversation: &str) -> Option<String> {
    let Opening::Message(message) = opening(&transcript(home, conversation)) else {
        return None;
    };
    sender(&message)?;
    let task = &message[message.find(" content=")? + " content=".len()..];
    let task = task.split("</SYSTEM_MESSAGE>").next().unwrap_or(task);
    crate::report::subagent_words(task, crate::report::SUBAGENT_TASK_LIMIT)
}

/// How a transcript opens.
enum Opening {
    /// With a parent's message, from its `[Message]` on: a subagent's.
    Message(String),
    /// With a step that is no system message, such as the operator's prompt:
    /// a conversation of its own.
    Own,
    /// With nothing yet, or only the steps hooks injected at its start.
    Unwritten,
}

/// The parent's message among the system messages that open the transcript:
/// agy's own (`source` `SYSTEM`), past those a hook injected (`SYSTEM_SDK`).
fn opening(path: &Path) -> Opening {
    let steps =
        hide_session::read_first_records(path, OPENING_LIMIT, OPENING_STEPS).unwrap_or_default();
    for line in steps {
        let Ok(step) = serde_json::from_str::<Value>(&line) else {
            return Opening::Own;
        };
        if step.get("type").and_then(Value::as_str) != Some("SYSTEM_MESSAGE") {
            return Opening::Own;
        }
        let content = step
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if step.get("source").and_then(Value::as_str) == Some("SYSTEM")
            && let Some(at) = content.find("[Message]")
        {
            return Opening::Message(content[at..].to_owned());
        }
    }
    Opening::Unwritten
}

/// The `sender=<id>` of the `[Message]` a parent conversation sent.
fn sender(message: &str) -> Option<String> {
    let start = message.find(" sender=")? + " sender=".len();
    let id = message.get(start..start + 36)?;
    valid_conversation(id).then(|| id.to_owned())
}

/// How much of a transcript's end is read for the prompt that started the
/// current turn.
const TAIL_LIMIT: u64 = 256 * 1024;

/// The operator's latest prompt in `conversation`, which `PreInvocation`
/// does not carry: the last `USER_INPUT` step of the transcript's tail, with
/// agy's `<USER_REQUEST>` wrapper taken off. It is only the search input of
/// Memory; the prompt itself is never changed.
pub fn last_prompt(home: &Path, conversation: &str) -> Option<String> {
    let tail = hide_session::read_tail(&transcript(home, conversation), TAIL_LIMIT).ok()?;
    tail.records()
        .filter_map(|(_, line)| serde_json::from_str::<Value>(line).ok())
        .filter(|step| step.get("type").and_then(Value::as_str) == Some("USER_INPUT"))
        .filter_map(|step| {
            step.get("content")
                .and_then(Value::as_str)
                .map(request_text)
        })
        .last()
}

/// The text inside agy's `<USER_REQUEST>` wrapper, or the whole content.
pub fn request_text(content: &str) -> String {
    let inner = content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, rest)| rest.split_once("</USER_REQUEST>"))
        .map_or(content, |(inner, _)| inner);
    inner.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARENT: &str = "ade08778-fcdd-4e96-88ba-fd1c4e594fa2";
    const CHILD: &str = "7956974a-cd2e-41c0-bd3b-4c12822badd5";

    fn write_transcript(home: &Path, conversation: &str, lines: &[Value]) {
        let path = transcript(home, conversation);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_answers_are_the_shapes_agy_reads() {
        assert_eq!(
            serde_json::from_str::<Value>(ASK).unwrap(),
            json!({"decision": "ask"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&deny("no")).unwrap(),
            json!({"decision": "deny", "reason": "no"})
        );
        assert_eq!(
            serde_json::from_str::<Value>(&inject("hello").unwrap()).unwrap(),
            json!({"injectSteps": [{"systemMessage": {"systemMessage": "hello"}}]})
        );
        assert!(inject("  \n").is_none());
        let long = "가".repeat(INJECT_LIMIT);
        let injected: Value = serde_json::from_str(&inject(&long).unwrap()).unwrap();
        let text = injected["injectSteps"][0]["systemMessage"]["systemMessage"]
            .as_str()
            .unwrap();
        assert!(text.len() <= INJECT_LIMIT && text.chars().all(|c| c == '가'));
    }

    #[test]
    fn an_event_names_its_conversation_only_when_it_is_an_agy_id() {
        let event = Event::read(
            json!({"conversationId": PARENT, "invocationNum": 0, "initialNumSteps": 3})
                .to_string()
                .as_bytes(),
            false,
        );
        assert_eq!(event.conversation.as_deref(), Some(PARENT));
        assert_eq!(event.invocation, Some(0));
        for id in [
            "",
            "../../etc",
            &PARENT.to_uppercase(),
            &format!("{PARENT}x"),
        ] {
            let read = Event::read(json!({"conversationId": id}).to_string().as_bytes(), false);
            assert_eq!(read.conversation, None, "{id}");
        }
        assert_eq!(
            Event::read(br#"{"conversationId":"#, false),
            Event::default()
        );
        let whole = json!({"conversationId": PARENT}).to_string();
        assert_eq!(Event::read(whole.as_bytes(), true), Event::default());
    }

    #[test]
    fn an_event_says_whether_the_operator_prompted_its_conversation() {
        let read = |input: Value| {
            Event::read(
                json!({"conversationId": PARENT, "lastUserInput": input})
                    .to_string()
                    .as_bytes(),
                false,
            )
            .prompted
        };
        assert_eq!(read(json!("Fix the test")), Some(true));
        // agy 1.5.9 sends a subagent's every event an empty one.
        assert_eq!(read(json!("")), Some(false));
        // An agy older than the field says nothing either way.
        assert_eq!(read(Value::Null), None);
    }

    /// The prompt hook names the prompt it runs for by its digest, so the
    /// core knows a bell's turn from the operator's own (with-hide/hide#2580).
    #[test]
    fn an_event_carries_the_prompt_it_runs_for() {
        let read = |input: Value| {
            Event::read(
                json!({"conversationId": PARENT, "invocationNum": 0, "lastUserInput": input})
                    .to_string()
                    .as_bytes(),
                false,
            )
            .prompt
        };
        assert_eq!(read(json!("Fix the test")).as_deref(), Some("Fix the test"));
        assert_eq!(
            read(json!("<USER_REQUEST>\nFix the test\n</USER_REQUEST>")).as_deref(),
            Some("Fix the test")
        );
        assert_eq!(read(json!("")), None);
        assert_eq!(read(Value::Null), None);
    }

    /// The first step agy writes for a subagent `parent` started.
    fn started_by(parent: &str) -> Value {
        json!({
            "step_index": 0, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "status": "DONE",
            "content": format!("The following is a <SYSTEM_MESSAGE> not actually sent by the user.\n\n<SYSTEM_MESSAGE>\n[Message] timestamp=2026-10-16T04:53:25Z sender={parent} priority=MESSAGE_PRIORITY_HIGH content=Run it.\n</SYSTEM_MESSAGE>"),
        })
    }

    #[test]
    fn a_subagent_is_told_by_the_message_its_parent_sent() {
        let home = tempfile::tempdir().unwrap();
        write_transcript(home.path(), CHILD, &[started_by(PARENT)]);
        write_transcript(
            home.path(),
            PARENT,
            &[
                json!({"step_index": 0, "type": "USER_INPUT", "content": "<USER_REQUEST>\nhi\n</USER_REQUEST>"}),
            ],
        );
        // A subagent the subagent started counts under the pane's own
        // conversation, the one the pane's agent reports.
        let grandchild = "c0c0c0c0-0000-4000-8000-000000000003";
        write_transcript(home.path(), grandchild, &[started_by(CHILD)]);
        let now = Instant::now();
        assert_eq!(root_of(home.path(), CHILD, now).as_deref(), Some(PARENT));
        assert_eq!(
            root_of(home.path(), grandchild, now).as_deref(),
            Some(PARENT)
        );
        assert_eq!(root_of(home.path(), PARENT, now), None);
        // What the parent asked is the message's words; the parent's own
        // conversation was asked nothing.
        assert_eq!(
            subagent_task(home.path(), CHILD).as_deref(),
            Some("Run it.")
        );
        assert_eq!(subagent_task(home.path(), PARENT), None);
        // No transcript before the deadline: a conversation of its own.
        let missing = "00000000-0000-4000-8000-000000000000";
        let waited = Instant::now();
        assert_eq!(
            root_of(home.path(), missing, waited + Duration::from_millis(60)),
            None
        );
        assert!(waited.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn a_subagent_is_told_past_the_steps_hooks_injected_at_its_start() {
        let home = tempfile::tempdir().unwrap();
        // agy 1.5.9: what a hook injected at the subagent's start comes
        // first, and the parent's message lands only after its first model
        // call.
        let injected = json!({
            "step_index": 0, "source": "SYSTEM_SDK", "type": "SYSTEM_MESSAGE",
            "content": "<SYSTEM_MESSAGE>\nThe following system message was injected by a SessionStart hook:\nInside Hide\n</SYSTEM_MESSAGE>",
        });
        write_transcript(home.path(), CHILD, std::slice::from_ref(&injected));
        let now = Instant::now();
        assert_eq!(root_of(home.path(), CHILD, now), None);
        assert_eq!(subagent_task(home.path(), CHILD), None);

        let mut message = started_by(PARENT);
        message["step_index"] = json!(1);
        write_transcript(home.path(), CHILD, &[injected, message]);
        assert_eq!(root_of(home.path(), CHILD, now).as_deref(), Some(PARENT));
        assert_eq!(
            subagent_task(home.path(), CHILD).as_deref(),
            Some("Run it.")
        );

        // The pane's own conversation opens with the operator's prompt, and
        // the replies its subagents send it later name no parent of it.
        let mut reply = started_by(CHILD);
        reply["step_index"] = json!(6);
        write_transcript(
            home.path(),
            PARENT,
            &[
                json!({"step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT", "content": "<USER_REQUEST>\nhi\n</USER_REQUEST>"}),
                reply,
            ],
        );
        assert_eq!(root_of(home.path(), PARENT, now), None);
        assert_eq!(subagent_task(home.path(), PARENT), None);
    }

    #[test]
    fn the_prompt_is_the_last_user_request_of_the_tail() {
        let home = tempfile::tempdir().unwrap();
        write_transcript(
            home.path(),
            PARENT,
            &[
                json!({"step_index": 0, "type": "USER_INPUT", "content": "<USER_REQUEST>\nfirst\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nlocal time\n</ADDITIONAL_METADATA>"}),
                json!({"step_index": 1, "type": "PLANNER_RESPONSE", "content": "ok"}),
                json!({"step_index": 2, "type": "USER_INPUT", "content": "<USER_REQUEST>\nsecond prompt\n</USER_REQUEST>"}),
                json!({"step_index": 3, "type": "SYSTEM_MESSAGE", "content": "injected"}),
            ],
        );
        assert_eq!(
            last_prompt(home.path(), PARENT).as_deref(),
            Some("second prompt")
        );
        assert_eq!(request_text("plain"), "plain");
        assert_eq!(last_prompt(home.path(), CHILD), None);
    }
}
