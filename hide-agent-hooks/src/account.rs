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

/// How a folder a variable names is compared, the same on the node that
/// lists a profile's variables and in the hook that reports them (PRD
/// agent-profiles D-03): the folder as the file system resolves it, links
/// followed, when it is there, else as written; without a final separator,
/// and on Windows in one case. Two spellings of one folder compare equal;
/// the stored spelling, which Claude keys its login by, is never changed by
/// this.
pub fn compared_folder(value: &str) -> String {
    let resolved = std::fs::canonicalize(value)
        .ok()
        .and_then(|path| path.into_os_string().into_string().ok())
        .unwrap_or_else(|| value.to_owned());
    let resolved = if cfg!(windows) {
        resolved
            .strip_prefix(r"\\?\")
            .unwrap_or(&resolved)
            .replace('/', "\\")
            .to_lowercase()
    } else {
        resolved
    };
    let trimmed = resolved.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        resolved
    } else {
        trimmed.to_owned()
    }
}

/// The account variables of the `kind` agent's `[account]` that are set in
/// its process, as `variable` reads them, each compared as
/// [`compared_folder`] does for a folder account and as written for a named
/// one: what a hook reports at a session's start, so the core reads the
/// pane in the account the agent runs in when the operator started it by
/// hand (PRD agent-profiles D-03, B24, B25). An empty list is the agent's
/// default account; `None` for an agent Hide keeps no accounts of.
///
/// Claude Code drops its variables from a hook's environment when
/// `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1`; its folder is then read from the
/// session's `CLAUDE_ENV_FILE`, which Claude Code keeps below the account's
/// folder ([`claude_env_file_folder`]).
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
        let value = read(&var.name).or_else(|| {
            (adapter.id == hide_agent_adapter::AgentId::ClaudeCode.adapter().id
                && read("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB").as_deref() == Some("1"))
            .then(|| read("CLAUDE_ENV_FILE"))
            .flatten()
            .and_then(|file| claude_env_file_folder(&file))
        });
        if let Some(value) = value {
            let value = match rule.by {
                By::Folder => compared_folder(&value),
                _ => value,
            };
            reported.push((var.name.clone(), value));
        }
    }
    Some(reported)
}

/// The account folder above a `CLAUDE_ENV_FILE`, which Claude Code writes at
/// `<folder>/session-env/<session>/…`: the folder above its `session-env`.
fn claude_env_file_folder(file: &str) -> Option<String> {
    let path = Path::new(file);
    path.ancestors()
        .find(|folder| folder.file_name().is_some_and(|name| name == "session-env"))
        .and_then(Path::parent)
        .filter(|folder| folder.is_absolute())
        .and_then(|folder| folder.to_str())
        .map(str::to_owned)
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
        let folder = tempfile::tempdir().unwrap();
        let spelled = format!("{}/", folder.path().display());
        let reported =
            reported_variables("claude-code", read(&[("CLAUDE_CONFIG_DIR", &spelled)])).unwrap();
        assert_eq!(
            reported,
            vec![(
                "CLAUDE_CONFIG_DIR".to_owned(),
                compared_folder(&folder.path().display().to_string())
            )]
        );
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
        let env_file = "/accounts/claude-a/session-env/s-1/hook-1.sh";
        let scrubbed = read(&[
            ("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB", "1"),
            ("CLAUDE_ENV_FILE", env_file),
        ]);
        assert_eq!(
            reported_variables("claude-code", scrubbed),
            Some(vec![(
                "CLAUDE_CONFIG_DIR".to_owned(),
                compared_folder("/accounts/claude-a")
            )])
        );
        // Without the scrub the file says nothing: the variable is absent
        // because the agent runs in its default account.
        assert_eq!(
            reported_variables("claude-code", read(&[("CLAUDE_ENV_FILE", env_file)])),
            Some(Vec::new())
        );
    }
}
