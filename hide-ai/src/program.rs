//! The CLI a backend runs, and the `PATH` it is run with.
//!
//! A CLI is found on the account's search (`hide_platform::programs`: the
//! login shell's folders, the daemon's own and the usual install folders), and
//! a CLI installed as a script starts its interpreter by name from its
//! environment (pnpm's `codex` runs `node`). So what finds a program also
//! runs it: every child Hide AI starts, a probe as much as a request, is
//! given the `PATH` its program was found on, here and nowhere else, because a
//! program found on the login shell's folders and run with the daemon's would
//! be reported ready and then die looking for its interpreter.
//!
//! What runs is how the program starts (`programs::Start`): on Windows npm's
//! launcher is read as the program it runs, so a child is started without
//! `cmd.exe` reading its arguments again, and a batch file that cannot be
//! read is run as found.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use hide_platform::programs;

/// A CLI that was found, how it starts, and the `PATH` it was found on.
#[derive(Clone, Debug)]
pub(crate) struct Program {
    path: PathBuf,
    start: programs::Start,
    /// What the child's `PATH` is set to; `None` leaves the environment's own,
    /// for a program the caller named by file and nothing searched for.
    search: Option<OsString>,
}

impl Program {
    /// The program `binary` names: a bare name is looked for on `search`, or
    /// on the account's search when `search` is `None`; a path with more than
    /// one component is used as given when it is a file.
    ///
    /// `search` is the `PATH` value to look on and run with
    /// (`programs::cli_path_with` builds one); only a caller that has to
    /// choose it (a test, with a stand-in login shell) passes one.
    pub(crate) fn resolve(binary: &Path, search: Option<&OsStr>) -> Option<Self> {
        let explicit = binary.components().count() > 1;
        let search = match search {
            Some(search) => Some(search.to_owned()),
            None if explicit => None,
            None => Some(programs::account_path()?),
        };
        let path = if explicit {
            binary.is_file().then(|| binary.to_path_buf())?
        } else {
            programs::Program::find(search.as_deref()?, binary.to_str()?)?.into_path()
        };
        // A program named by file runs with the environment's own `PATH`,
        // which is where its launcher's Node is found.
        let start = programs::start(
            &path,
            &search
                .clone()
                .or_else(|| std::env::var_os("PATH"))
                .unwrap_or_default(),
        );
        Some(Self {
            path,
            start,
            search,
        })
    }

    /// The file that was found, which a backend reports and looks beside.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// A command that starts the program, with the words its launcher puts
    /// first; its environment is the caller's to set ([`Self::apply`]).
    pub(crate) fn command(&self) -> Command {
        match &self.start {
            programs::Start::Direct { program, leading } => {
                let mut command = Command::new(program);
                command.args(leading);
                command
            }
            programs::Start::Batch => Command::new(&self.path),
        }
    }

    /// Gives `command` the `PATH` this program was found on.
    pub(crate) fn apply(&self, command: &mut Command) {
        if let Some(search) = &self.search {
            command.env("PATH", search);
        }
    }
}

/// How one agent item runs on its machine (PRD agent-profiles-phase2 D-09):
/// the variables its account and its profile set, given to every child the
/// backend starts after that child's own base environment, and the program
/// its profile runs in the agent's place. The default is the agent's own
/// program with nothing added: its built-in default profile. The node makes
/// it from its own profile file (`hide_host::profiles::ai_run`), as a start
/// of the profile in a pane would run, so no backend reads an account's
/// folder from anywhere else.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ItemRun {
    /// The account's variables, then the profile's own.
    pub env: Vec<(String, String)>,
    /// A wrapper, found on the machine, run with the agent's arguments.
    pub program: Option<Wrapper>,
}

/// A profile's own program as its machine starts it: the file, and the
/// words before the agent's arguments (npm's Windows launcher read as the
/// program it runs).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Wrapper {
    pub file: PathBuf,
    pub leading: Vec<String>,
}

impl ItemRun {
    /// The program a child of this item starts: the profile's wrapper, or
    /// the agent's own `binary` as [`Program::resolve`] finds it. A wrapper
    /// is run with the `PATH` its machine's account searches, which is where
    /// a wrapper finds the agent and its interpreter, as in a pane.
    pub(crate) fn program(&self, binary: &Path, search: Option<&OsStr>) -> Option<Program> {
        let Some(wrapper) = &self.program else {
            return Program::resolve(binary, search);
        };
        let search = match search {
            Some(search) => search.to_owned(),
            None => programs::account_path()?,
        };
        Some(Program {
            path: wrapper.file.clone(),
            start: programs::Start::Direct {
                program: wrapper.file.clone(),
                leading: wrapper.leading.iter().map(OsString::from).collect(),
            },
            search: Some(search),
        })
    }

    /// The variables a child is given on top of its base environment.
    pub(crate) fn set(&self) -> Vec<(&str, String)> {
        self.env
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect()
    }

    /// The value this item sets for `name`, else this process's own: what a
    /// child of the item reads for it.
    pub(crate) fn variable(&self, name: &str) -> Option<OsString> {
        self.env
            .iter()
            .rev()
            .find(|(key, _)| variable_named(key, name))
            .map(|(_, value)| OsString::from(value))
            .or_else(|| hide_platform::host::variable(name))
    }
}

/// Windows reads a variable's name regardless of case.
fn variable_named(key: &str, name: &str) -> bool {
    if cfg!(windows) {
        key.eq_ignore_ascii_case(name)
    } else {
        key == name
    }
}

/// Shared by every backend that starts a user-installed CLI, and by the node
/// before the core starts one in a pane: how `binary` starts, if it is
/// anywhere the account's own terminal would find it.
pub fn resolve_start(binary: &Path) -> Option<programs::Start> {
    Program::resolve(binary, None).map(|program| program.start)
}
