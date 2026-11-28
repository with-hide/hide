//! What Desk's agent asks to run, as Hide reads it (PRD Desk D-92).
//!
//! The agent runs in its own mode, so what it asks reaches Hide as
//! `session/request_permission` ([`read_request`]): the options it offers,
//! in its own words and order, and what would run, which the card shows
//! ([`card`]). Hide reads no command to decide what asks; the operator's
//! choice under Desk's box does. Under 다 허용 the core answers every
//! request with the agent's allow-once option; under 에이전트대로 each
//! becomes a card of the agent's own options, but a line of `hide` commands
//! ([`hide_line`]), which Hide answers once itself.

use std::path::Path;

use hide_shell::Syntax;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    /// Reading or searching files: what is read, as the agent names it.
    Read { what: String },
    /// Something else the agent names, such as a web fetch or a tool of
    /// its own; its title when Hide can tell nothing more.
    Other {
        title: String,
        reason: Option<String>,
    },
}

/// One answer the request offers, as the agent offers it: its id, its kind
/// and its own name ("Yes, and don't ask again for git commands").
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub id: String,
    pub kind: ChoiceKind,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    #[serde(other)]
    Other,
}

impl ChoiceKind {
    pub fn rejects(self) -> bool {
        matches!(self, Self::RejectOnce | Self::RejectAlways)
    }
}

/// The most options a card keeps, in the agent's order: ACP agents offer two
/// to six (Claude Code's way out of plan mode is the most, five), and every
/// option is kept in `desk.json` and sent on each snapshot.
pub const MAX_OPTIONS: usize = 8;

/// The longest option id kept: an id goes back to the agent as it came, so
/// a longer one cannot be cut and its option is left out.
pub const MAX_OPTION_ID_BYTES: usize = 256;

/// The longest option name kept; a longer one is cut at a character, since
/// it is only shown.
pub const MAX_OPTION_NAME_BYTES: usize = 200;

/// The option that allows this one call and nothing after it.
pub fn allow_once(choices: &[Choice]) -> Option<&str> {
    choices
        .iter()
        .find(|choice| choice.kind == ChoiceKind::AllowOnce)
        .map(|choice| choice.id.as_str())
}

/// A permission request as Hide reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub tool_call_id: Option<String>,
    /// The agent's options, in its order, at most [`MAX_OPTIONS`].
    pub choices: Vec<Choice>,
    pub action: Action,
}

/// What the operator sees on an approval card: the whole command (or the
/// files a change touches, or what is read), the folder, and the agent's
/// reason when it gave one.
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
    Read,
    Other,
}

/// Where a command runs when the agent does not say, and what a bare
/// `hide` and `~` name there.
pub struct Place<'a> {
    /// The `hide` program Desk's `PATH` finds first, the only one a bare
    /// `hide` names there.
    pub hide: &'a Path,
    /// Desk's working folder, in wire spelling.
    pub cwd: &'a str,
    /// The account's home, in wire spelling.
    pub home: Option<&'a str>,
}

/// The card for `action`, with the folder it acts in: the one the agent
/// named, else Desk's own, where the agent's shell starts; for an edit the
/// deepest folder holding every file it changes, none when a path cannot be
/// named.
pub fn card(action: &Action, place: &Place<'_>) -> Card {
    match action {
        Action::Command {
            command,
            folder,
            reason,
        } => Card {
            kind: CardKind::Command,
            command: command.clone(),
            folder: Some(
                folder
                    .as_deref()
                    .map(wire_word)
                    .unwrap_or_else(|| place.cwd.to_owned()),
            ),
            reason: reason.clone(),
        },
        Action::Edit { paths, reason } => Card {
            kind: CardKind::Edit,
            command: paths.join("\n"),
            folder: holding_folder(
                paths
                    .iter()
                    .map(|path| path_of(path, Some(place.cwd), place.home)),
            ),
            reason: reason.clone(),
        },
        Action::Read { what } => Card {
            kind: CardKind::Read,
            command: what.clone(),
            // Where a name it reads is found.
            folder: Some(place.cwd.to_owned()),
            reason: None,
        },
        Action::Other { title, reason } => Card {
            kind: CardKind::Other,
            command: title.clone(),
            folder: None,
            reason: reason.clone(),
        },
    }
}

/// A redirection to or from one of these reads and writes no file.
const SAFE_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "$null"];

/// Whether `line` runs `hide` commands and nothing else: the line Hide
/// answers itself under 에이전트대로 (D-92, "a line made only of `hide`
/// commands"). Each command is `hide` ([`names_hide`]) or the `hide` Desk
/// put first on the agent's `PATH`; they may be piped, joined by `;`, `&&`
/// or `||`, and redirected only to or from nowhere (`2>&1`, `>/dev/null`),
/// and a heredoc whose end word is quoted gives a letter its body. Any other
/// program (`hide ps | head` included), a variable set before a command, a
/// function, a command or process substitution, a backquote, a heredoc
/// that substitutes and a redirection to or from a file make it a line the
/// agent's own card asks, as does a redirection with no word after it,
/// which covers a redirection spelling this reading does not know. On Windows the line must read so in both a POSIX
/// shell's syntax and PowerShell's ([`Syntax::ON_THIS_SYSTEM`]), since Hide
/// does not know which of the two runs it.
pub fn hide_line(line: &str, place: &Place<'_>) -> bool {
    hide_line_in(line, place, Syntax::ON_THIS_SYSTEM)
}

fn hide_line_in(line: &str, place: &Place<'_>, syntaxes: &[Syntax]) -> bool {
    syntaxes.iter().all(|syntax| {
        let read = hide_shell::shell_line(line, *syntax);
        !read.commands.is_empty()
            && !read.missing_target
            && !read.functions
            && read.substitutions.is_empty()
            && read.backquoted.is_empty()
            && !read.bodies.iter().any(|body| *body)
            && read
                .targets
                .iter()
                .all(|(_, target)| SAFE_TARGETS.contains(&target.as_str()))
            && read.commands.iter().all(|words| {
                words
                    .first()
                    .is_some_and(|program| names_hide(program) || Path::new(program) == place.hide)
            })
    })
}

/// Whether a command word calls `hide` by its bare name: exactly `hide`,
/// and on Windows, which finds a program without case and without `.exe`,
/// `hide.exe` and any case of either. The one reading of a `hide` command
/// in Desk's permission and in its turn's steps.
fn names_hide(program: &str) -> bool {
    if cfg!(windows) {
        let lower = program.to_ascii_lowercase();
        lower == "hide" || lower == "hide.exe"
    } else {
        program == "hide"
    }
}

/// Reads `session/request_permission` params. `announced` is the last
/// `tool_call` or `tool_call_update` the agent sent for the same id, which
/// carries what Codex's request leaves out.
pub fn read_request(params: &Value, announced: Option<&Value>) -> Request {
    let call = params.get("toolCall").cloned().unwrap_or(Value::Null);
    let choices = params
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let id = option.get("optionId")?.as_str()?;
                    if id.len() > MAX_OPTION_ID_BYTES {
                        return None;
                    }
                    let mut name = text(option.get("name")).unwrap_or_default();
                    if name.len() > MAX_OPTION_NAME_BYTES {
                        let mut end = MAX_OPTION_NAME_BYTES;
                        while !name.is_char_boundary(end) {
                            end -= 1;
                        }
                        name.truncate(end);
                    }
                    Some(Choice {
                        id: id.to_owned(),
                        kind: option
                            .get("kind")
                            .cloned()
                            .and_then(|kind| serde_json::from_value(kind).ok())
                            .unwrap_or(ChoiceKind::Other),
                        name,
                    })
                })
                .take(MAX_OPTIONS)
                .collect()
        })
        .unwrap_or_default();
    let tool_call_id = call
        .get("toolCallId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let action = tool_action(&call, announced).unwrap_or_else(|| Action::Other {
        // What the agent called it, which is all the card can show.
        title: text(field(&call, announced, "title"))
            .or_else(|| text(field(&call, announced, "kind")))
            .unwrap_or_default(),
        reason: None,
    });
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
    let raw_path = || {
        raw.and_then(|raw| {
            text(raw.get("file_path"))
                .or_else(|| text(raw.get("path")))
                .or_else(|| text(raw.get("notebook_path")))
        })
    };
    match kind.as_str() {
        "read" | "search" => {
            let what = if paths.is_empty() {
                raw_path()
                    .or_else(|| raw.and_then(|raw| text(raw.get("pattern"))))
                    .or_else(|| text(field(call, announced, "title")))
                    .unwrap_or_default()
            } else {
                paths.join("\n")
            };
            Some(Action::Read { what })
        }
        "execute" => {
            // Antigravity names them `CommandLine` and `Cwd`.
            let command = command_text(
                raw.and_then(|raw| raw.get("command").or_else(|| raw.get("CommandLine"))),
            )?;
            let folder = text(raw.and_then(|raw| raw.get("cwd").or_else(|| raw.get("Cwd"))));
            Some(Action::Command {
                command,
                folder,
                reason,
            })
        }
        "edit" | "delete" | "move" => {
            let mut paths = paths;
            if paths.is_empty()
                && let Some(path) = raw_path()
            {
                paths.push(path);
            }
            (!paths.is_empty()).then_some(Action::Edit { paths, reason })
        }
        _ => {
            // With no title, `read_request` names it by its kind.
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
        Some(Action::Read { .. }) => Step::Files,
        Some(Action::Command { command, .. }) => hide_step(&command),
        _ => Step::Other,
    }
}

fn hide_step(line: &str) -> Step {
    let parsed = hide_shell::shell_line(line, hide_shell::Syntax::Posix);
    let mut step = None;
    for words in &parsed.commands {
        let Some((program, rest)) = words.split_first() else {
            continue;
        };
        if !names_hide(program) {
            return Step::Other;
        }
        // A session file keeps the spelling the command had when it ran.
        let rest = hide_shell::retired::current(rest);
        let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
        let this = match rest.as_slice() {
            ["request", "send" | "reply", ..] => Step::Letter,
            ["agent", "spawn", ..] => Step::Start,
            ["agent", "list" | "show", ..]
            | ["ps", ..]
            | ["inbox", ..]
            | ["request", "show", ..]
            | ["watch", "list", ..]
            | ["task", "show", ..]
            | ["mission", "show" | "list", ..]
            | ["view", "list" | "show", ..] => Step::Sessions,
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

/// The folder that holds every one of `paths`: the deepest folder they all
/// lie below, by the wire's own path rules, so a drive's root is `C:/` and
/// paths on two drives share none. None when a path cannot be named or
/// they share no folder.
fn holding_folder(paths: impl Iterator<Item = Option<String>>) -> Option<String> {
    use hide_platform::path::{wire_parent, wire_relative};
    let mut holding: Option<String> = None;
    for path in paths {
        let path = normalize(&path?);
        let mut held = match holding {
            Some(held) => held,
            None => wire_parent(&path)?.to_owned(),
        };
        while wire_relative(&held, &path).is_err() {
            held = wire_parent(&held)?.to_owned();
        }
        holding = Some(held);
    }
    holding
}

/// A word a command names as a path, in wire spelling: absolute, from
/// `~`, or from the folder the command runs in, with its `..` kept, since
/// the system takes `..` from where a link leads.
/// `None` when Hide cannot tell: a variable or a substitution, braces the
/// shell expands into other words, a pattern in a folder's name, which may
/// match a link, another account's `~`, or a relative path in a folder it
/// cannot tell.
fn path_of(word: &str, cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    if word.contains(['$', '`']) || braces_expand(word) {
        return None;
    }
    let word = wire_word(word);
    let names: Vec<&str> = word.split('/').collect();
    let pattern = |name: &&str| name.contains(['*', '?', '[']);
    let last = names
        .iter()
        .rposition(|name| !name.is_empty() && *name != ".");
    if names
        .iter()
        .enumerate()
        .any(|(index, name)| pattern(name) && (Some(index) != last || word.ends_with('/')))
    {
        return None;
    }
    let joined = if let Some(rest) = word.strip_prefix('~') {
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        format!("{}{rest}", home?)
    } else if hide_platform::path::is_wire_absolute(&word) {
        word
    } else {
        format!("{}/{word}", cwd?)
    };
    Some(tidy(&joined))
}

/// Whether the shell expands braces in a word into more words (`{a,b}`,
/// `{1..3}`); `{}` stays as it is.
fn braces_expand(word: &str) -> bool {
    let mut rest = word;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return false;
        };
        let inside = &after[..close];
        if inside.contains(',') || inside.contains("..") {
            return true;
        }
        rest = &after[close + 1..];
    }
    false
}

/// A path without its empty and `.` names; its `..` stay, and a last `/`
/// (or `/.`), which makes the system follow a link at the last name, stays
/// as `/`.
fn tidy(path: &str) -> String {
    let (root, rest) = if let Some(rest) = path.strip_prefix("//") {
        ("//", rest)
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/", rest)
    } else {
        ("", path)
    };
    let names: Vec<&str> = rest
        .split('/')
        .filter(|name| !name.is_empty() && *name != ".")
        .collect();
    let follows = !names.is_empty() && (rest.ends_with('/') || rest.ends_with("/."));
    format!(
        "{root}{}{}",
        names.join("/"),
        if follows { "/" } else { "" }
    )
}

/// A path as a command names it, in wire spelling; on Windows a Git Bash
/// drive mount (`/c/work`) or a native path (`C:\work`) is read as the
/// drive's.
fn wire_word(word: &str) -> String {
    if cfg!(windows) {
        if let Some(native) = hide_platform::path::shell_drive_path(word) {
            return hide_platform::path::to_wire_lossy(Path::new(&native));
        }
        if word.contains('\\') {
            return hide_platform::path::to_wire_lossy(Path::new(word));
        }
    }
    word.to_owned()
}

/// `.` and `..` taken out by names; a `..` at the root stays there, and a
/// drive (`C:`) stays first.
fn normalize(path: &str) -> String {
    let (root, rest) = if let Some(rest) = path.strip_prefix("//") {
        ("//", rest)
    } else if let Some(rest) = path.strip_prefix('/') {
        ("/", rest)
    } else {
        ("", path)
    };
    let mut names: Vec<&str> = Vec::new();
    let mut fixed = 0;
    for (index, name) in rest.split('/').enumerate() {
        match name {
            "" | "." => {}
            ".." => {
                if names.len() > fixed {
                    names.pop();
                }
            }
            name => {
                if index == 0 && root.is_empty() && name.ends_with(':') {
                    fixed = 1;
                }
                names.push(name);
            }
        }
    }
    format!("{root}{}", names.join("/"))
}

#[cfg(test)]
mod tests;
