//! The kit's pieces in the accounts profiles run in (PRD agent-profiles
//! D-09, D-28; docs/agent-hooks.md, Profile accounts).
//!
//! A profile whose account is not its agent's default gets the agent's hook
//! there and, where the agent keeps its skills in its own folder (Claude
//! Code), Hide's skill stubs; a skill folder every agent shares
//! (`~/.agents/skills`) stays the default account's alone. The account's
//! places come from `hide_agent_adapter::resolve`, so this module names no
//! folder of an agent's.
//!
//! A pass takes the profiles the node's profile file holds
//! ([`KitTarget::profiles`]). The profiles of one account folder, as the file
//! system names it, are one target: it is installed into once, and refers to
//! each of them. A target no profile refers to any more has Hide's pieces
//! taken out, after which the record forgets it; one whose pieces could not
//! all come out stays recorded with no profile, and the next pass tries
//! again. A profile whose folder is the agent's default account, by another
//! spelling, is that account's, which only the agent's switch changes.
//!
//! A pass reads the profiles from their source ([`ProfileSource`]) once it
//! holds the account's lock, so the profiles it installs and takes out by
//! are the ones the node's file holds while it writes: a change of the file
//! made meanwhile is followed by a pass of its own, which reads it.
//!
//! Every path the kit writes or takes out in a profile account passes one
//! judgment ([`judged`]): it must lie in the account's own folder with no
//! link from that folder down to it, the file itself included, so it is its
//! own real place and no write or removal follows a link out of the account.
//! The paths come from the account's places each piece writes
//! ([`written_by`], which names every place), the writes it asks the agent
//! for included, so a refused piece asks the agent for nothing either.
//! A file is taken out only when no one else keeps its real place: a default
//! account (the skills folder every agent shares among its places), a target
//! still in use, or another installation on the account. A piece the
//! judgment refuses stays as it is, a link included, and the log says why.
//! The record keeps each target's account as the file system names it, so
//! taking its pieces out reaches the folder they went into, whatever a link
//! the profile spelled leads to now.
//!
//! Every rule of the default account holds here too: only Hide's marked
//! entries and unedited files are written or taken out, through the same
//! hook code with its source check, link keeping and atomic replacement; a
//! piece the operator took out stays out until a Reinstall; the agent's
//! switch governs every target. A profile's account is written only while
//! every folder from the one the profile named down to the agent's own is a
//! real folder of this account that no other account can change (D-28). The
//! default account keeps the checks it always had, so an account whose
//! `~/.claude` is group-writable is not refused by an update.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use hide_agent_adapter::{
    Account, AccountKey, AccountPath, AccountRef, NodeEnv, Place, Profile, ProfileId,
};
use hide_agent_hooks::AgentRuntime;
use hide_agent_hooks::account::AccountAt;
use hide_agent_hooks::guidance::GuidanceAgent;
use hide_agent_hooks::plugin::{self, PluginObserved};
use hide_agent_hooks::route::HookTarget;
use serde::{Deserialize, Serialize};

use crate::agent_kit::{
    enabled, helper, hook_code, install_guidance, no_helper, not_found, observe_guidance,
    skill_code,
};
use crate::agents::{
    ADAPTERS, AgentAdapter, Detection, HookSupport, PieceReport, SkillDir, SkillObserved,
    install_skill_in, observe_skill_in, remove_skill_in, skill_files_in, too_old,
};
use crate::record::{Record, TargetRecord};
use crate::{
    ComponentId, ComponentState, KitTarget, Observed, PartCause, PartWhy, Reason, RemoveOutcome,
    Scope,
};

/// Where one profile's pieces are, as a report states it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProfilePlace {
    /// The agent's default account, whose pieces are the agent's own row.
    Default,
    /// An account of its own: its folder, and each piece the agent keeps in
    /// an account (none for a skill in the shared folder or an agent with no
    /// hook).
    Own {
        folder: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<PieceReport>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill: Option<PieceReport>,
    },
    /// Nothing can be judged or put in its account; the sentence says why.
    Refused { reason: String },
    /// The profile is being removed. `failure` says why Hide's pieces are
    /// still in its account; with none, they are out (or another profile
    /// still uses the account) and the profile can go.
    Removing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<String>,
    },
}

/// Where a pass reads the machine's profiles from: the node's profile file,
/// read once the pass holds the account's lock.
#[derive(Clone)]
pub struct ProfileSource(Arc<dyn Fn() -> Result<KitProfiles, String> + Send + Sync>);

impl ProfileSource {
    pub fn new(read: impl Fn() -> Result<KitProfiles, String> + Send + Sync + 'static) -> Self {
        Self(Arc::new(read))
    }

    /// Profiles that do not change, for a caller that holds them already.
    pub fn fixed(profiles: KitProfiles) -> Self {
        Self::new(move || Ok(profiles.clone()))
    }

    fn read(&self) -> Result<KitProfiles, String> {
        (self.0)()
    }
}

impl std::fmt::Debug for ProfileSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProfileSource")
    }
}

/// The profiles of a machine, as its node's profile file holds them.
#[derive(Clone, Debug, Default)]
pub struct KitProfiles {
    /// The profiles in use: their accounts get their agents' pieces.
    pub live: Vec<Profile>,
    /// The profiles being removed: their accounts lose Hide's pieces once no
    /// profile in use refers to them.
    pub going: Vec<Profile>,
}

/// One profile after a pass or a read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileReport {
    pub id: ProfileId,
    /// The profile's version the report is for, so an answer about an older
    /// version is told apart (PRD agent-profiles D-30).
    pub version: u64,
    pub place: ProfilePlace,
}

/// The record's key of the account whose own folder is `folder`.
pub(crate) fn key(agent: &str, folder: &Path) -> String {
    format!("{agent}:{}", folder.display())
}

/// What an account is to the kit.
enum Located {
    /// The agent's default account.
    Default,
    Own(Box<Own>),
}

/// A profile account of its own.
struct Own {
    key: String,
    adapter: &'static AgentAdapter,
    account: Account,
    /// The account as the file system names it, which the record keeps.
    physical: AccountRef,
    /// The agent's own folder in the account (`AccountPath::State`), as the
    /// file system names it.
    folder: PathBuf,
    /// The same folder as the account spells it, which every path of its
    /// pieces starts with.
    spelled: PathBuf,
    /// The folder the profile named, as the file system names it: every
    /// folder from it down to [`Self::folder`] must be the account's own.
    root: PathBuf,
}

impl Own {
    fn at<'a>(&'a self, home: &'a Path) -> AccountAt<'a> {
        AccountAt::Account {
            home,
            account: &self.account,
        }
    }
}

/// The account `spelled` of the agent `agent`, resolved for a node that
/// honours none of the agent's variables, as every install is.
fn locate(home: &Path, agent: &str, spelled: &AccountRef) -> Result<Located, Reason> {
    let adapter = crate::agents::adapter(agent).ok_or_else(|| {
        Reason::not_changed(format!("Hide installs nothing for an agent named {agent}"))
    })?;
    let account = resolved(adapter, home, spelled)?;
    let state = account
        .path(AccountPath::State)
        .ok_or_else(|| {
            Reason::not_changed(format!("{} keeps no folder of its own", adapter.label))
        })?
        .to_path_buf();
    let folder = canonical(&state)?;
    let default = adapter.declaration().place_in(home, AccountPath::State);
    if hide_platform::fs::identity::canonical(&default).is_ok_and(|default| default == folder) {
        return Ok(Located::Default);
    }
    let (root, physical) = match spelled {
        AccountRef::Folder(named) => {
            let root = canonical(Path::new(named))?;
            let physical = AccountRef::Folder(root.display().to_string());
            (root, physical)
        }
        AccountRef::Name(name) => (folder.clone(), AccountRef::Name(name.clone())),
    };
    Ok(Located::Own(Box::new(Own {
        key: key(adapter.id, &folder),
        adapter,
        account,
        physical,
        folder,
        spelled: state,
        root,
    })))
}

/// The account of a profile of `adapter` in `spelled`, its other settings
/// aside: they decide nothing about where its pieces go.
fn resolved(adapter: &AgentAdapter, home: &Path, spelled: &AccountRef) -> Result<Account, Reason> {
    let profile = Profile {
        account: Some(spelled.clone()),
        ..Profile::builtin_default(adapter.id)
    };
    adapter
        .declaration()
        .resolve(&profile, &NodeEnv::new(Some(home.to_path_buf())))
        .map_err(|refusal| Reason::not_changed(refusal.to_string()))
}

fn canonical(path: &Path) -> Result<PathBuf, Reason> {
    hide_platform::fs::identity::canonical(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Reason::with(
                PartCause::NotCreatedYet,
                path.display().to_string(),
                format!(
                    "{} is not there; adding the profile again makes it",
                    path.display()
                ),
            )
        } else {
            Reason::not_changed(format!("{} could not be read: {error}", path.display()))
        }
    })
}

/// Every folder from `root` down to `folder` is a real folder of this
/// account that no other account can change (PRD agent-profiles D-28).
fn checked(root: &Path, folder: &Path) -> Result<(), Reason> {
    use hide_platform::fs::private;
    let below = folder.strip_prefix(root).unwrap_or(Path::new(""));
    let mut at = root.to_path_buf();
    let mut folders = vec![at.clone()];
    for part in below.components() {
        at.push(part);
        folders.push(at.clone());
    }
    for folder in folders {
        let refused = |text: String| Reason::not_changed(text);
        let metadata = std::fs::symlink_metadata(&folder)
            .map_err(|error| refused(format!("{} could not be read: {error}", folder.display())))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(refused(format!("{} is not a folder", folder.display())));
        }
        let owned = private::owned_by_current_user(&folder).map_err(|error| {
            refused(format!(
                "{} could not be checked: {error}",
                folder.display()
            ))
        })?;
        if !owned {
            return Err(refused(format!(
                "{} belongs to another account, so Hide writes nothing there",
                folder.display()
            )));
        }
        let open = private::others_can_modify(&folder).map_err(|error| {
            refused(format!(
                "{} could not be checked: {error}",
                folder.display()
            ))
        })?;
        if open {
            return Err(refused(format!(
                "{} can be changed by another account, so Hide writes nothing there",
                folder.display()
            )));
        }
    }
    Ok(())
}

/// Makes the account `profile` runs in ready for its pieces when it is
/// added: the folder it names and the agent's own folder in it, each made
/// private when missing, and every folder between them checked as the
/// account's own (PRD agent-profiles D-20, D-28). A profile in its agent's
/// default account needs nothing. The profile's other settings are checked
/// by [`hide_agent_adapter::resolve`] first, and nothing is made for one it
/// refuses.
pub fn prepare_account(home: &Path, profile: &Profile) -> Result<(), String> {
    let Some(spelled) = &profile.account else {
        return Ok(());
    };
    let adapter = crate::agents::adapter(&profile.kind)
        .ok_or_else(|| format!("Hide has no agent named {}", profile.kind))?;
    let account = adapter
        .declaration()
        .resolve(profile, &NodeEnv::new(Some(home.to_path_buf())))
        .map_err(|refusal| refusal.to_string())?;
    if account.key == AccountKey::Default {
        return Ok(());
    }
    let state = account
        .path(AccountPath::State)
        .ok_or_else(|| format!("{} keeps no folder of its own", adapter.label))?;
    if let AccountRef::Folder(named) = spelled {
        make_private(Path::new(named))?;
    }
    make_private(state)?;
    match locate(home, adapter.id, spelled).map_err(|reason| reason.text)? {
        Located::Default => Ok(()),
        Located::Own(own) => checked(&own.root, &own.folder).map_err(|reason| reason.text),
    }
}

/// `folder` and every missing folder above it, made private; one that is
/// there is left as it is, for [`checked`] to judge.
fn make_private(folder: &Path) -> Result<(), String> {
    hide_platform::fs::private::create_dir_all(folder)
        .map_err(|error| format!("{} could not be made: {error}", folder.display()))
}

/// A piece's state from what was observed, whether the kit put it there, and
/// a failure of this pass.
pub(crate) fn state_of(
    observed: Observed,
    recorded: bool,
    failure: Option<String>,
) -> (ComponentState, Option<Reason>) {
    match (failure, observed) {
        (Some(failure), _) => (ComponentState::Failed, Some(Reason::not_changed(failure))),
        (None, Observed::Current) => (ComponentState::Installed, None),
        (None, Observed::Stale(reason) | Observed::Edited(reason)) => {
            (ComponentState::Outdated, Some(reason))
        }
        (None, Observed::Missing) if recorded => {
            (ComponentState::Removed, Some(Reason::taken_out()))
        }
        (None, Observed::Missing) => (ComponentState::NotInstalled, None),
        (None, Observed::Blocked(reason)) => (ComponentState::Failed, Some(reason)),
        (None, Observed::Absent(reason) | Observed::Unsupported(reason)) => {
            (ComponentState::Absent, Some(reason))
        }
    }
}

fn piece(state: ComponentState, reason: Option<Reason>, location: Option<String>) -> PieceReport {
    crate::agent_kit::piece_report(state, reason, location)
}

/// The plugin or extension in the account `at`, as a pass decides on it.
pub(crate) fn observe_plugin(
    target: &KitTarget,
    adapter: &AgentAdapter,
    dialect: hide_agent_adapter::PluginDialect,
    detection: &Detection,
    at: AccountAt<'_>,
) -> Observed {
    let file = plugin::of(dialect);
    let noun = file.noun();
    let helper = helper(target);
    let state = crate::layout::state_dir_from_process(&target.home);
    let hook_target =
        match HookTarget::new(&helper, &state).and_then(|hook| hook.of_install(&target.install)) {
            Ok(target) => target,
            Err(reason) => return Observed::Blocked(Reason::not_changed(reason)),
        };
    let observed = file.observe(at, hook_target);
    // A file of Hide's that is already current stays as it is; anything a
    // pass would write needs this build's helper and a CLI new enough.
    let writes = !matches!(
        observed,
        PluginObserved::Current | PluginObserved::ConfigAbsent | PluginObserved::Foreign(_)
    );
    if writes && !helper.is_file() {
        return Observed::Blocked(no_helper(&helper));
    }
    if writes
        && let Some(reason) = detection
            .program(adapter)
            .and_then(|program| too_old(adapter, program, noun))
    {
        return Observed::Blocked(reason);
    }
    match observed {
        PluginObserved::Current => Observed::Current,
        PluginObserved::Stale(reason) => {
            Observed::Stale(Reason::new(PartCause::OlderVersion, reason))
        }
        PluginObserved::Edited => Observed::Edited(Reason::new(
            PartCause::Edited,
            format!("the {noun} was edited after Hide wrote it; Reinstall puts Hide's back"),
        )),
        PluginObserved::Missing => Observed::Missing,
        PluginObserved::ConfigAbsent => {
            let folder = file.config_directory(at).display().to_string();
            Observed::Absent(Reason::with(
                PartCause::NotCreatedYet,
                folder.clone(),
                format!(
                    "{} has not created {folder} yet; the {noun} is put in once it has",
                    adapter.label
                ),
            ))
        }
        PluginObserved::Foreign(name) => Observed::Absent(Reason::with(
            PartCause::Foreign,
            name.clone(),
            format!(
                "a {noun} named {name} that Hide did not write is already there; Hide left it alone"
            ),
        )),
        PluginObserved::Unreadable(reason) => {
            Observed::Blocked(Reason::new(PartCause::Unreadable, reason))
        }
    }
}

fn plugin_install(
    target: &KitTarget,
    dialect: hide_agent_adapter::PluginDialect,
    at: AccountAt<'_>,
) -> Result<(), String> {
    let state = crate::layout::state_dir_from_process(&target.home);
    let helper = helper(target);
    let hook_target = HookTarget::new(&helper, &state)?.of_install(&target.install)?;
    plugin::of(dialect).install(at, hook_target).map(drop)
}

fn observe_skill_at(root: &Path) -> Observed {
    match observe_skill_in(root) {
        SkillObserved::Current => Observed::Current,
        SkillObserved::Stale => Observed::Stale(Reason::new(
            PartCause::OlderVersion,
            "an older version of the skill is there",
        )),
        SkillObserved::Edited => Observed::Edited(Reason::new(
            PartCause::Edited,
            "the skill was edited after Hide wrote it; Reinstall puts Hide's text back",
        )),
        SkillObserved::Missing => Observed::Missing,
        SkillObserved::Foreign(names) => Observed::Absent(Reason::with(
            PartCause::Foreign,
            names.join(", "),
            format!(
                "skills named {} that Hide did not write are already there; Hide left them alone",
                names.join(", ")
            ),
        )),
        SkillObserved::Unreadable(reason) => {
            Observed::Blocked(Reason::new(PartCause::Unreadable, reason))
        }
    }
}

/// The kind of the agent's hook in the account `at`, the code the record
/// keeps it by and where it is (the hook file, or the plugin folder), `None`
/// for an agent with no hook.
fn hook_at(adapter: &AgentAdapter, at: AccountAt<'_>) -> Option<(HookKind, String, PathBuf)> {
    match adapter.hook {
        HookSupport::None => None,
        HookSupport::Part(part) => {
            let runtime = runtime_of(part)?;
            Some((
                HookKind::Part(runtime),
                part.code().to_owned(),
                runtime.config_path(at),
            ))
        }
        HookSupport::Guidance(agent) => Some((
            HookKind::Guidance(agent),
            hook_code(adapter),
            agent.config_path(at),
        )),
        HookSupport::Plugin(dialect) => Some((
            HookKind::Plugin(dialect),
            hook_code(adapter),
            plugin::of(dialect)
                .path(at, None)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
        )),
    }
}

#[derive(Clone, Copy)]
enum HookKind {
    Part(AgentRuntime),
    Guidance(GuidanceAgent),
    Plugin(hide_agent_adapter::PluginDialect),
}

fn runtime_of(part: ComponentId) -> Option<AgentRuntime> {
    match part {
        ComponentId::ClaudeCodeHook => Some(AgentRuntime::ClaudeCode),
        ComponentId::CodexHook => Some(AgentRuntime::Codex),
        ComponentId::Cli | ComponentId::CoordinationRetirement => None,
    }
}

/// `path` as the file system names it: its deepest folder that exists with
/// every link resolved, and the rest of it as written.
fn real(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(found) = hide_platform::fs::identity::canonical(at) {
            return rest.iter().rev().fold(found, |path, part| path.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The pieces the kit keeps in an account, and the folder trust the node
/// writes there for the worktrees Hide makes (`hide_agent_hooks::folder_trust`),
/// which every write in a profile account is judged like.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PieceKind {
    Hook,
    Skill,
    FolderTrust,
}

/// The piece that writes the account's place `place`, itself or by asking
/// the agent to (Codex stores its trust of Hide's hooks in its hook trust
/// file), `None` for a place the kit never writes in a profile account.
/// Every place is named, so a place an account gains decides here whether
/// the kit writes it, and [`judged`] then guards it.
fn written_by(place: AccountPath) -> Option<PieceKind> {
    match place {
        AccountPath::Hooks | AccountPath::HookTrust | AccountPath::Plugins => Some(PieceKind::Hook),
        AccountPath::Skills => Some(PieceKind::Skill),
        // The node writes it for a start in a worktree Hide made, in the
        // account the start's profile runs in, and a going target takes
        // Hide's entries out (docs/agent-hooks.md, Profile accounts).
        AccountPath::FolderTrust => Some(PieceKind::FolderTrust),
        AccountPath::Sessions | AccountPath::Login | AccountPath::State | AccountPath::Usage => {
            None
        }
    }
}

/// Every path `piece` writes or takes out in `account`, from the places
/// [`written_by`] gives it: a file place itself, and in a folder place the
/// entries Hide keeps there (its skill stubs, its plugin's files under this
/// installation's name and the app's earlier one).
fn written(
    account: &Account,
    piece: PieceKind,
    hook: Option<HookKind>,
    install: &str,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for place in AccountPath::ALL {
        if written_by(place) != Some(piece) {
            continue;
        }
        let Place::At(at) = account.place(place) else {
            continue;
        };
        match (place, hook) {
            (AccountPath::Skills, _) => paths.extend(skill_files_in(at)),
            (AccountPath::Plugins, Some(HookKind::Plugin(dialect))) => {
                paths.extend(plugin::of(dialect).files.iter().flat_map(|file| {
                    [at.join(file.name(Some(install))), at.join(file.name(None))]
                }));
            }
            _ => paths.push(at.clone()),
        }
    }
    paths
}

/// The real places of the files the default accounts keep: what each
/// agent's pieces write there, its hook's own file, and Hide's stubs in
/// each skills folder Hide writes, the shared one among them. A profile
/// account's going takes none of them out, whatever link leads there.
fn default_files(home: &Path, install: &str) -> BTreeSet<PathBuf> {
    let skills = SkillDir::ALL
        .iter()
        .flat_map(|dir| skill_files_in(&dir.path(home)));
    let pieces = ADAPTERS.iter().flat_map(|adapter| {
        let account = adapter.declaration().account_in(home);
        let hook = hook_at(adapter, AccountAt::Home(home));
        let kind = hook.as_ref().map(|(kind, _, _)| *kind);
        let mut paths = written(&account, PieceKind::Hook, kind, install);
        paths.extend(written(&account, PieceKind::Skill, None, install));
        paths.extend(hook.map(|(_, _, location)| location));
        paths
    });
    skills.chain(pieces).map(|path| real(&path)).collect()
}

/// One piece of a profile account: where it is, as a report names it, and
/// every path it writes or takes out ([`written`]).
struct Spot {
    location: PathBuf,
    paths: Vec<PathBuf>,
}

/// The pieces of a profile account: its hook, and its skills folder unless
/// the agent keeps its skills in the shared one.
struct Places {
    hook: Option<(HookKind, String, Spot)>,
    skill: Option<Spot>,
}

impl Places {
    fn of(own: &Own, home: &Path, install: &str) -> Self {
        Self {
            hook: hook_at(own.adapter, own.at(home)).map(|(kind, code, location)| {
                let paths = written(&own.account, PieceKind::Hook, Some(kind), install);
                (kind, code, Spot { location, paths })
            }),
            skill: match own.account.place(AccountPath::Skills) {
                Place::At(root) => Some(Spot {
                    location: root.clone(),
                    paths: written(&own.account, PieceKind::Skill, None, install),
                }),
                Place::Absent | Place::Shared => None,
            },
        }
    }

    /// The real places of every file a pass may write here, which no
    /// going target takes out while this one is in use.
    fn kept(&self, own: &Own) -> Vec<PathBuf> {
        self.hook
            .iter()
            .map(|(_, _, spot)| spot)
            .chain(&self.skill)
            .filter_map(|spot| judged(own, spot, Touch::Write).ok())
            .flatten()
            .collect()
    }
}

/// What the kit does to a path in a profile account.
#[derive(Clone, Copy)]
enum Touch<'a> {
    /// Writes Hide's file there.
    Write,
    /// Takes Hide's file out, unless another keeps it: a default account or
    /// a target in use (`held`, real places), or another installation on the
    /// account (`record`).
    TakeOut {
        held: &'a BTreeSet<PathBuf>,
        record: &'a Record,
    },
}

/// The one judgment of every path the kit writes or takes out in the
/// profile account `own`, a piece's paths at once ([`Spot::paths`]); the
/// answer is their real places. A piece whose own place is not among the
/// paths its places give is refused whole: it would write where nothing
/// judges. Each path must lie in the account's own folder with no link from
/// that folder down to it, the path itself included: it is then its own real
/// place, inside the account, and a write lands there and a removal takes
/// out exactly that, never a file a link leads to. Taking a file out also
/// needs no one else to keep its real place. A refusal leaves every path of
/// the piece as it is, a link included, and asks the agent for nothing.
fn judged(own: &Own, spot: &Spot, touch: Touch<'_>) -> Result<Vec<PathBuf>, Reason> {
    if !spot
        .paths
        .iter()
        .any(|path| path.starts_with(&spot.location))
    {
        return Err(Reason::not_changed(format!(
            "Hide does not judge what it would write at {}, so it writes nothing there",
            spot.location.display()
        )));
    }
    spot.paths
        .iter()
        .map(|path| {
            let really = real_in(own, path)?;
            if let Touch::TakeOut { held, record } = touch {
                if held.contains(&really) {
                    return Err(Reason::not_changed(format!(
                        "{} is also a default account's or another profile's, so it stays",
                        really.display()
                    )));
                }
                if record.skill_kept_elsewhere(&really) {
                    return Err(Reason::not_changed(format!(
                        "another installation of Hide keeps {}, so it stays",
                        really.display()
                    )));
                }
            }
            Ok(really)
        })
        .collect()
}

/// The real place of `path` in the account `own`: the account's own folder
/// with the rest of the path, once nothing on the way is a link. A part that
/// is not there yet is made by the write as a plain folder or file.
fn real_in(own: &Own, path: &Path) -> Result<PathBuf, Reason> {
    let rest = path.strip_prefix(&own.spelled).map_err(|_| {
        Reason::not_changed(format!(
            "{} is not in the account's folder {}, so Hide writes nothing there",
            path.display(),
            own.spelled.display()
        ))
    })?;
    let mut at = own.folder.clone();
    let mut there = true;
    for part in rest.components() {
        let Component::Normal(name) = part else {
            return Err(Reason::not_changed(format!(
                "{} is not a plain path, so Hide writes nothing there",
                path.display()
            )));
        };
        at.push(name);
        if !there {
            continue;
        }
        match std::fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Reason::not_changed(format!(
                    "{} is a link, so Hide neither writes nor takes out anything through it",
                    at.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => there = false,
            Err(error) => {
                return Err(Reason::new(
                    PartCause::Unreadable,
                    format!("{} could not be read: {error}", at.display()),
                ));
            }
        }
    }
    Ok(at)
}

fn observe_hook(target: &KitTarget, own: &Own, kind: HookKind, detection: &Detection) -> Observed {
    let at = own.at(&target.home);
    match kind {
        HookKind::Part(runtime) => crate::hooks::observe(target, runtime, at),
        HookKind::Guidance(agent) => observe_guidance(target, own.adapter, agent, detection, at),
        HookKind::Plugin(dialect) => observe_plugin(target, own.adapter, dialect, detection, at),
    }
}

fn install_hook(target: &KitTarget, own: &Own, kind: HookKind) -> Result<(), String> {
    let at = own.at(&target.home);
    match kind {
        HookKind::Part(runtime) => crate::hooks::install(target, runtime, at),
        HookKind::Guidance(agent) => install_guidance(target, agent, at),
        HookKind::Plugin(dialect) => plugin_install(target, dialect, at),
    }
}

/// Takes Hide's pieces out of a profile account: its marked hook entries or
/// unedited script, and its unedited skill stubs, each piece only once
/// [`judged`] lets it go; one it does not stays, and the log says why. True
/// when one went.
fn take_out(
    target: &KitTarget,
    own: &Own,
    places: &Places,
    held: &BTreeSet<PathBuf>,
    record: &Record,
) -> Result<bool, String> {
    checked(&own.root, &own.folder).map_err(|reason| reason.text)?;
    let touch = Touch::TakeOut { held, record };
    // A path that could not be read is tried again by the next pass; one
    // Hide may not touch stays for good.
    let may = |spot: &Spot| match judged(own, spot, touch) {
        Ok(_) => Ok(true),
        Err(reason) if reason.why.cause == PartCause::Unreadable => Err(reason.text),
        Err(reason) => {
            log("profile_target.left", Some(&own.key), &reason.text);
            Ok(false)
        }
    };
    let at = own.at(&target.home);
    let install = Some(target.install.as_str());
    let mut removed = false;
    if let Some((kind, _, spot)) = &places.hook
        && may(spot)?
    {
        removed |= match *kind {
            HookKind::Part(runtime) => match crate::hooks::remove(target, runtime, at) {
                RemoveOutcome::Removed => true,
                RemoveOutcome::Absent => false,
                RemoveOutcome::Kept { reason } | RemoveOutcome::Failed { reason } => {
                    return Err(reason);
                }
            },
            HookKind::Guidance(agent) => {
                hide_agent_hooks::guidance::remove(agent, at, install)
                    .map_err(|failure| failure.message())?
                    .removed_entries
                    > 0
            }
            HookKind::Plugin(dialect) => plugin::of(dialect).remove(at, install)?,
        };
    }
    if let Some(spot) = &places.skill
        && may(spot)?
    {
        removed |= remove_skill_in(&spot.location)?;
    }
    removed |= forget_trust(target, own)?;
    Ok(removed)
}

/// Where the folder trust of `own`'s agent is kept in the account, as a
/// piece the kit judges ([`written`]); `None` for an agent whose trust Hide
/// does not record.
fn trust_spot(own: &Own) -> Option<(hide_agent_hooks::folder_trust::Agent, Spot)> {
    let agent = own
        .adapter
        .declaration()
        .start
        .and_then(hide_agent_hooks::folder_trust::Agent::of)?;
    let Place::At(location) = own.account.place(AccountPath::FolderTrust) else {
        return None;
    };
    Some((
        agent,
        Spot {
            location: location.clone(),
            paths: written(&own.account, PieceKind::FolderTrust, None, ""),
        },
    ))
}

/// Takes the folder trust Hide recorded in a going profile account out of
/// it, once [`judged`] lets Hide write there, by the rule [`take_out`] holds
/// every piece to: a trust file that could not be read, and a removal that
/// failed (Codex's own program missing included), are errors, so the
/// account's target and its `Removing` state stay for the next pass; a place
/// Hide may not write stays for good, and the log says why. True when an
/// entry went.
fn forget_trust(target: &KitTarget, own: &Own) -> Result<bool, String> {
    let Some((agent, spot)) = trust_spot(own) else {
        return Ok(false);
    };
    match judged(own, &spot, Touch::Write) {
        Ok(_) => {}
        Err(reason) if reason.why.cause == PartCause::Unreadable => return Err(reason.text),
        Err(reason) => {
            log("profile_target.trust_left", Some(&own.key), &reason.text);
            return Ok(false);
        }
    }
    let tools = hide_agent_hooks::folder_trust::Tools {
        codex: target.codex.as_ref(),
        stop: &target.stop,
    };
    hide_agent_hooks::folder_trust::forget_account(agent, &target.home, &own.account, tools)
        .map(|forgotten| forgotten > 0)
        .map_err(|failure| failure.message())
}

/// Whether the node may write `profile`'s folder trust in the account the
/// profile runs in: every folder from the one it names down to the agent's
/// own is the account's, and the trust file is reached through no link, the
/// judgment every write Hide makes in a profile account passes
/// (docs/agent-hooks.md, Profile accounts). A profile in its agent's default
/// account keeps the default's own checks.
pub fn trust_writable(home: &Path, profile: &Profile) -> Result<(), String> {
    let Some(spelled) = &profile.account else {
        return Ok(());
    };
    let adapter = crate::agents::adapter(&profile.kind)
        .ok_or_else(|| format!("Hide has no agent named {}", profile.kind))?;
    let own = match locate(home, adapter.id, spelled).map_err(|reason| reason.text)? {
        Located::Default => return Ok(()),
        Located::Own(own) => own,
    };
    checked(&own.root, &own.folder).map_err(|reason| reason.text)?;
    let (_, spot) = trust_spot(&own)
        .ok_or_else(|| format!("Hide records no folder trust for {}", adapter.label))?;
    judged(&own, &spot, Touch::Write)
        .map(drop)
        .map_err(|reason| reason.text)
}

/// What a pass may do: a read changes nothing.
#[derive(Clone, Copy)]
enum Mode<'a> {
    Read,
    Apply {
        scope: &'a Scope,
        reinstall: &'a BTreeSet<ProfileId>,
    },
}

/// The pieces of one profile account after a pass or a read.
struct Pieces {
    hook: Option<PieceReport>,
    skill: Option<PieceReport>,
}

/// Judges, and in a pass installs, the pieces of one profile account.
/// `pieces` is what the record says the kit put there; a pass changes it.
#[allow(clippy::too_many_arguments)] // one target's whole judgment, its inputs named
fn pass_own(
    target: &KitTarget,
    own: &Own,
    places: &Places,
    on: bool,
    mode: Mode<'_>,
    restore: bool,
    pieces: &mut BTreeSet<String>,
    detection: &Detection,
    kept: &BTreeSet<PathBuf>,
    record: &Record,
) -> Pieces {
    let hook = places.hook.as_ref();
    let skill = places.skill.as_ref();
    let at = |spot: &Spot| Some(spot.location.display().to_string());
    let all = |state: ComponentState, reason: Option<Reason>| Pieces {
        hook: hook.map(|(_, _, spot)| piece(state, reason.clone(), at(spot))),
        skill: skill.map(|spot| piece(state, reason.clone(), at(spot))),
    };
    let turning_off =
        matches!(mode, Mode::Apply { scope, .. } if scope.agent_off.contains(own.adapter.id));
    if turning_off {
        return match take_out(target, own, places, kept, record) {
            Ok(_) => {
                pieces.clear();
                all(ComponentState::Off, None)
            }
            Err(reason) => all(ComponentState::Failed, Some(Reason::not_changed(reason))),
        };
    }
    if !on {
        return all(ComponentState::Off, None);
    }
    if !detection.installed(own.adapter) {
        return Pieces {
            hook: hook.map(|_| piece(ComponentState::Absent, Some(not_found(own.adapter)), None)),
            skill: skill.map(|_| piece(ComponentState::Absent, Some(not_found(own.adapter)), None)),
        };
    }
    if let Err(reason) = checked(&own.root, &own.folder) {
        return all(ComponentState::Failed, Some(reason));
    }
    let writes = matches!(mode, Mode::Apply { .. });
    let mut decide = |code: &str,
                      spot: &Spot,
                      observe: &dyn Fn() -> Observed,
                      install: &dyn Fn() -> Result<(), String>|
     -> PieceReport {
        let recorded = pieces.contains(code);
        // A piece Hide may not write is left as it is, and said so.
        if let Err(reason) = judged(own, spot, Touch::Write) {
            let observed = match reason.why.cause {
                PartCause::Unreadable => Observed::Blocked(reason),
                _ => Observed::Absent(reason),
            };
            let (state, reason) = state_of(observed, recorded, None);
            return piece(state, reason, at(spot));
        }
        let observed = observe();
        let install_now = writes
            && match &observed {
                Observed::Stale(_) => true,
                Observed::Edited(_) => restore,
                Observed::Missing => restore || !recorded,
                Observed::Current
                | Observed::Blocked(_)
                | Observed::Absent(_)
                | Observed::Unsupported(_) => false,
            };
        let failure = install_now.then(install).and_then(Result::err);
        let after = if install_now && failure.is_none() {
            observe()
        } else {
            observed
        };
        if writes && failure.is_none() && matches!(after, Observed::Current) {
            pieces.insert(code.to_owned());
        }
        let (state, reason) = state_of(after, pieces.contains(code), failure);
        piece(state, reason, at(spot))
    };
    let mut hook_report = hook.map(|(kind, code, spot)| {
        decide(
            code,
            spot,
            &|| observe_hook(target, own, *kind, detection),
            &|| install_hook(target, own, *kind),
        )
    });
    let skill_report = skill.map(|spot| {
        decide(
            &skill_code(own.adapter.skill_dir),
            spot,
            &|| observe_skill_at(&spot.location),
            &|| install_skill_in(&spot.location, restore),
        )
    });
    // Codex asks the operator to review a hook it has not trusted, in each
    // account's own `config.toml`; a read repeats what the last pass found.
    if let (Some((HookKind::Part(AgentRuntime::Codex), _, _)), Some(report)) =
        (hook, hook_report.as_mut())
        && report.state == ComponentState::Installed
    {
        let untrusted = if writes {
            crate::codex_trust::ensure_at(target, own.at(&target.home), &own.folder, &[])
        } else {
            crate::codex_trust::remembered_at(&own.folder)
        };
        if let Some(reason) = untrusted {
            report.state = ComponentState::Failed;
            report.why = Some(PartWhy {
                cause: PartCause::CodexUntrusted,
                value: None,
            });
            report.reason = Some(reason);
        }
    }
    Pieces {
        hook: hook_report,
        skill: skill_report,
    }
}

/// Whether the operator's request in `scope` restores the pieces of
/// `adapter`'s accounts: switching it on, or Reinstall of its kit part.
fn restores(scope: &Scope, adapter: &AgentAdapter) -> bool {
    scope.agent_on.contains(adapter.id)
        || matches!(adapter.hook, HookSupport::Part(part) if scope.restore.contains(&part))
}

/// One pass over every profile account, or a read of them. In a pass, a
/// target no profile refers to has Hide's pieces taken out and is forgotten
/// once they are; the profiles in `reinstall` have theirs put back even where
/// the operator took them out. True in the second value when the record
/// changed.
fn pass(
    target: &KitTarget,
    record: &mut Record,
    mode: Mode<'_>,
    detection: &Detection,
) -> (Vec<ProfileReport>, bool) {
    let Some(source) = target.profiles.as_ref() else {
        return (Vec::new(), false);
    };
    // Read now, under the account's lock in a pass: a file that cannot be
    // read leaves every profile account as it is.
    let profiles = match source.read() {
        Ok(profiles) => profiles,
        Err(reason) => {
            log("profile_target.profiles_unread", None, &reason);
            return (Vec::new(), false);
        }
    };
    let home = &target.home;
    let defaults = default_files(home, &target.install);
    let mut changed = false;
    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let mut placed: Vec<(&Profile, Result<Option<String>, Reason>)> = Vec::new();
    let mut wanted: BTreeMap<String, (Own, BTreeSet<ProfileId>)> = BTreeMap::new();
    for profile in &profiles.live {
        let located = match &profile.account {
            None => Ok(Located::Default),
            Some(spelled) => locate(home, &profile.kind, spelled),
        };
        match located {
            Ok(Located::Default) => placed.push((profile, Ok(None))),
            Ok(Located::Own(own)) => {
                placed.push((profile, Ok(Some(own.key.clone()))));
                wanted
                    .entry(own.key.clone())
                    .or_insert_with(|| (*own, BTreeSet::new()))
                    .1
                    .insert(profile.id.clone());
            }
            Err(reason) => placed.push((profile, Err(reason))),
        }
    }
    let scope_now = match mode {
        Mode::Apply { scope, .. } => scope.clone(),
        Mode::Read => Scope::default(),
    };
    let wanted: BTreeMap<String, (Own, BTreeSet<ProfileId>, Places, bool)> = wanted
        .into_iter()
        .map(|(key, (own, refs))| {
            let places = Places::of(&own, home, &target.install);
            let on = enabled(record, &scope_now, own.adapter);
            (key, (own, refs, places, on))
        })
        .collect();
    // What no going target may take out: the default accounts' files and
    // every file a target that stays in use holds.
    let kept: BTreeSet<PathBuf> = defaults
        .into_iter()
        .chain(
            wanted
                .values()
                .filter(|(_, _, _, on)| *on)
                .flat_map(|(own, _, places, _)| places.kept(own)),
        )
        .collect();
    if matches!(mode, Mode::Apply { .. }) {
        for (key, gone) in record.targets().clone() {
            if wanted.contains_key(&key) {
                continue;
            }
            match release(target, record, &key, &gone, &kept) {
                Ok(()) => changed |= record.forget_target(&key),
                Err(reason) => {
                    log("profile_target.remove_failed", Some(&key), &reason);
                    failures.insert(key.clone(), reason);
                    let left = record.target_mut(&key, || gone.clone());
                    if !left.refs.is_empty() {
                        left.refs.clear();
                        changed = true;
                    }
                }
            }
        }
    }
    let mut found: BTreeMap<String, Pieces> = BTreeMap::new();
    for (key, (own, refs, places, on)) in &wanted {
        let restore = match mode {
            Mode::Apply { scope, reinstall } => {
                restores(scope, own.adapter) || refs.iter().any(|id| reinstall.contains(id))
            }
            Mode::Read => false,
        };
        let mut pieces = record
            .targets()
            .get(key)
            .map(|known| known.pieces.clone())
            .unwrap_or_default();
        let pieces_before = pieces.clone();
        let result = pass_own(
            target,
            own,
            places,
            *on,
            mode,
            restore,
            &mut pieces,
            detection,
            &kept,
            record,
        );
        if matches!(mode, Mode::Apply { .. }) {
            let skill_root = places
                .skill
                .as_ref()
                .filter(|_| pieces.contains(&skill_code(own.adapter.skill_dir)))
                .map(|spot| real(&spot.location));
            let kept_record = record.target_mut(key, || TargetRecord {
                agent: own.adapter.id.to_owned(),
                account: own.physical.clone(),
                pieces: BTreeSet::new(),
                skill_root: None,
                refs: BTreeSet::new(),
            });
            if kept_record.refs != *refs {
                kept_record.refs = refs.clone();
                changed = true;
            }
            if pieces != pieces_before || kept_record.pieces != pieces {
                kept_record.pieces = pieces;
                changed = true;
            }
            if kept_record.skill_root != skill_root {
                kept_record.skill_root = skill_root;
                changed = true;
            }
        }
        found.insert(key.clone(), result);
    }
    // A profile being removed can go once its account holds nothing of
    // Hide's that no profile in use refers to.
    let going = profiles.going.iter().map(|profile| {
        let failure = match &profile.account {
            Some(spelled) => match locate(home, &profile.kind, spelled) {
                Ok(Located::Own(own)) => failures.get(&own.key).cloned().or_else(|| match mode {
                    Mode::Apply { .. } => None,
                    Mode::Read => record
                        .targets()
                        .get(&own.key)
                        .filter(|kept| kept.refs.is_empty() && !kept.pieces.is_empty())
                        .map(|_| "Hide's pieces are still in its account".to_owned()),
                }),
                Ok(Located::Default) => None,
                Err(reason) if reason.why.cause == PartCause::NotCreatedYet => None,
                Err(reason) => Some(reason.text),
            },
            None => None,
        };
        ProfileReport {
            id: profile.id.clone(),
            version: profile.version,
            place: ProfilePlace::Removing { failure },
        }
    });
    let going: Vec<ProfileReport> = going.collect();
    let reports = placed
        .into_iter()
        .map(|(profile, place)| ProfileReport {
            id: profile.id.clone(),
            version: profile.version,
            place: match place {
                Ok(None) => ProfilePlace::Default,
                Ok(Some(key)) => {
                    let pieces = found.get(&key);
                    ProfilePlace::Own {
                        folder: wanted[&key].0.folder.display().to_string(),
                        hook: pieces.and_then(|pieces| pieces.hook.clone()),
                        skill: pieces.and_then(|pieces| pieces.skill.clone()),
                    }
                }
                Err(reason) => ProfilePlace::Refused {
                    reason: reason.text,
                },
            },
        })
        .chain(going)
        .collect();
    (reports, changed)
}

/// Takes Hide's pieces out of the recorded target `key`, reached through
/// the account the record keeps, which names the folder they went into. A
/// folder that is gone took its pieces with it. One that now leads to
/// another account (a link inside it moved) or to a default account is
/// left alone: Hide cannot reach what it put there without touching
/// another account's pieces, so it forgets the target and says so in the
/// log.
fn release(
    target: &KitTarget,
    record: &Record,
    key: &str,
    gone: &TargetRecord,
    kept: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    match locate(&target.home, &gone.agent, &gone.account) {
        Ok(Located::Own(own)) if own.key == key => {
            let places = Places::of(&own, &target.home, &target.install);
            take_out(target, &own, &places, kept, record).map(drop)
        }
        Ok(Located::Own(_) | Located::Default) => {
            log(
                "profile_target.moved",
                Some(key),
                "the recorded folder now leads to another account; nothing was taken out",
            );
            Ok(())
        }
        Err(reason) if reason.why.cause == PartCause::NotCreatedYet => Ok(()),
        Err(reason) => Err(reason.text),
    }
}

/// A pass over every profile account (`crate::apply`, `crate::apply_profiles`).
pub(crate) fn apply(
    target: &KitTarget,
    scope: &Scope,
    reinstall: &BTreeSet<ProfileId>,
    record: &mut Record,
    detection: &Detection,
) -> (Vec<ProfileReport>, bool) {
    pass(target, record, Mode::Apply { scope, reinstall }, detection)
}

/// Every profile account judged, nothing changed (`crate::status`).
pub(crate) fn status(
    target: &KitTarget,
    record: &Record,
    detection: &Detection,
) -> Vec<ProfileReport> {
    let mut record = record.clone();
    pass(target, &mut record, Mode::Read, detection).0
}

/// Takes Hide's pieces out of every profile account this installation
/// recorded, for a machine leaving Hide (`crate::remove`): one outcome per
/// account, named by its key.
pub(crate) fn remove(target: &KitTarget, record: &Record) -> Vec<(String, RemoveOutcome)> {
    let defaults = default_files(&target.home, &target.install);
    record
        .targets()
        .iter()
        .map(|(key, known)| {
            let outcome = match locate(&target.home, &known.agent, &known.account) {
                Ok(Located::Own(own)) if own.key == *key => {
                    let places = Places::of(&own, &target.home, &target.install);
                    match take_out(target, &own, &places, &defaults, record) {
                        Ok(true) => RemoveOutcome::Removed,
                        Ok(false) => RemoveOutcome::Absent,
                        Err(reason) => RemoveOutcome::Failed { reason },
                    }
                }
                Ok(Located::Own(_) | Located::Default) => RemoveOutcome::Kept {
                    reason: "the recorded folder now leads to another account".to_owned(),
                },
                Err(reason) if reason.why.cause == PartCause::NotCreatedYet => {
                    RemoveOutcome::Absent
                }
                Err(reason) => RemoveOutcome::Failed {
                    reason: reason.text,
                },
            };
            (format!("profile:{key}"), outcome)
        })
        .collect()
}

fn log(kind: &str, key: Option<&str>, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "kit",
            "kind": kind,
            "target": key,
            "reason": reason,
        })
    );
}
