//! The one answer to where an agent keeps its own folder for this account
//! (`~/.codex`, or what `CODEX_HOME` names), for every caller: the core's
//! usage reads, Hide AI's link to the operator's sign-in, Desk's, and the
//! hooks, plugins and trust Hide installs.
//!
//! The resolver is pure: the caller captures the variable's raw value
//! (`hide_platform::host::variable`) and hands it in with the account's home
//! and the scope it asks for. A home Hide makes for itself (Hide AI's
//! temporary `CODEX_HOME`, Desk's own) is not an agent home and is never
//! resolved here: its owner sets it on the child explicitly.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::AgentAdapter;

/// What the caller does with the folder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HomeScope {
    /// Reading what the agent itself reads as the operator runs it (its
    /// sign-in, its usage records): an override the agent honours moves it.
    Read,
    /// Installing Hide's integration (hooks, a plugin, folder trust): the
    /// account's default folder, where Herdr's integration for the agent goes
    /// too, whatever the launching shell exported. An agent run with its
    /// variable pointing elsewhere reads neither (docs/agent-hooks.md, Folder
    /// trust for the worktrees Hide makes).
    Install,
}

/// Why an agent's folder has no answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HomeUnresolved {
    /// The account has no home folder, and no override names the folder.
    NoAccountHome,
    /// The agent's variable names a folder Hide cannot place: a relative
    /// path, which the agent resolves against its own working folder, a
    /// folder no Hide process shares.
    InvalidOverride { variable: &'static str },
}

impl std::fmt::Display for HomeUnresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAccountHome => f.write_str("the account has no home folder"),
            Self::InvalidOverride { variable } => write!(f, "{variable} is not an absolute path"),
        }
    }
}

impl std::error::Error for HomeUnresolved {}

impl AgentAdapter {
    /// The agent's folder in [`HomeScope::Install`] for an account whose
    /// home is known, which always has an answer: the declared folder
    /// ([`HerdrIntegration::folder`](crate::HerdrIntegration)) under it.
    pub fn install_home(&self, account_home: &Path) -> PathBuf {
        self.herdr
            .folder
            .iter()
            .fold(account_home.to_path_buf(), |path, part| path.join(part))
    }

    /// The agent's folder for the account whose home is `account_home`.
    ///
    /// `value` is the raw value of the agent's declared variable
    /// ([`AgentAdapter::home_variable`]) as the caller captured it, `None`
    /// when unset; it is ignored for an agent that declares none and in
    /// [`HomeScope::Install`]. An empty value is unset, as Codex 0.166.0 reads
    /// an empty `CODEX_HOME` (probed 2026-10-31).
    pub fn home(
        &self,
        account_home: Option<&Path>,
        value: Option<&OsStr>,
        scope: HomeScope,
    ) -> Result<PathBuf, HomeUnresolved> {
        let default = || {
            account_home
                .map(|home| self.install_home(home))
                .ok_or(HomeUnresolved::NoAccountHome)
        };
        let (HomeScope::Read, Some(variable)) = (scope, self.home_variable) else {
            return default();
        };
        match value.filter(|value| !value.is_empty()) {
            None => default(),
            Some(value) if Path::new(value).is_absolute() => Ok(PathBuf::from(value)),
            Some(_) => Err(HomeUnresolved::InvalidOverride { variable }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentId;

    fn absolute(path: &str) -> PathBuf {
        // An absolute path on the system the test runs on.
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    #[test]
    fn codex_reads_its_variable_and_installs_in_the_account_folder() {
        let codex = AgentId::Codex.adapter();
        let home = absolute("/home/me");
        let custom = absolute("/elsewhere/codex");
        let read =
            |value: Option<&str>| codex.home(Some(&home), value.map(OsStr::new), HomeScope::Read);
        assert_eq!(read(None), Ok(home.join(".codex")));
        assert_eq!(read(Some("")), Ok(home.join(".codex")));
        assert_eq!(read(custom.to_str()), Ok(custom.clone()));
        assert_eq!(
            read(Some("relative-codex")),
            Err(HomeUnresolved::InvalidOverride {
                variable: "CODEX_HOME"
            })
        );
        for value in [None, Some("relative-codex"), custom.to_str()] {
            assert_eq!(
                codex.home(Some(&home), value.map(OsStr::new), HomeScope::Install),
                Ok(home.join(".codex"))
            );
        }
    }

    #[test]
    fn without_an_account_home_only_an_override_names_the_folder() {
        let codex = AgentId::Codex.adapter();
        let custom = absolute("/elsewhere/codex");
        assert_eq!(
            codex.home(None, custom.to_str().map(OsStr::new), HomeScope::Read),
            Ok(custom.clone())
        );
        for scope in [HomeScope::Read, HomeScope::Install] {
            assert_eq!(
                codex.home(None, None, scope),
                Err(HomeUnresolved::NoAccountHome)
            );
        }
        assert_eq!(
            codex.home(None, custom.to_str().map(OsStr::new), HomeScope::Install),
            Err(HomeUnresolved::NoAccountHome)
        );
    }

    #[test]
    fn an_agent_without_a_variable_is_its_declared_folder_in_every_scope() {
        let home = absolute("/home/me");
        let custom = absolute("/elsewhere");
        for adapter in crate::ADAPTERS {
            if adapter.home_variable.is_some() {
                continue;
            }
            let declared = adapter
                .herdr
                .folder
                .iter()
                .fold(home.clone(), |path, part| path.join(part));
            for scope in [HomeScope::Read, HomeScope::Install] {
                assert_eq!(
                    adapter.home(Some(&home), custom.to_str().map(OsStr::new), scope),
                    Ok(declared.clone()),
                    "{}",
                    adapter.id
                );
            }
        }
        assert_eq!(
            AgentId::Pi
                .adapter()
                .home(Some(&home), None, HomeScope::Install)
                .unwrap(),
            home.join(".pi").join("agent")
        );
    }
}
