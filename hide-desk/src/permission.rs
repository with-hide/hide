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

use hide_shell::Join;
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
    /// A `git push` that deletes a branch or tag the remote holds
    /// (`<remote> :ref`, `--delete`, `-d`, `--prune`, `--mirror`).
    DeleteRemote,
    /// Deleting a path outside every project Hide knows, the project's own
    /// folder included, or a path Hide cannot name.
    DeleteOutside,
    /// Overwriting or moving away a file outside every project Hide knows,
    /// or one Hide cannot name: a redirection that writes, `cp`, `mv`,
    /// `tee`, `install`, `ln -f`, `truncate`, `dd of=`, an output flag, or
    /// an edit (D-90).
    OverwriteOutside,
    /// A command whose program Hide cannot read, so it could be any of the
    /// above.
    Unreadable,
}

impl Irreversible {
    pub const ALL: [Irreversible; 6] = [
        Irreversible::EndWorkingSession,
        Irreversible::ForcePush,
        Irreversible::DeleteRemote,
        Irreversible::DeleteOutside,
        Irreversible::OverwriteOutside,
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
    /// The paths it overwrites or moves away, the same way.
    pub overwrites: Vec<Option<String>>,
    pub force_push: bool,
    pub delete_remote: bool,
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
    /// The first path outside every project it overwrites or moves away.
    pub overwritten: Option<String>,
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
    // Whether any path lies outside every project or cannot be named, and
    // the first one outside.
    let outside_of = |paths: &[Option<String>]| {
        let mut unnamed = false;
        let mut first = None;
        for path in paths {
            match path {
                None => unnamed = true,
                Some(path) if !facts.in_project(path) => {
                    first.get_or_insert_with(|| path.clone());
                }
                Some(_) => {}
            }
        }
        (first.is_some() || unnamed, first)
    };
    let (deletes_outside, outside) = outside_of(&reading.deletes);
    let (overwrites_outside, overwritten) = outside_of(&reading.overwrites);
    let mut asks: Vec<Ask> = Irreversible::ALL
        .into_iter()
        .filter(|irreversible| match irreversible {
            Irreversible::EndWorkingSession => !sessions.is_empty() || unnamed_end,
            Irreversible::ForcePush => reading.force_push,
            Irreversible::DeleteRemote => reading.delete_remote,
            Irreversible::DeleteOutside => deletes_outside,
            Irreversible::OverwriteOutside => overwrites_outside,
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
        overwritten,
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
            read_line(command, &vec![Some(cwd)], place, 0, &mut line);
            // A line that changes the environment its programs read may
            // change which program a name runs and what it does.
            let unreadable = line.unreadable || line.environment;
            Reading {
                row: if line.other || unreadable {
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
                deletes: line.changed(Change::Delete),
                overwrites: line.changed(Change::Overwrite),
                ends: line.ends,
                force_push: line.force_push,
                delete_remote: line.delete_remote,
                unreadable,
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
            // An edit that deletes, or one that writes what was there.
            deletes: if *delete {
                paths
                    .iter()
                    .map(|path| path_of(path, Some(place.cwd), place.home))
                    .collect()
            } else {
                Vec::new()
            },
            overwrites: if *delete {
                Vec::new()
            } else {
                paths
                    .iter()
                    .map(|path| path_of(path, Some(place.cwd), place.home))
                    .collect()
            },
            force_push: false,
            delete_remote: false,
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
            overwrites: Vec::new(),
            force_push: false,
            delete_remote: false,
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
            overwrites: Vec::new(),
            force_push: false,
            delete_remote: false,
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
    /// The paths it deletes or overwrites, in the order the line reaches
    /// them.
    paths: Vec<(Change, Option<String>)>,
    force_push: bool,
    delete_remote: bool,
    unreadable: bool,
    /// The line sets a variable programs read from the environment.
    environment: bool,
    /// A command has run that may make or move a link, so a path deleted
    /// after it may lead somewhere it does not lead now.
    writes: bool,
}

impl Line {
    /// A command Hide cannot read: it could do anything.
    fn unreadable(&mut self) {
        self.other = true;
        self.unreadable = true;
        self.writes = true;
    }

    /// A path the line deletes; after a command that may make links, one
    /// Hide cannot name.
    fn delete(&mut self, path: Option<String>) {
        self.change(Change::Delete, path);
    }

    /// A path the line overwrites or moves away, named the same way.
    fn overwrite(&mut self, path: Option<String>) {
        self.change(Change::Overwrite, path);
    }

    fn change(&mut self, change: Change, path: Option<String>) {
        let path = path.filter(|_| !self.writes);
        self.paths.push((change, path));
    }

    fn changed(&self, change: Change) -> Vec<Option<String>> {
        self.paths
            .iter()
            .filter(|(kind, _)| *kind == change)
            .map(|(_, path)| path.clone())
            .collect()
    }
}

/// What a line does to a path it names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Change {
    Delete,
    Overwrite,
}

/// The folders a command may run in. `None` stands for one Hide cannot
/// name, in which a relative path cannot be named either.
type Folders = Vec<Option<String>>;

fn add(folders: &mut Folders, folder: Option<String>) {
    if !folders.contains(&folder) {
        folders.push(folder);
    }
}

fn joined(left: &Folders, right: &Folders) -> Folders {
    let mut all = left.clone();
    for folder in right {
        add(&mut all, folder.clone());
    }
    all
}

/// What a command does to the folder of the shell it runs in.
enum Move {
    Stay,
    /// It goes to one of these when it succeeds and stays when it fails.
    To(Folders),
    /// It may go anywhere.
    Unknown,
}

/// Where the shell may be after a command, by whether it succeeded.
#[derive(Clone)]
struct After {
    ok: Folders,
    failed: Folders,
}

impl After {
    /// Where a command that follows by `join` runs.
    fn runs(&self, join: Join) -> Folders {
        match join {
            Join::And => self.ok.clone(),
            Join::Or => self.failed.clone(),
            Join::List | Join::Pipe => joined(&self.ok, &self.failed),
        }
    }

    /// Where the shell may be after a command that followed by `join`, ran
    /// in `ran` and did `moved`. Past `&&` or `||`, the shell may also be
    /// where it was when the command did not run.
    fn then(&self, join: Join, ran: &Folders, moved: Move) -> After {
        let (ok, failed) = match moved {
            Move::Stay => (ran.clone(), ran.clone()),
            Move::To(to) => (to, ran.clone()),
            Move::Unknown => {
                let mut anywhere = ran.clone();
                add(&mut anywhere, None);
                (anywhere.clone(), anywhere)
            }
        };
        match join {
            Join::And => After {
                failed: joined(&failed, &self.failed),
                ok,
            },
            Join::Or => After {
                ok: joined(&self.ok, &ok),
                failed,
            },
            Join::List | Join::Pipe => After { ok, failed },
        }
    }
}

/// How deep a command inside a command (`$(...)`, `bash -c`, `eval`) is
/// read; one deeper cannot be read.
const MAX_DEPTH: usize = 4;

/// A redirection to one of these writes no file.
const SAFE_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr"];

/// Words that open or close a shell body before a command's program.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}", "fi", "done", "esac",
];

/// Words that start a line naming no program to run.
const CONTROL: &[&str] = &["for", "select", "case"];

/// Words before a command that run it in a way Hide does not follow: in
/// the background with its own pipes, timed by the shell, in place of the
/// shell, or as a function's body.
const OPAQUE: &[&str] = &["coproc", "time", "exec", "function"];

/// What a program Hide knows does besides reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Effect {
    /// It only reads, unless one of its `writes` flags is given.
    Reads,
    /// It changes files or the shell's own state and makes no link.
    Changes,
    /// It may make or move a link.
    Links,
}

/// A program Hide knows (D-89): what it does, the flags that make it
/// change something, the flags whose value names a file it writes, the
/// flags that make it run another program, how many words it may name
/// besides its flags when more would name a file it writes, which of its
/// words name files it writes or moves away (D-90), and its options that
/// take a value. Any other program is one Hide cannot read.
struct Known {
    name: &'static str,
    effect: Effect,
    writes: &'static [&'static str],
    output: &'static [&'static str],
    runs: &'static [&'static str],
    names: Option<usize>,
    written: Written,
    valued: &'static [&'static str],
    /// Flags that make a program of `Changes` make or copy links.
    links_with: &'static [&'static str],
}

/// Which words of a program name the files it writes or moves away.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Written {
    Nothing,
    /// Its last word, or the folder `-t` names (`cp`, `install`).
    Last,
    /// Every word it names (`tee`, `truncate`).
    Every,
    /// Every word, what it moves and where (`mv`).
    Moved,
    /// Its last word, or the folder `-t` names, only when it replaces what
    /// is there (`ln -f`).
    Replaced,
    /// Its second word (`uniq`'s output).
    Second,
    /// The value of each word starting with this (`dd of=`).
    Operand(&'static str),
}

const fn known(name: &'static str, effect: Effect) -> Known {
    Known {
        name,
        effect,
        writes: &[],
        output: &[],
        runs: &[],
        names: None,
        written: Written::Nothing,
        valued: &[],
        links_with: &[],
    }
}

const fn reads(name: &'static str) -> Known {
    known(name, Effect::Reads)
}

const KNOWN: &[Known] = &[
    reads("cat"),
    reads("head"),
    reads("tail"),
    reads("ls"),
    reads("pwd"),
    reads("echo"),
    reads("printf"),
    reads("true"),
    reads("false"),
    reads("test"),
    reads("["),
    reads("[["),
    reads("wc"),
    reads("cut"),
    reads("tr"),
    reads("nl"),
    reads("jq"),
    reads("grep"),
    reads("egrep"),
    reads("fgrep"),
    reads("stat"),
    reads("file"),
    reads("du"),
    reads("df"),
    reads("which"),
    reads("type"),
    reads("whoami"),
    reads("id"),
    reads("basename"),
    reads("dirname"),
    reads("realpath"),
    reads("readlink"),
    reads("column"),
    reads("diff"),
    reads("cmp"),
    reads("comm"),
    reads("paste"),
    reads("fold"),
    reads("sha256sum"),
    reads("sha1sum"),
    reads("shasum"),
    reads("md5"),
    reads("md5sum"),
    reads("cksum"),
    reads("od"),
    reads("hexdump"),
    reads("strings"),
    reads("sleep"),
    reads("seq"),
    reads("tac"),
    reads("rev"),
    reads("printenv"),
    reads("uname"),
    reads("ps"),
    // The shell's own words that reach nothing outside it.
    reads(":"),
    reads("set"),
    reads("shopt"),
    reads("exit"),
    reads("return"),
    reads("break"),
    reads("continue"),
    reads("shift"),
    reads("wait"),
    reads("jobs"),
    Known {
        output: &["-o", "--output"],
        runs: &["--compress-program"],
        valued: &[
            "-k",
            "-t",
            "-S",
            "-T",
            "--key",
            "--field-separator",
            "--buffer-size",
            "--temporary-directory",
        ],
        ..reads("sort")
    },
    Known {
        names: Some(1),
        written: Written::Second,
        valued: &[
            "-f",
            "-s",
            "-w",
            "--skip-fields",
            "--skip-chars",
            "--check-chars",
        ],
        ..reads("uniq")
    },
    Known {
        runs: &["--pre"],
        ..reads("rg")
    },
    Known {
        runs: &["--pager"],
        ..reads("ag")
    },
    Known {
        runs: &["-x", "-X", "--exec", "--exec-batch"],
        ..reads("fd")
    },
    Known {
        output: &["-o", "--output"],
        ..reads("base64")
    },
    Known {
        output: &["-o"],
        valued: &["-L", "-P", "-I", "--charset", "--filelimit"],
        ..reads("tree")
    },
    Known {
        writes: &["-s", "--set"],
        ..reads("date")
    },
    Known {
        names: Some(0),
        ..reads("hostname")
    },
    known("mkdir", Effect::Changes),
    known("touch", Effect::Changes),
    known("chmod", Effect::Changes),
    Known {
        written: Written::Every,
        ..known("tee", Effect::Changes)
    },
    Known {
        written: Written::Every,
        valued: &["-s", "-r", "--size", "--reference"],
        ..known("truncate", Effect::Changes)
    },
    Known {
        written: Written::Operand("of="),
        ..known("dd", Effect::Changes)
    },
    Known {
        written: Written::Last,
        runs: &["--strip-program"],
        valued: &[
            "-t",
            "-S",
            "-m",
            "-o",
            "-g",
            "-B",
            "-f",
            "--target-directory",
            "--suffix",
            "--mode",
            "--owner",
            "--group",
        ],
        ..known("install", Effect::Changes)
    },
    known("kill", Effect::Changes),
    known("export", Effect::Changes),
    known("declare", Effect::Changes),
    known("typeset", Effect::Changes),
    known("readonly", Effect::Changes),
    known("local", Effect::Changes),
    known("read", Effect::Changes),
    known("getopts", Effect::Changes),
    known("let", Effect::Changes),
    known("unset", Effect::Changes),
    known("unalias", Effect::Changes),
    known("umask", Effect::Changes),
    known("ulimit", Effect::Changes),
    Known {
        runs: &["-C"],
        ..known("mapfile", Effect::Changes)
    },
    Known {
        runs: &["-C"],
        ..known("readarray", Effect::Changes)
    },
    Known {
        written: Written::Last,
        valued: &["-t", "-S", "--target-directory", "--suffix"],
        // A link, or a tree copied with the links in it.
        links_with: &[
            "-s",
            "-l",
            "-r",
            "-R",
            "-a",
            "-P",
            "-d",
            "--symbolic-link",
            "--link",
            "--recursive",
            "--archive",
            "--no-dereference",
        ],
        ..known("cp", Effect::Changes)
    },
    Known {
        written: Written::Moved,
        valued: &["-t", "-S", "--target-directory", "--suffix"],
        ..known("mv", Effect::Links)
    },
    Known {
        written: Written::Replaced,
        valued: &["-t", "-S", "--target-directory", "--suffix"],
        ..known("ln", Effect::Links)
    },
];

/// What `git` runs before its subcommand: the options that take a value,
/// and the ones that change nothing it runs. `-c`, `--config-env` and
/// `--exec-path` may name any program; any other option is not read.
const GIT_VALUED: &[&str] = &["-C", "--git-dir", "--work-tree", "--namespace"];
const GIT_FLAGS: &[&str] = &[
    "--no-pager",
    "-P",
    "-p",
    "--paginate",
    "--bare",
    "--no-replace-objects",
    "--literal-pathspecs",
    "--glob-pathspecs",
    "--noglob-pathspecs",
    "--icase-pathspecs",
    "--no-optional-locks",
    "--no-advice",
    "--version",
];

/// A `git` subcommand Hide knows, with the flags whose value names a file
/// it writes and the ones that make it run another program (a pager, an
/// external diff or filter, a remote program, a command per commit).
struct GitSub {
    name: &'static str,
    effect: Effect,
    writes: &'static [&'static str],
    runs: &'static [&'static str],
}

const fn git_sub(name: &'static str, effect: Effect) -> GitSub {
    GitSub {
        name,
        effect,
        writes: &[],
        runs: &[],
    }
}

/// What a diff of git's own runs: the external diff and the filters its
/// settings name.
const GIT_DIFF_RUNS: &[&str] = &["--ext-diff", "--textconv"];
const GIT_REMOTE_RUNS: &[&str] = &["--upload-pack", "-u"];
/// The merge strategies git carries; another names a program it finds.
const GIT_STRATEGIES: &[&str] = &["ort", "recursive", "resolve", "octopus", "ours", "subtree"];

const GIT_SUBS: &[GitSub] = &[
    git_sub("status", Effect::Reads),
    GitSub {
        writes: &["--output"],
        runs: GIT_DIFF_RUNS,
        ..git_sub("log", Effect::Reads)
    },
    GitSub {
        writes: &["--output"],
        runs: GIT_DIFF_RUNS,
        ..git_sub("diff", Effect::Reads)
    },
    GitSub {
        writes: &["--output"],
        runs: GIT_DIFF_RUNS,
        ..git_sub("show", Effect::Reads)
    },
    GitSub {
        runs: &["--textconv"],
        ..git_sub("blame", Effect::Reads)
    },
    GitSub {
        runs: &["--textconv", "--filters"],
        ..git_sub("cat-file", Effect::Reads)
    },
    GitSub {
        runs: &["-O", "--open-files-in-pager"],
        ..git_sub("grep", Effect::Reads)
    },
    GitSub {
        runs: &["--upload-pack"],
        ..git_sub("ls-remote", Effect::Reads)
    },
    git_sub("rev-parse", Effect::Reads),
    git_sub("rev-list", Effect::Reads),
    git_sub("ls-files", Effect::Reads),
    git_sub("ls-tree", Effect::Reads),
    git_sub("describe", Effect::Reads),
    git_sub("shortlog", Effect::Reads),
    git_sub("merge-base", Effect::Reads),
    git_sub("show-ref", Effect::Reads),
    git_sub("for-each-ref", Effect::Reads),
    git_sub("name-rev", Effect::Reads),
    git_sub("check-ignore", Effect::Reads),
    git_sub("count-objects", Effect::Reads),
    git_sub("version", Effect::Reads),
    git_sub("add", Effect::Changes),
    git_sub("commit", Effect::Changes),
    git_sub("tag", Effect::Changes),
    git_sub("mv", Effect::Changes),
    GitSub {
        runs: GIT_REMOTE_RUNS,
        ..git_sub("fetch", Effect::Changes)
    },
    GitSub {
        runs: &["--template"],
        ..git_sub("init", Effect::Changes)
    },
    git_sub("checkout", Effect::Links),
    git_sub("switch", Effect::Links),
    git_sub("restore", Effect::Links),
    git_sub("reset", Effect::Links),
    git_sub("merge", Effect::Links),
    git_sub("cherry-pick", Effect::Links),
    git_sub("revert", Effect::Links),
    git_sub("am", Effect::Links),
    git_sub("apply", Effect::Links),
    GitSub {
        runs: GIT_REMOTE_RUNS,
        ..git_sub("pull", Effect::Links)
    },
    GitSub {
        runs: &["-x", "--exec", "-i", "--interactive"],
        ..git_sub("rebase", Effect::Links)
    },
    GitSub {
        runs: &["--upload-pack", "-u", "--template", "-c", "--config"],
        ..git_sub("clone", Effect::Links)
    },
];

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
const DELETERS: &[&str] = &[
    "rm",
    "rmdir",
    "unlink",
    "shred",
    "srm",
    "trash",
    "trash-put",
];

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

/// Programs that run the command after their own options, which Hide reads
/// as a command of its own: their flags, the options that take a value,
/// and those that take the string `xargs` puts each input in.
struct Wrapper {
    name: &'static str,
    flags: &'static [&'static str],
    valued: &'static [&'static str],
    replace: &'static [&'static str],
}

const WRAPPERS: &[Wrapper] = &[
    Wrapper {
        name: "env",
        flags: &[
            "-i",
            "-0",
            "-v",
            "--ignore-environment",
            "--null",
            "--debug",
        ],
        valued: &["-u", "--unset", "-C", "--chdir"],
        replace: &[],
    },
    Wrapper {
        name: "command",
        flags: &["-p"],
        valued: &[],
        replace: &[],
    },
    Wrapper {
        name: "builtin",
        flags: &[],
        valued: &[],
        replace: &[],
    },
    Wrapper {
        name: "nice",
        flags: &[],
        valued: &["-n", "--adjustment"],
        replace: &[],
    },
    Wrapper {
        name: "timeout",
        flags: &["--foreground", "--preserve-status", "-v", "--verbose"],
        valued: &["-s", "-k", "--signal", "--kill-after"],
        replace: &[],
    },
    Wrapper {
        name: "stdbuf",
        flags: &[],
        valued: &["-i", "-o", "-e", "--input", "--output", "--error"],
        replace: &[],
    },
    Wrapper {
        name: "xargs",
        flags: &[
            "-0",
            "-r",
            "-t",
            "-x",
            "-o",
            "--null",
            "--no-run-if-empty",
            "--verbose",
            "--exit",
            "--open-tty",
        ],
        valued: &[
            "-L",
            "-n",
            "-P",
            "-s",
            "-d",
            "-E",
            "-a",
            "-R",
            "-S",
            "--max-lines",
            "--max-args",
            "--max-procs",
            "--max-chars",
            "--delimiter",
            "--eof",
            "--arg-file",
        ],
        replace: &["-I", "-J", "-i", "--replace"],
    },
];

/// What a word stands for that `xargs` fills from its input.
const FROM_INPUT: &str = "$input";

/// Shells whose `-c` runs the script after it, which Hide reads as a line
/// of its own; a script from a file or from the shell's input is not read.
const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh", "mksh"];

/// Commands that set the variables their words name.
const ASSIGNING: &[&str] = &[
    "export",
    "declare",
    "typeset",
    "readonly",
    "local",
    "read",
    "mapfile",
    "readarray",
    "getopts",
    "printf",
];

fn read_line(text: &str, folders: &Folders, place: &Place<'_>, depth: usize, line: &mut Line) {
    if depth > MAX_DEPTH {
        line.unreadable();
        return;
    }
    let parsed = hide_shell::shell_line(text);
    if parsed.commands.is_empty() && parsed.substitutions.is_empty() {
        line.other = true;
        // A quote the text ended inside makes the shell run none of it;
        // anything but blank lines and comments is read as unknown.
        if text
            .lines()
            .any(|row| !row.trim().is_empty() && !row.trim_start().starts_with('#'))
        {
            line.unreadable();
        }
        return;
    }
    let written: Vec<&(usize, String)> = parsed
        .writes
        .iter()
        .filter(|(_, target)| !SAFE_TARGETS.contains(&target.as_str()))
        .collect();
    line.other |= !written.is_empty();
    let first_delete = line.paths.len();
    if parsed.functions {
        // A function's body runs wherever a later word names it, and that
        // word may be any program's name.
        line.unreadable();
    }
    let start = folders.clone();
    let mut after = After {
        ok: start.clone(),
        failed: start.clone(),
    };
    // Every folder a command of the line may run in, which is where the
    // substitutions run.
    let mut ever = start;
    // Each open subshell: its number, how it follows the command before
    // it, and where the shell was then.
    let mut open: Vec<(usize, Join, After)> = Vec::new();
    let mut pipeline = first_delete;
    for (index, words) in parsed.commands.iter().enumerate() {
        let scope = &parsed.scopes[index];
        while open
            .last()
            .is_some_and(|(number, _, _)| !scope.contains(number))
        {
            if let Some((_, join, before)) = open.pop() {
                // A subshell's folder ends with it.
                after = before.then(join, &before.runs(join), Move::Stay);
            }
        }
        let join = parsed.joins[index];
        for number in scope {
            if !open.iter().any(|(opened, _, _)| opened == number) {
                open.push((*number, join, after.clone()));
            }
        }
        if join != Join::Pipe {
            pipeline = line.paths.len();
        }
        let own = line.paths.len();
        let wrote = line.writes;
        let ran = after.runs(join);
        ever = joined(&ever, &ran);
        // The shell opens a command's redirections before it runs.
        for (_, target) in written.iter().filter(|(at, _)| *at == index) {
            for path in paths(target, &ran, place.home) {
                line.overwrite(path);
            }
        }
        let moved = match read_command(words, &ran, place, depth, line) {
            // A body may run more than once, or after what follows it.
            Move::To(_) if parsed.bodies[index] => Move::Unknown,
            moved => moved,
        };
        after = after.then(join, &ran, moved);
        if line.writes && !wrote {
            // The commands of a pipeline run together: one may make a link
            // before another deletes or writes through it. What follows is
            // unnamed as it is read.
            unname(&mut line.paths[pipeline..own]);
        }
    }
    // A redirection with no command still opens its file.
    for (_, target) in written
        .iter()
        .filter(|(at, _)| *at >= parsed.commands.len())
    {
        for path in paths(target, &ever, place.home) {
            line.overwrite(path);
        }
    }
    let wrote = line.writes;
    for inner in parsed
        .substitutions
        .iter()
        .map(|(_, text)| text)
        .chain(&parsed.backquoted)
    {
        read_line(inner, &ever, place, depth + 1, line);
    }
    if line.writes && !wrote {
        // A substitution runs before the command it belongs to.
        unname(&mut line.paths[first_delete..]);
    }
}

fn unname(paths: &mut [(Change, Option<String>)]) {
    for (_, path) in paths {
        *path = None;
    }
}

fn read_command(
    words: &[String],
    folders: &Folders,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) -> Move {
    let mut words = words;
    let mut looped = false;
    while let Some((first, rest)) = words.split_first()
        && KEYWORDS.contains(&first.as_str())
    {
        looped |= matches!(first.as_str(), "while" | "until");
        words = rest;
    }
    let Some(first) = words.first() else {
        return Move::Stay;
    };
    if OPAQUE.contains(&first.as_str()) && !(first == "exec" && words.len() == 1) {
        line.unreadable();
        return Move::Unknown;
    }
    if CONTROL.contains(&first.as_str()) {
        // `for NAME in` and `select NAME in` set NAME.
        if matches!(first.as_str(), "for" | "select")
            && words.get(1).is_some_and(|name| sets_environment(name))
        {
            line.environment = true;
        }
        return Move::Stay;
    }
    let mut assigned = false;
    while let Some((first, rest)) = words.split_first()
        && hide_shell::is_assignment(first)
    {
        assigned = true;
        line.environment |= sets_environment(first);
        words = rest;
    }
    if words.is_empty() {
        return Move::Stay;
    }
    match run_program(words, assigned, false, folders, place, depth, line) {
        // A loop's test runs again and again.
        Move::To(_) if looped => Move::Unknown,
        moved => moved,
    }
}

/// Whether a word that sets a variable (`NAME=value`, `NAME+=value`, or a
/// name a command sets) sets one programs read from the environment: a
/// name in capitals, as the environment's are, or zsh's `path`, which is
/// `PATH`.
fn sets_environment(word: &str) -> bool {
    let name = word.split(['=', '+', '[']).next().unwrap_or(word);
    let is_name = name
        .chars()
        .enumerate()
        .all(|(index, c)| c == '_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit()));
    !name.is_empty() && is_name && (name.chars().any(|c| c.is_ascii_uppercase()) || name == "path")
}

/// Whether the shell expands a program word into something Hide does not
/// see: a variable, a substitution, a pattern or braces.
fn expands(program: &str) -> bool {
    program.contains(['$', '`', '*', '?', '{'])
        || (program.contains('[') && !matches!(program, "[" | "[["))
}

/// One program and its words. `assigned` when variables were set for it,
/// which could change what a bare name runs; `stdin_names` when more words
/// come from its input (`xargs`), as a word holding [`FROM_INPUT`] does.
fn run_program(
    words: &[String],
    assigned: bool,
    stdin_names: bool,
    folders: &Folders,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) -> Move {
    let Some((program, args)) = words.split_first() else {
        return Move::Stay;
    };
    if expands(program) {
        line.unreadable();
        return Move::Stay;
    }
    let from_input = stdin_names || args.iter().any(|word| word.contains(FROM_INPUT));
    if names_hide(program, place.hide) {
        hide_command(args, assigned, from_input, line);
        return Move::Stay;
    }
    if program.contains('/') || (cfg!(windows) && program.contains('\\')) {
        // A program named by its path: Hide knows programs by their names,
        // and the file there may hold anything.
        line.unreadable();
        return Move::Stay;
    }
    let name = program_name(program);
    if from_input
        && !DELETERS.contains(&name)
        && !KNOWN
            .iter()
            .any(|known| known.name == name && known.runs.is_empty())
    {
        // Words from the input may be any of the program's options, one
        // that runs a command included.
        line.unreadable();
        return Move::Stay;
    }
    if let Some(wrapper) = WRAPPERS.iter().find(|wrapper| wrapper.name == name) {
        return wrapped(wrapper, args, assigned, folders, place, depth, line);
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
            Some(script) => read_line(script, folders, place, depth + 1, line),
            // A script from a file or from the shell's input (a pipe, a
            // heredoc), which Hide does not read.
            None => line.unreadable(),
        }
        return Move::Stay;
    }
    if ASSIGNING.contains(&name) && args.iter().any(|word| sets_environment(word)) {
        line.environment = true;
    }
    match name {
        // It runs in this shell, so where it leaves the shell is not
        // followed.
        "eval" => {
            read_line(&args.join(" "), folders, place, depth + 1, line);
            Move::Unknown
        }
        "cd" | "pushd" => cd(args, folders, place.home),
        "popd" => Move::Unknown,
        // They change which program a name runs.
        "hash" | "enable" if !args.is_empty() => {
            line.other = true;
            line.environment = true;
            Move::Stay
        }
        "hash" | "enable" => Move::Stay,
        "git" => {
            git(args, folders, place, line);
            Move::Stay
        }
        "herdr" => {
            if HERDR_CLOSES.iter().any(|close| {
                args.windows(close.len()).any(|window| {
                    window
                        .iter()
                        .zip(close.iter())
                        .all(|(word, want)| word == want)
                })
            }) {
                line.other = true;
                line.ends.push(None);
            } else {
                // Its other commands start programs in panes.
                line.unreadable();
            }
            Move::Stay
        }
        "find" => {
            find(args, folders, place, depth, line);
            Move::Stay
        }
        "sed" => {
            if !sed_only_prints(args) {
                // A script may write files (`w`, `-i`) or run commands
                // (`e`, `s///e`).
                line.unreadable();
            }
            Move::Stay
        }
        _ if DELETERS.contains(&name) => {
            line.other = true;
            delete_names(args, folders, place, line);
            if from_input {
                line.delete(None);
            }
            Move::Stay
        }
        _ => {
            match KNOWN.iter().find(|known| known.name == name) {
                Some(known) => known_program(known, args, from_input, folders, place, line),
                // `source`, `alias`, `trap`, an interpreter, a build tool or
                // any program Hide does not know may run anything.
                None => line.unreadable(),
            }
            Move::Stay
        }
    }
}

/// A `hide` command; `from_input` when some of its words come from
/// `xargs`, which may make it an archive Hide cannot name.
fn hide_command(args: &[String], assigned: bool, from_input: bool, line: &mut Line) {
    if assigned {
        line.other = true;
    } else {
        line.hide = true;
    }
    // An archive whose words come from the input names a session Hide
    // cannot name.
    let seen: Vec<&String> = args
        .iter()
        .take_while(|word| !word.contains(FROM_INPUT))
        .collect();
    for ends in HIDE_ENDS {
        let starts = seen
            .iter()
            .zip(ends.iter())
            .all(|(word, want)| word == want);
        if starts && seen.len() > ends.len() {
            line.ends.push(
                seen[ends.len()..]
                    .iter()
                    .find(|word| !word.starts_with('-'))
                    .map(|word| (*word).clone()),
            );
        } else if starts && from_input {
            line.ends.push(None);
        }
    }
}

/// A program that runs the command after its options, read as a command of
/// its own; an option Hide does not know is not read past.
fn wrapped(
    wrapper: &Wrapper,
    args: &[String],
    assigned: bool,
    folders: &Folders,
    place: &Place<'_>,
    depth: usize,
    line: &mut Line,
) -> Move {
    let name = wrapper.name;
    let mut assigned = assigned;
    let mut folders = folders.clone();
    let mut replace: Option<String> = None;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        if word == "--" {
            break;
        }
        if name == "env" && hide_shell::is_assignment(word) {
            assigned = true;
            line.environment |= sets_environment(word);
            continue;
        }
        if !word.starts_with('-') || word == "-" {
            at -= 1;
            break;
        }
        if name == "command" && matches!(word.as_str(), "-v" | "-V") {
            // `command -v` names what would run and runs nothing.
            return Move::Stay;
        }
        if name == "nice" && word[1..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let (option, attached) = match word.split_once('=') {
            Some((option, value)) if word.starts_with("--") => (option, Some(value.to_owned())),
            _ if !word.starts_with("--") && word.len() > 2 => {
                (&word[..2], Some(word[2..].to_owned()))
            }
            _ => (word.as_str(), None),
        };
        if wrapper.flags.contains(&word.as_str()) {
            continue;
        }
        let value = if wrapper.replace.contains(&option) {
            // `-i` and `--replace` alone take `{}`.
            Some(attached.unwrap_or_else(|| {
                if matches!(option, "-i" | "--replace") {
                    "{}".to_owned()
                } else {
                    at += 1;
                    args.get(at - 1).cloned().unwrap_or_default()
                }
            }))
            .inspect(|string| replace = Some(string.clone()))
        } else if wrapper.valued.contains(&option) {
            attached.or_else(|| {
                at += 1;
                args.get(at - 1).cloned()
            })
        } else {
            // `env -S` splits its value into the command line, `env -P`
            // looks the program up in a folder it names; any other option
            // may take a value Hide would read as the program.
            line.unreadable();
            return Move::Stay;
        };
        if name == "env" && matches!(option, "-C" | "--chdir") {
            folders = value.map_or_else(|| vec![None], |dir| paths(&dir, &folders, place.home));
        }
    }
    if name == "timeout" {
        // The duration before the command.
        at += 1;
    }
    let inner = args.get(at..).unwrap_or_default();
    if inner.is_empty() {
        return Move::Stay;
    }
    let mut stdin_names = false;
    let inner: Vec<String> = match &replace {
        Some(string) if !string.is_empty() => inner
            .iter()
            .map(|word| word.replace(string.as_str(), FROM_INPUT))
            .collect(),
        _ => {
            stdin_names = name == "xargs";
            inner.to_vec()
        }
    };
    let moved = run_program(&inner, assigned, stdin_names, &folders, place, depth, line);
    // Only the shell's own `command` and `builtin` run its `cd`; the others
    // run a program, whose folder is its own.
    if matches!(name, "command" | "builtin") {
        moved
    } else {
        Move::Stay
    }
}

/// A program of [`KNOWN`] with its words.
fn known_program(
    known: &Known,
    args: &[String],
    from_input: bool,
    folders: &Folders,
    place: &Place<'_>,
    line: &mut Line,
) {
    if args.iter().any(|word| refused(word, known.runs)) {
        line.unreadable();
        return;
    }
    let mut files: Vec<String> = flag_values(args, known.output);
    let (names, target) = operands(args, known.valued);
    let forced = args
        .iter()
        .any(|word| word == "--force" || refused(word, &["-f"]));
    let last = || {
        target
            .clone()
            .or_else(|| names.get(1..).and_then(<[String]>::last).cloned())
    };
    match known.written {
        Written::Nothing => {}
        Written::Last => files.extend(last()),
        Written::Every => files.extend(names.iter().cloned()),
        Written::Moved => files.extend(names.iter().cloned().chain(target.clone())),
        Written::Replaced if forced => match (last(), names.as_slice()) {
            (Some(file), _) => files.push(file),
            // `ln -sf ../a` makes `a` here.
            (None, [only]) => files.extend(only.rsplit('/').next().map(str::to_owned)),
            _ => {}
        },
        Written::Replaced => {}
        Written::Second => files.extend(names.get(1).cloned()),
        Written::Operand(prefix) => files.extend(
            args.iter()
                .filter_map(|word| word.strip_prefix(prefix))
                .map(str::to_owned),
        ),
    }
    for file in &files {
        for path in paths(file, folders, place.home) {
            line.overwrite(path);
        }
    }
    let names_files = known.written != Written::Nothing || !known.output.is_empty();
    if from_input && names_files {
        // Words from the input may name a file it writes.
        line.overwrite(None);
    }
    let writes = !files.is_empty()
        || args.iter().any(|word| refused(word, known.writes))
        || known.names.is_some_and(|most| names.len() > most)
        // Words from the input may be a flag that writes or one more name.
        || (from_input && (!known.writes.is_empty() || known.names.is_some() || names_files));
    let links = args.iter().any(|word| refused(word, known.links_with));
    match known.effect {
        Effect::Reads => line.other |= writes,
        Effect::Changes if !links => line.other = true,
        Effect::Changes | Effect::Links => {
            line.other = true;
            line.writes = true;
        }
    }
}

/// The words a program names besides its options and their values, and the
/// folder `-t` (`--target-directory`) names.
fn operands(args: &[String], valued: &[&str]) -> (Vec<String>, Option<String>) {
    let mut names = Vec::new();
    let mut target = None;
    let mut options = true;
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if !options || !word.starts_with('-') || word == "-" {
            names.push(word.clone());
            continue;
        }
        if word == "--" {
            options = false;
            continue;
        }
        let (option, attached) = match word.split_once('=') {
            Some((option, value)) if word.starts_with("--") => (option, Some(value.to_owned())),
            _ if !word.starts_with("--") && word.len() > 2 && valued.contains(&&word[..2]) => {
                (&word[..2], Some(word[2..].to_owned()))
            }
            _ => (word.as_str(), None),
        };
        if !valued.contains(&option) {
            continue;
        }
        let value = attached.or_else(|| words.next().cloned());
        if matches!(option, "-t" | "--target-directory") {
            target = value;
        }
    }
    (names, target)
}

/// The values the flags `flags` take in `args`: `-o file`, `-ofile`,
/// `--output file`, `--output=file`.
fn flag_values(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        for flag in flags {
            if word == flag {
                values.extend(words.clone().next().cloned());
            } else if let Some(value) = word.strip_prefix(&format!("{flag}=")) {
                values.push(value.to_owned());
            } else if !flag.starts_with("--")
                && word.starts_with(flag)
                && word.len() > flag.len()
                && !word.starts_with("--")
            {
                values.push(word[flag.len()..].to_owned());
            }
        }
    }
    values
}

/// Whether `sed` only prints: no `-i`, no script file, and scripts of line
/// numbers with `p`, `d`, `q`, `n` and `=` only, such as `sed -n '1,80p'`.
fn sed_only_prints(args: &[String]) -> bool {
    let prints = |script: &str| {
        !script.is_empty()
            && script
                .chars()
                .all(|c| c.is_ascii_digit() || " \t\n,;$!pdqn=".contains(c))
    };
    let mut script = false;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        match word.as_str() {
            "-n" | "--quiet" | "--silent" | "-E" | "-r" | "--regexp-extended" | "-u" | "-s"
            | "-z" | "--" => {}
            "-e" | "--expression" => {
                if !args.get(at).is_some_and(|next| prints(next)) {
                    return false;
                }
                at += 1;
                script = true;
            }
            flags if flags.starts_with('-') && !flags.starts_with("--") && flags.len() > 1 => {
                // A group of short flags, which may end in `e` and take the
                // script after it.
                let letters = &flags[1..];
                let (group, takes) = letters
                    .strip_suffix('e')
                    .map_or((letters, false), |group| (group, true));
                if !group.chars().all(|c| "nErusz".contains(c)) {
                    return false;
                }
                if takes {
                    if !args.get(at).is_some_and(|next| prints(next)) {
                        return false;
                    }
                    at += 1;
                    script = true;
                }
            }
            other if other.starts_with('-') && other != "-" => return false,
            name if !script => {
                if !prints(name) {
                    return false;
                }
                script = true;
            }
            _ => {}
        }
    }
    script
}

/// Where `cd` (or `pushd`) goes: lexically, as the shell's `cd` without
/// `-P` goes. `cd -` goes back to a folder Hide does not track, `-P` with
/// `..` goes up from where a link leads, and a name that starts with no
/// `/`, `~` or `.` may be found through `CDPATH`.
fn cd(args: &[String], folders: &Folders, home: Option<&str>) -> Move {
    let mut physical = false;
    let mut options = true;
    let mut target = None;
    for word in args {
        if options && word == "--" {
            options = false;
        } else if options && word.starts_with('-') && word != "-" {
            physical |= word.contains('P');
        } else {
            target = Some(word.as_str());
            break;
        }
    }
    let Some(target) = target else {
        return Move::To(vec![home.map(str::to_owned)]);
    };
    if target == "-" || (physical && target.split('/').any(|name| name == "..")) {
        return Move::Unknown;
    }
    let mut to: Folders = Vec::new();
    for path in paths(target, folders, home) {
        add(&mut to, path.map(|path| normalize(&path)));
    }
    if !(target.starts_with(['/', '~', '.'])
        || hide_platform::path::is_wire_absolute(&wire_word(target)))
    {
        add(&mut to, None);
    }
    Move::To(to)
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
            word == *flag
                || word
                    .strip_prefix(flag)
                    .is_some_and(|rest| rest.starts_with('='))
        } else {
            let letter = flag.trim_start_matches('-');
            word.starts_with('-') && !word.starts_with("--") && word[1..].contains(letter)
        }
    })
}

fn git(args: &[String], folders: &Folders, place: &Place<'_>, line: &mut Line) {
    let mut folders = folders.clone();
    // Another repository or work tree: a path it deletes relative to it is
    // one Hide cannot name.
    let mut elsewhere = false;
    let mut at = 0;
    while let Some(word) = args.get(at) {
        at += 1;
        if !word.starts_with('-') {
            at -= 1;
            break;
        }
        let (option, attached) = match word.split_once('=') {
            Some((option, value)) if word.starts_with("--") => (option, Some(value.to_owned())),
            _ => (word.as_str(), None),
        };
        if GIT_FLAGS.contains(&word.as_str()) {
            continue;
        }
        if !GIT_VALUED.contains(&option) {
            // `-c`, `--config-env` and `--exec-path` may name any program
            // (an alias, a pager, a hook).
            line.unreadable();
            return;
        }
        let value = attached.or_else(|| {
            at += 1;
            args.get(at - 1).cloned()
        });
        match option {
            "-C" => {
                folders = value.map_or_else(|| vec![None], |dir| paths(&dir, &folders, place.home));
            }
            "--git-dir" | "--work-tree" => elsewhere = true,
            _ => {}
        }
    }
    if elsewhere {
        folders = vec![None];
    }
    let Some(sub) = args.get(at) else {
        return;
    };
    let rest = &args[at + 1..];
    let action = rest
        .iter()
        .find(|word| !word.starts_with('-'))
        .map(String::as_str);
    match sub.as_str() {
        "push" => {
            line.other = true;
            if rest
                .iter()
                .any(|word| refused(word, &["--receive-pack", "--exec"]))
            {
                line.unreadable();
                return;
            }
            line.force_push |= rest.iter().any(|word| {
                FORCE_PUSH_FLAGS.iter().any(|flag| {
                    word == flag
                        || word
                            .strip_prefix(flag)
                            .is_some_and(|tail| tail.starts_with('='))
                }) || (word.starts_with('-') && !word.starts_with("--") && word.contains('f'))
                    || (word.starts_with('+') && word.len() > 1)
            });
            // A refspec with nothing before its `:` deletes the remote's ref.
            line.delete_remote |= rest.iter().any(|word| {
                matches!(word.as_str(), "--delete" | "--prune" | "--mirror")
                    || (word.starts_with('-') && !word.starts_with("--") && word.contains('d'))
                    || (word.starts_with(':') && word.len() > 1)
                    || (word.starts_with("+:") && word.len() > 2)
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
                for folder in &folders {
                    line.delete(folder.as_deref().map(below));
                }
            } else {
                for name in names {
                    for path in paths(name, &folders, place.home) {
                        line.delete(path.map(|path| below(&path)));
                    }
                }
            }
        }
        "rm" => {
            line.other = true;
            if rest
                .iter()
                .any(|word| matches!(word.as_str(), "--cached" | "-n" | "--dry-run"))
            {
                return;
            }
            if rest
                .iter()
                .any(|word| word.starts_with("--pathspec-from-file"))
            {
                line.delete(None);
            }
            delete_names(rest, &folders, place, line);
        }
        "branch" => {
            if rest.iter().any(|word| word == "--edit-description") {
                // It opens an editor.
                line.unreadable();
                return;
            }
            line.other |= !rest
                .iter()
                .all(|word| GIT_BRANCH_LISTS.contains(&word.as_str()));
        }
        // A setting may name a program git runs later.
        "config" => {
            let reads = rest.iter().any(|word| {
                matches!(
                    word.as_str(),
                    "--get" | "--get-all" | "--get-regexp" | "--get-urlmatch" | "-l" | "--list"
                )
            }) || matches!(action, Some("get" | "list"));
            let writes = rest.iter().any(|word| {
                matches!(
                    word.as_str(),
                    "--add"
                        | "--unset"
                        | "--unset-all"
                        | "--replace-all"
                        | "--rename-section"
                        | "--remove-section"
                        | "-e"
                        | "--edit"
                )
            });
            if !reads || writes {
                line.unreadable();
            }
        }
        "remote" => match action {
            None | Some("show" | "get-url") => {}
            Some(
                "add" | "remove" | "rm" | "rename" | "set-url" | "set-head" | "set-branches"
                | "prune" | "update",
            ) => {
                line.other = true;
            }
            Some(_) => line.unreadable(),
        },
        "stash" => match action {
            Some("list" | "show") => {}
            None
            | Some(
                "push" | "pop" | "apply" | "drop" | "clear" | "save" | "branch" | "create"
                | "store",
            ) => {
                line.other = true;
                line.writes = true;
            }
            Some(_) => line.unreadable(),
        },
        "reflog" => line.other |= matches!(action, Some("expire" | "delete" | "drop")),
        "submodule" => match action {
            None | Some("status" | "summary") => {}
            Some(
                "init" | "update" | "add" | "sync" | "deinit" | "absorbgitdirs" | "set-branch"
                | "set-url",
            ) => {
                line.other = true;
                line.writes = true;
            }
            // `foreach` runs a command in each.
            Some(_) => line.unreadable(),
        },
        "worktree" => match action {
            Some("list") => {}
            Some("remove") => {
                line.other = true;
                let target = rest.iter().filter(|word| !word.starts_with('-')).nth(1);
                match target {
                    Some(target) => {
                        for path in paths(target, &folders, place.home) {
                            line.delete(path);
                        }
                    }
                    None => line.delete(None),
                }
            }
            Some("add" | "move" | "prune" | "lock" | "unlock" | "repair") => {
                line.other = true;
                line.writes = true;
            }
            _ => line.unreadable(),
        },
        sub => {
            let Some(known) = GIT_SUBS.iter().find(|known| known.name == sub) else {
                // An alias, `bisect run`, a tool, a filter or any subcommand
                // Hide does not know may run anything.
                line.unreadable();
                return;
            };
            if rest.iter().any(|word| refused(word, known.runs)) || !git_strategy_known(rest) {
                line.unreadable();
                return;
            }
            match known.effect {
                Effect::Reads => {
                    let files = flag_values(rest, known.writes);
                    line.other |= !files.is_empty();
                    for file in &files {
                        for path in paths(file, &folders, place.home) {
                            line.overwrite(path);
                        }
                    }
                }
                Effect::Changes => line.other = true,
                Effect::Links => {
                    line.other = true;
                    line.writes = true;
                }
            }
        }
    }
}

/// Whether a merge strategy named (`-s`, `--strategy`) is one git carries.
fn git_strategy_known(rest: &[String]) -> bool {
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let named = match word.as_str() {
            "-s" | "--strategy" => words.next().map(String::as_str),
            word if word.starts_with("--strategy=") => Some(&word["--strategy=".len()..]),
            word if word.starts_with("-s") && word.len() > 2 && !word.starts_with("--") => {
                Some(&word[2..])
            }
            _ => continue,
        };
        if !named.is_some_and(|name| GIT_STRATEGIES.contains(&name)) {
            return false;
        }
    }
    true
}

fn find(args: &[String], folders: &Folders, place: &Place<'_>, depth: usize, line: &mut Line) {
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
                    for path in paths(root, folders, place.home) {
                        line.delete(path.map(|path| below(&path)));
                    }
                }
            }
            "-exec" | "-execdir" | "-ok" | "-okdir" => {
                let end = args[at + 1..]
                    .iter()
                    .position(|word| word == ";" || word == "+")
                    .map_or(args.len(), |offset| at + 1 + offset);
                let inner = &args[at + 1..end];
                // `-execdir` runs the command in the folder of each file
                // found, at a depth Hide cannot name.
                let here = word.ends_with("dir");
                for root in &roots {
                    for folder in folders {
                        // What `find` hands the command lies below its root.
                        let found = path_of(root, folder.as_deref(), place.home)
                            .map_or_else(|| "$found".to_owned(), |root| below(&root));
                        let words: Vec<String> = inner
                            .iter()
                            .map(|word| word.replace("{}", &found))
                            .collect();
                        let runs_in = if here {
                            vec![None]
                        } else {
                            vec![folder.clone()]
                        };
                        run_program(&words, false, false, &runs_in, place, depth, line);
                    }
                }
                at = end;
            }
            "-fprint" | "-fprint0" | "-fprintf" | "-fls" => {
                line.other = true;
                match args.get(at + 1) {
                    Some(file) => {
                        for path in paths(file, folders, place.home) {
                            line.overwrite(path);
                        }
                    }
                    None => line.overwrite(None),
                }
            }
            _ => {}
        }
        at += 1;
    }
}

fn delete_names(args: &[String], folders: &Folders, place: &Place<'_>, line: &mut Line) {
    let mut options = true;
    for word in args {
        if options && word == "--" {
            options = false;
            continue;
        }
        if options && word.starts_with('-') && word.len() > 1 {
            continue;
        }
        for path in paths(word, folders, place.home) {
            line.delete(path);
        }
    }
}

/// A path standing for what lies below `folder`, which a command that
/// deletes inside a folder deletes, not the folder itself.
fn below(folder: &str) -> String {
    format!("{}/{{}}", folder.trim_end_matches('/'))
}

/// The paths a word names from each folder a command may run in.
fn paths(word: &str, folders: &Folders, home: Option<&str>) -> Folders {
    let mut found = Folders::new();
    for folder in folders {
        add(&mut found, path_of(word, folder.as_deref(), home));
    }
    if folders.is_empty() {
        add(&mut found, path_of(word, None, home));
    }
    found
}

/// A word a command names as a path, in wire spelling: absolute, from
/// `~`, or from the folder the command runs in, with its `..` kept, since
/// the system takes `..` from where a link leads ([`follow_links`]).
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

/// The paths a reading deletes or overwrites where the system finds them:
/// each link on the way followed and each `..` taken from where it leads. It reads the
/// filesystem, so it runs off the core's lock, in the worker that read the
/// request, before the core answers it.
pub fn follow_links(reading: &mut Reading) {
    for path in &mut reading.deletes {
        *path = path.take().and_then(|path| real_path(&path));
    }
    // Writing a name that is a link writes where it leads.
    for path in &mut reading.overwrites {
        *path = path
            .take()
            .and_then(|path| real_folder(path.trim_end_matches('/')));
    }
}

/// `path` with the links on the way to it followed. Deleting a name
/// removes that name, so a link there is not followed unless the path ends
/// in `/` or stands for what lies below a folder (`/{}`); the names past
/// the last one that exists are kept as written, since the system finds
/// none of them.
fn real_path(path: &str) -> Option<String> {
    let (path, below) = match path.strip_suffix("/{}") {
        Some(folder) => (folder, true),
        None => (path, false),
    };
    let follows = below || path.ends_with('/') || path.ends_with("/..");
    let path = path.trim_end_matches('/');
    let (folder, last) = if follows {
        (path, None)
    } else {
        let (folder, last) = path.rsplit_once('/')?;
        (if folder.is_empty() { "/" } else { folder }, Some(last))
    };
    let mut real = real_folder(folder)?;
    if let Some(last) = last {
        real = format!("{}/{last}", real.trim_end_matches('/'));
    }
    if below {
        real = format!("{}/{{}}", real.trim_end_matches('/'));
    }
    Some(real)
}

/// A folder with every link on the way to it followed, through the longest
/// part of it that exists.
fn real_folder(folder: &str) -> Option<String> {
    let names: Vec<&str> = folder.split('/').collect();
    (1..=names.len()).rev().find_map(|keep| {
        let mut found = names[..keep].join("/");
        // The root: `/`, or a Windows drive's (`C:/`).
        if found.is_empty() || found.ends_with(':') {
            found.push('/');
        }
        let native = hide_platform::path::from_wire(&found).ok()?;
        let real = hide_platform::fs::identity::canonical(&native).ok()?;
        let real = hide_platform::path::to_wire_lossy(&real);
        let rest = names[keep..].join("/");
        Some(if rest.is_empty() {
            real
        } else {
            normalize(&format!("{}/{rest}", real.trim_end_matches('/')))
        })
    })
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
    let action = match adapter.ask_mode() {
        Some(crate::adapter::AskMode::PiExtension) => pi_action(&call),
        _ => tool_action(&call, announced),
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
