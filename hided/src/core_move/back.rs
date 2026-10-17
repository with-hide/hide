//! A move of the core back to the node that dialed it (PRD
//! core-host-node-move B6, amendment 2). The node's own hided drives it
//! from its node role: it asks the core's machine to stop its core for the
//! move (`release`, which the core checks and answers before it stops),
//! pulls the staged copy over SSH, changes its owner back, places it in its
//! own state folder and has the core's machine retire its core, which is
//! the commit point; only then does its own core start. Every step before
//! the retirement is undone by restarting the core's machine's core from
//! its untouched folder (`resume`).
//!
//! The core's machine exports, as it stops, the facts the owner change
//! needs that only it has: its projects' repository roots, resolved on its
//! own disk, and the label records of the node it held in memory.

use std::path::{Path, PathBuf};

use herdr_core::node_migration::{self, IdTable, KnownProject, OwnerChange, copy};
use hide_node::ssh::transfer::FileCopy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::answer::StepAnswer;
use super::driver::Remote;
use super::journal::{BackPhase, Direction, ForwardPhase, Journal, MoveFailure, Peer, Phase};

const EXPORT_VERSION: u32 = 1;
const EXPORT_CAP: u64 = 16 * 1024 * 1024;

/// What the core's machine knows for a move back and the copy does not
/// say, written beside its staged copy as its core stops.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Export {
    pub version: u32,
    pub intent: String,
    /// The node whose core stopped.
    pub source: String,
    /// The node the core goes back to.
    pub target: String,
    pub projects: Vec<ExportedProject>,
    /// The target's label records, which the core held in memory, in the
    /// form `labels.json` stores them.
    pub labels: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExportedProject {
    pub id: String,
    pub device_id: String,
    pub path: String,
    /// The repository root the project's id digests.
    pub root: String,
    pub checkouts: Vec<(String, String)>,
}

impl Export {
    pub fn new(
        intent: &str,
        source: &herdr_core::ReleaseSource,
        target: &str,
        labels: Value,
        home: &Path,
    ) -> Self {
        let roots = super::driver::project_roots(&source.projects, &source.node, home);
        Self {
            version: EXPORT_VERSION,
            intent: intent.to_owned(),
            source: source.node.clone(),
            target: target.to_owned(),
            projects: source
                .projects
                .iter()
                .zip(roots)
                .map(|(project, root)| ExportedProject {
                    id: project.id.clone(),
                    device_id: project.device_id.clone(),
                    path: project.path.clone(),
                    root: root.to_string_lossy().into_owned(),
                    checkouts: project.checkouts.clone(),
                })
                .collect(),
            labels,
        }
    }
}

/// Where the export of `intent` sits: beside its staged copy.
pub fn export_path(state_dir: &Path, intent: &str) -> PathBuf {
    node_migration::staging_dir(state_dir, intent).with_extension("export.json")
}

/// Where the manifest of `intent`'s staged copy sits.
pub fn manifest_path(state_dir: &Path, intent: &str) -> PathBuf {
    node_migration::staging_dir(state_dir, intent).with_extension("manifest.json")
}

pub fn write_export(state_dir: &Path, export: &Export) -> Result<(), String> {
    let path = export_path(state_dir, &export.intent);
    if let Some(folder) = path.parent() {
        hide_platform::fs::private::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec(export).map_err(|error| error.to_string())?;
    hide_platform::fs::atomic::write_file_durable(
        &path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("{}: {error}", path.display()))
}

pub fn read_export(state_dir: &Path, intent: &str) -> Result<Export, String> {
    let path = export_path(state_dir, intent);
    let bytes = super::read_private(&path, EXPORT_CAP)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let export: Export = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "{} is not an export this build reads: {error}",
            path.display()
        )
    })?;
    if export.version != EXPORT_VERSION || export.intent != intent {
        return Err(format!("{} is not this move's export", path.display()));
    }
    Ok(export)
}

/// Removes the staged copy of `intent` and the records beside it. A path
/// under something that is not a folder (a file where `move-staging` goes)
/// holds nothing, so it is already removed; the next move's staging fails
/// on that file and names it.
pub fn remove_staging(state_dir: &Path, intent: &str) -> Result<(), String> {
    let staging = node_migration::staging_dir(state_dir, intent);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|error| format!("{}: {error}", staging.display()))?;
    }
    for path in [
        export_path(state_dir, intent),
        manifest_path(state_dir, intent),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) => {}
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    Ok(())
}

/// The owner change of a move back: the core's machine gives the core to
/// this node, which keeps the core's machine as the device it dialed before
/// the forward move (`registration`).
pub fn back_change(
    peer: &Peer,
    own_node: &str,
    registration: herdr_core::DeviceRegistration,
    peer_herdr_socket: String,
    own_herdr_socket: String,
) -> OwnerChange {
    OwnerChange {
        old_owner: peer.node.clone(),
        old_owner_as: peer.device.clone(),
        new_owner: own_node.to_owned(),
        new_owner_was: own_node.to_owned(),
        old_owner_registration: registration,
        old_owner_herdr_socket: peer_herdr_socket,
        new_owner_herdr_socket: own_herdr_socket,
    }
}

/// The registration id this machine's core gave `peer_node` before the
/// forward move to it: as that move's journal recorded it, or as a move
/// back to here that rolled back recorded it from there.
pub fn recorded_device(previous: Option<&Journal>, peer_node: &str) -> Option<String> {
    previous
        .filter(|journal| {
            journal.peer.node == peer_node
                && (journal.phase == Phase::Forward(ForwardPhase::Done)
                    || (journal.phase.direction() == Direction::Back
                        && journal.phase.is_rolled_back()))
        })
        .map(|journal| journal.peer.device.clone())
}

/// The registration this machine's core kept for the core's machine before
/// the forward move, as it set it aside (`moved-out`): the first that
/// `wanted` picks; none when the folder holds no such registration.
pub fn kept_registration(
    state_dir: &Path,
    wanted: impl Fn(&herdr_core::DeviceRegistration) -> bool,
) -> Option<herdr_core::DeviceRegistration> {
    let root = hide_kit::layout::moved_out(state_dir);
    let entries = std::fs::read_dir(&root).ok()?;
    for entry in entries.flatten() {
        let Ok(bytes) = std::fs::read(entry.path().join(node_migration::CORE_STATE)) else {
            continue;
        };
        let Ok(state) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let found = state["device_registrations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| serde_json::from_value(row.clone()).ok())
            .find(|registration| wanted(registration));
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The ids the forward move to `peer_node` gave, the other way round: every
/// project and checkout either machine had when the core left, which the
/// core's machine may not have listed again by the time it comes back.
pub fn forward_ids_reversed(previous: Option<&Journal>, peer_node: &str) -> IdTable {
    let mut reversed = IdTable::default();
    let Some(forward) = previous.filter(|journal| {
        journal.peer.node == peer_node && journal.phase == Phase::Forward(ForwardPhase::Done)
    }) else {
        return reversed;
    };
    for (from, to) in &forward.ids.registrations {
        reversed.registrations.insert(to.clone(), from.clone());
    }
    for (from, to) in &forward.ids.checkouts {
        reversed.checkouts.insert(to.clone(), from.clone());
    }
    reversed
}

/// What the core's machine answered the release.
#[derive(Debug, Eq, PartialEq)]
pub enum Released {
    /// Its core stopped for the move and its copy is staged.
    Staged,
    /// It retired its core for this move already: the move committed.
    Retired,
}

pub fn release(
    remote: &Remote,
    journal: &Journal,
    own_node: &str,
) -> Result<Released, MoveFailure> {
    match remote.step(
        "release",
        &[("intent", &journal.intent), ("target", own_node)],
    )? {
        StepAnswer::Staged { .. } => Ok(Released::Staged),
        StepAnswer::Retired { .. } => Ok(Released::Retired),
        other => Err(super::driver::unexpected("release", &other)),
    }
}

/// Pulls the staged copy and the records beside it into this machine's
/// staging folder and checks every file against the manifest; a file that
/// differs is pulled once more.
pub fn pull(
    remote: &Remote,
    state_dir: &Path,
    journal: &Journal,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<copy::Manifest, MoveFailure> {
    let local = |reason: String| MoveFailure::Local { reason };
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    remove_staging(state_dir, &journal.intent).map_err(local)?;
    let remote_staging = format!(
        "{}/{}/{}",
        journal.peer.state_dir.trim_end_matches('/'),
        node_migration::MOVE_STAGING,
        journal.intent
    );
    let records = [
        FileCopy {
            local: manifest_path(state_dir, &journal.intent),
            remote: format!("{remote_staging}.manifest.json"),
        },
        FileCopy {
            local: export_path(state_dir, &journal.intent),
            remote: format!("{remote_staging}.export.json"),
        },
    ];
    remote.download(
        &state_dir.join(node_migration::MOVE_STAGING),
        &records,
        &|_| {},
    )?;
    let bytes = std::fs::read(&records[0].local).map_err(|error| local(error.to_string()))?;
    let manifest: copy::Manifest =
        serde_json::from_slice(&bytes).map_err(|error| MoveFailure::Refused {
            step: "release".to_owned(),
            reason: format!("an unreadable manifest: {error}"),
        })?;
    let total = manifest.total_bytes();
    let received = std::sync::atomic::AtomicU64::new(0);
    let mut wanted: Vec<String> = manifest.files.keys().cloned().collect();
    for _ in 0..2 {
        let files = super::driver::copies(&wanted, &staging, &remote_staging)?;
        remote.download(&staging, &files, &|bytes| {
            let now = received.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed) + bytes;
            progress(now.min(total), total);
        })?;
        let pulled = copy::digest(&staging).map_err(|refusal| local(refusal.to_string()))?;
        wanted = manifest.differs(&pulled);
        if wanted.is_empty() {
            herdr_core::diagnostic!(json!({
                "component": "core_move",
                "kind": "copy.pulled",
                "intent": journal.intent,
                "files": manifest.files.len(),
            }));
            return Ok(manifest);
        }
    }
    Err(MoveFailure::Digest { files: wanted })
}

/// Changes the pulled copy's owner to this machine with the ids the core's
/// machine exported, and loads it with this build's readers.
pub fn rekey(state_dir: &Path, journal: &Journal) -> Result<IdTable, MoveFailure> {
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    let export =
        read_export(state_dir, &journal.intent).map_err(|reason| MoveFailure::Local { reason })?;
    if export.source != journal.change.old_owner || export.target != journal.change.new_owner {
        return Err(MoveFailure::Local {
            reason: format!(
                "the export names {} to {}, not this move's machines",
                export.source, export.target
            ),
        });
    }
    let roots: Vec<PathBuf> = export
        .projects
        .iter()
        .map(|project| PathBuf::from(&project.root))
        .collect();
    let projects = export
        .projects
        .iter()
        .zip(&roots)
        .map(|(project, root)| KnownProject {
            id: &project.id,
            device_id: &project.device_id,
            path: &project.path,
            root,
            checkouts: project
                .checkouts
                .iter()
                .map(|(id, path)| (id.as_str(), path.as_str()))
                .collect(),
        });
    // What the core's machine lists now, over what the forward move knew.
    let mut ids = journal.ids.clone();
    let listed = node_migration::id_table(&journal.change, projects);
    ids.registrations.extend(listed.registrations);
    ids.checkouts.extend(listed.checkouts);
    let failed = |refusal: node_migration::Refusal| MoveFailure::Staging {
        file: refusal.file.display().to_string(),
        reason: refusal.reason,
    };
    let outcome = node_migration::reown(&staging, &journal.change, &ids).map_err(failed)?;
    herdr_core::diagnostic!(json!({
        "component": "core_move",
        "kind": "staging.reowned",
        "intent": journal.intent,
        "files": outcome.files,
        "pruned": outcome.pruned,
    }));
    copy::check_loadable(&staging).map_err(|refusal| MoveFailure::Load {
        file: refusal.file.display().to_string(),
        reason: refusal.reason,
    })?;
    Ok(ids)
}

/// Places the pulled copy in this machine's state folder, its Hide AI
/// settings at `ai_settings`.
pub fn place_here(
    state_dir: &Path,
    ai_settings: &Path,
    journal: &Journal,
) -> Result<(), copy::NotPlaced> {
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    copy::place(&staging, state_dir, ai_settings).map(|_| ())
}

/// Takes a placed copy back out of this machine's state folder and removes
/// the staging folder, leaving the folder as the node role had it.
pub fn unplace_here(state_dir: &Path, ai_settings: &Path, journal: &Journal) -> Result<(), String> {
    let staging = node_migration::staging_dir(state_dir, &journal.intent);
    // Before `PlacedHere` nothing of the copy was placed, and brain state
    // here is this machine's own.
    if matches!(
        journal.phase,
        Phase::Back(BackPhase::PlacedHere | BackPhase::Retiring)
    ) && !copy::brain_present(state_dir).is_empty()
    {
        copy::unplace(state_dir, &staging, ai_settings).map_err(|refusal| refusal.to_string())?;
    }
    remove_staging(state_dir, &journal.intent)
}

/// Has the core's machine retire its core held for the move back `intent`;
/// the answer is what that machine left of its folder after the commit.
pub fn retire(remote: &Remote, intent: &str) -> Result<Option<String>, MoveFailure> {
    match remote.step("retire", &[("intent", intent)])? {
        StepAnswer::Retired { leftover } => Ok(leftover),
        other => Err(super::driver::unexpected("retire", &other)),
    }
}

/// What the core's machine answered a resume.
#[derive(Debug, Eq, PartialEq)]
pub enum Resumed {
    /// Its core runs on its folder as before; `leftover` names what of the
    /// staged copy it could not remove.
    Running { leftover: Option<String> },
    /// It retired its core for this move: the move committed.
    Retired,
}

pub fn resume(remote: &Remote, journal: &Journal) -> Result<Resumed, MoveFailure> {
    match remote.step("resume", &[("intent", &journal.intent)])? {
        StepAnswer::Running { leftover, .. } => Ok(Resumed::Running { leftover }),
        StepAnswer::Retired { .. } => Ok(Resumed::Retired),
        other => Err(super::driver::unexpected("resume", &other)),
    }
}
