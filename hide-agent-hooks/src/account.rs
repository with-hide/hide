//! The account an agent's hook goes into (docs/ARCHITECTURE.md, Agent
//! profiles and accounts).
//!
//! Every install, judgment and removal of a hook file, a plugin script or
//! Codex's trust of Hide's entries takes an [`AccountAt`]: the agent's
//! default account under a home, which is where every hook went before
//! profiles, or the account a profile runs in, as
//! `hide_agent_adapter::resolve` answered it. A home converts into the
//! first, so a caller that installs into the default account names the home
//! as before. The places themselves come from the agent's definition alone:
//! this crate names no account folder.

use std::path::{Path, PathBuf};

use hide_agent_adapter::{Account, AccountPath, AgentAdapter};

/// Where one agent's folders are.
#[derive(Clone, Copy, Debug)]
pub enum AccountAt<'a> {
    /// The agent's default account under this home, for a node that honours
    /// none of the agent's variables (`AgentAdapter::account_in`).
    Home(&'a Path),
    /// The account a profile runs in. `home` is the account's home, under
    /// which Hide keeps its own records.
    Account {
        home: &'a Path,
        account: &'a Account,
    },
}

impl<'a> From<&'a Path> for AccountAt<'a> {
    fn from(home: &'a Path) -> Self {
        Self::Home(home)
    }
}

impl<'a> From<&'a PathBuf> for AccountAt<'a> {
    fn from(home: &'a PathBuf) -> Self {
        Self::Home(home)
    }
}

impl<'a> AccountAt<'a> {
    /// The account's home: where Hide keeps its own records of the account.
    pub fn home(self) -> &'a Path {
        match self {
            Self::Home(home) | Self::Account { home, .. } => home,
        }
    }

    /// Whether this is the agent's default account.
    pub fn is_default(self) -> bool {
        match self {
            Self::Home(_) => true,
            Self::Account { account, .. } => account.key == hide_agent_adapter::AccountKey::Default,
        }
    }

    /// `adapter`'s place for `feature` in this account. The definition
    /// declares every place a hook dialect reads, so a missing one is a
    /// definition this crate's tests refuse.
    pub fn place(self, adapter: &AgentAdapter, feature: AccountPath) -> PathBuf {
        match self {
            Self::Home(home) => adapter.place_in(home, feature),
            Self::Account { account, .. } => account
                .path(feature)
                .unwrap_or_else(|| panic!("{} declares no {feature:?} place", adapter.id))
                .to_path_buf(),
        }
    }
}

/// The most account variables one report carries: no agent's `[account]`
/// names more than three.
pub const REPORTED_VARIABLES: usize = 8;

/// The longest value of one reported variable, a profile account's own
/// limit (`hide_agent_adapter::account::MAX_ACCOUNT_BYTES`) and room for
/// the folder a definition puts below it.
pub const REPORTED_VALUE_BYTES: usize = hide_agent_adapter::account::MAX_ACCOUNT_BYTES + 256;

/// The account variables of the `kind` agent's `[account]` that are set in
/// its process, as `variable` reads them, each exactly as set: what a hook
/// reports at a session's start, so the core reads the pane in the account
/// the agent runs in when the operator started it by hand (PRD
/// agent-profiles D-03, B24, B25). An account is the value as set (D-15):
/// Claude Code keys its login by the string, so `/a` and `/a/` are two
/// accounts, and nothing here resolves or respells a folder. An empty list
/// is the agent's default account; `None` for an agent Hide keeps no
/// accounts of.
///
/// An agent that drops its variables from a hook's environment has its own
/// way to the folder (Claude Code's: `crate::claude_account`).
pub fn reported_variables(
    kind: &str,
    variable: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<Vec<(String, String)>> {
    use hide_agent_adapter::definition::By;
    let adapter = hide_agent_adapter::adapter(kind)?;
    let rule = adapter.account_rule()?;
    if rule.by == By::None {
        return None;
    }
    let read = |name: &str| {
        variable(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty() && value.len() <= REPORTED_VALUE_BYTES)
    };
    let mut reported = Vec::new();
    for var in rule.vars.iter().take(REPORTED_VARIABLES) {
        let value = read(&var.name)
            .or_else(|| crate::claude_account::scrubbed_folder(adapter, &read));
        if let Some(value) = value {
            reported.push((var.name.clone(), value));
        }
    }
    Some(reported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsString;

    fn read(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let vars: BTreeMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), OsString::from(v)))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn a_hook_reports_the_account_variables_its_agent_runs_with() {
        // Exactly as set, the final separator kept: Claude Code's login is
        // keyed by the string (D-15).
        for spelled in ["/accounts/claude-a/", "/accounts/claude-a"] {
            assert_eq!(
                reported_variables("claude-code", read(&[("CLAUDE_CONFIG_DIR", spelled)])),
                Some(vec![("CLAUDE_CONFIG_DIR".to_owned(), spelled.to_owned())])
            );
        }
        // The default account sets none; agy keeps no accounts apart.
        assert_eq!(
            reported_variables("claude-code", read(&[])),
            Some(Vec::new())
        );
        assert_eq!(reported_variables("antigravity", read(&[])), None);
        // omp names its account; the name is reported as written.
        assert_eq!(
            reported_variables("omp", read(&[("OMP_PROFILE", "work")])),
            Some(vec![("OMP_PROFILE".to_owned(), "work".to_owned())])
        );
    }

    #[test]
    fn a_scrubbed_claude_hook_reports_the_folder_its_session_env_file_is_in() {
        // The file as Claude Code writes it on this system: below the
        // account's folder, in the system's own spelling (a drive and `\`
        // on Windows), which the folder is reported in as it is there.
        let folder = if cfg!(windows) {
            r"C:\Users\me\accounts\claude-a"
        } else {
            "/Users/me/accounts/claude-a"
        };
        let env_file = Path::new(folder)
            .join("session-env")
            .join("s-1")
            .join("hook-1.sh");
        let env_file = env_file.to_str().unwrap();
        let scrubbed = |file: &str| {
            read(&[
                ("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB", "1"),
                ("CLAUDE_ENV_FILE", file),
            ])
        };
        assert_eq!(
            reported_variables("claude-code", scrubbed(env_file)),
            Some(vec![("CLAUDE_CONFIG_DIR".to_owned(), folder.to_owned())])
        );
        // A file outside any account folder, or not absolute, names none.
        for file in ["session-env/s-1/hook-1.sh", "/tmp/hook-1.sh"] {
            assert_eq!(
                reported_variables("claude-code", scrubbed(file)),
                Some(Vec::new()),
                "{file}"
            );
        }
        // Without the scrub the file says nothing: the variable is absent
        // because the agent runs in its default account.
        assert_eq!(
            reported_variables("claude-code", read(&[("CLAUDE_ENV_FILE", env_file)])),
            Some(Vec::new())
        );
    }
}
