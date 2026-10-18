//! The one-time conversion of a state folder written before node ids existed
//! (PRD core-host-node D-23, B2).
//!
//! Every store used to name the machine the core ran on `"local"`. A key
//! holding that word names whichever machine reads it, so a folder copied to
//! another machine would point at the wrong one without an error. Each key
//! that names this machine is rewritten once to its node id; the keys that
//! name it only by being unqualified (a raw Herdr pane or tab id, a path) are
//! covered instead by `node.json`, the folder's statement of which node owns
//! every unqualified key in it. `KEYS` lists both kinds for every store.
//!
//! The conversion runs in the daemon before the core is created, under the
//! daemon lock. It is safe to run on every start: a store with nothing left
//! to convert is not written. Each file is replaced in one step (an atomic
//! rename, or one SQLite transaction), the files it changes are copied to
//! `node-migration-backup/` first, and `node.json` is written last. A failure
//! stops the start and names the file; the files not yet converted are
//! untouched, and a retry converges.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::node::{LEGACY_LOCAL_DEVICE_ID as LEGACY, NodeId};

pub mod copy;
mod reown;
pub use reown::{IdTable, KnownProject, OwnerChange, ReownOutcome, id_table, reown, staging_dir};

pub const MARKER_FILE: &str = "node.json";
/// The Hide AI settings' name in a move's copy. They move with the core
/// (PRD core-host-node-move Q7) but live outside the state folder
/// (`hide_ai::settings::settings_path`), so the copy's functions are told
/// where each machine keeps them.
pub const AI_SETTINGS: &str = "ai.json";
pub const BACKUP_DIR: &str = "node-migration-backup";
pub(crate) const MARKER_VERSION: u32 = 1;

pub const CORE_STATE: &str = "core-state.json";
const WORKSPACE_VIEWS: &str = "workspace-views.json";
const LABELS: &str = "labels.json";
const DELIVERY_LEDGER: &str = "delivery-ledger.json";
const PROJECT_MEMORY: &str = hide_memory::DATABASE_FILE;
pub(crate) const SESSION_SEARCH: &str = "session-search.sqlite3";
const LINKS: &str = "links.sqlite3";
const LOCAL_ISSUES: &str = "local-issues.json";
const FACTORY: &str = "factory.sqlite3";
/// Where a move keeps its staging copies (`reown::staging_dir`); the
/// move's other files, which `hide_kit::layout` names, are `UNBOUND`.
pub const MOVE_STAGING: &str = "move-staging";

/// The brain state a core move copies to the core's new machine (PRD
/// core-host-node-move Q8): each store `KEYS` names, except what the new
/// machine rebuilds ([`REBUILT_AFTER_MOVE`]), and the stores that name no
/// machine but belong to the core (the Factory, phone pairing). The old
/// session search file travels as its record retention alone
/// (`copy::stage`). `node.json` is last: a folder holds a marker only once
/// everything it names is there.
pub const MOVES_WITH_CORE: &[&str] = &[
    CORE_STATE,
    WORKSPACE_VIEWS,
    LABELS,
    DELIVERY_LEDGER,
    LINKS,
    hide_kit::layout::USAGE_STORE,
    LOCAL_ISSUES,
    PROJECT_MEMORY,
    SESSION_SEARCH,
    FACTORY,
    "factory.v1.sqlite3",
    "factory-files",
    "mobile.json",
    "phones.json",
    MARKER_FILE,
];

/// The store a core move does not copy: the GitHub snapshot, which the new
/// core rebuilds from what its nodes report.
pub const REBUILT_AFTER_MOVE: &[&str] = &["github-snapshot.json"];

/// How one machine-bound key is kept pointing at the right machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mechanism {
    /// The value or key `"local"` is rewritten to the node id.
    Rewritten,
    /// The key names this machine by being unqualified; `node.json` says
    /// which node that is.
    Owned,
}

/// How a machine-bound key moves when the core moves to another machine and
/// the folder changes owner (`reown`). A move never guesses: a key the
/// table leaves naming the new owner by its old id stops the move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Moves {
    /// A machine id: the new owner's old id becomes its node id, the old
    /// owner's node id becomes the id the result gives it.
    Device,
    /// A Herdr pane id: unqualified for the owner, `remote:<id>:pane:` for
    /// any other machine.
    Pane,
    /// A Herdr tab id, the same way.
    Tab,
    /// A registered project's id (`workspace:` for the owner,
    /// `remote:<id>:project:` otherwise), through the source's id table.
    Registration,
    /// A checkout's id, through the same table.
    Checkout,
    /// A session fold: a registration id or `cleanup/<machine>`.
    Fold,
    /// The owner's own setting, swapped with the `device_*` map beside it
    /// (`reown::OWN_PATHS`).
    OwnPath(&'static str),
    /// The label records, which travel one hop (`reown`, labels).
    Labels,
    /// Moved by code that reads the whole file, not by the pattern: the
    /// ledger's host scopes and spawn machines, the View layouts' tabs.
    Whole,
    /// Names its machine through a sibling machine id or a node id no owner
    /// change touches (a path beside its `device_id`, a native machine id),
    /// or by a project path every core reads the same
    /// whichever machine holds the project (an issue source, a base branch,
    /// an opened Inactive group, the Explorer's selection): it stays as it
    /// is.
    Stays,
}

/// Every key in the state folder that names a machine, by file and JSON
/// pointer pattern (`*` is every array element or object key), how the
/// layer-1 conversion keeps it, and how a core move changes it. The contract
/// tests hold a fully populated folder to this list in every shape (the
/// owner's, a linked node's, a dialed device's), so a store that gains a
/// machine-bound key must be added here.
pub const KEYS: &[(&str, &str, Mechanism, Moves)] = &[
    (
        hide_kit::layout::USAGE_STORE,
        "native node ids in devices, events, daily, cursors and backfills; qualified paths",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        CORE_STATE,
        "/workspace_registrations/*/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/workspace_registrations/*/id",
        Mechanism::Owned,
        Moves::Registration,
    ),
    (
        CORE_STATE,
        "/workspace_registrations/*/primary_checkout_id",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/workspace_registrations/*/path",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        CORE_STATE,
        "/device_registrations/*/id",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    (
        CORE_STATE,
        "/focused_device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/expanded_inactive_project_device_ids/*",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/recent_checkouts/*/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/recent_checkouts/*/checkout_id",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/device_expanded_paths/{key}",
        Mechanism::Rewritten,
        Moves::OwnPath("expanded_paths"),
    ),
    (
        CORE_STATE,
        "/selected_pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/expanded_agent_pane_ids/*",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/sessions_expanded_agent_pane_ids/*",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/detail/items/*/workspace_id",
        Mechanism::Owned,
        Moves::Registration,
    ),
    (
        CORE_STATE,
        "/detail/items/*/session/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/recent_pane_ids/*",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_text_scales/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_read_records/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_read_records/*/descendant_signals/*/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_completions/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_launches/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/request_verbs/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/pane_terminal_sizes/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/resolved_sessions/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/session_resolution_inputs/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/agent_sleep/stamps/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/agent_sleep/records/{key}",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/agent_sleep/records/*/parent_pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/agent_sleep/records/*/cwd",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        CORE_STATE,
        "/agent_sleep/dormant/*/node_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        CORE_STATE,
        "/agent_sleep/dormant/*/old_pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/agent_sleep/dormant/*/cwd",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        CORE_STATE,
        "/factory_secretary_pane",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        CORE_STATE,
        "/expanded_paths/*",
        Mechanism::Owned,
        Moves::OwnPath("device_expanded_paths"),
    ),
    (CORE_STATE, "/selected_path", Mechanism::Owned, Moves::Stays),
    (
        CORE_STATE,
        "/focused_checkout_id",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/collapsed_workspace_ids/*",
        Mechanism::Owned,
        Moves::Registration,
    ),
    (
        CORE_STATE,
        "/collapsed_checkout_ids/*",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/expanded_checkout_ids/*",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/session_collapsed_checkout_ids/*",
        Mechanism::Owned,
        Moves::Checkout,
    ),
    (
        CORE_STATE,
        "/session_open_folds/*",
        Mechanism::Rewritten,
        Moves::Fold,
    ),
    (
        CORE_STATE,
        "/project_base_branches/{key}",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        CORE_STATE,
        "/project_issue_sources/{key}",
        Mechanism::Owned,
        Moves::Stays,
    ),
    // A Project choice is keyed by the digest of its machine's id and root
    // and carries that id, so a move re-keys the records of the two
    // machines (`reown`, project settings).
    (
        CORE_STATE,
        "/project_settings/{key}",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        CORE_STATE,
        "/project_settings/*/device_id",
        Mechanism::Owned,
        Moves::Whole,
    ),
    // The names the operator gave tabs of the owner's own Herdr, by tab id.
    (
        CORE_STATE,
        "/operator_tab_names/{key}",
        Mechanism::Owned,
        Moves::Tab,
    ),
    (
        CORE_STATE,
        "/expanded_inactive_checkout_project_paths/*",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/path",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/agent_layout",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/tab_views/{key}",
        Mechanism::Owned,
        Moves::Tab,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/view_layout",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    // A View's opener names its agent's machine at any depth of the layout
    // tree (`reown`, openers);
    // the comments name their recipient's and their screenshot's machine
    // (PRD view-opener-feedback D-23).
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/feedback/comments/*/recipient/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/feedback/comments/*/shot/node",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        WORKSPACE_VIEWS,
        "/workspaces/*/feedback/waiting/*/to/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        LABELS,
        "/targets/{key}",
        Mechanism::Rewritten,
        Moves::Labels,
    ),
    (LABELS, "/targets/*/{key}", Mechanism::Owned, Moves::Labels),
    (
        DELIVERY_LEDGER,
        "/letters/*/sender/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/letters/*/recipient/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/letters/*/watch_warning/target/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/watches/*/parent/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/watches/*/target/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/machine",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/actor/device_id",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    // A machine's clock follows its id; the clock of the machine that becomes
    // the owner is ended by the new core's store, which keeps each letter's
    // wait (`Ledger::observe_reach`).
    (
        DELIVERY_LEDGER,
        "/node_clocks/{key}",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/native_machine",
        Mechanism::Rewritten,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/letters/*/sender/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/letters/*/recipient/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/letters/*/watch_warning/target/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/watches/*/parent/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/watches/*/target/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/actor/pane_id",
        Mechanism::Owned,
        Moves::Pane,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/pane",
        Mechanism::Owned,
        Moves::Stays,
    ),
    // A registration waiting for its agent's session is judged by the writer
    // of the machine it names, so that machine follows its id; its pane is
    // that machine's own (`coordination::identity_wait`).
    (
        DELIVERY_LEDGER,
        "/identity_waits/*/machine",
        Mechanism::Rewritten,
        Moves::Device,
    ),
    (
        DELIVERY_LEDGER,
        "/identity_waits/*/pane",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/host_scope",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/project",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/session",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/agents/*/instance",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/spawns/*/machine",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    // A promotion task names its card by number; a fold of Project Memory
    // that renumbered the card is read through (`reown`, promotion targets).
    (
        DELIVERY_LEDGER,
        "/spawns/*/task/target",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        DELIVERY_LEDGER,
        "/spawns/*/repo",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/spawns/*/path",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/spawns/*/requested_path",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        DELIVERY_LEDGER,
        "/spawns/*/pane",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        PROJECT_MEMORY,
        "projects.device_id, every project_id column",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    (
        SESSION_SEARCH,
        "policy.project",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    (
        LINKS,
        "projects.device and projects.key, prs.project, worktrees.project, every device column",
        Mechanism::Rewritten,
        Moves::Whole,
    ),
    (
        LINKS,
        "meta listed_at and listed_at:<device>, pr_issues.issue `local:<root>#n`, paths and cwds",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        FACTORY,
        "factories.data node, tasks.data worker.node (`None` is the owner's)",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        FACTORY,
        "tasks.data worker.pane and producer_pane",
        Mechanism::Owned,
        Moves::Whole,
    ),
    (
        "github-snapshot.json",
        "/projects/*/root_path",
        Mechanism::Owned,
        Moves::Stays,
    ),
    (
        LOCAL_ISSUES,
        "/projects/{key}",
        Mechanism::Owned,
        Moves::OwnPath("devices"),
    ),
    (
        LOCAL_ISSUES,
        "/devices/{key}",
        Mechanism::Rewritten,
        Moves::OwnPath("projects"),
    ),
];

/// The rest of a state folder, which names no machine: the daemon's own
/// files, phone pairing, pane credentials, attachments, label generators,
/// Workspace bridges, logs, and this conversion's own files. A file in the
/// folder that is neither here nor in `KEYS` fails the contract test.
pub const UNBOUND: &[&str] = &[
    "hided.json",
    "hided.lock",
    "connect.lock",
    "host-id",
    "mobile.json",
    "phones.json",
    "pane-capabilities",
    "attachments",
    "TerminalClipboard",
    // Device generator locks before they moved to each device's node.
    "label-generators",
    "workspace-bridges",
    "Logs",
    MARKER_FILE,
    BACKUP_DIR,
    // A core move's own files (`hide_kit::layout`).
    MOVE_STAGING,
    "move-incoming",
    "moved-out",
    "core-move.json",
    "core-move-abandoned.json",
    "core-handover.json",
    "core-handover.lock",
    "core-placement.json",
];

/// `node.json`: the node that owns every unqualified key in this folder,
/// whichever machine runs the core that reads it.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Marker {
    pub(crate) version: u32,
    pub(crate) node: String,
}

/// Why the daemon cannot start on this state folder.
#[derive(Debug, Eq, PartialEq)]
pub struct Refusal {
    pub file: PathBuf,
    pub reason: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the stored state in {} could not be converted for this machine: {}",
            self.file.display(),
            self.reason
        )
    }
}

/// What a conversion changed, for the daemon's diagnostic record.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    pub files: Vec<String>,
    pub projects: usize,
    /// Project ids recorded as moved, for the old search file's retention.
    pub retention_rows: usize,
}

/// Records each `(old, new)` project id in the link record, so the
/// retention the old session search file names under `old` follows the
/// project (`links::store::policies`). The session search file itself is
/// only ever read (D-20). The link record is kept as it was first.
fn alias_retention(
    path: &Path,
    pairs: &[(String, String)],
    backup: &Backup,
) -> Result<usize, String> {
    let (mut store, _) = crate::links::store::LinkStore::open(path)?;
    backup.copy_with(LINKS, |destination| store.copy_to(destination))?;
    store.alias_retention(pairs)
}

/// Converts the state folder at `state_dir` for `node`, with its Project
/// Memory database, the one store the core reads and writes and the agent
/// hooks ask the core for.
pub fn convert(state_dir: &Path, node: &NodeId) -> Result<Outcome, Refusal> {
    // With the name its original is kept under.
    let memory_paths = [(state_dir.join(PROJECT_MEMORY), PROJECT_MEMORY)];
    let marker_path = state_dir.join(MARKER_FILE);
    let refuse = |file: &Path, reason: String| Refusal {
        file: file.to_path_buf(),
        reason,
    };
    let marker = read_marker(&marker_path).map_err(|reason| refuse(&marker_path, reason))?;
    match &marker {
        Some(owner) if owner != node.as_str() => {
            return Err(refuse(
                &marker_path,
                format!(
                    "this folder's unqualified keys belong to node {owner}, not to this \
                     machine ({node}); a core that reads another node's state is not \
                     supported yet, so start this machine with its own state folder or \
                     run the folder on node {owner}"
                ),
            ));
        }
        _ => {}
    }

    // A store the conversion would push past its loader's limit stops the
    // start before anything is written, so every file stays as it was (B2).
    // Only stores whose converter reads no project id have a limit, so the
    // check needs none of the pairs found below.
    for (file, version_key, versions, convert) in JSON_STORES {
        let limit = stored_limit(file);
        let path = state_dir.join(file);
        if limit == usize::MAX || over_limit(&path, limit) {
            continue;
        }
        let Some(mut value) = read_json(&path).map_err(|reason| refuse(&path, reason))? else {
            continue;
        };
        if !current(&value, version_key, versions) {
            continue;
        }
        if convert(&mut value, node.as_str(), &BTreeMap::new()) {
            let bytes =
                serde_json::to_vec(&value).map_err(|error| refuse(&path, error.to_string()))?;
            if bytes.len() > limit {
                return Err(refuse(
                    &path,
                    format!(
                        "converted, it would be {} bytes, over the {limit} bytes its loader \
                         reads; start the previous build once so its retention trims the \
                         file, or move the file aside to start without it",
                        bytes.len()
                    ),
                ));
            }
        }
    }

    let mut outcome = Outcome::default();
    let backup = Backup::new(state_dir, marker.is_none());
    let mut project_pairs: Vec<(String, String)> = Vec::new();

    for (path, kept_as) in &memory_paths {
        if !path.is_file() {
            continue;
        }
        let moved =
            convert_memory(path, kept_as, node, &backup).map_err(|reason| refuse(path, reason))?;
        if moved > 0 {
            outcome.files.push(path.display().to_string());
            outcome.projects += moved;
        }
        // The pairs come from the node's Projects as they are now, not from
        // the ones this start moved, so a start retried after a later store
        // failed still re-keys the rows of a Project no folder registers.
        for root in memory_roots(path, node).map_err(|reason| refuse(path, reason))? {
            let pair = (
                hide_project::project_id(LEGACY, Path::new(&root)),
                hide_project::project_id(node.as_str(), Path::new(&root)),
            );
            if !project_pairs.contains(&pair) {
                project_pairs.push(pair);
            }
        }
    }

    // Project ids are a digest of device and root. The roots a Project was
    // known by are the Memory store's and the registered folders'.
    let core_state_path = state_dir.join(CORE_STATE);
    if let Some(value) =
        read_json(&core_state_path).map_err(|reason| refuse(&core_state_path, reason))?
    {
        for path in registered_paths(&value, node) {
            if let Ok(identity) = hide_project::resolve(Path::new(&path), LEGACY) {
                let pair = (
                    identity.id,
                    hide_project::project_id(node.as_str(), &identity.root),
                );
                if !project_pairs.contains(&pair) {
                    project_pairs.push(pair);
                }
            }
        }
    }
    let project_ids: BTreeMap<String, String> = project_pairs.iter().cloned().collect();

    for (file, version_key, versions, convert) in JSON_STORES {
        let path = state_dir.join(file);
        // A store its own loader already refuses is left to that loader,
        // as a store of an unknown version is.
        if over_limit(&path, stored_limit(file)) {
            continue;
        }
        let Some(mut value) = read_json(&path).map_err(|reason| refuse(&path, reason))? else {
            continue;
        };
        if !current(&value, version_key, versions) {
            continue;
        }
        if convert(&mut value, node.as_str(), &project_ids) {
            let bytes =
                serde_json::to_vec(&value).map_err(|error| refuse(&path, error.to_string()))?;
            backup
                .copy(&path, file)
                .map_err(|reason| refuse(&path, reason))?;
            write(&path, &bytes).map_err(|reason| refuse(&path, reason))?;
            outcome.files.push(path.display().to_string());
        }
    }

    let links = hide_kit::layout::links_store(state_dir);
    if state_dir.join(SESSION_SEARCH).is_file() && !project_pairs.is_empty() {
        outcome.retention_rows = alias_retention(&links, &project_pairs, &backup)
            .map_err(|reason| refuse(&links, reason))?;
        if outcome.retention_rows > 0 {
            outcome.files.push(links.display().to_string());
        }
    }

    if links.is_file()
        && convert_links(&links, node, &backup).map_err(|reason| refuse(&links, reason))?
        && !outcome.files.contains(&links.display().to_string())
    {
        outcome.files.push(links.display().to_string());
    }

    let marker = serde_json::to_vec(&Marker {
        version: MARKER_VERSION,
        node: node.to_string(),
    })
    .map_err(|error| refuse(&marker_path, error.to_string()))?;
    if read_marker(&marker_path).ok().flatten().as_deref() != Some(node.as_str()) {
        write(&marker_path, &marker).map_err(|reason| refuse(&marker_path, reason))?;
    }
    Ok(outcome)
}

type JsonConverter = fn(&mut Value, &str, &BTreeMap<String, String>) -> bool;

/// The size a store's own loader refuses beyond: the node id is longer than
/// `local`, so a store near its limit could otherwise convert into one its
/// loader would then refuse on every start.
fn stored_limit(file: &str) -> usize {
    match file {
        DELIVERY_LEDGER => crate::delivery::FILE_LIMIT,
        _ => usize::MAX,
    }
}

/// Each JSON store, the field that names its version, and the versions this
/// build reads. A file of any other version, or with none, is left as it is
/// for its own loader, which reports it or keeps it aside byte for byte.
const JSON_STORES: &[(&str, &str, &[u64], JsonConverter)] = &[
    (CORE_STATE, "schema_version", &[1], convert_core_state),
    (
        WORKSPACE_VIEWS,
        "schema_version",
        &[1, 2, 3],
        convert_workspace_views,
    ),
    (LABELS, "version", &[1], convert_labels),
    (DELIVERY_LEDGER, "version", &[1], convert_delivery_ledger),
];

/// Whether the store is one of the versions this build converts.
fn current(value: &Value, version_key: &str, versions: &[u64]) -> bool {
    value
        .get(version_key)
        .and_then(Value::as_u64)
        .is_some_and(|version| versions.contains(&version))
}

/// Whether the file is already larger than its loader reads.
fn over_limit(path: &Path, limit: usize) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > limit as u64)
}

pub(crate) fn read_marker(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<Marker>(&bytes)
            .map_err(|error| format!("node.json is not readable: {error}"))
            .and_then(|marker| {
                if marker.version == MARKER_VERSION {
                    Ok(Some(marker.node))
                } else {
                    Err(format!("node.json version {} is not known", marker.version))
                }
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// A store's JSON, or `None` when there is no file. A file that does not
/// parse is left to its own loader, which already handles a damaged store,
/// and is not converted.
pub(crate) fn read_json(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    hide_platform::fs::atomic::write_file(path, bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The folders this machine's registrations name: their Projects' roots.
/// The folders registered on this machine, under either name, so a start
/// retried after the file was converted still finds them.
fn registered_paths(core_state: &Value, node: &NodeId) -> Vec<String> {
    core_state
        .get("workspace_registrations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("device_id")
                .and_then(Value::as_str)
                .is_none_or(|device| device == LEGACY || device == node.as_str())
        })
        .filter_map(|row| row.get("path").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Sets `slot` to `node` when it holds the legacy id.
fn rewrite(slot: Option<&mut Value>, node: &str) -> bool {
    match slot {
        Some(value) if value.as_str() == Some(LEGACY) => {
            *value = Value::String(node.to_owned());
            true
        }
        _ => false,
    }
}

fn each<'a>(value: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

/// Renames the object key `"local"` to `node`. When both exist, an object or
/// a list takes the legacy entries the node's lacks, and any other value the
/// node wrote since stands.
fn rename_key(object: Option<&mut Value>, node: &str) -> bool {
    let Some(map) = object.and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(legacy) = map.remove(LEGACY) else {
        return false;
    };
    match map.get_mut(node) {
        Some(Value::Object(current)) => {
            if let Value::Object(legacy) = legacy {
                for (key, entry) in legacy {
                    current.entry(key).or_insert(entry);
                }
            }
        }
        // An expanded-paths list keeps the paths of both.
        Some(Value::Array(current)) => {
            if let Value::Array(legacy) = legacy {
                let mut held: HashSet<String> = current.iter().map(Value::to_string).collect();
                current.extend(
                    legacy
                        .into_iter()
                        .filter(|entry| held.insert(entry.to_string())),
                );
            }
        }
        // A value the node wrote since is the newer one.
        Some(_) => {}
        None => {
            map.insert(node.to_owned(), legacy);
        }
    }
    true
}

fn convert_core_state(value: &mut Value, node: &str, _projects: &BTreeMap<String, String>) -> bool {
    let mut changed = false;
    for row in each(value, "workspace_registrations") {
        match row.get_mut("device_id") {
            // A registration from before devices existed names this machine.
            None => {
                if let Some(row) = row.as_object_mut() {
                    row.insert("device_id".to_owned(), Value::String(node.to_owned()));
                    changed = true;
                }
            }
            slot => changed |= rewrite(slot, node),
        }
    }
    changed |= rewrite(value.get_mut("focused_device_id"), node);
    for device in each(value, "expanded_inactive_project_device_ids") {
        changed |= rewrite(Some(device), node);
    }
    // A list that already named the node holds it once.
    if let Some(devices) = value
        .get_mut("expanded_inactive_project_device_ids")
        .and_then(Value::as_array_mut)
    {
        let mut seen = HashSet::new();
        devices.retain(|device| seen.insert(device.to_string()));
    }
    for row in each(value, "recent_checkouts") {
        changed |= rewrite(row.get_mut("device_id"), node);
    }
    if let Some(items) = value
        .get_mut("detail")
        .and_then(|detail| detail.get_mut("items"))
        .and_then(Value::as_array_mut)
    {
        for item in items {
            changed |= rewrite(
                item.get_mut("session")
                    .and_then(|session| session.get_mut("device_id")),
                node,
            );
        }
    }
    changed |= rename_key(value.get_mut("device_expanded_paths"), node);
    for fold in each(value, "session_open_folds") {
        for prefix in ["cleanup", "no_agents"] {
            if fold.as_str() == Some(&format!("{prefix}/{LEGACY}")) {
                *fold = Value::String(format!("{prefix}/{node}"));
                changed = true;
            }
        }
    }
    if let Some(dormant) = value
        .get_mut("agent_sleep")
        .and_then(|sleep| sleep.get_mut("dormant"))
        .and_then(Value::as_object_mut)
    {
        for record in dormant.values_mut() {
            changed |= rewrite(record.get_mut("node_id"), node);
        }
    }
    changed
}

fn convert_workspace_views(value: &mut Value, node: &str, _: &BTreeMap<String, String>) -> bool {
    let mut changed = false;
    for row in each(value, "workspaces") {
        changed |= rewrite(row.get_mut("device_id"), node);
        changed |= each_opener(row, &mut |device| rewrite(Some(device), node));
        if let Some(feedback) = row.get_mut("feedback") {
            for comment in each(feedback, "comments") {
                changed |= rewrite(
                    comment
                        .get_mut("recipient")
                        .and_then(|recipient| recipient.get_mut("device_id")),
                    node,
                );
                changed |= rewrite(
                    comment
                        .get_mut("shot")
                        .and_then(|shot| shot.get_mut("node")),
                    node,
                );
            }
            for send in each(feedback, "waiting") {
                changed |= rewrite(
                    send.get_mut("to").and_then(|to| to.get_mut("device_id")),
                    node,
                );
            }
        }
    }
    changed
}

/// Calls `change` with every View opener's machine in one Workspace's
/// layout, at any depth.
pub(crate) fn each_opener(view: &mut Value, change: &mut dyn FnMut(&mut Value) -> bool) -> bool {
    fn walk(value: &mut Value, change: &mut dyn FnMut(&mut Value) -> bool) -> bool {
        match value {
            Value::Object(map) => {
                let mut changed = false;
                for (key, entry) in map.iter_mut() {
                    if key == "opener" {
                        if let Some(device) = entry.get_mut("device_id") {
                            changed |= change(device);
                        }
                    } else {
                        changed |= walk(entry, change);
                    }
                }
                changed
            }
            Value::Array(items) => items
                .iter_mut()
                .fold(false, |changed, item| walk(item, change) | changed),
            _ => false,
        }
    }
    view.get_mut("view_layout")
        .is_some_and(|layout| walk(layout, change))
}

fn convert_labels(value: &mut Value, node: &str, _: &BTreeMap<String, String>) -> bool {
    rename_key(value.get_mut("targets"), node)
}

fn convert_delivery_ledger(value: &mut Value, node: &str, _: &BTreeMap<String, String>) -> bool {
    let mut changed = false;
    let actor = |actor: Option<&mut Value>| {
        rewrite(actor.and_then(|actor| actor.get_mut("device_id")), node)
    };
    for letter in each(value, "letters") {
        changed |= actor(letter.get_mut("sender"));
        changed |= actor(letter.get_mut("recipient"));
        changed |= actor(
            letter
                .get_mut("watch_warning")
                .and_then(|warning| warning.get_mut("target")),
        );
    }
    for watch in each(value, "watches") {
        changed |= actor(watch.get_mut("parent"));
        changed |= actor(watch.get_mut("target"));
    }
    for agent in each(value, "agents") {
        changed |= rewrite(agent.get_mut("machine"), node);
        changed |= rewrite(agent.get_mut("native_machine"), node);
        changed |= actor(agent.get_mut("actor"));
    }
    for spawn in each(value, "spawns") {
        changed |= rewrite(spawn.get_mut("machine"), node);
    }
    for wait in each(value, "identity_waits") {
        changed |= rewrite(wait.get_mut("machine"), node);
    }
    changed
}

/// A store that does not open is left to Project Memory, which already
/// reports a damaged store as unavailable; only a readable store that fails
/// to convert stops the start.
fn convert_memory(
    path: &Path,
    kept_as: &str,
    node: &NodeId,
    backup: &Backup,
) -> Result<usize, String> {
    let Ok(store) = crate::runtime::memory::open_memory_store(path) else {
        return Ok(0);
    };
    let pending: bool = store
        .has_device(LEGACY)
        .map_err(|error| error.to_string())?;
    if !pending {
        return Ok(0);
    }
    backup.copy_with(kept_as, |destination| {
        store
            .copy_to(destination)
            .map_err(|error| error.to_string())
    })?;
    store
        .convert_device(LEGACY, node.as_str())
        .map(|moved| moved.len())
        .map_err(|error| error.to_string())
}

/// The roots of the node's Projects in a Memory store; none when the store
/// does not open, which Project Memory reports on its own.
fn memory_roots(path: &Path, node: &NodeId) -> Result<Vec<String>, String> {
    let Ok(store) = crate::runtime::memory::open_memory_store(path) else {
        return Ok(Vec::new());
    };
    store
        .roots_of_device(node.as_str())
        .map_err(|error| error.to_string())
}

/// The link record opens as its worker opens it: one of a newer schema, or
/// one that does not open, is left for the worker to report, and a damaged
/// one is set aside and made again, as the worker would on its own start.
/// The record is rebuilt from session files and GitHub, but its copy keeps
/// the closed links only it remembers.
fn convert_links(path: &Path, node: &NodeId, backup: &Backup) -> Result<bool, String> {
    let Ok((mut store, _)) = crate::links::store::LinkStore::open(path) else {
        return Ok(false);
    };
    if !store.has_device(LEGACY)? {
        return Ok(false);
    }
    backup.copy_with(LINKS, |destination| store.copy_to(destination))?;
    store.convert_device(LEGACY, node.as_str())?;
    Ok(true)
}

/// `node-migration-backup/<started-at>/`, made on the first file it copies.
/// A start that resumes a conversion an earlier start left unfinished (no
/// `node.json` yet) keeps its originals in that start's folder, and a store
/// already copied there is not copied again, so a start retried after a
/// failure adds no copies (engineering principle 15).
struct Backup {
    directory: PathBuf,
}

impl Backup {
    fn new(state_dir: &Path, unfinished: bool) -> Self {
        let root = state_dir.join(BACKUP_DIR);
        let earlier = unfinished
            .then(|| std::fs::read_dir(&root).ok())
            .flatten()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u128>().ok())
            .max();
        let started = earlier.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default()
        });
        Self {
            directory: root.join(started.to_string()),
        }
    }

    fn copy(&self, path: &Path, kept_as: &str) -> Result<(), String> {
        self.copy_with(kept_as, |destination| {
            std::fs::copy(path, destination)
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
    }

    /// Keeps the original as `kept_as`, written under a temporary name and
    /// renamed, so a copy cut short is never taken for a kept original.
    fn copy_with(
        &self,
        kept_as: &str,
        copy: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let destination = self.directory.join(kept_as);
        if destination.exists() {
            return Ok(());
        }
        hide_platform::fs::private::create_dir_all(&self.directory)
            .map_err(|error| format!("the backup folder could not be made: {error}"))?;
        let partial = self.directory.join(format!(".{kept_as}.partial"));
        let _ = std::fs::remove_file(&partial);
        copy(&partial).map_err(|error| format!("the backup copy failed: {error}"))?;
        hide_platform::fs::private::restrict_to_owner(&partial)
            .map_err(|error| format!("the backup copy could not be made private: {error}"))?;
        std::fs::rename(&partial, &destination)
            .map_err(|error| format!("the backup copy could not be kept: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NODE: &str = "8f1c2d3e-4a5b-4c6d-8e7f-90a1b2c3d4e5";

    fn node() -> NodeId {
        NodeId::parse(NODE).unwrap()
    }

    /// A state folder as the build before node ids left it, with a value at
    /// every key `KEYS` names.
    struct Legacy {
        _dir: tempfile::TempDir,
        state: PathBuf,
        project: PathBuf,
        old_project: String,
    }

    fn legacy() -> Legacy {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let project = dir.path().join("projects/alpha");
        for path in [&state, &project] {
            std::fs::create_dir_all(path).unwrap();
        }
        let project = hide_platform::fs::identity::canonical(&project).unwrap();
        let root = project.to_string_lossy().into_owned();
        let old_project = hide_project::resolve(&project, LEGACY).unwrap().id;
        let write = |name: &str, value: Value| {
            std::fs::write(state.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        };
        write(
            CORE_STATE,
            json!({
                "schema_version": 1,
                "workspace_registrations": [
                    {"id": "workspace:a", "label": "alpha", "path": root, "device_id": "local", "primary_checkout_id": "workspace:a:checkout:1"},
                    {"id": "workspace:b", "label": "old", "path": "/gone/old"},
                    {"id": "remote:mini:project:p", "label": "mini", "path": "/srv/p", "device_id": "mini"},
                ],
                "focused_device_id": "local",
                "expanded_inactive_project_device_ids": ["local", "mini"],
                "recent_checkouts": [{"device_id": "local", "checkout_id": "workspace:a:checkout:1", "device_name": "This Mac", "path": root}],
                "device_registrations": [{"id": "mini", "label": "Mac mini", "ssh_alias": "mini"}],
                "device_expanded_paths": {"local": ["/x"], "mini": ["/srv/p"]},
                "resolved_sessions": {"w1:p1": {"at_unix_ms": 1}},
                "session_resolution_inputs": {"w1:p1": 1},
                "session_collapsed_checkout_ids": ["workspace:a:checkout:1"],
                "session_open_folds": ["workspace:a", "cleanup/local"],
                "factory_secretary_pane": "w1:p3",
                "selected_pane_id": "w1:p1",
                "expanded_agent_pane_ids": ["w1:p1"],
                "sessions_expanded_agent_pane_ids": ["w1:p1"],
                "detail": {"items": [
                    {"kind": "pull_request", "workspace_id": "workspace:a", "number": 7},
                    {"kind": "record", "session": {"device_id": "local", "agent": "claude", "session_id": "s"}},
                ], "shown": true},
                "recent_pane_ids": ["w1:p1", "remote:mini:pane:w2:p1"],
                "pane_text_scales": {"w1:p1": 1.2},
                "pane_read_records": {"w1:p1": {"seen_at_unix_ms": 1, "descendant_signals": [{"pane_id": "w1:p4"}]}},
                "pane_completions": {"w1:p1": {"session": "s", "server": "m", "state_change_seq": 4}},
                "pane_launches": {"w1:p1": {"kind": "claude", "choice": {"effort": "high"}, "session": "s"}},
                "request_verbs": {"w1:p1": "review"},
                "pane_terminal_sizes": {"w1:p1": {"cols": 80, "rows": 24}},
                "agent_sleep": {
                    "stamps": {"w1:p1": {}},
                    "records": {"w1:p1": {"parent_pane_id": "w1:p0", "cwd": root}},
                    "dormant": {"sleep-1": {"node_id": "local", "old_pane_id": "w1:p5", "cwd": root}},
                },
                "expanded_paths": [root],
                "operator_tab_names": {"w1:t1": "review"},
                "project_base_branches": {root.clone(): "main"},
                "project_issue_sources": {root.clone(): "github"},
                "project_settings": {
                    crate::project_settings::key(NODE, &root): {
                        "version": 1, "device_id": NODE, "root": root,
                        "repository": {"mode": "auto"}, "new_work": "worktree"
                    }
                },
                "expanded_inactive_checkout_project_paths": [root],
                "selected_path": root,
                "focused_checkout_id": "workspace:a",
                "collapsed_workspace_ids": ["workspace:a"],
                "collapsed_checkout_ids": ["workspace:a"],
                "expanded_checkout_ids": ["workspace:a"],
            }),
        );
        write(
            WORKSPACE_VIEWS,
            json!({"schema_version": 3, "workspaces": [
                {"device_id": "local", "path": root, "agent_layout": {"tabs": ["w1:t1"]}, "tab_views": {"w1:t1": "d1"}, "view_layout": {"root": {"area": {"displays": [{"id": "d1", "opener": {"device_id": "local", "terminal_id": "t1"}}]}}},
                 "feedback": {"comments": [{"id": "c2", "recipient": {"device_id": "local"}, "shot": {"node": "local", "name": "shot.png"}}], "waiting": [{"to": {"device_id": "local"}}]}},
                {"device_id": "mini", "path": "/srv/p"},
            ]}),
        );
        write(
            LABELS,
            json!({"version": 1, "targets": {"local": {"w1:p1": {"owner": "v1:a"}}}}),
        );
        let actor = |device: &str| json!({"pane_id": "w1:p1", "name": "a", "kind": "claude", "device_id": device});
        write(
            DELIVERY_LEDGER,
            json!({
                "version": 1, "next_id": 9,
                "letters": [{"sender": actor("local"), "recipient": actor("mini"),
                             "watch_warning": {"target": actor("local")}}],
                "watches": [{"parent": actor("local"), "target": actor("local")}],
                "agents": [{"machine": "local", "native_machine": "local", "actor": actor("local"), "pane": "w1:p1", "host_scope": "/tmp/herdr.sock",
                            "project": root, "session": "s1", "instance": "term_1"}],
                "spawns": [{"repo": root, "path": root, "requested_path": root, "pane": "w1:p2", "machine": "mini",
                            "task": {"kind": "memory_promote", "target": "m1"}}],
                "identity_waits": [{"id": "wait-8", "machine": "local", "pane": "w1:p3", "name": "fork-w1-p1-1-1", "kind": "codex",
                                    "owed": {"for": "fork", "origin": "agent-1"}}],
                "node_clocks": {"mini": {"connected_ms": 60_000, "since": null}},
            }),
        );
        write(
            "github-snapshot.json",
            json!({"projects": [{"root_path": root}]}),
        );
        write(
            "local-issues.json",
            json!({"version": 1, "projects": {root.clone(): []}, "devices": {"mini": {"/srv/p": []}}}),
        );

        let memory = hide_memory::MemoryStore::open(&state.join(PROJECT_MEMORY)).unwrap();
        memory
            .ensure_project(&old_project, &project, LEGACY)
            .unwrap();
        drop(memory);
        let connection = rusqlite::Connection::open(state.join(PROJECT_MEMORY)).unwrap();
        connection
            .execute(
                "INSERT INTO receipt_cursors VALUES(?1,'claude','s1',0,x'',NULL,1,0)",
                [&old_project],
            )
            .unwrap();
        drop(connection);

        // The session search file an earlier build wrote: only its Copied
        // history days are still read.
        let connection = rusqlite::Connection::open(state.join(SESSION_SEARCH)).unwrap();
        connection
            .execute_batch("CREATE TABLE policy(project TEXT PRIMARY KEY, days INTEGER NOT NULL);")
            .unwrap();
        connection
            .execute("INSERT INTO policy VALUES(?1, 30)", [&old_project])
            .unwrap();
        let (links, _) = crate::links::store::LinkStore::open(&state.join(LINKS)).unwrap();
        drop(links);
        let connection = rusqlite::Connection::open(state.join(LINKS)).unwrap();
        connection
            .execute_batch(&format!(
                "INSERT INTO projects VALUES('{old}','local','{root}','id:R',NULL,1);
                 INSERT INTO projects VALUES('project:mini','mini','/srv/p',NULL,NULL,1);
                 INSERT INTO prs VALUES('id:R',7,'{old}','b','t','u',1,NULL,NULL,1);
                 INSERT INTO pr_issues VALUES('id:R',7,'local:{root}#4','hide',1,NULL);
                 INSERT INTO worktrees VALUES('{old}','{root}','b',1,1);
                 INSERT INTO sessions(device,agent,id,path,cwd) VALUES('local','claude','s1','/f','{root}');
                 INSERT INTO sessions(device,agent,id) VALUES('mini','claude','s1');
                 INSERT INTO session_branches(device,agent,id,file,branch,first_at,last_at)
                     VALUES('local','claude','s1','/f','b',1,1);
                 INSERT INTO session_prs(device,agent,id,file,repo_name,number,at)
                     VALUES('local','claude','s1','/f','acme/p',7,1);
                 INSERT INTO session_parents VALUES('local','claude','s2','claude','s1','p','local');
                 INSERT INTO cursors(device,path,agent,stamp) VALUES('local','/f','claude','1');
                 INSERT INTO meta VALUES('listed_at','1');",
                old = old_project,
            ))
            .unwrap();
        drop(connection);
        Legacy {
            _dir: dir,
            state,
            project,
            old_project,
        }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Every value a pointer pattern reaches; `*` is each array element or
    /// object value, `{key}` each object key.
    pub(super) fn reach(value: &Value, pattern: &str) -> Vec<Value> {
        let mut found = vec![value.clone()];
        for part in pattern.trim_start_matches('/').split('/') {
            found = found
                .iter()
                .flat_map(|value| match part {
                    "*" => match value {
                        Value::Array(items) => items.clone(),
                        Value::Object(map) => map.values().cloned().collect(),
                        _ => Vec::new(),
                    },
                    "{key}" => value
                        .as_object()
                        .map(|map| map.keys().cloned().map(Value::String).collect())
                        .unwrap_or_default(),
                    key => value.get(key).cloned().into_iter().collect(),
                })
                .collect();
        }
        found
    }

    fn snapshot(state: &Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(state)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn every_machine_bound_key_is_either_rewritten_to_the_node_or_owned_through_node_json() {
        let legacy = legacy();
        let before: BTreeMap<String, Value> = KEYS
            .iter()
            .filter(|(file, ..)| file.ends_with(".json"))
            .map(|(file, ..)| (file.to_string(), read(&legacy.state.join(file))))
            .collect();
        // The fixture holds a value at every key the table names, so a key
        // the table adds without a fixture value fails here.
        for (file, pattern, _, _) in KEYS.iter().filter(|(file, ..)| file.ends_with(".json")) {
            assert!(
                !reach(&before[*file], pattern).is_empty(),
                "the legacy fixture has no value at {file}{pattern}"
            );
        }

        let outcome = convert(&legacy.state, &node()).unwrap();
        assert!(outcome.files.len() >= 6, "{outcome:?}");

        for (file, pattern, mechanism, _) in
            KEYS.iter().filter(|(file, ..)| file.ends_with(".json"))
        {
            let after = reach(&read(&legacy.state.join(file)), pattern);
            match mechanism {
                Mechanism::Rewritten => {
                    assert!(
                        !after.contains(&json!("local")),
                        "{file}{pattern} still names `local`: {after:?}"
                    );
                }
                Mechanism::Owned => assert_eq!(
                    after,
                    reach(&before[*file], pattern),
                    "{file}{pattern} is node-owned and must not change"
                ),
            }
        }
        // No converted store names the legacy machine anywhere.
        for file in [CORE_STATE, WORKSPACE_VIEWS, LABELS, DELIVERY_LEDGER] {
            let text = std::fs::read_to_string(legacy.state.join(file)).unwrap();
            assert!(!text.contains("\"local\""), "{file}: {text}");
        }
        let core_state = read(&legacy.state.join(CORE_STATE));
        assert_eq!(core_state["workspace_registrations"][1]["device_id"], NODE);
        assert_eq!(
            core_state["workspace_registrations"][2]["device_id"],
            "mini"
        );
        assert_eq!(core_state["device_expanded_paths"][NODE], json!(["/x"]));
        let new_project = hide_project::project_id(NODE, &legacy.project);
        assert_eq!(
            read(&legacy.state.join(LABELS))["targets"][NODE]["w1:p1"]["owner"],
            "v1:a"
        );
        let ledger = read(&legacy.state.join(DELIVERY_LEDGER));
        assert_eq!(ledger["letters"][0]["recipient"]["device_id"], "mini");
        assert_eq!(ledger["agents"][0]["machine"], NODE);
        assert_eq!(ledger["agents"][0]["actor"]["device_id"], NODE);
        assert_eq!(ledger["identity_waits"][0]["machine"], NODE);

        let memory = hide_memory::MemoryStore::open(&legacy.state.join(PROJECT_MEMORY)).unwrap();
        assert!(!memory.has_device(LEGACY).unwrap());
        assert!(memory.has_device(NODE).unwrap());
        drop(memory);
        let connection = rusqlite::Connection::open(legacy.state.join(PROJECT_MEMORY)).unwrap();
        let cursor: String = connection
            .query_row("SELECT project_id FROM receipt_cursors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(cursor, new_project);
        // The session search file is only read (D-20); its retention row
        // reaches the project under its new id through the link record.
        let search = rusqlite::Connection::open(legacy.state.join(SESSION_SEARCH)).unwrap();
        let policy: String = search
            .query_row("SELECT project FROM policy", [], |row| row.get(0))
            .unwrap();
        assert_eq!(policy, legacy.old_project);
        drop(search);
        let (store, _) = crate::links::store::LinkStore::open(&legacy.state.join(LINKS)).unwrap();
        let policies = store.policies(&legacy.state.join(SESSION_SEARCH)).unwrap();
        assert_eq!(policies.get(&new_project), Some(&30));
        drop(store);
        assert_ne!(legacy.old_project, new_project);

        let links = rusqlite::Connection::open(legacy.state.join(LINKS)).unwrap();
        let count = |sql: &str| -> i64 { links.query_row(sql, [], |row| row.get(0)).unwrap() };
        for table in [
            "projects",
            "sessions",
            "session_branches",
            "session_prs",
            "session_parents",
            "cursors",
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT COUNT(*) FROM {table} WHERE device='local'"
                )),
                0,
                "{table} still names `local`"
            );
        }
        assert_eq!(
            count("SELECT COUNT(*) FROM session_parents WHERE parent_device='local'"),
            0
        );
        assert_eq!(
            count(
                "SELECT COUNT(*) FROM session_parents WHERE parent_device='8f1c2d3e-4a5b-4c6d-8e7f-90a1b2c3d4e5'"
            ),
            1
        );
        let moved: (String, String, String, String) = links
            .query_row(
                "SELECT (SELECT device FROM projects WHERE key=?1), \
                 (SELECT project FROM prs WHERE number=7), \
                 (SELECT project FROM worktrees), (SELECT device FROM cursors)",
                [&new_project],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            moved,
            (
                NODE.into(),
                new_project.clone(),
                new_project.clone(),
                NODE.into()
            )
        );
        // A remote device's rows and the node-owned keys stay as they were.
        assert_eq!(
            count("SELECT COUNT(*) FROM projects WHERE device='mini' AND key='project:mini'"),
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM sessions WHERE device='mini'"),
            1
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM pr_issues WHERE issue LIKE 'local:%#4'"),
            1
        );
        assert_eq!(count("SELECT COUNT(*) FROM meta WHERE key='listed_at'"), 1);

        assert_eq!(read(&legacy.state.join(MARKER_FILE))["node"], NODE);
    }

    /// The fields of the core state that hold no machine id, no Herdr id and
    /// no digest of one: the operator's own appearance and layout choices,
    /// whichever machine runs the core.
    const CORE_STATE_NAMES_NO_MACHINE: &[&str] = &[
        "schema_version",
        "left_sidebar_visible",
        "device_rail_visible",
        "right_panel_visible",
        "right_panel_section",
        "sidebar_lens",
        "task_view",
        "focus_from_herdr",
        "shortcut_bindings",
        "shortcut_bindings_imported",
        "browser_shortcut_bindings",
        "pet_visible",
        "pet_origin",
        "pet_shortcut",
        "machine_label",
        "accent_hex",
        "theme",
        "agent_logo_mode",
        "color_theme",
        "interface_language",
        "usage",
        "agent_onboarding",
        "agent_onboarding_agents",
        "first_run",
        "desktop_notifications",
        "desktop_notifications_done",
        "screen_intros_seen",
        "sidebar_width",
        "font_size",
        "editor_text_scale",
        "agent_sleep_after_hours",
        "issue_settings",
        "agent_start",
    ];

    /// A core state gains a field often, and a field that names a machine
    /// (a device id, a Herdr pane or tab, a digest of either) must move with
    /// the core or its move stops (`reown`). A field the core saves is
    /// therefore either a row of `KEYS` or listed above with the fact that it
    /// names no machine, so a new one cannot reach a move unclassified.
    #[test]
    fn every_field_the_core_saves_is_a_row_of_the_table_or_names_no_machine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CORE_STATE);
        crate::persistence::save(
            &path,
            &crate::model::UiStateSnapshot::default(),
            &Default::default(),
        )
        .unwrap();
        let saved = read(&path);
        let rows: Vec<&str> = KEYS
            .iter()
            .filter(|(file, ..)| *file == CORE_STATE)
            .map(|(_, pattern, ..)| pattern.trim_start_matches('/').split('/').next().unwrap())
            .collect();
        for field in saved.as_object().unwrap().keys() {
            let covered = rows.contains(&field.as_str());
            let unbound = CORE_STATE_NAMES_NO_MACHINE.contains(&field.as_str());
            assert!(
                covered != unbound,
                "{CORE_STATE} saves `{field}`, which must be a row of KEYS or listed in \
                 CORE_STATE_NAMES_NO_MACHINE, and only one of them"
            );
        }
        for field in CORE_STATE_NAMES_NO_MACHINE {
            // A field the core leaves out until it has a value is not in a
            // default save; every other listed field is.
            assert!(
                saved.get(field).is_some()
                    || matches!(*field, "focus_from_herdr" | "machine_label"),
                "{field} is listed as machine-free but the core no longer saves it"
            );
        }
    }

    #[test]
    fn every_file_a_state_folder_holds_is_listed_with_how_its_machine_keys_are_kept() {
        let legacy = legacy();
        convert(&legacy.state, &node()).unwrap();
        let listed: Vec<&str> = KEYS
            .iter()
            .map(|(file, ..)| *file)
            .chain(UNBOUND.iter().copied())
            .collect();
        for entry in std::fs::read_dir(&legacy.state).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                listed.contains(&name.as_str()),
                "{name} is in the state folder but neither KEYS nor UNBOUND names it"
            );
        }
        for file in UNBOUND {
            assert!(!KEYS.iter().any(|(listed, ..)| listed == file), "{file}");
        }
    }

    #[test]
    fn the_originals_are_kept_and_a_second_start_writes_nothing() {
        let legacy = legacy();
        let before = snapshot(&legacy.state);
        convert(&legacy.state, &node()).unwrap();
        let backups: Vec<PathBuf> = std::fs::read_dir(legacy.state.join(BACKUP_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1);
        for file in [CORE_STATE, WORKSPACE_VIEWS, LABELS, DELIVERY_LEDGER] {
            assert_eq!(
                std::fs::read(backups[0].join(file)).unwrap(),
                before[file],
                "{file}"
            );
        }
        assert!(backups[0].join(PROJECT_MEMORY).is_file());
        let copy = rusqlite::Connection::open(backups[0].join(LINKS)).unwrap();
        let legacy_rows: i64 = copy
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE device='local'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            legacy_rows, 1,
            "the links copy holds the rows before the move"
        );

        let converted = snapshot(&legacy.state);
        let again = convert(&legacy.state, &node()).unwrap();
        assert_eq!(again, Outcome::default());
        let after = snapshot(&legacy.state);
        assert_eq!(
            after.keys().collect::<Vec<_>>(),
            converted.keys().collect::<Vec<_>>()
        );
        for (file, bytes) in &after {
            assert!(bytes == &converted[file], "{file} was written again");
        }
    }

    #[test]
    fn a_store_that_cannot_be_converted_stops_the_start_names_the_file_and_keeps_the_rest() {
        let legacy = legacy();
        // The node's own copy of the Project already exists: moving the
        // legacy one onto it would merge two Projects, so nothing moves.
        let memory = hide_memory::MemoryStore::open(&legacy.state.join(PROJECT_MEMORY)).unwrap();
        memory
            .ensure_project(
                &hide_project::project_id(NODE, &legacy.project),
                &legacy.project,
                NODE,
            )
            .unwrap();
        drop(memory);
        let before = snapshot(&legacy.state);

        let refusal = convert(&legacy.state, &node()).unwrap_err();
        assert_eq!(refusal.file, legacy.state.join(PROJECT_MEMORY));
        assert!(
            refusal.to_string().contains("project-memory.sqlite3"),
            "{refusal}"
        );
        let after = snapshot(&legacy.state);
        for file in [
            CORE_STATE,
            WORKSPACE_VIEWS,
            LABELS,
            DELIVERY_LEDGER,
            SESSION_SEARCH,
            LINKS,
        ] {
            assert_eq!(after[file], before[file], "{file} changed");
        }
        assert!(!legacy.state.join(MARKER_FILE).exists());
        let memory = hide_memory::MemoryStore::open(&legacy.state.join(PROJECT_MEMORY)).unwrap();
        assert!(
            memory.has_device(LEGACY).unwrap(),
            "the transaction rolled back"
        );
    }

    #[test]
    fn a_conversion_leaves_the_search_file_byte_for_byte_and_its_retention_follows_each_project() {
        let legacy = legacy();
        // A Project Memory knows but no folder registers.
        let beta = legacy.project.with_file_name("beta");
        std::fs::create_dir_all(&beta).unwrap();
        let old_beta = hide_project::project_id(LEGACY, &beta);
        let memory = hide_memory::MemoryStore::open(&legacy.state.join(PROJECT_MEMORY)).unwrap();
        memory.ensure_project(&old_beta, &beta, LEGACY).unwrap();
        drop(memory);
        let search_path = legacy.state.join(SESSION_SEARCH);
        let search = rusqlite::Connection::open(&search_path).unwrap();
        search
            .execute("INSERT INTO policy VALUES(?1, 7)", [&old_beta])
            .unwrap();
        drop(search);
        let before = std::fs::read(&search_path).unwrap();

        convert(&legacy.state, &node()).unwrap();
        assert_eq!(
            std::fs::read(&search_path).unwrap(),
            before,
            "never written"
        );
        let (store, _) = crate::links::store::LinkStore::open(&legacy.state.join(LINKS)).unwrap();
        let policies = store.policies(&search_path).unwrap();
        assert_eq!(
            policies.get(&hide_project::project_id(NODE, &beta)),
            Some(&7)
        );
        assert_eq!(
            policies.get(&hide_project::project_id(NODE, &legacy.project)),
            Some(&30)
        );
    }

    #[test]
    fn a_converted_folder_starts_while_another_writer_holds_the_search_index() {
        let legacy = legacy();
        convert(&legacy.state, &node()).unwrap();
        let writer = rusqlite::Connection::open(legacy.state.join(SESSION_SEARCH)).unwrap();
        writer.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert_eq!(convert(&legacy.state, &node()).unwrap(), Outcome::default());
        writer.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn a_link_row_the_move_would_land_on_stops_the_start_and_rolls_the_record_back() {
        let legacy = legacy();
        let links = rusqlite::Connection::open(legacy.state.join(LINKS)).unwrap();
        links
            .execute(
                "INSERT INTO sessions(device,agent,id) VALUES(?1,'claude','s1')",
                [NODE],
            )
            .unwrap();
        drop(links);
        let refusal = convert(&legacy.state, &node()).unwrap_err();
        assert_eq!(refusal.file, legacy.state.join(LINKS));
        assert!(refusal.reason.contains("sessions"), "{refusal}");
        let links = rusqlite::Connection::open(legacy.state.join(LINKS)).unwrap();
        let legacy_rows: i64 = links
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE device='local'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(legacy_rows, 1, "the transaction rolled back");
        assert!(!legacy.state.join(MARKER_FILE).exists());
        drop(links);

        // Retried, it fails the same way and adds no copies.
        convert(&legacy.state, &node()).unwrap_err();
        let backups: Vec<PathBuf> = std::fs::read_dir(legacy.state.join(BACKUP_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        let mut kept: Vec<String> = std::fs::read_dir(&backups[0])
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert!(kept.contains(&LINKS.to_owned()), "{kept:?}");
        assert!(
            kept.iter().all(|name| !name.ends_with(".partial")),
            "{kept:?}"
        );
        let copy = rusqlite::Connection::open(backups[0].join(LINKS)).unwrap();
        let original_rows: i64 = copy
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE device='local'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(original_rows, 1, "the kept copy is the original");
    }

    #[test]
    fn a_folder_owned_by_another_node_is_refused_untouched() {
        let legacy = legacy();
        std::fs::write(
            legacy.state.join(MARKER_FILE),
            serde_json::to_vec(&json!({"version": 1, "node": "other-node"})).unwrap(),
        )
        .unwrap();
        let before = snapshot(&legacy.state);
        let refusal = convert(&legacy.state, &node()).unwrap_err();
        assert_eq!(refusal.file, legacy.state.join(MARKER_FILE));
        assert!(refusal.reason.contains("other-node"), "{refusal}");
        assert_eq!(snapshot(&legacy.state), before);
    }

    #[test]
    fn a_store_of_a_version_this_build_does_not_read_is_left_to_its_loader() {
        let dir = tempfile::tempdir().unwrap();
        let unknown = br#"{"schema_version": 99, "workspaces": [{"device_id": "local"}]}"#;
        std::fs::write(dir.path().join(WORKSPACE_VIEWS), unknown).unwrap();
        let outcome = convert(dir.path(), &node()).unwrap();
        assert!(outcome.files.is_empty(), "{outcome:?}");
        assert_eq!(
            std::fs::read(dir.path().join(WORKSPACE_VIEWS)).unwrap(),
            unknown
        );
        assert!(!dir.path().join(BACKUP_DIR).exists());
    }

    #[test]
    fn a_folder_both_names_wrote_keeps_the_entries_of_both() {
        let dir = tempfile::tempdir().unwrap();
        let core_state = json!({
            "schema_version": 1,
            "device_expanded_paths": {"local": ["/a", "/b"], NODE: ["/b", "/c"]},
            "expanded_inactive_project_device_ids": ["local", NODE, "mini"],
        });
        std::fs::write(dir.path().join(CORE_STATE), core_state.to_string()).unwrap();
        convert(dir.path(), &node()).unwrap();
        assert_eq!(
            read(&dir.path().join(CORE_STATE))["device_expanded_paths"],
            json!({NODE: ["/b", "/c", "/a"]})
        );
        assert_eq!(
            read(&dir.path().join(CORE_STATE))["expanded_inactive_project_device_ids"],
            json!([NODE, "mini"])
        );
    }

    #[test]
    fn a_ledger_the_longer_id_would_push_past_its_loader_limit_stops_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let actor = json!({"device_id": "local", "pane_id": "p"});
        let letter = json!({"sender": actor, "recipient": actor});
        // Each letter grows by twice the id's extra length once converted.
        let letter_bytes = letter.to_string().len() + 1;
        let growth = 2 * (NODE.len() - LEGACY.len());
        let count = crate::delivery::FILE_LIMIT / (letter_bytes + growth) + 1;
        let ledger = json!({"version": 1, "letters": vec![letter.clone(); count]});
        let bytes = serde_json::to_vec(&ledger).unwrap();
        assert!(
            bytes.len() <= crate::delivery::FILE_LIMIT,
            "fixture starts loadable"
        );
        std::fs::write(dir.path().join(DELIVERY_LEDGER), &bytes).unwrap();
        // A store converted before the ledger in a start that went through.
        let core_state = json!({"schema_version": 1, "focused_device_id": "local"}).to_string();
        std::fs::write(dir.path().join(CORE_STATE), &core_state).unwrap();
        let before = snapshot(dir.path());

        let refusal = convert(dir.path(), &node()).unwrap_err();
        assert_eq!(refusal.file, dir.path().join(DELIVERY_LEDGER));
        assert!(
            refusal.to_string().contains("move the file aside"),
            "{refusal}"
        );
        assert_eq!(snapshot(dir.path()), before, "nothing was written");
        assert!(!dir.path().join(BACKUP_DIR).exists());
    }

    #[test]
    fn a_ledger_its_loader_already_refuses_is_left_to_that_loader() {
        let dir = tempfile::tempdir().unwrap();
        let actor = json!({"device_id": "local", "pane_id": "p"});
        let letter = json!({"sender": actor, "recipient": actor});
        let count = crate::delivery::FILE_LIMIT / letter.to_string().len() + 1;
        let bytes =
            serde_json::to_vec(&json!({"version": 1, "letters": vec![letter; count]})).unwrap();
        assert!(
            bytes.len() > crate::delivery::FILE_LIMIT,
            "fixture starts unloadable"
        );
        std::fs::write(dir.path().join(DELIVERY_LEDGER), &bytes).unwrap();

        let outcome = convert(dir.path(), &node()).unwrap();
        assert!(outcome.files.is_empty(), "{outcome:?}");
        assert_eq!(
            std::fs::read(dir.path().join(DELIVERY_LEDGER)).unwrap(),
            bytes
        );
        assert_eq!(
            read(&dir.path().join(MARKER_FILE)),
            json!({"version": 1, "node": NODE})
        );
    }

    #[test]
    fn an_empty_folder_gets_only_its_owner() {
        let dir = tempfile::tempdir().unwrap();
        let outcome = convert(dir.path(), &node()).unwrap();
        assert_eq!(outcome, Outcome::default());
        assert_eq!(
            read(&dir.path().join(MARKER_FILE)),
            json!({"version": 1, "node": NODE})
        );
    }
}
