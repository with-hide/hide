//! A start's migrations over the state folders released builds wrote.
//!
//! Each folder under `tests/fixtures/release-state/` was written by a
//! published release's own programs (`scripts/release_state/generate.py`):
//! a core and an SSH device it dialed, with agents, Views, letters, a moved
//! pane, Memory cards and the rest that store a device's id. The suite runs
//! what `hided` runs before the core reads the folder, `convert` and then
//! `name_devices`, on a copy of each, and holds the start to what an
//! operator upgrading from that release sees: nothing refused, every dialed
//! device named and connected rather than held, every store read by this
//! build's own loader, nothing lost, and a second start that writes
//! nothing.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::copy::digest;
use super::{
    CORE_STATE, DELIVERY_LEDGER, MARKER_FILE, MISSIONS, PROJECT_MEMORY, WORKSPACE_VIEWS, convert,
    name_devices,
};
use crate::node::NodeId;

/// A diagnostic `kind` a loader emits when it did not use what was stored.
const NOT_USED: &[&str] = &[
    "unreadable",
    "discard",
    "version_unknown",
    "corrupt",
    "set_aside",
    "rebuilt",
    "dropped",
];

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/release-state");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{}: {error}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    folders
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn json(state: &Path, name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(state.join(name)).unwrap()).unwrap()
}

fn length(value: &Value, pointer: &str) -> usize {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// The Memory cards of every Project, read from the database as it is.
fn cards(state: &Path) -> i64 {
    let path = state.join(PROJECT_MEMORY);
    if !path.exists() {
        return 0;
    }
    let connection =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    connection
        .query_row("SELECT count(*) FROM memory_items", [], |row| row.get(0))
        .unwrap()
}

/// What an operator would count before and after the upgrade.
#[derive(Debug, Eq, PartialEq)]
struct Counts {
    projects: usize,
    letters: usize,
    cards: i64,
}

fn counts(state: &Path) -> Counts {
    Counts {
        projects: length(&json(state, CORE_STATE), "/workspace_registrations"),
        letters: length(&json(state, DELIVERY_LEDGER), "/letters"),
        cards: cards(state),
    }
}

/// Every JSON store under `state`, by its path, outside the backups the
/// migrations keep of the originals.
fn json_stores(state: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![state.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name() != Some(std::ffi::OsStr::new(super::BACKUP_DIR)) {
                    folders.push(path);
                }
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                found.push(path);
            }
        }
    }
    found
}

/// Reads every store with the loader the core starts with, and answers
/// what each did not use, by the loader's typed outcome or its diagnostic.
fn load_every_store(state: &Path, node: &NodeId) -> Vec<String> {
    let mut refused = Vec::new();
    let ((), records) = crate::diagnostics::capture(|| {
        let (ui, _, disposition) = crate::persistence::load(&state.join(CORE_STATE));
        if disposition != crate::persistence::LoadDisposition::Loaded {
            refused.push(format!("{CORE_STATE}: {disposition:?}"));
        }
        if ui.workspace_registrations.is_empty() {
            refused.push(format!("{CORE_STATE}: no project was read"));
        }
        let (_, outcome) = crate::workspace_views::load(&state.join(WORKSPACE_VIEWS), 0);
        if !matches!(outcome, crate::workspace_views::LoadOutcome::Loaded { .. }) {
            refused.push(format!("{WORKSPACE_VIEWS}: {outcome:?}"));
        }
        if let Err(reason) = crate::delivery::ledger::load(&state.join(DELIVERY_LEDGER)) {
            refused.push(format!("{DELIVERY_LEDGER}: {reason}"));
        }
        if let crate::missions::Restored::Unreadable(reason) =
            crate::missions::MissionStore::new(state.join(MISSIONS)).restore()
        {
            refused.push(format!("{MISSIONS}: {reason}"));
        }
        drop(crate::labels::store::LabelStore::open(
            Some(state),
            None,
            node.as_str(),
        ));
        drop(crate::agent_hooks::facts::HookFacts::restore(
            state.join(super::HOOK_FACTS),
            Vec::new,
        ));
        drop(
            crate::composer_store::ComposerStore::new(state.join(super::COMPOSER_MARKS)).restore(0),
        );
        let issues = state.join(super::LOCAL_ISSUES);
        if issues.exists()
            && let Err(reason) = crate::local_issues::load(&issues)
        {
            refused.push(format!("{}: {reason}", super::LOCAL_ISSUES));
        }
        match crate::links::store::LinkStore::open(&state.join(super::LINKS)) {
            Ok((_, crate::links::store::Opened::Existing)) => {}
            Ok((_, opened)) => refused.push(format!("{}: {opened:?}", super::LINKS)),
            Err(reason) => refused.push(format!("{}: {reason}", super::LINKS)),
        }
        let memory = state.join(PROJECT_MEMORY);
        if memory.exists()
            && let Err(error) = hide_memory::MemoryStore::open(&memory)
        {
            refused.push(format!("{PROJECT_MEMORY}: {error}"));
        }
        let usage = hide_kit::layout::usage_store(state);
        if usage.exists()
            && let Err(reason) = crate::usage_ledger::store::Store::open(&usage)
        {
            refused.push(format!("{}: {reason}", hide_kit::layout::USAGE_STORE));
        }
        let suggestions = state.join(hide_kit::layout::SUGGESTIONS_STORE);
        if suggestions.exists() {
            match crate::suggestions::store::SuggestionStore::open(&suggestions) {
                Ok((_, crate::suggestions::store::Opened::Existing)) => {}
                Ok((_, opened)) => refused.push(format!("suggestions: {opened:?}")),
                Err(reason) => refused.push(format!("suggestions: {reason}")),
            }
        }
    });
    refused.extend(
        records
            .iter()
            .filter(|record| {
                record["kind"]
                    .as_str()
                    .is_some_and(|kind| NOT_USED.iter().any(|word| kind.contains(word)))
            })
            .map(Value::to_string),
    );
    refused
}

/// What went wrong for one released folder; empty when it upgrades.
fn upgrade(fixture: &Path) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path();
    copy_tree(fixture, state);
    let marker = json(state, MARKER_FILE);
    let node = NodeId::parse(marker["node"].as_str().unwrap()).unwrap();
    let core = json(state, CORE_STATE);
    let dialed: Vec<(String, String)> = core["device_registrations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["ssh_alias"].is_string())
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_owned(),
                row.pointer("/host_consent/identity/machine_id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    let before = counts(state);
    let mut failures = Vec::new();

    if let Err(refusal) = convert(state, &node) {
        return vec![format!("the conversion refused: {refusal}")];
    }
    let named = match name_devices(state, &node) {
        Ok(named) => named,
        Err(refusal) => return vec![format!("the device naming refused: {refusal}")],
    };
    if !named.held.is_empty() {
        failures.push(format!("devices held unconnected: {:?}", named.held));
    }
    let core = json(state, CORE_STATE);
    for (old, machine) in &dialed {
        let row = core["device_registrations"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["ssh_alias"].is_string() && row["id"] == machine.as_str());
        if row.is_none_or(|row| row["naming"] != "named") {
            failures.push(format!("{old} is not registered as {machine}, named"));
        }
        // An old id that still scopes a project, pane, tab or checkout
        // would point at a device that is no longer registered.
        let scoped = format!("remote:{old}:");
        if old != machine {
            for path in json_stores(state) {
                if std::fs::read_to_string(&path).unwrap().contains(&scoped) {
                    let store = path.strip_prefix(state).unwrap().display().to_string();
                    failures.push(format!("{store} still names {scoped}"));
                }
            }
        }
    }

    let migrated = digest(state).unwrap();
    match (convert(state, &node), name_devices(state, &node)) {
        (Ok(converted), Ok(named)) => {
            if !converted.files.is_empty() || !named.renamed.is_empty() || !named.held.is_empty() {
                failures.push(format!(
                    "a second start changed something: {converted:?} {named:?}"
                ));
            }
        }
        (converted, named) => {
            failures.push(format!("a second start refused: {converted:?} {named:?}"))
        }
    }
    let changed = migrated.differs(&digest(state).unwrap());
    if !changed.is_empty() {
        failures.push(format!("a second start wrote {changed:?}"));
    }

    let after = counts(state);
    if after != before {
        failures.push(format!("counts changed: {before:?} -> {after:?}"));
    }
    failures.extend(load_every_store(state, &node));
    failures
}

/// The upgrade an operator makes from each released build: 1.2.1 users
/// lost their SSH device to a View's opener the renaming never reached.
#[test]
fn every_released_state_folder_upgrades_with_its_devices_named_and_nothing_lost() {
    let fixtures = fixtures();
    assert!(
        fixtures.iter().any(|path| path.ends_with("v1.2.1")),
        "the 1.2.1 folder is missing: {fixtures:?}"
    );
    let failures: Vec<String> = fixtures
        .iter()
        .flat_map(|fixture| {
            let tag = fixture.file_name().unwrap().to_string_lossy().into_owned();
            upgrade(fixture)
                .into_iter()
                .map(move |failure| format!("{tag}: {failure}"))
        })
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
