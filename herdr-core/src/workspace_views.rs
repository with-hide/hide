//! Each Workspace's presentation in a shell that draws Agent and View areas
//! side by side (PRD S6 D-10, B8, B19, B20; S7 D-09, D-10, B14, B17; PRD
//! right-lane D-01, D-03, D-05, D-11).
//!
//! A Workspace is one checkout on one device, keyed by the device id and the
//! checkout path, because a Herdr workspace id is not stable across a Herdr
//! restart and one checkout can hold tabs from several Herdr workspaces. The
//! state is Hide's own presentation - the right lane's tool, its pin, the
//! file tree column, the lane's and the tree's widths, and the View area
//! tree with its displays ([`crate::view_layout`]) - and never a terminal
//! layout: Herdr keeps panes, splits and zoom. Whether an unpinned lane is
//! floating over the agents right now, and an expanded lane, are the
//! shell's own moment and are never stored (D-11).
//!
//! It lives in its own versioned file, apart from `core-state.json`, so a
//! shell that does not know it never reads it and an older build's settings
//! are never rewritten by it. A schema 1 file (S6, one strip of View tabs per
//! Workspace) and a schema 2 file (the File Views and Tools columns, and the
//! side panel and layouts before them) migrate on load and are written as
//! schema 3 by the next save. A file this build cannot read, of another
//! version or one whose migration fails, is moved aside, never overwritten,
//! and the defaults load.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::view_layout::{DisplayKind, Layout};

pub const SCHEMA_VERSION: u32 = 3;

/// The Workspaces remembered at once. The least recently used is forgotten
/// first, so the file stays bounded however many checkouts come and go.
pub const MAX_WORKSPACES: usize = 256;

/// The widest a stored lane or tree width may be, in CSS pixels. The shell
/// keeps each above its own minimum and Agent Views above its own as it
/// draws; the core only keeps a crafted value from being absurd.
pub const MAX_WIDTH: u32 = 8192;

/// The one tool the right lane shows (PRD right-lane D-01,
/// sidebar-task-lens D-17): the Project's Memory and its pull requests, then
/// the checkout's files (the View areas, with the file tree as a column of
/// its own) and History, in the rail's order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// The View areas.
    Files,
    /// History, the checkout's changes.
    Changes,
    /// The Project's pull requests. A lane left on the Work tool (`work`, or
    /// `agent_sessions` before it) opens here once (PRD sidebar-task-lens
    /// D-34), and the file keeps writing `work` ([`stored_tool`]): a build
    /// before PRs refuses the whole file over a tool it does not know.
    #[serde(alias = "work", alias = "agent_sessions")]
    Prs,
    /// The Project's Memory.
    Memory,
}

impl Tool {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "files" => Some(Self::Files),
            "changes" => Some(Self::Changes),
            "prs" => Some(Self::Prs),
            "memory" => Some(Self::Memory),
            _ => None,
        }
    }

    /// The name `workspace-views.json` keeps the tool under, one every
    /// build since the Work tool reads.
    fn stored_name(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Changes => "changes",
            Self::Prs => "work",
            Self::Memory => "memory",
        }
    }
}

fn stored_tool<S: serde::Serializer>(
    tool: &Option<Tool>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    tool.map(Tool::stored_name).serialize(serializer)
}

/// A Workspace's PRs tool scope: whether its chip is `<checkout> only`,
/// kept across a restart (PRD sidebar-task-lens B26). The file keeps it
/// under the Work tool's `work` key, beside the basis older builds read.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrsView {
    #[serde(default)]
    pub only_checkout: bool,
}

impl PrsView {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The Tools column's tool as schema 2 stored it, read only to migrate.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LegacyTool {
    Explorer,
    Changes,
    AgentSessions,
    Memory,
}

/// The side panel's state an entry stored before the columns, read only to
/// tell whether its views showed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum LegacyPanel {
    Closed,
    Open,
    Expanded,
}

/// The three layouts a Workspace stored before the side panel, read only to
/// tell whether its views showed.
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
    /// The tool the right lane shows, or `None` while the lane is closed.
    /// An unpinned lane the operator hid by going back to a terminal keeps
    /// its tool: hiding it is the shell's moment, not a close.
    #[serde(serialize_with = "stored_tool")]
    pub tool: Option<Tool>,
    /// The tool the lane last showed, kept while it is closed so the next
    /// open comes back on it; `None` for a lane that never showed one. Set
    /// only through `set_tool`, so it follows `tool` and a close leaves it.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "stored_tool"
    )]
    pub last_tool: Option<Tool>,
    /// The PRs tool's scope; left out of the file while default, so a
    /// Workspace that never used it reads as before.
    #[serde(rename = "work", skip_serializing_if = "PrsView::is_default")]
    pub prs: PrsView,
    /// Whether the lane is pinned: docked beside Agent Views, taking its
    /// width from them, rather than floating over them (D-03).
    pub pinned: bool,
    /// Whether the files tool shows the file tree column (D-05). It stays
    /// on until the operator turns it off.
    pub tree: bool,
    /// The files tool's document width in CSS pixels once the operator
    /// resized it; until then the shell's default. The same width floats and
    /// docks (B6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane_width: Option<u32>,
    /// The width of every other tool and of the 보기 slot, the same way, so
    /// a wide document does not widen Memory or PRs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panel_width: Option<u32>,
    /// The tree column's width, the same way.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_width: Option<u32>,
    pub last_used_unix_ms: u64,
    /// The checkout's one View list, split and selection, whichever Agent tab
    /// is in front. Stored as `view_layout`: the keys earlier builds kept per
    /// tab (a schema 3 file's `layout`, `view_owner`, `view_presentations`) are never read.
    #[serde(rename = "view_layout")]
    pub layout: Layout,
    pub agent_layout: crate::agent_layout::Layout,
    /// The operator's comments on this checkout's Views (PRD
    /// view-opener-feedback D-13). Left out of the file while empty.
    #[serde(skip_serializing_if = "crate::view_feedback::Feedback::is_empty")]
    pub feedback: crate::view_feedback::Feedback,
}

/// A stored schema 3 entry.
#[derive(Deserialize)]
struct StoredLane {
    device_id: String,
    path: String,
    #[serde(default)]
    tool: Option<Tool>,
    #[serde(default)]
    last_tool: Option<Tool>,
    /// The Work tool's `lens` beside the scope is ignored (D-34).
    #[serde(default)]
    work: PrsView,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    tree: bool,
    #[serde(default)]
    lane_width: Option<u32>,
    #[serde(default)]
    panel_width: Option<u32>,
    #[serde(default)]
    tree_width: Option<u32>,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default, rename = "view_layout")]
    layout: Layout,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
    #[serde(default)]
    feedback: crate::view_feedback::Feedback,
}

impl StoredLane {
    fn into_view(self) -> WorkspaceView {
        WorkspaceView {
            device_id: self.device_id,
            path: self.path,
            tool: self.tool,
            // A file written before the last tool was kept has only the
            // tool an open lane shows.
            last_tool: self.last_tool.or(self.tool),
            prs: self.work,
            pinned: self.pinned,
            tree: self.tree,
            lane_width: self.lane_width,
            panel_width: self.panel_width,
            tree_width: self.tree_width,
            last_used_unix_ms: self.last_used_unix_ms,
            layout: self.layout,
            agent_layout: self.agent_layout,
            feedback: self.feedback,
        }
    }
}

/// An entry as any build of schema 2 wrote it: the columns' fields, or the
/// side panel or the layout and boundary that came before them, with the
/// one tool and its column or the two tool flags that came before those.
/// Keys no longer read (`pinned` of the side panel, its shares, a tab's
/// File Views visibility) are dropped by not being named here.
#[derive(Deserialize)]
struct StoredColumns {
    device_id: String,
    path: String,
    #[serde(default)]
    views: Option<bool>,
    #[serde(default)]
    panel: Option<LegacyPanel>,
    #[serde(default)]
    tool: Option<LegacyTool>,
    #[serde(default)]
    tools: Option<bool>,
    #[serde(default = "default_explorer")]
    explorer: bool,
    #[serde(default)]
    changes: bool,
    #[serde(default)]
    views_width: Option<u32>,
    #[serde(default)]
    last_used_unix_ms: u64,
    #[serde(default)]
    layout: Layout,
    #[serde(default)]
    agent_layout: crate::agent_layout::Layout,
    #[serde(default)]
    mode: Option<LegacyMode>,
    #[serde(default)]
    views_over_agents: bool,
}

impl StoredColumns {
    /// The columns become the lane once (PRD right-lane D-11, B22): File
    /// Views on gives the files tool, else Tools on gives its tool, the
    /// Explorer becoming the files tool with its tree; neither gives a
    /// closed lane. The pin starts off, and File Views' width is the lane's.
    fn into_view(self) -> WorkspaceView {
        let (views, columns) = match (self.views, self.panel) {
            (Some(views), _) => (views, true),
            (None, Some(panel)) => (panel != LegacyPanel::Closed, false),
            (None, None) => (legacy_views(self.mode, self.views_over_agents), false),
        };
        let (tool, tools) = match (self.tool, self.tools) {
            (Some(tool), Some(tools)) => (tool, tools),
            _ => legacy_tool(self.explorer, self.changes),
        };
        let (tool, tree) = lane_of(views, tools.then_some(tool));
        WorkspaceView {
            device_id: self.device_id,
            path: self.path,
            tool,
            last_tool: tool,
            prs: PrsView::default(),
            pinned: false,
            tree,
            // Widths stored before the columns were shares of a panel that
            // floated; the lane starts at its default.
            lane_width: if columns { self.views_width } else { None },
            panel_width: None,
            tree_width: None,
            last_used_unix_ms: self.last_used_unix_ms,
            layout: self.layout,
            agent_layout: self.agent_layout,
            feedback: Default::default(),
        }
    }
}

/// The lane an entry of the columns restarts with: the files tool where
/// File Views was on, else the shown tool, the Explorer as the files tool
/// with its tree; and whether the tree is on, which it is wherever the
/// Explorer showed.
fn lane_of(views: bool, shown_tool: Option<LegacyTool>) -> (Option<Tool>, bool) {
    let tree = shown_tool == Some(LegacyTool::Explorer);
    let tool = if views {
        Some(Tool::Files)
    } else {
        shown_tool.map(|tool| match tool {
            LegacyTool::Explorer => Tool::Files,
            LegacyTool::Changes => Tool::Changes,
            LegacyTool::AgentSessions => Tool::Prs,
            LegacyTool::Memory => Tool::Memory,
        })
    };
    (tool, tree)
}

/// Whether an entry stored before the side panel showed its views: Agents
/// only did not, unless its View areas were drawn over the agents; Agents
/// and Views and Views only did.
fn legacy_views(mode: Option<LegacyMode>, over: bool) -> bool {
    match mode {
        None | Some(LegacyMode::Agents) => over,
        Some(LegacyMode::Together | LegacyMode::Views) => true,
    }
}

/// The tool an entry stored with two independent flags showed: the
/// Explorer when it showed, since it won the panel while both did, else
/// History when that showed; with neither, nothing showed.
fn legacy_tool(explorer: bool, changes: bool) -> (LegacyTool, bool) {
    match (explorer, changes) {
        (false, true) => (LegacyTool::Changes, true),
        (explorer, _) => (LegacyTool::Explorer, explorer),
    }
}

fn default_explorer() -> bool {
    true
}

impl WorkspaceView {
    /// A Workspace seen for the first time shows Agent Views alone: the
    /// lane closed, unpinned, without its tree (B1).
    pub fn new(device_id: &str, path: &str) -> Self {
        Self {
            device_id: device_id.to_owned(),
            path: path.to_owned(),
            tool: None,
            last_tool: None,
            prs: PrsView::default(),
            pinned: false,
            tree: false,
            lane_width: None,
            panel_width: None,
            tree_width: None,
            last_used_unix_ms: 0,
            layout: Layout::default(),
            agent_layout: crate::agent_layout::Layout::default(),
            feedback: Default::default(),
        }
    }

    pub fn is(&self, device_id: &str, path: &str) -> bool {
        self.device_id == device_id && self.path == path
    }

    /// Shows a tool or closes the lane. A tool shown is remembered, a close
    /// is not a forgetting: the next open returns to `last_tool`.
    pub fn set_tool(&mut self, tool: Option<Tool>) {
        self.tool = tool;
        if tool.is_some() {
            self.last_tool = tool;
        }
    }

    /// Whether the lane holds the View areas: it is open on the files tool.
    pub fn shows_files(&self) -> bool {
        self.tool == Some(Tool::Files)
    }

    /// Whether the file tree column is drawn: the files tool with the tree
    /// on, or with no view to show beside it (B12).
    pub fn shows_tree(&self) -> bool {
        self.shows_files() && (self.tree || self.layout.displays().next().is_none())
    }
}

/// A stored or sent width inside what the core accepts.
pub fn clamp_width(value: u32) -> u32 {
    value.clamp(1, MAX_WIDTH)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceViews {
    pub workspaces: Vec<WorkspaceView>,
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
                .min_by_key(|(_, view)| view.last_used_unix_ms)
                .map(|(index, _)| index)
        {
            self.workspaces.remove(oldest);
        }
        self.workspaces.push(WorkspaceView::new(device_id, path));
        self.workspaces.last_mut().expect("just pushed")
    }
}

#[derive(Serialize)]
struct StoredWorkspaceViews<'a> {
    schema_version: u32,
    workspaces: &'a [WorkspaceView],
}

#[derive(Deserialize)]
struct StoredV3 {
    #[serde(default)]
    workspaces: Vec<StoredLane>,
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
    #[serde(default)]
    mode: Option<LegacyMode>,
    #[serde(default = "default_explorer")]
    explorer: bool,
    #[serde(default)]
    changes: bool,
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
        let (tool, tools) = legacy_tool(self.explorer, self.changes);
        let (tool, tree) = lane_of(legacy_views(self.mode, false), tools.then_some(tool));
        WorkspaceView {
            tool,
            last_tool: tool,
            tree,
            last_used_unix_ms: self.last_used_unix_ms,
            layout,
            ..WorkspaceView::new(&self.device_id, &self.path)
        }
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
        Ok(StoredVersion { schema_version: 3 }) => {
            match serde_json::from_slice::<serde_json::Value>(&bytes).and_then(|mut value| {
                let dropped = drop_retired_cards(&mut value);
                serde_json::from_value::<StoredV3>(value).map(|stored| (stored, dropped))
            }) {
                Ok((stored, dropped)) => {
                    let views = stored
                        .workspaces
                        .into_iter()
                        .map(StoredLane::into_view)
                        .collect();
                    let (views, mut outcome) = settle(views, None);
                    if dropped > 0
                        && let LoadOutcome::Loaded { repairs, .. } = &mut outcome
                    {
                        repairs.push(format!(
                            "{dropped} pull request cards stored as views were dropped; a card opens in the lane's 보기 slot"
                        ));
                    }
                    return (views, outcome);
                }
                Err(error) => format!("the file is not valid: {error}"),
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

/// Takes out every display an older build stored as a pull request's card
/// (PRD sidebar-task-lens B51, D-50): a card opens in the lane's 보기 slot
/// now and is no kind of View. The other displays of the same area, and
/// their arrangement, stay; an area left empty is collapsed by `settle`'s
/// repair like any other. Returns how many it took out.
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
        view.lane_width = view.lane_width.map(clamp_width);
        view.panel_width = view.panel_width.map(clamp_width);
        view.tree_width = view.tree_width.map(clamp_width);
        for note in view
            .layout
            .repair()
            .into_iter()
            .chain(view.agent_layout.repair())
        {
            repairs.push(format!("{} on {}: {note}", view.path, view.device_id));
        }
        for note in view.feedback.repair() {
            repairs.push(format!("{} on {}: {note}", view.path, view.device_id));
        }
    }
    workspaces.truncate(MAX_WORKSPACES);
    (
        WorkspaceViews { workspaces },
        LoadOutcome::Loaded {
            migrated_from,
            repairs,
        },
    )
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
    let stored = StoredWorkspaceViews {
        schema_version: SCHEMA_VERSION,
        workspaces: &views.workspaces,
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|_| "Workspace view state could not be encoded".to_owned())?;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".next");
    let temporary = path.with_file_name(name);
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)
        .map_err(|_| "Workspace view state temporary file could not be opened".to_owned())?;
    output
        .write_all(&bytes)
        .and_then(|_| output.sync_all())
        .map_err(|_| "Workspace view state temporary file could not be written".to_owned())?;
    fs::rename(&temporary, path)
        .map_err(|_| "Workspace view state file could not be replaced".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_layout::{Edge, Node, SplitAxis};

    /// A new folder, removed when the test drops it.
    fn scratch(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("herdr-core-workspace-views-{name}-"))
            .tempdir()
            .unwrap()
    }

    fn unbound(mut views: WorkspaceViews) -> WorkspaceViews {
        for view in &mut views.workspaces {
            for display in view.layout.displays_mut() {
                display.tab_id = None;
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

    /// S7 B14, PRD right-lane B22: the lane (its tool, pin, tree and both
    /// widths) and a nested tree come back with its axes, ratios, order,
    /// previews, active displays and the area in use; runtime bindings are
    /// never written.
    #[test]
    fn a_saved_lane_and_nested_layout_load_back_as_they_were() {
        let folder = scratch("roundtrip");
        let root = folder.path().to_path_buf();
        let path = root.join("workspace-views.json");
        let mut views = WorkspaceViews::default();
        let entry = views.entry("local", "/repo");
        entry.set_tool(Some(Tool::Changes));
        entry.pinned = true;
        entry.tree = true;
        entry.lane_width = Some(720);
        entry.tree_width = Some(300);
        let layout = &mut entry.layout;
        for (path, kind, committed, preview) in [
            ("/repo/a.md", DisplayKind::File, None, false),
            ("/repo/b.rs", DisplayKind::Diff, Some(true), false),
            ("/repo/c.rs", DisplayKind::File, None, false),
            ("/repo/d.md", DisplayKind::File, None, true),
        ] {
            let display = layout.new_display(path, kind, committed, preview);
            layout.insert("a1", display, 1).unwrap();
        }
        let b = layout.displays().nth(1).unwrap().id.clone();
        let right = layout.split(&b, "a1", Edge::Right, 2).unwrap();
        let c = layout.displays().nth(1).unwrap().id.clone();
        layout.split(&c, &right, Edge::Down, 3).unwrap();
        layout.focus_area("a1", 4).unwrap();
        if let Node::Split(split) = &mut layout.root {
            split.ratio = 0.7;
        }
        for display in layout.displays_mut() {
            display.tab_id = Some("file:bound-at-runtime".to_owned());
        }
        save(&path, &views).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\": 3"));
        assert!(!written.contains("bound-at-runtime"));
        let (loaded, outcome) = load(&path, 1);
        assert_eq!(outcome, current());
        assert_eq!(loaded, unbound(views));
        let view = &loaded.workspaces[0];
        assert_eq!(
            (
                view.tool,
                view.pinned,
                view.tree,
                view.lane_width,
                view.tree_width
            ),
            (Some(Tool::Changes), true, true, Some(720), Some(300))
        );
        let layout = &view.layout;
        let Node::Split(root_split) = &layout.root else {
            panic!("a split root")
        };
        assert_eq!(root_split.axis, SplitAxis::Row);
        assert_eq!(root_split.ratio, 0.7);
        assert_eq!(layout.active_area, "a1");
        let kept: Vec<_> = layout
            .area("a1")
            .unwrap()
            .displays
            .iter()
            .map(|display| (display.path.as_str(), display.preview))
            .collect();
        assert_eq!(kept, vec![("/repo/a.md", false), ("/repo/d.md", true)]);
    }

    /// B1: a Workspace seen for the first time has its lane closed,
    /// unpinned and without its tree.
    #[test]
    fn a_new_workspace_starts_with_the_lane_closed() {
        let view = WorkspaceView::new("local", "/repo");
        assert_eq!(
            (
                view.tool,
                view.pinned,
                view.tree,
                view.lane_width,
                view.tree_width
            ),
            (None, false, false, None, None)
        );
        assert!(!view.shows_files());
        assert!(!view.shows_tree());
    }

    /// B12: the files tool with no view to show draws the tree alone,
    /// whether or not the tree is on.
    #[test]
    fn the_files_tool_without_a_view_shows_the_tree() {
        let mut view = WorkspaceView::new("local", "/repo");
        view.set_tool(Some(Tool::Files));
        assert!(view.shows_tree());
        let display = view
            .layout
            .new_display("/repo/a.md", DisplayKind::File, None, false);
        view.layout.insert("a1", display, 1).unwrap();
        assert!(!view.shows_tree());
        view.tree = true;
        assert!(view.shows_tree());
        view.set_tool(Some(Tool::Memory));
        assert!(!view.shows_tree());
    }

    /// S7 D-10, B17; PRD right-lane D-11: an S6 file becomes one area
    /// holding its tabs in saved order, with its active tab and its preview,
    /// its shown views become the files tool, and the next save writes
    /// schema 3.
    #[test]
    fn a_schema_1_file_migrates_into_one_area() {
        let folder = scratch("migrate");
        let root = folder.path().to_path_buf();
        let path = root.join("workspace-views.json");
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
        assert_eq!(
            (
                view.tool,
                view.pinned,
                view.tree,
                view.lane_width,
                view.last_used_unix_ms
            ),
            (Some(Tool::Files), false, false, None, 7)
        );
        assert_eq!(view.layout.area_count(), 1);
        let area = view.layout.active_area();
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
        assert_eq!(again, loaded);
    }

    /// S6 B20, S7 B17, PRD right-lane B22: a file this build cannot read,
    /// of an unknown version, one whose migration fails or a schema 3 entry
    /// naming a tool no lane has, is kept byte for byte beside the path,
    /// and the defaults load.
    #[test]
    fn an_unreadable_future_or_unmigratable_file_is_preserved_and_defaults_load() {
        let folder = scratch("unreadable");
        let root = folder.path().to_path_buf();
        let path = root.join("workspace-views.json");
        for (index, body) in [
            b"{not json".as_slice(),
            br#"{"schema_version":9,"workspaces":[]}"#.as_slice(),
            br#"{"schema_version":1,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":2,"workspaces":[{"device_id":"local"}]}"#.as_slice(),
            br#"{"schema_version":3,"workspaces":[{"device_id":"local","path":"/r","tool":"explorer"}]}"#.as_slice(),
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
    /// other Views of its area and the arrangement stay.
    #[test]
    fn a_stored_pull_request_card_is_dropped_and_the_other_views_stay() {
        let folder = scratch("retired-card");
        let path = folder.path().join("workspace-views.json");
        let with_card = |files: &[&str], number: u32| {
            let mut layout = Layout::default();
            for file in files {
                let display = layout.new_display(file, DisplayKind::File, None, false);
                layout.insert("a1", display, 1).unwrap();
            }
            let mut value = serde_json::to_value(&layout).unwrap();
            fn displays(value: &mut serde_json::Value) -> Option<&mut Vec<serde_json::Value>> {
                match value {
                    serde_json::Value::Object(object) => {
                        if object.contains_key("displays") {
                            return object.get_mut("displays")?.as_array_mut();
                        }
                        object.values_mut().find_map(displays)
                    }
                    _ => None,
                }
            }
            displays(&mut value).unwrap().push(serde_json::json!(
                {"id": "d90", "path": "", "kind": "pull_request", "pr": number, "last_focused_unix_ms": 5}
            ));
            value
        };
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [{
                    "device_id": "local", "path": "/r", "tool": "files",
                    "view_layout": with_card(&["/r/a.rs"], 7),
                }]
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
        let shown: Vec<_> = view
            .layout
            .displays()
            .map(|display| display.path.as_str())
            .collect();
        assert_eq!(shown, ["/r/a.rs"]);
        let active = view.layout.active_area().active.clone();
        assert_eq!(
            active.as_deref(),
            view.layout.displays().next().map(|d| d.id.as_str())
        );
    }

    #[test]
    fn the_least_recently_used_workspace_is_forgotten_past_the_cap() {
        let mut views = WorkspaceViews::default();
        for index in 0..MAX_WORKSPACES {
            views
                .entry("local", &format!("/w{index}"))
                .last_used_unix_ms = index as u64 + 10;
        }
        views.entry("local", "/w0").last_used_unix_ms = 1_000;
        views.entry("local", "/new");
        assert_eq!(views.workspaces.len(), MAX_WORKSPACES);
        assert!(views.get("local", "/w1").is_none());
        assert!(views.get("local", "/w0").is_some());
    }

    /// One View on file, as an entry's `layout` key.
    fn one_view_layout() -> serde_json::Value {
        let mut layout = Layout::default();
        let display = layout.new_display("/repo/a.md", DisplayKind::File, None, false);
        layout.insert("a1", display, 1).unwrap();
        serde_json::to_value(&layout).unwrap()
    }

    fn stored_entry(path: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut entry =
            serde_json::json!({"device_id": "local", "path": path, "layout": one_view_layout()});
        entry
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        entry
    }

    /// PRD sidebar-task-lens B30, D-34: a lane stored on the Work tool
    /// (`work`, or `agent_sessions` before it) restarts on PRs with its other
    /// choices and its scope chip unchanged, the Work basis is dropped, and
    /// the next save writes the tool as `work` again, which every build since
    /// Work reads (a build before PRs refuses a file naming `prs`).
    #[test]
    fn a_lane_stored_on_work_restarts_on_prs_and_keeps_its_scope() {
        let folder = scratch("work-tool");
        let path = folder.path().join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [
                    {"device_id": "local", "path": "/old", "tool": "agent_sessions",
                     "pinned": true, "lane_width": 500},
                    {"device_id": "local", "path": "/new", "tool": "work", "last_tool": "work",
                     "work": {"lens": "sessions", "only_checkout": true}},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        assert!(matches!(outcome, LoadOutcome::Loaded { ref repairs, .. } if repairs.is_empty()));
        assert_eq!(loaded.workspaces[0].tool, Some(Tool::Prs));
        assert_eq!(loaded.workspaces[0].prs, PrsView::default());
        assert!(loaded.workspaces[0].pinned);
        assert_eq!(loaded.workspaces[0].lane_width, Some(500));
        assert_eq!(loaded.workspaces[1].tool, Some(Tool::Prs));
        assert_eq!(
            loaded.workspaces[1].prs,
            PrsView {
                only_checkout: true
            }
        );
        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            !written.contains("agent_sessions") && !written.contains("\"prs\""),
            "the tool is written under the name older builds read"
        );
        let (again, _) = load(&path, 2);
        assert_eq!(again, loaded);
        let stored: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(stored["workspaces"][1]["tool"], "work");
        assert_eq!(
            stored["workspaces"][1]["work"],
            serde_json::json!({"only_checkout": true})
        );
        assert!(
            stored["workspaces"][0].get("work").is_none(),
            "a default scope leaves the file as it was"
        );
    }

    /// A closed lane returns to the tool it last showed: the key is stored
    /// while the lane is closed, an entry written before it falls back to
    /// the tool an open lane shows, and a lane that never showed one has
    /// none (the shell opens it on Files).
    #[test]
    fn the_last_tool_is_stored_beside_a_closed_lane_and_read_from_older_files() {
        let folder = scratch("last-tool");
        let path = folder.path().join("workspace-views.json");
        let mut views = WorkspaceViews::default();
        let entry = views.entry("local", "/closed");
        entry.set_tool(Some(Tool::Memory));
        entry.set_tool(None);
        assert_eq!((entry.tool, entry.last_tool), (None, Some(Tool::Memory)));
        assert_eq!(views.entry("local", "/never").last_tool, None);
        save(&path, &views).unwrap();
        let (loaded, _) = load(&path, 1);
        let closed = loaded.get("local", "/closed").unwrap();
        assert_eq!((closed.tool, closed.last_tool), (None, Some(Tool::Memory)));

        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [
                    {"device_id": "local", "path": "/open", "tool": "changes"},
                    {"device_id": "local", "path": "/shut"},
                ]
            })
            .to_string(),
        )
        .unwrap();
        let (older, _) = load(&path, 2);
        assert_eq!(
            older.get("local", "/open").unwrap().last_tool,
            Some(Tool::Changes)
        );
        assert_eq!(older.get("local", "/shut").unwrap().last_tool, None);
    }

    /// PRD right-lane D-11, B22: every shape schema 2 stored becomes the
    /// lane once. File Views on gives the files tool, else Tools on gives
    /// its tool, the Explorer as the files tool with its tree on; neither
    /// gives a closed lane. The pin starts off, File Views' width is the
    /// lane's, a tab's File Views visibility is dropped while the stored View
    /// list stays, and the next save writes none of the columns' keys.
    #[test]
    fn a_schema_2_workspace_becomes_its_lane_once() {
        let folder = scratch("columns");
        let root = folder.path().to_path_buf();
        let path = root.join("workspace-views.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 2,
                "workspaces": [
                    stored_entry("/views", serde_json::json!({"views": true, "tools": true, "tool": "changes", "views_width": 700, "tools_width": 300})),
                    stored_entry("/sessions", serde_json::json!({"views": false, "tools": true, "tool": "agent_sessions", "tools_width": 300})),
                    stored_entry("/explorer", serde_json::json!({"views": false, "tools": true, "tool": "explorer"})),
                    stored_entry("/both", serde_json::json!({"views": true, "tools": true, "tool": "explorer"})),
                    stored_entry("/closed", serde_json::json!({"views": false, "tools": false, "tool": "memory", "view_visibility": {"w:t1": true}, "view_owner": "w:t1"})),
                    stored_entry("/panel", serde_json::json!({"panel": "open", "pinned": true, "views_width": 900, "tool": "changes", "tools": false})),
                    stored_entry("/layout", serde_json::json!({"mode": "agents", "explorer": false, "changes": false})),
                    stored_entry("/flags", serde_json::json!({"mode": "agents", "explorer": false, "changes": true})),
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
        let lanes: Vec<_> = loaded
            .workspaces
            .iter()
            .map(|view| {
                (
                    view.path.as_str(),
                    view.tool,
                    view.tree,
                    view.pinned,
                    view.lane_width,
                    view.tree_width,
                )
            })
            .collect();
        assert_eq!(
            lanes,
            vec![
                ("/views", Some(Tool::Files), false, false, Some(700), None),
                ("/sessions", Some(Tool::Prs), false, false, None, None),
                ("/explorer", Some(Tool::Files), true, false, None, None),
                ("/both", Some(Tool::Files), true, false, None, None),
                ("/closed", None, false, false, None, None),
                ("/panel", Some(Tool::Files), false, false, None, None),
                ("/layout", None, false, false, None, None),
                ("/flags", Some(Tool::Changes), false, false, None, None),
            ]
        );
        let closed = loaded.get("local", "/closed").unwrap();
        assert_eq!(closed.layout.display_count(), 1, "views are kept closed");

        save(&path, &loaded).unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\": 3"));
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
        assert_eq!(again, loaded);
    }

    /// B31: a panel value no build wrote makes the file unreadable, which
    /// keeps it aside and loads the defaults.
    #[test]
    fn an_unknown_panel_value_takes_the_unreadable_path() {
        let folder = scratch("legacy-panel-unknown");
        let root = folder.path().to_path_buf();
        let path = root.join("workspace-views.json");
        let body = serde_json::json!({
            "schema_version": 2,
            "workspaces": [{"device_id": "local", "path": "/r", "panel": "floating"}]
        })
        .to_string();
        fs::write(&path, &body).unwrap();
        let (loaded, outcome) = load(&path, 3);
        assert!(loaded.workspaces.is_empty());
        let LoadOutcome::Unreadable {
            preserved_as: Some(preserved),
            ..
        } = outcome
        else {
            panic!("expected the unreadable path, got {outcome:?}");
        };
        assert_eq!(fs::read_to_string(preserved).unwrap(), body);
    }

    /// The checkout's View list and selection survive obsolete tab bookmarks. A
    /// schema 3 file written by an earlier build, with its per-tab keys, loads
    /// the lane and the Agent layout with an empty View list and no repair.
    #[test]
    fn the_checkout_views_survive_a_save_and_obsolete_tab_keys_are_ignored() {
        let folder = scratch("bookmarks");
        let path = folder.path().join("workspace-views.json");
        let mut views = WorkspaceViews::default();
        let entry = views.entry("local", "/repo");
        let display = entry
            .layout
            .new_display("/repo/a.md", DisplayKind::File, None, false);
        let shown = display.id.clone();
        entry.layout.insert("a1", display, 1).unwrap();
        save(&path, &views).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"view_layout\"") && !written.contains("\"tab_views\""));
        assert!(!written.contains("\"view_bookmarks\"") && !written.contains("\"layout\": {"));
        fs::write(
            &path,
            written.replacen(
                "\"view_layout\":",
                "\"tab_views\": {\"w:t1\":\"removed-view\"}, \"view_layout\":",
                1,
            ),
        )
        .unwrap();
        let (loaded, outcome) = load(&path, 1);
        let entry = loaded.get("local", "/repo").unwrap();
        assert_eq!(entry.layout.display_count(), 1);
        assert_eq!(
            entry.layout.active_area().active.as_deref(),
            Some(shown.as_str())
        );
        assert_eq!(outcome, current());

        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 3,
                "workspaces": [stored_entry("/old", serde_json::json!({
                    "tool": "changes", "pinned": true, "tree": true,
                    "lane_width": 500, "tree_width": 260,
                    "feedback": {"next_id": 7},
                    "view_bookmarks": {"w:t1": {"a1": "d1"}},
                    "view_owner": "w:t1",
                    "view_presentations": {"w:t2": {"layout": one_view_layout()}},
                }))]
            })
            .to_string(),
        )
        .unwrap();
        let (older, outcome) = load(&path, 2);
        let old = older.get("local", "/old").unwrap();
        assert_eq!(outcome, current());
        assert_eq!(
            old.layout.display_count(),
            0,
            "the per-tab lists are not read"
        );
        assert_eq!((old.tool, old.pinned), (Some(Tool::Changes), true));
        assert_eq!(
            (
                old.tree,
                old.lane_width,
                old.tree_width,
                old.feedback.next_id
            ),
            (true, Some(500), Some(260), 7),
            "the lane's other choices and the comments survive (D-20)"
        );
    }

    #[test]
    fn a_stored_width_outside_the_bounds_is_clamped() {
        assert_eq!(clamp_width(0), 1);
        assert_eq!(clamp_width(u32::MAX), MAX_WIDTH);
        assert_eq!(clamp_width(640), 640);
    }
}
