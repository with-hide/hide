//! The one-time naming of dialed devices by their machines' own ids.
//!
//! A machine is named by its machine id wherever it is seen: the core's own
//! node id is its machine id (`NodeId::of_this_machine`), and a device is
//! registered under the id its node reports (`model::DeviceNaming`). An
//! earlier build registered a dialed device under an id its SSH alias made
//! (`mini`), and every key naming the device carries that id: its projects'
//! digests, its panes' addresses, its letters, its Project Memory, its
//! engine folder. This renames each such registration once, at the start,
//! before the core reads the folder, through the same `KEYS` table a core
//! move maps (`reown`), so the device keeps its projects, letters and
//! Memory cards under its machine id.
//!
//! The id is the machine id the device's consent bound
//! (`HostConsent::identity`), which the bootstrap probe read on the device
//! before anything was installed; every later connection checks that the
//! node reports the same (`runtime::device_names`). A registration whose
//! consent bound none is left for its next connection to bind one, and the
//! start after it to rename. One whose consent bound the id it already has
//! is only marked named.
//!
//! Each device is checked against every store before anything is written,
//! and a device a check refuses stays as it was, under its old id, with the
//! reason in the outcome; the core holds it unconnected, so nothing new is
//! written under the old id. The plan is written to `device-ids.json`
//! first, then Project Memory, the link record, the JSON stores, the engine
//! folders and `core-state.json`, which holds the registrations, last; then
//! the plan is removed. Every step leaves what it already renamed as it is,
//! so a start that finds the plan resumes it to the same result. Each
//! store's original is kept once in `node-migration-backup/device-ids-<ms>/`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::reown::{self, Change, IdTable};
use super::{
    BACKUP_DIR, Backup, COMPOSER_MARKS, CORE_STATE, DELIVERY_LEDGER, HOOK_FACTS, KEYS, LABELS,
    LINKS, LOCAL_ISSUES, MISSIONS, Moves, PROJECT_MEMORY, Refusal, SESSION_SEARCH, WORKSPACE_VIEWS,
    read_json, write,
};
use crate::address::{Kind, Scoped};
use crate::node::NodeId;

/// The plan of a renaming in progress, beside the stores it renames.
pub const JOURNAL: &str = "device-ids.json";
const JOURNAL_VERSION: u32 = 1;

/// The JSON stores renamed before `core-state.json`, in the order written.
const STORES: &[&str] = &[
    DELIVERY_LEDGER,
    LABELS,
    WORKSPACE_VIEWS,
    MISSIONS,
    LOCAL_ISSUES,
    COMPOSER_MARKS,
    HOOK_FACTS,
];

/// What a start's renaming did, for the daemon's diagnostic record.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct DeviceNames {
    /// Each renamed device's old id and machine id.
    pub renamed: Vec<(String, String)>,
    /// The devices whose id already was their machine id, now marked named.
    pub named: Vec<String>,
    /// Each device a check kept under its old id, with why.
    pub held: Vec<(String, String)>,
    /// The stores written.
    pub files: Vec<String>,
}

/// `device-ids.json`: everything a resumed start needs that the stores it
/// already renamed no longer say.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
struct Journal {
    version: u32,
    /// The backup folder's name under `node-migration-backup/`.
    backup: String,
    /// Each device's old id and its machine id.
    renames: BTreeMap<String, String>,
    /// The devices only marked named.
    named: Vec<String>,
    /// Each registered project id of a renamed device and its new one.
    registrations: BTreeMap<String, String>,
    /// Each Project id of a renamed device (`hide_project::project_id`) and
    /// its new one, for the retention the session search file keeps under
    /// the old (`links::store::policies`).
    projects: Vec<(String, String)>,
}

/// The renaming as `reown`'s mapper reads it.
struct Renames<'a>(&'a BTreeMap<String, String>);

impl Change for Renames<'_> {
    fn device(&self, id: &str) -> Option<String> {
        self.0.get(id).cloned()
    }

    fn scoped(&self, kind: Kind, id: &str) -> Option<String> {
        let scoped = Scoped::parse(id).filter(|scoped| scoped.kind == kind)?;
        let to = self.0.get(scoped.device)?;
        Some(crate::address::scoped_id(to, kind, scoped.id))
    }

    /// A checkout's id is a digest of its path alone, so it keeps its digest.
    fn checkout(&self, id: &str) -> Option<String> {
        self.scoped(Kind::Checkout, id)
    }

    fn lacks(&self, id: &str) -> bool {
        crate::address::device_of(id).is_some_and(|device| self.0.contains_key(device))
    }
}

/// Renames the dialed devices of the folder at `state_dir`, whose core is
/// `node`, to their machine ids. A failure to write stops the start and
/// names the file; the plan stays, and the next start resumes it.
pub fn name_devices(state_dir: &Path, node: &NodeId) -> Result<DeviceNames, Refusal> {
    let journal_path = state_dir.join(JOURNAL);
    let mut outcome = DeviceNames::default();
    let journal =
        match read_journal(&journal_path).map_err(|reason| refuse(&journal_path, reason))? {
            Some(journal) => journal,
            None => {
                let Some(journal) = plan(state_dir, node, &mut outcome)? else {
                    return Ok(outcome);
                };
                if !journal.renames.is_empty() {
                    let bytes = serde_json::to_vec(&journal)
                        .map_err(|error| refuse(&journal_path, error.to_string()))?;
                    write(&journal_path, &bytes).map_err(|reason| refuse(&journal_path, reason))?;
                }
                journal
            }
        };
    outcome.renamed = journal
        .renames
        .iter()
        .map(|(from, to)| (from.clone(), to.clone()))
        .collect();
    outcome.named = journal.named.clone();
    apply(state_dir, &journal, &mut outcome)?;
    match std::fs::remove_file(&journal_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(refuse(&journal_path, error.to_string())),
    }
    Ok(outcome)
}

fn refuse(file: &Path, reason: String) -> Refusal {
    Refusal {
        file: file.to_path_buf(),
        reason,
    }
}

fn read_journal(path: &Path) -> Result<Option<Journal>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let journal: Journal = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the device renaming plan is not readable: {error}"))?;
    if journal.version != JOURNAL_VERSION {
        return Err(format!(
            "the device renaming plan has version {}, which this build does not know",
            journal.version
        ));
    }
    Ok(Some(journal))
}

/// One dialed device an earlier build registered under its alias's id.
struct Candidate {
    from: String,
    to: String,
}

/// The renaming this start makes, or `None` when there is nothing to do.
/// Every check runs here, before anything is written; a device that fails
/// one is held under its old id.
fn plan(
    state_dir: &Path,
    node: &NodeId,
    outcome: &mut DeviceNames,
) -> Result<Option<Journal>, Refusal> {
    let core_path = state_dir.join(CORE_STATE);
    let Some(core) = read_json(&core_path).map_err(|reason| refuse(&core_path, reason))? else {
        return Ok(None);
    };
    let rows: Vec<&Value> = core
        .get("device_registrations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let id_of = |row: &Value| row.get("id").and_then(Value::as_str).map(str::to_owned);
    let ids: BTreeSet<String> = rows.iter().filter_map(|row| id_of(row)).collect();
    let bound = |row: &Value| {
        row.pointer("/host_consent/identity/machine_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    // Every machine id a consent bound, with how many registrations bound it.
    let mut bound_by: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        if let Some(machine) = bound(row) {
            *bound_by.entry(machine).or_default() += 1;
        }
    }
    let mut named = Vec::new();
    let mut candidates = Vec::new();
    for row in &rows {
        // Only a dialed device an earlier build wrote, which says no naming.
        let legacy = row.get("naming").is_none_or(Value::is_null)
            && row.get("ssh_alias").is_some_and(Value::is_string);
        let (Some(from), Some(to)) = (id_of(row), bound(row)) else {
            continue;
        };
        if !legacy {
            continue;
        }
        if from == to {
            named.push(from);
            continue;
        }
        let held = match NodeId::parse(&to) {
            Err(error) => Some(format!("its machine id {to:?} is not an id: {error}")),
            Ok(_) if to == node.as_str() => {
                Some(format!("its machine id {to} is this core's own machine"))
            }
            Ok(_) if ids.contains(&to) => Some(format!("another device is registered as {to}")),
            Ok(_) if bound_by.get(&to).copied().unwrap_or_default() > 1 => Some(format!(
                "another registration reaches the same machine {to}"
            )),
            Ok(_) => None,
        };
        match held {
            Some(reason) => outcome.held.push((from, reason)),
            None => candidates.push(Candidate { from, to }),
        }
    }
    if candidates.is_empty() && named.is_empty() {
        return Ok(None);
    }

    let stores = Stores::read(state_dir, true)?;
    let mut renames = BTreeMap::new();
    let mut registrations = BTreeMap::new();
    for candidate in candidates {
        let mut trial = renames.clone();
        trial.insert(candidate.from.clone(), candidate.to.clone());
        let checked = registration_ids(&core, &stores, &candidate).and_then(|table| {
            let mut all = registrations.clone();
            all.extend(table.clone());
            check(state_dir, &stores, &candidate, &trial, &all, &named)?;
            Ok(table)
        });
        match checked {
            Ok(table) => {
                registrations.extend(table);
                renames = trial;
            }
            Err(reason) => outcome.held.push((candidate.from, reason)),
        }
    }
    if renames.is_empty() && named.is_empty() {
        return Ok(None);
    }
    let projects = project_pairs(&stores, &renames, &[]);
    Ok(Some(Journal {
        version: JOURNAL_VERSION,
        backup: format!("device-ids-{}", now_ms()),
        renames,
        named,
        registrations,
        projects,
    }))
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}

/// What the checks read of the folder, once.
struct Stores {
    /// Each JSON store as it is, by file name.
    json: BTreeMap<&'static str, Value>,
    /// The Project roots each device has in Project Memory, or why the
    /// store does not open, which holds every device: its Projects could
    /// not follow it.
    memory: Result<BTreeMap<String, BTreeSet<String>>, String>,
    /// The link record, when there is one that opens; one that does not is
    /// left for its worker, which sets it aside and rebuilds it.
    links: Option<crate::links::store::LinkStore>,
    /// The project roots each device has in the link record.
    link_roots: BTreeMap<String, BTreeSet<String>>,
}

impl Stores {
    fn read(state_dir: &Path, json: bool) -> Result<Self, Refusal> {
        let mut stores = BTreeMap::new();
        for file in STORES.iter().chain([&CORE_STATE]).filter(|_| json) {
            let path = state_dir.join(file);
            if let Some(value) = read_json(&path).map_err(|reason| refuse(&path, reason))? {
                stores.insert(*file, value);
            }
        }
        let memory = state_dir.join(PROJECT_MEMORY);
        let memory = if memory.is_file() {
            crate::runtime::memory::open_memory_store(&memory)
                .map_err(|error| format!("Project Memory does not open: {error}"))
                .and_then(|_| rows_of(&memory, "SELECT device_id, root FROM projects"))
        } else {
            Ok(BTreeMap::new())
        };
        // A link record that does not open is not renamed yet: the open sets
        // a corrupt file aside and makes a new one itself, so what it refuses
        // (busy, newer, full, unavailable) is a record that still names the
        // old ids, and the start stops with the journal kept.
        let links_path = state_dir.join(LINKS);
        let links = if links_path.is_file() {
            let (store, _) = crate::links::store::LinkStore::open(&links_path).map_err(|code| {
                refuse(
                    &links_path,
                    format!("the link record does not open: {code}"),
                )
            })?;
            Some(store)
        } else {
            None
        };
        let link_roots = match &links {
            Some(_) => rows_of(&links_path, "SELECT device, root FROM projects")
                .map_err(|reason| refuse(&links_path, reason))?,
            None => BTreeMap::new(),
        };
        Ok(Self {
            json: stores,
            memory,
            links,
            link_roots,
        })
    }

    fn roots(&self, device: &str) -> BTreeSet<String> {
        let mut roots: BTreeSet<String> = self
            .link_roots
            .get(device)
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        if let Ok(memory) = &self.memory {
            roots.extend(memory.get(device).into_iter().flatten().cloned());
        }
        roots
    }
}

/// Each device's roots, read without writing.
fn rows_of(path: &Path, query: &str) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let failed = |error: rusqlite::Error| error.to_string();
    let connection = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(failed)?;
    let mut statement = connection.prepare(query).map_err(failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(failed)?;
    let mut roots: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        let (device, root) = row.map_err(failed)?;
        roots.entry(device).or_default().insert(root);
    }
    Ok(roots)
}

/// Each registered project id of the device, and every other project id of
/// it a root the folder knows makes, with the id its machine id gives the
/// same root. A registered project whose root no store knows refuses the
/// device: its id is a digest no id can be made from again.
fn registration_ids(
    core: &Value,
    stores: &Stores,
    candidate: &Candidate,
) -> Result<BTreeMap<String, String>, String> {
    let mut roots = stores.roots(&candidate.from);
    let registered: Vec<(String, String)> = core
        .get("workspace_registrations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("device_id").and_then(Value::as_str) == Some(&candidate.from))
        .filter_map(|row| {
            Some((
                row.get("id")?.as_str()?.to_owned(),
                row.get("path")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    roots.extend(registered.iter().map(|(_, path)| path.clone()));
    let table: BTreeMap<String, String> = roots
        .iter()
        .map(|root| {
            (
                crate::device_catalog::project_id(&candidate.from, Path::new(root)),
                crate::device_catalog::project_id(&candidate.to, Path::new(root)),
            )
        })
        .collect();
    if let Some((id, path)) = registered.iter().find(|(id, _)| !table.contains_key(id)) {
        return Err(format!(
            "its project {id} at {path} has a root no store knows, so its new id cannot be made"
        ));
    }
    Ok(table)
}

/// Whether the device can be renamed with the ones already accepted: no
/// store holds anything under its machine id yet, and every JSON store
/// renames to a file its loader reads with nothing left naming the old id.
fn check(
    state_dir: &Path,
    stores: &Stores,
    candidate: &Candidate,
    renames: &BTreeMap<String, String>,
    registrations: &BTreeMap<String, String>,
    named: &[String],
) -> Result<(), String> {
    let (from, to) = (candidate.from.as_str(), candidate.to.as_str());
    let memory = stores.memory.as_ref().map_err(Clone::clone)?;
    // Project Memory keeps card numbers only by a strict re-key, which a
    // Project already stored under the new id stops.
    let theirs = memory.get(to).cloned().unwrap_or_default();
    if let Some(root) = memory
        .get(from)
        .into_iter()
        .flatten()
        .find(|root| theirs.contains(*root))
    {
        return Err(format!(
            "Project Memory already holds the Project at {root} under its machine id {to}"
        ));
    }
    if let Some(links) = &stores.links
        && links.has_device(to)?
    {
        return Err(format!(
            "the link record already holds rows under its machine id {to}"
        ));
    }
    let folder = |id: &str| hide_kit::layout::device_engine_dir(state_dir, id);
    let old_folder = folder(from)?;
    if old_folder.exists() && folder(to)?.exists() {
        return Err(format!(
            "its engine folder and one for its machine id {to} both exist"
        ));
    }
    let ids = IdTable {
        registrations: registrations.clone(),
        checkouts: BTreeMap::new(),
    };
    for (file, value) in &stores.json {
        let mut value = value.clone();
        rename_store(file, &mut value, renames, &ids, named)
            .map_err(|reason| format!("{file}: {reason}"))?;
        if *file == DELIVERY_LEDGER {
            let size = serde_json::to_vec(&value)
                .map_err(|error| error.to_string())?
                .len();
            if size > crate::delivery::FILE_LIMIT {
                return Err(format!(
                    "{file} would be {size} bytes, over the {} its loader reads",
                    crate::delivery::FILE_LIMIT
                ));
            }
        }
    }
    Ok(())
}

/// The `(old, new)` Project ids of the renamed devices, for the retention
/// the session search file keeps under the old: from the roots either id
/// has now, so a resumed start finds the ones an earlier step re-keyed,
/// with the ones the plan recorded.
fn project_pairs(
    stores: &Stores,
    renames: &BTreeMap<String, String>,
    recorded: &[(String, String)],
) -> Vec<(String, String)> {
    let mut pairs: BTreeSet<(String, String)> = recorded.iter().cloned().collect();
    for (from, to) in renames {
        for root in stores.roots(from).into_iter().chain(stores.roots(to)) {
            pairs.insert((
                hide_project::project_id(from, Path::new(&root)),
                hide_project::project_id(to, Path::new(&root)),
            ));
        }
    }
    pairs.into_iter().collect()
}

/// Renames every key of one JSON store, as `KEYS` places them, and fails
/// when anything still names an old id after.
fn rename_store(
    file: &str,
    value: &mut Value,
    renames: &BTreeMap<String, String>,
    ids: &IdTable,
    named: &[String],
) -> Result<(), String> {
    let change = Renames(renames);
    let map = |moves: Moves, text: &str| reown::map_id(&change, ids, moves, text);
    for (_, pattern, _, moves) in KEYS.iter().filter(|(listed, ..)| *listed == file) {
        reown::apply_pattern(value, pattern, *moves, &map)
            .map_err(|reason| format!("{pattern}: {reason}"))?;
    }
    match file {
        CORE_STATE => {
            for (_, devices) in reown::OWN_PATHS {
                reown::apply_pattern(value, &format!("/{devices}/{{key}}"), Moves::Device, &map)?;
            }
            reown::map_project_settings(value, &|device| change.device(device))?;
            name_registrations(value, renames, named)?;
        }
        WORKSPACE_VIEWS => reown::map_views(value, &map),
        LABELS => {
            for section in ["devices", "moved"] {
                rename_keys(value, section, |key| {
                    let device = key.strip_prefix("device:")?;
                    Some(crate::labels::device_target(renames.get(device)?))
                })?;
            }
        }
        DELIVERY_LEDGER => {
            // A device's agents are scoped to their device's engine by its id.
            reown::apply_pattern(value, "/agents/*/host_scope", Moves::Device, &map)?;
            reown::apply_pattern(value, "/spawns/*/machine", Moves::Device, &map)?;
        }
        LOCAL_ISSUES => {
            reown::apply_pattern(value, "/devices/{key}", Moves::Device, &map)?;
        }
        _ => {}
    }
    for from in renames.keys() {
        if let Some(place) = reown::still_naming_id(value, from, true, "") {
            return Err(format!(
                "{place} still names {from} after the renaming; no row of the key table covers it"
            ));
        }
    }
    Ok(())
}

/// Each renamed registration takes its machine id and is named, and each
/// one whose id already was its machine id is named.
fn name_registrations(
    value: &mut Value,
    renames: &BTreeMap<String, String>,
    named: &[String],
) -> Result<(), String> {
    let Some(rows) = value
        .get_mut("device_registrations")
        .and_then(Value::as_array_mut)
    else {
        return Ok(());
    };
    let stored = crate::model::DeviceNaming::Named
        .stored()
        .expect("a named device is stored as named");
    for row in rows {
        let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let renamed = renames.get(&id).cloned();
        // A row an earlier step of this plan already renamed.
        let done = renames.values().any(|to| *to == id);
        if renamed.is_none() && !done && !named.contains(&id) {
            continue;
        }
        let Some(object) = row.as_object_mut() else {
            return Err(format!("device {id} is not a record"));
        };
        if let Some(to) = renamed {
            object.insert("id".to_owned(), Value::String(to));
        }
        object.insert("naming".to_owned(), Value::String(stored.to_owned()));
    }
    Ok(())
}

/// Renames the keys of the map at `section` that `rename` gives a new name.
fn rename_keys(
    value: &mut Value,
    section: &str,
    rename: impl Fn(&str) -> Option<String>,
) -> Result<(), String> {
    let Some(map) = value.get_mut(section) else {
        return Ok(());
    };
    let Some(map) = map.as_object_mut() else {
        return Err(format!("{section} is not a map"));
    };
    let mut renamed = Map::new();
    for (key, entry) in std::mem::take(map) {
        let key = rename(&key).unwrap_or(key);
        if renamed.insert(key.clone(), entry).is_some() {
            return Err(format!("two entries of {section} would both be {key}"));
        }
    }
    *map = renamed;
    Ok(())
}

/// Writes the plan, store by store, in its order.
fn apply(state_dir: &Path, journal: &Journal, outcome: &mut DeviceNames) -> Result<(), Refusal> {
    apply_steps(state_dir, journal, outcome, usize::MAX)
}

/// The first `steps` steps of the plan: Project Memory, the link record,
/// each JSON store, the engine folders, `core-state.json`. A start that
/// stops between two is what a test of resuming makes with fewer.
fn apply_steps(
    state_dir: &Path,
    journal: &Journal,
    outcome: &mut DeviceNames,
    mut steps: usize,
) -> Result<(), Refusal> {
    let mut next = || {
        let go = steps > 0;
        steps = steps.saturating_sub(1);
        go
    };
    let backup = Backup {
        directory: state_dir.join(BACKUP_DIR).join(&journal.backup),
    };
    let renames = &journal.renames;

    if !next() {
        return Ok(());
    }
    let memory = state_dir.join(PROJECT_MEMORY);
    if !renames.is_empty() && memory.is_file() {
        let store = crate::runtime::memory::open_memory_store(&memory)
            .map_err(|error| refuse(&memory, error.to_string()))?;
        let mut moved = false;
        for (from, to) in renames {
            if !store
                .has_device(from)
                .map_err(|error| refuse(&memory, error.to_string()))?
            {
                continue;
            }
            backup
                .copy_with(PROJECT_MEMORY, |destination| {
                    store
                        .copy_to(destination)
                        .map_err(|error| error.to_string())
                })
                .map_err(|reason| refuse(&memory, reason))?;
            store
                .convert_device(from, to)
                .map_err(|error| refuse(&memory, error.to_string()))?;
            moved = true;
        }
        if moved {
            outcome.files.push(PROJECT_MEMORY.to_owned());
        }
    }

    if !next() {
        return Ok(());
    }
    let links = state_dir.join(LINKS);
    if !renames.is_empty() && links.is_file() {
        let mut stores = Stores::read(state_dir, false)?;
        let pairs = project_pairs(&stores, renames, &journal.projects);
        if let Some(store) = stores.links.as_mut()
            && rename_links(store, &backup, renames, &pairs, state_dir)
                .map_err(|reason| refuse(&links, reason))?
        {
            outcome.files.push(LINKS.to_owned());
        }
    }

    let ids = IdTable {
        registrations: journal.registrations.clone(),
        checkouts: BTreeMap::new(),
    };
    for file in STORES.iter().chain([&CORE_STATE]) {
        if *file == CORE_STATE {
            if !next() {
                return Ok(());
            }
            rename_engine_folders(state_dir, renames)?;
        }
        if !next() {
            return Ok(());
        }
        let path = state_dir.join(file);
        let Some(mut value) = read_json(&path).map_err(|reason| refuse(&path, reason))? else {
            continue;
        };
        let before = value.clone();
        rename_store(file, &mut value, renames, &ids, &journal.named)
            .map_err(|reason| refuse(&path, reason))?;
        if value == before {
            continue;
        }
        let bytes = serde_json::to_vec(&value).map_err(|error| refuse(&path, error.to_string()))?;
        backup
            .copy(&path, file)
            .map_err(|reason| refuse(&path, reason))?;
        write(&path, &bytes).map_err(|reason| refuse(&path, reason))?;
        outcome.files.push((*file).to_owned());
    }
    Ok(())
}

/// The link record's rows and listing stamps of each renamed device, and
/// the retention its old Project ids kept; answers whether it changed.
fn rename_links(
    store: &mut crate::links::store::LinkStore,
    backup: &Backup,
    renames: &BTreeMap<String, String>,
    pairs: &[(String, String)],
    state_dir: &Path,
) -> Result<bool, String> {
    let listed = |device: &str| format!("{}:{device}", crate::links::worker::LISTED_AT);
    let mut pending = Vec::new();
    for (from, to) in renames {
        let rows = store.has_device(from)?;
        let stamp = store.meta(&listed(from))?;
        if rows || stamp.is_some() {
            pending.push((from, to, rows, stamp));
        }
    }
    let retention = state_dir.join(SESSION_SEARCH).is_file() && !pairs.is_empty();
    if pending.is_empty() && !retention {
        return Ok(false);
    }
    backup.copy_with(LINKS, |destination| store.copy_to(destination))?;
    let mut changed = false;
    if retention {
        changed |= store.alias_retention(pairs)? > 0;
    }
    for (from, to, rows, stamp) in pending {
        if rows {
            store.convert_device(from, to)?;
        }
        if let Some(stamp) = stamp {
            store.set_meta(&listed(to), &stamp)?;
            store.delete_meta(&listed(from))?;
        }
        changed = true;
    }
    Ok(changed)
}

/// Each renamed device's engine folder takes its machine id. A folder
/// already under the new id is a rename an earlier step made; both at once
/// is a folder something made under the new id after the checks, which
/// stops the start rather than lose either.
fn rename_engine_folders(
    state_dir: &Path,
    renames: &BTreeMap<String, String>,
) -> Result<(), Refusal> {
    for (from, to) in renames {
        let folder = |id: &str| -> Result<PathBuf, Refusal> {
            hide_kit::layout::device_engine_dir(state_dir, id)
                .map_err(|reason| refuse(state_dir, reason))
        };
        let (old, new) = (folder(from)?, folder(to)?);
        match (old.exists(), new.exists()) {
            (true, true) => {
                return Err(refuse(&old, format!("{} exists beside it", new.display())));
            }
            (true, false) => {
                std::fs::rename(&old, &new).map_err(|error| refuse(&old, error.to_string()))?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ALIAS: &str = "mini";
    const MACHINE: &str = "b06a260a-6c14-5287-9caf-672044350d94";
    const NODE: &str = "14ad0319-60d7-5a7c-ae5a-f71550dc8a60";
    const ROOT: &str = "/srv/p";
    const CHECKOUT: &str = "00000000000000aa";

    fn node() -> NodeId {
        NodeId::parse(NODE).unwrap()
    }

    fn project(device: &str) -> String {
        crate::device_catalog::project_id(device, Path::new(ROOT))
    }

    fn memory_project(device: &str) -> String {
        hide_project::project_id(device, Path::new(ROOT))
    }

    fn pane(device: &str) -> String {
        format!("remote:{device}:pane:w1:p1")
    }

    fn tab(device: &str) -> String {
        format!("remote:{device}:tab:w1:t1")
    }

    fn checkout(device: &str) -> String {
        format!("remote:{device}:checkout:{CHECKOUT}")
    }

    fn consent(machine: &str) -> Value {
        json!({"contract": 4, "helper_root": "~/.hide/host-helper", "granted_at_unix_ms": 1,
               "identity": {"user": "me", "hostname": "mini.local", "port": 22,
                            "machine_id": machine, "host_key_sha256": ""}})
    }

    fn write_json(state: &Path, file: &str, value: Value) {
        std::fs::write(state.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn read(state: &Path, file: &str) -> Value {
        serde_json::from_slice(&std::fs::read(state.join(file)).unwrap()).unwrap()
    }

    /// A folder an earlier build wrote for a core on `NODE` with the device
    /// `mini` registered under its alias: every store names it, its Project
    /// has a Memory card, and its engine keeps a session.
    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let registered = project(ALIAS);
        write_json(
            state,
            CORE_STATE,
            json!({
                "schema_version": 1,
                "device_registrations": [{"id": ALIAS, "label": "Mac mini", "ssh_alias": ALIAS,
                                          "host_consent": consent(MACHINE)}],
                "workspace_registrations": [{"id": registered, "device_id": ALIAS, "path": ROOT,
                                             "primary_checkout_id": checkout(ALIAS)}],
                "focused_device_id": ALIAS,
                "expanded_inactive_project_device_ids": [ALIAS],
                "recent_checkouts": [{"device_id": ALIAS, "checkout_id": checkout(ALIAS)}],
                "device_expanded_paths": {ALIAS: ["/srv/p/src"]},
                "device_agent_permissions": {ALIAS: {"codex": {"mode": "ask"}}},
                "selected_pane_id": pane(ALIAS),
                "session_open_folds": [format!("cleanup/{ALIAS}"), format!("asleep/{registered}")],
                "collapsed_workspace_ids": [registered],
                "pane_text_scales": {pane(ALIAS): 1.1},
                "operator_tab_names": {tab(ALIAS): "work"},
                "project_settings": {
                    crate::project_settings::key(ALIAS, ROOT): {"device_id": ALIAS, "root": ROOT}
                },
            }),
        );
        let actor = |device: &str, pane: &str| json!({"pane_id": pane, "name": "a", "kind": "claude", "device_id": device});
        write_json(
            state,
            DELIVERY_LEDGER,
            json!({
                "version": 1, "next_id": 3,
                "letters": [{"id": "letter-1", "sender": actor(NODE, "w9:p1"),
                             "recipient": actor(ALIAS, &pane(ALIAS))}],
                "watches": [{"parent": actor(NODE, "w9:p1"), "target": actor(ALIAS, &pane(ALIAS))}],
                "agents": [
                    {"id": "agent-0", "machine": NODE, "actor": actor(NODE, "w9:p1"), "host_scope": "/tmp/engine.sock"},
                    {"id": "agent-1", "machine": ALIAS, "actor": actor(ALIAS, &pane(ALIAS)), "host_scope": ALIAS, "pane": "w1:p1"},
                ],
                "spawns": [{"parent": "agent-0", "machine": ALIAS, "pane": "w1:p1"}],
                "node_clocks": {ALIAS: {"connected_ms": 60_000, "since": null}},
            }),
        );
        write_json(
            state,
            LABELS,
            json!({"version": 1, "targets": {NODE: {"w9:p1": {"owner": "v1:a"}}},
                   "devices": {crate::labels::device_target(ALIAS): {"w1:p1": {"owner": "v1:b"}}}}),
        );
        write_json(
            state,
            WORKSPACE_VIEWS,
            json!({"schema_version": 3, "workspaces": [{
                "device_id": ALIAS, "path": ROOT, "tab_views": {tab(ALIAS): "d1"},
                "view_layout": {"root": {"area": {"displays": [{"id": "d1", "opener": {"device_id": ALIAS, "terminal_id": "t1"}}]}}},
            }]}),
        );
        write_json(
            state,
            MISSIONS,
            json!({"schema_version": 1, "next": 2, "missions": [{
                "id": "mission-1", "device_id": ALIAS, "workspace_id": registered, "project": ROOT,
                "conductor": {"actor": actor(ALIAS, &pane(ALIAS)), "session": {"device_id": ALIAS, "agent": "claude", "session_id": "s"}},
            }]}),
        );
        write_json(
            state,
            LOCAL_ISSUES,
            json!({"version": 1, "projects": {}, "devices": {ALIAS: {ROOT: []}}}),
        );
        write_json(
            state,
            COMPOSER_MARKS,
            json!({"schema_version": 1, "marks": [{"pane_id": pane(ALIAS), "device_id": ALIAS, "session": "s", "since_unix_ms": 1}]}),
        );
        write_json(
            state,
            HOOK_FACTS,
            json!({"schema_version": 1, "panes": {pane(ALIAS): {"sessions": [], "seen_agent": true}}}),
        );

        let mut memory = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        memory
            .ensure_project(&memory_project(ALIAS), Path::new(ROOT), ALIAS)
            .unwrap();
        memory
            .set_enabled(&memory_project(ALIAS), true, true)
            .unwrap();
        for (index, text) in ["first card", "second card"].iter().enumerate() {
            memory
                .apply_candidates(
                    &hide_memory::AnalysisBatch {
                        id: format!("batch-{index}"),
                        project_id: memory_project(ALIAS),
                        provider: "codex".into(),
                        analysis_provider: "codex".into(),
                        session_id: "session-1".into(),
                        content_hash: (*text).into(),
                        created_at_unix_ms: 100,
                    },
                    &[hide_memory::Candidate {
                        text: (*text).into(),
                        title: String::new(),
                        aliases: Vec::new(),
                        kind: hide_memory::CandidateKind::Rule,
                        confidence: 0.9,
                        salience: 0.8,
                        source_offsets: vec![7],
                        human_offsets: vec![7],
                        source_times: BTreeMap::from([(7, 90)]),
                        source_words: Default::default(),
                        cited_paths: Vec::new(),
                        relation: hide_memory::CandidateRelation::New,
                    }],
                )
                .unwrap();
        }
        drop(memory);

        let search = rusqlite::Connection::open(state.join(SESSION_SEARCH)).unwrap();
        search
            .execute_batch("CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);")
            .unwrap();
        search
            .execute("INSERT INTO policy VALUES(?1, 30)", [memory_project(ALIAS)])
            .unwrap();
        drop(search);
        let (links, _) = crate::links::store::LinkStore::open(&state.join(LINKS)).unwrap();
        links
            .set_meta(
                &format!("{}:{ALIAS}", crate::links::worker::LISTED_AT),
                "200",
            )
            .unwrap();
        drop(links);
        rusqlite::Connection::open(state.join(LINKS))
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO projects VALUES('{key}','{ALIAS}','{ROOT}','id:R',NULL,1);
                 INSERT INTO sessions(device,agent,id) VALUES('{ALIAS}','claude','s1');",
                key = memory_project(ALIAS),
            ))
            .unwrap();

        let engine = hide_kit::layout::device_engine_dir(state, ALIAS).unwrap();
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::write(engine.join("session.json"), b"{\"tabs\":[]}").unwrap();
        dir
    }

    /// Each card of the device's Project: its number, text and lifecycle.
    fn cards(state: &Path, device: &str) -> Vec<(i64, String, String)> {
        let connection = rusqlite::Connection::open_with_flags(
            state.join(PROJECT_MEMORY),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT i.number, r.body, i.lifecycle FROM memory_items i \
                 JOIN memory_revisions r ON r.item_id=i.id AND r.revision=i.current_revision \
                 JOIN projects p ON p.id=i.project_id WHERE p.device_id=?1 ORDER BY i.number",
            )
            .unwrap();
        statement
            .query_map([device], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn link_devices(state: &Path) -> Vec<(String, String)> {
        let connection = rusqlite::Connection::open_with_flags(
            state.join(LINKS),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT 'projects', device FROM projects UNION ALL SELECT 'sessions', device FROM sessions ORDER BY 1",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// The link record's per-device listing stamps and its retention
    /// aliases.
    fn link_meta(state: &Path) -> Vec<(String, String)> {
        let connection = rusqlite::Connection::open_with_flags(
            state.join(LINKS),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT key, value FROM meta WHERE key LIKE ?1 UNION ALL \
                 SELECT 'retention:' || old, new FROM retention_aliases ORDER BY 1",
            )
            .unwrap();
        statement
            .query_map([format!("{}:%", crate::links::worker::LISTED_AT)], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// Every JSON store, Memory's cards and the link record's devices, and
    /// its listing stamps and retention.
    type Contents = (
        BTreeMap<String, Value>,
        Vec<(i64, String, String)>,
        Vec<(String, String)>,
        Vec<(String, String)>,
    );

    /// What a start reads of the folder at `state`.
    fn contents(state: &Path) -> Contents {
        let json = STORES
            .iter()
            .chain([&CORE_STATE])
            .map(|file| ((*file).to_owned(), read(state, file)))
            .collect();
        (
            json,
            cards(state, MACHINE),
            link_devices(state),
            link_meta(state),
        )
    }

    #[test]
    fn a_dialed_device_takes_its_machine_id_in_every_store_and_keeps_its_cards() {
        let dir = folder();
        let state = dir.path();
        let before_cards = cards(state, ALIAS);
        assert_eq!(before_cards.len(), 2);
        let originals: BTreeMap<&str, Vec<u8>> = STORES
            .iter()
            .chain([&CORE_STATE])
            .map(|file| (*file, std::fs::read(state.join(file)).unwrap()))
            .collect();

        let outcome = name_devices(state, &node()).unwrap();
        assert_eq!(
            outcome.renamed,
            vec![(ALIAS.to_owned(), MACHINE.to_owned())]
        );
        assert!(outcome.held.is_empty(), "{outcome:?}");

        let core = read(state, CORE_STATE);
        let registration = &core["device_registrations"][0];
        assert_eq!(registration["id"], MACHINE);
        assert_eq!(registration["naming"], "named");
        // The alias and the name the operator gave it stay theirs.
        assert_eq!(registration["ssh_alias"], ALIAS);
        assert_eq!(registration["label"], "Mac mini");
        let workspace = &core["workspace_registrations"][0];
        assert_eq!(workspace["id"], json!(project(MACHINE)));
        assert_eq!(workspace["device_id"], MACHINE);
        assert_eq!(workspace["primary_checkout_id"], json!(checkout(MACHINE)));
        assert_eq!(core["focused_device_id"], MACHINE);
        assert_eq!(
            core["expanded_inactive_project_device_ids"],
            json!([MACHINE])
        );
        assert_eq!(
            core["device_expanded_paths"],
            json!({MACHINE: ["/srv/p/src"]})
        );
        assert_eq!(
            core["device_agent_permissions"],
            json!({MACHINE: {"codex": {"mode": "ask"}}})
        );
        assert_eq!(core["selected_pane_id"], json!(pane(MACHINE)));
        assert_eq!(
            core["session_open_folds"],
            json!([
                format!("cleanup/{MACHINE}"),
                format!("asleep/{}", project(MACHINE))
            ])
        );
        assert_eq!(core["pane_text_scales"], json!({pane(MACHINE): 1.1}));
        assert_eq!(core["operator_tab_names"], json!({tab(MACHINE): "work"}));
        assert_eq!(
            core["project_settings"],
            json!({crate::project_settings::key(MACHINE, ROOT): {"device_id": MACHINE, "root": ROOT}})
        );

        let ledger = read(state, DELIVERY_LEDGER);
        assert_eq!(ledger["letters"][0]["id"], "letter-1");
        assert_eq!(ledger["letters"][0]["recipient"]["device_id"], MACHINE);
        assert_eq!(
            ledger["letters"][0]["recipient"]["pane_id"],
            json!(pane(MACHINE))
        );
        assert_eq!(ledger["letters"][0]["sender"]["device_id"], NODE);
        assert_eq!(ledger["watches"][0]["target"]["device_id"], MACHINE);
        assert_eq!(ledger["agents"][1]["machine"], MACHINE);
        assert_eq!(ledger["agents"][1]["host_scope"], MACHINE);
        assert_eq!(ledger["agents"][0]["host_scope"], "/tmp/engine.sock");
        assert_eq!(ledger["spawns"][0]["machine"], MACHINE);
        assert_eq!(
            ledger["node_clocks"],
            json!({MACHINE: {"connected_ms": 60_000, "since": null}})
        );
        assert_eq!(
            read(state, LABELS)["devices"],
            json!({crate::labels::device_target(MACHINE): {"w1:p1": {"owner": "v1:b"}}})
        );
        let views = read(state, WORKSPACE_VIEWS);
        assert_eq!(views["workspaces"][0]["device_id"], MACHINE);
        assert_eq!(
            views["workspaces"][0]["tab_views"],
            json!({tab(MACHINE): "d1"})
        );
        assert_eq!(
            views.pointer("/workspaces/0/view_layout/root/area/displays/0/opener/device_id"),
            Some(&json!(MACHINE))
        );
        let missions = read(state, MISSIONS);
        assert_eq!(missions["missions"][0]["device_id"], MACHINE);
        assert_eq!(
            missions["missions"][0]["workspace_id"],
            json!(project(MACHINE))
        );
        assert_eq!(
            missions["missions"][0]["conductor"]["actor"]["pane_id"],
            json!(pane(MACHINE))
        );
        assert_eq!(
            read(state, LOCAL_ISSUES)["devices"],
            json!({MACHINE: {ROOT: []}})
        );
        assert_eq!(
            read(state, COMPOSER_MARKS)["marks"][0]["device_id"],
            MACHINE
        );
        assert_eq!(
            read(state, COMPOSER_MARKS)["marks"][0]["pane_id"],
            json!(pane(MACHINE))
        );
        assert_eq!(
            read(state, HOOK_FACTS)["panes"],
            json!({pane(MACHINE): {"sessions": [], "seen_agent": true}})
        );

        // The cards keep their numbers, text and state under the new id.
        assert_eq!(cards(state, MACHINE), before_cards);
        assert!(cards(state, ALIAS).is_empty());
        assert_eq!(
            link_devices(state),
            vec![
                ("projects".to_owned(), MACHINE.to_owned()),
                ("sessions".to_owned(), MACHINE.to_owned())
            ]
        );
        let (links, _) = crate::links::store::LinkStore::open(&state.join(LINKS)).unwrap();
        let listed = |device: &str| format!("{}:{device}", crate::links::worker::LISTED_AT);
        assert_eq!(
            links.meta(&listed(MACHINE)).unwrap().as_deref(),
            Some("200")
        );
        assert_eq!(links.meta(&listed(ALIAS)).unwrap(), None);
        // The old search file's retention reaches the Project under its new id.
        let policies = links.policies(&state.join(SESSION_SEARCH)).unwrap();
        assert_eq!(policies.get(&memory_project(MACHINE)), Some(&30));
        drop(links);

        let engine = |id: &str| hide_kit::layout::device_engine_dir(state, id).unwrap();
        assert!(engine(MACHINE).join("session.json").is_file());
        assert!(!engine(ALIAS).exists());
        assert!(!state.join(JOURNAL).exists());
        // Each store's original is kept once, byte for byte.
        let backups: Vec<PathBuf> = std::fs::read_dir(state.join(BACKUP_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(
            backups[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("device-ids-")
        );
        for (file, bytes) in &originals {
            assert_eq!(
                &std::fs::read(backups[0].join(file)).unwrap(),
                bytes,
                "{file}"
            );
        }
        assert!(backups[0].join(PROJECT_MEMORY).is_file());
        assert!(backups[0].join(LINKS).is_file());
    }

    #[test]
    fn a_second_start_writes_nothing() {
        let dir = folder();
        let state = dir.path();
        name_devices(state, &node()).unwrap();
        let before = contents(state);
        let outcome = name_devices(state, &node()).unwrap();
        assert_eq!(outcome, DeviceNames::default());
        assert_eq!(contents(state), before);
    }

    /// A start that stopped after any step, the plan written and some stores
    /// renamed, is resumed by the next to the same folder an uninterrupted
    /// one leaves.
    #[test]
    fn a_renaming_stopped_after_any_step_resumes_to_the_same_result() {
        let whole = folder();
        name_devices(whole.path(), &node()).unwrap();
        let expected = contents(whole.path());
        let steps = 2 + STORES.len() + 2;
        for stopped_after in 0..=steps {
            let dir = folder();
            let state = dir.path();
            let journal = plan(state, &node(), &mut DeviceNames::default())
                .unwrap()
                .unwrap();
            write(&state.join(JOURNAL), &serde_json::to_vec(&journal).unwrap()).unwrap();
            apply_steps(state, &journal, &mut DeviceNames::default(), stopped_after).unwrap();
            let outcome = name_devices(state, &node()).unwrap();
            assert_eq!(
                outcome.renamed,
                vec![(ALIAS.to_owned(), MACHINE.to_owned())]
            );
            assert_eq!(
                contents(state),
                expected,
                "stopped after {stopped_after} steps"
            );
            assert!(!state.join(JOURNAL).exists());
            // The resumed start keeps its originals in the plan's one folder.
            assert_eq!(
                std::fs::read_dir(state.join(BACKUP_DIR)).unwrap().count(),
                1
            );
        }
    }

    /// A link record another writer holds does not open, and nothing past it
    /// is taken as renamed: refused before the plan, nothing is written;
    /// refused after Project Memory, the journal and what it renamed stay;
    /// once the record opens, the next start ends as an uninterrupted one.
    #[test]
    fn a_link_record_that_does_not_open_stops_the_renaming_until_it_does() {
        let whole = folder();
        name_devices(whole.path(), &node()).unwrap();
        let expected = contents(whole.path());
        let held = |state: &Path| {
            let writer = rusqlite::Connection::open(state.join(LINKS)).unwrap();
            writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
            writer
        };
        let refused = |state: &Path| {
            let refusal = name_devices(state, &node()).unwrap_err();
            assert_eq!(refusal.file, state.join(LINKS));
            assert!(
                refusal.reason.contains("links_store_busy"),
                "{}",
                refusal.reason
            );
        };

        let dir = folder();
        let state = dir.path();
        let before = contents(state);
        let writer = held(state);
        refused(state);
        drop(writer);
        assert!(!state.join(JOURNAL).exists());
        assert_eq!(contents(state), before);

        let journal = plan(state, &node(), &mut DeviceNames::default())
            .unwrap()
            .unwrap();
        write(&state.join(JOURNAL), &serde_json::to_vec(&journal).unwrap()).unwrap();
        apply_steps(state, &journal, &mut DeviceNames::default(), 1).unwrap();
        let writer = held(state);
        refused(state);
        drop(writer);
        assert!(state.join(JOURNAL).exists());
        assert_eq!(
            read(state, CORE_STATE)["device_registrations"][0]["id"],
            ALIAS
        );
        assert!(cards(state, ALIAS).is_empty(), "Memory was renamed first");

        name_devices(state, &node()).unwrap();
        assert_eq!(contents(state), expected);
        assert!(!state.join(JOURNAL).exists());
    }

    #[test]
    fn a_device_already_named_by_its_machine_id_is_only_marked_named() {
        let dir = folder();
        let state = dir.path();
        let mut core = read(state, CORE_STATE);
        core["device_registrations"][0]["id"] = json!(MACHINE);
        write_json(state, CORE_STATE, core.clone());
        let ledger = std::fs::read(state.join(DELIVERY_LEDGER)).unwrap();

        let outcome = name_devices(state, &node()).unwrap();
        assert_eq!(outcome.named, vec![MACHINE.to_owned()]);
        assert!(outcome.renamed.is_empty());
        core["device_registrations"][0]["naming"] = json!("named");
        assert_eq!(read(state, CORE_STATE), core);
        assert_eq!(std::fs::read(state.join(DELIVERY_LEDGER)).unwrap(), ledger);
        assert!(!state.join(JOURNAL).exists());
    }

    /// A device a check refuses keeps everything under its old id: no store
    /// is written and no plan is left.
    fn assert_held(state: &Path, reason: &str) {
        let before: BTreeMap<String, Vec<u8>> = STORES
            .iter()
            .chain([&CORE_STATE])
            .map(|file| ((*file).to_owned(), std::fs::read(state.join(file)).unwrap()))
            .collect();
        let outcome = name_devices(state, &node()).unwrap();
        assert!(outcome.renamed.is_empty(), "{outcome:?}");
        assert!(
            outcome
                .held
                .iter()
                .any(|(id, why)| id == ALIAS && why.contains(reason)),
            "{outcome:?}"
        );
        for (file, bytes) in before {
            assert_eq!(std::fs::read(state.join(&file)).unwrap(), bytes, "{file}");
        }
        assert_eq!(cards(state, ALIAS).len(), 2);
        assert!(!state.join(JOURNAL).exists());
        assert!(
            hide_kit::layout::device_engine_dir(state, ALIAS)
                .unwrap()
                .exists()
        );
    }

    #[test]
    fn a_device_whose_machine_is_the_core_is_held() {
        let dir = folder();
        let mut core = read(dir.path(), CORE_STATE);
        core["device_registrations"][0]["host_consent"] = consent(NODE);
        write_json(dir.path(), CORE_STATE, core);
        assert_held(dir.path(), "this core's own machine");
    }

    #[test]
    fn two_registrations_reaching_one_machine_are_both_held() {
        let dir = folder();
        let mut core = read(dir.path(), CORE_STATE);
        let twin = json!({"id": "twin", "label": "Twin", "ssh_alias": "twin", "host_consent": consent(MACHINE)});
        core["device_registrations"]
            .as_array_mut()
            .unwrap()
            .push(twin);
        write_json(dir.path(), CORE_STATE, core);
        assert_held(dir.path(), "same machine");
    }

    #[test]
    fn a_device_whose_project_memory_already_holds_its_machine_id_is_held() {
        let dir = folder();
        let memory = hide_memory::MemoryStore::open(&dir.path().join(PROJECT_MEMORY)).unwrap();
        memory
            .ensure_project(&memory_project(MACHINE), Path::new(ROOT), MACHINE)
            .unwrap();
        drop(memory);
        assert_held(dir.path(), "Project Memory already holds");
    }

    #[test]
    fn a_device_whose_engine_folder_exists_under_both_ids_is_held() {
        let dir = folder();
        std::fs::create_dir_all(hide_kit::layout::device_engine_dir(dir.path(), MACHINE).unwrap())
            .unwrap();
        assert_held(dir.path(), "both exist");
    }

    #[test]
    fn a_registered_project_whose_root_no_store_knows_is_held() {
        let dir = folder();
        let mut core = read(dir.path(), CORE_STATE);
        core["workspace_registrations"][0]["path"] = json!("/srv/p/sub");
        write_json(dir.path(), CORE_STATE, core);
        // Neither store knows /srv/p as this project's root any more.
        rusqlite::Connection::open(dir.path().join(LINKS))
            .unwrap()
            .execute("DELETE FROM projects", [])
            .unwrap();
        rusqlite::Connection::open(dir.path().join(PROJECT_MEMORY))
            .unwrap()
            .execute("UPDATE projects SET root='/elsewhere'", [])
            .unwrap();
        assert_held(dir.path(), "root no store knows");
    }

    #[test]
    fn a_device_whose_consent_bound_no_machine_id_waits() {
        let dir = folder();
        let mut core = read(dir.path(), CORE_STATE);
        core["device_registrations"][0]["host_consent"]["identity"]
            .as_object_mut()
            .unwrap()
            .remove("machine_id");
        write_json(dir.path(), CORE_STATE, core.clone());
        assert_eq!(
            name_devices(dir.path(), &node()).unwrap(),
            DeviceNames::default()
        );
        assert_eq!(read(dir.path(), CORE_STATE), core);
    }

    /// The operator's own folder, copied (`HIDE_TEST_STATE_COPY`, a folder
    /// whose SQLite stores were copied consistently with `.backup`): it is
    /// copied again, and only that copy is renamed. Every dialed device
    /// takes its machine id and keeps its projects, letters and Memory
    /// cards. Never run on the live state folder.
    #[test]
    #[ignore = "reads a copy of an operator's state folder named by HIDE_TEST_STATE_COPY"]
    fn an_operator_state_copy_renames_every_device_and_keeps_its_records() {
        let source = std::path::PathBuf::from(
            std::env::var_os("HIDE_TEST_STATE_COPY").expect("HIDE_TEST_STATE_COPY names the copy"),
        );
        let source = std::fs::canonicalize(&source).unwrap();
        let live = hide_platform::host::home_dir().unwrap().join(".hide/state");
        assert_ne!(
            Some(source.clone()),
            std::fs::canonicalize(&live).ok(),
            "the live state folder is never renamed"
        );
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        copy_tree(&source, state);
        let node = NodeId::parse(
            read(state, super::super::MARKER_FILE)["node"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let core = read(state, CORE_STATE);
        let devices: Vec<(String, String)> = core["device_registrations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row.get("ssh_alias").is_some_and(Value::is_string))
            .filter_map(|row| {
                Some((
                    row["id"].as_str()?.to_owned(),
                    row.pointer("/host_consent/identity/machine_id")?
                        .as_str()?
                        .to_owned(),
                ))
            })
            .collect();
        assert!(
            !devices.is_empty(),
            "the copy has no dialed device to rename"
        );
        let projects = |core: &Value| -> Vec<(Value, Value)> {
            let mut projects: Vec<(Value, Value)> = core["workspace_registrations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|project| (project["device_id"].clone(), project["path"].clone()))
                .collect();
            projects.sort_by_key(|project| project.1.to_string() + &project.0.to_string());
            projects
        };
        let letters = |ledger: &Value| -> Vec<(String, Value, Value)> {
            ledger["letters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|letter| {
                    (
                        letter["id"].as_str().unwrap_or_default().to_owned(),
                        letter["sender"]["device_id"].clone(),
                        letter["recipient"]["device_id"].clone(),
                    )
                })
                .collect()
        };
        let links_of = |ledger: &Value, key: &str| -> Vec<Value> {
            ledger[key]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| row.get("parent").cloned().unwrap_or(Value::Null))
                .collect()
        };
        let ledger_before = read(state, DELIVERY_LEDGER);
        let cards_before: BTreeMap<String, Vec<(i64, String, String)>> = devices
            .iter()
            .map(|(id, _)| (id.clone(), cards(state, id)))
            .collect();
        let core_cards = cards(state, node.as_str());

        let outcome = name_devices(state, &node).unwrap();
        assert!(outcome.held.is_empty(), "{outcome:?}");
        let mut renamed = outcome.renamed.clone();
        renamed.sort();
        let mut expected = devices.clone();
        expected.sort();
        assert_eq!(renamed, expected);

        let core_after = read(state, CORE_STATE);
        let ledger_after = read(state, DELIVERY_LEDGER);
        let to: BTreeMap<&str, &str> = devices
            .iter()
            .map(|(a, m)| (a.as_str(), m.as_str()))
            .collect();
        let mapped = |device: &Value| match device.as_str().and_then(|id| to.get(id)) {
            Some(machine) => json!(machine),
            None => device.clone(),
        };
        // Every project stays at its path, a device's under its machine id.
        let mut expected_projects: Vec<(Value, Value)> = projects(&core)
            .into_iter()
            .map(|(device, path)| (mapped(&device), path))
            .collect();
        expected_projects.sort_by_key(|project| project.1.to_string() + &project.0.to_string());
        assert_eq!(projects(&core_after), expected_projects);
        let expected_letters: Vec<(String, Value, Value)> = letters(&ledger_before)
            .into_iter()
            .map(|(id, sender, recipient)| (id, mapped(&sender), mapped(&recipient)))
            .collect();
        assert_eq!(letters(&ledger_after), expected_letters);
        // A watch's or spawn's parent is the same agent, its device and pane
        // now under the machine id.
        let actor = |parent: Value| -> Value {
            let mut parent = parent;
            if let Some(device) = parent.get("device_id").cloned() {
                parent["device_id"] = mapped(&device);
            }
            if let Some(pane) = parent["pane_id"].as_str().and_then(|pane| {
                to.iter().find_map(|(alias, machine)| {
                    pane.strip_prefix(&format!("remote:{alias}:"))
                        .map(|rest| format!("remote:{machine}:{rest}"))
                })
            }) {
                parent["pane_id"] = json!(pane);
            }
            parent
        };
        for key in ["watches", "spawns"] {
            let expected: Vec<Value> = links_of(&ledger_before, key)
                .into_iter()
                .map(actor)
                .collect();
            assert_eq!(links_of(&ledger_after, key), expected, "{key}");
        }
        for (alias, machine) in &devices {
            assert_eq!(
                &cards(state, machine),
                &cards_before[alias],
                "{alias}'s cards"
            );
            assert!(cards(state, alias).is_empty(), "{alias}");
        }
        assert_eq!(cards(state, node.as_str()), core_cards, "this core's cards");
        eprintln!(
            "renamed {:?}; files {:?}; device cards {:?}; core cards {}; letters {}; projects {} ({} on devices)",
            outcome.renamed,
            outcome.files,
            cards_before
                .iter()
                .map(|(id, cards)| (id, cards.len()))
                .collect::<Vec<_>>(),
            core_cards.len(),
            expected_letters.len(),
            projects(&core_after).len(),
            expected_projects
                .iter()
                .filter(|(device, _)| to.values().any(|machine| device == machine))
                .count(),
        );
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&path, &target);
            } else {
                std::fs::copy(&path, &target).unwrap();
            }
        }
    }
}
