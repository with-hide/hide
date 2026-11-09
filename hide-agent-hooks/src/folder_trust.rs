//! The folder trust Hide records in an agent's own state for the worktrees it
//! makes. Claude Code, Codex, agy, Grok and Pi each ask, the first time they
//! start interactively in a folder they have not been told to trust, whether
//! to trust it, and a delegated child stopped at that question waits for an
//! answer nobody is there to give. So Hide records that trust before it
//! starts the agent in a worktree it made, and only there (docs/agent-hooks.md,
//! Folder trust for the worktrees Hide makes; PRD antigravity-first-class D-08
//! for agy).
//!
//! Each agent keeps its trust in its own file and format ([`Agent`]):
//!
//! - Claude Code: `projects["<folder>"].hasTrustDialogAccepted` in
//!   `~/.claude.json`, changed while Hide holds Claude's own lock on that file.
//! - Codex: `[projects."<folder>"] trust_level = "trusted"` in
//!   `~/.codex/config.toml`, written by Codex itself through its app-server
//!   (`crate::codex_trust`), so Hide never writes that file.
//! - agy: one entry of `trustedWorkspaces` in
//!   `~/.gemini/antigravity-cli/settings.json`.
//! - Grok: `[folders."<folder>"] trusted = true` in
//!   `~/.grok/trusted_folders.toml`, changed while Hide holds Grok's own lock
//!   on that file.
//! - Pi: `"<folder>": true` in `~/.pi/agent/trust.json`, changed while Hide
//!   holds Pi's own lock on that file.
//!
//! The entry names the worktree itself, never the repository it was made
//! from, for every agent that reads one: answering an agent's own question in
//! a worktree trusts the repository's main checkout, which is the operator's;
//! Claude Code 2.1.302 and Codex 0.163.0 both read an entry for the exact
//! folder before the repository's, and an entry for a folder above a
//! repository does not reach into it (measured 2026-10-17). Grok 1.0.67 reads
//! only the main checkout's entry for every worktree of a repository
//! (`workspace_key` in its folder trust), so Grok's entry names the main
//! checkout, and Hide keeps it while any worktree it trusted it for is there
//! (measured 2026-10-23).
//!
//! Hide remembers in its own record which entries it added, by the worktree
//! each was added for: removing the worktree, or switching the agent off,
//! takes out only those, and an entry the operator added, before Hide or after
//! it, stays. An entry for a folder
//! that is gone goes with the next entry Hide adds, so the record's cap is
//! only reached by folders that exist. Every change holds one account lock
//! per agent, waited for as long as the longest change holds it, and the JSON
//! files are rewritten through the same lossless read and compare-before-
//! replace as the hook files, so their other keys and the operator's
//! formatting are kept byte for byte.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use hide_agent_adapter::{
    Account, AccountKey, AccountPath, AgentAdapter, AgentId, LaunchDialect, NodeEnv, Profile,
};
use hide_platform::programs::Program;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::account::AccountAt;
use crate::install::{HookDocument, InstallFailure, read_document, write_document};
use crate::runtime::AgentRuntime;

/// How long Hide waits for Claude Code's own lock on its config, or Pi's on
/// its trust file, which each holds only while it reads or saves the file.
const LOCK_FOLDER_WAIT: Duration = Duration::from_secs(2);
/// How long Hide waits for Grok's own lock on its trust file, which Grok holds
/// only while it reads and replaces the file.
const GROK_LOCK_WAIT: Duration = Duration::from_secs(2);
/// The most folders that exist one agent's record keeps; a worktree is
/// removed long before.
const RECORD_LIMIT: usize = 4096;

/// An agent whose folder trust Hide records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Agent {
    ClaudeCode,
    Codex,
    Antigravity,
    Grok,
    Pi,
}

impl Agent {
    pub const ALL: [Self; 5] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::Antigravity,
        Self::Grok,
        Self::Pi,
    ];

    /// The agent a start of `dialect` records trust for, when it asks one
    /// (`LaunchDialect::trusts_worktrees`).
    pub fn of(dialect: LaunchDialect) -> Option<Self> {
        if !dialect.trusts_worktrees() {
            return None;
        }
        match dialect {
            LaunchDialect::Claude => Some(Self::ClaudeCode),
            LaunchDialect::Codex => Some(Self::Codex),
            LaunchDialect::Antigravity => Some(Self::Antigravity),
            LaunchDialect::Grok => Some(Self::Grok),
            LaunchDialect::Pi => Some(Self::Pi),
            _ => None,
        }
    }

    /// The agent's name in a reason the operator reads.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Antigravity => "agy",
            Self::Grok => "Grok",
            Self::Pi => "Pi",
        }
    }

    /// The file whose trust entries this agent reads in the account `at`
    /// (`AccountPath::FolderTrust`): for Claude Code and Codex the one their
    /// hooks' folder names (`AgentRuntime::trust_path`).
    pub fn settings_path<'a>(self, at: impl Into<AccountAt<'a>>) -> PathBuf {
        let at = at.into();
        match self {
            Self::ClaudeCode => AgentRuntime::ClaudeCode.trust_path(at),
            Self::Codex => AgentRuntime::Codex.trust_path(at),
            Self::Antigravity | Self::Grok | Self::Pi => {
                at.place(self.adapter(), AccountPath::FolderTrust)
            }
        }
    }

    fn adapter(self) -> &'static AgentAdapter {
        match self {
            Self::ClaudeCode => AgentId::ClaudeCode,
            Self::Codex => AgentId::Codex,
            Self::Antigravity => AgentId::Antigravity,
            Self::Grok => AgentId::Grok,
            Self::Pi => AgentId::Pi,
        }
        .adapter()
    }

    /// The longest one change holds Hide's lock for this agent: a Codex
    /// session's whole bound, or Claude Code's wait for its own lock, with a
    /// second for Hide's files around it.
    fn longest_hold(self) -> Duration {
        let own = match self {
            Self::Codex => crate::codex_trust::SESSION_BOUND,
            Self::ClaudeCode | Self::Pi => LOCK_FOLDER_WAIT,
            Self::Grok => GROK_LOCK_WAIT,
            Self::Antigravity => Duration::ZERO,
        };
        own + Duration::from_secs(1)
    }

    /// The longest a [`trust`] or [`forget`] of this agent takes: the wait for
    /// a change that already holds the lock, then its own.
    pub fn longest_change(self) -> Duration {
        2 * self.longest_hold()
    }

    fn file_stem(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-trust",
            Self::Codex => "codex-trust",
            Self::Antigravity => "antigravity-trust",
            Self::Grok => "grok-trust",
            Self::Pi => "pi-trust",
        }
    }

    fn record_path(self, home: &Path) -> PathBuf {
        agent_hooks_dir(home).join(format!("{}.json", self.file_stem()))
    }

    fn lock_path(self, home: &Path) -> PathBuf {
        agent_hooks_dir(home).join(format!("{}.lock", self.file_stem()))
    }

    /// The spelling the agent compares a folder by. Claude Code writes its
    /// keys with `/` on Windows too; Codex compares them without case there
    /// and writes them in lower case (`project_trust_key`, codex-rs 0.163.0);
    /// Grok writes the resolved path as the system spells it, without the
    /// `\\?\` prefix on Windows (`dunce::canonicalize`), which is how a
    /// worktree reaches here.
    fn spelling(self, path: &Path) -> Result<String, InstallFailure> {
        let spelled = path
            .to_str()
            .filter(|_| path.is_absolute())
            .ok_or_else(|| InstallFailure::UnexpectedShape {
                path: path.display().to_string(),
                detail: format!(
                    "a folder {} trusts must be an absolute UTF-8 path",
                    self.label()
                ),
            })?;
        Ok(match self {
            Self::ClaudeCode if cfg!(windows) => spelled.replace('\\', "/"),
            Self::Codex if cfg!(windows) => spelled.to_lowercase(),
            _ => spelled.to_owned(),
        })
    }
}

/// What a change may start: the `codex` program whose app-server records
/// Codex's trust, and the flag that ends it, or a wait for an agent's own
/// lock, when Hide quits. Claude Code's, agy's, Grok's and Pi's changes
/// start nothing.
#[derive(Clone, Copy)]
pub struct Tools<'a> {
    pub codex: Option<&'a Program>,
    pub stop: &'a AtomicBool,
}

impl Tools<'static> {
    /// For a change that starts nothing: Claude Code's, agy's, Grok's and Pi's.
    pub fn none() -> Self {
        static NEVER: AtomicBool = AtomicBool::new(false);
        Self {
            codex: None,
            stop: &NEVER,
        }
    }
}

fn agent_hooks_dir(home: &Path) -> PathBuf {
    home.join(".hide").join("agent-hooks")
}

/// Hide's lock for `agent`'s trust, waited for as long as the change holding
/// it can take, so a second start while Codex records the first one's trust
/// waits for it instead of failing.
fn hold(
    agent: Agent,
    home: &Path,
    stop: &AtomicBool,
) -> Result<hide_platform::fs::lock::Lock, InstallFailure> {
    let path = agent.lock_path(home);
    let failure = |detail: String| InstallFailure::NotWritable {
        path: path.display().to_string(),
        detail,
    };
    if let Some(parent) = path.parent() {
        hide_platform::fs::private::create_dir_all(parent)
            .map_err(|error| failure(error.to_string()))?;
    }
    let file = hide_platform::fs::private::open_or_create_file(&path)
        .map_err(|error| failure(error.to_string()))?;
    match hide_platform::fs::lock::lock_file(
        file,
        hide_platform::fs::lock::Mode::Exclusive,
        agent.longest_hold(),
        &|| stop.load(Ordering::Relaxed),
    )
    .map_err(|error| failure(error.to_string()))?
    {
        hide_platform::fs::lock::Waited::Locked(held) => Ok(held),
        hide_platform::fs::lock::Waited::TimedOut => Err(failure(format!(
            "another change of {}'s trust held the lock for more than {} s",
            agent.label(),
            agent.longest_hold().as_secs()
        ))),
        hide_platform::fs::lock::Waited::Cancelled => Err(failure("Hide is stopping".to_owned())),
    }
}

/// One folder Hide trusted for an agent, as its record keeps it: the folder,
/// the entry it put in the agent's file when that names another folder, and
/// the account whose file holds it when that is not the agent's default
/// (PRD agent-profiles B19). Grok's entry names the repository's main
/// checkout, shared by every worktree of it, so the entry leaves only with
/// the last of them in its account.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Recorded {
    /// In a profile's account, which an earlier build, reading it as
    /// `Through`, takes for the default's. Read first: the forms below would
    /// match it with its account dropped.
    InAccount {
        folder: String,
        entry: String,
        account: AccountKey,
    },
    /// The entry is the folder itself, the form every record held before
    /// Grok's.
    Folder(String),
    Through {
        folder: String,
        entry: String,
    },
}

/// The default account, which a record entry without one is in.
const DEFAULT_ACCOUNT: AccountKey = AccountKey::Default;

impl Recorded {
    fn new(folder: String, entry: String, account: AccountKey) -> Self {
        if !account.is_default() {
            Self::InAccount {
                folder,
                entry,
                account,
            }
        } else if folder == entry {
            Self::Folder(folder)
        } else {
            Self::Through { folder, entry }
        }
    }

    fn folder(&self) -> &str {
        match self {
            Self::Folder(folder)
            | Self::Through { folder, .. }
            | Self::InAccount { folder, .. } => folder,
        }
    }

    fn entry(&self) -> &str {
        match self {
            Self::Folder(entry) | Self::Through { entry, .. } | Self::InAccount { entry, .. } => {
                entry
            }
        }
    }

    fn account(&self) -> &AccountKey {
        match self {
            Self::InAccount { account, .. } => account,
            Self::Folder(_) | Self::Through { .. } => &DEFAULT_ACCOUNT,
        }
    }

    /// Whether `other` puts the same entry in the same account's file.
    fn same_entry(&self, other: &Self) -> bool {
        self.entry() == other.entry() && self.account() == other.account()
    }
}

/// The entries of `leaving` in `account` that no record in `staying` still
/// needs there, each once.
fn entries_to_take_out<'a>(
    account: &AccountKey,
    leaving: impl IntoIterator<Item = &'a Recorded>,
    staying: &[&Recorded],
) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for recorded in leaving {
        let entry = recorded.entry();
        if recorded.account() == account
            && !staying.iter().any(|kept| kept.same_entry(recorded))
            && !entries.iter().any(|taken| taken == entry)
        {
            entries.push(entry.to_owned());
        }
    }
    entries
}

/// The account a record entry's key names, resolved as the node resolves a
/// profile's for a start: `None` for the agent's default account, which is
/// found under the home.
fn account_of(
    agent: Agent,
    home: &Path,
    key: &AccountKey,
) -> Result<Option<Account>, InstallFailure> {
    let Some(spelled) = key.account() else {
        return Ok(None);
    };
    let adapter = agent.adapter();
    let profile = Profile {
        account: Some(spelled),
        ..Profile::builtin_default(adapter.id)
    };
    adapter
        .resolve(&profile, &NodeEnv::new(Some(home.to_path_buf())))
        .map(Some)
        .map_err(|refusal| InstallFailure::UnexpectedShape {
            path: home.display().to_string(),
            detail: format!(
                "the account {key:?} {} trusted a folder in no longer resolves: {refusal}",
                agent.label()
            ),
        })
}

/// The account `at`'s key.
fn key_of(at: AccountAt<'_>) -> AccountKey {
    match at {
        AccountAt::Home(_) => AccountKey::Default,
        AccountAt::Account { account, .. } => account.key.clone(),
    }
}

fn read_record(agent: Agent, home: &Path) -> Result<Vec<Recorded>, InstallFailure> {
    let path = agent.record_path(home);
    let unreadable = |detail: String| InstallFailure::Unreadable {
        path: path.display().to_string(),
        detail,
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(unreadable(error.to_string())),
    }
}

fn write_record(agent: Agent, home: &Path, record: &[Recorded]) -> Result<(), InstallFailure> {
    let path = agent.record_path(home);
    let failure = |detail: String| InstallFailure::NotWritable {
        path: path.display().to_string(),
        detail,
    };
    let bytes = serde_json::to_vec(record).map_err(|error| failure(error.to_string()))?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|error| failure(error.to_string()))
}

/// What adding an entry found.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Added {
    /// Hide wrote the entry.
    New,
    /// The agent already decides the folder (trusted, or Pi's saved "Do not
    /// trust"), so the entry stays the operator's and Hide records nothing.
    Present,
}

/// Records that `agent` may trust `folder`, spelled as given, without asking,
/// in the account `at`: the agent's default account under a home, or a
/// profile's. A folder the agent already trusts is left as it is and stays
/// the operator's. Hide's record of it is kept under the account's home.
pub fn trust<'a>(
    agent: Agent,
    at: impl Into<AccountAt<'a>>,
    folder: &Path,
    tools: Tools,
) -> Result<(), InstallFailure> {
    let at = at.into();
    let home = at.home();
    let account = key_of(at);
    let settings = agent.settings_path(at);
    let folder_key = agent.spelling(folder)?;
    let repository = match agent {
        Agent::Codex | Agent::Grok => main_checkout(folder)
            .map(|root| agent.spelling(&root))
            .transpose()?,
        _ => None,
    };
    let key = match agent {
        Agent::Grok => repository.clone().ok_or_else(|| InstallFailure::UnexpectedShape {
            path: folder.display().to_string(),
            detail: "a worktree Grok trusts belongs to a repository with a main checkout, the folder Grok keeps a worktree's trust under".to_owned(),
        })?,
        _ => folder_key.clone(),
    };
    let new = Recorded::new(folder_key, key.clone(), account.clone());
    let _lock = hold(agent, home, tools.stop)?;
    let mut record = read_record(agent, home)?;
    // Hide's entries in this account for folders that are gone (removed
    // outside Hide, or by a removal that could not reach the agent) go in the
    // same write as the new one, so the cap is only reached by folders that
    // exist.
    let gone: Vec<Recorded> = record
        .iter()
        .filter(|recorded| {
            recorded.account() == &account
                && recorded.folder() != new.folder()
                && folder_is_gone(recorded.folder())
        })
        .cloned()
        .collect();
    let staying: Vec<&Recorded> = record
        .iter()
        .filter(|recorded| !gone.contains(recorded))
        .chain([&new])
        .collect();
    let gone_entries = entries_to_take_out(&account, &gone, &staying);
    // An entry another worktree of Hide's already needs in this account is
    // Hide's too, so this folder is recorded with it whatever the agent's
    // file says.
    let shared = record.iter().any(|recorded| {
        recorded.same_entry(&new) && recorded.folder() != new.folder() && !gone.contains(recorded)
    });
    let mut added = false;
    // The record is written first, so a crash between the two writes leaves a
    // recorded folder the agent does not trust, which a removal skips. A
    // write that fails takes the folder back out of the record: left there,
    // it would make a trust the operator gives later look like Hide's.
    let written = {
        let mut remember = || {
            if !record
                .iter()
                .any(|recorded| recorded.folder() == new.folder() && recorded.account() == &account)
            {
                if record.len() - gone.len() >= RECORD_LIMIT {
                    return Err(InstallFailure::NotWritable {
                        path: agent.record_path(home).display().to_string(),
                        detail: format!(
                            "Hide's record of {} trust already holds {RECORD_LIMIT} folders that exist",
                            agent.label()
                        ),
                    });
                }
                record.push(new.clone());
                write_record(agent, home, &record)?;
                added = true;
            }
            Ok(())
        };
        let shared_remembered = if shared { remember() } else { Ok(()) };
        shared_remembered.and_then(|()| match agent {
            Agent::ClaudeCode => claude::add(&settings, &key, &gone_entries, &mut remember),
            Agent::Codex => crate::codex_trust::add_project_trust(
                codex_program(&settings, tools)?,
                at,
                tools.stop,
                &key,
                repository.as_deref(),
                &gone_entries,
                &mut remember,
            ),
            Agent::Antigravity => antigravity::add(&settings, &key, &gone_entries, &mut remember),
            Agent::Grok => grok::add(&settings, &key, &gone_entries, tools.stop, &mut remember),
            Agent::Pi => pi::add(&settings, &key, &gone_entries, &mut remember),
        })
    };
    match written {
        Ok(Added::New) if !gone.is_empty() => {
            record.retain(|recorded| !gone.contains(recorded));
            // The agent no longer trusts them; an entry still recorded is
            // taken out of the record with the next one Hide adds.
            let _ = write_record(agent, home, &record);
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(failure) => {
            if added {
                record.retain(|recorded| recorded != &new);
                // The agent's failure is the one reported; a record left
                // behind is the crash case above.
                let _ = write_record(agent, home, &record);
            }
            Err(failure)
        }
    }
}

/// Whether the folder a record entry names is gone. A folder that cannot be
/// looked at is not taken for gone.
fn folder_is_gone(entry: &str) -> bool {
    matches!(Path::new(entry).try_exists(), Ok(false))
}

/// [`trust`] for a worktree Hide made for a start of `agent`, refused unless
/// the folder is one: a directory with a `.git` entry, named without `.` or
/// `..`, and neither the filesystem root, the home folder nor a folder above
/// it. Trust makes the agent load the folder's own settings and hooks
/// without asking, so a path that is not such a worktree is never trusted,
/// whoever asks.
pub fn trust_worktree<'a>(
    agent: Agent,
    at: impl Into<AccountAt<'a>>,
    folder: &Path,
    tools: Tools,
) -> Result<(), InstallFailure> {
    let at = at.into();
    let home = at.home();
    let refuse = |detail: &str| InstallFailure::UnexpectedShape {
        path: folder.display().to_string(),
        detail: format!("a worktree {} trusts {detail}", agent.label()),
    };
    // Read from the spelling itself: `components()` drops a `.` that is not
    // the first name, and an agent matches the entry as it is written.
    if folder
        .to_string_lossy()
        .split(['/', std::path::MAIN_SEPARATOR])
        .any(|name| name == "." || name == "..")
    {
        return Err(refuse("is named without . or .."));
    }
    let canonical = hide_platform::fs::identity::canonical(folder)
        .map_err(|_| refuse("names a folder that exists"))?;
    if !canonical.is_dir() || std::fs::symlink_metadata(canonical.join(".git")).is_err() {
        return Err(refuse("is a Git worktree"));
    }
    let account =
        hide_platform::fs::identity::canonical(home).unwrap_or_else(|_| home.to_path_buf());
    if canonical.parent().is_none() || account.starts_with(&canonical) {
        return Err(refuse(
            "is never the root, the home folder or a folder above it",
        ));
    }
    // The entry is the folder's resolved path, so a link along the way cannot
    // be pointed elsewhere after it is trusted, and `//` or a trailing
    // separator never reaches it; [`forget`] takes it out by either spelling.
    trust(agent, at, &canonical, tools)
}

/// The main checkout of the repository `worktree` belongs to, which Codex
/// reads a worktree's trust from when it has none of its own
/// (`resolve_root_git_project_for_trust`) and Grok reads it from always
/// (`workspace_key`): the folder holding the common `.git`.
fn main_checkout(worktree: &Path) -> Option<PathBuf> {
    let repository = hide_project::git::discover(worktree)?;
    let common = repository.common_dir;
    if common.file_name()? != ".git" {
        return None;
    }
    hide_platform::fs::identity::canonical(common.parent()?).ok()
}

fn codex_program<'a>(settings: &Path, tools: Tools<'a>) -> Result<&'a Program, InstallFailure> {
    tools.codex.ok_or_else(|| InstallFailure::NotWritable {
        path: settings.display().to_string(),
        detail: "no codex program was found to record Codex's trust through".to_owned(),
    })
}

/// Takes out the trust Hide added for `folder`, as when Hide removes the
/// worktree; a folder Hide did not add is left alone, and so is an entry
/// another folder Hide trusted still needs (Grok's main checkout). The folder
/// may already be gone and named as Git lists it: Hide's record holds the key
/// it wrote, and each entry there is compared with the folder as the system
/// compares folders. Answers how many entries left the agent's settings.
pub fn forget(
    agent: Agent,
    home: &Path,
    folder: &Path,
    tools: Tools,
) -> Result<usize, InstallFailure> {
    agent.spelling(folder)?;
    let names = folder_names(folder);
    forget_where(agent, home, tools, |recorded| {
        names
            .iter()
            .any(|name| same_folder(recorded.folder(), name))
    })
}

/// The ways `folder` may have been recorded: as given, and resolved, with
/// only its parent resolved once the folder itself is gone.
fn folder_names(folder: &Path) -> Vec<PathBuf> {
    let resolved = hide_platform::fs::identity::canonical(folder)
        .ok()
        .or_else(|| {
            let parent = hide_platform::fs::identity::canonical(folder.parent()?).ok()?;
            Some(parent.join(folder.file_name()?))
        });
    std::iter::once(folder.to_path_buf())
        .chain(resolved)
        .collect()
}

/// Whether a record entry names `path`, as the system compares folders: on
/// Windows without case and with either separator, which covers each agent's
/// spelling there (`Agent::spelling`), and exactly elsewhere.
fn same_folder(entry: &str, path: &Path) -> bool {
    let Some(path) = path.to_str() else {
        return false;
    };
    let trim = |name: &str| {
        name.trim_end_matches(['/', std::path::MAIN_SEPARATOR])
            .to_owned()
    };
    if cfg!(windows) {
        let fold = |name: &str| trim(name).replace('/', "\\").to_lowercase();
        fold(entry) == fold(path)
    } else {
        trim(entry) == trim(path)
    }
}

/// Takes out every trust Hide added for `agent` in its default account under
/// `home`, as when it is switched off there. A profile account's trust goes
/// with that account's pieces ([`forget_account`]), once the kit has judged
/// the account's file Hide's to write (docs/agent-hooks.md, Profile
/// accounts).
pub fn forget_all(agent: Agent, home: &Path, tools: Tools) -> Result<usize, InstallFailure> {
    forget_where(agent, home, tools, |recorded| {
        recorded.account().is_default()
    })
}

/// Takes out every trust Hide added for `agent` in the profile account
/// `account`, as when the last profile in it goes (docs/agent-hooks.md,
/// Profile accounts). An entry recorded under another spelling of the same
/// account (a final `/`, a link to it) is found by the agent's own folder
/// there, as the file system names it.
pub fn forget_account(
    agent: Agent,
    home: &Path,
    account: &Account,
    tools: Tools,
) -> Result<usize, InstallFailure> {
    let real = |account: &Account| {
        account
            .path(AccountPath::State)
            .and_then(|state| hide_platform::fs::identity::canonical(state).ok())
    };
    let going = real(account);
    forget_where(agent, home, tools, |recorded| {
        let key = recorded.account();
        !key.is_default()
            && (key == &account.key
                || going.is_some()
                    && account_of(agent, home, key)
                        .ok()
                        .flatten()
                        .as_ref()
                        .and_then(real)
                        == going)
    })
}

/// Takes out the trust Hide added for the record entries `chosen` picks, each
/// in its own account's file, unless another entry kept there still needs it.
fn forget_where(
    agent: Agent,
    home: &Path,
    tools: Tools,
    chosen: impl Fn(&Recorded) -> bool,
) -> Result<usize, InstallFailure> {
    // A machine where Hide never trusted a folder has no record and needs
    // no lock, so a removal there writes nothing of the agent's or Hide's.
    let record_path = agent.record_path(home);
    if !record_path
        .try_exists()
        .map_err(|error| InstallFailure::Unreadable {
            path: record_path.display().to_string(),
            detail: error.to_string(),
        })?
    {
        return Ok(0);
    }
    let _lock = hold(agent, home, tools.stop)?;
    let record = read_record(agent, home)?;
    let (gone, kept): (Vec<_>, Vec<_>) = record.into_iter().partition(|recorded| chosen(recorded));
    if gone.is_empty() {
        return Ok(0);
    }
    let staying: Vec<&Recorded> = kept.iter().collect();
    let mut accounts: Vec<&AccountKey> = Vec::new();
    for recorded in &gone {
        if !accounts.contains(&recorded.account()) {
            accounts.push(recorded.account());
        }
    }
    let mut removed = 0;
    for account in accounts {
        let entries = entries_to_take_out(account, &gone, &staying);
        if entries.is_empty() {
            continue;
        }
        let resolved = account_of(agent, home, account)?;
        let at = match &resolved {
            Some(account) => AccountAt::Account { home, account },
            None => AccountAt::Home(home),
        };
        let settings = agent.settings_path(at);
        removed += match agent {
            Agent::ClaudeCode => claude::remove(&settings, &entries)?,
            Agent::Codex => crate::codex_trust::remove_project_trust(
                codex_program(&settings, tools)?,
                at,
                tools.stop,
                &entries,
            )?,
            Agent::Antigravity => antigravity::remove(&settings, &entries)?,
            Agent::Grok => grok::remove(&settings, &entries, tools.stop)?,
            Agent::Pi => pi::remove(&settings, &entries)?,
        };
    }
    write_record(agent, home, &kept)?;
    Ok(removed)
}

/// The object under `key` of `parent`, made when it has none.
fn object_entry<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
    path: &Path,
    shape: &str,
) -> Result<&'a mut Map<String, Value>, InstallFailure> {
    parent
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| InstallFailure::UnexpectedShape {
            path: path.display().to_string(),
            detail: shape.to_owned(),
        })
}

fn root_object<'a>(
    document: &'a mut HookDocument,
    path: &Path,
    agent: Agent,
) -> Result<&'a mut Map<String, Value>, InstallFailure> {
    document
        .as_object_mut()
        .ok_or_else(|| InstallFailure::UnexpectedShape {
            path: path.display().to_string(),
            detail: format!("{}'s settings are not a JSON object", agent.label()),
        })
}

/// agy's trust list: `trustedWorkspaces`, one exact path per entry.
mod antigravity {
    use super::*;

    const KEY: &str = "trustedWorkspaces";

    fn entries<'a>(
        document: &'a mut HookDocument,
        settings: &Path,
    ) -> Result<&'a mut Vec<Value>, InstallFailure> {
        root_object(document, settings, Agent::Antigravity)?
            .entry(KEY)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| InstallFailure::UnexpectedShape {
                path: settings.display().to_string(),
                detail: "trustedWorkspaces is not a list".to_owned(),
            })
    }

    /// Adds `key` unless agy trusts it already, and takes out the entries in
    /// `gone` in the same write.
    pub(super) fn add(
        settings: &Path,
        key: &str,
        gone: &[String],
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let mut document = read_document(settings)?
            .unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
        let list = entries(&mut document, settings)?;
        if list.iter().any(|entry| entry.as_str() == Some(key)) {
            return Ok(Added::Present);
        }
        remember()?;
        take_out(list, gone);
        list.push(Value::String(key.to_owned()));
        write_document(settings, &document).map(|()| Added::New)
    }

    pub(super) fn remove(settings: &Path, gone: &[String]) -> Result<usize, InstallFailure> {
        let Some(mut document) = read_document(settings)? else {
            return Ok(0);
        };
        let removed = take_out(entries(&mut document, settings)?, gone);
        if removed > 0 {
            write_document(settings, &document)?;
        }
        Ok(removed)
    }

    fn take_out(list: &mut Vec<Value>, gone: &[String]) -> usize {
        let before = list.len();
        list.retain(|entry| {
            entry
                .as_str()
                .is_none_or(|entry| !gone.iter().any(|g| g == entry))
        });
        before - list.len()
    }
}

/// Grok's trust: `[folders."<folder>"] trusted = true` in
/// `trusted_folders.toml` of Grok's default home, the file Grok 1.0.67 reads
/// and writes as a serde document of `folders` alone (`TrustStore`). Grok
/// takes an exclusive lock on `trusted_folders.toml.lock` beside it (`fs2`,
/// `flock` on Unix and `LockFileEx` on Windows) and reads the file again
/// before each write of its own, so Hide holds the same lock across its read
/// and its write and neither drops the other's change.
mod grok {
    use toml_edit::{DocumentMut, Item, Table, TableLike, value};

    use super::*;

    const FOLDERS: &str = "folders";
    const TRUSTED: &str = "trusted";
    const DECIDED_AT: &str = "decided_at";

    /// Grok's lock on its trust file, held until dropped.
    fn take_lock(
        settings: &Path,
        stop: &AtomicBool,
    ) -> Result<hide_platform::fs::lock::Lock, InstallFailure> {
        let path = settings.with_extension("toml.lock");
        let failure = |detail: String| InstallFailure::NotWritable {
            path: settings.display().to_string(),
            detail,
        };
        if let Some(parent) = path.parent() {
            hide_platform::fs::private::create_dir_all(parent)
                .map_err(|error| failure(error.to_string()))?;
        }
        let file = hide_platform::fs::private::open_or_create_file(&path)
            .map_err(|error| failure(error.to_string()))?;
        match hide_platform::fs::lock::lock_file(
            file,
            hide_platform::fs::lock::Mode::Exclusive,
            GROK_LOCK_WAIT,
            &|| stop.load(Ordering::Relaxed),
        )
        .map_err(|error| failure(error.to_string()))?
        {
            hide_platform::fs::lock::Waited::Locked(held) => Ok(held),
            hide_platform::fs::lock::Waited::TimedOut => Err(failure(format!(
                "Grok held its trust lock ({}) for more than {} s",
                path.display(),
                GROK_LOCK_WAIT.as_secs()
            ))),
            hide_platform::fs::lock::Waited::Cancelled => {
                Err(failure("Hide is stopping".to_owned()))
            }
        }
    }

    /// The file Grok reads, through a link the operator made, and what it
    /// holds; `None` when there is none.
    fn read(settings: &Path) -> Result<(PathBuf, Option<DocumentMut>), InstallFailure> {
        let real = hide_platform::fs::identity::canonical(settings)
            .unwrap_or_else(|_| settings.to_path_buf());
        let text = match std::fs::read_to_string(&real) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((real, None)),
            Err(error) => {
                return Err(InstallFailure::Unreadable {
                    path: settings.display().to_string(),
                    detail: error.to_string(),
                });
            }
        };
        let document =
            text.parse::<DocumentMut>()
                .map_err(|error| InstallFailure::UnexpectedShape {
                    path: settings.display().to_string(),
                    detail: format!("it is not valid TOML ({error}); Hide left it untouched"),
                })?;
        Ok((real, Some(document)))
    }

    fn write(settings: &Path, real: &Path, document: &DocumentMut) -> Result<(), InstallFailure> {
        hide_platform::fs::atomic::write_file(
            real,
            document.to_string().as_bytes(),
            hide_platform::fs::Access::Private,
        )
        .map(drop)
        .map_err(|error| InstallFailure::NotWritable {
            path: settings.display().to_string(),
            detail: error.to_string(),
        })
    }

    fn folders<'a>(
        document: &'a mut DocumentMut,
        settings: &Path,
    ) -> Result<&'a mut dyn TableLike, InstallFailure> {
        document
            .entry(FOLDERS)
            .or_insert_with(|| {
                let mut folders = Table::new();
                folders.set_implicit(true);
                Item::Table(folders)
            })
            .as_table_like_mut()
            .ok_or_else(|| InstallFailure::UnexpectedShape {
                path: settings.display().to_string(),
                detail: "folders is not a table".to_owned(),
            })
    }

    /// What Grok decided for `key`: trusted, untrusted, or nothing.
    fn decision(folders: &dyn TableLike, key: &str) -> Option<bool> {
        folders
            .get(key)
            .and_then(Item::as_table_like)
            .map(|entry| entry.get(TRUSTED).and_then(Item::as_bool) == Some(true))
    }

    /// Records `key` as trusted unless Grok already decided it, and takes out
    /// the entries in `gone` in the same write. A folder the operator answered
    /// "No" for in Grok is refused, never overridden.
    pub(super) fn add(
        settings: &Path,
        key: &str,
        gone: &[String],
        stop: &AtomicBool,
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let _lock = take_lock(settings, stop)?;
        let (real, document) = read(settings)?;
        let mut document = document.unwrap_or_default();
        let folders = folders(&mut document, settings)?;
        match decision(folders, key) {
            Some(true) => return Ok(Added::Present),
            Some(false) => {
                return Err(InstallFailure::Refused {
                    path: settings.display().to_string(),
                    detail: format!(
                        "Grok marks {key} untrusted, the repository this worktree was made from; trust it in Grok, or start the agent there yourself"
                    ),
                });
            }
            None => {}
        }
        remember()?;
        take_out(folders, gone);
        let mut entry = Table::new();
        entry.insert(TRUSTED, value(true));
        // Optional in Grok's file, and left out as Grok leaves it out when the
        // clock cannot say.
        if let Some(now) = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        {
            entry.insert(DECIDED_AT, value(now));
        }
        folders.insert(key, Item::Table(entry));
        write(settings, &real, &document).map(|()| Added::New)
    }

    pub(super) fn remove(
        settings: &Path,
        gone: &[String],
        stop: &AtomicBool,
    ) -> Result<usize, InstallFailure> {
        if !settings.exists() {
            return Ok(0);
        }
        let _lock = take_lock(settings, stop)?;
        let (real, Some(mut document)) = read(settings)? else {
            return Ok(0);
        };
        let removed = take_out(folders(&mut document, settings)?, gone);
        if removed > 0 {
            write(settings, &real, &document)?;
        }
        Ok(removed)
    }

    /// Takes out each entry in `gone` that Grok still trusts; one the
    /// operator has since answered "No" for stays theirs.
    fn take_out(folders: &mut dyn TableLike, gone: &[String]) -> usize {
        let mut removed = 0;
        for key in gone {
            if decision(folders, key) == Some(true) {
                folders.remove(key);
                removed += 1;
            }
        }
        removed
    }
}

/// proper-lockfile's lock, which Claude Code and Pi take on a file before
/// they read it again and write it: a folder beside the file, made to take
/// the lock and removed to release it. Hide makes the same folder across its
/// own read and write of the file, so neither drops the other's change.
mod lock_folder {
    use std::time::SystemTime;

    use hide_platform::fs::identity::FileId;

    use super::*;

    /// proper-lockfile's default: a lock folder not touched for this long was
    /// left by a process that ended, and is taken over.
    const STALE: Duration = Duration::from_secs(10);
    const POLL: Duration = Duration::from_millis(50);

    /// Which lock folder a path names: its identity and its time, the two
    /// together because a filesystem may give a new folder a freed number.
    type Mark = (FileId, SystemTime);

    fn mark(path: &Path) -> Option<Mark> {
        let id = hide_platform::fs::identity::file_id_nofollow(path).ok()?;
        let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
        Some((id, modified))
    }

    /// The agent's lock on `file`, held until dropped.
    pub(super) struct LockFolder {
        path: PathBuf,
        file: PathBuf,
        owner: &'static str,
        /// The folder Hide made. A hold past [`STALE`] lets the agent take
        /// the lock over, and then the folder there is the agent's.
        made: Mark,
    }

    impl LockFolder {
        /// Makes the lock folder `path` for `file`, waiting up to
        /// [`LOCK_FOLDER_WAIT`] while `owner` (the agent) holds it.
        // Another process's lock folder announces nothing when it goes.
        #[allow(clippy::disallowed_methods)] // a production wait, not test code
        pub(super) fn take(
            path: PathBuf,
            file: &Path,
            owner: &'static str,
        ) -> Result<Self, InstallFailure> {
            let failure = |detail: String| InstallFailure::NotWritable {
                path: file.display().to_string(),
                detail,
            };
            let deadline = Instant::now() + LOCK_FOLDER_WAIT;
            loop {
                match std::fs::create_dir(&path) {
                    Ok(()) => {
                        let Some(made) = mark(&path) else {
                            let _ = std::fs::remove_dir(&path);
                            return Err(failure(format!(
                                "the lock folder Hide made ({}) could not be read",
                                path.display()
                            )));
                        };
                        return Ok(Self {
                            path,
                            file: file.to_path_buf(),
                            owner,
                            made,
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(failure(error.to_string())),
                }
                let seen = mark(&path);
                let stale = seen
                    .is_some_and(|(_, modified)| modified.elapsed().is_ok_and(|age| age > STALE));
                if stale {
                    // Taken over as proper-lockfile takes one over: only the
                    // folder seen stale, then one more try to make it, so a
                    // lock made meanwhile is waited for. Hide's own changes
                    // never meet here, since each holds Hide's lock first, so
                    // the only other taker is the agent, whose own takeover
                    // leaves the same moment between the look and the removal.
                    if mark(&path) == seen {
                        let _ = std::fs::remove_dir(&path);
                    }
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(failure(format!(
                        "{owner} held its lock ({}) for more than {} s",
                        path.display(),
                        LOCK_FOLDER_WAIT.as_secs()
                    )));
                }
                std::thread::sleep(POLL);
            }
        }

        /// Whether the lock folder is still the one Hide made.
        pub(super) fn held(&self) -> bool {
            mark(&self.path) == Some(self.made)
        }

        /// Refuses a write once the agent took the lock over.
        pub(super) fn still_held(&self) -> Result<(), InstallFailure> {
            if self.held() {
                return Ok(());
            }
            Err(InstallFailure::NotWritable {
                path: self.file.display().to_string(),
                detail: format!("{} took its lock over while Hide held it", self.owner),
            })
        }
    }

    impl Drop for LockFolder {
        fn drop(&mut self) {
            // A lock the agent took over is the agent's to remove.
            if self.held() {
                let _ = std::fs::remove_dir(&self.path);
            }
        }
    }
}

/// Pi's trust: `"<folder>": true` in `trust.json` of Pi's agent folder, a
/// JSON object of resolved folders, each `true`, `false` or `null`. Pi 1.0.4
/// asks in a folder whose project files it would load (a `.pi/` resource of
/// the folder, or `.agents/skills` in it or a folder above), and reads the
/// nearest entry from the folder up, so an entry for the worktree itself
/// answers it and a folder above that the operator decided decides it too
/// (`findNearestTrustEntry`, `dist/core/trust-manager.js`). Pi takes
/// proper-lockfile's lock on `trust.json.lock` beside the file, as spelled
/// (`realpath: false`), around every read and write, and gives up after ten
/// tries 20 ms apart, so Hide holds it only across its own read and write.
mod pi {
    use super::lock_folder::LockFolder;
    use super::*;

    fn lock(settings: &Path) -> Result<LockFolder, InstallFailure> {
        if let Some(parent) = settings.parent() {
            hide_platform::fs::private::create_dir_all(parent).map_err(|error| {
                InstallFailure::NotWritable {
                    path: settings.display().to_string(),
                    detail: error.to_string(),
                }
            })?;
        }
        let mut path = settings.as_os_str().to_owned();
        path.push(".lock");
        LockFolder::take(PathBuf::from(path), settings, "Pi")
    }

    /// Whether Pi reads a decision for `key`: the folder's own entry, or the
    /// nearest folder above it that has one, trusted or not (`null` is none).
    fn decided(decisions: &Map<String, Value>, key: &str) -> bool {
        Path::new(key)
            .ancestors()
            .filter_map(Path::to_str)
            .any(|folder| decisions.get(folder).is_some_and(Value::is_boolean))
    }

    /// Records `key` as trusted unless Pi already decides it, and takes out
    /// the entries in `gone` in the same write. A folder the operator
    /// answered for in Pi, the worktree or a folder above it, is left as it
    /// is, "Do not trust" included: Pi asks nothing once a decision is saved
    /// and starts as it would without Hide, so Hide writes nothing and the
    /// start goes on.
    pub(super) fn add(
        settings: &Path,
        key: &str,
        gone: &[String],
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let lock = lock(settings)?;
        let mut document = read_document(settings)?
            .unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
        let decisions = root_object(&mut document, settings, Agent::Pi)?;
        if decided(decisions, key) {
            return Ok(Added::Present);
        }
        remember()?;
        take_out(decisions, gone);
        decisions.insert(key.to_owned(), Value::Bool(true));
        lock.still_held()?;
        write_document(settings, &document).map(|()| Added::New)
    }

    pub(super) fn remove(settings: &Path, gone: &[String]) -> Result<usize, InstallFailure> {
        if !settings.exists() {
            return Ok(0);
        }
        let lock = lock(settings)?;
        let Some(mut document) = read_document(settings)? else {
            return Ok(0);
        };
        let removed = take_out(root_object(&mut document, settings, Agent::Pi)?, gone);
        if removed > 0 {
            lock.still_held()?;
            write_document(settings, &document)?;
        }
        Ok(removed)
    }

    /// Takes out each entry in `gone` that Pi still trusts; one the operator
    /// has since answered "Do not trust" for stays theirs.
    fn take_out(decisions: &mut Map<String, Value>, gone: &[String]) -> usize {
        let mut removed = 0;
        for key in gone {
            if decisions.get(key) == Some(&Value::Bool(true)) {
                decisions.shift_remove(key);
                removed += 1;
            }
        }
        removed
    }
}

/// Claude Code's trust: `hasTrustDialogAccepted` of a `projects` entry in its
/// global config, which Claude Code itself rewrites while it runs. Claude
/// Code takes that file's lock (proper-lockfile's `<file>.lock` folder) and
/// reads the file again before every write of its own, so Hide holds the same
/// lock across its read and its write and neither drops the other's change.
mod claude {
    use super::lock_folder::LockFolder;
    use super::*;

    const PROJECTS: &str = "projects";
    const ACCEPTED: &str = "hasTrustDialogAccepted";
    /// The lock folder beside the config's real path, where proper-lockfile
    /// puts it (it resolves links), so a linked `~/.claude.json` is locked
    /// where Claude Code locks it.
    pub(super) fn lock_path(config: &Path) -> PathBuf {
        let real =
            hide_platform::fs::identity::canonical(config).unwrap_or_else(|_| config.to_path_buf());
        let mut path = real.into_os_string();
        path.push(".lock");
        PathBuf::from(path)
    }

    fn lock(config: &Path) -> Result<LockFolder, InstallFailure> {
        LockFolder::take(lock_path(config), config, "Claude Code")
    }

    fn projects<'a>(
        document: &'a mut HookDocument,
        config: &Path,
    ) -> Result<&'a mut Map<String, Value>, InstallFailure> {
        object_entry(
            root_object(document, config, Agent::ClaudeCode)?,
            PROJECTS,
            config,
            "projects is not an object",
        )
    }

    /// Sets the flag on `key` unless Claude Code trusts it already, and takes
    /// Hide's flag off the entries in `gone` in the same write.
    pub(super) fn add(
        settings: &Path,
        key: &str,
        gone: &[String],
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let config = settings;
        let lock = lock(config)?;
        let mut document =
            read_document(config)?.unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
        let projects = projects(&mut document, config)?;
        let entry = object_entry(projects, key, config, "a projects entry is not an object")?;
        if entry.get(ACCEPTED) == Some(&Value::Bool(true)) {
            return Ok(Added::Present);
        }
        remember()?;
        entry.insert(ACCEPTED.to_owned(), Value::Bool(true));
        take_out(projects, gone);
        lock.still_held()?;
        write_document(config, &document).map(|()| Added::New)
    }

    pub(super) fn remove(config: &Path, gone: &[String]) -> Result<usize, InstallFailure> {
        if !config.exists() {
            return Ok(0);
        }
        let lock = lock(config)?;
        let Some(mut document) = read_document(config)? else {
            return Ok(0);
        };
        let removed = take_out(projects(&mut document, config)?, gone);
        if removed > 0 {
            lock.still_held()?;
            write_document(config, &document)?;
        }
        Ok(removed)
    }

    /// Takes Hide's flag off each entry in `gone` that still has it set, and
    /// the entry with it when the flag was all it held.
    fn take_out(projects: &mut Map<String, Value>, gone: &[String]) -> usize {
        let mut removed = 0;
        for key in gone {
            let Some(entry) = projects.get_mut(key).and_then(Value::as_object_mut) else {
                continue;
            };
            if entry.get(ACCEPTED) != Some(&Value::Bool(true)) {
                continue;
            }
            entry.shift_remove(ACCEPTED);
            if entry.is_empty() {
                projects.shift_remove(key);
            }
            removed += 1;
        }
        removed
    }
}

#[cfg(test)]
#[path = "folder_trust/tests.rs"]
mod tests;
