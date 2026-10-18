use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod agent_areas;
pub(crate) mod agent_choice;
mod agent_close;
mod agent_groups;
mod agent_sleep;
mod agents;
mod attachments;
mod backlog;
mod checks_wait;
mod clone;
mod control_lane;
pub(crate) mod core_move;
pub(crate) mod delivery;
mod detail;
mod device_catalog;
mod devices;
mod disk_sweep;
mod documents;
mod drawn_ahead;
mod editor;
mod events;
mod factory;
mod first_run;
mod github_poll;
pub(crate) mod group_starts;
mod guardrails;
mod home;
mod hosts;
mod inbound;
mod issues;
mod kit;
pub(crate) mod links;
pub(crate) mod memory;
pub(crate) mod memory_analysis;
pub(crate) mod memory_evidence;
pub(crate) mod memory_promotion;
mod memory_verify;
mod op_timing;
mod operations;
pub(crate) mod output_language;
mod pane_prediction;
mod pane_reopen;
pub(crate) mod project_memory;
mod project_settings;
mod projects;
mod pull_requests;
mod recent_checkouts;
mod recent_panes;
mod rename;
mod request_view;
mod session;
mod session_archive;
mod session_state;
mod snapshot_delta;
mod ssh_hosts;
mod tab_focus;
mod terminal;
mod terminal_input;
mod tree_close;
pub(crate) mod usage_ledger;
mod usage_limit;
mod view_areas;
mod view_bookmarks;
mod view_feedback;
mod workspace_control;
mod workspace_view;

pub(crate) use factory::{
    DormantState, DormantWorker, FactoryPane, FactoryWake, WorkerProbe, factory_herdr_pane,
    factory_pane_id,
};
pub use hosts::WorkspaceRemoteRoute;
pub(crate) use kit::{DeviceKitAnswer, DeviceKitCall, DeviceKitWork, KitJob};
pub use snapshot_delta::serialize_snapshot_delta;

use agent_areas::AgentLayoutPayload;
use agent_groups::AgentGroupPayload;
use agent_sleep::{AgentSleepSetPayload, AgentWakePayload, SleepingSessionPayload};
use control_lane::ControlLane;
pub(crate) use control_lane::LaneStart;
use events::*;
pub(crate) use op_timing::HerdrArrival;
use operations::*;
#[cfg(test)]
pub(crate) use terminal::NoTerminals;
use view_areas::{BrowserOpenPayload, BrowserStatePayload, ViewLayoutPayload};
use workspace_view::{AreaIntent, WorkspaceViewPayload, WorkspaceViewStore};

use crate::agent_state::ReadRecordScope;
use crate::checkout_owner::{OwnerOpen, TabHost};
use crate::fork::{ForkRequest, ForkableAgent, fork_name, is_forkable};
use crate::handle::ChangeNotifier;
use crate::live::{
    LiveContext, PaneControlAction, PaneControlOutcome, PaneResizeDirection, PaneSplitDirection,
    RemoteControlAction, RemoteControlContext, RemoteControlOutcome, SessionFetchError,
};
use crate::model::{
    AgentStatusCode, ArchiveDetailSnapshot, CheckoutSnapshot, CoreOptions, DEFAULT_PANE_TEXT_SCALE,
    DiagnosticSnapshot, Edited, EditorDocumentSnapshot, EditorTabKind, EditorTabSnapshot,
    ExplorerOperationSnapshot, LastErrorSnapshot, LinkOrigin, OperatorFocusAck,
    PANE_TEXT_SCALE_STEP, PaneFindOpened, PaneFindRoute, PaneFindSnapshot,
    PaneFocusRequestSnapshot, PaneForkSnapshot, PaneLayoutNodeSnapshot, PaneLayoutSnapshot,
    PaneSnapshot, PetBadgesSnapshot, PetOriginSnapshot, PetSnapshot, RemoteFileEntrySnapshot,
    RemoteFileListSnapshot, RemoteSessionSnapshot, RightPanelSection, SCHEMA_VERSION,
    SessionRowSnapshot, SidebarAgentSnapshot, Snapshot, StripTabKind, StripTabSnapshot, Surface,
    TabSnapshot, TerminalPaneSnapshot, UiStateSnapshot, WorkspaceSnapshot, clamp_pane_text_scale,
};
use crate::recent_closed::{
    ClosedAgent, ClosedContext, ClosedItem, ClosedPane, ClosedTabName, push_bounded,
};
use crate::remote::DeviceConnector;
use crate::sidebar::{SessionSnapshotPayload, project_agents};
use crate::{environment, files, live, persistence, pet, session_sync, workspace};

fn conversation_agent_kind(kind: &str) -> bool {
    hide_agent_adapter::adapter(kind).is_some_and(|row| row.conversation.is_some())
}

/// Which Herdr workspace each tab belongs to, from each workspace's tab order.
pub(super) fn tab_owners(order: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, String> {
    order
        .iter()
        .flat_map(|(workspace_id, tab_ids)| {
            tab_ids
                .iter()
                .map(move |tab_id| (tab_id.clone(), workspace_id.clone()))
        })
        .collect()
}

/// A device's Herdr workspaces' tab orders, keyed by Herdr's own workspace id
/// and naming each tab by its device-scoped id, as the device's strips do.
/// The raw session carries one checkout per Herdr workspace, in Herdr's order.
pub(super) fn device_workspace_tab_order(
    raw: &RemoteSessionSnapshot,
) -> BTreeMap<String, Vec<String>> {
    raw.workspaces
        .iter()
        .filter_map(|workspace| {
            let herdr_id = workspace.session_workspace_ids.first()?;
            let tabs = workspace
                .checkouts
                .iter()
                .flat_map(|checkout| checkout.tabs.iter())
                .filter_map(|tab| tab.id.clone())
                .collect();
            Some((herdr_id.clone(), tabs))
        })
        .collect()
}

/// Places one checkout's strip from its Herdr and editor tabs, settling a
/// held reorder once Herdr reports the order it asked for.
///
/// A held reorder lands the moment Herdr reports the order it asked for,
/// whichever path carried it: the `tab_moved` event, a move Herdr had already
/// made, or a move made from the TUI. A held reorder whose tabs are no longer
/// the checkout's tabs can never be reported, so it is dropped rather than
/// kept waiting for an order that cannot arrive.
fn place_checkout_strip(
    checkout: &mut CheckoutSnapshot,
    editor: Vec<StripTabSnapshot>,
    owners: &BTreeMap<String, String>,
    order: &mut BTreeMap<String, Vec<String>>,
    pending: &mut BTreeMap<String, PendingTabMove>,
    dropped_moves: &mut Vec<String>,
) {
    let herdr = StripTabSnapshot::from_herdr_tabs(&checkout.tabs);
    let stored = order.entry(checkout.id.clone()).or_default();
    if let Some(held) = pending.get(&checkout.id) {
        // Only the tabs the move was asked about: the workspace it named, as
        // this checkout currently holds them.
        let live_owned = herdr
            .iter()
            .map(|entry| &entry.source_id)
            .filter(|tab_id| owners.get(*tab_id) == Some(&held.workspace_id))
            .cloned()
            .collect::<Vec<_>>();
        if live_owned.iter().cloned().collect::<BTreeSet<_>>()
            != held.herdr_order.iter().cloned().collect::<BTreeSet<_>>()
        {
            pending.remove(&checkout.id);
            dropped_moves.push(checkout.id.clone());
        } else if live_owned == held.herdr_order {
            *stored = held.desired.clone();
            pending.remove(&checkout.id);
        }
    }
    checkout.strip = ordered_strip(stored, &herdr, &editor, owners);
    *stored = checkout
        .strip
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
}

/// Places one checkout's tab strip.
///
/// The strip is a sequence of slots. A slot an entry already held it keeps, so
/// a file tab the operator dropped between two Herdr tabs stays where it was
/// put. The Herdr slots are then filled from Herdr's own tab order, because
/// Herdr owns where its tabs sit and a tab it moved has to move here too. A
/// tab that is new to the strip takes a slot at the end, which is where both a
/// newly opened file and a newly created Herdr tab belong.
fn ordered_strip(
    stored: &[String],
    herdr: &[StripTabSnapshot],
    files: &[StripTabSnapshot],
    workspace_of: &BTreeMap<String, String>,
) -> Vec<StripTabSnapshot> {
    let mut by_id = BTreeMap::new();
    for entry in herdr.iter().chain(files.iter()) {
        by_id.insert(entry.id.as_str(), entry.clone());
    }
    // A stored order that names the same tab twice holds one slot, not two, so
    // the Herdr slot count stays equal to the Herdr tab count below.
    let mut placed = Vec::with_capacity(herdr.len() + files.len());
    let mut held = BTreeSet::new();
    for id in stored {
        let Some(entry) = by_id.get(id.as_str()) else {
            continue;
        };
        if held.insert(entry.id.clone()) {
            placed.push(entry.clone());
        }
    }
    placed.extend(
        herdr
            .iter()
            .chain(files.iter())
            .filter(|entry| !held.contains(&entry.id))
            .cloned(),
    );
    // Herdr orders its own workspace's tabs; it has no order that spans two of
    // them. So each Herdr slot is filled from the queue of the workspace that
    // slot already belongs to, which leaves Hide owning how the workspaces and
    // the file tabs interleave. Refilling from one flat queue is what undid
    // every drag in a checkout two Herdr workspaces share.
    let mut queues: BTreeMap<&str, VecDeque<&StripTabSnapshot>> = BTreeMap::new();
    let owner = |entry: &StripTabSnapshot| {
        workspace_of
            .get(&entry.source_id)
            .map_or("", |workspace_id| workspace_id.as_str())
    };
    for entry in herdr {
        queues.entry(owner(entry)).or_default().push_back(entry);
    }
    for entry in &mut placed {
        if entry.kind == StripTabKind::Herdr {
            *entry = queues
                .get_mut(owner(entry))
                .and_then(VecDeque::pop_front)
                .expect("every Herdr slot has a Herdr tab from its own workspace to fill it")
                .clone();
        }
    }
    placed
}

/// One reorder the operator asked for that Herdr has not reported back yet.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingTabMove {
    /// The whole strip order the entry was dropped into, as strip entry ids.
    desired: Vec<String>,
    /// The Herdr workspace the move was asked of. A checkout can hold tabs
    /// from several workspaces, and `tab.move` is indexed inside one of them,
    /// so this is the workspace whose reported order settles the move.
    workspace_id: String,
    /// The moved tab's own workspace's tabs in this checkout, in the order
    /// the operator asked for. That workspace reporting exactly this order is
    /// what commits the arrangement.
    herdr_order: Vec<String>,
    target_id: String,
    generation: u64,
    connection_generation: u64,
    phase: String,
    stage: String,
    started_at_unix_ms: u64,
    deadline_at_unix_ms: Option<u64>,
    message: Option<String>,
    retryable: bool,
}

/// The Herdr that carries a tab move: this machine's, or the device whose
/// checkout the moved tab is in.
enum TabMoveCarrier {
    Local,
    Device(RemoteControlContext),
}

/// Translates a wanted Herdr tab order into the index `tab.move` takes.
///
/// Herdr counts the insertion point in the list it still holds, before the
/// moved tab is taken out of it, so the index is the current position of
/// whichever tab is to end up behind the moved one.
///
/// `workspace_order` is the whole Herdr workspace's tab list, because that is
/// the list Herdr indexes. `desired` is the order the operator asked for in
/// one checkout's strip, which is a subset of it: a workspace's tabs are split
/// across a repository and its worktrees whenever their panes are. Reading the
/// index off the subset instead lands the tab elsewhere as soon as the
/// checkout's tabs do not start at the workspace's first position.
///
/// A tab dropped at the end of its checkout's strip has no successor there, so
/// it goes in front of whichever workspace tab currently follows the checkout's
/// last tab, and at the end of the workspace when nothing follows.
///
/// `None` means the move cannot be expressed as a Herdr move: the named tab is
/// not one of Herdr's, or an anchor it needs is not in the workspace's list.
fn herdr_insert_index(
    workspace_order: &[String],
    desired: &[String],
    moved: &str,
) -> Option<usize> {
    let position = desired.iter().position(|tab_id| tab_id == moved)?;
    let index_of = |wanted: &str| workspace_order.iter().position(|tab_id| tab_id == wanted);
    if let Some(successor) = desired.get(position + 1) {
        return index_of(successor);
    }
    let Some(predecessor) = position
        .checked_sub(1)
        .and_then(|before| desired.get(before))
    else {
        // The checkout has one Herdr tab, so there is nothing to move it past.
        // Its own position is the index that leaves the workspace unchanged.
        return index_of(moved);
    };
    let after_predecessor = index_of(predecessor)? + 1;
    Some(
        workspace_order[after_predecessor..]
            .iter()
            .position(|tab_id| tab_id != moved)
            .map_or(workspace_order.len(), |offset| after_predecessor + offset),
    )
}

/// The folders that have to be open for a revealed path to have a row in the
/// tree: every ancestor between the checkout root and the path, and the path
/// itself when it is a folder.
///
/// The checkout root is left out because the outline always expands its own
/// root row; recording it would put a path in the persisted set that the view
/// never reads.
/// `path` with the `source` prefix replaced by `destination`, or `None`
/// when `path` is neither `source` nor inside it. Component-wise, so
/// `/repo/src2` is not inside `/repo/src`.
/// The checkout an explorer change was decided for: its tabs, its device's
/// expanded folders and its host are the ones the result applies to,
/// whatever is in front when the result lands.
#[derive(Clone, Debug)]
pub(crate) struct ExplorerTarget {
    pub(crate) workspace_id: String,
    pub(crate) checkout_id: String,
    pub(crate) device_id: String,
    pub(crate) root: String,
}

/// An Explorer change as the page asked for it. One asked while another runs
/// waits in `explorer_queue` and is planned when its turn comes, against the
/// checkout in front then; changes run one at a time in the order they were
/// asked, so one that relies on the change before it (a rename, then a move
/// of the renamed item) reaches the host after that change landed.
#[derive(Clone, Debug)]
pub(crate) struct ExplorerRequest {
    pub(crate) root: String,
    pub(crate) device: Option<String>,
    pub(crate) change: ExplorerChange,
}

#[derive(Clone, Debug)]
pub(crate) enum ExplorerChange {
    Create {
        kind: files::ExplorerOperationKind,
        parent: String,
        name: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Move {
        path: String,
        destination: String,
    },
    /// `path` copied into the folder `destination`.
    Copy {
        path: String,
        destination: String,
    },
    /// A drop's items written into the folder `destination`.
    Import {
        destination: String,
        items: Vec<files::ImportItem>,
    },
    Trash {
        path: String,
        select_after: String,
        inode: Option<u64>,
    },
}

impl ExplorerChange {
    fn kind(&self) -> files::ExplorerOperationKind {
        match self {
            Self::Create { kind, .. } => *kind,
            Self::Rename { .. } => files::ExplorerOperationKind::PathRename,
            Self::Move { .. } => files::ExplorerOperationKind::PathMove,
            Self::Copy { .. } => files::ExplorerOperationKind::PathCopy,
            Self::Import { .. } => files::ExplorerOperationKind::PathImport,
            Self::Trash { .. } => files::ExplorerOperationKind::PathTrash,
        }
    }

    /// The row the change started from, where a refusal is shown: the folder
    /// a copy or a drop was meant to land in, since that is where the
    /// operator pasted or dropped.
    fn started_from(&self) -> &str {
        match self {
            Self::Create { parent, .. } => parent,
            Self::Copy { destination, .. } | Self::Import { destination, .. } => destination,
            Self::Rename { path, .. } | Self::Move { path, .. } | Self::Trash { path, .. } => path,
        }
    }

    fn plan(&self, root: &str) -> Result<files::ExplorerOperation, String> {
        match self {
            Self::Create { kind, parent, name } => {
                files::ExplorerOperation::create(*kind, root, parent, name)
            }
            Self::Rename { path, name } => files::ExplorerOperation::rename(root, path, name),
            Self::Move { path, destination } => {
                files::ExplorerOperation::move_into(root, path, destination)
            }
            Self::Copy { path, destination } => {
                files::ExplorerOperation::copy_into(root, path, destination)
            }
            Self::Import { destination, items } => {
                files::ExplorerOperation::import_into(root, destination, items.clone())
            }
            Self::Trash {
                path,
                select_after,
                inode,
            } => files::ExplorerOperation::trash(root, path, select_after, *inode),
        }
    }
}

/// A change waiting behind the running one, with the checkout that was in
/// front when it was asked: it runs only if that checkout is still in front
/// when its turn comes.
#[derive(Debug)]
struct WaitingExplorerChange {
    owner: Option<(String, String)>,
    request: ExplorerRequest,
}

/// How many Explorer changes wait behind the running one. A drag and the
/// keys that follow it queue one or two; past this the newest is refused.
const EXPLORER_QUEUE_LIMIT: usize = 4;

fn retarget_path(path: &str, source: &str, destination: &str) -> Option<String> {
    if path == source {
        return Some(destination.to_owned());
    }
    let rest = path.strip_prefix(source)?.strip_prefix('/')?;
    Some(format!("{destination}/{rest}"))
}

/// The folders from `checkout_path` down to `path`, both wire spellings of
/// one device's paths, and `path` itself when it is a folder.
fn reveal_expansion_paths(checkout_path: &str, path: &str, is_directory: bool) -> Vec<String> {
    let Ok(relative) = hide_platform::path::wire_relative(checkout_path, path) else {
        return Vec::new();
    };
    let mut expanded = Vec::new();
    let mut current = hide_platform::path::RelPath::root();
    let mut names = relative.names().peekable();
    while let Some(name) = names.next() {
        if names.peek().is_none() && !is_directory {
            break;
        }
        current = current.join(name).expect("a name of a parsed path");
        expanded.push(hide_platform::path::wire_join(checkout_path, &current));
    }
    expanded
}

/// How long the pet plays its waking pose after activity interrupts sleep.
const PET_WAKING_MS: u64 = 1_200;

/// The absolute lifetime of one stage of a Herdr-owned mutation: restore
/// capture, the request itself, and the topology wait each get this long
/// before the operation becomes a caller-visible unknown result (PRD D-08).
const CLOSE_STAGE_TIMEOUT_MS: u64 = 5_000;

/// How long a view-state notification may stay unconfirmed before Hide stops
/// treating Herdr's answer as pending.
///
/// The round trip measured on this machine is roughly 130 ms for the request
/// and 170 ms more for the confirming event, so three seconds never trips on
/// a healthy server while still releasing a value quickly when Herdr has
/// stopped answering. The value is kept when the wait expires; only the
/// waiting stops.
const VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS: u64 = 3_000;

// How many sent, unanswered tab moves Hide remembers at once, the same
// number of Herdr's moves the replica keeps. Each lives at most
// `VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS`; a burst of tab switches faster than
// that evicts the oldest and reports it.
use crate::sidebar::TAB_FOCUS_LIMIT;

/// A pane focus Hide has already made and told Herdr about.
///
/// Hide owns the focused pane, so the value in the snapshot is not a
/// prediction to be undone. This records only that a notification is in
/// flight, which is what tells a Herdr layout naming an older pane apart
/// from an operator focusing something outside Hide. The tab moves a pane
/// focus makes are kept with the tab focuses (`tab_focus.rs`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingViewFocus {
    /// The pane id Hide asked Herdr to focus.
    target_id: String,
    /// Present only for an explicit relationship Open/Return request. The
    /// core owns the outcome; the shell supplies this opaque correlation id
    /// only so it can consume the answer to the action it initiated.
    request_id: Option<String>,
    /// Local pane controls settle by operation identity, never by target alone.
    pane_control_serial: Option<u64>,
    /// Whether the request has left for Herdr. A request still waiting its
    /// turn on the control lane cannot be answered yet, and its wait does
    /// not run out.
    sent: bool,
    requested_at_unix_ms: u64,
}

impl PendingViewFocus {
    fn new(target_id: impl Into<String>) -> Self {
        Self {
            target_id: target_id.into(),
            request_id: None,
            pane_control_serial: None,
            sent: false,
            requested_at_unix_ms: unix_milliseconds(),
        }
    }

    fn pane_request(target_id: impl Into<String>, request_id: String) -> Self {
        Self {
            target_id: target_id.into(),
            request_id: Some(request_id),
            pane_control_serial: None,
            sent: false,
            requested_at_unix_ms: unix_milliseconds(),
        }
    }

    fn expired_at(&self, now_unix_ms: u64) -> bool {
        self.sent
            && now_unix_ms.saturating_sub(self.requested_at_unix_ms)
                >= VIEW_FOCUS_NOTIFICATION_TIMEOUT_MS
    }
}

/// One socket operation in the pane-focus chain. The latest intent lives in
/// `pending_pane_focus`; replacing it cannot start a second socket operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingPaneFocusControl {
    pub(crate) serial: u64,
    pub(crate) target_id: String,
    pub(crate) live_generation: u64,
}

/// How many diagnostics the snapshot keeps. Newest are kept; the oldest go.
const DIAGNOSTIC_RETENTION: usize = 256;
/// Pages whose last operator focus number the snapshot keeps, oldest evicted.
const OPERATOR_FOCUS_CLIENT_LIMIT: usize = 16;
/// Longest client id a page may give its operator focus events.
const OPERATOR_FOCUS_CLIENT_ID_LIMIT: usize = 64;

/// How many tabs keep their panes attached: the one on screen and the four
/// most recently shown.
///
/// Five is the operator's working set, not a memory bound. Time-based expiry
/// was rejected because the core owns no timer and the behaviour could not be
/// proved without one; a visit count is decided by the same events that draw
/// the screen.
pub(crate) const ATTACHED_TAB_LIMIT: usize = 5;

/// How long a relocation waits before Hide asks again.
///
/// A failed move is retried on the next ingest that finds the child still
/// split, and a delegated child changes state often enough that one arrives.
/// The window keeps a persistently refusing Herdr from being asked once per
/// event without adding a timer of its own (PRD B36).
/// A move Herdr acknowledged holds its stamp too, until the published layout
/// shows the child in its own tab; its events arrive after the answer.
const RELOCATION_RETRY_INTERVAL_MS: u64 = 5_000;

/// What Herdr says about its tabs, kept per Herdr workspace.
///
/// A checkout is keyed by path, so several Herdr workspaces can share one
/// checkout, and each of them names an active tab. Only the focused
/// workspace's active tab is a tab Herdr has focused; the others are each
/// workspace's memory of where it was last, and following one of those moved
/// the canvas away from the tab the operator had just chosen.
#[derive(Clone, Debug, Default, PartialEq)]
struct HerdrTabView {
    /// Herdr's active tab in each Herdr workspace.
    active_tab_by_workspace: BTreeMap<String, String>,
    /// The Herdr workspace that owns each tab.
    workspace_by_tab: BTreeMap<String, String>,
    /// The active tab of Herdr's focused workspace.
    focused_tab_id: Option<String>,
    focus_event: Option<crate::sidebar::SessionTabFocus>,
}

impl HerdrTabView {
    fn from_payload(payload: &SessionSnapshotPayload) -> Self {
        let active_tab_by_workspace = payload
            .workspaces
            .iter()
            .filter_map(|workspace| {
                let active_tab_id = workspace.active_tab_id.as_deref()?.trim();
                (!active_tab_id.is_empty())
                    .then(|| (workspace.workspace_id.clone(), active_tab_id.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut workspace_by_tab = payload
            .tabs
            .iter()
            .filter(|tab| !tab.workspace_id.trim().is_empty())
            .map(|tab| (tab.tab_id.clone(), tab.workspace_id.clone()))
            .collect::<BTreeMap<_, _>>();
        for layout in &payload.layouts {
            workspace_by_tab
                .entry(layout.tab_id.clone())
                .or_insert_with(|| layout.workspace_id.clone());
        }
        // The focused pane's workspace stands in when the session names no
        // focused workspace, which is the shape older payloads and the test
        // fixtures have.
        let focused_workspace_id = payload
            .focused_workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|workspace_id| !workspace_id.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                let focused_pane_id = payload.focused_pane_id.as_deref()?;
                payload
                    .layouts
                    .iter()
                    .find(|layout| {
                        layout
                            .panes
                            .iter()
                            .any(|pane| pane.pane_id == focused_pane_id)
                    })
                    .map(|layout| layout.workspace_id.clone())
            });
        let focused_tab_id = focused_workspace_id
            .as_deref()
            .and_then(|workspace_id| active_tab_by_workspace.get(workspace_id))
            .cloned();
        Self {
            focus_event: payload.tab_focus.clone().filter(|event| {
                focused_tab_id.as_ref() == Some(&event.tab_id)
                    && focused_workspace_id.as_ref() == Some(&event.workspace_id)
            }),
            active_tab_by_workspace,
            workspace_by_tab,
            focused_tab_id,
        }
    }

    /// Whether Herdr shows this tab in the workspace that owns it. That is
    /// what confirms a tab focus Hide asked for: which workspace Herdr's
    /// keyboard is in does not decide it. Older snapshots omit the active
    /// tab for a workspace that has only one tab; that shape is unambiguous.
    /// A multi-tab workspace without an active-tab verdict stays unknown so
    /// an old focused layout cannot acknowledge a new pane-focus request.
    fn is_active_in_its_workspace(&self, tab_id: &str) -> bool {
        let Some(workspace_id) = self.workspace_by_tab.get(tab_id) else {
            return false;
        };
        match self.active_tab_by_workspace.get(workspace_id) {
            Some(active_tab_id) => active_tab_id == tab_id,
            None => {
                self.workspace_by_tab
                    .values()
                    .filter(|candidate| *candidate == workspace_id)
                    .count()
                    == 1
            }
        }
    }

    fn active_tab_ids(&self) -> BTreeSet<String> {
        self.workspace_by_tab
            .keys()
            .filter(|tab_id| self.is_active_in_its_workspace(tab_id))
            .cloned()
            .collect()
    }
}

/// Which of the two view-state values a pending notification is about.
///
/// The tab and the pane run the same four paths - confirm, follow, refuse,
/// time out - and differ only in the words their records use. Naming those
/// words once is what keeps the paths from drifting apart: the refusal and
/// the timeout had already grown two different reporting shapes for the same
/// event before this existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewFocusSlot {
    Tab,
    Pane,
}

impl ViewFocusSlot {
    fn what(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Pane => "pane",
        }
    }

    fn id_key(self) -> &'static str {
        match self {
            Self::Tab => "tab_id",
            Self::Pane => "pane_id",
        }
    }

    fn refused_kind(self) -> &'static str {
        match self {
            Self::Tab => "tab.focus.refused",
            Self::Pane => "pane.focus.refused",
        }
    }

    /// What Hide does with the value it kept, said the way the operator would
    /// describe it: a tab is on screen, a pane has the keyboard.
    fn kept_phrase(self) -> &'static str {
        match self {
            Self::Tab => "Hide keeps showing it",
            Self::Pane => "Hide keeps it focused",
        }
    }
}

fn remote_tab_source_id<'a>(target_id: &str, projected_id: &'a str) -> Option<&'a str> {
    projected_id
        .strip_prefix(&format!("remote:{target_id}:tab:"))
        .filter(|tab_id| !tab_id.trim().is_empty())
}

/// The prefix every pane id belonging to one remote target carries. It is
/// both the terminal-pane ownership test and the read record ledger's scope
/// for that target, so the two cannot disagree about which panes are its own.
fn remote_pane_id_prefix(target_id: &str) -> String {
    format!("remote:{target_id}:pane:")
}

/// Whether a pane id is one the core scoped to a remote target
/// (`operations::remote_pane_id`), whichever target it was; a Herdr pane id on
/// this machine never takes that form.
fn is_remote_scoped_pane_id(pane_id: &str) -> bool {
    pane_id
        .strip_prefix("remote:")
        .is_some_and(|rest| rest.contains(":pane:"))
}

/// Copies each pane's status word and close-confirmation answer from the agent
/// rows that carry the read axis.
///
/// A pane tree is projected separately from the agent rows, by a pass that
/// cannot see the read record ledger and does not know which pane is focused.
/// Left alone it publishes `Done` and demands a close confirmation for every
/// pane the operator has already read. One owner decides the answer; every
/// tree copies it, local and remote alike, because a pane is a pane.
fn sync_pane_status(
    workspaces: &mut [WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
    focused: Option<&str>,
    machine: crate::agent_sleep::SleepMachine,
) -> bool {
    let by_pane = agents
        .iter()
        .map(|agent| {
            (
                agent.pane_id.as_str(),
                (
                    agent.status_code,
                    agent.requires_close_confirmation,
                    agent.requires_close_status_check,
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut changed = false;
    for pane in workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        let Some((status_code, requires_close_confirmation, requires_close_status_check)) =
            by_pane.get(pane.id.as_str()).copied()
        else {
            continue;
        };
        if pane.status_code != status_code {
            pane.status_code = status_code;
            changed = true;
        }
        if pane.requires_close_confirmation != requires_close_confirmation {
            pane.requires_close_confirmation = requires_close_confirmation;
            changed = true;
        }
        if pane.requires_close_status_check != requires_close_status_check {
            pane.requires_close_status_check = requires_close_status_check;
            changed = true;
        }
    }
    // The tab uses the very row whose read, lineage and sleep axes were just
    // derived. A second status derivation here would diverge from the sidebar.
    let agent_by_pane = agents
        .iter()
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect::<BTreeMap<_, _>>();
    for tab in workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.checkouts)
        .flat_map(|checkout| &mut checkout.tabs)
    {
        if let Some(focused) = focused.filter(|id| tab.panes.iter().any(|pane| pane.id == *id)) {
            changed |= tab.naming.focused_pane_id != focused;
            tab.naming.focused_pane_id = focused.to_owned();
        }
        let agent = agent_by_pane
            .get(tab.naming.focused_pane_id.as_str())
            .copied();
        if !tab.naming.focused_pane_id.is_empty() {
            tab.naming.automatic = crate::model::display_tab_label(
                "",
                tab.naming.number,
                agent.map(|agent| agent.identity_label.as_str()),
                tab.naming
                    .processes
                    .get(&tab.naming.focused_pane_id)
                    .map(String::as_str),
            );
            let label = crate::model::display_tab_label(
                &tab.naming.raw,
                tab.naming.number,
                Some(&tab.naming.automatic),
                None,
            );
            changed |= tab.label.as_deref() != Some(&label);
            tab.label = Some(label);
        }
        let representative = agent.map(crate::model::TabAgentSnapshot::from);
        if tab.agent != representative {
            tab.agent = representative;
            changed = true;
        }
    }
    for checkout in workspaces
        .iter_mut()
        .flat_map(|workspace| &mut workspace.checkouts)
    {
        rename::refresh_strip_labels(checkout);
    }
    // What the pane body and its menu say about sleep follows the same row,
    // and a pane whose agent left takes neither with it.
    let sleep_by_pane = agents
        .iter()
        .map(|agent| {
            (
                agent.pane_id.as_str(),
                (
                    agent.sleep.clone(),
                    crate::agent_sleep::sleep_action(agent, machine),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for pane in workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
        .flat_map(|checkout| checkout.tabs.iter_mut())
        .flat_map(|tab| tab.panes.iter_mut())
    {
        let (sleep, sleep_action) = sleep_by_pane
            .get(pane.id.as_str())
            .cloned()
            .unwrap_or((None, None));
        if pane.sleep != sleep {
            pane.sleep = sleep;
            changed = true;
        }
        if pane.sleep_action != sleep_action {
            pane.sleep_action = sleep_action;
            changed = true;
        }
    }
    changed |= crate::agent_state::sync_checkout_agent_summaries(workspaces, agents);
    changed |= sync_strip_agent_identity(workspaces, agents);
    changed |= crate::project_context::sort_projects(workspaces, agents);
    changed
}

/// Titles each Herdr strip entry after the one agent its tab holds.
///
/// Runs on the same passes as the pane status, so the entry's mark and
/// emphasis follow the read axis, and again whenever a strip is rebuilt, so
/// a fresh entry never reaches the shell without its identity (PRD D-16).
fn sync_strip_agent_identity(
    workspaces: &mut [WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
) -> bool {
    let mut changed = false;
    for checkout in workspaces
        .iter_mut()
        .flat_map(|workspace| workspace.checkouts.iter_mut())
    {
        for entry in checkout
            .strip
            .iter_mut()
            .filter(|entry| entry.kind == crate::model::StripTabKind::Herdr)
        {
            let identity = checkout
                .tabs
                .iter()
                .find(|tab| tab.id.as_deref() == Some(entry.source_id.as_str()))
                .and_then(|tab| {
                    let mut held = agents
                        .iter()
                        .filter(|agent| tab.panes.iter().any(|pane| pane.id == agent.pane_id));
                    let first = held.next()?;
                    held.next()
                        .is_none()
                        .then(|| crate::sidebar::agent_chip(first))
                });
            if entry.agent_identity != identity {
                entry.agent_identity = identity;
                changed = true;
            }
        }
    }
    changed
}

/// Drops the text scale of a pane the server no longer reports, on the same
/// pass that drops its read record, so neither map grows forever. Returns
/// whether anything was dropped.
///
/// It runs on every pass that carries a fresh agent list rather than only when
/// a read record moved: an operator who has looked at nothing moves no record,
/// and gating on that left dead panes' zoom in the store forever.
///
/// A pass only drops keys in the namespace it owns, for the same reason read
/// record eviction does: a local sync holds no remote pane list, so unscoped it
/// deleted every remote pane's zoom the moment any local agent changed state.
fn prune_pane_text_scales(
    scales: &mut BTreeMap<String, f32>,
    workspaces: &[WorkspaceSnapshot],
    agents: &[SidebarAgentSnapshot],
    scope: ReadRecordScope<'_>,
) -> bool {
    let live = workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .map(|pane| pane.id.as_str())
        .chain(agents.iter().map(|agent| agent.pane_id.as_str()))
        .collect::<HashSet<_>>();
    let before = scales.len();
    scales.retain(|pane_id, _| !scope.owns(pane_id) || live.contains(pane_id.as_str()));
    scales.len() != before
}

fn remote_pane_source_id<'a>(target_id: &str, projected_id: &'a str) -> Option<&'a str> {
    projected_id
        .strip_prefix(&remote_pane_id_prefix(target_id))
        .filter(|pane_id| !pane_id.trim().is_empty())
}

/// Every pane `session` lists, and the panes on screen: while its machine
/// is in front, exactly the tabs its selected group shows. Without a core
/// layout, the legacy Herdr-focused tab supplies the initial canvas.
fn remote_terminal_pane_sets(
    session: &RemoteSessionSnapshot,
    target_is_active: bool,
    shown_tabs: Option<&[String]>,
) -> (HashSet<String>, HashSet<String>) {
    let live_pane_ids = session
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .map(|pane| pane.id.clone())
        .collect::<HashSet<_>>();
    if !target_is_active {
        return (live_pane_ids, HashSet::new());
    }
    let active_tab_id = session.focused_tab_id.as_deref().or_else(|| {
        session
            .focused_checkout_id
            .as_deref()
            .and_then(|checkout_id| session.active_tab_ids.get(checkout_id))
            .map(String::as_str)
    });
    let active_pane_ids = session
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .filter(|tab| {
            tab.id.as_deref().is_some_and(|id| {
                shown_tabs.map_or(Some(id) == active_tab_id, |shown| {
                    shown.iter().any(|tab| tab == id)
                })
            })
        })
        .flat_map(|tab| tab.panes.iter())
        .map(|pane| pane.id.clone())
        .collect();
    (live_pane_ids, active_pane_ids)
}

fn remote_tab_creation_key(
    target_id: &str,
    action: &RemoteControlAction,
) -> Option<(String, String, String, String)> {
    match action {
        RemoteControlAction::CreateTab {
            workspace_id,
            cwd,
            label,
            ..
        } => Some((
            target_id.to_owned(),
            workspace_id.clone(),
            cwd.clone(),
            label.clone(),
        )),
        // An owner being opened is keyed by its folder.
        RemoteControlAction::OpenOwner {
            owner, cwd, label, ..
        } => Some((
            target_id.to_owned(),
            owner.path().to_owned(),
            cwd.clone(),
            label.clone(),
        )),
        _ => None,
    }
}

/// A file tab that has been read but not yet put on screen.

#[derive(Clone, Debug, Eq, PartialEq)]
struct PurposeOperationTarget {
    id: u64,
    checkout_id: String,
    remote_target_id: Option<String>,
}

/// The first prompt and CLI arguments of the agent one task starts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TaskAgentLaunch {
    pub(crate) task: u64,
    pub(crate) prompt: Option<String>,
    pub(crate) args: Vec<String>,
    /// The model and effort among `args`, which the started pane keeps for
    /// the agent's restarts.
    pub(crate) launch: hide_agent_adapter::LaunchChoice,
    pub(crate) resume_reference: Option<crate::sidebar::SessionAgentSessionPayload>,
}

/// One repository of a node's GitHub read: the generation it was last asked
/// with, when, and the linked issues that ask named, so a new link asks again
/// at once rather than at the next re-read.
#[derive(Clone, Debug)]
pub(crate) struct DeviceGithubRead {
    generation: u64,
    asked_at: std::time::Instant,
    links: Vec<crate::issues::IssueReference>,
}

pub struct Runtime {
    snapshot: Snapshot,
    delivery_ledger: Result<Arc<crate::delivery::ledger::Ledger>, String>,
    /// What the ledger says Hide itself started, derived again only when a
    /// ledger change moves it (PRD hide-ai-origin-mark D-01).
    hide_started: crate::coordination::started::HideStarted,
    delivery_client: Option<crate::delivery::worker::Client>,
    delivery_observations: HashMap<String, delivery::Observation>,
    /// View openers waiting for their document, Sends under way, and the
    /// feedback worker (PRD view-opener-feedback).
    view_feedback: view_feedback::FeedbackRuntime,
    /// The panes held as an unsent draft, kept across a restart so a daemon
    /// that starts over does not type a bell over what the operator typed
    /// (`composer_store.rs`); absent in a runtime with no state folder.
    composer_marks: Vec<crate::composer_store::Mark>,
    /// When each observed pane's input was last traced (`delivery.input`).
    delivery_input_traced: HashMap<String, u64>,
    composer_store: Option<crate::composer_store::ComposerStore>,
    /// Open Factories' code-owned recipients (`factory:<id>`), set by the
    /// Factory host; a letter to any other `factory:` name has no recipient.
    /// Each open Factory's id with its stall window (D-30, B24).
    factory_recipients: std::collections::BTreeMap<String, u64>,
    /// The panes the open Factories' workers run in, as the Factory host last
    /// published them: a sleep that closes a pane keeps whose conversation it
    /// holds (#857).
    factory_panes: HashMap<String, factory::FactoryPane>,
    /// The Factory screens' way to the engine thread; `None` when the host
    /// did not start.
    factory_screen: Option<crate::factory::ScreenPort>,
    delivery_overflow: HashSet<String>,
    delivery_connected: HashSet<String>,
    /// Each host's Herdr panes as its in-sync replica last published them.
    delivery_panes: HashMap<String, crate::coordination::PaneRead>,
    /// Live registrations whose panes Herdr no longer has, for the delivery
    /// store to end; an entry leaves once the published ledger shows it
    /// ended.
    registrations_gone: std::collections::BTreeMap<String, crate::coordination::PaneGone>,
    /// Registrations a sleeping session's wake continues, for the delivery
    /// store to point at the woken pane; an entry leaves once the published
    /// ledger shows the registration moved.
    registration_rebinds: std::collections::BTreeMap<String, crate::coordination::Rebind>,
    /// Archive deletes whose record lines the link worker is removing.
    archive_deletes_sent: HashSet<crate::session_archive::ArchiveId>,
    /// The archived sessions last handed to the link worker's retention.
    archive_retention_sent: Arc<Vec<crate::links::store::ArchivedSession>>,
    /// When each resumed archived member's registration began waiting for
    /// the delivery store to see its new pane, by agent id.
    archive_registration_waits: HashMap<String, u64>,
    /// The machine this core runs on. Device labels are mutable
    /// presentation, while lineage and every key naming this machine use
    /// this operating-system id.
    node: crate::node::NodeId,
    /// What this machine calls itself (`CoreOptions::machine_name`), on its
    /// device row.
    machine_name: Option<String>,
    device_machine_ids: HashMap<String, String>,
    unresolved_machine_lineage: HashSet<String>,
    file_roots: Option<crate::files::FileRoots>,
    state_path: PathBuf,
    pending_session_resolutions: BTreeMap<String, crate::agent_state::sessions::Resolution>,
    failed_session_resolutions: BTreeMap<String, crate::agent_state::sessions::Resolution>,
    delivery_holds: BTreeMap<String, crate::delivery::doorbell::Hold>,
    /// The panes whose letters the operator is shown waiting on a possible
    /// draft, with how many (`doorbell::letters_waiting`).
    letters_waiting: BTreeMap<String, u32>,
    /// When the oldest Resolved row leaves the 24-hour window.
    session_window_deadline_unix_ms: u64,
    #[cfg(test)]
    pane_header_derivations: usize,
    state_save_pending: bool,
    state_save_active: bool,
    state_save_worker: Option<thread::JoinHandle<()>>,
    project_settings_pending: BTreeMap<String, project_settings::Pending>,
    guardrails: guardrails::GuardrailsWork,
    /// This machine's primary language as an interface language, read once
    /// at launch (`output_language`); `None` when it names none Hide has.
    system_language: Option<crate::model::InterfaceLanguage>,
    /// Where the SSH config that names each registered device's host lives;
    /// `None` when the process has no usable HOME, which every connect then
    /// reports rather than guessing a path.
    home_path: Option<PathBuf>,
    /// The SSH client and coordinator of each registered device that is
    /// connected or connecting, keyed by device id. Removing a device drops
    /// its entry; the coordinator handle moves to `retired_remote_syncs`.
    remote_connections: HashMap<String, devices::RemoteDeviceConnection>,
    /// The link each node that dialed this core brought, until its
    /// connection takes it (`inbound`).
    inbound_arrivals: HashMap<String, crate::remote::Arrived>,
    /// Coordinators of removed devices, waiting for the FFI layer to join
    /// them off the runtime lock: a join under the lock would wait for a
    /// worker that is itself waiting for the lock.
    retired_remote_syncs: Vec<session_sync::SessionSyncHandle>,
    /// The last connection test of each device, kept apart from the device
    /// rows because those are rebuilt with every catalog.
    remote_device_tests: HashMap<String, crate::model::DeviceTestSnapshot>,
    /// Each device whose Herdr server its node last found older than the
    /// pin after a kit pass (PRD device-herdr-update B8).
    device_herdr_behind: HashMap<String, crate::model::HerdrBehindSnapshot>,
    /// Each device's helper connection and the consent it runs under.
    device_hosts: HashMap<String, hosts::DeviceHost>,
    /// When each device's lost or failed link is tried again.
    device_host_retries: HashMap<String, hosts::HostRetry>,
    /// The last helper attempt number, for every device. Never reset, so an
    /// answer from an attempt made before a device was removed and added
    /// again under the same id cannot match the new attempt.
    last_host_generation: u64,
    /// The device and folder `snapshot.changes` was read for.
    changes_published_key: Option<crate::changes::ChangesKey>,
    /// Each device's session as its Herdr reported it, before its projects
    /// are grouped from the helper's facts (`device_catalog`).
    device_raw_sessions: HashMap<String, RemoteSessionSnapshot>,
    device_facts: HashMap<String, crate::device_catalog::DeviceFacts>,
    /// Each device checkout's last tab the device's Herdr had in focus, by
    /// device then checkout id: what opening the row brings forward (PRD
    /// checkout-workspace-binding B5, D-13), as `visible_tab_ids` is here.
    /// Pruned to the checkouts the device's session lists.
    device_recent_tabs: HashMap<String, HashMap<String, String>>,
    /// Each device's repositories' worktrees, read through its helper
    /// (`hide_host::worktrees`); the device's rows carry them.
    device_worktrees: HashMap<String, crate::device_catalog::DeviceWorktrees>,
    /// The TCP listeners of each node that dials this core, read through its
    /// link; that device's panes carry them (PRD core-host-node-remote-core
    /// B13). An SSH device's listeners are not read, so its panes have none.
    device_ports: HashMap<String, Vec<crate::model::ListeningPortSnapshot>>,
    /// What each node that dials this core measured of its own projects.
    device_disk: HashMap<String, Vec<crate::model::DiskUsageSnapshot>>,
    /// The pull requests of each node that dials this core, read here by
    /// repository name, and when each of its repositories was last asked.
    device_github: HashMap<String, crate::model::GithubSnapshot>,
    device_github_reads: HashMap<(String, String), DeviceGithubRead>,
    /// The generation of each node repository's last answered read, which a
    /// Memory promotion's verdict waits for (`memory_promotion.rs`).
    device_github_read_generation: HashMap<(String, String), u64>,
    /// How many of each device's projects the pull-request read left out at
    /// its cap, last logged.
    device_github_over_limit: HashMap<String, usize>,
    /// This machine's file host: the helper's dispatch, run in place.
    own_node: Arc<dyn crate::node_access::NodeLink>,
    /// Opens the transport to each registered device; the node that holds
    /// the SSH configuration implements it, so the core never reads `~/.ssh`.
    devices: Arc<dyn DeviceConnector>,
    /// Where each open file tab's saves go.
    document_places: HashMap<String, crate::files::DocumentPlace>,
    /// Each file tab's save in flight, the newest draft waiting behind it,
    /// and a save whose answer was lost.
    document_saves: HashMap<String, documents::SaveSlot>,
    /// Remote reads not yet shown as tabs, by tab id, with the generation
    /// that fences a late answer.
    document_opens: HashMap<String, documents::OpenRequest>,
    next_document_generation: u64,
    host_helper_root: String,
    host_cli_dir: String,
    live: Option<LiveContext>,
    remote_controls: HashMap<String, RemoteControlContext>,
    remote_file_transports: HashMap<String, devices::DeviceReach>,
    remote_control_requests: VecDeque<(String, String)>,
    /// Remote mutations waiting for a transport answer or fresh topology,
    /// keyed by target and request id.
    remote_operations: HashMap<(String, String), PendingRemoteOperation>,
    /// Advances when a remote target's session connects or drops, so a late
    /// answer from an older connection cannot settle a newer request.
    remote_connection_generations: HashMap<String, u64>,
    remote_tab_creations_in_flight: HashSet<(String, String, String, String)>,
    /// Every node's terminals (PRD core-host-node-terminal): the core tells
    /// them which panes attach and hears their state back; keys and output
    /// never pass here.
    terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes>,
    /// Each pane's attach state as its node last reported it, which the
    /// terminal projection shows.
    terminal_states: HashMap<String, hide_node_link::terminal::PaneTerminalState>,
    /// Panes a node was asked to attach and not yet told to forget, whether
    /// or not its first report has arrived: a pane retired before then is
    /// forgotten at its node too.
    terminal_attach_requested: HashSet<String>,
    /// What the nodes were last told is on screen, asleep and closing, so
    /// only a change is sent.
    terminal_intents: terminal::TerminalIntents,
    /// Which panes take no input while their agent ends, sleeps or wakes,
    /// asked at the write by the writers that bypass a node's terminal.
    input_gate: Arc<crate::input_gate::PaneInputGate>,
    next_remote_file_generation: u64,
    attachment: Option<attachments::PendingAttachment>,
    attachment_rejection: Option<crate::model::AsyncOperationSnapshot>,
    attachment_worker: Option<thread::JoinHandle<()>>,
    terminal_sizes: HashMap<String, (u16, u16)>,
    /// Wheel lines that arrived before the pane had a session to take them
    /// (no size reported yet, or its attach still starting after a tab
    /// switch), summed per pane and sent at the pane's first frame through
    /// whichever mode it attached in.
    wheel_before_attach: HashMap<String, i32>,
    /// Observed panes with a `pane.scroll` request in flight, and the lines
    /// that arrived meanwhile, which go out as one request when it lands.
    viewport_scrolls: HashMap<String, i32>,
    /// Observed panes whose last wheel Herdr did not move: another client
    /// holds their scrolling (`TerminalPaneSnapshot::scroll_held_elsewhere`).
    panes_scroll_held: HashSet<String>,
    /// The tabs that have been on screen, most recent first. An attach lives
    /// for as long as its tab is in this window; every other pane's session is
    /// released. Herdr renders a pane for every attached client, so an attach
    /// nobody is looking at costs a child process here and a render there for
    /// the life of the process.
    /// Delegated child panes Hide has asked Herdr to move out of their
    /// parent's tab, and when it asked. A move that fails is retried quietly
    /// on a later ingest; it never reaches the pane header, because the
    /// operator did not ask for the move and cannot act on its failure
    /// (PRD B2, D-45).
    pane_relocations_in_flight: BTreeMap<String, u64>,
    /// What Hide's hook last reported for each pane, read out of the pane
    /// tokens the session snapshot already carries. Kept between ingests so
    /// the projection does not have to hold the whole payload alive.
    pane_hook_tokens: BTreeMap<String, crate::agent_hooks::PaneHookTokens>,
    /// The hook-install state of each runtime, read off the coordinator
    /// thread. `None` until that first read lands, which reads as "not known
    /// yet" rather than as "not installed".
    hook_diagnosis: Option<hide_agent_hooks::Diagnosis>,
    /// Each machine's install kit as its last check found it, keyed by
    /// device id (the core's own node id for this Mac); `runtime/kit.rs`
    /// owns it.
    kit_states: BTreeMap<String, crate::model::KitSnapshot>,
    /// Once per unknown id/device during this runtime, with a hard bound.
    unknown_kit_agents: BTreeSet<(String, String)>,
    unknown_kit_agents_limit_reported: bool,
    /// The install this Mac's kit worker runs next, merged across requests.
    local_kit_pending: Option<hide_kit::Scope>,
    /// The wizard's first step waits for the kit pass its Continue queued
    /// (`runtime/first_run.rs`); session-only, since a relaunch reopens the
    /// step and its next Continue asks again.
    first_run_apply: Option<first_run::FirstRunApply>,
    /// When this Mac's kit is next re-read while Settings is on screen.
    local_kit_next_status: Option<Instant>,
    /// A Settings tab asked for one read of this Mac's kit.
    local_kit_check_requested: bool,
    /// The kit call each device's helper runs next, merged across requests,
    /// keyed by device id.
    device_kit_pending: BTreeMap<String, KitJob>,
    /// The devices whose last kit report said they wait for the first-run
    /// agent choice, and whether this run already sent it: a device is sent
    /// the choice once, so a device that keeps waiting (its record would not
    /// save) is reported once instead of asked on every report.
    device_first_run_choice: BTreeMap<String, kit::FirstRunChoice>,
    /// Devices whose kit worker is running; at most one per device.
    device_kit_running: BTreeSet<String>,
    /// The numbered run of each device's shared-server turn-off its worker
    /// took: only the answer for the run still current settles the request,
    /// so a late answer from a connection that ended changes nothing (PRD
    /// codex-daemon-apply B11).
    codex_daemon_off_running: BTreeMap<String, u64>,
    /// The last turn-off run handed out.
    codex_daemon_off_runs: u64,
    /// The removal each removed device's kit worker runs next, holding the
    /// helper connection it runs on.
    device_kit_removals: BTreeMap<String, DeviceKitCall>,
    /// Removed devices whose kit is still coming off; a new connection to
    /// one waits for it.
    device_kit_removing: BTreeSet<String>,
    /// The operator's background AI choice as the core holds it. `None` until
    /// the coordinator's first read lands, which reads as "not known yet"
    /// rather than as "the defaults".
    ai_settings: Option<hide_ai::AiSettings>,
    /// A choice waiting for the coordinator to write it to the settings file
    /// off the mutex, the same way an approved hook install waits.
    pending_ai_settings_save: Option<hide_ai::AiSettings>,
    /// True while the Background AI group is on screen, which is the only
    /// time the provider probe runs.
    ai_observing: bool,
    /// A start surface shows its model menu; reads the catalog only.
    ai_start_observing: bool,
    /// What the providers last answered. Held beside the snapshot so a
    /// changed choice can restamp the rows without asking again.
    background_ai_providers: Vec<crate::model::BackgroundAiProviderSnapshot>,
    usage_window_visible: bool,
    usage_ledger_client: Option<crate::usage_ledger::worker::Client>,
    usage_popover_open: bool,
    usage_popover_open_generation: u64,
    /// The usage limit and the weekly figures the label and Memory routers
    /// judge each request by; written from the settings and the usage rows
    /// (`runtime/usage_limit.rs`).
    usage_limits: Arc<hide_ai::UsageLimits>,
    /// Where every router tells what its attempts cost; the usage ledger
    /// worker drains it (`runtime/usage_ledger.rs`).
    ai_usage: Arc<crate::usage_ledger::ai_usage::AiUsageQueue>,
    /// Whether the diagnostic log last said the order is paused by the limit.
    usage_pause_announced: bool,
    recent_visible_tabs: Vec<String>,
    pending_tab_rename: Option<rename::PendingRename>,
    /// Panes Hide has asked Herdr to close. Herdr closes the PTY first, so the
    /// attach child ends before the `pane_closed` event arrives and the pane
    /// is still on screen when its transport reports the close. Projecting
    /// that as a failure is what put "terminal attach ended" on screen for one
    /// frame every time the operator closed a pane.
    panes_closing: HashSet<String>,
    /// Closed panes the Factory engine has not taken yet: a worker there
    /// reads as closing, never as gone, until the engine has paused its Task
    /// (D-26). The engine takes them on its wake-up and before every tick.
    factory_closes_sent: HashSet<String>,
    /// Closes of an agent together with its descendants, deepest first
    /// (`tree_close.rs`). At most `TREE_CLOSE_ACTIVE_LIMIT` at once.
    tree_closes: Vec<tree_close::TreeClose>,
    next_tree_close_id: u64,
    /// User-initiated local closes, newest last. Memory only by contract.
    recent_closed: VecDeque<ClosedItem>,
    /// User closes in request order. A close is promoted onto
    /// `recent_closed` only from the front of this queue and only once its
    /// topology is confirmed, so the confirmed stack keeps the order the
    /// operator closed things in (PRD D-11).
    close_capture_order: VecDeque<String>,
    close_operations: HashMap<String, PendingClose>,
    close_status_checks_in_flight: HashSet<String>,
    /// One bounded read-only sleeping-session inspection, independent of
    /// durable transition admission. A generation fences its late reply.
    dormant_status_check: Option<(crate::agent_sleep::SleepId, u64)>,
    /// The local checkout paths the project list showed when the rows of
    /// owner-less sleeping sessions were last derived, as an order-free
    /// fingerprint: those rows follow the list only when it changes.
    unowned_dormant_listing: Option<u64>,
    pane_operations: HashMap<String, PendingPaneOperation>,
    /// The last complete raw layout Herdr confirmed for each tab. Mutation
    /// operations compare fresh geometry with this value, because a model
    /// layout has pane ids and split ratios but no PTY rectangle to confirm a
    /// resize against.
    confirmed_pane_layout_signatures: HashMap<String, PaneTopologySignature>,
    /// Stage times of each Hide-started tab creation and pane operation
    /// (PRD instant-pane-topology D-14).
    op_timings: op_timing::OpTimings,
    /// The creation requests keys are sent against before Herdr names their
    /// pane (PRD instant-pane-topology D-11); the node holds the keys.
    input_requests: terminal_input::InputRequests,
    /// A pane close that waited in its tab's line, by close key: the place
    /// it held there, which orders its drawing among the line's other
    /// predictions. Pruned to the live closes on each insert.
    close_line_places: HashMap<String, (u64, u64)>,
    /// A session update is being applied: the navigator still shows the one
    /// it replaces, so a close waiting in a tab's line starts only after it.
    ingesting_session: bool,
    /// A tab's line is being pumped; see `pump_geometry_queue`.
    pumping_geometry: bool,
    /// Herdr's confirmed layout of each tab with a drawn prediction, as the
    /// last session showed it; a new request is predicted on it rather than
    /// on the drawing.
    confirmed_layouts: HashMap<String, crate::model::PaneLayoutSnapshot>,
    /// Per tab, the closes drawn on the canvas ahead of Herdr; set with every
    /// session overlay and read by the layout writers.
    drawn_closes: HashMap<String, Vec<pane_prediction::Prediction>>,
    /// Created tabs drawn until Herdr's layout for them arrives (D-05).
    provisional_tabs: Vec<drawn_ahead::ProvisionalTab>,
    recent_closed_sequence: u64,
    reopen_in_flight: Option<String>,
    /// A reopen asked for while this machine's newest close was still
    /// settling: it runs once that close completes, and is dropped by any
    /// other outcome or by a newer close.
    reopen_after_close: Option<String>,
    /// Advances on every `set_live`, so a worker started against an earlier
    /// local Herdr connection cannot settle an operation on the current one.
    live_generation: u64,
    /// A shell-requested read-only `agent.list` refresh, drained by the
    /// coordinator on its next pass.
    status_refresh_requested: bool,
    /// The checkout each tab Hide created was created for, keyed by the tab
    /// id Herdr acknowledged; it decides the tab's checkout instead of the
    /// pane cwd (`session.rs`, `CreatedTabCheckout`).
    created_tab_checkouts: BTreeMap<String, session::CreatedTabCheckout>,
    /// The same record for tabs created on a device, by target and the tab id
    /// that device's Herdr gave it.
    created_device_tabs: BTreeMap<(String, String), session::CreatedTabCheckout>,
    /// Wakes the local coordinator to read the session again now, when the
    /// core draws something ahead of Herdr or takes it back (PRD
    /// instant-pane-topology D-05, D-07). None until the coordinator starts,
    /// and in tests, which ingest the session themselves.
    republish_waker: Option<session_sync::RepublishWaker>,
    /// A republish was asked for since a test last looked.
    #[cfg(test)]
    republish_requested: bool,
    /// The clock the birth window is measured on: monotonic, so a wall-clock
    /// step cannot end it early, and replaceable so a test moves it rather
    /// than waiting (`session.rs`, `BirthClock`).
    birth_clock: session::BirthClock,
    next_async_operation_id: u64,
    workspace_creations_in_flight: HashSet<String>,
    /// Each open file tab's document, edited under a number so a snapshot
    /// read re-sends a View document only when it moved.
    editor_documents: HashMap<String, Edited<EditorDocumentSnapshot>>,
    archive_documents: HashMap<String, ArchiveDetailSnapshot>,
    session_catalog_rows: Vec<SessionRowSnapshot>,
    memory_sessions_load_in_flight: bool,
    memory_sessions_load_pending: bool,
    memory_sessions_load_generation: u64,
    archive_detail_load_generation: u64,
    /// What the link worker was last handed, and its panel reads.
    links_work: links::LinksWork,
    backlog_work: backlog::BacklogWork,
    /// The minute the PRs tool's list was last judged against the clock
    /// (`runtime/request_view.rs`, `tick_work_clock`).
    work_clock_minute: u64,
    /// Project Memory's background pass, receipts and analysis state
    /// (`runtime/memory_analysis.rs`).
    memory_work: memory_analysis::MemoryWork,
    /// The named Project's Memory reads and action (`runtime/project_memory.rs`).
    project_memory: project_memory::ProjectMemoryWork,
    editor_tab_history: Vec<String>,
    worker_context: Option<RuntimeWorkerContext>,
    /// The last moment any agent was working or waiting on the user. The pet
    /// measures idleness from here, so roam and sleep are driven by real
    /// session activity rather than wall-clock uptime.
    pet_active_at_unix_ms: u64,
    /// When an agent last worked or waited on the operator; 0 until one has
    /// (the pet's own clock starts at launch, this one must not).
    agent_worked_at_unix_ms: u64,
    pet_waking_until_unix_ms: u64,
    pet_dragging: bool,
    /// The pane the operator chose to look at, and the only pane whose read
    /// record may be raised.
    ///
    /// Herdr's focus is not the operator's attention. A tab carries the pane
    /// it last had focused, so bringing a tab forward makes Herdr report a
    /// focus nobody asked for; a spawned pane takes focus on its own; and a
    /// relaunch inherits whatever focus the arriving session snapshot names.
    /// Counting any of those as a look cleared rows the operator never saw
    /// (one click on a Done row cleared two rows, and a restart cleared the
    /// last unread one). Only a focus Hide itself dispatched on the
    /// operator's behalf arms this, and it is memory only: a launch starts
    /// with the operator having looked at nothing.
    operator_focused_pane_id: Option<String>,
    /// The read axis's view of the current connection to the local Herdr
    /// server and to each device's: the mark that names the server and the
    /// panes settled on it so far (`agent_state::ReadConnection`). Memory
    /// only; every connection starts a new one.
    local_read_connection: crate::agent_state::ReadConnection,
    /// One per device target that has connected, so at most one per
    /// registered device.
    remote_read_connections: BTreeMap<String, crate::agent_state::ReadConnection>,
    /// Numbers the marks this process makes, so two connections made in the
    /// same millisecond still get different marks.
    read_marks_made: u64,
    /// The tab Hide is showing in each checkout it has been asked about.
    ///
    /// Hide owns the visible tab. The navigator is rebuilt from Herdr's
    /// session on every update, so the choice has to live outside it or every
    /// tick would hand the decision back to Herdr.
    visible_tab_ids: BTreeMap<String, String>,
    /// The tab moves that have left for Herdr and not been answered, in the
    /// order they left: tab focuses, the tab moves of pane focuses, and tabs
    /// created with focus. Which Herdr move is Hide's own answer, and when a
    /// move Herdr makes is followed, are read from it (`tab_focus.rs`).
    tab_focus_requests: Vec<tab_focus::TabFocusRequest>,
    /// How many of the replica's tab moves have been read, and of which
    /// replica, so the next session says which moves are new.
    herdr_tab_moves_read: Option<(u64, u64)>,
    /// The tabs active in their Herdr workspaces at the last replica session.
    /// When moves were dropped, the difference from it stands in for them.
    herdr_active_tabs_seen: BTreeSet<String>,
    /// The tab Herdr had focused at the last session update. A follow needs
    /// Herdr's focus to have moved; a focused tab that merely differs from
    /// Hide's, as it does after a notification Herdr never answered, is not
    /// an operator action and is not followed.
    herdr_focused_tab_seen: Option<String>,
    herdr_tab_focus_seen: Option<crate::sidebar::SessionTabFocus>,
    /// The pane focus Hide has told Herdr about and is still waiting to see
    /// confirmed.
    pending_pane_focus: Option<PendingViewFocus>,
    pane_focus_in_flight: Option<PendingPaneFocusControl>,
    /// The one read of Herdr's own focus that decides whether an external
    /// move in the session stream is followed. It runs off the lock while
    /// the session itself is already ingested, so a slow Herdr delays only
    /// the focus, never the topology that arrived with it.
    pane_focus_readback: Option<PendingPaneFocusControl>,
    /// A session update called for a readback while the one slot was taken.
    /// The outstanding answer may have been read before that update, so an
    /// unconfirmed answer is asked again rather than losing the move.
    pane_focus_readback_again: bool,
    next_pane_focus_serial: u64,
    /// The one ordered lane the local tab and pane focus controls travel on.
    control_lane: ControlLane,
    /// The lane serial of the newest focus the operator asked for. A tab Hide
    /// creates takes the screen only when no focus was asked for after it.
    last_view_intent_serial: u64,
    /// The tabs Herdr most recently reported active in their own workspaces.
    /// A pane layout remembers a focused pane even while its tab is hidden,
    /// so the layout alone cannot confirm a pane-focus request.
    herdr_active_tab_ids: BTreeSet<String>,
    /// First moment each currently-unseen pane became unseen. Memory only by
    /// decision (D-21): after a restart snapshot order decides instead.
    pet_unseen_observed: std::collections::BTreeMap<String, u64>,
    /// True until the first live session snapshot has been ingested. The pane
    /// and checkout ids loaded from disk describe a session that ended, so
    /// they are a restore hint rather than a user selection: the first
    /// snapshot that disagrees with them retargets silently. A selection the
    /// user makes against a live session is authoritative, and its
    /// disappearance stays a reported error.
    restore_hint_pending: bool,
    /// The Herdr workspaces the last session snapshot reported. A catalog
    /// rebuild triggered by a registration change is not a session update, so
    /// it reuses these rather than briefly emptying the navigator.
    last_session_spaces: Vec<workspace::SessionSpace>,
    /// Whether a window draws the snapshot (`ui_attached`); a core no daemon
    /// told otherwise is drawn.
    ui_attached: bool,
    /// The last labels the local session-sync coordinator published, laid on
    /// every local projection this runtime ingests, whichever path brings it.
    label_overlay: crate::labels::overlay::LabelOverlay,
    /// The label store and analyzer every session-sync coordinator's worker
    /// shares; `None` when the core was made without them (tests).
    label_services: Option<std::sync::Arc<crate::labels::LabelServices>>,
    /// GitHub's creation time of every pull request read, which the label
    /// workers judge their sessions' sightings against (PRD
    /// overview-request-view D-31); replaced only when it changes.
    pull_request_times: std::sync::Arc<crate::labels::facts::PullRequestTimes>,
    /// Some window shows the request view, and when its pending checks were
    /// last looked at (PRD overview-request-view D-32).
    request_view_observed: bool,
    pending_checks_read_at: Option<std::time::Instant>,
    /// What each of this Mac's rows waited on in a pull request's checks
    /// (`checks_wait`).
    checks_memory: checks_wait::ChecksMemory,
    issue_tokens: crate::wire::IssueTokens,
    issue_candidates: BTreeMap<String, crate::issues::IssueCandidate>,
    issue_write_pending: Option<issues::IssueWritePending>,
    unconfirmed_issue_tokens: BTreeMap<String, issues::UnconfirmedIssueToken>,
    /// The same, for each node that dials this core, by its Herdr's
    /// workspace ids; kept while the node's session is.
    unconfirmed_node_issue_tokens:
        BTreeMap<String, BTreeMap<String, issues::UnconfirmedIssueToken>>,
    /// The malformed link value each checkout's chain last logged, so one
    /// that stays malformed is logged once rather than on every pass.
    invalid_links: issues::InvalidLinks,
    /// Every project's Local issues, read when the runtime is created; `Err`
    /// holds why the file could not be read, and then no change is accepted,
    /// so a damaged file is never replaced by a partial one.
    local_issues: Result<crate::local_issues::LocalIssueStore, String>,
    /// Absent keeps Local issues in memory only (`CoreOptions::local_issues_path`).
    local_issues_path: Option<PathBuf>,
    /// Whether this core may act outside its machine yet (`effects.rs`).
    effects: Arc<crate::effects::EffectHold>,
    /// The save thread's flags, as `state_save_pending`/`state_save_active`.
    local_issues_save_pending: bool,
    local_issues_save_active: bool,
    /// Each checkout's Local issue number, for a project that reads Local
    /// issues; `sync_issues` fills it from the same chain a GitHub link uses.
    local_issue_links: BTreeMap<String, u32>,
    /// The one running GitHub search and the one waiting behind it
    /// (`issues::GithubSearchWork`).
    github_search: issues::GithubSearchWork,
    /// The checkout whose Hide link a Local `pr_link_issue` waits on: its
    /// answer in `ingest_issue_operation_result` settles `pr_work.link`.
    pr_link_checkout: Option<String>,
    /// How the agent a task starts is launched: its first prompt and its
    /// CLI arguments (the model, a Home agent's project folders), by task
    /// id. Kept off the snapshot: an issue body or a request has no business
    /// on the wire after the surface that sent it.
    task_agent_launch: Option<TaskAgentLaunch>,
    group_start: Option<group_starts::ActiveStart>,
    group_start_saves: Vec<group_starts::SaveWaiter>,
    /// How the pane a task created says its shell has finished starting,
    /// by task id, for that task's agent start.
    task_pane_shell: Option<(u64, crate::agent_start::ShellStart)>,
    /// Each device's Home link sync: the set last sent and whether it runs.
    home_links: HashMap<String, home::HomeLinkState>,
    /// The catalog and root index most recently accepted from the sync
    /// coordinator, reused when a later precomputation arrives stale so the
    /// reconcile never rebuilds under the runtime lock.
    last_accepted_catalog: Option<Vec<WorkspaceSnapshot>>,
    /// The path facts the accepted catalog was built from.
    catalog_paths: Arc<workspace::PathIndex>,
    /// Whether the sync coordinator has handed this runtime a catalog. From
    /// then on the coordinator is what asks the node, off the lock, and a
    /// session applied without a catalog reads the last paths it brought.
    catalog_from_coordinator: bool,
    /// The strip order each local checkout has, as strip entry ids. It is
    /// memory only by decision: Herdr persists its own tab order and file tabs
    /// do not survive a restart, so there is nothing here worth writing to
    /// disk. Without it a Herdr tab created next to open file tabs would land
    /// in front of them instead of at the end of the strip.
    checkout_tab_order: BTreeMap<String, Vec<String>>,
    /// Every Herdr workspace's whole tab list, in Herdr's order, keyed by the
    /// Herdr workspace id. A checkout holds only the tabs whose panes sit in
    /// its own directory, so one workspace's tabs can be split across a
    /// repository and its worktrees. `tab.move` counts its insertion index in
    /// the workspace's list, not in a checkout's part of it, so the index has
    /// to be read off this list or the tab lands somewhere else.
    herdr_workspace_tab_order: BTreeMap<String, Vec<String>>,
    herdr_worktrees: BTreeMap<String, crate::domain::WorktreeProjection>,
    /// The arrangement a reorder asked for and Herdr has not reported yet,
    /// per checkout. Herdr owns where its own tabs sit, so a drag that moves
    /// one of them is held here rather than written into the strip: an
    /// arrangement written early would show the file slots moved and the
    /// Herdr slots not, and a refusal would have nothing to revert to.
    pending_tab_move: BTreeMap<String, PendingTabMove>,
    /// Numbers each reorder request so a result that a later drag has already
    /// superseded cannot cancel the newer one.
    next_tab_move_generation: u64,
    /// Workspace/active-tab pairs Herdr named that the navigator could not
    /// place, as `<workspace id>/<tab id>`. Session sync reconciles once a
    /// second, so the diagnostic is emitted when the set changes rather than
    /// on every tick.
    unresolved_active_tabs: BTreeSet<String>,
    /// Panes whose fork has been started and not yet answered, by source
    /// pane. `agent.start` blocks until the agent has started, so without this
    /// a second activation during that wait would bill a second session.
    forks_in_flight: HashMap<String, agents::PendingFork>,
    /// The last keyboard move made for a fork (`agents::ForkFocus`).
    fork_focus: Option<agents::ForkFocus>,
    /// When the agent sleep decision may run next; the coordinator ticks every
    /// 250 ms and the decision runs once a minute (PRD agent-sleep).
    agent_sleep_next_decision_unix_ms: u64,
    /// The same for each node that dials this core (PRD
    /// core-host-node-remote-core B13); one entry per registered node.
    node_sleep_next_decision_unix_ms: HashMap<String, u64>,
    /// The tabs each such node's screen showed at its last session, so a
    /// tab its Herdr brings forward is a visit (B12).
    node_sleep_shown_tabs: HashMap<String, Vec<String>>,
    /// Panes whose agent would not end, and until when they are left alone (B8).
    agent_sleep_backoff: HashMap<String, agent_sleep::Backoff>,
    /// When Herdr took each wake's start, for the panes still waking: one
    /// whose agent has not appeared by the start's deadline failed.
    agent_wake_started: HashMap<String, u64>,
    /// Panes with a Reopen running or just refused, until the pane connects,
    /// leaves, or stops showing a connection chip (B29). One entry per pane,
    /// so a second press while one runs starts nothing.
    pane_reopens: HashMap<String, crate::model::PaneReopenSnapshot>,
    fork_sequence: u64,
    /// The machine's TCP listeners, refreshed on their own window by the
    /// session-sync coordinator. Held here rather than in the snapshot because
    /// what the shell renders is the per-pane attribution, not the raw list.
    listening_ports: crate::model::ListeningPortsSnapshot,
    /// Every open repository's worktrees, refreshed by repository and Herdr
    /// topology changes on the session-sync coordinator. Held here rather than
    /// in the snapshot because the shell renders the checkout rows these
    /// produce, not the raw list.
    worktree_catalog: crate::model::WorktreeCatalogSnapshot,
    /// Each repository's worktree paths as the catalog lists them, which a
    /// created tab's clamp reads through the newest path index on every
    /// session (`session.rs`, `CreatedTabClamp`).
    local_worktree_paths: Arc<Vec<Vec<String>>>,
    /// Every open repository's pull requests, from the operator's own `gh`.
    github: crate::model::GithubSnapshot,
    /// Measurements for the focused project's worktrees. The reader updates
    /// this only while Git is visible or after an explicit refresh.
    disk_usage: Vec<crate::model::DiskUsageSnapshot>,
    /// One counter per local git project, keyed by its navigator path. Opening
    /// the Git section or explicitly refreshing GitHub advances the target
    /// project counter; equal generations reuse the cached answer indefinitely.
    github_generations: HashMap<String, u64>,
    worktree_project_generations: HashMap<String, u64>,
    /// Where each local project is in the re-read cycle: five minutes after a
    /// good answer, sooner after a failed read (`reread_stale_github`).
    github_clock: HashMap<String, projects::GithubClock>,
    /// Projects a screen named with `github_request` or `overview_refresh`;
    /// they are read before the others when the count passes the limit.
    github_wanted: HashSet<String>,
    /// Each pull request address a session printed that set off a read, or
    /// waits for an unanswered one (`read_sighted_pull_requests`), so it sets
    /// off one read while it is recent and none after.
    github_sighted: HashMap<(String, u64), projects::Sighted>,
    /// Projects an answer arrived for since the clock last looked, each with
    /// how that read ended; the next wake starts their wait.
    github_answered: HashMap<String, crate::model::GithubReadOutcome>,
    /// The remotes each read project's repository had when the clock last
    /// looked (`Runtime::github_remotes_moved`).
    github_remotes: HashMap<String, Vec<String>>,
    /// The generation each project was last answered for: the reader returns
    /// its cached entry for every project in a request, and only a moved
    /// generation is a read.
    github_read_generation: HashMap<String, u64>,
    /// Local projects whose first read has been answered, with or without a
    /// result; until then a project without an answer is loading.
    github_settled: HashSet<String>,
    /// Pull requests whose Merge Hide just saw GitHub accept, as (project
    /// root, number) with the time it was seen: the list reads them as merged at once, since GitHub's
    /// own list can still answer open (merge state unknown) for a moment
    /// after its own merge. An entry leaves when a read shows the pull
    /// request merged or closed, or no longer lists it.
    merged_here: HashMap<(String, u32), u64>,
    /// The number of local Git projects past `GITHUB_PROJECT_LIMIT` last
    /// stated, so a count is logged once rather than on every wake.
    github_over_limit: usize,
    /// Projects the advisory poll saw change and the full read has not been
    /// asked for yet (`Runtime::ask_polled_changes`); at most one per read
    /// project.
    github_polled: BTreeSet<String>,
    /// The count past `POLL_PROJECT_LIMIT` last stated, so it is logged once
    /// when it changes rather than on every wake.
    github_poll_over_limit: usize,
    /// Where the last answer is kept across a restart; absent in a runtime
    /// that has no state folder to keep it in.
    github_store: Option<crate::github_store::GithubStore>,
    /// Bumped when visible Git rows must be measured again: section opening,
    /// explicit refresh, and opening the delete confirmation.
    disk_generation: u64,
    /// The Projects tab's disk sweep, per machine (`disk_sweep.rs`).
    disk_sweeps: disk_sweep::Sweeps,
    cleanup: Option<live::cleanup::CleanupSnapshot>,
    /// The node whose session last carried `cleanup`, so a cleanup that
    /// replaces it on another project also leaves that node's rows.
    cleanup_published_node: Option<String>,
    next_cleanup_id: u64,
    /// The id of the cleanup worker that is running, if one is. While it is,
    /// no other review starts and the snapshot cannot be dismissed, so two
    /// workers never overlap however the sheet is opened and closed.
    cleanup_worker: Option<u64>,
    /// Bumped when the worktree list itself is known to have changed through a
    /// manual refresh, a refused removal, or an observed Herdr worktree event.
    worktree_generation: u64,
    /// Local worktree removals settled so far. A catalog read carries the
    /// count it started under, so the core can tell a read that began before
    /// a removal from one that began after it.
    worktree_removals: u64,
    /// Checkouts a settled removal dropped from the catalog, with the count
    /// that dropped them. A read that started earlier still lists them; a
    /// read that started later retires the entry.
    removed_worktrees: Vec<(u64, String)>,
    /// Identifies one delete handshake across core, Herdr and the shell.
    /// A repeated callback for an older request cannot authorize a newer one.
    next_worktree_removal_id: u64,
    /// Registered projects whose panes a `Remove project…` is closing on a
    /// worker thread. The registration is only removed once the worker
    /// reports Herdr's confirmation, so a repeat for the same project while
    /// that runs is a quiet no-op rather than a second round of closes.
    workspace_removals_in_flight: HashSet<String>,
    next_task_operation_id: u64,
    next_repository_clone_id: u64,
    /// The clone `snapshot.repository_clone` reports, while its worker runs.
    repository_clone_job: Option<clone::CloneJob>,
    next_ssh_hosts_id: u64,
    /// The listing `status.ssh_hosts` reports as loading, while its worker runs.
    ssh_hosts_job: Option<ssh_hosts::SshHostsJob>,
    /// The checkout a purpose receipt belongs to. A remote checkout lives in
    /// `status.remote[].session`, not the local navigator, so the operation
    /// carries this target separately from its shell-facing receipt.
    purpose_operation_target: Option<PurposeOperationTarget>,
    /// Creation purpose writes registered after Herdr confirms the checkout
    /// but before the token request starts. This closes the event-ordering
    /// window where the mirror could observe the token before the task worker
    /// reports whether Git accepted it.
    created_purpose_writes_in_flight: HashMap<String, String>,
    /// Creation values whose token write succeeded but whose Git mirror and
    /// compensating token clear both failed. Kept only for this process so the
    /// row shows its fallback until the user tries Set purpose again.
    unconfirmed_created_purposes: HashMap<String, String>,
    next_explorer_operation_id: u64,
    explorer_queue: VecDeque<WaitingExplorerChange>,
    delta: snapshot_delta::DeltaState,
    /// Usage publications have their own wire revision, independent of navigation.
    usage_edit: u64,
    agent_scope_cache: crate::agent_state::ScopeCache,
    /// Each Workspace's presentation; present only in a shell that draws
    /// separate Agent and View areas (`CoreOptions::workspace_views_path`).
    workspace_views: Option<WorkspaceViewStore>,
    workspace_actions: VecDeque<workspace_control::RecordedAction>,
    browser_pages: HashMap<(String, String, String), workspace_control::ReportedBrowserPage>,
    /// The folders a test made for this runtime, its state file's and its
    /// fixtures'. Each is new, so nothing an earlier test process left can be
    /// read, and each goes with the runtime. Declared last so it is dropped
    /// after everything that may hold a file in it.
    #[cfg(test)]
    test_dirs: Vec<tempfile::TempDir>,
}

#[derive(Clone)]
pub(crate) struct RuntimeWorkerContext {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl Runtime {
    pub fn new(
        options: CoreOptions,
        environment: environment::EnvironmentReport,
        own_node: Arc<dyn crate::node_access::NodeLink>,
        devices: Arc<dyn DeviceConnector>,
    ) -> Self {
        let state_path = PathBuf::from(&options.app_state_path);
        let delivery_path =
            hide_kit::layout::delivery_ledger(state_path.parent().unwrap_or(Path::new(".")));
        let delivery_ledger = crate::delivery::ledger::recover(&delivery_path)
            .map(Arc::new)
            .map_err(|error| {
                crate::diagnostic!(serde_json::json!({
                    "component":"delivery","kind":"ledger.load_failed",
                    "code":error.code(),"persistence":error.diagnostic(),
                }));
                error.code().to_owned()
            });
        let (host_helper_root, host_cli_dir) = Self::helper_places_from(&options);
        let mut snapshot = Snapshot::initial(&options);
        if let Ok(ledger) = &delivery_ledger {
            snapshot.delivery_watches = ledger.watches.iter().map(|watch| watch.view()).collect();
        }
        snapshot.status.environment = environment.statuses;
        let (ui_state, pane_terminal_sizes, disposition) = persistence::load(&state_path);
        let local_issues_path = options.local_issues_path.as_ref().map(PathBuf::from);
        snapshot.ui_state = ui_state;
        snapshot.navigator.sleeping_sessions = snapshot.ui_state.agent_sleep.dormant_snapshots();
        snapshot.navigator.archived_sessions =
            snapshot.ui_state.session_archive.snapshots(&|_| None);
        let shortcuts_imported = options
            .shortcut_import_path
            .as_deref()
            .is_some_and(|source| import_native_app_shortcuts(&mut snapshot, Path::new(source)));
        snapshot.navigator.devices = workspace::devices(
            &options.node_id,
            options.machine_name.as_deref(),
            &snapshot.ui_state,
        );
        // This machine's row names the root a device consent would name, so
        // the add form can show it before the first device exists.
        if let Some(local) = snapshot
            .navigator
            .devices
            .iter_mut()
            .find(|device| options.node_id == device.id)
        {
            local.host.helper_root = Some(host_helper_root.clone());
            local.host.cli_dir = Some(host_cli_dir.clone());
        }
        snapshot.navigator.focused_device_id = Some(
            snapshot
                .ui_state
                .focused_device_id
                .clone()
                .unwrap_or_else(|| options.node_id.to_string()),
        );
        snapshot.navigator.focused_checkout_id = snapshot.ui_state.focused_checkout_id.clone();
        // Built before any lock exists, so the own node is asked here.
        let wanted = workspace::PathIndex::wanted(
            &options.node_id,
            &snapshot.ui_state.workspace_registrations,
            &[],
            &crate::model::WorktreeCatalogSnapshot::default(),
        );
        let catalog_paths = Arc::new(
            workspace::PathIndex::ask(own_node.as_ref(), wanted).unwrap_or_else(|error| {
                crate::diagnostic!(serde_json::json!({
                    "component": "workspace_catalog",
                    "kind": "catalog.path_facts_failed",
                    "message": error,
                }));
                workspace::PathIndex::default()
            }),
        );
        snapshot.navigator.workspaces = workspace::build_catalog(
            &options.node_id,
            &snapshot.ui_state.workspace_registrations,
            &[],
            &crate::model::WorktreeCatalogSnapshot::default(),
            &catalog_paths,
        );
        let diagnostics: Vec<(&str, &str)> = match disposition {
            persistence::LoadDisposition::Loaded => Vec::new(),
            persistence::LoadDisposition::Unrecognized(fields) => fields
                .iter()
                .map(|field| match field {
                    persistence::UnrecognizedField::Theme => (
                        "ui_state.theme_unknown",
                        "The stored theme was not recognized; Dark was used",
                    ),
                    persistence::UnrecognizedField::ColorTheme => (
                        "ui_state.color_theme_unknown",
                        "The stored color theme was not recognized; Hide was used",
                    ),
                    persistence::UnrecognizedField::AgentLogoMode => (
                        "ui_state.agent_logo_mode_unknown",
                        "The stored agent logo mode was not recognized; hidden was used",
                    ),
                    persistence::UnrecognizedField::SidebarLens => (
                        "ui_state.sidebar_lens_unknown",
                        "The stored sidebar lens or task view was not recognized; the projects, by state, were used",
                    ),
                    persistence::UnrecognizedField::Detail => (
                        "ui_state.detail_unknown",
                        "The stored 보기 slot was not recognized; it loaded empty",
                    ),
                })
                .collect(),
            persistence::LoadDisposition::Missing => vec![(
                "ui_state.missing",
                "UI state was not found; safe defaults were loaded",
            )],
            persistence::LoadDisposition::Corrupt => vec![(
                "ui_state.corrupt",
                "UI state could not be decoded; safe defaults were loaded",
            )],
        };
        for (kind, message) in diagnostics {
            crate::diagnostic!(serde_json::json!({
                "component": "ui_state",
                "kind": kind,
                "message": message,
                "fallback": "defaults"
            }));
            snapshot.status.diagnostics.push(DiagnosticSnapshot {
                kind: kind.to_owned(),
                message: message.to_owned(),
                occurred_at: unix_milliseconds(),
            });
        }
        if snapshot
            .ui_state
            .interface_language
            .as_ref()
            .is_some_and(|value| {
                serde_json::from_value::<crate::model::InterfaceLanguage>(value.clone()).is_err()
            })
        {
            let kind = "ui_state.interface_language_invalid";
            let message = "The stored interface language is invalid; English is used and the value is retained";
            crate::diagnostic!(serde_json::json!({
                "component": "ui_state", "kind": kind, "fallback": "en"
            }));
            snapshot.status.diagnostics.push(DiagnosticSnapshot {
                kind: kind.to_owned(),
                message: message.to_owned(),
                occurred_at: unix_milliseconds(),
            });
        }
        // A stored width the drag could not have produced (a hand-edited
        // store) is not drawn; the sidebar opens at its default.
        if !crate::model::sidebar_width_fits(snapshot.ui_state.sidebar_width) {
            let message = format!(
                "Stored sidebar width {} is outside {}..={}; the default was used",
                snapshot.ui_state.sidebar_width,
                crate::model::SIDEBAR_WIDTH_MIN,
                crate::model::SIDEBAR_WIDTH_MAX
            );
            crate::diagnostic!(serde_json::json!({
                "component": "ui_state",
                "kind": "ui_state.sidebar_width_out_of_range",
                "message": message,
            }));
            snapshot.status.diagnostics.push(DiagnosticSnapshot {
                kind: "ui_state.sidebar_width_out_of_range".to_owned(),
                message,
                occurred_at: unix_milliseconds(),
            });
            snapshot.ui_state.sidebar_width = crate::model::default_sidebar_width();
        }
        let workspace_views = options.workspace_views_path.as_ref().map(|path| {
            let (store, diagnostics) = WorkspaceViewStore::open(
                PathBuf::from(path),
                (
                    snapshot.ui_state.right_panel_visible,
                    snapshot.ui_state.right_panel_section,
                ),
            );
            for (kind, message) in diagnostics {
                crate::diagnostic!(serde_json::json!({
                    "component": "workspace_views",
                    "kind": kind,
                    "message": message,
                    "fallback": if kind == "workspace_views.unreadable" { "defaults" } else { "none" }
                }));
                snapshot.status.diagnostics.push(DiagnosticSnapshot {
                    kind: kind.to_owned(),
                    message,
                    occurred_at: unix_milliseconds(),
                });
            }
            store
        });
        let mut runtime = Self {
            snapshot,
            hide_started: delivery_ledger
                .as_ref()
                .map(|ledger| crate::coordination::started::hide_started(ledger))
                .unwrap_or_default(),
            delivery_ledger,
            delivery_client: None,
            delivery_observations: HashMap::new(),
            view_feedback: view_feedback::FeedbackRuntime::default(),
            composer_marks: Vec::new(),
            delivery_input_traced: HashMap::new(),
            composer_store: None,
            factory_recipients: std::collections::BTreeMap::new(),
            factory_panes: HashMap::new(),
            factory_screen: None,
            delivery_overflow: HashSet::new(),
            delivery_connected: HashSet::new(),
            delivery_panes: HashMap::new(),
            registrations_gone: std::collections::BTreeMap::new(),
            registration_rebinds: std::collections::BTreeMap::new(),
            archive_deletes_sent: HashSet::new(),
            archive_retention_sent: Arc::default(),
            archive_registration_waits: HashMap::new(),
            node: options.node_id.clone(),
            machine_name: options.machine_name.clone(),
            device_machine_ids: HashMap::new(),
            unresolved_machine_lineage: HashSet::new(),
            file_roots: None,
            state_path,
            home_path: environment.home_path,
            remote_connections: HashMap::new(),
            inbound_arrivals: HashMap::new(),
            retired_remote_syncs: Vec::new(),
            remote_device_tests: HashMap::new(),
            device_herdr_behind: HashMap::new(),
            device_hosts: HashMap::new(),
            device_host_retries: HashMap::new(),
            last_host_generation: 0,
            changes_published_key: None,
            device_raw_sessions: HashMap::new(),
            device_facts: HashMap::new(),
            device_recent_tabs: HashMap::new(),
            device_worktrees: HashMap::new(),
            device_ports: HashMap::new(),
            device_disk: HashMap::new(),
            device_github: HashMap::new(),
            device_github_reads: HashMap::new(),
            device_github_read_generation: HashMap::new(),
            device_github_over_limit: HashMap::new(),
            own_node,
            devices,
            document_places: HashMap::new(),
            document_saves: HashMap::new(),
            document_opens: HashMap::new(),
            next_document_generation: 0,
            host_helper_root,
            host_cli_dir,
            live: None,
            remote_controls: HashMap::new(),
            remote_file_transports: HashMap::new(),
            remote_control_requests: VecDeque::new(),
            remote_operations: HashMap::new(),
            remote_connection_generations: HashMap::new(),
            remote_tab_creations_in_flight: HashSet::new(),
            terminals: Arc::new(terminal::NoTerminals),
            terminal_states: HashMap::new(),
            terminal_attach_requested: HashSet::new(),
            terminal_intents: terminal::TerminalIntents::default(),
            input_gate: Arc::default(),
            next_remote_file_generation: 0,
            attachment: None,
            attachment_rejection: None,
            attachment_worker: None,
            terminal_sizes: pane_terminal_sizes.into_iter().collect(),
            wheel_before_attach: HashMap::new(),
            viewport_scrolls: HashMap::new(),
            panes_scroll_held: HashSet::new(),
            pane_relocations_in_flight: BTreeMap::new(),
            pane_hook_tokens: BTreeMap::new(),
            hook_diagnosis: None,
            kit_states: BTreeMap::new(),
            unknown_kit_agents: BTreeSet::new(),
            unknown_kit_agents_limit_reported: false,
            local_kit_pending: None,
            first_run_apply: None,
            local_kit_next_status: None,
            local_kit_check_requested: false,
            device_kit_pending: BTreeMap::new(),
            device_first_run_choice: BTreeMap::new(),
            device_kit_running: BTreeSet::new(),
            codex_daemon_off_running: BTreeMap::new(),
            codex_daemon_off_runs: 0,
            device_kit_removals: BTreeMap::new(),
            device_kit_removing: BTreeSet::new(),
            ai_settings: None,
            pending_ai_settings_save: None,
            ai_observing: false,
            ai_start_observing: false,
            background_ai_providers: Vec::new(),
            usage_window_visible: false,
            usage_ledger_client: None,
            usage_popover_open: false,
            usage_popover_open_generation: 0,
            usage_limits: Arc::new(hide_ai::UsageLimits::new()),
            ai_usage: crate::usage_ledger::ai_usage::AiUsageQueue::new(),
            usage_pause_announced: false,
            recent_visible_tabs: Vec::new(),
            pending_tab_rename: None,
            panes_closing: HashSet::new(),
            factory_closes_sent: HashSet::new(),
            tree_closes: Vec::new(),
            next_tree_close_id: 0,
            recent_closed: VecDeque::new(),
            close_capture_order: VecDeque::new(),
            close_operations: HashMap::new(),
            close_status_checks_in_flight: HashSet::new(),
            dormant_status_check: None,
            unowned_dormant_listing: None,
            pane_operations: HashMap::new(),
            confirmed_pane_layout_signatures: HashMap::new(),
            op_timings: op_timing::OpTimings::default(),
            input_requests: terminal_input::InputRequests::default(),
            drawn_closes: HashMap::new(),
            confirmed_layouts: HashMap::new(),
            close_line_places: HashMap::new(),
            ingesting_session: false,
            pumping_geometry: false,
            provisional_tabs: Vec::new(),
            recent_closed_sequence: 0,
            reopen_in_flight: None,
            reopen_after_close: None,
            live_generation: 0,
            status_refresh_requested: false,
            created_tab_checkouts: BTreeMap::new(),
            created_device_tabs: BTreeMap::new(),
            republish_waker: None,
            #[cfg(test)]
            republish_requested: false,
            birth_clock: Arc::new(Instant::now),
            next_async_operation_id: 0,
            workspace_creations_in_flight: HashSet::new(),
            editor_documents: HashMap::new(),
            archive_documents: HashMap::new(),
            session_catalog_rows: Vec::new(),
            memory_sessions_load_in_flight: false,
            memory_sessions_load_pending: false,
            memory_sessions_load_generation: 0,
            archive_detail_load_generation: 0,
            links_work: links::LinksWork::default(),
            backlog_work: backlog::BacklogWork::default(),
            work_clock_minute: 0,
            memory_work: Default::default(),
            project_memory: Default::default(),
            editor_tab_history: Vec::new(),
            worker_context: None,
            pending_session_resolutions: BTreeMap::new(),
            failed_session_resolutions: BTreeMap::new(),
            delivery_holds: BTreeMap::new(),
            letters_waiting: BTreeMap::new(),
            session_window_deadline_unix_ms: 0,
            #[cfg(test)]
            pane_header_derivations: 0,
            state_save_pending: false,
            state_save_active: false,
            state_save_worker: None,
            project_settings_pending: BTreeMap::new(),
            guardrails: guardrails::GuardrailsWork::default(),
            system_language: None,
            pet_active_at_unix_ms: unix_milliseconds(),
            agent_worked_at_unix_ms: 0,
            pet_waking_until_unix_ms: 0,
            pet_dragging: false,
            operator_focused_pane_id: None,
            local_read_connection: crate::agent_state::ReadConnection::new(read_mark(0)),
            remote_read_connections: BTreeMap::new(),
            read_marks_made: 1,
            visible_tab_ids: BTreeMap::new(),
            tab_focus_requests: Vec::new(),
            herdr_tab_moves_read: None,
            herdr_active_tabs_seen: BTreeSet::new(),
            herdr_focused_tab_seen: None,
            herdr_tab_focus_seen: None,
            pending_pane_focus: None,
            pane_focus_in_flight: None,
            pane_focus_readback: None,
            pane_focus_readback_again: false,
            control_lane: ControlLane::default(),
            last_view_intent_serial: 0,
            next_pane_focus_serial: 0,
            herdr_active_tab_ids: BTreeSet::new(),
            pet_unseen_observed: std::collections::BTreeMap::new(),
            restore_hint_pending: true,
            last_session_spaces: Vec::new(),
            ui_attached: true,
            label_overlay: Default::default(),
            label_services: None,
            pull_request_times: Default::default(),
            request_view_observed: false,
            pending_checks_read_at: None,
            checks_memory: Default::default(),
            issue_tokens: Default::default(),
            issue_candidates: Default::default(),
            issue_write_pending: None,
            unconfirmed_issue_tokens: BTreeMap::new(),
            unconfirmed_node_issue_tokens: BTreeMap::new(),
            invalid_links: Default::default(),
            local_issues: match &local_issues_path {
                Some(path) => crate::local_issues::load(path),
                None => Ok(crate::local_issues::LocalIssueStore::default()),
            },
            local_issues_path,
            effects: crate::effects::EffectHold::new(options.effects_held),
            local_issues_save_pending: false,
            local_issues_save_active: false,
            local_issue_links: BTreeMap::new(),
            github_search: issues::GithubSearchWork::default(),
            pr_link_checkout: None,
            task_agent_launch: None,
            group_start: None,
            group_start_saves: Vec::new(),
            task_pane_shell: None,
            home_links: HashMap::new(),
            last_accepted_catalog: None,
            catalog_paths,
            catalog_from_coordinator: false,
            checkout_tab_order: BTreeMap::new(),
            herdr_workspace_tab_order: BTreeMap::new(),
            herdr_worktrees: BTreeMap::new(),
            pending_tab_move: BTreeMap::new(),
            next_tab_move_generation: 0,
            unresolved_active_tabs: BTreeSet::new(),
            forks_in_flight: HashMap::new(),
            fork_focus: None,
            agent_sleep_next_decision_unix_ms: 0,
            node_sleep_next_decision_unix_ms: HashMap::new(),
            node_sleep_shown_tabs: HashMap::new(),
            agent_sleep_backoff: HashMap::new(),
            agent_wake_started: HashMap::new(),
            pane_reopens: HashMap::new(),
            fork_sequence: 0,
            listening_ports: crate::model::ListeningPortsSnapshot::default(),
            worktree_catalog: crate::model::WorktreeCatalogSnapshot::default(),
            local_worktree_paths: Arc::default(),
            github: crate::model::GithubSnapshot::default(),
            disk_usage: Vec::new(),
            github_generations: HashMap::new(),
            worktree_project_generations: HashMap::new(),
            github_clock: HashMap::new(),
            github_wanted: HashSet::new(),
            github_sighted: HashMap::new(),
            github_answered: HashMap::new(),
            github_remotes: HashMap::new(),
            github_read_generation: HashMap::new(),
            github_settled: HashSet::new(),
            merged_here: HashMap::new(),
            github_over_limit: 0,
            github_polled: BTreeSet::new(),
            github_poll_over_limit: 0,
            github_store: None,
            disk_generation: 0,
            disk_sweeps: Default::default(),
            cleanup: None,
            cleanup_published_node: None,
            next_cleanup_id: 0,
            cleanup_worker: None,
            worktree_generation: 0,
            worktree_removals: 0,
            removed_worktrees: Vec::new(),
            next_worktree_removal_id: 0,
            workspace_removals_in_flight: HashSet::new(),
            next_task_operation_id: 0,
            next_repository_clone_id: 0,
            repository_clone_job: None,
            next_ssh_hosts_id: 0,
            ssh_hosts_job: None,
            purpose_operation_target: None,
            created_purpose_writes_in_flight: HashMap::new(),
            unconfirmed_created_purposes: HashMap::new(),
            next_explorer_operation_id: 0,
            explorer_queue: VecDeque::new(),
            delta: snapshot_delta::DeltaState::default(),
            usage_edit: 1,
            agent_scope_cache: crate::agent_state::ScopeCache::default(),
            workspace_views,
            workspace_actions: VecDeque::new(),
            browser_pages: HashMap::new(),
            #[cfg(test)]
            test_dirs: Vec::new(),
        };
        runtime.resync_navigator_focus();
        runtime.apply_persisted_pet_state();
        runtime.refresh_pet();
        runtime.refresh_agent_scopes();
        // A sleep saved before this start is as closed as it was.
        runtime.sync_input_gate();
        if shortcuts_imported {
            runtime.persist_ui_state();
        }
        runtime
    }

    /// The gate the writers that bypass a node's terminal ask at the write.
    pub fn input_gate(&self) -> Arc<crate::input_gate::PaneInputGate> {
        Arc::clone(&self.input_gate)
    }

    /// Every node's terminals, reached once the shell has built them.
    pub(crate) fn set_terminals(
        &mut self,
        terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes>,
    ) {
        self.terminals = terminals;
    }

    /// The hold on this core's outside effects, for a worker that waits on
    /// it from a thread of its own.
    pub(crate) fn effects(&self) -> Arc<crate::effects::EffectHold> {
        Arc::clone(&self.effects)
    }

    /// Whether this core still waits for its move's link before it acts
    /// outside its machine (`effects.rs`).
    pub(crate) fn effects_held(&self) -> bool {
        self.effects.held()
    }

    /// Holds this core's outside effects, as `CoreOptions::effects_held`
    /// starts a core on a move's copy.
    #[cfg(test)]
    pub(crate) fn hold_effects(&mut self) {
        self.effects = crate::effects::EffectHold::new(true);
    }

    /// Lets the held effects run, once the move's link committed this core;
    /// the devices it dials are dialed now. Returns whether this released
    /// the hold.
    pub(crate) fn release_effects(&mut self) -> bool {
        if !self.effects.release() {
            return false;
        }
        crate::diagnostic!(serde_json::json!({"component": "core", "kind": "effects.released"}));
        self.connect_registered_devices();
        true
    }

    pub fn install_worker_context(
        &mut self,
        runtime: Weak<Mutex<Runtime>>,
        notifier: ChangeNotifier,
    ) {
        self.worker_context = Some(RuntimeWorkerContext { runtime, notifier });
        if self.snapshot.ui_state.right_panel_visible
            && self.snapshot.ui_state.right_panel_section == RightPanelSection::Sessions
        {
            self.request_sessions_refresh();
        }
    }

    pub(crate) fn take_state_save_worker(&mut self) -> Option<thread::JoinHandle<()>> {
        self.state_save_worker.take()
    }

    /// The machine this core runs on.
    pub(crate) fn node(&self) -> &crate::node::NodeId {
        &self.node
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The daemon installs only handles opened under its pinned registrations.
    /// An empty set still enforces the boundary; a client that pins nothing leaves this as `None`.
    /// Returns whether the front Workspace's View tabs, which waited for its
    /// root, began to restore, so the caller announces the change.
    pub fn set_file_roots(&mut self, roots: crate::files::FileRoots) -> bool {
        self.file_roots = Some(roots);
        self.restore_front_when_ready()
    }

    pub fn set_live(&mut self, context: LiveContext) {
        self.live_generation = self.live_generation.saturating_add(1);
        self.live = Some(context);
        // A sleeping session can wake only over a connection.
        self.refresh_dormant_rows();
    }

    pub fn install_remote_control(&mut self, context: RemoteControlContext) {
        self.remote_controls
            .insert(context.target_id().to_owned(), context);
    }

    /// The installed connection owns coordinator writes even before its first
    /// successful read. Retired coordinators retain their distinct connector
    /// allocation until they stop, so removal or replacement revokes writes.
    pub(crate) fn remote_coordinator_is_current(
        &self,
        target_id: &str,
        connector: &Arc<dyn hide_herdr_client::ApiConnector>,
    ) -> bool {
        self.remote_controls
            .get(target_id)
            .is_some_and(|context| Arc::ptr_eq(&context.api_connector(), connector))
    }

    /// The Herdr API connection the core already holds for a connected SSH
    /// device, for a daemon request that reads or writes one of its panes
    /// outside the attach set. A device that is not connected has none.
    pub fn remote_herdr_api(
        &self,
        target_id: &str,
    ) -> Option<Arc<dyn hide_herdr_client::ApiConnector>> {
        let connected = self
            .snapshot
            .status
            .remote
            .iter()
            .any(|status| status.target_id == target_id && status.state == "connected");
        if !connected {
            return None;
        }
        self.remote_controls
            .get(target_id)
            .map(RemoteControlContext::api_connector)
    }

    pub(crate) fn install_remote_file_transport(
        &mut self,
        target_id: impl Into<String>,
        transport: devices::DeviceReach,
    ) {
        self.remote_file_transports
            .insert(target_id.into(), transport);
    }

    /// Reports the workspaces whose Herdr-named active tab the navigator
    /// could not place. Reconcile runs once a second, so only a change in the
    /// set is worth a diagnostic; repeating it every tick would grow the
    /// status section without saying anything new.
    fn report_unresolved_active_tabs(&mut self, current: BTreeSet<String>) {
        let added = current
            .difference(&self.unresolved_active_tabs)
            .cloned()
            .collect::<Vec<_>>();
        for entry in &added {
            let (workspace_id, tab_id) = entry
                .split_once('/')
                .expect("unresolved active tab entries carry both ids");
            self.push_diagnostic(
                "tab.active_unresolved",
                format!(
                    "Herdr workspace {workspace_id} reports active tab {tab_id}, which is not in any checkout's tab list"
                ),
            );
        }
        self.unresolved_active_tabs = current;
    }

    fn push_diagnostic(&mut self, kind: impl Into<String>, message: impl Into<String>) {
        let diagnostic = DiagnosticSnapshot {
            kind: kind.into(),
            message: message.into(),
            occurred_at: unix_milliseconds(),
        };
        crate::diagnostic!(serde_json::json!({
            "kind": diagnostic.kind, "message": diagnostic.message, "occurred_at": diagnostic.occurred_at
        }));
        self.snapshot.status.diagnostics.push(diagnostic);
        // The list rides the revisioned rest section, so it is bounded: a
        // fault that repeats every few seconds otherwise grows what every
        // rest re-send carries for the life of the process.
        let excess = self
            .snapshot
            .status
            .diagnostics
            .len()
            .saturating_sub(DIAGNOSTIC_RETENTION);
        if excess > 0 {
            self.snapshot.status.diagnostics.drain(..excess);
        }
    }

    pub fn set_error(
        &mut self,
        kind: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) {
        let kind = kind.into();
        let message = message.into();
        // `last_error` lives until the next event clears it, and no surface
        // reads it while a headless client or a fixture drives the core, so a
        // refusal that is not logged here leaves no trace at all.
        crate::diagnostic!(serde_json::json!({
            "component": "runtime",
            "kind": "error.reported",
            "error_kind": kind,
            "message": message,
            "retryable": retryable,
        }));
        self.snapshot.status.last_error = Some(LastErrorSnapshot {
            kind,
            message,
            retryable,
            occurred_at: unix_milliseconds(),
            request_id: None,
        });
    }

    /// A refusal that answers one request, named so the surface that sent it
    /// reads it as its own (`LastErrorSnapshot::request_id`).
    pub(crate) fn set_request_error(
        &mut self,
        kind: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        request_id: Option<&str>,
    ) {
        self.set_error(kind, message, retryable);
        if let Some(error) = self.snapshot.status.last_error.as_mut() {
            error.request_id = request_id.map(str::to_owned);
        }
    }

    pub fn dispatch_json(&mut self, bytes: &[u8]) -> bool {
        let event = match events::decode(bytes) {
            Ok(event) => event,
            Err(error) => {
                self.set_error(error.kind, error.message, false);
                return true;
            }
        };

        // A stale sidebar row has no action left to offer. Reject the whole
        // open/disclosure intent before focus, persistence or visit effects.
        if let Event::FocusCheckout(payload) = &event
            && !self.snapshot.navigator.workspaces.iter().any(|workspace| {
                workspace.id == payload.workspace_id
                    && workspace
                        .checkouts
                        .iter()
                        .any(|checkout| checkout.id == payload.checkout_id)
            })
        {
            self.push_diagnostic(
                "checkout.row_stale",
                format!(
                    "Checkout {} is not available in {}",
                    payload.checkout_id, payload.workspace_id
                ),
            );
            return true;
        }

        let cleared_error = self.snapshot.status.last_error.take().is_some();
        let intent = self
            .separate_view_areas()
            .then(|| AreaIntent::of(&event))
            .flatten();
        // An open asked while the front files tool shows the tree alone
        // keeps the tree beside what it opens, whichever route it takes and
        // however late its view lands (B11, B12).
        let lone_tree = (self.separate_view_areas() && AreaIntent::opens_view(&event))
            .then(|| self.front_lone_tree())
            .flatten();
        // A keystroke in a document is as frequent as terminal input and
        // moves no area either: its first edit keeps its own displays open,
        // and the next snapshot read's sync publishes that.
        let high_frequency = event.is_terminal_io() || matches!(event, Event::FileDraft(_));
        let chooses_workspace = matches!(
            event,
            Event::FocusCheckout(_) | Event::FocusPane(_) | Event::FocusTab(_)
        );
        let supersedes_memory = matches!(event, Event::FocusDevice(_) | Event::FileFocus(_));
        let visible_tab_before = chooses_workspace
            .then(|| self.focused_visible_tab_id())
            .flatten();
        let mut changed = self.apply(event) || cleared_error;
        if supersedes_memory && self.snapshot.status.last_error.is_none() {
            self.supersede_memory_navigation();
        }
        // A committed visit to another tab wakes what sleeps there (PRD
        // agent-sleep B12); a refused one moved nothing.
        if chooses_workspace && self.snapshot.status.last_error.is_none() {
            changed |= self.agent_sleep_visit(visible_tab_before.as_deref());
        }
        // The areas follow the event that moved the screen, in the same
        // frame (D-08); terminal input and output and a document's
        // keystrokes never move them, so they skip the pass.
        if self.separate_view_areas() && !high_frequency {
            if let Some(intent) = intent
                && self.snapshot.status.last_error.is_none()
            {
                self.apply_area_intent(intent);
                // The call is numbered even when nothing else moved (an
                // open of the view already shown), and the shell must hear it.
                changed = true;
            }
            if let Some(key) = lone_tree
                && self.snapshot.status.last_error.is_none()
                && self.front_workspace_key().as_ref() == Some(&key)
            {
                changed |= self.keep_lone_tree(&key);
            }
            if chooses_workspace && self.snapshot.status.last_error.is_none() {
                self.mark_front_chosen();
            }
            self.sync_workspace_view();
        }
        // Every event that can move the visible tab funnels through here, so
        // the attach window is maintained once rather than at each of the four
        // places a tab becomes visible.
        let released = self.track_visible_tab_attachments();
        changed || released
    }
}

/// One tab's panes, as the navigator draws them.
///
/// Lifted out of the placement loop so every section projects a pane row
/// through the same code: a row that differed between sections would be a
/// second definition of what a pane is.
fn project_layout_panes(
    layout: &crate::sidebar::SessionLayoutPayload,
    payload: &SessionSnapshotPayload,
    projected_agents: &[SidebarAgentSnapshot],
    listening_ports: &[crate::model::ListeningPortSnapshot],
    fallback_cwd: &str,
) -> Vec<PaneSnapshot> {
    layout
        .panes
        .iter()
        .map(|pane| {
            let agent = projected_agents
                .iter()
                .find(|agent| agent.pane_id == pane.pane_id);
            let source = payload
                .panes
                .iter()
                .find(|source| source.pane_id == pane.pane_id);
            let cwd = source
                .and_then(|source| source.cwd.clone())
                .or_else(|| {
                    payload
                        .agents
                        .iter()
                        .find(|source| {
                            source.pane_id.as_deref().or(source.id.as_deref())
                                == Some(pane.pane_id.as_str())
                        })
                        .and_then(|source| source.cwd.clone())
                })
                .unwrap_or_else(|| fallback_cwd.to_owned());
            let ports = crate::ports::attributed_ports(&cwd, listening_ports);
            PaneSnapshot {
                start_claim: source.and_then(|source| {
                    source
                        .start_claim
                        .as_deref()
                        .filter(|label| crate::start_claim::is_reserved(label))
                        .or_else(|| {
                            source
                                .label
                                .as_deref()
                                .filter(|label| crate::start_claim::is_reserved(label))
                        })
                        .map(str::to_owned)
                }),
                terminal_id: source.and_then(|source| source.terminal_id.clone()),
                failed_terminal_id: source.and_then(|source| source.failed_terminal_id.clone()),
                id: pane.pane_id.clone(),
                herdr_label: source
                    .and_then(|source| crate::start_claim::public_label(source.label.as_deref())),
                terminal_title: source.and_then(|source| {
                    crate::start_claim::public_label(source.terminal_title.as_deref())
                }),
                cwd: cwd.clone(),
                // This projection cannot see the read record ledger, so both
                // read-dependent values are refilled from the navigator's
                // agent rows once those are final; see
                // `sync_pane_status_from_agents`.
                status_code: agent
                    .map(|agent| agent.status_code)
                    .unwrap_or(AgentStatusCode::Unknown),
                requires_close_confirmation: agent
                    .is_some_and(|agent| agent.requires_close_confirmation),
                requires_close_status_check: agent
                    .is_some_and(|agent| agent.requires_close_status_check),
                identity_label: agent.map(|agent| agent.identity_label.clone()),
                activity_at_unix_ms: agent.and_then(|agent| agent.changed_at_unix_ms),
                fork: pane_fork_snapshot(agent),
                ports,
                servers: crate::ports::attributed_servers(&cwd, listening_ports),
                // Both are refilled from the final agent list once the read
                // axis and the lineage are applied; see `sync_pane_lineage`.
                children: None,
                lineage_path: Vec::new(),
                sleep: None,
                sleep_action: None,
            }
        })
        .collect()
}

fn find_workspace_for_context<'a>(
    node: &crate::node::NodeId,
    workspaces: &'a mut Vec<crate::model::WorkspaceSnapshot>,
    context_path: Option<&str>,
    session_workspace_id: &str,
    paths: &workspace::PathIndex,
    unresolved_roots: &mut Vec<String>,
) -> Option<&'a mut crate::model::WorkspaceSnapshot> {
    let Some(raw_path) = context_path else {
        return workspaces
            .iter()
            .position(|workspace| workspace.id == session_workspace_id)
            .and_then(|index| workspaces.get_mut(index));
    };
    // The root was read by the node outside the runtime lock; a directory
    // the index does not carry is placed by its own path and reported,
    // never by reading the folder from here.
    if !paths.knows(raw_path) {
        unresolved_roots.push(raw_path.to_owned());
    }
    let root = paths.root(raw_path);
    let normalized = root.clone();
    // A navigator project records the Herdr workspaces occupying it. One
    // Herdr workspace can span two repositories and so two projects, so the
    // pane's directory picks between them. The path comparisons below are the
    // fallback for a registration Herdr has no workspace for, and for a
    // catalog precomputed from a slightly older session.
    let carrying = workspaces
        .iter()
        .enumerate()
        .filter(|(_, workspace)| {
            workspace
                .session_workspace_ids
                .iter()
                .any(|id| id == session_workspace_id)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if let Some(index) = carrying
        .iter()
        .copied()
        .find(|index| {
            workspaces[*index]
                .checkouts
                .iter()
                .any(|checkout| path_is_within_checkout(paths, raw_path, &checkout.path))
        })
        .or_else(|| carrying.first().copied())
    {
        return workspaces.get_mut(index);
    }
    if let Some(index) = workspaces.iter().position(|workspace| {
        workspace
            .checkouts
            .iter()
            .any(|checkout| workspace::comparison_by_names(Path::new(&checkout.path)) == normalized)
    }) {
        return workspaces.get_mut(index);
    }
    if let Some(index) = workspaces.iter().position(|workspace| {
        workspace::comparison_by_names(Path::new(&workspace.path)) == normalized
    }) {
        return workspaces.get_mut(index);
    }
    if let Some(index) = workspaces.iter().position(|workspace| {
        let workspace_path = workspace::comparison_by_names(Path::new(&workspace.path));
        normalized.starts_with(&format!("{workspace_path}/"))
    }) {
        return workspaces.get_mut(index);
    }
    let mut temporary = workspace::inspect_temporary(&root, node.as_str(), paths);
    temporary.session_workspace_ids = vec![session_workspace_id.to_owned()];
    workspaces.push(temporary);
    workspaces.last_mut()
}

fn path_is_within_checkout(paths: &workspace::PathIndex, path: &str, checkout_path: &str) -> bool {
    let path = PathBuf::from(paths.comparison(path));
    let checkout_path = PathBuf::from(workspace::comparison_by_names(Path::new(checkout_path)));
    // `Path::starts_with` compares path components, so `barista` cannot
    // match the checkout component `bar`.
    path.starts_with(checkout_path.as_path())
}

/// Projects the two fork facts the pane header renders from.
///
/// A pane with no agent is neither forkable nor a fork, which is why the
/// default carries both answers rather than the snapshot holding an optional.
pub fn pane_fork_snapshot(agent: Option<&SidebarAgentSnapshot>) -> PaneForkSnapshot {
    let Some(agent) = agent else {
        return PaneForkSnapshot::default();
    };
    let available = is_forkable(Some(agent.agent_kind.as_str()), agent.session_id.as_deref());
    PaneForkSnapshot {
        available,
        reason: (!available && ForkableAgent::parse(&agent.agent_kind).is_some())
            .then(|| "This agent has not reported its conversation yet".to_owned()),
        forked_from: agent.forked_from.clone(),
    }
}

pub fn validate_options(options: &CoreOptions) -> Result<(), &'static str> {
    if options.schema_version != SCHEMA_VERSION {
        return Err("options schema version does not match");
    }
    if options.app_state_path.trim().is_empty() {
        return Err("app_state_path must not be empty");
    }
    if options
        .herdr_socket_path
        .as_ref()
        .is_some_and(|path| path.trim().is_empty())
    {
        return Err("herdr_socket_path must be null or non-empty");
    }
    if options
        .herdr_bin_path
        .as_ref()
        .is_some_and(|path| path.trim().is_empty())
    {
        return Err("herdr_bin_path must be null or non-empty");
    }
    if options
        .home
        .as_ref()
        .is_some_and(|home| !std::path::Path::new(home).is_absolute())
    {
        return Err("home must be null or an absolute path");
    }
    Ok(())
}

/// What a waiting child is waiting for, in the words the tooltip uses.
///
/// It names the demand when there is one, because "waiting for an approval"
/// and "running with nothing to show for it" ask different things of the
/// operator.
/// The agent line for one worktree row.
///
/// Empty with no reason is nobody working here; empty with a reason is a
/// worktree Hide cannot see into. Keeping those two apart is the whole point
/// of the third uninstrumented position (PRD B35, D-60).
fn worktree_agent_line(
    panes: Option<&HashSet<String>>,
    chips: &HashMap<String, crate::model::AgentChipSnapshot>,
    instrumentation: &HashMap<String, crate::model::PaneChildrenSnapshot>,
    order: &[SidebarAgentSnapshot],
) -> crate::model::WorktreeAgentLineSnapshot {
    let Some(panes) = panes else {
        return crate::model::WorktreeAgentLineSnapshot::default();
    };
    // The sidebar's order, so the two screens read the same way.
    let agents = order
        .iter()
        .filter(|agent| panes.contains(&agent.pane_id))
        .filter_map(|agent| chips.get(&agent.pane_id).cloned())
        .collect::<Vec<_>>();
    // The first reason in the resolution order the crate declares, rather
    // than the first one the pane iteration happened to reach.
    let worst = order
        .iter()
        .filter(|agent| panes.contains(&agent.pane_id))
        .filter_map(|agent| instrumentation.get(&agent.pane_id))
        .filter_map(|children| children.uninstrumented_code.as_deref())
        .filter_map(hide_agent_hooks::diagnosis::UninstrumentedReason::from_code)
        .min();
    crate::model::WorktreeAgentLineSnapshot {
        agents,
        uninstrumented_reason: worst.map(|reason| reason.message().to_owned()),
        uninstrumented_label: worst.map(|reason| reason.accessibility_label().to_owned()),
        uninstrumented_code: worst.map(|reason| reason.code().to_owned()),
    }
}

/// Brings the removed native app's pane chords across once: only while the core's own
/// set is empty and no import ran before, so a set the operator edited here,
/// or reset to defaults after the import, is never overwritten. Returns
/// whether the UI state changed and needs saving. A file that is there but
/// cannot be used is recorded and left for the next launch to try again.
fn import_native_app_shortcuts(snapshot: &mut Snapshot, source: &Path) -> bool {
    let ui_state = &mut snapshot.ui_state;
    if ui_state.shortcut_bindings_imported || !ui_state.shortcut_bindings.is_empty() {
        return false;
    }
    let refused = match persistence::read_native_app_shortcuts(source) {
        persistence::NativeAppShortcuts::Missing => return false,
        persistence::NativeAppShortcuts::Found(bindings) if events::bindings_fit(&bindings) => {
            crate::diagnostic!(serde_json::json!({
                "component": "ui_state",
                "kind": "ui_state.shortcuts_imported",
                "message": "Pane shortcuts were brought across from the macOS app",
                "count": bindings.len(),
            }));
            ui_state.shortcut_bindings = bindings;
            ui_state.shortcut_bindings_imported = true;
            return true;
        }
        persistence::NativeAppShortcuts::Found(_) => "too many or too long",
        persistence::NativeAppShortcuts::Unreadable => "unreadable",
    };
    let message = format!("The macOS app's pane shortcuts were not brought across: {refused}");
    crate::diagnostic!(serde_json::json!({
        "component": "ui_state",
        "kind": "ui_state.shortcut_import_failed",
        "message": message,
        "fallback": "defaults"
    }));
    snapshot.status.diagnostics.push(DiagnosticSnapshot {
        kind: "ui_state.shortcut_import_failed".to_owned(),
        message,
        occurred_at: unix_milliseconds(),
    });
    false
}

/// A mark no other connection has written: this process's start, its id and
/// how many marks it has made. Two hided processes never share a pid at the
/// same millisecond, and one never makes the same count twice.
fn read_mark(made: u64) -> String {
    format!(
        "{:x}-{:x}-{made:x}",
        unix_milliseconds(),
        std::process::id()
    )
}

fn unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.cancel_memory_pass();
        self.input_requests.close();
        for control in self.input_requests.take_controls() {
            self.terminals.control(control);
        }
    }
}

#[cfg(test)]
mod tests;
