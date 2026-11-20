//! Another machine, reached through one owned `ssh`: signed in once and
//! kept for the commands run there and the files copied to and from it.
//! The connection ends when this is dropped. What a caller runs there and
//! how it reads the answer are the caller's: a move's steps
//! (`core_move::driver`), or an install's.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hide_node::ssh::session::{Ran, SessionError, SshTarget};
use hide_node::ssh::transfer::FileCopy;
use hide_node::ssh::upstream::Upstream;
use hide_node_link::device::{SignIn, SshProblem};

/// Why this machine's build was not installed on the other one.
#[derive(Debug)]
pub enum InstallFailure {
    /// This machine could not name where the build goes.
    Local(String),
    /// The copy stopped.
    Copy(String),
}

pub struct Remote {
    /// Dialed by the first command that reaches the machine: `ssh` reads
    /// the alias then, and a dial that fails is the machine not reached,
    /// which a wait asks again, so an alias that does not resolve at a
    /// start never keeps the daemon from it.
    upstream: Upstream,
    /// The `hided` this machine runs there.
    program: String,
    state_dir: Option<String>,
}

impl Remote {
    /// Reads and dials nothing yet: the first command does.
    pub fn new(home: &Path, alias: &str, program: &str) -> Self {
        Self {
            upstream: Upstream::new(SshTarget::new(home, alias)),
            program: program.to_owned(),
            state_dir: None,
        }
    }

    pub fn with_state_dir(mut self, state_dir: &str) -> Self {
        self.state_dir = Some(state_dir.to_owned());
        self
    }

    /// Where the next dial may ask the operator; a dial with none asks
    /// nobody.
    pub fn may_ask(&self, press: Option<Arc<dyn SignIn>>) {
        self.upstream.may_ask(press);
    }

    /// What `ssh` stopped on at the last dial; none when it signed in.
    pub fn ssh_problem(&self) -> Option<SshProblem> {
        self.upstream.ssh_problem()
    }

    /// Installs this machine's build from `packages` beside the `hided` it
    /// runs there, through the core update's install (the version folder
    /// under the same helper root, at most two builds kept), and answers
    /// the installed `hided`.
    pub fn install_build(&self, packages: &Path) -> Result<String, InstallFailure> {
        let root = hide_kit::build_of(&self.program)
            .map_err(InstallFailure::Local)?
            .root;
        self.upstream
            .install_build(
                &hide_node::ssh::host::HelperPackages::new(
                    Some(packages.to_path_buf()),
                    crate::cli::RELEASE,
                ),
                &root,
            )
            .map_err(|error| InstallFailure::Copy(error.to_string()))
    }

    /// Runs `hided <words>` there, with the state folder this was given
    /// and then each `--flag value` of `args`, and answers what it printed
    /// and how it ended.
    pub fn hided(
        &self,
        words: &[&str],
        args: &[(&str, &str)],
        cap: usize,
        within: Duration,
    ) -> Result<Ran, SessionError> {
        let mut command = vec![self.program.clone()];
        command.extend(words.iter().map(|word| (*word).to_owned()));
        if let Some(state_dir) = &self.state_dir {
            command.extend(["--state-dir".to_owned(), state_dir.clone()]);
        }
        for (flag, value) in args {
            command.extend([format!("--{flag}"), (*value).to_owned()]);
        }
        self.upstream.run(&command, cap, within)
    }

    pub fn upload(
        &self,
        files: &[FileCopy],
        sent: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.upstream.upload(files, sent)
    }

    /// Downloads `files`, each of which must be below `into`.
    pub fn download(
        &self,
        into: &Path,
        files: &[FileCopy],
        received: &(dyn Fn(u64) + Sync),
    ) -> Result<(), SessionError> {
        self.upstream.download(into, files, received)
    }
}
