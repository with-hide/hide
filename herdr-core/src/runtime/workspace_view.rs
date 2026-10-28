//! The runtime side of per-Workspace presentation (PRD S6 D-04, D-05, D-08,
//! D-10; S7; PRD right-lane D-01, D-03, D-05, D-11, D-13; PRD
//! lane-two-columns D-01, D-05, D-06): the front Workspace's right lane (its
//! viewing and tool columns, pin and four widths), its View area
//! tree, when that tree's documents are read back, and the file that keeps
//! it all. What the areas hold and how an open lands in them is
//! `view_areas.rs`.
//!
//! Everything here is inert unless the shell passed
//! `CoreOptions::workspace_views_path`; without it the rule is that a
//! document takes the terminal canvas and a terminal takes it back.

use std::collections::VecDeque;

use serde::Deserialize;

use super::view_areas::Reconciled;
use super::*;
use crate::model::{
    BrowserAreaScopeRow, BrowserViewInventoryRow, LaneCallSide, WorkspaceViewSnapshot,
};
use crate::view_layout::DisplayKind;
use crate::workspace_views::{self, Tool, WorkspaceView, WorkspaceViews};

/// A Workspace's identity: the device and the checkout path.
pub(super) type WorkspaceKey = (String, String);

/// How long a device choice the device accepted waits for its front to land.
/// The landing is the device's Herdr moving its focus and the device then
/// publishing its session over its link, which over a slow network can take
/// longer than the answer did; a click that outlives this does nothing, and
/// only the log says so (`workspace.choice_expired`).
pub(super) const CHOICE_LANDING_TIMEOUT_MS: u64 = 15_000;

/// A device request that chooses a Workspace, until the device's front
/// lands there. It lives as long as the request does and no longer, so what
/// rides on it (a Memory visit, a reveal) never runs for a click the device
/// let go of: a refusal or a lost answer, a later choice, the device
/// leaving, the device's front moving to a third checkout, or its deadline
/// each drop it.
enum TerminalChoice {
    Awaiting,
    Created(String),
}

struct PendingChoice {
    device_id: String,
    request_id: String,
    key: WorkspaceKey,
    /// The checkout the device had in front when the request left; a move
    /// to any other than this one or `key` is a choice made elsewhere.
    device_front: Option<String>,
    /// The answer gets `CLOSE_STAGE_TIMEOUT_MS`, as a Herdr-owned mutation's
    /// does (PRD D-08), and the front landing after an acceptance
    /// `CHOICE_LANDING_TIMEOUT_MS`.
    deadline_at_unix_ms: u64,
    memory_project: Option<String>,
    acknowledged: bool,
    /// Core checkout navigation has already landed, without a Herdr effect.
    core_choice: bool,
    /// A path the operator clicked in the chosen checkout, revealed once it
    /// is in front (`Runtime::reveal_when_in_front`).
    reveal: Option<super::RevealPathPayload>,
    terminal: Option<TerminalChoice>,
}

impl PendingChoice {
    fn carries(&self) -> &'static str {
        match (&self.memory_project, &self.reveal) {
            (Some(_), _) => "memory",
            (None, Some(_)) => "reveal",
            (None, None) => "choice",
        }
    }
}

pub(super) struct WorkspaceViewStore {
    pub(super) path: PathBuf,
    pub(super) views: WorkspaceViews,
    /// Workspaces whose displays were read back in this process. A document
    /// of one that no display shows gets a display; any other Workspace keeps
    /// the tree the file carried until it is shown.
    pub(super) live: HashSet<WorkspaceKey>,
    pub(super) agent_live: HashSet<WorkspaceKey>,
    pub(super) agent_admissions: BTreeMap<WorkspaceKey, HashSet<String>>,
    pub(super) agent_placements:
        BTreeMap<String, (WorkspaceKey, Option<String>, String, Option<usize>)>,
    /// The Workspace the last sync saw in front.
    pub(super) front: Option<WorkspaceKey>,
    /// Explicit checkout choices, independent of a remote Herdr's PTY focus.
    /// Rebuilt from the existing per-checkout last-used stamps on reload.
    pub(super) remote_front: HashMap<String, String>,
    /// One bounded acknowledgement slot for the last lane or tree width
    /// intent.
    /// It is published only for its Workspace and never saved.
    width_request: Option<(WorkspaceKey, String)>,
    /// Counts the layout changes made outside the reconcile, so an unchanged
    /// state costs the reconcile one comparison.
    pub(super) generation: u64,
    /// The live catalog identities behind the browser inventory, at most
    /// MAX_WORKSPACES saved layouts. A removed or disconnected checkout
    /// keeps its saved layout but cannot keep a native page eligible.
    browser_inventory_scope: Vec<WorkspaceKey>,
    /// What the last reconcile saw (`view_areas.rs`).
    pub(super) reconciled: Option<Reconciled>,
    /// The editor's active tab as the last reconcile set it; any other value
    /// was set by an event, which the reconcile shows.
    pub(super) derived_active: Option<String>,
    /// The last split request ids per Workspace, so a split sent twice
    /// splits once. Runtime only.
    pub(super) split_requests: HashMap<WorkspaceKey, VecDeque<String>>,
    /// Editor tabs the display cap keeps off screen, already reported.
    pub(super) unshown: HashSet<String>,
    /// The last load stamp a browser display was given (`next_browser_load`).
    /// Runtime only.
    pub(super) browser_load: u64,
    /// Numbers the intents that opened the lane, whichever page, CLI open
    /// from the operator's pane, event or rail press asked, and remembers
    /// the number of each Workspace's last call. A page shows an unpinned lane over the
    /// agents when the front Workspace's number passes the last one it read
    /// (PRD right-lane D-02, B16, B17), so a call into another Workspace
    /// never opens the front one's lane, and hiding the lane again is the
    /// page's own moment. Runtime only: whether the lane floats now is never
    /// stored (D-11).
    pub(super) lane_calls: u64,
    pub(super) lane_called: HashMap<WorkspaceKey, (u64, LaneCallSide)>,
    /// The Workspace the operator last chose, in this process or before it
    /// started: the one the app opens on (D-11); none on a first run.
    resumable: Option<WorkspaceKey>,
    /// A device Workspace the operator asked for, chosen once it is in front,
    /// with the device and request that asked, so a refusal drops it.
    pending_choice: Option<PendingChoice>,
    /// The right panel the settings file held at start. The snapshot's panel
    /// follows the front Workspace's tools, a projection the settings file
    /// never takes (D-10).
    saved_panel: (bool, RightPanelSection),
    /// Set when an unreadable file could not be moved aside: nothing is ever
    /// written over it.
    frozen: bool,
    save_pending: bool,
    save_active: bool,
    save_worker: Option<thread::JoinHandle<()>>,
}

/// What an event asks of a Workspace's lane once it has moved the screen
/// (PRD right-lane D-13, B16; lane-two-columns B6, B8): a document, diff or
/// page opened or focused shows the documents in the front Workspace's
/// viewing column and numbers that call, and so does a file a terminal link
/// names, applied to its own checkout's Workspace when it settles
/// (`settle_reveal_path`); the tool column is left as it is (D-06). A folder
/// a link names shows in the Explorer, so it opens the tool column on it and
/// leaves the viewing column. The Explorer's own reveal is a `workspace_view`
/// `reveal`, not an intent. Choosing an agent or a tab moves no lane state
/// the core keeps (B15); nothing else opens the lane on its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AreaIntent {
    Documents,
    /// A folder a link names: the Explorer tool, where it is shown.
    Explorer,
}

impl AreaIntent {
    pub(super) fn of(event: &Event) -> Option<Self> {
        match event {
            Event::FileOpen(_) | Event::FileFocus(_) => Some(Self::Documents),
            // A deselect opens nothing.
            Event::ChangesSelect(payload) if payload.path.is_some() => Some(Self::Documents),
            _ => None,
        }
    }
}

/// The payload of `workspace_view`: any subset of the front Workspace's
/// lane, each field the value it should end at, so the same event sent
/// twice lands where it did once (PRD right-lane D-12, B20). Absent fields
/// keep their value.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceViewPayload {
    /// `true` opens a closed lane in the shape it had, or on the documents
    /// when it has some and else on the Explorer when it had no column
    /// (B15); `false` closes the whole lane, keeping both columns' shape
    /// for the next open. The 보기 slot stops being drawn either way.
    #[serde(default)]
    pub(super) open: Option<bool>,
    /// `true` shows the checkout's documents in the viewing column, in front
    /// of the 보기 slot, which stays on the rail (B10, B11); `false` folds
    /// the viewing column.
    #[serde(default)]
    pub(super) documents: Option<bool>,
    /// `project`, `memory`, `explorer` or `changes` shows that tool in the tool
    /// column, a call the page shows even when the tool was already there;
    /// `null` closes the tool column alone (B14). The viewing column is
    /// left as it is.
    #[serde(default, deserialize_with = "present")]
    pub(super) tool: Option<Option<String>>,
    /// Whether the lane is pinned beside Agent Views (right-lane D-03).
    #[serde(default)]
    pub(super) pinned: Option<bool>,
    /// The viewing column's width with the documents, in CSS pixels.
    #[serde(default)]
    pub(super) document_width: Option<u32>,
    /// The viewing column's width with the 보기 slot, in CSS pixels.
    #[serde(default)]
    pub(super) detail_width: Option<u32>,
    /// The tool column's width with the Explorer, in CSS pixels.
    #[serde(default)]
    pub(super) explorer_width: Option<u32>,
    /// The tool column's width with the Project or Changes, in CSS pixels.
    #[serde(default)]
    pub(super) panel_width: Option<u32>,
    /// An optional opaque identity for a width intent, at most 128 bytes.
    #[serde(default)]
    pub(super) width_request_id: Option<String>,
    /// A file of the front Workspace to reveal: the tool column shows the
    /// Explorer with the file's folders unfolded, in the same event, and
    /// nothing is opened (B8); the viewing column stays.
    #[serde(default)]
    pub(super) reveal: Option<String>,
}

/// A field that is present, `null` included, as opposed to absent.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl WorkspaceViewStore {
    /// Numbers a call that opened `key`'s lane and records which column it
    /// brought forward. The call history has the store's bound; it is
    /// pruned only when crossing it, never on a repeat.
    pub(super) fn call_lane(&mut self, key: &WorkspaceKey, side: LaneCallSide) {
        self.lane_calls += 1;
        self.lane_called
            .insert(key.clone(), (self.lane_calls, side));
        if self.lane_called.len() > workspace_views::MAX_WORKSPACES {
            let views = &self.views;
            self.lane_called
                .retain(|(device, path), _| views.get(device, path).is_some());
        }
    }

    /// `saved_panel` is the right panel the older settings file holds, which
    /// is written back unchanged (D-10).
    /// The diagnostics say what the load had to do: move an unreadable file
    /// aside, migrate an older one, or repair a tree past its invariants.
    pub(super) fn open(
        path: PathBuf,
        saved_panel: (bool, RightPanelSection),
    ) -> (Self, Vec<(&'static str, String)>) {
        let (views, outcome) = workspace_views::load(&path, unix_milliseconds());
        let mut diagnostics = Vec::new();
        let frozen = match outcome {
            workspace_views::LoadOutcome::Missing => false,
            workspace_views::LoadOutcome::Loaded {
                migrated_from,
                repairs,
            } => {
                if let Some(version) = migrated_from {
                    diagnostics.push((
                        "workspace_views.migrated",
                        format!(
                            "Workspace view state was migrated from schema {version} to {}; each Workspace keeps its Views, Agent layout, groups and comments, and its lane starts closed at the default widths",
                            workspace_views::SCHEMA_VERSION
                        ),
                    ));
                }
                if !repairs.is_empty() {
                    diagnostics.push((
                        "workspace_views.repaired",
                        format!(
                            "Workspace view state was repaired on load: {}",
                            repairs.join("; ")
                        ),
                    ));
                }
                false
            }
            workspace_views::LoadOutcome::Unreadable {
                reason,
                preserved_as,
            } => match preserved_as {
                Some(preserved) => {
                    diagnostics.push((
                        "workspace_views.unreadable",
                        format!(
                            "Workspace view state could not be read ({reason}); the original was kept at {} and defaults were loaded",
                            preserved.display()
                        ),
                    ));
                    false
                }
                None => {
                    diagnostics.push((
                        "workspace_views.unreadable",
                        format!(
                            "Workspace view state could not be read ({reason}) and could not be moved aside; defaults are used and nothing will be written over it"
                        ),
                    ));
                    true
                }
            },
        };
        let mut remote_front = HashMap::new();
        let mut chosen = views
            .workspaces
            .iter()
            .filter(|view| view.last_used_unix_ms > 0)
            .collect::<Vec<_>>();
        chosen.sort_by_key(|view| view.last_used_unix_ms);
        for view in chosen {
            remote_front.insert(view.device_id.clone(), view.path.clone());
        }
        let resumable = views
            .workspaces
            .iter()
            .filter(|view| view.last_used_unix_ms > 0)
            .max_by_key(|view| view.last_used_unix_ms)
            .map(|view| (view.device_id.clone(), view.path.clone()));
        (
            Self {
                path,
                views,
                live: HashSet::new(),
                agent_live: HashSet::new(),
                agent_admissions: BTreeMap::new(),
                agent_placements: BTreeMap::new(),
                front: None,
                remote_front,
                width_request: None,
                generation: 0,
                browser_inventory_scope: Vec::new(),
                reconciled: None,
                derived_active: None,
                split_requests: HashMap::new(),
                browser_load: 0,
                lane_calls: 0,
                lane_called: HashMap::new(),
                unshown: HashSet::new(),
                resumable,
                pending_choice: None,
                saved_panel,
                frozen,
                save_pending: false,
                save_active: false,
                save_worker: None,
            },
            diagnostics,
        )
    }
}

impl Runtime {
    /// Catalog comparisons include the same core-owned group projection as
    /// the published frame, so rebuilding identical topology stays unchanged.
    pub(super) fn project_catalog_agent_groups(&self, workspaces: &mut [WorkspaceSnapshot]) {
        let Some(store) = &self.workspace_views else {
            return;
        };
        for workspace in workspaces {
            for checkout in &mut workspace.checkouts {
                let view = store.views.get(&workspace.device_id, &checkout.path);
                if let Some(view) = view {
                    checkout.tabs.retain(|tab| {
                        !super::group_starts::staged_tab(
                            view,
                            workspace.device_id == self.node.as_str(),
                            &workspace.device_id,
                            checkout.owner_workspace_id.as_deref(),
                            tab,
                        )
                    });
                    checkout.has_panes = checkout.tabs.iter().any(|tab| !tab.panes.is_empty());
                }
                checkout.agent_groups = view.map(|view| view.agent_groups_snapshot());
            }
        }
    }

    pub(super) fn separate_view_areas(&self) -> bool {
        self.workspace_views.is_some()
    }

    /// The device and path of a catalog checkout.
    pub(super) fn workspace_key(
        &self,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Option<WorkspaceKey> {
        self.catalog_checkout(workspace_id, checkout_id)
            .map(|(workspace, checkout)| (workspace.device_id.clone(), checkout.path.clone()))
    }

    /// The operator closed a view of `key`'s documents (lane-two-columns
    /// B12): when none is left the viewing column closes, and the lane with
    /// it when there is no tool column either; a tool column stays. The
    /// displays of a document that just closed leave first.
    pub(super) fn close_emptied_lane(&mut self, key: &WorkspaceKey) {
        self.reconcile_view_displays();
        let Some(entry) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
        else {
            return;
        };
        if !entry.documents || entry.layout.display_count() > 0 {
            return;
        }
        entry.documents = false;
        self.persist_workspace_views();
    }

    pub(super) fn front_workspace_key(&self) -> Option<WorkspaceKey> {
        let (workspace_id, checkout_id) = self.front_checkout()?;
        self.workspace_key(workspace_id, checkout_id)
    }

    /// Resolves a durable device/path identity without requiring a live tab.
    pub(super) fn agent_workspace_ids(&self, key: &WorkspaceKey) -> Option<(String, String)> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == key.0)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == key.1)
                    .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
            })
    }

    /// Chooses a checkout in the UI. An empty remote checkout creates no PTY.
    pub(super) fn choose_agent_workspace(&mut self, key: &WorkspaceKey) -> bool {
        let Some((workspace, checkout)) = self.agent_workspace_ids(key) else {
            self.set_error(
                "agent_group.workspace_unavailable",
                "This checkout is no longer available. Select it again.",
                false,
            );
            return false;
        };
        if key.0 == self.node.as_str() {
            self.focus_checkout(&workspace, &checkout);
        } else if let Some(store) = self.workspace_views.as_mut() {
            store.remote_front.insert(key.0.clone(), key.1.clone());
        } else {
            return false;
        }
        self.bring_device_forward(key.0.clone());
        self.mark_workspace_chosen(key);
        true
    }

    /// The terminal took the surface. Without View areas the document gives
    /// it back; with separate View areas the documents stay where they are.
    pub(super) fn yield_surface_to_terminal(&mut self) {
        if !self.separate_view_areas() {
            self.deactivate_editor_tab();
        }
    }

    /// The operator chose the Workspace in front: it is the one a reload or a
    /// restart opens on (D-11). A front that only Herdr's own focus moved is
    /// not, so a first run the operator spent on Main starts there again.
    pub(super) fn mark_front_chosen(&mut self) {
        if let Some(key) = self.front_workspace_key() {
            self.mark_workspace_chosen(&key);
        }
    }

    pub(super) fn mark_workspace_chosen(&mut self, key: &WorkspaceKey) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        store.pending_choice = None;
        // A click on a pane of the Workspace already chosen writes nothing.
        if store.resumable.as_ref() == Some(key) {
            return;
        }
        store.resumable = Some(key.clone());
        store.views.entry(&key.0, &key.1).last_used_unix_ms = unix_milliseconds();
        self.persist_workspace_views();
    }

    /// A device request that chooses a Workspace: the device's Herdr moves
    /// its front only when it accepts, so the choice is remembered when the
    /// sync sees the front land there, and a refusal remembers nothing.
    pub(super) fn choose_when_in_front(
        &mut self,
        device_id: &str,
        request_id: &str,
        key: WorkspaceKey,
    ) {
        let device_front = self.device_front_path(device_id);
        if let Some(store) = self.workspace_views.as_mut() {
            store.pending_choice = Some(PendingChoice {
                device_id: device_id.to_owned(),
                request_id: request_id.to_owned(),
                key,
                device_front,
                deadline_at_unix_ms: unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS),
                memory_project: None,
                acknowledged: false,
                core_choice: false,
                reveal: None,
                terminal: None,
            });
        }
    }

    pub(super) fn await_terminal_choice(&mut self, device: &str, request: &str) {
        if let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
            && pending.device_id == device
            && pending.request_id == request
        {
            pending.terminal = Some(TerminalChoice::Awaiting);
        }
    }

    pub(super) fn note_terminal_choice(&mut self, device: &str, request: &str, tab: String) {
        if let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
            && pending.device_id == device
            && pending.request_id == request
            && pending.terminal.is_some()
        {
            pending.terminal = Some(TerminalChoice::Created(tab));
        }
    }

    pub(super) fn pending_terminal_focus(&self) -> Option<(WorkspaceKey, String)> {
        let pending = self.workspace_views.as_ref()?.pending_choice.as_ref()?;
        match pending.terminal.as_ref()? {
            TerminalChoice::Created(tab) => Some((pending.key.clone(), tab.clone())),
            TerminalChoice::Awaiting => None,
        }
    }

    /// Keeps same-event Memory/reveal payloads correlated with a checkout
    /// choice that completed in core state, without waiting for Herdr focus.
    pub(super) fn remember_core_workspace_choice(
        &mut self,
        device: &str,
        request: &str,
        key: WorkspaceKey,
    ) {
        self.choose_when_in_front(device, request, key);
        if let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
        {
            pending.core_choice = true;
            pending.acknowledged = true;
        }
    }

    /// The path of the checkout a connected device's Herdr has in front.
    fn device_front_path(&self, device_id: &str) -> Option<String> {
        let session = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|status| status.target_id == device_id && status.state == "connected")?
            .session
            .as_ref()?;
        let focused = session.focused_checkout_id.as_deref()?;
        session
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.id == focused)
            .map(|checkout| checkout.path.clone())
    }

    /// A newer explicit visit supersedes the outstanding Memory visit;
    /// its late ACK or target arrival must no longer change the screen.
    pub(super) fn supersede_memory_navigation(&mut self) {
        if let Some(store) = self.workspace_views.as_mut()
            && store
                .pending_choice
                .as_ref()
                .is_some_and(|pending| pending.memory_project.is_some())
        {
            store.pending_choice = None;
        }
    }

    pub(super) fn attach_memory_navigation(
        &mut self,
        device: &str,
        request: &str,
        workspace: String,
    ) {
        if let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
            && pending.device_id == device
            && pending.request_id == request
        {
            pending.memory_project = Some(workspace);
        }
    }

    /// Keeps `reveal` on the device Workspace choice `request` made, to run
    /// once that Workspace is in front; false when no such choice waits.
    pub(super) fn attach_reveal(
        &mut self,
        device: &str,
        request: &str,
        reveal: super::RevealPathPayload,
    ) -> bool {
        let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
            .filter(|pending| pending.device_id == device && pending.request_id == request)
        else {
            return false;
        };
        pending.reveal = Some(reveal);
        true
    }

    /// The device accepted the request that made the choice: the front
    /// landing gets its own stage, and a Memory visit may now go ahead.
    pub(super) fn acknowledge_choice(&mut self, device: &str, request: &str) {
        if let Some(pending) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.pending_choice.as_mut())
            && pending.device_id == device
            && pending.request_id == request
        {
            pending.acknowledged = true;
            pending.deadline_at_unix_ms =
                unix_milliseconds().saturating_add(CHOICE_LANDING_TIMEOUT_MS);
            if pending.memory_project.is_some() {
                self.sync_workspace_view();
            }
        }
    }

    /// A choice whose request outlived its stage is dropped: the device
    /// never answered, or its front never landed, so a later visit to the
    /// same checkout is the operator's own and runs nothing for the click.
    pub(super) fn expire_pending_choice(&mut self, now_unix_ms: u64) -> bool {
        let Some(pending) = self.workspace_views.as_mut().and_then(|store| {
            store
                .pending_choice
                .take_if(|pending| pending.deadline_at_unix_ms <= now_unix_ms)
        }) else {
            return false;
        };
        self.push_diagnostic(
            "workspace.choice_expired",
            format!(
                "The {} request {} on {} did not land within its deadline; nothing runs for it",
                pending.carries(),
                pending.request_id,
                pending.device_id
            ),
        );
        true
    }

    pub(super) fn show_project_memory_at(&mut self, key: &WorkspaceKey, workspace: String) {
        if let Some(store) = self.workspace_views.as_mut() {
            let entry = store.views.entry(&key.0, &key.1);
            let before = (entry.open, entry.documents, entry.tool);
            let opened = entry.show_tool(Tool::Memory);
            let changed = before != (entry.open, entry.documents, entry.tool);
            store.call_lane(key, LaneCallSide::Tool);
            if changed {
                self.persist_workspace_views();
            }
            if opened {
                self.leave_detail_at(key);
            }
        }
        self.open_project_memory(Some(workspace));
    }

    /// A tool opened `key`'s closed lane on itself alone (`open_on`), so the
    /// window's 보기 slot, which ⌘E's close left shown, stops being drawn
    /// there and stays on the rail to come back to. The slot is the
    /// window's, so only the lane in front gives way.
    fn leave_detail_at(&mut self, key: &WorkspaceKey) {
        if self.front_workspace_key().as_ref() == Some(key) {
            self.leave_detail();
        }
    }

    /// The device refused the request, or its answer was lost: a later move
    /// Herdr makes there on its own is not the operator's choice.
    pub(super) fn drop_pending_choice(&mut self, device_id: &str, request_id: &str) {
        if let Some(store) = self.workspace_views.as_mut()
            && store.pending_choice.as_ref().is_some_and(|pending| {
                pending.device_id == device_id && pending.request_id == request_id
            })
        {
            store.pending_choice = None;
        }
    }

    /// Applies what the event that just ran asks of the front Workspace.
    pub(super) fn apply_area_intent(&mut self, intent: AreaIntent) {
        if let Some(key) = self.front_workspace_key() {
            self.apply_area_intent_to(&key, intent);
        }
    }

    /// Applies an area intent to one Workspace, in front or about to be.
    pub(super) fn apply_area_intent_to(&mut self, key: &WorkspaceKey, intent: AreaIntent) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let before = (entry.open, entry.documents, entry.tool);
        let tool_opened = match intent {
            AreaIntent::Documents => {
                entry.show_documents();
                false
            }
            AreaIntent::Explorer => entry.show_tool(Tool::Explorer),
        };
        let changed = before != (entry.open, entry.documents, entry.tool);
        store.call_lane(
            key,
            match intent {
                AreaIntent::Documents => LaneCallSide::View,
                AreaIntent::Explorer => LaneCallSide::Tool,
            },
        );
        if changed {
            self.persist_workspace_views();
        }
        // A file or diff opened from the card shows as a document in front
        // of the card, which keeps its rail slot to come back to (PRD
        // sidebar-task-lens B49, lane-two-columns B10). The slot is the
        // window's, so only the lane in front gives way: an open in a
        // Workspace behind it leaves the card or record the operator reads.
        if intent == AreaIntent::Documents || tool_opened {
            self.leave_detail_at(key);
        }
    }

    pub(super) fn apply_workspace_view(&mut self, payload: WorkspaceViewPayload) -> bool {
        if !self.separate_view_areas() {
            self.set_error(
                "workspace_view.unsupported",
                "This shell does not draw separate Agent and View areas",
                false,
            );
            return true;
        }
        if payload
            .width_request_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            self.set_error(
                "workspace_view.invalid_width_request",
                "The lane width request identity must contain 1 to 128 bytes",
                false,
            );
            return true;
        }
        let tool = match payload.tool.as_ref() {
            None => None,
            Some(None) => Some(None),
            Some(Some(value)) => match Tool::parse(value) {
                Some(tool) => Some(Some(tool)),
                None => {
                    self.set_error(
                        "workspace_view.unknown_tool",
                        format!(
                            "{value} is not a lane tool; expected project, memory, explorer or changes"
                        ),
                        false,
                    );
                    return true;
                }
            },
        };
        let Some(key) = self.front_workspace_key() else {
            self.set_error(
                "workspace_view.no_workspace",
                "No Workspace is open, so there is no layout to change",
                false,
            );
            return true;
        };
        let root = key.1.trim_end_matches('/');
        if let Some(path) = payload.reveal.as_deref()
            && crate::files::path_inside_root(root, path, false).is_err()
        {
            self.set_error(
                "workspace_view.reveal_outside",
                format!("{path} is not in the Workspace at {root}; nothing was revealed"),
                false,
            );
            return true;
        }
        let detail_shown =
            self.snapshot.ui_state.detail.shown && !self.snapshot.ui_state.detail.items.is_empty();
        let store = self.workspace_views.as_mut().expect("checked above");
        let entry = store.views.entry(&key.0, &key.1);
        let before = entry.clone();
        // Whether a tool opened the closed lane on itself alone (`open_on`).
        let mut tool_opened = false;
        match tool {
            Some(Some(tool)) => tool_opened = entry.show_tool(tool),
            Some(None) => entry.close_tool(),
            None => {}
        }
        match payload.documents {
            Some(true) => entry.show_documents(),
            Some(false) => entry.documents = false,
            None => {}
        }
        if let Some(pinned) = payload.pinned {
            entry.pinned = pinned;
        }
        // A reveal is the Explorer's: the tool column shows it.
        if payload.reveal.is_some() {
            tool_opened |= entry.show_tool(Tool::Explorer);
        }
        match payload.open {
            Some(false) => entry.open = false,
            Some(true) => {
                entry.open = true;
                // A lane that had no column comes back on the documents when
                // there are some, else on the Explorer (B15); the window's
                // 보기 slot, still shown, is a column of its own.
                if !entry.has_column() && !detail_shown {
                    if entry.layout.display_count() > 0 {
                        entry.documents = true;
                    } else {
                        entry.tool = Some(Tool::Explorer);
                    }
                }
            }
            None => {}
        }
        if let Some(width) = payload.document_width {
            entry.document_width = Some(workspace_views::clamp_width(width));
        }
        if let Some(width) = payload.detail_width {
            entry.detail_width = Some(workspace_views::clamp_width(width));
        }
        if let Some(width) = payload.explorer_width {
            entry.explorer_width = Some(workspace_views::clamp_width(width));
        }
        if let Some(width) = payload.panel_width {
            entry.panel_width = Some(workspace_views::clamp_width(width));
        }
        let entry_changed = *entry != before;
        // Every open is a call, so a page shows a lane it had hidden even
        // when the lane already held that column.
        let called = matches!(tool, Some(Some(_)))
            || payload.documents == Some(true)
            || payload.open == Some(true)
            || payload.reveal.is_some();
        if called {
            let side = if matches!(tool, Some(Some(_))) || payload.reveal.is_some() {
                LaneCallSide::Tool
            } else {
                LaneCallSide::View
            };
            store.call_lane(&key, side);
        }
        // The documents brought in front stand before the 보기 slot, which
        // stays on the rail (B10, B11); a lane closed whole keeps the slot
        // shown, so ⌘E brings it back (B15), and a tool that opens it
        // instead stands alone (`leave_detail_at`).
        let leaves_detail = payload.documents == Some(true) || tool_opened;
        let width_request_changed = if payload.document_width.is_some()
            || payload.detail_width.is_some()
            || payload.explorer_width.is_some()
            || payload.panel_width.is_some()
        {
            let next = payload.width_request_id.map(|id| (key.clone(), id));
            let changed = store.width_request != next;
            store.width_request = next;
            changed
        } else {
            false
        };
        let revealed = payload
            .reveal
            .is_some_and(|path| self.unfold_to(&key, &path));
        let detail_left = leaves_detail && self.leave_detail();
        if !entry_changed && !called && !revealed && !width_request_changed && !detail_left {
            return false;
        }
        if entry_changed {
            self.persist_workspace_views();
        }
        self.sync_workspace_view();
        true
    }

    /// Unfolds the Explorer folders down to `path`, a file the caller found
    /// inside the Workspace `key`; the shell selects the row.
    fn unfold_to(&mut self, key: &WorkspaceKey, path: &str) -> bool {
        let root = key.1.trim_end_matches('/');
        let folders = reveal_expansion_paths(root, path, false);
        let expanded = self.expanded_paths_on(&key.0);
        let mut unfolded = false;
        for folder in folders {
            if !expanded.contains(&folder) {
                expanded.push(folder);
                unfolded = true;
            }
        }
        if unfolded {
            self.persist_current_ui_state();
        }
        unfolded
    }

    /// Brings the snapshot, the editor and the file in line with the front
    /// Workspace. It runs after every event that can move the screen and
    /// before every snapshot read, so a front moved by Herdr or by a device's
    /// own focus is followed too; terminal input and output and a document's
    /// keystrokes skip the pass after their event, so a keystroke costs the
    /// read's pass alone. With nothing changed it costs a catalog lookup of
    /// the front key, one comparison of the editor's tabs, a copy of the
    /// published scalars, and the front Workspace's tree built again: one map
    /// of the editor's tabs and one root check, then a map lookup per display,
    /// of which there are at most 64. While a device choice waits, finding
    /// that device's front adds one pass over its session's checkouts. The
    /// Agent tab bookmark pass adds a comparison of the shown tabs and area
    /// fronts against what it saw last, in a few small vectors and with no
    /// write.
    pub(super) fn sync_workspace_view(&mut self) {
        // The recent list follows the front checkout wherever the layout
        // store exists or not, and writes only when the front moved.
        self.track_recent_checkouts();
        let Some(store) = self.workspace_views.as_ref() else {
            return;
        };
        // The device's front moved to a checkout other than the one it had
        // and the one asked for: someone chose that one, so the request's
        // landing, should it still come, is no longer the operator's click.
        let elsewhere = store.pending_choice.as_ref().and_then(|pending| {
            if pending.core_choice {
                return None;
            }
            let front = self.device_front_path(&pending.device_id)?;
            (Some(&front) != pending.device_front.as_ref() && front != pending.key.1).then(|| {
                (
                    pending.carries(),
                    pending.request_id.clone(),
                    pending.device_id.clone(),
                )
            })
        });
        if let Some((carries, request, device)) = elsewhere {
            if let Some(store) = self.workspace_views.as_mut() {
                store.pending_choice = None;
            }
            self.push_diagnostic(
                "workspace.choice_dropped",
                format!("The {carries} request {request} on {device} was dropped: the device's front moved elsewhere"),
            );
        }
        let memory_device = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.pending_choice.as_ref())
            .filter(|pending| pending.memory_project.is_some() && pending.acknowledged)
            .filter(|pending| {
                pending.core_choice
                    || self.device_front_path(&pending.device_id).as_ref() == Some(&pending.key.1)
            })
            .map(|pending| pending.device_id.clone());
        if let Some(device) = memory_device
            && !self.device_in_front(&device)
        {
            self.bring_device_forward(device);
        }
        let front = self.front_workspace_key();
        let store = self.workspace_views.as_ref().expect("checked above");
        if front != store.front {
            let store = self.workspace_views.as_mut().expect("checked above");
            store.front = front.clone();
        }
        let store = self.workspace_views.as_ref().expect("checked above");
        if let Some(key) = front.as_ref()
            && self
                .workspace_views
                .as_ref()
                .and_then(|store| store.pending_choice.as_ref())
                .is_some_and(|pending| {
                    &pending.key == key
                        && (pending.memory_project.is_none() || pending.acknowledged)
                        && match pending.terminal.as_ref() {
                            None => true,
                            Some(TerminalChoice::Awaiting) => false,
                            Some(TerminalChoice::Created(tab)) => {
                                !store.agent_placements.contains_key(tab)
                                    && store
                                        .views
                                        .get(&key.0, &key.1)
                                        .is_some_and(|view| view.agent_tab_group(tab).is_some())
                            }
                        }
                })
        {
            let (memory_project, reveal) = self
                .workspace_views
                .as_mut()
                .and_then(|store| store.pending_choice.as_mut())
                .map(|pending| (pending.memory_project.take(), pending.reveal.take()))
                .unwrap_or_default();
            self.mark_workspace_chosen(key);
            if let Some(project) = memory_project {
                self.show_project_memory_at(key, project);
            }
            if let Some(reveal) = reveal {
                self.reveal_path(reveal);
            }
        }
        self.restore_front_when_ready();
        // The strip follows the session whose pane has the keyboard before
        // the editor's active tab is derived from it (PRD session-files).
        self.settle_view_sessions();
        self.reconcile_view_displays();
        // What is in front of the operator now is seen, and the rows follow
        // whatever the View lists did (PRD unseen-agent-views D-07, D-09).
        self.settle_unseen_views();
        // The open record's gallery follows the View lists (D-40).
        self.settle_record_views();
        self.sync_agent_selection();
        if let Some(store) = &self.workspace_views {
            for workspace in &mut self.snapshot.navigator.workspaces {
                for checkout in &mut workspace.checkouts {
                    checkout.agent_groups = store
                        .views
                        .get(&workspace.device_id, &checkout.path)
                        .map(|view| view.agent_groups_snapshot());
                }
            }
            for remote in &mut self.snapshot.status.remote {
                if let Some(session) = &mut remote.session {
                    for workspace in &mut session.workspaces {
                        for checkout in &mut workspace.checkouts {
                            checkout.agent_groups = store
                                .views
                                .get(&remote.target_id, &checkout.path)
                                .map(|view| view.agent_groups_snapshot());
                        }
                    }
                }
            }
        }
        self.refresh_browser_inventory_scope();
        if let Some(store) = self.workspace_views.as_ref()
            && self.snapshot.browser_views_revision != Some(store.generation)
        {
            // Retain authority across ordinary layout changes, but not
            // across a revoke/regrant coalesced into one outgoing frame.
            // This bounded map is rebuilt only on a changed generation.
            let incarnations: HashMap<_, _> = self
                .snapshot
                .browser_scopes
                .iter()
                .map(|scope| {
                    (
                        (
                            scope.device_id.as_str(),
                            scope.path.as_str(),
                            scope.area_id.as_str(),
                        ),
                        scope.incarnation,
                    )
                })
                .collect();
            let mut displays = Vec::new();
            let mut scopes = Vec::new();
            for view in store.views.workspaces.iter().filter(|view| {
                store
                    .browser_inventory_scope
                    .iter()
                    .any(|key| key.0 == view.device_id && key.1 == view.path)
            }) {
                for area in view.layout.areas() {
                    scopes.push(BrowserAreaScopeRow {
                        device_id: view.device_id.clone(),
                        path: view.path.clone(),
                        area_id: area.id.clone(),
                        incarnation: incarnations
                            .get(&(
                                view.device_id.as_str(),
                                view.path.as_str(),
                                area.id.as_str(),
                            ))
                            .copied()
                            .unwrap_or(store.generation),
                    });
                    // An outside HTML file is a page the host draws like any
                    // other, kept alive while it is hidden.
                    displays.extend(
                        area.displays
                            .iter()
                            .filter(|display| {
                                display.kind == DisplayKind::Browser
                                    || display.external.as_ref().is_some_and(|file| {
                                        file.kind == crate::view_layout::ExternalKind::Html
                                    })
                            })
                            .map(|display| BrowserViewInventoryRow {
                                device_id: view.device_id.clone(),
                                path: view.path.clone(),
                                area_id: area.id.clone(),
                                view_id: display.id.clone(),
                            }),
                    );
                }
            }
            // Every Workspace's, connected or not: a grant ends only when no
            // tab stands on it.
            self.snapshot.external_grants = store
                .views
                .workspaces
                .iter()
                .flat_map(|view| view.layout.displays())
                .filter_map(|display| display.external.as_ref().map(|file| file.grant.clone()))
                .collect();
            self.snapshot.browser_views = displays;
            self.snapshot.browser_scopes = scopes;
            self.snapshot.browser_views_revision = Some(store.generation);
        }
        self.publish_workspace_view(front.as_ref());
    }

    /// Records authority transitions before an asynchronous catalog/session
    /// update can be coalesced with a later regrant. This does not reconcile
    /// the front tree or publish another notification.
    pub(super) fn refresh_browser_inventory_scope(&mut self) -> bool {
        // Compare borrowed identities without allocating on unchanged input.
        let changed = self.workspace_views.as_ref().is_some_and(|store| {
            !self.browser_inventory_scope().eq(store
                .browser_inventory_scope
                .iter()
                .map(|key| (key.0.as_str(), key.1.as_str())))
        });
        if !changed {
            return false;
        }
        let scope: Vec<_> = self
            .browser_inventory_scope()
            .map(|(device, path)| (device.to_owned(), path.to_owned()))
            .collect();
        let allowed: HashSet<_> = scope
            .iter()
            .map(|key| (key.0.as_str(), key.1.as_str()))
            .collect();
        // Forget the old incarnation at the authority boundary, even if no
        // snapshot read occurs before the same checkout becomes live again.
        self.snapshot
            .browser_scopes
            .retain(|row| allowed.contains(&(row.device_id.as_str(), row.path.as_str())));
        let store = self
            .workspace_views
            .as_mut()
            .expect("scope requires a store");
        store.browser_inventory_scope = scope;
        store.generation += 1;
        true
    }

    /// Positive page authority follows the current connected catalog, never
    /// a saved layout alone. The iterator is bounded by MAX_WORKSPACES (256)
    /// and allocates only when its identity sequence changes.
    fn browser_inventory_scope(&self) -> impl Iterator<Item = (&str, &str)> {
        self.workspace_views
            .as_ref()
            .into_iter()
            .flat_map(|store| &store.views.workspaces)
            .filter(|view| {
                let connected = if view.device_id == self.node.as_str() {
                    self.snapshot.status.herdr.state == "connected"
                } else {
                    self.snapshot.status.remote.iter().any(|remote| {
                        remote.target_id == view.device_id && remote.state == "connected"
                    })
                };
                connected
                    && self.catalog_workspaces().any(|workspace| {
                        workspace.device_id == view.device_id
                            && workspace
                                .checkouts
                                .iter()
                                .any(|checkout| checkout.path == view.path)
                    })
            })
            .map(|view| (view.device_id.as_str(), view.path.as_str()))
    }

    /// Reads the front Workspace's displays back the first time in this
    /// process that they can be read (contract 5). The tree is published
    /// before that, its displays `waiting`; a display binds when its document
    /// lands, so a Workspace whose files cannot be reached yet keeps what it
    /// saved. On this machine that waits for the daemon to open the
    /// checkout's root: a read before it is refused, and would mark every
    /// restored file unavailable. Returns whether a restore began.
    pub(super) fn restore_front_when_ready(&mut self) -> bool {
        let Some(store) = self.workspace_views.as_ref() else {
            return false;
        };
        // The restore opens into the live front, so it runs only while that is
        // the Workspace the last sync saw; a front that moved since waits for
        // the sync, which records it first.
        let Some(key) = store.front.clone() else {
            return false;
        };
        if store.live.contains(&key)
            || self.front_workspace_key().as_ref() != Some(&key)
            || !self.view_root_ready(&key)
        {
            return false;
        }
        let store = self.workspace_views.as_mut().expect("checked above");
        store.live.insert(key.clone());
        // A live Workspace has an entry, so the reconcile gives a document
        // opened into it by any path a display.
        store.views.entry(&key.0, &key.1);
        store.generation += 1;
        self.restore_view_displays(&key);
        true
    }

    /// Whether a file of this Workspace can be read now. A device's files
    /// are read through its helper, so its Workspace waits for the helper
    /// rather than marking every file unavailable while it starts, and for
    /// its catalog, which moves the device's checkouts into their Projects: a
    /// display restored before that names a Workspace the device no longer
    /// shows. The helper and the catalog each ask again once they are ready.
    /// Only the daemon keeps separate View areas, and it pins roots after its
    /// first snapshot read: a read before that would go around the pinned
    /// root, so a local Workspace waits for it.
    fn view_root_ready(&self, key: &WorkspaceKey) -> bool {
        self.view_root_wait(key).is_none()
    }

    fn publish_workspace_view(&mut self, front: Option<&WorkspaceKey>) {
        let Some(store) = self.workspace_views.as_ref() else {
            return;
        };
        // This runs on every snapshot read: the scalars are copied and the
        // tree is built again (`view_layout_snapshot`).
        let fresh;
        let view = match front {
            Some((device, path)) => Some(match store.views.get(device, path) {
                Some(view) => view,
                None => {
                    fresh = WorkspaceView::new(device, path);
                    &fresh
                }
            }),
            None => None,
        };
        let published = front.zip(view).map(|(key, view)| {
            let layout = self.view_layout_snapshot(key, &view.layout);
            WorkspaceViewSnapshot {
                device_id: view.device_id.clone(),
                path: view.path.clone(),
                git: self.checkout_header_snapshot(key),
                open: view.open,
                documents: view.documents,
                tool: view.tool,
                pinned: view.pinned,
                document_width: view.document_width,
                detail_width: view.detail_width,
                explorer_width: view.explorer_width,
                panel_width: view.panel_width,
                width_request_id: store
                    .width_request
                    .as_ref()
                    .filter(|(workspace, _)| workspace == key)
                    .map(|(_, request)| request.clone()),
                lane_called: store.lane_called.get(key).map_or(0, |(call, _)| *call),
                lane_called_side: store
                    .lane_called
                    .get(key)
                    .map_or(LaneCallSide::View, |(_, side)| *side),
                lane_calls: store.lane_calls,
                resumed: Some(key) == store.resumable.as_ref(),
                // A pane's `N views` lists what its session opened, drawn in
                // the strip or not (B8).
                feedback: self.view_feedback_snapshot(
                    key,
                    &self.view_layout_projection(key, &view.layout, false),
                ),
                session_drafts: self.session_drafts(key, &view.layout),
                layout,
                agent_layout: self
                    .arranges_agent_areas(&key.0)
                    .then(|| self.agent_layout_snapshot(key, view.selected_agent_layout())),
            }
        });
        // The one global panel every existing reader gates on (the Changes
        // reader, the device Explorer watch, Memory) follows the front
        // Workspace's tool column, so those readers keep one owner (A5) and
        // read nothing while the lane is closed or has no tool column.
        if let Some(view) = view {
            let (visible, section) = match view.tool.filter(|_| view.open) {
                Some(Tool::Explorer) => (true, RightPanelSection::Explorer),
                Some(Tool::Changes) => (true, RightPanelSection::Changes),
                // The Project tool and Memory both project as `Memory`, which no
                // reader gates on; `Overview` would start the disk and Git reads.
                Some(Tool::Project | Tool::Memory) => (true, RightPanelSection::Memory),
                None => (false, self.snapshot.ui_state.right_panel_section),
            };
            self.snapshot.ui_state.right_panel_visible = visible;
            self.snapshot.ui_state.right_panel_section = section;
        }
        self.snapshot.workspace_view = published;
    }

    /// A View display's file that could not be read back after a restart,
    /// or retried: its tab says why, and a retry that fails again says why
    /// this time. A tab that holds the file is left as it is.
    pub(super) fn insert_unavailable_file_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
        reason: String,
    ) {
        let tab_id = self.new_file_tab_id(workspace_id, checkout_id, path);
        self.push_diagnostic(
            "workspace_views.tab_unavailable",
            format!("A View file could not be read: {reason}"),
        );
        if let Some(tab) = self
            .snapshot
            .editor
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        {
            if tab.unavailable_reason.is_some() {
                tab.unavailable_reason = Some(reason);
            }
            return;
        }
        self.place_editor_tab(EditorTabSnapshot {
            id: tab_id,
            workspace_id: workspace_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            path: path.to_owned(),
            label: file_label(path),
            kind: EditorTabKind::File,
            diff_committed: None,
            markdown_live: true,
            wrap: true,
            dirty: false,
            preview,
            unavailable_reason: Some(reason),
        });
    }

    /// Queues a write of the Workspace view file on its own worker, outside
    /// the runtime mutex; one pending flag coalesces newer state.
    pub(super) fn persist_workspace_views(&mut self) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        if store.frozen {
            return;
        }
        let Some(context) = self.worker_context.clone() else {
            let views = store.views.clone();
            let result = workspace_views::save(&store.path, &views);
            self.acknowledge_group_start_save(&views, &result);
            self.acknowledge_pane_move_save(&views, &result);
            if let Err(message) = result {
                self.set_error("workspace_views.save_failed", message, true);
            }
            return;
        };
        store.save_pending = true;
        if store.save_active {
            return;
        }
        store.save_active = true;
        let spawned = thread::Builder::new()
            .name("hide-workspace-views-save".into())
            .spawn(move || {
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                loop {
                    let (path, views) = {
                        let mut guard = runtime.lock().unwrap_or_else(|e| e.into_inner());
                        let Some(store) = guard.workspace_views.as_mut() else {
                            return;
                        };
                        if !store.save_pending {
                            store.save_active = false;
                            return;
                        }
                        store.save_pending = false;
                        (store.path.clone(), store.views.clone())
                    };
                    let result = workspace_views::save(&path, &views);
                    runtime
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .acknowledge_group_start_save(&views, &result);
                    runtime
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .acknowledge_pane_move_save(&views, &result);
                    if let Err(message) = result {
                        runtime.lock().unwrap_or_else(|e| e.into_inner()).set_error(
                            "workspace_views.save_failed",
                            message,
                            true,
                        );
                        context.notifier.notify();
                    }
                }
            });
        let store = self.workspace_views.as_mut().expect("checked above");
        match spawned {
            // A finished worker's handle is replaced; the one that may still
            // be writing is joined when the core is dropped.
            Ok(worker) => store.save_worker = Some(worker),
            Err(error) => {
                store.save_active = false;
                self.set_error(
                    "workspace_views.save_failed",
                    format!("Workspace view state save worker could not start: {error}"),
                    true,
                );
            }
        }
    }

    /// The UI state the settings file takes: the snapshot's, with the right
    /// panel the file already held while the panel is a projection.
    pub(super) fn ui_state_to_save(&self) -> UiStateSnapshot {
        let mut state = self.snapshot.ui_state.clone();
        for (id, pending) in &self.project_settings_pending {
            state
                .project_settings
                .insert(id.clone(), pending.record.clone());
        }
        state
            .resolved_sessions
            .extend(self.pending_session_resolutions.clone());
        for (pane, pending) in &self.pending_revisit {
            match pending {
                Some(mark) => state.revisit_sessions.insert(pane.clone(), mark.clone()),
                None => state.revisit_sessions.remove(pane),
            };
        }
        if let Some(store) = self.workspace_views.as_ref() {
            (state.right_panel_visible, state.right_panel_section) = store.saved_panel;
        }
        state
    }

    pub(crate) fn take_workspace_views_save_worker(&mut self) -> Option<thread::JoinHandle<()>> {
        self.workspace_views.as_mut()?.save_worker.take()
    }

    /// Forgets what Hide remembered about a removed device's Workspaces.
    pub(super) fn forget_device_views(&mut self, device_id: &str) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let before = store.views.workspaces.len();
        store
            .views
            .workspaces
            .retain(|view| view.device_id != device_id);
        store.live.retain(|(device, _)| device != device_id);
        store
            .agent_admissions
            .retain(|(device, _), _| device != device_id);
        store
            .agent_placements
            .retain(|_, (key, _, _, _)| key.0 != device_id);
        store.remote_front.remove(device_id);
        store
            .lane_called
            .retain(|(device, _), _| device != device_id);
        store
            .split_requests
            .retain(|(device, _), _| device != device_id);
        store.generation += 1;
        if store
            .pending_choice
            .as_ref()
            .is_some_and(|pending| pending.device_id == device_id)
        {
            store.pending_choice = None;
        }
        if store
            .front
            .as_ref()
            .is_some_and(|(device, _)| device == device_id)
        {
            store.front = None;
        }
        if store
            .resumable
            .as_ref()
            .is_some_and(|(device, _)| device == device_id)
        {
            store.resumable = None;
        }
        if store.views.workspaces.len() != before {
            self.persist_workspace_views();
        }
    }
}

/// The last name of a path, which may be any device's, read by names alone.
pub(super) fn file_label(path: &str) -> String {
    hide_platform::path::wire_name(path).to_owned()
}
