//! The folder trust Hide records in an agent's own state for the worktrees it
//! makes. Claude Code, Codex, agy and Grok each ask, the first time they
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

use hide_agent_adapter::LaunchDialect;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::install::{HookDocument, InstallFailure, read_document, write_document};
use crate::runtime::AgentRuntime;

/// How long Hide waits for Claude Code's own lock on its config, which Claude
/// holds only while it saves the file.
const CLAUDE_LOCK_WAIT: Duration = Duration::from_secs(2);
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
}

impl Agent {
    pub const ALL: [Self; 4] = [Self::ClaudeCode, Self::Codex, Self::Antigravity, Self::Grok];

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
        }
    }

    /// The file whose trust entries this agent reads: for Claude Code and
    /// Codex the one their hooks' folder names (`AgentRuntime::trust_path`).
    pub fn settings_path(self, home: &Path) -> PathBuf {
        match self {
            Self::ClaudeCode => AgentRuntime::ClaudeCode.trust_path(home),
            Self::Codex => AgentRuntime::Codex.trust_path(home),
            Self::Antigravity => antigravity::settings_path(home),
            Self::Grok => grok::settings_path(home),
        }
    }

    /// The longest one change holds Hide's lock for this agent: a Codex
    /// session's whole bound, or Claude Code's wait for its own lock, with a
    /// second for Hide's files around it.
    fn longest_hold(self) -> Duration {
        let own = match self {
            Self::Codex => crate::codex_trust::SESSION_BOUND,
            Self::ClaudeCode => CLAUDE_LOCK_WAIT,
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
/// lock, when Hide quits. Claude Code's, agy's and Grok's changes start
/// nothing.
#[derive(Clone, Copy)]
pub struct Tools<'a> {
    pub codex: Option<&'a Path>,
    pub stop: &'a AtomicBool,
}

impl Tools<'static> {
    /// For a change that starts nothing: Claude Code's, agy's and Grok's.
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
/// and the entry it put in the agent's file when that names another folder.
/// Grok's entry names the repository's main checkout, shared by every
/// worktree of it, so the entry leaves only with the last of them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Recorded {
    /// The entry is the folder itself, the form every record held before
    /// Grok's.
    Folder(String),
    Through {
        folder: String,
        entry: String,
    },
}

impl Recorded {
    fn new(folder: String, entry: String) -> Self {
        if folder == entry {
            Self::Folder(folder)
        } else {
            Self::Through { folder, entry }
        }
    }

    fn folder(&self) -> &str {
        match self {
            Self::Folder(folder) | Self::Through { folder, .. } => folder,
        }
    }

    fn entry(&self) -> &str {
        match self {
            Self::Folder(entry) | Self::Through { entry, .. } => entry,
        }
    }
}

/// The entries of `leaving` that no record in `staying` still needs, each
/// once.
fn entries_to_take_out<'a>(
    leaving: impl IntoIterator<Item = &'a Recorded>,
    staying: &[&Recorded],
) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for recorded in leaving {
        let entry = recorded.entry();
        if !staying.iter().any(|kept| kept.entry() == entry)
            && !entries.iter().any(|taken| taken == entry)
        {
            entries.push(entry.to_owned());
        }
    }
    entries
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
    /// The agent already trusted the folder, so the entry stays the
    /// operator's and Hide records nothing.
    Present,
}

/// Records that `agent` may trust `folder`, spelled as given, without asking.
/// A folder the agent already trusts is left as it is and stays the
/// operator's.
pub fn trust(agent: Agent, home: &Path, folder: &Path, tools: Tools) -> Result<(), InstallFailure> {
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
    let new = Recorded::new(folder_key, key.clone());
    let _lock = hold(agent, home, tools.stop)?;
    let mut record = read_record(agent, home)?;
    // Hide's entries for folders that are gone (removed outside Hide, or by a
    // removal that could not reach the agent) go in the same write as the new
    // one, so the cap is only reached by folders that exist.
    let gone: Vec<Recorded> = record
        .iter()
        .filter(|recorded| recorded.folder() != new.folder() && folder_is_gone(recorded.folder()))
        .cloned()
        .collect();
    let staying: Vec<&Recorded> = record
        .iter()
        .filter(|recorded| !gone.contains(recorded))
        .chain([&new])
        .collect();
    let gone_entries = entries_to_take_out(&gone, &staying);
    // An entry another worktree of Hide's already needs is Hide's too, so
    // this folder is recorded with it whatever the agent's file says.
    let shared = record.iter().any(|recorded| {
        recorded.entry() == key && recorded.folder() != new.folder() && !gone.contains(recorded)
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
                .any(|recorded| recorded.folder() == new.folder())
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
            Agent::ClaudeCode => claude::add(home, &key, &gone_entries, &mut remember),
            Agent::Codex => crate::codex_trust::add_project_trust(
                codex_program(home, tools)?,
                home,
                tools.stop,
                &key,
                repository.as_deref(),
                &gone_entries,
                &mut remember,
            ),
            Agent::Antigravity => antigravity::add(home, &key, &gone_entries, &mut remember),
            Agent::Grok => grok::add(home, &key, &gone_entries, tools.stop, &mut remember),
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
                record.retain(|recorded| recorded.folder() != new.folder());
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
pub fn trust_worktree(
    agent: Agent,
    home: &Path,
    folder: &Path,
    tools: Tools,
) -> Result<(), InstallFailure> {
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
    trust(agent, home, &canonical, tools)
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

fn codex_program<'a>(home: &Path, tools: Tools<'a>) -> Result<&'a Path, InstallFailure> {
    tools.codex.ok_or_else(|| InstallFailure::NotWritable {
        path: Agent::Codex.settings_path(home).display().to_string(),
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
    forget_where(agent, home, tools, |entry| {
        names.iter().any(|name| same_folder(entry, name))
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

/// Takes out every trust Hide added for `agent`, as when it is switched off.
pub fn forget_all(agent: Agent, home: &Path, tools: Tools) -> Result<usize, InstallFailure> {
    forget_where(agent, home, tools, |_| true)
}

fn forget_where(
    agent: Agent,
    home: &Path,
    tools: Tools,
    chosen: impl Fn(&str) -> bool,
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
    let (gone, kept): (Vec<_>, Vec<_>) = record
        .into_iter()
        .partition(|recorded| chosen(recorded.folder()));
    if gone.is_empty() {
        return Ok(0);
    }
    let gone = entries_to_take_out(&gone, &kept.iter().collect::<Vec<_>>());
    let removed = match agent {
        _ if gone.is_empty() => 0,
        Agent::ClaudeCode => claude::remove(home, &gone)?,
        Agent::Codex => crate::codex_trust::remove_project_trust(
            codex_program(home, tools)?,
            home,
            tools.stop,
            &gone,
        )?,
        Agent::Antigravity => antigravity::remove(home, &gone)?,
        Agent::Grok => grok::remove(home, &gone, tools.stop)?,
    };
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

    pub(super) fn settings_path(home: &Path) -> PathBuf {
        home.join(".gemini")
            .join("antigravity-cli")
            .join("settings.json")
    }

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
        home: &Path,
        key: &str,
        gone: &[String],
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let settings = settings_path(home);
        let mut document = read_document(&settings)?
            .unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
        let list = entries(&mut document, &settings)?;
        if list.iter().any(|entry| entry.as_str() == Some(key)) {
            return Ok(Added::Present);
        }
        remember()?;
        take_out(list, gone);
        list.push(Value::String(key.to_owned()));
        write_document(&settings, &document).map(|()| Added::New)
    }

    pub(super) fn remove(home: &Path, gone: &[String]) -> Result<usize, InstallFailure> {
        let settings = settings_path(home);
        let Some(mut document) = read_document(&settings)? else {
            return Ok(0);
        };
        let removed = take_out(entries(&mut document, &settings)?, gone);
        if removed > 0 {
            write_document(&settings, &document)?;
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

    pub(super) fn settings_path(home: &Path) -> PathBuf {
        home.join(".grok").join("trusted_folders.toml")
    }

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
        home: &Path,
        key: &str,
        gone: &[String],
        stop: &AtomicBool,
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let settings = settings_path(home);
        let _lock = take_lock(&settings, stop)?;
        let (real, document) = read(&settings)?;
        let mut document = document.unwrap_or_default();
        let folders = folders(&mut document, &settings)?;
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
        write(&settings, &real, &document).map(|()| Added::New)
    }

    pub(super) fn remove(
        home: &Path,
        gone: &[String],
        stop: &AtomicBool,
    ) -> Result<usize, InstallFailure> {
        let settings = settings_path(home);
        if !settings.exists() {
            return Ok(0);
        }
        let _lock = take_lock(&settings, stop)?;
        let (real, Some(mut document)) = read(&settings)? else {
            return Ok(0);
        };
        let removed = take_out(folders(&mut document, &settings)?, gone);
        if removed > 0 {
            write(&settings, &real, &document)?;
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

/// Claude Code's trust: `hasTrustDialogAccepted` of a `projects` entry in its
/// global config, which Claude Code itself rewrites while it runs. Claude
/// Code takes that file's lock (proper-lockfile's `<file>.lock` folder) and
/// reads the file again before every write of its own, so Hide holds the same
/// lock across its read and its write and neither drops the other's change.
mod claude {
    use std::time::SystemTime;

    use hide_platform::fs::identity::FileId;

    use super::*;

    const PROJECTS: &str = "projects";
    const ACCEPTED: &str = "hasTrustDialogAccepted";
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

    /// Claude Code's lock on its config, held until dropped.
    pub(super) struct ConfigLock {
        path: PathBuf,
        /// The folder Hide made. A hold past [`STALE`] lets Claude Code take
        /// the lock over, and then the folder there is Claude's.
        made: Mark,
    }

    impl ConfigLock {
        /// The lock beside the config's real path, where proper-lockfile
        /// puts it (it resolves links), so a linked `~/.claude.json` is locked
        /// where Claude Code locks it.
        pub(super) fn path(config: &Path) -> PathBuf {
            let real = hide_platform::fs::identity::canonical(config)
                .unwrap_or_else(|_| config.to_path_buf());
            let mut path = real.into_os_string();
            path.push(".lock");
            PathBuf::from(path)
        }

        // Another process's lock folder announces nothing when it goes.
        #[allow(clippy::disallowed_methods)] // a production wait, not test code
        pub(super) fn take(config: &Path) -> Result<Self, InstallFailure> {
            let path = Self::path(config);
            let failure = |detail: String| InstallFailure::NotWritable {
                path: config.display().to_string(),
                detail,
            };
            let deadline = Instant::now() + CLAUDE_LOCK_WAIT;
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
                        return Ok(Self { path, made });
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
                    // the only other taker is Claude Code, whose own takeover
                    // leaves the same moment between the look and the removal.
                    if mark(&path) == seen {
                        let _ = std::fs::remove_dir(&path);
                    }
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(failure(format!(
                        "Claude Code held its config lock ({}) for more than {} s",
                        path.display(),
                        CLAUDE_LOCK_WAIT.as_secs()
                    )));
                }
                std::thread::sleep(POLL);
            }
        }

        /// Whether the lock folder is still the one Hide made.
        pub(super) fn held(&self) -> bool {
            mark(&self.path) == Some(self.made)
        }

        /// Refuses a write once Claude Code took the lock over.
        fn still_held(&self, config: &Path) -> Result<(), InstallFailure> {
            if self.held() {
                return Ok(());
            }
            Err(InstallFailure::NotWritable {
                path: config.display().to_string(),
                detail: "Claude Code took its config lock over while Hide held it".to_owned(),
            })
        }
    }

    impl Drop for ConfigLock {
        fn drop(&mut self) {
            // A lock Claude Code took over is Claude's to remove.
            if self.held() {
                let _ = std::fs::remove_dir(&self.path);
            }
        }
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
        home: &Path,
        key: &str,
        gone: &[String],
        remember: &mut dyn FnMut() -> Result<(), InstallFailure>,
    ) -> Result<Added, InstallFailure> {
        let config = Agent::ClaudeCode.settings_path(home);
        let lock = ConfigLock::take(&config)?;
        let mut document =
            read_document(&config)?.unwrap_or_else(|| HookDocument::new(Value::Object(Map::new())));
        let projects = projects(&mut document, &config)?;
        let entry = object_entry(projects, key, &config, "a projects entry is not an object")?;
        if entry.get(ACCEPTED) == Some(&Value::Bool(true)) {
            return Ok(Added::Present);
        }
        remember()?;
        entry.insert(ACCEPTED.to_owned(), Value::Bool(true));
        take_out(projects, gone);
        lock.still_held(&config)?;
        write_document(&config, &document).map(|()| Added::New)
    }

    pub(super) fn remove(home: &Path, gone: &[String]) -> Result<usize, InstallFailure> {
        let config = Agent::ClaudeCode.settings_path(home);
        if !config.exists() {
            return Ok(0);
        }
        let lock = ConfigLock::take(&config)?;
        let Some(mut document) = read_document(&config)? else {
            return Ok(0);
        };
        let removed = take_out(projects(&mut document, &config)?, gone);
        if removed > 0 {
            lock.still_held(&config)?;
            write_document(&config, &document)?;
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
