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

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use hide_platform::programs;

/// A CLI that was found, and the `PATH` it was found on.
#[derive(Clone, Debug)]
pub(crate) struct Program {
    path: PathBuf,
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
        Some(Self { path, search })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Gives `command` the `PATH` this program was found on.
    pub(crate) fn apply(&self, command: &mut Command) {
        if let Some(search) = &self.search {
            command.env("PATH", search);
        }
    }
}

/// Shared by every backend that starts a user-installed CLI, and by the core
/// before it asks Herdr to start one in a pane: where `binary` is, if it is
/// anywhere the account's own terminal would find it.
pub fn resolve_binary(binary: &Path) -> Option<PathBuf> {
    Program::resolve(binary, None).map(|program| program.path)
}
