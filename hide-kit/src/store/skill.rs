//! The skill's [`Applier`]: a skill is a folder, copied whole into each
//! skills folder its column's switch reaches.
//!
//! The places are the skill folders the agent definitions name
//! (`[account.paths] skills`, through `SkillDir`), each a column of its own
//! (the operator's choice: a folder's agents all see what is in it), and the
//! own skills folder of each profile account that has one (Claude's), in the
//! column of its agent's default folder (D-22). A place is its real folder:
//! two columns whose folders are one (`~/.agents/skills` a link to
//! `~/.claude/skills`) are one place both switches govern. Hide's own stubs
//! sit in the same folders and are never an item (B35).
//!
//! A status read every few seconds (`crate::status`) reads a folder's files
//! only when its stat fingerprint moved since they were last read
//! ([`REMEMBERED_LIMIT`] folders remembered in the process); a pass that may
//! change a folder always reads them.
//!
//! A copy is written beside its final name and renamed into place, so an
//! agent never reads half a skill; a copy Hide wrote is the only entry ever
//! replaced or taken out, which the store's record decides.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use hide_agent_adapter::AccountPath;
use sha2::{Digest, Sha256};

use super::{
    Applier, Carried, CarriedFile, Column, Ctx, Entry, FileEntry, ItemBody, ItemKind, Listed,
    Place, SKILL_BYTE_LIMIT, SKILL_FILE_LIMIT, SkillBody, Source, StoreItem, StoreRefusal,
    StoreRefusalCause,
};
use crate::agents::{ADAPTERS, SkillDir};

pub(crate) struct SkillApplier;

/// The columns in the order the table shows them: Claude, the shared
/// folder, agy.
const COLUMNS: [SkillDir; 3] = [SkillDir::Claude, SkillDir::Shared, SkillDir::Antigravity];

fn column_of(dir: SkillDir) -> Column {
    Column {
        id: dir.code().to_owned(),
        agents: ADAPTERS
            .iter()
            .filter(|adapter| adapter.skill_dir == dir)
            .map(|adapter| adapter.id.to_owned())
            .collect(),
        also: ADAPTERS
            .iter()
            .filter(|adapter| adapter.skill_dir != dir && adapter.also_reads(dir))
            .map(|adapter| adapter.id.to_owned())
            .collect(),
    }
}

impl Applier for SkillApplier {
    fn kind(&self) -> ItemKind {
        ItemKind::Skill
    }

    fn columns(&self) -> Vec<Column> {
        COLUMNS.into_iter().map(column_of).collect()
    }

    fn places(&self, ctx: &Ctx<'_>) -> Vec<Place> {
        let home = ctx.home();
        let mut places: Vec<Place> = Vec::new();
        for dir in COLUMNS {
            let path = real_folder(&dir.path(home));
            join(
                &mut places,
                Place {
                    key: key(&path),
                    blocked: (!dir.writable(home)).then(|| {
                        "its agent has not created its own folder yet; the skill goes in once it has"
                            .to_owned()
                    }),
                    path,
                    columns: vec![dir.code().to_owned()],
                    profiles: Vec::new(),
                    fresh: ctx.fresh,
                },
            );
        }
        for account in ctx.accounts.list() {
            let Some(root) = account.path(AccountPath::Skills) else {
                continue;
            };
            let (path, blocked) = match account.judge(&root) {
                Ok(real) => (real, None),
                Err(reason) => (root, Some(reason)),
            };
            join(
                &mut places,
                Place {
                    key: key(&path),
                    path,
                    columns: vec![account.adapter().skill_dir.code().to_owned()],
                    profiles: account.profiles().to_vec(),
                    blocked,
                    fresh: ctx.fresh,
                },
            );
        }
        places
    }

    fn supports(&self, _item: &StoreItem, _column: &Column) -> Result<(), String> {
        // Every supported agent reads its skills folder on every system
        // (docs/agent-hooks.md, Support table).
        Ok(())
    }

    fn entry(&self, place: &Place, name: &str) -> Result<Option<Entry>, String> {
        entry_at(&place.path.join(name), place.fresh)
    }

    fn entries(&self, place: &Place) -> Result<Vec<Listed>, String> {
        let folder = match std::fs::read_dir(&place.path) {
            Ok(folder) => folder,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(format!(
                    "{} could not be read: {error}",
                    place.path.display()
                ));
            }
        };
        let mut entries = Vec::new();
        for found in folder {
            let found = found
                .map_err(|error| format!("{} could not be read: {error}", place.path.display()))?;
            let name = found.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !found.path().join("SKILL.md").is_file() {
                continue;
            }
            match entry_at(&found.path(), place.fresh) {
                Ok(Some(entry)) => entries.push((name, Ok(entry))),
                Ok(None) => {}
                Err(reason) => entries.push((name, Err(reason))),
            }
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }

    fn write(&self, ctx: &Ctx<'_>, place: &Place, item: &StoreItem) -> Result<String, String> {
        // Only Hide's own copy is ever there to replace (`store::reconcile`).
        copy_into(&ctx.item_files(&item.id), &place.path, &item.name, true)?;
        digest(&place.path.join(&item.name))
    }

    fn remove(&self, place: &Place, name: &str) -> Result<(), String> {
        remove_at(&place.path.join(name))
    }

    fn leave_copy(&self, place: &Place, item: &StoreItem, files: &Path) -> Result<(), String> {
        copy_into(files, &place.path, &item.name, false)
    }

    fn adopt(
        &self,
        _ctx: &Ctx<'_>,
        place: &Place,
        name: &str,
        into: &Path,
    ) -> Result<(Source, ItemBody), StoreRefusal> {
        let at = place.path.join(name);
        let entry = entry_at(&at, true)
            .map_err(|reason| StoreRefusal::new(StoreRefusalCause::Failed, reason))?
            .ok_or_else(|| {
                StoreRefusal::new(
                    StoreRefusalCause::NotFound,
                    format!("{} is gone", at.display()),
                )
            })?;
        // A skill installed as a link is followed where the link leads
        // (D-20); any other is a copy the store keeps.
        let (from, source) = match entry.link {
            Some(target) => (
                target.clone(),
                Source::Folder {
                    path: target.display().to_string(),
                },
            ),
            None => (
                at.clone(),
                Source::Copy {
                    from: at.display().to_string(),
                },
            ),
        };
        let body = take_in(&from, into)?;
        Ok((source, ItemBody::Skill(body)))
    }

    fn set_aside(&self, place: &Place, name: &str, into: &Path) -> Result<ItemBody, StoreRefusal> {
        let at = place.path.join(name);
        // Read before anything moves: a folder past the skill limits stays.
        let body = body_of(&at)?;
        // A rename, not a copy, so a `.git` folder and the links inside go
        // along; a folder on another volume than the store stays where it
        // is, and the take-over is refused.
        super::move_folder(&at, into)
            .map_err(|reason| StoreRefusal::new(StoreRefusalCause::Failed, reason))?;
        Ok(ItemBody::Skill(body))
    }

    fn put_back(&self, place: &Place, name: &str, from: &Path) -> Result<bool, String> {
        let at = place.path.join(name);
        if std::fs::symlink_metadata(&at).is_ok() {
            return Ok(false);
        }
        super::move_folder(from, &at).map(|()| true)
    }

    fn purge(&self, _ctx: &Ctx<'_>, _item: &StoreItem) -> Result<(), String> {
        Ok(())
    }

    fn export(&self, ctx: &Ctx<'_>, item: &StoreItem) -> Result<Carried, StoreRefusal> {
        use base64::Engine;
        let root = ctx.item_files(&item.id);
        let files = walk(&root)?
            .files
            .into_iter()
            .map(|file| {
                let bytes = std::fs::read(&file.path).map_err(|error| {
                    StoreRefusal::new(
                        StoreRefusalCause::Failed,
                        format!("{} could not be read: {error}", file.path.display()),
                    )
                })?;
                Ok(CarriedFile {
                    executable: executable(&file.path),
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    path: file.relative,
                })
            })
            .collect::<Result<Vec<_>, StoreRefusal>>()?;
        Ok(Carried::Skill { files })
    }

    fn import(
        &self,
        _ctx: &Ctx<'_>,
        name: &str,
        content: &Carried,
        into: &Path,
    ) -> Result<ItemBody, StoreRefusal> {
        let Carried::Skill { files } = content else {
            return Err(StoreRefusal::new(
                StoreRefusalCause::Failed,
                format!("{name} was carried as another kind"),
            ));
        };
        let decoded = into.with_file_name(format!(
            "{}.decoded",
            into.file_name().unwrap_or_default().to_string_lossy()
        ));
        let written = write_carried(files, &decoded);
        let body = written.and_then(|()| take_in(&decoded, into));
        let _ = std::fs::remove_dir_all(&decoded);
        Ok(ItemBody::Skill(body?))
    }
}

/// Writes a carried skill's files under `root`, within the skill limits and
/// never outside it: a path that is absolute, climbs out or is not plain
/// names is refused (B28).
fn write_carried(files: &[CarriedFile], root: &Path) -> Result<(), StoreRefusal> {
    use base64::Engine;
    let refused = |reason: String| StoreRefusal::new(StoreRefusalCause::Failed, reason);
    if files.len() > SKILL_FILE_LIMIT {
        return Err(StoreRefusal::new(
            StoreRefusalCause::TooLarge,
            format!("a carried skill holds more than {SKILL_FILE_LIMIT} files"),
        ));
    }
    let mut bytes = 0u64;
    for file in files {
        let parts = file.path.split('/').collect::<Vec<_>>();
        if parts.iter().any(|part| {
            part.is_empty() || *part == "." || *part == ".." || part.contains(['\\', ':', '\0'])
        }) {
            return Err(refused(format!(
                "{} is not a path inside a skill",
                file.path
            )));
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(&file.data)
            .map_err(|error| refused(format!("{} could not be read: {error}", file.path)))?;
        bytes += data.len() as u64;
        if bytes > SKILL_BYTE_LIMIT {
            return Err(StoreRefusal::new(
                StoreRefusalCause::TooLarge,
                format!(
                    "a carried skill holds more than {} MiB",
                    SKILL_BYTE_LIMIT / 1024 / 1024
                ),
            ));
        }
        let target = parts
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part));
        if let Some(parent) = target.parent() {
            hide_platform::fs::private::create_dir_all(parent).map_err(|error| {
                refused(format!(
                    "{} could not be created: {error}",
                    parent.display()
                ))
            })?;
        }
        std::fs::write(&target, &data).map_err(|error| {
            refused(format!(
                "{} could not be written: {error}",
                target.display()
            ))
        })?;
        if file.executable {
            hide_platform::fs::permissions::mark_executable(&target).map_err(|error| {
                refused(format!(
                    "{} could not be made runnable: {error}",
                    target.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// Copies the store's folder `from` into `folder` as `name`, written
/// beside its final name and renamed into place, so an agent never reads
/// half a skill. With `replace`, what stands there goes first.
fn copy_into(from: &Path, folder: &Path, name: &str, replace: bool) -> Result<(), String> {
    std::fs::create_dir_all(folder)
        .map_err(|error| format!("{} could not be created: {error}", folder.display()))?;
    let to = folder.join(name);
    let half_written = format!(".{name}.hide-incoming-");
    let incoming = folder.join(format!("{half_written}{}", std::process::id()));
    // A copy a stopped pass left half written goes first: passes run one
    // at a time under the account's lock, so no other is writing it.
    if let Ok(found) = std::fs::read_dir(folder) {
        for stale in found.flatten() {
            if stale
                .file_name()
                .to_string_lossy()
                .starts_with(&half_written)
            {
                let _ = std::fs::remove_dir_all(stale.path());
            }
        }
    }
    copy_tree(from, &incoming).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&incoming);
    })?;
    if replace && std::fs::symlink_metadata(&to).is_ok() {
        remove_at(&to)?;
    }
    std::fs::rename(&incoming, &to).map_err(|error| {
        let _ = std::fs::remove_dir_all(&incoming);
        format!("{} could not be written: {error}", to.display())
    })
}

fn key(path: &Path) -> String {
    format!("skill:{}", path.display())
}

/// Lists `place`, or adds its columns to the place already listed for the
/// same real folder; that folder is blocked only while every spelling of it
/// is.
fn join(places: &mut Vec<Place>, place: Place) {
    match places.iter_mut().find(|listed| listed.key == place.key) {
        Some(listed) => {
            for column in place.columns {
                if !listed.governed_by(&column) {
                    listed.columns.push(column);
                }
            }
            if place.blocked.is_none() {
                listed.blocked = None;
            }
        }
        None => places.push(place),
    }
}

/// The real folder `path` names, links on the way followed as the agents
/// that read it follow them; the parts not there yet are spelled as given.
pub(super) fn real_folder(path: &Path) -> PathBuf {
    let mut there = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = hide_platform::fs::identity::canonical(there) {
            return rest.iter().rev().fold(real, |at, part| at.join(part));
        }
        match (there.parent(), there.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                there = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// What is at `at`: a skill folder, or a link to one, with its digest.
fn entry_at(at: &Path, fresh: bool) -> Result<Option<Entry>, String> {
    let metadata = match std::fs::symlink_metadata(at) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be read: {error}", at.display())),
    };
    if metadata.file_type().is_symlink() {
        let target = hide_platform::fs::identity::canonical(at)
            .map_err(|error| format!("{} could not be followed: {error}", at.display()))?;
        let (digest, whole) = facts(&target, fresh)?;
        return Ok(Some(Entry {
            digest,
            link: Some(target),
            whole,
            keys: None,
            server: None,
        }));
    }
    let (digest, whole) = facts(at, fresh)?;
    Ok(Some(Entry {
        digest,
        link: None,
        whole,
        keys: None,
        server: None,
    }))
}

/// At most this many folders' digests are remembered for the status read
/// every few seconds; past it the one read longest ago goes.
pub(crate) const REMEMBERED_LIMIT: usize = 512;

/// A folder's digest, with the stat fingerprint its files had when read.
struct Remembered {
    fingerprint: String,
    digest: String,
    read: u64,
}

/// The digests the status read reuses, by folder, and a counter that
/// orders their reads.
static REMEMBERED: Mutex<(u64, BTreeMap<PathBuf, Remembered>)> = Mutex::new((0, BTreeMap::new()));

/// The digest of the skill folder at `root`, and whether it is whole
/// ([`Entry::whole`]). With `fresh` false its files are read only when its
/// stat fingerprint moved since they were last read.
fn facts(root: &Path, fresh: bool) -> Result<(String, bool), String> {
    if !root.is_dir() {
        // A file where a skill folder would be: its bytes say what it is.
        let bytes = std::fs::read(root)
            .map_err(|error| format!("{} could not be read: {error}", root.display()))?;
        return Ok((format!("file:{}", hex(&Sha256::digest(bytes))), true));
    }
    let tree = walk(root).map_err(|refusal| refusal.reason)?;
    let whole = !tree.left_out;
    let fingerprint = stat_digest(&tree.files);
    if !fresh {
        let remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(seen) = remembered.1.get(root)
            && seen.fingerprint == fingerprint
        {
            return Ok((seen.digest.clone(), whole));
        }
    }
    let digest = hashed(&tree.files).map_err(|refusal| refusal.reason)?.0;
    remember(root, fingerprint, &digest);
    Ok((digest, whole))
}

fn remember(root: &Path, fingerprint: String, digest: &str) {
    let mut remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
    let (reads, folders) = &mut *remembered;
    *reads += 1;
    folders.insert(
        root.to_path_buf(),
        Remembered {
            fingerprint,
            digest: digest.to_owned(),
            read: *reads,
        },
    );
    while folders.len() > REMEMBERED_LIMIT {
        let Some(oldest) = folders
            .iter()
            .min_by_key(|(_, seen)| seen.read)
            .map(|(path, _)| path.clone())
        else {
            break;
        };
        folders.remove(&oldest);
    }
}

/// Takes the entry at `at` out: a link only as a link, never what it leads
/// to.
fn remove_at(at: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(at)
        .map_err(|error| format!("{} could not be read: {error}", at.display()))?;
    let removed = if metadata.file_type().is_symlink() {
        hide_platform::fs::link::remove_link(at)
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(at)
    } else {
        std::fs::remove_file(at)
    };
    removed.map_err(|error| format!("{} could not be removed: {error}", at.display()))
}

/// A skill folder's files, and whether anything in it was left out.
struct Tree {
    files: Vec<Walked>,
    left_out: bool,
}

/// One file of a skill folder.
struct Walked {
    relative: String,
    path: PathBuf,
    size: u64,
    modified: Option<std::time::SystemTime>,
}

/// Every file under `root`, sorted, within the skill limits: a `.git` folder
/// and any link inside are left out, so a copy never reaches outside, and
/// the tree says whether there was one.
fn walk(root: &Path) -> Result<Tree, StoreRefusal> {
    let failed = |path: &Path, error: std::io::Error| {
        StoreRefusal::new(
            StoreRefusalCause::Failed,
            format!("{} could not be read: {error}", path.display()),
        )
    };
    let mut files = Vec::new();
    let mut left_out = false;
    let mut bytes = 0u64;
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = pending.pop() {
        let entries = std::fs::read_dir(&folder).map_err(|error| failed(&folder, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| failed(&folder, error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|error| failed(&entry.path(), error))?;
            if metadata.file_type().is_symlink() {
                left_out = true;
                continue;
            }
            if metadata.is_dir() {
                if name == ".git" {
                    left_out = true;
                } else {
                    pending.push((entry.path(), relative));
                }
                continue;
            }
            bytes += metadata.len();
            files.push(Walked {
                relative,
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
            if files.len() > SKILL_FILE_LIMIT || bytes > SKILL_BYTE_LIMIT {
                return Err(StoreRefusal::new(
                    StoreRefusalCause::TooLarge,
                    format!(
                        "{} holds more than {SKILL_FILE_LIMIT} files or {} MiB",
                        root.display(),
                        SKILL_BYTE_LIMIT / 1024 / 1024
                    ),
                ));
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(Tree { files, left_out })
}

fn executable(path: &Path) -> bool {
    hide_platform::fs::permissions::is_executable(path).unwrap_or(false)
        || std::fs::File::open(path)
            .and_then(|mut file| {
                let mut head = [0u8; 2];
                std::io::Read::read_exact(&mut file, &mut head).map(|()| head == *b"#!")
            })
            .unwrap_or(false)
}

/// The skill folder `root`'s facts: its files and the digest of every
/// file's name, whether it runs, and its bytes.
pub(crate) fn body_of(root: &Path) -> Result<SkillBody, StoreRefusal> {
    let (digest, files) = hashed(&walk(root)?.files)?;
    let description = std::fs::read_to_string(root.join("SKILL.md"))
        .ok()
        .and_then(|text| front_matter(&text, "description"));
    Ok(SkillBody {
        digest,
        files,
        description,
    })
}

/// The digest of `files`' names, whether each runs, and their bytes, with
/// each file's facts.
fn hashed(files: &[Walked]) -> Result<(String, Vec<FileEntry>), StoreRefusal> {
    let mut hash = Sha256::new();
    let mut entries = Vec::with_capacity(files.len());
    for file in files {
        let bytes = std::fs::read(&file.path).map_err(|error| {
            StoreRefusal::new(
                StoreRefusalCause::Failed,
                format!("{} could not be read: {error}", file.path.display()),
            )
        })?;
        let runs = executable(&file.path);
        hash.update(file.relative.as_bytes());
        hash.update([0, u8::from(runs)]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
        entries.push(FileEntry {
            path: file.relative.clone(),
            size: file.size,
            executable: runs,
        });
    }
    Ok((hex(&hash.finalize()), entries))
}

/// The digest of the skill folder at `root` ([`body_of`]), read now.
pub(crate) fn digest(root: &Path) -> Result<String, String> {
    facts(root, true).map(|(digest, _)| digest)
}

/// A digest of every file's name, size and time under `root`, read without
/// opening a file, which a follow pass compares; `None` when the folder is
/// gone.
pub(crate) fn fingerprint(root: &Path) -> Result<Option<String>, String> {
    if !root.is_dir() {
        return Ok(None);
    }
    let tree = walk(root).map_err(|refusal| refusal.reason)?;
    Ok(Some(stat_digest(&tree.files)))
}

/// A digest of `files`' names, sizes and times, read without opening one.
fn stat_digest(files: &[Walked]) -> String {
    let mut hash = Sha256::new();
    for file in files {
        hash.update(file.relative.as_bytes());
        hash.update(file.size.to_le_bytes());
        let modified = file
            .modified
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_nanos())
            .unwrap_or(0);
        hash.update(modified.to_le_bytes());
    }
    hex(&hash.finalize())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    let files = walk(from).map_err(|refusal| refusal.reason)?.files;
    std::fs::create_dir_all(to)
        .map_err(|error| format!("{} could not be created: {error}", to.display()))?;
    for file in files {
        let target = to.join(&file.relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
        }
        // `copy` keeps the file's permission bits, so what ran still runs.
        std::fs::copy(&file.path, &target)
            .map_err(|error| format!("{} could not be copied: {error}", file.path.display()))?;
    }
    Ok(())
}

/// Copies the skill folder `from` into the store folder `into`, replacing
/// what was there whole only once the copy is complete, and answers what it
/// copied.
pub(crate) fn take_in(from: &Path, into: &Path) -> Result<SkillBody, StoreRefusal> {
    let failed = |reason: String| StoreRefusal::new(StoreRefusalCause::Failed, reason);
    if !from.join("SKILL.md").is_file() {
        return Err(StoreRefusal::new(
            StoreRefusalCause::NoSkills,
            format!("{} holds no SKILL.md", from.display()),
        ));
    }
    let parent = into
        .parent()
        .ok_or_else(|| failed(format!("{} has no parent folder", into.display())))?;
    hide_platform::fs::private::create_dir_all(parent).map_err(|error| {
        failed(format!(
            "{} could not be created: {error}",
            parent.display()
        ))
    })?;
    let name = into
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let incoming = parent.join(format!(".{name}.incoming-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&incoming);
    copy_tree(from, &incoming).map_err(|reason| {
        let _ = std::fs::remove_dir_all(&incoming);
        failed(reason)
    })?;
    let body = body_of(&incoming)?;
    if into.exists() {
        std::fs::remove_dir_all(into).map_err(|error| {
            failed(format!("{} could not be replaced: {error}", into.display()))
        })?;
    }
    std::fs::rename(&incoming, into)
        .map_err(|error| failed(format!("{} could not be written: {error}", into.display())))?;
    Ok(body)
}

/// One skill a source holds: its name, its folder relative to the source
/// root (`/` between names) and where it is.
pub(crate) struct Discovered {
    pub(crate) name: String,
    pub(crate) relative: String,
    pub(crate) path: PathBuf,
}

/// The skills under `root`: `root` itself when it holds a `SKILL.md`, else
/// each folder below it that does, at most four levels down; a skill's own
/// folders are its own and are not searched.
pub(crate) fn discover(root: &Path, fallback_name: &str) -> Result<Vec<Discovered>, String> {
    if root.join("SKILL.md").is_file() {
        let name = std::fs::read_to_string(root.join("SKILL.md"))
            .ok()
            .and_then(|text| front_matter(&text, "name"))
            .filter(|name| super::valid_name(name))
            .unwrap_or_else(|| fallback_name.to_owned());
        return Ok(vec![Discovered {
            name,
            relative: String::new(),
            path: root.to_path_buf(),
        }]);
    }
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((folder, prefix, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => return Err(format!("{} could not be read: {error}", folder.display())),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = std::fs::symlink_metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
            if !is_dir || name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if entry.path().join("SKILL.md").is_file() {
                if super::valid_name(&name) {
                    found.push(Discovered {
                        name,
                        relative,
                        path: entry.path(),
                    });
                }
            } else if depth < 4 {
                pending.push((entry.path(), relative, depth + 1));
            }
        }
    }
    found.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(found)
}

/// A field of a `SKILL.md`'s front matter, unquoted.
pub(crate) fn front_matter(text: &str, field: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    let (head, _) = body.split_once("\n---")?;
    head.lines().find_map(|line| {
        let value = line.strip_prefix(field)?.strip_prefix(':')?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_owned())
    })
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
