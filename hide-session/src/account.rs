//! The accounts a node reads an agent's sessions in (PRD agent-profiles
//! D-36; docs/ARCHITECTURE.md, Agent profiles and accounts).
//!
//! [`Accounts`] holds the node's home and, for every agent, the `sessions`
//! place (`hide_agent_adapter::resolve`) of each account the node registered:
//! the agent's built-in default account, and the account of each profile the
//! node's profile file holds. Every reader takes its root from here and from
//! nowhere else: a path a caller or a hook names is read only inside one of
//! these places, and a read for one agent's account names the account by its
//! key, which must be one of them ([`Accounts::reading`]).
//!
//! The default account follows only the variable the node captured and
//! checked at startup (`crate::environment`: OpenCode's `XDG_DATA_HOME`);
//! every other agent's default sessions are read at their default place
//! below the home. A profile's account is resolved as the node resolves it
//! for a start, for a node that honours no variable.

use std::path::{Path, PathBuf};

use hide_agent_adapter::{Account, AccountKey, AccountPath, AccountRef, AgentId, NodeEnv, Profile};

use crate::Agent;
use crate::refusal::ReadRefusal;

/// Why a read named an account the node did not register.
pub const ACCOUNT_UNREGISTERED: &str = "session_account_unregistered";

/// Why the accounts the node registered for an agent are not known to be all
/// it keeps: its profile file could not be read, or a profile's account
/// could not be registered.
pub const ACCOUNTS_UNSETTLED: &str = "session_accounts_unsettled";

/// The adapter of a supported agent; retired Cursor keeps no account.
fn adapter(agent: Agent) -> Option<&'static hide_agent_adapter::AgentAdapter> {
    let id = match agent {
        Agent::Claude => AgentId::ClaudeCode,
        Agent::Codex => AgentId::Codex,
        Agent::Grok => AgentId::Grok,
        Agent::Pi => AgentId::Pi,
        Agent::Omp => AgentId::Omp,
        Agent::OpenCode => AgentId::OpenCode,
        Agent::Antigravity => AgentId::Antigravity,
        Agent::Cursor => return None,
    };
    Some(id.adapter())
}

/// Where one account keeps one agent's sessions: `place`, a folder (or
/// OpenCode's database), below `anchor`, the folder a reader proves the path
/// down from with no link in between: the home, the account folder a profile
/// names, or the data folder the node's `XDG_DATA_HOME` names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Root {
    pub anchor: PathBuf,
    pub place: PathBuf,
}

impl Root {
    /// `place` relative to `anchor`.
    pub(crate) fn suffix(&self) -> &Path {
        self.place
            .strip_prefix(&self.anchor)
            .expect("an account's sessions place lies below its anchor")
    }
}

#[derive(Clone, Debug)]
struct Registered {
    agent: Agent,
    key: AccountKey,
    /// The account's place, or why it cannot be named (the node's own
    /// `XDG_DATA_HOME` is no usable path, for OpenCode's default).
    root: Result<Root, &'static str>,
    /// Every place of the account (`hide_agent_adapter::resolve`), when it
    /// could be resolved.
    places: Option<Account>,
}

/// The accounts a node reads sessions in; see the module documentation.
#[derive(Clone, Debug)]
pub struct Accounts {
    home: PathBuf,
    /// Each agent's accounts, its default account first.
    registered: Vec<Registered>,
    /// The agents whose registered accounts may not be all the node keeps
    /// ([`Self::default_alone`]).
    unsettled: Vec<Agent>,
}

impl Accounts {
    /// Every agent's built-in default account under `home`: what a node
    /// with no profile file reads.
    pub fn new(home: &Path) -> Self {
        let mut registered = Vec::new();
        for agent in Agent::supported() {
            let Some(adapter) = adapter(agent) else {
                continue;
            };
            let (root, places) = match default_root(home, agent, adapter) {
                Ok(None) => continue,
                Ok(Some((root, places))) => (Ok(root), Some(places)),
                Err(reason) => (Err(reason), None),
            };
            registered.push(Registered {
                agent,
                key: AccountKey::Default,
                root,
                places,
            });
        }
        Self {
            home: home.to_path_buf(),
            registered,
            unsettled: Vec::new(),
        }
    }

    /// These accounts, read for a node whose profile file could not be
    /// read: what it registered for every agent may not be all it keeps.
    #[must_use]
    pub fn unread(mut self) -> Self {
        self.unsettled = Agent::supported().collect();
        self
    }

    /// These accounts and the account of each of `profiles` that is not its
    /// agent's default: the node's profile file. A profile that no longer
    /// resolves, or one whose sessions lie outside the folder it names, adds
    /// nothing and leaves its agent's accounts unsettled
    /// ([`Self::default_alone`]); two profiles of one account add it once.
    pub fn with_profiles<'a>(mut self, profiles: impl IntoIterator<Item = &'a Profile>) -> Self {
        let node = NodeEnv::new(Some(self.home.clone()));
        for profile in profiles {
            if profile.account.is_none() {
                continue;
            }
            let Some(agent) = Agent::from_kind(&profile.kind) else {
                continue;
            };
            let Some(adapter) = adapter(agent) else {
                continue;
            };
            let account = match adapter.resolve(profile, &node) {
                Ok(account) => account,
                // A folder the agent's default account reads is that account.
                Err(hide_agent_adapter::Refusal::AccountIsDefault) => continue,
                Err(_) => {
                    self.unsettle(agent);
                    continue;
                }
            };
            if account.key.is_default()
                || self
                    .registered
                    .iter()
                    .any(|known| known.agent == agent && known.key == account.key)
            {
                continue;
            }
            let Some(place) = account.path(AccountPath::Sessions) else {
                self.unsettle(agent);
                continue;
            };
            let anchor = match &profile.account {
                Some(AccountRef::Folder(folder)) => PathBuf::from(folder),
                Some(AccountRef::Name(_)) | None => self.home.clone(),
            };
            if !place.starts_with(&anchor) || place == anchor {
                self.unsettle(agent);
                continue;
            }
            self.registered.push(Registered {
                agent,
                key: account.key.clone(),
                root: Ok(Root {
                    anchor,
                    place: place.to_path_buf(),
                }),
                places: Some(account),
            });
        }
        self
    }

    fn unsettle(&mut self, agent: Agent) {
        if !self.unsettled.contains(&agent) {
            self.unsettled.push(agent);
        }
    }

    /// Whether `agent`'s default account is the only one the node keeps for
    /// it: no profile of it names an account of its own, a profile being
    /// removed included, and a profile that runs in the default account
    /// adds none. Refused when the registration cannot tell, because the
    /// profile file was not read or a profile's account was not registered.
    pub fn default_alone(&self, agent: Agent) -> Result<bool, ReadRefusal> {
        if self.unsettled.contains(&agent) {
            return Err(ReadRefusal::from(ACCOUNTS_UNSETTLED));
        }
        Ok(self
            .registered
            .iter()
            .all(|known| known.agent != agent || known.key.is_default()))
    }

    /// The home of the node's account, which an agent that names a
    /// checkout's sessions by it reads (omp), and which retired Cursor and
    /// agy, whose accounts cannot move, keep their sessions under.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// These accounts with `agent` read in the account `key` alone: the
    /// read of one agent's session, which its start's account names.
    /// Refused when the node registered no such account (B31).
    pub fn reading(&self, agent: Agent, key: &AccountKey) -> Result<Self, ReadRefusal> {
        if key.is_default() {
            return Ok(self.only(agent, key));
        }
        if !self
            .registered
            .iter()
            .any(|known| known.agent == agent && known.key == *key)
        {
            return Err(ReadRefusal::from(ACCOUNT_UNREGISTERED));
        }
        Ok(self.only(agent, key))
    }

    fn only(&self, agent: Agent, key: &AccountKey) -> Self {
        Self {
            home: self.home.clone(),
            registered: self
                .registered
                .iter()
                .filter(|known| known.agent != agent || known.key == *key)
                .cloned()
                .collect(),
            unsettled: self.unsettled.clone(),
        }
    }

    /// The account a read of `agent` locates its session in: the one
    /// [`Self::reading`] chose, else the default. `Ok(None)` for an agent
    /// that keeps no account (Cursor); an error when its place cannot be
    /// named.
    pub(crate) fn root(&self, agent: Agent) -> Result<Option<&Root>, &'static str> {
        match self.registered.iter().find(|known| known.agent == agent) {
            Some(known) => known.root.as_ref().map(Some).map_err(|reason| *reason),
            None => Ok(None),
        }
    }

    /// Every account `agent` keeps sessions in that can be named, the
    /// default first: where a path a caller names may lie, and what a
    /// listing walks.
    pub(crate) fn roots(&self, agent: Agent) -> impl Iterator<Item = (&AccountKey, &Root)> {
        self.registered
            .iter()
            .filter(move |known| known.agent == agent)
            .filter_map(|known| known.root.as_ref().ok().map(|root| (&known.key, root)))
    }

    /// Each account `agent` keeps sessions in, as the accounts a read of it
    /// in that account alone sees ([`Self::reading`]), the default first:
    /// what a listing of a store kept per account (OpenCode's database)
    /// reads one by one.
    pub fn each(&self, agent: Agent) -> Vec<(AccountKey, Self)> {
        self.registered
            .iter()
            .filter(|known| known.agent == agent)
            .map(|known| (known.key.clone(), self.only(agent, &known.key)))
            .collect()
    }

    /// The place `path` of the account a read of `agent` reads in
    /// ([`Self::root`]), when the agent has one.
    pub(crate) fn place(&self, agent: Agent, path: AccountPath) -> Option<&Path> {
        self.registered
            .iter()
            .find(|known| known.agent == agent)
            .and_then(|known| known.places.as_ref())
            .and_then(|account| account.path(path))
    }

    /// The place `path` of every account of `agent` (Codex's archived
    /// sessions for usage), the default first.
    pub(crate) fn places(&self, agent: Agent, path: AccountPath) -> impl Iterator<Item = &Path> {
        self.registered
            .iter()
            .filter(move |known| known.agent == agent)
            .filter_map(move |known| known.places.as_ref()?.path(path))
    }

    /// The account whose sessions place of `agent` holds `path`, spelled
    /// from the account's folder as given or as resolved: the innermost
    /// when one account's folder lies inside another's place, the first
    /// registered of two with the same place.
    pub fn account_of(&self, agent: Agent, path: &Path) -> Option<&AccountKey> {
        self.innermost(agent, path).map(|(key, _)| key)
    }

    /// [`Self::account_of`] with the account's place.
    pub(crate) fn innermost(&self, agent: Agent, path: &Path) -> Option<(&AccountKey, &Root)> {
        let mut found: Option<(&AccountKey, &Root)> = None;
        for (key, root) in self.roots(agent).filter(|(_, root)| root.holds(path)) {
            let depth = root.place.components().count();
            if found.is_none_or(|(_, kept)| kept.place.components().count() < depth) {
                found = Some((key, root));
            }
        }
        found
    }
}

impl Root {
    /// Whether `path` names an entry strictly below the place, spelled from
    /// the anchor as given or as resolved; what is on the way is not looked
    /// at here.
    pub(crate) fn holds(&self, path: &Path) -> bool {
        let suffix = self.suffix();
        let below = |anchor: &Path| {
            hide_platform::path::strip_prefix(&anchor.join(suffix), path)
                .is_ok_and(|rest| !rest.as_os_str().is_empty())
        };
        below(&self.anchor)
            || hide_platform::fs::identity::canonical(&self.anchor)
                .is_ok_and(|anchor| below(&anchor))
    }
}

/// `agent`'s default account's sessions place under `home`, with every
/// place of the account: `Ok(None)` when the agent keeps no sessions, an
/// error when the node's own `XDG_DATA_HOME` is no usable path.
fn default_root(
    home: &Path,
    agent: Agent,
    adapter: &hide_agent_adapter::AgentAdapter,
) -> Result<Option<(Root, Account)>, &'static str> {
    let mut node = NodeEnv::new(Some(home.to_path_buf()));
    let mut anchor = home.to_path_buf();
    if agent == Agent::OpenCode {
        let data = crate::environment::data_home()?;
        node = node.with_variable(
            "XDG_DATA_HOME",
            data.map(|path| path.as_os_str().to_os_string()),
        );
        if let Some(data) = data {
            anchor = data.to_path_buf();
        }
    }
    let account = adapter
        .resolve(&Profile::builtin_default(adapter.id), &node)
        .expect("the node's checked variables and a known home leave nothing to refuse");
    let Some(place) = account.path(AccountPath::Sessions).map(Path::to_path_buf) else {
        return Ok(None);
    };
    Ok(Some((Root { anchor, place }, account)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(kind: &str, account: AccountRef) -> Profile {
        Profile {
            account: Some(account),
            ..Profile::builtin_default(kind)
        }
    }

    #[test]
    fn a_read_names_only_an_account_the_node_registered() {
        let home = std::env::temp_dir().join("hide-accounts-home");
        let other = std::env::temp_dir().join("hide-accounts-b");
        let b = profile(
            "claude-code",
            AccountRef::Folder(other.to_string_lossy().into_owned()),
        );
        let accounts = Accounts::new(&home).with_profiles([&b]);
        let key = AccountKey::Folder(other.to_string_lossy().into_owned());

        let reading = accounts.reading(Agent::Claude, &key).unwrap();
        assert_eq!(
            reading.root(Agent::Claude).unwrap().unwrap().place,
            other.join("projects")
        );
        assert_eq!(
            accounts.root(Agent::Claude).unwrap().unwrap().anchor,
            home,
            "the default account is read until a read names another"
        );
        let stranger = AccountKey::Folder(home.join("elsewhere").to_string_lossy().into_owned());
        assert_eq!(
            accounts
                .reading(Agent::Claude, &stranger)
                .unwrap_err()
                .reason(),
            ACCOUNT_UNREGISTERED
        );
        assert_eq!(
            accounts.reading(Agent::Codex, &key).unwrap_err().reason(),
            ACCOUNT_UNREGISTERED,
            "an account is registered for its own agent only"
        );
        assert_eq!(
            accounts.account_of(Agent::Claude, &other.join("projects/x/s.jsonl")),
            Some(&key)
        );
    }

    /// An account whose folder lies inside another's sessions place holds
    /// the sessions below its own place, whichever was registered first.
    #[test]
    fn a_path_belongs_to_the_innermost_account_that_holds_it() {
        let home = std::env::temp_dir().join("hide-accounts-home");
        let outer = std::env::temp_dir().join("hide-accounts-outer");
        let inner = outer.join("projects/inner");
        let folder = |path: &Path| AccountRef::Folder(path.to_string_lossy().into_owned());
        let key = |path: &Path| AccountKey::Folder(path.to_string_lossy().into_owned());
        let accounts = Accounts::new(&home).with_profiles([
            &profile("claude-code", folder(&outer)),
            &profile("claude-code", folder(&inner)),
        ]);
        let reversed = Accounts::new(&home).with_profiles([
            &profile("claude-code", folder(&inner)),
            &profile("claude-code", folder(&outer)),
        ]);

        for accounts in [&accounts, &reversed] {
            assert_eq!(
                accounts.account_of(Agent::Claude, &inner.join("projects/-w/s.jsonl")),
                Some(&key(&inner))
            );
            assert_eq!(
                accounts.account_of(Agent::Claude, &outer.join("projects/-w/s.jsonl")),
                Some(&key(&outer))
            );
            assert_eq!(
                accounts.account_of(Agent::Claude, &home.join(".claude/projects/-w/s.jsonl")),
                Some(&AccountKey::Default)
            );
            assert_eq!(
                accounts.account_of(Agent::Claude, &outer.join("projects")),
                None,
                "the place itself is no session"
            );
        }
    }
}
