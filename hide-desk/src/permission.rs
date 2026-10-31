//! What Hide answers when the Desk agent asks to run something (PRD Desk
//! D-82, D-83, D-60).
//!
//! The agent always runs in its ask mode, so what it runs reaches Hide as
//! `session/request_permission` ([`read_request`]). Hide sorts the action
//! into a row of Settings › Desk › 권한 and names what it plainly does that
//! cannot be undone ([`classify`]); the core answers it from the operator's
//! [`Table`], what the operator allowed for the rest of the conversation and
//! its own facts about sessions and projects ([`decide`]). A row set to 바로
//! runs, a row set to 묻기 and anything irreversible wait for the
//! operator's card, and an action Hide cannot read does not run at all
//! ([`readable`]). No answer ever picks an option that would let the agent
//! stop asking: an allowance lives in Hide, never in the agent.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
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
    /// A change to these files; `delete` when the change removes them.
    Edit {
        paths: Vec<String>,
        delete: bool,
        reason: Option<String>,
    },
    /// Reading or searching files: what is read, as the agent names it.
    Read { what: String },
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

/// The action Hide may answer, or `None` when it does not run at all
/// (D-60): Hide cannot tell what would run, or the request offers no
/// allowance for this one call.
pub fn readable(request: &Request) -> Option<&Action> {
    request.allow_once()?;
    request.action.as_ref()
}

/// A row of Settings › Desk › 권한 (D-82).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Row {
    /// Any `hide` command, alone or with commands that only read
    /// (`| jq`, `| head`, `echo`, `2>&1`, `;`, `&&`).
    Hide,
    /// Reading and searching files, by the agent's own tools or by a shell
    /// line of commands that only read.
    Read,
    /// Any other shell command, and anything else the agent asks to run.
    Shell,
    /// The agent's own changes to files.
    Edit,
}

/// What the operator chose for a row: run without a card, or ask.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    #[default]
    Run,
    Ask,
}

/// Settings › Desk › 권한: one answer per row, every row 바로 until the
/// operator changes it. The same table holds for every agent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Table {
    pub hide: Answer,
    pub read: Answer,
    pub shell: Answer,
    pub edit: Answer,
}

impl Table {
    pub fn answer(&self, row: Row) -> Answer {
        match row {
            Row::Hide => self.hide,
            Row::Read => self.read,
            Row::Shell => self.shell,
            Row::Edit => self.edit,
        }
    }
}

/// What always asks, whatever the table says (D-82's locked row).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Irreversible {
    /// Archiving or closing a session that works now, or one Hide cannot
    /// name: its work stops.
    EndWorkingSession,
    /// A `git push` that overwrites what the remote holds.
    ForcePush,
    /// Deleting a path outside every project Hide knows, the project's own
    /// folder included, or a path Hide cannot name.
    DeleteOutside,
    /// A command whose program Hide cannot read, so it could be any of the
    /// above.
    Unreadable,
}

impl Irreversible {
    pub const ALL: [Irreversible; 4] = [
        Irreversible::EndWorkingSession,
        Irreversible::ForcePush,
        Irreversible::DeleteOutside,
        Irreversible::Unreadable,
    ];
}

/// What a card asks and what `이 대화에서 계속 허용` keeps allowed: a row set
/// to 묻기, or one kind of irreversible action (D-83).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Ask {
    Irreversible(Irreversible),
    Row(Row),
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

/// An action as Hide reads it, before the core's facts and the operator's
/// table answer it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reading {
    pub row: Row,
    pub card: Card,
    /// The session registrations it archives or closes; `None` for one
    /// Hide cannot name.
    pub ends: Vec<Option<String>>,
    /// The paths it deletes, in wire spelling; `None` for one Hide cannot
    /// name.
    pub deletes: Vec<Option<String>>,
    pub force_push: bool,
    pub unreadable: bool,
}

/// What only the core knows about the things an action names.
pub trait Facts {
    /// Whether the session registration `agent` works now. One the core
    /// cannot find does not: its own archive refuses what it cannot find.
    fn working(&self, agent: &str) -> bool;
    /// Whether `path` (wire spelling) lies below the folder of a project
    /// Hide knows.
    fn in_project(&self, path: &str) -> bool;
}

/// How a request is answered.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Decision {
    /// What still waits for the operator, irreversible first; empty when it
    /// runs.
    pub asks: Vec<Ask>,
    /// The working sessions it ends, which an allowance lets the core
    /// archive without its own confirmation.
    pub sessions: Vec<String>,
    /// The first path outside every project it deletes.
    pub outside: Option<String>,
}

/// Answers a reading from the table, the asks the operator allowed for the
/// rest of the conversation, and the core's facts.
pub fn decide(
    reading: &Reading,
    table: &Table,
    facts: &dyn Facts,
    allowed: &BTreeSet<Ask>,
) -> Decision {
    let sessions: Vec<String> = reading
        .ends
        .iter()
        .flatten()
        .filter(|agent| facts.working(agent))
        .cloned()
        .collect();
    let unnamed_end = reading.ends.iter().any(Option::is_none);
    let mut outside_unnamed = false;
    let mut outside = None;
    for path in &reading.deletes {
        match path {
            None => outside_unnamed = true,
            Some(path) if !facts.in_project(path) => {
                outside.get_or_insert_with(|| path.clone());
            }
            Some(_) => {}
        }
    }
    let mut asks: Vec<Ask> = Irreversible::ALL
        .into_iter()
        .filter(|irreversible| match irreversible {
            Irreversible::EndWorkingSession => !sessions.is_empty() || unnamed_end,
            Irreversible::ForcePush => reading.force_push,
            Irreversible::DeleteOutside => outside.is_some() || outside_unnamed,
            Irreversible::Unreadable => reading.unreadable,
        })
        .map(Ask::Irreversible)
        .collect();
    if table.answer(reading.row) == Answer::Ask {
        asks.push(Ask::Row(reading.row));
    }
    asks.retain(|ask| !allowed.contains(ask));
    Decision {
        asks,
        sessions,
        outside,
    }
}

/// Sorts an action into its row and names what it does that cannot be
/// undone.
pub fn classify(action: &Action, place: &Place<'_>) -> Reading {
    match action {
        Action::Command {
            command,
            folder,
            reason,
        } => {
            let cwd = folder
                .as_deref()
                .map(wire_word)
                .unwrap_or_else(|| place.cwd.to_owned());
            let mut line = Line::default();
            read_line(command, Some(cwd), place, 0, &mut line);
            Reading {
                row: if line.other {
                    Row::Shell
                } else if line.hide {
                    Row::Hide
                } else {
                    Row::Read
                },
                card: Card {
                    kind: CardKind::Command,
                    command: command.clone(),
                    folder: folder.clone(),
                    reason: reason.clone(),
                },
                ends: line.ends,
                deletes: line.deletes,
                force_push: line.force_push,
                unreadable: line.unreadable,
            }
        }
        Action::Edit {
            paths,
            delete,
            reason,
        } => Reading {
            row: Row::Edit,
            card: Card {
                kind: CardKind::Edit,
                command: paths.join("\n"),
                folder: None,
                reason: reason.clone(),
            },
            ends: Vec::new(),
            deletes: if *delete {
                paths
                    .iter()
                    .map(|path| resolve(path, Some(place.cwd), place.home))
                    .collect()
            } else {
                Vec::new()
            },
            force_push: false,
            unreadable: false,
        },
        Action::Read { what } => Reading {
            row: Row::Read,
            card: Card {
                kind: CardKind::Read,
                command: what.clone(),
                folder: None,
                reason: None,
            },
            ends: Vec::new(),
            deletes: Vec::new(),
            force_push: false,
            unreadable: false,
        },
        Action::Other { title, reason } => Reading {
            row: Row::Shell,
            card: Card {
                kind: CardKind::Other,
                command: title.clone(),
                folder: None,
                reason: reason.clone(),
            },
            ends: Vec::new(),
            deletes: Vec::new(),
            force_push: false,
            unreadable: false,
        },
    }
}

/// What a shell line runs, as far as Hide reads it.
#[derive(Default)]
struct Line {
    /// A `hide` command runs.
    hide: bool,
    /// A command runs that neither is `hide` nor only reads, or the line
    /// writes a file.
    other: bool,
    ends: Vec<Option<String>>,
    deletes: Vec<Option<String>>,
    force_push: bool,
    unreadable: bool,
}

/// How deep a command inside a command (`$(...)`, `bash -c`, `eval`) is
/// read; one deeper cannot be read.
const MAX_DEPTH: usize = 4;

/// A redirection to one of these writes no file.
const SAFE_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr"];

/// Words that open or close a shell body before a command's program.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "do", "while", "until", "!", "time", "{", "}", "fi", "done",
    "esac",
];

/// Words that start a line naming no program to run.
const CONTROL: &[&str] = &["for", "select", "case", "function"];

/// Programs that only read, each with the flags that would make it write
/// a file or run another program, and how many words it may name besides
/// its flags when more would name a file it writes.
struct ReadOnly {
    name: &'static str,
    refused: &'static [&'static str],
    names: Option<usize>,
}

const fn read_only(name: &'static str) -> ReadOnly {
    ReadOnly {
        name,
        refused: &[],
        names: None,
    }
}

const READ_ONLY: &[ReadOnly] = &[
    read_only("cat"),
    read_only("head"),
    read_only("tail"),
    read_only("less"),
    read_only("more"),
    read_only("ls"),
    read_only("pwd"),
    read_only("echo"),
    read_only("printf"),
    read_only("true"),
    read_only("false"),
    read_only("test"),
    read_only("["),
    read_only("[["),
    read_only("wc"),
    read_only("cut"),
    read_only("tr"),
    read_only("nl"),
    read_only("jq"),
    read_only("grep"),
    read_only("egrep"),
    read_only("fgrep"),
    read_only("ag"),
    read_only("stat"),
    read_only("file"),
    read_only("du"),
    read_only("df"),
    read_only("which"),
    read_only("type"),
    read_only("whoami"),
    read_only("id"),
    read_only("basename"),
    read_only("dirname"),
    read_only("realpath"),
    read_only("readlink"),
    read_only("column"),
    read_only("diff"),
    read_only("cmp"),
    read_only("comm"),
    read_only("paste"),
    read_only("fold"),
    read_only("sha256sum"),
    read_only("sha1sum"),
    read_only("shasum"),
    read_only("md5"),
    read_only("md5sum"),
    read_only("cksum"),
    read_only("od"),
    read_only("hexdump"),
    read_only("strings"),
    read_only("sleep"),
    read_only("seq"),
    read_only("tac"),
    read_only("rev"),
    read_only("printenv"),
    read_only("uname"),
    read_only("ps"),
    ReadOnly {
        name: "sort",
        refused: &["-o", "--output"],
        names: None,
    },
    ReadOnly {
        name: "uniq",
        refused: &[],
        names: Some(1),
    },
    ReadOnly {
        name: "rg",
        refused: &["--pre"],
        names: None,
    },
    ReadOnly {
        name: "fd",
        refused: &["-x", "-X", "--exec", "--exec-batch"],
        names: None,
    },
    ReadOnly {
        name: "base64",
        refused: &["-o", "--output"],
        names: None,
    },
    ReadOnly {
        name: "tree",
        refused: &["-o"],
        names: None,
    },
    ReadOnly {
        name: "date",
        refused: &["-s", "--set"],
        names: None,
    },
    ReadOnly {
        name: "hostname",
        refused: &[],
        names: Some(0),
    },
];

/// `git` subcommands that only read, and the flag that would make them
/// write a file.
const GIT_READS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "rev-parse",
    "ls-files",
    "blame",
    "describe",
    "shortlog",
    "cat-file",
    "ls-tree",
    "merge-base",
    "grep",
    "reflog",
];
const GIT_WRITES_OUTPUT: &str = "--output";
/// `git branch` flags that only list.
const GIT_BRANCH_LISTS: &[&str] = &[
    "-a",
    "-r",
    "-v",
    "-vv",
    "--all",
    "--remotes",
    "--list",
    "--show-current",
    "--verbose",
];
/// `git push` flags that overwrite what the remote holds; a short flag
/// group holding `f` and a refspec starting with `+` do too.
const FORCE_PUSH_FLAGS: &[&str] = &[
    "--force",
    "--force-with-lease",
    "--force-if-includes",
    "--mirror",
];

/// Programs that delete the paths they name.
const DELETERS: &[&str] = &["rm", "rmdir", "unlink", "shred", "srm", "trash", "trash-put"];

/// `hide` commands that end a session registration, which the word after
/// them names.
const HIDE_ENDS: &[&[&str]] = &[&["agent", "archive"]];
/// `herdr` commands that close panes, and with them the sessions in them.
const HERDR_CLOSES: &[&[&str]] = &[
    &["pane", "close"],
    &["tab", "close"],
    &["workspace", "close"],
    &["server", "stop"],
];

/// Programs that run the command after their own options, the options of
/// theirs that take a value, and whether they themselves are more than a
/// read.
struct Wrapper {
    name: &'static str,
    valued: &'static [&'static str],
    acts: bool,
}

const WRAPPERS: &[Wrapper] = &[
    Wrapper {
        name: "sudo",
        valued: &["-u", "-g", "-h", "-p", "-C", "-D", "-r", "-t", "-U", "-T"],
        acts: true,
    },
    Wrapper {
        name: "doas",
        valued: &["-u", "-C"],
        acts: true,
    },
    Wrapper {
        name: "env",
        valued: &["-u", "-C", "-P"],
        acts: false,
    },
    Wrapper {
        name: "command",
        valued: &[],
        acts: false,
    },
    Wrapper {
        name: "builtin",
        valued: &[],
        acts: false,
    },
    Wrapper {
        name: "exec",
        valued: &["-a"],
        acts: false,
    },
    Wrapper {
        name: "nohup",
        valued: &[],
        acts: true,
    },
    Wrapper {
        name: "time",
        valued: &["-f"],
        acts: false,
    },
    Wrapper {
        name: "nice",
        valued: &["-n"],
        acts: false,
    },
    Wrapper {
        name: "timeout",
        valued: &["-s", "-k", "--signal", "--kill-after"],
        acts: false,
    },
    Wrapper {
        name: "stdbuf",
        valued: &["-i", "-o", "-e"],
        acts: false,
    },
    Wrapper {
        name: "xargs",
        valued: &["-I", "-L", "-n", "-P", "-s", "-d", "-E", "-a", "-J", "-R", "-S"],
        acts: false,
    },
];

/// Shells whose `-c` runs the script after it.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh", "mksh", "fish"];

fn read_line(text: &str, cwd: Option<String>, place: &Place<'_>, depth: usize, line: &mut Line) {
    if depth > MAX_DEPTH {
        line.other = true;
        line.unreadable = true;
        return;
    }
    let parsed = hide_shell::shell_line(text);
    if parsed.commands.is_empty() && parsed.substitutions.is_empty() {
        line.other = true;
        // A quote the text ended inside makes the shell run none of it;
        // anything but blank lines and comments is read as unknown.
        line.unreadable |= text
            .lines()
            .any(|row| !row.trim().is_empty() && !row.trim_start().starts_with('#'));
        return;
    }
    if parsed
        .targets
        .iter()
        .any(|(_, target)| !SAFE_TARGETS.contains(&target.as_str()))
    {
        line.other = true;
    }
    let mut cwd = cwd;
    for words in &parsed.commands {
        read_command(words, &mut cwd, place, depth, line);
    }
    for inner in parsed
        .substitutions
        .iter()
        .map(|(_, text)| text)
        .chain(&parsed.backquoted)
    {
        read_line(inner, cwd.clone(), place, depth + 1, line);
    }
}

fn read_command(
    words: &[String],
    cwd: &mut Option<String>,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) {
    let mut words = words;
    while let Some((first, rest)) = words.split_first()
        && KEYWORDS.contains(&first.as_str())
    {
        words = rest;
    }
    let Some(first) = words.first() else {
        return;
    };
    if CONTROL.contains(&first.as_str()) {
        return;
    }
    let mut assigned = false;
    while let Some((first, rest)) = words.split_first()
        && hide_shell::is_assignment(first)
    {
        assigned = true;
        words = rest;
    }
    if words.is_empty() {
        return;
    }
    run_program(words, assigned, false, cwd, place, depth, line);
}

/// One program and its words. `assigned` when variables were set for it,
/// which could change what a bare name runs; `stdin_names` when more words
/// come from its input (`xargs`).
fn run_program(
    words: &[String],
    assigned: bool,
    stdin_names: bool,
    cwd: &mut Option<String>,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) {
    let Some((program, args)) = words.split_first() else {
        return;
    };
    if program.contains(['$', '`', '*', '?']) {
        line.other = true;
        line.unreadable = true;
        return;
    }
    if names_hide(program, place.hide) {
        if assigned {
            line.other = true;
        } else {
            line.hide = true;
        }
        for ends in HIDE_ENDS {
            if args.len() > ends.len() && args.iter().zip(ends.iter()).all(|(word, want)| word == want) {
                line.ends.push(
                    args[ends.len()..]
                        .iter()
                        .find(|word| !word.starts_with('-'))
                        .cloned(),
                );
            }
        }
        return;
    }
    let name = program_name(program);
    if let Some(wrapper) = WRAPPERS.iter().find(|wrapper| wrapper.name == name) {
        line.other |= wrapper.acts;
        let mut at = 0;
        let mut assigned = assigned;
        while let Some(word) = args.get(at) {
            if word == "--" {
                at += 1;
                break;
            }
            if name == "env" && hide_shell::is_assignment(word) {
                assigned = true;
            } else if !word.starts_with('-') {
                break;
            } else if name == "command" && matches!(word.as_str(), "-v" | "-V") {
                // `command -v` names what would run and runs nothing.
                return;
            } else if name == "env" && word.starts_with("-S") {
                // `env -S` splits its value into the command line.
                line.other = true;
                line.unreadable = true;
                return;
            } else if wrapper.valued.contains(&word.as_str()) {
                at += 1;
            }
            at += 1;
        }
        if name == "timeout" {
            // The duration before the command.
            at += 1;
        }
        let inner = args.get(at..).unwrap_or_default();
        if !inner.is_empty() {
            run_program(
                inner,
                assigned,
                stdin_names || name == "xargs",
                cwd,
                place,
                depth,
                line,
            );
        }
        return;
    }
    if SHELLS.contains(&name) {
        let mut runs_script = false;
        let mut at = 0;
        while let Some(word) = args.get(at) {
            if !word.starts_with('-') || word == "--" {
                break;
            }
            runs_script |= !word.starts_with("--") && word.contains('c');
            at += 1;
        }
        match args.get(at).filter(|_| runs_script) {
            Some(script) => read_line(script, cwd.clone(), place, depth + 1, line),
            // A script file or the shell's input: a program Hide does not
            // read, like any other.
            None => line.other = true,
        }
        return;
    }
    match name {
        "eval" => read_line(&args.join(" "), cwd.clone(), place, depth + 1, line),
        "cd" | "pushd" => {
            *cwd = match args.iter().find(|word| !word.starts_with('-')) {
                Some(dir) => resolve(dir, cwd.as_deref(), place.home),
                None => place.home.map(str::to_owned),
            };
        }
        "popd" => *cwd = None,
        "git" => git(args, cwd, place, line),
        "herdr" => {
            line.other = true;
            if HERDR_CLOSES.iter().any(|close| {
                args.windows(close.len())
                    .any(|window| window.iter().zip(close.iter()).all(|(word, want)| word == want))
            }) {
                line.ends.push(None);
            }
        }
        "find" => find(args, cwd, place, depth, line),
        _ if DELETERS.contains(&name) => {
            line.other = true;
            delete_names(args, cwd.as_deref(), place, line);
            if stdin_names {
                line.deletes.push(None);
            }
        }
        _ => {
            let reads = READ_ONLY.iter().find(|entry| entry.name == name).is_some_and(|entry| {
                !args.iter().any(|word| refused(word, entry.refused))
                    && entry
                        .names
                        .is_none_or(|most| args.iter().filter(|word| !word.starts_with('-')).count() <= most)
            });
            line.other |= !reads;
        }
    }
}

fn names_hide(program: &str, hide: &Path) -> bool {
    program == "hide" || (cfg!(windows) && program == "hide.exe") || Path::new(program) == hide
}

/// A program's own name, without its folder (and `.exe` on Windows).
fn program_name(program: &str) -> &str {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    if cfg!(windows) {
        name.strip_suffix(".exe").unwrap_or(name)
    } else {
        name
    }
}

/// Whether `word` is one of `flags`: a long flag alone or with `=value`, a
/// short flag alone or in a group of short flags.
fn refused(word: &str, flags: &[&str]) -> bool {
    flags.iter().any(|flag| {
        if flag.starts_with("--") {
            word == *flag || word.strip_prefix(flag).is_some_and(|rest| rest.starts_with('='))
        } else {
            let letter = flag.trim_start_matches('-');
            word.starts_with('-') && !word.starts_with("--") && word[1..].contains(letter)
        }
    })
}

fn git(args: &[String], cwd: &mut Option<String>, place: &Place<'_>, line: &mut Line) {
    let mut cwd = cwd.clone();
    let mut at = 0;
    while let Some(word) = args.get(at) {
        match word.as_str() {
            "-C" => {
                cwd = args
                    .get(at + 1)
                    .and_then(|dir| resolve(dir, cwd.as_deref(), place.home));
                at += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => at += 2,
            word if word.starts_with('-') => at += 1,
            _ => break,
        }
    }
    let Some(sub) = args.get(at) else {
        return;
    };
    let rest = &args[at + 1..];
    match sub.as_str() {
        "push" => {
            line.other = true;
            line.force_push |= rest.iter().any(|word| {
                FORCE_PUSH_FLAGS
                    .iter()
                    .any(|flag| word == flag || word.strip_prefix(flag).is_some_and(|tail| tail.starts_with('=')))
                    || (word.starts_with('-') && !word.starts_with("--") && word.contains('f'))
                    || (word.starts_with('+') && word.len() > 1)
            });
        }
        "clean" => {
            line.other = true;
            if rest.iter().any(|word| word == "-n" || word == "--dry-run") {
                return;
            }
            // It deletes what lies below the folders it names, or below
            // the one it runs in.
            let names: Vec<&String> = rest.iter().filter(|word| !word.starts_with('-')).collect();
            if names.is_empty() {
                line.deletes.push(cwd.as_deref().map(below));
            } else {
                for name in names {
                    line.deletes
                        .push(resolve(name, cwd.as_deref(), place.home).map(|path| below(&path)));
                }
            }
        }
        "branch" => {
            line.other |= !rest.iter().all(|word| GIT_BRANCH_LISTS.contains(&word.as_str()));
        }
        sub if GIT_READS.contains(&sub) => {
            line.other |= rest.iter().any(|word| refused(word, &[GIT_WRITES_OUTPUT]));
        }
        _ => line.other = true,
    }
}

fn find(
    args: &[String],
    cwd: &mut Option<String>,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) {
    let roots: Vec<&String> = args
        .iter()
        .take_while(|word| !word.starts_with('-') && *word != "(" && *word != "!")
        .collect();
    let roots: Vec<String> = if roots.is_empty() {
        vec![".".to_owned()]
    } else {
        roots.into_iter().cloned().collect()
    };
    let mut at = 0;
    while let Some(word) = args.get(at) {
        match word.as_str() {
            "-delete" => {
                line.other = true;
                for root in &roots {
                    line.deletes
                        .push(resolve(root, cwd.as_deref(), place.home).map(|path| below(&path)));
                }
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                let end = args[at + 1..]
                    .iter()
                    .position(|word| word == ";" || word == "+")
                    .map_or(args.len(), |offset| at + 1 + offset);
                let inner = &args[at + 1..end];
                for root in &roots {
                    // What `find` hands the command lies below its root.
                    let found = format!("{}/{{}}", root.trim_end_matches('/'));
                    let words: Vec<String> = inner
                        .iter()
                        .map(|word| if word == "{}" { found.clone() } else { word.clone() })
                        .collect();
                    run_program(&words, false, false, &mut cwd.clone(), place, depth, line);
                }
                at = end;
            }
            "-fprint" | "-fprint0" | "-fprintf" | "-fls" => line.other = true,
            _ => {}
        }
        at += 1;
    }
}

fn delete_names(args: &[String], cwd: Option<&str>, place: &Place<'_>, line: &mut Line) {
    let mut options = true;
    for word in args {
        if options && word == "--" {
            options = false;
            continue;
        }
        if options && word.starts_with('-') && word.len() > 1 {
            continue;
        }
        line.deletes.push(resolve(word, cwd, place.home));
    }
}

/// A path standing for what lies below `folder`, which a command that
/// deletes inside a folder deletes, not the folder itself.
fn below(folder: &str) -> String {
    format!("{}/{{}}", folder.trim_end_matches('/'))
}

/// A word a command names as a path, in wire spelling: absolute, from
/// `~`, or from the folder the command runs in. `None` when Hide cannot
/// tell: a variable, a substitution, another account's `~`, or a relative
/// path in a folder it cannot tell.
fn resolve(word: &str, cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    if word.contains(['$', '`']) {
        return None;
    }
    let word = wire_word(word);
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
    Some(normalize(&joined))
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
                && let Some(path) = raw_path()
            {
                paths.push(path);
            }
            (!paths.is_empty()).then_some(Action::Edit {
                paths,
                delete: kind == "delete",
                reason,
            })
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
        Some(Action::Read { .. }) => Step::Files,
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
        "read" | "grep" | "find" | "ls" => Some(Action::Read {
            what: text(input.get("path"))
                .or_else(|| text(input.get("pattern")))
                .unwrap_or(tool),
        }),
        "bash" => Some(Action::Command {
            command: text(input.get("command"))?,
            folder: text(input.get("cwd")),
            reason: None,
        }),
        "edit" | "write" => Some(Action::Edit {
            paths: vec![text(input.get("path"))?],
            delete: false,
            reason: None,
        }),
        _ => Some(Action::Other {
            title: format!("{tool}\n{input}"),
            reason: None,
        }),
    }
}

#[cfg(test)]
mod tests;
