//! The machine's agent profiles, which the node that runs on it owns (PRD
//! agent-profiles D-04, D-20, D-24, D-29, D-30; docs/ARCHITECTURE.md, The
//! node's profile file).
//!
//! One file in the node's state folder (`hide_kit::layout::agent_profiles`)
//! holds every profile of the machine and each agent's default one, and only
//! this module reads or writes it, one read or change at a time, replacing it
//! whole. The kit reads it through [`source`] once a pass holds the account's
//! lock, so a pass installs and takes out by the file as it is then; this
//! module never holds the file while it waits on the kit, and every change is
//! followed by a pass of its own.
//! A change names the file's version it was made against, so two screens
//! editing at once never overwrite each other. Adding a profile checks it
//! (`hide_agent_adapter::resolve`), makes its account's folder private when
//! it is missing and checks that it is the account's own
//! (`hide_kit::prepare_account`), before anything is saved; the kit then
//! installs into it. Removing one marks it as being removed, and it leaves
//! the file once the kit has taken Hide's pieces out of its account, here or
//! in a later kit pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hide_agent_adapter::account::Refusal;
use hide_agent_adapter::{AccountRef, NodeEnv, Profile, ProfileId};
use hide_kit::{KitProfiles, ProfilePlace, ProfileReport, ProfileSource};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, HostError, HostResult};
use crate::serve::Env;
use hide_node_link::profiles::{
    ListedProfile, MAX_PROFILES, ProfileApplied, ProfileField, ProfileLaunch, ProfileList,
    ProfileOp, ProfileProgram, ProfileRefusal, ProfileStart,
};

const FORMAT: u32 = 1;
/// The largest profile file read or written: 64 profiles of ordinary
/// variables are a few KiB, and a start could not carry much more on any
/// system (Windows' whole environment block is 32 K characters).
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_NAME_BYTES: usize = 64;
const MAX_PROGRAM_BYTES: usize = 4096;

/// One read or change of the file at a time per process; the node is the
/// file's only writer. Never held while the kit runs: a pass takes it inside
/// the account's lock to read the file ([`source`]).
static FILE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct File {
    format: u32,
    version: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    profiles: Vec<Stored>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    defaults: BTreeMap<String, ProfileId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    profile: Profile,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    removing: bool,
}

/// Where this node keeps the file, and the home its accounts are under.
struct Place {
    file: PathBuf,
    home: PathBuf,
}

fn place(env: &Env) -> HostResult<Place> {
    match (&env.state, &env.home) {
        (Some(state), Some(home)) => Ok(Place {
            file: hide_kit::layout::agent_profiles(state),
            home: home.clone(),
        }),
        _ => Err(HostError::new(
            ErrorCode::Unsupported,
            "This node has no home or state folder, so it keeps no agent profiles",
        )),
    }
}

fn load(path: &Path) -> HostResult<File> {
    use std::io::Read;
    let unread = |error: std::io::Error| {
        HostError::new(
            ErrorCode::Io,
            format!("{} could not be read: {error}", path.display()),
        )
    };
    let opened = match std::fs::File::open(path) {
        Ok(opened) => opened,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(File {
                format: FORMAT,
                ..File::default()
            });
        }
        Err(error) => return Err(unread(error)),
    };
    // Read no more than the cap, whatever the file's length says: it may
    // grow between the look and the read.
    let mut bytes = Vec::new();
    opened
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unread)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(HostError::new(
            ErrorCode::TooLarge,
            format!(
                "{} is larger than {MAX_FILE_BYTES} bytes, so it was not read",
                path.display()
            ),
        ));
    }
    let file: File = serde_json::from_slice(&bytes).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("{} is damaged: {error}", path.display()),
        )
    })?;
    if file.format > FORMAT {
        return Err(HostError::new(
            ErrorCode::Unsupported,
            format!("{} was written by a newer Hide", path.display()),
        ));
    }
    Ok(file)
}

fn save(path: &Path, file: &File) -> HostResult<()> {
    let mut bytes = serde_json::to_vec_pretty(file).map_err(|error| {
        HostError::new(
            ErrorCode::Io,
            format!("The profiles could not be encoded: {error}"),
        )
    })?;
    bytes.push(b'\n');
    if bytes.len() > MAX_FILE_BYTES {
        return Err(HostError::new(
            ErrorCode::TooLarge,
            format!("The profiles would be larger than {MAX_FILE_BYTES} bytes"),
        ));
    }
    hide_platform::fs::atomic::write_file(path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|error| {
            HostError::new(
                ErrorCode::Io,
                format!("{} could not be written: {error}", path.display()),
            )
        })
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    FILE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The machine's profiles, each with its pieces as the kit judges them.
pub fn list(env: &Env, values: bool) -> HostResult<ProfileList> {
    let place = place(env)?;
    let judged = judge(env, None);
    let file = read(&place.file)?;
    Ok(listed(&file, judged, values))
}

/// Applies `op` to the profiles of the version `expected_version`, then
/// installs or takes out the pieces of the accounts it touches.
pub fn apply(env: &Env, op: ProfileOp, expected_version: u64) -> HostResult<ProfileApplied> {
    let place = place(env)?;
    let changed = {
        let _file = lock();
        let file = load(&place.file)?;
        if file.version != expected_version {
            return Err(HostError::new(
                ErrorCode::Conflict,
                "The profiles changed since they were read; read them again",
            ));
        }
        match change(&place.home, &file, op) {
            Ok(changed) => {
                save(&place.file, &changed.file)?;
                Ok(changed)
            }
            Err(refused) => Err((refused, file)),
        }
    };
    let changed = match changed {
        Ok(changed) => changed,
        Err((refused, file)) => {
            return Ok(ProfileApplied {
                refused: Some(refused),
                added: None,
                list: listed(&file, judge(env, None), false),
            });
        }
    };
    let judged = judge(env, changed.install.then(BTreeSet::new));
    let file = settle(&place.file, &judged)?;
    Ok(ProfileApplied {
        refused: None,
        added: changed.added,
        list: listed(&file, judged, false),
    })
}

/// Reinstall of the pieces of the profile `id`, even those the operator
/// took out of its account.
pub fn install(env: &Env, id: ProfileId) -> HostResult<ProfileList> {
    let place = place(env)?;
    let file = read(&place.file)?;
    if !file
        .profiles
        .iter()
        .any(|stored| stored.profile.id == id && !stored.removing)
    {
        return Err(HostError::new(
            ErrorCode::NotFound,
            format!("This machine has no profile {}", id.0),
        ));
    }
    let judged = judge(env, Some(BTreeSet::from([id])));
    let file = settle(&place.file, &judged)?;
    Ok(listed(&file, judged, false))
}

/// How a start of `kind` with the profile `id` runs on this machine (PRD
/// agent-profiles D-26, D-27, B22): the profile as the file holds it now,
/// resolved as it was checked when it was added, its program found, and the
/// `hide` beside this node, which the start line runs. A profile that is
/// gone says so, so its caller can name it and say how to start with
/// another; one being removed, another agent's, whose program is not here,
/// or that this node cannot start is refused with a reason that names it.
/// Only a node that keeps no profiles, or a file it cannot read, fails.
pub fn launch(env: &Env, kind: &str, id: &ProfileId) -> HostResult<ProfileLaunch> {
    let own = std::env::current_exe().ok();
    launch_beside(env, kind, id, own.as_deref())
}

/// [`launch`] by the node whose program is `own`.
fn launch_beside(
    env: &Env,
    kind: &str,
    id: &ProfileId,
    own: Option<&Path>,
) -> HostResult<ProfileLaunch> {
    let place = place(env)?;
    let file = read(&place.file)?;
    let Some(stored) = file
        .profiles
        .iter()
        .find(|stored| stored.profile.id == *id && !id.is_default())
    else {
        return Ok(ProfileLaunch::Gone);
    };
    let profile = &stored.profile;
    let refused = |text: String| Ok(ProfileLaunch::Refused { text });
    if stored.removing {
        return refused(format!("the profile {} is being removed", profile.name));
    }
    let Some(adapter) =
        hide_agent_adapter::adapter(kind).filter(|adapter| adapter.id == profile.kind)
    else {
        return refused(format!(
            "the profile {} is not one of {kind}'s",
            profile.name
        ));
    };
    let account = match adapter.resolve(profile, &NodeEnv::new(Some(place.home.clone()))) {
        Ok(account) => account,
        Err(refusal) => {
            return refused(format!(
                "the profile {} cannot start here: {refusal}",
                profile.name
            ));
        }
    };
    let program = match profile.program.as_deref().map(program_start) {
        None => None,
        Some(Ok(found)) => Some(found),
        Some(Err(why)) => {
            return refused(format!(
                "the profile {} cannot start its program: {why}",
                profile.name
            ));
        }
    };
    let exec = match own
        .ok_or_else(|| "its node's program is unknown".to_owned())
        .and_then(exec_command)
    {
        Ok(exec) => exec,
        Err(why) => {
            return refused(format!(
                "this machine cannot start the profile {}: {why}",
                profile.name
            ));
        }
    };
    Ok(ProfileLaunch::Ready(ProfileStart {
        name: profile.name.clone(),
        account: account.key,
        version: profile.version,
        env: account.env,
        program,
        exec,
    }))
}

/// How the profile's `program` starts here: a bare name found as a pane's
/// shell finds an agent's program (`hide_platform::programs::account_path`),
/// or an absolute path, each a program this account can start
/// (`hide_platform::programs::runnable`) and read as it starts
/// (`hide_platform::programs::Start`). Why it cannot start otherwise.
fn program_start(program: &str) -> Result<ProfileProgram, String> {
    use hide_platform::programs::{Program, Start, account_path, runnable};
    let search = account_path().ok_or("this account's PATH cannot be read")?;
    let named = Path::new(program);
    let found = if named.components().count() > 1 {
        if !named.is_absolute() {
            return Err(format!("{program} is not an absolute path"));
        }
        Program::at(named, search)?
    } else {
        let found = Program::find(&search, program)
            .ok_or_else(|| format!("{program} is not a program this machine finds"))?;
        runnable(found.path(), Some(found.search()))?;
        found
    };
    let (file, leading) = match found.start() {
        Start::Direct { program, leading } => (program, leading),
        Start::Batch => (found.into_path(), Vec::new()),
    };
    let text = |word: std::ffi::OsString| {
        word.into_string()
            .map_err(|_| format!("{program} starts with a path that is not text"))
    };
    Ok(ProfileProgram {
        file: text(file.into_os_string())?,
        leading: leading.into_iter().map(text).collect::<Result<_, _>>()?,
    })
}

/// The `hide` beside the node's own program `own`, which runs a profile's
/// start (`hide agent exec`): the node and it come from one build.
fn exec_command(own: &Path) -> Result<String, String> {
    let exec = own.with_file_name(format!("hide{}", std::env::consts::EXE_SUFFIX));
    hide_platform::programs::runnable(&exec, None)?;
    exec.into_os_string()
        .into_string()
        .map_err(|_| "its hide command's path is not text".to_owned())
}

/// Where a kit pass of this node reads the profiles
/// (`hide_kit::KitTarget::profiles`): the file, read when the pass holds
/// the account's lock. A node that keeps none has no source, and a file that
/// cannot be read leaves every profile account as it is; the kit logs why.
pub(crate) fn source(env: &Env) -> Option<ProfileSource> {
    let file = place(env).ok()?.file;
    Some(ProfileSource::new(move || {
        read(&file)
            .map(|file| kit_profiles(&file))
            .map_err(|error| error.message)
    }))
}

/// Drops the profiles being removed whose accounts a kit pass found free of
/// Hide's pieces.
pub(crate) fn settle_after_kit(env: &Env, reports: &[ProfileReport]) {
    let Ok(place) = place(env) else {
        return;
    };
    let judged = Judged {
        reports: Some(reports.to_vec()),
        unjudged: None,
    };
    if let Err(error) = settle(&place.file, &judged) {
        log("profiles.settle_failed", &error.message);
    }
}

/// The accounts this node reads sessions in (PRD agent-profiles D-36,
/// `hide_session::Accounts`): each agent's default account under `home`, and
/// the account of every profile the file holds, one being removed included,
/// whose sessions stay readable until it leaves the file. A node that keeps
/// no profiles, or whose file cannot be read, reads the default accounts
/// alone, and the log says why. The answer is kept while the file's stamp
/// holds, so a session read costs one stat of the file.
pub fn session_accounts(state: Option<&Path>, home: &Path) -> Arc<hide_session::Accounts> {
    static KEPT: Mutex<Option<KeptAccounts>> = Mutex::new(None);
    let file = state.map(hide_kit::layout::agent_profiles);
    let stamp = file.as_deref().and_then(hide_session::stamp::stamp_at);
    let mut kept = KEPT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(kept) = kept
        .as_ref()
        .filter(|kept| kept.home == home && kept.file == file && kept.stamp == stamp)
    {
        return Arc::clone(&kept.accounts);
    }
    let profiles = match file.as_deref().map(read).transpose() {
        Ok(read) => read.map_or_else(Vec::new, |read| {
            read.profiles
                .into_iter()
                .map(|stored| stored.profile)
                .collect()
        }),
        Err(error) => {
            log("profiles.session_accounts_unread", &error.message);
            Vec::new()
        }
    };
    let accounts = Arc::new(hide_session::Accounts::new(home).with_profiles(&profiles));
    *kept = Some(KeptAccounts {
        home: home.to_path_buf(),
        file,
        stamp,
        accounts: Arc::clone(&accounts),
    });
    accounts
}

/// [`session_accounts`]' last answer and what it was made from.
struct KeptAccounts {
    home: PathBuf,
    file: Option<PathBuf>,
    stamp: Option<String>,
    accounts: Arc<hide_session::Accounts>,
}

/// The file as it is now.
fn read(path: &Path) -> HostResult<File> {
    let _file = lock();
    load(path)
}

/// What the kit found for the profiles.
struct Judged {
    reports: Option<Vec<ProfileReport>>,
    unjudged: Option<String>,
}

/// The kit's judgment of every profile in the file as the kit reads it: a
/// pass when `reinstall` is given, with those profiles' pieces put back, a
/// read otherwise.
fn judge(env: &Env, reinstall: Option<BTreeSet<ProfileId>>) -> Judged {
    let mut target = match crate::kit::profile_target(env) {
        Ok(target) => target,
        Err(error) => {
            return Judged {
                reports: None,
                unjudged: Some(error.message),
            };
        }
    };
    target.profiles = source(env);
    let answer = match reinstall {
        Some(reinstall) => hide_kit::apply_profiles(&target, &reinstall).map(|applied| {
            if let Some(unsaved) = &applied.unsaved {
                log("profiles.record_unsaved", unsaved);
            }
            applied.profiles
        }),
        None => hide_kit::profile_status(&target),
    };
    match answer {
        Ok(reports) => Judged {
            reports: Some(reports),
            unjudged: None,
        },
        Err(refusal) => Judged {
            reports: None,
            unjudged: Some(refusal.detail),
        },
    }
}

/// Forgets the profiles being removed that the kit reported free to go, and
/// answers the file as it is then.
fn settle(path: &Path, judged: &Judged) -> HostResult<File> {
    let _file = lock();
    let mut file = load(path)?;
    let Some(reports) = &judged.reports else {
        return Ok(file);
    };
    let free: BTreeSet<(&ProfileId, u64)> = reports
        .iter()
        .filter(|report| matches!(report.place, ProfilePlace::Removing { failure: None }))
        .map(|report| (&report.id, report.version))
        .collect();
    let before = file.profiles.len();
    file.profiles.retain(|stored| {
        !(stored.removing && free.contains(&(&stored.profile.id, stored.profile.version)))
    });
    if file.profiles.len() != before {
        file.version += 1;
        save(path, &file)?;
    }
    Ok(file)
}

fn kit_profiles(file: &File) -> KitProfiles {
    let (going, live): (Vec<&Stored>, Vec<&Stored>) =
        file.profiles.iter().partition(|stored| stored.removing);
    KitProfiles {
        live: live
            .into_iter()
            .map(|stored| stored.profile.clone())
            .collect(),
        going: going
            .into_iter()
            .map(|stored| stored.profile.clone())
            .collect(),
    }
}

fn listed(file: &File, judged: Judged, values: bool) -> ProfileList {
    let places: BTreeMap<(&ProfileId, u64), &ProfilePlace> = judged
        .reports
        .iter()
        .flatten()
        .map(|report| ((&report.id, report.version), &report.place))
        .collect();
    let profiles = file
        .profiles
        .iter()
        .map(|stored| {
            let profile = &stored.profile;
            ListedProfile {
                id: profile.id.clone(),
                kind: profile.kind.clone(),
                name: profile.name.clone(),
                account: profile.account.clone(),
                program: profile.program.clone(),
                env_keys: profile.env.iter().map(|(key, _)| key.clone()).collect(),
                env: values.then(|| profile.env.clone()),
                version: profile.version,
                removing: stored.removing,
                place: places
                    .get(&(&profile.id, profile.version))
                    .map(|place| (*place).clone()),
            }
        })
        .collect();
    ProfileList {
        version: file.version,
        profiles,
        defaults: file.defaults.clone(),
        unjudged: judged.unjudged,
    }
}

/// A file after a change, and what the change needs of the kit.
struct Changed {
    file: File,
    added: Option<ProfileId>,
    /// The change touches an account, so the kit installs or takes out.
    install: bool,
}

fn refused(field: ProfileField, text: impl Into<String>) -> ProfileRefusal {
    ProfileRefusal {
        field,
        text: text.into(),
        env_key: None,
    }
}

fn change(home: &Path, file: &File, op: ProfileOp) -> Result<Changed, ProfileRefusal> {
    let mut file = file.clone();
    let version = file.version + 1;
    let mut added = None;
    let mut install = false;
    match op {
        ProfileOp::Add {
            kind,
            name,
            account,
            program,
            env,
            default,
        } => {
            let kind = agent(&kind)?;
            let name = named(&file, &kind, &name, None)?;
            if file.profiles.len() >= MAX_PROFILES {
                return Err(refused(
                    ProfileField::Profile,
                    format!("This machine already keeps {MAX_PROFILES} profiles"),
                ));
            }
            let profile = Profile {
                id: new_id(&file, &kind, &name),
                kind: kind.clone(),
                name,
                account: account.map(|account| expanded(account, home)),
                program: program_of(program)?,
                env,
                version,
            };
            checked(home, &profile)?;
            hide_kit::prepare_account(home, &profile)
                .map_err(|text| refused(ProfileField::Account, text))?;
            if default {
                file.defaults.insert(kind, profile.id.clone());
            }
            added = Some(profile.id.clone());
            install = profile.account.is_some();
            file.profiles.push(Stored {
                profile,
                removing: false,
            });
        }
        ProfileOp::Edit {
            id,
            name,
            program,
            env,
        } => {
            let at = live(&file, &id)?;
            let kind = file.profiles[at].profile.kind.clone();
            let name = named(&file, &kind, &name, Some(&id))?;
            let profile = Profile {
                name,
                program: program_of(program)?,
                env,
                version,
                ..file.profiles[at].profile.clone()
            };
            checked(home, &profile)?;
            file.profiles[at].profile = profile;
        }
        ProfileOp::Default { kind, id } => {
            let kind = agent(&kind)?;
            if id.is_default() {
                file.defaults.remove(&kind);
            } else {
                let at = live(&file, &id)?;
                if file.profiles[at].profile.kind != kind {
                    return Err(refused(
                        ProfileField::Profile,
                        format!("{} is not a profile of {kind}", id.0),
                    ));
                }
                file.defaults.insert(kind, id);
            }
        }
        ProfileOp::Remove { id } => {
            let at = live(&file, &id)?;
            let stored = &mut file.profiles[at];
            stored.removing = true;
            stored.profile.version = version;
            file.defaults.retain(|_, default| *default != id);
            install = true;
        }
    }
    file.version = version;
    Ok(Changed {
        file,
        added,
        install,
    })
}

/// The agent `kind` names, by the id its profiles are kept under.
fn agent(kind: &str) -> Result<String, ProfileRefusal> {
    hide_kit::agents::adapter(kind)
        .map(|adapter| adapter.id.to_owned())
        .ok_or_else(|| {
            refused(
                ProfileField::Kind,
                format!("Hide keeps no profiles for an agent named {kind}"),
            )
        })
}

/// The index of the profile `id`, which is in use.
fn live(file: &File, id: &ProfileId) -> Result<usize, ProfileRefusal> {
    if id.is_default() {
        return Err(refused(
            ProfileField::Profile,
            "The built-in profile cannot be changed or removed",
        ));
    }
    let at = file
        .profiles
        .iter()
        .position(|stored| stored.profile.id == *id)
        .ok_or_else(|| {
            refused(
                ProfileField::Profile,
                format!("This machine has no profile {}", id.0),
            )
        })?;
    if file.profiles[at].removing {
        return Err(refused(
            ProfileField::Profile,
            "The profile is being removed",
        ));
    }
    Ok(at)
}

/// `name` as a profile of `kind` may take it: unique among the agent's
/// profiles (D-30), apart from the profile `own`.
fn named(
    file: &File,
    kind: &str,
    name: &str,
    own: Option<&ProfileId>,
) -> Result<String, ProfileRefusal> {
    let name = name.trim();
    if name.is_empty() {
        return Err(refused(ProfileField::Name, "A profile needs a name"));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(refused(
            ProfileField::Name,
            format!("A profile's name is at most {MAX_NAME_BYTES} bytes"),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(refused(
            ProfileField::Name,
            "A profile's name cannot hold a control character",
        ));
    }
    if name.eq_ignore_ascii_case(&Profile::builtin_default(kind).name) {
        return Err(refused(
            ProfileField::Name,
            "The built-in profile already has that name",
        ));
    }
    let taken = file.profiles.iter().any(|stored| {
        stored.profile.kind == kind
            && Some(&stored.profile.id) != own
            && stored.profile.name.eq_ignore_ascii_case(name)
    });
    if taken {
        return Err(refused(
            ProfileField::Name,
            format!("{kind} already has a profile named {name}"),
        ));
    }
    Ok(name.to_owned())
}

fn program_of(program: Option<String>) -> Result<Option<String>, ProfileRefusal> {
    let Some(program) = program.filter(|program| !program.trim().is_empty()) else {
        return Ok(None);
    };
    if program.len() > MAX_PROGRAM_BYTES || program.contains('\0') {
        return Err(refused(
            ProfileField::Program,
            format!("A program is at most {MAX_PROGRAM_BYTES} bytes, with no NUL"),
        ));
    }
    Ok(Some(program))
}

/// A folder spelled with `~` expanded under `home`, once, as it is stored
/// (D-15); everything else as it was given, a final separator included.
fn expanded(account: AccountRef, home: &Path) -> AccountRef {
    match account {
        AccountRef::Folder(folder) if folder == "~" => {
            AccountRef::Folder(home.display().to_string())
        }
        AccountRef::Folder(folder) => match folder
            .strip_prefix("~/")
            .or_else(|| folder.strip_prefix("~\\").filter(|_| cfg!(windows)))
        {
            Some(rest) => AccountRef::Folder(home.join(rest).display().to_string()),
            None => AccountRef::Folder(folder),
        },
        name => name,
    }
}

/// The profile as the agent's definition takes it (D-15, D-29).
fn checked(home: &Path, profile: &Profile) -> Result<(), ProfileRefusal> {
    let adapter = hide_agent_adapter::adapter(&profile.kind).ok_or_else(|| {
        refused(
            ProfileField::Kind,
            format!("Hide keeps no profiles for an agent named {}", profile.kind),
        )
    })?;
    adapter
        .resolve(profile, &NodeEnv::new(Some(home.to_path_buf())))
        .map(drop)
        .map_err(|refusal| {
            let text = refusal.to_string();
            let (field, env_key) = match refusal {
                Refusal::OtherAgent => (ProfileField::Kind, None),
                Refusal::NodeVariable { .. } | Refusal::NodeAccountName { .. } => {
                    (ProfileField::Profile, None)
                }
                Refusal::NoAccountHome
                | Refusal::AccountUnsupported { .. }
                | Refusal::AccountKind { .. }
                | Refusal::AccountEmpty
                | Refusal::AccountRelative
                | Refusal::AccountIsDefault
                | Refusal::AccountName => (ProfileField::Account, None),
                Refusal::TooManyVariables { .. } | Refusal::VariableName => {
                    (ProfileField::Env, None)
                }
                Refusal::VariableReserved { key }
                | Refusal::VariableRepeated { key }
                | Refusal::VariableValue { key } => (ProfileField::Env, Some(key)),
            };
            ProfileRefusal {
                field,
                text,
                env_key,
            }
        })
}

/// A stable id no profile of the file has: what a start, an agent's record
/// and a project's choice name the profile by, kept across a rename.
fn new_id(file: &File, kind: &str, name: &str) -> ProfileId {
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut hash = Sha256::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        hash.update(now.as_nanos().to_le_bytes());
        hash.update(std::process::id().to_le_bytes());
        hash.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        hash.update(kind.as_bytes());
        hash.update([0]);
        hash.update(name.as_bytes());
        let digest = hash.finalize();
        let id = ProfileId(format!(
            "p-{}",
            digest[..8]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ));
        if !id.is_default() && file.profiles.iter().all(|stored| stored.profile.id != id) {
            return id;
        }
    }
}

fn log(kind: &str, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "profiles",
            "kind": kind,
            "reason": reason,
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Call;
    use crate::serve::{KitPlace, handle_in};
    use serde_json::Value;

    /// A node of a machine whose home and state folder are a temporary
    /// folder's, with a desktop package of its own. The agents' pieces are
    /// the kit's tests' (`hide_kit`'s `profile_cases`); these hold what the
    /// node keeps and answers.
    struct Node {
        _dir: tempfile::TempDir,
        root: PathBuf,
        env: Env,
    }

    impl Node {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = hide_platform::fs::identity::canonical(dir.path()).unwrap();
            let home = root.join("home");
            let state = root.join("state");
            let kit = root.join("kit");
            for folder in [&home, &state, &kit] {
                hide_platform::fs::private::create_dir_all(folder).unwrap();
            }
            let mut env = Env::standalone(Some(home));
            env.kit = KitPlace::Bundled(kit);
            env.state = Some(state);
            // Raised, so the kit asks no login shell for its search: what it
            // finds then is the process's own `PATH`, which these tests
            // never depend on.
            env.stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            Self {
                _dir: dir,
                root,
                env,
            }
        }

        fn home(&self) -> &Path {
            self.env.home.as_deref().unwrap()
        }

        fn call(&self, call: Call) -> HostResult<Value> {
            handle_in(call, &self.env)
        }

        fn list(&self, values: bool) -> ProfileList {
            serde_json::from_value(self.call(Call::ProfileList { values }).unwrap()).unwrap()
        }

        fn apply(&self, change: ProfileOp) -> ProfileApplied {
            let expected_version = self.list(false).version;
            serde_json::from_value(
                self.call(Call::ProfileApply {
                    change,
                    expected_version,
                })
                .unwrap(),
            )
            .unwrap()
        }
    }

    fn add(kind: &str, name: &str, account: Option<AccountRef>) -> ProfileOp {
        add_with(kind, name, account, &[], false)
    }

    fn add_with(
        kind: &str,
        name: &str,
        account: Option<AccountRef>,
        env: &[(&str, &str)],
        default: bool,
    ) -> ProfileOp {
        ProfileOp::Add {
            kind: kind.to_owned(),
            name: name.to_owned(),
            account,
            program: None,
            env: env
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            default,
        }
    }

    fn folder(path: &str) -> Option<AccountRef> {
        Some(AccountRef::Folder(path.to_owned()))
    }

    fn refusal(applied: &ProfileApplied) -> &ProfileRefusal {
        applied
            .refused
            .as_ref()
            .unwrap_or_else(|| panic!("not refused: {applied:?}"))
    }

    #[test]
    fn an_added_profile_is_kept_with_its_account_expanded_and_made_private() {
        let node = Node::new();

        let applied = node.apply(add_with(
            "claude",
            "work",
            folder("~/accounts/work/"),
            &[("ANTHROPIC_BASE_URL", "https://gw.example")],
            false,
        ));

        assert_eq!(applied.refused, None, "{applied:?}");
        let id = applied.added.clone().unwrap();
        let account = node.home().join("accounts/work");
        let listed = &applied.list.profiles[0];
        assert_eq!(listed.id, id);
        assert_eq!(listed.kind, "claude-code", "kept under the agent's id");
        assert_eq!(
            listed.account,
            Some(AccountRef::Folder(format!("{}/", account.display()))),
            "`~` expanded once, the final separator kept"
        );
        assert_eq!(listed.env_keys, ["ANTHROPIC_BASE_URL"]);
        assert_eq!(listed.env, None, "no value unless asked");
        assert!(
            matches!(&listed.place, Some(ProfilePlace::Own { folder, .. }) if Path::new(folder) == account),
            "{listed:?}"
        );
        assert!(account.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for made in [node.home().join("accounts"), account.clone()] {
                let mode = std::fs::metadata(&made).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "{}", made.display());
            }
        }
        let read = node.list(true);
        assert_eq!(read.version, applied.list.version);
        assert_eq!(
            read.profiles[0].env,
            Some(vec![(
                "ANTHROPIC_BASE_URL".to_owned(),
                "https://gw.example".to_owned()
            )])
        );
    }

    #[test]
    fn a_change_made_against_an_older_list_is_a_conflict() {
        let node = Node::new();
        node.apply(add("claude-code", "one", None));

        let stale = node
            .call(Call::ProfileApply {
                change: add("claude-code", "two", None),
                expected_version: 0,
            })
            .unwrap_err();

        assert_eq!(stale.code, ErrorCode::Conflict);
        assert_eq!(node.list(false).profiles.len(), 1);
    }

    #[test]
    fn each_refusal_names_its_field_and_saves_nothing() {
        let node = Node::new();
        node.apply(add("claude-code", "work", None));
        let shared = node.root.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        }
        let reserved = add_with(
            "claude-code",
            "reserved",
            None,
            &[("CLAUDE_CONFIG_DIR", "/elsewhere")],
            false,
        );
        let forbidden = add_with(
            "codex",
            "forbidden",
            None,
            &[("CODEX_SQLITE_HOME", "/elsewhere")],
            false,
        );
        let mut cases = vec![
            (add("gemini", "x", None), ProfileField::Kind, None),
            (add("claude-code", "Work", None), ProfileField::Name, None),
            (
                add("claude-code", "default", None),
                ProfileField::Name,
                None,
            ),
            (
                add("claude-code", "rel", folder("accounts")),
                ProfileField::Account,
                None,
            ),
            (
                add("antigravity", "moved", folder("/tmp/agy")),
                ProfileField::Account,
                None,
            ),
            (reserved, ProfileField::Env, Some("CLAUDE_CONFIG_DIR")),
            (forbidden, ProfileField::Env, Some("CODEX_SQLITE_HOME")),
        ];
        if cfg!(unix) {
            cases.push((
                add("claude-code", "open", folder(&shared.display().to_string())),
                ProfileField::Account,
                None,
            ));
        }
        let before = node.list(false);

        for (change, field, key) in cases {
            let applied = node.apply(change.clone());
            let refused = refusal(&applied);
            assert_eq!(refused.field, field, "{change:?}: {refused:?}");
            assert_eq!(refused.env_key.as_deref(), key, "{change:?}");
            assert_eq!(applied.list.version, before.version, "{change:?}");
        }
        assert_eq!(node.list(false).profiles, before.profiles);
    }

    #[test]
    fn a_machine_keeps_at_most_64_profiles() {
        let node = Node::new();
        let place = place(&node.env).unwrap();
        let file = File {
            format: FORMAT,
            version: 1,
            profiles: (0..MAX_PROFILES)
                .map(|n| Stored {
                    profile: Profile {
                        id: ProfileId(format!("p-{n}")),
                        name: format!("p{n}"),
                        version: 1,
                        ..Profile::builtin_default("claude-code")
                    },
                    removing: false,
                })
                .collect(),
            defaults: BTreeMap::new(),
        };
        save(&place.file, &file).unwrap();

        let applied = node.apply(add("codex", "one more", None));

        let refused = refusal(&applied);
        assert_eq!(refused.field, ProfileField::Profile);
        assert!(refused.text.contains("64"), "{refused:?}");
    }

    #[test]
    fn an_edit_keeps_the_account_and_a_removal_takes_the_profile_and_its_default() {
        let node = Node::new();
        let added = node.apply(add_with("claude-code", "work", folder("~/work"), &[], true));
        let id = added.added.clone().unwrap();
        assert_eq!(added.list.defaults.get("claude-code"), Some(&id));

        let edited = node.apply(ProfileOp::Edit {
            id: id.clone(),
            name: "office".to_owned(),
            program: Some("claude-gw".to_owned()),
            env: Vec::new(),
        });
        let profile = &edited.list.profiles[0];
        assert_eq!(edited.refused, None, "{edited:?}");
        assert_eq!(
            (profile.name.as_str(), profile.program.as_deref()),
            ("office", Some("claude-gw"))
        );
        assert_eq!(profile.account, added.list.profiles[0].account);
        assert!(profile.version > added.list.profiles[0].version);

        let removed = node.apply(ProfileOp::Remove { id: id.clone() });

        assert_eq!(removed.refused, None, "{removed:?}");
        assert!(removed.list.profiles.is_empty(), "{removed:?}");
        assert!(removed.list.defaults.is_empty());
        assert_eq!(node.list(false).profiles, Vec::new());
        let gone = node.call(Call::ProfileInstall { profile: id }).unwrap_err();
        assert_eq!(gone.code, ErrorCode::NotFound);
    }

    /// A node program `own` with the `hide` beside it, as every package
    /// places them.
    fn package(node: &Node) -> PathBuf {
        let bin = node.root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let hide = bin.join(format!("hide{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&hide, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hide, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin.join(format!("hided{}", std::env::consts::EXE_SUFFIX))
    }

    #[test]
    fn a_profiles_start_carries_its_account_its_variables_and_the_hide_beside_the_node() {
        let node = Node::new();
        let own = package(&node);
        let added = node.apply(add_with(
            "claude-code",
            "work",
            folder("~/accounts/work/"),
            &[("ANTHROPIC_BASE_URL", "https://gw.example")],
            false,
        ));
        let id = added.added.clone().unwrap();
        let account = format!("{}/", node.home().join("accounts/work").display());

        let start = launch_beside(&node.env, "claude", &id, Some(&own)).unwrap();

        assert_eq!(
            start,
            ProfileLaunch::Ready(ProfileStart {
                name: "work".to_owned(),
                account: hide_agent_adapter::AccountKey::Folder(account.clone()),
                version: added.list.profiles[0].version,
                env: vec![
                    ("CLAUDE_CONFIG_DIR".to_owned(), account),
                    (
                        "ANTHROPIC_BASE_URL".to_owned(),
                        "https://gw.example".to_owned()
                    ),
                ],
                program: None,
                exec: own
                    .with_file_name(format!("hide{}", std::env::consts::EXE_SUFFIX))
                    .display()
                    .to_string(),
            })
        );
    }

    #[test]
    fn a_profile_that_is_gone_another_agents_or_whose_program_is_missing_starts_nothing() {
        let node = Node::new();
        let own = package(&node);
        let missing = node.root.join("no-such-gateway");
        let added = node.apply(ProfileOp::Add {
            kind: "codex".to_owned(),
            name: "gateway".to_owned(),
            account: None,
            program: Some(missing.display().to_string()),
            env: Vec::new(),
            default: false,
        });
        let id = added.added.clone().unwrap();
        let launch = |kind: &str, id: &ProfileId, own: Option<&Path>| {
            launch_beside(&node.env, kind, id, own).unwrap()
        };

        assert_eq!(
            launch("codex", &id, Some(&own)),
            ProfileLaunch::Refused {
                text: format!(
                    "the profile gateway cannot start its program: {} is not a file",
                    missing.display()
                )
            }
        );
        assert_eq!(
            launch("claude-code", &id, Some(&own)),
            ProfileLaunch::Refused {
                text: "the profile gateway is not one of claude-code's".to_owned()
            }
        );
        assert_eq!(
            launch("codex", &ProfileId::default_profile(), Some(&own)),
            ProfileLaunch::Gone,
            "the built-in profile is never asked"
        );

        assert_eq!(
            node.apply(ProfileOp::Remove { id: id.clone() }).refused,
            None
        );
        let gone = node
            .call(Call::ProfileLaunch {
                kind: "codex".to_owned(),
                profile: id,
            })
            .unwrap();
        assert_eq!(
            serde_json::from_value::<ProfileLaunch>(gone).unwrap(),
            ProfileLaunch::Gone
        );
    }

    #[test]
    fn a_node_without_its_hide_cannot_start_a_profiles_agent() {
        let node = Node::new();
        let id = node
            .apply(add("pi", "work", folder("~/accounts/pi/")))
            .added
            .unwrap();
        let unpackaged = node
            .root
            .join(format!("hided{}", std::env::consts::EXE_SUFFIX));

        let refused = launch_beside(&node.env, "pi", &id, Some(&unpackaged)).unwrap();

        assert!(
            matches!(&refused, ProfileLaunch::Refused { text } if text.starts_with("this machine cannot start the profile work: ")),
            "{refused:?}"
        );
    }

    #[test]
    fn a_node_without_a_state_folder_keeps_no_profiles() {
        let node = Node::new();
        let standalone = Env::standalone(node.env.home.clone());

        let refused = handle_in(Call::ProfileList { values: false }, &standalone).unwrap_err();

        assert_eq!(refused.code, ErrorCode::Unsupported);
    }
}
