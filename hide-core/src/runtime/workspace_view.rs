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
use crate::workspace_views::Place;
use crate::workspace_views::{self, Tool, WorkspaceView, WorkspaceViews};

/// A Workspace's identity: the device and the checkout path.
pub(super) type WorkspaceKey = (String, String);

/// How long a device choice the device accepted waits for its front to land.
/// The landing is the device's engine moving its focus and the device then
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
    /// The answer gets `CLOSE_STAGE_TIMEOUT_MS`, as an engine-owned mutation's
    /// does (PRD D-08), and the front landing after an acceptance
    /// `CHOICE_LANDING_TIMEOUT_MS`.
    deadline_at_unix_ms: u64,
    memory_project: Option<String>,
    acknowledged: bool,
    /// Core checkout navigation has already landed, without an engine effect.
    core_choice: bool,
    /// What the operator's press shows in the chosen checkout, once it is
    /// in front (`Runtime::reveal_when_in_front`).
    reveal: Option<super::session::FrontReveal>,
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
    /// Explicit checkout choices, independent of a remote engine's PTY focus.
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
    /// The card whose data is being read: the tray in front's card tab in
    /// front, as `sync_card_reads` last saw it (PRD lane-desk-pair-view,
    /// conductor decision 3), and whether a press asked it to be read again.
    /// Runtime only.
    pub(super) card_read: Option<crate::model::DetailItem>,
    pub(super) card_read_again: bool,
    /// Pair view's left pane while the mode is on (PRD lane-desk-pair-view
    /// D-45): the pane the operator last went to, whose View an event put on
    /// its tab's desk. Runtime only: after a restart the front tab's
    /// keyboard pane stands for it (`Runtime::pair_left_of`).
    pub(super) pair_left: Option<super::pair_view::PairLeft>,
    /// The pane that held the keyboard when pair view was entered, which
    /// gets it back on leaving (B26). Runtime only.
    pub(super) pair_before: Option<(WorkspaceKey, String)>,
    /// The pane drawn alone for having no View of its own whose desk stands
    /// beside it all the same, since a View the operator opened came onto
    /// it (D-53, conductor decision on #2775 review 1). Runtime only, and
    /// gone when the mode moves to another pane or ends.
    pub(super) pair_stood: Option<super::pair_view::PairLeft>,
    /// When focus mode last came on, in the layouts' focus stamps' clock:
    /// a tab nobody's agent owns shows on the drawn desk's strip only once
    /// the operator brought it in front since then (D-60). Runtime only, so
    /// after a restart the mode reads as begun at the load.
    pub(super) pair_since: u64,
    /// The desks pair view drew since the mode last moved or ended, each
    /// with how it was before the mode first drew it, which it gets back
    /// when the mode leaves it (D-51); a desk the mode passed drawing its
    /// tab's usual screen is never here (D-45). Runtime only, and at most
    /// the desk drawn now and the one it came from: every move restores and
    /// drops the others.
    pub(super) pair_drawn: Vec<super::pair_view::PairDrawn>,
    /// The desk that floats with the tool now (PRD lane-desk-pair-view
    /// D-50): a View came onto that tab's desk while the lane showed a tool
    /// and not the desk, so the desk is drawn beside the tool over the
    /// agents until the tool closes, and then takes its place on the right.
    /// Every other shown desk stands. Runtime only, at most one, and gone
    /// in the view pass (`settle_workspace_view`) once the lane no longer
    /// draws both on the front tab.
    pub(super) tray_floats: Option<(WorkspaceKey, String)>,
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
    /// `true` opens a closed lane in the shape it had, or on the tray of the
    /// tab in front when it has tabs and else on the Explorer when it had no
    /// column (B15); `false` closes the whole lane, keeping both columns'
    /// shape for the next open (PRD lane-desk-pair-view B30).
    #[serde(default)]
    pub(super) open: Option<bool>,
    /// `true` shows the tray of the Agent tab in front (B14); `false` folds
    /// it.
    #[serde(default)]
    pub(super) documents: Option<bool>,
    /// `project`, `memory`, `explorer` or `changes` shows that tool in the tool
    /// column, a call the page shows even when the tool was already there;
    /// `null` closes the tool column alone (B14). The viewing column is
    /// left as it is.
    #[serde(default, deserialize_with = "present")]
    pub(super) tool: Option<Option<String>>,
    /// The tray's width, in CSS pixels.
    #[serde(default)]
    pub(super) document_width: Option<u32>,
    /// The tool column's width with the Explorer, in CSS pixels.
    #[serde(default)]
    pub(super) explorer_width: Option<u32>,
    /// The tool column's width with the Project or Changes, in CSS pixels.
    #[serde(default)]
    pub(super) panel_width: Option<u32>,
    /// An optional opaque identity for a width intent, at most 128 bytes.
    #[serde(default)]
    pub(super) width_request_id: Option<String>,
    /// Pair view's divider landing (D-47): the left pane's share of the
    /// body in thousandths, one value for the window's mode.
    #[serde(default)]
    pub(super) pair_ratio: Option<u16>,
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
    /// Whether `tab`'s desk of `key` floats with the tool now (D-50): a
    /// View came onto it while the lane drew a tool and not the desk, and
    /// the open lane still shows both. The one reading of the floating
    /// desk. Shown, not drawn: a file still loading leaves the desk with no
    /// View yet while the shell already draws it, and it floats then too.
    pub(super) fn tray_floats(&self, key: &WorkspaceKey, tab: &str) -> bool {
        self.tray_floats
            .as_ref()
            .is_some_and(|(at, at_tab)| at == key && at_tab == tab)
            && self.views.get(&key.0, &key.1).is_some_and(|view| {
                view.open && view.tool.is_some() && view.tray(tab).is_some_and(|tray| tray.shown)
            })
    }

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
                    // A schema 4 or 5 lane keeps its state; an older one
                    // starts from the default shape (`workspace_views`).
                    let lane = if version >= 4 {
                        "its Views move to the desk of the Agent tab it showed last, and its lane keeps whether it was open, its tool and its widths"
                    } else {
                        "its lane starts closed at the default widths"
                    };
                    diagnostics.push((
                        "workspace_views.migrated",
                        format!(
                            "Workspace view state was migrated from schema {version} to {}; each Workspace keeps its Views, Agent layout, groups and comments, and {lane}",
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
                card_read: None,
                card_read_again: false,
                pair_left: None,
                pair_before: None,
                pair_stood: None,
                pair_since: unix_milliseconds(),
                pair_drawn: Vec::new(),
                tray_floats: None,
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

    /// The operator closed a tab of the tray in front (PRD
    /// lane-desk-pair-view B11, B26): a pair view left with nothing to draw
    /// ends (D-46), and when no tab is left the tray folds; a tool column
    /// stays. The displays of a document that just closed leave first.
    pub(super) fn close_emptied_lane(&mut self, key: &WorkspaceKey) {
        self.reconcile_view_displays();
        if self.end_pair_view_if_undrawn() {
            return;
        }
        let tab = self.front_tab(key);
        let Some(tray) = self
            .workspace_views
            .as_mut()
            .and_then(|store| store.views.get_mut(&key.0, &key.1))
            .and_then(|entry| entry.tray_mut(&tab))
        else {
            return;
        };
        if tray.layout.display_count() > 0 {
            return;
        }
        if std::mem::take(&mut tray.shown) {
            self.persist_workspace_views();
        }
    }

    /// Gives the tray a migration or a launch could not yet name to the
    /// Agent tab `key` shows, once it shows one (PRD lane-desk-pair-view
    /// D-18): one lookup while there is none.
    fn adopt_pending_tray(&mut self, key: &WorkspaceKey) {
        let pending = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
            .is_some_and(|view| view.tray(workspace_views::PENDING_TRAY).is_some());
        if !pending {
            return;
        }
        let tab = self.front_tab(key);
        if tab == workspace_views::PENDING_TRAY {
            return;
        }
        let store = self.workspace_views.as_mut().expect("read above");
        if let Some(adopted) = store.views.entry(&key.0, &key.1).adopt_pending_tray(&tab) {
            store.generation += 1;
            crate::diagnostic!(serde_json::json!({
                "component": "view_layout", "kind": "tray.pending_adopted",
                "device": key.0, "tab": tab, "inherited": adopted.inherited,
                "shelved": adopted.shelved, "dropped": adopted.dropped,
            }));
            self.persist_workspace_views();
        }
    }

    /// Opens `key`'s lane on the tray at `place` when the operator opened a
    /// tab there: a closed lane opens on it alone (`open_on`), a folded
    /// tray unfolds, and the call is numbered as the tray's. A View on no
    /// tray opens no lane.
    pub(super) fn open_tray_for(&mut self, key: &WorkspaceKey, place: &Place) {
        let Place::Tray(tab) = place else {
            return;
        };
        // Whether the operator saw this desk drawn: beside the pane focus
        // mode draws, else the view the core last published, since the View
        // that calls this is already on the desk.
        let drawn = self.pair_desk_drawn(key, tab).unwrap_or_else(|| {
            self.snapshot.workspace_view.as_ref().is_some_and(|view| {
                view.device_id == key.0
                    && view.path == key.1
                    && view.open
                    && view.tray.tab_id == *tab
                    && view.tray.shown
                    && view.tray.count > 0
            })
        });
        self.stand_pair_desk(key, tab);
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let before = (
            entry.open,
            entry.tool,
            entry.tray(tab).is_some_and(|tray| tray.shown),
        );
        entry.show_tray(tab);
        let changed = before != (entry.open, entry.tool, true);
        // A View that comes onto a desk the operator did not see drawn while
        // a tool stays drawn floats with that tool (D-50); a desk already
        // drawn keeps standing, or floating, as it was.
        if entry.tool.is_some() && !drawn {
            store.tray_floats = Some((key.clone(), tab.clone()));
        }
        store.call_lane(key, LaneCallSide::View);
        if changed {
            self.persist_workspace_views();
        }
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
        let (changed, read) = self.change_look(|runtime| runtime.choose_agent_workspace_steps(key));
        changed || read
    }

    /// The steps of `choose_agent_workspace`, which runs them as one selection (`look.rs`).
    fn choose_agent_workspace_steps(&mut self, key: &WorkspaceKey) -> bool {
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
    /// restart opens on (D-11). A front that only the engine's own focus moved is
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

    /// A device request that chooses a Workspace: the device's engine moves
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
    /// choice that completed in core state, without waiting for engine focus.
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

    /// The path of the checkout a connected device's engine has in front.
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
        reveal: super::session::FrontReveal,
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

    /// Whether a press's reveal waits for `device`'s front to land
    /// (`reveal_when_in_front`).
    pub(super) fn reveal_waits_on(&self, device: &str) -> bool {
        self.workspace_views
            .as_ref()
            .and_then(|store| store.pending_choice.as_ref())
            .is_some_and(|pending| pending.device_id == device && pending.reveal.is_some())
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
        let tab = self.front_tab(key);
        if let Some(store) = self.workspace_views.as_mut() {
            let entry = store.views.entry(&key.0, &key.1);
            let before = entry.clone();
            entry.show_tool(Tool::Memory, Some(&tab));
            let changed = *entry != before;
            store.call_lane(key, LaneCallSide::Tool);
            if changed {
                self.persist_workspace_views();
            }
        }
        self.open_project_memory(Some(workspace));
    }

    /// The device refused the request, or its answer was lost: a later move
    /// the engine makes there on its own is not the operator's choice.
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

    /// Applies an area intent to one Workspace, in front or about to be:
    /// a View the operator opened shows the tray of the tab in front, a
    /// folder the Explorer.
    pub(super) fn apply_area_intent_to(&mut self, key: &WorkspaceKey, intent: AreaIntent) {
        match intent {
            AreaIntent::Documents => {
                let place = self.front_place(key);
                self.open_tray_for(key, &place);
            }
            AreaIntent::Explorer => {
                let tab = self.front_tab(key);
                let Some(store) = self.workspace_views.as_mut() else {
                    return;
                };
                let entry = store.views.entry(&key.0, &key.1);
                let before = (entry.open, entry.tool);
                entry.show_tool(Tool::Explorer, Some(&tab));
                let changed = before != (entry.open, entry.tool);
                store.call_lane(key, LaneCallSide::Tool);
                if changed {
                    self.persist_workspace_views();
                }
            }
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
        let tab = self.front_tab(&key);
        let store = self.workspace_views.as_mut().expect("checked above");
        let entry = store.views.entry(&key.0, &key.1);
        let before = entry.clone();
        match tool {
            Some(Some(tool)) => {
                entry.show_tool(tool, Some(&tab));
            }
            Some(None) => entry.close_tool(),
            None => {}
        }
        match payload.documents {
            Some(true) => entry.show_tray(&tab),
            Some(false) => {
                if let Some(tray) = entry.tray_mut(&tab) {
                    tray.shown = false;
                }
            }
            None => {}
        }
        // A reveal is the Explorer's: the tool column shows it.
        if payload.reveal.is_some() {
            entry.show_tool(Tool::Explorer, Some(&tab));
        }
        match payload.open {
            Some(false) => entry.open = false,
            Some(true) => {
                entry.open = true;
                // A lane that had no column comes back on the tray when it
                // has tabs, else on the Explorer (B15).
                if !entry.has_column(Some(&tab)) {
                    if entry
                        .tray(&tab)
                        .is_some_and(|tray| tray.layout.display_count() > 0)
                    {
                        entry.tray_entry(&tab).shown = true;
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
        if let Some(width) = payload.explorer_width {
            entry.explorer_width = Some(workspace_views::clamp_width(width));
        }
        if let Some(width) = payload.panel_width {
            entry.panel_width = Some(workspace_views::clamp_width(width));
        }
        let mut entry_changed = *entry != before;
        if let Some(ratio) = payload.pair_ratio.map(workspace_views::clamp_pair_ratio)
            && store.views.pair.ratio != Some(ratio)
        {
            store.views.pair.ratio = Some(ratio);
            entry_changed = true;
        }
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
        let width_request_changed = if payload.document_width.is_some()
            || payload.explorer_width.is_some()
            || payload.panel_width.is_some()
            || payload.pair_ratio.is_some()
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
        if !entry_changed && !called && !revealed && !width_request_changed {
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
    /// before every snapshot read, so a front moved by the engine or by a device's
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
    /// write. The pass is one selection (`look.rs`): the look it leaves is
    /// compared once with the look the rows were read with, and the rows are
    /// read again only when it moved; true when that changed a row.
    pub(super) fn sync_workspace_view(&mut self) -> bool {
        let ((), read) = self.change_look(Self::settle_workspace_view);
        read
    }

    /// The view pass itself (`sync_workspace_view`).
    fn settle_workspace_view(&mut self) {
        // The recent list follows the front checkout wherever the layout
        // store exists or not, and writes only when the front moved.
        self.track_recent_checkouts();
        let Some(store) = self.workspace_views.as_ref() else {
            return;
        };
        // The floating desk takes its place once its lane no longer draws it
        // beside a tool on the front tab: the tool closed (its ✕, Escape, a
        // press outside) or changed, the lane closed, the desk folded or
        // emptied, or another tab came to the front (D-50).
        if let Some((key, tab)) = store.tray_floats.clone()
            && (self.front_tab(&key) != tab || !store.tray_floats(&key, &tab))
        {
            self.workspace_views
                .as_mut()
                .expect("read above")
                .tray_floats = None;
        }
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
                self.reveal_in_checkout(reveal);
            }
        }
        self.restore_front_when_ready();
        // The tray a migration could not yet name goes to the tab the
        // Workspace shows once it shows one (PRD lane-desk-pair-view D-18).
        if let Some(key) = front.as_ref() {
            self.adopt_pending_tray(key);
        }
        self.reconcile_view_displays();
        // What is in front of the operator now is seen, and the rows follow
        // whatever the View lists did (PRD unseen-agent-views D-07, D-09).
        self.settle_unseen_views();
        // The open record's gallery follows the View lists (D-40).
        self.settle_record_views();
        // Only the card in front of the tray drawn is read (conductor
        // decision 3).
        self.sync_card_reads();
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
                // A page on no tray is still held, loaded off screen by the
                // host (PRD lane-desk-pair-view D-37, B32).
                for (place, layout) in view.layouts() {
                    let tray = match &place {
                        Place::Tray(tab) => Some(tab.clone()),
                        Place::Shelf => None,
                    };
                    for area in layout.areas() {
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
                                    tray: tray.clone(),
                                    url: (display.kind == DisplayKind::Browser)
                                        .then(|| display.url.clone())
                                        .flatten(),
                                    load: display.load,
                                }),
                        );
                    }
                }
            }
            // Every Workspace's, connected or not: a grant ends only when no
            // tab stands on it.
            self.snapshot.external_grants = store
                .views
                .workspaces
                .iter()
                .flat_map(|view| view.views())
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
                    self.snapshot.status.engine.state == "connected"
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
            let place = self.front_place(key);
            let Place::Tray(tab) = &place else {
                unreachable!("the place in front is a tray");
            };
            let empty = crate::view_layout::Layout::default();
            let tray = view.tray(tab);
            let tray_layout = tray.map_or(&empty, |tray| &tray.layout);
            let layout = self.view_layout_snapshot(key, tray_layout);
            let every = self.every_view_snapshot(key);
            // The pair's reads are taken once for the publish.
            let reads = self.pair_reads(key);
            let panes_with_views = self.panes_with_views(key, &reads);
            let pair = self.pair_left_of(key, tab).map(|pane| {
                let mut pair = crate::model::PairViewSnapshot {
                    tab_id: tab.clone(),
                    alone: !self.pair_desk_beside(key, tab, &pane, &reads),
                    pane_id: pane,
                    folded: Vec::new(),
                };
                if !pair.alone {
                    pair.folded = self.pair_folded(key, &pair, &layout.root);
                }
                pair
            });
            WorkspaceViewSnapshot {
                device_id: view.device_id.clone(),
                path: view.path.clone(),
                git: self.checkout_header_snapshot(key),
                open: view.open,
                tool: view.tool,
                document_width: view.document_width,
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
                tray: crate::model::TraySnapshot {
                    tab_id: if tab == crate::workspace_views::PENDING_TRAY {
                        String::new()
                    } else {
                        tab.clone()
                    },
                    shown: tray.is_some_and(|tray| tray.shown),
                    count: tray_layout.display_count(),
                    floats: store.tray_floats(key, tab),
                    pair,
                },
                pair_mode: crate::model::PairModeSnapshot {
                    on: store.views.pair.on,
                    ratio: store.views.pair.ratio,
                },
                panes_with_views,
                // A pane's `N views` lists what its session opened, on a
                // tray or not (PRD lane-desk-pair-view D-04, D-19).
                feedback: self.view_feedback_snapshot(key, &every),
                session_drafts: self.session_drafts(key),
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
                // The Project tool and Memory project as `Memory`, which no
                // reader gates on; `Overview` would start the disk and Git
                // reads.
                Some(Tool::Project | Tool::Memory) => (true, RightPanelSection::Memory),
                None => (false, self.snapshot.ui_state.right_panel_section),
            };
            self.snapshot.ui_state.right_panel_visible = visible;
            self.snapshot.ui_state.right_panel_section = section;
        }
        self.snapshot.workspace_view = published;
        // A pair drawn without the mode moving onto it (after a restart)
        // is recorded as it is now, alone or beside its desk.
        let drawn = self.snapshot.workspace_view.as_ref().and_then(|view| {
            let pair = view.tray.pair.as_ref()?;
            Some((
                (view.device_id.clone(), view.path.clone()),
                pair.tab_id.clone(),
            ))
        });
        if let Some((key, tab)) = drawn {
            self.note_pair_desk(&key, &tab);
        }
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
            device_id: Default::default(),
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
