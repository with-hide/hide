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

use super::documents::OpenRequestFields;
use super::workspace_view::{AreaIntent, WorkspaceKey, file_label};
use super::*;
use crate::model::{
    BrowserOpenReceiptSnapshot, ViewAreaSnapshot, ViewDisplaySnapshot, ViewDisplayState,
    ViewLayoutSnapshot, ViewLimitsSnapshot, ViewNodeSnapshot, ViewSplitSnapshot,
};
use crate::view_layout::{
    Display, DisplayKind, Edge, ExternalFile, ExternalKind, Layout, LayoutError, MAX_SPLIT_DEPTH,
    MAX_VIEW_AREAS, MAX_VIEW_DISPLAYS, Node, browser_address, browser_title, is_file_address,
};

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
}

/// Where a document being read shows once it lands: the Workspace and the
/// area the operator asked in, and how.
#[derive(Clone, Debug)]
pub(super) struct ViewPlacement {
    pub(super) key: WorkspaceKey,
    pub(super) owner: Option<String>,
    pub(super) area: String,
    pub(super) preview: bool,
    pub(super) beside: bool,
    pub(super) replace: Option<String>,
}

impl ViewPlacement {
    pub(super) fn same_destination(&self, other: &Self) -> bool {
        self.key == other.key
            && self.owner == other.owner
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
    generation: u64,
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
                    let source = source_key(&key, &display.path, roots);
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
    path: &str,
    kind: DisplayKind,
    committed: Option<bool>,
) -> Option<String> {
    displays
        .iter()
        .filter(|display| display.shows(path, kind, committed))
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

/// The view `make_view_room` closes.
struct OldestView {
    display_id: String,
    kind: DisplayKind,
    document: Option<String>,
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

/// Brings one Workspace's displays in line with the editor's tabs. Returns
/// whether anything the file stores changed, and the documents the display
/// cap kept off screen.
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

impl Runtime {
    pub(super) fn view_layout_of(&self, key: &WorkspaceKey) -> Option<&Layout> {
        self.workspace_views
            .as_ref()?
            .views
            .get(&key.0, &key.1)
            .map(|view| &view.layout)
    }

    /// Every View of the Workspace, in tree order.
    pub(super) fn view_displays(&self, key: &WorkspaceKey) -> impl Iterator<Item = &Display> {
        self.view_layout_of(key)
            .into_iter()
            .flat_map(|layout| layout.displays())
    }

    /// Every Workspace's layout, each with the Workspace it belongs to.
    fn every_view_layout(&self) -> impl Iterator<Item = (WorkspaceKey, &Layout)> {
        self.workspace_views.iter().flat_map(|store| {
            store
                .views
                .workspaces
                .iter()
                .map(|view| ((view.device_id.clone(), view.path.clone()), &view.layout))
        })
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
            .and_then(|key| self.view_layout_of(&key))
            .and_then(|layout| layout.display(display_id))
            .map(|_| ())
            .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))
    }

    /// A display of the checkout just brought forward takes its View area,
    /// shown with the Agent area if only Agents showed (D-08). Returns
    /// whether the layout changed.
    pub(super) fn focus_view_display_of(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        display_id: &str,
    ) -> bool {
        let Some(key) = self.workspace_key(workspace_id, checkout_id) else {
            return false;
        };
        self.apply_area_intent_to(&key, AreaIntent::Documents);
        match self.change_view_layout(&key, |layout, stamp| {
            layout
                .focus(display_id, stamp)
                .map(|changed| (changed, changed))
        }) {
            Ok(changed) => changed,
            Err(error) => {
                self.set_error(error.kind(), error.message(), false);
                true
            }
        }
    }

    /// Applies one change to a Workspace's layout, created when the file has
    /// none for it, and saves it when it changed something.
    pub(super) fn change_view_layout<T>(
        &mut self,
        key: &WorkspaceKey,
        change: impl FnOnce(&mut Layout, u64) -> Result<(T, bool), LayoutError>,
    ) -> Result<T, LayoutError> {
        let now = unix_milliseconds();
        let Some(store) = self.workspace_views.as_mut() else {
            return Err(LayoutError::UnknownArea(key.1.clone()));
        };
        let entry = store.views.entry(&key.0, &key.1);
        let before = entry.layout.clone();
        let stamp = entry.layout.next_stamp(now);
        let (value, changed) = match change(&mut entry.layout, stamp) {
            Ok(done) => done,
            Err(error) => {
                entry.layout = before;
                return Err(error);
            }
        };
        if !changed {
            entry.layout = before;
            return Ok(value);
        }
        store.generation += 1;
        self.persist_workspace_views();
        Ok(value)
    }

    /// Whether an open may add what it would add to the Workspace `key`; a
    /// refusal is set as the error and nothing changes (contract 2). An open
    /// of a document already shown is a focus and is never refused.
    pub(super) fn admit_view_open(
        &mut self,
        key: &WorkspaceKey,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        preview: bool,
        beside: bool,
    ) -> bool {
        let Some(layout) = self.view_layout_of(key) else {
            return true;
        };
        let shown = layout.display_count() + self.pending_view_placements(key);
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
                            .any(|display| display.shows(path, kind, committed))
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
        } else if self.new_tab_target(key, false).is_some()
            || layout
                .displays()
                .any(|display| display.shows(path, kind, committed))
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
            refusal.kind != LayoutError::DisplayLimit.kind() || !self.make_view_room(key)
        });
        match refusal {
            Some(refusal) => {
                self.set_error(refusal.kind, refusal.message, false);
                false
            }
            None => true,
        }
    }

    /// Room for one more view in the Workspace `key`. While its views and the
    /// opens still being read fill `MAX_VIEW_DISPLAYS`, the view looked at
    /// longest ago closes. A view in front of any area and a document with
    /// unsaved work never close, and when too few can close nothing does.
    /// Returns whether there is room (PRD lane-files-checkout-views D-07).
    pub(super) fn make_view_room(&mut self, key: &WorkspaceKey) -> bool {
        let shown = self.view_displays(key).count() + self.pending_view_placements(key);
        let count = (shown + 1).saturating_sub(MAX_VIEW_DISPLAYS);
        if count == 0 {
            return true;
        }
        let oldest = self.closable_views(key);
        if oldest.len() < count {
            return false;
        }
        oldest
            .into_iter()
            .take(count)
            .all(|view| self.close_oldest_view(key, view))
    }

    /// Whether a caller's open adds a view rather than focus one the
    /// Workspace already holds, so room is made for it first. `path` is a file
    /// or diff open's read path.
    pub(super) fn open_adds_view(
        &self,
        key: &WorkspaceKey,
        action: &crate::workspace_control::Action,
        path: Option<&str>,
    ) -> bool {
        use crate::workspace_control::Action;
        let holds = |held: &dyn Fn(&Display) -> bool| self.view_displays(key).any(held);
        match (action, path) {
            (Action::OpenFile { .. }, Some(path)) => {
                !holds(&|display| display.shows(path, DisplayKind::File, None))
            }
            (Action::OpenDiff { .. }, Some(path)) => {
                !holds(&|display| display.shows(path, DisplayKind::Diff, Some(false)))
            }
            (Action::OpenBrowser { url, .. }, _) => {
                url.is_empty()
                    || !holds(&|display| {
                        display.kind == DisplayKind::Browser && display.url.as_deref() == Some(url)
                    })
            }
            _ => false,
        }
    }

    /// The views that may close, looked at longest ago first.
    fn closable_views(&self, key: &WorkspaceKey) -> Vec<OldestView> {
        let Some(layout) = self.view_layout_of(key) else {
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
            // A file still being restored has no document to close yet.
            .filter(|display| match &display.tab_id {
                Some(tab_id) => !self.document_kept(tab_id),
                None => matches!(display.kind, DisplayKind::Browser | DisplayKind::External),
            })
            .collect();
        closable.sort_by_key(|display| display.last_focused_unix_ms);
        closable
            .into_iter()
            .map(|display| OldestView {
                display_id: display.id.clone(),
                kind: display.kind,
                document: display.tab_id.clone(),
            })
            .collect()
    }

    /// Takes the view out of the layout, and closes its document when no
    /// other view shows it, the way closing its last view does (so Reopen
    /// Closed brings a file back).
    fn close_oldest_view(&mut self, key: &WorkspaceKey, oldest: OldestView) -> bool {
        let removed = self.change_view_layout(key, |layout, _| {
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
            && !self.every_view_layout().any(|(_, layout)| {
                layout
                    .displays()
                    .any(|display| display.tab_id.as_ref() == Some(&document))
            })
        {
            self.close_file_tab_now(&document);
        }
        true
    }

    fn new_tab_target(&self, key: &WorkspaceKey, beside: bool) -> Option<String> {
        if beside {
            return None;
        }
        let area = self.view_layout_of(key)?.active_area();
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
        if !self.admit_view_open(&key, path, DisplayKind::File, None, preview, beside) {
            return;
        }
        // The Explorer shows the Workspace's own tree, which holds no file of
        // another checkout to select.
        if self.workspace_key(workspace_id, checkout_id).as_ref() == Some(&key) {
            self.snapshot.ui_state.selected_path = Some(path.to_owned());
        }
        let replace = self.new_tab_target(&key, beside);
        match prepared {
            super::editor::PreparedFileTab::Open(tab_id) => {
                self.place_document(&key, &tab_id, preview, beside, None, replace.as_deref());
            }
            read @ super::editor::PreparedFileTab::Read { .. } => {
                if let Some(tab_id) =
                    self.insert_file_tab(read, workspace_id, checkout_id, path, preview)
                {
                    self.place_document(&key, &tab_id, preview, beside, None, replace.as_deref());
                }
            }
            super::editor::PreparedFileTab::Reading { root, channel } => {
                // A display that shows the file but could not read it is
                // focused now, and the read fills its tab (S6 B20).
                let shown = if !beside && replace.is_none() {
                    match self.focus_view_document(&key, path, DisplayKind::File, None, !preview) {
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
                    owner: self.workspace_active_tab(&key),
                    area: self
                        .view_layout_of(&key)
                        .map(|layout| layout.active_area().id.clone())
                        .unwrap_or_else(|| Layout::default().active_area),
                    key: key.clone(),
                    preview,
                    beside,
                    replace,
                });
                self.start_document_open(
                    root,
                    channel,
                    OpenRequestFields {
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

    /// Finish a GUI read where it was asked: the Workspace's list. The tab it
    /// was asked in remembers the View, and while that tab is not the active
    /// one the open leaves the selection as it was (B7).
    pub(super) fn complete_file_in_view(
        &mut self,
        prepared: super::editor::PreparedFileTab,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        placement: ViewPlacement,
    ) {
        self.reconcile_view_displays();
        let background = placement
            .owner
            .clone()
            .filter(|owner| self.workspace_active_tab(&placement.key).as_ref() != Some(owner));
        let kept = background
            .is_some()
            .then(|| self.keep_front(&placement.key));
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
        if background.is_none() && self.front_workspace_key().as_ref() == Some(&placement.key) {
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
        if !self.admit_view_open(
            &key,
            path,
            DisplayKind::Diff,
            Some(committed),
            preview,
            beside,
        ) {
            return true;
        }
        let replace = self.new_tab_target(&key, beside);
        let tab_id = Self::diff_tab_id(workspace_id, checkout_id, path, committed);
        if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
            self.insert_diff_tab(workspace_id, checkout_id, path, committed, preview);
        }
        self.place_document(&key, &tab_id, preview, beside, None, replace.as_deref())
    }

    /// Focuses the display of a document that was focused last, keeping it
    /// open when `keep_open`. Returns whether the Workspace shows it at all.
    fn focus_view_document(
        &mut self,
        key: &WorkspaceKey,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
        keep_open: bool,
    ) -> Result<bool, LayoutError> {
        let Some(display_id) = self.view_layout_of(key).and_then(|layout| {
            last_focused_showing(
                &layout.displays().collect::<Vec<_>>(),
                path,
                kind,
                committed,
            )
        }) else {
            return Ok(false);
        };
        self.change_view_layout(key, |layout, stamp| {
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
    pub(super) fn place_document(
        &mut self,
        key: &WorkspaceKey,
        tab_id: &str,
        preview: bool,
        beside: bool,
        area: Option<&str>,
        replace: Option<&str>,
    ) -> bool {
        match self.try_place_document(key, tab_id, preview, beside, area, replace) {
            Ok(changed) => changed,
            Err(error) => {
                self.refuse_placement(key, tab_id, error);
                true
            }
        }
    }

    fn try_place_document(
        &mut self,
        key: &WorkspaceKey,
        tab_id: &str,
        preview: bool,
        beside: bool,
        area: Option<&str>,
        replace: Option<&str>,
    ) -> Result<bool, LayoutError> {
        let placed = self.try_place_document_at(key, tab_id, preview, beside, area, replace)?;
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

    fn try_place_document_at(
        &mut self,
        key: &WorkspaceKey,
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
        let base = match (self.view_layout_of(key), area) {
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
            (None, _) => Layout::default().active_area,
        };
        if beside {
            return self.place_beside(key, &base, tab_id, &path, kind, committed);
        }
        if let Some(id) = replace {
            // The read belongs to the empty tab captured at open. A close or
            // navigation meanwhile must never overwrite the operator's newer work.
            let placed = self.change_view_layout(key, |layout, stamp| {
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
                    .filter(|d| d.id != id && d.shows(&path, kind, committed))
                    .map(|d| d.id.clone())
                    .collect();
                for twin in twins {
                    layout.remove(&twin);
                }
                let display = layout.display_mut(id).unwrap();
                display.kind = kind;
                display.path = path.clone();
                display.tab_id = Some(tab_id.to_owned());
                display.committed =
                    (kind == DisplayKind::Diff).then_some(committed.unwrap_or(false));
                display.url = None;
                display.title = None;
                display.load = 0;
                display.preview = false;
                display.opener = None;
                layout.focus(id, stamp)?;
                Ok(((), true))
            });
            placed?;
            return Ok(true);
        }
        if self.focus_view_document(key, &path, kind, committed, !preview)? {
            return Ok(true);
        }
        // The preview display the open takes, with the document it showed.
        let slot = preview
            .then(|| {
                let area = self.view_layout_of(key)?.area(&base)?;
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
        let placed = self.change_view_layout(key, |layout, stamp| {
            match &slot {
                Some((slot_id, _, _, _)) if !old_kept => {
                    let display = layout
                        .display_mut(slot_id)
                        .ok_or_else(|| LayoutError::UnknownDisplay(slot_id.clone()))?;
                    display.tab_id = Some(tab_id.to_owned());
                    display.path = path.clone();
                    display.kind = kind;
                    display.committed =
                        (kind == DisplayKind::Diff).then_some(committed.unwrap_or(false));
                    // The preview shows another document now, opened by
                    // whoever opens it.
                    display.opener = None;
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
    fn place_beside(
        &mut self,
        key: &WorkspaceKey,
        base: &str,
        tab_id: &str,
        path: &str,
        kind: DisplayKind,
        committed: Option<bool>,
    ) -> Result<bool, LayoutError> {
        let placed = self.change_view_layout(key, |layout, stamp| {
            let neighbour = BESIDE_ORDER
                .iter()
                .find_map(|edge| layout.neighbour(base, *edge));
            let Some(target) = neighbour else {
                let mut display = layout.new_display(path, kind, committed, false);
                display.tab_id = Some(tab_id.to_owned());
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
        if shown || self.document_kept(tab_id) {
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
                if shown {
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
            for display in view.layout.displays_mut() {
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
        if !self.area_workspace_is_current(&key, "view") {
            return true;
        }
        // The action names displays by what the last frame showed; bring
        // their bindings up to date first.
        self.reconcile_view_displays();
        let outcome = match action {
            ViewLayoutAction::Focus { display_id } => {
                self.change_view_layout(&key, |layout, stamp| {
                    layout
                        .focus(&display_id, stamp)
                        .map(|changed| (changed, changed))
                })
            }
            ViewLayoutAction::FocusArea { area_id } => {
                self.change_view_layout(&key, |layout, stamp| {
                    layout
                        .focus_area(&area_id, stamp)
                        .map(|changed| (changed, changed))
                })
            }
            ViewLayoutAction::Move {
                display_id,
                area_id,
                index,
            } => self.change_view_layout(&key, |layout, stamp| {
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
                let split = self.change_view_layout(&key, |layout, stamp| {
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
                self.change_view_layout(&key, |layout, _| {
                    layout
                        .resize(&split_id, ratio)
                        .map(|changed| (changed, changed))
                })
            }
            ViewLayoutAction::KeepOpen { display_id } => {
                self.change_view_layout(&key, |layout, _| {
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
            } => return self.close_view_display(&key, &display_id, pending_save, discard),
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
            .and_then(|pane| self.opener_of_pane(pane));
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
            request_id,
        } = payload;
        let outcome = ExternalFile::admitted(&grant, &name, kind)
            .map_err(external_refusal)
            .and_then(|file| self.place_external(file, workspace, pane_id.as_deref(), reveal));
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
        let shown = |display: &Display| {
            display
                .external
                .as_ref()
                .is_some_and(|shown| shown.grant == file.grant)
        };
        if !self.view_displays(&key).any(shown) {
            self.make_view_room(&key);
        }
        let before_load = self
            .workspace_views
            .as_ref()
            .map(|store| store.browser_load);
        let load = self.next_browser_load();
        let placed = self
            .change_view_layout(&key, |layout, stamp| {
                let existing = layout.displays().find(|d| shown(d)).map(|d| d.id.clone());
                if let Some(display_id) = existing {
                    // The same file clicked again comes forward and is read
                    // again from the disk (B10).
                    layout.focus(&display_id, stamp)?;
                    if let Some(display) = layout.display_mut(&display_id) {
                        display.load = load;
                        display.external = Some(file.clone());
                    }
                    return Ok((display_id, true));
                }
                let display = layout.new_external_display(file.clone(), load);
                let display_id = display.id.clone();
                let area = layout.active_area().id.clone();
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
        let stale: Vec<WorkspaceKey> = self
            .every_view_layout()
            .filter(|(_, layout)| {
                layout.displays().any(|display| {
                    display.kind == DisplayKind::External
                        && !display
                            .external
                            .as_ref()
                            .is_some_and(|file| live.contains(file.grant.as_str()))
                })
            })
            .map(|(key, _)| key)
            .collect();
        let mut dropped = 0;
        for key in &stale {
            let _ = self.change_view_layout(key, |layout, _| {
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
        opener: Option<crate::view_feedback::Opener>,
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
                if !self.catalog_has_workspace(&key) {
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
        let bound_area = area_id.map(str::to_owned).or_else(|| {
            beside_display.and_then(|id| {
                self.view_layout_of(&key)
                    .and_then(|layout| layout.area_of(id).map(|area| area.id.clone()))
            })
        });
        let caller = pane_id
            .map(str::to_owned)
            .unwrap_or_else(|| crate::workspace_control::checkout_caller_id("browser", &key.1));
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
        if self.open_adds_view(&key, &action, None) {
            self.make_view_room(&key);
        }
        // A pane of a tab the operator is not looking at leaves the selection
        // and the lane as they were, unless it asked to reveal (B7, B8).
        let kept = (!reveal)
            .then(|| self.background_caller_tab(&key, &caller))
            .flatten()
            .map(|_| self.keep_front(&key));
        let before_load = self
            .workspace_views
            .as_ref()
            .map(|store| store.browser_load);
        let load = self.next_browser_load();
        let placed = self
            .change_view_layout(&key, |layout, stamp| {
                let shown = layout
                    .displays()
                    .find(|display| {
                        !url.is_empty()
                            && display.kind == DisplayKind::Browser
                            && display.url.as_deref() == Some(url)
                    })
                    .map(|display| display.id.clone());
                if let Some(display_id) = shown {
                    layout.focus(&display_id, stamp)?;
                    if let Some(display) = layout.display_mut(&display_id) {
                        display.load = load;
                        if opener.is_some() {
                            display.opener = opener;
                        }
                    }
                    return Ok((display_id, true));
                }
                let mut display = layout.new_browser_display(url, load);
                display.opener = opener;
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
                let area = area_id.unwrap_or(&layout.active_area().id).to_owned();
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
        match kept {
            Some(kept) => self.restore_front(kept),
            None => self.apply_area_intent_to(&key, AreaIntent::Documents),
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
        let outcome = self.change_view_layout(key, |layout, _| {
            let display = layout
                .display_mut(display_id)
                .filter(|display| display.kind == DisplayKind::Browser)
                .ok_or_else(|| LayoutError::UnknownDisplay(display_id.to_owned()))?;
            let changed = display.url.as_deref() != Some(url.as_str());
            if changed {
                display.url = Some(url);
                display.title = None;
                // The operator's own address makes the page the operator's
                // (D-03, B4).
                display.opener = None;
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
            .view_layout_of(&key)
            .and_then(|layout| layout.display(&display_id))
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
                        view.layout
                            .displays()
                            .filter(|display| display.kind == DisplayKind::Browser)
                            .map(|display| {
                                (
                                    view.device_id.clone(),
                                    view.path.clone(),
                                    display.id.clone(),
                                )
                            })
                    })
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
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let Some(view) = store.views.get_mut(&key.0, &key.1) else {
            return false;
        };
        let Some(display) = view.layout.display_mut(&display_id) else {
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
    fn pane_workspace_key(&self, pane_id: &str) -> Option<WorkspaceKey> {
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

    /// Close View (B10): a display another display's document also shows,
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
    fn close_view_display(
        &mut self,
        key: &WorkspaceKey,
        display_id: &str,
        pending_save: Option<FileSavePayload>,
        discard: bool,
    ) -> bool {
        let Some(layout) = self.view_layout_of(key) else {
            return self.close_absent_display(key, display_id);
        };
        let Some(display) = layout.display(display_id) else {
            return self.close_absent_display(key, display_id);
        };
        let (path, kind, committed) = (display.path.clone(), display.kind, display.committed);
        let tab = display.tab_id.clone();
        // A document can show in another Workspace too, a file a terminal
        // link opened where the operator was, and stays open for it.
        let shared = match &tab {
            Some(tab) => self.every_view_layout().any(|(view, layout)| {
                layout.displays().any(|other| {
                    other.tab_id.as_ref() == Some(tab) && (&view != key || other.id != display_id)
                })
            }),
            None => self
                .view_displays(key)
                .any(|other| other.id != display_id && other.shows(&path, kind, committed)),
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
        let _ = self.change_view_layout(key, |layout, _| {
            let removed = layout.remove(display_id).is_some();
            Ok(((), removed))
        });
        if tab.is_none()
            && !shared
            && kind == DisplayKind::File
            && let Some((workspace_id, checkout_id)) = self.display_source(key, &path)
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
        let Some(display) = self
            .view_layout_of(key)
            .and_then(|layout| layout.display(display_id))
            .cloned()
        else {
            let error = LayoutError::UnknownDisplay(display_id.to_owned());
            self.set_error(error.kind(), error.message(), false);
            return true;
        };
        let front = self.front_checkout_owned();
        let (state, reason) = self.display_state(&self.display_facts(key), &display);
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
                let Some((workspace_id, checkout_id)) = front else {
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
                let (workspace_id, checkout_id) = self
                    .display_source(key, &display.path)
                    .unwrap_or((workspace_id, checkout_id));
                match self.document_source(&workspace_id, &checkout_id) {
                    Ok((root, channel)) => self.start_document_open(
                        root,
                        channel,
                        OpenRequestFields {
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
    /// is started in Herdr, and nothing takes the screen: the tree was
    /// published already, and each display binds to its tab when it lands.
    pub(super) fn restore_view_displays(&mut self, key: &WorkspaceKey) {
        let Some(front) = self.front_checkout_owned() else {
            return;
        };
        let Some(layout) = self.view_layout_of(key) else {
            return;
        };
        let areas = layout.area_count();
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
                DisplayKind::Diff | DisplayKind::Browser | DisplayKind::External => None,
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
                DisplayKind::Browser | DisplayKind::External => {}
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
        if store.reconciled.as_ref().is_some_and(|seen| {
            seen.generation == store.generation
                && seen.front == store.front
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
        let shown = ShownByViews::of(self.every_view_layout(), &roots);
        let now = unix_milliseconds();
        let store = self.workspace_views.as_mut().expect("checked above");
        let mut stored = false;
        let mut every_preview: HashMap<String, bool> = HashMap::new();
        let mut unshown: Vec<(String, String, String)> = Vec::new();
        for view in &mut store.views.workspaces {
            let key = (view.device_id.clone(), view.path.clone());
            let requested = requested.as_deref().filter(|requested| {
                tabs.iter()
                    .any(|tab| tab.id == *requested && tab.key.as_ref() == Some(&key))
            });
            // A document another Workspace shows, a file a terminal link
            // opened where the operator was, is not one to add here.
            let owned: HashSet<String> = tabs
                .iter()
                .filter(|tab| tab.key.as_ref() == Some(&key) && shown.elsewhere(tab, &key))
                .map(|tab| tab.id.clone())
                .collect();
            let (changed, capped) = reconcile_layout(
                &mut view.layout,
                &key,
                store.live.contains(&key),
                &owned,
                &tabs,
                &roots,
                requested,
                now,
            );
            stored |= changed;
            unshown.extend(
                capped
                    .into_iter()
                    .map(|tab| (tab.id.clone(), tab.path.clone(), key.1.clone())),
            );
            for display in view.layout.displays() {
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
                        "{path} is open but not shown: {workspace} has no room for another view ({MAX_VIEW_DISPLAYS}), and it shows once one is closed"
                    ),
                );
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
        let store = self.workspace_views.as_mut().expect("checked above");
        store.derived_active = self.snapshot.editor.active_tab_id.clone();
        store.reconciled = Some(Reconciled {
            tabs: self.snapshot.editor.tabs.clone(),
            active: self.snapshot.editor.active_tab_id.clone(),
            front: store.front.clone(),
            generation: store.generation,
        });
    }

    /// The document the front Workspace's active area shows, while it can
    /// show it (contract 3).
    fn front_active_view_tab(&self) -> Option<String> {
        let store = self.workspace_views.as_ref()?;
        let key = store.front.as_ref()?;
        let layout = &store.views.get(&key.0, &key.1)?.layout;
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

    /// The front Workspace's area-active displays while its View areas are on
    /// screen: what is shown, one per area, in tree order.
    fn visible_view_displays(&self) -> Vec<&Display> {
        let Some(store) = self.workspace_views.as_ref() else {
            return Vec::new();
        };
        let Some(view) = store
            .front
            .as_ref()
            .and_then(|key| store.views.get(&key.0, &key.1))
        else {
            return Vec::new();
        };
        if !view.shows_documents() {
            return Vec::new();
        }
        view.layout
            .areas()
            .into_iter()
            .filter_map(|area| {
                let active = area.active.as_ref()?;
                area.displays.iter().find(|display| &display.id == active)
            })
            .collect()
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

    /// The checkout, by its catalog ids, that a display of the Workspace
    /// `key` reads the file `path` from (`source_key`).
    pub(super) fn display_source(
        &self,
        key: &WorkspaceKey,
        path: &str,
    ) -> Option<(String, String)> {
        let source = if hide_platform::path::wire_relative(&key.1, path).is_ok() {
            key.clone()
        } else {
            source_key(key, path, &self.checkout_roots())
        };
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
        }
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
        if matches!(display.kind, DisplayKind::Browser | DisplayKind::External) {
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
        if let Some(reason) = &facts.wait {
            return (ViewDisplayState::Waiting, Some(reason.clone()));
        }
        if display.kind == DisplayKind::File
            && self.display_source(&facts.key, &display.path).is_some_and(
                |(workspace_id, checkout_id)| {
                    self.document_opens_path(&workspace_id, &checkout_id, &display.path)
                },
            )
        {
            return (ViewDisplayState::Opening, None);
        }
        let reason = match display.kind {
            DisplayKind::File | DisplayKind::Browser | DisplayKind::External => {
                "This view's file is not open; Retry reads it again"
            }
            DisplayKind::Diff => "This view's diff is not open; Retry opens it again",
        };
        (ViewDisplayState::Unavailable, Some(reason.to_owned()))
    }

    /// The front Workspace's tree as the snapshot carries it. It runs on
    /// every snapshot read, over at most `MAX_VIEW_DISPLAYS` displays: one
    /// map of the editor's tabs and one root check, then a map lookup per
    /// display.
    pub(super) fn view_layout_snapshot(
        &self,
        key: &WorkspaceKey,
        layout: &Layout,
    ) -> ViewLayoutSnapshot {
        let facts = self.display_facts(key);
        ViewLayoutSnapshot {
            root: self.view_node_snapshot(&facts, &layout.root),
            active_area: layout.active_area().id.clone(),
            limits: ViewLimitsSnapshot {
                areas: MAX_VIEW_AREAS,
                depth: MAX_SPLIT_DEPTH,
                displays: MAX_VIEW_DISPLAYS,
            },
            display_count: layout.display_count(),
        }
    }

    fn view_node_snapshot(&self, facts: &DisplayFacts<'_>, node: &Node) -> ViewNodeSnapshot {
        match node {
            Node::Area(area) => ViewNodeSnapshot::Area(ViewAreaSnapshot {
                id: area.id.clone(),
                active: area.active.clone(),
                displays: area
                    .displays
                    .iter()
                    .map(|display| {
                        let (state, reason) = self.display_state(facts, display);
                        let tab = display
                            .tab_id
                            .as_deref()
                            .and_then(|id| facts.tabs.get(id).copied());
                        let (comments, not_delivered) =
                            facts.feedback.map_or((0, 0), |feedback| {
                                feedback.on(display).fold((0, 0), |(all, expired), comment| {
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
                                (Some(tab), _) => tab.label.clone(),
                                (None, DisplayKind::File) => file_label(&display.path),
                                (None, DisplayKind::Diff) => super::editor::diff_label(
                                    &display.path,
                                    display.committed.unwrap_or(false),
                                ),
                            },
                            kind: display.kind,
                            committed: display.committed,
                            preview: display.preview,
                            state,
                            reason,
                            url: display.url.clone(),
                            title: display.title.clone(),
                            load: display.load,
                            opener: display
                                .opener
                                .as_ref()
                                .map(|opener| self.view_opener_snapshot(opener)),
                            comments,
                            not_delivered,
                            external: display.external.clone(),
                        }
                    })
                    .collect(),
            }),
            Node::Split(split) => ViewNodeSnapshot::Split(ViewSplitSnapshot {
                id: split.id.clone(),
                axis: split.axis,
                ratio: split.ratio,
                first: Box::new(self.view_node_snapshot(facts, &split.first)),
                second: Box::new(self.view_node_snapshot(facts, &split.second)),
            }),
        }
    }
}

/// A page's name on its tab: its title once it has one, else its host (for
/// a file, its file name), else its address.
fn browser_label(display: &Display) -> String {
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
    if is_file_address(url) {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let name = path.rsplit('/').next().unwrap_or(path);
        return file_label(&percent_encoding::percent_decode_str(name).decode_utf8_lossy());
    }
    url.split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(rest))
        .filter(|host| !host.is_empty())
        .unwrap_or(url)
        .to_owned()
}
