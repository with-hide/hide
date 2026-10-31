//! What Hide answers when the Desk agent asks to run something (PRD Desk
//! D-18, D-52, D-53, D-60).
//!
//! The agent always runs in its ask mode, so every action its own rules do
//! not already treat as a read reaches Hide as `session/request_permission`.
//! Hide reads what the action would run, from the request and from the tool
//! call the agent announced under the same id, and decides here:
//! a read and a `hide` command Desk may run on its own run without asking,
//! every other command and every change to files waits for the operator's
//! card, and an action Hide cannot read does not run at all. No answer ever
//! picks an option that would let the agent stop asking.

use std::path::Path;

use serde_json::Value;

use crate::adapter::AdapterId;

/// What a permission request asks to run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// A shell command, with the folder it runs in when the agent says.
    Command {
        command: String,
        folder: Option<String>,
        reason: Option<String>,
    },
    /// A change to these files.
    Edit {
        paths: Vec<String>,
        reason: Option<String>,
    },
    /// Reading or searching files.
    Read,
    /// Something else the agent names, such as a web fetch or a tool of
    /// its own.
    Other {
        title: String,
        reason: Option<String>,
    },
}

/// One answer the request offers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Choice {
    pub id: String,
    pub kind: ChoiceKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChoiceKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Other,
}

/// A permission request as Hide reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub tool_call_id: Option<String>,
    pub choices: Vec<Choice>,
    /// `None` when Hide cannot tell what would run.
    pub action: Option<Action>,
}

impl Request {
    /// The option that allows this one call and nothing after it.
    pub fn allow_once(&self) -> Option<&str> {
        self.choices
            .iter()
            .find(|choice| choice.kind == ChoiceKind::AllowOnce)
            .map(|choice| choice.id.as_str())
    }

    /// The option that refuses this call, preferring the one that refuses
    /// only it.
    pub fn reject(&self) -> Option<&str> {
        [ChoiceKind::RejectOnce, ChoiceKind::RejectAlways]
            .into_iter()
            .find_map(|kind| self.choices.iter().find(|choice| choice.kind == kind))
            .map(|choice| choice.id.as_str())
    }
}

/// What the operator sees on an approval card: the whole command (or the
/// files a change touches), the folder, and the agent's reason when it gave
/// one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Card {
    pub kind: CardKind,
    pub command: String,
    pub folder: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CardKind {
    Command,
    Edit,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// Runs without a card.
    Allow,
    /// Waits for the operator's `한 번 허용` or `거절`.
    Ask(Card),
    /// Hide cannot tell what would run, or cannot answer with a one-time
    /// allowance: it does not run (D-60).
    Refuse,
}

/// The `hide` commands Desk runs without asking: reading what runs, letters,
/// watches and starting work, which is what Desk is for, and the CLI's own
/// help. Every other `hide` command (`hide agent archive`, `hide stop`, ...)
/// waits for a card (D-38); the Workspace, View, Memory, mission and plan
/// commands the core refuses Desk (`desk_has_no_checkout`) are not here.
/// A spawn's words for the agent's own CLI, which could lift its
/// permissions, are refused by the core (`desk_spawn_args`), since Codex's
/// rules ([`crate::codex`]) free this list by its start alone.
pub(crate) const HIDE_WITHOUT_CARD: &[&[&str]] = &[
    &["guide"],
    &["contract"],
    &["status"],
    &["version"],
    &["agent", "list"],
    &["agent", "show"],
    &["agent", "catalog"],
    &["agent", "spawn"],
    &["request", "send"],
    &["request", "reply"],
    &["request", "show"],
    &["request", "ack"],
    &["request", "cancel"],
    &["inbox"],
    &["watch", "start"],
    &["watch", "list"],
    &["watch", "stop"],
    &["watch", "assign"],
    &["ps"],
    &["device", "list"],
];

/// Decides a request. `hide` is the `hide` program Desk's `PATH` finds
/// first, the only one a bare `hide` names there.
pub fn decide(request: &Request, hide: &Path) -> Verdict {
    if request.allow_once().is_none() {
        return Verdict::Refuse;
    }
    let Some(action) = &request.action else {
        return Verdict::Refuse;
    };
    match action {
        Action::Read => Verdict::Allow,
        Action::Command {
            command,
            folder,
            reason,
        } => {
            if hide_without_card(command, hide) {
                Verdict::Allow
            } else {
                Verdict::Ask(Card {
                    kind: CardKind::Command,
                    command: command.clone(),
                    folder: folder.clone(),
                    reason: reason.clone(),
                })
            }
        }
        Action::Edit { paths, reason } => Verdict::Ask(Card {
            kind: CardKind::Edit,
            command: paths.join("\n"),
            folder: None,
            reason: reason.clone(),
        }),
        Action::Other { title, reason } => Verdict::Ask(Card {
            kind: CardKind::Other,
            command: title.clone(),
            folder: None,
            reason: reason.clone(),
        }),
    }
}

/// Every simple command of the line is a `hide` command Desk may run on its
/// own, and the line writes nothing and runs nothing else: no redirection,
/// no substitution, no assignment.
fn hide_without_card(line: &str, hide: &Path) -> bool {
    if line.contains('`') || line.contains("$(") || line.contains("<(") || line.contains(">(") {
        return false;
    }
    let parsed = hide_shell::shell_line(line);
    if !parsed.targets.is_empty() || !parsed.substitutions.is_empty() || parsed.commands.is_empty()
    {
        return false;
    }
    // A heredoc's body is set aside by the reader, so a line holding one
    // is not read whole here.
    if line.contains("<<") {
        return false;
    }
    parsed.commands.iter().all(|words| {
        let Some((program, rest)) = words.split_first() else {
            return false;
        };
        let names_hide = program == "hide" || Path::new(program) == hide;
        names_hide
            && rest.iter().all(|word| !word.contains('$'))
            && HIDE_WITHOUT_CARD.iter().any(|allowed| {
                rest.len() >= allowed.len()
                    && rest
                        .iter()
                        .zip(allowed.iter())
                        .all(|(word, want)| word == want)
            })
    })
}

/// Reads `session/request_permission` params. `announced` is the last
/// `tool_call` or `tool_call_update` the agent sent for the same id, which
/// carries what Codex's request leaves out.
pub fn read_request(adapter: AdapterId, params: &Value, announced: Option<&Value>) -> Request {
    let call = params.get("toolCall").cloned().unwrap_or(Value::Null);
    let choices = params
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    Some(Choice {
                        id: option.get("optionId")?.as_str()?.to_owned(),
                        kind: match option.get("kind").and_then(Value::as_str) {
                            Some("allow_once") => ChoiceKind::AllowOnce,
                            Some("allow_always") => ChoiceKind::AllowAlways,
                            Some("reject_once") => ChoiceKind::RejectOnce,
                            Some("reject_always") => ChoiceKind::RejectAlways,
                            _ => ChoiceKind::Other,
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let tool_call_id = call
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let action = match adapter {
        AdapterId::Pi => pi_action(&call),
        AdapterId::Claude | AdapterId::Codex => tool_action(&call, announced),
    };
    Request {
        tool_call_id,
        choices,
        action,
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A command given as words, as Codex may, read as the line a shell would
/// run: `bash -lc <script>` is its script.
fn command_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(line) => text(Some(&Value::String(line.clone()))),
        Value::Array(given) => {
            let words: Vec<&str> = given.iter().filter_map(Value::as_str).collect();
            if words.len() != given.len() || words.is_empty() {
                return None;
            }
            match words.as_slice() {
                [shell, flag, script]
                    if ["-c", "-lc"].contains(flag)
                        && ["bash", "sh", "zsh", "/bin/bash", "/bin/sh", "/bin/zsh"]
                            .contains(shell) =>
                {
                    Some((*script).to_owned())
                }
                _ => Some(words.join(" ")),
            }
        }
        _ => None,
    }
}

/// The value of `key` in the request's own call, or else in the call the
/// agent announced.
fn field<'a>(call: &'a Value, announced: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    call.get(key)
        .filter(|value| !value.is_null() && *value != &Value::Object(Default::default()))
        .or_else(|| announced.and_then(|announced| announced.get(key)))
        .filter(|value| !value.is_null())
}

fn tool_action(call: &Value, announced: Option<&Value>) -> Option<Action> {
    let kind = text(field(call, announced, "kind")).unwrap_or_default();
    let raw = field(call, announced, "rawInput");
    let reason = raw
        .and_then(|raw| text(raw.get("description")).or_else(|| text(raw.get("reason"))))
        .or_else(|| text(raw.and_then(|raw| raw.get("justification"))));
    let paths: Vec<String> = field(call, announced, "locations")
        .and_then(Value::as_array)
        .map(|locations| {
            locations
                .iter()
                .filter_map(|location| text(location.get("path")))
                .collect()
        })
        .unwrap_or_default();
    match kind.as_str() {
        "read" | "search" => Some(Action::Read),
        "execute" => {
            let command = command_text(raw.and_then(|raw| raw.get("command")))?;
            let folder = text(raw.and_then(|raw| raw.get("cwd")));
            Some(Action::Command {
                command,
                folder,
                reason,
            })
        }
        "edit" | "delete" | "move" => {
            let mut paths = paths;
            if paths.is_empty()
                && let Some(path) = raw.and_then(|raw| {
                    text(raw.get("file_path"))
                        .or_else(|| text(raw.get("path")))
                        .or_else(|| text(raw.get("notebook_path")))
                })
            {
                paths.push(path);
            }
            (!paths.is_empty()).then_some(Action::Edit { paths, reason })
        }
        _ => {
            // A title that only names a generic kind ("Run command") says
            // nothing the operator could judge.
            let title = text(field(call, announced, "title"))?;
            let input = raw
                .filter(|raw| raw.as_object().is_some_and(|raw| !raw.is_empty()))
                .map(|raw| format!("{title}\n{raw}"));
            Some(Action::Other {
                title: input.unwrap_or(title),
                reason,
            })
        }
    }
}

/// What one of Desk's tool calls did, as its turn's folded line counts it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// A `hide` command that reads what runs: sessions, letters, plans.
    Sessions,
    /// A file read or search.
    Files,
    /// `hide request send` or `reply`: a letter.
    Letter,
    /// `hide agent spawn`: a new session.
    Start,
    /// Any other command or tool.
    Other,
}

/// What an announced tool call (`tool_call`, merged with its updates) did.
pub fn step(call: &Value) -> Step {
    match tool_action(call, None) {
        Some(Action::Read) => Step::Files,
        Some(Action::Command { command, .. }) => hide_step(&command),
        _ => Step::Other,
    }
}

fn hide_step(line: &str) -> Step {
    let parsed = hide_shell::shell_line(line);
    let mut step = None;
    for words in &parsed.commands {
        let Some((program, rest)) = words.split_first() else {
            continue;
        };
        if Path::new(program)
            .file_stem()
            .and_then(|stem| stem.to_str())
            != Some("hide")
        {
            return Step::Other;
        }
        let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
        let this = match rest.as_slice() {
            ["request", "send" | "reply", ..] => Step::Letter,
            ["agent", "spawn", ..] => Step::Start,
            ["agent", "list" | "show", ..]
            | ["ps", ..]
            | ["inbox", ..]
            | ["request", "show", ..]
            | ["watch", "list", ..]
            | ["plan", "show", ..]
            | ["mission", "show" | "list", ..]
            | ["view", "list" | "status", ..] => Step::Sessions,
            _ => Step::Other,
        };
        // A line of several commands counts as its most telling one.
        step = Some(match (step, this) {
            (Some(Step::Start), _) | (_, Step::Start) => Step::Start,
            (Some(Step::Letter), _) | (_, Step::Letter) => Step::Letter,
            (Some(Step::Sessions), _) | (_, Step::Sessions) => Step::Sessions,
            _ => Step::Other,
        });
    }
    step.unwrap_or(Step::Other)
}

/// The confirm Desk's own Pi extension sends for each tool call it holds:
/// its message carries the tool and its input.
fn pi_action(call: &Value) -> Option<Action> {
    let raw = call.get("rawInput")?;
    if raw.get("method").and_then(Value::as_str) != Some("confirm")
        || raw.get("title").and_then(Value::as_str) != Some(crate::adapter::PI_CONFIRM_TITLE)
    {
        return None;
    }
    let message: Value = serde_json::from_str(raw.get("message")?.as_str()?).ok()?;
    if message.get("hide_desk").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let tool = text(message.get("tool"))?;
    let input = message.get("input").cloned().unwrap_or(Value::Null);
    match tool.as_str() {
        "read" | "grep" | "find" | "ls" => Some(Action::Read),
        "bash" => Some(Action::Command {
            command: text(input.get("command"))?,
            folder: text(input.get("cwd")),
            reason: None,
        }),
        "edit" | "write" => Some(Action::Edit {
            paths: vec![text(input.get("path"))?],
            reason: None,
        }),
        _ => Some(Action::Other {
            title: format!("{tool}\n{input}"),
            reason: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HIDE: &str = "/Applications/hide.app/Contents/Resources/hide";

    #[test]
    fn a_turns_tool_calls_count_as_sessions_read_letters_and_starts() {
        let run =
            |command: &str| step(&json!({"kind": "execute", "rawInput": {"command": command}}));
        assert_eq!(run("hide agent show agent-3"), Step::Sessions);
        assert_eq!(run("hide ps && hide agent list"), Step::Sessions);
        assert_eq!(
            run("hide request send agent-3 --intent k --body hi"),
            Step::Letter
        );
        assert_eq!(run("hide agent spawn --name a --intent k"), Step::Start);
        assert_eq!(run("hide agent archive agent-3"), Step::Other);
        assert_eq!(run("ls -la"), Step::Other);
        assert_eq!(
            step(&json!({"kind": "read", "locations": [{"path": "/a"}]})),
            Step::Files
        );
    }

    fn options() -> Value {
        json!([
            {"optionId": "allow-once", "name": "Yes", "kind": "allow_once"},
            {"optionId": "allow-always", "name": "Always", "kind": "allow_always"},
            {"optionId": "reject", "name": "No", "kind": "reject_once"}
        ])
    }

    fn claude_bash(command: &str) -> Request {
        read_request(
            AdapterId::Claude,
            &json!({"toolCall": {"toolCallId": "t1", "kind": "execute", "title": command,
                "rawInput": {"command": command, "description": "Why it runs"}},
                "options": options()}),
            None,
        )
    }

    fn verdict(command: &str) -> Verdict {
        decide(&claude_bash(command), Path::new(HIDE))
    }

    #[test]
    fn a_hide_command_desk_uses_runs_without_a_card() {
        for line in [
            "hide agent list",
            "hide ps --json",
            "hide request send record-now --intent desk-1 --body 'check the dark capture'",
            "hide agent spawn --name mention --intent desk-2 --kind claude --repo /r --branch desk/mention",
            &format!("{HIDE} inbox"),
            "hide guide && hide agent list",
        ] {
            assert_eq!(verdict(line), Verdict::Allow, "{line}");
        }
    }

    #[test]
    fn archive_stop_and_any_other_hide_command_ask_first() {
        for line in [
            "hide agent archive agent-3",
            "hide stop",
            "hide agent end agent-3",
        ] {
            assert!(matches!(verdict(line), Verdict::Ask(_)), "{line}");
        }
    }

    #[test]
    fn a_hide_command_joined_to_anything_else_asks() {
        for line in [
            "hide agent list | jq .",
            "hide agent list > agents.json",
            "hide agent list; rm -rf build",
            "hide request send a --intent i --body \"$(cat notes.md)\"",
            "hide request send a --intent i --body `cat notes.md`",
            "FOO=1 hide agent list",
            "hide request send a --intent i --body $SECRET",
            "/tmp/hide agent list",
            "hide request send a --intent i --body-file - <<EOF\nhi\nEOF",
            "hide agent list\nrm -rf build",
            "(hide agent list; rm -rf build)",
            "hide() { rm -rf build; }; hide agent list",
            "hide mission show mission-1",
        ] {
            assert!(matches!(verdict(line), Verdict::Ask(_)), "{line}");
        }
    }

    #[test]
    fn a_shell_read_from_claude_still_asks() {
        // Claude's own read-only commands are held by Desk's ask rule and
        // reach Hide; only Codex's sandboxed reads pass on the agent's side.
        let Verdict::Ask(card) = verdict("ls -la") else {
            panic!("ls asks")
        };
        assert_eq!(card.command, "ls -la");
        assert_eq!(card.reason.as_deref(), Some("Why it runs"));
    }

    #[test]
    fn a_read_runs_and_an_edit_asks_with_its_files() {
        let read = read_request(
            AdapterId::Claude,
            &json!({"toolCall": {"toolCallId": "t2", "kind": "read", "title": "Read"},
                "options": options()}),
            None,
        );
        assert_eq!(decide(&read, Path::new(HIDE)), Verdict::Allow);
        let edit = read_request(
            AdapterId::Claude,
            &json!({"toolCall": {"toolCallId": "t3", "kind": "edit", "title": "Edit",
                "rawInput": {"file_path": "/r/web/src/PaneHeader.tsx"}},
                "options": options()}),
            None,
        );
        let Verdict::Ask(card) = decide(&edit, Path::new(HIDE)) else {
            panic!("an edit asks")
        };
        assert_eq!(card.kind, CardKind::Edit);
        assert_eq!(card.command, "/r/web/src/PaneHeader.tsx");
    }

    #[test]
    fn codex_s_request_is_read_with_the_call_it_announced() {
        // codex-acp 2.3 sent the request with only the id and the kind.
        let announced = json!({"sessionUpdate": "tool_call", "toolCallId": "exec-1",
            "kind": "execute", "title": "touch x",
            "rawInput": {"command": ["/bin/zsh", "-lc", "touch x"], "cwd": "/r"}});
        let request = read_request(
            AdapterId::Codex,
            &json!({"toolCall": {"toolCallId": "exec-1", "kind": "execute", "status": "pending"},
                "options": [{"optionId": "allow_once", "kind": "allow_once"},
                            {"optionId": "cancel", "kind": "reject_once"}]}),
            Some(&announced),
        );
        let Verdict::Ask(card) = decide(&request, Path::new(HIDE)) else {
            panic!("asks")
        };
        assert_eq!(card.command, "touch x");
        assert_eq!(card.folder.as_deref(), Some("/r"));
        assert_eq!(request.reject(), Some("cancel"));
    }

    #[test]
    fn a_request_hide_cannot_read_does_not_run() {
        let bare = read_request(
            AdapterId::Codex,
            &json!({"toolCall": {"toolCallId": "exec-2", "kind": "execute"}, "options": options()}),
            None,
        );
        assert_eq!(bare.action, None);
        assert_eq!(decide(&bare, Path::new(HIDE)), Verdict::Refuse);
        let no_once = read_request(
            AdapterId::Claude,
            &json!({"toolCall": {"toolCallId": "t", "kind": "read"},
                "options": [{"optionId": "always", "kind": "allow_always"}]}),
            None,
        );
        assert_eq!(decide(&no_once, Path::new(HIDE)), Verdict::Refuse);
    }

    #[test]
    fn pi_s_confirm_from_desk_s_extension_is_read_and_another_is_not() {
        let confirm = |title: &str, message: &str| {
            read_request(
                AdapterId::Pi,
                &json!({"toolCall": {"toolCallId": "pi-ui-1", "kind": "other", "title": title,
                    "rawInput": {"method": "confirm", "title": title, "message": message}},
                    "options": [{"optionId": "yes", "kind": "allow_once"},
                                {"optionId": "no", "kind": "reject_once"}]}),
                None,
            )
        };
        let bash = confirm(
            crate::adapter::PI_CONFIRM_TITLE,
            r#"{"hide_desk":1,"tool":"bash","input":{"command":"hide agent list"}}"#,
        );
        assert_eq!(decide(&bash, Path::new(HIDE)), Verdict::Allow);
        let write = confirm(
            crate::adapter::PI_CONFIRM_TITLE,
            r#"{"hide_desk":1,"tool":"write","input":{"path":"notes.md","content":"x"}}"#,
        );
        assert!(matches!(decide(&write, Path::new(HIDE)), Verdict::Ask(_)));
        let other = confirm("Delete branch?", "yes or no");
        assert_eq!(decide(&other, Path::new(HIDE)), Verdict::Refuse);
    }
}
