//! Each Workspace's presentation in a shell that draws Agent and View areas
//! side by side (PRD S6 D-10, B8, B19, B20; S7 D-09, D-10, B14, B17; PRD
//! right-lane D-01, D-05, D-11; PRD lane-two-columns D-01, D-02, D-09, D-10,
//! D-20; PRD lane-desk-pair-view D-05, D-06, D-15, D-18, D-20).
//!
//! A Workspace is one checkout on one device, keyed by the device id and the
//! checkout path, because an engine workspace id names a workspace, not a checkout,
//! and one checkout can hold tabs from several engine workspaces. The
//! state is Hide's own presentation - the right lane (whether it is open,
//! the tool column beside the rail, the three widths), each Agent tab's tray
//! (its View areas with their displays, [`crate::view_layout`]), the shelf of
//! the checkout's Views on no desk, and the window's pair view mode
//! ([`PairMode`], D-45, D-47) - and never a
//! terminal layout: the engine keeps panes, splits and zoom. Whether an open
//! lane is floating over the agents right now, and an expanded lane, are the
//! shell's own moment and are never stored (D-11).
//!
//! A tray is what PRD lane-desk-pair-view calls a desk (책상): one Agent tab's
//! Views in front of the operator. The code name keeps clear of the Desk
//! feature (`crate::desk`, D-35).
//!
//! It lives in its own versioned file, apart from `core-state.json`, so a
//! shell that does not know it never reads it and an older build's settings
//! are never rewritten by it. A schema 1 file (S6, one strip of View tabs per
//! Workspace), a schema 2 file (the File Views and Tools columns, and the
//! side panel and layouts before them) and a schema 3 file (one lane showing
//! one tool, with the file tree as a column of its Files tool) load their
//! View lists, Agent layouts, groups and comments and start their lane
//! closed at the default widths (lane-two-columns D-10). A schema 4 or 5
//! file (one lane and one View list per checkout) moves once (lane-desk-pair-
//! view D-18): the checkout's View list becomes the desk of the Agent tab
//! the checkout showed last, a pinned lane opens unpinned, and a tool that is
//! gone reads as no tool column. All are written as schema 6 by the next
//! save. A file this build cannot read, of another version or one whose
//! migration fails, is moved aside, never overwritten, and the defaults load.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::view_layout::{DisplayKind, Layout};

pub const SCHEMA_VERSION: u32 = 6;

/// The Workspaces remembered at once. The least recently used is forgotten
/// first, so the file stays bounded however many checkouts come and go.
pub const MAX_WORKSPACES: usize = 256;

/// The widest a stored column width may be, in CSS pixels. The shell keeps
/// each above its own minimum and Agent Views above its own as it draws; the
/// core only keeps a crafted value from being absurd.
pub const MAX_WIDTH: u32 = 8192;

/// The tool the lane's tool column shows (PRD lane-two-columns D-03, D-07,
/// D-14): the Project's status and its Memory, then the checkout's Explorer
/// (the file tree) and History, in the rail's order. Memory is a tool of its
/// own, no longer a tab of the Project tool (operator decision 2026-10-23,
/// replacing D-20). The documents are no tool: they are the viewing column's
/// (`WorkspaceView::documents`). Missions are no tool either: they are the
/// page's mission screen (PRD mission-screen D-02), so a lane saved on the
/// retired `mission` tool reads as no tool column.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// The Project's status: its counts and what waits on the operator.
    Project,
    /// The Project's Memory.
    Memory,
    /// The file tree.
    Explorer,
    /// History, the checkout's changes.
    Changes,
}

impl Tool {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "memory" => Some(Self::Memory),
            "explorer" => Some(Self::Explorer),
            "changes" => Some(Self::Changes),
            _ => None,
        }
    }
}

/// The three layouts a Workspace stored before the side panel (schema 1 and
/// early schema 2), read only so the file still parses; the lane they
/// described is not carried over.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LegacyMode {
    Agents,
    Together,
    Views,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkspaceView {
    pub device_id: String,
    pub path: String,
    /// Whether the lane is open: the front Agent tab's desk is drawn while
    /// that desk is `shown`, and the tool column while there is a `tool`. A
    /// close keeps both, so ⌘E brings the lane back in the shape it had
    /// (lane-two-columns B15, lane-desk-pair-view B30). It stays the
    /// Workspace's because the tool column it closes with the desk is the
    /// Workspace's (D-14). A lane the operator hid by going back to a
    /// terminal stays open: hiding it is the shell's moment, not a close.
    pub open: bool,
    /// The tool column's tool, or `None` while there is no tool column. It
    /// stays as it is when the operator moves between Agent tabs (D-14).
    pub tool: Option<Tool>,
    /// The desk's width, in CSS pixels, once the operator resized it; until
    /// then the shell's default. Every tab's desk is drawn at it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_width: Option<u32>,
    /// The tool column's width while it shows the Explorer, the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explorer_width: Option<u32>,
    /// The tool column's width for the Project and Changes, the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_width: Option<u32>,
    /// When the operator last chose this checkout; `0` until they do. The
    /// checkout resumed after a restart is the latest of these.
    pub last_used_unix_ms: u64,
    /// When this entry was made. The cap counts it as a use, so an entry
    /// made for a checkout nobody has opened yet, such as a project just
    /// added, is not the first one forgotten (`WorkspaceViews::entry`).
    pub made_unix_ms: u64,
    /// Each Agent tab's desk (PRD lane-desk-pair-view D-06): the Views the
    /// operator put in front of that tab, with their View areas. One per tab,
    /// at most [`MAX_TRAYS`].
    pub trays: Vec<Tray>,
    /// The checkout's Views on no desk (D-19): what an agent the operator
    /// was not watching opened, kept in its pane's list until the operator
    /// puts it on a desk or it closes. One area, never drawn; at most
    /// [`crate::view_layout::MAX_VIEW_DISPLAYS`], the View looked at longest
    /// ago closing first.
    pub shelf: Layout,
    pub agent_layout: crate::agent_layout::Layout,
    /// Named collections are additive: the original tree remains ungrouped.
    pub agent_groups: crate::agent_groups::Groups,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) pane_moves: std::collections::BTreeMap<String, crate::runtime::pane_move::Intent>,
    pub(crate) agent_starts:
        std::collections::BTreeMap<String, crate::runtime::group_starts::StartIntent>,
    /// The operator's comments on this checkout's Views (PRD
    /// view-opener-feedback D-13). Left out of the file while empty.
    #[serde(skip_serializing_if = "crate::view_feedback::Feedback::is_empty")]
    pub feedback: crate::view_feedback::Feedback,
}

/// The most trays one checkout keeps: one per Agent tab, which the Agent
/// layout caps at the same number. A desk past it is one of a tab the engine
/// no longer lists, pruned as the topology arrives.
pub const MAX_TRAYS: usize = 64;

/// The tab id a desk carries before its Agent tab is known: an older file's
/// checkout documents whose Agent layout named no tab (D-18). The first tab
/// the Workspace shows takes it (`WorkspaceView::adopt_pending_tray`).
pub const PENDING_TRAY: &str = "";

/// One Agent tab's desk (PRD lane-desk-pair-view D-05, D-06, D-07): the Views
/// the operator opened there, and those the agent they watched opened, as
/// tabs in View areas, in the order they were opened.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Tray {
    /// The engine's id of the Agent tab whose desk this is.
    pub tab: String,
    /// The desk's View areas and their tabs.
    pub layout: Layout,
    /// Whether the lane shows this desk while it is open: an open the
    /// operator makes shows it, its last tab closing or the desk button
    /// folding it hides it (D-07, B11).
    #[serde(default)]
    pub shown: bool,
}

/// Pair view (PRD lane-desk-pair-view D-08, D-45, D-47): one mode for the
/// window, not a tab's. While it is on, the pane the operator is on is drawn
/// beside its tab's desk wherever they go; a pane with nothing to show is
/// drawn alone at the body's full width and the mode stays on (D-53). The
/// left pane is never saved:
/// it is the pane the operator is on (`runtime::pair_view`). A file an
/// earlier build wrote with a pair view on a tray (D-15) loads with the mode
/// off, the tray's `pair` ignored.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PairMode {
    #[serde(default)]
    pub on: bool,
    /// The left pane's share of the body, in thousandths, once the operator
    /// dragged the divider (D-47); half until then. Kept while the mode is
    /// off, so entering again draws the same split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<u16>,
}

/// The narrowest and widest share of the body the left pane may have, in
/// thousandths; the shell keeps each side above its own minimum width.
pub const PAIR_RATIO_MIN: u16 = 100;
pub const PAIR_RATIO_MAX: u16 = 900;

/// A stored or sent ratio inside what the core accepts.
pub fn clamp_pair_ratio(value: u16) -> u16 {
    value.clamp(PAIR_RATIO_MIN, PAIR_RATIO_MAX)
}

/// Where one of a checkout's Views sits: on one Agent tab's desk, or on the
/// shelf of Views on no desk (D-19, D-20). A View is in exactly one place.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Place {
    Tray(String),
    Shelf,
}

impl Tray {
    /// An empty desk whose area takes the next id of the checkout, so area
    /// and View ids stay unique across every desk and the shelf.
    fn new(tab: &str, next_id: u64) -> Self {
        Self {
            tab: tab.to_owned(),
            layout: fresh_layout(next_id),
            shown: false,
        }
    }
}

/// The one area of the shelf. The shelf is never drawn and no command
/// addresses its area, so it takes no number from the count the desks' areas
/// share: the first desk's area is `a1`, as the documents' was.
pub const SHELF_AREA: &str = "shelf";

/// An empty shelf whose next id is `next_id`.
fn shelf_layout(next_id: u64) -> Layout {
    let mut layout = Layout {
        next_id: next_id.max(1),
        ..Layout::default()
    };
    if let crate::view_layout::Node::Area(root) = &mut layout.root {
        root.id = SHELF_AREA.to_owned();
    }
    layout.active_area = SHELF_AREA.to_owned();
    layout
}

/// One empty area numbered from `next_id`.
fn fresh_layout(next_id: u64) -> Layout {
    let mut layout = Layout {
        next_id: next_id.max(1),
        ..Layout::default()
    };
    let area = layout.mint('a');
    if let crate::view_layout::Node::Area(root) = &mut layout.root {
        root.id = area.clone();
    }
    layout.active_area = area;
    layout
}

/// A stored schema 6 entry: the lane, its trays and its shelf.
#[derive(Deserialize)]
struct StoredTrays {
    device_id: String,
    path: String,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    document_width: Option<u32>,
    #[serde(default)]
    explorer_width: Option<u32>,
    #[serde(default)]
    panel_width: Option<u32>,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default)]
    made_unix_ms: u64,
    #[serde(default)]
    trays: Vec<Tray>,
    #[serde(default)]
    shelf: Option<Layout>,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
    #[serde(default)]
    agent_groups: crate::agent_groups::Groups,
    #[serde(default)]
    agent_starts: std::collections::BTreeMap<String, crate::runtime::group_starts::StartIntent>,
    #[serde(default)]
    pane_moves: std::collections::BTreeMap<String, crate::runtime::pane_move::Intent>,
    #[serde(default)]
    feedback: crate::view_feedback::Feedback,
}

impl StoredTrays {
    fn into_view(self) -> WorkspaceView {
        WorkspaceView {
            device_id: self.device_id,
            path: self.path,
            open: self.open,
            tool: self.tool.as_deref().and_then(Tool::parse),
            document_width: self.document_width,
            explorer_width: self.explorer_width,
            panel_width: self.panel_width,
            last_used_unix_ms: self.last_used_unix_ms,
            made_unix_ms: self.made_unix_ms,
            trays: self.trays,
            shelf: self.shelf.unwrap_or_else(|| shelf_layout(1)),
            agent_layout: self.agent_layout,
            agent_groups: self.agent_groups,
            agent_starts: self.agent_starts,
            pane_moves: self.pane_moves,
            feedback: self.feedback,
        }
    }
}

/// A stored schema 5 or schema 4 entry: one lane per checkout, with the
/// checkout's one View list. It moves once (PRD lane-desk-pair-view D-18):
/// the View list becomes the desk of the Agent tab the checkout showed last,
/// shown when the documents were, and a pinned lane opens unpinned; the 보기
/// slot's width is dropped. A tool this build no longer has (a schema 4
/// file's `prs`, or the retired `mission`) reads as no tool column.
#[derive(Deserialize)]
struct StoredLane {
    device_id: String,
    path: String,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    documents: bool,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    document_width: Option<u32>,
    #[serde(default)]
    explorer_width: Option<u32>,
    #[serde(default)]
    panel_width: Option<u32>,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default)]
    made_unix_ms: u64,
    #[serde(default, rename = "view_layout")]
    layout: Layout,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
    #[serde(default)]
    agent_groups: crate::agent_groups::Groups,
    #[serde(default)]
    agent_starts: std::collections::BTreeMap<String, crate::runtime::group_starts::StartIntent>,
    #[serde(default)]
    pane_moves: std::collections::BTreeMap<String, crate::runtime::pane_move::Intent>,
    #[serde(default)]
    feedback: crate::view_feedback::Feedback,
}

impl StoredLane {
    fn into_view(self) -> WorkspaceView {
        let mut view = WorkspaceView {
            open: self.open,
            tool: self.tool.as_deref().and_then(Tool::parse),
            document_width: self.document_width,
            explorer_width: self.explorer_width,
            panel_width: self.panel_width,
            last_used_unix_ms: self.last_used_unix_ms,
            made_unix_ms: self.made_unix_ms,
            agent_layout: self.agent_layout,
            agent_groups: self.agent_groups,
            agent_starts: self.agent_starts,
            pane_moves: self.pane_moves,
            feedback: self.feedback,
            ..WorkspaceView::new(&self.device_id, &self.path)
        };
        view.adopt_checkout_documents(self.layout, self.documents);
        view
    }
}

/// A stored schema 3 entry: one lane showing one tool. Its lane (`tool`,
/// `last_tool`, `pinned`, `tree` and the widths) is not read, because the
/// lane has two columns now and the old shape is not carried over (D-10);
/// everything that is not the lane is.
#[derive(Deserialize)]
struct StoredOneToolLane {
    device_id: String,
    path: String,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default, rename = "view_layout")]
    layout: Layout,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
    #[serde(default)]
    agent_groups: crate::agent_groups::Groups,
    #[serde(default)]
    agent_starts: std::collections::BTreeMap<String, crate::runtime::group_starts::StartIntent>,
    #[serde(default)]
    pane_moves: std::collections::BTreeMap<String, crate::runtime::pane_move::Intent>,
    #[serde(default)]
    feedback: crate::view_feedback::Feedback,
}

impl StoredOneToolLane {
    fn into_view(self) -> WorkspaceView {
        let mut view = WorkspaceView {
            last_used_unix_ms: self.last_used_unix_ms,
            agent_layout: self.agent_layout,
            agent_groups: self.agent_groups,
            agent_starts: self.agent_starts,
            pane_moves: self.pane_moves,
            feedback: self.feedback,
            ..WorkspaceView::new(&self.device_id, &self.path)
        };
        view.adopt_checkout_documents(self.layout, false);
        view
    }
}

/// An entry as any build of schema 2 wrote it: the columns' fields, or the
/// side panel or the layout and boundary that came before them. Only the
/// View list, the Agent layout and the stamp are read; the keys of the
/// columns and panels (`views`, `tools`, `tool`, `panel`, `mode`, the widths
/// and shares) are dropped by not being named here.
#[derive(Deserialize)]
struct StoredColumns {
    device_id: String,
    path: String,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default)]
    layout: Layout,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
}

impl StoredColumns {
    fn into_view(self) -> WorkspaceView {
        let mut view = WorkspaceView {
            last_used_unix_ms: self.last_used_unix_ms,
            agent_layout: self.agent_layout,
            ..WorkspaceView::new(&self.device_id, &self.path)
        };
        view.adopt_checkout_documents(self.layout, false);
        view
    }
}

impl WorkspaceView {
    /// A Workspace seen for the first time shows Agent Views alone: the
    /// lane closed, no tool, no desk and the default widths (B1, B23).
    pub fn new(device_id: &str, path: &str) -> Self {
        Self {
            device_id: device_id.to_owned(),
            path: path.to_owned(),
            open: false,
            tool: None,
            document_width: None,
            explorer_width: None,
            panel_width: None,
            last_used_unix_ms: 0,
            made_unix_ms: 0,
            trays: Vec::new(),
            shelf: shelf_layout(1),
            agent_layout: crate::agent_layout::Layout::default(),
            agent_groups: Default::default(),
            agent_starts: Default::default(),
            pane_moves: Default::default(),
            feedback: Default::default(),
        }
    }

    pub fn is(&self, device_id: &str, path: &str) -> bool {
        self.device_id == device_id && self.path == path
    }

    /// An older file's checkout documents move once onto the desk of the
    /// Agent tab the checkout showed last, shown when the documents were
    /// (PRD lane-desk-pair-view D-18, B34); with no tab named they wait on
    /// the pending desk the first tab shown takes. Every View keeps its id,
    /// opener marks, unseen mark and comments.
    fn adopt_checkout_documents(&mut self, layout: Layout, shown: bool) {
        if layout.display_count() == 0 {
            return;
        }
        let tab = self
            .group_layout(self.agent_groups.selected.as_deref())
            .and_then(|agents| agents.active())
            .or_else(|| self.agent_layout.active())
            .unwrap_or(PENDING_TRAY)
            .to_owned();
        let next_id = layout.next_id;
        self.trays.push(Tray { tab, layout, shown });
        // The shelf numbers after the documents, so no id repeats.
        self.shelf = shelf_layout(next_id);
    }

    /// The next id no tree of the checkout has minted: every desk and the
    /// shelf number from one count, so a View, area or divider id names one
    /// thing in the checkout (`hide view split <view> --area <area>`).
    pub fn next_id(&self) -> u64 {
        self.trays
            .iter()
            .map(|tray| tray.layout.next_id)
            .chain(std::iter::once(self.shelf.next_id))
            .max()
            .unwrap_or(1)
    }

    /// Brings the tree at `place` up to the checkout's count before it
    /// mints anything.
    pub fn align_ids(&mut self, place: &Place) {
        let next = self.next_id();
        if let Some(layout) = self.layout_mut(place) {
            layout.next_id = layout.next_id.max(next);
        }
    }

    pub fn tray(&self, tab: &str) -> Option<&Tray> {
        self.trays.iter().find(|tray| tray.tab == tab)
    }

    pub fn tray_mut(&mut self, tab: &str) -> Option<&mut Tray> {
        self.trays.iter_mut().find(|tray| tray.tab == tab)
    }

    /// `tab`'s desk, made empty on first use.
    pub fn tray_entry(&mut self, tab: &str) -> &mut Tray {
        if let Some(index) = self.trays.iter().position(|tray| tray.tab == tab) {
            return &mut self.trays[index];
        }
        let tray = Tray::new(tab, self.next_id());
        self.trays.push(tray);
        self.trays.last_mut().expect("just pushed")
    }

    /// The tree at `place`; a desk not made yet has none.
    pub fn layout(&self, place: &Place) -> Option<&Layout> {
        match place {
            Place::Tray(tab) => self.tray(tab).map(|tray| &tray.layout),
            Place::Shelf => Some(&self.shelf),
        }
    }

    pub fn layout_mut(&mut self, place: &Place) -> Option<&mut Layout> {
        match place {
            Place::Tray(tab) => self.tray_mut(tab).map(|tray| &mut tray.layout),
            Place::Shelf => Some(&mut self.shelf),
        }
    }

    /// Every tree of the checkout with its place: each desk, then the shelf.
    pub fn layouts(&self) -> impl Iterator<Item = (Place, &Layout)> {
        self.trays
            .iter()
            .map(|tray| (Place::Tray(tray.tab.clone()), &tray.layout))
            .chain(std::iter::once((Place::Shelf, &self.shelf)))
    }

    pub fn layouts_mut(&mut self) -> impl Iterator<Item = &mut Layout> {
        self.trays
            .iter_mut()
            .map(|tray| &mut tray.layout)
            .chain(std::iter::once(&mut self.shelf))
    }

    /// Every View of the checkout, on a desk or not.
    pub fn views(&self) -> impl Iterator<Item = &crate::view_layout::Display> {
        self.trays
            .iter()
            .flat_map(|tray| tray.layout.displays())
            .chain(self.shelf.displays())
    }

    pub fn views_mut(&mut self) -> Vec<&mut crate::view_layout::Display> {
        let mut views = Vec::new();
        for layout in self.layouts_mut() {
            views.extend(layout.displays_mut());
        }
        views
    }

    pub fn view_count(&self) -> usize {
        self.trays
            .iter()
            .map(|tray| tray.layout.display_count())
            .sum::<usize>()
            + self.shelf.display_count()
    }

    pub fn view(&self, id: &str) -> Option<&crate::view_layout::Display> {
        self.views().find(|display| display.id == id)
    }

    pub fn view_mut(&mut self, id: &str) -> Option<&mut crate::view_layout::Display> {
        self.views_mut()
            .into_iter()
            .find(|display| display.id == id)
    }

    /// Where the View `id` sits.
    pub fn place_of(&self, id: &str) -> Option<Place> {
        self.layouts()
            .find(|(_, layout)| layout.display(id).is_some())
            .map(|(place, _)| place)
    }

    /// Moves the View `id` to `to`, at the end of its area in use, and
    /// returns where it was (D-20: one View, one desk). A View already there
    /// stays where it is in its tree. The tree it leaves collapses an area it
    /// emptied, as a close does.
    pub fn move_view(&mut self, id: &str, to: &Place) -> Option<Place> {
        let from = self.place_of(id)?;
        if &from == to {
            return Some(from);
        }
        if let Place::Tray(tab) = to {
            self.tray_entry(tab);
        }
        let display = self.layout_mut(&from)?.remove(id)?;
        let target = self.layout_mut(to).expect("made above");
        let area = target.active_area().id.clone();
        if target.append(&area, display.clone()).is_err() {
            // Past the target's cap: the View stays where it was.
            let back = self.layout_mut(&from).expect("it was there");
            let area = back.active_area().id.clone();
            let _ = back.append(&area, display);
            return None;
        }
        Some(from)
    }

    /// The pending desk an older file's documents wait on goes to `tab`,
    /// the first Agent tab the Workspace shows (D-18): taken whole when the
    /// tab has no desk yet, else its Views join the tab's. A View the tab's
    /// desk has no room for goes where `retire_tray` sends it: an agent's to
    /// the shelf, still in its pane's list, the operator's nowhere, since the
    /// shelf shows only agents' Views; it is counted. `None` when nothing was
    /// pending.
    pub fn adopt_pending_tray(&mut self, tab: &str) -> Option<Retired> {
        if tab == PENDING_TRAY {
            return None;
        }
        let index = self
            .trays
            .iter()
            .position(|tray| tray.tab == PENDING_TRAY)?;
        let pending = self.trays.remove(index);
        let mut retired = Retired::default();
        match self.trays.iter().position(|tray| tray.tab == tab) {
            None => {
                retired.inherited = pending.layout.display_count();
                self.trays.push(Tray {
                    tab: tab.to_owned(),
                    ..pending
                });
            }
            Some(at) => {
                self.trays[at].shown |= pending.shown;
                for display in pending.layout.displays().cloned().collect::<Vec<_>>() {
                    let layout = &mut self.trays[at].layout;
                    let area = layout.active_area().id.clone();
                    match layout.append(&area, display.clone()) {
                        Ok(()) => retired.inherited += 1,
                        Err(_) => self.shelve_or_drop(display, &mut retired),
                    }
                }
            }
        }
        Some(retired)
    }

    /// A View a desk has no room for: an agent's goes to the shelf, which
    /// its pane's list shows; the operator's, which no list shows, and one
    /// the shelf has no room for, are dropped. Counted in `retired`.
    fn shelve_or_drop(&mut self, display: crate::view_layout::Display, retired: &mut Retired) {
        if display.operator || display.openers.is_empty() {
            retired.dropped += 1;
            return;
        }
        let area = self.shelf.active_area().id.clone();
        match self.shelf.append(&area, display) {
            Ok(()) => retired.shelved += 1,
            Err(_) => retired.dropped += 1,
        }
    }

    /// Takes away the tray of a tab the engine no longer lists: its agents'
    /// Views go on no tray, to stay in their panes' lists, and the
    /// operator's go to `heir`'s tray (the desk in front, or the pending one
    /// while no tab is shown, and when `heir` is the tab that left), so no
    /// page or document leaves with the tab. A View neither has room for is
    /// dropped, and counted.
    pub fn retire_tray(&mut self, tab: &str, heir: &str) -> Retired {
        let mut retired = Retired::default();
        let Some(index) = self.trays.iter().position(|tray| tray.tab == tab) else {
            return retired;
        };
        let tray = self.trays.remove(index);
        let heir = if heir == tab { PENDING_TRAY } else { heir };
        for display in tray.layout.displays().cloned().collect::<Vec<_>>() {
            if !display.operator && !display.openers.is_empty() {
                self.shelve_or_drop(display, &mut retired);
                continue;
            }
            let layout = &mut self.tray_entry(heir).layout;
            let area = layout.active_area().id.clone();
            match layout.append(&area, display) {
                Ok(()) => retired.inherited += 1,
                Err(_) => retired.dropped += 1,
            }
        }
        retired
    }

    /// Opens a closed lane on the column a call brings forward, the other
    /// column closed: a tool pressed, or a View opened onto `tab`'s desk,
    /// into a lane ⌘E closed shows that alone, and only ⌘E's `open: true`
    /// brings back the columns it closed with (B15; operator decision
    /// 2026-10-23). Whether the lane was closed.
    pub fn open_on(&mut self, side: crate::model::LaneCallSide, tab: Option<&str>) -> bool {
        if self.open {
            return false;
        }
        self.open = true;
        match side {
            crate::model::LaneCallSide::View => self.tool = None,
            crate::model::LaneCallSide::Tool => {
                if let Some(tray) = tab.and_then(|tab| self.tray_mut(tab)) {
                    tray.shown = false;
                }
            }
        }
        true
    }

    /// Shows a tool in the tool column (B14), a closed lane opening on it
    /// alone (`open_on`), `tab`'s desk folded. Whether the lane was closed.
    pub fn show_tool(&mut self, tool: Tool, tab: Option<&str>) -> bool {
        let opened = self.open_on(crate::model::LaneCallSide::Tool, tab);
        self.tool = Some(tool);
        opened
    }

    /// Closes the tool column alone (B14); the desk stays.
    pub fn close_tool(&mut self) {
        self.tool = None;
    }

    /// Shows `tab`'s desk (B5, B6), a closed lane opening on it alone
    /// (`open_on`).
    pub fn show_tray(&mut self, tab: &str) {
        self.open_on(crate::model::LaneCallSide::View, Some(tab));
        self.tray_entry(tab).shown = true;
    }

    /// Whether `tab`'s desk is drawn: the lane is open with it shown, and it
    /// has a tab to show.
    pub fn draws_tray(&self, tab: &str) -> bool {
        self.open
            && self
                .tray(tab)
                .is_some_and(|tray| tray.shown && tray.layout.display_count() > 0)
    }

    /// Whether anything is drawn while the lane is open with `tab` in front:
    /// its desk (with a tab to show) or a tool.
    pub fn has_column(&self, tab: Option<&str>) -> bool {
        tab.and_then(|tab| self.tray(tab))
            .is_some_and(|tray| tray.shown && tray.layout.display_count() > 0)
            || self.tool.is_some()
    }

    /// Whether the tool column shows the Explorer: the readers that gate on
    /// the tree (the device Explorer watch) read only then.
    pub fn shows_explorer(&self) -> bool {
        self.open && self.tool == Some(Tool::Explorer)
    }
}

/// Where the Views of a retired tray went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retired {
    /// Agents' Views now on no tray, in their panes' lists.
    pub shelved: usize,
    /// The operator's Views now on the heir's tray.
    pub inherited: usize,
    /// Views no tree had room for.
    pub dropped: usize,
}

/// A stored or sent width inside what the core accepts.
pub fn clamp_width(value: u32) -> u32 {
    value.clamp(1, MAX_WIDTH)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceViews {
    pub workspaces: Vec<WorkspaceView>,
    /// The window's pair view mode (D-45).
    pub pair: PairMode,
}

impl WorkspaceViews {
    pub fn get(&self, device_id: &str, path: &str) -> Option<&WorkspaceView> {
        self.workspaces.iter().find(|view| view.is(device_id, path))
    }

    pub fn get_mut(&mut self, device_id: &str, path: &str) -> Option<&mut WorkspaceView> {
        self.workspaces
            .iter_mut()
            .find(|view| view.is(device_id, path))
    }

    /// The Workspace's entry, made with the defaults on first use. Making one
    /// past the cap forgets the least recently used entry.
    pub fn entry(&mut self, device_id: &str, path: &str) -> &mut WorkspaceView {
        self.entry_at(device_id, path, crate::observed::now_unix_ms())
    }

    /// `entry`, with the moment an entry it makes counts as used. Ranked by
    /// its unset choice stamp alone, a just-made entry was the first one
    /// forgotten, and two checkouts nobody had opened yet forgot each other
    /// at every session publish: a project just added showed the empty start
    /// screen over its live terminal.
    pub fn entry_at(
        &mut self,
        device_id: &str,
        path: &str,
        now_unix_ms: u64,
    ) -> &mut WorkspaceView {
        if let Some(index) = self
            .workspaces
            .iter()
            .position(|view| view.is(device_id, path))
        {
            return &mut self.workspaces[index];
        }
        if self.workspaces.len() >= MAX_WORKSPACES
            && let Some(oldest) = self
                .workspaces
                .iter()
                .enumerate()
                .min_by_key(|(_, view)| view.last_used_unix_ms.max(view.made_unix_ms))
                .map(|(index, _)| index)
        {
            self.workspaces.remove(oldest);
        }
        self.workspaces.push(WorkspaceView {
            made_unix_ms: now_unix_ms,
            ..WorkspaceView::new(device_id, path)
        });
        self.workspaces.last_mut().expect("just pushed")
    }
}

#[derive(Serialize)]
struct StoredWorkspaceViews<'a> {
    schema_version: u32,
    workspaces: &'a [WorkspaceView],
    #[serde(skip_serializing_if = "pair_mode_unset")]
    pair_view: PairMode,
}

fn pair_mode_unset(mode: &PairMode) -> bool {
    *mode == PairMode::default()
}

#[derive(Deserialize)]
struct StoredWorkspaces {
    #[serde(default)]
    workspaces: Vec<StoredTrays>,
    #[serde(default)]
    pair_view: PairMode,
}

/// Schemas 4 and 5: one lane and one View list per checkout. Read only to
/// migrate.
#[derive(Deserialize)]
struct StoredLanes {
    #[serde(default)]
    workspaces: Vec<StoredLane>,
}

/// Schema 3: one lane showing one tool. Read only to migrate.
#[derive(Deserialize)]
struct StoredV3 {
    #[serde(default)]
    workspaces: Vec<StoredOneToolLane>,
}

/// Schema 2: the File Views and Tools columns. Read only to migrate.
#[derive(Deserialize)]
struct StoredV2 {
    #[serde(default)]
    workspaces: Vec<StoredColumns>,
}

/// Only the version, read before the body so each version is parsed as what
/// it is.
#[derive(Deserialize)]
struct StoredVersion {
    schema_version: u32,
}

/// Schema 1 (S6): one strip of View tabs per Workspace. Read only to migrate.
#[derive(Deserialize)]
struct StoredV1 {
    #[serde(default)]
    workspaces: Vec<V1View>,
}

#[derive(Deserialize)]
struct V1View {
    device_id: String,
    path: String,
    /// Read so an older file still parses; the lane is not carried over.
    #[serde(default)]
    #[allow(dead_code)]
    mode: Option<LegacyMode>,
    #[serde(default)]
    tabs: Vec<V1Tab>,
    #[serde(default)]
    active: Option<V1Tab>,
    #[serde(default)]
    last_used_unix_ms: u64,
}

#[derive(Deserialize)]
struct V1Tab {
    path: String,
    kind: DisplayKind,
    #[serde(default)]
    committed: Option<bool>,
    #[serde(default)]
    preview: bool,
}

impl V1View {
    /// The strip becomes the displays of one area, in saved order, with the
    /// active tab as that area's active display. A strip past the display
    /// cap is taken whole and trimmed by the load's repair, which says so.
    fn migrate(self) -> WorkspaceView {
        let mut layout = Layout::default();
        let area = layout.active_area.clone();
        let mut active = None;
        let mut displays = Vec::new();
        for tab in &self.tabs {
            let display = layout.new_display(&tab.path, tab.kind, tab.committed, tab.preview);
            if self.active.as_ref().is_some_and(|saved| {
                saved.path == tab.path && saved.kind == tab.kind && saved.committed == tab.committed
            }) {
                active = Some(display.id.clone());
            }
            displays.push(display);
        }
        if let Some(area) = layout.area_mut(&area) {
            area.displays = displays;
            area.active = active.or_else(|| area.displays.last().map(|display| display.id.clone()));
        }
        let mut view = WorkspaceView {
            last_used_unix_ms: self.last_used_unix_ms,
            ..WorkspaceView::new(&self.device_id, &self.path)
        };
        view.adopt_checkout_documents(layout, false);
        view
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadOutcome {
    /// Read as this version, or migrated from `migrated_from`. `repairs`
    /// says what the load had to change to hold the layout invariants.
    Loaded {
        migrated_from: Option<u32>,
        repairs: Vec<String>,
    },
    Missing,
    /// The file could not be read as this version. `preserved_as` is where
    /// the original now sits; `None` means it could not be moved and is left
    /// where it was, in which case nothing will be written over it either.
    Unreadable {
        reason: String,
        preserved_as: Option<PathBuf>,
    },
}

/// Reads the file. An unreadable, unknown-version or unmigratable file is
/// renamed beside itself (`<name>.unreadable-<ms>`) before the defaults
/// load, so the next save cannot destroy what an operator or another build
/// wrote there.
pub fn load(path: &Path, now_unix_ms: u64) -> (WorkspaceViews, LoadOutcome) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (WorkspaceViews::default(), LoadOutcome::Missing);
        }
        Err(error) => {
            return (
                WorkspaceViews::default(),
                LoadOutcome::Unreadable {
                    reason: format!("the file could not be read: {error}"),
                    preserved_as: None,
                },
            );
        }
    };
    let reason = match serde_json::from_slice::<StoredVersion>(&bytes) {
        Ok(StoredVersion { schema_version: 6 }) => {
            match serde_json::from_slice::<StoredWorkspaces>(&bytes) {
                Ok(stored) => {
                    let views = stored
                        .workspaces
                        .into_iter()
                        .map(StoredTrays::into_view)
                        .collect();
                    let (mut loaded, outcome) = settle(views, None);
                    loaded.pair = PairMode {
                        on: stored.pair_view.on,
                        ratio: stored.pair_view.ratio.map(clamp_pair_ratio),
                    };
                    return (loaded, outcome);
                }
                Err(error) => format!("the file is not valid: {error}"),
            }
        }
        Ok(StoredVersion {
            schema_version: version @ (4 | 5),
        }) => match serde_json::from_slice::<StoredLanes>(&bytes) {
            Ok(stored) => {
                let views = stored
                    .workspaces
                    .into_iter()
                    .map(StoredLane::into_view)
                    .collect();
                return settle(views, Some(version));
            }
            Err(error) => format!("the schema {version} file could not be migrated: {error}"),
        },
        Ok(StoredVersion { schema_version: 3 }) => {
            match serde_json::from_slice::<serde_json::Value>(&bytes).and_then(|mut value| {
                let dropped = drop_retired_cards(&mut value);
                serde_json::from_value::<StoredV3>(value).map(|stored| (stored, dropped))
            }) {
                Ok((stored, dropped)) => {
                    let views = stored
                        .workspaces
                        .into_iter()
                        .map(StoredOneToolLane::into_view)
                        .collect();
                    let (views, mut outcome) = settle(views, Some(3));
                    if dropped > 0
                        && let LoadOutcome::Loaded { repairs, .. } = &mut outcome
                    {
                        repairs.push(format!(
                            "{dropped} pull request cards stored as views were dropped"
                        ));
                    }
                    return (views, outcome);
                }
                Err(error) => format!("the schema 3 file could not be migrated: {error}"),
            }
        }
        Ok(StoredVersion { schema_version: 2 }) => {
            match serde_json::from_slice::<StoredV2>(&bytes) {
                Ok(stored) => {
                    let views = stored
                        .workspaces
                        .into_iter()
                        .map(StoredColumns::into_view)
                        .collect();
                    return settle(views, Some(2));
                }
                Err(error) => format!("the schema 2 file could not be migrated: {error}"),
            }
        }
        Ok(StoredVersion { schema_version: 1 }) => {
            match serde_json::from_slice::<StoredV1>(&bytes) {
                Ok(stored) => {
                    let migrated: Vec<_> =
                        stored.workspaces.into_iter().map(V1View::migrate).collect();
                    return settle(migrated, Some(1));
                }
                Err(error) => format!("the schema 1 file could not be migrated: {error}"),
            }
        }
        Ok(StoredVersion { schema_version }) => {
            format!("schema version {schema_version} is not {SCHEMA_VERSION}")
        }
        Err(error) => format!("the file is not valid: {error}"),
    };
    let preserved = preserved_path(path, now_unix_ms);
    let preserved_as = fs::rename(path, &preserved).ok().map(|()| preserved);
    (
        WorkspaceViews::default(),
        LoadOutcome::Unreadable {
            reason,
            preserved_as,
        },
    )
}

/// Takes out every display a schema 3 build stored as a pull request's card
/// (PRD sidebar-task-lens B51, D-50), a kind no build reads since. The other
/// displays of the same area, and their arrangement, stay; an area left
/// empty is collapsed by `settle`'s repair like any other. Returns how many
/// it took out.
fn drop_retired_cards(value: &mut serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(object) => {
            let mut dropped = 0;
            if let Some(serde_json::Value::Array(displays)) = object.get_mut("displays") {
                let before = displays.len();
                displays.retain(|display| {
                    display.get("kind").and_then(|kind| kind.as_str()) != Some("pull_request")
                });
                dropped += before - displays.len();
            }
            dropped + object.values_mut().map(drop_retired_cards).sum::<usize>()
        }
        serde_json::Value::Array(items) => items.iter_mut().map(drop_retired_cards).sum(),
        _ => 0,
    }
}

/// Brings loaded entries inside the bounds every operation keeps; excess is
/// trimmed here, never on an operator's action.
fn settle(
    mut workspaces: Vec<WorkspaceView>,
    migrated_from: Option<u32>,
) -> (WorkspaceViews, LoadOutcome) {
    let mut repairs = Vec::new();
    for view in &mut workspaces {
        view.document_width = view.document_width.map(clamp_width);
        view.explorer_width = view.explorer_width.map(clamp_width);
        view.panel_width = view.panel_width.map(clamp_width);
        let mut notes = Vec::new();
        if view.trays.len() > MAX_TRAYS {
            notes.push(format!(
                "{} trays were kept past the {MAX_TRAYS} a checkout holds",
                view.trays.len() - MAX_TRAYS
            ));
            view.trays.truncate(MAX_TRAYS);
        }
        let mut tabs = std::collections::HashSet::new();
        let before = view.trays.len();
        view.trays.retain(|tray| tabs.insert(tray.tab.clone()));
        if view.trays.len() != before {
            notes.push(format!(
                "{} trays named a tab twice",
                before - view.trays.len()
            ));
        }
        // No build writes an outside file's tab; one in the file has no grant
        // behind it.
        let external: usize = view
            .layouts_mut()
            .map(|layout| layout.remove_external(|_| false))
            .sum();
        if external > 0 {
            notes.push(format!("{external} outside-file views had no grant"));
        }
        for layout in view.layouts_mut() {
            notes.extend(layout.repair());
            notes.extend(remove_cardless(layout));
        }
        notes.extend(view.repair_shelf());
        notes.extend(view.repair_ids());
        notes.extend(view.repair_agent_groups());
        notes.extend(view.feedback.repair());
        for text in notes {
            repairs.push(format!("{} on {}: {text}", view.path, view.device_id));
        }
    }
    workspaces.truncate(MAX_WORKSPACES);
    (
        WorkspaceViews {
            workspaces,
            pair: PairMode::default(),
        },
        LoadOutcome::Loaded {
            migrated_from,
            repairs,
        },
    )
}

/// A card display that names no card shows nothing and goes.
fn remove_cardless(layout: &mut Layout) -> Option<String> {
    let gone: Vec<String> = layout
        .displays()
        .filter(|display| display.kind == DisplayKind::Card && display.card.is_none())
        .map(|display| display.id.clone())
        .collect();
    for id in &gone {
        layout.remove(id);
    }
    (!gone.is_empty()).then(|| format!("{} card views named no card", gone.len()))
}

impl WorkspaceView {
    /// The shelf is one area and holds no card or outside file: a View on
    /// no desk is an agent's file, diff or page (D-19). A crafted file's
    /// split shelf is flattened, and a card on it goes.
    fn repair_shelf(&mut self) -> Vec<String> {
        let mut notes = Vec::new();
        let cards: Vec<String> = self
            .shelf
            .displays()
            .filter(|display| display.kind == DisplayKind::Card)
            .map(|display| display.id.clone())
            .collect();
        for id in &cards {
            self.shelf.remove(id);
        }
        if !cards.is_empty() {
            notes.push(format!("{} cards on no desk went", cards.len()));
        }
        if self.shelf.area_count() > 1 {
            let displays: Vec<_> = self.shelf.displays().cloned().collect();
            let mut flat = shelf_layout(self.shelf.next_id);
            let area = flat.active_area.clone();
            for display in displays {
                let _ = flat.append(&area, display);
            }
            self.shelf = flat;
            notes.push("the Views on no desk were in more than one area".to_owned());
        }
        notes
    }

    /// Every View, area and divider id names one thing in the checkout: an
    /// id a second tree repeats takes a new one there, and every tree mints
    /// past the highest.
    fn repair_ids(&mut self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut next = self.next_id();
        let mut renamed = 0usize;
        for layout in self.layouts_mut() {
            let mut ids: Vec<String> = layout.areas().iter().map(|area| area.id.clone()).collect();
            ids.extend(layout.displays().map(|display| display.id.clone()));
            for id in ids {
                if seen.insert(id.clone()) {
                    continue;
                }
                let prefix = id.chars().next().unwrap_or('d');
                let fresh = format!("{prefix}{next}");
                next += 1;
                renamed += 1;
                if let Some(area) = layout.area_mut(&id) {
                    area.id = fresh.clone();
                    if layout.active_area == id {
                        layout.active_area = fresh.clone();
                    }
                } else if let Some(display) = layout.display_mut(&id) {
                    display.id = fresh.clone();
                    let area = layout.area_of(&fresh).map(|area| area.id.clone());
                    if let Some(area) = area.and_then(|area| layout.area_mut(&area))
                        && area.active.as_deref() == Some(id.as_str())
                    {
                        area.active = Some(fresh.clone());
                    }
                }
                seen.insert(fresh);
            }
        }
        for layout in self.layouts_mut() {
            layout.next_id = layout.next_id.max(next);
        }
        if renamed == 0 {
            Vec::new()
        } else {
            vec![format!(
                "{renamed} ids repeated across trays and were renumbered"
            )]
        }
    }
}

fn preserved_path(path: &Path, now_unix_ms: u64) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".unreadable-{now_unix_ms}"));
    path.with_file_name(name)
}

/// Writes the file atomically: a sibling temporary file, fsync, rename.
pub fn save(path: &Path, views: &WorkspaceViews) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "Workspace view state path has no parent directory".to_owned())?;
    hide_platform::fs::private::create_dir_all(parent)
        .map_err(|_| "Workspace view state directory could not be prepared".to_owned())?;
    // An outside file's tab lives as long as the desktop host's grant, which
    // ends with the app: it is never written (PRD outside-path-in-hide D-07).
    let kept: Vec<WorkspaceView>;
    let workspaces = if views.workspaces.iter().any(|view| {
        view.views()
            .any(|display| display.kind == crate::view_layout::DisplayKind::External)
    }) {
        kept = views
            .workspaces
            .iter()
            .cloned()
            .map(|mut view| {
                for layout in view.layouts_mut() {
                    layout.remove_external(|_| false);
                }
                view
            })
            .collect();
        &kept[..]
    } else {
        &views.workspaces[..]
    };
    let stored = StoredWorkspaceViews {
        schema_version: SCHEMA_VERSION,
        workspaces,
        pair_view: views.pair,
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|_| "Workspace view state could not be encoded".to_owned())?;
    hide_platform::fs::atomic::write_file_durable(
        path,
        &bytes,
        hide_platform::fs::atomic::DurableAccess::Private,
    )
    .map(|_| ())
    .map_err(|error| format!("Workspace view state could not be saved durably: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_layout::{Edge, Node, SplitAxis};

    /// A new folder, removed when the test drops it.
    fn scratch(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("hide-core-workspace-views-{name}-"))
            .tempdir()
            .unwrap()
    }

    fn unbound(mut views: WorkspaceViews) -> WorkspaceViews {
        for view in &mut views.workspaces {
            for display in view.views_mut() {
                display.tab_id = None;
            }
        }
        views
    }

    /// `views` as a load reads it back: every tree numbering from the
    /// view's one counter, and comments with nothing in them left out.
    fn settled(mut views: WorkspaceViews) -> WorkspaceViews {
        for view in &mut views.workspaces {
            let next = view.next_id();
            for layout in view.layouts_mut() {
                layout.next_id = next;
            }
            if view.feedback.is_empty() {
                view.feedback = Default::default();
            }
        }
        views
    }

    fn current() -> LoadOutcome {
        LoadOutcome::Loaded {
            migrated_from: None,
            repairs: Vec::new(),
        }
    }

    /// The lane a view holds, as the shell reads it: open, the first tray
    /// shown, the tool.
    fn lane(view: &WorkspaceView) -> (bool, bool, Option<Tool>) {
        (
            view.open,
            view.trays.first().is_some_and(|tray| tray.shown),
            view.tool,
        )
    }

    fn operators(layout: &mut Layout, path: &str) -> String {
        let mut display = layout.new_display(path, DisplayKind::File, None, false);
        display.operator = true;
        let id = display.id.clone();
        let area = layout.active_area().id.clone();
        layout.insert(&area, display, 1).unwrap();
        id
    }

    fn agents(layout: &mut Layout, path: &str) -> String {
        let mut display = layout.new_display(path, DisplayKind::File, None, false);
        display.opened_by(
            crate::view_feedback::Opener {
                device_id: "local".into(),
                terminal_id: "t1".into(),
                session: Some("s1".into()),
                resume: None,
                resume_path: None,
                provider: "claude".into(),
                name: "claude".into(),
                profile: None,
            },
            1,
        );
        let id = display.id.clone();
        let area = layout.active_area().id.clone();
        layout.insert(&area, display, 1).unwrap();
        id
    }

    /// D-45: a desk an earlier build saved in its tab's own pair view loads
    /// as a desk, with the window's mode off; nothing turns it back on.
    #[test]
    fn a_tabs_saved_pair_view_loads_as_its_desk_with_the_mode_off() {
        let folder = scratch("tab-pair");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({"schema_version": 6, "workspaces": [
                {"device_id": "local", "path": "/repo",
                 "trays": [{"tab": "w1:t1", "shown": true, "pair": {"pane": "w1:p1", "before": "w1:p2"},
                            "layout": {"root": {"area": {"id": "a1", "displays": []}}, "active_area": "a1", "next_id": 2}}]}
            ]})
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        assert_eq!(outcome, current());
        assert_eq!(loaded.pair, PairMode::default());
        assert!(loaded.workspaces[0].tray("w1:t1").unwrap().shown);
        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(!written.contains("\"pair"), "{written}");
    }

    /// PRD lane-desk-pair-view D-06, D-45, D-47: each Agent tab's tray (its
    /// nested tree and shown flag), the Views on no tray, the tool column,
    /// three widths and the window's pair view mode with its ratio come back
    /// as they were; runtime bindings are never written.
    #[test]
    fn saved_trays_the_pair_mode_and_the_views_on_no_tray_load_back_as_they_were() {
        let folder = scratch("roundtrip");
        let path = folder.path().join("workspace-views.json");
        let mut views = WorkspaceViews::default();
        let entry = views.entry("local", "/repo");
        entry.show_tray("w1:t1");
        entry.show_tool(Tool::Changes, Some("w1:t1"));
        entry.document_width = Some(720);
        entry.explorer_width = Some(300);
        entry.panel_width = Some(420);
        entry.align_ids(&Place::Tray("w1:t1".into()));
        let layout = &mut entry.tray_mut("w1:t1").unwrap().layout;
        for (path, kind, committed, preview) in [
            ("/repo/a.md", DisplayKind::File, None, false),
            ("/repo/b.rs", DisplayKind::Diff, Some(true), false),
            ("/repo/c.rs", DisplayKind::File, None, false),
            ("/repo/d.md", DisplayKind::File, None, true),
        ] {
            let mut display = layout.new_display(path, kind, committed, preview);
            display.operator = true;
            let area = layout.active_area().id.clone();
            layout.insert(&area, display, 1).unwrap();
        }
        let first = layout.active_area().id.clone();
        let b = layout.displays().nth(1).unwrap().id.clone();
        let right = layout.split(&b, &first, Edge::Right, 2).unwrap();
        let c = layout.displays().nth(1).unwrap().id.clone();
        layout.split(&c, &right, Edge::Down, 3).unwrap();
        layout.focus_area(&first, 4).unwrap();
        if let Node::Split(split) = &mut layout.root {
            split.ratio = 0.7;
        }
        entry.tray_entry("w1:t2");
        views.pair = PairMode {
            on: true,
            ratio: Some(620),
        };
        let entry = views.entry("local", "/repo");
        entry.align_ids(&Place::Tray("w1:t2".into()));
        let card = entry.tray_mut("w1:t2").unwrap().layout.new_card_display(
            crate::model::DetailItem::Mission {
                mission: "mission-7".into(),
            },
        );
        let tray = &mut entry.tray_mut("w1:t2").unwrap().layout;
        let area = tray.active_area().id.clone();
        tray.insert(&area, card, 5).unwrap();
        entry.align_ids(&Place::Shelf);
        agents(&mut entry.shelf, "/repo/agent.md");
        for display in entry.views_mut() {
            if display.kind != DisplayKind::Card {
                display.tab_id = Some("file:bound-at-runtime".to_owned());
            }
        }
        save(&path, &views).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\": 6"));
        assert!(!written.contains("bound-at-runtime"));
        let (loaded, outcome) = load(&path, 1);
        assert_eq!(outcome, current());
        assert_eq!(settled(loaded.clone()), settled(unbound(views)));
        let view = &loaded.workspaces[0];
        assert_eq!(lane(view), (true, true, Some(Tool::Changes)));
        assert_eq!(
            (view.document_width, view.explorer_width, view.panel_width),
            (Some(720), Some(300), Some(420))
        );
        let layout = &view.tray("w1:t1").unwrap().layout;
        let Node::Split(root_split) = &layout.root else {
            panic!("a split root")
        };
        assert_eq!(root_split.axis, SplitAxis::Row);
        assert_eq!(root_split.ratio, 0.7);
        assert_eq!(
            loaded.pair,
            PairMode {
                on: true,
                ratio: Some(620)
            },
            "D-45, D-47: one mode and its ratio for the window"
        );
        let paired = view.tray("w1:t2").unwrap();
        assert_eq!(
            paired.layout.displays().next().unwrap().kind,
            DisplayKind::Card
        );
        assert_eq!(view.shelf.display_count(), 1);
        let ids: Vec<&str> = view.views().map(|display| display.id.as_str()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "one id names one View: {ids:?}");
    }

    /// B1, B23: a Workspace seen for the first time has its lane closed,
    /// with no tray, no tool and the default widths.
    #[test]
    fn a_new_workspace_starts_with_the_lane_closed() {
        let view = WorkspaceView::new("local", "/repo");
        assert_eq!(lane(&view), (false, false, None));
        assert_eq!(
            (view.document_width, view.explorer_width, view.panel_width),
            (None, None, None)
        );
        assert!(view.trays.is_empty());
        assert!(!view.has_column(Some("w1:t1")));
        assert!(!view.shows_explorer());
    }

    /// B5, B12, B14, B15 (lane-two-columns), PRD lane-desk-pair-view B9,
    /// B30: a tray counts as a column only with a tab to show, a tool closes
    /// alone, ⌘E's close keeps both columns' shape for its reopen, a tool or
    /// a tray called into a closed lane opens its own column alone, and each
    /// tab's tray is its own.
    #[test]
    fn the_columns_open_close_and_keep_their_shape_through_a_close() {
        let mut view = WorkspaceView::new("local", "/repo");
        view.show_tray("t1");
        assert_eq!(lane(&view), (true, true, None));
        assert!(!view.has_column(Some("t1")), "no tab to show");
        operators(&mut view.tray_mut("t1").unwrap().layout, "/repo/a.md");
        assert!(view.has_column(Some("t1")) && view.draws_tray("t1"));
        assert!(!view.has_column(Some("t2")), "another tab's tray is empty");
        view.show_tool(Tool::Explorer, Some("t1"));
        assert!(view.shows_explorer());
        view.close_tool();
        assert_eq!(lane(&view), (true, true, None));
        view.show_tool(Tool::Project, Some("t1"));
        view.open = false;
        assert_eq!(lane(&view), (false, true, Some(Tool::Project)));
        assert!(!view.shows_explorer() && !view.draws_tray("t1"));
        view.open = true;
        assert_eq!(lane(&view), (true, true, Some(Tool::Project)));
        view.open = false;
        assert!(view.show_tool(Tool::Memory, Some("t1")));
        assert_eq!(
            lane(&view),
            (true, false, Some(Tool::Memory)),
            "a tool opens a closed lane on itself alone"
        );
        assert!(!view.show_tool(Tool::Changes, Some("t1")));
        view.show_tray("t1");
        assert_eq!(lane(&view), (true, true, Some(Tool::Changes)));
        view.open = false;
        view.show_tray("t1");
        assert_eq!(
            lane(&view),
            (true, true, None),
            "a tray opens a closed lane on itself alone"
        );
    }

    /// S7 D-10, B17; lane-desk-pair-view D-18: an S6 file becomes one area
    /// holding its tabs in saved order, with its active tab and its preview,
    /// on the tray the first tab shown takes, the lane closed.
    #[test]
    fn a_schema_1_file_migrates_into_one_area_on_the_tray_to_come() {
        let folder = scratch("migrate");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 1,
                "workspaces": [{
                    "device_id": "local", "path": "/repo", "mode": "together",
                    "explorer": false, "changes": true, "agent_share": 0.4,
                    "last_used_unix_ms": 7,
                    "tabs": [
                        {"path": "/repo/a.md", "kind": "file", "committed": null, "preview": false},
                        {"path": "/repo/b.rs", "kind": "diff", "committed": true, "preview": false},
                        {"path": "/repo/c.md", "kind": "file", "committed": null, "preview": true}
                    ],
                    "active": {"path": "/repo/b.rs", "kind": "diff", "committed": true, "preview": false}
                }]
            })
            .to_string(),
        )
        .unwrap();

        let (loaded, outcome) = load(&path, 1);

        assert!(matches!(
            outcome,
            LoadOutcome::Loaded {
                migrated_from: Some(1),
                ..
            }
        ));
        let view = &loaded.workspaces[0];
        assert_eq!(lane(view), (false, false, None));
        assert_eq!(view.last_used_unix_ms, 7);
        let tray = view.tray(PENDING_TRAY).expect("the tray to come");
        assert_eq!(tray.layout.area_count(), 1);
        let area = tray.layout.active_area();
        let displays: Vec<_> = area
            .displays
            .iter()
            .map(|display| (display.path.as_str(), display.kind, display.preview))
            .collect();
        assert_eq!(
            displays,
            vec![
                ("/repo/a.md", DisplayKind::File, false),
                ("/repo/b.rs", DisplayKind::Diff, false),
                ("/repo/c.md", DisplayKind::File, true),
            ]
        );
        assert_eq!(area.active.as_ref(), Some(&area.displays[1].id));

        save(&path, &loaded).unwrap();
        let (again, outcome) = load(&path, 2);
        assert_eq!(outcome, current());
        assert_eq!(settled(again), settled(loaded));
    }

    /// S6 B20, S7 B17, PRD right-lane B22: a file this build cannot read,
    /// of an unknown version or one whose migration fails is kept byte for
    /// byte beside the path, and the defaults load.
    #[test]
    fn an_unreadable_future_or_unmigratable_file_is_preserved_and_defaults_load() {
        let folder = scratch("unreadable");
        let path = folder.path().join("workspace-views.json");
        for (index, body) in [
            b"{not json".as_slice(),
            br#"{"schema_version":9,"workspaces":[]}"#.as_slice(),
            br#"{"schema_version":1,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":2,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":3,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":5,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":6,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&path, body).unwrap();
            let (loaded, outcome) = load(&path, index as u64);
            assert!(loaded.workspaces.is_empty());
            let LoadOutcome::Unreadable {
                preserved_as: Some(preserved),
                ..
            } = outcome
            else {
                panic!("expected a preserved unreadable file, got {outcome:?}");
            };
            assert!(
                !path.exists(),
                "the original path is free for the next save"
            );
            assert_eq!(fs::read(&preserved).unwrap(), body);
        }
    }

    /// PRD sidebar-task-lens B51: a pull request card an older build stored
    /// as a View is dropped as the file loads, without an error, and the
    /// other Views of its area stay.
    #[test]
    fn a_stored_pull_request_card_is_dropped_and_the_other_views_stay() {
        let folder = scratch("retired-card");
        let path = folder.path().join("workspace-views.json");
        let mut layout = Layout::default();
        let display = layout.new_display("/r/a.rs", DisplayKind::File, None, false);
        layout.insert("a1", display, 1).unwrap();
        let mut value = serde_json::to_value(&layout).unwrap();
        value["root"]["area"]["displays"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(
                {"id": "d90", "path": "", "kind": "pull_request", "pr": 7, "last_focused_unix_ms": 5}
            ));
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [{"device_id": "local", "path": "/r", "tool": "files", "view_layout": value}]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        let LoadOutcome::Loaded { repairs, .. } = outcome else {
            panic!("the file loads, got {outcome:?}");
        };
        assert!(
            repairs
                .iter()
                .any(|note| note.starts_with("1 pull request cards")),
            "{repairs:?}"
        );
        let view = loaded.get("local", "/r").unwrap();
        let shown: Vec<_> = view.views().map(|display| display.path.as_str()).collect();
        assert_eq!(shown, ["/r/a.rs"]);
    }

    #[test]
    fn the_least_recently_used_workspace_is_forgotten_past_the_cap() {
        let mut views = WorkspaceViews::default();
        for index in 0..MAX_WORKSPACES {
            views
                .entry_at("local", &format!("/w{index}"), 1)
                .last_used_unix_ms = index as u64 + 10;
        }
        views.entry_at("local", "/w0", 2_000).last_used_unix_ms = 1_000;
        views.entry_at("local", "/new", 2_000);
        assert_eq!(views.workspaces.len(), MAX_WORKSPACES);
        assert!(views.get("local", "/w1").is_none());
        assert!(views.get("local", "/w0").is_some());
    }

    /// Being made counts as a use: two Workspaces made past the cap for
    /// checkouts nobody has chosen yet both stay, and the two stalest
    /// chosen ones go, rather than each forgetting the other.
    #[test]
    fn a_workspace_made_past_the_cap_is_not_the_next_one_forgotten() {
        let mut views = WorkspaceViews::default();
        for index in 0..MAX_WORKSPACES {
            let stamp = index as u64 + 10;
            views
                .entry_at("local", &format!("/w{index}"), stamp)
                .last_used_unix_ms = stamp;
        }
        views.entry_at("local", "/added", 5_000);
        views.entry_at("local", "/other", 5_001);
        views.entry_at("local", "/added", 5_002);
        assert_eq!(views.workspaces.len(), MAX_WORKSPACES);
        assert!(views.get("local", "/added").is_some());
        assert!(views.get("local", "/other").is_some());
        assert!(views.get("local", "/w0").is_none());
        assert!(views.get("local", "/w1").is_none());
        assert!(views.get("local", "/w2").is_some());
        assert_eq!(
            views.get("local", "/added").unwrap().last_used_unix_ms,
            0,
            "made is not chosen"
        );
    }

    /// One View on file, as an entry's `view_layout` key.
    fn one_view_layout() -> serde_json::Value {
        let mut layout = Layout::default();
        let display = layout.new_display("/repo/a.md", DisplayKind::File, None, false);
        layout.insert("a1", display, 1).unwrap();
        serde_json::to_value(&layout).unwrap()
    }

    /// A schema 2 entry, whose View list was its `layout`.
    fn stored_entry(path: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut entry =
            serde_json::json!({"device_id": "local", "path": path, "layout": one_view_layout()});
        entry
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        entry
    }

    /// An Agent layout of two tabs, the second in front: the tab last seen.
    fn agent_layout() -> serde_json::Value {
        serde_json::json!({
            "root": {"area": {"id": "a1", "active": "w1:t2", "displays": [{"id": "w1:t1"}, {"id": "w1:t2"}]}},
            "active_area": "a1", "next_id": 2,
        })
    }

    /// PRD lane-desk-pair-view D-18, B34, B35: a schema 5 checkout's
    /// documents move once onto the tray of the Agent tab last seen in that
    /// checkout, shown as its documents were; a pinned lane loads unpinned,
    /// open as it was; comments survive; the next save writes schema 6 with
    /// none of the checkout-wide keys.
    #[test]
    fn a_schema_5_checkout_moves_its_documents_to_the_tab_last_seen_once() {
        let folder = scratch("tray-move");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 5,
                "workspaces": [
                    {"device_id": "local", "path": "/docs", "open": true, "documents": true,
                     "tool": "changes", "pinned": true, "detail_width": 380, "document_width": 640,
                     "agent_layout": agent_layout(), "view_layout": one_view_layout(),
                     "feedback": {"next_id": 7}},
                    {"device_id": "local", "path": "/unseen", "documents": false,
                     "view_layout": one_view_layout()},
                    {"device_id": "local", "path": "/empty", "open": true, "pinned": true},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        assert!(matches!(
            outcome,
            LoadOutcome::Loaded { migrated_from: Some(5), ref repairs } if repairs.is_empty()
        ));
        let docs = loaded.get("local", "/docs").unwrap();
        assert_eq!(lane(docs), (true, true, Some(Tool::Changes)));
        assert_eq!(docs.document_width, Some(640));
        let tray = docs.tray("w1:t2").expect("the tab last seen holds them");
        assert_eq!(tray.layout.display_count(), 1);
        assert_eq!(docs.feedback.next_id, 7, "the comments survive");
        let unseen = loaded.get("local", "/unseen").unwrap();
        assert_eq!(unseen.tray(PENDING_TRAY).unwrap().layout.display_count(), 1);
        assert!(!unseen.tray(PENDING_TRAY).unwrap().shown);
        let empty = loaded.get("local", "/empty").unwrap();
        assert!(empty.trays.is_empty() && empty.open);

        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\": 6"));
        for retired in [
            "\"pinned\"",
            "\"documents\"",
            "detail_width",
            "\"view_layout\"",
        ] {
            assert!(!written.contains(retired), "{retired} is not written again");
        }
        let (again, outcome) = load(&path, 2);
        assert_eq!(outcome, current(), "it moves once");
        assert_eq!(settled(again), settled(loaded));
    }

    /// PRD lane-two-columns D-10, B23: a schema 3 lane, whatever tool, tree,
    /// pin and widths it stored, starts closed at the default widths; its
    /// View list and comments stay, and the next save writes none of the old
    /// keys, the PRs tool's scope (`work`) among them.
    #[test]
    fn a_schema_3_lane_starts_closed_and_keeps_its_views() {
        let folder = scratch("one-tool-lane");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [
                    {"device_id": "local", "path": "/old", "tool": "agent_sessions",
                     "pinned": true, "lane_width": 500, "tree": true, "tree_width": 260,
                     "view_layout": one_view_layout(), "feedback": {"next_id": 7},
                     "view_bookmarks": {"w:t1": {"a1": "d1"}}, "view_owner": "w:t1"},
                    {"device_id": "local", "path": "/new", "tool": "work", "last_tool": "work",
                     "work": {"lens": "sessions", "only_checkout": true}},
                    {"device_id": "local", "path": "/files", "tool": "files", "last_tool": "memory",
                     "panel_width": 440},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        assert!(matches!(
            outcome,
            LoadOutcome::Loaded { migrated_from: Some(3), ref repairs } if repairs.is_empty()
        ));
        for view in &loaded.workspaces {
            assert_eq!(lane(view), (false, false, None), "{}", view.path);
            assert_eq!(
                (view.document_width, view.explorer_width, view.panel_width),
                (None, None, None),
                "{}",
                view.path
            );
        }
        let old = loaded.get("local", "/old").unwrap();
        assert_eq!(old.view_count(), 1);
        assert_eq!(old.feedback.next_id, 7, "the comments survive");

        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\": 6"));
        for retired in [
            "last_tool",
            "\"tree\"",
            "lane_width",
            "tree_width",
            "\"work\"",
            "\"prs\"",
            "agent_sessions",
            "view_bookmarks",
            "view_owner",
        ] {
            assert!(!written.contains(retired), "{retired} is not written again");
        }
        let (again, outcome) = load(&path, 2);
        assert_eq!(outcome, current());
        assert_eq!(settled(again), settled(loaded));
    }

    /// PRD lane-two-columns D-20, D-10: a schema 4 lane loads as it was, a
    /// PRs tool column reading as none and its PRs scope dropped, a Memory
    /// one as the Memory tool it is again (operator decision 2026-10-23).
    #[test]
    fn a_schema_4_lane_loads_whole_and_a_retired_tool_reads_as_no_tool_column() {
        let folder = scratch("two-column-lane");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 4,
                "workspaces": [
                    {"device_id": "local", "path": "/memory", "open": true, "documents": true,
                     "tool": "memory", "pinned": true, "panel_width": 420,
                     "view_layout": one_view_layout()},
                    {"device_id": "local", "path": "/prs", "open": true, "tool": "prs",
                     "prs": {"only_checkout": true}},
                    {"device_id": "local", "path": "/explorer", "open": true, "tool": "explorer",
                     "explorer_width": 250},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        assert!(matches!(
            outcome,
            LoadOutcome::Loaded { migrated_from: Some(4), ref repairs } if repairs.is_empty()
        ));
        let memory = loaded.get("local", "/memory").unwrap();
        assert_eq!(lane(memory), (true, true, Some(Tool::Memory)));
        assert_eq!(memory.panel_width, Some(420));
        assert_eq!(memory.view_count(), 1);
        assert_eq!(
            lane(loaded.get("local", "/prs").unwrap()),
            (true, false, None)
        );
        let explorer = loaded.get("local", "/explorer").unwrap();
        assert_eq!(lane(explorer), (true, false, Some(Tool::Explorer)));
        assert_eq!(explorer.explorer_width, Some(250));

        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("\"prs\""),
            "the PRs scope is not written again"
        );
        assert!(written.contains("\"tool\": \"memory\""));
        assert_eq!(load(&path, 2).1, current());
    }

    /// PRD mission-screen D-02, B18: a lane saved open on the retired Mission
    /// tool loads with no tool column, its documents and the rest of the
    /// lane as they were, and the next save writes no `mission` tool.
    #[test]
    fn a_lane_saved_on_the_mission_tool_loads_with_no_tool_column() {
        let folder = scratch("mission-tool-lane");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 5,
                "workspaces": [
                    {"device_id": "local", "path": "/docs", "open": true, "documents": true,
                     "tool": "mission", "pinned": true, "view_layout": one_view_layout()},
                    {"device_id": "local", "path": "/alone", "open": true, "tool": "mission"},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, _) = load(&path, 1);
        let docs = loaded.get("local", "/docs").unwrap();
        assert_eq!(lane(docs), (true, true, None));
        assert_eq!(docs.view_count(), 1);
        assert_eq!(
            lane(loaded.get("local", "/alone").unwrap()),
            (true, false, None)
        );

        save(&path, &loaded).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("\"mission\""));
    }

    /// PRD lane-two-columns D-10: every shape schema 2 stored loads its View
    /// list and Agent layout with the lane closed, and the next save writes
    /// none of the columns' keys.
    #[test]
    fn a_schema_2_workspace_loads_its_views_with_the_lane_closed() {
        let folder = scratch("columns");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 2,
                "workspaces": [
                    stored_entry("/views", serde_json::json!({"views": true, "tools": true, "tool": "changes", "views_width": 700, "tools_width": 300})),
                    stored_entry("/explorer", serde_json::json!({"views": false, "tools": true, "tool": "explorer"})),
                    stored_entry("/closed", serde_json::json!({"views": false, "tools": false, "tool": "memory", "view_visibility": {"w:t1": true}, "view_owner": "w:t1"})),
                    stored_entry("/panel", serde_json::json!({"panel": "floating", "pinned": true, "views_width": 900})),
                    stored_entry("/layout", serde_json::json!({"mode": "agents", "explorer": false, "changes": false})),
                ]
            })
            .to_string(),
        )
        .unwrap();

        let (loaded, outcome) = load(&path, 1);

        assert_eq!(
            outcome,
            LoadOutcome::Loaded {
                migrated_from: Some(2),
                repairs: Vec::new(),
            }
        );
        assert_eq!(loaded.workspaces.len(), 5);
        for view in &loaded.workspaces {
            assert_eq!(lane(view), (false, false, None), "{}", view.path);
            assert_eq!(view.view_count(), 1, "views are kept");
        }

        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        for retired in [
            "\"views\"",
            "\"tools\"",
            "views_width",
            "tools_width",
            "view_visibility",
            "\"panel\"",
            "\"mode\"",
            "\"explorer\"",
        ] {
            assert!(!written.contains(retired), "{retired} is not written again");
        }
        let (again, outcome) = load(&path, 2);
        assert_eq!(outcome, current());
        assert_eq!(settled(again), settled(loaded));
    }

    /// A saved file whose trays break the model loads repaired, each fix
    /// said: a tab named twice keeps its first tray, a card on no tray goes
    /// (a card is the operator's, opened on a tray), and an id two trees
    /// both use is minted again so one id names one View.
    #[test]
    fn a_saved_tray_past_the_model_is_repaired_on_load() {
        let folder = scratch("repair");
        let path = folder.path().join("workspace-views.json");
        let mut views = WorkspaceViews::default();
        let entry = views.entry("local", "/repo");
        operators(&mut entry.tray_entry("t1").layout, "/repo/a.md");
        operators(&mut entry.tray_entry("t2").layout, "/repo/b.md");
        let card = entry
            .shelf
            .new_card_display(crate::model::DetailItem::Cost {
                workspace_id: "w".into(),
            });
        let area = entry.shelf.active_area().id.clone();
        entry.shelf.insert(&area, card, 1).unwrap();
        let mut value = serde_json::to_value(&views.workspaces[0]).unwrap();
        let first = value["trays"][0].clone();
        value["trays"].as_array_mut().unwrap().push(first);
        fs::write(
            &path,
            serde_json::json!({"schema_version": 6, "workspaces": [value]}).to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        let LoadOutcome::Loaded { repairs, .. } = outcome else {
            panic!("loads, got {outcome:?}");
        };
        assert!(
            repairs
                .iter()
                .any(|note| note.contains("named a tab twice")),
            "{repairs:?}"
        );
        let view = loaded.get("local", "/repo").unwrap();
        assert_eq!(view.trays.len(), 2);
        assert_eq!(view.shelf.display_count(), 0, "a card on no tray goes");
        let ids: Vec<&str> = view.views().map(|display| display.id.as_str()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
    }

    /// PRD lane-desk-pair-view D-19, B11: a tab that left takes its tray;
    /// its agent's Views stay on no tray, in their pane's list, and the
    /// operator's go to the heir's tray rather than with it.
    #[test]
    fn a_retired_tray_keeps_its_agents_views_on_no_tray() {
        let mut view = WorkspaceView::new("local", "/repo");
        let layout = &mut view.tray_entry("t1").layout;
        operators(layout, "/repo/mine.md");
        agents(layout, "/repo/agent.md");
        assert_eq!(
            view.retire_tray("t1", "t2"),
            Retired {
                shelved: 1,
                inherited: 1,
                dropped: 0
            }
        );
        assert!(view.tray("t1").is_none());
        let paths = |layout: &Layout| -> Vec<String> {
            layout.displays().map(|d| d.path.clone()).collect()
        };
        assert_eq!(paths(&view.shelf), ["/repo/agent.md"]);
        assert_eq!(paths(&view.tray("t2").unwrap().layout), ["/repo/mine.md"]);
    }

    /// D-18: the tray a migration could not name goes to the first tab
    /// shown, joining that tab's own tray when it already has one.
    #[test]
    fn the_pending_tray_goes_to_the_first_tab_shown() {
        let mut view = WorkspaceView::new("local", "/repo");
        let mut layout = Layout::default();
        operators(&mut layout, "/repo/a.md");
        view.adopt_checkout_documents(layout, true);
        assert!(view.tray(PENDING_TRAY).is_some());
        assert!(view.adopt_pending_tray("t1").is_some());
        assert!(view.tray(PENDING_TRAY).is_none());
        let tray = view.tray("t1").unwrap();
        assert!(tray.shown);
        assert_eq!(tray.layout.display_count(), 1);
        assert!(
            view.adopt_pending_tray("t2").is_none(),
            "nothing is pending any more"
        );
    }

    #[test]
    fn a_stored_width_outside_the_bounds_is_clamped() {
        assert_eq!(clamp_width(0), 1);
        assert_eq!(clamp_width(u32::MAX), MAX_WIDTH);
        assert_eq!(clamp_width(640), 640);
    }
}
