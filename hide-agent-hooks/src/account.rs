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
