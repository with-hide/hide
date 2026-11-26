//! The agent's own commands (ACP v1 slash commands): the list an agent sends
//! as `available_commands_update`, any time after its session opens and again
//! whenever it changes, and how Desk tells a message that runs one. A command
//! runs as the first text of a prompt, `/name args`; there is no call of its
//! own.
//!
//! What Desk keeps out of an agent's list is the agent's definition file's
//! `[desk] left_out` ([`AdapterId::left_out`]), read here and nowhere else.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AdapterId;

/// One command an agent lists: its name as the operator types it to run it
/// (`/compact`; [`written`]), what it does, and the hint for what follows
/// it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Commands kept from one list; an agent with hundreds of skills sends more,
/// and the rest are left out, in the agent's order, with a diagnostic.
pub const MAX_COMMANDS: usize = 256;
/// A longer name cannot be cut and still run, so its command is left out.
pub const MAX_NAME_BYTES: usize = 64;
/// A description or hint past these is cut on a character and ends with `…`.
pub const MAX_DESCRIPTION_BYTES: usize = 200;
pub const MAX_HINT_BYTES: usize = 100;

/// A list as read: the commands kept, and how many the caps left out.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Commands {
    pub commands: Vec<Command>,
    pub cut: usize,
}

/// The commands of an `available_commands_update`, but those `adapter`'s
/// definition leaves out, within the caps.
pub fn read(adapter: AdapterId, update: &Value) -> Commands {
    let left_out = adapter.left_out();
    let mut read = Commands::default();
    for entry in update
        .get("availableCommands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = entry
            .get("name")
            .and_then(Value::as_str)
            .map(|name| name.trim().trim_start_matches('/'))
        else {
            continue;
        };
        if left_out.iter().any(|left| left == name) {
            continue;
        }
        if name.is_empty()
            || name.len() > MAX_NAME_BYTES
            || name.chars().any(char::is_whitespace)
            || read.commands.len() == MAX_COMMANDS
        {
            read.cut += 1;
            continue;
        }
        let name = written(name);
        if read.commands.iter().any(|known| known.name == name) {
            continue;
        }
        let text = |pointer: &str, cap: usize| {
            entry
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| cut(text, cap))
        };
        read.commands.push(Command {
            name,
            description: text("/description", MAX_DESCRIPTION_BYTES).unwrap_or_default(),
            hint: text("/input/hint", MAX_HINT_BYTES),
        });
    }
    read
}

/// A list as it is saved: ACP's own entries (`{name, description,
/// input.hint}`), so reading one back is [`read`] and the caps and the
/// definition's left-outs of the build that reads it apply again.
pub fn saved(commands: &[Command]) -> Value {
    Value::Array(
        commands
            .iter()
            .map(|command| {
                let mut entry = serde_json::json!({"name": command.name, "description": command.description});
                if let Some(hint) = &command.hint {
                    entry["input"] = serde_json::json!({"hint": hint});
                }
                entry
            })
            .collect(),
    )
}

/// A saved list (`saved`) read back for `adapter`: an entry that is not a
/// command is skipped, as in an agent's own list.
pub fn read_saved(adapter: AdapterId, entries: &Value) -> Commands {
    read(adapter, &serde_json::json!({"availableCommands": entries}))
}

/// The first line of `text`, at most `cap` bytes, cut on a character.
fn cut(text: &str, cap: usize) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.len() <= cap && line.len() == text.len() {
        return line.to_owned();
    }
    let room = cap - '…'.len_utf8();
    let mut end = line.len().min(room);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &line[..end])
}

/// How Desk sends a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    /// Words for the agent, which Desk's guide may go before.
    Message,
    /// It starts with a command the agent listed: it goes as written, with
    /// nothing in front, so the agent runs it.
    Command,
    /// It starts with a command `adapter`'s definition leaves out: it never
    /// goes.
    LeftOut,
}

/// What `message` is to `adapter`, whose last list is `commands`: it runs a
/// command when its first word is a listed command as typed, and it is left
/// out when it starts with the words of one of the definition's left-out
/// commands after a `/`, listed or not.
pub fn kind(adapter: AdapterId, commands: &[Command], message: &str) -> Kind {
    let message = message.trim_start();
    if let Some(typed) = message.strip_prefix('/') {
        let words: Vec<&str> = typed.split_whitespace().collect();
        let left_out = adapter.left_out().iter().any(|left| {
            let left: Vec<&str> = left.split(' ').collect();
            words.len() >= left.len() && words[..left.len()] == left[..]
        });
        if left_out {
            return Kind::LeftOut;
        }
    }
    let first = message.split_whitespace().next().unwrap_or_default();
    if commands.iter().any(|command| command.name == first) {
        Kind::Command
    } else {
        Kind::Message
    }
}

/// A command as the operator types it: `/` and the name the agent lists, or
/// that name alone when it starts with a sigil of its own (Codex's
/// `$skill`).
fn written(name: &str) -> String {
    if name.starts_with(|c: char| c.is_alphanumeric()) {
        format!("/{name}")
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(names: &[&str]) -> Value {
        json!({"sessionUpdate": "available_commands_update", "availableCommands":
            names.iter().map(|name| json!({"name": name, "description": format!("{name} it")})).collect::<Vec<_>>()})
    }

    #[test]
    fn a_list_keeps_name_description_and_hint_and_leaves_out_what_the_definition_names() {
        let mut list = update(&["compact", "logout", "status", "compact"]);
        list["availableCommands"][0]["input"] = json!({"hint": "what to keep"});
        let read = read(AdapterId::CODEX, &list);
        assert_eq!(
            read.commands,
            [
                Command {
                    name: "/compact".into(),
                    description: "compact it".into(),
                    hint: Some("what to keep".into()),
                },
                Command {
                    name: "/status".into(),
                    description: "status it".into(),
                    hint: None,
                },
            ]
        );
        assert_eq!(read.cut, 0);
        // Another agent's definition leaves no `logout` out.
        let pi = read_names(AdapterId::PI, &["logout"]);
        assert_eq!(pi, ["/logout"]);
    }

    fn read_names(adapter: AdapterId, names: &[&str]) -> Vec<String> {
        read(adapter, &update(names))
            .commands
            .into_iter()
            .map(|command| command.name)
            .collect()
    }

    #[test]
    fn every_left_out_command_of_the_brief_is_kept_out_of_its_agent_s_list() {
        for (adapter, names) in [
            (AdapterId::CODEX, &["logout"][..]),
            (
                AdapterId::CLAUDE,
                &["remote-control", "config", "auto-mode-setup"][..],
            ),
            (
                AdapterId::OMP,
                &["move", "wt", "ssh", "plugins", "force"][..],
            ),
        ] {
            assert_eq!(
                read_names(adapter, names),
                Vec::<String>::new(),
                "{adapter:?}"
            );
            for name in names {
                assert_eq!(kind(adapter, &[], &format!("/{name}")), Kind::LeftOut);
            }
        }
        // `/session` stays omp's; only `/session delete` never goes.
        assert_eq!(read_names(AdapterId::OMP, &["session"]), ["/session"]);
        let session = read(AdapterId::OMP, &update(&["session"])).commands;
        assert_eq!(kind(AdapterId::OMP, &session, "/session"), Kind::Command);
        assert_eq!(
            kind(AdapterId::OMP, &session, "  /session   delete abc"),
            Kind::LeftOut
        );
    }

    #[test]
    fn a_list_past_its_caps_keeps_the_first_commands_and_cuts_long_words_on_a_character() {
        let names: Vec<String> = (0..MAX_COMMANDS + 3).map(|n| format!("c{n}")).collect();
        let mut list = update(&names.iter().map(String::as_str).collect::<Vec<_>>());
        list["availableCommands"][0]["description"] = json!("가".repeat(MAX_DESCRIPTION_BYTES));
        list["availableCommands"][1]["name"] = json!("x".repeat(MAX_NAME_BYTES + 1));
        list["availableCommands"][2]["description"] = json!("first line\nsecond");
        let read = read(AdapterId::PI, &list);
        assert_eq!(read.commands.len(), MAX_COMMANDS);
        // The long name, and the two past the cap.
        assert_eq!(read.cut, 3);
        let long = &read.commands[0].description;
        assert!(long.len() <= MAX_DESCRIPTION_BYTES && long.ends_with('…'));
        assert_eq!(read.commands[1].name, "/c2");
        assert_eq!(read.commands[1].description, "first line…");
        assert_eq!(
            read.commands.last().unwrap().name,
            format!("/c{MAX_COMMANDS}")
        );
    }

    #[test]
    fn a_message_runs_a_command_only_when_its_first_word_is_a_listed_one() {
        let commands = read(
            AdapterId::CODEX,
            &update(&["compact", "$review-pr", "/init"]),
        )
        .commands;
        let names: Vec<&str> = commands
            .iter()
            .map(|command| command.name.as_str())
            .collect();
        assert_eq!(names, ["/compact", "$review-pr", "/init"]);
        let kind = |message: &str| kind(AdapterId::CODEX, &commands, message);
        assert_eq!(kind("/compact"), Kind::Command);
        assert_eq!(kind("/compact keep the plan"), Kind::Command);
        assert_eq!(kind("$review-pr 12"), Kind::Command);
        assert_eq!(kind("/compactly"), Kind::Message);
        assert_eq!(kind("/unknown"), Kind::Message);
        assert_eq!(kind("please /compact"), Kind::Message);
        assert_eq!(kind("compact"), Kind::Message);
        assert_eq!(kind("/init ."), Kind::Command);
    }
}
