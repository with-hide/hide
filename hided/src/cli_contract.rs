//! The contract of the commands other tools call (`hide agent`, `request`,
//! `inbox`, `watch`): each command's words, arguments and options, where its
//! answer sits in the line it prints, and the JSON Schema of that answer.
//!
//! The table below is what the CLI admits: a verb or a flag of these topics
//! that the table does not name is refused before any parser reads it
//! (`admit`), so a command or option cannot reach a caller without reaching
//! the contract, and the tests below hold each parser to what the table says
//! it takes. The answer schemas come from the core's answer types
//! (`herdr_core::delivery::answer`). `hide contract --json` prints the
//! document and `hide version --json` its digest, and `contracts/hide-cli.json`
//! is the committed copy a review sees change.

use herdr_core::delivery::{BODY_LIMIT, HOOK_LETTERS};
use serde_json::{Map, Value, json};

/// The document's own format; a caller that reads another refuses it.
pub const FORMAT: u32 = 1;

/// The topics whose every command is in the table.
const TOPICS: &[&str] = &["agent", "request", "inbox", "watch"];

/// What a value must be for the command's parser to take it.
#[derive(Clone, Copy)]
pub enum Input {
    /// Non-empty, without control characters, not starting with `--`.
    Text,
    /// An id, name or intent: non-empty, at most 256 bytes, without control
    /// characters, not starting with `-`.
    Key,
    /// A native session id or session file path: non-empty, at most
    /// `herdr_core::delivery::SESSION_LIMIT` bytes, without control
    /// characters, not starting with `-`.
    Session,
    /// A letter body: not blank, at most `BODY_LIMIT` bytes.
    Body,
    /// A SHA-256 as 64 lowercase hexadecimal digits.
    Sha256,
    /// A watch approval: not blank, at most 256 bytes, without control
    /// characters.
    Approval,
    /// A decimal integer from 0 to 2^64 - 1.
    Unsigned,
    OneOf(&'static [&'static str]),
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Text => "text",
            Key => "key",
            Session => "session",
            Body => "body",
            Sha256 => "sha256",
            Approval => "approval",
            Unsigned => "unsigned",
            OneOf(_) => "one_of",
        }
    }
}

pub struct Arg {
    pub name: &'static str,
    pub value: Input,
}

pub struct Opt {
    pub name: &'static str,
    /// None for a switch.
    pub value: Option<Input>,
    pub required: bool,
    /// Options this one is refused without.
    pub requires: &'static [&'static str],
}

pub struct Spec {
    /// The words after `hide`, mode flags such as `inbox --hook` included.
    pub words: &'static [&'static str],
    /// Positional arguments, in order.
    pub arguments: &'static [Arg],
    /// When the last argument repeats, at most how many times.
    pub repeats: Option<usize>,
    pub options: &'static [Opt],
    /// What `--` passes through unread, if the command takes it.
    pub rest: Option<&'static str>,
    /// The answer schemas (`answers`) the command can return.
    pub answers: &'static [&'static str],
    /// The failure codes the contract declares for the command's own rule,
    /// beside those every command of its topic can answer (the daemon, the
    /// pane, the ledger). Not exhaustive; written only where declared.
    pub refusals: &'static [&'static str],
}

const fn required(name: &'static str, value: Input) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: true,
        requires: &[],
    }
}

const fn optional(name: &'static str, value: Input) -> Opt {
    Opt {
        name,
        value: Some(value),
        required: false,
        requires: &[],
    }
}

const fn switch(name: &'static str) -> Opt {
    Opt {
        name,
        value: None,
        required: false,
        requires: &[],
    }
}

const fn spec(
    words: &'static [&'static str],
    arguments: &'static [Arg],
    options: &'static [Opt],
    answers: &'static [&'static str],
) -> Spec {
    Spec {
        words,
        arguments,
        repeats: None,
        options,
        rest: None,
        answers,
        refusals: &[],
    }
}

use Input::{Approval, Body, Key, OneOf, Session, Sha256, Text, Unsigned};

const ID: &[Arg] = &[Arg {
    name: "id",
    value: Key,
}];
const TARGET: &[Arg] = &[Arg {
    name: "target",
    value: Key,
}];
const AGENT_ID: &[Arg] = &[Arg {
    name: "id",
    value: Text,
}];

pub const COMMANDS: &[Spec] = &[
    spec(
        &["agent", "register"],
        &[],
        &[
            switch("--check"),
            optional("--machine", Text),
            required("--host-scope", Text),
            required("--session", Session),
            required("--instance", Text),
            required("--name", Text),
            required("--pane", Text),
            optional("--parent", Text),
            optional("--project", Text),
            switch("--json"),
        ],
        &["agent", "agent_register_check"],
    ),
    spec(
        &["agent", "list"],
        &[],
        &[switch("--json")],
        &["agent_list"],
    ),
    spec(
        &["agent", "show"],
        AGENT_ID,
        &[switch("--json")],
        &["agent"],
    ),
    // The caller's own registration, by its attested pane, device and
    // session; only a pane-bound credential can ask.
    Spec {
        words: &["agent", "show", "here"],
        arguments: &[],
        repeats: None,
        options: &[switch("--json")],
        rest: None,
        answers: &["agent"],
        refusals: &[
            "agent_pane_required",
            "caller_identity_conflict",
            "participant_unavailable",
            "participant_ended",
            "participant_session_changed",
            "ambiguous_participant",
        ],
    },
    spec(
        &["agent", "end"],
        AGENT_ID,
        &[optional("--actor", Text), switch("--json")],
        &["agent"],
    ),
    Spec {
        words: &["agent", "spawn"],
        arguments: &[],
        repeats: None,
        options: &[
            optional("--parent", Text),
            optional("--machine", Text),
            required("--name", Text),
            required("--intent", Text),
            required("--kind", Text),
            required("--repo", Text),
            required("--branch", Text),
            optional("--path", Text),
            optional("--model", Text),
            optional("--effort", Text),
            switch("--help"),
            switch("--json"),
        ],
        rest: Some("the agent's own arguments"),
        answers: &["agent"],
        refusals: &[
            "spawn_busy",
            "machine_unknown",
            "machine_not_permitted",
            "machine_unavailable",
            "repository_unavailable",
            "agent_not_installed",
            "intent_conflict",
            "invalid_launch_choice",
            "shell_starting",
            "agent_not_started",
            "worktree_unconfirmed",
            "native_identity_unavailable",
            "identity_waits_full",
            "session_too_long",
        ],
    },
    spec(
        &["request", "send"],
        TARGET,
        &[
            required("--intent", Key),
            required("--body", Body),
            optional("--kind", OneOf(&["request", "block", "report", "progress"])),
            switch("--now"),
        ],
        &["letter"],
    ),
    spec(
        &["request", "reply"],
        ID,
        &[required("--intent", Key), required("--body", Body)],
        &["letter"],
    ),
    spec(&["request", "ack"], ID, &[], &["letter"]),
    spec(&["request", "cancel"], ID, &[], &["letter"]),
    spec(&["request", "show"], ID, &[], &["letter"]),
    spec(&["inbox"], &[], &[], &["inbox"]),
    spec(
        &["inbox", "--hook"],
        &[],
        &[
            optional("--prompt-digest", Sha256),
            optional("--session", Session),
            Opt {
                name: "--tool",
                value: None,
                required: false,
                requires: &["--session"],
            },
        ],
        &["intake"],
    ),
    Spec {
        words: &["inbox", "--confirm"],
        arguments: ID,
        repeats: Some(HOOK_LETTERS),
        options: &[],
        rest: None,
        answers: &["confirmed"],
        refusals: &[],
    },
    spec(
        &["watch", "start"],
        TARGET,
        &[optional("--observer", Key), optional("--actor", Key)],
        &["watch"],
    ),
    spec(
        &["watch", "assign"],
        ID,
        &[
            required("--observer", Key),
            optional("--actor", Key),
            optional("--expected-generation", Unsigned),
            Opt {
                name: "--approval",
                value: Some(Approval),
                required: false,
                requires: &["--expected-generation"],
            },
        ],
        &["watch"],
    ),
    spec(&["watch", "stop"], ID, &[], &["stopped"]),
    spec(&["watch", "list"], &[], &[], &["watch_list"]),
];

/// The command `args` (the words after `hide`) name, by its longest words.
fn find(args: &[String]) -> Option<&'static Spec> {
    COMMANDS
        .iter()
        .filter(|spec| {
            spec.words.len() <= args.len()
                && spec.words.iter().zip(args).all(|(word, arg)| word == arg)
        })
        .max_by_key(|spec| spec.words.len())
}

/// Refuses a command of the contract's topics that the table does not name,
/// and a flag the table does not give the command, before its parser reads
/// either. The parser still checks the values.
pub fn admit(args: &[String]) -> Result<(), String> {
    let Some(spec) = find(args) else {
        return match args.first() {
            Some(topic) if TOPICS.contains(&topic.as_str()) => Err(format!(
                "unknown command: hide {}",
                args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
            )),
            _ => Ok(()),
        };
    };
    let mut rest = args[spec.words.len()..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--" && spec.rest.is_some() {
            break;
        }
        if !arg.starts_with("--") {
            continue;
        }
        let option = spec
            .options
            .iter()
            .find(|option| option.name == arg)
            .ok_or_else(|| format!("Unknown flag for hide {}: {arg}", spec.words.join(" ")))?;
        if option.value.is_some() {
            rest.next();
        }
    }
    Ok(())
}

fn value_json(value: Input) -> Value {
    match value {
        OneOf(values) => json!({"type": value.name(), "values": values}),
        _ => json!({"type": value.name()}),
    }
}

fn command_json(spec: &Spec) -> Value {
    let mut command = json!({
        "command": spec.words.join(" "),
        "arguments": spec.arguments.iter().map(|argument| json!({
            "name": argument.name,
            "value": value_json(argument.value),
        })).collect::<Vec<_>>(),
        "repeats_at_most": spec.repeats,
        "options": spec.options.iter().map(|option| json!({
            "name": option.name,
            "value": option.value.map(value_json),
            "required": option.required,
            "requires": option.requires,
        })).collect::<Vec<_>>(),
        "rest": spec.rest,
        "answers": spec.answers,
    });
    if !spec.refusals.is_empty() {
        command["refusals"] = json!(spec.refusals);
    }
    command
}

/// The contract without its digest.
fn body() -> Value {
    json!({
        "format": FORMAT,
        "commands": COMMANDS.iter().map(command_json).collect::<Vec<_>>(),
        "value_types": {
            "text": "non-empty, no control characters, does not start with --",
            "key": "non-empty, at most 256 bytes, no control characters, does not start with -",
            "session": format!(
                "non-empty, at most {} bytes, no control characters, does not start with -",
                herdr_core::delivery::SESSION_LIMIT
            ),
            "body": format!("not blank, at most {BODY_LIMIT} bytes"),
            "approval": "not blank, at most 256 bytes, no control characters",
            "unsigned": "a decimal integer from 0 to 18446744073709551615",
            "one_of": "one of the listed values",
        },
        // Where each topic's printed line carries the answer and, when the
        // command fails (a non-zero exit), the failure's code.
        "envelopes": {
            "agent": {"ok": "ok", "answer": "value", "code": "error.code"},
            "request": {"ok": "ok", "answer": "result", "code": "reason"},
            "inbox": {"ok": "ok", "answer": "result", "code": "reason"},
            "watch": {"ok": "ok", "answer": "result", "code": "reason"},
        },
        "answers": herdr_core::delivery::answer::answer_schemas(),
    })
}

/// `value` with every object's keys in order, so the digest does not depend
/// on whether a build orders JSON maps by insertion.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonical(value)))
                    .collect::<Map<_, _>>(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// `sha256:<hex>` of the canonical contract body.
pub fn digest() -> String {
    digest_of(&canonical(body()))
}

fn digest_of(body: &Value) -> String {
    let bytes = serde_json::to_vec(body).expect("a JSON value serializes");
    format!(
        "sha256:{}",
        hex::encode(ring::digest::digest(&ring::digest::SHA256, &bytes))
    )
}

/// The document `hide contract --json` prints: the body and its digest.
pub fn document() -> Value {
    let body = canonical(body());
    let digest = digest_of(&body);
    let Value::Object(mut map) = body else {
        unreachable!("the body is an object")
    };
    map.insert("digest".to_owned(), Value::String(digest));
    canonical(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{CommandKind, parse_args};

    /// A value the parser takes, and one the value type rules out.
    fn sample(value: Input) -> (String, String) {
        let (taken, refused) = match value {
            Text => ("some text", "--x"),
            Key => ("k1", "-x"),
            Session => ("/sessions/a.jsonl", "-x"),
            Body => ("multi\nline body", " "),
            Sha256 => (
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "BA7816BF",
            ),
            Approval => ("approved by the lead", " "),
            Unsigned => ("1", "-1"),
            OneOf(values) => (values[0], "other"),
        };
        (taken.to_owned(), refused.to_owned())
    }

    /// The command line with every argument and option the contract names,
    /// less `leave_out`, with `refuse`'s value replaced by a refused one.
    fn argv(spec: &Spec, leave_out: &[&str], refuse: Option<&str>, repeat: usize) -> Vec<String> {
        let mut argv = vec!["hide".to_owned()];
        argv.extend(spec.words.iter().map(|word| (*word).to_owned()));
        for (index, argument) in spec.arguments.iter().enumerate() {
            let (taken, refused) = sample(argument.value);
            let value = if refuse == Some(argument.name) {
                refused
            } else {
                taken
            };
            let last = index + 1 == spec.arguments.len();
            for copy in 0..if last { repeat } else { 1 } {
                argv.push(format!(
                    "{value}{}",
                    if copy == 0 {
                        String::new()
                    } else {
                        copy.to_string()
                    }
                ));
            }
        }
        for option in spec.options {
            // Help exits before delivery and has its own standalone argv.
            if option.name == "--help" || leave_out.contains(&option.name) {
                continue;
            }
            argv.push(option.name.to_owned());
            if let Some(value) = option.value {
                let (taken, refused) = sample(value);
                argv.push(if refuse == Some(option.name) {
                    refused
                } else {
                    taken
                });
            }
        }
        if spec.rest.is_some() {
            argv.extend(["--".to_owned(), "--model".to_owned()]);
        }
        argv
    }

    fn takes(argv: &[String]) -> bool {
        matches!(parse_args(argv), Ok(CommandKind::Delivery(_)))
    }

    /// Each command the contract names parses with everything it names, and
    /// refuses what the contract rules out: a required option left out, an
    /// option without the one it requires, a value outside its type, one
    /// repetition too many. A parser that drifts from the table fails here.
    #[test]
    fn each_command_takes_what_the_contract_names_and_nothing_it_rules_out() {
        for spec in COMMANDS {
            let command = spec.words.join(" ");
            if spec.options.iter().any(|option| option.name == "--help") {
                let mut help = vec!["hide".to_owned()];
                help.extend(spec.words.iter().map(|word| (*word).to_owned()));
                help.push("--help".to_owned());
                assert_eq!(parse_args(&help).unwrap(), CommandKind::AgentSpawnHelp);
            }
            let most = spec.repeats.unwrap_or(1);
            let full = argv(spec, &[], None, most);
            assert!(takes(&full), "hide {command}: {:?}", parse_args(&full));
            if spec.repeats.is_some() {
                assert!(
                    !takes(&argv(spec, &[], None, most + 1)),
                    "hide {command} repeated"
                );
            }
            for option in spec.options {
                if option.required {
                    assert!(
                        !takes(&argv(spec, &[option.name], None, most)),
                        "hide {command} without {}",
                        option.name
                    );
                }
                if !option.requires.is_empty() {
                    assert!(
                        !takes(&argv(spec, option.requires, None, most)),
                        "hide {command} {} alone",
                        option.name
                    );
                }
                if option.value.is_some() {
                    assert!(
                        !takes(&argv(spec, &[], Some(option.name), most)),
                        "hide {command} {} refused value",
                        option.name
                    );
                }
            }
            for argument in spec.arguments {
                assert!(
                    !takes(&argv(spec, &[], Some(argument.name), most)),
                    "hide {command} refused {}",
                    argument.name
                );
            }
        }
    }

    #[test]
    fn a_command_or_flag_the_contract_does_not_name_is_refused() {
        let args = |line: &[&str]| {
            line.iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        assert!(admit(&args(&["agent", "resume", "a1"])).is_err());
        assert!(admit(&args(&["watch", "pause", "w1"])).is_err());
        assert!(admit(&args(&["factory", "status"])).is_ok());
        assert!(admit(&args(&["request", "send", "t", "--wait"])).is_err());
        assert!(parse_args(&args(&["hide", "watch", "stop", "w1", "--observer", "o"])).is_err());
        // A value that looks like a flag is a value.
        assert!(admit(&args(&["request", "send", "t", "--body", "--wait"])).is_ok());
        // What `--` passes through is the agent's own.
        assert!(admit(&args(&["agent", "spawn", "--", "--model", "x"])).is_ok());
        assert!(
            admit(&args(&[
                "inbox",
                "--hook",
                "--session",
                "s",
                "--prompt-digest",
                "d"
            ]))
            .is_ok()
        );
        assert!(admit(&args(&["inbox", "--hook", "--bell"])).is_err());
        assert!(admit(&args(&["inbox", "--prompt-digest", "d"])).is_err());
    }

    /// The committed copy is the contract this build exports, so a change to
    /// a command's options or answer fields shows in review as a change to
    /// `contracts/hide-cli.json`. Regenerate it with
    /// `target/debug/hide contract --json | python3 -m json.tool > contracts/hide-cli.json`.
    #[test]
    fn the_committed_contract_is_the_exported_one() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contracts/hide-cli.json");
        let committed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            committed == document(),
            "contracts/hide-cli.json differs from the exported contract; regenerate it as this test's comment says"
        );
    }
}
