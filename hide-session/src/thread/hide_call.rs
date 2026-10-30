//! A `hide` command a session ran, read from its shell call, so a thread can
//! draw it as the Hide part it made (PRD session-docs-and-assistant B26,
//! D-72): a plan's steps, a new session, a letter, an opened View, a
//! mission. Only the command's words are read; what the part shows now is
//! Hide's own state, which the screen looks up by the names read here.

use hide_shell::{is_assignment, is_prefix, simple_commands};
use serde_json::Value;

/// What a `hide` command made.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HideCall {
    /// `hide plan set|node|approve|drop`: the session's own plan moved.
    Plan,
    /// `hide agent spawn`: a new session, by the name and intent it was
    /// given, and the agent id its answer named when the call printed one.
    Spawn {
        name: Option<String>,
        intent: Option<String>,
        agent_id: Option<String>,
    },
    /// `hide request send <to>`: a letter, with its body when it was written
    /// on the command line.
    Letter { to: String, body: Option<String> },
    /// `hide browser|file|diff open <target>`: a View the session opened.
    View { view: ViewKind, target: String },
    /// `hide mission ...`: a command of a mission, named when it said which.
    Mission { mission: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Browser,
    File,
    Diff,
}

/// The `hide` command a shell call's command line ran, the first one when
/// it ran several; `None` when it ran none a thread draws as a part (a read
/// such as `hide plan show` or `hide inbox` stays a shell call).
pub fn hide_call(command: &str) -> Option<HideCall> {
    simple_commands(command)
        .iter()
        .find_map(|words| read(program_words(words)?))
}

/// The agent a `hide agent spawn` call's output names: the id of the
/// `workspace_result` it printed.
pub fn spawned_agent(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let answer = serde_json::from_str::<Value>(line.trim()).ok()?;
        let result = &answer["result"];
        [&result["agent_id"], &result["id"]]
            .into_iter()
            .find_map(Value::as_str)
            .filter(|id| id.starts_with("agent-"))
            .map(str::to_owned)
    })
}

/// A command's words after the environment and prefix words, when its
/// program is `hide`.
fn program_words(words: &[String]) -> Option<&[String]> {
    let start = words
        .iter()
        .position(|word| !is_assignment(word) && !is_prefix(word, true))?;
    let words = &words[start..];
    let program = words.first()?.rsplit(['/', '\\']).next()?;
    matches!(program, "hide" | "hide.exe").then(|| &words[1..])
}

fn read(args: &[String]) -> Option<HideCall> {
    let (verb, rest) = args.split_first()?;
    let sub = rest.first().map(String::as_str);
    let tail = rest.get(1..).unwrap_or_default();
    match (verb.as_str(), sub) {
        ("plan", Some("set" | "node" | "approve" | "drop")) => Some(HideCall::Plan),
        ("agent", Some("spawn")) => Some(HideCall::Spawn {
            name: option(tail, "--name"),
            intent: option(tail, "--intent"),
            agent_id: None,
        }),
        ("request", Some("send")) => Some(HideCall::Letter {
            to: positional(tail, &["--intent", "--body", "--kind", "--reply-to"])?.to_owned(),
            body: option(tail, "--body"),
        }),
        ("browser" | "file" | "diff", Some("open")) => Some(HideCall::View {
            view: match verb.as_str() {
                "browser" => ViewKind::Browser,
                "file" => ViewKind::File,
                _ => ViewKind::Diff,
            },
            target: positional(tail, &[])?.to_owned(),
        }),
        ("mission", Some(_)) => Some(HideCall::Mission {
            mission: option(tail, "--mission"),
        }),
        _ => None,
    }
}

/// The value of `--flag value` or `--flag=value`.
fn option(args: &[String], flag: &str) -> Option<String> {
    args.iter().enumerate().find_map(|(at, word)| {
        if word == flag {
            args.get(at + 1).cloned()
        } else {
            word.strip_prefix(flag)?
                .strip_prefix('=')
                .map(str::to_owned)
        }
    })
}

/// The first word that is neither a flag nor the value of one of the
/// `valued` flags.
fn positional<'a>(args: &'a [String], valued: &[&str]) -> Option<&'a str> {
    let mut skip = false;
    for word in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if word.starts_with("--") {
            skip = valued.contains(&word.as_str());
            continue;
        }
        return Some(word);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(to: &str, body: Option<&str>) -> HideCall {
        HideCall::Letter {
            to: to.to_owned(),
            body: body.map(str::to_owned),
        }
    }

    #[test]
    fn each_hide_command_a_thread_draws_is_read_as_its_part() {
        let cases = [
            (
                "hide plan set --json '{\"nodes\":[]}'",
                Some(HideCall::Plan),
            ),
            ("hide plan node core --status done", Some(HideCall::Plan)),
            ("hide plan show", None),
            (
                "hide agent spawn --name review --intent m:review --kind claude --repo . --branch main",
                Some(HideCall::Spawn {
                    name: Some("review".to_owned()),
                    intent: Some("m:review".to_owned()),
                    agent_id: None,
                }),
            ),
            (
                "hide request send agent-7 --kind report --intent m:a --body \"끝났어요\"",
                Some(letter("agent-7", Some("끝났어요"))),
            ),
            (
                "hide browser open --wait http://localhost:5173",
                Some(HideCall::View {
                    view: ViewKind::Browser,
                    target: "http://localhost:5173".to_owned(),
                }),
            ),
            (
                "hide file open docs/mock.png",
                Some(HideCall::View {
                    view: ViewKind::File,
                    target: "docs/mock.png".to_owned(),
                }),
            ),
            (
                "hide mission record threads c1 --pass --evidence ok",
                Some(HideCall::Mission { mission: None }),
            ),
            (
                "hide mission show --mission mission-20",
                Some(HideCall::Mission {
                    mission: Some("mission-20".to_owned()),
                }),
            ),
            ("hide inbox", None),
            ("hidden plan set", None),
            ("echo hide plan set", None),
        ];
        for (command, expected) in cases {
            assert_eq!(hide_call(command), expected, "{command}");
        }
    }

    /// Hide's guidance tells an agent to run the bundled `hide` by its path
    /// with its capability before it; a compound line's first `hide` counts.
    #[test]
    fn a_hide_command_is_found_behind_its_capability_path_and_in_a_list() {
        assert_eq!(
            hide_call(
                "HIDE_CAP_REF='/Users/me/.hide/state/pane-capabilities/a.json' '/Applications/hide.app/Contents/Resources/hide' request send agent-18238 --kind progress --intent m:t --body \"계획\""
            ),
            Some(letter("agent-18238", Some("계획")))
        );
        assert_eq!(
            hide_call("cd app && 'C:\\Hide\\hide.exe' plan approve; ls"),
            Some(HideCall::Plan)
        );
        assert_eq!(
            hide_call("hide request send --body=hi --intent k agent-3"),
            Some(letter("agent-3", Some("hi")))
        );
    }

    #[test]
    fn a_spawns_answer_names_the_agent_it_made() {
        let output = "{\"type\":\"workspace_result\",\"ok\":true,\"result\":{\"id\":\"agent-42\",\"name\":\"review\"}}\n";
        assert_eq!(spawned_agent(output).as_deref(), Some("agent-42"));
        assert_eq!(
            spawned_agent("{\"ok\":true,\"result\":{\"id\":\"letter-1\"}}"),
            None
        );
        assert_eq!(spawned_agent("spawn refused: mission_brief_missing"), None);
    }
}
