//! The View areas of each Workspace in a shell that draws them (PRD S7 A2,
//! A3): where an opened document shows, which editor tab each display shows,
//! the operator's `view_layout` actions, and what each display can show now.
//!
//! The tree and its pure operations are `crate::view_layout`; the stored file
//! is `crate::workspace_views`. A display names its document by path, kind
//! and Changes group and binds at runtime to the editor tab that holds the
//! document (the S5.5 buffer), so several displays can show one buffer and a
//! restart binds them again. The reconcile after every event and before every
//! snapshot read keeps the two in step: a display whose tab closed goes, a
//! tab no display shows gets one, a dirty document is never a preview, and
//! the editor's active tab is the active area's active display.
//!
//! A display shows a document of its Workspace's checkout, except one a
//! terminal link opened there for a file of another registered checkout of
//! the same device (`file_open` `view`): that document stays its own
//! checkout's tab, read and saved there, and the display reads from the
//! checkout whose root holds its path (`display_source`).
//!
//! A browser display (issue 155) shows a page, not a document: it binds to
//! no tab, so the reconcile never removes or binds one, and its address and
//! title are what the desktop host reports of the page (`browser_state`).
//!
//! Inert without `CoreOptions::workspace_views_path`: a shell that passes
//! none keeps one canvas and one preview slot per checkout.

use serde::Deserialize;

use super::desk::views::is_desk_key;
use super::documents::OpenRequestFields;
use super::workspace_view::{AreaIntent, WorkspaceKey, file_label};
use super::*;
use crate::model::{
    BrowserOpenReceiptSnapshot, ViewAreaSnapshot, ViewDisplaySnapshot, ViewDisplayState,
    ViewLayoutSnapshot, ViewLimitsSnapshot, ViewNodeSnapshot, ViewSplitSnapshot,
};
use crate::view_layout::{
    Display, DisplayKind, Edge, ExternalFile, ExternalKind, Layout, LayoutError, MAX_SPLIT_DEPTH,
    MAX_VIEW_AREAS, MAX_VIEW_DISPLAYS, Node, ViewOrigin, browser_address, browser_title,
};
use crate::workspace_views::Place;

/// Split request ids remembered per Workspace, so a split sent twice splits
/// once (engineering principle 11). Runtime only.
const SPLIT_REQUESTS_KEPT: usize = 32;

/// `browser_open` receipts kept for the requests that named themselves; a
/// CLI waiting on one reads it within a frame or two of its event.
const BROWSER_OPEN_RECEIPTS_KEPT: usize = 8;
const BROWSER_PAGE_REPORTS_KEPT: usize = 128;

/// The most grants one `external_grants` may list. The desktop host holds far
/// fewer at once (its own cap); a longer list is not one of its answers.
const MAX_EXTERNAL_GRANTS_LISTED: usize = 256;

const UNLOADABLE_ADDRESS: &str = "A page opens from an http, https or file address, or about:blank";

/// Where Open to the side looks for an area next to the one in use.
const BESIDE_ORDER: [Edge; 4] = [Edge::Right, Edge::Left, Edge::Down, Edge::Up];

// Every View diff is taken in the one Changes read, one per area at most.
const _: () = assert!(MAX_VIEW_AREAS <= hide_node_link::git::MAX_DIFFS);

/// The payload of `view_layout`: one operator action on the View areas of
/// the Workspace the operator saw. Each is one event and one frame; a
/// refusal says why and changes nothing.
#[derive(Debug, Deserialize)]
pub(super) struct ViewLayoutPayload {
    /// The Workspace of the frame the action was taken on, as that frame's
    /// `workspace_view` named it. Display, area and split ids repeat across
    /// Workspaces and the front can move between that frame and this event,
    /// so an action is applied only while its Workspace is still in front.
    workspace: ViewWorkspace,
    #[serde(flatten)]
    action: ViewLayoutAction,
}

#[derive(Debug, Deserialize)]
pub(super) struct ViewWorkspace {
    pub(super) device_id: String,
    pub(super) path: String,
}

/// The payload of `browser_open` (issue 155): show `url` as a page in a View
/// area of a Workspace, which need not be the one in front: the one named,
/// else the one the pane `pane_id` works in (`hide browser open` from an
/// agent's pane), else the one in front. A Workspace already showing `url`
/// shows that page again, loaded again, rather than a second one. A page's
/// own new tab names that page (`beside_display`) and opens beside it, in
/// place of `area_id`.
#[derive(Debug, Deserialize)]
pub(super) struct BrowserOpenPayload {
    url: String,
    /// An operator's terminal link brings its owning checkout forward in the same event.
    #[serde(default)]
    reveal: bool,
    #[serde(default)]
    area_id: Option<String>,
    /// The page whose own new tab this is: the new page opens beside it, so
    /// the page that asked stays in view.
    #[serde(default)]
    beside_display: Option<String>,
    #[serde(default)]
    workspace: Option<ViewWorkspace>,
    #[serde(default)]
    pane_id: Option<String>,
    /// The agent pane whose terminal link the operator clicked: the page is
    /// that agent's (PRD view-opener-feedback D-02).
    #[serde(default)]
    opener_pane: Option<String>,
    /// Names the receipt in `status.browser_opens` the sender waits for.
    #[serde(default)]
    request_id: Option<String>,
}

/// The payload of `browser_state`: what a browser display's page says now,
/// its address after it navigated and its title, as the desktop host reports
/// it. It records; it never loads anything.
#[derive(Debug, Deserialize)]
pub(super) struct BrowserStatePayload {
    workspace: ViewWorkspace,
    display_id: String,
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    load: Option<u64>,
    #[serde(default)]
    loading: bool,
    #[serde(default)]
    failure: Option<String>,
    #[serde(default)]
    present: Option<bool>,
}

/// The payload of `external_open` (PRD outside-path-in-hide B1, B13): a file
/// outside every checkout that the desktop host granted its window, named by
/// that grant alone. It opens in the Workspace named, the one in front, or,
/// with none in front, in the Workspace of the pane whose terminal printed it,
/// brought forward (`reveal`). A grant already shown there is focused and
/// read again (B10).
#[derive(Debug, Deserialize)]
pub(super) struct ExternalOpenPayload {
    grant: String,
    name: String,
    kind: ExternalKind,
    #[serde(default)]
    workspace: Option<ViewWorkspace>,
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    reveal: bool,
    /// The pane whose terminal link was pressed: the View is that pane's,
    /// its agent's or, for a shell, its terminal's (D-54). Absent for an
    /// open nobody's link made.
    #[serde(default)]
    opener_pane: Option<String>,
    /// Names the receipt in `status.browser_opens` the sender waits for.
    #[serde(default)]
    request_id: Option<String>,
}

/// The payload of `external_grants`: every grant the desktop host holds now,
/// sent when its window attaches. An outside file's tab whose grant is not
/// among them has nothing behind it and goes (PRD outside-path-in-hide D-07).
#[derive(Debug, Deserialize)]
pub(super) struct ExternalGrantsPayload {
    live: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum ViewLayoutAction {
    Focus {
        display_id: String,
    },
    FocusArea {
        area_id: String,
    },
    /// `index` is the display's place after the move, so the same move sent
    /// twice changes nothing the second time.
    Move {
        display_id: String,
        area_id: String,
        index: usize,
    },
    Split {
        display_id: String,
        area_id: String,
        edge: Edge,
        request_id: String,
    },
    Resize {
        split_id: String,
        ratio: f32,
    },
    /// The save a dirty document's last display waits on before it closes,
    /// or `discard`, the operator's Don't Save; a last display of unsaved
    /// work with neither is refused.
    Close {
        display_id: String,
        #[serde(default)]
        pending_save: Option<FileSavePayload>,
        #[serde(default)]
        discard: bool,
    },
    KeepOpen {
        display_id: String,
    },
    Retry {
        display_id: String,
    },
    /// The operator typed an address into a browser display's toolbar.
    Navigate {
        display_id: String,
        url: String,
    },
}

/// What every display of one Workspace's tree reads to say its state.
struct DisplayFacts<'a> {
    /// The Workspace whose displays these are; a display waits on the read
    /// of the checkout that holds its file (`display_source`).
    key: WorkspaceKey,
    tabs: HashMap<&'a str, &'a EditorTabSnapshot>,
    /// Why the Workspace's root cannot be read yet.
    wait: Option<String>,
    /// The checkout's comments, which each display counts its own of.
    feedback: Option<&'a crate::view_feedback::Feedback>,
    /// The machine's live sessions by pane, which a record or plan card
    /// names its owner from (`owner_pane`); read on the first card only.
    sessions: std::cell::OnceCell<Vec<(String, crate::links::ledger::SessionIdentity)>>,
    /// The Workspace's machine's home as wire spellings, as named and as its
    /// filesystem names it, which a file's `home_relative` is read below.
    homes: Vec<String>,
}

/// Where a document being read shows once it lands: the Workspace, the tray
/// and the area the operator asked in, and how.
#[derive(Clone, Debug)]
pub(super) struct ViewPlacement {
    pub(super) key: WorkspaceKey,
    pub(super) place: Place,
    pub(super) area: String,
    pub(super) preview: bool,
    pub(super) beside: bool,
    pub(super) replace: Option<String>,
}

impl ViewPlacement {
    pub(super) fn same_destination(&self, other: &Self) -> bool {
        self.key == other.key
            && self.place == other.place
            && self.area == other.area
            && self.beside == other.beside
            && self.replace == other.replace
    }
}

/// What the last reconcile saw, so an unchanged editor costs a comparison.
pub(super) struct Reconciled {
    tabs: Vec<EditorTabSnapshot>,
    active: Option<String>,
    front: Option<WorkspaceKey>,
    /// The desk the front Workspace showed: a tab switch changes which tree
    /// the editor follows without moving the store's generation.
    desk: Option<Place>,
    generation: u64,
    /// Desk's desk's own count of changes (`Runtime::desk_views_generation`).
    desk_generation: u64,
}

/// What a display needs to know about an editor tab.
struct TabFacts {
    id: String,
    key: Option<WorkspaceKey>,
    path: String,
    kind: DisplayKind,
    committed: Option<bool>,
    /// Dirty, saving or holding a save's outcome: never a preview (B5).
    kept: bool,
}

fn display_kind(kind: EditorTabKind) -> Option<DisplayKind> {
    match kind {
        EditorTabKind::File => Some(DisplayKind::File),
        EditorTabKind::Diff => Some(DisplayKind::Diff),
        EditorTabKind::Session => None,
    }
}

/// The checkout, as its Workspace key, whose root holds the file `path` a
/// display of the Workspace `key` names: the Workspace's own for a path
/// under its root, else the registered checkout of the same device whose
/// root holds it, the deepest when roots nest. A path under no root stays
/// the Workspace's own, whose read then says why it cannot show.
fn source_key(key: &WorkspaceKey, path: &str, roots: &[WorkspaceKey]) -> WorkspaceKey {
    let holds = |root: &str| hide_platform::path::wire_relative(root, path).is_ok();
    if holds(&key.1) {
        return key.clone();
    }
    roots
        .iter()
        .filter(|root| root.0 == key.0 && holds(&root.1))
        .max_by_key(|root| root.1.len())
        .cloned()
        .unwrap_or_else(|| key.clone())
}

/// Which Workspaces' displays show each document, so a checkout whose
/// document another Workspace shows adds no display of its own for it.
#[derive(Default)]
struct ShownByViews {
    /// Each tab a display is bound to, with the Workspaces whose displays are.
    tabs: HashMap<String, Vec<WorkspaceKey>>,
    /// Each restored display of another checkout's file that waits for its
    /// tab, by that checkout and the path, with the Workspaces that show it.
    files: HashMap<(WorkspaceKey, String), Vec<WorkspaceKey>>,
}

impl ShownByViews {
    fn of<'v>(
        views: impl Iterator<Item = (WorkspaceKey, &'v Layout)>,
        roots: &[WorkspaceKey],
    ) -> Self {
        let mut shown = Self::default();
        for (key, layout) in views {
            for display in layout.displays() {
                if let Some(tab_id) = &display.tab_id {
                    shown
                        .tabs
                        .entry(tab_id.clone())
                        .or_default()
                        .push(key.clone());
                } else if display.kind == DisplayKind::File {
                    let source = display
                        .origin
                        .as_ref()
                        .map_or_else(|| source_key(&key, &display.path, roots), ViewOrigin::key);
                    if source != key {
                        let views = shown.files.entry((source, display.path.clone()));
                        views.or_default().push(key.clone());
                    }
                }
            }
        }
        shown
    }

    /// Whether a Workspace other than `key`, the tab's own, shows the tab.
    fn elsewhere(&self, tab: &TabFacts, key: &WorkspaceKey) -> bool {
        let other = |views: &Vec<WorkspaceKey>| views.iter().any(|view| view != key);
        self.tabs.get(&tab.id).is_some_and(other)
            || (tab.kind == DisplayKind::File
                && self
                    .files
                    .get(&(key.clone(), tab.path.clone()))
                    .is_some_and(other))
    }
}

/// The display that shows a document and was focused last.
fn last_focused_showing(
    displays: &[&Display],
    origin: Option<&ViewOrigin>,
    path: &str,
    kind: DisplayKind,
    committed: Option<bool>,
) -> Option<String> {
    displays
        .iter()
        .filter(|display| display.shows_from(origin, path, kind, committed))
        .max_by_key(|display| display.last_focused_unix_ms)
        .map(|display| display.id.clone())
}

/// The cap with the opens still being read counted, so reads in flight
/// cannot pass it together.
fn display_limit(shown: usize) -> Option<LayoutRefusal> {
    (shown >= MAX_VIEW_DISPLAYS).then(|| LayoutError::DisplayLimit.into())
}

/// A browser open refused for a reason other than the View layout's.
fn browser_refusal(message: impl Into<String>) -> LayoutRefusal {
    LayoutRefusal {
        kind: "browser.open_refused",
        message: message.into(),
    }
}

fn external_refusal(message: impl Into<String>) -> LayoutRefusal {
    LayoutRefusal {
        kind: "external.open_refused",
        message: message.into(),
    }
}

/// The View `make_view_room` takes off its tree.
struct OldestView {
    display_id: String,
    kind: DisplayKind,
    document: Option<String>,
    /// An agent opened it and the operator did not: off a desk it stays in
    /// that agent's pane list.
    agents: bool,
}

struct LayoutRefusal {
    kind: &'static str,
    message: String,
}

impl From<LayoutError> for LayoutRefusal {
    fn from(error: LayoutError) -> Self {
        Self {
            kind: error.kind(),
            message: error.message(),
        }
    }
}

/// Brings one tree of a Workspace in line with the editor's tabs. `live`
/// says the tree is the desk the Workspace shows, which takes a document no
/// tree of the checkout shows. Returns whether anything the file stores
/// changed, and the documents the display cap kept off screen.
#[allow(clippy::too_many_arguments)]
fn reconcile_layout<'t>(
    layout: &mut Layout,
    key: &WorkspaceKey,
    live: bool,
    owned: &HashSet<String>,
    tabs: &'t [TabFacts],
    roots: &[WorkspaceKey],
    requested: Option<&str>,
    now: u64,
) -> (bool, Vec<&'t TabFacts>) {
    let mut stored = false;
    let mut unshown = Vec::new();
    // A display whose document closed, by any path, goes with it.
    let gone: Vec<String> = layout
        .displays()
        .filter(|display| {
            display
                .tab_id
                .as_ref()
                .is_some_and(|id| !tabs.iter().any(|tab| &tab.id == id))
        })
        .map(|display| display.id.clone())
        .collect();
    for display_id in gone {
        layout.remove(&display_id);
        stored = true;
    }
    for display in layout.displays_mut() {
        let tab = match display
            .tab_id
            .as_ref()
            .and_then(|id| tabs.iter().find(|tab| &tab.id == id))
        {
            Some(tab) => tab,
            // A restored display binds to its document once it is read, the
            // tab of the checkout that holds its file.
            None => {
                let source = source_key(key, &display.path, roots);
                let Some(tab) = tabs.iter().find(|tab| {
                    tab.key.as_ref() == Some(&source)
                        && display.shows(&tab.path, tab.kind, tab.committed)
                }) else {
                    continue;
                };
                display.tab_id = Some(tab.id.clone());
                tab
            }
        };
        // A rename keeps the tab, and the display follows it.
        if display.path != tab.path {
            display.path = tab.path.clone();
            stored = true;
        }
        if tab.kept && display.preview {
            display.preview = false;
            stored = true;
        }
    }
    // A document of this Workspace that no display shows gets one, at the
    // end of the area in use: a Reopen Closed, a created file. The actions
    // that make one are refused at the cap; one that landed after the tree
    // filled waits off screen until a view is closed.
    if live {
        for tab in tabs
            .iter()
            .filter(|tab| tab.key.as_ref() == Some(key) && !owned.contains(&tab.id))
        {
            if layout
                .displays()
                .any(|display| display.tab_id.as_deref() == Some(tab.id.as_str()))
            {
                continue;
            }
            if layout.display_count() >= MAX_VIEW_DISPLAYS {
                unshown.push(tab);
                continue;
            }
            let mut display = layout.new_display(&tab.path, tab.kind, tab.committed, false);
            display.tab_id = Some(tab.id.clone());
            let area = layout.active_area().id.clone();
            if layout.append(&area, display).is_ok() {
                stored = true;
            }
        }
    }
    // A tab an event activated (File Focus, Reopen Closed) shows in the
    // display that showed it last.
    if let Some(requested) = requested {
        let bound: Vec<&Display> = layout
            .displays()
            .filter(|display| display.tab_id.as_deref() == Some(requested))
            .collect();
        if let Some(display_id) = bound
            .iter()
            .max_by_key(|display| display.last_focused_unix_ms)
            .map(|display| display.id.clone())
        {
            let stamp = layout.next_stamp(now);
            stored |= layout.focus(&display_id, stamp).unwrap_or(false);
        }
    }
    (stored, unshown)
}

/// Brings Desk's desk in line with the editor's tabs (PRD desk-own-desk
/// D-11): a file's View binds to the document of its own checkout once it is
/// read, follows a rename, stops being a preview once it holds work, and goes
/// when its document closed by any path, as a Workspace's View does. Desk's
/// desk never takes in a document no View shows. Returns whether anything
/// changed and whether the change is one the store keeps.
fn reconcile_desk_layout(layout: &mut Layout, tabs: &[TabFacts]) -> (bool, bool) {
    let gone: Vec<String> = layout
        .displays()
        .filter(|display| {
            display
                .tab_id
                .as_ref()
                .is_some_and(|id| !tabs.iter().any(|tab| &tab.id == id))
        })
        .map(|display| display.id.clone())
        .collect();
    let mut stored = !gone.is_empty();
    for id in &gone {
        layout.remove(id);
    }
    let mut changed = stored;
    for display in layout.displays_mut() {
        let Some(origin) = display.origin.as_ref().map(ViewOrigin::key) else {
            continue;
        };
        let tab = match display
            .tab_id
            .as_ref()
            .and_then(|id| tabs.iter().find(|tab| &tab.id == id))
        {
            Some(tab) => tab,
            None => {
                let Some(tab) = tabs.iter().find(|tab| {
                    tab.key.as_ref() == Some(&origin)
                        && display.shows(&tab.path, tab.kind, tab.committed)
                }) else {
                    continue;
                };
                display.tab_id = Some(tab.id.clone());
                changed = true;
                tab
            }
        };
        if display.path != tab.path {
            display.path = tab.path.clone();
            stored = true;
        }
        if tab.kept && display.preview {
            display.preview = false;
            stored = true;
        }
    }
    (changed || stored, stored)
}

impl Runtime {
    /// The desk the Workspace `key` shows: its Agent tab in front's, or the
    /// pending desk while it shows no tab (PRD lane-desk-pair-view D-06). An
    /// open the operator makes, and one the agent they watch makes, lands
    /// here. On this machine the tab is the one the core shows for the
    /// checkout, which a tab or pane choice moves on its own event, before
    /// the engine confirms it.
    pub(super) fn front_place(&self, key: &WorkspaceKey) -> Place {
        if is_desk_key(key) {
            return Place::Desk;
        }
        let shown = (key.0 == self.node.as_str())
            .then(|| self.agent_workspace_ids(key))
            .flatten()
            .and_then(|(_, checkout)| self.visible_tab_ids.get(&checkout).cloned());
        let catalog = || {
            self.catalog_workspaces()
                .filter(|workspace| workspace.device_id == key.0)
                .flat_map(|workspace| &workspace.checkouts)
                .find(|checkout| checkout.path == key.1)?
                .active_tab_id
                .clone()
        };
        Place::Tray(
            shown
                .or_else(|| self.workspace_active_tab(key))
                .or_else(catalog)
                .unwrap_or_else(|| crate::workspace_views::PENDING_TRAY.to_owned()),
        )
    }

    /// The tab of the desk the Workspace `key` shows (`front_place`): the
    /// pending desk's empty name while it shows no tab.
    pub(super) fn front_tab(&self, key: &WorkspaceKey) -> String {
        match self.front_place(key) {
            Place::Tray(tab) => tab,
            Place::Shelf | Place::Desk => unreachable!("a Workspace's front is its tab's desk"),
        }
    }

    /// The tree at `place` of the Workspace `key`.
    pub(super) fn view_layout_at(&self, key: &WorkspaceKey, place: &Place) -> Option<&Layout> {
        if *place == Place::Desk || is_desk_key(key) {
            return (*place == Place::Desk && is_desk_key(key))
                .then(|| self.desk_views())
                .flatten();
        }
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)?
            .layout(place)
    }

    /// The desk the Workspace `key` shows (`front_place`).
    pub(super) fn front_tray_layout(&self, key: &WorkspaceKey) -> Option<&Layout> {
        self.view_layout_at(key, &self.front_place(key))
    }

    /// Where the View `id` of the Workspace `key` sits.
    pub(super) fn view_place(&self, key: &WorkspaceKey, id: &str) -> Option<Place> {
        if is_desk_key(key) {
            return self.desk_views()?.display(id).map(|_| Place::Desk);
        }
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)?
            .place_of(id)
    }

    /// The View `id` of the Workspace `key`, on a desk or not.
    pub(super) fn view_display(&self, key: &WorkspaceKey, id: &str) -> Option<&Display> {
        if is_desk_key(key) {
            return self.desk_views()?.display(id);
        }
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)?
            .view(id)
    }

    /// Every View of the Workspace, on each desk and then on the shelf.
    pub(super) fn view_displays(&self, key: &WorkspaceKey) -> impl Iterator<Item = &Display> {
        let desk = is_desk_key(key).then(|| self.desk_views()).flatten();
        let workspace = self
            .workspace_views
            .as_ref()
            .filter(|_| !is_desk_key(key))
            .and_then(|store| store.views.get(&key.0, &key.1));
        desk.into_iter()
            .flat_map(Layout::displays)
            .chain(workspace.into_iter().flat_map(|view| view.views()))
    }

    /// Every tree of every Workspace, each with the Workspace and the place
    /// it is.
    /// Desk's desk is one of them, under its own key, while Desk is on.
    pub(super) fn every_view_layout(&self) -> impl Iterator<Item = (WorkspaceKey, Place, &Layout)> {
        let desk = self
            .desk_views()
            .map(|layout| (self.desk_key(), Place::Desk, layout));
        self.workspace_views
            .iter()
            .flat_map(|store| {
                store.views.workspaces.iter().flat_map(|view| {
                    view.layouts().map(|(place, layout)| {
                        ((view.device_id.clone(), view.path.clone()), place, layout)
                    })
                })
            })
            .chain(desk)
    }

    /// Every View of every Workspace, and of Desk's desk while Desk is on.
    pub(super) fn every_view(&self) -> impl Iterator<Item = (WorkspaceKey, &Display)> {
        let desk = self
            .desk_views()
            .into_iter()
            .flat_map(|layout| layout.displays().map(|display| (self.desk_key(), display)));
        self.workspace_views
            .iter()
            .flat_map(|store| {
                store.views.workspaces.iter().flat_map(|view| {
                    view.views()
                        .map(|display| ((view.device_id.clone(), view.path.clone()), display))
                })
            })
            .chain(desk)
    }

    /// Whether the Workspace of a checkout holds `display_id`, before a
    /// `focus_checkout` that names it moves anything.
    pub(super) fn view_display_known(
        &self,
        workspace_id: &str,
        checkout_id: &str,
        display_id: &str,
    ) -> Result<(), LayoutError> {
        if !self.separate_view_areas() {
            return Err(LayoutError::UnknownDisplay(display_id.to_owned()));
        }
        self.workspace_key(workspace_id, checkout_id)
            .and_then(|key| self.view_display(&key, display_id))
            .map(|_| ())
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))
    }

    /// A View of the checkout just brought forward, pressed by the operator
    /// (a pane's `N views` row, a record's tile, Recent Panels), comes to the
    /// desk the Workspace shows, in front, and its lane opens on it (PRD
    /// lane-desk-pair-view B3, D-20). Returns whether the layout changed.
    pub(super) fn focus_view_display_of(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        display_id: &str,
    ) -> bool {
        let Some(key) = self.workspace_key(workspace_id, checkout_id) else {
            return false;
        };
        let place = self.front_place(&key);
        let moved = self.bring_view_to(&key, display_id, &place);
        self.apply_area_intent_to(&key, AreaIntent::Documents);
        match self.change_view_layout(&key, &place, |layout, stamp| {
            layout
                .focus(display_id, stamp)
                .map(|changed| (changed, changed))
        }) {
            Ok(changed) => changed || moved,
            Err(error) => {
                self.set_error(error.kind(), error.message(), false);
                true
            }
        }
    }

    /// Whether the View `display_id` is a run's summary card, which is fixed
    /// on its run's desk: it is not closed (by the window or by `hide view
    /// close`) and does not go to another desk, and a refusal says which
    /// (`view_layout.run_summary_refused`). Moving it inside its desk and
    /// splitting it are the layout's own refusals (`AreaItem::pinned`).
    pub(super) fn refuse_fixed_view(
        &self,
        key: &WorkspaceKey,
        display_id: &str,
        action: &str,
    ) -> bool {
        let fixed = self
            .view_display(key, display_id)
            .is_some_and(|display| display.is_run_summary());
        if fixed {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "view_layout.run_summary_refused",
                "device": key.0,
                "display": display_id,
                "action": action,
            }));
        }
        fixed
    }

    /// Moves the View `id` of `key` onto `place` (D-20: one View, one desk),
    /// making room there first. Whether it moved.
    pub(super) fn bring_view_to(&mut self, key: &WorkspaceKey, id: &str, place: &Place) -> bool {
        let from = match self.view_place(key, id) {
            Some(from) if &from != place => from,
            _ => return false,
        };
        if self.refuse_fixed_view(key, id, "move") {
            return false;
        }
        if !self.make_view_room(key, place) {
            return false;
        }
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let moved = entry.move_view(id, place).is_some();
        if !moved {
            return false;
        }
        store.generation += 1;
        self.persist_workspace_views();
        // A pair view the move left with nothing to show stays on, its desk
        // settled as after a close (D-64).
        if matches!(from, Place::Tray(_)) {
            self.keep_pair_front_shown();
        }
        true
    }

    /// The area in use of the tree at `place`, the tree made first when the
    /// Workspace has none there yet: a desk's first area takes its number
    /// from the checkout's one count, so no fixed id names it.
    pub(super) fn tree_area(&mut self, key: &WorkspaceKey, place: &Place) -> String {
        if let Some(layout) = self.view_layout_at(key, place) {
            return layout.active_area().id.clone();
        }
        if *place == Place::Desk {
            return Layout::default().active_area;
        }
        let Some(store) = self.workspace_views.as_mut() else {
            return Layout::default().active_area;
        };
        let entry = store.views.entry(&key.0, &key.1);
        if let Place::Tray(tab) = place {
            entry.tray_entry(tab);
        }
        entry
            .layout(place)
            .map(|layout| layout.active_area().id.clone())
            .unwrap_or_else(|| Layout::default().active_area)
    }

    /// The desk the Workspace `key` shows, made when it has none yet.
    #[cfg(test)]
    pub(super) fn front_desk_mut(&mut self, key: &WorkspaceKey) -> &mut Layout {
        let tab = self.front_tab(key);
        &mut self
            .workspace_views
            .as_mut()
            .expect("a View store")
            .views
            .entry(&key.0, &key.1)
            .tray_entry(&tab)
            .layout
    }

    /// `change_view_layout` on the desk the Workspace `key` shows.
    #[cfg(test)]
    pub(super) fn change_front_layout<T>(
        &mut self,
        key: &WorkspaceKey,
        change: impl FnOnce(&mut Layout, u64) -> Result<(T, bool), LayoutError>,
    ) -> Result<T, LayoutError> {
        let place = self.front_place(key);
        self.change_view_layout(key, &place, change)
    }

    /// Applies one change to the tree at `place` of a Workspace, made when
    /// the file has none for it, and saves it when it changed something. The
    /// tree mints from the checkout's one count (`WorkspaceView::align_ids`).
    pub(super) fn change_view_layout<T>(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        change: impl FnOnce(&mut Layout, u64) -> Result<(T, bool), LayoutError>,
    ) -> Result<T, LayoutError> {
        if *place == Place::Desk || is_desk_key(key) {
            if *place != Place::Desk || !is_desk_key(key) {
                return Err(LayoutError::UnknownArea(key.1.clone()));
            }
            return self.change_desk_views(change);
        }
        let now = unix_milliseconds();
        let Some(store) = self.workspace_views.as_mut() else {
            return Err(LayoutError::UnknownArea(key.1.clone()));
        };
        let entry = store.views.entry(&key.0, &key.1);
        if let Place::Tray(tab) = place {
            entry.tray_entry(tab);
        }
        entry.align_ids(place);
        let layout = entry.layout_mut(place).expect("made above");
        let before = layout.clone();
        let stamp = layout.next_stamp(now);
        let (value, changed) = match change(layout, stamp) {
            Ok(done) => done,
            Err(error) => {
                *layout = before;
                return Err(error);
            }
        };
        if !changed {
            *layout = before;
            return Ok(value);
        }
        store.generation += 1;
        self.persist_workspace_views();
        Ok(value)
    }

    /// `change_view_layout` on the tree that holds the View `id`.
    pub(super) fn change_view<T>(
        &mut self,
        key: &WorkspaceKey,
        id: &str,
        change: impl FnOnce(&mut Layout, u64) -> Result<(T, bool), LayoutError>,
    ) -> Result<T, LayoutError> {
        let place = self
            .view_place(key, id)
            .ok_or_else(|| LayoutError::UnknownDisplay(id.to_owned()))?;
        self.change_view_layout(key, &place, change)
    }

    /// Whether an open may add what it would add at `place` of the Workspace
    /// `key`; a refusal is set as the error and nothing changes (contract
    /// 2). An open of a document the checkout already shows is a focus, or a
    /// move onto the desk, and is never refused.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn admit_view_open(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        preview: bool,
        beside: bool,
    ) -> bool {
        self.admit_view_open_from(key, place, None, path, kind, committed, preview, beside)
    }

    /// `admit_view_open` of a document of the checkout `origin`, which a
    /// View of Desk's desk names (`Display::origin`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn admit_view_open_from(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        origin: Option<&ViewOrigin>,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        preview: bool,
        beside: bool,
    ) -> bool {
        let empty = Layout::default();
        let layout = self.view_layout_at(key, place).unwrap_or(&empty);
        let shown = layout.display_count() + self.pending_view_placements(key, place);
        let held = self
            .view_displays(key)
            .any(|display| display.shows_from(origin, path, kind, committed));
        let base = layout.active_area();
        let refusal = if beside {
            match BESIDE_ORDER
                .iter()
                .find_map(|edge| layout.neighbour(&base.id, *edge))
            {
                Some(target)
                    if layout.area(&target).is_some_and(|area| {
                        area.displays
                            .iter()
                            .any(|display| display.shows_from(origin, path, kind, committed))
                    }) =>
                {
                    None
                }
                Some(_) => display_limit(shown),
                None if base.displays.is_empty() => display_limit(shown),
                None => match layout.can_split(&base.id, Edge::Right) {
                    Err(error) => Some(error.into()),
                    Ok(()) => display_limit(shown),
                },
            }
        } else if self.new_tab_target(key, place, false).is_some()
            || layout
                .displays()
                .any(|display| display.shows_from(origin, path, kind, committed))
            || (held && *place == Place::Shelf)
        {
            None
        } else if preview
            && base.displays.iter().any(|display| {
                display.preview
                    && !display
                        .tab_id
                        .as_deref()
                        .is_some_and(|tab_id| self.document_kept(tab_id))
            })
        {
            // The area's preview display takes the document in place; one
            // whose document holds work stays and the open adds a display.
            None
        } else {
            display_limit(shown)
        };
        let refusal = refusal.filter(|refusal| {
            refusal.kind != LayoutError::DisplayLimit.kind() || !self.make_view_room(key, place)
        });
        match refusal {
            Some(refusal) => {
                self.set_error(refusal.kind, refusal.message, false);
                false
            }
            None => true,
        }
    }

    /// The Views at `place` of the Workspace `key`, with the opens still
    /// being read into it, which `MAX_VIEW_DISPLAYS` caps.
    pub(super) fn views_at(&self, key: &WorkspaceKey, place: &Place) -> usize {
        self.view_layout_at(key, place)
            .map_or(0, Layout::display_count)
            + self.pending_view_placements(key, place)
    }

    /// Room for one more View at `place` of the Workspace `key` (PRD
    /// lane-desk-pair-view D-23, B12; lane-files-checkout-views D-07). While
    /// the tree and the opens still being read into it fill
    /// `MAX_VIEW_DISPLAYS`, the View looked at longest ago leaves it: off a
    /// desk an agent's View goes to the shelf, still in its pane's list, and
    /// the operator's own or a card closes; off the shelf a View closes. A
    /// View in front of any area and a document with unsaved work never
    /// leave, and when too few can, nothing does. Returns whether there is
    /// room.
    pub(super) fn make_view_room(&mut self, key: &WorkspaceKey, place: &Place) -> bool {
        let count = (self.views_at(key, place) + 1).saturating_sub(MAX_VIEW_DISPLAYS);
        if count == 0 {
            return true;
        }
        let oldest = self.closable_views(key, place);
        if oldest.len() < count {
            return false;
        }
        oldest
            .into_iter()
            .take(count)
            .all(|view| self.retire_oldest_view(key, place, view))
    }

    /// Whether a caller's open adds a View to `place` rather than focus one
    /// there, or move one the checkout holds elsewhere onto it, so room is
    /// made for it first. `path` is a file or diff open's read path.
    pub(super) fn open_adds_view(
        &self,
        key: &WorkspaceKey,
        place: &Place,
        action: &crate::workspace_control::Action,
        path: Option<&str>,
    ) -> bool {
        use crate::workspace_control::Action;
        let holds = |held: &dyn Fn(&Display) -> bool| match place {
            // A View anywhere in the checkout stays where it is for a caller
            // whose open goes to the shelf.
            Place::Shelf => self.view_displays(key).any(held),
            desk => self
                .view_layout_at(key, desk)
                .is_some_and(|layout| layout.displays().any(held)),
        };
        match (action, path) {
            (Action::OpenFile { .. }, Some(path)) => {
                !holds(&|display| display.shows(path, DisplayKind::File, None))
            }
            (Action::OpenDiff { .. }, Some(path)) => {
                !holds(&|display| display.shows(path, DisplayKind::Diff, Some(false)))
            }
            (Action::OpenExternal { grant, .. }, _) => !holds(&|display| {
                display
                    .external
                    .as_ref()
                    .is_some_and(|shown| shown.grant == *grant)
            }),
            (Action::OpenBrowser { url, .. }, _) => {
                url.is_empty()
                    || !holds(&|display| {
                        display.kind == DisplayKind::Browser && display.url.as_deref() == Some(url)
                    })
            }
            _ => false,
        }
    }

    /// The Views that may leave `place`, looked at longest ago first.
    fn closable_views(&self, key: &WorkspaceKey, place: &Place) -> Vec<OldestView> {
        let Some(layout) = self.view_layout_at(key, place) else {
            return Vec::new();
        };
        let fronts: HashSet<&str> = layout
            .areas()
            .into_iter()
            .filter_map(|area| area.active.as_deref())
            .collect();
        let mut closable: Vec<_> = layout
            .displays()
            .filter(|display| !fronts.contains(display.id.as_str()))
            // A run's summary card is never closed to make room.
            .filter(|display| !display.is_run_summary())
            // A file still being restored has no document to close yet.
            .filter(|display| match &display.tab_id {
                Some(tab_id) => !self.document_kept(tab_id),
                None => matches!(
                    display.kind,
                    DisplayKind::Browser | DisplayKind::External | DisplayKind::Card
                ),
            })
            .collect();
        closable.sort_by_key(|display| display.last_focused_unix_ms);
        closable
            .into_iter()
            .map(|display| OldestView {
                display_id: display.id.clone(),
                kind: display.kind,
                document: display.tab_id.clone(),
                agents: !display.operator && !display.openers.is_empty(),
            })
            .collect()
    }

    /// Takes the View off `place`: an agent's View on a desk goes to the
    /// shelf, any other closes, and its document with it when no other View
    /// shows it, the way closing its last View does (so Reopen Closed brings
    /// a file back).
    fn retire_oldest_view(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        oldest: OldestView,
    ) -> bool {
        if oldest.agents
            && matches!(place, Place::Tray(_))
            && self.make_view_room(key, &Place::Shelf)
            && self.bring_view_to(key, &oldest.display_id, &Place::Shelf)
        {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "view_layout.oldest_shelved",
                "device": key.0,
                "display": oldest.display_id,
            }));
            return true;
        }
        let removed = self.change_view_layout(key, place, |layout, _| {
            let removed = layout.remove(&oldest.display_id).is_some();
            Ok((removed, removed))
        });
        if !removed.unwrap_or(false) {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "view_areas",
            "kind": "view_layout.oldest_closed",
            "device": key.0,
            "display": oldest.display_id,
            "display_kind": oldest.kind,
        }));
        if let Some(document) = oldest.document
            && !self.desk_holds_tab(&document)
            && !self
                .every_view()
                .any(|(_, display)| display.tab_id.as_ref() == Some(&document))
        {
            self.close_file_tab_now(&document);
        }
        true
    }

    fn new_tab_target(&self, key: &WorkspaceKey, place: &Place, beside: bool) -> Option<String> {
        if beside {
            return None;
        }
        let area = self.view_layout_at(key, place)?.active_area();
        area.displays
            .iter()
            .find(|d| Some(&d.id) == area.active.as_ref() && d.is_new_tab())
            .map(|d| d.id.clone())
    }

    /// `file_open` naming the Workspace in front (`view`) for a file of
    /// another registered checkout of its device: a terminal link opens the
    /// file where the operator is. The file stays its own checkout's
    /// document, read and saved there, and only the display is the front
    /// Workspace's, so no checkout comes forward and the Explorer keeps its
    /// tree. A Workspace that is not in front, or a checkout of another
    /// device, opens nothing.
    pub(super) fn open_file_in_front_view(
        &mut self,
        view: &ViewWorkspace,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        beside: bool,
    ) {
        let destination = (view.device_id.clone(), view.path.clone());
        let source = self.workspace_key(workspace_id, checkout_id);
        if self.front_workspace_key().as_ref() != Some(&destination)
            || source.is_none_or(|source| source.0 != destination.0)
        {
            self.set_error(
                "file.invalid_context",
                "A file opens where it was asked only in the Workspace in front, from a checkout of its device",
                false,
            );
            return;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "view_areas",
            "kind": "file.open_in_front",
            "device": destination.0,
            "checkout": checkout_id,
        }));
        self.open_file_in_view(&destination, workspace_id, checkout_id, path, false, beside);
    }

    /// `file_open` with View areas, into the Workspace `key`: `beside` asks
    /// for the area next to the one in use (Open to the side, B4).
    pub(super) fn open_file_in_view(
        &mut self,
        key: &WorkspaceKey,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
        beside: bool,
    ) {
        match self.prepare_file_tab(workspace_id, checkout_id, path) {
            Ok(prepared) => self.show_file_in_view(
                prepared,
                key,
                workspace_id,
                checkout_id,
                path,
                preview,
                beside,
            ),
            // The read failed before anything moved, so every display keeps
            // what it shows (B15).
            Err(message) => self.set_error("file.open_failed", message, true),
        }
    }

    /// Opens a file of the checkout `workspace_id`/`checkout_id` into the
    /// View areas of the Workspace `key`, the front one (contract 4.2). A
    /// file being read shows when the read lands, where it was asked for; a
    /// failed read moves nothing.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show_file_in_view(
        &mut self,
        prepared: super::editor::PreparedFileTab,
        key: &WorkspaceKey,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
        beside: bool,
    ) {
        self.reconcile_view_displays();
        let key = key.clone();
        let place = self.front_place(&key);
        let origin = self.desk_origin(&key, workspace_id, checkout_id);
        if !self.admit_view_open_from(
            &key,
            &place,
            origin.as_ref(),
            path,
            DisplayKind::File,
            None,
            preview,
            beside,
        ) {
            return;
        }
        // The Explorer shows the Workspace's own tree, which holds no file of
        // another checkout to select.
        if self.workspace_key(workspace_id, checkout_id).as_ref() == Some(&key) {
            self.snapshot.ui_state.selected_path = Some(path.to_owned());
        }
        let replace = self.new_tab_target(&key, &place, beside);
        match prepared {
            super::editor::PreparedFileTab::Open(tab_id) => {
                self.place_document(
                    &key,
                    &place,
                    &tab_id,
                    preview,
                    beside,
                    None,
                    replace.as_deref(),
                );
            }
            read @ super::editor::PreparedFileTab::Read { .. } => {
                if let Some(tab_id) =
                    self.insert_file_tab(read, workspace_id, checkout_id, path, preview)
                {
                    self.place_document(
                        &key,
                        &place,
                        &tab_id,
                        preview,
                        beside,
                        None,
                        replace.as_deref(),
                    );
                }
            }
            super::editor::PreparedFileTab::Reading { root, channel } => {
                // A display that shows the file but could not read it is
                // focused now, and the read fills its tab (S6 B20).
                let shown = if !beside && replace.is_none() {
                    match self.focus_view_document(
                        &key,
                        &place,
                        origin.as_ref(),
                        path,
                        DisplayKind::File,
                        None,
                        !preview,
                    ) {
                        Ok(shown) => shown,
                        Err(error) => {
                            self.set_error(error.kind(), error.message(), false);
                            return;
                        }
                    }
                } else {
                    false
                };
                let placement = (!shown).then(|| ViewPlacement {
                    area: self.tree_area(&key, &place),
                    place: place.clone(),
                    key: key.clone(),
                    preview,
                    beside,
                    replace,
                });
                self.start_document_open(
                    root,
                    channel,
                    OpenRequestFields {
                        follow: false,
                        workspace_id: workspace_id.to_owned(),
                        checkout_id: checkout_id.to_owned(),
                        path: path.to_owned(),
                        preview,
                        reload: false,
                        reveal: None,
                        restore: false,
                        placement,
                    },
                );
            }
        }
    }

    /// Complete every admitted GUI destination of one shared read. Material
    /// is cloned only when no destination has retained a buffer yet, so a
    /// refusal in one destination cannot discard another owner's request.
    pub(super) fn complete_file_in_views(
        &mut self,
        prepared: super::editor::PreparedFileTab,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        mut placements: Vec<ViewPlacement>,
    ) {
        if placements.len() == 1 {
            self.complete_file_in_view(
                prepared,
                workspace_id,
                checkout_id,
                path,
                placements.pop().unwrap(),
            );
            return;
        }
        // Keep the existing buffer's own material until all destinations
        // finish. A refusal may retire a clean, otherwise unreferenced
        // buffer, but the next admitted destination still owns its open.
        let prepared = match prepared {
            super::editor::PreparedFileTab::Open(ref tab_id) => {
                match self
                    .editor_documents
                    .get(tab_id)
                    .zip(self.document_places.get(tab_id))
                {
                    Some((document, place)) => super::editor::PreparedFileTab::Read {
                        tab_id: tab_id.clone(),
                        document: Box::new(EditorDocumentSnapshot::clone(document)),
                        place: place.clone(),
                    },
                    None => prepared,
                }
            }
            other => other,
        };
        for placement in placements {
            let next = match &prepared {
                super::editor::PreparedFileTab::Read {
                    tab_id,
                    document,
                    place,
                } => {
                    if self
                        .snapshot
                        .editor
                        .tabs
                        .iter()
                        .any(|tab| &tab.id == tab_id)
                    {
                        super::editor::PreparedFileTab::Open(tab_id.clone())
                    } else {
                        super::editor::PreparedFileTab::Read {
                            tab_id: tab_id.clone(),
                            document: document.clone(),
                            place: place.clone(),
                        }
                    }
                }
                super::editor::PreparedFileTab::Open(tab_id) => {
                    super::editor::PreparedFileTab::Open(tab_id.clone())
                }
                super::editor::PreparedFileTab::Reading { .. } => {
                    unreachable!("completion already has its read result")
                }
            };
            self.complete_file_in_view(next, workspace_id, checkout_id, path, placement);
        }
    }

    /// Finish a GUI read where it was asked: the desk of the tab it was asked
    /// in. While that tab is not the one in front, the open leaves that
    /// desk's selection as it was (B7).
    pub(super) fn complete_file_in_view(
        &mut self,
        prepared: super::editor::PreparedFileTab,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        placement: ViewPlacement,
    ) {
        self.reconcile_view_displays();
        let background = self.front_place(&placement.key) != placement.place;
        let kept = background.then(|| self.keep_front(&placement.key));
        let tab_id = match prepared {
            super::editor::PreparedFileTab::Open(tab_id) => Some(tab_id),
            prepared => {
                self.insert_file_tab(prepared, workspace_id, checkout_id, path, placement.preview)
            }
        };
        let Some(tab_id) = tab_id else {
            return;
        };
        let placed = self.try_place_document(
            &placement.key,
            &placement.place,
            &tab_id,
            placement.preview,
            placement.beside,
            Some(placement.area.as_str()),
            placement.replace.as_deref(),
        );
        if let Err(error) = placed {
            self.refuse_placement(&placement.key, &tab_id, error);
            if let Some(kept) = kept {
                self.restore_front(kept);
            }
            return;
        }
        if !background && self.front_workspace_key().as_ref() == Some(&placement.key) {
            self.apply_area_intent_to(&placement.key, AreaIntent::Documents);
            if self.front_checkout() == Some((workspace_id, checkout_id)) {
                self.snapshot.ui_state.selected_path = Some(path.to_owned());
            }
        }
        if let Some(kept) = kept {
            self.restore_front(kept);
        }
    }

    /// Opens a diff into the front Workspace's View areas by the same rules
    /// as a file (A10). Its text comes from the Changes read. Returns whether
    /// anything changed.
    pub(super) fn show_diff_in_view(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        committed: bool,
        preview: bool,
        beside: bool,
    ) -> bool {
        self.reconcile_view_displays();
        let Some(key) = self.workspace_key(workspace_id, checkout_id) else {
            self.set_error(
                "diff.invalid_context",
                "The diff's project or checkout is no longer available",
                false,
            );
            return true;
        };
        let place = self.front_place(&key);
        if !self.admit_view_open(
            &key,
            &place,
            path,
            DisplayKind::Diff,
            Some(committed),
            preview,
            beside,
        ) {
            return true;
        }
        let replace = self.new_tab_target(&key, &place, beside);
        let tab_id = Self::diff_tab_id(workspace_id, checkout_id, path, committed);
        if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
            self.insert_diff_tab(workspace_id, checkout_id, path, committed, preview);
        }
        self.place_document(
            &key,
            &place,
            &tab_id,
            preview,
            beside,
            None,
            replace.as_deref(),
        )
    }

    /// Focuses the display at `place` of a document that was focused last,
    /// keeping it open when `keep_open`. A desk takes a View of the document
    /// the checkout holds elsewhere first (D-20: one View, one desk); the
    /// shelf leaves a View the checkout holds anywhere where it is. Returns
    /// whether the checkout shows the document at all.
    #[allow(clippy::too_many_arguments)]
    fn focus_view_document(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        origin: Option<&ViewOrigin>,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        keep_open: bool,
    ) -> Result<bool, LayoutError> {
        let here = self.view_layout_at(key, place).and_then(|layout| {
            last_focused_showing(
                &layout.displays().collect::<Vec<_>>(),
                origin,
                path,
                kind,
                committed,
            )
        });
        let display_id = match here {
            Some(display_id) => display_id,
            None => {
                // Desk's desk has no other place a View of it could be.
                if *place == Place::Desk {
                    return Ok(false);
                }
                let Some(elsewhere) = last_focused_showing(
                    &self.view_displays(key).collect::<Vec<_>>(),
                    origin,
                    path,
                    kind,
                    committed,
                ) else {
                    return Ok(false);
                };
                if *place == Place::Shelf {
                    self.claim_view_opener(key, path, kind, committed);
                    return Ok(true);
                }
                if !self.bring_view_to(key, &elsewhere, place) {
                    return Err(LayoutError::DisplayLimit);
                }
                elsewhere
            }
        };
        self.change_view_layout(key, place, |layout, stamp| {
            let mut changed = layout.focus(&display_id, stamp)?;
            if keep_open && let Some(display) = layout.display_mut(&display_id) {
                changed |= std::mem::replace(&mut display.preview, false);
            }
            Ok(((), changed))
        })?;
        self.claim_view_opener(key, path, kind, committed);
        Ok(true)
    }

    /// Shows the document of editor tab `tab_id` in the Workspace `key`
    /// (contract 4.2). A display that shows it already is focused, and kept
    /// open unless a preview was asked for. Otherwise it opens in `area`, the
    /// area in use when the open was asked (the one in use now if that area
    /// is gone): a preview takes the area's preview display in place, and
    /// anything else is a new display at the end. `beside` puts it in the
    /// area next to that one, or a new area to its right. Returns whether the
    /// layout changed.
    ///
    /// It decides by the displays' bindings, so every caller reconciles them
    /// before it adds the tab it places: reads land on workers, and one that
    /// landed since the last reconcile leaves a display unbound to the
    /// document it shows.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn place_document(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        tab_id: &str,
        preview: bool,
        beside: bool,
        area: Option<&str>,
        replace: Option<&str>,
    ) -> bool {
        match self.try_place_document(key, place, tab_id, preview, beside, area, replace) {
            Ok(changed) => changed,
            Err(error) => {
                self.refuse_placement(key, tab_id, error);
                true
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_place_document(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        tab_id: &str,
        preview: bool,
        beside: bool,
        area: Option<&str>,
        replace: Option<&str>,
    ) -> Result<bool, LayoutError> {
        let placed =
            self.try_place_document_at(key, place, tab_id, preview, beside, area, replace)?;
        let shown = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| {
                Some((
                    tab.path.clone(),
                    display_kind(tab.kind)?,
                    tab.diff_committed,
                ))
            });
        let claimed = shown.is_some_and(|(path, kind, committed)| {
            self.claim_view_opener(key, &path, kind, committed)
        });
        Ok(placed || claimed)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_place_document_at(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        tab_id: &str,
        preview: bool,
        beside: bool,
        area: Option<&str>,
        replace: Option<&str>,
    ) -> Result<bool, LayoutError> {
        let Some((path, kind, committed)) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| {
                Some((
                    tab.path.clone(),
                    display_kind(tab.kind)?,
                    tab.diff_committed,
                ))
            })
        else {
            return Err(LayoutError::UnknownDisplay(tab_id.to_owned()));
        };
        // A View of Desk's desk names the checkout its document is of.
        let origin = (*place == Place::Desk)
            .then(|| self.tab_origin(tab_id))
            .flatten();
        let base = match (self.view_layout_at(key, place), area) {
            (Some(layout), Some(area)) if layout.area(area).is_some() => area.to_owned(),
            (Some(layout), asked) => {
                if let Some(asked) = asked {
                    crate::diagnostic!(serde_json::json!({
                        "component": "view_areas",
                        "kind": "view_layout.area_gone",
                        "device": key.0,
                        "area": asked,
                        "tab": tab_id,
                    }));
                }
                layout.active_area().id.clone()
            }
            (None, _) => self.tree_area(key, place),
        };
        if beside {
            return self.place_beside(
                key,
                place,
                &base,
                tab_id,
                origin.as_ref(),
                &path,
                kind,
                committed,
            );
        }
        if let Some(id) = replace {
            // The read belongs to the empty tab captured at open. A close or
            // navigation meanwhile must never overwrite the operator's newer work.
            let placed = self.change_view_layout(key, place, |layout, stamp| {
                if !layout.display(id).is_some_and(Display::is_new_tab) {
                    return Err(LayoutError::UnknownDisplay(id.to_owned()));
                }
                let area_id = layout
                    .area_of(id)
                    .ok_or_else(|| LayoutError::UnknownDisplay(id.to_owned()))?
                    .id
                    .clone();
                // An existing copy in this area gives way; other areas keep theirs.
                let twins: Vec<_> = layout
                    .area(&area_id)
                    .unwrap()
                    .displays
                    .iter()
                    .filter(|d| d.id != id && d.shows_from(origin.as_ref(), &path, kind, committed))
                    .map(|d| d.id.clone())
                    .collect();
                for twin in twins {
                    layout.remove(&twin);
                }
                let display = layout.display_mut(id).unwrap();
                display.kind = kind;
                display.path = path.clone();
                display.origin = origin.clone();
                display.tab_id = Some(tab_id.to_owned());
                display.committed =
                    (kind == DisplayKind::Diff).then_some(committed.unwrap_or(false));
                display.url = None;
                display.title = None;
                display.load = 0;
                display.preview = false;
                display.forget_openers();
                layout.focus(id, stamp)?;
                Ok(((), true))
            });
            placed?;
            return Ok(true);
        }
        if self.focus_view_document(
            key,
            place,
            origin.as_ref(),
            &path,
            kind,
            committed,
            !preview,
        )? {
            return Ok(true);
        }
        // The preview display the open takes, with the document it showed.
        let slot = preview
            .then(|| {
                let area = self.view_layout_at(key, place)?.area(&base)?;
                area.displays
                    .iter()
                    .find(|display| display.preview)
                    .map(|display| {
                        (
                            display.id.clone(),
                            display.tab_id.clone(),
                            display.path.clone(),
                            display.kind,
                        )
                    })
            })
            .flatten();
        let old_kept = slot
            .as_ref()
            .and_then(|(_, old_tab, _, _)| old_tab.as_deref())
            .is_some_and(|old_tab| self.document_kept(old_tab));
        let placed = self.change_view_layout(key, place, |layout, stamp| {
            match &slot {
                Some((slot_id, _, _, _)) if !old_kept => {
                    let display = layout
                        .display_mut(slot_id)
                        .ok_or_else(|| LayoutError::UnknownDisplay(slot_id.clone()))?;
                    display.tab_id = Some(tab_id.to_owned());
                    display.path = path.clone();
                    display.origin = origin.clone();
                    display.kind = kind;
                    display.committed =
                        (kind == DisplayKind::Diff).then_some(committed.unwrap_or(false));
                    // The preview shows another document now, opened by
                    // whoever opens it.
                    display.forget_openers();
                    layout.focus(slot_id, stamp)?;
                }
                // A document with unsaved work keeps its display, opened for
                // good, and the new preview goes beside it.
                kept => {
                    if let Some((slot_id, _, _, _)) = kept
                        && let Some(display) = layout.display_mut(slot_id)
                    {
                        display.preview = false;
                    }
                    let mut display = layout.new_display(&path, kind, committed, preview);
                    display.tab_id = Some(tab_id.to_owned());
                    display.origin = origin.clone();
                    layout.insert(&base, display, stamp)?;
                }
            }
            Ok(((), true))
        });
        placed?;
        if let Some((_, old_tab, old_path, old_kind)) = slot
            && !old_kept
        {
            self.release_previewed(key, old_tab, (&old_path, old_kind), tab_id);
        }
        Ok(true)
    }

    /// Open to the side (B4): a kept-open display in the area next to
    /// `base`, or in a new area to its right when there is none. A display of
    /// the document already in that area is focused instead.
    #[allow(clippy::too_many_arguments)]
    fn place_beside(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        base: &str,
        tab_id: &str,
        origin: Option<&ViewOrigin>,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
    ) -> Result<bool, LayoutError> {
        let placed = self.change_view_layout(key, place, |layout, stamp| {
            let neighbour = BESIDE_ORDER
                .iter()
                .find_map(|edge| layout.neighbour(base, *edge));
            let Some(target) = neighbour else {
                let mut display = layout.new_display(path, kind, committed, false);
                display.tab_id = Some(tab_id.to_owned());
                display.origin = origin.cloned();
                // An empty area is only ever the root of a Workspace with no
                // view yet: there is nothing to stand beside, and a split
                // would leave that empty area on screen next to the new one.
                if layout
                    .area(base)
                    .is_some_and(|area| area.displays.is_empty())
                {
                    layout.insert(base, display, stamp)?;
                    return Ok(((), true));
                }
                if let Err(error) = layout.split_new(base, Edge::Right, display, stamp) {
                    // The tree changed while the file was read: a display
                    // that shows the document already is focused, and with
                    // none the open is refused rather than doubled up in the
                    // area it was asked from.
                    let Some(shown) = last_focused_showing(
                        &layout.displays().collect::<Vec<_>>(),
                        origin,
                        path,
                        kind,
                        committed,
                    ) else {
                        return Err(error);
                    };
                    crate::diagnostic!(serde_json::json!({
                        "component": "view_areas",
                        "kind": "view_layout.beside_refused",
                        "device": key.0,
                        "reason": error.kind(),
                    }));
                    layout.focus(&shown, stamp)?;
                }
                return Ok(((), true));
            };
            let shown = layout.area(&target).and_then(|area| {
                last_focused_showing(
                    &area.displays.iter().collect::<Vec<_>>(),
                    origin,
                    path,
                    kind,
                    committed,
                )
            });
            match shown {
                Some(display_id) => {
                    layout.focus(&display_id, stamp)?;
                    if let Some(display) = layout.display_mut(&display_id) {
                        display.preview = false;
                    }
                }
                None => {
                    let mut display = layout.new_display(path, kind, committed, false);
                    display.tab_id = Some(tab_id.to_owned());
                    display.origin = origin.cloned();
                    layout.insert(&target, display, stamp)?;
                }
            }
            Ok(((), true))
        });
        placed?;
        Ok(true)
    }

    /// An open whose display cannot be added says why, and the document it
    /// read goes with it unless a display shows it or it holds work, so the
    /// reconcile does not put it on screen anyway.
    fn refuse_placement(&mut self, key: &WorkspaceKey, tab_id: &str, error: LayoutError) {
        self.set_error(error.kind(), error.message(), false);
        let shown = self
            .view_displays(key)
            .any(|display| display.tab_id.as_deref() == Some(tab_id));
        if shown || self.document_kept(tab_id) || self.desk_holds_tab(tab_id) {
            return;
        }
        if let Some(index) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        {
            self.retire_editor_tab(index);
            self.rebuild_tab_strips();
        }
    }

    /// The document a preview display showed before it took `new_tab`'s
    /// goes when nothing else shows it, without a Recent Closed entry, as the
    /// preview tab it replaces did (D-04); one still being read is dropped.
    fn release_previewed(
        &mut self,
        key: &WorkspaceKey,
        old_tab: Option<String>,
        (old_path, old_kind): (&str, DisplayKind),
        new_tab: &str,
    ) {
        match old_tab {
            Some(old_tab) if old_tab != new_tab => {
                let shown = self
                    .view_displays(key)
                    .any(|display| display.tab_id.as_deref() == Some(old_tab.as_str()));
                if shown || self.desk_holds_tab(&old_tab) {
                    return;
                }
                if let Some(index) = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .position(|tab| tab.id == old_tab)
                {
                    self.retire_editor_tab(index);
                    self.rebuild_tab_strips();
                    self.push_diagnostic(
                        "editor.preview_replaced",
                        format!("Preview tab {old_tab} replaced by {new_tab}"),
                    );
                }
            }
            None if old_kind == DisplayKind::File
                && !self
                    .view_displays(key)
                    .any(|display| display.shows(old_path, DisplayKind::File, None)) =>
            {
                // The Workspace's checkout is the one the new document is in.
                if let Some((workspace_id, checkout_id)) = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .find(|tab| tab.id == new_tab)
                    .map(|tab| (tab.workspace_id.clone(), tab.checkout_id.clone()))
                {
                    self.cancel_document_read(&workspace_id, &checkout_id, old_path);
                }
            }
            _ => {}
        }
    }

    /// Whether a document holds work a preview would lose: a draft, a save
    /// running, or a save's outcome the operator has not seen settle.
    pub(super) fn document_kept(&self, tab_id: &str) -> bool {
        self.snapshot
            .editor
            .tabs
            .iter()
            .any(|tab| tab.id == tab_id && tab.dirty)
            || self
                .editor_documents
                .get(tab_id)
                .is_some_and(|document| document.save.is_some())
    }

    /// Keep Open for a document: every preview display of it stays open, and
    /// the tab's flag follows (B5). A display binds only to a tab of its own
    /// Workspace, so that Workspace is the only one looked at.
    pub(super) fn promote_view_displays(&mut self, tab_id: &str) -> bool {
        let mut changed = false;
        let key = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| self.workspace_key(&tab.workspace_id, &tab.checkout_id));
        if let Some(store) = self.workspace_views.as_mut()
            && let Some((device, path)) = key
            && let Some(view) = store.views.get_mut(&device, &path)
        {
            for display in view.views_mut() {
                if display.tab_id.as_deref() == Some(tab_id) && display.preview {
                    display.preview = false;
                    changed = true;
                }
            }
            if changed {
                store.generation += 1;
            }
        }
        if changed {
            self.persist_workspace_views();
        }
        if let Some(tab) = self
            .snapshot
            .editor
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id && tab.preview)
        {
            tab.preview = false;
            self.rebuild_tab_strips();
            changed = true;
        }
        if changed {
            self.push_diagnostic("editor.preview_promoted", format!("Tab {tab_id} kept open"));
        }
        changed
    }

    /// Whether `tab_id` is an open file with a document, which a draft or a
    /// conflict choice can name.
    pub(super) fn file_document_open(&self, tab_id: &str) -> bool {
        self.editor_documents.contains_key(tab_id)
            && self
                .snapshot
                .editor
                .tabs
                .iter()
                .any(|tab| tab.id == tab_id && tab.kind == EditorTabKind::File)
    }

    /// One operator action on the front Workspace's View areas (contract
    /// 4.1). Returns whether the snapshot changed.
    pub(super) fn apply_view_layout(&mut self, payload: ViewLayoutPayload) -> bool {
        if !self.separate_view_areas() {
            self.set_error(
                "view_layout.unsupported",
                "This shell does not draw View areas",
                false,
            );
            return true;
        }
        let ViewLayoutPayload { workspace, action } = payload;
        let key = (workspace.device_id, workspace.path);
        // Desk's desk is in front wherever the window drew it; a Workspace's
        // only while it is the one in front.
        if is_desk_key(&key) {
            if self.desk_views().is_none() {
                self.set_error("view_layout.desk_off", "Desk is switched off", false);
                return true;
            }
        } else if !self.area_workspace_is_current(&key, "view") {
            return true;
        }
        // The action names displays by what the last frame showed; bring
        // their bindings up to date first.
        self.reconcile_view_displays();
        // Every action but a close names the desk the frame drew: the one
        // the Workspace shows now.
        let place = self.front_place(&key);
        let outcome = match action {
            ViewLayoutAction::Focus { display_id } => {
                // A View on no desk or on another tab's, a pane's `N views`
                // row, comes to this desk in front (PRD lane-desk-pair-view
                // B3, D-20).
                let moved = self.bring_view_to(&key, &display_id, &place);
                let focused = self.change_view_layout(&key, &place, |layout, stamp| {
                    layout
                        .focus(&display_id, stamp)
                        .map(|changed| (changed, changed))
                });
                // One brought from elsewhere is one the operator asked to
                // see: the lane shows the tray.
                if moved {
                    self.open_tray_for(&key, &place);
                }
                self.follow_pair_front(&key, &display_id);
                // A View chosen onto a desk ⌘E folded stands it, in the same
                // event (D-63).
                let unfolded = match &place {
                    Place::Tray(tab) => self.unfold_pair_desk(&key, tab),
                    _ => false,
                };
                focused.map(|changed| changed || moved || unfolded)
            }
            ViewLayoutAction::FocusArea { area_id } => {
                self.change_view_layout(&key, &place, |layout, stamp| {
                    layout
                        .focus_area(&area_id, stamp)
                        .map(|changed| (changed, changed))
                })
            }
            ViewLayoutAction::Move {
                display_id,
                area_id,
                index,
            } => self.change_view_layout(&key, &place, |layout, stamp| {
                layout
                    .move_display(&display_id, &area_id, index, stamp)
                    .map(|changed| (changed, changed))
            }),
            ViewLayoutAction::Split {
                display_id,
                area_id,
                edge,
                request_id,
            } => {
                if self.split_request_seen(&key, &request_id) {
                    return false;
                }
                let split = self.change_view_layout(&key, &place, |layout, stamp| {
                    layout
                        .split(&display_id, &area_id, edge, stamp)
                        .map(|_| (true, true))
                });
                if split.is_ok() {
                    self.remember_split_request(&key, request_id);
                }
                split
            }
            ViewLayoutAction::Resize { split_id, ratio } => {
                self.change_view_layout(&key, &place, |layout, _| {
                    layout
                        .resize(&split_id, ratio)
                        .map(|changed| (changed, changed))
                })
            }
            ViewLayoutAction::KeepOpen { display_id } => {
                self.change_view(&key, &display_id, |layout, _| {
                    let display = layout
                        .display_mut(&display_id)
                        .ok_or_else(|| LayoutError::UnknownDisplay(display_id.clone()))?;
                    let changed = std::mem::replace(&mut display.preview, false);
                    Ok((changed, changed))
                })
            }
            ViewLayoutAction::Close {
                display_id,
                pending_save,
                discard,
            } => {
                // A tray tab an agent opened leaves the tray alone: the View
                // stays in its pane's list to open again (PRD
                // lane-desk-pair-view D-07, B11), and the last tab leaving
                // folds the tray.
                let agents = self
                    .view_display(&key, &display_id)
                    .is_some_and(|display| !display.operator && !display.openers.is_empty());
                if agents
                    && matches!(self.view_place(&key, &display_id), Some(Place::Tray(_)))
                    && self.bring_view_to(&key, &display_id, &Place::Shelf)
                {
                    self.close_emptied_lane(&key);
                    return true;
                }
                return self.close_view_display(&key, &display_id, pending_save, discard);
            }
            ViewLayoutAction::Retry { display_id } => {
                return self.retry_view_display(&key, &display_id);
            }
            ViewLayoutAction::Navigate { display_id, url } => {
                return self.navigate_browser(&key, &display_id, url);
            }
        };
        match outcome {
            Ok(changed) => changed,
            Err(error) => {
                self.set_error(error.kind(), error.message(), false);
                true
            }
        }
    }

    /// `browser_open`: a page in a View area of the Workspace the payload
    /// names (see `BrowserOpenPayload`), in its area in use, and the View
    /// area brought back if only Agents showed. A refusal says why and
    /// changes nothing; a request that named itself gets a receipt either way.
    pub(super) fn open_browser(&mut self, payload: BrowserOpenPayload) -> bool {
        let BrowserOpenPayload {
            url,
            reveal,
            workspace,
            pane_id,
            opener_pane,
            request_id,
            area_id,
            beside_display,
        } = payload;
        let opener = opener_pane
            .as_deref()
            .map_or(crate::view_feedback::OpenedBy::Operator, |pane| {
                self.link_owner(pane)
            });
        let outcome = self.place_browser(
            &url,
            workspace,
            pane_id.as_deref(),
            area_id.as_deref(),
            beside_display.as_deref(),
            reveal,
            opener,
        );
        if let Err(refusal) = &outcome {
            self.set_error(refusal.kind, refusal.message.clone(), false);
        }
        if let Some(request_id) = request_id {
            self.record_open_receipt(request_id, outcome);
        }
        true
    }

    /// Opens `url` in the Workspace `key`, brought forward with its device,
    /// as `opener`'s page, or the operator's with none: a decision's closed
    /// page opened again (PRD mission-human-gate D-20). The display it is
    /// in, or why it was refused.
    pub(super) fn place_browser_as(
        &mut self,
        url: &str,
        key: WorkspaceKey,
        opener: Option<crate::view_feedback::Opener>,
    ) -> Result<String, &'static str> {
        let workspace = ViewWorkspace {
            device_id: key.0,
            path: key.1,
        };
        self.place_browser(url, Some(workspace), None, None, None, true, opener.into())
            .map(|(_, display_id)| display_id)
            .map_err(|refusal| refusal.kind)
    }

    /// Opens `url` on the desk of `key` as the operator's page, nothing
    /// brought forward: Desk's desk opening a session's page (PRD
    /// desk-own-desk D-12). The display it is in, or why it was refused.
    pub(super) fn open_browser_at(
        &mut self,
        url: &str,
        key: WorkspaceKey,
    ) -> Result<String, &'static str> {
        let workspace = ViewWorkspace {
            device_id: key.0,
            path: key.1,
        };
        self.place_browser(
            url,
            Some(workspace),
            None,
            None,
            None,
            false,
            crate::view_feedback::OpenedBy::Operator,
        )
        .map(|(_, display_id)| display_id)
        .map_err(|refusal| refusal.kind)
    }

    /// The receipt a sender that named its open waits for.
    fn record_open_receipt(
        &mut self,
        request_id: String,
        outcome: Result<(WorkspaceKey, String), LayoutRefusal>,
    ) {
        let receipt = match outcome {
            Ok(((device_id, path), display_id)) => BrowserOpenReceiptSnapshot {
                request_id,
                ok: true,
                message: None,
                device_id: Some(device_id),
                path: Some(path),
                display_id: Some(display_id),
            },
            Err(refusal) => BrowserOpenReceiptSnapshot {
                request_id,
                ok: false,
                message: Some(refusal.message),
                device_id: None,
                path: None,
                display_id: None,
            },
        };
        let receipts = &mut self.snapshot.status.browser_opens;
        receipts.push(receipt);
        if receipts.len() > BROWSER_OPEN_RECEIPTS_KEPT {
            receipts.remove(0);
        }
    }

    /// `external_open`: an outside file's tab in a View area (see
    /// `ExternalOpenPayload`). A refusal says why and changes nothing; the log
    /// names the grant and the kind, never a path, which the core never holds.
    pub(super) fn open_external(&mut self, payload: ExternalOpenPayload) -> bool {
        let ExternalOpenPayload {
            grant,
            name,
            kind,
            workspace,
            pane_id,
            reveal,
            opener_pane,
            request_id,
        } = payload;
        let outcome = ExternalFile::admitted(&grant, &name, kind)
            .map_err(external_refusal)
            .and_then(|file| self.place_external(file, workspace, pane_id.as_deref(), reveal));
        if let (Ok((key, display_id)), Some(pane)) = (&outcome, opener_pane.as_deref()) {
            let by = self.link_owner(pane);
            self.stamp_view_opener(key, display_id, by);
        }
        crate::diagnostic!(serde_json::json!({
            "component": "view_areas",
            "kind": "external.open",
            "grant": crate::view_layout::external_grant(&grant).then_some(grant.as_str()),
            "file_kind": kind,
            "outcome": match &outcome {
                Ok(_) => "placed",
                Err(refusal) => refusal.kind,
            },
        }));
        if let Err(refusal) = &outcome {
            self.set_error(refusal.kind, refusal.message.clone(), false);
        }
        if let Some(request_id) = request_id {
            self.record_open_receipt(request_id, outcome);
        }
        true
    }

    fn place_external(
        &mut self,
        file: ExternalFile,
        workspace: Option<ViewWorkspace>,
        pane_id: Option<&str>,
        reveal: bool,
    ) -> Result<(WorkspaceKey, String), LayoutRefusal> {
        if !self.separate_view_areas() {
            return Err(external_refusal("This shell does not draw View areas"));
        }
        let key = match (workspace, pane_id) {
            (Some(workspace), _) => {
                let key = (workspace.device_id, workspace.path);
                if !self.catalog_has_workspace(&key) {
                    return Err(external_refusal(format!(
                        "{} is not a Workspace hide shows",
                        key.1
                    )));
                }
                key
            }
            (None, Some(pane_id)) => self.pane_workspace_key(pane_id).ok_or_else(|| {
                external_refusal(format!(
                    "Pane {pane_id} is not in a Workspace hide shows on this Mac"
                ))
            })?,
            (None, None) => self
                .front_workspace_key()
                .ok_or_else(|| external_refusal("No Workspace is in front to open the file in"))?,
        };
        let caller = pane_id
            .map(str::to_owned)
            .unwrap_or_else(|| crate::workspace_control::checkout_caller_id("external", &key.1));
        let reveal_context = if reveal {
            Some(
                self.resolve_caller(&key.0, crate::workspace_control::Caller::parse(&caller))
                    .map_err(|refusal| external_refusal(refusal.next_action))?,
            )
        } else {
            None
        };
        let place = self.front_place(&key);
        if !self.shows_external(&key, &file.grant) {
            self.make_view_room(&key, &place);
        }
        let (display_id, load) = self
            .put_external(&key, &place, file)
            .map_err(LayoutRefusal::from)?;
        self.apply_area_intent_to(&key, AreaIntent::Documents);
        if let Some(context) = reveal_context {
            self.reveal_workspace_control(
                &context,
                &key,
                &format!("external-open-{load}"),
                &caller,
            );
        }
        Ok((key, display_id))
    }

    /// Whether a display of `key` stands on `grant`.
    pub(super) fn shows_external(&self, key: &WorkspaceKey, grant: &str) -> bool {
        self.view_displays(key).any(|display| {
            display
                .external
                .as_ref()
                .is_some_and(|shown| shown.grant == grant)
        })
    }

    /// Puts `file` in the active area at `place` of `key`, or brings forward
    /// the display its grant already has and reads it again from the disk
    /// (B10); answers the display and its load. Room is the caller's to make
    /// first.
    pub(super) fn put_external(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        file: ExternalFile,
    ) -> Result<(String, u64), LayoutError> {
        let before_load = self
            .workspace_views
            .as_ref()
            .map(|store| store.browser_load);
        let load = self.next_browser_load();
        let existing = self.claim_existing_view(key, place, |display| {
            display
                .external
                .as_ref()
                .is_some_and(|shown| shown.grant == file.grant)
        });
        let placed = match existing {
            Some((display_id, at)) => self.change_view_layout(key, &at, |layout, stamp| {
                if at == *place {
                    layout.focus(&display_id, stamp)?;
                }
                if let Some(display) = layout.display_mut(&display_id) {
                    display.load = load;
                    display.external = Some(file.clone());
                }
                Ok((display_id.clone(), true))
            }),
            None => self.change_view_layout(key, place, |layout, stamp| {
                let display = layout.new_external_display(file.clone(), load);
                let display_id = display.id.clone();
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
                Ok((display_id, true))
            }),
        };
        match placed {
            Ok(display_id) => Ok((display_id, load)),
            Err(error) => {
                if let (Some(load), Some(store)) = (before_load, self.workspace_views.as_mut()) {
                    store.browser_load = load;
                }
                Err(error)
            }
        }
    }

    /// The View of the checkout `key` that `shows` matches, last focused
    /// first, with where it stands once the open has its way: a desk takes it
    /// from wherever it is (D-20), and the shelf leaves it where it is.
    pub(super) fn claim_existing_view(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        shows: impl Fn(&Display) -> bool,
    ) -> Option<(String, Place)> {
        let here = self.view_layout_at(key, place).and_then(|layout| {
            layout
                .displays()
                .filter(|display| shows(display))
                .max_by_key(|display| display.last_focused_unix_ms)
                .map(|display| display.id.clone())
        });
        if let Some(id) = here {
            return Some((id, place.clone()));
        }
        let elsewhere = self
            .view_displays(key)
            .filter(|display| shows(display))
            .max_by_key(|display| display.last_focused_unix_ms)
            .map(|display| display.id.clone())?;
        if *place == Place::Shelf {
            let at = self.view_place(key, &elsewhere)?;
            return Some((elsewhere, at));
        }
        if self.bring_view_to(key, &elsewhere, place) {
            return Some((elsewhere, place.clone()));
        }
        let at = self.view_place(key, &elsewhere)?;
        Some((elsewhere, at))
    }

    /// `external_grants`: the desktop host's grants now. An outside file's tab
    /// whose grant it no longer holds (the app quit and came back) goes, in
    /// every Workspace, so no tab stands for a file nobody can read.
    pub(super) fn keep_external_grants(&mut self, payload: ExternalGrantsPayload) -> bool {
        if payload.live.len() > MAX_EXTERNAL_GRANTS_LISTED {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "external.grants_refused",
                "listed": payload.live.len(),
            }));
            return false;
        }
        let live: HashSet<&str> = payload.live.iter().map(String::as_str).collect();
        let stale: Vec<(WorkspaceKey, Place)> = self
            .every_view_layout()
            .filter(|(_, _, layout)| {
                layout.displays().any(|display| {
                    display.kind == DisplayKind::External
                        && !display
                            .external
                            .as_ref()
                            .is_some_and(|file| live.contains(file.grant.as_str()))
                })
            })
            .map(|(key, place, _)| (key, place))
            .collect();
        let mut dropped = 0;
        for (key, place) in &stale {
            let _ = self.change_view_layout(key, place, |layout, _| {
                let gone = layout.remove_external(|file| live.contains(file.grant.as_str()));
                dropped += gone;
                Ok(((), gone > 0))
            });
            self.close_emptied_lane(key);
        }
        if dropped > 0 {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "external.grants_gone",
                "dropped": dropped,
            }));
        }
        dropped > 0
    }

    #[allow(clippy::too_many_arguments)]
    fn place_browser(
        &mut self,
        url: &str,
        workspace: Option<ViewWorkspace>,
        pane_id: Option<&str>,
        area_id: Option<&str>,
        beside_display: Option<&str>,
        reveal: bool,
        opener: crate::view_feedback::OpenedBy,
    ) -> Result<(WorkspaceKey, String), LayoutRefusal> {
        if !self.separate_view_areas() {
            return Err(browser_refusal("This shell does not draw View areas"));
        }
        if !url.is_empty() && !browser_address(url) {
            return Err(browser_refusal(UNLOADABLE_ADDRESS));
        }
        let key = match (workspace, pane_id) {
            (Some(workspace), _) => {
                let key = (workspace.device_id, workspace.path);
                if is_desk_key(&key) {
                    if self.desk_views().is_none() {
                        return Err(browser_refusal("Desk is switched off"));
                    }
                } else if !self.catalog_has_workspace(&key) {
                    return Err(browser_refusal(format!(
                        "{} is not a Workspace hide shows",
                        key.1
                    )));
                }
                key
            }
            (None, Some(pane_id)) => self.pane_workspace_key(pane_id).ok_or_else(|| {
                browser_refusal(format!(
                    "Pane {pane_id} is not in a Workspace hide shows on this Mac"
                ))
            })?,
            (None, None) => self
                .front_workspace_key()
                .ok_or_else(|| browser_refusal("No Workspace is in front to open the page in"))?,
        };
        let caller = pane_id
            .map(str::to_owned)
            .unwrap_or_else(|| crate::workspace_control::checkout_caller_id("browser", &key.1));
        // A page's own new tab joins that page's tree; a pane the operator
        // does not watch puts its page on no desk, unless it asked to reveal
        // (PRD lane-desk-pair-view B1, D-37); anything else is the
        // operator's, on the desk in front.
        let place = match beside_display.and_then(|id| self.view_place(&key, id)) {
            Some(place) => place,
            None if pane_id.is_some() && !reveal => self.control_place(&key, &caller),
            None => self.front_place(&key),
        };
        let bound_area = area_id.map(str::to_owned).or_else(|| {
            beside_display.and_then(|id| {
                self.view_layout_at(&key, &place)
                    .and_then(|layout| layout.area_of(id).map(|area| area.id.clone()))
            })
        });
        let action = crate::workspace_control::Action::OpenBrowser {
            url: url.to_owned(),
            area_id: bound_area,
            new_target: false,
        };
        let reveal_context = if reveal {
            Some(
                self.resolve_caller(&key.0, crate::workspace_control::Caller::parse(&caller))
                    .map_err(|refusal| browser_refusal(refusal.next_action))?,
            )
        } else {
            None
        };
        if self.open_adds_view(&key, &place, &action, None) {
            self.make_view_room(&key, &place);
        }
        let before_load = self
            .workspace_views
            .as_ref()
            .map(|store| store.browser_load);
        let load = self.next_browser_load();
        // A page the checkout holds already: a desk takes it (D-20), the
        // shelf leaves it where it is.
        let existing = (!url.is_empty())
            .then(|| {
                self.claim_existing_view(&key, &place, |display| {
                    display.kind == DisplayKind::Browser && display.url.as_deref() == Some(url)
                })
            })
            .flatten();
        let at = existing
            .as_ref()
            .map_or_else(|| place.clone(), |(_, at)| at.clone());
        let placed = self
            .change_view_layout(&key, &at, |layout, stamp| {
                // A page's own new tab belongs to whoever that page belongs
                // to; any other page opened with no agent is the operator's
                // (PRD session-files D-10, D-30).
                let parent = beside_display
                    .and_then(|id| layout.display(id))
                    .map(|page| {
                        let owners = (page.opener.clone(), page.link_terminal.clone());
                        (page.openers.clone(), page.operator, owners)
                    });
                let own = |display: &mut Display| match (&opener, &parent) {
                    (
                        crate::view_feedback::OpenedBy::Operator,
                        Some((openers, operator, (mark, terminal))),
                    ) => {
                        // The page's own new tab is its sessions' newest View.
                        display.opened_stamp = stamp;
                        for known in openers {
                            if !display.openers.iter().any(|held| held.same_agent(known)) {
                                display.openers.push(known.clone());
                            }
                        }
                        display.operator |= *operator;
                        display.opener = display.opener.take().or_else(|| mark.clone());
                        display.link_terminal =
                            display.link_terminal.take().or_else(|| terminal.clone());
                    }
                    (by, _) => {
                        display.opened_as(by.clone(), stamp);
                    }
                };
                if let Some((display_id, _)) = &existing {
                    if at == place {
                        layout.focus(display_id, stamp)?;
                    }
                    if let Some(display) = layout.display_mut(display_id) {
                        display.load = load;
                        own(display);
                    }
                    return Ok((display_id.clone(), true));
                }
                let mut display = layout.new_browser_display(url, load);
                own(&mut display);
                let display_id = display.id.clone();
                // A page's own new tab stands beside that page (Open to the
                // side's order): in the area next to it, else, its area being
                // the only one, in a new area to its right, which the area cap
                // always leaves room for. With the page gone it opens where
                // any page would.
                let opener = beside_display
                    .and_then(|opener| layout.area_of(opener))
                    .map(|area| area.id.clone());
                if let Some(base) = opener {
                    match BESIDE_ORDER
                        .iter()
                        .find_map(|edge| layout.neighbour(&base, *edge))
                    {
                        Some(target) => layout.insert(&target, display, stamp)?,
                        None => {
                            layout.split_new(&base, Edge::Right, display, stamp)?;
                        }
                    }
                    return Ok((display_id, true));
                }
                let area = area_id
                    .filter(|area| layout.area(area).is_some())
                    .unwrap_or(&layout.active_area().id)
                    .to_owned();
                layout.insert(&area, display, stamp)?;
                Ok((display_id, true))
            })
            .map_err(LayoutRefusal::from);
        let display_id = match placed {
            Ok(id) => id,
            Err(error) => {
                if let (Some(load), Some(store)) = (before_load, self.workspace_views.as_mut()) {
                    store.browser_load = load;
                }
                return Err(error);
            }
        };
        if at != Place::Shelf && self.front_place(&key) == at {
            self.apply_area_intent_to(&key, AreaIntent::Documents);
        }
        if let Some(context) = reveal_context {
            self.reveal_workspace_control(&context, &key, &format!("browser-open-{load}"), &caller);
        }
        Ok((key, display_id))
    }

    /// `view_layout` navigate: the operator's own address in a browser
    /// display's toolbar, which its page loads.
    pub(super) fn navigate_browser(
        &mut self,
        key: &WorkspaceKey,
        display_id: &str,
        url: String,
    ) -> bool {
        if !browser_address(&url) {
            self.set_error("browser.navigate_refused", UNLOADABLE_ADDRESS, false);
            return true;
        }
        let load = self.next_browser_load();
        let outcome = self.change_view(key, display_id, |layout, _| {
            let display = layout
                .display_mut(display_id)
                .filter(|display| display.kind == DisplayKind::Browser)
                .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?;
            let changed = display.url.as_deref() != Some(url.as_str());
            if changed {
                display.url = Some(url);
                display.title = None;
                // The operator's own address makes the page the operator's
                // (D-03, B4), in every session's strip (PRD session-files
                // D-10).
                display.opener = None;
                display.operator = true;
            }
            display.load = load;
            Ok(((), changed))
        });
        if let Err(error) = outcome {
            self.set_error(error.kind(), error.message(), false);
        }
        true
    }

    /// `browser_state`: what a browser display's page says now. It applies to
    /// any Workspace, since a page keeps loading while another is in front,
    /// and records only: the page is already where it says.
    pub(super) fn record_browser_state(&mut self, payload: BrowserStatePayload) -> bool {
        let BrowserStatePayload {
            workspace,
            display_id,
            url,
            title,
            load,
            loading,
            failure,
            present,
        } = payload;
        let key = (workspace.device_id, workspace.path);
        if !browser_address(&url) {
            // A page that moved somewhere a display does not hold keeps the
            // last address it may; the log names only the scheme.
            let scheme: String = url
                .split(':')
                .next()
                .unwrap_or("")
                .chars()
                .take(16)
                .collect();
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "browser.state_refused",
                "device": key.0,
                "display": display_id,
                "scheme": scheme,
            }));
            return false;
        }
        let title = Some(browser_title(&title)).filter(|title| !title.is_empty());
        let Some(display) = self
            .view_display(&key, &display_id)
            .filter(|display| display.kind == DisplayKind::Browser)
        else {
            // A page's last report can land after its display closed.
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas",
                "kind": "browser.state_stale",
                "device": key.0,
                "display": display_id,
            }));
            return false;
        };
        let same_address_and_title =
            display.url.as_deref() == Some(url.as_str()) && display.title == title;
        let current_load = display.load;
        if let Some(load) = load {
            if load != current_load {
                crate::diagnostic!(serde_json::json!({
                    "component": "view_areas",
                    "kind": "browser.state_load_stale",
                    "device": key.0,
                    "display": display_id,
                }));
                return false;
            }
            if self.browser_pages.len() >= BROWSER_PAGE_REPORTS_KEPT
                && !self.browser_pages.contains_key(&(
                    key.0.clone(),
                    key.1.clone(),
                    display_id.clone(),
                ))
            {
                let active: std::collections::HashSet<_> = self
                    .workspace_views
                    .as_ref()
                    .into_iter()
                    .flat_map(|store| &store.views.workspaces)
                    .flat_map(|view| {
                        view.views()
                            .filter(|display| display.kind == DisplayKind::Browser)
                            .map(|display| {
                                (
                                    view.device_id.clone(),
                                    view.path.clone(),
                                    display.id.clone(),
                                )
                            })
                    })
                    .chain(
                        self.desk_browser_inventory()
                            .into_iter()
                            .map(|row| (row.device_id, row.path, row.view_id)),
                    )
                    .collect();
                self.browser_pages.retain(|key, _| active.contains(key));
                if self.browser_pages.len() >= BROWSER_PAGE_REPORTS_KEPT {
                    crate::diagnostic!(
                        serde_json::json!({"component":"view_areas","kind":"browser.state_capacity","limit":BROWSER_PAGE_REPORTS_KEPT})
                    );
                    return false;
                }
            }
            self.browser_pages.insert(
                (key.0.clone(), key.1.clone(), display_id.clone()),
                super::workspace_control::ReportedBrowserPage {
                    load,
                    present: present != Some(false),
                    loading,
                    failure: failure.map(|reason| reason.chars().take(300).collect()),
                },
            );
        }
        if same_address_and_title {
            return false;
        }
        if is_desk_key(&key) {
            return self
                .change_desk_views(|layout, _| {
                    let display = layout
                        .display_mut(&display_id)
                        .ok_or_else(|| LayoutError::UnknownDisplay(display_id.clone()))?;
                    display.url = Some(url);
                    display.title = title;
                    Ok(((), true))
                })
                .is_ok();
        }
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let Some(view) = store.views.get_mut(&key.0, &key.1) else {
            return false;
        };
        let Some(display) = view.view_mut(&display_id) else {
            return false;
        };
        display.url = Some(url);
        display.title = title;
        store.generation += 1;
        self.persist_workspace_views();
        true
    }

    /// A load stamp later than every one given in this process, and than the
    /// clock, so a host that outlived a daemon restart still sees a new one
    /// as newer than any it loaded.
    pub(super) fn next_browser_load(&mut self) -> u64 {
        let now = unix_milliseconds();
        let Some(store) = self.workspace_views.as_mut() else {
            return now;
        };
        store.browser_load = now.max(store.browser_load + 1);
        store.browser_load
    }

    /// The Workspace of a pane on this Mac: the checkout whose tab holds it.
    pub(super) fn pane_workspace_key(&self, pane_id: &str) -> Option<WorkspaceKey> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| {
                        checkout
                            .tabs
                            .iter()
                            .any(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
                    })
                    .map(|checkout| (workspace.device_id.clone(), checkout.path.clone()))
            })
    }

    fn catalog_has_workspace(&self, key: &WorkspaceKey) -> bool {
        self.catalog_workspaces().any(|workspace| {
            workspace.device_id == key.0
                && workspace
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.path == key.1)
        })
    }

    pub(super) fn split_request_seen(&self, key: &WorkspaceKey, request_id: &str) -> bool {
        self.workspace_views.as_ref().is_some_and(|store| {
            store
                .split_requests
                .get(key)
                .is_some_and(|seen| seen.iter().any(|id| id == request_id))
        })
    }

    pub(super) fn remember_split_request(&mut self, key: &WorkspaceKey, request_id: String) {
        if let Some(store) = self.workspace_views.as_mut() {
            let seen = store.split_requests.entry(key.clone()).or_default();
            seen.push_back(request_id);
            if seen.len() > SPLIT_REQUESTS_KEPT {
                seen.pop_front();
            }
        }
    }

    /// Close View (B10), or a View whose sessions ended: a display another
    /// display's document also shows,
    /// or one whose file was never read, goes alone, and a save sent with it
    /// is not needed. The last display of a document closes the document the
    /// way its file tab closes, after the save it waits on when there is
    /// one, or without saving on the operator's Don't Save (`discard`); the
    /// display goes when the document does, so a refused save keeps both.
    /// With neither, a last display of unsaved work is refused and nothing
    /// changes (contract 4.1): the web's frame may still have shown another
    /// view of it, so a close it sent bare can never drop the text. A close
    /// that leaves the files tool with no view and its tree off closes the
    /// lane, once the save it waits on has landed (`close_emptied_lane`).
    pub(super) fn close_view_display(
        &mut self,
        key: &WorkspaceKey,
        display_id: &str,
        pending_save: Option<FileSavePayload>,
        discard: bool,
    ) -> bool {
        let Some(place) = self.view_place(key, display_id) else {
            return self.close_absent_display(key, display_id);
        };
        let Some(display) = self.view_display(key, display_id) else {
            return self.close_absent_display(key, display_id);
        };
        if self.refuse_fixed_view(key, display_id, "close") {
            return false;
        }
        let (path, kind, committed) = (display.path.clone(), display.kind, display.committed);
        let origin = display.origin.clone();
        let source = self.display_source_of(key, display);
        let tab = display.tab_id.clone();
        // A document can show in another Workspace too, a file a terminal
        // link opened where the operator was, or on Desk's desk, and stays
        // open for it (PRD desk-own-desk D-11).
        let shared = match &tab {
            Some(tab) => self.every_view().any(|(view, other)| {
                other.tab_id.as_ref() == Some(tab) && (&view != key || other.id != display_id)
            }),
            None => self.view_displays(key).any(|other| {
                other.id != display_id && other.shows_from(origin.as_ref(), &path, kind, committed)
            }),
        };
        if let Some(tab_id) = tab.as_ref()
            && !shared
        {
            if let Some(save) = pending_save {
                return self.start_file_save_then_close(tab_id.clone(), save, Some(key.clone()));
            }
            if !discard && self.document_kept(tab_id) {
                let label = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .find(|held| &held.id == tab_id)
                    .map_or(path.as_str(), |held| held.label.as_str());
                let message = format!(
                    "{label} has changes that are not saved, so its last view stays open; close it with Save or Don't Save"
                );
                self.set_error("view_layout.unsaved", message, false);
                return true;
            }
            let changed = self.close_file_tab_now(tab_id);
            self.close_emptied_lane(key);
            return changed;
        }
        let _ = self.change_view_layout(key, &place, |layout, _| {
            let removed = layout.remove(display_id).is_some();
            Ok(((), removed))
        });
        if tab.is_none()
            && !shared
            && kind == DisplayKind::File
            && let Some((workspace_id, checkout_id)) = source
        {
            self.cancel_document_read(&workspace_id, &checkout_id, &path);
        }
        self.close_emptied_lane(key);
        true
    }

    /// A close of a display that is already gone, by an earlier send of the
    /// same close or by its document closing, has nothing left to do.
    fn close_absent_display(&self, key: &WorkspaceKey, display_id: &str) -> bool {
        crate::diagnostic!(serde_json::json!({
            "component": "view_areas",
            "kind": "view_layout.close_absent",
            "device": key.0,
            "display": display_id,
        }));
        false
    }

    /// Retry (B16): reads an unavailable display's file again into its tab.
    /// A display waiting for its device says what it waits for.
    fn retry_view_display(&mut self, key: &WorkspaceKey, display_id: &str) -> bool {
        let Some(display) = self.view_display(key, display_id).cloned() else {
            let error = LayoutError::UnknownDisplay(display_id.to_owned());
            self.set_error(error.kind(), error.message(), false);
            return true;
        };
        let front = self.front_checkout_owned();
        let source = self.display_source_of(key, &display);
        let (state, reason) = self.display_state(&self.display_facts(key), &display);
        // A View of Desk's desk waiting for its device reads again once the
        // device is back: Retry after a reconnect is the operator's way to
        // ask (PRD desk-own-desk B41).
        let state = match state {
            ViewDisplayState::Waiting
                if display
                    .origin
                    .as_ref()
                    .is_some_and(|origin| self.view_root_wait(&origin.key()).is_none()) =>
            {
                ViewDisplayState::Unavailable
            }
            state => state,
        };
        match state {
            ViewDisplayState::Waiting => {
                self.set_error(
                    "view_layout.waiting",
                    reason.unwrap_or_else(|| "This view waits for its device".to_owned()),
                    false,
                );
                true
            }
            ViewDisplayState::Open | ViewDisplayState::Opening => false,
            ViewDisplayState::Unavailable => {
                let Some((workspace_id, checkout_id)) = source.clone().or(front) else {
                    return false;
                };
                // A diff is taken by the Changes read; its view needs only
                // its tab back, which the reconcile binds it to.
                if display.kind == DisplayKind::Diff {
                    self.insert_diff_tab(
                        &workspace_id,
                        &checkout_id,
                        &display.path,
                        display.committed.unwrap_or(false),
                        display.preview,
                    );
                    return true;
                }
                let (workspace_id, checkout_id) = source.unwrap_or((workspace_id, checkout_id));
                match self.document_source(&workspace_id, &checkout_id) {
                    Ok((root, channel)) => self.start_document_open(
                        root,
                        channel,
                        OpenRequestFields {
                            follow: false,
                            workspace_id,
                            checkout_id,
                            path: display.path,
                            preview: display.preview,
                            reload: false,
                            reveal: None,
                            restore: true,
                            placement: None,
                        },
                    ),
                    Err(message) => self.insert_unavailable_file_tab(
                        &workspace_id,
                        &checkout_id,
                        &display.path,
                        display.preview,
                        message,
                    ),
                }
                true
            }
        }
    }

    /// Reads back the documents of a Workspace's displays, the first time in
    /// this process that its files can be read (contract 5). Files are read
    /// on the document worker like any open; a file that cannot be read
    /// becomes an unavailable tab that says why. Diffs show at once. Nothing
    /// is started in the engine, and nothing takes the screen: the tree was
    /// published already, and each display binds to its tab when it lands.
    pub(super) fn restore_view_displays(&mut self, key: &WorkspaceKey) {
        let Some(front) = self.front_checkout_owned() else {
            return;
        };
        let Some(view) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.views.get(&key.0, &key.1))
        else {
            return;
        };
        let areas: usize = view.layouts().map(|(_, layout)| layout.area_count()).sum();
        let mut targets: Vec<(String, DisplayKind, Option<bool>)> = Vec::new();
        // A page has no document to read back; the host loads it once shown.
        for display in self
            .view_displays(key)
            .filter(|display| display.kind.is_document())
        {
            let target = (display.path.clone(), display.kind, display.committed);
            if display.tab_id.is_none() && !targets.contains(&target) {
                targets.push(target);
            }
        }
        let mut reads = 0usize;
        for (path, kind, committed) in &targets {
            // A file a terminal link opened here from another checkout is
            // read from that checkout; a diff is always this one's.
            let (workspace_id, checkout_id) = match kind {
                DisplayKind::File => self.display_source(key, path),
                DisplayKind::Diff
                | DisplayKind::Browser
                | DisplayKind::External
                | DisplayKind::Card => None,
            }
            .unwrap_or_else(|| front.clone());
            let open = self.snapshot.editor.tabs.iter().any(|tab| {
                tab.workspace_id == workspace_id
                    && tab.checkout_id == checkout_id
                    && tab.path == *path
                    && display_kind(tab.kind) == Some(*kind)
                    && (*kind == DisplayKind::File || tab.diff_committed == *committed)
            });
            if open {
                continue;
            }
            match kind {
                DisplayKind::Browser | DisplayKind::External | DisplayKind::Card => {}
                DisplayKind::Diff => {
                    self.insert_diff_tab(
                        &workspace_id,
                        &checkout_id,
                        path,
                        committed.unwrap_or(false),
                        false,
                    );
                }
                DisplayKind::File => {
                    reads += 1;
                    match self.document_source(&workspace_id, &checkout_id) {
                        Ok((root, channel)) => self.start_document_open(
                            root,
                            channel,
                            OpenRequestFields {
                                follow: false,
                                workspace_id: workspace_id.clone(),
                                checkout_id: checkout_id.clone(),
                                path: path.clone(),
                                preview: false,
                                reload: false,
                                reveal: None,
                                restore: true,
                                placement: None,
                            },
                        ),
                        Err(message) => self.insert_unavailable_file_tab(
                            &workspace_id,
                            &checkout_id,
                            path,
                            false,
                            message,
                        ),
                    }
                }
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "workspace_views",
            "kind": "workspace_views.restore",
            "device": key.0,
            "areas": areas,
            "documents": targets.len(),
            "reads": reads,
        }));
    }

    /// Keeps every Workspace's displays in step with the editor's tabs, and
    /// the editor's active tab on the front Workspace's active display. Runs
    /// after every event and before every snapshot read; with nothing
    /// changed since the last pass it costs one comparison of the editor's
    /// tabs.
    pub(super) fn reconcile_view_displays(&mut self) {
        let Some(store) = self.workspace_views.as_ref() else {
            return;
        };
        let desk = store.front.as_ref().map(|key| self.front_place(key));
        let desk_generation = self.desk_views_generation();
        if store.reconciled.as_ref().is_some_and(|seen| {
            seen.generation == store.generation
                && seen.desk_generation == desk_generation
                && seen.front == store.front
                && seen.desk == desk
                && seen.active == self.snapshot.editor.active_tab_id
                && seen.tabs == self.snapshot.editor.tabs
        }) {
            return;
        }
        // An active tab the last pass did not set was activated by an event.
        let requested = self
            .snapshot
            .editor
            .active_tab_id
            .clone()
            .filter(|active| store.derived_active.as_ref() != Some(active));
        let tabs: Vec<TabFacts> = self
            .snapshot
            .editor
            .tabs
            .iter()
            .filter_map(|tab| {
                Some(TabFacts {
                    id: tab.id.clone(),
                    key: self.workspace_key(&tab.workspace_id, &tab.checkout_id),
                    path: tab.path.clone(),
                    kind: display_kind(tab.kind)?,
                    committed: tab.diff_committed,
                    kept: tab.dirty
                        || self
                            .editor_documents
                            .get(&tab.id)
                            .is_some_and(|document| document.save.is_some()),
                })
            })
            .collect();
        let roots = self.checkout_roots();
        // Desk's desk binds its files first, so no Workspace takes in a
        // document it shows (PRD desk-own-desk D-11).
        if let Some(layout) = self.desk_views_mut() {
            let (changed, stored) = reconcile_desk_layout(layout, &tabs);
            if changed {
                self.desk_views_changed(stored);
            }
        }
        let store = self.workspace_views.as_ref().expect("checked above");
        let shown = ShownByViews::of(
            self.every_view_layout()
                .map(|(key, _, layout)| (key, layout)),
            &roots,
        );
        // Each live Workspace's desk in front, where a document no View shows
        // lands, and where a tab an event activated comes forward.
        let fronts: HashMap<WorkspaceKey, Place> = store
            .live
            .iter()
            .map(|key| (key.clone(), self.front_place(key)))
            .collect();
        let desk_held: HashSet<String> = tabs
            .iter()
            .filter(|tab| self.desk_holds_tab(&tab.id))
            .map(|tab| tab.id.clone())
            .collect();
        let now = unix_milliseconds();
        let store = self.workspace_views.as_mut().expect("checked above");
        let mut stored = false;
        let mut every_preview: HashMap<String, bool> = HashMap::new();
        let mut unshown: Vec<(String, String, String)> = Vec::new();
        for view in &mut store.views.workspaces {
            let key = (view.device_id.clone(), view.path.clone());
            let front = fronts.get(&key);
            let requested = requested.as_deref().filter(|requested| {
                tabs.iter()
                    .any(|tab| tab.id == *requested && tab.key.as_ref() == Some(&key))
            });
            // A tab an event activated (File Focus, Reopen Closed) is the
            // operator's: its View comes to the desk in front (D-20).
            if let (Some(front), Some(requested)) = (front, requested) {
                let bound = view
                    .views()
                    .filter(|display| display.tab_id.as_deref() == Some(requested))
                    .max_by_key(|display| display.last_focused_unix_ms)
                    .map(|display| display.id.clone());
                if let Some(id) = bound
                    && view.place_of(&id).as_ref() != Some(front)
                {
                    view.tray_entry(match front {
                        Place::Tray(tab) => tab,
                        Place::Shelf | Place::Desk => unreachable!("a tab's desk is in front"),
                    });
                    stored |= view.move_view(&id, front).is_some();
                }
            }
            if let Some(Place::Tray(tab)) = front {
                view.tray_entry(tab);
            }
            // Every tree binds its restored displays first, so the desk in
            // front learns which documents another tree already shows.
            let next = view.next_id();
            let places: Vec<Place> = view.layouts().map(|(place, _)| place).collect();
            for place in &places {
                let layout = view.layout_mut(place).expect("listed above");
                layout.next_id = layout.next_id.max(next);
                let (changed, _) = reconcile_layout(
                    layout,
                    &key,
                    false,
                    &HashSet::new(),
                    &tabs,
                    &roots,
                    None,
                    now,
                );
                stored |= changed;
            }
            // A document another Workspace shows, a file a terminal link
            // opened where the operator was, is not one to add here; nor is
            // one another tree of this checkout shows.
            let bound: HashSet<String> = view
                .views()
                .filter_map(|display| display.tab_id.clone())
                .collect();
            // Nor is a file Desk's desk shows (PRD desk-own-desk D-11).
            let owned: HashSet<String> = tabs
                .iter()
                .filter(|tab| {
                    tab.key.as_ref() == Some(&key)
                        && (shown.elsewhere(tab, &key)
                            || bound.contains(&tab.id)
                            || desk_held.contains(&tab.id))
                })
                .map(|tab| tab.id.clone())
                .collect();
            if let Some(front) = front
                && let Some(layout) = view.layout_mut(front)
            {
                let (changed, capped) =
                    reconcile_layout(layout, &key, true, &owned, &tabs, &roots, requested, now);
                stored |= changed;
                unshown.extend(
                    capped
                        .into_iter()
                        .map(|tab| (tab.id.clone(), tab.path.clone(), key.1.clone())),
                );
            }
            for display in view.views() {
                if let Some(tab_id) = &display.tab_id {
                    *every_preview.entry(tab_id.clone()).or_insert(true) &= display.preview;
                }
            }
        }
        // A document the cap keeps off screen is reported once, not on every
        // pass that still finds it there.
        let reported = std::mem::take(&mut store.unshown);
        store.unshown = unshown
            .iter()
            .map(|(tab_id, _, _)| tab_id.clone())
            .collect();
        if stored {
            store.generation += 1;
            self.persist_workspace_views();
        }
        for (tab_id, path, workspace) in unshown {
            if !reported.contains(&tab_id) {
                self.push_diagnostic(
                    "view_layout.display_limit",
                    format!(
                        "{path} is open but not shown: the desk in front in {workspace} has no room for another view ({MAX_VIEW_DISPLAYS}), and it shows once one is closed"
                    ),
                );
            }
        }
        if let Some(layout) = self.desk_views() {
            for display in layout.displays() {
                if let Some(tab_id) = &display.tab_id {
                    *every_preview.entry(tab_id.clone()).or_insert(true) &= display.preview;
                }
            }
        }
        // The tab's own flag says what its displays say.
        let mut strips = false;
        for tab in &mut self.snapshot.editor.tabs {
            if let Some(every) = every_preview.get(&tab.id)
                && tab.preview != *every
            {
                tab.preview = *every;
                strips = true;
            }
        }
        if strips {
            self.rebuild_tab_strips();
        }
        let derived = self.front_active_view_tab();
        if derived != self.snapshot.editor.active_tab_id {
            match derived.as_deref() {
                Some(tab_id) => {
                    if let Err(message) = self.activate_editor_tab(tab_id) {
                        self.set_error("editor.focus_failed", message, false);
                    }
                }
                None => self.deactivate_editor_tab(),
            }
        }
        let desk = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.front.as_ref())
            .map(|key| self.front_place(key));
        // Counted after this pass's own binding of Desk's desk.
        let desk_generation = self.desk_views_generation();
        let store = self.workspace_views.as_mut().expect("checked above");
        store.derived_active = self.snapshot.editor.active_tab_id.clone();
        store.reconciled = Some(Reconciled {
            desk_generation,
            tabs: self.snapshot.editor.tabs.clone(),
            active: self.snapshot.editor.active_tab_id.clone(),
            front: store.front.clone(),
            desk,
            generation: store.generation,
        });
    }

    /// The document the active area of the front Workspace's desk shows,
    /// while it can show it (contract 3).
    fn front_active_view_tab(&self) -> Option<String> {
        let store = self.workspace_views.as_ref()?;
        let key = store.front.as_ref()?;
        let layout = self.front_tray_layout(key)?;
        let area = layout.active_area();
        let tab_id = area
            .active
            .as_ref()
            .and_then(|active| area.displays.iter().find(|display| &display.id == active))?
            .tab_id
            .as_ref()?;
        let tab = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| &tab.id == tab_id)?;
        let open = tab.kind == EditorTabKind::Diff
            || (tab.unavailable_reason.is_none() && self.editor_documents.contains_key(tab_id));
        open.then(|| tab_id.clone())
    }

    /// The area-active displays of the desk the front Workspace draws: what
    /// is shown, one per area, in tree order. A folded desk or a closed lane
    /// shows none.
    pub(super) fn visible_view_displays(&self) -> Vec<&Display> {
        let Some(key) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.front.as_ref())
        else {
            return Vec::new();
        };
        let Some(layout) = self.drawn_tray_layout(key) else {
            return Vec::new();
        };
        layout
            .areas()
            .into_iter()
            .filter_map(|area| {
                let active = area.active.as_ref()?;
                area.displays.iter().find(|display| &display.id == active)
            })
            .collect()
    }

    /// The tray the Workspace `key` draws: the one of its Agent tab in
    /// front, while the open lane shows it or pair view draws it beside the
    /// tab's pane (PRD lane-desk-pair-view D-06, D-16, D-53), or the
    /// automation screen shows a run of that tab beside its desk (operator
    /// decision 2026-11-21). A tray with no tab draws nothing.
    pub(super) fn drawn_tray_layout(&self, key: &WorkspaceKey) -> Option<&Layout> {
        let view = self.workspace_views.as_ref()?.views.get(&key.0, &key.1)?;
        let tab = self.front_tab(key);
        let tray = view.tray(&tab)?;
        let drawn = self
            .pair_desk_drawn(key, &tab)
            .unwrap_or_else(|| view.draws_tray(&tab))
            || self.viewed_run_tab(key, &tab);
        (drawn && tray.layout.display_count() > 0).then_some(&tray.layout)
    }

    /// Whether `tab` of the Workspace `key` holds the terminal of a run the
    /// automation screen shows, which draws the tab's desk beside it.
    pub(super) fn viewed_run_tab(&self, key: &WorkspaceKey, tab: &str) -> bool {
        let viewed = self.viewed_panes();
        !viewed.is_empty()
            && self
                .catalog_workspaces()
                .filter(|workspace| workspace.device_id == key.0)
                .flat_map(|workspace| &workspace.checkouts)
                .filter(|checkout| checkout.path == key.1)
                .flat_map(|checkout| &checkout.tabs)
                .filter(|row| row.id.as_deref() == Some(tab))
                .flat_map(|row| &row.panes)
                .any(|pane| viewed.contains(&pane.id))
    }

    /// Places the summary card of the run `automation` / `run` first in the
    /// first area of the desk of the tab holding `pane`, its terminal: the
    /// card the run's screen draws beside it, whose tab is fixed there
    /// (operator choice 2026-11-24). It takes the front only on an empty
    /// desk, so what the operator or the run's agent has in front stays in
    /// front. Made again each time the screen says it shows the run, once
    /// the desk holds it. Whether a card was placed.
    pub(super) fn place_run_summary(&mut self, pane: &str, automation: &str, run: u64) -> bool {
        let Some(key) = self.pane_workspace_key(pane) else {
            return false;
        };
        let Some(tab) = self.pane_tab(&key, pane) else {
            return false;
        };
        let place = Place::Tray(tab);
        let card = crate::model::DetailItem::AutomationRun {
            automation: automation.to_owned(),
            run,
        };
        if self.view_layout_at(&key, &place).is_some_and(|layout| {
            layout
                .displays()
                .any(|display| display.card.as_ref() == Some(&card))
        }) {
            return false;
        }
        if !self.make_view_room(&key, &place) {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas", "kind": "view_layout.run_summary_no_room",
                "device": key.0, "automation": automation, "run": run,
            }));
            return false;
        }
        let placed = self.change_view_layout(&key, &place, |layout, _| {
            let display = layout.new_card_display(card);
            let area = layout
                .areas()
                .first()
                .map(|area| area.id.clone())
                .unwrap_or_else(|| layout.active_area().id.clone());
            layout.prepend(&area, display)?;
            Ok(((), true))
        });
        if let Err(error) = placed {
            crate::diagnostic!(serde_json::json!({
                "component": "view_areas", "kind": "view_layout.run_summary_refused",
                "device": key.0, "automation": automation, "run": run,
                "error": error.kind(),
            }));
            return false;
        }
        true
    }

    /// A run the automation screen shows draws its tab's desk beside its
    /// terminal (`viewed_run_tab`). While that desk holds nothing but the
    /// run's summary card, what the run's agent opened while nobody watched
    /// its pane, which waits on no desk (B1), comes onto it, the last opened
    /// in front, as it would have had the operator watched (operator
    /// decision 2026-11-21); with nothing to bring, the summary stays in
    /// front. A desk that already holds another View is left as the operator
    /// arranged it. Whether a View moved.
    pub(super) fn gather_run_views(&mut self, pane: &str) -> bool {
        let Some(key) = self.pane_workspace_key(pane) else {
            return false;
        };
        let Some(tab) = self.pane_tab(&key, pane) else {
            return false;
        };
        let place = Place::Tray(tab);
        if self
            .view_layout_at(&key, &place)
            .is_some_and(|layout| layout.holds_views_beyond_run_summary())
        {
            return false;
        }
        let ids: Vec<String> = self
            .view_layout_at(&key, &Place::Shelf)
            .into_iter()
            .flat_map(|layout| layout.displays())
            .filter(|display| {
                display
                    .openers
                    .iter()
                    .any(|opener| self.opener_pane(opener).as_deref() == Some(pane))
            })
            .map(|display| display.id.clone())
            .collect();
        let mut last = None;
        for id in ids {
            if self.bring_view_to(&key, &id, &place) {
                last = Some(id);
            }
        }
        let Some(id) = last else {
            return false;
        };
        let _ = self.change_view_layout(&key, &place, |layout, stamp| {
            layout.focus(&id, stamp).map(|changed| ((), changed))
        });
        true
    }

    /// The card in front of the area in use of the tray the front Workspace
    /// draws: the one card whose data is read (PRD lane-desk-pair-view,
    /// conductor decision 3).
    pub(super) fn shown_card(&self) -> Option<&crate::model::DetailItem> {
        // While Desk's room is the place in front, the card in front of
        // Desk's desk is the one read (PRD desk-own-desk B4).
        if self.snapshot.ui_state.recent_places.first() == Some(&crate::model::RecentPlace::Desk)
            && let Some(card) = self.desk_front_card()
        {
            return Some(card);
        }
        let key = self.workspace_views.as_ref()?.front.as_ref()?;
        let layout = self.drawn_tray_layout(key)?;
        let area = layout.active_area();
        let active = area.active.as_ref()?;
        layout.display(active)?.card.as_ref()
    }

    /// The file documents on screen, which the snapshot's `documents`
    /// section carries (contract 3.1).
    pub(super) fn visible_view_documents(&self) -> Vec<String> {
        let mut visible: Vec<String> = Vec::new();
        for display in self.visible_view_displays() {
            if display.kind != DisplayKind::File {
                continue;
            }
            let Some(tab_id) = display.tab_id.as_ref() else {
                continue;
            };
            if self.editor_documents.contains_key(tab_id) && !visible.contains(tab_id) {
                visible.push(tab_id.clone());
            }
        }
        // The files in front of Desk's desk's areas: whether the desk is
        // drawn is the window's (PRD desk-own-desk D-11).
        for tab_id in self.desk_front_documents() {
            if self.editor_documents.contains_key(&tab_id) && !visible.contains(&tab_id) {
                visible.push(tab_id);
            }
        }
        visible
    }

    pub(super) fn new_tab_visible(&self) -> bool {
        self.visible_view_displays()
            .iter()
            .any(|display| display.is_new_tab())
    }

    /// The diffs on screen, which the Changes read takes with its own
    /// (contract 3.2), by absolute path.
    pub(super) fn visible_view_diffs(&self) -> Vec<hide_node_link::git::DiffTarget> {
        let mut visible: Vec<hide_node_link::git::DiffTarget> = Vec::new();
        for display in self.visible_view_displays() {
            if display.kind != DisplayKind::Diff || display.tab_id.is_none() {
                continue;
            }
            let target = hide_node_link::git::DiffTarget {
                path: display.path.clone(),
                committed: display.committed.unwrap_or(false),
            };
            if !visible.contains(&target) {
                visible.push(target);
            }
        }
        visible
    }

    /// Why a Workspace's files cannot be read yet, or `None` when they can.
    /// A device's files are read through its helper, and its Workspace also
    /// waits for its catalog, which moves the device's checkouts into their
    /// Projects; this machine's wait for the daemon to open the checkout's
    /// root. A helper that cannot be used gives the device's own reason, the
    /// one History and Settings show, and its displays still wait, so they
    /// open once the device is fixed.
    pub(super) fn view_root_wait(&self, key: &WorkspaceKey) -> Option<String> {
        if key.0 != self.node.as_str() {
            let label = self
                .snapshot
                .ui_state
                .device_registrations
                .iter()
                .find(|device| device.id == key.0)
                .map_or(key.0.as_str(), |device| device.label.as_str());
            match self.device_hosts.get(&key.0).map(|host| &host.phase) {
                None | Some(hosts::HostPhase::Connecting) => {
                    return Some(format!("Waiting for {label} to connect"));
                }
                Some(hosts::HostPhase::Ready { host, .. }) if host.closed_reason().is_none() => {}
                Some(_) => {
                    return Some(
                        self.host_snapshot(&key.0)
                            .message
                            .unwrap_or_else(|| format!("Waiting for {label} to connect")),
                    );
                }
            }
            let catalog_ready = self
                .snapshot
                .status
                .remote
                .iter()
                .find(|status| status.target_id == key.0)
                .is_some_and(|status| status.catalog.state == "ready");
            return (!catalog_ready).then(|| format!("Waiting for {label} to list its projects"));
        }
        let pinned = self
            .file_roots
            .as_ref()
            .is_some_and(|roots| roots.pinned_root(Path::new(&key.1)).is_some());
        (!pinned).then(|| "Waiting for this checkout to open".to_owned())
    }

    /// Every registered checkout as its Workspace key.
    fn checkout_roots(&self) -> Vec<WorkspaceKey> {
        self.catalog_workspaces()
            .flat_map(|workspace| {
                let device = &workspace.device_id;
                workspace
                    .checkouts
                    .iter()
                    .map(move |checkout| (device.clone(), checkout.path.clone()))
            })
            .collect()
    }

    /// The checkout, as its Workspace key, that a display of the Workspace
    /// `key` reads the file `path` from (`source_key`).
    pub(super) fn display_source_key(&self, key: &WorkspaceKey, path: &str) -> WorkspaceKey {
        if hide_platform::path::wire_relative(&key.1, path).is_ok() {
            key.clone()
        } else {
            source_key(key, path, &self.checkout_roots())
        }
    }

    /// The checkout, by its catalog ids, that a display of the Workspace
    /// `key` reads the file `path` from (`display_source_key`).
    pub(super) fn display_source(
        &self,
        key: &WorkspaceKey,
        path: &str,
    ) -> Option<(String, String)> {
        let source = self.display_source_key(key, path);
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == source.0)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == source.1)
                    .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
            })
    }

    /// The checkout a display reads its file from: the one a View of Desk's
    /// desk names, else `display_source` of its Workspace.
    pub(super) fn display_source_of(
        &self,
        key: &WorkspaceKey,
        display: &Display,
    ) -> Option<(String, String)> {
        match &display.origin {
            Some(origin) => self.checkout_ids(&origin.device_id, &origin.path),
            None => self.display_source(key, &display.path),
        }
    }

    /// The catalog's ids of the checkout at `path` on `device`.
    pub(super) fn checkout_ids(&self, device: &str, path: &str) -> Option<(String, String)> {
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == path)
                    .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
            })
    }

    /// What a display's state reads that is the same for every display of
    /// the Workspace `key`, taken once per tree rather than per display.
    fn display_facts(&self, key: &WorkspaceKey) -> DisplayFacts<'_> {
        DisplayFacts {
            key: key.clone(),
            tabs: self
                .snapshot
                .editor
                .tabs
                .iter()
                .map(|tab| (tab.id.as_str(), tab))
                .collect(),
            wait: self.view_root_wait(key),
            feedback: self
                .workspace_views
                .as_ref()
                .and_then(|store| store.views.get(&key.0, &key.1))
                .map(|view| &view.feedback)
                .filter(|feedback| !feedback.comments.is_empty()),
            sessions: std::cell::OnceCell::new(),
            homes: self.machine_homes(&key.0),
        }
    }

    /// The home of the machine `device` names, as wire spellings: this
    /// machine's as named and as its filesystem names it, a connected
    /// device's as its helper reported it; none while it is not known.
    fn machine_homes(&self, device: &str) -> Vec<String> {
        if device != self.node.as_str() {
            return self.device_homes.get(device).cloned().into_iter().collect();
        }
        let mut homes: Vec<String> = [self.home_path.as_deref(), self.home_real.as_deref()]
            .into_iter()
            .flatten()
            .map(hide_platform::path::to_wire_lossy)
            .collect();
        homes.dedup();
        homes
    }

    /// What one display can show now, and why not when it cannot.
    fn display_state(
        &self,
        facts: &DisplayFacts<'_>,
        display: &Display,
    ) -> (ViewDisplayState, Option<String>) {
        // The host loads a page wherever it is; whether it loaded is the
        // page's own state, which the host draws. An outside file is read by
        // the desktop host for its own window, which says whether it could.
        if matches!(
            display.kind,
            DisplayKind::Browser | DisplayKind::External | DisplayKind::Card
        ) {
            return (ViewDisplayState::Open, None);
        }
        if let Some(tab) = display.tab_id.as_deref().and_then(|id| facts.tabs.get(id)) {
            return match (tab.kind, &tab.unavailable_reason) {
                (EditorTabKind::Diff, _) => (ViewDisplayState::Open, None),
                // Retry reads the file again into the same tab, which keeps
                // its old reason until the read lands.
                (_, Some(_))
                    if self.document_opens_path(&tab.workspace_id, &tab.checkout_id, &tab.path) =>
                {
                    (ViewDisplayState::Opening, None)
                }
                (_, Some(reason)) => (ViewDisplayState::Unavailable, Some(reason.clone())),
                _ if self.editor_documents.contains_key(&tab.id) => (ViewDisplayState::Open, None),
                _ => (
                    ViewDisplayState::Unavailable,
                    Some("This file has no document to show".to_owned()),
                ),
            };
        }
        let wait = match &display.origin {
            Some(origin) => self.view_root_wait(&origin.key()),
            None => facts.wait.clone(),
        };
        if let Some(reason) = wait {
            return (ViewDisplayState::Waiting, Some(reason));
        }
        if display.kind == DisplayKind::File
            && self.display_source_of(&facts.key, display).is_some_and(
                |(workspace_id, checkout_id)| {
                    self.document_opens_path(&workspace_id, &checkout_id, &display.path)
                },
            )
        {
            return (ViewDisplayState::Opening, None);
        }
        let reason = match display.kind {
            DisplayKind::File
            | DisplayKind::Browser
            | DisplayKind::External
            | DisplayKind::Card => "This view's file is not open; Retry reads it again",
            DisplayKind::Diff => "This view's diff is not open; Retry opens it again",
        };
        (ViewDisplayState::Unavailable, Some(reason.to_owned()))
    }

    /// A desk's tree as the snapshot carries it. It runs on every snapshot
    /// read, over at most `MAX_VIEW_DISPLAYS` displays: one map of the
    /// editor's tabs and one root check, then a map lookup per display.
    pub(super) fn view_layout_snapshot(
        &self,
        key: &WorkspaceKey,
        layout: &Layout,
    ) -> ViewLayoutSnapshot {
        let facts = self.display_facts(key);
        let active_area = layout.active_area().id.clone();
        let root = self
            .view_node_snapshot(&facts, &layout.root)
            .unwrap_or_else(|| {
                ViewNodeSnapshot::Area(ViewAreaSnapshot {
                    id: active_area.clone(),
                    active: None,
                    displays: Vec::new(),
                })
            });
        let active_area = if snapshot_has_area(&root, &active_area) {
            active_area
        } else {
            first_snapshot_area(&root).to_owned()
        };
        ViewLayoutSnapshot {
            root,
            active_area,
            limits: ViewLimitsSnapshot {
                areas: MAX_VIEW_AREAS,
                depth: MAX_SPLIT_DEPTH,
                displays: MAX_VIEW_DISPLAYS,
            },
            display_count: layout.display_count(),
        }
    }

    /// Every View of the checkout as the snapshot carries one, on a desk or
    /// not: a pane's `N views` lists what its session opened wherever it is
    /// (B3, D-19). It runs under the runtime lock on each snapshot of the
    /// Workspace in front, so its bound is the checkout's: at most
    /// `MAX_TRAYS` (64) desks of `MAX_VIEW_DISPLAYS` (32) Views each, and
    /// the shelf's 32 (`docs/PERFORMANCE_TESTING.md`).
    pub(super) fn every_view_snapshot(&self, key: &WorkspaceKey) -> Vec<ViewDisplaySnapshot> {
        let facts = self.display_facts(key);
        self.view_displays(key)
            .map(|display| self.display_snapshot(&facts, display))
            .collect()
    }

    fn display_snapshot(&self, facts: &DisplayFacts<'_>, display: &Display) -> ViewDisplaySnapshot {
        let (state, reason) = self.display_state(facts, display);
        let tab = display
            .tab_id
            .as_deref()
            .and_then(|id| facts.tabs.get(id).copied());
        let (comments, not_delivered) = facts.feedback.map_or((0, 0), |feedback| {
            feedback
                .on(display)
                .fold((0, 0), |(all, expired), comment| {
                    (
                        all + 1,
                        expired
                            + usize::from(matches!(
                                comment.state,
                                crate::view_feedback::CommentState::NotDelivered { .. }
                            )),
                    )
                })
        });
        let opener = display
            .opener
            .as_ref()
            .map(|opener| self.view_opener_snapshot(opener));
        let none = [];
        let sessions = if display.card.is_some() {
            facts
                .sessions
                .get_or_init(|| self.live_sessions())
                .as_slice()
        } else {
            &none
        };
        let owner_pane = self.owner_pane_of(display, opener.as_ref(), sessions);
        let file = display.kind == DisplayKind::File;
        let origin_homes = display
            .origin
            .as_ref()
            .map(|origin| self.machine_homes(&origin.device_id));
        let homes = origin_homes.as_ref().unwrap_or(&facts.homes);
        let home_relative = file
            .then(|| {
                homes.iter().find_map(|home| {
                    hide_platform::path::wire_relative(home, &display.path)
                        .ok()
                        .filter(|below| !below.is_root())
                        .map(|below| format!("~/{below}"))
                })
            })
            .flatten();
        ViewDisplaySnapshot {
            id: display.id.clone(),
            tab_id: tab.map(|tab| tab.id.clone()),
            path: display.path.clone(),
            label: match (tab, display.kind) {
                (_, DisplayKind::Browser) => browser_label(display),
                (_, DisplayKind::External) => display
                    .external
                    .as_ref()
                    .map_or_else(String::new, |file| file.name.clone()),
                (_, DisplayKind::Card) => String::new(),
                (Some(tab), _) => tab.label.clone(),
                (None, DisplayKind::File) => file_label(&display.path),
                (None, DisplayKind::Diff) => {
                    super::editor::diff_label(&display.path, display.committed.unwrap_or(false))
                }
            },
            kind: display.kind,
            committed: display.committed,
            preview: display.preview,
            state,
            reason,
            url: display.url.clone(),
            title: display.title.clone(),
            load: display.load,
            opener,
            comments,
            not_delivered,
            unseen: display.unseen.is_some(),
            external: display.external.clone(),
            card: display.card.clone(),
            owner_pane,
            pasted: file && hide_node_link::attachments::is_clipboard_wire_path(&display.path),
            home_relative,
            origin: display.origin.clone(),
        }
    }

    fn view_node_snapshot(
        &self,
        facts: &DisplayFacts<'_>,
        node: &Node,
    ) -> Option<ViewNodeSnapshot> {
        match node {
            Node::Area(area) => {
                let active = area
                    .active
                    .as_ref()
                    .filter(|id| area.displays.iter().any(|display| &display.id == *id))
                    .cloned()
                    .or_else(|| {
                        area.displays
                            .iter()
                            .max_by_key(|display| display.last_focused_unix_ms)
                            .map(|display| display.id.clone())
                    })?;
                Some(ViewNodeSnapshot::Area(ViewAreaSnapshot {
                    id: area.id.clone(),
                    active: Some(active),
                    displays: area
                        .displays
                        .iter()
                        .map(|display| self.display_snapshot(facts, display))
                        .collect(),
                }))
            }
            Node::Split(split) => {
                let first = self.view_node_snapshot(facts, &split.first);
                let second = self.view_node_snapshot(facts, &split.second);
                match (first, second) {
                    (Some(first), Some(second)) => {
                        Some(ViewNodeSnapshot::Split(ViewSplitSnapshot {
                            id: split.id.clone(),
                            axis: split.axis,
                            ratio: split.ratio,
                            first: Box::new(first),
                            second: Box::new(second),
                        }))
                    }
                    (one, other) => one.or(other),
                }
            }
        }
    }
}

fn snapshot_has_area(node: &ViewNodeSnapshot, id: &str) -> bool {
    match node {
        ViewNodeSnapshot::Area(area) => area.id == id,
        ViewNodeSnapshot::Split(split) => {
            snapshot_has_area(&split.first, id) || snapshot_has_area(&split.second, id)
        }
    }
}

fn first_snapshot_area(node: &ViewNodeSnapshot) -> &str {
    match node {
        ViewNodeSnapshot::Area(area) => &area.id,
        ViewNodeSnapshot::Split(split) => first_snapshot_area(&split.first),
    }
}

/// A page's name on its tab: its title once it has one, else its host (for
/// a file, its file name), else its address.
/// A View's name as its tab says it, from what the View holds alone.
pub(super) fn display_label(display: &Display) -> String {
    match display.kind {
        DisplayKind::Browser => browser_label(display),
        DisplayKind::External => display
            .external
            .as_ref()
            .map_or_else(String::new, |file| file.name.clone()),
        DisplayKind::File => file_label(&display.path),
        DisplayKind::Diff => {
            super::editor::diff_label(&display.path, display.committed.unwrap_or(false))
        }
        DisplayKind::Card => String::new(),
    }
}

pub(super) fn browser_label(display: &Display) -> String {
    if display.is_new_tab() {
        return "New tab".to_owned();
    }
    if let Some(title) = display.title.as_deref().filter(|title| !title.is_empty()) {
        return title.to_owned();
    }
    let url = display
        .url
        .as_deref()
        .unwrap_or(crate::view_layout::BLANK_PAGE);
    if let Some((path, _)) = hide_platform::path::from_file_url(url) {
        return file_label(&path);
    }
    url.split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest))
        .filter(|host| !host.is_empty())
        .unwrap_or(url)
        .to_owned()
}
